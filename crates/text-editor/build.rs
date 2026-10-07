fn main() {
    #[cfg(windows)]
    rmac_windows_resource_build::embed("Text Editor", "text-editor");
}
