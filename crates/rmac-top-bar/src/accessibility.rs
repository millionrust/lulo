//! Bounded, framework-neutral semantics for one passive top-bar surface.

use std::fmt;

use crate::{BuiltinIcon, Content, IndicatorKind, PanelTarget, SYSTEM_MARK_ACCESSIBLE_NAME};

pub const TOP_BAR_NAME: &str = "rmac top bar";
pub const ROOT_ID: &str = "top-bar";
pub const SYSTEM_MARK_ID: &str = "top-bar-system";
pub const ACTIVE_APP_ID: &str = "top-bar-active-app";
pub const ACTIVE_APP_NAME: &str = "Active application";
pub const WORKSPACE_ID: &str = "top-bar-workspace";
pub const WORKSPACE_NAME: &str = "Workspace";
pub const CLOCK_ID: &str = "top-bar-clock";
pub const OPEN_QUICK_SETTINGS_NAME: &str = "Open Quick Settings";
pub const OPEN_NOTIFICATION_CENTER_NAME: &str = "Open Notification Center";
pub const MAX_INDICATORS: usize = 7;
pub const MAX_TEXT_BYTES: usize = 512;
pub const MAX_SEMANTIC_TEXT_BYTES: usize = 8 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessibleRole {
    Toolbar,
    Image,
    Text,
    Time,
    Button,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessibleActionKind {
    OpenQuickSettings,
    OpenNotificationCenter,
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleAction {
    pub id: String,
    pub name: &'static str,
    pub kind: AccessibleActionKind,
    pub target: PanelTarget,
}

impl fmt::Debug for AccessibleAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleAction")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("target", &self.target)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleNode {
    pub id: String,
    pub role: AccessibleRole,
    pub name: String,
    pub value: Option<String>,
    pub urgent: bool,
    pub focusable: bool,
    pub action: Option<AccessibleAction>,
}

impl fmt::Debug for AccessibleNode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleNode")
            .field("id", &self.id)
            .field("role", &self.role)
            .field("name", &"<redacted>")
            .field("value", &self.value.as_ref().map(|_| "<redacted>"))
            .field("urgent", &self.urgent)
            .field("focusable", &self.focusable)
            .field("action", &self.action)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct TopBarAccessibilitySnapshot {
    pub root_id: &'static str,
    pub role: AccessibleRole,
    pub name: &'static str,
    /// The layer surface must not enter ordinary application Tab order.
    pub keyboard_interactive: bool,
    pub system_mark: AccessibleNode,
    pub active_app: AccessibleNode,
    pub workspace: Option<AccessibleNode>,
    pub clock: AccessibleNode,
    pub indicators: Vec<AccessibleNode>,
    pub reading_order: Vec<String>,
    /// Empty by design: AT actions remain invokable without stealing focus.
    pub keyboard_focus_order: Vec<String>,
}

impl fmt::Debug for TopBarAccessibilitySnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TopBarAccessibilitySnapshot")
            .field("root_id", &self.root_id)
            .field("role", &self.role)
            .field("name", &self.name)
            .field("keyboard_interactive", &self.keyboard_interactive)
            .field("system_mark", &self.system_mark)
            .field("active_app", &self.active_app)
            .field("has_workspace", &self.workspace.is_some())
            .field("clock", &self.clock)
            .field("indicator_count", &self.indicators.len())
            .field("reading_order", &self.reading_order)
            .field("keyboard_focus_order", &self.keyboard_focus_order)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessibilityProjectionError {
    InvalidSystemMark,
    IndicatorLimit,
    DuplicateIndicator,
    InvalidIndicatorOrder,
    InvalidIndicator,
    InvalidText,
    TextValueLimit,
    TextLimit,
}

pub fn project_accessibility(
    content: &Content,
) -> Result<TopBarAccessibilitySnapshot, AccessibilityProjectionError> {
    if content.system_mark.icon != BuiltinIcon::System
        || content.system_mark.accessible != SYSTEM_MARK_ACCESSIBLE_NAME
    {
        return Err(AccessibilityProjectionError::InvalidSystemMark);
    }
    if content.indicators.len() > MAX_INDICATORS {
        return Err(AccessibilityProjectionError::IndicatorLimit);
    }

    let mut budget = TextBudget::default();
    for fixed in [
        TOP_BAR_NAME,
        ROOT_ID,
        SYSTEM_MARK_ID,
        SYSTEM_MARK_ACCESSIBLE_NAME,
        ACTIVE_APP_ID,
        ACTIVE_APP_NAME,
        WORKSPACE_ID,
        WORKSPACE_NAME,
        CLOCK_ID,
        OPEN_QUICK_SETTINGS_NAME,
        OPEN_NOTIFICATION_CENTER_NAME,
    ] {
        budget.add_required(fixed)?;
    }
    budget.add_required(&content.active_app)?;
    if let Some(workspace) = &content.workspace {
        budget.add_required(workspace)?;
    }
    budget.add_required(&content.clock.visible)?;
    budget.add_required(&content.clock.accessible)?;
    if content.clock.activation != PanelTarget::NotificationCenter {
        return Err(AccessibilityProjectionError::InvalidIndicator);
    }

    let system_mark = node(
        SYSTEM_MARK_ID,
        AccessibleRole::Image,
        content.system_mark.accessible,
        None,
        false,
        None,
    );
    let active_app = node(
        ACTIVE_APP_ID,
        AccessibleRole::Text,
        ACTIVE_APP_NAME,
        Some(content.active_app.as_str()),
        false,
        None,
    );
    let workspace = content.workspace.as_deref().map(|workspace| {
        node(
            WORKSPACE_ID,
            AccessibleRole::Text,
            WORKSPACE_NAME,
            Some(workspace),
            false,
            None,
        )
    });
    let clock_action = action(CLOCK_ID, content.clock.activation);
    budget.add_required(&clock_action.id)?;
    let clock = node(
        CLOCK_ID,
        AccessibleRole::Time,
        &content.clock.accessible,
        Some(content.clock.visible.as_str()),
        false,
        Some(clock_action),
    );

    let mut seen = Vec::with_capacity(content.indicators.len());
    let mut prior_rank = None;
    let mut indicators = Vec::with_capacity(content.indicators.len());
    for indicator in &content.indicators {
        if seen.contains(&indicator.kind) {
            return Err(AccessibilityProjectionError::DuplicateIndicator);
        }
        seen.push(indicator.kind);
        let rank = indicator_rank(indicator.kind);
        if prior_rank.is_some_and(|prior| rank <= prior) {
            return Err(AccessibilityProjectionError::InvalidIndicatorOrder);
        }
        prior_rank = Some(rank);
        if indicator.icon != BuiltinIcon::from(indicator.kind)
            || indicator.activation != indicator.kind.panel_target()
            || (indicator.urgent && indicator.kind != IndicatorKind::Notifications)
        {
            return Err(AccessibilityProjectionError::InvalidIndicator);
        }
        budget.add_required(&indicator.visible)?;
        budget.add_required(&indicator.accessible)?;
        let id = indicator_id(indicator.kind);
        budget.add_required(id)?;
        let action = action(id, indicator.activation);
        budget.add_required(&action.id)?;
        indicators.push(node(
            id,
            AccessibleRole::Button,
            &indicator.accessible,
            Some(indicator.visible.as_str()),
            indicator.urgent,
            Some(action),
        ));
    }

    let mut reading_order = Vec::with_capacity(indicators.len() + 4);
    reading_order.push(SYSTEM_MARK_ID.into());
    reading_order.push(ACTIVE_APP_ID.into());
    if workspace.is_some() {
        reading_order.push(WORKSPACE_ID.into());
    }
    reading_order.push(CLOCK_ID.into());
    reading_order.extend(indicators.iter().map(|indicator| indicator.id.clone()));

    Ok(TopBarAccessibilitySnapshot {
        root_id: ROOT_ID,
        role: AccessibleRole::Toolbar,
        name: TOP_BAR_NAME,
        keyboard_interactive: false,
        system_mark,
        active_app,
        workspace,
        clock,
        indicators,
        reading_order,
        keyboard_focus_order: Vec::new(),
    })
}

fn node(
    id: &str,
    role: AccessibleRole,
    name: &str,
    value: Option<&str>,
    urgent: bool,
    action: Option<AccessibleAction>,
) -> AccessibleNode {
    AccessibleNode {
        id: id.into(),
        role,
        name: name.into(),
        value: value.map(str::to_owned),
        urgent,
        focusable: false,
        action,
    }
}

fn action(id: &str, target: PanelTarget) -> AccessibleAction {
    let (name, kind) = match target {
        PanelTarget::QuickSettings => (
            OPEN_QUICK_SETTINGS_NAME,
            AccessibleActionKind::OpenQuickSettings,
        ),
        PanelTarget::NotificationCenter => (
            OPEN_NOTIFICATION_CENTER_NAME,
            AccessibleActionKind::OpenNotificationCenter,
        ),
    };
    AccessibleAction {
        id: format!("{id}-open"),
        name,
        kind,
        target,
    }
}

const fn indicator_id(kind: IndicatorKind) -> &'static str {
    match kind {
        IndicatorKind::Focus => "top-bar-focus",
        IndicatorKind::Vpn => "top-bar-vpn",
        IndicatorKind::Network => "top-bar-network",
        IndicatorKind::Bluetooth => "top-bar-bluetooth",
        IndicatorKind::Sound => "top-bar-sound",
        IndicatorKind::Battery => "top-bar-battery",
        IndicatorKind::Notifications => "top-bar-notifications",
    }
}

const fn indicator_rank(kind: IndicatorKind) -> u8 {
    match kind {
        IndicatorKind::Focus => 0,
        IndicatorKind::Vpn => 1,
        IndicatorKind::Network => 2,
        IndicatorKind::Bluetooth => 3,
        IndicatorKind::Sound => 4,
        IndicatorKind::Battery => 5,
        IndicatorKind::Notifications => 6,
    }
}

#[derive(Default)]
struct TextBudget {
    bytes: usize,
}

impl TextBudget {
    fn add_required(&mut self, text: &str) -> Result<(), AccessibilityProjectionError> {
        if text.trim().is_empty() || text.chars().any(char::is_control) {
            return Err(AccessibilityProjectionError::InvalidText);
        }
        if text.len() > MAX_TEXT_BYTES {
            return Err(AccessibilityProjectionError::TextValueLimit);
        }
        self.bytes = self.bytes.saturating_add(text.len());
        if self.bytes > MAX_SEMANTIC_TEXT_BYTES {
            return Err(AccessibilityProjectionError::TextLimit);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClockLabel, IndicatorLabel, SystemMark};

    fn indicator(kind: IndicatorKind, private: &str, urgent: bool) -> IndicatorLabel {
        IndicatorLabel {
            kind,
            icon: kind.into(),
            activation: kind.panel_target(),
            visible: private.into(),
            accessible: format!("{private} state"),
            urgent,
        }
    }

    fn content() -> Content {
        Content {
            system_mark: SystemMark::default(),
            active_app: "Private Editor".into(),
            workspace: Some("Private Workspace".into()),
            clock: ClockLabel {
                visible: "9:41 PM".into(),
                accessible: "Private date and time".into(),
                activation: PanelTarget::NotificationCenter,
            },
            indicators: vec![
                indicator(IndicatorKind::Focus, "Private Focus", false),
                indicator(IndicatorKind::Network, "Private Network", false),
                indicator(IndicatorKind::Notifications, "Private Notifications", true),
            ],
        }
    }

    #[test]
    fn complete_passive_toolbar_semantics_preserve_reading_order_and_actions() {
        let snapshot = project_accessibility(&content()).unwrap();

        assert_eq!(snapshot.role, AccessibleRole::Toolbar);
        assert_eq!(snapshot.name, TOP_BAR_NAME);
        assert!(!snapshot.keyboard_interactive);
        assert!(snapshot.keyboard_focus_order.is_empty());
        assert!(snapshot.reading_order.starts_with(&[
            SYSTEM_MARK_ID.into(),
            ACTIVE_APP_ID.into(),
            WORKSPACE_ID.into()
        ]));
        assert_eq!(snapshot.clock.role, AccessibleRole::Time);
        assert_eq!(
            snapshot.clock.action.as_ref().unwrap().kind,
            AccessibleActionKind::OpenNotificationCenter
        );
        assert!(snapshot.indicators.iter().all(|node| !node.focusable));
        assert_eq!(
            snapshot.indicators[0].action.as_ref().unwrap().kind,
            AccessibleActionKind::OpenQuickSettings
        );
        assert!(snapshot.indicators[2].urgent);
    }

    #[test]
    fn diagnostics_redact_visible_shell_identity_and_state() {
        let snapshot = project_accessibility(&content()).unwrap();
        let diagnostics = format!("{snapshot:?}");
        for private in [
            "Private Editor",
            "Private Workspace",
            "Private date and time",
            "Private Focus",
            "Private Network",
            "Private Notifications",
        ] {
            assert!(!diagnostics.contains(private));
        }
    }

    #[test]
    fn malformed_duplicate_reordered_and_oversized_content_fails_closed() {
        let mut duplicate = content();
        duplicate.indicators[1] = duplicate.indicators[0].clone();
        assert_eq!(
            project_accessibility(&duplicate),
            Err(AccessibilityProjectionError::DuplicateIndicator)
        );

        let mut reordered = content();
        reordered.indicators.swap(0, 1);
        assert_eq!(
            project_accessibility(&reordered),
            Err(AccessibilityProjectionError::InvalidIndicatorOrder)
        );

        let mut wrong_target = content();
        wrong_target.indicators[0].activation = PanelTarget::NotificationCenter;
        assert_eq!(
            project_accessibility(&wrong_target),
            Err(AccessibilityProjectionError::InvalidIndicator)
        );

        let mut oversized = content();
        oversized.active_app = "x".repeat(MAX_TEXT_BYTES + 1);
        assert_eq!(
            project_accessibility(&oversized),
            Err(AccessibilityProjectionError::TextValueLimit)
        );

        let mut aggregate = content();
        aggregate.active_app = "a".repeat(MAX_TEXT_BYTES);
        aggregate.workspace = Some("w".repeat(MAX_TEXT_BYTES));
        aggregate.clock.visible = "v".repeat(MAX_TEXT_BYTES);
        aggregate.clock.accessible = "c".repeat(MAX_TEXT_BYTES);
        aggregate.indicators = [
            IndicatorKind::Focus,
            IndicatorKind::Vpn,
            IndicatorKind::Network,
            IndicatorKind::Bluetooth,
            IndicatorKind::Sound,
            IndicatorKind::Battery,
            IndicatorKind::Notifications,
        ]
        .into_iter()
        .map(|kind| indicator(kind, &"s".repeat(MAX_TEXT_BYTES - 6), false))
        .collect();
        assert_eq!(
            project_accessibility(&aggregate),
            Err(AccessibilityProjectionError::TextLimit)
        );
    }
}
