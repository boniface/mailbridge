use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::client::{MessageId, SendReceipt};
use crate::config::MailbridgeConfig;
use crate::email::{Attachment, EmailMessage};
use crate::error::{MailError, Result};
use crate::provider::shared::{
    ApiAddress, addresses, attachment_content_base64, cloned_domains, configured_http_client,
    join_url, optional_env, parse_base_url, provider_error, required_env, response_text,
    secret_copy, validate_secret,
};
use crate::provider::{MailProvider, SendStatus};

const DEFAULT_BASE_URL: &str = "https://api.sendpulse.com";

#[derive(Debug, Clone)]
pub struct SendPulseConfig {
    auth: SendPulseAuthConfig,
    base_url: Url,
    allowed_from_domains: BTreeSet<String>,
    request_timeout: Duration,
}

#[derive(Debug, Clone)]
enum SendPulseAuthConfig {
    AccessToken(SecretString),
    ClientCredentials {
        client_id: SecretString,
        client_secret: SecretString,
    },
}

impl SendPulseConfig {
    /// Builds SendPulse SMTP API configuration from a bearer token and shared
    /// Mailbridge configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when the token or default SendPulse base URL is invalid.
    pub fn new(api_key: impl Into<String>, config: &MailbridgeConfig) -> Result<Self> {
        let access_token = api_key.into();
        validate_secret(&access_token, "sendpulse")?;

        Ok(Self {
            auth: SendPulseAuthConfig::AccessToken(SecretString::new(
                access_token.into_boxed_str(),
            )),
            base_url: parse_base_url(DEFAULT_BASE_URL, "sendpulse")?,
            allowed_from_domains: cloned_domains(config.allowed_from_domains()),
            request_timeout: config.request_timeout(),
        })
    }

    /// Builds SendPulse SMTP API configuration from client credentials.
    ///
    /// # Errors
    ///
    /// Returns an error when credentials or the default SendPulse base URL are
    /// invalid.
    pub fn from_client_credentials(
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        config: &MailbridgeConfig,
    ) -> Result<Self> {
        let client_id = required_secret(client_id.into(), "sendpulse client id")?;
        let client_secret = required_secret(client_secret.into(), "sendpulse client secret")?;

        Ok(Self {
            auth: SendPulseAuthConfig::ClientCredentials {
                client_id,
                client_secret,
            },
            base_url: parse_base_url(DEFAULT_BASE_URL, "sendpulse")?,
            allowed_from_domains: cloned_domains(config.allowed_from_domains()),
            request_timeout: config.request_timeout(),
        })
    }

    /// Builds SendPulse configuration from `MAILBRIDGE_SENDPULSE_*` environment
    /// variables and shared Mailbridge configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when required variables are missing or malformed.
    pub fn from_env(config: &MailbridgeConfig) -> Result<Self> {
        let access_token = optional_env("MAILBRIDGE_SENDPULSE_ACCESS_TOKEN")
            .or_else(|| optional_env("MAILBRIDGE_SENDPULSE_API_KEY"));
        let mut value = if let Some(access_token) = access_token {
            Self::new(access_token, config)?
        } else {
            Self::from_client_credentials(
                required_env("MAILBRIDGE_SENDPULSE_CLIENT_ID")?,
                required_env("MAILBRIDGE_SENDPULSE_CLIENT_SECRET")?,
                config,
            )?
        };
        if let Some(base_url) = optional_env("MAILBRIDGE_SENDPULSE_BASE_URL") {
            value = value.with_base_url(&base_url)?;
        }
        Ok(value)
    }

    /// Sets the SendPulse API base URL.
    ///
    /// # Errors
    ///
    /// Returns an error when `base_url` is not a valid URL.
    pub fn with_base_url(mut self, base_url: impl AsRef<str>) -> Result<Self> {
        self.base_url = parse_base_url(base_url.as_ref(), "sendpulse")?;
        Ok(self)
    }

    #[must_use]
    pub const fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }
}

#[derive(Debug, Clone)]
pub struct SendPulseProvider {
    http: Option<reqwest::Client>,
    send_url: Url,
    auth: SendPulseAuth,
    allowed_from_domains: BTreeSet<String>,
}

#[derive(Debug, Clone)]
enum SendPulseAuth {
    AccessToken(SecretString),
    ClientCredentials {
        client_id: SecretString,
        client_secret: SecretString,
        token_url: Url,
        token_cache: Arc<tokio::sync::Mutex<Option<CachedToken>>>,
    },
}

#[derive(Debug)]
struct CachedToken {
    access_token: SecretString,
    expires_at: Instant,
}

impl Default for SendPulseProvider {
    fn default() -> Self {
        Self {
            http: None,
            send_url: Url::parse(DEFAULT_BASE_URL)
                .expect("default sendpulse base url must be valid"),
            auth: SendPulseAuth::AccessToken(SecretString::new(String::new().into_boxed_str())),
            allowed_from_domains: BTreeSet::new(),
        }
    }
}

impl SendPulseProvider {
    /// Builds a configured SendPulse provider.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client or endpoint URL cannot be built.
    pub fn from_config(config: &SendPulseConfig) -> Result<Self> {
        let auth = match &config.auth {
            SendPulseAuthConfig::AccessToken(access_token) => {
                SendPulseAuth::AccessToken(secret_copy(access_token))
            }
            SendPulseAuthConfig::ClientCredentials {
                client_id,
                client_secret,
            } => SendPulseAuth::ClientCredentials {
                client_id: secret_copy(client_id),
                client_secret: secret_copy(client_secret),
                token_url: join_url(&config.base_url, "oauth/access_token", "sendpulse")?,
                token_cache: Arc::new(tokio::sync::Mutex::new(None)),
            },
        };

        Ok(Self {
            http: Some(configured_http_client(config.request_timeout)?),
            send_url: join_url(&config.base_url, "smtp/emails", "sendpulse")?,
            auth,
            allowed_from_domains: cloned_domains(&config.allowed_from_domains),
        })
    }
}

#[async_trait]
impl MailProvider for SendPulseProvider {
    async fn send(&self, message: &EmailMessage) -> Result<SendReceipt> {
        let http = self
            .http
            .as_ref()
            .ok_or_else(|| MailError::Config("sendpulse provider is not configured".to_owned()))?;
        message.validate()?;
        message.validate_sender_domain(&self.allowed_from_domains)?;
        let access_token = self.access_token(http).await?;

        let response = http
            .post(self.send_url.clone())
            .bearer_auth(access_token.expose_secret())
            .json(&SendPulseRequest::from_message(message))
            .send()
            .await
            .map_err(|error| MailError::Temporary(format!("sendpulse request failed: {error}")))?;
        let status = response.status();

        if status.is_success() {
            let body = response
                .json::<SendPulseSendResponse>()
                .await
                .map_err(|error| {
                    MailError::Temporary(format!("sendpulse success response was invalid: {error}"))
                })?;

            if body.result == Some(false) {
                return Err(MailError::RelayRejected {
                    status: status.as_u16(),
                    message: body
                        .message
                        .unwrap_or_else(|| "sendpulse rejected request".to_owned()),
                });
            }

            let provider_id = body.provider_id();
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
        "sendpulse"
    }
}

impl SendPulseProvider {
    async fn access_token(&self, http: &reqwest::Client) -> Result<SecretString> {
        match &self.auth {
            SendPulseAuth::AccessToken(access_token) => Ok(secret_copy(access_token)),
            SendPulseAuth::ClientCredentials {
                client_id,
                client_secret,
                token_url,
                token_cache,
            } => {
                if let Some(access_token) = cached_access_token(token_cache).await {
                    return Ok(access_token);
                }

                let token = fetch_access_token(http, token_url, client_id, client_secret).await?;
                let mut guard = token_cache.lock().await;
                *guard = Some(CachedToken {
                    access_token: secret_copy(&token.access_token),
                    expires_at: token.expires_at,
                });
                Ok(token.access_token)
            }
        }
    }
}

#[derive(Debug)]
struct FetchedToken {
    access_token: SecretString,
    expires_at: Instant,
}

#[derive(Debug, Serialize)]
struct SendPulseTokenRequest<'a> {
    grant_type: &'static str,
    client_id: &'a str,
    client_secret: &'a str,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct SendPulseTokenResponse {
    access_token: String,
    expires_in: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct SendPulseRequest<'a> {
    email: SendPulseEmail<'a>,
}

impl<'a> SendPulseRequest<'a> {
    fn from_message(message: &'a EmailMessage) -> Self {
        Self {
            email: SendPulseEmail::from_message(message),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct SendPulseEmail<'a> {
    from: ApiAddress<'a>,
    to: Vec<ApiAddress<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    cc: Vec<ApiAddress<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    bcc: Vec<ApiAddress<'a>>,
    subject: Cow<'a, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<Cow<'a, str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    html: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    headers: BTreeMap<Cow<'a, str>, Cow<'a, str>>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    attachments_binary: BTreeMap<Cow<'a, str>, String>,
}

impl<'a> SendPulseEmail<'a> {
    fn from_message(message: &'a EmailMessage) -> Self {
        Self {
            from: ApiAddress::from(message.from_address()),
            to: addresses(message.to()),
            cc: addresses(message.cc()),
            bcc: addresses(message.bcc()),
            subject: Cow::Borrowed(message.subject()),
            text: message.body_text().map(Cow::Borrowed),
            html: message.body_html().map(html_base64),
            headers: message
                .headers()
                .iter()
                .map(|(name, value)| (Cow::Borrowed(name.as_str()), Cow::Borrowed(value.as_str())))
                .collect(),
            attachments_binary: attachments(message.attachments()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct SendPulseSendResponse {
    result: Option<bool>,
    id: Option<serde_json::Value>,
    message_id: Option<serde_json::Value>,
    message: Option<String>,
}

impl SendPulseSendResponse {
    fn provider_id(&self) -> Option<String> {
        self.id
            .as_ref()
            .and_then(json_field_string)
            .or_else(|| self.message_id.as_ref().and_then(json_field_string))
    }
}

fn attachments(attachments: &[Attachment]) -> BTreeMap<Cow<'_, str>, String> {
    attachments
        .iter()
        .map(|attachment| {
            (
                Cow::Borrowed(attachment.file_name()),
                attachment_content_base64(attachment),
            )
        })
        .collect()
}

async fn cached_access_token(
    token_cache: &tokio::sync::Mutex<Option<CachedToken>>,
) -> Option<SecretString> {
    let guard = token_cache.lock().await;
    guard
        .as_ref()
        .filter(|cached| cached.expires_at > Instant::now() + Duration::from_secs(30))
        .map(|cached| secret_copy(&cached.access_token))
}

async fn fetch_access_token(
    http: &reqwest::Client,
    token_url: &Url,
    client_id: &SecretString,
    client_secret: &SecretString,
) -> Result<FetchedToken> {
    let request = SendPulseTokenRequest {
        grant_type: "client_credentials",
        client_id: client_id.expose_secret(),
        client_secret: client_secret.expose_secret(),
    };
    let response = http
        .post(token_url.clone())
        .json(&request)
        .send()
        .await
        .map_err(|error| {
            MailError::Temporary(format!("sendpulse token request failed: {error}"))
        })?;
    let status = response.status();

    if status.is_success() {
        let body = response
            .json::<SendPulseTokenResponse>()
            .await
            .map_err(|error| {
                MailError::Temporary(format!("sendpulse token response was invalid: {error}"))
            })?;
        if body.access_token.trim().is_empty() {
            return Err(MailError::Authentication);
        }

        let ttl = body.expires_in.unwrap_or(3600).saturating_sub(60).max(1);
        return Ok(FetchedToken {
            access_token: SecretString::new(body.access_token.into_boxed_str()),
            expires_at: Instant::now() + Duration::from_secs(ttl),
        });
    }

    let text = response_text(response, "sendpulse").await;
    Err(provider_error("sendpulse", status.as_u16(), text))
}

fn required_secret(value: String, label: &str) -> Result<SecretString> {
    if value.trim().is_empty() {
        return Err(MailError::Config(format!("{label} is required")));
    }

    Ok(SecretString::new(value.into_boxed_str()))
}

fn html_base64(value: &str) -> String {
    use base64::Engine;

    base64::engine::general_purpose::STANDARD.encode(value.as_bytes())
}

fn json_field_string(value: &serde_json::Value) -> Option<String> {
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
    use crate::config::MailbridgeConfig;
    use crate::provider::shared::test_support::{test_server, test_server_with_responses};

    #[test]
    fn request_serialization_encodes_html_and_attachments() {
        let message = EmailMessage::builder()
            .from("App", "sender@example.com")
            .expect("valid from")
            .to("User", "user@example.net")
            .expect("valid to")
            .subject("Hello")
            .text("Plain")
            .html("<p>Plain</p>")
            .attachment(
                Attachment::new("test.txt", "text/plain", b"file".to_vec())
                    .expect("valid attachment"),
            )
            .expect("attachment accepted")
            .build()
            .expect("valid message");

        let json =
            serde_json::to_value(SendPulseRequest::from_message(&message)).expect("json request");

        assert_eq!(json["email"]["from"]["email"], "sender@example.com");
        assert_eq!(json["email"]["to"][0]["email"], "user@example.net");
        assert_eq!(json["email"]["html"], "PHA+UGxhaW48L3A+");
        assert_eq!(json["email"]["attachments_binary"]["test.txt"], "ZmlsZQ==");
    }

    #[tokio::test]
    async fn send_posts_json_to_smtp_emails_endpoint() {
        let (base_url, request_rx) = test_server(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 32\r\n\r\n{\"result\":true,\"id\":\"pulse-123\"}",
        );
        let config = shared_config();
        let sendpulse_config = SendPulseConfig::new("secret", &config)
            .expect("valid sendpulse config")
            .with_base_url(base_url)
            .expect("valid base url");
        let provider = SendPulseProvider::from_config(&sendpulse_config).expect("provider builds");
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

        assert_eq!(receipt.message_id().as_str(), "pulse-123");
        assert!(request.contains("post /smtp/emails http/1.1"));
        assert!(request.contains("authorization: bearer secret"));
        assert!(request.contains("\"subject\":\"hello\""));
    }

    #[tokio::test]
    async fn send_fetches_access_token_from_client_credentials() {
        let (base_url, request_rx) = test_server_with_responses(vec![
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"access_token\":\"oauth-token\",\"expires_in\":3600}",
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"result\":true,\"id\":\"pulse-456\"}",
        ]);
        let config = shared_config();
        let sendpulse_config =
            SendPulseConfig::from_client_credentials("client-id", "client-secret", &config)
                .expect("valid sendpulse config")
                .with_base_url(base_url)
                .expect("valid base url");
        let provider = SendPulseProvider::from_config(&sendpulse_config).expect("provider builds");
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
        let token_request = request_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("token request captured");
        let send_request = request_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("send request captured");

        assert_eq!(receipt.message_id().as_str(), "pulse-456");
        assert!(token_request.contains("post /oauth/access_token http/1.1"));
        assert!(token_request.contains("\"client_id\":\"client-id\""));
        assert!(send_request.contains("post /smtp/emails http/1.1"));
        assert!(send_request.contains("authorization: bearer oauth-token"));
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
