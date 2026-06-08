use std::borrow::Cow;
use std::collections::BTreeMap;

use async_trait::async_trait;
use base64::Engine;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::client::{MessageId, SendReceipt};
use crate::config::MailbridgeConfig;
use crate::email::{Attachment, EmailAddress, EmailMessage};
use crate::error::{MailError, Result};
use crate::provider::{MailProvider, SendStatus};

type Recipients<'a> = Vec<ApiAddress<'a>>;

#[derive(Debug)]
pub struct HyvorRelayProvider {
    http: reqwest::Client,
    sends_url: Url,
    api_key: SecretString,
    allowed_from_domains: std::collections::BTreeSet<String>,
}

impl HyvorRelayProvider {
    /// Builds a Relay provider from library configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client or Relay sends URL cannot be
    /// constructed.
    pub fn from_config(config: &MailbridgeConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(config.request_timeout())
            .build()
            .map_err(|error| MailError::Config(format!("failed to build http client: {error}")))?;

        Ok(Self {
            http,
            sends_url: sends_url(config.api_base_url())?,
            api_key: secret_copy(config.api_key()),
            allowed_from_domains: config.allowed_from_domains().clone(),
        })
    }

    #[must_use]
    pub fn sends_url(&self) -> &Url {
        &self.sends_url
    }
}

#[async_trait]
impl MailProvider for HyvorRelayProvider {
    async fn send(&self, message: &EmailMessage) -> Result<SendReceipt> {
        message.validate()?;
        message.validate_sender_domain(&self.allowed_from_domains)?;

        let request = ApiSendRequest::from_message(message);
        let mut builder = self
            .http
            .post(self.sends_url.clone())
            .bearer_auth(self.api_key.expose_secret())
            .json(&request);

        if let Some(key) = message.idempotency_key() {
            builder = builder.header("Idempotency-Key", key);
        }

        let response = builder
            .send()
            .await
            .map_err(|error| MailError::Temporary(format!("relay request failed: {error}")))?;
        let status = response.status();

        if status.is_success() {
            let body = response.json::<ApiSendResponse>().await.map_err(|error| {
                MailError::Temporary(format!("relay success response was invalid: {error}"))
            })?;
            return Ok(SendReceipt::new(
                self.provider_name(),
                MessageId::new(body.message_id),
                Some(body.id.to_string()),
            ));
        }

        let message = response
            .text()
            .await
            .unwrap_or_else(|error| format!("failed to read relay error response: {error}"));
        map_status_error(status.as_u16(), message)
    }

    async fn get_status(&self, id: &MessageId) -> Result<Option<SendStatus>> {
        let response = self
            .http
            .get(send_uuid_url(&self.sends_url, id)?)
            .bearer_auth(self.api_key.expose_secret())
            .send()
            .await
            .map_err(|error| {
                MailError::Temporary(format!("relay status request failed: {error}"))
            })?;
        let status = response.status();

        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        if status.is_success() {
            let body = response
                .json::<ApiSendStatusResponse>()
                .await
                .map_err(|error| {
                    MailError::Temporary(format!("relay status response was invalid: {error}"))
                })?;
            return Ok(Some(body.status()));
        }

        let message = response
            .text()
            .await
            .unwrap_or_else(|error| format!("failed to read relay error response: {error}"));
        map_status_error(status.as_u16(), message).map(|_| None)
    }

    fn provider_name(&self) -> &'static str {
        "hyvor-relay"
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct ApiSendRequest<'a> {
    from: ApiAddress<'a>,
    to: Recipients<'a>,
    #[serde(skip_serializing_if = "Recipients::is_empty")]
    cc: Recipients<'a>,
    #[serde(skip_serializing_if = "Recipients::is_empty")]
    bcc: Recipients<'a>,
    subject: Cow<'a, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body_text: Option<Cow<'a, str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body_html: Option<Cow<'a, str>>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    headers: BTreeMap<Cow<'a, str>, Cow<'a, str>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<ApiAttachment<'a>>,
}

impl<'a> ApiSendRequest<'a> {
    fn from_message(message: &'a EmailMessage) -> Self {
        Self {
            from: ApiAddress::from(message.from_address()),
            to: recipients(message.to()),
            cc: recipients(message.cc()),
            bcc: recipients(message.bcc()),
            subject: Cow::Borrowed(message.subject()),
            body_text: message.body_text().map(Cow::Borrowed),
            body_html: message.body_html().map(Cow::Borrowed),
            headers: message
                .headers()
                .iter()
                .map(|(name, value)| (Cow::Borrowed(name.as_str()), Cow::Borrowed(value.as_str())))
                .collect(),
            attachments: message
                .attachments()
                .iter()
                .map(ApiAttachment::from)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
enum ApiAddress<'a> {
    Email(Cow<'a, str>),
    Named {
        email: Cow<'a, str>,
        name: Cow<'a, str>,
    },
}

impl<'a> From<&'a EmailAddress> for ApiAddress<'a> {
    fn from(address: &'a EmailAddress) -> Self {
        match address.name() {
            Some(name) => Self::Named {
                email: Cow::Borrowed(address.email()),
                name: Cow::Borrowed(name),
            },
            None => Self::Email(Cow::Borrowed(address.email())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct ApiAttachment<'a> {
    name: Cow<'a, str>,
    content_type: Cow<'a, str>,
    content_base64: String,
}

impl<'a> From<&'a Attachment> for ApiAttachment<'a> {
    fn from(attachment: &'a Attachment) -> Self {
        Self {
            name: Cow::Borrowed(attachment.file_name()),
            content_type: Cow::Borrowed(attachment.content_type()),
            content_base64: base64::engine::general_purpose::STANDARD.encode(attachment.content()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct ApiSendResponse {
    id: serde_json::Value,
    message_id: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct ApiSendStatusResponse {
    queued: bool,
    recipients: Vec<ApiSendRecipient>,
}

impl ApiSendStatusResponse {
    fn status(&self) -> SendStatus {
        if self.queued || self.recipients.iter().any(ApiSendRecipient::is_waiting) {
            return SendStatus::Queued;
        }

        if self.recipients.iter().all(ApiSendRecipient::is_accepted) {
            return SendStatus::Sent;
        }

        if self.recipients.iter().any(ApiSendRecipient::is_failed) {
            return SendStatus::Failed;
        }

        SendStatus::Unknown
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct ApiSendRecipient {
    status: String,
}

impl ApiSendRecipient {
    fn is_waiting(&self) -> bool {
        matches!(self.status.as_str(), "queued" | "deferred")
    }

    fn is_accepted(&self) -> bool {
        self.status == "accepted"
    }

    fn is_failed(&self) -> bool {
        matches!(
            self.status.as_str(),
            "bounced" | "complained" | "suppressed" | "failed"
        )
    }
}

fn recipients(addresses: &[EmailAddress]) -> Recipients<'_> {
    addresses.iter().map(ApiAddress::from).collect()
}

fn sends_url(base_url: &Url) -> Result<Url> {
    let mut base = base_url.clone();
    if !base.path().ends_with('/') {
        let path = format!("{}/", base.path());
        base.set_path(&path);
    }

    base.join("sends")
        .map_err(|error| MailError::Config(format!("invalid relay sends url: {error}")))
}

fn send_uuid_url(sends_url: &Url, id: &MessageId) -> Result<Url> {
    let mut url = sends_url.clone();
    url.path_segments_mut()
        .map_err(|()| MailError::Config("relay sends url cannot be a base URL".to_owned()))?
        .push("uuid")
        .push(id.as_str());

    Ok(url)
}

fn map_status_error(status: u16, message: String) -> Result<SendReceipt> {
    match status {
        401 | 403 => Err(MailError::Authentication),
        500..=599 => Err(MailError::Temporary(format!(
            "relay temporary failure: status={status}, message={message}"
        ))),
        _ => Err(MailError::RelayRejected { status, message }),
    }
}

fn secret_copy(secret: &SecretString) -> SecretString {
    SecretString::new(secret.expose_secret().to_owned().into_boxed_str())
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;
    use crate::config::MailbridgeConfig;

    #[test]
    fn sends_url_appends_sends_to_api_base() {
        let base = Url::parse("https://relay.example.com/api/console").expect("valid url");

        let url = sends_url(&base).expect("valid sends url");

        assert_eq!(url.as_str(), "https://relay.example.com/api/console/sends");
    }

    #[test]
    fn request_serialization_matches_relay_shape() {
        let message = EmailMessage::builder()
            .from("Hashcode", "no-reply@example.com")
            .expect("valid from")
            .to("User", "user@example.net")
            .expect("valid to")
            .subject("Test")
            .text("plain")
            .html("<p>plain</p>")
            .header("X-App", "accounts")
            .expect("valid header")
            .build()
            .expect("valid message");

        let request = ApiSendRequest::from_message(&message);
        let json = serde_json::to_value(&request).expect("serializable request");

        assert_eq!(json["from"]["email"], "no-reply@example.com");
        assert_eq!(json["from"]["name"], "Hashcode");
        assert_eq!(json["to"][0]["email"], "user@example.net");
        assert_eq!(json["to"][0]["name"], "User");
        assert_eq!(json["subject"], "Test");
        assert_eq!(json["body_text"], "plain");
        assert_eq!(json["headers"]["X-App"], "accounts");
    }

    #[tokio::test]
    async fn send_includes_authorization_and_idempotency_headers() {
        let (base_url, request_rx) = test_server(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 33\r\n\r\n{\"id\":123,\"message_id\":\"abc-123\"}",
        );
        let config = MailbridgeConfig::builder()
            .api_base_url(base_url)
            .expect("valid url")
            .api_key("secret")
            .allowed_from_domain("example.com")
            .build()
            .expect("valid config");
        let provider = HyvorRelayProvider::from_config(&config).expect("provider builds");
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

        assert_eq!(receipt.message_id().as_str(), "abc-123");
        assert!(request.contains("post /api/console/sends http/1.1"));
        assert!(request.contains("authorization: bearer secret"));
        assert!(request.contains("idempotency-key: order-123"));
    }

    #[tokio::test]
    async fn send_maps_500_to_retryable_temporary_error() {
        let (base_url, _request_rx) =
            test_server("HTTP/1.1 500 Internal Server Error\r\nContent-Length: 4\r\n\r\nfail");
        let config = MailbridgeConfig::builder()
            .api_base_url(base_url)
            .expect("valid url")
            .api_key("secret")
            .allowed_from_domain("example.com")
            .build()
            .expect("valid config");
        let provider = HyvorRelayProvider::from_config(&config).expect("provider builds");
        let message = EmailMessage::builder()
            .from("App", "sender@example.com")
            .expect("valid from")
            .to("User", "user@example.net")
            .expect("valid to")
            .subject("Hello")
            .text("Body")
            .build()
            .expect("valid message");

        let error = provider.send(&message).await.expect_err("send fails");

        assert!(error.is_retryable());
    }

    #[tokio::test]
    async fn get_status_reads_send_by_uuid() {
        let (base_url, request_rx) = test_server(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 75\r\n\r\n{\"queued\":false,\"recipients\":[{\"status\":\"accepted\"},{\"status\":\"accepted\"}]}",
        );
        let config = MailbridgeConfig::builder()
            .api_base_url(base_url)
            .expect("valid url")
            .api_key("secret")
            .allowed_from_domain("example.com")
            .build()
            .expect("valid config");
        let provider = HyvorRelayProvider::from_config(&config).expect("provider builds");

        let status = provider
            .get_status(&MessageId::new("abc-123"))
            .await
            .expect("status succeeds");
        let request = request_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("request captured");

        assert_eq!(status, Some(SendStatus::Sent));
        assert!(request.contains("get /api/console/sends/uuid/abc-123 http/1.1"));
        assert!(request.contains("authorization: bearer secret"));
    }

    #[tokio::test]
    async fn get_status_returns_none_for_missing_send() {
        let (base_url, _request_rx) =
            test_server("HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\n\r\nnot found");
        let config = MailbridgeConfig::builder()
            .api_base_url(base_url)
            .expect("valid url")
            .api_key("secret")
            .allowed_from_domain("example.com")
            .build()
            .expect("valid config");
        let provider = HyvorRelayProvider::from_config(&config).expect("provider builds");

        let status = provider
            .get_status(&MessageId::new("missing"))
            .await
            .expect("missing send is not an error");

        assert_eq!(status, None);
    }

    fn test_server(response: &'static str) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener binds");
        let address = listener.local_addr().expect("local addr");
        let (request_tx, request_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("connection accepted");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("read timeout set");
            let mut buffer = [0_u8; 8192];
            let read = stream.read(&mut buffer).expect("request read");
            let request = String::from_utf8_lossy(&buffer[..read]).to_ascii_lowercase();
            request_tx.send(request).expect("request sent");
            stream
                .write_all(response.as_bytes())
                .expect("response written");
        });

        (format!("http://{address}/api/console"), request_rx)
    }
}
