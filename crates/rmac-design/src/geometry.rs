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
    /// Title-bar-only windows (TextEdit, Terminal).
    pub window: f32,
    /// Windows with a unified toolbar (Finder, Settings, Calculator).
    pub window_toolbar: f32,
    /// The green button's Move & Resize popover.
    pub tile_popover: f32,
    /// Alert panels.
    pub alert: f32,
    pub dock: f32,
    pub hud: f32,
    pub tooltip: f32,
    pub cc_module: f32,
    pub cc_toggle: f32,
}

impl Default for Radii {
    fn default() -> Self {
        Self {
            // Measured 2026-09-23 on TextEdit Settings: push buttons, text
            // fields and segmented controls all round at 6.
            control: 6.0,
            card: 12.0,
            popover: 20.0,
            large: 24.0,
            pill: 30.0,
            menu: 10.0,
            menu_item: 6.0,
            // Measured 2026-09-23 (design-lab/chrome.html): a circle fit to
            // the 2x corner gives 32–34 px on TextEdit and 54 px on
            // Calculator, Finder and System Settings.
            window: 16.0,
            window_toolbar: 27.0,
            tile_popover: 13.0,
            alert: 27.0,
            // Measured 2026-09-23: 28.5 at tile 64 (design-lab/dock.html).
            dock: 28.5,
            hud: 28.0,
            tooltip: 8.0,
            // design-lab/control-center.html: slider module radius 26.
            cc_module: 26.0,
            cc_toggle: 999.0,
        }
    }
}

impl Radii {
    /// The Dock shelf radius scales with the configured tile size: 28.5 at
    /// tile 64 on the owner's Mac (design-lab/dock.html).
    pub fn dock_for_tile(tile: f32) -> f32 {
        tile * 0.445
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
    /// Tahoe switch thumbs are capsules wider than they are tall.
    pub switch_regular_thumb_width: f32,
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
    /// Width of the capsule knob; `slider_knob_regular` is its height.
    pub slider_knob_width: f32,

    pub checkbox_size: f32,
    pub checkbox_radius: f32,
    pub radio_size: f32,
    pub radio_dot: f32,
    /// The circle holding ⌃⌄ at the end of a form pop-up button.
    pub popup_chevron: f32,
    pub searchfield_height: f32,
    pub segmented_height: f32,
    pub segmented_radius: f32,

    pub traffic_diameter: f32,
    /// Centre-to-centre pitch of the three lights.
    pub traffic_spacing: f32,
    /// Each light's square hit box (the AX button frame).
    pub traffic_hit: f32,
    /// The first light's centre from the window's left and top edges.
    pub traffic_center_toolbar: f32,
    pub traffic_center_titlebar: f32,
    /// Window title sizes (bold) and the gap after the last hit box.
    pub title_toolbar_size: f32,
    pub title_titlebar_size: f32,
    pub title_gap: f32,
    /// Height of a toolbar's glass capsule group.
    pub toolbar_group_height: f32,

    pub tile_popover_width: f32,
    pub tile_popover_height: f32,
    pub tile_icon_width: f32,
    pub tile_icon_height: f32,
    pub tile_icon_pitch: f32,

    pub alert_width: f32,
    pub alert_padding: f32,
    pub alert_icon: f32,
    pub alert_text_width: f32,
    pub alert_button_height: f32,
    pub alert_button_gap: f32,

    pub settings_row_height: f32,

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
            // Lights centred 16 from the top (design-lab/chrome.html).
            titlebar_height: 32.0,
            sidebar_width_default: 220.0,
            sidebar_width_min: 180.0,
            sidebar_width_max: 320.0,
            sidebar_width_settings: 248.0,
            sidebar_row_height: 28.0,
            list_row_height_compact: 24.0,
            list_row_height_regular: 24.0,
            table_header_height: 24.0,

            control_height_mini: 16.0,
            control_height_small: 20.0,
            control_height_regular: 24.0,
            control_height_large: 32.0,
            button_padding_x_regular: 12.0,
            button_padding_x_small: 8.0,

            // Measured 2026-09-23: AX 36 × 16; a 21 × 13 capsule thumb
            // inset 1.5 (design-lab/chrome.html).
            switch_regular_width: 36.0,
            switch_regular_height: 16.0,
            switch_regular_thumb: 13.0,
            switch_regular_thumb_width: 21.0,
            // Not measured: the small and mini switches keep the pre-Tahoe
            // AppKit ratios to the regular one (32/38 and 26/38) until a Mac
            // app that uses them is captured. No rmac surface uses them yet.
            switch_small_width: 30.0,
            switch_small_height: 13.0,
            switch_small_thumb: 10.0,
            switch_mini_width: 25.0,
            switch_mini_height: 11.0,
            switch_mini_thumb: 8.5,

            // Measured on Desktop & Dock: a 6 pt track and a 20 × 16 knob.
            slider_track_height: 6.0,
            slider_track_radius: 3.0,
            slider_knob_regular: 16.0,
            slider_knob_small: 16.0,
            slider_knob_width: 20.0,

            checkbox_size: 14.0,
            checkbox_radius: 4.0,
            radio_size: 14.0,
            radio_dot: 6.0,
            popup_chevron: 20.0,
            searchfield_height: 28.0,
            segmented_height: 24.0,
            segmented_radius: 6.0,

            // AX frames on TextEdit, Terminal, Finder, Calculator and System
            // Settings (design-lab/chrome.html): 16 pt boxes 23 apart, the
            // first centred (16, 16) or (26, 26).
            traffic_diameter: 14.0,
            traffic_spacing: 23.0,
            traffic_hit: 16.0,
            traffic_center_toolbar: 26.0,
            traffic_center_titlebar: 16.0,
            title_toolbar_size: 15.0,
            title_titlebar_size: 13.0,
            title_gap: 13.0,
            toolbar_group_height: 36.0,

            tile_popover_width: 229.0,
            tile_popover_height: 193.0,
            tile_icon_width: 25.0,
            tile_icon_height: 20.0,
            tile_icon_pitch: 51.5,

            alert_width: 260.0,
            alert_padding: 16.0,
            alert_icon: 64.0,
            alert_text_width: 180.0,
            alert_button_height: 28.0,
            alert_button_gap: 8.0,

            settings_row_height: 38.0,

            // Measured 2026-09-23 (design-lab/dock.html): tile 64 with a 52
            // visible squircle, pitch 68 (gap 4), shelf 84 tall (padding 10),
            // 5 above the screen edge, a 4 pt dot 2 below the tile.
            dock_tile: 64.0,
            dock_tile_min: 32.0,
            dock_tile_max: 128.0,
            dock_gap: 4.0,
            dock_padding: 10.0,
            dock_bottom_margin: 5.0,
            dock_indicator: 4.0,
            dock_indicator_offset: 2.0,

            tooltip_padding_x: 8.0,
            tooltip_padding_y: 4.0,
            focus_ring_width: 3.0,
            focus_ring_width_high_contrast: 4.0,
            hit_target_min: 24.0,
        }
    }
}
