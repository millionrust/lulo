//! Fuzzes `rmac-theme`'s stored-preferences JSON parser
//! (crates/rmac-theme/src/store.rs, `ThemeStore::load`) through the same
//! in-memory `Backend` trick as shell_settings_config.rs.
#![no_main]

use std::io;
use std::path::{Path, PathBuf};

use libfuzzer_sys::fuzz_target;
use rmac_storage::Backend;

struct FuzzBackend<'a>(&'a [u8]);

impl Backend for FuzzBackend<'_> {
    fn read_to_string(&self, _path: &Path) -> io::Result<String> {
        std::str::from_utf8(self.0)
            .map(str::to_owned)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }
}

fuzz_target!(|data: &[u8]| {
    let store = rmac_theme::ThemeStore::with_backend(
        PathBuf::from("/fuzz/rmac/theme.json"),
        FuzzBackend(data),
    );
    let host = rmac_appearance::Snapshot::default();
    let _ = store.load(&host);
});
