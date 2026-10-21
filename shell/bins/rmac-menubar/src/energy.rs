//! The Battery menu's "Using Significant Energy" section (BAR-05).
//!
//! macOS lists the apps whose recent energy impact is high. Lulo has no
//! per-app power meter, so it uses the best stand-in Linux and Windows
//! offer: CPU time (`/proc/<pid>/stat`; `GetProcessTimes`).
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
#[cfg(any(not(windows), test))]
const CLOCK_TICKS: f64 = 100.0;
/// The Mac lists a handful of apps at most.
const MAX_APPS: usize = 5;

/// The display names of apps using significant energy, busiest first; `None`
/// where CPU time cannot be read (neither Linux nor Windows).
pub fn significant(windows: &[(i32, String)]) -> Option<Vec<String>> {
    if !cfg!(any(target_os = "linux", windows)) {
        return None;
    }
    let mut apps: BTreeMap<&str, Vec<i32>> = BTreeMap::new();
    for (pid, app_id) in windows {
        let pids = apps.entry(app_id.as_str()).or_default();
        if !pids.contains(pid) {
            pids.push(*pid);
        }
    }
    let before = cpu_seconds(&apps);
    std::thread::sleep(SAMPLE);
    let after = cpu_seconds(&apps);
    let mut busy = apps
        .keys()
        .filter_map(|app| {
            let used = (after.get(app)? - before.get(app)?).max(0.0);
            let share = used / SAMPLE.as_secs_f64();
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

/// Each app's processes' CPU time so far, in seconds.
fn cpu_seconds<'a>(apps: &BTreeMap<&'a str, Vec<i32>>) -> BTreeMap<&'a str, f64> {
    apps.iter()
        .map(|(app, pids)| {
            let seconds = pids.iter().filter_map(|pid| process_seconds(*pid)).sum();
            (*app, seconds)
        })
        .collect()
}

#[cfg(not(windows))]
fn process_seconds(pid: i32) -> Option<f64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    Some(process_ticks(&stat)? as f64 / CLOCK_TICKS)
}

/// Kernel and user time from `GetProcessTimes`, in 100 ns units.
#[cfg(windows)]
fn process_seconds(pid: i32) -> Option<f64> {
    use windows::Win32::Foundation::{CloseHandle, FILETIME};
    use windows::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    let pid = u32::try_from(pid).ok()?;
    // SAFETY: opens a process for a limited query, reads its times into
    // FILETIMEs and closes it.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let (mut created, mut exited, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        let read = GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user);
        let _ = CloseHandle(process);
        read.ok()?;
        let units =
            |time: FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
        Some((units(kernel) + units(user)) as f64 / 10_000_000.0)
    }
}

/// utime + stime from one `/proc/<pid>/stat` line. The command name may
/// hold spaces and parentheses, so fields are counted after its last ')'.
#[cfg(any(not(windows), test))]
fn process_ticks(stat: &str) -> Option<u64> {
    let rest = &stat[stat.rfind(')')? + 1..];
    let mut fields = rest.split_whitespace();
    // After the name: state (field 3), then 4…; utime is 14, stime 15.
    let utime = fields.nth(11)?.parse::<u64>().ok()?;
    let stime = fields.next()?.parse::<u64>().ok()?;
    Some(utime + stime)
}

fn display_name(app_id: &str) -> String {
    #[cfg(windows)]
    if let Some(name) = rmac_apps::windows_apps::app(app_id)
        .map(|app| app.name.to_owned())
        .or_else(|| rmac_apps::windows_apps::display_name_for(app_id))
    {
        return name;
    }
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
