use crate::model::{LOCALE_KEYS, MAX_ERROR_BYTES, MAX_VALUE_BYTES, MAX_X11_LAYOUTS};
use crate::{Assignment, Error, ErrorKind, MAX_INSTALLED_LOCALES, MAX_INSTALLED_X11_LAYOUTS};

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
    Ok(canonicalize_assignments(assignments))
}

pub fn canonicalize_assignments(mut assignments: Vec<Assignment>) -> Vec<Assignment> {
    let language = assignments
        .iter()
        .find(|assignment| assignment.key == "LANG")
        .map(|assignment| assignment.value.clone());
    assignments.retain(|assignment| {
        assignment.key == "LANG"
            || language
                .as_ref()
                .is_none_or(|language| assignment.value != *language)
    });
    assignments
}

pub fn locale_assignments_match(left: &[Assignment], right: &[Assignment]) -> bool {
    fn sorted(assignments: &[Assignment]) -> Vec<(&str, &str)> {
        let mut values = assignments
            .iter()
            .map(|assignment| (assignment.key.as_str(), assignment.value.as_str()))
            .collect::<Vec<_>>();
        values.sort_unstable();
        values
    }
    sorted(left) == sorted(right)
}

pub fn complete_locale_request(current: &[Assignment], desired: &[Assignment]) -> Vec<String> {
    LOCALE_KEYS
        .iter()
        .filter_map(|key| {
            desired
                .iter()
                .find(|assignment| assignment.key == *key)
                .map(Assignment::encoded)
                .or_else(|| {
                    current
                        .iter()
                        .any(|assignment| assignment.key == *key)
                        .then(|| format!("{key}="))
                })
        })
        .collect()
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

pub(crate) fn equivalent_locale(left: &str, right: &str) -> bool {
    canonical_locale(left) == canonical_locale(right)
}

fn canonical_locale(locale: &str) -> String {
    locale
        .to_ascii_lowercase()
        .replace("utf-8", "utf8")
        .replace('-', "")
}

pub(crate) fn bounded_text(value: &str) -> String {
    let normalized = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let mut end = normalized.len().min(MAX_ERROR_BYTES);
    while !normalized.is_char_boundary(end) {
        end -= 1;
    }
    normalized[..end].trim().to_string()
}
