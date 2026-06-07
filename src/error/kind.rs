pub type Result<T> = std::result::Result<T, MailError>;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MailError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("invalid email message: {0}")]
    Validation(String),

    #[error("sender domain is not allowed: {domain}")]
    SenderDomainNotAllowed { domain: String },

    #[error("request rate limited")]
    RateLimited,

    #[error("relay authentication failed")]
    Authentication,

    #[error("relay rejected request: status={status}, message={message}")]
    RelayRejected { status: u16, message: String },

    #[error("temporary delivery failure: {0}")]
    Temporary(String),

    #[error("queue error: {0}")]
    Queue(String),
}

impl MailError {
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        match self {
            Self::RateLimited | Self::Temporary(_) => true,
            Self::RelayRejected { status, .. } => *status == 429 || *status >= 500,
            Self::Config(_)
            | Self::Validation(_)
            | Self::SenderDomainNotAllowed { .. }
            | Self::Authentication
            | Self::Queue(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_classification_marks_temporary_errors_retryable() {
        let error = MailError::Temporary("timeout".to_owned());

        assert!(error.is_retryable());
    }

    #[test]
    fn retry_classification_marks_validation_errors_permanent() {
        let error = MailError::Validation("missing body".to_owned());

        assert!(!error.is_retryable());
    }
}
