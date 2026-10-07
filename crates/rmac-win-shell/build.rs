fn main() {
    // Deliberately no resource embedding here. Enabling it for
    // rmac-finder (Files) hit a real CI link failure (CVTRES CVT1100,
    // "duplicate resource. type:VERSION") not understood in the time this
    // pass had -- see crates/finder/build.rs and ADR 0023 "Installer"
    // "What is left". This crate has the same lib-plus-multiple-bins
    // shape (lulo-session, lulo-shell) that could plausibly hit the same
    // or a related issue, so it is left unembedded too rather than
    // shipped unverified.
}
