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

#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// The nine checkboxes under Settings' own "New Document" tab
    /// (TXT-SETTINGS-003/004/006/007/012..015/017): the state every new
    /// document window's Edit ▸ Spelling and Grammar / Substitutions
    /// toggles start from, rather than always the same hard-coded Mac
    /// default regardless of what the owner last set.
    pub(crate) check_spelling_while_typing_default: bool,
    pub(crate) check_grammar_with_spelling_default: bool,
    pub(crate) correct_spelling_automatically_default: bool,
    pub(crate) smart_copy_paste_default: bool,
    pub(crate) smart_quotes_default: bool,
    pub(crate) smart_dashes_default: bool,
    pub(crate) smart_links_default: bool,
    pub(crate) data_detectors_default: bool,
    pub(crate) text_replacement_default: bool,
    /// Settings ▸ New Document ▸ Properties (TXT-SETTINGS-001/003/006):
    /// the Author/Organisation/Copyright a freshly created rich document's
    /// Document Properties sheet (`view/format_extras.rs`, TE-10) starts
    /// with, instead of always blank. Like the Mac, these are plain text
    /// defaults, not applied to a document already open.
    pub(crate) author_default: String,
    pub(crate) organisation_default: String,
    pub(crate) copyright_default: String,
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
            // Mirrors `rmac_ui::text_assist::TextAssistSettings::default()`
            // and the view's own `data_detectors` default, so a document
            // window opened before anyone touches Settings behaves exactly
            // as it always has.
            check_spelling_while_typing_default: true,
            check_grammar_with_spelling_default: false,
            correct_spelling_automatically_default: false,
            smart_copy_paste_default: true,
            smart_quotes_default: true,
            smart_dashes_default: true,
            smart_links_default: true,
            data_detectors_default: true,
            text_replacement_default: true,
            author_default: String::new(),
            organisation_default: String::new(),
            copyright_default: String::new(),
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
            "check_spelling_while_typing_default" => {
                settings.check_spelling_while_typing_default = value == "true";
            }
            "check_grammar_with_spelling_default" => {
                settings.check_grammar_with_spelling_default = value == "true";
            }
            "correct_spelling_automatically_default" => {
                settings.correct_spelling_automatically_default = value == "true";
            }
            "smart_copy_paste_default" => settings.smart_copy_paste_default = value == "true",
            "smart_quotes_default" => settings.smart_quotes_default = value == "true",
            "smart_dashes_default" => settings.smart_dashes_default = value == "true",
            "smart_links_default" => settings.smart_links_default = value == "true",
            "data_detectors_default" => settings.data_detectors_default = value == "true",
            "text_replacement_default" => settings.text_replacement_default = value == "true",
            "author_default" => settings.author_default = value.to_string(),
            "organisation_default" => settings.organisation_default = value.to_string(),
            "copyright_default" => settings.copyright_default = value.to_string(),
            _ => {}
        }
    }
    settings
}

/// A Properties default for the hand-rolled `key=value` line format: one
/// line per setting, so an embedded newline would corrupt the next key —
/// collapsed to a space, like a single-line AppKit text field would show.
fn one_line(value: &str) -> String {
    value.replace(['\n', '\r'], " ")
}

fn serialize(settings: &Settings) -> String {
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
        "version=1\nwidth_chars={}\nheight_lines={}\nfont_size={}\nwrap_to_page={}\ndefault_encoding={encoding}\nrich_text_default={}\nrich_text_font={rich_text_font}\nrich_text_font_size={}\nshow_ruler_default={}\ncheck_spelling_while_typing_default={}\ncheck_grammar_with_spelling_default={}\ncorrect_spelling_automatically_default={}\nsmart_copy_paste_default={}\nsmart_quotes_default={}\nsmart_dashes_default={}\nsmart_links_default={}\ndata_detectors_default={}\ntext_replacement_default={}\nauthor_default={}\norganisation_default={}\ncopyright_default={}\n",
        settings.width_chars,
        settings.height_lines,
        settings.font_size,
        settings.wrap_to_page,
        settings.rich_text_default,
        settings.rich_text_font_size,
        settings.show_ruler_default,
        settings.check_spelling_while_typing_default,
        settings.check_grammar_with_spelling_default,
        settings.correct_spelling_automatically_default,
        settings.smart_copy_paste_default,
        settings.smart_quotes_default,
        settings.smart_dashes_default,
        settings.smart_links_default,
        settings.data_detectors_default,
        settings.text_replacement_default,
        one_line(&settings.author_default),
        one_line(&settings.organisation_default),
        one_line(&settings.copyright_default),
    )
}

fn save(path: &Path, settings: &Settings) -> io::Result<()> {
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
                        if let Err(error) = save(path, &next) {
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
    STORE
        .get()
        .expect("settings initialized")
        .current
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone()
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
        current.clone()
    };
    let _ = store.writer.send(next.clone());
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
            check_spelling_while_typing_default: false,
            check_grammar_with_spelling_default: true,
            correct_spelling_automatically_default: true,
            smart_copy_paste_default: false,
            smart_quotes_default: false,
            smart_dashes_default: false,
            smart_links_default: false,
            data_detectors_default: false,
            text_replacement_default: false,
            author_default: "A. Writer".to_string(),
            organisation_default: "Acme".to_string(),
            copyright_default: "\u{a9} 2026 Acme".to_string(),
        };
        assert_eq!(parse(&serialize(&settings)), settings);
        let bounded =
            parse("width_chars=999\nheight_lines=1\nfont_size=255\nrich_text_font_size=255\n");
        assert_eq!(bounded.width_chars, 240);
        assert_eq!(bounded.height_lines, 10);
        assert_eq!(bounded.font_size, 32);
        assert_eq!(bounded.rich_text_font_size, 32);
    }

    #[test]
    fn spelling_and_substitution_defaults_match_the_shared_mac_defaults() {
        // TXT-SETTINGS-003/004/006/007/012..015/017: a fresh install (no
        // settings file yet) seeds every new document window exactly as it
        // always has — `rmac_ui::text_assist::TextAssistSettings::default()`
        // and the view's own `data_detectors: true`.
        let settings = Settings::default();
        assert!(settings.check_spelling_while_typing_default);
        assert!(!settings.check_grammar_with_spelling_default);
        assert!(!settings.correct_spelling_automatically_default);
        assert!(settings.smart_copy_paste_default);
        assert!(settings.smart_quotes_default);
        assert!(settings.smart_dashes_default);
        assert!(settings.smart_links_default);
        assert!(settings.data_detectors_default);
        assert!(settings.text_replacement_default);
    }

    #[test]
    fn properties_defaults_start_blank() {
        // TXT-SETTINGS-001/003/006: a fresh install's Document Properties
        // defaults are empty, like the Mac's own, not some placeholder.
        let settings = Settings::default();
        assert_eq!(settings.author_default, "");
        assert_eq!(settings.organisation_default, "");
        assert_eq!(settings.copyright_default, "");
    }

    #[test]
    fn properties_defaults_round_trip_and_collapse_newlines() {
        let mut settings = Settings::default();
        settings.author_default = "A. Writer".to_string();
        settings.organisation_default = "Acme, Inc.".to_string();
        settings.copyright_default = "line one\nline two".to_string();
        let serialized = serialize(&settings);
        let parsed = parse(&serialized);
        assert_eq!(parsed.author_default, "A. Writer");
        assert_eq!(parsed.organisation_default, "Acme, Inc.");
        // A newline would corrupt the next `key=value` line, so it is
        // collapsed to a space rather than written raw.
        assert_eq!(parsed.copyright_default, "line one line two");
    }
}
