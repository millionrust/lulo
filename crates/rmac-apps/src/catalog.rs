//! Public catalog discovery, launch, association, and watch authority.

use super::*;

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

/// Resolve the shared-mime-info type, current default, and visible compatible
/// applications for one local file.
///
/// Linux uses the XDG association authority rather than guessing from the file
/// extension. The returned applications come from the same bounded desktop
/// catalog used by the launcher and Dock.
pub fn file_association(path: &Path) -> io::Result<FileAssociation> {
    #[cfg(target_os = "linux")]
    {
        let mime_type = query_file_mime_type(path)?;
        let default_application_id = query_default_application(&mime_type)?;
        let handlers =
            matching_file_handlers(discover()?, &mime_type, default_application_id.as_deref());
        Ok(FileAssociation {
            mime_type,
            default_application_id,
            handlers,
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Open With associations are available in the supported Linux session",
        ))
    }
}

/// Open one file with an exact compatible desktop application, optionally
/// making that application the XDG default first.
///
/// Both the file MIME type and desktop catalog are re-read immediately before
/// dispatch. `gio launch` interprets the trusted desktop entry and its field
/// codes without involving a shell.
pub fn open_file_with(
    path: &Path,
    expected_mime_type: &str,
    application_id: &str,
    make_default: bool,
) -> Result<(), OpenFileWithError> {
    #[cfg(target_os = "linux")]
    {
        let current_mime_type = query_file_mime_type(path)?;
        if current_mime_type != expected_mime_type {
            return Err(io::Error::other(
                "the file type changed while the Open With panel was visible",
            )
            .into());
        }
        let application = discover()?
            .into_iter()
            .find(|application| {
                application.id == application_id
                    && application
                        .mime_types
                        .iter()
                        .any(|candidate| candidate == &current_mime_type)
            })
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "the selected application is no longer installed or compatible",
                )
            })
            .map_err(OpenFileWithError::from)?;

        if make_default {
            run_xdg_mime(&[
                std::ffi::OsStr::new("default"),
                std::ffi::OsStr::new(&application.id),
                std::ffi::OsStr::new(&current_mime_type),
            ])?;
            if query_default_application(&current_mime_type)?.as_deref()
                != Some(application.id.as_str())
            {
                return Err(io::Error::other(
                    "the desktop did not retain the new default application",
                )
                .into());
            }
        }

        let launch = run_command_success(
            Command::new("gio")
                .arg("launch")
                .arg(&application.source)
                .arg(path),
            "launch the selected application",
        );
        if make_default {
            launch.map_err(|error| OpenFileWithError {
                default_changed: true,
                detail: format!(
                    "The default application changed, but the file could not be opened: {error}"
                ),
            })
        } else {
            launch.map_err(OpenFileWithError::from)
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (path, expected_mime_type, application_id, make_default);
        Err(OpenFileWithError {
            default_changed: false,
            detail: "Open With is available in the supported Linux session".into(),
        })
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

/// Exact XDG icon-theme resolver shared by shell surfaces. Resolution keeps
/// theme metadata cached, accepts only safe theme icon names, and returns a
/// private source path that callers must decode on a bounded worker.
pub struct ThemedIconResolver {
    pub(super) environment: Environment,
}

impl ThemedIconResolver {
    pub fn current() -> Self {
        Self {
            environment: Environment::current(),
        }
    }

    /// Resolves one name at the requested physical pixel edge. Scale-qualified
    /// theme directories participate through their physical-size distance.
    pub fn resolve(&self, name: &str, pixel_edge: u32) -> Option<PathBuf> {
        if pixel_edge == 0 || pixel_edge > 512 {
            return None;
        }
        resolve_named_icon(name, pixel_edge, &self.environment)
    }
}

impl Default for ThemedIconResolver {
    fn default() -> Self {
        Self::current()
    }
}

impl std::fmt::Debug for ThemedIconResolver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ThemedIconResolver")
            .field("theme", &"<redacted>")
            .field(
                "cached_themes",
                &self.environment.theme_cache.borrow().len(),
            )
            .finish()
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

/// Produce the exact argv that niri can spawn with an XDG activation token.
/// A working directory is represented by GNU `env --chdir`, which preserves
/// the token while directly execing the real application without a shell.
pub fn activation_spawn_argv(spec: &LaunchSpec) -> Option<Vec<String>> {
    activation_spawn_argv_with_terminal(
        spec,
        std::env::var_os("TERMINAL")
            .filter(|value| !value.is_empty())
            .and_then(|value| value.into_string().ok()),
    )
}

pub(super) fn activation_spawn_argv_with_terminal(
    spec: &LaunchSpec,
    terminal: Option<String>,
) -> Option<Vec<String>> {
    let LaunchSpec::Command {
        program,
        args,
        working_dir,
        terminal: needs_terminal,
    } = spec
    else {
        return None;
    };
    let mut command = if *needs_terminal {
        let mut command = vec![
            terminal.unwrap_or_else(|| "x-terminal-emulator".into()),
            "-e".into(),
            program.clone(),
        ];
        command.extend(args.iter().cloned());
        command
    } else {
        let mut command = Vec::with_capacity(args.len() + 1);
        command.push(program.clone());
        command.extend(args.iter().cloned());
        command
    };
    if let Some(directory) = working_dir {
        let directory = directory.to_str()?.to_string();
        command.splice(
            0..0,
            [
                "/usr/bin/env".into(),
                "--chdir".into(),
                directory,
                "--".into(),
            ],
        );
    }
    Some(command)
}

pub(super) fn terminal_command(program: &str, args: &[String]) -> Command {
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
pub(super) fn catalog_directories() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/Applications/Utilities"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Applications/Utilities"),
    ]
}

#[cfg(not(target_os = "macos"))]
pub(super) fn catalog_directories() -> Vec<PathBuf> {
    Environment::current().application_dirs()
}

pub(super) fn catalog_event_is_relevant(event: &Event) -> bool {
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
pub(super) fn catalog_path_is_relevant(path: &Path) -> bool {
    path.ancestors().any(|ancestor| {
        ancestor
            .extension()
            .and_then(|extension| extension.to_str())
            == Some("app")
    })
}

#[cfg(not(target_os = "macos"))]
pub(super) fn catalog_path_is_relevant(path: &Path) -> bool {
    path.extension().and_then(|extension| extension.to_str()) == Some("desktop")
        || path.file_name().and_then(|name| name.to_str()) == Some("applications")
}
