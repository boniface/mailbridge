#[cfg(feature = "smtp")]
use async_trait::async_trait;
#[cfg(feature = "smtp")]
use lettre::message::{
    Attachment as LettreAttachment, Mailbox, MultiPart, SinglePart, header::ContentType,
};
#[cfg(feature = "smtp")]
use lettre::transport::smtp::Error as SmtpError;
#[cfg(feature = "smtp")]
use lettre::transport::smtp::authentication::Credentials;
#[cfg(feature = "smtp")]
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
#[cfg(feature = "smtp")]
use secrecy::{ExposeSecret, SecretString};
#[cfg(feature = "smtp")]
use std::collections::BTreeSet;

#[cfg(feature = "smtp")]
use crate::client::{MessageId, SendReceipt};
#[cfg(feature = "smtp")]
use crate::config::MailbridgeConfig;
#[cfg(feature = "smtp")]
use crate::email::{Attachment, EmailAddress, EmailMessage};
#[cfg(feature = "smtp")]
use crate::error::{MailError, Result};
#[cfg(feature = "smtp")]
use crate::provider::{MailProvider, SendStatus};

#[cfg(feature = "smtp")]
#[derive(Debug, Clone)]
pub struct SmtpClient {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    allowed_from_domains: BTreeSet<String>,
}

#[cfg(feature = "smtp")]
impl SmtpClient {
    pub fn from_config(config: &MailbridgeConfig) -> Result<Self> {
        let smtp = config
            .smtp()
            .ok_or_else(|| MailError::Config("smtp configuration is required".to_owned()))?;
        let credentials = Credentials::new(
            smtp.username().to_owned(),
            secret_copy(smtp.password()).expose_secret().to_owned(),
        );
        let transport = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(smtp.host())
            .map_err(|error| MailError::Config(format!("failed to build smtp transport: {error}")))?
            .port(smtp.port())
            .credentials(credentials)
            .build();

        Ok(Self {
            transport,
            allowed_from_domains: config.allowed_from_domains().clone(),
        })
    }
}

#[cfg(feature = "smtp")]
#[async_trait]
impl MailProvider for SmtpClient {
    async fn send(&self, message: &EmailMessage) -> Result<SendReceipt> {
        message.validate()?;
        message.validate_sender_domain(&self.allowed_from_domains)?;

        let message_id = message
            .idempotency_key()
            .map_or_else(|| uuid::Uuid::new_v4().to_string(), str::to_owned);
        let message = lettre_message(message)?;
        self.transport.send(message).await.map_err(map_smtp_error)?;

        Ok(SendReceipt::new(
            self.provider_name(),
            MessageId::new(message_id),
            None,
        ))
    }

    async fn get_status(&self, _id: &MessageId) -> Result<Option<SendStatus>> {
        Ok(None)
    }

    fn provider_name(&self) -> &'static str {
        "smtp"
    }
}

#[cfg(feature = "smtp")]
fn lettre_message(message: &EmailMessage) -> Result<Message> {
    let builder = message.to().iter().try_fold(
        Message::builder().from(mailbox(message.from_address())?),
        |builder, to| Ok::<_, MailError>(builder.to(mailbox(to)?)),
    )?;
    let builder = message.cc().iter().try_fold(builder, |builder, cc| {
        Ok::<_, MailError>(builder.cc(mailbox(cc)?))
    })?;
    let builder = message
        .bcc()
        .iter()
        .try_fold(builder, |builder, bcc| {
            Ok::<_, MailError>(builder.bcc(mailbox(bcc)?))
        })?
        .subject(message.subject());

    if message.attachments().is_empty() {
        return match (message.body_text(), message.body_html()) {
            (Some(text), Some(html)) => builder
                .multipart(MultiPart::alternative_plain_html(
                    text.to_owned(),
                    html.to_owned(),
                ))
                .map_err(|error| MailError::Validation(format!("invalid smtp message: {error}"))),
            (Some(text), None) => builder
                .body(text.to_owned())
                .map_err(|error| MailError::Validation(format!("invalid smtp message: {error}"))),
            (None, Some(html)) => builder
                .singlepart(SinglePart::html(html.to_owned()))
                .map_err(|error| MailError::Validation(format!("invalid smtp message: {error}"))),
            (None, None) => Err(MailError::Validation(
                "at least one message body is required".to_owned(),
            )),
        };
    }

    let multipart = message.attachments().iter().try_fold(
        MultiPart::mixed().multipart(body_part(message)?),
        |multipart, attachment| {
            Ok::<_, MailError>(multipart.singlepart(attachment_part(attachment)?))
        },
    )?;

    builder
        .multipart(multipart)
        .map_err(|error| MailError::Validation(format!("invalid smtp message: {error}")))
}

#[cfg(feature = "smtp")]
fn body_part(message: &EmailMessage) -> Result<MultiPart> {
    match (message.body_text(), message.body_html()) {
        (Some(text), Some(html)) => Ok(MultiPart::alternative_plain_html(
            text.to_owned(),
            html.to_owned(),
        )),
        (Some(text), None) => {
            Ok(MultiPart::alternative().singlepart(SinglePart::plain(text.to_owned())))
        }
        (None, Some(html)) => {
            Ok(MultiPart::alternative().singlepart(SinglePart::html(html.to_owned())))
        }
        (None, None) => Err(MailError::Validation(
            "at least one message body is required".to_owned(),
        )),
    }
}

#[cfg(feature = "smtp")]
fn attachment_part(attachment: &Attachment) -> Result<SinglePart> {
    let content_type = ContentType::parse(attachment.content_type()).map_err(|error| {
        MailError::Validation(format!(
            "invalid attachment content type {}: {error}",
            attachment.content_type()
        ))
    })?;

    Ok(LettreAttachment::new(attachment.file_name().to_owned())
        .body(attachment.content().to_vec(), content_type))
}

#[cfg(feature = "smtp")]
fn mailbox(address: &EmailAddress) -> Result<Mailbox> {
    address
        .formatted()
        .parse::<Mailbox>()
        .map_err(|error| MailError::Validation(format!("invalid smtp mailbox: {error}")))
}

#[cfg(feature = "smtp")]
fn secret_copy(secret: &SecretString) -> SecretString {
    SecretString::new(secret.expose_secret().to_owned().into_boxed_str())
}

#[cfg(feature = "smtp")]
fn map_smtp_error(error: SmtpError) -> MailError {
    let message = format!("smtp send failed: {error}");
    if error.is_permanent() {
        return MailError::RelayRejected {
            status: 400,
            message,
        };
    }

    MailError::Temporary(message)
}

#[cfg(not(feature = "smtp"))]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SmtpClient;

#[cfg(all(test, feature = "smtp"))]
mod tests {
    use super::*;

    #[test]
    fn smtp_message_maps_plain_text_message() {
        let message = EmailMessage::builder()
            .from("App", "sender@example.com")
            .expect("valid from")
            .to("User", "user@example.net")
            .expect("valid to")
            .subject("Hello")
            .text("Body")
            .build()
            .expect("valid message");

        let smtp = lettre_message(&message).expect("smtp message should build");

        assert_eq!(smtp.envelope().to().len(), 1);
    }
}
