# Changelog

All notable changes to Mailbridge will be documented in this file.

This project follows semantic versioning after the first crates.io release.

## Unreleased

No unreleased changes.

## 0.1.1

### Added

- MIT license file for open-source distribution.
- Contributor guide.
- Architecture and design documentation.
- Public roadmap.
- Security policy.
- Usage guide.

### Changed

- Upgraded `reqwest` to 0.13.
- Updated the `rustls` feature mapping to use `reqwest/rustls`.
- Updated SQLx 0.9 queue features to use `sqlx/runtime-tokio` and
  `sqlx/tls-rustls`.
- Raised the documented MSRV to Rust 1.94.

## 0.1.0

### Added

- Provider-neutral `MailProvider` abstraction.
- Hyvor Relay HTTP provider.
- Reserved provider feature flags for SendGrid, SendPulse, and Mailgun.
- SMTP provider feature through `lettre`.
- Typed email message, address, and attachment models.
- Sender-domain validation.
- In-memory queue backend.
- SQLite, PostgreSQL, and Scylla queue backends behind feature flags.
- Queue worker with retry and dead-letter behavior.
- Global and per-domain rate limiting.
- Optional telemetry events that avoid secrets and message bodies.
- GitHub Actions CI workflow.
- Manual crates.io release workflow.
