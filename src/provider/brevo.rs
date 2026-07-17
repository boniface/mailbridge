use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::client::{MessageId, SendReceipt};
use crate::config::MailbridgeConfig;
use crate::email::{Attachment, EmailAddress, EmailMessage};
use crate::error::{MailError, Result};
use crate::provider::shared::{
    bytes_base64, cloned_domains, configured_http_client, join_url, optional_env, parse_base_url,
    provider_error, response_text, secret_copy, secret_from_env, validate_secret,
};
use crate::provider::{MailProvider, ProviderCapabilities, SendStatus};

const DEFAULT_BASE_URL: &str = "https://api.brevo.com";

#[derive(Debug, Clone)]
pub struct BrevoConfig {
    api_key: SecretString,
    base_url: Url,
    allowed_from_domains: BTreeSet<String>,
    request_timeout: Duration,
}

impl BrevoConfig {
    /// Builds Brevo configuration from an API key and shared Mailbridge
    /// configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when the API key or default base URL is invalid.
    pub fn new(api_key: impl Into<String>, config: &MailbridgeConfig) -> Result<Self> {
        let api_key = api_key.into();
        validate_secret(&api_key, "brevo")?;

        Ok(Self {
            api_key: SecretString::new(api_key.into_boxed_str()),
            base_url: parse_base_url(DEFAULT_BASE_URL, "brevo")?,
            allowed_from_domains: cloned_domains(config.allowed_from_domains()),
            request_timeout: config.request_timeout(),
        })
    }

    /// Builds Brevo configuration from `MAILBRIDGE_BREVO_*` environment
    /// variables and shared Mailbridge configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when required variables are missing or malformed.
    pub fn from_env(config: &MailbridgeConfig) -> Result<Self> {
        let mut value = Self::new(
            secret_from_env("MAILBRIDGE_BREVO_API_KEY")?.expose_secret(),
            config,
        )?;
        if let Some(base_url) = optional_env("MAILBRIDGE_BREVO_BASE_URL") {
            value = value.with_base_url(&base_url)?;
        }
        Ok(value)
    }

    /// Sets the Brevo API base URL.
    ///
    /// # Errors
    ///
    /// Returns an error when `base_url` is not a valid URL.
    pub fn with_base_url(mut self, base_url: impl AsRef<str>) -> Result<Self> {
        self.base_url = parse_base_url(base_url.as_ref(), "brevo")?;
        Ok(self)
    }

    #[must_use]
    pub const fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }
}

#[derive(Debug, Clone)]
pub struct BrevoProvider {
    http: Option<reqwest::Client>,
    send_url: Url,
    api_key: SecretString,
    allowed_from_domains: BTreeSet<String>,
}

impl Default for BrevoProvider {
    fn default() -> Self {
        Self {
            http: None,
            send_url: Url::parse(DEFAULT_BASE_URL).expect("default brevo base url must be valid"),
            api_key: SecretString::new(String::new().into_boxed_str()),
            allowed_from_domains: BTreeSet::new(),
        }
    }
}

impl BrevoProvider {
    /// Builds a configured Brevo provider.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client or endpoint URL cannot be built.
    pub fn from_config(config: &BrevoConfig) -> Result<Self> {
        Ok(Self {
            http: Some(configured_http_client(config.request_timeout)?),
            send_url: join_url(&config.base_url, "v3/smtp/email", "brevo")?,
            api_key: secret_copy(&config.api_key),
            allowed_from_domains: cloned_domains(&config.allowed_from_domains),
        })
    }
}

#[async_trait]
impl MailProvider for BrevoProvider {
    async fn send(&self, message: &EmailMessage) -> Result<SendReceipt> {
        let http = self
            .http
            .as_ref()
            .ok_or_else(|| MailError::Config("brevo provider is not configured".to_owned()))?;
        message.validate()?;
        message.validate_sender_domain(&self.allowed_from_domains)?;

        let response = http
            .post(self.send_url.clone())
            .header("api-key", self.api_key.expose_secret())
            .json(&BrevoRequest::from_message(message))
            .send()
            .await
            .map_err(|error| MailError::Temporary(format!("brevo request failed: {error}")))?;
        let status = response.status();

        if status.is_success() {
            let body = response
                .json::<BrevoSendResponse>()
                .await
                .map_err(|error| {
                    MailError::Temporary(format!("brevo success response was invalid: {error}"))
                })?;
            let provider_id = Some(body.message_id);
            let message_id = provider_id
                .as_deref()
                .or_else(|| message.idempotency_key())
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
        "brevo"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::new()
            .with_attachments()
            .with_custom_headers()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct BrevoRequest<'a> {
    sender: BrevoAddress<'a>,
    to: Vec<BrevoAddress<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    cc: Vec<BrevoAddress<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    bcc: Vec<BrevoAddress<'a>>,
    subject: Cow<'a, str>,
    #[serde(rename = "textContent", skip_serializing_if = "Option::is_none")]
    text_content: Option<Cow<'a, str>>,
    #[serde(rename = "htmlContent", skip_serializing_if = "Option::is_none")]
    html_content: Option<Cow<'a, str>>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    headers: BTreeMap<Cow<'a, str>, Cow<'a, str>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachment: Vec<BrevoAttachment<'a>>,
}

impl<'a> BrevoRequest<'a> {
    fn from_message(message: &'a EmailMessage) -> Self {
        let html_content = message.body_html().map(Cow::Borrowed);
        let text_content = if html_content.is_none() {
            message.body_text().map(Cow::Borrowed)
        } else {
            None
        };

        Self {
            sender: BrevoAddress::from(message.from_address()),
            to: addresses(message.to()),
            cc: addresses(message.cc()),
            bcc: addresses(message.bcc()),
            subject: Cow::Borrowed(message.subject()),
            text_content,
            html_content,
            headers: message
                .headers()
                .iter()
                .map(|(name, value)| (Cow::Borrowed(name.as_str()), Cow::Borrowed(value.as_str())))
                .collect(),
            attachment: message
                .attachments()
                .iter()
                .map(BrevoAttachment::from)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct BrevoAddress<'a> {
    email: Cow<'a, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<Cow<'a, str>>,
}

impl<'a> From<&'a EmailAddress> for BrevoAddress<'a> {
    fn from(address: &'a EmailAddress) -> Self {
        Self {
            email: Cow::Borrowed(address.email()),
            name: address.name().map(Cow::Borrowed),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct BrevoAttachment<'a> {
    name: Cow<'a, str>,
    content: String,
}

impl<'a> From<&'a Attachment> for BrevoAttachment<'a> {
    fn from(attachment: &'a Attachment) -> Self {
        Self {
            name: Cow::Borrowed(attachment.file_name()),
            content: bytes_base64(attachment.content()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct BrevoSendResponse {
    #[serde(rename = "messageId")]
    message_id: String,
}

fn addresses(addresses: &[EmailAddress]) -> Vec<BrevoAddress<'_>> {
    addresses.iter().map(BrevoAddress::from).collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::provider::shared::test_support::test_server;

    #[test]
    fn request_serialization_matches_brevo_shape() {
        let message = EmailMessage::builder()
            .from("App", "sender@example.com")
            .expect("valid from")
            .to("User", "user@example.net")
            .expect("valid to")
            .subject("Hello")
            .html("<p>Plain</p>")
            .build()
            .expect("valid message");

        let json = serde_json::to_value(BrevoRequest::from_message(&message)).expect("json");

        assert_eq!(json["sender"]["email"], "sender@example.com");
        assert_eq!(json["to"][0]["name"], "User");
        assert_eq!(json["htmlContent"], "<p>Plain</p>");
    }

    #[tokio::test]
    async fn send_posts_to_smtp_email_endpoint() {
        let (base_url, request_rx) = test_server(
            "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: 25\r\n\r\n{\"messageId\":\"brevo-123\"}",
        );
        let config = shared_config();
        let provider_config = BrevoConfig::new("secret", &config)
            .expect("valid config")
            .with_base_url(base_url)
            .expect("valid base url");
        let provider = BrevoProvider::from_config(&provider_config).expect("provider builds");
        let message = EmailMessage::builder()
            .from("App", "sender@example.com")
            .expect("valid from")
            .to("User", "user@example.net")
            .expect("valid to")
            .subject("Hello")
            .text("Body")
            .build()
            .expect("valid message");

        let receipt = provider.send(&message).await.expect("send succeeds");
        let request = request_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("request captured");

        assert_eq!(receipt.message_id().as_str(), "brevo-123");
        assert!(request.contains("post /v3/smtp/email http/1.1"));
        assert!(request.contains("api-key: secret"));
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
