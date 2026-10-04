//! View ▸ Dock Icon: Lulo's own Dock (`crates/rmac-dock`) has no API for an
//! app to replace its tile's artwork or draw an animated graph into it — the
//! one piece of Dock-tile state an app can push live is the
//! `com.canonical.Unity.LauncherEntry` progress bar (`crates/rmac-dock/src/
//! badges.rs`, already used by Mail's unread badge). That is a single
//! scalar, not a bitmap, so only one honest live reading can be shown this
//! way; "Show CPU Usage" is it, matching the task's own fallback ("the
//! simplest honest one"). "Show CPU History", "Show Disk Activity" and
//! "Show Network Usage" are left out of the menu rather than wired to the
//! exact same scalar under a different name, which would be indistinguishable
//! from CPU Usage and so not a real, distinct option.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DockIconMode {
    /// The plain application icon — the Unity LauncherEntry progress bar
    /// turned off.
    #[default]
    Application,
    /// The progress bar driven by the host's current total CPU usage.
    CpuUsage,
}

/// Push the Dock tile's progress bar to match `mode`, given the host's
/// current overall CPU usage as a 0.0..=1.0 fraction (`None` before the
/// first sample is ready). Best-effort: a Dock that isn't running, or any
/// other D-Bus failure, is silently ignored, the same as Mail's own badge.
pub(crate) fn publish(mode: DockIconMode, cpu_fraction: Option<f64>) {
    let (progress, progress_visible) = launcher_properties(mode, cpu_fraction);
    publish_launcher_update(progress, progress_visible);
}

/// The pure half of `publish`: what the Dock tile's progress bar should
/// show for `mode`, given the host's current CPU usage fraction.
fn launcher_properties(mode: DockIconMode, cpu_fraction: Option<f64>) -> (f64, bool) {
    match mode {
        DockIconMode::Application => (0.0, false),
        DockIconMode::CpuUsage => (cpu_fraction.unwrap_or(0.0).clamp(0.0, 1.0), true),
    }
}

#[cfg(target_os = "linux")]
fn publish_launcher_update(progress: f64, progress_visible: bool) {
    use std::collections::HashMap;
    use zbus::zvariant::Value;

    if let Ok(connection) = zbus::blocking::Connection::session() {
        let mut properties = HashMap::new();
        properties.insert("progress", Value::F64(progress));
        properties.insert("progress-visible", Value::Bool(progress_visible));
        let _ = connection.emit_signal(
            None::<&str>,
            "/com/canonical/Unity/LauncherEntry",
            "com.canonical.Unity.LauncherEntry",
            "Update",
            &("application://org.rmac.SystemMonitor.desktop", properties),
        );
    }
}

#[cfg(not(target_os = "linux"))]
fn publish_launcher_update(progress: f64, progress_visible: bool) {
    let _ = (progress, progress_visible);
}

#[cfg(test)]
mod tests {
    use super::{launcher_properties, DockIconMode};

    #[test]
    fn application_mode_is_the_default_and_hides_the_progress_bar() {
        assert_eq!(DockIconMode::default(), DockIconMode::Application);
        assert_eq!(
            launcher_properties(DockIconMode::Application, Some(0.8)),
            (0.0, false)
        );
    }

    #[test]
    fn cpu_usage_mode_shows_the_clamped_fraction() {
        assert_eq!(
            launcher_properties(DockIconMode::CpuUsage, Some(0.42)),
            (0.42, true)
        );
        // Out-of-range or missing samples never crash the Dock tile.
        assert_eq!(
            launcher_properties(DockIconMode::CpuUsage, Some(5.0)),
            (1.0, true)
        );
        assert_eq!(
            launcher_properties(DockIconMode::CpuUsage, Some(-5.0)),
            (0.0, true)
        );
        assert_eq!(
            launcher_properties(DockIconMode::CpuUsage, None),
            (0.0, true)
        );
    }
}
