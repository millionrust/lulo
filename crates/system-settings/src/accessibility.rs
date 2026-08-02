//! Bounded sidebar, search, account, detail, subpage, and global-feedback semantics.

use std::collections::HashSet;
use std::fmt;

pub const SETTINGS_NAME: &str = "System Settings";
pub const ROOT_ID: &str = "system-settings";
pub const SEARCH_ID: &str = "settings-search";
pub const SEARCH_NAME: &str = "Search";
pub const SIDEBAR_ID: &str = "sidebar-scroll";
pub const DETAIL_ID: &str = "detail-scroll";
pub const BACK_ID: &str = "nav-back";
pub const BACK_NAME: &str = "Back";
pub const LOCAL_ACCOUNT_LABEL: &str = "Local Account";
pub const GLOBAL_ERROR_ID: &str = "settings-error";
pub const GLOBAL_ERROR_TITLE: &str = "Settings error";
pub const GLOBAL_ERROR_DISMISS_ID: &str = "settings-error-dismiss";
pub const DISMISS_NAME: &str = "Dismiss";
pub const MAX_NAVIGATION_SECTIONS: usize = 8;
pub const MAX_NAVIGATION_CATEGORIES: usize = 64;
pub const MAX_NAVIGATION_DEPTH: usize = 8;
pub const MAX_QUERY_BYTES: usize = 4 * 1024;
pub const MAX_LABEL_BYTES: usize = 4 * 1024;
pub const MAX_ERROR_BYTES: usize = 16 * 1024;
pub const MAX_SEMANTIC_TEXT_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NavigationCategoryInput<'a> {
    pub pane_id: &'a str,
    pub name: &'a str,
    pub description: &'a str,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct NavigationInput<'a> {
    pub sections: &'a [Vec<NavigationCategoryInput<'a>>],
    pub selected: (usize, usize),
    pub query: &'a str,
    pub account_name: &'a str,
    pub subpage_title: Option<&'a str>,
    pub back_depth: usize,
    pub global_error: Option<&'a str>,
}

impl fmt::Debug for NavigationInput<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NavigationInput")
            .field("section_count", &self.sections.len())
            .field("selected", &self.selected)
            .field("query", &"<redacted>")
            .field("account_name", &"<redacted>")
            .field("has_subpage", &self.subpage_title.is_some())
            .field("back_depth", &self.back_depth)
            .field("has_global_error", &self.global_error.is_some())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigationActionKind {
    SelectPane,
    Back,
    DismissError,
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleNavigationAction {
    pub id: String,
    pub name: String,
    pub kind: NavigationActionKind,
    pub enabled: bool,
}

impl fmt::Debug for AccessibleNavigationAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleNavigationAction")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("enabled", &self.enabled)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleNavigationItem {
    pub pane_id: String,
    pub name: String,
    pub description: String,
    pub selected: bool,
    pub position_in_section: usize,
    pub section_size: usize,
    pub position_in_sidebar: usize,
    pub sidebar_size: usize,
    pub action: AccessibleNavigationAction,
}

impl fmt::Debug for AccessibleNavigationItem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleNavigationItem")
            .field("pane_id", &self.pane_id)
            .field("name", &self.name)
            .field("description", &self.description)
            .field("selected", &self.selected)
            .field("position_in_section", &self.position_in_section)
            .field("section_size", &self.section_size)
            .field("position_in_sidebar", &self.position_in_sidebar)
            .field("sidebar_size", &self.sidebar_size)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccessibleNavigationSection {
    pub position_in_set: usize,
    pub set_size: usize,
    pub items: Vec<AccessibleNavigationItem>,
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleSearchField {
    pub id: &'static str,
    pub name: &'static str,
    pub value: String,
    pub result_count: usize,
    pub controls: &'static str,
}

impl fmt::Debug for AccessibleSearchField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleSearchField")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("value", &"<redacted>")
            .field("result_count", &self.result_count)
            .field("controls", &self.controls)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleAccount {
    pub name: String,
    pub description: &'static str,
}

impl fmt::Debug for AccessibleAccount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleAccount")
            .field("name", &"<redacted>")
            .field("description", &self.description)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleDetail {
    pub pane_id: String,
    pub pane_name: String,
    pub pane_description: String,
    pub heading: String,
    pub subpage_title: Option<String>,
    pub back_depth: usize,
}

impl fmt::Debug for AccessibleDetail {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleDetail")
            .field("pane_id", &self.pane_id)
            .field("pane_name", &self.pane_name)
            .field("pane_description", &self.pane_description)
            .field("heading", &"<redacted>")
            .field("has_subpage", &self.subpage_title.is_some())
            .field("back_depth", &self.back_depth)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LivePoliteness {
    Polite,
    Assertive,
}

#[derive(Clone, Eq, PartialEq)]
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
            .field("text", &"<redacted>")
            .field("politeness", &self.politeness)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct SettingsNavigationAccessibilitySnapshot {
    pub name: &'static str,
    pub root_id: &'static str,
    pub search: AccessibleSearchField,
    pub account: AccessibleAccount,
    pub sidebar_id: &'static str,
    pub sections: Vec<AccessibleNavigationSection>,
    pub selected_pane_id: String,
    pub selected_visible: bool,
    pub detail_id: &'static str,
    pub detail: AccessibleDetail,
    pub back_action: Option<AccessibleNavigationAction>,
    pub error_action: Option<AccessibleNavigationAction>,
    /// Shell-chrome controls only; pane-owned controls append their own order.
    pub navigation_focus_order: Vec<String>,
    pub initial_focus: String,
    pub announcements: Vec<LiveAnnouncement>,
}

impl fmt::Debug for SettingsNavigationAccessibilitySnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SettingsNavigationAccessibilitySnapshot")
            .field("search", &self.search)
            .field("account", &self.account)
            .field("visible_section_count", &self.sections.len())
            .field(
                "visible_category_count",
                &self
                    .sections
                    .iter()
                    .map(|section| section.items.len())
                    .sum::<usize>(),
            )
            .field("selected_pane_id", &self.selected_pane_id)
            .field("selected_visible", &self.selected_visible)
            .field("detail", &self.detail)
            .field("has_back_action", &self.back_action.is_some())
            .field("has_error_action", &self.error_action.is_some())
            .field("navigation_focus_order", &self.navigation_focus_order)
            .field("initial_focus", &self.initial_focus)
            .field("announcement_count", &self.announcements.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessibilityProjectionError {
    SectionLimit,
    CategoryLimit,
    NavigationDepth,
    InvalidSelection,
    InvalidCategory,
    DuplicateCategory,
    InvalidSubpage,
    InvalidAction,
    InvalidText,
    TextValueLimit,
    TextLimit,
}

pub fn project_settings_navigation(
    input: NavigationInput<'_>,
) -> Result<SettingsNavigationAccessibilitySnapshot, AccessibilityProjectionError> {
    if input.sections.is_empty() || input.sections.len() > MAX_NAVIGATION_SECTIONS {
        return Err(AccessibilityProjectionError::SectionLimit);
    }
    let category_count = input.sections.iter().map(Vec::len).sum::<usize>();
    if category_count == 0 || category_count > MAX_NAVIGATION_CATEGORIES {
        return Err(AccessibilityProjectionError::CategoryLimit);
    }
    let selected = input
        .sections
        .get(input.selected.0)
        .and_then(|section| section.get(input.selected.1))
        .ok_or(AccessibilityProjectionError::InvalidSelection)?;
    if input.back_depth > MAX_NAVIGATION_DEPTH {
        return Err(AccessibilityProjectionError::NavigationDepth);
    }
    if (input.back_depth == 0) != input.subpage_title.is_none() {
        return Err(AccessibilityProjectionError::InvalidSubpage);
    }

    let mut budget = TextBudget::default();
    for fixed in [
        SETTINGS_NAME,
        ROOT_ID,
        SEARCH_ID,
        SEARCH_NAME,
        SIDEBAR_ID,
        DETAIL_ID,
        BACK_ID,
        BACK_NAME,
        LOCAL_ACCOUNT_LABEL,
        GLOBAL_ERROR_ID,
        GLOBAL_ERROR_TITLE,
        GLOBAL_ERROR_DISMISS_ID,
        DISMISS_NAME,
    ] {
        budget.add_required(fixed, MAX_LABEL_BYTES, false)?;
    }
    budget.add_optional(input.query, MAX_QUERY_BYTES, false)?;
    budget.add_required(input.account_name, MAX_LABEL_BYTES, false)?;
    if let Some(title) = input.subpage_title {
        budget.add_required(title, MAX_LABEL_BYTES, false)?;
    }
    if let Some(error) = input.global_error {
        budget.add_required(error, MAX_ERROR_BYTES, true)?;
    }

    let mut seen_ids = HashSet::with_capacity(category_count);
    let mut seen_names = HashSet::with_capacity(category_count);
    for section in input.sections {
        if section.is_empty() {
            return Err(AccessibilityProjectionError::InvalidCategory);
        }
        for category in section {
            validate_pane_id(category.pane_id)?;
            budget.add_required(category.pane_id, 128, false)?;
            budget.add_required(category.name, MAX_LABEL_BYTES, false)?;
            budget.add_required(category.description, MAX_LABEL_BYTES, true)?;
            if !seen_ids.insert(category.pane_id) || !seen_names.insert(category.name) {
                return Err(AccessibilityProjectionError::DuplicateCategory);
            }
        }
    }

    let visible_indices = input
        .sections
        .iter()
        .enumerate()
        .filter_map(|(section_index, section)| {
            let items = section
                .iter()
                .enumerate()
                .filter_map(|(item_index, category)| {
                    category_matches(input.query, category.name)
                        .then_some((section_index, item_index))
                })
                .collect::<Vec<_>>();
            (!items.is_empty()).then_some(items)
        })
        .collect::<Vec<_>>();
    let visible_section_count = visible_indices.len();
    let visible_category_count = visible_indices.iter().map(Vec::len).sum::<usize>();

    let mut sidebar_position = 0usize;
    let mut sections = Vec::with_capacity(visible_section_count);
    let mut selected_visible = false;
    let mut navigation_focus_order = Vec::with_capacity(visible_category_count + 3);
    let back_action = (input.back_depth > 0).then(|| AccessibleNavigationAction {
        id: BACK_ID.into(),
        name: BACK_NAME.into(),
        kind: NavigationActionKind::Back,
        enabled: true,
    });
    if back_action.is_some() {
        navigation_focus_order.push(BACK_ID.into());
    }
    let error_action = input.global_error.map(|_| AccessibleNavigationAction {
        id: GLOBAL_ERROR_DISMISS_ID.into(),
        name: DISMISS_NAME.into(),
        kind: NavigationActionKind::DismissError,
        enabled: true,
    });
    if error_action.is_some() {
        navigation_focus_order.push(GLOBAL_ERROR_DISMISS_ID.into());
    }
    navigation_focus_order.push(SEARCH_ID.into());

    for (visible_section_index, indices) in visible_indices.into_iter().enumerate() {
        let section_size = indices.len();
        let mut items = Vec::with_capacity(section_size);
        for (position_in_section, (section_index, item_index)) in indices.into_iter().enumerate() {
            let category = input.sections[section_index][item_index];
            sidebar_position += 1;
            let is_selected = (section_index, item_index) == input.selected;
            selected_visible |= is_selected;
            let action_id = format!("cat-{section_index}-{item_index}");
            let action_name = format!("Open {} settings", category.name);
            budget.add_required(&action_id, 128, false)?;
            budget.add_required(&action_name, MAX_LABEL_BYTES, false)?;
            navigation_focus_order.push(action_id.clone());
            items.push(AccessibleNavigationItem {
                pane_id: category.pane_id.into(),
                name: category.name.into(),
                description: category.description.into(),
                selected: is_selected,
                position_in_section: position_in_section + 1,
                section_size,
                position_in_sidebar: sidebar_position,
                sidebar_size: visible_category_count,
                action: AccessibleNavigationAction {
                    id: action_id,
                    name: action_name,
                    kind: NavigationActionKind::SelectPane,
                    enabled: true,
                },
            });
        }
        sections.push(AccessibleNavigationSection {
            position_in_set: visible_section_index + 1,
            set_size: visible_section_count,
            items,
        });
    }

    let heading = input.subpage_title.unwrap_or(selected.name).to_owned();
    budget.add_required(&heading, MAX_LABEL_BYTES, false)?;
    let mut announcements = Vec::new();
    if !input.query.is_empty() {
        let text = if visible_category_count == 1 {
            "1 matching settings category".to_owned()
        } else {
            format!("{visible_category_count} matching settings categories")
        };
        budget.add_required(&text, MAX_LABEL_BYTES, false)?;
        announcements.push(LiveAnnouncement {
            id: "settings-search-results".into(),
            text,
            politeness: LivePoliteness::Polite,
        });
    }
    if let Some(error) = input.global_error {
        announcements.push(LiveAnnouncement {
            id: GLOBAL_ERROR_ID.into(),
            text: error.into(),
            politeness: LivePoliteness::Assertive,
        });
    }

    let initial_focus = if input.back_depth > 0 {
        BACK_ID
    } else {
        SEARCH_ID
    }
    .to_owned();
    validate_actions(&navigation_focus_order)?;

    Ok(SettingsNavigationAccessibilitySnapshot {
        name: SETTINGS_NAME,
        root_id: ROOT_ID,
        search: AccessibleSearchField {
            id: SEARCH_ID,
            name: SEARCH_NAME,
            value: input.query.into(),
            result_count: visible_category_count,
            controls: SIDEBAR_ID,
        },
        account: AccessibleAccount {
            name: input.account_name.into(),
            description: LOCAL_ACCOUNT_LABEL,
        },
        sidebar_id: SIDEBAR_ID,
        sections,
        selected_pane_id: selected.pane_id.into(),
        selected_visible,
        detail_id: DETAIL_ID,
        detail: AccessibleDetail {
            pane_id: selected.pane_id.into(),
            pane_name: selected.name.into(),
            pane_description: selected.description.into(),
            heading,
            subpage_title: input.subpage_title.map(str::to_owned),
            back_depth: input.back_depth,
        },
        back_action,
        error_action,
        navigation_focus_order,
        initial_focus,
        announcements,
    })
}

pub fn category_matches(query: &str, category_name: &str) -> bool {
    query.is_empty() || category_name.to_lowercase().contains(&query.to_lowercase())
}

fn validate_pane_id(pane_id: &str) -> Result<(), AccessibilityProjectionError> {
    if pane_id.is_empty()
        || pane_id.len() > 128
        || pane_id
            .bytes()
            .any(|byte| !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'))
    {
        return Err(AccessibilityProjectionError::InvalidCategory);
    }
    Ok(())
}

fn validate_actions(actions: &[String]) -> Result<(), AccessibilityProjectionError> {
    let mut seen = HashSet::with_capacity(actions.len());
    if actions.iter().any(|action| !seen.insert(action.as_str())) {
        return Err(AccessibilityProjectionError::InvalidAction);
    }
    Ok(())
}

#[derive(Default)]
struct TextBudget {
    bytes: usize,
}

impl TextBudget {
    fn add_required(
        &mut self,
        text: &str,
        max: usize,
        multiline: bool,
    ) -> Result<(), AccessibilityProjectionError> {
        if text.trim().is_empty() {
            return Err(AccessibilityProjectionError::InvalidText);
        }
        self.add_optional(text, max, multiline)
    }

    fn add_optional(
        &mut self,
        text: &str,
        max: usize,
        multiline: bool,
    ) -> Result<(), AccessibilityProjectionError> {
        if text.len() > max {
            return Err(AccessibilityProjectionError::TextValueLimit);
        }
        if text.chars().any(|character| {
            character.is_control() && !(multiline && matches!(character, '\n' | '\t'))
        }) {
            return Err(AccessibilityProjectionError::InvalidText);
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

    fn category<'a>(id: &'a str, name: &'a str) -> NavigationCategoryInput<'a> {
        NavigationCategoryInput {
            pane_id: id,
            name,
            description: "Authoritative settings description",
        }
    }

    fn sections() -> Vec<Vec<NavigationCategoryInput<'static>>> {
        vec![
            vec![
                category("wifi", "Wi-Fi"),
                category("bluetooth", "Bluetooth"),
            ],
            vec![
                category("appearance", "Appearance"),
                category("displays", "Displays"),
            ],
        ]
    }

    #[test]
    fn complete_sidebar_selection_detail_and_focus_order_are_projected() {
        let sections = sections();
        let projected = project_settings_navigation(NavigationInput {
            sections: &sections,
            selected: (0, 1),
            query: "",
            account_name: "Jacob",
            subpage_title: None,
            back_depth: 0,
            global_error: None,
        })
        .unwrap();

        assert_eq!(projected.search.result_count, 4);
        assert_eq!(projected.sections.len(), 2);
        assert!(projected.sections[0].items[1].selected);
        assert_eq!(projected.selected_pane_id, "bluetooth");
        assert_eq!(projected.detail.heading, "Bluetooth");
        assert!(projected.selected_visible);
        assert_eq!(
            projected.navigation_focus_order,
            [
                "settings-search",
                "cat-0-0",
                "cat-0-1",
                "cat-1-0",
                "cat-1-1",
            ]
        );
        assert_eq!(projected.initial_focus, SEARCH_ID);
    }

    #[test]
    fn filtering_subpage_back_error_and_diagnostics_preserve_private_state() {
        let sections = sections();
        let input = NavigationInput {
            sections: &sections,
            selected: (0, 1),
            query: "display",
            account_name: "Private Account",
            subpage_title: Some("Private Application"),
            back_depth: 1,
            global_error: Some("Private backend detail"),
        };
        let projected = project_settings_navigation(input).unwrap();

        assert_eq!(projected.search.result_count, 1);
        assert_eq!(projected.sections[0].items[0].pane_id, "displays");
        assert!(!projected.selected_visible);
        assert_eq!(projected.detail.heading, "Private Application");
        assert_eq!(projected.initial_focus, BACK_ID);
        assert_eq!(
            &projected.navigation_focus_order[..3],
            [BACK_ID, GLOBAL_ERROR_DISMISS_ID, SEARCH_ID]
        );
        assert_eq!(
            projected.announcements.last().unwrap().politeness,
            LivePoliteness::Assertive
        );

        let diagnostics = format!("{input:?} {projected:?}");
        for private in [
            "display",
            "Private Account",
            "Private Application",
            "Private backend detail",
        ] {
            assert!(!diagnostics.contains(private));
        }
    }

    #[test]
    fn duplicate_invalid_selection_depth_and_oversized_query_fail_closed() {
        let mut duplicate = sections();
        duplicate[1][0].pane_id = "wifi";
        assert_eq!(
            project_settings_navigation(NavigationInput {
                sections: &duplicate,
                selected: (0, 0),
                query: "",
                account_name: "Account",
                subpage_title: None,
                back_depth: 0,
                global_error: None,
            }),
            Err(AccessibilityProjectionError::DuplicateCategory)
        );

        let sections = sections();
        let base = NavigationInput {
            sections: &sections,
            selected: (9, 9),
            query: "",
            account_name: "Account",
            subpage_title: None,
            back_depth: 0,
            global_error: None,
        };
        assert_eq!(
            project_settings_navigation(base),
            Err(AccessibilityProjectionError::InvalidSelection)
        );
        assert_eq!(
            project_settings_navigation(NavigationInput {
                selected: (0, 0),
                subpage_title: Some("Too deep"),
                back_depth: MAX_NAVIGATION_DEPTH + 1,
                ..base
            }),
            Err(AccessibilityProjectionError::NavigationDepth)
        );
        let oversized = "x".repeat(MAX_QUERY_BYTES + 1);
        assert_eq!(
            project_settings_navigation(NavigationInput {
                selected: (0, 0),
                query: &oversized,
                ..base
            }),
            Err(AccessibilityProjectionError::TextValueLimit)
        );
    }
}
