//! Generation-tagged capture, transport, pipeline and event-pump ownership.
//! Install and retirement keep their existing ordering and cancellation boundaries.

use super::cleanup_owner::{self, CleanupTicket};
use super::ownership::{clear_owned_value_if_generation, SESSION_START_CANCELLED};
use super::SessionManager;
use crate::audio::echo_pipeline::EchoPipeline;
use crate::audio::send_pipeline::AudioSendPipeline;
use crate::audio::{AudioCapture, CaptureFailureSender};
use crate::clients::translation::translation_client::TranslationClient;
use crate::core::audio::audio_input::AudioSource;
use crate::core::diagnostics::support_diagnostics::CaptureObservation;
use crate::core::services::configuration::LiveTranslationConfiguration;
use crate::pipeline_log;
use mimi_runtime::core::lifecycle::health::DISCONNECT_TIMEOUT;
use mimi_runtime::core::lifecycle::recovery::{MTBudgetContinuity, NO_GENERATION};
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::AppHandle;
use tokio::task::JoinHandle;

#[derive(Clone, Copy)]
pub(super) enum EventPumpCleanup {
    Preserve,
    Abort,
}

/// Establishing a selection is transactional: a later source's failure must
/// release the earlier source too, including partially installed resources.
pub(super) async fn connect_source_group<Start, Started, Rollback, RolledBack>(
    sources: &[AudioSource],
    mut start: Start,
    rollback: Rollback,
) -> Result<(), String>
where
    Start: FnMut(AudioSource) -> Started,
    Started: Future<Output = Result<(), String>>,
    Rollback: FnOnce(EventPumpCleanup) -> RolledBack,
    RolledBack: Future<Output = ()>,
{
    for &source in sources {
        if let Err(error) = start(source).await {
            // The event pump may have invalidated this startup and still own
            // a recovery handoff. Release captures/clients without aborting it.
            rollback(EventPumpCleanup::Preserve).await;
            return Err(error);
        }
    }
    Ok(())
}

/// Retired capture and transport are independent once PCM ingress is sealed.
pub(super) async fn release_retired_sources<Capture, CaptureDone, Disconnect, DisconnectDone>(
    stop_capture: Capture,
    disconnect: Disconnect,
) where
    Capture: Fn(AudioSource) -> CaptureDone,
    CaptureDone: Future<Output = ()>,
    Disconnect: Fn(AudioSource) -> DisconnectDone,
    DisconnectDone: Future<Output = ()>,
{
    tokio::join!(
        stop_capture(AudioSource::System),
        stop_capture(AudioSource::Microphone),
        disconnect(AudioSource::System),
        disconnect(AudioSource::Microphone),
    );
}

/// A source owns every transport, capture, revision and timer it can mutate.
/// Lifecycle generations are shared so either lane failing tears down both.
#[derive(Clone)]
pub(super) struct RuntimeLane {
    pub(super) audio: Arc<Mutex<AudioCapture>>,
    pub(super) client: Arc<Mutex<Option<TranslationClient>>>,
    pub(super) client_generation: Arc<AtomicU64>,
    pub(super) subtitle_content_revision: Arc<Mutex<(u64, u64)>>,
    pub(super) mt_budget_continuity: Arc<Mutex<MTBudgetContinuity>>,
    pub(super) audio_pipeline: Arc<Mutex<Option<Arc<AudioSendPipeline>>>>,
    pub(super) audio_pipeline_generation: Arc<AtomicU64>,
    pub(super) capture_generation: Arc<AtomicU64>,
    pub(super) pump_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    pub(super) pump_generation: Arc<AtomicU64>,
    pub(super) translation_timeout_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    pub(super) translation_timeout_task_id: Arc<AtomicU64>,
}

impl RuntimeLane {
    pub(super) fn new(app: &AppHandle) -> Self {
        let audio_capture = AudioCapture::for_app(app);
        Self {
            audio: Arc::new(Mutex::new(audio_capture)),
            client: Arc::new(Mutex::new(None)),
            client_generation: Arc::new(AtomicU64::new(NO_GENERATION)),
            subtitle_content_revision: Default::default(),
            mt_budget_continuity: Default::default(),
            audio_pipeline: Arc::new(Mutex::new(None)),
            audio_pipeline_generation: Arc::new(AtomicU64::new(NO_GENERATION)),
            capture_generation: Arc::new(AtomicU64::new(NO_GENERATION)),
            pump_task: Arc::new(Mutex::new(None)),
            pump_generation: Arc::new(AtomicU64::new(NO_GENERATION)),
            translation_timeout_task: Arc::new(Mutex::new(None)),
            translation_timeout_task_id: Arc::new(AtomicU64::new(NO_GENERATION)),
        }
    }
}

pub(super) type EchoPipelineSlot = Arc<Mutex<Option<(u64, Arc<EchoPipeline>)>>>;

impl SessionManager {
    pub(super) async fn finish_source_pipeline(&self, source: AudioSource, timeout: Duration) {
        let pipeline = self.lane(source).audio_pipeline.lock().unwrap().take();
        if let Some(pipeline) = pipeline {
            self.lane(source)
                .audio_pipeline_generation
                .store(NO_GENERATION, Ordering::SeqCst);
            if !pipeline.finish(timeout).await {
                pipeline_log!("audio pipeline drain timed out");
            }
        }
    }

    /// Capacity-one channel through which a native callback reports its first
    /// fatal failure without blocking the real-time audio thread.
    pub(super) fn capture_failure_channel(
        self: &Arc<Self>,
        generation: u64,
        attempt: u64,
    ) -> CaptureFailureSender {
        let (tx, mut rx) = CaptureFailureSender::channel();
        let self_arc = Arc::clone(self);
        tokio::spawn(async move {
            if let Some(failure) = rx.recv().await {
                self_arc
                    .clone()
                    .handle_capture_failure(generation, attempt, failure)
                    .await;
            }
        });
        tx
    }

    pub(super) async fn install_client(
        &self,
        source: AudioSource,
        generation: u64,
        client: TranslationClient,
    ) -> Result<(), String> {
        // Recovery may install outside the command lifecycle guard. Keep its
        // owner/revision atomic with a Clear that is still awaiting the old client.
        let _content = self.subtitle_content_lock.lock().await;
        let _transition = self.generation_transition.lock().unwrap();
        self.ensure_generation_current(generation)?;
        let mut slot = self.lane(source).client.lock().unwrap();
        if slot.is_some() {
            return Err("A live translation client is already installed.".into());
        }
        *self.lane(source).subtitle_content_revision.lock().unwrap() =
            (generation, client.content_revision());
        self.controller
            .lock()
            .unwrap()
            .set_atomic_preview(client.uses_atomic_preview());
        *slot = Some(client);
        self.lane(source)
            .client_generation
            .store(generation, Ordering::SeqCst);
        // Settings can rebuild a client within the same lifecycle generation.
        // Its new socket must not inherit the previous socket's probe time.
        *self.health_latency.lock().unwrap() = None;
        Ok(())
    }

    pub(super) fn set_active_settings(
        &self,
        generation: u64,
        configuration: Option<LiveTranslationConfiguration>,
    ) {
        let mut slot = self.active_settings.lock().unwrap();
        *slot = configuration;
        self.active_settings_generation.store(
            if slot.is_some() {
                generation
            } else {
                NO_GENERATION
            },
            Ordering::SeqCst,
        );
    }

    pub(super) fn retag_active_settings(&self, generation: u64) {
        let slot = self.active_settings.lock().unwrap();
        if slot.is_some() {
            self.active_settings_generation
                .store(generation, Ordering::SeqCst);
        }
    }

    pub(super) fn clear_active_settings_for_generation(&self, generation: u64) -> bool {
        clear_owned_value_if_generation(
            &self.active_settings,
            &self.active_settings_generation,
            generation,
        )
    }

    pub(super) fn clear_active_settings(&self) {
        *self.active_settings.lock().unwrap() = None;
        self.active_settings_generation
            .store(NO_GENERATION, Ordering::SeqCst);
    }

    pub(super) fn client_for_generation(
        &self,
        source: AudioSource,
        generation: u64,
    ) -> Option<TranslationClient> {
        let slot = self.lane(source).client.lock().unwrap();
        if self.lane(source).client_generation.load(Ordering::SeqCst) == generation {
            slot.clone()
        } else {
            None
        }
    }

    pub(super) fn take_client_for_generation(
        &self,
        source: AudioSource,
        generation: u64,
    ) -> Option<TranslationClient> {
        let mut slot = self.lane(source).client.lock().unwrap();
        if self.lane(source).client_generation.load(Ordering::SeqCst) != generation {
            return None;
        }
        self.lane(source)
            .client_generation
            .store(NO_GENERATION, Ordering::SeqCst);
        slot.take()
    }

    pub(super) fn take_any_client(&self, source: AudioSource) -> Option<TranslationClient> {
        let mut slot = self.lane(source).client.lock().unwrap();
        self.lane(source)
            .client_generation
            .store(NO_GENERATION, Ordering::SeqCst);
        slot.take()
    }

    pub(super) fn install_pipeline(
        &self,
        source: AudioSource,
        generation: u64,
        pipeline: Arc<AudioSendPipeline>,
    ) -> Result<(), String> {
        let _transition = self.generation_transition.lock().unwrap();
        self.ensure_generation_current(generation)?;
        if !self.is_lifecycle_request_current(generation) {
            return Err(SESSION_START_CANCELLED.into());
        }
        let mut slot = self.lane(source).audio_pipeline.lock().unwrap();
        if slot.is_some() {
            return Err("An audio send pipeline is already installed.".into());
        }
        *slot = Some(pipeline);
        self.lane(source)
            .audio_pipeline_generation
            .store(generation, Ordering::SeqCst);
        Ok(())
    }

    pub(super) fn take_pipeline_for_generation(
        &self,
        source: AudioSource,
        generation: u64,
    ) -> Option<Arc<AudioSendPipeline>> {
        let mut slot = self.lane(source).audio_pipeline.lock().unwrap();
        if self
            .lane(source)
            .audio_pipeline_generation
            .load(Ordering::SeqCst)
            != generation
        {
            return None;
        }
        self.lane(source)
            .audio_pipeline_generation
            .store(NO_GENERATION, Ordering::SeqCst);
        let pipeline = slot.take();
        if let Some(pipeline) = &pipeline {
            let (pcm_data_recent, sound_recent) = pipeline.input_activity();
            *self.diagnostic_capture.lock().unwrap() = Some((
                CaptureObservation {
                    pcm_data_recent,
                    sound_recent,
                },
                Instant::now(),
            ));
        }
        pipeline
    }

    pub(super) fn install_pump(&self, source: AudioSource, generation: u64, pump: JoinHandle<()>) {
        let _transition = self.generation_transition.lock().unwrap();
        if !self.is_generation_current(generation) || !self.is_lifecycle_request_current(generation)
        {
            pump.abort();
            return;
        }
        let mut slot = self.lane(source).pump_task.lock().unwrap();
        if let Some(old) = slot.replace(pump) {
            old.abort();
        }
        self.lane(source)
            .pump_generation
            .store(generation, Ordering::SeqCst);
    }

    pub(super) fn stop_pump_for_generation(&self, source: AudioSource, generation: u64) {
        if let Some(task) = self.take_pump_for_generation(source, generation) {
            task.abort();
        }
    }

    pub(super) fn take_pump_for_generation(
        &self,
        source: AudioSource,
        generation: u64,
    ) -> Option<JoinHandle<()>> {
        let mut slot = self.lane(source).pump_task.lock().unwrap();
        if self.lane(source).pump_generation.load(Ordering::SeqCst) != generation {
            return None;
        }
        self.lane(source)
            .pump_generation
            .store(NO_GENERATION, Ordering::SeqCst);
        slot.take()
    }

    pub(super) async fn finish_pump_for_generation(
        &self,
        source: AudioSource,
        generation: u64,
        timeout: Duration,
    ) {
        let Some(mut task) = self.take_pump_for_generation(source, generation) else {
            return;
        };
        if tokio::time::timeout(timeout, &mut task).await.is_err() {
            task.abort();
        }
    }

    pub(super) fn stop_any_pump(&self, source: AudioSource) {
        self.lane(source)
            .pump_generation
            .store(NO_GENERATION, Ordering::SeqCst);
        if let Some(task) = self.lane(source).pump_task.lock().unwrap().take() {
            task.abort();
        }
    }

    pub(super) async fn stop_capture_for_generation(&self, source: AudioSource, generation: u64) {
        if self
            .lane(source)
            .capture_generation
            .compare_exchange(
                generation,
                NO_GENERATION,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_err()
        {
            return;
        }
        if let Some(observation) = self.current_capture_observation() {
            *self.diagnostic_capture.lock().unwrap() = Some((observation, Instant::now()));
        }
        let capture = self.lane(source).audio.lock().unwrap().clone();
        if tokio::time::timeout(Duration::from_secs(2), capture.stop())
            .await
            .is_err()
        {
            pipeline_log!("audio capture stop timed out");
        }
    }

    pub(super) async fn stop_any_capture(&self, source: AudioSource) {
        if self
            .lane(source)
            .capture_generation
            .swap(NO_GENERATION, Ordering::SeqCst)
            == NO_GENERATION
        {
            return;
        }
        let capture = self.lane(source).audio.lock().unwrap().clone();
        if tokio::time::timeout(Duration::from_secs(2), capture.stop())
            .await
            .is_err()
        {
            pipeline_log!("audio capture stop timed out");
        }
    }

    pub(super) async fn finish_pipeline_for_generation(
        &self,
        source: AudioSource,
        generation: u64,
        timeout: Duration,
    ) {
        if let Some(pipeline) = self.take_pipeline_for_generation(source, generation) {
            if !pipeline.finish(timeout).await {
                pipeline_log!("audio pipeline drain timed out");
            }
        }
    }

    pub(super) fn cleanup_generation(&self, generation: u64) -> CleanupTicket {
        self.cleanup_generation_resources(generation, EventPumpCleanup::Abort)
    }

    pub(super) fn cleanup_generation_without_pump(&self, generation: u64) -> CleanupTicket {
        self.cleanup_generation_resources(generation, EventPumpCleanup::Preserve)
    }

    pub(super) fn cleanup_generation_resources(
        &self,
        generation: u64,
        pumps: EventPumpCleanup,
    ) -> CleanupTicket {
        if generation == NO_GENERATION {
            return CleanupTicket::completed();
        }
        let guard = self.begin_teardown_operation();
        let manager = self.clone();
        let after = if matches!(pumps, EventPumpCleanup::Abort) {
            let manager = self.clone();
            Some(Box::new(move || {
                for source in [AudioSource::System, AudioSource::Microphone] {
                    manager.stop_pump_for_generation(source, generation);
                }
            }) as cleanup_owner::CleanupAfter)
        } else {
            None
        };
        self.cleanup_owners.register(
            generation,
            guard,
            move || async move { manager.cleanup_generation_owned(generation).await },
            after,
        )
    }

    pub(super) async fn cleanup_generation_owned(&self, generation: u64) {
        self.retire_audio_attempt(generation);
        if let Some(echo) = self.take_echo_pipeline(generation) {
            echo.stop();
        }
        for source in [AudioSource::System, AudioSource::Microphone] {
            if let Some(pipeline) = self.take_pipeline_for_generation(source, generation) {
                pipeline.stop();
            }
        }
        release_retired_sources(
            |source| self.stop_capture_for_generation(source, generation),
            |source| self.disconnect_retired_client(source, generation),
        )
        .await;
    }

    pub(super) async fn disconnect_retired_client(&self, source: AudioSource, generation: u64) {
        if let Some(client) = self.take_client_for_generation(source, generation) {
            self.remember_mt_request_budget(source, generation, &client)
                .await;
            let _ = tokio::time::timeout(DISCONNECT_TIMEOUT, client.disconnect()).await;
        }
    }

    pub(super) fn retire_audio_attempt(&self, generation: u64) {
        let _transition = self.generation_transition.lock().unwrap();
        let mut owner = self.audio_attempt.lock().unwrap();
        if generation == NO_GENERATION || owner.is_some_and(|(active, _)| active == generation) {
            *owner = None;
        }
    }

    pub(super) fn take_echo_pipeline(&self, generation: u64) -> Option<Arc<EchoPipeline>> {
        let mut slot = self.echo_pipeline.lock().unwrap();
        if generation == NO_GENERATION
            || slot.as_ref().is_some_and(|(owner, _)| *owner == generation)
        {
            slot.take().map(|(_, echo)| echo)
        } else {
            None
        }
    }

    pub(super) async fn remember_mt_request_budget(
        &self,
        source: AudioSource,
        generation: u64,
        client: &TranslationClient,
    ) {
        let lease = self
            .lane(source)
            .mt_budget_continuity
            .lock()
            .unwrap()
            .take_lease(generation);
        let Some((token, scope)) = lease else {
            return;
        };
        if let Some(budget) = client.suspend_mt_request_budget().await {
            self.lane(source)
                .mt_budget_continuity
                .lock()
                .unwrap()
                .remember(token, scope, budget);
        }
    }
}
