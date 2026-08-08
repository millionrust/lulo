//! System Settings navigation accessibility snapshot projection.

use std::collections::HashSet;

use super::budget::*;
use super::matching::*;
use super::model::*;

pub fn project_settings_navigation(
    input: NavigationInput<'_>,
) -> Result<SettingsNavigationAccessibilitySnapshot, AccessibilityProjectionError> {
    if input.sections.is_empty() || input.sections.len() > MAX_NAVIGATION_SECTIONS {
        return Err(AccessibilityProjectionError::SectionLimit);
    }
    let category_count = input.sections.iter().map(Vec::len).sum::<usize>();
    if category_count == 0 || category_count > MAX_NAVIGATION_CATEGORIES {
        return Err(AccessibilityProjectionError::CategoryLimit);
    }
    let selected = input
        .sections
        .get(input.selected.0)
        .and_then(|section| section.get(input.selected.1))
        .ok_or(AccessibilityProjectionError::InvalidSelection)?;
    if input.back_depth > MAX_NAVIGATION_DEPTH {
        return Err(AccessibilityProjectionError::NavigationDepth);
    }
    if !input.sidebar_visible && !input.detail_visible {
        return Err(AccessibilityProjectionError::InvalidVisibility);
    }
    if (input.back_depth == 0) != input.subpage_title.is_none() {
        return Err(AccessibilityProjectionError::InvalidSubpage);
    }

    let mut budget = TextBudget::default();
    for fixed in [
        SETTINGS_NAME,
        ROOT_ID,
        SEARCH_ID,
        SEARCH_NAME,
        SIDEBAR_ID,
        SIDEBAR_TOGGLE_ID,
        SHOW_SIDEBAR_NAME,
        HIDE_SIDEBAR_NAME,
        DETAIL_ID,
        BACK_ID,
        BACK_NAME,
        LOCAL_ACCOUNT_LABEL,
        GLOBAL_ERROR_ID,
        GLOBAL_ERROR_TITLE,
        GLOBAL_ERROR_DISMISS_ID,
        DISMISS_NAME,
    ] {
        budget.add_required(fixed, MAX_LABEL_BYTES, false)?;
    }
    budget.add_optional(input.query, MAX_QUERY_BYTES, false)?;
    budget.add_required(input.account_name, MAX_LABEL_BYTES, false)?;
    if let Some(title) = input.subpage_title {
        budget.add_required(title, MAX_LABEL_BYTES, false)?;
    }
    if let Some(error) = input.global_error {
        budget.add_required(error, MAX_ERROR_BYTES, true)?;
    }

    let mut seen_ids = HashSet::with_capacity(category_count);
    let mut seen_names = HashSet::with_capacity(category_count);
    for section in input.sections {
        if section.is_empty() {
            return Err(AccessibilityProjectionError::InvalidCategory);
        }
        for category in section {
            validate_pane_id(category.pane_id)?;
            budget.add_required(category.pane_id, 128, false)?;
            budget.add_required(category.name, MAX_LABEL_BYTES, false)?;
            budget.add_required(category.description, MAX_LABEL_BYTES, true)?;
            if category.search_terms.len() > MAX_CATEGORY_SEARCH_TERMS {
                return Err(AccessibilityProjectionError::InvalidCategory);
            }
            let mut seen_terms = HashSet::with_capacity(category.search_terms.len());
            for term in category.search_terms {
                budget.add_required(term, MAX_LABEL_BYTES, false)?;
                if !seen_terms.insert(*term) {
                    return Err(AccessibilityProjectionError::DuplicateCategory);
                }
            }
            if !seen_ids.insert(category.pane_id) || !seen_names.insert(category.name) {
                return Err(AccessibilityProjectionError::DuplicateCategory);
            }
        }
    }

    let visible_indices = if input.sidebar_visible {
        input
            .sections
            .iter()
            .enumerate()
            .filter_map(|(section_index, section)| {
                let items = section
                    .iter()
                    .enumerate()
                    .filter_map(|(item_index, category)| {
                        category_matches(
                            input.query,
                            category.name,
                            category.description,
                            category.search_terms,
                        )
                        .then_some((section_index, item_index))
                    })
                    .collect::<Vec<_>>();
                (!items.is_empty()).then_some(items)
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let visible_section_count = visible_indices.len();
    let visible_category_count = visible_indices.iter().map(Vec::len).sum::<usize>();

    let mut sidebar_position = 0usize;
    let mut sections = Vec::with_capacity(visible_section_count);
    let mut selected_visible = false;
    let mut navigation_focus_order = Vec::with_capacity(visible_category_count + 3);
    let compact = input.sidebar_visible != input.detail_visible;
    let sidebar_toggle_action = compact.then(|| AccessibleNavigationAction {
        id: SIDEBAR_TOGGLE_ID.into(),
        name: if input.sidebar_visible {
            HIDE_SIDEBAR_NAME
        } else {
            SHOW_SIDEBAR_NAME
        }
        .into(),
        kind: NavigationActionKind::ToggleSidebar,
        enabled: true,
    });
    if sidebar_toggle_action.is_some() {
        navigation_focus_order.push(SIDEBAR_TOGGLE_ID.into());
    }
    let back_action = (input.back_depth > 0).then(|| AccessibleNavigationAction {
        id: BACK_ID.into(),
        name: BACK_NAME.into(),
        kind: NavigationActionKind::Back,
        enabled: true,
    });
    if back_action.is_some() {
        navigation_focus_order.push(BACK_ID.into());
    }
    let error_action = input.global_error.map(|_| AccessibleNavigationAction {
        id: GLOBAL_ERROR_DISMISS_ID.into(),
        name: DISMISS_NAME.into(),
        kind: NavigationActionKind::DismissError,
        enabled: true,
    });
    if error_action.is_some() {
        navigation_focus_order.push(GLOBAL_ERROR_DISMISS_ID.into());
    }
    if input.sidebar_visible {
        navigation_focus_order.push(SEARCH_ID.into());
    }

    for (visible_section_index, indices) in visible_indices.into_iter().enumerate() {
        let section_size = indices.len();
        let mut items = Vec::with_capacity(section_size);
        for (position_in_section, (section_index, item_index)) in indices.into_iter().enumerate() {
            let category = input.sections[section_index][item_index];
            sidebar_position += 1;
            let is_selected = (section_index, item_index) == input.selected;
            selected_visible |= is_selected;
            let action_id = format!("cat-{section_index}-{item_index}");
            let action_name = format!("Open {} settings", category.name);
            let match_hint = category_match_hint(input.query, category.search_terms);
            budget.add_required(&action_id, 128, false)?;
            budget.add_required(&action_name, MAX_LABEL_BYTES, false)?;
            if let Some(match_hint) = match_hint {
                budget.add_required(match_hint, MAX_LABEL_BYTES, false)?;
            }
            navigation_focus_order.push(action_id.clone());
            items.push(AccessibleNavigationItem {
                pane_id: category.pane_id.into(),
                name: category.name.into(),
                description: category.description.into(),
                match_hint: match_hint.map(str::to_owned),
                selected: is_selected,
                position_in_section: position_in_section + 1,
                section_size,
                position_in_sidebar: sidebar_position,
                sidebar_size: visible_category_count,
                action: AccessibleNavigationAction {
                    id: action_id,
                    name: action_name,
                    kind: NavigationActionKind::SelectPane,
                    enabled: true,
                },
            });
        }
        sections.push(AccessibleNavigationSection {
            position_in_set: visible_section_index + 1,
            set_size: visible_section_count,
            items,
        });
    }

    let heading = input.subpage_title.unwrap_or(selected.name).to_owned();
    budget.add_required(&heading, MAX_LABEL_BYTES, false)?;
    let mut announcements = Vec::new();
    if input.sidebar_visible && !input.query.is_empty() {
        let text = if visible_category_count == 1 {
            "1 matching settings pane".to_owned()
        } else {
            format!("{visible_category_count} matching settings panes")
        };
        budget.add_required(&text, MAX_LABEL_BYTES, false)?;
        announcements.push(LiveAnnouncement {
            id: "settings-search-results".into(),
            text,
            politeness: LivePoliteness::Polite,
        });
    }
    if let Some(error) = input.global_error {
        announcements.push(LiveAnnouncement {
            id: GLOBAL_ERROR_ID.into(),
            text: error.into(),
            politeness: LivePoliteness::Assertive,
        });
    }

    let initial_focus = if input.back_depth > 0 {
        BACK_ID
    } else if input.sidebar_visible {
        SEARCH_ID
    } else {
        DETAIL_ID
    }
    .to_owned();
    validate_actions(&navigation_focus_order)?;

    Ok(SettingsNavigationAccessibilitySnapshot {
        name: SETTINGS_NAME,
        root_id: ROOT_ID,
        search: AccessibleSearchField {
            id: SEARCH_ID,
            name: SEARCH_NAME,
            value: input.query.into(),
            result_count: visible_category_count,
            controls: SIDEBAR_ID,
        },
        account: AccessibleAccount {
            name: input.account_name.into(),
            description: LOCAL_ACCOUNT_LABEL,
        },
        sidebar_id: SIDEBAR_ID,
        sidebar_visible: input.sidebar_visible,
        sections,
        selected_pane_id: selected.pane_id.into(),
        selected_visible,
        detail_id: DETAIL_ID,
        detail_visible: input.detail_visible,
        detail: AccessibleDetail {
            pane_id: selected.pane_id.into(),
            pane_name: selected.name.into(),
            pane_description: selected.description.into(),
            heading,
            subpage_title: input.subpage_title.map(str::to_owned),
            back_depth: input.back_depth,
        },
        sidebar_toggle_action,
        back_action,
        error_action,
        navigation_focus_order,
        initial_focus,
        announcements,
    })
}
