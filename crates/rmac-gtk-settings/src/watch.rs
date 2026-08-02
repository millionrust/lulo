use std::io::Read as _;
use std::process::{Command, Stdio};

use crate::api::{
    command_error, Error, WatchEvent, MAX_ERROR_BYTES, PROCESS_POLL_INTERVAL, SCHEMA,
    TEXT_SCALE_KEY,
};

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
    let event_sender = sender.clone();
    let (reader_done_tx, reader_done_rx) = std::sync::mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let mut buffer = [0_u8; 1024];
        loop {
            match stdout.read(&mut buffer) {
                Ok(0) => break,
                Ok(_) => {
                    let _ = event_sender.try_send(WatchEvent::Changed);
                }
                Err(_) => break,
            }
        }
        let _ = reader_done_tx.send(());
    });
    loop {
        if sender.is_closed() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Ok(());
        }
        if reader_done_rx.try_recv().is_ok() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(Error::new(
                "watch GTK text scaling",
                "the gsettings monitor stream ended",
            ));
        }
        match child.try_wait() {
            Ok(Some(_)) => {
                let _ = reader.join();
                return Err(Error::new(
                    "watch GTK text scaling",
                    "the gsettings monitor process ended",
                ));
            }
            Ok(None) => std::thread::sleep(PROCESS_POLL_INTERVAL),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(Error::new(
                    "watch GTK text scaling",
                    "the gsettings monitor could not be inspected",
                ));
            }
        }
    }
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
