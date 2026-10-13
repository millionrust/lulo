//! Optional per-process figures read from `/proc` (MON-MENU-003/004/006/007/
//! 008/009/011/015/016/017): the Mac's Memory, Real Private Memory, Real
//! Shared Memory, Compressed Memory, Kind, Sandbox, Idle Wake Ups, % GPU and
//! GPU Time columns, and the GPU Processes filter.
//!
//! These cost one or two extra file reads per process, so they are only
//! gathered while at least one of those columns is visible
//! ([`ColKey::needs_extra`]); with them hidden the monitor does exactly the
//! work it did before. Every reader returns `None` when the file is
//! unreadable (another user's process, an exited process, a kernel thread),
//! and the table shows "—" instead of a false zero.

/// What `/proc/<pid>/smaps_rollup` reports, in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SmapsRollup {
    /// Proportional set size: the Mac's per-process "Memory" footprint, where
    /// shared pages are split fairly between the processes mapping them.
    pub(crate) pss: u64,
    /// Private_Clean + Private_Dirty: "Real Private Memory".
    pub(crate) private: u64,
    /// Shared_Clean + Shared_Dirty: "Real Shared Memory".
    pub(crate) shared: u64,
    /// SwapPss: this process's share of pages pushed out to swap or zram,
    /// the closest Linux figure to the Mac's "Compressed Memory".
    pub(crate) swapped: u64,
}

/// The extra figures for one process; every field is `None` until read.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ProcExtra {
    pub(crate) rollup: Option<SmapsRollup>,
    pub(crate) kind: Option<&'static str>,
    pub(crate) sandboxed: Option<bool>,
    /// Voluntary context switches since the previous sample: how often the
    /// process woke from sleep, the Linux counterpart of Idle Wake Ups.
    pub(crate) wakeups: Option<u64>,
    /// The process's graphics-engine usage, when it holds a DRM client.
    pub(crate) gpu: Option<GpuUsage>,
    /// Busiest-engine utilisation since the previous sample, 0..=100.
    pub(crate) gpu_percent: Option<f32>,
}

/// What a process's open DRM clients report in `/proc/<pid>/fdinfo`: busy
/// time per engine (`drm-engine-<name>: N ns`, i915 and amdgpu) or busy
/// and total cycles per engine (`drm-cycles-<name>` with
/// `drm-total-cycles-<name>`, xe).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct GpuUsage {
    pub(crate) engines: Vec<(String, u64)>,
    pub(crate) cycles: Vec<(String, u64, u64)>,
}

impl GpuUsage {
    fn add_engine(&mut self, name: &str, nanoseconds: u64) {
        match self.engines.iter_mut().find(|(known, _)| known == name) {
            Some((_, total)) => *total = total.saturating_add(nanoseconds),
            None => self.engines.push((name.to_owned(), nanoseconds)),
        }
    }

    fn add_cycles(&mut self, name: &str, busy: u64, total: u64) {
        match self.cycles.iter_mut().find(|(known, _, _)| known == name) {
            Some((_, known_busy, known_total)) => {
                *known_busy = known_busy.saturating_add(busy);
                *known_total = known_total.saturating_add(total);
            }
            None => self.cycles.push((name.to_owned(), busy, total)),
        }
    }

    /// Total busy time across engines, for the GPU Time column; `None` when
    /// the driver reports cycles instead of time.
    pub(crate) fn total_nanoseconds(&self) -> Option<u64> {
        (!self.engines.is_empty()).then(|| {
            self.engines
                .iter()
                .fold(0u64, |sum, (_, ns)| sum.saturating_add(*ns))
        })
    }
}

/// One DRM client's usage from a `fdinfo` file, with the id that tells
/// duplicated descriptors of the same client apart. `None` for a descriptor
/// that is not a DRM client or reports no engine usage.
pub(crate) fn parse_drm_fdinfo(text: &str) -> Option<(Option<String>, GpuUsage)> {
    let mut usage = GpuUsage::default();
    let mut client = None;
    let mut is_drm = false;
    let mut busy_cycles: Vec<(String, u64)> = Vec::new();
    let mut total_cycles: Vec<(String, u64)> = Vec::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if key == "drm-driver" {
            is_drm = true;
        } else if key == "drm-client-id" {
            client = Some(value.to_owned());
        } else if let Some(name) = key.strip_prefix("drm-total-cycles-") {
            if let Ok(total) = value.parse() {
                total_cycles.push((name.to_owned(), total));
            }
        } else if let Some(name) = key.strip_prefix("drm-cycles-") {
            if let Ok(busy) = value.parse() {
                busy_cycles.push((name.to_owned(), busy));
            }
        } else if let Some(name) = key.strip_prefix("drm-engine-") {
            if name.starts_with("capacity-") {
                continue;
            }
            let digits = value.strip_suffix("ns").unwrap_or(value).trim();
            if let Ok(nanoseconds) = digits.parse() {
                usage.add_engine(name, nanoseconds);
            }
        }
    }
    for (name, busy) in busy_cycles {
        if let Some((_, total)) = total_cycles.iter().find(|(known, _)| *known == name) {
            usage.add_cycles(&name, busy, *total);
        }
    }
    (is_drm && (!usage.engines.is_empty() || !usage.cycles.is_empty())).then_some((client, usage))
}

/// Add every client in `clients` once (by id) into one process total.
pub(crate) fn merge_clients(clients: Vec<(Option<String>, GpuUsage)>) -> GpuUsage {
    let mut seen: Vec<String> = Vec::new();
    let mut total = GpuUsage::default();
    for (client, usage) in clients {
        if let Some(id) = client {
            if seen.contains(&id) {
                continue;
            }
            seen.push(id);
        }
        for (name, nanoseconds) in usage.engines {
            total.add_engine(&name, nanoseconds);
        }
        for (name, busy, all) in usage.cycles {
            total.add_cycles(&name, busy, all);
        }
    }
    total
}

/// The busiest engine's utilisation between two readings, 0..=100. `None`
/// when no engine can be compared or no time passed.
pub(crate) fn gpu_percent(
    previous: &GpuUsage,
    current: &GpuUsage,
    elapsed_nanoseconds: u64,
) -> Option<f32> {
    let mut best: Option<f32> = None;
    let mut consider = |percent: f32| {
        best = Some(best.map_or(percent, |known| known.max(percent)));
    };
    if elapsed_nanoseconds > 0 {
        for (name, now) in &current.engines {
            if let Some((_, before)) = previous.engines.iter().find(|(known, _)| known == name) {
                let busy = now.saturating_sub(*before) as f64;
                consider((busy / elapsed_nanoseconds as f64 * 100.0) as f32);
            }
        }
    }
    for (name, busy, total) in &current.cycles {
        if let Some((_, before_busy, before_total)) =
            previous.cycles.iter().find(|(known, _, _)| known == name)
        {
            let elapsed = total.saturating_sub(*before_total) as f64;
            if elapsed > 0.0 {
                let used = busy.saturating_sub(*before_busy) as f64;
                consider((used / elapsed * 100.0) as f32);
            }
        }
    }
    best.map(|percent| percent.clamp(0.0, 100.0))
}

/// Read a process's DRM clients. Looks only at descriptors that point at a
/// `/dev/dri` node, so the cost is one `readlink` per open descriptor.
#[cfg(target_os = "linux")]
pub(crate) fn read_gpu(pid: u32) -> Option<GpuUsage> {
    let descriptors = std::fs::read_dir(format!("/proc/{pid}/fd")).ok()?;
    let mut clients = Vec::new();
    for entry in descriptors.flatten() {
        let Ok(target) = std::fs::read_link(entry.path()) else {
            continue;
        };
        if !target.starts_with("/dev/dri/") {
            continue;
        }
        let Some(fd) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/fdinfo/{fd}")) {
            if let Some(client) = parse_drm_fdinfo(&text) {
                clients.push(client);
            }
        }
    }
    (!clients.is_empty()).then(|| merge_clients(clients))
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn read_gpu(_pid: u32) -> Option<GpuUsage> {
    None
}

fn kb_field(line: &str, name: &str) -> Option<u64> {
    let rest = line.strip_prefix(name)?.strip_prefix(':')?;
    let kb: u64 = rest.trim().strip_suffix("kB")?.trim().parse().ok()?;
    Some(kb.saturating_mul(1024))
}

/// Parse `smaps_rollup` text. `None` when it has no `Pss:` line (a kernel
/// thread's file is empty).
pub(crate) fn parse_smaps_rollup(text: &str) -> Option<SmapsRollup> {
    let mut rollup = SmapsRollup::default();
    let mut seen_pss = false;
    for line in text.lines() {
        if let Some(value) = kb_field(line, "Pss") {
            rollup.pss = value;
            seen_pss = true;
        } else if let Some(value) = kb_field(line, "Private_Clean") {
            rollup.private = rollup.private.saturating_add(value);
        } else if let Some(value) = kb_field(line, "Private_Dirty") {
            rollup.private = rollup.private.saturating_add(value);
        } else if let Some(value) = kb_field(line, "Shared_Clean") {
            rollup.shared = rollup.shared.saturating_add(value);
        } else if let Some(value) = kb_field(line, "Shared_Dirty") {
            rollup.shared = rollup.shared.saturating_add(value);
        } else if let Some(value) = kb_field(line, "SwapPss") {
            rollup.swapped = value;
        }
    }
    seen_pss.then_some(rollup)
}

/// `voluntary_ctxt_switches` from `/proc/<pid>/status` text.
pub(crate) fn parse_voluntary_switches(status: &str) -> Option<u64> {
    status
        .lines()
        .find_map(|line| line.strip_prefix("voluntary_ctxt_switches:"))
        .and_then(|value| value.trim().parse().ok())
}

/// True when `/proc/<pid>/status` shows a seccomp filter (`Seccomp: 2`),
/// which is how Flatpak, Snap, browsers and hardened services confine a
/// process: the Linux meaning of the Mac's Sandbox column.
pub(crate) fn parse_sandboxed(status: &str) -> Option<bool> {
    let mode = status
        .lines()
        .find_map(|line| line.strip_prefix("Seccomp:"))?;
    Some(mode.trim() == "2")
}

/// The architecture an ELF executable was built for, from its header: the
/// Mac's Kind column says Apple or Intel, here it says what the binary is.
pub(crate) fn elf_kind(header: &[u8]) -> Option<&'static str> {
    if header.len() < 20 || &header[..4] != b"\x7fELF" {
        return None;
    }
    let little = match header[5] {
        1 => true,
        2 => false,
        _ => return None,
    };
    let machine = if little {
        u16::from_le_bytes([header[18], header[19]])
    } else {
        u16::from_be_bytes([header[18], header[19]])
    };
    Some(match machine {
        0x03 => "Intel 32-bit",
        0x3E => "Intel 64-bit",
        0xB7 => "ARM 64-bit",
        0x28 => "ARM 32-bit",
        0xF3 => "RISC-V",
        _ => "Other",
    })
}

/// Switches this sample minus the previous one. `None` for a process first
/// seen now (no baseline) and for a counter that went backwards (PID reuse).
pub(crate) fn wakeup_delta(previous: Option<u64>, current: u64) -> Option<u64> {
    previous.and_then(|previous| current.checked_sub(previous))
}

#[cfg(target_os = "linux")]
pub(crate) fn read_rollup(pid: u32) -> Option<SmapsRollup> {
    parse_smaps_rollup(&std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).ok()?)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn read_rollup(_pid: u32) -> Option<SmapsRollup> {
    None
}

/// `(voluntary context switches, sandboxed)` from one read of the status file.
#[cfg(target_os = "linux")]
pub(crate) fn read_status(pid: u32) -> (Option<u64>, Option<bool>) {
    match std::fs::read_to_string(format!("/proc/{pid}/status")) {
        Ok(text) => (parse_voluntary_switches(&text), parse_sandboxed(&text)),
        Err(_) => (None, None),
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn read_status(_pid: u32) -> (Option<u64>, Option<bool>) {
    (None, None)
}

#[cfg(target_os = "linux")]
pub(crate) fn read_kind(pid: u32) -> Option<&'static str> {
    use std::io::Read as _;
    let mut header = [0u8; 20];
    std::fs::File::open(format!("/proc/{pid}/exe"))
        .ok()?
        .read_exact(&mut header)
        .ok()?;
    elf_kind(&header)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn read_kind(_pid: u32) -> Option<&'static str> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROLLUP: &str = "55d0a0a00000-7ffd6b3a1000 ---p 00000000 00:00 0   [rollup]\n\
Rss:               12000 kB\n\
Pss:                8000 kB\n\
Pss_Dirty:          3000 kB\n\
Shared_Clean:       3000 kB\n\
Shared_Dirty:        500 kB\n\
Private_Clean:      1000 kB\n\
Private_Dirty:      6000 kB\n\
Swap:                400 kB\n\
SwapPss:             300 kB\n";

    #[test]
    fn smaps_rollup_splits_private_shared_and_swapped_memory() {
        let rollup = parse_smaps_rollup(ROLLUP).unwrap();
        assert_eq!(rollup.pss, 8000 * 1024);
        assert_eq!(rollup.private, 7000 * 1024);
        assert_eq!(rollup.shared, 3500 * 1024);
        assert_eq!(rollup.swapped, 300 * 1024);
    }

    #[test]
    fn an_empty_rollup_is_not_a_zero_reading() {
        assert_eq!(parse_smaps_rollup(""), None);
        assert_eq!(parse_smaps_rollup("Rss: 4 kB\n"), None);
    }

    #[test]
    fn status_gives_wakeups_and_sandbox_state() {
        let status = "Name:\tfoo\nSeccomp:\t2\nSeccomp_filters:\t1\n\
voluntary_ctxt_switches:\t1234\nnonvoluntary_ctxt_switches:\t9\n";
        assert_eq!(parse_voluntary_switches(status), Some(1234));
        assert_eq!(parse_sandboxed(status), Some(true));
        assert_eq!(parse_sandboxed("Seccomp:\t0\n"), Some(false));
        assert_eq!(parse_sandboxed("Name:\tfoo\n"), None);
    }

    const I915_FDINFO: &str = "pos:\t0\nflags:\t02100002\ndrm-driver:\ti915\n\
drm-client-id:\t7\ndrm-engine-render:\t5000000000 ns\ndrm-engine-capacity-render:\t1\n\
drm-engine-copy:\t100 ns\n";

    #[test]
    fn drm_fdinfo_gives_busy_time_per_engine() {
        let (client, usage) = parse_drm_fdinfo(I915_FDINFO).unwrap();
        assert_eq!(client.as_deref(), Some("7"));
        assert_eq!(
            usage.engines,
            [
                ("render".to_owned(), 5_000_000_000),
                ("copy".to_owned(), 100)
            ]
        );
        assert_eq!(usage.total_nanoseconds(), Some(5_000_000_100));
        // Not a DRM descriptor, or one with no engine usage.
        assert!(parse_drm_fdinfo("pos:\t0\nflags:\t0\n").is_none());
        assert!(parse_drm_fdinfo("drm-driver:\ti915\n").is_none());
    }

    #[test]
    fn xe_cycles_pair_busy_with_total() {
        let text = "drm-driver:\txe\ndrm-client-id:\t3\ndrm-cycles-rcs:\t40\n\
drm-total-cycles-rcs:\t100\ndrm-cycles-bcs:\t5\n";
        let (_, usage) = parse_drm_fdinfo(text).unwrap();
        // `bcs` has no total, so it cannot be compared and is left out.
        assert_eq!(usage.cycles, [("rcs".to_owned(), 40, 100)]);
        assert_eq!(usage.total_nanoseconds(), None);
    }

    #[test]
    fn a_client_shared_by_several_descriptors_counts_once() {
        let (id, usage) = parse_drm_fdinfo(I915_FDINFO).unwrap();
        let merged = merge_clients(vec![(id.clone(), usage.clone()), (id, usage.clone())]);
        assert_eq!(merged, usage);
        // Two different clients add up.
        let (other, other_usage) =
            parse_drm_fdinfo(&I915_FDINFO.replace("\t7\n", "\t8\n")).unwrap();
        let both = merge_clients(vec![(Some("7".to_owned()), usage), (other, other_usage)]);
        assert_eq!(both.total_nanoseconds(), Some(10_000_000_200));
    }

    #[test]
    fn gpu_percent_is_the_busiest_engine_over_the_interval() {
        let before = GpuUsage {
            engines: vec![("render".into(), 1_000_000_000), ("copy".into(), 0)],
            cycles: vec![("rcs".into(), 10, 100)],
        };
        let after = GpuUsage {
            engines: vec![
                ("render".into(), 1_500_000_000),
                ("copy".into(), 100_000_000),
            ],
            cycles: vec![("rcs".into(), 30, 200)],
        };
        // render: 0.5 s busy in 1 s = 50 %; rcs: 20 of 100 cycles = 20 %.
        let percent = gpu_percent(&before, &after, 1_000_000_000).unwrap();
        assert!((percent - 50.0).abs() < 0.01, "{percent}");
        // Busier than the interval is capped, and no common engine is None.
        assert_eq!(gpu_percent(&before, &after, 100_000_000), Some(100.0));
        assert_eq!(
            gpu_percent(&GpuUsage::default(), &after, 1_000_000_000),
            None
        );
        assert_eq!(gpu_percent(&before, &after, 0), Some(20.0));
    }

    #[test]
    fn wakeups_are_a_delta_and_never_wrap() {
        assert_eq!(wakeup_delta(Some(10), 25), Some(15));
        assert_eq!(wakeup_delta(None, 25), None);
        assert_eq!(wakeup_delta(Some(30), 25), None);
    }

    #[test]
    fn elf_header_names_the_architecture() {
        let mut header = [0u8; 20];
        header[..4].copy_from_slice(b"\x7fELF");
        header[4] = 2;
        header[5] = 1;
        header[18] = 0x3E;
        assert_eq!(elf_kind(&header), Some("Intel 64-bit"));
        header[18] = 0xB7;
        assert_eq!(elf_kind(&header), Some("ARM 64-bit"));
        header[5] = 2;
        header[18] = 0;
        header[19] = 0x3E;
        assert_eq!(elf_kind(&header), Some("Intel 64-bit"));
        assert_eq!(elf_kind(b"#!/bin/sh\n"), None);
    }
}
