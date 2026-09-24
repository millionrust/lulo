#!/usr/bin/env bash
# Create the Lulo OS (rmac) APT archive signing key, offline, on the owner's
# own machine (macOS or Linux).
#
# What it makes (docs/update-trust.md "Signing and rotation"):
#   * an offline ed25519 PRIMARY key that can only certify (no expiry);
#   * one ed25519 SIGNING SUBKEY with a bounded lifetime, which is the only
#     secret CI ever receives (as RMAC_APT_SIGNING_SUBKEY, passphrase removed,
#     with the primary present only as a gnu-dummy stub);
#   * the public keyring the rmac-archive-keyring package ships;
#   * an encrypted backup of the primary key, its revocation certificate and
#     restore/renew instructions.
#
# It never uploads anything and never touches your everyday GnuPG home: every
# gpg call runs with --homedir pointing at a fresh temporary directory that is
# deleted (after stopping its gpg-agent) when the script exits. Passphrases
# reach gpg only through a pipe on file descriptor 0 (--passphrase-fd 0),
# never through a command line or an exported environment variable.
#
# Usage:
#   scripts/release/create-archive-key.sh [options]
#
# Options:
#   --email ADDRESS           e-mail address for the key's user ID (prompted
#                             for when omitted; there is no default)
#   --name NAME               user ID name (default "Lulo OS Archive Signing Key")
#   --subkey-lifetime N[dwmy] signing-subkey lifetime (default 1y)
#   --output-dir DIR          where to write the results; must not exist
#                             (default ./lulo-archive-key-<UTC date>)
#   --write-repo DIR          repository to update with the new public key
#                             (default: the checkout containing this script)
#   --no-write-repo           do not update any repository files
#   --replace-existing-key    allow replacing a fingerprint that is already
#                             pinned in packaging/apt/archive-key.json
#   -h, --help                show this help
#
# Test-suite only (scripts/test_create_archive_key.py); do not use for the
# real key, whose passphrases must never be written to a file:
#   --non-interactive                 no prompts and no confirmation
#   --primary-passphrase-file FILE    first line = primary-key passphrase
#   --backup-passphrase-file FILE     first line = backup passphrase
#
# Requires GnuPG >= 2.2 (gpg, gpgv, gpgconf); on macOS: brew install gnupg.
set -euo pipefail
umask 077

readonly MIN_PASSPHRASE_LENGTH=16
readonly DEFAULT_NAME="Lulo OS Archive Signing Key"
readonly DEFAULT_LIFETIME="1y"
readonly BACKUP_DIR_NAME="lulo-archive-key-backup"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

uid_name="$DEFAULT_NAME"
uid_email=""
subkey_lifetime=""
output_dir=""
write_repo_mode="default"   # default | explicit | none
repo_dir=""
replace_existing=0
non_interactive=0
primary_passphrase_file=""
backup_passphrase_file=""

work_dir=""
output_created=0
outputs_complete=0
gpg_log=""

say() { printf '%s\n' "$*" >&2; }

die() {
    say ""
    say "ERROR: $*"
    if [ -n "$gpg_log" ] && [ -s "$gpg_log" ]; then
        say ""
        say "Last GnuPG messages:"
        tail -n 20 "$gpg_log" | sed 's/^/    /' >&2
    fi
    exit 1
}

usage() {
    sed -n '2,/^set -euo pipefail$/p' "${BASH_SOURCE[0]}" | sed -e '$d' -e 's/^# \{0,1\}//'
}

need_value() {
    [ "$#" -ge 2 ] && [ -n "$2" ] || die "$1 needs a value"
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --email) need_value "$@"; uid_email="$2"; shift 2 ;;
        --name) need_value "$@"; uid_name="$2"; shift 2 ;;
        --subkey-lifetime) need_value "$@"; subkey_lifetime="$2"; shift 2 ;;
        --output-dir) need_value "$@"; output_dir="$2"; shift 2 ;;
        --write-repo) need_value "$@"; write_repo_mode="explicit"; repo_dir="$2"; shift 2 ;;
        --no-write-repo) write_repo_mode="none"; repo_dir=""; shift ;;
        --replace-existing-key) replace_existing=1; shift ;;
        --non-interactive) non_interactive=1; shift ;;
        --primary-passphrase-file) need_value "$@"; primary_passphrase_file="$2"; shift 2 ;;
        --backup-passphrase-file) need_value "$@"; backup_passphrase_file="$2"; shift 2 ;;
        -h | --help) usage; exit 0 ;;
        *) die "unknown option '$1' (see --help)" ;;
    esac
done

# --- temporary state and cleanup --------------------------------------------

stop_agent() {
    if [ -d "$1" ]; then
        gpgconf --homedir "$1" --kill all >/dev/null 2>&1 || true
        gpgconf --homedir "$1" --remove-socketdir >/dev/null 2>&1 || true
    fi
}

cleanup() {
    status=$?
    # Clean up even if stderr is gone (a closed pipe must not stop rm -rf).
    set +e
    trap '' PIPE INT TERM HUP
    if [ -t 0 ]; then
        stty echo 2>/dev/null || true
    fi
    if [ -n "$work_dir" ] && [ -d "$work_dir" ]; then
        for home in "$work_dir"/home-*; do
            stop_agent "$home"
        done
        rm -rf "$work_dir"
    fi
    if [ "$status" -ne 0 ] && [ "$output_created" -eq 1 ] && [ "$outputs_complete" -eq 0 ]; then
        rm -rf "$output_dir"
        say "Removed the incomplete output directory $output_dir."
    fi
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT TERM HUP
trap 'exit 141' PIPE

# Every gpg call names its (temporary) home explicitly.
gpg_at() {
    local home=$1
    shift
    gpg --homedir "$home" --batch --no-tty --no-options "$@" 2>>"$gpg_log"
}

# The passphrase travels through a pipe from the printf builtin: it is never
# an argument of any process and never exported.
gpg_with_passphrase() {
    local home=$1 passphrase=$2
    shift 2
    printf '%s\n' "$passphrase" \
        | gpg --homedir "$home" --batch --no-tty --no-options \
            --pinentry-mode loopback --passphrase-fd 0 "$@" 2>>"$gpg_log"
}

new_home() {
    local home="$work_dir/home-$1"
    mkdir -m 700 "$home"
    printf '%s\n' "$home"
}

# --- preflight --------------------------------------------------------------

for tool in gpg gpgv gpgconf tar awk; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        if [ "$(uname -s)" = "Darwin" ]; then
            die "'$tool' was not found; install GnuPG with: brew install gnupg"
        fi
        die "'$tool' was not found; install GnuPG 2.2 or newer (e.g. sudo apt install gnupg gpgv)"
    fi
done

# A short path keeps gpg-agent's socket paths under the Unix socket length
# limit (macOS's per-user temporary directory is long).
work_dir="$(mktemp -d /tmp/lulo-archive-key.XXXXXXXX)"
chmod 700 "$work_dir"
gpg_log="$work_dir/gpg.log"
: >"$gpg_log"

main_home="$(new_home main)"
gpg_version="$(gpg_at "$main_home" --with-colons --list-config version | awk -F: '$1 == "cfg" && $2 == "version" { print $3; exit }')"
gpg_major="${gpg_version%%.*}"
gpg_rest="${gpg_version#*.}"
gpg_minor="${gpg_rest%%.*}"
case "$gpg_major.$gpg_minor" in
    *[!0-9.]* | .* | *.) die "could not determine the GnuPG version (got '$gpg_version')" ;;
esac
if [ "$gpg_major" -lt 2 ] || { [ "$gpg_major" -eq 2 ] && [ "$gpg_minor" -lt 2 ]; }; then
    die "GnuPG $gpg_version is too old; 2.2 or newer is required (macOS: brew install gnupg)"
fi

if [ "$non_interactive" -eq 1 ]; then
    [ -n "$uid_email" ] || die "--non-interactive needs --email"
    [ -n "$primary_passphrase_file" ] && [ -n "$backup_passphrase_file" ] \
        || die "--non-interactive needs --primary-passphrase-file and --backup-passphrase-file"
else
    [ -z "$primary_passphrase_file$backup_passphrase_file" ] \
        || die "passphrase files are for the test suite only (--non-interactive)"
    [ -t 0 ] || die "run this from an interactive terminal (standard input is not a TTY)"
fi

# --- repository to update ----------------------------------------------------

archive_key_json_rel="packaging/apt/archive-key.json"
archive_keyring_rel="packaging/apt/archive-keyring.asc"
install_sh_rel="scripts/linux/install.sh"
fingerprint_line_re='^RMAC_ARCHIVE_KEYRING_FINGERPRINT="[^"]*"$'

if [ "$write_repo_mode" = "default" ]; then
    command -v git >/dev/null 2>&1 \
        || die "git was not found to locate the repository; pass --write-repo DIR or --no-write-repo"
    repo_dir="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel 2>/dev/null)" \
        || die "this script is not inside a git checkout; pass --write-repo DIR or --no-write-repo"
fi

if [ "$write_repo_mode" != "none" ]; then
    command -v python3 >/dev/null 2>&1 || die "python3 is required to update $archive_key_json_rel"
    [ -d "$repo_dir" ] || die "repository directory $repo_dir does not exist"
    repo_dir="$(cd "$repo_dir" && pwd)"
    [ -f "$repo_dir/$archive_key_json_rel" ] || die "$repo_dir/$archive_key_json_rel is missing"
    [ -f "$repo_dir/$install_sh_rel" ] || die "$repo_dir/$install_sh_rel is missing"
    line_count="$(grep -c "$fingerprint_line_re" "$repo_dir/$install_sh_rel" || true)"
    [ "$line_count" = "1" ] \
        || die "$install_sh_rel must contain exactly one RMAC_ARCHIVE_KEYRING_FINGERPRINT=\"...\" line (found $line_count)"
    pinned="$(python3 -c '
import json, sys
with open(sys.argv[1], encoding="utf-8") as handle:
    data = json.load(handle)
values = data.get("primary_fingerprints") if isinstance(data, dict) else None
if not isinstance(data, dict) or data.get("format") != 1 or not isinstance(values, list):
    sys.exit("unsupported archive-key.json (expected format 1 with a primary_fingerprints list)")
print(" ".join(str(value) for value in values))
' "$repo_dir/$archive_key_json_rel")" || die "could not read $archive_key_json_rel"
    if [ -n "$pinned" ] && [ "$replace_existing" -eq 0 ]; then
        die "$archive_key_json_rel already pins $pinned.
A different archive key is a ROTATION, not a first-time setup: clients that
trust the published key would reject anything signed by a new one. Follow
docs/update-trust.md \"Signing and rotation\" (overlap-first: ship both public
keys in a keyring package signed by the current key before switching).
Only if that key was never published may you rerun with --replace-existing-key."
    fi
fi

# --- questions ---------------------------------------------------------------

ask() {
    # ask VAR "Prompt" DEFAULT
    local answer
    if [ -n "$3" ]; then
        read -r -p "$2 [$3]: " answer || die "no answer"
        answer="${answer:-$3}"
    else
        read -r -p "$2: " answer || die "no answer"
    fi
    printf -v "$1" '%s' "$answer"
}

valid_email() {
    printf '%s' "$1" | grep -Eq '^[^@<>()[:space:]"]+@[^@<>()[:space:]"]+\.[^@<>()[:space:]"]+$'
}

if [ "$non_interactive" -eq 0 ]; then
    say "Lulo OS archive signing key -- offline key ceremony"
    say "Nothing is uploaded. All GnuPG work happens in a temporary home under /tmp."
    say ""
    ask uid_name "Key user ID name" "$uid_name"
    while ! valid_email "$uid_email"; do
        [ -z "$uid_email" ] || say "  '$uid_email' does not look like an e-mail address."
        ask uid_email "E-mail address for the key (e.g. the project's security contact)" ""
    done
    ask subkey_lifetime "Signing-subkey lifetime (NNd, NNw, NNm or NNy)" "${subkey_lifetime:-$DEFAULT_LIFETIME}"
    ask output_dir "Output directory (must not exist yet)" \
        "${output_dir:-./lulo-archive-key-$(date -u +%Y-%m-%d)}"
fi

subkey_lifetime="${subkey_lifetime:-$DEFAULT_LIFETIME}"
output_dir="${output_dir:-./lulo-archive-key-$(date -u +%Y-%m-%d)}"

valid_email "$uid_email" || die "'$uid_email' is not a valid e-mail address"
case "$uid_name" in
    "" | *"<"* | *">"* | *"("* | *")"* | *"@"*) die "the key name must be plain text without <, >, (, ) or @" ;;
esac
printf '%s' "$subkey_lifetime" | grep -Eq '^[1-9][0-9]{0,3}[dwmy]$' \
    || die "subkey lifetime must look like 90d, 26w, 12m or 1y (got '$subkey_lifetime')"
[ ! -e "$output_dir" ] || die "output directory $output_dir already exists; choose a new one"
output_parent="$(dirname "$output_dir")"
[ -d "$output_parent" ] || die "the parent directory $output_parent does not exist"
output_dir="$(cd "$output_parent" && pwd)/$(basename "$output_dir")"
uid="$uid_name <$uid_email>"

# --- passphrases -------------------------------------------------------------

read_secret_twice() {
    # read_secret_twice VAR "what"
    local first second
    while :; do
        read -r -s -p "Enter the $2: " first || die "no passphrase"
        printf '\n' >&2
        if [ "${#first}" -lt "$MIN_PASSPHRASE_LENGTH" ]; then
            say "  Too short: use at least $MIN_PASSPHRASE_LENGTH characters (a long random passphrase from a password manager is best)."
            continue
        fi
        read -r -s -p "Repeat the $2: " second || die "no passphrase"
        printf '\n' >&2
        if [ "$first" != "$second" ]; then
            say "  The two entries differ; try again."
            continue
        fi
        break
    done
    printf -v "$1" '%s' "$first"
}

read_secret_file() {
    local line=""
    [ -f "$2" ] || die "passphrase file $2 does not exist"
    IFS= read -r line <"$2" || [ -n "$line" ] || die "passphrase file $2 is empty"
    printf -v "$1" '%s' "$line"
}

primary_passphrase=""
backup_passphrase=""
if [ "$non_interactive" -eq 1 ]; then
    read_secret_file primary_passphrase "$primary_passphrase_file"
    read_secret_file backup_passphrase "$backup_passphrase_file"
else
    say ""
    say "Two DIFFERENT passphrases are needed:"
    say "  1. the primary-key passphrase protects the offline primary key itself;"
    say "  2. the backup passphrase encrypts the backup archive that holds it."
    say "They must differ so that the backup file plus one leaked passphrase is"
    say "still not enough to use the primary key. Store them separately."
    say ""
    read_secret_twice primary_passphrase "primary-key passphrase"
    while :; do
        read_secret_twice backup_passphrase "backup-archive passphrase"
        [ "$backup_passphrase" != "$primary_passphrase" ] && break
        say "  The backup passphrase must differ from the primary-key passphrase."
    done
fi
[ "${#primary_passphrase}" -ge "$MIN_PASSPHRASE_LENGTH" ] \
    || die "the primary-key passphrase must have at least $MIN_PASSPHRASE_LENGTH characters"
[ "${#backup_passphrase}" -ge "$MIN_PASSPHRASE_LENGTH" ] \
    || die "the backup passphrase must have at least $MIN_PASSPHRASE_LENGTH characters"
[ "$backup_passphrase" != "$primary_passphrase" ] \
    || die "the backup passphrase must differ from the primary-key passphrase"

# --- confirmation ------------------------------------------------------------

say ""
say "About to create:"
say "  user ID:          $uid"
say "  primary key:      ed25519, certify only, no expiry (stays offline)"
say "  signing subkey:   ed25519, sign only, expires in $subkey_lifetime"
say "  output directory: $output_dir"
if [ "$write_repo_mode" = "none" ]; then
    say "  repository:       not updated (--no-write-repo)"
else
    say "  repository:       $repo_dir ($archive_keyring_rel, $archive_key_json_rel, $install_sh_rel)"
fi
say "  GnuPG:            $gpg_version (temporary home, removed on exit)"
if [ "$non_interactive" -eq 0 ]; then
    say ""
    read -r -p "Type 'create' to continue: " confirmation || die "cancelled"
    [ "$confirmation" = "create" ] || die "cancelled; nothing was created"
fi

mkdir -m 700 "$output_dir"
output_created=1

# --- key generation ----------------------------------------------------------

say ""
say "Generating the primary key..."
gpg_with_passphrase "$main_home" "$primary_passphrase" \
    --cert-digest-algo SHA512 \
    --quick-generate-key "$uid" ed25519 cert never >/dev/null \
    || die "gpg could not generate the primary key"

# Colon listings (doc/DETAILS in GnuPG): field 1 record type, 2 validity,
# 4 algorithm (22 = EdDSA), 7 expiry (empty = never), 10 fingerprint (fpr),
# 12 capabilities (lower case = this key's own usage), 15 secret-key status
# on sec/ssb ('#' = stub without secret material, '+' = secret present).
listing="$(gpg_at "$main_home" --with-colons --with-keygrip --list-secret-keys)"
[ "$(printf '%s\n' "$listing" | grep -c '^sec:' || true)" = "1" ] \
    || die "expected exactly one primary key in the temporary home"
primary_fpr="$(printf '%s\n' "$listing" | awk -F: '$1 == "sec" { want = 1; next } want && $1 == "fpr" { print $10; exit }')"
printf '%s' "$primary_fpr" | grep -Eq '^[0-9A-F]{40}$' \
    || die "the new primary fingerprint '$primary_fpr' is not a v4 (40-hex) fingerprint; APT's verifier needs an OpenPGP v4 key"

say "Adding the signing subkey (expires in $subkey_lifetime)..."
gpg_with_passphrase "$main_home" "$primary_passphrase" \
    --cert-digest-algo SHA512 \
    --quick-add-key "$primary_fpr" ed25519 sign "$subkey_lifetime" >/dev/null \
    || die "gpg could not add the signing subkey"

public_listing="$(gpg_at "$main_home" --with-colons --list-keys "$primary_fpr")"
primary_record="$(printf '%s\n' "$public_listing" | awk -F: '$1 == "pub"')"
[ "$(printf '%s\n' "$primary_record" | grep -c '^pub:' || true)" = "1" ] || die "expected one pub record"
[ "$(printf '%s\n' "$primary_record" | cut -d: -f4)" = "22" ] || die "the primary key is not EdDSA"
[ -z "$(printf '%s\n' "$primary_record" | cut -d: -f7)" ] || die "the primary key unexpectedly has an expiry date"
primary_usage="$(printf '%s\n' "$primary_record" | cut -d: -f12 | tr -cd 'a-z')"
[ "$primary_usage" = "c" ] || die "the primary key's own usage is '$primary_usage', expected certify only ('c')"

sub_records="$(printf '%s\n' "$public_listing" | awk -F: '$1 == "sub"')"
[ "$(printf '%s\n' "$sub_records" | grep -c '^sub:' || true)" = "1" ] || die "expected exactly one subkey"
[ "$(printf '%s\n' "$sub_records" | cut -d: -f4)" = "22" ] || die "the subkey is not EdDSA"
[ "$(printf '%s\n' "$sub_records" | cut -d: -f12)" = "s" ] || die "the subkey is not sign-only"
subkey_expiry="$(printf '%s\n' "$sub_records" | cut -d: -f7)"
printf '%s' "$subkey_expiry" | grep -Eq '^[0-9]+$' || die "the signing subkey has no expiry date"
subkey_fpr="$(printf '%s\n' "$public_listing" | awk -F: '$1 == "sub" { want = 1; next } want && $1 == "fpr" { print $10; exit }')"
printf '%s' "$subkey_fpr" | grep -Eq '^[0-9A-F]{40}$' || die "could not read the subkey fingerprint"

listing="$(gpg_at "$main_home" --with-colons --with-keygrip --list-secret-keys)"
primary_grip="$(printf '%s\n' "$listing" | awk -F: '$1 == "sec" { want = 1; next } want && $1 == "grp" { print $10; exit }')"
subkey_grip="$(printf '%s\n' "$listing" | awk -F: '$1 == "ssb" { want = 1; next } want && $1 == "grp" { print $10; exit }')"
[ -n "$primary_grip" ] && [ -n "$subkey_grip" ] || die "could not read the keygrips"

subkey_expiry_date="$(date -u -d "@$subkey_expiry" +%Y-%m-%d 2>/dev/null || date -u -r "$subkey_expiry" +%Y-%m-%d)"
revocation_cert="$main_home/openpgp-revocs.d/$primary_fpr.rev"
[ -s "$revocation_cert" ] || die "gpg did not write the revocation certificate"

# --- public keyring ----------------------------------------------------------

say "Exporting the public keyring..."
gpg_at "$main_home" --armor --export-options export-minimal --export "$primary_fpr" \
    >"$output_dir/archive-keyring.asc" || die "could not export the armored public keyring"
gpg_at "$main_home" --export-options export-minimal --export "$primary_fpr" \
    >"$output_dir/rmac-archive-keyring.gpg" || die "could not export the binary public keyring"
printf '%s\n' "$primary_fpr" >"$output_dir/primary-fingerprint.txt"

shown="$(gpg_at "$main_home" --with-colons --import-options show-only --dry-run --import "$output_dir/archive-keyring.asc")"
! printf '%s\n' "$shown" | grep -Eq '^(sec|ssb):' || die "the public keyring export contains secret key material"
[ "$(printf '%s\n' "$shown" | grep -c '^pub:' || true)" = "1" ] || die "the public keyring must hold exactly one primary key"

# --- CI signing-subkey export --------------------------------------------------

say "Exporting the signing subkey for CI (passphrase removed, primary as a stub)..."
gpg_with_passphrase "$main_home" "$primary_passphrase" \
    --armor --export-secret-subkeys "$subkey_fpr!" >"$work_dir/subkey-protected.asc" \
    || die "could not export the signing subkey"

# GnuPG 2.1+ no longer implements --export-options export-reset-subkey-passwd,
# so the passphrase is removed in a second temporary home that only ever holds
# the subkey: --passwd asks for the old passphrase, then the new (empty) one.
# gpg exits non-zero there because the primary is a stub ("No secret key");
# the SUCCESS status and the no-pinentry signing check below are the proof.
strip_home="$(new_home strip)"
gpg_at "$strip_home" --import "$work_dir/subkey-protected.asc" || die "could not import the subkey export"
printf '%s\n\n' "$primary_passphrase" \
    | gpg --homedir "$strip_home" --no-tty --no-options --pinentry-mode loopback \
        --command-fd 0 --status-file "$work_dir/passwd.status" \
        --passwd "$primary_fpr" 2>>"$gpg_log" || true
grep -q '^\[GNUPG:\] SUCCESS keyedit.passwd' "$work_dir/passwd.status" \
    || die "could not remove the passphrase from the signing subkey"
rm -f "$work_dir/subkey-protected.asc"
gpg_at "$strip_home" --pinentry-mode error --armor --export-secret-subkeys "$subkey_fpr!" \
    >"$output_dir/RMAC_APT_SIGNING_SUBKEY.asc" || die "could not export the unprotected signing subkey"
stop_agent "$strip_home"

say "Checking the CI export in a separate temporary home..."
check_home="$(new_home check)"
gpg_at "$check_home" --import "$output_dir/RMAC_APT_SIGNING_SUBKEY.asc" || die "the CI export does not import"
check="$(gpg_at "$check_home" --with-colons --with-keygrip --list-secret-keys)"
[ "$(printf '%s\n' "$check" | grep -c '^sec:' || true)" = "1" ] || die "the CI export must hold exactly one primary"
[ "$(printf '%s\n' "$check" | grep -c '^ssb:' || true)" = "1" ] || die "the CI export must hold exactly one subkey"
check_primary_fpr="$(printf '%s\n' "$check" | awk -F: '$1 == "sec" { want = 1; next } want && $1 == "fpr" { print $10; exit }')"
check_sub_fpr="$(printf '%s\n' "$check" | awk -F: '$1 == "ssb" { want = 1; next } want && $1 == "fpr" { print $10; exit }')"
[ "$check_primary_fpr" = "$primary_fpr" ] && [ "$check_sub_fpr" = "$subkey_fpr" ] \
    || die "the CI export holds unexpected keys"
[ "$(printf '%s\n' "$check" | awk -F: '$1 == "sec" { print $15 }')" = "#" ] \
    || die "PRIMARY SECRET KEY LEAKED into RMAC_APT_SIGNING_SUBKEY.asc (sec is not a '#' stub). Do not use this output."
case "$(printf '%s\n' "$check" | awk -F: '$1 == "ssb" { print $15 }')" in
    "#" | ">"*) die "the signing subkey's secret is missing from the CI export" ;;
esac
# The agent must hold exactly one private key: the subkey's.
agent_keys="$(cd "$check_home/private-keys-v1.d" 2>/dev/null && ls -1 | tr '\n' ' ')"
[ "$agent_keys" = "$subkey_grip.key " ] \
    || die "PRIMARY SECRET KEY LEAKED or subkey missing: the CI export produced private keys '$agent_keys'"

printf 'Lulo OS archive signing self-test\n' >"$work_dir/test-message.txt"
gpg_at "$check_home" --pinentry-mode error --local-user "$primary_fpr" --digest-algo SHA512 \
    --clearsign --output "$work_dir/test-message.asc" "$work_dir/test-message.txt" \
    || die "the CI export cannot sign without a passphrase"
gpgv_home="$(new_home gpgv)"
gpgv --homedir "$gpgv_home" --status-fd 1 --keyring "$output_dir/rmac-archive-keyring.gpg" \
    "$work_dir/test-message.asc" >"$work_dir/gpgv.status" 2>>"$gpg_log" \
    || die "gpgv rejected the test signature against rmac-archive-keyring.gpg"
# VALIDSIG <signing fpr> <date> <timestamp> <expiry> <version> <reserved>
#          <pubkey algo> <hash algo> <class> <primary fpr>
awk -v sub_fpr="$subkey_fpr" -v pri_fpr="$primary_fpr" \
    '$1 == "[GNUPG:]" && $2 == "VALIDSIG" && $3 == sub_fpr && $12 == pri_fpr { ok = 1 } END { exit !ok }' \
    "$work_dir/gpgv.status" || die "the test signature was not made by the signing subkey"
stop_agent "$check_home"

# --- encrypted primary-key backup --------------------------------------------

say "Writing the encrypted primary-key backup..."
stage="$work_dir/stage/$BACKUP_DIR_NAME"
mkdir -p "$stage"
gpg_with_passphrase "$main_home" "$primary_passphrase" \
    --armor --export-secret-keys "$primary_fpr" >"$stage/primary-secret-key.asc" \
    || die "could not export the primary secret key"
cp "$revocation_cert" "$stage/revocation-certificate.rev"
cp "$output_dir/archive-keyring.asc" "$stage/archive-keyring.asc"
cat >"$stage/README.txt" <<EOF
Lulo OS APT archive key -- offline primary-key backup
======================================================

Primary fingerprint:  $primary_fpr
Signing subkey:       $subkey_fpr (expires $subkey_expiry_date UTC)
User ID:              $uid
Created:              $(date -u +%Y-%m-%dT%H:%M:%SZ) with GnuPG $gpg_version

Files
  primary-secret-key.asc      primary + subkey secrets, protected by the
                              PRIMARY-KEY passphrase
  revocation-certificate.rev  revokes the primary key; see "Revoke" below
  archive-keyring.asc         the public keyring (packaging/apt/archive-keyring.asc)

Restore (only on a trusted, preferably offline machine; never into your
everyday GnuPG home):
  export GNUPGHOME="\$(mktemp -d)"; chmod 700 "\$GNUPGHOME"
  gpg --decrypt --output backup.tar primary-key-backup.tar.gpg   # backup passphrase
  tar -xf backup.tar
  gpg --import $BACKUP_DIR_NAME/primary-secret-key.asc

Renew the signing subkey (start a month before $subkey_expiry_date):
  gpg --quick-set-expire $primary_fpr 1y $subkey_fpr         # primary passphrase
  gpg --armor --export-options export-minimal --export $primary_fpr > archive-keyring.asc
  # CI copy of the subkey, passphrase removed, primary as a stub:
  gpg --armor --export-secret-subkeys $subkey_fpr! > subkey-protected.asc
  export CI_HOME="\$(mktemp -d)"; chmod 700 "\$CI_HOME"
  gpg --homedir "\$CI_HOME" --import subkey-protected.asc
  gpg --homedir "\$CI_HOME" --passwd $primary_fpr
  #   old = primary passphrase, new = empty; the "No secret key" message
  #   about the primary stub is expected.
  gpg --homedir "\$CI_HOME" --armor --export-secret-subkeys $subkey_fpr! > RMAC_APT_SIGNING_SUBKEY.asc
  Then: replace packaging/apt/archive-keyring.asc with the new archive-keyring.asc,
  update the RMAC_APT_SIGNING_SUBKEY secret in the apt-signing and apt-refresh
  environments, and publish a release so the rmac-archive-keyring package ships
  the new expiry BEFORE $subkey_expiry_date. Securely delete the exports and
  both temporary homes (gpgconf --homedir DIR --kill all; rm -rf DIR).

Revoke (compromise only; docs/update-trust.md "Signing and rotation"):
  Edit a copy of revocation-certificate.rev and remove the leading ':' from
  the "-----BEGIN PGP PUBLIC KEY BLOCK-----" line, then
  gpg --import <that copy> and publish the revoked public key through the
  reviewed recovery channel.
EOF

tar_path="$work_dir/backup.tar"
COPYFILE_DISABLE=1 tar -C "$work_dir/stage" -cf "$tar_path" "$BACKUP_DIR_NAME" \
    || die "could not create the backup tar"
gpg_with_passphrase "$main_home" "$backup_passphrase" \
    --no-symkey-cache --symmetric --cipher-algo AES256 \
    --s2k-mode 3 --s2k-digest-algo SHA512 --s2k-count 65011712 \
    --output "$output_dir/primary-key-backup.tar.gpg" "$tar_path" \
    || die "could not encrypt the backup"
stop_agent "$main_home"

say "Verifying that the backup decrypts and restores..."
restore_home="$(new_home restore)"
gpg_with_passphrase "$restore_home" "$backup_passphrase" \
    --no-symkey-cache --decrypt --output "$work_dir/restored.tar" \
    "$output_dir/primary-key-backup.tar.gpg" || die "the backup does not decrypt with the backup passphrase"
cmp -s "$tar_path" "$work_dir/restored.tar" || die "the decrypted backup differs from what was encrypted"
mkdir "$work_dir/restored"
tar -C "$work_dir/restored" -xf "$work_dir/restored.tar" || die "the decrypted backup is not a valid tar"
for member in primary-secret-key.asc revocation-certificate.rev archive-keyring.asc README.txt; do
    [ -s "$work_dir/restored/$BACKUP_DIR_NAME/$member" ] || die "the backup is missing $member"
done
gpg_at "$restore_home" --import "$work_dir/restored/$BACKUP_DIR_NAME/primary-secret-key.asc" \
    || die "the backed-up primary key does not import"
restored="$(gpg_at "$restore_home" --with-colons --list-secret-keys "$primary_fpr")"
case "$(printf '%s\n' "$restored" | awk -F: '$1 == "sec" { print $15 }')" in
    "#" | "") die "the backup does not contain the primary secret key" ;;
esac
# Exporting a protected key needs its passphrase, so this proves the backed-up
# primary unlocks with the primary-key passphrase (the fresh agent has no cache).
gpg_with_passphrase "$restore_home" "$primary_passphrase" \
    --export-secret-keys "$primary_fpr" >/dev/null \
    || die "the backed-up primary key does not unlock with the primary-key passphrase"
stop_agent "$restore_home"

cat >"$output_dir/key-info.txt" <<EOF
primary_fingerprint=$primary_fpr
signing_subkey_fingerprint=$subkey_fpr
signing_subkey_expires=$subkey_expiry_date
user_id=$uid
EOF

chmod 0600 "$output_dir/RMAC_APT_SIGNING_SUBKEY.asc" "$output_dir/primary-key-backup.tar.gpg"
chmod 0644 "$output_dir/archive-keyring.asc" "$output_dir/rmac-archive-keyring.gpg" \
    "$output_dir/primary-fingerprint.txt" "$output_dir/key-info.txt"
outputs_complete=1

# --- repository update ---------------------------------------------------------

if [ "$write_repo_mode" != "none" ]; then
    say "Updating $repo_dir..."
    python3 - "$repo_dir" "$primary_fpr" "$output_dir/archive-keyring.asc" "$replace_existing" <<'PY' \
        || die "could not update the repository files (the key itself is complete in the output directory)"
import json
import os
import re
import sys
import tempfile

repo, fingerprint, keyring, replace_existing = sys.argv[1:5]


def atomic_write(path, data):
    mode = os.stat(path).st_mode & 0o7777 if os.path.exists(path) else 0o644
    directory = os.path.dirname(path)
    handle, temporary = tempfile.mkstemp(dir=directory, prefix=".archive-key.")
    try:
        with os.fdopen(handle, "wb") as stream:
            stream.write(data)
        os.chmod(temporary, mode)
        os.replace(temporary, path)
    except BaseException:
        if os.path.exists(temporary):
            os.unlink(temporary)
        raise


json_path = os.path.join(repo, "packaging/apt/archive-key.json")
install_path = os.path.join(repo, "scripts/linux/install.sh")
keyring_path = os.path.join(repo, "packaging/apt/archive-keyring.asc")

with open(json_path, encoding="utf-8") as stream:
    data = json.load(stream)
if data.get("format") != 1 or not isinstance(data.get("primary_fingerprints"), list):
    sys.exit("unsupported archive-key.json")
if data["primary_fingerprints"] and replace_existing != "1":
    sys.exit("archive-key.json already pins a key; see docs/update-trust.md")
data["primary_fingerprints"] = [fingerprint]

with open(install_path, encoding="utf-8") as stream:
    script = stream.read()
pattern = re.compile(r'^RMAC_ARCHIVE_KEYRING_FINGERPRINT="[^"\n]*"$', re.MULTILINE)
if len(pattern.findall(script)) != 1:
    sys.exit("install.sh must contain exactly one RMAC_ARCHIVE_KEYRING_FINGERPRINT line")
script = pattern.sub('RMAC_ARCHIVE_KEYRING_FINGERPRINT="%s"' % fingerprint, script)

with open(keyring, "rb") as stream:
    armored = stream.read()

atomic_write(keyring_path, armored)
atomic_write(json_path, (json.dumps(data, indent=2) + "\n").encode("utf-8"))
atomic_write(install_path, script.encode("utf-8"))
PY
fi

# --- summary -----------------------------------------------------------------

if [ "$(uname -s)" = "Darwin" ]; then
    secure_delete="rm -P"
else
    secure_delete="shred -u"
fi
subkey_export="$output_dir/RMAC_APT_SIGNING_SUBKEY.asc"

cat <<EOF

Done. The archive key exists only in $output_dir
(the temporary GnuPG home is being deleted now).

  Primary fingerprint:   $primary_fpr   (offline, certify only, no expiry)
  Signing subkey:        $subkey_fpr
  Subkey expires:        $subkey_expiry_date (UTC)

  archive-keyring.asc, rmac-archive-keyring.gpg   public keyring
  primary-fingerprint.txt, key-info.txt           public facts
  RMAC_APT_SIGNING_SUBKEY.asc                     CI signing subkey (SECRET, no passphrase)
  primary-key-backup.tar.gpg                      encrypted primary-key backup (SECRET)

Next steps:

 1. Give CI the signing subkey (both environments):
      gh secret set RMAC_APT_SIGNING_SUBKEY --env apt-signing < "$subkey_export"
      gh secret set RMAC_APT_SIGNING_SUBKEY --env apt-refresh < "$subkey_export"

 2. Tell CI which key to sign as (the PRIMARY fingerprint; gpg then picks the
    signing subkey):
      gh variable set RMAC_ARCHIVE_SIGNING_FINGERPRINT --body $primary_fpr

 3. Securely delete the CI subkey export once both secrets are set:
      $secure_delete "$subkey_export"
EOF
if [ "$write_repo_mode" != "none" ]; then
    cat <<EOF

 4. Review and commit the three repository files:
      git -C "$repo_dir" diff -- $archive_keyring_rel $archive_key_json_rel $install_sh_rel
      git -C "$repo_dir" add $archive_keyring_rel $archive_key_json_rel $install_sh_rel
      git -C "$repo_dir" commit -m "Pin the Lulo OS archive signing key $primary_fpr"
EOF
else
    cat <<EOF

 4. Put the key in the repository (not done: --no-write-repo): copy
    archive-keyring.asc to $archive_keyring_rel, set "primary_fingerprints"
    in $archive_key_json_rel to ["$primary_fpr"], set
    RMAC_ARCHIVE_KEYRING_FINGERPRINT="$primary_fpr" in $install_sh_rel, commit.
EOF
fi
cat <<EOF

 5. Copy primary-key-backup.tar.gpg to TWO separate offline media (e.g. two
    USB drives kept in different places), then delete it from this machine:
      $secure_delete "$output_dir/primary-key-backup.tar.gpg"
    Store the two passphrases separately: the primary-key passphrase and the
    backup passphrase each in the password manager AND on paper, never
    together with the backup media.

 6. Record the subkey expiry ($subkey_expiry_date) and set a calendar reminder
    one month before it. Renewal needs the offline primary: decrypt the backup
    into a fresh GNUPGHOME, run
      gpg --quick-set-expire $primary_fpr 1y $subkey_fpr
    re-export the public keyring and the CI subkey (README.txt inside the
    backup has the exact commands), update the RMAC_APT_SIGNING_SUBKEY secret
    and $archive_keyring_rel, and publish a release so the
    rmac-archive-keyring package ships the new expiry BEFORE the old one lapses.

 7. Replacing this key later is a rotation: follow docs/update-trust.md
    "Signing and rotation" (overlap-first), never just rerun this script.
EOF
