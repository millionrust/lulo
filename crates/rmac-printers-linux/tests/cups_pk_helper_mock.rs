//! Printer administration round trips against a python-dbusmock
//! cups-pk-helper (tests/dbusmock/cups_pk_helper.py) on a private bus.
//! Skipped when python3-dbusmock is missing unless RMAC_REQUIRE_DBUSMOCK=1.
#![cfg(target_os = "linux")]

#[path = "../../../tests/dbusmock/private_bus.rs"]
mod private_bus;

use std::collections::HashMap;

use rmac_printers_linux::{Error, PrinterAdmin};

type Queues = HashMap<String, (String, String, String, String, bool, bool)>;

fn admin() -> Option<(private_bus::MockBus, PrinterAdmin)> {
    let bus = private_bus::MockBus::start(
        "cups_pk_helper.py",
        "org.opensuse.CupsPkHelper.Mechanism",
        "{}",
    )?;
    let admin = PrinterAdmin::on_connection(bus.connection());
    Some((bus, admin))
}

#[test]
fn discovers_only_driverless_printers() {
    let Some((_bus, admin)) = admin() else {
        return;
    };
    let devices = admin.discover(5).unwrap();
    let names: Vec<_> = devices.iter().map(|device| device.display_name()).collect();
    assert_eq!(names, ["Lulo Laser", "Photo Printer"]);
}

#[test]
fn adds_an_ipp_everywhere_queue_enabled_and_accepting() {
    let Some((bus, admin)) = admin() else {
        return;
    };
    let devices = admin.discover(5).unwrap();
    admin
        .add_printer("Lulo_Laser", &devices[0], "Study")
        .unwrap();
    let queues: Queues = bus.call_mock("GetPrinters");
    let (uri, model, info, location, enabled, accepting) = &queues["Lulo_Laser"];
    assert!(uri.starts_with("dnssd://"));
    assert_eq!(model, "everywhere");
    assert_eq!(info, "Lulo Laser");
    assert_eq!(location, "Study");
    assert!(*enabled && *accepting);
    // A name CUPS would reject never reaches the helper.
    assert_eq!(
        admin.add_printer("Lulo Laser", &devices[0], ""),
        Err(Error::Failed)
    );
    // The helper's own CUPS error string is a failure.
    assert_eq!(
        admin.add_printer("Lulo_Laser", &devices[0], ""),
        Err(Error::Failed)
    );
}

#[test]
fn removes_pauses_and_cancels() {
    let Some((bus, admin)) = admin() else {
        return;
    };
    let devices = admin.discover(5).unwrap();
    admin.add_printer("Photo", &devices[1], "").unwrap();
    admin.set_enabled("Photo", false).unwrap();
    let queues: Queues = bus.call_mock("GetPrinters");
    assert!(!queues["Photo"].4);
    admin.cancel_job(42).unwrap();
    let cancelled: Vec<i32> = bus.call_mock("GetCancelledJobs");
    assert_eq!(cancelled, [42]);
    admin.delete_printer("Photo").unwrap();
    let queues: Queues = bus.call_mock("GetPrinters");
    assert!(queues.is_empty());
    assert_eq!(admin.delete_printer("Photo"), Err(Error::Failed));
    assert_eq!(admin.cancel_job(0), Err(Error::Failed));
}

#[test]
fn polkit_refusal_is_reported() {
    let Some((bus, admin)) = admin() else {
        return;
    };
    let devices = admin.discover(5).unwrap();
    bus.call_mock_with::<_, ()>("SetAuthorized", &(false,));
    assert_eq!(
        admin.add_printer("Lulo_Laser", &devices[0], ""),
        Err(Error::NotAuthorized)
    );
    assert_eq!(admin.discover(5), Err(Error::NotAuthorized));
    let queues: Queues = bus.call_mock("GetPrinters");
    assert!(queues.is_empty());
}
