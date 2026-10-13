//! Windows display backend.
//!
//! Each active display is addressed by its GDI source name ("\\.\DISPLAY1"),
//! the name `ChangeDisplaySettingsExW` takes. Display Configuration
//! (`QueryDisplayConfig`) supplies the monitor names, connector types and
//! native modes; `EnumDisplaySettingsW` supplies the current and available
//! modes, position and rotation in the units the setters write back.
//!
//! Desktop coordinates on Windows are physical pixels, so the layout's
//! `logical` rectangle is the current mode's pixel size; `scale` is the
//! Windows "Scale" setting (effective DPI / 96) and is read-only, because
//! Windows has no public per-monitor scale setter.
//!
//! Every change is staged per display with `CDS_UPDATEREGISTRY |
//! CDS_NORESET` and then applied in one call, so it is persistent the moment
//! it succeeds; "Keep these settings?" reverts by staging the earlier state
//! the same way.

use super::*;

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig,
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_PREFERRED_MODE, DISPLAYCONFIG_MODE_INFO,
    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_COMPONENT_VIDEO,
    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_COMPOSITE_VIDEO,
    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED,
    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EXTERNAL,
    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_USB_TUNNEL, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DVI,
    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_HD15, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_HDMI,
    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INDIRECT_VIRTUAL,
    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INDIRECT_WIRED, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL,
    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_LVDS, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_MIRACAST,
    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_SVIDEO, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED,
    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EXTERNAL, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME,
    DISPLAYCONFIG_TARGET_PREFERRED_MODE, DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY,
    QDC_ONLY_ACTIVE_PATHS,
};
use windows::Win32::Foundation::{POINT, POINTL};
use windows::Win32::Graphics::Gdi::{
    ChangeDisplaySettingsExW, EnumDisplayDevicesW, EnumDisplaySettingsW, MonitorFromPoint,
    CDS_NORESET, CDS_SET_PRIMARY, CDS_TYPE, CDS_UPDATEREGISTRY, DEVMODEW,
    DEVMODE_DISPLAY_ORIENTATION, DISPLAY_DEVICEW, DISPLAY_DEVICE_ATTACHED_TO_DESKTOP,
    DISPLAY_DEVICE_PRIMARY_DEVICE, DISP_CHANGE, DISP_CHANGE_BADDUALVIEW, DISP_CHANGE_BADFLAGS,
    DISP_CHANGE_BADMODE, DISP_CHANGE_BADPARAM, DISP_CHANGE_FAILED, DISP_CHANGE_NOTUPDATED,
    DISP_CHANGE_RESTART, DISP_CHANGE_SUCCESSFUL, DMDO_180, DMDO_270, DMDO_90, DMDO_DEFAULT,
    DM_DISPLAYFREQUENCY, DM_DISPLAYORIENTATION, DM_PELSHEIGHT, DM_PELSWIDTH, DM_POSITION,
    ENUM_CURRENT_SETTINGS, ENUM_DISPLAY_SETTINGS_MODE, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

/// What Display Configuration knows about one active source.
struct ConfigPath {
    gdi_name: String,
    friendly: String,
    technology: DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY,
    /// The monitor's native size and refresh rate in hertz.
    preferred: Option<(u32, u32, f64)>,
}

pub(super) fn system_snapshot() -> Result<Snapshot, Error> {
    let paths = config_paths()?;
    let adapters = adapters();
    let mut outputs = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        let device = HSTRING::from(path.gdi_name.as_str());
        let Some(current) = current_devmode(&device) else {
            continue;
        };
        let orientation = devmode_orientation(&current);
        let (x, y) = devmode_position(&current);
        let current_mode = devmode_mode(&current);
        let mut modes = available_modes(&device, current_mode, orientation);
        if let Some((width, height, hertz)) = path.preferred {
            let (width, height) = if orientation_is_portrait(orientation) {
                (height, width)
            } else {
                (width, height)
            };
            mark_preferred(&mut modes, width, height, hertz);
        }
        let current_index = modes
            .iter()
            .position(|mode| (mode.width, mode.height, mode.refresh_rate) == current_mode);
        let built_in = technology_is_internal(path.technology);
        let name = if !path.friendly.is_empty() {
            path.friendly.clone()
        } else if built_in {
            "Built-in Display".to_owned()
        } else {
            format!("Display {}", index + 1)
        };
        let primary = adapters
            .iter()
            .find(|adapter| adapter.name == path.gdi_name)
            .map_or((x, y) == (0, 0), |adapter| adapter.primary);
        outputs.push(Output {
            id: path.gdi_name.clone(),
            connector: technology_label(path.technology).to_owned(),
            name,
            serial: None,
            physical_size_mm: None,
            modes,
            current_mode: current_index,
            logical: Some(LogicalOutput {
                x,
                y,
                width: u32::from(current_mode.0),
                height: u32::from(current_mode.1),
                scale: monitor_scale(x, y),
                transform: orientation_transform(orientation),
            }),
            primary,
            detail: None,
        });
    }
    // The main display first, then by device name.
    outputs.sort_by(|left, right| {
        (!left.primary)
            .cmp(&!right.primary)
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut graphics = Vec::<String>::new();
    for adapter in &adapters {
        if !adapter.description.is_empty() && !graphics.contains(&adapter.description) {
            graphics.push(adapter.description.clone());
        }
    }
    Ok(Snapshot {
        available: true,
        can_configure: true,
        can_persist: true,
        mirror_supported: false,
        compositor: "Windows".to_owned(),
        graphics: (!graphics.is_empty()).then(|| graphics.join(", ")),
        outputs,
        persistence_detail: None,
    })
}

pub(super) fn system_apply_output(
    output: &str,
    change: OutputChange<'_>,
    operation: &'static str,
) -> Result<(), Error> {
    let snapshot = system_snapshot()?;
    let target = snapshot
        .outputs
        .iter()
        .find(|candidate| candidate.id == output)
        .ok_or_else(|| Error::new(operation, "the selected display disconnected"))?;
    let device = HSTRING::from(output);
    let mut mode = current_devmode(&device)
        .ok_or_else(|| Error::new(operation, "Windows did not report the current display mode"))?;
    match change {
        OutputChange::Mode(requested) => {
            mode.dmPelsWidth = u32::from(requested.width);
            mode.dmPelsHeight = u32::from(requested.height);
            mode.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT;
            if requested.refresh_rate >= 2_000 {
                mode.dmDisplayFrequency = requested.refresh_rate / 1000;
                mode.dmFields |= DM_DISPLAYFREQUENCY;
            }
            commit(&[Staged::new(device, mode, false)], operation)
        }
        OutputChange::Scale(scale) => Err(Error::new(
            operation,
            format!(
                "Windows has no public interface for setting a display's scale ({:.0}% was requested); change it in Windows Settings",
                scale * 100.0
            ),
        )),
        OutputChange::Transform(transform) => {
            let orientation = transform_orientation(transform).ok_or_else(|| {
                Error::new(operation, "Windows can rotate a display but cannot flip it")
            })?;
            let previous = devmode_orientation(&mode);
            if orientation_is_portrait(previous) != orientation_is_portrait(orientation) {
                std::mem::swap(&mut mode.dmPelsWidth, &mut mode.dmPelsHeight);
            }
            set_devmode_orientation(&mut mode, orientation);
            mode.dmFields = DM_DISPLAYORIENTATION | DM_PELSWIDTH | DM_PELSHEIGHT;
            commit(&[Staged::new(device, mode, false)], operation)
        }
        OutputChange::Position { x, y } => {
            let mut rectangles = enabled_rectangles(&snapshot);
            let moved = rectangles
                .iter_mut()
                .find(|rectangle| rectangle.id == target.id)
                .ok_or_else(|| Error::new(operation, "the selected display is not enabled"))?;
            moved.x = x;
            moved.y = y;
            check_arrangement(&rectangles, operation)?;
            let primary = snapshot
                .outputs
                .iter()
                .find(|candidate| candidate.primary)
                .map_or(target.id.as_str(), |candidate| candidate.id.as_str());
            stage_arrangement(&rectangles, primary, operation)
        }
    }
}

pub(super) fn system_persist_layout(layout: &Layout) -> Result<Snapshot, Error> {
    let operation = "save the display layout";
    let fresh = system_snapshot()?;
    validate_layout(layout, &fresh)?;
    // Mode, rotation and position changes are already in the registry; only
    // a new main display is left to write.
    if fresh
        .outputs
        .iter()
        .any(|output| output.id == layout.primary && !output.primary)
    {
        let rectangles = layout
            .outputs
            .iter()
            .map(|output| Rectangle {
                id: output.id.clone(),
                x: output.x,
                y: output.y,
                width: output.logical_width,
                height: output.logical_height,
            })
            .collect::<Vec<_>>();
        stage_arrangement(&rectangles, &layout.primary, operation)?;
    }
    let snapshot = system_snapshot()?;
    if !snapshot
        .outputs
        .iter()
        .any(|output| output.id == layout.primary && output.primary)
    {
        return Err(Error::new(
            "verify the display layout",
            "Windows did not make the selected display the main display",
        ));
    }
    Ok(snapshot)
}

pub(super) fn system_restore_snapshot(
    expected: &Snapshot,
    owned_current: &Snapshot,
) -> Result<Snapshot, Error> {
    let operation = "restore display layout";
    let primary = expected
        .outputs
        .iter()
        .find(|output| output.primary && output.logical.is_some())
        .or_else(|| {
            expected
                .outputs
                .iter()
                .find(|output| output.logical.is_some())
        })
        .map(|output| output.id.clone())
        .ok_or_else(|| Error::new(operation, "no enabled display was captured"))?;
    let layout = current_layout(expected, &primary)?;
    let current = system_snapshot()?;
    if !same_complete_layout(owned_current, &current) {
        return Err(Error::new(
            operation,
            "the display layout changed outside this confirmation; refresh Displays before changing it again",
        ));
    }
    if !same_enabled_output_identities(expected, &current) {
        return Err(Error::new(
            operation,
            "the connected display hardware changed; automatic restoration was stopped",
        ));
    }
    let (origin_x, origin_y) = layout
        .outputs
        .iter()
        .find(|output| output.id == primary)
        .map_or((0, 0), |output| (output.x, output.y));
    let mut staged = Vec::new();
    for configured in &layout.outputs {
        let orientation = transform_orientation(&configured.transform)
            .ok_or_else(|| Error::new(operation, "the captured rotation is not supported"))?;
        // Windows lists modes in the display's current orientation, so a
        // rotation being reverted lists the captured mode turned.
        let advertised = current
            .outputs
            .iter()
            .find(|output| output.id == configured.id)
            .is_some_and(|output| {
                let turned = output.logical.as_ref().is_some_and(|logical| {
                    transform_orientation(&logical.transform).is_some_and(|now| {
                        orientation_is_portrait(now) != orientation_is_portrait(orientation)
                    })
                });
                output.modes.iter().any(|mode| {
                    let (width, height) = if turned {
                        (mode.height, mode.width)
                    } else {
                        (mode.width, mode.height)
                    };
                    (width, height, mode.refresh_rate)
                        == (
                            configured.mode.width,
                            configured.mode.height,
                            configured.mode.refresh_rate,
                        )
                })
            });
        if !advertised {
            return Err(Error::new(
                operation,
                "a captured mode is no longer advertised by its display",
            ));
        }
        let device = HSTRING::from(configured.id.as_str());
        let mut mode = current_devmode(&device).ok_or_else(|| {
            Error::new(operation, "Windows did not report the current display mode")
        })?;
        mode.dmPelsWidth = u32::from(configured.mode.width);
        mode.dmPelsHeight = u32::from(configured.mode.height);
        mode.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_POSITION | DM_DISPLAYORIENTATION;
        if configured.mode.refresh_rate >= 2_000 {
            mode.dmDisplayFrequency = configured.mode.refresh_rate / 1000;
            mode.dmFields |= DM_DISPLAYFREQUENCY;
        }
        set_devmode_orientation(&mut mode, orientation);
        set_devmode_position(&mut mode, configured.x - origin_x, configured.y - origin_y);
        staged.push(Staged::new(device, mode, configured.id == primary));
    }
    commit(&staged, operation)?;
    let restored = system_snapshot()?;
    if same_complete_layout(expected, &restored) {
        Ok(restored)
    } else {
        Err(Error::new(
            "verify restored display layout",
            "Windows did not restore the complete captured layout",
        ))
    }
}

/// One enabled display's desktop rectangle, in physical pixels.
#[derive(Clone, Debug, PartialEq)]
struct Rectangle {
    id: String,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

fn enabled_rectangles(snapshot: &Snapshot) -> Vec<Rectangle> {
    snapshot
        .outputs
        .iter()
        .filter_map(|output| {
            let logical = output.logical.as_ref()?;
            Some(Rectangle {
                id: output.id.clone(),
                x: logical.x,
                y: logical.y,
                width: logical.width,
                height: logical.height,
            })
        })
        .collect()
}

/// Windows needs one connected desktop: no two displays may overlap, and
/// every display must share part of an edge with the rest.
fn check_arrangement(rectangles: &[Rectangle], operation: &'static str) -> Result<(), Error> {
    for (index, left) in rectangles.iter().enumerate() {
        for right in rectangles.iter().skip(index + 1) {
            if rectangles_intersect(left, right) {
                return Err(Error::new(operation, "displays must not overlap"));
            }
        }
    }
    if !arrangement_is_connected(rectangles) {
        return Err(Error::new(
            operation,
            "every display must touch another display along an edge",
        ));
    }
    Ok(())
}

fn bounds(rectangle: &Rectangle) -> (i64, i64, i64, i64) {
    let x = i64::from(rectangle.x);
    let y = i64::from(rectangle.y);
    (
        x,
        y,
        x + i64::from(rectangle.width),
        y + i64::from(rectangle.height),
    )
}

fn rectangles_intersect(left: &Rectangle, right: &Rectangle) -> bool {
    let (l_x1, l_y1, l_x2, l_y2) = bounds(left);
    let (r_x1, r_y1, r_x2, r_y2) = bounds(right);
    l_x1 < r_x2 && r_x1 < l_x2 && l_y1 < r_y2 && r_y1 < l_y2
}

fn rectangles_share_edge(left: &Rectangle, right: &Rectangle) -> bool {
    let (l_x1, l_y1, l_x2, l_y2) = bounds(left);
    let (r_x1, r_y1, r_x2, r_y2) = bounds(right);
    let vertical_overlap = l_y1.max(r_y1) < l_y2.min(r_y2);
    let horizontal_overlap = l_x1.max(r_x1) < l_x2.min(r_x2);
    ((l_x2 == r_x1 || r_x2 == l_x1) && vertical_overlap)
        || ((l_y2 == r_y1 || r_y2 == l_y1) && horizontal_overlap)
}

fn arrangement_is_connected(rectangles: &[Rectangle]) -> bool {
    if rectangles.is_empty() {
        return true;
    }
    let mut reached = vec![false; rectangles.len()];
    reached[0] = true;
    let mut pending = vec![0];
    while let Some(index) = pending.pop() {
        for (other, rectangle) in rectangles.iter().enumerate() {
            if !reached[other] && rectangles_share_edge(&rectangles[index], rectangle) {
                reached[other] = true;
                pending.push(other);
            }
        }
    }
    reached.into_iter().all(|reached| reached)
}

/// Writes every display's position with `primary` at the desktop origin,
/// which Windows requires of the main display.
fn stage_arrangement(
    rectangles: &[Rectangle],
    primary: &str,
    operation: &'static str,
) -> Result<(), Error> {
    let (origin_x, origin_y) = rectangles
        .iter()
        .find(|rectangle| rectangle.id == primary)
        .map(|rectangle| (rectangle.x, rectangle.y))
        .ok_or_else(|| Error::new(operation, "the main display is not enabled"))?;
    let mut staged = Vec::new();
    for rectangle in rectangles {
        let device = HSTRING::from(rectangle.id.as_str());
        let mut mode = current_devmode(&device).ok_or_else(|| {
            Error::new(operation, "Windows did not report the current display mode")
        })?;
        set_devmode_position(&mut mode, rectangle.x - origin_x, rectangle.y - origin_y);
        mode.dmFields = DM_POSITION;
        staged.push(Staged::new(device, mode, rectangle.id == primary));
    }
    commit(&staged, operation)
}

struct Staged {
    device: HSTRING,
    mode: DEVMODEW,
    primary: bool,
}

impl Staged {
    fn new(device: HSTRING, mode: DEVMODEW, primary: bool) -> Self {
        Self {
            device,
            mode,
            primary,
        }
    }
}

/// Stages each display's new settings in the registry, then applies them
/// all at once. If Windows refuses one, the displays staged so far get their
/// current settings staged back and nothing is applied.
fn commit(staged: &[Staged], operation: &'static str) -> Result<(), Error> {
    let mut originals: Vec<(&HSTRING, DEVMODEW)> = Vec::new();
    for entry in staged {
        let original = current_devmode(&entry.device).ok_or_else(|| {
            Error::new(operation, "Windows did not report the current display mode")
        })?;
        let mut flags = CDS_UPDATEREGISTRY | CDS_NORESET;
        if entry.primary {
            flags |= CDS_SET_PRIMARY;
        }
        let result = change_settings(&entry.device, &entry.mode, flags);
        if result != DISP_CHANGE_SUCCESSFUL {
            unstage(&originals);
            return Err(Error::new(operation, disp_change_text(result)));
        }
        originals.push((&entry.device, original));
    }
    // SAFETY: a null device and mode apply the settings staged above.
    let result = unsafe { ChangeDisplaySettingsExW(PCWSTR::null(), None, None, CDS_TYPE(0), None) };
    if result != DISP_CHANGE_SUCCESSFUL {
        unstage(&originals);
        return Err(Error::new(operation, disp_change_text(result)));
    }
    Ok(())
}

fn unstage(originals: &[(&HSTRING, DEVMODEW)]) {
    for (device, original) in originals {
        let mut original = *original;
        original.dmFields = DM_PELSWIDTH
            | DM_PELSHEIGHT
            | DM_DISPLAYFREQUENCY
            | DM_POSITION
            | DM_DISPLAYORIENTATION;
        let mut flags = CDS_UPDATEREGISTRY | CDS_NORESET;
        if devmode_position(&original) == (0, 0) {
            flags |= CDS_SET_PRIMARY;
        }
        let _ = change_settings(device, &original, flags);
    }
}

fn change_settings(device: &HSTRING, mode: &DEVMODEW, flags: CDS_TYPE) -> DISP_CHANGE {
    // SAFETY: `mode` is a complete DEVMODEW whose `dmSize` came from
    // `EnumDisplaySettingsW`; the device name is a valid string.
    unsafe { ChangeDisplaySettingsExW(device, Some(mode as *const DEVMODEW), None, flags, None) }
}

fn disp_change_text(result: DISP_CHANGE) -> String {
    match result {
        DISP_CHANGE_RESTART => "Windows must restart before this display change takes effect",
        DISP_CHANGE_BADMODE => "the display does not support this mode",
        DISP_CHANGE_BADDUALVIEW => {
            "this display is in a DualView configuration Windows cannot change"
        }
        DISP_CHANGE_BADFLAGS | DISP_CHANGE_BADPARAM => "Windows rejected the display settings",
        DISP_CHANGE_FAILED => "the graphics driver could not apply the display settings",
        DISP_CHANGE_NOTUPDATED => "Windows could not save the display settings",
        _ => return format!("Windows refused the display change (code {})", result.0),
    }
    .to_owned()
}

fn empty_devmode() -> DEVMODEW {
    DEVMODEW {
        dmSize: std::mem::size_of::<DEVMODEW>() as u16,
        ..Default::default()
    }
}

fn current_devmode(device: &HSTRING) -> Option<DEVMODEW> {
    let mut mode = empty_devmode();
    // SAFETY: `dmSize` is set; the device name is a valid string.
    unsafe { EnumDisplaySettingsW(device, ENUM_CURRENT_SETTINGS, &mut mode) }
        .as_bool()
        .then_some(mode)
}

fn devmode_position(mode: &DEVMODEW) -> (i32, i32) {
    // SAFETY: display DEVMODEs use the display arm of this union.
    let position = unsafe { mode.Anonymous1.Anonymous2.dmPosition };
    (position.x, position.y)
}

fn set_devmode_position(mode: &mut DEVMODEW, x: i32, y: i32) {
    // SAFETY: display DEVMODEs use the display arm of this union.
    let mut display = unsafe { mode.Anonymous1.Anonymous2 };
    display.dmPosition = POINTL { x, y };
    mode.Anonymous1.Anonymous2 = display;
}

fn devmode_orientation(mode: &DEVMODEW) -> u32 {
    // SAFETY: display DEVMODEs use the display arm of this union.
    unsafe { mode.Anonymous1.Anonymous2.dmDisplayOrientation.0 }
}

fn set_devmode_orientation(mode: &mut DEVMODEW, orientation: u32) {
    // SAFETY: display DEVMODEs use the display arm of this union.
    let mut display = unsafe { mode.Anonymous1.Anonymous2 };
    display.dmDisplayOrientation = DEVMODE_DISPLAY_ORIENTATION(orientation);
    mode.Anonymous1.Anonymous2 = display;
}

/// Width, height and refresh rate in millihertz, as the model stores them.
/// Windows reports whole hertz; 0 and 1 mean "the hardware default".
fn devmode_mode(mode: &DEVMODEW) -> (u16, u16, u32) {
    (
        u16::try_from(mode.dmPelsWidth).unwrap_or(u16::MAX),
        u16::try_from(mode.dmPelsHeight).unwrap_or(u16::MAX),
        refresh_millihertz(mode.dmDisplayFrequency),
    )
}

fn refresh_millihertz(hertz: u32) -> u32 {
    if hertz <= 1 {
        0
    } else {
        hertz.saturating_mul(1000)
    }
}

fn available_modes(device: &HSTRING, current: (u16, u16, u32), orientation: u32) -> Vec<Mode> {
    let mut found = Vec::new();
    for index in 0.. {
        let mut mode = empty_devmode();
        // SAFETY: `dmSize` is set; the device name is a valid string.
        if !unsafe { EnumDisplaySettingsW(device, ENUM_DISPLAY_SETTINGS_MODE(index), &mut mode) }
            .as_bool()
        {
            break;
        }
        found.push((devmode_orientation(&mode), devmode_mode(&mode)));
    }
    // Keep the modes listed for the current orientation; a driver that
    // lists every mode unrotated has them turned by `unique_modes`.
    let in_orientation = found.iter().any(|(listed, _)| *listed == orientation);
    let found = found
        .into_iter()
        .filter(|(listed, _)| !in_orientation || *listed == orientation)
        .map(|(_, mode)| mode)
        .collect();
    unique_modes(found, current, orientation_is_portrait(orientation))
}

/// Deduplicates the listed modes (Windows lists each one per colour depth
/// and scaling option), orients them like the current mode, keeps the
/// current one listed, and sorts largest and fastest first.
fn unique_modes(
    found: Vec<(u16, u16, u32)>,
    current: (u16, u16, u32),
    portrait: bool,
) -> Vec<Mode> {
    let mut modes = found;
    let listed_as_landscape = !modes
        .iter()
        .any(|&(width, height, _)| (width, height) == (current.0, current.1))
        && modes
            .iter()
            .any(|&(width, height, _)| (height, width) == (current.0, current.1));
    if portrait && listed_as_landscape {
        for mode in &mut modes {
            std::mem::swap(&mut mode.0, &mut mode.1);
        }
    }
    modes.push(current);
    modes.sort_by(|left, right| right.cmp(left));
    modes.dedup();
    modes
        .into_iter()
        .map(|(width, height, refresh_rate)| Mode {
            width,
            height,
            refresh_rate,
            preferred: false,
        })
        .collect()
}

/// Marks the listed mode closest to the monitor's native timing.
fn mark_preferred(modes: &mut [Mode], width: u32, height: u32, hertz: f64) {
    let target = hertz * 1000.0;
    let best = modes
        .iter_mut()
        .filter(|mode| u32::from(mode.width) == width && u32::from(mode.height) == height)
        .min_by(|left, right| {
            let left = (f64::from(left.refresh_rate) - target).abs();
            let right = (f64::from(right.refresh_rate) - target).abs();
            left.total_cmp(&right)
        });
    if let Some(mode) = best {
        mode.preferred = true;
    }
}

fn orientation_is_portrait(orientation: u32) -> bool {
    orientation == DMDO_90.0 || orientation == DMDO_270.0
}

fn orientation_transform(orientation: u32) -> Transform {
    match orientation {
        value if value == DMDO_DEFAULT.0 => Transform::Normal,
        value if value == DMDO_90.0 => Transform::Rotate90,
        value if value == DMDO_180.0 => Transform::Rotate180,
        value if value == DMDO_270.0 => Transform::Rotate270,
        other => Transform::Other(format!("orientation {other}")),
    }
}

fn transform_orientation(transform: &Transform) -> Option<u32> {
    match transform {
        Transform::Normal => Some(DMDO_DEFAULT.0),
        Transform::Rotate90 => Some(DMDO_90.0),
        Transform::Rotate180 => Some(DMDO_180.0),
        Transform::Rotate270 => Some(DMDO_270.0),
        _ => None,
    }
}

fn technology_is_internal(technology: DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY) -> bool {
    technology == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL
        || technology == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED
        || technology == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED
        || technology == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_LVDS
}

fn technology_label(technology: DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY) -> &'static str {
    match technology {
        value if technology_is_internal(value) => "Built-in",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_HDMI => "HDMI",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EXTERNAL => "DisplayPort",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_USB_TUNNEL => "USB-C",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DVI => "DVI",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_HD15 => "VGA",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EXTERNAL => "UDI",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_SVIDEO => "S-Video",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_COMPOSITE_VIDEO => "Composite",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_COMPONENT_VIDEO => "Component",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_MIRACAST => "Wireless",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INDIRECT_WIRED
        | DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INDIRECT_VIRTUAL => "Virtual",
        _ => "Other",
    }
}

fn monitor_scale(x: i32, y: i32) -> f64 {
    // SAFETY: plain value call; NEAREST never returns null for a point on a
    // display.
    let monitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
    // SAFETY: two u32 outs.
    if unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }.is_ok()
        && dpi_x > 0
    {
        f64::from(dpi_x) / 96.0
    } else {
        1.0
    }
}

fn wide_text(buffer: &[u16]) -> String {
    let length = buffer
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..length])
        .trim()
        .to_owned()
}

struct Adapter {
    /// The GDI source name, such as "\\.\DISPLAY1".
    name: String,
    /// The graphics adapter driving it, such as "Intel(R) UHD Graphics".
    description: String,
    primary: bool,
}

fn adapters() -> Vec<Adapter> {
    let mut adapters = Vec::new();
    for index in 0.. {
        let mut device = DISPLAY_DEVICEW {
            cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
            ..Default::default()
        };
        // SAFETY: `cb` is set; a null device lists the adapters' sources.
        if !unsafe { EnumDisplayDevicesW(PCWSTR::null(), index, &mut device, 0) }.as_bool() {
            break;
        }
        if device.StateFlags.0 & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP.0 == 0 {
            continue;
        }
        adapters.push(Adapter {
            name: wide_text(&device.DeviceName),
            description: wide_text(&device.DeviceString),
            primary: device.StateFlags.0 & DISPLAY_DEVICE_PRIMARY_DEVICE.0 != 0,
        });
    }
    adapters
}

/// The active paths, one per desktop source (a mirrored source's further
/// targets are left out).
fn config_paths() -> Result<Vec<ConfigPath>, Error> {
    let operation = "read Windows displays";
    let mut path_count = 0u32;
    let mut mode_count = 0u32;
    // SAFETY: two counts out.
    let sized = unsafe {
        GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
    };
    if sized.is_err() {
        return Err(Error::new(
            operation,
            format!("GetDisplayConfigBufferSizes failed ({})", sized.0),
        ));
    }
    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
    // SAFETY: the buffers hold the counts passed with them.
    let queried = unsafe {
        QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut path_count,
            paths.as_mut_ptr(),
            &mut mode_count,
            modes.as_mut_ptr(),
            None,
        )
    };
    if queried.is_err() {
        return Err(Error::new(
            operation,
            format!("QueryDisplayConfig failed ({})", queried.0),
        ));
    }
    paths.truncate(path_count as usize);
    let mut result: Vec<ConfigPath> = Vec::new();
    for path in &paths {
        let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
        source.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
        source.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
        source.header.adapterId = path.sourceInfo.adapterId;
        source.header.id = path.sourceInfo.id;
        // SAFETY: the header names its own size and type.
        if unsafe { DisplayConfigGetDeviceInfo(&mut source.header) } != 0 {
            continue;
        }
        let gdi_name = wide_text(&source.viewGdiDeviceName);
        if gdi_name.is_empty() || result.iter().any(|known| known.gdi_name == gdi_name) {
            continue;
        }

        let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
        target.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
        target.header.size = std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
        target.header.adapterId = path.targetInfo.adapterId;
        target.header.id = path.targetInfo.id;
        // SAFETY: as above.
        let named = unsafe { DisplayConfigGetDeviceInfo(&mut target.header) } == 0;

        let mut preferred = DISPLAYCONFIG_TARGET_PREFERRED_MODE::default();
        preferred.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_PREFERRED_MODE;
        preferred.header.size = std::mem::size_of::<DISPLAYCONFIG_TARGET_PREFERRED_MODE>() as u32;
        preferred.header.adapterId = path.targetInfo.adapterId;
        preferred.header.id = path.targetInfo.id;
        // SAFETY: as above.
        let has_preferred = unsafe { DisplayConfigGetDeviceInfo(&mut preferred.header) } == 0;
        let vsync = preferred.targetMode.targetVideoSignalInfo.vSyncFreq;

        result.push(ConfigPath {
            gdi_name,
            friendly: if named {
                wide_text(&target.monitorFriendlyDeviceName)
            } else {
                String::new()
            },
            technology: path.targetInfo.outputTechnology,
            preferred: if has_preferred && preferred.width > 0 && vsync.Denominator > 0 {
                Some((
                    preferred.width,
                    preferred.height,
                    f64::from(vsync.Numerator) / f64::from(vsync.Denominator),
                ))
            } else {
                None
            },
        });
    }
    Ok(result)
}

#[cfg(test)]
mod win_tests {
    use super::*;

    fn rectangle(id: &str, x: i32, y: i32, width: u32, height: u32) -> Rectangle {
        Rectangle {
            id: id.into(),
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn arrangement_needs_shared_edges_and_no_overlap() {
        let main = rectangle("1", 0, 0, 1920, 1080);
        let right = rectangle("2", 1920, 0, 2560, 1440);
        assert!(check_arrangement(&[main.clone(), right.clone()], "test").is_ok());

        let gap = rectangle("2", 2000, 0, 2560, 1440);
        assert!(check_arrangement(&[main.clone(), gap], "test").is_err());

        let corner = rectangle("2", 1920, 1080, 800, 600);
        assert!(check_arrangement(&[main.clone(), corner], "test").is_err());

        let overlap = rectangle("2", 1000, 0, 1920, 1080);
        assert!(check_arrangement(&[main.clone(), overlap], "test").is_err());

        let above = rectangle("3", 0, -900, 1600, 900);
        assert!(check_arrangement(&[main, right, above], "test").is_ok());
    }

    #[test]
    fn orientations_round_trip_and_flips_are_refused() {
        for transform in [
            Transform::Normal,
            Transform::Rotate90,
            Transform::Rotate180,
            Transform::Rotate270,
        ] {
            let orientation = transform_orientation(&transform).unwrap();
            assert_eq!(orientation_transform(orientation), transform);
        }
        assert_eq!(transform_orientation(&Transform::Flipped), None);
        assert!(orientation_is_portrait(DMDO_90.0));
        assert!(!orientation_is_portrait(DMDO_180.0));
    }

    #[test]
    fn modes_are_deduplicated_oriented_and_keep_the_current_one() {
        let listed = vec![
            (1920, 1080, 60_000),
            (1920, 1080, 60_000),
            (1280, 720, 60_000),
            (1920, 1080, 144_000),
        ];
        let modes = unique_modes(listed.clone(), (1920, 1080, 60_000), false);
        assert_eq!(modes.len(), 3);
        assert_eq!(
            (modes[0].width, modes[0].height, modes[0].refresh_rate),
            (1920, 1080, 144_000)
        );

        let portrait = unique_modes(listed, (1080, 1920, 60_000), true);
        assert!(portrait.iter().all(|mode| mode.height >= mode.width));

        let unlisted = unique_modes(Vec::new(), (800, 600, 0), false);
        assert_eq!(unlisted.len(), 1);
        assert_eq!(refresh_millihertz(1), 0);
        assert_eq!(refresh_millihertz(60), 60_000);
    }

    #[test]
    fn preferred_mode_is_the_closest_native_timing() {
        let mut modes = unique_modes(
            vec![
                (1920, 1080, 59_000),
                (1920, 1080, 60_000),
                (1280, 720, 60_000),
            ],
            (1920, 1080, 60_000),
            false,
        );
        mark_preferred(&mut modes, 1920, 1080, 59.94);
        let preferred = modes
            .iter()
            .filter(|mode| mode.preferred)
            .collect::<Vec<_>>();
        assert_eq!(preferred.len(), 1);
        assert_eq!(preferred[0].refresh_rate, 60_000);
    }
}
