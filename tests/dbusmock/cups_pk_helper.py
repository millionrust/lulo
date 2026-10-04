"""cups-pk-helper (org.opensuse.CupsPkHelper.Mechanism) python-dbusmock
template.

Used by crates/rmac-printers-linux/tests/cups_pk_helper_mock.rs and by
scripts/behavior/fake_hardware.py. It records the queues Lulo asks for
instead of talking to cupsd: DevicesGet returns the flattened device
dictionary cups-pk-helper produces, PrinterAdd/PrinterDelete/
PrinterSetEnabled/JobCancelPurge update an in-memory table that
GetPrinters (on the mock interface) exposes to tests. SetAuthorized(False)
makes every method fail the way cups-pk-helper does when polkit refuses
(org.opensuse.CupsPkHelper.Mechanism.NotPrivileged).

Parameters (JSON): {"devices": {"device-uri:0": "...", ...}}.
"""

import dbus

from dbusmock import MOCK_IFACE

BUS_NAME = "org.opensuse.CupsPkHelper.Mechanism"
MAIN_OBJ = "/"
MAIN_IFACE = "org.opensuse.CupsPkHelper.Mechanism"
SYSTEM_BUS = True

STATE = {"authorized": True, "printers": {}, "cancelled": []}

DEFAULT_DEVICES = {
    "device-uri:0": "dnssd://Lulo%20Laser._ipp._tcp.local/?uuid=0e5c2c1a",
    "device-info:0": "Lulo Laser",
    "device-make-and-model:0": "Lulo Laser 400",
    "device-class:0": "network",
    "device-uri:1": "ipps://photo.local:631/ipp/print",
    "device-info:1": "Photo Printer",
    "device-make-and-model:1": "Photo 9",
    "device-class:1": "network",
    "device-uri:2": "usb://Old/Printer?serial=1",
    "device-info:2": "Old Printer",
    "device-class:2": "direct",
}


def _check():
    if not STATE["authorized"]:
        raise dbus.exceptions.DBusException(
            "Not authorized", name="org.opensuse.CupsPkHelper.Mechanism.NotPrivileged"
        )


def _devices_get(self, timeout, limit, include, exclude):
    _check()
    return ("", dbus.Dictionary(STATE["devices"], signature="ss"))


def _printer_add(self, name, uri, ppd, info, location):
    _check()
    if name in STATE["printers"]:
        return "client-error-not-possible"
    STATE["printers"][str(name)] = {
        "uri": str(uri),
        "ppd": str(ppd),
        "info": str(info),
        "location": str(location),
        "enabled": False,
        "accepting": False,
    }
    return ""


def _printer_delete(self, name):
    _check()
    if STATE["printers"].pop(str(name), None) is None:
        return "client-error-not-found"
    return ""


def _set_flag(flag):
    def set_flag(self, name, enabled, *rest):
        _check()
        printer = STATE["printers"].get(str(name))
        if printer is None:
            return "client-error-not-found"
        printer[flag] = bool(enabled)
        return ""

    return set_flag


def _job_cancel_purge(self, job_id, purge):
    _check()
    STATE["cancelled"].append(int(job_id))
    return ""


def load(mock, parameters):
    STATE["authorized"] = True
    STATE["printers"] = {}
    STATE["cancelled"] = []
    STATE["devices"] = dict(parameters.get("devices", DEFAULT_DEVICES))
    mock.AddMethods(
        MAIN_IFACE,
        [
            ("DevicesGet", "iiasas", "sa{ss}", _devices_get),
            ("PrinterAdd", "sssss", "s", _printer_add),
            ("PrinterDelete", "s", "s", _printer_delete),
            ("PrinterSetEnabled", "sb", "s", _set_flag("enabled")),
            ("PrinterSetAcceptJobs", "sbs", "s", _set_flag("accepting")),
            ("JobCancelPurge", "ib", "s", _job_cancel_purge),
        ],
    )


@dbus.service.method(MOCK_IFACE, in_signature="b", out_signature="")
def SetAuthorized(self, authorized):
    """Make every method succeed (True) or fail as polkit refusing (False)."""
    STATE["authorized"] = bool(authorized)


@dbus.service.method(MOCK_IFACE, in_signature="", out_signature="a{s(ssssbb)}")
def GetPrinters(self):
    """name → (uri, model, info, location, enabled, accepting) for every queue."""
    return dbus.Dictionary(
        {
            name: (p["uri"], p["ppd"], p["info"], p["location"], p["enabled"], p["accepting"])
            for name, p in STATE["printers"].items()
        },
        signature="s(ssssbb)",
    )


@dbus.service.method(MOCK_IFACE, in_signature="", out_signature="ai")
def GetCancelledJobs(self):
    return dbus.Array(STATE["cancelled"], signature="i")
