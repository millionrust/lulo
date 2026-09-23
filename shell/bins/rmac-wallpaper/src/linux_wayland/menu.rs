//! The desktop's context menus, as measured on macOS 26 (design-lab/
//! desktop.html): 24-point rows, 11-point separators, 4 padding inside a
//! 1-point border, glyphs centred 24 from the outer left edge, labels at 39,
//! chevrons ending 18 from the outer right edge.

use super::*;

const PADDING: f32 = 4.0;
const BORDER: f32 = 1.0;
const ROW: f32 = 24.0;
const SEPARATOR: f32 = 11.0;
/// Glyph box left edge and size inside a row (centre 24 from the outer edge).
const GLYPH_LEFT: f32 = 12.5;
const GLYPH: f32 = 13.0;
/// Label start inside a row (39 from the outer edge).
const LABEL_LEFT: f32 = 34.0;
/// Space after the widest label: 202 − 39 − 116 measured for the desktop
/// menu, whose widest label is "Show View Options".
const TRAILING: f32 = 47.0;
const CHEVRON_RIGHT: f32 = 13.0;
const CHEVRON: f32 = 9.0;
const SUBMENU_OVERLAP: f32 = 3.0;
const LABEL: u32 = 0xDCDCDEFF;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum MenuTarget {
    Background,
    Items(Vec<PathBuf>),
    Widget(u64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Submenu {
    SortBy,
    CleanUpBy,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Command {
    NewFolder,
    GetInfo,
    ChangeWallpaper,
    EditWidgets,
    ToggleStacks,
    CleanUp,
    CleanUpBy(rmac_desktop::SortOrder),
    Arrange(Arrangement),
    ViewOptions,
    Open,
    MoveToTrash,
    Rename,
    Duplicate,
    RemoveWidget(u64),
    ShowSubmenu(Submenu),
}

#[derive(Clone)]
pub(crate) struct Row {
    pub label: SharedString,
    pub glyph: Option<&'static str>,
    pub command: Command,
    pub section: u8,
    pub checked: bool,
    pub enabled: bool,
}

impl Row {
    fn new(
        label: &'static str,
        glyph: Option<&'static str>,
        command: Command,
        section: u8,
    ) -> Self {
        Self {
            label: label.into(),
            glyph,
            command,
            section,
            checked: false,
            enabled: true,
        }
    }

    fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    fn submenu(&self) -> Option<Submenu> {
        match self.command {
            Command::ShowSubmenu(submenu) => Some(submenu),
            _ => None,
        }
    }
}

pub(crate) struct DesktopMenu {
    pub position: Point<Pixels>,
    pub target: MenuTarget,
    pub selected: Option<usize>,
    pub submenu: Option<Submenu>,
    pub submenu_selected: Option<usize>,
    /// Keyboard focus is in the open submenu.
    pub in_submenu: bool,
}

impl DesktopMenu {
    pub fn new(position: Point<Pixels>, target: MenuTarget) -> Self {
        Self {
            position,
            target,
            selected: None,
            submenu: None,
            submenu_selected: None,
            in_submenu: false,
        }
    }
}

/// The macOS 26 desktop menus. Rows without an rmac backend (Import from
/// iPhone, Group Stacks By, Compress, Make Alias, Quick Look, Copy, Tags,
/// Share) are left out, and so is Rename for several items (Finder's batch
/// rename).
pub(crate) fn rows(target: &MenuTarget, settings: &DesktopSettings, files: bool) -> Vec<Row> {
    let free = !settings.use_stacks && !settings.arrangement.is_sorted();
    match target {
        MenuTarget::Background => vec![
            Row::new("New Folder", Some("new-folder"), Command::NewFolder, 0),
            Row::new("Get Info", Some("info"), Command::GetInfo, 1),
            Row::new("Change Wallpaper…", None, Command::ChangeWallpaper, 1),
            Row::new("Edit Widgets…", None, Command::EditWidgets, 1),
            Row::new("Use Stacks", Some("stacks"), Command::ToggleStacks, 2)
                .checked(settings.use_stacks),
            Row::new(
                "Sort By",
                Some("sort"),
                Command::ShowSubmenu(Submenu::SortBy),
                2,
            )
            .enabled(!settings.use_stacks),
            Row::new("Clean Up", Some("stacks"), Command::CleanUp, 2).enabled(free),
            Row::new(
                "Clean Up By",
                None,
                Command::ShowSubmenu(Submenu::CleanUpBy),
                2,
            )
            .enabled(free),
            Row::new("Show View Options", Some("gear"), Command::ViewOptions, 2),
        ],
        MenuTarget::Items(paths) => {
            let mut rows = vec![
                Row::new("Open", Some("open"), Command::Open, 0),
                Row::new("Move to Trash", Some("trash"), Command::MoveToTrash, 1),
                Row::new("Get Info", Some("info"), Command::GetInfo, 2),
            ];
            if paths.len() == 1 {
                rows.push(Row::new("Rename", Some("rename"), Command::Rename, 2));
            }
            rows.push(
                Row::new("Duplicate", Some("duplicate"), Command::Duplicate, 2).enabled(files),
            );
            rows
        }
        MenuTarget::Widget(id) => vec![
            Row::new("Remove Widget", None, Command::RemoveWidget(*id), 0),
            Row::new("Edit Widgets…", None, Command::EditWidgets, 1),
        ],
    }
}

pub(crate) fn submenu_rows(submenu: Submenu, settings: &DesktopSettings) -> Vec<Row> {
    use rmac_desktop::SortOrder;
    match submenu {
        Submenu::SortBy => [
            ("None", Arrangement::None, 0),
            ("Snap to Grid", Arrangement::SnapToGrid, 0),
            ("Name", Arrangement::Name, 1),
            ("Kind", Arrangement::Kind, 1),
            ("Date Modified", Arrangement::DateModified, 1),
            ("Size", Arrangement::Size, 1),
        ]
        .into_iter()
        .map(|(label, arrangement, section)| {
            Row::new(label, None, Command::Arrange(arrangement), section)
                .checked(settings.arrangement == arrangement)
        })
        .collect(),
        Submenu::CleanUpBy => [
            ("Name", SortOrder::Name),
            ("Kind", SortOrder::Kind),
            ("Date Modified", SortOrder::DateModified),
            ("Size", SortOrder::Size),
        ]
        .into_iter()
        .map(|(label, order)| Row::new(label, None, Command::CleanUpBy(order), 0))
        .collect(),
    }
}

fn rows_height(rows: &[Row]) -> f32 {
    let separators = rows
        .windows(2)
        .filter(|pair| pair[0].section != pair[1].section)
        .count();
    rows.len() as f32 * ROW + separators as f32 * SEPARATOR
}

fn panel_width(rows: &[Row], window: &Window) -> f32 {
    let text = rows
        .iter()
        .map(|row| rmac_shell_ui::text_width(window, &row.label, FontWeight::NORMAL))
        .fold(0.0, f32::max);
    (BORDER + PADDING + LABEL_LEFT + text + TRAILING).ceil()
}

/// Top of a row inside its panel's row area.
fn row_top(rows: &[Row], index: usize) -> f32 {
    let separators = rows[..=index.min(rows.len().saturating_sub(1))]
        .windows(2)
        .filter(|pair| pair[0].section != pair[1].section)
        .count();
    index as f32 * ROW + separators as f32 * SEPARATOR
}

pub(crate) fn next_enabled(rows: &[Row], from: Option<usize>, forward: bool) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    let count = rows.len();
    let mut index = match (from, forward) {
        (Some(index), true) => (index + 1) % count,
        (Some(index), false) => (index + count - 1) % count,
        (None, true) => 0,
        (None, false) => count - 1,
    };
    for _ in 0..count {
        if rows[index].enabled {
            return Some(index);
        }
        index = if forward {
            (index + 1) % count
        } else {
            (index + count - 1) % count
        };
    }
    None
}

impl Wallpaper {
    pub(crate) fn main_rows(&self, cx: &App) -> Vec<Row> {
        let Some(menu) = &self.desk.menu else {
            return Vec::new();
        };
        let files = match &menu.target {
            MenuTarget::Items(paths) => paths.iter().all(|path| path.is_file()),
            _ => false,
        };
        rows(&menu.target, &self.status.read(cx).settings, files)
    }

    pub(crate) fn open_submenu_rows(&self, cx: &App) -> Vec<Row> {
        match self.desk.menu.as_ref().and_then(|menu| menu.submenu) {
            Some(submenu) => submenu_rows(submenu, &self.status.read(cx).settings),
            None => Vec::new(),
        }
    }

    pub(crate) fn menu_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.main_rows(cx);
        let sub_rows = self.open_submenu_rows(cx);
        let Some(menu) = &mut self.desk.menu else {
            return;
        };
        match key {
            "escape" if menu.in_submenu || menu.submenu.is_some() => {
                menu.submenu = None;
                menu.submenu_selected = None;
                menu.in_submenu = false;
            }
            "escape" => self.desk.menu = None,
            "down" | "up" => {
                let forward = key == "down";
                if menu.in_submenu {
                    menu.submenu_selected = next_enabled(&sub_rows, menu.submenu_selected, forward);
                } else {
                    menu.selected = next_enabled(&rows, menu.selected, forward);
                    menu.submenu = None;
                }
            }
            "right" => {
                if let Some(submenu) = menu
                    .selected
                    .and_then(|index| rows.get(index))
                    .filter(|row| row.enabled)
                    .and_then(Row::submenu)
                {
                    menu.submenu = Some(submenu);
                    menu.in_submenu = true;
                    let fresh = submenu_rows(submenu, &self.status.read(cx).settings);
                    if let Some(menu) = &mut self.desk.menu {
                        menu.submenu_selected = next_enabled(&fresh, None, true);
                    }
                }
            }
            "left" if menu.in_submenu => {
                menu.in_submenu = false;
                menu.submenu = None;
                menu.submenu_selected = None;
            }
            "enter" | "space" => {
                let row = if menu.in_submenu {
                    menu.submenu_selected.and_then(|index| sub_rows.get(index))
                } else {
                    menu.selected.and_then(|index| rows.get(index))
                };
                if let Some(row) = row.filter(|row| row.enabled).cloned() {
                    self.menu_command(row.command, window, cx);
                    return;
                }
            }
            _ => return,
        }
        cx.notify();
    }

    pub(crate) fn menu_command(
        &mut self,
        command: Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Command::ShowSubmenu(submenu) = command {
            if let Some(menu) = &mut self.desk.menu {
                menu.submenu = Some(submenu);
                menu.in_submenu = false;
                menu.submenu_selected = None;
            }
            cx.notify();
            return;
        }
        let target = self.desk.menu.take().map(|menu| menu.target);
        self.run_command(command, target, window, cx);
        cx.notify();
    }

    pub(crate) fn render_menu(&self, window: &Window, cx: &Context<Self>) -> Option<AnyElement> {
        let menu = self.desk.menu.as_ref()?;
        let rows = self.main_rows(cx);
        let screen = window.bounds().size;
        let (screen_width, screen_height) = (f32::from(screen.width), f32::from(screen.height));
        let width = panel_width(&rows, window);
        let height = rows_height(&rows) + 2.0 * (PADDING + BORDER);
        let left = f32::from(menu.position.x).clamp(8.0, (screen_width - width - 8.0).max(8.0));
        let top = f32::from(menu.position.y).clamp(8.0, (screen_height - height - 8.0).max(8.0));
        let main = self.menu_panel(&rows, left, top, width, false, menu.selected, cx);
        let mut panels = vec![main];
        if let Some(submenu) = menu.submenu {
            let sub_rows = submenu_rows(submenu, &self.status.read(cx).settings);
            let parent = rows
                .iter()
                .position(|row| row.submenu() == Some(submenu))
                .unwrap_or(0);
            let sub_width = panel_width(&sub_rows, window);
            let sub_height = rows_height(&sub_rows) + 2.0 * (PADDING + BORDER);
            let right_start = left + width - SUBMENU_OVERLAP;
            let sub_left = if right_start + sub_width <= screen_width - 8.0 {
                right_start
            } else {
                (left - sub_width + SUBMENU_OVERLAP).max(8.0)
            };
            let sub_top = (top + row_top(&rows, parent))
                .clamp(8.0, (screen_height - sub_height - 8.0).max(8.0));
            panels.push(self.menu_panel(
                &sub_rows,
                sub_left,
                sub_top,
                sub_width,
                true,
                menu.submenu_selected,
                cx,
            ));
        }
        Some(
            div()
                .id("desktop-menu-scrim")
                .absolute()
                .inset_0()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.desk.menu = None;
                        cx.notify();
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.desk.menu = None;
                        cx.notify();
                    }),
                )
                .children(panels)
                .into_any_element(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn menu_panel(
        &self,
        rows: &[Row],
        left: f32,
        top: f32,
        width: f32,
        in_submenu: bool,
        selected: Option<usize>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let mut children = Vec::new();
        let mut previous = None;
        for (index, row) in rows.iter().enumerate() {
            if previous.is_some_and(|section| section != row.section) {
                children.push(
                    div()
                        .h(px(BORDER))
                        .mx(px(11.0))
                        .my(px((SEPARATOR - BORDER) / 2.0))
                        .bg(rgba(tokens::separator()))
                        .into_any_element(),
                );
            }
            previous = Some(row.section);
            let highlighted = selected == Some(index) && row.enabled;
            let command = row.command;
            let enabled = row.enabled;
            let submenu = row.submenu();
            let foreground = if !enabled {
                rgba(tokens::disabled_text())
            } else if highlighted {
                rgba(0xFFFFFFFF)
            } else {
                rgba(LABEL)
            };
            let leading = if row.checked {
                Some("checkmark")
            } else {
                row.glyph
            };
            let mut element = div()
                .id(format!("desktop-menu-{}-{index}", u8::from(in_submenu)))
                .role(Role::MenuItem)
                .aria_label(row.label.clone())
                .relative()
                .h(px(ROW))
                .pl(px(LABEL_LEFT))
                .flex()
                .items_center()
                .rounded(px(tokens::menu_item_radius()))
                .text_color(foreground)
                .when(highlighted, |element| element.bg(rgba(tokens::accent())))
                .children(leading.map(|glyph| {
                    svg()
                        .absolute()
                        .left(px(GLYPH_LEFT))
                        .top(px((ROW - GLYPH) / 2.0))
                        .size(px(GLYPH))
                        .path(menu_icon_path(glyph))
                        .text_color(foreground)
                }))
                .child(row.label.clone())
                .children(submenu.map(|_| {
                    svg()
                        .absolute()
                        .right(px(CHEVRON_RIGHT))
                        .top(px((ROW - CHEVRON) / 2.0))
                        .size(px(CHEVRON))
                        .path(menu_icon_path("chevron-right"))
                        .text_color(foreground)
                }));
            if enabled {
                element = element
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if !*hovered {
                            return;
                        }
                        let Some(menu) = &mut this.desk.menu else {
                            return;
                        };
                        if in_submenu {
                            menu.submenu_selected = Some(index);
                            menu.in_submenu = true;
                        } else {
                            menu.selected = Some(index);
                            menu.in_submenu = false;
                            // Hovering a submenu row opens it; another row
                            // closes it.
                            if menu.submenu != submenu {
                                menu.submenu = submenu;
                                menu.submenu_selected = None;
                            }
                        }
                        cx.notify();
                    }))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.menu_command(command, window, cx);
                    }));
            }
            children.push(element.into_any_element());
        }
        div()
            .id(format!("desktop-menu-panel-{}", u8::from(in_submenu)))
            .role(Role::Menu)
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(width))
            .p(px(PADDING))
            .rounded(px(tokens::menu_radius()))
            .bg(rgba(tokens::regular_dark_tint()))
            .border_1()
            .border_color(rgba(tokens::light_border()))
            .shadow_lg()
            .text_size(px(13.0))
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .children(children)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_menu_matches_the_measured_rows() {
        let settings = DesktopSettings::default();
        let rows = rows(&MenuTarget::Background, &settings, false);
        let labels = rows
            .iter()
            .map(|row| row.label.to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            [
                "New Folder",
                "Get Info",
                "Change Wallpaper…",
                "Edit Widgets…",
                "Use Stacks",
                "Sort By",
                "Clean Up",
                "Clean Up By",
                "Show View Options",
            ]
        );
        // 9 rows and 2 separators (Import from iPhone and its section are
        // omitted): 9 × 24 + 2 × 11.
        assert_eq!(rows_height(&rows), 238.0);
        assert_eq!(next_enabled(&rows, Some(8), true), Some(0));
    }

    #[test]
    fn item_menu_has_rename_after_get_info_for_one_item() {
        let settings = DesktopSettings::default();
        let labels = |paths: Vec<PathBuf>| {
            rows(&MenuTarget::Items(paths), &settings, true)
                .iter()
                .map(|row| row.label.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            labels(vec![PathBuf::from("/d/a.txt")]),
            ["Open", "Move to Trash", "Get Info", "Rename", "Duplicate"]
        );
        assert_eq!(
            labels(vec![PathBuf::from("/d/a.txt"), PathBuf::from("/d/b.txt")]),
            ["Open", "Move to Trash", "Get Info", "Duplicate"]
        );
    }
}
