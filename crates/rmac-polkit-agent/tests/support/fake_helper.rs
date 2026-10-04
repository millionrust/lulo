//! A stand-in for `polkit-agent-helper-1`: a /bin/sh script speaking the
//! helper's line protocol. It accepts "correct horse" for the user named on
//! its command line, refuses anything else, and fails outright when the
//! cookie is not the one it expects on stdin or when more than the user
//! name is on its command line (polkit never puts the cookie there).

#![allow(dead_code)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::sync::OnceLock;

pub const PASSWORD: &str = "correct horse";

const SCRIPT: &str = r#"#!/bin/sh
[ "$#" -eq 1 ] || { echo FAILURE; exit 1; }
IFS= read -r cookie || exit 1
case "$cookie" in
  cookie-*|ok-*|bad-*|wait-*|queue-*) ;;
  *) echo FAILURE; exit 1 ;;
esac
printf '%s\n' 'PAM_TEXT_INFO Hello\tthere'
printf '%s\n' 'PAM_PROMPT_ECHO_OFF Password: '
IFS= read -r password || exit 1
if [ "$password" = "correct horse" ]; then
  echo SUCCESS
else
  printf '%s\n' 'PAM_ERROR_MSG Sorry'
  echo FAILURE
fi
"#;

/// Written once per test binary, before any test spawns it, so no other
/// thread can hold the file open for writing at exec time (ETXTBSY).
pub fn path() -> PathBuf {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("rmac-polkit-agent-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("polkit-agent-helper-1");
        std::fs::write(&path, SCRIPT).expect("write fake helper");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("make fake helper executable");
        path
    })
    .clone()
}
