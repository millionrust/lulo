This is the `gpui-component` 0.5.2 UI, assets, and macros source from
`longbridge/gpui-component` commit `0775df394083c1ed74f36f846b78868d1267398f`.
The upstream Apache-2.0 license is in `LICENSE-APACHE`.

The functional changes are limited to `crates/ui/src/input/blink_cursor.rs`
and `crates/ui/src/input/state.rs`: focused carets park visible after two
seconds without input, and the timer stops when the window deactivates. This
keeps text fields ready to type without repainting an idle software-rendered
window. Two upstream whitespace lines were also normalized in `actions.rs`
and `text/text_view.rs`.
`Cargo.toml` limits the vendored workspace to the three required crates,
uses rmac's GPUI compatibility patches, and matches rmac's `iterate` profile.
The vendored workspace's lockfile was seeded from rmac's root lockfile, so
its focused tests use the same GPUI revision and shared build artifacts.
