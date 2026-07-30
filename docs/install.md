# Install rmac

> rmac is not released for general installation yet.

The repository contains the applications, session integration payload, and
native package assembly contract, but the signed APT repository and clean-PC
install gates are still pending. Do not install rmac on a primary machine or
replace the stock Ubuntu desktop. Use a separate test account on a disposable
Ubuntu 26.04 machine and keep the Ubuntu/GNOME Wayland session available.

## Before you start

The current target is Ubuntu 26.04 with a Vulkan-capable GPU and niri. Check
the provisional [hardware support](hardware-support.md) first. Keep at least
25 GiB free before a release build and stop if the data volume approaches
15 GiB free.

Install Rust with `rustup`; the repository's `rust-toolchain.toml` selects the
required toolchain. Install the Ubuntu build dependencies listed in the
[README](../README.md#ubuntu-build-dependencies), then clone the repository.

## Build one application

Build only what you need while developing:

```sh
cargo build --locked -p rmac-text-editor
cargo run --locked -p rmac-text-editor
```

Other application packages are listed in the [README](../README.md#applications).
Use the normal `target` directory so dependencies are reused. A release build
is appropriate only for performance or package evidence.

## Development session

The home-directory development installer is documented in
[Session supervisor](session-supervisor.md). It builds the current session
components and installs user-owned units under XDG locations; it does not
install a system package or alter `/usr`.

Before launching the session, run the read-only Linux preflight:

```sh
scripts/linux/run-reference-gates.sh --session niri --preflight-only
```

Enable the generated niri shortcut include only when the shortcut status says
`fallback-required`. The GlobalShortcuts portal is otherwise the sole shortcut
owner.

## Native packages

The native package assembler is for clean build/VM evidence. It must never be
pointed at `/` or used as a root installer. The resulting packages are not a
public release until signing, APT trust, clean install/upgrade/rollback, and
hardware gates pass. See [Native packaging](native-packaging.md).

## First login

At GDM, choose the separate rmac session. Do not remove Ubuntu/GNOME. Confirm
that the desktop, top bar, Dock, shortcuts, Settings, sound, network, lock, and
logout work before placing any test data in the account. If startup fails,
return to GDM and choose Ubuntu/GNOME, then follow
[Troubleshooting and recovery](troubleshooting.md).
