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
    for path in sorted((REPO_ROOT / ".github/workflows").glob("*.yml")):
        document = _load(path)
        for job_name, step_name, uses in _iter_uses(document):
            match = SHA_PINNED_USES.match(uses)
            assert match, (
                f"{path.name}:{job_name}:{step_name} 'uses: {uses}' is not "
                "pinned by a 40-character commit SHA"
            )


def test_one_action_version_is_pinned_to_one_commit_everywhere():
    # A second SHA for the same "# vX.Y.Z" label means one of them is not the
    # tag's commit (for example an annotated tag object's own SHA).
    seen: dict[tuple[str, str], str] = {}
    for path in sorted((REPO_ROOT / ".github/workflows").glob("*.yml")):
        for line in path.read_text(encoding="utf-8").splitlines():
            match = re.search(r"uses:\s*([^@\s]+)@([0-9a-f]{40})\s+#\s*(\S+)", line)
            if not match:
                continue
            action, sha, label = match.groups()
            previous = seen.setdefault((action, label), sha)
            assert previous == sha, f"{action} {label} is pinned to {previous} and {sha}"


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
        assert "RMAC_SOURCE_PACKAGING_READY" not in text
        assert "vars.RMAC_APT_PUBLISHING_ENABLED == 'true'" in text
        for forbidden in ("gpg --gen-key", "gpg --full-generate-key", "gpg --quick-generate-key"):
            assert forbidden not in text, f"{path.name} must never generate keys"


def test_apt_signing_job_uses_the_environment_gate():
    # Publishing new packages waits for the apt-signing reviewer; the
    # unattended rollout/refresh signer is a separate environment and can
    # only republish already-published pool objects (--rollout-only).
    release = _load(RELEASE)["jobs"]["apt-repository"]
    assert release["environment"] == "apt-signing"
    rollout = _load(ROLLOUT)["jobs"]["rollout"]
    assert rollout["environment"] == "apt-refresh"
    assert "--mode rollout" in "\n".join(step.get("run", "") for step in rollout["steps"])
    publish = Path(REPO_ROOT / "scripts/linux/publish-apt-repository.sh").read_text(encoding="utf-8")
    assert 'stage_args+=(--rollout-only)' in publish


def test_both_publishers_share_one_serialized_concurrency_group():
    release = _load(RELEASE)["jobs"]["apt-repository"]
    rollout = _load(ROLLOUT)["jobs"]["rollout"]
    for job in (release, rollout):
        assert job["concurrency"] == {"group": "rmac-apt-publication", "cancel-in-progress": False}


def test_apt_repository_waits_for_the_release_and_the_source_rebuild():
    job = _load(RELEASE)["jobs"]["apt-repository"]
    for needed in ("attach-release", "rmac-source-rebuild", "keyring", "dependency-policy"):
        assert needed in job["needs"]
        assert f"needs.{needed}.result == 'success'" in job["if"]
    script = "\n".join(step.get("run", "") for step in job["steps"])
    assert "scripts/linux/publish-apt-repository.sh" in script
    assert "--mode release" in script
    # No wget mirror of the live site: state comes from GitHub Releases (SR-12).
    for path in (RELEASE, ROLLOUT):
        assert "wget" not in path.read_text(encoding="utf-8")
    permissions = job["permissions"]
    assert permissions["contents"] == "write"
    assert permissions["attestations"] == "read"


def test_pages_deploys_run_in_the_pages_environment_and_never_go_backwards():
    for path, publisher_job, deploy_job in (
        (RELEASE, "apt-repository", "apt-pages"),
        (ROLLOUT, "rollout", "pages"),
    ):
        jobs = _load(path)["jobs"]
        publish = jobs[publisher_job]
        assert set(publish["outputs"]) == {"deploy", "snapshot"}
        assert "pages" not in publish["permissions"]
        assert "id-token" not in publish["permissions"]
        uses = [step.get("uses", "") for step in publish["steps"]]
        assert any("upload-pages-artifact" in value for value in uses)
        assert not any("deploy-pages" in value for value in uses)
        deploy = jobs[deploy_job]
        assert deploy["needs"] == [publisher_job]
        assert deploy["if"] == f"needs.{publisher_job}.outputs.deploy == 'true'"
        assert deploy["environment"]["name"] == "github-pages"
        assert deploy["permissions"] == {"contents": "read", "id-token": "write", "pages": "write"}
        assert deploy["concurrency"] == {"group": "rmac-apt-pages", "cancel-in-progress": False}
        fresh = next(step for step in deploy["steps"] if step.get("id") == "fresh")
        assert "apt-publication.py is-newest" in fresh["run"]
        final = deploy["steps"][-1]
        assert "deploy-pages" in final["uses"]
        assert final["if"] == "steps.fresh.outputs.deploy == 'true'"


def test_release_attaches_the_source_package_and_the_apt_inputs():
    jobs = _load(RELEASE)["jobs"]
    source = jobs["rmac-source"]
    script = "\n".join(step.get("run", "") for step in source["steps"])
    assert "build-rmac-source-package.sh source" in script
    assert '--revision "$GITHUB_SHA"' in script
    rebuild = jobs["rmac-source-rebuild"]
    assert rebuild["needs"] == ["rmac-source"]
    assert "build-rmac-source-package.sh rebuild" in "\n".join(
        step.get("run", "") for step in rebuild["steps"]
    )
    attach = jobs["attach-release"]
    assert "rmac-source" in attach["needs"]
    assert "needs.rmac-source.result == 'success'" in attach["if"]
    steps = {step.get("name", ""): step for step in attach["steps"]}
    assemble = next(step["run"] for name, step in steps.items() if name.startswith("Assemble"))
    assert 'apt-inputs-${GITHUB_REF_NAME}.tar' in assemble
    assert assemble.index("apt-inputs-") < assemble.index("sha256sum")
    seal = next(step["run"] for name, step in steps.items() if name.startswith("Refuse to change"))
    assert "apt-snapshot-" in seal
    names = list(steps)
    assert names.index(next(n for n in names if n.startswith("Refuse to change"))) < names.index(
        next(n for n in names if n.startswith("Create or update"))
    )


def test_rmac_source_rebuild_installs_every_build_dependency():
    install = _load(RELEASE)["jobs"]["rmac-source-rebuild"]["steps"][0]["run"]
    control = (REPO_ROOT / "packaging/rmac-source/debian/control").read_text(encoding="utf-8")
    block = re.search(r"(?ms)^Build-Depends:(.*?)(?=^\S)", control).group(1)
    for relation in block.split(","):
        name = relation.split("(")[0].strip()
        if not name:
            continue
        assert re.search(rf"^\s+{re.escape(name)}(?: \\)?$", install, re.M), (
            f"rmac-source-rebuild does not install {name}"
        )


def test_keyring_job_packages_the_committed_public_keyring_without_secrets():
    job = _load(RELEASE)["jobs"]["keyring"]
    script = "\n".join(step.get("run", "") for step in job["steps"])
    assert "packaging/apt/archive-keyring.asc" in script
    assert "archive-key-pin.py" in script
    assert "secrets." not in RELEASE.read_text(encoding="utf-8").split("  keyring:")[1].split("  apt-repository:")[0]


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
