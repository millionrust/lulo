use super::*;

pub(crate) fn sanitize_dialog_name(name: &str) -> String {
    let mut output = String::new();
    let mut truncated = false;
    for (index, character) in name.chars().enumerate() {
        if index == 120 {
            truncated = true;
            break;
        }
        output.push(if character.is_control() {
            '\u{fffd}'
        } else {
            character
        });
    }
    if truncated {
        output.push('…');
    }
    output
}

impl Render for DragPreview {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let n = self.count;
        div()
            .px_2()
            .py_0p5()
            .rounded(px(rmac_ui::mac::radius_menu_item()))
            .bg(rmac_ui::mac::accent())
            .text_color(rmac_ui::mac::on_accent())
            .text_size(rmac_ui::text_px(12.0))
            .child(if n == 1 {
                "1 item".to_string()
            } else {
                format!("{n} items")
            })
    }
}

#[derive(rust_embed::RustEmbed)]
#[folder = "assets"]
#[include = "icons/**/*.svg"]
struct AppAssets;

pub(super) struct CombinedAssets;
impl AssetSource for CombinedAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(f) = AppAssets::get(path) {
            return Ok(Some(f.data));
        }
        if let Some(data) = rmac_quick_look::asset(path) {
            return Ok(Some(data));
        }
        gpui_component_assets::Assets.load(path)
    }
    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut v: Vec<SharedString> = AppAssets::iter()
            .filter(|p| p.starts_with(path))
            .map(|p| SharedString::from(p.to_string()))
            .chain(
                rmac_quick_look::asset_paths(path)
                    .into_iter()
                    .map(SharedString::from),
            )
            .collect();
        if let Ok(mut o) = gpui_component_assets::Assets.list(path) {
            v.append(&mut o);
        }
        Ok(v)
    }
}

#[cfg(target_os = "macos")]
pub(super) fn hsl(h: u32) -> Hsla {
    gpui::rgb(h).into()
}
pub(super) fn list_bg() -> Hsla {
    rmac_ui::mac::list()
}
pub(super) fn accent() -> Hsla {
    rmac_ui::mac::accent()
}
pub(super) fn sep() -> Hsla {
    rmac_ui::mac::separator()
}
pub(super) fn label() -> Hsla {
    rmac_ui::mac::text()
}
pub(super) fn secondary() -> Hsla {
    rmac_ui::mac::text_secondary()
}
pub(super) fn drive_gray() -> Hsla {
    rmac_ui::mac::text_secondary()
}

pub(super) fn icon(path: &'static str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(path)
        .w(px(size))
        .h(px(size))
        .text_color(color)
        .flex_none()
}

/// The raster of the full-colour folder or document artwork to draw at
/// `size` points. GPUI rasterises an SVG image once, at twice its intrinsic
/// size, so each use picks the file whose raster is closest above it.
pub(super) fn item_artwork_path(is_dir: bool, size: f32) -> &'static str {
    match (is_dir, size) {
        (true, size) if size <= 24.0 => "icons/folder-artwork-16.svg",
        (true, size) if size <= 128.0 => "icons/folder-artwork-96.svg",
        (true, _) => "icons/folder-artwork-320.svg",
        (false, size) if size <= 24.0 => "icons/document-artwork-16.svg",
        (false, size) if size <= 128.0 => "icons/document-artwork-96.svg",
        (false, _) => "icons/document-artwork-320.svg",
    }
}

/// Short uppercase label a generic document carries, as Finder prints
/// "GZ" on an archive: the last extension, when it is 1–4 alphanumerics.
pub(super) fn document_badge(name: &str) -> Option<String> {
    let (stem, extension) = name.rsplit_once('.')?;
    (!stem.is_empty()
        && (1..=4).contains(&extension.len())
        && extension.chars().all(|c| c.is_ascii_alphanumeric()))
    .then(|| extension.to_ascii_uppercase())
}

/// Folder or document artwork in full colour (design-lab/finder.html: the
/// Tahoe light-blue folder and white page). Documents 32 pt and larger
/// carry their extension near the foot of the page.
pub(super) fn item_artwork(is_dir: bool, name: &str, size: f32) -> gpui::AnyElement {
    let artwork = img(item_artwork_path(is_dir, size))
        .w(px(size))
        .h(px(size))
        .flex_none();
    match (!is_dir && size >= 32.0)
        .then(|| document_badge(name))
        .flatten()
    {
        Some(badge) => div()
            .relative()
            .w(px(size))
            .h(px(size))
            .flex_none()
            .child(artwork)
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom(px(size * 0.12))
                    .flex()
                    .justify_center()
                    .text_size(px(size * 0.15))
                    .line_height(px(size * 0.18))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(gpui::rgb(0x9a9a9a))
                    .child(badge),
            )
            .into_any_element(),
        None => artwork.into_any_element(),
    }
}

/// One "Information" row shared by the gallery inspector and the column
/// preview: the label on the left, the value right-aligned in semibold.
pub(super) fn inspector_row(rule: bool, name: &'static str, value: SharedString) -> Div {
    div()
        .h(px(GALLERY_INSPECTOR_ROW_HEIGHT))
        .flex()
        .items_center()
        .gap_2()
        .when(rule, |row| row.border_t_1().border_color(header_divider()))
        .text_size(rmac_ui::text_px(12.0))
        .child(div().flex_none().text_color(secondary_text()).child(name))
        .child(
            div()
                .min_w(px(0.0))
                .flex_1()
                .truncate()
                .text_right()
                .font_weight(rmac_ui::mac::SEMIBOLD)
                .text_color(label())
                .child(value),
        )
}

// design-lab/finder.html: Date Modified 181, Size 97, Kind 115.
pub(super) const DATE_W: f32 = 181.0;
pub(super) const SIZE_W: f32 = 97.0;
pub(super) const KIND_W: f32 = 115.0;

pub(super) fn root_volume_name() -> &'static str {
    rmac_finder::places::root_volume_name()
}
