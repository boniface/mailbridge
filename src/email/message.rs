use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::email::{Attachment, EmailAddress};
use crate::error::{MailError, Result};

const MAX_RECIPIENTS: usize = 20;
const MAX_SUBJECT_CHARS: usize = 998;
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 10 * 1024 * 1024;
const MAX_ATTACHMENTS: usize = 10;
const MAX_IDEMPOTENCY_KEY_BYTES: usize = 255;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailMessage {
    from: EmailAddress,
    to: Vec<EmailAddress>,
    cc: Vec<EmailAddress>,
    bcc: Vec<EmailAddress>,
    subject: String,
    body_text: Option<String>,
    body_html: Option<String>,
    headers: BTreeMap<String, String>,
    attachments: Vec<Attachment>,
    idempotency_key: Option<String>,
}

impl EmailMessage {
    #[must_use]
    pub fn builder() -> EmailMessageBuilder {
        EmailMessageBuilder::default()
    }

    #[must_use]
    pub fn from_address(&self) -> &EmailAddress {
        &self.from
    }

    #[must_use]
    pub fn to(&self) -> &[EmailAddress] {
        &self.to
    }

    #[must_use]
    pub fn cc(&self) -> &[EmailAddress] {
        &self.cc
    }

    #[must_use]
    pub fn bcc(&self) -> &[EmailAddress] {
        &self.bcc
    }

    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }

    #[must_use]
    pub fn body_text(&self) -> Option<&str> {
        self.body_text.as_deref()
    }

    #[must_use]
    pub fn body_html(&self) -> Option<&str> {
        self.body_html.as_deref()
    }

    #[must_use]
    pub fn headers(&self) -> &BTreeMap<String, String> {
        &self.headers
    }

    #[must_use]
    pub fn attachments(&self) -> &[Attachment] {
        &self.attachments
    }

    #[must_use]
    pub fn idempotency_key(&self) -> Option<&str> {
        self.idempotency_key.as_deref()
    }

    #[must_use]
    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    #[must_use]
    pub fn ensure_idempotency_key(mut self) -> Self {
        if self.idempotency_key.is_none() {
            self.idempotency_key = Some(uuid::Uuid::new_v4().to_string());
        }

        self
    }

    /// Validates message limits, headers, body content, attachments, and
    /// idempotency metadata.
    ///
    /// # Errors
    ///
    /// Returns an error when the message violates validation constraints.
    pub fn validate(&self) -> Result<()> {
        validate_recipients(self.recipient_count())?;
        validate_bodies(self.body_text(), self.body_html())?;
        validate_subject(&self.subject)?;
        validate_total_size(self)?;
        validate_attachments(&self.attachments)?;
        validate_idempotency_key(self.idempotency_key.as_deref())?;
        self.headers
            .iter()
            .try_for_each(|(name, value)| validate_header(name, value))
    }

    /// Validates the sender domain against an allowlist.
    ///
    /// # Errors
    ///
    /// Returns an error when the sender domain is not present in
    /// `allowed_domains`.
    pub fn validate_sender_domain(&self, allowed_domains: &BTreeSet<String>) -> Result<()> {
        if allowed_domains.contains(self.from.domain()) {
            return Ok(());
        }

        Err(MailError::SenderDomainNotAllowed {
            domain: self.from.domain().to_owned(),
        })
    }

    #[must_use]
    pub fn recipient_count(&self) -> usize {
        self.to.len() + self.cc.len() + self.bcc.len()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmailMessageBuilder {
    from: Option<EmailAddress>,
    to: Vec<EmailAddress>,
    cc: Vec<EmailAddress>,
    bcc: Vec<EmailAddress>,
    subject: Option<String>,
    body_text: Option<String>,
    body_html: Option<String>,
    headers: BTreeMap<String, String>,
    attachments: Vec<Attachment>,
    idempotency_key: Option<String>,
}

impl EmailMessageBuilder {
    /// Sets the sender address.
    ///
    /// # Errors
    ///
    /// Returns an error when the sender address is invalid.
    pub fn from(mut self, name: impl Into<String>, email: impl Into<String>) -> Result<Self> {
        self.from = Some(EmailAddress::new(name, email)?);
        Ok(self)
    }

    /// Adds a `To` recipient.
    ///
    /// # Errors
    ///
    /// Returns an error when the recipient address is invalid.
    pub fn to(mut self, name: impl Into<String>, email: impl Into<String>) -> Result<Self> {
        self.to.push(EmailAddress::new(name, email)?);
        Ok(self)
    }

    /// Adds a `Cc` recipient.
    ///
    /// # Errors
    ///
    /// Returns an error when the recipient address is invalid.
    pub fn cc(mut self, name: impl Into<String>, email: impl Into<String>) -> Result<Self> {
        self.cc.push(EmailAddress::new(name, email)?);
        Ok(self)
    }

    /// Adds a `Bcc` recipient.
    ///
    /// # Errors
    ///
    /// Returns an error when the recipient address is invalid.
    pub fn bcc(mut self, name: impl Into<String>, email: impl Into<String>) -> Result<Self> {
        self.bcc.push(EmailAddress::new(name, email)?);
        Ok(self)
    }

    #[must_use]
    pub fn subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }

    #[must_use]
    pub fn text(mut self, body: impl Into<String>) -> Self {
        self.body_text = Some(body.into());
        self
    }

    #[must_use]
    pub fn html(mut self, body: impl Into<String>) -> Self {
        self.body_html = Some(body.into());
        self
    }

    /// Adds a custom header.
    ///
    /// # Errors
    ///
    /// Returns an error when the header name is forbidden or malformed, or the
    /// value contains control characters.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Result<Self> {
        let name = name.into();
        let value = value.into();
        validate_header(&name, &value)?;
        self.headers.insert(name, value);
        Ok(self)
    }

    /// Adds an attachment.
    ///
    /// # Errors
    ///
    /// Returns an error when the message would exceed the attachment limit.
    pub fn attachment(mut self, attachment: Attachment) -> Result<Self> {
        self.attachments.push(attachment);
        validate_attachments(&self.attachments)?;
        Ok(self)
    }

    #[must_use]
    pub fn idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    /// Builds and validates an email message.
    ///
    /// # Errors
    ///
    /// Returns an error when required fields are missing or validation fails.
    pub fn build(self) -> Result<EmailMessage> {
        let from = self
            .from
            .ok_or_else(|| MailError::Validation("from address is required".to_owned()))?;
        let subject = self.subject.unwrap_or_default();
        let message = EmailMessage {
            from,
            to: self.to,
            cc: self.cc,
            bcc: self.bcc,
            subject,
            body_text: self.body_text,
            body_html: self.body_html,
            headers: self.headers,
            attachments: self.attachments,
            idempotency_key: self.idempotency_key,
        };

        message.validate()?;
        Ok(message)
    }
}

fn validate_recipients(count: usize) -> Result<()> {
    if count == 0 {
        return Err(MailError::Validation(
            "at least one recipient is required".to_owned(),
        ));
    }

    if count > MAX_RECIPIENTS {
        return Err(MailError::Validation(format!(
            "recipient count must not exceed {MAX_RECIPIENTS}"
        )));
    }

    Ok(())
}

fn validate_bodies(body_text: Option<&str>, body_html: Option<&str>) -> Result<()> {
    if body_text.is_none() && body_html.is_none() {
        return Err(MailError::Validation(
            "at least one message body is required".to_owned(),
        ));
    }

    if body_text.is_some_and(|body| body.len() > MAX_BODY_BYTES) {
        return Err(MailError::Validation(format!(
            "text body must not exceed {MAX_BODY_BYTES} bytes"
        )));
    }

    if body_html.is_some_and(|body| body.len() > MAX_BODY_BYTES) {
        return Err(MailError::Validation(format!(
            "html body must not exceed {MAX_BODY_BYTES} bytes"
        )));
    }

    Ok(())
}

fn validate_subject(subject: &str) -> Result<()> {
    if subject.contains('\r') || subject.contains('\n') {
        return Err(MailError::Validation(
            "subject must not contain line breaks".to_owned(),
        ));
    }

    if subject.chars().count() > MAX_SUBJECT_CHARS {
        return Err(MailError::Validation(format!(
            "subject length must not exceed {MAX_SUBJECT_CHARS} characters"
        )));
    }

    Ok(())
}

fn validate_total_size(message: &EmailMessage) -> Result<()> {
    let body_size =
        message.body_text().map_or(0, str::len) + message.body_html().map_or(0, str::len);
    let attachment_size = message
        .attachments()
        .iter()
        .map(Attachment::len)
        .sum::<usize>();

    if body_size + attachment_size > MAX_TOTAL_BYTES {
        return Err(MailError::Validation(format!(
            "total email size must not exceed {MAX_TOTAL_BYTES} bytes"
        )));
    }

    Ok(())
}

fn validate_attachments(attachments: &[Attachment]) -> Result<()> {
    if attachments.len() > MAX_ATTACHMENTS {
        return Err(MailError::Validation(format!(
            "attachment count must not exceed {MAX_ATTACHMENTS}"
        )));
    }

    Ok(())
}

fn validate_header(name: &str, value: &str) -> Result<()> {
    if name.trim().is_empty() {
        return Err(MailError::Validation("header name is required".to_owned()));
    }

    if !is_header_name(name) {
        return Err(MailError::Validation(
            "header name must be an RFC token".to_owned(),
        ));
    }

    if value.chars().any(char::is_control) {
        return Err(MailError::Validation(
            "headers must not contain control characters".to_owned(),
        ));
    }

    if is_forbidden_header(name) {
        return Err(MailError::Validation(format!(
            "header {name} cannot be set by callers"
        )));
    }

    Ok(())
}

fn is_forbidden_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "from"
            | "to"
            | "cc"
            | "bcc"
            | "subject"
            | "date"
            | "message-id"
            | "dkim-signature"
            | "return-path"
            | "received"
            | "sender"
            | "authentication-results"
            | "x-mailer"
            | "x-originating-ip"
            | "content-type"
            | "content-transfer-encoding"
            | "mime-version"
    )
}

fn validate_idempotency_key(key: Option<&str>) -> Result<()> {
    let Some(key) = key else {
        return Ok(());
    };

    if key.trim().is_empty() {
        return Err(MailError::Validation(
            "idempotency key must not be empty".to_owned(),
        ));
    }

    if key.len() > MAX_IDEMPOTENCY_KEY_BYTES {
        return Err(MailError::Validation(format!(
            "idempotency key must not exceed {MAX_IDEMPOTENCY_KEY_BYTES} bytes"
        )));
    }

    if key.chars().any(char::is_control) {
        return Err(MailError::Validation(
            "idempotency key must not contain control characters".to_owned(),
        ));
    }

    Ok(())
}

fn is_header_name(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_creates_valid_message() {
        let message = EmailMessage::builder()
            .from("App", "sender@example.com")
            .expect("valid from")
            .to("User", "user@example.net")
            .expect("valid to")
            .subject("Welcome")
            .text("Hello")
            .build()
            .expect("valid message");

        assert_eq!(message.recipient_count(), 1);
        assert_eq!(message.from_address().domain(), "example.com");
    }

    #[test]
    fn builder_rejects_missing_recipient() {
        let error = EmailMessage::builder()
            .from("App", "sender@example.com")
            .expect("valid from")
            .subject("Welcome")
            .text("Hello")
            .build()
            .expect_err("recipient is required");

        assert!(matches!(error, MailError::Validation(_)));
    }

    #[test]
    fn builder_rejects_forbidden_header() {
        let error = EmailMessage::builder()
            .from("App", "sender@example.com")
            .expect("valid from")
            .to("User", "user@example.net")
            .expect("valid to")
            .subject("Welcome")
            .text("Hello")
            .header("Message-Id", "custom")
            .expect_err("message-id is forbidden");

        assert!(matches!(error, MailError::Validation(_)));
    }

    #[test]
    fn sender_domain_validation_rejects_unknown_domain() {
        let message = EmailMessage::builder()
            .from("App", "sender@example.com")
            .expect("valid from")
            .to("User", "user@example.net")
            .expect("valid to")
            .subject("Welcome")
            .text("Hello")
            .build()
            .expect("valid message");
        let allowed = BTreeSet::from(["allowed.example".to_owned()]);

        let error = message
            .validate_sender_domain(&allowed)
            .expect_err("sender domain should be rejected");

        assert_eq!(
            error,
            MailError::SenderDomainNotAllowed {
                domain: "example.com".to_owned()
            }
        );
    }

    #[test]
    fn builder_rejects_header_name_with_separator() {
        let error = EmailMessage::builder()
            .from("App", "sender@example.com")
            .expect("valid from")
            .to("User", "user@example.net")
            .expect("valid to")
            .subject("Welcome")
            .text("Hello")
            .header("X App", "accounts")
            .expect_err("header name with space is invalid");

        assert!(matches!(error, MailError::Validation(_)));
    }

    #[test]
    fn builder_rejects_invalid_idempotency_key() {
        let error = EmailMessage::builder()
            .from("App", "sender@example.com")
            .expect("valid from")
            .to("User", "user@example.net")
            .expect("valid to")
            .subject("Welcome")
            .text("Hello")
            .idempotency_key("order\r\n123")
            .build()
            .expect_err("idempotency key is invalid");

        assert!(matches!(error, MailError::Validation(_)));
    }
}
