//! The Lulo menu's Sleep, Restart, Shut Down, Lock Screen and Log Out,
//! through the same Windows calls Start's power menu uses. Open apps get
//! the usual end-of-session messages and may ask to save first.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, LUID};
use windows::Win32::Security::{
    AdjustTokenPrivileges, LookupPrivilegeValueW, LUID_AND_ATTRIBUTES, SE_PRIVILEGE_ENABLED,
    SE_SHUTDOWN_NAME, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows::Win32::System::Power::SetSuspendState;
use windows::Win32::System::Shutdown::{
    ExitWindowsEx, InitiateShutdownW, LockWorkStation, EWX_LOGOFF, SHTDN_REASON_FLAG_PLANNED,
    SHUTDOWN_HYBRID, SHUTDOWN_POWEROFF, SHUTDOWN_RESTART,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Command {
    Sleep,
    Restart,
    ShutDown,
    LockScreen,
    LogOut,
}

/// Ordinary users hold the shutdown privilege, but it starts disabled.
fn enable_shutdown_privilege() {
    // SAFETY: adjusts this process's own token; handles are closed.
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
        .is_err()
        {
            return;
        }
        let mut luid = LUID::default();
        if LookupPrivilegeValueW(PCWSTR::null(), SE_SHUTDOWN_NAME, &mut luid).is_ok() {
            let privileges = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES {
                    Luid: luid,
                    Attributes: SE_PRIVILEGE_ENABLED,
                }],
            };
            let _ = AdjustTokenPrivileges(token, false, Some(&privileges), 0, None, None);
        }
        let _ = CloseHandle(token);
    }
}

/// Carry out `command`. Sleep returns once the PC wakes, so it runs on its
/// own thread.
pub fn run(command: Command) {
    match command {
        Command::Sleep => {
            let _ = std::thread::Builder::new()
                .name("lulo-sleep".into())
                .spawn(|| {
                    enable_shutdown_privilege();
                    // SAFETY: suspends the PC; no pointers.
                    let _ = unsafe { SetSuspendState(false, false, false) };
                });
        }
        Command::Restart | Command::ShutDown => {
            enable_shutdown_privilege();
            let flags = if command == Command::Restart {
                SHUTDOWN_RESTART
            } else {
                // As Start's Shut down does, with fast start-up when on.
                SHUTDOWN_POWEROFF | SHUTDOWN_HYBRID
            };
            // SAFETY: no message and the local machine (null strings).
            let status = unsafe {
                InitiateShutdownW(
                    PCWSTR::null(),
                    PCWSTR::null(),
                    0,
                    flags,
                    SHTDN_REASON_FLAG_PLANNED,
                )
            };
            if status != 0 {
                eprintln!("lulo-shell: Windows did not start to shut down (error {status})");
            }
        }
        Command::LockScreen => {
            // SAFETY: no arguments.
            let _ = unsafe { LockWorkStation() };
        }
        Command::LogOut => {
            // SAFETY: logs the user out; apps are asked to close.
            let _ = unsafe { ExitWindowsEx(EWX_LOGOFF, SHTDN_REASON_FLAG_PLANNED) };
        }
    }
}
