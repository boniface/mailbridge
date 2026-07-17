# Usage Guide

This guide covers the current public Mailbridge API for sending transactional
email from Rust services.

## Install

```toml
[dependencies]
mailbridge = "0.1"
```

Default features enable:

- Hyvor Relay HTTP sending;
- Rustls TLS;
- in-memory queueing;
- local rate limiting.

For SMTP:

```toml
[dependencies]
mailbridge = { version = "0.1", features = ["smtp"] }
```

For durable queues:

```toml
[dependencies]
mailbridge = { version = "0.1", features = ["queue-sqlite"] }
```

Use only the queue backend features your application needs.

## Environment Configuration

`MailbridgeConfig::from_env()` reads `RELAY_*` environment variables.

Required for Hyvor Relay HTTP:

```sh
RELAY_API_BASE_URL=https://relay.example.com/api/console
RELAY_API_KEY=your-api-key
RELAY_ALLOWED_FROM_DOMAINS=example.com,example.org
```

Optional default sender:

```sh
RELAY_DEFAULT_FROM_NAME=Example App
RELAY_DEFAULT_FROM_EMAIL=no-reply@example.com
```

Optional rate and retry settings:

```sh
RELAY_GLOBAL_RATE_PER_SECOND=50
RELAY_DOMAIN_RATE_PER_SECOND=10
RELAY_MAX_RETRIES=5
RELAY_RETRY_BASE_DELAY_MS=500
RELAY_REQUEST_TIMEOUT_SECS=15
```

Optional SMTP settings:

```sh
RELAY_SMTP_HOST=smtp.example.com
RELAY_SMTP_PORT=587
RELAY_SMTP_USERNAME=relay-user
RELAY_SMTP_PASSWORD=relay-password
```

Optional queue backend:

```sh
RELAY_QUEUE_BACKEND=memory
```

SQLite:

```sh
RELAY_QUEUE_BACKEND=sqlite
RELAY_QUEUE_SQLITE_PATH=./mailbridge-queue.sqlite
```

PostgreSQL:

```sh
RELAY_QUEUE_BACKEND=postgres
RELAY_QUEUE_POSTGRES_URL=postgres://user:password@localhost/mailbridge
```

ScyllaDB:

```sh
RELAY_QUEUE_BACKEND=scylla
RELAY_QUEUE_SCYLLA_URI=127.0.0.1:9042
RELAY_QUEUE_SCYLLA_KEYSPACE=mailbridge
RELAY_QUEUE_SCYLLA_TABLE=mail_queue
```

Keep secrets in `.env` or your deployment secret manager. Do not commit
credentials.

## Send With Hyvor Relay

```rust
use mailbridge::{EmailMessage, HyvorRelayProvider, MailClient, MailbridgeConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = MailbridgeConfig::from_env()?;
    let provider = HyvorRelayProvider::from_config(&config)?;
    let client = MailClient::try_from_config(provider, &config).await?;

    let message = EmailMessage::builder()
        .from("Example App", "no-reply@example.com")?
        .to("User", "user@example.net")?
        .subject("Welcome")
        .text("Thanks for signing up.")
        .build()?;

    let receipt = client.send(message).await?;
    println!("{}", receipt.message_id());

    Ok(())
}
```

Runnable example:

```sh
cargo run --example send_http
```

## Send HTML

```rust
let message = EmailMessage::builder()
    .from("Example App", "no-reply@example.com")?
    .to("User", "user@example.net")?
    .subject("Receipt")
    .text("Your receipt is attached.")
    .html("<p>Your receipt is attached.</p>")
    .build()?;
```

## Add CC, BCC, Headers, And Attachments

```rust
let message = EmailMessage::builder()
    .from("Example App", "no-reply@example.com")?
    .to("User", "user@example.net")?
    .cc("Support", "support@example.com")?
    .bcc("Audit", "audit@example.com")?
    .subject("Invoice")
    .text("Invoice attached.")
    .header("X-Request-Id", "req_123")?
    .attachment("invoice.txt", "text/plain", b"invoice contents".to_vec())?
    .build()?;
```

Mailbridge validates email addresses, message bodies, headers, attachments,
and allowed sender domains before making provider requests.

## Queue A Message

```rust
use mailbridge::{
    EmailMessage, HyvorRelayProvider, MailClient, MailbridgeConfig, QueueWorker,
    QueueWorkerConfig,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = MailbridgeConfig::from_env()?;
    let provider = HyvorRelayProvider::from_config(&config)?;
    let client = MailClient::try_from_config(provider, &config).await?;
    let queue = client
        .queue()
        .cloned()
        .unwrap_or_else(mailbridge::QueueHandle::memory_default);

    let worker = QueueWorker::new(
        client.clone().with_queue(queue.clone()),
        queue,
        QueueWorkerConfig::new("worker-1"),
    );

    let message = EmailMessage::builder()
        .from("Example App", "no-reply@example.com")?
        .to("User", "user@example.net")?
        .subject("Queued message")
        .text("This was queued first.")
        .build()?;

    client.enqueue(message).await?;
    worker.run_once().await?;

    Ok(())
}
```

Runnable example:

```sh
cargo run --example queue_worker
```

## Send With SMTP

Enable the `smtp` feature and provide SMTP configuration:

```toml
[dependencies]
mailbridge = { version = "0.1", features = ["smtp"] }
```

```sh
RELAY_SMTP_HOST=smtp.example.com
RELAY_SMTP_PORT=587
RELAY_SMTP_USERNAME=relay-user
RELAY_SMTP_PASSWORD=relay-password
```

Then build `SmtpClient` from the same config:

```rust
use mailbridge::{EmailMessage, MailClient, MailbridgeConfig, SmtpClient};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = MailbridgeConfig::from_env()?;
    let provider = SmtpClient::from_config(&config)?;
    let client = MailClient::try_from_config(provider, &config).await?;

    let message = EmailMessage::builder()
        .from("Example App", "no-reply@example.com")?
        .to("User", "user@example.net")?
        .subject("SMTP test")
        .text("Sent through SMTP.")
        .build()?;

    client.send(message).await?;
    Ok(())
}
```

Current SMTP support is generic username/password SMTP over STARTTLS. Gmail,
Microsoft 365, Yahoo, Yandex, OAuth/XOAUTH2, and provider-specific presets are
planned in the roadmap.

## Send With Optional HTTP Providers

Enable one of the provider feature flags and build the provider from its
configuration type:

```toml
[dependencies]
mailbridge = { version = "0.1", features = ["sendgrid"] }
```

SendGrid:

```sh
MAILBRIDGE_SENDGRID_API_KEY=sendgrid-api-key
MAILBRIDGE_SENDGRID_BASE_URL=https://api.sendgrid.com
```

```rust
use mailbridge::{MailClient, MailbridgeConfig, SendGridConfig, SendGridProvider};

let config = MailbridgeConfig::from_env()?;
let provider_config = SendGridConfig::from_env(&config)?;
let provider = SendGridProvider::from_config(&provider_config)?;
let client = MailClient::try_from_config(provider, &config).await?;
```

Mailgun:

```sh
MAILBRIDGE_MAILGUN_API_KEY=mailgun-api-key
MAILBRIDGE_MAILGUN_DOMAIN=mg.example.com
MAILBRIDGE_MAILGUN_BASE_URL=https://api.mailgun.net
```

```rust
use mailbridge::{MailClient, MailbridgeConfig, MailgunConfig, MailgunProvider};

let config = MailbridgeConfig::from_env()?;
let provider_config = MailgunConfig::from_env(&config)?;
let provider = MailgunProvider::from_config(&provider_config)?;
let client = MailClient::try_from_config(provider, &config).await?;
```

SendPulse:

```sh
MAILBRIDGE_SENDPULSE_CLIENT_ID=sendpulse-client-id
MAILBRIDGE_SENDPULSE_CLIENT_SECRET=sendpulse-client-secret
MAILBRIDGE_SENDPULSE_BASE_URL=https://api.sendpulse.com
```

For deployments that already manage SendPulse tokens externally, use
`MAILBRIDGE_SENDPULSE_ACCESS_TOKEN` instead of client credentials.

```rust
use mailbridge::{MailClient, MailbridgeConfig, SendPulseConfig, SendPulseProvider};

let config = MailbridgeConfig::from_env()?;
let provider_config = SendPulseConfig::from_env(&config)?;
let provider = SendPulseProvider::from_config(&provider_config)?;
let client = MailClient::try_from_config(provider, &config).await?;
```

## Feature Flags

Common features:

```text
api              HTTP provider support through reqwest
hyvor-relay      Hyvor Relay provider
sendgrid         SendGrid HTTP provider
mailgun          Mailgun HTTP provider
sendpulse        SendPulse SMTP API provider
smtp             SMTP provider through lettre
rustls           Rustls TLS backend
native-tls       Native TLS backend
queue-memory     in-memory queue
queue-sqlite     SQLite queue backend
queue-postgres   PostgreSQL queue backend
queue-scylla     ScyllaDB queue backend
rate-limit       local rate limiting
telemetry        tracing events
dotenv           load .env before reading RELAY_* variables
```

`rustls` and `native-tls` are mutually exclusive.

Mailbridge uses `reqwest` 0.13 for HTTP providers. The `rustls` feature maps to
`reqwest/rustls`, while `native-tls` maps to `reqwest/native-tls`. Default
features use Rustls so applications get a pure-Rust TLS stack for Relay HTTP
calls unless they explicitly opt into `native-tls`.

Queue backends using SQLx 0.9 enable `sqlx/runtime-tokio` and `sqlx/tls-rustls`
through the Mailbridge queue feature flags. Because SQLx 0.9 requires Rust
1.94, Mailbridge's crate-level MSRV is Rust 1.94 or newer.

## Error Handling

Most APIs return `mailbridge::Result<T>`, using `MailError` for validation,
configuration, provider rejection, and temporary transport failures.

Queue workers use retry classification to decide whether a failed message can
be retried or should be dead-lettered.

## Safety Notes

- Keep `.env` out of Git.
- Use allowed sender domains to prevent accidental spoofing.
- Do not log API keys, SMTP passwords, OAuth tokens, message bodies, attachment
  content, or full recipient lists.
- Prefer dedicated transactional relays for production application email.
  Personal mailbox SMTP is planned, but it is not a replacement for a relay at
  transactional volume.
