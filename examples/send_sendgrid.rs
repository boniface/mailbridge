use mailbridge::{EmailMessage, MailClient, MailbridgeConfig, SendGridConfig, SendGridProvider};

#[tokio::main]
async fn main() -> mailbridge::Result<()> {
    let config = MailbridgeConfig::from_env()?;
    let provider_config = SendGridConfig::from_env(&config)?;
    let provider = SendGridProvider::from_config(&provider_config)?;
    let client = MailClient::try_from_config(provider, &config).await?;

    let message = EmailMessage::builder()
        .from("Example App", "no-reply@example.com")?
        .to("User", "user@example.net")?
        .subject("SendGrid test")
        .text("Sent through SendGrid.")
        .build()?;

    let receipt = client.send(message).await?;
    println!("{}", receipt.message_id());

    Ok(())
}
