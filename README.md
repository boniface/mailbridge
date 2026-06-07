# mailbridge

Provider-neutral transactional email library for Rust services.

## Features

- `hyvor-relay`, `api`, `rustls`, `queue-memory`, and `rate-limit` are enabled
  by default.
- `smtp` enables SMTP submission through `lettre`.
- `queue-sqlite`, `queue-postgres`, and `queue-scylla` enable durable queue
  adapters.
- `telemetry` emits `tracing` events without API keys, message bodies, or full
  recipient lists.
- `sendgrid`, `sendpulse`, and `mailgun` are reserved provider flags. Their
  types implement `MailProvider` and return a configuration error until real
  provider implementations are added.

## Queue Backends

Use `queue-memory` for tests and local development. It is process-local and
does not survive restarts.

Use `queue-sqlite` for single-node durable queueing.

Use `queue-postgres` when the application already operates PostgreSQL or needs
SQL visibility. The adapter uses row locking with `FOR UPDATE SKIP LOCKED`.

Use `queue-scylla` for high-throughput distributed queueing only when the
deployment can own Scylla operations. The adapter creates:

- `{table}` for item rows keyed by `(bucket, id)`.
- `{table}_due` for due-time scans keyed by `(bucket, available_at_ms, id)`.
- `{table}_dead_letters` for failed messages.

Scylla operational notes:

- Tune bucket count to spread queue scans across partitions. The default is 64.
- Keep lock timeout longer than normal provider latency and shorter than worker
  recovery requirements. The default is five minutes.
- Reservation uses lightweight transactions, so size worker concurrency and
  bucket count with LWT cost in mind.
- Replication strategy, compaction, dead-letter retention, TTL, and schema
  migration policy belong to the application deployment.
- Live tests use `MAILBRIDGE_TEST_SCYLLA_URI`,
  `MAILBRIDGE_TEST_SCYLLA_KEYSPACE`, and `MAILBRIDGE_TEST_SCYLLA_TABLE`.

## Live Backend Tests

Live persistent-backend tests are local opt-in only. They never run when the
standard `CI` environment variable is present.

PostgreSQL live tests run only when `MAILBRIDGE_RUN_PERSISTENT_TESTS=true` and
`MAILBRIDGE_TEST_POSTGRES_URL` are set.

ScyllaDB live tests run only when `MAILBRIDGE_RUN_PERSISTENT_TESTS=true` and
`MAILBRIDGE_TEST_SCYLLA_URI` are set. The keyspace must already exist.
