use gpui::{
    Action, AsKeystroke, FocusHandle, Half, IntoElement, KeyContext, Keystroke, ParentElement as _,
    RenderOnce, StyleRefinement, Styled, Window, div, prelude::FluentBuilder as _, relative,
};

use crate::{ActiveTheme, StyledExt};

/// A tag for displaying keyboard keybindings.
#[derive(IntoElement, Clone, Debug)]
pub struct Kbd {
    style: StyleRefinement,
    stroke: Keystroke,
    appearance: bool,
    outline: bool,
}

impl From<Keystroke> for Kbd {
    fn from(stroke: Keystroke) -> Self {
        Self {
            style: StyleRefinement::default(),
            stroke,
            appearance: true,
            outline: false,
        }
    }
}

impl Kbd {
    /// Create a new Kbd element with the given [`Keystroke`].
    pub fn new(stroke: Keystroke) -> Self {
        Self {
            style: StyleRefinement::default(),
            stroke,
            appearance: true,
            outline: false,
        }
    }

    /// Set the appearance of the keybinding, default is `true`.
    pub fn appearance(mut self, appearance: bool) -> Self {
        self.appearance = appearance;
        self
    }

    /// Use outline style for the keybinding, default is `false`.
    pub fn outline(mut self) -> Self {
        self.outline = true;
        self
    }

    /// Return the first keybinding for the given action and context.
    pub fn binding_for_action(
        action: &dyn Action,
        context: Option<&str>,
        window: &Window,
    ) -> Option<Self> {
        let key_context = context.and_then(|context| KeyContext::parse(context).ok());
        let binding = match key_context {
            Some(context) => {
                window.highest_precedence_binding_for_action_in_context(action, context)
            }
            None => window.highest_precedence_binding_for_action(action),
        }?;

        if let Some(key) = binding.keystrokes().first() {
            Some(Self::new(key.as_keystroke().clone()))
        } else {
            None
        }
    }

    /// Return the first keybinding for the given action and focus handle.
    pub fn binding_for_action_in(
        action: &dyn Action,
        focus_handle: &FocusHandle,
        window: &Window,
    ) -> Option<Self> {
        let binding = window.highest_precedence_binding_for_action_in(action, focus_handle)?;
        if let Some(key) = binding.keystrokes().first() {
            Some(Self::new(key.as_keystroke().clone()))
        } else {
            None
        }
    }

    /// Return the keybinding string for a KeyStroke in macOS glyphs.
    ///
    /// rmac: Lulo shows Mac shortcut glyphs on every platform (⌘ is Super),
    /// so menus read "⌘⌫" rather than upstream's Linux "Win+Backspace".
    /// macOS: https://support.apple.com/en-us/HT201236
    pub fn format(key: &Keystroke) -> String {
        const SEPARATOR: &str = "";

        let mut parts = vec![];

        // The key map order in macOS is: ⌃⌥⇧⌘
        if key.modifiers.control {
            parts.push("⌃");
        }

        if key.modifiers.alt {
            parts.push("⌥");
        }

        if key.modifiers.shift {
            parts.push("⇧");
        }

        if key.modifiers.platform {
            parts.push("⌘");
        }

        let mut keys = String::new();
        let key_str = key.key.as_str();
        match key_str {
            "ctrl" => keys.push('⌃'),
            "alt" => keys.push('⌥'),
            "shift" => keys.push('⇧'),
            "cmd" => keys.push('⌘'),
            "space" => keys.push_str("Space"),
            "backspace" => keys.push('⌫'),
            "delete" => keys.push('⌫'),
            "escape" => keys.push('⎋'),
            "enter" => keys.push('⏎'),
            "pagedown" => keys.push_str("Page Down"),
            "pageup" => keys.push_str("Page Up"),
            "left" => keys.push('←'),
            "right" => keys.push('→'),
            "up" => keys.push('↑'),
            "down" => keys.push('↓'),
            _ => {
                if key_str.len() == 1 {
                    keys.push_str(&key_str.to_uppercase());
                } else {
                    let mut chars = key_str.chars();
                    if let Some(first_char) = chars.next() {
                        keys.push_str(&format!(
                            "{}{}",
                            first_char.to_uppercase(),
                            chars.collect::<String>()
                        ));
                    } else {
                        keys.push_str(&key_str);
                    }
                }
            }
        }

        parts.push(&keys);
        parts.join(SEPARATOR)
    }
}

impl Styled for Kbd {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Kbd {
    fn render(self, _: &mut gpui::Window, cx: &mut gpui::App) -> impl gpui::IntoElement {
        if !self.appearance {
            return Self::format(&self.stroke).into_any_element();
        }

        div()
            .text_color(cx.theme().muted_foreground)
            .bg(cx.theme().tokens.muted)
            .when(self.outline, |this| {
                this.border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().tokens.background)
            })
            .py_0p5()
            .px_1()
            .min_w_5()
            .text_center()
            .rounded(cx.theme().radius.half())
            .line_height(relative(1.))
            .text_xs()
            .whitespace_normal()
            .flex_shrink_0()
            .refine_style(&self.style)
            .child(Self::format(&self.stroke))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_format() {
        use super::Kbd;
        use gpui::Keystroke;

        assert_eq!(Kbd::format(&Keystroke::parse("cmd-a").unwrap()), "⌘A");
        assert_eq!(Kbd::format(&Keystroke::parse("cmd--").unwrap()), "⌘-");
        assert_eq!(Kbd::format(&Keystroke::parse("cmd-+").unwrap()), "⌘+");
        assert_eq!(Kbd::format(&Keystroke::parse("cmd-enter").unwrap()), "⌘⏎");
        // `secondary` is ⌘ on macOS and Control elsewhere.
        assert_eq!(
            Kbd::format(&Keystroke::parse("secondary-f12").unwrap()),
            if cfg!(target_os = "macos") {
                "⌘F12"
            } else {
                "⌃F12"
            }
        );
        assert_eq!(
            Kbd::format(&Keystroke::parse("shift-pagedown").unwrap()),
            "⇧Page Down"
        );
        assert_eq!(
            Kbd::format(&Keystroke::parse("shift-pageup").unwrap()),
            "⇧Page Up"
        );
        assert_eq!(
            Kbd::format(&Keystroke::parse("shift-space").unwrap()),
            "⇧Space"
        );
        assert_eq!(Kbd::format(&Keystroke::parse("cmd-ctrl-a").unwrap()), "⌃⌘A");
        assert_eq!(
            Kbd::format(&Keystroke::parse("cmd-alt-backspace").unwrap()),
            "⌥⌘⌫"
        );
        assert_eq!(
            Kbd::format(&Keystroke::parse("shift-delete").unwrap()),
            "⇧⌫"
        );
        assert_eq!(
            Kbd::format(&Keystroke::parse("cmd-ctrl-shift-a").unwrap()),
            "⌃⇧⌘A"
        );
        assert_eq!(
            Kbd::format(&Keystroke::parse("cmd-ctrl-shift-alt-a").unwrap()),
            "⌃⌥⇧⌘A"
        );
    }
}
