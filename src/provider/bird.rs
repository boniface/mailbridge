use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use url::Url;

use crate::client::{MessageId, SendReceipt};
use crate::config::MailbridgeConfig;
use crate::email::{EmailAddress, EmailMessage};
use crate::error::{MailError, Result};
use crate::provider::shared::{
    cloned_domains, configured_http_client, join_url, optional_env, parse_base_url, provider_error,
    response_text, secret_copy, secret_from_env, validate_secret,
};
use crate::provider::{MailProvider, ProviderCapabilities, SendStatus};

const DEFAULT_BASE_URL: &str = "https://us1.platform.bird.com";

#[derive(Debug, Clone)]
pub struct BirdConfig {
    token: SecretString,
    base_url: Url,
    allowed_from_domains: BTreeSet<String>,
    request_timeout: Duration,
}

impl BirdConfig {
    /// Builds Bird configuration from a bearer token and shared Mailbridge
    /// configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when the token or default base URL is invalid.
    pub fn new(token: impl Into<String>, config: &MailbridgeConfig) -> Result<Self> {
        let token = token.into();
        validate_secret(&token, "bird")?;

        Ok(Self {
            token: SecretString::new(token.into_boxed_str()),
            base_url: parse_base_url(DEFAULT_BASE_URL, "bird")?,
            allowed_from_domains: cloned_domains(config.allowed_from_domains()),
            request_timeout: config.request_timeout(),
        })
    }

    /// Builds Bird configuration from `MAILBRIDGE_BIRD_*` environment variables
    /// and shared Mailbridge configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when required variables are missing or malformed.
    pub fn from_env(config: &MailbridgeConfig) -> Result<Self> {
        let mut value = Self::new(
            secret_from_env("MAILBRIDGE_BIRD_TOKEN")?.expose_secret(),
            config,
        )?;
        if let Some(base_url) = optional_env("MAILBRIDGE_BIRD_BASE_URL") {
            value = value.with_base_url(&base_url)?;
        }
        Ok(value)
    }

    /// Sets the Bird API base URL.
    ///
    /// # Errors
    ///
    /// Returns an error when `base_url` is not a valid URL.
    pub fn with_base_url(mut self, base_url: impl AsRef<str>) -> Result<Self> {
        self.base_url = parse_base_url(base_url.as_ref(), "bird")?;
        Ok(self)
    }

    #[must_use]
    pub const fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }
}

#[derive(Debug, Clone)]
pub struct BirdProvider {
    http: Option<reqwest::Client>,
    send_url: Url,
    token: SecretString,
    allowed_from_domains: BTreeSet<String>,
}

impl Default for BirdProvider {
    fn default() -> Self {
        Self {
            http: None,
            send_url: Url::parse(DEFAULT_BASE_URL).expect("default bird base url must be valid"),
            token: SecretString::new(String::new().into_boxed_str()),
            allowed_from_domains: BTreeSet::new(),
        }
    }
}

impl BirdProvider {
    /// Builds a configured Bird provider.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client or endpoint URL cannot be built.
    pub fn from_config(config: &BirdConfig) -> Result<Self> {
        Ok(Self {
            http: Some(configured_http_client(config.request_timeout)?),
            send_url: join_url(&config.base_url, "v1/email/messages", "bird")?,
            token: secret_copy(&config.token),
            allowed_from_domains: cloned_domains(&config.allowed_from_domains),
        })
    }
}

#[async_trait]
impl MailProvider for BirdProvider {
    async fn send(&self, message: &EmailMessage) -> Result<SendReceipt> {
        let http = self
            .http
            .as_ref()
            .ok_or_else(|| MailError::Config("bird provider is not configured".to_owned()))?;
        message.validate()?;
        message.validate_sender_domain(&self.allowed_from_domains)?;
        if !message.attachments().is_empty() {
            return Err(MailError::Validation(
                "bird provider does not support attachments through this API".to_owned(),
            ));
        }

        let response = http
            .post(self.send_url.clone())
            .bearer_auth(self.token.expose_secret())
            .json(&BirdRequest::from_message(message))
            .send()
            .await
            .map_err(|error| MailError::Temporary(format!("bird request failed: {error}")))?;
        let status = response.status();

        if status.is_success() {
            let body = response
                .json::<serde_json::Value>()
                .await
                .map_err(|error| {
                    MailError::Temporary(format!("bird success response was invalid: {error}"))
                })?;
            let provider_id = first_field_string(&body, &["id", "messageId", "message_id"]);
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
        "bird"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::new()
            .with_custom_headers()
            .with_regions()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct BirdRequest<'a> {
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
}

impl<'a> BirdRequest<'a> {
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
        }
    }
}

fn formatted_addresses(addresses: &[EmailAddress]) -> Vec<String> {
    addresses.iter().map(EmailAddress::formatted).collect()
}

fn first_field_string(value: &serde_json::Value, fields: &[&str]) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => fields
            .iter()
            .find_map(|field| map.get(*field).and_then(json_scalar_string))
            .or_else(|| {
                map.values()
                    .find_map(|value| first_field_string(value, fields))
            }),
        serde_json::Value::Array(values) => values
            .iter()
            .find_map(|value| first_field_string(value, fields)),
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => None,
    }
}

fn json_scalar_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(value.to_owned()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        serde_json::Value::Null | serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::email::Attachment;
    use crate::provider::shared::test_support::test_server;

    #[test]
    fn request_serialization_matches_bird_shape() {
        let message = EmailMessage::builder()
            .from("App", "sender@example.com")
            .expect("valid from")
            .to("User", "user@example.net")
            .expect("valid to")
            .subject("Hello")
            .text("Plain")
            .build()
            .expect("valid message");

        let json = serde_json::to_value(BirdRequest::from_message(&message)).expect("json");

        assert_eq!(json["from"], "App <sender@example.com>");
        assert_eq!(json["to"][0], "User <user@example.net>");
        assert_eq!(json["text"], "Plain");
    }

    #[tokio::test]
    async fn send_posts_to_email_messages_endpoint() {
        let (base_url, request_rx) = test_server(
            "HTTP/1.1 202 Accepted\r\nContent-Type: application/json\r\nContent-Length: 17\r\n\r\n{\"id\":\"bird-123\"}",
        );
        let config = shared_config();
        let provider_config = BirdConfig::new("secret", &config)
            .expect("valid config")
            .with_base_url(base_url)
            .expect("valid base url");
        let provider = BirdProvider::from_config(&provider_config).expect("provider builds");
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

        assert_eq!(receipt.message_id().as_str(), "bird-123");
        assert!(request.contains("post /v1/email/messages http/1.1"));
        assert!(request.contains("authorization: bearer secret"));
    }

    #[tokio::test]
    async fn send_rejects_attachments_before_http_call() {
        let config = shared_config();
        let provider_config = BirdConfig::new("secret", &config).expect("valid config");
        let provider = BirdProvider::from_config(&provider_config).expect("provider builds");
        let message = EmailMessage::builder()
            .from("App", "sender@example.com")
            .expect("valid from")
            .to("User", "user@example.net")
            .expect("valid to")
            .subject("Hello")
            .text("Body")
            .attachment(
                Attachment::new("test.txt", "text/plain", b"file".to_vec())
                    .expect("valid attachment"),
            )
            .expect("attachment accepted")
            .build()
            .expect("valid message");

        let error = provider.send(&message).await.expect_err("send fails");

        assert!(matches!(error, MailError::Validation(_)));
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
