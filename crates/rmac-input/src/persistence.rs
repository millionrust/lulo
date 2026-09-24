//! Validated input persistence, rollback, and niri reload transactions.

use super::*;

pub(super) fn validate_settings(settings: &InputSettings) -> Result<(), Error> {
    if !(100..=2_000).contains(&settings.keyboard.repeat_delay_ms) {
        return Err(Error::new(
            "validate keyboard settings",
            "repeat delay must be between 100 and 2000 ms",
        ));
    }
    if !(1..=100).contains(&settings.keyboard.repeat_rate) {
        return Err(Error::new(
            "validate keyboard settings",
            "repeat rate must be between 1 and 100 characters per second",
        ));
    }
    for (device, pointer) in [
        ("mouse", &settings.mouse),
        ("touchpad", &settings.touchpad.pointer),
    ] {
        if !pointer.accel_speed.is_finite() || !(-1.0..=1.0).contains(&pointer.accel_speed) {
            return Err(Error::new(
                "validate pointer settings",
                format!("{device} tracking speed must be between -1 and 1"),
            ));
        }
    }
    Ok(())
}

pub(super) fn save_with_authority(
    authority: Authority,
    settings: &InputSettings,
) -> Result<Snapshot, Error> {
    if authority.effective.settings == *settings {
        return Ok(snapshot_from_authority(authority, true));
    }
    let managed = update_managed_source(&authority, settings)?;
    ensure_authority_unchanged(&authority)?;
    validate_candidate(&authority, &managed)?;
    ensure_authority_unchanged(&authority)?;
    let mut reload_witness = ReloadWitness::open()?;
    ensure_authority_unchanged(&authority)?;

    let previous_managed = authority.managed_source.as_deref().map(str::as_bytes);
    if let Some(previous) = previous_managed {
        let backup = authority
            .managed_path
            .with_file_name(format!("{MANAGED_CONFIG_NAME}.last-good"));
        rmac_storage::atomic_write(&backup, previous).map_err(|error| {
            Error::new(
                "save the last-known-good input configuration",
                error.to_string(),
            )
        })?;
    }
    rmac_storage::atomic_write(&authority.managed_path, managed.as_bytes())
        .map_err(|error| Error::new("save the managed input configuration", error.to_string()))?;
    if !authority.has_managed_include {
        let main = main_with_managed_include(&authority.main_source, MANAGED_CONFIG_NAME);
        if let Err(error) = rmac_storage::atomic_write(&authority.main_path, main.as_bytes()) {
            let rollback = restore_managed(&authority.managed_path, previous_managed);
            return Err(Error::new(
                "enable the managed input configuration",
                match rollback {
                    Ok(()) => error.to_string(),
                    Err(rollback) => format!(
                        "{error}; restoring the previous managed file also failed: {rollback}"
                    ),
                },
            ));
        }
    }

    if let Err(error) = reload_witness.wait_for_reload() {
        let rollback = rollback_input_transaction(
            &authority,
            !authority.has_managed_include,
            previous_managed,
        );
        return Err(Error::new(
            "verify niri adopted the input configuration",
            match rollback {
                Ok(()) => format!("{error}; the previous configuration was restored"),
                Err(rollback) => format!("{error}; automatic rollback failed: {rollback}"),
            },
        ));
    }

    let snapshot = match snapshot() {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let rollback = rollback_input_transaction(
                &authority,
                !authority.has_managed_include,
                previous_managed,
            );
            return Err(Error::new(
                "verify the saved input configuration",
                match rollback {
                    Ok(()) => format!("{error}; the previous configuration was restored"),
                    Err(rollback) => format!("{error}; automatic rollback failed: {rollback}"),
                },
            ));
        }
    };
    if snapshot.settings != *settings {
        let rollback = rollback_input_transaction(
            &authority,
            !authority.has_managed_include,
            previous_managed,
        );
        return Err(Error::new(
            "verify the saved input configuration",
            match rollback {
                Ok(()) => "the complete saved values did not match; the previous configuration was restored".into(),
                Err(rollback) => format!(
                    "the complete saved values did not match, and automatic rollback failed: {rollback}"
                ),
            },
        ));
    }
    Ok(snapshot)
}

pub(super) fn update_managed_source(
    authority: &Authority,
    settings: &InputSettings,
) -> Result<String, Error> {
    let mut document = match authority.managed_source.as_deref() {
        Some(source) => parse_managed_document(source)?,
        None => KdlDocument::new(),
    };
    let input = ensure_children(&mut document, "input");
    if settings.keyboard != authority.effective.settings.keyboard || input.get("keyboard").is_some()
    {
        let keyboard = ensure_children(input, "keyboard");
        replace_value(
            keyboard,
            "repeat-delay",
            i128::from(settings.keyboard.repeat_delay_ms),
        );
        replace_value(
            keyboard,
            "repeat-rate",
            i128::from(settings.keyboard.repeat_rate),
        );
        replace_explicit_flag(keyboard, "numlock", settings.keyboard.numlock)?;
    }

    if settings.mouse != authority.effective.settings.mouse || input.get("mouse").is_some() {
        if input.get("mouse").is_none() {
            input.nodes_mut().push(
                authority
                    .effective
                    .mouse_node
                    .clone()
                    .unwrap_or_else(|| input_node("mouse")),
            );
        }
        let mouse = input
            .get_mut("mouse")
            .expect("mouse node exists")
            .ensure_children();
        write_pointer(mouse, &settings.mouse);
    }

    if settings.touchpad != authority.effective.settings.touchpad || input.get("touchpad").is_some()
    {
        if input.get("touchpad").is_none() {
            input.nodes_mut().push(
                authority
                    .effective
                    .touchpad_node
                    .clone()
                    .unwrap_or_else(|| input_node("touchpad")),
            );
        }
        let touchpad = input
            .get_mut("touchpad")
            .expect("touchpad node exists")
            .ensure_children();
        write_pointer(touchpad, &settings.touchpad.pointer);
        replace_flag(touchpad, "tap", settings.touchpad.tap_to_click);
        replace_flag(touchpad, "dwt", settings.touchpad.disable_while_typing);
        replace_flag(touchpad, "drag-lock", settings.touchpad.drag_lock);
        replace_string_value(
            touchpad,
            "click-method",
            settings.touchpad.secondary_click.id(),
        );
    }
    document.ensure_v1();
    Ok(format!("{MANAGED_HEADER}\n{document}"))
}

pub(super) fn input_node(name: &str) -> KdlNode {
    let mut node = KdlNode::new(name);
    node.set_children(KdlDocument::new());
    node
}

pub(super) fn parse_managed_document(source: &str) -> Result<KdlDocument, Error> {
    let body = if source == MANAGED_HEADER {
        ""
    } else {
        source
            .strip_prefix(MANAGED_HEADER)
            .and_then(|body| body.strip_prefix('\n'))
            .ok_or_else(|| {
                Error::new(
                    "parse the managed input configuration",
                    "the exact rmac ownership header is missing",
                )
            })?
    };
    let document = KdlDocument::parse_v1(body)
        .map_err(|error| Error::new("parse the managed input configuration", format!("{error}")))?;
    if document.nodes().len() > 1
        || document
            .nodes()
            .iter()
            .any(|node| node.name().value() != "input" || !node.is_empty())
    {
        return Err(Error::new(
            "parse the managed input configuration",
            "the managed file may contain only one argument-free input block",
        ));
    }
    if let Some(input) = document.get("input").and_then(KdlNode::children) {
        let mut sections = HashSet::new();
        for node in input.nodes() {
            let name = node.name().value();
            if !matches!(name, "keyboard" | "mouse" | "touchpad")
                || !node.is_empty()
                || !sections.insert(name)
            {
                return Err(Error::new(
                    "parse the managed input configuration",
                    "the managed input block contains an unsupported or duplicate device section",
                ));
            }
            if name == "keyboard" {
                validate_managed_keyboard(node)?;
            }
        }
    }
    Ok(document)
}

pub(super) fn validate_managed_keyboard(node: &KdlNode) -> Result<(), Error> {
    let Some(keyboard) = node.children() else {
        return Ok(());
    };
    let mut settings = HashSet::new();
    for setting in keyboard.nodes() {
        let name = setting.name().value();
        let valid_value = match name {
            "repeat-delay" | "repeat-rate" => {
                setting.len() == 1
                    && setting.get(0).and_then(KdlValue::as_integer).is_some()
                    && setting.children().is_none()
            }
            "numlock" => {
                (setting.is_empty()
                    || (setting.len() == 1 && setting.get(0).and_then(KdlValue::as_bool).is_some()))
                    && setting.children().is_none()
            }
            _ => false,
        };
        if !valid_value || !settings.insert(name) {
            return Err(Error::new(
                "parse the managed input configuration",
                "the managed keyboard section contains unsupported, duplicate, or malformed settings",
            ));
        }
    }
    Ok(())
}

pub(super) fn ensure_children<'a>(
    document: &'a mut KdlDocument,
    name: &str,
) -> &'a mut KdlDocument {
    if document.get(name).is_none() {
        let mut node = KdlNode::new(name);
        node.set_children(KdlDocument::new());
        document.nodes_mut().push(node);
    }
    let node = document.get_mut(name).expect("node was just created");
    node.ensure_children()
}

pub(super) fn write_pointer(document: &mut KdlDocument, pointer: &PointerSettings) {
    replace_flag(document, "off", !pointer.enabled);
    replace_flag(document, "natural-scroll", pointer.natural_scroll);
    replace_value(document, "accel-speed", pointer.accel_speed);
    replace_string_value(document, "accel-profile", pointer.accel_profile.id());
    replace_flag(document, "left-handed", pointer.left_handed);
    replace_flag(document, "middle-emulation", pointer.middle_emulation);
}

pub(super) fn remove_named(document: &mut KdlDocument, name: &str) {
    document
        .nodes_mut()
        .retain(|node| node.name().value() != name);
}

pub(super) fn replace_flag(document: &mut KdlDocument, name: &str, enabled: bool) {
    remove_named(document, name);
    if enabled {
        document.nodes_mut().push(KdlNode::new(name));
    }
}

pub(super) fn replace_explicit_flag(
    document: &mut KdlDocument,
    name: &str,
    enabled: bool,
) -> Result<(), Error> {
    remove_named(document, name);
    if enabled {
        document.nodes_mut().push(KdlNode::new(name));
    } else {
        let parsed = KdlDocument::parse_v1(&format!("{name} false\n"))
            .map_err(|error| Error::new("write an explicit niri input flag", format!("{error}")))?;
        document.nodes_mut().extend(parsed.nodes().iter().cloned());
    }
    Ok(())
}

pub(super) fn replace_value(
    document: &mut KdlDocument,
    name: &str,
    value: impl Into<kdl::KdlEntry>,
) {
    remove_named(document, name);
    let mut node = KdlNode::new(name);
    node.push(value);
    document.nodes_mut().push(node);
}

pub(super) fn replace_string_value(document: &mut KdlDocument, name: &str, value: &str) {
    remove_named(document, name);
    let parsed = KdlDocument::parse_v1(&format!("{name} \"{value}\"\n"))
        .expect("controlled niri string settings are valid KDL v1");
    document.nodes_mut().extend(parsed.nodes().iter().cloned());
}

pub(super) fn validate_candidate(authority: &Authority, managed_source: &str) -> Result<(), Error> {
    let sequence = CANDIDATE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let managed_name = format!(
        ".rmac-input-{}-{sequence}.candidate.kdl",
        std::process::id()
    );
    let managed_candidate = authority.managed_path.with_file_name(&managed_name);
    let main_name = format!(
        ".rmac-config-{}-{sequence}.candidate.kdl",
        std::process::id()
    );
    let main_candidate = authority.main_path.with_file_name(main_name);
    let result = (|| {
        rmac_storage::atomic_write(&managed_candidate, managed_source.as_bytes())
            .map_err(|error| Error::new("write the validation candidate", error.to_string()))?;
        let main_source = if authority.has_managed_include {
            let mut document = KdlDocument::parse_v1(&authority.main_source).map_err(|error| {
                Error::new("parse the validation candidate", format!("{error}"))
            })?;
            let include = document.nodes_mut().last_mut().ok_or_else(|| {
                Error::new(
                    "prepare the validation candidate",
                    "managed include is missing",
                )
            })?;
            let value = include.get_mut(0).ok_or_else(|| {
                Error::new(
                    "prepare the validation candidate",
                    "managed include path is missing",
                )
            })?;
            *value = KdlValue::String(managed_name);
            document.ensure_v1();
            document.to_string()
        } else {
            main_with_managed_include(&authority.main_source, &managed_name)
        };
        rmac_storage::atomic_write(&main_candidate, main_source.as_bytes()).map_err(|error| {
            Error::new("write the main validation candidate", error.to_string())
        })?;
        let mut command = Command::new("niri");
        command.arg("validate").arg("--config").arg(&main_candidate);
        bounded_command_output(&mut command, "run niri validation")
    })();
    let cleanup = [&main_candidate, &managed_candidate]
        .into_iter()
        .filter_map(|path| match std::fs::remove_file(path) {
            Ok(()) => None,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => Some(format!("{}: {error}", path.display())),
        })
        .collect::<Vec<_>>();
    if !cleanup.is_empty() {
        return Err(Error::new(
            "remove the input validation candidates",
            match result {
                Ok(_) => cleanup.join("; "),
                Err(error) => format!("{error}; cleanup also failed: {}", cleanup.join("; ")),
            },
        ));
    }
    let output = result?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(Error::new(
            "validate the niri configuration",
            if detail.is_empty() {
                format!("niri exited with {}", output.status)
            } else {
                detail
            },
        ));
    }
    Ok(())
}

pub(super) fn main_with_managed_include(source: &str, managed_name: &str) -> String {
    let mut main = source.to_owned();
    if !main.ends_with('\n') {
        main.push('\n');
    }
    main.push_str(&format!("include \"{managed_name}\"\n"));
    main
}

pub(super) fn ensure_authority_unchanged(authority: &Authority) -> Result<(), Error> {
    for file in &authority.files {
        if read_bounded_config(&file.path)? != file.source {
            return Err(Error::new(
                "save input settings",
                format!(
                    "{} changed in the niri include graph; refresh before trying again",
                    file.path.display()
                ),
            ));
        }
    }
    for path in &authority.missing_optional_files {
        match std::fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) => {
                return Err(Error::new(
                    "save input settings",
                    format!(
                        "the optional include {} appeared; refresh before trying again",
                        path.display()
                    ),
                ));
            }
            Err(error) => return Err(Error::new("save input settings", error.to_string())),
        }
    }
    match authority.managed_source.as_deref() {
        Some(expected) if read_bounded_config(&authority.managed_path)? != expected => {
            Err(Error::new(
                "save input settings",
                "the managed input configuration changed; refresh before trying again",
            ))
        }
        Some(_) => Ok(()),
        None => match std::fs::symlink_metadata(&authority.managed_path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Ok(_) => Err(Error::new(
                "save input settings",
                "the managed input path was created concurrently; refresh before trying again",
            )),
            Err(error) => Err(Error::new("save input settings", error.to_string())),
        },
    }
}

pub(super) fn restore_managed(path: &Path, previous: Option<&[u8]>) -> Result<(), String> {
    match previous {
        Some(previous) => {
            rmac_storage::atomic_write(path, previous).map_err(|error| error.to_string())
        }
        None => match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        },
    }
}

pub(super) fn rollback_input_transaction(
    authority: &Authority,
    restore_main: bool,
    previous_managed: Option<&[u8]>,
) -> Result<(), String> {
    let main_error = restore_main
        .then(|| {
            rmac_storage::atomic_write(&authority.main_path, authority.main_source.as_bytes())
                .map_err(|error| format!("main configuration: {error}"))
        })
        .transpose()
        .err();
    let managed_error = restore_managed(&authority.managed_path, previous_managed)
        .map_err(|error| format!("managed input configuration: {error}"))
        .err();
    match (main_error, managed_error) {
        (None, None) => Ok(()),
        (Some(error), None) | (None, Some(error)) => Err(error),
        (Some(main), Some(managed)) => Err(format!("{main}; {managed}")),
    }
}

#[cfg(unix)]
pub(super) struct ReloadWitness {
    reader: std::io::BufReader<std::os::unix::net::UnixStream>,
}

#[cfg(unix)]
impl ReloadWitness {
    fn open() -> Result<Self, Error> {
        let socket = std::env::var_os("NIRI_SOCKET")
            .filter(|socket| !socket.is_empty())
            .ok_or_else(|| Error::new("watch niri config adoption", "NIRI_SOCKET is not set"))?;
        let mut stream = std::os::unix::net::UnixStream::connect(PathBuf::from(socket))
            .map_err(|error| Error::new("watch niri config adoption", error.to_string()))?;
        stream
            .set_write_timeout(Some(std::time::Duration::from_millis(500)))
            .map_err(|error| Error::new("watch niri config adoption", error.to_string()))?;
        stream
            .write_all(b"\"EventStream\"\n")
            .and_then(|()| stream.flush())
            .map_err(|error| Error::new("start the niri event stream", error.to_string()))?;
        let mut reader = std::io::BufReader::new(stream);
        let deadline = std::time::Instant::now() + RELOAD_TIMEOUT;
        let reply = read_reload_json_line(&mut reader, deadline)?;
        if reply.get("Ok").is_none() {
            return Err(Error::new(
                "start the niri event stream",
                "niri rejected the event-stream request",
            ));
        }
        if next_config_load(&mut reader, deadline)? {
            return Err(Error::new(
                "watch niri config adoption",
                "niri reports that its current configuration failed to load",
            ));
        }
        Ok(Self { reader })
    }

    fn wait_for_reload(&mut self) -> Result<(), Error> {
        let failed =
            next_config_load(&mut self.reader, std::time::Instant::now() + RELOAD_TIMEOUT)?;
        if failed {
            Err(Error::new(
                "watch niri config adoption",
                "niri rejected the updated configuration",
            ))
        } else {
            Ok(())
        }
    }
}

#[cfg(not(unix))]
pub(super) struct ReloadWitness;

#[cfg(not(unix))]
impl ReloadWitness {
    fn open() -> Result<Self, Error> {
        Err(Error::new(
            "watch niri config adoption",
            "niri config adoption is available only on Linux",
        ))
    }

    fn wait_for_reload(&mut self) -> Result<(), Error> {
        Err(Error::new(
            "watch niri config adoption",
            "niri config adoption is available only on Linux",
        ))
    }
}

#[cfg(unix)]
pub(super) fn read_reload_json_line(
    reader: &mut std::io::BufReader<std::os::unix::net::UnixStream>,
    deadline: std::time::Instant,
) -> Result<serde_json::Value, Error> {
    loop {
        let now = std::time::Instant::now();
        if now >= deadline {
            return Err(Error::new(
                "read the niri event stream",
                "niri did not report config state within three seconds",
            ));
        }
        reader
            .get_mut()
            .set_read_timeout(Some(
                deadline
                    .saturating_duration_since(now)
                    .min(std::time::Duration::from_millis(250)),
            ))
            .map_err(|error| Error::new("read the niri event stream", error.to_string()))?;
        let mut bytes = Vec::new();
        let mut limited = reader.take(MAX_COMMAND_OUTPUT_BYTES as u64 + 1);
        match limited.read_until(b'\n', &mut bytes) {
            Ok(0) => {
                return Err(Error::new(
                    "read the niri event stream",
                    "the niri IPC socket closed",
                ));
            }
            Ok(_) if bytes.len() > MAX_COMMAND_OUTPUT_BYTES => {
                return Err(Error::new(
                    "read the niri event stream",
                    "a niri event exceeded the 4 MiB safety limit",
                ));
            }
            Ok(_) => {
                while matches!(bytes.last(), Some(b'\n' | b'\r')) {
                    bytes.pop();
                }
                return serde_json::from_slice(&bytes).map_err(|error| {
                    Error::new("decode the niri event stream", error.to_string())
                });
            }
            Err(error)
                if bytes.is_empty()
                    && matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
            Err(error) => {
                return Err(Error::new("read the niri event stream", error.to_string()));
            }
        }
    }
}

#[cfg(unix)]
pub(super) fn next_config_load(
    reader: &mut std::io::BufReader<std::os::unix::net::UnixStream>,
    deadline: std::time::Instant,
) -> Result<bool, Error> {
    loop {
        let event = read_reload_json_line(reader, deadline)?;
        if let Some(failed) = config_load_failed(&event) {
            return Ok(failed);
        }
    }
}

#[cfg(any(unix, test))]
pub(super) fn config_load_failed(event: &serde_json::Value) -> Option<bool> {
    event.get("ConfigLoaded")?.get("failed")?.as_bool()
}

pub(super) fn niri_available() -> bool {
    if !cfg!(target_os = "linux")
        || std::env::var_os("NIRI_SOCKET").is_none_or(|socket| socket.is_empty())
    {
        return false;
    }
    let mut command = Command::new("niri");
    command.args(["msg", "--json", "version"]);
    bounded_command_output(&mut command, "contact the niri compositor")
        .is_ok_and(|output| output.status.success())
}

pub(super) fn validate_current_config(path: &Path) -> Result<(), Error> {
    let mut command = Command::new("niri");
    command.arg("validate").arg("--config").arg(path);
    let output = bounded_command_output(&mut command, "validate the current niri configuration")?;
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(Error::new(
        "validate the current niri configuration",
        if detail.is_empty() {
            format!("niri exited with {}", output.status)
        } else {
            detail
        },
    ))
}

pub(super) struct CommandOutput {
    status: ExitStatus,
    _stdout: Vec<u8>,
    stderr: Vec<u8>,
}

pub(super) fn bounded_command_output(
    command: &mut Command,
    operation: &'static str,
) -> Result<CommandOutput, Error> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| Error::new(operation, error.to_string()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::new(operation, "stdout was not captured"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::new(operation, "stderr was not captured"))?;
    let stdout_reader = std::thread::spawn(move || drain_bounded(stdout, MAX_COMMAND_OUTPUT_BYTES));
    let stderr_reader = std::thread::spawn(move || drain_bounded(stderr, MAX_COMMAND_OUTPUT_BYTES));
    let deadline = std::time::Instant::now() + COMMAND_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(Error::new(
                    operation,
                    "the command did not finish within ten seconds",
                ));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(Error::new(operation, error.to_string()));
            }
        }
    };
    let (stdout, stdout_truncated) = stdout_reader
        .join()
        .map_err(|_| Error::new(operation, "the stdout reader stopped unexpectedly"))?
        .map_err(|error| Error::new(operation, error.to_string()))?;
    let (stderr, stderr_truncated) = stderr_reader
        .join()
        .map_err(|_| Error::new(operation, "the stderr reader stopped unexpectedly"))?
        .map_err(|error| Error::new(operation, error.to_string()))?;
    let status = status?;
    if stdout_truncated || stderr_truncated {
        return Err(Error::new(
            operation,
            "the command output exceeded the 4 MiB safety limit",
        ));
    }
    Ok(CommandOutput {
        status,
        _stdout: stdout,
        stderr,
    })
}

pub(super) fn drain_bounded(
    mut reader: impl std::io::Read,
    limit: usize,
) -> std::io::Result<(Vec<u8>, bool)> {
    let mut captured = Vec::with_capacity(limit.min(8192));
    let mut buffer = [0_u8; 8192];
    let mut truncated = false;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let remaining = limit.saturating_sub(captured.len());
        captured.extend_from_slice(&buffer[..read.min(remaining)]);
        truncated |= read > remaining;
    }
    Ok((captured, truncated))
}
