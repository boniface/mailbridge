# Security Policy

Mailbridge handles email credentials, API keys, OAuth tokens, recipient
addresses, message bodies, and attachment content. Security fixes and reports
are taken seriously.

## Supported Versions

Security fixes target the latest released minor version unless a broader fix is
practical. The `main` branch carries unreleased fixes until the next release is
published.

## Reporting A Vulnerability

Please do not report security vulnerabilities through public GitHub issues.

Report privately by contacting the repository owner through GitHub, or by using
GitHub private vulnerability reporting if it is enabled for the repository.
Include:

- affected version or commit;
- feature flags involved;
- a minimal reproduction when possible;
- impact assessment;
- whether credentials, message bodies, or recipient data can be exposed.

## Handling Expectations

- Acknowledgement target: within 7 days.
- Initial assessment target: within 14 days.
- Fix timing depends on severity and release complexity.

## Security Requirements For Contributions

- Do not commit secrets, API keys, SMTP passwords, OAuth tokens, or private
  mailbox data.
- Keep `.env` ignored.
- Use `SecretString` or equivalent secret-bearing types for credentials.
- Redact secrets in `Debug`, errors, logs, telemetry, and test output.
- Do not log message bodies, attachment content, or full recipient lists.
