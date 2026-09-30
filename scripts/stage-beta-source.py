#!/usr/bin/env python3
"""Copy the current source worktree to a fresh reference-PC staging directory."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shlex
import shutil
import subprocess
import sys
import tempfile


DEFAULT_HOST = "jacob@192.168.18.52"
SECRET_NAMES = {
    "id_rsa", "id_ed25519", "id_ecdsa", "id_dsa", "credentials", "secrets",
    "secrets.json", "credentials.json", ".netrc", ".npmrc", ".pypirc",
}
SECRET_SUFFIXES = {".pem", ".key", ".p12", ".pfx", ".mobileprovision"}


def run_git(root: Path, *args: str, text: bool = True) -> str | bytes:
    result = subprocess.run(
        ["git", "-C", str(root), *args], check=True, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE, text=text,
    )
    return result.stdout


def is_excluded(relative: str) -> bool:
    path = PurePosixPath(relative)
    parts = path.parts
    lower_parts = [part.lower() for part in parts]
    if not parts or any(part in {".git", "target"} for part in lower_parts):
        return True
    name = path.name.lower()
    if name in SECRET_NAMES or name.endswith(tuple(SECRET_SUFFIXES)):
        return True
    if name.startswith(".env") or name.endswith((".secret", ".secrets")):
        return True
    if any(part in {".ssh", "credentials", "secrets"} for part in lower_parts[:-1]):
        return True
    return False


def tracked_and_untracked(root: Path) -> list[str]:
    raw = run_git(root, "ls-files", "--cached", "--others", "--exclude-standard", "-z", text=False)
    assert isinstance(raw, bytes)
    paths: list[str] = []
    for item in raw.split(b"\0"):
        if not item:
            continue
        relative = os.fsdecode(item)
        if is_excluded(relative):
            continue
        source = root / relative
        if not source.exists() and not source.is_symlink():
            continue
        if source.is_symlink():
            try:
                if Path(os.readlink(source)).is_absolute():
                    continue
                source.resolve(strict=True).relative_to(root.resolve())
            except (OSError, ValueError):
                continue
        elif not source.is_file():
            continue
        paths.append(relative)
    return sorted(set(paths), key=os.fsencode)


def build_snapshot(root: Path, destination: Path) -> dict[str, object]:
    source = destination / "source"
    source.mkdir(parents=True)
    paths = tracked_and_untracked(root)
    records: list[dict[str, object]] = []
    fingerprint = hashlib.sha256()
    for relative in paths:
        original = root / relative
        target = source / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        if original.is_symlink():
            target.symlink_to(os.readlink(original))
            payload = os.fsencode(os.readlink(original))
        else:
            shutil.copy2(original, target)
            payload = target.read_bytes()
        digest = hashlib.sha256(payload).hexdigest()
        fingerprint.update(os.fsencode(relative) + b"\0" + bytes.fromhex(digest))
        records.append({"path": relative, "sha256": digest, "size": len(payload)})

    head = str(run_git(root, "rev-parse", "HEAD")).strip()
    (destination / "HEAD").write_text(head + "\n", encoding="utf-8")
    # Keep the patch aligned with the source allowlist. A plain `git diff HEAD`
    # can carry the contents of excluded credentials or secret files.
    tracked_raw = run_git(root, "ls-files", "--cached", "-z", text=False)
    assert isinstance(tracked_raw, bytes)
    tracked = {os.fsdecode(item) for item in tracked_raw.split(b"\0") if item}
    diff_paths = sorted((path for path in tracked if not is_excluded(path)), key=os.fsencode)
    patch = (
        run_git(root, "diff", "--binary", "--no-ext-diff", "HEAD", "--", *diff_paths, text=False)
        if diff_paths else b""
    )
    assert isinstance(patch, bytes)
    (destination / "worktree.patch").write_bytes(patch)
    manifest = {
        "head": head,
        "fingerprint_sha256": fingerprint.hexdigest(),
        "files": records,
        "provenance": {
            "patch": "worktree.patch (tracked changes relative to HEAD)",
        },
    }
    (destination / "manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return manifest


def verify_snapshot(destination: Path) -> None:
    """Fail if any source byte or link changed during transfer."""
    manifest = json.loads((destination / "manifest.json").read_text(encoding="utf-8"))
    source = destination / "source"
    if not source.is_dir() or source.is_symlink():
        raise ValueError("staged source directory is invalid")
    fingerprint = hashlib.sha256()
    seen: set[str] = set()
    for record in manifest["files"]:
        relative = record["path"]
        path = PurePosixPath(relative)
        if path.is_absolute() or ".." in path.parts or relative in seen:
            raise ValueError(f"invalid manifest path: {relative}")
        seen.add(relative)
        item = source / relative
        if item.is_symlink():
            payload = os.fsencode(os.readlink(item))
        elif item.is_file():
            payload = item.read_bytes()
        else:
            raise ValueError(f"missing source file: {relative}")
        digest = hashlib.sha256(payload).hexdigest()
        if len(payload) != record["size"] or digest != record["sha256"]:
            raise ValueError(f"source mismatch: {relative}")
        fingerprint.update(os.fsencode(relative) + b"\0" + bytes.fromhex(digest))
    # Cargo and build scripts can read files that are not in the manifest.
    # The native checker creates only these two known artifact links *after*
    # its first verification, so a later verification can still be repeated.
    artifact_links = {
        "target": {str(Path.home() / "rmac/target")},
        "shell/target": {
            str(Path.home() / "rmac/shell/target"),
            str(Path.home() / "rmac-wt/shell/target"),
        },
    }
    for item in source.rglob("*"):
        if item.is_symlink() or not item.is_dir():
            relative = item.relative_to(source).as_posix()
            if relative not in seen:
                if item.is_symlink() and os.readlink(item) in artifact_links.get(relative, set()):
                    continue
                raise ValueError(f"unlisted source file: {relative}")
    if fingerprint.hexdigest() != manifest["fingerprint_sha256"]:
        raise ValueError("source fingerprint mismatch")


def validate_host(host: str) -> None:
    if not re.fullmatch(r"(?:[A-Za-z0-9_.-]+@)?[A-Za-z0-9_.-]+", host) or host.startswith("-"):
        raise ValueError("host must be an SSH hostname or user@hostname without shell characters")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("host", nargs="?", default=DEFAULT_HOST, help=f"SSH destination (default: {DEFAULT_HOST})")
    parser.add_argument("--verify", type=Path, help="verify a staged payload and exit")
    args = parser.parse_args()
    try:
        if args.verify is not None:
            verify_snapshot(args.verify)
            print(f"Verified staged source at {args.verify}")
            return 0
        validate_host(args.host)
        root = Path(str(run_git(Path.cwd(), "rev-parse", "--show-toplevel")).strip()).resolve()
        head = str(run_git(root, "rev-parse", "--short", "HEAD")).strip()
        stamp = dt.datetime.now().astimezone().strftime("%Y%m%d-%H%M%S")
        stage_id = f"rmac-source-{stamp}-{head}-{os.getpid()}"
        remote_dir = f"$HOME/rmac-source-staging/{stage_id}"
        # Keep this snapshot in the repository's normal ignored target tree;
        # AGENTS.md forbids copying the repository into /private/tmp.
        staging_root = root / "target"
        staging_root.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(
            prefix="rmac-source-stage-", dir=staging_root
        ) as temporary:
            local = Path(temporary) / "payload"
            local.mkdir()
            manifest = build_snapshot(root, local)
            print(
                f"Prepared {len(manifest['files'])} files; "
                f"source SHA-256 {manifest['fingerprint_sha256']}",
                flush=True,
            )
            print(f"HEAD {manifest['head']}; staging on {args.host}:{remote_dir}", flush=True)
            # Keep one authenticated TCP connection across mkdir, rsync,
            # verification and native checks. The reference PC's SSH port can
            # intermittently reject new connections even while it is online.
            control_path = Path(temporary) / "ssh.sock"
            ssh_options = [
                "-o", "BatchMode=yes",
                "-o", "ConnectTimeout=8",
                "-o", "ControlMaster=auto",
                "-o", "ControlPersist=60",
                "-o", f"ControlPath={control_path}",
            ]
            ssh = ["ssh", *ssh_options, args.host]
            remote_mkdir = (
                f'umask 077; mkdir -p "$HOME/rmac-source-staging" && '
                f'mkdir -p -m 700 "$HOME/rmac-source-staging/{stage_id}"'
            )
            try:
                for attempt in range(3):
                    result = subprocess.run([*ssh, remote_mkdir], check=False)
                    if result.returncode == 0:
                        break
                    if result.returncode != 255 or attempt == 2:
                        raise subprocess.CalledProcessError(result.returncode, result.args)
                rsync_ssh = shlex.join(["ssh", *ssh_options])
                destination = f"{args.host}:~/rmac-source-staging/{stage_id}/"
                subprocess.run(
                    ["rsync", "-a", "-e", rsync_ssh, str(local) + "/", destination],
                    check=True,
                )
                verify_command = (
                    f'python3 "$HOME/rmac-source-staging/{stage_id}/source/scripts/stage-beta-source.py" '
                    f'--verify "$HOME/rmac-source-staging/{stage_id}"'
                )
                subprocess.run([*ssh, verify_command], check=True)
                print(f"Staged at {args.host}:~/rmac-source-staging/{stage_id}/", flush=True)
                native_command = (
                    f'bash "$HOME/rmac-source-staging/{stage_id}/source/scripts/linux/'
                    'check-staged-beta-source.sh"'
                )
                subprocess.run([*ssh, native_command], check=True)
            finally:
                subprocess.run(
                    ["ssh", *ssh_options, "-O", "exit", args.host],
                    check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                )
        print("Scoped native checks passed; review native-check-results.txt in the staging directory.")
        return 0
    except (OSError, subprocess.CalledProcessError, ValueError) as error:
        print(f"stage-beta-source: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
