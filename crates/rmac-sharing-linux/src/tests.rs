use crate::system::{
    bounded_shares, parse_smb_conf_shares, parse_ufw_conf_enabled, requested_state_reached,
};
use crate::watch::{firewall_path_relevant, owner_change_reappeared, samba_path_relevant};

#[test]
fn ufw_conf_enabled_reads_ufws_own_key_value_setting() {
    assert_eq!(
        parse_ufw_conf_enabled("# comment\n\nENABLED=no\nLOGLEVEL=low\n"),
        Some(false)
    );
    assert_eq!(parse_ufw_conf_enabled("ENABLED=yes\n"), Some(true));
    // Quoting and surrounding whitespace, as some `ufw.conf` variants use.
    assert_eq!(parse_ufw_conf_enabled("ENABLED = \"yes\"\n"), Some(true));
    assert_eq!(
        parse_ufw_conf_enabled("# ENABLED=yes\nLOGLEVEL=low\n"),
        None
    );
    assert_eq!(parse_ufw_conf_enabled("LOGLEVEL=low\n"), None);
    assert_eq!(parse_ufw_conf_enabled("ENABLED=maybe\n"), None);
}

#[test]
fn samba_parser_exposes_only_available_share_sections() {
    let names = parse_smb_conf_shares(
        "[global]\n   workgroup = WORKGROUP\n\n[homes]\n   browseable = no\n\n[printers]\n\n[print$]\n\n[Team Files]\n   path = /srv/team\n   available = no\n\n[team files]\n   path = /srv/team2\n\n; a comment\n# another comment\n[Backups]\n   path = /srv/backups\n",
    );
    let (shares, truncated) = bounded_shares(names);
    assert!(!truncated);
    let mut names: Vec<_> = shares.iter().map(|share| share.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["Backups", "homes", "team files"]);
}

#[test]
fn samba_parser_share_names_are_bounded() {
    let names: Vec<String> = (0..130).map(|index| format!("share-{index}")).collect();
    let (shares, truncated) = bounded_shares(names);
    assert!(truncated);
    assert_eq!(shares.len(), 128);
}

#[test]
fn convergence_requires_both_runtime_and_boot_authorities() {
    assert!(requested_state_reached(Some("active"), true, true));
    assert!(!requested_state_reached(Some("active"), false, true));
    assert!(!requested_state_reached(Some("activating"), true, true));
    assert!(requested_state_reached(Some("inactive"), false, false));
    assert!(requested_state_reached(Some("failed"), false, false));
    assert!(!requested_state_reached(Some("inactive"), true, false));
    assert!(!requested_state_reached(None, false, false));
}

#[test]
fn watcher_filters_firewall_files_and_systemd_idle_exit() {
    assert!(firewall_path_relevant(std::path::Path::new(
        "/etc/ufw/user.rules"
    )));
    assert!(!firewall_path_relevant(std::path::Path::new(
        "/etc/ufw/sysctl.conf"
    )));
    assert!(samba_path_relevant(std::path::Path::new(
        "/etc/samba/smb.conf"
    )));
    assert!(!samba_path_relevant(std::path::Path::new(
        "/etc/samba/private/secrets.tdb"
    )));
    assert!(!owner_change_reappeared("org.freedesktop.systemd1", ""));
    assert!(owner_change_reappeared("org.freedesktop.systemd1", ":1.42"));
}
