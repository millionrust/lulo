//! Notes ▸ Settings: controls backed by the library's sort order and the
//! editor's current text scale.

use gpui::{
    div, px, App, AppContext as _, Context, Entity, FocusHandle, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Window,
    WindowHandle,
};
use rmac_notes_store::SortOrder;
use rmac_ui::{Button, Checkbox, Root, StyledExt as _};

use crate::{NotesView, ShowSettings};

const WIDTH: f32 = 456.0;
const HEIGHT: f32 = 310.0;

thread_local! {
    static OPEN: std::cell::Cell<Option<WindowHandle<Root>>> = const { std::cell::Cell::new(None) };
}

pub(crate) fn show(main: Entity<NotesView>, cx: &mut App) {
    if let Some(handle) = OPEN.with(std::cell::Cell::get) {
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
        window.set_window_title("Notes Settings");
        let view = cx.new(|cx| SettingsView::new(main, window, cx));
        let focus = view.read(cx).focus.clone();
        window.focus(&focus, cx);
        cx.new(|cx| Root::new(view, window, cx))
    }) {
        Ok(handle) => OPEN.with(|open| open.set(Some(handle))),
        Err(error) => eprintln!("rmac-notes: could not open Settings: {error}"),
    }
}

struct SettingsView {
    focus: FocusHandle,
    main: Entity<NotesView>,
}

impl SettingsView {
    fn new(main: Entity<NotesView>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        rmac_ui::observe_window_state(rmac_ui::app_id::NOTES, window, cx);
        cx.observe(&main, |_, _, cx| cx.notify()).detach();
        let focus = cx.focus_handle();
        rmac_ui::register_menu_target(window, &focus, cx);
        Self { focus, main }
    }

    fn set_sort(&mut self, order: SortOrder, cx: &mut Context<Self>) {
        self.main.update(cx, |notes, cx| notes.set_sort(order, cx));
        cx.notify();
    }

    fn change_zoom(&mut self, delta: i8, cx: &mut Context<Self>) {
        self.main.update(cx, |notes, cx| {
            notes.note_zoom = (notes.note_zoom + delta).clamp(-5, 12);
            cx.notify();
        });
        cx.notify();
    }

    fn set_group_by_date(&mut self, value: bool, cx: &mut Context<Self>) {
        self.main.update(cx, |notes, cx| {
            notes.group_notes_by_date = value;
            cx.notify();
        });
        cx.notify();
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let notes = self.main.read(cx);
        let sort = notes
            .session
            .snapshot()
            .map(|snapshot| snapshot.sort_order)
            .unwrap_or(SortOrder::Edited);
        let zoom = notes.note_zoom;
        let group_by_date = notes.group_notes_by_date;
        div()
            .track_focus(&self.focus)
            .key_context("Notes")
            .on_action(cx.listener(|_, _: &rmac_ui::RequestClose, window, _| {
                window.remove_window();
            }))
            .on_action(cx.listener(|_, _: &ShowSettings, window, _| window.activate_window()))
            .size_full()
            .v_flex()
            .bg(rmac_ui::mac::window())
            .text_color(rmac_ui::mac::text())
            .child(rmac_ui::title_bar_content(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .font_weight(FontWeight::BOLD)
                    .child("Notes Settings"),
            ))
            .child(
                div()
                    .v_flex()
                    .gap_4()
                    .px_4()
                    .py_4()
                    .child(
                        div().v_flex().gap_2().child("Sort notes by:").child(
                            div().flex().gap_2().children(
                                [
                                    ("Date Edited", SortOrder::Edited),
                                    ("Date Created", SortOrder::Created),
                                    ("Title", SortOrder::Title),
                                ]
                                .into_iter()
                                .map(|(label, order)| {
                                    Button::new(format!("notes-settings-sort-{label}"), label)
                                        .small()
                                        .selected(sort == order)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.set_sort(order, cx);
                                        }))
                                }),
                            ),
                        ),
                    )
                    .child(
                        div().v_flex().gap_2().child("Text size:").child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(Button::new("notes-settings-smaller", "−").small().on_click(
                                    cx.listener(|this, _, _, cx| this.change_zoom(-1, cx)),
                                ))
                                .child(
                                    div()
                                        .w(px(40.0))
                                        .child(format!("{}%", 100 + zoom as i32 * 10)),
                                )
                                .child(Button::new("notes-settings-larger", "+").small().on_click(
                                    cx.listener(|this, _, _, cx| this.change_zoom(1, cx)),
                                )),
                        ),
                    )
                    .child(
                        Checkbox::new("notes-settings-group-by-date")
                            .label("Group notes by date")
                            .checked(group_by_date)
                            .on_change(cx.listener(|this, value: &bool, _, cx| {
                                this.set_group_by_date(*value, cx);
                            })),
                    ),
            )
    }
}
