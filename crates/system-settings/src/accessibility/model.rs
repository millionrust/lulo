//! System Settings navigation accessibility vocabulary and public model.

use std::fmt;

pub const SETTINGS_NAME: &str = "System Settings";
pub const ROOT_ID: &str = "system-settings";
pub const SEARCH_ID: &str = "settings-search";
pub const SEARCH_NAME: &str = "Search";
pub const SIDEBAR_ID: &str = "sidebar-scroll";
pub const SIDEBAR_TOGGLE_ID: &str = "toggle-settings-sidebar";
pub const SHOW_SIDEBAR_NAME: &str = "Show Settings Sidebar";
pub const HIDE_SIDEBAR_NAME: &str = "Hide Settings Sidebar";
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
pub const MAX_CATEGORY_SEARCH_TERMS: usize = 32;
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
    pub search_terms: &'a [&'a str],
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
    pub sidebar_visible: bool,
    pub detail_visible: bool,
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
            .field("sidebar_visible", &self.sidebar_visible)
            .field("detail_visible", &self.detail_visible)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigationActionKind {
    SelectPane,
    ToggleSidebar,
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
    /// The implemented setting label that best explains a filtered result.
    pub match_hint: Option<String>,
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
            .field("match_hint", &self.match_hint)
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
    pub sidebar_visible: bool,
    pub sections: Vec<AccessibleNavigationSection>,
    pub selected_pane_id: String,
    pub selected_visible: bool,
    pub detail_id: &'static str,
    pub detail_visible: bool,
    pub detail: AccessibleDetail,
    pub sidebar_toggle_action: Option<AccessibleNavigationAction>,
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
            .field("sidebar_visible", &self.sidebar_visible)
            .field("detail_visible", &self.detail_visible)
            .field("detail", &self.detail)
            .field(
                "has_sidebar_toggle_action",
                &self.sidebar_toggle_action.is_some(),
            )
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
    InvalidVisibility,
    InvalidCategory,
    DuplicateCategory,
    InvalidSubpage,
    InvalidAction,
    InvalidText,
    TextValueLimit,
    TextLimit,
}
