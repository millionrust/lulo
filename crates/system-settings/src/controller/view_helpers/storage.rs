//! System Settings storage capacity formatting projection.

/// Format bytes as decimal GB (matching macOS storage display).
pub(in crate::controller) fn fmt_gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
}
