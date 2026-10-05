use crate::system::{
    bounded_shares, combined_remote_state, job_succeeded, parse_smb_conf_shares,
    parse_ufw_conf_enabled, remote_login_units, requested_state_reached,
};

#[test]
fn a_listening_ssh_socket_means_remote_login_is_on() {
    // Ubuntu socket-activates sshd: ssh.service idles while ssh.socket holds
    // port 22 open, which must never read as Remote Login off.
    assert_eq!(combined_remote_state("inactive", Some("active")), "active");
    assert!(!requested_state_reached(
        Some(&combined_remote_state("inactive", Some("active"))),
        false,
        false
    ));
    assert_eq!(
        combined_remote_state("inactive", Some("inactive")),
        "inactive"
    );
    assert_eq!(combined_remote_state("active", None), "active");
    assert_eq!(combined_remote_state("failed", None), "failed");
}
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

#[test]
fn remote_login_off_combines_the_socket_and_service_in_one_unit_file_call() {
    // SHARE-SLOW: turning Remote Login off used to stop/disable
    // `ssh.socket` through its own full round trip, then stop/disable
    // `ssh.service` through a second one, each with its own Reload. The
    // socket must come first so it cannot restart the service once that
    // is stopped, but both now move through a single call.
    assert_eq!(
        remote_login_units("ssh.service", false, true),
        vec!["ssh.socket", "ssh.service"]
    );
    // No socket unit on this system: nothing to add.
    assert_eq!(
        remote_login_units("ssh.service", false, false),
        vec!["ssh.service"]
    );
    // Turning it on never re-enables socket activation, socket present or
    // not.
    assert_eq!(
        remote_login_units("ssh.service", true, true),
        vec!["ssh.service"]
    );
    assert_eq!(
        remote_login_units("ssh.service", true, false),
        vec!["ssh.service"]
    );
}

#[test]
fn only_a_done_job_counts_as_success() {
    // A `StartUnit`/`StopUnit` job that systemd reports as anything other
    // than "done" (cancelled, timed out, failed, a missed dependency, or
    // skipped) must not be read as the unit having reached that state.
    assert!(job_succeeded("done"));
    assert!(!job_succeeded("canceled"));
    assert!(!job_succeeded("timeout"));
    assert!(!job_succeeded("failed"));
    assert!(!job_succeeded("dependency"));
    assert!(!job_succeeded("skipped"));
}

#[test]
fn a_remote_login_toggle_reloads_systemd_at_most_once() {
    // SHARE-SLOW: one toggle used to run three `EnableUnitFiles`/
    // `DisableUnitFiles` + `Reload` round trips (socket, service, and a
    // failed-readback retry). `system_set_service` now has exactly one
    // `Reload` call in its enable branch and one in its disable branch, so
    // a single invocation can never reload the daemon more than once, and
    // it no longer sleeps waiting for systemd to catch up.
    let source = include_str!("system.rs");
    let reload_calls = source.matches("\"Reload\"").count();
    assert_eq!(
        reload_calls, 2,
        "expected exactly one Reload call site per system_set_service branch"
    );
    assert!(
        !source.contains("thread::sleep"),
        "readback must wait on systemd's JobRemoved signal, not poll with a sleep"
    );
    assert!(
        source.contains("JobRemoved"),
        "StartUnit/StopUnit completion must be confirmed via the JobRemoved signal"
    );
}

#[test]
fn every_systemd_mutation_may_ask_polkit_interactively() {
    // F-1: without ALLOW_INTERACTIVE_AUTHORIZATION, systemd refuses Sharing
    // toggles under the default auth_admin_keep policy instead of prompting.
    // Plain `.call(` is left only for read-only queries.
    let source = include_str!("system.rs");
    let mut plain_calls = 0;
    for (index, _) in source.match_indices(".call") {
        let rest = &source[index + ".call".len()..];
        if rest.starts_with("_with_flags") {
            continue;
        }
        plain_calls += 1;
        let method = rest
            .split('"')
            .nth(1)
            .expect("a D-Bus call names its method");
        assert!(
            ["ListUnitFiles", "LoadUnit"].contains(&method),
            "{method} is called without interactive authorization"
        );
    }
    assert!(plain_calls > 0);
}
