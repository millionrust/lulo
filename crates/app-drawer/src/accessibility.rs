//! Bounded application-grid, category, menu, empty-state, and feedback semantics.

use std::collections::HashSet;

pub const MAX_ACCESSIBLE_APPLICATIONS: usize = 4_096;
pub const MAX_ACCESSIBLE_DECLARED_ACTIONS: usize = 32;
pub const MAX_ACCESSIBLE_APPLICATION_ID_BYTES: usize = 512;
pub const MAX_ACCESSIBLE_APPLICATION_NAME_BYTES: usize = 4 * 1024;
pub const MAX_ACCESSIBLE_ACTION_ID_BYTES: usize = 255;
pub const MAX_ACCESSIBLE_ACTION_NAME_BYTES: usize = 512;
pub const MAX_ACCESSIBLE_TEXT_BYTES: usize = 2 * 1024 * 1024;
pub const APPS_NAME: &str = "Apps";
pub const OPEN_ACTION_NAME: &str = "Open";
pub const SHOW_IN_FOLDER_ACTION_NAME: &str = "Show in Folder";
pub const OPENING_ANNOUNCEMENT: &str = "Opening application…";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationCategory {
    Productivity,
    Internet,
    Media,
    Developer,
    Utilities,
    Games,
    System,
    Other,
}

impl ApplicationCategory {
    pub const ORDER: [Self; 8] = [
        Self::Productivity,
        Self::Internet,
        Self::Media,
        Self::Developer,
        Self::Utilities,
        Self::Games,
        Self::System,
        Self::Other,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Productivity => "Productivity",
            Self::Internet => "Internet",
            Self::Media => "Media",
            Self::Developer => "Developer",
            Self::Utilities => "Utilities",
            Self::Games => "Games",
            Self::System => "System",
            Self::Other => "Other",
        }
    }
}

/// The catalog-facing subset an accessibility adapter is allowed to observe.
/// Source paths, icons, search metadata, and launch specifications deliberately
/// do not cross this boundary.
pub trait ApplicationSemantics {
    fn stable_id(&self) -> &str;
    fn name(&self) -> &str;
    fn generic_name(&self) -> Option<&str>;
    fn category(&self) -> ApplicationCategory;
    fn declared_action_count(&self) -> usize;
    fn declared_action_id(&self, index: usize) -> Option<&str>;
    fn declared_action_name(&self, index: usize) -> Option<&str>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawerView {
    Grid,
    List,
}

impl DrawerView {
    pub const fn collection_name(self) -> &'static str {
        match self {
            Self::Grid => "Application grid",
            Self::List => "Application list",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawerFeedback<'a> {
    Ready,
    Busy,
    Error(&'a str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawerProjectionState<'a> {
    pub view: DrawerView,
    /// `None` represents the visible All category.
    pub filter: Option<ApplicationCategory>,
    pub selected_index: Option<usize>,
    pub search_active: bool,
    pub context_menu_open: bool,
    pub feedback: DrawerFeedback<'a>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibleCategory {
    pub category: Option<ApplicationCategory>,
    pub name: &'static str,
    pub result_count: usize,
    pub selected: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationActionKind {
    Open,
    Declared,
    ShowInFolder,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibleApplicationAction {
    pub id: String,
    pub name: String,
    pub kind: ApplicationActionKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibleApplication {
    pub stable_id: String,
    pub name: String,
    pub generic_name: Option<String>,
    pub label: String,
    pub category: ApplicationCategory,
    pub category_name: &'static str,
    pub position_in_set: usize,
    pub set_size: usize,
    pub selected: bool,
    pub actions: Vec<AccessibleApplicationAction>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibleApplicationMenu {
    pub name: String,
    pub application_id: String,
    pub items: Vec<AccessibleApplicationAction>,
    pub initial_focus: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawerEmptyState {
    EmptyCatalog,
    NoMatches,
}

impl DrawerEmptyState {
    pub const fn title(self) -> &'static str {
        match self {
            Self::EmptyCatalog => "No applications found",
            Self::NoMatches => "No matching applications",
        }
    }

    pub const fn message(self) -> &'static str {
        match self {
            Self::EmptyCatalog => {
                "Install an application or add a visible desktop entry to an XDG application directory"
            }
            Self::NoMatches => "Try another name, keyword, category, or application action",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LivePoliteness {
    Polite,
    Assertive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveAnnouncement {
    pub text: String,
    pub politeness: LivePoliteness,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppDrawerAccessibilitySnapshot {
    pub name: &'static str,
    pub view: DrawerView,
    pub collection_name: &'static str,
    pub application_count: usize,
    pub search_result_count: usize,
    pub visible_result_count: usize,
    /// Whether a nonempty query is active. The query itself remains private.
    pub search_active: bool,
    pub categories: Vec<AccessibleCategory>,
    pub applications: Vec<AccessibleApplication>,
    pub selected_application: Option<usize>,
    pub context_menu: Option<AccessibleApplicationMenu>,
    pub empty_state: Option<DrawerEmptyState>,
    pub announcement: Option<LiveAnnouncement>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessibilityProjectionError {
    ApplicationLimit,
    InvalidApplication,
    DuplicateApplication,
    InvalidIndices,
    InvalidFilter,
    InvalidSelection,
    ActionLimit,
    InvalidAction,
    DuplicateAction,
    InvalidFeedback,
    TextLimit,
}

/// Project the exact controller-owned search and visible indices into a model
/// suitable for a future GPUI/AT-SPI adapter. Neither search matching nor
/// category filtering is repeated or guessed by this boundary.
pub fn project_app_drawer<T: ApplicationSemantics>(
    applications: &[T],
    search_matches: &[usize],
    visible: &[usize],
    state: DrawerProjectionState<'_>,
) -> Result<AppDrawerAccessibilitySnapshot, AccessibilityProjectionError> {
    if applications.len() > MAX_ACCESSIBLE_APPLICATIONS {
        return Err(AccessibilityProjectionError::ApplicationLimit);
    }
    validate_applications(applications)?;
    validate_indices(search_matches, applications.len())?;
    validate_indices(visible, applications.len())?;

    let expected_visible = search_matches
        .iter()
        .copied()
        .filter(|index| {
            state
                .filter
                .is_none_or(|category| applications[*index].category() == category)
        })
        .collect::<Vec<_>>();
    if expected_visible != visible {
        return Err(AccessibilityProjectionError::InvalidFilter);
    }
    if state.filter.is_some_and(|category| {
        !search_matches
            .iter()
            .any(|index| applications[*index].category() == category)
    }) {
        return Err(AccessibilityProjectionError::InvalidFilter);
    }
    match (visible.is_empty(), state.selected_index) {
        (true, None) => {}
        (false, Some(index)) if index < visible.len() => {}
        _ => return Err(AccessibilityProjectionError::InvalidSelection),
    }
    if state.context_menu_open && state.selected_index.is_none() {
        return Err(AccessibilityProjectionError::InvalidSelection);
    }

    let mut budget = TextBudget::default();
    budget.add(APPS_NAME)?;
    budget.add(state.view.collection_name())?;

    let mut category_counts = [0usize; ApplicationCategory::ORDER.len()];
    for index in search_matches {
        let category = applications[*index].category();
        let position = ApplicationCategory::ORDER
            .iter()
            .position(|candidate| *candidate == category)
            .expect("every application category appears in ORDER");
        category_counts[position] = category_counts[position].saturating_add(1);
    }
    budget.add("All")?;
    let mut categories = vec![AccessibleCategory {
        category: None,
        name: "All",
        result_count: search_matches.len(),
        selected: state.filter.is_none(),
    }];
    for (category, result_count) in ApplicationCategory::ORDER
        .into_iter()
        .zip(category_counts)
        .filter(|(_, count)| *count > 0)
    {
        budget.add(category.label())?;
        categories.push(AccessibleCategory {
            category: Some(category),
            name: category.label(),
            result_count,
            selected: state.filter == Some(category),
        });
    }

    let mut projected = Vec::with_capacity(visible.len());
    for (position, index) in visible.iter().copied().enumerate() {
        let application = &applications[index];
        let stable_id = application.stable_id().to_string();
        let name = application.name().to_string();
        let generic_name = application.generic_name().map(str::to_string);
        let label = generic_name
            .as_deref()
            .filter(|generic| *generic != name.as_str())
            .map_or_else(|| name.clone(), |generic| format!("{name}, {generic}"));
        budget.add(&stable_id)?;
        budget.add(&name)?;
        if let Some(generic_name) = &generic_name {
            budget.add(generic_name)?;
        }
        budget.add(&label)?;
        budget.add(application.category().label())?;
        let actions = project_actions(application, &mut budget)?;
        projected.push(AccessibleApplication {
            stable_id,
            name,
            generic_name,
            label,
            category: application.category(),
            category_name: application.category().label(),
            position_in_set: position + 1,
            set_size: visible.len(),
            selected: state.selected_index == Some(position),
            actions,
        });
    }

    let context_menu = if state.context_menu_open {
        let selected = state
            .selected_index
            .and_then(|index| projected.get(index))
            .ok_or(AccessibilityProjectionError::InvalidSelection)?;
        let name = format!("{} actions", selected.name);
        budget.add(&name)?;
        budget.add(&selected.stable_id)?;
        for action in &selected.actions {
            budget.add(&action.id)?;
            budget.add(&action.name)?;
        }
        Some(AccessibleApplicationMenu {
            name,
            application_id: selected.stable_id.clone(),
            items: selected.actions.clone(),
            initial_focus: 0,
        })
    } else {
        None
    };

    let empty_state = visible.is_empty().then_some(if applications.is_empty() {
        DrawerEmptyState::EmptyCatalog
    } else {
        DrawerEmptyState::NoMatches
    });
    if let Some(empty_state) = empty_state {
        budget.add(empty_state.title())?;
        budget.add(empty_state.message())?;
    }
    let announcement = match state.feedback {
        DrawerFeedback::Ready => None,
        DrawerFeedback::Busy => {
            let text = OPENING_ANNOUNCEMENT.to_string();
            budget.add(&text)?;
            Some(LiveAnnouncement {
                text,
                politeness: LivePoliteness::Polite,
            })
        }
        DrawerFeedback::Error(text) => {
            if !valid_text(text, MAX_ACCESSIBLE_APPLICATION_NAME_BYTES) {
                return Err(AccessibilityProjectionError::InvalidFeedback);
            }
            budget.add(text)?;
            Some(LiveAnnouncement {
                text: text.to_string(),
                politeness: LivePoliteness::Assertive,
            })
        }
    };

    Ok(AppDrawerAccessibilitySnapshot {
        name: APPS_NAME,
        view: state.view,
        collection_name: state.view.collection_name(),
        application_count: applications.len(),
        search_result_count: search_matches.len(),
        visible_result_count: visible.len(),
        search_active: state.search_active,
        categories,
        applications: projected,
        selected_application: state.selected_index,
        context_menu,
        empty_state,
        announcement,
    })
}

fn validate_applications<T: ApplicationSemantics>(
    applications: &[T],
) -> Result<(), AccessibilityProjectionError> {
    let mut application_ids = HashSet::with_capacity(applications.len());
    for application in applications {
        if !valid_id(application.stable_id(), MAX_ACCESSIBLE_APPLICATION_ID_BYTES)
            || !valid_text(application.name(), MAX_ACCESSIBLE_APPLICATION_NAME_BYTES)
            || application
                .generic_name()
                .is_some_and(|name| !valid_text(name, MAX_ACCESSIBLE_APPLICATION_NAME_BYTES))
        {
            return Err(AccessibilityProjectionError::InvalidApplication);
        }
        if !application_ids.insert(application.stable_id()) {
            return Err(AccessibilityProjectionError::DuplicateApplication);
        }
        if application.declared_action_count() > MAX_ACCESSIBLE_DECLARED_ACTIONS {
            return Err(AccessibilityProjectionError::ActionLimit);
        }
        let mut action_ids = HashSet::with_capacity(application.declared_action_count());
        for index in 0..application.declared_action_count() {
            let id = application
                .declared_action_id(index)
                .ok_or(AccessibilityProjectionError::InvalidAction)?;
            let name = application
                .declared_action_name(index)
                .ok_or(AccessibilityProjectionError::InvalidAction)?;
            if !valid_action_id(id) || !valid_text(name, MAX_ACCESSIBLE_ACTION_NAME_BYTES) {
                return Err(AccessibilityProjectionError::InvalidAction);
            }
            if !action_ids.insert(id) {
                return Err(AccessibilityProjectionError::DuplicateAction);
            }
        }
    }
    Ok(())
}

fn validate_indices(
    indices: &[usize],
    application_count: usize,
) -> Result<(), AccessibilityProjectionError> {
    let mut seen = vec![false; application_count];
    for index in indices {
        let Some(slot) = seen.get_mut(*index) else {
            return Err(AccessibilityProjectionError::InvalidIndices);
        };
        if std::mem::replace(slot, true) {
            return Err(AccessibilityProjectionError::InvalidIndices);
        }
    }
    Ok(())
}

fn project_actions<T: ApplicationSemantics>(
    application: &T,
    budget: &mut TextBudget,
) -> Result<Vec<AccessibleApplicationAction>, AccessibilityProjectionError> {
    let mut actions = Vec::with_capacity(application.declared_action_count() + 2);
    push_action(
        &mut actions,
        budget,
        "open".to_string(),
        OPEN_ACTION_NAME.to_string(),
        ApplicationActionKind::Open,
    )?;
    for index in 0..application.declared_action_count() {
        let source_id = application
            .declared_action_id(index)
            .ok_or(AccessibilityProjectionError::InvalidAction)?;
        let name = application
            .declared_action_name(index)
            .ok_or(AccessibilityProjectionError::InvalidAction)?;
        push_action(
            &mut actions,
            budget,
            format!("desktop-action:{source_id}"),
            name.to_string(),
            ApplicationActionKind::Declared,
        )?;
    }
    push_action(
        &mut actions,
        budget,
        "show-in-folder".to_string(),
        SHOW_IN_FOLDER_ACTION_NAME.to_string(),
        ApplicationActionKind::ShowInFolder,
    )?;
    Ok(actions)
}

fn push_action(
    actions: &mut Vec<AccessibleApplicationAction>,
    budget: &mut TextBudget,
    id: String,
    name: String,
    kind: ApplicationActionKind,
) -> Result<(), AccessibilityProjectionError> {
    budget.add(&id)?;
    budget.add(&name)?;
    actions.push(AccessibleApplicationAction { id, name, kind });
    Ok(())
}

fn valid_id(value: &str, limit: usize) -> bool {
    !value.is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}

fn valid_action_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ACCESSIBLE_ACTION_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn valid_text(value: &str, limit: usize) -> bool {
    !value.trim().is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
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

    struct Action {
        id: String,
        name: String,
    }

    struct Application {
        id: String,
        name: String,
        generic_name: Option<String>,
        category: ApplicationCategory,
        actions: Vec<Action>,
    }

    impl ApplicationSemantics for Application {
        fn stable_id(&self) -> &str {
            &self.id
        }

        fn name(&self) -> &str {
            &self.name
        }

        fn generic_name(&self) -> Option<&str> {
            self.generic_name.as_deref()
        }

        fn category(&self) -> ApplicationCategory {
            self.category
        }

        fn declared_action_count(&self) -> usize {
            self.actions.len()
        }

        fn declared_action_id(&self, index: usize) -> Option<&str> {
            self.actions.get(index).map(|action| action.id.as_str())
        }

        fn declared_action_name(&self, index: usize) -> Option<&str> {
            self.actions.get(index).map(|action| action.name.as_str())
        }
    }

    fn app(
        id: &str,
        name: &str,
        generic_name: Option<&str>,
        category: ApplicationCategory,
    ) -> Application {
        Application {
            id: id.to_string(),
            name: name.to_string(),
            generic_name: generic_name.map(str::to_string),
            category,
            actions: vec![Action {
                id: "new-window".to_string(),
                name: "New Window".to_string(),
            }],
        }
    }

    #[test]
    fn exact_catalog_order_selection_categories_and_menu_are_projected() {
        let applications = vec![
            app(
                "firefox.desktop",
                "Firefox",
                None,
                ApplicationCategory::Internet,
            ),
            app(
                "code.desktop",
                "Code",
                Some("Code Editor"),
                ApplicationCategory::Developer,
            ),
        ];
        let snapshot = project_app_drawer(
            &applications,
            &[0, 1],
            &[0, 1],
            DrawerProjectionState {
                view: DrawerView::Grid,
                filter: None,
                selected_index: Some(1),
                search_active: true,
                context_menu_open: true,
                feedback: DrawerFeedback::Ready,
            },
        )
        .unwrap();

        assert_eq!(snapshot.collection_name, "Application grid");
        assert_eq!(snapshot.application_count, 2);
        assert_eq!(snapshot.search_result_count, 2);
        assert!(snapshot.search_active);
        assert_eq!(snapshot.categories[0].name, "All");
        assert_eq!(snapshot.categories[0].result_count, 2);
        assert_eq!(snapshot.categories[1].name, "Internet");
        assert_eq!(snapshot.categories[2].name, "Developer");
        assert_eq!(snapshot.applications[0].stable_id, "firefox.desktop");
        assert_eq!(snapshot.applications[1].label, "Code, Code Editor");
        assert!(!snapshot.applications[0].selected);
        assert!(snapshot.applications[1].selected);
        assert_eq!(snapshot.applications[1].position_in_set, 2);
        assert_eq!(snapshot.applications[1].set_size, 2);
        assert_eq!(
            snapshot.applications[1]
                .actions
                .iter()
                .map(|action| action.name.as_str())
                .collect::<Vec<_>>(),
            ["Open", "New Window", "Show in Folder"]
        );
        let menu = snapshot.context_menu.unwrap();
        assert_eq!(menu.name, "Code actions");
        assert_eq!(menu.application_id, "code.desktop");
        assert_eq!(menu.initial_focus, 0);
        assert_eq!(menu.items, snapshot.applications[1].actions);
        assert_eq!(snapshot.empty_state, None);
        assert_eq!(snapshot.announcement, None);
    }

    #[test]
    fn empty_no_match_busy_and_error_states_remain_distinct() {
        let empty = project_app_drawer::<Application>(
            &[],
            &[],
            &[],
            DrawerProjectionState {
                view: DrawerView::List,
                filter: None,
                selected_index: None,
                search_active: false,
                context_menu_open: false,
                feedback: DrawerFeedback::Busy,
            },
        )
        .unwrap();
        assert_eq!(empty.empty_state, Some(DrawerEmptyState::EmptyCatalog));
        assert_eq!(
            empty.announcement,
            Some(LiveAnnouncement {
                text: "Opening application…".to_string(),
                politeness: LivePoliteness::Polite,
            })
        );

        let applications = vec![app(
            "terminal.desktop",
            "Terminal",
            None,
            ApplicationCategory::System,
        )];
        let no_match = project_app_drawer(
            &applications,
            &[],
            &[],
            DrawerProjectionState {
                view: DrawerView::List,
                filter: None,
                selected_index: None,
                search_active: true,
                context_menu_open: false,
                feedback: DrawerFeedback::Error("Could not open application"),
            },
        )
        .unwrap();
        assert_eq!(no_match.empty_state, Some(DrawerEmptyState::NoMatches));
        assert_eq!(
            no_match.announcement.unwrap().politeness,
            LivePoliteness::Assertive
        );
    }

    #[test]
    fn inconsistent_or_unbounded_authority_inputs_fail_closed() {
        let applications = vec![
            app(
                "browser.desktop",
                "Browser",
                None,
                ApplicationCategory::Internet,
            ),
            app(
                "editor.desktop",
                "Editor",
                None,
                ApplicationCategory::Developer,
            ),
        ];
        let state = DrawerProjectionState {
            view: DrawerView::Grid,
            filter: Some(ApplicationCategory::Developer),
            selected_index: Some(0),
            search_active: false,
            context_menu_open: false,
            feedback: DrawerFeedback::Ready,
        };
        assert_eq!(
            project_app_drawer(&applications, &[0, 1], &[0], state),
            Err(AccessibilityProjectionError::InvalidFilter)
        );

        let duplicates = vec![
            app("same.desktop", "One", None, ApplicationCategory::Other),
            app("same.desktop", "Two", None, ApplicationCategory::Other),
        ];
        assert_eq!(
            project_app_drawer(
                &duplicates,
                &[0, 1],
                &[0, 1],
                DrawerProjectionState {
                    filter: None,
                    selected_index: Some(0),
                    ..state
                },
            ),
            Err(AccessibilityProjectionError::DuplicateApplication)
        );

        let mut too_many_actions = app("many.desktop", "Many", None, ApplicationCategory::Other);
        too_many_actions.actions = (0..=MAX_ACCESSIBLE_DECLARED_ACTIONS)
            .map(|index| Action {
                id: format!("action-{index}"),
                name: format!("Action {index}"),
            })
            .collect();
        assert_eq!(
            project_app_drawer(
                &[too_many_actions],
                &[0],
                &[0],
                DrawerProjectionState {
                    filter: None,
                    selected_index: Some(0),
                    ..state
                },
            ),
            Err(AccessibilityProjectionError::ActionLimit)
        );
    }
}
