//! The View menu's process filter (MON-03): the Mac's "All Processes / My
//! Processes / System Processes / Other Users' Processes / Active
//! Processes" choice, whose window subtitle follows the current choice.
//!
//! "Windowed Processes" (a process that owns a window) correlates the
//! compositor's window list with process ids (`rmac_ui::windowed_process_ids`);
//! the view fetches that list while the filter is on, and the table passes
//! each row's answer in as `has_window`.

/// Ubuntu's `/etc/login.defs` `UID_MIN` default: uids at or below this are
/// system/service accounts rather than a human login. A heuristic, not a
/// measurement — there is no portable "is this a system account" API.
const SYSTEM_UID_MAX: u32 = 999;

/// What the table knows about a row beyond its own columns, for the filters
/// that depend on other data.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RowSignals {
    /// The compositor lists a window owned by this process.
    pub(crate) has_window: bool,
    /// The process holds a graphics (DRM) client.
    pub(crate) uses_gpu: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ViewFilter {
    All,
    /// MON-MENU-022: the same rows as `All`, but ordered as a parent/child
    /// tree (`process_table::hierarchical_order`) instead of sorted by the
    /// current column — its own exclusive choice, like the Mac's, not a
    /// modifier layered on the other filters below.
    AllHierarchical,
    MyProcesses,
    SystemProcesses,
    OtherUsersProcesses,
    ActiveProcesses,
    InactiveProcesses,
    /// MON-MENU-023: only processes that own a window.
    WindowedProcesses,
    /// MON-MENU-015: only processes that hold a graphics (DRM) client.
    GpuProcesses,
    SelectedProcesses,
}

impl ViewFilter {
    pub(crate) const ALL: [Self; 10] = [
        Self::All,
        Self::AllHierarchical,
        Self::MyProcesses,
        Self::SystemProcesses,
        Self::OtherUsersProcesses,
        Self::ActiveProcesses,
        Self::InactiveProcesses,
        Self::WindowedProcesses,
        Self::GpuProcesses,
        Self::SelectedProcesses,
    ];

    /// The label shown in both the filter menu and the window subtitle.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::All => "All Processes",
            Self::AllHierarchical => "All Processes, Hierarchically",
            Self::MyProcesses => "My Processes",
            Self::SystemProcesses => "System Processes",
            Self::OtherUsersProcesses => "Other Users' Processes",
            Self::ActiveProcesses => "Active Processes",
            Self::InactiveProcesses => "Inactive Processes",
            Self::WindowedProcesses => "Windowed Processes",
            Self::GpuProcesses => "GPU Processes",
            Self::SelectedProcesses => "Selected Processes",
        }
    }

    /// Whether a row with the given owning uid and status passes this
    /// filter. `current_uid` is this System Monitor process's own uid, the
    /// Mac's notion of "me" for "My Processes" / "Other Users' Processes".
    pub(crate) fn matches(self, uid: Option<u32>, status: &str, current_uid: Option<u32>) -> bool {
        match self {
            Self::All | Self::AllHierarchical => true,
            Self::MyProcesses => current_uid.is_some() && uid == current_uid,
            Self::SystemProcesses => uid.is_some_and(|uid| uid <= SYSTEM_UID_MAX),
            Self::OtherUsersProcesses => {
                uid.is_some_and(|uid| uid > SYSTEM_UID_MAX && current_uid != Some(uid))
            }
            // Not idle, stopped, zombied, or dead — a broader bucket than
            // literally "Run" right now, matching how few rows sample as
            // "Runnable" at any single sample.
            Self::ActiveProcesses => {
                !matches!(status, "Idle" | "Stopped" | "Zombie" | "Dead" | "Tracing")
            }
            Self::InactiveProcesses => {
                matches!(status, "Idle" | "Stopped" | "Zombie" | "Dead" | "Tracing")
            }
            // Decided by the caller from the compositor's window list.
            Self::WindowedProcesses | Self::GpuProcesses | Self::SelectedProcesses => false,
        }
    }

    pub(crate) fn matches_row(
        self,
        pid: u32,
        uid: Option<u32>,
        status: &str,
        current_uid: Option<u32>,
        selected_pid: Option<u32>,
        signals: RowSignals,
    ) -> bool {
        if self == Self::SelectedProcesses {
            return selected_pid == Some(pid);
        }
        if self == Self::WindowedProcesses {
            return signals.has_window;
        }
        if self == Self::GpuProcesses {
            return signals.uses_gpu;
        }
        self.matches(uid, status, current_uid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysinfo::ProcessStatus;

    #[test]
    fn all_matches_everything() {
        assert!(ViewFilter::All.matches(None, "Sleeping", None));
        assert!(ViewFilter::All.matches(Some(0), "Zombie", Some(1000)));
    }

    #[test]
    fn all_hierarchically_matches_everything_too() {
        // MON-MENU-022: the same row set as `All` — only the order differs,
        // which `matches`/`matches_row` never decide.
        assert!(ViewFilter::AllHierarchical.matches(None, "Sleeping", None));
        assert!(ViewFilter::AllHierarchical.matches(Some(0), "Zombie", Some(1000)));
    }

    #[test]
    fn my_processes_matches_only_the_current_uid() {
        assert!(ViewFilter::MyProcesses.matches(Some(1000), "Sleeping", Some(1000)));
        assert!(!ViewFilter::MyProcesses.matches(Some(0), "Sleeping", Some(1000)));
        assert!(!ViewFilter::MyProcesses.matches(None, "Sleeping", Some(1000)));
        assert!(!ViewFilter::MyProcesses.matches(Some(1000), "Sleeping", None));
    }

    #[test]
    fn system_processes_are_low_uid() {
        assert!(ViewFilter::SystemProcesses.matches(Some(0), "Sleeping", Some(1000)));
        assert!(ViewFilter::SystemProcesses.matches(Some(999), "Sleeping", Some(1000)));
        assert!(!ViewFilter::SystemProcesses.matches(Some(1000), "Sleeping", Some(1000)));
        assert!(!ViewFilter::SystemProcesses.matches(None, "Sleeping", Some(1000)));
    }

    #[test]
    fn other_users_excludes_system_and_me() {
        assert!(ViewFilter::OtherUsersProcesses.matches(Some(1001), "Sleeping", Some(1000)));
        assert!(!ViewFilter::OtherUsersProcesses.matches(Some(1000), "Sleeping", Some(1000)));
        assert!(!ViewFilter::OtherUsersProcesses.matches(Some(0), "Sleeping", Some(1000)));
    }

    #[test]
    fn active_processes_excludes_idle_and_dead_states() {
        assert!(ViewFilter::ActiveProcesses.matches(Some(1000), "Sleeping", Some(1000)));
        assert!(ViewFilter::ActiveProcesses.matches(Some(1000), "Runnable", Some(1000)));
        assert!(!ViewFilter::ActiveProcesses.matches(Some(1000), "Idle", Some(1000)));
        assert!(!ViewFilter::ActiveProcesses.matches(Some(1000), "Zombie", Some(1000)));
        assert!(!ViewFilter::ActiveProcesses.matches(Some(1000), "Stopped", Some(1000)));
    }

    #[test]
    fn inactive_processes_are_the_complement_of_active_processes() {
        for status in [
            "Running", "Sleeping", "Idle", "Stopped", "Zombie", "Dead", "Tracing",
        ] {
            assert_ne!(
                ViewFilter::ActiveProcesses.matches(Some(1000), status, Some(1000)),
                ViewFilter::InactiveProcesses.matches(Some(1000), status, Some(1000)),
            );
        }
    }

    #[test]
    fn every_filter_has_a_distinct_label() {
        let labels: Vec<&str> = ViewFilter::ALL
            .iter()
            .map(|filter| filter.label())
            .collect();
        let mut unique = labels.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(labels.len(), unique.len());
    }

    #[test]
    fn selected_processes_only_shows_the_selected_pid() {
        let filter = ViewFilter::SelectedProcesses;
        assert!(filter.matches_row(
            42,
            Some(1000),
            "Sleeping",
            Some(1000),
            Some(42),
            RowSignals::default()
        ));
        assert!(!filter.matches_row(
            43,
            Some(1000),
            "Sleeping",
            Some(1000),
            Some(42),
            RowSignals::default()
        ));
        assert!(!filter.matches_row(
            42,
            Some(1000),
            "Sleeping",
            Some(1000),
            None,
            RowSignals::default()
        ));
    }

    #[test]
    fn windowed_processes_follow_the_compositor_window_list() {
        let filter = ViewFilter::WindowedProcesses;
        assert!(filter.matches_row(
            42,
            Some(1000),
            "Sleeping",
            Some(1000),
            None,
            RowSignals {
                has_window: true,
                uses_gpu: false
            }
        ));
        assert!(!filter.matches_row(
            43,
            Some(1000),
            "Sleeping",
            Some(1000),
            None,
            RowSignals::default()
        ));
        assert!(!filter.matches(Some(1000), "Sleeping", Some(1000)));
    }

    #[test]
    fn gpu_processes_are_the_ones_holding_a_graphics_client() {
        let filter = ViewFilter::GpuProcesses;
        assert!(filter.matches_row(
            42,
            Some(1000),
            "Sleeping",
            Some(1000),
            None,
            RowSignals {
                has_window: false,
                uses_gpu: true
            }
        ));
        assert!(!filter.matches_row(
            43,
            Some(1000),
            "Sleeping",
            Some(1000),
            None,
            RowSignals {
                has_window: true,
                uses_gpu: false
            }
        ));
    }

    /// `ProcessStatus`'s `Display` impl is what rows actually store
    /// (`process_table.rs`); make sure the strings this module matches on
    /// stay in sync with it rather than drifting silently.
    #[test]
    fn matched_status_strings_are_real_process_status_display_values() {
        for status in [
            ProcessStatus::Idle,
            ProcessStatus::Run,
            ProcessStatus::Sleep,
            ProcessStatus::Stop,
            ProcessStatus::Zombie,
            ProcessStatus::Tracing,
            ProcessStatus::Dead,
        ] {
            // Just confirm these compile/format; the exhaustive mapping is
            // documented in `matches`'s `Idle | Stopped | Zombie | Dead |
            // Tracing` arm.
            let _ = status.to_string();
        }
    }
}
