fn main() {
    // Shared by both of this crate's exes (lulo-session, lulo-shell): a
    // build.rs's embedded resources apply crate-wide, not per [[bin]].
    #[cfg(windows)]
    rmac_windows_resource_build::embed("Lulo", "lulo-session");
}
