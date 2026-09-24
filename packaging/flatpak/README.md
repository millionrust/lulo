# Flatpak package sources

`org.rmac.TextEditor.json` is the only reviewed application manifest.
`decisions.json` is the authoritative native/sandbox boundary for all seven
applications. `cargo-sources.json` is generated data and must not be edited by
hand.

Regenerate Cargo sources from the committed lockfile with the reviewed
flatpak-builder-tools revision:

```sh
curl -fsSL \
  https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/737c0085912f9f7dabf9341d4608e2a77a51a73a/cargo/flatpak-cargo-generator.py \
  -o /tmp/flatpak-cargo-generator.py
uv run /tmp/flatpak-cargo-generator.py Cargo.lock \
  -o packaging/flatpak/cargo-sources.json
python3 scripts/linux/verify-flatpak-package.py
```

The generator clones each locked Git repository (Zed, gpui-component and
Zed's forks) into `$XDG_CACHE_HOME/flatpak-cargo/` and vendors each Git crate
from the last directory it finds with that package name. Zed's tree also
carries lint fixtures named `gpui` and `gpui_shared_string` under
`tooling/lints/test_fixture/`, so delete that directory from the cached Zed
checkout before generating; the verifier rejects a vendored Git crate whose
name or version differs from `Cargo.lock`.

The verifier binds every generated crate URL and SHA-256 to `Cargo.lock`,
every Git checkout to its locked repository and commit,
requires offline Cargo configuration, rejects an incomplete/extra inventory,
and enforces the reviewed runtime permissions. A lockfile change must update
the generated sources in the same commit.
