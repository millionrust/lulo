//! Bounded combobox, result-option, action, section, and live-region semantics.

use std::collections::{BTreeSet, HashSet};
use std::fmt;

use rmac_launcher::{ActivationMode, Category};

use crate::{CatalogHealth, Phase, Snapshot};

pub const LAUNCHER_NAME: &str = "Spotlight";
pub const QUERY_ID: &str = "launcher-query";
pub const QUERY_NAME: &str = "Spotlight Search";
pub const RESULTS_ID: &str = "launcher-results-scroll";
pub const APPLICATIONS_SECTION_NAME: &str = "Applications";
pub const SUGGESTIONS_SECTION_NAME: &str = "Suggestions";
pub const SEARCH_SCOPE_DESCRIPTION: &str = "Applications, Settings, files, and calculations";
pub const KEYBOARD_HELP: &str = "↑↓ Select   Return Open   Ctrl/⌘-Return More   Esc Close";
pub const CLOSED_LABEL: &str = "Closed";
pub const SEARCHING_LABEL: &str = "Searching…";
pub const DEGRADED_LABEL: &str = "Some results are unavailable";
pub const EMPTY_LABEL: &str = "No results";
pub const UNAVAILABLE_LABEL: &str = "Search providers are unavailable";
pub const OPENING_LABEL: &str = "Opening…";
pub const ACTIVATION_FAILED_LABEL: &str = "Could not open the selection";
pub const MAX_ACCESSIBLE_RESULTS: usize = 40;
pub const MAX_ACCESSIBLE_PROVIDERS: usize = 64;
pub const MAX_ACCESSIBLE_QUERY_BYTES: usize = 64 * 1024;
pub const MAX_ACCESSIBLE_TITLE_BYTES: usize = 4 * 1024;
pub const MAX_ACCESSIBLE_SUBTITLE_BYTES: usize = 16 * 1024;
pub const MAX_ACCESSIBLE_TEXT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResultLayout {
    GridTile,
    ListRow,
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleResultAction {
    pub id: String,
    pub name: String,
    pub mode: ActivationMode,
    pub enabled: bool,
    pub busy: bool,
}

impl fmt::Debug for AccessibleResultAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleResultAction")
            .field("id", &self.id)
            .field("name", &"<redacted>")
            .field("mode", &self.mode)
            .field("enabled", &self.enabled)
            .field("busy", &self.busy)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleResult {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub category: Category,
    pub category_label: &'static str,
    pub layout: ResultLayout,
    pub selected: bool,
    pub position_in_set: usize,
    pub set_size: usize,
    pub position_in_section: usize,
    pub section_size: usize,
    pub actions: Vec<AccessibleResultAction>,
}

impl fmt::Debug for AccessibleResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleResult")
            .field("id", &self.id)
            .field("name", &"<redacted>")
            .field(
                "description",
                &self.description.as_ref().map(|_| "<redacted>"),
            )
            .field("category", &self.category)
            .field("layout", &self.layout)
            .field("selected", &self.selected)
            .field("position_in_set", &self.position_in_set)
            .field("set_size", &self.set_size)
            .field("position_in_section", &self.position_in_section)
            .field("section_size", &self.section_size)
            .field("action_count", &self.actions.len())
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleSection {
    pub id: String,
    pub name: &'static str,
    pub position_in_set: usize,
    pub set_size: usize,
    pub results: Vec<AccessibleResult>,
}

impl fmt::Debug for AccessibleSection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleSection")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("position_in_set", &self.position_in_set)
            .field("set_size", &self.set_size)
            .field("result_count", &self.results.len())
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleSearchField {
    pub id: &'static str,
    pub name: &'static str,
    pub value: String,
    pub enabled: bool,
    pub busy: bool,
    pub expanded: bool,
    pub controls: &'static str,
    pub active_descendant: Option<String>,
}

impl fmt::Debug for AccessibleSearchField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleSearchField")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("value", &"<redacted>")
            .field("enabled", &self.enabled)
            .field("busy", &self.busy)
            .field("expanded", &self.expanded)
            .field("controls", &self.controls)
            .field("active_descendant", &self.active_descendant)
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

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct SurfaceStatus<'a> {
    pub settings_error: Option<&'a str>,
}

impl fmt::Debug for SurfaceStatus<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SurfaceStatus")
            .field("has_settings_error", &self.settings_error.is_some())
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct LauncherAccessibilitySnapshot {
    pub name: &'static str,
    pub search: AccessibleSearchField,
    pub results_id: &'static str,
    pub sections: Vec<AccessibleSection>,
    pub result_count: usize,
    pub status: String,
    pub description: &'static str,
    pub keyboard_help: &'static str,
    /// The query retains DOM/AT focus; arrows update `active_descendant`.
    pub focus_order: Vec<&'static str>,
    pub initial_focus: &'static str,
    pub announcements: Vec<LiveAnnouncement>,
}

impl fmt::Debug for LauncherAccessibilitySnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LauncherAccessibilitySnapshot")
            .field("search", &self.search)
            .field("section_count", &self.sections.len())
            .field("result_count", &self.result_count)
            .field("status", &"<redacted>")
            .field("focus_order", &self.focus_order)
            .field("initial_focus", &self.initial_focus)
            .field("announcement_count", &self.announcements.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessibilityProjectionError {
    ClosedSnapshot,
    ResultLimit,
    ProviderLimit,
    InvalidPhase,
    InvalidSelection,
    InvalidPresentationOrder,
    InvalidCategory,
    InvalidAction,
    DuplicateResult,
    DuplicateAction,
    InvalidText,
    TextValueLimit,
    TextLimit,
}

pub fn project_launcher(
    snapshot: &Snapshot,
    surface: SurfaceStatus<'_>,
) -> Result<LauncherAccessibilitySnapshot, AccessibilityProjectionError> {
    validate_snapshot(snapshot)?;
    let mut budget = TextBudget::default();
    for text in [
        LAUNCHER_NAME,
        QUERY_ID,
        QUERY_NAME,
        RESULTS_ID,
        SEARCH_SCOPE_DESCRIPTION,
        KEYBOARD_HELP,
    ] {
        budget.add_required(text, MAX_ACCESSIBLE_TITLE_BYTES, false)?;
    }
    budget.add_optional(&snapshot.query, MAX_ACCESSIBLE_QUERY_BYTES, false)?;

    let section_specs = section_specs(snapshot)?;
    let section_count = section_specs.len();
    let content_enabled = snapshot.phase != Phase::Activating;
    let mut sections = Vec::with_capacity(section_count);
    let mut selected_id = None;
    let mut action_ids = HashSet::new();
    for (section_index, spec) in section_specs.into_iter().enumerate() {
        budget.add_required(spec.name, MAX_ACCESSIBLE_TITLE_BYTES, false)?;
        let section_size = spec.indices.len();
        let mut results = Vec::with_capacity(section_size);
        for (section_position, index) in spec.indices.into_iter().enumerate() {
            let row = &snapshot.rows[index];
            budget.add_required(&row.title, MAX_ACCESSIBLE_TITLE_BYTES, false)?;
            if let Some(subtitle) = &row.subtitle {
                budget.add_optional(subtitle, MAX_ACCESSIBLE_SUBTITLE_BYTES, false)?;
            }
            budget.add_required(row.primary_label, MAX_ACCESSIBLE_TITLE_BYTES, false)?;
            if let Some(label) = row.alternate_label {
                budget.add_required(label, MAX_ACCESSIBLE_TITLE_BYTES, false)?;
            }

            let layout = if snapshot.query.is_empty() && row.category == Category::Applications {
                ResultLayout::GridTile
            } else {
                ResultLayout::ListRow
            };
            let primary_id = match layout {
                ResultLayout::GridTile => format!("launcher-grid-{index}"),
                ResultLayout::ListRow => format!("launcher-row-{index}"),
            };
            let primary_busy = row.selected && snapshot.activating == Some(ActivationMode::Primary);
            let primary = result_action(
                primary_id.clone(),
                row.primary_label,
                ActivationMode::Primary,
                content_enabled,
                primary_busy,
                &mut budget,
                &mut action_ids,
            )?;
            let mut actions = vec![primary];
            if let Some(label) = row.alternate_label {
                let alternate = result_action(
                    format!("launcher-alternate-{index}"),
                    label,
                    ActivationMode::Alternate,
                    content_enabled,
                    row.selected && snapshot.activating == Some(ActivationMode::Alternate),
                    &mut budget,
                    &mut action_ids,
                )?;
                actions.push(alternate);
            }
            if row.selected {
                selected_id = Some(primary_id.clone());
            }
            results.push(AccessibleResult {
                id: primary_id,
                name: row.title.clone(),
                description: row.subtitle.clone(),
                category: row.category,
                category_label: row.category_label,
                layout,
                selected: row.selected,
                position_in_set: index + 1,
                set_size: snapshot.rows.len(),
                position_in_section: section_position + 1,
                section_size,
                actions,
            });
        }
        sections.push(AccessibleSection {
            id: format!("launcher-section-{section_index}"),
            name: spec.name,
            position_in_set: section_index + 1,
            set_size: section_count,
            results,
        });
    }

    let busy = matches!(
        snapshot.phase,
        Phase::Loading
            | Phase::Results {
                still_searching: true,
                ..
            }
            | Phase::Activating
    );
    let search = AccessibleSearchField {
        id: QUERY_ID,
        name: QUERY_NAME,
        value: snapshot.query.clone(),
        enabled: snapshot.phase != Phase::Activating,
        busy,
        expanded: !snapshot.rows.is_empty(),
        controls: RESULTS_ID,
        active_descendant: selected_id,
    };

    let status = visible_phase_label(snapshot);
    budget.add_required(&status, MAX_ACCESSIBLE_SUBTITLE_BYTES, true)?;
    let mut announcements = Vec::new();
    if let Some(announcement) = &snapshot.announcement {
        push_announcement(
            &mut announcements,
            &mut budget,
            "launcher-runtime-status",
            announcement,
            if matches!(snapshot.phase, Phase::Unavailable | Phase::ActivationFailed) {
                LivePoliteness::Assertive
            } else {
                LivePoliteness::Polite
            },
        )?;
    }
    if matches!(snapshot.phase, Phase::Results { degraded: true, .. }) {
        push_announcement(
            &mut announcements,
            &mut budget,
            "launcher-degraded",
            DEGRADED_LABEL,
            LivePoliteness::Assertive,
        )?;
    }
    if let Some(error) = surface.settings_error {
        push_announcement(
            &mut announcements,
            &mut budget,
            "launcher-settings-error",
            error,
            LivePoliteness::Assertive,
        )?;
    }

    Ok(LauncherAccessibilitySnapshot {
        name: LAUNCHER_NAME,
        search,
        results_id: RESULTS_ID,
        sections,
        result_count: snapshot.rows.len(),
        status,
        description: SEARCH_SCOPE_DESCRIPTION,
        keyboard_help: KEYBOARD_HELP,
        focus_order: vec![QUERY_ID],
        initial_focus: QUERY_ID,
        announcements,
    })
}

pub fn visible_phase_label(snapshot: &Snapshot) -> String {
    match &snapshot.phase {
        Phase::Closed => CLOSED_LABEL.into(),
        Phase::Loading
        | Phase::Results {
            still_searching: true,
            degraded: false,
        } => SEARCHING_LABEL.into(),
        Phase::Results { degraded: true, .. } => DEGRADED_LABEL.into(),
        Phase::Results { .. } => format!("{} results", snapshot.rows.len()),
        Phase::Empty => EMPTY_LABEL.into(),
        Phase::Unavailable => UNAVAILABLE_LABEL.into(),
        Phase::Activating => OPENING_LABEL.into(),
        Phase::ActivationFailed => snapshot
            .announcement
            .clone()
            .unwrap_or_else(|| ACTIVATION_FAILED_LABEL.into()),
    }
}

fn validate_snapshot(snapshot: &Snapshot) -> Result<(), AccessibilityProjectionError> {
    if !snapshot.open || snapshot.phase == Phase::Closed {
        return Err(AccessibilityProjectionError::ClosedSnapshot);
    }
    if snapshot.rows.len() > MAX_ACCESSIBLE_RESULTS {
        return Err(AccessibilityProjectionError::ResultLimit);
    }
    if snapshot.pending_providers > MAX_ACCESSIBLE_PROVIDERS
        || snapshot.failed_providers > MAX_ACCESSIBLE_PROVIDERS
        || snapshot
            .pending_providers
            .saturating_add(snapshot.failed_providers)
            > MAX_ACCESSIBLE_PROVIDERS
    {
        return Err(AccessibilityProjectionError::ProviderLimit);
    }
    let mut result_ids = BTreeSet::new();
    let mut selected = 0usize;
    for row in &snapshot.rows {
        if !result_ids.insert(&row.id) {
            return Err(AccessibilityProjectionError::DuplicateResult);
        }
        if row.category_label != row.category.label() {
            return Err(AccessibilityProjectionError::InvalidCategory);
        }
        if row.has_alternate != row.alternate_label.is_some() {
            return Err(AccessibilityProjectionError::InvalidAction);
        }
        if row.selected {
            selected += 1;
        }
    }
    if selected != usize::from(!snapshot.rows.is_empty()) {
        return Err(AccessibilityProjectionError::InvalidSelection);
    }

    let catalog_starting = snapshot.application_catalog == CatalogHealth::Starting;
    let catalog_unavailable = snapshot.application_catalog == CatalogHealth::Unavailable;
    let phase_valid = match snapshot.phase {
        Phase::Closed => false,
        Phase::Loading => {
            snapshot.rows.is_empty()
                && (snapshot.pending_providers > 0 || catalog_starting)
                && snapshot.activating.is_none()
        }
        Phase::Results {
            still_searching,
            degraded,
        } => {
            !snapshot.rows.is_empty()
                && still_searching == (snapshot.pending_providers > 0)
                && degraded == (snapshot.failed_providers > 0 || catalog_unavailable)
                && snapshot.activating.is_none()
        }
        Phase::Empty => {
            snapshot.rows.is_empty()
                && snapshot.pending_providers == 0
                && snapshot.failed_providers == 0
                && !catalog_starting
                && !catalog_unavailable
                && snapshot.activating.is_none()
        }
        Phase::Unavailable => {
            snapshot.rows.is_empty()
                && (snapshot.failed_providers > 0 || catalog_unavailable)
                && snapshot.activating.is_none()
        }
        Phase::Activating => !snapshot.rows.is_empty() && snapshot.activating.is_some(),
        Phase::ActivationFailed => {
            !snapshot.rows.is_empty()
                && snapshot.activating.is_none()
                && snapshot.announcement.is_some()
        }
    };
    if !phase_valid {
        return Err(AccessibilityProjectionError::InvalidPhase);
    }
    if snapshot.announcement.is_none() {
        return Err(AccessibilityProjectionError::InvalidPhase);
    }
    Ok(())
}

struct SectionSpec {
    name: &'static str,
    indices: Vec<usize>,
}

fn section_specs(snapshot: &Snapshot) -> Result<Vec<SectionSpec>, AccessibilityProjectionError> {
    if snapshot.rows.is_empty() {
        return Ok(Vec::new());
    }
    if snapshot.query.is_empty() {
        let split = snapshot
            .rows
            .iter()
            .position(|row| row.category != Category::Applications)
            .unwrap_or(snapshot.rows.len());
        if snapshot.rows[split..]
            .iter()
            .any(|row| row.category == Category::Applications)
        {
            return Err(AccessibilityProjectionError::InvalidPresentationOrder);
        }
        let mut sections = Vec::new();
        if split > 0 {
            sections.push(SectionSpec {
                name: APPLICATIONS_SECTION_NAME,
                indices: (0..split).collect(),
            });
        }
        if split < snapshot.rows.len() {
            sections.push(SectionSpec {
                name: SUGGESTIONS_SECTION_NAME,
                indices: (split..snapshot.rows.len()).collect(),
            });
        }
        return Ok(sections);
    }

    let mut sections: Vec<SectionSpec> = Vec::new();
    for (index, row) in snapshot.rows.iter().enumerate() {
        if sections
            .last()
            .is_none_or(|section| section.name != row.category_label)
        {
            sections.push(SectionSpec {
                name: row.category_label,
                indices: Vec::new(),
            });
        }
        sections.last_mut().unwrap().indices.push(index);
    }
    Ok(sections)
}

fn result_action(
    id: String,
    name: &str,
    mode: ActivationMode,
    enabled: bool,
    busy: bool,
    budget: &mut TextBudget,
    action_ids: &mut HashSet<String>,
) -> Result<AccessibleResultAction, AccessibilityProjectionError> {
    budget.add_required(&id, MAX_ACCESSIBLE_TITLE_BYTES, false)?;
    budget.add_required(name, MAX_ACCESSIBLE_TITLE_BYTES, false)?;
    if !action_ids.insert(id.clone()) {
        return Err(AccessibilityProjectionError::DuplicateAction);
    }
    if busy && enabled {
        return Err(AccessibilityProjectionError::InvalidAction);
    }
    Ok(AccessibleResultAction {
        id,
        name: name.into(),
        mode,
        enabled,
        busy,
    })
}

fn push_announcement(
    announcements: &mut Vec<LiveAnnouncement>,
    budget: &mut TextBudget,
    id: &str,
    text: &str,
    politeness: LivePoliteness,
) -> Result<(), AccessibilityProjectionError> {
    budget.add_required(id, MAX_ACCESSIBLE_TITLE_BYTES, false)?;
    budget.add_required(text, MAX_ACCESSIBLE_SUBTITLE_BYTES, true)?;
    announcements.push(LiveAnnouncement {
        id: id.into(),
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
        if self.bytes > MAX_ACCESSIBLE_TEXT_BYTES {
            return Err(AccessibilityProjectionError::TextLimit);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CatalogHealth, Row};
    use rmac_launcher::{Category, ResultId};

    fn row(
        provider: &str,
        local: &str,
        category: Category,
        title: &str,
        selected: bool,
        alternate: bool,
    ) -> Row {
        Row {
            id: ResultId {
                provider: rmac_shell_settings::ProviderId(provider.into()),
                local: local.into(),
            },
            category,
            category_label: category.label(),
            title: title.into(),
            subtitle: Some(format!("Private subtitle for {title}")),
            icon: Some(format!("/home/alex/{title}.png").into()),
            selected,
            primary_label: if category == Category::Calculator {
                "Copy"
            } else {
                "Open"
            },
            has_alternate: alternate,
            alternate_label: alternate.then_some("Show in Folder"),
        }
    }

    fn results(query: &str, rows: Vec<Row>) -> Snapshot {
        let selected = rows.iter().find(|row| row.selected).unwrap();
        Snapshot {
            open: true,
            query: query.into(),
            phase: Phase::Results {
                still_searching: false,
                degraded: false,
            },
            pending_providers: 0,
            failed_providers: 0,
            application_catalog: CatalogHealth::Healthy,
            activating: None,
            announcement: Some(format!(
                "{}, {}, 1 of {} results",
                selected.title,
                selected.category_label,
                rows.len()
            )),
            rows,
        }
    }

    fn status() -> SurfaceStatus<'static> {
        SurfaceStatus {
            settings_error: None,
        }
    }

    #[test]
    fn empty_query_projects_grid_then_suggestions_with_active_descendant() {
        let snapshot = results(
            "",
            vec![
                row(
                    "applications",
                    "terminal",
                    Category::Applications,
                    "Terminal",
                    true,
                    true,
                ),
                row(
                    "settings",
                    "sound",
                    Category::Settings,
                    "Sound",
                    false,
                    false,
                ),
                row("calculator", "two", Category::Calculator, "2", false, false),
            ],
        );
        let projected = project_launcher(&snapshot, status()).unwrap();

        assert_eq!(
            projected
                .sections
                .iter()
                .map(|section| section.name)
                .collect::<Vec<_>>(),
            [APPLICATIONS_SECTION_NAME, SUGGESTIONS_SECTION_NAME]
        );
        assert_eq!(
            projected.sections[0].results[0].layout,
            ResultLayout::GridTile
        );
        assert_eq!(
            projected.sections[1].results[0].layout,
            ResultLayout::ListRow
        );
        assert_eq!(
            projected.search.active_descendant.as_deref(),
            Some("launcher-grid-0")
        );
        assert_eq!(projected.focus_order, [QUERY_ID]);
        assert_eq!(
            projected.sections[0].results[0]
                .actions
                .iter()
                .map(|action| action.id.as_str())
                .collect::<Vec<_>>(),
            ["launcher-grid-0", "launcher-alternate-0"]
        );
        assert_eq!(projected.sections[1].results[1].actions[0].name, "Copy");
    }

    #[test]
    fn exact_busy_action_degraded_feedback_and_diagnostics_are_private() {
        let mut snapshot = results(
            "private query",
            vec![
                row(
                    "files",
                    "private-file",
                    Category::Files,
                    "Private Report.txt",
                    true,
                    true,
                ),
                row(
                    "settings",
                    "privacy",
                    Category::Settings,
                    "Privacy",
                    false,
                    false,
                ),
            ],
        );
        snapshot.phase = Phase::Activating;
        snapshot.activating = Some(ActivationMode::Alternate);
        snapshot.announcement = Some("Opening selection".into());
        let surface = SurfaceStatus {
            settings_error: Some("Private settings backend detail"),
        };

        let projected = project_launcher(&snapshot, surface).unwrap();
        let selected = &projected.sections[0].results[0];
        assert!(!projected.search.enabled);
        assert!(!selected.actions[0].busy);
        assert!(selected.actions[1].busy);
        assert!(selected.actions.iter().all(|action| !action.enabled));
        assert!(projected
            .announcements
            .iter()
            .any(|announcement| announcement.politeness == LivePoliteness::Assertive));

        let diagnostics = format!("{snapshot:?} {surface:?} {projected:?}");
        for private in [
            "private query",
            "Private Report.txt",
            "Private subtitle",
            "private-file",
            "/home/alex",
            "Private settings backend detail",
            "Opening selection",
        ] {
            assert!(!diagnostics.contains(private));
        }
    }

    #[test]
    fn closed_duplicate_misordered_and_oversized_snapshots_fail_closed() {
        let closed = Snapshot {
            open: false,
            query: String::new(),
            phase: Phase::Closed,
            rows: Vec::new(),
            pending_providers: 0,
            failed_providers: 0,
            application_catalog: CatalogHealth::Healthy,
            activating: None,
            announcement: None,
        };
        assert_eq!(
            project_launcher(&closed, status()),
            Err(AccessibilityProjectionError::ClosedSnapshot)
        );

        let setting = row(
            "settings",
            "sound",
            Category::Settings,
            "Sound",
            true,
            false,
        );
        let mut application = row(
            "applications",
            "terminal",
            Category::Applications,
            "Terminal",
            false,
            false,
        );
        let misordered = results("", vec![setting, application.clone()]);
        assert_eq!(
            project_launcher(&misordered, status()),
            Err(AccessibilityProjectionError::InvalidPresentationOrder)
        );

        application.selected = true;
        let duplicate = results("term", vec![application.clone(), application]);
        assert_eq!(
            project_launcher(&duplicate, status()),
            Err(AccessibilityProjectionError::DuplicateResult)
        );

        let mut oversized = results(
            "term",
            vec![row(
                "applications",
                "terminal",
                Category::Applications,
                "Terminal",
                true,
                false,
            )],
        );
        oversized.query = "x".repeat(MAX_ACCESSIBLE_QUERY_BYTES + 1);
        assert_eq!(
            project_launcher(&oversized, status()),
            Err(AccessibilityProjectionError::TextValueLimit)
        );
    }
}
