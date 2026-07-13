//! Platform-neutral system locale and default keyboard metadata.

use std::fmt;

pub const MAX_INSTALLED_LOCALES: usize = 4096;
pub const MAX_INSTALLED_X11_LAYOUTS: usize = 512;
const MAX_VALUE_BYTES: usize = 128;
const MAX_X11_LAYOUTS: usize = 4;

const LOCALE_KEYS: [&str; 14] = [
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchEvent {
    Changed,
    Unavailable,
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
        Ok(locale)
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
    Mutation,
    Protocol,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    detail: String,
}

impl Error {
    pub fn new(kind: ErrorKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
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

pub fn normalize_assignments(values: Vec<String>) -> Result<Vec<Assignment>, Error> {
    let mut assignments: Vec<Assignment> = Vec::new();
    for value in values {
        let assignment = Assignment::parse(&value).ok_or_else(|| {
            Error::new(
                ErrorKind::Protocol,
                "the locale service returned an invalid assignment",
            )
        })?;
        if let Some(existing) = assignments
            .iter_mut()
            .find(|existing| existing.key == assignment.key)
        {
            *existing = assignment;
        } else {
            assignments.push(assignment);
        }
    }
    Ok(assignments)
}

pub fn normalize_installed_locales(values: Vec<String>) -> (Vec<String>, bool) {
    let mut locales = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| validate_locale_syntax(value).is_ok())
        .collect::<Vec<_>>();
    locales.sort_unstable();
    locales.dedup();
    let truncated = locales.len() > MAX_INSTALLED_LOCALES;
    locales.truncate(MAX_INSTALLED_LOCALES);
    (locales, truncated)
}

pub fn normalize_installed_x11_layouts(values: Vec<String>) -> (Vec<String>, bool) {
    let mut layouts = values
        .into_iter()
        .map(|value| value.trim().to_owned())
        .filter(|value| valid_xkb_name(value))
        .collect::<Vec<_>>();
    layouts.sort_unstable();
    layouts.dedup();
    let truncated = layouts.len() > MAX_INSTALLED_X11_LAYOUTS;
    layouts.truncate(MAX_INSTALLED_X11_LAYOUTS);
    (layouts, truncated)
}

pub fn validate_x11_keyboard(layout: &str, variant: &str, options: &str) -> Result<(), Error> {
    let layouts = layout.split(',').collect::<Vec<_>>();
    if layouts.is_empty()
        || layouts.len() > MAX_X11_LAYOUTS
        || layouts.iter().any(|value| !valid_xkb_name(value))
    {
        return Err(Error::new(
            ErrorKind::InvalidKeyboard,
            "enter one to four comma-separated XKB layouts such as us,de",
        ));
    }
    let variants = if variant.is_empty() {
        Vec::new()
    } else {
        variant.split(',').collect::<Vec<_>>()
    };
    if variants.len() > layouts.len()
        || variants
            .iter()
            .any(|value| !value.is_empty() && !valid_xkb_name(value))
        || options.len() > MAX_VALUE_BYTES
        || options.chars().any(|character| {
            !(character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | ':' | ','))
        })
    {
        return Err(Error::new(
            ErrorKind::InvalidKeyboard,
            "the XKB variant or options are invalid for this layout list",
        ));
    }
    if layouts.len() > 1 && !options.split(',').any(|option| option.starts_with("grp:")) {
        return Err(Error::new(
            ErrorKind::InvalidKeyboard,
            "multiple layouts require an XKB grp: switching option",
        ));
    }
    Ok(())
}

fn valid_xkb_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_VALUE_BYTES
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

pub fn validate_locale_syntax(locale: &str) -> Result<(), Error> {
    if locale.is_empty()
        || locale.len() > MAX_VALUE_BYTES
        || locale.chars().any(|character| {
            !(character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | '@'))
        })
    {
        return Err(Error::new(
            ErrorKind::InvalidLocale,
            "enter an installed locale such as en_US.UTF-8",
        ));
    }
    Ok(())
}

fn equivalent_locale(left: &str, right: &str) -> bool {
    canonical_locale(left) == canonical_locale(right)
}

fn canonical_locale(locale: &str) -> String {
    locale
        .to_ascii_lowercase()
        .replace("utf-8", "utf8")
        .replace('-', "")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct FakeService {
        snapshot: RefCell<Snapshot>,
        mutation_error: Option<Error>,
    }

    impl Service for FakeService {
        fn snapshot(&self) -> Result<Snapshot, Error> {
            Ok(self.snapshot.borrow().clone())
        }

        fn set_locale(&self, assignments: &[String]) -> Result<Snapshot, Error> {
            if let Some(error) = &self.mutation_error {
                return Err(error.clone());
            }
            let mut next = self.snapshot.borrow().clone();
            next.locale = normalize_assignments(assignments.to_vec())?;
            *self.snapshot.borrow_mut() = next.clone();
            Ok(next)
        }

        fn set_x11_keyboard(&self, keyboard: &X11Keyboard) -> Result<Snapshot, Error> {
            if let Some(error) = &self.mutation_error {
                return Err(error.clone());
            }
            let mut next = self.snapshot.borrow().clone();
            next.x11_layout = keyboard.layout.clone();
            next.x11_model = keyboard.model.clone();
            next.x11_variant = keyboard.variant.clone();
            next.x11_options = keyboard.options.clone();
            *self.snapshot.borrow_mut() = next.clone();
            Ok(next)
        }
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            locale: normalize_assignments(vec![
                "LANG=en_GB.UTF-8".into(),
                "LC_TIME=en_DK.UTF-8".into(),
            ])
            .unwrap(),
            installed_locales: vec!["C".into(), "en_GB.utf8".into(), "fr_FR.utf8".into()],
            installed_x11_layouts: vec!["de".into(), "us".into()],
            ..Snapshot::default()
        }
    }

    #[test]
    fn language_preview_preserves_format_overrides() {
        let preview = snapshot().preview_language("fr_FR.UTF-8").unwrap();
        assert_eq!(preview[0].encoded(), "LANG=fr_FR.UTF-8");
        assert!(preview
            .iter()
            .any(|assignment| assignment.encoded() == "LC_TIME=en_DK.UTF-8"));
    }

    #[test]
    fn installed_validation_accepts_utf8_alias_spelling() {
        let snapshot = snapshot();
        assert!(snapshot.validate_installed("en_GB.UTF-8").is_ok());
        assert_eq!(
            snapshot
                .validate_installed("xx_YY.UTF-8")
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidLocale
        );
        assert!(validate_locale_syntax("../../etc/passwd").is_err());
    }

    #[test]
    fn assignment_decoder_rejects_unknown_and_malformed_keys() {
        assert!(normalize_assignments(vec!["LANG=en_US.UTF-8".into()]).is_ok());
        assert!(normalize_assignments(vec!["LC_ALL=en_US.UTF-8".into()]).is_err());
        assert!(normalize_assignments(vec!["PATH=/tmp".into()]).is_err());
    }

    #[test]
    fn installed_inventory_is_sorted_deduplicated_and_bounded() {
        let mut values = (0..=MAX_INSTALLED_LOCALES)
            .map(|index| format!("x_{index}.UTF-8"))
            .collect::<Vec<_>>();
        values.push("C".into());
        values.push("C".into());
        let (values, truncated) = normalize_installed_locales(values);
        assert_eq!(values.len(), MAX_INSTALLED_LOCALES);
        assert!(truncated);
        assert!(values.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn service_boundary_returns_authoritative_snapshot_after_mutation() {
        let service = FakeService {
            snapshot: RefCell::new(snapshot()),
            mutation_error: None,
        };
        let assignments = service
            .snapshot()
            .unwrap()
            .preview_language("fr_FR.UTF-8")
            .unwrap()
            .iter()
            .map(Assignment::encoded)
            .collect::<Vec<_>>();
        let changed = service.set_locale(&assignments).unwrap();
        assert_eq!(changed.language(), "fr_FR.UTF-8");
        assert_eq!(service.snapshot().unwrap(), changed);

        let keyboard = changed
            .preview_x11_keyboard("us,de", ",nodeadkeys", "grp:ctrl_space_toggle")
            .unwrap();
        let changed = service.set_x11_keyboard(&keyboard).unwrap();
        assert_eq!(changed.x11_layout, "us,de");
        assert_eq!(changed.x11_variant, ",nodeadkeys");
    }

    #[test]
    fn failed_service_mutation_preserves_last_authoritative_snapshot() {
        let original = snapshot();
        let service = FakeService {
            snapshot: RefCell::new(original.clone()),
            mutation_error: Some(Error::new(ErrorKind::Authorization, "cancelled")),
        };
        assert!(service.set_locale(&["LANG=fr_FR.UTF-8".into()]).is_err());
        assert_eq!(service.snapshot().unwrap(), original);
    }

    #[test]
    fn keyboard_preview_validates_installed_layouts_and_variant_count() {
        let snapshot = snapshot();
        let keyboard = snapshot
            .preview_x11_keyboard("us,de", ",nodeadkeys", "grp:ctrl_space_toggle")
            .unwrap();
        assert_eq!(keyboard.layout, "us,de");
        assert_eq!(keyboard.variant, ",nodeadkeys");
        assert_eq!(
            snapshot
                .preview_x11_keyboard("us,xx", "", "")
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidKeyboard
        );
        assert!(snapshot
            .preview_x11_keyboard("us", "basic,nodeadkeys", "")
            .is_err());
        assert!(snapshot.preview_x11_keyboard("us,de", "", "").is_err());
    }

    #[test]
    fn x11_layout_inventory_is_sorted_deduplicated_and_bounded() {
        let mut values = (0..=MAX_INSTALLED_X11_LAYOUTS)
            .map(|index| format!("layout_{index}"))
            .collect::<Vec<_>>();
        values.extend(["us".into(), "us".into(), "../../bad".into()]);
        let (values, truncated) = normalize_installed_x11_layouts(values);
        assert_eq!(values.len(), MAX_INSTALLED_X11_LAYOUTS);
        assert!(truncated);
        assert!(values.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
