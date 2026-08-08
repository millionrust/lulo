//! Cross-platform App Drawer application category classification.

use super::*;

/// Resolve every macOS application's category concurrently while preserving
/// catalog order.
#[cfg(target_os = "macos")]
pub(super) fn parallel_categorize(pairs: &[(String, PathBuf)]) -> Vec<Category> {
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
pub(super) fn categorize_desktop(categories: &[String]) -> Category {
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
