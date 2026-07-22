//! Keyboard-accessible Dock context-menu and overflow-stack presentation.

use std::fmt;

use crate::presentation::{EntryId, OverflowGroup};
use crate::{ContextAction, ContextMenu, MoveDirection, PinCommand};

pub const MAX_MENU_ROWS: usize = 512;
const MAX_LABEL_CHARACTERS: usize = 96;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RowId {
    OverflowApplication(String),
    LaunchNew,
    Window(rmac_compositor::WindowId),
    Pin,
    Move(MoveDirection),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Section {
    Applications,
    Commands,
    Windows,
    Organization,
}

#[derive(Clone, Eq, PartialEq)]
pub enum Action {
    ActivateEntry(EntryId),
    Context(ContextAction),
}

impl fmt::Debug for Action {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ActivateEntry(id) => formatter.debug_tuple("ActivateEntry").field(id).finish(),
            Self::Context(_) => formatter.write_str("Context(<redacted>)"),
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct Row {
    pub id: RowId,
    pub section: Section,
    pub label: String,
    pub accessible_label: String,
    pub enabled: bool,
    pub checked: bool,
    pub urgent: bool,
    pub primary: Option<Action>,
    /// A separately exposed accessibility action/trailing control. Keyboard
    /// renderers route alternate activation here without changing selection.
    pub secondary: Option<Action>,
}

impl fmt::Debug for Row {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Row")
            .field("id", &self.id)
            .field("section", &self.section)
            .field("label", &"<redacted>")
            .field("accessible_label", &"<redacted>")
            .field("enabled", &self.enabled)
            .field("checked", &self.checked)
            .field("urgent", &self.urgent)
            .field("has_primary", &self.primary.is_some())
            .field("has_secondary", &self.secondary.is_some())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyCommand {
    ArrowDown,
    ArrowUp,
    Home,
    End,
    Return,
    AlternateReturn,
    Escape,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Effect {
    None,
    SelectionChanged,
    Activate {
        action: Action,
        restore_focus: EntryId,
    },
    Dismissed {
        restore_focus: EntryId,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuError {
    TooManyRows { count: usize },
    InvalidOverflowEntry,
    InvalidPinAction,
}

impl fmt::Display for MenuError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyRows { count } => {
                write!(
                    formatter,
                    "Dock menu has {count} rows; the limit is {MAX_MENU_ROWS}"
                )
            }
            Self::InvalidOverflowEntry => {
                formatter.write_str("Dock overflow contains a non-application entry")
            }
            Self::InvalidPinAction => {
                formatter.write_str("Dock context menu contains an invalid pin action")
            }
        }
    }
}

impl std::error::Error for MenuError {}

pub struct Session {
    invoker: EntryId,
    title: String,
    accessible_title: String,
    rows: Vec<Row>,
    selected: Option<usize>,
    open: bool,
}

impl fmt::Debug for Session {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Session")
            .field("invoker", &self.invoker)
            .field("title", &"<redacted>")
            .field("accessible_title", &"<redacted>")
            .field("rows", &self.rows)
            .field("selected", &self.selected)
            .field("open", &self.open)
            .finish()
    }
}

impl Session {
    pub fn overflow(overflow: &OverflowGroup) -> Result<Self, MenuError> {
        if overflow.hidden_applications.len() > MAX_MENU_ROWS {
            return Err(MenuError::TooManyRows {
                count: overflow.hidden_applications.len(),
            });
        }
        let rows = overflow
            .hidden_applications
            .iter()
            .map(|entry| {
                let EntryId::Application(app_id) = &entry.id else {
                    return Err(MenuError::InvalidOverflowEntry);
                };
                Ok(Row {
                    id: RowId::OverflowApplication(app_id.clone()),
                    section: Section::Applications,
                    label: bounded(&entry.label),
                    accessible_label: bounded(&entry.accessible_label),
                    enabled: entry.enabled,
                    checked: entry.activity == crate::presentation::ActivityIndicator::Active,
                    urgent: entry.urgent,
                    primary: entry
                        .enabled
                        .then(|| Action::ActivateEntry(entry.id.clone())),
                    secondary: None,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(
            EntryId::Overflow,
            "More Applications".into(),
            overflow.entry.accessible_label.clone(),
            rows,
        )
    }

    pub fn context(menu: &ContextMenu) -> Result<Self, MenuError> {
        let row_count = usize::from(menu.launch_new.is_some())
            + menu.windows.len()
            + 1
            + usize::from(menu.move_left.is_some())
            + usize::from(menu.move_right.is_some());
        if row_count > MAX_MENU_ROWS {
            return Err(MenuError::TooManyRows { count: row_count });
        }
        let mut rows = Vec::new();
        if let Some(action) = &menu.launch_new {
            rows.push(Row {
                id: RowId::LaunchNew,
                section: Section::Commands,
                label: "New Window".into(),
                accessible_label: bounded(&format!(
                    "New {} window",
                    bounded(&menu.application_name)
                )),
                enabled: true,
                checked: false,
                urgent: false,
                primary: Some(Action::Context(action.clone())),
                secondary: None,
            });
        }
        rows.extend(menu.windows.iter().map(|window| {
            let title = bounded(&window.title);
            let mut accessible = vec![title.clone()];
            if window.focused {
                accessible.push("focused".into());
            }
            if window.urgent {
                accessible.push("needs attention".into());
            }
            accessible.push("alternate action closes window".into());
            Row {
                id: RowId::Window(window.id),
                section: Section::Windows,
                label: title,
                accessible_label: bounded(&accessible.join(", ")),
                enabled: true,
                checked: window.focused,
                urgent: window.urgent,
                primary: Some(Action::Context(window.focus.clone())),
                secondary: Some(Action::Context(window.close.clone())),
            }
        }));
        let pin_label = match &menu.pin {
            PinCommand::Pin { .. } => "Keep in Dock",
            PinCommand::Unpin { .. } => "Remove from Dock",
            PinCommand::Move { .. } | PinCommand::MoveTo { .. } => {
                return Err(MenuError::InvalidPinAction);
            }
        };
        rows.push(Row {
            id: RowId::Pin,
            section: Section::Organization,
            label: pin_label.into(),
            accessible_label: bounded(&format!("{pin_label}, {}", bounded(&menu.application_name))),
            enabled: true,
            checked: false,
            urgent: false,
            primary: Some(Action::Context(ContextAction::UpdatePins(menu.pin.clone()))),
            secondary: None,
        });
        for (direction, command, label) in [
            (MoveDirection::Left, menu.move_left.as_ref(), "Move Left"),
            (MoveDirection::Right, menu.move_right.as_ref(), "Move Right"),
        ] {
            if let Some(command) = command {
                rows.push(Row {
                    id: RowId::Move(direction),
                    section: Section::Organization,
                    label: label.into(),
                    accessible_label: bounded(&format!(
                        "{label}, {}",
                        bounded(&menu.application_name)
                    )),
                    enabled: true,
                    checked: false,
                    urgent: false,
                    primary: Some(Action::Context(ContextAction::UpdatePins(command.clone()))),
                    secondary: None,
                });
            }
        }
        Self::new(
            EntryId::Application(menu.app_id.clone()),
            bounded(&menu.application_name),
            format!("{} Dock menu", bounded(&menu.application_name)),
            rows,
        )
    }

    fn new(
        invoker: EntryId,
        title: String,
        accessible_title: String,
        rows: Vec<Row>,
    ) -> Result<Self, MenuError> {
        if rows.len() > MAX_MENU_ROWS {
            return Err(MenuError::TooManyRows { count: rows.len() });
        }
        let selected = rows.iter().position(|row| row.enabled);
        Ok(Self {
            invoker,
            title: bounded(&title),
            accessible_title: bounded(&accessible_title),
            rows,
            selected,
            open: true,
        })
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn accessible_title(&self) -> &str {
        &self.accessible_title
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn selected(&self) -> Option<&RowId> {
        self.selected.map(|index| &self.rows[index].id)
    }

    pub fn select(&mut self, id: &RowId) -> bool {
        if !self.open {
            return false;
        }
        let next = self
            .rows
            .iter()
            .position(|row| &row.id == id && row.enabled);
        if next.is_some() && next != self.selected {
            self.selected = next;
            true
        } else {
            false
        }
    }

    pub fn handle_key(&mut self, command: KeyCommand) -> Effect {
        if !self.open {
            return Effect::None;
        }
        match command {
            KeyCommand::ArrowDown => self.move_selection(true),
            KeyCommand::ArrowUp => self.move_selection(false),
            KeyCommand::Home => self.move_to_edge(true),
            KeyCommand::End => self.move_to_edge(false),
            KeyCommand::Return => self.activate(false),
            KeyCommand::AlternateReturn => self.activate(true),
            KeyCommand::Escape => {
                self.open = false;
                Effect::Dismissed {
                    restore_focus: self.invoker.clone(),
                }
            }
        }
    }

    fn enabled_indices(&self) -> Vec<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| row.enabled.then_some(index))
            .collect()
    }

    fn move_selection(&mut self, forward: bool) -> Effect {
        let enabled = self.enabled_indices();
        if enabled.is_empty() {
            return Effect::None;
        }
        let current = self
            .selected
            .and_then(|selected| enabled.iter().position(|index| *index == selected));
        let next = match (current, forward) {
            (Some(index), true) => enabled[(index + 1) % enabled.len()],
            (Some(0), false) | (None, false) => *enabled.last().expect("enabled is nonempty"),
            (Some(index), false) => enabled[index - 1],
            (None, true) => enabled[0],
        };
        self.set_selection(next)
    }

    fn move_to_edge(&mut self, first: bool) -> Effect {
        let enabled = self.enabled_indices();
        let next = if first {
            enabled.first()
        } else {
            enabled.last()
        };
        next.copied()
            .map(|index| self.set_selection(index))
            .unwrap_or(Effect::None)
    }

    fn set_selection(&mut self, next: usize) -> Effect {
        if self.selected == Some(next) {
            Effect::None
        } else {
            self.selected = Some(next);
            Effect::SelectionChanged
        }
    }

    fn activate(&mut self, secondary: bool) -> Effect {
        let Some(row) = self.selected.and_then(|index| self.rows.get(index)) else {
            return Effect::None;
        };
        let action = if secondary {
            row.secondary.as_ref()
        } else {
            row.primary.as_ref()
        };
        let Some(action) = action.cloned() else {
            return Effect::None;
        };
        self.open = false;
        Effect::Activate {
            action,
            restore_focus: self.invoker.clone(),
        }
    }
}

fn bounded(value: &str) -> String {
    let mut characters = value.trim().chars();
    let mut result = characters
        .by_ref()
        .take(MAX_LABEL_CHARACTERS)
        .collect::<String>();
    if characters.next().is_some() {
        result.pop();
        result.push('…');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presentation::{ActivityIndicator, BuiltinIcon, Entry, Icon};
    use crate::{ContextMenu, WindowMenu};

    fn entry(id: &str, enabled: bool) -> Entry {
        Entry {
            id: EntryId::Application(id.into()),
            label: id.trim_end_matches(".desktop").into(),
            accessible_label: id.trim_end_matches(".desktop").into(),
            icon: Icon::Builtin(BuiltinIcon::Application),
            enabled,
            activity: ActivityIndicator::None,
            urgent: false,
            badge: None,
        }
    }

    fn focus(app_id: &str, window: u64) -> ContextAction {
        ContextAction::FocusWindow {
            app_id: app_id.into(),
            window: rmac_compositor::WindowId(window),
        }
    }

    fn close(app_id: &str, window: u64) -> ContextAction {
        ContextAction::CloseWindow {
            app_id: app_id.into(),
            window: rmac_compositor::WindowId(window),
        }
    }

    #[test]
    fn overflow_preserves_order_skips_disabled_rows_and_restores_focus() {
        let overflow = OverflowGroup {
            entry: Entry {
                id: EntryId::Overflow,
                label: "More".into(),
                accessible_label: "More applications, 3 hidden applications".into(),
                icon: Icon::Builtin(BuiltinIcon::More),
                enabled: true,
                activity: ActivityIndicator::None,
                urgent: false,
                badge: Some(3),
            },
            hidden_applications: vec![
                entry("one.desktop", false),
                entry("two.desktop", true),
                entry("three.desktop", true),
            ],
        };
        let mut session = Session::overflow(&overflow).unwrap();

        assert_eq!(
            session
                .rows()
                .iter()
                .map(|row| row.id.clone())
                .collect::<Vec<_>>(),
            [
                RowId::OverflowApplication("one.desktop".into()),
                RowId::OverflowApplication("two.desktop".into()),
                RowId::OverflowApplication("three.desktop".into()),
            ]
        );
        assert_eq!(
            session.selected(),
            Some(&RowId::OverflowApplication("two.desktop".into()))
        );
        assert_eq!(
            session.handle_key(KeyCommand::ArrowUp),
            Effect::SelectionChanged
        );
        assert_eq!(
            session.selected(),
            Some(&RowId::OverflowApplication("three.desktop".into()))
        );
        assert_eq!(
            session.handle_key(KeyCommand::Return),
            Effect::Activate {
                action: Action::ActivateEntry(EntryId::Application("three.desktop".into())),
                restore_focus: EntryId::Overflow,
            }
        );
        assert!(!session.is_open());
        assert!(!session.select(&RowId::OverflowApplication("two.desktop".into())));
        assert_eq!(session.handle_key(KeyCommand::Escape), Effect::None);
    }

    #[test]
    fn context_rows_keep_exact_actions_and_alternate_close() {
        let menu = ContextMenu {
            app_id: "terminal.desktop".into(),
            application_name: "Terminal".into(),
            launch_new: None,
            windows: vec![WindowMenu {
                id: rmac_compositor::WindowId(7),
                title: "/home/alex/private — shell".into(),
                focused: true,
                urgent: true,
                focus: focus("terminal.desktop", 7),
                close: close("terminal.desktop", 7),
            }],
            pin: PinCommand::Unpin {
                app_id: "terminal.desktop".into(),
            },
            move_left: Some(PinCommand::Move {
                app_id: "terminal.desktop".into(),
                direction: MoveDirection::Left,
            }),
            move_right: None,
        };
        let mut session = Session::context(&menu).unwrap();

        assert_eq!(
            session.selected(),
            Some(&RowId::Window(rmac_compositor::WindowId(7)))
        );
        assert!(session.rows()[0].checked);
        assert!(session.rows()[0].urgent);
        assert!(!format!("{session:?}").contains("alex"));
        assert_eq!(
            session.handle_key(KeyCommand::AlternateReturn),
            Effect::Activate {
                action: Action::Context(close("terminal.desktop", 7)),
                restore_focus: EntryId::Application("terminal.desktop".into()),
            }
        );
    }

    #[test]
    fn home_end_escape_and_pointer_selection_are_deterministic() {
        let menu = ContextMenu {
            app_id: "music.desktop".into(),
            application_name: "Music".into(),
            launch_new: None,
            windows: Vec::new(),
            pin: PinCommand::Pin {
                app_id: "music.desktop".into(),
            },
            move_left: None,
            move_right: None,
        };
        let mut session = Session::context(&menu).unwrap();
        assert_eq!(session.selected(), Some(&RowId::Pin));
        assert_eq!(session.handle_key(KeyCommand::Home), Effect::None);
        assert!(!session.select(&RowId::LaunchNew));
        assert_eq!(
            session.handle_key(KeyCommand::Escape),
            Effect::Dismissed {
                restore_focus: EntryId::Application("music.desktop".into())
            }
        );
        assert!(!session.is_open());
    }

    #[test]
    fn labels_are_bounded_and_oversized_menus_fail_explicitly() {
        let long = "x".repeat(MAX_LABEL_CHARACTERS + 20);
        assert_eq!(bounded(&long).chars().count(), MAX_LABEL_CHARACTERS);
        assert!(bounded(&long).ends_with('…'));

        let overflow = OverflowGroup {
            entry: entry("more.desktop", true),
            hidden_applications: (0..=MAX_MENU_ROWS)
                .map(|index| entry(&format!("app-{index}.desktop"), true))
                .collect(),
        };
        assert_eq!(
            Session::overflow(&overflow).unwrap_err(),
            MenuError::TooManyRows {
                count: MAX_MENU_ROWS + 1
            }
        );

        let malformed = OverflowGroup {
            entry: entry("more.desktop", true),
            hidden_applications: vec![Entry {
                id: EntryId::Special(crate::SpecialItemKind::Files),
                ..entry("files.desktop", true)
            }],
        };
        assert_eq!(
            Session::overflow(&malformed).unwrap_err(),
            MenuError::InvalidOverflowEntry
        );
    }
}
