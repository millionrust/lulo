//! View ▸ Sample Process (MON-10/MON-14, MON-MENU-031): a fresh window per
//! sample showing the exact report text, the same "open a snapshot window
//! each time" pattern `rmac-files`' Show Clipboard uses.

use gpui::{
    div, px, App, AppContext as _, Context, FocusHandle, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window,
};
use rmac_ui::{mac, Root, StyledExt as _};

use crate::sampling_report::{format_report, SampleReport};

const WIDTH: f32 = 520.0;
const HEIGHT: f32 = 420.0;

pub(crate) fn show(report: SampleReport, cx: &mut App) {
    // Not `window_options_for_app`: this snapshot window would then inherit
    // whatever size the main Activity Monitor window last saved under the
    // same app_id (the UIA-06/UIA-09 window-geometry-key bug).
    let options =
        rmac_ui::window_options_for_panel(rmac_ui::app_id::SYSTEM_MONITOR, WIDTH, HEIGHT, cx);
    let opened = cx.open_window(options, move |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        let view = cx.new(|cx| SampleView::new(report, window, cx));
        cx.new(|cx| Root::new(view, window, cx))
    });
    if let Err(error) = opened {
        eprintln!("rmac-system-monitor: could not open Sample Process: {error}");
    }
}

struct SampleView {
    focus: FocusHandle,
    title: String,
    text: String,
}

impl SampleView {
    fn new(report: SampleReport, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let title = format!("Sample of {}", report.process_name);
        window.set_window_title(&title);
        Self {
            focus: cx.focus_handle(),
            title,
            text: format_report(&report),
        }
    }
}

impl Render for SampleView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus)
            .size_full()
            .v_flex()
            .bg(mac::window())
            .child(rmac_ui::title_bar_content(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(mac::text_secondary())
                    .child(self.title.clone()),
            ))
            .child(
                div()
                    .id("sample-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .p_3()
                    .v_flex()
                    .text_size(px(11.0))
                    .font_family(rmac_ui::MONO_FONT)
                    .text_color(mac::text())
                    .children(
                        self.text
                            .lines()
                            .map(|line| div().child(line.to_string()))
                            .collect::<Vec<_>>(),
                    ),
            )
    }
}
