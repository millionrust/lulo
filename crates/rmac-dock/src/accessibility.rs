//! Bounded shelf, overflow, and context-menu semantics for one Dock surface.

use std::collections::HashSet;
use std::fmt;

use crate::dock::canonical_app_id;
use crate::menu::{self, RowId};
use crate::presentation::{
    self, ActivityIndicator, BuiltinIcon, Entry, EntryId, Icon, ShelfContent, ShelfLayoutPlan,
};
use crate::{Activation, Model, SpecialActivation, SpecialItemKind};

pub const DOCK_NAME: &str = "Dock";
pub const ROOT_ID: &str = "dock";
pub const OVERFLOW_ID: &str = "dock-more";
pub const FILES_ID: &str = "dock-files";
pub const DOWNLOADS_ID: &str = "dock-downloads";
pub const TRASH_ID: &str = "dock-trash";
pub const MENU_ID: &str = "dock-menu";
pub const ACTIVATE_NAME: &str = "Open";
pub const SHOW_MENU_NAME: &str = "Show Menu";
pub const ACTIVATE_MENU_ITEM_NAME: &str = "Activate";
pub const CLOSE_WINDOW_NAME: &str = "Close Window";
pub const FORCE_QUIT_NAME: &str = "Force Quit";
pub const CLOSE_MENU_NAME: &str = "Close Menu";
pub const MAX_APPLICATIONS: usize = 512;
pub const MAX_PLACES: usize = 3;
pub const MAX_ID_BYTES: usize = 512;
pub const MAX_TEXT_BYTES: usize = 4 * 1024;
pub const MAX_SEMANTIC_TEXT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessibleRole {
    Toolbar,
    Button,
    Menu,
    MenuItem,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShelfGroup {
    Applications,
    Places,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessibleActionKind {
    ActivateShelfItem,
    ShowMenu,
    ActivateMenuItem,
    CloseWindow,
    ForceQuitApplication,
    CloseMenu,
}

#[derive(Clone, Eq, PartialEq)]
pub enum AccessibleActionTarget {
    Shelf(EntryId),
    MenuRow { row: RowId, secondary: bool },
    Menu,
}

impl fmt::Debug for AccessibleActionTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shelf(_) => formatter.write_str("Shelf(<redacted>)"),
            Self::MenuRow { secondary, .. } => formatter
                .debug_struct("MenuRow")
                .field("row", &"<redacted>")
                .field("secondary", secondary)
                .finish(),
            Self::Menu => formatter.write_str("Menu"),
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleAction {
    pub id: String,
    pub name: &'static str,
    pub kind: AccessibleActionKind,
    pub enabled: bool,
    pub target: AccessibleActionTarget,
}

impl fmt::Debug for AccessibleAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleAction")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("enabled", &self.enabled)
            .field("target", &self.target)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleShelfItem {
    pub id: String,
    pub role: AccessibleRole,
    pub name: String,
    pub visible_label: String,
    pub group: ShelfGroup,
    pub position_in_group: usize,
    pub group_size: usize,
    pub position_in_shelf: usize,
    pub shelf_size: usize,
    /// Whether the rendered item has a valid primary presentation action.
    pub visually_enabled: bool,
    /// Whether at least one semantic action is currently invokable.
    pub enabled: bool,
    pub focusable: bool,
    pub activity: ActivityIndicator,
    pub urgent: bool,
    pub badge: Option<usize>,
    pub actions: Vec<AccessibleAction>,
}

impl fmt::Debug for AccessibleShelfItem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleShelfItem")
            .field("id", &self.id)
            .field("role", &self.role)
            .field("name", &"<redacted>")
            .field("visible_label", &"<redacted>")
            .field("group", &self.group)
            .field("position_in_group", &self.position_in_group)
            .field("group_size", &self.group_size)
            .field("position_in_shelf", &self.position_in_shelf)
            .field("shelf_size", &self.shelf_size)
            .field("visually_enabled", &self.visually_enabled)
            .field("enabled", &self.enabled)
            .field("focusable", &self.focusable)
            .field("activity", &self.activity)
            .field("urgent", &self.urgent)
            .field("badge", &self.badge)
            .field("actions", &self.actions)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleMenuItem {
    pub id: String,
    pub role: AccessibleRole,
    pub name: String,
    pub visible_label: String,
    pub section: menu::Section,
    pub position_in_menu: usize,
    pub menu_size: usize,
    pub enabled: bool,
    pub selected: bool,
    pub checked: bool,
    pub urgent: bool,
    pub destructive: bool,
    pub actions: Vec<AccessibleAction>,
}

impl fmt::Debug for AccessibleMenuItem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleMenuItem")
            .field("id", &self.id)
            .field("role", &self.role)
            .field("name", &"<redacted>")
            .field("visible_label", &"<redacted>")
            .field("section", &self.section)
            .field("position_in_menu", &self.position_in_menu)
            .field("menu_size", &self.menu_size)
            .field("enabled", &self.enabled)
            .field("selected", &self.selected)
            .field("checked", &self.checked)
            .field("urgent", &self.urgent)
            .field("destructive", &self.destructive)
            .field("actions", &self.actions)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleMenu {
    pub id: &'static str,
    pub role: AccessibleRole,
    pub name: String,
    pub items: Vec<AccessibleMenuItem>,
    pub reading_order: Vec<String>,
    pub focus_order: Vec<String>,
    pub initial_focus: Option<String>,
    pub restore_focus: String,
    pub dismiss_action: AccessibleAction,
}

impl fmt::Debug for AccessibleMenu {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleMenu")
            .field("id", &self.id)
            .field("role", &self.role)
            .field("name", &"<redacted>")
            .field("items", &self.items)
            .field("reading_order", &self.reading_order)
            .field("focus_order", &self.focus_order)
            .field("initial_focus", &self.initial_focus)
            .field("restore_focus", &self.restore_focus)
            .field("dismiss_action", &self.dismiss_action)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct DockAccessibilitySnapshot {
    pub root_id: &'static str,
    pub role: AccessibleRole,
    pub name: &'static str,
    pub items: Vec<AccessibleShelfItem>,
    pub reading_order: Vec<String>,
    pub focus_order: Vec<String>,
    pub initial_focus: Option<String>,
    pub menu: Option<AccessibleMenu>,
}

impl fmt::Debug for DockAccessibilitySnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DockAccessibilitySnapshot")
            .field("root_id", &self.root_id)
            .field("role", &self.role)
            .field("name", &self.name)
            .field("items", &self.items)
            .field("reading_order", &self.reading_order)
            .field("focus_order", &self.focus_order)
            .field("initial_focus", &self.initial_focus)
            .field("menu", &self.menu)
            .finish()
    }
}

#[derive(Clone, Copy)]
pub struct DockAccessibilityInput<'a> {
    pub model: &'a Model,
    pub content: &'a ShelfContent,
    pub layout: &'a ShelfLayoutPlan,
    pub menu: Option<&'a menu::Session>,
}

impl fmt::Debug for DockAccessibilityInput<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DockAccessibilityInput")
            .field("application_count", &self.content.applications.len())
            .field("place_count", &self.content.places.len())
            .field("visible_count", &self.layout.visible_ids().len())
            .field("has_overflow", &self.layout.overflow.is_some())
            .field("has_menu", &self.menu.is_some())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessibilityProjectionError {
    ApplicationLimit,
    PlaceLimit,
    ModelContentMismatch,
    DuplicateEntry,
    InvalidEntry,
    InvalidLayout,
    InvalidMenu,
    InvalidAction,
    InvalidText,
    TextValueLimit,
    TextLimit,
}

pub fn project_accessibility(
    input: DockAccessibilityInput<'_>,
) -> Result<DockAccessibilitySnapshot, AccessibilityProjectionError> {
    if input.model.items.len() > MAX_APPLICATIONS
        || input.content.applications.len() > MAX_APPLICATIONS
    {
        return Err(AccessibilityProjectionError::ApplicationLimit);
    }
    if input.model.special_items.len() > MAX_PLACES || input.content.places.len() > MAX_PLACES {
        return Err(AccessibilityProjectionError::PlaceLimit);
    }
    if ShelfContent::project(input.model) != *input.content {
        return Err(AccessibilityProjectionError::ModelContentMismatch);
    }

    let mut budget = TextBudget::default();
    for fixed in [
        DOCK_NAME,
        ROOT_ID,
        OVERFLOW_ID,
        FILES_ID,
        DOWNLOADS_ID,
        TRASH_ID,
        MENU_ID,
        ACTIVATE_NAME,
        SHOW_MENU_NAME,
        ACTIVATE_MENU_ITEM_NAME,
        CLOSE_WINDOW_NAME,
        CLOSE_MENU_NAME,
    ] {
        budget.add_required(fixed)?;
    }
    validate_content(input.content, &mut budget)?;
    let visible = validate_layout(input.content, input.layout)?;
    let shelf_size = visible.len();
    let application_group_size = visible
        .iter()
        .filter(|entry| entry.group == ShelfGroup::Applications)
        .count();
    let place_group_size = shelf_size - application_group_size;
    let mut application_position = 0usize;
    let mut place_position = 0usize;
    let mut items = Vec::with_capacity(shelf_size);
    let mut reading_order = Vec::with_capacity(shelf_size);
    let mut focus_order = Vec::with_capacity(shelf_size);
    let mut action_ids = HashSet::with_capacity(shelf_size * 2 + menu::MAX_MENU_ROWS * 2 + 1);

    for (position, visible) in visible.into_iter().enumerate() {
        let group_position = match visible.group {
            ShelfGroup::Applications => {
                application_position += 1;
                application_position
            }
            ShelfGroup::Places => {
                place_position += 1;
                place_position
            }
        };
        let group_size = match visible.group {
            ShelfGroup::Applications => application_group_size,
            ShelfGroup::Places => place_group_size,
        };
        let id = visible.semantic_id;
        budget.add_id(&id)?;
        if matches!(visible.entry.id, EntryId::Overflow) {
            validate_entry(visible.entry, &mut budget)?;
        }
        let menu_enabled = match &visible.entry.id {
            EntryId::Application(_) | EntryId::Overflow => true,
            EntryId::Special(_) => visible.entry.enabled,
            // Minimized tiles restore on activation and have no menu yet.
            EntryId::Minimized(_) => false,
        };
        let mut actions = Vec::with_capacity(2);
        if !matches!(visible.entry.id, EntryId::Overflow) {
            let primary_enabled = match &visible.entry.id {
                EntryId::Application(app_id) => !matches!(
                    input.model.activate(app_id),
                    Activation::NoAction | Activation::Unavailable { .. }
                ),
                EntryId::Special(kind) => {
                    let activation_available = !matches!(
                        input.model.activate_special(*kind),
                        SpecialActivation::Unavailable { .. }
                    );
                    if visible.entry.enabled != activation_available {
                        return Err(AccessibilityProjectionError::InvalidEntry);
                    }
                    activation_available
                }
                EntryId::Minimized(window) => {
                    let available = !matches!(
                        input.model.activate_minimized(*window),
                        Activation::NoAction | Activation::Unavailable { .. }
                    );
                    if visible.entry.enabled != available {
                        return Err(AccessibilityProjectionError::InvalidEntry);
                    }
                    available
                }
                EntryId::Overflow => unreachable!("overflow handled above"),
            };
            actions.push(shelf_action(
                &id,
                ACTIVATE_NAME,
                AccessibleActionKind::ActivateShelfItem,
                primary_enabled,
                visible.entry.id.clone(),
            ));
        }
        actions.push(shelf_action(
            &id,
            SHOW_MENU_NAME,
            AccessibleActionKind::ShowMenu,
            menu_enabled,
            visible.entry.id.clone(),
        ));
        validate_action_ids(&actions, &mut action_ids, &mut budget)?;
        let focusable = actions.iter().any(|action| action.enabled);
        reading_order.push(id.clone());
        if focusable {
            focus_order.push(id.clone());
        }
        items.push(AccessibleShelfItem {
            id,
            role: AccessibleRole::Button,
            name: visible.entry.accessible_label.clone(),
            visible_label: visible.entry.label.clone(),
            group: visible.group,
            position_in_group: group_position,
            group_size,
            position_in_shelf: position + 1,
            shelf_size,
            visually_enabled: visible.entry.enabled,
            enabled: focusable,
            focusable,
            activity: visible.entry.activity,
            urgent: visible.entry.urgent,
            badge: visible.entry.badge,
            actions,
        });
    }

    let shelf_initial_focus = focus_order.first().cloned();
    let menu = input
        .menu
        .map(|menu| {
            validate_menu_authority(input.model, input.layout, menu)?;
            project_menu(menu, &items, &mut budget, &mut action_ids)
        })
        .transpose()?;
    let (focus_order, initial_focus) = menu
        .as_ref()
        .map(|menu| (menu.focus_order.clone(), menu.initial_focus.clone()))
        .unwrap_or((focus_order, shelf_initial_focus));

    Ok(DockAccessibilitySnapshot {
        root_id: ROOT_ID,
        role: AccessibleRole::Toolbar,
        name: DOCK_NAME,
        items,
        reading_order,
        focus_order,
        initial_focus,
        menu,
    })
}

struct VisibleEntry<'a> {
    entry: &'a Entry,
    semantic_id: String,
    group: ShelfGroup,
}

fn validate_content(
    content: &ShelfContent,
    budget: &mut TextBudget,
) -> Result<(), AccessibilityProjectionError> {
    let mut identities = HashSet::with_capacity(content.applications.len());
    for entry in &content.applications {
        let EntryId::Application(app_id) = &entry.id else {
            return Err(AccessibilityProjectionError::InvalidEntry);
        };
        budget.add_id(app_id)?;
        let canonical = canonical_app_id(app_id);
        if canonical.is_empty() || !identities.insert(canonical) {
            return Err(AccessibilityProjectionError::DuplicateEntry);
        }
        validate_entry(entry, budget)?;
    }
    let place_ids = content
        .places
        .iter()
        .map(|entry| entry.id.clone())
        .collect::<Vec<_>>();
    if !place_ids.is_empty()
        && place_ids
            != [
                EntryId::Special(SpecialItemKind::Files),
                EntryId::Special(SpecialItemKind::Downloads),
                EntryId::Special(SpecialItemKind::Trash),
            ]
    {
        return Err(AccessibilityProjectionError::InvalidEntry);
    }
    for entry in &content.places {
        validate_entry(entry, budget)?;
    }
    Ok(())
}

fn validate_entry(
    entry: &Entry,
    budget: &mut TextBudget,
) -> Result<(), AccessibilityProjectionError> {
    budget.add_required(&entry.label)?;
    budget.add_required(&entry.accessible_label)?;
    match (&entry.id, &entry.icon) {
        (EntryId::Application(_), Icon::File(_) | Icon::Builtin(BuiltinIcon::Application)) => {}
        (EntryId::Special(SpecialItemKind::Files), Icon::Builtin(BuiltinIcon::Files)) => {}
        (EntryId::Special(SpecialItemKind::Downloads), Icon::Builtin(BuiltinIcon::Downloads)) => {}
        (EntryId::Special(SpecialItemKind::Trash), Icon::Builtin(BuiltinIcon::TrashEmpty))
        | (EntryId::Special(SpecialItemKind::Trash), Icon::Builtin(BuiltinIcon::TrashFull)) => {}
        (EntryId::Overflow, Icon::Builtin(BuiltinIcon::More)) => {}
        _ => return Err(AccessibilityProjectionError::InvalidEntry),
    }
    if entry.activity == ActivityIndicator::Active && !entry.enabled
        || matches!(entry.id, EntryId::Special(_))
            && (entry.activity != ActivityIndicator::None || entry.urgent)
        || matches!(entry.id, EntryId::Application(_)) && entry.badge.is_some()
        || matches!(entry.id, EntryId::Overflow) && entry.badge.is_none_or(|badge| badge == 0)
    {
        return Err(AccessibilityProjectionError::InvalidEntry);
    }
    Ok(())
}

fn validate_layout<'a>(
    content: &'a ShelfContent,
    layout: &'a ShelfLayoutPlan,
) -> Result<Vec<VisibleEntry<'a>>, AccessibilityProjectionError> {
    let mut expected_ids = Vec::new();
    let visible_application_count = if let Some(overflow) = &layout.overflow {
        if overflow.hidden_applications.is_empty()
            || overflow.hidden_applications.len() > content.applications.len()
        {
            return Err(AccessibilityProjectionError::InvalidLayout);
        }
        let visible_count = content.applications.len() - overflow.hidden_applications.len();
        if overflow.hidden_applications != content.applications[visible_count..]
            || *overflow
                != presentation::overflow_group(content.applications[visible_count..].to_vec())
        {
            return Err(AccessibilityProjectionError::InvalidLayout);
        }
        expected_ids.extend(
            content.applications[..visible_count]
                .iter()
                .map(|entry| entry.id.clone()),
        );
        expected_ids.push(EntryId::Overflow);
        visible_count
    } else {
        expected_ids.extend(content.applications.iter().map(|entry| entry.id.clone()));
        content.applications.len()
    };
    expected_ids.extend(content.places.iter().map(|entry| entry.id.clone()));
    if layout.visible_ids() != expected_ids {
        return Err(AccessibilityProjectionError::InvalidLayout);
    }

    let mut visible = Vec::with_capacity(expected_ids.len());
    for (position, id) in expected_ids.iter().enumerate() {
        match id {
            EntryId::Application(_) => {
                let entry = content
                    .applications
                    .get(position)
                    .ok_or(AccessibilityProjectionError::InvalidLayout)?;
                visible.push(VisibleEntry {
                    entry,
                    semantic_id: format!("dock-application-{position}"),
                    group: ShelfGroup::Applications,
                });
            }
            EntryId::Overflow => {
                let overflow = layout
                    .overflow
                    .as_ref()
                    .ok_or(AccessibilityProjectionError::InvalidLayout)?;
                visible.push(VisibleEntry {
                    entry: &overflow.entry,
                    semantic_id: OVERFLOW_ID.into(),
                    group: ShelfGroup::Applications,
                });
            }
            EntryId::Special(kind) => {
                let entry = content
                    .places
                    .iter()
                    .find(|entry| entry.id == EntryId::Special(*kind))
                    .ok_or(AccessibilityProjectionError::InvalidLayout)?;
                visible.push(VisibleEntry {
                    entry,
                    semantic_id: special_id(*kind).into(),
                    group: ShelfGroup::Places,
                });
            }
            EntryId::Minimized(window) => {
                let entry = content
                    .places
                    .iter()
                    .find(|entry| entry.id == EntryId::Minimized(*window))
                    .ok_or(AccessibilityProjectionError::InvalidLayout)?;
                visible.push(VisibleEntry {
                    entry,
                    semantic_id: format!("dock-minimized-{}", window.0),
                    group: ShelfGroup::Places,
                });
            }
        }
    }
    let projected_application_count = visible
        .iter()
        .filter(|entry| entry.group == ShelfGroup::Applications)
        .count();
    if visible_application_count + usize::from(layout.overflow.is_some())
        != projected_application_count
    {
        return Err(AccessibilityProjectionError::InvalidLayout);
    }
    Ok(visible)
}

fn validate_menu_authority(
    model: &Model,
    layout: &ShelfLayoutPlan,
    session: &menu::Session,
) -> Result<(), AccessibilityProjectionError> {
    let expected = match session.invoker() {
        EntryId::Application(app_id) => model
            .context_menu(app_id)
            .as_ref()
            .and_then(|menu| menu::Session::context(menu).ok()),
        EntryId::Special(kind) => model
            .special_context_menu(*kind)
            .as_ref()
            .and_then(|menu| menu::Session::special(menu).ok()),
        EntryId::Overflow => layout
            .overflow
            .as_ref()
            .and_then(|overflow| menu::Session::overflow(overflow).ok()),
        // A minimized tile never owns a menu session.
        EntryId::Minimized(_) => None,
    }
    .ok_or(AccessibilityProjectionError::InvalidMenu)?;
    if !session.is_open()
        || session.invoker() != expected.invoker()
        || session.title() != expected.title()
        || session.accessible_title() != expected.accessible_title()
        || session.rows() != expected.rows()
    {
        return Err(AccessibilityProjectionError::InvalidMenu);
    }
    Ok(())
}

fn project_menu(
    session: &menu::Session,
    shelf_items: &[AccessibleShelfItem],
    budget: &mut TextBudget,
    action_ids: &mut HashSet<String>,
) -> Result<AccessibleMenu, AccessibilityProjectionError> {
    if !session.is_open() || session.rows().len() > menu::MAX_MENU_ROWS {
        return Err(AccessibilityProjectionError::InvalidMenu);
    }
    budget.add_required(session.title())?;
    budget.add_required(session.accessible_title())?;
    let restore_focus = shelf_items
        .iter()
        .find(|item| {
            item.actions.iter().any(|action| {
                action.kind == AccessibleActionKind::ShowMenu
                    && action.enabled
                    && matches!(
                        &action.target,
                        AccessibleActionTarget::Shelf(target) if target == session.invoker()
                    )
            })
        })
        .map(|item| item.id.clone())
        .ok_or(AccessibilityProjectionError::InvalidMenu)?;
    let selected = session.selected();
    if selected.is_some_and(|selected| {
        !session
            .rows()
            .iter()
            .any(|row| &row.id == selected && row.enabled)
    }) {
        return Err(AccessibilityProjectionError::InvalidMenu);
    }

    let mut items = Vec::with_capacity(session.rows().len());
    let mut reading_order = Vec::with_capacity(session.rows().len());
    let mut focus_order = Vec::with_capacity(session.rows().len());
    let mut selected_id = None;
    for (position, row) in session.rows().iter().enumerate() {
        budget.add_required(&row.label)?;
        budget.add_required(&row.accessible_label)?;
        if row.enabled != row.primary.is_some()
            || row.secondary.is_some() && row.primary.is_none()
            || row.destructive && row.secondary.is_some()
        {
            return Err(AccessibilityProjectionError::InvalidMenu);
        }
        let id = format!("dock-menu-row-{position}");
        budget.add_id(&id)?;
        let mut actions = Vec::with_capacity(2);
        if row.primary.is_some() {
            actions.push(menu_action(
                &id,
                ACTIVATE_MENU_ITEM_NAME,
                AccessibleActionKind::ActivateMenuItem,
                row.id.clone(),
                false,
            ));
        }
        if row.secondary.is_some() {
            let (name, kind) = if row.id == RowId::Quit {
                (FORCE_QUIT_NAME, AccessibleActionKind::ForceQuitApplication)
            } else {
                (CLOSE_WINDOW_NAME, AccessibleActionKind::CloseWindow)
            };
            actions.push(menu_action(&id, name, kind, row.id.clone(), true));
        }
        validate_action_ids(&actions, action_ids, budget)?;
        let is_selected = selected == Some(&row.id);
        if is_selected {
            selected_id = Some(id.clone());
        }
        reading_order.push(id.clone());
        if row.enabled {
            focus_order.push(id.clone());
        }
        items.push(AccessibleMenuItem {
            id,
            role: AccessibleRole::MenuItem,
            name: row.accessible_label.clone(),
            visible_label: row.label.clone(),
            section: row.section,
            position_in_menu: position + 1,
            menu_size: session.rows().len(),
            enabled: row.enabled,
            selected: is_selected,
            checked: row.checked,
            urgent: row.urgent,
            destructive: row.destructive,
            actions,
        });
    }
    if selected_id.is_none() != focus_order.is_empty() {
        return Err(AccessibilityProjectionError::InvalidMenu);
    }
    let dismiss_action = AccessibleAction {
        id: "dock-menu-dismiss".into(),
        name: CLOSE_MENU_NAME,
        kind: AccessibleActionKind::CloseMenu,
        enabled: true,
        target: AccessibleActionTarget::Menu,
    };
    validate_action_ids(std::slice::from_ref(&dismiss_action), action_ids, budget)?;
    Ok(AccessibleMenu {
        id: MENU_ID,
        role: AccessibleRole::Menu,
        name: session.accessible_title().into(),
        items,
        reading_order,
        focus_order,
        initial_focus: selected_id,
        restore_focus,
        dismiss_action,
    })
}

fn shelf_action(
    id: &str,
    name: &'static str,
    kind: AccessibleActionKind,
    enabled: bool,
    target: EntryId,
) -> AccessibleAction {
    let suffix = match kind {
        AccessibleActionKind::ActivateShelfItem => "activate",
        AccessibleActionKind::ShowMenu => "menu",
        AccessibleActionKind::ActivateMenuItem
        | AccessibleActionKind::CloseWindow
        | AccessibleActionKind::ForceQuitApplication
        | AccessibleActionKind::CloseMenu => unreachable!("shelf action kind"),
    };
    AccessibleAction {
        id: format!("{id}-{suffix}"),
        name,
        kind,
        enabled,
        target: AccessibleActionTarget::Shelf(target),
    }
}

fn menu_action(
    id: &str,
    name: &'static str,
    kind: AccessibleActionKind,
    row: RowId,
    secondary: bool,
) -> AccessibleAction {
    AccessibleAction {
        id: format!("{id}-{}", if secondary { "secondary" } else { "activate" }),
        name,
        kind,
        enabled: true,
        target: AccessibleActionTarget::MenuRow { row, secondary },
    }
}

fn validate_action_ids(
    actions: &[AccessibleAction],
    seen: &mut HashSet<String>,
    budget: &mut TextBudget,
) -> Result<(), AccessibilityProjectionError> {
    for action in actions {
        budget.add_id(&action.id)?;
        budget.add_required(action.name)?;
        if !seen.insert(action.id.clone()) {
            return Err(AccessibilityProjectionError::InvalidAction);
        }
    }
    Ok(())
}

const fn special_id(kind: SpecialItemKind) -> &'static str {
    match kind {
        SpecialItemKind::Files => FILES_ID,
        SpecialItemKind::Downloads => DOWNLOADS_ID,
        SpecialItemKind::Trash => TRASH_ID,
    }
}

#[derive(Default)]
struct TextBudget {
    bytes: usize,
}

impl TextBudget {
    fn add_id(&mut self, text: &str) -> Result<(), AccessibilityProjectionError> {
        self.add(text, MAX_ID_BYTES)
    }

    fn add_required(&mut self, text: &str) -> Result<(), AccessibilityProjectionError> {
        self.add(text, MAX_TEXT_BYTES)
    }

    fn add(&mut self, text: &str, max: usize) -> Result<(), AccessibilityProjectionError> {
        if text.trim().is_empty() || text.chars().any(char::is_control) {
            return Err(AccessibilityProjectionError::InvalidText);
        }
        if text.len() > max {
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
    use std::path::PathBuf;

    use super::*;
    use crate::{Item, SpecialActivation, SpecialItem, SurfaceDescription, WindowItem};

    fn app(name: &str, running: bool, launchable: bool) -> Item {
        Item {
            id: format!("{}.desktop", name.to_lowercase().replace(' ', "-")),
            name: name.into(),
            icon: None,
            pinned: true,
            running,
            active: running,
            urgent: false,
            launchable,
            windows: Vec::new(),
            launch: None,
            source: None,
            actions: Vec::new(),
            mime_types: Vec::new(),
        }
    }

    fn place(kind: SpecialItemKind, available: bool) -> SpecialItem {
        SpecialItem {
            kind,
            name: match kind {
                SpecialItemKind::Files => "Files",
                SpecialItemKind::Downloads => "Downloads",
                SpecialItemKind::Trash => "Trash",
            },
            available,
            item_count: (kind == SpecialItemKind::Trash && available).then_some(3),
            activation: if available {
                match kind {
                    SpecialItemKind::Trash => SpecialActivation::OpenTrash,
                    SpecialItemKind::Files | SpecialItemKind::Downloads => {
                        SpecialActivation::OpenDirectory {
                            kind,
                            path: PathBuf::from("/private/place"),
                        }
                    }
                }
            } else {
                SpecialActivation::Unavailable {
                    kind,
                    detail: "private failure".into(),
                }
            },
        }
    }

    fn surface(axis: f64) -> SurfaceDescription {
        SurfaceDescription {
            output: "private-output".into(),
            placement: rmac_shell_settings::DockPlacement::Bottom,
            output_axis_length: axis,
            output_scale: 1.0,
            base_thickness: 64.0,
            maximum_thickness: 80.0,
            exclusive_zone: 64.0,
            reveal_edge_thickness: 0.0,
            keyboard_interactive: false,
            autohide: false,
            overview_visible: false,
            magnification_enabled: true,
            animate: true,
            magnification: Default::default(),
        }
    }

    fn complete_model() -> Model {
        let mut finder = app("Private Finder", true, false);
        finder.windows.push(WindowItem {
            id: rmac_compositor::WindowId(7),
            title: Some("Private document title".into()),
            pid: None,
            focused: true,
            urgent: true,
            focus_timestamp: None,
        });
        Model {
            items: vec![finder, app("Missing", false, false)],
            special_items: vec![
                place(SpecialItemKind::Files, true),
                place(SpecialItemKind::Downloads, false),
                place(SpecialItemKind::Trash, true),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn complete_shelf_projects_groups_state_actions_and_focus_order() {
        let model = complete_model();
        let content = ShelfContent::project(&model);
        let layout = content.prepare_layout(&surface(1200.0)).unwrap();
        let snapshot = project_accessibility(DockAccessibilityInput {
            model: &model,
            content: &content,
            layout: &layout,
            menu: None,
        })
        .unwrap();

        assert_eq!(snapshot.role, AccessibleRole::Toolbar);
        assert_eq!(snapshot.items.len(), 5);
        assert_eq!(snapshot.items[0].activity, ActivityIndicator::Active);
        assert!(snapshot.items[1].focusable);
        assert!(snapshot.items[1].enabled);
        assert!(!snapshot.items[1].visually_enabled);
        assert!(snapshot.items[1].actions[1].enabled);
        assert!(!snapshot.items[3].focusable);
        assert_eq!(snapshot.items[4].badge, Some(3));
        assert_eq!(snapshot.reading_order.len(), 5);
        assert_eq!(snapshot.focus_order.len(), 4);
        assert_eq!(snapshot.initial_focus, Some("dock-application-0".into()));
    }

    #[test]
    fn context_menu_exposes_selection_close_action_and_exact_focus_restoration() {
        let model = complete_model();
        let content = ShelfContent::project(&model);
        let layout = content.prepare_layout(&surface(1200.0)).unwrap();
        let menu_model = model.context_menu("private-finder.desktop").unwrap();
        let menu = menu::Session::context(&menu_model).unwrap();
        let snapshot = project_accessibility(DockAccessibilityInput {
            model: &model,
            content: &content,
            layout: &layout,
            menu: Some(&menu),
        })
        .unwrap();

        let active_focus_order = snapshot.focus_order.clone();
        let active_initial_focus = snapshot.initial_focus.clone();
        let menu = snapshot.menu.unwrap();
        assert_eq!(menu.restore_focus, "dock-application-0");
        assert_eq!(menu.initial_focus, Some("dock-menu-row-0".into()));
        assert_eq!(active_focus_order, menu.focus_order);
        assert_eq!(active_initial_focus, menu.initial_focus);
        assert!(menu.items[0].selected);
        assert!(menu.items[0].checked);
        assert!(menu.items[0].urgent);
        assert_eq!(
            menu.items[0].actions[1].kind,
            AccessibleActionKind::CloseWindow
        );
        assert_eq!(menu.dismiss_action.kind, AccessibleActionKind::CloseMenu);
        assert!(!format!("{menu:?}").contains("Private document title"));
    }

    #[test]
    fn overflow_menu_preserves_hidden_order_and_restores_more_focus() {
        let model = Model {
            items: (0..8)
                .map(|index| app(&format!("Private App {index}"), true, false))
                .collect(),
            ..Default::default()
        };
        let content = ShelfContent::project(&model);
        let layout = content.prepare_layout(&surface(200.0)).unwrap();
        let overflow = layout.overflow.as_ref().expect("narrow Dock uses More");
        let menu = menu::Session::overflow(overflow).unwrap();
        let snapshot = project_accessibility(DockAccessibilityInput {
            model: &model,
            content: &content,
            layout: &layout,
            menu: Some(&menu),
        })
        .unwrap();

        let overflow_item = snapshot
            .items
            .iter()
            .find(|item| item.id == OVERFLOW_ID)
            .unwrap();
        assert_eq!(overflow_item.actions.len(), 1);
        assert_eq!(
            overflow_item.actions[0].kind,
            AccessibleActionKind::ShowMenu
        );
        let menu = snapshot.menu.unwrap();
        assert_eq!(menu.items.len(), overflow.hidden_applications.len());
        assert_eq!(menu.restore_focus, OVERFLOW_ID);
        assert_eq!(menu.initial_focus, menu.focus_order.first().cloned());
        assert!(menu.items.iter().all(|item| item.enabled));
    }

    #[test]
    fn mismatch_duplicate_closed_menu_oversize_and_diagnostics_fail_safely() {
        let model = complete_model();
        let mut content = ShelfContent::project(&model);
        let layout = content.prepare_layout(&surface(1200.0)).unwrap();
        let snapshot = project_accessibility(DockAccessibilityInput {
            model: &model,
            content: &content,
            layout: &layout,
            menu: None,
        })
        .unwrap();
        let diagnostics = format!("{snapshot:?}");
        assert!(!diagnostics.contains("Private Finder"));
        assert!(!diagnostics.contains("private-output"));
        assert!(!diagnostics.contains("/private/place"));

        content.applications[0].accessible_label = "different".into();
        assert_eq!(
            project_accessibility(DockAccessibilityInput {
                model: &model,
                content: &content,
                layout: &layout,
                menu: None,
            }),
            Err(AccessibilityProjectionError::ModelContentMismatch)
        );

        let mut canonical_alias = app("Same", true, false);
        canonical_alias.id = "SAME".into();
        let duplicate_model = Model {
            items: vec![app("Same", true, false), canonical_alias],
            ..Default::default()
        };
        let duplicate_content = ShelfContent::project(&duplicate_model);
        let duplicate_layout = duplicate_content.prepare_layout(&surface(1200.0)).unwrap();
        assert_eq!(
            project_accessibility(DockAccessibilityInput {
                model: &duplicate_model,
                content: &duplicate_content,
                layout: &duplicate_layout,
                menu: None,
            }),
            Err(AccessibilityProjectionError::DuplicateEntry)
        );

        let mut oversized_model = Model {
            items: vec![app("Small", true, false)],
            ..Default::default()
        };
        oversized_model.items[0].name = "x".repeat(MAX_TEXT_BYTES + 1);
        let oversized_content = ShelfContent::project(&oversized_model);
        let oversized_layout = oversized_content.prepare_layout(&surface(1200.0)).unwrap();
        assert_eq!(
            project_accessibility(DockAccessibilityInput {
                model: &oversized_model,
                content: &oversized_content,
                layout: &oversized_layout,
                menu: None,
            }),
            Err(AccessibilityProjectionError::TextValueLimit)
        );

        let mut changed_model = complete_model();
        let stale_menu_model = changed_model
            .context_menu("private-finder.desktop")
            .unwrap();
        let stale_menu = menu::Session::context(&stale_menu_model).unwrap();
        changed_model.items[0].windows[0].title = Some("New authoritative title".into());
        let changed_content = ShelfContent::project(&changed_model);
        let changed_layout = changed_content.prepare_layout(&surface(1200.0)).unwrap();
        assert_eq!(
            project_accessibility(DockAccessibilityInput {
                model: &changed_model,
                content: &changed_content,
                layout: &changed_layout,
                menu: Some(&stale_menu),
            }),
            Err(AccessibilityProjectionError::InvalidMenu)
        );

        let overflow_model = Model {
            items: (0..8)
                .map(|index| app(&format!("App {index}"), true, false))
                .collect(),
            ..Default::default()
        };
        let overflow_content = ShelfContent::project(&overflow_model);
        let overflow_layout = overflow_content.prepare_layout(&surface(200.0)).unwrap();
        let mut closed =
            menu::Session::overflow(overflow_layout.overflow.as_ref().unwrap()).unwrap();
        closed.handle_key(menu::KeyCommand::Escape);
        assert_eq!(
            project_accessibility(DockAccessibilityInput {
                model: &overflow_model,
                content: &overflow_content,
                layout: &overflow_layout,
                menu: Some(&closed),
            }),
            Err(AccessibilityProjectionError::InvalidMenu)
        );
    }
}
