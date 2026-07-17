use mailbridge::{EmailMessage, MailClient, MailbridgeConfig, SendPulseConfig, SendPulseProvider};

#[tokio::main]
async fn main() -> mailbridge::Result<()> {
    let config = MailbridgeConfig::from_env()?;
    let provider_config = SendPulseConfig::from_env(&config)?;
    let provider = SendPulseProvider::from_config(&provider_config)?;
    let client = MailClient::try_from_config(provider, &config).await?;

    let message = EmailMessage::builder()
        .from("Example App", "no-reply@example.com")?
        .to("User", "user@example.net")?
        .subject("SendPulse test")
        .text("Sent through SendPulse.")
        .build()?;

    let receipt = client.send(message).await?;
    println!("{}", receipt.message_id());

    Ok(())
}
