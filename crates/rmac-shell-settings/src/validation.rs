//! Complete shared shell settings validation.

use super::*;

pub(super) fn validate(settings: &ShellSettings, path: &Path) -> Result<(), Error> {
    if settings.pinned_apps.len() > MAX_PINNED_APPS {
        return Err(invalid(
            path,
            "pinned apps exceed the 128-item safety limit",
        ));
    }
    let mut pinned = BTreeSet::new();
    for app in &settings.pinned_apps {
        validate_identifier(&app.0, "pinned application", path)?;
        if !pinned.insert(&app.0) {
            return Err(invalid(path, "pinned applications must be unique"));
        }
    }
    if settings.dock_stacks.len() > MAX_DOCK_STACKS {
        return Err(invalid(path, "Dock stacks exceed the 32-item safety limit"));
    }
    let mut stacks = BTreeSet::new();
    for stack in &settings.dock_stacks {
        let key = match &stack.kind {
            DockStackKind::Downloads => "downloads".to_owned(),
            DockStackKind::Path { path: stack_path } => {
                validate_identifier(stack_path, "Dock stack path", path)?;
                let normalized = Path::new(stack_path);
                if !normalized.is_absolute()
                    || normalized.components().any(|component| {
                        matches!(
                            component,
                            std::path::Component::CurDir | std::path::Component::ParentDir
                        )
                    })
                {
                    return Err(invalid(
                        path,
                        "Dock stack path must be a normalized absolute path",
                    ));
                }
                format!("path:{stack_path}")
            }
        };
        if !stacks.insert(key) {
            return Err(invalid(path, "Dock stacks must be unique"));
        }
    }
    if !(1.0..=2.5).contains(&settings.dock.magnification_scale)
        || !settings.dock.magnification_scale.is_finite()
    {
        return Err(invalid(
            path,
            "Dock magnification scale must be finite and between 1.0 and 2.5",
        ));
    }
    if !(MIN_DOCK_TILE_SIZE..=MAX_DOCK_TILE_SIZE).contains(&settings.dock.tile_size)
        || !settings.dock.tile_size.is_finite()
    {
        return Err(invalid(
            path,
            "Dock tile size must be finite and between 32 and 128",
        ));
    }
    if let OutputScope::Named(output) = &settings.dock.outputs {
        validate_identifier(output, "Dock output", path)?;
    }
    validate_wallpaper(&settings.wallpaper.default, path)?;
    for (output, wallpaper) in &settings.wallpaper.per_output {
        validate_identifier(output, "wallpaper output", path)?;
        validate_wallpaper(wallpaper, path)?;
    }
    if let Some(mode) = &settings.focus.selected_mode {
        validate_identifier(mode, "Focus mode", path)?;
    }
    if settings.focus.enabled && settings.focus.selected_mode.is_none() {
        return Err(invalid(path, "enabled Focus requires a selected mode"));
    }
    if settings.focus.ends_at_unix_ms.is_some() && !settings.focus.enabled {
        return Err(invalid(
            path,
            "a disabled Focus mode cannot have an end time",
        ));
    }
    for provider in settings.providers.keys() {
        validate_identifier(&provider.0, "provider", path)?;
    }
    if settings.spotlight.excluded_paths.len() > MAX_SPOTLIGHT_EXCLUSIONS {
        return Err(invalid(
            path,
            "Spotlight exclusions exceed the 128-item safety limit",
        ));
    }
    let mut exclusions = BTreeSet::new();
    for exclusion in &settings.spotlight.excluded_paths {
        validate_identifier(exclusion, "Spotlight exclusion", path)?;
        let exclusion_path = Path::new(exclusion);
        if !exclusion_path.is_absolute()
            || exclusion_path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::CurDir | std::path::Component::ParentDir
                )
            })
        {
            return Err(invalid(
                path,
                "Spotlight exclusions must be normalized absolute paths",
            ));
        }
        if !exclusions.insert(exclusion) {
            return Err(invalid(path, "Spotlight exclusions must be unique"));
        }
    }
    Ok(())
}

pub(super) fn validate_wallpaper(selection: &WallpaperSelection, path: &Path) -> Result<(), Error> {
    if let Some(source) = &selection.source {
        validate_identifier(source, "wallpaper source", path)?;
        // Built-in identifiers are owned by rmac-wallpaper, which resolves an
        // unknown one to its fallback with a visible issue; here only the
        // shape is checked, so every shipped built-in (not just Aurora) saves.
        if let Some(id) = source.strip_prefix("builtin:") {
            let well_formed = !id.is_empty()
                && id.len() <= 64
                && id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
            if well_formed {
                return Ok(());
            }
            return Err(invalid(path, "wallpaper built-in identifier is malformed"));
        }
        let source_path = if source.starts_with("file:") {
            let url = url::Url::parse(source)
                .map_err(|_| invalid(path, "wallpaper file URI is invalid"))?;
            if url.scheme() != "file" || url.host_str().is_some() {
                return Err(invalid(path, "wallpaper file URI must be local"));
            }
            url.to_file_path()
                .map_err(|_| invalid(path, "wallpaper file URI is invalid"))?
        } else {
            if source.contains("://") {
                return Err(invalid(path, "wallpaper source scheme is unsupported"));
            }
            PathBuf::from(source)
        };
        if !source_path.is_absolute()
            || source_path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::CurDir | std::path::Component::ParentDir
                )
            })
        {
            return Err(invalid(
                path,
                "wallpaper file path must be normalized and absolute",
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_identifier(value: &str, label: &str, path: &Path) -> Result<(), Error> {
    if value.trim().is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(invalid(
            path,
            format!("{label} must be non-empty, bounded, and contain no control characters"),
        ));
    }
    Ok(())
}

pub(super) fn invalid(path: &Path, detail: impl Into<String>) -> Error {
    Failure::message(Operation::ValidateSettings, path, detail)
}

pub(super) fn parent_path(path: &Path) -> Result<&Path, Error> {
    path.parent().ok_or_else(|| {
        Failure::message(
            Operation::ResolvePath,
            path,
            "shell settings path has no parent directory",
        )
    })
}

pub(super) fn event_targets_path(paths: &[PathBuf], target: &Path) -> bool {
    paths.iter().any(|path| path == target)
}

pub(super) fn shell_settings_path() -> Result<PathBuf, Error> {
    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
        Failure::message(
            Operation::ResolvePath,
            Path::new("shell.json"),
            "HOME is not set",
        )
    })?;
    #[cfg(target_os = "macos")]
    let path = home.join("Library/Application Support/rmac/shell.json");
    #[cfg(not(target_os = "macos"))]
    let path = match std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from) {
        Some(xdg) if xdg.is_absolute() => xdg.join("rmac/shell.json"),
        _ => home.join(".config/rmac/shell.json"),
    };
    Ok(path)
}
