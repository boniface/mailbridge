//! Provider-neutral transactional email client.
//!
//! # Example
//!
//! ```ignore
//! use mailbridge::{EmailMessage, HyvorRelayProvider, MailClient, MailbridgeConfig};
//!
//! # async fn send_email() -> anyhow::Result<()> {
//! let config = MailbridgeConfig::from_env()?;
//! let provider = HyvorRelayProvider::from_config(&config)?;
//! let client = MailClient::try_from_config(provider, &config).await?;
//!
//! let message = EmailMessage::builder()
//!     .from("Hashcode", "no-reply@hashcode-fibration.com")?
//!     .to("User", "user@example.com")?
//!     .subject("Relay test")
//!     .text("This is a test email.")
//!     .build()?;
//!
//! let receipt = client.send(message).await?;
//! println!("{}", receipt.message_id());
//! # Ok(())
//! # }
//! ```
//!
//! # Queue Backends
//!
//! `queue-memory` is intended for tests, local development, and workloads that can
//! tolerate losing queued mail on process restart. Use `queue-sqlite` for
//! single-node durable queueing and `queue-postgres` for multi-worker durable
//! queueing in applications that already operate `PostgreSQL`. Use
//! `queue-scylla` for high-throughput `ScyllaDB` deployments where bucket count,
//! lease timeout, replication, compaction, and dead-letter retention are tuned
//! by the owning application.
//!
//! Optional provider flags (`sendgrid`, `sendpulse`, `mailgun`, `resend`,
//! `mailjet`, `brevo`, and `bird`) add HTTP provider implementations for those
//! transactional email services.
//!
//! Live durable-backend tests are environment gated:
//!
//! - `MAILBRIDGE_TEST_POSTGRES_URL` for `PostgreSQL`.
//! - `MAILBRIDGE_TEST_SCYLLA_URI`, `MAILBRIDGE_TEST_SCYLLA_KEYSPACE`, and
//!   `MAILBRIDGE_TEST_SCYLLA_TABLE` for `ScyllaDB`.

#[cfg(all(feature = "rustls", feature = "native-tls"))]
compile_error!("features `rustls` and `native-tls` cannot be enabled together");

mod client;
mod config;
mod email;
mod error;
mod provider;
mod queue;
mod rate_limit;
mod smtp;
mod telemetry;

pub use client::{DeliveryMode, MailClient, MailClientBuilder, MessageId, SendReceipt};
pub use config::{MailbridgeConfig, MailbridgeConfigBuilder, QueueBackend, SmtpConfig};
pub use email::{Attachment, EmailAddress, EmailMessage, EmailMessageBuilder};
pub use error::{MailError, Result};
pub use provider::{MailProvider, ProviderCapabilities, SendStatus};
#[cfg(feature = "queue-postgres")]
pub use queue::PostgresQueue;
#[cfg(feature = "queue-scylla")]
pub use queue::ScyllaQueue;
#[cfg(feature = "queue-sqlite")]
pub use queue::SqliteQueue;
pub use queue::{
    MailQueue, QueueHandle, QueueId, QueueItem, QueueWorker, QueueWorkerConfig, QueuedEmail,
};
pub use rate_limit::RateLimitConfig;

#[cfg(feature = "rate-limit")]
pub use rate_limit::MailRateLimiter;
pub use smtp::SmtpClient;
pub use telemetry::{TelemetryEvent, TelemetryFields};

#[cfg(all(feature = "hyvor-relay", feature = "api"))]
pub use provider::HyvorRelayProvider;
#[cfg(feature = "bird")]
pub use provider::{BirdConfig, BirdProvider};
#[cfg(feature = "brevo")]
pub use provider::{BrevoConfig, BrevoProvider};
#[cfg(feature = "mailgun")]
pub use provider::{MailgunConfig, MailgunProvider};
#[cfg(feature = "mailjet")]
pub use provider::{MailjetApiVersion, MailjetConfig, MailjetProvider};
#[cfg(feature = "resend")]
pub use provider::{ResendConfig, ResendProvider};
#[cfg(feature = "sendgrid")]
pub use provider::{SendGridConfig, SendGridProvider};
#[cfg(feature = "sendpulse")]
pub use provider::{SendPulseConfig, SendPulseProvider};
