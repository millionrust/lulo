#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# Clearsign one staged APT Release file into InRelease with the online
# signing SUBKEY, in a throwaway GNUPGHOME that is destroyed afterwards.
#
#   RMAC_APT_SIGNING_SUBKEY=<armored subkey-only secret export> \
#   RMAC_ARCHIVE_SIGNING_FINGERPRINT=<primary fingerprint> \
#   sign-apt-release.sh --release /abs/Release --output /abs/InRelease \
#                       [--public-keyring /abs/archive-keyring.asc]
#
# It refuses a secret that carries the offline primary key: gpg marks a
# secret key whose primary is absent with "#" in field 15 of its "sec"
# record (GnuPG doc/DETAILS). Importing the committed public keyring after
# the secret picks up the newest subkey binding, so a subkey whose expiry
# was extended offline keeps signing without a new secret.
set -euo pipefail

release=""
output=""
public_keyring=""

fail() {
  echo "sign-apt-release: $*" >&2
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --release|--output|--public-keyring)
      option=$1
      shift
      [[ $# -gt 0 ]] || fail "$option needs a value"
      case "$option" in
        --release) release=$1 ;;
        --output) output=$1 ;;
        --public-keyring) public_keyring=$1 ;;
      esac
      ;;
    *) fail "unknown argument: $1" ;;
  esac
  shift
done

[[ "$release" == /* && -f "$release" && ! -L "$release" ]] || fail "--release must be an absolute regular file"
[[ "$output" == /* && ! -e "$output" ]] || fail "--output must be an absolute path that does not exist"
[[ -n "${RMAC_APT_SIGNING_SUBKEY:-}" ]] || fail "RMAC_APT_SIGNING_SUBKEY is not set"
[[ "${RMAC_ARCHIVE_SIGNING_FINGERPRINT:-}" =~ ^([0-9A-F]{40}|[0-9A-F]{64})$ ]] \
  || fail "RMAC_ARCHIVE_SIGNING_FINGERPRINT must be an uppercase primary fingerprint"
command -v gpg >/dev/null 2>&1 || fail "gpg is required"

gnupg_home="$(mktemp -d)"
cleanup() {
  gpgconf --homedir "$gnupg_home" --kill all >/dev/null 2>&1 || true
  rm -rf "$gnupg_home"
}
trap cleanup EXIT
chmod 700 "$gnupg_home"
gpg_batch=(gpg --homedir "$gnupg_home" --batch --quiet --no-tty --pinentry-mode error)

printf '%s\n' "$RMAC_APT_SIGNING_SUBKEY" | "${gpg_batch[@]}" --import \
  || fail "the signing subkey secret could not be imported"
if [[ -n "$public_keyring" ]]; then
  "${gpg_batch[@]}" --import "$public_keyring" || fail "the public keyring could not be imported"
fi

listing="$("${gpg_batch[@]}" --with-colons --list-secret-keys "$RMAC_ARCHIVE_SIGNING_FINGERPRINT")" \
  || fail "the secret does not belong to $RMAC_ARCHIVE_SIGNING_FINGERPRINT"
primary_state="$(printf '%s\n' "$listing" | awk -F: '$1 == "sec" { print $15; exit }')"
[[ "$primary_state" == "#" ]] \
  || fail "RMAC_APT_SIGNING_SUBKEY contains the offline primary key; export the signing subkey only"
printf '%s\n' "$listing" | awk -F: '$1 == "ssb" && $12 ~ /s/ { found = 1 } END { exit !found }' \
  || fail "RMAC_APT_SIGNING_SUBKEY holds no usable signing subkey"

"${gpg_batch[@]}" --yes --local-user "$RMAC_ARCHIVE_SIGNING_FINGERPRINT" \
  --digest-algo SHA512 --clearsign --output "$output" "$release" \
  || fail "clearsigning failed (is the subkey expired?)"
echo "sign-apt-release: signed $(basename "$output")"
