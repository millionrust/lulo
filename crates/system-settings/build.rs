fn main() {
    #[cfg(windows)]
    rmac_windows_resource_build::embed("System Settings", "system-settings");
}
