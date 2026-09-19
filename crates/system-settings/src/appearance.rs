#[derive(Clone, Copy)]
pub(super) enum ThemeChange {
    Scheme(rmac_theme::SchemePreference),
    Accent(rmac_theme::AccentPreference),
    Contrast(rmac_theme::ContrastPreference),
    Motion(rmac_theme::MotionPreferenceSetting),
    TextScale(rmac_theme::TextScalePreference),
    WallpaperTinting(bool),
}

#[derive(Clone, Copy)]
pub(super) enum ThemeStoreWatchEvent {
    Available,
    Changed,
    Unavailable,
}

pub(super) struct ThemeLoad {
    pub(super) host: rmac_appearance::Snapshot,
    pub(super) theme: rmac_theme::Snapshot,
}

pub(super) const ACCENTS: &[(&str, u32)] = &[
    ("Blue", 0x1372f9),
    ("Purple", 0xaf52de),
    ("Pink", 0xff2d55),
    ("Red", 0xff3b30),
    ("Orange", 0xff9500),
    ("Yellow", 0xffcc00),
    ("Green", 0x34c759),
    ("Graphite", 0x8e8e93),
];

pub(super) type ThemeOption = (&'static str, ThemeChange);

pub(super) const THEME_CONTRAST_OPTIONS: [ThemeOption; 3] = [
    (
        "Automatic",
        ThemeChange::Contrast(rmac_theme::ContrastPreference::Automatic),
    ),
    (
        "Normal",
        ThemeChange::Contrast(rmac_theme::ContrastPreference::Normal),
    ),
    (
        "Higher",
        ThemeChange::Contrast(rmac_theme::ContrastPreference::Higher),
    ),
];

pub(super) const THEME_MOTION_OPTIONS: [ThemeOption; 3] = [
    (
        "Automatic",
        ThemeChange::Motion(rmac_theme::MotionPreferenceSetting::Automatic),
    ),
    (
        "Full",
        ThemeChange::Motion(rmac_theme::MotionPreferenceSetting::Full),
    ),
    (
        "Reduced",
        ThemeChange::Motion(rmac_theme::MotionPreferenceSetting::Reduced),
    ),
];

pub(super) const THEME_TEXT_SCALE_OPTIONS: [ThemeOption; 3] = [
    (
        "Standard",
        ThemeChange::TextScale(rmac_theme::TextScalePreference::Standard),
    ),
    (
        "Large",
        ThemeChange::TextScale(rmac_theme::TextScalePreference::Large),
    ),
    (
        "Extra Large",
        ThemeChange::TextScale(rmac_theme::TextScalePreference::ExtraLarge),
    ),
];

pub(super) fn accent_preference(hex: u32) -> rmac_theme::AccentPreference {
    rmac_theme::AccentPreference::Custom([
        f64::from((hex >> 16) & 0xff) / 255.0,
        f64::from((hex >> 8) & 0xff) / 255.0,
        f64::from(hex & 0xff) / 255.0,
    ])
}

pub(super) async fn load_theme_state() -> std::result::Result<ThemeLoad, String> {
    let host = match rmac_appearance_portal::snapshot().await {
        Ok(host) => host,
        Err(_) => rmac_appearance::Snapshot::unavailable(
            "The desktop Settings portal is temporarily unavailable.",
        ),
    };
    let store = rmac_theme::ThemeStore::from_environment()
        .map_err(|_| "the rmac appearance preference authority is unavailable".to_string())?;
    let theme = store
        .load(&host)
        .map_err(|_| "the rmac appearance preferences could not be read".to_string())?;
    Ok(ThemeLoad { host, theme })
}

fn apply_theme_change_to_preferences(
    preferences: &mut rmac_theme::Preferences,
    change: ThemeChange,
) {
    match change {
        ThemeChange::Scheme(value) => preferences.color_scheme = value,
        ThemeChange::Accent(value) => preferences.accent_color = value,
        ThemeChange::Contrast(value) => preferences.contrast = value,
        ThemeChange::Motion(value) => preferences.motion = value,
        ThemeChange::TextScale(value) => preferences.text_scale = value,
        ThemeChange::WallpaperTinting(value) => preferences.allow_wallpaper_tinting = value,
    }
}

pub(super) async fn apply_theme_change_authoritatively(
    change: ThemeChange,
    expected: rmac_theme::Preferences,
) -> std::result::Result<ThemeLoad, String> {
    let fresh = load_theme_state().await?;
    if fresh.theme.preferences != expected {
        return Err(
            "appearance preferences changed before save; refresh and try again".to_string(),
        );
    }

    let mut requested = fresh.theme.preferences.clone();
    apply_theme_change_to_preferences(&mut requested, change);
    if requested == fresh.theme.preferences {
        return Ok(fresh);
    }

    let store = rmac_theme::ThemeStore::from_environment()
        .map_err(|_| "the rmac appearance preference authority is unavailable".to_string())?;
    store
        .save(&requested, &fresh.host)
        .map_err(|_| "the appearance preference could not be saved".to_string())?;
    let theme = store
        .load(&fresh.host)
        .map_err(|_| "the saved appearance preference could not be read back".to_string())?;
    if theme.preferences != requested {
        return Err("the saved appearance preference did not match after readback".to_string());
    }
    Ok(ThemeLoad {
        host: fresh.host,
        theme,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_touch_only_the_selected_preference() {
        let original = rmac_theme::Preferences {
            color_scheme: rmac_theme::SchemePreference::Dark,
            accent_color: rmac_theme::AccentPreference::Custom([0.1, 0.2, 0.3]),
            contrast: rmac_theme::ContrastPreference::Normal,
            motion: rmac_theme::MotionPreferenceSetting::Full,
            text_scale: rmac_theme::TextScalePreference::Large,
            allow_wallpaper_tinting: true,
        };
        let mut changed = original.clone();
        apply_theme_change_to_preferences(
            &mut changed,
            ThemeChange::Contrast(rmac_theme::ContrastPreference::Higher),
        );
        assert_eq!(changed.contrast, rmac_theme::ContrastPreference::Higher);
        assert_eq!(changed.color_scheme, original.color_scheme);
        assert_eq!(changed.accent_color, original.accent_color);
        assert_eq!(changed.motion, original.motion);
        assert_eq!(changed.text_scale, original.text_scale);
        assert_eq!(
            changed.allow_wallpaper_tinting,
            original.allow_wallpaper_tinting
        );
    }
}
