//! Print this PC's hardware profile.
//!
//!   rmac-hardware             the profile as JSON, then the summary line
//!   rmac-hardware --check     also fail (exit 1) when detection is over budget
//!   rmac-hardware --first-run store it in Lulo's settings as the shell does
//!
//! Detection runs three times and the fastest run is the one reported and
//! checked: the first also pays for loading the platform's libraries.

fn main() {
    #[cfg(windows)]
    // SAFETY: before any window exists; makes per-monitor scales real.
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let check = arguments.iter().any(|argument| argument == "--check");
    let first_run = arguments.iter().any(|argument| argument == "--first-run");
    let first = rmac_hardware::detect();
    let mut best = first.clone();
    for _ in 0..2 {
        let next = rmac_hardware::detect();
        if next.detection_micros < best.detection_micros {
            best = next;
        }
    }
    match serde_json::to_string_pretty(&best) {
        Ok(json) => println!("{json}"),
        Err(error) => eprintln!("rmac-hardware: {error}"),
    }
    println!("{}", best.summary());
    println!(
        "detection: first {} us, best {} us, budget {} us",
        first.detection_micros,
        best.detection_micros,
        rmac_hardware::BUDGET_MICROS
    );
    if first_run {
        let outcome = rmac_hardware::store::record(&best);
        println!("stored: {outcome:?}");
    }
    if check && best.detection_micros > rmac_hardware::BUDGET_MICROS {
        eprintln!("rmac-hardware: detection took longer than the budget");
        std::process::exit(1);
    }
}
