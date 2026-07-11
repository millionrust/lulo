//! Cross-platform display discovery and safe niri output controls.

#[cfg(any(not(target_os = "macos"), test))]
use serde::Deserialize;
#[cfg(any(not(target_os = "macos"), test))]
use std::collections::HashMap;
use std::fmt;
use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mode {
    pub width: u16,
    pub height: u16,
    /// Refresh rate in millihertz.
    pub refresh_rate: u32,
    pub preferred: bool,
}

impl Mode {
    pub fn label(self) -> String {
        if self.refresh_rate == 0 {
            format!("{} × {}", self.width, self.height)
        } else {
            format!(
                "{} × {} at {:.3} Hz",
                self.width,
                self.height,
                f64::from(self.refresh_rate) / 1000.0
            )
        }
    }

    fn niri_argument(self) -> String {
        format!(
            "{}x{}@{:.3}",
            self.width,
            self.height,
            f64::from(self.refresh_rate) / 1000.0
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transform {
    Normal,
    Rotate90,
    Rotate180,
    Rotate270,
    Flipped,
    Flipped90,
    Flipped180,
    Flipped270,
    Other(String),
}

impl Transform {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Normal => "Standard",
            Self::Rotate90 => "90°",
            Self::Rotate180 => "180°",
            Self::Rotate270 => "270°",
            Self::Flipped => "Flipped",
            Self::Flipped90 => "Flipped 90°",
            Self::Flipped180 => "Flipped 180°",
            Self::Flipped270 => "Flipped 270°",
            Self::Other(_) => "Unknown",
        }
    }

    pub fn is_configurable(&self) -> bool {
        !matches!(self, Self::Other(_))
    }

    fn niri_argument(&self) -> Option<&'static str> {
        match self {
            Self::Normal => Some("normal"),
            Self::Rotate90 => Some("90"),
            Self::Rotate180 => Some("180"),
            Self::Rotate270 => Some("270"),
            Self::Flipped => Some("flipped"),
            Self::Flipped90 => Some("flipped-90"),
            Self::Flipped180 => Some("flipped-180"),
            Self::Flipped270 => Some("flipped-270"),
            Self::Other(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LogicalOutput {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub transform: Transform,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    /// Stable output name used to address this output through niri.
    pub id: String,
    pub connector: String,
    pub name: String,
    pub serial: Option<String>,
    pub physical_size_mm: Option<(u32, u32)>,
    pub modes: Vec<Mode>,
    pub current_mode: Option<usize>,
    pub logical: Option<LogicalOutput>,
    pub primary: bool,
    pub detail: Option<String>,
}

impl Output {
    pub fn current_mode(&self) -> Option<Mode> {
        self.current_mode
            .and_then(|index| self.modes.get(index).copied())
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub available: bool,
    pub can_configure: bool,
    pub compositor: String,
    pub graphics: Option<String>,
    pub outputs: Vec<Output>,
}

#[derive(Debug)]
pub struct Error {
    operation: &'static str,
    detail: String,
}

impl Error {
    fn new(operation: &'static str, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "could not {}: {}", self.operation, self.detail)
    }
}

impl std::error::Error for Error {}

pub fn snapshot() -> Result<Snapshot, Error> {
    system_snapshot()
}

pub fn set_mode(output: &str, mode: Mode) -> Result<(), Error> {
    validate_output_id(output)?;
    run_niri_output(
        output,
        &["mode", &mode.niri_argument()],
        "change display mode",
    )
}

pub fn set_scale(output: &str, scale: f64) -> Result<(), Error> {
    validate_output_id(output)?;
    if !scale.is_finite() || !(0.5..=4.0).contains(&scale) {
        return Err(Error::new("change display scale", "invalid scale"));
    }
    run_niri_output(
        output,
        &["scale", &format!("{scale:.2}")],
        "change display scale",
    )
}

pub fn set_transform(output: &str, transform: &Transform) -> Result<(), Error> {
    validate_output_id(output)?;
    let transform = transform
        .niri_argument()
        .ok_or_else(|| Error::new("change display rotation", "unsupported transform"))?;
    run_niri_output(output, &["transform", transform], "change display rotation")
}

#[cfg(not(target_os = "macos"))]
fn system_snapshot() -> Result<Snapshot, Error> {
    let output = command("niri", &["msg", "--json", "outputs"], "read niri displays")?;
    let outputs = parse_niri_outputs(&output)?;
    Ok(Snapshot {
        available: true,
        can_configure: true,
        compositor: "niri".to_string(),
        graphics: None,
        outputs,
    })
}

#[cfg(target_os = "macos")]
fn system_snapshot() -> Result<Snapshot, Error> {
    let output = command(
        "system_profiler",
        &["SPDisplaysDataType"],
        "read display information",
    )?;
    let (graphics, outputs) = parse_macos_displays(&output);
    Ok(Snapshot {
        available: true,
        can_configure: false,
        compositor: "macOS".to_string(),
        graphics,
        outputs,
    })
}

#[cfg(not(target_os = "macos"))]
fn run_niri_output(output: &str, arguments: &[&str], operation: &'static str) -> Result<(), Error> {
    let mut command_arguments = vec!["msg", "output", output];
    command_arguments.extend_from_slice(arguments);
    command("niri", &command_arguments, operation)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn run_niri_output(_: &str, _: &[&str], operation: &'static str) -> Result<(), Error> {
    Err(Error::new(
        operation,
        "display configuration is only available in the niri session",
    ))
}

fn validate_output_id(output: &str) -> Result<(), Error> {
    if output.trim().is_empty() || output.contains('\0') {
        Err(Error::new("address display", "invalid output name"))
    } else {
        Ok(())
    }
}

fn command(
    program: &'static str,
    arguments: &[&str],
    operation: &'static str,
) -> Result<String, Error> {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .map_err(|error| Error::new(operation, error.to_string()))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(Error::new(
            operation,
            if detail.is_empty() {
                format!("{program} exited with {}", output.status)
            } else {
                detail
            },
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(any(not(target_os = "macos"), test))]
#[derive(Deserialize)]
struct NiriOutput {
    name: String,
    #[serde(default)]
    make: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    serial: Option<String>,
    #[serde(default)]
    physical_size: Option<(u32, u32)>,
    #[serde(default)]
    modes: Vec<NiriMode>,
    #[serde(default)]
    current_mode: Option<usize>,
    #[serde(default)]
    logical: Option<NiriLogicalOutput>,
}

#[cfg(any(not(target_os = "macos"), test))]
#[derive(Deserialize)]
struct NiriMode {
    width: u16,
    height: u16,
    refresh_rate: u32,
    #[serde(default)]
    is_preferred: bool,
}

#[cfg(any(not(target_os = "macos"), test))]
#[derive(Deserialize)]
struct NiriLogicalOutput {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale: f64,
    transform: String,
}

#[cfg(any(not(target_os = "macos"), test))]
fn parse_niri_outputs(json: &str) -> Result<Vec<Output>, Error> {
    let raw = serde_json::from_str::<HashMap<String, NiriOutput>>(json)
        .map_err(|error| Error::new("parse niri displays", error.to_string()))?;
    let mut outputs = raw
        .into_iter()
        .map(|(id, output)| {
            let display_name = [output.make.as_str(), output.model.as_str()]
                .into_iter()
                .filter(|part| !part.is_empty() && *part != "Unknown")
                .collect::<Vec<_>>()
                .join(" ");
            Output {
                id,
                connector: output.name.clone(),
                name: if display_name.is_empty() {
                    output.name
                } else {
                    display_name
                },
                serial: output.serial,
                physical_size_mm: output.physical_size,
                modes: output
                    .modes
                    .into_iter()
                    .map(|mode| Mode {
                        width: mode.width,
                        height: mode.height,
                        refresh_rate: mode.refresh_rate,
                        preferred: mode.is_preferred,
                    })
                    .collect(),
                current_mode: output.current_mode,
                logical: output.logical.map(|logical| LogicalOutput {
                    x: logical.x,
                    y: logical.y,
                    width: logical.width,
                    height: logical.height,
                    scale: logical.scale,
                    transform: parse_transform(&logical.transform),
                }),
                primary: false,
                detail: None,
            }
        })
        .collect::<Vec<_>>();
    outputs.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(outputs)
}

#[cfg(any(not(target_os = "macos"), test))]
fn parse_transform(transform: &str) -> Transform {
    match transform {
        "Normal" | "normal" => Transform::Normal,
        "90" => Transform::Rotate90,
        "180" => Transform::Rotate180,
        "270" => Transform::Rotate270,
        "Flipped" | "flipped" => Transform::Flipped,
        "Flipped90" | "flipped-90" => Transform::Flipped90,
        "Flipped180" | "flipped-180" => Transform::Flipped180,
        "Flipped270" | "flipped-270" => Transform::Flipped270,
        other => Transform::Other(other.to_string()),
    }
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_displays(output: &str) -> (Option<String>, Vec<Output>) {
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
fn parse_macos_resolution(resolution: &str) -> Option<(u16, u16)> {
    let mut numbers = resolution
        .split_whitespace()
        .filter_map(|part| part.replace(',', "").parse::<u16>().ok());
    Some((numbers.next()?, numbers.next()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn niri_fixture_preserves_modes_layout_and_stable_name() {
        let outputs = parse_niri_outputs(
            r#"{
              "Dell Inc. U2723QE ABC": {
                "name":"DP-1", "make":"Dell Inc.", "model":"U2723QE", "serial":"ABC",
                "physical_size":[600,340],
                "modes":[
                  {"width":3840,"height":2160,"refresh_rate":60000,"is_preferred":true},
                  {"width":2560,"height":1440,"refresh_rate":59951,"is_preferred":false}
                ],
                "current_mode":0, "is_custom_mode":false,
                "vrr_supported":false, "vrr_enabled":false,
                "logical":{"x":0,"y":0,"width":1920,"height":1080,"scale":2.0,"transform":"Normal"}
              }
            }"#,
        )
        .unwrap();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].id, "Dell Inc. U2723QE ABC");
        assert_eq!(outputs[0].connector, "DP-1");
        assert_eq!(outputs[0].current_mode().unwrap().width, 3840);
        assert_eq!(outputs[0].logical.as_ref().unwrap().scale, 2.0);
    }

    #[test]
    fn unknown_transform_is_preserved_without_breaking_snapshot() {
        assert_eq!(parse_transform("Flipped90"), Transform::Flipped90);
        assert!(Transform::Flipped90.is_configurable());
        assert_eq!(
            parse_transform("future-transform"),
            Transform::Other("future-transform".to_string())
        );
        assert!(!parse_transform("future-transform").is_configurable());
    }

    #[test]
    fn mode_and_scale_arguments_are_bounded_and_precise() {
        let mode = Mode {
            width: 2560,
            height: 1440,
            refresh_rate: 143_912,
            preferred: false,
        };
        assert_eq!(mode.niri_argument(), "2560x1440@143.912");
        assert!(set_scale("", 1.0).is_err());
        assert!(set_scale("DP-1", 10.0).is_err());
    }

    #[test]
    fn macos_fixture_preserves_main_display_and_graphics() {
        let (graphics, outputs) = parse_macos_displays(concat!(
            "Graphics/Displays:\n\n",
            "      Chipset Model: Apple M3 Pro\n",
            "        Built-in Liquid Retina XDR Display:\n",
            "          Display Type: Built-In Liquid Retina XDR Display\n",
            "          Resolution: 3456 x 2234 Retina\n",
            "          Main Display: Yes",
        ));
        assert_eq!(graphics.as_deref(), Some("Apple M3 Pro"));
        assert_eq!(outputs.len(), 1);
        assert!(outputs[0].primary);
        assert_eq!(outputs[0].current_mode().unwrap().height, 2234);
    }

    #[test]
    fn errors_keep_operation_context() {
        let error = Error::new("read niri displays", "socket unavailable");
        assert_eq!(
            error.to_string(),
            "could not read niri displays: socket unavailable"
        );
    }
}
