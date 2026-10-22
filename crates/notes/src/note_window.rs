//! Window ▸ Open Note in New Window (NOT-MENU-064): the selected note in a
//! window of its own. The window keeps showing that note, with its own
//! editors, caret and undo (`note_pane::NotePane`, NOTES-25), whatever the
//! main window selects next; opening the same note again brings its
//! window forward.

use gpui::{
    div, AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, Window,
};
use rmac_ui::{Root, StyledExt as _};

use crate::note_pane::NotePane;
use crate::NotesView;

const WIDTH: f32 = 420.0;
const HEIGHT: f32 = 480.0;

pub(crate) fn show(main: Entity<NotesView>, cx: &mut gpui::App) {
    let Some(note_id) = main.read(cx).selected_note_id() else {
        return;
    };
    let existing = main
        .read(cx)
        .note_windows
        .iter()
        .find(|(id, _)| *id == note_id)
        .map(|(_, handle)| *handle);
    if let Some(handle) = existing {
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            return;
        }
    }
    // Not `window_options_for_app`: this window would then inherit
    // whatever size the main Notes window last saved under the same
    // app_id (the UIA-06/UIA-09 window-geometry-key bug).
    let options = rmac_ui::window_options_for_panel(rmac_ui::app_id::NOTES, WIDTH, HEIGHT, cx);
    match cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        window.set_window_title("Note");
        let view = cx.new(|cx| NoteWindowView::new(main.clone(), note_id, window, cx));
        cx.new(|cx| Root::new(view, window, cx))
    }) {
        Ok(handle) => {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            main.update(cx, |notes, cx| {
                notes.note_windows.retain(|(id, _)| *id != note_id);
                notes.note_windows.push((note_id, handle));
                cx.notify();
            });
        }
        Err(error) => eprintln!("rmac-notes: could not open a note window: {error}"),
    }
}

struct NoteWindowView {
    main: Entity<NotesView>,
    pane: Entity<NotePane>,
    shown_title: String,
}

impl NoteWindowView {
    fn new(
        main: Entity<NotesView>,
        note_id: rmac_notes_store::NoteId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // `track_key_window` only: `observe_window_state` would also
        // persist this window's geometry under `app_id::NOTES`, the same
        // key the main window uses (the UIA-06/UIA-09 bug).
        rmac_ui::track_key_window(window, cx);
        let pane = cx.new(|cx| NotePane::new(main.clone(), Some(note_id), window, cx));
        cx.observe(&pane, |_, _, cx| cx.notify()).detach();
        cx.observe(&main, |_, _, cx| cx.notify()).detach();
        Self {
            main,
            pane,
            shown_title: String::new(),
        }
    }

    /// The note's title, or "New Note" for an untitled one.
    fn note_title(&self, cx: &gpui::App) -> String {
        let note_id = self.pane.read(cx).note();
        self.main
            .read(cx)
            .session
            .snapshot()
            .and_then(|snapshot| snapshot.notes.iter().find(|note| Some(note.id) == note_id))
            .map(|note| crate::input_support::display_title(&note.title).to_string())
            .unwrap_or_else(|| "Note".to_owned())
    }
}

impl Render for NoteWindowView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self.note_title(cx);
        if self.shown_title != title {
            window.set_window_title(&title);
            self.shown_title = title.clone();
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
                    .child(title),
            ))
            .child(div().flex_1().min_h(gpui::px(0.0)).child(self.pane.clone()))
    }
}
