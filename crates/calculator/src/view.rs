//! The Calculator window: toolbar, display and keypad.

use std::time::Duration;

use gpui::{
    div, px, rgb, rgba, svg, ClipboardItem, Context, FocusHandle, FontWeight,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, WindowControlArea,
};
use rmac_calculator::engine::{fitted_font_size, Calculator, Key};
use rmac_calculator::keypad::{
    self, key_face, key_for_input, key_origin, key_style, KeyFace, KeyStyle, Palette,
};
use rmac_ui::mac;

use crate::{CloseWindow, Copy, Paste, ShowBasic};

/// How long a key stays lit after a hardware key press.
const KEY_FLASH: Duration = Duration::from_millis(110);

pub(crate) struct CalculatorView {
    pub(crate) focus: FocusHandle,
    calculator: Calculator,
    /// The key lit by the last hardware key press, and a generation so an
    /// older timer cannot clear a newer flash.
    flash: Option<(Key, u64)>,
    flash_generation: u64,
}

impl CalculatorView {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            calculator: Calculator::new(),
            flash: None,
            flash_generation: 0,
        }
    }

    fn press(&mut self, key: Key, cx: &mut Context<Self>) {
        self.calculator.press(key);
        cx.notify();
    }

    fn press_from_keyboard(&mut self, key: Key, cx: &mut Context<Self>) {
        self.press(key, cx);
        self.flash_generation = self.flash_generation.wrapping_add(1);
        let generation = self.flash_generation;
        self.flash = Some((key, generation));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(KEY_FLASH).await;
            let _ = this.update(cx, |view, cx| {
                if view.flash.is_some_and(|(_, current)| current == generation) {
                    view.flash = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let shortcut = keystroke.modifiers.platform || keystroke.modifiers.control;
        if let Some(key) = key_for_input(&keystroke.key, keystroke.key_char.as_deref(), shortcut) {
            self.press_from_keyboard(key, cx);
            cx.stop_propagation();
        }
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(self.calculator.copy_text()));
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        if self.calculator.paste(&text) {
            cx.notify();
        }
    }

    fn render_toolbar(&self, palette: Palette, window: &Window) -> impl IntoElement {
        let (light_x, light_y) = keypad::TRAFFIC_LIGHT_CENTER;
        let hit_width = mac::traffic_light_hit_width();
        let hit_height = mac::traffic_light_hit_height();
        let button = |id: &'static str, center_x: f32, glyph: &'static str| {
            let diameter = keypad::TOOLBAR_BUTTON_DIAMETER;
            div()
                .id(id)
                .absolute()
                .left(px(center_x - diameter / 2.0))
                .top(px(light_y - diameter / 2.0))
                .size(px(diameter))
                .rounded_full()
                .bg(rgb(palette.toolbar_button))
                .border_1()
                .border_color(rgba(palette.rim))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    svg()
                        .path(glyph)
                        .size(px(20.0))
                        .text_color(rgb(palette.toolbar_glyph)),
                )
        };
        div()
            .absolute()
            .top_0()
            .left_0()
            .w_full()
            .h(px(mac::toolbar_height()))
            .child(
                div()
                    .id("calculator-drag")
                    .absolute()
                    .size_full()
                    .window_control_area(WindowControlArea::Drag),
            )
            .child(
                div()
                    .absolute()
                    .left(px(light_x - hit_width / 2.0))
                    .top(px(light_y - hit_height / 2.0))
                    .child(rmac_ui::traffic_lights_fixed_size(
                        window.is_window_active(),
                    )),
            )
            .child(button(
                "calculator-history",
                keypad::SIDEBAR_BUTTON_CENTER_X,
                "icons/calculator/sidebar.svg",
            ))
            .child(button(
                "calculator-mode",
                keypad::MODE_BUTTON_CENTER_X,
                "icons/calculator/calculator.svg",
            ))
    }

    fn render_display(&self, palette: Palette) -> impl IntoElement {
        let inset = keypad::DISPLAY_RIGHT_INSET;
        let width = keypad::WINDOW_WIDTH - inset * 2.0;
        let expression = self.calculator.expression().to_owned();
        let result = self.calculator.display();
        let expression_size = fitted_font_size(&expression, width, keypad::EXPRESSION_SIZE, 11.0);
        let result_size = fitted_font_size(
            &result,
            width,
            keypad::RESULT_MAX_SIZE,
            keypad::RESULT_MIN_SIZE,
        );
        let line = |top: f32, height: f32| {
            div()
                .absolute()
                .left(px(inset))
                .right(px(inset))
                .top(px(top))
                .h(px(height))
                .line_height(px(height))
                .flex()
                .items_center()
                .justify_end()
                .whitespace_nowrap()
                .overflow_hidden()
        };
        div()
            .child(
                line(keypad::EXPRESSION_TOP, keypad::EXPRESSION_LINE)
                    .text_size(px(expression_size))
                    .text_color(rgb(palette.expression))
                    .child(SharedString::from(expression)),
            )
            .child(
                line(keypad::RESULT_TOP, keypad::RESULT_LINE)
                    .id("calculator-result")
                    .text_size(px(result_size))
                    .font_weight(FontWeight::LIGHT)
                    .text_color(rgb(palette.result))
                    .child(SharedString::from(result)),
            )
    }

    fn render_key(
        &self,
        key: Key,
        row: usize,
        column: usize,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (x, y) = key_origin(row, column);
        let selected = matches!(key, Key::Operator(operator)
            if self.calculator.highlighted_operator() == Some(operator));
        let (fill, label) = match key_style(key) {
            KeyStyle::Function => (palette.function_key, palette.function_label),
            KeyStyle::Digit => (palette.digit_key, palette.digit_label),
            KeyStyle::Operator if selected => (
                palette.operator_selected_key,
                palette.operator_selected_label,
            ),
            KeyStyle::Operator => (palette.operator_key, palette.operator_label),
        };
        let pressed_fill = blend(fill, palette.pressed_overlay);
        let flashing = self.flash.is_some_and(|(flash, _)| flash == key);
        let face = match (key, key_face(key)) {
            (Key::Clear, _) => KeyFace::Text(self.calculator.clear_label().text()),
            (_, face) => face,
        };
        let face = match face {
            KeyFace::Text(text) => div()
                .text_size(px(keypad::LABEL_SIZE))
                .line_height(px(keypad::LABEL_SIZE))
                .text_color(rgb(label))
                .child(text)
                .into_any_element(),
            KeyFace::Glyph(path) => svg()
                .path(path)
                .size(px(keypad::GLYPH_SIZE))
                .text_color(rgb(label))
                .into_any_element(),
        };
        div()
            .id(SharedString::from(format!(
                "key-{}",
                keypad::key_name(key).replace(' ', "-")
            )))
            .absolute()
            .left(px(x))
            .top(px(y))
            .size(px(keypad::KEY_DIAMETER))
            .rounded_full()
            .bg(rgb(if flashing { pressed_fill } else { fill }))
            .border_1()
            .border_color(rgba(palette.rim))
            .flex()
            .items_center()
            .justify_center()
            .active(move |style| style.bg(rgb(pressed_fill)))
            .child(face)
            .on_click(cx.listener(move |this, _, _, cx| this.press(key, cx)))
    }
}

/// Composite a 0xRRGGBBAA overlay onto an opaque 0xRRGGBB colour.
fn blend(base: u32, overlay: u32) -> u32 {
    let alpha = (overlay & 0xFF) as f32 / 255.0;
    let channel = |shift: u32| {
        let below = ((base >> shift) & 0xFF) as f32;
        let above = ((overlay >> (shift + 8)) & 0xFF) as f32;
        ((below + (above - below) * alpha).round() as u32) << shift
    };
    channel(16) | channel(8) | channel(0)
}

fn palette() -> Palette {
    if mac::window().l > 0.5 {
        keypad::LIGHT
    } else {
        keypad::DARK
    }
}

impl Render for CalculatorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = palette();
        let mut keys = Vec::with_capacity(20);
        for (row, keys_in_row) in keypad::LAYOUT.iter().enumerate() {
            for (column, key) in keys_in_row.iter().enumerate() {
                keys.push(self.render_key(*key, row, column, palette, cx));
            }
        }
        div()
            .id("calculator")
            .track_focus(&self.focus)
            .key_context("Calculator")
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                this.on_key_down(event, cx);
            }))
            .on_action(cx.listener(|this, _: &Copy, _, cx| this.copy(cx)))
            .on_action(cx.listener(|this, _: &Paste, _, cx| this.paste(cx)))
            .on_action(cx.listener(|_, _: &ShowBasic, _, _| {}))
            .on_action(cx.listener(|_, _: &CloseWindow, _, cx| cx.quit()))
            .on_action(cx.listener(|_, _: &rmac_ui::RequestClose, _, cx| cx.quit()))
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(palette.window))
            .font_features(mac::tabular_font_features())
            .child(self.render_toolbar(palette, window))
            .child(self.render_display(palette))
            .children(keys)
    }
}

#[cfg(test)]
mod tests {
    use super::blend;

    #[test]
    fn blend_mixes_overlay_by_alpha() {
        assert_eq!(blend(0x000000, 0xFFFFFF00), 0x000000);
        assert_eq!(blend(0x000000, 0xFFFFFFFF), 0xFFFFFF);
        assert_eq!(blend(0x000000, 0xFFFFFF80), 0x808080);
        assert_eq!(blend(0xFF9200, 0x00000000), 0xFF9200);
    }
}
