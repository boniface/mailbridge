use async_trait::async_trait;

use crate::client::{MessageId, SendReceipt};
use crate::email::EmailMessage;
use crate::error::{MailError, Result};
use crate::provider::{MailProvider, SendStatus};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SendPulseProvider;

#[async_trait]
impl MailProvider for SendPulseProvider {
    async fn send(&self, _message: &EmailMessage) -> Result<SendReceipt> {
        Err(MailError::Config(
            "sendpulse provider is not implemented in this release".to_owned(),
        ))
    }

    async fn get_status(&self, _id: &MessageId) -> Result<Option<SendStatus>> {
        Err(MailError::Config(
            "sendpulse provider is not implemented in this release".to_owned(),
        ))
    }

    fn provider_name(&self) -> &'static str {
        "sendpulse"
    }
}
