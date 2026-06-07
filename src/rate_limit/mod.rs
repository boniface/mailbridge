mod limiter;

pub use limiter::RateLimitConfig;

#[cfg(feature = "rate-limit")]
pub use limiter::MailRateLimiter;
