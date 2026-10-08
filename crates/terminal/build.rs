fn main() {
    #[cfg(windows)]
    rmac_windows_resource_build::embed("rmac-terminal", "Terminal", "terminal");
}
