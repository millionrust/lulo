//! System Settings shell semantics and shared visible navigation/error authority.

use super::*;

impl Settings {
    #[allow(dead_code)]
    pub(super) fn navigation_accessibility_snapshot(
        &self,
        layout: crate::responsive_layout::SettingsLayout,
        cx: &Context<Self>,
    ) -> std::result::Result<
        rmac_system_settings::accessibility::SettingsNavigationAccessibilitySnapshot,
        rmac_system_settings::accessibility::AccessibilityProjectionError,
    > {
        use rmac_system_settings::accessibility::{
            project_settings_navigation, AccessibilityProjectionError, NavigationCategoryInput,
            NavigationInput,
        };

        let mut sections = Vec::with_capacity(self.sections.len());
        for section in &self.sections {
            let mut categories = Vec::with_capacity(section.len());
            for category in section {
                let pane_id = pane_id_for_category_name(category.name.as_ref())
                    .ok_or(AccessibilityProjectionError::InvalidCategory)?;
                categories.push(NavigationCategoryInput {
                    pane_id,
                    name: category.name.as_ref(),
                    description: category.desc.as_ref(),
                    search_terms: category.search_terms,
                });
            }
            sections.push(categories);
        }
        let query = self.search.read(cx).value().to_string();
        let subpage_title = self.nav.last().map(|subpage| self.subpage_title(subpage));
        let global_error = self.global_settings_error().map(|error| {
            rmac_ui::user_error_message(rmac_ui::ErrorSurface::Settings, error.as_ref(), false)
        });
        project_settings_navigation(NavigationInput {
            sections: &sections,
            selected: self.selected,
            query: &query,
            account_name: self.account.as_ref(),
            subpage_title: subpage_title.as_deref(),
            back_depth: self.nav.len(),
            global_error: global_error.as_ref().map(|error| error.as_ref()),
            sidebar_visible: layout.sidebar_visible,
            detail_visible: layout.detail_visible,
        })
    }

    pub(super) fn subpage_title(&self, subpage: &SubPage) -> String {
        match subpage {
            SubPage::About => "About".into(),
            SubPage::SoftwareUpdate => "Software Update".into(),
            SubPage::Storage => "Storage".into(),
            SubPage::NotificationApp { app_id } => self
                .application_identity(app_id)
                .map(|identity| identity.name.clone())
                .unwrap_or_else(|| app_id.clone()),
            SubPage::FocusMode { mode_id } => rmac_focus::ModeId::parse(mode_id)
                .ok()
                .and_then(|mode_id| {
                    self.focus_policy_config
                        .as_ref()
                        .and_then(|configuration| configuration.mode(&mode_id))
                        .map(|mode| mode.name().to_owned())
                })
                .unwrap_or_else(|| "Focus".into()),
            SubPage::FocusSchedule { .. } => "Schedule".into(),
        }
    }

    pub(super) fn global_settings_error(&self) -> Option<&SharedString> {
        self.system_data_error
            .as_ref()
            .or(self.system_data_stream_error.as_ref())
            .or(self.updates_error.as_ref())
            .or(self.updates_stream_error.as_ref())
            .or(self.storage_error.as_ref())
            .or(self.storage_stream_error.as_ref())
            .or(self.time_error.as_ref())
            .or(self.time_stream_error.as_ref())
            .or(self.locale_error.as_ref())
            .or(self.locale_stream_error.as_ref())
            .or(self.login_items_error.as_ref())
            .or(self.login_items_stream_error.as_ref())
            .or(self.sharing_error.as_ref())
            .or(self.sharing_stream_error.as_ref())
            .or(self.wifi_error.as_ref())
            .or(self.wifi_stream_error.as_ref())
            .or(self.bluetooth_error.as_ref())
            .or(self.bluetooth_stream_error.as_ref())
            .or(self.network_error.as_ref())
            .or(self.network_stream_error.as_ref())
            .or(self.vpn_error.as_ref())
            .or(self.vpn_stream_error.as_ref())
            .or(self.audio_error.as_ref())
            .or(self.audio_stream_error.as_ref())
            .or(self.power_error.as_ref())
            .or(self.power_stream_error.as_ref())
            .or(self.display_error.as_ref())
            .or(self.input_error.as_ref())
            .or(self.input_stream_error.as_ref())
            .or(self.theme_error.as_ref())
            .or(self.theme_store_stream_error.as_ref())
            .or(self.theme_portal_stream_error.as_ref())
            .or(self.shell_settings_error.as_ref())
            .or(self.shell_settings_stream_error.as_ref())
            .or(self.wallpaper_error.as_ref())
            .or(self.spotlight_error.as_ref())
            .or(self.gtk_text_error.as_ref())
            .or(self.gtk_text_stream_error.as_ref())
            .or(self.privacy_error.as_ref())
            .or(self.privacy_stream_error.as_ref())
    }
}
