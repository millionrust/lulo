//! Linux side: keyd, systemd-localed, pkexec and the niri focus follower.
//!
//! Privilege split: System Settings runs as the user and only reads. Every
//! change to `/etc/keyd` or to the keyd service goes through
//! `pkexec /usr/libexec/rmac/rmac-mac-keyboard apply …`, whose arguments are
//! enumerated values only. The follower runs as the user and talks to keyd's
//! socket, which the keyd package restricts to the `keyd` group.

use std::ffi::CStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use rmac_locale::X11Keyboard;

use crate::{
    bind_arguments, detect, helper_arguments, keyd_config, parse_group_id, parse_keyd_header,
    profile_for_app, supports_option_characters, xkb_for, Error, MacKeyboard, PhysicalLayout,
    Profile, Status,
};

pub const KEYD_DIRECTORY: &str = "/etc/keyd";
pub const KEYD_CONFIG: &str = "/etc/keyd/rmac.conf";
pub const HELPER: &str = "/usr/libexec/rmac/rmac-mac-keyboard";
pub const FOLLOWER_UNIT: &str = "rmac-mac-keyboard.service";
const KEYD_GROUP: &str = "keyd";
const KEYD_SERVICE: &str = "keyd.service";
/// Debian renames upstream's `keyd` to `keyd.rvaiya` (Debian bug #1098982).
const KEYD_BINARIES: [&str; 3] = [
    "/usr/bin/keyd.rvaiya",
    "/usr/bin/keyd",
    "/usr/local/bin/keyd",
];
const PKEXEC: &str = "/usr/bin/pkexec";
const SYSTEMCTL: &str = "/usr/bin/systemctl";
const GPASSWD: &str = "/usr/bin/gpasswd";

pub fn status() -> Result<Status, Error> {
    let snapshot = rmac_locale_linux::snapshot().map_err(|error| Error::new(error.to_string()))?;
    let keyboard = snapshot.x11_keyboard();
    let config = std::fs::read_to_string(KEYD_CONFIG).ok();
    Ok(Status {
        state: detect(config.as_deref(), &keyboard),
        option_characters_available: supports_option_characters(&keyboard),
        keyboard,
        keyd_installed: keyd_binary().is_some(),
        helper_installed: Path::new(HELPER).is_file(),
        session_can_bind: session_in_group(KEYD_GROUP),
        foreign_keyd_configs: foreign_keyd_configs(),
    })
}

/// Apply `target` from the user session. Changes that involve keyd go
/// through the privileged helper (one authentication prompt); XKB-only
/// changes go straight to localed, which asks for authentication itself.
pub fn apply(target: &MacKeyboard) -> Result<Status, Error> {
    let current = status()?;
    if target.shortcuts_in_all_apps || current.state.shortcuts_in_all_apps {
        let output = Command::new(PKEXEC)
            .arg(HELPER)
            .args(helper_arguments(target))
            .stdin(Stdio::null())
            .output()
            .map_err(|error| Error::new(format!("could not start pkexec: {error}")))?;
        if !output.status.success() {
            return Err(Error::new(command_failure(
                "the keyboard helper",
                &output.stderr,
                output.status.code(),
            )));
        }
        // keyd restarted and dropped every dynamic binding; restart the
        // follower so it re-applies the focused app's profile (or stop it).
        let verb = if target.shortcuts_in_all_apps {
            "restart"
        } else {
            "stop"
        };
        let _ = Command::new(SYSTEMCTL)
            .args(["--user", verb, FOLLOWER_UNIT])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    } else {
        let keyboard = xkb_for(&current.keyboard, target);
        if keyboard != current.keyboard {
            rmac_locale_linux::set_x11_keyboard(&keyboard)
                .map_err(|error| Error::new(error.to_string()))?;
        }
    }
    status()
}

/// Root half of [`apply`], run by the helper under pkexec.
pub fn apply_as_root(target: &MacKeyboard) -> Result<(), Error> {
    require_root()?;
    let snapshot = rmac_locale_linux::snapshot().map_err(|error| Error::new(error.to_string()))?;
    let keyboard = xkb_for(&snapshot.x11_keyboard(), target);
    if target.shortcuts_in_all_apps {
        if keyd_binary().is_none() {
            return Err(Error::new(
                "keyd is not installed; install the keyd package first",
            ));
        }
        let foreign = foreign_keyd_configs();
        if !foreign.is_empty() {
            return Err(Error::new(format!(
                "another keyd configuration is installed ({}); Lulo OS will not change it",
                foreign.join(", ")
            )));
        }
        write_keyd_config(&target.layout)?;
        if let Some(user) = pkexec_user()? {
            run(GPASSWD, &["-a", &user, KEYD_GROUP])?;
        }
        run(SYSTEMCTL, &["enable", KEYD_SERVICE])?;
        run(SYSTEMCTL, &["restart", KEYD_SERVICE])?;
        set_keyboard(&snapshot.x11_keyboard(), &keyboard)
    } else {
        match std::fs::remove_file(KEYD_CONFIG) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(Error::new(format!(
                    "could not remove {KEYD_CONFIG}: {error}"
                )))
            }
        }
        if foreign_keyd_configs().is_empty() {
            run(SYSTEMCTL, &["disable", "--now", KEYD_SERVICE])?;
        } else {
            run(SYSTEMCTL, &["restart", KEYD_SERVICE])?;
        }
        set_keyboard(&snapshot.x11_keyboard(), &keyboard)
    }
}

/// Rewrite an installed rmac keyd file with this version's generator (run
/// from the package's postinst so upgrades pick up new bindings).
pub fn regenerate_as_root() -> Result<bool, Error> {
    require_root()?;
    let Ok(source) = std::fs::read_to_string(KEYD_CONFIG) else {
        return Ok(false);
    };
    let Some(layout) = parse_keyd_header(&source) else {
        return Ok(false);
    };
    if source == keyd_config(&layout) {
        return Ok(false);
    }
    write_keyd_config(&layout)?;
    Ok(true)
}

fn set_keyboard(current: &X11Keyboard, keyboard: &X11Keyboard) -> Result<(), Error> {
    if current == keyboard {
        return Ok(());
    }
    rmac_locale_linux::set_x11_keyboard(keyboard)
        .map(|_| ())
        .map_err(|error| Error::new(format!("could not update the keyboard layout: {error}")))
}

fn write_keyd_config(layout: &PhysicalLayout) -> Result<(), Error> {
    use std::os::unix::fs::PermissionsExt as _;

    std::fs::create_dir_all(KEYD_DIRECTORY)
        .map_err(|error| Error::new(format!("could not create {KEYD_DIRECTORY}: {error}")))?;
    let path = Path::new(KEYD_CONFIG);
    rmac_storage::atomic_write(path, keyd_config(layout).as_bytes())
        .map_err(|error| Error::new(format!("could not write {KEYD_CONFIG}: {error}")))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644))
        .map_err(|error| Error::new(format!("could not set {KEYD_CONFIG} permissions: {error}")))
}

fn require_root() -> Result<(), Error> {
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } == 0 {
        Ok(())
    } else {
        Err(Error::new("this command must run as root through pkexec"))
    }
}

/// The user who asked pkexec to run the helper.
fn pkexec_user() -> Result<Option<String>, Error> {
    let Some(value) = std::env::var_os("PKEXEC_UID") else {
        return Ok(None);
    };
    let uid = value
        .to_str()
        .and_then(|value| value.parse::<libc::uid_t>().ok())
        .ok_or_else(|| Error::new("PKEXEC_UID is not a user ID"))?;
    user_name(uid)
        .map(Some)
        .ok_or_else(|| Error::new("the requesting user could not be resolved"))
}

fn user_name(uid: libc::uid_t) -> Option<String> {
    let mut buffer = vec![0u8; 16 * 1024];
    // SAFETY: passwd is plain data; getpwuid_r writes it and the strings it
    // points to into `buffer`, which outlives every read below.
    unsafe {
        let mut entry: libc::passwd = std::mem::zeroed();
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        let code = libc::getpwuid_r(
            uid,
            &mut entry,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        );
        if code != 0 || result.is_null() || entry.pw_name.is_null() {
            return None;
        }
        CStr::from_ptr(entry.pw_name)
            .to_str()
            .ok()
            .map(str::to_owned)
    }
}

fn run(program: &str, arguments: &[&str]) -> Result<(), Error> {
    let output = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| Error::new(format!("could not start {program}: {error}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(Error::new(command_failure(
            program,
            &output.stderr,
            output.status.code(),
        )))
    }
}

fn command_failure(program: &str, stderr: &[u8], code: Option<i32>) -> String {
    let detail = String::from_utf8_lossy(stderr);
    let detail = detail.trim();
    match (code, detail.is_empty()) {
        // pkexec: 126 = dismissed, 127 = not authorised.
        (Some(126), _) | (Some(127), _) if program == "the keyboard helper" => {
            "authentication was cancelled".into()
        }
        (_, false) => format!(
            "{program} failed: {}",
            detail.lines().last().unwrap_or(detail)
        ),
        (Some(code), true) => format!("{program} exited with status {code}"),
        (None, true) => format!("{program} was stopped by a signal"),
    }
}

pub fn keyd_binary() -> Option<PathBuf> {
    KEYD_BINARIES
        .iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
}

/// `*.conf` files in `/etc/keyd` other than rmac's.
pub fn foreign_keyd_configs() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(KEYD_DIRECTORY) else {
        return Vec::new();
    };
    let mut names = entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.ends_with(".conf") && !name.starts_with('.') && name != "rmac.conf")
        .collect::<Vec<_>>();
    names.sort();
    names
}

/// Whether this process (and so this login session) carries `group`.
fn session_in_group(group: &str) -> bool {
    let Some(gid) = group_id(group) else {
        return false;
    };
    // SAFETY: a zero-length query returns the count; the second call fills a
    // buffer of exactly that many entries.
    let groups = unsafe {
        let count = libc::getgroups(0, std::ptr::null_mut());
        if count < 0 {
            return false;
        }
        let mut groups = vec![0 as libc::gid_t; count as usize];
        let filled = libc::getgroups(count, groups.as_mut_ptr());
        if filled < 0 {
            return false;
        }
        groups.truncate(filled as usize);
        groups
    };
    // SAFETY: getegid has no preconditions.
    groups.contains(&gid) || unsafe { libc::getegid() } == gid
}

fn group_id(group: &str) -> Option<u32> {
    let source = std::fs::read_to_string("/etc/group").ok()?;
    parse_group_id(&source, group)
}

/// Switch keyd to `profile`.
pub fn bind(keyd: &Path, profile: Profile) -> Result<(), Error> {
    let output = Command::new(keyd)
        .arg("bind")
        .args(bind_arguments(profile))
        .stdin(Stdio::null())
        .output()
        .map_err(|error| Error::new(format!("could not start keyd: {error}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(Error::new(command_failure(
            "keyd bind",
            &output.stderr,
            output.status.code(),
        )))
    }
}

/// The session follower: watch niri's focus and keep keyd's `cmd`/`opt`
/// layers matched to the focused app. Returns when niri's stream ends for
/// good; systemd restarts it with the session.
pub fn follow() -> Result<(), Error> {
    let keyd = keyd_binary().ok_or_else(|| Error::new("keyd is not installed"))?;
    let installed = std::fs::read_to_string(KEYD_CONFIG)
        .ok()
        .and_then(|source| parse_keyd_header(&source));
    if installed.is_none() {
        // Mac shortcuts are off: nothing to follow.
        return Ok(());
    }
    let (sender, receiver) = async_channel::unbounded();
    std::thread::Builder::new()
        .name("niri-events".into())
        .spawn(move || {
            if let Err(error) = async_io::block_on(rmac_compositor_niri::watch(sender)) {
                eprintln!("rmac-mac-keyboard: niri event stream ended: {error}");
            }
        })
        .map_err(|error| Error::new(format!("could not start the niri watcher: {error}")))?;

    let mut state = rmac_compositor::State::default();
    let mut applied = None;
    let mut reported = false;
    while let Ok(event) = receiver.recv_blocking() {
        state.apply(event);
        let profile = focused_profile(&state);
        if applied == Some(profile) {
            continue;
        }
        match bind(&keyd, profile) {
            Ok(()) => {
                applied = Some(profile);
                reported = false;
            }
            Err(error) if !reported => {
                eprintln!("rmac-mac-keyboard: {error}");
                reported = true;
            }
            Err(_) => {}
        }
    }
    let _ = bind(&keyd, Profile::Native);
    Ok(())
}

/// Drop every dynamic binding (the unit's stop hook).
pub fn reset() -> Result<(), Error> {
    let keyd = keyd_binary().ok_or_else(|| Error::new("keyd is not installed"))?;
    bind(&keyd, Profile::Native)
}

fn focused_profile(state: &rmac_compositor::State) -> Profile {
    match &state.focus.target {
        Some(rmac_compositor::FocusTarget::Window(id)) => profile_for_app(
            state
                .windows
                .get(id)
                .and_then(|window| window.app_id.as_deref()),
        ),
        // Layer surfaces are rmac's own shell (Spotlight, Control Center…).
        _ => Profile::Native,
    }
}
