//! Live design tokens for the shell.
//!
//! All shell surfaces read colors, radii, and metrics from here. The values
//! come from [`rmac_design`], the same token source the applications use, and
//! update when the host appearance or an rmac preference changes.

use std::sync::{OnceLock, RwLock};

use gpui::{App, AsyncApp, Hsla};
use rmac_design::{Rgba, Tokens as DesignTokens};

static CURRENT: OnceLock<RwLock<DesignTokens>> = OnceLock::new();

fn store() -> &'static RwLock<DesignTokens> {
    CURRENT.get_or_init(|| RwLock::new(DesignTokens::light_default()))
}

/// The current resolved design tokens.
pub fn current() -> DesignTokens {
    *store().read().expect("shell token lock poisoned")
}

/// Replace the current tokens; returns whether they changed.
pub fn set(tokens: DesignTokens) -> bool {
    let mut current = store().write().expect("shell token lock poisoned");
    if *current == tokens {
        false
    } else {
        *current = tokens;
        true
    }
}

/// Convert a design color to a GPUI color.
pub fn hsla(color: Rgba) -> Hsla {
    if color.alpha() == 0xff {
        gpui::rgb(color.0 >> 8).into()
    } else {
        gpui::rgba(color.0).into()
    }
}

fn hex(color: Rgba) -> u32 {
    color.0
}

/// Start watching the host appearance and rmac preferences, repainting every
/// shell surface when the resolved tokens change.
pub fn install_appearance_watch(cx: &mut App) {
    let initial = cx
        .background_executor()
        .spawn(async { load_tokens().await });
    cx.spawn(async move |cx: &mut AsyncApp| {
        if let Ok(tokens) = initial.await {
            apply(tokens, cx);
        }
    })
    .detach();

    let (portal_tx, portal_rx) = async_channel::bounded(8);
    cx.background_executor()
        .spawn(async move { rmac_appearance_portal::watch(portal_tx).await })
        .detach();
    cx.spawn(async move |cx: &mut AsyncApp| {
        while let Ok(event) = portal_rx.recv().await {
            let rmac_appearance::Event::Snapshot(host) = event else {
                continue;
            };
            let result = cx
                .background_executor()
                .spawn(async move { load_tokens_with_host(host) })
                .await;
            if let Ok(tokens) = result {
                apply(tokens, cx);
            }
        }
    })
    .detach();

    cx.spawn(async move |cx: &mut AsyncApp| {
        let watcher = cx
            .background_executor()
            .spawn(async {
                rmac_theme::ThemeStore::from_environment()
                    .and_then(|store| store.watch())
                    .map_err(|error| error.to_string())
            })
            .await;
        let Ok(watcher) = watcher else {
            return;
        };
        while let Ok(event) = watcher.recv().await {
            if !matches!(event, rmac_theme::StoreEvent::Changed) {
                continue;
            }
            let result = cx
                .background_executor()
                .spawn(async { load_tokens().await })
                .await;
            if let Ok(tokens) = result {
                apply(tokens, cx);
            }
        }
    })
    .detach();
}

async fn load_tokens() -> Result<DesignTokens, String> {
    let host = match rmac_appearance_portal::snapshot().await {
        Ok(host) => host,
        Err(error) => rmac_appearance::Snapshot::unavailable(error.to_string()),
    };
    load_tokens_with_host(host)
}

fn load_tokens_with_host(host: rmac_appearance::Snapshot) -> Result<DesignTokens, String> {
    let store = rmac_theme::ThemeStore::from_environment().map_err(|error| error.to_string())?;
    let resolved = store.load(&host).map_err(|error| error.to_string())?;
    Ok(DesignTokens::resolve(resolved.effective))
}

fn apply(tokens: DesignTokens, cx: &mut AsyncApp) {
    if !set(tokens) {
        return;
    }
    cx.update(|app| app.refresh_windows());
}

// Semantic accessors used by the shell hosts. Each reads the live tokens so a
// repaint after an appearance change recolors every surface.

pub fn primary_text() -> u32 {
    hex(current().colors.label_primary)
}

pub fn secondary_text() -> u32 {
    hex(current().colors.label_secondary)
}

pub fn disabled_text() -> u32 {
    hex(current().colors.label_disabled)
}

pub fn top_bar_tint() -> u32 {
    hex(current().materials.menubar.fallback)
}

pub fn regular_dark_tint() -> u32 {
    hex(current().materials.menu.tint)
}

pub fn hud_tint() -> u32 {
    hex(current().materials.hud.tint)
}

pub fn dock_tint() -> u32 {
    hex(current().materials.dock.tint)
}

pub fn light_hover() -> u32 {
    hex(current().colors.fill_hover)
}

pub fn light_selection() -> u32 {
    hex(current().colors.selection_unfocused)
}

pub fn light_border() -> u32 {
    hex(current().colors.separator)
}

pub fn dock_border() -> u32 {
    hex(current().materials.dock.border)
}

pub fn separator() -> u32 {
    hex(current().colors.separator)
}

pub fn accent() -> u32 {
    hex(current().colors.accent)
}

pub fn accent_hover() -> u32 {
    hex(current().colors.accent)
}

pub fn menu_radius() -> f32 {
    current().radii.menu
}

pub fn menu_item_radius() -> f32 {
    current().radii.menu_item
}

pub fn dock_radius() -> f32 {
    current().radii.dock
}

pub fn tooltip_radius() -> f32 {
    current().radii.tooltip
}

pub fn hud_radius() -> f32 {
    current().radii.hud
}

pub fn control_radius() -> f32 {
    current().radii.control
}

pub fn card_radius() -> f32 {
    current().radii.card
}

pub fn pill_radius() -> f32 {
    current().radii.pill
}

/// Dock app-tile corner radius: the tile radius scales with the icon size
/// rather than the shelf radius token.
pub fn dock_tile_radius(tile: f32) -> f32 {
    tile * 0.232
}

pub fn transparent() -> u32 {
    0
}

pub fn surface_window() -> u32 {
    hex(current().colors.surface_window)
}

pub fn surface_raised() -> u32 {
    hex(current().colors.surface_raised)
}

pub fn fill_control() -> u32 {
    hex(current().colors.fill_control)
}

pub fn danger() -> u32 {
    hex(current().colors.danger)
}

pub fn system_red() -> u32 {
    hex(current().colors.system_red)
}

pub fn system_blue() -> u32 {
    hex(current().colors.system_blue)
}

pub fn on_accent() -> u32 {
    hex(current().colors.on_accent)
}

pub fn selection_text() -> u32 {
    hex(current().colors.selection_text)
}

pub fn overlay_chip() -> u32 {
    hex(current().materials.hud.tint)
}

pub fn tooltip_tint() -> u32 {
    hex(current().materials.tooltip.tint)
}

pub fn tooltip_border() -> u32 {
    hex(current().materials.tooltip.border)
}

/// A translucent primary-color tick/dot for HUD controls.
pub fn overlay_tick() -> u32 {
    hex(current().colors.label_primary.with_alpha(0xb5))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_appearance::{Contrast, MotionPreference, ResolvedColorScheme, TextScale};

    fn appearance(scheme: ResolvedColorScheme) -> rmac_appearance::ResolvedAppearance {
        rmac_appearance::ResolvedAppearance {
            color_scheme: scheme,
            accent_color: rmac_appearance::AccentColor::new(0.0, 0.478, 1.0).unwrap(),
            contrast: Contrast::Normal,
            motion: MotionPreference::Full,
            text_scale: TextScale::Standard,
        }
    }

    #[test]
    fn surface_colors_follow_the_resolved_appearance() {
        set(DesignTokens::resolve(appearance(
            ResolvedColorScheme::Light,
        )));
        assert_eq!(top_bar_tint(), 0xf6f6f6f2);
        assert_eq!(primary_text(), 0x1d1d1fff);
        assert_eq!(menu_radius(), 10.0);

        set(DesignTokens::resolve(appearance(ResolvedColorScheme::Dark)));
        assert_eq!(top_bar_tint(), 0x1e1e20f2);
        assert_eq!(primary_text(), 0xf5f5f7ff);

        assert!(!set(DesignTokens::resolve(appearance(
            ResolvedColorScheme::Dark
        ))));
    }
}
