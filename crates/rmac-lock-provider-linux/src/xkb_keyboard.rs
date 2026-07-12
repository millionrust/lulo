//! Linux XKB keymap and modifier decoder for the future Wayland lock surface.

use std::env;
use std::ffi::OsString;
use std::fmt;
use std::os::fd::OwnedFd;

use xkbcommon::xkb::{self, compose};

use crate::keyboard::{DecodedKey, DecodedText};

const WAYLAND_KEYCODE_OFFSET: u32 = 8;
const MAX_KEYMAP_BYTES: u32 = 16 * 1024 * 1024;

pub(super) struct KeyboardDecoder {
    context: xkb::Context,
    state: Option<xkb::State>,
    compose: Option<compose::State>,
}

impl KeyboardDecoder {
    pub(super) fn new() -> Self {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let compose = compose_locale()
            .and_then(|locale| {
                compose::Table::new_from_locale(&context, &locale, compose::COMPILE_NO_FLAGS).ok()
            })
            .map(|table| compose::State::new(&table, compose::STATE_NO_FLAGS));
        Self {
            context,
            state: None,
            compose,
        }
    }

    pub(super) fn install_keymap(&mut self, fd: OwnedFd, size: u32) -> Result<(), Error> {
        if !(2..=MAX_KEYMAP_BYTES).contains(&size) {
            return Err(Error::InvalidKeymapSize);
        }
        let size = usize::try_from(size).map_err(|_| Error::InvalidKeymapSize)?;
        // SAFETY: Wayland transfers ownership of a valid keymap descriptor.
        // Size is nonzero and bounded before xkbcommon privately maps it.
        let keymap = unsafe {
            xkb::Keymap::new_from_fd(
                &self.context,
                fd,
                size,
                xkb::KEYMAP_FORMAT_TEXT_V1,
                xkb::KEYMAP_COMPILE_NO_FLAGS,
            )
        }
        .map_err(|_| Error::MapKeymap)?
        .ok_or(Error::CompileKeymap)?;
        self.state = Some(xkb::State::new(&keymap));
        if let Some(compose) = &mut self.compose {
            compose.reset();
        }
        Ok(())
    }

    pub(super) fn clear_keymap(&mut self) {
        self.state = None;
        if let Some(compose) = &mut self.compose {
            compose.reset();
        }
    }

    pub(super) fn is_ready(&self) -> bool {
        self.state.is_some()
    }

    #[cfg(test)]
    fn install_test_layout(&mut self, layout: &str) -> Result<(), Error> {
        let keymap = xkb::Keymap::new_from_names(
            &self.context,
            "",
            "",
            layout,
            "",
            None,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .ok_or(Error::CompileKeymap)?;
        self.state = Some(xkb::State::new(&keymap));
        Ok(())
    }

    pub(super) fn update_modifiers(
        &mut self,
        depressed: u32,
        latched: u32,
        locked: u32,
        group: u32,
    ) -> Result<(), Error> {
        let state = self.state.as_mut().ok_or(Error::KeymapUnavailable)?;
        state.update_mask(depressed, latched, locked, 0, 0, group);
        Ok(())
    }

    pub(super) fn decode_press(&mut self, raw_keycode: u32) -> Result<Option<DecodedKey>, Error> {
        let keycode = wayland_keycode(raw_keycode)?;
        let state = self.state.as_ref().ok_or(Error::KeymapUnavailable)?;
        let keysym = state.key_get_one_sym(keycode);
        let raw_keysym = keysym.raw();

        let action = match raw_keysym {
            xkb::keysyms::KEY_BackSpace => {
                self.reset_compose();
                Some(DecodedKey::Backspace)
            }
            xkb::keysyms::KEY_Return | xkb::keysyms::KEY_KP_Enter | xkb::keysyms::KEY_ISO_Enter => {
                self.reset_compose();
                Some(DecodedKey::Submit)
            }
            xkb::keysyms::KEY_Escape => {
                self.reset_compose();
                Some(DecodedKey::Cancel)
            }
            xkb::keysyms::KEY_Left | xkb::keysyms::KEY_Up => Some(DecodedKey::SelectPrevious),
            xkb::keysyms::KEY_Right | xkb::keysyms::KEY_Down => Some(DecodedKey::SelectNext),
            _ => self.decode_text(keysym, keycode)?,
        };
        Ok(action)
    }

    pub(super) fn key_repeats(&self, raw_keycode: u32) -> Result<bool, Error> {
        let keycode = wayland_keycode(raw_keycode)?;
        let state = self.state.as_ref().ok_or(Error::KeymapUnavailable)?;
        Ok(state.get_keymap().key_repeats(keycode))
    }

    fn decode_text(
        &mut self,
        keysym: xkb::Keysym,
        keycode: xkb::Keycode,
    ) -> Result<Option<DecodedKey>, Error> {
        if let Some(compose) = &mut self.compose {
            if compose.feed(keysym) == compose::FeedResult::Accepted {
                match compose.status() {
                    compose::Status::Composing => return Ok(None),
                    compose::Status::Composed => {
                        let value = compose.utf8();
                        compose.reset();
                        return Ok(value.and_then(decoded_text));
                    }
                    compose::Status::Cancelled => {
                        compose.reset();
                        return Ok(None);
                    }
                    compose::Status::Nothing => {}
                }
            }
        }

        let value = self
            .state
            .as_ref()
            .ok_or(Error::KeymapUnavailable)?
            .key_get_utf8(keycode);
        Ok(decoded_text(value))
    }

    pub(super) fn reset_compose(&mut self) {
        if let Some(compose) = &mut self.compose {
            compose.reset();
        }
    }
}

fn wayland_keycode(raw_keycode: u32) -> Result<xkb::Keycode, Error> {
    raw_keycode
        .checked_add(WAYLAND_KEYCODE_OFFSET)
        .filter(|keycode| *keycode <= xkb::KEYCODE_MAX)
        .map(Into::into)
        .ok_or(Error::InvalidKeycode)
}

fn decoded_text(value: String) -> Option<DecodedKey> {
    DecodedText::new(value).ok().map(DecodedKey::Text)
}

impl fmt::Debug for KeyboardDecoder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("KeyboardDecoder(<redacted>)")
    }
}

fn compose_locale() -> Option<OsString> {
    ["LC_ALL", "LC_CTYPE", "LANG"]
        .into_iter()
        .find_map(|name| env::var_os(name).filter(|value| !value.is_empty()))
        .or_else(|| Some(OsString::from("C")))
}

#[derive(Debug)]
pub(super) enum Error {
    InvalidKeymapSize,
    MapKeymap,
    CompileKeymap,
    KeymapUnavailable,
    InvalidKeycode,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    #[test]
    fn decodes_us_text_space_and_lock_actions() {
        let mut decoder = KeyboardDecoder::new();
        decoder.install_test_layout("us").unwrap();

        let Some(DecodedKey::Text(letter)) = decoder.decode_press(30).unwrap() else {
            panic!("evdev A did not decode as text");
        };
        letter.expose(|value| assert_eq!(value, "a"));
        assert!(matches!(decoder.key_repeats(30), Ok(true)));

        let Some(DecodedKey::Text(space)) = decoder.decode_press(57).unwrap() else {
            panic!("space must remain valid credential text");
        };
        space.expose(|value| assert_eq!(value, " "));

        assert!(matches!(
            decoder.decode_press(14),
            Ok(Some(DecodedKey::Backspace))
        ));
        assert!(matches!(
            decoder.decode_press(28),
            Ok(Some(DecodedKey::Submit))
        ));
        assert!(matches!(
            decoder.decode_press(1),
            Ok(Some(DecodedKey::Cancel))
        ));
    }

    #[test]
    fn rejects_unusable_keymap_sizes_before_mapping() {
        let mut decoder = KeyboardDecoder::new();
        let fd = OwnedFd::from(File::open("/dev/null").unwrap());
        assert!(matches!(
            decoder.install_keymap(fd, 0),
            Err(Error::InvalidKeymapSize)
        ));

        let fd = OwnedFd::from(File::open("/dev/null").unwrap());
        assert!(matches!(
            decoder.install_keymap(fd, MAX_KEYMAP_BYTES + 1),
            Err(Error::InvalidKeymapSize)
        ));
    }

    #[test]
    fn rejects_key_events_until_a_keymap_is_ready() {
        let mut decoder = KeyboardDecoder::new();
        assert!(matches!(
            decoder.decode_press(30),
            Err(Error::KeymapUnavailable)
        ));
        assert!(matches!(
            decoder.decode_press(u32::MAX),
            Err(Error::InvalidKeycode)
        ));
        assert_eq!(format!("{decoder:?}"), "KeyboardDecoder(<redacted>)");
    }
}
