use std::any::Any;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

use crate::client::SendReceipt;
use crate::email::EmailMessage;
use crate::error::{MailError, Result};
use crate::queue::{MailQueue, QueueHandle, QueueId, QueueItem, QueuedEmail};

#[derive(Debug, Clone)]
pub struct SqliteQueue {
    path: PathBuf,
    pool: SqlitePool,
    lock_timeout: Duration,
}

impl SqliteQueue {
    /// Opens a `SQLite` queue database and initializes its schema.
    ///
    /// # Errors
    ///
    /// Returns an error when the path cannot be converted to `SQLite` options,
    /// the database cannot be opened, or schema initialization fails.
    pub async fn connect(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let url = sqlite_url(&path);
        let options = SqliteConnectOptions::from_str(&url)
            .map_err(queue_error)?
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .map_err(queue_error)?;
        let queue = Self {
            path,
            pool,
            lock_timeout: Duration::from_mins(5),
        };
        queue.initialize().await?;
        Ok(queue)
    }

    #[must_use]
    pub fn from_pool(path: impl Into<PathBuf>, pool: SqlitePool) -> Self {
        Self {
            path: path.into(),
            pool,
            lock_timeout: Duration::from_mins(5),
        }
    }

    #[must_use]
    pub fn with_lock_timeout(mut self, timeout: Duration) -> Self {
        self.lock_timeout = timeout;
        self
    }

    /// Initializes the `SQLite` queue schema and indexes.
    ///
    /// # Errors
    ///
    /// Returns an error when any schema statement fails.
    pub async fn initialize(&self) -> Result<()> {
        sqlx::query(Self::schema_sql())
            .execute(&self.pool)
            .await
            .map_err(queue_error)?;
        sqlx::query(Self::reservation_index_sql())
            .execute(&self.pool)
            .await
            .map_err(queue_error)?;
        sqlx::query(Self::locked_index_sql())
            .execute(&self.pool)
            .await
            .map_err(queue_error)?;
        sqlx::query(Self::dead_letter_schema_sql())
            .execute(&self.pool)
            .await
            .map_err(queue_error)?;
        sqlx::query(Self::dead_letter_created_index_sql())
            .execute(&self.pool)
            .await
            .map_err(queue_error)?;
        Ok(())
    }

    #[must_use]
    pub fn handle(self) -> QueueHandle {
        QueueHandle::new(self)
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub const fn schema_sql() -> &'static str {
        "create table if not exists email_queue (
            id text primary key,
            created_at_ms integer not null,
            available_at_ms integer not null,
            attempt_count integer not null,
            locked_at_ms integer,
            locked_by text,
            last_error text,
            message_json text not null,
            idempotency_key text not null
        )"
    }

    #[must_use]
    pub const fn dead_letter_schema_sql() -> &'static str {
        "create table if not exists email_queue_dead_letters (
            id text primary key,
            created_at_ms integer not null,
            dead_lettered_at_ms integer not null,
            attempt_count integer not null,
            last_error text not null,
            message_json text not null,
            idempotency_key text not null
        )"
    }

    #[must_use]
    pub const fn reservation_sql() -> &'static str {
        "select id, message_json, attempt_count from email_queue
         where available_at_ms <= ? and (locked_at_ms is null or locked_at_ms <= ?)
         order by available_at_ms asc
         limit ?"
    }

    #[must_use]
    pub const fn reservation_index_sql() -> &'static str {
        "create index if not exists idx_email_queue_available
         on email_queue (available_at_ms, locked_at_ms)"
    }

    #[must_use]
    pub const fn locked_index_sql() -> &'static str {
        "create index if not exists idx_email_queue_locked
         on email_queue (locked_at_ms, locked_by)"
    }

    #[must_use]
    pub const fn dead_letter_created_index_sql() -> &'static str {
        "create index if not exists idx_email_queue_dead_letters_created
         on email_queue_dead_letters (dead_lettered_at_ms)"
    }
}

#[async_trait]
impl MailQueue for SqliteQueue {
    fn as_any(&self) -> &dyn Any {
        self
    }

    async fn enqueue(&self, item: QueueItem) -> Result<QueueId> {
        let id = QueueId::new_uuid();
        let message = item.into_message();
        let idempotency_key = message
            .idempotency_key()
            .ok_or_else(|| MailError::Queue("queued message idempotency key missing".to_owned()))?
            .to_owned();
        let message_json = serde_json::to_string(&message).map_err(queue_error)?;
        let now = epoch_millis();

        sqlx::query(
            "insert into email_queue (
                id, created_at_ms, available_at_ms, attempt_count, message_json, idempotency_key
            ) values (?, ?, ?, 0, ?, ?)",
        )
        .bind(id.as_str())
        .bind(now)
        .bind(now)
        .bind(message_json)
        .bind(idempotency_key)
        .execute(&self.pool)
        .await
        .map_err(queue_error)?;

        Ok(id)
    }

    async fn reserve_batch(&self, worker_id: &str, limit: u32) -> Result<Vec<QueuedEmail>> {
        let mut tx = self.pool.begin().await.map_err(queue_error)?;
        let now = epoch_millis();
        let expired_before = now.saturating_sub(duration_millis_i64(self.lock_timeout));
        let rows = sqlx::query(Self::reservation_sql())
            .bind(now)
            .bind(expired_before)
            .bind(i64::from(limit))
            .fetch_all(&mut *tx)
            .await
            .map_err(queue_error)?;

        let mut queued = Vec::with_capacity(rows.len());
        for row in rows {
            let id: String = row.try_get("id").map_err(queue_error)?;
            let message_json: String = row.try_get("message_json").map_err(queue_error)?;
            let attempt_count: i64 = row.try_get("attempt_count").map_err(queue_error)?;
            sqlx::query(
                "update email_queue
                 set locked_at_ms = ?, locked_by = ?
                 where id = ?",
            )
            .bind(now)
            .bind(worker_id)
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(queue_error)?;
            queued.push(QueuedEmail::new(
                QueueId::new(id),
                serde_json::from_str::<EmailMessage>(&message_json).map_err(queue_error)?,
                u32::try_from(attempt_count).unwrap_or(u32::MAX),
            ));
        }

        tx.commit().await.map_err(queue_error)?;
        Ok(queued)
    }

    async fn mark_sent(&self, id: &QueueId, _receipt: &SendReceipt) -> Result<()> {
        sqlx::query("delete from email_queue where id = ?")
            .bind(id.as_str())
            .execute(&self.pool)
            .await
            .map_err(queue_error)?;
        Ok(())
    }

    async fn release_for_retry(
        &self,
        id: &QueueId,
        delay: Duration,
        error: &MailError,
    ) -> Result<()> {
        let available_at = epoch_millis().saturating_add(duration_millis_i64(delay));
        let result = sqlx::query(
            "update email_queue
             set attempt_count = attempt_count + 1,
                 available_at_ms = ?,
                 locked_at_ms = null,
                 locked_by = null,
                 last_error = ?
             where id = ?",
        )
        .bind(available_at)
        .bind(error.to_string())
        .bind(id.as_str())
        .execute(&self.pool)
        .await
        .map_err(queue_error)?;
        if result.rows_affected() == 0 {
            return Err(MailError::Queue(format!("queued message not found: {id}")));
        }
        Ok(())
    }

    async fn dead_letter(&self, id: &QueueId, error: &MailError) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(queue_error)?;
        let row = sqlx::query(
            "select id, created_at_ms, attempt_count, message_json, idempotency_key
             from email_queue
             where id = ?",
        )
        .bind(id.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(queue_error)?
        .ok_or_else(|| MailError::Queue(format!("queued message not found: {id}")))?;

        sqlx::query(
            "insert into email_queue_dead_letters (
                id, created_at_ms, dead_lettered_at_ms, attempt_count,
                last_error, message_json, idempotency_key
            ) values (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(row.try_get::<String, _>("id").map_err(queue_error)?)
        .bind(
            row.try_get::<i64, _>("created_at_ms")
                .map_err(queue_error)?,
        )
        .bind(epoch_millis())
        .bind(
            row.try_get::<i64, _>("attempt_count")
                .map_err(queue_error)?,
        )
        .bind(error.to_string())
        .bind(
            row.try_get::<String, _>("message_json")
                .map_err(queue_error)?,
        )
        .bind(
            row.try_get::<String, _>("idempotency_key")
                .map_err(queue_error)?,
        )
        .execute(&mut *tx)
        .await
        .map_err(queue_error)?;

        sqlx::query("delete from email_queue where id = ?")
            .bind(id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(queue_error)?;

        tx.commit().await.map_err(queue_error)?;
        Ok(())
    }

    async fn pending_len(&self) -> Result<usize> {
        count_rows(&self.pool, "select count(*) from email_queue").await
    }

    async fn dead_letter_len(&self) -> Result<usize> {
        count_rows(&self.pool, "select count(*) from email_queue_dead_letters").await
    }
}

async fn count_rows(pool: &SqlitePool, sql: &str) -> Result<usize> {
    let count = sqlx::query(sql)
        .fetch_one(pool)
        .await
        .map_err(queue_error)?
        .try_get::<i64, _>(0)
        .map_err(queue_error)?;
    Ok(usize::try_from(count).unwrap_or(usize::MAX))
}

fn sqlite_url(path: &Path) -> String {
    let value = path.to_string_lossy();
    if value.starts_with("sqlite:") {
        value.into_owned()
    } else {
        format!("sqlite://{value}")
    }
}

fn epoch_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, duration_millis_i64)
}

fn duration_millis_i64(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

fn queue_error(error: impl std::fmt::Display) -> MailError {
    MailError::Queue(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{MessageId, SendReceipt};

    #[tokio::test]
    async fn sqlite_queue_reserves_and_marks_sent() {
        let queue = SqliteQueue::connect("sqlite::memory:")
            .await
            .expect("sqlite queue connects");
        let id = queue
            .enqueue(QueueItem::new(test_message()))
            .await
            .expect("enqueue succeeds");

        let reserved = queue
            .reserve_batch("worker", 1)
            .await
            .expect("reserve succeeds");

        assert_eq!(reserved.len(), 1);
        assert_eq!(reserved[0].id(), &id);
        queue
            .mark_sent(
                &id,
                &SendReceipt::new("test", MessageId::new("message"), None),
            )
            .await
            .expect("mark sent succeeds");
        assert_eq!(queue.pending_len().await.expect("count"), 0);
    }

    #[tokio::test]
    async fn sqlite_queue_persists_dead_letters() {
        let queue = SqliteQueue::connect("sqlite::memory:")
            .await
            .expect("sqlite queue connects");
        let id = queue
            .enqueue(QueueItem::new(test_message()))
            .await
            .expect("enqueue succeeds");
        queue
            .reserve_batch("worker", 1)
            .await
            .expect("reserve succeeds");

        queue
            .dead_letter(&id, &MailError::Temporary("failed".to_owned()))
            .await
            .expect("dead letter succeeds");

        assert_eq!(queue.dead_letter_len().await.expect("count"), 1);
    }

    #[tokio::test]
    async fn sqlite_queue_retry_missing_message_returns_error() {
        let queue = SqliteQueue::connect("sqlite::memory:")
            .await
            .expect("sqlite queue connects");
        let id = QueueId::new("missing");

        let error = queue
            .release_for_retry(
                &id,
                Duration::ZERO,
                &MailError::Temporary("failed".to_owned()),
            )
            .await
            .expect_err("missing retry should fail");

        assert!(matches!(error, MailError::Queue(_)));
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
