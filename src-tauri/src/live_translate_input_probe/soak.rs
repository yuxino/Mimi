//! Manual 2–3 hour input schedules. Service/runtime/DSP implementations remain
//! production code; this driver supplies fixed public PCM instead of capture.

use super::*;
use std::collections::BTreeMap;

#[derive(serde::Deserialize)]
struct Clip {
    path: PathBuf,
    bytes: usize,
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum Feed {
    Named(String),
    Offset { clip: String, offset_seconds: usize },
}

impl Feed {
    fn parts(&self) -> (&str, usize) {
        match self {
            Self::Named(name) => (name, 0),
            Self::Offset {
                clip,
                offset_seconds,
            } => (clip, *offset_seconds),
        }
    }
}

#[derive(serde::Deserialize)]
struct Phase {
    label: String,
    seconds: u64,
    system: Feed,
    microphone: Feed,
}

#[derive(serde::Deserialize)]
struct Manifest {
    clips: BTreeMap<String, Clip>,
    phases: Vec<Phase>,
}

struct Scenario {
    clips: BTreeMap<String, Vec<u8>>,
    phases: Vec<Phase>,
    seconds: u64,
}

impl Scenario {
    fn load(path: PathBuf) -> Self {
        let bytes = std::fs::read(path).unwrap();
        assert!(bytes.len() <= 128_000);
        let manifest: Manifest = serde_json::from_slice(&bytes).unwrap();
        assert!((1..=8).contains(&manifest.clips.len()));
        assert!((1..=200).contains(&manifest.phases.len()));
        assert!(manifest.clips.values().map(|c| c.bytes).sum::<usize>() <= 160_000_000);
        let label_ok = |v: &str| {
            !v.is_empty()
                && v.len() <= 64
                && v.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
        };
        let clips: BTreeMap<_, _> = manifest
            .clips
            .into_iter()
            .map(|(name, clip)| {
                assert!(label_ok(&name) && !matches!(name.as_str(), "silence" | "absent"));
                assert!(
                    (640..=32_000 * 3600).contains(&clip.bytes) && clip.bytes.is_multiple_of(2)
                );
                let data = std::fs::read(clip.path).unwrap();
                assert_eq!(data.len(), clip.bytes);
                (name, data)
            })
            .collect();
        for phase in &manifest.phases {
            assert!(label_ok(&phase.label));
            assert!((10..=1800).contains(&phase.seconds));
            for feed in [&phase.system, &phase.microphone] {
                let (name, offset) = feed.parts();
                assert!(matches!(name, "silence" | "absent") || clips.contains_key(name));
                assert!(offset <= 3600);
            }
        }
        let seconds = manifest.phases.iter().map(|p| p.seconds).sum();
        // Reuse the same driver for a bounded failing slice and the remaining
        // schedule after a long run stops; neither is a fresh full-run pass.
        assert!(
            (180..=10800).contains(&seconds),
            "authorized scenario must be between 3 minutes and 3 hours"
        );
        Self {
            clips,
            phases: manifest.phases,
            seconds,
        }
    }

    fn phase(&self, elapsed_ms: u64) -> Option<(&Phase, u64)> {
        let mut start = 0;
        for phase in &self.phases {
            let end = start + phase.seconds * 1000;
            if elapsed_ms < end {
                return Some((phase, elapsed_ms - start));
            }
            start = end;
        }
        None
    }

    fn packet(&self, source: AudioSource, elapsed_ms: u64) -> Option<Vec<u8>> {
        let (phase, elapsed_ms) = self.phase(elapsed_ms)?;
        let feed = match source {
            AudioSource::System => &phase.system,
            AudioSource::Microphone => &phase.microphone,
        };
        let (name, start_seconds) = feed.parts();
        match name {
            "absent" => None,
            "silence" => Some(vec![0; 640]),
            _ => {
                let pcm = &self.clips[name];
                let offset =
                    ((elapsed_ms / 20) as usize * 640 + start_seconds * 32_000) % pcm.len();
                Some((0..640).map(|i| pcm[(offset + i) % pcm.len()]).collect())
            }
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Playback<'a> {
    scenario: &'a Scenario,
    started: Instant,
}

impl<'a> Playback<'a> {
    pub(super) fn elapsed_ms(self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }
    pub(super) fn complete(self) -> bool {
        self.started.elapsed().as_secs() >= self.scenario.seconds
    }
    pub(super) fn phase_name(self) -> &'a str {
        self.scenario
            .phase(self.elapsed_ms())
            .map_or("complete", |(p, _)| p.label.as_str())
    }

    pub(super) fn packet(self, source: AudioSource) -> Option<Vec<u8>> {
        // A reconnect joins the current live position. Do not replay speech
        // missed while disconnected or dilate the source clock.
        self.scenario.packet(source, self.elapsed_ms())
    }
}

#[test]
fn schedule_keeps_source_identity_and_live_position_across_quiet_boundaries() {
    let scenario = Scenario {
        clips: [
            ("a".into(), [vec![1; 640], vec![2; 640]].concat()),
            ("b".into(), vec![3; 640]),
        ]
        .into(),
        phases: vec![
            Phase {
                label: "talk".into(),
                seconds: 10,
                system: Feed::Named("a".into()),
                microphone: Feed::Named("absent".into()),
            },
            Phase {
                label: "swap".into(),
                seconds: 10,
                system: Feed::Named("silence".into()),
                microphone: Feed::Named("b".into()),
            },
        ],
        seconds: 20,
    };
    assert_eq!(scenario.packet(AudioSource::System, 20), Some(vec![2; 640]));
    assert_eq!(scenario.packet(AudioSource::System, 60), Some(vec![2; 640]));
    assert_eq!(scenario.packet(AudioSource::Microphone, 9980), None);
    assert_eq!(
        scenario.packet(AudioSource::System, 10000),
        Some(vec![0; 640])
    );
    assert_eq!(
        scenario.packet(AudioSource::Microphone, 10000),
        Some(vec![3; 640])
    );
    assert_eq!(scenario.packet(AudioSource::Microphone, 20000), None);
}

#[test]
#[ignore = "requires explicit public-fixture schedule; no credentials or network"]
fn manual_live_translate_soak_preflight() {
    let scenario = Scenario::load(
        std::env::var_os("MIMI_INPUT_PROBE_SOAK_MANIFEST")
            .unwrap()
            .into(),
    );
    let mut blocks = [0; 3];
    for ms in (0..scenario.seconds * 1000).step_by(1000) {
        for source in [AudioSource::System, AudioSource::Microphone] {
            let packet = scenario.packet(source, ms);
            let category = match packet {
                None => 0,
                Some(bytes) => {
                    assert_eq!(bytes.len(), 640);
                    if bytes.iter().all(|&b| b == 0) {
                        1
                    } else {
                        2
                    }
                }
            };
            blocks[category] += 1;
        }
    }
    assert!(blocks.iter().all(|&n| n > 0));
    println!(
        "{}",
        json!({"phase":"soak_preflight","seconds":scenario.seconds,"phases":scenario.phases.len(),"sampled_absent_blocks":blocks[0],"sampled_silent_blocks":blocks[1],"sampled_nonzero_blocks":blocks[2]})
    );
}

pub(super) async fn run(configuration: &LiveTranslationConfiguration, path: PathBuf) -> bool {
    use mimi_runtime::core::recovery::{
        provider_error_is_retryable, provider_recovery_minimum_delay, RecoverySchedule,
    };
    let scenario = Scenario::load(path);
    let started = Instant::now();
    let playback = Playback {
        scenario: &scenario,
        started,
    };
    let mut recovery: Option<RecoverySchedule> = None;
    let mut connections = 0;
    println!(
        "{}",
        json!({"phase":"soak_started","seconds":scenario.seconds,"phases":scenario.phases.len(),"clips":scenario.clips.len(),"proxy_mode":configuration.network_proxy.mode})
    );
    loop {
        connections += 1;
        let connected_at = Instant::now();
        let outcome = observe_case(
            configuration,
            &[0; 640],
            AudioInput::Both,
            scenario.seconds,
            "multilingual_soak",
            true,
            Some(playback),
        )
        .await;
        let Some(code) = outcome.failure else {
            println!(
                "{}",
                json!({"phase":"soak_finished","elapsed_ms":playback.elapsed_ms(),"connection_generations":connections})
            );
            return playback.complete();
        };
        if !provider_error_is_retryable(&code) {
            println!(
                "{}",
                json!({"phase":"soak_terminal_failure","code":code,"elapsed_ms":playback.elapsed_ms()})
            );
            return false;
        }
        // Bound consecutive establishment failures, but a later independent
        // interruption starts a fresh recovery window after stable streaming.
        if connected_at.elapsed() >= Duration::from_secs(30) {
            recovery = None;
        }
        let schedule = recovery.get_or_insert_with(|| {
            RecoverySchedule::new(
                connections,
                connections + 1,
                provider_recovery_minimum_delay(&code),
            )
        });
        let Some(attempt) = schedule.next_attempt() else {
            println!(
                "{}",
                json!({"phase":"soak_retries_exhausted","elapsed_ms":playback.elapsed_ms()})
            );
            return false;
        };
        println!(
            "{}",
            json!({"phase":"soak_recovery","code":code,"elapsed_ms":playback.elapsed_ms(),"attempt":attempt.index+1,"delay_ms":attempt.delay.as_millis()})
        );
        tokio::time::sleep(attempt.delay).await;
    }
}
