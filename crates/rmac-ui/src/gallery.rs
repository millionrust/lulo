//! Component-gallery contract for the shared rmac design system.
//!
//! The gallery is an executable specification, not a collection of arbitrary
//! screenshots.  Keeping the inventory and its required states in this crate
//! lets tests fail when a shared control is forgotten or a renderer silently
//! drops an interaction state.

/// Logical preview scales required by the design-system acceptance gate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreviewScale {
    /// Label shown in the gallery and visual-reference filenames.
    pub label: &'static str,
    /// Multiplier applied by the deterministic preview renderer.
    pub factor: f32,
}

impl PreviewScale {
    /// Scale a logical pixel value for a deterministic preview.
    pub const fn px(self, value: f32) -> f32 {
        value * self.factor
    }
}

/// The three deterministic preview scales required for every component state.
pub const PREVIEW_SCALES: [PreviewScale; 3] = [
    PreviewScale {
        label: "100%",
        factor: 1.0,
    },
    PreviewScale {
        label: "150%",
        factor: 1.5,
    },
    PreviewScale {
        label: "200%",
        factor: 2.0,
    },
];

/// Controls that Phase B requires rmac-ui to own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GalleryComponent {
    Button,
    Toggle,
    Slider,
    TextField,
    SearchField,
    List,
    Table,
    Tree,
    Tabs,
    Dialog,
    Alert,
    ContextMenu,
    Tooltip,
    Progress,
    EmptyState,
    Toast,
}

impl GalleryComponent {
    /// Stable identifier used by element IDs and visual-reference filenames.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Button => "button",
            Self::Toggle => "toggle",
            Self::Slider => "slider",
            Self::TextField => "text-field",
            Self::SearchField => "search-field",
            Self::List => "list",
            Self::Table => "table",
            Self::Tree => "tree",
            Self::Tabs => "tabs",
            Self::Dialog => "dialog",
            Self::Alert => "alert",
            Self::ContextMenu => "context-menu",
            Self::Tooltip => "tooltip",
            Self::Progress => "progress",
            Self::EmptyState => "empty-state",
            Self::Toast => "toast",
        }
    }

    /// Human-readable component name.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Button => "Button",
            Self::Toggle => "Toggle",
            Self::Slider => "Slider",
            Self::TextField => "Text field",
            Self::SearchField => "Search field",
            Self::List => "List",
            Self::Table => "Table",
            Self::Tree => "Tree",
            Self::Tabs => "Tabs",
            Self::Dialog => "Dialog",
            Self::Alert => "Alert",
            Self::ContextMenu => "Context menu",
            Self::Tooltip => "Tooltip",
            Self::Progress => "Progress",
            Self::EmptyState => "Empty state",
            Self::Toast => "Toast",
        }
    }
}

/// Visual and behavioral states represented by the gallery.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GalleryState {
    Default,
    Hover,
    Pressed,
    Focused,
    Disabled,
    Busy,
    Selected,
    Unselected,
    On,
    Off,
    Mixed,
    Empty,
    Filled,
    Invalid,
    Loading,
    Stale,
    Unavailable,
    Expanded,
    Collapsed,
    SortedAscending,
    SortedDescending,
    Destructive,
    Informational,
    Success,
    Warning,
    Error,
    Determinate,
    Indeterminate,
    Complete,
}

impl GalleryState {
    /// Stable label used in the gallery and evidence manifests.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Hover => "Hover",
            Self::Pressed => "Pressed",
            Self::Focused => "Focused",
            Self::Disabled => "Disabled",
            Self::Busy => "Busy",
            Self::Selected => "Selected",
            Self::Unselected => "Unselected",
            Self::On => "On",
            Self::Off => "Off",
            Self::Mixed => "Mixed",
            Self::Empty => "Empty",
            Self::Filled => "Filled",
            Self::Invalid => "Invalid",
            Self::Loading => "Loading",
            Self::Stale => "Stale",
            Self::Unavailable => "Unavailable",
            Self::Expanded => "Expanded",
            Self::Collapsed => "Collapsed",
            Self::SortedAscending => "Sorted ascending",
            Self::SortedDescending => "Sorted descending",
            Self::Destructive => "Destructive",
            Self::Informational => "Informational",
            Self::Success => "Success",
            Self::Warning => "Warning",
            Self::Error => "Error",
            Self::Determinate => "Determinate",
            Self::Indeterminate => "Indeterminate",
            Self::Complete => "Complete",
        }
    }
}

/// Keyboard journey attached to an interactive component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyboardBehavior {
    /// How keyboard focus reaches the control.
    pub enter: &'static str,
    /// Keys that perform the component's primary interaction.
    pub operate: &'static str,
    /// How the user leaves or dismisses the interaction.
    pub exit: &'static str,
}

/// Complete gallery requirements for one shared component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComponentSpec {
    pub component: GalleryComponent,
    pub states: &'static [GalleryState],
    pub keyboard: Option<KeyboardBehavior>,
}

const BUTTON_STATES: &[GalleryState] = &[
    GalleryState::Default,
    GalleryState::Hover,
    GalleryState::Pressed,
    GalleryState::Focused,
    GalleryState::Disabled,
    GalleryState::Busy,
    GalleryState::Destructive,
];
const TOGGLE_STATES: &[GalleryState] = &[
    GalleryState::Off,
    GalleryState::On,
    GalleryState::Mixed,
    GalleryState::Focused,
    GalleryState::Disabled,
];
const SLIDER_STATES: &[GalleryState] = &[
    GalleryState::Default,
    GalleryState::Focused,
    GalleryState::Disabled,
    GalleryState::Unavailable,
];
const FIELD_STATES: &[GalleryState] = &[
    GalleryState::Empty,
    GalleryState::Filled,
    GalleryState::Focused,
    GalleryState::Invalid,
    GalleryState::Disabled,
];
const SEARCH_STATES: &[GalleryState] = &[
    GalleryState::Empty,
    GalleryState::Filled,
    GalleryState::Focused,
    GalleryState::Loading,
    GalleryState::Unavailable,
];
const COLLECTION_STATES: &[GalleryState] = &[
    GalleryState::Unselected,
    GalleryState::Selected,
    GalleryState::Focused,
    GalleryState::Empty,
    GalleryState::Loading,
    GalleryState::Stale,
    GalleryState::Unavailable,
];
const TABLE_STATES: &[GalleryState] = &[
    GalleryState::Unselected,
    GalleryState::Selected,
    GalleryState::Focused,
    GalleryState::SortedAscending,
    GalleryState::SortedDescending,
    GalleryState::Empty,
    GalleryState::Loading,
    GalleryState::Stale,
    GalleryState::Unavailable,
];
const TREE_STATES: &[GalleryState] = &[
    GalleryState::Collapsed,
    GalleryState::Expanded,
    GalleryState::Selected,
    GalleryState::Focused,
    GalleryState::Empty,
    GalleryState::Loading,
    GalleryState::Unavailable,
];
const TABS_STATES: &[GalleryState] = &[
    GalleryState::Unselected,
    GalleryState::Selected,
    GalleryState::Focused,
    GalleryState::Disabled,
];
const DIALOG_STATES: &[GalleryState] = &[
    GalleryState::Default,
    GalleryState::Focused,
    GalleryState::Busy,
    GalleryState::Destructive,
    GalleryState::Error,
];
const ALERT_STATES: &[GalleryState] = &[
    GalleryState::Informational,
    GalleryState::Success,
    GalleryState::Warning,
    GalleryState::Error,
];
const MENU_STATES: &[GalleryState] = &[
    GalleryState::Default,
    GalleryState::Hover,
    GalleryState::Focused,
    GalleryState::Disabled,
    GalleryState::Destructive,
];
const TOOLTIP_STATES: &[GalleryState] = &[GalleryState::Default, GalleryState::Focused];
const PROGRESS_STATES: &[GalleryState] = &[
    GalleryState::Determinate,
    GalleryState::Indeterminate,
    GalleryState::Complete,
    GalleryState::Error,
];
const EMPTY_STATES: &[GalleryState] = &[
    GalleryState::Empty,
    GalleryState::Unavailable,
    GalleryState::Error,
];
const TOAST_STATES: &[GalleryState] = &[
    GalleryState::Informational,
    GalleryState::Success,
    GalleryState::Warning,
    GalleryState::Error,
];

const ACTIVATE: KeyboardBehavior = KeyboardBehavior {
    enter: "Tab / Shift+Tab",
    operate: "Enter or Space",
    exit: "Tab / Shift+Tab",
};
const ADJUST: KeyboardBehavior = KeyboardBehavior {
    enter: "Tab / Shift+Tab",
    operate: "Arrow keys; Home / End",
    exit: "Tab / Shift+Tab",
};
const EDIT: KeyboardBehavior = KeyboardBehavior {
    enter: "Tab / Shift+Tab or shortcut",
    operate: "Platform text editing keys",
    exit: "Tab / Shift+Tab; Escape where transient",
};
const NAVIGATE: KeyboardBehavior = KeyboardBehavior {
    enter: "Tab / Shift+Tab",
    operate: "Arrow keys; Home / End; Enter",
    exit: "Tab / Shift+Tab",
};
const MODAL: KeyboardBehavior = KeyboardBehavior {
    enter: "Focus moves from invoker to safe default",
    operate: "Tab cycles inside; Enter activates default",
    exit: "Escape cancels; focus returns to invoker",
};

/// Authoritative Phase B gallery inventory.
pub const COMPONENT_SPECS: &[ComponentSpec] = &[
    ComponentSpec {
        component: GalleryComponent::Button,
        states: BUTTON_STATES,
        keyboard: Some(ACTIVATE),
    },
    ComponentSpec {
        component: GalleryComponent::Toggle,
        states: TOGGLE_STATES,
        keyboard: Some(ACTIVATE),
    },
    ComponentSpec {
        component: GalleryComponent::Slider,
        states: SLIDER_STATES,
        keyboard: Some(ADJUST),
    },
    ComponentSpec {
        component: GalleryComponent::TextField,
        states: FIELD_STATES,
        keyboard: Some(EDIT),
    },
    ComponentSpec {
        component: GalleryComponent::SearchField,
        states: SEARCH_STATES,
        keyboard: Some(EDIT),
    },
    ComponentSpec {
        component: GalleryComponent::List,
        states: COLLECTION_STATES,
        keyboard: Some(NAVIGATE),
    },
    ComponentSpec {
        component: GalleryComponent::Table,
        states: TABLE_STATES,
        keyboard: Some(NAVIGATE),
    },
    ComponentSpec {
        component: GalleryComponent::Tree,
        states: TREE_STATES,
        keyboard: Some(NAVIGATE),
    },
    ComponentSpec {
        component: GalleryComponent::Tabs,
        states: TABS_STATES,
        keyboard: Some(NAVIGATE),
    },
    ComponentSpec {
        component: GalleryComponent::Dialog,
        states: DIALOG_STATES,
        keyboard: Some(MODAL),
    },
    ComponentSpec {
        component: GalleryComponent::Alert,
        states: ALERT_STATES,
        keyboard: Some(MODAL),
    },
    ComponentSpec {
        component: GalleryComponent::ContextMenu,
        states: MENU_STATES,
        keyboard: Some(NAVIGATE),
    },
    ComponentSpec {
        component: GalleryComponent::Tooltip,
        states: TOOLTIP_STATES,
        keyboard: None,
    },
    ComponentSpec {
        component: GalleryComponent::Progress,
        states: PROGRESS_STATES,
        keyboard: None,
    },
    ComponentSpec {
        component: GalleryComponent::EmptyState,
        states: EMPTY_STATES,
        keyboard: None,
    },
    ComponentSpec {
        component: GalleryComponent::Toast,
        states: TOAST_STATES,
        keyboard: Some(MODAL),
    },
];

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn required_preview_scales_are_exact_and_ordered() {
        assert_eq!(
            PREVIEW_SCALES.map(|scale| (scale.label, scale.factor)),
            [("100%", 1.0), ("150%", 1.5), ("200%", 2.0)]
        );
    }

    #[test]
    fn every_phase_b_component_has_one_nonempty_spec() {
        let required = [
            GalleryComponent::Button,
            GalleryComponent::Toggle,
            GalleryComponent::Slider,
            GalleryComponent::TextField,
            GalleryComponent::SearchField,
            GalleryComponent::List,
            GalleryComponent::Table,
            GalleryComponent::Tree,
            GalleryComponent::Tabs,
            GalleryComponent::Dialog,
            GalleryComponent::Alert,
            GalleryComponent::ContextMenu,
            GalleryComponent::Tooltip,
            GalleryComponent::Progress,
            GalleryComponent::EmptyState,
            GalleryComponent::Toast,
        ];
        let actual: HashSet<_> = COMPONENT_SPECS
            .iter()
            .map(|spec| {
                assert!(
                    !spec.states.is_empty(),
                    "{} has no states",
                    spec.component.id()
                );
                spec.component
            })
            .collect();

        assert_eq!(
            actual.len(),
            COMPONENT_SPECS.len(),
            "duplicate component spec"
        );
        assert_eq!(actual, required.into_iter().collect());
    }

    #[test]
    fn interactive_specs_cover_focus_and_disabled_or_transient_exit() {
        for spec in COMPONENT_SPECS
            .iter()
            .filter(|spec| spec.keyboard.is_some())
        {
            let has_focus = spec.states.contains(&GalleryState::Focused);
            let is_transient = matches!(
                spec.component,
                GalleryComponent::Dialog
                    | GalleryComponent::Alert
                    | GalleryComponent::ContextMenu
                    | GalleryComponent::Toast
            );
            assert!(
                has_focus || is_transient,
                "{} omits focus",
                spec.component.id()
            );

            if !is_transient {
                assert!(
                    spec.states.contains(&GalleryState::Disabled)
                        || spec.states.contains(&GalleryState::Unavailable),
                    "{} omits disabled/unavailable",
                    spec.component.id()
                );
            }
        }
    }

    #[test]
    fn state_lists_do_not_repeat_entries() {
        for spec in COMPONENT_SPECS {
            let unique: HashSet<_> = spec.states.iter().collect();
            assert_eq!(
                unique.len(),
                spec.states.len(),
                "{} repeats a state",
                spec.component.id()
            );
        }
    }
}
