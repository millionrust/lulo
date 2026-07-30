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

The verifier binds every generated crate URL and SHA-256 to `Cargo.lock`,
requires offline Cargo configuration, rejects an incomplete/extra inventory,
and enforces the reviewed runtime permissions. A lockfile change must update
the generated sources in the same commit.
