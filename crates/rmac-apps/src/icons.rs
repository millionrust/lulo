//! XDG icon-theme discovery and exact bounded resolution.

use super::*;

pub(super) fn resolve_icon(icon: &str, environment: &Environment) -> Option<PathBuf> {
    let icon_path = Path::new(icon);
    if icon_path.is_absolute() && icon_path.is_file() {
        return Some(icon_path.to_path_buf());
    }
    resolve_named_icon(icon, 64, environment)
}

pub(super) fn resolve_named_icon(
    icon: &str,
    pixel_edge: u32,
    environment: &Environment,
) -> Option<PathBuf> {
    let bases = icon_base_directories(environment);
    let icon = normalized_icon_name(icon)?;
    let theme = environment.icon_theme.as_deref().unwrap_or("Adwaita");
    for theme in icon_theme_order(theme, &bases, &environment.theme_cache) {
        let Some(metadata) = load_icon_theme_cached(&theme, &bases, &environment.theme_cache)
        else {
            continue;
        };
        if let Some(path) = lookup_icon_in_theme(icon, pixel_edge, 1, &theme, &bases, &metadata) {
            return Some(path);
        }
    }

    let filenames = icon_filenames(icon);
    if let Some(path) = bases
        .iter()
        .flat_map(|base| filenames.iter().map(move |name| base.join(name)))
        .find(|path| path.is_file())
    {
        return Some(path);
    }
    environment
        .data_dirs
        .iter()
        .chain(environment.data_home.iter())
        .flat_map(|directory| {
            filenames
                .iter()
                .map(move |name| directory.join("pixmaps").join(name))
        })
        .find(|path| path.is_file())
}

pub(super) fn active_icon_theme(
    config_home: Option<&Path>,
    prefer_kde: bool,
    prefer_gnome: bool,
) -> Option<String> {
    if let Some(theme) = std::env::var("XDG_ICON_THEME")
        .ok()
        .and_then(|theme| valid_theme_name(&theme))
    {
        return Some(theme);
    }

    if prefer_gnome {
        if let Some(theme) = gsettings_icon_theme() {
            return Some(theme);
        }
    }

    if let Some(theme) = config_home.and_then(|home| configured_icon_theme(home, prefer_kde)) {
        return Some(theme);
    }

    gsettings_icon_theme()
}

pub(super) fn gsettings_icon_theme() -> Option<String> {
    let output = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "icon-theme"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    valid_theme_name(value.trim().trim_matches(['\'', '"']))
}

pub(super) fn configured_icon_theme(config_home: &Path, prefer_kde: bool) -> Option<String> {
    let gtk = [
        (
            config_home.join("gtk-4.0/settings.ini"),
            "Settings",
            "gtk-icon-theme-name",
        ),
        (
            config_home.join("gtk-3.0/settings.ini"),
            "Settings",
            "gtk-icon-theme-name",
        ),
    ]
    .into_iter()
    .find_map(|(path, group, key)| configured_value(&path, group, key));
    let kde = configured_value(&config_home.join("kdeglobals"), "Icons", "Theme");
    if prefer_kde {
        kde.or(gtk)
    } else {
        gtk.or(kde)
    }
}

pub(super) fn configured_value(path: &Path, group: &str, key: &str) -> Option<String> {
    let contents = std::fs::read_to_string(path).ok()?;
    ini_value(&contents, group, key).and_then(valid_theme_name)
}

pub(super) fn valid_theme_name(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.is_ascii()
        && !value.contains([',', '/', '\\'])
        && !value.chars().any(char::is_whitespace))
    .then(|| value.to_string())
}

pub(super) fn ini_value<'a>(contents: &'a str, group: &str, key: &str) -> Option<&'a str> {
    let mut active = false;
    for raw in contents.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            active = &line[1..line.len() - 1] == group;
        } else if active && !line.starts_with('#') {
            if let Some((candidate, value)) = line.split_once('=') {
                if candidate.trim() == key {
                    return Some(value.trim());
                }
            }
        }
    }
    None
}

pub(super) fn icon_base_directories(environment: &Environment) -> Vec<PathBuf> {
    let mut bases = Vec::new();
    if let Some(home) = &environment.home {
        bases.push(home.join(".icons"));
    }
    bases.extend(environment.data_home.iter().map(|path| path.join("icons")));
    bases.extend(environment.data_dirs.iter().map(|path| path.join("icons")));
    bases
}

pub(super) fn normalized_icon_name(icon: &str) -> Option<&str> {
    let icon = Path::new(icon)
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| matches!(*extension, "png" | "svg" | "xpm"))
        .and_then(|_| Path::new(icon).file_stem())
        .and_then(|name| name.to_str())
        .unwrap_or(icon);
    (!icon.is_empty() && !icon.contains(['/', '\\'])).then_some(icon)
}

pub(super) fn icon_filenames(icon: &str) -> [String; 3] {
    [
        format!("{icon}.png"),
        format!("{icon}.svg"),
        format!("{icon}.xpm"),
    ]
}

#[derive(Clone, Copy)]
pub(super) enum IconDirectoryType {
    Fixed,
    Scalable,
    Threshold,
}

#[derive(Clone)]
pub(super) struct IconDirectory {
    pub(super) path: String,
    pub(super) size: u32,
    pub(super) scale: u32,
    pub(super) kind: IconDirectoryType,
    pub(super) min_size: u32,
    pub(super) max_size: u32,
    pub(super) threshold: u32,
}

#[derive(Clone)]
pub(super) struct IconTheme {
    pub(super) inherits: Vec<String>,
    pub(super) directories: Vec<IconDirectory>,
}

pub(super) fn icon_theme_order(
    theme: &str,
    bases: &[PathBuf],
    cache: &RefCell<HashMap<String, Option<IconTheme>>>,
) -> Vec<String> {
    fn visit(
        theme: &str,
        bases: &[PathBuf],
        cache: &RefCell<HashMap<String, Option<IconTheme>>>,
        seen: &mut HashSet<String>,
        order: &mut Vec<String>,
    ) {
        if !seen.insert(theme.to_string()) {
            return;
        }
        order.push(theme.to_string());
        if let Some(metadata) = load_icon_theme_cached(theme, bases, cache) {
            for parent in metadata.inherits {
                visit(&parent, bases, cache, seen, order);
            }
        }
    }

    let mut seen = HashSet::new();
    let mut order = Vec::new();
    visit(theme, bases, cache, &mut seen, &mut order);
    visit("hicolor", bases, cache, &mut seen, &mut order);
    order
}

pub(super) fn load_icon_theme_cached(
    theme: &str,
    bases: &[PathBuf],
    cache: &RefCell<HashMap<String, Option<IconTheme>>>,
) -> Option<IconTheme> {
    if let Some(metadata) = cache.borrow().get(theme) {
        return metadata.clone();
    }
    let metadata = load_icon_theme(theme, bases);
    cache
        .borrow_mut()
        .insert(theme.to_string(), metadata.clone());
    metadata
}

pub(super) fn load_icon_theme(theme: &str, bases: &[PathBuf]) -> Option<IconTheme> {
    let contents = bases
        .iter()
        .find_map(|base| std::fs::read_to_string(base.join(theme).join("index.theme")).ok())?;
    let root = ini_group(&contents, "Icon Theme");
    let inherits = comma_list(root.get("Inherits").copied())
        .into_iter()
        .filter_map(valid_theme_name)
        .collect();
    let directories = comma_list(root.get("Directories").copied())
        .into_iter()
        .chain(comma_list(root.get("ScaledDirectories").copied()))
        .filter_map(|path| parse_icon_directory(&contents, path))
        .collect();
    Some(IconTheme {
        inherits,
        directories,
    })
}

pub(super) fn ini_group<'a>(contents: &'a str, group: &str) -> HashMap<&'a str, &'a str> {
    let mut values = HashMap::new();
    let mut active = false;
    for raw in contents.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            active = &line[1..line.len() - 1] == group;
        } else if active && !line.is_empty() && !line.starts_with('#') {
            if let Some((key, value)) = line.split_once('=') {
                values.entry(key.trim()).or_insert(value.trim());
            }
        }
    }
    values
}

pub(super) fn comma_list(value: Option<&str>) -> Vec<&str> {
    value
        .into_iter()
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect()
}

pub(super) fn parse_icon_directory(contents: &str, path: &str) -> Option<IconDirectory> {
    if Path::new(path).is_absolute()
        || Path::new(path)
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return None;
    }
    let values = ini_group(contents, path);
    let size = values.get("Size")?.parse().ok()?;
    let scale = values
        .get("Scale")
        .and_then(|value| value.parse().ok())
        .unwrap_or(1);
    let kind = match values.get("Type").copied().unwrap_or("Threshold") {
        "Fixed" => IconDirectoryType::Fixed,
        "Scalable" => IconDirectoryType::Scalable,
        "Threshold" => IconDirectoryType::Threshold,
        _ => return None,
    };
    Some(IconDirectory {
        path: path.to_string(),
        size,
        scale,
        kind,
        min_size: values
            .get("MinSize")
            .and_then(|value| value.parse().ok())
            .unwrap_or(size),
        max_size: values
            .get("MaxSize")
            .and_then(|value| value.parse().ok())
            .unwrap_or(size),
        threshold: values
            .get("Threshold")
            .and_then(|value| value.parse().ok())
            .unwrap_or(2),
    })
}

pub(super) fn lookup_icon_in_theme(
    icon: &str,
    size: u32,
    scale: u32,
    theme: &str,
    bases: &[PathBuf],
    metadata: &IconTheme,
) -> Option<PathBuf> {
    let filenames = icon_filenames(icon);
    for directory in metadata
        .directories
        .iter()
        .filter(|directory| directory_matches(directory, size, scale))
    {
        if let Some(path) = find_themed_icon(theme, directory, &filenames, bases) {
            return Some(path);
        }
    }

    metadata
        .directories
        .iter()
        .filter_map(|directory| {
            find_themed_icon(theme, directory, &filenames, bases)
                .map(|path| (directory_distance(directory, size, scale), path))
        })
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, path)| path)
}

pub(super) fn find_themed_icon(
    theme: &str,
    directory: &IconDirectory,
    filenames: &[String],
    bases: &[PathBuf],
) -> Option<PathBuf> {
    bases
        .iter()
        .flat_map(|base| {
            filenames
                .iter()
                .map(move |filename| base.join(theme).join(&directory.path).join(filename))
        })
        .find(|path| path.is_file())
}

pub(super) fn directory_matches(directory: &IconDirectory, size: u32, scale: u32) -> bool {
    if directory.scale != scale {
        return false;
    }
    match directory.kind {
        IconDirectoryType::Fixed => directory.size == size,
        IconDirectoryType::Scalable => (directory.min_size..=directory.max_size).contains(&size),
        IconDirectoryType::Threshold => size.abs_diff(directory.size) <= directory.threshold,
    }
}

pub(super) fn directory_distance(directory: &IconDirectory, size: u32, scale: u32) -> u32 {
    let desired = size.saturating_mul(scale);
    let (minimum, maximum) = match directory.kind {
        IconDirectoryType::Fixed => (directory.size, directory.size),
        IconDirectoryType::Scalable => (directory.min_size, directory.max_size),
        IconDirectoryType::Threshold => (
            directory.size.saturating_sub(directory.threshold),
            directory.size.saturating_add(directory.threshold),
        ),
    };
    let minimum = minimum.saturating_mul(directory.scale);
    let maximum = maximum.saturating_mul(directory.scale);
    if desired < minimum {
        minimum - desired
    } else {
        desired.saturating_sub(maximum)
    }
}

pub(super) fn sort_applications(applications: &mut [Application]) {
    applications.sort_by_cached_key(|application| application.name.to_lowercase());
}
