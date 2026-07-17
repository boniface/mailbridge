use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use url::Url;

use crate::client::{MessageId, SendReceipt};
use crate::config::MailbridgeConfig;
use crate::email::{Attachment, EmailMessage};
use crate::error::{MailError, Result};
use crate::provider::shared::{
    ApiAddress, addresses, attachment_content_base64, cloned_domains, configured_http_client,
    join_url, optional_env, parse_base_url, provider_error, response_text, secret_copy,
    secret_from_env,
};
use crate::provider::{MailProvider, ProviderCapabilities, SendStatus};

const DEFAULT_BASE_URL: &str = "https://api.sendgrid.com";

#[derive(Debug, Clone)]
pub struct SendGridConfig {
    api_key: SecretString,
    base_url: Url,
    allowed_from_domains: BTreeSet<String>,
    request_timeout: Duration,
}

impl SendGridConfig {
    /// Builds SendGrid configuration from an API key and shared Mailbridge
    /// configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when the default SendGrid base URL is invalid.
    pub fn new(api_key: impl Into<String>, config: &MailbridgeConfig) -> Result<Self> {
        let api_key = api_key.into();
        crate::provider::shared::validate_secret(&api_key, "sendgrid")?;

        Ok(Self {
            api_key: SecretString::new(api_key.into_boxed_str()),
            base_url: parse_base_url(DEFAULT_BASE_URL, "sendgrid")?,
            allowed_from_domains: cloned_domains(config.allowed_from_domains()),
            request_timeout: config.request_timeout(),
        })
    }

    /// Builds SendGrid configuration from `MAILBRIDGE_SENDGRID_*` environment
    /// variables and shared Mailbridge configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when required variables are missing or malformed.
    pub fn from_env(config: &MailbridgeConfig) -> Result<Self> {
        let api_key = secret_from_env("MAILBRIDGE_SENDGRID_API_KEY")?;
        let mut value = Self {
            api_key,
            base_url: parse_base_url(DEFAULT_BASE_URL, "sendgrid")?,
            allowed_from_domains: cloned_domains(config.allowed_from_domains()),
            request_timeout: config.request_timeout(),
        };
        if let Some(base_url) = optional_env("MAILBRIDGE_SENDGRID_BASE_URL") {
            value = value.with_base_url(&base_url)?;
        }
        Ok(value)
    }

    /// Sets the SendGrid API base URL.
    ///
    /// # Errors
    ///
    /// Returns an error when `base_url` is not a valid URL.
    pub fn with_base_url(mut self, base_url: impl AsRef<str>) -> Result<Self> {
        self.base_url = parse_base_url(base_url.as_ref(), "sendgrid")?;
        Ok(self)
    }

    #[must_use]
    pub const fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }
}

#[derive(Debug, Clone)]
pub struct SendGridProvider {
    http: Option<reqwest::Client>,
    send_url: Url,
    api_key: SecretString,
    allowed_from_domains: BTreeSet<String>,
}

impl Default for SendGridProvider {
    fn default() -> Self {
        Self {
            http: None,
            send_url: Url::parse(DEFAULT_BASE_URL)
                .expect("default sendgrid base url must be valid"),
            api_key: SecretString::new(String::new().into_boxed_str()),
            allowed_from_domains: BTreeSet::new(),
        }
    }
}

impl SendGridProvider {
    /// Builds a configured SendGrid provider.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client or endpoint URL cannot be built.
    pub fn from_config(config: &SendGridConfig) -> Result<Self> {
        Ok(Self {
            http: Some(configured_http_client(config.request_timeout)?),
            send_url: join_url(&config.base_url, "v3/mail/send", "sendgrid")?,
            api_key: secret_copy(&config.api_key),
            allowed_from_domains: cloned_domains(&config.allowed_from_domains),
        })
    }
}

#[async_trait]
impl MailProvider for SendGridProvider {
    async fn send(&self, message: &EmailMessage) -> Result<SendReceipt> {
        let http = self
            .http
            .as_ref()
            .ok_or_else(|| MailError::Config("sendgrid provider is not configured".to_owned()))?;
        message.validate()?;
        message.validate_sender_domain(&self.allowed_from_domains)?;

        let request = SendGridRequest::from_message(message);
        let response = http
            .post(self.send_url.clone())
            .bearer_auth(self.api_key.expose_secret())
            .json(&request)
            .send()
            .await
            .map_err(|error| MailError::Temporary(format!("sendgrid request failed: {error}")))?;
        let status = response.status();

        if status == reqwest::StatusCode::ACCEPTED {
            let provider_id = response
                .headers()
                .get("x-message-id")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let message_id = message
                .idempotency_key()
                .map_or_else(|| uuid::Uuid::new_v4().to_string(), str::to_owned);
            return Ok(SendReceipt::new(
                self.provider_name(),
                MessageId::new(message_id),
                provider_id,
            ));
        }

        let text = response_text(response, self.provider_name()).await;
        Err(provider_error(self.provider_name(), status.as_u16(), text))
    }

    async fn get_status(&self, _id: &MessageId) -> Result<Option<SendStatus>> {
        Ok(None)
    }

    fn provider_name(&self) -> &'static str {
        "sendgrid"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::new()
            .with_attachments()
            .with_custom_headers()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct SendGridRequest<'a> {
    personalizations: Vec<Personalization<'a>>,
    from: ApiAddress<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    subject: Option<Cow<'a, str>>,
    content: Vec<Content<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<SendGridAttachment<'a>>,
}

impl<'a> SendGridRequest<'a> {
    fn from_message(message: &'a EmailMessage) -> Self {
        Self {
            personalizations: vec![Personalization::from_message(message)],
            from: ApiAddress::from(message.from_address()),
            subject: Some(Cow::Borrowed(message.subject())),
            content: content(message),
            attachments: message
                .attachments()
                .iter()
                .map(SendGridAttachment::from)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct Personalization<'a> {
    to: Vec<ApiAddress<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    cc: Vec<ApiAddress<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    bcc: Vec<ApiAddress<'a>>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    headers: BTreeMap<Cow<'a, str>, Cow<'a, str>>,
}

impl<'a> Personalization<'a> {
    fn from_message(message: &'a EmailMessage) -> Self {
        Self {
            to: addresses(message.to()),
            cc: addresses(message.cc()),
            bcc: addresses(message.bcc()),
            headers: message
                .headers()
                .iter()
                .map(|(name, value)| (Cow::Borrowed(name.as_str()), Cow::Borrowed(value.as_str())))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct Content<'a> {
    #[serde(rename = "type")]
    content_type: &'static str,
    value: Cow<'a, str>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct SendGridAttachment<'a> {
    content: String,
    #[serde(rename = "type")]
    content_type: Cow<'a, str>,
    filename: Cow<'a, str>,
}

impl<'a> From<&'a Attachment> for SendGridAttachment<'a> {
    fn from(attachment: &'a Attachment) -> Self {
        Self {
            content: attachment_content_base64(attachment),
            content_type: Cow::Borrowed(attachment.content_type()),
            filename: Cow::Borrowed(attachment.file_name()),
        }
    }
}

fn content(message: &EmailMessage) -> Vec<Content<'_>> {
    let mut parts = Vec::with_capacity(2);
    if let Some(text) = message.body_text() {
        parts.push(Content {
            content_type: "text/plain",
            value: Cow::Borrowed(text),
        });
    }
    if let Some(html) = message.body_html() {
        parts.push(Content {
            content_type: "text/html",
            value: Cow::Borrowed(html),
        });
    }
    parts
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::config::MailbridgeConfig;
    use crate::provider::shared::test_support::test_server;

    #[test]
    fn request_serialization_matches_sendgrid_shape() {
        let message = EmailMessage::builder()
            .from("App", "sender@example.com")
            .expect("valid from")
            .to("User", "user@example.net")
            .expect("valid to")
            .subject("Hello")
            .text("Plain")
            .html("<p>Plain</p>")
            .header("X-App", "mailbridge")
            .expect("valid header")
            .build()
            .expect("valid message");

        let json =
            serde_json::to_value(SendGridRequest::from_message(&message)).expect("json request");

        assert_eq!(json["from"]["email"], "sender@example.com");
        assert_eq!(
            json["personalizations"][0]["to"][0]["email"],
            "user@example.net"
        );
        assert_eq!(
            json["personalizations"][0]["headers"]["X-App"],
            "mailbridge"
        );
        assert_eq!(json["content"][0]["type"], "text/plain");
        assert_eq!(json["content"][1]["type"], "text/html");
    }

    #[tokio::test]
    async fn send_posts_json_to_mail_send_endpoint() {
        let (base_url, request_rx) = test_server(
            "HTTP/1.1 202 Accepted\r\nx-message-id: sg-123\r\nContent-Length: 0\r\n\r\n",
        );
        let config = shared_config();
        let sendgrid_config = SendGridConfig::new("secret", &config)
            .expect("valid sendgrid config")
            .with_base_url(base_url)
            .expect("valid base url");
        let provider = SendGridProvider::from_config(&sendgrid_config).expect("provider builds");
        let message = EmailMessage::builder()
            .from("App", "sender@example.com")
            .expect("valid from")
            .to("User", "user@example.net")
            .expect("valid to")
            .subject("Hello")
            .text("Body")
            .idempotency_key("order-123")
            .build()
            .expect("valid message");

        let receipt = provider.send(&message).await.expect("send succeeds");
        let request = request_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("request captured");

        assert_eq!(receipt.message_id().as_str(), "order-123");
        assert_eq!(receipt.provider_id(), Some("sg-123"));
        assert!(request.contains("post /v3/mail/send http/1.1"));
        assert!(request.contains("authorization: bearer secret"));
        assert!(request.contains("\"subject\":\"hello\""));
    }

    fn shared_config() -> MailbridgeConfig {
        MailbridgeConfig::builder()
            .api_base_url("https://relay.example.com")
            .expect("valid relay url")
            .api_key("relay-secret")
            .allowed_from_domain("example.com")
            .build()
            .expect("valid shared config")
    }
}
