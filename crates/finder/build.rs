fn main() {
    // Files' own version info and icon, linked into `rmac-files` only. The
    // CVTRES "duplicate resource. type:VERSION" link failure this once hit
    // came from Preview's resources, which the old `winres` linked as a
    // native library that Cargo passed on to Files through Quick Look;
    // `rmac-windows-resource-build` now links each binary's own script
    // into that binary alone.
    #[cfg(windows)]
    rmac_windows_resource_build::embed("rmac-files", "Files", "files");
}
