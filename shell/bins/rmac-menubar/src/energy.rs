//! The Battery menu's "Using Significant Energy" section (BAR-05).
//!
//! macOS lists the apps whose recent energy impact is high. Lulo has no
//! per-app power meter, so it uses the best stand-in Linux offers: CPU time.
//! When the menu opens, the processes of every app with a window are read
//! twice, `SAMPLE` apart, off the UI thread; an app whose processes kept
//! more than `THRESHOLD` of one core busy is listed. Nothing runs while the
//! menu is closed.

use std::collections::BTreeMap;
use std::time::Duration;

const SAMPLE: Duration = Duration::from_millis(400);
/// A quarter of one core, sustained over the sample.
const THRESHOLD: f64 = 0.25;
/// Linux's user-visible clock tick (`sysconf(_SC_CLK_TCK)`), 100 on every
/// supported kernel configuration.
const CLOCK_TICKS: f64 = 100.0;
/// The Mac lists a handful of apps at most.
const MAX_APPS: usize = 5;

/// The display names of apps using significant energy, busiest first; `None`
/// where CPU time cannot be read (not Linux).
pub fn significant(windows: &[(i32, String)]) -> Option<Vec<String>> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let mut apps: BTreeMap<&str, Vec<i32>> = BTreeMap::new();
    for (pid, app_id) in windows {
        let pids = apps.entry(app_id.as_str()).or_default();
        if !pids.contains(pid) {
            pids.push(*pid);
        }
    }
    let before = cpu_ticks(&apps);
    std::thread::sleep(SAMPLE);
    let after = cpu_ticks(&apps);
    let mut busy = apps
        .keys()
        .filter_map(|app| {
            let used = after.get(app)?.saturating_sub(*before.get(app)?) as f64;
            let share = used / CLOCK_TICKS / SAMPLE.as_secs_f64();
            (share >= THRESHOLD).then_some((*app, share))
        })
        .collect::<Vec<_>>();
    busy.sort_by(|left, right| right.1.total_cmp(&left.1));
    Some(
        busy.into_iter()
            .take(MAX_APPS)
            .map(|(app, _)| display_name(app))
            .collect(),
    )
}

fn cpu_ticks<'a>(apps: &BTreeMap<&'a str, Vec<i32>>) -> BTreeMap<&'a str, u64> {
    apps.iter()
        .map(|(app, pids)| {
            let ticks = pids
                .iter()
                .filter_map(|pid| std::fs::read_to_string(format!("/proc/{pid}/stat")).ok())
                .filter_map(|stat| process_ticks(&stat))
                .sum();
            (*app, ticks)
        })
        .collect()
}

/// utime + stime from one `/proc/<pid>/stat` line. The command name may
/// hold spaces and parentheses, so fields are counted after its last ')'.
fn process_ticks(stat: &str) -> Option<u64> {
    let rest = &stat[stat.rfind(')')? + 1..];
    let mut fields = rest.split_whitespace();
    // After the name: state (field 3), then 4…; utime is 14, stime 15.
    let utime = fields.nth(11)?.parse::<u64>().ok()?;
    let stime = fields.next()?.parse::<u64>().ok()?;
    Some(utime + stime)
}

fn display_name(app_id: &str) -> String {
    rmac_apps::identity::window_title(app_id)
        .map(str::to_owned)
        .or_else(|| {
            rmac_apps::discover_for_window(app_id)
                .ok()
                .flatten()
                .map(|entry| entry.name)
        })
        .unwrap_or_else(|| app_id.rsplit('.').next().unwrap_or(app_id).to_owned())
}

#[cfg(test)]
mod tests {
    use super::process_ticks;

    #[test]
    fn reads_user_and_system_time_past_a_tricky_command_name() {
        let stat = "4242 (Web (Content) x) S 1 4242 4242 0 -1 4194560 100 0 0 0 \
                    150 25 0 0 20 0 12 0 9000 0 0";
        assert_eq!(process_ticks(stat), Some(175));
        assert_eq!(process_ticks("garbage"), None);
    }
}
