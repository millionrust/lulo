"""Structural checks for the release/rollout GitHub Actions workflows.

These do not run the workflows (that needs a real GitHub Actions runner);
they check the properties the release-pipeline brief called for: valid YAML,
every third-party action pinned by a full commit SHA, the tag trigger, and
that the workflows reference scripts that actually exist in this repository.
"""

from __future__ import annotations

import re
from pathlib import Path

import yaml


REPO_ROOT = Path(__file__).resolve().parents[1]
RELEASE = REPO_ROOT / ".github/workflows/release.yml"
ROLLOUT = REPO_ROOT / ".github/workflows/rollout.yml"
SHA_PINNED_USES = re.compile(r"^([^@]+)@([0-9a-f]{40})(?:\s+#.*)?$")


def _load(path: Path) -> dict:
    with path.open(encoding="utf-8") as source:
        return yaml.safe_load(source)


def _iter_uses(document: dict):
    for job_name, job in document["jobs"].items():
        for step in job.get("steps", ()):
            if "uses" in step:
                yield job_name, step.get("name", "<unnamed step>"), step["uses"]


def test_both_workflows_are_valid_yaml():
    for path in (RELEASE, ROLLOUT):
        document = _load(path)
        assert document["jobs"], f"{path} defines no jobs"


def test_release_triggers_on_v_tags():
    document = _load(RELEASE)
    tags = document[True]["push"]["tags"]
    assert tags == ["v*"]


def test_every_action_is_pinned_by_commit_sha():
    for path in (RELEASE, ROLLOUT):
        document = _load(path)
        for job_name, step_name, uses in _iter_uses(document):
            match = SHA_PINNED_USES.match(uses)
            assert match, (
                f"{path.name}:{job_name}:{step_name} 'uses: {uses}' is not "
                "pinned by a 40-character commit SHA"
            )


def test_referenced_local_scripts_exist():
    document = _load(RELEASE)
    text = RELEASE.read_text(encoding="utf-8")
    for reference in re.findall(r"scripts/linux/[A-Za-z0-9_.-]+\.(?:py|sh)", text):
        assert (REPO_ROOT / reference).is_file(), f"release.yml references missing {reference}"

    text = ROLLOUT.read_text(encoding="utf-8")
    for reference in re.findall(r"scripts/linux/[A-Za-z0-9_.-]+\.(?:py|sh)", text):
        assert (REPO_ROOT / reference).is_file(), f"rollout.yml references missing {reference}"


def test_apt_publishing_jobs_are_gated_and_never_generate_keys():
    for path in (RELEASE, ROLLOUT):
        text = path.read_text(encoding="utf-8")
        assert "RMAC_SOURCE_PACKAGING_READY" in text
        for forbidden in ("gpg --gen-key", "gpg --full-generate-key", "gpg --quick-generate-key"):
            assert forbidden not in text, f"{path.name} must never generate keys"


def test_apt_signing_job_uses_the_environment_gate():
    document = _load(RELEASE)
    assert document["jobs"]["apt-repository"]["environment"] == "apt-signing"
    document = _load(ROLLOUT)
    assert document["jobs"]["rollout"]["environment"] == "apt-signing"


def test_niri_packages_build_in_the_ubuntu_container_and_gate_the_release():
    document = _load(RELEASE)
    jobs = document["jobs"]
    for architecture in ("amd64", "arm64"):
        job = jobs[f"build-third-party-{architecture}"]
        assert job["container"]["image"] == "ubuntu:26.04"
        script = "\n".join(step.get("run", "") for step in job["steps"])
        assert "scripts/linux/build-niri-packages.sh" in script
        assert "--build-deps system" in script
        assert "sudo -E -u builder" in script
        assert "rustup toolchain install 1.95.0" in script
        uploads = [step for step in job["steps"] if "upload-artifact" in step.get("uses", "")]
        assert uploads[0]["with"]["name"] == f"third-party-{architecture}"
    assert jobs["build-third-party-arm64"]["if"] == jobs["build-arm64"]["if"]
    # The container installs every Build-Depends the Debian packaging names.
    install = jobs["build-third-party-amd64"]["steps"][0]["run"]
    for control in (REPO_ROOT / "packaging/third-party").glob("*/debian/control"):
        text = control.read_text(encoding="utf-8")
        block = re.search(r"(?ms)^Build-Depends:(.*?)(?=^\S)", text).group(1)
        for relation in block.split(","):
            name = relation.split("(")[0].strip()
            assert re.search(rf"^\s+{re.escape(name)}(?: \\)?$", install, re.M), (
                f"release.yml does not install {name} from {control}"
            )

    attach = jobs["attach-release"]
    assert "build-third-party-amd64" in attach["needs"]
    assert "needs.build-third-party-amd64.result == 'success'" in attach["if"]
    assemble = next(step["run"] for step in attach["steps"] if step.get("name", "").startswith("Assemble"))
    for suffix in ("*.dsc", "*.orig.tar.gz", "*.orig-*.tar.xz", "*.debian.tar.xz", "*.cdx.json"):
        assert suffix in assemble
    assert "sha256sum" in assemble


def test_rollout_schedule_and_manual_dispatch_exist():
    document = _load(ROLLOUT)
    triggers = document[True]
    assert "schedule" in triggers
    assert "workflow_dispatch" in triggers
    options = triggers["workflow_dispatch"]["inputs"]["phase"]["options"]
    assert options == ["0", "10", "25", "50", "100"]


def test_no_job_condition_reads_the_secrets_context():
    # GitHub rejects a workflow whose job-level `if` names `secrets`, which
    # would stop every release job (checksums, SBOM, provenance) from running.
    for path in (RELEASE, ROLLOUT):
        document = _load(path)
        for job_name, job in document["jobs"].items():
            condition = str(job.get("if", ""))
            assert "secrets." not in condition, (
                f"{path.name}:{job_name} job-level if reads the secrets context"
            )


def test_release_is_attached_only_after_the_dependency_gate_passes():
    job = _load(RELEASE)["jobs"]["attach-release"]
    assert "dependency-policy" in job["needs"]
    assert "needs.dependency-policy.result == 'success'" in job["if"]


def test_fetched_repository_values_never_expand_inside_run_scripts():
    for path in (RELEASE, ROLLOUT):
        document = _load(path)
        for job_name, job in document["jobs"].items():
            for step in job.get("steps", ()):
                script = step.get("run", "")
                for expression in ("steps.live.outputs", "github.event.inputs"):
                    assert "${{ " + expression not in script, (
                        f"{path.name}:{job_name}:{step.get('name')} expands "
                        f"{expression} into shell text"
                    )


def test_uploaded_asset_names_match_sha256sums_after_github_renames_tildes():
    # GitHub stores "~" in an uploaded asset's name as ".", so a Debian
    # pre-release like 0.9.0~beta.1 must be renamed before SHA256SUMS is
    # written and before upload, or the listing names files that do not exist.
    import shutil
    import subprocess
    import tempfile

    attach = _load(RELEASE)["jobs"]["attach-release"]
    steps = {step.get("name", ""): step for step in attach["steps"]}
    assemble = next(step["run"] for name, step in steps.items() if name.startswith("Assemble"))
    start = assemble.index('for path in "$bundle"/*~*; do')
    end = assemble.index("ls -l")
    snippet = assemble[start:end]
    assert "sha256sum" in snippet
    upload = next(step["run"] for name, step in steps.items() if name.startswith("Create or update"))
    assert "gh release upload \"$tag\" release-bundle/*" in upload
    provenance = next(step for name, step in steps.items() if name.startswith("Generate build provenance"))
    assert provenance["with"]["subject-path"] == "release-bundle/*"
    assert assemble.index("*~*") < assemble.index("sha256sum")

    if shutil.which("sha256sum") is None or shutil.which("bash") is None:
        return
    with tempfile.TemporaryDirectory() as temporary:
        bundle = Path(temporary)
        for name in (
            "rmac-apps_0.9.0~beta.1-38_amd64.deb",
            "rmac-session_0.9.0~beta.1-38_amd64.deb",
            "niri_26.04-0lulo1_amd64.deb",
        ):
            (bundle / name).write_bytes(name.encode())
        subprocess.run(
            ["bash", "-euo", "pipefail", "-c", f'bundle="{bundle}"\n{snippet}'],
            check=True,
            capture_output=True,
        )
        names = sorted(path.name for path in bundle.iterdir())
        assert names == [
            "SHA256SUMS",
            "niri_26.04-0lulo1_amd64.deb",
            "rmac-apps_0.9.0.beta.1-38_amd64.deb",
            "rmac-session_0.9.0.beta.1-38_amd64.deb",
        ]
        listed = sorted(
            line.split()[-1]
            for line in (bundle / "SHA256SUMS").read_text(encoding="utf-8").splitlines()
        )
        assert listed == [name for name in names if name != "SHA256SUMS"]
