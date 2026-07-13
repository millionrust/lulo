//! Linux portal privacy decisions backed by the XDG PermissionStore.

use rmac_privacy::{
    AutomaticUpdates, PackageSources, PortalDecision, PortalResource, ProStatus,
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
const MAX_PRO_OUTPUT_BYTES: usize = 1024 * 1024;
const PRO_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    operation: &'static str,
    detail: String,
}

impl Error {
    fn new(operation: &'static str, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: detail.into(),
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
    fn delete_permission(&self, resource: PortalResource, app_id: &str) -> Result<(), String>;
}

impl Store for Proxy<'_> {
    fn version(&self) -> Result<u32, String> {
        self.get_property("version")
            .map_err(|error| error.to_string())
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
            error => LookupError::Failed(error.to_string()),
        })
    }

    fn delete_permission(&self, resource: PortalResource, app_id: &str) -> Result<(), String> {
        self.call("DeletePermission", &(DEVICE_TABLE, resource.id(), app_id))
            .map_err(|error| error.to_string())
    }
}

pub fn snapshot() -> Result<Snapshot, Error> {
    let connection = match Connection::session() {
        Ok(connection) => connection,
        Err(error) => {
            return Ok(Snapshot {
                detail: Some(format!("The session D-Bus is unavailable: {error}")),
                ..Snapshot::default()
            });
        }
    };
    let proxy = match Proxy::new(&connection, DESTINATION, PATH, INTERFACE) {
        Ok(proxy) => proxy,
        Err(error) => {
            return Ok(Snapshot {
                detail: Some(format!(
                    "The portal PermissionStore is unavailable: {error}"
                )),
                ..Snapshot::default()
            });
        }
    };
    snapshot_with(&proxy)
}

pub fn reset_decision(resource: PortalResource, app_id: &str) -> Result<Snapshot, Error> {
    validate_app_id(app_id)?;
    let connection = Connection::session()
        .map_err(|error| Error::new("connect to the portal PermissionStore", error.to_string()))?;
    let proxy = Proxy::new(&connection, DESTINATION, PATH, INTERFACE)
        .map_err(|error| Error::new("open the portal PermissionStore", error.to_string()))?;
    reset_with(&proxy, resource, app_id)
}

trait ProRunner {
    fn api(&self, endpoint: &'static str) -> Result<Vec<u8>, String>;
}

struct SystemProRunner;

impl ProRunner for SystemProRunner {
    fn api(&self, endpoint: &'static str) -> Result<Vec<u8>, String> {
        let mut child = Command::new("pro")
            .args(["api", endpoint])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("could not run Ubuntu Pro Client: {error}"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "could not capture Ubuntu Pro Client output".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "could not capture Ubuntu Pro Client errors".to_string())?;
        let stdout_reader = std::thread::spawn(move || read_bounded(stdout));
        let stderr_reader = std::thread::spawn(move || read_bounded(stderr));
        let deadline = Instant::now() + PRO_TIMEOUT;
        let status = loop {
            if let Some(status) = child
                .try_wait()
                .map_err(|error| format!("could not wait for Ubuntu Pro Client: {error}"))?
            {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Ubuntu Pro Client timed out after 15 seconds".into());
            }
            std::thread::sleep(Duration::from_millis(25));
        };
        let stdout = stdout_reader
            .join()
            .map_err(|_| "Ubuntu Pro Client output reader failed".to_string())??;
        let stderr = stderr_reader
            .join()
            .map_err(|_| "Ubuntu Pro Client error reader failed".to_string())??;
        if stdout.len() > MAX_PRO_OUTPUT_BYTES || stderr.len() > MAX_PRO_OUTPUT_BYTES {
            return Err("Ubuntu Pro Client output exceeded the 1 MiB safety limit".into());
        }
        if !status.success() {
            let detail = String::from_utf8_lossy(&stderr).trim().to_string();
            return Err(if detail.is_empty() {
                format!("Ubuntu Pro Client exited with {status}")
            } else {
                detail
            });
        }
        Ok(stdout)
    }
}

fn read_bounded(reader: impl Read) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    reader
        .take((MAX_PRO_OUTPUT_BYTES + 1) as u64)
        .read_to_end(&mut output)
        .map_err(|error| format!("could not read Ubuntu Pro Client output: {error}"))?;
    Ok(output)
}

pub fn security_coverage_snapshot() -> SecurityCoverageSnapshot {
    security_coverage_with(&SystemProRunner)
}

fn security_coverage_with(runner: &impl ProRunner) -> SecurityCoverageSnapshot {
    let mut snapshot = SecurityCoverageSnapshot::default();

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

fn api_attributes(runner: &impl ProRunner, endpoint: &'static str) -> Result<Value, String> {
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

fn api_error_summary(envelope: &Value) -> String {
    envelope
        .get("errors")
        .and_then(Value::as_array)
        .and_then(|errors| errors.first())
        .and_then(|error| error.get("title"))
        .and_then(Value::as_str)
        .unwrap_or("Ubuntu Pro Client reported a failed result")
        .chars()
        .take(256)
        .collect()
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
    Ok(ProStatus {
        attached: boolean(&attached, "is_attached")?,
        contract_valid: boolean(&attached, "is_attached_and_contract_valid")?,
        contract_status: attached
            .get("contract_status")
            .and_then(Value::as_str)
            .map(str::to_string),
        contract_remaining_days: attached
            .get("contract_remaining_days")
            .and_then(Value::as_i64)
            .ok_or_else(|| "response omitted contract_remaining_days".to_string())?,
        enabled_services: Vec::new(),
    })
}

fn parse_enabled_services(services: Value) -> Result<Vec<String>, String> {
    services
        .get("enabled_services")
        .and_then(Value::as_array)
        .ok_or_else(|| "response omitted enabled_services".to_string())?
        .iter()
        .map(|service| {
            service
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty() && name.len() <= 128)
                .map(str::to_string)
                .ok_or_else(|| "response contained an invalid service name".to_string())
        })
        .collect::<Result<Vec<_>, _>>()
}

fn parse_automatic_updates(attributes: Value) -> Result<AutomaticUpdates, String> {
    let allowed_origins = attributes
        .get("unattended_upgrades_allowed_origins")
        .and_then(Value::as_array)
        .ok_or_else(|| "response omitted unattended_upgrades_allowed_origins".to_string())?
        .iter()
        .map(|origin| {
            origin
                .as_str()
                .filter(|origin| origin.len() <= 256 && !origin.chars().any(char::is_control))
                .map(str::to_string)
                .ok_or_else(|| "response contained an invalid allowed origin".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let disabled_reason = attributes
        .pointer("/unattended_upgrades_disabled_reason/msg")
        .and_then(Value::as_str)
        .map(|reason| reason.chars().take(256).collect());
    Ok(AutomaticUpdates {
        running: boolean(&attributes, "unattended_upgrades_running")?,
        apt_timer_enabled: boolean(&attributes, "systemd_apt_timer_enabled")?,
        periodic_job_enabled: boolean(&attributes, "apt_periodic_job_enabled")?,
        package_list_frequency_days: unsigned(&attributes, "package_lists_refresh_frequency_days")?,
        upgrade_frequency_days: unsigned(&attributes, "unattended_upgrades_frequency_days")?,
        allowed_origins,
        last_run: attributes
            .get("unattended_upgrades_last_run")
            .and_then(Value::as_str)
            .map(str::to_string),
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

fn snapshot_with(store: &impl Store) -> Result<Snapshot, Error> {
    let version = store
        .version()
        .map_err(|error| Error::new("read PermissionStore version", error))?;
    let mut decisions = Vec::new();
    for resource in RESOURCES {
        match store.lookup(resource) {
            Ok(entries) => {
                decisions.extend(
                    entries
                        .into_iter()
                        .map(|(app_id, permissions)| PortalDecision {
                            resource,
                            app_id,
                            permissions,
                        }),
                )
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

fn reset_with(
    store: &impl Store,
    resource: PortalResource,
    app_id: &str,
) -> Result<Snapshot, Error> {
    let version = store
        .version()
        .map_err(|error| Error::new("read PermissionStore version", error))?;
    if version < 2 {
        return Err(Error::new(
            "reset portal permission",
            "PermissionStore version 2 is required",
        ));
    }
    store
        .delete_permission(resource, app_id)
        .map_err(|error| Error::new("reset portal permission", error))?;
    snapshot_with(store)
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::RefCell;

    struct FakeStore {
        version: u32,
        entries: RefCell<HashMap<PortalResource, HashMap<String, Vec<String>>>>,
        deleted: RefCell<Vec<(PortalResource, String)>>,
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

        fn delete_permission(&self, resource: PortalResource, app_id: &str) -> Result<(), String> {
            self.deleted
                .borrow_mut()
                .push((resource, app_id.to_string()));
            if let Some(entries) = self.entries.borrow_mut().get_mut(&resource) {
                entries.remove(app_id);
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
        let snapshot = reset_with(&store, PortalResource::Camera, "org.example.Camera").unwrap();
        assert_eq!(
            store.deleted.borrow().as_slice(),
            &[(PortalResource::Camera, "org.example.Camera".into())]
        );
        assert_eq!(snapshot.decisions.len(), 1);
        assert_eq!(snapshot.decisions[0].resource, PortalResource::Microphone);
    }

    #[test]
    fn reset_requires_version_two_and_a_bounded_app_id() {
        assert!(reset_with(&store(1), PortalResource::Camera, "org.example.Camera").is_err());
        assert!(validate_app_id("").is_err());
        assert!(validate_app_id("org.example\nBad").is_err());
    }

    struct FakeProRunner {
        responses: HashMap<&'static str, Result<Value, String>>,
    }

    impl ProRunner for FakeProRunner {
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
    }

    fn complete_pro_runner() -> FakeProRunner {
        FakeProRunner {
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
        }
    }

    #[test]
    fn security_coverage_keeps_authorities_separate() {
        let snapshot = security_coverage_with(&complete_pro_runner());
        assert!(snapshot.pro_client_available);
        assert!(snapshot.issues.is_empty());
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
        let snapshot = security_coverage_with(&runner);
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
        let snapshot = security_coverage_with(&runner);
        assert!(snapshot.pro.unwrap().contract_valid);
        assert!(snapshot
            .issues
            .iter()
            .any(|issue| issue.starts_with("Ubuntu Pro services:")));
    }
}
