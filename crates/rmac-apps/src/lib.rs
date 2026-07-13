//! Cross-platform installed-application catalog and launcher.

#![cfg_attr(target_os = "macos", allow(dead_code))]

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Application {
    pub id: String,
    pub name: String,
    pub generic_name: Option<String>,
    pub keywords: Vec<String>,
    pub source: PathBuf,
    pub icon: Option<PathBuf>,
    pub categories: Vec<String>,
    pub launch: LaunchSpec,
    /// Additional launcher actions declared by the desktop entry, in the
    /// author's `Actions=` order. macOS catalog entries currently omit these.
    pub actions: Vec<DesktopAction>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopAction {
    pub id: String,
    pub name: String,
    pub icon: Option<PathBuf>,
    pub launch: LaunchSpec,
}

const MAX_DESKTOP_ACTIONS: usize = 32;
const MAX_ACTION_ID_BYTES: usize = 255;
const MAX_ACTION_NAME_BYTES: usize = 512;
const MAX_GENERIC_NAME_BYTES: usize = 512;
const MAX_SEARCH_KEYWORDS: usize = 64;
const MAX_KEYWORD_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum ApplicationSource {
    Flatpak,
    Snap,
    AppImage,
    UserDesktopEntry,
    SystemDesktopEntry,
    OtherDesktopEntry,
    MacApplication,
}

impl ApplicationSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Flatpak => "Flatpak",
            Self::Snap => "Snap",
            Self::AppImage => "AppImage",
            Self::UserDesktopEntry => "User desktop entries",
            Self::SystemDesktopEntry => "System desktop entries",
            Self::OtherDesktopEntry => "Other desktop entries",
            Self::MacApplication => "macOS applications",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceInventory {
    pub flatpak: usize,
    pub snap: usize,
    pub appimage: usize,
    pub user_desktop_entries: usize,
    pub system_desktop_entries: usize,
    pub other_desktop_entries: usize,
    pub mac_applications: usize,
}

impl SourceInventory {
    pub fn total(&self) -> usize {
        self.flatpak
            + self.snap
            + self.appimage
            + self.user_desktop_entries
            + self.system_desktop_entries
            + self.other_desktop_entries
            + self.mac_applications
    }
}

impl Application {
    /// Lower-cased, display-safe metadata used by application pickers. Source
    /// paths and command lines are deliberately excluded.
    pub fn searchable_text(&self) -> String {
        std::iter::once(self.name.to_lowercase())
            .chain(self.generic_name.iter().map(|value| value.to_lowercase()))
            .chain(self.keywords.iter().map(|value| value.to_lowercase()))
            .chain(self.categories.iter().map(|value| value.to_lowercase()))
            .chain(self.actions.iter().map(|action| action.name.to_lowercase()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn source_kind(&self) -> ApplicationSource {
        let source = self.source.to_string_lossy();
        if source.contains("/flatpak/exports/share/applications/") {
            ApplicationSource::Flatpak
        } else if source.contains("/snapd/desktop/applications/") {
            ApplicationSource::Snap
        } else if self.id.starts_with("appimagekit_")
            || matches!(&self.launch, LaunchSpec::Command { program, .. } if program.to_ascii_lowercase().ends_with(".appimage"))
        {
            ApplicationSource::AppImage
        } else if self.source.extension().and_then(|value| value.to_str()) == Some("app") {
            ApplicationSource::MacApplication
        } else if source.contains("/.local/share/applications/") {
            ApplicationSource::UserDesktopEntry
        } else if source.starts_with("/usr/share/applications/")
            || source.starts_with("/usr/local/share/applications/")
        {
            ApplicationSource::SystemDesktopEntry
        } else {
            ApplicationSource::OtherDesktopEntry
        }
    }
}

pub fn source_inventory(applications: &[Application]) -> SourceInventory {
    let mut inventory = SourceInventory::default();
    for application in applications {
        match application.source_kind() {
            ApplicationSource::Flatpak => inventory.flatpak += 1,
            ApplicationSource::Snap => inventory.snap += 1,
            ApplicationSource::AppImage => inventory.appimage += 1,
            ApplicationSource::UserDesktopEntry => inventory.user_desktop_entries += 1,
            ApplicationSource::SystemDesktopEntry => inventory.system_desktop_entries += 1,
            ApplicationSource::OtherDesktopEntry => inventory.other_desktop_entries += 1,
            ApplicationSource::MacApplication => inventory.mac_applications += 1,
        }
    }
    inventory
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchSpec {
    OpenPath(PathBuf),
    Command {
        program: String,
        args: Vec<String>,
        working_dir: Option<PathBuf>,
        terminal: bool,
    },
}

/// Keeps native watches for installed-application directories alive.
pub struct CatalogWatcher {
    _watcher: RecommendedWatcher,
}

/// Notify `on_change` when an application entry may have been added, removed,
/// or edited. Access-only events are ignored, and directories that are absent
/// are skipped so a minimal installation can still open the catalog.
pub fn watch_catalog(on_change: impl Fn() + Send + 'static) -> io::Result<CatalogWatcher> {
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
        if result.as_ref().is_ok_and(catalog_event_is_relevant) {
            on_change();
        }
    })
    .map_err(io::Error::other)?;

    let mut existing = 0;
    let mut watched = 0;
    let mut last_error = None;
    for directory in catalog_directories()
        .into_iter()
        .filter(|directory| directory.is_dir())
    {
        existing += 1;
        // One unavailable system directory must not disable watches for every
        // other XDG data directory.
        match watcher.watch(&directory, RecursiveMode::Recursive) {
            Ok(()) => watched += 1,
            Err(error) => last_error = Some(error),
        }
    }
    if existing > 0 && watched == 0 {
        return Err(io::Error::other(
            last_error.expect("an existing directory produced a watch result"),
        ));
    }

    Ok(CatalogWatcher { _watcher: watcher })
}

pub fn discover() -> io::Result<Vec<Application>> {
    #[cfg(target_os = "macos")]
    {
        discover_macos()
    }
    #[cfg(not(target_os = "macos"))]
    {
        discover_linux(&Environment::current())
    }
}

/// Resolve a trusted desktop-entry application ID without fuzzy matching.
///
/// Portal notification IDs normally omit the `.desktop` suffix while the XDG
/// catalog keeps it. Exact IDs always win; the suffix alias is the only
/// fallback so unrelated applications can never be mislabeled by basename or
/// display-name similarity.
pub fn find_desktop_entry<'a>(
    catalog: &'a [Application],
    application_id: &str,
) -> Option<&'a Application> {
    catalog
        .iter()
        .find(|application| application.id == application_id)
        .or_else(|| {
            if application_id.ends_with(".desktop") {
                return None;
            }
            let desktop_id = format!("{application_id}.desktop");
            catalog
                .iter()
                .find(|application| application.id == desktop_id)
        })
}

pub fn launch(spec: &LaunchSpec) -> io::Result<Child> {
    match spec {
        LaunchSpec::OpenPath(path) => Command::new("open").arg(path).spawn(),
        LaunchSpec::Command {
            program,
            args,
            working_dir,
            terminal,
        } => {
            let mut command = if *terminal {
                terminal_command(program, args)
            } else {
                let mut command = Command::new(program);
                command.args(args);
                command
            };
            if let Some(directory) = working_dir {
                command.current_dir(directory);
            }
            command.spawn()
        }
    }
}

pub async fn reveal(application: &Application) -> Result<(), rmac_portal::Error> {
    rmac_portal::show_item(&application.source).await
}

fn terminal_command(program: &str, args: &[String]) -> Command {
    if let Some(terminal) = std::env::var_os("TERMINAL").filter(|value| !value.is_empty()) {
        let mut command = Command::new(terminal);
        command.arg("-e").arg(program).args(args);
        return command;
    }
    let mut command = Command::new("x-terminal-emulator");
    command.arg("-e").arg(program).args(args);
    command
}

#[cfg(target_os = "macos")]
fn discover_macos() -> io::Result<Vec<Application>> {
    let directories = catalog_directories();
    let mut seen = HashSet::new();
    let mut applications = Vec::new();
    for directory in directories {
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("app") {
                continue;
            }
            let name = path
                .file_stem()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name.is_empty() || !seen.insert(name.clone()) {
                continue;
            }
            applications.push(Application {
                id: name.clone(),
                name,
                generic_name: None,
                keywords: Vec::new(),
                source: path.clone(),
                icon: None,
                categories: Vec::new(),
                launch: LaunchSpec::OpenPath(path),
                actions: Vec::new(),
            });
        }
    }
    sort_applications(&mut applications);
    Ok(applications)
}

#[cfg(target_os = "macos")]
fn catalog_directories() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/Applications/Utilities"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Applications/Utilities"),
    ]
}

#[cfg(not(target_os = "macos"))]
fn catalog_directories() -> Vec<PathBuf> {
    Environment::current().application_dirs()
}

fn catalog_event_is_relevant(event: &Event) -> bool {
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    // Some backends report an empty path list for an overflow/rescan event.
    event.paths.is_empty()
        || event
            .paths
            .iter()
            .any(|path| catalog_path_is_relevant(path))
}

#[cfg(target_os = "macos")]
fn catalog_path_is_relevant(path: &Path) -> bool {
    path.ancestors().any(|ancestor| {
        ancestor
            .extension()
            .and_then(|extension| extension.to_str())
            == Some("app")
    })
}

#[cfg(not(target_os = "macos"))]
fn catalog_path_is_relevant(path: &Path) -> bool {
    path.extension().and_then(|extension| extension.to_str()) == Some("desktop")
        || path.file_name().and_then(|name| name.to_str()) == Some("applications")
}

#[derive(Clone)]
struct Environment {
    home: Option<PathBuf>,
    data_home: Option<PathBuf>,
    data_dirs: Vec<PathBuf>,
    icon_theme: Option<String>,
    desktops: Vec<String>,
    locale: String,
    path: Vec<PathBuf>,
    theme_cache: RefCell<HashMap<String, Option<IconTheme>>>,
}

impl Environment {
    fn current() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| home.as_ref().map(|home| home.join(".local/share")));
        let data_dirs = std::env::var_os("XDG_DATA_DIRS")
            .filter(|value| !value.is_empty())
            .map(|value| std::env::split_paths(&value).collect())
            .unwrap_or_else(|| {
                vec![
                    PathBuf::from("/usr/local/share"),
                    PathBuf::from("/usr/share"),
                ]
            });
        let desktops = std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .split(':')
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| home.as_ref().map(|home| home.join(".config")));
        let prefer_kde = desktops
            .iter()
            .any(|desktop| desktop.eq_ignore_ascii_case("KDE"));
        let prefer_gnome = desktops.iter().any(|desktop| {
            ["GNOME", "Unity", "ubuntu"]
                .iter()
                .any(|name| desktop.eq_ignore_ascii_case(name))
        });
        let icon_theme = active_icon_theme(config_home.as_deref(), prefer_kde, prefer_gnome);
        let locale = std::env::var("LC_MESSAGES")
            .or_else(|_| std::env::var("LANG"))
            .unwrap_or_default();
        let path = std::env::var_os("PATH")
            .map(|value| std::env::split_paths(&value).collect())
            .unwrap_or_default();
        Self {
            home,
            data_home,
            data_dirs,
            icon_theme,
            desktops,
            locale,
            path,
            theme_cache: RefCell::new(HashMap::new()),
        }
    }

    fn application_dirs(&self) -> Vec<PathBuf> {
        self.data_home
            .iter()
            .chain(self.data_dirs.iter())
            .map(|directory| directory.join("applications"))
            .collect()
    }
}

#[cfg(any(not(target_os = "macos"), test))]
fn discover_linux(environment: &Environment) -> io::Result<Vec<Application>> {
    let mut seen = HashSet::new();
    let mut applications = Vec::new();
    for directory in environment.application_dirs() {
        let mut files = Vec::new();
        collect_desktop_files(&directory, &directory, &mut files);
        files.sort();
        for (id, path) in files {
            if !seen.insert(id.clone()) {
                continue;
            }
            let Ok(contents) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Some(application) = parse_desktop_entry(&id, &path, &contents, environment) {
                applications.push(application);
            }
        }
    }
    sort_applications(&mut applications);
    Ok(applications)
}

fn collect_desktop_files(root: &Path, directory: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_desktop_files(root, &path, out);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("desktop") {
            if let Ok(relative) = path.strip_prefix(root) {
                let id = relative.to_string_lossy().replace('/', "-");
                out.push((id, path));
            }
        }
    }
}

fn parse_desktop_entry(
    id: &str,
    path: &Path,
    contents: &str,
    environment: &Environment,
) -> Option<Application> {
    let values = desktop_group(contents);
    if values.get("Type").map(String::as_str) != Some("Application")
        || bool_value(values.get("Hidden"))
        || bool_value(values.get("NoDisplay"))
        || !desktop_visible(&values, &environment.desktops)
    {
        return None;
    }
    if let Some(try_exec) = values.get("TryExec") {
        if !executable_exists(try_exec, &environment.path) {
            return None;
        }
    }
    let name = localized_value(&values, "Name", &environment.locale)?.to_string();
    let generic_name = localized_value(&values, "GenericName", &environment.locale)
        .filter(|value| !value.trim().is_empty() && value.len() <= MAX_GENERIC_NAME_BYTES)
        .map(str::to_string);
    let keywords = localized_value(&values, "Keywords", &environment.locale)
        .map(bounded_keywords)
        .unwrap_or_default();
    let exec = values.get("Exec")?;
    let icon_name = values.get("Icon").map(String::as_str);
    let (program, args) = expand_exec(exec, &name, icon_name, path)?;
    let icon = icon_name.and_then(|icon| resolve_icon(icon, environment));
    let categories = split_list(values.get("Categories"));
    let actions = desktop_actions(contents, &values, &name, path, environment);
    Some(Application {
        id: id.to_string(),
        name,
        generic_name,
        keywords,
        source: path.to_path_buf(),
        icon,
        categories,
        launch: LaunchSpec::Command {
            program,
            args,
            working_dir: values
                .get("Path")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
            terminal: bool_value(values.get("Terminal")),
        },
        actions,
    })
}

fn desktop_actions(
    contents: &str,
    entry: &HashMap<String, String>,
    application_name: &str,
    source: &Path,
    environment: &Environment,
) -> Vec<DesktopAction> {
    let working_dir = entry
        .get("Path")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let terminal = bool_value(entry.get("Terminal"));
    let mut seen = HashSet::new();
    split_list(entry.get("Actions"))
        .into_iter()
        .filter(|id| valid_action_id(id) && seen.insert(id.clone()))
        .filter_map(|id| {
            let values = desktop_group_named(contents, &format!("Desktop Action {id}"));
            let name = localized_value(&values, "Name", &environment.locale)?;
            if name.trim().is_empty() || name.len() > MAX_ACTION_NAME_BYTES {
                return None;
            }
            // rmac does not yet advertise desktop-entry D-Bus activation, so
            // an action without the compatibility Exec key is not actionable.
            let exec = values.get("Exec")?;
            let icon_name = values.get("Icon").map(String::as_str);
            let (program, args) = expand_exec(exec, application_name, icon_name, source)?;
            Some(DesktopAction {
                id,
                name: name.to_string(),
                icon: icon_name.and_then(|icon| resolve_icon(icon, environment)),
                launch: LaunchSpec::Command {
                    program,
                    args,
                    working_dir: working_dir.clone(),
                    terminal,
                },
            })
        })
        .take(MAX_DESKTOP_ACTIONS)
        .collect()
}

fn valid_action_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ACTION_ID_BYTES
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn desktop_group(contents: &str) -> HashMap<String, String> {
    desktop_group_named(contents, "Desktop Entry")
}

fn desktop_group_named(contents: &str, group: &str) -> HashMap<String, String> {
    let mut values = HashMap::new();
    let mut active = false;
    for raw in contents.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            active = &line[1..line.len() - 1] == group;
            continue;
        }
        if !active || line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            values
                .entry(key.to_string())
                .or_insert_with(|| value.to_string());
        }
    }
    values
}

fn bool_value(value: Option<&String>) -> bool {
    value.is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

fn split_list(value: Option<&String>) -> Vec<String> {
    value.map_or_else(Vec::new, |value| split_list_value(value))
}

fn split_list_value(value: &str) -> Vec<String> {
    value
        .split(';')
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn bounded_keywords(value: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    value
        .split(';')
        .map(str::trim)
        .filter(|keyword| {
            !keyword.is_empty()
                && keyword.len() <= MAX_KEYWORD_BYTES
                && seen.insert(keyword.to_lowercase())
        })
        .take(MAX_SEARCH_KEYWORDS)
        .map(str::to_string)
        .collect()
}

fn desktop_visible(values: &HashMap<String, String>, desktops: &[String]) -> bool {
    let only = split_list(values.get("OnlyShowIn"));
    let excluded = split_list(values.get("NotShowIn"));
    !desktops.iter().any(|desktop| excluded.contains(desktop))
        && (only.is_empty() || desktops.iter().any(|desktop| only.contains(desktop)))
}

fn localized_value<'a>(
    values: &'a HashMap<String, String>,
    key: &str,
    locale: &str,
) -> Option<&'a str> {
    let locale = locale.split('.').next().unwrap_or(locale);
    let language = locale.split(['_', '@']).next().unwrap_or(locale);
    [
        format!("{key}[{locale}]"),
        format!("{key}[{language}]"),
        key.to_string(),
    ]
    .into_iter()
    .find_map(|candidate| values.get(&candidate).map(String::as_str))
}

fn executable_exists(program: &str, path: &[PathBuf]) -> bool {
    let candidate = Path::new(program);
    if candidate.is_absolute() {
        return is_executable(candidate);
    }
    path.iter()
        .any(|directory| is_executable(&directory.join(candidate)))
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

fn expand_exec(
    exec: &str,
    name: &str,
    icon: Option<&str>,
    source: &Path,
) -> Option<(String, Vec<String>)> {
    if exec.len() > 32 * 1024 {
        return None;
    }
    let tokens = tokenize_exec(exec)?;
    let mut expanded = Vec::new();
    for token in tokens {
        match token.as_str() {
            "%f" | "%F" | "%u" | "%U" | "%d" | "%D" | "%n" | "%N" | "%v" | "%m" => continue,
            "%i" => {
                if let Some(icon) = icon {
                    expanded.push("--icon".to_string());
                    expanded.push(icon.to_string());
                }
            }
            _ => {
                let value = token
                    .replace("%%", "\0")
                    .replace("%c", name)
                    .replace("%k", &source.to_string_lossy());
                if value.contains('%') {
                    return None;
                }
                expanded.push(value.replace('\0', "%"));
            }
        }
    }
    let program = expanded.first()?.clone();
    Some((program, expanded.into_iter().skip(1).collect()))
}

fn tokenize_exec(value: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            token.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '"' {
            quoted = !quoted;
        } else if character.is_whitespace() && !quoted {
            if !token.is_empty() {
                tokens.push(std::mem::take(&mut token));
            }
        } else {
            token.push(character);
        }
    }
    if quoted || escaped {
        return None;
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    Some(tokens)
}

fn resolve_icon(icon: &str, environment: &Environment) -> Option<PathBuf> {
    let icon_path = Path::new(icon);
    if icon_path.is_absolute() && icon_path.is_file() {
        return Some(icon_path.to_path_buf());
    }
    let bases = icon_base_directories(environment);
    let icon = normalized_icon_name(icon)?;
    let theme = environment.icon_theme.as_deref().unwrap_or("Adwaita");
    for theme in icon_theme_order(theme, &bases, &environment.theme_cache) {
        let Some(metadata) = load_icon_theme_cached(&theme, &bases, &environment.theme_cache)
        else {
            continue;
        };
        if let Some(path) = lookup_icon_in_theme(icon, 64, 1, &theme, &bases, &metadata) {
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

fn active_icon_theme(
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

fn gsettings_icon_theme() -> Option<String> {
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

fn configured_icon_theme(config_home: &Path, prefer_kde: bool) -> Option<String> {
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

fn configured_value(path: &Path, group: &str, key: &str) -> Option<String> {
    let contents = std::fs::read_to_string(path).ok()?;
    ini_value(&contents, group, key).and_then(valid_theme_name)
}

fn valid_theme_name(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.is_ascii()
        && !value.contains([',', '/', '\\'])
        && !value.chars().any(char::is_whitespace))
    .then(|| value.to_string())
}

fn ini_value<'a>(contents: &'a str, group: &str, key: &str) -> Option<&'a str> {
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

fn icon_base_directories(environment: &Environment) -> Vec<PathBuf> {
    let mut bases = Vec::new();
    if let Some(home) = &environment.home {
        bases.push(home.join(".icons"));
    }
    bases.extend(environment.data_home.iter().map(|path| path.join("icons")));
    bases.extend(environment.data_dirs.iter().map(|path| path.join("icons")));
    bases
}

fn normalized_icon_name(icon: &str) -> Option<&str> {
    let icon = Path::new(icon)
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| matches!(*extension, "png" | "svg" | "xpm"))
        .and_then(|_| Path::new(icon).file_stem())
        .and_then(|name| name.to_str())
        .unwrap_or(icon);
    (!icon.is_empty() && !icon.contains(['/', '\\'])).then_some(icon)
}

fn icon_filenames(icon: &str) -> [String; 3] {
    [
        format!("{icon}.png"),
        format!("{icon}.svg"),
        format!("{icon}.xpm"),
    ]
}

#[derive(Clone, Copy)]
enum IconDirectoryType {
    Fixed,
    Scalable,
    Threshold,
}

#[derive(Clone)]
struct IconDirectory {
    path: String,
    size: u32,
    scale: u32,
    kind: IconDirectoryType,
    min_size: u32,
    max_size: u32,
    threshold: u32,
}

#[derive(Clone)]
struct IconTheme {
    inherits: Vec<String>,
    directories: Vec<IconDirectory>,
}

fn icon_theme_order(
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

fn load_icon_theme_cached(
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

fn load_icon_theme(theme: &str, bases: &[PathBuf]) -> Option<IconTheme> {
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

fn ini_group<'a>(contents: &'a str, group: &str) -> HashMap<&'a str, &'a str> {
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

fn comma_list(value: Option<&str>) -> Vec<&str> {
    value
        .into_iter()
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect()
}

fn parse_icon_directory(contents: &str, path: &str) -> Option<IconDirectory> {
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

fn lookup_icon_in_theme(
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

fn find_themed_icon(
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

fn directory_matches(directory: &IconDirectory, size: u32, scale: u32) -> bool {
    if directory.scale != scale {
        return false;
    }
    match directory.kind {
        IconDirectoryType::Fixed => directory.size == size,
        IconDirectoryType::Scalable => (directory.min_size..=directory.max_size).contains(&size),
        IconDirectoryType::Threshold => size.abs_diff(directory.size) <= directory.threshold,
    }
}

fn directory_distance(directory: &IconDirectory, size: u32, scale: u32) -> u32 {
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

fn sort_applications(applications: &mut [Application]) {
    applications.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn environment() -> Environment {
        Environment {
            home: Some(PathBuf::from("/home/user")),
            data_home: Some(PathBuf::from("/home/user/.local/share")),
            data_dirs: vec![PathBuf::from("/usr/share")],
            icon_theme: None,
            desktops: vec!["niri".into()],
            locale: "en_GB.UTF-8".into(),
            path: vec![PathBuf::from("/usr/bin")],
            theme_cache: RefCell::new(HashMap::new()),
        }
    }

    fn application(id: &str, name: &str) -> Application {
        Application {
            id: id.into(),
            name: name.into(),
            generic_name: None,
            keywords: Vec::new(),
            source: PathBuf::from(format!("/apps/{id}")),
            icon: None,
            categories: Vec::new(),
            launch: LaunchSpec::OpenPath(PathBuf::from(format!("/apps/{id}"))),
            actions: Vec::new(),
        }
    }

    #[test]
    fn desktop_entry_resolution_is_exact_with_one_portal_suffix_alias() {
        let catalog = vec![
            application("org.example.Chat.desktop", "Chat"),
            application("org.example.Chat.Beta.desktop", "Chat Beta"),
        ];
        assert_eq!(
            find_desktop_entry(&catalog, "org.example.Chat")
                .map(|application| application.name.as_str()),
            Some("Chat")
        );
        assert_eq!(
            find_desktop_entry(&catalog, "org.example.Chat.desktop")
                .map(|application| application.name.as_str()),
            Some("Chat")
        );
        assert!(find_desktop_entry(&catalog, "Chat").is_none());
        assert!(find_desktop_entry(&catalog, "org.example").is_none());
    }

    #[test]
    fn source_inventory_uses_authoritative_catalog_paths_and_launch_metadata() {
        let mut flatpak = application("org.example.Flatpak.desktop", "Flatpak App");
        flatpak.source = PathBuf::from(
            "/var/lib/flatpak/exports/share/applications/org.example.Flatpak.desktop",
        );
        let mut snap = application("snap-app.desktop", "Snap App");
        snap.source = PathBuf::from("/var/lib/snapd/desktop/applications/snap-app.desktop");
        let mut appimage = application("demo.desktop", "AppImage App");
        appimage.launch = LaunchSpec::Command {
            program: "/home/user/Applications/Demo.AppImage".into(),
            args: Vec::new(),
            working_dir: None,
            terminal: false,
        };
        let mut system = application("native.desktop", "System App");
        system.source = PathBuf::from("/usr/share/applications/native.desktop");

        let inventory = source_inventory(&[flatpak, snap, appimage, system]);
        assert_eq!(inventory.flatpak, 1);
        assert_eq!(inventory.snap, 1);
        assert_eq!(inventory.appimage, 1);
        assert_eq!(inventory.system_desktop_entries, 1);
        assert_eq!(inventory.total(), 4);
    }

    #[test]
    fn parses_localized_visible_application_and_exec_codes() {
        let entry = parse_desktop_entry(
            "demo.desktop",
            Path::new("/apps/demo.desktop"),
            "[Desktop Entry]\nType=Application\nName=Demo\nName[en_GB]=Demonstration\nGenericName=Tool\nGenericName[en_GB]=Developer Tool\nKeywords=code;editor;\nKeywords[en_GB]=develop;build;\nExec=demo --title %c %% %f\nIcon=demo\nCategories=Development;Utility;\nOnlyShowIn=niri;\n",
            &environment(),
        )
        .unwrap();

        assert_eq!(entry.name, "Demonstration");
        assert_eq!(entry.generic_name.as_deref(), Some("Developer Tool"));
        assert_eq!(entry.keywords, ["develop", "build"]);
        let searchable = entry.searchable_text();
        assert!(searchable.contains("demonstration"));
        assert!(searchable.contains("developer tool"));
        assert!(searchable.contains("develop"));
        assert!(searchable.contains("utility"));
        assert_eq!(entry.categories, ["Development", "Utility"]);
        assert_eq!(
            entry.launch,
            LaunchSpec::Command {
                program: "demo".into(),
                args: vec!["--title".into(), "Demonstration".into(), "%".into()],
                working_dir: None,
                terminal: false,
            }
        );
    }

    #[test]
    fn parses_bounded_localized_desktop_actions_in_declared_order() {
        let entry = parse_desktop_entry(
            "demo.desktop",
            Path::new("/apps/demo.desktop"),
            "[Desktop Entry]\nType=Application\nName=Demo\nName[en_GB]=Demonstration\nExec=demo\nPath=/work\nTerminal=true\nActions=New-Window;Duplicate;Missing;New-Window;bad_id;\n\
             [Desktop Action New-Window]\nName=New Window\nName[en_GB]=Fresh Window\nExec=demo --new-window --title %c\n\
             [Desktop Action Duplicate]\nName=Duplicate\nExec=demo --duplicate\n\
             [Desktop Action Missing]\nName=Missing Exec\n\
             [Desktop Action bad_id]\nName=Invalid identifier\nExec=demo --invalid\n",
            &environment(),
        )
        .unwrap();

        assert_eq!(entry.actions.len(), 2);
        assert_eq!(entry.actions[0].id, "New-Window");
        assert_eq!(entry.actions[0].name, "Fresh Window");
        assert_eq!(
            entry.actions[0].launch,
            LaunchSpec::Command {
                program: "demo".into(),
                args: vec![
                    "--new-window".into(),
                    "--title".into(),
                    "Demonstration".into()
                ],
                working_dir: Some("/work".into()),
                terminal: true,
            }
        );
        assert_eq!(entry.actions[1].id, "Duplicate");
    }

    #[test]
    fn desktop_actions_reject_invalid_or_excessive_metadata() {
        assert!(valid_action_id("New-Window"));
        assert!(!valid_action_id("new_window"));
        assert!(!valid_action_id(""));
        assert!(!valid_action_id(&"a".repeat(MAX_ACTION_ID_BYTES + 1)));
        assert!(expand_exec(
            &"x".repeat(32 * 1024 + 1),
            "Demo",
            None,
            Path::new("demo.desktop")
        )
        .is_none());

        let action_ids = (0..40)
            .map(|index| format!("Action{index};"))
            .collect::<String>();
        let action_groups = (0..40)
            .map(|index| {
                format!(
                    "[Desktop Action Action{index}]\nName=Action {index}\nExec=demo --action {index}\n"
                )
            })
            .collect::<String>();
        let contents = format!(
            "[Desktop Entry]\nType=Application\nName=Demo\nExec=demo\nActions={action_ids}\n{action_groups}"
        );
        let entry = parse_desktop_entry(
            "demo.desktop",
            Path::new("demo.desktop"),
            &contents,
            &environment(),
        )
        .unwrap();
        assert_eq!(entry.actions.len(), MAX_DESKTOP_ACTIONS);
        assert_eq!(entry.actions.first().unwrap().id, "Action0");
        assert_eq!(entry.actions.last().unwrap().id, "Action31");

        let keywords = (0..70)
            .map(|index| format!("keyword{index};"))
            .collect::<String>();
        let contents = format!(
            "[Desktop Entry]\nType=Application\nName=Demo\nGenericName={}\nKeywords={keywords}\nExec=demo\n",
            "g".repeat(MAX_GENERIC_NAME_BYTES + 1)
        );
        let entry = parse_desktop_entry(
            "demo.desktop",
            Path::new("demo.desktop"),
            &contents,
            &environment(),
        )
        .unwrap();
        assert!(entry.generic_name.is_none());
        assert_eq!(entry.keywords.len(), MAX_SEARCH_KEYWORDS);
        assert_eq!(entry.keywords.last().unwrap(), "keyword63");
    }

    #[test]
    fn hidden_no_display_and_desktop_exclusions_are_ignored() {
        for extra in [
            "Hidden=true",
            "NoDisplay=true",
            "NotShowIn=niri;",
            "OnlyShowIn=GNOME;",
        ] {
            let contents =
                format!("[Desktop Entry]\nType=Application\nName=Demo\nExec=demo\n{extra}\n");
            assert!(parse_desktop_entry(
                "demo.desktop",
                Path::new("demo.desktop"),
                &contents,
                &environment()
            )
            .is_none());
        }
    }

    #[test]
    fn malformed_exec_and_unknown_field_codes_are_rejected() {
        assert!(tokenize_exec("demo \"unterminated").is_none());
        assert!(expand_exec("demo %Z", "Demo", None, Path::new("demo.desktop")).is_none());
    }

    #[test]
    fn catalog_events_ignore_reads_and_unrelated_files() {
        use notify::event::{AccessKind, AccessMode, CreateKind};

        let read = Event::new(EventKind::Access(AccessKind::Open(AccessMode::Read)))
            .add_path(PathBuf::from("/usr/share/applications/demo.desktop"));
        let unrelated = Event::new(EventKind::Create(CreateKind::File))
            .add_path(PathBuf::from("/usr/share/applications/readme.txt"));
        #[cfg(not(target_os = "macos"))]
        let catalog_entry = Event::new(EventKind::Create(CreateKind::File))
            .add_path(PathBuf::from("/usr/share/applications/demo.desktop"));
        #[cfg(target_os = "macos")]
        let catalog_entry = Event::new(EventKind::Create(CreateKind::File))
            .add_path(PathBuf::from("/Applications/Demo.app/Contents/Info.plist"));

        assert!(!catalog_event_is_relevant(&read));
        assert!(!catalog_event_is_relevant(&unrelated));
        assert!(catalog_event_is_relevant(&catalog_entry));
    }

    #[test]
    fn configured_theme_prefers_gtk4_then_gtk3_and_kde() {
        let root = temporary_directory("theme-config");
        std::fs::create_dir_all(root.join("gtk-3.0")).unwrap();
        std::fs::create_dir_all(root.join("gtk-4.0")).unwrap();
        std::fs::write(root.join("kdeglobals"), "[Icons]\nTheme=Breeze\n").unwrap();
        std::fs::write(
            root.join("gtk-3.0/settings.ini"),
            "[Settings]\ngtk-icon-theme-name=Adwaita\n",
        )
        .unwrap();
        std::fs::write(
            root.join("gtk-4.0/settings.ini"),
            "[Settings]\ngtk-icon-theme-name=Yaru\n",
        )
        .unwrap();

        assert_eq!(configured_icon_theme(&root, false).as_deref(), Some("Yaru"));
        assert_eq!(
            configured_icon_theme(&root, true).as_deref(),
            Some("Breeze")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn icon_metadata_rejects_parent_paths_and_invalid_names() {
        let metadata = "[../apps]\nSize=64\nType=Fixed\n";

        assert!(parse_icon_directory(metadata, "../apps").is_none());
        assert_eq!(normalized_icon_name("demo.svg"), Some("demo"));
        assert_eq!(normalized_icon_name("../demo"), None);
        assert_eq!(valid_theme_name("../../theme"), None);
    }

    #[test]
    fn icon_lookup_honors_theme_inheritance_and_base_precedence() {
        let root = temporary_directory("icon-theme");
        let user = root.join("user");
        let system = root.join("system");
        write_theme(
            &user,
            "Child",
            "Parent",
            "16x16/apps",
            "Size=16\nType=Fixed",
        );
        write_theme(&system, "Parent", "", "64x64/apps", "Size=64\nType=Fixed");
        let child_icon = user.join("icons/Child/16x16/apps/demo.png");
        let system_parent_icon = system.join("icons/Parent/64x64/apps/demo.png");
        std::fs::write(&child_icon, b"child").unwrap();
        std::fs::write(&system_parent_icon, b"system parent").unwrap();
        let mut environment = environment();
        environment.home = None;
        environment.data_home = Some(user.clone());
        environment.data_dirs = vec![system.clone()];
        environment.icon_theme = Some("Child".into());

        // A current-theme icon wins before a closer inherited icon.
        assert_eq!(resolve_icon("demo", &environment), Some(child_icon.clone()));

        std::fs::remove_file(child_icon).unwrap();
        assert_eq!(
            resolve_icon("demo", &environment),
            Some(system_parent_icon.clone())
        );

        // A user extension of the inherited theme overrides its system icon.
        let user_parent_icon = user.join("icons/Parent/64x64/apps/demo.png");
        std::fs::create_dir_all(user_parent_icon.parent().unwrap()).unwrap();
        std::fs::write(&user_parent_icon, b"user parent").unwrap();
        assert_eq!(resolve_icon("demo", &environment), Some(user_parent_icon));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn icon_lookup_uses_hicolor_then_unthemed_fallbacks() {
        let root = temporary_directory("icon-fallback");
        write_theme(&root, "hicolor", "", "48x48/apps", "Size=48\nType=Fixed");
        let themed = root.join("icons/hicolor/48x48/apps/demo.png");
        std::fs::write(&themed, b"themed").unwrap();
        let mut environment = environment();
        environment.home = None;
        environment.data_home = Some(root.clone());
        environment.data_dirs.clear();
        environment.icon_theme = Some("MissingTheme".into());

        assert_eq!(resolve_icon("demo", &environment), Some(themed.clone()));

        std::fs::remove_file(themed).unwrap();
        let unthemed = root.join("icons/demo.svg");
        std::fs::write(&unthemed, b"unthemed").unwrap();
        assert_eq!(resolve_icon("demo", &environment), Some(unthemed));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn icon_directory_types_apply_size_and_scale_metadata() {
        let fixed =
            parse_icon_directory("[fixed]\nSize=64\nScale=2\nType=Fixed\n", "fixed").unwrap();
        let scalable = parse_icon_directory(
            "[scalable]\nSize=64\nType=Scalable\nMinSize=32\nMaxSize=128\n",
            "scalable",
        )
        .unwrap();
        let threshold = parse_icon_directory(
            "[threshold]\nSize=64\nType=Threshold\nThreshold=4\n",
            "threshold",
        )
        .unwrap();

        assert!(directory_matches(&fixed, 64, 2));
        assert!(!directory_matches(&fixed, 64, 1));
        assert!(directory_matches(&scalable, 96, 1));
        assert!(directory_matches(&threshold, 68, 1));
        assert_eq!(directory_distance(&threshold, 72, 1), 4);
    }

    #[test]
    fn user_hidden_entry_suppresses_lower_priority_system_entry() {
        let root = std::env::temp_dir().join(format!(
            "rmac-apps-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let user = root.join("user");
        let system = root.join("system");
        std::fs::create_dir_all(user.join("applications")).unwrap();
        std::fs::create_dir_all(system.join("applications")).unwrap();
        std::fs::write(
            user.join("applications/demo.desktop"),
            "[Desktop Entry]\nType=Application\nName=Demo\nHidden=true\nExec=/bin/sh\n",
        )
        .unwrap();
        std::fs::write(
            system.join("applications/demo.desktop"),
            "[Desktop Entry]\nType=Application\nName=System Demo\nExec=/bin/sh\n",
        )
        .unwrap();
        std::fs::write(
            system.join("applications/other.desktop"),
            "[Desktop Entry]\nType=Application\nName=Other\nTryExec=/bin/sh\nExec=/bin/sh\n",
        )
        .unwrap();
        let environment = Environment {
            home: None,
            data_home: Some(user),
            data_dirs: vec![system],
            icon_theme: None,
            desktops: vec!["niri".into()],
            locale: "C".into(),
            path: vec![PathBuf::from("/bin")],
            theme_cache: RefCell::new(HashMap::new()),
        };

        let applications = discover_linux(&environment).unwrap();

        assert_eq!(applications.len(), 1);
        assert_eq!(applications[0].name, "Other");
        std::fs::remove_dir_all(root).unwrap();
    }

    fn temporary_directory(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rmac-apps-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn write_theme(
        data_directory: &Path,
        theme: &str,
        inherits: &str,
        directory: &str,
        directory_metadata: &str,
    ) {
        let root = data_directory.join("icons").join(theme);
        std::fs::create_dir_all(root.join(directory)).unwrap();
        std::fs::write(
            root.join("index.theme"),
            format!(
                "[Icon Theme]\nName={theme}\nComment=Test\nInherits={inherits}\nDirectories={directory}\n\n[{directory}]\n{directory_metadata}\n"
            ),
        )
        .unwrap();
    }
}
