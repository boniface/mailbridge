pub(crate) fn send_span(provider: &str, sender_domain: &str, delivery_mode: &str) -> tracing::Span {
    tracing::info_span!(
        target: "mailbridge",
        "mailbridge.send",
        mailbridge_provider = provider,
        mailbridge_sender_domain = sender_domain,
        mailbridge_delivery_mode = delivery_mode,
    )
}

pub(crate) fn provider_send_span(provider: &str, sender_domain: &str) -> tracing::Span {
    tracing::info_span!(
        target: "mailbridge",
        "mailbridge.provider.send",
        mailbridge_provider = provider,
        mailbridge_sender_domain = sender_domain,
    )
}

pub(crate) fn queue_enqueue_span(queue_backend: &str, sender_domain: &str) -> tracing::Span {
    tracing::info_span!(
        target: "mailbridge",
        "mailbridge.queue.enqueue",
        mailbridge_queue_backend = queue_backend,
        mailbridge_sender_domain = sender_domain,
    )
}

pub(crate) fn queue_reserve_span(queue_backend: &str) -> tracing::Span {
    tracing::info_span!(
        target: "mailbridge",
        "mailbridge.queue.reserve",
        mailbridge_queue_backend = queue_backend,
    )
}

pub(crate) fn queue_process_span(queue_backend: &str, attempt_count: u32) -> tracing::Span {
    tracing::info_span!(
        target: "mailbridge",
        "mailbridge.queue.process",
        mailbridge_queue_backend = queue_backend,
        mailbridge_retry_attempt = attempt_count,
    )
}

pub(crate) fn queue_retry_span(
    queue_backend: &str,
    attempt_count: u32,
    retryable: bool,
) -> tracing::Span {
    tracing::info_span!(
        target: "mailbridge",
        "mailbridge.queue.retry",
        mailbridge_queue_backend = queue_backend,
        mailbridge_retry_attempt = attempt_count,
        mailbridge_retryable = retryable,
    )
}

pub(crate) fn queue_dead_letter_span(
    queue_backend: &str,
    attempt_count: u32,
    error_kind: &str,
) -> tracing::Span {
    tracing::info_span!(
        target: "mailbridge",
        "mailbridge.queue.dead_letter",
        mailbridge_queue_backend = queue_backend,
        mailbridge_retry_attempt = attempt_count,
        mailbridge_error_kind = error_kind,
    )
}
