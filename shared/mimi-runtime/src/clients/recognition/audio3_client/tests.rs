use super::*;

#[test]
fn silence_clock_starts_at_task_ready_without_setup_debt() {
    let before_setup = tokio::time::Instant::now();
    let ready = before_setup + Duration::from_secs(60);
    let mut clock = Audio3SendClock::default();
    assert!(!clock.can_send_silence(ready));
    clock.start(ready);
    assert!(!clock.can_send_silence(ready + Duration::from_millis(99)));
    assert!(clock.can_send_silence(ready + SILENCE_INTERVAL));
}

fn continuous_batched_clock(
    clock: &mut Audio3SendClock,
    start: tokio::time::Instant,
    batch_ticks: u32,
) -> usize {
    let mut silence_bytes = 0;
    // One hour of continuous PCM delivered in batches. A native callback
    // runs after the timer at equal timestamps, exercising the worst order.
    for tick in 1..=36_000u32 {
        let now = start + SILENCE_INTERVAL * tick;
        if clock.can_send_silence(now) {
            clock.sent(SILENCE_PCM.len(), now, now);
            silence_bytes += SILENCE_PCM.len();
        }
        if tick.is_multiple_of(batch_ticks) {
            clock.sent(SILENCE_PCM.len() * batch_ticks as usize, now, now);
        }
    }
    silence_bytes
}

#[test]
fn one_hour_of_batched_pcm_has_only_one_bounded_initial_silence_gap() {
    let start = tokio::time::Instant::now();
    for batch_ticks in [2, 5] {
        let mut clock = Audio3SendClock::default();
        clock.start(start);
        assert_eq!(
            continuous_batched_clock(&mut clock, start, batch_ticks),
            SILENCE_PCM.len() * batch_ticks as usize
        );
        assert_eq!(
            clock.audio_end_ns,
            Duration::from_millis(3_600_000 + 100 * u64::from(batch_ticks)).as_nanos()
        );
    }
}

#[test]
fn missed_idle_ticks_cannot_fund_silence_after_continuous_pcm_resumes() {
    let start = tokio::time::Instant::now();
    let resumed = start + Duration::from_secs(600);
    let mut clock = Audio3SendClock::default();
    clock.start(start);
    // No task ran for ten minutes. A real batch resumes without making up
    // that lost time as synthetic audio during subsequent capture gaps.
    clock.sent(6400, resumed, resumed);
    assert_eq!(clock.audio_end_ns, Duration::from_secs(600).as_nanos());
    assert_eq!(continuous_batched_clock(&mut clock, resumed, 2), 6400);
}

#[test]
fn silence_clock_preserves_submillisecond_pcm_frames() {
    let start = tokio::time::Instant::now();
    let mut clock = Audio3SendClock::default();
    clock.start(start);
    for _ in 0..16_000 {
        clock.sent(2, start, start);
    }
    assert_eq!(clock.audio_end_ns, Duration::from_secs(1).as_nanos());
    assert!(!clock.can_send_silence(start + Duration::from_secs(1)));
    assert!(clock.can_send_silence(start + Duration::from_millis(1100)));
}

#[test]
fn slow_send_does_not_backfill_history_and_keeps_the_idle_guard() {
    let start = tokio::time::Instant::now();
    let mut clock = Audio3SendClock::default();
    clock.start(start);
    clock.sent(
        3200,
        start + SILENCE_INTERVAL,
        start + Duration::from_secs(1),
    );
    assert!(!clock.can_send_silence(start + Duration::from_secs(1)));
    let next = start + Duration::from_millis(1100);
    assert!(clock.can_send_silence(next));
    clock.sent(3200, next, next);
    assert_eq!(clock.audio_end_ns, Duration::from_millis(1100).as_nanos());
    assert!(!clock.can_send_silence(next));
}

#[test]
fn a_new_task_resets_old_audio_lead() {
    let start = tokio::time::Instant::now();
    let mut clock = Audio3SendClock::default();
    clock.start(start);
    clock.sent(32_000, start, start);
    let next_task = start + Duration::from_millis(200);
    clock.start(next_task);
    assert_eq!(clock.audio_end_ns, 0);
    assert!(!clock.can_send_silence(next_task));
    assert!(clock.can_send_silence(next_task + SILENCE_INTERVAL));
}

#[tokio::test]
async fn clearing_captions_keeps_the_audio_clock() {
    let client = Audio3ASRClient::new("fixture-only", SourceLanguage::English).unwrap();
    let start = tokio::time::Instant::now();
    {
        let mut clock = client.inner.audio_clock.lock().await;
        clock.start(start);
        clock.sent(6400, start, start);
    }
    client.clear_content().await;
    let clock = client.inner.audio_clock.lock().await;
    assert_eq!(clock.started_at, Some(start));
    assert_eq!(clock.audio_end_ns, Duration::from_millis(200).as_nanos());
    assert_eq!(clock.last_sent_at, Some(start));
}

fn caption(id: Option<u64>, is_final: bool) -> Audio3ASRServerEvent {
    Audio3ASRServerEvent::Transcription {
        text: "Synthetic caption".into(),
        is_final,
        sentence_id: id,
    }
}

#[test]
fn content_clear_suppresses_current_sentence_and_replays_but_accepts_next_id() {
    let mut gate = Audio3ContentGate::default();
    assert!(gate.accepts(&caption(Some(7), false)));
    gate.clear(1);
    assert!(!gate.accepts(&caption(Some(7), false)));
    gate.clear(2);
    assert!(!gate.accepts(&caption(None, false)));
    assert!(!gate.accepts(&caption(Some(7), false)));
    assert!(gate.accepts(&caption(Some(8), false)));
    assert!(!gate.accepts(&caption(Some(7), true)));
    assert!(gate.accepts(&caption(Some(8), true)));
    gate.clear(2);
    assert!(!gate.accepts(&caption(Some(8), true)));
    assert!(gate.accepts(&caption(Some(9), false)));
}

#[test]
fn missing_identity_clear_waits_for_a_real_final_without_guessing_a_prefix() {
    let mut gate = Audio3ContentGate::default();
    assert!(gate.accepts(&caption(None, false)));
    gate.clear(1);
    gate.clear(2);
    assert!(!gate.accepts(&caption(None, false)));
    assert!(!gate.accepts(&caption(None, false)));
    assert!(!gate.accepts(&caption(Some(1), true)));
    assert!(!gate.accepts(&caption(Some(1), true)));
    assert!(gate.accepts(&caption(Some(2), false)));
    let mut unseen = Audio3ContentGate::default();
    unseen.clear(1);
    // No observed sentence means there is no honest capture-time cutoff.
    assert!(unseen.accepts(&caption(Some(1), false)));
}

#[test]
fn failures_preserve_setup_or_recognition_stage() {
    assert_eq!(
        task_failure_token("CLIENT_ERROR", "timeout", false),
        "audio3_error.setup.timeout.CLIENT_ERROR"
    );
    assert_eq!(
        task_failure_token("SERVER_ERROR", "service", true),
        "audio3_error.recognition.service.SERVER_ERROR"
    );
}
