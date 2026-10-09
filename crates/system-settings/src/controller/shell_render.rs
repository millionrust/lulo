//! Root settings window composition and overlay routing.

use super::*;
impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        rmac_ui::trace_mark("settings_render");
        self.scale_factor = window.scale_factor();
        rmac_ui::set_menu_enabled("system_settings::GoBack", self.can_go_back(), cx);
        rmac_ui::set_menu_enabled("system_settings::GoForward", self.can_go_forward(), cx);
        rmac_ui::set_menu_enabled("system_settings::EnterFullScreen", false, cx);
        for (action, pane) in [
            ("system_settings::ShowBattery", "Battery"),
            ("system_settings::ShowBluetooth", "Bluetooth"),
            ("system_settings::ShowDisplays", "Displays"),
            ("system_settings::ShowTrackpad", "Trackpad"),
            ("system_settings::ShowWifi", "Wi-Fi"),
        ] {
            rmac_ui::set_menu_enabled(action, self.pane_available(pane), cx);
        }
        if !self.focused_once {
            self.focused_once = true;
            // The Mac's own System Settings opens with keyboard focus
            // already on the search field, not the window as a whole.
            let search_focus = self.search.read(cx).focus_handle(cx);
            window.focus(&search_focus, cx);
        }
        let title_subject = self
            .nav
            .last()
            .map(|subpage| self.subpage_title(subpage))
            .unwrap_or_else(|| self.current().name.to_string());
        let native_window_title = rmac_ui::native_window_title(&title_subject, "Settings");
        if self.native_window_title != native_window_title {
            window.set_window_title(&native_window_title);
            self.native_window_title = native_window_title;
        }
        let layout = self.layout(window);
        self.rendered_layout = Some(layout);
        let entity = cx.entity();
        let child_views = self
            .views
            .get_or_insert_with(|| views::SettingsViews::new(&entity, cx));
        let (sidebar_view, pane_view) = (child_views.sidebar.clone(), child_views.pane.clone());
        let cache_views = !window.is_a11y_active();
        self.rendered_banner = self.global_settings_error().cloned();
        let settings_error = self.rendered_banner.as_ref().map(|error| {
            rmac_ui::user_error_message(rmac_ui::ErrorSurface::Settings, error.as_ref(), false)
        });
        let wifi_password_dialog = self.render_wifi_password_dialog(cx);
        let wifi_enterprise_dialog = self.render_wifi_enterprise_dialog(cx);
        let wifi_forget_dialog = self.render_wifi_forget_dialog(cx);
        let bluetooth_pairing_dialog = self.render_bluetooth_pairing_dialog(cx);
        let bluetooth_forget_dialog = self.render_bluetooth_forget_dialog(cx);
        let clock_confirmation_dialog = self.render_clock_confirmation(cx);
        let vpn_import_dialog = self.render_vpn_import_dialog(cx);
        let vpn_secret_clear_dialog = self.render_vpn_secret_clear_dialog(cx);
        let vpn_delete_dialog = self.render_vpn_delete_dialog(cx);
        let update_install_dialog = self.render_update_install_dialog(cx);
        let update_info_sheet = self.render_update_info_sheet(cx);
        let update_auto_sheet = self.render_update_auto_sheet(cx);
        let keyboard_shortcuts_sheet = self.render_keyboard_shortcuts_sheet(cx);
        let internet_account_sheet = self.render_internet_account_sheet(cx);
        let internet_account_delete = self.render_internet_account_delete(cx);
        let users_overlay = self.render_users_overlay(cx);
        let printers_overlay = self.render_printers_overlay(cx);
        div()
            .id(rmac_system_settings::accessibility::ROOT_ID)
            .size_full()
            .relative()
            .flex()
            .font_features(rmac_ui::mac::tabular_font_features())
            .track_focus(&self.focus)
            .key_context("SystemSettings")
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "enter"
                    && this.clock_confirmation.is_some()
                    && !this.clock_setting
                {
                    cx.stop_propagation();
                    this.confirm_clock_change(cx);
                } else if event.keystroke.key == "escape" && this.keyboard_shortcuts_open {
                    cx.stop_propagation();
                    this.close_keyboard_shortcuts(cx);
                } else if event.keystroke.key == "escape" && this.clock_confirmation.is_some() {
                    cx.stop_propagation();
                    this.cancel_clock_confirmation(cx);
                } else if event.keystroke.key == "escape"
                    && this.recent_history_confirmation
                    && !this.recent_history_busy
                {
                    cx.stop_propagation();
                    this.cancel_recent_history_clear(cx);
                } else if event.keystroke.key == "escape" && this.updates_plan.is_some() {
                    cx.stop_propagation();
                    this.cancel_update_plan(cx);
                } else if event.keystroke.key == "escape" && this.updates_auto_sheet {
                    cx.stop_propagation();
                    this.close_update_auto_sheet(cx);
                } else if event.keystroke.key == "escape" && this.updates_info_sheet.is_some() {
                    cx.stop_propagation();
                    this.close_update_info_sheet(cx);
                } else if event.keystroke.key == "escape" && this.wifi_forget_confirmation.is_some()
                {
                    cx.stop_propagation();
                    this.cancel_wifi_forget(cx);
                } else if event.keystroke.key == "escape"
                    && this.bluetooth_forget_confirmation.is_some()
                {
                    cx.stop_propagation();
                    this.cancel_bluetooth_forget(cx);
                } else if event.keystroke.key == "escape" && this.vpn_cancellation.is_some() {
                    cx.stop_propagation();
                    this.cancel_vpn_activation(cx);
                } else if event.keystroke.key == "escape"
                    && this.vpn_import_preview.is_some()
                    && !this.vpn_import_busy
                {
                    cx.stop_propagation();
                    this.finish_vpn_import(false, cx);
                } else if event.keystroke.key == "escape"
                    && this.vpn_secret_preview.is_some()
                    && !this.vpn_secret_busy
                {
                    cx.stop_propagation();
                    this.cancel_vpn_secret_clear(cx);
                } else if event.keystroke.key == "escape"
                    && this.vpn_delete_preview.is_some()
                    && !this.vpn_delete_busy
                {
                    cx.stop_propagation();
                    this.cancel_vpn_delete(cx);
                } else if event.keystroke.key == "escape"
                    && this.vpn_editor.is_some()
                    && !this.vpn_editor_busy
                {
                    cx.stop_propagation();
                    this.cancel_vpn_edit(cx);
                } else if event.keystroke.key == "escape"
                    && this.network_editor.is_some()
                    && !this.network_busy
                {
                    cx.stop_propagation();
                    this.cancel_network_edit(cx);
                } else if !this.search.read(cx).value().trim().is_empty() {
                    let handled = match event.keystroke.key.as_str() {
                        "down" => {
                            let moved = this.move_search_selection(1, cx);
                            if moved {
                                // Matches the Mac: Down hands keyboard focus
                                // to the results list itself (a `Role::List`
                                // an assistive-technology client reports),
                                // not just an internal highlight index while
                                // the search field keeps focus.
                                let results_focus = this.results_focus.clone();
                                window.focus(&results_focus, cx);
                            }
                            moved
                        }
                        "up" => this.move_search_selection(-1, cx),
                        "enter" => this.activate_search_selection(window, cx),
                        "escape" => {
                            this.clear_search(window, cx);
                            true
                        }
                        _ => false,
                    };
                    if handled {
                        cx.stop_propagation();
                        cx.notify();
                    }
                }
            }))
            .on_action(cx.listener(|t, _: &GoBack, window, cx| t.go_back(window, cx)))
            .on_action(cx.listener(|t, _: &GoForward, window, cx| t.go_forward(window, cx)))
            .on_action(cx.listener(|t, action: &NavigateToPane, window, cx| {
                t.navigate_to_pane(&action.pane, window, cx)
            }))
            .on_action(cx.listener(|_, _: &CloseAll, window, cx| {
                window.dispatch_action(Box::new(rmac_ui::RequestClose), cx);
            }))
            .on_action(cx.listener(|_, _: &ArrangeInFront, window, _| {
                window.activate_window();
            }))
            .on_action(
                cx.listener(|t, _: &ShowAbout, window, cx| t.navigate_to_pane("about", window, cx)),
            )
            .on_action(cx.listener(|t, _: &ShowAccessibility, window, cx| {
                t.navigate_to_pane("accessibility", window, cx)
            }))
            .on_action(cx.listener(|t, _: &ShowAppearance, window, cx| {
                t.navigate_to_pane("appearance", window, cx)
            }))
            .on_action(cx.listener(|t, _: &ShowBattery, window, cx| {
                t.navigate_to_pane("battery", window, cx)
            }))
            .on_action(cx.listener(|t, _: &ShowBluetooth, window, cx| {
                t.navigate_to_pane("bluetooth", window, cx)
            }))
            .on_action(cx.listener(|t, _: &ShowDateTime, window, cx| {
                t.navigate_to_pane("date-time", window, cx)
            }))
            .on_action(cx.listener(|t, _: &ShowDesktopDock, window, cx| {
                t.navigate_to_pane("desktop-dock", window, cx)
            }))
            .on_action(cx.listener(|t, _: &ShowDisplays, window, cx| {
                t.navigate_to_pane("displays", window, cx)
            }))
            .on_action(
                cx.listener(|t, _: &ShowFocus, window, cx| t.navigate_to_pane("focus", window, cx)),
            )
            .on_action(cx.listener(|t, _: &ShowKeyboard, window, cx| {
                t.navigate_to_pane("keyboard", window, cx)
            }))
            .on_action(cx.listener(|t, _: &ShowLanguageRegion, window, cx| {
                t.navigate_to_pane("language-region", window, cx)
            }))
            .on_action(cx.listener(|t, _: &ShowLockScreen, window, cx| {
                t.navigate_to_pane("lock-screen", window, cx)
            }))
            .on_action(cx.listener(|t, _: &ShowLoginPassword, window, cx| {
                t.navigate_to_pane("login-password", window, cx)
            }))
            .on_action(cx.listener(|t, _: &ShowUsersGroups, window, cx| {
                t.navigate_to_pane("users-groups", window, cx)
            }))
            .on_action(cx.listener(|t, _: &ShowPrintersScanners, window, cx| {
                t.navigate_to_pane("printers-scanners", window, cx)
            }))
            .on_action(cx.listener(|t, _: &ShowLoginItems, window, cx| {
                t.navigate_to_pane("login-items", window, cx)
            }))
            .on_action(cx.listener(|t, _: &ShowMenuBar, window, cx| {
                t.navigate_to_pane("menu-bar", window, cx)
            }))
            .on_action(cx.listener(|t, _: &ShowNetwork, window, cx| {
                t.navigate_to_pane("network", window, cx)
            }))
            .on_action(cx.listener(|t, _: &ShowInternetAccounts, window, cx| {
                t.navigate_to_pane("internet-accounts", window, cx)
            }))
            .on_action(cx.listener(|t, _: &ShowNotifications, window, cx| {
                t.navigate_to_pane("notifications", window, cx)
            }))
            .on_action(cx.listener(|t, _: &ShowPrivacySecurity, window, cx| {
                t.navigate_to_pane("privacy-security", window, cx)
            }))
            .on_action(cx.listener(|t, _: &ShowSharing, window, cx| {
                t.navigate_to_pane("sharing", window, cx)
            }))
            .on_action(cx.listener(|t, _: &ShowSoftwareUpdate, window, cx| {
                t.navigate_to_pane("software-update", window, cx)
            }))
            .on_action(
                cx.listener(|t, _: &ShowSound, window, cx| t.navigate_to_pane("sound", window, cx)),
            )
            .on_action(cx.listener(|t, _: &ShowSpotlight, window, cx| {
                t.navigate_to_pane("spotlight", window, cx)
            }))
            .on_action(cx.listener(|t, _: &ShowStorage, window, cx| {
                t.navigate_to_pane("storage", window, cx)
            }))
            .on_action(cx.listener(|t, _: &ShowTrackpad, window, cx| {
                t.navigate_to_pane("trackpad", window, cx)
            }))
            .on_action(cx.listener(|t, _: &ShowWallpaper, window, cx| {
                t.navigate_to_pane("wallpaper", window, cx)
            }))
            .on_action(
                cx.listener(|t, _: &ShowWifi, window, cx| t.navigate_to_pane("wifi", window, cx)),
            )
            .on_action(cx.listener(|t, _: &FocusSearch, window, cx| {
                let focus = t.search.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectAlert, _, cx| {
                this.apply_sound_policy_change(
                    sound::SoundPolicyChange::AlertSound(rmac_sound::Cue::Alert),
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &SelectErrorAlert, _, cx| {
                this.apply_sound_policy_change(
                    sound::SoundPolicyChange::AlertSound(rmac_sound::Cue::Error),
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &SelectNotificationAlert, _, cx| {
                this.apply_sound_policy_change(
                    sound::SoundPolicyChange::AlertSound(rmac_sound::Cue::Notification),
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &rmac_ui::RequestClose, window, cx| {
                if this.clock_setting {
                    return;
                }
                if this.keyboard_shortcuts_open {
                    this.close_keyboard_shortcuts(cx);
                    return;
                }
                if this.recent_history_busy {
                    return;
                }
                if this.recent_history_confirmation {
                    this.cancel_recent_history_clear(cx);
                    return;
                }
                if this.clock_confirmation.take().is_some() {
                    cx.notify();
                    return;
                }
                if this.clock_editor.take().is_some() {
                    this.time_error = None;
                    cx.notify();
                    return;
                }
                if this.updates_installing {
                    return;
                }
                if this.updates_preparing {
                    this.cancel_update_operation(cx);
                    return;
                }
                if this.updates_plan.take().is_some() {
                    cx.notify();
                    return;
                }
                if this.updates_auto_sheet {
                    this.close_update_auto_sheet(cx);
                    return;
                }
                if this.updates_info_sheet.take().is_some() {
                    cx.notify();
                    return;
                }
                if let Some(cancellation) = &this.vpn_cancellation {
                    cancellation.cancel();
                    return;
                }
                if this.vpn_import_preview.is_some() {
                    if !this.vpn_import_busy {
                        this.finish_vpn_import(false, cx);
                    }
                    return;
                }
                if this.vpn_import_busy {
                    return;
                }
                if this.vpn_delete_preparing.is_some() || this.vpn_delete_busy {
                    return;
                }
                if this.vpn_delete_preview.take().is_some() {
                    window.remove_window();
                    return;
                }
                if this.vpn_secret_preparing || this.vpn_secret_busy {
                    return;
                }
                if this.vpn_secret_preview.take().is_some() {
                    window.remove_window();
                    return;
                }
                if this.wifi_forgetting.is_some()
                    || this.bluetooth_forgetting.is_some()
                    || this.network_busy
                    || this.vpn_editor_busy
                {
                    return;
                }
                if this.vpn_busy.is_some() {
                    return;
                }
                if let Some(cancellation) = &this.wifi_cancellation {
                    cancellation.cancel();
                }
                if let Some(pairing) = &this.bluetooth_pairing {
                    pairing.session.cancel();
                }
                window.remove_window();
            }))
            .on_action(cx.listener(|_, _: &Minimize, _, cx| {
                rmac_ui::minimize_focused_window(cx);
            }))
            .bg(pane_bg())
            .text_color(label())
            .when(layout.sidebar_visible, |root| {
                // The sidebar column's own size; its view fills it.
                root.child(
                    div()
                        .h_full()
                        .flex_shrink_0()
                        .when(layout.compact, |column| column.flex_1())
                        .when(!layout.compact, |column| {
                            column.w(px(style::SIDEBAR_COLUMN_WIDTH))
                        })
                        .child(views::view_element(&sidebar_view, cache_views)),
                )
            })
            .when(layout.detail_visible, |root| {
                root.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .v_flex()
                        // A boundary, not a stop of its own (`tab_stop(false)`):
                        // `select_position` focuses it then steps onto the
                        // pane's first real control, so Tab reaches this
                        // pane's own content and Shift-Tab from that first
                        // control returns to the sidebar.
                        .track_focus(&self.content_focus.clone().tab_stop(false))
                        .child(self.render_toolbar(layout, cx))
                        .when_some(settings_error, |settings, message| {
                            settings.child(
                                Toast::new(
                                    rmac_system_settings::accessibility::GLOBAL_ERROR_ID,
                                    ToastKind::Error,
                                    rmac_system_settings::accessibility::GLOBAL_ERROR_TITLE,
                                )
                                .message(message)
                                .rounded(px(rmac_ui::mac::radius_none()))
                                .border_l_0()
                                .border_r_0()
                                .on_dismiss(cx.listener(
                                    |this, _, _, cx| {
                                        this.system_data_error = None;
                                        this.system_data_stream_error = None;
                                        this.updates_error = None;
                                        this.updates_stream_error = None;
                                        this.storage_error = None;
                                        this.storage_stream_error = None;
                                        this.time_error = None;
                                        this.time_stream_error = None;
                                        this.locale_error = None;
                                        this.locale_stream_error = None;
                                        this.login_items_error = None;
                                        this.login_items_stream_error = None;
                                        this.sharing_error = None;
                                        this.sharing_stream_error = None;
                                        this.wifi_error = None;
                                        this.wifi_stream_error = None;
                                        this.bluetooth_error = None;
                                        this.bluetooth_stream_error = None;
                                        this.network_error = None;
                                        this.network_stream_error = None;
                                        this.vpn_error = None;
                                        this.vpn_stream_error = None;
                                        this.audio_error = None;
                                        this.audio_stream_error = None;
                                        this.power_error = None;
                                        this.power_stream_error = None;
                                        this.display_error = None;
                                        this.input_error = None;
                                        this.input_stream_error = None;
                                        this.theme_error = None;
                                        this.theme_store_stream_error = None;
                                        this.theme_portal_stream_error = None;
                                        this.shell_settings_error = None;
                                        this.shell_settings_stream_error = None;
                                        this.wallpaper_error = None;
                                        this.spotlight_error = None;
                                        this.gtk_text_error = None;
                                        this.gtk_text_stream_error = None;
                                        this.privacy_error = None;
                                        this.privacy_stream_error = None;
                                        cx.notify();
                                    },
                                )),
                            )
                        })
                        .child(
                            div()
                                .flex_1()
                                .min_h(px(0.0))
                                .w_full()
                                .child(views::view_element(&pane_view, cache_views)),
                        ),
                )
            })
            .when_some(wifi_password_dialog, |root, dialog| root.child(dialog))
            .when_some(wifi_enterprise_dialog, |root, dialog| root.child(dialog))
            .when_some(wifi_forget_dialog, |root, dialog| root.child(dialog))
            .when_some(bluetooth_pairing_dialog, |root, dialog| root.child(dialog))
            .when_some(bluetooth_forget_dialog, |root, dialog| root.child(dialog))
            .when_some(clock_confirmation_dialog, |root, dialog| root.child(dialog))
            .when_some(vpn_import_dialog, |root, dialog| root.child(dialog))
            .when_some(vpn_secret_clear_dialog, |root, dialog| root.child(dialog))
            .when_some(vpn_delete_dialog, |root, dialog| root.child(dialog))
            .when_some(update_install_dialog, |root, dialog| root.child(dialog))
            .when_some(update_info_sheet, |root, sheet| root.child(sheet))
            .when_some(update_auto_sheet, |root, sheet| root.child(sheet))
            .when_some(keyboard_shortcuts_sheet, |root, sheet| root.child(sheet))
            .when_some(internet_account_sheet, |root, sheet| root.child(sheet))
            .when_some(internet_account_delete, |root, alert| root.child(alert))
            .when_some(users_overlay, |root, sheet| root.child(sheet))
            .when_some(printers_overlay, |root, sheet| root.child(sheet))
    }
}
