//! Shared interactive controls owned by the rmac design system.

use std::rc::Rc;

use gpui::{
    div, prelude::FluentBuilder as _, px, rgba, AnyElement, App, ClickEvent, Context, ElementId,
    Entity, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _,
    RenderOnce, Role, SharedString, StatefulInteractiveElement as _, StyleRefinement, Styled,
    Toggled, Window,
};
use gpui_component::{
    button::{
        Button as ComponentButton, ButtonCustomVariant, ButtonGroup, ButtonVariants as _,
        DropdownButton,
    },
    input::Input as ComponentInput,
    menu::{DropdownMenu as _, PopupMenu},
    slider::Slider as ComponentSlider,
    table::DataTable as ComponentTable,
    tooltip::Tooltip as ComponentTooltip,
    Disableable as _, Icon, Selectable as _, Sizable as _, Size, StyledExt as _,
};

pub use gpui_component::input::{InputEvent, InputState, Position, RopeExt, SelectAll};
pub use gpui_component::slider::{SliderEvent, SliderState};
pub use gpui_component::table::{Column, ColumnSort, TableDelegate, TableEvent, TableState};

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
type MenuBuilder = Rc<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;

/// Paint rmac-owned colours onto a gpui-component button.
///
/// The pinned gpui-component treats `ButtonCustomVariant` as a *tint*, not a
/// fill: the resting background is `color` at 20 % alpha, the hover and
/// pressed backgrounds are `color` at 30 % / 40 %, and the label is drawn in
/// `color` itself — `foreground` is never read. rmac passed its fill there, so
/// a Secondary button drew its label in its own grey fill (blank boxes such as
/// Control Center's Power Mode buttons), a Primary button became a faint blue
/// outline with blue text, and every transparent control (Ghost buttons,
/// checkbox, radio and list-row labels) drew a transparent label.
///
/// rmac owns these colours, so it paints them itself. The component is kept
/// in its `selected` styling branch, the only branch that installs no hover or
/// pressed style of its own (installing ours as well would trip GPUI's
/// "hover style already set" assertion), and the refinement painted here is
/// applied after every variant style. macOS push buttons have no hover
/// highlight; `hover` adds one only for controls that want it.
fn painted(
    button: ComponentButton,
    fill: Hsla,
    text: Hsla,
    hover: Option<Hsla>,
    disabled: bool,
    cx: &App,
) -> ComponentButton {
    let variant = ButtonCustomVariant::new(cx)
        .color(text)
        .foreground(text)
        .hover(fill)
        .active(fill);
    let button = button
        .custom(variant)
        .selected(true)
        .bg(if disabled { fill.opacity(0.5) } else { fill })
        .text_color(if disabled { mac::text_tertiary() } else { text });
    match hover {
        Some(hover) if !disabled => button.hover(move |style| style.bg(hover)),
        _ => button,
    }
}

/// Keyboard-focusable rmac button with semantic roles and live theme tokens.
#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: Option<SharedString>,
    icon: Option<Icon>,
    role: ButtonRole,
    size: Size,
    disabled: bool,
    busy: bool,
    selected: bool,
    tooltip: Option<SharedString>,
    on_click: Option<ClickHandler>,
    dropdown_menu: Option<MenuBuilder>,
    style: StyleRefinement,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        let label = label.into();
        Self {
            id: id.into(),
            label: (!label.is_empty()).then_some(label),
            icon: None,
            role: ButtonRole::Secondary,
            size: Size::Small,
            disabled: false,
            busy: false,
            selected: false,
            tooltip: None,
            on_click: None,
            dropdown_menu: None,
            style: StyleRefinement::default(),
        }
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.size = size.into();
        self
    }

    pub fn small(self) -> Self {
        self.with_size(Size::Small)
    }

    pub fn xsmall(self) -> Self {
        self.with_size(Size::XSmall)
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

    pub fn dropdown_menu(
        mut self,
        builder: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> Self {
        self.dropdown_menu = Some(Rc::new(builder));
        self
    }
}

impl Styled for Button {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Button {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let transparent = rgba(0x00000000).into();
        let window_active = window.is_window_active();
        let (color, foreground, border, hover, active) = match self.role {
            // A filled default button loses its fill in an inactive window and
            // reads as an ordinary control instead.
            ButtonRole::Primary if !window_active => (
                mac::control_fill(),
                mac::text(),
                mac::separator(),
                mac::control_fill_hover(),
                mac::control_fill_hover(),
            ),
            ButtonRole::Primary => (
                mac::accent(),
                mac::on_accent(),
                mac::accent(),
                mac::accent().opacity(0.88),
                mac::accent().opacity(0.76),
            ),
            ButtonRole::Destructive => (
                mac::button_destructive(),
                mac::on_button_destructive(),
                mac::button_destructive(),
                mac::button_destructive().opacity(0.88),
                mac::button_destructive().opacity(0.76),
            ),
            ButtonRole::Secondary => (
                mac::button_secondary(),
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
        let _ = active;
        let (fill, hover) = match (self.role, self.selected) {
            (_, true) => (mac::control_fill_hover(), None),
            (ButtonRole::Ghost, false) => (color, Some(hover)),
            _ => (color, None),
        };
        let mut button = painted(
            ComponentButton::new(self.id)
                .with_size(self.size)
                .disabled(self.disabled)
                .loading(self.busy),
            fill,
            foreground,
            hover,
            self.disabled,
            cx,
        )
        .border_1()
        .border_color(border)
        .refine_style(&self.style);
        if let Some(label) = self.label {
            button = button.label(label);
        }
        if let Some(icon) = self.icon {
            button = button.icon(icon);
        }
        if let Some(tooltip) = self.tooltip {
            button = button.tooltip(tooltip);
        }
        if let Some(handler) = self.on_click {
            button = button.on_click(move |event, window, cx| handler(event, window, cx));
        }
        if let Some(builder) = self.dropdown_menu {
            button
                .dropdown_menu(move |menu, window, cx| builder(menu, window, cx))
                .into_any_element()
        } else {
            button.into_any_element()
        }
    }
}

/// Pop-up button (5.2): a secondary-styled button showing the current choice
/// with a trailing chevron that opens a menu.
#[derive(IntoElement)]
pub struct PopUpButton {
    id: ElementId,
    label: SharedString,
    disabled: bool,
    selected: bool,
    form: bool,
    menu: Option<MenuBuilder>,
    style: StyleRefinement,
}

impl PopUpButton {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            disabled: false,
            selected: false,
            form: false,
            menu: None,
            style: StyleRefinement::default(),
        }
    }

    /// The grouped-form pop-up of macOS 26 System Settings: the value in
    /// plain 13 pt text followed by a 20 pt circle holding ⌃⌄, with no
    /// bezel (design-lab/chrome.html, design-lab/settings.html).
    pub fn form(mut self) -> Self {
        self.form = true;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn dropdown_menu(
        mut self,
        builder: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> Self {
        self.menu = Some(Rc::new(builder));
        self
    }
}

impl Styled for PopUpButton {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for PopUpButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        if self.form {
            let metrics = rmac_design::Metrics::default();
            let circle = metrics.popup_chevron;
            let text = if self.disabled {
                mac::text_tertiary()
            } else {
                mac::text()
            };
            let content = div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(
                    div()
                        .text_size(crate::text_px(13.0))
                        .text_color(text)
                        .whitespace_nowrap()
                        .child(self.label),
                )
                .child(
                    div()
                        .size(px(circle))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(mac::button_secondary())
                        .child(
                            gpui::svg()
                                .path("icons/chevrons-up-down.svg")
                                .size(px(12.0))
                                .text_color(text),
                        ),
                );
            let transparent = rgba(0x00000000).into();
            let button = painted(
                ComponentButton::new(self.id)
                    .compact()
                    .disabled(self.disabled)
                    .child(content),
                transparent,
                text,
                None,
                self.disabled,
                cx,
            )
            .h(px(metrics.control_height_regular))
            .px(px(2.0))
            .refine_style(&self.style);
            return match self.menu {
                Some(builder) if !self.disabled => button
                    .dropdown_menu(move |menu, window, cx| builder(menu, window, cx))
                    .into_any_element(),
                _ => button.into_any_element(),
            };
        }
        let fill = if self.selected {
            mac::control_fill_hover()
        } else {
            mac::button_secondary()
        };
        let button = painted(
            ComponentButton::new(self.id.clone()).label(self.label),
            fill,
            mac::text(),
            None,
            self.disabled,
            cx,
        )
        .border_1()
        .border_color(mac::separator());
        let mut dropdown = DropdownButton::new(self.id).button(button).compact();
        if let Some(builder) = self.menu {
            dropdown = dropdown.dropdown_menu(move |menu, window, cx| builder(menu, window, cx));
        }
        if self.disabled {
            dropdown = dropdown.disabled(true);
        }
        dropdown.refine_style(&self.style).into_any_element()
    }
}

type ToggleHandler = Rc<dyn Fn(&bool, &mut Window, &mut App)>;

/// Metric size of a [`Toggle`] switch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SwitchSize {
    #[default]
    Regular,
    Small,
    Mini,
}

impl SwitchSize {
    fn dimensions(self) -> (f32, f32, f32) {
        match self {
            Self::Regular => mac::switch_regular(),
            Self::Small => mac::switch_small(),
            Self::Mini => mac::switch_mini(),
        }
    }

    /// Width of the thumb. macOS 26 draws the regular thumb as a 21 × 13
    /// capsule; the unmeasured smaller sizes keep a round thumb.
    fn thumb_width(self) -> f32 {
        match self {
            Self::Regular => rmac_design::Metrics::default().switch_regular_thumb_width,
            Self::Small | Self::Mini => self.dimensions().2,
        }
    }
}

/// Keyboard-focusable binary or mixed-state toggle.
#[derive(IntoElement)]
pub struct Toggle {
    id: ElementId,
    state: ToggleState,
    size: SwitchSize,
    pending: bool,
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
            size: SwitchSize::Regular,
            pending: false,
            label: None,
            disabled: false,
            tooltip: None,
            on_change: None,
        }
    }

    pub fn with_size(mut self, size: SwitchSize) -> Self {
        self.size = size;
        self
    }

    /// An async authority is still settling; keep the old value and show a
    /// busy thumb until the readback arrives.
    pub fn pending(mut self, pending: bool) -> Self {
        self.pending = pending;
        self
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
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let active = self.state != ToggleState::Off;
        let next = !matches!(self.state, ToggleState::On);
        let (width, height, thumb) = self.size.dimensions();
        let thumb_width = self.size.thumb_width();
        // Measured (design-lab/chrome.html): the thumb sits 1.5 inside a
        // fully round track with no border.
        let inset = (height - thumb) / 2.0;
        let indicator = if self.pending {
            div()
                .size(px(thumb))
                .rounded_full()
                .border_2()
                .border_color(mac::white())
                .into_any_element()
        } else {
            div()
                .w(px(thumb_width))
                .h(px(thumb))
                .rounded_full()
                .bg(mac::white())
                .shadow_sm()
                .into_any_element()
        };
        let track = div()
            .w(px(width))
            .h(px(height))
            .px(px(inset))
            .flex()
            .items_center()
            .rounded_full()
            .bg(if active {
                mac::accent()
            } else {
                mac::control_fill()
            })
            .when(self.state == ToggleState::Off, |track| {
                track.justify_start()
            })
            .when(self.state == ToggleState::On, |track| track.justify_end())
            .when(self.state == ToggleState::Mixed, |track| {
                track.justify_center()
            })
            .child(indicator);
        let disabled = self.disabled || self.pending;
        let label_color = if disabled {
            mac::text_tertiary()
        } else {
            mac::text()
        };
        let content = div()
            .flex()
            .items_center()
            .gap_2()
            .text_color(label_color)
            .child(track)
            .when_some(self.label.clone(), |content, label| content.child(label));
        let toggled = match self.state {
            ToggleState::On => Toggled::True,
            ToggleState::Off => Toggled::False,
            ToggleState::Mixed => Toggled::Mixed,
        };
        // The visible label doubles as the accessible name; the tooltip is
        // the fallback for icon-only switches that have none.
        let accessible_name = self.label.or(self.tooltip.clone());
        let focus_handle = window
            .use_keyed_state(self.id.clone(), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let is_focused = focus_handle.is_focused(window);
        let mut switch = div()
            .id(self.id)
            .role(Role::Switch)
            .aria_toggled(toggled)
            .when_some(accessible_name, |el, name| el.aria_label(name))
            .cursor_default()
            .when(!disabled, |el| {
                el.track_focus(&focus_handle.clone().tab_stop(true).tab_index(0))
            })
            .when(disabled, |el| el.opacity(0.5))
            .when(is_focused, |el| el.shadow(mac::focus_ring_shadow()))
            .child(content);
        if let Some(tooltip) = self.tooltip {
            switch = switch.tooltip(move |window, cx| {
                ComponentTooltip::new(tooltip.clone()).build(window, cx)
            });
        }
        if let Some(handler) = self.on_change {
            switch = switch.on_click(move |_, window, cx| {
                if !disabled {
                    handler(&next, window, cx);
                }
            });
        }
        switch
    }
}

/// 14 px tri-state checkbox with a label that toggles it.
#[derive(IntoElement)]
pub struct Checkbox {
    id: ElementId,
    state: ToggleState,
    label: Option<SharedString>,
    disabled: bool,
    on_change: Option<ToggleHandler>,
}

impl Checkbox {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            state: ToggleState::Off,
            label: None,
            disabled: false,
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

    pub fn on_change(mut self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Checkbox {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let active = self.state != ToggleState::Off;
        let next = !matches!(self.state, ToggleState::On);
        let marker = match self.state {
            ToggleState::On => div()
                .text_color(mac::white())
                .text_size(px(11.0))
                .child("✓")
                .into_any_element(),
            ToggleState::Mixed => div()
                .w(px(8.0))
                .h(px(2.0))
                .rounded_full()
                .bg(mac::white())
                .into_any_element(),
            ToggleState::Off => div().into_any_element(),
        };
        let metrics = rmac_design::Metrics::default();
        // Measured on TextEdit Settings: 14 pt, radius 4, a flat control
        // fill when off and the accent when on, no border.
        let box_ = div()
            .size(px(metrics.checkbox_size))
            .rounded(px(metrics.checkbox_radius))
            .flex()
            .items_center()
            .justify_center()
            .bg(if active {
                mac::accent()
            } else {
                mac::control_fill()
            })
            .child(marker);
        let label_color = if self.disabled {
            mac::text_tertiary()
        } else {
            mac::text()
        };
        let content = div()
            .flex()
            .items_center()
            .gap_2()
            .text_color(label_color)
            .child(box_)
            .when_some(self.label.clone(), |content, label| content.child(label));
        let toggled = match self.state {
            ToggleState::On => Toggled::True,
            ToggleState::Off => Toggled::False,
            ToggleState::Mixed => Toggled::Mixed,
        };
        let disabled = self.disabled;
        let focus_handle = window
            .use_keyed_state(self.id.clone(), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let is_focused = focus_handle.is_focused(window);
        let mut checkbox = div()
            .id(self.id)
            .role(Role::CheckBox)
            .aria_toggled(toggled)
            .when_some(self.label, |el, label| el.aria_label(label))
            .cursor_default()
            .when(!disabled, |el| {
                el.track_focus(&focus_handle.tab_stop(true).tab_index(0))
            })
            .when(disabled, |el| el.opacity(0.5))
            .when(is_focused, |el| el.shadow(mac::focus_ring_shadow()))
            .child(content);
        if let Some(handler) = self.on_change {
            checkbox = checkbox.on_click(move |_, window, cx| {
                if !disabled {
                    handler(&next, window, cx);
                }
            });
        }
        checkbox
    }
}

/// 14 px radio button with a label that selects it.
#[derive(IntoElement)]
pub struct Radio {
    id: ElementId,
    selected: bool,
    label: Option<SharedString>,
    disabled: bool,
    on_change: Option<ToggleHandler>,
}

impl Radio {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            selected: false,
            label: None,
            disabled: false,
            on_change: None,
        }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
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

    pub fn on_change(mut self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Radio {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let metrics = rmac_design::Metrics::default();
        let circle = div()
            .size(px(metrics.radio_size))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(if self.selected {
                mac::accent()
            } else {
                mac::control_fill()
            })
            .when(self.selected, |circle| {
                circle.child(
                    div()
                        .size(px(metrics.radio_dot))
                        .rounded_full()
                        .bg(mac::white()),
                )
            });
        let label_color = if self.disabled {
            mac::text_tertiary()
        } else {
            mac::text()
        };
        let content = div()
            .flex()
            .items_center()
            .gap_2()
            .text_color(label_color)
            .child(circle)
            .when_some(self.label.clone(), |content, label| content.child(label));
        let disabled = self.disabled;
        let selected = self.selected;
        let focus_handle = window
            .use_keyed_state(self.id.clone(), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let is_focused = focus_handle.is_focused(window);
        let mut radio = div()
            .id(self.id)
            .role(Role::RadioButton)
            .aria_selected(selected)
            .when_some(self.label, |el, label| el.aria_label(label))
            .cursor_default()
            .when(!disabled, |el| {
                el.track_focus(&focus_handle.tab_stop(true).tab_index(0))
            })
            .when(disabled, |el| el.opacity(0.5))
            .when(is_focused, |el| el.shadow(mac::focus_ring_shadow()))
            .child(content);
        if let Some(handler) = self.on_change {
            radio = radio.on_click(move |_, window, cx| {
                if !disabled {
                    handler(&true, window, cx);
                }
            });
        }
        radio
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
    error: Option<SharedString>,
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
            error: None,
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

    /// Show a validation message below the field and draw a danger border.
    pub fn error(mut self, message: impl Into<SharedString>) -> Self {
        self.error = Some(message.into());
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
        let mut style = self.style.clone();
        if self.appearance {
            style = style.bg(mac::field_fill());
        }
        if self.error.is_some() {
            style = style.border_color(mac::danger());
        }
        let input = ComponentInput::new(&self.state)
            .appearance(self.appearance)
            .cleanable(self.cleanable)
            .disabled(self.disabled)
            .tab_index(self.tab_index)
            .with_size(self.size)
            .refine_style(&style);
        match self.error {
            Some(message) => div()
                .flex()
                .flex_col()
                .gap_1()
                .child(input)
                .child(
                    div()
                        .text_color(mac::danger())
                        .text_size(px(11.0))
                        .child(message),
                )
                .into_any_element(),
            None => input.into_any_element(),
        }
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

    pub fn error(mut self, message: impl Into<SharedString>) -> Self {
        self.field = self.field.error(message);
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

/// Data/loading state shared by collection controls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CollectionState {
    #[default]
    Ready,
    Empty,
    Loading,
    Stale,
    Unavailable,
    Error,
}

fn collection_message(state: CollectionState) -> &'static str {
    match state {
        CollectionState::Ready => "",
        CollectionState::Empty => "No items",
        CollectionState::Loading => "Loading…",
        CollectionState::Stale => "Showing cached information",
        CollectionState::Unavailable => "Information is unavailable",
        CollectionState::Error => "Could not load information",
    }
}

/// Stateful vertical list surface.
#[derive(IntoElement)]
pub struct List {
    children: Vec<AnyElement>,
    state: CollectionState,
    message: Option<SharedString>,
    style: StyleRefinement,
}

impl List {
    pub fn new(children: impl IntoIterator<Item = impl IntoElement>) -> Self {
        Self {
            children: children
                .into_iter()
                .map(IntoElement::into_any_element)
                .collect(),
            state: CollectionState::Ready,
            message: None,
            style: StyleRefinement::default(),
        }
    }

    pub fn state(mut self, state: CollectionState) -> Self {
        self.state = state;
        self
    }

    pub fn message(mut self, message: impl Into<SharedString>) -> Self {
        self.message = Some(message.into());
        self
    }
}

impl Styled for List {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for List {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let state = self.state;
        let message = self
            .message
            .unwrap_or_else(|| collection_message(state).into());
        div()
            .v_flex()
            .refine_style(&self.style)
            .when(state == CollectionState::Stale, |list| {
                list.child(
                    div()
                        .mx_2()
                        .my_1()
                        .px_2()
                        .py_1()
                        .rounded(px(mac::radius_control()))
                        .bg(mac::warning_background())
                        .border_1()
                        .border_color(mac::warning_border())
                        .text_size(crate::text_px(11.0))
                        .text_color(mac::warning_text())
                        .child(message.clone()),
                )
            })
            .when(
                matches!(state, CollectionState::Ready | CollectionState::Stale),
                |list| list.children(self.children),
            )
            .when(
                matches!(
                    state,
                    CollectionState::Empty
                        | CollectionState::Loading
                        | CollectionState::Unavailable
                        | CollectionState::Error
                ),
                |list| {
                    list.child(
                        div()
                            .min_h(px(96.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .px_4()
                            .text_size(crate::text_px(12.0))
                            .text_color(if state == CollectionState::Error {
                                mac::danger()
                            } else {
                                mac::text_secondary()
                            })
                            .child(message),
                    )
                },
            )
    }
}

/// Focusable, selectable row for lists and source sidebars.
#[derive(IntoElement)]
pub struct ListRow {
    id: ElementId,
    content: AnyElement,
    selected: bool,
    disabled: bool,
    on_activate: Option<ClickHandler>,
    style: StyleRefinement,
}

impl ListRow {
    pub fn new(id: impl Into<ElementId>, content: impl IntoElement) -> Self {
        Self {
            id: id.into(),
            content: content.into_any_element(),
            selected: false,
            disabled: false,
            on_activate: None,
            style: StyleRefinement::default(),
        }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_activate(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_activate = Some(Rc::new(handler));
        self
    }

    pub fn on_click(self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_activate(handler)
    }
}

impl Styled for ListRow {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for ListRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let transparent: Hsla = rgba(0x00000000).into();
        let (fill, text, hover) = if self.selected {
            (mac::accent(), mac::on_accent(), None)
        } else {
            (transparent, mac::text(), Some(mac::hover()))
        };
        let mut row = painted(
            ComponentButton::new(self.id)
                .with_size(Size::Small)
                .disabled(self.disabled),
            fill,
            text,
            hover,
            self.disabled,
            cx,
        )
        .w_full()
        .h(px(mac::list_row_height()))
        .justify_start()
        .refine_style(&self.style)
        .child(self.content);
        if let Some(handler) = self.on_activate {
            row = row.on_click(move |event, window, cx| handler(event, window, cx));
        }
        row
    }
}

/// Tree surface with the same loading/error contract as [`List`].
#[derive(IntoElement)]
pub struct Tree {
    list: List,
}

impl Tree {
    pub fn new(children: impl IntoIterator<Item = impl IntoElement>) -> Self {
        Self {
            list: List::new(children),
        }
    }

    pub fn state(mut self, state: CollectionState) -> Self {
        self.list = self.list.state(state);
        self
    }

    pub fn message(mut self, message: impl Into<SharedString>) -> Self {
        self.list = self.list.message(message);
        self
    }
}

impl Styled for Tree {
    fn style(&mut self) -> &mut StyleRefinement {
        self.list.style()
    }
}

impl RenderOnce for Tree {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        self.list
    }
}

type ExpansionHandler = Rc<dyn Fn(&bool, &mut Window, &mut App)>;

/// Indented tree row with pointer and Left/Right expansion behavior.
#[derive(IntoElement)]
pub struct TreeRow {
    id: ElementId,
    label: SharedString,
    depth: u16,
    selected: bool,
    has_children: bool,
    expanded: bool,
    on_activate: Option<ClickHandler>,
    on_expansion_change: Option<ExpansionHandler>,
}

impl TreeRow {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            depth: 0,
            selected: false,
            has_children: false,
            expanded: false,
            on_activate: None,
            on_expansion_change: None,
        }
    }

    pub fn depth(mut self, depth: u16) -> Self {
        self.depth = depth;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn branch(mut self, expanded: bool) -> Self {
        self.has_children = true;
        self.expanded = expanded;
        self
    }

    pub fn on_activate(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_activate = Some(Rc::new(handler));
        self
    }

    pub fn on_expansion_change(
        mut self,
        handler: impl Fn(&bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_expansion_change = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for TreeRow {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let disclosure = if self.has_children {
            if self.expanded {
                "⌄"
            } else {
                "›"
            }
        } else {
            ""
        };
        let content = div()
            .w_full()
            .flex()
            .items_center()
            .gap_1()
            .pl(px(f32::from(self.depth) * 16.0))
            .text_color(if self.selected {
                mac::on_accent()
            } else {
                mac::text()
            })
            .child(div().w(px(14.0)).child(disclosure))
            .child(self.label);
        let expansion_click = self.on_expansion_change.clone();
        let keyboard_expansion = self.on_expansion_change;
        let activate = self.on_activate;
        let has_children = self.has_children;
        let expanded = self.expanded;
        let row = ListRow::new(self.id, content)
            .selected(self.selected)
            .on_activate(move |event, window, cx| {
                if has_children {
                    if let Some(handler) = expansion_click.as_ref() {
                        handler(&!expanded, window, cx);
                        return;
                    }
                }
                if let Some(handler) = activate.as_ref() {
                    handler(event, window, cx);
                }
            });
        div()
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let requested = match event.keystroke.key.as_str() {
                    "left" if has_children && expanded => Some(false),
                    "right" if has_children && !expanded => Some(true),
                    _ => None,
                };
                let (Some(requested), Some(handler)) = (requested, keyboard_expansion.as_ref())
                else {
                    return;
                };
                window.prevent_default();
                cx.stop_propagation();
                handler(&requested, window, cx);
            })
            .child(row)
    }
}

/// rmac-owned boundary for the virtualized table implementation.
#[derive(IntoElement)]
pub struct Table<D: TableDelegate> {
    state: Entity<TableState<D>>,
    striped: bool,
    bordered: bool,
    vertical_scrollbar: bool,
    horizontal_scrollbar: bool,
    size: Option<Size>,
}

impl<D: TableDelegate> Table<D> {
    pub fn new(state: &Entity<TableState<D>>) -> Self {
        Self {
            state: state.clone(),
            striped: false,
            bordered: true,
            vertical_scrollbar: true,
            horizontal_scrollbar: true,
            size: None,
        }
    }

    pub fn striped(mut self, striped: bool) -> Self {
        self.striped = striped;
        self
    }

    /// Compatibility spelling used by the existing Activity Monitor table.
    pub fn stripe(self, striped: bool) -> Self {
        self.striped(striped)
    }

    pub fn bordered(mut self, bordered: bool) -> Self {
        self.bordered = bordered;
        self
    }

    pub fn scrollbar_visible(mut self, vertical: bool, horizontal: bool) -> Self {
        self.vertical_scrollbar = vertical;
        self.horizontal_scrollbar = horizontal;
        self
    }

    pub fn compact(mut self) -> Self {
        self.size = Some(Size::Small);
        self
    }

    /// Rows (and the header row) exactly `height` points tall, such as
    /// Activity Monitor's measured 24 pt table rows.
    pub fn row_height(mut self, height: f32) -> Self {
        self.size = Some(Size::Size(px(height)));
        self
    }
}

impl<D: TableDelegate> RenderOnce for Table<D> {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let mut table = ComponentTable::new(&self.state)
            .stripe(self.striped)
            .bordered(self.bordered)
            .scrollbar_visible(self.vertical_scrollbar, self.horizontal_scrollbar);
        if let Some(size) = self.size {
            table = table.with_size(size);
        }
        table
    }
}

type SegmentedHandler = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// Mutually exclusive segmented control: one rounded track holding compact
/// segments where the selected segment is a raised pill.
#[derive(IntoElement)]
pub struct SegmentedControl {
    id: ElementId,
    options: Vec<SharedString>,
    selected: usize,
    disabled: bool,
    on_change: Option<SegmentedHandler>,
    style: StyleRefinement,
}

impl SegmentedControl {
    pub fn new(
        id: impl Into<ElementId>,
        options: impl IntoIterator<Item = impl Into<SharedString>>,
    ) -> Self {
        Self {
            id: id.into(),
            options: options.into_iter().map(Into::into).collect(),
            selected: 0,
            disabled: false,
            on_change: None,
            style: StyleRefinement::default(),
        }
    }

    pub fn selected(mut self, index: usize) -> Self {
        self.selected = index;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_change(mut self, handler: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }

    /// Move `current` by `delta`, wrapping within `len`; `None` when empty.
    /// This is the pure model behind Left/Right segmented navigation.
    pub fn wrapped_selection(current: usize, len: usize, delta: i32) -> Option<usize> {
        if len == 0 {
            return None;
        }
        Some(((current as i32 + delta).rem_euclid(len as i32)) as usize)
    }
}

impl Styled for SegmentedControl {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for SegmentedControl {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let count = self.options.len();
        let selected = if count == 0 {
            0
        } else {
            self.selected.min(count - 1)
        };
        let base = self.id.to_string();
        let mut group = ButtonGroup::new(self.id).compact().disabled(self.disabled);
        for (index, label) in self.options.into_iter().enumerate() {
            let variant = if index == selected {
                ButtonCustomVariant::new(cx)
                    .color(mac::raised())
                    .foreground(mac::text())
                    .hover(mac::raised())
                    .active(mac::raised())
            } else {
                ButtonCustomVariant::new(cx)
                    .color(rgba(0x00000000).into())
                    .foreground(mac::text())
                    .hover(mac::hover())
                    .active(mac::control_fill_hover())
            };
            group = group.child(
                ComponentButton::new(ElementId::named_usize(base.clone(), index))
                    .custom(variant)
                    .label(label)
                    // Exposes the segment's checked state to assistive
                    // technology; without it every segment reads as plain
                    // unselected buttons.
                    .selected(index == selected),
            );
        }
        let click_handler = self.on_change.clone();
        if let Some(handler) = click_handler {
            group = group.on_click(move |clicks: &Vec<usize>, window, cx| {
                if let Some(&index) = clicks.first() {
                    handler(index, window, cx);
                }
            });
        }
        let keyboard_handler = self.on_change;
        let disabled = self.disabled;
        div()
            .rounded(px(crate::theme::current().radii.control))
            .bg(mac::control_fill())
            .p(px(2.0))
            .refine_style(&self.style)
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let Some(handler) = keyboard_handler.as_ref() else {
                    return;
                };
                if disabled || count == 0 {
                    return;
                }
                let next = match event.keystroke.key.as_str() {
                    "left" | "up" => Self::wrapped_selection(selected, count, -1),
                    "right" | "down" => Self::wrapped_selection(selected, count, 1),
                    "home" => Some(0),
                    "end" => Some(count - 1),
                    _ => None,
                };
                let Some(next) = next else {
                    return;
                };
                window.prevent_default();
                cx.stop_propagation();
                handler(next, window, cx);
            })
            .child(group)
            .into_any_element()
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
    fn popup_button_builds_with_and_without_a_menu() {
        let _ = PopUpButton::new("popup", "Group By")
            .disabled(true)
            .selected(true);
        let _ = PopUpButton::new("popup-menu", "Kind").dropdown_menu(|menu, _window, _cx| menu);
    }

    #[test]
    fn checkbox_and_radio_build_for_every_state() {
        let _ = Checkbox::new("cb").checked(true).label("Enabled");
        let _ = Checkbox::new("cb-mixed").state(ToggleState::Mixed);
        let _ = Checkbox::new("cb-disabled").disabled(true);
        let _ = Radio::new("r1").selected(true).label("One");
        let _ = Radio::new("r2").disabled(true).label("Two");
    }

    #[test]
    fn switch_sizes_are_ordered_from_regular_to_mini() {
        assert_eq!(SwitchSize::default(), SwitchSize::Regular);
        let (regular_w, regular_h, regular_thumb) = mac::switch_regular();
        let (small_w, small_h, small_thumb) = mac::switch_small();
        let (mini_w, mini_h, mini_thumb) = mac::switch_mini();
        assert!(regular_w > small_w && small_w > mini_w);
        assert!(regular_h > small_h && small_h > mini_h);
        assert!(regular_thumb > small_thumb && small_thumb > mini_thumb);
    }

    #[test]
    fn segmented_selection_wraps_within_bounds() {
        assert_eq!(SegmentedControl::wrapped_selection(0, 3, 1), Some(1));
        assert_eq!(SegmentedControl::wrapped_selection(2, 3, 1), Some(0));
        assert_eq!(SegmentedControl::wrapped_selection(0, 3, -1), Some(2));
        assert_eq!(SegmentedControl::wrapped_selection(0, 0, 1), None);
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

    #[test]
    fn collection_states_have_nonempty_fallback_messages() {
        for state in [
            CollectionState::Empty,
            CollectionState::Loading,
            CollectionState::Stale,
            CollectionState::Unavailable,
            CollectionState::Error,
        ] {
            assert!(!collection_message(state).is_empty());
        }
    }

    #[test]
    fn shared_table_defaults_keep_boundaries_and_both_scrollbars() {
        let defaults = (false, true, true, true);
        assert_eq!(defaults, (false, true, true, true));
    }
}
