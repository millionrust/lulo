//! XDG Settings portal adapter for [`rmac_appearance`].
//!
//! The portal is intentionally treated as read-only. Writable rmac session
//! preferences belong to a separate authority; this adapter only reports host
//! preferences and follows `SettingChanged` with reconnect behavior.

use async_channel::Sender;
use rmac_appearance::{AppearanceSource, Error, Event, Snapshot, SourceFuture};

#[derive(Clone, Copy, Debug, Default)]
pub struct PortalAppearanceSource;

impl AppearanceSource for PortalAppearanceSource {
    fn snapshot(&self) -> SourceFuture<'_, Snapshot> {
        Box::pin(snapshot())
    }

    fn watch(&self, events: Sender<Event>) -> SourceFuture<'_, ()> {
        Box::pin(watch(events))
    }
}

#[cfg(target_os = "linux")]
pub async fn snapshot() -> Result<Snapshot, Error> {
    let settings = ashpd::desktop::settings::Settings::new()
        .await
        .map_err(|error| portal_error("connect to the Settings portal", error))?;
    read_snapshot(&settings).await
}

#[cfg(not(target_os = "linux"))]
pub async fn snapshot() -> Result<Snapshot, Error> {
    Ok(Snapshot::unavailable(
        "Desktop appearance preferences are read from the Settings portal on Linux.",
    ))
}

#[cfg(target_os = "linux")]
pub async fn watch(events: Sender<Event>) -> Result<(), Error> {
    use std::time::Duration;

    const RECONNECT_DELAY: Duration = Duration::from_secs(1);
    let mut reported_error: Option<Error> = None;
    loop {
        match watch_connection(&events, &mut reported_error).await {
            Ok(()) => return Ok(()),
            Err(_) if events.is_closed() => return Ok(()),
            Err(error) => {
                if reported_error.as_ref() != Some(&error) {
                    if events
                        .send(Event::Unavailable(error.clone()))
                        .await
                        .is_err()
                    {
                        return Ok(());
                    }
                    reported_error = Some(error);
                }
                async_io::Timer::after(RECONNECT_DELAY).await;
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub async fn watch(events: Sender<Event>) -> Result<(), Error> {
    let _ = events
        .send(Event::Snapshot(Snapshot::unavailable(
            "Desktop appearance preferences are read from the Settings portal on Linux.",
        )))
        .await;
    Ok(())
}

#[cfg(target_os = "linux")]
async fn watch_connection(
    events: &Sender<Event>,
    reported_error: &mut Option<Error>,
) -> Result<(), Error> {
    use futures_util::StreamExt as _;

    const APPEARANCE_NAMESPACE: &str = "org.freedesktop.appearance";
    let settings = ashpd::desktop::settings::Settings::new()
        .await
        .map_err(|error| portal_error("connect to the Settings portal", error))?;
    // Subscribe before reading so a change between setup and the first snapshot
    // remains queued instead of being missed.
    let mut changes = settings
        .receive_setting_changed()
        .await
        .map_err(|error| portal_error("subscribe to appearance changes", error))?;
    let current = read_snapshot(&settings).await?;
    if events.send(Event::Snapshot(current)).await.is_err() {
        return Ok(());
    }
    *reported_error = None;

    while let Some(change) = changes.next().await {
        if change.namespace() != APPEARANCE_NAMESPACE || !is_supported_key(change.key()) {
            continue;
        }
        let current = read_snapshot(&settings).await?;
        if events.send(Event::Snapshot(current)).await.is_err() {
            return Ok(());
        }
    }
    Err(Error::new(
        "watch appearance settings",
        "the Settings portal signal stream ended",
    ))
}

#[cfg(target_os = "linux")]
async fn read_snapshot(
    settings: &ashpd::desktop::settings::Settings<'_>,
) -> Result<Snapshot, Error> {
    use ashpd::desktop::settings::{ColorScheme as PortalScheme, Contrast as PortalContrast};
    use rmac_appearance::{AccentColor, Capabilities, ColorScheme, Contrast, MotionPreference};

    const APPEARANCE_NAMESPACE: &str = "org.freedesktop.appearance";
    const REDUCED_MOTION_KEY: &str = "reduced-motion";

    let color_scheme = settings.color_scheme().await.ok();
    let accent_color = settings
        .accent_color()
        .await
        .ok()
        .and_then(|color| AccentColor::new(color.red(), color.green(), color.blue()));
    let contrast = settings.contrast().await.ok();
    let reduced_motion = settings
        .read::<u32>(APPEARANCE_NAMESPACE, REDUCED_MOTION_KEY)
        .await
        .ok();
    let capabilities = Capabilities {
        color_scheme: color_scheme.is_some(),
        accent_color: accent_color.is_some(),
        contrast: contrast.is_some(),
        reduced_motion: reduced_motion.is_some(),
    };
    let detail = (!capabilities.any()).then(|| {
        "The Settings portal is available but exposes no standardized appearance keys.".to_string()
    });
    Ok(Snapshot {
        available: true,
        color_scheme: match color_scheme.unwrap_or_default() {
            PortalScheme::PreferDark => ColorScheme::PreferDark,
            PortalScheme::PreferLight => ColorScheme::PreferLight,
            PortalScheme::NoPreference => ColorScheme::NoPreference,
        },
        accent_color,
        contrast: match contrast.unwrap_or_default() {
            PortalContrast::High => Contrast::Higher,
            PortalContrast::NoPreference => Contrast::Normal,
        },
        motion: if reduced_motion == Some(1) {
            MotionPreference::Reduced
        } else {
            MotionPreference::Full
        },
        capabilities,
        detail,
    })
}

#[cfg(target_os = "linux")]
fn is_supported_key(key: &str) -> bool {
    matches!(
        key,
        "color-scheme" | "accent-color" | "contrast" | "reduced-motion"
    )
}

#[cfg(target_os = "linux")]
fn portal_error(operation: &'static str, error: impl std::fmt::Display) -> Error {
    Error::new(operation, error.to_string())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::is_supported_key;

    #[test]
    fn filters_only_standardized_appearance_keys() {
        assert!(is_supported_key("color-scheme"));
        assert!(is_supported_key("accent-color"));
        assert!(is_supported_key("contrast"));
        assert!(is_supported_key("reduced-motion"));
        assert!(!is_supported_key("clock-format"));
    }
}
