//! Platform-independent screenshot model: the wire commands, the persistent
//! Options menu state, file naming, selection geometry, and the macOS 26
//! layout measured in design-lab/screenshot.html.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

// ⇧⌘4 readout and drag selection.
pub const READOUT_SIZE: f32 = 9.0;
pub const READOUT_LINE: f32 = 9.0;
/// The readout's left edge sits 8.5 right of the hotspot; its first line box
/// starts 2 below it (cap top 3.5 below).
pub const READOUT_DX: f32 = 8.5;
pub const READOUT_DY: f32 = 2.0;
/// The white copy behind the black digits.
pub const READOUT_SHADOW_DX: f32 = 0.5;
pub const READOUT_SHADOW_DY: f32 = 1.0;
/// Drag fill: white ≈ 11.5 % (a 30 grey reads 56); 1 pt white edge inside.
pub const DRAG_FILL: u32 = 0xFFFF_FF1D;
pub const DRAG_EDGE: u32 = 0xFFFF_FFFF;
/// Window mode: black reads 88,107,129 and white 200,219,241.
pub const WINDOW_HIGHLIGHT: u32 = 0x92B8_E480;

// ⇧⌘5 toolbar (record group omitted: rmac has no screen recorder).
pub const TOOLBAR_WIDTH: f32 = 364.5;
pub const TOOLBAR_HEIGHT: f32 = 50.0;
/// Bottom edge above the screen bottom (7 above the 90 pt Dock).
pub const TOOLBAR_BOTTOM: f32 = 97.0;
pub const TOOLBAR_RADIUS: f32 = 15.0;
pub const TOOLBAR_TINT: u32 = 0x2929_31F5;
pub const TOOLBAR_GLYPH: u32 = 0xA9A9_ADFF;
pub const TOOLBAR_GLYPH_SELECTED: u32 = 0xB0B0_B3FF;
pub const TOOLBAR_PLATE: u32 = 0x3A3A_42FF;
pub const CLOSE_CENTER_X: f32 = 22.25;
pub const CLOSE_SIZE: f32 = 16.0;
/// Entire screen, window, selection.
pub const TARGET_CENTERS: [f32; 3] = [63.75, 111.75, 159.75];
pub const GLYPH_WIDTH: f32 = 28.0;
pub const GLYPH_HEIGHT: f32 = 22.0;
pub const PLATE_WIDTH: f32 = 44.0;
pub const PLATE_HEIGHT: f32 = 35.0;
pub const PLATE_RADIUS: f32 = 7.0;
pub const SEPARATOR_X: f32 = 190.0;
pub const SEPARATOR_TOP: f32 = 13.5;
pub const SEPARATOR_HEIGHT: f32 = 23.0;
pub const OPTIONS_X: f32 = 206.0;
pub const OPTIONS_TEXT_SIZE: f32 = 13.0;
/// The chevron glyph (5 × 4) starts 10 after the text; its 9 pt box has 2
/// of padding on each side.
pub const CHEVRON_BOX: f32 = 9.0;
pub const CHEVRON_GAP: f32 = 8.0;
pub const CAPTURE_WIDTH: f32 = 78.5;
pub const CAPTURE_HEIGHT: f32 = 36.0;
pub const CAPTURE_INSET: f32 = 7.0;
pub const CAPTURE_RADIUS: f32 = 10.0;
pub const CAPTURE_FILL: u32 = 0x1FAE_FFFF;

// ⇧⌘5 selection frame.
pub const DIM: u32 = 0x0000_0080;
pub const DASH: f32 = 4.0;
pub const HANDLE_SIZE: f32 = 8.0;
pub const HANDLE_FILL: u32 = 0x9292_92FF;
/// How far from a handle or edge a press still grabs it.
pub const HANDLE_REACH: f32 = 6.0;

// Options menu (app-menu material).
pub const MENU_WIDTH: f32 = 196.0;
pub const MENU_RADIUS: f32 = 13.0;
pub const MENU_PADDING: f32 = 5.0;
pub const MENU_ROW: f32 = 24.0;
pub const MENU_HEADER: f32 = 24.0;
pub const MENU_SEPARATOR: f32 = 11.0;
pub const MENU_TEXT_X: f32 = 25.5;
pub const MENU_HEADER_X: f32 = 25.0;
pub const MENU_CHECK_CENTER_X: f32 = 15.75;
pub const MENU_SEPARATOR_INSET: f32 = 16.5;
/// Menu left edge relative to the "Options" text, and its bottom edge
/// relative to the toolbar's top edge (it covers the button, as on the Mac).
pub const MENU_LEFT_FROM_OPTIONS: f32 = -10.5;
pub const MENU_BOTTOM_FROM_TOOLBAR_TOP: f32 = 27.0;

// Floating thumbnail.
pub const THUMB_IMAGE_WIDTH: f32 = 147.0;
pub const THUMB_BORDER: f32 = 4.0;
pub const THUMB_RADIUS: f32 = 3.0;
pub const THUMB_RIGHT: f32 = 12.0;
pub const THUMB_BOTTOM: f32 = 102.0;
/// From capture to the start of the slide out (measured ≈ 5.2 s).
pub const THUMB_HOLD_MS: u64 = 5_000;
pub const THUMB_SLIDE_MS: u64 = 300;
/// A swipe right further than this saves at once.
pub const THUMB_SWIPE: f32 = 40.0;

/// Time between removing the overlay and reading the screen, so the
/// compositor has presented a frame without it.
pub const SETTLE_MS: u64 = 120;

/// One word from the niri binds to the resident service.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Command {
    /// ⇧⌘3
    Screen,
    /// ⌃⇧⌘3
    ScreenToClipboard,
    /// ⇧⌘4
    Selection,
    /// ⌃⇧⌘4
    SelectionToClipboard,
    /// ⇧⌘5
    Toolbar,
    Cancel,
}

impl Command {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "screen" => Self::Screen,
            "screen-to-clipboard" => Self::ScreenToClipboard,
            "selection" => Self::Selection,
            "selection-to-clipboard" => Self::SelectionToClipboard,
            "toolbar" => Self::Toolbar,
            "cancel" => Self::Cancel,
            _ => return None,
        })
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Screen => "screen",
            Self::ScreenToClipboard => "screen-to-clipboard",
            Self::Selection => "selection",
            Self::SelectionToClipboard => "selection-to-clipboard",
            Self::Toolbar => "toolbar",
            Self::Cancel => "cancel",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Destination {
    Desktop,
    Documents,
    Downloads,
    Clipboard,
}

impl Destination {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Desktop => "Desktop",
            Self::Documents => "Documents",
            Self::Downloads => "Downloads",
            Self::Clipboard => "Clipboard",
        }
    }

    const fn key(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Documents => "documents",
            Self::Downloads => "downloads",
            Self::Clipboard => "clipboard",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        [
            Self::Desktop,
            Self::Documents,
            Self::Downloads,
            Self::Clipboard,
        ]
        .into_iter()
        .find(|destination| destination.key() == value)
    }

    /// The `user-dirs.dirs` key and the fallback folder under `$HOME`.
    pub const fn user_dir(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::Desktop => Some(("XDG_DESKTOP_DIR", "Desktop")),
            Self::Documents => Some(("XDG_DOCUMENTS_DIR", "Documents")),
            Self::Downloads => Some(("XDG_DOWNLOAD_DIR", "Downloads")),
            Self::Clipboard => None,
        }
    }
}

/// What ⇧⌘5 captures; ⇧⌘4 uses Selection and Window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Target {
    Screen,
    Window,
    Selection,
}

impl Target {
    pub const ALL: [Self; 3] = [Self::Screen, Self::Window, Self::Selection];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Screen => "Capture Entire Screen",
            Self::Window => "Capture Selected Window",
            Self::Selection => "Capture Selected Portion",
        }
    }

    pub const fn glyph(self) -> &'static str {
        match self {
            Self::Screen => "screenshot/capture-screen.svg",
            Self::Window => "screenshot/capture-window.svg",
            Self::Selection => "screenshot/capture-selection.svg",
        }
    }

    const fn key(self) -> &'static str {
        match self {
            Self::Screen => "screen",
            Self::Window => "window",
            Self::Selection => "selection",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|target| target.key() == value)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// A rectangle in output-local logical points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn from_corners(a: Point, b: Point) -> Self {
        Self {
            x: a.x.min(b.x),
            y: a.y.min(b.y),
            width: (a.x - b.x).abs(),
            height: (a.y - b.y).abs(),
        }
    }

    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    pub fn contains(&self, point: Point) -> bool {
        point.x >= self.x && point.x < self.right() && point.y >= self.y && point.y < self.bottom()
    }

    /// Too small to be a capture: a click, not a drag.
    pub fn is_empty(&self) -> bool {
        self.width < 1.0 || self.height < 1.0
    }

    /// The part of `self` inside a `width × height` output.
    pub fn clipped(&self, width: f32, height: f32) -> Self {
        let x = self.x.clamp(0.0, width);
        let y = self.y.clamp(0.0, height);
        Self {
            x,
            y,
            width: (self.right().min(width) - x).max(0.0),
            height: (self.bottom().min(height) - y).max(0.0),
        }
    }

    /// Moved by `dx, dy` but kept whole inside a `width × height` output.
    pub fn moved_within(&self, dx: f32, dy: f32, width: f32, height: f32) -> Self {
        Self {
            x: (self.x + dx).clamp(0.0, (width - self.width).max(0.0)),
            y: (self.y + dy).clamp(0.0, (height - self.height).max(0.0)),
            ..*self
        }
    }
}

/// The two readout lines beside the crosshair: the pointer position while
/// idle, the selection size while dragging. A 300 pt drag reads 301 on the
/// Mac, so both ends count.
pub fn readout(pointer: Point, drag_start: Option<Point>) -> (String, String) {
    match drag_start {
        Some(start) => (
            format!("{}", ((pointer.x - start.x).abs().floor() as i64) + 1),
            format!("{}", ((pointer.y - start.y).abs().floor() as i64) + 1),
        ),
        None => (
            format!("{}", pointer.x.max(0.0).floor() as i64),
            format!("{}", pointer.y.max(0.0).floor() as i64),
        ),
    }
}

/// One of the eight selection handles.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Handle {
    TopLeft,
    Top,
    TopRight,
    Left,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Handle {
    pub const ALL: [Self; 8] = [
        Self::TopLeft,
        Self::Top,
        Self::TopRight,
        Self::Left,
        Self::Right,
        Self::BottomLeft,
        Self::Bottom,
        Self::BottomRight,
    ];

    /// Fractions of the width and height where the handle sits.
    pub const fn anchor(self) -> (f32, f32) {
        match self {
            Self::TopLeft => (0.0, 0.0),
            Self::Top => (0.5, 0.0),
            Self::TopRight => (1.0, 0.0),
            Self::Left => (0.0, 0.5),
            Self::Right => (1.0, 0.5),
            Self::BottomLeft => (0.0, 1.0),
            Self::Bottom => (0.5, 1.0),
            Self::BottomRight => (1.0, 1.0),
        }
    }

    pub fn center(self, rect: &Rect) -> Point {
        let (fx, fy) = self.anchor();
        Point::new(rect.x + rect.width * fx, rect.y + rect.height * fy)
    }

    const fn moves_left(self) -> bool {
        matches!(self, Self::TopLeft | Self::Left | Self::BottomLeft)
    }

    const fn moves_right(self) -> bool {
        matches!(self, Self::TopRight | Self::Right | Self::BottomRight)
    }

    const fn moves_top(self) -> bool {
        matches!(self, Self::TopLeft | Self::Top | Self::TopRight)
    }

    const fn moves_bottom(self) -> bool {
        matches!(self, Self::BottomLeft | Self::Bottom | Self::BottomRight)
    }
}

/// The handle (or edge) a press at `point` grabs: handles first, then the
/// nearest edge within reach.
pub fn handle_at(rect: &Rect, point: Point) -> Option<Handle> {
    let near = |a: f32, b: f32| (a - b).abs() <= HANDLE_REACH;
    if let Some(handle) = Handle::ALL.into_iter().find(|handle| {
        let center = handle.center(rect);
        near(center.x, point.x) && near(center.y, point.y)
    }) {
        return Some(handle);
    }
    let within_x = point.x >= rect.x - HANDLE_REACH && point.x <= rect.right() + HANDLE_REACH;
    let within_y = point.y >= rect.y - HANDLE_REACH && point.y <= rect.bottom() + HANDLE_REACH;
    if within_y && near(point.x, rect.x) {
        Some(Handle::Left)
    } else if within_y && near(point.x, rect.right()) {
        Some(Handle::Right)
    } else if within_x && near(point.y, rect.y) {
        Some(Handle::Top)
    } else if within_x && near(point.y, rect.bottom()) {
        Some(Handle::Bottom)
    } else {
        None
    }
}

/// `original` resized by dragging `handle` by `dx, dy`, normalised when an
/// edge crosses its opposite and kept inside the output.
pub fn resized(original: &Rect, handle: Handle, dx: f32, dy: f32, width: f32, height: f32) -> Rect {
    let mut left = original.x;
    let mut right = original.right();
    let mut top = original.y;
    let mut bottom = original.bottom();
    if handle.moves_left() {
        left += dx;
    }
    if handle.moves_right() {
        right += dx;
    }
    if handle.moves_top() {
        top += dy;
    }
    if handle.moves_bottom() {
        bottom += dy;
    }
    Rect::from_corners(Point::new(left, top), Point::new(right, bottom)).clipped(width, height)
}

/// ⇧⌘5's selection when none is remembered: the middle half of the output.
pub fn default_selection(width: f32, height: f32) -> Rect {
    Rect::new(width / 4.0, height / 4.0, width / 2.0, height / 2.0)
}

/// Index of the topmost window under `point`; `windows` is topmost-first.
pub fn window_at(windows: &[Rect], point: Point) -> Option<usize> {
    windows.iter().position(|rect| rect.contains(point))
}

/// Toolbar origin on a `width × height` output.
pub fn toolbar_origin(width: f32, height: f32) -> Point {
    Point::new(
        (width - TOOLBAR_WIDTH) / 2.0,
        height - TOOLBAR_BOTTOM - TOOLBAR_HEIGHT,
    )
}

/// The persistent Options menu state.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub destination: Destination,
    pub timer_seconds: u8,
    pub show_thumbnail: bool,
    pub remember_selection: bool,
    pub show_pointer: bool,
    pub target: Target,
    pub last_selection: Option<Rect>,
}

impl Default for Settings {
    /// macOS defaults: Desktop, no timer, thumbnail on, remember the last
    /// selection, no pointer, "Capture Selected Portion".
    fn default() -> Self {
        Self {
            destination: Destination::Desktop,
            timer_seconds: 0,
            show_thumbnail: true,
            remember_selection: true,
            show_pointer: false,
            target: Target::Selection,
            last_selection: None,
        }
    }
}

impl Settings {
    /// Lenient `key=value` lines: unknown keys and bad values keep defaults.
    pub fn parse(text: &str) -> Self {
        let mut settings = Self::default();
        let flag = |value: &str| match value {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        };
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "destination" => {
                    if let Some(destination) = Destination::parse(value) {
                        settings.destination = destination;
                    }
                }
                "timer" => {
                    if let Ok(seconds @ (0 | 5 | 10)) = value.parse::<u8>() {
                        settings.timer_seconds = seconds;
                    }
                }
                "thumbnail" => {
                    if let Some(on) = flag(value) {
                        settings.show_thumbnail = on;
                    }
                }
                "remember-selection" => {
                    if let Some(on) = flag(value) {
                        settings.remember_selection = on;
                    }
                }
                "pointer" => {
                    if let Some(on) = flag(value) {
                        settings.show_pointer = on;
                    }
                }
                "target" => {
                    if let Some(target) = Target::parse(value) {
                        settings.target = target;
                    }
                }
                "selection" => {
                    let parts = value
                        .split(',')
                        .map(|part| part.trim().parse::<f32>())
                        .collect::<Result<Vec<_>, _>>();
                    if let Ok(parts) = parts {
                        if let [x, y, width, height] = parts[..] {
                            let rect = Rect::new(x, y, width, height);
                            if parts.iter().all(|value| value.is_finite()) && !rect.is_empty() {
                                settings.last_selection = Some(rect);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        settings
    }

    pub fn serialize(&self) -> String {
        let mut text = format!(
            "destination={}\ntimer={}\nthumbnail={}\nremember-selection={}\npointer={}\ntarget={}\n",
            self.destination.key(),
            self.timer_seconds,
            self.show_thumbnail,
            self.remember_selection,
            self.show_pointer,
            self.target.key(),
        );
        if let Some(rect) = self.last_selection {
            text.push_str(&format!(
                "selection={},{},{},{}\n",
                rect.x, rect.y, rect.width, rect.height
            ));
        }
        text
    }

    pub fn load() -> Self {
        settings_path()
            .and_then(|path| fs::read_to_string(path).ok())
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    pub fn save(&self) -> io::Result<()> {
        let path = settings_path()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no configuration directory"))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("conf.tmp");
        fs::write(&temporary, self.serialize())?;
        fs::rename(temporary, path)
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

fn config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| home().map(|home| home.join(".config")))
}

fn settings_path() -> Option<PathBuf> {
    config_home().map(|config| config.join("rmac/screenshot.conf"))
}

/// The folder a file destination saves into, from `user-dirs.dirs` like
/// Files and the Desktop, falling back to `$HOME/<Name>`.
pub fn destination_directory(destination: Destination) -> Option<PathBuf> {
    let (key, fallback) = destination.user_dir()?;
    let home = home()?;
    let user_dirs =
        config_home().and_then(|config| fs::read_to_string(config.join("user-dirs.dirs")).ok());
    Some(resolve_user_dir(&home, user_dirs.as_deref(), key, fallback))
}

/// `XDG_*_DIR="$HOME/…"` or an absolute path; anything else (other
/// variables, command substitution, relative paths) uses the fallback.
pub fn resolve_user_dir(
    home: &Path,
    user_dirs: Option<&str>,
    key: &str,
    fallback: &str,
) -> PathBuf {
    let prefix = format!("{key}=");
    let value = user_dirs.and_then(|contents| {
        contents.lines().find_map(|line| {
            let line = line.trim();
            (!line.starts_with('#'))
                .then(|| line.strip_prefix(prefix.as_str()))
                .flatten()
        })
    });
    let parsed = value.and_then(|raw| {
        let value = raw.trim().strip_prefix('"')?.strip_suffix('"')?;
        let path = if let Some(rest) = value
            .strip_prefix("$HOME")
            .or_else(|| value.strip_prefix("${HOME}"))
        {
            let rest = rest.strip_prefix('/').unwrap_or(rest);
            if rest.contains('$') || rest.contains('`') || rest.split('/').any(|part| part == "..")
            {
                return None;
            }
            if rest.is_empty() {
                home.to_path_buf()
            } else {
                home.join(rest)
            }
        } else {
            if value.contains('$') || value.contains('`') {
                return None;
            }
            PathBuf::from(value)
        };
        path.is_absolute().then_some(path)
    });
    parsed.unwrap_or_else(|| home.join(fallback))
}

/// "Screenshot 2026-09-23 at 1.36.39 PM.png": a 12-hour clock without a
/// leading zero and U+202F before AM/PM, exactly as macOS 26 names it.
pub fn file_name(time: &chrono::NaiveDateTime) -> String {
    format!(
        "Screenshot {} at {}\u{202F}{}.png",
        time.format("%Y-%m-%d"),
        time.format("%-I.%M.%S"),
        time.format("%p"),
    )
}

/// `directory/name`, or "name (2).png", "name (3).png"… when taken.
pub fn unique_path(directory: &Path, name: &str, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let candidate = directory.join(name);
    if !exists(&candidate) {
        return candidate;
    }
    let stem = name.strip_suffix(".png").unwrap_or(name);
    (2..)
        .map(|index| directory.join(format!("{stem} ({index}).png")))
        .find(|path| !exists(path))
        .expect("an unused numbered name exists")
}

/// Thumbnail frame size for a capture of `width × height` points: the image
/// is 147 wide (95.5 tall for the 1470 × 956 screen) inside a 4 pt border.
/// Tall captures are fitted into a 147 square instead (not measured).
pub fn thumbnail_size(width: f32, height: f32) -> (f32, f32) {
    let (width, height) = (width.max(1.0), height.max(1.0));
    let scale = (THUMB_IMAGE_WIDTH / width).min(THUMB_IMAGE_WIDTH / height);
    (
        width * scale + 2.0 * THUMB_BORDER,
        height * scale + 2.0 * THUMB_BORDER,
    )
}

/// Ease-out cubic, `t` in 0…1.
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// One row of the Options menu.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuItem {
    Destination(Destination),
    Timer(u8),
    ShowThumbnail,
    RememberSelection,
    ShowPointer,
}

impl MenuItem {
    pub fn label(self) -> &'static str {
        match self {
            Self::Destination(destination) => destination.label(),
            Self::Timer(0) => "None",
            Self::Timer(5) => "5 Seconds",
            Self::Timer(_) => "10 Seconds",
            Self::ShowThumbnail => "Show Floating Thumbnail",
            Self::RememberSelection => "Remember Last Selection",
            Self::ShowPointer => "Show Mouse Pointer",
        }
    }

    pub fn checked(self, settings: &Settings) -> bool {
        match self {
            Self::Destination(destination) => settings.destination == destination,
            Self::Timer(seconds) => settings.timer_seconds == seconds,
            Self::ShowThumbnail => settings.show_thumbnail,
            Self::RememberSelection => settings.remember_selection,
            Self::ShowPointer => settings.show_pointer,
        }
    }

    pub fn apply(self, settings: &mut Settings) {
        match self {
            Self::Destination(destination) => settings.destination = destination,
            Self::Timer(seconds) => settings.timer_seconds = seconds,
            Self::ShowThumbnail => settings.show_thumbnail = !settings.show_thumbnail,
            Self::RememberSelection => settings.remember_selection = !settings.remember_selection,
            Self::ShowPointer => settings.show_pointer = !settings.show_pointer,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuEntry {
    Header(&'static str),
    Item(MenuItem),
    Separator,
}

impl MenuEntry {
    pub const fn height(self) -> f32 {
        match self {
            Self::Header(_) => MENU_HEADER,
            Self::Item(_) => MENU_ROW,
            Self::Separator => MENU_SEPARATOR,
        }
    }
}

/// The Mac's sections, keeping only destinations rmac can deliver to (Mail,
/// Preview and Other Location… have no backend). Clipboard needs wl-copy.
pub fn menu_entries(clipboard: bool) -> Vec<MenuEntry> {
    let mut entries = vec![
        MenuEntry::Header("Save to"),
        MenuEntry::Item(MenuItem::Destination(Destination::Desktop)),
        MenuEntry::Item(MenuItem::Destination(Destination::Documents)),
        MenuEntry::Item(MenuItem::Destination(Destination::Downloads)),
    ];
    if clipboard {
        entries.push(MenuEntry::Item(MenuItem::Destination(
            Destination::Clipboard,
        )));
    }
    entries.extend([
        MenuEntry::Separator,
        MenuEntry::Header("Timer"),
        MenuEntry::Item(MenuItem::Timer(0)),
        MenuEntry::Item(MenuItem::Timer(5)),
        MenuEntry::Item(MenuItem::Timer(10)),
        MenuEntry::Separator,
        MenuEntry::Header("Options"),
        MenuEntry::Item(MenuItem::ShowThumbnail),
        MenuEntry::Item(MenuItem::RememberSelection),
        MenuEntry::Separator,
        MenuEntry::Header("Capture"),
        MenuEntry::Item(MenuItem::ShowPointer),
    ]);
    entries
}

pub fn menu_height(entries: &[MenuEntry]) -> f32 {
    2.0 * MENU_PADDING + entries.iter().map(|entry| entry.height()).sum::<f32>()
}

/// Menu origin for a toolbar at `toolbar`.
pub fn menu_origin(toolbar: Point, entries: &[MenuEntry]) -> Point {
    Point::new(
        toolbar.x + OPTIONS_X + MENU_LEFT_FROM_OPTIONS,
        toolbar.y + MENU_BOTTOM_FROM_TOOLBAR_TOP - menu_height(entries),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_round_trip_and_reject_other_words() {
        for command in [
            Command::Screen,
            Command::ScreenToClipboard,
            Command::Selection,
            Command::SelectionToClipboard,
            Command::Toolbar,
            Command::Cancel,
        ] {
            assert_eq!(Command::parse(command.as_str()), Some(command));
        }
        assert_eq!(Command::parse("screen\n"), None);
        assert_eq!(Command::parse("record"), None);
    }

    #[test]
    fn file_names_match_macos() {
        let time = chrono::NaiveDate::from_ymd_opt(2026, 9, 23)
            .unwrap()
            .and_hms_opt(13, 36, 39)
            .unwrap();
        assert_eq!(
            file_name(&time),
            "Screenshot 2026-09-23 at 1.36.39\u{202F}PM.png"
        );
        let morning = chrono::NaiveDate::from_ymd_opt(2026, 9, 22)
            .unwrap()
            .and_hms_opt(0, 5, 7)
            .unwrap();
        assert_eq!(
            file_name(&morning),
            "Screenshot 2026-09-22 at 12.05.07\u{202F}AM.png"
        );
    }

    #[test]
    fn taken_names_get_a_number() {
        let taken = [PathBuf::from("/d/a.png"), PathBuf::from("/d/a (2).png")];
        let path = unique_path(Path::new("/d"), "a.png", |path| {
            taken.iter().any(|t| t == path)
        });
        assert_eq!(path, PathBuf::from("/d/a (3).png"));
        assert_eq!(
            unique_path(Path::new("/d"), "b.png", |_| false),
            PathBuf::from("/d/b.png")
        );
    }

    #[test]
    fn readout_shows_position_then_inclusive_size() {
        assert_eq!(
            readout(Point::new(705.4, 405.9), None),
            ("705".into(), "405".into())
        );
        let start = Point::new(500.0, 300.0);
        assert_eq!(
            readout(Point::new(800.0, 500.0), Some(start)),
            ("301".into(), "201".into())
        );
        assert_eq!(
            readout(Point::new(400.0, 300.0), Some(start)),
            ("101".into(), "1".into())
        );
    }

    #[test]
    fn settings_round_trip_and_tolerate_damage() {
        let settings = Settings {
            destination: Destination::Downloads,
            timer_seconds: 5,
            show_thumbnail: false,
            remember_selection: true,
            show_pointer: true,
            target: Target::Window,
            last_selection: Some(Rect::new(0.0, 152.5, 1467.0, 712.0)),
        };
        assert_eq!(Settings::parse(&settings.serialize()), settings);
        let damaged = Settings::parse("timer=7\ndestination=mail\nselection=1,2,x,4\nnonsense\n");
        assert_eq!(damaged, Settings::default());
    }

    #[test]
    fn user_dirs_follow_xdg_and_refuse_expansion() {
        let home = Path::new("/home/me");
        let dirs = "# c\nXDG_DESKTOP_DIR=\"$HOME/Schreibtisch\"\nXDG_DOCUMENTS_DIR=\"$OTHER/x\"\nXDG_DOWNLOAD_DIR=\"/data/dl\"\n";
        assert_eq!(
            resolve_user_dir(home, Some(dirs), "XDG_DESKTOP_DIR", "Desktop"),
            PathBuf::from("/home/me/Schreibtisch")
        );
        assert_eq!(
            resolve_user_dir(home, Some(dirs), "XDG_DOCUMENTS_DIR", "Documents"),
            PathBuf::from("/home/me/Documents")
        );
        assert_eq!(
            resolve_user_dir(home, Some(dirs), "XDG_DOWNLOAD_DIR", "Downloads"),
            PathBuf::from("/data/dl")
        );
        assert_eq!(
            resolve_user_dir(home, None, "XDG_DESKTOP_DIR", "Desktop"),
            PathBuf::from("/home/me/Desktop")
        );
    }

    #[test]
    fn handles_grab_corners_then_edges() {
        let rect = Rect::new(100.0, 100.0, 200.0, 100.0);
        assert_eq!(
            handle_at(&rect, Point::new(103.0, 97.0)),
            Some(Handle::TopLeft)
        );
        assert_eq!(
            handle_at(&rect, Point::new(200.0, 200.0)),
            Some(Handle::Bottom)
        );
        assert_eq!(
            handle_at(&rect, Point::new(150.0, 102.0)),
            Some(Handle::Top)
        );
        assert_eq!(
            handle_at(&rect, Point::new(299.0, 130.0)),
            Some(Handle::Right)
        );
        assert_eq!(handle_at(&rect, Point::new(200.0, 150.0)), None);
    }

    #[test]
    fn resizing_normalises_and_stays_on_the_output() {
        let rect = Rect::new(100.0, 100.0, 200.0, 100.0);
        assert_eq!(
            resized(&rect, Handle::Left, 250.0, 0.0, 1470.0, 956.0),
            Rect::new(300.0, 100.0, 50.0, 100.0)
        );
        assert_eq!(
            resized(&rect, Handle::BottomRight, 2000.0, 2000.0, 1470.0, 956.0),
            Rect::new(100.0, 100.0, 1370.0, 856.0)
        );
        assert_eq!(
            rect.moved_within(-500.0, 10.0, 1470.0, 956.0),
            Rect::new(0.0, 110.0, 200.0, 100.0)
        );
    }

    #[test]
    fn topmost_window_wins() {
        let windows = [
            Rect::new(0.0, 0.0, 100.0, 100.0),
            Rect::new(0.0, 0.0, 500.0, 500.0),
        ];
        assert_eq!(window_at(&windows, Point::new(50.0, 50.0)), Some(0));
        assert_eq!(window_at(&windows, Point::new(200.0, 50.0)), Some(1));
        assert_eq!(window_at(&windows, Point::new(600.0, 50.0)), None);
    }

    #[test]
    fn measured_layout_holds() {
        let toolbar = toolbar_origin(1470.0, 956.0);
        assert_eq!(toolbar.y + TOOLBAR_HEIGHT, 859.0);
        assert_eq!(
            CAPTURE_INSET + CAPTURE_HEIGHT + CAPTURE_INSET,
            TOOLBAR_HEIGHT
        );
        let entries = menu_entries(true);
        assert_eq!(
            menu_height(&entries),
            5.0 + 4.0 * 24.0 + 10.0 * 24.0 + 3.0 * 11.0 + 5.0
        );
        let menu = menu_origin(toolbar, &entries);
        assert_eq!(menu.y + menu_height(&entries), toolbar.y + 27.0);
        assert!(
            !menu_entries(false).contains(&MenuEntry::Item(MenuItem::Destination(
                Destination::Clipboard
            )))
        );
        let (width, height) = thumbnail_size(1470.0, 956.0);
        assert_eq!(width, 155.0);
        assert!((height - 103.6).abs() < 0.1);
    }

    #[test]
    fn menu_items_toggle_and_select() {
        let mut settings = Settings::default();
        MenuItem::Timer(10).apply(&mut settings);
        MenuItem::ShowThumbnail.apply(&mut settings);
        MenuItem::Destination(Destination::Documents).apply(&mut settings);
        assert!(MenuItem::Timer(10).checked(&settings));
        assert!(!MenuItem::Timer(0).checked(&settings));
        assert!(!settings.show_thumbnail);
        assert_eq!(settings.destination, Destination::Documents);
    }
}
