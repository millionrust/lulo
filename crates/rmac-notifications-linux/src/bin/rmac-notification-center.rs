fn main() -> Result<(), Box<dyn std::error::Error>> {
    async_io::block_on(async {
        let (service, events) = rmac_notifications_linux::service::serve().await?;
        // This is one receiver, not a broadcast channel. The future E2 host
        // must record each borrowed event here and then move that same event
        // into banner::BannerSession; it must never create a competing
        // receiver. Persist Center history now without blocking the session-bus
        // dispatch executor.
        while let Ok(event) = events.recv().await {
            let history = service.history().clone();
            if let Some(outcome) = blocking::unblock(move || history.record(&event)).await {
                if !outcome.persisted {
                    eprintln!("notification service failed (History)");
                }
                if let Err(error) = service.emit_indicator(outcome.indicator).await {
                    eprintln!("{error}");
                }
            }
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })
}
