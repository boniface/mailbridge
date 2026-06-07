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

#[async_trait]
pub trait MailProvider: Send + Sync {
    async fn send(&self, message: &EmailMessage) -> Result<SendReceipt>;

    async fn get_status(&self, _id: &MessageId) -> Result<Option<SendStatus>> {
        Ok(None)
    }

    fn provider_name(&self) -> &'static str;
}
