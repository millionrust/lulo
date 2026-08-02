//! Focused Linux privacy and security contracts.

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
    assert!(snapshot
        .issues
        .iter()
        .any(|issue| issue == "Ubuntu Pro status: response contained an invalid contract status"));
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
