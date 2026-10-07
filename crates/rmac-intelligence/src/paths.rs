//! Where Lulo Intelligence keeps its files (ADR 0024 §4).
//!
//! - settings: `$XDG_CONFIG_HOME/rmac/intelligence.json`
//! - models: `$XDG_DATA_HOME/lulo/intelligence/models/<sha256>.gguf`
//! - saved prompt state: `$XDG_CACHE_HOME/lulo/intelligence/`

use std::path::{Path, PathBuf};

use crate::manifest::Model;

fn xdg(variable: &str, fallback: &str) -> Option<PathBuf> {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(fallback))
        })
}

pub fn config_file() -> Option<PathBuf> {
    xdg("XDG_CONFIG_HOME", ".config").map(|root| root.join("rmac/intelligence.json"))
}

pub fn models_dir() -> Option<PathBuf> {
    xdg("XDG_DATA_HOME", ".local/share").map(|root| root.join("lulo/intelligence/models"))
}

pub fn cache_dir() -> Option<PathBuf> {
    xdg("XDG_CACHE_HOME", ".cache").map(|root| root.join("lulo/intelligence"))
}

/// The model file inside `models`.
pub fn model_file(models: &Path, model: &Model) -> PathBuf {
    models.join(format!("{}.gguf", model.sha256))
}

/// A download in progress, renamed onto [`model_file`] once verified.
pub fn partial_file(models: &Path, model: &Model) -> PathBuf {
    models.join(format!("{}.gguf.partial", model.sha256))
}

/// The licence text kept beside a model.
pub fn licence_file(models: &Path, model: &Model) -> PathBuf {
    models.join(format!("{}.LICENSE.txt", model.sha256))
}

/// The verified-stamp beside a model (see [`crate::verify`]).
pub fn stamp_file(models: &Path, model: &Model) -> PathBuf {
    models.join(format!("{}.verified", model.sha256))
}
