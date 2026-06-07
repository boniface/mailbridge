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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TelemetryFields<'a> {
    domain: Option<&'a str>,
    provider: Option<&'a str>,
    status_code: Option<u16>,
    attempt_count: Option<u32>,
    queue_backend: Option<&'a str>,
    elapsed_ms: Option<u128>,
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
}

pub fn emit(event: TelemetryEvent, fields: &TelemetryFields<'_>) {
    emit_inner(event, fields);
}

#[cfg(feature = "telemetry")]
fn emit_inner(event: TelemetryEvent, fields: &TelemetryFields<'_>) {
    let event_name = event_name(event);
    tracing::info!(
        event = event_name,
        domain = fields.domain,
        provider = fields.provider,
        status_code = fields.status_code,
        attempt_count = fields.attempt_count,
        queue_backend = fields.queue_backend,
        elapsed_ms = fields.elapsed_ms,
    );
}

#[cfg(not(feature = "telemetry"))]
fn emit_inner(_event: TelemetryEvent, _fields: &TelemetryFields<'_>) {}

#[cfg(feature = "telemetry")]
fn event_name(event: TelemetryEvent) -> &'static str {
    match event {
        TelemetryEvent::SendStarted => "mailbridge.send.started",
        TelemetryEvent::SendAccepted => "mailbridge.send.accepted",
        TelemetryEvent::SendFailed => "mailbridge.send.failed",
        TelemetryEvent::QueueEnqueued => "mailbridge.queue.enqueued",
        TelemetryEvent::QueueRetryScheduled => "mailbridge.queue.retry_scheduled",
        TelemetryEvent::QueueDeadLettered => "mailbridge.queue.dead_lettered",
        TelemetryEvent::RateLimited => "mailbridge.rate_limited",
    }
}
