# Contributing

[README](../README.md) | [Usage](usage.md) | [Architecture](architecture-and-design.md) | [Telemetry](telemetry.md) | [Roadmap](ROADMAP.md) | [Changelog](CHANGELOG.md)

Thank you for considering a contribution to Mailbridge.

Mailbridge is a provider-neutral transactional email library for Rust services.
Contributions should keep provider-specific behavior isolated, preserve the
public provider-neutral API, and avoid exposing secrets or message content in
logs, errors, telemetry, tests, or examples.

## Development Setup

Requirements:

- Rust 1.91 or newer.
- Cargo.
- Optional provider services only for live tests.

Recommended local checks:

```sh
cargo fmt --all -- --check
cargo test
cargo test --no-default-features
cargo test --no-default-features --features hyvor-relay,api,rustls,queue-memory,rate-limit,smtp,telemetry,queue-sqlite,queue-postgres,queue-scylla,sendgrid,sendpulse,mailgun,resend,mailjet,brevo,bird,dotenv,test-utils
cargo clippy -- -D warnings
cargo clippy --no-default-features --features hyvor-relay,api,rustls,queue-memory,rate-limit,smtp,telemetry,queue-sqlite,queue-postgres,queue-scylla,sendgrid,sendpulse,mailgun,resend,mailjet,brevo,bird,dotenv,test-utils -- -D warnings
cargo doc --no-deps --no-default-features --features hyvor-relay,api,rustls,queue-memory,rate-limit,smtp,telemetry,queue-sqlite,queue-postgres,queue-scylla,sendgrid,sendpulse,mailgun,resend,mailjet,brevo,bird,dotenv
cargo publish --dry-run
```

## Code Style

- Keep every `mod.rs` minimal: module declarations and `pub use` re-exports
  only.
- Keep provider-specific request/response mapping inside provider modules.
- Keep application-facing types re-exported from `lib.rs`.
- Use typed errors from `MailError`; do not use `.unwrap()` in library code.
- Prefer borrowed values and avoid unnecessary clones.
- Use `SecretString` for credentials and tokens.
- Never log API keys, SMTP passwords, OAuth tokens, message bodies, attachment
  content, or full recipient lists.
- Gate optional providers, queues, telemetry, and SMTP integrations behind
  Cargo features.

## Tests

All new public types and helper functions should have focused unit tests.
Provider integrations should use mock HTTP or mock transport tests by default.
Live provider tests must be opt-in and skipped in normal CI unless a dedicated
manual workflow enables them.

Persistent backend tests are local opt-in only:

```sh
MAILBRIDGE_RUN_PERSISTENT_TESTS=true cargo test --features queue-postgres queue::postgres
MAILBRIDGE_RUN_PERSISTENT_TESTS=true cargo test --features queue-scylla queue::scylla
```

## Pull Requests

Good pull requests should:

- explain the user-facing behavior change;
- include tests for changed behavior;
- update docs for public API or feature changes;
- keep unrelated refactors out of the same change;
- pass formatting, tests, clippy, and publish dry-run checks.

## Commit Messages

Use clear, descriptive commit messages. Prefer a short imperative subject, for
example:

```text
Add SMTP preset validation
Document queue backend behavior
Fix temporary error classification
```

## Security Issues

Please do not open public issues for security vulnerabilities. Follow the
process in [SECURITY.md](SECURITY.md).
