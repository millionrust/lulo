//! Real host-wide CPU tick counters from Mach on macOS and `/proc/stat` on Linux.
//! The difference of two reads gives the live User / System / Idle split.

#[cfg(target_os = "macos")]
const HOST_CPU_LOAD_INFO: i32 = 3;
#[cfg(target_os = "macos")]
const CPU_STATE_USER: usize = 0;
#[cfg(target_os = "macos")]
const CPU_STATE_SYSTEM: usize = 1;
#[cfg(target_os = "macos")]
const CPU_STATE_IDLE: usize = 2;
#[cfg(target_os = "macos")]
const CPU_STATE_NICE: usize = 3;

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct HostCpuLoadInfo {
    cpu_ticks: [u32; 4],
}

#[cfg(target_os = "macos")]
extern "C" {
    fn mach_host_self() -> u32;
    fn host_statistics(
        host_priv: u32,
        flavor: i32,
        host_info_out: *mut u32,
        host_info_out_cnt: *mut u32,
    ) -> i32;
}

/// Cumulative CPU ticks since boot, summed across all cores:
/// `[user, system, idle, nice]`. `None` if the mach call fails.
#[cfg(target_os = "macos")]
pub fn read() -> Option<[u64; 4]> {
    let mut info = HostCpuLoadInfo::default();
    // HOST_CPU_LOAD_INFO_COUNT = size_of::<HostCpuLoadInfo>() / size_of::<integer_t>()
    let mut count = (std::mem::size_of::<HostCpuLoadInfo>() / std::mem::size_of::<u32>()) as u32;
    let kr = unsafe {
        host_statistics(
            mach_host_self(),
            HOST_CPU_LOAD_INFO,
            &mut info as *mut _ as *mut u32,
            &mut count,
        )
    };
    if kr != 0 {
        return None;
    }
    Some([
        info.cpu_ticks[CPU_STATE_USER] as u64,
        info.cpu_ticks[CPU_STATE_SYSTEM] as u64,
        info.cpu_ticks[CPU_STATE_IDLE] as u64,
        info.cpu_ticks[CPU_STATE_NICE] as u64,
    ])
}

#[cfg(target_os = "linux")]
pub fn read() -> Option<[u64; 4]> {
    use std::io::Read as _;

    let mut source = std::fs::File::open("/proc/stat").ok()?.take(4096);
    let mut contents = String::new();
    source.read_to_string(&mut contents).ok()?;
    parse_linux_cpu_ticks(&contents)
}

#[cfg(target_os = "linux")]
fn parse_linux_cpu_ticks(contents: &str) -> Option<[u64; 4]> {
    let mut fields = contents.lines().next()?.split_ascii_whitespace();
    if fields.next()? != "cpu" {
        return None;
    }
    let values = fields
        .take(10)
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if values.len() < 4 {
        return None;
    }
    let value = |index| values.get(index).copied().unwrap_or(0);
    Some([
        value(0),
        value(2)
            .saturating_add(value(5))
            .saturating_add(value(6))
            .saturating_add(value(7)),
        value(3).saturating_add(value(4)),
        value(1),
    ])
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn read() -> Option<[u64; 4]> {
    None
}

/// User / System / Idle percentages from the delta between two tick reads.
/// User folds in nice time (as macOS Activity Monitor does).
pub fn split(prev: [u64; 4], now: [u64; 4]) -> Option<(f32, f32, f32)> {
    let d = |i: usize| now[i].saturating_sub(prev[i]) as f64;
    let (user, system, idle, nice) = (d(0), d(1), d(2), d(3));
    let total = user + system + idle + nice;
    if total <= 0.0 {
        return None;
    }
    Some((
        (((user + nice) / total) * 100.0) as f32,
        ((system / total) * 100.0) as f32,
        ((idle / total) * 100.0) as f32,
    ))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn linux_cpu_ticks_preserve_the_user_system_idle_nice_contract() {
        assert_eq!(
            parse_linux_cpu_ticks("cpu  100 20 30 400 50 6 7 8 0 0\ncpu0 0 0 0 0\n"),
            Some([100, 51, 450, 20])
        );
        assert_eq!(parse_linux_cpu_ticks("intr 1 2 3\n"), None);
    }
}
