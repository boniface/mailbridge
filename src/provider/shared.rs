use std::collections::BTreeSet;
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use url::Url;

use crate::error::{MailError, Result};

#[cfg(any(feature = "sendgrid", feature = "sendpulse"))]
use std::borrow::Cow;

#[cfg(any(feature = "sendgrid", feature = "sendpulse"))]
use base64::Engine;
#[cfg(any(feature = "sendgrid", feature = "sendpulse"))]
use serde::Serialize;

#[cfg(any(feature = "sendgrid", feature = "sendpulse"))]
use crate::email::{Attachment, EmailAddress};

#[cfg(any(feature = "sendgrid", feature = "sendpulse"))]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(super) struct ApiAddress<'a> {
    pub(super) email: Cow<'a, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) name: Option<Cow<'a, str>>,
}

#[cfg(any(feature = "sendgrid", feature = "sendpulse"))]
impl<'a> From<&'a EmailAddress> for ApiAddress<'a> {
    fn from(address: &'a EmailAddress) -> Self {
        Self {
            email: Cow::Borrowed(address.email()),
            name: address.name().map(Cow::Borrowed),
        }
    }
}

#[cfg(any(feature = "sendgrid", feature = "sendpulse"))]
pub(super) fn addresses(addresses: &[EmailAddress]) -> Vec<ApiAddress<'_>> {
    addresses.iter().map(ApiAddress::from).collect()
}

#[cfg(any(feature = "sendgrid", feature = "sendpulse"))]
pub(super) fn attachment_content_base64(attachment: &Attachment) -> String {
    base64::engine::general_purpose::STANDARD.encode(attachment.content())
}

pub(super) fn validate_secret(value: &str, provider: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(MailError::Config(format!("{provider} api key is required")));
    }

    Ok(())
}

pub(super) fn configured_http_client(timeout: Duration) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|error| MailError::Config(format!("failed to build http client: {error}")))
}

pub(super) fn parse_base_url(value: &str, provider: &str) -> Result<Url> {
    Url::parse(value).map_err(|error| {
        MailError::Config(format!("invalid {provider} api base url {value}: {error}"))
    })
}

pub(super) fn join_url(base_url: &Url, path: &str, provider: &str) -> Result<Url> {
    let mut base = base_url.clone();
    if !base.path().ends_with('/') {
        let path = format!("{}/", base.path());
        base.set_path(&path);
    }

    base.join(path)
        .map_err(|error| MailError::Config(format!("invalid {provider} api url: {error}")))
}

pub(super) fn optional_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

pub(super) fn required_env(key: &str) -> Result<String> {
    optional_env(key).ok_or_else(|| MailError::Config(format!("{key} is required")))
}

pub(super) fn secret_copy(secret: &SecretString) -> SecretString {
    SecretString::new(secret.expose_secret().to_owned().into_boxed_str())
}

pub(super) fn secret_from_env(key: &str) -> Result<SecretString> {
    Ok(SecretString::new(required_env(key)?.into_boxed_str()))
}

pub(super) fn cloned_domains(domains: &BTreeSet<String>) -> BTreeSet<String> {
    domains.iter().cloned().collect()
}

pub(super) async fn response_text(response: reqwest::Response, provider: &str) -> String {
    response
        .text()
        .await
        .unwrap_or_else(|error| format!("failed to read {provider} error response: {error}"))
}

pub(super) fn provider_error(provider: &str, status: u16, message: String) -> MailError {
    match status {
        401 | 403 => MailError::Authentication,
        408 | 409 | 425 | 429 | 500..=599 => MailError::Temporary(format!(
            "{provider} temporary failure: status={status}, message={message}"
        )),
        _ => MailError::RelayRejected { status, message },
    }
}
