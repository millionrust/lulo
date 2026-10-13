//! `org.rmac.Voice1`: D-Bus activated by the talk key, exits when idle
//! (ADR 0024 §8).

fn main() {
    async_io::block_on(async {
        match rmac_intelligence_service::voice::service::serve().await {
            Ok(handle) => rmac_intelligence_service::voice::service::run(handle).await,
            Err(error) => {
                eprintln!("rmac-voice-service: could not own the bus name: {error}");
                std::process::exit(1);
            }
        }
    });
}
