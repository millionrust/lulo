//! Real host-wide CPU tick counters via the mach `host_statistics` call, so the
//! CPU pane can show the genuine macOS User / System / Idle load split (the
//! difference of two reads over the refresh interval gives the live split).
//!
//! `sysinfo` only exposes total busy percentage, not the per-state breakdown,
//! so this minimal FFI reads `HOST_CPU_LOAD_INFO` directly — no fabricated data.

const HOST_CPU_LOAD_INFO: i32 = 3;
const CPU_STATE_USER: usize = 0;
const CPU_STATE_SYSTEM: usize = 1;
const CPU_STATE_IDLE: usize = 2;
const CPU_STATE_NICE: usize = 3;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct HostCpuLoadInfo {
    cpu_ticks: [u32; 4],
}

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
