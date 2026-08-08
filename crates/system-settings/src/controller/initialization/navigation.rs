//! Initial pane selection and navigation persistence.

use super::*;

impl Settings {
    pub(super) fn initial_navigation(
        cx: &mut Context<Self>,
    ) -> (Vec<Vec<Category>>, (usize, usize), NavigationPersistence) {
        let sections = categories();
        let requested_category =
            std::env::args()
                .collect::<Vec<_>>()
                .windows(2)
                .find_map(|arguments| {
                    (arguments[0] == "--pane")
                        .then(|| category_name_for_pane_id(&arguments[1]))
                        .flatten()
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
        (sections, selected, navigation_persistence)
    }
}
