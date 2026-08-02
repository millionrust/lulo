//! macOS development display backend and fixture parsers.

use super::*;

#[cfg(target_os = "macos")]
pub(super) fn system_restore_snapshot(_: &Snapshot, _: &Snapshot) -> Result<Snapshot, Error> {
    Err(Error::new(
        "restore display layout",
        "display configuration is only available in the niri session",
    ))
}

#[cfg(target_os = "macos")]
pub(super) fn system_snapshot() -> Result<Snapshot, Error> {
    let output = command(
        "system_profiler",
        &["SPDisplaysDataType"],
        "read display information",
    )?;
    let (graphics, outputs) = parse_macos_displays(&output);
    Ok(Snapshot {
        available: true,
        can_configure: false,
        can_persist: false,
        mirror_supported: false,
        compositor: "macOS".to_string(),
        graphics,
        outputs,
        persistence_detail: Some(
            "Persistent display changes are available only in the niri session.".into(),
        ),
    })
}

#[cfg(target_os = "macos")]
pub(super) fn system_persist_layout(_: &Layout) -> Result<Snapshot, Error> {
    Err(Error::new(
        "save the display layout",
        "persistent display changes are available only in the niri session",
    ))
}

#[cfg(all(target_os = "macos", not(test)))]
pub(super) fn validate_layout(_: &Layout, _: &Snapshot) -> Result<(), Error> {
    Err(Error::new(
        "validate the display layout",
        "configurable layouts are available only in the niri session",
    ))
}

#[cfg(target_os = "macos")]
pub(super) fn run_niri_output(_: &str, _: &[&str], operation: &'static str) -> Result<(), Error> {
    Err(Error::new(
        operation,
        "display configuration is only available in the niri session",
    ))
}

#[cfg(any(target_os = "macos", test))]
pub(super) fn parse_macos_displays(output: &str) -> (Option<String>, Vec<Output>) {
    let mut graphics = None;
    let mut outputs = Vec::new();
    for raw in output.lines() {
        let indentation = raw.len() - raw.trim_start().len();
        let line = raw.trim();
        if graphics.is_none() {
            graphics = line
                .strip_prefix("Chipset Model:")
                .map(str::trim)
                .map(str::to_string);
        }
        if indentation == 8 && line.ends_with(':') && line != "Displays:" {
            let name = line.trim_end_matches(':').to_string();
            outputs.push(Output {
                id: name.clone(),
                connector: name.clone(),
                name,
                serial: None,
                physical_size_mm: None,
                modes: Vec::new(),
                current_mode: None,
                logical: None,
                primary: false,
                detail: None,
            });
        } else if let Some(display) = outputs.last_mut() {
            if let Some(resolution) = line.strip_prefix("Resolution:") {
                if let Some((width, height)) = parse_macos_resolution(resolution) {
                    display.modes.push(Mode {
                        width,
                        height,
                        refresh_rate: 0,
                        preferred: true,
                    });
                    display.current_mode = Some(0);
                }
            } else if let Some(detail) = line.strip_prefix("Display Type:") {
                display.detail = Some(detail.trim().to_string());
            } else if line == "Main Display: Yes" {
                display.primary = true;
            }
        }
    }
    (graphics, outputs)
}

#[cfg(any(target_os = "macos", test))]
pub(super) fn parse_macos_resolution(resolution: &str) -> Option<(u16, u16)> {
    let mut numbers = resolution
        .split_whitespace()
        .filter_map(|part| part.replace(',', "").parse::<u16>().ok());
    Some((numbers.next()?, numbers.next()?))
}
