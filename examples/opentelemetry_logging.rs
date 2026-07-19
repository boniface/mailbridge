use async_trait::async_trait;
use mailbridge::{EmailMessage, MailClient, MailProvider, MessageId, Result, SendReceipt};
use tracing_subscriber::{EnvFilter, fmt};

#[derive(Debug, Clone, PartialEq, Eq)]
struct LocalTelemetryProvider;

#[async_trait]
impl MailProvider for LocalTelemetryProvider {
    async fn send(&self, message: &EmailMessage) -> Result<SendReceipt> {
        Ok(SendReceipt::new(
            self.provider_name(),
            MessageId::new(format!("local-{}", message.subject().len())),
            None,
        ))
    }

    fn provider_name(&self) -> &'static str {
        "local-telemetry"
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let filter = match EnvFilter::try_from_default_env() {
        Ok(filter) => filter,
        Err(_error) => EnvFilter::new("mailbridge=info"),
    };

    fmt().json().with_env_filter(filter).init();

    let client = MailClient::new(LocalTelemetryProvider);
    let message = EmailMessage::builder()
        .from("Example App", "no-reply@example.com")?
        .to("User", "user@example.net")?
        .subject("Telemetry smoke test")
        .text("This local provider does not send a live email.")
        .build()?;

    let _receipt = client.send(message).await?;

    Ok(())
}
