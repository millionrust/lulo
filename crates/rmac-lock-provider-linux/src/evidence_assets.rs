const CUSTOM_UNIT: &str = include_str!("../evidence/units/rmac-lock-provider-evidence.service");
const FALLBACK_UNIT: &str = include_str!("../evidence/units/rmac-lock-fallback-evidence.service");
const NESTED_SWAY: &str = include_str!("../evidence/nested-sway.conf");
const INSTALLER: &str = include_str!("../../../scripts/linux/install-lock-provider-evidence.sh");
const LAUNCHER: &str = include_str!("../../../scripts/linux/launch-lock-provider-evidence.sh");
const NORMAL_INSTALLER: &str = include_str!("../../../scripts/linux/install-session-units.sh");
const RECOVERY_GATE: &str =
    include_str!("../../../scripts/linux/run-lock-provider-recovery-gate.sh");
const RECOVERY_RUNBOOK: &str = include_str!("../../../docs/secure-lock-recovery.md");

#[test]
fn evidence_units_are_separate_readiness_gated_crash_domains() {
    assert!(CUSTOM_UNIT.contains("Type=notify"));
    assert!(CUSTOM_UNIT.contains("NotifyAccess=all"));
    assert!(CUSTOM_UNIT.contains("WatchdogSec=10s"));
    assert!(CUSTOM_UNIT.contains("WatchdogSignal=SIGKILL"));
    assert!(CUSTOM_UNIT.contains("Restart=on-failure"));
    assert!(CUSTOM_UNIT.contains("StartLimitIntervalSec=30s"));
    assert!(CUSTOM_UNIT.contains("StartLimitBurst=5"));
    assert!(CUSTOM_UNIT.contains("KillMode=control-group"));
    assert!(CUSTOM_UNIT.contains("EnvironmentFile=%t/rmac-lock-evidence/environment"));
    assert!(CUSTOM_UNIT.contains("/rmac-evidence/rmac-lock-provider"));

    assert!(FALLBACK_UNIT.contains("Type=notify"));
    assert!(FALLBACK_UNIT.contains("NotifyAccess=all"));
    assert!(FALLBACK_UNIT.contains("Restart=on-failure"));
    assert!(FALLBACK_UNIT.contains("StartLimitIntervalSec=0"));
    assert!(FALLBACK_UNIT.contains("/rmac/rmac-locker --config"));
    assert!(FALLBACK_UNIT.contains("EnvironmentFile=%t/rmac-lock-evidence/environment"));

    for unit in [CUSTOM_UNIT, FALLBACK_UNIT] {
        assert!(!unit.contains("[Install]"));
        assert!(!unit.contains("/bin/sh"));
        assert!(!unit.contains("rmac-lock.service"));
    }
}

#[test]
fn evidence_scripts_require_opt_in_nested_recovery_without_installing_it_normally() {
    assert!(INSTALLER.contains("--features development-provider"));
    assert!(INSTALLER.contains("at least 25 GiB free"));
    assert!(INSTALLER.contains("no provider was started or enabled"));
    assert!(!INSTALLER.contains("systemctl --user enable"));
    assert!(!INSTALLER.contains("rmac-lock.service"));
    assert!(!NORMAL_INSTALLER.contains("development-provider"));
    assert!(!NORMAL_INSTALLER.contains("rmac-lock-provider-evidence"));

    assert!(LAUNCHER.contains("/run/user/$(id -u)"));
    assert!(LAUNCHER.contains("XDG_SESSION_ID"));
    assert!(LAUNCHER.contains("rmac-lock-provider-evidence.service"));
    assert!(NESTED_SWAY.contains("exec rmac-lock-provider-evidence-launch"));

    assert!(RECOVERY_GATE.contains("NESTED-LOCK-RECOVERY"));
    assert!(RECOVERY_GATE.contains("WLR_BACKENDS=wayland"));
    assert!(RECOVERY_GATE.contains("--signal=KILL"));
    assert!(RECOVERY_GATE.contains("--signal=STOP"));
    assert!(RECOVERY_GATE.contains("rmac-lock-fallback-evidence.service"));
    assert!(RECOVERY_GATE.contains("SetLockedHint b false"));
    assert!(RECOVERY_GATE.contains("target/linux-evidence"));
    assert!(RECOVERY_GATE.contains("custom_restart_count"));
    assert!(RECOVERY_GATE.contains("watchdog_restart=pass"));
    assert!(!RECOVERY_GATE.contains("journalctl"));
    assert!(!RECOVERY_GATE.contains("rmac-lock.service"));
}

#[test]
fn recovery_runbook_restores_authentication_or_terminates_the_exact_session() {
    assert!(RECOVERY_RUNBOOK.contains("systemctl --user start rmac-lock.service"));
    assert!(RECOVERY_RUNBOOK.contains("systemctl --user reset-failed rmac-lock.service"));
    assert!(RECOVERY_RUNBOOK.contains("Do not use `sudo systemctl --user`"));
    assert!(RECOVERY_RUNBOOK.contains("loginctl terminate-session SESSION_ID"));
    assert!(RECOVERY_RUNBOOK.contains("explicitly accepts that loss"));
    assert!(RECOVERY_RUNBOOK.contains("not clear `LockedHint`"));
    assert!(RECOVERY_RUNBOOK.contains("Type=wayland"));
    assert!(RECOVERY_RUNBOOK.contains("Remote=no"));
    assert!(!RECOVERY_RUNBOOK.contains("loginctl unlock-session"));
    assert!(!RECOVERY_RUNBOOK.contains("SetLockedHint b false"));
    assert!(!RECOVERY_RUNBOOK.contains("killall"));
    assert!(!RECOVERY_RUNBOOK.contains("pkill"));
}
