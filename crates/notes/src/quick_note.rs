//! Quick Note (NOT-025): a small floating note window, independent of
//! the Mac's hot-corner/Control-Centre entry point (Lulo has neither a
//! hot-corner service nor a Control Centre Notes applet yet), reachable
//! here from File ▸ New Quick Note. Notes ▸ Settings… offers the Mac's
//! two matching controls (NOT-SETTINGS-001/003): "Always resume to last
//! Quick Note" and the hot-corner/shortcut-specific variant of the same
//! choice — both session-only, like every other Notes setting today
//! (NOTES-13).
//!
//! Known limitation (see `note_window.rs` for the identical root cause):
//! Quick Note shares the single `NotesSession` selection with the main
//! window and any Open Note in New Window, so opening or resuming a
//! Quick Note changes what the main window shows too, rather than living
//! in a fully independent note session.

use gpui::{
    div, AppContext as _, Context, Entity, FocusHandle, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Styled as _, Window,
};
use rmac_ui::{Root, StyledExt as _};

use crate::NotesView;

const WIDTH: f32 = 360.0;
const HEIGHT: f32 = 420.0;

pub(crate) fn show(main: Entity<NotesView>, cx: &mut gpui::App) {
    if let Some(handle) = main.read(cx).quick_note_window {
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            return;
        }
    }
    let resume_id = {
        let notes = main.read(cx);
        (notes.always_resume_quick_note || notes.quick_note_hot_corner_resume)
            .then(|| notes.quick_note_id)
            .flatten()
    };
    let options = rmac_ui::window_options_for_app(rmac_ui::app_id::NOTES, WIDTH, HEIGHT, cx);
    match cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        window.set_window_title("Quick Note");
        main.update(cx, |notes, cx| {
            let resumed = if let Some(id) = resume_id {
                notes.select_note(id, window, cx);
                notes.selected_note_id() == Some(id)
            } else {
                false
            };
            if !resumed {
                notes.create_note(cx);
            }
        });
        let view = cx.new(|cx| QuickNoteWindowView::new(main.clone(), window, cx));
        cx.new(|cx| Root::new(view, window, cx))
    }) {
        Ok(handle) => {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            main.update(cx, |notes, cx| {
                notes.quick_note_window = Some(handle);
                cx.notify();
            });
        }
        Err(error) => eprintln!("rmac-notes: could not open Quick Note: {error}"),
    }
}

struct QuickNoteWindowView {
    focus: FocusHandle,
    main: Entity<NotesView>,
}

impl QuickNoteWindowView {
    fn new(main: Entity<NotesView>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        rmac_ui::observe_window_state(rmac_ui::app_id::NOTES, window, cx);
        cx.observe(&main, |_, _, cx| cx.notify()).detach();
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self { focus, main }
    }
}

impl Render for QuickNoteWindowView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // While this window is the one the person is using, remember
        // whatever note it ends up showing (freshly created or resumed)
        // as "the last Quick Note" for a future resume — scoped to this
        // window's own active state so switching notes in the *main*
        // window never overwrites it.
        if window.is_window_active() {
            self.main.update(cx, |notes, _| {
                let current = notes.selected_note_id();
                if current.is_some() {
                    notes.quick_note_id = current;
                }
            });
        }
        let editor = self.main.update(cx, |notes, cx| notes.render_editor(cx));
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
                    .child("Quick Note"),
            ))
            .child(div().flex_1().min_h(gpui::px(0.0)).child(editor))
    }
}

#[cfg(test)]
mod tests {
    // `show`/`QuickNoteWindowView` need a live `gpui::App`/window, like
    // `settings_window`'s equivalent, and are exercised through the
    // nested behavior runner. See `audio_recorder`/`note_window` for the
    // pure logic this crate's other new modules do unit-test.
    #[test]
    fn module_compiles() {}
}
