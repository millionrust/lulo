use rmac_time::{Error, ErrorKind};

#[cfg(target_os = "linux")]
const RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

#[cfg(target_os = "linux")]
pub async fn watch(sender: async_channel::Sender<rmac_time::WatchEvent>) -> Result<(), Error> {
    loop {
        match watch_once(&sender).await {
            Ok(()) if sender.is_closed() => return Ok(()),
            Ok(()) => {}
            Err(_) if sender.is_closed() => return Ok(()),
            Err(_) => {
                let _ = sender.try_send(rmac_time::WatchEvent::Unavailable);
            }
        }
        async_io::Timer::after(RECONNECT_DELAY).await;
    }
}

/// Wait for an absolute realtime deadline.
///
/// Unlike a monotonic userspace sleep, this deadline advances during suspend.
/// Dropping the future closes its timer descriptor, so callers can replace an
/// obsolete deadline immediately after a clock or time-zone event.
#[cfg(target_os = "linux")]
pub async fn wait_until_realtime(deadline: std::time::SystemTime) -> Result<(), Error> {
    use rustix::time::{timerfd_create, timerfd_settime, Itimerspec, TimerfdClockId, TimerfdFlags};

    let (seconds, nanoseconds) = realtime_parts(deadline)?;
    let descriptor = timerfd_create(
        TimerfdClockId::Realtime,
        TimerfdFlags::CLOEXEC | TimerfdFlags::NONBLOCK,
    )
    .map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "could not create a realtime deadline",
        )
    })?;
    timerfd_settime(
        &descriptor,
        rustix::time::TimerfdTimerFlags::ABSTIME,
        &Itimerspec {
            it_interval: rustix::time::Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
            it_value: rustix::time::Timespec {
                tv_sec: seconds,
                tv_nsec: nanoseconds,
            },
        },
    )
    .map_err(|_| Error::new(ErrorKind::Unavailable, "could not arm a realtime deadline"))?;
    async_io::Async::new(descriptor)
        .map_err(|_| {
            Error::new(
                ErrorKind::Unavailable,
                "could not register a realtime deadline",
            )
        })?
        .read_with(|descriptor| {
            let mut value = [0_u8; 8];
            match rustix::io::read(descriptor, &mut value) {
                Ok(8) => Ok(()),
                Ok(_) => Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "incomplete realtime deadline event",
                )),
                Err(error) => Err(std::io::Error::from(error)),
            }
        })
        .await
        .map_err(|_| Error::new(ErrorKind::Unavailable, "the realtime deadline failed"))
}

#[cfg(not(target_os = "linux"))]
pub async fn watch(sender: async_channel::Sender<rmac_time::WatchEvent>) -> Result<(), Error> {
    sender
        .send(rmac_time::WatchEvent::Unavailable)
        .await
        .map_err(|_| Error::new(ErrorKind::Unavailable, "date and time watcher closed"))
}

#[cfg(target_os = "linux")]
async fn watch_once(sender: &async_channel::Sender<rmac_time::WatchEvent>) -> Result<(), Error> {
    use futures_util::{FutureExt as _, StreamExt as _};
    use zbus::{message::Type, MatchRule, MessageStream};

    let connection = rmac_dbus::system().await.map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "the system date and time event stream is unavailable",
        )
    })?;
    let properties_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.timedate1")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid timedated signal sender"))?
        .path("/org/freedesktop/timedate1")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid timedated event path"))?
        .interface("org.freedesktop.DBus.Properties")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid properties interface"))?
        .member("PropertiesChanged")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid properties signal"))?
        .add_arg("org.freedesktop.timedate1")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid timedated property filter"))?
        .build();
    let owner_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid D-Bus sender"))?
        .interface("org.freedesktop.DBus")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid D-Bus interface"))?
        .member("NameOwnerChanged")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid owner-change signal"))?
        .add_arg("org.freedesktop.timedate1")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid timedated owner filter"))?
        .build();
    let mut properties = MessageStream::for_match_rule(properties_rule, &connection, Some(8))
        .await
        .map_err(|_| Error::new(ErrorKind::Unavailable, "could not watch timedated changes"))?
        .fuse();
    let mut owners = MessageStream::for_match_rule(owner_rule, &connection, Some(4))
        .await
        .map_err(|_| Error::new(ErrorKind::Unavailable, "could not watch timedated restarts"))?
        .fuse();
    let clock = clock_change_detector()?;

    // Publish only after every subscription and the discontinuous-clock
    // detector are armed. Consumers can take their initial wall-clock sample
    // without a snapshot-before-watch race.
    if sender.send(rmac_time::WatchEvent::Changed).await.is_err() {
        return Ok(());
    }

    loop {
        let closed = sender.closed().fuse();
        let clock_changed = wait_for_clock_change(&clock).fuse();
        futures_util::pin_mut!(closed);
        futures_util::pin_mut!(clock_changed);
        let changed = futures_util::select! {
            message = properties.next() => {
                message
                    .ok_or_else(|| Error::new(ErrorKind::Unavailable, "timedated event stream ended"))?
                    .map_err(|_| Error::new(ErrorKind::Unavailable, "timedated event stream failed"))?;
                true
            },
            message = owners.next() => {
                owner_reappeared(message)?
            },
            result = clock_changed => {
                result?;
                true
            },
            _ = closed => return Ok(()),
        };
        if changed {
            let _ = sender.try_send(rmac_time::WatchEvent::Changed);
        }
    }
}

#[cfg(target_os = "linux")]
fn clock_change_detector() -> Result<async_io::Async<rustix::fd::OwnedFd>, Error> {
    use rustix::time::{timerfd_create, TimerfdClockId, TimerfdFlags};

    let descriptor = timerfd_create(
        TimerfdClockId::Realtime,
        TimerfdFlags::CLOEXEC | TimerfdFlags::NONBLOCK,
    )
    .map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "could not watch discontinuous system-clock changes",
        )
    })?;
    arm_clock_change_detector(&descriptor)?;
    async_io::Async::new(descriptor).map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "could not register the system-clock change detector",
        )
    })
}

#[cfg(target_os = "linux")]
fn arm_clock_change_detector(descriptor: &rustix::fd::OwnedFd) -> Result<(), Error> {
    use rustix::time::{timerfd_settime, Itimerspec, TimerfdTimerFlags, Timespec};

    // A finite rolling deadline avoids overflowing the kernel's internal
    // nanosecond range. Natural expiry causes one harmless annual refresh and
    // rearm; a discontinuous CLOCK_REALTIME change cancels it immediately.
    const ONE_YEAR_SECONDS: u64 = 365 * 24 * 60 * 60;
    const KTIME_MAX_SECONDS: u64 = i64::MAX as u64 / 1_000_000_000;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| {
            Error::new(
                ErrorKind::Unavailable,
                "could not read the realtime clock for change detection",
            )
        })?
        .as_secs();
    let deadline = now
        .checked_add(ONE_YEAR_SECONDS)
        .filter(|deadline| *deadline <= KTIME_MAX_SECONDS)
        .ok_or_else(|| {
            Error::new(
                ErrorKind::Unavailable,
                "the realtime clock is outside the safe change-detection range",
            )
        })? as i64;
    let far_future = Itimerspec {
        it_interval: Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        },
        it_value: Timespec {
            tv_sec: deadline,
            tv_nsec: 0,
        },
    };
    timerfd_settime(
        descriptor,
        TimerfdTimerFlags::ABSTIME | TimerfdTimerFlags::CANCEL_ON_SET,
        &far_future,
    )
    .map(|_| ())
    .map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "could not arm the system-clock change detector",
        )
    })
}

#[cfg(target_os = "linux")]
async fn wait_for_clock_change(
    detector: &async_io::Async<rustix::fd::OwnedFd>,
) -> Result<(), Error> {
    detector
        .read_with(|descriptor| {
            let mut value = [0_u8; 8];
            match rustix::io::read(descriptor, &mut value) {
                Ok(_) | Err(rustix::io::Errno::CANCELED) => Ok(()),
                Err(error) => Err(std::io::Error::from(error)),
            }
        })
        .await
        .map_err(|_| {
            Error::new(
                ErrorKind::Unavailable,
                "the system-clock change detector failed",
            )
        })?;
    arm_clock_change_detector(detector.get_ref())
}

#[cfg(target_os = "linux")]
fn owner_reappeared(message: Option<Result<zbus::Message, zbus::Error>>) -> Result<bool, Error> {
    let message = message
        .ok_or_else(|| Error::new(ErrorKind::Unavailable, "D-Bus owner stream ended"))?
        .map_err(|_| Error::new(ErrorKind::Unavailable, "D-Bus owner stream failed"))?;
    let (name, _old_owner, new_owner): (String, String, String) = message
        .body()
        .deserialize()
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid timedated owner change"))?;
    Ok(owner_change_reappeared(&name, &new_owner))
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn owner_change_reappeared(name: &str, new_owner: &str) -> bool {
    name == "org.freedesktop.timedate1" && !new_owner.is_empty()
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn realtime_parts(deadline: std::time::SystemTime) -> Result<(i64, i64), Error> {
    let deadline = deadline
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| {
            Error::new(
                ErrorKind::InvalidTime,
                "realtime deadline predates the epoch",
            )
        })?;
    let seconds = i64::try_from(deadline.as_secs()).map_err(|_| {
        Error::new(
            ErrorKind::InvalidTime,
            "realtime deadline exceeds the kernel clock range",
        )
    })?;
    Ok((seconds, i64::from(deadline.subsec_nanos())))
}
