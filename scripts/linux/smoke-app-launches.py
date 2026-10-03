#!/usr/bin/env python3
"""Run a startup-only smoke for packaged and installed first-party apps.

This runner starts each app once in a private D-Bus session, fresh HOME/XDG
directories, and a headless Sway compositor. It records only startup readiness,
not interaction or visual quality. Archive Utility is a no-window helper, so
its readiness check verifies that it safely expands a generated ZIP fixture.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import wave
import zipfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Optional


@dataclass(frozen=True)
class AppSpec:
    app_id: str
    binary: str
    fixture: Optional[str] = None
    mode: str = "window"
    window_app_id: Optional[str] = None


# Several installed apps also have behavior scenarios; startup readiness stays
# useful as a cheaper check that their packaged entry points create windows.
# App Drawer is started in its explicit supervised show mode, which opens its
# overlay without dispatching into the live shell's shortcut endpoint.
APP_SPECS = (
    AppSpec("archive-utility", "rmac-archive-utility", "zip", "archive", "org.rmac.ArchiveUtility"),
    AppSpec("app-drawer", "rmac-app-drawer", mode="layer"),
    AppSpec("clock", "rmac-clock", window_app_id="org.rmac.Clock"),
    AppSpec("calendar", "rmac-calendar", window_app_id="org.rmac.Calendar"),
    AppSpec("mail", "rmac-mail", window_app_id="org.rmac.Mail"),
    AppSpec("notes", "rmac-notes", window_app_id="org.rmac.Notes"),
    AppSpec("player", "rmac-player", "wav", window_app_id="org.rmac.Player"),
    AppSpec("preview", "rmac-preview", "pdf", window_app_id="org.rmac.Preview"),
    AppSpec("system-monitor", "rmac-system-monitor", window_app_id="org.rmac.SystemMonitor"),
    AppSpec("terminal", "rmac-terminal", window_app_id="org.rmac.Terminal"),
    AppSpec("weather", "rmac-weather", window_app_id="org.rmac.Weather"),
    AppSpec("calculator", "rmac-calculator", window_app_id="org.rmac.Calculator"),
    AppSpec("system-settings", "rmac-system-settings", window_app_id="org.rmac.SystemSettings"),
    AppSpec("text-editor", "rmac-text-editor", window_app_id="org.rmac.TextEditor"),
    AppSpec("files", "rmac-files", "files", window_app_id="org.rmac.Files"),
)

DEFAULT_TIMEOUT = 20.0
OUTPUT_SIZE = (1280, 800)
READINESS_STABILITY_SECONDS = 0.5


def fixture_arguments(spec: AppSpec, fixture_dir: Path) -> list[str]:
    if spec.fixture == "zip":
        return [str(fixture_dir / "smoke-archive.zip")]
    if spec.fixture == "wav":
        return [str(fixture_dir / "smoke-audio.wav")]
    if spec.fixture == "pdf":
        return [str(fixture_dir / "smoke-document.pdf")]
    if spec.fixture == "files":
        return ["--path", str(fixture_dir)]
    return ["--service", "--show"] if spec.app_id == "app-drawer" else []


def make_pdf() -> bytes:
    """Create a small valid one-page PDF without external tools."""
    stream = b"BT /F1 12 Tf 20 50 Td (Lulo smoke fixture) Tj ET\n"
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
        f"<< /Length {len(stream)} >>\nstream\n".encode() + stream + b"endstream",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    ]
    output = bytearray(b"%PDF-1.4\n")
    offsets = [0]
    for index, obj in enumerate(objects, start=1):
        offsets.append(len(output))
        output.extend(f"{index} 0 obj\n".encode())
        output.extend(obj)
        output.extend(b"\nendobj\n")
    xref = len(output)
    output.extend(f"xref\n0 {len(offsets)}\n0000000000 65535 f \n".encode())
    for offset in offsets[1:]:
        output.extend(f"{offset:010d} 00000 n \n".encode())
    output.extend(
        f"trailer\n<< /Size {len(offsets)} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode()
    )
    return bytes(output)


def create_fixtures(directory: Path) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(directory / "smoke-archive.zip", "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("smoke-extracted.txt", "Lulo disposable launch fixture\n")
    (directory / "smoke-document.pdf").write_bytes(make_pdf())
    with wave.open(str(directory / "smoke-audio.wav"), "wb") as audio:
        audio.setnchannels(1)
        audio.setsampwidth(2)
        audio.setframerate(8000)
        audio.writeframes(b"\x00\x00" * 800)


def isolated_environment(work: Path, inherited: Optional[dict[str, str]] = None) -> dict[str, str]:
    """Build an environment that cannot address the user's live Wayland session."""
    inherited = os.environ if inherited is None else inherited
    env = {key: inherited[key] for key in ("PATH", "LANG", "USER", "LOGNAME", "SHELL") if key in inherited}
    home = work / "home"
    runtime = work / "runtime"
    for directory in (
        home,
        runtime,
        work / "tmp",
        home / ".config",
        home / ".local/share",
        home / ".local/state",
        home / ".cache",
    ):
        directory.mkdir(parents=True, exist_ok=True)
    runtime.chmod(0o700)
    env.update(
        {
            "HOME": str(home),
            "XDG_RUNTIME_DIR": str(runtime),
            "XDG_CONFIG_HOME": str(home / ".config"),
            "XDG_DATA_HOME": str(home / ".local/share"),
            "XDG_STATE_HOME": str(home / ".local/state"),
            "XDG_CACHE_HOME": str(home / ".cache"),
            "XDG_SESSION_TYPE": "wayland",
            "XDG_CURRENT_DESKTOP": "rmac:niri",
            "TMPDIR": str(work / "tmp"),
            "GSETTINGS_BACKEND": "memory",
            "WLR_BACKENDS": "headless",
            "WLR_HEADLESS_OUTPUTS": "1",
            "WLR_LIBINPUT_NO_DEVICES": "1",
            "WLR_RENDERER": "pixman",
            "LIBGL_ALWAYS_SOFTWARE": "1",
            "PULSE_SERVER": "unix:/nonexistent/lulo-smoke-pulse",
        }
    )
    return env


def refuse_live_environment(env: dict[str, str]) -> None:
    runtime = os.path.realpath(env.get("XDG_RUNTIME_DIR", ""))
    display = env.get("WAYLAND_DISPLAY", "")
    if not runtime or runtime.startswith("/run/user/") or display == "wayland-1":
        raise RuntimeError("refusing to run in a live Wayland session")


def _source_revision(root: Path) -> str:
    try:
        result = subprocess.run(
            ["git", "-C", str(root), "rev-parse", "--short", "HEAD"],
            capture_output=True,
            text=True,
            timeout=3,
            check=True,
        )
        return result.stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return "unknown"


def _source_dirty(root: Path) -> bool:
    try:
        result = subprocess.run(
            ["git", "-C", str(root), "status", "--porcelain"],
            capture_output=True,
            text=True,
            timeout=3,
            check=True,
        )
        return bool(result.stdout.strip())
    except (OSError, subprocess.SubprocessError):
        return False


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as binary:
        for chunk in iter(lambda: binary.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


class NestedSway:
    def __init__(self, work: Path, env: dict[str, str]) -> None:
        self.work = work
        self.env = env
        self.log = (work / "sway.log").open("w")
        config = work / "sway.conf"
        config.write_text(
            "xwayland disable\n"
            "default_border none\n"
            f"output HEADLESS-1 mode {OUTPUT_SIZE[0]}x{OUTPUT_SIZE[1]}\n"
            "seat seat0 fallback true\n"
        )
        self.locks: list[Any] = []
        try:
            import fcntl

            runtime = Path(env["XDG_RUNTIME_DIR"])
            for name in ("wayland-0.lock", "wayland-1.lock"):
                lock = (runtime / name).open("w")
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                self.locks.append(lock)
        except (OSError, ImportError):
            self._close_locks()
            raise RuntimeError("could not reserve nested Wayland socket names")
        self.process = subprocess.Popen(
            ["sway", "--unsupported-gpu", "--config", str(config)],
            env=env,
            stdout=self.log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
            close_fds=True,
        )
        try:
            self._wait_for_sockets()
        except BaseException:
            self.close()
            raise

    def _close_locks(self) -> None:
        for lock in self.locks:
            lock.close()
        self.locks.clear()

    def _wait_for_sockets(self) -> None:
        runtime = Path(self.env["XDG_RUNTIME_DIR"])
        deadline = time.monotonic() + 20
        wayland: Optional[Path] = None
        ipc: Optional[Path] = None
        while time.monotonic() < deadline:
            wayland = next((p for p in runtime.glob("wayland-*") if not p.name.endswith(".lock")), None)
            ipc = next(iter(runtime.glob("sway-ipc.*.sock")), None)
            if wayland and ipc:
                break
            if self.process.poll() is not None:
                raise RuntimeError("nested Sway exited before creating its sockets")
            time.sleep(0.1)
        if not (wayland and ipc):
            raise RuntimeError("nested Sway socket startup timed out")
        self.env["WAYLAND_DISPLAY"] = wayland.name
        self.env["SWAYSOCK"] = str(ipc)
        os.environ.update({"WAYLAND_DISPLAY": wayland.name, "SWAYSOCK": str(ipc)})

    def tree(self) -> Any:
        result = subprocess.run(
            ["swaymsg", "-r", "-t", "get_tree"],
            env=self.env,
            capture_output=True,
            text=True,
            timeout=5,
            check=True,
        )
        return json.loads(result.stdout)

    def pids_with_windows(self) -> set[int]:
        pids: set[int] = set()

        def walk(node: dict[str, Any]) -> None:
            pid = node.get("pid")
            if isinstance(pid, int) and node.get("type") in {"con", "floating_con"}:
                pids.add(pid)
            for child in node.get("nodes", []) + node.get("floating_nodes", []):
                if isinstance(child, dict):
                    walk(child)

        tree = self.tree()
        if isinstance(tree, dict):
            walk(tree)
        return pids

    def has_window(self, pid: int, expected_app_id: Optional[str] = None) -> bool:
        """Check that this process owns a mapped window with the requested app ID."""

        def walk(node: dict[str, Any]) -> bool:
            if (
                node.get("pid") == pid
                and node.get("type") in {"con", "floating_con"}
                and (expected_app_id is None or node.get("app_id") == expected_app_id)
            ):
                return True
            return any(
                walk(child)
                for child in node.get("nodes", []) + node.get("floating_nodes", [])
                if isinstance(child, dict)
            )

        tree = self.tree()
        return isinstance(tree, dict) and walk(tree)

    def close(self) -> None:
        if getattr(self, "process", None) is not None and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(5)
        self._close_locks()
        if not self.log.closed:
            self.log.close()


def _atspi_window(pid: int) -> tuple[bool, int]:
    import pyatspi

    desktop = pyatspi.Registry.getDesktop(0)
    for index in range(desktop.childCount):
        try:
            app = desktop.getChildAtIndex(index)
            if app is None or app.get_process_id() != pid:
                continue
            count = app.childCount
            for child_index in range(count):
                frame = app.getChildAtIndex(child_index)
                if frame is not None and frame.childCount > 0:
                    return True, count
            return False, count
        except Exception:
            continue
    return False, 0


def _terminate_group(process: subprocess.Popen[Any]) -> None:
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    if process.poll() is None:
        try:
            process.wait(5)
        except subprocess.TimeoutExpired:
            pass
    # The leader may have exited while a child still holds its process group.
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    if process.poll() is None:
        process.wait(5)


def readiness_is_stable(
    process: subprocess.Popen[Any],
    readiness_check: Any,
    duration: float = READINESS_STABILITY_SECONDS,
) -> bool:
    """Require the process and its observable readiness to persist briefly."""
    deadline = time.monotonic() + duration
    while True:
        if process.poll() is not None or not readiness_check():
            return False
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            return True
        time.sleep(min(0.1, remaining))


def reap_private_processes(runtime: Path) -> list[int]:
    """Stop D-Bus activated helpers that retained this run's private runtime."""
    needle = f"XDG_RUNTIME_DIR={runtime}".encode()
    stopped = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit() or int(entry.name) == os.getpid():
            continue
        try:
            environ = (entry / "environ").read_bytes().split(b"\0")
        except OSError:
            continue
        if needle in environ:
            try:
                os.kill(int(entry.name), signal.SIGTERM)
                stopped.append(int(entry.name))
            except OSError:
                pass
    if stopped:
        time.sleep(0.25)
    return stopped


def run_app(
    spec: AppSpec,
    binary: Path,
    fixture_dir: Path,
    work: Path,
    base_env: dict[str, str],
    sway: NestedSway,
    timeout: float,
) -> dict[str, Any]:
    record: dict[str, Any] = {
        "app": spec.app_id,
        "binary": spec.binary,
        "binary_sha256": _sha256(binary) if binary.is_file() else None,
    }
    if not binary.is_file() or not os.access(binary, os.X_OK):
        record.update(outcome="failed", reason_code="binary_missing_or_not_executable")
        return record

    app_root = work / "apps" / spec.app_id
    home = app_root / "home"
    app_env = dict(base_env)
    for key, path in (
        ("HOME", home),
        ("XDG_CONFIG_HOME", home / ".config"),
        ("XDG_DATA_HOME", home / ".local/share"),
        ("XDG_STATE_HOME", home / ".local/state"),
        ("XDG_CACHE_HOME", home / ".cache"),
    ):
        path.mkdir(parents=True, exist_ok=True)
        app_env[key] = str(path)
    log_path = work / f"{spec.app_id}.log"
    started = time.monotonic()
    process: Optional[subprocess.Popen[Any]] = None
    try:
        with log_path.open("w") as log:
            process = subprocess.Popen(
                [str(binary), *fixture_arguments(spec, fixture_dir)],
                cwd=home,
                env=app_env,
                stdout=log,
                stderr=subprocess.STDOUT,
                start_new_session=True,
                close_fds=True,
            )
            deadline = started + timeout
            accessible = False
            window_count = 0
            while time.monotonic() < deadline:
                return_code = process.poll()
                if return_code is not None:
                    if spec.mode == "archive" and return_code == 0:
                        extracted = fixture_dir / "smoke-extracted.txt"
                        if extracted.is_file() and extracted.read_text() == "Lulo disposable launch fixture\n":
                            record.update(outcome="passed", readiness="fixture_extracted")
                            break
                        record.update(outcome="failed", reason_code="fixture_extraction_not_verified")
                        break
                    record.update(outcome="failed", reason_code="process_exited_before_ready")
                    break
                mapped = sway.has_window(process.pid, spec.window_app_id)
                if mapped or spec.mode == "layer":
                    accessible, window_count = _atspi_window(process.pid)
                    if accessible:
                        def ready_now() -> bool:
                            if spec.mode == "layer":
                                return _atspi_window(process.pid)[0]
                            return sway.has_window(process.pid, spec.window_app_id) and _atspi_window(process.pid)[0]

                        if readiness_is_stable(process, ready_now):
                            readiness = "accessible_layer_surface" if spec.mode == "layer" else "mapped_and_accessible"
                            record.update(outcome="passed", readiness=readiness)
                        else:
                            record.update(outcome="failed", reason_code="readiness_not_stable")
                        break
                time.sleep(0.15)
            else:
                record.update(
                    outcome="failed",
                    reason_code="startup_timeout",
                    mapped=sway.has_window(process.pid, spec.window_app_id),
                    accessible_frames=window_count,
                )
    except (OSError, subprocess.SubprocessError):
        record.update(outcome="failed", reason_code="launch_error")
    finally:
        if process is not None:
            _terminate_group(process)
    record["elapsed_ms"] = round((time.monotonic() - started) * 1000)
    return record


def selected_apps(requested: Optional[list[str]]) -> tuple[AppSpec, ...]:
    if requested is None:
        return APP_SPECS
    selected = set(requested)
    return tuple(spec for spec in APP_SPECS if spec.app_id in selected)


def bounded_diagnostic(stderr: str, work: Path, limit: int = 800) -> Optional[str]:
    """Keep a short startup clue while removing private and home-directory paths."""
    text = stderr.strip()
    if not text:
        return None
    text = text.replace(str(work), "<private-temp>")
    text = re.sub(r"/home/[^/\s]+[^\s:'\"]*", "<home-path>", text)
    text = re.sub(r"/tmp/[^\s:'\"]+", "<temporary-path>", text)
    excerpt = "\n".join(text.splitlines()[-8:])
    return excerpt[:limit]


def _inner(args: argparse.Namespace, work: Path) -> dict[str, Any]:
    env = isolated_environment(work)
    # Keep only the private bus address supplied by dbus-run-session. The outer
    # environment is filtered, so a live desktop bus is never propagated.
    for name in ("DBUS_SESSION_BUS_ADDRESS", "DBUS_SESSION_BUS_PID"):
        if name in os.environ:
            env[name] = os.environ[name]
    refuse_live_environment(env)
    sway = NestedSway(work, env)
    try:
        subprocess.run(
            ["busctl", "--user", "set-property", "org.a11y.Bus", "/org/a11y/bus", "org.a11y.Status", "IsEnabled", "b", "true"],
            env=env,
            capture_output=True,
            timeout=10,
            check=True,
        )
        import pyatspi  # noqa: F401

        fixture_dir = work / "fixtures"
        create_fixtures(fixture_dir)
        results = [
            run_app(
                spec,
                args.bin_dir / spec.binary,
                fixture_dir,
                work,
                env,
                sway,
                args.timeout,
            )
            for spec in selected_apps(args.apps)
        ]
        return {"results": results}
    finally:
        sway.close()


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", required=True, type=Path, help="directory containing app binaries")
    parser.add_argument("--output", type=Path, help="write a privacy-bounded JSON report")
    parser.add_argument("--source-revision", help="source commit label; defaults to the current Git HEAD")
    parser.add_argument("--binary-revision", default="unknown", help="build commit label for supplied binaries")
    parser.add_argument("--timeout", type=float, default=DEFAULT_TIMEOUT, help="per-app startup timeout in seconds")
    parser.add_argument("--keep", action="store_true", help="retain private logs under /tmp for diagnosis")
    parser.add_argument(
        "--apps",
        nargs="+",
        choices=[spec.app_id for spec in APP_SPECS],
        help="run only selected app IDs (default: all fourteen)",
    )
    parser.add_argument("--_inner-work", type=Path, help=argparse.SUPPRESS)
    return parser


def main(argv: Optional[list[str]] = None) -> int:
    args = build_parser().parse_args(argv)
    if args.timeout <= 0 or args.timeout > 120:
        raise SystemExit("--timeout must be greater than zero and at most 120 seconds")
    args.bin_dir = args.bin_dir.resolve()
    if args._inner_work is not None:
        report = _inner(args, args._inner_work)
        print(json.dumps(report, sort_keys=True))
        return 0 if all(row["outcome"] != "failed" for row in report["results"]) else 1

    if sys.platform != "linux":
        raise SystemExit("this smoke requires Linux")
    for tool in ("sway", "swaymsg", "dbus-run-session", "busctl"):
        if shutil.which(tool) is None:
            raise SystemExit(f"{tool} is required")
    root = Path(__file__).resolve().parents[2]
    work = Path(tempfile.mkdtemp(prefix="lulo-app-smoke-"))
    env = isolated_environment(work)
    refuse_live_environment(env)
    services = work / "dbus-services"
    services.mkdir()
    atspi_service = Path("/usr/share/dbus-1/services/org.a11y.Bus.service")
    if not atspi_service.is_file():
        shutil.rmtree(work, ignore_errors=True)
        raise SystemExit("AT-SPI D-Bus service is required")
    shutil.copyfile(atspi_service, services / atspi_service.name)
    (work / "session.conf").write_text(
        '<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"\n'
        ' "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">\n'
        '<busconfig><type>session</type>'
        f'<listen>unix:dir={work}</listen><auth>EXTERNAL</auth><servicedir>{services}</servicedir>'
        '<policy context="default"><allow send_destination="*" eavesdrop="true"/>'
        '<allow eavesdrop="true"/><allow own="*"/></policy></busconfig>\n'
    )
    args._inner_work = work
    args.source_revision = args.source_revision or _source_revision(root)
    source_dirty = _source_dirty(root)
    command = [
        "dbus-run-session",
        f"--config-file={work / 'session.conf'}",
        "--",
        sys.executable,
        str(Path(__file__).resolve()),
        "--bin-dir",
        str(args.bin_dir),
        "--source-revision",
        args.source_revision,
        "--binary-revision",
        args.binary_revision,
        "--timeout",
        str(args.timeout),
    ]
    if args.apps:
        command.extend(["--apps", *args.apps])
    command.extend(["--_inner-work", str(work)])
    try:
        run = subprocess.run(command, env=env, capture_output=True, text=True, timeout=1200)
        diagnostic = bounded_diagnostic(run.stderr, work)
        if run.stderr:
            (work / "session.stderr").write_text(run.stderr)
        if run.stdout.strip():
            inner = json.loads(run.stdout.splitlines()[-1])
            report = {
                "schema_version": 1,
                "scope": "startup_readiness_only",
                "source_revision": args.source_revision,
                "source_dirty": source_dirty,
                "binary_revision": args.binary_revision,
                "binary_dir": args.bin_dir.name,
                "results": inner["results"],
                "summary": {
                    "passed": sum(r["outcome"] == "passed" for r in inner["results"]),
                    "failed": sum(r["outcome"] == "failed" for r in inner["results"]),
                    "skipped": sum(r["outcome"] == "skipped" for r in inner["results"]),
                    "total": len(inner["results"]),
                },
            }
        else:
            report = {
                "schema_version": 1,
                "scope": "startup_readiness_only",
                "source_revision": args.source_revision,
                "source_dirty": source_dirty,
                "binary_revision": args.binary_revision,
                "binary_dir": args.bin_dir.name,
                "results": [],
                "summary": {
                    "passed": 0,
                    "failed": len(selected_apps(args.apps)),
                    "skipped": 0,
                    "total": len(selected_apps(args.apps)),
                },
                "error": "nested_session_start_failed",
                "session_exit_code": run.returncode,
            }
            if diagnostic:
                report["diagnostic_excerpt"] = diagnostic
    finally:
        reap_private_processes(work / "runtime")
        if not args.keep:
            shutil.rmtree(work, ignore_errors=True)
        else:
            print(f"kept private diagnostic directory: {work}", file=sys.stderr)
    rendered = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered)
    print(rendered, end="")
    return 1 if report["summary"]["failed"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
