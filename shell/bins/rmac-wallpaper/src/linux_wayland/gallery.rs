//! The Edit Widgets gallery, over everything like the Mac's, measured on
//! macOS 26 (design-lab/desktop.html): a 1027 × 552 panel 41 from the left
//! edge, flush with the screen bottom; a 240 sidebar of widget sources;
//! the "Widgets on Desktop" card; 112-point tiles 11.5 apart; a 62 footer
//! with Done; and the Notification Centre column to drag widgets into.
//! Widgets are added by dragging a tile out of the panel or by clicking it.
//! The search field is left out: the shell has no text field yet.

use super::*;
use rmac_desktop::widgets::{self as desk_widgets, Widget};
use rmac_desktop_widgets::{face, GALLERY_SCALE};

const PANEL_LEFT: f32 = 41.0;
const PANEL_WIDTH: f32 = 1027.0;
const PANEL_HEIGHT: f32 = 552.0;
/// S: the panel's top corners.
const PANEL_RADIUS: f32 = 26.0;
const SIDEBAR: f32 = 240.0;
const FOOTER: f32 = 62.0;
/// Measured over the owner's blue wallpaper: sidebar #24327F; the selected
/// source pill is white 0.2 over it (#5163B3).
const PANEL_FILL: u32 = 0x1E2A70E6;
const PANEL_RIM: u32 = 0xFFFFFF2E;
const SELECTED_SOURCE: u32 = 0xFFFFFF33;
const SOURCE_TOP: f32 = 19.0;
const SOURCE_LEFT: f32 = 11.0;
const SOURCE_WIDTH: f32 = 210.0;
const SOURCE_ROW: f32 = 40.0;
const SOURCE_ICON: f32 = 22.0;
/// Header card: x 270–1016 and y 20–192 inside the panel.
const CONTENT_LEFT: f32 = 270.0;
const HEADER_TOP: f32 = 20.0;
const HEADER_HEIGHT: f32 = 172.0;
const HEADER_FILL: u32 = 0xFFFFFF14;
const SUGGESTIONS_TOP: f32 = 211.0;
const SUGGESTIONS_COLOUR: u32 = 0x8FA6FFFF;
const TILES_TOP: f32 = 236.0;
const TILE_PITCH: f32 = 112.0 + 11.5;
const DONE_WIDTH: f32 = 83.0;
const DONE_HEIGHT: f32 = 28.0;
/// The Notification Centre column during editing: centred 284 from the
/// right edge; its Done pill is 43 × 22.
const COLUMN_CENTRE: f32 = 284.0;
const COLUMN_WIDTH: f32 = 344.0;
const COLUMN_TOP: f32 = 45.0;
const COLUMN_GAP: f32 = 4.0;
/// S: the remove badge on widgets while editing.
const BADGE: f32 = 20.0;
const BADGE_FILL: u32 = 0x3A3A3CFF;
const DRAG_THRESHOLD: f32 = 3.0;

struct GalleryDrag {
    kind: WidgetKind,
    start: Point<Pixels>,
    current: Point<Pixels>,
    moved: bool,
}

pub(crate) struct Gallery {
    status: Entity<WallpaperStatus>,
    target: GalleryTarget,
    /// `None` shows every source ("All Widgets").
    source: Option<WidgetKind>,
    drag: Option<GalleryDrag>,
    focus: FocusHandle,
}

/// Opens the gallery, or leaves the open one in place.
pub(crate) fn open(
    status: Entity<WallpaperStatus>,
    target: GalleryTarget,
    display: Option<DisplayId>,
    cx: &mut App,
) {
    if status.read(cx).gallery.is_some() {
        return;
    }
    let display = display
        .and_then(|id| cx.displays().into_iter().find(|display| display.id() == id))
        .or_else(|| cx.primary_display());
    let Some(display) = display else {
        eprintln!("the widget gallery has no display");
        return;
    };
    let size = display.bounds().size;
    let options = WindowOptions {
        titlebar: None,
        focus: true,
        show: true,
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(0.0), px(0.0)),
            size,
        })),
        display_id: Some(display.id()),
        app_id: Some("dev.rmac.Wallpaper".to_owned()),
        window_background: WindowBackgroundAppearance::Transparent,
        kind: WindowKind::LayerShell(LayerShellOptions {
            namespace: "rmac-widget-gallery".to_owned(),
            layer: Layer::Top,
            anchor: Anchor::TOP | Anchor::RIGHT | Anchor::BOTTOM | Anchor::LEFT,
            exclusive_zone: Some(px(-1.0)),
            keyboard_interactivity: KeyboardInteractivity::Exclusive,
            ..Default::default()
        }),
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    let view_status = status.clone();
    match cx.open_window(options, move |window, cx| {
        cx.new(|cx| Gallery::new(view_status, target, window, cx))
    }) {
        Ok(handle) => status.update(cx, |status, cx| {
            status.gallery = Some(handle.into());
            cx.notify();
        }),
        Err(error) => eprintln!("the widget gallery could not be opened: {error}"),
    }
}

impl Gallery {
    fn new(
        status: Entity<WallpaperStatus>,
        target: GalleryTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&status, |_, _, cx| cx.notify()).detach();
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            status,
            target,
            source: None,
            drag: None,
            focus,
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.status.update(cx, |status, cx| {
            status.gallery = None;
            cx.notify();
        });
        window.remove_window();
    }

    fn add(&mut self, kind: WidgetKind, location: WidgetLocation, cx: &mut Context<Self>) {
        self.status.update(cx, |status, cx| {
            status.add_widget(kind, location, cx);
        });
    }

    fn remove(&mut self, id: u64, cx: &mut Context<Self>) {
        self.status.update(cx, |status, cx| {
            status.update_settings(cx, |settings| {
                settings.remove_widget(id);
            });
        });
    }

    fn panel_frame(screen: (f32, f32)) -> (f32, f32, f32, f32) {
        let width = PANEL_WIDTH
            .min(screen.0 - 2.0 * PANEL_LEFT)
            .max(SIDEBAR + 200.0);
        let height = PANEL_HEIGHT.min(screen.1 - 40.0);
        (PANEL_LEFT, screen.1 - height, width, height)
    }

    fn column_frame(screen: (f32, f32), panel_top: f32) -> (f32, f32, f32, f32) {
        let left = screen.0 - COLUMN_CENTRE - COLUMN_WIDTH / 2.0;
        (left, 0.0, COLUMN_WIDTH, panel_top.max(COLUMN_TOP))
    }

    fn release(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        let size = window.viewport_size();
        let screen = (f32::from(size.width), f32::from(size.height));
        let (x, y) = (f32::from(position.x), f32::from(position.y));
        let inside = |(left, top, width, height): (f32, f32, f32, f32)| {
            x >= left && x < left + width && y >= top && y < top + height
        };
        let panel = Self::panel_frame(screen);
        let column = Self::column_frame(screen, panel.1);
        let existing = self.status.read(cx).settings.widgets.clone();
        let location = if !drag.moved {
            match self.target {
                GalleryTarget::Desktop => {
                    let (left, top) =
                        desk_widgets::next_desktop_origin(&existing, WidgetSize::Small, screen);
                    Some(WidgetLocation::Desktop { left, top })
                }
                GalleryTarget::NotificationCenter => Some(WidgetLocation::NotificationCenter),
            }
        } else if inside(column) {
            Some(WidgetLocation::NotificationCenter)
        } else if !inside(panel) {
            let half = desk_widgets::SMALL / 2.0;
            let (left, top) =
                desk_widgets::clamp_origin(WidgetSize::Small, x - half, y - half, screen);
            Some(WidgetLocation::Desktop { left, top })
        } else {
            None
        };
        if let Some(location) = location {
            self.add(drag.kind, location, cx);
        }
        cx.notify();
    }

    fn badge(&self, id: u64, left: f32, top: f32, cx: &Context<Self>) -> AnyElement {
        div()
            .id(("gallery-remove", id as usize))
            .role(Role::Button)
            .aria_label("Remove Widget")
            .absolute()
            .left(px(left - BADGE / 3.0))
            .top(px(top - BADGE / 3.0))
            .size(px(BADGE))
            .rounded_full()
            .bg(rgba(BADGE_FILL))
            .border_1()
            .border_color(rgba(PANEL_RIM))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(9.0))
                    .h(px(2.0))
                    .rounded(px(1.0))
                    .bg(rgba(0xFFFFFFFF)),
            )
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.remove(id, cx);
            }))
            .into_any_element()
    }

    fn source_icon(kind: Option<WidgetKind>) -> AnyElement {
        let tile = div()
            .relative()
            .flex_none()
            .size(px(SOURCE_ICON))
            .rounded(px(6.0))
            .overflow_hidden();
        match kind {
            None => tile
                .flex()
                .items_center()
                .justify_center()
                .gap(px(2.0))
                .child(
                    div()
                        .w(px(9.0))
                        .h(px(9.0))
                        .rounded(px(2.0))
                        .border_1()
                        .border_color(rgba(0xFFFFFFFF)),
                )
                .child(
                    div()
                        .w(px(6.0))
                        .h(px(9.0))
                        .rounded(px(2.0))
                        .border_1()
                        .border_color(rgba(0xFFFFFFFF)),
                )
                .into_any_element(),
            Some(WidgetKind::Battery) => tile
                .bg(rgba(0x70D871FF))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .w(px(13.0))
                        .h(px(7.0))
                        .rounded(px(2.0))
                        .bg(rgba(0xFFFFFFFF)),
                )
                .into_any_element(),
            Some(WidgetKind::Calendar) => tile
                .bg(rgba(0xFFFFFFFF))
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .top_0()
                        .h(px(6.0))
                        .bg(rgba(0xFF453AFF)),
                )
                .into_any_element(),
            Some(WidgetKind::Clock) => tile
                .bg(rgba(0x1C1C1EFF))
                .flex()
                .items_center()
                .justify_center()
                .child(div().size(px(16.0)).rounded_full().bg(rgba(0xFFFFFFFF)))
                .into_any_element(),
            Some(WidgetKind::Weather) => tile
                .bg(gpui::linear_gradient(
                    180.0,
                    linear_color_stop(rgba(0x3D83D3FF), 0.0),
                    linear_color_stop(rgba(0x74ABE6FF), 1.0),
                ))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    svg()
                        .size(px(14.0))
                        .path("widgets/weather/sun.svg")
                        .text_color(rgba(0xFFD60AFF)),
                )
                .into_any_element(),
        }
    }
}

impl Render for Gallery {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let size = window.viewport_size();
        let screen = (f32::from(size.width), f32::from(size.height));
        let (panel_left, panel_top, panel_width, panel_height) = Self::panel_frame(screen);
        let (column_left, _, _, _) = Self::column_frame(screen, panel_top);
        let (widgets, data): (Vec<Widget>, WidgetData) = {
            let status = self.status.read(cx);
            (status.settings.widgets.clone(), status.widgets.clone())
        };
        let mut root = div()
            .id("widget-gallery")
            .size_full()
            .relative()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "escape" | "enter") {
                    cx.stop_propagation();
                    this.close(window, cx);
                }
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if let Some(drag) = &mut this.drag {
                    drag.current = event.position;
                    let dx = f32::from(drag.current.x - drag.start.x);
                    let dy = f32::from(drag.current.y - drag.start.y);
                    if dx.hypot(dy) > DRAG_THRESHOLD {
                        drag.moved = true;
                    }
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    this.release(event.position, window, cx);
                }),
            );

        // Remove badges on the desktop's widgets.
        for widget in &widgets {
            if let WidgetLocation::Desktop { left, top } = widget.location {
                root = root.child(self.badge(widget.id, left, top, cx));
            }
        }

        // The Notification Centre column.
        let centre_widgets = widgets
            .iter()
            .filter(|widget| widget.location == WidgetLocation::NotificationCenter)
            .copied()
            .collect::<Vec<_>>();
        let pitch = desk_widgets::SMALL + COLUMN_GAP;
        let rows = centre_widgets.len().div_ceil(2) as f32;
        for (index, widget) in centre_widgets.iter().enumerate() {
            let left = column_left + (index % 2) as f32 * pitch;
            let top = COLUMN_TOP + (index / 2) as f32 * pitch;
            root = root
                .child(div().absolute().left(px(left)).top(px(top)).child(face(
                    widget.kind,
                    widget.size,
                    1.0,
                    &data,
                )))
                .child(self.badge(widget.id, left, top, cx));
        }
        let hint_top = COLUMN_TOP + rows * pitch;
        root = root.child(
            div()
                .absolute()
                .left(px(column_left))
                .top(px(hint_top))
                .w(px(COLUMN_WIDTH))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(12.0))
                .text_color(rgba(0xFFFFFFE6))
                .when(centre_widgets.is_empty(), |column| {
                    column.child(
                        div()
                            .mt(px(135.0))
                            .text_size(px(15.0))
                            .line_height(px(19.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_center()
                            .max_w(px(330.0))
                            .child("Drag widgets here to add them to Notification Centre"),
                    )
                })
                .child(
                    div()
                        .id("gallery-centre-done")
                        .role(Role::Button)
                        .aria_label("Done")
                        .mt(px(if centre_widgets.is_empty() {
                            150.0
                        } else {
                            12.0
                        }))
                        .w(px(43.0))
                        .h(px(22.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(rgba(0xFFFFFF26))
                        .border_1()
                        .border_color(rgba(0xFFFFFF45))
                        .text_size(px(12.0))
                        .text_color(rgba(0xFFFFFFBF))
                        .child("Done")
                        .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
                ),
        );

        // The gallery panel.
        let sources = std::iter::once(None)
            .chain(
                WidgetKind::ALL
                    .into_iter()
                    .filter(|kind| data.offers(*kind))
                    .map(Some),
            )
            .enumerate()
            .map(|(index, kind)| {
                let selected = self.source == kind;
                div()
                    .id(("gallery-source", index))
                    .role(Role::Button)
                    .aria_label(kind.map_or("All Widgets", WidgetKind::source))
                    .absolute()
                    .left(px(SOURCE_LEFT))
                    .top(px(SOURCE_TOP + index as f32 * SOURCE_ROW))
                    .w(px(SOURCE_WIDTH))
                    .h(px(SOURCE_ROW))
                    .pl(px(7.0))
                    .flex()
                    .items_center()
                    .gap(px(7.0))
                    .rounded(px(10.0))
                    .when(selected, |row| row.bg(rgba(SELECTED_SOURCE)))
                    .child(Self::source_icon(kind))
                    .child(kind.map_or("All Widgets", WidgetKind::source))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.source = kind;
                        cx.notify();
                    }))
            })
            .collect::<Vec<_>>();
        let tiles = WidgetKind::ALL
            .into_iter()
            .filter(|kind| data.offers(*kind))
            .filter(|kind| self.source.is_none_or(|source| source == *kind))
            .enumerate()
            .map(|(index, kind)| {
                div()
                    .id(("gallery-tile", index))
                    .role(Role::Button)
                    .aria_label(kind.source())
                    .absolute()
                    .left(px(CONTENT_LEFT + index as f32 * TILE_PITCH))
                    .top(px(TILES_TOP))
                    .child(face(kind, WidgetSize::Small, GALLERY_SCALE, &data))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.drag = Some(GalleryDrag {
                                kind,
                                start: event.position,
                                current: event.position,
                                moved: false,
                            });
                            cx.notify();
                        }),
                    )
            })
            .collect::<Vec<_>>();
        root = root.child(
            div()
                .id("gallery-panel")
                .absolute()
                .left(px(panel_left))
                .top(px(panel_top))
                .w(px(panel_width))
                .h(px(panel_height))
                .rounded_t(px(PANEL_RADIUS))
                .overflow_hidden()
                .bg(rgba(PANEL_FILL))
                .border_1()
                .border_color(rgba(PANEL_RIM))
                .shadow_lg()
                .occlude()
                .text_size(px(13.0))
                .text_color(rgba(0xFFFFFFFF))
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .w(px(SIDEBAR))
                        .bottom(px(FOOTER))
                        .border_r_1()
                        .border_color(rgba(tokens::separator()))
                        .children(sources),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(CONTENT_LEFT))
                        .top(px(HEADER_TOP))
                        .w(px((panel_width - CONTENT_LEFT - 11.0).max(0.0)))
                        .h(px(HEADER_HEIGHT))
                        .px(px(44.0))
                        .flex()
                        .flex_col()
                        .justify_center()
                        .rounded(px(16.0))
                        .bg(rgba(HEADER_FILL))
                        .child(
                            div()
                                .mb(px(6.0))
                                .text_size(px(15.0))
                                .line_height(px(18.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Widgets on Desktop"),
                        )
                        .child(
                            div()
                                .max_w(px(500.0))
                                .line_height(px(15.0))
                                .text_color(rgba(0xFFFFFFE0))
                                .child(
                                    "Place widgets directly on the desktop by dragging them \
                                     from the widget gallery.",
                                ),
                        ),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(CONTENT_LEFT))
                        .top(px(SUGGESTIONS_TOP))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgba(SUGGESTIONS_COLOUR))
                        .child("Suggestions"),
                )
                .children(tiles)
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .h(px(FOOTER))
                        .px(px(20.0))
                        .flex()
                        .items_center()
                        .border_t_1()
                        .border_color(rgba(tokens::separator()))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(
                            div()
                                .flex_1()
                                .child("Drag a widget to place it on the desktop…"),
                        )
                        .child(
                            div()
                                .id("gallery-done")
                                .role(Role::Button)
                                .aria_label("Done")
                                .w(px(DONE_WIDTH))
                                .h(px(DONE_HEIGHT))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .bg(rgba(tokens::accent()))
                                .font_weight(FontWeight::MEDIUM)
                                .child("Done")
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.close(window, cx)),
                                ),
                        ),
                ),
        );

        // The widget being dragged, at full size under the pointer.
        if let Some(drag) = self.drag.as_ref().filter(|drag| drag.moved) {
            let half = desk_widgets::SMALL / 2.0;
            root = root.child(
                div()
                    .absolute()
                    .left(px(f32::from(drag.current.x) - half))
                    .top(px(f32::from(drag.current.y) - half))
                    .opacity(0.9)
                    .child(face(drag.kind, WidgetSize::Small, 1.0, &data)),
            );
        }
        root
    }
}
