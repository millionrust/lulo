//! Appearance choice, validation, announcement, and text-budget helpers.

use super::model::*;

pub(super) fn scheme_group(
    selected: rmac_theme::SchemePreference,
    enabled: bool,
    busy: bool,
) -> AccessibleChoiceGroup {
    let choices = [
        ("theme-light", "Light", rmac_theme::SchemePreference::Light),
        ("theme-dark", "Dark", rmac_theme::SchemePreference::Dark),
        (
            "theme-auto",
            "Automatic",
            rmac_theme::SchemePreference::Automatic,
        ),
    ]
    .into_iter()
    .map(|(id, name, value)| AccessibleAction {
        id: id.into(),
        name,
        kind: AppearanceAction::SetScheme(value),
        enabled,
        selected: value == selected,
    })
    .collect();
    AccessibleChoiceGroup {
        id: "theme-scheme",
        name: "Appearance",
        kind: ChoiceGroupKind::Scheme,
        value_text: scheme_name(selected),
        busy,
        choices,
    }
}

pub(super) fn accent_group(
    selected: rmac_theme::AccentPreference,
    enabled: bool,
    busy: bool,
) -> Result<AccessibleChoiceGroup, AccessibilityProjectionError> {
    let selected_choice = accent_choice(selected)?;
    let choices = AccentChoice::ALL
        .into_iter()
        .enumerate()
        .map(|(index, choice)| AccessibleAction {
            id: if index == 0 {
                "theme-accent-auto".into()
            } else {
                format!("theme-accent-{}", index - 1)
            },
            name: choice.name(),
            kind: AppearanceAction::SetAccent(choice),
            enabled,
            selected: selected_choice == Some(choice),
        })
        .collect();
    Ok(AccessibleChoiceGroup {
        id: "theme-accent",
        name: "Accent color",
        kind: ChoiceGroupKind::Accent,
        value_text: selected_choice.map_or("Custom", AccentChoice::name),
        busy,
        choices,
    })
}

pub(super) fn contrast_group(
    selected: rmac_theme::ContrastPreference,
    enabled: bool,
    busy: bool,
) -> AccessibleChoiceGroup {
    let choices = [
        ("Automatic", rmac_theme::ContrastPreference::Automatic),
        ("Normal", rmac_theme::ContrastPreference::Normal),
        ("Higher", rmac_theme::ContrastPreference::Higher),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (name, value))| AccessibleAction {
        id: format!("theme-contrast-{index}"),
        name,
        kind: AppearanceAction::SetContrast(value),
        enabled,
        selected: value == selected,
    })
    .collect();
    AccessibleChoiceGroup {
        id: "theme-contrast",
        name: "Contrast",
        kind: ChoiceGroupKind::Contrast,
        value_text: contrast_name(selected),
        busy,
        choices,
    }
}

pub(super) fn motion_group(
    selected: rmac_theme::MotionPreferenceSetting,
    enabled: bool,
    busy: bool,
) -> AccessibleChoiceGroup {
    let choices = [
        ("Automatic", rmac_theme::MotionPreferenceSetting::Automatic),
        ("Full", rmac_theme::MotionPreferenceSetting::Full),
        ("Reduced", rmac_theme::MotionPreferenceSetting::Reduced),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (name, value))| AccessibleAction {
        id: format!("theme-motion-{index}"),
        name,
        kind: AppearanceAction::SetMotion(value),
        enabled,
        selected: value == selected,
    })
    .collect();
    AccessibleChoiceGroup {
        id: "theme-motion",
        name: "Motion",
        kind: ChoiceGroupKind::Motion,
        value_text: motion_name(selected),
        busy,
        choices,
    }
}

pub(super) fn validate_groups(
    groups: &[AccessibleChoiceGroup],
) -> Result<(), AccessibilityProjectionError> {
    for group in groups {
        let selected = group
            .choices
            .iter()
            .filter(|choice| choice.selected)
            .count();
        if selected > 1 || (group.kind != ChoiceGroupKind::Accent && selected != 1) {
            return Err(AccessibilityProjectionError::InvalidSelection);
        }
    }
    Ok(())
}

pub(super) fn accent_choice(
    preference: rmac_theme::AccentPreference,
) -> Result<Option<AccentChoice>, AccessibilityProjectionError> {
    match preference {
        rmac_theme::AccentPreference::Automatic => Ok(Some(AccentChoice::Automatic)),
        rmac_theme::AccentPreference::Custom(components) => {
            validate_components(components)?;
            Ok(AccentChoice::ALL.into_iter().skip(1).find(|choice| {
                choice
                    .hex()
                    .is_some_and(|hex| components == components_from_hex(hex))
            }))
        }
    }
}

pub(super) fn validate_accent(
    preference: rmac_theme::AccentPreference,
) -> Result<(), AccessibilityProjectionError> {
    if let rmac_theme::AccentPreference::Custom(components) = preference {
        validate_components(components)?;
    }
    Ok(())
}

pub(super) fn validate_components(
    components: [f64; 3],
) -> Result<(), AccessibilityProjectionError> {
    if components
        .into_iter()
        .all(|component| component.is_finite() && (0.0..=1.0).contains(&component))
    {
        Ok(())
    } else {
        Err(AccessibilityProjectionError::InvalidAccent)
    }
}

pub(super) fn components_from_hex(hex: u32) -> [f64; 3] {
    [
        f64::from((hex >> 16) & 0xff) / 255.0,
        f64::from((hex >> 8) & 0xff) / 255.0,
        f64::from(hex & 0xff) / 255.0,
    ]
}

const fn scheme_name(value: rmac_theme::SchemePreference) -> &'static str {
    match value {
        rmac_theme::SchemePreference::Automatic => "Automatic",
        rmac_theme::SchemePreference::Light => "Light",
        rmac_theme::SchemePreference::Dark => "Dark",
    }
}

const fn contrast_name(value: rmac_theme::ContrastPreference) -> &'static str {
    match value {
        rmac_theme::ContrastPreference::Automatic => "Automatic",
        rmac_theme::ContrastPreference::Normal => "Normal",
        rmac_theme::ContrastPreference::Higher => "Higher",
    }
}

const fn motion_name(value: rmac_theme::MotionPreferenceSetting) -> &'static str {
    match value {
        rmac_theme::MotionPreferenceSetting::Automatic => "Automatic",
        rmac_theme::MotionPreferenceSetting::Full => "Full",
        rmac_theme::MotionPreferenceSetting::Reduced => "Reduced",
    }
}

pub(super) fn push_announcement(
    announcements: &mut Vec<LiveAnnouncement>,
    id: &'static str,
    text: &str,
    politeness: LivePoliteness,
    budget: &mut TextBudget,
) -> Result<(), AccessibilityProjectionError> {
    budget.add_required(id, false)?;
    budget.add_required(text, true)?;
    announcements.push(LiveAnnouncement {
        id,
        text: text.into(),
        politeness,
    });
    Ok(())
}

#[derive(Default)]
pub(super) struct TextBudget {
    bytes: usize,
}

impl TextBudget {
    pub(super) fn add_required(
        &mut self,
        value: &str,
        multiline: bool,
    ) -> Result<(), AccessibilityProjectionError> {
        if value.trim().is_empty() {
            return Err(AccessibilityProjectionError::InvalidText);
        }
        if value.len() > MAX_TEXT_VALUE_BYTES {
            return Err(AccessibilityProjectionError::TextValueLimit);
        }
        if value.chars().any(|character| {
            character.is_control() && !(multiline && matches!(character, '\n' | '\t'))
        }) {
            return Err(AccessibilityProjectionError::InvalidText);
        }
        self.bytes = self.bytes.saturating_add(value.len());
        if self.bytes > MAX_TEXT_BYTES {
            return Err(AccessibilityProjectionError::TextLimit);
        }
        Ok(())
    }
}
