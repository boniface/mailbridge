use std::any::Any;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};

use crate::client::SendReceipt;
use crate::email::EmailMessage;
use crate::error::{MailError, Result};
use crate::queue::{MailQueue, QueueHandle, QueueId, QueueItem, QueuedEmail};

#[derive(Debug, Clone)]
pub struct PostgresQueue {
    database_url: SecretString,
    pool: PgPool,
    lock_timeout: Duration,
}

impl PostgresQueue {
    /// Opens a `PostgreSQL` queue pool and initializes its schema.
    ///
    /// # Errors
    ///
    /// Returns an error when the database connection or schema initialization
    /// fails.
    pub async fn connect(database_url: SecretString) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url.expose_secret())
            .await
            .map_err(queue_error)?;
        let queue = Self {
            database_url,
            pool,
            lock_timeout: Duration::from_mins(5),
        };
        queue.initialize().await?;
        Ok(queue)
    }

    #[must_use]
    pub fn from_pool(database_url: SecretString, pool: PgPool) -> Self {
        Self {
            database_url,
            pool,
            lock_timeout: Duration::from_mins(5),
        }
    }

    #[must_use]
    pub fn with_lock_timeout(mut self, timeout: Duration) -> Self {
        self.lock_timeout = timeout;
        self
    }

    /// Initializes the `PostgreSQL` queue schema and indexes.
    ///
    /// # Errors
    ///
    /// Returns an error when advisory locking or any schema statement fails.
    pub async fn initialize(&self) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(queue_error)?;
        sqlx::query("select pg_advisory_xact_lock(hashtext('mailbridge_email_queue_schema'))")
            .execute(&mut *tx)
            .await
            .map_err(queue_error)?;
        sqlx::query(Self::schema_sql())
            .execute(&mut *tx)
            .await
            .map_err(queue_error)?;
        sqlx::query(Self::reservation_index_sql())
            .execute(&mut *tx)
            .await
            .map_err(queue_error)?;
        sqlx::query(Self::locked_index_sql())
            .execute(&mut *tx)
            .await
            .map_err(queue_error)?;
        sqlx::query(Self::dead_letter_schema_sql())
            .execute(&mut *tx)
            .await
            .map_err(queue_error)?;
        sqlx::query(Self::dead_letter_created_index_sql())
            .execute(&mut *tx)
            .await
            .map_err(queue_error)?;
        tx.commit().await.map_err(queue_error)?;
        Ok(())
    }

    #[must_use]
    pub fn handle(self) -> QueueHandle {
        QueueHandle::new(self)
    }

    #[must_use]
    pub const fn database_url(&self) -> &SecretString {
        &self.database_url
    }

    #[must_use]
    pub const fn schema_sql() -> &'static str {
        "create table if not exists email_queue (
            id text primary key,
            created_at_ms bigint not null,
            available_at_ms bigint not null,
            attempt_count integer not null,
            locked_at_ms bigint,
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
            created_at_ms bigint not null,
            dead_lettered_at_ms bigint not null,
            attempt_count integer not null,
            last_error text not null,
            message_json text not null,
            idempotency_key text not null
        )"
    }

    #[must_use]
    pub const fn reservation_sql() -> &'static str {
        "select id, message_json, attempt_count from email_queue
         where available_at_ms <= $1 and (locked_at_ms is null or locked_at_ms <= $2)
         order by available_at_ms asc
         for update skip locked
         limit $3"
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
impl MailQueue for PostgresQueue {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn backend_name(&self) -> &'static str {
        "postgres"
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
            ) values ($1, $2, $3, 0, $4, $5)",
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
            let attempt_count: i32 = row.try_get("attempt_count").map_err(queue_error)?;
            sqlx::query(
                "update email_queue
                 set locked_at_ms = $1, locked_by = $2
                 where id = $3",
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
        sqlx::query("delete from email_queue where id = $1")
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
                 available_at_ms = $1,
                 locked_at_ms = null,
                 locked_by = null,
                 last_error = $2
             where id = $3",
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
             where id = $1",
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
            ) values ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(row.try_get::<String, _>("id").map_err(queue_error)?)
        .bind(
            row.try_get::<i64, _>("created_at_ms")
                .map_err(queue_error)?,
        )
        .bind(epoch_millis())
        .bind(
            row.try_get::<i32, _>("attempt_count")
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

        sqlx::query("delete from email_queue where id = $1")
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

async fn count_rows(pool: &PgPool, sql: &'static str) -> Result<usize> {
    let count = sqlx::query(sql)
        .fetch_one(pool)
        .await
        .map_err(queue_error)?
        .try_get::<i64, _>(0)
        .map_err(queue_error)?;
    Ok(usize::try_from(count).unwrap_or(usize::MAX))
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
    use secrecy::SecretString;

    static POSTGRES_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    #[test]
    fn reservation_sql_uses_skip_locked() {
        assert!(PostgresQueue::reservation_sql().contains("skip locked"));
    }

    #[tokio::test]
    async fn postgres_queue_reserves_and_marks_sent_when_database_is_available() {
        let _guard = POSTGRES_TEST_LOCK.lock().await;
        let Some(queue) = test_queue().await else {
            return;
        };
        let id = queue
            .enqueue(QueueItem::new(test_message("one@example.com")))
            .await
            .expect("enqueue succeeds");

        let reserved = queue
            .reserve_batch("worker-a", 1)
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
    async fn postgres_queue_concurrent_workers_do_not_reserve_same_message_when_database_is_available()
     {
        let _guard = POSTGRES_TEST_LOCK.lock().await;
        let Some(queue) = test_queue().await else {
            return;
        };
        for index in 0..4 {
            queue
                .enqueue(QueueItem::new(test_message(format!(
                    "user-{index}@example.com"
                ))))
                .await
                .expect("enqueue succeeds");
        }
        let first = queue.clone();
        let second = queue.clone();

        let (a, b) = tokio::join!(
            first.reserve_batch("worker-a", 2),
            second.reserve_batch("worker-b", 2)
        );
        let mut ids = a
            .expect("worker a reserves")
            .into_iter()
            .chain(b.expect("worker b reserves"))
            .map(|queued| queued.id().as_str().to_owned())
            .collect::<Vec<_>>();
        ids.sort();
        ids.dedup();

        assert_eq!(ids.len(), 4);
    }

    async fn test_queue() -> Option<PostgresQueue> {
        if !persistent_backend_tests_enabled(
            std::env::var_os("CI").is_some(),
            std::env::var("MAILBRIDGE_RUN_PERSISTENT_TESTS")
                .ok()
                .as_deref(),
        ) {
            return None;
        }
        let database_url = std::env::var("MAILBRIDGE_TEST_POSTGRES_URL").ok()?;
        let queue = PostgresQueue::connect(secret_string(database_url))
            .await
            .expect("postgres test queue connects");
        truncate(&queue).await;
        Some(queue)
    }

    fn persistent_backend_tests_enabled(is_ci: bool, opt_in: Option<&str>) -> bool {
        !is_ci && matches!(opt_in, Some("1" | "true" | "TRUE" | "yes" | "YES"))
    }

    #[test]
    fn persistent_backend_tests_require_local_opt_in_outside_ci() {
        assert!(persistent_backend_tests_enabled(false, Some("true")));
        assert!(persistent_backend_tests_enabled(false, Some("1")));
        assert!(!persistent_backend_tests_enabled(false, None));
        assert!(!persistent_backend_tests_enabled(false, Some("false")));
        assert!(!persistent_backend_tests_enabled(true, Some("true")));
    }

    async fn truncate(queue: &PostgresQueue) {
        sqlx::query("truncate table email_queue, email_queue_dead_letters")
            .execute(&queue.pool)
            .await
            .expect("truncate succeeds");
    }

    fn test_message(email: impl Into<String>) -> EmailMessage {
        EmailMessage::builder()
            .from("App", "sender@example.com")
            .expect("valid from")
            .to("User", email)
            .expect("valid to")
            .subject("Hello")
            .text("Body")
            .build()
            .expect("valid message")
    }

    fn secret_string(value: String) -> SecretString {
        SecretString::new(value.into_boxed_str())
    }
}
