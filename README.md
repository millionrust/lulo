# Lulo OS

Lulo OS is an open-source desktop for Linux that feels like a Mac. It gives
you a menu bar, a Dock, Spotlight, Mission Control, Control Center and
notifications, along with a set of apps built to match macOS 26. Everything is
written in Rust and drawn with [GPUI](https://github.com/zed-industries/zed),
the GPU UI framework behind the Zed editor.

It runs on Ubuntu 26.04 as its own login session on top of the
[niri](https://github.com/niri-wm/niri) Wayland compositor. Your normal Ubuntu
desktop stays installed, so you can always log back into it.

Lulo OS was called rmac until September 2026. The code, packages and settings
still use the `rmac` name while the rename is in progress.

> **Status:** early and moving fast. Lulo OS isn't ready for daily use yet. See
> [known limitations](docs/known-limitations.md) and the [plan](PLAN_NEW.md).

## What's in it

**The desktop:** menu bar, Dock, Spotlight, Mission Control and App Exposé,
Control Center, Notification Center, ⌘Tab switching, screenshots, clipboard
history, a lock screen, and Open/Save panels that every app on the system uses.

**The apps:** Files, System Settings, Terminal, Text Editor, Notes, System
Monitor, Preview, Calculator, Clock, Weather, Media Player and Archive Utility.

The measurements come from a real Mac, but no Apple artwork, fonts or services
are used. Linux services such as NetworkManager, BlueZ, PipeWire and systemd
still do the real work underneath.

## Try an app

You need Rust (the right version installs itself from `rust-toolchain.toml`).
On macOS, also install the Metal toolchain once:

```sh
xcodebuild -downloadComponent MetalToolchain
```

On Ubuntu, install the build dependencies listed in the
[developer guide](docs/developer-guide.md#ubuntu-build-dependencies).

Then run any app:

```sh
cargo run -p rmac-finder          # Files
cargo run -p rmac-notes           # Notes
cargo run -p rmac-terminal        # Terminal
cargo run -p rmac-calculator      # Calculator
```

More commands are in the
[developer guide](docs/developer-guide.md#applications).

## Run the whole desktop

The complete session needs an Ubuntu 26.04 PC with niri. Follow
[Linux reference bring-up](docs/linux-reference-bringup.md). It keeps the stock
GNOME session untouched, so it's safe to try.

## Learn more

- [User guide](docs/user-guide.md): using the desktop and apps
- [Architecture](ARCHITECTURE.md): how the pieces fit together
- [Developer guide](docs/developer-guide.md): crates, builds, checks and all the docs
- [Why niri](docs/decisions/0007-compositor-choice.md): one of the design decisions recorded in `docs/decisions/`
- [Troubleshooting](docs/troubleshooting.md): recovery, safe mode and logs

## Contributing

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) first. It
explains how to keep changes small and test only what you touched. Please
report security problems privately, as described in [SECURITY.md](SECURITY.md).

## License

MIT. See [LICENSE](LICENSE). Third-party code and assets keep their own
licenses.
