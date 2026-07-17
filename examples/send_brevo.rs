use mailbridge::{BrevoConfig, BrevoProvider, EmailMessage, MailClient, MailbridgeConfig};

#[tokio::main]
async fn main() -> mailbridge::Result<()> {
    let config = MailbridgeConfig::from_env()?;
    let provider_config = BrevoConfig::from_env(&config)?;
    let provider = BrevoProvider::from_config(&provider_config)?;
    let client = MailClient::try_from_config(provider, &config).await?;

    let message = EmailMessage::builder()
        .from("Example App", "no-reply@example.com")?
        .to("User", "user@example.net")?
        .subject("Brevo test")
        .text("Sent through Brevo.")
        .build()?;

    let receipt = client.send(message).await?;
    println!("{}", receipt.message_id());

    Ok(())
}
