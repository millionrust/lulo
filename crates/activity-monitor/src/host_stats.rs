//! Host-wide figures Activity Monitor's bottom panel shows that `sysinfo`
//! does not expose: the thread count (CPU tab) and the page cache (Memory
//! tab's "Cached Files"). Both come from `/proc` on Linux; elsewhere they are
//! unavailable and the panel omits the row rather than inventing a value.

/// Every kernel task (thread) on the host, from `/proc/loadavg`'s
/// `running/total` field.
#[cfg(target_os = "linux")]
pub(crate) fn thread_count() -> Option<u64> {
    let contents = read_bounded("/proc/loadavg")?;
    parse_loadavg_threads(&contents)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn thread_count() -> Option<u64> {
    None
}

/// Bytes of file-backed page cache, from `/proc/meminfo`'s `Cached:` line.
#[cfg(target_os = "linux")]
pub(crate) fn cached_bytes() -> Option<u64> {
    let contents = read_bounded("/proc/meminfo")?;
    parse_meminfo_cached(&contents)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn cached_bytes() -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
fn read_bounded(path: &str) -> Option<String> {
    use std::io::Read as _;

    let mut source = std::fs::File::open(path).ok()?.take(16 * 1024);
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
    fn meminfo_cached_is_read_in_bytes() {
        let meminfo = "MemTotal: 8000 kB\nBuffers: 10 kB\nCached: 1310 kB\nSwapCached: 5 kB\n";
        assert_eq!(parse_meminfo_cached(meminfo), Some(1310 * 1024));
        assert_eq!(parse_meminfo_cached("MemTotal: 8000 kB\n"), None);
    }
}
