use crate::columns::ColKey;

/// Which top-level pane is active. Each tab re-focuses the table on a different
/// metric and surfaces its corresponding bottom summary panel.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    Cpu,
    Memory,
    Energy,
    Disk,
    Network,
}

impl Tab {
    pub(crate) const ALL: [Self; 5] = [
        Self::Cpu,
        Self::Memory,
        Self::Energy,
        Self::Disk,
        Self::Network,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Memory => "Memory",
            Self::Energy => "Energy",
            Self::Disk => "Disk",
            Self::Network => "Network",
        }
    }

    /// The descending table sort adopted when this tab is activated.
    ///
    /// Network is summary-only, so its returned column is never displayed.
    pub(crate) fn default_sort_key(self) -> ColKey {
        match self {
            Self::Cpu => ColKey::Cpu,
            Self::Memory => ColKey::Mem,
            Self::Energy => ColKey::Energy,
            Self::Disk => ColKey::Disk,
            Self::Network => ColKey::Cpu,
        }
    }

    pub(crate) fn has_process_table(self) -> bool {
        !matches!(self, Self::Network)
    }
}

/// Index into [`Tab::ALL`] that Left, Right, Home or End move to from
/// `current` — the toolbar's CPU/Memory/Energy/Disk/Network pill strip's
/// roving arrow-key selection, matching the same left-to-right-only
/// convention `rmac_ui::SegmentedControl` uses for its own horizontal strip
/// (no Up/Down, since this is a row, not a set). `None` for any other key.
pub(crate) fn tab_roving_target(current: usize, key: &str) -> Option<usize> {
    let len = Tab::ALL.len();
    match key {
        "left" => Some((current + len - 1) % len),
        "right" => Some((current + 1) % len),
        "home" => Some(0),
        "end" => Some(len - 1),
        _ => None,
    }
}

/// Aggregate readings recomputed every tick for summaries and graphs.
#[derive(Default)]
pub(crate) struct Aggregates {
    pub(crate) mem_used: u64,
    pub(crate) mem_total: u64,
    pub(crate) swap_used: u64,
    pub(crate) energy_total: f32,
    pub(crate) disk_read_rate: f64,
    pub(crate) disk_write_rate: f64,
    pub(crate) net_recv_rate: f64,
    pub(crate) net_sent_rate: f64,
    /// Bytes received / sent by every interface since boot.
    pub(crate) net_total_recv: u64,
    pub(crate) net_total_sent: u64,
    /// Host thread count, when the platform exposes it.
    pub(crate) threads: Option<u64>,
    /// File-backed page cache ("Cached Files"), when the platform exposes it.
    pub(crate) mem_cached: Option<u64>,
}

/// A bounded ring of recent samples for each metric.
#[derive(Default)]
pub(crate) struct History {
    /// User and System CPU percentages, drawn as the CPU LOAD graph's blue
    /// and red series.
    pub(crate) cpu_user: Vec<f32>,
    pub(crate) cpu_system: Vec<f32>,
    pub(crate) mem: Vec<f32>,
    pub(crate) energy: Vec<f32>,
    /// Bytes per second read / written, the Disk graph's two series.
    pub(crate) disk_read: Vec<f32>,
    pub(crate) disk_write: Vec<f32>,
    /// Bytes per second received / sent, the Network graph's two series.
    pub(crate) net_recv: Vec<f32>,
    pub(crate) net_sent: Vec<f32>,
}

impl History {
    pub(crate) const CAP: usize = 60;

    pub(crate) fn push(samples: &mut Vec<f32>, value: f32) {
        samples.push(value);
        if samples.len() > Self::CAP {
            samples.remove(0);
        }
    }
}

/// One system-wide network-interface sample.
#[derive(Clone)]
pub(crate) struct NetIface {
    pub(crate) name: String,
    pub(crate) total_recv: u64,
    pub(crate) total_sent: u64,
    pub(crate) recv_rate: f64,
    pub(crate) sent_rate: f64,
}

pub(crate) const REFRESH_SECS: f64 = 2.0;

pub(crate) fn format_mem(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let bytes = bytes as f64;
    if bytes >= GB {
        format!("{:.2} GB", bytes / GB)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes / MB)
    } else {
        format!("{:.0} KB", bytes / KB)
    }
}

/// Format elapsed seconds as `H:MM:SS`, or `M:SS` under an hour.
pub(crate) fn format_duration(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

pub(crate) fn format_rate(bytes_per_second: f64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    if bytes_per_second >= GB {
        format!("{:.2} GB/s", bytes_per_second / GB)
    } else if bytes_per_second >= MB {
        format!("{:.1} MB/s", bytes_per_second / MB)
    } else {
        format!("{:.0} KB/s", bytes_per_second / KB)
    }
}

pub(crate) fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let value = bytes as f64;
    if value >= GB {
        format!("{:.2} GB", value / GB)
    } else if value >= MB {
        format!("{:.1} MB", value / MB)
    } else if value >= KB {
        format!("{:.0} KB", value / KB)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_history_and_refresh_policy_are_strictly_bounded() {
        let mut history = Vec::new();
        for sample in 0..(History::CAP + 5) {
            History::push(&mut history, sample as f32);
        }
        assert_eq!(history.len(), History::CAP);
        assert_eq!(history[0], 5.0);
        assert_eq!(REFRESH_SECS, 2.0);
    }

    #[test]
    fn metric_formatting_preserves_exact_units_and_thresholds() {
        assert_eq!(format_mem(1024), "1 KB");
        assert_eq!(format_mem(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1 KB");
        assert_eq!(format_rate(1024.0 * 1024.0), "1.0 MB/s");
        assert_eq!(format_duration(59), "0:59");
        assert_eq!(format_duration(3601), "1:00:01");
    }

    #[test]
    fn network_is_the_only_summary_only_tab() {
        assert!(Tab::ALL
            .into_iter()
            .filter(|tab| !tab.has_process_table())
            .eq([Tab::Network]));
        assert_eq!(Tab::Cpu.default_sort_key(), ColKey::Cpu);
        assert_eq!(Tab::Memory.default_sort_key(), ColKey::Mem);
        assert_eq!(Tab::Energy.default_sort_key(), ColKey::Energy);
        assert_eq!(Tab::Disk.default_sort_key(), ColKey::Disk);
    }

    #[test]
    fn tab_roving_wraps_at_both_ends() {
        assert_eq!(tab_roving_target(0, "left"), Some(4));
        assert_eq!(tab_roving_target(4, "right"), Some(0));
    }

    #[test]
    fn tab_roving_steps_by_one() {
        assert_eq!(tab_roving_target(1, "right"), Some(2));
        assert_eq!(tab_roving_target(2, "left"), Some(1));
    }

    #[test]
    fn tab_roving_home_and_end_jump_to_the_edges() {
        assert_eq!(tab_roving_target(2, "home"), Some(0));
        assert_eq!(tab_roving_target(2, "end"), Some(4));
    }

    #[test]
    fn tab_roving_ignores_unrelated_keys() {
        assert_eq!(tab_roving_target(2, "up"), None);
        assert_eq!(tab_roving_target(2, "down"), None);
        assert_eq!(tab_roving_target(2, "tab"), None);
    }
}
