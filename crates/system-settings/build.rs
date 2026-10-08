fn main() {
    #[cfg(windows)]
    rmac_windows_resource_build::embed("rmac-system-settings", "System Settings", "system-settings");
}
