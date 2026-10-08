fn main() {
    // The Lulo layer's version info and icon (ADR 0023 "Installer"), one
    // script per binary: `lulo-session` is what the Start menu's Lulo
    // shortcut runs, and `lulo-shell` is the process the Dock, Task
    // Manager and the taskbar show while Lulo runs.
    #[cfg(windows)]
    {
        rmac_windows_resource_build::embed("lulo-session", "Lulo", "lulo-session");
        rmac_windows_resource_build::embed("lulo-shell", "Lulo", "lulo-shell");
    }
}
