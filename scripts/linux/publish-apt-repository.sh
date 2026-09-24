#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# Publish (or re-sign) the rmac APT repository from GitHub Releases.
#
# Used by release.yml's apt-repository job (--mode release: publish the
# tagged release's packages at phase 10) and rollout.yml (--mode rollout:
# step the phase, halt, or refresh the signature before Valid-Until). The
# steps, all against authoritative inputs (docs/update-trust.md
# "Stateless publication from GitHub Releases"):
#
#   1. apt-publication.py collect  -- rebuild the published repository from
#      the newest retained apt-snapshot-*.tar bundles and the verified
#      apt-inputs-<tag>.tar of every Release they name;
#   2. apt-publication.py decide   -- pick the phase, or stop;
#   3. stage-apt-snapshot.py       -- stage the new snapshot (carrying every
#      already-published version forward byte for byte; --rollout-only for
#      rollout, which may not add a single pool object);
#   4. clearsign Release with the signing subkey in a throwaway GNUPGHOME;
#   5. publish-apt-snapshot.py     -- verify with the PACKAGED keyring and
#      promote atomically onto the rebuilt repository (monotonic Date and
#      snapshot, immutable pool, retention);
#   6. upload apt-snapshot-<id>.tar to the Release (before Pages changes,
#      so the next run always sees what might be live);
#   7. write the Pages site directory.
#
# Environment: GH_TOKEN (contents: write for the upload),
# RMAC_APT_SIGNING_SUBKEY (ASCII-armored export of the signing SUBKEY only;
# the offline primary must be a stub), RMAC_ARCHIVE_SIGNING_FINGERPRINT (the
# archive's PRIMARY fingerprint, as in packaging/apt/archive-key.json).
#
# Writes "deploy=true|false" and "site=<dir>" to $GITHUB_OUTPUT when set.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
linux="$repo_root/scripts/linux"

mode=""
repository=""
work=""
tag=""
product_revision=""
requested_phase=""
allow_first=false
retain=3
valid_hours=48

usage() {
  cat >&2 <<'EOF'
usage: publish-apt-repository.sh --mode release|rollout --repository OWNER/NAME --work /abs/new/dir
                                 [--tag TAG --product-revision SHA]   (release mode)
                                 [--phase 0|10|25|50|100]              (release: override 10; rollout: manual step)
                                 [--allow-first-publication] [--retain N]
EOF
}

fail() {
  echo "publish-apt-repository: $*" >&2
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --mode|--repository|--work|--tag|--product-revision|--phase|--retain)
      option=$1
      shift
      [[ $# -gt 0 ]] || { usage; exit 2; }
      case "$option" in
        --mode) mode=$1 ;;
        --repository) repository=$1 ;;
        --work) work=$1 ;;
        --tag) tag=$1 ;;
        --product-revision) product_revision=$1 ;;
        --phase) requested_phase=$1 ;;
        --retain) retain=$1 ;;
      esac
      ;;
    --allow-first-publication) allow_first=true ;;
    -h|--help) usage; exit 0 ;;
    *) usage; exit 2 ;;
  esac
  shift
done

output() {
  if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
    printf '%s\n' "$1" >>"$GITHUB_OUTPUT"
  fi
}

[[ "$mode" == release || "$mode" == rollout ]] || { usage; exit 2; }
[[ -n "$repository" ]] || fail "--repository is required"
[[ "$work" == /* && "$work" != / ]] || fail "--work must be an absolute path"
[[ ! -e "$work" ]] || fail "$work already exists"
if [[ -n "$requested_phase" ]]; then
  [[ "$requested_phase" =~ ^(0|10|25|50|100)$ ]] || fail "--phase must be 0, 10, 25, 50, or 100"
fi
if [[ "$mode" == release ]]; then
  [[ -n "$tag" ]] || fail "--tag is required in release mode"
  [[ "$product_revision" =~ ^[0-9a-f]{40}$ ]] || fail "--product-revision must be a 40-hex commit"
else
  [[ -z "$tag" && -z "$product_revision" ]] || fail "rollout mode republishes the live release; do not pass --tag"
  [[ "$allow_first" == false ]] || fail "rollout mode never starts a repository"
fi
[[ -n "${RMAC_APT_SIGNING_SUBKEY:-}" ]] || fail "RMAC_APT_SIGNING_SUBKEY is not set (apt-signing / apt-refresh environment secret)"
[[ "${RMAC_ARCHIVE_SIGNING_FINGERPRINT:-}" =~ ^([0-9A-F]{40}|[0-9A-F]{64})$ ]] \
  || fail "RMAC_ARCHIVE_SIGNING_FINGERPRINT must be the archive's uppercase primary fingerprint"
python3 "$linux/archive-key-pin.py" --require "$RMAC_ARCHIVE_SIGNING_FINGERPRINT" \
  || fail "RMAC_ARCHIVE_SIGNING_FINGERPRINT is not listed in packaging/apt/archive-key.json"
for tool in gh gpg gpgv python3 tar; do
  command -v "$tool" >/dev/null 2>&1 || fail "'$tool' is required"
done

mkdir -p "$work"

# 1. Rebuild the published state from the Releases.
collect_args=(collect --repository "$repository" --work "$work/state" --retain "$retain")
if [[ "$mode" == release ]]; then
  collect_args+=(--target-tag "$tag")
fi
if [[ "$allow_first" == true ]]; then
  collect_args+=(--allow-first-publication)
fi
python3 "$linux/apt-publication.py" "${collect_args[@]}" >"$work/summary.json"

# 2. Decide.
decide_args=(decide --summary "$work/summary.json" --mode "$mode")
if [[ -n "$requested_phase" ]]; then
  decide_args+=(--requested-phase "$requested_phase")
fi
python3 "$linux/apt-publication.py" "${decide_args[@]}" >"$work/decision.json"
read_json() {
  python3 -c 'import json, sys
value = json.load(open(sys.argv[1]))
for key in sys.argv[2].split("."):
    value = value[key] if isinstance(value, dict) else None
print("" if value is None else value)' "$1" "$2"
}
action="$(read_json "$work/decision.json" action)"
reason="$(read_json "$work/decision.json" reason)"
echo "publish-apt-repository: decision: $action ($reason)"
if [[ "$action" != publish ]]; then
  output "deploy=false"
  exit 0
fi
phase="$(read_json "$work/decision.json" phase)"
target_tag="$(read_json "$work/summary.json" target_tag)"
inputs="$(read_json "$work/summary.json" inputs)"
keyring="$(read_json "$work/summary.json" keyring)"
previous="$(read_json "$work/summary.json" previous_repository)"
previous_sidecar="$(read_json "$work/summary.json" previous_sidecar)"
if [[ "$mode" == rollout ]]; then
  product_revision="$(read_json "$work/summary.json" latest.product_revision)"
fi

# 3. Stage. The release job only reaches this point after dependency-policy
# (licences and advisories), the two byte-identical package assemblies, the
# offline rebuild of the rmac source package, and attach-release's checksums
# and provenance all passed; a rollout step republishes exactly those bytes.
stage_args=(
  --inputs "$inputs"
  --output "$work/staged"
  --sidecar-output "$work/rmac-snapshot.json"
  --phase "$phase"
  --valid-hours "$valid_hours"
  --signer-fingerprint "$RMAC_ARCHIVE_SIGNING_FINGERPRINT"
  --product-revision "$product_revision"
  --release-tag "$target_tag"
  --binary-packages-verified
  --licenses-verified
  --reproducibility-verified
  --source-offer-verified
)
if [[ -n "$previous" ]]; then
  stage_args+=(--previous-repository "$previous" --previous-sidecar "$previous_sidecar")
fi
if [[ "$mode" == rollout ]]; then
  stage_args+=(--rollout-only)
fi
python3 "$linux/stage-apt-snapshot.py" "${stage_args[@]}"

# 4. Sign with the online subkey only, in a throwaway keyring (the script
# refuses a secret that carries the offline primary).
bash "$linux/sign-apt-release.sh" \
  --release "$work/staged/dists/resolute/Release" \
  --output "$work/staged/dists/resolute/InRelease" \
  --public-keyring "$repo_root/packaging/apt/archive-keyring.asc"
rm -f "$work/staged/dists/resolute/Release"

# 5. Verify with the packaged keyring and promote onto the rebuilt repository.
repository_dir="$work/repository"
if [[ -n "$previous" ]]; then
  mv "$previous" "$repository_dir"
else
  mkdir "$repository_dir"
fi
python3 "$linux/publish-apt-snapshot.py" \
  --staging-dir "$work/staged" \
  --repository-dir "$repository_dir" \
  --keyring "$keyring" \
  --retain "$retain"

# 6. Record the publication on its Release before anything becomes visible.
bundle="$(python3 "$linux/apt-publication.py" bundle \
  --repository-dir "$repository_dir" \
  --sidecar "$work/rmac-snapshot.json" \
  --output-dir "$work/bundle")"
gh release upload "$target_tag" "$bundle" --repo "$repository"

# 7. The Pages site: the repository plus the bootstrap files install.sh uses.
site="$work/site"
mkdir "$site"
cp -R "$repository_dir/dists" "$repository_dir/pool" "$site/"
cp "$repo_root/scripts/linux/install.sh" "$site/install.sh"
cp "$repo_root/scripts/linux/uninstall.sh" "$site/uninstall.sh"
keyring_deb="$(find "$inputs/keyring" -maxdepth 1 -name 'rmac-archive-keyring_*_all.deb' | head -n 1)"
[[ -n "$keyring_deb" ]] || fail "the release's keyring package is missing"
cp "$keyring_deb" "$site/rmac-archive-keyring-latest.deb"
cp "$repo_root/packaging/apt/archive-keyring.asc" "$site/archive-keyring.asc"
snapshot="$(read_json "$work/rmac-snapshot.json" snapshot)"
echo "publish-apt-repository: $(basename "$bundle") published from $target_tag at phase $phase"
output "deploy=true"
output "site=$site"
output "snapshot=$snapshot"
