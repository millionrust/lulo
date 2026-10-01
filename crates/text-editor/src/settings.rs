//! Defaults for newly opened Text Editor windows. Disk access happens before
//! GPUI starts, or on one event-driven writer thread after an edit.

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{mpsc, Mutex, OnceLock},
};

use crate::document::TextEncoding;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Settings {
    pub(crate) width_chars: u16,
    pub(crate) height_lines: u16,
    pub(crate) font_size: u8,
    pub(crate) wrap_to_page: bool,
    pub(crate) default_encoding: TextEncoding,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            width_chars: 96,
            height_lines: 30,
            font_size: 11,
            wrap_to_page: false,
            default_encoding: TextEncoding::Utf8,
        }
    }
}

fn settings_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("rmac/text-editor-settings.conf"))
}

fn parse(contents: &str) -> Settings {
    let mut settings = Settings::default();
    for line in contents.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "width_chars" => {
                if let Ok(width) = value.parse::<u16>() {
                    settings.width_chars = width.clamp(40, 240);
                }
            }
            "height_lines" => {
                if let Ok(height) = value.parse::<u16>() {
                    settings.height_lines = height.clamp(10, 100);
                }
            }
            "font_size" => {
                if let Ok(size) = value.parse::<u8>() {
                    settings.font_size = size.clamp(8, 32);
                }
            }
            "wrap_to_page" => settings.wrap_to_page = value == "true",
            "default_encoding" => {
                settings.default_encoding = match value {
                    "utf8-bom" => TextEncoding::Utf8Bom,
                    "utf16-le" => TextEncoding::Utf16Le,
                    "utf16-be" => TextEncoding::Utf16Be,
                    _ => TextEncoding::Utf8,
                };
            }
            _ => {}
        }
    }
    settings
}

fn serialize(settings: Settings) -> String {
    let encoding = match settings.default_encoding {
        TextEncoding::Utf8 => "utf8",
        TextEncoding::Utf8Bom => "utf8-bom",
        TextEncoding::Utf16Le => "utf16-le",
        TextEncoding::Utf16Be => "utf16-be",
    };
    format!(
        "version=1\nwidth_chars={}\nheight_lines={}\nfont_size={}\nwrap_to_page={}\ndefault_encoding={encoding}\n",
        settings.width_chars,
        settings.height_lines,
        settings.font_size,
        settings.wrap_to_page,
    )
}

fn save(path: &Path, settings: Settings) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    rmac_storage::atomic_write_private(path, serialize(settings).as_bytes())
}

struct Store {
    current: Mutex<Settings>,
    writer: mpsc::Sender<Settings>,
}

static STORE: OnceLock<Store> = OnceLock::new();

pub(crate) fn initialize() {
    STORE.get_or_init(|| {
        let path = settings_path();
        let initial = path
            .as_ref()
            .and_then(|path| fs::read_to_string(path).ok())
            .map(|contents| parse(&contents))
            .unwrap_or_default();
        let (writer, receiver) = mpsc::channel::<Settings>();
        std::thread::Builder::new()
            .name("text-editor-settings-writer".into())
            .spawn(move || {
                while let Ok(mut next) = receiver.recv() {
                    while let Ok(newer) = receiver.try_recv() {
                        next = newer;
                    }
                    if let Some(path) = &path {
                        if let Err(error) = save(path, next) {
                            eprintln!("Text Editor could not save Settings: {error}");
                        }
                    }
                }
            })
            .expect("could not start Text Editor settings writer");
        Store {
            current: Mutex::new(initial),
            writer,
        }
    });
}

pub(crate) fn current() -> Settings {
    initialize();
    *STORE
        .get()
        .expect("settings initialized")
        .current
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

pub(crate) fn update(edit: impl FnOnce(&mut Settings)) -> Settings {
    initialize();
    let store = STORE.get().expect("settings initialized");
    let next = {
        let mut current = store
            .current
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        edit(&mut current);
        *current
    };
    let _ = store.writer.send(next);
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_bounds() {
        let settings = Settings {
            width_chars: 120,
            height_lines: 42,
            font_size: 14,
            wrap_to_page: true,
            default_encoding: TextEncoding::Utf16Le,
        };
        assert_eq!(parse(&serialize(settings)), settings);
        let bounded = parse("width_chars=999\nheight_lines=1\nfont_size=255\n");
        assert_eq!(bounded.width_chars, 240);
        assert_eq!(bounded.height_lines, 10);
        assert_eq!(bounded.font_size, 32);
    }
}
