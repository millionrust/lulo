//! The minimise animation (DOCK-11): the window's picture pours into its
//! Dock tile with macOS's Genie effect, or shrinks into it with the Scale
//! effect (Desktop & Dock ▸ Minimise windows using, DOCK-09).
//!
//! niri moves a minimised window to the parking workspace at once, so the
//! window itself cannot be animated. The Dock already holds the picture the
//! minimise path took just before (`MinimizedItem::thumbnail`) and knows
//! exactly where the new tile sits, so it plays the effect on a short-lived,
//! input-less full-output layer surface and closes it again. Frames are
//! requested only while it plays; a low-spec PC (reduced motion) skips it.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::{
    div, img, layer_shell::*, point, prelude::*, px, App, Bounds, Context, DisplayId, ObjectFit,
    Size, Window, WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions,
};

/// macOS 26's Genie takes about half a second; Scale a little less.
pub const GENIE_DURATION: Duration = Duration::from_millis(520);
pub const SCALE_DURATION: Duration = Duration::from_millis(330);
/// Horizontal strips the picture is cut into for the Genie bend.
const STRIPS: usize = 28;

/// A rectangle in output-local logical points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Frame {
    fn lerp(self, to: Frame, t: f32) -> Frame {
        Frame {
            x: self.x + (to.x - self.x) * t,
            y: self.y + (to.y - self.y) * t,
            width: self.width + (to.width - self.width) * t,
            height: self.height + (to.height - self.height) * t,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Effect {
    Genie,
    Scale,
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// One strip of the Genie at `progress` (0–1): the picture's rows
/// `[v0, v1]` (0 is the top) drawn into `(x, y, width, height)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Strip {
    pub v0: f32,
    pub v1: f32,
    pub frame: Frame,
}

/// Where a picture row `v` sits and how wide it is at `progress`. The
/// bottom of the window starts first and every row slides down into the
/// tile; on the way a row narrows toward the tile the closer it gets, so
/// the picture bends into the funnel the Mac draws.
fn genie_row(from: Frame, to: Frame, v: f32, progress: f32) -> (f32, f32, f32) {
    // Phase one (the first ~40 %) bends the bottom toward the tile; the
    // rows then pour in, bottom rows first.
    let bend = smoothstep(0.0, 0.4, progress);
    let travel = smoothstep(0.15 + (1.0 - v) * 0.45, 1.0, progress);
    let source_y = from.y + v * from.height;
    let target_y = to.y + v * to.height;
    let y = source_y + (target_y - source_y) * travel;
    // How close this row is to the tile, from the window's top (0) to the
    // tile's top (1): the funnel narrows rows as they near it.
    let span = (to.y - from.y).max(1.0);
    let near = smoothstep(0.0, 1.0, ((y - from.y) / span).clamp(0.0, 1.0));
    let squeeze = (near * bend).max(travel);
    let width = from.width + (to.width - from.width) * squeeze;
    let centre_from = from.x + from.width / 2.0;
    let centre_to = to.x + to.width / 2.0;
    let centre = centre_from + (centre_to - centre_from) * squeeze;
    (centre - width / 2.0, y, width)
}

/// The Genie's strips at `progress`, top to bottom.
pub fn genie_strips(from: Frame, to: Frame, progress: f32) -> Vec<Strip> {
    (0..STRIPS)
        .map(|index| {
            let v0 = index as f32 / STRIPS as f32;
            let v1 = (index + 1) as f32 / STRIPS as f32;
            let (x0, y0, w0) = genie_row(from, to, v0, progress);
            let (x1, y1, w1) = genie_row(from, to, v1, progress);
            Strip {
                v0,
                v1,
                frame: Frame {
                    x: x0.min(x1),
                    y: y0,
                    width: (w0 + w1) / 2.0,
                    height: (y1 - y0).max(0.0),
                },
            }
        })
        .collect()
}

/// The Scale effect: the whole picture shrinks into the tile.
pub fn scale_frame(from: Frame, to: Frame, progress: f32) -> Frame {
    from.lerp(to, smoothstep(0.0, 1.0, progress))
}

/// The effect in flight on one output's overlay.
pub struct MinimizeAnimation {
    picture: PathBuf,
    from: Frame,
    to: Frame,
    effect: Effect,
    started: Instant,
}

impl MinimizeAnimation {
    fn duration(&self) -> Duration {
        match self.effect {
            Effect::Genie => GENIE_DURATION,
            Effect::Scale => SCALE_DURATION,
        }
    }
}

impl Render for MinimizeAnimation {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let progress =
            (self.started.elapsed().as_secs_f32() / self.duration().as_secs_f32()).min(1.0);
        if progress < 1.0 {
            window.request_animation_frame();
        }
        // The last stretch fades as the picture lands in the tile.
        let opacity = 1.0 - smoothstep(0.85, 1.0, progress);
        let root = div().size_full().relative().opacity(opacity);
        match self.effect {
            Effect::Scale => {
                let frame = scale_frame(self.from, self.to, progress);
                root.child(
                    img(self.picture.clone())
                        .absolute()
                        .left(px(frame.x))
                        .top(px(frame.y))
                        .w(px(frame.width))
                        .h(px(frame.height))
                        .object_fit(ObjectFit::Fill),
                )
            }
            Effect::Genie => root.children(
                genie_strips(self.from, self.to, progress)
                    .into_iter()
                    .map(|strip| {
                        let frame = strip.frame;
                        let full_height = frame.height / (strip.v1 - strip.v0).max(f32::EPSILON);
                        div()
                            .absolute()
                            .left(px(frame.x))
                            .top(px(frame.y))
                            .w(px(frame.width))
                            .h(px(frame.height))
                            .overflow_hidden()
                            .child(
                                img(self.picture.clone())
                                    .absolute()
                                    .left_0()
                                    .top(px(-strip.v0 * full_height))
                                    .w(px(frame.width))
                                    .h(px(full_height))
                                    .object_fit(ObjectFit::Fill),
                            )
                    }),
            ),
        }
    }
}

/// Play the effect on `display`, from the window's last frame to its tile,
/// then close the overlay.
pub fn play(
    display: Option<DisplayId>,
    picture: PathBuf,
    from: Frame,
    to: Frame,
    effect: Effect,
    cx: &mut App,
) {
    let options = WindowOptions {
        titlebar: None,
        focus: false,
        show: true,
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(0.0), px(0.0)),
            size: Size::new(px(1.0), px(1.0)),
        })),
        display_id: display,
        app_id: Some("dev.rmac.MinimizeEffect".to_owned()),
        window_background: WindowBackgroundAppearance::Transparent,
        kind: WindowKind::LayerShell(LayerShellOptions {
            namespace: "rmac-minimize-effect".to_owned(),
            layer: Layer::Overlay,
            anchor: Anchor::TOP | Anchor::RIGHT | Anchor::BOTTOM | Anchor::LEFT,
            // Over the menu bar and the Dock: the picture pours into a tile.
            exclusive_zone: Some(px(-1.0)),
            keyboard_interactivity: KeyboardInteractivity::None,
            ..Default::default()
        }),
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    let duration = match effect {
        Effect::Genie => GENIE_DURATION,
        Effect::Scale => SCALE_DURATION,
    };
    let opened = cx.open_window(options, move |window, cx| {
        window.set_input_region(Some(&[]));
        cx.new(|_| MinimizeAnimation {
            picture,
            from,
            to,
            effect,
            started: Instant::now(),
        })
    });
    match opened {
        Ok(handle) => {
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(duration + Duration::from_millis(40))
                    .await;
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            })
            .detach();
        }
        Err(error) => eprintln!("could not play the minimise effect: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Frame = Frame {
        x: 200.0,
        y: 100.0,
        width: 600.0,
        height: 400.0,
    };
    const TILE: Frame = Frame {
        x: 900.0,
        y: 740.0,
        width: 48.0,
        height: 32.0,
    };

    #[test]
    fn the_genie_starts_as_the_window_and_ends_in_the_tile() {
        let start = genie_strips(WINDOW, TILE, 0.0);
        assert_eq!(start.len(), STRIPS);
        assert!((start[0].frame.x - WINDOW.x).abs() < 0.01);
        assert!((start[0].frame.width - WINDOW.width).abs() < 0.01);
        assert!((start[0].frame.y - WINDOW.y).abs() < 0.01);
        let end = genie_strips(WINDOW, TILE, 1.0);
        for strip in &end {
            assert!((strip.frame.width - TILE.width).abs() < 0.5, "{strip:?}");
            assert!(strip.frame.y >= TILE.y - 0.5 && strip.frame.y <= TILE.y + TILE.height);
        }
    }

    #[test]
    fn midway_the_bottom_is_narrower_than_the_top() {
        let middle = genie_strips(WINDOW, TILE, 0.45);
        let top = middle.first().unwrap().frame.width;
        let bottom = middle.last().unwrap().frame.width;
        assert!(
            bottom < top,
            "bottom {bottom} should funnel below top {top}"
        );
        // Rows never cross over: each strip starts where the one above ends.
        for pair in middle.windows(2) {
            assert!((pair[0].frame.y + pair[0].frame.height - pair[1].frame.y).abs() < 0.01);
        }
    }

    #[test]
    fn scale_moves_straight_into_the_tile() {
        assert_eq!(scale_frame(WINDOW, TILE, 0.0), WINDOW);
        assert_eq!(scale_frame(WINDOW, TILE, 1.0), TILE);
    }
}
