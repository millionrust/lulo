//! The right-click menu in a document's text, shared by Text Editor, Notes
//! and Mail's compose window, in TextEdit's shape (MROBOT-13): Cut, Copy,
//! Paste and the app's paste variants; then Font, Spelling and Grammar,
//! Substitutions, Transformations and Speech; then Writing Tools.
//!
//! The submenus are not a second copy of the app's commands. Each one
//! mirrors the app's own menu-bar submenu ([`rmac_ui::menu_submenu`]), so
//! a row here dispatches the very action the menu bar row does and shows
//! the tick the app last published for it (Smart Quotes on, Bold on).
//!
//! TextEdit's Layout Orientation and AutoFill submenus are left out: Lulo's
//! text has no vertical layout, and there is no contact card or password
//! store to fill from.

use gpui::{App, Entity, Window};
use rmac_app_menu::{CheckState, Item, Menu};
use rmac_ui::NativeMenu;

/// One of the app's menu-bar submenus to repeat in the text menu.
#[derive(Clone, Copy, Debug)]
pub struct Mirror {
    /// The submenu's own action, such as `"notes::SpeechMenu"`.
    pub action: &'static str,
    /// The rows to keep, by action; empty keeps them all. TextEdit's text
    /// menu shows the standard font panel's rows, not Format ▸ Font's
    /// rich-text extras.
    pub keep: &'static [&'static str],
    /// Shown for read-only text too (Speech reads any text aloud).
    pub read_only: bool,
}

/// Which rows an app's text menu has.
#[derive(Clone, Copy, Debug)]
pub struct TextMenuSpec {
    /// Rows after Paste that also need something on the clipboard:
    /// `(label, action)`.
    pub paste_variants: &'static [(&'static str, &'static str)],
    pub mirrors: &'static [Mirror],
}

const fn mirror(action: &'static str) -> Mirror {
    Mirror {
        action,
        keep: &[],
        read_only: false,
    }
}

const fn speech(action: &'static str) -> Mirror {
    Mirror {
        action,
        keep: &[],
        read_only: true,
    }
}

/// Text Editor: TextEdit's own text menu.
pub const TEXT_EDITOR: TextMenuSpec = TextMenuSpec {
    paste_variants: &[(
        "Paste and Match Style",
        rmac_app_menu::PASTE_MATCH_STYLE_ACTION,
    )],
    mirrors: &[
        Mirror {
            action: "text_editor::FontMenu",
            keep: &[
                "text_editor::ShowFonts",
                "text_editor::ToggleBold",
                "text_editor::ToggleItalic",
                "text_editor::ToggleUnderline",
                "text_editor::IncreaseFont",
                "text_editor::DecreaseFont",
                "text_editor::KernMenu",
                "text_editor::LigaturesMenu",
                "text_editor::BaselineMenu",
                "text_editor::ShowColours",
                "text_editor::CopyStyle",
                "text_editor::PasteStyle",
            ],
            read_only: false,
        },
        mirror("text_editor::SpellingAndGrammarMenu"),
        mirror("text_editor::SubstitutionsMenu"),
        speech("text_editor::SpeechMenu"),
    ],
};

/// Notes: as Notes on the Mac, with Transformations.
pub const NOTES: TextMenuSpec = TextMenuSpec {
    paste_variants: &[("Paste and Match Style", "notes::PastePlainText")],
    mirrors: &[
        mirror("notes::FontMenu"),
        mirror("notes::SpellingAndGrammarMenu"),
        mirror("notes::SubstitutionsMenu"),
        mirror("notes::TransformationsMenu"),
        speech("notes::SpeechMenu"),
    ],
};

/// Mail's compose body: Paste as Quotation, and the Edit submenus Mail
/// answers.
pub const MAIL: TextMenuSpec = TextMenuSpec {
    paste_variants: &[
        ("Paste as Quotation", "mail::PasteAsQuotation"),
        (
            "Paste and Match Style",
            rmac_app_menu::PASTE_MATCH_STYLE_ACTION,
        ),
    ],
    mirrors: &[
        mirror("mail::SpellingAndGrammarMenu"),
        mirror("mail::SubstitutionsMenu"),
        mirror("mail::TransformationsMenu"),
        speech("mail::SpeechMenu"),
    ],
};

/// One row of a text menu, before it becomes a native or a Lulo-drawn
/// menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Command {
        label: String,
        action: String,
        enabled: bool,
        checked: bool,
    },
    Separator,
    Submenu {
        label: String,
        rows: Vec<Row>,
    },
}

/// What the text under the pointer allows right now.
#[derive(Clone, Copy, Debug)]
pub struct TextState {
    pub selected: bool,
    pub editable: bool,
    pub pasteable: bool,
}

/// The rows of `spec`'s text menu, with `lookup` answering each mirrored
/// submenu's live state (see [`rmac_ui::menu_submenu`]).
pub fn rows(
    spec: &TextMenuSpec,
    state: TextState,
    lookup: impl Fn(&str) -> Option<Item>,
) -> Vec<Row> {
    let command = |label: &str, action: &str, enabled: bool| Row::Command {
        label: label.to_owned(),
        action: action.to_owned(),
        enabled,
        checked: false,
    };
    let mut rows = vec![
        command("Cut", "input::Cut", state.editable && state.selected),
        command("Copy", "input::Copy", state.selected),
        command("Paste", "input::Paste", state.pasteable),
    ];
    for (label, action) in spec.paste_variants {
        rows.push(command(label, action, state.pasteable));
    }
    let mut first = true;
    for mirror in spec.mirrors {
        if !(state.editable || mirror.read_only) {
            continue;
        }
        let Some(item) = lookup(mirror.action) else {
            continue;
        };
        let children = kept(item.children, mirror.keep);
        if children.is_empty() {
            continue;
        }
        if first {
            rows.push(Row::Separator);
            first = false;
        }
        rows.push(Row::Submenu {
            label: item.label,
            rows: children.iter().flat_map(item_rows).collect(),
        });
    }
    rows
}

/// `items` without the rows `keep` leaves out (all kept when empty),
/// moving a removed row's separator onto the next row kept.
fn kept(items: Vec<Item>, keep: &[&str]) -> Vec<Item> {
    if keep.is_empty() {
        return items;
    }
    let mut out: Vec<Item> = Vec::new();
    let mut carry = false;
    for mut item in items {
        if !keep.contains(&item.action.as_str()) {
            carry |= item.separator_before;
            continue;
        }
        item.separator_before = (item.separator_before || carry) && !out.is_empty();
        carry = false;
        out.push(item);
    }
    out
}

fn item_rows(item: &Item) -> Vec<Row> {
    let mut rows = Vec::with_capacity(2);
    if item.separator_before {
        rows.push(Row::Separator);
    }
    rows.push(if item.children.is_empty() {
        Row::Command {
            label: item.label.clone(),
            action: item.action.clone(),
            enabled: item.enabled,
            checked: item.checked != CheckState::Off,
        }
    } else {
        Row::Submenu {
            label: item.label.clone(),
            rows: item.children.iter().flat_map(item_rows).collect(),
        }
    });
    rows
}

/// A mirrored submenu from a static menu table, for tests and for apps
/// that publish no live menu state.
pub fn find_submenu(menus: &[Menu], action: &str) -> Option<Item> {
    fn find(items: &[Item], action: &str) -> Option<Item> {
        items.iter().find_map(|item| {
            if item.action == action && !item.children.is_empty() {
                Some(item.clone())
            } else {
                find(&item.children, action)
            }
        })
    }
    menus.iter().find_map(|menu| find(&menu.items, action))
}

fn live_rows(spec: &TextMenuSpec, state: TextState, cx: &App) -> Vec<Row> {
    rows(spec, state, |action| rmac_ui::menu_submenu(action, cx))
}

fn native(mut menu: NativeMenu, rows: &[Row], cx: &App) -> NativeMenu {
    for row in rows {
        menu = match row {
            Row::Separator => menu.separator(),
            Row::Submenu { label, rows } => {
                menu.submenu(label.clone(), native(NativeMenu::new(), rows, cx))
            }
            Row::Command {
                label,
                action,
                enabled,
                checked,
            } => {
                let Ok(built) = cx.build_action(action, None) else {
                    continue;
                };
                if !enabled {
                    menu.menu_with_disabled(label.clone(), true, built)
                } else if *checked {
                    menu.menu_with_check(label.clone(), true, built)
                } else {
                    menu.menu(label.clone(), built)
                }
            }
        };
    }
    menu
}

fn drawn(rows: &[Row], position: gpui::Point<gpui::Pixels>, cx: &App) -> rmac_ui::ContextMenu {
    let mut menu = rmac_ui::ContextMenu::new(position);
    for row in rows {
        menu = match row {
            Row::Separator => menu.separator(),
            Row::Submenu { label, rows } => menu.submenu(label.clone(), drawn(rows, position, cx)),
            Row::Command {
                label,
                action,
                enabled,
                checked,
            } => {
                let Ok(built) = cx.build_action(action, None) else {
                    continue;
                };
                if !enabled {
                    menu.disabled_item(label.clone(), built)
                } else if *checked {
                    menu.checked_item(label.clone(), rmac_ui::MenuCheck::On, built)
                } else {
                    menu.item(label.clone(), built)
                }
            }
        };
    }
    menu
}

/// A text field's right-click menu: `spec`'s rows, then Writing Tools while
/// it is available and the field is editable. The builder runs after the
/// field's own update (MROBOT-01), so reading the field here is safe.
pub fn text_menu(
    field: Entity<rmac_ui::InputState>,
    editable: bool,
    spec: &'static TextMenuSpec,
) -> rmac_ui::TextMenuBuilder {
    std::rc::Rc::new(
        move |menu: NativeMenu, _window: &mut Window, cx: &mut App| {
            let state = TextState {
                selected: !field.read(cx).selected_range().is_empty(),
                editable,
                pasteable: editable && cx.read_from_clipboard().is_some(),
            };
            let menu = native(menu, &live_rows(spec, state, cx), cx);
            crate::append_native_writing_tools(menu, editable, cx)
        },
    )
}

/// The same menu for a Lulo-drawn [`rmac_ui::ContextMenu`] (Text Editor's
/// rich-text body), Writing Tools included.
pub fn context_menu(
    spec: &TextMenuSpec,
    state: TextState,
    position: gpui::Point<gpui::Pixels>,
    cx: &App,
) -> rmac_ui::ContextMenu {
    let menu = drawn(&live_rows(spec, state, cx), position, cx);
    crate::context_menu_rows(menu, position, state.editable, cx)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                Row::Command { label, .. } => label.clone(),
                Row::Separator => "-".to_owned(),
                Row::Submenu { label, .. } => format!("{label} >"),
            })
            .collect()
    }

    fn submenu<'a>(rows: &'a [Row], name: &str) -> &'a [Row] {
        rows.iter()
            .find_map(|row| match row {
                Row::Submenu { label, rows } if label == name => Some(rows.as_slice()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no {name} submenu"))
    }

    fn app_rows(app_id: &str, spec: &TextMenuSpec, state: TextState) -> Vec<Row> {
        let menus = rmac_app_menu::static_definition(app_id).expect("menu table");
        rows(spec, state, |action| find_submenu(&menus, action))
    }

    const EDITING: TextState = TextState {
        selected: true,
        editable: true,
        pasteable: true,
    };

    #[test]
    fn text_editor_matches_textedit() {
        let rows = app_rows(rmac_apps::identity::TEXT_EDITOR, &TEXT_EDITOR, EDITING);
        assert_eq!(
            labels(&rows),
            [
                "Cut",
                "Copy",
                "Paste",
                "Paste and Match Style",
                "-",
                "Font >",
                "Spelling and Grammar >",
                "Substitutions >",
                "Speech >",
            ]
        );
        let font = labels(submenu(&rows, "Font"));
        assert_eq!(font.first().map(String::as_str), Some("Show Fonts"));
        for wanted in ["Bold", "Italic", "Underline", "Bigger", "Smaller", "Kern >"] {
            assert!(
                font.iter().any(|label| label == wanted),
                "{wanted} in {font:?}"
            );
        }
        for dropped in ["Outline", "Styles…", "Highlight >"] {
            assert!(
                !font.iter().any(|label| label == dropped),
                "{dropped} in {font:?}"
            );
        }
        assert_ne!(font.first().map(String::as_str), Some("-"));
        assert_eq!(
            labels(submenu(&rows, "Speech")),
            ["Start Speaking", "Stop Speaking"]
        );
    }

    #[test]
    fn rows_run_the_menu_bar_actions() {
        let rows = app_rows(rmac_apps::identity::TEXT_EDITOR, &TEXT_EDITOR, EDITING);
        let substitutions = submenu(&rows, "Substitutions");
        assert!(substitutions.iter().any(|row| matches!(row,
            Row::Command { label, action, .. }
                if label == "Smart Quotes" && action == "text_editor::ToggleSmartQuotes")));
    }

    #[test]
    fn read_only_text_keeps_only_speech() {
        let rows = app_rows(
            rmac_apps::identity::TEXT_EDITOR,
            &TEXT_EDITOR,
            TextState {
                selected: true,
                editable: false,
                pasteable: false,
            },
        );
        assert_eq!(
            labels(&rows),
            [
                "Cut",
                "Copy",
                "Paste",
                "Paste and Match Style",
                "-",
                "Speech >"
            ]
        );
        assert!(matches!(&rows[0], Row::Command { enabled: false, .. }));
        assert!(matches!(&rows[1], Row::Command { enabled: true, .. }));
    }

    #[test]
    fn notes_and_mail_rows() {
        let notes = app_rows(rmac_apps::identity::NOTES, &NOTES, EDITING);
        assert_eq!(
            labels(&notes),
            [
                "Cut",
                "Copy",
                "Paste",
                "Paste and Match Style",
                "-",
                "Font >",
                "Spelling and Grammar >",
                "Substitutions >",
                "Transformations >",
                "Speech >",
            ]
        );
        let mail = app_rows(rmac_apps::identity::MAIL, &MAIL, EDITING);
        let mail = labels(&mail);
        assert_eq!(
            &mail[..6],
            [
                "Cut",
                "Copy",
                "Paste",
                "Paste as Quotation",
                "Paste and Match Style",
                "-"
            ]
        );
        assert!(mail.contains(&"Speech >".to_owned()));
        assert!(mail.contains(&"Transformations >".to_owned()));
    }

    #[test]
    fn checkmarks_follow_the_published_state() {
        let menus = rmac_app_menu::static_definition(rmac_apps::identity::TEXT_EDITOR).unwrap();
        let menus = rmac_app_menu::apply_state(&menus, |item| {
            (item.action == "text_editor::ToggleSmartQuotes").then(|| rmac_app_menu::ItemState {
                checked: Some(CheckState::On),
                ..Default::default()
            })
        });
        let rows = rows(&TEXT_EDITOR, EDITING, |action| find_submenu(&menus, action));
        assert!(submenu(&rows, "Substitutions")
            .iter()
            .any(|row| matches!(row,
            Row::Command { label, checked: true, .. } if label == "Smart Quotes")));
    }
}
