use serde::{Deserialize, Serialize};

use crate::error::{MailError, Result};

const MAX_EMAIL_BYTES: usize = 254;
const MAX_LOCAL_BYTES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailAddress {
    name: Option<String>,
    email: String,
    domain: String,
}

impl EmailAddress {
    pub fn new(name: impl Into<String>, email: impl Into<String>) -> Result<Self> {
        let name = normalize_name(name.into())?;
        let email = email.into();
        let domain = parse_domain(&email)?;

        Ok(Self {
            name,
            email,
            domain,
        })
    }

    pub fn without_name(email: impl Into<String>) -> Result<Self> {
        let email = email.into();
        let domain = parse_domain(&email)?;

        Ok(Self {
            name: None,
            email,
            domain,
        })
    }

    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    #[must_use]
    pub fn email(&self) -> &str {
        &self.email
    }

    #[must_use]
    pub fn domain(&self) -> &str {
        &self.domain
    }

    #[must_use]
    pub fn formatted(&self) -> String {
        match self.name() {
            Some(name) => format!("{} <{}>", format_display_name(name), self.email),
            None => self.email.clone(),
        }
    }
}

fn normalize_name(name: String) -> Result<Option<String>> {
    validate_no_control_chars(&name, "address name")?;

    if name.trim().is_empty() {
        return Ok(None);
    }

    Ok(Some(name.trim().to_owned()))
}

fn parse_domain(email: &str) -> Result<String> {
    validate_no_control_chars(email, "email address")?;

    let trimmed = email.trim();
    if trimmed != email {
        return Err(MailError::Validation(
            "email address must not contain surrounding whitespace".to_owned(),
        ));
    }

    let (local, domain) = trimmed
        .rsplit_once('@')
        .ok_or_else(|| MailError::Validation("email address must contain @".to_owned()))?;

    if local.is_empty() || domain.is_empty() || domain.contains('@') {
        return Err(MailError::Validation(
            "email address must contain one local part and one domain".to_owned(),
        ));
    }

    if trimmed.len() > MAX_EMAIL_BYTES {
        return Err(MailError::Validation(format!(
            "email address must not exceed {MAX_EMAIL_BYTES} bytes"
        )));
    }

    validate_local_part(local)?;
    validate_domain(domain)?;

    Ok(domain.to_ascii_lowercase())
}

fn validate_local_part(local: &str) -> Result<()> {
    if local.len() > MAX_LOCAL_BYTES {
        return Err(MailError::Validation(format!(
            "email local part must not exceed {MAX_LOCAL_BYTES} bytes"
        )));
    }

    if local.starts_with('.') || local.ends_with('.') || local.contains("..") {
        return Err(MailError::Validation(
            "email local part has invalid dot placement".to_owned(),
        ));
    }

    if !local.bytes().all(is_allowed_local_byte) {
        return Err(MailError::Validation(
            "email local part contains unsupported characters".to_owned(),
        ));
    }

    Ok(())
}

fn validate_domain(domain: &str) -> Result<()> {
    if domain.starts_with('.') || domain.ends_with('.') || !domain.contains('.') {
        return Err(MailError::Validation(
            "email domain must contain at least one dot and no edge dots".to_owned(),
        ));
    }

    for label in domain.split('.') {
        validate_domain_label(label)?;
    }

    Ok(())
}

fn validate_domain_label(label: &str) -> Result<()> {
    if label.is_empty() || label.len() > 63 {
        return Err(MailError::Validation(
            "email domain labels must be between 1 and 63 bytes".to_owned(),
        ));
    }

    if label.starts_with('-')
        || label.ends_with('-')
        || !label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(MailError::Validation(
            "email domain labels must contain only letters, digits, or interior hyphens".to_owned(),
        ));
    }

    Ok(())
}

fn validate_no_control_chars(value: &str, field: &str) -> Result<()> {
    if value.chars().any(char::is_control) {
        return Err(MailError::Validation(format!(
            "{field} must not contain control characters"
        )));
    }

    Ok(())
}

fn is_allowed_local_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'.' | b'!'
                | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'/'
                | b'='
                | b'?'
                | b'^'
                | b'_'
                | b'`'
                | b'{'
                | b'|'
                | b'}'
                | b'~'
        )
}

fn format_display_name(name: &str) -> String {
    if name.bytes().all(is_safe_atom_display_byte) {
        return name.to_owned();
    }

    let escaped = name
        .chars()
        .fold(String::with_capacity(name.len() + 2), |mut out, ch| {
            if ch == '"' || ch == '\\' {
                out.push('\\');
            }
            out.push(ch);
            out
        });
    format!("\"{escaped}\"")
}

fn is_safe_atom_display_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'-' | b'_' | b'.' | b'\'')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_formats_named_recipient() {
        let address = EmailAddress::new("User", "user@example.com").expect("valid address");

        assert_eq!(address.formatted(), "User <user@example.com>");
        assert_eq!(address.domain(), "example.com");
    }

    #[test]
    fn address_rejects_header_injection() {
        let error = EmailAddress::new("User\r\nBcc: x@example.com", "user@example.com")
            .expect_err("header injection should fail");

        assert!(matches!(error, MailError::Validation(_)));
    }

    #[test]
    fn address_rejects_malformed_mailboxes() {
        for email in [
            "user example.com",
            "user@example",
            ".user@example.com",
            "user..name@example.com",
            "user@example..com",
            "user@-example.com",
        ] {
            let error = EmailAddress::without_name(email).expect_err("invalid mailbox");

            assert!(matches!(error, MailError::Validation(_)));
        }
    }

    #[test]
    fn display_name_is_quoted_when_needed() {
        let address = EmailAddress::new("Example, Inc.", "user@example.com").expect("valid");

        assert_eq!(address.formatted(), "\"Example, Inc.\" <user@example.com>");
    }
}
