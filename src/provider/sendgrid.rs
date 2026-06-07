use async_trait::async_trait;

use crate::client::{MessageId, SendReceipt};
use crate::email::EmailMessage;
use crate::error::{MailError, Result};
use crate::provider::{MailProvider, SendStatus};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SendGridProvider;

#[async_trait]
impl MailProvider for SendGridProvider {
    async fn send(&self, _message: &EmailMessage) -> Result<SendReceipt> {
        Err(MailError::Config(
            "sendgrid provider is not implemented in this release".to_owned(),
        ))
    }

    async fn get_status(&self, _id: &MessageId) -> Result<Option<SendStatus>> {
        Err(MailError::Config(
            "sendgrid provider is not implemented in this release".to_owned(),
        ))
    }

    fn provider_name(&self) -> &'static str {
        "sendgrid"
    }
}
