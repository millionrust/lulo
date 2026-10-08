fn main() {
    #[cfg(windows)]
    rmac_windows_resource_build::embed("rmac-text-editor", "Text Editor", "text-editor");
}
