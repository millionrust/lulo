//! The rmac notification authority and its on-screen banners.
//!
//! One process serves `org.freedesktop.Notifications`, the notification
//! portal backend and the Notification Center history, and draws the macOS 26
//! banners for what arrives. The banner session has to consume the service's
//! single event stream and drive the same service handle for dismiss, expiry
//! and actions, so it lives here rather than in the on-demand Center panel.

mod host;
#[allow(dead_code)]
#[path = "../model.rs"]
mod model;
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
