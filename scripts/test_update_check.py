"""Tests for rmac-update-check, the daily Lulo OS update check.

The program runs as a subprocess against a fake `gi` package placed first on
PYTHONPATH. The fake PackageKitGlib, Gio and GLib replay a scripted scenario
(a JSON file named by RMAC_FAKE_GI_SCENARIO) and append every call they
receive to a JSON-lines log (RMAC_FAKE_GI_LOG), so each test can assert the
exact PackageKit transactions and D-Bus calls the program made.
"""

from __future__ import annotations

import json
import os
from pathlib import Path
import py_compile
import stat
import subprocess
import sys
import tempfile
import textwrap
import unittest


PROGRAM = Path(__file__).parent / "linux" / "rmac-update-check"

# Values verified on the Ubuntu 26.04 reference PC (PackageKit 1.3.4,
# gir1.2-packagekitglib-1.0): enum values and GError domains.
ONLY_TRUSTED = 1
ONLY_DOWNLOAD = 3
FILTER_NONE = 1
INFO_NORMAL = 5
INFO_SECURITY = 8
INFO_BLOCKED = 9
OFFLINE_REBOOT = 1
OFFLINE_UNSET = 3
CLIENT_DOMAIN = "pk-client-error-quark"
OFFLINE_DOMAIN = "pk-offline-error-quark"
ERROR_NO_NETWORK = 2
ERROR_GPG_FAILURE = 5
ERROR_NOT_AUTHORIZED = 48
TRANSACTION_ERROR_BASE = 0xFF


FAKE_GI_INIT = '''
def require_version(namespace, version):
    known = {"PackageKitGlib": "1.0", "Gio": "2.0", "GLib": "2.0"}
    if known.get(namespace) != version:
        raise ValueError("Namespace %s not available" % namespace)
'''

FAKE_REPOSITORY = textwrap.dedent(
    '''
    """Fake gi.repository: PackageKitGlib, Gio and GLib replaying a scenario."""

    import json
    import os
    import types

    with open(os.environ["RMAC_FAKE_GI_SCENARIO"], encoding="utf-8") as _file:
        SCENARIO = json.load(_file)


    def _log(call, **fields):
        fields["call"] = call
        with open(os.environ["RMAC_FAKE_GI_LOG"], "a", encoding="utf-8") as log:
            log.write(json.dumps(fields, sort_keys=True) + "\\n")


    class _Error(Exception):
        def __init__(self, domain, code, message="secret detail /home/user"):
            super().__init__(message)
            self.domain = domain
            self.code = code
            self.message = message


    def _maybe_raise(key):
        error = SCENARIO.get(key)
        if error:
            raise _Error(error["domain"], error["code"])


    GLib = types.ModuleType("GLib")
    GLib.Error = _Error


    class _Variant:
        def __init__(self, signature, value):
            self.signature = signature
            self.value = value


    class _VariantType:
        @staticmethod
        def new(signature):
            return signature


    GLib.Variant = _Variant
    GLib.VariantType = _VariantType


    class _Enum:
        def __init__(self, **values):
            self.__dict__.update(values)


    PackageKitGlib = types.ModuleType("PackageKitGlib")
    Pk = PackageKitGlib
    Pk.TransactionFlagEnum = _Enum(
        NONE=0, ONLY_TRUSTED=1, SIMULATE=2, ONLY_DOWNLOAD=3,
        ALLOW_REINSTALL=4, JUST_REINSTALL=5, ALLOW_DOWNGRADE=6,
    )
    Pk.FilterEnum = _Enum(UNKNOWN=0, NONE=1, INSTALLED=2)
    Pk.InfoEnum = _Enum(NORMAL=5, SECURITY=8, BLOCKED=9)
    Pk.ExitEnum = _Enum(UNKNOWN=0, SUCCESS=1, FAILED=2, CANCELLED=3)
    Pk.OfflineAction = _Enum(UNKNOWN=0, REBOOT=1, POWER_OFF=2, UNSET=3)
    Pk.ErrorEnum = _Enum(NO_NETWORK=2, GPG_FAILURE=5, NOT_AUTHORIZED=48)
    Pk.ClientError = _Enum(
        FAILED=0, FAILED_AUTH=1, NO_TID=2, ALREADY_TID=3, ROLE_UNKNOWN=4,
        CANNOT_START_DAEMON=5, INVALID_INPUT=6, INVALID_FILE=7,
        NOT_SUPPORTED=8, DECLINED_SIMULATION=9, DECLINED_INTERACTION=10,
    )
    Pk.OfflineError = _Enum(FAILED=0, INVALID_VALUE=1, NO_DATA=2)

    _ERROR_NAMES = {2: "no-network", 5: "gpg-failure", 48: "not-authorized"}
    _EXIT_NAMES = {1: "success", 2: "failed", 3: "cancelled"}


    def error_enum_to_string(code):
        return _ERROR_NAMES.get(code, "unknown")


    def exit_enum_to_string(code):
        return _EXIT_NAMES.get(code, "unknown")


    Pk.error_enum_to_string = error_enum_to_string
    Pk.exit_enum_to_string = exit_enum_to_string


    class _Package:
        def __init__(self, record):
            self._record = record

        def get_id(self):
            return self._record["id"]

        def get_name(self):
            return self._record["id"].split(";", 1)[0]

        def get_info(self):
            return self._record.get("info", 5)


    class _Results:
        def __init__(self, exit_code=1, packages=()):
            self._exit = exit_code
            self._packages = [_Package(record) for record in packages]

        def get_exit_code(self):
            return self._exit

        def get_package_array(self):
            return list(self._packages)


    def _check_callback(cancellable, progress_callback, user_data):
        assert cancellable is None
        assert callable(progress_callback)
        progress_callback(None, 0, user_data)


    class _Client:
        @staticmethod
        def new():
            _log("client_new")
            _maybe_raise("client_error")
            return _Client()

        def set_interactive(self, interactive):
            _log("set_interactive", value=interactive)

        def set_background(self, background):
            _log("set_background", value=background)

        def refresh_cache(self, force, cancellable, progress_callback, user_data):
            _check_callback(cancellable, progress_callback, user_data)
            _log("refresh_cache", force=force)
            _maybe_raise("refresh_error")
            return _Results(SCENARIO.get("refresh_exit", 1))

        def get_updates(self, filters, cancellable, progress_callback, user_data):
            _check_callback(cancellable, progress_callback, user_data)
            _log("get_updates", filters=filters)
            _maybe_raise("get_updates_error")
            return _Results(1, SCENARIO.get("updates", []))

        def update_packages(
            self, flags, package_ids, cancellable, progress_callback, user_data
        ):
            _check_callback(cancellable, progress_callback, user_data)
            _log("update_packages", flags=flags, package_ids=list(package_ids))
            _maybe_raise("update_error")
            return _Results(SCENARIO.get("update_exit", 1))


    Pk.Client = _Client


    def offline_get_prepared_ids():
        _log("offline_get_prepared_ids")
        prepared = SCENARIO.get("prepared_ids")
        if prepared is None:
            raise _Error("pk-offline-error-quark", 2, "No offline updates")
        return list(prepared)


    def offline_get_action():
        _log("offline_get_action")
        return SCENARIO.get("offline_action", 3)


    def offline_trigger(action, cancellable):
        _log("offline_trigger", action=action)
        _maybe_raise("trigger_error")
        return SCENARIO.get("trigger_result", True)


    Pk.offline_get_prepared_ids = offline_get_prepared_ids
    Pk.offline_get_action = offline_get_action
    Pk.offline_trigger = offline_trigger


    Gio = types.ModuleType("Gio")
    Gio.BusType = _Enum(SYSTEM=1, SESSION=2)
    Gio.DBusCallFlags = _Enum(NONE=0)


    class _Bus:
        def call_sync(
            self, name, path, interface, method, parameters, reply_type,
            flags, timeout, cancellable,
        ):
            _log(
                "dbus_call",
                name=name,
                path=path,
                interface=interface,
                method=method,
                signature=parameters.signature,
                parameters=list(parameters.value),
                reply_type=reply_type,
                timeout=timeout,
            )
            _maybe_raise("notify_error")
            return None


    def bus_get_sync(bus_type, cancellable):
        _log("bus_get_sync", bus_type=bus_type)
        _maybe_raise("bus_error")
        return _Bus()


    Gio.bus_get_sync = bus_get_sync
    '''
)


def pk_id(name: str, version: str = "1.2.0-1", origin: str = "rmac") -> str:
    return f"{name};{version};amd64;{origin}"


LULO_IDS = sorted(
    pk_id(name)
    for name in (
        "niri",
        "rmac-apps",
        "rmac-archive-keyring",
        "rmac-session",
        "xwayland-satellite",
    )
)
OTHER_IDS = [
    pk_id("bluez", "5.85-4ubuntu0.2", "ubuntu-resolute-updates-main"),
    pk_id("mesa-vulkan-drivers", "26.0.8-1", "ubuntu-resolute-updates-main"),
]
DOWNLOAD_FLAGS = (1 << ONLY_TRUSTED) | (1 << ONLY_DOWNLOAD)


def updates(ids, info=INFO_NORMAL):
    return [{"id": package_id, "info": info} for package_id in ids]


class Run:
    def __init__(self, result: subprocess.CompletedProcess, calls: list):
        self.returncode = result.returncode
        self.stdout = result.stdout
        self.stderr = result.stderr
        self.calls = calls

    def named(self, call: str) -> list:
        return [entry for entry in self.calls if entry["call"] == call]

    def notifications(self) -> list:
        return [
            entry
            for entry in self.named("dbus_call")
            if entry["method"] == "Notify"
        ]


def run_program(scenario: dict, *, with_packagekit: bool = True) -> Run:
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        package = root / "gi"
        repository = package / "repository"
        repository.mkdir(parents=True)
        (package / "__init__.py").write_text(FAKE_GI_INIT, encoding="utf-8")
        source = FAKE_REPOSITORY
        if not with_packagekit:
            source = source.replace(
                "PackageKitGlib = types.ModuleType",
                "raise ImportError('typelib PackageKitGlib not found')\n"
                "PackageKitGlib = types.ModuleType",
            )
        (repository / "__init__.py").write_text(source, encoding="utf-8")
        scenario_path = root / "scenario.json"
        scenario_path.write_text(json.dumps(scenario), encoding="utf-8")
        log_path = root / "calls.jsonl"
        log_path.write_text("", encoding="utf-8")
        environment = {
            "PATH": "/usr/bin:/bin",
            "HOME": str(root),
            "PYTHONPATH": str(root),
            "PYTHONDONTWRITEBYTECODE": "1",
            "RMAC_FAKE_GI_SCENARIO": str(scenario_path),
            "RMAC_FAKE_GI_LOG": str(log_path),
        }
        result = subprocess.run(
            [sys.executable, str(PROGRAM)],
            env=environment,
            check=False,
            capture_output=True,
            text=True,
            timeout=30,
        )
        calls = [
            json.loads(line)
            for line in log_path.read_text(encoding="utf-8").splitlines()
            if line
        ]
    return Run(result, calls)


class ProgramFileTests(unittest.TestCase):
    def test_program_compiles(self):
        with tempfile.TemporaryDirectory() as temporary:
            py_compile.compile(
                str(PROGRAM),
                cfile=str(Path(temporary) / "rmac-update-check.pyc"),
                doraise=True,
            )

    def test_program_is_an_executable_python3_script(self):
        mode = PROGRAM.stat().st_mode
        self.assertTrue(mode & stat.S_IXUSR)
        text = PROGRAM.read_text(encoding="utf-8")
        self.assertTrue(text.startswith("#!/usr/bin/python3\n"))
        self.assertNotIn("\r", text)

    def test_program_never_shells_out(self):
        text = PROGRAM.read_text(encoding="utf-8")
        for forbidden in ("subprocess", "os.system", "pkcon", "notify-send"):
            self.assertNotIn(forbidden, text)
        self.assertIn('gi.require_version("PackageKitGlib", "1.0")', text)


class UpdateCheckTests(unittest.TestCase):
    def assert_refreshed_non_interactively(self, run: Run) -> None:
        self.assertEqual(run.named("set_interactive"), [
            {"call": "set_interactive", "value": False}
        ])
        self.assertEqual(run.named("set_background"), [
            {"call": "set_background", "value": True}
        ])
        self.assertEqual(run.named("refresh_cache"), [
            {"call": "refresh_cache", "force": False}
        ])

    def assert_no_offline_work(self, run: Run) -> None:
        self.assertEqual(run.named("update_packages"), [])
        self.assertEqual(run.named("offline_trigger"), [])

    def test_no_updates_sends_no_notification(self):
        run = run_program({"updates": []})
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assert_refreshed_non_interactively(run)
        self.assertEqual(
            run.named("get_updates"),
            [{"call": "get_updates", "filters": 1 << FILTER_NONE}],
        )
        self.assert_no_offline_work(run)
        self.assertEqual(run.named("bus_get_sync"), [])
        self.assertEqual(run.stderr, "")

    def test_only_other_updates_send_a_count_notification(self):
        run = run_program({"updates": updates(OTHER_IDS)})
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assert_no_offline_work(run)
        [notification] = run.notifications()
        self.assertEqual(notification["name"], "org.freedesktop.Notifications")
        self.assertEqual(notification["path"], "/org/freedesktop/Notifications")
        self.assertEqual(notification["interface"], "org.freedesktop.Notifications")
        self.assertEqual(notification["signature"], "(susssasa{sv}i)")
        self.assertEqual(notification["reply_type"], "(u)")
        app, replaces, icon, summary, body, actions, hints, timeout = (
            notification["parameters"]
        )
        self.assertEqual(app, "Software Update")
        self.assertEqual(replaces, 0)
        self.assertEqual(icon, "software-update-available")
        self.assertEqual(summary, "Updates available")
        self.assertEqual(
            body,
            "2 updates are available — open System Settings to review "
            "and install them.",
        )
        self.assertEqual((actions, hints, timeout), ([], {}, -1))
        self.assertEqual(
            run.named("bus_get_sync"), [{"call": "bus_get_sync", "bus_type": 2}]
        )

    def test_single_other_update_uses_the_singular(self):
        run = run_program({"updates": updates(OTHER_IDS[:1])})
        self.assertEqual(run.returncode, 0, run.stderr)
        [notification] = run.notifications()
        self.assertTrue(
            notification["parameters"][4].startswith("1 update is available")
        )

    def test_lulo_updates_are_downloaded_trusted_and_triggered_for_reboot(self):
        run = run_program(
            {"updates": updates(LULO_IDS, INFO_SECURITY) + updates(OTHER_IDS)}
        )
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assert_refreshed_non_interactively(run)
        self.assertEqual(
            run.named("update_packages"),
            [
                {
                    "call": "update_packages",
                    "flags": DOWNLOAD_FLAGS,
                    "package_ids": LULO_IDS,
                }
            ],
        )
        self.assertEqual(
            run.named("offline_trigger"),
            [{"call": "offline_trigger", "action": OFFLINE_REBOOT}],
        )
        order = [entry["call"] for entry in run.calls]
        self.assertLess(order.index("update_packages"), order.index("offline_trigger"))
        [notification] = run.notifications()
        summary, body = notification["parameters"][3:5]
        self.assertEqual(summary, "Lulo OS update ready")
        self.assertEqual(
            body,
            "Lulo OS updates are ready — they will be installed the next "
            "time you restart. 2 other updates are available — open "
            "System Settings to review and install them.",
        )

    def test_only_lulo_updates_send_only_the_ready_sentence(self):
        run = run_program({"updates": updates(LULO_IDS[:2])})
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertEqual(
            run.named("update_packages")[0]["package_ids"], LULO_IDS[:2]
        )
        [notification] = run.notifications()
        self.assertEqual(
            notification["parameters"][4],
            "Lulo OS updates are ready — they will be installed the next "
            "time you restart.",
        )

    def test_download_flags_always_include_only_trusted(self):
        run = run_program({"updates": updates(LULO_IDS)})
        [call] = run.named("update_packages")
        self.assertTrue(call["flags"] & (1 << ONLY_TRUSTED))
        self.assertTrue(call["flags"] & (1 << ONLY_DOWNLOAD))
        self.assertEqual(call["flags"], DOWNLOAD_FLAGS)

    def test_similarly_named_packages_are_not_lulo_packages(self):
        lookalikes = [pk_id("niri-git"), pk_id("rmac-apps-dbg"), pk_id("xniri")]
        run = run_program({"updates": updates(lookalikes)})
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assert_no_offline_work(run)
        [notification] = run.notifications()
        self.assertTrue(
            notification["parameters"][4].startswith("3 updates are available")
        )

    def test_blocked_updates_are_ignored(self):
        run = run_program(
            {
                "updates": updates(LULO_IDS[:1], INFO_BLOCKED)
                + updates(OTHER_IDS[:1], INFO_BLOCKED)
                + updates(LULO_IDS[1:2])
                + updates(OTHER_IDS[1:])
            }
        )
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertEqual(
            run.named("update_packages")[0]["package_ids"], LULO_IDS[1:2]
        )
        [notification] = run.notifications()
        self.assertIn("1 other update is available", notification["parameters"][4])

    def test_only_blocked_updates_send_nothing(self):
        run = run_program(
            {"updates": updates(LULO_IDS + OTHER_IDS, INFO_BLOCKED)}
        )
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assert_no_offline_work(run)
        self.assertEqual(run.notifications(), [])

    def test_already_prepared_and_triggered_update_is_not_repeated(self):
        run = run_program(
            {
                "updates": updates(LULO_IDS),
                "prepared_ids": list(reversed(LULO_IDS)),
                "offline_action": OFFLINE_REBOOT,
            }
        )
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assert_no_offline_work(run)
        [notification] = run.notifications()
        self.assertEqual(notification["parameters"][3], "Lulo OS update ready")

    def test_prepared_but_untriggered_update_is_triggered_without_download(self):
        run = run_program(
            {
                "updates": updates(LULO_IDS),
                "prepared_ids": LULO_IDS,
                "offline_action": OFFLINE_UNSET,
            }
        )
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertEqual(run.named("update_packages"), [])
        self.assertEqual(len(run.named("offline_trigger")), 1)

    def test_a_different_prepared_set_is_downloaded_again(self):
        stale = [pk_id("rmac-apps", "1.1.0-1")]
        run = run_program(
            {
                "updates": updates(LULO_IDS),
                "prepared_ids": stale,
                "offline_action": OFFLINE_REBOOT,
            }
        )
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertEqual(
            run.named("update_packages")[0]["package_ids"], LULO_IDS
        )
        self.assertEqual(len(run.named("offline_trigger")), 1)

    def test_refresh_failure_fails_without_touching_updates(self):
        run = run_program(
            {
                "updates": updates(LULO_IDS),
                "refresh_error": {
                    "domain": CLIENT_DOMAIN,
                    "code": TRANSACTION_ERROR_BASE + ERROR_GPG_FAILURE,
                },
            }
        )
        self.assertEqual(run.returncode, 1)
        self.assertEqual(run.named("get_updates"), [])
        self.assert_no_offline_work(run)
        self.assertEqual(run.notifications(), [])
        self.assertEqual(
            run.stderr, "rmac-update-check: refresh failed: gpg-failure\n"
        )
        self.assertNotIn("secret", run.stderr)

    def test_refresh_that_does_not_succeed_is_a_failure(self):
        run = run_program({"updates": updates(LULO_IDS), "refresh_exit": 3})
        self.assertEqual(run.returncode, 1)
        self.assertEqual(
            run.stderr, "rmac-update-check: refresh failed: exit-cancelled\n"
        )
        self.assert_no_offline_work(run)

    def test_packagekit_unavailable_is_a_failure(self):
        run = run_program(
            {"refresh_error": {"domain": CLIENT_DOMAIN, "code": 5}}
        )
        self.assertEqual(run.returncode, 1)
        self.assertEqual(
            run.stderr, "rmac-update-check: refresh failed: cannot-start-daemon\n"
        )

    def test_dbus_error_is_classified_without_its_message(self):
        run = run_program(
            {"refresh_error": {"domain": "g-dbus-error-quark", "code": 2}}
        )
        self.assertEqual(run.returncode, 1)
        self.assertEqual(
            run.stderr, "rmac-update-check: refresh failed: g-dbus-error-quark-2\n"
        )

    def test_missing_packagekit_library_is_a_distinct_failure(self):
        run = run_program({}, with_packagekit=False)
        self.assertEqual(run.returncode, 2)
        self.assertIn("packagekit client library unavailable", run.stderr)
        self.assertEqual(run.calls, [])

    def test_download_authorization_failure_falls_back_to_system_settings(self):
        run = run_program(
            {
                "updates": updates(LULO_IDS) + updates(OTHER_IDS),
                "update_error": {
                    "domain": CLIENT_DOMAIN,
                    "code": TRANSACTION_ERROR_BASE + ERROR_NOT_AUTHORIZED,
                },
            }
        )
        # Non-zero: the automatic path did not happen and the unit should
        # say so, but the person still gets an actionable notification.
        self.assertEqual(run.returncode, 1)
        self.assertEqual(len(run.named("update_packages")), 1)
        self.assertEqual(run.named("offline_trigger"), [])
        self.assertIn("download failed: not-authorized", run.stderr)
        self.assertIn("System Settings > Software Update", run.stderr)
        [notification] = run.notifications()
        summary, body = notification["parameters"][3:5]
        self.assertEqual(summary, "Updates available")
        self.assertEqual(
            body,
            "7 updates are available — open System Settings to review "
            "and install them.",
        )

    def test_client_side_auth_refusal_is_also_an_authorization_failure(self):
        run = run_program(
            {
                "updates": updates(LULO_IDS),
                "update_error": {"domain": CLIENT_DOMAIN, "code": 1},
            }
        )
        self.assertEqual(run.returncode, 1)
        self.assertIn("download failed: failed-auth", run.stderr)
        self.assertIn("System Settings > Software Update", run.stderr)
        self.assertEqual(run.named("offline_trigger"), [])

    def test_download_failure_fails_and_falls_back(self):
        run = run_program(
            {
                "updates": updates(LULO_IDS),
                "update_error": {
                    "domain": CLIENT_DOMAIN,
                    "code": TRANSACTION_ERROR_BASE + ERROR_NO_NETWORK,
                },
            }
        )
        self.assertEqual(run.returncode, 1)
        self.assertEqual(run.named("offline_trigger"), [])
        self.assertEqual(
            run.stderr, "rmac-update-check: download failed: no-network\n"
        )
        [notification] = run.notifications()
        self.assertTrue(
            notification["parameters"][4].startswith("5 updates are available")
        )

    def test_trigger_failure_is_never_swallowed(self):
        run = run_program(
            {
                "updates": updates(LULO_IDS),
                "trigger_error": {"domain": OFFLINE_DOMAIN, "code": 0},
            }
        )
        self.assertEqual(run.returncode, 1)
        self.assertEqual(len(run.named("update_packages")), 1)
        self.assertEqual(len(run.named("offline_trigger")), 1)
        self.assertEqual(
            run.stderr, "rmac-update-check: trigger failed: offline-failed\n"
        )
        [notification] = run.notifications()
        self.assertEqual(notification["parameters"][3], "Updates available")
        self.assertNotIn("ready", notification["parameters"][4])

    def test_trigger_returning_false_is_a_failure(self):
        run = run_program({"updates": updates(LULO_IDS), "trigger_result": False})
        self.assertEqual(run.returncode, 1)
        self.assertEqual(
            run.stderr, "rmac-update-check: trigger failed: refused\n"
        )

    def test_missing_notification_service_is_not_fatal(self):
        run = run_program(
            {
                "updates": updates(LULO_IDS) + updates(OTHER_IDS),
                "notify_error": {
                    "domain": "g-dbus-error-quark",
                    "code": 2,
                },
            }
        )
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertEqual(len(run.named("offline_trigger")), 1)
        self.assertEqual(len(run.notifications()), 1)
        self.assertIn("notification not shown", run.stderr)

    def test_missing_session_bus_is_not_fatal(self):
        run = run_program(
            {
                "updates": updates(OTHER_IDS),
                "bus_error": {"domain": "g-io-error-quark", "code": 0},
            }
        )
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertIn("notification not shown", run.stderr)


if __name__ == "__main__":
    unittest.main()
