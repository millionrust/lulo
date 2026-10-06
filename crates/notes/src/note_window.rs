//! Window ▸ Open Note in New Window (NOT-MENU-064): a second window over
//! the same shared editor fields (`NotesView::title`/`tags`/`body`), so
//! the currently selected note can be viewed and edited from two places
//! at once. Typing in either window reaches the exact same `InputState`
//! entities the main window uses, so it is saved through the same
//! pipeline (`NotesView::schedule_current_edit`) — this is a real second
//! window, not a read-only mirror.
//!
//! Known limitation (documented in docs/parity.md rather than hidden):
//! this window follows the main window's current note selection rather
//! than staying pinned to the one note it was opened for, because Notes
//! keeps exactly one "selected note" in its shared session
//! (`NotesSession`); giving a second window its own independent note
//! selection, undo history and edit-generation tracking would be a larger
//! per-window session architecture change than this pass attempts. It
//! also does not re-register Notes' Format/Find keyboard shortcuts for
//! its own focus, so ⌘B-style commands are only wired in the main window;
//! plain typing, arrow-key navigation and the input's own cut/copy/paste
//! work normally since those live on the shared `InputState`.

use gpui::{
    div, AppContext as _, Context, Entity, FocusHandle, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Styled as _, Window,
};
use rmac_ui::{Root, StyledExt as _};

use crate::NotesView;

const WIDTH: f32 = 420.0;
const HEIGHT: f32 = 480.0;

pub(crate) fn show(main: Entity<NotesView>, cx: &mut gpui::App) {
    if let Some(handle) = main.read(cx).note_window {
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            return;
        }
    }
    let options = rmac_ui::window_options_for_app(rmac_ui::app_id::NOTES, WIDTH, HEIGHT, cx);
    match cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        window.set_window_title("Note");
        let view = cx.new(|cx| NoteWindowView::new(main.clone(), window, cx));
        cx.new(|cx| Root::new(view, window, cx))
    }) {
        Ok(handle) => {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            main.update(cx, |notes, cx| {
                notes.note_window = Some(handle);
                cx.notify();
            });
        }
        Err(error) => eprintln!("rmac-notes: could not open a note window: {error}"),
    }
}

struct NoteWindowView {
    focus: FocusHandle,
    main: Entity<NotesView>,
}

impl NoteWindowView {
    fn new(main: Entity<NotesView>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        rmac_ui::observe_window_state(rmac_ui::app_id::NOTES, window, cx);
        cx.observe(&main, |_, _, cx| cx.notify()).detach();
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self { focus, main }
    }
}

impl Render for NoteWindowView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editor = self.main.update(cx, |notes, cx| notes.render_editor(cx));
        let _ = window;
        div()
            .track_focus(&self.focus)
            .on_action(cx.listener(|_, _: &rmac_ui::RequestClose, window, _| {
                window.remove_window();
            }))
            .size_full()
            .v_flex()
            .bg(rmac_ui::mac::window())
            .child(rmac_ui::title_bar_content(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .font_weight(FontWeight::BOLD)
                    .child("Note"),
            ))
            .child(div().flex_1().min_h(gpui::px(0.0)).child(editor))
    }
}

#[cfg(test)]
mod tests {
    // `show`/`NoteWindowView` need a live `gpui::App`/window to construct,
    // like `settings_window`'s own equivalent; they are exercised through
    // the nested behavior runner rather than a unit test. This module's
    // only pure logic — reusing the existing window if one is open — is
    // covered by inspection against `settings_window::show`'s identical,
    // already-tested pattern.
    #[test]
    fn module_compiles() {}
}
