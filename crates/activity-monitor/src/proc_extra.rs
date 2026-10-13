//! Optional per-process figures read from `/proc` (MON-MENU-006/007/008/009/
//! 011/016/017): the Mac's Memory, Real Private Memory, Real Shared Memory,
//! Compressed Memory, Kind, Sandbox and Idle Wake Ups columns.
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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProcExtra {
    pub(crate) rollup: Option<SmapsRollup>,
    pub(crate) kind: Option<&'static str>,
    pub(crate) sandboxed: Option<bool>,
    /// Voluntary context switches since the previous sample: how often the
    /// process woke from sleep, the Linux counterpart of Idle Wake Ups.
    pub(crate) wakeups: Option<u64>,
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
