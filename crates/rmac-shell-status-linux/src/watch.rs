use async_channel::Sender;

use crate::{Error, Event, Sources};

/// Watch Linux system services until the receiving side closes.
///
/// Use a bounded channel with room for several events. Refresh hints are
/// coalesced at the source, but `Unavailable` must not be silently discarded.
#[cfg(target_os = "linux")]
pub async fn watch(sender: Sender<Event>) -> Result<(), Error> {
    let dbus = reconnecting_system_bus(sender.clone());
    let audio = reconnecting_audio(sender.clone());
    futures_util::try_join!(dbus, audio)?;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub async fn watch(sender: Sender<Event>) -> Result<(), Error> {
    let _ = sender
        .send(Event::Unavailable {
            sources: Sources::all(),
            detail: "shell service signals are available on Linux".into(),
        })
        .await;
    Ok(())
}

#[cfg(target_os = "linux")]
const RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

/// `reconnecting_audio`'s cap: once PipeWire has been unavailable for a
/// while, a failed `pw-dump` spawn-connect-exit cycle every second is a
/// real, avoidable cost (a private test session with no PipeWire socket at
/// all hits this constantly; a real machine would too for as long as
/// PipeWire is down after a crash or before login finishes). Back off
/// 1s, 2s, 4s... up to this ceiling instead of retrying at a fixed pace.
#[cfg(target_os = "linux")]
const AUDIO_RECONNECT_MAX_DELAY: std::time::Duration = std::time::Duration::from_secs(60);

/// An attempt that stayed up at least this long really did connect (and
/// later lost PipeWire, rather than never reaching it); reset the backoff
/// so losing a working connection still reconnects quickly.
#[cfg(target_os = "linux")]
const AUDIO_CONNECTED_RESET_THRESHOLD: std::time::Duration = std::time::Duration::from_secs(5);

#[cfg(target_os = "linux")]
// `pw-dump --monitor` prints the full PipeWire graph once, then a fresh JSON
// array of changed objects on every subsequent state change. This debounce
// waits for a burst of those changes to settle before rebuilding the
// authoritative snapshot.
const QUIET_PERIOD: std::time::Duration = std::time::Duration::from_secs(1);

#[cfg(target_os = "linux")]
async fn reconnecting_system_bus(sender: Sender<Event>) -> Result<(), Error> {
    let mut previous_error = None;
    loop {
        match watch_system_bus_once(&sender, &mut previous_error).await {
            Ok(()) => return Ok(()),
            Err(_) if sender.is_closed() => return Ok(()),
            Err(error) => {
                if let Err(report_error) =
                    report_once(&sender, Sources::system_bus(), &error, &mut previous_error).await
                {
                    return if sender.is_closed() {
                        Ok(())
                    } else {
                        Err(report_error)
                    };
                }
                async_io::Timer::after(RECONNECT_DELAY).await;
            }
        }
    }
}

#[cfg(target_os = "linux")]
async fn reconnecting_audio(sender: Sender<Event>) -> Result<(), Error> {
    let mut previous_error = None;
    let mut delay = RECONNECT_DELAY;
    loop {
        let attempt_started = std::time::Instant::now();
        match watch_audio_once(&sender, &mut previous_error).await {
            Ok(()) => return Ok(()),
            Err(_) if sender.is_closed() => return Ok(()),
            Err(error) => {
                if let Err(report_error) =
                    report_once(&sender, Sources::audio(), &error, &mut previous_error).await
                {
                    return if sender.is_closed() {
                        Ok(())
                    } else {
                        Err(report_error)
                    };
                }
                async_io::Timer::after(delay).await;
                delay = if attempt_started.elapsed() >= AUDIO_CONNECTED_RESET_THRESHOLD {
                    RECONNECT_DELAY
                } else {
                    (delay * 2).min(AUDIO_RECONNECT_MAX_DELAY)
                };
            }
        }
    }
}

#[cfg(target_os = "linux")]
async fn report_once(
    sender: &Sender<Event>,
    sources: Sources,
    error: &Error,
    previous: &mut Option<Error>,
) -> Result<(), Error> {
    if previous.as_ref() != Some(error) {
        sender
            .send(Event::Unavailable {
                sources,
                detail: error.to_string(),
            })
            .await
            .map_err(|_| Error::new("report watcher failure", "consumer closed"))?;
        *previous = Some(error.clone());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
async fn watch_system_bus_once(
    sender: &Sender<Event>,
    previous_error: &mut Option<Error>,
) -> Result<(), Error> {
    use futures_util::StreamExt as _;
    use zbus::{message::Type, MatchRule, MessageStream};

    let connection = zbus::Connection::system()
        .await
        .map_err(|error| Error::new("connect to the system bus", error.to_string()))?;
    let rule = |path: &'static str| -> Result<_, Error> {
        MatchRule::builder()
            .msg_type(Type::Signal)
            .path_namespace(path)
            .map_err(|error| Error::new("build system signal rule", error.to_string()))
            .map(|builder| builder.build())
    };
    // NetworkManager republishes a connected Wi-Fi link's AccessPoint
    // Strength and Device.Wireless Bitrate every few seconds on real
    // hardware. Neither changes what the bar shows (strength is throttled,
    // bitrate is never shown), but receiving and parsing them still costs
    // real CPU: ~20 wakes/minute, enough on its own to push top-bar's idle
    // CPU over its 0.5% budget (measured 0.567%; CI's wifi-less runners
    // never see this and so never caught it). Subscribe narrowly instead
    // of filtering after delivery: skip the AccessPoint path and the
    // Device.Wireless interface entirely, so the bus never delivers that
    // chatter here at all. StateChanged still catches every real
    // connect/disconnect/roam state transition; `network_tick` below
    // covers the signal-strength display and an AP roam that keeps the
    // same device state.
    let mut network_state = MessageStream::for_match_rule(
        MatchRule::builder()
            .msg_type(Type::Signal)
            .path_namespace("/org/freedesktop/NetworkManager/Devices")
            .map_err(|error| Error::new("build system signal rule", error.to_string()))?
            .member("StateChanged")
            .map_err(|error| Error::new("build system signal rule", error.to_string()))?
            .build(),
        &connection,
        Some(16),
    )
    .await
    .map_err(|error| {
        Error::new(
            "subscribe to NetworkManager device state",
            error.to_string(),
        )
    })?
    .fuse();
    let mut network_props = MessageStream::for_match_rule(
        MatchRule::builder()
            .msg_type(Type::Signal)
            .path_namespace("/org/freedesktop/NetworkManager/Devices")
            .map_err(|error| Error::new("build system signal rule", error.to_string()))?
            .member("PropertiesChanged")
            .map_err(|error| Error::new("build system signal rule", error.to_string()))?
            .arg(0, "org.freedesktop.NetworkManager.Device")
            .map_err(|error| Error::new("build system signal rule", error.to_string()))?
            .build(),
        &connection,
        Some(16),
    )
    .await
    .map_err(|error| {
        Error::new(
            "subscribe to NetworkManager device properties",
            error.to_string(),
        )
    })?
    .fuse();
    let mut network_root = MessageStream::for_match_rule(
        MatchRule::builder()
            .msg_type(Type::Signal)
            .path("/org/freedesktop/NetworkManager")
            .map_err(|error| Error::new("build system signal rule", error.to_string()))?
            .build(),
        &connection,
        Some(16),
    )
    .await
    .map_err(|error| Error::new("subscribe to NetworkManager", error.to_string()))?
    .fuse();
    let mut bluetooth = MessageStream::for_match_rule(rule("/org/bluez")?, &connection, Some(32))
        .await
        .map_err(|error| Error::new("subscribe to BlueZ", error.to_string()))?
        .fuse();
    let mut upower =
        MessageStream::for_match_rule(rule("/org/freedesktop/UPower")?, &connection, Some(16))
            .await
            .map_err(|error| Error::new("subscribe to UPower", error.to_string()))?
            .fuse();
    let mut legacy_profiles =
        MessageStream::for_match_rule(rule("/net/hadess/PowerProfiles")?, &connection, Some(8))
            .await
            .map_err(|error| Error::new("subscribe to power profiles", error.to_string()))?
            .fuse();

    send(sender, Event::Refresh(Sources::system_bus())).await?;
    *previous_error = None;
    // No per-signal throttle left to gate: AccessPoint and Device.Wireless
    // are never subscribed, so nothing here can be the frequent, filtered
    // "Unshown"/"SignalStrength" case `record_message` still defends
    // against for any interface a future rule broadens to include.
    let strength_due = false;
    let mut next_network_tick = std::time::Instant::now() + crate::model::SIGNAL_STRENGTH_REFRESH;
    // A wired or offline machine has no Wi-Fi strength to display and no AP
    // to roam between, so the tick would just be a wake with nothing to do.
    let mut wifi_active = wifi_device_connected(&connection).await;

    loop {
        let mut pending = Sources::empty();
        let closed = futures_util::FutureExt::fuse(sender.closed());
        let network_tick = futures_util::FutureExt::fuse(if wifi_active {
            futures_util::future::Either::Left(async_io::Timer::at(next_network_tick))
        } else {
            futures_util::future::Either::Right(std::future::pending())
        });
        futures_util::pin_mut!(closed, network_tick);
        futures_util::select! {
            message = network_state.next() => record_message(message, Sources { network: true, ..Sources::empty() }, strength_due, &mut pending)?,
            message = network_props.next() => record_message(message, Sources { network: true, ..Sources::empty() }, strength_due, &mut pending)?,
            message = network_root.next() => record_message(message, Sources { network: true, ..Sources::empty() }, strength_due, &mut pending)?,
            message = bluetooth.next() => record_message(message, Sources { bluetooth: true, ..Sources::empty() }, strength_due, &mut pending)?,
            message = upower.next() => record_message(message, Sources { power: true, ..Sources::empty() }, strength_due, &mut pending)?,
            message = legacy_profiles.next() => record_message(message, Sources { power: true, ..Sources::empty() }, strength_due, &mut pending)?,
            _ = network_tick => {
                next_network_tick = std::time::Instant::now() + crate::model::SIGNAL_STRENGTH_REFRESH;
                pending.merge(Sources { network: true, ..Sources::empty() });
            }
            _ = closed => return Ok(()),
        }
        if pending.is_empty() {
            continue;
        }

        loop {
            let quiet = futures_util::FutureExt::fuse(async_io::Timer::after(QUIET_PERIOD));
            let closed = futures_util::FutureExt::fuse(sender.closed());
            futures_util::pin_mut!(quiet, closed);
            futures_util::select! {
                message = network_state.next() => record_message(message, Sources { network: true, ..Sources::empty() }, strength_due, &mut pending)?,
                message = network_props.next() => record_message(message, Sources { network: true, ..Sources::empty() }, strength_due, &mut pending)?,
                message = network_root.next() => record_message(message, Sources { network: true, ..Sources::empty() }, strength_due, &mut pending)?,
                message = bluetooth.next() => record_message(message, Sources { bluetooth: true, ..Sources::empty() }, strength_due, &mut pending)?,
                message = upower.next() => record_message(message, Sources { power: true, ..Sources::empty() }, strength_due, &mut pending)?,
                message = legacy_profiles.next() => record_message(message, Sources { power: true, ..Sources::empty() }, strength_due, &mut pending)?,
                _ = quiet => break,
                _ = closed => return Ok(()),
            }
        }
        if pending.network {
            wifi_active = wifi_device_connected(&connection).await;
        }
        send(sender, Event::Refresh(pending)).await?;
    }
}

/// Whether any Wi-Fi device is currently activated, so `network_tick` can
/// skip its periodic wake entirely on a wired or offline machine. Best
/// effort: any D-Bus error (including no system bus) reads as "no", which
/// just means the tick stays off rather than the watcher failing.
#[cfg(target_os = "linux")]
async fn wifi_device_connected(connection: &zbus::Connection) -> bool {
    const WIFI_DEVICE_TYPE: u32 = 2;
    const ACTIVATED_STATE: u32 = 100;

    let Ok(manager) = zbus::Proxy::new(
        connection,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager",
        "org.freedesktop.NetworkManager",
    )
    .await
    else {
        return false;
    };
    let Ok(devices) = manager
        .call::<_, _, Vec<zbus::zvariant::OwnedObjectPath>>("GetDevices", &())
        .await
    else {
        return false;
    };
    for path in devices {
        let Ok(device) = zbus::Proxy::new(
            connection,
            "org.freedesktop.NetworkManager",
            path.as_str(),
            "org.freedesktop.NetworkManager.Device",
        )
        .await
        else {
            continue;
        };
        let Ok(device_type) = device.get_property::<u32>("DeviceType").await else {
            continue;
        };
        if device_type != WIFI_DEVICE_TYPE {
            continue;
        }
        if device.get_property::<u32>("State").await == Ok(ACTIVATED_STATE) {
            return true;
        }
    }
    false
}

/// Record which services a signal asks to re-read. `strength_due` says
/// whether a Wi-Fi signal-strength change alone may re-read the network yet.
#[cfg(target_os = "linux")]
fn record_message(
    message: Option<Result<zbus::Message, zbus::Error>>,
    sources: Sources,
    strength_due: bool,
    pending: &mut Sources,
) -> Result<(), Error> {
    use crate::model::PropertyChange;

    match message {
        Some(Ok(message)) => {
            match property_change(&message) {
                PropertyChange::Shown => pending.merge(sources),
                PropertyChange::SignalStrength if strength_due => pending.merge(sources),
                PropertyChange::SignalStrength | PropertyChange::Unshown => {}
            }
            Ok(())
        }
        Some(Err(error)) => Err(Error::new("read system service signal", error.to_string())),
        None => Err(Error::new(
            "read system service signal",
            "the D-Bus signal stream ended",
        )),
    }
}

#[cfg(target_os = "linux")]
fn property_change(message: &zbus::Message) -> crate::model::PropertyChange {
    use std::collections::HashMap;

    let header = message.header();
    if header.member().map(|member| member.as_str()) != Some("PropertiesChanged") {
        return crate::model::PropertyChange::Shown;
    }
    let body = message.body();
    let Ok((interface, changed, invalidated)) =
        body.deserialize::<(&str, HashMap<&str, zbus::zvariant::Value<'_>>, Vec<&str>)>()
    else {
        return crate::model::PropertyChange::Shown;
    };
    let changed = changed.keys().copied().collect::<Vec<_>>();
    crate::model::property_change(interface, &changed, &invalidated)
}

/// Builds the `async_process::Command` that runs `program args...`
/// (`pw-dump --monitor --no-colors` in production; a test passes a
/// different program/args so it never depends on PipeWire being
/// installed), bound to this process (see [`rmac_process::bind_to_parent`])
/// with its stdio piped/nulled and `kill_on_drop` set.
///
/// Same fix as `rmac-audio`'s identical watcher (`rmac-audio/src/linux.rs`'s
/// `build_monitor_command`, see its doc comment for the full story):
/// `Command::from(std::process::Command)` resets `async_process`'s own
/// stdin/stdout/stderr tracking, so a plain `.spawn()` would otherwise
/// silently replace the piped stdout with `Stdio::inherit()`,
/// `child.stdout` would always be `None`, and the freshly spawned `pw-dump`
/// would be SIGKILLed by `kill_on_drop` a moment after every spawn -- a
/// permanent one-second reconnect loop. Re-asserting the same stdio through
/// `async_process::Command`'s own builder sets the tracking flags so
/// `spawn()` leaves them alone -- `tests::command_pipes_stdout_through_async_process`
/// regression-tests this.
#[cfg(target_os = "linux")]
pub(crate) fn build_monitor_command(program: &str, args: &[&str]) -> async_process::Command {
    use std::process::Stdio;

    let mut monitor = std::process::Command::new(program);
    monitor
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // `kill_on_drop` covers a watcher that stops; binding covers a process
    // that exits without dropping it, so the monitor never outlives it.
    rmac_process::bind_to_parent(&mut monitor);
    let mut command = async_process::Command::from(monitor);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    command
}

#[cfg(target_os = "linux")]
async fn watch_audio_once(
    sender: &Sender<Event>,
    previous_error: &mut Option<Error>,
) -> Result<(), Error> {
    use futures_lite::io::AsyncReadExt as _;

    // `pw-dump --monitor` is the machine-readable PipeWire graph monitor: it
    // prints the full graph once, then a fresh JSON array of changed objects
    // on every subsequent state change. Any array that changes more than
    // PipeWire's client list is a "something changed, re-read the
    // authoritative state" trigger, matching `rmac-audio`'s own watcher.
    let mut command = build_monitor_command("pw-dump", &["--monitor", "--no-colors"]);
    let mut child = command
        .spawn()
        .map_err(|error| Error::new("start the PipeWire monitor", error.to_string()))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::new("start the PipeWire monitor", "stdout was not captured"))?;
    let mut buffer = [0_u8; 8192];
    // Re-reading the audio state runs one-shot PipeWire clients, which this
    // monitor reports; reacting to those would re-read forever.
    let mut changes = rmac_audio::MonitorChanges::default();
    let mut audio_changed = |bytes: &[u8]| {
        changes
            .feed(bytes)
            .map_err(|error| Error::new("read PipeWire changes", error))
    };

    // The process is listening before the authoritative audio snapshot is read.
    send(sender, Event::Refresh(Sources::audio())).await?;
    *previous_error = None;
    loop {
        let read = {
            let next = futures_util::FutureExt::fuse(stdout.read(&mut buffer));
            let closed = futures_util::FutureExt::fuse(sender.closed());
            futures_util::pin_mut!(next, closed);
            futures_util::select! {
                read = next => read,
                _ = closed => return Ok(()),
            }
        }
        .map_err(|error| Error::new("read PipeWire changes", error.to_string()))?;
        if read == 0 {
            return child_status_error(child).await;
        }
        if !audio_changed(&buffer[..read])? {
            continue;
        }

        loop {
            let read = {
                let next = futures_util::FutureExt::fuse(stdout.read(&mut buffer));
                let quiet = futures_util::FutureExt::fuse(async_io::Timer::after(QUIET_PERIOD));
                let closed = futures_util::FutureExt::fuse(sender.closed());
                futures_util::pin_mut!(next, quiet, closed);
                futures_util::select! {
                    read = next => read,
                    _ = quiet => break,
                    _ = closed => return Ok(()),
                }
            };
            let read =
                read.map_err(|error| Error::new("read PipeWire changes", error.to_string()))?;
            if read == 0 {
                return child_status_error(child).await;
            }
            // Already refreshing; this only keeps the monitor's framing.
            audio_changed(&buffer[..read])?;
        }
        send(sender, Event::Refresh(Sources::audio())).await?;
    }
}

#[cfg(target_os = "linux")]
async fn child_status_error(mut child: async_process::Child) -> Result<(), Error> {
    let status = child
        .status()
        .await
        .map_err(|error| Error::new("wait for the PipeWire monitor", error.to_string()))?;
    Err(Error::new(
        "watch PipeWire changes",
        format!("pw-dump --monitor exited with {status}"),
    ))
}

#[cfg(target_os = "linux")]
async fn send(sender: &Sender<Event>, event: Event) -> Result<(), Error> {
    sender
        .send(event)
        .await
        .map_err(|_| Error::new("publish service change", "consumer closed"))
}
