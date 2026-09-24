use std::io::Read as _;
use std::process::{Command, Stdio};

use crate::api::{command_error, Error, WatchEvent, MAX_ERROR_BYTES, SCHEMA, TEXT_SCALE_KEY};

pub(crate) fn monitor_once(sender: &async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    let mut child = Command::new("gsettings")
        .args(["monitor", SCHEMA, TEXT_SCALE_KEY])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| command_error("watch GTK text scaling", error))?;
    let mut stdout = child.stdout.take().ok_or_else(|| {
        Error::new(
            "watch GTK text scaling",
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
                    let _ = child.kill();
                    let _ = child.wait();
                    return Ok(());
                }
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    if sender.is_closed() {
        return Ok(());
    }
    Err(Error::new(
        "watch GTK text scaling",
        "the gsettings monitor stream ended",
    ))
}

pub(crate) fn bounded_text(value: &str) -> String {
    let normalized = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let mut end = normalized.len().min(MAX_ERROR_BYTES);
    while !normalized.is_char_boundary(end) {
        end -= 1;
    }
    normalized[..end].trim().to_string()
}
