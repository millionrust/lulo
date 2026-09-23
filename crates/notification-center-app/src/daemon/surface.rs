//! One banner layer surface per output, drawn at the measured macOS 26 sizes
//! (design-lab/notification-banners.html).
//!
//! The surface spans from the menu bar to just below the banners and from
//! the screen edge to a little left of the card, so a card can slide in from
//! past the edge. Only the cards take pointer input; everything else passes
//! through to the windows underneath.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    canvas, div, img, linear_color_stop, linear_gradient, px, rgba, size, AnyElement,
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, Pixels, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_component::{IconName, StyledExt as _};
use rmac_notifications::NotificationId;
use rmac_notifications_runtime::presentation::ControlId;
use rmac_ui::mac;

use crate::host::{metrics, BannerHost, CardView};

/// Card geometry and colours measured from a macOS 26.2 banner (dark mode).
mod card {
    pub const RADIUS: f32 = 20.0;
    pub const PAD_TOP: f32 = 12.0;
    pub const PAD_BOTTOM: f32 = 13.0;
    pub const PAD_LEFT: f32 = 13.0;
    pub const ICON: f32 = 32.0;
    pub const ICON_GAP: f32 = 13.0;
    /// The body wraps at 270 pt; the title stops 15 pt earlier (255 pt).
    pub const BODY_RIGHT: f32 = 16.0;
    pub const TITLE_EXTRA_RIGHT: f32 = 15.0;
    pub const TEXT_SIZE: f32 = 13.0;
    pub const LINE: f32 = 16.0;
    pub const BODY_LINES: usize = 4;
    /// Hover ×: a 20 pt disc 6 left of and 4 above the card's corner.
    pub const CLOSE: f32 = 20.0;
    pub const CLOSE_LEFT: f32 = 6.0;
    pub const CLOSE_TOP: f32 = 4.0;
    pub const CLOSE_GLYPH: f32 = 8.0;
    /// Hover action button ("Show"): 22 tall, at least 64 wide, 10 from the
    /// card's right and bottom edges, over a fade that hides the text.
    pub const BUTTON_HEIGHT: f32 = 22.0;
    pub const BUTTON_MIN_WIDTH: f32 = 64.0;
    pub const BUTTON_PAD: f32 = 16.0;
    pub const BUTTON_INSET: f32 = 10.0;
    pub const BUTTON_FADE: f32 = 24.0;

    /// A shared layer surface cannot blur per card, so the glass is drawn in
    /// the Center's nearly opaque tone of the same colour.
    pub const FILL: u32 = 0x242428F0;
    /// 0.5 pt rim, white ≈ 0.15.
    pub const BORDER: u32 = 0xFFFFFF26;
    /// Title and body are the same white ≈ 0.87 (222/255).
    pub const TEXT: u32 = 0xFFFFFFDE;
    pub const CLOSE_FILL: u32 = 0x353539F2;
    pub const CLOSE_BORDER: u32 = 0xFFFFFF1F;
    pub const CLOSE_GLYPH_COLOUR: u32 = 0xFFFFFFA8;
    pub const BUTTON_FILL: u32 = 0xFFFFFF11;
    pub const BUTTON_TEXT: u32 = 0xFFFFFFF5;
}

/// Room left of the card for the × overhang and the shadow, and below the
/// last card for its shadow.
const LEFT_ROOM: f32 = 18.0;
const BOTTOM_ROOM: f32 = 24.0;
const SURFACE_WIDTH: f32 = LEFT_ROOM + card::CLOSE_LEFT + metrics::CARD_WIDTH + metrics::RIGHT;
/// Before the first layout the surface is one small banner tall.
const INITIAL_HEIGHT: f32 = 120.0;
const OPTIONS_LABEL: &str = "Options";

pub(crate) struct BannerSurface {
    output: String,
    host: Entity<BannerHost>,
}

impl BannerSurface {
    fn new(output: String, host: Entity<BannerHost>, cx: &mut Context<Self>) -> Self {
        cx.observe(&host, |_, _, cx| cx.notify()).detach();
        Self { output, host }
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn open(
    output: String,
    host: Entity<BannerHost>,
    cx: &mut App,
) -> Option<AnyWindowHandle> {
    use gpui::layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions};
    use gpui::{
        point, PlatformDisplay as _, WindowBackgroundAppearance, WindowBounds, WindowKind,
        WindowOptions,
    };

    // GPUI names each Wayland output by the same v5 UUID of its name that
    // the compositor snapshot uses.
    let wanted = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_DNS, output.as_bytes());
    let display_id = cx
        .displays()
        .into_iter()
        .filter(|display| display.uuid().ok() == Some(wanted))
        .max_by_key(|display| u64::from(display.id()))
        .map(|display| display.id());
    let options = WindowOptions {
        titlebar: None,
        focus: false,
        show: true,
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(0.0), px(0.0)),
            size(px(SURFACE_WIDTH), px(INITIAL_HEIGHT)),
        ))),
        display_id,
        app_id: Some("org.rmac.NotificationBanners".into()),
        window_background: WindowBackgroundAppearance::Transparent,
        kind: WindowKind::LayerShell(LayerShellOptions {
            namespace: rmac_notifications_linux::surfaces::NAMESPACE.into(),
            // Overlay is the only layer niri keeps above full-screen windows.
            layer: Layer::Overlay,
            anchor: Anchor::TOP | Anchor::RIGHT,
            // A zero exclusive zone keeps the surface below the menu bar.
            margin: Some((px(0.0), px(0.0), px(0.0), px(0.0))),
            // Banners never take keyboard focus from the frontmost app.
            keyboard_interactivity: KeyboardInteractivity::None,
            ..Default::default()
        }),
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    let opened = cx.open_window(options, |window, cx| {
        window.set_window_title("Notification Banners");
        rmac_ui::prepare_surface_window(window, cx);
        let view = cx.new(|cx| BannerSurface::new(output, host, cx));
        cx.new(|cx| rmac_ui::shell_surface_root(view, window, cx))
    });
    match opened {
        Ok(handle) => Some(handle.into()),
        Err(error) => {
            eprintln!("notification banners: could not open a surface: {error}");
            None
        }
    }
}

/// Banners are drawn as Wayland layer surfaces; elsewhere the service runs
/// without them.
#[cfg(not(target_os = "linux"))]
pub(crate) fn open(
    _output: String,
    _host: Entity<BannerHost>,
    _cx: &mut App,
) -> Option<AnyWindowHandle> {
    None
}

type Regions = Rc<RefCell<Vec<Bounds<Pixels>>>>;

/// Records an element's bounds as part of the surface's input region.
fn input_region(regions: &Regions) -> impl IntoElement {
    let regions = regions.clone();
    canvas(
        move |bounds, _, _| regions.borrow_mut().push(bounds),
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}

fn app_icon(view: &CardView) -> AnyElement {
    if let Some(icon) = &view.icon {
        return img(icon.clone())
            .size(px(card::ICON))
            .flex_none()
            .into_any_element();
    }
    // An unresolved sender gets a plain plate, like a generic app icon.
    div()
        .size(px(card::ICON))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(card::ICON * 0.225))
        .bg(mac::control_fill())
        .text_color(mac::text_secondary())
        .font_weight(mac::SEMIBOLD)
        .text_size(rmac_ui::text_px(card::ICON * 0.45))
        .child(view.initial.clone())
        .into_any_element()
}

fn close_button(id: NotificationId, host: &Entity<BannerHost>) -> AnyElement {
    let host = host.clone();
    div()
        .id(SharedString::from(format!("banner-{}-dismiss", id.get())))
        .absolute()
        .left_0()
        .top_0()
        .size(px(card::CLOSE))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(rgba(card::CLOSE_FILL))
        .border(px(0.5))
        .border_color(rgba(card::CLOSE_BORDER))
        .role(Role::Button)
        .aria_label("Dismiss notification")
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            host.update(cx, |host, cx| host.activate(ControlId::Dismiss(id), cx));
        })
        .child(
            gpui_component::Icon::new(IconName::Close)
                .size(px(card::CLOSE_GLYPH))
                .text_color(rgba(card::CLOSE_GLYPH_COLOUR)),
        )
        .into_any_element()
}

fn pill(
    id: SharedString,
    label: SharedString,
    busy: bool,
    on_click: impl Fn(&mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .h(px(card::BUTTON_HEIGHT))
        .min_w(px(card::BUTTON_MIN_WIDTH))
        .px(px(card::BUTTON_PAD))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(rgba(card::FILL))
        .child(
            div()
                .absolute()
                .inset_0()
                .rounded_full()
                .bg(rgba(card::BUTTON_FILL)),
        )
        .relative()
        .when(busy, |pill| pill.opacity(0.5))
        .text_size(rmac_ui::text_px(card::TEXT_SIZE))
        .line_height(px(card::LINE))
        .font_weight(mac::MEDIUM)
        .text_color(rgba(card::BUTTON_TEXT))
        .whitespace_nowrap()
        .role(Role::Button)
        .aria_label(label.clone())
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            on_click(cx);
        })
        .child(div().relative().child(label))
        .into_any_element()
}

/// The hover button in the card's bottom-right corner: the one action, or
/// "Options" when there are several, over a fade that hides the text end.
fn action_button(view: &CardView, host: &Entity<BannerHost>) -> Option<AnyElement> {
    let id = view.id;
    let button = match view.actions.as_slice() {
        [] => return None,
        [(control, label)] => {
            let control = *control;
            let host = host.clone();
            pill(
                SharedString::from(format!("banner-{}-action", id.get())),
                label.clone(),
                view.busy == Some(control),
                move |cx| host.update(cx, |host, cx| host.activate(control, cx)),
            )
        }
        _ => {
            let host = host.clone();
            pill(
                SharedString::from(format!("banner-{}-options", id.get())),
                OPTIONS_LABEL.into(),
                view.busy.is_some(),
                move |cx| host.update(cx, |host, cx| host.toggle_options(id, cx)),
            )
        }
    };
    Some(
        div()
            .absolute()
            .right(px(card::BUTTON_INSET))
            .bottom(px(card::BUTTON_INSET))
            .flex()
            .items_center()
            .child(
                div()
                    .w(px(card::BUTTON_FADE))
                    .h(px(card::BUTTON_HEIGHT))
                    .bg(linear_gradient(
                        90.0,
                        linear_color_stop(rgba(card::FILL & 0xFFFF_FF00), 0.0),
                        linear_color_stop(rgba(card::FILL), 1.0),
                    )),
            )
            .child(button)
            .into_any_element(),
    )
}

/// With several actions, "Options" lists them all under the text.
fn options_list(view: &CardView, host: &Entity<BannerHost>) -> AnyElement {
    let id = view.id;
    let rows = view
        .actions
        .iter()
        .enumerate()
        .map(|(index, (control, label))| {
            let control = *control;
            let host = host.clone();
            pill(
                SharedString::from(format!("banner-{}-option-{index}", id.get())),
                label.clone(),
                view.busy == Some(control),
                move |cx| host.update(cx, |host, cx| host.activate(control, cx)),
            )
        })
        .collect::<Vec<_>>();
    div()
        .pt(px(8.0))
        .flex()
        .flex_wrap()
        .gap(px(6.0))
        .children(rows)
        .into_any_element()
}

fn banner(view: CardView, host: &Entity<BannerHost>, regions: &Regions) -> AnyElement {
    let id = view.id;
    let show_controls = view.hovered || view.persistent;
    let action = show_controls.then(|| action_button(&view, host)).flatten();
    let options = (show_controls && view.options_open && view.actions.len() > 1)
        .then(|| options_list(&view, host));
    let hover_host = host.clone();
    let click_host = host.clone();
    let role = if view.assertive {
        Role::Alert
    } else {
        Role::Status
    };

    let card_element = div()
        .id(SharedString::from(format!("banner-{}", id.get())))
        .relative()
        .w(px(metrics::CARD_WIDTH))
        .flex()
        .items_center()
        .pt(px(card::PAD_TOP))
        .pb(px(card::PAD_BOTTOM))
        .pl(px(card::PAD_LEFT))
        .rounded(px(card::RADIUS))
        .bg(rgba(card::FILL))
        .border(px(0.5))
        .border_color(rgba(card::BORDER))
        .shadow_lg()
        .role(role)
        .aria_label(view.accessible_label.clone())
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, _, cx| {
            click_host.update(cx, |host, cx| host.click(id, cx));
        })
        .child(app_icon(&view))
        .child(
            div()
                .ml(px(card::ICON_GAP))
                .pr(px(card::BODY_RIGHT))
                .flex_1()
                .min_w_0()
                .v_flex()
                .text_size(rmac_ui::text_px(card::TEXT_SIZE))
                .line_height(px(card::LINE))
                .text_color(rgba(card::TEXT))
                .child(
                    div()
                        .mr(px(card::TITLE_EXTRA_RIGHT))
                        .truncate()
                        .font_weight(mac::SEMIBOLD)
                        .child(view.title.clone()),
                )
                .when(!view.body.is_empty(), |text| {
                    text.child(
                        div()
                            .whitespace_normal()
                            .line_clamp(card::BODY_LINES)
                            .child(view.body.clone()),
                    )
                })
                .when_some(options, |text, options| text.child(options)),
        )
        .when_some(action, |card_element, action| card_element.child(action));

    // The wrapper reaches over the × so moving onto it keeps the hover.
    div()
        .id(SharedString::from(format!("banner-{}-hover", id.get())))
        .relative()
        .left(px(view.offset))
        .pl(px(card::CLOSE_LEFT))
        .pt(px(card::CLOSE_TOP))
        .on_hover(move |hovered: &bool, _, cx| {
            let hovered = *hovered;
            hover_host.update(cx, |host, cx| host.set_hovered(id, hovered, cx));
        })
        .child(input_region(regions))
        .child(card_element)
        .when(view.hovered && view.dismissible, |wrapper| {
            wrapper.child(close_button(id, host))
        })
        .into_any_element()
}

impl Render for BannerSurface {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (views, animating) = {
            let host = self.host.read(cx);
            (host.cards(&self.output), host.animating())
        };
        if animating {
            window.request_animation_frame();
        }
        let regions: Regions = Rc::new(RefCell::new(Vec::new()));
        let banners = views
            .into_iter()
            .map(|view| banner(view, &self.host, &regions))
            .collect::<Vec<_>>();

        // Sizes the surface to its banners after layout.
        let measure = canvas(
            |bounds, window, cx| {
                let wanted =
                    (f32::from(bounds.size.height) + BOTTOM_ROOM).max(INITIAL_HEIGHT / 2.0);
                let current = f32::from(window.viewport_size().height);
                if (wanted - current).abs() > 0.5 {
                    window.defer(cx, move |window, _| {
                        window.resize(size(px(SURFACE_WIDTH), px(wanted)));
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0();

        // Painted last, after every card has recorded its bounds.
        let apply_region = {
            let regions = regions.clone();
            canvas(
                |_, _, _| {},
                move |_, _, window, _| {
                    window.set_input_region(Some(regions.borrow().as_slice()));
                },
            )
            .absolute()
            .inset_0()
        };

        div()
            .size_full()
            .relative()
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right(px(metrics::RIGHT))
                    .w(px(card::CLOSE_LEFT + metrics::CARD_WIDTH))
                    .pt(px(metrics::TOP - card::CLOSE_TOP))
                    .v_flex()
                    .gap(px(metrics::GAP - card::CLOSE_TOP))
                    .children(banners)
                    .child(measure),
            )
            .child(apply_region)
    }
}
