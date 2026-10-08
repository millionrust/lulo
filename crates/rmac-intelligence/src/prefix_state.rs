//! Where the evaluated prompt prefix is kept between service runs (ADR
//! 0024 "Prompt-prefix caching", "Phase 1.1").
//!
//! The service exits a minute after its last request, so the state it
//! saves on disk is what makes the next start warm: reading it back takes
//! tens of milliseconds, evaluating the prefix again takes seconds. The
//! file lives in `$XDG_CACHE_HOME/lulo/intelligence/` (directory 0700, file
//! 0600) and its name is a digest of everything the state depends on: the
//! model's SHA-256, the prompt version, style and text, the context size
//! and the llama.cpp binding's version. An update that changes any of
//! them names a different file, so a stale state is never restored, and
//! the service deletes the older files once the new one is written.
//!
//! This module only names and finds the file, so System Settings can tell
//! cheaply (one `stat`) whether the model is ready or still needs its
//! one-time warm-up, without loading anything.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::manifest::Model;
use crate::prompt::{self, PromptStyle};

/// The context the service creates: the prefix plus the longest request and
/// answer, with room to spare.
pub const CONTEXT_TOKENS: u32 = 1536;

/// The llama.cpp binding the service is built with. A unit test holds it
/// equal to `Cargo.lock`, so a dependency bump cannot keep an old state.
pub const ENGINE_VERSION: &str = "llama-cpp-2 0.1.158";

const PREFIX: &str = "prefix-";
const SUFFIX: &str = ".state";

/// The state file for this model and prompt style in `directory`.
pub fn file(directory: &Path, model: &Model, style: PromptStyle) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(model.sha256.as_bytes());
    hasher.update(prompt::PROMPT_VERSION.to_le_bytes());
    hasher.update(style.as_str().as_bytes());
    hasher.update(prompt::prefix(style).as_bytes());
    hasher.update(CONTEXT_TOKENS.to_le_bytes());
    hasher.update(ENGINE_VERSION.as_bytes());
    let digest = crate::verify::hex(&hasher.finalize());
    directory.join(format!("{PREFIX}{}{SUFFIX}", &digest[..24]))
}

/// The state file the service would read for `model` with the default
/// prompt, in the user's cache directory.
pub fn current(model: &Model) -> Option<PathBuf> {
    crate::paths::cache_dir().map(|directory| file(&directory, model, PromptStyle::DEFAULT))
}

/// Whether the warm-up has been done for `model`: its prefix state is on
/// disk for the current prompt and engine.
pub fn is_ready(model: &Model) -> bool {
    current(model).is_some_and(|path| path.is_file())
}

/// Delete every saved prefix state in `directory` except `keep` (an update
/// changed the prompt or the model, so the old ones can never be used).
pub fn remove_others(directory: &Path, keep: &Path) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path != keep && name.starts_with(PREFIX) && name.contains(SUFFIX) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Tier;

    #[test]
    fn the_name_follows_the_model_and_the_prompt() {
        let directory = Path::new("/cache");
        let tiny = file(directory, Tier::Tiny.model(), PromptStyle::List);
        let standard = file(directory, Tier::Standard.model(), PromptStyle::List);
        let chat = file(directory, Tier::Tiny.model(), PromptStyle::Chat);
        assert_ne!(tiny, standard);
        assert_ne!(tiny, chat);
        assert_eq!(tiny, file(directory, Tier::Tiny.model(), PromptStyle::List));
        let name = tiny.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with(PREFIX) && name.ends_with(SUFFIX), "{name}");
    }

    #[test]
    fn the_engine_version_matches_the_lockfile() {
        let lock = include_str!("../../../Cargo.lock");
        let version = lock
            .split("[[package]]")
            .find(|package| package.contains("\nname = \"llama-cpp-2\"\n"))
            .and_then(|package| {
                package
                    .lines()
                    .find_map(|line| line.strip_prefix("version = \""))
                    .map(|rest| rest.trim_end_matches('"').to_owned())
            })
            .expect("llama-cpp-2 is in Cargo.lock");
        assert_eq!(ENGINE_VERSION, format!("llama-cpp-2 {version}"));
    }

    #[test]
    fn old_states_are_removed_and_the_current_one_kept() {
        let directory =
            std::env::temp_dir().join(format!("rmac-prefix-state-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let keep = file(&directory, Tier::Tiny.model(), PromptStyle::List);
        let old = directory.join("prefix-0123456789abcdef01234567.state");
        let partial = directory.join("prefix-0123456789abcdef01234567.state.42");
        let other = directory.join("notes.txt");
        for path in [&keep, &old, &partial, &other] {
            std::fs::write(path, b"x").unwrap();
        }
        remove_others(&directory, &keep);
        assert!(keep.is_file());
        assert!(!old.exists());
        assert!(!partial.exists());
        assert!(other.is_file());
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
