"""Fake NetworkManager/BlueZ/UPower/backlight for the nested behaviour-test
session (docs/behavior-suite.md).

The private nested session (run_lulo.py and friends) has no real
NetworkManager, BlueZ, UPower or backlight: Control Centre, the top-bar
status menus (Wi-Fi, Bluetooth, Sound, Battery) and the matching System
Settings panes all ran against empty/unavailable data, which never
exercised the code paths the real laptop hits with real hardware -- one of
which panicked Control Centre on 2026-10-03.

`start(work)` spins up a *private* D-Bus system bus (never the real one;
`DBUS_SYSTEM_BUS_ADDRESS` only ever points the nested apps at it, through
the env dict callers merge in) and loads three python-dbusmock templates
onto it:

  * NetworkManager: a Wi-Fi device with three access points (two secured,
    one open), one of them active.
  * BlueZ: one adapter with one paired, connected device.
  * UPower: an 80%, discharging display battery.
  * AccountsService (tests/dbusmock/accounts_service.py): the account
    running the session (its real UID, so Settings treats it as the
    signed-in user, an administrator) and one standard user.
  * cups-pk-helper (tests/dbusmock/cups_pk_helper.py): driverless printers
    to discover, and a record of every printer change.

Printers are read from cupsd's socket, not D-Bus, so `fake_cupsd.py`
serves one idle printer and one waiting job on a scratch socket that
CUPS_SERVER points the nested apps at.

It also writes a scratch tree shaped like `/sys` (battery, backlight,
bluetooth and wifi classes) so `system-settings::hardware::current()` and
`rmac-osd`'s backlight reader -- which still read real sysfs paths by
default -- detect the same hardware when pointed at it via
`LULO_FAKE_SYS_ROOT`.

python3-dbusmock is an Ubuntu package (see AGENT-BRIEF) but is not
installed everywhere (e.g. this repo's reference laptop, which has no
sudo access for an agent to install it). `start()` degrades gracefully: if
the import fails, or any mock fails to start, it logs a warning and
returns `None` instead of raising, so every caller falls back to today's
behaviour (hardware stays unavailable) rather than failing the whole run.
"""

from __future__ import annotations

import json
import os
import pwd
import subprocess
import sys
from pathlib import Path
from typing import Optional


class FakeHardware:
    """A running set of hardware mocks plus the env vars that point the
    nested session's apps at them. Call `stop()` exactly once, from the
    same `finally` block that tears down the rest of the nested session."""

    def __init__(
        self,
        env: dict[str, str],
        processes: list[subprocess.Popen],
        logs: list,
        system_bus,
        cupsd=None,
    ) -> None:
        self.env = env
        self._processes = processes
        self._logs = logs
        self._system_bus = system_bus
        self._cupsd = cupsd

    def stop(self) -> None:
        if self._cupsd is not None:
            try:
                self._cupsd.stop()
            except Exception:
                pass
        for process in self._processes:
            try:
                process.terminate()
            except OSError:
                pass
        for process in self._processes:
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
            except Exception:
                pass
        for log in self._logs:
            try:
                log.close()
            except OSError:
                pass
        if self._system_bus is not None:
            try:
                self._system_bus.stop()
            except Exception:
                pass


def _write(path: Path, value: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(value)


def fake_sysfs(root: Path) -> None:
    """A scratch tree shaped like /sys: just enough for
    system-settings::hardware::scan() (battery/backlight/bluetooth/wifi
    class presence) and rmac-osd's backlight reader to see the same
    devices the D-Bus mocks below provide."""

    _write(root / "class/power_supply/BAT0/type", "Battery\n")
    _write(root / "class/backlight/intel_backlight/brightness", "120\n")
    _write(root / "class/backlight/intel_backlight/max_brightness", "255\n")
    _write(root / "class/backlight/intel_backlight/type", "firmware\n")
    _write(root / "class/bluetooth/hci0/address", "00:01:02:03:04:05\n")
    _write(root / "class/net/wlan0/wireless/empty", "")


def start(work: Path) -> Optional["FakeHardware"]:
    """Best-effort: start the mocks and return a `FakeHardware`, or `None`
    (after printing a warning) if python3-dbusmock is unavailable or any
    mock fails to start. Never raises."""

    try:
        import dbus
        import dbus.mainloop.glib
        import dbusmock
        from dbusmock.templates.bluez5 import BLUEZ_MOCK_IFACE
        from dbusmock.templates.networkmanager import (
            DeviceState,
            InfrastructureMode,
            NM80211ApSecurityFlags,
            NMActiveConnectionState,
            WIRELESS_DEVICE_IFACE,
        )
        from dbusmock.testcase import BusType, PrivateDBus, SpawnedMock
    except ImportError as error:
        print(
            f"fake_hardware: python3-dbusmock unavailable ({error}); hardware stays unavailable",
            file=sys.stderr,
        )
        return None

    dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)

    logs_dir = work / "logs"
    logs_dir.mkdir(exist_ok=True)
    system_bus = PrivateDBus(BusType.SYSTEM)
    processes: list[subprocess.Popen] = []
    logs: list = []

    try:
        system_bus.start()  # sets os.environ["DBUS_SYSTEM_BUS_ADDRESS"]

        nm_log = open(logs_dir / "fake-networkmanager.log", "w")
        logs.append(nm_log)
        nm_server = SpawnedMock.spawn_with_template(
            "networkmanager", {"WirelessEnabled": True}, BusType.SYSTEM, stdout=nm_log, stderr=nm_log,
        )
        processes.append(nm_server.process)
        nm_mock = dbus.Interface(nm_server.obj, dbusmock.MOCK_IFACE)
        wifi = nm_mock.AddWiFiDevice("wlan0", "wlan0", DeviceState.ACTIVATED)
        home_ap = nm_mock.AddAccessPoint(
            wifi, "Home_Network", "Casa Lulo", "AA:BB:CC:DD:EE:02",
            InfrastructureMode.NM_802_11_MODE_INFRA, 2437, 5400, 78,
            NM80211ApSecurityFlags.NM_802_11_AP_SEC_KEY_MGMT_PSK,
        )
        nm_mock.AddAccessPoint(
            wifi, "Neighbour", "Neighbour 5G", "AA:BB:CC:DD:EE:03",
            InfrastructureMode.NM_802_11_MODE_INFRA, 5240, 5400, 46,
            NM80211ApSecurityFlags.NM_802_11_AP_SEC_KEY_MGMT_PSK,
        )
        nm_mock.AddAccessPoint(
            wifi, "Open_Cafe", "Open Cafe", "AA:BB:CC:DD:EE:04",
            InfrastructureMode.NM_802_11_MODE_INFRA, 2462, 5400, 22,
            NM80211ApSecurityFlags.NM_802_11_AP_SEC_NONE,
        )
        home_connection = nm_mock.AddWiFiConnection(wifi, "Casa_Lulo", "Casa Lulo", "wpa-psk")
        active = nm_mock.AddActiveConnection(
            [wifi], home_connection, home_ap, "Casa_Lulo",
            NMActiveConnectionState.NM_ACTIVE_CONNECTION_STATE_ACTIVATED,
        )
        # AddWiFiDevice never declares an ActiveAccessPoint property, and
        # the generic Properties.Set (NM's own SetProperty included) can
        # only change a property that already exists -- it raises
        # UnknownProperty for one that doesn't. rmac-network reads
        # ActiveAccessPoint unconditionally
        # (crates/rmac-network/src/linux.rs), so leaving it unset would
        # make Wi-Fi report "unavailable" instead of showing the three
        # networks below with Casa Lulo connected. AddProperty (unlike
        # Set) creates a new property, but it has to be called on the
        # device object itself, not the manager object `nm_mock` is bound
        # to -- both mock objects live in the same spawned process.
        wifi_device = dbus.Interface(
            BusType.SYSTEM.get_connection().get_object("org.freedesktop.NetworkManager", wifi),
            dbusmock.MOCK_IFACE,
        )
        wifi_device.AddProperty(WIRELESS_DEVICE_IFACE, "ActiveAccessPoint", dbus.ObjectPath(home_ap))
        nm_mock.SetDeviceActive(wifi, active)

        bt_log = open(logs_dir / "fake-bluez.log", "w")
        logs.append(bt_log)
        bt_server = SpawnedMock.spawn_with_template(
            "bluez5", {}, BusType.SYSTEM, stdout=bt_log, stderr=bt_log,
        )
        processes.append(bt_server.process)
        # bluez5's convenience methods (unlike NetworkManager's and
        # UPower's) live on their own org.bluez.Mock interface, not the
        # generic org.freedesktop.DBus.Mock.
        bt_mock = dbus.Interface(bt_server.obj, BLUEZ_MOCK_IFACE)
        bt_mock.AddAdapter("hci0", "lulo-laptop")
        bt_mock.AddDevice("hci0", "AA:BB:CC:DD:EE:06", "Lulo Headphones")
        bt_mock.PairDevice("hci0", "AA:BB:CC:DD:EE:06")
        bt_mock.ConnectDevice("hci0", "AA:BB:CC:DD:EE:06")

        up_log = open(logs_dir / "fake-upower.log", "w")
        logs.append(up_log)
        up_server = SpawnedMock.spawn_with_template(
            "upower", {"OnBattery": True}, BusType.SYSTEM, stdout=up_log, stderr=up_log,
        )
        processes.append(up_server.process)
        up_mock = dbus.Interface(up_server.obj, dbusmock.MOCK_IFACE)
        # type=2 (BATTERY), state=2 (DISCHARGING), 80%, ~2h to empty,
        # warning_level=1 (NONE) -- see dbusmock/templates/upower.py.
        up_mock.SetupDisplayDevice(2, 2, 80.0, 80.0, 100.0, -8.0, 7200, 0, True, "battery-full-symbolic", 1)
    except Exception as error:  # dbusmock/dbus-python plumbing failure: degrade, never crash the run
        print(f"fake_hardware: failed to start mocks ({error}); hardware stays unavailable", file=sys.stderr)
        for process in processes:
            try:
                process.terminate()
                process.wait(timeout=5)
            except Exception:
                pass
        for log in logs:
            try:
                log.close()
            except OSError:
                pass
        try:
            system_bus.stop()
        except Exception:
            pass
        return None

    # Accounts and printers ride on the same private bus but must never take
    # the hardware mocks above down with them: a failure here only leaves
    # Users & Groups and Printers & Scanners unavailable.
    templates = Path(__file__).resolve().parents[2] / "tests" / "dbusmock"
    try:
        uid = os.getuid()
        try:
            account = pwd.getpwuid(uid)
            name, real_name = account.pw_name, (account.pw_gecos.split(",")[0] or account.pw_name)
        except KeyError:
            name, real_name = "lulo", "Lulo User"
        # Passed as one JSON string: dbus-python cannot marshal a list of
        # mixed-type dicts into the a{sv} AddTemplate takes.
        people = json.dumps([
            {"uid": uid, "name": name, "real_name": real_name, "admin": True},
            {"uid": uid + 7000, "name": "amy", "real_name": "Amy Brown", "admin": False},
        ])
        accounts_log = open(logs_dir / "fake-accountsservice.log", "w")
        logs.append(accounts_log)
        accounts = SpawnedMock.spawn_with_template(
            str(templates / "accounts_service.py"), {"users_json": people}, BusType.SYSTEM,
            stdout=accounts_log, stderr=accounts_log,
        )
        processes.append(accounts.process)

        printers_log = open(logs_dir / "fake-cups-pk-helper.log", "w")
        logs.append(printers_log)
        printers = SpawnedMock.spawn_with_template(
            str(templates / "cups_pk_helper.py"), {}, BusType.SYSTEM,
            stdout=printers_log, stderr=printers_log,
        )
        processes.append(printers.process)
    except Exception as error:  # accounts/printers stay unavailable; never fail the run
        print(f"fake_hardware: accounts/printer mocks unavailable ({error})", file=sys.stderr)

    sys_root = work / "fake-sys"
    fake_sysfs(sys_root)

    env = {
        "DBUS_SYSTEM_BUS_ADDRESS": system_bus.address,
        "LULO_FAKE_SYS_ROOT": str(sys_root),
    }
    cupsd = None
    try:
        sys.path.insert(0, str(Path(__file__).resolve().parent))
        import fake_cupsd

        cupsd = fake_cupsd.FakeCupsd(work / "fake-cups.sock")
        env["CUPS_SERVER"] = str(cupsd.socket_path)
    except Exception as error:  # printers stay "unavailable"; never fail the run
        print(f"fake_hardware: fake cupsd unavailable ({error})", file=sys.stderr)
    return FakeHardware(env, processes, logs, system_bus, cupsd)
