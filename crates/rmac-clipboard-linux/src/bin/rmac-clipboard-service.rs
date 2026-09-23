fn main() -> Result<(), Box<dyn std::error::Error>> {
    async_io::block_on(async {
        let service = rmac_clipboard_linux::service::serve().await?;
        rmac_clipboard_linux::service::run(service).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })
}
