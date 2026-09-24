//! Initial pane selection and navigation persistence.

use super::*;

impl Settings {
    pub(super) fn initial_navigation(
        cx: &mut Context<Self>,
    ) -> (
        Vec<Vec<Category>>,
        (usize, usize),
        Vec<SubPage>,
        NavigationPersistence,
    ) {
        let sections = categories();
        let requested_pane = std::env::args()
            .collect::<Vec<_>>()
            .windows(2)
            .find_map(|arguments| (arguments[0] == "--pane").then(|| arguments[1].clone()));
        let requested_subpage = requested_pane.as_deref().and_then(subpage_route);
        let requested_category = requested_pane.as_deref().and_then(|pane_id| {
            category_name_for_pane_id(pane_id)
                .or_else(|| requested_subpage.as_ref().map(|(category, _)| *category))
        });
        let restored_pane = NavigationPersistence::restore();
        let selected = requested_category
            .or_else(|| restored_pane.as_deref().and_then(category_name_for_pane_id))
            .and_then(|category| category_position(&sections, category))
            .unwrap_or((1, 0));
        let navigation_persistence = NavigationPersistence::start(cx);
        if let Some(pane_id) =
            pane_id_for_category_name(sections[selected.0][selected.1].name.as_ref())
        {
            navigation_persistence.schedule(pane_id);
        }
        // A subpage route opens only when its pane is the one selected.
        let nav = requested_subpage
            .filter(|(category, _)| sections[selected.0][selected.1].name == *category)
            .map(|(_, page)| vec![page])
            .unwrap_or_default();
        (sections, selected, nav, navigation_persistence)
    }
}
