//! Handing a `.ics` / `text/calendar` Mail attachment to Calendar (MAIL-8,
//! `docs/design/calendar-mail.md` §3's "`.ics` attachments handed to
//! Calendar"). Mail's job stops at staging the bytes under a private,
//! owner-only temp path and asking the desktop portal to open it;
//! `text/calendar` is registered to `org.rmac.Calendar.desktop` in
//! `packaging/rmac-apps`, so `rmac_portal::open_item` (called from the
//! view, which owns an async executor) routes there.

use std::io;
use std::path::{Path, PathBuf};

use rmac_mail_mime::Attachment;

/// Mail directory for staged calendar hand-offs, kept apart from any other
/// temp content so a stale file is obviously Mail's to clean up.
const STAGING_DIRECTORY: &str = "lulo-mail-calendar-handoff";

/// Whether `attachment` is a calendar invite/subscription Calendar should
/// open, by content type or (IMAP servers often mislabel `.ics` parts as
/// `application/octet-stream`) by filename extension.
pub fn is_calendar_attachment(attachment: &Attachment) -> bool {
    attachment
        .content_type
        .eq_ignore_ascii_case("text/calendar")
        || has_ics_extension(&attachment.filename)
}

pub fn has_ics_extension(filename: &str) -> bool {
    Path::new(filename)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ics"))
}

/// Writes `bytes` to a private temp file named after the attachment, ready
/// for `rmac_portal::open_item`. Pure I/O, no portal call, so it is unit
/// tested directly; the live hand-off is covered by ADR 0022's "How to
/// verify on the reference laptop".
pub fn stage_for_handoff(filename: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    let directory = std::env::temp_dir().join(STAGING_DIRECTORY);
    rmac_storage::create_dir_all_private(&directory)?;
    let path = directory.join(sanitize_filename(filename));
    rmac_storage::atomic_write_private(&path, bytes)?;
    Ok(path)
}

/// Keeps only the final path component, so an attachment filename can never
/// stage itself outside `STAGING_DIRECTORY`; a filename that leaves nothing
/// usable falls back to a fixed name.
fn sanitize_filename(filename: &str) -> String {
    Path::new(filename)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("invite.ics")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attachment(filename: &str, content_type: &str) -> Attachment {
        Attachment {
            filename: filename.to_owned(),
            content_type: content_type.to_owned(),
            bytes: Vec::new(),
            content_id: None,
        }
    }

    #[test]
    fn calendar_mime_type_is_recognised_regardless_of_filename() {
        assert!(is_calendar_attachment(&attachment(
            "whatever.bin",
            "text/calendar"
        )));
        assert!(is_calendar_attachment(&attachment(
            "whatever.bin",
            "TEXT/CALENDAR"
        )));
    }

    #[test]
    fn ics_extension_is_recognised_even_when_mislabelled() {
        assert!(is_calendar_attachment(&attachment(
            "Team-sync.ICS",
            "application/octet-stream"
        )));
        assert!(!is_calendar_attachment(&attachment(
            "Menu.pdf",
            "application/pdf"
        )));
    }

    #[test]
    fn staging_writes_the_exact_bytes_as_owner_only() {
        let path =
            stage_for_handoff("Reminder-sync.ics", b"BEGIN:VCALENDAR\nEND:VCALENDAR\n").unwrap();
        assert_eq!(path.file_name().unwrap(), "Reminder-sync.ics");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"BEGIN:VCALENDAR\nEND:VCALENDAR\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_path_escaping_filename_stays_inside_the_staging_directory() {
        let path = stage_for_handoff("../../etc/passwd.ics", b"x").unwrap();
        let directory = std::env::temp_dir().join(STAGING_DIRECTORY);
        assert_eq!(path.parent().unwrap(), directory);
        assert_eq!(path.file_name().unwrap(), "passwd.ics");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_empty_filename_falls_back_to_a_fixed_name() {
        assert_eq!(sanitize_filename(""), "invite.ics");
        assert_eq!(sanitize_filename("../"), "invite.ics");
    }
}
