use mailbridge::{EmailMessage, HyvorRelayProvider, MailClient, MailbridgeConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = MailbridgeConfig::from_env()?;
    let provider = HyvorRelayProvider::from_config(&config)?;
    let client = MailClient::try_from_config(provider, &config).await?;

    let message = EmailMessage::builder()
        .from("Hashcode", "no-reply@hashcode-fibration.com")?
        .to("User", "user@example.com")?
        .subject("Relay test")
        .text("This is a test email.")
        .build()?;

    let receipt = client.send(message).await?;
    println!("{}", receipt.message_id());

    Ok(())
}
