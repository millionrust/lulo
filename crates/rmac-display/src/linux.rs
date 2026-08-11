//! Linux niri display backend and persistence authority.

use super::*;

#[cfg(not(target_os = "macos"))]
pub(super) fn system_restore_snapshot(
    expected: &Snapshot,
    owned_current: &Snapshot,
) -> Result<Snapshot, Error> {
    let primary = expected
        .outputs
        .iter()
        .find(|output| output.primary && output.logical.is_some())
        .or_else(|| {
            expected
                .outputs
                .iter()
                .find(|output| output.logical.is_some())
        })
        .map(|output| output.id.as_str())
        .ok_or_else(|| Error::new("restore display layout", "no enabled display was captured"))?;
    let layout = current_layout(expected, primary)?;
    let current = system_snapshot()?;
    if !same_complete_layout(owned_current, &current) {
        return Err(Error::new(
            "restore display layout",
            "the display layout changed outside this confirmation; refresh Displays before changing it again",
        ));
    }
    validate_restore_authority(expected, &current, &layout)?;

    let mut first_error = None;
    for phase in 0..4 {
        for configured in &layout.outputs {
            let result = match phase {
                0 => run_niri_output(
                    &configured.id,
                    &["mode", &configured.mode.niri_argument()],
                    "restore display mode",
                ),
                1 => run_niri_output(
                    &configured.id,
                    &["scale", &format!("{:.2}", configured.scale)],
                    "restore display scale",
                ),
                2 => run_niri_output(
                    &configured.id,
                    &[
                        "transform",
                        configured
                            .transform
                            .niri_argument()
                            .expect("validated transform is configurable"),
                    ],
                    "restore display rotation",
                ),
                3 => run_niri_output(
                    &configured.id,
                    &[
                        "position",
                        "set",
                        &configured.x.to_string(),
                        &configured.y.to_string(),
                    ],
                    "restore display position",
                ),
                _ => unreachable!("display restoration has four command phases"),
            };
            if first_error.is_none() {
                first_error = result.err();
            }
        }
    }

    match verify_snapshot_restore(expected, &layout) {
        Ok(snapshot) => Ok(snapshot),
        Err(verification) => Err(match first_error {
            Some(command) => Error::new(
                "restore display layout",
                format!("{command}; {verification}"),
            ),
            None => verification,
        }),
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn validate_restore_authority(
    expected: &Snapshot,
    current: &Snapshot,
    layout: &Layout,
) -> Result<(), Error> {
    if !current.can_configure || !same_enabled_output_identities(expected, current) {
        return Err(Error::new(
            "restore display layout",
            "the connected display hardware changed; automatic restoration was stopped",
        ));
    }
    for configured in &layout.outputs {
        let current_output = current
            .outputs
            .iter()
            .find(|output| output.id == configured.id)
            .expect("matching enabled identities were checked");
        if !current_output.modes.contains(&configured.mode) {
            return Err(Error::new(
                "restore display layout",
                "a captured mode is no longer advertised by its display",
            ));
        }
    }
    Ok(())
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn same_enabled_output_identities(expected: &Snapshot, current: &Snapshot) -> bool {
    let expected = expected
        .outputs
        .iter()
        .filter(|output| output.logical.is_some() && output.current_mode().is_some())
        .collect::<Vec<_>>();
    let current = current
        .outputs
        .iter()
        .filter(|output| output.logical.is_some() && output.current_mode().is_some())
        .collect::<Vec<_>>();
    expected.len() == current.len()
        && expected.iter().all(|expected| {
            current.iter().any(|current| {
                current.id == expected.id
                    && current.connector == expected.connector
                    && current.serial == expected.serial
                    && current.name == expected.name
            })
        })
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn same_complete_layout(expected: &Snapshot, current: &Snapshot) -> bool {
    if !same_enabled_output_identities(expected, current) {
        return false;
    }
    let expected_primary = expected
        .outputs
        .iter()
        .find(|output| output.primary && output.logical.is_some())
        .map(|output| output.id.as_str());
    let current_primary = current
        .outputs
        .iter()
        .find(|output| output.primary && output.logical.is_some())
        .map(|output| output.id.as_str());
    if expected_primary != current_primary {
        return false;
    }
    let Some(primary) = expected_primary.or_else(|| {
        expected
            .outputs
            .iter()
            .find(|output| output.logical.is_some())
            .map(|output| output.id.as_str())
    }) else {
        return false;
    };
    match (
        current_layout(expected, primary),
        current_layout(current, primary),
    ) {
        (Ok(expected), Ok(current)) => expected == current,
        _ => false,
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn verify_snapshot_restore(
    expected: &Snapshot,
    layout: &Layout,
) -> Result<Snapshot, Error> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        if let Ok(snapshot) = system_snapshot() {
            if same_enabled_output_identities(expected, &snapshot)
                && layout_matches_snapshot(layout, &snapshot)
            {
                return Ok(snapshot);
            }
        }
        if std::time::Instant::now() >= deadline {
            return Err(Error::new(
                "verify restored display layout",
                "niri did not restore the complete captured layout within three seconds",
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn system_snapshot() -> Result<Snapshot, Error> {
    let output = command("niri", &["msg", "--json", "outputs"], "read niri displays")?;
    let mut outputs = parse_niri_outputs(&output)?;
    let persistence = persistence_authority();
    let primary = persistence
        .as_ref()
        .ok()
        .and_then(|authority| authority.primary.as_deref())
        .filter(|primary| outputs.iter().any(|output| output.id == *primary))
        .map(str::to_owned)
        .or_else(|| {
            outputs
                .iter()
                .find(|output| output.logical.is_some())
                .map(|output| output.id.clone())
        });
    for output in &mut outputs {
        output.primary = primary.as_deref() == Some(output.id.as_str());
    }
    let (can_persist, persistence_detail) = match persistence {
        Ok(authority) => (true, authority.detail),
        Err(error) => (false, Some(error.to_string())),
    };
    Ok(Snapshot {
        available: true,
        can_configure: true,
        can_persist,
        mirror_supported: false,
        compositor: "niri".to_string(),
        graphics: None,
        outputs,
        persistence_detail,
    })
}

#[cfg(not(target_os = "macos"))]
#[derive(Debug)]
pub(super) struct PersistenceAuthority {
    main_path: PathBuf,
    main_source: String,
    managed_path: PathBuf,
    managed_source: Option<String>,
    has_include: bool,
    primary: Option<String>,
    detail: Option<String>,
}

#[cfg(not(target_os = "macos"))]
pub(super) fn system_persist_layout(layout: &Layout) -> Result<Snapshot, Error> {
    let fresh = system_snapshot()?;
    if !fresh.can_persist {
        return Err(Error::new(
            "save the display layout",
            fresh
                .persistence_detail
                .unwrap_or_else(|| "persistent niri configuration is unavailable".into()),
        ));
    }
    validate_layout(layout, &fresh)?;
    let authority = persistence_authority()?;
    let managed_source = update_managed_source(authority.managed_source.as_deref(), layout)?;
    validate_config_transaction(&authority, &managed_source)?;

    let previous_managed = authority.managed_source.as_deref().map(str::as_bytes);
    if let Some(previous) = previous_managed {
        let backup = authority.managed_path.with_file_name(format!(
            "{}.last-good",
            authority
                .managed_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(MANAGED_CONFIG_NAME)
        ));
        rmac_storage::atomic_write(&backup, previous).map_err(|error| {
            Error::new("save the last-known-good display layout", error.to_string())
        })?;
    }
    rmac_storage::atomic_write(&authority.managed_path, managed_source.as_bytes())
        .map_err(|error| Error::new("save the display layout", error.to_string()))?;

    let next_main =
        (!authority.has_include).then(|| live_main_with_managed_include(&authority.main_source));
    if let Some(next_main) = &next_main {
        if let Err(error) = rmac_storage::atomic_write(&authority.main_path, next_main.as_bytes()) {
            let rollback = restore_managed(&authority.managed_path, previous_managed);
            let detail = match rollback {
                Ok(()) => error.to_string(),
                Err(rollback) => format!(
                    "{error}; restoring the previous managed display file also failed: {rollback}"
                ),
            };
            return Err(Error::new("enable the persistent display layout", detail));
        }
    }

    std::thread::sleep(std::time::Duration::from_millis(100));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        match system_snapshot() {
            Ok(snapshot)
                if layout_matches_snapshot(layout, &snapshot)
                    && snapshot
                        .outputs
                        .iter()
                        .any(|output| output.id == layout.primary && output.primary) =>
            {
                return Ok(snapshot);
            }
            Ok(_) | Err(_) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Ok(_) | Err(_) => {
                let rollback =
                    rollback_layout_transaction(&authority, next_main.is_some(), previous_managed);
                let detail = match rollback {
                    Ok(()) => "niri did not retain the confirmed layout within three seconds; the previous configuration was restored".to_owned(),
                    Err(rollback) => format!(
                        "niri did not retain the confirmed layout within three seconds, and automatic rollback failed: {rollback}"
                    ),
                };
                return Err(Error::new("verify the persistent display layout", detail));
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
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

#[cfg(not(target_os = "macos"))]
pub(super) fn rollback_layout_transaction(
    authority: &PersistenceAuthority,
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
        .map_err(|error| format!("managed display configuration: {error}"))
        .err();
    match (main_error, managed_error) {
        (None, None) => Ok(()),
        (Some(error), None) | (None, Some(error)) => Err(error),
        (Some(main), Some(managed)) => Err(format!("{main}; {managed}")),
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn persistence_authority() -> Result<PersistenceAuthority, Error> {
    let main_path = niri_config_path()?.ok_or_else(|| {
        Error::new(
            "resolve persistent display configuration",
            "no existing user niri configuration was found",
        )
    })?;
    reject_symlink(&main_path, "use the niri configuration")?;
    let main_source = read_bounded_config(&main_path, "read the niri configuration")?;
    let main: KdlDocument = main_source
        .parse()
        .map_err(|error| Error::new("parse the niri configuration", format!("{error}")))?;
    let include_indexes = main
        .nodes()
        .iter()
        .enumerate()
        .filter_map(|(index, node)| {
            (node.name().value() == "include"
                && node.get(0).and_then(KdlValue::as_string) == Some(MANAGED_CONFIG_NAME))
            .then_some(index)
        })
        .collect::<Vec<_>>();
    if include_indexes.len() > 1 {
        return Err(Error::new(
            "resolve persistent display configuration",
            "the rmac display include appears more than once",
        ));
    }
    if include_indexes.first().is_some_and(|index| *index != 0) {
        return Err(Error::new(
            "resolve persistent display configuration",
            "the rmac display include must remain the first top-level niri node so its output identity is deterministic",
        ));
    }
    let has_include = !include_indexes.is_empty();
    let managed_path = main_path.with_file_name(MANAGED_CONFIG_NAME);
    let managed_source = match std::fs::symlink_metadata(&managed_path) {
        Ok(_) => {
            reject_symlink(&managed_path, "use the rmac display configuration")?;
            let source =
                read_bounded_config(&managed_path, "read persistent display configuration")?;
            if !source.starts_with(MANAGED_HEADER) {
                return Err(Error::new(
                    "read persistent display configuration",
                    format!(
                        "{} already exists but is not owned by rmac",
                        managed_path.display()
                    ),
                ));
            }
            Some(source)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !has_include => None,
        Err(error) => {
            return Err(Error::new(
                "read persistent display configuration",
                error.to_string(),
            ));
        }
    };
    let primary = managed_source
        .as_deref()
        .map(parse_managed_primary)
        .transpose()?;
    Ok(PersistenceAuthority {
        main_path,
        main_source,
        managed_path,
        managed_source,
        has_include,
        primary: primary.flatten(),
        detail: (!has_include)
            .then(|| "The current layout is live but has not yet been saved by rmac.".into()),
    })
}

#[cfg(not(target_os = "macos"))]
pub(super) fn read_bounded_config(path: &Path, operation: &'static str) -> Result<String, Error> {
    use std::io::Read as _;

    let file =
        std::fs::File::open(path).map_err(|error| Error::new(operation, error.to_string()))?;
    if file
        .metadata()
        .map_err(|error| Error::new(operation, error.to_string()))?
        .len()
        > MAX_CONFIG_BYTES
    {
        return Err(Error::new(
            operation,
            "the configuration exceeds the 2 MiB safety limit",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| Error::new(operation, error.to_string()))?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(Error::new(
            operation,
            "the configuration exceeds the 2 MiB safety limit",
        ));
    }
    String::from_utf8(bytes)
        .map_err(|_| Error::new(operation, "the configuration is not valid UTF-8"))
}

#[cfg(not(target_os = "macos"))]
pub(super) fn niri_config_path() -> Result<Option<PathBuf>, Error> {
    if let Some(value) = std::env::var_os("NIRI_CONFIG").filter(|value| !value.is_empty()) {
        let path = PathBuf::from(value);
        if !path.is_absolute() {
            return Err(Error::new(
                "resolve the niri configuration",
                "NIRI_CONFIG must be an absolute path",
            ));
        }
        return Ok(path.is_file().then_some(path));
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| Error::new("resolve the niri configuration", "HOME is not set"))?;
    let path = match std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from) {
        Some(path) if path.is_absolute() => path.join("niri/config.kdl"),
        _ => home.join(".config/niri/config.kdl"),
    };
    Ok(path.is_file().then_some(path))
}

#[cfg(not(target_os = "macos"))]
pub(super) fn reject_symlink(path: &Path, operation: &'static str) -> Result<(), Error> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| Error::new(operation, error.to_string()))?;
    if metadata.file_type().is_symlink() {
        return Err(Error::new(
            operation,
            format!(
                "{} is a symbolic link and will not be replaced",
                path.display()
            ),
        ));
    }
    Ok(())
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn parse_managed_document(source: &str) -> Result<KdlDocument, Error> {
    let body = source.strip_prefix(MANAGED_HEADER).ok_or_else(|| {
        Error::new(
            "parse persistent display configuration",
            "the rmac ownership header is missing",
        )
    })?;
    let document: KdlDocument = body.parse().map_err(|error| {
        Error::new("parse persistent display configuration", format!("{error}"))
    })?;
    if document.nodes().len() > MAX_OUTPUTS
        || document
            .nodes()
            .iter()
            .any(|node| node.name().value() != "output")
    {
        return Err(Error::new(
            "parse persistent display configuration",
            "the managed file must contain at most 32 output blocks and no unrelated settings",
        ));
    }
    let mut identities = HashSet::new();
    for node in document.nodes() {
        let id = output_node_id(node)?;
        if !identities.insert(id) {
            return Err(Error::new(
                "parse persistent display configuration",
                "managed output identities must be unique",
            ));
        }
    }
    Ok(document)
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn output_node_id(node: &KdlNode) -> Result<&str, Error> {
    let id = node.get(0).and_then(KdlValue::as_string).ok_or_else(|| {
        Error::new(
            "parse persistent display configuration",
            "every output block must have one string identity",
        )
    })?;
    validate_output_id(id)?;
    if id.len() > MAX_OUTPUT_ID_BYTES || node.len() != 1 {
        return Err(Error::new(
            "parse persistent display configuration",
            "output identities must be bounded and contain no extra arguments",
        ));
    }
    Ok(id)
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn parse_managed_primary(source: &str) -> Result<Option<String>, Error> {
    let document = parse_managed_document(source)?;
    let primaries = document
        .nodes()
        .iter()
        .filter(|node| {
            node.children()
                .is_some_and(|children| children.get("focus-at-startup").is_some())
        })
        .map(output_node_id)
        .collect::<Result<Vec<_>, _>>()?;
    if primaries.len() > 1 {
        return Err(Error::new(
            "parse persistent display configuration",
            "only one output may be the rmac main display",
        ));
    }
    Ok(primaries.first().map(|id| (*id).to_owned()))
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn update_managed_source(
    existing: Option<&str>,
    layout: &Layout,
) -> Result<String, Error> {
    let mut document = match existing {
        Some(source) => parse_managed_document(source)?,
        None => KdlDocument::new(),
    };
    let current_ids = layout
        .outputs
        .iter()
        .map(|output| output.id.as_str())
        .collect::<HashSet<_>>();
    document
        .nodes_mut()
        .retain(|node| output_node_id(node).is_ok_and(|id| !current_ids.contains(id)));
    for node in document.nodes_mut() {
        if let Some(children) = node.children_mut() {
            children
                .nodes_mut()
                .retain(|child| child.name().value() != "focus-at-startup");
        }
    }
    let mut current = layout.outputs.clone();
    current.sort_by(|left, right| {
        (left.id != layout.primary)
            .cmp(&(right.id != layout.primary))
            .then_with(|| left.id.cmp(&right.id))
    });
    for output in current {
        document
            .nodes_mut()
            .push(configuration_node(&output, output.id == layout.primary));
    }
    if document.nodes().len() > MAX_OUTPUTS {
        return Err(Error::new(
            "save the display layout",
            "saved and connected displays exceed the 32-output safety limit",
        ));
    }
    document.ensure_v1();
    Ok(format!("{MANAGED_HEADER}\n{document}"))
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn configuration_node(output: &OutputConfiguration, primary: bool) -> KdlNode {
    let mut node = KdlNode::new("output");
    node.push(output.id.clone());
    let mut children = KdlDocument::new();
    let mut mode = KdlNode::new("mode");
    mode.push(output.mode.niri_argument());
    children.nodes_mut().push(mode);
    let mut scale = KdlNode::new("scale");
    scale.push(output.scale);
    children.nodes_mut().push(scale);
    let mut transform = KdlNode::new("transform");
    transform.push(
        output
            .transform
            .niri_argument()
            .expect("validated transforms are serializable"),
    );
    children.nodes_mut().push(transform);
    let mut position = KdlNode::new("position");
    position.insert("x", i128::from(output.x));
    position.insert("y", i128::from(output.y));
    children.nodes_mut().push(position);
    if primary {
        children.nodes_mut().push(KdlNode::new("focus-at-startup"));
    }
    node.set_children(children);
    node
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn validate_layout(layout: &Layout, snapshot: &Snapshot) -> Result<(), Error> {
    let enabled = snapshot
        .outputs
        .iter()
        .filter(|output| output.logical.is_some() && output.current_mode().is_some())
        .collect::<Vec<_>>();
    if layout.outputs.is_empty()
        || layout.outputs.len() > MAX_OUTPUTS
        || layout.outputs.len() != enabled.len()
    {
        return Err(Error::new(
            "validate the display layout",
            "the layout must contain every enabled display exactly once",
        ));
    }
    let mut identities = HashSet::new();
    for configured in &layout.outputs {
        validate_output_id(&configured.id)?;
        if configured.id.len() > MAX_OUTPUT_ID_BYTES || !identities.insert(&configured.id) {
            return Err(Error::new(
                "validate the display layout",
                "display identities must be bounded and unique",
            ));
        }
        if !configured.scale.is_finite()
            || !(0.5..=4.0).contains(&configured.scale)
            || !configured.transform.is_configurable()
            || configured.logical_width == 0
            || configured.logical_height == 0
            || configured.x.unsigned_abs() > MAX_LAYOUT_COORDINATE as u32
            || configured.y.unsigned_abs() > MAX_LAYOUT_COORDINATE as u32
        {
            return Err(Error::new(
                "validate the display layout",
                "display scale, transform, size, or position is outside the safe range",
            ));
        }
        let current = enabled
            .iter()
            .find(|output| output.id == configured.id)
            .ok_or_else(|| {
                Error::new(
                    "validate the display layout",
                    "a configured display is no longer enabled",
                )
            })?;
        if !current.modes.contains(&configured.mode) {
            return Err(Error::new(
                "validate the display layout",
                "a configured mode is no longer advertised by its display",
            ));
        }
        let logical = current.logical.as_ref().expect("enabled output checked");
        if current.current_mode() != Some(configured.mode)
            || (logical.scale - configured.scale).abs() > 0.001
            || logical.transform != configured.transform
            || logical.x != configured.x
            || logical.y != configured.y
            || logical.width != configured.logical_width
            || logical.height != configured.logical_height
        {
            return Err(Error::new(
                "validate the display layout",
                "the compositor state changed before the layout could be saved",
            ));
        }
    }
    if !identities.contains(&layout.primary) {
        return Err(Error::new(
            "validate the display layout",
            "the main display must be one of the enabled displays",
        ));
    }
    for (index, left) in layout.outputs.iter().enumerate() {
        for right in layout.outputs.iter().skip(index + 1) {
            if rectangles_overlap(left, right) {
                return Err(Error::new(
                    "validate the display layout",
                    "display rectangles must not overlap",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn rectangles_overlap(left: &OutputConfiguration, right: &OutputConfiguration) -> bool {
    let left_x2 = i64::from(left.x) + i64::from(left.logical_width);
    let left_y2 = i64::from(left.y) + i64::from(left.logical_height);
    let right_x2 = i64::from(right.x) + i64::from(right.logical_width);
    let right_y2 = i64::from(right.y) + i64::from(right.logical_height);
    i64::from(left.x) < right_x2
        && i64::from(right.x) < left_x2
        && i64::from(left.y) < right_y2
        && i64::from(right.y) < left_y2
}

#[cfg(not(target_os = "macos"))]
pub(super) fn layout_matches_snapshot(layout: &Layout, snapshot: &Snapshot) -> bool {
    current_layout(snapshot, &layout.primary).is_ok_and(|current| current == *layout)
}

#[cfg(not(target_os = "macos"))]
pub(super) fn live_main_with_managed_include(source: &str) -> String {
    format!("include \"{MANAGED_CONFIG_NAME}\"\n{source}")
}

#[cfg(not(target_os = "macos"))]
pub(super) fn validate_config_transaction(
    authority: &PersistenceAuthority,
    managed_source: &str,
) -> Result<(), Error> {
    let sequence = CANDIDATE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let managed_candidate_name = format!(
        ".rmac-displays-{}-{sequence}.candidate.kdl",
        std::process::id()
    );
    let managed_candidate = authority
        .managed_path
        .with_file_name(&managed_candidate_name);
    let main_candidate_name = format!(
        ".rmac-config-{}-{sequence}.candidate.kdl",
        std::process::id()
    );
    let main_candidate = authority.main_path.with_file_name(main_candidate_name);
    rmac_storage::atomic_write(&managed_candidate, managed_source.as_bytes())
        .map_err(|error| Error::new("write the display validation candidate", error.to_string()))?;

    let candidate_source = if authority.has_include {
        let mut document: KdlDocument = authority.main_source.parse().map_err(|error| {
            Error::new("parse the niri validation candidate", format!("{error}"))
        })?;
        let include = document
            .nodes_mut()
            .first_mut()
            .filter(|node| node.name().value() == "include")
            .ok_or_else(|| {
                Error::new(
                    "prepare the niri validation candidate",
                    "the managed include is no longer first",
                )
            })?;
        let value = include.get_mut(0).ok_or_else(|| {
            Error::new(
                "prepare the niri validation candidate",
                "the managed include has no path",
            )
        })?;
        *value = KdlValue::String(managed_candidate_name);
        document.to_string()
    } else {
        format!(
            "include \"{managed_candidate_name}\"\n{}",
            authority.main_source
        )
    };
    if let Err(error) = rmac_storage::atomic_write(&main_candidate, candidate_source.as_bytes()) {
        let _ = std::fs::remove_file(&managed_candidate);
        return Err(Error::new(
            "write the niri validation candidate",
            error.to_string(),
        ));
    }
    let mut validation = Command::new("niri");
    validation
        .arg("validate")
        .arg("--config")
        .arg(&main_candidate);
    let result = bounded_command_output(&mut validation, "run niri validation");
    let _ = std::fs::remove_file(&main_candidate);
    let _ = std::fs::remove_file(&managed_candidate);
    let output = result?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(Error::new(
            "validate the persistent display layout",
            if detail.is_empty() {
                format!("niri exited with {}", output.status)
            } else {
                detail
            },
        ));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub(super) fn run_niri_output(
    output: &str,
    arguments: &[&str],
    operation: &'static str,
) -> Result<(), Error> {
    let mut command_arguments = vec!["msg", "output", output];
    command_arguments.extend_from_slice(arguments);
    command("niri", &command_arguments, operation)?;
    Ok(())
}

#[cfg(any(not(target_os = "macos"), test))]
#[derive(Deserialize)]
pub(super) struct NiriOutput {
    name: String,
    #[serde(default)]
    make: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    serial: Option<String>,
    #[serde(default)]
    physical_size: Option<(u32, u32)>,
    #[serde(default)]
    modes: Vec<NiriMode>,
    #[serde(default)]
    current_mode: Option<usize>,
    #[serde(default)]
    logical: Option<NiriLogicalOutput>,
}

#[cfg(any(not(target_os = "macos"), test))]
#[derive(Deserialize)]
pub(super) struct NiriMode {
    width: u16,
    height: u16,
    refresh_rate: u32,
    #[serde(default)]
    is_preferred: bool,
}

#[cfg(any(not(target_os = "macos"), test))]
#[derive(Deserialize)]
pub(super) struct NiriLogicalOutput {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale: f64,
    transform: String,
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn parse_niri_outputs(json: &str) -> Result<Vec<Output>, Error> {
    let raw = serde_json::from_str::<HashMap<String, NiriOutput>>(json)
        .map_err(|error| Error::new("parse niri displays", error.to_string()))?;
    let mut outputs = raw
        .into_iter()
        .map(|(id, output)| {
            let display_name = [output.make.as_str(), output.model.as_str()]
                .into_iter()
                .filter(|part| !part.is_empty() && *part != "Unknown")
                .collect::<Vec<_>>()
                .join(" ");
            Output {
                id,
                connector: output.name.clone(),
                name: if display_name.is_empty() {
                    output.name
                } else {
                    display_name
                },
                serial: output.serial,
                physical_size_mm: output.physical_size,
                modes: output
                    .modes
                    .into_iter()
                    .map(|mode| Mode {
                        width: mode.width,
                        height: mode.height,
                        refresh_rate: mode.refresh_rate,
                        preferred: mode.is_preferred,
                    })
                    .collect(),
                current_mode: output.current_mode,
                logical: output.logical.map(|logical| LogicalOutput {
                    x: logical.x,
                    y: logical.y,
                    width: logical.width,
                    height: logical.height,
                    scale: logical.scale,
                    transform: parse_transform(&logical.transform),
                }),
                primary: false,
                detail: None,
            }
        })
        .collect::<Vec<_>>();
    outputs.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(outputs)
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn parse_transform(transform: &str) -> Transform {
    match transform {
        "Normal" | "normal" => Transform::Normal,
        "90" => Transform::Rotate90,
        "180" => Transform::Rotate180,
        "270" => Transform::Rotate270,
        "Flipped" | "flipped" => Transform::Flipped,
        "Flipped90" | "flipped-90" => Transform::Flipped90,
        "Flipped180" | "flipped-180" => Transform::Flipped180,
        "Flipped270" | "flipped-270" => Transform::Flipped270,
        other => Transform::Other(other.to_string()),
    }
}
