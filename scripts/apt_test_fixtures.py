"""Synthetic release package sets for the APT staging and publication tests.

Builds real (tiny) Debian binary packages -- an ar archive with xz control
and data members -- plus `.dsc` source packages with correct checksums, laid
out exactly like an extracted ``apt-inputs-<tag>.tar`` from release.yml.
"""

from __future__ import annotations

import hashlib
import importlib.util
import io
import json
from pathlib import Path
import shutil
import sys
import tarfile
from typing import Callable, Dict, Iterable, List, Optional, Tuple


LINUX = Path(__file__).resolve().parent / "linux"
if str(LINUX) not in sys.path:
    sys.path.insert(0, str(LINUX))

SIGNER = "A" * 40
MAINTAINER = "Jacob Samas <samasjacob@icloud.com>"

# publish() below exercises publisher.promote() against a synthetic,
# throwaway work directory, not the real archive host, so it should not be
# subject to the real host's free disk space. Tests that want to exercise
# the storage-floor check itself inject their own free_bytes into
# publisher.promote() directly (see test_apt_publisher.py).
PLENTY_OF_SPACE: Callable[[Path], int] = lambda _path: 1 << 50  # noqa: E731


def load_script(name: str, filename: str):
    if name in sys.modules:
        return sys.modules[name]
    spec = importlib.util.spec_from_file_location(name, LINUX / filename)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def _tar_xz(members: Dict[str, Tuple[bytes, int]]) -> bytes:
    raw = io.BytesIO()
    with tarfile.open(fileobj=raw, mode="w:xz") as archive:
        for name, (data, mode) in sorted(members.items()):
            info = tarfile.TarInfo(name)
            info.size = len(data)
            info.mode = mode
            archive.addfile(info, io.BytesIO(data))
    return raw.getvalue()


def _ar(members: List[Tuple[str, bytes]]) -> bytes:
    output = bytearray(b"!<arch>\n")
    for name, data in members:
        header = f"{name:<16}{0:<12}{0:<6}{0:<6}{'100644':<8}{len(data):<10}`\n"
        output += header.encode("ascii") + data
        if len(data) % 2:
            output += b"\n"
    return bytes(output)


def make_deb(path: Path, fields: List[Tuple[str, str]], files: Dict[str, bytes]) -> Path:
    control = "\n".join(
        f"{name}: {value}" if "\n" not in value else f"{name}: " + value.replace("\n", "\n ")
        for name, value in fields
    ) + "\n"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(
        _ar(
            [
                ("debian-binary", b"2.0\n"),
                ("control.tar.xz", _tar_xz({"./control": (control.encode(), 0o644)})),
                ("data.tar.xz", _tar_xz({f"./{name}": (data, 0o644) for name, data in files.items()})),
            ]
        )
    )
    return path


def _sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def make_source(
    directory: Path,
    name: str,
    version: str,
    files: Dict[str, bytes],
    binaries: Iterable[str],
) -> Path:
    directory.mkdir(parents=True, exist_ok=True)
    rows = []
    for filename, data in sorted(files.items()):
        (directory / filename).write_bytes(data)
        rows.append(f" {_sha256(data)} {len(data)} {filename}")
    dsc = directory / f"{name}_{version}.dsc"
    dsc.write_text(
        "\n".join(
            [
                "Format: 3.0 (quilt)",
                f"Source: {name}",
                "Binary: " + ", ".join(binaries),
                "Architecture: any",
                f"Version: {version}",
                f"Maintainer: {MAINTAINER}",
                "Build-Depends: dpkg-dev (>= 1.22), python3",
                "Checksums-Sha256:",
                *rows,
            ]
        )
        + "\n",
        encoding="utf-8",
    )
    return dsc


def _description(summary: str) -> str:
    return f"{summary}\nA longer description line.\n.\nA second paragraph."


def build_inputs(
    root: Path,
    *,
    rmac: str = "1.0.0-38",
    niri: str = "26.04-0lulo1",
    xwayland: str = "0.8.2-0lulo1",
    keyring: str = "1.0.0-1",
    architectures: Tuple[str, ...] = ("amd64",),
    salt: str = "",
) -> Path:
    """An extracted apt-inputs tree; `salt` changes rebuilt bytes, not versions."""
    inputs = root
    for architecture in architectures:
        native = inputs / f"native-{architecture}"
        packages = []
        for package, depends in (
            ("rmac-apps", "libc6 (>= 2.39), wl-clipboard"),
            ("rmac-session", f"niri (>= 26.04), rmac-apps (= {rmac}), xwayland-satellite (>= 0.8.2)"),
        ):
            filename = f"{package}_{rmac}_{architecture}.deb"
            make_deb(
                native / filename,
                [
                    ("Package", package),
                    ("Version", rmac),
                    ("Section", "x11"),
                    ("Priority", "optional"),
                    ("Architecture", architecture),
                    ("Maintainer", MAINTAINER),
                    ("Installed-Size", "42"),
                    ("Depends", depends),
                    ("Description", _description(f"{package} summary")),
                ],
                {f"usr/share/doc/{package}/build": f"{package} {architecture}{salt}".encode()},
            )
            packages.append({"filename": filename, "package": package})
        (native / "native-packages.json").write_text(
            json.dumps({"architecture": architecture, "format": 1, "packages": packages, "version": rmac}),
            encoding="utf-8",
        )
        third = inputs / f"third-party-{architecture}"
        for name, version, depends in (
            ("niri", niri, "libc6, xwayland-satellite (>= 0.8.2)"),
            ("xwayland-satellite", xwayland, "libc6, xwayland"),
        ):
            make_deb(
                third / f"{name}_{version}_{architecture}.deb",
                [
                    ("Package", name),
                    ("Version", version),
                    ("Architecture", architecture),
                    ("Maintainer", MAINTAINER),
                    ("Installed-Size", "7"),
                    ("Depends", depends),
                    ("Section", "x11"),
                    ("Priority", "optional"),
                    ("Homepage", f"https://example.invalid/{name}"),
                    ("Description", _description(f"{name} (Lulo OS build)")),
                ],
                {f"usr/bin/{name}": f"{name} {architecture}{salt}".encode()},
            )
            for suffix in ("buildinfo", "changes"):
                (third / f"{name}_{version}_{architecture}.{suffix}").write_text(
                    f"{name} {architecture} {suffix}{salt}\n", encoding="utf-8"
                )
    third = inputs / "third-party-amd64"
    for name, version in (("niri", niri), ("xwayland-satellite", xwayland)):
        upstream = version.rsplit("-", 1)[0]
        make_source(
            third,
            name,
            version,
            {
                f"{name}_{upstream}.orig.tar.gz": f"{name} upstream {upstream}".encode(),
                f"{name}_{upstream}.orig-vendor.tar.xz": f"{name} vendor {upstream}".encode(),
                f"{name}_{version}.debian.tar.xz": f"{name} debian {version}{salt}".encode(),
            },
            [name],
        )
    source = inputs / "rmac-source"
    upstream = rmac.rsplit("-", 1)[0]
    make_source(
        source,
        "rmac",
        rmac,
        {
            f"rmac_{upstream}.orig.tar.xz": f"rmac upstream {upstream}".encode(),
            f"rmac_{upstream}.orig-vendor.tar.xz": f"rmac vendor {upstream}".encode(),
            f"rmac_{rmac}.debian.tar.xz": f"rmac debian {rmac}{salt}".encode(),
        },
        ["rmac-apps", "rmac-session"],
    )
    for suffix in ("buildinfo", "changes"):
        (source / f"rmac_{rmac}_source.{suffix}").write_text(f"rmac source {suffix}{salt}\n", encoding="utf-8")
    ring = inputs / "keyring"
    make_deb(
        ring / f"rmac-archive-keyring_{keyring}_all.deb",
        [
            ("Package", "rmac-archive-keyring"),
            ("Version", keyring),
            ("Architecture", "all"),
            ("Maintainer", MAINTAINER),
            ("Installed-Size", "1"),
            ("Section", "misc"),
            ("Priority", "optional"),
            ("Description", _description("rmac archive keyring")),
        ],
        {"usr/share/keyrings/rmac-archive-keyring.gpg": b"public keyring bytes"},
    )
    keyring_upstream = keyring.rsplit("-", 1)[0]
    make_source(
        ring,
        "rmac-archive-keyring",
        keyring,
        {
            f"rmac-archive-keyring_{keyring_upstream}.orig.tar.xz": f"keyring upstream {keyring_upstream}".encode(),
            f"rmac-archive-keyring_{keyring}.debian.tar.xz": f"keyring debian {keyring}".encode(),
        },
        ["rmac-archive-keyring"],
    )
    for suffix in ("buildinfo", "changes"):
        (ring / f"rmac-archive-keyring_{keyring}_all.{suffix}").write_text(f"keyring {suffix}\n", encoding="utf-8")
    (ring / "keyring-packages.json").write_text(
        json.dumps({"format": 1, "package": "rmac-archive-keyring", "package_architecture": "all", "version": keyring}),
        encoding="utf-8",
    )
    return inputs


def stage_arguments(inputs: Path, **overrides) -> Dict[str, object]:
    architectures = [
        architecture for architecture in ("amd64", "arm64") if (inputs / f"native-{architecture}").is_dir()
    ]
    arguments: Dict[str, object] = {
        "native_dirs": {architecture: inputs / f"native-{architecture}" for architecture in architectures},
        "third_party_dirs": {architecture: inputs / f"third-party-{architecture}" for architecture in architectures},
        "keyring_dir": inputs / "keyring",
        "rmac_source_dir": inputs / "rmac-source",
        "phase": 10,
        "valid_hours": 24,
        "signer_fingerprints": [SIGNER],
        "product_revision": "b" * 40,
        "release_tag": "v1.0.0",
        "now_seconds": 1_700_000_000,
        "gate_binary_packages": True,
        "gate_licenses": True,
        "gate_reproducibility": True,
        "gate_source_offer": True,
    }
    arguments.update(overrides)
    return arguments


def unsigned_verifier(inrelease: Path, keyring: Path, *_ignored) -> Tuple[bytes, Tuple[str, ...]]:
    """Stand-in for gpgv in tests without GnuPG: "InRelease" is the plain Release."""
    return inrelease.read_bytes(), (SIGNER,)


class FakeGitHub:
    """GitHub Releases backed by a directory, with explicit attestations."""

    def __init__(self, root: Path):
        self.root = root
        self.tags: List[str] = []
        self.attested: set = set()

    def add_release(self, tag: str, inputs: Path, *, attest: bool = True) -> None:
        directory = self.root / tag
        directory.mkdir(parents=True)
        archive = directory / f"apt-inputs-{tag}.tar"
        with tarfile.open(archive, "w") as output:
            output.add(str(inputs), arcname=".")
        sums = directory / "SHA256SUMS"
        sums.write_text(f"{_sha256(archive.read_bytes())}  {archive.name}\n", encoding="utf-8")
        if attest:
            self.attested.add(_sha256(archive.read_bytes()))
            self.attested.add(_sha256(sums.read_bytes()))
        self.tags.append(tag)

    def releases(self):
        return [
            {"tag": tag, "draft": False, "assets": sorted(path.name for path in (self.root / tag).iterdir())}
            for tag in reversed(self.tags)
        ]

    def download(self, tag: str, name: str, directory: Path) -> Path:
        directory.mkdir(parents=True, exist_ok=True)
        source = self.root / tag / name
        if not source.is_file():
            raise FileNotFoundError(name)
        destination = directory / name
        shutil.copyfile(source, destination)
        return destination

    def verify_attestation(self, path: Path) -> None:
        publication = load_script("rmac_apt_publication", "apt-publication.py")
        if _sha256(path.read_bytes()) not in self.attested:
            raise publication.PublicationError(f"no attestation for {path.name}")

    def upload(self, tag: str, path: Path) -> None:
        destination = self.root / tag / path.name
        if destination.exists():
            raise FileExistsError(path.name)
        shutil.copyfile(path, destination)


def publish(
    github: FakeGitHub,
    work: Path,
    *,
    mode: str,
    now: int,
    tag: Optional[str] = None,
    allow_first: bool = False,
    requested: Optional[int] = None,
    retain: int = 3,
    free_bytes: Callable[[Path], int] = PLENTY_OF_SPACE,
):
    """publish-apt-repository.sh's steps, with a plain InRelease instead of gpg."""
    publication = load_script("rmac_apt_publication", "apt-publication.py")
    stager = load_script("rmac_stage_apt_snapshot", "stage-apt-snapshot.py")
    publisher = publication.publisher
    work.mkdir(parents=True)
    collector = publication.Collector(
        github, work / "state", keyring_inspector=lambda _path: (SIGNER,)
    )
    summary = collector.collect(
        retain=retain,
        target_tag=tag if mode == "release" else None,
        allow_first_publication=allow_first,
        pinned=(SIGNER,),
    )
    decision = publication.decide(summary, mode=mode, now_seconds=now, requested_phase=requested)
    if decision["action"] != "publish":
        return summary, decision, None
    inputs = Path(summary["inputs"])
    previous = summary["previous_repository"]
    previous_sidecar = (
        json.loads(Path(summary["previous_sidecar"]).read_text(encoding="utf-8")) if previous else None
    )
    staged = work / "staged"
    sidecar = work / "rmac-snapshot.json"
    stager.stage(
        **stage_arguments(
            inputs,
            output=staged,
            sidecar_output=sidecar,
            phase=decision["phase"],
            valid_hours=48,
            release_tag=summary["target_tag"],
            product_revision=(
                "c" * 40 if mode == "release" else summary["latest"]["product_revision"]
            ),
            previous_repository=Path(previous) if previous else None,
            previous_sidecar=previous_sidecar,
            rollout_only=mode == "rollout",
            now_seconds=now,
        )
    )
    release = staged / "dists/resolute/Release"
    release.rename(staged / publisher.INRELEASE_PATH)
    keyring = Path(summary["keyring"])
    result = publisher.validate_staging(staged, keyring, now_seconds=now)
    repository = work / "repository"
    if previous:
        Path(previous).rename(repository)
    else:
        repository.mkdir()
    publisher.promote(staged, repository, keyring, result, retain=retain, free_bytes=free_bytes)
    bundle = publication.bundle(repository, sidecar, work / "bundle")
    github.upload(summary["target_tag"], bundle)
    return summary, decision, repository


def tree_digest(root: Path, subdirectories: Iterable[str] = ("dists", "pool")) -> Dict[str, str]:
    result = {}
    for subdirectory in subdirectories:
        for path in sorted((root / subdirectory).rglob("*")):
            if path.is_file():
                result[path.relative_to(root).as_posix()] = _sha256(path.read_bytes())
    return result
