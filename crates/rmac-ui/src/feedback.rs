//! Shared transient, status, and empty-content feedback surfaces.

use std::rc::Rc;

use gpui::{
    div, prelude::FluentBuilder as _, px, relative, App, ClickEvent, ElementId,
    InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce, Role, SharedString,
    StatefulInteractiveElement as _, StyleRefinement, Styled, Window,
};
use gpui_component::StyledExt as _;

use crate::mac;

/// The application boundary presenting an internal failure to a person.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorSurface {
    Settings,
    Files,
}

/// Reduce an internal error chain to one calm, bounded sentence. Domain
/// errors remain intact for diagnostics; only the visible and accessibility
/// projections pass through this boundary.
pub fn user_error_message(
    surface: ErrorSurface,
    raw: &str,
    recovery_pending: bool,
) -> SharedString {
    let lower = raw.to_ascii_lowercase();
    if surface == ErrorSurface::Files
        && lower.contains("trash")
        && (lower.contains("recovery") || lower.contains("verified"))
    {
        return if recovery_pending {
            "Review interrupted Trash operations before continuing.".into()
        } else {
            "Trash isn’t available right now—try again.".into()
        };
    }
    if surface == ErrorSurface::Files && recovery_pending {
        return "Review interrupted file operations before continuing.".into();
    }

    let first_line = raw.lines().next().unwrap_or_default().trim();
    let headline = first_line
        .split_once("Caused by")
        .map_or(first_line, |(headline, _)| headline);
    let headline = headline
        .split_once(':')
        .map_or(headline, |(headline, _)| headline);
    let headline = headline
        .split_once(';')
        .map_or(headline, |(headline, _)| headline)
        .trim();
    let headline_lower = headline.to_ascii_lowercase();
    let exposes_internal_name = [
        "niri",
        "socket",
        "dbus", // wording: internal
        "wayland",
        "caused by",
        "backtrace",
    ]
    .iter()
    .any(|term| headline_lower.contains(term));

    if headline.is_empty() || exposes_internal_name {
        return match surface {
            ErrorSurface::Settings => "This setting isn’t available right now—try again.".into(),
            ErrorSurface::Files => "Files couldn’t complete that action—try again.".into(),
        };
    }

    let sentence_end = headline
        .char_indices()
        .find_map(|(index, character)| matches!(character, '.' | '!' | '?').then_some(index + 1));
    let sentence = sentence_end.map_or(headline, |end| &headline[..end]).trim();
    let sentence = if sentence.len() <= 160 {
        sentence.to_owned()
    } else {
        let mut end = 160;
        while !sentence.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", sentence[..end].trim_end())
    };
    if sentence.ends_with(['.', '!', '?', '…']) {
        sentence.into()
    } else {
        format!("{sentence}.").into()
    }
}

/// Compact explanatory surface used by hover/focus tooltip hosts.
#[derive(IntoElement)]
pub struct Tooltip {
    text: SharedString,
}

impl Tooltip {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self { text: text.into() }
    }
}

impl RenderOnce for Tooltip {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .max_w(px(280.0))
            .px_2()
            .py_1()
            .rounded(px(mac::radius_control()))
            .bg(mac::text())
            .text_color(mac::window())
            .text_size(crate::text_px(11.0))
            .shadow_lg()
            .child(self.text)
    }
}

/// Twelve-spoke activity indicator. The owner advances `phase` on a timer;
/// a hidden spinner stops rendering, so it costs nothing while idle.
#[derive(IntoElement)]
pub struct Spinner {
    size: f32,
    phase: u8,
    style: StyleRefinement,
}

const SPINNER_SPOKES: u8 = 12;

impl Spinner {
    pub fn new() -> Self {
        Self {
            size: 16.0,
            phase: 0,
            style: StyleRefinement::default(),
        }
    }

    pub fn small() -> Self {
        Self {
            size: 12.0,
            ..Self::new()
        }
    }

    pub fn large() -> Self {
        Self {
            size: 32.0,
            ..Self::new()
        }
    }

    pub fn phase(mut self, phase: u8) -> Self {
        self.phase = phase;
        self
    }

    /// Opacity of spoke `index`: the phase head is fully opaque and the trail
    /// fades back to 20%.
    pub fn spoke_opacity(phase: u8, index: u8, spokes: u8) -> f32 {
        if spokes == 0 {
            return 1.0;
        }
        let head = phase % spokes;
        let distance = (index + spokes - head) % spokes;
        1.0 - (f32::from(distance) / f32::from(spokes)) * 0.8
    }
}

impl Default for Spinner {
    fn default() -> Self {
        Self::new()
    }
}

impl Styled for Spinner {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Spinner {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let size = self.size;
        let center = size / 2.0;
        let radius = (center - 2.0).max(0.0);
        let dot = (size * 0.16).max(2.0);
        // Indeterminate: no numeric value, matching gpui-component's own
        // `progress/progress.rs` role for a spinner-style indicator.
        let mut container = div()
            .id("spinner")
            .role(Role::ProgressIndicator)
            .relative()
            .size(px(size))
            .refine_style(&self.style);
        for index in 0..SPINNER_SPOKES {
            let angle = f32::from(index) * std::f32::consts::TAU / f32::from(SPINNER_SPOKES);
            let x = center + radius * angle.sin() - dot / 2.0;
            let y = center - radius * angle.cos() - dot / 2.0;
            let opacity = Self::spoke_opacity(self.phase, index, SPINNER_SPOKES);
            container = container.child(
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .size(px(dot))
                    .rounded_full()
                    .bg(mac::accent().opacity(opacity)),
            );
        }
        container
    }
}

/// Visual outcome of a progress operation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProgressStatus {
    #[default]
    Running,
    Complete,
    Error,
}

/// Determinate or indeterminate progress with an optional visible label.
#[derive(IntoElement)]
pub struct Progress {
    value: Option<f32>,
    status: ProgressStatus,
    label: Option<SharedString>,
    style: StyleRefinement,
}

impl Progress {
    pub fn new(value: f32) -> Self {
        Self {
            value: Some(value.clamp(0.0, 1.0)),
            status: ProgressStatus::Running,
            label: None,
            style: StyleRefinement::default(),
        }
    }

    pub fn indeterminate() -> Self {
        Self {
            value: None,
            status: ProgressStatus::Running,
            label: None,
            style: StyleRefinement::default(),
        }
    }

    pub fn status(mut self, status: ProgressStatus) -> Self {
        self.status = status;
        if status == ProgressStatus::Complete {
            self.value = Some(1.0);
        }
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }
}

impl Styled for Progress {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Progress {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let width = self.value.unwrap_or(0.35);
        let determinate = self.value;
        let fill = if self.status == ProgressStatus::Error {
            mac::danger()
        } else {
            mac::accent()
        };
        div()
            .id("progress")
            .role(Role::ProgressIndicator)
            // Indeterminate progress leaves `aria_numeric_value` unset, matching
            // the ARIA convention of no `aria-valuenow` on a busy indicator.
            .when_some(determinate, |el, value| {
                el.aria_numeric_value(f64::from(value))
                    .aria_min_numeric_value(0.0)
                    .aria_max_numeric_value(1.0)
            })
            .v_flex()
            .gap_1()
            .refine_style(&self.style)
            .when_some(self.label, |progress, label| {
                progress.child(
                    div()
                        .text_size(crate::text_px(11.0))
                        .text_color(if self.status == ProgressStatus::Error {
                            mac::danger()
                        } else {
                            mac::text_secondary()
                        })
                        .child(label),
                )
            })
            .child(
                div()
                    .h(px(6.0))
                    .w_full()
                    .rounded_full()
                    .bg(mac::separator())
                    .child(div().h_full().w(relative(width)).rounded_full().bg(fill)),
            )
    }
}

/// Centered empty/unavailable/error content with an optional recovery action.
#[derive(IntoElement)]
pub struct EmptyState {
    title: SharedString,
    message: Option<SharedString>,
    action: Option<gpui::AnyElement>,
    error: bool,
    style: StyleRefinement,
}

impl EmptyState {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            message: None,
            action: None,
            error: false,
            style: StyleRefinement::default(),
        }
    }

    pub fn message(mut self, message: impl Into<SharedString>) -> Self {
        self.message = Some(message.into());
        self
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }

    pub fn error(mut self, error: bool) -> Self {
        self.error = error;
        self
    }
}

impl Styled for EmptyState {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for EmptyState {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        // The error variant appears asynchronously (a load that failed), so it
        // is announced the same way `Toast` announces itself: one alert node
        // with the title and message combined into its accessible name. A
        // plain empty state (no error) stays a silent, unlabeled container.
        let id: ElementId = SharedString::from(format!("empty-state-{}", self.title)).into();
        let accessible_name = match &self.message {
            Some(message) => SharedString::from(format!("{}. {}", self.title, message)),
            None => self.title.clone(),
        };
        let error = self.error;
        div()
            .id(id)
            .when(error, |el| el.role(Role::Alert).aria_label(accessible_name))
            .min_h(px(96.0))
            .w_full()
            .v_flex()
            .items_center()
            .justify_center()
            .gap_1()
            .px_4()
            .text_center()
            .refine_style(&self.style)
            .child(
                div()
                    .text_size(crate::text_px(13.0))
                    .font_weight(mac::SEMIBOLD)
                    .text_color(if self.error {
                        mac::danger()
                    } else {
                        mac::text()
                    })
                    .child(self.title),
            )
            .when_some(self.message, |empty, message| {
                empty.child(
                    div()
                        .max_w(px(360.0))
                        .text_size(crate::text_px(11.0))
                        .text_color(mac::text_secondary())
                        .child(message),
                )
            })
            .when_some(self.action, |empty, action| {
                empty.child(div().pt_2().child(action))
            })
    }
}

/// Semantic role of a transient toast.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToastKind {
    #[default]
    Informational,
    Success,
    Warning,
    Error,
}

type DismissHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// Compact transient message. Lifetime and stacking remain owned by the app.
#[derive(IntoElement)]
pub struct Toast {
    id: SharedString,
    kind: ToastKind,
    title: SharedString,
    message: Option<SharedString>,
    on_dismiss: Option<DismissHandler>,
    style: StyleRefinement,
}

impl Toast {
    pub fn new(
        id: impl Into<SharedString>,
        kind: ToastKind,
        title: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            title: title.into(),
            message: None,
            on_dismiss: None,
            style: StyleRefinement::default(),
        }
    }

    pub fn message(mut self, message: impl Into<SharedString>) -> Self {
        self.message = Some(message.into());
        self
    }

    pub fn on_dismiss(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_dismiss = Some(Rc::new(handler));
        self
    }
}

impl Styled for Toast {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Toast {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let (background, border, text, symbol) = match self.kind {
            ToastKind::Informational => {
                (mac::accent_subtle(), mac::accent_border(), mac::text(), "i")
            }
            ToastKind::Success => (mac::accent_subtle(), mac::accent_border(), mac::text(), "✓"),
            ToastKind::Warning => (
                mac::warning_background(),
                mac::warning_border(),
                mac::warning_text(),
                "!",
            ),
            ToastKind::Error => (
                mac::error_background(),
                mac::error_border(),
                mac::danger(),
                "×",
            ),
        };
        let dismiss_id: ElementId = SharedString::from(format!("{}-dismiss", self.id)).into();
        // A toast is announced as it appears rather than found by keyboard
        // navigation, so the whole message goes on one alert node
        // (AccessKit's live-region equivalent — matches the component library's own
        // `alert.rs`) rather than relying on the child text runs.
        let accessible_name = match &self.message {
            Some(message) => SharedString::from(format!("{}. {}", self.title, message)),
            None => self.title.clone(),
        };
        div()
            .id(self.id)
            .role(Role::Alert)
            .aria_label(accessible_name)
            .relative()
            .min_h(px(44.0))
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .rounded(px(mac::radius_control()))
            .bg(background)
            .border_1()
            .border_color(border)
            .text_color(text)
            .refine_style(&self.style)
            .child(
                div()
                    .size(px(18.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .border_1()
                    .border_color(border)
                    .text_size(crate::text_px(11.0))
                    .font_weight(mac::BOLD)
                    .child(symbol),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .v_flex()
                    .pr_20()
                    .child(
                        div()
                            .text_size(crate::text_px(12.0))
                            .font_weight(mac::SEMIBOLD)
                            .child(self.title),
                    )
                    .when_some(self.message, |body, message| {
                        body.child(div().text_size(crate::text_px(11.0)).child(message))
                    }),
            )
            .when_some(self.on_dismiss, |toast, handler| {
                toast.child(
                    div()
                        .id(dismiss_id)
                        .absolute()
                        .right_2()
                        .top(px(10.0))
                        .h(px(24.0))
                        .px_2()
                        .flex()
                        .items_center()
                        .rounded(px(mac::radius_control()))
                        .text_size(crate::text_px(11.0))
                        .font_weight(mac::SEMIBOLD)
                        .cursor_pointer()
                        .hover(|button| button.bg(mac::control_fill_hover()))
                        .child("Dismiss")
                        .on_click(move |event, window, cx| handler(event, window, cx)),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_values_are_clamped() {
        let clamp = |value: f32| value.clamp(0.0, 1.0);
        assert_eq!(clamp(-1.0), 0.0);
        assert_eq!(clamp(2.0), 1.0);
    }

    #[test]
    fn toast_roles_are_distinct() {
        assert_ne!(ToastKind::Informational, ToastKind::Error);
        assert_ne!(ToastKind::Success, ToastKind::Warning);
    }

    #[test]
    fn spinner_head_is_opaque_and_trail_fades() {
        assert_eq!(Spinner::spoke_opacity(0, 0, 12), 1.0);
        assert!(Spinner::spoke_opacity(0, 11, 12) < Spinner::spoke_opacity(0, 1, 12));
        assert!(Spinner::spoke_opacity(0, 6, 12) < 1.0);
        assert_eq!(Spinner::spoke_opacity(3, 0, 0), 1.0);
    }

    #[test]
    fn visible_errors_hide_internal_names_and_cause_chains() {
        assert_eq!(
            user_error_message(
                ErrorSurface::Settings,
                "NIRI_SOCKET is not set, are you running this within niri?",
                false,
            ),
            "This setting isn’t available right now—try again."
        );
        assert_eq!(
            user_error_message(
                ErrorSurface::Settings,
                "Could not update Displays: NIRI_SOCKET is not set\nCaused by: backend detail",
                false,
            ),
            "Could not update Displays."
        );
        assert_eq!(
            user_error_message(
                ErrorSurface::Files,
                "Trash recovery data could not be verified; Trash actions are disabled",
                true,
            ),
            "Review interrupted Trash operations before continuing."
        );
    }
}
