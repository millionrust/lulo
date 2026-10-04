//! Pure menu geometry and status-menu models, measured on macOS 26
//! (design-lab/menus.html has the numbers and where each came from). Kept
//! free of GPUI so it is unit tested on every host.

use std::time::Duration;

use rmac_app_menu::Item;
use rmac_network::{NetworkDevice, WifiNetwork, WifiNetworkId, WifiSecurity, WifiSnapshot};

// ---- App menus (rmac menu, app menu, exported menus) ----

/// A dropdown hangs one point below the bar.
pub const MENU_TOP_GAP: f32 = 1.0;
/// Title menus open 4 pt left of the title's item frame, which sits 1 pt
/// inside rmac's highlight slot.
pub const TITLE_MENU_OFFSET: f32 = -3.0;
/// The rmac (Apple) menu opens 4 pt left of the logo slot.
pub const LOGO_MENU_OFFSET: f32 = -4.0;
pub const APP_MENU_RADIUS: f32 = 12.0;
pub const APP_MENU_PADDING: f32 = 5.0;
pub const APP_ROW_HEIGHT: f32 = 24.0;
/// 5 pt · 1 pt line · 5 pt.
pub const APP_SEPARATOR_HEIGHT: f32 = 11.0;
pub const APP_SEPARATOR_INSET: f32 = 16.0;
/// Highlight inset from the panel edge.
pub const ROW_INSET: f32 = 5.0;
/// Text column without icons.
pub const APP_TEXT_INSET: f32 = 16.5;
/// Icon glyphs centre here; text follows at [`APP_ICON_TEXT`].
pub const APP_ICON_CENTRE: f32 = 24.5;
pub const APP_ICON_TEXT: f32 = 39.0;
/// The Apple menu's laptop glyph is 14.5 wide, which pushes its text column.
pub const APP_WIDE_ICON_CENTRE: f32 = 25.0;
pub const APP_WIDE_ICON_TEXT: f32 = 41.5;
/// Icons are drawn in a 16 pt box.
pub const MENU_ICON_BOX: f32 = 16.0;
/// Modifier glyphs sit centred in 13.75 pt cells; the key itself is left
/// aligned 26 pt from the right edge (a 12 pt cell ending 14 pt in).
pub const KEY_CELL: f32 = 13.75;
pub const KEY_LETTER_GAP: f32 = 3.1;
pub const KEY_LETTER_WIDTH: f32 = 12.0;
pub const KEY_RIGHT: f32 = 14.0;
/// Submenu chevron: 5 × 9 glyph ending 17 pt from the right edge.
pub const CHEVRON_RIGHT: f32 = 17.0;
pub const CHEVRON_WIDTH: f32 = 5.0;
/// Minimum space between a title and its shortcut or chevron.
pub const SHORTCUT_GAP: f32 = 24.0;
/// A count capsule ("1 update") sits this far after its title.
pub const BADGE_GAP: f32 = 24.0;
/// The capsule's padding (8.5 each side) and its 11 pt text against the
/// 13 pt menu text `label_width` measures.
const BADGE_PADDING: f32 = 17.0;
const BADGE_TEXT_SCALE: f32 = 11.0 / 13.0;
/// Exported and synthesized menus mark a submenu with this shortcut.
pub const SUBMENU_MARK: &str = "›";
const MODIFIERS: [char; 5] = ['⌃', '⌥', '⇧', '⌘', '🌐'];

/// A checkmark column adds 7.5 to the text and icon columns; the 9 × 8.5
/// checkmark is centred 14 from the panel edge (design-lab/menus.html).
pub const CHECK_COLUMN: f32 = 7.5;
pub const CHECK_CENTRE: f32 = 14.0;
pub const CHECK_WIDTH: f32 = 9.0;
/// The Help menu's search row: 40 tall, a 25 pt capsule 9.5 in from the
/// top and sides (design-lab/menus.html).
pub const HELP_SEARCH_ROW_HEIGHT: f32 = 40.0;
pub const HELP_SEARCH_CAPSULE_HEIGHT: f32 = 25.0;
pub const HELP_SEARCH_INSET: f32 = 9.5;
/// The synthesized Help menu's search field row. It is never activated.
pub const HELP_SEARCH_ACTION: &str = "help::search";
/// The synthesized Help menu's "<App> Help" row opens that app's bundled
/// help page; its key equivalent follows that app's recorded menu.
pub const APP_HELP_ACTION: &str = "help::app-help";
/// The synthesized Window menu's "Minimise All" row (⌥⌘M). It runs through
/// `dispatch_app_menu_action` exactly like Hide App does (§2.2's parking
/// model), since minimising every window of the app is the same operation.
pub const MINIMISE_ALL_ACTION: &str = "app::minimise-all";
/// A submenu overlaps its parent by this much (as Recent Items does).
pub const SUBMENU_OVERLAP: f32 = 4.0;
/// How long the pointer rests on a submenu row before the submenu opens,
/// and on another row before an open one closes. S: not yet measured on
/// the Mac (FEEL_SPEC.md lists the submenu capture as still to do); AppKit
/// waits a moment so a pointer passing over rows does not flash menus.
pub const SUBMENU_HOVER_DELAY: Duration = Duration::from_millis(100);

fn row_height(item: &Item) -> f32 {
    if item.action == HELP_SEARCH_ACTION {
        HELP_SEARCH_ROW_HEIGHT
    } else {
        APP_ROW_HEIGHT
    }
}

pub fn app_menu_height(items: &[Item]) -> f32 {
    let separators = items.iter().filter(|item| item.separator_before).count() as f32;
    2.0 * APP_MENU_PADDING
        + items.iter().map(row_height).sum::<f32>()
        + APP_SEPARATOR_HEIGHT * separators
}

/// Top of item `index` measured from the panel's top edge.
pub fn app_menu_item_top(items: &[Item], index: usize) -> f32 {
    let separators = items
        .iter()
        .take(index + 1)
        .filter(|item| item.separator_before)
        .count() as f32;
    APP_MENU_PADDING
        + items.iter().take(index).map(row_height).sum::<f32>()
        + APP_SEPARATOR_HEIGHT * separators
}

/// Whether a menu needs the checkmark column.
pub fn has_checks(items: &[Item]) -> bool {
    items
        .iter()
        .any(|item| item.checked != rmac_app_menu::CheckState::Off)
}

/// Whether a row opens a submenu: exported submenus carry their items,
/// the system menu's Recent Items is marked with [`SUBMENU_MARK`].
pub fn opens_submenu(item: &Item) -> bool {
    item.is_submenu() || item.shortcut == SUBMENU_MARK
}

/// Where a submenu opens: beside its parent row, its first row level with
/// the parent row, on the right unless that runs off the screen, and moved
/// up rather than past `max_bottom`.
pub fn submenu_origin(
    parent_left: f32,
    parent_width: f32,
    parent_row_top: f32,
    size: (f32, f32),
    screen_width: f32,
    max_bottom: f32,
) -> (f32, f32) {
    let (width, height) = size;
    let right = parent_left + parent_width - SUBMENU_OVERLAP;
    let left = if right + width > screen_width - 4.0 {
        (parent_left - width + SUBMENU_OVERLAP).max(4.0)
    } else {
        right
    };
    let top = (parent_row_top - APP_MENU_PADDING).min(max_bottom - height);
    (left, top.max(0.0))
}

/// The next enabled row after (or before) `from`, wrapping around, skipping
/// the Help menu's search field. `None` when no row is enabled.
pub fn next_enabled_row(items: &[Item], from: Option<usize>, forward: bool) -> Option<usize> {
    let count = items.len();
    if count == 0 {
        return None;
    }
    let selectable =
        |index: usize| items[index].enabled && items[index].action != HELP_SEARCH_ACTION;
    let start = match (from, forward) {
        (None, true) => count - 1,
        (None, false) => 0,
        (Some(index), _) => index.min(count - 1),
    };
    (1..=count)
        .map(|step| {
            if forward {
                (start + step) % count
            } else {
                (start + count - step % count) % count
            }
        })
        .find(|&index| selectable(index))
}

// ---- The standard Window and Help menus ----

/// One of the focused app's windows, for the Window menu's list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuWindow {
    pub id: rmac_compositor::WindowId,
    pub title: String,
    /// Minimised: parked on the hidden workspace.
    pub parked: bool,
}

/// What a Window-menu row does. The menu bar runs these itself, so every
/// app, first-party or not, has them: minimising and focusing through the
/// compositor, sizing through Mission Control's one-word commands (the
/// path its keyboard shortcuts take), which also remember the size to
/// return to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowCommand {
    Minimise,
    Zoom,
    /// Window ▸ Zoom All: fill every window of the app, the way Minimise
    /// All minimises every one of them.
    ZoomAll,
    Fill,
    Centre,
    Tile(WindowRegion),
    /// Window ▸ Move & Resize's "Arrange" group (WIN-01): tile the window
    /// the menu was opened over into `.0` and, when the app has a second
    /// window, tile that one into `.1`. The Mac's "… & Quarters" variants
    /// offer a two-more-window quarter picker; Lulo places just the one
    /// other window into the first of those quarters as a simpler,
    /// still-real stand-in (documented at its call site).
    ComboTile(WindowRegion, WindowRegion),
    ReturnToPreviousSize,
    /// Window ▸ Full-Screen Tile's "Left of Screen"/"Right of Screen":
    /// macOS puts the window into a full-screen Space split with another
    /// app's window. Lulo has no multi-app full-screen Space yet, so this
    /// runs the equivalent non-full-screen half tile (WIN-01).
    FullScreenTileSide(WindowRegion),
    /// Window ▸ Full-Screen Tile itself, for a fixed-size window that can
    /// never go full screen (Calculator): always disabled, never run.
    FullScreenTileUnavailable,
    /// Window ▸ Remove Window from Set: always disabled until Lulo has a
    /// window-tab-set feature to remove a window from, matching the Mac's
    /// own greyed state when no window belongs to a set.
    RemoveFromSet,
    /// Window ▸ Always on Top (Calculator only, CALC-08): raises the
    /// window immediately. A persistent pin that keeps re-raising it as
    /// focus moves elsewhere needs compositor support rmac-compositor-niri
    /// does not have yet, so this is a one-shot raise rather than a true
    /// toggle (documented at its dispatch site).
    AlwaysOnTop,
    BringAllToFront,
    /// Window ▸ Arrange in Front: raises every window of the app without
    /// changing which one is frontmost among them. macOS also cascades
    /// their positions; Lulo does not reposition windows yet, so this
    /// reuses Bring All to Front's raise.
    ArrangeInFront,
    Focus(rmac_compositor::WindowId),
}

/// The halves and quarters of the screen Move & Resize offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowRegion {
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

const WINDOW_ACTION_PREFIX: &str = "window::";

/// A region's name as it appears inside a `window::` action string.
fn region_name(region: WindowRegion) -> &'static str {
    match region {
        WindowRegion::Left => "left",
        WindowRegion::Right => "right",
        WindowRegion::Top => "top",
        WindowRegion::Bottom => "bottom",
        WindowRegion::TopLeft => "top-left",
        WindowRegion::TopRight => "top-right",
        WindowRegion::BottomLeft => "bottom-left",
        WindowRegion::BottomRight => "bottom-right",
    }
}

/// The 8 "Arrange" combinations Window ▸ Move & Resize offers, in the
/// Mac's own order, paired with the primary/secondary regions
/// [`WindowCommand::ComboTile`] runs (see its doc comment for the
/// secondary-window simplification).
const COMBO_TILES: &[(&str, &str, WindowRegion, WindowRegion)] = &[
    (
        "Left & Right",
        "left-right",
        WindowRegion::Left,
        WindowRegion::Right,
    ),
    (
        "Left & Quarters",
        "left-quarters",
        WindowRegion::Left,
        WindowRegion::Right,
    ),
    (
        "Right & Left",
        "right-left",
        WindowRegion::Right,
        WindowRegion::Left,
    ),
    (
        "Right & Quarters",
        "right-quarters",
        WindowRegion::Right,
        WindowRegion::Left,
    ),
    (
        "Top & Bottom",
        "top-bottom",
        WindowRegion::Top,
        WindowRegion::Bottom,
    ),
    (
        "Top & Quarters",
        "top-quarters",
        WindowRegion::Top,
        WindowRegion::Bottom,
    ),
    (
        "Bottom & Top",
        "bottom-top",
        WindowRegion::Bottom,
        WindowRegion::Top,
    ),
    (
        "Bottom & Quarters",
        "bottom-quarters",
        WindowRegion::Bottom,
        WindowRegion::Top,
    ),
];

impl WindowCommand {
    pub fn action(self) -> String {
        let name = match self {
            Self::Minimise => "minimise",
            Self::Zoom => "zoom",
            Self::ZoomAll => "zoom-all",
            Self::Fill => "fill",
            Self::Centre => "centre",
            Self::ReturnToPreviousSize => "restore-size",
            Self::BringAllToFront => "bring-all-to-front",
            Self::ArrangeInFront => "arrange-in-front",
            Self::AlwaysOnTop => "always-on-top",
            Self::RemoveFromSet => "remove-from-set",
            Self::FullScreenTileUnavailable => "full-screen-tile",
            Self::FullScreenTileSide(WindowRegion::Left) => "full-screen-tile-left",
            Self::FullScreenTileSide(WindowRegion::Right) => "full-screen-tile-right",
            Self::FullScreenTileSide(_) => "full-screen-tile-left",
            Self::Tile(WindowRegion::Left) => "tile-left",
            Self::Tile(WindowRegion::Right) => "tile-right",
            Self::Tile(WindowRegion::Top) => "tile-top",
            Self::Tile(WindowRegion::Bottom) => "tile-bottom",
            Self::Tile(WindowRegion::TopLeft) => "tile-top-left",
            Self::Tile(WindowRegion::TopRight) => "tile-top-right",
            Self::Tile(WindowRegion::BottomLeft) => "tile-bottom-left",
            Self::Tile(WindowRegion::BottomRight) => "tile-bottom-right",
            Self::ComboTile(primary, secondary) => {
                return format!(
                    "{WINDOW_ACTION_PREFIX}combo-{}-{}",
                    region_name(primary),
                    region_name(secondary)
                )
            }
            Self::Focus(window) => return format!("{WINDOW_ACTION_PREFIX}focus.{}", window.0),
        };
        format!("{WINDOW_ACTION_PREFIX}{name}")
    }

    pub fn parse(action: &str) -> Option<Self> {
        let name = action.strip_prefix(WINDOW_ACTION_PREFIX)?;
        if let Some(id) = name.strip_prefix("focus.") {
            return id
                .parse()
                .ok()
                .map(|id| Self::Focus(rmac_compositor::WindowId(id)));
        }
        if let Some(combo) = name.strip_prefix("combo-") {
            return COMBO_TILES
                .iter()
                .find(|entry| entry.1 == combo)
                .map(|entry| Self::ComboTile(entry.2, entry.3));
        }
        Some(match name {
            "minimise" => Self::Minimise,
            "zoom" => Self::Zoom,
            "zoom-all" => Self::ZoomAll,
            "fill" => Self::Fill,
            "centre" => Self::Centre,
            "restore-size" => Self::ReturnToPreviousSize,
            "bring-all-to-front" => Self::BringAllToFront,
            "arrange-in-front" => Self::ArrangeInFront,
            "always-on-top" => Self::AlwaysOnTop,
            "remove-from-set" => Self::RemoveFromSet,
            "full-screen-tile" => Self::FullScreenTileUnavailable,
            "full-screen-tile-left" => Self::FullScreenTileSide(WindowRegion::Left),
            "full-screen-tile-right" => Self::FullScreenTileSide(WindowRegion::Right),
            "tile-left" => Self::Tile(WindowRegion::Left),
            "tile-right" => Self::Tile(WindowRegion::Right),
            "tile-top" => Self::Tile(WindowRegion::Top),
            "tile-bottom" => Self::Tile(WindowRegion::Bottom),
            "tile-top-left" => Self::Tile(WindowRegion::TopLeft),
            "tile-top-right" => Self::Tile(WindowRegion::TopRight),
            "tile-bottom-left" => Self::Tile(WindowRegion::BottomLeft),
            "tile-bottom-right" => Self::Tile(WindowRegion::BottomRight),
            _ => return None,
        })
    }

    /// The `rmac-mission-control` command that sizes the focused window,
    /// for the commands that go that way. The new multi-window and
    /// always-greyed commands run through [`dispatch_window_command`]'s
    /// own direct compositor actions instead (they are not focused-window
    /// sizing, or they need more than one window), so this stays `None`.
    pub fn mission_control_command(self) -> Option<&'static str> {
        Some(match self {
            // The green button's Zoom fills the working area, as Fill does.
            Self::Zoom | Self::Fill => "fill",
            Self::Centre => "centre",
            Self::ReturnToPreviousSize => "restore-size",
            Self::Tile(WindowRegion::Left) => "tile-left",
            Self::Tile(WindowRegion::Right) => "tile-right",
            Self::Tile(WindowRegion::Top) => "tile-top",
            Self::Tile(WindowRegion::Bottom) => "tile-bottom",
            Self::Tile(WindowRegion::TopLeft) => "tile-top-left",
            Self::Tile(WindowRegion::TopRight) => "tile-top-right",
            Self::Tile(WindowRegion::BottomLeft) => "tile-bottom-left",
            Self::Tile(WindowRegion::BottomRight) => "tile-bottom-right",
            Self::Minimise
            | Self::ZoomAll
            | Self::BringAllToFront
            | Self::ArrangeInFront
            | Self::AlwaysOnTop
            | Self::RemoveFromSet
            | Self::FullScreenTileUnavailable
            | Self::FullScreenTileSide(_)
            | Self::ComboTile(..)
            | Self::Focus(_) => return None,
        })
    }
}

/// The Window menu every app gets, as on the Mac: the window commands, Move
/// & Resize, Full-Screen Tile, Remove Window from Set, the app's own
/// Window items (Files' tabs), Bring All to Front, Arrange in Front and
/// the app's windows with a check on the current one.
///
/// The hints are the session's keys for the same commands (niri binds
/// them; a PC has no Globe key, so the Mac's fn⌃ is ⌃⌘ here — WIN-01,
/// ADR 0017). The 8 "Arrange" combinations and "Left of Screen"/"Right of
/// Screen" are free of that conflict (niri reserves bare ⌃←/→/↑/↓ for
/// Mission Control's Spaces, not ⌃⇧ or ⌃⌥⇧), so they carry the Mac's own
/// shortcut text.
pub fn window_menu(
    windows: &[MenuWindow],
    focused: Option<rmac_compositor::WindowId>,
    app_items: Vec<Item>,
    words: rmac_locale::FileVocabulary,
    app_id: Option<&str>,
) -> rmac_app_menu::Menu {
    let has_focus = focused.is_some();
    // Calculator's window is fixed-size (CALC-01/02): it cannot be zoomed,
    // filled, or put in full-screen Split View, and (observed on the
    // reference Mac with no Calculator window open) Minimise All shows no
    // key equivalent there either, unlike every other app's.
    let is_calculator = app_id == Some(rmac_apps::identity::CALCULATOR);
    let command = |label: &str, command: WindowCommand, shortcut: &str| {
        Item::new(label, command.action(), shortcut).enabled(has_focus)
    };
    let tile =
        |label: &str, half, shortcut: &str| command(label, WindowCommand::Tile(half), shortcut);
    // Each Arrange row is named by its own slug: "Left & Right" and "Left &
    // Quarters" share regions, and the menu wire format rejects a repeated
    // action. `WindowCommand::parse` maps the slug back to its regions.
    let combo = |entry: &(&str, &str, WindowRegion, WindowRegion), shortcut: &str| {
        Item::new(
            entry.0,
            format!("{WINDOW_ACTION_PREFIX}combo-{}", entry.1),
            shortcut,
        )
        .enabled(has_focus)
    };
    let combo_shortcuts = ["⌃⇧←", "⌃⌥⇧←", "⌃⇧→", "⌃⌥⇧→", "⌃⇧↑", "⌃⌥⇧↑", "⌃⇧↓", "⌃⌥⇧↓"];
    let mut move_and_resize_children = vec![
        Item::new("Halves", "window::heading-halves", "").enabled(false),
        tile("Left", WindowRegion::Left, "⌃⌘←"),
        tile("Right", WindowRegion::Right, "⌃⌘→"),
        tile("Top", WindowRegion::Top, "⌃⌘↑"),
        tile("Bottom", WindowRegion::Bottom, "⌃⌘↓"),
        Item::new("Quarters", "window::heading-quarters", "")
            .enabled(false)
            .separated(),
        tile("Top Left", WindowRegion::TopLeft, ""),
        tile("Top Right", WindowRegion::TopRight, ""),
        tile("Bottom Left", WindowRegion::BottomLeft, ""),
        tile("Bottom Right", WindowRegion::BottomRight, ""),
        Item::new("Arrange", "window::heading-arrange", "")
            .enabled(false)
            .separated(),
    ];
    move_and_resize_children.extend(
        COMBO_TILES
            .iter()
            .zip(combo_shortcuts)
            .map(|(entry, shortcut)| combo(entry, shortcut)),
    );
    move_and_resize_children.push(
        command(
            "Return to Previous Size",
            WindowCommand::ReturnToPreviousSize,
            "⌃⇧⌘R",
        )
        .separated(),
    );
    let minimise_all_shortcut = if is_calculator { "" } else { "⌥⌘M" };
    let full_screen_tile = if is_calculator {
        // A fixed-size window can never go full screen on the Mac either
        // (its own Full-Screen Tile row has no children there).
        Item::new(
            "Full-Screen Tile",
            WindowCommand::FullScreenTileUnavailable.action(),
            "",
        )
        .enabled(false)
    } else {
        Item::submenu(
            "Full-Screen Tile",
            "window::full-screen-tile",
            vec![
                command(
                    "Left of Screen",
                    WindowCommand::FullScreenTileSide(WindowRegion::Left),
                    "",
                ),
                command(
                    "Right of Screen",
                    WindowCommand::FullScreenTileSide(WindowRegion::Right),
                    "",
                ),
            ],
        )
        .enabled(has_focus)
    };
    let mut items = vec![
        command(words.minimise(), WindowCommand::Minimise, "⌘M"),
        Item::new("Minimise All", MINIMISE_ALL_ACTION, minimise_all_shortcut)
            .enabled(!windows.is_empty()),
        command("Zoom", WindowCommand::Zoom, ""),
        Item::new("Zoom All", WindowCommand::ZoomAll.action(), "").enabled(!windows.is_empty()),
        command("Fill", WindowCommand::Fill, "⌃⇧⌘F"),
        command(words.centre(), WindowCommand::Centre, "⌃⌘C"),
        Item::submenu(
            "Move & Resize",
            "window::move-and-resize",
            move_and_resize_children,
        )
        .enabled(has_focus)
        .separated(),
        full_screen_tile,
        Item::new(
            "Remove Window from Set",
            WindowCommand::RemoveFromSet.action(),
            "",
        )
        .enabled(false)
        .separated(),
    ];
    if is_calculator {
        items.push(
            Item::new("Always on Top", WindowCommand::AlwaysOnTop.action(), "")
                .enabled(has_focus)
                .separated(),
        );
    }
    let mut app_items = app_items.into_iter();
    if let Some(first) = app_items.next() {
        items.push(first.separated());
        items.extend(app_items);
    }
    items.push(
        Item::new(
            "Bring All to Front",
            WindowCommand::BringAllToFront.action(),
            "",
        )
        .enabled(!windows.is_empty())
        .separated(),
    );
    items.push(
        Item::new(
            "Arrange in Front",
            WindowCommand::ArrangeInFront.action(),
            "",
        )
        .enabled(!windows.is_empty()),
    );
    for (index, window) in windows.iter().enumerate() {
        let title = if window.title.trim().is_empty() {
            "Untitled".to_owned()
        } else {
            shorten(&window.title, 60)
        };
        let row = Item::new(title, WindowCommand::Focus(window.id).action(), "")
            .checked(focused == Some(window.id));
        items.push(if index == 0 { row.separated() } else { row });
    }
    rmac_app_menu::Menu {
        label: rmac_app_menu::WINDOW_MENU.to_owned(),
        items,
    }
}

fn shorten(text: &str, limit: usize) -> String {
    let mut characters = text.chars();
    let shortened = characters.by_ref().take(limit).collect::<String>();
    if characters.next().is_some() {
        format!("{shortened}…")
    } else {
        shortened
    }
}

/// The Help menu every app gets: a search field that finds the app's menu
/// commands by name (as the Mac's Help search does), "<App> Help",
/// which opens that app's bundled help page, then the app's own help
/// items. Results name the menu each command lives in.
pub fn help_menu(
    query: &str,
    app_name: &str,
    app_menus: &[rmac_app_menu::Menu],
    help_items: Vec<Item>,
) -> rmac_app_menu::Menu {
    const MAX_RESULTS: usize = 10;
    let mut items = vec![Item::new(
        if query.is_empty() { "Search" } else { query },
        HELP_SEARCH_ACTION,
        "",
    )];
    let needle = query.trim().to_lowercase();
    if !needle.is_empty() {
        let results = app_menus
            .iter()
            .flat_map(|menu| {
                menu.leaves()
                    .into_iter()
                    .map(move |(path, item)| (menu.label.as_str(), path, item))
            })
            .filter(|(_, _, item)| item.enabled && item.label.to_lowercase().contains(&needle))
            .take(MAX_RESULTS)
            .map(|(menu, path, item)| {
                let trail = std::iter::once(menu)
                    .chain(path)
                    .chain(std::iter::once(item.label.as_str()))
                    .collect::<Vec<_>>()
                    .join(" ▸ ");
                Item {
                    label: shorten(&trail, 60),
                    ..item.clone()
                }
            })
            .collect::<Vec<_>>();
        if results.is_empty() {
            items.push(Item::new("No Results", "help::no-results", "").enabled(false));
        } else {
            items.extend(results);
        }
    }
    // Preview, Terminal and TextEdit expose Help without a key equivalent
    // on macOS 26.
    let shortcut = if matches!(app_name, "Preview" | "Terminal" | "Text Editor") {
        ""
    } else {
        "⌘?"
    };
    let app_help = Item::new(format!("{app_name} Help"), APP_HELP_ACTION, shortcut);
    let mut help_items = std::iter::once(app_help).chain(help_items);
    if let Some(first) = help_items.next() {
        items.push(first.separated());
        items.extend(help_items);
    }
    rmac_app_menu::Menu {
        label: "Help".to_owned(),
        items,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub modifiers: Vec<char>,
    pub key: String,
}

/// Splits "⇧⌘N" into its modifier glyphs and the key they apply to.
pub fn split_shortcut(shortcut: &str) -> Shortcut {
    let modifiers = shortcut
        .chars()
        .take_while(|glyph| MODIFIERS.contains(glyph))
        .collect::<Vec<_>>();
    let key = shortcut.chars().skip(modifiers.len()).collect::<String>();
    Shortcut { modifiers, key }
}

/// Width of the shortcut column for `shortcut`, 0 when there is none.
pub fn shortcut_width(shortcut: &str) -> f32 {
    if shortcut.is_empty() || shortcut == SUBMENU_MARK {
        return 0.0;
    }
    let parts = split_shortcut(shortcut);
    parts.modifiers.len() as f32 * KEY_CELL
        + if parts.key.is_empty() {
            0.0
        } else {
            KEY_LETTER_GAP + KEY_LETTER_WIDTH
        }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconColumn {
    None,
    Standard,
    Wide,
}

impl IconColumn {
    pub fn for_icons<'a>(icons: impl IntoIterator<Item = Option<&'a str>>) -> Self {
        let mut column = Self::None;
        for icon in icons.into_iter().flatten() {
            if icon == "laptop" {
                return Self::Wide;
            }
            column = Self::Standard;
        }
        column
    }

    pub fn text_x(self) -> f32 {
        match self {
            Self::None => APP_TEXT_INSET,
            Self::Standard => APP_ICON_TEXT,
            Self::Wide => APP_WIDE_ICON_TEXT,
        }
    }

    /// Left edge of the 16 pt icon box.
    pub fn icon_x(self) -> f32 {
        match self {
            Self::None | Self::Standard => APP_ICON_CENTRE - MENU_ICON_BOX / 2.0,
            Self::Wide => APP_WIDE_ICON_CENTRE - MENU_ICON_BOX / 2.0,
        }
    }
}

/// A content-sized menu: the widest title plus its shortcut or chevron.
pub fn app_menu_width(
    items: &[Item],
    column: IconColumn,
    min_width: f32,
    label_width: impl Fn(&str) -> f32,
) -> f32 {
    let checks = if has_checks(items) { CHECK_COLUMN } else { 0.0 };
    let content = items
        .iter()
        .map(|item| {
            let trailing = if opens_submenu(item) {
                SHORTCUT_GAP + CHEVRON_WIDTH + CHEVRON_RIGHT
            } else if item.shortcut.is_empty() {
                APP_TEXT_INSET
            } else {
                SHORTCUT_GAP + shortcut_width(&item.shortcut) + KEY_RIGHT
            };
            let badge = if item.badge.is_empty() {
                0.0
            } else {
                BADGE_GAP + label_width(&item.badge) * BADGE_TEXT_SCALE + BADGE_PADDING
            };
            column.text_x() + checks + label_width(&item.label) + badge + trailing
        })
        .fold(0.0, f32::max);
    content.ceil().max(min_width)
}

/// macOS 26 decorates standard menu items with a symbol. The rmac menu and
/// the synthesized app menu are matched by action; exported menus by title.
pub fn menu_item_icon(action: &str, label: &str) -> Option<&'static str> {
    let by_action = match action {
        "system::about" => Some("laptop"),
        "system::settings" => Some("gear"),
        "system::software-center" => Some("store"),
        "system::recents" => Some("clock"),
        "system::force-quit" => Some("force-quit"),
        "system::sleep" => Some("sleep"),
        "system::restart" => Some("restart"),
        "system::shutdown" => Some("power"),
        "system::lock" => Some("lock"),
        "system::logout" => Some("person"),
        "app::about" | rmac_app_menu::ABOUT_ACTION => Some("info"),
        "app::services" => Some("services"),
        "app::hide" => Some("hide"),
        MINIMISE_ALL_ACTION => Some("minimize"),
        "app::hide-others" => Some("hide-others"),
        "app::show-all" => Some("show-all"),
        APP_HELP_ACTION => Some("help-book"),
        _ => None,
    };
    if by_action.is_some()
        || action.starts_with("system::")
        || action.starts_with("app::")
        || action.starts_with("window::")
        || action.starts_with("help::")
    {
        return match WindowCommand::parse(action) {
            Some(WindowCommand::Minimise) => Some("minimize"),
            Some(WindowCommand::Zoom) => Some("zoom"),
            _ => by_action,
        };
    }
    let title = label
        .split(" “")
        .next()
        .unwrap_or(label)
        .trim_end_matches('…')
        .trim_end_matches("...");
    Some(match title {
        "New Window" | "New Finder Window" => "new-window",
        "New Folder" => "new-folder",
        "New Tab" => "new-tab",
        "Open" => "open",
        "Close" | "Close Window" | "Close Tab" => "close",
        "Get Info" => "info",
        "Rename" => "rename",
        "Duplicate" => "duplicate",
        "Quick Look" => "eye",
        "Print" => "print",
        "Share" => "share",
        "Add to Sidebar" => "star",
        "Move to Trash" | "Move to Bin" => "trash",
        "Eject" => "eject",
        "Find" => "search",
        "Undo" => "undo",
        "Redo" => "redo",
        "Cut" => "cut",
        "Copy" => "copy",
        "Paste" => "paste",
        "Select All" => "select-all",
        "Show Clipboard" => "clipboard",
        "Minimize" | "Minimise" => "minimize",
        "Zoom" => "zoom",
        "Show Sidebar" | "Hide Sidebar" => "sidebar",
        "Enter Full Screen" | "Exit Full Screen" => "full-screen",
        "Settings" | "Preferences" => "settings",
        _ => return None,
    })
}

// ---- Status menus (Wi-Fi, Battery) ----

pub const STATUS_MENU_WIDTH: f32 = 308.0;
pub const STATUS_MENU_RADIUS: f32 = 15.0;
pub const STATUS_PADDING_TOP: f32 = 5.0;
pub const STATUS_PADDING_BOTTOM: f32 = 5.5;
pub const STATUS_TEXT_INSET: f32 = 14.5;
pub const STATUS_SEPARATOR_INSET: f32 = 14.0;
pub const STATUS_SWITCH_RIGHT: f32 = 14.0;
pub const STATUS_BADGE: f32 = 26.0;
pub const STATUS_BADGE_TEXT: f32 = 48.5;
pub const STATUS_DETAIL_SIZE: f32 = 11.0;
pub const SWITCH_WIDTH: f32 = 54.0;
pub const SWITCH_HEIGHT: f32 = 24.0;
pub const SWITCH_KNOB_WIDTH: f32 = 32.0;
pub const SWITCH_KNOB_HEIGHT: f32 = 20.0;
/// The Sound menu's volume slider (Control Centre's Sound module,
/// 2026-09-29 live capture on macOS 26: a 4 pt track — matching the
/// already-guessed `SWITCH_HEIGHT / 6` — with an 18 × 14 pt pill knob
/// centred on it, flanked by a mute and a max-volume glyph 8 pt from the
/// track). The standalone menu-bar Sound extra was not captured (enabling
/// it requires a persistent System Settings change outside this pass); this
/// reuses Control Centre's numbers, which already matched the panel's other
/// measured constants (width, row insets) exactly.
pub const SLIDER_TRACK_HEIGHT: f32 = SWITCH_HEIGHT / 6.0;
pub const SLIDER_KNOB_WIDTH: f32 = 18.0;
pub const SLIDER_KNOB_HEIGHT: f32 = 14.0;
pub const SLIDER_ICON_GAP: f32 = 8.0;
pub const MAX_LISTED_NETWORKS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum StatusMenuKind {
    Wifi,
    Battery,
    Sound,
    Bluetooth,
    Focus,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StatusAction {
    ToggleWifi,
    Join(WifiNetworkId),
    ToggleOtherNetworks,
    OpenSettings(&'static str),
    ToggleLowPower,
    /// Dismiss a failed Wi-Fi mutation's banner without changing anything.
    DismissWifiError,
    /// Dismiss a failed energy-mode mutation's banner without changing
    /// anything.
    DismissBatteryError,
    /// Switch the output device in the Sound menu's Output list, by id.
    SelectOutput(String),
    /// Dismiss a failed Sound mutation's banner without changing anything.
    DismissSoundError,
    ToggleBluetooth,
    /// Connect or disconnect a known Bluetooth device, by id. The current
    /// connection state is read at execution time, so the row never bakes
    /// in a stale direction.
    SetBluetoothConnected(String),
    /// Dismiss a failed Bluetooth mutation's banner without changing
    /// anything.
    DismissBluetoothError,
    /// Turn Do Not Disturb on or off from the Focus menu.
    ToggleFocus,
    /// Dismiss a failed Focus mutation's banner without changing anything.
    DismissFocusError,
}

impl StatusAction {
    /// Whether choosing it dismisses the menu (switches, the disclosure and
    /// error dismissals act in place, as on macOS). Selecting a Sound
    /// output, like joining a Wi-Fi network, closes the menu once chosen.
    pub fn closes_menu(&self) -> bool {
        matches!(
            self,
            Self::Join(_) | Self::OpenSettings(_) | Self::SelectOutput(_)
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeGlyph {
    Wifi(u8),
    LowPower,
    /// An audio output device (Sound menu's Output list). Every device
    /// shares one glyph: `rmac_audio` does not report a device type to
    /// pick a more specific icon by.
    Speaker,
    /// A paired Bluetooth device (Bluetooth menu's Devices list). Every
    /// device shares one glyph for the same reason as `Speaker`.
    BluetoothDevice,
}

impl BadgeGlyph {
    pub fn icon(self) -> &'static str {
        match self {
            Self::Wifi(1) => "wifi-1",
            Self::Wifi(2) => "wifi-2",
            Self::Wifi(_) => "wifi-3",
            Self::LowPower => "battery-low",
            Self::Speaker => "volume-high",
            Self::BluetoothDevice => "bluetooth-glyph",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum StatusRow {
    /// Bold title with an optional trailing value or switch.
    Title {
        label: String,
        value: Option<String>,
        switch: Option<bool>,
        action: Option<StatusAction>,
    },
    /// Secondary text line such as "Power Source: Battery".
    Info(String),
    Item {
        label: String,
        warning: bool,
        action: StatusAction,
    },
    Separator,
    Header(String),
    Disclosure {
        label: String,
        expanded: bool,
    },
    /// A row led by a 26 pt circular badge (networks, Low Power).
    Badge {
        label: String,
        glyph: BadgeGlyph,
        on: bool,
        locked: bool,
        action: Option<StatusAction>,
    },
    /// Small secondary line under a network (Option-click details).
    Detail(String),
    /// The extra point that closes a badge group before its separator.
    GroupEnd,
    /// The Sound menu's output-volume slider (0–100). Dragged with the
    /// pointer only, like the switches above. Row height, track and knob
    /// measured from Control Centre's Sound module (Mac, 2026-09-29); the
    /// standalone menu-bar Sound dropdown itself was not captured (enabling
    /// it needs a persistent System Settings change outside this pass).
    Slider {
        value: u8,
    },
    /// A plain item with a trailing checkmark when it is the active choice
    /// (Focus's Do Not Disturb). Sound's Output list and Bluetooth's
    /// Devices list use `Badge` instead — measured on the Mac, 2026-09-29,
    /// they are icon-circle rows coloured for the current device, not
    /// checkmarked items.
    Check {
        label: String,
        checked: bool,
        action: StatusAction,
    },
}

impl StatusRow {
    pub fn height(&self) -> f32 {
        match self {
            Self::Title { .. } => 31.0,
            Self::Info(_) => 20.0,
            Self::Item { .. } | Self::Disclosure { .. } | Self::Check { .. } => 24.0,
            Self::Separator => 9.0,
            Self::Header(_) => 23.0,
            Self::Badge { .. } => 32.0,
            Self::Detail(_) => 16.0,
            Self::GroupEnd => 1.0,
            // Was a guessed 28 (between a plain item and a badge row).
            // Control Centre's Sound module, 2026-09-29 live capture: the
            // gap from the Title row's bottom to the "Output" section
            // header's row is 40 pt = this row + one 9 pt separator, so the
            // slider row is 31 — the same height as Title.
            Self::Slider { .. } => 31.0,
        }
    }

    pub fn action(&self) -> Option<StatusAction> {
        match self {
            Self::Title { action, .. } | Self::Badge { action, .. } => action.clone(),
            Self::Item { action, .. } | Self::Check { action, .. } => Some(action.clone()),
            Self::Disclosure { .. } => Some(StatusAction::ToggleOtherNetworks),
            _ => None,
        }
    }

    /// Rows the arrow keys stop on: every actionable row except the title,
    /// whose switch is reached with the pointer.
    pub fn selectable(&self) -> bool {
        !matches!(self, Self::Title { .. }) && self.action().is_some()
    }
}

pub fn status_menu_height(rows: &[StatusRow]) -> f32 {
    STATUS_PADDING_TOP + rows.iter().map(StatusRow::height).sum::<f32>() + STATUS_PADDING_BOTTOM
}

/// Top of row `index` measured from the panel's top edge.
#[cfg_attr(not(test), allow(dead_code))]
pub fn status_row_top(rows: &[StatusRow], index: usize) -> f32 {
    STATUS_PADDING_TOP + rows.iter().take(index).map(StatusRow::height).sum::<f32>()
}

/// Status menus start at the item's highlight and flip to end at its right
/// edge when they would run off the screen (the Wi-Fi menu does).
pub fn status_menu_left(slot_left: f32, slot_right: f32, width: f32, screen_width: f32) -> f32 {
    if slot_left + width <= screen_width {
        slot_left
    } else {
        (slot_right - width).max(0.0)
    }
}

/// The next selectable row from `current` (none selected when `None`).
pub fn next_status_selection(
    rows: &[StatusRow],
    current: Option<usize>,
    forward: bool,
) -> Option<usize> {
    let selectable = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.selectable())
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let position = current.and_then(|current| selectable.iter().position(|&i| i == current));
    let next = match (position, forward) {
        (None, true) => 0,
        (None, false) => selectable.len().checked_sub(1)?,
        (Some(position), true) => (position + 1) % selectable.len(),
        (Some(position), false) => position.checked_sub(1).unwrap_or(selectable.len() - 1),
    };
    selectable.get(next).copied()
}

/// Evidence capture: which status menu to open on the first frame.
pub fn parse_capture_status(value: &str) -> Option<(StatusMenuKind, bool)> {
    match value {
        "wifi" => Some((StatusMenuKind::Wifi, false)),
        "wifi-option" => Some((StatusMenuKind::Wifi, true)),
        "battery" => Some((StatusMenuKind::Battery, false)),
        "sound" => Some((StatusMenuKind::Sound, false)),
        "bluetooth" => Some((StatusMenuKind::Bluetooth, false)),
        "focus" => Some((StatusMenuKind::Focus, false)),
        _ => None,
    }
}

pub fn wifi_bars(strength: u8) -> u8 {
    match strength.min(100) {
        0..=32 => 1,
        33..=65 => 2,
        _ => 3,
    }
}

pub fn security_label(security: WifiSecurity) -> &'static str {
    match security {
        WifiSecurity::Open => "None",
        WifiSecurity::EnhancedOpen => "Enhanced Open",
        WifiSecurity::Personal(rmac_network::WifiPersonalMode::Psk) => "WPA/WPA2 Personal",
        WifiSecurity::Personal(rmac_network::WifiPersonalMode::Sae) => "WPA3 Personal",
        WifiSecurity::Personal(rmac_network::WifiPersonalMode::Transition) => "WPA2/WPA3 Personal",
        WifiSecurity::Enterprise => "Enterprise",
        WifiSecurity::Legacy => "WEP",
        WifiSecurity::Protected => "Protected",
    }
}

pub struct WifiMenuInput<'a> {
    pub wifi: Option<&'a WifiSnapshot>,
    /// The Wi-Fi device, for Option-click details.
    pub device: Option<&'a NetworkDevice>,
    pub option: bool,
    pub others_expanded: bool,
    /// The network a join is currently in flight for, if any.
    pub joining: Option<&'a WifiNetworkId>,
    /// The last join or radio-toggle failure, shown until dismissed.
    pub error: Option<&'a str>,
}

/// One entry per network name: the connected access point, else the
/// strongest, sorted by name as the Mac lists them.
fn unique_networks<'a>(networks: impl Iterator<Item = &'a WifiNetwork>) -> Vec<&'a WifiNetwork> {
    let mut unique: Vec<&WifiNetwork> = Vec::new();
    for network in networks.filter(|network| !network.ssid.is_empty()) {
        match unique.iter_mut().find(|seen| seen.ssid == network.ssid) {
            Some(seen) => {
                if (network.connected, network.strength) > (seen.connected, seen.strength) {
                    *seen = network;
                }
            }
            None => unique.push(network),
        }
    }
    unique.sort_by(|left, right| {
        left.ssid
            .to_lowercase()
            .cmp(&right.ssid.to_lowercase())
            .then_with(|| left.ssid.cmp(&right.ssid))
    });
    unique.truncate(MAX_LISTED_NETWORKS);
    unique
}

fn network_row(network: &WifiNetwork, joining: bool) -> StatusRow {
    // Saved and open networks join directly; a new protected network needs
    // credentials, which Wi-Fi Settings asks for. A join already in flight
    // is not reactivatable until it resolves.
    let action = if joining || network.connected {
        None
    } else if network.known
        || matches!(
            network.security,
            WifiSecurity::Open | WifiSecurity::EnhancedOpen
        )
    {
        Some(StatusAction::Join(network.id.clone()))
    } else {
        Some(StatusAction::OpenSettings("wifi"))
    };
    StatusRow::Badge {
        label: network.ssid.clone(),
        glyph: BadgeGlyph::Wifi(wifi_bars(network.strength)),
        on: network.connected,
        locked: network.security.is_secure(),
        action,
    }
}

fn first_ipv4(addresses: &[String]) -> Option<&str> {
    addresses
        .iter()
        .map(|address| address.split('/').next().unwrap_or(address))
        .find(|address| address.contains('.'))
}

fn connected_details(network: &WifiNetwork, device: Option<&NetworkDevice>) -> Vec<StatusRow> {
    let mut rows = Vec::new();
    if let Some(address) = device.and_then(|device| first_ipv4(&device.addresses)) {
        rows.push(StatusRow::Detail(format!("IP Address: {address}")));
    }
    if let Some(router) = device.and_then(|device| device.gateway.as_deref()) {
        rows.push(StatusRow::Detail(format!("Router: {router}")));
    }
    rows.push(StatusRow::Detail(format!(
        "Security: {}",
        security_label(network.security)
    )));
    rows
}

pub fn wifi_menu_rows(input: WifiMenuInput<'_>) -> Vec<StatusRow> {
    let settings = StatusRow::Item {
        label: "Wi-Fi Settings…".into(),
        warning: false,
        action: StatusAction::OpenSettings("wifi"),
    };
    let error_row = input.error.map(|error| StatusRow::Item {
        label: error.to_string(),
        warning: true,
        action: StatusAction::DismissWifiError,
    });
    let Some(wifi) = input.wifi else {
        let mut rows = vec![StatusRow::Title {
            label: "Wi-Fi".into(),
            value: None,
            switch: None,
            action: None,
        }];
        rows.extend(error_row);
        rows.push(StatusRow::Separator);
        rows.push(settings);
        return rows;
    };
    let mut rows = vec![StatusRow::Title {
        label: "Wi-Fi".into(),
        value: None,
        switch: wifi.available.then_some(wifi.enabled),
        action: wifi.available.then_some(StatusAction::ToggleWifi),
    }];
    rows.extend(error_row);
    if !wifi.available {
        rows.push(StatusRow::Info("Wi-Fi Unavailable".into()));
    }
    if input.option {
        if let Some(interface) = &wifi.interface {
            rows.push(StatusRow::Info(format!("Interface Name: {interface}")));
        }
        if let Some(address) = input
            .device
            .and_then(|device| device.hardware_address.as_deref())
        {
            rows.push(StatusRow::Info(format!("Address: {address}")));
        }
    }
    if wifi.available && wifi.enabled {
        let connected = wifi.networks.iter().find(|network| network.connected);
        if connected.is_some_and(|network| {
            matches!(network.security, WifiSecurity::Open | WifiSecurity::Legacy)
        }) {
            rows.push(StatusRow::Item {
                label: "Weak Security…".into(),
                warning: true,
                action: StatusAction::OpenSettings("wifi"),
            });
        }
        let known = unique_networks(wifi.networks.iter().filter(|network| network.known));
        if !known.is_empty() {
            rows.push(StatusRow::Separator);
            // Mac (2026-09-29 live capture): the header agrees in number
            // with the list below it — "Known Network" singular with one
            // reachable saved network, "Known Networks" with more than one.
            let header = if known.len() == 1 {
                "Known Network"
            } else {
                "Known Networks"
            };
            rows.push(StatusRow::Header(header.into()));
            for network in known.iter().copied() {
                let joining = input.joining == Some(&network.id);
                rows.push(network_row(network, joining));
                if joining {
                    rows.push(StatusRow::Detail("Connecting…".into()));
                } else if input.option && network.connected {
                    rows.extend(connected_details(network, input.device));
                }
            }
            rows.push(StatusRow::GroupEnd);
        }
        let known_names = known
            .iter()
            .map(|network| network.ssid.as_str())
            .collect::<Vec<_>>();
        let others = unique_networks(
            wifi.networks
                .iter()
                .filter(|network| !network.known && !known_names.contains(&network.ssid.as_str())),
        );
        rows.push(StatusRow::Separator);
        rows.push(StatusRow::Disclosure {
            label: "Other Networks".into(),
            expanded: input.others_expanded,
        });
        if input.others_expanded {
            if others.is_empty() {
                rows.push(StatusRow::Info("No Other Networks".into()));
            } else {
                for network in others {
                    let joining = input.joining == Some(&network.id);
                    rows.push(network_row(network, joining));
                    if joining {
                        rows.push(StatusRow::Detail("Connecting…".into()));
                    } else if input.option && network.connected {
                        rows.extend(connected_details(network, input.device));
                    }
                }
                rows.push(StatusRow::GroupEnd);
            }
        }
    }
    rows.push(StatusRow::Separator);
    rows.push(settings);
    rows
}

pub fn battery_menu_rows(
    snapshot: Option<&rmac_power::Snapshot>,
    error: Option<&str>,
) -> Vec<StatusRow> {
    let battery = snapshot.and_then(|snapshot| snapshot.battery.as_ref());
    let mut rows = vec![StatusRow::Title {
        label: "Battery".into(),
        value: battery.map(|battery| format!("{}%", battery.percentage.min(100))),
        switch: None,
        action: None,
    }];
    if let Some(error) = error {
        rows.push(StatusRow::Item {
            label: error.to_string(),
            warning: true,
            action: StatusAction::DismissBatteryError,
        });
    }
    if let Some(battery) = battery {
        rows.push(StatusRow::Info(
            if battery.on_battery {
                "Power Source: Battery"
            } else {
                "Power Source: Power Adapter"
            }
            .into(),
        ));
        // `battery.seconds_remaining` (UPower TimeToEmpty/TimeToFull) is read
        // by the backend and already shown in System Settings' Battery pane
        // (`system-settings/src/power.rs::format_duration`). It is
        // deliberately not repeated here: the reference capture of the real
        // Tahoe menu-bar Battery dropdown at 38%/discharging
        // (target/evidence/mac-2026-09-23/ax/status-Battery.png) shows only
        // the percentage and power source, no time estimate — inventing a
        // row macOS doesn't draw here would violate "measure, never invent
        // numbers." Re-check a charging capture before adding one.
    }
    if let Some(profiles) = snapshot
        .map(|snapshot| &snapshot.profiles)
        .filter(|profiles| {
            profiles.available
                && profiles
                    .supported
                    .contains(&rmac_power::PowerProfile::PowerSaver)
        })
    {
        rows.push(StatusRow::Separator);
        rows.push(StatusRow::Header("Energy Mode".into()));
        rows.push(StatusRow::Badge {
            label: "Low Power".into(),
            glyph: BadgeGlyph::LowPower,
            on: profiles.active == Some(rmac_power::PowerProfile::PowerSaver),
            locked: false,
            action: Some(StatusAction::ToggleLowPower),
        });
        rows.push(StatusRow::GroupEnd);
    }
    rows.push(StatusRow::Separator);
    rows.push(StatusRow::Item {
        label: "Battery Settings…".into(),
        warning: false,
        action: StatusAction::OpenSettings("battery"),
    });
    rows
}

pub struct SoundMenuInput<'a> {
    pub audio: Option<&'a rmac_audio::Snapshot>,
    /// The last output-volume or output-selection mutation failure, shown
    /// until dismissed.
    pub error: Option<&'a str>,
}

/// The Sound menu: header, output-volume slider, the Output device list as
/// icon-circle badges coloured for the current device, then Sound
/// Settings… (PipeWire/pactl via `rmac-audio`, the same backend Control
/// Center's Sound module uses).
pub fn sound_menu_rows(input: SoundMenuInput<'_>) -> Vec<StatusRow> {
    let settings = StatusRow::Item {
        label: "Sound Settings…".into(),
        warning: false,
        action: StatusAction::OpenSettings("sound"),
    };
    let error_row = input.error.map(|error| StatusRow::Item {
        label: error.to_string(),
        warning: true,
        action: StatusAction::DismissSoundError,
    });
    let title = StatusRow::Title {
        label: "Sound".into(),
        value: None,
        switch: None,
        action: None,
    };
    let Some(audio) = input.audio.filter(|audio| audio.available) else {
        let mut rows = vec![title];
        rows.extend(error_row);
        rows.push(StatusRow::Info("Sound Unavailable".into()));
        rows.push(StatusRow::Separator);
        rows.push(settings);
        return rows;
    };
    let mut rows = vec![title];
    rows.extend(error_row);
    if audio.has_output {
        rows.push(StatusRow::Slider {
            value: audio.output.volume.min(100),
        });
        if !audio.outputs.is_empty() {
            rows.push(StatusRow::Separator);
            rows.push(StatusRow::Header("Output".into()));
            for device in &audio.outputs {
                // Mac (2026-09-29, Control Centre's Sound module): each
                // Output row is a 26 pt icon-circle badge, not a plain item
                // with a trailing checkmark — the same row style as Wi-Fi's
                // networks, and the current device is shown by the badge's
                // colour (`on`), not a checkmark.
                rows.push(StatusRow::Badge {
                    label: device.name.clone(),
                    glyph: BadgeGlyph::Speaker,
                    on: device.is_default,
                    locked: false,
                    action: Some(StatusAction::SelectOutput(device.id.clone())),
                });
            }
            rows.push(StatusRow::GroupEnd);
        }
    } else {
        rows.push(StatusRow::Info("No Output Device".into()));
    }
    rows.push(StatusRow::Separator);
    rows.push(settings);
    rows
}

pub struct BluetoothMenuInput<'a> {
    pub bluetooth: Option<&'a rmac_bluetooth::Snapshot>,
    /// The last power or connect/disconnect mutation failure, shown until
    /// dismissed.
    pub error: Option<&'a str>,
}

/// The Bluetooth menu: header with the power switch, the Devices list as
/// icon-circle badges coloured for the connected device (click to connect
/// or disconnect), then Bluetooth Settings… (BlueZ via `rmac-bluetooth`,
/// the same authority System Settings' Bluetooth pane uses). Only paired
/// devices are listed, as on macOS's menu-bar dropdown; raw discovery
/// results belong to the Settings pane's pairing flow, not this menu.
pub fn bluetooth_menu_rows(input: BluetoothMenuInput<'_>) -> Vec<StatusRow> {
    let settings = StatusRow::Item {
        label: "Bluetooth Settings…".into(),
        warning: false,
        action: StatusAction::OpenSettings("bluetooth"),
    };
    let error_row = input.error.map(|error| StatusRow::Item {
        label: error.to_string(),
        warning: true,
        action: StatusAction::DismissBluetoothError,
    });
    let Some(bluetooth) = input.bluetooth else {
        let mut rows = vec![StatusRow::Title {
            label: "Bluetooth".into(),
            value: None,
            switch: None,
            action: None,
        }];
        rows.extend(error_row);
        rows.push(StatusRow::Separator);
        rows.push(settings);
        return rows;
    };
    let mut rows = vec![StatusRow::Title {
        label: "Bluetooth".into(),
        value: None,
        switch: bluetooth.available.then_some(bluetooth.powered),
        action: bluetooth.available.then_some(StatusAction::ToggleBluetooth),
    }];
    rows.extend(error_row);
    if !bluetooth.available {
        rows.push(StatusRow::Info("Bluetooth Unavailable".into()));
    } else if bluetooth.powered {
        let mut devices: Vec<&rmac_bluetooth::Device> = bluetooth
            .devices
            .iter()
            .filter(|device| device.paired)
            .collect();
        devices.sort_by(|left, right| {
            right
                .connected
                .cmp(&left.connected)
                .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
                .then_with(|| left.id.cmp(&right.id))
        });
        rows.push(StatusRow::Separator);
        rows.push(StatusRow::Header("Devices".into()));
        if devices.is_empty() {
            rows.push(StatusRow::Info("No Devices".into()));
        } else {
            for device in devices {
                // Same row style as the Sound menu's Output list (Mac,
                // 2026-09-29): an icon-circle badge, coloured for the
                // connected device rather than checkmarked.
                rows.push(StatusRow::Badge {
                    label: device.name.clone(),
                    glyph: BadgeGlyph::BluetoothDevice,
                    on: device.connected,
                    locked: false,
                    action: Some(StatusAction::SetBluetoothConnected(device.id.clone())),
                });
            }
        }
        rows.push(StatusRow::GroupEnd);
    }
    rows.push(StatusRow::Separator);
    rows.push(settings);
    rows
}

/// The Focus menu: header, a Do Not Disturb toggle sharing state with
/// Control Center and the notification center, then Focus Settings….
pub fn focus_menu_rows(enabled: Option<bool>, error: Option<&str>) -> Vec<StatusRow> {
    let mut rows = vec![StatusRow::Title {
        label: "Focus".into(),
        value: None,
        switch: None,
        action: None,
    }];
    if let Some(error) = error {
        rows.push(StatusRow::Item {
            label: error.to_string(),
            warning: true,
            action: StatusAction::DismissFocusError,
        });
    }
    rows.push(StatusRow::Check {
        label: "Do Not Disturb".into(),
        checked: enabled.unwrap_or(false),
        action: StatusAction::ToggleFocus,
    });
    rows.push(StatusRow::Separator);
    rows.push(StatusRow::Item {
        label: "Focus Settings…".into(),
        warning: false,
        action: StatusAction::OpenSettings("focus"),
    });
    rows
}

// ---- Output reconciliation ----

/// How long the bar waits before its `attempt`th (0-based) re-check for an
/// output niri reports but GPUI has no display for yet, or `None` once it
/// should stop and wait for the next output change instead.
///
/// A hot-plugged output normally reaches GPUI within a few milliseconds of
/// niri announcing it, so the first check is quick; the delay then doubles,
/// and after about 6 s the bar gives up rather than polling forever for an
/// output GPUI will never show (idle means idle).
pub fn reconcile_retry_delay(attempt: u32) -> Option<Duration> {
    const FIRST: Duration = Duration::from_millis(50);
    const ATTEMPTS: u32 = 7;
    (attempt < ATTEMPTS).then(|| FIRST * 2u32.pow(attempt))
}

// ---- Log Out, Restart and Shut Down ----

/// How long a confirmed Log Out, Restart or Shut Down waits for every
/// application to close its windows. As on macOS, the request first asks
/// each app to quit, so an edited document gets its Save / Don't Save /
/// Cancel alert; an app still open when this runs out cancels the whole
/// request instead of losing work. Not measured on the Mac.
pub const QUIT_ALL_GRACE: Duration = Duration::from_secs(30);
/// How often the window list is re-read while a confirmed request waits.
/// This runs only during that request, never while idle.
pub const QUIT_ALL_CHECK: Duration = Duration::from_millis(250);

/// Where a confirmed Log Out, Restart or Shut Down stands after it asked
/// every window to close.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum QuitAllProgress {
    /// Every window closed: end the session or power off now.
    Proceed,
    /// Windows remain and the grace period has not run out.
    Wait,
    /// These applications still had windows open when the grace period ran
    /// out (usually an unsaved-changes alert). The request is cancelled.
    Interrupted(Vec<String>),
}

/// `remaining` names the application of each window still open, already
/// resolved to a display name (`None` when the window has no app ID).
pub fn quit_all_progress(remaining: &[Option<String>], elapsed: Duration) -> QuitAllProgress {
    if remaining.is_empty() {
        return QuitAllProgress::Proceed;
    }
    if elapsed < QUIT_ALL_GRACE {
        return QuitAllProgress::Wait;
    }
    let mut names = Vec::<String>::new();
    for name in remaining {
        let name = name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or("An application");
        if !names.iter().any(|known| known == name) {
            names.push(name.to_owned());
        }
    }
    QuitAllProgress::Interrupted(names)
}

/// Whether `quit_all_then` should stop retrying a failing compositor
/// connection and end the session anyway, instead of polling forever. Once
/// every window was asked to close, losing the compositor mid-wait must not
/// hang Shut Down, Restart or Log Out any longer than an unresponsive app
/// would: the same grace period applies, even though there is no window list
/// left to judge by.
pub fn quit_all_gives_up_on_errors(elapsed: Duration) -> bool {
    elapsed >= QUIT_ALL_GRACE
}

/// Title and body of the notice shown when an application stops a Log Out,
/// Restart or Shut Down. Wording is rmac's; the Mac's alert was not captured.
pub fn quit_all_interrupted_copy(action: &str, apps: &[String]) -> (String, String) {
    let (title, retry) = match action {
        "system::restart" => ("Restart Cancelled", "restart"),
        "system::shutdown" => ("Shut Down Cancelled", "shut down"),
        _ => ("Log Out Cancelled", "log out"),
    };
    let subject = match apps {
        [] => "An application".to_owned(),
        [only] => only.clone(),
        [first, second] => format!("{first} and {second}"),
        [first, second, rest @ ..] => format!("{first}, {second} and {} more", rest.len()),
    };
    let pronoun = if apps.len() > 1 { "their" } else { "its" };
    (
        title.to_owned(),
        format!("{subject} didn't quit. Save or close {pronoun} windows, then {retry} again."),
    )
}

// ---- Session confirmations ----

/// The dialog a second press of the power button opens (the Mac shows it
/// on a long press, which an x86 power button cannot report; see
/// `rmac_shortcuts::power_key`).
pub const POWER_DIALOG_ACTION: &str = "system::power-dialog";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfirmationButton {
    pub label: &'static str,
    /// The system action the button runs; `None` is Cancel.
    pub action: Option<&'static str>,
    /// Return runs this one, and it is drawn in the accent colour.
    pub default: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Confirmation {
    pub title: &'static str,
    pub detail: &'static str,
    pub buttons: Vec<ConfirmationButton>,
    pub width: f32,
    pub height: f32,
    /// One of the `menu/*.svg` glyphs, shown large in a grey disc above the
    /// title, as on the Mac.
    pub icon: &'static str,
    /// Log Out, Restart and Shut Down show a live "If you do nothing…"
    /// countdown in place of `detail`; the power-button dialog has no
    /// countdown (measured on macOS 26.2: it never auto-runs). The reopen
    /// windows option is omitted until session restore can honor it.
    pub countdown: bool,
}

/// Width of the Log Out / Restart / Shut Down confirmation, measured on
/// macOS 26.2 (scanning a Retina capture for the panel's opaque edges:
/// ~531 px / 2 = ~265 pt, rounded).
pub const CONFIRMATION_WIDTH: f32 = 264.0;
/// Height of the confirmation without a reopen-windows checkbox. The Mac's
/// measured 288 pt layout includes that control; this panel trims its 24 pt.
pub const CONFIRMATION_HEIGHT: f32 = 264.0;
/// The power-button dialog has four buttons. S: not measured on the Mac
/// (reachable only by holding the physical power key, which this audit
/// does not press); width kept at its prior estimate.
pub const POWER_DIALOG_WIDTH: f32 = 360.0;
/// The rendered power dialog is 198 pt high in the nested 1440×900 capture.
/// Keep two points of input-region clearance around its button row.
pub const POWER_DIALOG_HEIGHT: f32 = 200.0;
/// How long a menu-triggered Log Out, Restart or Shut Down waits before it
/// runs on its own, as on the Mac (measured on macOS 26.2: starts at 60
/// and counts down one per second).
pub const CONFIRMATION_COUNTDOWN: Duration = Duration::from_secs(60);
const fn button(
    label: &'static str,
    action: Option<&'static str>,
    default: bool,
) -> ConfirmationButton {
    ConfirmationButton {
        label,
        action,
        default,
    }
}

/// What the menu shows in place of its items while `action` waits for a
/// confirmation. Log Out, Restart and Shut Down each ask every app to quit
/// first (`quit_all_then`), and so do the power dialog's Restart and
/// Shut Down, so no path from the power button skips the Save alerts.
/// Wording and layout are transcribed from macOS 26.2 (see
/// design-lab/session-dialogs.html for the capture notes).
pub fn system_confirmation(action: &str) -> Confirmation {
    let timed = |title, confirm, confirm_action, icon| Confirmation {
        title,
        // Replaced at render time by `confirmation_body`, which formats
        // the live countdown; kept here as the text at zero seconds left,
        // e.g. for anything that reads `detail` directly (tests, S callers).
        detail: "",
        buttons: vec![
            button("Cancel", None, false),
            button(confirm, Some(confirm_action), true),
        ],
        width: CONFIRMATION_WIDTH,
        height: CONFIRMATION_HEIGHT,
        icon,
        countdown: true,
    };
    match action {
        "system::restart" => timed(
            "Are you sure you want to restart your computer now?",
            "Restart",
            "system::restart",
            "restart",
        ),
        "system::shutdown" => timed(
            "Are you sure you want to shut down your computer now?",
            "Shut Down",
            "system::shutdown",
            "power",
        ),
        "system::logout" => timed(
            "Are you sure you want to quit all applications and log out now?",
            "Log Out",
            "system::logout",
            "person",
        ),
        // The Mac's wording for its power-button dialog. S: the button
        // labels and order are measured (screenshot, pre-2026-09-25
        // capture); no button is the blue/default one and Return does
        // nothing there (K: this is Apple's guard against an accidental
        // press of the physical power key triggering a shutdown), and it
        // carries no countdown — holding the key does not arm an unattended
        // shutdown the way the menu items do.
        POWER_DIALOG_ACTION => Confirmation {
            title: "Are you sure you want to shut down your computer now?",
            detail: "",
            buttons: vec![
                button("Restart", Some("system::restart"), false),
                button("Sleep", Some("system::sleep"), false),
                button("Cancel", None, false),
                button("Shut Down", Some("system::shutdown"), false),
            ],
            width: POWER_DIALOG_WIDTH,
            height: POWER_DIALOG_HEIGHT,
            icon: "power",
            countdown: false,
        },
        _ => Confirmation {
            title: "Continue?",
            detail: "Confirm this system action.",
            buttons: vec![button("Cancel", None, false)],
            width: CONFIRMATION_WIDTH,
            height: POWER_DIALOG_HEIGHT,
            icon: "power",
            countdown: false,
        },
    }
}

/// The action Return runs in `action`'s confirmation (`None` when no
/// button is default, as on the power dialog: Return does nothing there).
pub fn confirmation_default_action(action: &str) -> Option<&'static str> {
    system_confirmation(action)
        .buttons
        .into_iter()
        .find(|button| button.default)
        .and_then(|button| button.action)
}

/// One control Tab can land keyboard focus on inside a confirmation. Tab and
/// Shift-Tab cycle every button,
/// Space activates whichever one has focus, and Return always runs the
/// default button regardless of focus (`confirmation_default_action`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmationControl {
    Button(usize),
}

/// `action`'s focusable controls, in tab order.
pub fn confirmation_controls(action: &str) -> Vec<ConfirmationControl> {
    let confirmation = system_confirmation(action);
    (0..confirmation.buttons.len())
        .map(ConfirmationControl::Button)
        .collect()
}

/// Where keyboard focus starts when `action`'s confirmation opens: its
/// default button, or the first control when it has none (the power-button
/// dialog: no button is default there, K, so Tab must still start
/// somewhere).
pub fn confirmation_initial_focus(action: &str) -> usize {
    system_confirmation(action)
        .buttons
        .iter()
        .position(|button| button.default)
        .unwrap_or(0)
}

/// Tab (`forward`) or Shift-Tab: the next control to focus, wrapping. `0`
/// when there is nothing to focus.
pub fn confirmation_next_focus(control_count: usize, current: usize, forward: bool) -> usize {
    if control_count == 0 {
        return 0;
    }
    let current = current.min(control_count - 1);
    if forward {
        (current + 1) % control_count
    } else {
        (current + control_count - 1) % control_count
    }
}

/// The live body text of a timed confirmation (Log Out, Restart, Shut
/// Down), given how long it has been open. `elapsed` is clamped to the
/// 60-second countdown, matching the Mac; past that the caller is expected
/// to have already run the default action.
pub fn confirmation_body(action: &str, elapsed: Duration) -> String {
    let verb = match action {
        "system::restart" => "the computer will restart",
        "system::shutdown" => "the computer will shut down",
        "system::logout" => "you will be logged out",
        _ => return system_confirmation(action).detail.to_owned(),
    };
    let remaining = CONFIRMATION_COUNTDOWN.saturating_sub(elapsed).as_secs();
    let plural = if remaining == 1 { "" } else { "s" };
    format!("If you do nothing, {verb} automatically in {remaining} second{plural}.")
}

// ---- Low battery ----

/// Battery levels, in percent, that post a warning while running on
/// battery: one at 10 % and a stronger one at 5 %. Not measured on the Mac.
pub const LOW_BATTERY_WARNINGS: [u8; 2] = [10, 5];

/// Remembers which low-battery warning has been shown for the current
/// discharge, so each level is announced once. Connecting power resets it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LowBatteryWatch {
    warned_at: Option<u8>,
}

impl LowBatteryWatch {
    /// Feed each battery reading. Returns the warning level to announce now,
    /// if the reading reached one that has not been announced yet.
    pub fn observe(&mut self, percentage: u8, on_battery: bool) -> Option<u8> {
        if !on_battery {
            self.warned_at = None;
            return None;
        }
        let level = LOW_BATTERY_WARNINGS
            .iter()
            .copied()
            .filter(|threshold| percentage <= *threshold)
            .min()?;
        if self.warned_at.is_some_and(|warned| warned <= level) {
            return None;
        }
        self.warned_at = Some(level);
        Some(level)
    }
}

/// Title and body of a low-battery warning. Wording is rmac's.
pub fn low_battery_copy(level: u8, percentage: u8) -> (String, String) {
    if level <= LOW_BATTERY_WARNINGS[1] {
        (
            "Battery Very Low".to_owned(),
            format!(
                "{percentage}% of battery remains. Connect to power now to avoid losing unsaved work."
            ),
        )
    } else {
        (
            "Low Battery".to_owned(),
            format!("{percentage}% of battery remains. Connect to power soon."),
        )
    }
}

// ---- System Settings… and Force Quit… ----

/// What choosing System Settings… or Force Quit… does. Like the Mac, a
/// second choice brings the open window forward instead of opening another.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OpenOrFocus {
    /// No window yet: start the app.
    Launch,
    /// Raise this visible window, the app's most recently focused one.
    Focus(rmac_compositor::WindowId),
    /// Every window is hidden: bring them back.
    Restore(Vec<rmac_compositor::WindowId>),
}

pub fn open_or_focus(snapshot: &rmac_compositor::Snapshot, app_id: &str) -> OpenOrFocus {
    let windows = snapshot
        .windows
        .iter()
        .filter(|window| window.app_id.as_deref() == Some(app_id))
        .collect::<Vec<_>>();
    let recency = |window: &&rmac_compositor::Window| {
        window
            .focus_timestamp
            .map(|stamp| (stamp.seconds, stamp.nanoseconds))
    };
    let visible = windows
        .iter()
        .copied()
        .filter(|window| !rmac_compositor::window_is_parked(snapshot, window))
        .max_by_key(recency);
    match visible {
        Some(window) => OpenOrFocus::Focus(window.id),
        None if windows.is_empty() => OpenOrFocus::Launch,
        None => OpenOrFocus::Restore(windows.iter().map(|window| window.id).collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(label: &str, shortcut: &str, separator_before: bool) -> Item {
        Item {
            separator_before,
            ..Item::new(label, format!("test::{label}"), shortcut)
        }
    }

    #[test]
    fn a_checkmark_column_widens_the_menu_by_its_measured_step() {
        let plain = vec![item("as List", "", false)];
        let checked = vec![item("as List", "", false).checked(true)];
        let width = |items: &[Item]| app_menu_width(items, IconColumn::None, 0.0, |_| 40.0);
        assert_eq!(width(&plain), (2.0 * APP_TEXT_INSET + 40.0).ceil());
        assert_eq!(
            width(&checked),
            (2.0 * APP_TEXT_INSET + 40.0 + CHECK_COLUMN).ceil()
        );
        assert!(has_checks(&checked) && !has_checks(&plain));
    }

    #[test]
    fn an_update_badge_widens_its_row_by_the_capsule() {
        let plain = vec![item("System Settings…", "", false)];
        let badged = vec![item("System Settings…", "", false).badge("1 update")];
        let width = |items: &[Item]| app_menu_width(items, IconColumn::None, 0.0, |_| 26.0);
        assert_eq!(width(&plain), (2.0 * APP_TEXT_INSET + 26.0).ceil());
        assert_eq!(
            width(&badged),
            (2.0 * APP_TEXT_INSET + 26.0 + BADGE_GAP + 22.0 + BADGE_PADDING).ceil()
        );
    }

    #[test]
    fn submenu_rows_draw_a_chevron_instead_of_a_shortcut() {
        let parent = Item::submenu("Find", "test::FindMenu", vec![item("Find…", "⌘F", false)]);
        assert!(opens_submenu(&parent));
        assert!(opens_submenu(&item("Recent Items", SUBMENU_MARK, false)));
        let width = app_menu_width(&[parent], IconColumn::None, 0.0, |_| 30.0);
        assert_eq!(
            width,
            (APP_TEXT_INSET + 30.0 + SHORTCUT_GAP + CHEVRON_WIDTH + CHEVRON_RIGHT).ceil()
        );
    }

    #[test]
    fn submenus_open_beside_their_row_and_flip_at_the_screen_edge() {
        // Parent at x 100, 200 wide, row 60 down: the submenu's first row
        // lines up with it.
        assert_eq!(
            submenu_origin(100.0, 200.0, 60.0, (180.0, 100.0), 1536.0, 680.0),
            (296.0, 55.0)
        );
        // Too close to the right edge: open on the left instead.
        assert_eq!(
            submenu_origin(1300.0, 200.0, 60.0, (180.0, 100.0), 1536.0, 680.0),
            (1124.0, 55.0)
        );
        // Never below the surface.
        assert_eq!(
            submenu_origin(100.0, 200.0, 640.0, (180.0, 100.0), 1536.0, 680.0).1,
            580.0
        );
    }

    #[test]
    fn keyboard_selection_skips_disabled_rows_and_wraps() {
        let items = vec![
            item("A", "", false),
            item("B", "", false).enabled(false),
            item("C", "", false),
        ];
        assert_eq!(next_enabled_row(&items, None, true), Some(0));
        assert_eq!(next_enabled_row(&items, Some(0), true), Some(2));
        assert_eq!(next_enabled_row(&items, Some(2), true), Some(0));
        assert_eq!(next_enabled_row(&items, None, false), Some(2));
        assert_eq!(next_enabled_row(&items, Some(2), false), Some(0));
        let none = vec![item("A", "", false).enabled(false)];
        assert_eq!(next_enabled_row(&none, None, true), None);
    }

    fn window(id: u64, title: &str) -> MenuWindow {
        MenuWindow {
            id: rmac_compositor::WindowId(id),
            title: title.into(),
            parked: false,
        }
    }

    #[test]
    fn every_app_gets_a_window_menu_listing_its_windows() {
        let words = rmac_locale::FileVocabulary::for_locale("en_GB.UTF-8");
        let tabs = vec![Item::new("Show Next Tab", "finder::NextTab", "⌃⇥")];
        let menu = window_menu(
            &[window(7, "Documents"), window(9, "")],
            Some(rmac_compositor::WindowId(9)),
            tabs,
            words,
            Some("org.rmac.Finder"),
        );
        let labels = menu
            .items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            [
                "Minimise",
                "Minimise All",
                "Zoom",
                "Zoom All",
                "Fill",
                "Centre",
                "Move & Resize",
                "Full-Screen Tile",
                "Remove Window from Set",
                "Show Next Tab",
                "Bring All to Front",
                "Arrange in Front",
                "Documents",
                "Untitled",
            ]
        );
        assert_eq!(menu.items[0].shortcut, "⌘M");
        assert_eq!(menu.items[1].shortcut, "⌥⌘M");
        assert_eq!(menu.items[1].action, MINIMISE_ALL_ACTION);
        assert!(menu.items[1].enabled);
        assert!(menu.items[3].enabled);
        assert_eq!(
            WindowCommand::parse(&menu.items[3].action),
            Some(WindowCommand::ZoomAll)
        );
        assert_eq!(menu.items[4].shortcut, "⌃⇧⌘F");
        assert_eq!(menu.items[5].shortcut, "⌃⌘C");
        assert!(menu.items[6].is_submenu());
        assert!(!menu.items[6].children[0].enabled);
        assert_eq!(menu.items[6].children[1].shortcut, "⌃⌘←");
        assert_eq!(menu.items[6].children[6].label, "Top Left");
        assert_eq!(menu.items[6].children[6].action, "window::tile-top-left");
        assert!(!menu.items[6].children[10].enabled);
        assert_eq!(menu.items[6].children[10].label, "Arrange");
        assert_eq!(menu.items[6].children[11].label, "Left & Right");
        assert_eq!(menu.items[6].children[11].shortcut, "⌃⇧←");
        assert_eq!(
            WindowCommand::parse(&menu.items[6].children[11].action),
            Some(WindowCommand::ComboTile(
                WindowRegion::Left,
                WindowRegion::Right
            ))
        );
        assert_eq!(menu.items[6].children[18].label, "Bottom & Quarters");
        assert_eq!(
            WindowCommand::parse(&menu.items[6].children[19].action),
            Some(WindowCommand::ReturnToPreviousSize)
        );
        assert!(menu.items[7].is_submenu());
        assert_eq!(menu.items[7].children[0].label, "Left of Screen");
        assert!(!menu.items[8].enabled);
        assert_eq!(
            WindowCommand::parse(&menu.items[8].action),
            Some(WindowCommand::RemoveFromSet)
        );
        assert!(menu.items[9].separator_before);
        assert_eq!(
            WindowCommand::parse(&menu.items[11].action),
            Some(WindowCommand::ArrangeInFront)
        );
        assert_eq!(menu.items[13].checked, rmac_app_menu::CheckState::On);
        assert_eq!(menu.items[12].checked, rmac_app_menu::CheckState::Off);
        assert_eq!(
            WindowCommand::parse(&menu.items[12].action),
            Some(WindowCommand::Focus(rmac_compositor::WindowId(7)))
        );
        assert!(rmac_app_menu::validate_menus(std::slice::from_ref(&menu)).is_ok());

        // With no window focused, the window commands are greyed out, and
        // Minimise All is greyed out too since there is nothing to minimise.
        let idle = window_menu(&[], None, Vec::new(), words, Some("org.rmac.Finder"));
        assert!(!idle.items[0].enabled);
        assert!(!idle.items[1].enabled);
        assert!(!idle.items.last().unwrap().enabled);
    }

    #[test]
    fn calculators_fixed_size_window_gets_the_mac_exceptions() {
        let words = rmac_locale::FileVocabulary::for_locale("en_GB.UTF-8");
        // CALC-08/MENU-13: Calculator's own Window menu has no Minimise
        // All key equivalent (observed on the reference Mac) and no
        // Full-Screen Tile children (its window is fixed-size), but it
        // does get Always on Top, which no other app has.
        let menu = window_menu(
            &[],
            None,
            Vec::new(),
            words,
            Some(rmac_apps::identity::CALCULATOR),
        );
        let minimise_all = menu
            .items
            .iter()
            .find(|item| item.label == "Minimise All")
            .unwrap();
        assert_eq!(minimise_all.shortcut, "");
        let full_screen_tile = menu
            .items
            .iter()
            .find(|item| item.label == "Full-Screen Tile")
            .unwrap();
        assert!(!full_screen_tile.is_submenu());
        assert!(!full_screen_tile.enabled);
        let always_on_top = menu
            .items
            .iter()
            .find(|item| item.label == "Always on Top")
            .expect("Calculator has Always on Top");
        assert_eq!(
            WindowCommand::parse(&always_on_top.action),
            Some(WindowCommand::AlwaysOnTop)
        );
        // No window is focused in this test, so Always on Top is greyed out.
        assert!(!always_on_top.enabled);
        assert!(rmac_app_menu::validate_menus(std::slice::from_ref(&menu)).is_ok());
    }

    #[test]
    fn window_actions_round_trip() {
        for command in [
            WindowCommand::Minimise,
            WindowCommand::Zoom,
            WindowCommand::ZoomAll,
            WindowCommand::Fill,
            WindowCommand::Centre,
            WindowCommand::ReturnToPreviousSize,
            WindowCommand::BringAllToFront,
            WindowCommand::ArrangeInFront,
            WindowCommand::AlwaysOnTop,
            WindowCommand::RemoveFromSet,
            WindowCommand::FullScreenTileUnavailable,
            WindowCommand::FullScreenTileSide(WindowRegion::Left),
            WindowCommand::FullScreenTileSide(WindowRegion::Right),
            WindowCommand::Tile(WindowRegion::Bottom),
            WindowCommand::Tile(WindowRegion::TopLeft),
            WindowCommand::Tile(WindowRegion::BottomRight),
            WindowCommand::ComboTile(WindowRegion::Left, WindowRegion::Right),
            WindowCommand::ComboTile(WindowRegion::Right, WindowRegion::Left),
            WindowCommand::ComboTile(WindowRegion::Top, WindowRegion::Bottom),
            WindowCommand::ComboTile(WindowRegion::Bottom, WindowRegion::Top),
            WindowCommand::Focus(rmac_compositor::WindowId(42)),
        ] {
            assert_eq!(WindowCommand::parse(&command.action()), Some(command));
        }
        // All 8 "Arrange" combinations round-trip, including the two that
        // share the same primary/secondary pair as a plain half tile
        // (Left & Quarters reuses Left & Right's pair, see its doc comment).
        for (_, slug, primary, secondary) in COMBO_TILES {
            let command = WindowCommand::ComboTile(*primary, *secondary);
            assert_eq!(
                WindowCommand::parse(&format!("window::combo-{slug}")),
                Some(command)
            );
        }
        // Sizing goes through Mission Control's own command words.
        assert_eq!(
            WindowCommand::Tile(WindowRegion::Left).mission_control_command(),
            Some("tile-left")
        );
        assert_eq!(WindowCommand::Zoom.mission_control_command(), Some("fill"));
        assert_eq!(
            WindowCommand::ReturnToPreviousSize.mission_control_command(),
            Some("restore-size")
        );
        assert_eq!(WindowCommand::Minimise.mission_control_command(), None);
        assert_eq!(WindowCommand::ZoomAll.mission_control_command(), None);
        assert_eq!(
            WindowCommand::ArrangeInFront.mission_control_command(),
            None
        );
        assert_eq!(
            WindowCommand::ComboTile(WindowRegion::Left, WindowRegion::Right)
                .mission_control_command(),
            None
        );
        assert_eq!(WindowCommand::parse("window::focus.x"), None);
        assert_eq!(WindowCommand::parse("window::combo-nonsense"), None);
        assert_eq!(WindowCommand::parse("finder::NextTab"), None);
    }

    #[test]
    fn help_search_finds_commands_in_every_menu() {
        let edit = rmac_app_menu::Menu {
            label: "Edit".into(),
            items: vec![
                item("Copy", "⌘C", false),
                Item::submenu(
                    "Find",
                    "test::FindMenu",
                    vec![
                        item("Find Next", "⌘G", false),
                        item("Find Previous", "", false),
                    ],
                ),
            ],
        };
        let help = help_menu("find n", "Files", std::slice::from_ref(&edit), Vec::new());
        assert_eq!(help.items[0].action, HELP_SEARCH_ACTION);
        assert_eq!(help.items[0].label, "find n");
        assert_eq!(help.items[1].label, "Edit ▸ Find ▸ Find Next");
        assert_eq!(help.items[1].action, "test::Find Next");
        // "<App> Help" follows the search results.
        assert_eq!(help.items[2].label, "Files Help");
        assert_eq!(help.items[2].shortcut, "⌘?");
        assert_eq!(help.items[2].action, APP_HELP_ACTION);
        assert!(help.items[2].separator_before);
        for app in ["Preview", "Terminal"] {
            let menu = help_menu("", app, &[], Vec::new());
            assert_eq!(menu.items[1].shortcut, "");
        }
        assert_eq!(help.items.len(), 3);
        for app in ["Terminal", "Text Editor"] {
            let help = help_menu("", app, &[], Vec::new());
            assert_eq!(help.items[1].shortcut, "");
        }

        let empty = help_menu(
            "",
            "Files",
            std::slice::from_ref(&edit),
            vec![item("File Format Help", "", false)],
        );
        assert_eq!(empty.items[0].label, "Search");
        assert_eq!(empty.items[1].label, "Files Help");
        assert!(empty.items[1].separator_before);
        assert_eq!(empty.items[2].label, "File Format Help");
        assert!(!empty.items[2].separator_before);

        let none = help_menu("zzz", "Files", &[edit], Vec::new());
        assert!(!none.items[1].enabled);
        // The search row is taller.
        assert_eq!(
            app_menu_item_top(&none.items, 1),
            APP_MENU_PADDING + HELP_SEARCH_ROW_HEIGHT
        );
    }

    /// The Apple menu's shape: 10 rows, 5 separators — 295 × 305 on the Mac.
    fn apple_shape() -> Vec<Item> {
        [
            false, true, false, true, true, true, false, false, true, false,
        ]
        .iter()
        .enumerate()
        .map(|(index, &separator)| item(&format!("Row {index}"), "", separator))
        .collect()
    }

    #[test]
    fn app_menu_geometry_matches_the_mac_apple_menu() {
        let items = apple_shape();
        assert_eq!(app_menu_height(&items), 305.0);
        // AX: About 39, System Settings 74, Recent Items 133, Sleep 203,
        // Lock Screen 286 — minus the panel top at 34.
        assert_eq!(app_menu_item_top(&items, 0), 5.0);
        assert_eq!(app_menu_item_top(&items, 1), 40.0);
        assert_eq!(app_menu_item_top(&items, 3), 99.0);
        assert_eq!(app_menu_item_top(&items, 5), 169.0);
        assert_eq!(app_menu_item_top(&items, 8), 252.0);
    }

    #[test]
    fn shortcuts_split_into_modifier_cells_and_a_key() {
        assert_eq!(
            split_shortcut("⇧⌘N"),
            Shortcut {
                modifiers: vec!['⇧', '⌘'],
                key: "N".into()
            }
        );
        assert_eq!(split_shortcut("⌘+").key, "+");
        assert_eq!(shortcut_width(""), 0.0);
        assert_eq!(shortcut_width(SUBMENU_MARK), 0.0);
        assert_eq!(
            shortcut_width("⌘Q"),
            KEY_CELL + KEY_LETTER_GAP + KEY_LETTER_WIDTH
        );
        assert_eq!(
            shortcut_width("⌥⌘H"),
            2.0 * KEY_CELL + KEY_LETTER_GAP + KEY_LETTER_WIDTH
        );
    }

    #[test]
    fn menu_width_is_the_widest_row_and_never_below_the_minimum() {
        let items = vec![
            item("Short", "", false),
            item("A much longer title", "⇧⌘N", false),
        ];
        let width = app_menu_width(&items, IconColumn::Standard, 100.0, |label| {
            label.len() as f32 * 7.0
        });
        let expected =
            APP_ICON_TEXT + 19.0 * 7.0 + SHORTCUT_GAP + shortcut_width("⇧⌘N") + KEY_RIGHT;
        assert_eq!(width, expected.ceil());
        assert_eq!(
            app_menu_width(&[item("x", "", false)], IconColumn::None, 180.0, |_| 7.0),
            180.0
        );
    }

    #[test]
    fn icon_column_follows_the_widest_glyph() {
        assert_eq!(IconColumn::for_icons([None, None]), IconColumn::None);
        assert_eq!(
            IconColumn::for_icons([None, Some("copy")]),
            IconColumn::Standard
        );
        assert_eq!(
            IconColumn::for_icons([Some("gear"), Some("laptop")]),
            IconColumn::Wide
        );
        assert_eq!(IconColumn::Standard.text_x(), 39.0);
        assert_eq!(IconColumn::Wide.text_x(), 41.5);
    }

    #[test]
    fn standard_items_get_their_macos_symbols() {
        assert_eq!(
            menu_item_icon("system::about", "About This Lulo OS"),
            Some("laptop")
        );
        assert_eq!(menu_item_icon("app::quit", "Quit Files"), None);
        assert_eq!(menu_item_icon("terminal::Copy", "Copy"), Some("copy"));
        assert_eq!(menu_item_icon("x", "Copy “notes”"), Some("copy"));
        assert_eq!(
            menu_item_icon("text_editor::OpenFile", "Open…"),
            Some("open")
        );
        assert_eq!(menu_item_icon("terminal::Clear", "Clear"), None);
        assert_eq!(
            menu_item_icon(MINIMISE_ALL_ACTION, "Minimise All"),
            Some("minimize")
        );
        assert_eq!(
            menu_item_icon(APP_HELP_ACTION, "Files Help"),
            Some("help-book")
        );
    }

    fn network(
        name: &str,
        strength: u8,
        known: bool,
        connected: bool,
        security: WifiSecurity,
    ) -> WifiNetwork {
        WifiNetwork {
            id: WifiNetworkId::from_bytes(name.as_bytes().to_vec(), security).unwrap(),
            ssid: name.into(),
            strength,
            security,
            known,
            connected,
        }
    }

    fn psk() -> WifiSecurity {
        WifiSecurity::Personal(rmac_network::WifiPersonalMode::Psk)
    }

    fn snapshot(networks: Vec<WifiNetwork>) -> WifiSnapshot {
        WifiSnapshot {
            available: true,
            enabled: true,
            interface: Some("wlan0".into()),
            current_ssid: None,
            networks,
            saved_networks: Vec::new(),
        }
    }

    fn labels(rows: &[StatusRow]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                StatusRow::Title { label, .. } => format!("title:{label}"),
                StatusRow::Info(label) => format!("info:{label}"),
                StatusRow::Item { label, .. } => format!("item:{label}"),
                StatusRow::Separator => "---".into(),
                StatusRow::Header(label) => format!("head:{label}"),
                StatusRow::Disclosure { label, .. } => format!("more:{label}"),
                StatusRow::Badge { label, .. } => format!("badge:{label}"),
                StatusRow::Detail(label) => format!("detail:{label}"),
                StatusRow::GroupEnd => "end".into(),
                StatusRow::Slider { value } => format!("slider:{value}"),
                StatusRow::Check { label, checked, .. } => {
                    format!("check:{}{label}", if *checked { "*" } else { "" })
                }
            })
            .collect()
    }

    #[test]
    fn wifi_menu_lists_known_networks_by_name_and_folds_the_rest() {
        let wifi = snapshot(vec![
            network("Home Wi-Fi", 90, true, true, psk()),
            network("cafe", 40, true, false, psk()),
            network("Home Wi-Fi", 20, true, false, psk()),
            network("Neighbour", 70, false, false, psk()),
        ]);
        let rows = wifi_menu_rows(WifiMenuInput {
            wifi: Some(&wifi),
            device: None,
            option: false,
            others_expanded: false,
            joining: None,
            error: None,
        });
        assert_eq!(
            labels(&rows),
            [
                "title:Wi-Fi",
                "---",
                "head:Known Networks",
                "badge:cafe",
                "badge:Home Wi-Fi",
                "end",
                "---",
                "more:Other Networks",
                "---",
                "item:Wi-Fi Settings…",
            ]
        );
        // The connected network is highlighted and has nothing to do.
        let StatusRow::Badge {
            on, action, glyph, ..
        } = &rows[4]
        else {
            panic!("expected the connected network");
        };
        assert!(*on);
        assert_eq!(*action, None);
        assert_eq!(*glyph, BadgeGlyph::Wifi(3));
        assert!(matches!(
            rows[3],
            StatusRow::Badge {
                action: Some(StatusAction::Join(_)),
                glyph: BadgeGlyph::Wifi(2),
                ..
            }
        ));
    }

    #[test]
    fn known_networks_header_agrees_in_number_with_one_saved_network() {
        // Mac (2026-09-29 live capture, this laptop's reference screen): with
        // exactly one reachable saved network the header reads "Known
        // Network" singular, not "Known Networks".
        let wifi = snapshot(vec![network("Example Wi-Fi", 90, true, true, psk())]);
        let rows = wifi_menu_rows(WifiMenuInput {
            wifi: Some(&wifi),
            device: None,
            option: false,
            others_expanded: false,
            joining: None,
            error: None,
        });
        assert_eq!(
            labels(&rows),
            [
                "title:Wi-Fi",
                "---",
                "head:Known Network",
                "badge:Example Wi-Fi",
                "end",
                "---",
                "more:Other Networks",
                "---",
                "item:Wi-Fi Settings…",
            ]
        );
    }

    #[test]
    fn other_networks_expand_and_new_protected_ones_open_settings() {
        let wifi = snapshot(vec![
            network("Neighbour", 70, false, false, psk()),
            network("Guest", 10, false, false, WifiSecurity::Open),
        ]);
        let rows = wifi_menu_rows(WifiMenuInput {
            wifi: Some(&wifi),
            device: None,
            option: false,
            others_expanded: true,
            joining: None,
            error: None,
        });
        assert_eq!(
            labels(&rows)[1..6],
            [
                "---",
                "more:Other Networks",
                "badge:Guest",
                "badge:Neighbour",
                "end"
            ]
        );
        assert!(matches!(
            &rows[3],
            StatusRow::Badge {
                action: Some(StatusAction::Join(_)),
                locked: false,
                ..
            }
        ));
        assert!(matches!(
            &rows[4],
            StatusRow::Badge {
                action: Some(StatusAction::OpenSettings("wifi")),
                locked: true,
                ..
            }
        ));
    }

    #[test]
    fn option_click_adds_interface_and_connection_details() {
        let wifi = snapshot(vec![network("Home Wi-Fi", 90, true, true, psk())]);
        let device = NetworkDevice {
            interface: "wlan0".into(),
            kind: rmac_network::DeviceKind::WiFi,
            state: rmac_network::DeviceState::Connected,
            connection: Some("Home Wi-Fi".into()),
            primary: true,
            addresses: vec!["fe80::1/64".into(), "192.168.1.20/24".into()],
            gateway: Some("192.168.1.1".into()),
            dns: Vec::new(),
            hardware_address: Some("00:11:22:33:44:55".into()),
            configuration: None,
            configuration_error: None,
        };
        let rows = wifi_menu_rows(WifiMenuInput {
            wifi: Some(&wifi),
            device: Some(&device),
            option: true,
            others_expanded: false,
            joining: None,
            error: None,
        });
        assert_eq!(
            labels(&rows)[..9],
            [
                "title:Wi-Fi",
                "info:Interface Name: wlan0",
                "info:Address: 00:11:22:33:44:55",
                "---",
                "head:Known Network",
                "badge:Home Wi-Fi",
                "detail:IP Address: 192.168.1.20",
                "detail:Router: 192.168.1.1",
                "detail:Security: WPA/WPA2 Personal",
            ]
        );
    }

    #[test]
    fn a_weak_connection_is_flagged_and_wifi_off_hides_the_lists() {
        let wifi = snapshot(vec![network("Cafe", 90, true, true, WifiSecurity::Open)]);
        let rows = wifi_menu_rows(WifiMenuInput {
            wifi: Some(&wifi),
            device: None,
            option: false,
            others_expanded: false,
            joining: None,
            error: None,
        });
        assert_eq!(labels(&rows)[1], "item:Weak Security…");

        let mut off = snapshot(Vec::new());
        off.enabled = false;
        let rows = wifi_menu_rows(WifiMenuInput {
            wifi: Some(&off),
            device: None,
            option: false,
            others_expanded: false,
            joining: None,
            error: None,
        });
        assert_eq!(
            labels(&rows),
            ["title:Wi-Fi", "---", "item:Wi-Fi Settings…"]
        );
        assert!(matches!(
            rows[0],
            StatusRow::Title {
                switch: Some(false),
                action: Some(StatusAction::ToggleWifi),
                ..
            }
        ));
    }

    #[test]
    fn a_network_being_joined_shows_connecting_and_cannot_be_reactivated() {
        let wifi = snapshot(vec![
            network("Home Wi-Fi", 90, true, true, psk()),
            network("cafe", 40, true, false, psk()),
        ]);
        let joining = wifi.networks[1].id.clone();
        let rows = wifi_menu_rows(WifiMenuInput {
            wifi: Some(&wifi),
            device: None,
            option: false,
            others_expanded: false,
            joining: Some(&joining),
            error: None,
        });
        assert_eq!(
            labels(&rows),
            [
                "title:Wi-Fi",
                "---",
                "head:Known Networks",
                "badge:cafe",
                "detail:Connecting…",
                "badge:Home Wi-Fi",
                "end",
                "---",
                "more:Other Networks",
                "---",
                "item:Wi-Fi Settings…",
            ]
        );
        // The row being joined cannot be clicked again until it resolves.
        assert!(matches!(rows[3], StatusRow::Badge { action: None, .. }));
    }

    #[test]
    fn a_join_or_radio_failure_shows_a_dismissible_banner() {
        let wifi = snapshot(vec![network("cafe", 40, true, false, psk())]);
        let rows = wifi_menu_rows(WifiMenuInput {
            wifi: Some(&wifi),
            device: None,
            option: false,
            others_expanded: false,
            joining: None,
            error: Some("Couldn't join \u{201c}cafe\u{201d}: wrong password"),
        });
        assert_eq!(
            labels(&rows)[..2],
            [
                "title:Wi-Fi",
                "item:Couldn't join \u{201c}cafe\u{201d}: wrong password"
            ]
        );
        assert!(matches!(
            rows[1],
            StatusRow::Item {
                warning: true,
                action: StatusAction::DismissWifiError,
                ..
            }
        ));

        // Wi-Fi unavailable still shows the banner ahead of the title-only
        // fallback.
        let rows = wifi_menu_rows(WifiMenuInput {
            wifi: None,
            device: None,
            option: false,
            others_expanded: false,
            joining: None,
            error: Some("Couldn't reach the Wi-Fi service"),
        });
        assert_eq!(
            labels(&rows),
            [
                "title:Wi-Fi",
                "item:Couldn't reach the Wi-Fi service",
                "---",
                "item:Wi-Fi Settings…",
            ]
        );
    }

    #[test]
    fn a_battery_mutation_failure_shows_a_dismissible_banner() {
        let battery = rmac_power::Battery {
            percentage: 61,
            state: rmac_power::BatteryState::Discharging,
            on_battery: true,
            seconds_remaining: None,
            capacity: None,
            charge_cycles: None,
            energy_rate_watts: None,
            model: None,
            charge_threshold: Default::default(),
            history: Default::default(),
        };
        let snapshot = rmac_power::Snapshot {
            battery: Some(battery),
            profiles: rmac_power::Profiles::default(),
        };
        let rows = battery_menu_rows(
            Some(&snapshot),
            Some("Couldn't change the energy mode: not authorized"),
        );
        assert_eq!(
            labels(&rows)[..2],
            [
                "title:Battery",
                "item:Couldn't change the energy mode: not authorized",
            ]
        );
        assert!(matches!(
            rows[1],
            StatusRow::Item {
                warning: true,
                action: StatusAction::DismissBatteryError,
                ..
            }
        ));
    }

    #[test]
    fn status_geometry_matches_the_mac_wifi_menu() {
        // f021: title, Weak Security, hotspot section, three known
        // networks, Other Networks, Wi-Fi Settings — 326 tall.
        let badge = || StatusRow::Badge {
            label: "n".into(),
            glyph: BadgeGlyph::Wifi(3),
            on: false,
            locked: true,
            action: None,
        };
        let rows = vec![
            StatusRow::Title {
                label: "Wi-Fi".into(),
                value: None,
                switch: Some(true),
                action: None,
            },
            StatusRow::Item {
                label: "Weak Security…".into(),
                warning: true,
                action: StatusAction::OpenSettings("wifi"),
            },
            StatusRow::Separator,
            StatusRow::Header("Personal Hotspot".into()),
            badge(),
            StatusRow::GroupEnd,
            StatusRow::Separator,
            StatusRow::Header("Known Networks".into()),
            badge(),
            badge(),
            badge(),
            StatusRow::GroupEnd,
            StatusRow::Separator,
            StatusRow::Disclosure {
                label: "Other Networks".into(),
                expanded: false,
            },
            StatusRow::Separator,
            StatusRow::Item {
                label: "Wi-Fi Settings…".into(),
                warning: false,
                action: StatusAction::OpenSettings("wifi"),
            },
        ];
        assert_eq!(status_menu_height(&rows), 325.5);
        // Row centres measured on the Mac: Weak Security 48, hotspot 107.75,
        // known networks 172.75, Wi-Fi Settings 307.75.
        assert_eq!(status_row_top(&rows, 1) + 12.0, 48.0);
        assert_eq!(status_row_top(&rows, 4) + 16.0, 108.0);
        assert_eq!(status_row_top(&rows, 8) + 16.0, 173.0);
        assert_eq!(status_row_top(&rows, 15) + 12.0, 308.0);
    }

    #[test]
    fn capture_hook_names_each_status_menu() {
        assert_eq!(
            parse_capture_status("wifi-option"),
            Some((StatusMenuKind::Wifi, true))
        );
        assert_eq!(
            parse_capture_status("battery"),
            Some((StatusMenuKind::Battery, false))
        );
        assert_eq!(parse_capture_status("clock"), None);
    }

    #[test]
    fn status_menus_flip_at_the_screen_edge() {
        // Battery fits and starts at its highlight; Wi-Fi would overflow a
        // 1470 pt screen so it ends at its highlight's right edge.
        assert_eq!(status_menu_left(1150.5, 1197.0, 308.0, 1470.0), 1150.5);
        assert_eq!(status_menu_left(1192.5, 1234.5, 308.0, 1470.0), 926.5);
    }

    #[test]
    fn arrow_keys_skip_titles_headers_and_separators() {
        let wifi = snapshot(vec![
            network("A", 90, true, true, psk()),
            network("B", 90, true, false, psk()),
        ]);
        let rows = wifi_menu_rows(WifiMenuInput {
            wifi: Some(&wifi),
            device: None,
            option: false,
            others_expanded: false,
            joining: None,
            error: None,
        });
        // A is connected (no action): first stop is B, then the disclosure,
        // then Wi-Fi Settings, then back to B.
        let first = next_status_selection(&rows, None, true);
        assert_eq!(first, Some(4));
        let second = next_status_selection(&rows, first, true);
        assert!(matches!(
            rows[second.unwrap()],
            StatusRow::Disclosure { .. }
        ));
        let third = next_status_selection(&rows, second, true);
        assert!(matches!(rows[third.unwrap()], StatusRow::Item { .. }));
        assert_eq!(next_status_selection(&rows, third, true), first);
        assert_eq!(next_status_selection(&rows, first, false), third);
        assert_eq!(next_status_selection(&[], None, true), None);
    }

    #[test]
    fn battery_menu_reports_source_and_low_power_only_when_backed() {
        let battery = rmac_power::Battery {
            percentage: 38,
            state: rmac_power::BatteryState::Discharging,
            on_battery: true,
            seconds_remaining: None,
            capacity: None,
            charge_cycles: None,
            energy_rate_watts: None,
            model: None,
            charge_threshold: Default::default(),
            history: Default::default(),
        };
        let mut snapshot = rmac_power::Snapshot {
            battery: Some(battery),
            profiles: rmac_power::Profiles::default(),
        };
        assert_eq!(
            labels(&battery_menu_rows(Some(&snapshot), None)),
            [
                "title:Battery",
                "info:Power Source: Battery",
                "---",
                "item:Battery Settings…"
            ]
        );
        snapshot.profiles = rmac_power::Profiles {
            available: true,
            active: Some(rmac_power::PowerProfile::PowerSaver),
            supported: vec![
                rmac_power::PowerProfile::PowerSaver,
                rmac_power::PowerProfile::Balanced,
            ],
            performance_degraded: None,
        };
        let rows = battery_menu_rows(Some(&snapshot), None);
        assert_eq!(
            labels(&rows),
            [
                "title:Battery",
                "info:Power Source: Battery",
                "---",
                "head:Energy Mode",
                "badge:Low Power",
                "end",
                "---",
                "item:Battery Settings…",
            ]
        );
        assert!(
            matches!(rows[0], StatusRow::Title { value: Some(ref value), .. } if value == "38%")
        );
        assert!(matches!(rows[4], StatusRow::Badge { on: true, .. }));
        // Measured Battery menu without the per-app energy section.
        assert_eq!(status_menu_height(&rows), 159.5);
    }

    #[test]
    fn new_status_rows_report_their_measured_heights() {
        assert_eq!(StatusRow::Slider { value: 50 }.height(), 31.0);
        assert_eq!(
            StatusRow::Check {
                label: "x".into(),
                checked: true,
                action: StatusAction::ToggleFocus,
            }
            .height(),
            24.0
        );
    }

    #[test]
    fn the_volume_slider_is_not_keyboard_selectable_like_the_title_switch() {
        let rows = vec![
            StatusRow::Title {
                label: "Sound".into(),
                value: None,
                switch: None,
                action: None,
            },
            StatusRow::Slider { value: 10 },
            StatusRow::Item {
                label: "Sound Settings…".into(),
                warning: false,
                action: StatusAction::OpenSettings("sound"),
            },
        ];
        assert!(!rows[1].selectable());
        assert_eq!(next_status_selection(&rows, None, true), Some(2));
    }

    #[test]
    fn selecting_a_sound_output_closes_the_menu_like_joining_wifi() {
        assert!(StatusAction::SelectOutput("id".into()).closes_menu());
        assert!(!StatusAction::ToggleBluetooth.closes_menu());
        assert!(!StatusAction::SetBluetoothConnected("id".into()).closes_menu());
        assert!(!StatusAction::ToggleFocus.closes_menu());
    }

    #[test]
    fn sound_menu_shows_unavailable_state_without_a_slider() {
        let rows = sound_menu_rows(SoundMenuInput {
            audio: None,
            error: None,
        });
        assert_eq!(
            labels(&rows),
            [
                "title:Sound",
                "info:Sound Unavailable",
                "---",
                "item:Sound Settings…"
            ]
        );
    }

    #[test]
    fn sound_menu_lists_the_volume_slider_when_output_is_available() {
        let snapshot = rmac_audio::Snapshot {
            available: true,
            has_output: true,
            output: rmac_audio::Level {
                volume: 42,
                muted: false,
            },
            ..Default::default()
        };
        let rows = sound_menu_rows(SoundMenuInput {
            audio: Some(&snapshot),
            error: None,
        });
        assert_eq!(
            labels(&rows),
            ["title:Sound", "slider:42", "---", "item:Sound Settings…"]
        );
    }

    #[test]
    fn sound_menu_surfaces_a_dismissible_error() {
        let snapshot = rmac_audio::Snapshot {
            available: true,
            has_output: true,
            ..Default::default()
        };
        let rows = sound_menu_rows(SoundMenuInput {
            audio: Some(&snapshot),
            error: Some("boom"),
        });
        assert_eq!(labels(&rows)[1], "item:boom");
        assert_eq!(rows[1].action(), Some(StatusAction::DismissSoundError));
    }

    fn bt_device(id: &str, name: &str, paired: bool, connected: bool) -> rmac_bluetooth::Device {
        rmac_bluetooth::Device {
            id: id.into(),
            name: name.into(),
            address: "AA:BB:CC:DD:EE:FF".into(),
            kind: "headset".into(),
            paired,
            trusted: paired,
            connected,
        }
    }

    #[test]
    fn bluetooth_menu_lists_paired_devices_connected_first_as_badges() {
        let snapshot = rmac_bluetooth::Snapshot {
            available: true,
            powered: true,
            discoverable: false,
            discovering: false,
            adapter_name: Some("Adapter".into()),
            devices: vec![
                bt_device("b", "Wireless Keyboard", true, false),
                bt_device("a", "AirPods", true, true),
                bt_device("c", "Unknown Scanner", false, false),
            ],
        };
        let rows = bluetooth_menu_rows(BluetoothMenuInput {
            bluetooth: Some(&snapshot),
            error: None,
        });
        assert_eq!(
            labels(&rows),
            [
                "title:Bluetooth",
                "---",
                "head:Devices",
                "badge:AirPods",
                "badge:Wireless Keyboard",
                "end",
                "---",
                "item:Bluetooth Settings…",
            ]
        );
        assert!(matches!(
            rows[0],
            StatusRow::Title {
                switch: Some(true),
                ..
            }
        ));
        assert!(matches!(
            rows[3],
            StatusRow::Badge {
                on: true,
                glyph: BadgeGlyph::BluetoothDevice,
                ..
            }
        ));
        assert!(matches!(rows[4], StatusRow::Badge { on: false, .. }));
        assert_eq!(
            rows[3].action(),
            Some(StatusAction::SetBluetoothConnected("a".into()))
        );
    }

    #[test]
    fn bluetooth_menu_reports_unavailable_and_powered_off_states() {
        let unavailable = rmac_bluetooth::Snapshot::default();
        assert_eq!(
            labels(&bluetooth_menu_rows(BluetoothMenuInput {
                bluetooth: Some(&unavailable),
                error: None,
            })),
            [
                "title:Bluetooth",
                "info:Bluetooth Unavailable",
                "---",
                "item:Bluetooth Settings…"
            ]
        );
        let off = rmac_bluetooth::Snapshot {
            available: true,
            powered: false,
            ..Default::default()
        };
        assert_eq!(
            labels(&bluetooth_menu_rows(BluetoothMenuInput {
                bluetooth: Some(&off),
                error: None,
            })),
            ["title:Bluetooth", "---", "item:Bluetooth Settings…"]
        );
    }

    #[test]
    fn bluetooth_menu_shows_no_devices_when_none_are_paired() {
        let snapshot = rmac_bluetooth::Snapshot {
            available: true,
            powered: true,
            devices: vec![bt_device("x", "Nearby Speaker", false, false)],
            ..Default::default()
        };
        let rows = bluetooth_menu_rows(BluetoothMenuInput {
            bluetooth: Some(&snapshot),
            error: None,
        });
        assert_eq!(
            labels(&rows),
            [
                "title:Bluetooth",
                "---",
                "head:Devices",
                "info:No Devices",
                "end",
                "---",
                "item:Bluetooth Settings…"
            ]
        );
    }

    #[test]
    fn focus_menu_shows_the_dnd_toggle_and_settings_link() {
        let rows = focus_menu_rows(Some(true), None);
        assert_eq!(
            labels(&rows),
            [
                "title:Focus",
                "check:*Do Not Disturb",
                "---",
                "item:Focus Settings…"
            ]
        );
        assert_eq!(rows[1].action(), Some(StatusAction::ToggleFocus));

        let off = focus_menu_rows(Some(false), None);
        assert_eq!(labels(&off)[1], "check:Do Not Disturb");

        let unknown = focus_menu_rows(None, None);
        assert_eq!(labels(&unknown)[1], "check:Do Not Disturb");
    }

    #[test]
    fn focus_menu_surfaces_a_dismissible_error() {
        let rows = focus_menu_rows(Some(false), Some("boom"));
        assert_eq!(
            labels(&rows),
            [
                "title:Focus",
                "item:boom",
                "check:Do Not Disturb",
                "---",
                "item:Focus Settings…"
            ]
        );
        assert_eq!(rows[1].action(), Some(StatusAction::DismissFocusError));
    }

    #[test]
    fn quit_all_proceeds_once_every_window_has_closed() {
        assert_eq!(
            quit_all_progress(&[], Duration::ZERO),
            QuitAllProgress::Proceed
        );
        assert_eq!(
            quit_all_progress(&[], QUIT_ALL_GRACE * 2),
            QuitAllProgress::Proceed
        );
    }

    #[test]
    fn quit_all_gives_up_on_a_failing_compositor_after_the_same_grace_period() {
        // Losing niri mid-wait must not hang the request any longer than an
        // unresponsive app would (`quit_all_then`'s `Err` branch).
        assert!(!quit_all_gives_up_on_errors(Duration::ZERO));
        assert!(!quit_all_gives_up_on_errors(
            QUIT_ALL_GRACE - QUIT_ALL_CHECK
        ));
        assert!(quit_all_gives_up_on_errors(QUIT_ALL_GRACE));
        assert!(quit_all_gives_up_on_errors(QUIT_ALL_GRACE * 2));
    }

    #[test]
    fn a_timed_confirmation_tabs_through_cancel_and_confirm() {
        let controls = confirmation_controls("system::shutdown");
        assert_eq!(
            controls,
            vec![
                ConfirmationControl::Button(0),
                ConfirmationControl::Button(1),
            ]
        );
        // Cancel, Confirm: Shut Down is the second (default) button.
        assert_eq!(confirmation_initial_focus("system::shutdown"), 1);
        assert_eq!(confirmation_next_focus(controls.len(), 1, true), 0);
        assert_eq!(confirmation_next_focus(controls.len(), 0, false), 1);
    }

    #[test]
    fn the_power_dialog_has_no_checkbox_and_starts_at_its_first_button() {
        let controls = confirmation_controls(POWER_DIALOG_ACTION);
        assert_eq!(
            controls,
            (0..4).map(ConfirmationControl::Button).collect::<Vec<_>>()
        );
        // No button is default (K), so Tab must still start somewhere.
        assert_eq!(confirmation_initial_focus(POWER_DIALOG_ACTION), 0);
        assert_eq!(confirmation_next_focus(controls.len(), 3, true), 0);
        assert_eq!(confirmation_next_focus(controls.len(), 0, false), 3);
    }

    #[test]
    fn tab_focus_never_panics_with_nothing_to_focus() {
        assert_eq!(confirmation_next_focus(0, 0, true), 0);
        assert_eq!(confirmation_next_focus(0, 5, false), 0);
    }

    #[test]
    fn quit_all_waits_for_open_windows_then_is_interrupted_never_forced() {
        let remaining = vec![
            Some("Text Editor".to_owned()),
            Some("Text Editor".to_owned()),
            None,
            Some("  ".to_owned()),
            Some("Firefox".to_owned()),
        ];
        assert_eq!(
            quit_all_progress(&remaining, QUIT_ALL_GRACE - QUIT_ALL_CHECK),
            QuitAllProgress::Wait
        );
        assert_eq!(
            quit_all_progress(&remaining, QUIT_ALL_GRACE),
            QuitAllProgress::Interrupted(vec![
                "Text Editor".to_owned(),
                "An application".to_owned(),
                "Firefox".to_owned(),
            ])
        );
    }

    #[test]
    fn interrupted_notice_names_the_apps_and_the_request() {
        let one = ["Text Editor".to_owned()];
        assert_eq!(
            quit_all_interrupted_copy("system::logout", &one),
            (
                "Log Out Cancelled".to_owned(),
                "Text Editor didn't quit. Save or close its windows, then log out again."
                    .to_owned()
            )
        );
        let two = ["Text Editor".to_owned(), "Firefox".to_owned()];
        assert_eq!(
            quit_all_interrupted_copy("system::restart", &two).1,
            "Text Editor and Firefox didn't quit. Save or close their windows, then restart again."
        );
        let four = [
            "A".to_owned(),
            "B".to_owned(),
            "C".to_owned(),
            "D".to_owned(),
        ];
        let (title, body) = quit_all_interrupted_copy("system::shutdown", &four);
        assert_eq!(title, "Shut Down Cancelled");
        assert!(body.starts_with("A, B and 2 more didn't quit."));
        assert!(body.ends_with("then shut down again."));
    }

    #[test]
    fn low_battery_warns_once_per_level_and_resets_on_power() {
        let mut watch = LowBatteryWatch::default();
        assert_eq!(watch.observe(40, true), None);
        assert_eq!(watch.observe(11, true), None);
        assert_eq!(watch.observe(10, true), Some(10));
        assert_eq!(watch.observe(9, true), None);
        assert_eq!(watch.observe(5, true), Some(5));
        assert_eq!(watch.observe(3, true), None);
        // Plugged in: the next discharge warns again.
        assert_eq!(watch.observe(3, false), None);
        assert_eq!(watch.observe(4, true), Some(5));
        // Starting below both levels announces only the stronger warning.
        let mut late = LowBatteryWatch::default();
        assert_eq!(late.observe(2, true), Some(5));
        assert_eq!(late.observe(1, true), None);
        // Charging while low never warns.
        assert_eq!(LowBatteryWatch::default().observe(2, false), None);
    }

    #[test]
    fn low_battery_copy_names_the_percentage() {
        assert_eq!(
            low_battery_copy(10, 9),
            (
                "Low Battery".to_owned(),
                "9% of battery remains. Connect to power soon.".to_owned()
            )
        );
        assert_eq!(low_battery_copy(5, 4).0, "Battery Very Low");
        assert!(low_battery_copy(5, 4)
            .1
            .starts_with("4% of battery remains."));
    }

    #[test]
    fn system_apps_come_forward_instead_of_opening_twice() {
        use rmac_compositor::{Snapshot, Timestamp, Window, WindowId, Workspace, WorkspaceId};

        let workspace = |id: u64, name: &str| Workspace {
            id: WorkspaceId(id),
            index: id as u8,
            name: Some(name.into()),
            output: None,
            urgent: false,
            active: id == 1,
            focused: id == 1,
            active_window: None,
        };
        let window = |id: u64, app: &str, workspace: u64, seconds: u64| Window {
            id: WindowId(id),
            title: None,
            app_id: Some(app.into()),
            pid: None,
            workspace: Some(WorkspaceId(workspace)),
            focused: false,
            floating: true,
            urgent: false,
            focus_timestamp: Some(Timestamp {
                seconds,
                nanoseconds: 0,
            }),
            layout: Default::default(),
        };
        let settings = rmac_apps::identity::SYSTEM_SETTINGS;
        let mut snapshot = Snapshot {
            workspaces: vec![
                workspace(1, "Desktop"),
                workspace(2, rmac_compositor::PARKING_WORKSPACE),
            ],
            windows: vec![window(1, "firefox", 1, 50)],
            ..Snapshot::default()
        };
        assert_eq!(open_or_focus(&snapshot, settings), OpenOrFocus::Launch);

        snapshot.windows.push(window(2, settings, 2, 10));
        snapshot.windows.push(window(3, settings, 2, 20));
        assert_eq!(
            open_or_focus(&snapshot, settings),
            OpenOrFocus::Restore(vec![WindowId(2), WindowId(3)])
        );

        snapshot.windows.push(window(4, settings, 1, 5));
        snapshot.windows.push(window(5, settings, 1, 30));
        assert_eq!(
            open_or_focus(&snapshot, settings),
            OpenOrFocus::Focus(WindowId(5))
        );
    }

    #[test]
    fn output_reconcile_retries_back_off_and_then_stop() {
        let delays = (0..)
            .map_while(reconcile_retry_delay)
            .map(|delay| delay.as_millis())
            .collect::<Vec<_>>();
        assert_eq!(delays, [50, 100, 200, 400, 800, 1600, 3200]);
        assert_eq!(reconcile_retry_delay(u32::MAX), None);
    }

    #[test]
    fn the_power_dialog_offers_restart_sleep_cancel_and_shut_down() {
        let dialog = system_confirmation(POWER_DIALOG_ACTION);
        let labels = dialog
            .buttons
            .iter()
            .map(|button| button.label)
            .collect::<Vec<_>>();
        assert_eq!(labels, ["Restart", "Sleep", "Cancel", "Shut Down"]);
        // Restart and Shut Down go through the same quit-every-app path as
        // the menu items, so unsaved documents get their Save alerts.
        assert_eq!(dialog.buttons[0].action, Some("system::restart"));
        assert_eq!(dialog.buttons[1].action, Some("system::sleep"));
        assert_eq!(dialog.buttons[2].action, None);
        assert_eq!(dialog.buttons[3].action, Some("system::shutdown"));
        // On the Mac none of the four buttons is the blue/default one, so a
        // stray Return from the physical power key does nothing (K).
        assert!(dialog.buttons.iter().all(|button| !button.default));
        assert_eq!(confirmation_default_action(POWER_DIALOG_ACTION), None);
        assert!(dialog.width > CONFIRMATION_WIDTH);
        assert!(!dialog.countdown);
    }

    #[test]
    fn menu_confirmations_keep_cancel_and_their_own_action() {
        for (action, confirm) in [
            ("system::restart", "Restart"),
            ("system::shutdown", "Shut Down"),
            ("system::logout", "Log Out"),
        ] {
            let confirmation = system_confirmation(action);
            assert_eq!(confirmation.buttons.len(), 2);
            assert_eq!(confirmation.buttons[0].label, "Cancel");
            assert!(!confirmation.buttons[0].default);
            assert_eq!(confirmation.buttons[1].label, confirm);
            assert_eq!(confirmation.buttons[1].action, Some(action));
            assert!(confirmation.buttons[1].default);
            assert_eq!(confirmation_default_action(action), Some(action));
            assert!(confirmation.countdown);
            assert_eq!(confirmation.width, CONFIRMATION_WIDTH);
            assert_eq!(confirmation.height, CONFIRMATION_HEIGHT);
        }
        assert_eq!(confirmation_default_action("system::unknown"), None);
    }

    #[test]
    fn menu_confirmations_match_the_macos_26_wording() {
        assert_eq!(
            system_confirmation("system::shutdown").title,
            "Are you sure you want to shut down your computer now?"
        );
        assert_eq!(
            system_confirmation("system::restart").title,
            "Are you sure you want to restart your computer now?"
        );
        assert_eq!(
            system_confirmation("system::logout").title,
            "Are you sure you want to quit all applications and log out now?"
        );
        assert_eq!(system_confirmation("system::shutdown").icon, "power");
        assert_eq!(system_confirmation("system::restart").icon, "restart");
        assert_eq!(system_confirmation("system::logout").icon, "person");
    }

    #[test]
    fn the_countdown_body_counts_down_from_sixty_and_pluralises_one() {
        assert_eq!(
            confirmation_body("system::shutdown", Duration::ZERO),
            "If you do nothing, the computer will shut down automatically in 60 seconds."
        );
        assert_eq!(
            confirmation_body("system::restart", Duration::from_secs(55)),
            "If you do nothing, the computer will restart automatically in 5 seconds."
        );
        assert_eq!(
            confirmation_body("system::logout", Duration::from_secs(59)),
            "If you do nothing, you will be logged out automatically in 1 second."
        );
        // Never goes negative if a render lags past the deadline.
        assert_eq!(
            confirmation_body("system::shutdown", Duration::from_secs(90)),
            "If you do nothing, the computer will shut down automatically in 0 seconds."
        );
    }
}
