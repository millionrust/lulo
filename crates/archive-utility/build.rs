fn main() {
    #[cfg(windows)]
    rmac_windows_resource_build::embed("rmac-archive-utility", "Archive Utility", "archive-utility");
}
