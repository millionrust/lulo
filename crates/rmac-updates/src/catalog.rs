//! The Mac-style presentation of an update snapshot: Lulo OS as one item,
//! everything else as "Other Updates".

use super::*;

/// Lulo OS's own packages. An update to any of them is part of the one
/// "Lulo OS <version>" item, exactly as `rmac-update-check` groups them.
pub const LULO_OS_PACKAGES: [&str; 5] = [
    "rmac-apps",
    "rmac-session",
    "rmac-archive-keyring",
    "niri",
    "xwayland-satellite",
];

/// The package whose version names a Lulo OS release, then its fallback.
const VERSION_PACKAGES: [&str; 2] = ["rmac-session", "rmac-apps"];

/// The "Lulo OS" item's key in selections, never a valid package ID.
pub const LULO_OS_ITEM: &str = "lulo-os";

pub fn is_lulo_os_package(name: &str) -> bool {
    LULO_OS_PACKAGES.contains(&name)
}

/// One Lulo OS release offered as a single update.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LuloOsUpdate {
    /// "0.9.1" or "0.9.0 Beta 2"; `None` when only a component such as
    /// niri changed and neither rmac package did.
    pub version: Option<String>,
    pub packages: Vec<Update>,
    pub download_size: Option<u64>,
    pub security: bool,
}

impl LuloOsUpdate {
    /// "Lulo OS 0.9.1", or "Lulo OS Update" for a component-only update.
    pub fn title(&self) -> String {
        match &self.version {
            Some(version) => format!("Lulo OS {version}"),
            None => "Lulo OS Update".into(),
        }
    }

    /// The Mac's "27 — 14.73 GB" subtitle.
    pub fn subtitle(&self) -> String {
        let version = self.version.clone().unwrap_or_else(|| {
            self.packages
                .iter()
                .map(|update| update.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        });
        match self.download_size {
            Some(size) => format!("{version} — {}", format_size(size)),
            None => version,
        }
    }

    pub fn package_ids(&self) -> Vec<String> {
        let mut ids = self
            .packages
            .iter()
            .map(|update| update.package_id.clone())
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }
}

/// A snapshot split the way the Mac's pane shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalog {
    pub lulo_os: Option<LuloOsUpdate>,
    /// Installable non-Lulo updates, sorted by name.
    pub other: Vec<Update>,
    pub blocked: usize,
}

impl Catalog {
    pub fn from_snapshot(snapshot: &Snapshot) -> Self {
        let mut lulo = Vec::new();
        let mut other = Vec::new();
        let mut blocked = 0;
        for update in &snapshot.updates {
            if update.kind == UpdateKind::Blocked {
                blocked += 1;
            } else if is_lulo_os_package(&update.name) {
                lulo.push(update.clone());
            } else {
                other.push(update.clone());
            }
        }
        other.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.package_id.cmp(&right.package_id))
        });
        let lulo_os = (!lulo.is_empty()).then(|| {
            lulo.sort_by(|left, right| left.package_id.cmp(&right.package_id));
            let version = VERSION_PACKAGES.iter().find_map(|name| {
                lulo.iter()
                    .find(|update| update.name == *name)
                    .map(|update| display_version(&update.version))
            });
            LuloOsUpdate {
                version,
                download_size: total_size(snapshot, &lulo),
                security: lulo.iter().any(|update| update.kind.is_security_relevant()),
                packages: lulo,
            }
        });
        Self {
            lulo_os,
            other,
            blocked,
        }
    }

    /// Items the pane lists, and the menu bar's "N updates" count: Lulo OS
    /// counts once, and all other updates together count once, as the
    /// Mac's single "Other Updates" row does.
    pub fn item_count(&self) -> usize {
        usize::from(self.lulo_os.is_some()) + usize::from(!self.other.is_empty())
    }

    pub fn is_empty(&self) -> bool {
        self.lulo_os.is_none() && self.other.is_empty()
    }

    /// "firefox, libc6 and 12 more…", the Mac's "macOS Tahoe 26.7 and 2
    /// more…".
    pub fn other_summary(&self) -> Option<String> {
        let names = distinct_names(&self.other);
        match names.as_slice() {
            [] => None,
            [one] => Some((*one).to_owned()),
            [first, second] => Some(format!("{first} and {second}")),
            [first, second, rest @ ..] => {
                Some(format!("{first}, {second} and {} more…", rest.len()))
            }
        }
    }

    pub fn other_ids(&self) -> Vec<String> {
        let mut ids = self
            .other
            .iter()
            .map(|update| update.package_id.clone())
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }

    pub fn other_size(&self, snapshot: &Snapshot) -> Option<u64> {
        total_size(snapshot, &self.other)
    }
}

fn distinct_names(updates: &[Update]) -> Vec<&str> {
    let mut names: Vec<&str> = Vec::new();
    for update in updates {
        if !names.contains(&update.name.as_str()) {
            names.push(&update.name);
        }
    }
    names
}

/// The sum of the known download sizes, or `None` when any is unknown, so a
/// partial total is never shown as the whole.
fn total_size(snapshot: &Snapshot, updates: &[Update]) -> Option<u64> {
    updates.iter().try_fold(0u64, |total, update| {
        snapshot
            .download_sizes
            .get(&update.package_id)
            .map(|size| total.saturating_add(*size))
    })
}

/// A Debian version as a person reads it: no epoch, no Debian revision, and
/// a `~beta.2` pre-release suffix as " Beta 2".
pub fn display_version(version: &str) -> String {
    let without_epoch = version
        .split_once(':')
        .filter(|(epoch, _)| !epoch.is_empty() && epoch.bytes().all(|b| b.is_ascii_digit()))
        .map_or(version, |(_, rest)| rest);
    let upstream = without_epoch
        .rsplit_once('-')
        .map_or(without_epoch, |(upstream, _)| upstream);
    match upstream.split_once('~') {
        Some((base, pre)) if !base.is_empty() && !pre.is_empty() => {
            let mut words = pre.split(['.', '-', '_']).filter(|word| !word.is_empty());
            let mut text = base.to_owned();
            for (index, word) in words.by_ref().enumerate() {
                text.push(' ');
                if index == 0 {
                    let mut chars = word.chars();
                    if let Some(first) = chars.next() {
                        text.extend(first.to_uppercase());
                        text.push_str(chars.as_str());
                    }
                } else {
                    text.push_str(word);
                }
            }
            text
        }
        _ => upstream.to_owned(),
    }
}

/// Decimal sizes the way macOS writes them: "942.5 MB", "14.73 GB",
/// "10 GB", "12 KB".
pub fn format_size(bytes: u64) -> String {
    const KB: f64 = 1_000.0;
    const MB: f64 = 1_000_000.0;
    const GB: f64 = 1_000_000_000.0;
    let value = bytes as f64;
    let (scaled, unit, decimals) = if value >= GB {
        (value / GB, "GB", 2)
    } else if value >= MB {
        (value / MB, "MB", 1)
    } else if value >= KB {
        (value / KB, "KB", 0)
    } else {
        return if bytes == 1 {
            "1 byte".into()
        } else {
            format!("{bytes} bytes")
        };
    };
    let mut text = format!("{scaled:.decimals$}");
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    format!("{text} {unit}")
}

/// The updates a Software Update action prepares: the selected items
/// ([`LULO_OS_ITEM`] or package IDs), plus whatever is already prepared
/// for the next restart and still offered, because PackageKit keeps only
/// one prepared set and a new download replaces it.
pub fn resolve_selection(snapshot: &Snapshot, selection: &[String]) -> Result<Vec<Update>, Error> {
    if snapshot.truncated {
        return Err(Error::new(
            ErrorKind::Protocol,
            "the complete update set is too large to confirm safely",
        ));
    }
    let lulo = selection.iter().any(|key| key == LULO_OS_ITEM);
    let requested = snapshot
        .installable_updates()
        .filter(|update| {
            (lulo && is_lulo_os_package(&update.name))
                || selection.contains(&update.package_id)
                || snapshot.offline.prepared.contains(&update.package_id)
        })
        .cloned()
        .collect::<Vec<_>>();
    if requested.is_empty() {
        return Err(Error::new(
            ErrorKind::Stale,
            "no installable updates remain",
        ));
    }
    Ok(requested)
}
