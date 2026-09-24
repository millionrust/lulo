use std::fmt;

#[cfg(target_os = "linux")]
const MAX_PLUGIN_COUNT: usize = 64;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct VpnImportCapabilityId {
    type_name: String,
    service: String,
}

impl fmt::Debug for VpnImportCapabilityId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VpnImportCapabilityId")
            .field("type", &"<opaque>")
            .field("service", &"<opaque>")
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VpnImportCapability {
    pub id: VpnImportCapabilityId,
    pub name: String,
    pub format_hint: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VpnImportCapabilities {
    pub available: bool,
    pub plugins: Vec<VpnImportCapability>,
    pub limitation: Option<String>,
}

#[derive(Clone)]
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub struct VpnImportPreviewId {
    profile: super::VpnProfileId,
    settings: SettingsMap,
}

impl fmt::Debug for VpnImportPreviewId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VpnImportPreviewId")
            .field("profile", &"<redacted>")
            .field("settings", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct VpnImportPreview {
    pub id: VpnImportPreviewId,
    pub name: String,
    pub service: String,
    pub source_name: String,
}

#[cfg(not(target_os = "macos"))]
type SettingsMap = std::collections::HashMap<
    String,
    std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
>;

#[cfg(target_os = "macos")]
type SettingsMap = ();

pub(super) fn capabilities() -> VpnImportCapabilities {
    #[cfg(target_os = "linux")]
    {
        linux_capabilities()
    }
    #[cfg(not(target_os = "linux"))]
    {
        VpnImportCapabilities {
            available: false,
            plugins: Vec::new(),
            limitation: Some(
                "VPN configuration import is available in the supported Linux session".into(),
            ),
        }
    }
}

pub(super) fn preview(
    capability: &VpnImportCapabilityId,
    path: &std::path::Path,
) -> Result<VpnImportPreview, super::Error> {
    #[cfg(not(target_os = "macos"))]
    {
        linux_preview(capability, path)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = (capability, path);
        Err(super::Error::new(
            "import VPN configuration",
            "VPN import is available in the supported Linux session",
        ))
    }
}

pub(super) fn finish(
    preview: &VpnImportPreviewId,
    keep: bool,
) -> Result<super::VpnSnapshot, super::Error> {
    #[cfg(not(target_os = "macos"))]
    {
        linux_finish(preview, keep)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = (preview, keep);
        Err(super::Error::new(
            "finish VPN import",
            "VPN import is available in the supported Linux session",
        ))
    }
}

#[cfg(not(target_os = "macos"))]
const MAX_IMPORT_BYTES: u64 = 4 * 1024 * 1024;
#[cfg(not(target_os = "macos"))]
const MAX_COMMAND_OUTPUT_BYTES: usize = 16 * 1024;
#[cfg(not(target_os = "macos"))]
const IMPORT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
#[cfg(not(target_os = "macos"))]
const IMPORT_SETTLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
#[cfg(not(target_os = "macos"))]
const IMPORT_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

#[cfg(not(target_os = "macos"))]
struct StagedImport {
    id: VpnImportPreviewId,
    name: String,
    service: String,
}

#[cfg(not(target_os = "macos"))]
struct SensitiveBytes(Vec<u8>);

#[cfg(not(target_os = "macos"))]
impl Drop for SensitiveBytes {
    fn drop(&mut self) {
        use zeroize::Zeroize as _;
        self.0.zeroize();
    }
}

#[cfg(not(target_os = "macos"))]
fn linux_preview(
    capability: &VpnImportCapabilityId,
    path: &std::path::Path,
) -> Result<VpnImportPreview, super::Error> {
    let current = capabilities();
    if !current.available
        || !current
            .plugins
            .iter()
            .any(|candidate| candidate.id == *capability)
    {
        return Err(super::Error::new(
            "import VPN configuration",
            "the selected importer is no longer available",
        ));
    }
    let canonical = path.canonicalize().map_err(|_| {
        super::Error::new(
            "import VPN configuration",
            "the selected file is no longer available",
        )
    })?;
    let metadata = canonical.metadata().map_err(|_| {
        super::Error::new(
            "import VPN configuration",
            "the selected file could not be inspected",
        )
    })?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_IMPORT_BYTES {
        return Err(super::Error::new(
            "import VPN configuration",
            "choose a non-empty regular configuration file no larger than 4 MiB",
        ));
    }
    let source_name = canonical
        .file_name()
        .map(|name| name.to_string_lossy().chars().take(200).collect::<String>())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "VPN configuration".to_string());
    let before_bytes = read_import_file(&canonical)?;
    let connection = super::system_connection("connect to NetworkManager for VPN import")?;
    let before = connection_paths(&connection)?;
    let command = run_import_command(capability, &canonical);
    let after = connection_paths(&connection)?;
    let new_paths = after
        .into_iter()
        .filter(|path| !before.contains(path))
        .collect::<Vec<_>>();
    let staged = find_staged_import(
        &connection,
        capability,
        &new_paths,
        command
            .as_ref()
            .ok()
            .and_then(|result| result.uuid.as_deref()),
    );
    if let Err(error) = command {
        if let Ok(candidate) = &staged {
            let _ = delete_exact_staged(&connection, &candidate.id);
        }
        return Err(error);
    }
    let staged = staged?;
    let after_bytes = match read_import_file(&canonical) {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = delete_exact_staged(&connection, &staged.id);
            return Err(error);
        }
    };
    let file_unchanged = before_bytes.0 == after_bytes.0;
    if !file_unchanged {
        let _ = delete_exact_staged(&connection, &staged.id);
        return Err(super::Error::new(
            "import VPN configuration",
            "the selected file changed while its importer was reading it",
        ));
    }
    require_staged_autoconnect_disabled(&connection, &staged.id)?;
    Ok(VpnImportPreview {
        id: staged.id,
        name: staged.name,
        service: staged.service,
        source_name,
    })
}

#[cfg(not(target_os = "macos"))]
fn linux_finish(
    preview: &VpnImportPreviewId,
    keep: bool,
) -> Result<super::VpnSnapshot, super::Error> {
    let connection = super::system_connection("connect to NetworkManager for VPN import")?;
    let profile = exact_staged_profile(&connection, preview)?;
    if keep {
        profile
            .call::<_, _, ()>("Save", &())
            .map_err(|error| super::Error::new("save imported VPN", error.to_string()))?;
        let deadline = std::time::Instant::now() + IMPORT_SETTLE_TIMEOUT;
        loop {
            let current = exact_profile_settings(&connection, &preview.profile)?;
            if current == preview.settings
                && !profile
                    .get_property::<bool>("Unsaved")
                    .map_err(|error| super::Error::new("verify imported VPN", error.to_string()))?
            {
                let snapshot = super::linux_vpn_snapshot()?;
                if snapshot
                    .profiles
                    .iter()
                    .any(|profile| profile.id == preview.profile)
                {
                    return Ok(snapshot);
                }
            }
            if std::time::Instant::now() >= deadline {
                return Err(super::Error::new(
                    "save imported VPN",
                    "NetworkManager did not confirm persistent storage within 10 seconds",
                ));
            }
            std::thread::sleep(IMPORT_POLL_INTERVAL);
        }
    }

    profile
        .call::<_, _, ()>("Delete", &())
        .map_err(|error| super::Error::new("cancel VPN import", error.to_string()))?;
    let deadline = std::time::Instant::now() + IMPORT_SETTLE_TIMEOUT;
    loop {
        if !connection_paths(&connection)?
            .iter()
            .any(|path| path.as_str() == preview.profile.object_path)
        {
            return super::linux_vpn_snapshot();
        }
        if std::time::Instant::now() >= deadline {
            return Err(super::Error::new(
                "cancel VPN import",
                "NetworkManager did not remove the temporary profile within 10 seconds",
            ));
        }
        std::thread::sleep(IMPORT_POLL_INTERVAL);
    }
}

#[cfg(not(target_os = "macos"))]
fn read_import_file(path: &std::path::Path) -> Result<SensitiveBytes, super::Error> {
    use std::io::Read as _;

    let file = std::fs::File::open(path).map_err(|_| {
        super::Error::new(
            "read VPN configuration",
            "the selected file could not be opened",
        )
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_IMPORT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            super::Error::new(
                "read VPN configuration",
                "the selected file could not be read",
            )
        })?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_IMPORT_BYTES {
        use zeroize::Zeroize as _;
        bytes.zeroize();
        return Err(super::Error::new(
            "read VPN configuration",
            "choose a non-empty configuration file no larger than 4 MiB",
        ));
    }
    Ok(SensitiveBytes(bytes))
}

#[cfg(not(target_os = "macos"))]
struct ImportCommandResult {
    uuid: Option<String>,
}

#[cfg(not(target_os = "macos"))]
fn run_import_command(
    capability: &VpnImportCapabilityId,
    path: &std::path::Path,
) -> Result<ImportCommandResult, super::Error> {
    use std::process::{Command, Stdio};
    use zeroize::Zeroize as _;

    fn drain_bounded(mut reader: impl std::io::Read) -> Vec<u8> {
        let mut retained = Vec::new();
        let mut chunk = [0_u8; 4096];
        while let Ok(read) = reader.read(&mut chunk) {
            if read == 0 {
                break;
            }
            let remaining = MAX_COMMAND_OUTPUT_BYTES.saturating_sub(retained.len());
            retained.extend_from_slice(&chunk[..read.min(remaining)]);
        }
        retained
    }

    let mut child = Command::new("nmcli")
        .args([
            "--wait",
            "25",
            "connection",
            "import",
            "--temporary",
            "type",
            capability.type_name.as_str(),
            "file",
        ])
        .arg(path)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| {
            super::Error::new(
                "import VPN configuration",
                "NetworkManager's import helper could not be started",
            )
        })?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (Some(stdout), Some(stderr)) = (stdout, stderr) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(super::Error::new(
            "import VPN configuration",
            "the import helper lost its bounded output channel",
        ));
    };
    let stdout_reader = std::thread::spawn(move || drain_bounded(stdout));
    let stderr_reader = std::thread::spawn(move || drain_bounded(stderr));
    let deadline = std::time::Instant::now() + IMPORT_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(super::Error::new(
                    "import VPN configuration",
                    "the installed importer did not finish within 30 seconds",
                ));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(super::Error::new(
                    "import VPN configuration",
                    "the installed importer could not be monitored",
                ));
            }
        }
    };
    let mut stdout = stdout_reader.join().unwrap_or_default();
    let mut stderr = stderr_reader.join().unwrap_or_default();
    let result = match status {
        Ok(status) if status.success() => Ok(ImportCommandResult {
            uuid: find_uuid(&stdout),
        }),
        Ok(_) => Err(super::Error::new(
            "import VPN configuration",
            "the installed VPN importer rejected the selected configuration",
        )),
        Err(error) => Err(error),
    };
    stdout.zeroize();
    stderr.zeroize();
    result
}

#[cfg(any(not(target_os = "macos"), test))]
fn find_uuid(bytes: &[u8]) -> Option<String> {
    bytes.windows(36).find_map(|candidate| {
        let valid = candidate.iter().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                *byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        });
        valid.then(|| String::from_utf8_lossy(candidate).to_ascii_lowercase())
    })
}

#[cfg(not(target_os = "macos"))]
fn connection_paths(
    connection: &zbus::blocking::Connection,
) -> Result<Vec<zbus::zvariant::OwnedObjectPath>, super::Error> {
    zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager/Settings",
        "org.freedesktop.NetworkManager.Settings",
    )
    .map_err(|error| super::Error::new("open saved VPN profiles", error.to_string()))?
    .call::<_, _, Vec<zbus::zvariant::OwnedObjectPath>>("ListConnections", &())
    .map_err(|error| super::Error::new("list saved VPN profiles", error.to_string()))
}

#[cfg(not(target_os = "macos"))]
fn find_staged_import(
    connection: &zbus::blocking::Connection,
    capability: &VpnImportCapabilityId,
    new_paths: &[zbus::zvariant::OwnedObjectPath],
    uuid_hint: Option<&str>,
) -> Result<StagedImport, super::Error> {
    let mut candidates = Vec::new();
    for path in new_paths {
        let Ok(proxy) = profile_proxy(connection, path.as_str()) else {
            continue;
        };
        if !proxy.get_property::<bool>("Unsaved").unwrap_or(false) {
            continue;
        }
        let Ok(settings) = proxy.call::<_, _, SettingsMap>("GetSettings", &()) else {
            continue;
        };
        let Some(connection_setting) = settings.get("connection") else {
            continue;
        };
        let Some(uuid) = super::property_string(connection_setting, "uuid") else {
            continue;
        };
        let Some(connection_type) = super::property_string(connection_setting, "type") else {
            continue;
        };
        let service = settings
            .get("vpn")
            .and_then(|vpn| super::property_string(vpn, "service-type"));
        if !import_matches_capability(capability, &connection_type, service.as_deref()) {
            continue;
        }
        let name = super::property_string(connection_setting, "id")
            .filter(|name| !name.is_empty() && name.len() <= 256)
            .unwrap_or_else(|| "Imported VPN".to_string());
        candidates.push((
            uuid.clone(),
            StagedImport {
                id: VpnImportPreviewId {
                    profile: super::VpnProfileId {
                        object_path: path.to_string(),
                        uuid,
                    },
                    settings,
                },
                name,
                service: super::vpn_service_label(&connection_type, service.as_deref()),
            },
        ));
    }
    if candidates.len() > 1 {
        if let Some(hint) = uuid_hint {
            candidates.retain(|(uuid, _)| uuid == hint);
        }
    }
    if candidates.len() != 1 {
        return Err(super::Error::new(
            "identify imported VPN",
            if candidates.is_empty() {
                "NetworkManager did not expose one new temporary profile"
            } else {
                "concurrent profile changes made the temporary import ambiguous"
            },
        ));
    }
    Ok(candidates.remove(0).1)
}

#[cfg(any(not(target_os = "macos"), test))]
fn import_matches_capability(
    capability: &VpnImportCapabilityId,
    connection_type: &str,
    service: Option<&str>,
) -> bool {
    if capability.type_name == "wireguard" {
        return connection_type == "wireguard";
    }
    connection_type == "vpn"
        && service.is_some_and(|service| {
            service == capability.service
                || service.rsplit('.').next() == Some(capability.type_name.as_str())
        })
}

#[cfg(not(target_os = "macos"))]
fn require_staged_autoconnect_disabled(
    connection: &zbus::blocking::Connection,
    preview: &VpnImportPreviewId,
) -> Result<(), super::Error> {
    let _ = exact_staged_profile(connection, preview)?;
    let connection_setting = preview.settings.get("connection").ok_or_else(|| {
        super::Error::new(
            "stage imported VPN",
            "the profile lost its connection settings",
        )
    })?;
    if super::property::<bool>(connection_setting, "autoconnect") != Some(false) {
        delete_exact_staged(connection, preview).map_err(|error| {
            super::Error::new(
                "stage imported VPN",
                format!(
                    "the importer enabled automatic connection and cleanup could not be verified: {error}"
                ),
            )
        })?;
        return Err(super::Error::new(
            "stage imported VPN",
            "the importer did not disable automatic connection; its temporary profile was removed",
        ));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn exact_staged_profile<'a>(
    connection: &'a zbus::blocking::Connection,
    preview: &'a VpnImportPreviewId,
) -> Result<zbus::blocking::Proxy<'a>, super::Error> {
    let proxy = profile_proxy(connection, &preview.profile.object_path)?;
    let settings = proxy
        .call::<_, _, SettingsMap>("GetSettings", &())
        .map_err(|error| super::Error::new("revalidate imported VPN", error.to_string()))?;
    let uuid = settings
        .get("connection")
        .and_then(|setting| super::property_string(setting, "uuid"));
    let unsaved = proxy.get_property::<bool>("Unsaved").unwrap_or(false);
    if uuid.as_deref() != Some(preview.profile.uuid.as_str())
        || settings != preview.settings
        || !unsaved
    {
        return Err(super::Error::new(
            "revalidate imported VPN",
            "the temporary profile changed; it was left untouched",
        ));
    }
    Ok(proxy)
}

#[cfg(not(target_os = "macos"))]
fn exact_profile_settings(
    connection: &zbus::blocking::Connection,
    id: &super::VpnProfileId,
) -> Result<SettingsMap, super::Error> {
    let settings = profile_proxy(connection, &id.object_path)?
        .call::<_, _, SettingsMap>("GetSettings", &())
        .map_err(|error| super::Error::new("read imported VPN", error.to_string()))?;
    if settings
        .get("connection")
        .and_then(|setting| super::property_string(setting, "uuid"))
        .as_deref()
        != Some(id.uuid.as_str())
    {
        return Err(super::Error::new(
            "read imported VPN",
            "the temporary profile identity changed",
        ));
    }
    Ok(settings)
}

#[cfg(not(target_os = "macos"))]
fn delete_exact_staged(
    connection: &zbus::blocking::Connection,
    preview: &VpnImportPreviewId,
) -> Result<(), super::Error> {
    exact_staged_profile(connection, preview)?
        .call::<_, _, ()>("Delete", &())
        .map_err(|error| super::Error::new("remove temporary VPN import", error.to_string()))
}

#[cfg(not(target_os = "macos"))]
fn profile_proxy<'a>(
    connection: &'a zbus::blocking::Connection,
    path: &'a str,
) -> Result<zbus::blocking::Proxy<'a>, super::Error> {
    zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.NetworkManager",
        path,
        "org.freedesktop.NetworkManager.Settings.Connection",
    )
    .map_err(|error| super::Error::new("open imported VPN profile", error.to_string()))
}

#[cfg(target_os = "linux")]
fn linux_capabilities() -> VpnImportCapabilities {
    use std::process::{Command, Stdio};

    let nmcli_available = Command::new("nmcli")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !nmcli_available {
        return VpnImportCapabilities {
            available: false,
            plugins: Vec::new(),
            limitation: Some(
                "NetworkManager's nmcli import helper is not installed on this system".into(),
            ),
        };
    }

    let (mut plugins, limitation) = match unsafe { discover_libnm_plugins() } {
        Ok(plugins) => (plugins, None),
        Err(detail) => (
            Vec::new(),
            Some(format!(
                "Installed VPN plugins could not be inspected ({detail}); native WireGuard import remains available"
            )),
        ),
    };
    plugins.push(capability("wireguard", "wireguard"));
    plugins.sort_by(|left, right| left.name.cmp(&right.name));
    plugins.dedup_by(|left, right| left.id.type_name == right.id.type_name);
    VpnImportCapabilities {
        available: true,
        plugins,
        limitation,
    }
}

#[cfg(target_os = "linux")]
#[repr(C)]
struct GSList {
    data: *mut std::ffi::c_void,
    next: *mut GSList,
}

#[cfg(target_os = "linux")]
unsafe fn discover_libnm_plugins() -> Result<Vec<VpnImportCapability>, String> {
    use libloading::Library;
    use std::ffi::{c_char, c_void, CStr};

    type ListLoad = unsafe extern "C" fn() -> *mut GSList;
    type InfoString = unsafe extern "C" fn(*mut c_void) -> *const c_char;
    type LoadEditor = unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> *mut c_void;
    type Capabilities = unsafe extern "C" fn(*mut c_void) -> u32;
    type Unref = unsafe extern "C" fn(*mut c_void);
    type ListFree = unsafe extern "C" fn(*mut GSList);

    let libnm = Library::new("libnm.so.0").map_err(|error| error.to_string())?;
    let libglib = Library::new("libglib-2.0.so.0").map_err(|error| error.to_string())?;
    let libgobject = Library::new("libgobject-2.0.so.0").map_err(|error| error.to_string())?;
    let list_load: ListLoad = *libnm
        .get(b"nm_vpn_plugin_info_list_load\0")
        .map_err(|error| error.to_string())?;
    let get_name: InfoString = *libnm
        .get(b"nm_vpn_plugin_info_get_name\0")
        .map_err(|error| error.to_string())?;
    let get_service: InfoString = *libnm
        .get(b"nm_vpn_plugin_info_get_service\0")
        .map_err(|error| error.to_string())?;
    let load_editor: LoadEditor = *libnm
        .get(b"nm_vpn_plugin_info_load_editor_plugin\0")
        .map_err(|error| error.to_string())?;
    let get_capabilities: Capabilities = *libnm
        .get(b"nm_vpn_editor_plugin_get_capabilities\0")
        .map_err(|error| error.to_string())?;
    let unref: Unref = *libgobject
        .get(b"g_object_unref\0")
        .map_err(|error| error.to_string())?;
    let list_free: ListFree = *libglib
        .get(b"g_slist_free\0")
        .map_err(|error| error.to_string())?;

    let list = list_load();
    let mut cursor = list;
    let mut plugins = Vec::new();
    let mut visited = 0;
    while !cursor.is_null() && visited < MAX_PLUGIN_COUNT {
        let info = (*cursor).data;
        if !info.is_null() {
            let name_ptr = get_name(info);
            let service_ptr = get_service(info);
            let editor = load_editor(info, std::ptr::null_mut());
            if !name_ptr.is_null() && !service_ptr.is_null() && !editor.is_null() {
                const IMPORT_CAPABILITY: u32 = 0x1;
                if get_capabilities(editor) & IMPORT_CAPABILITY != 0 {
                    let name = CStr::from_ptr(name_ptr).to_string_lossy();
                    let service = CStr::from_ptr(service_ptr).to_string_lossy();
                    if let Some(type_name) = supported_type_name(&name, &service) {
                        plugins.push(capability(type_name, &service));
                    }
                }
            }
            unref(info);
        }
        cursor = (*cursor).next;
        visited += 1;
    }
    list_free(list);
    if !cursor.is_null() {
        return Err("the plugin inventory exceeded its safety bound".into());
    }
    Ok(plugins)
}

#[cfg(any(target_os = "linux", test))]
fn supported_type_name<'a>(name: &'a str, service: &'a str) -> Option<&'a str> {
    let name = name.trim().to_ascii_lowercase();
    let service_type = service
        .trim()
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    SUPPORTED_TYPES
        .iter()
        .copied()
        .find(|candidate| *candidate == name || *candidate == service_type)
}

#[cfg(any(target_os = "linux", test))]
const SUPPORTED_TYPES: &[&str] = &[
    "fortisslvpn",
    "libreswan",
    "l2tp",
    "openconnect",
    "openvpn",
    "sstp",
    "strongswan",
];

#[cfg(any(target_os = "linux", test))]
fn capability(type_name: &str, service: &str) -> VpnImportCapability {
    let (name, format_hint) = match type_name {
        "fortisslvpn" => ("Fortinet SSL VPN", "Fortinet configuration"),
        "libreswan" => ("IPsec (Libreswan)", "IPsec configuration"),
        "l2tp" => ("L2TP over IPsec", "L2TP/IPsec configuration"),
        "openconnect" => ("OpenConnect", "OpenConnect configuration"),
        "openvpn" => ("OpenVPN", "OpenVPN configuration (.ovpn)"),
        "sstp" => ("SSTP", "SSTP configuration"),
        "strongswan" => ("IPsec (strongSwan)", "IPsec configuration"),
        "wireguard" => ("WireGuard", "wg-quick configuration (.conf)"),
        _ => ("VPN", "VPN configuration"),
    };
    VpnImportCapability {
        id: VpnImportCapabilityId {
            type_name: type_name.to_string(),
            service: service.to_string(),
        },
        name: name.to_string(),
        format_hint: format_hint.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_reviewed_secure_plugin_types_are_admitted() {
        assert_eq!(
            supported_type_name("openvpn", "org.freedesktop.NetworkManager.openvpn"),
            Some("openvpn")
        );
        assert_eq!(
            supported_type_name("vendor name", "org.freedesktop.NetworkManager.strongswan"),
            Some("strongswan")
        );
        assert_eq!(
            supported_type_name("pptp", "org.freedesktop.NetworkManager.pptp"),
            None
        );
        assert_eq!(
            supported_type_name("unknown", "org.example.NetworkManager.unknown"),
            None
        );
    }

    #[test]
    fn capability_identity_does_not_expose_command_arguments() {
        let capability = capability("openvpn", "org.freedesktop.NetworkManager.openvpn");
        let debug = format!("{:?}", capability.id);
        assert_eq!(
            debug,
            "VpnImportCapabilityId { type: \"<opaque>\", service: \"<opaque>\" }"
        );
        assert!(!debug.contains("openvpn"));
    }

    #[test]
    fn import_identity_requires_the_exact_reviewed_connection_type() {
        let openvpn = capability("openvpn", "org.freedesktop.NetworkManager.openvpn");
        assert!(import_matches_capability(
            &openvpn.id,
            "vpn",
            Some("org.freedesktop.NetworkManager.openvpn")
        ));
        assert!(!import_matches_capability(
            &openvpn.id,
            "vpn",
            Some("org.freedesktop.NetworkManager.openconnect")
        ));
        assert!(!import_matches_capability(&openvpn.id, "wireguard", None));

        let wireguard = capability("wireguard", "wireguard");
        assert!(import_matches_capability(&wireguard.id, "wireguard", None));
        assert!(!import_matches_capability(&wireguard.id, "vpn", None));
    }

    #[test]
    fn command_uuid_hint_is_parsed_without_human_output_fields() {
        let uuid = "12345678-90ab-cdef-1234-567890abcdef";
        assert_eq!(
            find_uuid(format!("Connection ({uuid}) added").as_bytes()).as_deref(),
            Some(uuid)
        );
        assert!(find_uuid(b"not-a-uuid").is_none());
    }
}
