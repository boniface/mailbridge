#[cfg(feature = "rate-limit")]
use std::collections::BTreeMap;
#[cfg(feature = "rate-limit")]
use std::num::NonZeroU32;
#[cfg(feature = "rate-limit")]
use std::sync::Arc;

#[cfg(feature = "rate-limit")]
use governor::{DefaultDirectRateLimiter, Quota, RateLimiter};

#[cfg(feature = "rate-limit")]
use crate::error::{MailError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitConfig {
    global_per_second: u32,
    domain_per_second: u32,
}

impl RateLimitConfig {
    pub const DEFAULT_GLOBAL_PER_SECOND: u32 = 20;
    pub const DEFAULT_DOMAIN_PER_SECOND: u32 = 5;

    #[must_use]
    pub const fn new(global_per_second: u32, domain_per_second: u32) -> Self {
        Self {
            global_per_second,
            domain_per_second,
        }
    }

    #[must_use]
    pub const fn global_per_second(&self) -> u32 {
        self.global_per_second
    }

    #[must_use]
    pub const fn domain_per_second(&self) -> u32 {
        self.domain_per_second
    }
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self::new(
            Self::DEFAULT_GLOBAL_PER_SECOND,
            Self::DEFAULT_DOMAIN_PER_SECOND,
        )
    }
}

#[cfg(feature = "rate-limit")]
#[derive(Debug, Clone)]
pub struct MailRateLimiter {
    global: Arc<DefaultDirectRateLimiter>,
    domains: Arc<BTreeMap<String, Arc<DefaultDirectRateLimiter>>>,
    fallback_domain: Arc<DefaultDirectRateLimiter>,
}

#[cfg(feature = "rate-limit")]
impl MailRateLimiter {
    #[must_use]
    pub fn new(config: &RateLimitConfig, domains: impl IntoIterator<Item = String>) -> Self {
        let global = Arc::new(direct_limiter(config.global_per_second()));
        let fallback_domain = Arc::new(direct_limiter(config.domain_per_second()));
        let domains = domains
            .into_iter()
            .map(|domain| (domain, Arc::new(direct_limiter(config.domain_per_second()))))
            .collect();

        Self {
            global,
            domains: Arc::new(domains),
            fallback_domain,
        }
    }

    pub async fn wait(&self, domain: &str) {
        self.global.until_ready().await;
        self.domain_limiter(domain).until_ready().await;
    }

    pub fn check(&self, domain: &str) -> Result<()> {
        self.global.check().map_err(|_| MailError::RateLimited)?;
        self.domain_limiter(domain)
            .check()
            .map_err(|_| MailError::RateLimited)?;
        Ok(())
    }

    fn domain_limiter(&self, domain: &str) -> &DefaultDirectRateLimiter {
        self.domains
            .get(domain)
            .map_or(self.fallback_domain.as_ref(), Arc::as_ref)
    }
}

#[cfg(feature = "rate-limit")]
fn direct_limiter(per_second: u32) -> DefaultDirectRateLimiter {
    RateLimiter::direct(Quota::per_second(non_zero_rate(per_second)))
}

#[cfg(feature = "rate-limit")]
fn non_zero_rate(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value.max(1)).expect("rate limit was clamped to at least one")
}

#[cfg(all(test, feature = "rate-limit"))]
mod tests {
    use super::*;

    #[test]
    fn check_rejects_when_global_limit_is_exhausted() {
        let limiter =
            MailRateLimiter::new(&RateLimitConfig::new(1, 10), ["example.com".to_owned()]);

        assert!(limiter.check("example.com").is_ok());
        assert_eq!(
            limiter
                .check("example.com")
                .expect_err("second check fails"),
            MailError::RateLimited
        );
    }
}
