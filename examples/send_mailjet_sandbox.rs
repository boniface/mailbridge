use mailbridge::{EmailMessage, MailClient, MailbridgeConfig, MailjetConfig, MailjetProvider};

#[tokio::main]
async fn main() -> mailbridge::Result<()> {
    let config = MailbridgeConfig::from_env()?;
    let provider_config = MailjetConfig::from_env(&config)?.with_sandbox_mode(true);
    let provider = MailjetProvider::from_config(&provider_config)?;
    let client = MailClient::try_from_config(provider, &config).await?;

    let message = EmailMessage::builder()
        .from("Example App", "no-reply@example.com")?
        .to("User", "user@example.net")?
        .subject("Mailjet sandbox test")
        .text("Validated through Mailjet sandbox mode.")
        .build()?;

    let receipt = client.send(message).await?;
    println!("{}", receipt.message_id());

    Ok(())
}
