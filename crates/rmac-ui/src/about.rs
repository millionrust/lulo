//! App ▸ About <App>: the standard About panel, as AppKit's
//! `orderFrontStandardAboutPanel` draws it — the app's icon, its name in
//! bold, the version and the copyright line, centred in a small window that
//! only closes.
//!
//! Every fact on it is real: the name is the app's desktop identity, the
//! version is the one this binary was built as, and the copyright line is
//! the project's LICENSE. The panel's geometry has not been measured on the
//! Mac yet (S): the sizes follow TextEdit's panel by eye.

// Opened from the menu bar, which exists only on Linux.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::path::PathBuf;

use gpui::{
    div, img, prelude::*, px, AnyWindowHandle, App, Context, FocusHandle, Global, KeyBinding,
    SharedString, Window,
};

use crate::{mac, text_px, StyledExt as _};

/// S: TextEdit's About panel, by eye.
const WIDTH: f32 = 284.0;
const HEIGHT: f32 = 250.0;
const ICON_EDGE: f32 = 64.0;
/// Resolve the icon at twice its drawn size for a sharp HiDPI image.
const ICON_PIXELS: u32 = 128;
/// From the project's LICENSE.
const COPYRIGHT: &str = "Copyright © 2026 rmac contributors";
const CONTEXT: &str = "RmacAboutPanel";

/// The open panel, so a second About brings it forward instead of opening
/// another.
struct AboutPanelWindow(AnyWindowHandle);

impl Global for AboutPanelWindow {}

/// Keys bound once per process.
struct AboutKeysBound;

impl Global for AboutKeysBound {}

pub(crate) fn show(app_id: &'static str, cx: &mut App) {
    if let Some(handle) = cx.try_global::<AboutPanelWindow>().map(|panel| panel.0) {
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            return;
        }
    }
    if !cx.has_global::<AboutKeysBound>() {
        cx.bind_keys([KeyBinding::new(
            crate::shortcuts::CLOSE.keystroke,
            crate::RequestClose,
            Some(CONTEXT),
        )]);
        cx.set_global(AboutKeysBound);
    }
    let name = rmac_apps::identity::window_title(app_id).unwrap_or(app_id);
    match crate::window::open_panel_window(
        app_id,
        format!("About {name}"),
        WIDTH,
        HEIGHT,
        cx,
        move |window, cx| AboutPanel::new(app_id, name, window, cx),
    ) {
        Ok(handle) => cx.set_global(AboutPanelWindow(handle)),
        Err(error) => eprintln!("{app_id}: could not open the About panel: {error}"),
    }
}

struct AboutPanel {
    name: SharedString,
    icon: Option<PathBuf>,
    focus: FocusHandle,
}

impl AboutPanel {
    fn new(
        app_id: &'static str,
        name: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        // Icon themes live on disk; resolve off the UI thread.
        cx.spawn(async move |this, cx| {
            let icon = cx
                .background_executor()
                .spawn(async move {
                    rmac_apps::ThemedIconResolver::current().resolve(app_id, ICON_PIXELS)
                })
                .await;
            if icon.is_none() {
                eprintln!("{app_id}: no icon found for the About panel");
            }
            // The panel may have closed while the theme was read.
            this.update(cx, |this, cx| {
                this.icon = icon;
                cx.notify();
            })
            .ok();
        })
        .detach();
        Self {
            name: name.into(),
            icon: None,
            focus,
        }
    }
}

impl Render for AboutPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let icon = match &self.icon {
            Some(path) => img(path.clone()).size(px(ICON_EDGE)).into_any_element(),
            None => div().size(px(ICON_EDGE)).into_any_element(),
        };
        div()
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|_, _: &crate::RequestClose, window, _| {
                window.remove_window();
            }))
            .size_full()
            .v_flex()
            .bg(mac::window())
            .text_color(mac::text())
            .child(crate::title_bar_content(div()))
            .child(
                div()
                    .flex_1()
                    .v_flex()
                    .items_center()
                    .px(px(20.0))
                    .child(icon)
                    .child(
                        div()
                            .mt(px(12.0))
                            .text_size(text_px(14.0))
                            .font_weight(mac::BOLD)
                            .child(self.name.clone()),
                    )
                    .child(
                        div()
                            .mt(px(6.0))
                            .text_size(text_px(11.0))
                            .text_color(mac::text_secondary())
                            .child(format!("Version {}", env!("CARGO_PKG_VERSION"))),
                    )
                    .child(
                        div()
                            .mt(px(14.0))
                            .text_size(text_px(11.0))
                            .text_color(mac::text_secondary())
                            .child(COPYRIGHT),
                    ),
            )
    }
}
