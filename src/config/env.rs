use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use url::Url;

use crate::email::EmailAddress;
use crate::error::{MailError, Result};
use crate::rate_limit::RateLimitConfig;

const DEFAULT_RETRY_BASE_DELAY_MS: u64 = 500;
const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 15;
const DEFAULT_MAX_RETRIES: u32 = 5;

#[derive(Debug)]
pub struct MailbridgeConfig {
    api_base_url: Url,
    api_key: SecretString,
    allowed_from_domains: BTreeSet<String>,
    default_from: Option<EmailAddress>,
    smtp: Option<SmtpConfig>,
    queue_backend: QueueBackend,
    rate_limit: RateLimitConfig,
    max_retries: u32,
    retry_base_delay: Duration,
    request_timeout: Duration,
}

impl MailbridgeConfig {
    #[must_use]
    pub fn builder() -> MailbridgeConfigBuilder {
        MailbridgeConfigBuilder::default()
    }

    /// Builds configuration from `RELAY_*` environment variables.
    ///
    /// # Errors
    ///
    /// Returns an error when required environment variables are missing,
    /// malformed, or select a disabled queue backend.
    pub fn from_env() -> Result<Self> {
        #[cfg(feature = "dotenv")]
        {
            let _ = dotenvy::dotenv();
        }

        let mut builder = Self::builder()
            .api_base_url(required_env("RELAY_API_BASE_URL")?)?
            .api_key(required_env("RELAY_API_KEY")?);

        builder = parse_domains(&required_env("RELAY_ALLOWED_FROM_DOMAINS")?)?
            .into_iter()
            .fold(builder, MailbridgeConfigBuilder::allowed_from_domain);

        if let Some(default_email) = optional_env("RELAY_DEFAULT_FROM_EMAIL") {
            let default_name = optional_env("RELAY_DEFAULT_FROM_NAME").unwrap_or_default();
            builder = builder.default_from(default_name, default_email)?;
        }

        if let Some(smtp) = SmtpConfig::from_env()? {
            builder = builder.smtp(smtp);
        }

        builder = builder
            .queue_backend(parse_queue_backend()?)
            .rate_limit(RateLimitConfig::new(
                optional_env_u32("RELAY_GLOBAL_RATE_PER_SECOND")?
                    .unwrap_or(RateLimitConfig::DEFAULT_GLOBAL_PER_SECOND),
                optional_env_u32("RELAY_DOMAIN_RATE_PER_SECOND")?
                    .unwrap_or(RateLimitConfig::DEFAULT_DOMAIN_PER_SECOND),
            ))
            .max_retries(optional_env_u32("RELAY_MAX_RETRIES")?.unwrap_or(DEFAULT_MAX_RETRIES))
            .retry_base_delay(Duration::from_millis(
                optional_env_u64("RELAY_RETRY_BASE_DELAY_MS")?
                    .unwrap_or(DEFAULT_RETRY_BASE_DELAY_MS),
            ))
            .request_timeout(Duration::from_secs(
                optional_env_u64("RELAY_REQUEST_TIMEOUT_SECS")?
                    .unwrap_or(DEFAULT_REQUEST_TIMEOUT_SECS),
            ));

        builder.build()
    }

    #[must_use]
    pub fn api_base_url(&self) -> &Url {
        &self.api_base_url
    }

    #[must_use]
    pub fn api_key(&self) -> &SecretString {
        &self.api_key
    }

    #[must_use]
    pub fn allowed_from_domains(&self) -> &BTreeSet<String> {
        &self.allowed_from_domains
    }

    #[must_use]
    pub fn default_from(&self) -> Option<&EmailAddress> {
        self.default_from.as_ref()
    }

    #[must_use]
    pub fn smtp(&self) -> Option<&SmtpConfig> {
        self.smtp.as_ref()
    }

    #[must_use]
    pub fn queue_backend(&self) -> &QueueBackend {
        &self.queue_backend
    }

    #[must_use]
    pub fn rate_limit(&self) -> &RateLimitConfig {
        &self.rate_limit
    }

    #[must_use]
    pub const fn max_retries(&self) -> u32 {
        self.max_retries
    }

    #[must_use]
    pub const fn retry_base_delay(&self) -> Duration {
        self.retry_base_delay
    }

    #[must_use]
    pub const fn request_timeout(&self) -> Duration {
        self.request_timeout
    }
}

#[derive(Debug)]
pub struct MailbridgeConfigBuilder {
    api_base_url: Option<Url>,
    api_key: Option<SecretString>,
    allowed_from_domains: BTreeSet<String>,
    default_from: Option<EmailAddress>,
    smtp: Option<SmtpConfig>,
    queue_backend: QueueBackend,
    rate_limit: RateLimitConfig,
    max_retries: u32,
    retry_base_delay: Duration,
    request_timeout: Duration,
}

impl MailbridgeConfigBuilder {
    /// Sets and validates the Relay API base URL.
    ///
    /// # Errors
    ///
    /// Returns an error when `url` is not a valid URL.
    pub fn api_base_url(mut self, url: impl AsRef<str>) -> Result<Self> {
        self.api_base_url =
            Some(Url::parse(url.as_ref()).map_err(|error| {
                MailError::Config(format!("invalid relay api base url: {error}"))
            })?);
        Ok(self)
    }

    #[must_use]
    pub fn api_key(mut self, value: impl Into<String>) -> Self {
        self.api_key = Some(secret_string(value.into()));
        self
    }

    #[must_use]
    pub fn allowed_from_domain(mut self, domain: impl Into<String>) -> Self {
        let domain = normalize_domain(&domain.into());
        if !domain.is_empty() {
            self.allowed_from_domains.insert(domain);
        }
        self
    }

    /// Sets the default sender address.
    ///
    /// # Errors
    ///
    /// Returns an error when the sender address is invalid.
    pub fn default_from(
        mut self,
        name: impl Into<String>,
        email: impl Into<String>,
    ) -> Result<Self> {
        self.default_from = Some(EmailAddress::new(name, email)?);
        Ok(self)
    }

    #[must_use]
    pub fn smtp(mut self, smtp: SmtpConfig) -> Self {
        self.smtp = Some(smtp);
        self
    }

    #[must_use]
    pub fn queue_backend(mut self, backend: QueueBackend) -> Self {
        self.queue_backend = backend;
        self
    }

    #[must_use]
    pub fn rate_limit(mut self, config: RateLimitConfig) -> Self {
        self.rate_limit = config;
        self
    }

    #[must_use]
    pub const fn max_retries(mut self, value: u32) -> Self {
        self.max_retries = value;
        self
    }

    #[must_use]
    pub const fn retry_base_delay(mut self, value: Duration) -> Self {
        self.retry_base_delay = value;
        self
    }

    #[must_use]
    pub const fn request_timeout(mut self, value: Duration) -> Self {
        self.request_timeout = value;
        self
    }

    /// Builds a validated configuration value.
    ///
    /// # Errors
    ///
    /// Returns an error when required fields are missing, values are invalid,
    /// or the selected queue backend is not available with enabled features.
    pub fn build(self) -> Result<MailbridgeConfig> {
        let api_base_url = self
            .api_base_url
            .ok_or_else(|| MailError::Config("relay api base url is required".to_owned()))?;
        let api_key = self
            .api_key
            .ok_or_else(|| MailError::Config("relay api key is required".to_owned()))?;

        if self.allowed_from_domains.is_empty() {
            return Err(MailError::Config(
                "at least one allowed from domain is required".to_owned(),
            ));
        }

        if api_key.expose_secret().trim().is_empty() {
            return Err(MailError::Config("relay api key is required".to_owned()));
        }

        #[cfg(not(all(
            feature = "queue-sqlite",
            feature = "queue-postgres",
            feature = "queue-scylla"
        )))]
        validate_queue_backend_features(&self.queue_backend)?;
        validate_rate_limit(&self.rate_limit)?;

        if let Some(default_from) = &self.default_from
            && !self.allowed_from_domains.contains(default_from.domain())
        {
            return Err(MailError::Config(format!(
                "default from domain is not allowed: {}",
                default_from.domain()
            )));
        }

        Ok(MailbridgeConfig {
            api_base_url,
            api_key,
            allowed_from_domains: self.allowed_from_domains,
            default_from: self.default_from,
            smtp: self.smtp,
            queue_backend: self.queue_backend,
            rate_limit: self.rate_limit,
            max_retries: self.max_retries,
            retry_base_delay: self.retry_base_delay,
            request_timeout: self.request_timeout,
        })
    }
}

impl Default for MailbridgeConfigBuilder {
    fn default() -> Self {
        Self {
            api_base_url: None,
            api_key: None,
            allowed_from_domains: BTreeSet::new(),
            default_from: None,
            smtp: None,
            queue_backend: QueueBackend::Memory,
            rate_limit: RateLimitConfig::default(),
            max_retries: DEFAULT_MAX_RETRIES,
            retry_base_delay: Duration::from_millis(DEFAULT_RETRY_BASE_DELAY_MS),
            request_timeout: Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS),
        }
    }
}

#[derive(Debug, Default)]
pub enum QueueBackend {
    #[default]
    Memory,
    Sqlite {
        path: PathBuf,
    },
    Postgres {
        database_url: SecretString,
    },
    Scylla {
        uri: String,
        keyspace: String,
        table: String,
    },
}

#[derive(Debug)]
pub struct SmtpConfig {
    host: String,
    port: u16,
    username: String,
    password: SecretString,
}

impl SmtpConfig {
    /// Builds SMTP configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when `host` or `username` is empty.
    pub fn new(
        host: impl Into<String>,
        port: u16,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<Self> {
        let host = host.into();
        let username = username.into();

        if host.trim().is_empty() {
            return Err(MailError::Config("smtp host is required".to_owned()));
        }

        if username.trim().is_empty() {
            return Err(MailError::Config("smtp username is required".to_owned()));
        }

        Ok(Self {
            host,
            port,
            username,
            password: secret_string(password.into()),
        })
    }

    fn from_env() -> Result<Option<Self>> {
        let Some(host) = optional_env("RELAY_SMTP_HOST") else {
            return Ok(None);
        };
        let port = required_env("RELAY_SMTP_PORT")?
            .parse::<u16>()
            .map_err(|error| MailError::Config(format!("invalid relay smtp port: {error}")))?;
        let username = required_env("RELAY_SMTP_USERNAME")?;
        let password = required_env("RELAY_SMTP_PASSWORD")?;

        Self::new(host, port, username, password).map(Some)
    }

    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    #[must_use]
    pub fn username(&self) -> &str {
        &self.username
    }

    #[must_use]
    pub fn password(&self) -> &SecretString {
        &self.password
    }
}

fn required_env(key: &str) -> Result<String> {
    std::env::var(key).map_err(|error| MailError::Config(format!("{key} is required: {error}")))
}

fn optional_env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

fn optional_env_u32(key: &str) -> Result<Option<u32>> {
    optional_env(key)
        .map(|value| {
            value
                .parse::<u32>()
                .map_err(|error| MailError::Config(format!("invalid {key}: {error}")))
        })
        .transpose()
}

fn optional_env_u64(key: &str) -> Result<Option<u64>> {
    optional_env(key)
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|error| MailError::Config(format!("invalid {key}: {error}")))
        })
        .transpose()
}

fn parse_domains(value: &str) -> Result<BTreeSet<String>> {
    let domains = value
        .split(',')
        .map(normalize_domain)
        .filter(|domain| !domain.is_empty())
        .collect::<BTreeSet<_>>();

    if domains.is_empty() {
        return Err(MailError::Config(
            "at least one allowed from domain is required".to_owned(),
        ));
    }

    Ok(domains)
}

fn parse_queue_backend() -> Result<QueueBackend> {
    match optional_env("RELAY_QUEUE_BACKEND")
        .unwrap_or_else(|| "memory".to_owned())
        .to_ascii_lowercase()
        .as_str()
    {
        "memory" => Ok(QueueBackend::Memory),
        "sqlite" => Ok(QueueBackend::Sqlite {
            path: PathBuf::from(
                optional_env("RELAY_QUEUE_SQLITE_PATH")
                    .unwrap_or_else(|| "./relay-mail-queue.sqlite".to_owned()),
            ),
        }),
        "postgres" => Ok(QueueBackend::Postgres {
            database_url: secret_string(required_env("RELAY_QUEUE_POSTGRES_URL")?),
        }),
        "scylla" => Ok(QueueBackend::Scylla {
            uri: required_env("RELAY_QUEUE_SCYLLA_URI")?,
            keyspace: required_env("RELAY_QUEUE_SCYLLA_KEYSPACE")?,
            table: required_env("RELAY_QUEUE_SCYLLA_TABLE")?,
        }),
        value => Err(MailError::Config(format!(
            "unsupported relay queue backend: {value}"
        ))),
    }
}

#[cfg(not(all(
    feature = "queue-sqlite",
    feature = "queue-postgres",
    feature = "queue-scylla"
)))]
fn validate_queue_backend_features(backend: &QueueBackend) -> Result<()> {
    match backend {
        QueueBackend::Memory => Ok(()),
        QueueBackend::Sqlite { .. } => {
            #[cfg(feature = "queue-sqlite")]
            {
                Ok(())
            }
            #[cfg(not(feature = "queue-sqlite"))]
            {
                Err(MailError::Config(
                    "queue backend sqlite requires the queue-sqlite feature".to_owned(),
                ))
            }
        }
        QueueBackend::Postgres { .. } => {
            #[cfg(feature = "queue-postgres")]
            {
                Ok(())
            }
            #[cfg(not(feature = "queue-postgres"))]
            {
                Err(MailError::Config(
                    "queue backend postgres requires the queue-postgres feature".to_owned(),
                ))
            }
        }
        QueueBackend::Scylla { .. } => {
            #[cfg(feature = "queue-scylla")]
            {
                Ok(())
            }
            #[cfg(not(feature = "queue-scylla"))]
            {
                Err(MailError::Config(
                    "queue backend scylla requires the queue-scylla feature".to_owned(),
                ))
            }
        }
    }
}

fn validate_rate_limit(config: &RateLimitConfig) -> Result<()> {
    if config.global_per_second() == 0 || config.domain_per_second() == 0 {
        return Err(MailError::Config(
            "rate limits must be greater than zero".to_owned(),
        ));
    }

    Ok(())
}

fn normalize_domain(domain: &str) -> String {
    domain.trim().trim_matches('.').to_ascii_lowercase()
}

fn secret_string(value: String) -> SecretString {
    SecretString::new(value.into_boxed_str())
}

impl QueueBackend {
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Sqlite { .. } => "sqlite",
            Self::Postgres { .. } => "postgres",
            Self::Scylla { .. } => "scylla",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_builds_valid_config_with_defaults() {
        let config = MailbridgeConfig::builder()
            .api_base_url("https://relay.example.com/api/console")
            .expect("valid url")
            .api_key("secret")
            .allowed_from_domain("Example.COM")
            .build()
            .expect("valid config");

        assert!(config.allowed_from_domains().contains("example.com"));
        assert_eq!(config.queue_backend().kind(), "memory");
        assert_eq!(config.max_retries(), 5);
    }

    #[test]
    fn builder_rejects_missing_allowed_domains() {
        let error = MailbridgeConfig::builder()
            .api_base_url("https://relay.example.com/api/console")
            .expect("valid url")
            .api_key("secret")
            .build()
            .expect_err("allowed domain is required");

        assert!(matches!(error, MailError::Config(_)));
    }

    #[test]
    fn builder_rejects_default_from_domain_outside_allowlist() {
        let error = MailbridgeConfig::builder()
            .api_base_url("https://relay.example.com/api/console")
            .expect("valid url")
            .api_key("secret")
            .allowed_from_domain("example.com")
            .default_from("App", "sender@other.example")
            .expect("valid address")
            .build()
            .expect_err("default sender domain should be rejected");

        assert!(matches!(error, MailError::Config(_)));
    }

    #[test]
    fn builder_rejects_zero_rate_limit() {
        let error = MailbridgeConfig::builder()
            .api_base_url("https://relay.example.com/api/console")
            .expect("valid url")
            .api_key("secret")
            .allowed_from_domain("example.com")
            .rate_limit(RateLimitConfig::new(0, 5))
            .build()
            .expect_err("zero rate should be rejected");

        assert!(matches!(error, MailError::Config(_)));
    }
}
