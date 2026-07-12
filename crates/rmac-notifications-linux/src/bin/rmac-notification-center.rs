fn main() -> Result<(), Box<dyn std::error::Error>> {
    async_io::block_on(async {
        let (service, events) = rmac_notifications_linux::service::serve().await?;
        // Presentation consumes the same bounded stream in E2. Persist Center
        // history now without blocking the session-bus dispatch executor.
        while let Ok(event) = events.recv().await {
            let history = service.history().clone();
            if let Err(error) = blocking::unblock(move || history.record(&event)).await {
                eprintln!("{error}");
            }
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })
}
