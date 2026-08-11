//! Overlay-facing launcher lifecycle model.

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FocusTarget {
    Query,
}

#[derive(Clone, Debug)]
pub struct OpenEffect {
    pub request: Request,
    pub focus: FocusTarget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyCommand {
    ArrowDown,
    ArrowUp,
    Return,
    AlternateReturn,
    Escape,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Activation {
    pub id: rmac_launcher_system::ActivationId,
    pub action: rmac_launcher::Action,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeyEffect {
    None,
    SelectionChanged,
    Activate(Activation),
    Dismissed,
}

#[derive(Clone, Debug)]
pub enum ShortcutEffect {
    None,
    Open(OpenEffect),
    Dismissed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Phase {
    Closed,
    Loading,
    Results {
        still_searching: bool,
        degraded: bool,
    },
    Empty,
    Unavailable,
    Activating,
    ActivationFailed,
}

#[derive(Clone, Eq, PartialEq)]
pub struct Row {
    pub id: ResultId,
    pub category: rmac_launcher::Category,
    pub application_group: Option<rmac_launcher::ApplicationGroup>,
    pub category_label: &'static str,
    pub title: String,
    pub subtitle: Option<String>,
    pub icon: Option<std::path::PathBuf>,
    pub selected: bool,
    pub primary_label: &'static str,
    pub has_alternate: bool,
    pub alternate_label: Option<&'static str>,
}

impl fmt::Debug for Row {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Row")
            .field("id", &"<redacted>")
            .field("category", &self.category)
            .field("application_group", &self.application_group)
            .field("title", &"<redacted>")
            .field("subtitle", &self.subtitle.as_ref().map(|_| "<redacted>"))
            .field("icon", &self.icon.as_ref().map(|_| "<redacted>"))
            .field("selected", &self.selected)
            .field("primary_label", &self.primary_label)
            .field("has_alternate", &self.has_alternate)
            .field("alternate_label", &self.alternate_label)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct Snapshot {
    pub open: bool,
    pub query: String,
    pub phase: Phase,
    pub rows: Vec<Row>,
    pub pending_providers: usize,
    pub failed_providers: usize,
    pub application_catalog: CatalogHealth,
    /// Exact action mode currently executing for the selected row.
    pub activating: Option<ActivationMode>,
    /// A concise live-region message. It intentionally contains no provider
    /// error detail, file path, or action payload.
    pub announcement: Option<String>,
}

impl fmt::Debug for Snapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Snapshot")
            .field("open", &self.open)
            .field("query", &"<redacted>")
            .field("phase", &self.phase)
            .field("row_count", &self.rows.len())
            .field("pending_providers", &self.pending_providers)
            .field("failed_providers", &self.failed_providers)
            .field("application_catalog", &self.application_catalog)
            .field("activating", &self.activating)
            .field(
                "announcement",
                &self.announcement.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

pub struct Coordinator {
    pub(super) launcher: rmac_launcher::Launcher,
    pub(super) descriptors: Vec<ProviderDescriptor>,
    pub(super) policies:
        BTreeMap<rmac_shell_settings::ProviderId, rmac_shell_settings::ProviderPolicy>,
    pub(super) next_activation: u64,
    pub(super) activation: Option<rmac_launcher_system::ActivationId>,
    pub(super) activation_mode: Option<ActivationMode>,
    pub(super) activation_error: Option<String>,
    pub(super) last_shortcut_timestamp_ms: Option<u64>,
    pub(super) application_catalog: CatalogHealth,
    pub(super) application_catalog_revision: u64,
}
