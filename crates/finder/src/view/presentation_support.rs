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
            .rounded(px(6.0))
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
        gpui_component_assets::Assets.load(path)
    }
    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut v: Vec<SharedString> = AppAssets::iter()
            .filter(|p| p.starts_with(path))
            .map(|p| SharedString::from(p.to_string()))
            .collect();
        if let Ok(mut o) = gpui_component_assets::Assets.list(path) {
            v.append(&mut o);
        }
        Ok(v)
    }
}

pub(super) fn hsl(h: u32) -> Hsla {
    gpui::rgb(h).into()
}
pub(super) fn list_bg() -> Hsla {
    rmac_ui::mac::list()
}
pub(super) fn sidebar_bg() -> Hsla {
    rmac_ui::mac::material()
}
pub(super) fn alt_row() -> Hsla {
    rmac_ui::mac::row_alternate()
}
pub(super) fn sel() -> Hsla {
    rmac_ui::mac::accent()
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
pub(super) fn tertiary() -> Hsla {
    rmac_ui::mac::text_tertiary()
}
pub(super) fn drive_gray() -> Hsla {
    rmac_ui::mac::text_secondary()
}
pub(super) fn white() -> Hsla {
    rmac_ui::mac::on_accent()
}

pub(super) fn icon(path: &'static str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(path)
        .w(px(size))
        .h(px(size))
        .text_color(color)
        .flex_none()
}

pub(super) const DATE_W: f32 = 184.0;
pub(super) const SIZE_W: f32 = 80.0;
pub(super) const KIND_W: f32 = 150.0;

pub(super) fn root_volume_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "Macintosh HD"
    } else {
        "Computer"
    }
}
