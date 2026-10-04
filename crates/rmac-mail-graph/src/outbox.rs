//! Outbox submission through Graph's `sendMail` with a MIME body. Graph keeps
//! a copy in Sent Items, as the Mac does for Exchange accounts.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use rmac_mail_runtime::Error;
use rmac_mail_storage::{MailStorage, OutboxState};

use crate::{
    http::MAX_JSON_BYTES, http_error, status_error, Client, HttpError, Method, GRAPH_ROOT,
};

/// Graph takes recipients from the MIME headers, but Mail's MIME builder
/// keeps Bcc out of the message (it is envelope-only for SMTP). Envelope
/// recipients that are not in To or Cc go back in as a `Bcc:` header, which
/// Exchange uses for delivery and strips from the delivered copies.
pub fn with_bcc(mime: &[u8], envelope: &[String]) -> Vec<u8> {
    let visible: Vec<String> = rmac_mail_mime::parse(mime)
        .map(|parsed| {
            parsed
                .to
                .iter()
                .chain(&parsed.cc)
                .map(|mailbox| mailbox.address.to_ascii_lowercase())
                .collect()
        })
        .unwrap_or_default();
    let hidden: Vec<&str> = envelope
        .iter()
        .map(String::as_str)
        .filter(|address| rmac_mail_mime::valid_address(address))
        .filter(|address| !address.bytes().any(|byte| byte.is_ascii_control()))
        .filter(|address| !visible.contains(&address.to_ascii_lowercase()))
        .collect();
    if hidden.is_empty() {
        return mime.to_vec();
    }
    let mut out = format!("Bcc: {}\r\n", hidden.join(", ")).into_bytes();
    out.extend_from_slice(mime);
    out
}

/// Sends every queued Outbox message. Mirrors the SMTP Outbox rules: a
/// message the server definitely did not accept goes back to Queued; one
/// whose fate is unknown (the connection dropped after sending, or the
/// server refused its content) is Held so it is never sent twice silently.
pub fn drain_outbox(client: &Client, store: &mut MailStorage) -> Result<usize, Error> {
    let mut sent = 0;
    while let Some(message) = store.claim_outbox()? {
        let mime = with_bcc(&message.bytes, &message.recipients);
        let body = STANDARD.encode(mime).into_bytes();
        let result = client.send(
            Method::Post,
            format!("{GRAPH_ROOT}/me/sendMail"),
            Some(("text/plain", body)),
            None,
            MAX_JSON_BYTES,
        );
        match result {
            Ok(response) if (200..300).contains(&response.status) => {
                store.complete_outbox(message.id)?;
                sent += 1;
            }
            Ok(response) => {
                let state = match response.status {
                    401 | 403 | 429 | 503 => OutboxState::Queued,
                    _ => OutboxState::Held,
                };
                store.update_outbox_state(message.id, state)?;
                return Err(status_error(response.status));
            }
            Err(error) => {
                let state = if error == HttpError::NotSent {
                    OutboxState::Queued
                } else {
                    OutboxState::Held
                };
                store.update_outbox_state(message.id, state)?;
                return Err(http_error(error));
            }
        }
    }
    Ok(sent)
}
