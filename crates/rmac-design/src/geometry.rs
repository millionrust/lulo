//! Spacing, corner radii, and component metrics in logical pixels.

/// The four-point spacing rhythm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spacing {
    pub x1: f32,
    pub x2: f32,
    pub x3: f32,
    pub x4: f32,
    pub x5: f32,
    pub x6: f32,
    pub x8: f32,
}

impl Default for Spacing {
    fn default() -> Self {
        Self {
            x1: 4.0,
            x2: 8.0,
            x3: 12.0,
            x4: 16.0,
            x5: 20.0,
            x6: 24.0,
            x8: 32.0,
        }
    }
}

/// Corner radii.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Radii {
    pub control: f32,
    pub card: f32,
    pub popover: f32,
    pub large: f32,
    pub pill: f32,
    pub menu: f32,
    pub menu_item: f32,
    pub window: f32,
    pub window_toolbar: f32,
    pub dock: f32,
    pub hud: f32,
    pub tooltip: f32,
    pub cc_module: f32,
    pub cc_toggle: f32,
}

impl Default for Radii {
    fn default() -> Self {
        Self {
            control: 8.0,
            card: 12.0,
            popover: 20.0,
            large: 24.0,
            pill: 30.0,
            menu: 10.0,
            menu_item: 6.0,
            // macOS 27 standardized one window radius; the old "16 with a
            // unified toolbar, 12 otherwise" split is gone.
            window: 16.0,
            window_toolbar: 16.0,
            dock: 26.0,
            hud: 28.0,
            tooltip: 8.0,
            cc_module: 18.0,
            cc_toggle: 999.0,
        }
    }
}

impl Radii {
    /// The Dock shelf radius scales with the configured tile size.
    pub fn dock_for_tile(tile: f32) -> f32 {
        tile * 0.46
    }
}

/// Component geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub menubar_height: f32,
    pub menubar_item_height: f32,
    pub menubar_item_padding_x: f32,
    pub menubar_leading_inset: f32,
    pub menubar_trailing_inset: f32,
    pub menubar_status_icon: f32,
    pub menubar_status_hit: f32,

    pub menu_row_height: f32,
    pub menu_padding_v: f32,
    pub menu_padding_h: f32,
    pub menu_min_width: f32,
    pub menu_max_width: f32,
    pub menu_separator_line: f32,
    pub menu_separator_block: f32,
    pub menu_separator_inset: f32,
    pub menu_shortcut_gap: f32,

    pub toolbar_height: f32,
    pub titlebar_height: f32,
    pub sidebar_width_default: f32,
    pub sidebar_width_min: f32,
    pub sidebar_width_max: f32,
    pub sidebar_width_settings: f32,
    pub sidebar_row_height: f32,
    pub list_row_height_compact: f32,
    pub list_row_height_regular: f32,
    pub table_header_height: f32,

    pub control_height_mini: f32,
    pub control_height_small: f32,
    pub control_height_regular: f32,
    pub control_height_large: f32,
    pub button_padding_x_regular: f32,
    pub button_padding_x_small: f32,

    pub switch_regular_width: f32,
    pub switch_regular_height: f32,
    pub switch_regular_thumb: f32,
    pub switch_small_width: f32,
    pub switch_small_height: f32,
    pub switch_small_thumb: f32,
    pub switch_mini_width: f32,
    pub switch_mini_height: f32,
    pub switch_mini_thumb: f32,

    pub slider_track_height: f32,
    pub slider_track_radius: f32,
    pub slider_knob_regular: f32,
    pub slider_knob_small: f32,

    pub checkbox_size: f32,
    pub checkbox_radius: f32,
    pub radio_size: f32,
    pub searchfield_height: f32,
    pub segmented_height: f32,
    pub segmented_radius: f32,

    pub traffic_diameter: f32,
    pub traffic_spacing: f32,
    pub traffic_leading_inset_toolbar: f32,
    pub traffic_leading_inset_titlebar: f32,

    pub dock_tile: f32,
    pub dock_tile_min: f32,
    pub dock_tile_max: f32,
    pub dock_gap: f32,
    pub dock_padding: f32,
    pub dock_bottom_margin: f32,
    pub dock_indicator: f32,
    pub dock_indicator_offset: f32,

    pub tooltip_padding_x: f32,
    pub tooltip_padding_y: f32,
    pub focus_ring_width: f32,
    pub focus_ring_width_high_contrast: f32,
    pub hit_target_min: f32,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            menubar_height: 29.0,
            menubar_item_height: 22.0,
            menubar_item_padding_x: 8.0,
            menubar_leading_inset: 12.0,
            menubar_trailing_inset: 10.0,
            menubar_status_icon: 16.0,
            menubar_status_hit: 22.0,

            menu_row_height: 24.0,
            menu_padding_v: 5.0,
            menu_padding_h: 5.0,
            menu_min_width: 180.0,
            menu_max_width: 420.0,
            menu_separator_line: 1.0,
            menu_separator_block: 9.0,
            menu_separator_inset: 10.0,
            menu_shortcut_gap: 24.0,

            toolbar_height: 52.0,
            titlebar_height: 38.0,
            sidebar_width_default: 220.0,
            sidebar_width_min: 180.0,
            sidebar_width_max: 320.0,
            sidebar_width_settings: 248.0,
            sidebar_row_height: 28.0,
            list_row_height_compact: 24.0,
            list_row_height_regular: 30.0,
            table_header_height: 24.0,

            control_height_mini: 16.0,
            control_height_small: 20.0,
            control_height_regular: 24.0,
            control_height_large: 32.0,
            button_padding_x_regular: 12.0,
            button_padding_x_small: 8.0,

            switch_regular_width: 38.0,
            switch_regular_height: 22.0,
            switch_regular_thumb: 20.0,
            switch_small_width: 32.0,
            switch_small_height: 18.0,
            switch_small_thumb: 16.0,
            switch_mini_width: 26.0,
            switch_mini_height: 15.0,
            switch_mini_thumb: 13.0,

            slider_track_height: 4.0,
            slider_track_radius: 2.0,
            slider_knob_regular: 20.0,
            slider_knob_small: 16.0,

            checkbox_size: 14.0,
            checkbox_radius: 4.0,
            radio_size: 14.0,
            searchfield_height: 28.0,
            segmented_height: 24.0,
            segmented_radius: 7.0,

            traffic_diameter: 12.0,
            traffic_spacing: 20.0,
            traffic_leading_inset_toolbar: 20.0,
            traffic_leading_inset_titlebar: 8.0,

            // Measured 2026-09-18: rendered tile 64, pitch 76 (gap 12), shelf
            // 72 tall (padding 4), 18 above the edge, running dot below it.
            dock_tile: 64.0,
            dock_tile_min: 32.0,
            dock_tile_max: 128.0,
            dock_gap: 12.0,
            dock_padding: 4.0,
            dock_bottom_margin: 18.0,
            dock_indicator: 4.0,
            dock_indicator_offset: 6.0,

            tooltip_padding_x: 8.0,
            tooltip_padding_y: 4.0,
            focus_ring_width: 3.0,
            focus_ring_width_high_contrast: 4.0,
            hit_target_min: 24.0,
        }
    }
}
