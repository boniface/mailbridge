mod events;
#[cfg(feature = "telemetry")]
mod spans;

pub use events::{TelemetryEvent, TelemetryFields, emit};
#[cfg(feature = "telemetry")]
pub(crate) use spans::{
    provider_send_span, queue_dead_letter_span, queue_enqueue_span, queue_process_span,
    queue_reserve_span, queue_retry_span, send_span,
};
