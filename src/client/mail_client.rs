use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::config::MailbridgeConfig;
use crate::email::EmailMessage;
use crate::error::{MailError, Result};
use crate::provider::MailProvider;
use crate::queue::{MailQueue, QueueHandle, QueueId, QueueItem};
use crate::telemetry::{TelemetryEvent, TelemetryFields, emit};

#[cfg(feature = "rate-limit")]
use crate::rate_limit::MailRateLimiter;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(String);

impl MessageId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for MessageId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendReceipt {
    provider: &'static str,
    message_id: MessageId,
    provider_id: Option<String>,
}

impl SendReceipt {
    #[must_use]
    pub fn new(provider: &'static str, message_id: MessageId, provider_id: Option<String>) -> Self {
        Self {
            provider,
            message_id,
            provider_id,
        }
    }

    #[must_use]
    pub fn provider(&self) -> &'static str {
        self.provider
    }

    #[must_use]
    pub fn message_id(&self) -> &MessageId {
        &self.message_id
    }

    #[must_use]
    pub fn provider_id(&self) -> Option<&str> {
        self.provider_id.as_deref()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeliveryMode {
    #[default]
    SendNow,
    Queue,
}

#[derive(Debug)]
pub struct MailClient<P> {
    provider: Arc<P>,
    allowed_from_domains: BTreeSet<String>,
    queue: Option<QueueHandle>,
    #[cfg(feature = "rate-limit")]
    rate_limiter: Option<MailRateLimiter>,
}

impl<P> Clone for MailClient<P> {
    fn clone(&self) -> Self {
        Self {
            provider: Arc::clone(&self.provider),
            allowed_from_domains: self.allowed_from_domains.clone(),
            queue: self.queue.clone(),
            #[cfg(feature = "rate-limit")]
            rate_limiter: self.rate_limiter.clone(),
        }
    }
}

impl<P> MailClient<P> {
    #[must_use]
    pub fn new(provider: P) -> Self {
        Self {
            provider: Arc::new(provider),
            allowed_from_domains: BTreeSet::new(),
            queue: None,
            #[cfg(feature = "rate-limit")]
            rate_limiter: None,
        }
    }

    pub fn from_config(provider: P, config: &MailbridgeConfig) -> Result<Self> {
        if !matches!(config.queue_backend(), crate::config::QueueBackend::Memory) {
            return Err(MailError::Config(
                "MailClient::from_config only supports the memory queue backend; use MailClient::try_from_config for durable queue backends"
                    .to_owned(),
            ));
        }

        let builder = MailClientBuilder::new(provider).allowed_from_domains(
            config
                .allowed_from_domains()
                .iter()
                .map(std::string::ToString::to_string),
        );

        #[cfg(feature = "rate-limit")]
        let builder = builder.rate_limiter(MailRateLimiter::new(
            config.rate_limit(),
            config
                .allowed_from_domains()
                .iter()
                .map(std::string::ToString::to_string),
        ));

        #[cfg(feature = "queue-memory")]
        let builder = if matches!(config.queue_backend(), crate::config::QueueBackend::Memory) {
            builder.queue(QueueHandle::memory_default())
        } else {
            builder
        };

        Ok(builder.build())
    }

    pub async fn try_from_config(provider: P, config: &MailbridgeConfig) -> Result<Self> {
        let builder = MailClientBuilder::new(provider).allowed_from_domains(
            config
                .allowed_from_domains()
                .iter()
                .map(std::string::ToString::to_string),
        );

        #[cfg(feature = "rate-limit")]
        let builder = builder.rate_limiter(MailRateLimiter::new(
            config.rate_limit(),
            config
                .allowed_from_domains()
                .iter()
                .map(std::string::ToString::to_string),
        ));

        let queue = QueueHandle::from_backend(config.queue_backend()).await?;
        Ok(builder.queue(queue).build())
    }

    #[must_use]
    pub fn provider(&self) -> &P {
        self.provider.as_ref()
    }

    #[must_use]
    pub fn with_allowed_from_domains(mut self, domains: impl IntoIterator<Item = String>) -> Self {
        self.allowed_from_domains = domains.into_iter().collect();
        self
    }

    #[must_use]
    pub fn with_queue(mut self, queue: QueueHandle) -> Self {
        self.queue = Some(queue);
        self
    }

    #[cfg(feature = "rate-limit")]
    #[must_use]
    pub fn with_rate_limiter(mut self, limiter: MailRateLimiter) -> Self {
        self.rate_limiter = Some(limiter);
        self
    }

    #[must_use]
    pub fn queue(&self) -> Option<&QueueHandle> {
        self.queue.as_ref()
    }
}

impl<P> MailClient<P>
where
    P: MailProvider,
{
    pub async fn send(&self, message: EmailMessage) -> Result<SendReceipt> {
        self.validate(&message)?;
        self.wait_for_rate_limit(&message).await;

        let started = Instant::now();
        emit(
            TelemetryEvent::SendStarted,
            &TelemetryFields::new()
                .domain(message.from_address().domain())
                .provider(self.provider.provider_name()),
        );
        let result = self.provider.send(&message).await;

        emit(
            if result.is_ok() {
                TelemetryEvent::SendAccepted
            } else {
                TelemetryEvent::SendFailed
            },
            &TelemetryFields::new()
                .domain(message.from_address().domain())
                .provider(self.provider.provider_name())
                .elapsed_ms(started.elapsed().as_millis()),
        );

        result
    }

    pub async fn try_send(&self, message: EmailMessage) -> Result<SendReceipt> {
        self.validate(&message)?;
        if let Err(error) = self.check_rate_limit(&message) {
            emit(
                TelemetryEvent::RateLimited,
                &TelemetryFields::new()
                    .domain(message.from_address().domain())
                    .provider(self.provider.provider_name()),
            );
            return Err(error);
        }

        let started = Instant::now();
        emit(
            TelemetryEvent::SendStarted,
            &TelemetryFields::new()
                .domain(message.from_address().domain())
                .provider(self.provider.provider_name()),
        );
        let result = self.provider.send(&message).await;
        emit(
            if result.is_ok() {
                TelemetryEvent::SendAccepted
            } else {
                TelemetryEvent::SendFailed
            },
            &TelemetryFields::new()
                .domain(message.from_address().domain())
                .provider(self.provider.provider_name())
                .elapsed_ms(started.elapsed().as_millis()),
        );

        result
    }

    pub async fn enqueue(&self, message: EmailMessage) -> Result<QueueId> {
        self.validate(&message)?;
        let domain = message.from_address().domain().to_owned();
        let queue = self
            .queue
            .as_ref()
            .ok_or_else(|| MailError::Queue("mail queue is not configured".to_owned()))?;

        let id = queue.enqueue(QueueItem::new(message)).await?;
        emit(
            TelemetryEvent::QueueEnqueued,
            &TelemetryFields::new().domain(&domain),
        );

        Ok(id)
    }

    fn validate(&self, message: &EmailMessage) -> Result<()> {
        message.validate()?;

        if self.allowed_from_domains.is_empty() {
            return Ok(());
        }

        message.validate_sender_domain(&self.allowed_from_domains)
    }

    #[cfg(feature = "rate-limit")]
    async fn wait_for_rate_limit(&self, message: &EmailMessage) {
        if let Some(limiter) = &self.rate_limiter {
            limiter.wait(message.from_address().domain()).await;
        }
    }

    #[cfg(not(feature = "rate-limit"))]
    async fn wait_for_rate_limit(&self, _message: &EmailMessage) {}

    #[cfg(feature = "rate-limit")]
    fn check_rate_limit(&self, message: &EmailMessage) -> Result<()> {
        self.rate_limiter.as_ref().map_or(Ok(()), |limiter| {
            limiter.check(message.from_address().domain())
        })
    }

    #[cfg(not(feature = "rate-limit"))]
    fn check_rate_limit(&self, _message: &EmailMessage) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
pub struct MailClientBuilder<P> {
    provider: P,
    allowed_from_domains: BTreeSet<String>,
    queue: Option<QueueHandle>,
    #[cfg(feature = "rate-limit")]
    rate_limiter: Option<MailRateLimiter>,
}

impl<P> MailClientBuilder<P> {
    #[must_use]
    pub fn new(provider: P) -> Self {
        Self {
            provider,
            allowed_from_domains: BTreeSet::new(),
            queue: None,
            #[cfg(feature = "rate-limit")]
            rate_limiter: None,
        }
    }

    #[must_use]
    pub fn allowed_from_domains(mut self, domains: impl IntoIterator<Item = String>) -> Self {
        self.allowed_from_domains = domains.into_iter().collect();
        self
    }

    #[must_use]
    pub fn queue(mut self, queue: QueueHandle) -> Self {
        self.queue = Some(queue);
        self
    }

    #[cfg(feature = "rate-limit")]
    #[must_use]
    pub fn rate_limiter(mut self, limiter: MailRateLimiter) -> Self {
        self.rate_limiter = Some(limiter);
        self
    }

    #[must_use]
    pub fn build(self) -> MailClient<P> {
        MailClient {
            provider: Arc::new(self.provider),
            allowed_from_domains: self.allowed_from_domains,
            queue: self.queue,
            #[cfg(feature = "rate-limit")]
            rate_limiter: self.rate_limiter,
        }
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::*;
    use crate::email::EmailMessage;
    use crate::error::MailError;
    use crate::provider::SendStatus;

    #[derive(Debug, Clone)]
    struct MockProvider;

    #[async_trait]
    impl MailProvider for MockProvider {
        async fn send(&self, message: &EmailMessage) -> Result<SendReceipt> {
            Ok(SendReceipt::new(
                self.provider_name(),
                MessageId::new(message.subject()),
                None,
            ))
        }

        async fn get_status(&self, _id: &MessageId) -> Result<Option<SendStatus>> {
            Ok(None)
        }

        fn provider_name(&self) -> &'static str {
            "mock"
        }
    }

    #[tokio::test]
    async fn send_rejects_disallowed_sender_domain_before_provider_call() {
        let message = EmailMessage::builder()
            .from("App", "sender@example.com")
            .expect("valid from address")
            .to("User", "user@example.net")
            .expect("valid to address")
            .subject("hello")
            .text("body")
            .build()
            .expect("valid message");
        let config = MailbridgeConfig::builder()
            .api_base_url("https://relay.example.com/api/console")
            .expect("valid url")
            .api_key("secret")
            .allowed_from_domain("allowed.example")
            .build()
            .expect("valid config");
        let client = MailClient::from_config(MockProvider, &config).expect("client builds");

        let error = client
            .send(message)
            .await
            .expect_err("sender domain should be rejected");

        assert_eq!(
            error,
            MailError::SenderDomainNotAllowed {
                domain: "example.com".to_owned()
            }
        );
    }

    #[tokio::test]
    async fn enqueue_requires_configured_queue() {
        let message = EmailMessage::builder()
            .from("App", "sender@example.com")
            .expect("valid from address")
            .to("User", "user@example.net")
            .expect("valid to address")
            .subject("hello")
            .text("body")
            .build()
            .expect("valid message");
        let client = MailClient::new(MockProvider);

        let error = client
            .enqueue(message)
            .await
            .expect_err("missing queue should fail");

        assert_eq!(
            error,
            MailError::Queue("mail queue is not configured".to_owned())
        );
    }

    #[tokio::test]
    async fn enqueue_uses_configured_queue() {
        let message = EmailMessage::builder()
            .from("App", "sender@example.com")
            .expect("valid from address")
            .to("User", "user@example.net")
            .expect("valid to address")
            .subject("hello")
            .text("body")
            .build()
            .expect("valid message");
        let queue = QueueHandle::memory(2);
        let client = MailClient::new(MockProvider).with_queue(queue);

        let id = client.enqueue(message).await.expect("enqueue succeeds");

        assert!(!id.as_str().is_empty());
    }

    #[cfg(feature = "queue-postgres")]
    #[test]
    fn from_config_rejects_durable_queue_backends() {
        let config = MailbridgeConfig::builder()
            .api_base_url("https://relay.example.com/api/console")
            .expect("valid url")
            .api_key("secret")
            .allowed_from_domain("example.com")
            .queue_backend(crate::config::QueueBackend::Postgres {
                database_url: secrecy::SecretString::new(
                    "postgres://localhost/mailbridge"
                        .to_owned()
                        .into_boxed_str(),
                ),
            })
            .build()
            .expect("valid config");

        let error = MailClient::from_config(MockProvider, &config)
            .expect_err("durable backend should require async constructor");

        assert!(matches!(error, MailError::Config(message) if message.contains("try_from_config")));
    }
}
