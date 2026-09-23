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
                "Menu Bar" => self.render_menu_bar(cx),
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

        // Measured: content 20 in from the detail column (460 wide in the
        // Mac's 723 pt window), starting right under the toolbar.
        div()
            .id(rmac_system_settings::accessibility::DETAIL_ID)
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .overflow_y_scroll()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.sidebar_focused {
                        this.sidebar_focused = false;
                        cx.notify();
                    }
                }),
            )
            .child(
                div()
                    .w_full()
                    .max_w(px(style::DETAIL_CONTENT_WIDTH + 2.0 * style::DETAIL_INSET))
                    .mx_auto()
                    .px(px(style::DETAIL_INSET))
                    .pb(px(style::DETAIL_INSET))
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

    /// macOS 26 opens General and Accessibility with a hero group (large
    /// icon, title and description) and leaves their toolbar untitled; every
    /// other pane starts directly with its first group under a titled toolbar.
    pub(super) fn pane_has_hero(&self) -> bool {
        matches!(self.current().name.as_ref(), "General" | "Accessibility")
    }

    /// General's hero: a 164 pt group with the 52 pt icon 24 from the top,
    /// a 22 pt bold title and the 13 pt description (design-lab/settings.html).
    pub(super) fn render_hero(&self) -> Div {
        let cat = self.current();
        div()
            .v_flex()
            .items_center()
            .min_h(px(style::HERO_HEIGHT))
            .mb(px(style::GROUP_GAP))
            .pt(px(style::HERO_ICON_TOP))
            .pb(px(style::DETAIL_INSET))
            .px(px(style::ROW_PADDING))
            .rounded(px(style::GROUP_RADIUS))
            .bg(card_bg())
            .child(tile(cat.icon, cat.color, style::HERO_ICON))
            .child(
                div()
                    .mt(px(10.0))
                    .text_size(rmac_ui::text_px(style::HERO_TITLE))
                    .line_height(px(26.0))
                    .font_weight(rmac_ui::mac::BOLD)
                    .text_color(label())
                    .child(cat.name.clone()),
            )
            .child(
                div()
                    .max_w(px(style::HERO_TEXT_WIDTH))
                    .text_center()
                    .text_size(rmac_ui::text_px(13.0))
                    .line_height(px(16.0))
                    .text_color(secondary())
                    .child(cat.desc.clone()),
            )
    }

    pub(super) fn pane(&self, cards: Vec<Div>) -> Div {
        div()
            .v_flex()
            .when(self.pane_has_hero(), |pane| pane.child(self.render_hero()))
            .when(!self.pane_has_hero(), |pane| {
                pane.pt(px(style::FIRST_SECTION_TOP))
            })
            .children(cards)
    }

    pub(super) fn render_unregistered_category(&self) -> Div {
        self.pane(vec![note_card(
            "This category is not registered with a System Settings renderer. It does not read or change system settings.",
        )])
    }
    pub(super) fn render_subpage(&self, sub: &SubPage, cx: &Context<Self>) -> Div {
        let body = match sub {
            SubPage::About => self.about_body(cx),
            SubPage::SoftwareUpdate => self.software_update_body(cx),
            SubPage::Storage => self.storage_body(cx),
            SubPage::NotificationApp { app_id } => self.notification_app_body(app_id, cx),
            SubPage::FocusMode { mode_id } => self.focus_mode_body(mode_id, cx),
            SubPage::FocusSchedule { schedule_id } => self.focus_schedule_body(schedule_id, cx),
        };

        // The toolbar carries the subpage title and the back button, as on
        // the Mac; the body starts directly under it.
        div().v_flex().pt(px(style::FIRST_SECTION_TOP)).child(body)
    }
}
