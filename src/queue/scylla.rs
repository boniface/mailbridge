use std::any::Any;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use scylla::client::session::Session;
use scylla::client::session_builder::SessionBuilder;

use crate::client::SendReceipt;
use crate::email::EmailMessage;
use crate::error::{MailError, Result};
use crate::queue::{MailQueue, QueueHandle, QueueId, QueueItem, QueuedEmail};

const DEFAULT_BUCKET_COUNT: u16 = 64;
const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(300);
const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

#[derive(Debug, Clone)]
pub struct ScyllaQueue {
    session: Arc<Session>,
    tables: ScyllaTables,
    bucket_count: u16,
    lock_timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScyllaTables {
    keyspace: String,
    items: String,
    due: String,
    dead_letters: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QueueRecord {
    bucket: i16,
    id: String,
    created_at_ms: i64,
    available_at_ms: i64,
    attempt_count: i32,
    message_json: String,
    idempotency_key: String,
}

impl ScyllaQueue {
    pub async fn connect(
        uri: impl AsRef<str>,
        keyspace: impl Into<String>,
        table: impl Into<String>,
    ) -> Result<Self> {
        let session = SessionBuilder::new()
            .known_node(uri.as_ref())
            .build()
            .await
            .map_err(queue_error)?;
        let queue = Self::from_session(session, keyspace, table)?;
        queue.initialize().await?;
        Ok(queue)
    }

    pub fn from_session(
        session: Session,
        keyspace: impl Into<String>,
        table: impl Into<String>,
    ) -> Result<Self> {
        Self::from_shared_session(Arc::new(session), keyspace, table)
    }

    pub fn from_shared_session(
        session: Arc<Session>,
        keyspace: impl Into<String>,
        table: impl Into<String>,
    ) -> Result<Self> {
        Ok(Self {
            session,
            tables: ScyllaTables::new(keyspace.into(), table.into())?,
            bucket_count: DEFAULT_BUCKET_COUNT,
            lock_timeout: DEFAULT_LOCK_TIMEOUT,
        })
    }

    pub async fn initialize(&self) -> Result<()> {
        self.query(self.items_schema_cql(), &[]).await?;
        self.query(self.due_schema_cql(), &[]).await?;
        self.query(self.dead_letter_schema_cql(), &[]).await?;
        Ok(())
    }

    #[must_use]
    pub fn handle(self) -> QueueHandle {
        QueueHandle::new(self)
    }

    pub fn with_bucket_count(mut self, bucket_count: u16) -> Result<Self> {
        validate_bucket_count(bucket_count)?;
        self.bucket_count = bucket_count;
        Ok(self)
    }

    #[must_use]
    pub fn with_lock_timeout(mut self, timeout: Duration) -> Self {
        self.lock_timeout = timeout;
        self
    }

    #[must_use]
    pub fn bucket_count(&self) -> u16 {
        self.bucket_count
    }

    #[must_use]
    pub fn lock_timeout(&self) -> Duration {
        self.lock_timeout
    }

    #[must_use]
    pub fn keyspace(&self) -> &str {
        &self.tables.keyspace
    }

    #[must_use]
    pub fn item_table(&self) -> &str {
        &self.tables.items
    }

    #[must_use]
    pub fn due_table(&self) -> &str {
        &self.tables.due
    }

    #[must_use]
    pub fn dead_letter_table(&self) -> &str {
        &self.tables.dead_letters
    }

    #[must_use]
    pub fn items_schema_cql(&self) -> String {
        format!(
            "create table if not exists {} (
                bucket smallint,
                id text,
                created_at_ms bigint,
                available_at_ms bigint,
                attempt_count int,
                locked_until_ms bigint,
                locked_by text,
                last_error text,
                message_json text,
                idempotency_key text,
                primary key ((bucket), id)
            )",
            self.tables.items_name()
        )
    }

    #[must_use]
    pub fn due_schema_cql(&self) -> String {
        format!(
            "create table if not exists {} (
                bucket smallint,
                available_at_ms bigint,
                id text,
                primary key ((bucket), available_at_ms, id)
            ) with clustering order by (available_at_ms asc, id asc)",
            self.tables.due_name()
        )
    }

    #[must_use]
    pub fn dead_letter_schema_cql(&self) -> String {
        format!(
            "create table if not exists {} (
                bucket smallint,
                dead_lettered_at_ms bigint,
                id text,
                created_at_ms bigint,
                attempt_count int,
                last_error text,
                message_json text,
                idempotency_key text,
                primary key ((bucket), dead_lettered_at_ms, id)
            ) with clustering order by (dead_lettered_at_ms asc, id asc)",
            self.tables.dead_letters_name()
        )
    }

    async fn query(
        &self,
        statement: impl Into<scylla::statement::unprepared::Statement>,
        values: impl scylla::serialize::row::SerializeRow,
    ) -> Result<scylla::response::query_result::QueryResult> {
        self.session
            .query_unpaged(statement, values)
            .await
            .map_err(queue_error)
    }

    async fn select_record(&self, id: &QueueId) -> Result<Option<QueueRecord>> {
        let bucket = bucket_for(id.as_str(), self.bucket_count);
        let rows = self
            .query(
                format!(
                    "select bucket, id, created_at_ms, available_at_ms, attempt_count,
                        message_json, idempotency_key
                     from {}
                     where bucket = ? and id = ?",
                    self.tables.items_name()
                ),
                (bucket, id.as_str()),
            )
            .await?
            .into_rows_result()
            .map_err(queue_error)?;

        rows.maybe_first_row::<(i16, String, i64, i64, i32, String, String)>()
            .map_err(queue_error)
            .map(|row| {
                row.map(
                    |(
                        bucket,
                        id,
                        created_at_ms,
                        available_at_ms,
                        attempt_count,
                        message_json,
                        idempotency_key,
                    )| QueueRecord {
                        bucket,
                        id,
                        created_at_ms,
                        available_at_ms,
                        attempt_count,
                        message_json,
                        idempotency_key,
                    },
                )
            })
    }

    async fn delete_due(&self, bucket: i16, available_at_ms: i64, id: &str) -> Result<()> {
        self.query(
            format!(
                "delete from {} where bucket = ? and available_at_ms = ? and id = ?",
                self.tables.due_name()
            ),
            (bucket, available_at_ms, id),
        )
        .await?;
        Ok(())
    }

    async fn cleanup_stale_due(&self, bucket: i16, available_at_ms: i64, id: &str) -> Result<()> {
        self.delete_due(bucket, available_at_ms, id).await
    }

    async fn claim_record(
        &self,
        row: DueRow,
        worker_id: &str,
        now: i64,
    ) -> Result<Option<QueuedEmail>> {
        let Some(record) = self.select_record(&QueueId::new(&row.id)).await? else {
            self.cleanup_stale_due(row.bucket, row.available_at_ms, &row.id)
                .await?;
            return Ok(None);
        };
        if record.available_at_ms != row.available_at_ms {
            self.cleanup_stale_due(row.bucket, row.available_at_ms, &row.id)
                .await?;
            return Ok(None);
        }

        let lock_until = now.saturating_add(duration_millis_i64(self.lock_timeout));
        let applied = self
            .query(
                format!(
                    "update {}
                     set locked_until_ms = ?, locked_by = ?
                     where bucket = ? and id = ?
                     if locked_until_ms <= ? and available_at_ms = ?",
                    self.tables.items_name()
                ),
                (
                    lock_until,
                    worker_id,
                    record.bucket,
                    record.id.as_str(),
                    now,
                    record.available_at_ms,
                ),
            )
            .await?
            .into_rows_result()
            .map_err(queue_error)?
            .first_row::<(bool, Option<i64>, Option<i64>)>()
            .map_err(queue_error)?
            .0;

        if !applied {
            return Ok(None);
        }

        let message =
            serde_json::from_str::<EmailMessage>(&record.message_json).map_err(queue_error)?;
        Ok(Some(QueuedEmail::new(
            QueueId::new(record.id),
            message,
            u32::try_from(record.attempt_count).unwrap_or(u32::MAX),
        )))
    }
}

#[async_trait]
impl MailQueue for ScyllaQueue {
    fn as_any(&self) -> &dyn Any {
        self
    }

    async fn enqueue(&self, item: QueueItem) -> Result<QueueId> {
        let id = QueueId::new_uuid();
        let bucket = bucket_for(id.as_str(), self.bucket_count);
        let message = item.into_message();
        let idempotency_key = message
            .idempotency_key()
            .ok_or_else(|| MailError::Queue("queued message idempotency key missing".to_owned()))?
            .to_owned();
        let message_json = serde_json::to_string(&message).map_err(queue_error)?;
        let now = epoch_millis();

        self.query(
            format!(
                "insert into {} (
                    bucket, id, created_at_ms, available_at_ms, attempt_count,
                    locked_until_ms, locked_by, message_json, idempotency_key
                ) values (?, ?, ?, ?, 0, 0, '', ?, ?)",
                self.tables.items_name()
            ),
            (
                bucket,
                id.as_str(),
                now,
                now,
                message_json.as_str(),
                idempotency_key.as_str(),
            ),
        )
        .await?;

        self.query(
            format!(
                "insert into {} (bucket, available_at_ms, id) values (?, ?, ?)",
                self.tables.due_name()
            ),
            (bucket, now, id.as_str()),
        )
        .await?;

        Ok(id)
    }

    async fn reserve_batch(&self, worker_id: &str, limit: u32) -> Result<Vec<QueuedEmail>> {
        let target = usize::try_from(limit).unwrap_or(usize::MAX);
        if target == 0 {
            return Ok(Vec::new());
        }

        let now = epoch_millis();
        let mut queued = Vec::with_capacity(target.min(usize::from(self.bucket_count)));
        for bucket in 0..self.bucket_count {
            let rows = self
                .query(
                    format!(
                        "select bucket, available_at_ms, id
                         from {}
                         where bucket = ? and available_at_ms <= ?
                         limit ?",
                        self.tables.due_name()
                    ),
                    (
                        i16::try_from(bucket).unwrap_or(i16::MAX),
                        now,
                        i32::try_from(limit).unwrap_or(i32::MAX),
                    ),
                )
                .await?
                .into_rows_result()
                .map_err(queue_error)?;

            for row in rows
                .rows::<(i16, i64, String)>()
                .map_err(queue_error)?
                .map(|row| row.map_err(queue_error))
            {
                let (bucket, available_at_ms, id) = row?;
                if let Some(email) = self
                    .claim_record(
                        DueRow {
                            bucket,
                            available_at_ms,
                            id,
                        },
                        worker_id,
                        now,
                    )
                    .await?
                {
                    queued.push(email);
                    if queued.len() == target {
                        return Ok(queued);
                    }
                }
            }
        }

        Ok(queued)
    }

    async fn mark_sent(&self, id: &QueueId, _receipt: &SendReceipt) -> Result<()> {
        let Some(record) = self.select_record(id).await? else {
            return Ok(());
        };

        self.query(
            format!(
                "delete from {} where bucket = ? and id = ?",
                self.tables.items_name()
            ),
            (record.bucket, id.as_str()),
        )
        .await?;
        self.delete_due(record.bucket, record.available_at_ms, id.as_str())
            .await?;
        Ok(())
    }

    async fn release_for_retry(
        &self,
        id: &QueueId,
        delay: Duration,
        error: &MailError,
    ) -> Result<()> {
        let Some(record) = self.select_record(id).await? else {
            return Err(MailError::Queue(format!("queued message not found: {id}")));
        };

        let available_at = epoch_millis().saturating_add(duration_millis_i64(delay));
        let next_attempt = record.attempt_count.saturating_add(1);
        let last_error = error.to_string();

        self.query(
            format!(
                "insert into {} (bucket, available_at_ms, id) values (?, ?, ?)",
                self.tables.due_name()
            ),
            (record.bucket, available_at, id.as_str()),
        )
        .await?;
        self.query(
            format!(
                "update {}
                 set attempt_count = ?,
                     available_at_ms = ?,
                     locked_until_ms = 0,
                     locked_by = '',
                     last_error = ?
                 where bucket = ? and id = ?",
                self.tables.items_name()
            ),
            (
                next_attempt,
                available_at,
                last_error.as_str(),
                record.bucket,
                id.as_str(),
            ),
        )
        .await?;
        self.delete_due(record.bucket, record.available_at_ms, id.as_str())
            .await?;
        Ok(())
    }

    async fn dead_letter(&self, id: &QueueId, error: &MailError) -> Result<()> {
        let Some(record) = self.select_record(id).await? else {
            return Err(MailError::Queue(format!("queued message not found: {id}")));
        };
        let last_error = error.to_string();

        self.query(
            format!(
                "insert into {} (
                    bucket, dead_lettered_at_ms, id, created_at_ms, attempt_count,
                    last_error, message_json, idempotency_key
                ) values (?, ?, ?, ?, ?, ?, ?, ?)",
                self.tables.dead_letters_name()
            ),
            (
                record.bucket,
                epoch_millis(),
                id.as_str(),
                record.created_at_ms,
                record.attempt_count,
                last_error.as_str(),
                record.message_json.as_str(),
                record.idempotency_key.as_str(),
            ),
        )
        .await?;
        self.query(
            format!(
                "delete from {} where bucket = ? and id = ?",
                self.tables.items_name()
            ),
            (record.bucket, id.as_str()),
        )
        .await?;
        self.delete_due(record.bucket, record.available_at_ms, id.as_str())
            .await?;
        Ok(())
    }

    async fn pending_len(&self) -> Result<usize> {
        let table = self.tables.items_name();
        self.count_buckets(&table).await
    }

    async fn dead_letter_len(&self) -> Result<usize> {
        let table = self.tables.dead_letters_name();
        self.count_buckets(&table).await
    }
}

impl ScyllaQueue {
    async fn count_buckets(&self, table: &str) -> Result<usize> {
        let mut total = 0usize;
        for bucket in 0..self.bucket_count {
            let rows = self
                .query(
                    format!("select count(*) from {table} where bucket = ?"),
                    (i16::try_from(bucket).unwrap_or(i16::MAX),),
                )
                .await?
                .into_rows_result()
                .map_err(queue_error)?;
            let (count,) = rows.first_row::<(i64,)>().map_err(queue_error)?;
            total = total.saturating_add(usize::try_from(count).unwrap_or(usize::MAX));
        }
        Ok(total)
    }
}

impl ScyllaTables {
    fn new(keyspace: String, table: String) -> Result<Self> {
        validate_identifier("scylla keyspace", &keyspace)?;
        validate_identifier("scylla table", &table)?;

        Ok(Self {
            keyspace,
            due: format!("{table}_due"),
            dead_letters: format!("{table}_dead_letters"),
            items: table,
        })
    }

    fn items_name(&self) -> String {
        format!("{}.{}", self.keyspace, self.items)
    }

    fn due_name(&self) -> String {
        format!("{}.{}", self.keyspace, self.due)
    }

    fn dead_letters_name(&self) -> String {
        format!("{}.{}", self.keyspace, self.dead_letters)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DueRow {
    bucket: i16,
    available_at_ms: i64,
    id: String,
}

fn validate_identifier(label: &str, value: &str) -> Result<()> {
    let Some(first) = value.bytes().next() else {
        return Err(MailError::Config(format!(
            "{label} must start with an ascii letter or underscore"
        )));
    };

    if !(first.is_ascii_alphabetic() || first == b'_')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(MailError::Config(format!(
            "{label} must start with an ascii letter or underscore and contain only ascii letters, digits, and underscores"
        )));
    }

    Ok(())
}

fn bucket_for(value: &str, bucket_count: u16) -> i16 {
    let hash = value
        .as_bytes()
        .iter()
        .fold(FNV_OFFSET_BASIS, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
        });
    i16::try_from(hash % u64::from(bucket_count)).unwrap_or(0)
}

fn validate_bucket_count(bucket_count: u16) -> Result<()> {
    if bucket_count == 0 || bucket_count > i16::MAX as u16 {
        return Err(MailError::Config(
            "scylla queue bucket count must be between 1 and 32767".to_owned(),
        ));
    }

    Ok(())
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static SCYLLA_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    static SCYLLA_TABLE_COUNTER: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn validates_cql_identifiers() {
        assert!(validate_identifier("table", "email_queue_1").is_ok());
        assert!(validate_identifier("table", "").is_err());
        assert!(validate_identifier("table", "1_email_queue").is_err());
        assert!(validate_identifier("table", "email-queue").is_err());
        assert!(validate_identifier("table", "email_queue;drop").is_err());
    }

    #[test]
    fn bucket_for_is_stable_and_in_range() {
        let first = bucket_for("queue-id", 64);
        let second = bucket_for("queue-id", 64);

        assert_eq!(first, second);
        assert!((0..64).contains(&first));
    }

    #[test]
    fn bucket_count_validation_rejects_invalid_values() {
        assert!(validate_bucket_count(1).is_ok());
        assert!(validate_bucket_count(i16::MAX as u16).is_ok());
        assert!(validate_bucket_count(0).is_err());
        assert!(validate_bucket_count(i16::MAX as u16 + 1).is_err());
    }

    #[test]
    fn table_names_are_derived_from_base_table() {
        let tables = ScyllaTables::new("mailbridge".to_owned(), "email_queue".to_owned())
            .expect("valid tables");

        assert_eq!(tables.items_name(), "mailbridge.email_queue");
        assert_eq!(tables.due_name(), "mailbridge.email_queue_due");
        assert_eq!(
            tables.dead_letters_name(),
            "mailbridge.email_queue_dead_letters"
        );
    }

    #[tokio::test]
    async fn scylla_queue_reserves_and_marks_sent_when_database_is_available() {
        let _guard = SCYLLA_TEST_LOCK.lock().await;
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
                &SendReceipt::new("test", crate::client::MessageId::new("message"), None),
            )
            .await
            .expect("mark sent succeeds");
        assert_eq!(queue.pending_len().await.expect("count"), 0);
    }

    #[tokio::test]
    async fn scylla_queue_concurrent_workers_do_not_reserve_same_message_when_database_is_available()
     {
        let _guard = SCYLLA_TEST_LOCK.lock().await;
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
        let concurrent_count = ids.len();
        ids.sort();
        ids.dedup();

        assert_eq!(ids.len(), concurrent_count);

        let remaining = queue
            .reserve_batch("worker-c", 4)
            .await
            .expect("remaining messages reserve");
        ids.extend(
            remaining
                .into_iter()
                .map(|queued| queued.id().as_str().to_owned()),
        );
        ids.sort();
        ids.dedup();

        assert_eq!(ids.len(), 4);
    }

    async fn test_queue() -> Option<ScyllaQueue> {
        if !persistent_backend_tests_enabled(
            std::env::var_os("CI").is_some(),
            std::env::var("MAILBRIDGE_RUN_PERSISTENT_TESTS")
                .ok()
                .as_deref(),
        ) {
            return None;
        }
        let uri = std::env::var("MAILBRIDGE_TEST_SCYLLA_URI").ok()?;
        let keyspace = std::env::var("MAILBRIDGE_TEST_SCYLLA_KEYSPACE")
            .unwrap_or_else(|_| "mailbridge_test".to_owned());
        let base_table = std::env::var("MAILBRIDGE_TEST_SCYLLA_TABLE")
            .unwrap_or_else(|_| "email_queue".to_owned());
        let table = format!(
            "{}_{}_{}",
            base_table,
            epoch_millis(),
            SCYLLA_TABLE_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let queue = ScyllaQueue::connect(uri, keyspace, table)
            .await
            .expect("scylla test queue connects");
        Some(queue.with_bucket_count(8).expect("valid bucket count"))
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
}
