//! Maths Notes… (View ▸ Maths Notes…, ⌥⌘M): a simple multi-line expression
//! notebook, backed by the pure evaluator in `rmac_calculator::mathnotes`
//! (CALC-06).
//!
//! Each line is evaluated independently and can refer to names assigned by
//! earlier lines; results show in a read-only column to the right, one row
//! per text line, updating as you type. This is deliberately the "simple
//! maths-notes window that evaluates lines" the Calculator parity row asks
//! for, not the Mac's fuller Soulver-derived feature (unit-aware arithmetic,
//! running totals, sums over a block) — see `mathnotes.rs`'s doc comment.

use gpui::{
    div, px, App, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, Window, WindowHandle,
};
use rmac_ui::{mac, InputEvent, InputState, Root, TextField};

use crate::view::CalculatorView;

const WIDTH: f32 = 440.0;
const HEIGHT: f32 = 460.0;
const RESULTS_WIDTH: f32 = 150.0;
const LINE_HEIGHT: f32 = 20.0;

thread_local! {
    static OPEN: std::cell::Cell<Option<WindowHandle<Root>>> = const { std::cell::Cell::new(None) };
}

/// Open Maths Notes, or bring the existing window forward (one at a time,
/// like Weather's Settings… window).
pub(crate) fn show(main: Entity<CalculatorView>, cx: &mut App) {
    if let Some(handle) = OPEN.with(std::cell::Cell::get) {
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            return;
        }
    }
    let options = rmac_ui::window_options_for_app(rmac_ui::app_id::CALCULATOR, WIDTH, HEIGHT, cx);
    match cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        window.set_window_title("Maths Notes");
        let view = cx.new(|cx| MathsNotesView::new(main, window, cx));
        let focus = view.read(cx).focus.clone();
        window.focus(&focus, cx);
        cx.new(|cx| Root::new(view, window, cx))
    }) {
        Ok(handle) => OPEN.with(|open| open.set(Some(handle))),
        Err(error) => eprintln!("rmac-calculator: could not open Maths Notes: {error}"),
    }
}

struct MathsNotesView {
    focus: FocusHandle,
    input: Entity<InputState>,
    results: Vec<String>,
    /// Kept alive only so Maths Notes shares the Calculator app's lifetime;
    /// it does not read or write the main window's value.
    _main: Entity<CalculatorView>,
}

impl MathsNotesView {
    fn new(main: Entity<CalculatorView>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        rmac_ui::observe_window_state(rmac_ui::app_id::CALCULATOR, window, cx);
        // No soft wrap: one text line must stay one visual row so the
        // results column lines up with it.
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder("rent = 1200\nrent * 12")
        });
        cx.subscribe(&input, |this: &mut Self, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.recompute(cx);
            }
        })
        .detach();
        let focus = cx.focus_handle();
        rmac_ui::register_menu_target(window, &focus, cx);
        let mut view = Self {
            focus,
            input,
            results: Vec::new(),
            _main: main,
        };
        view.recompute(cx);
        view
    }

    fn recompute(&mut self, cx: &mut Context<Self>) {
        let text = rmac_editor::value(&self.input, cx);
        self.results = rmac_calculator::mathnotes::evaluate(&text);
        cx.notify();
    }
}

impl Render for MathsNotesView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let results = self.results.iter().enumerate().map(|(index, text)| {
            div()
                .id(SharedString::from(format!("maths-notes-result-{index}")))
                .h(px(LINE_HEIGHT))
                .line_height(px(LINE_HEIGHT))
                .text_size(px(13.0))
                .text_color(mac::text_tertiary())
                .overflow_hidden()
                .whitespace_nowrap()
                .child(SharedString::from(text.clone()))
                .into_any_element()
        });
        div()
            .id("maths-notes")
            .track_focus(&self.focus)
            .key_context("MathsNotes")
            .size_full()
            .flex()
            .bg(mac::window())
            .child(
                div()
                    .id("maths-notes-input")
                    .flex_1()
                    .min_w(px(0.0))
                    .h_full()
                    .child(
                        TextField::new(&self.input)
                            .large()
                            .h_full()
                            .appearance(false)
                            .text_size(px(13.0))
                            .line_height(px(LINE_HEIGHT))
                            .pl(px(12.0))
                            .pt(px(10.0)),
                    ),
            )
            .child(
                div()
                    .id("maths-notes-results")
                    .w(px(RESULTS_WIDTH))
                    .h_full()
                    .flex()
                    .flex_col()
                    .border_l_1()
                    .border_color(mac::separator())
                    .pl(px(10.0))
                    .pt(px(10.0))
                    .children(results),
            )
    }
}
