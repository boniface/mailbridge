# Roadmap

This roadmap describes intended direction, not a guarantee of delivery order.
Provider APIs and hosted email policies can change, so each release should
reconfirm external provider behavior before implementation.

## 0.1.x: Stabilize The First Public Release

- Publish the initial crate to crates.io.
- Keep the Hyvor Relay provider and provider-neutral API stable.
- Improve examples and API docs based on first users.
- Keep CI, release, audit, and publish dry-run checks green.
- Avoid breaking changes unless they fix a release-blocking problem.

## 0.2.0: SMTP Foundation And Mailbox Presets

- Strengthen the generic SMTP provider.
- Add typed SMTP transport configuration.
- Add explicit TLS modes for STARTTLS and implicit TLS.
- Add password/app-password auth validation.
- Add SMTP presets for Gmail, Google Workspace relay, Microsoft 365, Yahoo,
  Yandex, and custom SMTP hosts.
- Document that mailbox SMTP is not equivalent to a transactional relay.

Detailed implementation planning is tracked internally in `dev-docs/`.

## 0.3.0: OAuth For Mailbox Sending

- Add an `AccessTokenProvider` abstraction.
- Add SMTP XOAUTH2 support where provider support is practical.
- Add OAuth-focused examples for Google and Microsoft accounts.
- Keep refresh-token storage outside Mailbridge.

## 0.4.0: Mailbox HTTP API Providers

- Add Gmail API `users.messages.send` provider.
- Add Microsoft Graph `sendMail` provider.
- Add mock HTTP tests for auth failures, throttling, quota responses, and
  sender permission failures.
- Document when to choose SMTP versus provider HTTP APIs.

## Future Provider Work

- Implement SendGrid.
- Implement Mailgun.
- Implement SendPulse.
- Evaluate Amazon SES, Postmark, Resend, and additional relays.
- Add richer provider capability metadata.
- Add provider-specific webhook/status integrations where they fit the
  provider-neutral model.

## Queue And Operations

- Improve operational docs for durable queue backends.
- Add migration guidance for SQL queue schemas.
- Add examples for worker deployment.
- Consider optional metrics exporters after the telemetry surface settles.
