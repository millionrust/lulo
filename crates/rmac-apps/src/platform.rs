//! Platform application discovery and desktop-entry parsing.

use super::*;

pub(super) const MAX_DESKTOP_ACTIONS: usize = 32;
pub(super) const MAX_ACTION_ID_BYTES: usize = 255;
pub(super) const MAX_ACTION_NAME_BYTES: usize = 512;
pub(super) const MAX_GENERIC_NAME_BYTES: usize = 512;
pub(super) const MAX_SEARCH_KEYWORDS: usize = 64;
pub(super) const MAX_KEYWORD_BYTES: usize = 256;
pub(super) const MAX_MIME_TYPES: usize = 256;
pub(super) const MAX_MIME_TYPE_BYTES: usize = 255;
pub(super) const MAX_DESKTOP_ID_BYTES: usize = 512;
pub(super) const MAX_ASSOCIATION_OUTPUT_BYTES: usize = 4 * 1024;
#[cfg(target_os = "linux")]
pub(super) const ASSOCIATION_COMMAND_TIMEOUT: Duration = Duration::from_secs(8);
#[cfg(target_os = "linux")]
pub(super) const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[cfg(target_os = "linux")]
struct BoundedCommandOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

#[cfg(target_os = "macos")]
pub(super) fn discover_macos() -> io::Result<Vec<Application>> {
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
                mime_types: Vec::new(),
                launch: LaunchSpec::OpenPath(path),
                actions: Vec::new(),
            });
        }
    }
    sort_applications(&mut applications);
    Ok(applications)
}

#[derive(Clone)]
pub(super) struct Environment {
    pub(super) home: Option<PathBuf>,
    pub(super) data_home: Option<PathBuf>,
    pub(super) data_dirs: Vec<PathBuf>,
    pub(super) icon_theme: Option<String>,
    pub(super) desktops: Vec<String>,
    pub(super) locale: String,
    pub(super) path: Vec<PathBuf>,
    pub(super) theme_cache: RefCell<HashMap<String, Option<IconTheme>>>,
}

impl Environment {
    pub(super) fn current() -> Self {
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

    pub(super) fn application_dirs(&self) -> Vec<PathBuf> {
        self.data_home
            .iter()
            .chain(self.data_dirs.iter())
            .map(|directory| directory.join("applications"))
            .collect()
    }
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn discover_linux(environment: &Environment) -> io::Result<Vec<Application>> {
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

pub(super) fn collect_desktop_files(
    root: &Path,
    directory: &Path,
    out: &mut Vec<(String, PathBuf)>,
) {
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

pub(super) fn parse_desktop_entry(
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
    let mime_types = bounded_mime_types(values.get("MimeType"));
    let actions = desktop_actions(contents, &values, &name, path, environment);
    Some(Application {
        id: id.to_string(),
        name,
        generic_name,
        keywords,
        source: path.to_path_buf(),
        icon,
        categories,
        mime_types,
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

pub(super) fn desktop_actions(
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

pub(super) fn valid_action_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ACTION_ID_BYTES
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

pub(super) fn desktop_group(contents: &str) -> HashMap<String, String> {
    desktop_group_named(contents, "Desktop Entry")
}

pub(super) fn desktop_group_named(contents: &str, group: &str) -> HashMap<String, String> {
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

pub(super) fn bool_value(value: Option<&String>) -> bool {
    value.is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

pub(super) fn split_list(value: Option<&String>) -> Vec<String> {
    value.map_or_else(Vec::new, |value| split_list_value(value))
}

pub(super) fn split_list_value(value: &str) -> Vec<String> {
    value
        .split(';')
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

pub(super) fn bounded_mime_types(value: Option<&String>) -> Vec<String> {
    let mut seen = HashSet::new();
    split_list(value)
        .into_iter()
        .filter(|mime_type| valid_mime_type(mime_type) && seen.insert(mime_type.clone()))
        .take(MAX_MIME_TYPES)
        .collect()
}

pub(super) fn valid_mime_type(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_MIME_TYPE_BYTES || !value.is_ascii() {
        return false;
    }
    let Some((kind, subtype)) = value.split_once('/') else {
        return false;
    };
    !kind.is_empty()
        && !subtype.is_empty()
        && !subtype.contains('/')
        && kind.bytes().all(valid_mime_token_byte)
        && subtype.bytes().all(valid_mime_token_byte)
}

pub(super) fn valid_mime_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#' | b'$' | b'&' | b'^' | b'_' | b'.' | b'+' | b'-'
        )
}

pub(super) fn matching_file_handlers(
    catalog: Vec<Application>,
    mime_type: &str,
    default_application_id: Option<&str>,
) -> Vec<Application> {
    let mut handlers = catalog
        .into_iter()
        .filter(|application| {
            application
                .mime_types
                .iter()
                .any(|candidate| candidate == mime_type)
        })
        .collect::<Vec<_>>();
    handlers.sort_by(|left, right| {
        let left_default = default_application_id == Some(left.id.as_str());
        let right_default = default_application_id == Some(right.id.as_str());
        right_default.cmp(&left_default).then_with(|| {
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        })
    });
    handlers
}

#[cfg(target_os = "linux")]
pub(super) fn query_file_mime_type(path: &Path) -> io::Result<String> {
    if !path.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Open With requires a regular file",
        ));
    }
    let output = run_xdg_mime(&[
        std::ffi::OsStr::new("query"),
        std::ffi::OsStr::new("filetype"),
        path.as_os_str(),
    ])?;
    parse_mime_output(&output)
}

#[cfg(target_os = "linux")]
pub(super) fn query_default_application(mime_type: &str) -> io::Result<Option<String>> {
    if !valid_mime_type(mime_type) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid MIME type",
        ));
    }
    let output = run_xdg_mime(&[
        std::ffi::OsStr::new("query"),
        std::ffi::OsStr::new("default"),
        std::ffi::OsStr::new(mime_type),
    ])?;
    parse_default_application_output(&output)
}

#[cfg(target_os = "linux")]
pub(super) fn run_xdg_mime(arguments: &[&std::ffi::OsStr]) -> io::Result<Vec<u8>> {
    let mut command = Command::new("xdg-mime");
    command.args(arguments);
    let output = bounded_command_output(&mut command).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("could not run the XDG MIME authority: {error}"),
        )
    })?;
    if !output.status.success() {
        return Err(command_failure(
            "query the XDG MIME authority",
            &output.stderr,
        ));
    }
    Ok(output.stdout)
}

pub(super) fn parse_mime_output(output: &[u8]) -> io::Result<String> {
    let value = parse_one_line(output, "MIME type")?;
    if !valid_mime_type(&value) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the XDG MIME authority returned an invalid MIME type",
        ));
    }
    Ok(value)
}

pub(super) fn parse_default_application_output(output: &[u8]) -> io::Result<Option<String>> {
    if output.iter().all(u8::is_ascii_whitespace) {
        return Ok(None);
    }
    let value = parse_one_line(output, "default application")?;
    if value.len() > MAX_DESKTOP_ID_BYTES
        || !value.ends_with(".desktop")
        || value.bytes().any(|byte| {
            byte.is_ascii_control()
                || byte.is_ascii_whitespace()
                || matches!(byte, b'/' | b'\\' | b';')
        })
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the XDG MIME authority returned an invalid desktop application ID",
        ));
    }
    Ok(Some(value))
}

pub(super) fn parse_one_line(output: &[u8], label: &str) -> io::Result<String> {
    if output.len() > MAX_ASSOCIATION_OUTPUT_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("the {label} response was too large"),
        ));
    }
    let text = std::str::from_utf8(output).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("the {label} response was not UTF-8"),
        )
    })?;
    let value = text.trim();
    if value.is_empty() || value.lines().count() != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("the {label} response was empty or ambiguous"),
        ));
    }
    Ok(value.to_string())
}

#[cfg(target_os = "linux")]
pub(super) fn run_command_success(command: &mut Command, operation: &str) -> io::Result<()> {
    let output = bounded_command_output(command)
        .map_err(|error| io::Error::new(error.kind(), format!("could not {operation}: {error}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_failure(operation, &output.stderr))
    }
}

#[cfg(target_os = "linux")]
pub(super) fn command_failure(operation: &str, stderr: &[u8]) -> io::Error {
    let detail = bounded_command_detail(stderr);
    io::Error::other(if detail.is_empty() {
        format!("could not {operation}: the command failed")
    } else {
        format!("could not {operation}: {detail}")
    })
}

#[cfg(target_os = "linux")]
pub(super) fn bounded_command_detail(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let mut detail = String::new();
    let mut truncated = false;
    for (index, character) in text.trim().chars().enumerate() {
        if index == 512 {
            truncated = true;
            break;
        }
        detail.push(if character.is_control() {
            ' '
        } else {
            character
        });
    }
    if truncated {
        detail.push('…');
    }
    detail
}

#[cfg(target_os = "linux")]
pub(super) fn bounded_command_output(command: &mut Command) -> io::Result<BoundedCommandOutput> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("missing command stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("missing command stderr"))?;
    let stdout_reader = std::thread::spawn(move || drain_bounded(stdout));
    let stderr_reader = std::thread::spawn(move || drain_bounded(stderr));
    let deadline = Instant::now() + ASSOCIATION_COMMAND_TIMEOUT;
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None if Instant::now() < deadline => std::thread::sleep(PROCESS_POLL_INTERVAL),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the command did not finish before the bounded deadline",
                ));
            }
        }
    };
    let (stdout, stdout_excessive) = stdout_reader
        .join()
        .map_err(|_| io::Error::other("command stdout reader failed"))??;
    let (stderr, stderr_excessive) = stderr_reader
        .join()
        .map_err(|_| io::Error::other("command stderr reader failed"))??;
    if stdout_excessive || stderr_excessive {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the command returned excessive output",
        ));
    }
    Ok(BoundedCommandOutput {
        status,
        stdout,
        stderr,
    })
}

#[cfg(target_os = "linux")]
pub(super) fn drain_bounded(reader: impl io::Read) -> io::Result<(Vec<u8>, bool)> {
    let mut bytes = Vec::with_capacity(MAX_ASSOCIATION_OUTPUT_BYTES);
    reader
        .take(MAX_ASSOCIATION_OUTPUT_BYTES.saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;
    let excessive = bytes.len() > MAX_ASSOCIATION_OUTPUT_BYTES;
    bytes.truncate(MAX_ASSOCIATION_OUTPUT_BYTES);
    Ok((bytes, excessive))
}

pub(super) fn bounded_keywords(value: &str) -> Vec<String> {
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

pub(super) fn desktop_visible(values: &HashMap<String, String>, desktops: &[String]) -> bool {
    let only = split_list(values.get("OnlyShowIn"));
    let excluded = split_list(values.get("NotShowIn"));
    !desktops.iter().any(|desktop| excluded.contains(desktop))
        && (only.is_empty() || desktops.iter().any(|desktop| only.contains(desktop)))
}

pub(super) fn localized_value<'a>(
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

pub(super) fn executable_exists(program: &str, path: &[PathBuf]) -> bool {
    let candidate = Path::new(program);
    if candidate.is_absolute() {
        return is_executable(candidate);
    }
    path.iter()
        .any(|directory| is_executable(&directory.join(candidate)))
}

pub(super) fn is_executable(path: &Path) -> bool {
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

pub(super) fn expand_exec(
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

pub(super) fn tokenize_exec(value: &str) -> Option<Vec<String>> {
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
