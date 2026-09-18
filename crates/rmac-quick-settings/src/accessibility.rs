//! Bounded control, value, focus-order, and live-feedback semantics.

use std::collections::HashSet;
use std::fmt;

use crate::{Control, Tile, View};

pub const QUICK_SETTINGS_TITLE: &str = "Quick Settings";
pub const QUICK_SETTINGS_DESCRIPTION: &str = "Live controls for this computer";
pub const CHANGING_LABEL: &str = "Changing…";
pub const READING_SYSTEM_STATE_LABEL: &str = "Reading system state…";
pub const SYSTEM_SETTINGS_LABEL: &str = "System Settings…";
pub const DISMISS_LABEL: &str = "Dismiss";
pub const MAX_ACCESSIBLE_POWER_PROFILES: usize = 3;
pub const MAX_ACCESSIBLE_TEXT_BYTES: usize = 64 * 1024;
pub const MAX_ACCESSIBLE_LABEL_BYTES: usize = 4 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    Switch,
    Slider,
    Radio,
    Dismiss,
    Route,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionValue {
    Checked(bool),
    Range {
        value: u8,
        minimum: u8,
        maximum: u8,
        step: u8,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibleAction {
    pub id: String,
    pub name: String,
    pub kind: ActionKind,
    pub enabled: bool,
    pub value: Option<ActionValue>,
    pub value_text: Option<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct AccessibleControl {
    pub control: Control,
    pub name: &'static str,
    pub summary: String,
    pub available: bool,
    pub busy: bool,
    pub actions: Vec<AccessibleAction>,
    pub error: Option<String>,
    pub error_action: Option<AccessibleAction>,
}

impl fmt::Debug for AccessibleControl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleControl")
            .field("control", &self.control)
            .field("available", &self.available)
            .field("busy", &self.busy)
            .field("action_count", &self.actions.len())
            .field("has_error", &self.error.is_some())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LivePoliteness {
    Polite,
    Assertive,
}

#[derive(Clone, PartialEq, Eq)]
pub struct LiveAnnouncement {
    pub id: String,
    pub text: String,
    pub politeness: LivePoliteness,
}

impl fmt::Debug for LiveAnnouncement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LiveAnnouncement")
            .field("id", &self.id)
            .field("politeness", &self.politeness)
            .field("text", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceStatus<'a> {
    pub received_snapshot: bool,
    pub stream_error: Option<&'a str>,
    pub operation_error: Option<&'a str>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct QuickSettingsAccessibilitySnapshot {
    pub title: &'static str,
    pub description: &'static str,
    pub controls: Vec<AccessibleControl>,
    /// Enabled action identities in the product-contract keyboard order.
    pub keyboard_order: Vec<String>,
    pub initial_focus: String,
    pub announcements: Vec<LiveAnnouncement>,
    pub operation_error_action: Option<AccessibleAction>,
    pub system_settings_action: AccessibleAction,
}

impl fmt::Debug for QuickSettingsAccessibilitySnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("QuickSettingsAccessibilitySnapshot")
            .field("control_count", &self.controls.len())
            .field("keyboard_action_count", &self.keyboard_order.len())
            .field("initial_focus", &self.initial_focus)
            .field("announcement_count", &self.announcements.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessibilityProjectionError {
    InvalidControlOrder,
    InvalidControl,
    InvalidAction,
    DuplicateAction,
    InvalidPowerProfiles,
    InvalidText,
    TextLimit,
}

pub fn project_quick_settings(
    view: &View,
    status: SurfaceStatus<'_>,
) -> Result<QuickSettingsAccessibilitySnapshot, AccessibilityProjectionError> {
    let mut budget = TextBudget::default();
    budget.add(QUICK_SETTINGS_TITLE)?;
    budget.add(QUICK_SETTINGS_DESCRIPTION)?;

    let controls = vec![
        binary_control(Control::Wifi, &view.wifi, &mut budget)?,
        binary_control(Control::Bluetooth, &view.bluetooth, &mut budget)?,
        sound_control(&view.sound, &mut budget)?,
        power_control(&view.power, &mut budget)?,
        focus_control(&view.focus, &mut budget)?,
    ];
    if controls.iter().map(|control| control.control).ne([
        Control::Wifi,
        Control::Bluetooth,
        Control::Sound,
        Control::Power,
        Control::Focus,
    ]) {
        return Err(AccessibilityProjectionError::InvalidControlOrder);
    }

    let mut announcements = Vec::new();
    if !status.received_snapshot {
        push_announcement(
            &mut announcements,
            &mut budget,
            "quick-settings-loading",
            READING_SYSTEM_STATE_LABEL,
            LivePoliteness::Polite,
        )?;
    }
    if let Some(error) = status.stream_error {
        push_announcement(
            &mut announcements,
            &mut budget,
            "quick-settings-stream-error",
            error,
            LivePoliteness::Assertive,
        )?;
    }
    if let Some(error) = status.operation_error {
        push_announcement(
            &mut announcements,
            &mut budget,
            "quick-settings-operation-error",
            error,
            LivePoliteness::Assertive,
        )?;
    }
    for control in &controls {
        if control.busy {
            push_announcement(
                &mut announcements,
                &mut budget,
                &format!("quick-settings-{}-busy", control_id(control.control)),
                &format!("{}. {CHANGING_LABEL}", control.name),
                LivePoliteness::Polite,
            )?;
        }
        if let Some(error) = &control.error {
            push_announcement(
                &mut announcements,
                &mut budget,
                &format!("quick-settings-{}-error", control_id(control.control)),
                &format!("{}. {error}", control.name),
                LivePoliteness::Assertive,
            )?;
        }
    }

    let operation_error_action = status.operation_error.map(|_| {
        action(
            "dismiss-quick-settings-error",
            DISMISS_LABEL,
            ActionKind::Dismiss,
            true,
            None,
            None,
        )
    });
    let system_settings_action = action(
        "open-system-settings",
        SYSTEM_SETTINGS_LABEL,
        ActionKind::Route,
        true,
        None,
        None,
    );
    validate_action(&system_settings_action, &mut budget)?;
    if let Some(action) = &operation_error_action {
        validate_action(action, &mut budget)?;
    }

    let mut keyboard_order = controls
        .iter()
        .flat_map(|control| control.actions.iter())
        .filter(|action| action.enabled)
        .map(|action| action.id.clone())
        .collect::<Vec<_>>();
    if let Some(action) = &operation_error_action {
        keyboard_order.push(action.id.clone());
    }
    for control in &controls {
        if let Some(action) = &control.error_action {
            keyboard_order.push(action.id.clone());
        }
    }
    keyboard_order.push(system_settings_action.id.clone());
    let mut ids = HashSet::with_capacity(keyboard_order.len());
    if keyboard_order.iter().any(|id| !ids.insert(id.as_str())) {
        return Err(AccessibilityProjectionError::DuplicateAction);
    }
    let initial_focus = keyboard_order
        .first()
        .cloned()
        .ok_or(AccessibilityProjectionError::InvalidAction)?;
    for id in &keyboard_order {
        budget.add(id)?;
    }

    Ok(QuickSettingsAccessibilitySnapshot {
        title: QUICK_SETTINGS_TITLE,
        description: QUICK_SETTINGS_DESCRIPTION,
        controls,
        keyboard_order,
        initial_focus,
        announcements,
        operation_error_action,
        system_settings_action,
    })
}

fn binary_control(
    control: Control,
    tile: &Tile<bool>,
    budget: &mut TextBudget,
) -> Result<AccessibleControl, AccessibilityProjectionError> {
    let name = control.label();
    let summary = displayed_summary(tile.busy, &tile.summary);
    let action = action(
        format!("quick-{control:?}"),
        name,
        ActionKind::Switch,
        tile.available && !tile.busy,
        Some(ActionValue::Checked(tile.value)),
        Some(if tile.value { "On" } else { "Off" }.to_string()),
    );
    control_from_parts(control, name, summary, tile, vec![action], budget)
}

fn sound_control(
    tile: &Tile<crate::SoundValue>,
    budget: &mut TextBudget,
) -> Result<AccessibleControl, AccessibilityProjectionError> {
    if tile.value.volume > 100 {
        return Err(AccessibilityProjectionError::InvalidControl);
    }
    let enabled = tile.available && !tile.busy;
    let actions = vec![
        action(
            "quick-sound-mute",
            "Mute output",
            ActionKind::Switch,
            enabled,
            Some(ActionValue::Checked(tile.value.muted)),
            Some(if tile.value.muted { "Muted" } else { "Unmuted" }.to_string()),
        ),
        action(
            "quick-sound-volume",
            "Output volume",
            ActionKind::Slider,
            enabled,
            Some(ActionValue::Range {
                value: tile.value.volume,
                minimum: 0,
                maximum: 100,
                step: 1,
            }),
            Some(format!("{}%", tile.value.volume)),
        ),
    ];
    control_from_parts(
        Control::Sound,
        Control::Sound.label(),
        displayed_summary(tile.busy, &tile.summary),
        tile,
        actions,
        budget,
    )
}

fn power_control(
    tile: &Tile<crate::PowerValue>,
    budget: &mut TextBudget,
) -> Result<AccessibleControl, AccessibilityProjectionError> {
    if tile.value.supported.len() > MAX_ACCESSIBLE_POWER_PROFILES {
        return Err(AccessibilityProjectionError::InvalidPowerProfiles);
    }
    let mut seen = HashSet::with_capacity(tile.value.supported.len());
    if tile
        .value
        .supported
        .iter()
        .any(|profile| !seen.insert(profile.id()))
        || tile
            .value
            .active
            .is_some_and(|active| !tile.value.supported.contains(&active))
    {
        return Err(AccessibilityProjectionError::InvalidPowerProfiles);
    }
    let enabled = tile.available && !tile.busy;
    let actions = tile
        .value
        .supported
        .iter()
        .copied()
        .map(|profile| {
            action(
                format!("quick-power-{}", profile.id()),
                profile.label(),
                ActionKind::Radio,
                enabled,
                Some(ActionValue::Checked(tile.value.active == Some(profile))),
                None,
            )
        })
        .collect();
    control_from_parts(
        Control::Power,
        Control::Power.label(),
        displayed_summary(tile.busy, &tile.summary),
        tile,
        actions,
        budget,
    )
}

fn focus_control(
    tile: &Tile<crate::FocusValue>,
    budget: &mut TextBudget,
) -> Result<AccessibleControl, AccessibilityProjectionError> {
    let bool_tile = Tile {
        available: tile.available,
        busy: tile.busy,
        value: tile.value.enabled,
        summary: tile.summary.clone(),
        error: tile.error.clone(),
    };
    binary_control(Control::Focus, &bool_tile, budget)
}

fn control_from_parts<T>(
    control: Control,
    name: &'static str,
    summary: String,
    tile: &Tile<T>,
    actions: Vec<AccessibleAction>,
    budget: &mut TextBudget,
) -> Result<AccessibleControl, AccessibilityProjectionError> {
    validate_label(name, budget)?;
    validate_label(&summary, budget)?;
    for action in &actions {
        validate_action(action, budget)?;
    }
    let error = tile.error.clone();
    let error_action = error.as_ref().map(|_| {
        action(
            format!("dismiss-{control:?}-error"),
            DISMISS_LABEL,
            ActionKind::Dismiss,
            true,
            None,
            None,
        )
    });
    if let Some(error) = &error {
        validate_label(error, budget)?;
    }
    if let Some(action) = &error_action {
        validate_action(action, budget)?;
    }
    Ok(AccessibleControl {
        control,
        name,
        summary,
        available: tile.available,
        busy: tile.busy,
        actions,
        error,
        error_action,
    })
}

fn displayed_summary(busy: bool, authoritative: &str) -> String {
    if busy {
        CHANGING_LABEL.to_string()
    } else {
        authoritative.to_string()
    }
}

fn action(
    id: impl Into<String>,
    name: impl Into<String>,
    kind: ActionKind,
    enabled: bool,
    value: Option<ActionValue>,
    value_text: Option<String>,
) -> AccessibleAction {
    AccessibleAction {
        id: id.into(),
        name: name.into(),
        kind,
        enabled,
        value,
        value_text,
    }
}

fn validate_action(
    action: &AccessibleAction,
    budget: &mut TextBudget,
) -> Result<(), AccessibilityProjectionError> {
    validate_id(&action.id, budget)?;
    validate_label(&action.name, budget)?;
    if let Some(value_text) = &action.value_text {
        validate_label(value_text, budget)?;
    }
    match (&action.kind, &action.value) {
        (ActionKind::Switch | ActionKind::Radio, Some(ActionValue::Checked(_)))
        | (ActionKind::Dismiss | ActionKind::Route, None) => Ok(()),
        (
            ActionKind::Slider,
            Some(ActionValue::Range {
                value,
                minimum,
                maximum,
                step,
            }),
        ) if minimum <= value && value <= maximum && minimum < maximum && *step != 0 => Ok(()),
        _ => Err(AccessibilityProjectionError::InvalidAction),
    }
}

fn push_announcement(
    announcements: &mut Vec<LiveAnnouncement>,
    budget: &mut TextBudget,
    id: &str,
    text: &str,
    politeness: LivePoliteness,
) -> Result<(), AccessibilityProjectionError> {
    validate_id(id, budget)?;
    validate_label(text, budget)?;
    announcements.push(LiveAnnouncement {
        id: id.to_string(),
        text: text.to_string(),
        politeness,
    });
    Ok(())
}

fn control_id(control: Control) -> &'static str {
    match control {
        Control::Wifi => "wifi",
        Control::Bluetooth => "bluetooth",
        Control::Sound => "sound",
        Control::Power => "power",
        Control::Focus => "focus",
    }
}

fn validate_id(value: &str, budget: &mut TextBudget) -> Result<(), AccessibilityProjectionError> {
    if value.is_empty()
        || value.len() > MAX_ACCESSIBLE_LABEL_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(AccessibilityProjectionError::InvalidAction);
    }
    budget.add(value)
}

fn validate_label(
    value: &str,
    budget: &mut TextBudget,
) -> Result<(), AccessibilityProjectionError> {
    if value.trim().is_empty()
        || value.len() > MAX_ACCESSIBLE_LABEL_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(AccessibilityProjectionError::InvalidText);
    }
    budget.add(value)
}

#[derive(Default)]
struct TextBudget {
    bytes: usize,
}

impl TextBudget {
    fn add(&mut self, value: &str) -> Result<(), AccessibilityProjectionError> {
        self.bytes = self.bytes.saturating_add(value.len());
        if self.bytes > MAX_ACCESSIBLE_TEXT_BYTES {
            Err(AccessibilityProjectionError::TextLimit)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile<T>(value: T, summary: &str) -> Tile<T> {
        Tile {
            available: true,
            busy: false,
            value,
            summary: summary.to_string(),
            error: None,
        }
    }

    fn view() -> View {
        View {
            wifi: tile(true, "Home Network"),
            bluetooth: tile(false, "Off"),
            sound: tile(
                crate::SoundValue {
                    volume: 42,
                    muted: false,
                },
                "42%",
            ),
            power: tile(
                crate::PowerValue {
                    active: Some(rmac_power::PowerProfile::Balanced),
                    supported: vec![
                        rmac_power::PowerProfile::PowerSaver,
                        rmac_power::PowerProfile::Balanced,
                        rmac_power::PowerProfile::Performance,
                    ],
                },
                "Automatic",
            ),
            focus: tile(
                crate::FocusValue {
                    enabled: true,
                    mode: Some("Work".to_string()),
                    ends_at_unix_ms: None,
                },
                "Work",
            ),
        }
    }

    #[test]
    fn exact_control_values_and_keyboard_order_are_projected() {
        let snapshot = project_quick_settings(
            &view(),
            SurfaceStatus {
                received_snapshot: true,
                stream_error: None,
                operation_error: None,
            },
        )
        .unwrap();

        assert_eq!(
            snapshot
                .controls
                .iter()
                .map(|control| control.control)
                .collect::<Vec<_>>(),
            [
                Control::Wifi,
                Control::Bluetooth,
                Control::Sound,
                Control::Power,
                Control::Focus,
            ]
        );
        assert_eq!(snapshot.controls[0].summary, "Home Network");
        assert_eq!(
            snapshot.controls[2].actions[1].value,
            Some(ActionValue::Range {
                value: 42,
                minimum: 0,
                maximum: 100,
                step: 1,
            })
        );
        assert_eq!(snapshot.controls[3].actions.len(), 3);
        assert_eq!(snapshot.initial_focus, "quick-Wifi");
        assert_eq!(
            snapshot.keyboard_order.last().map(String::as_str),
            Some("open-system-settings")
        );
    }

    #[test]
    fn loading_busy_errors_and_disabled_controls_have_truthful_feedback() {
        let mut view = view();
        view.wifi.busy = true;
        view.bluetooth.available = false;
        view.sound.error = Some("The audio service rejected the change".to_string());
        let snapshot = project_quick_settings(
            &view,
            SurfaceStatus {
                received_snapshot: false,
                stream_error: Some("Live updates stopped"),
                operation_error: Some("Could not open System Settings"),
            },
        )
        .unwrap();

        assert_eq!(snapshot.controls[0].summary, CHANGING_LABEL);
        assert!(!snapshot.controls[0].actions[0].enabled);
        assert!(!snapshot.controls[1].actions[0].enabled);
        assert!(snapshot.controls[2].error_action.is_some());
        assert!(snapshot
            .announcements
            .iter()
            .any(|announcement| announcement.politeness == LivePoliteness::Polite));
        assert!(snapshot
            .announcements
            .iter()
            .any(|announcement| announcement.politeness == LivePoliteness::Assertive));
        assert!(snapshot
            .keyboard_order
            .iter()
            .any(|id| id == "dismiss-Sound-error"));
    }

    #[test]
    fn invalid_volume_profiles_and_oversized_text_fail_closed() {
        let mut invalid_volume = view();
        invalid_volume.sound.value.volume = 101;
        assert_eq!(
            project_quick_settings(
                &invalid_volume,
                SurfaceStatus {
                    received_snapshot: true,
                    stream_error: None,
                    operation_error: None,
                },
            ),
            Err(AccessibilityProjectionError::InvalidControl)
        );

        let mut duplicate_profiles = view();
        duplicate_profiles.power.value.supported = vec![
            rmac_power::PowerProfile::Balanced,
            rmac_power::PowerProfile::Balanced,
        ];
        assert_eq!(
            project_quick_settings(
                &duplicate_profiles,
                SurfaceStatus {
                    received_snapshot: true,
                    stream_error: None,
                    operation_error: None,
                },
            ),
            Err(AccessibilityProjectionError::InvalidPowerProfiles)
        );

        let mut oversized = view();
        oversized.wifi.summary = "x".repeat(MAX_ACCESSIBLE_LABEL_BYTES + 1);
        assert_eq!(
            project_quick_settings(
                &oversized,
                SurfaceStatus {
                    received_snapshot: true,
                    stream_error: None,
                    operation_error: None,
                },
            ),
            Err(AccessibilityProjectionError::InvalidText)
        );
    }
}
