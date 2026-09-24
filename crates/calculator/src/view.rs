//! The Calculator window: toolbar, display and keypad, for both Basic and
//! Scientific (CALC-01, CALC-02).

use std::time::Duration;

use gpui::{
    div, prelude::FluentBuilder as _, px, rgb, rgba, size, svg, AnyElement, ClipboardItem, Context,
    FocusHandle, FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Render, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, WindowControlArea,
};
use rmac_calculator::engine::{fitted_font_size, Calculator, HistoryEntry, Key as BasicKey};
use rmac_calculator::keypad::{
    self, key_face, key_for_input, key_origin, key_style, KeyFace, KeyStyle, Palette,
};
use rmac_calculator::scientific::{self, ScientificCalculator};
use rmac_calculator::scientific_keypad;
use rmac_ui::mac;

use crate::{CloseWindow, Copy, Paste, ShowBasic, ShowHistory, ShowScientific};

/// How long a key stays lit after a hardware key press.
const KEY_FLASH: Duration = Duration::from_millis(110);

/// A popover background, matching the material `--mat-menu` uses elsewhere
/// in the design lab (`design-lab/tokens.css`): 0x2C2C32 at 80% opacity.
const POPOVER_BG: u32 = 0x2C2C32CC;
const POPOVER_BORDER: u32 = 0xFFFFFF1A;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub(crate) enum Mode {
    #[default]
    Basic,
    Scientific,
}

/// Which key is lit from a hardware key press. Basic and Scientific use
/// different `Key` types, so the flash marker carries whichever is active.
#[derive(Clone, Copy, Eq, PartialEq)]
enum FlashKey {
    Basic(BasicKey),
    Scientific(scientific::Key),
}

pub(crate) struct CalculatorView {
    pub(crate) focus: FocusHandle,
    mode: Mode,
    calculator: Calculator,
    scientific: ScientificCalculator,
    /// The key lit by the last hardware key press, and a generation so an
    /// older timer cannot clear a newer flash.
    flash: Option<(FlashKey, u64)>,
    flash_generation: u64,
    mode_menu_open: bool,
    history_open: bool,
}

impl CalculatorView {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            mode: Mode::Basic,
            calculator: Calculator::new(),
            scientific: ScientificCalculator::new(),
            flash: None,
            flash_generation: 0,
            mode_menu_open: false,
            history_open: false,
        }
    }

    fn press_basic(&mut self, key: BasicKey, cx: &mut Context<Self>) {
        self.calculator.press(key);
        cx.notify();
    }

    fn press_scientific(&mut self, key: scientific::Key, cx: &mut Context<Self>) {
        self.scientific.press(key);
        cx.notify();
    }

    fn start_flash(&mut self, key: FlashKey, cx: &mut Context<Self>) {
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
        match self.mode {
            Mode::Basic => {
                if let Some(key) =
                    key_for_input(&keystroke.key, keystroke.key_char.as_deref(), shortcut)
                {
                    self.press_basic(key, cx);
                    self.start_flash(FlashKey::Basic(key), cx);
                    cx.stop_propagation();
                }
            }
            Mode::Scientific => {
                if let Some(key) = scientific_keypad::key_for_input(
                    &keystroke.key,
                    keystroke.key_char.as_deref(),
                    shortcut,
                ) {
                    self.press_scientific(key, cx);
                    self.start_flash(FlashKey::Scientific(key), cx);
                    cx.stop_propagation();
                }
            }
        }
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        let text = match self.mode {
            Mode::Basic => self.calculator.copy_text(),
            Mode::Scientific => self.scientific.copy_text(),
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let pasted = match self.mode {
            Mode::Basic => self.calculator.paste(&text),
            Mode::Scientific => self.scientific.paste(&text),
        };
        if pasted {
            cx.notify();
        }
    }

    /// Switch mode, resizing the fixed-size window to that mode's geometry
    /// (CALC-02). Both modes stay non-resizable-by-the-user; only the
    /// programmatic size changes.
    fn set_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        self.mode_menu_open = false;
        if self.mode == mode {
            cx.notify();
            return;
        }
        self.mode = mode;
        let (width, height) = match mode {
            Mode::Basic => (keypad::WINDOW_WIDTH, keypad::WINDOW_HEIGHT),
            Mode::Scientific => (
                scientific_keypad::WINDOW_WIDTH,
                scientific_keypad::WINDOW_HEIGHT,
            ),
        };
        window.resize(size(px(width), px(height)));
        cx.notify();
    }

    fn toggle_mode_menu(&mut self, cx: &mut Context<Self>) {
        self.mode_menu_open = !self.mode_menu_open;
        self.history_open = false;
        cx.notify();
    }

    fn toggle_history(&mut self, cx: &mut Context<Self>) {
        self.history_open = !self.history_open;
        self.mode_menu_open = false;
        cx.notify();
    }

    /// Load a history entry's result back in as the current value, like
    /// clicking a row in macOS's history tape.
    fn load_history(&mut self, index: usize, cx: &mut Context<Self>) {
        match self.mode {
            Mode::Basic => {
                if let Some(entry) = self.calculator.history().get(index).cloned() {
                    self.calculator.paste(&entry.result);
                }
            }
            Mode::Scientific => {
                if let Some(entry) = self.scientific.history().get(index).cloned() {
                    self.scientific.paste(&entry.result);
                }
            }
        }
        self.history_open = false;
        cx.notify();
    }

    fn history(&self) -> &[HistoryEntry] {
        match self.mode {
            Mode::Basic => self.calculator.history(),
            Mode::Scientific => self.scientific.history(),
        }
    }

    fn window_width(&self) -> f32 {
        match self.mode {
            Mode::Basic => keypad::WINDOW_WIDTH,
            Mode::Scientific => scientific_keypad::WINDOW_WIDTH,
        }
    }

    fn window_height(&self) -> f32 {
        match self.mode {
            Mode::Basic => keypad::WINDOW_HEIGHT,
            Mode::Scientific => scientific_keypad::WINDOW_HEIGHT,
        }
    }

    fn sidebar_button_center_x(&self) -> f32 {
        match self.mode {
            Mode::Basic => keypad::SIDEBAR_BUTTON_CENTER_X,
            Mode::Scientific => scientific_keypad::SIDEBAR_BUTTON_CENTER_X,
        }
    }

    fn mode_button_center_x(&self) -> f32 {
        match self.mode {
            Mode::Basic => keypad::MODE_BUTTON_CENTER_X,
            Mode::Scientific => scientific_keypad::MODE_BUTTON_CENTER_X,
        }
    }

    fn render_toolbar(
        &self,
        palette: Palette,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (light_x, light_y) = keypad::TRAFFIC_LIGHT_CENTER;
        let hit_width = mac::traffic_light_hit_width();
        let hit_height = mac::traffic_light_hit_height();
        let sidebar_x = self.sidebar_button_center_x();
        let mode_x = self.mode_button_center_x();
        let button = |id: &'static str, name: &'static str, center_x: f32, glyph: &'static str| {
            let diameter = keypad::TOOLBAR_BUTTON_DIAMETER;
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(name)
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
            .child(
                button(
                    "calculator-history",
                    "History",
                    sidebar_x,
                    "icons/calculator/sidebar.svg",
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_history(cx))),
            )
            .child(
                button(
                    "calculator-mode",
                    "Mode",
                    mode_x,
                    "icons/calculator/calculator.svg",
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_mode_menu(cx))),
            )
    }

    /// The Basic / Scientific / Programmer / Convert popup opened by the
    /// mode button (CALC-01). Programmer and Convert are listed but inert —
    /// out of scope for this task — and are visually dimmed with a "Soon"
    /// tag rather than silently doing nothing indistinguishably from a live
    /// choice.
    fn render_mode_menu(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let diameter = keypad::TOOLBAR_BUTTON_DIAMETER;
        let mode_x = self.mode_button_center_x();
        let (_, light_y) = keypad::TRAFFIC_LIGHT_CENTER;
        let top = light_y + diameter / 2.0 + 6.0;
        let width = 148.0;
        let row = |id: &'static str, name: &'static str, active: bool| {
            div()
                .id(id)
                .role(Role::MenuItem)
                .aria_label(name)
                .aria_selected(active)
                .h(px(24.0))
                .px(px(10.0))
                .flex()
                .items_center()
                .justify_between()
                .rounded(px(6.0))
                .text_size(px(13.0))
                .text_color(if active {
                    rgb(palette.result)
                } else {
                    rgb(palette.expression)
                })
                .when(active, |style| style.bg(rgba(0xFFFFFF14)))
        };
        let soon = || {
            div()
                .text_size(px(10.0))
                .text_color(rgb(palette.expression))
                .child("Soon")
        };
        div()
            .id("calculator-mode-menu")
            .role(Role::Menu)
            .aria_label("Mode")
            .absolute()
            .top(px(top))
            .left(px((mode_x - width / 2.0)
                .max(6.0)
                .min(self.window_width() - width - 6.0)))
            .w(px(width))
            .p(px(5.0))
            .rounded(px(10.0))
            .bg(rgba(POPOVER_BG))
            .border_1()
            .border_color(rgba(POPOVER_BORDER))
            .shadow_lg()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(
                row("calculator-mode-basic", "Basic", self.mode == Mode::Basic)
                    .cursor_pointer()
                    .child("Basic")
                    .when(self.mode == Mode::Basic, |el| el.child("✓"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.set_mode(Mode::Basic, window, cx);
                    })),
            )
            .child(
                row(
                    "calculator-mode-scientific",
                    "Scientific",
                    self.mode == Mode::Scientific,
                )
                .cursor_pointer()
                .child("Scientific")
                .when(self.mode == Mode::Scientific, |el| el.child("✓"))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.set_mode(Mode::Scientific, window, cx);
                })),
            )
            .child(
                row("calculator-mode-programmer", "Programmer", false)
                    .opacity(0.4)
                    .child("Programmer")
                    .child(soon()),
            )
            .child(
                row("calculator-mode-convert", "Convert", false)
                    .opacity(0.4)
                    .child("Convert")
                    .child(soon()),
            )
    }

    /// The history tape opened by the sidebar button (CALC-01/CALC-03): a
    /// panel over the keypad listing every completed calculation in the
    /// active mode, newest first. Clicking one loads its result back in.
    fn render_history_panel(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let top = mac::toolbar_height();
        let width = self.window_width();
        let height = self.window_height() - top;
        let entries = self.history();
        let mut rows = Vec::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate().rev() {
            let expression = SharedString::from(entry.expression.clone());
            let result = SharedString::from(entry.result.clone());
            rows.push(
                div()
                    .id(SharedString::from(format!("calculator-history-{index}")))
                    .role(Role::MenuItem)
                    .aria_label(SharedString::from(format!(
                        "{} = {}",
                        entry.expression, entry.result
                    )))
                    .cursor_pointer()
                    .px(px(12.0))
                    .py(px(6.0))
                    .flex()
                    .flex_col()
                    .items_end()
                    .gap(px(2.0))
                    .border_b_1()
                    .border_color(rgba(palette.rim))
                    .hover(|style| style.bg(rgba(0xFFFFFF0D)))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(palette.expression))
                            .child(expression),
                    )
                    .child(
                        div()
                            .text_size(px(17.0))
                            .text_color(rgb(palette.result))
                            .child(result),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.load_history(index, cx)))
                    .into_any_element(),
            );
        }
        let body: AnyElement = if rows.is_empty() {
            div()
                .flex()
                .items_center()
                .justify_center()
                .size_full()
                .text_size(px(13.0))
                .text_color(rgb(palette.expression))
                .child("No history yet")
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .size_full()
                .overflow_hidden()
                .children(rows)
                .into_any_element()
        };
        div()
            .id("calculator-history-panel")
            .role(Role::Menu)
            .aria_label("History")
            .absolute()
            .top(px(top))
            .left_0()
            .w(px(width))
            .h(px(height))
            .bg(rgba(POPOVER_BG))
            .border_t_1()
            .border_color(rgba(POPOVER_BORDER))
            .child(body)
    }

    fn render_display(&self, palette: Palette) -> impl IntoElement {
        let inset = keypad::DISPLAY_RIGHT_INSET;
        let width = self.window_width() - inset * 2.0;
        let (expression, result) = match self.mode {
            Mode::Basic => (
                self.calculator.expression().to_owned(),
                self.calculator.display(),
            ),
            Mode::Scientific => (
                self.scientific.expression().to_owned(),
                self.scientific.display(),
            ),
        };
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

    fn render_basic_key(
        &self,
        key: BasicKey,
        row: usize,
        column: usize,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (x, y) = key_origin(row, column);
        let selected = matches!(key, BasicKey::Operator(operator)
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
        let flashing = self
            .flash
            .is_some_and(|(flash, _)| flash == FlashKey::Basic(key));
        let face = match (key, key_face(key)) {
            (BasicKey::Clear, _) => KeyFace::Text(self.calculator.clear_label().text()),
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
            .on_click(cx.listener(move |this, _, _, cx| this.press_basic(key, cx)))
    }

    fn render_scientific_key(
        &self,
        key: scientific::Key,
        row: usize,
        column: usize,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (x, y) = scientific_keypad::key_origin(row, column);
        let highlighted = self.scientific.highlighted_operator();
        let selected = match key {
            scientific::Key::Operator(operator) => highlighted == Some(operator),
            scientific::Key::PowerOrRoot => matches!(
                highlighted,
                Some(scientific::BinaryOp::Power | scientific::BinaryOp::Root)
            ),
            _ => false,
        };
        let (fill, label) = match scientific_keypad::key_style(key) {
            KeyStyle::Function => (palette.function_key, palette.function_label),
            KeyStyle::Digit => (palette.digit_key, palette.digit_label),
            KeyStyle::Operator if selected => (
                palette.operator_selected_key,
                palette.operator_selected_label,
            ),
            KeyStyle::Operator => (palette.operator_key, palette.operator_label),
        };
        // `2nd` itself highlights like a selected operator while active, so
        // its own state is visible at a glance.
        let (fill, label) = if key == scientific::Key::Second && self.scientific.second() {
            (
                palette.operator_selected_key,
                palette.operator_selected_label,
            )
        } else {
            (fill, label)
        };
        let pressed_fill = blend(fill, palette.pressed_overlay);
        let flashing = self
            .flash
            .is_some_and(|(flash, _)| flash == FlashKey::Scientific(key));
        let face = match (
            key,
            scientific_keypad::key_face(
                key,
                self.scientific.second(),
                self.scientific.angle_mode(),
            ),
        ) {
            (scientific::Key::Clear, _) => {
                scientific_keypad::KeyFace::Text(self.scientific.clear_label().text())
            }
            (_, face) => face,
        };
        let is_digit_tier = matches!(
            scientific_keypad::key_style(key),
            KeyStyle::Digit | KeyStyle::Operator
        );
        let label_size = if is_digit_tier {
            scientific_keypad::DIGIT_LABEL_SIZE
        } else {
            scientific_keypad::LABEL_SIZE
        };
        let face = match face {
            scientific_keypad::KeyFace::Text(text) => div()
                .text_size(px(label_size))
                .line_height(px(label_size))
                .text_color(rgb(label))
                .child(text)
                .into_any_element(),
            scientific_keypad::KeyFace::Glyph(path) => svg()
                .path(path)
                .size(px(scientific_keypad::GLYPH_SIZE * 0.8))
                .text_color(rgb(label))
                .into_any_element(),
        };
        div()
            .id(SharedString::from(format!(
                "sci-key-{}",
                scientific_keypad::key_name(key)
            )))
            .absolute()
            .left(px(x))
            .top(px(y))
            .size(px(scientific_keypad::KEY_DIAMETER))
            .rounded_full()
            .bg(rgb(if flashing { pressed_fill } else { fill }))
            .border_1()
            .border_color(rgba(palette.rim))
            .flex()
            .items_center()
            .justify_center()
            .active(move |style| style.bg(rgb(pressed_fill)))
            .child(face)
            .on_click(cx.listener(move |this, _, _, cx| this.press_scientific(key, cx)))
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
        let mode = self.mode;
        let mut keys = Vec::with_capacity(45);
        match mode {
            Mode::Basic => {
                for (row, keys_in_row) in keypad::LAYOUT.iter().enumerate() {
                    for (column, key) in keys_in_row.iter().enumerate() {
                        keys.push(
                            self.render_basic_key(*key, row, column, palette, cx)
                                .into_any_element(),
                        );
                    }
                }
            }
            Mode::Scientific => {
                for (row, keys_in_row) in scientific_keypad::LAYOUT.iter().enumerate() {
                    for (column, key) in keys_in_row.iter().enumerate() {
                        let Some(key) = key else { continue };
                        keys.push(
                            self.render_scientific_key(*key, row, column, palette, cx)
                                .into_any_element(),
                        );
                    }
                }
            }
        }
        let window_width = self.window_width();
        let window_height = self.window_height();
        div()
            .id("calculator")
            .track_focus(&self.focus)
            .key_context("Calculator")
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                this.on_key_down(event, cx);
            }))
            .on_action(cx.listener(|this, _: &Copy, _, cx| this.copy(cx)))
            .on_action(cx.listener(|this, _: &Paste, _, cx| this.paste(cx)))
            .on_action(cx.listener(|this, _: &ShowBasic, window, cx| {
                this.set_mode(Mode::Basic, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowScientific, window, cx| {
                this.set_mode(Mode::Scientific, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowHistory, _, cx| this.toggle_history(cx)))
            .on_action(cx.listener(|_, _: &CloseWindow, _, cx| cx.quit()))
            .on_action(cx.listener(|_, _: &rmac_ui::RequestClose, _, cx| cx.quit()))
            .relative()
            .w(px(window_width))
            .h(px(window_height))
            .overflow_hidden()
            .bg(rgb(palette.window))
            .font_features(mac::tabular_font_features())
            .child(self.render_toolbar(palette, window, cx))
            .child(self.render_display(palette))
            .children(keys)
            .when(self.mode_menu_open, |el| {
                el.child(self.render_mode_menu(palette, cx))
            })
            .when(self.history_open, |el| {
                el.child(self.render_history_panel(palette, cx))
            })
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
