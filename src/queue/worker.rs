use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use crate::client::MailClient;
use crate::error::Result;
use crate::provider::MailProvider;
use crate::queue::{MailQueue, QueueHandle, QueuedEmail};
use crate::telemetry::{TelemetryEvent, TelemetryFields, emit};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueWorkerConfig {
    worker_id: String,
    batch_size: u32,
    max_retries: u32,
    retry_base_delay: Duration,
    idle_delay: Duration,
}

impl QueueWorkerConfig {
    #[must_use]
    pub fn new(worker_id: impl Into<String>) -> Self {
        Self {
            worker_id: worker_id.into(),
            batch_size: 10,
            max_retries: 5,
            retry_base_delay: Duration::from_millis(500),
            idle_delay: Duration::from_millis(250),
        }
    }

    #[must_use]
    pub const fn batch_size(mut self, value: u32) -> Self {
        self.batch_size = value;
        self
    }

    #[must_use]
    pub const fn max_retries(mut self, value: u32) -> Self {
        self.max_retries = value;
        self
    }

    #[must_use]
    pub const fn retry_base_delay(mut self, value: Duration) -> Self {
        self.retry_base_delay = value;
        self
    }

    #[must_use]
    pub const fn idle_delay(mut self, value: Duration) -> Self {
        self.idle_delay = value;
        self
    }

    #[must_use]
    pub fn worker_id(&self) -> &str {
        &self.worker_id
    }
}

#[derive(Debug, Clone)]
pub struct QueueWorker<P> {
    client: MailClient<P>,
    queue: QueueHandle,
    config: QueueWorkerConfig,
}

impl<P> QueueWorker<P>
where
    P: MailProvider,
{
    #[must_use]
    pub fn new(client: MailClient<P>, queue: QueueHandle, config: QueueWorkerConfig) -> Self {
        Self {
            client,
            queue,
            config,
        }
    }

    /// Processes one queue batch.
    ///
    /// # Errors
    ///
    /// Returns provider or queue backend errors encountered while reserving,
    /// sending, retrying, or dead-lettering messages.
    pub async fn run_once(&self) -> Result<usize> {
        let batch = self
            .queue
            .reserve_batch(self.config.worker_id(), self.config.batch_size)
            .await?;
        let count = batch.len();

        for queued in batch {
            self.process_one(queued).await?;
        }

        Ok(count)
    }

    /// Processes queue batches until shutdown is requested.
    ///
    /// # Errors
    ///
    /// Returns provider or queue backend errors encountered during processing.
    pub async fn run_until_shutdown(
        &self,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) -> Result<()> {
        while !*shutdown.borrow() {
            let processed = self.run_once().await?;
            if processed == 0 {
                tokio::select! {
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() {
                            break;
                        }
                    }
                    () = tokio::time::sleep(self.config.idle_delay) => {}
                }
            }
        }

        Ok(())
    }

    async fn process_one(&self, queued: QueuedEmail) -> Result<()> {
        let id = queued.id().clone();
        let attempt_count = queued.attempt_count();

        match self.client.send(queued.into_message()).await {
            Ok(receipt) => self.queue.mark_sent(&id, &receipt).await,
            Err(error) if error.is_retryable() && attempt_count < self.config.max_retries => {
                emit(
                    TelemetryEvent::QueueRetryScheduled,
                    &TelemetryFields::new().attempt_count(attempt_count.saturating_add(1)),
                );
                self.queue
                    .release_for_retry(
                        &id,
                        retry_delay(self.config.retry_base_delay, attempt_count, &id),
                        &error,
                    )
                    .await
            }
            Err(error) => {
                emit(
                    TelemetryEvent::QueueDeadLettered,
                    &TelemetryFields::new().attempt_count(attempt_count),
                );
                self.queue.dead_letter(&id, &error).await
            }
        }
    }
}

fn retry_delay(base: Duration, attempt_count: u32, id: &crate::queue::QueueId) -> Duration {
    let multiplier = 1_u32.checked_shl(attempt_count.min(16)).unwrap_or(u32::MAX);
    base.saturating_mul(multiplier)
        .saturating_add(jitter(base, id))
}

fn jitter(base: Duration, id: &crate::queue::QueueId) -> Duration {
    if base.is_zero() {
        return Duration::ZERO;
    }

    let mut hasher = DefaultHasher::new();
    id.hash(&mut hasher);
    let jitter_ms = hasher.finish() % u64::try_from(base.as_millis()).unwrap_or(u64::MAX).max(1);
    Duration::from_millis(jitter_ms)
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::*;
    use crate::client::{MessageId, SendReceipt};
    use crate::email::EmailMessage;
    use crate::error::{MailError, Result};
    use crate::provider::SendStatus;

    #[derive(Debug)]
    struct AlwaysTemporary;

    #[async_trait]
    impl MailProvider for AlwaysTemporary {
        async fn send(&self, _message: &EmailMessage) -> Result<SendReceipt> {
            Err(MailError::Temporary("timeout".to_owned()))
        }

        async fn get_status(&self, _id: &MessageId) -> Result<Option<SendStatus>> {
            Ok(None)
        }

        fn provider_name(&self) -> &'static str {
            "temporary"
        }
    }

    #[tokio::test]
    async fn worker_dead_letters_after_retry_exhaustion() {
        let queue = QueueHandle::memory(4);
        let client = MailClient::new(AlwaysTemporary).with_queue(queue.clone());
        let worker = QueueWorker::new(
            client.clone(),
            queue.clone(),
            QueueWorkerConfig::new("worker").max_retries(0),
        );

        client
            .enqueue(test_message())
            .await
            .expect("enqueue should succeed");

        let processed = worker.run_once().await.expect("worker succeeds");

        assert_eq!(processed, 1);
        assert_eq!(queue.dead_letter_len().await.expect("len"), 1);
    }

    fn test_message() -> EmailMessage {
        EmailMessage::builder()
            .from("App", "sender@example.com")
            .expect("valid from")
            .to("User", "user@example.net")
            .expect("valid to")
            .subject("Hello")
            .text("Body")
            .build()
            .expect("valid message")
    }
}
