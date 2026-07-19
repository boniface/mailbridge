# Telemetry

[README](../README.md) | [Usage](usage.md) | [Architecture](architecture-and-design.md) | [Telemetry](telemetry.md) | [Roadmap](ROADMAP.md) | [Changelog](CHANGELOG.md)

Mailbridge telemetry is an opt-in `tracing` surface for applications that want
email delivery activity in logs, traces, and OpenTelemetry pipelines.

Mailbridge does not configure global subscribers, OpenTelemetry SDKs,
collectors, exporters, resource attributes, or sampling policy. The consuming
application owns that setup. This keeps the library lightweight and prevents
Mailbridge from choosing deployment-specific observability behavior.

## Enable Telemetry

```toml
[dependencies]
mailbridge = { version = "0.3", features = ["telemetry"] }
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt", "json"] }
```

Minimal JSON logging setup:

```rust
use tracing_subscriber::{EnvFilter, fmt};

let filter = match EnvFilter::try_from_default_env() {
    Ok(filter) => filter,
    Err(_error) => EnvFilter::new("mailbridge=info"),
};

fmt().json().with_env_filter(filter).init();
```

Run the local no-credentials example:

```sh
cargo run --example opentelemetry_logging --features telemetry
```

## Events

Mailbridge emits low-cardinality events under the `mailbridge.*` namespace:

| Event | Meaning |
| --- | --- |
| `mailbridge.send.started` | A validated send attempt is starting. |
| `mailbridge.send.accepted` | The provider accepted the send request. |
| `mailbridge.send.failed` | The provider or validation path failed. |
| `mailbridge.queue.enqueued` | A message was accepted into a configured queue. |
| `mailbridge.queue.retry_scheduled` | A queued message was released for retry. |
| `mailbridge.queue.dead_lettered` | A queued message reached the dead-letter path. |
| `mailbridge.rate_limited` | `try_send` rejected a message because local capacity was unavailable. |

## Spans

When the `telemetry` feature is enabled, Mailbridge creates spans around major
operational boundaries:

| Span | Boundary |
| --- | --- |
| `mailbridge.send` | Client-level send operation. |
| `mailbridge.provider.send` | Provider request execution. |
| `mailbridge.queue.enqueue` | Queue enqueue operation. |
| `mailbridge.queue.reserve` | Queue reservation operation. |
| `mailbridge.queue.process` | Worker processing of one queued item. |
| `mailbridge.queue.retry` | Retry scheduling operation. |
| `mailbridge.queue.dead_letter` | Dead-letter write operation. |

These spans are children of whatever application span is active when
Mailbridge is called, so request handlers, jobs, and workers can correlate
email delivery work with the larger application flow.

## Attributes

Mailbridge uses stable, low-cardinality field names:

| Attribute | Description |
| --- | --- |
| `mailbridge_event` | Event name such as `mailbridge.send.accepted`. |
| `mailbridge_provider` | Provider name such as `hyvor-relay`, `sendgrid`, or `smtp`. |
| `mailbridge_sender_domain` | Sender domain only, never the full sender address. |
| `mailbridge_delivery_mode` | `send_now` or `queue`. |
| `mailbridge_queue_backend` | Queue backend such as `memory`, `sqlite`, `postgres`, or `scylla`. |
| `mailbridge_retry_attempt` | Retry attempt count. |
| `mailbridge_retryable` | Whether Mailbridge classified the error as retryable. |
| `mailbridge_status_code` | Provider HTTP status code when available. |
| `mailbridge_elapsed_ms` | Provider send elapsed time in milliseconds. |
| `mailbridge_error_kind` | Stable failure class such as `validation`, `authentication`, or `temporary`. |

## Error Kinds

Failure telemetry uses stable categories instead of raw error text:

| Error kind | Typical cause |
| --- | --- |
| `config` | Missing or invalid application configuration. |
| `validation` | Invalid message, address, body, header, or attachment input. |
| `sender_domain_not_allowed` | Sender domain failed the configured allow-list. |
| `rate_limited` | Local rate limiter rejected `try_send`. |
| `authentication` | Provider authentication failed. |
| `provider_rejected` | Provider rejected the request with an HTTP status. |
| `temporary` | Retryable provider or transport failure. |
| `queue` | Queue backend failure. |

## OpenTelemetry

To export Mailbridge telemetry through OpenTelemetry, configure the
application's normal `tracing` to OpenTelemetry bridge. Typical application
dependencies are:

```toml
[dependencies]
mailbridge = { version = "0.3", features = ["telemetry"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "registry"] }
tracing-opentelemetry = "0.33"
opentelemetry = "0.32"
opentelemetry_sdk = { version = "0.32", features = ["rt-tokio"] }
opentelemetry-otlp = "0.32"
```

The application should install its subscriber once at process startup, attach
Mailbridge spans to the current request or worker span, and send telemetry to
its collector or backend according to its own deployment policy.

## Safety Policy

Mailbridge telemetry may include:

- provider name;
- sender domain;
- delivery mode;
- queue backend;
- retry attempt count;
- retryable classification;
- provider status code;
- elapsed time;
- static event and span names;
- stable error category.

Mailbridge telemetry must not include:

- API keys;
- SMTP passwords;
- OAuth tokens;
- full sender or recipient email addresses;
- message subject, text, or HTML body;
- attachment names or content;
- raw provider response bodies;
- arbitrary email headers;
- tenant, user, or customer identifiers supplied by the application.

Application code may add its own spans and attributes around Mailbridge calls,
but it should apply the same privacy and cardinality rules.

## Metrics

Version 0.3.0 intentionally does not add a metrics API. Most counters and
latency views can be derived from the emitted events and spans by an
OpenTelemetry collector or observability backend.

Future metrics work should remain opt-in, avoid exporter dependencies in the
default build, and use low-cardinality labels that match the telemetry
attribute contract above.
