//! Bounded choice, authority, focus-order, and live-state semantics for Appearance.

use std::collections::HashSet;
use std::fmt;

pub const APPEARANCE_TITLE: &str = "Appearance";
pub const REFRESH_ID: &str = "theme-refresh";
pub const REFRESH_LABEL: &str = "Refresh";
pub const LOADING_LABEL: &str = "Loading appearance preferences…";
pub const APPLYING_LABEL: &str = "Applying appearance preferences…";
pub const UNAVAILABLE_LABEL: &str = "The rmac theme preference service is unavailable.";
pub const HOST_UNAVAILABLE_LABEL: &str = "The Linux Settings portal is unavailable. Automatic values use safe rmac defaults; explicit choices remain writable.";
pub const MAX_TEXT_VALUE_BYTES: usize = 16 * 1024;
pub const MAX_TEXT_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaneState {
    Loading,
    Unavailable,
    Ready,
    Busy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LivePoliteness {
    Polite,
    Assertive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccentChoice {
    Automatic,
    Blue,
    Purple,
    Pink,
    Red,
    Orange,
    Yellow,
    Green,
    Graphite,
}

impl AccentChoice {
    pub const ALL: [Self; 9] = [
        Self::Automatic,
        Self::Blue,
        Self::Purple,
        Self::Pink,
        Self::Red,
        Self::Orange,
        Self::Yellow,
        Self::Green,
        Self::Graphite,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Automatic => "Automatic",
            Self::Blue => "Blue",
            Self::Purple => "Purple",
            Self::Pink => "Pink",
            Self::Red => "Red",
            Self::Orange => "Orange",
            Self::Yellow => "Yellow",
            Self::Green => "Green",
            Self::Graphite => "Graphite",
        }
    }

    pub const fn hex(self) -> Option<u32> {
        match self {
            Self::Automatic => None,
            Self::Blue => Some(0x0a84ff),
            Self::Purple => Some(0xaf52de),
            Self::Pink => Some(0xff2d55),
            Self::Red => Some(0xff3b30),
            Self::Orange => Some(0xff9500),
            Self::Yellow => Some(0xffcc00),
            Self::Green => Some(0x34c759),
            Self::Graphite => Some(0x8e8e93),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppearanceAction {
    Refresh,
    SetScheme(rmac_theme::SchemePreference),
    SetAccent(AccentChoice),
    SetContrast(rmac_theme::ContrastPreference),
    SetMotion(rmac_theme::MotionPreferenceSetting),
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleAction {
    pub id: String,
    pub name: &'static str,
    pub kind: AppearanceAction,
    pub enabled: bool,
    pub selected: bool,
}

impl fmt::Debug for AccessibleAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleAction")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("enabled", &self.enabled)
            .field("selected", &self.selected)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChoiceGroupKind {
    Scheme,
    Accent,
    Contrast,
    Motion,
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleChoiceGroup {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: ChoiceGroupKind,
    pub value_text: &'static str,
    pub busy: bool,
    pub choices: Vec<AccessibleAction>,
}

impl fmt::Debug for AccessibleChoiceGroup {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleChoiceGroup")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("value_text", &self.value_text)
            .field("busy", &self.busy)
            .field("choice_count", &self.choices.len())
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccessibleAuthority {
    pub host_preference: &'static str,
    pub host_preference_exposed: bool,
    pub effective_appearance: &'static str,
}

#[derive(Clone, Eq, PartialEq)]
pub struct LiveAnnouncement {
    pub id: &'static str,
    pub text: String,
    pub politeness: LivePoliteness,
}

impl fmt::Debug for LiveAnnouncement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LiveAnnouncement")
            .field("id", &self.id)
            .field("text", &"<redacted>")
            .field("politeness", &self.politeness)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct AppearanceAccessibilitySnapshot {
    pub title: &'static str,
    pub state: PaneState,
    pub refresh_action: AccessibleAction,
    pub groups: Vec<AccessibleChoiceGroup>,
    pub authority: Option<AccessibleAuthority>,
    pub detail: Option<String>,
    pub keyboard_order: Vec<String>,
    pub initial_focus: Option<String>,
    pub announcements: Vec<LiveAnnouncement>,
}

impl fmt::Debug for AppearanceAccessibilitySnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AppearanceAccessibilitySnapshot")
            .field("state", &self.state)
            .field("refresh_action", &self.refresh_action)
            .field("group_count", &self.groups.len())
            .field("has_authority", &self.authority.is_some())
            .field("has_detail", &self.detail.is_some())
            .field("keyboard_action_count", &self.keyboard_order.len())
            .field("initial_focus", &self.initial_focus)
            .field("announcement_count", &self.announcements.len())
            .finish()
    }
}

#[derive(Clone, Copy)]
pub struct AppearanceInput<'a> {
    pub theme: Option<&'a rmac_theme::Snapshot>,
    pub host: &'a rmac_appearance::Snapshot,
    pub loading: bool,
    pub busy: bool,
    pub refreshing: bool,
    pub error: Option<&'a str>,
}

impl fmt::Debug for AppearanceInput<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AppearanceInput")
            .field("has_theme", &self.theme.is_some())
            .field("host_available", &self.host.available)
            .field("loading", &self.loading)
            .field("busy", &self.busy)
            .field("refreshing", &self.refreshing)
            .field("has_error", &self.error.is_some())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessibilityProjectionError {
    InvalidAccent,
    DuplicateAction,
    InvalidSelection,
    InvalidText,
    TextValueLimit,
    TextLimit,
}

pub fn project_appearance(
    input: AppearanceInput<'_>,
) -> Result<AppearanceAccessibilitySnapshot, AccessibilityProjectionError> {
    let state = if input.loading {
        PaneState::Loading
    } else if input.theme.is_none() {
        PaneState::Unavailable
    } else if input.busy || input.refreshing {
        PaneState::Busy
    } else {
        PaneState::Ready
    };
    let mut budget = TextBudget::default();
    for fixed in [
        APPEARANCE_TITLE,
        REFRESH_ID,
        REFRESH_LABEL,
        LOADING_LABEL,
        APPLYING_LABEL,
        UNAVAILABLE_LABEL,
        HOST_UNAVAILABLE_LABEL,
    ] {
        budget.add_required(fixed, false)?;
    }
    if let Some(error) = input.error {
        budget.add_required(error, true)?;
    }

    let refresh_enabled = !input.loading && !input.busy && !input.refreshing;
    let refresh_action = AccessibleAction {
        id: REFRESH_ID.into(),
        name: REFRESH_LABEL,
        kind: AppearanceAction::Refresh,
        enabled: refresh_enabled,
        selected: false,
    };
    let mut groups = Vec::new();
    let mut authority = None;
    let mut detail = None;
    if let Some(theme) = input.theme.filter(|_| !input.loading) {
        validate_accent(theme.preferences.accent_color)?;
        let enabled = state == PaneState::Ready;
        groups = vec![
            scheme_group(
                theme.preferences.color_scheme,
                enabled,
                state == PaneState::Busy,
            ),
            accent_group(
                theme.preferences.accent_color,
                enabled,
                state == PaneState::Busy,
            )?,
            contrast_group(
                theme.preferences.contrast,
                enabled,
                state == PaneState::Busy,
            ),
            motion_group(theme.preferences.motion, enabled, state == PaneState::Busy),
        ];
        authority = Some(AccessibleAuthority {
            host_preference: if input.host.capabilities.color_scheme {
                input.host.color_scheme.label()
            } else {
                "Not exposed"
            },
            host_preference_exposed: input.host.capabilities.color_scheme,
            effective_appearance: match theme.effective.color_scheme {
                rmac_appearance::ResolvedColorScheme::Light => "Light",
                rmac_appearance::ResolvedColorScheme::Dark => "Dark",
            },
        });
        if let Some(value) = &theme.detail {
            budget.add_required(value, true)?;
            detail = Some(value.clone());
        }
    }

    validate_groups(&groups)?;
    let mut keyboard_order = Vec::new();
    if refresh_action.enabled {
        keyboard_order.push(refresh_action.id.clone());
    }
    keyboard_order.extend(
        groups
            .iter()
            .flat_map(|group| &group.choices)
            .filter(|choice| choice.enabled)
            .map(|choice| choice.id.clone()),
    );
    let mut action_ids = HashSet::with_capacity(keyboard_order.len());
    if keyboard_order
        .iter()
        .any(|id| !action_ids.insert(id.as_str()))
    {
        return Err(AccessibilityProjectionError::DuplicateAction);
    }

    let mut announcements = Vec::new();
    match state {
        PaneState::Loading => push_announcement(
            &mut announcements,
            "appearance-loading",
            LOADING_LABEL,
            LivePoliteness::Polite,
            &mut budget,
        )?,
        PaneState::Unavailable => push_announcement(
            &mut announcements,
            "appearance-unavailable",
            UNAVAILABLE_LABEL,
            LivePoliteness::Assertive,
            &mut budget,
        )?,
        PaneState::Busy => push_announcement(
            &mut announcements,
            "appearance-busy",
            APPLYING_LABEL,
            LivePoliteness::Polite,
            &mut budget,
        )?,
        PaneState::Ready => {}
    }
    if let Some(error) = input.error {
        push_announcement(
            &mut announcements,
            "appearance-error",
            error,
            LivePoliteness::Assertive,
            &mut budget,
        )?;
    }
    if !input.host.available {
        push_announcement(
            &mut announcements,
            "appearance-host-unavailable",
            HOST_UNAVAILABLE_LABEL,
            LivePoliteness::Polite,
            &mut budget,
        )?;
    }

    Ok(AppearanceAccessibilitySnapshot {
        title: APPEARANCE_TITLE,
        state,
        refresh_action,
        groups,
        authority,
        detail,
        initial_focus: keyboard_order.first().cloned(),
        keyboard_order,
        announcements,
    })
}

fn scheme_group(
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

fn accent_group(
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

fn contrast_group(
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

fn motion_group(
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

fn validate_groups(groups: &[AccessibleChoiceGroup]) -> Result<(), AccessibilityProjectionError> {
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

fn accent_choice(
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

fn validate_accent(
    preference: rmac_theme::AccentPreference,
) -> Result<(), AccessibilityProjectionError> {
    if let rmac_theme::AccentPreference::Custom(components) = preference {
        validate_components(components)?;
    }
    Ok(())
}

fn validate_components(components: [f64; 3]) -> Result<(), AccessibilityProjectionError> {
    if components
        .into_iter()
        .all(|component| component.is_finite() && (0.0..=1.0).contains(&component))
    {
        Ok(())
    } else {
        Err(AccessibilityProjectionError::InvalidAccent)
    }
}

fn components_from_hex(hex: u32) -> [f64; 3] {
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

fn push_announcement(
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
struct TextBudget {
    bytes: usize,
}

impl TextBudget {
    fn add_required(
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn host(available: bool) -> rmac_appearance::Snapshot {
        rmac_appearance::Snapshot {
            available,
            color_scheme: rmac_appearance::ColorScheme::PreferDark,
            capabilities: rmac_appearance::Capabilities {
                color_scheme: true,
                ..rmac_appearance::Capabilities::default()
            },
            ..rmac_appearance::Snapshot::default()
        }
    }

    fn theme(accent: rmac_theme::AccentPreference) -> rmac_theme::Snapshot {
        rmac_theme::Snapshot {
            preferences: rmac_theme::Preferences {
                color_scheme: rmac_theme::SchemePreference::Dark,
                accent_color: accent,
                contrast: rmac_theme::ContrastPreference::Higher,
                motion: rmac_theme::MotionPreferenceSetting::Reduced,
                text_scale: rmac_theme::TextScalePreference::Large,
            },
            effective: rmac_appearance::ResolvedAppearance {
                color_scheme: rmac_appearance::ResolvedColorScheme::Dark,
                accent_color: rmac_appearance::AccentColor::new(10.0 / 255.0, 132.0 / 255.0, 1.0)
                    .unwrap(),
                contrast: rmac_appearance::Contrast::Higher,
                motion: rmac_appearance::MotionPreference::Reduced,
                text_scale: rmac_appearance::TextScale::Large,
            },
            path: PathBuf::from("/private/theme-8472.json"),
            recovered_from_last_good: false,
            detail: Some("Private recovery detail 8472".into()),
        }
    }

    #[test]
    fn ready_projection_matches_visual_order_selection_and_typed_actions() {
        let host = host(true);
        let theme = theme(rmac_theme::AccentPreference::Custom(components_from_hex(
            0x0a84ff,
        )));
        let snapshot = project_appearance(AppearanceInput {
            theme: Some(&theme),
            host: &host,
            loading: false,
            busy: false,
            refreshing: false,
            error: None,
        })
        .unwrap();

        assert_eq!(snapshot.state, PaneState::Ready);
        assert_eq!(snapshot.groups.len(), 4);
        assert_eq!(
            snapshot
                .groups
                .iter()
                .map(|group| group.kind)
                .collect::<Vec<_>>(),
            vec![
                ChoiceGroupKind::Scheme,
                ChoiceGroupKind::Accent,
                ChoiceGroupKind::Contrast,
                ChoiceGroupKind::Motion,
            ]
        );
        assert_eq!(snapshot.groups[1].value_text, "Blue");
        assert_eq!(
            snapshot.groups[1]
                .choices
                .iter()
                .filter(|choice| choice.selected)
                .map(|choice| choice.kind)
                .collect::<Vec<_>>(),
            vec![AppearanceAction::SetAccent(AccentChoice::Blue)]
        );
        assert_eq!(snapshot.keyboard_order.len(), 19);
        assert_eq!(snapshot.initial_focus.as_deref(), Some(REFRESH_ID));
        assert_eq!(
            snapshot.authority.as_ref().unwrap().effective_appearance,
            "Dark"
        );
        let debug = format!("{snapshot:?}");
        assert!(!debug.contains("/private"));
        assert!(!debug.contains("Private recovery"));
        assert!(!debug.contains("8472"));
    }

    #[test]
    fn loading_busy_unavailable_and_custom_accent_states_are_truthful() {
        let unavailable_host = host(false);
        let loading = project_appearance(AppearanceInput {
            theme: None,
            host: &unavailable_host,
            loading: true,
            busy: false,
            refreshing: false,
            error: None,
        })
        .unwrap();
        assert_eq!(loading.state, PaneState::Loading);
        assert!(loading.keyboard_order.is_empty());
        assert_eq!(loading.announcements.len(), 2);

        let theme = theme(rmac_theme::AccentPreference::Custom([0.1, 0.2, 0.3]));
        let busy = project_appearance(AppearanceInput {
            theme: Some(&theme),
            host: &unavailable_host,
            loading: false,
            busy: true,
            refreshing: false,
            error: Some("Private mutation failure 8472"),
        })
        .unwrap();
        assert_eq!(busy.state, PaneState::Busy);
        assert_eq!(busy.groups[1].value_text, "Custom");
        assert!(busy.groups[1]
            .choices
            .iter()
            .all(|choice| !choice.selected && !choice.enabled));
        assert!(busy.keyboard_order.is_empty());
        assert_eq!(busy.announcements.len(), 3);
        assert!(!format!("{busy:?}").contains("Private mutation"));

        let unavailable = project_appearance(AppearanceInput {
            theme: None,
            host: &host(true),
            loading: false,
            busy: false,
            refreshing: false,
            error: None,
        })
        .unwrap();
        assert_eq!(unavailable.state, PaneState::Unavailable);
        assert_eq!(unavailable.keyboard_order, vec![REFRESH_ID.to_owned()]);
        assert_eq!(
            unavailable.announcements[0].politeness,
            LivePoliteness::Assertive
        );
    }

    #[test]
    fn malformed_accent_and_oversized_error_fail_closed() {
        let host = host(true);
        let invalid = theme(rmac_theme::AccentPreference::Custom([f64::NAN, 0.0, 0.0]));
        assert_eq!(
            project_appearance(AppearanceInput {
                theme: Some(&invalid),
                host: &host,
                loading: false,
                busy: false,
                refreshing: false,
                error: None,
            }),
            Err(AccessibilityProjectionError::InvalidAccent)
        );
        let error = "x".repeat(MAX_TEXT_VALUE_BYTES + 1);
        assert_eq!(
            project_appearance(AppearanceInput {
                theme: None,
                host: &host,
                loading: false,
                busy: false,
                refreshing: false,
                error: Some(&error),
            }),
            Err(AccessibilityProjectionError::TextValueLimit)
        );
    }
}
