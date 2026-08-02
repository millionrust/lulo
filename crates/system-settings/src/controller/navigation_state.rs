//! Category selection and detail back-stack state.

use super::*;

impl Settings {
    pub(super) fn current(&self) -> &Category {
        &self.sections[self.selected.0][self.selected.1]
    }

    pub(super) fn application_identity(&self, app_id: &str) -> Option<&rmac_apps::Application> {
        rmac_apps::find_desktop_entry(&self.app_catalog, app_id)
    }

    pub(super) fn go_back(&mut self, cx: &mut Context<Self>) {
        self.nav.pop();
        cx.notify();
    }

    pub(super) fn push(&mut self, sub: SubPage, cx: &mut Context<Self>) {
        if self.nav.len() >= rmac_system_settings::accessibility::MAX_NAVIGATION_DEPTH {
            return;
        }
        self.nav.push(sub);
        cx.notify();
    }

    pub(super) fn select_category(&mut self, name: &str, cx: &mut Context<Self>) {
        let target = self
            .sections
            .iter()
            .enumerate()
            .find_map(|(section, items)| {
                items
                    .iter()
                    .position(|category| category.name.as_ref() == name)
                    .map(|item| (section, item))
            });
        if let Some(target) = target {
            self.selected = target;
            self.nav.clear();
            cx.notify();
        }
    }
}
