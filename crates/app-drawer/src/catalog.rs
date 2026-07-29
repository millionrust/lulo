#[cfg(target_os = "macos")]
use std::path::Path;
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::process::Command;

use gpui::SharedString;

/// App-Library-style buckets projected from platform catalog metadata.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Category {
    Productivity,
    Internet,
    Media,
    Developer,
    Utilities,
    Games,
    System,
    Other,
}

impl Category {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Productivity => "Productivity",
            Self::Internet => "Internet",
            Self::Media => "Media",
            Self::Developer => "Developer",
            Self::Utilities => "Utilities",
            Self::Games => "Games",
            Self::System => "System",
            Self::Other => "Other",
        }
    }

    pub(crate) const ORDER: [Self; 8] = [
        Self::Productivity,
        Self::Internet,
        Self::Media,
        Self::Developer,
        Self::Utilities,
        Self::Games,
        Self::System,
        Self::Other,
    ];
}

/// Renderer-ready application projection retaining the exact parsed launch
/// specifications and actions supplied by `rmac-apps`.
#[derive(Clone)]
pub(crate) struct App {
    pub(crate) id: String,
    pub(crate) name: SharedString,
    pub(crate) generic_name: Option<String>,
    pub(crate) keywords: Vec<String>,
    pub(crate) path: PathBuf,
    pub(crate) icon: Option<PathBuf>,
    pub(crate) category: Category,
    pub(crate) source_categories: Vec<String>,
    pub(crate) mime_types: Vec<String>,
    pub(crate) search_text: String,
    pub(crate) launch: rmac_apps::LaunchSpec,
    pub(crate) actions: Vec<rmac_apps::DesktopAction>,
}

pub(crate) fn scan() -> (Vec<App>, Option<SharedString>) {
    let catalog = match rmac_apps::discover() {
        Ok(catalog) => catalog,
        Err(error) => {
            return (
                Vec::new(),
                Some(format!("Could not load applications: {error}").into()),
            );
        }
    };

    #[cfg(target_os = "macos")]
    let categories = {
        let pairs = catalog
            .iter()
            .map(|application| (application.name.clone(), application.source.clone()))
            .collect::<Vec<_>>();
        parallel_categorize(&pairs)
    };
    #[cfg(not(target_os = "macos"))]
    let categories = catalog
        .iter()
        .map(|application| categorize_desktop(&application.categories))
        .collect::<Vec<_>>();

    let apps = catalog
        .into_iter()
        .zip(categories)
        .map(|(application, category)| App {
            search_text: format!(
                "{}\n{}",
                application.searchable_text(),
                category.label().to_lowercase()
            ),
            id: application.id,
            name: application.name.into(),
            generic_name: application.generic_name,
            keywords: application.keywords,
            path: application.source,
            icon: application.icon,
            category,
            source_categories: application.categories,
            mime_types: application.mime_types,
            launch: application.launch,
            actions: application.actions,
        })
        .collect();
    (apps, None)
}

pub(crate) fn signal_change(sender: &async_channel::Sender<()>) {
    let _ = sender.try_send(());
}

/// Resolve every macOS application's category concurrently while preserving
/// catalog order.
#[cfg(target_os = "macos")]
fn parallel_categorize(pairs: &[(String, PathBuf)]) -> Vec<Category> {
    let count = pairs.len();
    if count == 0 {
        return Vec::new();
    }
    let workers = 8.min(count);
    let chunk = count.div_ceil(workers);
    let mut result = vec![Category::Other; count];
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for (chunk_index, slice) in pairs.chunks(chunk).enumerate() {
            handles.push((
                chunk_index,
                scope.spawn(move || {
                    slice
                        .iter()
                        .map(|(name, path)| categorize(name, path))
                        .collect::<Vec<_>>()
                }),
            ));
        }
        for (chunk_index, handle) in handles {
            if let Ok(part) = handle.join() {
                let start = chunk_index * chunk;
                for (index, category) in part.into_iter().enumerate() {
                    result[start + index] = category;
                }
            }
        }
    });
    result
}

#[cfg(target_os = "macos")]
fn real_category(path: &Path) -> Option<Category> {
    let info = path.join("Contents/Info");
    let output = Command::new("defaults")
        .arg("read")
        .arg(&info)
        .arg("LSApplicationCategoryType")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_lowercase();
    if value.is_empty() {
        return None;
    }
    Some(if value.contains("developer") {
        Category::Developer
    } else if value.contains("game") {
        Category::Games
    } else if value.contains("music")
        || value.contains("video")
        || value.contains("photo")
        || value.contains("entertainment")
        || value.contains("graphics")
    {
        Category::Media
    } else if value.contains("social") || value.contains("news") {
        Category::Internet
    } else if value.contains("utilit") {
        Category::Utilities
    } else if value.contains("productivity")
        || value.contains("business")
        || value.contains("finance")
        || value.contains("reference")
        || value.contains("education")
        || value.contains("weather")
    {
        Category::Productivity
    } else {
        Category::Other
    })
}

#[cfg(target_os = "macos")]
fn categorize(name: &str, path: &Path) -> Category {
    if let Some(category) = real_category(path) {
        return category;
    }
    let path_text = path.to_string_lossy();
    if path_text.contains("/Utilities/") {
        return Category::Utilities;
    }
    let normalized_name = name.to_lowercase();

    const INTERNET: &[&str] = &[
        "safari", "mail", "messages", "facetime", "chrome", "firefox", "edge", "news", "contacts",
        "freeform", "maps",
    ];
    const MEDIA: &[&str] = &[
        "music",
        "tv",
        "photos",
        "podcasts",
        "quicktime",
        "books",
        "voice memos",
        "image capture",
        "photo booth",
        "garageband",
        "imovie",
    ];
    const PRODUCTIVITY: &[&str] = &[
        "calendar",
        "notes",
        "reminders",
        "numbers",
        "pages",
        "keynote",
        "stocks",
        "weather",
        "calculator",
        "dictionary",
        "home",
        "clock",
        "shortcuts",
        "preview",
        "stickies",
        "textedit",
        "font book",
    ];
    const DEVELOPER: &[&str] = &[
        "xcode",
        "terminal",
        "script editor",
        "automator",
        "console",
        "instruments",
        "simulator",
        "visual studio",
        "code",
    ];
    const GAMES: &[&str] = &["chess", "game center"];

    let contains_any = |needles: &[&str]| {
        needles
            .iter()
            .any(|needle| normalized_name.contains(needle))
    };

    if contains_any(GAMES) {
        Category::Games
    } else if contains_any(DEVELOPER) {
        Category::Developer
    } else if contains_any(INTERNET) {
        Category::Internet
    } else if contains_any(MEDIA) {
        Category::Media
    } else if contains_any(PRODUCTIVITY) {
        Category::Productivity
    } else if name == "System Settings"
        || name == "App Store"
        || name == "Find My"
        || name == "Passwords"
        || name == "Tips"
        || path_text.starts_with("/System/Applications")
    {
        Category::System
    } else {
        Category::Other
    }
}

#[cfg(not(target_os = "macos"))]
fn categorize_desktop(categories: &[String]) -> Category {
    let has = |names: &[&str]| {
        categories
            .iter()
            .any(|category| names.contains(&category.as_str()))
    };
    if has(&["Game"]) {
        Category::Games
    } else if has(&["Development", "IDE", "Building", "Debugger"]) {
        Category::Developer
    } else if has(&["Network", "WebBrowser", "Email", "InstantMessaging"]) {
        Category::Internet
    } else if has(&["AudioVideo", "Audio", "Video", "Graphics", "Photography"]) {
        Category::Media
    } else if has(&["Office", "Education", "Science", "Finance"]) {
        Category::Productivity
    } else if has(&["Settings", "System"]) {
        Category::System
    } else if has(&["Utility", "FileTools", "Archiving"]) {
        Category::Utilities
    } else {
        Category::Other
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn cache_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_owned());
    let directory = PathBuf::from(home).join("Library/Caches/rmac-app-drawer");
    std::fs::create_dir_all(&directory).ok();
    directory
}

#[cfg(target_os = "macos")]
pub(crate) fn hydrate_icons(apps: &mut [App]) {
    let cache = cache_dir();
    for app in apps {
        app.icon = extract_icon(&app.name, &app.path, &cache);
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn extract_icon(name: &str, app: &Path, cache: &Path) -> Option<PathBuf> {
    let safe_name: String = name
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect();
    let output = cache.join(format!("{safe_name}.png"));
    if output.exists() {
        return Some(output);
    }
    let icon = icns_path(app)?;
    let converted = Command::new("sips")
        .args([
            "-s",
            "format",
            "png",
            "-Z",
            "128",
            icon.to_str()?,
            "--out",
            output.to_str()?,
        ])
        .output()
        .ok()
        .is_some_and(|result| result.status.success());
    converted
        .then(|| output.clone())
        .filter(|path| path.exists())
}

#[cfg(target_os = "macos")]
fn icns_path(app: &Path) -> Option<PathBuf> {
    let resources = app.join("Contents/Resources");
    let plist = app.join("Contents/Info.plist");

    if let Ok(output) = Command::new("/usr/libexec/PlistBuddy")
        .args([
            "-c",
            "Print :CFBundleIconFile",
            plist.to_string_lossy().as_ref(),
        ])
        .output()
    {
        let name = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if !name.is_empty() {
            let mut path = resources.join(name);
            if path.extension().is_none() {
                path.set_extension("icns");
            }
            if path.exists() {
                return Some(path);
            }
        }
    }

    std::fs::read_dir(&resources)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("icns"))
        .max_by_key(|path| path.metadata().map(|metadata| metadata.len()).unwrap_or(0))
}
