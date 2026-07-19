#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetryEvent {
    SendStarted,
    SendAccepted,
    SendFailed,
    QueueEnqueued,
    QueueRetryScheduled,
    QueueDeadLettered,
    RateLimited,
}

impl TelemetryEvent {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SendStarted => "mailbridge.send.started",
            Self::SendAccepted => "mailbridge.send.accepted",
            Self::SendFailed => "mailbridge.send.failed",
            Self::QueueEnqueued => "mailbridge.queue.enqueued",
            Self::QueueRetryScheduled => "mailbridge.queue.retry_scheduled",
            Self::QueueDeadLettered => "mailbridge.queue.dead_lettered",
            Self::RateLimited => "mailbridge.rate_limited",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TelemetryFields<'a> {
    domain: Option<&'a str>,
    provider: Option<&'a str>,
    status_code: Option<u16>,
    attempt_count: Option<u32>,
    queue_backend: Option<&'a str>,
    elapsed_ms: Option<u128>,
    delivery_mode: Option<&'a str>,
    error_kind: Option<&'a str>,
    retryable: Option<bool>,
}

impl<'a> TelemetryFields<'a> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            domain: None,
            provider: None,
            status_code: None,
            attempt_count: None,
            queue_backend: None,
            elapsed_ms: None,
            delivery_mode: None,
            error_kind: None,
            retryable: None,
        }
    }

    #[must_use]
    pub const fn domain(mut self, value: &'a str) -> Self {
        self.domain = Some(value);
        self
    }

    #[must_use]
    pub const fn provider(mut self, value: &'a str) -> Self {
        self.provider = Some(value);
        self
    }

    #[must_use]
    pub const fn status_code(mut self, value: u16) -> Self {
        self.status_code = Some(value);
        self
    }

    #[must_use]
    pub const fn attempt_count(mut self, value: u32) -> Self {
        self.attempt_count = Some(value);
        self
    }

    #[must_use]
    pub const fn queue_backend(mut self, value: &'a str) -> Self {
        self.queue_backend = Some(value);
        self
    }

    #[must_use]
    pub const fn elapsed_ms(mut self, value: u128) -> Self {
        self.elapsed_ms = Some(value);
        self
    }

    #[must_use]
    pub const fn delivery_mode(mut self, value: &'a str) -> Self {
        self.delivery_mode = Some(value);
        self
    }

    #[must_use]
    pub const fn error_kind(mut self, value: &'a str) -> Self {
        self.error_kind = Some(value);
        self
    }

    #[must_use]
    pub const fn retryable(mut self, value: bool) -> Self {
        self.retryable = Some(value);
        self
    }
}

pub fn emit(event: TelemetryEvent, fields: &TelemetryFields<'_>) {
    emit_inner(event, fields);
}

#[cfg(feature = "telemetry")]
fn emit_inner(event: TelemetryEvent, fields: &TelemetryFields<'_>) {
    tracing::info!(
        target: "mailbridge",
        mailbridge_event = event.as_str(),
        mailbridge_sender_domain = fields.domain,
        mailbridge_provider = fields.provider,
        mailbridge_status_code = fields.status_code,
        mailbridge_retry_attempt = fields.attempt_count,
        mailbridge_queue_backend = fields.queue_backend,
        mailbridge_elapsed_ms = fields.elapsed_ms,
        mailbridge_delivery_mode = fields.delivery_mode,
        mailbridge_error_kind = fields.error_kind,
        mailbridge_retryable = fields.retryable,
    );
}

#[cfg(not(feature = "telemetry"))]
fn emit_inner(_event: TelemetryEvent, _fields: &TelemetryFields<'_>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_names_are_stable_and_namespaced() {
        assert_eq!(
            TelemetryEvent::SendStarted.as_str(),
            "mailbridge.send.started"
        );
        assert_eq!(
            TelemetryEvent::QueueDeadLettered.as_str(),
            "mailbridge.queue.dead_lettered"
        );
    }

    #[test]
    fn fields_do_not_store_private_message_content() {
        let fields = TelemetryFields::new()
            .provider("hyvor-relay")
            .domain("example.com")
            .error_kind("temporary")
            .retryable(true);

        assert_eq!(
            fields,
            TelemetryFields {
                domain: Some("example.com"),
                provider: Some("hyvor-relay"),
                status_code: None,
                attempt_count: None,
                queue_backend: None,
                elapsed_ms: None,
                delivery_mode: None,
                error_kind: Some("temporary"),
                retryable: Some(true),
            }
        );
    }
}
