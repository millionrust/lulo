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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Row {
    pub id: ResultId,
    pub category: rmac_launcher::Category,
    pub category_label: &'static str,
    pub title: String,
    pub subtitle: Option<String>,
    pub icon: Option<std::path::PathBuf>,
    pub selected: bool,
    pub has_alternate: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Snapshot {
    pub open: bool,
    pub query: String,
    pub phase: Phase,
    pub rows: Vec<Row>,
    pub pending_providers: usize,
    pub failed_providers: usize,
    pub application_catalog: CatalogHealth,
    /// A concise live-region message. It intentionally contains no provider
    /// error detail, file path, or action payload.
    pub announcement: Option<String>,
}

pub struct Coordinator {
    pub(super) launcher: rmac_launcher::Launcher,
    pub(super) descriptors: Vec<ProviderDescriptor>,
    pub(super) policies:
        BTreeMap<rmac_shell_settings::ProviderId, rmac_shell_settings::ProviderPolicy>,
    pub(super) next_activation: u64,
    pub(super) activation: Option<rmac_launcher_system::ActivationId>,
    pub(super) activation_error: Option<String>,
    pub(super) last_shortcut_timestamp_ms: Option<u64>,
    pub(super) application_catalog: CatalogHealth,
    pub(super) application_catalog_revision: u64,
}
