//! Fuzzes deserialization of niri's IPC wire protocol
//! (crates/rmac-compositor-niri/src/wire.rs), the same `serde_json`
//! `Deserialize` impls `translate::decode_event` and
//! `transport::request_reply_once` parse event-stream lines and command
//! replies with.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rmac_compositor_niri::wire::{Event, Reply};

fuzz_target!(|data: &[u8]| {
    if let Ok(line) = std::str::from_utf8(data) {
        let _ = serde_json::from_str::<Event>(line);
        let _ = serde_json::from_str::<Reply>(line);
    }
});
