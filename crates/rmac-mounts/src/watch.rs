#[cfg(target_os = "linux")]
use std::io;
#[cfg(target_os = "linux")]
use std::path::Path;

#[cfg(target_os = "linux")]
use crate::model::MOUNT_WATCH_TIMEOUT_SECONDS;
use crate::{Error, WatchEvent};

/// Watch the caller's Linux mount namespace. `/proc/self/mounts` implements
/// `POLLPRI` for mount and unmount changes; each event is only a hint and
/// consumers must take a complete fresh `volumes()` snapshot.
pub async fn watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    #[cfg(target_os = "linux")]
    {
        let worker_sender = sender.clone();
        let result = blocking::unblock(move || watch_mount_changes(&worker_sender)).await;
        if let Err(error) = result {
            let _ = sender.send(WatchEvent::Unavailable).await;
            return Err(error);
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = sender.send(WatchEvent::Unavailable).await;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn watch_mount_changes(sender: &async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    use rustix::event::{poll, PollFd, PollFlags, Timespec};

    let path = Path::new("/proc/self/mounts");
    let file = std::fs::File::open(path).map_err(|source| Error::Io {
        operation: "watch mount table",
        path: path.to_path_buf(),
        source,
    })?;
    let _ = sender.try_send(WatchEvent::Changed);
    let timeout = Timespec {
        tv_sec: MOUNT_WATCH_TIMEOUT_SECONDS,
        tv_nsec: 0,
    };
    while !sender.is_closed() {
        let mut descriptors = [PollFd::new(&file, PollFlags::PRI | PollFlags::ERR)];
        let ready = poll(&mut descriptors, Some(&timeout)).map_err(|source| Error::Io {
            operation: "watch mount table",
            path: path.to_path_buf(),
            source: io::Error::from(source),
        })?;
        if ready > 0
            && descriptors[0]
                .revents()
                .intersects(PollFlags::PRI | PollFlags::ERR)
        {
            let _ = sender.try_send(WatchEvent::Changed);
        }
    }
    Ok(())
}
