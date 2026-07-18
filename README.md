# Mailbridge

[![CI](https://github.com/boniface/mailbridge/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/boniface/mailbridge/actions/workflows/ci.yml)
[![Scheduled Security](https://github.com/boniface/mailbridge/actions/workflows/scheduled-security.yml/badge.svg?branch=main)](https://github.com/boniface/mailbridge/actions/workflows/scheduled-security.yml)
[![Release](https://github.com/boniface/mailbridge/actions/workflows/release.yml/badge.svg)](https://github.com/boniface/mailbridge/actions/workflows/release.yml)
[![Crates.io](https://img.shields.io/crates/v/mailbridge.svg)](https://crates.io/crates/mailbridge)
[![Docs.rs](https://docs.rs/mailbridge/badge.svg)](https://docs.rs/mailbridge)
[![License](https://img.shields.io/crates/l/mailbridge.svg)](https://github.com/boniface/mailbridge#license)
[![Rust Version](https://img.shields.io/badge/rust-1.94%2B-blue.svg)](https://github.com/boniface/mailbridge/blob/main/Cargo.toml)
[![Rust Edition](https://img.shields.io/badge/edition-2024-blue.svg)](https://doc.rust-lang.org/edition-guide/rust-2024/)
[![Dependencies](https://deps.rs/repo/github/boniface/mailbridge/status.svg)](https://deps.rs/repo/github/boniface/mailbridge)

Mailbridge is a Rust library for sending application email without tying your
code to one provider's SDK or request model. It gives services one typed API
for building, validating, sending, queueing, retrying, and rate-limiting
transactional email while keeping provider-specific behavior behind opt-in
feature flags.

Use it when your application needs to send operational email such as
verifications, password resets, receipts, alerts, invitations, and product
notifications through Hyvor Relay, SendGrid, Mailgun, SendPulse, Resend,
Mailjet, Brevo, Bird, or SMTP. The goal is to make provider switching and local
testing practical without leaking API keys, message bodies, or provider SDK
types through the rest of your codebase.

Mailbridge is not a marketing automation platform, campaign manager, template
designer, or address-book system. It focuses on the reliable delivery path that
Rust services need at runtime.

## Documentation

- [Usage guide](docs/usage.md)
- [Architecture and design](docs/architecture-and-design.md)
- [Roadmap](docs/ROADMAP.md)
- [Changelog](docs/CHANGELOG.md)
- [Contributing](docs/CONTRIBUTING.md)
- [Contributors](docs/CONTRIBUTORS.md)
- [Security policy](docs/SECURITY.md)

## Features

- `hyvor-relay`, `api`, `rustls`, `queue-memory`, and `rate-limit` are enabled
  by default.
- `smtp` enables SMTP submission through `lettre`.
- `queue-sqlite`, `queue-postgres`, and `queue-scylla` enable durable queue
  adapters.
- `telemetry` emits `tracing` events without API keys, message bodies, or full
  recipient lists.
- `sendgrid`, `sendpulse`, `mailgun`, `resend`, `mailjet`, `brevo`, and
  `bird` enable opt-in HTTP providers for those transactional email services.

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

## License

This project is licensed under the [MIT License](LICENSE).
