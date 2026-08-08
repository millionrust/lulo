use super::*;

fn category<'a>(id: &'a str, name: &'a str) -> NavigationCategoryInput<'a> {
    NavigationCategoryInput {
        pane_id: id,
        name,
        description: "Authoritative settings description",
        search_terms: &[],
    }
}

fn sections() -> Vec<Vec<NavigationCategoryInput<'static>>> {
    vec![
        vec![
            category("wifi", "Wi-Fi"),
            category("bluetooth", "Bluetooth"),
        ],
        vec![
            category("appearance", "Appearance"),
            category("displays", "Displays"),
        ],
    ]
}

#[test]
fn complete_sidebar_selection_detail_and_focus_order_are_projected() {
    let sections = sections();
    let projected = project_settings_navigation(NavigationInput {
        sections: &sections,
        selected: (0, 1),
        query: "",
        account_name: "Jacob",
        subpage_title: None,
        back_depth: 0,
        global_error: None,
        sidebar_visible: true,
        detail_visible: true,
    })
    .unwrap();

    assert_eq!(projected.search.result_count, 4);
    assert_eq!(projected.sections.len(), 2);
    assert!(projected.sections[0].items[1].selected);
    assert_eq!(projected.selected_pane_id, "bluetooth");
    assert_eq!(projected.detail.heading, "Bluetooth");
    assert!(projected.selected_visible);
    assert_eq!(
        projected.navigation_focus_order,
        [
            "settings-search",
            "cat-0-0",
            "cat-0-1",
            "cat-1-0",
            "cat-1-1",
        ]
    );
    assert_eq!(projected.initial_focus, SEARCH_ID);
}

#[test]
fn filtering_subpage_back_error_and_diagnostics_preserve_private_state() {
    let sections = sections();
    let input = NavigationInput {
        sections: &sections,
        selected: (0, 1),
        query: "display",
        account_name: "Private Account",
        subpage_title: Some("Private Application"),
        back_depth: 1,
        global_error: Some("Private backend detail"),
        sidebar_visible: true,
        detail_visible: true,
    };
    let projected = project_settings_navigation(input).unwrap();

    assert_eq!(projected.search.result_count, 1);
    assert_eq!(projected.sections[0].items[0].pane_id, "displays");
    assert!(!projected.selected_visible);
    assert_eq!(projected.detail.heading, "Private Application");
    assert_eq!(projected.initial_focus, BACK_ID);
    assert_eq!(
        &projected.navigation_focus_order[..3],
        [BACK_ID, GLOBAL_ERROR_DISMISS_ID, SEARCH_ID]
    );
    assert_eq!(
        projected.announcements.last().unwrap().politeness,
        LivePoliteness::Assertive
    );

    let diagnostics = format!("{input:?} {projected:?}");
    for private in [
        "display",
        "Private Account",
        "Private Application",
        "Private backend detail",
    ] {
        assert!(!diagnostics.contains(private));
    }
}

#[test]
fn search_finds_implemented_setting_labels_and_exposes_the_match_reason() {
    let mut sections = sections();
    sections[1][0].search_terms = &["Light appearance", "Dark appearance", "Accent color"];
    let projected = project_settings_navigation(NavigationInput {
        sections: &sections,
        selected: (0, 0),
        query: "dark",
        account_name: "Account",
        subpage_title: None,
        back_depth: 0,
        global_error: None,
        sidebar_visible: true,
        detail_visible: true,
    })
    .unwrap();

    assert_eq!(projected.search.result_count, 1);
    assert_eq!(projected.sections[0].items[0].pane_id, "appearance");
    assert_eq!(
        projected.sections[0].items[0].match_hint.as_deref(),
        Some("Dark appearance")
    );
    assert_eq!(projected.announcements[0].text, "1 matching settings pane");
}

#[test]
fn compact_detail_hides_sidebar_items_search_focus_and_announcements() {
    let sections = sections();
    let projected = project_settings_navigation(NavigationInput {
        sections: &sections,
        selected: (1, 1),
        query: "display",
        account_name: "Account",
        subpage_title: None,
        back_depth: 0,
        global_error: None,
        sidebar_visible: false,
        detail_visible: true,
    })
    .unwrap();

    assert!(!projected.sidebar_visible);
    assert!(projected.detail_visible);
    assert!(projected.sections.is_empty());
    assert_eq!(projected.search.result_count, 0);
    assert!(!projected.selected_visible);
    assert_eq!(projected.initial_focus, DETAIL_ID);
    assert_eq!(projected.navigation_focus_order, [SIDEBAR_TOGGLE_ID]);
    assert_eq!(
        projected.sidebar_toggle_action.unwrap().name,
        SHOW_SIDEBAR_NAME
    );
    assert!(projected.announcements.is_empty());
}

#[test]
fn duplicate_invalid_selection_depth_and_oversized_query_fail_closed() {
    let mut duplicate = sections();
    duplicate[1][0].pane_id = "wifi";
    assert_eq!(
        project_settings_navigation(NavigationInput {
            sections: &duplicate,
            selected: (0, 0),
            query: "",
            account_name: "Account",
            subpage_title: None,
            back_depth: 0,
            global_error: None,
            sidebar_visible: true,
            detail_visible: true,
        }),
        Err(AccessibilityProjectionError::DuplicateCategory)
    );

    let sections = sections();
    let base = NavigationInput {
        sections: &sections,
        selected: (9, 9),
        query: "",
        account_name: "Account",
        subpage_title: None,
        back_depth: 0,
        global_error: None,
        sidebar_visible: true,
        detail_visible: true,
    };
    assert_eq!(
        project_settings_navigation(base),
        Err(AccessibilityProjectionError::InvalidSelection)
    );
    assert_eq!(
        project_settings_navigation(NavigationInput {
            selected: (0, 0),
            subpage_title: Some("Too deep"),
            back_depth: MAX_NAVIGATION_DEPTH + 1,
            ..base
        }),
        Err(AccessibilityProjectionError::NavigationDepth)
    );
    let oversized = "x".repeat(MAX_QUERY_BYTES + 1);
    assert_eq!(
        project_settings_navigation(NavigationInput {
            selected: (0, 0),
            query: &oversized,
            ..base
        }),
        Err(AccessibilityProjectionError::TextValueLimit)
    );
}
