//! `org.rmac.Intelligence1`: D-Bus activated, exits when idle (ADR 0024).

fn main() {
    async_io::block_on(async {
        match rmac_intelligence_service::service::serve().await {
            Ok(handle) => rmac_intelligence_service::service::run(handle).await,
            Err(error) => {
                eprintln!("rmac-intelligence-service: could not own the bus name: {error}");
                std::process::exit(1);
            }
        }
    });
}
