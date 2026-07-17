# Changelog

All notable changes to Mailbridge will be documented in this file.

This project follows semantic versioning after the first crates.io release.

## Unreleased

No unreleased changes.

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
