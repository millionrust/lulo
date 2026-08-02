//! Focused Linux login-item contracts.

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn environment() -> (PathBuf, Environment) {
    let root = std::env::temp_dir().join(format!(
        "rmac-login-items-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let user = root.join("user");
    let system = root.join("system");
    std::fs::create_dir_all(system.join("autostart")).unwrap();
    (
        root.clone(),
        Environment {
            config_home: user,
            config_dirs: vec![system],
            data_home: root.join("data"),
            desktops: vec!["rmac".into()],
        },
    )
}

#[test]
fn precedence_disable_and_managed_restore_follow_xdg_contract() {
    let (root, environment) = environment();
    let system = environment.config_dirs[0]
        .join("autostart")
        .join("demo.desktop");
    std::fs::write(
        &system,
        "[Desktop Entry]\nType=Application\nName=Demo\nExec=demo\n",
    )
    .unwrap();
    let service = SystemService {
        environment: environment.clone(),
    };
    assert!(service.snapshot().unwrap().items[0].enabled);
    let disabled = service.set_enabled("demo.desktop", false).unwrap();
    assert!(!disabled.items[0].enabled);
    assert!(disabled.items[0].managed_override);
    let enabled = service.set_enabled("demo.desktop", true).unwrap();
    assert!(enabled.items[0].enabled);
    assert_eq!(enabled.items[0].source, system);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn malformed_higher_priority_entry_is_reported_and_hides_lower_entry() {
    let (root, environment) = environment();
    let user = environment.config_home.join("autostart");
    std::fs::create_dir_all(&user).unwrap();
    std::fs::write(user.join("demo.desktop"), "not a desktop entry").unwrap();
    std::fs::write(
        environment.config_dirs[0].join("autostart/demo.desktop"),
        "[Desktop Entry]\nType=Application\nName=System Demo\nExec=demo\n",
    )
    .unwrap();
    let snapshot = discover(&environment).unwrap();
    assert!(snapshot.items.is_empty());
    assert_eq!(snapshot.issues.len(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn user_manager_idle_exit_is_ignored_but_reappearance_refreshes() {
    assert!(!owner_change_reappeared("org.freedesktop.systemd1", ""));
    assert!(owner_change_reappeared("org.freedesktop.systemd1", ":1.42"));
    assert!(!owner_change_reappeared("org.example.Other", ":1.42"));
}

#[test]
fn add_requires_explicit_replacement_and_installs_enabled_entry() {
    let (root, environment) = environment();
    let source = root.join("demo.desktop");
    let target = environment.config_home.join("autostart/demo.desktop");
    std::fs::write(
        &source,
        "[Desktop Entry]\nType=Application\nName=Demo\nHidden=true\nExec=demo\n",
    )
    .unwrap();
    let service = SystemService { environment };
    let preview = service.prepare_add(&source).unwrap();
    assert!(!preview.replacing);
    assert_eq!(preview.command, "demo");
    let snapshot = service.add(&preview).unwrap();
    assert!(snapshot.items[0].enabled);
    assert!(service.add(&preview).is_err());
    let replacement = service.prepare_add(&source).unwrap();
    assert!(replacement.replacing);
    std::fs::write(
        &target,
        "[Desktop Entry]\nType=Application\nName=External\nExec=external\n",
    )
    .unwrap();
    assert_eq!(
        service.add(&replacement).unwrap_err().kind(),
        ErrorKind::Conflict
    );
    assert!(std::fs::read_to_string(&target)
        .unwrap()
        .contains("Exec=external"));
    let replacement = service.prepare_add(&source).unwrap();
    assert!(service.add(&replacement).is_ok());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn add_rejects_a_source_changed_after_confirmation() {
    let (root, environment) = environment();
    let source = root.join("demo.desktop");
    std::fs::write(
        &source,
        "[Desktop Entry]\nType=Application\nName=Demo\nExec=demo\n",
    )
    .unwrap();
    let service = SystemService { environment };
    let preview = service.prepare_add(&source).unwrap();
    std::fs::write(
        &source,
        "[Desktop Entry]\nType=Application\nName=Changed\nExec=other\n",
    )
    .unwrap();
    assert_eq!(
        service.add(&preview).unwrap_err().kind(),
        ErrorKind::Conflict
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn bounded_reader_rejects_oversized_and_linked_entries() {
    let (root, environment) = environment();
    let oversized = environment.config_dirs[0].join("autostart/oversized.desktop");
    std::fs::write(
        &oversized,
        vec![b'x'; rmac_login_items::MAX_ENTRY_BYTES + 1],
    )
    .unwrap();
    let snapshot = discover(&environment).unwrap();
    assert!(snapshot.items.is_empty());
    assert_eq!(snapshot.issues[0].detail, "entry is too large");

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let real = root.join("real.desktop");
        std::fs::write(
            &real,
            "[Desktop Entry]\nType=Application\nName=Real\nExec=real\n",
        )
        .unwrap();
        let linked = root.join("linked.desktop");
        symlink(&real, &linked).unwrap();
        assert_eq!(
            prepare_add(&environment, &linked).unwrap_err().kind(),
            ErrorKind::InvalidEntry
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn removal_keeps_a_revealed_system_entry_disabled() {
    let (root, environment) = environment();
    let system = environment.config_dirs[0]
        .join("autostart")
        .join("demo.desktop");
    std::fs::write(
        &system,
        "[Desktop Entry]\nType=Application\nName=System Demo\nExec=system-demo\n",
    )
    .unwrap();
    let user = environment
        .config_home
        .join("autostart")
        .join("demo.desktop");
    std::fs::create_dir_all(user.parent().unwrap()).unwrap();
    let contents =
        "[Desktop Entry]\nType=Application\nName=User Demo\nHidden=true\nExec=user-demo\n";
    std::fs::write(&user, contents).unwrap();
    let service = SystemService { environment };
    let preview = service.prepare_remove("demo.desktop").unwrap();
    std::fs::remove_file(&user).unwrap();
    let revealed = service.snapshot().unwrap();
    assert!(revealed.items[0].enabled);
    let protected = preserve_disabled_after_removal(&service, &preview, revealed).unwrap();
    assert!(!protected.items[0].enabled);
    assert!(protected.items[0].managed_override);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn removal_recovery_never_overwrites_a_concurrent_user_entry() {
    let (root, environment) = environment();
    let system = environment.config_dirs[0]
        .join("autostart")
        .join("demo.desktop");
    std::fs::write(
        &system,
        "[Desktop Entry]\nType=Application\nName=System Demo\nExec=system-demo\n",
    )
    .unwrap();
    let user = environment
        .config_home
        .join("autostart")
        .join("demo.desktop");
    std::fs::create_dir_all(user.parent().unwrap()).unwrap();
    std::fs::write(
        &user,
        "[Desktop Entry]\nType=Application\nName=Old User\nHidden=true\nExec=old\n",
    )
    .unwrap();
    let service = SystemService { environment };
    let preview = service.prepare_remove("demo.desktop").unwrap();
    std::fs::remove_file(&user).unwrap();
    let revealed = service.snapshot().unwrap();
    let replacement = "[Desktop Entry]\nType=Application\nName=New User\nExec=new\n";
    std::fs::write(&user, replacement).unwrap();
    assert_eq!(
        preserve_disabled_after_removal(&service, &preview, revealed)
            .unwrap_err()
            .kind(),
        ErrorKind::Conflict
    );
    assert_eq!(std::fs::read_to_string(&user).unwrap(), replacement);
    std::fs::remove_dir_all(root).unwrap();
}
