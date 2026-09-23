use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub(crate) const DEFAULT_LIMIT: usize = 40;
pub(crate) const DEFAULT_CATEGORY_LIMIT: usize = 12;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Category {
    Applications,
    Settings,
    /// Calculations, unit and currency conversions: the answer card.
    Calculator,
    /// The time in another city: the answer card.
    Clock,
    /// A word's definition from a local dictionary.
    Dictionary,
    Files,
    Other,
    /// The fixed "Search in Files" row macOS keeps at the end of every list
    /// ("Search in Finder").
    SearchIn,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Self::Applications => "Applications",
            Self::Settings => "Settings",
            Self::Calculator => "Calculator",
            Self::Clock => "World Clock",
            Self::Dictionary => "Dictionary",
            Self::Files => "Files",
            Self::Other => "Other",
            Self::SearchIn => "Search",
        }
    }

    pub(crate) fn rank(self) -> u16 {
        match self {
            Self::Applications => 50,
            Self::Settings => 40,
            Self::Calculator | Self::Clock => 30,
            Self::Files => 20,
            Self::Dictionary => 15,
            Self::Other => 10,
            Self::SearchIn => 0,
        }
    }

    /// Answers computed from the query itself ("12*7", "5 km in miles",
    /// "time in tokyo"). They lead the list, drawn as the Mac's answer card
    /// under the bar.
    pub fn is_answer(self) -> bool {
        matches!(self, Self::Calculator | Self::Clock)
    }

    /// Rows whose provider alone decides relevance: their titles need not
    /// contain the query ("define serendipity" → "serendipity").
    pub(crate) fn matches_any_query(self) -> bool {
        matches!(
            self,
            Self::Calculator | Self::Clock | Self::Dictionary | Self::SearchIn
        )
    }
}

/// Tahoe's fixed Apps browse categories. Suggestions are intentionally not a
/// category: they require truthful usage ranking and are omitted until that
/// authority is available. Arcade is Apple Games content and is likewise not
/// synthesized from ordinary Linux games.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ApplicationGroup {
    ProductivityFinance,
    Social,
    Creativity,
    InformationReading,
    Entertainment,
    Utilities,
    Other,
}

impl ApplicationGroup {
    pub const ORDER: [Self; 7] = [
        Self::ProductivityFinance,
        Self::Social,
        Self::Creativity,
        Self::InformationReading,
        Self::Entertainment,
        Self::Utilities,
        Self::Other,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::ProductivityFinance => "Productivity & Finance",
            Self::Social => "Social",
            Self::Creativity => "Creativity",
            Self::InformationReading => "Information & Reading",
            Self::Entertainment => "Entertainment",
            Self::Utilities => "Utilities",
            Self::Other => "Other",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Privacy {
    pub private_content: bool,
    pub network: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderDescriptor {
    pub id: rmac_shell_settings::ProviderId,
    pub category: Category,
    pub privacy: Privacy,
}

pub fn enabled_providers(
    descriptors: &[ProviderDescriptor],
    policies: &BTreeMap<rmac_shell_settings::ProviderId, rmac_shell_settings::ProviderPolicy>,
) -> Vec<ProviderDescriptor> {
    let mut seen = BTreeSet::new();
    descriptors
        .iter()
        .filter(|descriptor| {
            let policy = policies.get(&descriptor.id).cloned().unwrap_or_default();
            policy.enabled
                && (!descriptor.privacy.private_content || policy.allow_private_content)
                && (!descriptor.privacy.network || policy.allow_network)
                && seen.insert(descriptor.id.clone())
        })
        .cloned()
        .collect()
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ResultId {
    pub provider: rmac_shell_settings::ProviderId,
    pub local: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    LaunchApplication {
        app_id: String,
        spec: rmac_apps::LaunchSpec,
    },
    RevealApplication {
        source: PathBuf,
    },
    OpenSetting {
        pane_id: String,
    },
    OpenFile {
        path: PathBuf,
    },
    RevealFile {
        path: PathBuf,
    },
    CopyText {
        text: String,
    },
    /// Open Files searching the home folder for `query`.
    SearchFiles {
        query: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchResult {
    pub id: ResultId,
    pub category: Category,
    /// Apps-browse grouping supplied only by the installed-application
    /// provider. Other result categories leave this unset.
    pub application_group: Option<ApplicationGroup>,
    pub title: String,
    pub subtitle: Option<String>,
    /// A third line an answer card shows: the time in a city, the rate
    /// source for a currency, the dictionary a definition comes from.
    pub detail: Option<String>,
    /// Optional host-resolved icon for presentation. Providers may omit it;
    /// the surface then uses an original category fallback.
    pub icon: Option<PathBuf>,
    pub primary: Action,
    pub alternate: Option<Action>,
    /// Provider-normalized 0–100 recency/frequency signal, never wall time.
    pub recency_rank: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RankedResult {
    pub result: SearchResult,
    pub score: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderError {
    pub detail: String,
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for ProviderError {}

#[derive(Clone, Debug)]
pub struct Cancellation {
    cancelled: Arc<AtomicBool>,
}

impl Default for Cancellation {
    fn default() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Cancellation {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn flag(&self) -> &AtomicBool {
        &self.cancelled
    }
}

#[derive(Clone, Debug)]
pub struct Request {
    pub generation: u64,
    pub query: String,
    pub providers: Vec<ProviderDescriptor>,
    pub cancellation: Cancellation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MoveSelection {
    Next,
    Previous,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivationMode {
    Primary,
    Alternate,
}
