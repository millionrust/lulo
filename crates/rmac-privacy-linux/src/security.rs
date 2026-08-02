//! Ubuntu security coverage evidence and bounded helper execution.

use super::*;

pub(super) trait SecurityRunner {
    fn api(&self, endpoint: &'static str) -> Result<Vec<u8>, String>;
    fn release_days(&self, series: &str) -> Result<i64, String>;
}

pub(super) struct SystemSecurityRunner;

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

pub(super) fn run_bounded(
    program: &str,
    arguments: &[&str],
    label: &str,
) -> Result<Vec<u8>, String> {
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

pub(super) fn read_bounded(mut reader: impl Read) -> Result<(Vec<u8>, bool), String> {
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

pub(super) fn ubuntu_series() -> Result<String, String> {
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

pub(super) fn security_coverage_with(
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

pub(super) fn api_attributes(
    runner: &impl SecurityRunner,
    endpoint: &'static str,
) -> Result<Value, String> {
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

pub(super) fn api_error_summary(_envelope: &Value) -> String {
    "Ubuntu Pro Client reported a failed result".to_string()
}

pub(super) fn parse_package_sources(attributes: Value) -> Result<PackageSources, String> {
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

pub(super) fn parse_pro_attachment(attached: Value) -> Result<ProStatus, String> {
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

pub(super) fn parse_enabled_services(services: Value) -> Result<Vec<String>, String> {
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

pub(super) fn parse_automatic_updates(attributes: Value) -> Result<AutomaticUpdates, String> {
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

pub(super) fn unsigned(value: &Value, key: &str) -> Result<u64, String> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("response omitted {key}"))
}

pub(super) fn boolean(value: &Value, key: &str) -> Result<bool, String> {
    value
        .get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("response omitted {key}"))
}

pub(super) fn validated_text(
    value: &str,
    maximum_bytes: usize,
    label: &str,
) -> Result<String, String> {
    if value.is_empty() || value.len() > maximum_bytes || value.chars().any(char::is_control) {
        return Err(format!("response contained an invalid {label}"));
    }
    Ok(value.to_string())
}

pub(super) fn bounded_text(value: &str, maximum_bytes: usize) -> String {
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
