use std::io::Read as _;
use std::process::{Command, Stdio};

use crate::api::{command_error, Error, WatchEvent, SCHEMA, SCREEN_READER_KEY};

pub(crate) fn monitor_once(sender: &async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    // Bound to this watcher thread, so the monitor never outlives the app
    // (audit finding SES-01) and inherits none of its stray descriptors.
    let mut child = rmac_process::spawn_bound(
        Command::new("gsettings")
            .args(["monitor", SCHEMA, SCREEN_READER_KEY])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    )
    .map_err(|error| command_error("watch the screen-reader setting", error))?;
    let mut stdout = child.stdout.take().ok_or_else(|| {
        Error::new(
            "watch the screen-reader setting",
            "gsettings monitor did not provide an output stream",
        )
    })?;
    let _ = sender.try_send(WatchEvent::Available);
    // Block on the monitor's output: each line is a change. Nothing polls,
    // so an idle monitor costs no wake-ups. A closed receiver is noticed on
    // the next change (or when the monitor ends) and stops the monitor.
    let mut buffer = [0_u8; 1024];
    loop {
        match stdout.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if let Err(async_channel::TrySendError::Closed(_)) =
                    sender.try_send(WatchEvent::Changed)
                {
                    // Dropping the child kills and reaps it.
                    return Ok(());
                }
            }
        }
    }
    drop(child);
    if sender.is_closed() {
        return Ok(());
    }
    Err(Error::new(
        "watch the screen-reader setting",
        "the gsettings monitor stream ended",
    ))
}
