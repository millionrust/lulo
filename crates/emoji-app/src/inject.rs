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

use self::vk::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1;
use self::vk::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;

/// The virtual-keyboard protocol, generated from `protocols/*.xml`.
mod vk {
    #![allow(
        missing_docs,
        non_upper_case_globals,
        non_camel_case_types,
        clippy::all
    )]

    use wayland_client::protocol::*;
    use wayland_client::{self};

    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/virtual-keyboard-unstable-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("protocols/virtual-keyboard-unstable-v1.xml");
}

/// Evdev key codes.
const KEY_V: u32 = 47;
const KEY_LEFTCTRL: u32 = 29;
const KEY_LEFTSHIFT: u32 = 42;
const KEY_LEFTMETA: u32 = 125;
/// XKB modifier bits: Shift, Control and Mod4 (Super, the ⌘ key).
const MOD_SHIFT: u32 = 1;
const MOD_CONTROL: u32 = 4;
const MOD_SUPER: u32 = 64;
const KEY_RELEASED: u32 = 0;
const KEY_PRESSED: u32 = 1;
const XKB_V1: u32 = 1;

/// The default US keymap, as text, from libxkbcommon: the same one a real
/// keyboard gets, so the focused client reads the chord exactly as typed.
fn keymap_text() -> Result<String, String> {
    use xkbcommon::xkb;
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap = xkb::Keymap::new_from_names(
        &context,
        "",
        "",
        "us",
        "",
        None,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .ok_or_else(|| "could not build a keymap".to_owned())?;
    Ok(keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1))
}

/// The paste shortcut the focused app listens for. keyd turns ⌘V into Ctrl+V
/// for other apps, but a virtual keyboard bypasses keyd, so the chord has to
/// be the one the app itself sees.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Paste {
    /// ⌘V: Lulo's own apps, which take Super as ⌘.
    Command,
    /// Ctrl+V: GTK, Qt and the browsers.
    Control,
    /// Ctrl+Shift+V: other terminals, where Ctrl+V is a literal ^V.
    ControlShift,
}

impl Paste {
    pub fn for_app(app_id: Option<&str>) -> Self {
        let Some(app_id) = app_id else {
            return Self::Control;
        };
        if app_id.starts_with("org.rmac.") {
            return Self::Command;
        }
        let app_id = app_id.to_lowercase();
        let terminal = [
            "terminal",
            "kitty",
            "alacritty",
            "foot",
            "wezterm",
            "konsole",
            "ptyxis",
            "xterm",
        ]
        .iter()
        .any(|name| app_id.contains(name));
        if terminal {
            Self::ControlShift
        } else {
            Self::Control
        }
    }

    fn modifiers(self) -> u32 {
        match self {
            Self::Command => MOD_SUPER,
            Self::Control => MOD_CONTROL,
            Self::ControlShift => MOD_CONTROL | MOD_SHIFT,
        }
    }
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
    let kinds: Vec<&str> = listing
        .lines()
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .collect();
    if kinds.is_empty() {
        return Saved::Empty;
    }
    let text_only = kinds.iter().all(|kind| {
        kind.starts_with("text/") || *kind == "UTF8_STRING" || *kind == "STRING" || *kind == "TEXT"
    });
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
pub fn paste(glyph: &str, chord: Paste) -> Result<(), String> {
    let saved = save_clipboard();
    if !set_clipboard(glyph) {
        return Err("wl-copy is not available".to_owned());
    }
    // Let the selection reach the focused client before the key arrives.
    std::thread::sleep(Duration::from_millis(60));
    let sent = press_paste(chord);
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
fn keymap_file(text: &str) -> Result<std::fs::File, String> {
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
    file.write_all(text.as_bytes())
        .and_then(|()| file.write_all(&[0]))
        .map_err(|error| format!("keymap file: {error}"))?;
    Ok(file)
}

fn press_paste(chord: Paste) -> Result<(), String> {
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
    let text = keymap_text()?;
    let file = keymap_file(&text)?;
    let size = u32::try_from(text.len() + 1).map_err(|error| error.to_string())?;
    keyboard.keymap(XKB_V1, file.as_fd(), size);

    let start = Instant::now();
    let time = || u32::try_from(start.elapsed().as_millis()).unwrap_or(0);
    // Like a real keyboard: each modifier key goes down (and the modifier
    // state follows), then V, then everything comes back up.
    let held: &[(u32, u32)] = match chord {
        Paste::Command => &[(KEY_LEFTMETA, MOD_SUPER)],
        Paste::Control => &[(KEY_LEFTCTRL, MOD_CONTROL)],
        Paste::ControlShift => &[(KEY_LEFTCTRL, MOD_CONTROL), (KEY_LEFTSHIFT, MOD_SHIFT)],
    };
    let mut mask = 0;
    for &(key, bit) in held {
        keyboard.key(time(), key, KEY_PRESSED);
        mask |= bit;
        keyboard.modifiers(mask, 0, 0, 0);
    }
    keyboard.key(time(), KEY_V, KEY_PRESSED);
    queue
        .roundtrip(&mut State)
        .map_err(|error| format!("virtual keyboard: {error}"))?;
    std::thread::sleep(Duration::from_millis(30));
    keyboard.key(time(), KEY_V, KEY_RELEASED);
    for &(key, bit) in held.iter().rev() {
        keyboard.key(time(), key, KEY_RELEASED);
        mask &= !bit;
        keyboard.modifiers(mask, 0, 0, 0);
    }
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
    fn the_paste_chord_is_the_one_the_focused_app_sees() {
        assert_eq!(Paste::for_app(Some("org.rmac.Terminal")), Paste::Command);
        assert_eq!(Paste::for_app(Some("org.rmac.TextEditor")), Paste::Command);
        assert_eq!(Paste::for_app(Some("Alacritty")), Paste::ControlShift);
        assert_eq!(Paste::for_app(Some("org.gnome.TextEditor")), Paste::Control);
        assert_eq!(Paste::for_app(Some("firefox")), Paste::Control);
        assert_eq!(Paste::for_app(None), Paste::Control);
    }

    #[test]
    fn the_paste_keys_are_evdev_codes() {
        assert_eq!(KEY_V, 47);
        assert_eq!(KEY_LEFTMETA, 125);
        assert_eq!(MOD_SUPER, 1 << 6);
    }
}
