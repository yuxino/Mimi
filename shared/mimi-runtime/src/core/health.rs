//! Timing policy shared by native session drivers.

use crate::clients::translation_client::{ConnectError, TranslationClient};
use std::time::{Duration, Instant};

pub const SNAPSHOT_PUBLISH_INTERVAL: Duration = Duration::from_millis(60);
pub const HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(10);
pub const HEALTH_PROBE_TIMEOUT: Duration = Duration::from_secs(4);
pub const AUDIO_DRAIN_TIMEOUT: Duration = Duration::from_secs(1);
pub const DISCONNECT_TIMEOUT: Duration = Duration::from_secs(2);
pub const PROVIDER_FINISH_TIMEOUT: Duration =
    Duration::from_millis(mimi_core::translation_policy::PROVIDER_FINISH_TIMEOUT_MS);
/// The native shell's watchdog must cover every sequential stop phase and
/// leave two publication ticks for the final snapshot/acknowledgement.
pub const SESSION_FINISH_TIMEOUT: Duration = Duration::from_millis(
    AUDIO_DRAIN_TIMEOUT.as_millis() as u64
        + PROVIDER_FINISH_TIMEOUT.as_millis() as u64
        + DISCONNECT_TIMEOUT.as_millis() as u64
        + 2 * SNAPSHOT_PUBLISH_INTERVAL.as_millis() as u64,
);

/// Run the client's actual protocol health check and measure its elapsed time.
/// Session drivers retain ownership checks before publishing the measurement.
pub async fn probe_connection_health(client: &TranslationClient) -> Result<Duration, ConnectError> {
    let started_at = Instant::now();
    client.ping(HEALTH_PROBE_TIMEOUT).await?;
    Ok(started_at.elapsed())
}
