//! The Windows transport for the menu contract (ADR 0023 phase 3).
//!
//! Windows has no session bus, so the Lulo menu bar (`lulo-shell`) serves a
//! named pipe, [`pipe_name`], and each Lulo app that finds it connects
//! twice, because a synchronous Windows handle serialises a blocking read
//! with any write on the same handle:
//!
//! - the **menus** connection, which the app writes: a [`Message::Hello`]
//!   line, then a [`Message::Menus`] line whenever the bar should have its
//!   current menus (on connecting and after every [`Command::Validate`]);
//! - the **commands** connection, which the app reads after a first
//!   [`Message::Commands`] line: [`Command::Activate`] runs a menu command,
//!   [`Command::Validate`] asks for freshly validated menus, as a D-Bus
//!   reader's `Layout` call does.
//!
//! The bar learns which process is on each end from the pipe itself
//! (`GetNamedPipeClientProcessId`), never from the message.
//!
//! Every message is one line. Labels, shortcuts and actions cannot hold
//! control characters (`validate_menus` rejects them), so the ASCII
//! separators below split the menu tree without any quoting. The menus
//! travel as the same pre-order rows as `org.rmac.AppMenu2` and are read
//! back through the same decoder, with the same limits.

use crate::wire::{self, WireItemV2, WireMenuV2};
use crate::{valid_action, Error, Menu};

/// Between menus.
const MENU_SEPARATOR: char = '\u{1d}';
/// Between a menu's label and its rows, and between rows.
const ROW_SEPARATOR: char = '\u{1e}';
/// Between a row's fields.
const FIELD_SEPARATOR: char = '\u{1f}';
/// A longer line is refused: twelve full menus fit well inside it.
pub const MAX_LINE_BYTES: usize = 256 * 1024;

/// `\\.\pipe\lulo-menubar-<user>`: one per signed-in user. The bar creates
/// it without remote clients, with the default same-user security.
pub fn pipe_name(user: &str) -> String {
    format!(r"\\.\pipe\lulo-menubar-{}", sanitized_user(user))
}

/// `Local\lulo-menubar-<user>`: a manual-reset event the bar sets once its
/// pipe exists and resets when it quits, so an app waits for a bar without
/// polling.
pub fn ready_event_name(user: &str) -> String {
    format!(r"Local\lulo-menubar-{}", sanitized_user(user))
}

fn sanitized_user(user: &str) -> String {
    user.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect()
}

/// What an app writes to the bar.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Message {
    /// The first line of the menus connection.
    Hello { app_id: String },
    /// The first line of the commands connection.
    Commands { app_id: String },
    /// The app's menus as the bar should show them, the bold app menu
    /// first.
    Menus(Vec<Menu>),
}

/// What the bar writes to an app.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    Activate(String),
    Validate,
}

fn valid_app_id(app_id: &str) -> bool {
    !app_id.is_empty()
        && app_id.len() <= 128
        && app_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

/// The line for `message`, newline included.
pub fn encode_message(message: &Message) -> Result<String, Error> {
    match message {
        Message::Hello { app_id } if valid_app_id(app_id) => Ok(format!("hello\t{app_id}\n")),
        Message::Commands { app_id } if valid_app_id(app_id) => Ok(format!("commands\t{app_id}\n")),
        Message::Menus(menus) => {
            crate::validate_menus(menus)?;
            let encoded = wire::encode_v2(menus)
                .into_iter()
                .map(|(label, items)| {
                    let mut menu = label;
                    for (label, action, shortcut, flags, depth) in items {
                        menu.push(ROW_SEPARATOR);
                        menu.push_str(&label);
                        menu.push(FIELD_SEPARATOR);
                        menu.push_str(&action);
                        menu.push(FIELD_SEPARATOR);
                        menu.push_str(&shortcut);
                        menu.push(FIELD_SEPARATOR);
                        menu.push_str(&flags.to_string());
                        menu.push(FIELD_SEPARATOR);
                        menu.push_str(&depth.to_string());
                    }
                    menu
                })
                .collect::<Vec<_>>()
                .join(&MENU_SEPARATOR.to_string());
            let line = format!("menus\t{encoded}\n");
            if line.len() > MAX_LINE_BYTES {
                return Err(Error::Protocol);
            }
            Ok(line)
        }
        _ => Err(Error::Protocol),
    }
}

/// The message on one line (without its newline), validated as strictly as
/// a D-Bus reply.
pub fn decode_message(line: &str) -> Result<Message, Error> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    let (kind, body) = line.split_once('\t').ok_or(Error::Protocol)?;
    match kind {
        "hello" if valid_app_id(body) => Ok(Message::Hello {
            app_id: body.to_owned(),
        }),
        "commands" if valid_app_id(body) => Ok(Message::Commands {
            app_id: body.to_owned(),
        }),
        "menus" => {
            let wire = body
                .split(MENU_SEPARATOR)
                .map(|menu| {
                    let mut rows = menu.split(ROW_SEPARATOR);
                    let label = rows.next().ok_or(Error::Protocol)?.to_owned();
                    let items = rows.map(decode_row).collect::<Result<Vec<_>, _>>()?;
                    Ok((label, items))
                })
                .collect::<Result<Vec<WireMenuV2>, Error>>()?;
            wire::decode_v2(wire).map(Message::Menus)
        }
        _ => Err(Error::Protocol),
    }
}

fn decode_row(row: &str) -> Result<WireItemV2, Error> {
    let fields = row.split(FIELD_SEPARATOR).collect::<Vec<_>>();
    let [label, action, shortcut, flags, depth] = fields[..] else {
        return Err(Error::Protocol);
    };
    Ok((
        label.to_owned(),
        action.to_owned(),
        shortcut.to_owned(),
        flags.parse().map_err(|_| Error::Protocol)?,
        depth.parse().map_err(|_| Error::Protocol)?,
    ))
}

/// The line for `command`, newline included.
pub fn encode_command(command: &Command) -> Result<String, Error> {
    match command {
        Command::Activate(action) if valid_action(action) => Ok(format!("activate\t{action}\n")),
        Command::Activate(_) => Err(Error::Protocol),
        Command::Validate => Ok("validate\n".to_owned()),
    }
}

pub fn decode_command(line: &str) -> Result<Command, Error> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    if line == "validate" {
        return Ok(Command::Validate);
    }
    match line.split_once('\t') {
        Some(("activate", action)) if valid_action(action) => {
            Ok(Command::Activate(action.to_owned()))
        }
        _ => Err(Error::Protocol),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CheckState, Item};

    fn menus() -> Vec<Menu> {
        let mut view = Item::new("View", "calculator::View", "");
        view.children = vec![
            Item::new("Basic", "calculator::ShowBasic", "⌘1").checked(CheckState::On),
            Item::new("Scientific", "calculator::ShowScientific", "⌘2"),
        ];
        vec![
            Menu {
                label: "Calculator".into(),
                items: vec![
                    Item::new("About Calculator", crate::ABOUT_ACTION, ""),
                    Item::new("Quit Calculator", "rmac_ui::QuitApplication", "⌘Q").separated(),
                ],
            },
            Menu {
                label: "View".into(),
                items: vec![
                    view,
                    Item::new("RPN Mode", "calculator::ToggleRpnMode", "⌘R").enabled(false),
                ],
            },
        ]
    }

    #[test]
    fn menus_round_trip_through_one_line() {
        let line = encode_message(&Message::Menus(menus())).unwrap();
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        let decoded = decode_message(line.trim_end_matches('\n')).unwrap();
        assert_eq!(decoded, Message::Menus(menus()));
    }

    #[test]
    fn greetings_round_trip_and_reject_bad_app_ids() {
        for message in [
            Message::Hello {
                app_id: "org.rmac.Calculator".into(),
            },
            Message::Commands {
                app_id: "org.rmac.Notes".into(),
            },
        ] {
            let line = encode_message(&message).unwrap();
            assert_eq!(decode_message(line.trim_end()).unwrap(), message);
        }
        assert!(encode_message(&Message::Hello {
            app_id: "bad\tid".into()
        })
        .is_err());
        assert!(decode_message("hello\t").is_err());
        assert!(decode_message("hello\tC:\\evil").is_err());
    }

    #[test]
    fn commands_round_trip_and_reject_malformed_actions() {
        for command in [
            Command::Activate("calculator::ShowBasic".into()),
            Command::Validate,
        ] {
            let line = encode_command(&command).unwrap();
            assert_eq!(decode_command(line.trim_end()).unwrap(), command);
        }
        assert!(encode_command(&Command::Activate("no namespace".into())).is_err());
        assert!(decode_command("activate\trm -rf").is_err());
        assert!(decode_command("launch\tcalculator::ShowBasic").is_err());
    }

    #[test]
    fn malformed_menu_lines_are_refused() {
        assert!(decode_message("menus\t").is_err());
        assert!(decode_message("menus\tFile\u{1e}Open\u{1f}a::b").is_err());
        assert!(decode_message("menus\tFile\u{1e}Open\u{1f}a::b\u{1f}\u{1f}x\u{1f}0").is_err());
        // A child row with no parent submenu.
        assert!(decode_message("menus\tFile\u{1e}Open\u{1f}a::b\u{1f}\u{1f}1\u{1f}1").is_err());
    }

    #[test]
    fn names_carry_only_safe_user_characters() {
        assert_eq!(
            pipe_name("Ada Lovelace\\x"),
            r"\\.\pipe\lulo-menubar-AdaLovelacex"
        );
        assert_eq!(ready_event_name("ruban"), r"Local\lulo-menubar-ruban");
    }
}
