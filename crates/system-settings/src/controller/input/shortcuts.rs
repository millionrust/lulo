//! Keyboard › Keyboard Shortcuts…: the Mac's 670 × 468 sheet with a
//! category sidebar and the shortcut list (design-lab/settings.html). It
//! lists the global shortcuts rmac registers with the session shortcut
//! broker; changing one opens the portal's own configuration UI, the only
//! authority that may rebind them.

use super::*;

/// Sheet categories: the Mac's name, its sidebar icon and colour, the rmac
/// shortcut id and the Mac's wording for the action.
const SHORTCUT_CATEGORIES: [(&str, &str, u32, &str, &str); 2] = [
    (
        "Spotlight",
        "icons/search.svg",
        0x1372f9,
        "launcher",
        "Show Spotlight search",
    ),
    (
        "Lock Screen",
        "icons/lock.svg",
        0x1d1d1f,
        "lock",
        "Lock Screen",
    ),
];

const SHEET_WIDTH: f32 = 670.0;
const SHEET_HEIGHT: f32 = 468.0;

/// "Mod+Ctrl+Q" / "LOGO+CTRL+q" as the Mac writes it: ⌃⌥⇧⌘ then the key.
fn mac_trigger(trigger: &str) -> String {
    let mut control = false;
    let mut option = false;
    let mut shift = false;
    let mut command = false;
    let mut key = String::new();
    for part in trigger
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => control = true,
            "alt" | "option" => option = true,
            "shift" => shift = true,
            "mod" | "super" | "logo" | "cmd" | "command" => command = true,
            "space" => key = "Space".to_owned(),
            other if other.chars().count() == 1 => key = other.to_uppercase(),
            _ => key = part.to_owned(),
        }
    }
    let mut out = String::new();
    if control {
        out.push('⌃');
    }
    if option {
        out.push('⌥');
    }
    if shift {
        out.push('⇧');
    }
    if command {
        out.push('⌘');
    }
    out.push_str(&key);
    out
}

impl Settings {
    pub(in crate::controller) fn open_keyboard_shortcuts(&mut self, cx: &mut Context<Self>) {
        self.keyboard_shortcuts_open = true;
        self.keyboard_shortcuts_category = 0;
        if self.shortcut_status.is_none() {
            self.refresh_shortcut_status(cx);
        }
        cx.notify();
    }

    pub(in crate::controller) fn close_keyboard_shortcuts(&mut self, cx: &mut Context<Self>) {
        self.keyboard_shortcuts_open = false;
        cx.notify();
    }

    /// The key combination the active backend binds for `spec`: the niri
    /// fallback's own trigger, or the trigger rmac asked the portal for.
    /// None while the broker has not reported which backend is active.
    fn active_trigger(&self, spec: &rmac_shortcuts::ShortcutSpec) -> Option<String> {
        match self.shortcut_status.as_ref()? {
            rmac_shortcuts::BackendStatus::FallbackRequired { .. } => {
                Some(mac_trigger(&spec.niri_trigger))
            }
            rmac_shortcuts::BackendStatus::Portal { .. } => {
                Some(mac_trigger(&spec.preferred_trigger))
            }
        }
    }

    pub(in crate::controller) fn render_keyboard_shortcuts_sheet(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        if !self.keyboard_shortcuts_open {
            return None;
        }
        let view = cx.entity();
        let selected = self
            .keyboard_shortcuts_category
            .min(SHORTCUT_CATEGORIES.len() - 1);
        let sidebar = div()
            .v_flex()
            .children(SHORTCUT_CATEGORIES.iter().enumerate().map(
                |(index, (name, icon, color, _, _))| {
                    let select_view = view.clone();
                    sheet_sidebar_row(
                        SharedString::from(format!("keyboard-shortcuts-category-{index}")),
                        tile(icon, hsl(*color), style::SIDEBAR_ICON).into_any_element(),
                        *name,
                        index == selected,
                        move |_, cx| {
                            select_view.update(cx, |settings, cx| {
                                settings.keyboard_shortcuts_category = index;
                                cx.notify();
                            });
                        },
                    )
                },
            ));

        let (_, _, _, shortcut_id, action) = SHORTCUT_CATEGORIES[selected];
        let configurable = shortcut_configuration_available(self.shortcut_status.as_ref())
            && !self.shortcut_configuration_busy;
        let spec = rmac_shortcuts::default_shortcuts()
            .into_iter()
            .find(|spec| spec.id.0 == shortcut_id);
        let keys = spec.as_ref().and_then(|spec| self.active_trigger(spec));
        let configure_view = view.clone();
        let keys_element = div()
            .id("keyboard-shortcut-keys")
            .text_size(rmac_ui::text_px(13.0))
            .text_color(label())
            .when(configurable, |keys| {
                keys.cursor_pointer().on_click(move |_, _, cx| {
                    configure_view.update(cx, |settings, cx| {
                        settings.configure_global_shortcuts(cx);
                    });
                })
            })
            .child(keys.unwrap_or_default());
        let mut list = group();
        if configurable {
            list = list
                .child(
                    div()
                        .px(px(style::ROW_PADDING))
                        .py(px(style::ROW_PADDING))
                        .text_size(rmac_ui::text_px(13.0))
                        .line_height(px(16.0))
                        .text_color(label())
                        .child("To change a shortcut, click the key combination."),
                )
                .child(row_separator());
        }
        list = list.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .h(px(style::WELL_ROW_HEIGHT))
                .px(px(style::ROW_PADDING))
                .my(px(8.0))
                .child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(label())
                        .child(action),
                )
                .child(keys_element),
        );
        let mut body = div().v_flex().child(list);
        if let Some(error) = &self.shortcut_configuration_error {
            body = body.child(note_card(error.clone()));
        }
        if let Some(error) = &self.shortcut_status_error {
            body = body.child(note_card(error.clone()));
        }

        let done_view = view.clone();
        Some(
            settings_sheet(
                "keyboard-shortcuts-sheet",
                SHEET_WIDTH,
                SHEET_HEIGHT,
                Some(sidebar.into_any_element()),
                body.into_any_element(),
                vec![sheet_default_button(
                    "keyboard-shortcuts-done",
                    "Done",
                    move |_, cx| {
                        done_view.update(cx, |settings, cx| settings.close_keyboard_shortcuts(cx));
                    },
                )],
            )
            .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::mac_trigger;

    #[test]
    fn triggers_read_like_the_mac() {
        assert_eq!(mac_trigger("Mod+Space"), "⌘Space");
        assert_eq!(mac_trigger("Mod+Ctrl+Q"), "⌃⌘Q");
        assert_eq!(mac_trigger("LOGO+CTRL+q"), "⌃⌘Q");
        assert_eq!(mac_trigger("LOGO+space"), "⌘Space");
    }
}
