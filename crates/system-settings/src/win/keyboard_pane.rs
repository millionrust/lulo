//! Keyboard on Windows: the shortcuts Lulo apps answer, to read. They are
//! the Mac's, with Ctrl where the Mac has ⌘ (`rmac_ui::shortcuts`), so each
//! one is shown the way it is pressed here.

use gpui::{div, Div, ParentElement as _};
use rmac_ui::StyledExt as _;

use super::form::{card, first_section_header, footnote, section_header, value_row};
use super::WinSettings;

/// (section, [(command, Mac hint)]), in the order the Mac's menus list them.
pub(super) const SECTIONS: [(&str, &[(&str, &str)]); 4] = [
    (
        "Apps and Windows",
        &[
            ("New", "⌘N"),
            ("Open…", "⌘O"),
            ("Save", "⌘S"),
            ("Print…", "⌘P"),
            ("Settings…", "⌘,"),
            ("Close Window", "⌘W"),
            ("Minimise", "⌘M"),
            ("Hide", "⌘H"),
            ("Quit", "⌘Q"),
        ],
    ),
    (
        "Editing Text",
        &[
            ("Undo", "⌘Z"),
            ("Redo", "⇧⌘Z"),
            ("Cut", "⌘X"),
            ("Copy", "⌘C"),
            ("Paste", "⌘V"),
            ("Select All", "⌘A"),
            ("Find", "⌘F"),
            ("Find Next", "⌘G"),
        ],
    ),
    (
        "Menus",
        &[
            ("Open the menu bar", "Alt"),
            ("Move between menus", "←  →"),
            ("Choose a menu command", "↩"),
            ("Close the menu", "Esc"),
        ],
    ),
    (
        "System Settings",
        &[("Back", "⌘["), ("Forward", "⌘]"), ("Search", "⌘F")],
    ),
];

impl WinSettings {
    pub(super) fn render_keyboard(&self) -> Div {
        let mut cards = Vec::new();
        for (index, (section, shortcuts)) in SECTIONS.iter().enumerate() {
            cards.push(if index == 0 {
                first_section_header(*section)
            } else {
                section_header(*section)
            });
            cards.push(card(
                shortcuts
                    .iter()
                    .map(|(command, hint)| {
                        value_row(
                            *command,
                            rmac_ui::shortcuts::display_hint(hint).into_owned(),
                        )
                    })
                    .collect(),
            ));
        }
        cards.push(footnote(
            "Lulo apps use Ctrl where the Mac uses ⌘. Shortcuts that work \
             everywhere in Windows, such as Spotlight's, come with the Lulo \
             shell for Windows.",
        ));
        div().v_flex().children(cards)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_are_shown_with_ctrl() {
        let shown: Vec<String> = SECTIONS[0]
            .1
            .iter()
            .map(|(_, hint)| rmac_ui::shortcuts::display_hint(hint).into_owned())
            .collect();
        assert_eq!(shown[0], "Ctrl+N");
        assert!(shown.iter().all(|hint| !hint.contains('⌘')));
        assert_eq!(
            rmac_ui::shortcuts::display_hint("⇧⌘Z").into_owned(),
            "Ctrl+Shift+Z"
        );
    }

    #[test]
    fn every_command_appears_once_per_section() {
        for (_, shortcuts) in SECTIONS {
            let mut commands: Vec<_> = shortcuts.iter().map(|(command, _)| *command).collect();
            commands.sort_unstable();
            commands.dedup();
            assert_eq!(commands.len(), shortcuts.len());
        }
    }
}
