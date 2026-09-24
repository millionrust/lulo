//! Control Center, laid out on the macOS 26 module grid.
//!
//! Every number here comes from `design-lab/control-center.html`, which
//! records where each was measured on the owner's Mac and which are rmac's
//! own start values.

mod cards;
mod controls;
mod detail;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, rgba, size, svg, AnyElement, Context, Div, FontWeight, Hsla, InteractiveElement as _,
    IntoElement, KeyDownEvent, MouseButton, MouseMoveEvent, MouseUpEvent, ParentElement as _,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
};
use rmac_quick_settings::layout;
use rmac_ui::mac;

use crate::view::{QuickSettingsView, SliderKind};

/// Inter's vertical metrics (hhea), used to put a baseline where measured.
const INTER_ASCENT: f32 = 0.968_75;
const INTER_DESCENT: f32 = 0.242_187_5;

const CELL: f32 = layout::CELL as f32;
const PITCH: f32 = (layout::CELL + layout::GAP) as f32;
const PILL_WIDTH: f32 = layout::PILL_WIDTH as f32;
const GRID_WIDTH: f32 = layout::GRID_WIDTH as f32;
const PADDING: f32 = layout::PADDING as f32;

fn color(value: u32) -> Hsla {
    rgba(value).into()
}

/// Everything behind the modules is darkened ≈ 38 % on the Mac.
fn backdrop() -> Hsla {
    color(0x0000_0061)
}
/// Module glass: white 15.8 % over the darkened backdrop.
fn module_fill() -> Hsla {
    color(0xffff_ff28)
}
/// 1 pt module rim, white ≈ 34 %.
fn module_rim() -> Hsla {
    color(0xffff_ff57)
}
/// Off icon circles and accessory circles: white 24.5 %.
fn circle_off() -> Hsla {
    color(0xffff_ff3e)
}
/// On icon circles are white with a system-blue glyph.
fn glyph_on() -> Hsla {
    color(0x007a_ffff)
}
fn title_text() -> Hsla {
    color(0xffff_ffe6)
}
fn subtitle_text() -> Hsla {
    color(0xffff_ffb3)
}
/// Unavailable transport glyphs, white ≈ 40 %.
fn dim_glyph() -> Hsla {
    color(0xffff_ff66)
}
/// Slider track, black 43 %.
fn slider_track() -> Hsla {
    color(0x0000_006e)
}
/// Now Playing artwork placeholder, white 13.5 %.
fn artwork_fill() -> Hsla {
    color(0xffff_ff22)
}

/// A text line placed so its baseline lands `baseline` below the parent's
/// top edge, as measured on the Mac.
fn text_at(
    x: f32,
    baseline: f32,
    text_size: f32,
    weight: FontWeight,
    color: Hsla,
    text: impl Into<SharedString>,
) -> Div {
    let line = (text_size * 1.25).ceil();
    let top = baseline
        - ((line - (INTER_ASCENT + INTER_DESCENT) * text_size) / 2.0 + INTER_ASCENT * text_size);
    div()
        .absolute()
        .left(px(x))
        .top(px(top))
        .h(px(line))
        .line_height(px(line))
        .text_size(rmac_ui::text_px(text_size))
        .font_weight(weight)
        .text_color(color)
        .whitespace_nowrap()
        .child(text.into())
}

/// A glyph of `width` × `height` centred on (`cx`, `cy`).
fn glyph_at(path: &'static str, cx: f32, cy: f32, width: f32, height: f32, color: Hsla) -> Div {
    div()
        .absolute()
        .left(px(cx - width / 2.0))
        .top(px(cy - height / 2.0))
        .w(px(width))
        .h(px(height))
        .child(svg().path(path).size_full().text_color(color))
}

/// A glass module at (`x`, `y`) in the content box. The rim is an overlay,
/// not a border, so children keep the module's outer coordinates.
fn module(x: f32, y: f32, width: f32, height: f32, radius: f32) -> Div {
    div()
        .absolute()
        .left(px(x))
        .top(px(y))
        .w(px(width))
        .h(px(height))
        .rounded(px(radius))
        .bg(module_fill())
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .rounded(px(radius))
                .border_1()
                .border_color(module_rim()),
        )
}

/// A full-size layer for absolutely placed children.
fn layer() -> Div {
    div().absolute().top_0().left_0().size_full()
}

impl Render for QuickSettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let modules = self.modules();
        let panel = self.panel();
        // A detail view replaces the grid; any banners stay above it.
        let banner_block = if modules.banners_shown() > 0 {
            PADDING + modules.banners_shown() as f32 * (layout::BANNER_HEIGHT + layout::GAP) as f32
        } else {
            0.0
        };
        let height = match &panel {
            Some(panel) => banner_block + panel.layout().1,
            None => modules.surface_height() as f32,
        };
        if (height - self.surface_height).abs() > 0.5 {
            self.surface_height = height;
            window.resize(size(px(layout::SURFACE_WIDTH as f32), px(height)));
        }

        let view = self.state.view();
        let mut children: Vec<AnyElement> = Vec::new();
        for (index, (control, message)) in self
            .banners()
            .into_iter()
            .take(modules.banners_shown())
            .enumerate()
        {
            let top = index as f32 * (layout::BANNER_HEIGHT + layout::GAP) as f32;
            children.push(self.banner(index, top, control, message, cx));
        }
        let detail = panel
            .as_ref()
            .map(|panel| self.detail_view(panel, banner_block, cx));

        let row_top = |row: usize| modules.row_top(row) as f32;
        let mut row = 0;
        if detail.is_none() {
            children.push(self.pill_wifi(0.0, row_top(row), &view.wifi, cx));
            if self.player.is_some() {
                children.push(self.now_playing(PITCH * 2.0, row_top(row), cx));
                children.push(self.pill_bluetooth(0.0, row_top(row + 1), &view.bluetooth, cx));
                row += 2;
            } else {
                children.push(self.pill_bluetooth(PITCH * 2.0, row_top(row), &view.bluetooth, cx));
                row += 1;
            }
            let mut column = 0.0;
            if layout::low_power_available(&view.power.value) && view.power.available {
                children.push(self.low_power_circle(column, row_top(row), &view.power, cx));
                column += PITCH;
            }
            children.push(self.screenshot_circle(column, row_top(row), cx));
            children.push(self.pill_focus(PITCH * 2.0, row_top(row), &view.focus, cx));
            row += 1;
            if let Some(level) = self.brightness {
                children.push(self.display_module(row_top(row), level, cx));
                row += 1;
            }
            children.push(self.sound_module(row_top(row), &view.sound, cx));
        }
        let content_height = if detail.is_some() {
            (banner_block - PADDING).max(0.0)
        } else {
            modules.content_height() as f32
        };

        div()
            .id("control-center")
            .size_full()
            .relative()
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.key_down(event, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                let Some(kind) = this.dragging else {
                    return;
                };
                if event.pressed_button == Some(MouseButton::Left) {
                    let value = slider_value(kind, f32::from(event.position.x));
                    this.slide(kind, value, cx);
                } else {
                    this.end_drag(cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.end_drag(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.end_drag(cx)),
            )
            .overflow_hidden()
            .rounded(px(layout::SURFACE_RADIUS as f32))
            .bg(backdrop())
            .text_color(mac::white())
            .font_family(rmac_ui::UI_FONT)
            .child(
                div()
                    .absolute()
                    .left(px(PADDING))
                    .top(px(PADDING))
                    .w(px(GRID_WIDTH))
                    .h(px(content_height))
                    .children(children),
            )
            .children(detail)
    }
}

/// Track geometry inside a slider module: (left, width), measured.
pub(super) fn track(kind: SliderKind) -> (f32, f32) {
    use rmac_quick_settings::detail::geometry;
    match kind {
        SliderKind::Brightness => (36.5, 183.0),
        SliderKind::Volume => (34.0, 179.5),
        SliderKind::DetailVolume => (geometry::SLIDER_LEFT, geometry::SLIDER_WIDTH),
    }
}

/// Percentage under a window-relative x on a slider's track. The modules
/// span the full grid width and the detail panel sits at a fixed inset, so
/// the track's window x is fixed.
fn slider_value(kind: SliderKind, window_x: f32) -> u8 {
    let (left, width) = track(kind);
    let origin = match kind {
        SliderKind::DetailVolume => rmac_quick_settings::detail::geometry::PANEL_LEFT,
        SliderKind::Brightness | SliderKind::Volume => PADDING,
    };
    let fraction = (window_x - origin - left) / width;
    (fraction.clamp(0.0, 1.0) * 100.0).round() as u8
}
