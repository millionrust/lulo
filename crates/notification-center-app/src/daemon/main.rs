//! The rmac notification authority and its on-screen banners.
//!
//! One process serves `org.freedesktop.Notifications`, the notification
//! portal backend and the Notification Center history, and draws the macOS 26
//! banners for what arrives. The banner session has to consume the service's
//! single event stream and drive the same service handle for dismiss, expiry
//! and actions, so it lives here rather than in the on-demand Center panel.

// Banners are layer-shell surfaces; off Linux `surface::open` is a stub, so
// the drawing code and its host bookkeeping are compiled but never reached.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod host;
#[allow(dead_code)]
#[path = "../model.rs"]
mod model;
#[cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]
mod surface;

fn main() {
    rmac_ui::application()
        .with_assets(gpui_component_assets::Assets)
        // Banners come and go; the service must outlive every window.
        .with_quit_mode(gpui::QuitMode::Explicit)
        .run(|cx: &mut gpui::App| {
            rmac_ui::init_application(cx);
            host::start(cx);
        });
}
