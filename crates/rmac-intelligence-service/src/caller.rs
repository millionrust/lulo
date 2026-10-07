//! Who may call the service (ADR 0024 §4 "Caller checks"): the same user,
//! from a Lulo program. The program check reads the caller's executable
//! path; it is defence in depth, not a security boundary (a same-user
//! process could always run a Lulo program itself).

use std::path::{Path, PathBuf};

/// Directories Lulo programs are installed in: the package's, and the
/// directory this service itself runs from (development installs under
/// `~/.local/libexec/rmac`, and the private test sessions, keep every
/// program side by side).
pub fn trusted_directories() -> Vec<PathBuf> {
    let mut directories = vec![
        PathBuf::from("/usr/libexec/rmac"),
        PathBuf::from("/usr/bin"),
    ];
    if let Some(own) = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
    {
        directories.push(own);
    }
    directories
}

/// Whether `executable` is a program in one of `trusted`.
pub fn executable_allowed(executable: &Path, trusted: &[PathBuf]) -> bool {
    let name_ok = executable
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("rmac-"));
    name_ok
        && executable
            .parent()
            .is_some_and(|parent| trusted.iter().any(|directory| directory == parent))
}

/// This process's user id: the owner of `/proc/self`.
#[cfg(target_os = "linux")]
pub fn own_uid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt as _;
    std::fs::metadata("/proc/self")
        .ok()
        .map(|metadata| metadata.uid())
}

/// The executable of a process, by pid.
#[cfg(target_os = "linux")]
pub fn executable_of(pid: u32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/exe")).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_lulo_programs_in_lulo_directories_are_trusted() {
        let trusted = vec![
            PathBuf::from("/usr/libexec/rmac"),
            PathBuf::from("/opt/lulo/libexec/rmac"),
        ];
        assert!(executable_allowed(
            Path::new("/usr/libexec/rmac/rmac-launcher"),
            &trusted
        ));
        assert!(executable_allowed(
            Path::new("/opt/lulo/libexec/rmac/rmac-system-settings"),
            &trusted
        ));
        assert!(!executable_allowed(Path::new("/usr/bin/python3"), &trusted));
        assert!(!executable_allowed(
            Path::new("/tmp/rmac-launcher"),
            &trusted
        ));
        assert!(!executable_allowed(
            Path::new("/usr/libexec/rmac/evil"),
            &trusted
        ));
        assert!(!executable_allowed(
            Path::new("/usr/libexec/rmac/sub/rmac-launcher"),
            &trusted
        ));
    }
}
