mod r#trait;

#[cfg(any(feature = "sendgrid", feature = "sendpulse", feature = "mailgun"))]
mod shared;

#[cfg(all(feature = "hyvor-relay", feature = "api"))]
mod hyvor_relay;
#[cfg(feature = "mailgun")]
mod mailgun;
#[cfg(feature = "sendgrid")]
mod sendgrid;
#[cfg(feature = "sendpulse")]
mod sendpulse;

#[cfg(all(feature = "hyvor-relay", feature = "api"))]
pub use hyvor_relay::HyvorRelayProvider;
#[cfg(feature = "mailgun")]
pub use mailgun::{MailgunConfig, MailgunProvider};
#[cfg(feature = "sendgrid")]
pub use sendgrid::{SendGridConfig, SendGridProvider};
#[cfg(feature = "sendpulse")]
pub use sendpulse::{SendPulseConfig, SendPulseProvider};
pub use r#trait::{MailProvider, SendStatus};
