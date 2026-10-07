fn main() {
    // Deliberately no resource embedding here (contrast every other app
    // crate's build.rs). A real `cargo build --release` of this package
    // alongside the other Windows apps hit CVTRES error CVT1100
    // ("duplicate resource. type:VERSION, name:1, language:0x0409") at
    // link time -- `rmac-windows-resource-build`'s own output
    // (resource.lib) listed twice in the linker command for reasons not
    // pinned down in the time this pass had (gpui's own manifest resource,
    // the only other embedded Windows resource in the graph, links once,
    // correctly; winres's `compile()` only ever prints its
    // `cargo:rustc-link-lib` once per run; nothing else in this package's
    // dependency graph should request a native "resource" lib by name).
    // Files still installs and runs correctly; it just keeps Explorer's
    // default binary icon and has no FileDescription/CompanyName until
    // this is understood. ADR 0023 "Installer" "What is left" records it.
}
