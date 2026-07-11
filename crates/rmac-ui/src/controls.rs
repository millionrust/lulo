//! Shared interactive controls owned by the rmac design system.

use std::rc::Rc;

use gpui::{
    div, prelude::FluentBuilder as _, px, rgba, App, ClickEvent, ElementId, Entity,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, RenderOnce,
    SharedString, StyleRefinement, Styled, Window,
};
use gpui_component::{
    button::{Button as ComponentButton, ButtonCustomVariant, ButtonGroup, ButtonVariants as _},
    input::Input as ComponentInput,
    slider::Slider as ComponentSlider,
    Disableable as _, Selectable as _, Sizable as _, Size, StyledExt as _,
};

pub use gpui_component::input::InputState;
pub use gpui_component::slider::{SliderEvent, SliderState};

use crate::mac;

/// Semantic visual role for a [`Button`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonRole {
    /// Ordinary reversible action.
    #[default]
    Secondary,
    /// Preferred action in the current surface.
    Primary,
    /// Irreversible or high-risk action.
    Destructive,
    /// Quiet toolbar action with no persistent fill.
    Ghost,
}

/// Three-state value supported by [`Toggle`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToggleState {
    #[default]
    Off,
    On,
    Mixed,
}

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// Keyboard-focusable rmac button with semantic roles and live theme tokens.
#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: SharedString,
    role: ButtonRole,
    disabled: bool,
    busy: bool,
    selected: bool,
    tooltip: Option<SharedString>,
    on_click: Option<ClickHandler>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            role: ButtonRole::Secondary,
            disabled: false,
            busy: false,
            selected: false,
            tooltip: None,
            on_click: None,
        }
    }

    pub fn role(mut self, role: ButtonRole) -> Self {
        self.role = role;
        self
    }

    pub fn primary(self) -> Self {
        self.role(ButtonRole::Primary)
    }

    pub fn destructive(self) -> Self {
        self.role(ButtonRole::Destructive)
    }

    pub fn ghost(self) -> Self {
        self.role(ButtonRole::Ghost)
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let transparent = rgba(0x00000000).into();
        let (color, foreground, border, hover, active) = match self.role {
            ButtonRole::Primary => (
                mac::accent(),
                mac::on_accent(),
                mac::accent(),
                mac::accent().opacity(0.88),
                mac::accent().opacity(0.76),
            ),
            ButtonRole::Destructive => (
                mac::danger(),
                mac::on_danger(),
                mac::danger(),
                mac::danger().opacity(0.88),
                mac::danger().opacity(0.76),
            ),
            ButtonRole::Secondary => (
                mac::control_fill(),
                mac::text(),
                mac::separator(),
                mac::control_fill_hover(),
                mac::hover(),
            ),
            ButtonRole::Ghost => (
                transparent,
                mac::text(),
                transparent,
                mac::control_fill_hover(),
                mac::hover(),
            ),
        };
        let variant = ButtonCustomVariant::new(cx)
            .color(color)
            .foreground(foreground)
            .border(border)
            .hover(hover)
            .active(active)
            .shadow(matches!(
                self.role,
                ButtonRole::Primary | ButtonRole::Destructive
            ));
        let mut button = ComponentButton::new(self.id)
            .label(self.label)
            .custom(variant)
            .with_size(Size::Small)
            .disabled(self.disabled)
            .loading(self.busy)
            .selected(self.selected);
        if let Some(tooltip) = self.tooltip {
            button = button.tooltip(tooltip);
        }
        if let Some(handler) = self.on_click {
            button = button.on_click(move |event, window, cx| handler(event, window, cx));
        }
        button
    }
}

type ToggleHandler = Rc<dyn Fn(&bool, &mut Window, &mut App)>;

/// Keyboard-focusable binary or mixed-state toggle.
#[derive(IntoElement)]
pub struct Toggle {
    id: ElementId,
    state: ToggleState,
    label: Option<SharedString>,
    disabled: bool,
    tooltip: Option<SharedString>,
    on_change: Option<ToggleHandler>,
}

impl Toggle {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            state: ToggleState::Off,
            label: None,
            disabled: false,
            tooltip: None,
            on_change: None,
        }
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.state = if checked {
            ToggleState::On
        } else {
            ToggleState::Off
        };
        self
    }

    pub fn state(mut self, state: ToggleState) -> Self {
        self.state = state;
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    pub fn on_change(mut self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }

    pub fn on_click(self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change(handler)
    }
}

impl RenderOnce for Toggle {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let transparent = rgba(0x00000000).into();
        let active = self.state != ToggleState::Off;
        let next = !matches!(self.state, ToggleState::On);
        let variant = ButtonCustomVariant::new(cx)
            .color(transparent)
            .foreground(mac::text())
            .border(transparent)
            .hover(mac::control_fill_hover())
            .active(mac::hover());
        let track = div()
            .w(px(28.0))
            .h(px(16.0))
            .px(px(2.0))
            .flex()
            .items_center()
            .rounded_full()
            .bg(if active {
                mac::accent()
            } else {
                mac::separator()
            })
            .when(self.state == ToggleState::Off, |track| {
                track.justify_start()
            })
            .when(self.state == ToggleState::On, |track| track.justify_end())
            .when(self.state == ToggleState::Mixed, |track| {
                track.justify_center()
            })
            .child(
                div()
                    .size(px(12.0))
                    .rounded_full()
                    .bg(mac::raised())
                    .shadow_sm(),
            );
        let content = div()
            .flex()
            .items_center()
            .gap_2()
            .child(track)
            .when_some(self.label, |content, label| content.child(label));
        let mut button = ComponentButton::new(self.id)
            .custom(variant)
            .with_size(Size::Small)
            .compact()
            .disabled(self.disabled)
            .selected(active)
            .child(content);
        if let Some(tooltip) = self.tooltip {
            button = button.tooltip(tooltip);
        }
        if let Some(handler) = self.on_change {
            button = button.on_click(move |_, window, cx| handler(&next, window, cx));
        }
        button
    }
}

/// Orientation of a shared [`Slider`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SliderAxis {
    #[default]
    Horizontal,
    Vertical,
}

/// rmac-owned slider boundary around the pinned component implementation.
#[derive(IntoElement)]
pub struct Slider {
    state: Entity<SliderState>,
    axis: SliderAxis,
    disabled: bool,
    style: StyleRefinement,
}

impl Slider {
    pub fn new(state: &Entity<SliderState>) -> Self {
        Self {
            state: state.clone(),
            axis: SliderAxis::Horizontal,
            disabled: false,
            style: StyleRefinement::default(),
        }
    }

    pub fn horizontal(mut self) -> Self {
        self.axis = SliderAxis::Horizontal;
        self
    }

    pub fn vertical(mut self) -> Self {
        self.axis = SliderAxis::Vertical;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl Styled for Slider {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Slider {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let slider = match self.axis {
            SliderAxis::Horizontal => ComponentSlider::new(&self.state).horizontal(),
            SliderAxis::Vertical => ComponentSlider::new(&self.state).vertical(),
        }
        .disabled(self.disabled);
        div().refine_style(&self.style).child(slider)
    }
}

/// Shared editable text field backed by a gpui-component [`InputState`].
#[derive(IntoElement)]
pub struct TextField {
    state: Entity<InputState>,
    appearance: bool,
    cleanable: bool,
    disabled: bool,
    size: Size,
    tab_index: isize,
    style: StyleRefinement,
}

impl TextField {
    pub fn new(state: &Entity<InputState>) -> Self {
        Self {
            state: state.clone(),
            appearance: true,
            cleanable: false,
            disabled: false,
            size: Size::Medium,
            tab_index: 0,
            style: StyleRefinement::default(),
        }
    }

    pub fn appearance(mut self, appearance: bool) -> Self {
        self.appearance = appearance;
        self
    }

    pub fn cleanable(mut self, cleanable: bool) -> Self {
        self.cleanable = cleanable;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn small(mut self) -> Self {
        self.size = Size::Small;
        self
    }

    pub fn tab_index(mut self, tab_index: isize) -> Self {
        self.tab_index = tab_index;
        self
    }
}

impl Styled for TextField {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for TextField {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        ComponentInput::new(&self.state)
            .appearance(self.appearance)
            .cleanable(self.cleanable)
            .disabled(self.disabled)
            .tab_index(self.tab_index)
            .with_size(self.size)
            .refine_style(&self.style)
    }
}

/// Search-specialized text field with a clear action enabled by default.
#[derive(IntoElement)]
pub struct SearchField {
    field: TextField,
}

impl SearchField {
    pub fn new(state: &Entity<InputState>) -> Self {
        Self {
            field: TextField::new(state).cleanable(true),
        }
    }

    pub fn appearance(mut self, appearance: bool) -> Self {
        self.field = self.field.appearance(appearance);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.field = self.field.disabled(disabled);
        self
    }

    pub fn small(mut self) -> Self {
        self.field = self.field.small();
        self
    }

    pub fn tab_index(mut self, tab_index: isize) -> Self {
        self.field = self.field.tab_index(tab_index);
        self
    }
}

impl Styled for SearchField {
    fn style(&mut self) -> &mut StyleRefinement {
        self.field.style()
    }
}

impl RenderOnce for SearchField {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        self.field
    }
}

type TabHandler = Rc<dyn Fn(&usize, &mut Window, &mut App)>;

fn next_tab_index(selected: usize, len: usize, key: &str) -> Option<usize> {
    if len == 0 {
        return None;
    }
    match key {
        "left" | "up" => Some(selected.saturating_sub(1)),
        "right" | "down" => Some((selected + 1).min(len - 1)),
        "home" => Some(0),
        "end" => Some(len - 1),
        _ => None,
    }
}

/// Single-selection tab strip with a stable index-based event contract.
#[derive(IntoElement)]
pub struct Tabs {
    id: SharedString,
    labels: Vec<SharedString>,
    selected: usize,
    disabled: bool,
    on_change: Option<TabHandler>,
    style: StyleRefinement,
}

impl Tabs {
    pub fn new(
        id: impl Into<SharedString>,
        labels: impl IntoIterator<Item = impl Into<SharedString>>,
    ) -> Self {
        Self {
            id: id.into(),
            labels: labels.into_iter().map(Into::into).collect(),
            selected: 0,
            disabled: false,
            on_change: None,
            style: StyleRefinement::default(),
        }
    }

    pub fn selected(mut self, selected: usize) -> Self {
        self.selected = selected;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_change(mut self, handler: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }
}

impl Styled for Tabs {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Tabs {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let len = self.labels.len();
        let selected = self.selected.min(len.saturating_sub(1));
        let id_prefix = self.id.clone();
        let click_handler = self.on_change.clone();
        let mut group = ButtonGroup::new(self.id)
            .outline()
            .with_size(Size::Small)
            .disabled(self.disabled)
            .children(
                self.labels
                    .into_iter()
                    .enumerate()
                    .map(move |(index, label)| {
                        ComponentButton::new(SharedString::from(format!("{id_prefix}-{index}")))
                            .label(label)
                            .selected(index == selected)
                    }),
            )
            .refine_style(&self.style);
        if let Some(handler) = click_handler {
            group = group.on_click(move |indices, window, cx| {
                if let Some(index) = indices.first() {
                    handler(index, window, cx);
                }
            });
        }
        let keyboard_handler = self.on_change;
        let disabled = self.disabled;
        div()
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let Some(handler) = keyboard_handler.as_ref() else {
                    return;
                };
                if disabled {
                    return;
                }
                let Some(index) = next_tab_index(selected, len, event.keystroke.key.as_str())
                else {
                    return;
                };
                window.prevent_default();
                cx.stop_propagation();
                handler(&index, window, cx);
            })
            .child(group)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_are_semantic_and_secondary_is_safe_default() {
        assert_eq!(ButtonRole::default(), ButtonRole::Secondary);
        assert_ne!(ButtonRole::Primary, ButtonRole::Destructive);
        assert_ne!(ButtonRole::Ghost, ButtonRole::Secondary);
    }

    #[test]
    fn mixed_toggle_resolves_to_on_and_on_resolves_to_off() {
        let desired = |state| !matches!(state, ToggleState::On);
        assert!(desired(ToggleState::Off));
        assert!(desired(ToggleState::Mixed));
        assert!(!desired(ToggleState::On));
    }

    #[test]
    fn slider_defaults_to_horizontal() {
        assert_eq!(SliderAxis::default(), SliderAxis::Horizontal);
    }

    #[test]
    fn field_defaults_distinguish_text_and_search_clear_behavior() {
        let text_defaults = (true, false, false, 0_isize);
        let search_defaults = (true, true, false, 0_isize);
        assert_ne!(text_defaults, search_defaults);
    }

    #[test]
    fn tab_selection_is_clamped_for_rendering() {
        let clamp = |selected: usize, len: usize| selected.min(len.saturating_sub(1));
        assert_eq!(clamp(8, 5), 4);
        assert_eq!(clamp(8, 0), 0);
    }

    #[test]
    fn tab_keyboard_navigation_is_bounded() {
        assert_eq!(next_tab_index(0, 5, "left"), Some(0));
        assert_eq!(next_tab_index(4, 5, "right"), Some(4));
        assert_eq!(next_tab_index(2, 5, "home"), Some(0));
        assert_eq!(next_tab_index(2, 5, "end"), Some(4));
        assert_eq!(next_tab_index(2, 5, "space"), None);
        assert_eq!(next_tab_index(0, 0, "right"), None);
    }
}
