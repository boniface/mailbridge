use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::client::{MessageId, SendReceipt};
use crate::email::EmailMessage;
use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SendStatus {
    Queued,
    Sent,
    Delivered,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    supports_idempotency: bool,
    supports_status_lookup: bool,
    supports_sandbox_mode: bool,
    supports_attachments: bool,
    supports_custom_headers: bool,
    supports_templates: bool,
    supports_regions: bool,
}

impl ProviderCapabilities {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            supports_idempotency: false,
            supports_status_lookup: false,
            supports_sandbox_mode: false,
            supports_attachments: false,
            supports_custom_headers: false,
            supports_templates: false,
            supports_regions: false,
        }
    }

    #[must_use]
    pub const fn with_idempotency(mut self) -> Self {
        self.supports_idempotency = true;
        self
    }

    #[must_use]
    pub const fn with_status_lookup(mut self) -> Self {
        self.supports_status_lookup = true;
        self
    }

    #[must_use]
    pub const fn with_sandbox_mode(mut self) -> Self {
        self.supports_sandbox_mode = true;
        self
    }

    #[must_use]
    pub const fn with_attachments(mut self) -> Self {
        self.supports_attachments = true;
        self
    }

    #[must_use]
    pub const fn with_custom_headers(mut self) -> Self {
        self.supports_custom_headers = true;
        self
    }

    #[must_use]
    pub const fn with_templates(mut self) -> Self {
        self.supports_templates = true;
        self
    }

    #[must_use]
    pub const fn with_regions(mut self) -> Self {
        self.supports_regions = true;
        self
    }

    #[must_use]
    pub const fn supports_idempotency(self) -> bool {
        self.supports_idempotency
    }

    #[must_use]
    pub const fn supports_status_lookup(self) -> bool {
        self.supports_status_lookup
    }

    #[must_use]
    pub const fn supports_sandbox_mode(self) -> bool {
        self.supports_sandbox_mode
    }

    #[must_use]
    pub const fn supports_attachments(self) -> bool {
        self.supports_attachments
    }

    #[must_use]
    pub const fn supports_custom_headers(self) -> bool {
        self.supports_custom_headers
    }

    #[must_use]
    pub const fn supports_templates(self) -> bool {
        self.supports_templates
    }

    #[must_use]
    pub const fn supports_regions(self) -> bool {
        self.supports_regions
    }
}

#[async_trait]
pub trait MailProvider: Send + Sync {
    async fn send(&self, message: &EmailMessage) -> Result<SendReceipt>;

    async fn get_status(&self, _id: &MessageId) -> Result<Option<SendStatus>> {
        Ok(None)
    }

    fn provider_name(&self) -> &'static str;

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_builder_sets_expected_flags() {
        let capabilities = ProviderCapabilities::new()
            .with_idempotency()
            .with_attachments()
            .with_custom_headers();

        assert!(capabilities.supports_idempotency());
        assert!(capabilities.supports_attachments());
        assert!(capabilities.supports_custom_headers());
        assert!(!capabilities.supports_sandbox_mode());
    }
}
