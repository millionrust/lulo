//! Putting the chosen emoji into whatever text field has the keyboard.
//!
//! Wayland gives a client no way to type into another client's field, so the
//! emoji travels the way a paste does: it becomes the selection and a virtual
//! keyboard presses the paste shortcut (niri accepts `zwp_virtual_keyboard_v1`
//! from ordinary clients). The previous text selection is put back afterwards.

use std::io::Write as _;
use std::os::fd::AsFd as _;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1;
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;

/// Evdev key codes.
const KEY_V: u32 = 47;
/// XKB modifier bits: Shift and Control.
const MOD_SHIFT: u32 = 1;
const MOD_CONTROL: u32 = 4;
const KEY_RELEASED: u32 = 0;
const KEY_PRESSED: u32 = 1;
const XKB_V1: u32 = 1;

/// The keymap the virtual keyboard declares: the two modifiers and `v`.
const KEYMAP: &str = r#"xkb_keymap {
  xkb_keycodes "rmac" { minimum = 8; maximum = 255; <LCTL> = 37; <LFSH> = 50; <AB04> = 55; };
  xkb_types "rmac" { include "complete" };
  xkb_compat "rmac" { include "complete" };
  xkb_symbols "rmac" {
    key <LCTL> { [ Control_L ] };
    key <LFSH> { [ Shift_L ] };
    key <AB04> { [ v, V ] };
    modifier_map Control { <LCTL> };
    modifier_map Shift { <LFSH> };
  };
};
"#;

/// Terminals take Ctrl+Shift+V; Ctrl+V is a literal ^V there.
pub fn wants_terminal_paste(app_id: Option<&str>) -> bool {
    let Some(app_id) = app_id else {
        return false;
    };
    let app_id = app_id.to_lowercase();
    ["terminal", "kitty", "alacritty", "foot", "wezterm", "konsole", "ptyxis", "xterm"]
        .iter()
        .any(|name| app_id.contains(name))
}

/// What the clipboard held before the emoji went in, so it can come back.
enum Saved {
    Empty,
    Text(String),
    /// Anything but plain text cannot be restored faithfully.
    Other,
}

fn save_clipboard() -> Saved {
    let types = Command::new("wl-paste")
        .arg("--list-types")
        .stderr(Stdio::null())
        .output();
    let Ok(types) = types else {
        return Saved::Other;
    };
    let listing = String::from_utf8_lossy(&types.stdout).into_owned();
    let kinds: Vec<&str> = listing.lines().map(str::trim).filter(|k| !k.is_empty()).collect();
    if kinds.is_empty() {
        return Saved::Empty;
    }
    let text_only = kinds
        .iter()
        .all(|kind| kind.starts_with("text/") || *kind == "UTF8_STRING" || *kind == "STRING" || *kind == "TEXT");
    if !text_only {
        return Saved::Other;
    }
    match Command::new("wl-paste")
        .args(["--no-newline", "--type", "text/plain"])
        .stderr(Stdio::null())
        .output()
    {
        Ok(output) if output.status.success() => {
            Saved::Text(String::from_utf8_lossy(&output.stdout).into_owned())
        }
        _ => Saved::Other,
    }
}

fn set_clipboard(text: &str) -> bool {
    let Ok(mut child) = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let wrote = child
        .stdin
        .take()
        .is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
    // wl-copy forks into the background as the selection's owner and exits
    // here; reap it so it never lingers as a zombie.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    wrote
}

fn clear_clipboard() {
    let _ = Command::new("wl-copy")
        .arg("--clear")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Paste `glyph` into the focused field. Returns whether the paste keys
/// were sent.
pub fn paste(glyph: &str, terminal: bool) -> Result<(), String> {
    let saved = save_clipboard();
    if !set_clipboard(glyph) {
        return Err("wl-copy is not available".to_owned());
    }
    // Let the selection reach the focused client before the key arrives.
    std::thread::sleep(Duration::from_millis(60));
    let sent = press_paste(terminal);
    std::thread::sleep(Duration::from_millis(250));
    match saved {
        Saved::Text(text) => {
            set_clipboard(&text);
        }
        Saved::Empty => clear_clipboard(),
        Saved::Other => {}
    }
    sent
}

struct State;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpVirtualKeyboardManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwpVirtualKeyboardManagerV1,
        _: <ZwpVirtualKeyboardManagerV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpVirtualKeyboardV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwpVirtualKeyboardV1,
        _: <ZwpVirtualKeyboardV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// The keymap as an anonymous file the compositor can map.
fn keymap_file() -> Result<std::fs::File, String> {
    let directory = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(std::env::temp_dir);
    let path = directory.join(format!("rmac-emoji-keymap-{}", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| format!("keymap file: {error}"))?;
    let _ = std::fs::remove_file(&path);
    file.write_all(KEYMAP.as_bytes())
        .and_then(|()| file.write_all(&[0]))
        .map_err(|error| format!("keymap file: {error}"))?;
    Ok(file)
}

fn press_paste(terminal: bool) -> Result<(), String> {
    let connection = Connection::connect_to_env().map_err(|error| error.to_string())?;
    let (globals, mut queue) =
        registry_queue_init::<State>(&connection).map_err(|error| error.to_string())?;
    let qh = queue.handle();
    let seat: wl_seat::WlSeat = globals
        .bind(&qh, 1..=1, ())
        .map_err(|error| format!("no seat: {error}"))?;
    let manager: ZwpVirtualKeyboardManagerV1 = globals
        .bind(&qh, 1..=1, ())
        .map_err(|error| format!("no virtual keyboard: {error}"))?;
    let keyboard = manager.create_virtual_keyboard(&seat, &qh, ());
    let file = keymap_file()?;
    let size = u32::try_from(KEYMAP.len() + 1).map_err(|error| error.to_string())?;
    keyboard.keymap(XKB_V1, file.as_fd(), size);

    let start = Instant::now();
    let time = |offset: u32| u32::try_from(start.elapsed().as_millis()).unwrap_or(0) + offset;
    let modifiers = if terminal {
        MOD_CONTROL | MOD_SHIFT
    } else {
        MOD_CONTROL
    };
    keyboard.modifiers(modifiers, 0, 0, 0);
    keyboard.key(time(1), KEY_V, KEY_PRESSED);
    keyboard.key(time(2), KEY_V, KEY_RELEASED);
    keyboard.modifiers(0, 0, 0, 0);
    keyboard.destroy();
    queue
        .roundtrip(&mut State)
        .map_err(|error| format!("virtual keyboard: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminals_paste_with_shift() {
        assert!(wants_terminal_paste(Some("org.rmac.Terminal")));
        assert!(wants_terminal_paste(Some("Alacritty")));
        assert!(!wants_terminal_paste(Some("org.rmac.TextEditor")));
        assert!(!wants_terminal_paste(None));
    }

    #[test]
    fn the_keymap_declares_only_what_the_paste_needs() {
        assert!(KEYMAP.contains("<AB04> = 55"));
        assert_eq!(KEY_V + 8, 55);
    }
}
