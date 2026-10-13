//! Hot-plug: block until the OS says a device came or went, wait for the
//! burst to settle, then call `changed`. The thread sleeps in the kernel
//! between events; there is no timer while nothing happens.

#[cfg(any(target_os = "linux", windows))]
use std::time::Duration;

/// How long a burst of device events (a dock with five devices, a USB hub)
/// must be quiet before detection runs again.
#[cfg(any(target_os = "linux", windows))]
const SETTLE: Duration = Duration::from_millis(400);

/// Subsystems whose add/remove changes the profile. A `change` counts only
/// for `drm` (a monitor plugged into an existing connector) and `switch`.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn relevant_uevent(message: &[u8]) -> bool {
    let mut fields = message.split(|byte| *byte == 0);
    let Some(header) = fields.next() else {
        return false;
    };
    // libudev's own messages start "libudev"; the kernel's "ACTION@path".
    if !header.contains(&b'@') {
        return false;
    }
    let mut action = "";
    let mut subsystem = "";
    for field in fields {
        let Ok(field) = std::str::from_utf8(field) else {
            continue;
        };
        if let Some(value) = field.strip_prefix("ACTION=") {
            action = value;
        } else if let Some(value) = field.strip_prefix("SUBSYSTEM=") {
            subsystem = value;
        }
    }
    const HOTPLUG: [&str; 13] = [
        "input",
        "power_supply",
        "bluetooth",
        "net",
        "usb",
        "video4linux",
        "sound",
        "iio",
        "backlight",
        "leds",
        "wwan",
        "drm",
        "hid",
    ];
    match action {
        "add" | "remove" => HOTPLUG.contains(&subsystem),
        "change" => subsystem == "drm",
        _ => false,
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn run(changed: impl Fn()) {
    // SAFETY: a netlink socket owned by this thread for its whole life;
    // buffers outlive each call.
    unsafe {
        let fd = libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_DGRAM | libc::SOCK_CLOEXEC,
            libc::NETLINK_KOBJECT_UEVENT,
        );
        if fd < 0 {
            return;
        }
        let mut address: libc::sockaddr_nl = std::mem::zeroed();
        address.nl_family = libc::AF_NETLINK as u16;
        // Group 1: the kernel's own uevents, readable without privileges.
        address.nl_groups = 1;
        if libc::bind(
            fd,
            &address as *const libc::sockaddr_nl as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_nl>() as u32,
        ) < 0
        {
            libc::close(fd);
            return;
        }
        let mut buffer = vec![0u8; 8192];
        let receive = |buffer: &mut [u8]| -> isize {
            libc::recv(fd, buffer.as_mut_ptr().cast(), buffer.len(), 0)
        };
        loop {
            let length = receive(&mut buffer);
            if length < 0 {
                if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                break;
            }
            if !relevant_uevent(&buffer[..length as usize]) {
                continue;
            }
            // Settle: keep reading until the socket is quiet for SETTLE.
            loop {
                let mut poll = libc::pollfd {
                    fd,
                    events: libc::POLLIN,
                    revents: 0,
                };
                let ready = libc::poll(&mut poll, 1, SETTLE.as_millis() as i32);
                if ready <= 0 {
                    break;
                }
                if receive(&mut buffer) < 0 {
                    break;
                }
            }
            changed();
        }
        libc::close(fd);
    }
}

#[cfg(windows)]
pub(crate) fn run(changed: impl Fn()) {
    use std::sync::mpsc;
    use std::sync::{Mutex, OnceLock};

    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        CM_Register_Notification, CM_NOTIFY_ACTION, CM_NOTIFY_EVENT_DATA, CM_NOTIFY_FILTER,
        CM_NOTIFY_FILTER_FLAG_ALL_DEVICE_INSTANCES, CM_NOTIFY_FILTER_TYPE_DEVICEINSTANCE,
        CR_SUCCESS, HCMNOTIFICATION,
    };

    static EVENTS: OnceLock<Mutex<mpsc::Sender<()>>> = OnceLock::new();

    unsafe extern "system" fn on_device(
        _: HCMNOTIFICATION,
        _: *const core::ffi::c_void,
        _: CM_NOTIFY_ACTION,
        _: *const CM_NOTIFY_EVENT_DATA,
        _: u32,
    ) -> u32 {
        if let Some(sender) = EVENTS.get().and_then(|sender| sender.lock().ok()) {
            let _ = sender.send(());
        }
        0
    }

    let (sender, events) = mpsc::channel();
    if EVENTS.set(Mutex::new(sender)).is_err() {
        return;
    }
    let filter = CM_NOTIFY_FILTER {
        cbSize: std::mem::size_of::<CM_NOTIFY_FILTER>() as u32,
        Flags: CM_NOTIFY_FILTER_FLAG_ALL_DEVICE_INSTANCES,
        FilterType: CM_NOTIFY_FILTER_TYPE_DEVICEINSTANCE,
        ..Default::default()
    };
    let mut handle = HCMNOTIFICATION(std::ptr::null_mut());
    // SAFETY: the filter is read during the call; the registration lives
    // as long as the process (the handle is never closed).
    let registered =
        unsafe { CM_Register_Notification(&filter, None, Some(on_device), &mut handle) };
    if registered != CR_SUCCESS {
        return;
    }
    while events.recv().is_ok() {
        // Settle: a device brings several instances (a USB composite).
        while events.recv_timeout(SETTLE).is_ok() {}
        changed();
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
pub(crate) fn run(changed: impl Fn()) {
    let _ = changed;
}
