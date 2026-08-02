use std::fmt;

use crate::normalization::{
    bounded_text, canonicalize_assignments, equivalent_locale, validate_locale_syntax,
    validate_x11_keyboard,
};

pub const MAX_INSTALLED_LOCALES: usize = 4096;
pub const MAX_INSTALLED_X11_LAYOUTS: usize = 512;
pub(crate) const MAX_VALUE_BYTES: usize = 128;
pub(crate) const MAX_X11_LAYOUTS: usize = 4;
pub(crate) const MAX_ERROR_BYTES: usize = 512;

pub(crate) const LOCALE_KEYS: [&str; 14] = [
    "LANG",
    "LC_CTYPE",
    "LC_NUMERIC",
    "LC_TIME",
    "LC_COLLATE",
    "LC_MONETARY",
    "LC_MESSAGES",
    "LC_PAPER",
    "LC_NAME",
    "LC_ADDRESS",
    "LC_TELEPHONE",
    "LC_MEASUREMENT",
    "LC_IDENTIFICATION",
    "LANGUAGE",
];

pub(crate) const REGION_KEYS: [&str; 8] = [
    "LC_NUMERIC",
    "LC_TIME",
    "LC_MONETARY",
    "LC_PAPER",
    "LC_NAME",
    "LC_ADDRESS",
    "LC_TELEPHONE",
    "LC_MEASUREMENT",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchEvent {
    Changed,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HourCycle {
    TwelveHour,
    TwentyFourHour,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assignment {
    pub key: String,
    pub value: String,
}

impl Assignment {
    pub fn parse(value: &str) -> Option<Self> {
        let (key, value) = value.split_once('=')?;
        if !LOCALE_KEYS.contains(&key)
            || value.is_empty()
            || value.len() > MAX_VALUE_BYTES
            || value.chars().any(char::is_control)
            || (key == "LANGUAGE"
                && value.chars().any(|character| {
                    !(character.is_ascii_alphanumeric()
                        || matches!(character, '_' | '-' | '.' | '@' | ':'))
                }))
            || (key != "LANGUAGE" && validate_locale_syntax(value).is_err())
        {
            return None;
        }
        Some(Self {
            key: key.into(),
            value: value.into(),
        })
    }

    pub fn encoded(&self) -> String {
        format!("{}={}", self.key, self.value)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FormatPreview {
    pub date_time: String,
    pub number: String,
    pub currency: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct X11Keyboard {
    pub layout: String,
    pub model: String,
    pub variant: String,
    pub options: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub locale: Vec<Assignment>,
    pub installed_locales: Vec<String>,
    pub installed_locales_truncated: bool,
    pub format_preview: Option<FormatPreview>,
    pub format_preview_error: Option<String>,
    pub installed_x11_layouts: Vec<String>,
    pub installed_x11_layouts_truncated: bool,
    pub x11_layouts_error: Option<String>,
    pub x11_layout: String,
    pub x11_model: String,
    pub x11_variant: String,
    pub x11_options: String,
    pub console_keymap: String,
}

impl Snapshot {
    pub fn language(&self) -> &str {
        self.locale
            .iter()
            .find(|assignment| assignment.key == "LANG")
            .map(|assignment| assignment.value.as_str())
            .unwrap_or("C")
    }

    pub fn encoded_locale(&self) -> Vec<String> {
        self.locale.iter().map(Assignment::encoded).collect()
    }

    pub fn preview_language(&self, language: &str) -> Result<Vec<Assignment>, Error> {
        self.validate_installed(language)?;
        let mut locale = self.locale.clone();
        if let Some(current) = locale
            .iter_mut()
            .find(|assignment| assignment.key == "LANG")
        {
            current.value = language.into();
        } else {
            locale.insert(
                0,
                Assignment {
                    key: "LANG".into(),
                    value: language.into(),
                },
            );
        }
        Ok(canonicalize_assignments(locale))
    }

    pub fn preview_region(&self, region: &str) -> Result<Vec<Assignment>, Error> {
        self.validate_installed(region)?;
        let mut locale = self.locale.clone();
        for key in REGION_KEYS {
            if let Some(current) = locale.iter_mut().find(|assignment| assignment.key == key) {
                current.value = region.into();
            } else {
                locale.push(Assignment {
                    key: key.into(),
                    value: region.into(),
                });
            }
        }
        Ok(canonicalize_assignments(locale))
    }

    pub fn effective_format_locale(&self, key: &str) -> &str {
        self.locale
            .iter()
            .find(|assignment| assignment.key == key)
            .map(|assignment| assignment.value.as_str())
            .unwrap_or_else(|| self.language())
    }

    pub fn region_locale(&self) -> &str {
        self.effective_format_locale("LC_TIME")
    }

    pub fn formats_are_mixed(&self) -> bool {
        let first = self.effective_format_locale(REGION_KEYS[0]);
        REGION_KEYS[1..]
            .iter()
            .any(|key| self.effective_format_locale(key) != first)
    }

    pub fn validate_installed(&self, locale: &str) -> Result<(), Error> {
        validate_locale_syntax(locale)?;
        if self
            .installed_locales
            .iter()
            .any(|installed| equivalent_locale(installed, locale))
        {
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::InvalidLocale,
                "the locale is not installed on this system",
            ))
        }
    }

    pub fn x11_keyboard(&self) -> X11Keyboard {
        X11Keyboard {
            layout: self.x11_layout.clone(),
            model: self.x11_model.clone(),
            variant: self.x11_variant.clone(),
            options: self.x11_options.clone(),
        }
    }

    pub fn preview_x11_keyboard(
        &self,
        layout: &str,
        variant: &str,
        options: &str,
    ) -> Result<X11Keyboard, Error> {
        validate_x11_keyboard(layout, variant, options)?;
        for candidate in layout.split(',') {
            if !self
                .installed_x11_layouts
                .iter()
                .any(|installed| installed == candidate)
            {
                return Err(Error::new(
                    ErrorKind::InvalidKeyboard,
                    format!("the XKB layout {candidate} is not installed"),
                ));
            }
        }
        Ok(X11Keyboard {
            layout: layout.into(),
            model: self.x11_model.clone(),
            variant: variant.into(),
            options: options.into(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    InvalidLocale,
    InvalidKeyboard,
    Unavailable,
    Authorization,
    Conflict,
    Mutation,
    Mismatch,
    Protocol,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    detail: String,
}

impl Error {
    pub fn new(kind: ErrorKind, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self {
            kind,
            detail: bounded_text(&detail),
        }
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for Error {}

pub trait Service {
    fn snapshot(&self) -> Result<Snapshot, Error>;
    fn set_locale(&self, assignments: &[String]) -> Result<Snapshot, Error>;
    fn set_x11_keyboard(&self, keyboard: &X11Keyboard) -> Result<Snapshot, Error>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocaleRollback {
    previous: Vec<Assignment>,
    expected: Vec<Assignment>,
}

impl LocaleRollback {
    pub fn new(previous: &Snapshot, expected: &Snapshot) -> Self {
        Self {
            previous: previous.locale.clone(),
            expected: expected.locale.clone(),
        }
    }

    pub fn previous(&self) -> &[Assignment] {
        &self.previous
    }

    pub fn expected(&self) -> &[Assignment] {
        &self.expected
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyboardRollback {
    previous: X11Keyboard,
    expected: X11Keyboard,
}

impl KeyboardRollback {
    pub fn new(previous: &Snapshot, expected: &Snapshot) -> Self {
        Self {
            previous: previous.x11_keyboard(),
            expected: expected.x11_keyboard(),
        }
    }

    pub fn previous(&self) -> &X11Keyboard {
        &self.previous
    }

    pub fn expected(&self) -> &X11Keyboard {
        &self.expected
    }
}
