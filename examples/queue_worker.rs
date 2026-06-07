use mailbridge::{
    EmailMessage, HyvorRelayProvider, MailClient, MailbridgeConfig, QueueWorker, QueueWorkerConfig,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = MailbridgeConfig::from_env()?;
    let provider = HyvorRelayProvider::from_config(&config)?;
    let client = MailClient::try_from_config(provider, &config).await?;
    let queue = client
        .queue()
        .cloned()
        .unwrap_or_else(mailbridge::QueueHandle::memory_default);
    let worker = QueueWorker::new(
        client.clone().with_queue(queue.clone()),
        queue,
        QueueWorkerConfig::new("example-worker"),
    );

    let message = EmailMessage::builder()
        .from("Hashcode", "no-reply@hashcode-fibration.com")?
        .to("User", "user@example.com")?
        .subject("Queued relay test")
        .text("This is a queued test email.")
        .build()?;

    client.enqueue(message).await?;
    worker.run_once().await?;

    Ok(())
}
