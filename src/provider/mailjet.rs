use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
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

const DEFAULT_BASE_URL: &str = "https://api.mailjet.com";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MailjetApiVersion {
    V3,
    #[default]
    V31,
}

impl MailjetApiVersion {
    fn endpoint_path(self) -> &'static str {
        match self {
            Self::V3 => "v3/send",
            Self::V31 => "v3.1/send",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MailjetConfig {
    api_key: SecretString,
    secret_key: SecretString,
    base_url: Url,
    api_version: MailjetApiVersion,
    sandbox_mode: bool,
    allowed_from_domains: BTreeSet<String>,
    request_timeout: Duration,
}

impl MailjetConfig {
    /// Builds Mailjet configuration from API credentials and shared Mailbridge
    /// configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when credentials or the default base URL are invalid.
    pub fn new(
        api_key: impl Into<String>,
        secret_key: impl Into<String>,
        config: &MailbridgeConfig,
    ) -> Result<Self> {
        let api_key = api_key.into();
        let secret_key = secret_key.into();
        validate_secret(&api_key, "mailjet api key")?;
        validate_secret(&secret_key, "mailjet secret key")?;

        Ok(Self {
            api_key: SecretString::new(api_key.into_boxed_str()),
            secret_key: SecretString::new(secret_key.into_boxed_str()),
            base_url: parse_base_url(DEFAULT_BASE_URL, "mailjet")?,
            api_version: MailjetApiVersion::default(),
            sandbox_mode: false,
            allowed_from_domains: cloned_domains(config.allowed_from_domains()),
            request_timeout: config.request_timeout(),
        })
    }

    /// Builds Mailjet configuration from `MAILBRIDGE_MAILJET_*` environment
    /// variables and shared Mailbridge configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when required variables are missing or malformed.
    pub fn from_env(config: &MailbridgeConfig) -> Result<Self> {
        let mut value = Self::new(
            secret_from_env("MAILBRIDGE_MAILJET_API_KEY")?.expose_secret(),
            secret_from_env("MAILBRIDGE_MAILJET_SECRET_KEY")?.expose_secret(),
            config,
        )?;
        if let Some(base_url) = optional_env("MAILBRIDGE_MAILJET_BASE_URL") {
            value = value.with_base_url(&base_url)?;
        }
        if let Some(version) = optional_env("MAILBRIDGE_MAILJET_API_VERSION") {
            value = value.with_api_version(parse_api_version(&version)?);
        }
        if let Some(sandbox_mode) = optional_env("MAILBRIDGE_MAILJET_SANDBOX_MODE") {
            value = value.with_sandbox_mode(parse_bool(&sandbox_mode, "mailjet sandbox mode")?);
        }
        Ok(value)
    }

    /// Sets the Mailjet API base URL.
    ///
    /// # Errors
    ///
    /// Returns an error when `base_url` is not a valid URL.
    pub fn with_base_url(mut self, base_url: impl AsRef<str>) -> Result<Self> {
        self.base_url = parse_base_url(base_url.as_ref(), "mailjet")?;
        Ok(self)
    }

    #[must_use]
    pub const fn with_api_version(mut self, api_version: MailjetApiVersion) -> Self {
        self.api_version = api_version;
        self
    }

    #[must_use]
    pub const fn with_sandbox_mode(mut self, sandbox_mode: bool) -> Self {
        self.sandbox_mode = sandbox_mode;
        self
    }

    #[must_use]
    pub const fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }
}

#[derive(Debug, Clone)]
pub struct MailjetProvider {
    http: Option<reqwest::Client>,
    send_url: Url,
    api_key: SecretString,
    secret_key: SecretString,
    api_version: MailjetApiVersion,
    sandbox_mode: bool,
    allowed_from_domains: BTreeSet<String>,
}

impl Default for MailjetProvider {
    fn default() -> Self {
        Self {
            http: None,
            send_url: Url::parse(DEFAULT_BASE_URL).expect("default mailjet base url must be valid"),
            api_key: SecretString::new(String::new().into_boxed_str()),
            secret_key: SecretString::new(String::new().into_boxed_str()),
            api_version: MailjetApiVersion::default(),
            sandbox_mode: false,
            allowed_from_domains: BTreeSet::new(),
        }
    }
}

impl MailjetProvider {
    /// Builds a configured Mailjet provider.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client or endpoint URL cannot be built.
    pub fn from_config(config: &MailjetConfig) -> Result<Self> {
        Ok(Self {
            http: Some(configured_http_client(config.request_timeout)?),
            send_url: join_url(
                &config.base_url,
                config.api_version.endpoint_path(),
                "mailjet",
            )?,
            api_key: secret_copy(&config.api_key),
            secret_key: secret_copy(&config.secret_key),
            api_version: config.api_version,
            sandbox_mode: config.sandbox_mode,
            allowed_from_domains: cloned_domains(&config.allowed_from_domains),
        })
    }
}

#[async_trait]
impl MailProvider for MailjetProvider {
    async fn send(&self, message: &EmailMessage) -> Result<SendReceipt> {
        let http = self
            .http
            .as_ref()
            .ok_or_else(|| MailError::Config("mailjet provider is not configured".to_owned()))?;
        message.validate()?;
        message.validate_sender_domain(&self.allowed_from_domains)?;

        let request = MailjetRequest::from_message(message, self.api_version, self.sandbox_mode);
        let response = http
            .post(self.send_url.clone())
            .basic_auth(
                self.api_key.expose_secret(),
                Some(self.secret_key.expose_secret()),
            )
            .json(&request)
            .send()
            .await
            .map_err(|error| MailError::Temporary(format!("mailjet request failed: {error}")))?;
        let status = response.status();

        if status.is_success() {
            let body = response
                .json::<serde_json::Value>()
                .await
                .map_err(|error| {
                    MailError::Temporary(format!("mailjet success response was invalid: {error}"))
                })?;
            let provider_id = first_field_string(&body, &["MessageUUID", "MessageID"]);
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
        "mailjet"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::new()
            .with_sandbox_mode()
            .with_attachments()
            .with_custom_headers()
            .with_templates()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
enum MailjetRequest<'a> {
    V3(Box<MailjetV3Request<'a>>),
    V31(Box<MailjetV31Request<'a>>),
}

impl<'a> MailjetRequest<'a> {
    fn from_message(
        message: &'a EmailMessage,
        api_version: MailjetApiVersion,
        sandbox_mode: bool,
    ) -> Self {
        match api_version {
            MailjetApiVersion::V3 => Self::V3(Box::new(MailjetV3Request::from_message(message))),
            MailjetApiVersion::V31 => Self::V31(Box::new(MailjetV31Request::from_message(
                message,
                sandbox_mode,
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct MailjetV31Request<'a> {
    #[serde(rename = "Messages")]
    messages: Vec<MailjetV31Message<'a>>,
    #[serde(rename = "SandboxMode", skip_serializing_if = "is_false")]
    sandbox_mode: bool,
}

impl<'a> MailjetV31Request<'a> {
    fn from_message(message: &'a EmailMessage, sandbox_mode: bool) -> Self {
        Self {
            messages: vec![MailjetV31Message::from_message(message)],
            sandbox_mode,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct MailjetV31Message<'a> {
    #[serde(rename = "From")]
    from: MailjetAddress<'a>,
    #[serde(rename = "To")]
    to: Vec<MailjetAddress<'a>>,
    #[serde(rename = "Cc", skip_serializing_if = "Vec::is_empty")]
    cc: Vec<MailjetAddress<'a>>,
    #[serde(rename = "Bcc", skip_serializing_if = "Vec::is_empty")]
    bcc: Vec<MailjetAddress<'a>>,
    #[serde(rename = "Subject")]
    subject: Cow<'a, str>,
    #[serde(rename = "TextPart", skip_serializing_if = "Option::is_none")]
    text_part: Option<Cow<'a, str>>,
    #[serde(rename = "HTMLPart", skip_serializing_if = "Option::is_none")]
    html_part: Option<Cow<'a, str>>,
    #[serde(rename = "Headers", skip_serializing_if = "BTreeMap::is_empty")]
    headers: BTreeMap<Cow<'a, str>, Cow<'a, str>>,
    #[serde(rename = "Attachments", skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<MailjetV31Attachment<'a>>,
}

impl<'a> MailjetV31Message<'a> {
    fn from_message(message: &'a EmailMessage) -> Self {
        Self {
            from: MailjetAddress::from(message.from_address()),
            to: addresses(message.to()),
            cc: addresses(message.cc()),
            bcc: addresses(message.bcc()),
            subject: Cow::Borrowed(message.subject()),
            text_part: message.body_text().map(Cow::Borrowed),
            html_part: message.body_html().map(Cow::Borrowed),
            headers: message
                .headers()
                .iter()
                .map(|(name, value)| (Cow::Borrowed(name.as_str()), Cow::Borrowed(value.as_str())))
                .collect(),
            attachments: message
                .attachments()
                .iter()
                .map(MailjetV31Attachment::from)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct MailjetV3Request<'a> {
    #[serde(rename = "FromEmail")]
    from_email: Cow<'a, str>,
    #[serde(rename = "FromName", skip_serializing_if = "Option::is_none")]
    from_name: Option<Cow<'a, str>>,
    #[serde(rename = "To")]
    to: String,
    #[serde(rename = "Cc", skip_serializing_if = "String::is_empty")]
    cc: String,
    #[serde(rename = "Bcc", skip_serializing_if = "String::is_empty")]
    bcc: String,
    #[serde(rename = "Subject")]
    subject: Cow<'a, str>,
    #[serde(rename = "Text-part", skip_serializing_if = "Option::is_none")]
    text_part: Option<Cow<'a, str>>,
    #[serde(rename = "Html-part", skip_serializing_if = "Option::is_none")]
    html_part: Option<Cow<'a, str>>,
    #[serde(rename = "Headers", skip_serializing_if = "BTreeMap::is_empty")]
    headers: BTreeMap<Cow<'a, str>, Cow<'a, str>>,
    #[serde(rename = "Attachments", skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<MailjetV3Attachment<'a>>,
}

impl<'a> MailjetV3Request<'a> {
    fn from_message(message: &'a EmailMessage) -> Self {
        Self {
            from_email: Cow::Borrowed(message.from_address().email()),
            from_name: message.from_address().name().map(Cow::Borrowed),
            to: formatted_join(message.to()),
            cc: formatted_join(message.cc()),
            bcc: formatted_join(message.bcc()),
            subject: Cow::Borrowed(message.subject()),
            text_part: message.body_text().map(Cow::Borrowed),
            html_part: message.body_html().map(Cow::Borrowed),
            headers: message
                .headers()
                .iter()
                .map(|(name, value)| (Cow::Borrowed(name.as_str()), Cow::Borrowed(value.as_str())))
                .collect(),
            attachments: message
                .attachments()
                .iter()
                .map(MailjetV3Attachment::from)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct MailjetAddress<'a> {
    #[serde(rename = "Email")]
    email: Cow<'a, str>,
    #[serde(rename = "Name", skip_serializing_if = "Option::is_none")]
    name: Option<Cow<'a, str>>,
}

impl<'a> From<&'a EmailAddress> for MailjetAddress<'a> {
    fn from(address: &'a EmailAddress) -> Self {
        Self {
            email: Cow::Borrowed(address.email()),
            name: address.name().map(Cow::Borrowed),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct MailjetV31Attachment<'a> {
    #[serde(rename = "ContentType")]
    content_type: Cow<'a, str>,
    #[serde(rename = "Filename")]
    filename: Cow<'a, str>,
    #[serde(rename = "Base64Content")]
    base64_content: String,
}

impl<'a> From<&'a Attachment> for MailjetV31Attachment<'a> {
    fn from(attachment: &'a Attachment) -> Self {
        Self {
            content_type: Cow::Borrowed(attachment.content_type()),
            filename: Cow::Borrowed(attachment.file_name()),
            base64_content: bytes_base64(attachment.content()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct MailjetV3Attachment<'a> {
    #[serde(rename = "Content-type")]
    content_type: Cow<'a, str>,
    #[serde(rename = "Filename")]
    filename: Cow<'a, str>,
    content: String,
}

impl<'a> From<&'a Attachment> for MailjetV3Attachment<'a> {
    fn from(attachment: &'a Attachment) -> Self {
        Self {
            content_type: Cow::Borrowed(attachment.content_type()),
            filename: Cow::Borrowed(attachment.file_name()),
            content: bytes_base64(attachment.content()),
        }
    }
}

fn addresses(addresses: &[EmailAddress]) -> Vec<MailjetAddress<'_>> {
    addresses.iter().map(MailjetAddress::from).collect()
}

fn formatted_join(addresses: &[EmailAddress]) -> String {
    addresses
        .iter()
        .map(EmailAddress::formatted)
        .collect::<Vec<_>>()
        .join(",")
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

fn parse_api_version(value: &str) -> Result<MailjetApiVersion> {
    match value.trim().to_ascii_lowercase().as_str() {
        "v3" | "3" => Ok(MailjetApiVersion::V3),
        "v3.1" | "3.1" | "v31" | "31" => Ok(MailjetApiVersion::V31),
        _ => Err(MailError::Config(format!(
            "unsupported mailjet api version: {value}"
        ))),
    }
}

fn parse_bool(value: &str, label: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(MailError::Config(format!("{label} must be true or false"))),
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::provider::shared::test_support::test_server;

    #[test]
    fn v31_request_serialization_matches_mailjet_shape() {
        let message = EmailMessage::builder()
            .from("App", "sender@example.com")
            .expect("valid from")
            .to("User", "user@example.net")
            .expect("valid to")
            .subject("Hello")
            .text("Plain")
            .build()
            .expect("valid message");

        let json = serde_json::to_value(MailjetRequest::from_message(
            &message,
            MailjetApiVersion::V31,
            true,
        ))
        .expect("json");

        assert_eq!(json["SandboxMode"], true);
        assert_eq!(json["Messages"][0]["From"]["Email"], "sender@example.com");
        assert_eq!(json["Messages"][0]["To"][0]["Name"], "User");
    }

    #[test]
    fn v3_request_serialization_matches_mailjet_shape() {
        let message = EmailMessage::builder()
            .from("App", "sender@example.com")
            .expect("valid from")
            .to("User", "user@example.net")
            .expect("valid to")
            .subject("Hello")
            .text("Plain")
            .build()
            .expect("valid message");

        let json = serde_json::to_value(MailjetRequest::from_message(
            &message,
            MailjetApiVersion::V3,
            false,
        ))
        .expect("json");

        assert_eq!(json["FromEmail"], "sender@example.com");
        assert_eq!(json["To"], "User <user@example.net>");
        assert_eq!(json["Text-part"], "Plain");
    }

    #[tokio::test]
    async fn send_posts_to_v31_endpoint() {
        let (base_url, request_rx) = test_server(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 69\r\n\r\n{\"Messages\":[{\"To\":[{\"MessageUUID\":\"mailjet-123\",\"MessageID\":123}]}]}",
        );
        let config = shared_config();
        let provider_config = MailjetConfig::new("public", "private", &config)
            .expect("valid config")
            .with_base_url(base_url)
            .expect("valid base url")
            .with_sandbox_mode(true);
        let provider = MailjetProvider::from_config(&provider_config).expect("provider builds");
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

        assert_eq!(receipt.message_id().as_str(), "mailjet-123");
        assert!(request.contains("post /v3.1/send http/1.1"));
        assert!(request.contains("authorization: basic"));
        assert!(request.contains("\"sandboxmode\":true"));
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
