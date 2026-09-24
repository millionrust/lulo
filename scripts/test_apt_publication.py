"""Stateless APT publication rebuilt from GitHub Releases (SR-12).

The whole release/rollout pipeline runs against a directory-backed fake of
GitHub Releases, with a plain InRelease standing in for the gpg signature
(the gpgv path itself is covered by test_apt_publisher.py and
test_apt_snapshot_staging.py when GnuPG is installed).
"""

from __future__ import annotations

import io
import json
from pathlib import Path
import shutil
import tarfile
import tempfile
import unittest
from unittest import mock

import apt_test_fixtures as fixtures


publication = fixtures.load_script("rmac_apt_publication", "apt-publication.py")
stager = fixtures.load_script("rmac_stage_apt_snapshot", "stage-apt-snapshot.py")
publisher = publication.publisher
apt_archive = __import__("apt_archive")

DAY = 24 * 3600
T0 = 1_750_000_000


class PublicationTestCase(unittest.TestCase):
    def setUp(self):
        self._temporary = tempfile.TemporaryDirectory()
        self.root = Path(self._temporary.name)
        self.github = fixtures.FakeGitHub(self.root / "github")
        self.runs = 0
        patcher = mock.patch.object(publisher, "_verify_inrelease", fixtures.unsigned_verifier)
        patcher.start()
        self.addCleanup(patcher.stop)
        self.addCleanup(self._temporary.cleanup)

    def release(self, tag: str, **versions) -> Path:
        inputs = fixtures.build_inputs(self.root / "builds" / tag, **versions)
        self.github.add_release(tag, inputs)
        return inputs

    def publish(self, **arguments):
        self.runs += 1
        return fixtures.publish(self.github, self.root / f"run-{self.runs}", **arguments)

    def bundles(self, tag: str):
        return sorted(path.name for path in (self.root / "github" / tag).glob("apt-snapshot-*.tar"))


class FirstPublicationTests(PublicationTestCase):
    def test_no_history_fails_closed_without_the_explicit_first_publication_flag(self):
        self.release("v1.0.0")
        with self.assertRaisesRegex(publication.PublicationError, "RMAC_APT_FIRST_PUBLICATION"):
            self.publish(mode="release", tag="v1.0.0", now=T0)

    def test_first_publication_serves_every_lulo_package_and_records_its_state(self):
        self.release("v1.0.0")
        summary, decision, repository = self.publish(mode="release", tag="v1.0.0", now=T0, allow_first=True)
        self.assertTrue(summary["first_publication"])
        self.assertEqual(decision, {"action": "publish", "phase": 10, "reason": "new release"})
        names = self.bundles("v1.0.0")
        self.assertEqual(len(names), 1)
        with tarfile.open(self.root / "github/v1.0.0" / names[0]) as archive:
            members = archive.getnames()
        self.assertIn("rmac-snapshot.json", members)
        self.assertIn("snapshot/dists/resolute/InRelease", members)
        self.assertFalse(any(name.startswith("snapshot/pool/") for name in members))
        amd64 = apt_archive.read_deb822_file(
            repository / "dists/resolute/main/binary-amd64/Packages", "Packages"
        )
        self.assertEqual(len(amd64), 5)

    def test_the_first_publication_flag_is_refused_once_history_exists(self):
        self.release("v1.0.0")
        self.publish(mode="release", tag="v1.0.0", now=T0, allow_first=True)
        self.release("v1.0.1", rmac="1.0.1-38", keyring="1.0.1-1")
        with self.assertRaisesRegex(publication.PublicationError, "still set"):
            self.publish(mode="release", tag="v1.0.1", now=T0 + DAY, allow_first=True)

    def test_a_rollout_before_any_publication_does_nothing(self):
        self.release("v1.0.0")
        summary, decision, repository = self.publish(mode="rollout", now=T0)
        self.assertFalse(summary["live"])
        self.assertEqual(decision["action"], "none")
        self.assertIsNone(repository)


class ReleaseHistoryTests(PublicationTestCase):
    def setUp(self):
        super().setUp()
        self.release("v1.0.0")
        _, _, self.first = self.publish(mode="release", tag="v1.0.0", now=T0, allow_first=True)

    def test_the_rebuilt_repository_is_the_published_one_byte_for_byte(self):
        self.release("v1.0.1", rmac="1.0.1-38", keyring="1.0.1-1")
        collector = publication.Collector(
            self.github, self.root / "inspect", keyring_inspector=lambda _path: (fixtures.SIGNER,)
        )
        summary = collector.collect(
            retain=3, target_tag="v1.0.1", allow_first_publication=False, pinned=(fixtures.SIGNER,)
        )
        rebuilt = Path(summary["previous_repository"])
        self.assertEqual(fixtures.tree_digest(rebuilt), fixtures.tree_digest(self.first))
        self.assertEqual(summary["latest"]["release_tag"], "v1.0.0")
        self.assertEqual(summary["latest"]["phase"], 10)

    def test_a_new_release_carries_an_unchanged_niri_forward_byte_for_byte(self):
        # v1.0.1 rebuilds niri 26.04-0lulo1 with different bytes; the pool is
        # immutable, so the published v1.0.0 objects keep being served.
        self.release("v1.0.1", rmac="1.0.1-38", keyring="1.0.1-1", salt="-rebuilt")
        _, _, second = self.publish(mode="release", tag="v1.0.1", now=T0 + DAY)
        niri = "pool/main/n/niri/niri_26.04-0lulo1_amd64.deb"
        self.assertEqual(
            fixtures.tree_digest(second)[niri], fixtures.tree_digest(self.first)[niri]
        )
        sidecar = json.loads(
            (self.root / "run-2" / "rmac-snapshot.json").read_text(encoding="utf-8")
        )
        self.assertEqual(sidecar["origins"][niri], "v1.0.0")
        self.assertEqual(sidecar["origins"]["pool/main/r/rmac/rmac-apps_1.0.1-38_amd64.deb"], "v1.0.1")
        packages = {
            paragraph["Package"]: paragraph["Version"]
            for paragraph in apt_archive.read_deb822_file(
                second / "dists/resolute/main/binary-amd64/Packages", "Packages"
            )
        }
        self.assertEqual(packages["rmac-apps"], "1.0.1-38")
        self.assertEqual(packages["niri"], "26.04-0lulo1")
        # A third release rebuilds the pool from both earlier Releases.
        self.release("v1.0.2", rmac="1.0.2-38", keyring="1.0.2-1")
        _, _, third = self.publish(mode="release", tag="v1.0.2", now=T0 + 2 * DAY)
        self.assertEqual(fixtures.tree_digest(third)[niri], fixtures.tree_digest(self.first)[niri])

    def test_an_older_version_is_never_published_again(self):
        self.release("v1.0.1", rmac="1.0.1-38", keyring="1.0.1-1")
        self.publish(mode="release", tag="v1.0.1", now=T0 + DAY)
        self.release("v0.9.9", rmac="0.9.9-38", keyring="0.9.9-1")
        with self.assertRaisesRegex(stager.StagingError, "older than the published"):
            self.publish(mode="release", tag="v0.9.9", now=T0 + 2 * DAY)

    def test_a_reused_orig_tarball_name_with_new_bytes_is_refused(self):
        # A Debian-revision-only bump must keep the same orig tarball.
        inputs = fixtures.build_inputs(self.root / "builds/v1.0.0-r", rmac="1.0.0-39", keyring="1.0.0-2")
        (inputs / "rmac-source/rmac_1.0.0.orig.tar.xz").write_bytes(b"different upstream")
        fixtures.make_source(
            inputs / "rmac-source",
            "rmac",
            "1.0.0-39",
            {
                "rmac_1.0.0.orig.tar.xz": b"different upstream",
                "rmac_1.0.0.orig-vendor.tar.xz": b"rmac vendor 1.0.0",
                "rmac_1.0.0-39.debian.tar.xz": b"rmac debian 1.0.0-39",
            },
            ["rmac-apps", "rmac-session"],
        )
        self.github.add_release("v1.0.0-r", inputs)
        with self.assertRaisesRegex(stager.StagingError, "already published with different bytes"):
            self.publish(mode="release", tag="v1.0.0-r", now=T0 + DAY)

    def test_monotonic_date_is_checked_against_the_rebuilt_state(self):
        self.release("v1.0.1", rmac="1.0.1-38", keyring="1.0.1-1")
        with self.assertRaisesRegex(publisher.PublisherError, "must both increase"):
            self.publish(mode="release", tag="v1.0.1", now=T0 - 60)

    def test_a_tampered_snapshot_bundle_fails_closed(self):
        name = self.bundles("v1.0.0")[0]
        path = self.root / "github/v1.0.0" / name
        replaced = io.BytesIO()
        with tarfile.open(path) as source, tarfile.open(fileobj=replaced, mode="w") as target:
            for member in source.getmembers():
                data = source.extractfile(member).read() if member.isfile() else None
                if member.name.endswith("binary-amd64/Packages"):
                    data = data.replace(b"Phased-Update-Percentage: 10", b"Phased-Update-Percentage: 100")
                    member.size = len(data)
                target.addfile(member, io.BytesIO(data) if data is not None else None)
        path.write_bytes(replaced.getvalue())
        self.release("v1.0.1", rmac="1.0.1-38", keyring="1.0.1-1")
        with self.assertRaises(publication.PublicationError):
            self.publish(mode="release", tag="v1.0.1", now=T0 + DAY)

    def test_unattested_release_inputs_are_refused(self):
        inputs = fixtures.build_inputs(self.root / "builds/v1.0.1", rmac="1.0.1-38", keyring="1.0.1-1")
        self.github.add_release("v1.0.1", inputs, attest=False)
        with self.assertRaisesRegex(publication.PublicationError, "attestation"):
            self.publish(mode="release", tag="v1.0.1", now=T0 + DAY)

    def test_inputs_that_do_not_match_sha256sums_are_refused(self):
        self.release("v1.0.1", rmac="1.0.1-38", keyring="1.0.1-1")
        (self.root / "github/v1.0.1/SHA256SUMS").write_text("0" * 64 + "  apt-inputs-v1.0.1.tar\n")
        with self.assertRaisesRegex(publication.PublicationError, "SHA256SUMS"):
            self.publish(mode="release", tag="v1.0.1", now=T0 + DAY)

    def test_a_retained_pool_object_that_vanished_from_its_release_fails_closed(self):
        self.release("v1.0.1", rmac="1.0.1-38", keyring="1.0.1-1")
        self.publish(mode="release", tag="v1.0.1", now=T0 + DAY)
        (self.root / "github/v1.0.0/apt-inputs-v1.0.0.tar").unlink()
        self.release("v1.0.2", rmac="1.0.2-38", keyring="1.0.2-1")
        with self.assertRaisesRegex(publication.PublicationError, "v1.0.0"):
            self.publish(mode="release", tag="v1.0.2", now=T0 + 2 * DAY)

    def test_a_keyring_that_does_not_match_the_pin_is_refused(self):
        self.release("v1.0.1", rmac="1.0.1-38", keyring="1.0.1-1")
        collector = publication.Collector(
            self.github, self.root / "inspect", keyring_inspector=lambda _path: ("B" * 40,)
        )
        with self.assertRaisesRegex(publication.PublicationError, "archive-key.json"):
            collector.collect(
                retain=3, target_tag="v1.0.1", allow_first_publication=False, pinned=(fixtures.SIGNER,)
            )

    def test_only_the_newest_snapshots_are_retained(self):
        for index, version in enumerate(("1.0.1", "1.0.2", "1.0.3", "1.0.4"), start=1):
            tag = f"v{version}"
            self.release(tag, rmac=f"{version}-38", keyring=f"{version}-1")
            _, _, repository = self.publish(mode="release", tag=tag, now=T0 + index * DAY)
        snapshots = sorted(
            path.name for path in (repository / ".rmac-publisher/snapshots").iterdir()
        )
        self.assertEqual(len(snapshots), 3)
        # v1.0.0's rmac packages left the retained window, but the niri it
        # first published is still what every snapshot serves.
        self.assertFalse((repository / "pool/main/r/rmac/rmac-apps_1.0.0-38_amd64.deb").exists())
        self.assertTrue((repository / "pool/main/r/rmac/rmac-apps_1.0.1-38_amd64.deb").exists())
        self.assertTrue((repository / "pool/main/n/niri/niri_26.04-0lulo1_amd64.deb").exists())


class RolloutTests(PublicationTestCase):
    def setUp(self):
        super().setUp()
        self.release("v1.0.0")
        self.publish(mode="release", tag="v1.0.0", now=T0, allow_first=True)

    def phase(self, repository: Path) -> str:
        return next(
            paragraph["Phased-Update-Percentage"]
            for paragraph in apt_archive.read_deb822_file(
                repository / "dists/resolute/main/binary-amd64/Packages", "Packages"
            )
            if paragraph["Package"] == "rmac-apps"
        )

    def test_a_fresh_repository_needs_no_tick(self):
        _, decision, repository = self.publish(mode="rollout", now=T0 + 3600)
        self.assertEqual(decision["action"], "none")
        self.assertIsNone(repository)

    def test_scheduled_ticks_step_the_phase_and_refresh_the_signature(self):
        _, decision, first = self.publish(mode="rollout", now=T0 + DAY + 60)
        self.assertEqual((decision["action"], decision["phase"]), ("publish", 25))
        self.assertEqual(self.phase(first), "25")
        # 19 hours later: too soon to step, but Valid-Until is under 30 hours away.
        refresh_time = T0 + DAY + 60 + 19 * 3600 + 60
        summary, decision, refreshed = self.publish(mode="rollout", now=refresh_time)
        self.assertEqual(decision, {"action": "publish", "phase": 25, "reason": "signature refresh"})
        sidecar = json.loads((self.root / f"run-{self.runs}" / "rmac-snapshot.json").read_text(encoding="utf-8"))
        # A refresh does not restart the phase's observation window.
        self.assertEqual(sidecar["phase_since_seconds"], T0 + DAY + 60)
        _, decision, _ = self.publish(mode="rollout", now=T0 + 2 * DAY + 120)
        self.assertEqual(decision["phase"], 50)

    def test_a_rollout_republishes_exactly_the_published_pool(self):
        before = fixtures.tree_digest(self.root / "run-1" / "repository", ("pool",))
        _, _, after = self.publish(mode="rollout", now=T0 + 3600, requested=0)
        self.assertEqual(fixtures.tree_digest(after, ("pool",)), before)
        self.assertEqual(self.phase(after), "0")
        # A halt is never resumed by the schedule.
        _, decision, _ = self.publish(mode="rollout", now=T0 + 5 * DAY)
        self.assertEqual(decision["reason"], "signature refresh")
        self.assertEqual(decision["phase"], 0)

    def test_a_rollout_step_cannot_add_packages(self):
        summary = publication.Collector(
            self.github, self.root / "inspect", keyring_inspector=lambda _path: (fixtures.SIGNER,)
        ).collect(retain=3, target_tag=None, allow_first_publication=False, pinned=(fixtures.SIGNER,))
        newer = fixtures.build_inputs(self.root / "builds/newer", rmac="1.0.1-38", keyring="1.0.1-1")
        with self.assertRaisesRegex(stager.StagingError, "rollout step cannot publish a new pool object"):
            stager.stage(
                **fixtures.stage_arguments(
                    newer,
                    output=self.root / "staged",
                    sidecar_output=self.root / "sidecar.json",
                    previous_repository=Path(summary["previous_repository"]),
                    previous_sidecar=json.loads(Path(summary["previous_sidecar"]).read_text(encoding="utf-8")),
                    rollout_only=True,
                    now_seconds=T0 + DAY,
                )
            )


class DecideTests(unittest.TestCase):
    def summary(self, *, phase=10, since=T0, valid_until=T0 + 48 * 3600):
        return {
            "live": True,
            "latest": {
                "phase": phase,
                "phase_since_seconds": since,
                "valid_until_seconds": valid_until,
            },
        }

    def test_release_defaults_to_ten_percent_and_accepts_an_override(self):
        self.assertEqual(publication.decide({}, mode="release", now_seconds=T0)["phase"], 10)
        self.assertEqual(
            publication.decide({}, mode="release", now_seconds=T0, requested_phase=100)["phase"], 100
        )

    def test_a_manual_halt_is_immediate(self):
        decision = publication.decide(self.summary(), mode="rollout", now_seconds=T0 + 60, requested_phase=0)
        self.assertEqual((decision["action"], decision["phase"]), ("publish", 0))

    def test_a_manual_step_backwards_is_refused(self):
        with self.assertRaises(publication.PublicationError):
            publication.decide(self.summary(phase=50), mode="rollout", now_seconds=T0 + 60, requested_phase=25)

    def test_full_rollout_only_refreshes(self):
        self.assertEqual(
            publication.decide(self.summary(phase=100), mode="rollout", now_seconds=T0 + 3600)["action"], "none"
        )
        decision = publication.decide(self.summary(phase=100), mode="rollout", now_seconds=T0 + 20 * 3600)
        self.assertEqual((decision["action"], decision["phase"]), ("publish", 100))


class GitHubOutputTests(unittest.TestCase):
    def test_compact_and_pretty_json_streams_both_parse(self):
        compact = '{"tag":"v1","draft":false,"assets":["a"]}\n{"tag":"v2","draft":true,"assets":[]}\n'
        pretty = '{\n  "tag": "v1",\n  "draft": false,\n  "assets": [\n    "a"\n  ]\n}\n{"tag": "v2", "draft": true, "assets": []}'
        for text in (compact, pretty):
            releases = publication.parse_json_stream(text)
            self.assertEqual([release["tag"] for release in releases], ["v1", "v2"])
        self.assertEqual(publication.parse_json_stream("\n"), [])
        with self.assertRaises(publication.PublicationError):
            publication.parse_json_stream('["not a release"]')


class NewestSnapshotTests(PublicationTestCase):
    def test_a_superseded_site_is_detected(self):
        self.release("v1.0.0")
        self.publish(mode="release", tag="v1.0.0", now=T0, allow_first=True)
        first = publication.newest_snapshot(self.github)
        self.publish(mode="rollout", now=T0 + DAY + 60)
        second = publication.newest_snapshot(self.github)
        self.assertGreater(second, first)
        self.assertIsNone(publication.newest_snapshot(fixtures.FakeGitHub(self.root / "empty")))


class BundleTests(PublicationTestCase):
    def test_bundle_refuses_a_sidecar_for_another_snapshot(self):
        self.release("v1.0.0")
        _, _, repository = self.publish(mode="release", tag="v1.0.0", now=T0, allow_first=True)
        sidecar = self.root / "other.json"
        sidecar.write_text(json.dumps({"snapshot": "20000101T000000Z"}), encoding="utf-8")
        with self.assertRaisesRegex(publication.PublicationError, "visible snapshot"):
            publication.bundle(repository, sidecar, self.root / "out")


class PublishScriptTests(unittest.TestCase):
    SCRIPT = Path(__file__).parent / "linux/publish-apt-repository.sh"

    def run_script(self, *arguments, environment=None):
        import os
        import subprocess

        return subprocess.run(
            ["bash", str(self.SCRIPT), *arguments],
            env={"PATH": os.environ["PATH"], **(environment or {})},
            capture_output=True,
            text=True,
            timeout=30,
        )

    def test_script_is_valid_bash(self):
        import subprocess

        subprocess.run(["bash", "-n", str(self.SCRIPT)], check=True)

    def test_rollout_mode_never_takes_a_tag_or_starts_a_repository(self):
        with tempfile.TemporaryDirectory() as temporary:
            result = self.run_script(
                "--mode", "rollout", "--repository", "o/r", "--work", f"{temporary}/w", "--tag", "v1.0.0"
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("do not pass --tag", result.stderr)
            result = self.run_script(
                "--mode", "rollout", "--repository", "o/r", "--work", f"{temporary}/w",
                "--allow-first-publication",
            )
            self.assertIn("never starts a repository", result.stderr)

    def test_the_signing_secret_and_pinned_fingerprint_are_required(self):
        with tempfile.TemporaryDirectory() as temporary:
            arguments = (
                "--mode", "release", "--repository", "o/r", "--work", f"{temporary}/w",
                "--tag", "v1.0.0", "--product-revision", "a" * 40,
            )
            result = self.run_script(*arguments)
            self.assertIn("RMAC_APT_SIGNING_SUBKEY is not set", result.stderr)
            pinned = json.loads(
                (Path(__file__).parent.parent / "packaging/apt/archive-key.json").read_text(encoding="utf-8")
            )["primary_fingerprints"]
            if not pinned:
                result = self.run_script(
                    *arguments,
                    environment={"RMAC_APT_SIGNING_SUBKEY": "x", "RMAC_ARCHIVE_SIGNING_FINGERPRINT": "A" * 40},
                )
                self.assertIn("not listed in packaging/apt/archive-key.json", result.stderr)

    def test_the_order_of_operations_is_fixed(self):
        text = self.SCRIPT.read_text(encoding="utf-8")
        order = [
            'apt-publication.py" "${collect_args[@]}"',
            'python3 "$linux/stage-apt-snapshot.py"',
            'bash "$linux/sign-apt-release.sh"',
            'python3 "$linux/publish-apt-snapshot.py"',
            'gh release upload "$target_tag"',
            'output "deploy=true"',
        ]
        positions = [text.index(marker) for marker in order]
        self.assertEqual(positions, sorted(positions))
        signer = (self.SCRIPT.parent / "sign-apt-release.sh").read_text(encoding="utf-8")
        # Only a subkey export is accepted: the primary must be a stub ("#").
        self.assertIn('[[ "$primary_state" == "#" ]]', signer)
        self.assertIn("mktemp -d", signer)
        self.assertIn("packaging/apt/archive-keyring.asc", text)
        self.assertIn("--rollout-only", text)
        self.assertNotIn("--clobber", text)


@unittest.skipUnless(shutil.which("gpg") and shutil.which("gpgv"), "GnuPG tools are unavailable")
class RealKeySigningTests(unittest.TestCase):
    """The owner's key script, the CI signer, and gpgv, end to end."""

    SIGNER_SCRIPT = Path(__file__).parent / "linux/sign-apt-release.sh"
    KEY_SCRIPT = Path(__file__).parent / "release/create-archive-key.sh"

    @classmethod
    def setUpClass(cls):
        import subprocess

        cls._temporary = tempfile.TemporaryDirectory()
        cls.root = Path(cls._temporary.name)
        (cls.root / "primary").write_text("primary passphrase for tests\n", encoding="utf-8")
        (cls.root / "backup").write_text("a different backup passphrase\n", encoding="utf-8")
        cls.keys = cls.root / "keys"
        subprocess.run(
            [
                "bash", str(cls.KEY_SCRIPT), "--non-interactive", "--no-write-repo",
                "--email", "archive@example.invalid", "--subkey-lifetime", "30d",
                "--output-dir", str(cls.keys),
                "--primary-passphrase-file", str(cls.root / "primary"),
                "--backup-passphrase-file", str(cls.root / "backup"),
            ],
            check=True,
            capture_output=True,
            timeout=300,
        )
        cls.fingerprint = (cls.keys / "primary-fingerprint.txt").read_text(encoding="utf-8").strip()

    @classmethod
    def tearDownClass(cls):
        cls._temporary.cleanup()

    def sign(self, release: Path, output: Path, secret: str):
        import os
        import subprocess

        return subprocess.run(
            [
                "bash", str(self.SIGNER_SCRIPT), "--release", str(release), "--output", str(output),
                "--public-keyring", str(self.keys / "archive-keyring.asc"),
            ],
            env={
                "PATH": os.environ["PATH"],
                "RMAC_APT_SIGNING_SUBKEY": secret,
                "RMAC_ARCHIVE_SIGNING_FINGERPRINT": self.fingerprint,
            },
            capture_output=True,
            text=True,
            timeout=120,
        )

    def test_the_subkey_export_signs_a_staged_release_that_the_publisher_accepts(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            inputs = fixtures.build_inputs(root / "inputs")
            staged = root / "staged"
            now = int(__import__("time").time())
            stager.stage(
                **fixtures.stage_arguments(
                    inputs,
                    output=staged,
                    sidecar_output=root / "sidecar.json",
                    signer_fingerprints=[self.fingerprint],
                    now_seconds=now,
                )
            )
            release = staged / "dists/resolute/Release"
            result = self.sign(
                release,
                staged / publisher.INRELEASE_PATH,
                (self.keys / "RMAC_APT_SIGNING_SUBKEY.asc").read_text(encoding="utf-8"),
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            release.unlink()
            verified = publisher.validate_staging(
                staged, (self.keys / "rmac-archive-keyring.gpg").resolve(), now_seconds=now
            )
            self.assertEqual(verified.signers, (self.fingerprint,))

    @unittest.skipUnless(shutil.which("apt-get") and shutil.which("apt-cache"), "APT is unavailable")
    def test_apt_itself_accepts_the_signed_repository(self):
        """A real APT client: signature via Signed-By, by-hash, Packages, Sources."""
        import os
        import subprocess
        import time

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            inputs = fixtures.build_inputs(root / "inputs", rmac="1.0.0~beta.1-38")
            staged = root / "staged"
            now = int(time.time())
            stager.stage(
                **fixtures.stage_arguments(
                    inputs,
                    output=staged,
                    sidecar_output=root / "sidecar.json",
                    signer_fingerprints=[self.fingerprint],
                    valid_hours=48,
                    now_seconds=now,
                )
            )
            release = staged / "dists/resolute/Release"
            result = self.sign(
                release,
                staged / publisher.INRELEASE_PATH,
                (self.keys / "RMAC_APT_SIGNING_SUBKEY.asc").read_text(encoding="utf-8"),
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            release.unlink()
            keyring = (self.keys / "rmac-archive-keyring.gpg").resolve()
            repository = root / "repository"
            repository.mkdir()
            publisher.promote(
                staged, repository, keyring, publisher.validate_staging(staged, keyring, now_seconds=now), retain=3
            )
            parts = root / "sources.list.d"
            parts.mkdir()
            (parts / "rmac.sources").write_text(
                "Types: deb deb-src\n"
                f"URIs: file:{repository}\n"
                "Suites: resolute\n"
                "Components: main\n"
                "Architectures: amd64 arm64\n"
                f"Signed-By: {keyring}\n"
                "Check-Valid-Until: yes\n",
                encoding="utf-8",
            )
            for directory in ("state/lists/partial", "cache/archives/partial", "preferences.d", "downloads"):
                (root / directory).mkdir(parents=True)
            (root / "status").write_text("", encoding="utf-8")
            options = [
                "-o", "Debug::NoLocking=true",
                "-o", "APT::Sandbox::User=",
                "-o", "APT::Architecture=amd64",
                "-o", "APT::Architectures::=amd64",
                "-o", "Dir::Etc::SourceList=/dev/null",
                "-o", f"Dir::Etc::SourceParts={parts}",
                "-o", f"Dir::Etc::PreferencesParts={root / 'preferences.d'}",
                "-o", f"Dir::State={root / 'state'}",
                "-o", f"Dir::State::status={root / 'status'}",
                "-o", f"Dir::Cache={root / 'cache'}",
            ]
            environment = {"PATH": os.environ["PATH"], "LC_ALL": "C"}

            def apt(tool, *arguments, cwd=None):
                return subprocess.run(
                    [tool, *options, *arguments], env=environment, cwd=cwd,
                    capture_output=True, text=True, timeout=120,
                )

            update = apt("apt-get", "update")
            self.assertEqual(update.returncode, 0, update.stdout + update.stderr)
            for warning in ("NO_PUBKEY", "not signed", "W: ", "E: "):
                self.assertNotIn(warning, update.stdout + update.stderr)
            show = apt("apt-cache", "show", "rmac-session")
            self.assertEqual(show.returncode, 0, show.stderr)
            record = apt_archive.parse_deb822(show.stdout, "apt-cache show")[0]
            self.assertEqual(record["Version"], "1.0.0~beta.1-38")
            self.assertIn("niri (>= 26.04)", record["Depends"])
            download = apt("apt-get", "download", "rmac-session", "niri", cwd=root / "downloads")
            self.assertEqual(download.returncode, 0, download.stdout + download.stderr)
            source = apt("apt-get", "source", "--download-only", "rmac", cwd=root / "downloads")
            self.assertEqual(source.returncode, 0, source.stdout + source.stderr)
            self.assertTrue((root / "downloads/rmac_1.0.0~beta.1-38.dsc").is_file())

            # Phasing is honoured for this origin: with the phase at 10%, a
            # machine APT keeps out of the phase holds the upgrade back and one
            # included in it upgrades (APT's own switches stand in for the
            # machine-ID draw).
            (root / "status").write_text(
                "".join(
                    f"Package: {name}\nStatus: install ok installed\nPriority: optional\n"
                    f"Section: misc\nMaintainer: x <x@example.invalid>\nArchitecture: amd64\n"
                    f"Version: {version}\nDescription: {name}\n\n"
                    for name, version in (("libc6", "2.42-0ubuntu1"), ("wl-clipboard", "2.2.1-2"),
                                          ("rmac-apps", "0.9.0-1"))
                ),
                encoding="utf-8",
            )
            always = apt("apt-get", "-s", "-o", "APT::Get::Always-Include-Phased-Updates=true", "upgrade")
            self.assertEqual(always.returncode, 0, always.stderr)
            self.assertIn("Inst rmac-apps [0.9.0-1] (1.0.0~beta.1-38", always.stdout)
            never = apt("apt-get", "-s", "-o", "APT::Get::Never-Include-Phased-Updates=true", "upgrade")
            self.assertEqual(never.returncode, 0, never.stderr)
            self.assertNotIn("Inst rmac-apps", never.stdout)

    def test_a_secret_that_carries_the_primary_key_is_refused(self):
        import os
        import subprocess

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            home = root / "home"
            home.mkdir(mode=0o700)
            environment = {**os.environ, "GNUPGHOME": str(home)}
            subprocess.run(
                ["gpg", "--batch", "--passphrase", "", "--quick-generate-key", "full key", "ed25519", "sign", "1d"],
                env=environment, check=True, capture_output=True, timeout=60,
            )
            full = subprocess.run(
                ["gpg", "--batch", "--armor", "--pinentry-mode", "loopback", "--passphrase", "",
                 "--export-secret-keys"],
                env=environment, check=True, capture_output=True, timeout=60,
            ).stdout.decode("ascii")
            fingerprint = next(
                line.split(":")[9]
                for line in subprocess.run(
                    ["gpg", "--batch", "--with-colons", "--list-keys"],
                    env=environment, check=True, capture_output=True, timeout=60,
                ).stdout.decode("ascii").splitlines()
                if line.startswith("fpr:")
            )
            subprocess.run(["gpgconf", "--kill", "all"], env=environment, check=False)
            release = root / "Release"
            release.write_text("Origin: rmac\n", encoding="utf-8")
            result = subprocess.run(
                ["bash", str(self.SIGNER_SCRIPT), "--release", str(release), "--output", str(root / "InRelease")],
                env={"PATH": os.environ["PATH"], "RMAC_APT_SIGNING_SUBKEY": full,
                     "RMAC_ARCHIVE_SIGNING_FINGERPRINT": fingerprint},
                capture_output=True, text=True, timeout=120,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("offline primary key", result.stderr)
            self.assertFalse((root / "InRelease").exists())


class ArchiveKeyPinTests(unittest.TestCase):
    def setUp(self):
        self.pin = fixtures.load_script("rmac_archive_key_pin", "archive-key-pin.py")

    def write(self, root: Path, fingerprints, keyring=True) -> Path:
        apt = root / "packaging/apt"
        apt.mkdir(parents=True)
        path = apt / "archive-key.json"
        path.write_text(
            json.dumps(
                {
                    "format": 1,
                    "primary_fingerprints": fingerprints,
                    "public_keyring": "packaging/apt/archive-keyring.asc",
                }
            ),
            encoding="utf-8",
        )
        if keyring:
            (apt / "archive-keyring.asc").write_text("public", encoding="utf-8")
        return path

    def test_the_committed_pin_is_well_formed(self):
        self.pin.load()

    def test_a_pinned_key_needs_its_committed_public_keyring(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = self.write(Path(temporary), ["A" * 40], keyring=False)
            with self.assertRaisesRegex(self.pin.PinError, "missing"):
                self.pin.load(path)

    def test_fingerprints_must_be_canonical(self):
        for bad in (["a" * 40], ["A" * 39], ["B" * 40, "A" * 40], ["A" * 40] * 3):
            with tempfile.TemporaryDirectory() as temporary:
                path = self.write(Path(temporary), bad)
                with self.assertRaises(self.pin.PinError):
                    self.pin.load(path)
        with tempfile.TemporaryDirectory() as temporary:
            self.assertEqual(self.pin.load(self.write(Path(temporary), ["A" * 40])), ("A" * 40,))


if __name__ == "__main__":
    unittest.main()
