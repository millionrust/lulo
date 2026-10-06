//! Window ▸ GPU History (MON-10/MON-14, MON-MENU-035): real GPU
//! utilisation, where the kernel actually exposes one.
//!
//! `amdgpu` publishes a ready-made percentage at
//! `/sys/class/drm/card*/device/gpu_busy_percent` — no parsing, no
//! per-process aggregation, just a real kernel-reported number. `i915`
//! (and newer Intel `xe`) has no equivalent single file; reading its
//! per-process `drm-engine-*` busy-time fields out of every process's
//! `/proc/<pid>/fdinfo` and turning that into a rate is real but
//! substantially more code, so it is left out rather than shipped
//! half-right — this honestly reports "unavailable" on Intel instead of
//! guessing.

use std::fs;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GpuReading {
    /// A real `gpu_busy_percent` reading, 0..=100.
    Percent(u8),
    /// No supported GPU sysfs source was found on this system.
    Unavailable,
}

/// Every `/sys/class/drm/card*/device/gpu_busy_percent` file that exists,
/// in a stable order (`card0` before `card1`, …).
fn candidate_paths(drm_root: &str) -> Vec<std::path::PathBuf> {
    let Ok(entries) = fs::read_dir(drm_root) else {
        return Vec::new();
    };
    let mut cards: Vec<_> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("card") && !name.contains('-'))
        })
        .collect();
    cards.sort();
    cards
        .into_iter()
        .map(|card| card.join("device/gpu_busy_percent"))
        .collect()
}

/// Parse `gpu_busy_percent`'s contents (a bare integer, newline-terminated)
/// into a clamped 0..=100 reading.
fn parse_busy_percent(contents: &str) -> Option<u8> {
    let value: u32 = contents.trim().parse().ok()?;
    Some(value.min(100) as u8)
}

/// Read every candidate card's `gpu_busy_percent` and average them — most
/// systems have exactly one discrete/integrated GPU, so this is almost
/// always just that one real reading.
pub(crate) fn read() -> GpuReading {
    read_from("/sys/class/drm")
}

fn read_from(drm_root: &str) -> GpuReading {
    let readings: Vec<u8> = candidate_paths(drm_root)
        .into_iter()
        .filter_map(|path| fs::read_to_string(path).ok())
        .filter_map(|contents| parse_busy_percent(&contents))
        .collect();
    if readings.is_empty() {
        return GpuReading::Unavailable;
    }
    let total: u32 = readings.iter().map(|&value| value as u32).sum();
    GpuReading::Percent((total / readings.len() as u32) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parses_a_bare_percentage() {
        assert_eq!(parse_busy_percent("37\n"), Some(37));
        assert_eq!(parse_busy_percent("0"), Some(0));
    }

    #[test]
    fn clamps_an_out_of_range_reading_rather_than_overflowing() {
        assert_eq!(parse_busy_percent("255"), Some(100));
    }

    #[test]
    fn rejects_unparsable_contents() {
        assert_eq!(parse_busy_percent("not a number"), None);
        assert_eq!(parse_busy_percent(""), None);
    }

    #[test]
    fn no_drm_root_is_honestly_unavailable_not_zero() {
        assert_eq!(
            read_from("/nonexistent-drm-root-for-tests"),
            GpuReading::Unavailable
        );
    }

    #[test]
    fn averages_every_real_card_reading() {
        let dir = std::env::temp_dir().join(format!(
            "rmac-gpu-stats-test-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = fs::remove_dir_all(&dir);
        for (card, value) in [("card0", "20"), ("card1", "60"), ("card1-eDP", "99")] {
            let device = dir.join(card).join("device");
            fs::create_dir_all(&device).unwrap();
            fs::write(device.join("gpu_busy_percent"), value).unwrap();
        }
        // A non-card entry (e.g. "version") must never be treated as a card.
        fs::write(dir.join("version"), "1").unwrap();
        let reading = read_from(dir.to_str().unwrap());
        // card0=20, card1=60; "card1-eDP" is a connector, not a card, and is
        // excluded by the no-hyphen filter.
        assert_eq!(reading, GpuReading::Percent(40));
        fs::remove_dir_all(&dir).unwrap();
    }
}
