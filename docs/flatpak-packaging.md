# Flatpak and native packaging boundary

rmac uses Flatpak only where the sandbox strengthens the product without
requiring misleading escape hatches. A manifest is not accepted merely because
an application can be made to launch with `--filesystem=host`,
`--socket=session-bus`, or `--device=all`.

The policy follows the official Flatpak model:

- applications start without host files, network, devices, processes, or broad
  D-Bus access;
- user-mediated resources cross XDG portals;
- static filesystem access is avoided when a portal covers the workflow;
- unfiltered session/system buses and broad host/home access are not acceptable
  substitutes for a native system application.

References:

- <https://docs.flatpak.org/en/latest/manifests.html>
- <https://docs.flatpak.org/en/latest/sandbox-permissions.html>
- <https://docs.flatpak.org/en/latest/flatpak-builder-command-reference.html>
- <https://docs.flathub.org/docs/for-app-authors/requirements>

## Packaged application

Text Editor is the sole current Flatpak candidate. Its Linux open/save prompts
use the XDG FileChooser portal, printing uses the XDG Print portal, and recovery
plus application state use XDG application directories. The manifest therefore
grants only:

- `--socket=wayland` to present native Wayland windows;
- `--device=dri` for GPUI GPU rendering.

It grants no host/home filesystem, network, audio, X11, broad D-Bus, or
all-device access. Portal APIs remain available through Flatpak's filtered
session bus without an extra `--talk-name` grant.

The manifest uses Freedesktop SDK/Platform 25.08 and the matching Rust SDK
extension. Cargo dependencies are generated from the committed lockfile as
individually checksummed, offline sources. The application source is the local
repository so developers can build an exact worktree; release automation must
replace that source with a signed tag/archive before publication.

On each native Linux architecture, prepare the SDK/runtime and source cache
explicitly while online, then build in the separate no-download phase:

```sh
bash scripts/linux/build-flatpak-candidate.sh --prepare-online
bash scripts/linux/build-flatpak-candidate.sh --build-offline
flatpak install --user \
  "target/flatpak-candidate/$(uname -m)/result/org.rmac.TextEditor.$(uname -m).flatpak"
flatpak run org.rmac.TextEditor
```

Both phases require the same clean tracked and untracked source tree. The
online phase is the only phase allowed to use
`--install-deps-from=flathub`; it installs the matching user runtimes and asks
Flatpak Builder to cache every manifest source, then records the exact commit,
architecture, tool, manifest, and generated-source hashes. The offline phase
refuses an oversized or mismatched preparation record, uses
`--disable-download` and `--sandbox`, exports without installing, and atomically
publishes a bounded bundle hash summary under ignored `target/`. Disconnecting
the network during that second command provides stronger external evidence that
no ambient downloader bypassed the builder contract. Installation and launch
stay explicit because they begin the H3 portal, permission, and interaction
review.

## Native and deferred applications

The authoritative matrix is
`packaging/flatpak/decisions.json`.

- Notes remains native temporarily. Its import/export authority is
  portal-compatible, but its reserved Wayland application identity and Linux
  acceptance evidence must land before a Flatpak manifest.
- Files remains native because broad filesystem, mount, Trash, metadata, and
  host-open authority is its product purpose.
- Terminal remains native because a useful terminal must launch the host shell
  and control PTYs, processes, signals, and the host filesystem.
- System Monitor remains native because the sandbox intentionally hides the
  host process and metric authorities it reports and controls.
- Apps remains native because it resolves host desktop-entry precedence
  and activates arbitrary reviewed host applications.
- System Settings remains native because its purpose is coordinated system
  D-Bus, polkit, package, hardware, privacy, and session mutation.
- Shell components remain trusted native session processes. Granting a sandbox
  the compositor, layer-shell, system-service, and cross-application authority
  they require would weaken isolation while misrepresenting their trust role.

## Publication gates

Text Editor is not ready for a public Flatpak claim until Ubuntu/niri proves:

- the manifest builds offline for x86_64 and aarch64;
- the exported desktop identity groups windows correctly;
- portal open, multi-open, save, save-copy, conflict, and Print journeys work;
- no undeclared permission is added during runtime auditing;
- recovery stays private to the Flatpak application data;
- Wayland, GPU fallback, scaling, IME, clipboard, accessibility, and failure
  recovery meet the corresponding application gates.

Notes gains a manifest only after its identity and the same portal/sandbox
evidence are complete. Native applications do not gain broad Flatpak manifests
unless a future portal can preserve their exact authority with less access.
