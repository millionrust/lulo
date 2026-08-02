use super::*;
use crate::inventory::{
    decode_mount_field, parse_mountinfo, parse_mountinfo_line, revalidated_mount,
    sanitize_display_name, volume_usage,
};
use crate::model::{MAX_DISPLAY_NAME_BYTES, MAX_MOUNTS};
use std::path::{Path, PathBuf};

#[test]
fn mountinfo_decodes_and_filters_user_visible_mounts() {
    let contents = "36 25 8:1 / /media/alice/My\\040Drive rw,nosuid - vfat /dev/sdb1 rw\n\
                    37 25 0:42 / /run/user/1000/gvfs/smb-share:server=nas,share=docs rw - fuse.gvfsd-fuse gvfsd-fuse rw\n\
                    38 25 0:5 / /proc rw - proc proc rw\n\
                    39 25 8:2 / /mnt/Backup rw shared:7 - ext4 /dev/sdc1 rw\n";

    let (mounts, truncated) = parse_mountinfo(contents);

    assert!(!truncated);
    assert_eq!(mounts.len(), 3);
    assert!(mounts
        .iter()
        .any(|mount| mount.path == Path::new("/media/alice/My Drive")));
    assert!(mounts.iter().any(|mount| mount.name == "docs"));
    assert!(mounts
        .iter()
        .all(|mount| mount.identity.starts_with("linux:")));
    assert!(!mounts.iter().any(|mount| mount.path == Path::new("/proc")));
}

#[test]
fn malformed_and_unknown_mount_escapes_are_rejected() {
    assert_eq!(decode_mount_field("My\\040Drive"), Some("My Drive".into()));
    assert_eq!(decode_mount_field("bad\\999escape"), None);
    assert!(parse_mountinfo_line("not mountinfo").is_none());
    assert!(
        parse_mountinfo_line("bad-id 25 8:1 / /media/alice/Drive rw - vfat /dev/sdb1 rw").is_none()
    );
}

#[test]
fn duplicate_mount_points_are_removed() {
    let line = "36 25 8:1 / /media/alice/Drive rw - vfat /dev/sdb1 rw\n";
    let (mounts, truncated) = parse_mountinfo(&format!("{line}{line}"));
    assert_eq!(mounts.len(), 1);
    assert!(!truncated);
}

#[test]
fn usage_math_is_saturating_and_flags_low_space() {
    let healthy = Usage::from_blocks(4096, 10_000_000, 5_000_000);
    assert_eq!(healthy.total, 40_960_000_000);
    assert_eq!(healthy.used, 20_480_000_000);
    assert!(!healthy.is_low_space());

    let low = Usage::from_blocks(4096, 10_000_000, 100_000);
    assert!(low.is_low_space());
    assert!(low.used_fraction() > 0.9);

    let inconsistent = Usage::from_blocks(u64::MAX, u64::MAX, u64::MAX);
    assert_eq!(inconsistent.available, inconsistent.total);
    assert_eq!(inconsistent.used, 0);
}

#[test]
fn live_root_volume_has_bounded_capacity_relationships() {
    let usage = volume_usage(Path::new("/")).unwrap();
    assert!(usage.total > 0);
    assert!(usage.available <= usage.total);
    assert_eq!(usage.used, usage.total - usage.available);
}

#[test]
fn display_names_are_bounded_and_cannot_inject_controls_or_remote_identity() {
    let control =
        parse_mountinfo_line("36 25 8:1 / /media/alice/Evil\\012Name rw - vfat /dev/sdb1 rw")
            .unwrap();
    assert_eq!(control.name, "Evil Name");
    assert!(!control.name.chars().any(char::is_control));

    let remote = parse_mountinfo_line(
        "37 25 0:42 / /run/user/1000/gvfs/google-drive:host=example,user=private rw - fuse.gvfsd-fuse gvfsd-fuse rw",
    )
    .unwrap();
    assert_eq!(remote.name, "Remote Volume");
    assert!(!remote.name.contains("private"));

    let long = sanitize_display_name(&"x".repeat(MAX_DISPLAY_NAME_BYTES + 50));
    assert_eq!(long.len(), MAX_DISPLAY_NAME_BYTES);
}

#[test]
fn discovery_is_bounded_to_the_supported_visible_volume_count() {
    let contents = (0..MAX_MOUNTS + 20)
        .map(|index| {
            format!(
                "{} 25 8:1 / /media/alice/Drive-{index} rw - vfat /dev/sdb1 rw",
                index + 1
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let (mounts, truncated) = parse_mountinfo(&contents);
    assert_eq!(mounts.len(), MAX_MOUNTS);
    assert!(truncated);
}

#[test]
fn revalidation_requires_identity_path_and_mount_class() {
    let expected = Mount {
        identity: "linux:42".into(),
        name: "Backup".into(),
        path: PathBuf::from("/media/alice/Backup"),
        ejectable: true,
    };
    let replacement = Mount {
        identity: "linux:43".into(),
        name: "Backup".into(),
        path: expected.path.clone(),
        ejectable: true,
    };
    assert!(revalidated_mount(&expected, vec![replacement]).is_none());

    let exact = expected.clone();
    assert_eq!(
        revalidated_mount(&expected, vec![exact.clone()]),
        Some(exact)
    );
}
