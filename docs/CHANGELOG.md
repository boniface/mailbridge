# Changelog

[README](../README.md) | [Usage](usage.md) | [Architecture](architecture-and-design.md) | [Telemetry](telemetry.md) | [Roadmap](ROADMAP.md) | [Changelog](CHANGELOG.md)

All notable changes to Mailbridge will be documented in this file.

This project follows semantic versioning after the first crates.io release.

## Unreleased

No unreleased changes.

## 0.3.0 - 2026-07-19

### Added

- Added opt-in `tracing` spans for send, provider send, queue enqueue, queue
  reserve, queue processing, retry scheduling, and dead-letter operations.
- Added stable telemetry event and attribute naming for Mailbridge operations.
- Added stable delivery mode labels for direct sends and queued worker sends.
- Added safe error classification fields for telemetry without logging raw
  provider messages, message bodies, attachment content, credentials, or full
  recipient lists.
- Added queue backend names to queue telemetry.
- Added a no-credentials telemetry example.
- Added public telemetry documentation covering `tracing` setup,
  OpenTelemetry integration ownership, event names, span names, attributes,
  and safety policy.

### Changed

- Updated the roadmap to mark the OpenTelemetry-friendly observability
  milestone as delivered.
- Updated usage and architecture documentation for the 0.3 release line.

## 0.2.1 - 2026-07-17

### Fixed

- Refreshed `anyhow` to resolve `RUSTSEC-2026-0190` in scheduled security
  checks.
- Refreshed `quinn-proto` to resolve `RUSTSEC-2026-0185` from the transitive
  `reqwest` dependency graph.
- Updated scheduled security and CI feature matrices to include every
  `0.2.x` provider feature.
- Constrained scheduled semver checks to Mailbridge's default feature set so
  mutually-exclusive TLS features are not enabled together.

## 0.2.0 - 2026-07-17

### Added

- Implemented the `sendgrid`, `mailgun`, and `sendpulse` provider feature
  flags with opt-in HTTP sending support.
- Added provider configuration types for SendGrid, Mailgun, and SendPulse,
  including environment-based constructors and provider-specific base URL
  overrides.
- Added SendPulse client-credentials token acquisition with an access-token
  fallback for externally managed tokens.
- Implemented `resend`, `mailjet`, `brevo`, and `bird` provider feature flags.
- Added Mailjet v3.1 and v3 send API support, including sandbox mode.
- Added provider capability metadata through `ProviderCapabilities` and
  `MailProvider::capabilities()`.
- Added examples for every HTTP provider feature.

### Fixed

- Refreshed the lockfile to resolve the yanked transitive `spin 0.9.8`
  package warning during crate packaging.

## 0.1.2

### Fixed

- Changed Hyvor Relay sender serialization for named addresses. The Hyvor
  provider now sends named `from` addresses as Relay-compatible objects, for
  example `{ "email": "no-reply@audience-desk.com", "name": "Audience Desk Support" }`,
  instead of formatted mailbox strings such as
  `"Audience Desk Support <no-reply@audience-desk.com>"`.

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
