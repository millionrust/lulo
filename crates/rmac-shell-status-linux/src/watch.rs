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

#[cfg(target_os = "linux")]
// `pw-mon` prints a line for every PipeWire event, and each coalesced flush
// makes the shell rebuild the full audio snapshot (a `pw-dump` subprocess plus
// JSON parse) on the blocking pool. 75 ms flushed several times a second at
// idle and kept the menu bar ~35% busy; 500 ms bounds that while keeping a
// volume change prompt.
const QUIET_PERIOD: std::time::Duration = std::time::Duration::from_millis(500);

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
    loop {
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
                async_io::Timer::after(RECONNECT_DELAY).await;
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
    let mut network = MessageStream::for_match_rule(
        rule("/org/freedesktop/NetworkManager")?,
        &connection,
        Some(32),
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

    loop {
        let mut pending = Sources::empty();
        let closed = futures_util::FutureExt::fuse(sender.closed());
        futures_util::pin_mut!(closed);
        futures_util::select! {
            message = network.next() => record_message(message, Sources { network: true, ..Sources::empty() }, &mut pending)?,
            message = bluetooth.next() => record_message(message, Sources { bluetooth: true, ..Sources::empty() }, &mut pending)?,
            message = upower.next() => record_message(message, Sources { power: true, ..Sources::empty() }, &mut pending)?,
            message = legacy_profiles.next() => record_message(message, Sources { power: true, ..Sources::empty() }, &mut pending)?,
            _ = closed => return Ok(()),
        }

        loop {
            let quiet = futures_util::FutureExt::fuse(async_io::Timer::after(QUIET_PERIOD));
            let closed = futures_util::FutureExt::fuse(sender.closed());
            futures_util::pin_mut!(quiet, closed);
            futures_util::select! {
                message = network.next() => record_message(message, Sources { network: true, ..Sources::empty() }, &mut pending)?,
                message = bluetooth.next() => record_message(message, Sources { bluetooth: true, ..Sources::empty() }, &mut pending)?,
                message = upower.next() => record_message(message, Sources { power: true, ..Sources::empty() }, &mut pending)?,
                message = legacy_profiles.next() => record_message(message, Sources { power: true, ..Sources::empty() }, &mut pending)?,
                _ = quiet => break,
                _ = closed => return Ok(()),
            }
        }
        send(sender, Event::Refresh(pending)).await?;
    }
}

#[cfg(target_os = "linux")]
fn record_message(
    message: Option<Result<zbus::Message, zbus::Error>>,
    sources: Sources,
    pending: &mut Sources,
) -> Result<(), Error> {
    match message {
        Some(Ok(_)) => {
            pending.merge(sources);
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
async fn watch_audio_once(
    sender: &Sender<Event>,
    previous_error: &mut Option<Error>,
) -> Result<(), Error> {
    use std::process::Stdio;

    use futures_lite::{
        io::{AsyncBufReadExt as _, BufReader},
        StreamExt as _,
    };

    let mut command = async_process::Command::new("pw-mon");
    command
        .arg("--color=never")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|error| Error::new("start the PipeWire monitor", error.to_string()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::new("start the PipeWire monitor", "stdout was not captured"))?;
    let mut lines = BufReader::new(stdout).lines();

    // The process is listening before the authoritative audio snapshot is read.
    send(sender, Event::Refresh(Sources::audio())).await?;
    *previous_error = None;
    loop {
        let next = futures_util::FutureExt::fuse(lines.next());
        let closed = futures_util::FutureExt::fuse(sender.closed());
        futures_util::pin_mut!(next, closed);
        let line = futures_util::select! {
            line = next => line,
            _ = closed => return Ok(()),
        };
        let Some(line) = line else {
            return child_status_error(child).await;
        };
        line.map_err(|error| Error::new("read PipeWire changes", error.to_string()))?;
        loop {
            let next = futures_util::FutureExt::fuse(lines.next());
            let quiet = futures_util::FutureExt::fuse(async_io::Timer::after(QUIET_PERIOD));
            let closed = futures_util::FutureExt::fuse(sender.closed());
            futures_util::pin_mut!(next, quiet, closed);
            futures_util::select! {
                line = next => match line {
                    Some(line) => {
                        line.map_err(|error| Error::new("read PipeWire changes", error.to_string()))?;
                    }
                    None => return child_status_error(child).await,
                },
                _ = quiet => break,
                _ = closed => return Ok(()),
            }
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
        format!("pw-mon exited with {status}"),
    ))
}

#[cfg(target_os = "linux")]
async fn send(sender: &Sender<Event>, event: Event) -> Result<(), Error> {
    sender
        .send(event)
        .await
        .map_err(|_| Error::new("publish service change", "consumer closed"))
}
