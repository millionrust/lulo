//! Notes ▸ Settings: controls backed by the library's sort order and the
//! editor's current text scale, plus the session-only preferences
//! documented on `NotesView`'s own fields (`light_background_default`,
//! `auto_sort_ticked_items`) and the locked-notes password
//! (`view_model::LockDialog`).
//!
//! The Mac's "New notes start with: Title/Heading/Body" is deliberately
//! not offered at all (see NOTES-13 in docs/parity.md): Notes keeps the
//! title in its own field rather than the Mac's single first line, so
//! this would have to style the *body's* first line instead — and doing
//! that for real means visibly inserting a Markdown heading marker
//! (`# `/`## `) the moment a brand-new note's body first receives text,
//! which is not what a person typing a plain note expects and broke
//! several behaviour scenarios that create a note and check its exact
//! body text (convert-to-text, find-replace, return-in-title). A
//! stored-but-inert version of the control was tried and rejected too:
//! it is a dead control, which the project's own no-stub rule forbids.

use gpui::{
    div, px, App, AppContext as _, Context, Entity, FocusHandle, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Window, WindowHandle,
};
use rmac_notes_store::SortOrder;
use rmac_ui::{Button, Checkbox, Root, Slider, SliderEvent, SliderState, StyledExt as _};

use crate::{NotesView, ShowSettings};

const WIDTH: f32 = 480.0;
const HEIGHT: f32 = 600.0;

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
        Ok(handle) => {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            OPEN.with(|open| open.set(Some(handle)));
        }
        Err(error) => eprintln!("rmac-notes: could not open Settings: {error}"),
    }
}

pub(crate) fn close(cx: &mut App) {
    if let Some(handle) = OPEN.with(std::cell::Cell::take) {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    }
}

struct SettingsView {
    focus: FocusHandle,
    main: Entity<NotesView>,
    text_size_slider: Entity<SliderState>,
}

impl SettingsView {
    fn new(main: Entity<NotesView>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        rmac_ui::observe_window_state(rmac_ui::app_id::NOTES, window, cx);
        cx.observe(&main, |_, _, cx| cx.notify()).detach();
        let focus = cx.focus_handle();
        rmac_ui::register_menu_target(window, &focus, cx);
        let initial_zoom = main.read(cx).note_zoom;
        let text_size_slider = cx.new(|_| {
            SliderState::new()
                .min(-5.0)
                .max(12.0)
                .step(1.0)
                .default_value(f32::from(initial_zoom))
        });
        cx.subscribe(&text_size_slider, |this, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(value) = event {
                this.set_zoom(value.start().round() as i8, cx);
            }
        })
        .detach();
        Self {
            focus,
            main,
            text_size_slider,
        }
    }

    fn set_sort(&mut self, order: SortOrder, cx: &mut Context<Self>) {
        self.main.update(cx, |notes, cx| notes.set_sort(order, cx));
        cx.notify();
    }

    fn change_zoom(&mut self, delta: i8, window: &mut Window, cx: &mut Context<Self>) {
        let zoom = self.main.update(cx, |notes, cx| {
            notes.note_zoom = (notes.note_zoom + delta).clamp(-5, 12);
            cx.notify();
            notes.note_zoom
        });
        self.text_size_slider.update(cx, |slider, cx| {
            slider.set_value(f32::from(zoom), window, cx);
        });
        cx.notify();
    }

    /// From the slider's own drag, which already carries the clamped value;
    /// unlike `change_zoom` this never needs to push a value back into the
    /// slider itself.
    fn set_zoom(&mut self, zoom: i8, cx: &mut Context<Self>) {
        self.main.update(cx, |notes, cx| {
            notes.note_zoom = zoom.clamp(-5, 12);
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

    /// Notes ▸ Settings… ▸ Automatically sort ticked items.
    fn set_auto_sort_ticked_items(&mut self, value: bool, cx: &mut Context<Self>) {
        self.main.update(cx, |notes, cx| {
            notes.auto_sort_ticked_items = value;
            cx.notify();
        });
        cx.notify();
    }

    /// Notes ▸ Settings… ▸ Use dark backgrounds for note content: checked
    /// means dark, so `light_background_default` (true = light) is the
    /// checkbox's own inverse.
    fn set_dark_backgrounds(&mut self, checked: bool, cx: &mut Context<Self>) {
        self.main.update(cx, |notes, cx| {
            notes.light_background_default = !checked;
            cx.notify();
        });
        cx.notify();
    }

    fn begin_change_password(&mut self, cx: &mut Context<Self>) {
        self.main
            .update(cx, |notes, cx| notes.begin_change_password(cx));
        cx.notify();
    }

    fn begin_reset_password(&mut self, cx: &mut Context<Self>) {
        self.main
            .update(cx, |notes, cx| notes.begin_reset_password(cx));
        cx.notify();
    }

    /// Notes ▸ Settings… ▸ Locked notes ▸ Help: real guidance, local to
    /// Notes rather than an online help article Lulo has no site to host
    /// (same choice as Help ▸ Using Smart Folders/Using Tags).
    fn show_locked_notes_help(&mut self, cx: &mut Context<Self>) {
        self.main.update(cx, |notes, cx| {
            notes.notes_help = Some(
                "Lock a note with File ▸ Lock Note. Locked notes are encrypted on this PC with \
                 the password you set here, and stay open until you choose Close All Locked \
                 Notes, Notes is idle for 8 minutes, the PC sleeps or the screen locks. A \
                 forgotten password can't be recovered: Reset Password sets a new one for \
                 notes you lock from then on, and notes already locked keep their old one.",
            );
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
        let auto_sort_ticked_items = notes.auto_sort_ticked_items;
        let dark_backgrounds = !notes.light_background_default;
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
                    .id("notes-settings-scroll")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
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
                        div().v_flex().gap_2().child("Default text size:").child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(Button::new("notes-settings-smaller", "−").small().on_click(
                                    cx.listener(|this, _, window, cx| this.change_zoom(-1, window, cx)),
                                ))
                                .child(
                                    div()
                                        .w(px(220.0))
                                        .child(Slider::new(&self.text_size_slider)),
                                )
                                .child(Button::new("notes-settings-larger", "+").small().on_click(
                                    cx.listener(|this, _, window, cx| this.change_zoom(1, window, cx)),
                                ))
                                .child(
                                    div()
                                        .w(px(40.0))
                                        .child(format!("{}%", 100 + zoom as i32 * 10)),
                                ),
                        ),
                    )
                    .child(
                        div()
                            .v_flex()
                            .gap_1()
                            .child(
                                Checkbox::new("notes-settings-group-by-date")
                                    .label("Group notes by date")
                                    .checked(group_by_date)
                                    .on_change(cx.listener(|this, value: &bool, _, cx| {
                                        this.set_group_by_date(*value, cx);
                                    })),
                            )
                            .child(
                                div()
                                    .text_size(rmac_ui::text_px(11.0))
                                    .text_color(rmac_ui::mac::text_secondary())
                                    .child("When sorted by Date Edited or Date Created, group notes by date."),
                            ),
                    )
                    .child(
                        div().v_flex().gap_1().child(
                            Checkbox::new("settings-auto-sort-ticked")
                                .label("Automatically sort ticked items")
                                .checked(auto_sort_ticked_items)
                                .on_change(cx.listener(|this, value: &bool, _, cx| {
                                    this.set_auto_sort_ticked_items(*value, cx);
                                })),
                        )
                        .child(
                            div()
                                .text_size(rmac_ui::text_px(11.0))
                                .text_color(rmac_ui::mac::text_secondary())
                                .child("Automatically move checklist items to the bottom of the list as they are ticked off."),
                        ),
                    )
                    .child(
                        Checkbox::new("settings-dark-backgrounds")
                            .label("Use dark backgrounds for note content")
                            .checked(dark_backgrounds)
                            .on_change(cx.listener(|this, value: &bool, _, cx| {
                                this.set_dark_backgrounds(*value, cx);
                            })),
                    )
                    .child(
                        div()
                            .v_flex()
                            .gap_2()
                            .p_3()
                            .rounded(px(rmac_ui::mac::radius_control()))
                            .bg(rmac_ui::mac::control_fill())
                            .child("Default account:")
                            .child(
                                // Lulo has exactly one notes account (no
                                // iCloud, no Exchange), so the Mac's
                                // account-choosing popup is shown with its
                                // one real value rather than faked choices.
                                Button::new("settings-default-account", "On This PC")
                                    .small()
                                    .disabled(true),
                            )
                            .child("Locked notes:")
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(
                                        Button::new("settings-change-password", "Change Password…")
                                            .small()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.begin_change_password(cx);
                                            })),
                                    )
                                    .child(
                                        Button::new("settings-reset-password", "Reset Password…")
                                            .small()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.begin_reset_password(cx);
                                            })),
                                    )
                                    .child(
                                        Button::new("settings-locked-notes-help", "Help")
                                            .small()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.show_locked_notes_help(cx);
                                            })),
                                    ),
                            )
                            .child(
                                // Lulo has no Apple ID/Keychain password to
                                // offer as the alternative, so this is the
                                // one real, always-selected choice rather
                                // than a faked picker (docs/parity.md).
                                Button::new("settings-use-custom-password", "Use Custom Password")
                                    .small()
                                    .selected(true)
                                    .disabled(true),
                            )
                            .child(
                                Checkbox::new("settings-enable-on-my-mac")
                                    .label("Enable the On My Mac account")
                                    .checked(true)
                                    .disabled(true),
                            )
                            .child(
                                div()
                                    .text_size(rmac_ui::text_px(11.0))
                                    .text_color(rmac_ui::mac::text_secondary())
                                    .child("Notes in On My Mac are stored on this computer. Disabling this account doesn’t affect your other notes."),
                            ),
                    ),
            )
    }
}
