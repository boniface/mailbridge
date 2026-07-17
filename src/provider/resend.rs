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

const DEFAULT_BASE_URL: &str = "https://api.resend.com";

#[derive(Debug, Clone)]
pub struct ResendConfig {
    api_key: SecretString,
    base_url: Url,
    allowed_from_domains: BTreeSet<String>,
    request_timeout: Duration,
}

impl ResendConfig {
    /// Builds Resend configuration from an API key and shared Mailbridge
    /// configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when the API key or default base URL is invalid.
    pub fn new(api_key: impl Into<String>, config: &MailbridgeConfig) -> Result<Self> {
        let api_key = api_key.into();
        validate_secret(&api_key, "resend")?;

        Ok(Self {
            api_key: SecretString::new(api_key.into_boxed_str()),
            base_url: parse_base_url(DEFAULT_BASE_URL, "resend")?,
            allowed_from_domains: cloned_domains(config.allowed_from_domains()),
            request_timeout: config.request_timeout(),
        })
    }

    /// Builds Resend configuration from `MAILBRIDGE_RESEND_*` environment
    /// variables and shared Mailbridge configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when required variables are missing or malformed.
    pub fn from_env(config: &MailbridgeConfig) -> Result<Self> {
        let mut value = Self::new(
            secret_from_env("MAILBRIDGE_RESEND_API_KEY")?.expose_secret(),
            config,
        )?;
        if let Some(base_url) = optional_env("MAILBRIDGE_RESEND_BASE_URL") {
            value = value.with_base_url(&base_url)?;
        }
        Ok(value)
    }

    /// Sets the Resend API base URL.
    ///
    /// # Errors
    ///
    /// Returns an error when `base_url` is not a valid URL.
    pub fn with_base_url(mut self, base_url: impl AsRef<str>) -> Result<Self> {
        self.base_url = parse_base_url(base_url.as_ref(), "resend")?;
        Ok(self)
    }

    #[must_use]
    pub const fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }
}

#[derive(Debug, Clone)]
pub struct ResendProvider {
    http: Option<reqwest::Client>,
    send_url: Url,
    api_key: SecretString,
    allowed_from_domains: BTreeSet<String>,
}

impl Default for ResendProvider {
    fn default() -> Self {
        Self {
            http: None,
            send_url: Url::parse(DEFAULT_BASE_URL).expect("default resend base url must be valid"),
            api_key: SecretString::new(String::new().into_boxed_str()),
            allowed_from_domains: BTreeSet::new(),
        }
    }
}

impl ResendProvider {
    /// Builds a configured Resend provider.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client or endpoint URL cannot be built.
    pub fn from_config(config: &ResendConfig) -> Result<Self> {
        Ok(Self {
            http: Some(configured_http_client(config.request_timeout)?),
            send_url: join_url(&config.base_url, "emails", "resend")?,
            api_key: secret_copy(&config.api_key),
            allowed_from_domains: cloned_domains(&config.allowed_from_domains),
        })
    }
}

#[async_trait]
impl MailProvider for ResendProvider {
    async fn send(&self, message: &EmailMessage) -> Result<SendReceipt> {
        let http = self
            .http
            .as_ref()
            .ok_or_else(|| MailError::Config("resend provider is not configured".to_owned()))?;
        message.validate()?;
        message.validate_sender_domain(&self.allowed_from_domains)?;

        let mut builder = http
            .post(self.send_url.clone())
            .bearer_auth(self.api_key.expose_secret())
            .json(&ResendRequest::from_message(message));
        if let Some(key) = message.idempotency_key() {
            builder = builder.header("Idempotency-Key", key);
        }

        let response = builder
            .send()
            .await
            .map_err(|error| MailError::Temporary(format!("resend request failed: {error}")))?;
        let status = response.status();

        if status.is_success() {
            let body = response
                .json::<ResendSendResponse>()
                .await
                .map_err(|error| {
                    MailError::Temporary(format!("resend success response was invalid: {error}"))
                })?;
            let provider_id = Some(body.id);
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
        "resend"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::new()
            .with_idempotency()
            .with_attachments()
            .with_custom_headers()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct ResendRequest<'a> {
    from: String,
    to: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    cc: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    bcc: Vec<String>,
    subject: Cow<'a, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<Cow<'a, str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    html: Option<Cow<'a, str>>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    headers: BTreeMap<Cow<'a, str>, Cow<'a, str>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<ResendAttachment<'a>>,
}

impl<'a> ResendRequest<'a> {
    fn from_message(message: &'a EmailMessage) -> Self {
        Self {
            from: message.from_address().formatted(),
            to: formatted_addresses(message.to()),
            cc: formatted_addresses(message.cc()),
            bcc: formatted_addresses(message.bcc()),
            subject: Cow::Borrowed(message.subject()),
            text: message.body_text().map(Cow::Borrowed),
            html: message.body_html().map(Cow::Borrowed),
            headers: message
                .headers()
                .iter()
                .map(|(name, value)| (Cow::Borrowed(name.as_str()), Cow::Borrowed(value.as_str())))
                .collect(),
            attachments: message
                .attachments()
                .iter()
                .map(ResendAttachment::from)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct ResendAttachment<'a> {
    filename: Cow<'a, str>,
    content: String,
}

impl<'a> From<&'a Attachment> for ResendAttachment<'a> {
    fn from(attachment: &'a Attachment) -> Self {
        Self {
            filename: Cow::Borrowed(attachment.file_name()),
            content: bytes_base64(attachment.content()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct ResendSendResponse {
    id: String,
}

fn formatted_addresses(addresses: &[EmailAddress]) -> Vec<String> {
    addresses.iter().map(EmailAddress::formatted).collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::provider::shared::test_support::test_server;

    #[test]
    fn request_serialization_matches_resend_shape() {
        let message = EmailMessage::builder()
            .from("App", "sender@example.com")
            .expect("valid from")
            .to("User", "user@example.net")
            .expect("valid to")
            .subject("Hello")
            .text("Plain")
            .header("X-App", "mailbridge")
            .expect("valid header")
            .build()
            .expect("valid message");

        let json = serde_json::to_value(ResendRequest::from_message(&message)).expect("json");

        assert_eq!(json["from"], "App <sender@example.com>");
        assert_eq!(json["to"][0], "User <user@example.net>");
        assert_eq!(json["headers"]["X-App"], "mailbridge");
    }

    #[tokio::test]
    async fn send_posts_to_emails_endpoint_with_idempotency() {
        let (base_url, request_rx) = test_server(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 18\r\n\r\n{\"id\":\"email-123\"}",
        );
        let config = shared_config();
        let provider_config = ResendConfig::new("secret", &config)
            .expect("valid config")
            .with_base_url(base_url)
            .expect("valid base url");
        let provider = ResendProvider::from_config(&provider_config).expect("provider builds");
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

        assert_eq!(receipt.message_id().as_str(), "email-123");
        assert!(request.contains("post /emails http/1.1"));
        assert!(request.contains("authorization: bearer secret"));
        assert!(request.contains("idempotency-key: order-123"));
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
