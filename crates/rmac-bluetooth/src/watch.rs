#[cfg(not(target_os = "macos"))]
use std::time::Duration;

use crate::{Error, WatchEvent};

#[cfg(not(target_os = "macos"))]
const BLUEZ_SERVICE: &str = "org.bluez";
#[cfg(not(target_os = "macos"))]
const WATCH_RECONNECT_DELAY: Duration = Duration::from_secs(1);
#[cfg(not(target_os = "macos"))]
const WATCH_QUIET_PERIOD: Duration = Duration::from_millis(75);
#[cfg(not(target_os = "macos"))]
pub(crate) async fn system_watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    let mut unavailable_reported = false;
    loop {
        match watch_once(&sender, &mut unavailable_reported).await {
            Ok(()) if sender.is_closed() => return Ok(()),
            Ok(()) => {}
            Err(_) if sender.is_closed() => return Ok(()),
            Err(_) => publish_unavailable(&sender, &mut unavailable_reported).await?,
        }
        async_io::Timer::after(WATCH_RECONNECT_DELAY).await;
    }
}

#[cfg(target_os = "macos")]
pub(crate) async fn system_watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    sender
        .send(WatchEvent::Unavailable)
        .await
        .map_err(|_| Error::new("watch Bluetooth changes", "the event consumer closed"))
}

#[cfg(not(target_os = "macos"))]
async fn watch_once(
    sender: &async_channel::Sender<WatchEvent>,
    unavailable_reported: &mut bool,
) -> Result<(), Error> {
    use futures_util::{FutureExt as _, StreamExt as _};
    use zbus::{message::Type, MatchRule, MessageStream};

    let connection = rmac_dbus::system()
        .await
        .map_err(|error| Error::new("connect Bluetooth event stream", error.to_string()))?;
    let bluez_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(BLUEZ_SERVICE)
        .map_err(|error| Error::new("build Bluetooth signal filter", error.to_string()))?
        .build();
    let owner_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")
        .map_err(|error| Error::new("build Bluetooth owner filter", error.to_string()))?
        .path("/org/freedesktop/DBus")
        .map_err(|error| Error::new("build Bluetooth owner filter", error.to_string()))?
        .interface("org.freedesktop.DBus")
        .map_err(|error| Error::new("build Bluetooth owner filter", error.to_string()))?
        .member("NameOwnerChanged")
        .map_err(|error| Error::new("build Bluetooth owner filter", error.to_string()))?
        .add_arg(BLUEZ_SERVICE)
        .map_err(|error| Error::new("build Bluetooth owner filter", error.to_string()))?
        .build();
    let mut bluez = MessageStream::for_match_rule(bluez_rule, &connection, Some(64))
        .await
        .map_err(|error| Error::new("subscribe to Bluetooth changes", error.to_string()))?
        .fuse();
    let mut owners = MessageStream::for_match_rule(owner_rule, &connection, Some(8))
        .await
        .map_err(|error| Error::new("subscribe to BlueZ restarts", error.to_string()))?
        .fuse();

    let dbus = zbus::fdo::DBusProxy::new(&connection)
        .await
        .map_err(|error| Error::new("inspect BlueZ service", error.to_string()))?;
    let service = zbus::names::BusName::try_from(BLUEZ_SERVICE)
        .map_err(|error| Error::new("inspect BlueZ service", error.to_string()))?;
    let mut available = dbus
        .name_has_owner(service)
        .await
        .map_err(|error| Error::new("inspect BlueZ service", error.to_string()))?;
    if available {
        publish_changed(sender, unavailable_reported).await?;
    } else {
        publish_unavailable(sender, unavailable_reported).await?;
    }

    loop {
        let closed = sender.closed().fuse();
        futures_util::pin_mut!(closed);
        let signal = futures_util::select! {
            message = bluez.next() => {
                read_signal(message)?;
                Some(true)
            },
            message = owners.next() => read_bluez_owner(message)?,
            _ = closed => return Ok(()),
        };
        let Some(signal_available) = signal else {
            continue;
        };
        if !signal_available {
            available = false;
            publish_unavailable(sender, unavailable_reported).await?;
            continue;
        }
        if !available {
            available = true;
        }
        let mut refresh_pending = true;

        loop {
            let quiet = futures_util::FutureExt::fuse(async_io::Timer::after(WATCH_QUIET_PERIOD));
            let closed = sender.closed().fuse();
            futures_util::pin_mut!(quiet, closed);
            let signal = futures_util::select! {
                message = bluez.next() => {
                    read_signal(message)?;
                    Some(true)
                },
                message = owners.next() => read_bluez_owner(message)?,
                _ = quiet => break,
                _ = closed => return Ok(()),
            };
            if let Some(signal_available) = signal {
                available = signal_available;
                refresh_pending = signal_available;
                if !signal_available {
                    publish_unavailable(sender, unavailable_reported).await?;
                }
            }
        }
        if available && refresh_pending {
            publish_changed(sender, unavailable_reported).await?;
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn read_signal(message: Option<Result<zbus::Message, zbus::Error>>) -> Result<(), Error> {
    match message {
        Some(Ok(_)) => Ok(()),
        Some(Err(error)) => Err(Error::new("read Bluetooth change", error.to_string())),
        None => Err(Error::new(
            "read Bluetooth change",
            "the signal stream ended",
        )),
    }
}

#[cfg(not(target_os = "macos"))]
fn read_bluez_owner(
    message: Option<Result<zbus::Message, zbus::Error>>,
) -> Result<Option<bool>, Error> {
    let message = message
        .ok_or_else(|| Error::new("read BlueZ owner", "the signal stream ended"))?
        .map_err(|error| Error::new("read BlueZ owner", error.to_string()))?;
    let (name, _old_owner, new_owner): (String, String, String) = message
        .body()
        .deserialize()
        .map_err(|error| Error::new("read BlueZ owner", error.to_string()))?;
    Ok(bluez_owner_availability(&name, &new_owner))
}

#[cfg(any(not(target_os = "macos"), test))]
pub(crate) fn bluez_owner_availability(name: &str, new_owner: &str) -> Option<bool> {
    (name == "org.bluez").then_some(!new_owner.is_empty())
}

#[cfg(not(target_os = "macos"))]
async fn publish_changed(
    sender: &async_channel::Sender<WatchEvent>,
    unavailable_reported: &mut bool,
) -> Result<(), Error> {
    if *unavailable_reported {
        sender
            .send(WatchEvent::Changed)
            .await
            .map_err(|_| Error::new("publish Bluetooth change", "the event consumer closed"))?;
    } else {
        let _ = sender.try_send(WatchEvent::Changed);
    }
    *unavailable_reported = false;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
async fn publish_unavailable(
    sender: &async_channel::Sender<WatchEvent>,
    unavailable_reported: &mut bool,
) -> Result<(), Error> {
    if !*unavailable_reported {
        sender
            .send(WatchEvent::Unavailable)
            .await
            .map_err(|_| Error::new("publish Bluetooth outage", "the event consumer closed"))?;
        *unavailable_reported = true;
    }
    Ok(())
}
