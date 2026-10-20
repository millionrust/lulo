//! General ▸ Startup Disk on Lulo OS: the firmware's UEFI boot entries.
//!
//! The entries are read straight from efivarfs (`BootOrder`, `BootCurrent`
//! and each `Boot####`), which any user may read. Choosing one sets
//! `BootNext` through `pkexec efibootmgr`, so only the next start uses it,
//! and then the menu bar restarts the way Restart in the Lulo menu does.

use std::path::{Path, PathBuf};

const GLOBAL_GUID: &str = "8be4df61-93ca-11d2-aa0d-00e098032b8c";
const MAX_VARIABLE_BYTES: usize = 64 * 1024;
/// The load option's "active" attribute (`LOAD_OPTION_ACTIVE`).
const LOAD_OPTION_ACTIVE: u32 = 0x1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct BootEntry {
    pub(super) number: u16,
    pub(super) description: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct StartupDisks {
    pub(super) current: Option<u16>,
    /// Active entries in the firmware's boot order.
    pub(super) entries: Vec<BootEntry>,
}

fn efivars_root() -> PathBuf {
    std::env::var_os("LULO_FAKE_SYS_ROOT")
        .map(|root| Path::new(&root).join("firmware/efi/efivars"))
        .unwrap_or_else(|| PathBuf::from("/sys/firmware/efi/efivars"))
}

/// Whether this computer started through UEFI (so it has boot entries).
pub(super) fn available() -> bool {
    efivars_root().is_dir()
}

fn variable(root: &Path, name: &str) -> Option<Vec<u8>> {
    let bytes = rmac_storage::read_bounded_no_follow(
        &root.join(format!("{name}-{GLOBAL_GUID}")),
        MAX_VARIABLE_BYTES,
    )
    .ok()?;
    // efivarfs prefixes each value with its 4-byte attributes.
    (bytes.len() >= 4).then(|| bytes[4..].to_vec())
}

fn u16s(bytes: &[u8]) -> Vec<u16> {
    bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect()
}

/// An `EFI_LOAD_OPTION`: attributes, the device-path length, then the
/// NUL-terminated UTF-16 description.
pub(super) fn parse_load_option(bytes: &[u8]) -> Option<(bool, String)> {
    let attributes = u32::from_le_bytes(bytes.get(0..4)?.try_into().ok()?);
    let description = u16s(bytes.get(6..)?)
        .into_iter()
        .take_while(|unit| *unit != 0)
        .collect::<Vec<_>>();
    let description = String::from_utf16(&description).ok()?;
    Some((
        attributes & LOAD_OPTION_ACTIVE != 0,
        description.trim().to_owned(),
    ))
}

pub(super) fn read_from(root: &Path) -> StartupDisks {
    let current = variable(root, "BootCurrent").and_then(|bytes| u16s(&bytes).first().copied());
    let order = variable(root, "BootOrder")
        .map(|bytes| u16s(&bytes))
        .unwrap_or_default();
    let entries = order
        .into_iter()
        .filter_map(|number| {
            let bytes = variable(root, &format!("Boot{number:04X}"))?;
            let (active, description) = parse_load_option(&bytes)?;
            (active && !description.is_empty()).then_some(BootEntry {
                number,
                description,
            })
        })
        .collect();
    StartupDisks { current, entries }
}

/// The entries, read off the UI thread by the caller.
pub(super) fn read() -> StartupDisks {
    read_from(&efivars_root())
}

/// Make `number` the entry the next start uses. Asks for an administrator
/// password through polkit, as the Mac asks before changing the startup
/// disk.
#[cfg(target_os = "linux")]
pub(super) fn set_next(number: u16) -> Result<(), String> {
    let status = std::process::Command::new("/usr/bin/pkexec")
        .args(["/usr/bin/efibootmgr", "--quiet", "--bootnext"])
        .arg(format!("{number:04X}"))
        .stdin(std::process::Stdio::null())
        .status()
        .map_err(|error| format!("could not ask for permission: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("the startup disk was not changed".to_owned())
    }
}

#[cfg(not(target_os = "linux"))]
pub(super) fn set_next(_number: u16) -> Result<(), String> {
    Err("Startup Disk is a Lulo OS feature".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_option(active: bool, name: &str) -> Vec<u8> {
        let mut bytes = (if active { 1u32 } else { 0 }).to_le_bytes().to_vec();
        bytes.extend_from_slice(&4u16.to_le_bytes());
        for unit in name.encode_utf16().chain([0]) {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes.extend_from_slice(&[0x7f, 0xff, 0x04, 0x00]);
        bytes
    }

    fn write(root: &Path, name: &str, value: &[u8]) {
        let mut bytes = 7u32.to_le_bytes().to_vec();
        bytes.extend_from_slice(value);
        std::fs::write(root.join(format!("{name}-{GLOBAL_GUID}")), bytes).unwrap();
    }

    #[test]
    fn reads_active_entries_in_boot_order() {
        let root = std::env::temp_dir().join(format!("rmac-startup-disk-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        write(&root, "BootOrder", &[2, 0, 0, 0, 1, 0]);
        write(&root, "BootCurrent", &[0, 0]);
        write(&root, "Boot0000", &load_option(true, "ubuntu"));
        write(&root, "Boot0001", &load_option(false, "Old"));
        write(
            &root,
            "Boot0002",
            &load_option(true, "Windows Boot Manager"),
        );
        let disks = read_from(&root);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(disks.current, Some(0));
        assert_eq!(
            disks
                .entries
                .iter()
                .map(|entry| entry.description.as_str())
                .collect::<Vec<_>>(),
            ["Windows Boot Manager", "ubuntu"]
        );
    }
}
