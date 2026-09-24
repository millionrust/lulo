"""Fixture tests for package-owned rmac session integration."""

from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import stat
import subprocess
import shutil
import sys
import tempfile
import unittest
import wave


def load_script(name: str, filename: str):
    script = Path(__file__).parent / "linux" / filename
    spec = importlib.util.spec_from_file_location(name, script)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


stage_package = load_script("stage_session_package", "stage-session-package.py")
verify_package = load_script("verify_session_package", "verify-session-package.py")


def write_executable(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
    path.chmod(0o755)


def add_runtime_fixture(root: Path, *, include_gnome: bool) -> None:
    for executable in (
        "niri-session",
        "systemctl",
        "install",
        "awk",
        "sleep",
    ):
        write_executable(root / "usr/bin" / executable)
    write_executable(root / "usr/sbin/gdm3")
    for executable in verify_package.REQUIRED_RMAC_EXECUTABLES:
        write_executable(root / "usr/libexec/rmac" / executable)
    if include_gnome:
        session = root / "usr/share/wayland-sessions/ubuntu.desktop"
        session.write_text(
            "[Desktop Entry]\n"
            "Name=Ubuntu\n"
            "Exec=/usr/bin/gnome-session\n"
            "Type=Application\n"
            "DesktopNames=ubuntu;GNOME\n",
            encoding="utf-8",
        )
        session.chmod(0o644)


def rendered_session_wrapper(root: Path) -> Path:
    source = (
        stage_package.REPO_ROOT / "packaging/rmac-session/rmac-wayland-session"
    ).read_text(encoding="utf-8")
    source = source.replace("/usr/libexec/rmac/", f"{root}/usr/libexec/rmac/")
    source = source.replace("/usr/share/rmac/niri", f"{root}/usr/share/rmac/niri")
    source = source.replace("/usr/bin/", f"{root}/usr/bin/")
    wrapper = root / "wrapper"
    wrapper.write_text(source, encoding="utf-8")
    wrapper.chmod(0o755)
    return wrapper


def rendered_start_script(root: Path) -> Path:
    source = (
        stage_package.REPO_ROOT / "scripts/linux/start-rmac-session.sh"
    ).read_text(encoding="utf-8")
    source = source.replace(
        "/usr/share/rmac/session",
        str(root / "usr/share/rmac/session"),
    )
    source = source.replace("/usr/bin/", f"{root}/usr/bin/")
    source = source.replace("/usr/libexec/rmac/", f"{root}/usr/libexec/rmac/")
    script = root / "start-session"
    script.write_text(source, encoding="utf-8")
    script.chmod(0o755)
    return script


def write_program(path: Path, contents: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("#!/bin/sh\n" + contents, encoding="utf-8")
    path.chmod(0o755)


def add_wrapper_config_fixture(root: Path) -> None:
    packaged = root / "usr/share/rmac/niri/config.kdl"
    packaged.parent.mkdir(parents=True, exist_ok=True)
    packaged.write_text('include "/usr/share/rmac/niri/shell.kdl"\n', encoding="utf-8")
    for executable in ("install", "mv"):
        target = shutil.which(executable)
        assert target is not None
        write_program(root / "usr/bin" / executable, f'exec "{target}" "$@"\n')


class SessionPackageTests(unittest.TestCase):
    def stage(self, parent: Path) -> Path:
        root = parent / "package-root"
        stage_package.stage(root)
        return root

    def test_stages_and_verifies_deterministic_system_only_payload(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = self.stage(Path(temporary))
            verify_package.verify_tree(root)

            manifest = json.loads(
                (root / verify_package.MANIFEST).read_text(encoding="utf-8")
            )
            paths = [entry["path"] for entry in manifest["files"]]
            self.assertEqual(paths, sorted(paths))
            self.assertEqual(
                [path for path in paths if not path.startswith("/usr/")],
                ["/etc/fonts/conf.d/99-rmac.conf", "/etc/pam.d/rmac-lock"],
            )
            self.assertFalse(any("/home/" in path or "/root/" in path for path in paths))
            self.assertFalse(
                any(
                    component.lower() in {"gnome", "ubuntu"}
                    for path in paths
                    for component in Path(path).parts
                )
            )
            font_policy = (root / "etc/fonts/conf.d/99-rmac.conf").read_text(
                encoding="utf-8"
            )
            for feature in ("tnum", "cv08", "ss03"):
                self.assertIn(f"<string>{feature}</string>", font_policy)
            sound_files = sorted(
                path.name for path in (root / "usr/share/rmac/sounds").glob("*.wav")
            )
            self.assertEqual(sound_files, sorted(verify_package._SOUND_FILES))
            self.assertTrue(
                all(
                    (root / "usr/share/rmac/sounds" / name).stat().st_size > 44
                    for name in sound_files
                )
            )
            for name in sound_files:
                with wave.open(
                    str(root / "usr/share/rmac/sounds" / name), "rb"
                ) as sound:
                    sound.setpos(sound.getnframes() - 1)
                    self.assertEqual(sound.readframes(1), b"\0\0")

            wallpapers = root / "usr/share/rmac/wallpapers"
            self.assertEqual(
                sorted(path.name for path in wallpapers.iterdir()),
                sorted(verify_package._WALLPAPER_FILES),
            )
            for name in verify_package._WALLPAPER_FILES:
                data = (wallpapers / name).read_bytes()
                # Every wallpaper is a JPEG within the per-image size budget.
                self.assertTrue(data.startswith(b"\xff\xd8\xff"), name)
                self.assertLess(len(data), 3 * 1024 * 1024, name)
            # Each built-in pairs a light and a dark image at every size.
            for name in verify_package._WALLPAPER_FILES:
                if "-light-" in name:
                    self.assertTrue(
                        (wallpapers / name.replace("-light-", "-dark-")).is_file()
                    )

            wrapper = root / "usr/libexec/rmac/rmac-wayland-session"
            self.assertEqual(stat.S_IMODE(wrapper.stat().st_mode), 0o755)
            unit = (
                root / "usr/lib/systemd/user/rmac-notification-center.service"
            ).read_text(encoding="utf-8")
            self.assertIn("/usr/libexec/rmac/rmac-notification-center", unit)
            self.assertNotIn("%h/.local/libexec", unit)
            shell = (root / "usr/share/rmac/niri/shell.kdl").read_text(
                encoding="utf-8"
            )
            self.assertLess(
                shell.index('workspace "Desktop"'),
                shell.index('workspace "rmac-parking"'),
            )

    def test_ships_the_rmac_gtk_theme_and_desktop_scoped_defaults(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = self.stage(Path(temporary))
            theme = root / "usr/share/themes/rmac"
            for relative in (
                "index.theme",
                "palette-dark.css",
                "palette-light.css",
                "libadwaita.css",
                "gtk-3.0/gtk.css",
                "gtk-3.0/gtk-dark.css",
                "gtk-3.0/rmac.css",
                "gtk-4.0/gtk.css",
                "gtk-4.0/gtk-dark.css",
                "gtk-4.0/rmac.css",
            ):
                self.assertTrue((theme / relative).is_file(), relative)
            dark = (theme / "palette-dark.css").read_text(encoding="utf-8")
            self.assertIn("@define-color rmac_accent #1372F9;", dark)
            self.assertIn(
                '@import url("../palette-dark.css");',
                (theme / "gtk-3.0/gtk-dark.css").read_text(encoding="utf-8"),
            )
            override = root / "usr/share/glib-2.0/schemas/91_rmac-desktop.gschema.override"
            text = override.read_text(encoding="utf-8")
            self.assertIn("[org.gnome.desktop.interface:rmac]\n", text)
            self.assertIn("button-layout='close,minimize,maximize:'\n", text)

            # Widening the defaults past the rmac desktop is refused.
            override.write_text(
                text.replace(
                    "[org.gnome.desktop.interface:rmac]", "[org.gnome.desktop.interface]"
                ),
                encoding="utf-8",
            )
            with self.assertRaisesRegex(
                verify_package.VerificationError, "installed content differs"
            ):
                verify_package.verify_tree(root)
            with self.assertRaisesRegex(
                verify_package.VerificationError, "leaves the rmac desktop"
            ):
                verify_package._verify_desktop_override(override)

    def test_refuses_live_root_relative_and_nonempty_destinations(self):
        with self.assertRaises(stage_package.PackageError):
            stage_package.stage(Path("/"))
        with self.assertRaises(stage_package.PackageError):
            stage_package.stage(Path("relative"))
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "package-root"
            root.mkdir()
            (root / "owned").write_text("leave me alone", encoding="utf-8")
            with self.assertRaises(stage_package.PackageError):
                stage_package.stage(root)
            self.assertEqual((root / "owned").read_text(encoding="utf-8"), "leave me alone")

    def test_verifier_rejects_tampering_and_symlink_substitution(self):
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary)
            root = self.stage(parent)
            desktop = root / "usr/share/wayland-sessions/rmac.desktop"
            desktop.write_text("[Desktop Entry]\nType=Application\n", encoding="utf-8")
            with self.assertRaisesRegex(
                verify_package.VerificationError, "installed content differs"
            ):
                verify_package.verify_tree(root)

            root = parent / "second-root"
            stage_package.stage(root)
            desktop = root / "usr/share/wayland-sessions/rmac.desktop"
            desktop.unlink()
            desktop.symlink_to(root / "usr/share/rmac/session/lock-policy.json")
            with self.assertRaisesRegex(
                verify_package.VerificationError, "not a regular file"
            ):
                verify_package.verify_tree(root)

            root = parent / "third-root"
            stage_package.stage(root)
            extra = root / "usr/share/rmac/unclaimed"
            extra.write_text("unexpected\n", encoding="utf-8")
            with self.assertRaisesRegex(
                verify_package.VerificationError, "unexpected path"
            ):
                verify_package.verify_tree(root)

    def test_installed_gate_requires_runtimes_and_separate_gnome_recovery(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = self.stage(Path(temporary))
            with self.assertRaisesRegex(
                verify_package.VerificationError, "runtime executable is missing"
            ):
                verify_package.verify_installed_host(root)

            add_runtime_fixture(root, include_gnome=False)
            with self.assertRaisesRegex(
                verify_package.VerificationError, "GNOME Wayland recovery"
            ):
                verify_package.verify_installed_host(root)

            session = root / "usr/share/wayland-sessions/other.desktop"
            session.write_text(
                "[Desktop Entry]\n"
                "Name=Other\n"
                "Exec=/usr/bin/other\n"
                "Type=Application\n"
                "DesktopNames=not-gnome\n",
                encoding="utf-8",
            )
            with self.assertRaisesRegex(
                verify_package.VerificationError, "GNOME Wayland recovery"
            ):
                verify_package.verify_installed_host(root)

            add_runtime_fixture(root, include_gnome=True)
            verify_package.verify_installed_host(root)

    def test_recovery_only_gate_needs_no_rmac_package(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            sessions = root / "usr/share/wayland-sessions"
            sessions.mkdir(parents=True)
            (sessions / "rmac.desktop").write_text(
                "[Desktop Entry]\nDesktopNames=rmac;niri\n",
                encoding="utf-8",
            )
            with self.assertRaisesRegex(
                verify_package.VerificationError, "GNOME Wayland recovery"
            ):
                verify_package.verify_recovery_session(root)

            add_runtime_fixture(root, include_gnome=True)
            verify_package.verify_recovery_session(root)

    def test_manifest_cannot_claim_recovery_desktop_or_user_data(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = self.stage(Path(temporary))
            manifest_path = root / verify_package.MANIFEST
            document = json.loads(manifest_path.read_text(encoding="utf-8"))
            document["files"][0]["path"] = "/usr/share/gnome/session.desktop"
            document["files"].sort(key=lambda entry: entry["path"])
            manifest_path.write_text(
                json.dumps(document, sort_keys=True) + "\n",
                encoding="utf-8",
            )
            with self.assertRaisesRegex(
                verify_package.VerificationError, "recovery-desktop path"
            ):
                verify_package.verify_tree(root)

    def test_session_wrapper_starts_normal_and_safe_modes_with_exact_identity(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            add_wrapper_config_fixture(root)
            write_program(
                root / "usr/bin/niri-session",
                ': >"$RMAC_TEST_ROOT/niri-running"\n'
                "/bin/sleep 0.8\n"
                'rm -f "$RMAC_TEST_ROOT/niri-running"\n',
            )
            write_program(
                root / "usr/bin/systemctl",
                'printf "%s\\n" "$*" >>"$RMAC_TEST_SYSTEMCTL"\n'
                'case "$*" in\n'
                '  *"is-active"*) [ -e "$RMAC_TEST_ROOT/niri-running" ] ;;\n'
                '  *"show-environment"*)\n'
                '    echo "XDG_CURRENT_DESKTOP=niri"\n'
                '    echo "XDG_SESSION_DESKTOP=niri"\n'
                '    echo "XDG_SESSION_TYPE=wayland"\n'
                '    [ -z "${NIRI_CONFIG-}" ] || echo "NIRI_CONFIG=$NIRI_CONFIG"\n'
                '    echo "WAYLAND_DISPLAY=wayland-9"\n'
                '    echo "NIRI_SOCKET=$XDG_RUNTIME_DIR/niri.wayland-9.42.sock"\n'
                '    exit 0 ;;\n'
                "  *) exit 0 ;;\n"
                "esac\n",
            )
            for executable in ("awk", "sleep"):
                target = shutil.which(executable)
                assert target is not None
                write_program(
                    root / "usr/bin" / executable,
                    f'exec "{target}" "$@"\n',
                )
            write_program(
                root / "usr/libexec/rmac/rmac-session-start",
                'printf "%s|%s|%s|%s|%s|%s\\n" "$XDG_CURRENT_DESKTOP" '
                '"$XDG_SESSION_DESKTOP" "${NIRI_CONFIG-}" '
                '"$WAYLAND_DISPLAY" "$NIRI_SOCKET" '
                '"$*" >"$RMAC_TEST_CAPTURE"\n',
            )
            wrapper = rendered_session_wrapper(root)

            capture = root / "normal"
            systemctl_capture = root / "systemctl.log"
            environment = os.environ.copy()
            environment.update(
                {
                    "HOME": str(root / "home"),
                    "XDG_STATE_HOME": str(root / "state"),
                    "XDG_RUNTIME_DIR": str(root / "runtime"),
                    "RMAC_TEST_CAPTURE": str(capture),
                    "RMAC_TEST_SYSTEMCTL": str(systemctl_capture),
                    "RMAC_TEST_ROOT": str(root),
                }
            )
            result = subprocess.run(
                [str(wrapper)],
                env=environment,
                check=False,
                capture_output=True,
                text=True,
                timeout=5,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                capture.read_text(encoding="utf-8"),
                f"rmac:niri|rmac|{root}/home/.config/rmac/niri/session.kdl|"
                f"wayland-9|{root}/runtime/niri.wayland-9.42.sock|"
                "--system-package\n",
            )
            user_config = root / "home/.config/rmac/niri/config.kdl"
            self.assertEqual(
                user_config.read_text(encoding="utf-8"),
                'include "/usr/share/rmac/niri/shell.kdl"\n',
            )
            self.assertEqual(stat.S_IMODE(user_config.stat().st_mode), 0o600)
            session_config = root / "home/.config/rmac/niri/session.kdl"
            self.assertEqual(
                session_config.read_text(encoding="utf-8"),
                'include "config.kdl"\ninclude "shortcuts-generated.kdl"\n',
            )
            generated_shortcuts = root / "home/.config/rmac/niri/shortcuts-generated.kdl"
            self.assertEqual(generated_shortcuts.read_text(encoding="utf-8"), "")
            self.assertEqual(stat.S_IMODE(generated_shortcuts.stat().st_mode), 0o600)
            cleanup = systemctl_capture.read_text(encoding="utf-8")
            self.assertIn("--user stop waybar.service", cleanup)
            for unit in (
                "rmac-session.target",
                "rmac-safe-mode.target",
                "rmac-idle-lock.service",
                "rmac-lock-coordinator.service",
                "rmac-session-supervisor.service",
            ):
                self.assertIn(unit, cleanup)

            # Without a runnable supervisor the wrapper still consumes the
            # marker itself: safe mode lasts exactly one login.
            marker = root / "state/rmac/session/safe-mode.json"
            marker.parent.mkdir(parents=True)
            marker.write_text("{}\n", encoding="utf-8")
            session_config.unlink()
            capture = root / "safe"
            environment["RMAC_TEST_CAPTURE"] = str(capture)
            result = subprocess.run(
                [str(wrapper)],
                env=environment,
                check=False,
                capture_output=True,
                text=True,
                timeout=5,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                capture.read_text(encoding="utf-8"),
                f"niri|niri||wayland-9|{root}/runtime/niri.wayland-9.42.sock|"
                "--system-package --safe-mode\n",
            )
            self.assertIn("next login starts normally", result.stderr)
            self.assertFalse(marker.exists())
            self.assertEqual(
                (marker.parent / "safe-mode.last.json").read_text(encoding="utf-8"),
                "{}\n",
            )
            # Safe mode keeps the rmac entry point ready for --clear-safe-mode.
            self.assertEqual(
                session_config.read_text(encoding="utf-8"),
                'include "config.kdl"\ninclude "shortcuts-generated.kdl"\n',
            )

            capture = root / "after-safe"
            environment["RMAC_TEST_CAPTURE"] = str(capture)
            result = subprocess.run(
                [str(wrapper)],
                env=environment,
                check=False,
                capture_output=True,
                text=True,
                timeout=5,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(
                capture.read_text(encoding="utf-8").startswith("rmac:niri|rmac|"),
                "the login after a safe login must start normally",
            )

            # The supervisor decides when it can run: a rebuilt component
            # skips the marker, and a safe decision is passed through.
            supervisor = root / "usr/libexec/rmac/rmac-session-supervisor"
            for decision, expected in (
                ("normal", "rmac:niri|rmac|"),
                ("safe", "niri|niri||"),
            ):
                write_program(
                    supervisor,
                    '[ "$*" = begin-login ] || exit 9\n'
                    f'rm -f "{marker}"\n'
                    f"echo {decision}\n",
                )
                marker.write_text("{}\n", encoding="utf-8")
                capture = root / f"supervised-{decision}"
                environment["RMAC_TEST_CAPTURE"] = str(capture)
                result = subprocess.run(
                    [str(wrapper)],
                    env=environment,
                    check=False,
                    capture_output=True,
                    text=True,
                    timeout=5,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertTrue(
                    capture.read_text(encoding="utf-8").startswith(expected),
                    decision,
                )
                self.assertFalse(marker.exists())

    def leftover_desktop_fixture(self, root: Path, other_state: str) -> dict[str, str]:
        add_wrapper_config_fixture(root)
        write_program(
            root / "usr/bin/niri-session",
            ': >"$RMAC_TEST_ROOT/niri-running"\n'
            "/bin/sleep 0.8\n"
            'rm -f "$RMAC_TEST_ROOT/niri-running"\n',
        )
        # An earlier login's niri and rmac units still run in the shared
        # user manager until something stops them.
        (root / "leftover").write_text("", encoding="utf-8")
        write_program(
            root / "usr/bin/systemctl",
            'printf "%s\\n" "$*" >>"$RMAC_TEST_SYSTEMCTL"\n'
            'case "$*" in\n'
            '  *"stop --no-block"*) rm -f "$RMAC_TEST_ROOT/leftover" ;;\n'
            '  *"is-active"*)\n'
            '    [ -e "$RMAC_TEST_ROOT/leftover" ] ||\n'
            '      [ -e "$RMAC_TEST_ROOT/niri-running" ] ;;\n'
            '  *"show-environment"*)\n'
            '    echo "XDG_SESSION_TYPE=wayland"\n'
            '    [ -z "${NIRI_CONFIG-}" ] || echo "NIRI_CONFIG=$NIRI_CONFIG"\n'
            '    echo "WAYLAND_DISPLAY=wayland-9"\n'
            '    echo "NIRI_SOCKET=$XDG_RUNTIME_DIR/niri.wayland-9.42.sock"\n'
            '    exit 0 ;;\n'
            "  *) exit 0 ;;\n"
            "esac\n",
        )
        write_program(
            root / "usr/bin/loginctl",
            'printf "%s\\n" "$*" >>"$RMAC_TEST_LOGINCTL"\n'
            'case "$*" in\n'
            '  "show-user 1000 --property=Sessions --value") echo "2 7" ;;\n'
            '  "show-session 2 --property=Type --value") echo wayland ;;\n'
            f'  "show-session 2 --property=State --value") echo {other_state} ;;\n'
            '  "show-session 2 --property=Seat --value") echo seat0 ;;\n'
            "  *) exit 1 ;;\n"
            "esac\n",
        )
        write_program(root / "usr/bin/id", '[ "$*" = -u ] && echo 1000\n')
        for executable in ("awk", "sleep"):
            target = shutil.which(executable)
            assert target is not None
            write_program(root / "usr/bin" / executable, f'exec "{target}" "$@"\n')
        write_program(
            root / "usr/libexec/rmac/rmac-session-start",
            'printf "%s\\n" "$*" >"$RMAC_TEST_CAPTURE"\n',
        )
        return {
            **os.environ,
            "HOME": str(root / "home"),
            "XDG_STATE_HOME": str(root / "state"),
            "XDG_RUNTIME_DIR": str(root / "runtime"),
            "XDG_SESSION_ID": "7",
            "RMAC_TEST_ROOT": str(root),
            "RMAC_TEST_CAPTURE": str(root / "started"),
            "RMAC_TEST_SYSTEMCTL": str(root / "systemctl.log"),
            "RMAC_TEST_LOGINCTL": str(root / "loginctl.log"),
        }

    def test_session_wrapper_stops_a_desktop_left_by_an_earlier_login(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            environment = self.leftover_desktop_fixture(root, "closing")
            result = subprocess.run(
                [str(rendered_session_wrapper(root))],
                env=environment,
                check=False,
                capture_output=True,
                text=True,
                timeout=15,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("left running by an earlier login", result.stderr)
            calls = (root / "systemctl.log").read_text(encoding="utf-8")
            stop = next(
                line for line in calls.splitlines() if "stop --no-block" in line
            )
            for unit in ("niri.service", "rmac-session.target", "rmac-safe-mode.target"):
                self.assertIn(unit, stop)
            self.assertIn("reset-failed niri.service", calls)
            # Cleanup happens before this login's compositor starts.
            self.assertLess(
                calls.index("stop --no-block"), calls.index("show-environment")
            )
            self.assertEqual(
                (root / "started").read_text(encoding="utf-8"),
                "--system-package\n",
            )

    def test_session_wrapper_refuses_while_another_desktop_is_in_the_foreground(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            environment = self.leftover_desktop_fixture(root, "active")
            result = subprocess.run(
                [str(rendered_session_wrapper(root))],
                env=environment,
                check=False,
                capture_output=True,
                text=True,
                timeout=15,
            )
            self.assertEqual(result.returncode, 1)
            self.assertIn("still logged in to another desktop", result.stderr)
            self.assertIn("session 2 on seat0", result.stderr)
            calls = (root / "systemctl.log").read_text(encoding="utf-8")
            self.assertNotIn("stop", calls)
            self.assertFalse((root / "started").exists())
            self.assertTrue((root / "leftover").exists())

    def test_session_wrapper_rejects_clean_exit_before_readiness(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            add_wrapper_config_fixture(root)
            write_program(root / "usr/bin/niri-session", "exit 0\n")
            write_program(
                root / "usr/bin/systemctl",
                'case "$*" in\n'
                '  "--user import-environment NIRI_CONFIG RMAC_COLOR_SCHEME") exit 0 ;;\n'
                '  *) exit 1 ;;\n'
                "esac\n",
            )
            for executable in ("awk", "sleep"):
                target = shutil.which(executable)
                assert target is not None
                write_program(
                    root / "usr/bin" / executable,
                    f'exec "{target}" "$@"\n',
                )
            write_program(
                root / "usr/libexec/rmac/rmac-session-start",
                "exit 0\n",
            )
            wrapper = rendered_session_wrapper(root)
            result = subprocess.run(
                [str(wrapper)],
                env={
                    **os.environ,
                    "HOME": str(root / "home"),
                    "XDG_STATE_HOME": str(root / "state"),
                },
                check=False,
                capture_output=True,
                text=True,
                timeout=5,
            )
            self.assertEqual(result.returncode, 1)
            self.assertIn("exited before", result.stderr)

    def test_packaged_start_provisions_only_missing_private_defaults(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            defaults = root / "usr/share/rmac/session"
            defaults.mkdir(parents=True)
            (defaults / "swaylock.conf").write_text("packaged lock\n", encoding="utf-8")
            (defaults / "lock-policy.json").write_text("{}\n", encoding="utf-8")
            config = root / "config/rmac"
            config.mkdir(parents=True)
            (config / "swaylock.conf").write_text("user lock\n", encoding="utf-8")
            log = root / "systemctl.log"
            write_program(
                root / "usr/bin/systemctl",
                'printf "%s|%s\\n" "$XDG_CURRENT_DESKTOP" "$*" >>"$RMAC_TEST_LOG"\n',
            )
            write_program(
                root / "usr/bin/dbus-update-activation-environment",
                "exit 0\n",
            )
            install = shutil.which("install")
            assert install is not None
            write_program(
                root / "usr/bin/install",
                f'exec "{install}" "$@"\n',
            )
            script = rendered_start_script(root)
            environment = {
                **os.environ,
                "HOME": str(root / "home"),
                "PATH": "/usr/bin:/bin",
                "XDG_CONFIG_HOME": str(root / "config"),
                "XDG_STATE_HOME": str(root / "state"),
                "XDG_CURRENT_DESKTOP": "niri",
                "XDG_SESSION_TYPE": "wayland",
                "WAYLAND_DISPLAY": "wayland-1",
                "RMAC_TEST_LOG": str(log),
            }
            result = subprocess.run(
                [str(script), "--system-package"],
                env=environment,
                check=False,
                capture_output=True,
                text=True,
                timeout=5,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                (config / "swaylock.conf").read_text(encoding="utf-8"),
                "user lock\n",
            )
            policy = config / "lock-policy.json"
            self.assertEqual(policy.read_text(encoding="utf-8"), "{}\n")
            self.assertEqual(stat.S_IMODE(policy.stat().st_mode), 0o600)
            lines = log.read_text(encoding="utf-8").splitlines()
            self.assertTrue(
                any(
                    line.startswith("rmac:niri|")
                    and "start rmac-session.target" in line
                    for line in lines
                )
            )
            self.assertTrue(any("try-restart xdg-desktop-portal.service" in line for line in lines))

            log.write_text("", encoding="utf-8")
            nongraphical = {
                **environment,
                "XDG_CURRENT_DESKTOP": "ssh",
                "XDG_SESSION_TYPE": "tty",
            }
            nongraphical.pop("WAYLAND_DISPLAY")
            result = subprocess.run(
                [str(script), "--system-package"],
                env=nongraphical,
                check=False,
                capture_output=True,
                text=True,
                timeout=5,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            lines = log.read_text(encoding="utf-8").splitlines()
            self.assertFalse(any("import-environment" in line for line in lines))
            self.assertTrue(
                any(
                    line.startswith("ssh|") and "start rmac-session.target" in line
                    for line in lines
                )
            )

            # The login wrapper consumed the marker and chose safe mode.
            log.write_text("", encoding="utf-8")
            result = subprocess.run(
                [str(script), "--system-package", "--safe-mode"],
                env=environment,
                check=False,
                capture_output=True,
                text=True,
                timeout=5,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            lines = log.read_text(encoding="utf-8").splitlines()
            self.assertTrue(
                any(
                    line.startswith("niri|")
                    and "start rmac-safe-mode.target" in line
                    for line in lines
                )
            )
            self.assertFalse(any("try-restart" in line for line in lines))

            # A marker the wrapper already consumed is not re-read here.
            marker = root / "state/rmac/session/safe-mode.json"
            marker.parent.mkdir(parents=True)
            marker.write_text("{}\n", encoding="utf-8")
            log.write_text("", encoding="utf-8")
            result = subprocess.run(
                [str(script), "--system-package"],
                env=environment,
                check=False,
                capture_output=True,
                text=True,
                timeout=5,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            lines = log.read_text(encoding="utf-8").splitlines()
            self.assertTrue(any("start rmac-session.target" in line for line in lines))

            result = subprocess.run(
                [str(script), "--safe-mode", "--clear-safe-mode"],
                env=environment,
                check=False,
                capture_output=True,
                text=True,
                timeout=5,
            )
            self.assertEqual(result.returncode, 2)
            self.assertIn("--clear-safe-mode", result.stderr)

    def start_script_fixture(self, root: Path) -> tuple[Path, dict[str, str]]:
        write_program(
            root / "usr/bin/systemctl",
            'printf "%s|%s\\n" "$XDG_CURRENT_DESKTOP" "$*" >>"$RMAC_TEST_LOG"\n',
        )
        write_program(root / "usr/bin/dbus-update-activation-environment", "exit 0\n")
        mv = shutil.which("mv")
        assert mv is not None
        write_program(root / "usr/bin/mv", f'exec "{mv}" "$@"\n')
        environment = {
            **os.environ,
            "HOME": str(root / "home"),
            "PATH": "/usr/bin:/bin",
            "XDG_CONFIG_HOME": str(root / "config"),
            "XDG_STATE_HOME": str(root / "state"),
            "XDG_CURRENT_DESKTOP": "niri",
            "XDG_SESSION_DESKTOP": "niri",
            "XDG_SESSION_TYPE": "wayland",
            "WAYLAND_DISPLAY": "wayland-1",
            "RMAC_TEST_LOG": str(root / "systemctl.log"),
        }
        return rendered_start_script(root), environment

    def test_development_start_consumes_safe_mode_for_one_start(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            script, environment = self.start_script_fixture(root)
            log = root / "systemctl.log"
            marker = root / "state/rmac/session/safe-mode.json"
            marker.parent.mkdir(parents=True)
            marker.write_text("{}\n", encoding="utf-8")
            for expected in ("start rmac-safe-mode.target", "start rmac-session.target"):
                log.write_text("", encoding="utf-8")
                result = subprocess.run(
                    [str(script)],
                    env=environment,
                    check=False,
                    capture_output=True,
                    text=True,
                    timeout=5,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn(expected, log.read_text(encoding="utf-8"))
                self.assertFalse(marker.exists())
            self.assertTrue((marker.parent / "safe-mode.last.json").exists())

    def test_clear_safe_mode_restores_the_rmac_session_without_logging_out(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            script, environment = self.start_script_fixture(root)
            calls = root / "supervisor.log"
            write_program(
                root / "usr/libexec/rmac/rmac-session-supervisor",
                f'printf "%s\\n" "$*" >>"{calls}"\n',
            )
            log = root / "systemctl.log"
            result = subprocess.run(
                [str(script), "--clear-safe-mode"],
                env=environment,
                check=False,
                capture_output=True,
                text=True,
                timeout=5,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(calls.read_text(encoding="utf-8"), "leave-safe-mode\n")
            lines = log.read_text(encoding="utf-8").splitlines()
            imports = next(line for line in lines if "import-environment" in line)
            self.assertTrue(imports.startswith("rmac:niri|"))
            self.assertIn("XDG_SESSION_DESKTOP", imports)
            self.assertIn("rmac:niri|--user stop waybar.service", lines)
            self.assertTrue(
                any(line.endswith("--user start rmac-session.target") for line in lines)
            )
            self.assertTrue(any("try-restart xdg-desktop-portal" in line for line in lines))

            # When niri cannot switch configuration live the supervisor says
            # so, and the normal target is not started half-configured.
            write_program(
                root / "usr/libexec/rmac/rmac-session-supervisor",
                'echo "Safe mode is cleared, but niri could not switch. '
                'Log out and back in to start normally." >&2\nexit 1\n',
            )
            log.write_text("", encoding="utf-8")
            result = subprocess.run(
                [str(script), "--clear-safe-mode"],
                env=environment,
                check=False,
                capture_output=True,
                text=True,
                timeout=5,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Log out and back in", result.stderr)
            self.assertNotIn("start rmac-session.target", log.read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
