fn main() -> Result<(), Box<dyn std::error::Error>> {
    async_io::block_on(async {
        let service = rmac_focus_linux::service::serve().await?;
        rmac_focus_linux::service::run(service).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })
}
