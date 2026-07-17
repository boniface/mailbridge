use std::collections::BTreeSet;
use std::time::Duration;

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use url::Url;

use crate::client::{MessageId, SendReceipt};
use crate::config::MailbridgeConfig;
use crate::email::{EmailAddress, EmailMessage};
use crate::error::{MailError, Result};
use crate::provider::shared::{
    cloned_domains, configured_http_client, join_url, optional_env, parse_base_url, provider_error,
    required_env, response_text, secret_copy, secret_from_env, validate_secret,
};
use crate::provider::{MailProvider, ProviderCapabilities, SendStatus};

const DEFAULT_BASE_URL: &str = "https://api.mailgun.net";

#[derive(Debug, Clone)]
pub struct MailgunConfig {
    api_key: SecretString,
    domain: String,
    base_url: Url,
    allowed_from_domains: BTreeSet<String>,
    request_timeout: Duration,
}

impl MailgunConfig {
    /// Builds Mailgun configuration from an API key, sending domain, and shared
    /// Mailbridge configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when the API key, domain, or default base URL is invalid.
    pub fn new(
        api_key: impl Into<String>,
        domain: impl Into<String>,
        config: &MailbridgeConfig,
    ) -> Result<Self> {
        let api_key = api_key.into();
        validate_secret(&api_key, "mailgun")?;
        let domain = validate_domain(domain.into())?;

        Ok(Self {
            api_key: SecretString::new(api_key.into_boxed_str()),
            domain,
            base_url: parse_base_url(DEFAULT_BASE_URL, "mailgun")?,
            allowed_from_domains: cloned_domains(config.allowed_from_domains()),
            request_timeout: config.request_timeout(),
        })
    }

    /// Builds Mailgun configuration from `MAILBRIDGE_MAILGUN_*` environment
    /// variables and shared Mailbridge configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when required variables are missing or malformed.
    pub fn from_env(config: &MailbridgeConfig) -> Result<Self> {
        let api_key = secret_from_env("MAILBRIDGE_MAILGUN_API_KEY")?;
        let domain = validate_domain(required_env("MAILBRIDGE_MAILGUN_DOMAIN")?)?;
        let mut value = Self {
            api_key,
            domain,
            base_url: parse_base_url(DEFAULT_BASE_URL, "mailgun")?,
            allowed_from_domains: cloned_domains(config.allowed_from_domains()),
            request_timeout: config.request_timeout(),
        };
        if let Some(base_url) = optional_env("MAILBRIDGE_MAILGUN_BASE_URL") {
            value = value.with_base_url(&base_url)?;
        }
        Ok(value)
    }

    /// Sets the Mailgun API base URL.
    ///
    /// # Errors
    ///
    /// Returns an error when `base_url` is not a valid URL.
    pub fn with_base_url(mut self, base_url: impl AsRef<str>) -> Result<Self> {
        self.base_url = parse_base_url(base_url.as_ref(), "mailgun")?;
        Ok(self)
    }

    #[must_use]
    pub const fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }
}

#[derive(Debug, Clone)]
pub struct MailgunProvider {
    http: Option<reqwest::Client>,
    send_url: Url,
    api_key: SecretString,
    allowed_from_domains: BTreeSet<String>,
}

impl Default for MailgunProvider {
    fn default() -> Self {
        Self {
            http: None,
            send_url: Url::parse(DEFAULT_BASE_URL).expect("default mailgun base url must be valid"),
            api_key: SecretString::new(String::new().into_boxed_str()),
            allowed_from_domains: BTreeSet::new(),
        }
    }
}

impl MailgunProvider {
    /// Builds a configured Mailgun provider.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client or endpoint URL cannot be built.
    pub fn from_config(config: &MailgunConfig) -> Result<Self> {
        let path = format!("v3/{}/messages", config.domain);

        Ok(Self {
            http: Some(configured_http_client(config.request_timeout)?),
            send_url: join_url(&config.base_url, &path, "mailgun")?,
            api_key: secret_copy(&config.api_key),
            allowed_from_domains: cloned_domains(&config.allowed_from_domains),
        })
    }
}

#[async_trait]
impl MailProvider for MailgunProvider {
    async fn send(&self, message: &EmailMessage) -> Result<SendReceipt> {
        let http = self
            .http
            .as_ref()
            .ok_or_else(|| MailError::Config("mailgun provider is not configured".to_owned()))?;
        message.validate()?;
        message.validate_sender_domain(&self.allowed_from_domains)?;

        let response = http
            .post(self.send_url.clone())
            .basic_auth("api", Some(self.api_key.expose_secret()))
            .multipart(mailgun_form(message)?)
            .send()
            .await
            .map_err(|error| MailError::Temporary(format!("mailgun request failed: {error}")))?;
        let status = response.status();

        if status.is_success() {
            let body = response
                .json::<MailgunSendResponse>()
                .await
                .map_err(|error| {
                    MailError::Temporary(format!("mailgun success response was invalid: {error}"))
                })?;
            let message_id = body
                .id
                .as_deref()
                .or_else(|| message.idempotency_key())
                .map_or_else(|| uuid::Uuid::new_v4().to_string(), str::to_owned);
            return Ok(SendReceipt::new(
                self.provider_name(),
                MessageId::new(message_id),
                body.id,
            ));
        }

        let text = response_text(response, self.provider_name()).await;
        Err(provider_error(self.provider_name(), status.as_u16(), text))
    }

    async fn get_status(&self, _id: &MessageId) -> Result<Option<SendStatus>> {
        Ok(None)
    }

    fn provider_name(&self) -> &'static str {
        "mailgun"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::new()
            .with_attachments()
            .with_custom_headers()
            .with_regions()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct MailgunSendResponse {
    id: Option<String>,
}

fn validate_domain(value: String) -> Result<String> {
    let domain = value.trim();
    if domain.is_empty() {
        return Err(MailError::Config("mailgun domain is required".to_owned()));
    }

    if domain
        .bytes()
        .any(|byte| byte.is_ascii_whitespace() || matches!(byte, b'/' | b'?' | b'#'))
    {
        return Err(MailError::Config(
            "mailgun domain must not contain whitespace, /, ?, or #".to_owned(),
        ));
    }

    Ok(domain.to_owned())
}

fn add_mailboxes(
    form: reqwest::multipart::Form,
    field: &'static str,
    addresses: &[EmailAddress],
) -> reqwest::multipart::Form {
    addresses
        .iter()
        .fold(form, |form, address| form.text(field, address.formatted()))
}

fn mailgun_form(message: &EmailMessage) -> Result<reqwest::multipart::Form> {
    let mut form = reqwest::multipart::Form::new()
        .text("from", message.from_address().formatted())
        .text("subject", message.subject().to_owned());

    form = add_mailboxes(form, "to", message.to());
    form = add_mailboxes(form, "cc", message.cc());
    form = add_mailboxes(form, "bcc", message.bcc());

    if let Some(text) = message.body_text() {
        form = form.text("text", text.to_owned());
    }
    if let Some(html) = message.body_html() {
        form = form.text("html", html.to_owned());
    }

    form = message.headers().iter().fold(form, |form, (name, value)| {
        form.text(format!("h:{name}"), value.to_owned())
    });

    message
        .attachments()
        .iter()
        .try_fold(form, |form, attachment| {
            let part = reqwest::multipart::Part::bytes(attachment.content().to_vec())
                .file_name(attachment.file_name().to_owned())
                .mime_str(attachment.content_type())
                .map_err(|error| {
                    MailError::Validation(format!(
                        "mailgun attachment content type was rejected: {error}"
                    ))
                })?;
            Ok(form.part("attachment", part))
        })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::config::MailbridgeConfig;
    use crate::provider::shared::test_support::test_server;

    #[test]
    fn validate_domain_rejects_path_segments() {
        let error = validate_domain("example.com/messages".to_owned())
            .expect_err("slash should be rejected");

        assert!(matches!(error, MailError::Config(_)));
    }

    #[tokio::test]
    async fn send_posts_multipart_to_domain_endpoint() {
        let (base_url, request_rx) = test_server(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 26\r\n\r\n{\"id\":\"<abc@example.com>\"}",
        );
        let config = shared_config();
        let mailgun_config = MailgunConfig::new("secret", "mg.example.com", &config)
            .expect("valid mailgun config")
            .with_base_url(base_url)
            .expect("valid base url");
        let provider = MailgunProvider::from_config(&mailgun_config).expect("provider builds");
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

        assert_eq!(receipt.message_id().as_str(), "<abc@example.com>");
        assert!(request.contains("post /v3/mg.example.com/messages http/1.1"));
        assert!(request.contains("authorization: basic yxbponnl"));
        assert!(request.contains("content-disposition: form-data; name=\"from\""));
        assert!(request.contains("app <sender@example.com>"));
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
