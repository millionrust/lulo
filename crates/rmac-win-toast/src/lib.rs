//! One Windows toast for a Lulo app.
//!
//! A toast belongs to an AppUserModelID (`Lulo.Mail`, `Lulo.Calendar`); Windows
//! shows it as that app's own only when a Start Menu shortcut carries the same
//! id, which the installer's shortcuts do (`packaging/windows/apps.json`). A
//! build run straight from `cargo run` has no such shortcut and Windows then
//! drops the toast, which `show` reports as `false`. Clicking a toast starts
//! the app, and a running Mail or Calendar takes that launch over its
//! single-instance pipe.

/// `text` with the characters XML gives meaning to escaped.
pub fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() && c != '\n' => {}
            c => out.push(c),
        }
    }
    out
}

/// The toast's XML payload.
pub fn payload(title: &str, body: &str) -> String {
    format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",
        xml_escape(title),
        xml_escape(body)
    )
}

/// Show a toast with this title and body for `aumid`. True when Windows
/// accepted it.
#[cfg(windows)]
pub fn show(aumid: &str, title: &str, body: &str) -> bool {
    use windows::core::HSTRING;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

    // SAFETY: initialising the apartment for this thread; a second call
    // (already initialised) returns an error that is fine to ignore.
    let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
    let attempt = || -> windows::core::Result<()> {
        let document = XmlDocument::new()?;
        document.LoadXml(&HSTRING::from(payload(title, body)))?;
        let toast = ToastNotification::CreateToastNotification(&document)?;
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(aumid))?.Show(&toast)
    };
    attempt().is_ok()
}

#[cfg(not(windows))]
pub fn show(_aumid: &str, _title: &str, _body: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_text_cannot_break_out_of_the_payload() {
        let xml = payload("A <b> & \"C\"", "x</text><text>y");
        assert!(!xml.contains("<b>"));
        assert!(xml.contains("&lt;b&gt; &amp; &quot;C&quot;"));
        assert_eq!(xml.matches("<text>").count(), 2);
    }

    #[test]
    fn control_characters_are_dropped() {
        assert_eq!(xml_escape("a\u{0}b\u{7}c\nd"), "abc\nd");
    }
}
