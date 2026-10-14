//! The "Archive Utility Settings" window, at the Mac's 476 × 321 with its
//! rows in the Mac's order: labels right-aligned in a 127 pt column, the
//! controls from x = 152 (`tests/robot/mac/Archive Utility.json`).

use std::cell::Cell;
use std::path::PathBuf;

use gpui::{
    div, px, App, AppContext as _, Context, FocusHandle, InteractiveElement as _, IntoElement,
    ParentElement as _, PathPromptOptions, Render, SharedString, Styled as _, Window, WindowHandle,
};
use rmac_ui::{mac, Checkbox, PopUpButton, PopupMenuItem, Root, StyledExt as _};

use crate::settings::{Afterwards, Settings};
use crate::{ArchiveSettings, CloseWindow, APP_ID};

const SIZE: (f32, f32) = (476.0, 321.0);
const LABEL_WIDTH: f32 = 127.0;

thread_local! {
    static OPEN: Cell<Option<WindowHandle<Root>>> = const { Cell::new(None) };
}

pub fn show(cx: &mut App) {
    if let Some(handle) = OPEN.with(Cell::get) {
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            return;
        }
    }
    let options = rmac_ui::window_options_for_panel_with_title(
        APP_ID,
        "Archive Utility Settings",
        SIZE.0,
        SIZE.1,
        cx,
    );
    match cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        let view = cx.new(|cx| SettingsView::new(window, cx));
        let focus = view.read(cx).focus.clone();
        window.focus(&focus, cx);
        cx.new(|cx| Root::new(view, window, cx))
    }) {
        Ok(handle) => OPEN.with(|open| open.set(Some(handle))),
        Err(error) => eprintln!("rmac-archive-utility: could not open Settings: {error}"),
    }
    cx.activate(true);
}

/// Which "Save … into" pop-up a folder choice is for.
#[derive(Clone, Copy)]
enum Destination {
    Expanded,
    Archives,
}

struct SettingsView {
    focus: FocusHandle,
}

impl SettingsView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_global::<ArchiveSettings>(|_, cx| cx.notify())
            .detach();
        let focus = cx.focus_handle();
        // Menu commands reach this window while it is key.
        rmac_ui::register_menu_target(window, &focus, cx);
        Self { focus }
    }
}

fn change(cx: &mut App, update: impl FnOnce(&mut Settings)) {
    let mut settings = cx.global::<ArchiveSettings>().0.clone();
    update(&mut settings);
    let saved = settings.clone();
    cx.set_global(ArchiveSettings(settings));
    cx.background_executor()
        .spawn(async move {
            if let Err(error) = crate::settings::save(&saved) {
                eprintln!("rmac-archive-utility: could not save Settings: {error}");
            }
        })
        .detach();
}

/// Other…: a folder chosen in the Open panel becomes the destination.
fn choose_folder(destination: Destination, cx: &mut App) {
    let receiver = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Choose".into()),
    });
    cx.spawn(async move |cx| {
        let Ok(Ok(Some(paths))) = receiver.await else {
            return;
        };
        let Some(folder) = paths.into_iter().next() else {
            return;
        };
        cx.update(|cx| {
            change(cx, |settings| match destination {
                Destination::Expanded => settings.expand_into = Some(folder),
                Destination::Archives => settings.archive_into = Some(folder),
            })
        });
    })
    .detach();
}

fn folder_name(folder: &std::path::Path) -> String {
    folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| folder.display().to_string())
}

fn row(label: &'static str, control: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .min_h(px(26.0))
        .child(
            div()
                .w(px(LABEL_WIDTH))
                .flex_none()
                .text_right()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(mac::text())
                .child(label),
        )
        .child(control)
}

/// A checkbox under the pop-ups, aligned with them.
fn indented(control: impl IntoElement) -> impl IntoElement {
    div().pl(px(LABEL_WIDTH + 6.0)).child(control)
}

fn destination_popup(
    id: &'static str,
    same_folder: &'static str,
    current: Option<PathBuf>,
    destination: Destination,
) -> impl IntoElement {
    let label: SharedString = current
        .as_deref()
        .map(|folder| format!("into {}", folder_name(folder)))
        .unwrap_or_else(|| same_folder.to_owned())
        .into();
    PopUpButton::new(id, label)
        .w(px(232.0))
        .dropdown_menu(move |menu, _, _| {
            let mut menu = menu.item(
                PopupMenuItem::new(same_folder)
                    .checked(current.is_none())
                    .on_click(move |_, _, cx| {
                        change(cx, |settings| match destination {
                            Destination::Expanded => settings.expand_into = None,
                            Destination::Archives => settings.archive_into = None,
                        })
                    }),
            );
            if let Some(folder) = current.clone() {
                menu = menu.item(PopupMenuItem::new(folder_name(&folder)).checked(true));
            }
            menu.separator().item(
                PopupMenuItem::new("Other…")
                    .on_click(move |_, _, cx| choose_folder(destination, cx)),
            )
        })
}

fn afterwards_popup(
    id: &'static str,
    current: Afterwards,
    label: fn(Afterwards) -> &'static str,
    set: fn(&mut Settings, Afterwards),
) -> impl IntoElement {
    PopUpButton::new(id, label(current))
        .w(px(172.0))
        .dropdown_menu(move |menu, _, _| {
            Afterwards::ALL.iter().fold(menu, |menu, &choice| {
                menu.item(
                    PopupMenuItem::new(label(choice))
                        .checked(choice == current)
                        .on_click(move |_, _, cx| change(cx, |settings| set(settings, choice))),
                )
            })
        })
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = cx.global::<ArchiveSettings>().0.clone();
        div()
            .id("archive-utility-settings")
            .key_context("ArchiveUtilitySettings")
            .track_focus(&self.focus)
            .on_action(|_: &CloseWindow, window, _| window.remove_window())
            .on_action(|_: &rmac_ui::RequestClose, window, _| window.remove_window())
            .size_full()
            .bg(mac::window())
            .font_family(rmac_ui::UI_FONT)
            .pt(px(48.0))
            .px(px(19.0))
            .v_flex()
            .gap(px(8.0))
            .child(row(
                "Save expanded files:",
                destination_popup(
                    "archive-settings-expand-into",
                    "into same folder as archive",
                    settings.expand_into.clone(),
                    Destination::Expanded,
                ),
            ))
            .child(row(
                "After expanding:",
                afterwards_popup(
                    "archive-settings-after-expanding",
                    settings.after_expanding,
                    Afterwards::expanding_label,
                    |settings, choice| settings.after_expanding = choice,
                ),
            ))
            .child(indented(
                Checkbox::new("archive-settings-reveal-expanded")
                    .label("Reveal expanded item(s) in Files")
                    .checked(settings.reveal_expanded)
                    .on_change(|value, _, cx| {
                        let value = *value;
                        change(cx, |settings| settings.reveal_expanded = value)
                    }),
            ))
            .child(indented(
                Checkbox::new("archive-settings-keep-expanding")
                    .label("Keep expanding if possible")
                    .checked(settings.keep_expanding)
                    .on_change(|value, _, cx| {
                        let value = *value;
                        change(cx, |settings| settings.keep_expanding = value)
                    }),
            ))
            .child(div().h(px(18.0)))
            .child(row(
                "Save archives:",
                destination_popup(
                    "archive-settings-archive-into",
                    "into same folder as originals",
                    settings.archive_into.clone(),
                    Destination::Archives,
                ),
            ))
            .child(row(
                "After archiving:",
                afterwards_popup(
                    "archive-settings-after-archiving",
                    settings.after_archiving,
                    Afterwards::archiving_label,
                    |settings, choice| settings.after_archiving = choice,
                ),
            ))
            .child(indented(
                Checkbox::new("archive-settings-reveal-archive")
                    .label("Reveal archive in Files")
                    .checked(settings.reveal_archive)
                    .on_change(|value, _, cx| {
                        let value = *value;
                        change(cx, |settings| settings.reveal_archive = value)
                    }),
            ))
    }
}
