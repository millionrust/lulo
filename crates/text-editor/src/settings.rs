//! Defaults for newly opened Text Editor windows. Disk access happens before
//! GPUI starts, or on one event-driven writer thread after an edit.

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{mpsc, Mutex, OnceLock},
};

use crate::document::TextEncoding;

/// Format ▸ Font ▸ Show Fonts has no counterpart here (TE-03/TE-14): Lulo's
/// rich-text "font" is still a whole-document choice, so a new rich-text
/// window's default font is one of these two known-installed families
/// (`rmac_ui::UI_FONT`/`MONO_FONT`), not an arbitrary system font.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum RichTextFont {
    #[default]
    Inter,
    JetBrainsMono,
}

impl RichTextFont {
    pub(crate) fn family(self) -> &'static str {
        match self {
            Self::Inter => rmac_ui::UI_FONT,
            Self::JetBrainsMono => rmac_ui::MONO_FONT,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        self.family()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Settings {
    pub(crate) width_chars: u16,
    pub(crate) height_lines: u16,
    pub(crate) font_size: u8,
    pub(crate) wrap_to_page: bool,
    pub(crate) default_encoding: TextEncoding,
    /// Settings ▸ New Document ▸ Format: the default a new document opens
    /// in, `Make Rich Text`'s own default (TXT-SETTINGS-013).
    pub(crate) rich_text_default: bool,
    /// Settings ▸ New Document ▸ Font ▸ Rich text font (TXT-SETTINGS-002/010/014).
    pub(crate) rich_text_font: RichTextFont,
    pub(crate) rich_text_font_size: u8,
    /// Settings ▸ New Document ▸ Options ▸ Show ruler (TXT-SETTINGS-015):
    /// whether a new rich-text window starts with its ruler shown.
    pub(crate) show_ruler_default: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            width_chars: 96,
            height_lines: 30,
            font_size: 11,
            wrap_to_page: false,
            default_encoding: TextEncoding::Utf8,
            // TextEdit's own default: new documents are rich text (TE-23).
            rich_text_default: true,
            rich_text_font: RichTextFont::default(),
            rich_text_font_size: 12,
            show_ruler_default: true,
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
            "rich_text_default" => settings.rich_text_default = value == "true",
            "rich_text_font" => {
                settings.rich_text_font = match value {
                    "jetbrains-mono" => RichTextFont::JetBrainsMono,
                    _ => RichTextFont::Inter,
                };
            }
            "rich_text_font_size" => {
                if let Ok(size) = value.parse::<u8>() {
                    settings.rich_text_font_size = size.clamp(8, 32);
                }
            }
            "show_ruler_default" => settings.show_ruler_default = value == "true",
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
    let rich_text_font = match settings.rich_text_font {
        RichTextFont::Inter => "inter",
        RichTextFont::JetBrainsMono => "jetbrains-mono",
    };
    format!(
        "version=1\nwidth_chars={}\nheight_lines={}\nfont_size={}\nwrap_to_page={}\ndefault_encoding={encoding}\nrich_text_default={}\nrich_text_font={rich_text_font}\nrich_text_font_size={}\nshow_ruler_default={}\n",
        settings.width_chars,
        settings.height_lines,
        settings.font_size,
        settings.wrap_to_page,
        settings.rich_text_default,
        settings.rich_text_font_size,
        settings.show_ruler_default,
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
            rich_text_default: true,
            rich_text_font: RichTextFont::JetBrainsMono,
            rich_text_font_size: 18,
            show_ruler_default: false,
        };
        assert_eq!(parse(&serialize(settings)), settings);
        let bounded =
            parse("width_chars=999\nheight_lines=1\nfont_size=255\nrich_text_font_size=255\n");
        assert_eq!(bounded.width_chars, 240);
        assert_eq!(bounded.height_lines, 10);
        assert_eq!(bounded.font_size, 32);
        assert_eq!(bounded.rich_text_font_size, 32);
    }
}
