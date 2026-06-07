use serde::{Deserialize, Serialize};

use crate::error::{MailError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    file_name: String,
    content_type: String,
    content: Vec<u8>,
}

impl Attachment {
    pub fn new(
        file_name: impl Into<String>,
        content_type: impl Into<String>,
        content: Vec<u8>,
    ) -> Result<Self> {
        let file_name = file_name.into();
        let content_type = content_type.into();

        if file_name.trim().is_empty() {
            return Err(MailError::Validation(
                "attachment file name is required".to_owned(),
            ));
        }

        validate_no_control_chars("attachment file name", &file_name)?;

        if content_type.trim().is_empty() {
            return Err(MailError::Validation(
                "attachment content type is required".to_owned(),
            ));
        }

        validate_no_control_chars("attachment content type", &content_type)?;
        validate_content_type(&content_type)?;

        Ok(Self {
            file_name,
            content_type,
            content,
        })
    }

    #[must_use]
    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    #[must_use]
    pub fn content_type(&self) -> &str {
        &self.content_type
    }

    #[must_use]
    pub fn content(&self) -> &[u8] {
        &self.content
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.content.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }
}

fn validate_no_control_chars(label: &str, value: &str) -> Result<()> {
    if value.chars().any(char::is_control) {
        return Err(MailError::Validation(format!(
            "{label} must not contain control characters"
        )));
    }

    Ok(())
}

fn validate_content_type(value: &str) -> Result<()> {
    let (primary, sub) = value.split_once('/').ok_or_else(|| {
        MailError::Validation("attachment content type must contain /".to_owned())
    })?;

    if !is_token(primary) || !is_token(sub) {
        return Err(MailError::Validation(
            "attachment content type must be a valid media type".to_owned(),
        ));
    }

    Ok(())
}

fn is_token(value: &str) -> bool {
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
    fn attachment_rejects_header_injection() {
        let error = Attachment::new("invoice\r\nx", "application/pdf", vec![1])
            .expect_err("control characters should be rejected");

        assert!(matches!(error, MailError::Validation(_)));
    }

    #[test]
    fn attachment_rejects_invalid_content_type() {
        let error = Attachment::new("invoice.pdf", "application pdf", vec![1])
            .expect_err("invalid media type should be rejected");

        assert!(matches!(error, MailError::Validation(_)));
    }
}
