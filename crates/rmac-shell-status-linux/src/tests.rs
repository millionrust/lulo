use super::*;

#[test]
fn source_sets_merge_without_losing_independent_services() {
    let mut sources = Sources::audio();
    sources.merge(Sources::system_bus());
    assert_eq!(sources, Sources::all());
    assert!(!sources.is_empty());
    assert!(Sources::empty().is_empty());
}

#[test]
fn bitrate_and_poll_time_changes_do_not_refresh() {
    use crate::model::{property_change, PropertyChange};

    let wireless = "org.freedesktop.NetworkManager.Device.Wireless";
    let battery = "org.freedesktop.UPower.Device";
    let access_point = "org.freedesktop.NetworkManager.AccessPoint";
    assert_eq!(
        property_change(wireless, &["Bitrate"], &[]),
        PropertyChange::Unshown
    );
    assert_eq!(
        property_change(battery, &["UpdateTime"], &[]),
        PropertyChange::Unshown
    );
    assert_eq!(
        property_change(access_point, &["Strength"], &[]),
        PropertyChange::SignalStrength
    );

    // Anything the bar shows, or anything unknown, still refreshes.
    for (interface, changed, invalidated) in [
        (battery, &["UpdateTime", "Percentage"][..], &[][..]),
        (wireless, &["ActiveAccessPoint"][..], &[][..]),
        (wireless, &["Bitrate"][..], &["Bitrate"][..]),
        (wireless, &[][..], &[][..]),
        (access_point, &["Strength", "Ssid"][..], &[][..]),
    ] {
        assert_eq!(
            property_change(interface, changed, invalidated),
            PropertyChange::Shown,
            "{interface} {changed:?} {invalidated:?}"
        );
    }
}

#[test]
fn signal_strength_rereads_the_network_at_most_every_30_seconds() {
    use crate::model::signal_strength_refresh_due;
    use std::time::Duration;

    assert!(signal_strength_refresh_due(None));
    assert!(!signal_strength_refresh_due(Some(Duration::from_secs(6))));
    assert!(signal_strength_refresh_due(Some(Duration::from_secs(30))));
}

#[test]
fn source_sets_scope_transport_failures() {
    assert!(!Sources::system_bus().audio);
    assert_eq!(
        Sources::audio(),
        Sources {
            audio: true,
            ..Sources::empty()
        }
    );
}

#[cfg(target_os = "linux")]
mod pipewire_monitor_command {
    use crate::watch::build_monitor_command;
    use futures_lite::io::AsyncReadExt as _;

    /// Regression test for the bug fixed alongside this test (see
    /// `build_monitor_command`'s doc comment, and `rmac-audio`'s identical
    /// fix/test): wrapping an already-configured `std::process::Command` in
    /// `async_process::Command::from(...)` used to silently drop the piped
    /// stdout in favour of `Stdio::inherit()`, so `child.stdout` was always
    /// `None` and every spawn of the real `pw-dump --monitor` watcher was
    /// killed a moment later by `kill_on_drop`. This spawns a real, trivial
    /// child (`sh -c "echo ..."`, not `pw-dump`) through the exact same
    /// builder and asserts its stdout is actually captured and readable.
    #[test]
    fn command_pipes_stdout_through_async_process() {
        let mut command = build_monitor_command("sh", &["-c", "echo lulo-status-watch-test"]);
        let mut child = command.spawn().expect("spawn the test child");
        let mut stdout = child
            .stdout
            .take()
            .expect("stdout must be piped, not inherited");
        let mut collected = Vec::new();
        futures_lite::future::block_on(async {
            stdout
                .read_to_end(&mut collected)
                .await
                .expect("read the child's stdout");
        });
        assert_eq!(
            String::from_utf8_lossy(&collected).trim(),
            "lulo-status-watch-test",
        );
    }

    /// The same regression, checked the other direction: before the fix,
    /// `async_process::Command`'s own `stdin`/`stdout`/`stderr` tracking
    /// booleans were left `false` by `Command::from(std::process::Command)`.
    /// `Command`'s alternate (`{:#?}`) `Debug` impl prints those exact
    /// fields (its plain `{:?}` delegates to the wrapped
    /// `std::process::Command` instead and would not show this).
    #[test]
    fn command_reports_stdio_as_explicitly_configured() {
        let command = build_monitor_command("true", &[]);
        let debug = format!("{command:#?}");
        assert!(
            debug.contains("stdin: true")
                && debug.contains("stdout: true")
                && debug.contains("stderr: true"),
            "async_process::Command did not track its stdio as explicitly set: {debug}",
        );
    }
}
