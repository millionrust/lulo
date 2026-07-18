//! Linux portal privacy decisions backed by the XDG PermissionStore.

use rmac_privacy::{
    AutomaticUpdates, PackageSources, PortalDecision, PortalResource, ProStatus, ReleaseSupport,
    SecurityCoverageSnapshot, Snapshot,
};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedValue;

const DESTINATION: &str = "org.freedesktop.impl.portal.PermissionStore";
const PATH: &str = "/org/freedesktop/impl/portal/PermissionStore";
const INTERFACE: &str = "org.freedesktop.impl.portal.PermissionStore";
const DEVICE_TABLE: &str = "devices";
const NOT_FOUND: &str = "org.freedesktop.portal.Error.NotFound";
const RESOURCES: [PortalResource; 2] = [PortalResource::Camera, PortalResource::Microphone];
const MAX_HELPER_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_DECISIONS: usize = 512;
const MAX_PERMISSIONS_PER_DECISION: usize = 32;
const MAX_PERMISSION_TOKEN_BYTES: usize = 256;
const MAX_PERMISSION_SUMMARY_BYTES: usize = 512;
const MAX_ERROR_BYTES: usize = 512;
const MAX_SECURITY_LIST_ITEMS: usize = 64;
const HELPER_TIMEOUT: Duration = Duration::from_secs(15);
#[cfg(target_os = "linux")]
const RECONNECT_DELAY: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    operation: &'static str,
    detail: String,
}

impl Error {
    fn new(operation: &'static str, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: bounded_text(&detail.into(), MAX_ERROR_BYTES),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.operation, self.detail)
    }
}

impl std::error::Error for Error {}

enum LookupError {
    NotFound,
    Failed(String),
}

trait Store {
    fn version(&self) -> Result<u32, String>;
    fn lookup(&self, resource: PortalResource)
        -> Result<HashMap<String, Vec<String>>, LookupError>;
    fn get_permission(
        &self,
        resource: PortalResource,
        app_id: &str,
    ) -> Result<Vec<String>, LookupError>;
    fn delete_permission(&self, resource: PortalResource, app_id: &str) -> Result<(), String>;
}

impl Store for Proxy<'_> {
    fn version(&self) -> Result<u32, String> {
        self.get_property("version")
            .map_err(|_| "PermissionStore did not provide its interface version".to_string())
    }

    fn lookup(
        &self,
        resource: PortalResource,
    ) -> Result<HashMap<String, Vec<String>>, LookupError> {
        self.call::<_, _, (HashMap<String, Vec<String>>, OwnedValue)>(
            "Lookup",
            &(DEVICE_TABLE, resource.id()),
        )
        .map(|(permissions, _)| permissions)
        .map_err(|error| match error {
            zbus::Error::MethodError(name, _, _) if name.as_str() == NOT_FOUND => {
                LookupError::NotFound
            }
            _ => LookupError::Failed(
                "PermissionStore could not read a device decision table".to_string(),
            ),
        })
    }

    fn get_permission(
        &self,
        resource: PortalResource,
        app_id: &str,
    ) -> Result<Vec<String>, LookupError> {
        self.call("GetPermission", &(DEVICE_TABLE, resource.id(), app_id))
            .map_err(|error| match error {
                zbus::Error::MethodError(name, _, _) if name.as_str() == NOT_FOUND => {
                    LookupError::NotFound
                }
                _ => LookupError::Failed(
                    "PermissionStore could not read the selected decision".to_string(),
                ),
            })
    }

    fn delete_permission(&self, resource: PortalResource, app_id: &str) -> Result<(), String> {
        self.call("DeletePermission", &(DEVICE_TABLE, resource.id(), app_id))
            .map_err(|_| "PermissionStore rejected the decision reset".to_string())
    }
}

pub fn snapshot() -> Result<Snapshot, Error> {
    let connection = match Connection::session() {
        Ok(connection) => connection,
        Err(_) => {
            return Ok(Snapshot {
                detail: Some("The session D-Bus is unavailable.".into()),
                ..Snapshot::default()
            });
        }
    };
    let proxy = match Proxy::new(&connection, DESTINATION, PATH, INTERFACE) {
        Ok(proxy) => proxy,
        Err(_) => {
            return Ok(Snapshot {
                detail: Some("The portal PermissionStore is unavailable.".into()),
                ..Snapshot::default()
            });
        }
    };
    snapshot_with(&proxy)
}

pub fn reset_decision(expected: &PortalDecision) -> Result<Snapshot, Error> {
    validate_decision(expected)?;
    let connection = Connection::session().map_err(|_| {
        Error::new(
            "connect to the portal PermissionStore",
            "session D-Bus is unavailable",
        )
    })?;
    let proxy = Proxy::new(&connection, DESTINATION, PATH, INTERFACE).map_err(|_| {
        Error::new(
            "open the portal PermissionStore",
            "PermissionStore is unavailable",
        )
    })?;
    reset_with(&proxy, expected)
}

#[cfg(target_os = "linux")]
pub async fn watch(sender: async_channel::Sender<rmac_privacy::WatchEvent>) -> Result<(), Error> {
    loop {
        match watch_once(&sender).await {
            Ok(()) if sender.is_closed() => return Ok(()),
            Ok(()) => {}
            Err(_) if sender.is_closed() => return Ok(()),
            Err(_) => {
                if sender
                    .send(rmac_privacy::WatchEvent::Unavailable)
                    .await
                    .is_err()
                {
                    return Ok(());
                }
            }
        }
        async_io::Timer::after(RECONNECT_DELAY).await;
    }
}

#[cfg(not(target_os = "linux"))]
pub async fn watch(sender: async_channel::Sender<rmac_privacy::WatchEvent>) -> Result<(), Error> {
    sender
        .send(rmac_privacy::WatchEvent::Unavailable)
        .await
        .map_err(|_| Error::new("watch portal permissions", "the watcher closed"))
}

#[cfg(target_os = "linux")]
async fn watch_once(sender: &async_channel::Sender<rmac_privacy::WatchEvent>) -> Result<(), Error> {
    use futures_util::{FutureExt as _, StreamExt as _};
    use zbus::{message::Type, MatchRule, MessageStream};

    let connection = zbus::Connection::session()
        .await
        .map_err(|error| Error::new("watch portal permissions", error.to_string()))?;
    let changed_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(DESTINATION)
        .map_err(|error| Error::new("watch portal permissions", error.to_string()))?
        .path(PATH)
        .map_err(|error| Error::new("watch portal permissions", error.to_string()))?
        .interface(INTERFACE)
        .map_err(|error| Error::new("watch portal permissions", error.to_string()))?
        .member("Changed")
        .map_err(|error| Error::new("watch portal permissions", error.to_string()))?
        .build();
    let owner_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")
        .map_err(|error| Error::new("watch portal permissions", error.to_string()))?
        .interface("org.freedesktop.DBus")
        .map_err(|error| Error::new("watch portal permissions", error.to_string()))?
        .member("NameOwnerChanged")
        .map_err(|error| Error::new("watch portal permissions", error.to_string()))?
        .add_arg(DESTINATION)
        .map_err(|error| Error::new("watch portal permissions", error.to_string()))?
        .build();
    let mut changes = MessageStream::for_match_rule(changed_rule, &connection, Some(16))
        .await
        .map_err(|error| Error::new("watch portal permissions", error.to_string()))?
        .fuse();
    let mut owners = MessageStream::for_match_rule(owner_rule, &connection, Some(4))
        .await
        .map_err(|error| Error::new("watch portal permissions", error.to_string()))?
        .fuse();

    sender
        .send(rmac_privacy::WatchEvent::Changed)
        .await
        .map_err(|_| Error::new("watch portal permissions", "the watcher closed"))?;

    loop {
        let closed = sender.closed().fuse();
        futures_util::pin_mut!(closed);
        let event = futures_util::select! {
            message = changes.next() => {
                message
                    .ok_or_else(|| Error::new("watch portal permissions", "the change stream ended"))?
                    .map_err(|error| Error::new("watch portal permissions", error.to_string()))?;
                rmac_privacy::WatchEvent::Changed
            },
            message = owners.next() => owner_event_from_message(message)?,
            _ = closed => return Ok(()),
        };
        let _ = sender.try_send(event);
    }
}

#[cfg(target_os = "linux")]
fn owner_event_from_message(
    message: Option<Result<zbus::Message, zbus::Error>>,
) -> Result<rmac_privacy::WatchEvent, Error> {
    let message = message
        .ok_or_else(|| Error::new("watch portal permissions", "the owner stream ended"))?
        .map_err(|error| Error::new("watch portal permissions", error.to_string()))?;
    let (name, _old_owner, new_owner): (String, String, String) = message
        .body()
        .deserialize()
        .map_err(|error| Error::new("watch portal permissions", error.to_string()))?;
    permission_store_owner_event(&name, &new_owner)
        .ok_or_else(|| Error::new("watch portal permissions", "an unrelated owner changed"))
}

#[cfg(any(target_os = "linux", test))]
fn permission_store_owner_event(name: &str, new_owner: &str) -> Option<rmac_privacy::WatchEvent> {
    (name == DESTINATION).then_some(if new_owner.is_empty() {
        rmac_privacy::WatchEvent::Unavailable
    } else {
        rmac_privacy::WatchEvent::Changed
    })
}

trait SecurityRunner {
    fn api(&self, endpoint: &'static str) -> Result<Vec<u8>, String>;
    fn release_days(&self, series: &str) -> Result<i64, String>;
}

struct SystemSecurityRunner;

impl SecurityRunner for SystemSecurityRunner {
    fn api(&self, endpoint: &'static str) -> Result<Vec<u8>, String> {
        run_bounded("pro", &["api", endpoint], "Ubuntu Pro Client")
    }

    fn release_days(&self, series: &str) -> Result<i64, String> {
        let output = run_bounded(
            "ubuntu-distro-info",
            &["--series", series, "--days=eol"],
            "ubuntu-distro-info",
        )?;
        let days = String::from_utf8(output)
            .map_err(|_| "ubuntu-distro-info returned non-UTF-8 output".to_string())?;
        days.trim()
            .parse()
            .map_err(|_| "ubuntu-distro-info returned an invalid EOL day count".to_string())
    }
}

fn run_bounded(program: &str, arguments: &[&str], label: &str) -> Result<Vec<u8>, String> {
    let mut child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => format!("{label} is not installed"),
            std::io::ErrorKind::PermissionDenied => {
                format!("permission was denied while starting {label}")
            }
            _ => format!("{label} could not be started"),
        })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("could not capture {label} output"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| format!("could not capture {label} errors"))?;
    let stdout_reader = std::thread::spawn(move || read_bounded(stdout));
    let stderr_reader = std::thread::spawn(move || read_bounded(stderr));
    let deadline = Instant::now() + HELPER_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{label} could not be inspected"));
            }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{label} timed out after 15 seconds"));
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let (stdout, stdout_excessive) = stdout_reader
        .join()
        .map_err(|_| format!("{label} output reader failed"))??;
    let (_, stderr_excessive) = stderr_reader
        .join()
        .map_err(|_| format!("{label} error reader failed"))??;
    if stdout_excessive || stderr_excessive {
        return Err(format!("{label} output exceeded the 1 MiB safety limit"));
    }
    if !status.success() {
        return Err(format!("{label} reported a failure"));
    }
    Ok(stdout)
}

fn read_bounded(mut reader: impl Read) -> Result<(Vec<u8>, bool), String> {
    let mut output = Vec::new();
    reader
        .by_ref()
        .take((MAX_HELPER_OUTPUT_BYTES + 1) as u64)
        .read_to_end(&mut output)
        .map_err(|_| "could not read helper output".to_string())?;
    let excessive = output.len() > MAX_HELPER_OUTPUT_BYTES;
    output.truncate(MAX_HELPER_OUTPUT_BYTES);
    std::io::copy(&mut reader, &mut std::io::sink())
        .map_err(|_| "could not drain helper output".to_string())?;
    Ok((output, excessive))
}

fn ubuntu_series() -> Result<String, String> {
    let os_release = std::fs::read_to_string("/etc/os-release")
        .map_err(|_| "could not read /etc/os-release".to_string())?;
    let fields = os_release
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key, value.trim_matches(['\'', '"'])))
        .collect::<HashMap<_, _>>();
    if fields.get("ID").copied() != Some("ubuntu") {
        return Err("the installed operating system is not identified as Ubuntu".into());
    }
    let series = fields
        .get("VERSION_CODENAME")
        .copied()
        .filter(|series| {
            !series.is_empty()
                && series.len() <= 32
                && series
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
        .ok_or_else(|| "/etc/os-release omitted a valid VERSION_CODENAME".to_string())?;
    Ok(series.to_string())
}

pub fn security_coverage_snapshot() -> SecurityCoverageSnapshot {
    security_coverage_with(&SystemSecurityRunner, ubuntu_series())
}

fn security_coverage_with(
    runner: &impl SecurityRunner,
    series: Result<String, String>,
) -> SecurityCoverageSnapshot {
    let mut snapshot = SecurityCoverageSnapshot::default();

    match series.and_then(|series| {
        runner
            .release_days(&series)
            .map(|days_remaining| ReleaseSupport {
                series,
                days_remaining,
            })
    }) {
        Ok(release_support) => snapshot.release_support = Some(release_support),
        Err(error) => snapshot
            .issues
            .push(format!("Ubuntu release lifecycle: {error}")),
    }

    match api_attributes(runner, "u.pro.packages.summary.v1").and_then(parse_package_sources) {
        Ok(sources) => {
            snapshot.pro_client_available = true;
            snapshot.package_sources = Some(sources);
        }
        Err(error) => snapshot.issues.push(format!("Package sources: {error}")),
    }

    match api_attributes(runner, "u.pro.status.is_attached.v1").and_then(parse_pro_attachment) {
        Ok(pro) => {
            snapshot.pro_client_available = true;
            snapshot.pro = Some(pro);
        }
        Err(error) => snapshot.issues.push(format!("Ubuntu Pro status: {error}")),
    }
    match api_attributes(runner, "u.pro.status.enabled_services.v1")
        .and_then(parse_enabled_services)
    {
        Ok(services) => {
            snapshot.pro_client_available = true;
            if let Some(pro) = &mut snapshot.pro {
                pro.enabled_services = services;
            } else {
                snapshot
                    .issues
                    .push("Ubuntu Pro services: attachment status is unavailable".into());
            }
        }
        Err(error) => snapshot
            .issues
            .push(format!("Ubuntu Pro services: {error}")),
    }

    match api_attributes(runner, "u.unattended_upgrades.status.v1")
        .and_then(parse_automatic_updates)
    {
        Ok(automatic_updates) => {
            snapshot.pro_client_available = true;
            snapshot.automatic_updates = Some(automatic_updates);
        }
        Err(error) => snapshot
            .issues
            .push(format!("Automatic security updates: {error}")),
    }

    snapshot.issues.sort();
    snapshot.issues.dedup();
    snapshot
}

fn api_attributes(runner: &impl SecurityRunner, endpoint: &'static str) -> Result<Value, String> {
    let output = runner.api(endpoint)?;
    let envelope: Value = serde_json::from_slice(&output)
        .map_err(|error| format!("invalid JSON from Ubuntu Pro Client: {error}"))?;
    if envelope.get("result").and_then(Value::as_str) != Some("success") {
        return Err(api_error_summary(&envelope));
    }
    envelope
        .pointer("/data/attributes")
        .cloned()
        .ok_or_else(|| "Ubuntu Pro Client response omitted data.attributes".into())
}

fn api_error_summary(_envelope: &Value) -> String {
    "Ubuntu Pro Client reported a failed result".to_string()
}

fn parse_package_sources(attributes: Value) -> Result<PackageSources, String> {
    let summary = attributes
        .get("summary")
        .ok_or_else(|| "response omitted summary".to_string())?;
    Ok(PackageSources {
        installed: unsigned(summary, "num_installed_packages")?,
        main: unsigned(summary, "num_main_packages")?,
        restricted: unsigned(summary, "num_restricted_packages")?,
        universe: unsigned(summary, "num_universe_packages")?,
        multiverse: unsigned(summary, "num_multiverse_packages")?,
        esm_apps: unsigned(summary, "num_esm_apps_packages")?,
        esm_infra: unsigned(summary, "num_esm_infra_packages")?,
        third_party: unsigned(summary, "num_third_party_packages")?,
        unknown: unsigned(summary, "num_unknown_packages")?,
    })
}

fn parse_pro_attachment(attached: Value) -> Result<ProStatus, String> {
    let contract_status = attached
        .get("contract_status")
        .and_then(Value::as_str)
        .map(|status| validated_text(status, 64, "contract status"))
        .transpose()?;
    Ok(ProStatus {
        attached: boolean(&attached, "is_attached")?,
        contract_valid: boolean(&attached, "is_attached_and_contract_valid")?,
        contract_status,
        contract_remaining_days: attached
            .get("contract_remaining_days")
            .and_then(Value::as_i64)
            .ok_or_else(|| "response omitted contract_remaining_days".to_string())?,
        enabled_services: Vec::new(),
    })
}

fn parse_enabled_services(services: Value) -> Result<Vec<String>, String> {
    let services = services
        .get("enabled_services")
        .and_then(Value::as_array)
        .ok_or_else(|| "response omitted enabled_services".to_string())?;
    if services.len() > MAX_SECURITY_LIST_ITEMS {
        return Err("response contained too many enabled services".to_string());
    }
    services
        .iter()
        .map(|service| {
            service
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| "response omitted a service name".to_string())
                .and_then(|name| validated_text(name, 128, "service name"))
        })
        .collect::<Result<Vec<_>, _>>()
}

fn parse_automatic_updates(attributes: Value) -> Result<AutomaticUpdates, String> {
    let allowed_origin_values = attributes
        .get("unattended_upgrades_allowed_origins")
        .and_then(Value::as_array)
        .ok_or_else(|| "response omitted unattended_upgrades_allowed_origins".to_string())?;
    if allowed_origin_values.len() > MAX_SECURITY_LIST_ITEMS {
        return Err("response contained too many allowed origins".to_string());
    }
    let allowed_origins = allowed_origin_values
        .iter()
        .map(|origin| {
            origin
                .as_str()
                .ok_or_else(|| "response contained an invalid allowed origin".to_string())
                .and_then(|origin| validated_text(origin, 256, "allowed origin"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let disabled_reason = attributes
        .pointer("/unattended_upgrades_disabled_reason/msg")
        .and_then(Value::as_str)
        .map(|reason| validated_text(reason, 256, "disabled reason"))
        .transpose()?;
    let last_run = attributes
        .get("unattended_upgrades_last_run")
        .and_then(Value::as_str)
        .map(|value| validated_text(value, 128, "last-run value"))
        .transpose()?;
    Ok(AutomaticUpdates {
        running: boolean(&attributes, "unattended_upgrades_running")?,
        apt_timer_enabled: boolean(&attributes, "systemd_apt_timer_enabled")?,
        periodic_job_enabled: boolean(&attributes, "apt_periodic_job_enabled")?,
        package_list_frequency_days: unsigned(&attributes, "package_lists_refresh_frequency_days")?,
        upgrade_frequency_days: unsigned(&attributes, "unattended_upgrades_frequency_days")?,
        allowed_origins,
        last_run,
        disabled_reason,
    })
}

fn unsigned(value: &Value, key: &str) -> Result<u64, String> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("response omitted {key}"))
}

fn boolean(value: &Value, key: &str) -> Result<bool, String> {
    value
        .get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("response omitted {key}"))
}

fn validated_text(value: &str, maximum_bytes: usize, label: &str) -> Result<String, String> {
    if value.is_empty() || value.len() > maximum_bytes || value.chars().any(char::is_control) {
        return Err(format!("response contained an invalid {label}"));
    }
    Ok(value.to_string())
}

fn bounded_text(value: &str, maximum_bytes: usize) -> String {
    let normalized = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let mut end = normalized.len().min(maximum_bytes);
    while !normalized.is_char_boundary(end) {
        end -= 1;
    }
    normalized[..end].trim().to_string()
}

fn snapshot_with(store: &impl Store) -> Result<Snapshot, Error> {
    let version = store
        .version()
        .map_err(|error| Error::new("read PermissionStore version", error))?;
    let mut decisions = Vec::new();
    for resource in RESOURCES {
        match store.lookup(resource) {
            Ok(entries) => {
                if decisions.len().saturating_add(entries.len()) > MAX_DECISIONS {
                    return Err(Error::new(
                        "read portal device permissions",
                        "PermissionStore returned too many decisions",
                    ));
                }
                for (app_id, permissions) in entries {
                    let decision = PortalDecision {
                        resource,
                        app_id,
                        permissions,
                    };
                    validate_decision(&decision)?;
                    decisions.push(decision);
                }
            }
            Err(LookupError::NotFound) => {}
            Err(LookupError::Failed(error)) => {
                return Err(Error::new("read portal device permissions", error));
            }
        }
    }
    decisions
        .sort_by(|left, right| (left.resource, &left.app_id).cmp(&(right.resource, &right.app_id)));
    Ok(Snapshot {
        available: true,
        version,
        can_reset: version >= 2,
        decisions,
        detail: (version < 2).then(|| {
            "PermissionStore version 2 is required to reset individual decisions".to_string()
        }),
    })
}

fn reset_with(store: &impl Store, expected: &PortalDecision) -> Result<Snapshot, Error> {
    validate_decision(expected)?;
    let version = store
        .version()
        .map_err(|error| Error::new("read PermissionStore version", error))?;
    if version < 2 {
        return Err(Error::new(
            "reset portal permission",
            "PermissionStore version 2 is required",
        ));
    }
    let current_permissions = match store.get_permission(expected.resource, &expected.app_id) {
        Ok(permissions) => permissions,
        Err(LookupError::NotFound) => {
            return Err(Error::new(
                "reset portal permission",
                "the selected decision no longer exists; refresh and try again",
            ));
        }
        Err(LookupError::Failed(_)) => {
            return Err(Error::new(
                "reset portal permission",
                "the selected decision could not be revalidated",
            ));
        }
    };
    let current = PortalDecision {
        resource: expected.resource,
        app_id: expected.app_id.clone(),
        permissions: current_permissions,
    };
    validate_decision(&current)?;
    if current != *expected {
        return Err(Error::new(
            "reset portal permission",
            "the selected decision changed before reset; refresh and try again",
        ));
    }
    store
        .delete_permission(expected.resource, &expected.app_id)
        .map_err(|error| Error::new("reset portal permission", error))?;
    let snapshot = snapshot_with(store)?;
    if snapshot.decisions.iter().any(|decision| {
        decision.resource == expected.resource && decision.app_id == expected.app_id
    }) {
        return Err(Error::new(
            "reset portal permission",
            "the selected decision remained after reset",
        ));
    }
    Ok(snapshot)
}

fn validate_app_id(app_id: &str) -> Result<(), Error> {
    if app_id.is_empty() || app_id.len() > 255 || app_id.chars().any(char::is_control) {
        return Err(Error::new(
            "validate portal application ID",
            "the application ID is empty, too long, or contains control characters",
        ));
    }
    Ok(())
}

fn validate_decision(decision: &PortalDecision) -> Result<(), Error> {
    validate_app_id(&decision.app_id)?;
    if decision.permissions.len() > MAX_PERMISSIONS_PER_DECISION
        || decision
            .permissions
            .iter()
            .map(String::len)
            .fold(0_usize, usize::saturating_add)
            .saturating_add(decision.permissions.len().saturating_sub(1) * 2)
            > MAX_PERMISSION_SUMMARY_BYTES
        || decision.permissions.iter().any(|permission| {
            permission.len() > MAX_PERMISSION_TOKEN_BYTES
                || permission.chars().any(char::is_control)
        })
    {
        return Err(Error::new(
            "validate portal decision",
            "the permission tokens exceed the safe display bounds",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::RefCell;

    struct FakeStore {
        version: u32,
        entries: RefCell<HashMap<PortalResource, HashMap<String, Vec<String>>>>,
        deleted: RefCell<Vec<(PortalResource, String)>>,
        delete_effective: bool,
    }

    impl Store for FakeStore {
        fn version(&self) -> Result<u32, String> {
            Ok(self.version)
        }

        fn lookup(
            &self,
            resource: PortalResource,
        ) -> Result<HashMap<String, Vec<String>>, LookupError> {
            self.entries
                .borrow()
                .get(&resource)
                .cloned()
                .ok_or(LookupError::NotFound)
        }

        fn get_permission(
            &self,
            resource: PortalResource,
            app_id: &str,
        ) -> Result<Vec<String>, LookupError> {
            self.entries
                .borrow()
                .get(&resource)
                .and_then(|entries| entries.get(app_id))
                .cloned()
                .ok_or(LookupError::NotFound)
        }

        fn delete_permission(&self, resource: PortalResource, app_id: &str) -> Result<(), String> {
            self.deleted
                .borrow_mut()
                .push((resource, app_id.to_string()));
            if self.delete_effective {
                if let Some(entries) = self.entries.borrow_mut().get_mut(&resource) {
                    entries.remove(app_id);
                }
            }
            Ok(())
        }
    }

    fn store(version: u32) -> FakeStore {
        FakeStore {
            version,
            entries: RefCell::new(HashMap::from([
                (
                    PortalResource::Camera,
                    HashMap::from([("org.example.Camera".into(), vec!["yes".into()])]),
                ),
                (
                    PortalResource::Microphone,
                    HashMap::from([("org.example.Chat".into(), vec!["no".into()])]),
                ),
            ])),
            deleted: RefCell::new(Vec::new()),
            delete_effective: true,
        }
    }

    #[test]
    fn snapshot_is_sorted_and_preserves_raw_permission_tokens() {
        let snapshot = snapshot_with(&store(2)).unwrap();
        assert!(snapshot.available);
        assert!(snapshot.can_reset);
        assert_eq!(snapshot.decisions.len(), 2);
        assert_eq!(snapshot.decisions[0].resource, PortalResource::Camera);
        assert_eq!(snapshot.decisions[0].permissions, ["yes"]);
    }

    #[test]
    fn version_one_is_visible_but_not_resettable() {
        let snapshot = snapshot_with(&store(1)).unwrap();
        assert!(snapshot.available);
        assert!(!snapshot.can_reset);
        assert!(snapshot.detail.unwrap().contains("version 2"));
    }

    #[test]
    fn reset_deletes_only_the_selected_app_resource_pair_and_resamples() {
        let store = store(2);
        let expected = snapshot_with(&store).unwrap().decisions[0].clone();
        let snapshot = reset_with(&store, &expected).unwrap();
        assert_eq!(
            store.deleted.borrow().as_slice(),
            &[(PortalResource::Camera, "org.example.Camera".into())]
        );
        assert_eq!(snapshot.decisions.len(), 1);
        assert_eq!(snapshot.decisions[0].resource, PortalResource::Microphone);
    }

    #[test]
    fn reset_requires_version_two_and_a_bounded_app_id() {
        let expected = PortalDecision {
            resource: PortalResource::Camera,
            app_id: "org.example.Camera".into(),
            permissions: vec!["yes".into()],
        };
        assert!(reset_with(&store(1), &expected).is_err());
        assert!(validate_app_id("").is_err());
        assert!(validate_app_id("org.example\nBad").is_err());
    }

    #[test]
    fn reset_refuses_a_decision_changed_after_confirmation() {
        let store = store(2);
        let mut expected = snapshot_with(&store).unwrap().decisions[0].clone();
        expected.permissions = vec!["no".into()];
        let error = reset_with(&store, &expected).unwrap_err();
        assert!(error.to_string().contains("changed before reset"));
        assert!(store.deleted.borrow().is_empty());
    }

    #[test]
    fn reset_requires_authoritative_absence_after_delete() {
        let mut store = store(2);
        store.delete_effective = false;
        let expected = snapshot_with(&store).unwrap().decisions[0].clone();
        let error = reset_with(&store, &expected).unwrap_err();
        assert!(error.to_string().contains("remained after reset"));
    }

    #[test]
    fn snapshot_rejects_unbounded_or_control_bearing_decisions() {
        let store = store(2);
        store
            .entries
            .borrow_mut()
            .get_mut(&PortalResource::Camera)
            .unwrap()
            .insert("org.example\nBad".into(), vec!["yes".into()]);
        assert!(snapshot_with(&store).is_err());
    }

    struct FakeSecurityRunner {
        responses: HashMap<&'static str, Result<Value, String>>,
        release_days: Result<i64, String>,
    }

    impl SecurityRunner for FakeSecurityRunner {
        fn api(&self, endpoint: &'static str) -> Result<Vec<u8>, String> {
            let attributes = self
                .responses
                .get(endpoint)
                .ok_or_else(|| format!("unexpected endpoint {endpoint}"))?
                .clone()?;
            serde_json::to_vec(&json!({
                "result": "success",
                "data": { "attributes": attributes },
                "errors": []
            }))
            .map_err(|error| error.to_string())
        }

        fn release_days(&self, _series: &str) -> Result<i64, String> {
            self.release_days.clone()
        }
    }

    fn complete_pro_runner() -> FakeSecurityRunner {
        FakeSecurityRunner {
            responses: HashMap::from([
                (
                    "u.pro.packages.summary.v1",
                    Ok(json!({
                        "summary": {
                            "num_installed_packages": 100,
                            "num_esm_apps_packages": 2,
                            "num_esm_infra_packages": 3,
                            "num_main_packages": 40,
                            "num_multiverse_packages": 5,
                            "num_restricted_packages": 10,
                            "num_third_party_packages": 7,
                            "num_universe_packages": 30,
                            "num_unknown_packages": 3
                        }
                    })),
                ),
                (
                    "u.pro.status.is_attached.v1",
                    Ok(json!({
                        "contract_remaining_days": 360,
                        "contract_status": "active",
                        "is_attached": true,
                        "is_attached_and_contract_valid": true
                    })),
                ),
                (
                    "u.pro.status.enabled_services.v1",
                    Ok(json!({
                        "enabled_services": [
                            {"name": "esm-apps", "variant_enabled": false, "variant_name": null},
                            {"name": "esm-infra", "variant_enabled": false, "variant_name": null}
                        ]
                    })),
                ),
                (
                    "u.unattended_upgrades.status.v1",
                    Ok(json!({
                        "apt_periodic_job_enabled": true,
                        "package_lists_refresh_frequency_days": 1,
                        "systemd_apt_timer_enabled": true,
                        "unattended_upgrades_allowed_origins": ["${distro_id}:${distro_codename}-security"],
                        "unattended_upgrades_disabled_reason": null,
                        "unattended_upgrades_frequency_days": 1,
                        "unattended_upgrades_last_run": "2026-07-13T08:30:00Z",
                        "unattended_upgrades_running": true
                    })),
                ),
            ]),
            release_days: Ok(1_750),
        }
    }

    #[test]
    fn security_coverage_keeps_authorities_separate() {
        let snapshot = security_coverage_with(&complete_pro_runner(), Ok("resolute".into()));
        assert!(snapshot.pro_client_available);
        assert!(snapshot.issues.is_empty());
        assert_eq!(snapshot.release_support.unwrap().series, "resolute");
        assert_eq!(snapshot.package_sources.unwrap().third_party, 7);
        let pro = snapshot.pro.unwrap();
        assert!(pro.contract_valid);
        assert_eq!(pro.enabled_services, ["esm-apps", "esm-infra"]);
        assert!(snapshot.automatic_updates.unwrap().fully_enabled());
    }

    #[test]
    fn one_failed_endpoint_does_not_hide_other_security_authorities() {
        let mut runner = complete_pro_runner();
        runner.responses.insert(
            "u.pro.packages.summary.v1",
            Err("endpoint unavailable".into()),
        );
        let snapshot = security_coverage_with(&runner, Ok("resolute".into()));
        assert!(snapshot.package_sources.is_none());
        assert!(snapshot.pro.is_some());
        assert!(snapshot.automatic_updates.is_some());
        assert_eq!(snapshot.issues.len(), 1);
    }

    #[test]
    fn unavailable_service_list_keeps_contract_status() {
        let mut runner = complete_pro_runner();
        runner.responses.insert(
            "u.pro.status.enabled_services.v1",
            Err("endpoint unavailable".into()),
        );
        let snapshot = security_coverage_with(&runner, Ok("resolute".into()));
        assert!(snapshot.pro.unwrap().contract_valid);
        assert!(snapshot
            .issues
            .iter()
            .any(|issue| issue.starts_with("Ubuntu Pro services:")));
    }

    #[test]
    fn unavailable_release_lifecycle_keeps_package_and_update_status() {
        let snapshot = security_coverage_with(
            &complete_pro_runner(),
            Err("not an Ubuntu installation".into()),
        );
        assert!(snapshot.release_support.is_none());
        assert!(snapshot.package_sources.is_some());
        assert!(snapshot.automatic_updates.is_some());
        assert!(snapshot
            .issues
            .iter()
            .any(|issue| issue.starts_with("Ubuntu release lifecycle:")));
    }

    #[test]
    fn control_bearing_security_status_is_not_renderable() {
        let mut runner = complete_pro_runner();
        runner.responses.insert(
            "u.pro.status.is_attached.v1",
            Ok(json!({
                "contract_remaining_days": 1,
                "contract_status": "active\nprivate",
                "is_attached": true,
                "is_attached_and_contract_valid": false
            })),
        );
        let snapshot = security_coverage_with(&runner, Ok("resolute".into()));
        assert!(snapshot.pro.is_none());
        assert!(snapshot.issues.iter().any(
            |issue| issue == "Ubuntu Pro status: response contained an invalid contract status"
        ));
    }

    #[test]
    fn public_errors_are_bounded_and_control_normalized() {
        let error = Error::new("test", format!("{}\nprivate", "x".repeat(600)));
        assert!(error.to_string().len() <= MAX_ERROR_BYTES + "test: ".len());
        assert!(!error.to_string().contains('\n'));
    }

    #[test]
    fn permission_store_owner_changes_distinguish_loss_and_reappearance() {
        assert_eq!(
            permission_store_owner_event(DESTINATION, ""),
            Some(rmac_privacy::WatchEvent::Unavailable)
        );
        assert_eq!(
            permission_store_owner_event(DESTINATION, ":1.42"),
            Some(rmac_privacy::WatchEvent::Changed)
        );
        assert_eq!(
            permission_store_owner_event("org.example.Other", ":1.7"),
            None
        );
    }
}
