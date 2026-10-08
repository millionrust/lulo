//! Displays on Windows: each connected display's resolution, refresh rate
//! and scale, read from Windows when the pane opens and again whenever the
//! window comes forward. Windows owns these settings; Settings shows them
//! and opens Windows' own Display settings to change them.

use gpui::{div, Context, Div, IntoElement as _, ParentElement as _};
use rmac_ui::StyledExt as _;

use super::form::{
    card, first_section_header, footer_buttons, note_card, push_button, section_header, value_row,
};
use super::host::{looks_like_label, resolution_label};
use super::WinSettings;

/// Open a page of Windows' own Settings app, off the UI thread.
pub(super) fn open_windows_settings(page: &'static str, cx: &mut gpui::App) {
    // Child processes start from the blocking pool, not an executor thread
    // (LINUX-HW-07, scripts/check-background-executor-command.sh).
    cx.background_executor()
        .spawn(blocking::unblock(move || {
            if let Err(error) = std::process::Command::new("explorer.exe").arg(page).spawn() {
                eprintln!("rmac-system-settings: could not open {page}: {error}");
            }
        }))
        .detach();
}

impl WinSettings {
    pub(super) fn render_displays(&self, _cx: &mut Context<Self>) -> Div {
        let mut cards = Vec::new();
        match &self.displays {
            None => cards.push(note_card("Reading the connected displays…")),
            Some(Err(error)) => cards.push(note_card(error.clone())),
            Some(Ok(displays)) if displays.is_empty() => {
                cards.push(note_card("Windows reports no connected display."))
            }
            Some(Ok(displays)) => {
                for (index, display) in displays.iter().enumerate() {
                    let heading = if displays.len() > 1 && display.primary {
                        format!("{} (Main Display)", display.name)
                    } else {
                        display.name.clone()
                    };
                    cards.push(if index == 0 {
                        first_section_header(heading)
                    } else {
                        section_header(heading)
                    });
                    cards.push(card(vec![
                        value_row("Resolution", resolution_label(display)),
                        value_row("Scale", format!("{}%", display.scale_percent)),
                        value_row("Desktop size", looks_like_label(display)),
                    ]));
                }
            }
        }
        cards.push(footer_buttons(vec![push_button(
            "displays-open-windows-settings",
            "Display Settings…",
        )
        .on_click(|_, _, cx| open_windows_settings("ms-settings:display", cx))
        .into_any_element()]));
        div().v_flex().children(cards)
    }
}
