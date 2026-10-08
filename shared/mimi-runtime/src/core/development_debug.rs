//! Development trace API. The journal implementation exists only in explicitly
//! enabled development builds or tests; production has no trace state or sink.

use serde::{Deserialize, Serialize};

#[cfg(any(test, feature = "development-debugger"))]
#[path = "development_debug_enabled.rs"]
mod enabled;
#[cfg(any(test, feature = "development-debugger"))]
pub use enabled::*;

#[cfg(any(test, not(feature = "development-debugger")))]
#[path = "development_debug_disabled.rs"]
mod disabled;
#[cfg(not(any(test, feature = "development-debugger")))]
pub use disabled::*;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Admission {
    QueueDraftAttempt,
    QueueReliableAttempt,
    ProducerClosed,
    ProducerBackpressureResult,
    ObsoleteProducer,
    ProducerBackpressure,
    OversizedText,
    Accepted,
    StaleContent,
    StaleGeneration,
    Paused,
    SetupAcknowledgement,
    DuplicateFinal,
}

#[cfg(any(test, feature = "development-debugger"))]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DebugProducer {
    #[default]
    Session,
    Provider,
    Recognition,
}
