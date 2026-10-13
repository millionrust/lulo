//! Host-wide figures Activity Monitor's bottom panel shows that `sysinfo`
//! does not expose: the thread count (CPU tab) and the page cache (Memory
//! tab's "Cached Files"). Both come from `/proc` on Linux and from the
//! Windows performance counters (`windows_host`) there; elsewhere they are
//! unavailable and the panel omits the row rather than inventing a value.

/// Every kernel task (thread) on the host, from `/proc/loadavg`'s
/// `running/total` field.
#[cfg(target_os = "linux")]
pub(crate) fn thread_count() -> Option<u64> {
    let contents = read_bounded("/proc/loadavg")?;
    parse_loadavg_threads(&contents)
}

#[cfg(windows)]
pub(crate) fn thread_count() -> Option<u64> {
    crate::windows_host::performance_info().map(|info| u64::from(info.ThreadCount))
}

#[cfg(not(any(target_os = "linux", windows)))]
pub(crate) fn thread_count() -> Option<u64> {
    None
}

/// Bytes of file-backed page cache, from `/proc/meminfo`'s `Cached:` line.
#[cfg(target_os = "linux")]
pub(crate) fn cached_bytes() -> Option<u64> {
    let contents = read_bounded("/proc/meminfo")?;
    parse_meminfo_cached(&contents)
}

#[cfg(windows)]
pub(crate) fn cached_bytes() -> Option<u64> {
    crate::windows_host::performance_info()
        .map(|info| (info.SystemCache as u64).saturating_mul(info.PageSize as u64))
}

#[cfg(not(any(target_os = "linux", windows)))]
pub(crate) fn cached_bytes() -> Option<u64> {
    None
}

/// Cumulative block-device counters (MON-09), summed over whole physical
/// disks: the Disk tab's "Reads in", "Writes out", "Data read" and "Data
/// written".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DiskTotals {
    pub(crate) reads: u64,
    pub(crate) writes: u64,
    pub(crate) bytes_read: u64,
    pub(crate) bytes_written: u64,
}

/// Totals since boot from `/proc/diskstats`. Partitions, loop, RAM, zram,
/// device-mapper, RAID and optical devices are left out, so a byte is
/// counted once, on the disk it reached.
#[cfg(target_os = "linux")]
pub(crate) fn disk_totals() -> Option<DiskTotals> {
    let contents = read_bounded("/proc/diskstats")?;
    Some(parse_diskstats(&contents, |name| {
        !is_virtual_block_device(name) && std::path::Path::new("/sys/block").join(name).exists()
    }))
}

#[cfg(windows)]
pub(crate) fn disk_totals() -> Option<DiskTotals> {
    crate::windows_host::disk_totals()
}

#[cfg(not(any(target_os = "linux", windows)))]
pub(crate) fn disk_totals() -> Option<DiskTotals> {
    None
}

/// Memory stall share over the last 10 seconds, 0..=100, from the kernel's
/// pressure-stall information (`/proc/pressure/memory`, MON-08). `None`
/// when the kernel was built without PSI.
#[cfg(target_os = "linux")]
pub(crate) fn memory_pressure() -> Option<f32> {
    parse_psi_some_avg10(&read_bounded("/proc/pressure/memory")?)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn memory_pressure() -> Option<f32> {
    None
}

/// The Memory tab's green, yellow and red pressure bands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PressureLevel {
    Normal,
    Warning,
    Critical,
}

impl PressureLevel {
    /// Under 5 % of the last 10 seconds stalled on memory is ordinary
    /// reclaim; from 25 % the machine is visibly thrashing.
    pub(crate) fn from_stall_percent(percent: f32) -> Self {
        if percent < 5.0 {
            Self::Normal
        } else if percent < 25.0 {
            Self::Warning
        } else {
            Self::Critical
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Warning => "Warning",
            Self::Critical => "Critical",
        }
    }
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn is_virtual_block_device(name: &str) -> bool {
    ["loop", "ram", "zram", "dm-", "md", "sr", "fd", "nbd"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn parse_diskstats(contents: &str, counts: impl Fn(&str) -> bool) -> DiskTotals {
    let mut totals = DiskTotals::default();
    for line in contents.lines() {
        // major minor name reads merged sectors-read ms writes merged
        // sectors-written ...; a sector is always 512 bytes here.
        let mut fields = line.split_ascii_whitespace();
        let Some(name) = fields.nth(2) else {
            continue;
        };
        if !counts(name) {
            continue;
        }
        let values: Vec<u64> = fields.take(7).filter_map(|v| v.parse().ok()).collect();
        if values.len() < 7 {
            continue;
        }
        totals.reads = totals.reads.saturating_add(values[0]);
        totals.bytes_read = totals
            .bytes_read
            .saturating_add(values[2].saturating_mul(512));
        totals.writes = totals.writes.saturating_add(values[4]);
        totals.bytes_written = totals
            .bytes_written
            .saturating_add(values[6].saturating_mul(512));
    }
    totals
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn parse_psi_some_avg10(contents: &str) -> Option<f32> {
    let line = contents.lines().find(|line| line.starts_with("some "))?;
    let value: f32 = line
        .split_ascii_whitespace()
        .find_map(|field| field.strip_prefix("avg10="))?
        .parse()
        .ok()?;
    value.is_finite().then_some(value.clamp(0.0, 100.0))
}

/// Operations (or bytes) per second between two cumulative readings; a
/// counter that went backwards (device removed) reads as zero.
pub(crate) fn per_second(previous: u64, current: u64, elapsed_secs: f64) -> f64 {
    current.saturating_sub(previous) as f64 / elapsed_secs.max(0.001)
}

#[cfg(target_os = "linux")]
fn read_bounded(path: &str) -> Option<String> {
    use std::io::Read as _;

    let mut source = std::fs::File::open(path).ok()?.take(64 * 1024);
    let mut contents = String::new();
    source.read_to_string(&mut contents).ok()?;
    Some(contents)
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn parse_loadavg_threads(contents: &str) -> Option<u64> {
    let tasks = contents.split_ascii_whitespace().nth(3)?;
    let (_, total) = tasks.split_once('/')?;
    total.parse().ok()
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn parse_meminfo_cached(contents: &str) -> Option<u64> {
    let line = contents.lines().find(|line| line.starts_with("Cached:"))?;
    let mut fields = line.split_ascii_whitespace().skip(1);
    let value: u64 = fields.next()?.parse().ok()?;
    match fields.next() {
        Some("kB") | None => value.checked_mul(1024),
        Some(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loadavg_total_tasks_are_the_thread_count() {
        assert_eq!(
            parse_loadavg_threads("0.52 0.58 0.59 2/1234 56789\n"),
            Some(1234)
        );
        assert_eq!(parse_loadavg_threads("0.52 0.58\n"), None);
    }

    #[test]
    fn diskstats_sum_whole_disks_in_operations_and_bytes() {
        let stats = concat!(
            "   8       0 sda 100 5 2000 30 50 2 1000 20 0 10 50\n",
            "   8       1 sda1 90 5 1800 25 40 2 800 15 0 8 40\n",
            "   7       0 loop0 7 0 14 1 0 0 0 0 0 1 1\n",
            " 259       0 nvme0n1 10 0 100 3 20 0 400 5 0 4 8\n",
            " garbage\n",
        );
        let physical =
            |name: &str| !is_virtual_block_device(name) && matches!(name, "sda" | "nvme0n1");
        let totals = parse_diskstats(stats, physical);
        assert_eq!(totals.reads, 110);
        assert_eq!(totals.writes, 70);
        assert_eq!(totals.bytes_read, (2000 + 100) * 512);
        assert_eq!(totals.bytes_written, (1000 + 400) * 512);
        assert!(is_virtual_block_device("loop3") && is_virtual_block_device("dm-0"));
        assert!(!is_virtual_block_device("sda") && !is_virtual_block_device("nvme0n1"));
    }

    #[test]
    fn per_second_rates_never_go_negative() {
        assert_eq!(per_second(100, 250, 5.0), 30.0);
        assert_eq!(per_second(250, 100, 5.0), 0.0);
    }

    #[test]
    fn memory_pressure_is_the_psi_ten_second_stall_in_three_bands() {
        let psi = concat!(
            "some avg10=12.50 avg60=3.00 avg300=1.00 total=123456\n",
            "full avg10=4.00 avg60=1.00 avg300=0.50 total=1234\n",
        );
        assert_eq!(parse_psi_some_avg10(psi), Some(12.5));
        assert_eq!(parse_psi_some_avg10("full avg10=4.00\n"), None);
        assert_eq!(parse_psi_some_avg10("some avg10=nan\n"), None);
        assert_eq!(
            PressureLevel::from_stall_percent(0.0),
            PressureLevel::Normal
        );
        assert_eq!(
            PressureLevel::from_stall_percent(4.9),
            PressureLevel::Normal
        );
        assert_eq!(
            PressureLevel::from_stall_percent(5.0),
            PressureLevel::Warning
        );
        assert_eq!(
            PressureLevel::from_stall_percent(25.0),
            PressureLevel::Critical
        );
        assert_eq!(PressureLevel::Warning.label(), "Warning");
    }

    #[test]
    fn meminfo_cached_is_read_in_bytes() {
        let meminfo = "MemTotal: 8000 kB\nBuffers: 10 kB\nCached: 1310 kB\nSwapCached: 5 kB\n";
        assert_eq!(parse_meminfo_cached(meminfo), Some(1310 * 1024));
        assert_eq!(parse_meminfo_cached("MemTotal: 8000 kB\n"), None);
    }
}
