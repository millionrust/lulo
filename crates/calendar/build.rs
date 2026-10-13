fn main() {
    #[cfg(windows)]
    rmac_windows_resource_build::embed("rmac-calendar", "Calendar", "calendar");
}
