//! rmac (ADR 0026): cached views through a live resize.
//!
//! `RMAC_GPUI_RESIZE_REFRESH=1` brings back upstream's full refresh on every
//! bounds change, so the same build can be measured both ways
//! (`scripts/behavior/run_speed_sweep.py`). `RMAC_GPUI_CACHE_TRACE=1` prints
//! why each cached view was not reused. Both are read once per process.

use std::sync::OnceLock;

use crate::{Bounds, Decorations, Pixels, Size, WindowBounds};

pub(crate) const GEOMETRY_VIEWPORT: u8 = 1 << 0;
pub(crate) const GEOMETRY_BOUNDS: u8 = 1 << 1;
pub(crate) const GEOMETRY_WINDOW_BOUNDS: u8 = 1 << 2;
pub(crate) const GEOMETRY_INNER_BOUNDS: u8 = 1 << 3;
pub(crate) const GEOMETRY_MAXIMIZED: u8 = 1 << 4;
pub(crate) const GEOMETRY_FULLSCREEN: u8 = 1 << 5;
pub(crate) const GEOMETRY_DECORATIONS: u8 = 1 << 6;

/// The parts of a window's geometry a cached view read (`reads`) and their
/// values then; parts not read are `None`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GeometrySnapshot {
    pub(crate) reads: u8,
    pub(crate) viewport_size: Option<Size<Pixels>>,
    pub(crate) bounds: Option<Bounds<Pixels>>,
    pub(crate) window_bounds: Option<WindowBounds>,
    pub(crate) inner_window_bounds: Option<WindowBounds>,
    pub(crate) maximized: Option<bool>,
    pub(crate) fullscreen: Option<bool>,
    pub(crate) decorations: Option<Decorations>,
}

/// Each condition a cached view's last frame must meet to be reused.
#[derive(Debug)]
pub(crate) struct CacheCheck {
    pub(crate) bounds: bool,
    pub(crate) content_mask: bool,
    pub(crate) text_style: bool,
    pub(crate) clean: bool,
    pub(crate) not_refreshing: bool,
    pub(crate) geometry: bool,
}

impl CacheCheck {
    pub(crate) fn reusable(&self) -> bool {
        self.bounds
            && self.content_mask
            && self.text_style
            && self.clean
            && self.not_refreshing
            && self.geometry
    }
}

pub(crate) fn trace_cache_check(check: &CacheCheck) {
    static TRACE: OnceLock<bool> = OnceLock::new();
    if *TRACE
        .get_or_init(|| std::env::var_os("RMAC_GPUI_CACHE_TRACE").is_some_and(|value| value == "1"))
        && !check.reusable()
    {
        eprintln!("gpui cache miss: {check:?}");
    }
}

pub(crate) fn full_refresh_on_resize() -> bool {
    static FULL_REFRESH: OnceLock<bool> = OnceLock::new();
    *FULL_REFRESH.get_or_init(|| {
        std::env::var_os("RMAC_GPUI_RESIZE_REFRESH").is_some_and(|value| value == "1")
    })
}
