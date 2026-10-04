//! Turning what polkit and the helper send into text the dialog can draw.
//! Everything here is untrusted input: it is decoded, stripped of control
//! characters and bounded, and the dialog renders it as plain text (GPUI has
//! no markup, so nothing in it is ever interpreted).

/// Longest message drawn from polkit or PAM, in characters.
pub const MAX_MESSAGE_CHARS: usize = 300;
/// Longest requesting-app name, in characters.
pub const MAX_APP_NAME_CHARS: usize = 40;

/// GLib's `g_strcompress`, which polkit's agent library applies to every
/// helper line: `\b \f \n \r \t \v \\ \"`, one-to-three-digit octal escapes,
/// and any other escaped character standing for itself.
pub fn unescape(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        index += 1;
        if byte != b'\\' {
            out.push(byte);
            continue;
        }
        let Some(&next) = bytes.get(index) else {
            break;
        };
        index += 1;
        match next {
            b'0'..=b'7' => {
                let mut value = u32::from(next - b'0');
                let mut digits = 1;
                while digits < 3 {
                    match bytes.get(index) {
                        Some(&digit @ b'0'..=b'7') => {
                            value = value * 8 + u32::from(digit - b'0');
                            index += 1;
                            digits += 1;
                        }
                        _ => break,
                    }
                }
                out.push((value & 0xff) as u8);
            }
            b'b' => out.push(0x08),
            b'f' => out.push(0x0c),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'v' => out.push(0x0b),
            other => out.push(other),
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Plain, single-paragraph display text: whitespace controls become spaces,
/// other controls (and bidi overrides, which could reorder what the user
/// reads) are dropped, runs of spaces collapse, and the result is cut to
/// `max_chars` with an ellipsis.
pub fn display(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut count = 0;
    let mut pending_space = false;
    for character in text.chars() {
        let character = match character {
            '\n' | '\r' | '\t' | '\u{0b}' | '\u{0c}' => ' ',
            '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}' => continue,
            other if other.is_control() => continue,
            other => other,
        };
        if character == ' ' {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            if count + 1 >= max_chars {
                break;
            }
            out.push(' ');
            count += 1;
            pending_space = false;
        }
        if count >= max_chars {
            out.push('…');
            return out;
        }
        out.push(character);
        count += 1;
    }
    out
}

/// The name the dialog's title gives a requesting executable: Lulo's own
/// apps by their app names, anything else by its sanitised command name.
pub fn app_name(executable: &str) -> String {
    let base = executable.rsplit('/').next().unwrap_or(executable);
    let known = match base {
        "rmac-system-settings" | "rmac-system-set" | "system-settings" => "System Settings",
        "rmac-files" | "finder" => "Files",
        "rmac-setup-assistant" | "rmac-setup-assi" => "Setup Assistant",
        "rmac-system-monitor" | "rmac-system-mon" => "Activity Monitor",
        "rmac-archive-utility" | "rmac-archive-ut" => "Archive Utility",
        "rmac-terminal" => "Terminal",
        "rmac-text-editor" | "rmac-text-edito" => "TextEdit",
        "rmac-preview" => "Preview",
        "rmac-app-drawer" => "Apps",
        "rmac-dock" => "Dock",
        "rmac-top-bar" => "Lulo",
        "rmac-update-check" | "rmac-update-che" => "Software Update",
        _ => "",
    };
    if !known.is_empty() {
        return known.to_owned();
    }
    let name = display(base, MAX_APP_NAME_CHARS);
    if name.is_empty() {
        "An app".to_owned()
    } else {
        name
    }
}

/// Polkit's placeholder for the password field: "Password: " → "Password".
pub fn prompt_placeholder(prompt: &str) -> String {
    let text = display(prompt, 60);
    let trimmed = text.trim().trim_end_matches(':').trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("password") {
        "Password".to_owned()
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescape_matches_g_strcompress() {
        assert_eq!(unescape(r"a\tb\\c\042\nd\q"), "a\tb\\c\"\ndq");
        assert_eq!(unescape(r"\303\251"), "é");
        assert_eq!(unescape("trailing\\"), "trailing");
        assert_eq!(unescape(r"\0"), "\0");
    }

    #[test]
    fn display_text_is_plain_and_bounded() {
        assert_eq!(
            display("  <b>Hi</b>\n\tthere\u{202e}evil\u{7}  ", 100),
            "<b>Hi</b> thereevil"
        );
        assert_eq!(display("abcdef", 3), "abc…");
        assert_eq!(display("", 3), "");
    }

    #[test]
    fn requesting_apps_get_their_names() {
        assert_eq!(
            app_name("/usr/libexec/rmac/rmac-system-settings"),
            "System Settings"
        );
        assert_eq!(app_name("rmac-system-set"), "System Settings");
        assert_eq!(app_name("/usr/bin/pkexec"), "pkexec");
        assert_eq!(app_name("\u{1b}[31m"), "[31m");
        assert_eq!(app_name("\u{7}"), "An app");
    }

    #[test]
    fn prompts_become_placeholders() {
        assert_eq!(prompt_placeholder("Password: "), "Password");
        assert_eq!(
            prompt_placeholder("Verification code:"),
            "Verification code"
        );
        assert_eq!(prompt_placeholder(""), "Password");
    }
}
