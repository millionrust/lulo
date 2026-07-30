//! Detail-pane dispatch and shared category framing.

use super::*;

impl Settings {
    pub(super) fn render_detail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        debug_assert!(category_has_dedicated_renderer(
            self.current().name.as_ref()
        ));
        let content: Div = if let Some(sub) = self.nav.last().cloned() {
            self.render_subpage(&sub, cx)
        } else {
            match self.current().name.as_ref() {
                "Wi-Fi" => self.render_wifi(cx),
                "Bluetooth" => self.render_bluetooth(cx),
                "General" => self.render_general(cx),
                "Appearance" => self.render_appearance(cx),
                "Notifications" => self.render_notifications(cx),
                "Focus" => self.render_focus(cx),
                "Lock Screen" => self.render_lock_screen(cx),
                "Sound" => self.render_sound(cx),
                "Keyboard" => self.render_keyboard(cx),
                "Mouse" => self.render_mouse(cx),
                "Trackpad" => self.render_trackpad(cx),
                "Battery" => self.render_battery(cx),
                "Displays" => self.render_displays(cx),
                "Date & Time" => self.render_date_time(cx),
                "Language & Region" => self.render_language_region(cx),
                "Login Items" => self.render_login_items(cx),
                "Sharing" => self.render_sharing(cx),
                "Accessibility" => self.render_accessibility(cx),
                "Privacy & Security" => self.render_privacy_security(cx),
                "Network" => self.render_network(cx),
                "VPN" => self.render_vpn(cx),
                "Desktop & Dock" => self.render_desktop_dock(cx),
                "Spotlight" => self.render_spotlight(cx),
                "Wallpaper" => self.render_wallpaper(cx),
                _ => self.render_unregistered_category(),
            }
        };

        div()
            .id("detail-scroll")
            .flex_1()
            .h_full()
            .bg(pane_bg())
            .overflow_y_scroll()
            .child(
                div()
                    .max_w(px(560.0))
                    .mx_auto()
                    .px_5()
                    .pb_8()
                    .when(self.system_data_loading, |el| {
                        el.child(
                            Progress::indeterminate()
                                .label("Loading system information…")
                                .mb_3(),
                        )
                    })
                    .child(content),
            )
    }

    pub(super) fn render_hero(&self) -> Div {
        let cat = self.current();
        div()
            .v_flex()
            .items_center()
            .gap_2()
            .pt_6()
            .pb_5()
            .child(tile(cat.icon, cat.color, 64.0))
            .child(
                div()
                    .text_size(rmac_ui::text_px(22.0))
                    .font_weight(rmac_ui::mac::BOLD)
                    .text_color(label())
                    .child(cat.name.clone()),
            )
            .child(
                div()
                    .max_w(px(440.0))
                    .text_center()
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(secondary())
                    .child(cat.desc.clone()),
            )
    }

    pub(super) fn pane(&self, cards: Vec<Div>) -> Div {
        div().v_flex().child(self.render_hero()).children(cards)
    }

    pub(super) fn render_unregistered_category(&self) -> Div {
        self.pane(vec![note_card(
            "This category is not registered with a System Settings renderer. It does not read or change system settings.",
        )])
    }
    pub(super) fn render_subpage(&self, sub: &SubPage, cx: &Context<Self>) -> Div {
        let (title, body): (SharedString, Div) = match sub {
            SubPage::About => ("About".into(), self.about_body(cx)),
            SubPage::SoftwareUpdate => ("Software Update".into(), self.software_update_body(cx)),
            SubPage::Storage => ("Storage".into(), self.storage_body(cx)),
            SubPage::NotificationApp { app_id } => (
                self.application_identity(app_id)
                    .map(|identity| identity.name.clone())
                    .unwrap_or_else(|| app_id.clone())
                    .into(),
                self.notification_app_body(app_id, cx),
            ),
            SubPage::FocusMode { mode_id } => {
                let title = rmac_focus::ModeId::parse(mode_id)
                    .ok()
                    .and_then(|mode_id| {
                        self.focus_policy_config
                            .as_ref()
                            .and_then(|configuration| configuration.mode(&mode_id))
                            .map(|mode| mode.name().to_owned())
                    })
                    .unwrap_or_else(|| "Focus".into());
                (title.into(), self.focus_mode_body(mode_id, cx))
            }
            SubPage::FocusSchedule { schedule_id } => {
                ("Schedule".into(), self.focus_schedule_body(schedule_id, cx))
            }
        };

        let header = div()
            .v_flex()
            .items_center()
            .gap_1()
            .pt_6()
            .pb_4()
            .child(
                div()
                    .text_size(rmac_ui::text_px(20.0))
                    .font_weight(rmac_ui::mac::BOLD)
                    .text_color(label())
                    .child(title),
            )
            .child(
                div()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(secondary())
                    .child("‹ Back, or press ⌘["),
            );

        div().v_flex().child(header).child(body)
    }
}
