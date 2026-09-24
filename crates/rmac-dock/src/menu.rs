//! Keyboard-accessible Dock context-menu and overflow-stack presentation.

use std::fmt;

use crate::presentation::{EntryId, OverflowGroup};
use crate::{
    ContextAction, ContextMenu, PinCommand, SpecialActivation, SpecialContextAction,
    SpecialContextMenu, SpecialItemKind,
};

pub const MAX_MENU_ROWS: usize = 512;
const MAX_LABEL_CHARACTERS: usize = 96;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RowId {
    OverflowApplication(String),
    Open,
    ApplicationCommand(String),
    Window(rmac_compositor::WindowId),
    Pin,
    ShowInFinder,
    ShowAllWindows,
    Hide,
    Quit,
    OpenSpecial(SpecialItemKind),
    EmptyTrash,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Section {
    Applications,
    Commands,
    Windows,
    Organization,
    /// Open or Quit, always last in a macOS Dock menu.
    Lifecycle,
    Destructive,
}

/// A submenu that groups rows under one parent row, as macOS does with
/// Options ▸ (Keep in Dock, Open at Login, Show in Finder).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Submenu {
    Options,
}

impl Submenu {
    pub fn label(self) -> &'static str {
        match self {
            Self::Options => "Options",
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub enum Action {
    ActivateEntry(EntryId),
    Context(ContextAction),
    SpecialContext(SpecialContextAction),
}

impl fmt::Debug for Action {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ActivateEntry(id) => formatter.debug_tuple("ActivateEntry").field(id).finish(),
            Self::Context(_) => formatter.write_str("Context(<redacted>)"),
            Self::SpecialContext(_) => formatter.write_str("SpecialContext(<redacted>)"),
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
    /// True for an action that must open an explicit confirmation sheet before
    /// any irreversible work. Selection alone never authorizes the mutation.
    pub destructive: bool,
    pub primary: Option<Action>,
    /// A separately exposed accessibility action/trailing control. Keyboard
    /// renderers route alternate activation here without changing selection.
    pub secondary: Option<Action>,
    /// Label shown instead of `label` while Option is held; only rows whose
    /// secondary action is the Option alternative (Quit → Force Quit) have it.
    pub alternate_label: Option<String>,
    /// Pointer renderers draw rows with a submenu inside that submenu's
    /// panel. Keyboard order and accessibility keep one flat list.
    pub submenu: Option<Submenu>,
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
            .field("destructive", &self.destructive)
            .field("has_primary", &self.primary.is_some())
            .field("has_secondary", &self.secondary.is_some())
            .field("submenu", &self.submenu)
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
    InvalidSpecialAction,
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
            Self::InvalidSpecialAction => {
                formatter.write_str("Dock context menu contains an invalid special-item action")
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
                    destructive: false,
                    primary: entry
                        .enabled
                        .then(|| Action::ActivateEntry(entry.id.clone())),
                    secondary: None,
                    alternate_label: None,
                    submenu: None,
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

    /// The macOS 26 Dock menu, top to bottom: the app's windows (the
    /// focused one ticked), the app's own commands, Options ▸ with Keep in
    /// Dock and Show in Files, then Open for a closed app or Quit for a
    /// running one, preceded by Show All Windows and Hide (Option turns
    /// Hide into Hide Others and Quit into Force Quit). Hiding parks the
    /// windows (ADR 0014); Show All Windows is App Exposé.
    pub fn context(menu: &ContextMenu) -> Result<Self, MenuError> {
        let row_count = usize::from(menu.open.is_some())
            + menu.application_commands.len()
            + menu.windows.len()
            + 1
            + usize::from(menu.show_in_finder.is_some())
            + usize::from(menu.show_all_windows.is_some())
            + usize::from(menu.hide.is_some())
            + usize::from(menu.quit.is_some());
        if row_count > MAX_MENU_ROWS {
            return Err(MenuError::TooManyRows { count: row_count });
        }
        let running = !menu.windows.is_empty() || menu.quit.is_some();
        let mut rows = Vec::new();
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
                destructive: false,
                primary: Some(Action::Context(window.focus.clone())),
                secondary: Some(Action::Context(window.close.clone())),
                alternate_label: None,
                submenu: None,
            }
        }));
        rows.extend(menu.application_commands.iter().map(|command| Row {
            id: RowId::ApplicationCommand(command.id.clone()),
            section: Section::Commands,
            label: bounded(&command.name),
            accessible_label: bounded(&format!(
                "{}, {}",
                bounded(&command.name),
                bounded(&menu.application_name)
            )),
            enabled: true,
            checked: false,
            urgent: false,
            destructive: false,
            primary: Some(Action::Context(command.action.clone())),
            secondary: None,
            alternate_label: None,
            submenu: None,
        }));
        // A kept app that is running shows a ticked "Keep in Dock" whose
        // choice removes it; a closed kept app offers "Remove from Dock".
        let (pin_label, pin_checked) = match &menu.pin {
            PinCommand::Pin { .. } => ("Keep in Dock", false),
            PinCommand::Unpin { .. } if running => ("Keep in Dock", true),
            PinCommand::Unpin { .. } => ("Remove from Dock", false),
            PinCommand::Move { .. } | PinCommand::MoveTo { .. } => {
                return Err(MenuError::InvalidPinAction);
            }
        };
        let pin_accessible = match &menu.pin {
            PinCommand::Unpin { .. } => "Remove from Dock",
            _ => "Keep in Dock",
        };
        rows.push(Row {
            id: RowId::Pin,
            section: Section::Organization,
            label: pin_label.into(),
            accessible_label: bounded(&format!(
                "{pin_accessible}, {}",
                bounded(&menu.application_name)
            )),
            enabled: true,
            checked: pin_checked,
            urgent: false,
            destructive: false,
            primary: Some(Action::Context(ContextAction::UpdatePins(menu.pin.clone()))),
            secondary: None,
            alternate_label: None,
            submenu: Some(Submenu::Options),
        });
        if let Some(action) = &menu.show_in_finder {
            rows.push(Row {
                id: RowId::ShowInFinder,
                section: Section::Organization,
                label: "Show in Files".into(),
                accessible_label: bounded(&format!(
                    "Show {} in Files",
                    bounded(&menu.application_name)
                )),
                enabled: true,
                checked: false,
                urgent: false,
                destructive: false,
                primary: Some(Action::Context(action.clone())),
                secondary: None,
                alternate_label: None,
                submenu: Some(Submenu::Options),
            });
        }
        if let Some(action) = &menu.open {
            rows.push(Row {
                id: RowId::Open,
                section: Section::Lifecycle,
                label: "Open".into(),
                accessible_label: bounded(&format!("Open {}", bounded(&menu.application_name))),
                enabled: true,
                checked: false,
                urgent: false,
                destructive: false,
                primary: Some(Action::Context(action.clone())),
                secondary: None,
                alternate_label: None,
                submenu: None,
            });
        }
        if let Some(action) = &menu.show_all_windows {
            rows.push(Row {
                id: RowId::ShowAllWindows,
                section: Section::Lifecycle,
                label: "Show All Windows".into(),
                accessible_label: bounded(&format!(
                    "Show All Windows of {}",
                    bounded(&menu.application_name)
                )),
                enabled: true,
                checked: false,
                urgent: false,
                destructive: false,
                primary: Some(Action::Context(action.clone())),
                secondary: None,
                alternate_label: None,
                submenu: None,
            });
        }
        if let Some(action) = &menu.hide {
            let hide_others = menu.hide_others.clone().map(Action::Context);
            rows.push(Row {
                id: RowId::Hide,
                section: Section::Lifecycle,
                label: "Hide".into(),
                accessible_label: bounded(&format!(
                    "Hide {}, hold Option to Hide Others",
                    bounded(&menu.application_name)
                )),
                enabled: true,
                checked: false,
                urgent: false,
                destructive: false,
                primary: Some(Action::Context(action.clone())),
                alternate_label: hide_others.as_ref().map(|_| "Hide Others".to_owned()),
                secondary: hide_others,
                submenu: None,
            });
        }
        if let Some(action) = &menu.quit {
            let force_quit = menu.force_quit.clone().map(Action::Context);
            rows.push(Row {
                id: RowId::Quit,
                section: Section::Lifecycle,
                label: "Quit".into(),
                accessible_label: bounded(&format!(
                    "Quit {}, hold Option to Force Quit",
                    bounded(&menu.application_name)
                )),
                enabled: true,
                checked: false,
                urgent: false,
                destructive: false,
                primary: Some(Action::Context(action.clone())),
                alternate_label: force_quit.as_ref().map(|_| "Force Quit".to_owned()),
                secondary: force_quit,
                submenu: None,
            });
        }
        Self::new(
            EntryId::Application(menu.app_id.clone()),
            bounded(&menu.application_name),
            format!("{} Dock menu", bounded(&menu.application_name)),
            rows,
        )
    }

    /// The macOS Trash menu: Open, a separator, then Empty Trash, which stays
    /// visible but disabled while the Trash is empty.
    pub fn special(menu: &SpecialContextMenu) -> Result<Self, MenuError> {
        let available = validate_special_menu(menu)?;
        let name = special_name(menu.kind);
        let mut rows = vec![Row {
            id: RowId::OpenSpecial(menu.kind),
            section: Section::Commands,
            label: "Open".into(),
            accessible_label: format!("Open {name}"),
            enabled: available,
            checked: false,
            urgent: false,
            destructive: false,
            primary: available.then_some(Action::ActivateEntry(EntryId::Special(menu.kind))),
            secondary: None,
            alternate_label: None,
            submenu: None,
        }];
        match &menu.empty_trash {
            Some(action) => {
                let SpecialContextAction::EmptyTrash {
                    expected_item_count,
                } = action;
                let item_label = if *expected_item_count == 1 {
                    "item"
                } else {
                    "items"
                };
                rows.push(Row {
                    id: RowId::EmptyTrash,
                    section: Section::Destructive,
                    label: "Empty Trash".into(),
                    accessible_label: bounded(&format!(
                        "Empty Trash permanently, {expected_item_count} {item_label}, requires confirmation"
                    )),
                    enabled: true,
                    checked: false,
                    urgent: false,
                    destructive: true,
                    primary: Some(Action::SpecialContext(action.clone())),
                    secondary: None,
                    alternate_label: None,
                    submenu: None,
                });
            }
            None if menu.kind == SpecialItemKind::Trash && available => rows.push(Row {
                id: RowId::EmptyTrash,
                section: Section::Destructive,
                label: "Empty Trash".into(),
                accessible_label: "Empty Trash, the Trash is empty".into(),
                enabled: false,
                checked: false,
                urgent: false,
                destructive: false,
                primary: None,
                secondary: None,
                alternate_label: None,
                submenu: None,
            }),
            None => {}
        }
        Self::new(
            EntryId::Special(menu.kind),
            name.into(),
            format!("{name} Dock menu"),
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

    pub fn invoker(&self) -> &EntryId {
        &self.invoker
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

fn validate_special_menu(menu: &SpecialContextMenu) -> Result<bool, MenuError> {
    let available = match (&menu.open, menu.kind) {
        (SpecialActivation::OpenDirectory { kind, .. }, SpecialItemKind::Files)
        | (SpecialActivation::OpenDirectory { kind, .. }, SpecialItemKind::Downloads)
            if *kind == menu.kind =>
        {
            true
        }
        (SpecialActivation::OpenTrash, SpecialItemKind::Trash) => true,
        (SpecialActivation::Unavailable { kind, .. }, _) if *kind == menu.kind => false,
        _ => return Err(MenuError::InvalidSpecialAction),
    };
    if let Some(SpecialContextAction::EmptyTrash {
        expected_item_count,
    }) = &menu.empty_trash
    {
        if menu.kind != SpecialItemKind::Trash || !available || *expected_item_count == 0 {
            return Err(MenuError::InvalidSpecialAction);
        }
    }
    Ok(available)
}

fn special_name(kind: SpecialItemKind) -> &'static str {
    match kind {
        SpecialItemKind::Files => "Files",
        SpecialItemKind::Downloads => "Downloads",
        SpecialItemKind::Trash => "Trash",
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
            miniature: None,
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

    fn terminate(app_id: &str, kind: crate::TerminationKind) -> ContextAction {
        ContextAction::TerminateApplication {
            app_id: app_id.into(),
            pids: vec![42],
            kind,
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
                miniature: None,
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
            open: None,
            application_commands: Vec::new(),
            windows: vec![WindowMenu {
                id: rmac_compositor::WindowId(7),
                title: "/home/alex/private — shell".into(),
                focused: true,
                urgent: true,
                focus: focus("terminal.desktop", 7),
                close: close("terminal.desktop", 7),
            }],
            show_in_finder: None,
            pin: PinCommand::Unpin {
                app_id: "terminal.desktop".into(),
            },
            quit: None,
            force_quit: None,
            show_all_windows: None,
            hide: None,
            hide_others: None,
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
            open: None,
            application_commands: Vec::new(),
            windows: Vec::new(),
            show_in_finder: None,
            pin: PinCommand::Pin {
                app_id: "music.desktop".into(),
            },
            quit: None,
            force_quit: None,
            show_all_windows: None,
            hide: None,
            hide_others: None,
        };
        let mut session = Session::context(&menu).unwrap();
        assert_eq!(session.selected(), Some(&RowId::Pin));
        assert_eq!(session.handle_key(KeyCommand::Home), Effect::None);
        assert!(!session.select(&RowId::Open));
        assert_eq!(
            session.handle_key(KeyCommand::Escape),
            Effect::Dismissed {
                restore_focus: EntryId::Application("music.desktop".into())
            }
        );
        assert!(!session.is_open());
    }

    #[test]
    fn catalog_actions_and_option_force_quit_remain_exact() {
        let launch = ContextAction::LaunchNew {
            app_id: "music.desktop".into(),
            spec: rmac_apps::LaunchSpec::Command {
                program: "music".into(),
                args: Vec::new(),
                working_dir: None,
                terminal: false,
            },
        };
        let mut menu = ContextMenu {
            app_id: "music.desktop".into(),
            application_name: "Music".into(),
            open: Some(launch.clone()),
            application_commands: Vec::new(),
            windows: Vec::new(),
            show_in_finder: None,
            pin: PinCommand::Pin {
                app_id: "music.desktop".into(),
            },
            quit: None,
            force_quit: None,
            show_all_windows: None,
            hide: None,
            hide_others: None,
        };
        let closed = Session::context(&menu).unwrap();
        let last = closed.rows().last().unwrap();
        assert_eq!(last.id, RowId::Open);
        assert_eq!(last.label, "Open");
        assert_eq!(last.section, Section::Lifecycle);

        menu.open = None;
        menu.application_commands = vec![crate::ApplicationCommand {
            id: "new-window".into(),
            name: "New Window".into(),
            action: launch,
        }];
        menu.quit = Some(terminate("music.desktop", crate::TerminationKind::Quit));
        menu.force_quit = Some(terminate(
            "music.desktop",
            crate::TerminationKind::ForceQuit,
        ));
        let mut running = Session::context(&menu).unwrap();
        assert_eq!(running.rows()[0].label, "New Window");
        assert!(running.select(&RowId::Quit));
        assert_eq!(
            running.handle_key(KeyCommand::AlternateReturn),
            Effect::Activate {
                action: Action::Context(terminate(
                    "music.desktop",
                    crate::TerminationKind::ForceQuit,
                )),
                restore_focus: EntryId::Application("music.desktop".into()),
            }
        );
    }

    fn mac_menu(pin: PinCommand, windows: Vec<WindowMenu>, quit: bool) -> ContextMenu {
        let visible = windows.iter().map(|window| window.id).collect::<Vec<_>>();
        let first = visible.first().copied();
        ContextMenu {
            app_id: "terminal.desktop".into(),
            application_name: "Terminal".into(),
            open: (windows.is_empty() && !quit).then(|| ContextAction::LaunchNew {
                app_id: "terminal.desktop".into(),
                spec: rmac_apps::LaunchSpec::Command {
                    program: "terminal".into(),
                    args: Vec::new(),
                    working_dir: None,
                    terminal: false,
                },
            }),
            application_commands: vec![crate::ApplicationCommand {
                id: "new-window".into(),
                name: "New Window".into(),
                action: ContextAction::LaunchNew {
                    app_id: "terminal.desktop".into(),
                    spec: rmac_apps::LaunchSpec::Command {
                        program: "terminal".into(),
                        args: vec!["--new-window".into()],
                        working_dir: None,
                        terminal: false,
                    },
                },
            }],
            windows,
            show_in_finder: Some(ContextAction::RevealApplication {
                app_id: "terminal.desktop".into(),
                source: "/apps/terminal.desktop".into(),
            }),
            pin,
            quit: quit.then(|| terminate("terminal.desktop", crate::TerminationKind::Quit)),
            force_quit: quit
                .then(|| terminate("terminal.desktop", crate::TerminationKind::ForceQuit)),
            show_all_windows: first.map(|window| ContextAction::ShowAllWindows {
                app_id: "terminal.desktop".into(),
                window,
            }),
            hide: first.map(|_| ContextAction::HideApplication {
                app_id: "terminal.desktop".into(),
                windows: visible.clone(),
            }),
            hide_others: first.map(|_| ContextAction::HideOthers {
                app_id: "terminal.desktop".into(),
                windows: vec![rmac_compositor::WindowId(9)],
            }),
        }
    }

    fn window_menu(id: u64, focused: bool) -> WindowMenu {
        WindowMenu {
            id: rmac_compositor::WindowId(id),
            title: format!("Window {id}"),
            focused,
            urgent: false,
            focus: focus("terminal.desktop", id),
            close: close("terminal.desktop", id),
        }
    }

    #[test]
    fn running_kept_app_menu_follows_the_macos_order() {
        let menu = mac_menu(
            PinCommand::Unpin {
                app_id: "terminal.desktop".into(),
            },
            vec![window_menu(2, true), window_menu(1, false)],
            true,
        );
        let session = Session::context(&menu).unwrap();
        let rows = session.rows();
        assert_eq!(
            rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>(),
            [
                RowId::Window(rmac_compositor::WindowId(2)),
                RowId::Window(rmac_compositor::WindowId(1)),
                RowId::ApplicationCommand("new-window".into()),
                RowId::Pin,
                RowId::ShowInFinder,
                RowId::ShowAllWindows,
                RowId::Hide,
                RowId::Quit,
            ]
        );
        // The current window is ticked; Options holds a ticked Keep in Dock.
        assert!(rows[0].checked && !rows[1].checked);
        assert_eq!(rows[3].label, "Keep in Dock");
        assert!(rows[3].checked);
        assert_eq!(rows[3].submenu, Some(Submenu::Options));
        assert_eq!(rows[4].submenu, Some(Submenu::Options));
        assert_eq!(Submenu::Options.label(), "Options");
        // Show All Windows and Hide sit with Quit; Option turns Hide into
        // Hide Others and Quit into Force Quit.
        assert_eq!(rows[5].label, "Show All Windows");
        assert_eq!(rows[6].label, "Hide");
        assert_eq!(rows[6].alternate_label.as_deref(), Some("Hide Others"));
        assert!(matches!(
            rows[6].secondary,
            Some(Action::Context(ContextAction::HideOthers { .. }))
        ));
        assert!(rows[5..]
            .iter()
            .all(|row| row.section == Section::Lifecycle));
        assert_eq!(rows[7].label, "Quit");
        assert_eq!(rows[7].alternate_label.as_deref(), Some("Force Quit"));
        assert_eq!(
            rows[7].secondary,
            Some(Action::Context(terminate(
                "terminal.desktop",
                crate::TerminationKind::ForceQuit
            )))
        );
    }

    #[test]
    fn closed_kept_app_offers_remove_from_dock_and_open_last() {
        let menu = mac_menu(
            PinCommand::Unpin {
                app_id: "terminal.desktop".into(),
            },
            Vec::new(),
            false,
        );
        let session = Session::context(&menu).unwrap();
        let rows = session.rows();
        let pin = rows.iter().find(|row| row.id == RowId::Pin).unwrap();
        assert_eq!(pin.label, "Remove from Dock");
        assert!(!pin.checked);
        assert_eq!(rows.last().unwrap().id, RowId::Open);
        assert!(rows.iter().all(|row| row.id != RowId::Quit));
    }

    #[test]
    fn running_app_not_in_the_dock_offers_an_unticked_keep_in_dock() {
        let menu = mac_menu(
            PinCommand::Pin {
                app_id: "terminal.desktop".into(),
            },
            vec![window_menu(1, false)],
            true,
        );
        let session = Session::context(&menu).unwrap();
        let pin = session
            .rows()
            .iter()
            .find(|row| row.id == RowId::Pin)
            .unwrap();
        assert_eq!(pin.label, "Keep in Dock");
        assert!(!pin.checked);
    }

    #[test]
    fn empty_trash_menu_keeps_a_disabled_empty_trash_row() {
        let menu = SpecialContextMenu {
            kind: SpecialItemKind::Trash,
            open: SpecialActivation::OpenTrash,
            empty_trash: None,
        };
        let session = Session::special(&menu).unwrap();
        let rows = session.rows();
        assert_eq!(rows[0].label, "Open");
        assert_eq!(rows[1].id, RowId::EmptyTrash);
        assert_eq!(rows[1].label, "Empty Trash");
        assert!(!rows[1].enabled);
        assert!(rows[1].primary.is_none());
        assert_eq!(
            session.selected(),
            Some(&RowId::OpenSpecial(SpecialItemKind::Trash))
        );
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

    #[test]
    fn nonempty_trash_has_a_distinct_reviewed_destructive_action() {
        let menu = SpecialContextMenu {
            kind: SpecialItemKind::Trash,
            open: SpecialActivation::OpenTrash,
            empty_trash: Some(SpecialContextAction::EmptyTrash {
                expected_item_count: 4,
            }),
        };
        let mut session = Session::special(&menu).unwrap();

        assert_eq!(
            session.selected(),
            Some(&RowId::OpenSpecial(SpecialItemKind::Trash))
        );
        assert_eq!(session.rows().len(), 2);
        assert!(!session.rows()[0].destructive);
        assert_eq!(session.rows()[1].section, Section::Destructive);
        assert!(session.rows()[1].destructive);
        assert!(session.rows()[1].accessible_label.contains("4 items"));
        assert_eq!(
            session.handle_key(KeyCommand::ArrowDown),
            Effect::SelectionChanged
        );
        assert_eq!(session.selected(), Some(&RowId::EmptyTrash));
        assert_eq!(
            session.handle_key(KeyCommand::Return),
            Effect::Activate {
                action: Action::SpecialContext(SpecialContextAction::EmptyTrash {
                    expected_item_count: 4,
                }),
                restore_focus: EntryId::Special(SpecialItemKind::Trash),
            }
        );
    }

    #[test]
    fn special_menu_never_copies_a_private_directory_into_its_actions_or_debug() {
        let menu = SpecialContextMenu {
            kind: SpecialItemKind::Downloads,
            open: SpecialActivation::OpenDirectory {
                kind: SpecialItemKind::Downloads,
                path: std::path::PathBuf::from("/home/alex/Private Downloads"),
            },
            empty_trash: None,
        };
        let mut session = Session::special(&menu).unwrap();

        assert!(!format!("{session:?}").contains("Private Downloads"));
        assert_eq!(
            session.handle_key(KeyCommand::Return),
            Effect::Activate {
                action: Action::ActivateEntry(EntryId::Special(SpecialItemKind::Downloads)),
                restore_focus: EntryId::Special(SpecialItemKind::Downloads),
            }
        );
    }

    #[test]
    fn unavailable_and_malformed_special_menus_fail_safely() {
        let unavailable = SpecialContextMenu {
            kind: SpecialItemKind::Files,
            open: SpecialActivation::Unavailable {
                kind: SpecialItemKind::Files,
                detail: "private backend detail".into(),
            },
            empty_trash: None,
        };
        let mut session = Session::special(&unavailable).unwrap();
        assert_eq!(session.selected(), None);
        assert_eq!(session.handle_key(KeyCommand::Return), Effect::None);
        assert!(!format!("{session:?}").contains("private backend detail"));

        let files_with_empty = SpecialContextMenu {
            kind: SpecialItemKind::Files,
            open: SpecialActivation::OpenDirectory {
                kind: SpecialItemKind::Files,
                path: "/home/alex".into(),
            },
            empty_trash: Some(SpecialContextAction::EmptyTrash {
                expected_item_count: 1,
            }),
        };
        assert_eq!(
            Session::special(&files_with_empty).unwrap_err(),
            MenuError::InvalidSpecialAction
        );

        let zero_count = SpecialContextMenu {
            kind: SpecialItemKind::Trash,
            open: SpecialActivation::OpenTrash,
            empty_trash: Some(SpecialContextAction::EmptyTrash {
                expected_item_count: 0,
            }),
        };
        assert_eq!(
            Session::special(&zero_count).unwrap_err(),
            MenuError::InvalidSpecialAction
        );
    }
}
