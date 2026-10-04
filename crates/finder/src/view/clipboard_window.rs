//! Edit ▸ Show Clipboard: a small window listing exactly what Copy or Cut
//! last put on the clipboard, matching the Mac's own "Clipboard" window.
//! Opens a fresh window each time, the same as `search_info_controller.rs`'s
//! Get Info windows: a snapshot of the clipboard at the moment it's chosen,
//! not a live subscription.

use gpui::{
    div, px, App, AppContext as _, Context, FocusHandle, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window,
};
use gpui_component::StyledExt as _;
use rmac_ui::Root;
use std::path::PathBuf;

const WIDTH: f32 = 360.0;
const HEIGHT: f32 = 320.0;

/// Open a new Clipboard window with the given snapshot of what's on the
/// clipboard right now.
pub(super) fn show(paths: Vec<PathBuf>, cut: bool, cx: &mut App) {
    let options = rmac_ui::window_options_for_app(rmac_ui::app_id::FILES, WIDTH, HEIGHT, cx);
    let opened = cx.open_window(options, move |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        let view = cx.new(|cx| ClipboardView::new(paths, cut, window, cx));
        cx.new(|cx| Root::new(view, window, cx))
    });
    if let Err(error) = opened {
        eprintln!("rmac-files: could not open Show Clipboard: {error}");
    }
}

struct ClipboardView {
    focus: FocusHandle,
    paths: Vec<PathBuf>,
    cut: bool,
}

impl ClipboardView {
    fn new(paths: Vec<PathBuf>, cut: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        window.set_window_title("Clipboard");
        Self {
            focus: cx.focus_handle(),
            paths,
            cut,
        }
    }
}

fn clipboard_summary(count: usize, cut: bool) -> String {
    match count {
        0 => "The Clipboard is empty".to_owned(),
        1 if cut => "1 item ready to be moved".to_owned(),
        1 => "1 item ready to be pasted".to_owned(),
        _ if cut => format!("{count} items ready to be moved"),
        _ => format!("{count} items ready to be pasted"),
    }
}

impl Render for ClipboardView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus)
            .size_full()
            .v_flex()
            .bg(rmac_ui::mac::window())
            .child(rmac_ui::title_bar_content(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rmac_ui::mac::text_secondary())
                    .child("Clipboard"),
            ))
            .child(
                div()
                    .px_3()
                    .py_2()
                    .text_size(px(11.0))
                    .text_color(rmac_ui::mac::text_secondary())
                    .child(clipboard_summary(self.paths.len(), self.cut)),
            )
            .child(
                div()
                    .id("clipboard-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .children(self.paths.iter().map(|path| {
                        let name: SharedString = path
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.display().to_string())
                            .into();
                        div()
                            .px_3()
                            .py_1()
                            .text_size(px(12.0))
                            .text_color(rmac_ui::mac::text())
                            .child(name)
                    })),
            )
    }
}

impl super::FinderView {
    /// Edit ▸ Show Clipboard.
    pub(super) fn show_clipboard(&mut self, cx: &mut Context<Self>) {
        show(self.clipboard.clone(), self.clip_cut, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::clipboard_summary;

    #[test]
    fn summary_describes_an_empty_clipboard() {
        assert_eq!(clipboard_summary(0, false), "The Clipboard is empty");
        assert_eq!(clipboard_summary(0, true), "The Clipboard is empty");
    }

    #[test]
    fn summary_distinguishes_copy_from_cut() {
        assert_eq!(clipboard_summary(1, false), "1 item ready to be pasted");
        assert_eq!(clipboard_summary(1, true), "1 item ready to be moved");
        assert_eq!(clipboard_summary(3, false), "3 items ready to be pasted");
        assert_eq!(clipboard_summary(3, true), "3 items ready to be moved");
    }
}
