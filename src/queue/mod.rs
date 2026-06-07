mod memory;
mod worker;

#[cfg(feature = "queue-postgres")]
mod postgres;
#[cfg(feature = "queue-scylla")]
mod scylla;
#[cfg(feature = "queue-sqlite")]
mod sqlite;

pub use memory::{MailQueue, QueueHandle, QueueId, QueueItem, QueuedEmail};
#[cfg(feature = "queue-postgres")]
pub use postgres::PostgresQueue;
#[cfg(feature = "queue-scylla")]
pub use scylla::ScyllaQueue;
#[cfg(feature = "queue-sqlite")]
pub use sqlite::SqliteQueue;
pub use worker::{QueueWorker, QueueWorkerConfig};
