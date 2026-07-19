use std::any::Any;
use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::client::SendReceipt;
use crate::config::QueueBackend;
use crate::email::EmailMessage;
use crate::error::{MailError, Result};

#[cfg(feature = "queue-postgres")]
use secrecy::{ExposeSecret, SecretString};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QueueId(String);

impl QueueId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn new_uuid() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for QueueId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueItem {
    message: EmailMessage,
    idempotency_key: Option<String>,
}

impl QueueItem {
    #[must_use]
    pub fn new(message: EmailMessage) -> Self {
        Self {
            message,
            idempotency_key: None,
        }
    }

    #[must_use]
    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    #[must_use]
    pub fn into_message(self) -> EmailMessage {
        match self.idempotency_key {
            Some(key) => self.message.with_idempotency_key(key),
            None => self.message.ensure_idempotency_key(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedEmail {
    id: QueueId,
    message: EmailMessage,
    attempt_count: u32,
}

impl QueuedEmail {
    #[must_use]
    pub fn new(id: QueueId, message: EmailMessage, attempt_count: u32) -> Self {
        Self {
            id,
            message,
            attempt_count,
        }
    }

    #[must_use]
    pub fn id(&self) -> &QueueId {
        &self.id
    }

    #[must_use]
    pub fn message(&self) -> &EmailMessage {
        &self.message
    }

    #[must_use]
    pub const fn attempt_count(&self) -> u32 {
        self.attempt_count
    }

    #[must_use]
    pub fn into_message(self) -> EmailMessage {
        self.message
    }
}

#[async_trait]
pub trait MailQueue: Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn backend_name(&self) -> &'static str {
        "custom"
    }

    async fn enqueue(&self, item: QueueItem) -> Result<QueueId>;
    async fn reserve_batch(&self, worker_id: &str, limit: u32) -> Result<Vec<QueuedEmail>>;
    async fn mark_sent(&self, id: &QueueId, receipt: &SendReceipt) -> Result<()>;
    async fn release_for_retry(
        &self,
        id: &QueueId,
        delay: Duration,
        error: &MailError,
    ) -> Result<()>;
    async fn dead_letter(&self, id: &QueueId, error: &MailError) -> Result<()>;
    async fn pending_len(&self) -> Result<usize>;
    async fn dead_letter_len(&self) -> Result<usize>;
}

#[derive(Clone)]
pub struct QueueHandle {
    inner: Arc<dyn MailQueue>,
}

impl fmt::Debug for QueueHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("QueueHandle")
            .finish_non_exhaustive()
    }
}

impl QueueHandle {
    #[must_use]
    pub fn new(queue: impl MailQueue + 'static) -> Self {
        Self {
            inner: Arc::new(queue),
        }
    }

    #[must_use]
    pub fn from_arc(queue: Arc<dyn MailQueue>) -> Self {
        Self { inner: queue }
    }

    #[must_use]
    pub fn memory(capacity: usize) -> Self {
        Self::new(MemoryQueue::new(capacity))
    }

    #[must_use]
    pub fn memory_default() -> Self {
        Self::memory(1024)
    }

    /// Builds a queue handle for a configured backend.
    ///
    /// # Errors
    ///
    /// Returns an error when the backend feature is disabled or the durable
    /// backend cannot be initialized.
    pub async fn from_backend(backend: &QueueBackend) -> Result<Self> {
        match backend {
            QueueBackend::Memory => Ok(Self::memory_default()),
            QueueBackend::Sqlite { path } => {
                #[cfg(feature = "queue-sqlite")]
                {
                    crate::queue::SqliteQueue::connect(path.clone())
                        .await
                        .map(crate::queue::SqliteQueue::handle)
                }
                #[cfg(not(feature = "queue-sqlite"))]
                {
                    let _ = path;
                    Err(MailError::Config(
                        "queue backend sqlite requires the queue-sqlite feature".to_owned(),
                    ))
                }
            }
            QueueBackend::Postgres { database_url } => {
                #[cfg(feature = "queue-postgres")]
                {
                    crate::queue::PostgresQueue::connect(secret_copy(database_url))
                        .await
                        .map(crate::queue::PostgresQueue::handle)
                }
                #[cfg(not(feature = "queue-postgres"))]
                {
                    let _ = database_url;
                    Err(MailError::Config(
                        "queue backend postgres requires the queue-postgres feature".to_owned(),
                    ))
                }
            }
            QueueBackend::Scylla {
                uri,
                keyspace,
                table,
            } => {
                #[cfg(feature = "queue-scylla")]
                {
                    Box::pin(crate::queue::ScyllaQueue::connect(
                        uri.clone(),
                        keyspace.clone(),
                        table.clone(),
                    ))
                    .await
                    .map(crate::queue::ScyllaQueue::handle)
                }
                #[cfg(not(feature = "queue-scylla"))]
                {
                    let _ = (uri, keyspace, table);
                    Err(MailError::Config(
                        "queue backend scylla requires the queue-scylla feature".to_owned(),
                    ))
                }
            }
        }
    }

    /// Returns the number of pending messages.
    ///
    /// # Errors
    ///
    /// Returns an error when the backing queue cannot count pending messages.
    pub async fn pending_len(&self) -> Result<usize> {
        self.inner.pending_len().await
    }

    /// Returns the number of dead-lettered messages.
    ///
    /// # Errors
    ///
    /// Returns an error when the backing queue cannot count dead letters.
    pub async fn dead_letter_len(&self) -> Result<usize> {
        self.inner.dead_letter_len().await
    }

    #[must_use]
    pub fn is_memory(&self) -> bool {
        self.inner.as_any().is::<MemoryQueue>()
    }

    #[must_use]
    pub fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }
}

#[cfg(feature = "queue-postgres")]
fn secret_copy(secret: &SecretString) -> SecretString {
    SecretString::new(secret.expose_secret().to_owned().into_boxed_str())
}

impl Default for QueueHandle {
    fn default() -> Self {
        Self::memory_default()
    }
}

#[async_trait]
impl MailQueue for QueueHandle {
    fn as_any(&self) -> &dyn Any {
        self.inner.as_any()
    }

    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }

    async fn enqueue(&self, item: QueueItem) -> Result<QueueId> {
        self.inner.enqueue(item).await
    }

    async fn reserve_batch(&self, worker_id: &str, limit: u32) -> Result<Vec<QueuedEmail>> {
        self.inner.reserve_batch(worker_id, limit).await
    }

    async fn mark_sent(&self, id: &QueueId, receipt: &SendReceipt) -> Result<()> {
        self.inner.mark_sent(id, receipt).await
    }

    async fn release_for_retry(
        &self,
        id: &QueueId,
        delay: Duration,
        error: &MailError,
    ) -> Result<()> {
        self.inner.release_for_retry(id, delay, error).await
    }

    async fn dead_letter(&self, id: &QueueId, error: &MailError) -> Result<()> {
        self.inner.dead_letter(id, error).await
    }

    async fn pending_len(&self) -> Result<usize> {
        self.inner.pending_len().await
    }

    async fn dead_letter_len(&self) -> Result<usize> {
        self.inner.dead_letter_len().await
    }
}

#[derive(Debug)]
struct MemoryQueue {
    capacity: usize,
    state: Mutex<MemoryQueueState>,
}

impl MemoryQueue {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            state: Mutex::new(MemoryQueueState::default()),
        }
    }
}

#[async_trait]
impl MailQueue for MemoryQueue {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn backend_name(&self) -> &'static str {
        "memory"
    }

    async fn enqueue(&self, item: QueueItem) -> Result<QueueId> {
        let mut state = self.state.lock().await;
        if state.pending.len() >= self.capacity {
            return Err(MailError::Queue("memory queue is full".to_owned()));
        }

        let id = QueueId::new_uuid();
        let queued = PendingEmail {
            id: id.clone(),
            message: item.into_message(),
            available_at: Instant::now(),
            attempt_count: 0,
            locked_by: None,
            last_error: None,
        };
        state.pending.push_back(queued);

        Ok(id)
    }

    async fn reserve_batch(&self, worker_id: &str, limit: u32) -> Result<Vec<QueuedEmail>> {
        let mut state = self.state.lock().await;
        let now = Instant::now();
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        let mut reserved = Vec::with_capacity(limit.min(state.pending.len()));

        let mut remaining = VecDeque::with_capacity(state.pending.len());
        while let Some(mut pending) = state.pending.pop_front() {
            if reserved.len() < limit && pending.available_at <= now && pending.locked_by.is_none()
            {
                pending.locked_by = Some(worker_id.to_owned());
                reserved.push(QueuedEmail::new(
                    pending.id.clone(),
                    pending.message.clone(),
                    pending.attempt_count,
                ));
                state.inflight.push(pending);
            } else {
                remaining.push_back(pending);
            }
        }

        state.pending = remaining;
        Ok(reserved)
    }

    async fn mark_sent(&self, id: &QueueId, _receipt: &SendReceipt) -> Result<()> {
        let mut state = self.state.lock().await;
        state.inflight.retain(|item| item.id != *id);
        Ok(())
    }

    async fn release_for_retry(
        &self,
        id: &QueueId,
        delay: Duration,
        error: &MailError,
    ) -> Result<()> {
        let mut state = self.state.lock().await;
        let Some(index) = state.inflight.iter().position(|item| item.id == *id) else {
            return Err(MailError::Queue(format!("queued message not found: {id}")));
        };

        let mut item = state.inflight.remove(index);
        item.attempt_count = item.attempt_count.saturating_add(1);
        item.available_at = Instant::now() + delay;
        item.locked_by = None;
        item.last_error = Some(error.to_string());
        state.pending.push_back(item);

        Ok(())
    }

    async fn dead_letter(&self, id: &QueueId, error: &MailError) -> Result<()> {
        let mut state = self.state.lock().await;
        let Some(index) = state.inflight.iter().position(|item| item.id == *id) else {
            return Err(MailError::Queue(format!("queued message not found: {id}")));
        };

        let mut item = state.inflight.remove(index);
        item.last_error = Some(error.to_string());
        state.dead_letters.push(item);

        Ok(())
    }

    async fn pending_len(&self) -> Result<usize> {
        Ok(self.state.lock().await.pending.len())
    }

    async fn dead_letter_len(&self) -> Result<usize> {
        Ok(self.state.lock().await.dead_letters.len())
    }
}

#[derive(Debug, Default)]
struct MemoryQueueState {
    pending: VecDeque<PendingEmail>,
    inflight: Vec<PendingEmail>,
    dead_letters: Vec<PendingEmail>,
}

#[derive(Debug, Clone)]
struct PendingEmail {
    id: QueueId,
    message: EmailMessage,
    available_at: Instant,
    attempt_count: u32,
    locked_by: Option<String>,
    last_error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{MessageId, SendReceipt};

    #[tokio::test]
    async fn memory_queue_enqueues_and_reserves_message() {
        let queue = QueueHandle::memory(4);
        let message = test_message();

        let id = queue
            .enqueue(QueueItem::new(message))
            .await
            .expect("enqueue succeeds");
        let reserved = queue
            .reserve_batch("worker", 2)
            .await
            .expect("reserve succeeds");

        assert_eq!(reserved.len(), 1);
        assert_eq!(reserved[0].id(), &id);
        assert!(reserved[0].message().idempotency_key().is_some());
    }

    #[tokio::test]
    async fn memory_queue_dead_letters_reserved_message() {
        let queue = QueueHandle::memory(4);
        let id = queue
            .enqueue(QueueItem::new(test_message()))
            .await
            .expect("enqueue succeeds");
        let _reserved = queue
            .reserve_batch("worker", 1)
            .await
            .expect("reserve succeeds");

        queue
            .dead_letter(&id, &MailError::Validation("bad".to_owned()))
            .await
            .expect("dead letter succeeds");

        assert_eq!(queue.dead_letter_len().await.expect("len"), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn memory_queue_releases_for_retry_after_delay() {
        let queue = QueueHandle::memory(4);
        let id = queue
            .enqueue(QueueItem::new(test_message()))
            .await
            .expect("enqueue succeeds");
        let _reserved = queue
            .reserve_batch("worker", 1)
            .await
            .expect("reserve succeeds");

        queue
            .release_for_retry(&id, Duration::from_secs(2), &MailError::RateLimited)
            .await
            .expect("release succeeds");

        assert!(
            queue
                .reserve_batch("worker", 1)
                .await
                .expect("reserve")
                .is_empty()
        );
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

    #[allow(dead_code)]
    fn test_receipt() -> SendReceipt {
        SendReceipt::new("test", MessageId::new("message"), None)
    }
}
