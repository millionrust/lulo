fn main() {
    #[cfg(windows)]
    rmac_windows_resource_build::embed("rmac-system-monitor", "System Monitor", "system-monitor");
}
