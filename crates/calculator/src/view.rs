//! The Calculator window: toolbar, display and keypad, for both Basic and
//! Scientific (CALC-01, CALC-02).

use std::time::Duration;

use gpui::{
    accesskit, div, prelude::FluentBuilder as _, px, rgb, rgba, size, svg, A11ySubtreeBuilder,
    AnyElement, ClipboardItem, Context, FocusHandle, FontWeight, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, WindowControlArea,
};
use rmac_calculator::convert::{self, ConvertState};
use rmac_calculator::convert_keypad::{self, Key as ConvertKey};
use rmac_calculator::engine::{fitted_font_size, Calculator, HistoryEntry, Key as BasicKey};
use rmac_calculator::keypad::{
    self, key_face, key_for_input, key_origin, key_style, KeyFace, KeyStyle, Palette,
};
use rmac_calculator::programmer::{self, ProgrammerCalculator};
use rmac_calculator::programmer_keypad;
use rmac_calculator::rpn::{self, RpnEngine};
use rmac_calculator::scientific::{self, ScientificCalculator};
use rmac_calculator::scientific_keypad;
use rmac_ui::mac;

use crate::{
    CloseWindow, Copy, DecimalPlaces0, DecimalPlaces1, DecimalPlaces10, DecimalPlaces11,
    DecimalPlaces12, DecimalPlaces13, DecimalPlaces14, DecimalPlaces15, DecimalPlaces2,
    DecimalPlaces3, DecimalPlaces4, DecimalPlaces5, DecimalPlaces6, DecimalPlaces7, DecimalPlaces8,
    DecimalPlaces9, EnterFullScreen, Paste, ShowBasic, ShowConvert, ShowHistory, ShowMathsNotes,
    ShowProgrammer, ShowScientific, ToggleRpnMode, ToggleThousandsSeparator,
};

/// How long a key stays lit after a hardware key press.
const KEY_FLASH: Duration = Duration::from_millis(110);

/// RPN Mode reuses Basic's own `+ − × ÷` keys (see `rpn.rs`); this maps
/// Basic's operator type to RPN's.
fn rpn_operator(operator: rmac_calculator::engine::Operator) -> rpn::Operator {
    use rmac_calculator::engine::Operator as BasicOperator;
    match operator {
        BasicOperator::Add => rpn::Operator::Add,
        BasicOperator::Subtract => rpn::Operator::Subtract,
        BasicOperator::Multiply => rpn::Operator::Multiply,
        BasicOperator::Divide => rpn::Operator::Divide,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub(crate) enum Mode {
    #[default]
    Basic,
    Scientific,
    Programmer,
    Convert,
}

/// Which key is lit from a hardware key press. Each mode with its own
/// keypad carries its own `Key` type; RPN mode reuses Basic's own keys (see
/// `rpn.rs`'s module doc comment) so it has no flash variant of its own.
#[derive(Clone, Copy, Eq, PartialEq)]
enum FlashKey {
    Basic(BasicKey),
    Scientific(scientific::Key),
    Programmer(programmer::Key),
}

pub(crate) struct CalculatorView {
    pub(crate) focus: FocusHandle,
    mode: Mode,
    calculator: Calculator,
    scientific: ScientificCalculator,
    programmer: ProgrammerCalculator,
    convert: ConvertState,
    /// RPN Mode (View ▸ RPN Mode, ⌘R): scoped to Basic's keypad (CALC-06's
    /// doc comment explains why). `rpn` holds the stack even while the
    /// toggle is off, so turning it back on does not lose work.
    rpn_enabled: bool,
    rpn: RpnEngine,
    /// The key lit by the last hardware key press, and a generation so an
    /// older timer cannot clear a newer flash.
    flash: Option<(FlashKey, u64)>,
    flash_generation: u64,
    mode_menu_open: bool,
    history_open: bool,
    hide_thousands_separator: bool,
    decimal_places: usize,
}

impl CalculatorView {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            mode: Mode::Basic,
            calculator: Calculator::new(),
            scientific: ScientificCalculator::new(),
            programmer: ProgrammerCalculator::new(),
            convert: ConvertState::new(),
            rpn_enabled: false,
            rpn: RpnEngine::new(),
            flash: None,
            flash_generation: 0,
            mode_menu_open: false,
            history_open: false,
            hide_thousands_separator: false,
            decimal_places: 8,
        }
    }

    /// In RPN Mode, Basic's own keys drive the stack engine instead of the
    /// ordinary algebraic one (see `rpn.rs`'s module doc comment).
    fn press_basic(&mut self, key: BasicKey, cx: &mut Context<Self>) {
        if self.rpn_enabled {
            self.press_rpn(key, cx);
            return;
        }
        self.calculator.press(key);
        cx.notify();
    }

    fn press_rpn(&mut self, key: BasicKey, cx: &mut Context<Self>) {
        match key {
            BasicKey::Digit(digit) => self.rpn.digit(digit),
            BasicKey::Decimal => self.rpn.decimal(),
            BasicKey::Operator(operator) => self.rpn.operator(rpn_operator(operator)),
            BasicKey::Equals => self.rpn.enter(),
            BasicKey::ToggleSign => self.rpn.toggle_sign(),
            BasicKey::Clear => self.rpn.clear(),
            BasicKey::Backspace => self.rpn.backspace(),
            BasicKey::Percent => self.rpn.percent(),
        }
        cx.notify();
    }

    fn press_scientific(&mut self, key: scientific::Key, cx: &mut Context<Self>) {
        self.scientific.press(key);
        cx.notify();
    }

    fn press_programmer(&mut self, key: programmer::Key, cx: &mut Context<Self>) {
        self.programmer.press(key);
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
            Mode::Programmer => {
                if let Some(key) = programmer_keypad::key_for_input(
                    &keystroke.key,
                    keystroke.key_char.as_deref(),
                    shortcut,
                ) {
                    if programmer_keypad::key_enabled(key, self.programmer.base()) {
                        self.press_programmer(key, cx);
                        self.start_flash(FlashKey::Programmer(key), cx);
                        cx.stop_propagation();
                    }
                }
            }
            Mode::Convert => {
                if shortcut {
                    return;
                }
                if let Some(character) = keystroke.key_char.as_deref().and_then(|text| {
                    let mut chars = text.chars();
                    let (Some(character), None) = (chars.next(), chars.next()) else {
                        return None;
                    };
                    Some(character)
                }) {
                    match character {
                        '0'..='9' => {
                            self.convert.digit(character as u8 - b'0');
                            cx.notify();
                        }
                        '.' | ',' => {
                            self.convert.decimal();
                            cx.notify();
                        }
                        _ => {}
                    }
                    cx.stop_propagation();
                } else {
                    match keystroke.key.as_str() {
                        "backspace" | "delete" => {
                            self.convert.backspace();
                            cx.notify();
                            cx.stop_propagation();
                        }
                        "escape" => {
                            self.convert.clear();
                            cx.notify();
                            cx.stop_propagation();
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        let text = match self.mode {
            Mode::Basic if self.rpn_enabled => self.rpn.current_text(),
            Mode::Basic => self.calculator.copy_text(),
            Mode::Scientific => self.scientific.copy_text(),
            Mode::Programmer => self.programmer.display(),
            Mode::Convert => self.convert.to_text(),
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let pasted = match self.mode {
            Mode::Basic if self.rpn_enabled => false,
            Mode::Basic => self.calculator.paste(&text),
            Mode::Scientific => self.scientific.paste(&text),
            Mode::Programmer | Mode::Convert => false,
        };
        if pasted {
            cx.notify();
        }
    }

    /// Switch mode, resizing the fixed-size window to that mode's geometry
    /// (CALC-02). Every mode stays non-resizable-by-the-user; only the
    /// programmatic size changes. The current value carries across
    /// Basic/Scientific/Programmer (CALC-14); Convert's From/To fields are
    /// independent of the other modes' shared register, like on the Mac.
    fn set_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        self.mode_menu_open = false;
        if self.mode == mode {
            cx.notify();
            return;
        }
        let outgoing_value = match self.mode {
            Mode::Basic if self.rpn_enabled => None,
            Mode::Basic => Some(self.calculator.current_value()),
            Mode::Scientific => Some(self.scientific.current_value()),
            Mode::Programmer => Some(self.programmer.current_value() as f64),
            Mode::Convert => None,
        };
        if let Some(value) = outgoing_value {
            match mode {
                Mode::Basic => self.calculator.restore_value(value),
                Mode::Scientific => self.scientific.restore_value(value),
                Mode::Programmer => self.programmer.restore_value(value),
                Mode::Convert => {}
            }
        }
        self.mode = mode;
        rmac_ui::set_menu_checked("calculator::ShowBasic", mode == Mode::Basic, cx);
        rmac_ui::set_menu_checked("calculator::ShowScientific", mode == Mode::Scientific, cx);
        rmac_ui::set_menu_checked("calculator::ShowProgrammer", mode == Mode::Programmer, cx);
        rmac_ui::set_menu_checked("calculator::ShowConvert", mode == Mode::Convert, cx);
        // RPN Mode only drives Basic's keypad (see `rpn.rs`'s module doc
        // comment), so it is greyed everywhere else, like a Mac row that
        // genuinely does not apply right now.
        rmac_ui::set_menu_enabled("calculator::ToggleRpnMode", mode == Mode::Basic, cx);
        let (width, height) = match mode {
            Mode::Basic => (keypad::WINDOW_WIDTH, keypad::WINDOW_HEIGHT),
            Mode::Scientific => (
                scientific_keypad::WINDOW_WIDTH,
                scientific_keypad::WINDOW_HEIGHT,
            ),
            Mode::Programmer => (
                programmer_keypad::WINDOW_WIDTH,
                programmer_keypad::WINDOW_HEIGHT,
            ),
            Mode::Convert => (convert_keypad::WINDOW_WIDTH, convert_keypad::WINDOW_HEIGHT),
        };
        window.resize(size(px(width), px(height)));
        cx.notify();
    }

    /// View ▸ RPN Mode, ⌘R. Only meaningful in Basic (see `rpn.rs`'s
    /// module doc comment); a stray ⌘R elsewhere is a no-op rather than
    /// surprising whatever mode is actually showing.
    fn toggle_rpn_mode(&mut self, cx: &mut Context<Self>) {
        if self.mode != Mode::Basic {
            return;
        }
        self.rpn_enabled = !self.rpn_enabled;
        rmac_ui::set_menu_checked("calculator::ToggleRpnMode", self.rpn_enabled, cx);
        self.mode_menu_open = false;
        cx.notify();
    }

    fn set_base(&mut self, base: programmer::Base, cx: &mut Context<Self>) {
        self.programmer.set_base(base);
        cx.notify();
    }

    fn set_word_size(&mut self, word_size: programmer::WordSize, cx: &mut Context<Self>) {
        self.programmer.set_word_size(word_size);
        cx.notify();
    }

    fn convert_next_category(&mut self, cx: &mut Context<Self>) {
        self.convert.next_category();
        cx.notify();
    }

    fn convert_next_from_unit(&mut self, cx: &mut Context<Self>) {
        self.convert.next_from_unit();
        cx.notify();
    }

    fn convert_next_to_unit(&mut self, cx: &mut Context<Self>) {
        self.convert.next_to_unit();
        cx.notify();
    }

    fn press_convert(&mut self, key: ConvertKey, cx: &mut Context<Self>) {
        match key {
            ConvertKey::Digit(digit) => self.convert.digit(digit),
            ConvertKey::Decimal => self.convert.decimal(),
            ConvertKey::Clear => self.convert.clear(),
            ConvertKey::Backspace => self.convert.backspace(),
            ConvertKey::Swap => self.convert.swap(),
        }
        cx.notify();
    }

    fn toggle_mode_menu(&mut self, cx: &mut Context<Self>) {
        self.mode_menu_open = !self.mode_menu_open;
        self.history_open = false;
        rmac_ui::set_menu_checked("calculator::ShowHistory", false, cx);
        cx.notify();
    }

    fn toggle_history(&mut self, cx: &mut Context<Self>) {
        self.history_open = !self.history_open;
        rmac_ui::set_menu_checked("calculator::ShowHistory", self.history_open, cx);
        self.mode_menu_open = false;
        cx.notify();
    }

    fn toggle_thousands_separator(&mut self, cx: &mut Context<Self>) {
        self.hide_thousands_separator = !self.hide_thousands_separator;
        rmac_ui::set_menu_checked(
            "calculator::ToggleThousandsSeparator",
            self.hide_thousands_separator,
            cx,
        );
        cx.notify();
    }

    fn set_decimal_places(&mut self, places: usize, cx: &mut Context<Self>) {
        let old = format!("calculator::DecimalPlaces{}", self.decimal_places);
        let new = format!("calculator::DecimalPlaces{places}");
        rmac_ui::set_menu_checked(&old, false, cx);
        rmac_ui::set_menu_checked(&new, true, cx);
        self.decimal_places = places;
        cx.notify();
    }

    fn display_result(&self, text: &str, value: Option<f64>) -> String {
        if text.contains('e') {
            return self.display_grouping(text);
        }
        value
            .and_then(|value| format_decimal_value(value, self.decimal_places))
            .map(|formatted| self.display_grouping(&formatted))
            .unwrap_or_else(|| self.display_grouping(text))
    }

    fn display_grouping(&self, text: &str) -> String {
        if self.hide_thousands_separator {
            text.replace(',', "")
        } else {
            text.to_owned()
        }
    }

    /// Load a history entry's result back in as the current value, like
    /// clicking a row in macOS's history tape.
    fn load_history(&mut self, index: usize, cx: &mut Context<Self>) {
        match self.mode {
            Mode::Basic if !self.rpn_enabled => {
                if let Some(entry) = self.calculator.history().get(index).cloned() {
                    self.calculator.paste(&entry.result);
                }
            }
            Mode::Scientific => {
                if let Some(entry) = self.scientific.history().get(index).cloned() {
                    self.scientific.paste(&entry.result);
                }
            }
            Mode::Basic | Mode::Programmer | Mode::Convert => {}
        }
        self.history_open = false;
        rmac_ui::set_menu_checked("calculator::ShowHistory", false, cx);
        cx.notify();
    }

    /// Programmer and Convert have no history tape; RPN's working stack is
    /// shown by the display itself, not the history panel.
    fn history(&self) -> &[HistoryEntry] {
        match self.mode {
            Mode::Basic if !self.rpn_enabled => self.calculator.history(),
            Mode::Scientific => self.scientific.history(),
            Mode::Basic | Mode::Programmer | Mode::Convert => &[],
        }
    }

    fn window_width(&self) -> f32 {
        match self.mode {
            Mode::Basic => keypad::WINDOW_WIDTH,
            Mode::Scientific => scientific_keypad::WINDOW_WIDTH,
            Mode::Programmer => programmer_keypad::WINDOW_WIDTH,
            Mode::Convert => convert_keypad::WINDOW_WIDTH,
        }
    }

    fn window_height(&self) -> f32 {
        match self.mode {
            Mode::Basic => keypad::WINDOW_HEIGHT,
            Mode::Scientific => scientific_keypad::WINDOW_HEIGHT,
            Mode::Programmer => programmer_keypad::WINDOW_HEIGHT,
            Mode::Convert => convert_keypad::WINDOW_HEIGHT,
        }
    }

    fn sidebar_button_center_x(&self) -> f32 {
        match self.mode {
            Mode::Basic => keypad::SIDEBAR_BUTTON_CENTER_X,
            Mode::Scientific => scientific_keypad::SIDEBAR_BUTTON_CENTER_X,
            Mode::Programmer => programmer_keypad::SIDEBAR_BUTTON_CENTER_X,
            Mode::Convert => keypad::SIDEBAR_BUTTON_CENTER_X,
        }
    }

    fn mode_button_center_x(&self) -> f32 {
        match self.mode {
            Mode::Basic => keypad::MODE_BUTTON_CENTER_X,
            Mode::Scientific => scientific_keypad::MODE_BUTTON_CENTER_X,
            Mode::Programmer => programmer_keypad::MODE_BUTTON_CENTER_X,
            Mode::Convert => keypad::MODE_BUTTON_CENTER_X,
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
                // Basic and Scientific are fixed-size on the Mac. Keep the
                // drag area, but do not attach the shared Zoom-on-double-
                // click handler: niri can otherwise widen this fixed surface.
                div()
                    .id("calculator-drag")
                    .window_control_area(WindowControlArea::Drag)
                    .absolute()
                    .size_full(),
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
                .rounded(px(mac::radius_menu_item()))
                .text_size(px(13.0))
                .text_color(if active {
                    rgb(palette.result)
                } else {
                    rgb(palette.expression)
                })
                .when(active, |style| style.bg(rgba(palette.menu_selected)))
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
            .rounded(px(mac::radius_menu()))
            .bg(rgba(palette.popover))
            .border_1()
            .border_color(rgba(palette.popover_border))
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
                row(
                    "calculator-mode-programmer",
                    "Programmer",
                    self.mode == Mode::Programmer,
                )
                .cursor_pointer()
                .child("Programmer")
                .when(self.mode == Mode::Programmer, |el| el.child("✓"))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.set_mode(Mode::Programmer, window, cx);
                })),
            )
            .child(
                row(
                    "calculator-mode-convert",
                    "Convert",
                    self.mode == Mode::Convert,
                )
                .cursor_pointer()
                .child("Convert")
                .when(self.mode == Mode::Convert, |el| el.child("✓"))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.set_mode(Mode::Convert, window, cx);
                })),
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
            let expression = SharedString::from(self.display_grouping(&entry.expression));
            let result = SharedString::from(self.display_grouping(&entry.result));
            rows.push(
                div()
                    .id(SharedString::from(format!("calculator-history-{index}")))
                    .role(Role::MenuItem)
                    .aria_label(SharedString::from(self.display_grouping(&format!(
                        "{} = {}",
                        entry.expression, entry.result
                    ))))
                    .cursor_pointer()
                    .px(px(12.0))
                    .py(px(6.0))
                    .flex()
                    .flex_col()
                    .items_end()
                    .gap(px(2.0))
                    .border_b_1()
                    .border_color(rgba(palette.rim))
                    .hover(|style| style.bg(rgba(palette.menu_hover)))
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
            .bg(rgba(palette.popover))
            .border_t_1()
            .border_color(rgba(palette.popover_border))
            .child(body)
    }

    /// One AccessKit text run publishing `text` under this node: AccessKit
    /// (and so AT-SPI's Text interface) only reads a node's text from
    /// `Role::TextRun` children, never a plain `value`/`aria_value` string
    /// (see `rmac_ui::accessibility`, which this read-only display cannot
    /// use since it names an `InputState` field, not a static result).
    fn accessible_display_text(text: String) -> impl FnOnce(&mut A11ySubtreeBuilder) + 'static {
        move |builder| {
            let text = flatten_superscript_digits(&text);
            let id = builder.synthetic_node_id(("display-text", 0usize));
            let mut node = accesskit::Node::new(accesskit::Role::TextRun);
            node.set_value(text.clone());
            node.set_character_lengths(
                text.chars().map(|c| c.len_utf8() as u8).collect::<Vec<_>>(),
            );
            builder.push_child(id, node);
            // AT-SPI announces a live node when its *name* changes. Publish
            // the result on the parent as both label and value; changing only
            // the text-run child emits text events, which Orca ignores while
            // keyboard focus remains on the calculator window.
            builder.parent_node().set_label(text.clone());
            builder.parent_node().set_value(text);
            // Keep the control's purpose in its description while the name
            // carries the changing result. The behavior probe finds the
            // display by name or description.
            builder.parent_node().set_description("Display");
            builder.parent_node().set_live(accesskit::Live::Polite);
            builder.parent_node().set_live_atomic();
        }
    }

    /// RPN Mode's display: up to three stack lines in small type, then the
    /// current entry/top-of-stack in the same large type Basic's result
    /// line uses. See `rpn.rs`'s module doc comment.
    fn render_rpn_display(&self, palette: Palette) -> AnyElement {
        let inset = keypad::DISPLAY_RIGHT_INSET;
        let width = self.window_width() - inset * 2.0;
        const STACK_LINE_HEIGHT: f32 = 13.0;
        const STACK_TOP: f32 = 40.0;
        let stack_rows = self
            .rpn
            .stack_lines()
            .into_iter()
            .enumerate()
            .map(|(index, text)| {
                div()
                    .absolute()
                    .left(px(inset))
                    .right(px(inset))
                    .top(px(STACK_TOP + index as f32 * STACK_LINE_HEIGHT))
                    .h(px(STACK_LINE_HEIGHT))
                    .line_height(px(STACK_LINE_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_end()
                    .text_size(px(11.0))
                    .text_color(rgb(palette.expression))
                    .child(SharedString::from(self.display_grouping(&text)))
                    .into_any_element()
            });
        let current = self.display_grouping(&self.rpn.current_text());
        let result_size = fitted_font_size(
            &current,
            width,
            keypad::RESULT_MAX_SIZE,
            keypad::RESULT_MIN_SIZE,
        );
        div()
            .children(stack_rows)
            .child(
                div()
                    .id("calculator-result")
                    .role(Role::Label)
                    .aria_label("Display")
                    .absolute()
                    .left(px(inset))
                    .right(px(inset))
                    .top(px(keypad::RESULT_TOP))
                    .h(px(keypad::RESULT_LINE))
                    .line_height(px(keypad::RESULT_LINE))
                    .flex()
                    .items_center()
                    .justify_end()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .a11y_synthetic_children(Self::accessible_display_text(current.clone()))
                    .text_size(px(result_size))
                    .font_weight(FontWeight::LIGHT)
                    .text_color(rgb(palette.result))
                    .child(SharedString::from(current)),
            )
            .into_any_element()
    }

    fn render_display(&self, palette: Palette) -> AnyElement {
        if self.mode == Mode::Basic && self.rpn_enabled {
            return self.render_rpn_display(palette);
        }
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
            Mode::Programmer | Mode::Convert => (String::new(), String::new()),
        };
        let expression = self.display_grouping(&expression);
        let settled_value = match self.mode {
            Mode::Basic if self.calculator.has_settled_result() => {
                Some(self.calculator.current_value())
            }
            Mode::Scientific if self.scientific.has_settled_result() => {
                Some(self.scientific.current_value())
            }
            _ => None,
        };
        let result = self.display_result(&result, settled_value);
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
                    .role(Role::Label)
                    .aria_label("Display")
                    .a11y_synthetic_children(Self::accessible_display_text(result.clone()))
                    .text_size(px(result_size))
                    .font_weight(FontWeight::LIGHT)
                    .text_color(rgb(palette.result))
                    .child(SharedString::from(result)),
            )
            .into_any_element()
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
            KeyStyle::Scientific => (palette.scientific_key, palette.scientific_label),
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
            .role(Role::Button)
            .aria_label(keypad::key_name(key))
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
            scientific::Key::Power => highlighted == Some(scientific::BinaryOp::Power),
            scientific::Key::YRoot => highlighted == Some(scientific::BinaryOp::Root),
            _ => false,
        };
        let (fill, label) = match scientific_keypad::key_style(key) {
            KeyStyle::Function => (palette.function_key, palette.function_label),
            KeyStyle::Digit => (palette.digit_key, palette.digit_label),
            KeyStyle::Scientific => (palette.scientific_key, palette.scientific_label),
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
            scientific_keypad::KeyFace::Text(text) => scientific_label(text, label_size, label),
            scientific_keypad::KeyFace::Glyph(path) => svg()
                .path(path)
                .size(px(scientific_keypad::GLYPH_SIZE))
                .text_color(rgb(label))
                .into_any_element(),
        };
        div()
            .id(SharedString::from(format!(
                "sci-key-{}",
                scientific_keypad::key_name(key)
            )))
            .role(Role::Button)
            .aria_label(scientific_keypad::key_name(key))
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(scientific_keypad::KEY_WIDTH))
            .h(px(scientific_keypad::KEY_HEIGHT))
            .rounded(px(scientific_keypad::KEY_HEIGHT / 2.0))
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

    /// The HEX/DEC/OCT/BIN base tabs and the word-size cycle button shown
    /// above Programmer's keypad.
    fn render_base_strip(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let base = self.programmer.base();
        let tabs = programmer_keypad::BASES.iter().map(|&candidate| {
            let active = candidate == base;
            div()
                .id(SharedString::from(format!(
                    "prog-base-{}",
                    candidate.label()
                )))
                .role(Role::Button)
                .aria_label(candidate.label())
                .aria_selected(active)
                .cursor_pointer()
                .px(px(6.0))
                .text_size(px(11.0))
                .font_weight(if active {
                    FontWeight::BOLD
                } else {
                    FontWeight::NORMAL
                })
                .text_color(if active {
                    rgb(palette.result)
                } else {
                    rgb(palette.expression)
                })
                .child(candidate.label())
                .on_click(cx.listener(move |this, _, _, cx| this.set_base(candidate, cx)))
                .into_any_element()
        });
        let word_size = self.programmer.word_size();
        let word_size_button = div()
            .id("prog-word-size")
            .role(Role::Button)
            .aria_label(SharedString::from(format!(
                "Word size: {}",
                word_size.label()
            )))
            .cursor_pointer()
            .px(px(6.0))
            .text_size(px(11.0))
            .text_color(rgb(palette.expression))
            .child(word_size.label())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_word_size(word_size.next(), cx);
            }));
        div()
            .absolute()
            .left(px(programmer_keypad::KEYPAD_LEFT))
            .right(px(programmer_keypad::KEYPAD_LEFT))
            .top(px(programmer_keypad::BASE_STRIP_TOP))
            .h(px(programmer_keypad::BASE_STRIP_HEIGHT))
            .flex()
            .items_center()
            .justify_between()
            .child(div().flex().items_center().gap(px(8.0)).children(tabs))
            .child(word_size_button)
    }

    /// Programmer's single display line (no formula echo — see CALC-06's
    /// parity note for why this is simpler than Basic/Scientific's).
    fn render_programmer_display(&self, palette: Palette) -> impl IntoElement {
        let inset = programmer_keypad::DISPLAY_RIGHT_INSET;
        let width = self.window_width() - inset * 2.0;
        let text = self.programmer.display();
        let size = fitted_font_size(
            &text,
            width,
            programmer_keypad::RESULT_MAX_SIZE,
            programmer_keypad::RESULT_MIN_SIZE,
        );
        div()
            .id("calculator-result")
            .role(Role::Label)
            .aria_label("Display")
            .absolute()
            .left(px(inset))
            .right(px(inset))
            .top(px(programmer_keypad::RESULT_TOP))
            .h(px(programmer_keypad::RESULT_LINE))
            .line_height(px(programmer_keypad::RESULT_LINE))
            .flex()
            .items_center()
            .justify_end()
            .whitespace_nowrap()
            .overflow_hidden()
            .a11y_synthetic_children(Self::accessible_display_text(text.clone()))
            .text_size(px(size))
            .font_weight(FontWeight::LIGHT)
            .text_color(rgb(palette.result))
            .child(SharedString::from(text))
    }

    fn render_programmer_key(
        &self,
        key: programmer::Key,
        row: usize,
        column: usize,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (x, y) = programmer_keypad::key_origin(row, column);
        let enabled = programmer_keypad::key_enabled(key, self.programmer.base());
        let selected = matches!(key, programmer::Key::Operator(operator)
            if self.programmer.highlighted_operator() == Some(operator));
        let (fill, label) = match programmer_keypad::key_style(key) {
            programmer_keypad::KeyStyle::Function => (palette.function_key, palette.function_label),
            programmer_keypad::KeyStyle::Digit => (palette.digit_key, palette.digit_label),
            programmer_keypad::KeyStyle::Hex | programmer_keypad::KeyStyle::Bitwise => {
                (palette.scientific_key, palette.scientific_label)
            }
            programmer_keypad::KeyStyle::Operator if selected => (
                palette.operator_selected_key,
                palette.operator_selected_label,
            ),
            programmer_keypad::KeyStyle::Operator => (palette.operator_key, palette.operator_label),
        };
        let pressed_fill = blend(fill, palette.pressed_overlay);
        let flashing = self
            .flash
            .is_some_and(|(flash, _)| flash == FlashKey::Programmer(key));
        let face = match (key, programmer_keypad::key_face(key)) {
            (programmer::Key::Clear, _) => {
                programmer_keypad::KeyFace::Text(self.programmer.clear_label().text())
            }
            (_, face) => face,
        };
        let label_size = match programmer_keypad::key_style(key) {
            programmer_keypad::KeyStyle::Bitwise => programmer_keypad::FUNCTION_LABEL_SIZE,
            _ => programmer_keypad::LABEL_SIZE,
        };
        let face = match face {
            programmer_keypad::KeyFace::Text(text) => div()
                .text_size(px(label_size))
                .line_height(px(label_size))
                .text_color(rgb(label))
                .child(text)
                .into_any_element(),
            programmer_keypad::KeyFace::Glyph(path) => svg()
                .path(path)
                .size(px(programmer_keypad::GLYPH_SIZE))
                .text_color(rgb(label))
                .into_any_element(),
        };
        div()
            .id(SharedString::from(format!(
                "prog-key-{}",
                programmer_keypad::key_name(key)
            )))
            .role(Role::Button)
            .aria_label(programmer_keypad::key_name(key))
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(programmer_keypad::KEY_WIDTH))
            .h(px(programmer_keypad::KEY_HEIGHT))
            .rounded(px(programmer_keypad::KEY_HEIGHT / 2.0))
            .bg(rgb(if flashing { pressed_fill } else { fill }))
            .border_1()
            .border_color(rgba(palette.rim))
            .flex()
            .items_center()
            .justify_center()
            .opacity(if enabled { 1.0 } else { 0.35 })
            .child(face)
            .when(enabled, |el| {
                el.cursor_pointer()
                    .active(move |style| style.bg(rgb(pressed_fill)))
                    .on_click(cx.listener(move |this, _, _, cx| this.press_programmer(key, cx)))
            })
    }

    /// Convert's category selector and From/To rows, above its keypad.
    fn render_convert_header(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let category_label = convert::category_label(self.convert.category());
        let category_row = div()
            .id("convert-category")
            .role(Role::Button)
            .aria_label(SharedString::from(format!("Category: {category_label}")))
            .cursor_pointer()
            .absolute()
            .left(px(keypad::DISPLAY_RIGHT_INSET))
            .right(px(keypad::DISPLAY_RIGHT_INSET))
            .top(px(convert_keypad::CATEGORY_ROW_TOP))
            .h(px(convert_keypad::CATEGORY_ROW_HEIGHT))
            .flex()
            .items_center()
            .justify_end()
            .text_size(px(13.0))
            .text_color(rgb(palette.expression))
            .child(format!("{category_label} ▾"))
            .on_click(cx.listener(|this, _, _, cx| this.convert_next_category(cx)));
        // A plain (non-interactive) template; the caller attaches its own
        // `.on_click` so this closure never has to capture `cx` itself.
        let unit_row =
            |id: &'static str, top: f32, value: String, unit_label: String, editable: bool| {
                let value_color = if editable {
                    palette.result
                } else {
                    palette.expression
                };
                div()
                    .id(SharedString::from(id))
                    .absolute()
                    .left(px(keypad::DISPLAY_RIGHT_INSET))
                    .right(px(keypad::DISPLAY_RIGHT_INSET))
                    .top(px(top))
                    .h(px(convert_keypad::ROW_HEIGHT))
                    .flex()
                    .flex_col()
                    .items_end()
                    .justify_center()
                    .gap(px(2.0))
                    .child(
                        div()
                            .id(SharedString::from(format!("{id}-value")))
                            .text_size(px(22.0))
                            .font_weight(FontWeight::LIGHT)
                            .text_color(rgb(value_color))
                            .child(SharedString::from(value)),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("{id}-unit")))
                            .role(Role::Button)
                            .aria_label(SharedString::from(format!("Unit: {unit_label}")))
                            .cursor_pointer()
                            .text_size(px(13.0))
                            .text_color(rgb(palette.expression))
                            .child(SharedString::from(format!("{unit_label} ▾"))),
                    )
            };
        let from_row = unit_row(
            "convert-from",
            convert_keypad::FROM_ROW_TOP,
            self.convert.from_text(),
            self.convert.from_unit().plural.to_owned(),
            true,
        )
        .on_click(cx.listener(|this, _, _, cx| this.convert_next_from_unit(cx)));
        let to_row = unit_row(
            "convert-to",
            convert_keypad::TO_ROW_TOP,
            self.convert.to_text(),
            self.convert.to_unit().plural.to_owned(),
            false,
        )
        .on_click(cx.listener(|this, _, _, cx| this.convert_next_to_unit(cx)));
        div().child(category_row).child(from_row).child(to_row)
    }

    fn render_convert_key(
        &self,
        key: ConvertKey,
        row: usize,
        column: usize,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (x, y) = convert_keypad::key_origin(row, column);
        let (fill, label) = match convert_keypad::key_style(key) {
            convert_keypad::KeyStyle::Function => (palette.function_key, palette.function_label),
            convert_keypad::KeyStyle::Digit => (palette.digit_key, palette.digit_label),
            convert_keypad::KeyStyle::Swap => (palette.operator_key, palette.operator_label),
        };
        let pressed_fill = blend(fill, palette.pressed_overlay);
        let face = match convert_keypad::key_face(key) {
            convert_keypad::KeyFace::Text(text) => div()
                .text_size(px(keypad::LABEL_SIZE))
                .line_height(px(keypad::LABEL_SIZE))
                .text_color(rgb(label))
                .child(text)
                .into_any_element(),
            convert_keypad::KeyFace::Glyph(path) => svg()
                .path(path)
                .size(px(keypad::GLYPH_SIZE))
                .text_color(rgb(label))
                .into_any_element(),
        };
        div()
            .id(SharedString::from(format!(
                "convert-key-{}",
                convert_keypad::key_name(key).replace(' ', "-")
            )))
            .role(Role::Button)
            .aria_label(convert_keypad::key_name(key))
            .absolute()
            .left(px(x))
            .top(px(y))
            .size(px(convert_keypad::KEY_DIAMETER))
            .rounded_full()
            .bg(rgb(fill))
            .border_1()
            .border_color(rgba(palette.rim))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .active(move |style| style.bg(rgb(pressed_fill)))
            .child(face)
            .on_click(cx.listener(move |this, _, _, cx| this.press_convert(key, cx)))
    }
}

/// Round only the visible, settled result. The calculator keeps its full
/// internal value so changing precision never changes the next operation.
fn format_decimal_value(value: f64, places: usize) -> Option<String> {
    if !value.is_finite() || value.abs() >= 1e9 || places > 15 {
        return None;
    }
    let fixed = format!("{value:.places$}");
    let trimmed = if fixed.contains('.') {
        fixed.trim_end_matches('0').trim_end_matches('.')
    } else {
        &fixed
    };
    let trimmed = if trimmed == "-0" { "0" } else { trimmed };
    Some(rmac_calculator::engine::format_entry(trimmed))
}

/// Replace a plain exponent's Unicode superscript digit, and a typeset
/// minus sign used as the subtract operator, with their ASCII equivalents,
/// for the accessible text only (the visible glyphs keep the real
/// superscript and minus). Measured on the Mac (macOS 26.2, 2026-09-25,
/// `tests/behavior/calculator/scientific.json`): Scientific's `x²` shows a
/// raised "2" on screen, but AX reads the display's value as the plain
/// digits "22", not "2²" — Calculator's exponent is a baseline-offset
/// attribute on an ordinary digit, not a distinct character, and AX drops
/// text attributes. Lulo's engine (`scientific.rs`) uses the real Unicode
/// superscript characters for on-screen formula text (`format!("{d}²")`,
/// `format!("{d}³")`) since GPUI has no per-character baseline offset; this
/// flattens the same way only where it was actually measured — `²` and `³`
/// as bare exponents. `¹` is deliberately excluded even though it is also a
/// superscript digit: this codebase only ever uses it inside `⁻¹`
/// (`"sin⁻¹({d})"` and friends), which is not an exponent and has no
/// capture saying it should flatten too; every other superscript glyph
/// (`ʸ`, `ᵧ`, …) is left alone for the same reason.
///
/// Also measured (macOS 26.2, 2026-09-29,
/// `tests/behavior/calculator/percent-chain.json`): Basic's formula display
/// draws the subtract operator as a typeset minus (`−`, U+2212, what
/// `Operator::symbol`/`BinaryOp::symbol` return for on-screen rendering),
/// but AX reads it back as a plain ASCII hyphen-minus (`-`), same as a
/// negative number's sign. Only `−` is flattened here — `×`/`÷` have no
/// bare-ASCII equivalent and are unmeasured.
fn flatten_superscript_digits(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '²' => '2',
            '³' => '3',
            '−' => '-',
            other => other,
        })
        .collect()
}

/// Set exponents and indices with separate GPUI text sizes and baselines.
/// The visual glyphs remain independent of the accessible key name.
fn scientific_label(text: &'static str, size: f32, color: u32) -> AnyElement {
    let part = |value: &'static str, font_size: f32, rise: f32, italic: bool| {
        div()
            .relative()
            .bottom(px(rise))
            .text_size(px(font_size))
            .line_height(px(font_size))
            .text_color(rgb(color))
            .when(italic, |el| el.italic())
            .child(value)
            .into_any_element()
    };
    let segments: Vec<AnyElement> = match text {
        "2nd" => vec![part("2", size, 0.0, false), part("nd", 10.0, 5.0, false)],
        "x²" => vec![part("x", size, 0.0, false), part("2", 11.0, 6.0, false)],
        "x³" => vec![part("x", size, 0.0, false), part("3", 11.0, 6.0, false)],
        "xʸ" => vec![part("x", size, 0.0, false), part("y", 11.0, 6.0, false)],
        "eˣ" => vec![part("e", size, 0.0, true), part("x", 11.0, 6.0, false)],
        "yˣ" => vec![part("y", size, 0.0, false), part("x", 11.0, 6.0, false)],
        "10ˣ" => vec![part("10", size, 0.0, false), part("x", 11.0, 6.0, false)],
        "2ˣ" => vec![part("2", size, 0.0, false), part("x", 11.0, 6.0, false)],
        "²√x" => vec![part("2", 11.0, 7.0, false), part("√x", 22.0, 0.0, false)],
        "³√x" => vec![part("3", 11.0, 7.0, false), part("√x", 22.0, 0.0, false)],
        "ʸ√x" => vec![part("y", 11.0, 7.0, false), part("√x", 22.0, 0.0, false)],
        "1/x" => vec![
            part("1", 12.0, 6.0, false),
            part("⁄", 21.0, 0.0, false),
            part("x", 12.0, -5.0, false),
        ],
        "log₁₀" => vec![part("log", size, 0.0, false), part("10", 11.0, -5.0, false)],
        "log₂" => vec![part("log", size, 0.0, false), part("2", 11.0, -5.0, false)],
        "logᵧ" => vec![part("log", size, 0.0, false), part("y", 11.0, -5.0, false)],
        "e" => vec![part("e", size, 0.0, true)],
        _ => vec![part(text, size, 0.0, false)],
    };
    div()
        .flex()
        .items_center()
        .children(segments)
        .into_any_element()
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

/// UIA-12: the Mac's Calculator stays dark -- dark keys, orange operators
/// -- in both appearances (confirmed against the Mac captures: `keypad::LIGHT`
/// was speculative, "not yet measured from a light-mode capture" per its own
/// doc comment, and following the system's Light appearance here was the
/// bug). `keypad::DARK` is Calculator's one true palette now; `LIGHT` stays
/// for its own `operator_key` parity test.
fn palette() -> Palette {
    keypad::DARK
}

impl Render for CalculatorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = palette();
        let mode = self.mode;
        let mut keys = Vec::with_capacity(50);
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
                        keys.push(
                            self.render_scientific_key(*key, row, column, palette, cx)
                                .into_any_element(),
                        );
                    }
                }
            }
            Mode::Programmer => {
                for (row, cells) in programmer_keypad::LAYOUT.iter().enumerate() {
                    for (column, cell) in cells.iter().enumerate() {
                        if let Some(key) = cell {
                            keys.push(
                                self.render_programmer_key(*key, row, column, palette, cx)
                                    .into_any_element(),
                            );
                        }
                    }
                }
            }
            Mode::Convert => {
                for (row, cells) in convert_keypad::LAYOUT.iter().enumerate() {
                    for (column, cell) in cells.iter().enumerate() {
                        if let Some(key) = cell {
                            keys.push(
                                self.render_convert_key(*key, row, column, palette, cx)
                                    .into_any_element(),
                            );
                        }
                    }
                }
            }
        }
        let prefix: AnyElement = match mode {
            Mode::Basic | Mode::Scientific => self.render_display(palette),
            Mode::Programmer => div()
                .child(self.render_base_strip(palette, cx))
                .child(self.render_programmer_display(palette))
                .into_any_element(),
            Mode::Convert => self.render_convert_header(palette, cx).into_any_element(),
        };
        // The Mac shows a small persistent "Rad" label above the keypad
        // whenever radians is active — separate from the toggle key itself,
        // which always names the *other* mode (see `scientific_keypad`'s
        // doc comment).
        let show_angle_indicator = mode == Mode::Scientific
            && self.scientific.angle_mode() == scientific::AngleMode::Radians;
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
            .on_action(cx.listener(|this, _: &ShowProgrammer, window, cx| {
                this.set_mode(Mode::Programmer, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowConvert, window, cx| {
                this.set_mode(Mode::Convert, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleRpnMode, _, cx| this.toggle_rpn_mode(cx)))
            .on_action(cx.listener(|_, _: &ShowMathsNotes, _, cx| {
                let main = cx.entity();
                cx.defer(move |cx| crate::maths_notes::show(main, cx));
            }))
            .on_action(cx.listener(|this, _: &ShowHistory, _, cx| this.toggle_history(cx)))
            .on_action(cx.listener(|this, _: &ToggleThousandsSeparator, _, cx| {
                this.toggle_thousands_separator(cx);
            }))
            .on_action(
                cx.listener(|this, _: &DecimalPlaces0, _, cx| this.set_decimal_places(0, cx)),
            )
            .on_action(
                cx.listener(|this, _: &DecimalPlaces1, _, cx| this.set_decimal_places(1, cx)),
            )
            .on_action(
                cx.listener(|this, _: &DecimalPlaces2, _, cx| this.set_decimal_places(2, cx)),
            )
            .on_action(
                cx.listener(|this, _: &DecimalPlaces3, _, cx| this.set_decimal_places(3, cx)),
            )
            .on_action(
                cx.listener(|this, _: &DecimalPlaces4, _, cx| this.set_decimal_places(4, cx)),
            )
            .on_action(
                cx.listener(|this, _: &DecimalPlaces5, _, cx| this.set_decimal_places(5, cx)),
            )
            .on_action(
                cx.listener(|this, _: &DecimalPlaces6, _, cx| this.set_decimal_places(6, cx)),
            )
            .on_action(
                cx.listener(|this, _: &DecimalPlaces7, _, cx| this.set_decimal_places(7, cx)),
            )
            .on_action(
                cx.listener(|this, _: &DecimalPlaces8, _, cx| this.set_decimal_places(8, cx)),
            )
            .on_action(
                cx.listener(|this, _: &DecimalPlaces9, _, cx| this.set_decimal_places(9, cx)),
            )
            .on_action(
                cx.listener(|this, _: &DecimalPlaces10, _, cx| this.set_decimal_places(10, cx)),
            )
            .on_action(
                cx.listener(|this, _: &DecimalPlaces11, _, cx| this.set_decimal_places(11, cx)),
            )
            .on_action(
                cx.listener(|this, _: &DecimalPlaces12, _, cx| this.set_decimal_places(12, cx)),
            )
            .on_action(
                cx.listener(|this, _: &DecimalPlaces13, _, cx| this.set_decimal_places(13, cx)),
            )
            .on_action(
                cx.listener(|this, _: &DecimalPlaces14, _, cx| this.set_decimal_places(14, cx)),
            )
            .on_action(
                cx.listener(|this, _: &DecimalPlaces15, _, cx| this.set_decimal_places(15, cx)),
            )
            .on_action(cx.listener(|_, _: &CloseWindow, _, cx| cx.quit()))
            .on_action(cx.listener(|_, _: &EnterFullScreen, _, _| {}))
            .on_action(cx.listener(|_, _: &rmac_ui::RequestClose, _, cx| cx.quit()))
            .relative()
            .w(px(window_width))
            .h(px(window_height))
            .rounded(px(mac::radius_window_toolbar()))
            .overflow_hidden()
            .bg(rgb(palette.window))
            .font_features(mac::tabular_font_features())
            .child(self.render_toolbar(palette, window, cx))
            .child(prefix)
            .children(keys)
            .when(show_angle_indicator, |el| {
                el.child(
                    div()
                        .absolute()
                        .left(px(scientific_keypad::KEYPAD_LEFT))
                        .top(px(scientific_keypad::ANGLE_INDICATOR_TOP))
                        .text_size(px(scientific_keypad::ANGLE_INDICATOR_SIZE))
                        .text_color(rgb(palette.expression))
                        .child("Rad"),
                )
            })
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
    use super::{blend, flatten_superscript_digits, format_decimal_value};

    #[test]
    fn decimal_places_round_display_without_changing_value() {
        let value = 1.0 / 3.0;
        assert_eq!(format_decimal_value(value, 2).as_deref(), Some("0.33"));
        assert_eq!(
            format_decimal_value(value, 8).as_deref(),
            Some("0.33333333")
        );
        assert_eq!(
            format_decimal_value(value, 15).as_deref(),
            Some("0.333333333333333")
        );
        assert_eq!(format_decimal_value(1234.5, 2).as_deref(), Some("1,234.5"));
        assert_eq!(format_decimal_value(-0.0001, 0).as_deref(), Some("0"));
    }

    #[test]
    fn blend_mixes_overlay_by_alpha() {
        assert_eq!(blend(0x000000, 0xFFFFFF00), 0x000000);
        assert_eq!(blend(0x000000, 0xFFFFFFFF), 0xFFFFFF);
        assert_eq!(blend(0x000000, 0xFFFFFF80), 0x808080);
        assert_eq!(blend(0xFF9200, 0x00000000), 0xFF9200);
    }

    #[test]
    fn flatten_superscript_digits_matches_the_macs_measured_ax_text() {
        // Measured: Scientific's "2²" (a raised "2" on screen) reads as the
        // plain digits "22" to AX, not "2²".
        assert_eq!(flatten_superscript_digits("2²"), "22");
        assert_eq!(flatten_superscript_digits("1,024³"), "1,0243");
        // Left alone: unmeasured superscript glyphs, and plain text.
        assert_eq!(flatten_superscript_digits("sin⁻¹(1)"), "sin⁻¹(1)");
        assert_eq!(flatten_superscript_digits("42"), "42");
    }
}
