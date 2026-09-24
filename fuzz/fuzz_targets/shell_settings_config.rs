//! Fuzzes `rmac-shell-settings`'s stored-settings JSON parser
//! (crates/rmac-shell-settings/src/store.rs, `ShellSettingsStore::load`)
//! by handing it fuzz input through a minimal in-memory `Backend`
//! instead of a real file, so the real version-detection and migration
//! logic runs unmodified.
#![no_main]

use std::io;
use std::path::{Path, PathBuf};

use libfuzzer_sys::fuzz_target;
use rmac_storage::Backend;

/// Returns the fuzz input for every read, mirroring the `InvalidData`
/// error `std::fs::read_to_string` gives on non-UTF-8 files.
struct FuzzBackend<'a>(&'a [u8]);

impl Backend for FuzzBackend<'_> {
    fn read_to_string(&self, _path: &Path) -> io::Result<String> {
        std::str::from_utf8(self.0)
            .map(str::to_owned)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }
}

fuzz_target!(|data: &[u8]| {
    let store = rmac_shell_settings::ShellSettingsStore::with_backend(
        PathBuf::from("/fuzz/rmac/shell-settings.json"),
        FuzzBackend(data),
    );
    let _ = store.load();
});
