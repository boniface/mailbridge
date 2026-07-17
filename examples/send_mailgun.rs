use mailbridge::{EmailMessage, MailClient, MailbridgeConfig, MailgunConfig, MailgunProvider};

#[tokio::main]
async fn main() -> mailbridge::Result<()> {
    let config = MailbridgeConfig::from_env()?;
    let provider_config = MailgunConfig::from_env(&config)?;
    let provider = MailgunProvider::from_config(&provider_config)?;
    let client = MailClient::try_from_config(provider, &config).await?;

    let message = EmailMessage::builder()
        .from("Example App", "no-reply@example.com")?
        .to("User", "user@example.net")?
        .subject("Mailgun test")
        .text("Sent through Mailgun.")
        .build()?;

    let receipt = client.send(message).await?;
    println!("{}", receipt.message_id());

    Ok(())
}
