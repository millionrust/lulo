//! Drift guard: `design-lab/tokens.css` is the design lab's source of truth
//! and every value it shares with this crate must stay identical. The test
//! parses the dark `:root` block and compares each mapped custom property
//! with the Rust token, so changing one side without the other fails CI.

use std::collections::HashMap;

use rmac_appearance::{AccentColor, Contrast, ResolvedColorScheme, TextScale};

use crate::{Colors, Metrics, Radii, Rgba, TypeScale};

const TOKENS_CSS: &str = include_str!("../../../design-lab/tokens.css");

/// Custom properties of the first (dark) `:root { … }` block, comments
/// stripped, keyed without the leading `--`.
fn dark_root_properties() -> HashMap<String, String> {
    let mut source = String::with_capacity(TOKENS_CSS.len());
    let mut rest = TOKENS_CSS;
    while let Some(start) = rest.find("/*") {
        source.push_str(&rest[..start]);
        rest = match rest[start..].find("*/") {
            Some(end) => &rest[start + end + 2..],
            None => "",
        };
    }
    source.push_str(rest);

    let open = source
        .find(":root {")
        .expect("tokens.css has a :root block")
        + ":root {".len();
    let close = open + source[open..].find('}').expect(":root block is closed");
    source[open..close]
        .split(';')
        .filter_map(|declaration| {
            let declaration = declaration.trim();
            let name = declaration.strip_prefix("--")?;
            let (name, value) = name.split_once(':')?;
            Some((name.trim().to_owned(), value.trim().to_owned()))
        })
        .collect()
}

fn px(properties: &HashMap<String, String>, name: &str) -> f32 {
    let value = properties
        .get(name)
        .unwrap_or_else(|| panic!("tokens.css has no --{name}"));
    value
        .strip_suffix("px")
        .unwrap_or(value)
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("--{name}: {value} is not a length"))
}

fn hex(properties: &HashMap<String, String>, name: &str) -> Rgba {
    let value = properties
        .get(name)
        .unwrap_or_else(|| panic!("tokens.css has no --{name}"));
    let digits = value
        .strip_prefix('#')
        .unwrap_or_else(|| panic!("--{name}: {value} is not a hex colour"));
    assert_eq!(digits.len(), 6, "--{name}: {value} must be #RRGGBB");
    Rgba::rgb(u32::from_str_radix(digits, 16).expect("valid hex digits"))
}

#[test]
fn design_lab_tokens_match_rmac_design() {
    let css = dark_root_properties();
    let metrics = Metrics::default();
    let radii = Radii::default();

    let lengths: &[(&str, f32)] = &[
        // menu bar and menus
        ("menubar-h", metrics.menubar_height),
        ("menubar-lead", metrics.menubar_leading_inset),
        ("menubar-trail", metrics.menubar_trailing_inset),
        ("menubar-icon", metrics.menubar_status_icon),
        ("menubar-hit", metrics.menubar_status_hit),
        ("menu-row", metrics.menu_row_height),
        ("menu-pad", metrics.menu_padding_v),
        ("menu-radius", radii.menu),
        ("menu-item-radius", radii.menu_item),
        // Dock
        ("dock-tile", metrics.dock_tile),
        ("dock-gap", metrics.dock_gap),
        ("dock-pad", metrics.dock_padding),
        ("dock-radius", radii.dock),
        ("dock-bottom", metrics.dock_bottom_margin),
        ("dock-dot", metrics.dock_indicator),
        // Control Center
        ("cc-module-radius", radii.cc_module),
        // windows
        ("win-radius", radii.window_toolbar),
        ("win-radius-titlebar", radii.window),
        ("toolbar-h", metrics.toolbar_height),
        ("titlebar-h", metrics.titlebar_height),
        ("traffic", metrics.traffic_diameter),
        ("traffic-hit", metrics.traffic_hit),
        ("traffic-pitch", metrics.traffic_spacing),
        ("traffic-center-toolbar", metrics.traffic_center_toolbar),
        ("traffic-center-titlebar", metrics.traffic_center_titlebar),
        ("title-toolbar", metrics.title_toolbar_size),
        ("title-titlebar", metrics.title_titlebar_size),
        ("title-gap", metrics.title_gap),
        ("toolbar-group-h", metrics.toolbar_group_height),
        ("sidebar-w", metrics.sidebar_width_default),
        ("settings-sidebar", metrics.sidebar_width_settings),
        // green-button tiling popover
        ("tile-w", metrics.tile_popover_width),
        ("tile-h", metrics.tile_popover_height),
        ("tile-radius", radii.tile_popover),
        ("tile-icon-w", metrics.tile_icon_width),
        ("tile-icon-h", metrics.tile_icon_height),
        ("tile-pitch", metrics.tile_icon_pitch),
        // alert
        ("alert-w", metrics.alert_width),
        ("alert-radius", radii.alert),
        ("alert-pad", metrics.alert_padding),
        ("alert-icon", metrics.alert_icon),
        ("alert-btn-h", metrics.alert_button_height),
        ("alert-btn-gap", metrics.alert_button_gap),
        // controls
        ("ctl-h", metrics.control_height_regular),
        ("ctl-radius", radii.control),
        ("switch-w", metrics.switch_regular_width),
        ("switch-h", metrics.switch_regular_height),
        ("switch-thumb", metrics.switch_regular_thumb),
        ("switch-thumb-w", metrics.switch_regular_thumb_width),
        ("slider-track", metrics.slider_track_height),
        ("slider-knob", metrics.slider_knob_regular),
        ("slider-knob-w", metrics.slider_knob_width),
        ("checkbox", metrics.checkbox_size),
        ("checkbox-radius", metrics.checkbox_radius),
        ("radio", metrics.radio_size),
        ("radio-dot", metrics.radio_dot),
        ("popup-chevron", metrics.popup_chevron),
        ("seg-h", metrics.segmented_height),
        ("seg-radius", metrics.segmented_radius),
        ("focus-ring", metrics.focus_ring_width),
        ("row-h", metrics.settings_row_height),
        ("list-row", metrics.list_row_height_compact),
        ("list-row", metrics.list_row_height_regular),
        ("card", radii.card),
    ];
    let mut drift = Vec::new();
    for &(name, rust) in lengths {
        let lab = px(&css, name);
        if (lab - rust).abs() > f32::EPSILON {
            drift.push(format!("--{name}: tokens.css {lab} ≠ rmac-design {rust}"));
        }
    }
    let gap = px(&css, "traffic-gap");
    if (gap - (metrics.traffic_spacing - metrics.traffic_hit)).abs() > f32::EPSILON {
        drift.push(format!(
            "--traffic-gap {gap} must be the pitch minus the hit box"
        ));
    }

    let type_scale = TypeScale::resolve(TextScale::Standard);
    for (name, rust) in [
        ("t-body", type_scale.body.size),
        ("t-callout", type_scale.callout.size),
        ("t-sub", type_scale.subheadline.size),
        ("t-foot", type_scale.footnote.size),
        ("t-head", type_scale.headline.size),
        ("t-title3", type_scale.title3.size),
        ("t-title2", type_scale.title2.size),
        ("t-title1", type_scale.title1.size),
        ("t-large", type_scale.large_title.size),
    ] {
        let lab = px(&css, name);
        if (lab - rust).abs() > f32::EPSILON {
            drift.push(format!("--{name}: tokens.css {lab} ≠ rmac-design {rust}"));
        }
    }

    let accent = AccentColor::new(19.0 / 255.0, 114.0 / 255.0, 249.0 / 255.0)
        .expect("default accent is valid");
    let colors = Colors::resolve(ResolvedColorScheme::Dark, accent, Contrast::Normal);
    for (name, rust) in [
        ("accent", colors.system_blue),
        ("sys-red", colors.system_red),
        ("sys-orange", colors.system_orange),
        ("sys-yellow", colors.system_yellow),
        ("sys-green", colors.system_green),
        ("sys-teal", colors.system_teal),
        ("sys-indigo", colors.system_indigo),
        ("sys-purple", colors.system_purple),
        ("sys-pink", colors.system_pink),
        ("sys-gray", colors.system_gray),
        ("field", colors.field_fill),
        ("statusbar", colors.statusbar),
        ("btn-secondary", colors.button_secondary),
        ("btn-destructive", colors.button_destructive),
    ] {
        let lab = hex(&css, name);
        if lab != rust {
            drift.push(format!(
                "--{name}: tokens.css {lab:?} ≠ rmac-design {rust:?}"
            ));
        }
    }

    assert!(
        drift.is_empty(),
        "design-lab/tokens.css and rmac-design disagree:\n{}",
        drift.join("\n")
    );
}
