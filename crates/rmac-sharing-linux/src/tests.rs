use rmac_sharing::FirewallState;

use crate::system::{
    parse_samba_shares, parse_ufw_status, requested_state_reached, FirewallService,
};
use crate::watch::{firewall_path_relevant, owner_change_reappeared, samba_path_relevant};

#[test]
fn ufw_parser_requires_an_explicit_allow_rule() {
    assert_eq!(
        parse_ufw_status("Status: inactive\n", FirewallService::Ssh),
        FirewallState::Inactive
    );
    assert_eq!(
        parse_ufw_status(
            "Status: active\n22/tcp ALLOW Anywhere\n",
            FirewallService::Ssh
        ),
        FirewallState::Allows
    );
    assert_eq!(
        parse_ufw_status(
            "Status: active\n22/tcp DENY Anywhere\n",
            FirewallService::Ssh
        ),
        FirewallState::ActiveUnverified
    );
    assert_eq!(
        parse_ufw_status(
            "Status: active\nSamba ALLOW Anywhere\n",
            FirewallService::Samba
        ),
        FirewallState::Allows
    );
    assert_eq!(
        parse_ufw_status(
            "Status: active\n445/tcp ALLOW Anywhere\n",
            FirewallService::Samba
        ),
        FirewallState::ActiveUnverified
    );
}

#[test]
fn samba_parser_exposes_only_bounded_file_share_names() {
    let (shares, truncated) = parse_samba_shares(
        "[global]\n[homes]\n[printers]\n[print$]\n[Team Files]\n[team files]\n\t[option value]\n",
    );
    assert!(!truncated);
    assert_eq!(
        shares
            .iter()
            .map(|share| share.name.as_str())
            .collect::<Vec<_>>(),
        ["homes", "Team Files"]
    );

    let input = (0..130)
        .map(|index| format!("[share-{index}]"))
        .collect::<Vec<_>>()
        .join("\n");
    let (shares, truncated) = parse_samba_shares(&input);
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
