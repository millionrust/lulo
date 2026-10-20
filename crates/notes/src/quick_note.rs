//! Quick Note (NOT-025): a small floating note window, reachable from File
//! ▸ New Quick Note (Lulo has no hot corner or Control Centre Notes applet
//! to open it from). Notes ▸ Settings… offers the Mac's two matching
//! controls (NOT-SETTINGS-001/003): "Always resume to last Quick Note" and
//! its hot-corner variant.
//!
//! The window has its own note pane (`note_pane::NotePane`, NOTES-25): a
//! new Quick Note is created without moving the main window's selection,
//! and resuming shows the last Quick Note only here.

use gpui::{
    div, AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, Window,
};
use rmac_ui::{Root, StyledExt as _};

use crate::note_pane::NotePane;
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
        notes
            .always_resume_quick_note
            .then_some(notes.quick_note_id)
            .flatten()
            .filter(|id| {
                notes.session.snapshot().is_some_and(|snapshot| {
                    snapshot
                        .notes
                        .iter()
                        .any(|note| note.id == *id && !note.deleted)
                })
            })
    };
    if resume_id.is_none() {
        main.update(cx, |notes, cx| notes.create_quick_note(cx));
    }
    // Not `window_options_for_app`: Quick Note would then inherit whatever
    // size the main Notes window last saved under the same app_id (the
    // UIA-06/UIA-09 window-geometry-key bug).
    let options = rmac_ui::window_options_for_panel(rmac_ui::app_id::NOTES, WIDTH, HEIGHT, cx);
    match cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        window.set_window_title("Quick Note");
        let view = cx.new(|cx| QuickNoteWindowView::new(main.clone(), resume_id, window, cx));
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
    main: Entity<NotesView>,
    pane: Entity<NotePane>,
}

impl QuickNoteWindowView {
    fn new(
        main: Entity<NotesView>,
        note: Option<rmac_notes_store::NoteId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // `track_key_window` only: `observe_window_state` would also
        // persist this window's geometry under `app_id::NOTES`, the same
        // key the main window uses (the UIA-06/UIA-09 bug).
        rmac_ui::track_key_window(window, cx);
        let pane = cx.new(|cx| NotePane::new(main.clone(), note, window, cx));
        // A new Quick Note: show it once the library has created it.
        cx.observe_in(&main, window, |this, main, window, cx| {
            let created = main.read(cx).quick_note_id;
            this.pane.update(cx, |pane, cx| {
                if pane.note().is_none() {
                    if let Some(note) = created {
                        pane.show_note(note, window, cx);
                    }
                }
            });
        })
        .detach();
        cx.observe(&pane, |_, _, cx| cx.notify()).detach();
        Self { main, pane }
    }
}

impl Render for QuickNoteWindowView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The note this window shows is "the last Quick Note" to resume.
        if let Some(note) = self.pane.read(cx).note() {
            self.main.update(cx, |notes, _| notes.quick_note_id = Some(note));
        }
        div()
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
            .child(div().flex_1().min_h(gpui::px(0.0)).child(self.pane.clone()))
    }
}
