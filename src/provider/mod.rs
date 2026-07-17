mod r#trait;

#[cfg(any(
    feature = "sendgrid",
    feature = "sendpulse",
    feature = "mailgun",
    feature = "mailjet",
    feature = "bird",
    feature = "resend",
    feature = "brevo"
))]
mod shared;

#[cfg(feature = "bird")]
mod bird;
#[cfg(feature = "brevo")]
mod brevo;
#[cfg(all(feature = "hyvor-relay", feature = "api"))]
mod hyvor_relay;
#[cfg(feature = "mailgun")]
mod mailgun;
#[cfg(feature = "mailjet")]
mod mailjet;
#[cfg(feature = "resend")]
mod resend;
#[cfg(feature = "sendgrid")]
mod sendgrid;
#[cfg(feature = "sendpulse")]
mod sendpulse;

#[cfg(feature = "bird")]
pub use bird::{BirdConfig, BirdProvider};
#[cfg(feature = "brevo")]
pub use brevo::{BrevoConfig, BrevoProvider};
#[cfg(all(feature = "hyvor-relay", feature = "api"))]
pub use hyvor_relay::HyvorRelayProvider;
#[cfg(feature = "mailgun")]
pub use mailgun::{MailgunConfig, MailgunProvider};
#[cfg(feature = "mailjet")]
pub use mailjet::{MailjetApiVersion, MailjetConfig, MailjetProvider};
#[cfg(feature = "resend")]
pub use resend::{ResendConfig, ResendProvider};
#[cfg(feature = "sendgrid")]
pub use sendgrid::{SendGridConfig, SendGridProvider};
#[cfg(feature = "sendpulse")]
pub use sendpulse::{SendPulseConfig, SendPulseProvider};
pub use r#trait::{MailProvider, ProviderCapabilities, SendStatus};
