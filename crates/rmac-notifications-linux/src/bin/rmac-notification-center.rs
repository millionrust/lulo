fn main() -> Result<(), Box<dyn std::error::Error>> {
    async_io::block_on(async {
        let (_service, events) = rmac_notifications_linux::service::serve().await?;
        // E2/E3 will consume these events to render banners and Notification
        // Center. Draining them now keeps the protocol service responsive
        // without polling or pretending that presentation already exists.
        while events.recv().await.is_ok() {}
        Ok::<_, Box<dyn std::error::Error>>(())
    })
}
