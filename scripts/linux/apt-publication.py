#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Rebuild the rmac APT repository's published state from GitHub Releases.

GitHub Pages keeps no history: `actions/deploy-pages` replaces the whole site
with one artifact, and a crawler cannot mirror a site without directory
listings. So the publisher's state never lives on Pages. Instead every signed
publication is attached to the GitHub Release whose packages it serves, as
``apt-snapshot-<snapshot>.tar`` (the exact signed metadata snapshot that
`publish-apt-snapshot.py` retains, plus an unsigned ``rmac-snapshot.json``
sidecar naming the Release each pool object came from and when the current
rollout phase began). Each run of the publication job rebuilds the
repository from those authoritative inputs:

``collect``
    lists the Releases, takes the newest ``--retain`` snapshot bundles,
    verifies each one's InRelease with ``gpgv`` against the archive keyring
    from the target Release's own keyring package (whose primary fingerprints
    must equal ``packaging/apt/archive-key.json``), checks every metadata file
    against the signed manifest, and re-fetches every pool object those
    snapshots name from the ``apt-inputs-<tag>.tar`` of the Release that
    produced it -- each such Release asset verified against that Release's
    ``SHA256SUMS`` and its build-provenance attestation (signed by this
    repository's ``release.yml``). The result is the previously published
    repository, byte for byte, so `publish-apt-snapshot.py`'s monotonic
    Date/snapshot, immutable-pool, and retention checks run against real
    state. No snapshot bundle at all fails closed unless the very first
    publication is explicitly allowed.
``decide``
    chooses the phase for a release (10, or an explicit override) or a
    rollout tick (`next-rollout-phase.py` rules, plus a signature refresh
    before ``Valid-Until`` gets close).
``bundle``
    packs the snapshot `publish-apt-snapshot.py` just retained, with its
    sidecar, for upload to the Release.

It never signs and never deploys; `publish-apt-repository.sh` drives it.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from datetime import datetime, timezone
from typing import Callable, Dict, List, Optional, Sequence, Tuple

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import apt_archive  # noqa: E402
from apt_archive import ArchiveError  # noqa: E402
import keyring_package_contract  # noqa: E402


REPO_ROOT = HERE.parents[1]
ARCHIVE_KEY_PATH = REPO_ROOT / "packaging/apt/archive-key.json"
BUNDLE_RE = re.compile(r"apt-snapshot-([0-9]{8}T[0-9]{6}Z)\.tar")
TAG_RE = re.compile(r"v[0-9][0-9A-Za-z.+-]{0,63}")
FINGERPRINT_RE = re.compile(r"(?:[0-9A-F]{40}|[0-9A-F]{64})")
SIDECAR_NAME = "rmac-snapshot.json"
SNAPSHOT_PREFIX = "snapshot"
MAX_INPUTS_BYTES = 12 * 1024 * 1024 * 1024
MAX_BUNDLE_BYTES = 256 * 1024 * 1024
# Re-sign when less than this remains before Valid-Until. With --valid-hours
# 48 and a 6-hourly schedule this re-signs roughly every 18 to 24 hours and
# leaves a day of margin for a missed tick.
REFRESH_MARGIN_SECONDS = 30 * 3600
RELEASE_PHASE = 10


class PublicationError(RuntimeError):
    """A bounded, privacy-safe publication-state failure."""


def _load_script(name: str, filename: str):
    specification = importlib.util.spec_from_file_location(name, HERE / filename)
    if specification is None or specification.loader is None:
        raise PublicationError(f"{filename} cannot be loaded")
    module = importlib.util.module_from_spec(specification)
    sys.modules[specification.name] = module
    specification.loader.exec_module(module)
    return module


publisher = _load_script("rmac_publish_apt_snapshot", "publish-apt-snapshot.py")
rollout = _load_script("rmac_next_rollout_phase", "next-rollout-phase.py")


def asset_name(filename: str) -> str:
    """GitHub stores "~" in an uploaded asset's name as "."."""
    return filename.replace("~", ".")


def inputs_asset(tag: str) -> str:
    return f"apt-inputs-{tag}.tar"


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while True:
            chunk = source.read(1024 * 1024)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def pinned_fingerprints(path: Path = ARCHIVE_KEY_PATH) -> Tuple[str, ...]:
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise PublicationError("packaging/apt/archive-key.json is unreadable") from error
    values = document.get("primary_fingerprints") if isinstance(document, dict) else None
    if (
        not isinstance(values, list)
        or not 1 <= len(values) <= 2
        or values != sorted(set(values))
        or any(not isinstance(value, str) or not FINGERPRINT_RE.fullmatch(value) for value in values)
    ):
        raise PublicationError(
            "packaging/apt/archive-key.json names no archive key yet; run scripts/release/create-archive-key.sh"
        )
    return tuple(values)


# --- GitHub ------------------------------------------------------------------------


class GitHub:
    """The few `gh` calls the job needs, all with structured (JSON) output."""

    def __init__(self, repository: str, gh: str = "gh"):
        if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
            raise PublicationError("repository must be OWNER/NAME")
        self.repository = repository
        self.gh = gh

    def _run(self, arguments: List[str], label: str, timeout: int = 1800) -> bytes:
        try:
            result = subprocess.run(
                [self.gh, *arguments],
                check=False,
                capture_output=True,
                timeout=timeout,
            )
        except (OSError, subprocess.TimeoutExpired) as error:
            raise PublicationError(f"gh could not {label}") from error
        if result.returncode != 0:
            detail = result.stderr.decode("utf-8", "replace").strip().splitlines()[-1:] or [""]
            raise PublicationError(f"gh could not {label}: {detail[0][:200]}")
        return result.stdout

    def releases(self) -> List[Dict[str, object]]:
        output = self._run(
            [
                "api",
                "--paginate",
                f"repos/{self.repository}/releases?per_page=100",
                "--jq",
                ".[] | {tag: .tag_name, draft: .draft, assets: [.assets[].name]}",
            ],
            "list the releases",
        )
        releases = []
        for line in output.decode("utf-8").splitlines():
            if line.strip():
                releases.append(json.loads(line))
        return releases

    def download(self, tag: str, name: str, directory: Path) -> Path:
        directory.mkdir(parents=True, exist_ok=True)
        destination = directory / name
        if destination.exists():
            return destination
        self._run(
            [
                "release",
                "download",
                tag,
                "--repo",
                self.repository,
                "--pattern",
                name,
                "--dir",
                str(directory),
            ],
            f"download {name} from {tag}",
        )
        if not destination.is_file():
            raise PublicationError(f"{tag} has no asset named {name}")
        return destination

    def verify_attestation(self, path: Path) -> None:
        self._run(
            [
                "attestation",
                "verify",
                str(path),
                "--repo",
                self.repository,
                "--signer-workflow",
                f"{self.repository}/.github/workflows/release.yml",
            ],
            f"verify the build provenance of {path.name}",
            timeout=300,
        )

    def upload(self, tag: str, path: Path) -> None:
        self._run(
            ["release", "upload", tag, str(path), "--repo", self.repository],
            f"upload {path.name} to {tag}",
        )


# --- keyring -------------------------------------------------------------------------


def gpg_primary_fingerprints(keyring: Path) -> Tuple[str, ...]:
    gpg = shutil.which("gpg")
    if gpg is None:
        raise PublicationError("gpg is required to inspect the archive keyring")
    with tempfile.TemporaryDirectory() as home:
        os.chmod(home, 0o700)
        try:
            result = subprocess.run(
                [
                    gpg,
                    "--batch",
                    "--no-options",
                    "--homedir",
                    home,
                    "--with-colons",
                    "--import-options",
                    "show-only",
                    "--dry-run",
                    "--import",
                    str(keyring),
                ],
                check=False,
                capture_output=True,
                timeout=60,
                env={"LC_ALL": "C", "PATH": os.environ.get("PATH", "")},
            )
        except (OSError, subprocess.TimeoutExpired) as error:
            raise PublicationError("gpg could not inspect the archive keyring") from error
    if result.returncode != 0:
        raise PublicationError("gpg could not inspect the archive keyring")
    primaries: List[str] = []
    pending = False
    for line in result.stdout.decode("utf-8", "replace").splitlines():
        fields = line.split(":")
        if fields[0] in {"sec", "ssb"}:
            raise PublicationError("the packaged archive keyring contains secret material")
        if fields[0] == "pub":
            pending = True
        elif fields[0] == "fpr" and pending and len(fields) > 9:
            primaries.append(fields[9])
            pending = False
    return tuple(sorted(primaries))


# --- collect -------------------------------------------------------------------------


class ReleaseInputs:
    """One Release's verified apt-inputs tar, extracted and indexed by digest."""

    def __init__(self, tag: str, directory: Path):
        self.tag = tag
        self.directory = directory
        self.by_digest: Dict[Tuple[int, str], Path] = {}
        for path in sorted(directory.rglob("*")):
            if path.is_file() and not path.is_symlink():
                self.by_digest.setdefault((path.stat().st_size, _sha256(path)), path)


class Collector:
    def __init__(
        self,
        github: GitHub,
        work: Path,
        *,
        keyring_inspector: Callable[[Path], Tuple[str, ...]] = gpg_primary_fingerprints,
        verify_signature: Optional[Callable[[Path, Path], Tuple[bytes, Tuple[str, ...]]]] = None,
    ):
        self.github = github
        self.work = work
        self.keyring_inspector = keyring_inspector
        self.contract = publisher.load_contract()
        self.verify_signature = verify_signature or (
            lambda inrelease, keyring: publisher._verify_inrelease(
                inrelease, keyring, int(self.contract["maximum_keyring_bytes"])
            )
        )
        self._inputs: Dict[str, ReleaseInputs] = {}
        self._releases: Optional[Dict[str, List[str]]] = None

    def releases(self) -> Dict[str, List[str]]:
        if self._releases is None:
            releases: Dict[str, List[str]] = {}
            for release in self.github.releases():
                tag = release.get("tag")
                if release.get("draft") or not isinstance(tag, str) or not TAG_RE.fullmatch(tag):
                    continue
                releases[tag] = [str(name) for name in release.get("assets", [])]
            self._releases = releases
        return self._releases

    def release_inputs(self, tag: str) -> ReleaseInputs:
        if tag in self._inputs:
            return self._inputs[tag]
        assets = self.releases().get(tag)
        if assets is None:
            raise PublicationError(f"release {tag} does not exist")
        name = inputs_asset(tag)
        if name not in assets or "SHA256SUMS" not in assets:
            raise PublicationError(f"release {tag} has no {name} and SHA256SUMS")
        download = self.work / "downloads" / tag
        archive = self.github.download(tag, name, download)
        sums = self.github.download(tag, "SHA256SUMS", download)
        listed = [
            line.split()
            for line in sums.read_text(encoding="utf-8").splitlines()
            if line.strip()
        ]
        matches = [parts[0] for parts in listed if len(parts) == 2 and parts[1].lstrip("*") == name]
        if len(matches) != 1 or matches[0] != _sha256(archive):
            raise PublicationError(f"{name} does not match {tag}'s SHA256SUMS")
        self.github.verify_attestation(sums)
        self.github.verify_attestation(archive)
        directory = self.work / "releases" / tag
        if directory.exists():
            shutil.rmtree(directory)
        directory.mkdir(parents=True)
        try:
            apt_archive.safe_extract_tar(archive, directory, MAX_INPUTS_BYTES)
        except ArchiveError as error:
            raise PublicationError(f"{name}: {error}") from error
        inputs = ReleaseInputs(tag, directory)
        self._inputs[tag] = inputs
        return inputs

    def keyring(self, tag: str, pinned: Sequence[str]) -> Path:
        inputs = self.release_inputs(tag)
        candidates = sorted((inputs.directory / "keyring").glob("rmac-archive-keyring_*_all.deb"))
        if len(candidates) != 1:
            raise PublicationError(f"{tag} does not carry exactly one rmac-archive-keyring package")
        try:
            keyring_bytes = keyring_package_contract.extract_binary_keyring(candidates[0].read_bytes())
        except keyring_package_contract.KeyringPackageError as error:
            raise PublicationError(f"{tag}'s keyring package is invalid: {error}") from error
        path = self.work / "rmac-archive-keyring.gpg"
        path.write_bytes(keyring_bytes)
        if self.keyring_inspector(path) != tuple(sorted(pinned)):
            raise PublicationError(
                "the packaged archive keyring's primary keys differ from packaging/apt/archive-key.json"
            )
        return path

    def verify_bundle(self, tag: str, name: str, keyring: Path) -> Tuple[Path, Dict[str, object], object]:
        snapshot = BUNDLE_RE.fullmatch(name).group(1)
        archive = self.github.download(tag, name, self.work / "downloads" / tag)
        directory = self.work / "bundles" / snapshot
        if directory.exists():
            shutil.rmtree(directory)
        directory.mkdir(parents=True)
        try:
            names = apt_archive.safe_extract_tar(archive, directory, MAX_BUNDLE_BYTES)
        except ArchiveError as error:
            raise PublicationError(f"{name}: {error}") from error
        if any(not (item == SIDECAR_NAME or item.startswith(SNAPSHOT_PREFIX + "/")) for item in names):
            raise PublicationError(f"{name} holds files outside its snapshot")
        root = directory / SNAPSHOT_PREFIX
        contract = self.contract
        try:
            manifest, records = publisher._parse_manifest(root, contract)
            manifest["_manifest_path"] = root / str(contract["publication_manifest"])
            release, signers = self.verify_signature(root / publisher.INRELEASE_PATH, keyring)
            if not set(signers).intersection(manifest["signer_fingerprints"]):
                raise PublicationError(f"{name} is signed by a key its manifest does not authorize")
            # A retained snapshot is historical: check its window against its own Date.
            publisher.validate_release(
                release,
                manifest=manifest,
                records=records,
                contract=contract,
                now_seconds=int(manifest["date_seconds"]),
            )
            maximum = int(contract["maximum_metadata_bytes"])
            publisher._verify_package_indices(root, records, maximum)
            publisher._verify_source_index(root, records, maximum)
        except publisher.PublisherError as error:
            raise PublicationError(f"{name}: {error}") from error
        if manifest["snapshot"] != snapshot:
            raise PublicationError(f"{name} holds snapshot {manifest['snapshot']}")
        publication = publisher.Publication(
            snapshot=manifest["snapshot"],
            product_revision=manifest["product_revision"],
            date_seconds=manifest["date_seconds"],
            valid_until_seconds=manifest["valid_until_seconds"],
            signers=tuple(manifest["signer_fingerprints"]),
            records=records,
            release_bytes=release,
            manifest_identity=publisher._hash_file(root / str(contract["publication_manifest"])),
            inrelease_identity=publisher._hash_file(root / publisher.INRELEASE_PATH),
        )
        expected = set(publisher._metadata_paths(publication, contract))
        actual = {item[len(SNAPSHOT_PREFIX) + 1:] for item in names if item != SIDECAR_NAME}
        if actual != expected:
            raise PublicationError(f"{name} metadata inventory is not exact")
        for relative in expected - {publisher.INRELEASE_PATH}:
            if publisher._hash_file(root / relative) != publisher._metadata_identity(
                relative, publication, contract
            ):
                raise PublicationError(f"{name} metadata differs from its signed manifest")
        try:
            sidecar = json.loads((directory / SIDECAR_NAME).read_text(encoding="utf-8"))
        except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
            raise PublicationError(f"{name} has no readable sidecar") from error
        pool = {record.path for record in records if record.path.startswith("pool/")}
        origins = sidecar.get("origins") if isinstance(sidecar, dict) else None
        phases = [
            paragraph.get("Phased-Update-Percentage")
            for paragraph in apt_archive.read_deb822_file(
                root / "dists/resolute/main/binary-amd64/Packages", "retained Packages index"
            )
            if paragraph.get("Package") == "rmac-apps"
        ]
        if (
            not isinstance(sidecar, dict)
            or sidecar.get("format") != 1
            or sidecar.get("snapshot") != snapshot
            or not isinstance(origins, dict)
            or set(origins) != pool
            or any(not isinstance(value, str) or not TAG_RE.fullmatch(value) for value in origins.values())
            or not isinstance(sidecar.get("release_tag"), str)
            or not TAG_RE.fullmatch(sidecar["release_tag"])
            or sidecar.get("release_tag") != tag
            or phases != [str(sidecar.get("phase"))]
            or type(sidecar.get("phase_since_seconds")) is not int
            or not 0 <= sidecar["phase_since_seconds"] <= manifest["date_seconds"]
            or sidecar.get("product_revision") != manifest["product_revision"]
        ):
            raise PublicationError(f"{name}'s sidecar does not match its signed snapshot")
        return root, sidecar, publication

    def collect(
        self,
        *,
        retain: int,
        target_tag: Optional[str],
        allow_first_publication: bool,
        pinned: Sequence[str],
    ) -> Dict[str, object]:
        minimum = int(self.contract["minimum_retained_snapshots"])
        if not minimum <= retain <= 32:
            raise PublicationError("retained snapshot count is outside the reviewed bound")
        if target_tag is not None and not TAG_RE.fullmatch(target_tag):
            raise PublicationError("target tag is invalid")
        bundles: List[Tuple[str, str, str]] = []  # (snapshot, tag, asset)
        for tag, assets in self.releases().items():
            for name in assets:
                match = BUNDLE_RE.fullmatch(name)
                if match:
                    bundles.append((match.group(1), tag, name))
        snapshots = [snapshot for snapshot, _, _ in bundles]
        if len(set(snapshots)) != len(snapshots):
            raise PublicationError("one snapshot is attached to more than one release")
        bundles.sort(reverse=True)
        summary: Dict[str, object] = {"format": 1}

        if not bundles:
            if target_tag is None:
                summary.update({"live": False})
                return summary
            if not allow_first_publication:
                raise PublicationError(
                    "no published APT snapshot was found on any release; refusing to start a new "
                    "repository history. For the very first publication only, set the repository "
                    "variable RMAC_APT_FIRST_PUBLICATION=true, then delete it."
                )
            keyring = self.keyring(target_tag, pinned)
            summary.update(
                {
                    "first_publication": True,
                    "inputs": str(self.release_inputs(target_tag).directory),
                    "keyring": str(keyring),
                    "latest": None,
                    "live": False,
                    "previous_repository": None,
                    "previous_sidecar": None,
                    "retained": [],
                    "target_tag": target_tag,
                }
            )
            return summary
        if allow_first_publication:
            raise PublicationError(
                "RMAC_APT_FIRST_PUBLICATION is still set although a published history exists; delete it"
            )

        retained = bundles[:retain]
        latest_tag = retained[0][1]
        target = target_tag or latest_tag
        keyring = self.keyring(target, pinned)
        verified = [self.verify_bundle(tag, name, keyring) for _, tag, name in retained]

        previous = self.work / "previous"
        if previous.exists():
            shutil.rmtree(previous)
        previous.mkdir(parents=True)
        state = previous / str(self.contract["state_directory"]) / "snapshots"
        state.mkdir(parents=True)
        for index, (root, sidecar, publication) in enumerate(verified):
            shutil.copytree(root, state / publication.snapshot)
            for relative in publisher._metadata_paths(publication, self.contract):
                is_by_hash = "/by-hash/" in relative
                if index == 0 or is_by_hash:
                    destination = previous / relative
                    if destination.exists():
                        if publisher._hash_file(destination) != publisher._hash_file(root / relative):
                            raise PublicationError("retained snapshots disagree about one metadata file")
                        continue
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(root / relative, destination)
            for record in publication.records:
                if not record.path.startswith("pool/"):
                    continue
                inputs = self.release_inputs(str(sidecar["origins"][record.path]))
                source = inputs.by_digest.get((record.size, record.sha256))
                if source is None or publisher._hash_file(source)[2] != record.sha512:
                    raise PublicationError(
                        f"{record.path} is not in {inputs.tag}'s verified apt inputs"
                    )
                destination = previous / record.path
                if destination.exists():
                    if publisher._hash_file(destination) != (record.size, record.sha256, record.sha512):
                        raise PublicationError(f"retained snapshots disagree about {record.path}")
                    continue
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(source, destination)

        latest_root, latest_sidecar, latest = verified[0]
        previous_sidecar = self.work / "previous-sidecar.json"
        previous_sidecar.write_text(json.dumps(latest_sidecar, sort_keys=True, indent=2) + "\n", encoding="utf-8")
        summary.update(
            {
                "first_publication": False,
                "inputs": str(self.release_inputs(target).directory),
                "keyring": str(keyring),
                "latest": {
                    "date_seconds": latest.date_seconds,
                    "phase": latest_sidecar["phase"],
                    "phase_since_seconds": latest_sidecar["phase_since_seconds"],
                    "product_revision": latest.product_revision,
                    "release_tag": latest_sidecar["release_tag"],
                    "snapshot": latest.snapshot,
                    "valid_until_seconds": latest.valid_until_seconds,
                },
                "live": True,
                "previous_repository": str(previous),
                "previous_sidecar": str(previous_sidecar),
                "retained": [publication.snapshot for _, _, publication in verified],
                "target_tag": target,
            }
        )
        return summary


# --- decide --------------------------------------------------------------------------


def decide(
    summary: Dict[str, object],
    *,
    mode: str,
    now_seconds: int,
    requested_phase: Optional[int] = None,
) -> Dict[str, object]:
    if mode == "release":
        phase = RELEASE_PHASE if requested_phase is None else requested_phase
        if phase not in rollout.ALL_PERCENTAGES:
            raise PublicationError("a release phase must be one of 0, 10, 25, 50, 100")
        return {"action": "publish", "phase": phase, "reason": "new release"}
    if mode != "rollout":
        raise PublicationError("mode must be release or rollout")
    if not summary.get("live"):
        return {"action": "none", "reason": "no repository is published yet"}
    latest = summary["latest"]
    try:
        step = rollout.next_phase(
            current_phase=int(latest["phase"]),
            current_date_seconds=int(latest["phase_since_seconds"]),
            now_seconds=now_seconds,
            requested_phase=requested_phase,
        )
    except rollout.RolloutError as error:
        raise PublicationError(str(error)) from error
    if step is not None and (step != latest["phase"] or requested_phase is not None):
        reason = "manual phase request" if requested_phase is not None else "scheduled phase step"
        return {"action": "publish", "phase": step, "reason": reason}
    if int(latest["valid_until_seconds"]) - now_seconds < REFRESH_MARGIN_SECONDS:
        return {"action": "publish", "phase": int(latest["phase"]), "reason": "signature refresh"}
    return {"action": "none", "reason": "nothing to do yet"}


def newest_snapshot(github) -> Optional[str]:
    """The newest snapshot bundle attached to any non-draft Release."""
    newest: Optional[str] = None
    for release in github.releases():
        if release.get("draft"):
            continue
        for name in release.get("assets", []):
            match = BUNDLE_RE.fullmatch(str(name))
            if match and (newest is None or match.group(1) > newest):
                newest = match.group(1)
    return newest


# --- bundle --------------------------------------------------------------------------


def bundle(repository: Path, sidecar_path: Path, output_directory: Path) -> Path:
    contract = publisher.load_contract()
    try:
        sidecar = json.loads(sidecar_path.read_text(encoding="utf-8"))
        manifest = json.loads((repository / str(contract["publication_manifest"])).read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise PublicationError("the published snapshot or its sidecar is unreadable") from error
    snapshot = sidecar.get("snapshot")
    if snapshot != manifest.get("snapshot") or not BUNDLE_RE.fullmatch(f"apt-snapshot-{snapshot}.tar"):
        raise PublicationError("the sidecar does not describe the visible snapshot")
    source = repository / str(contract["state_directory"]) / "snapshots" / str(snapshot)
    if not source.is_dir() or source.is_symlink():
        raise PublicationError("the publisher did not retain the new snapshot")
    output_directory.mkdir(parents=True, exist_ok=True)
    path = output_directory / f"apt-snapshot-{snapshot}.tar"
    if path.exists():
        raise PublicationError(f"{path.name} already exists")

    def normalize(info: tarfile.TarInfo) -> tarfile.TarInfo:
        info.uid = info.gid = 0
        info.uname = info.gname = ""
        info.mode = 0o755 if info.isdir() else 0o644
        return info

    with tarfile.open(path, "w", format=tarfile.PAX_FORMAT) as archive:
        archive.add(str(sidecar_path), arcname=SIDECAR_NAME, filter=normalize)
        for item in sorted(source.rglob("*")):
            if item.is_symlink() or not (item.is_file() or item.is_dir()):
                raise PublicationError("the retained snapshot holds a link or special file")
            if item.is_file():
                archive.add(
                    str(item),
                    arcname=f"{SNAPSHOT_PREFIX}/{item.relative_to(source).as_posix()}",
                    filter=normalize,
                )
    return path


# --- CLI -----------------------------------------------------------------------------


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)

    collect_parser = commands.add_parser("collect")
    collect_parser.add_argument("--repository", required=True)
    collect_parser.add_argument("--work", required=True, type=Path)
    collect_parser.add_argument("--retain", type=int, default=3)
    collect_parser.add_argument("--target-tag")
    collect_parser.add_argument("--allow-first-publication", action="store_true")

    decide_parser = commands.add_parser("decide")
    decide_parser.add_argument("--summary", required=True, type=Path)
    decide_parser.add_argument("--mode", required=True, choices=("release", "rollout"))
    decide_parser.add_argument("--requested-phase", type=int)
    decide_parser.add_argument("--now-seconds", type=int)

    bundle_parser = commands.add_parser("bundle")
    bundle_parser.add_argument("--repository-dir", required=True, type=Path)
    bundle_parser.add_argument("--sidecar", required=True, type=Path)
    bundle_parser.add_argument("--output-dir", required=True, type=Path)

    current_parser = commands.add_parser(
        "is-newest", help="exit 0 when SNAPSHOT is the newest published bundle, 3 when a newer one exists"
    )
    current_parser.add_argument("--repository", required=True)
    current_parser.add_argument("--snapshot", required=True)

    arguments = parser.parse_args(argv)
    try:
        if arguments.command == "is-newest":
            newest = newest_snapshot(GitHub(arguments.repository))
            if newest is None or newest < arguments.snapshot:
                raise PublicationError("the snapshot being deployed was never recorded on a release")
            if newest != arguments.snapshot:
                print(f"a newer snapshot {newest} supersedes {arguments.snapshot}; not deploying")
                return 3
            print(f"{arguments.snapshot} is the newest snapshot")
            return 0
        if arguments.command == "collect":
            if not arguments.work.is_absolute():
                raise PublicationError("--work must be absolute")
            arguments.work.mkdir(parents=True, exist_ok=True)
            collector = Collector(GitHub(arguments.repository), arguments.work)
            summary = collector.collect(
                retain=arguments.retain,
                target_tag=arguments.target_tag,
                allow_first_publication=arguments.allow_first_publication,
                pinned=pinned_fingerprints(),
            )
            print(json.dumps(summary, sort_keys=True))
        elif arguments.command == "decide":
            summary = json.loads(arguments.summary.read_text(encoding="utf-8"))
            now = (
                int(datetime.now(timezone.utc).timestamp())
                if arguments.now_seconds is None
                else arguments.now_seconds
            )
            print(
                json.dumps(
                    decide(
                        summary,
                        mode=arguments.mode,
                        now_seconds=now,
                        requested_phase=arguments.requested_phase,
                    ),
                    sort_keys=True,
                )
            )
        else:
            print(bundle(arguments.repository_dir, arguments.sidecar, arguments.output_dir))
    except (PublicationError, ArchiveError, OSError, json.JSONDecodeError) as error:
        parser.exit(5, f"apt-publication: {error}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
