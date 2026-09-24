//! Fuzzes the freedesktop `.desktop` entry tokenizer that
//! `rmac-apps` uses to read installed-application catalogs
//! (crates/rmac-apps/src/platform.rs, `desktop_group_named`).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(contents) = std::str::from_utf8(data) {
        let _ = rmac_apps::desktop_group_named(contents, "Desktop Entry");
    }
});
