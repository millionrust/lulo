//! The built-in display's brightness on Windows, through WMI's
//! `WmiMonitorBrightness` (read) and `WmiMonitorBrightnessMethods`
//! (`WmiSetBrightness`), as Windows' own brightness slider does. External
//! monitors have no such class; their brightness stays unavailable, as on
//! Lulo OS without a backlight.
//!
//! WMI loads several system libraries into the caller. The Lulo shell keeps
//! them out of its own process by naming a helper ([`set_helper`]): the
//! calls then run in `<helper> --brightness get|set <level>` (WIN-OS-53).

use std::path::PathBuf;
use std::sync::OnceLock;

use windows::core::{w, BSTR};
use windows::Win32::System::Com::{
    CoCreateInstance, CoIncrementMTAUsage, CoSetProxyBlanket, CLSCTX_INPROC_SERVER, EOAC_NONE,
    RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::Variant::{
    VariantToInt32, VariantToStringAlloc, VARENUM, VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0,
    VT_I4,
};
use windows::Win32::System::Wmi::{
    IEnumWbemClassObject, IWbemClassObject, IWbemLocator, IWbemServices, WbemLocator,
    WBEM_FLAG_FORWARD_ONLY, WBEM_FLAG_RETURN_IMMEDIATELY, WBEM_GENERIC_FLAG_TYPE, WBEM_INFINITE,
};

static HELPER: OnceLock<PathBuf> = OnceLock::new();

/// Run the WMI calls in `helper --brightness …` instead of this process.
pub fn set_helper(helper: PathBuf) {
    let _ = HELPER.set(helper);
}

pub(crate) fn brightness() -> Option<u8> {
    match HELPER.get() {
        Some(helper) => run_helper(helper, &["get"]),
        None => read_in_process(),
    }
}

pub(crate) fn set_brightness(level: u8) -> Option<u8> {
    match HELPER.get() {
        Some(helper) => run_helper(helper, &["set", &level.to_string()]),
        None => write_in_process(level),
    }
}

fn run_helper(helper: &PathBuf, arguments: &[&str]) -> Option<u8> {
    use std::os::windows::process::CommandExt as _;
    let output = std::process::Command::new(helper)
        .arg("--brightness")
        .args(arguments)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(windows::Win32::System::Threading::CREATE_NO_WINDOW.0)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

/// `<helper> --brightness get|set <level>`: print the level and exit.
pub fn run_brightness_helper(arguments: &[String]) -> i32 {
    let level = match arguments {
        [get] if get == "get" => read_in_process(),
        [set, level] if set == "set" => level.parse().ok().and_then(write_in_process),
        _ => None,
    };
    match level {
        Some(level) => {
            println!("{level}");
            0
        }
        None => 1,
    }
}

/// `result`'s value; an error is told on stderr (`stage` names the call),
/// which the helper's caller discards and a person running the helper sees.
fn checked<T>(stage: &str, result: windows::core::Result<T>) -> Option<T> {
    result
        .map_err(|error| eprintln!("brightness: {stage}: {error}"))
        .ok()
}

fn services() -> Option<IWbemServices> {
    // No thread joins an apartment here (one left on a Windows thread-pool
    // thread kept the shell's Open dialog from showing, op/win-flakes): the
    // process keeps the multithreaded apartment alive once, and a thread
    // without COM uses it implicitly.
    static MTA: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    MTA.get_or_init(|| {
        // SAFETY: a plain COM call; the cookie is kept for the process.
        let _ = unsafe { CoIncrementMTAUsage() };
    });
    // SAFETY: COM in that apartment (or the thread's own); the objects are
    // released when dropped.
    unsafe {
        let locator: IWbemLocator = checked(
            "WbemLocator",
            CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER),
        )?;
        let services = checked(
            "ConnectServer",
            locator.ConnectServer(
                &BSTR::from("ROOT\\WMI"),
                &BSTR::new(),
                &BSTR::new(),
                &BSTR::new(),
                0,
                &BSTR::new(),
                None,
            ),
        )?;
        // WMI's provider acts as the caller: without impersonation the
        // monitor classes answer nothing (10: NTLM / Negotiate, 0: no
        // authorisation service).
        checked(
            "CoSetProxyBlanket",
            CoSetProxyBlanket(
                &services,
                10,
                0,
                None,
                RPC_C_AUTHN_LEVEL_CALL,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                EOAC_NONE,
            ),
        )?;
        Some(services)
    }
}

fn first(services: &IWbemServices, query: &str) -> Option<IWbemClassObject> {
    // SAFETY: a WQL query on a live service; one object is taken.
    unsafe {
        let objects: IEnumWbemClassObject = checked(
            "ExecQuery",
            services.ExecQuery(
                &BSTR::from("WQL"),
                &BSTR::from(query),
                WBEM_GENERIC_FLAG_TYPE(WBEM_FLAG_FORWARD_ONLY.0 | WBEM_FLAG_RETURN_IMMEDIATELY.0),
                None,
            ),
        )?;
        let mut found = [None];
        let mut returned = 0u32;
        let status = objects.Next(WBEM_INFINITE, &mut found, &mut returned);
        if returned == 0 {
            eprintln!("brightness: {query}: nothing ({status:?})");
            return None;
        }
        found[0].take()
    }
}

fn read_in_process() -> Option<u8> {
    let services = services()?;
    let object = first(
        &services,
        "SELECT CurrentBrightness FROM WmiMonitorBrightness",
    )?;
    let mut value = VARIANT::default();
    // SAFETY: reads one property into a VARIANT this function owns.
    unsafe {
        checked(
            "CurrentBrightness",
            object.Get(w!("CurrentBrightness"), 0, &mut value, None, None),
        )?;
        u8::try_from(checked("VariantToInt32", VariantToInt32(&value))?.clamp(0, 100)).ok()
    }
}

fn write_in_process(level: u8) -> Option<u8> {
    let level = level.min(100);
    let services = services()?;
    let instance = first(&services, "SELECT * FROM WmiMonitorBrightnessMethods")?;
    // SAFETY: WMI calls on live objects; VARIANTs are owned here.
    unsafe {
        let mut path = VARIANT::default();
        instance.Get(w!("__PATH"), 0, &mut path, None, None).ok()?;
        let text = VariantToStringAlloc(&path).ok()?;
        let path = BSTR::from_wide(text.as_wide());
        windows::Win32::System::Com::CoTaskMemFree(Some(text.0 as *const core::ffi::c_void));
        let mut class = None;
        services
            .GetObject(
                &BSTR::from("WmiMonitorBrightnessMethods"),
                WBEM_GENERIC_FLAG_TYPE(0),
                None,
                Some(&mut class),
                None,
            )
            .ok()?;
        let class: IWbemClassObject = class?;
        let mut signature = None;
        class
            .GetMethod(
                w!("WmiSetBrightness"),
                0,
                &mut signature,
                std::ptr::null_mut(),
            )
            .ok()?;
        let parameters = signature?.SpawnInstance(0).ok()?;
        let timeout = int_variant(1);
        let brightness = int_variant(i32::from(level));
        parameters.Put(w!("Timeout"), 0, &timeout, 0).ok()?;
        parameters.Put(w!("Brightness"), 0, &brightness, 0).ok()?;
        services
            .ExecMethod(
                &path,
                &BSTR::from("WmiSetBrightness"),
                WBEM_GENERIC_FLAG_TYPE(0),
                None,
                &parameters,
                None,
                None,
            )
            .ok()?;
    }
    read_in_process().or(Some(level))
}

/// A `VT_I4` VARIANT holding `value`.
fn int_variant(value: i32) -> VARIANT {
    VARIANT {
        Anonymous: VARIANT_0 {
            Anonymous: std::mem::ManuallyDrop::new(VARIANT_0_0 {
                vt: VARENUM(VT_I4.0),
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: VARIANT_0_0_0 { lVal: value },
            }),
        },
    }
}
