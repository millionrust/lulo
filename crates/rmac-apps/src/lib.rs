//! Cross-platform installed-application catalog and launcher.

#![cfg_attr(target_os = "macos", allow(dead_code))]

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Application {
    pub id: String,
    pub name: String,
    pub source: PathBuf,
    pub icon: Option<PathBuf>,
    pub categories: Vec<String>,
    pub launch: LaunchSpec,
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

pub fn reveal(application: &Application) -> io::Result<Child> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg("-R")
            .arg(&application.source)
            .spawn()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Command::new("xdg-open")
            .arg(application.source.parent().unwrap_or(&application.source))
            .spawn()
    }
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
    let directories = [
        PathBuf::from("/Applications"),
        PathBuf::from("/Applications/Utilities"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Applications/Utilities"),
    ];
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
                source: path.clone(),
                icon: None,
                categories: Vec::new(),
                launch: LaunchSpec::OpenPath(path),
            });
        }
    }
    sort_applications(&mut applications);
    Ok(applications)
}

#[derive(Clone)]
struct Environment {
    home: Option<PathBuf>,
    data_home: Option<PathBuf>,
    data_dirs: Vec<PathBuf>,
    desktops: Vec<String>,
    locale: String,
    path: Vec<PathBuf>,
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
            .collect();
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
            desktops,
            locale,
            path,
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
    let exec = values.get("Exec")?;
    let icon_name = values.get("Icon").map(String::as_str);
    let (program, args) = expand_exec(exec, &name, icon_name, path)?;
    let icon = icon_name.and_then(|icon| resolve_icon(icon, environment));
    let categories = split_list(values.get("Categories"));
    Some(Application {
        id: id.to_string(),
        name,
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
    })
}

fn desktop_group(contents: &str) -> HashMap<String, String> {
    let mut values = HashMap::new();
    let mut active = false;
    for raw in contents.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            active = line == "[Desktop Entry]";
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
    value
        .into_iter()
        .flat_map(|value| value.split(';'))
        .filter(|value| !value.is_empty())
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
    let mut bases = Vec::new();
    if let Some(home) = &environment.home {
        bases.push(home.join(".icons"));
    }
    bases.extend(environment.data_home.iter().map(|path| path.join("icons")));
    bases.extend(environment.data_dirs.iter().map(|path| path.join("icons")));
    let filenames = [format!("{icon}.png"), format!("{icon}.svg")];
    for base in &bases {
        for theme in ["hicolor", "Adwaita"] {
            for directory in ["128x128/apps", "64x64/apps", "48x48/apps", "scalable/apps"] {
                for filename in &filenames {
                    let candidate = base.join(theme).join(directory).join(filename);
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }
        }
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
            desktops: vec!["niri".into()],
            locale: "en_GB.UTF-8".into(),
            path: vec![PathBuf::from("/usr/bin")],
        }
    }

    #[test]
    fn parses_localized_visible_application_and_exec_codes() {
        let entry = parse_desktop_entry(
            "demo.desktop",
            Path::new("/apps/demo.desktop"),
            "[Desktop Entry]\nType=Application\nName=Demo\nName[en_GB]=Demonstration\nExec=demo --title %c %% %f\nIcon=demo\nCategories=Development;Utility;\nOnlyShowIn=niri;\n",
            &environment(),
        )
        .unwrap();

        assert_eq!(entry.name, "Demonstration");
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
            desktops: vec!["niri".into()],
            locale: "C".into(),
            path: vec![PathBuf::from("/bin")],
        };

        let applications = discover_linux(&environment).unwrap();

        assert_eq!(applications.len(), 1);
        assert_eq!(applications[0].name, "Other");
        std::fs::remove_dir_all(root).unwrap();
    }
}
