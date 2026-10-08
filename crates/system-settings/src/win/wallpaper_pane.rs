//! Wallpaper on Windows, laid out like Lulo OS's pane (macOS 26): the
//! current picture beside its placement, then the gallery.
//!
//! Choosing a picture sets Lulo's wallpaper (`rmac-shell-settings`), which
//! the Lulo shell for Windows will show. Windows' own desktop changes only
//! when the user asks for it with "Use as Windows Desktop Picture": the
//! picture is handed to Windows as a file (a packaged JPEG, the user's own
//! photo, or for a gradient a PNG rendered once into Lulo's cache) and
//! placed through `SystemParametersInfo` (`host::Host`).

use std::path::{Path, PathBuf};

use gpui::{
    div, img, prelude::FluentBuilder as _, px, Context, Div, ElementId, InteractiveElement as _,
    ObjectFit, ParentElement as _, PathPromptOptions, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, StyledImage as _, Window,
};
use rmac_shell_settings::{WallpaperFit, WallpaperSelection};
use rmac_ui::StyledExt as _;

use super::form::{
    card, choice, footnote, glyph, label, note_card, popup_row, push_button, secondary, value_row,
};
use super::{hex, style, WinSettings};
use crate::shell_settings::{
    persist_shell_settings_mutation, render_wallpaper_preview, validate_wallpaper_choice,
    wallpaper_source_name, ShellSettingsMutation, WallpaperChange, WallpaperTarget,
};

/// Measured on macOS 26.2 (Lulo OS's `controller/wallpaper/render.rs`).
const TOP_INSET: f32 = 16.0;
const PREVIEW_WIDTH: f32 = 160.0;
const PREVIEW_HEIGHT: f32 = 100.0;
const PREVIEW_RADIUS: f32 = 4.0;
const PREVIEW_GAP: f32 = 10.0;
const GALLERY_INSET: f32 = 20.0;
const THUMB_WIDTH: f32 = 108.0;
const THUMB_HEIGHT: f32 = 72.0;
const THUMB_RADIUS: f32 = 2.0;
const THUMB_TILE_HEIGHT: f32 = 100.0;
const THUMB_GAP: f32 = 9.0;
const SECTION_TITLE_GAP: f32 = 9.0;
const SECTION_GAP: f32 = 20.0;
const THUMB_RING: f32 = 2.0;
const THUMB_RING_INSET: f32 = 3.0;

const FITS: [(&str, WallpaperFit); 5] = [
    ("Fill Screen", WallpaperFit::Fill),
    ("Fit to Screen", WallpaperFit::Fit),
    ("Stretch to Fill Screen", WallpaperFit::Stretch),
    ("Centre", WallpaperFit::Center),
    ("Tile", WallpaperFit::Tile),
];

/// What a preview was rendered for: the saved source, its fit and whether
/// the dark form of a built-in was used.
pub(super) type PreviewKey = (Option<String>, WallpaperFit, bool);

/// Where Windows can read a picture straight from the file.
fn windows_reads(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "bmp"
            )
        })
}

/// The file to hand Windows for `selection` on a `width` × `height`
/// display: the packaged artwork or the user's own picture as is, or else
/// the picture rendered once at that size into `cache` as a PNG.
pub(super) fn desktop_picture(
    selection: &WallpaperSelection,
    dark: bool,
    (width, height): (u32, u32),
    cache: &Path,
) -> Result<PathBuf, String> {
    let source = rmac_wallpaper::parse_source(selection.source.as_deref())
        .map_err(|_| "The saved wallpaper is not a picture Lulo can show.".to_owned())?;
    match &source {
        rmac_wallpaper::Source::BuiltIn(id) => {
            if let Some(path) = id
                .metadata()
                .artwork_path(dark, width, height)
                .filter(|path| path.is_file())
            {
                return Ok(path);
            }
        }
        rmac_wallpaper::Source::File(path) if windows_reads(path) && path.is_file() => {
            return Ok(path.clone());
        }
        rmac_wallpaper::Source::File(_) => {}
    }
    let resolved =
        rmac_wallpaper_system::resolve(&source).map_err(|error| error.detail().to_owned())?;
    let decoded = rmac_wallpaper_image::Cache::new(0)
        .get_or_decode_for(
            resolved,
            rmac_compositor::PhysicalSize { width, height },
            dark,
        )
        .map_err(|error| error.detail().to_owned())?;
    let picture = image::RgbaImage::from_raw(decoded.width, decoded.height, decoded.rgba.to_vec())
        .ok_or_else(|| "The wallpaper could not be rendered.".to_owned())?;
    std::fs::create_dir_all(cache)
        .map_err(|error| format!("Lulo's cache folder could not be made ({error})."))?;
    let path = cache.join("desktop-picture.png");
    picture
        .save_with_format(&path, image::ImageFormat::Png)
        .map_err(|error| format!("The desktop picture could not be saved ({error})."))?;
    Ok(path)
}

/// `%LOCALAPPDATA%\Lulo\Cache\rmac\wallpaper` (`rmac_ui::application`
/// fills `XDG_CACHE_HOME` on Windows).
fn cache_folder() -> Option<PathBuf> {
    std::env::var_os("XDG_CACHE_HOME")
        .map(|cache| PathBuf::from(cache).join("rmac").join("wallpaper"))
}

impl WinSettings {
    fn selection(&self) -> Option<WallpaperSelection> {
        match &self.shell {
            Some(Ok(snapshot)) => Some(snapshot.settings.wallpaper.default.clone()),
            _ => None,
        }
    }

    pub(super) fn load_wallpaper(&mut self, cx: &mut Context<Self>) {
        let task = cx.background_executor().spawn(async {
            rmac_shell_settings::ShellSettingsStore::from_environment()
                .and_then(|store| store.load())
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            let loaded = task.await;
            let _ = this.update(cx, |this, cx| {
                this.shell = Some(loaded.map_err(SharedString::from));
                cx.notify();
            });
        })
        .detach();
    }

    fn change_wallpaper(&mut self, change: WallpaperChange, cx: &mut Context<Self>) {
        if self.wallpaper_busy || !matches!(self.shell, Some(Ok(_))) {
            return;
        }
        self.wallpaper_busy = true;
        self.wallpaper_error = None;
        self.desktop_status = None;
        cx.notify();
        let task = cx.background_executor().spawn(async move {
            persist_shell_settings_mutation(ShellSettingsMutation::Wallpaper {
                target: WallpaperTarget::Default,
                change,
            })
            .map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            let saved = task.await;
            let _ = this.update(cx, |this, cx| {
                this.wallpaper_busy = false;
                match saved {
                    Ok(snapshot) => this.shell = Some(Ok(snapshot)),
                    Err(error) => {
                        this.wallpaper_error =
                            Some(format!("The wallpaper was not saved: {error}.").into());
                        this.load_wallpaper(cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn choose_wallpaper_file(&mut self, cx: &mut Context<Self>) {
        let Some(selection) = self.selection() else {
            return;
        };
        if self.wallpaper_busy {
            return;
        }
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        let fit = selection.fit;
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let checked = cx
                .background_executor()
                .spawn(async move { validate_wallpaper_choice(path, fit) })
                .await;
            let _ = this.update(cx, |this, cx| match checked {
                Ok(source) => this.change_wallpaper(WallpaperChange::Source(Some(source)), cx),
                Err(error) => {
                    this.wallpaper_error = Some(error.into());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn use_as_desktop_picture(&mut self, cx: &mut Context<Self>) {
        let Some(selection) = self.selection() else {
            return;
        };
        if self.desktop_busy {
            return;
        }
        let Some(cache) = cache_folder() else {
            self.desktop_status = Some(Err("Lulo has no cache folder on this PC.".into()));
            cx.notify();
            return;
        };
        self.desktop_busy = true;
        self.desktop_status = None;
        cx.notify();
        let host = self.host.clone();
        let dark = style::dark();
        let name = wallpaper_source_name(&selection);
        let task = cx.background_executor().spawn(async move {
            let size = host
                .displays()
                .ok()
                .and_then(|displays| {
                    displays
                        .first()
                        .map(|display| (display.width, display.height))
                })
                .unwrap_or((1920, 1080));
            let picture = desktop_picture(&selection, dark, size, &cache)?;
            host.set_desktop_wallpaper(&picture, selection.fit)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.desktop_busy = false;
                this.desktop_status = Some(match result {
                    Ok(()) => Ok(format!("The Windows desktop now shows “{name}”.").into()),
                    Err(error) => Err(error.into()),
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// Render the preview for the current choice once, off the UI thread.
    fn ensure_preview(&mut self, cx: &mut Context<Self>) {
        let Some(selection) = self.selection() else {
            return;
        };
        let dark = style::dark();
        let key: PreviewKey = (selection.source.clone(), selection.fit, dark);
        if self
            .wallpaper_preview
            .as_ref()
            .is_some_and(|(current, _)| *current == key)
            || self.wallpaper_preview_pending.as_ref() == Some(&key)
        {
            return;
        }
        self.wallpaper_preview_pending = Some(key.clone());
        let task = cx
            .background_executor()
            .spawn(async move { render_wallpaper_preview(&selection, dark) });
        cx.spawn(async move |this, cx| {
            let rendered = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.wallpaper_preview_pending.as_ref() == Some(&key) {
                    this.wallpaper_preview_pending = None;
                }
                if let Ok(image) = rendered {
                    if let Some((_, old)) = this.wallpaper_preview.replace((key, image)) {
                        this.garbage.push(old);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_wallpaper(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let root = div()
            .id("wallpaper-pane")
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .v_flex();
        let gallery_scroll = |content: Div| {
            div()
                .id(rmac_system_settings::accessibility::DETAIL_ID)
                .flex_1()
                .min_h(px(0.0))
                .w_full()
                .overflow_y_scroll()
                .child(content.w_full().p(px(GALLERY_INSET)))
        };
        let selection = match &self.shell {
            None => {
                return root.child(gallery_scroll(
                    div().child(note_card("Reading Lulo's wallpaper settings…")),
                ))
            }
            Some(Err(error)) => {
                return root.child(gallery_scroll(div().child(note_card(format!(
                    "Lulo's wallpaper settings could not be read: {error}."
                )))))
            }
            Some(Ok(snapshot)) => snapshot.settings.wallpaper.default.clone(),
        };
        self.ensure_preview(cx);
        let view = cx.entity();
        let enabled = !self.wallpaper_busy;
        let dark = style::dark();
        let source = rmac_wallpaper::parse_source(selection.source.as_deref()).ok();
        let current_builtin = match &source {
            Some(rmac_wallpaper::Source::BuiltIn(id)) => Some(*id),
            _ => None,
        };

        let preview = div()
            .w(px(PREVIEW_WIDTH))
            .h(px(PREVIEW_HEIGHT))
            .flex_none()
            .rounded(px(PREVIEW_RADIUS))
            .overflow_hidden()
            .bg(style::well_fill());
        let preview = match (self.wallpaper_preview.as_ref(), current_builtin) {
            (Some((_, image)), _) => preview.child(
                img(image.clone())
                    .w_full()
                    .h_full()
                    .object_fit(ObjectFit::Cover),
            ),
            (None, Some(id)) => preview.child(builtin_picture(id, dark)),
            (None, None) => preview.flex().items_center().justify_center().child(
                div()
                    .text_size(rmac_ui::text_px(11.0))
                    .text_color(secondary())
                    .child("Preparing preview…"),
            ),
        };
        let fit_choices = FITS
            .into_iter()
            .map(|(name, fit)| {
                let view = view.clone();
                choice(name, selection.fit == fit, move |_, cx| {
                    view.update(cx, |settings, cx| {
                        settings.change_wallpaper(WallpaperChange::Fit(fit), cx)
                    });
                })
            })
            .collect();
        let desktop_view = view.clone();
        let summary = card(vec![
            value_row("Wallpaper", wallpaper_source_name(&selection)),
            popup_row("wallpaper-fit", "Placement", fit_choices, enabled),
        ])
        .mb(px(0.0));
        let top = div()
            .flex_none()
            .flex()
            .items_start()
            .gap(px(PREVIEW_GAP))
            .p(px(TOP_INSET))
            .child(preview)
            .child(
                div().flex_1().min_w_0().v_flex().child(summary).child(
                    div().mt(px(PREVIEW_GAP)).flex().justify_end().child(
                        push_button("wallpaper-use-on-windows", "Use as Windows Desktop Picture")
                            .disabled(self.desktop_busy)
                            .busy(self.desktop_busy)
                            .on_click(move |_, _, cx| {
                                desktop_view
                                    .update(cx, |settings, cx| settings.use_as_desktop_picture(cx));
                            }),
                    ),
                ),
            );

        let mut notes = Vec::new();
        if let Some(error) = &self.wallpaper_error {
            notes.push(note_card(error.clone()));
        }
        match &self.desktop_status {
            Some(Ok(message)) => notes.push(footnote(message.clone())),
            Some(Err(error)) => notes.push(note_card(error.clone())),
            None => notes.push(footnote(
                "Lulo apps and the Lulo shell for Windows use this wallpaper. The Windows desktop keeps its own picture until you choose Use as Windows Desktop Picture.",
            )),
        }

        let builtin_tile = |id: rmac_wallpaper::BuiltInId| {
            let using = current_builtin == Some(id);
            let view = view.clone();
            let change = if id == rmac_wallpaper::DEFAULT_BUILT_IN {
                WallpaperChange::Source(None)
            } else {
                WallpaperChange::Source(Some(format!("builtin:{}", id.id())))
            };
            gallery_tile(
                SharedString::from(format!("wallpaper-use-{}", id.id())),
                builtin_picture(id, dark),
                id.metadata().title.into(),
                using,
            )
            .when(enabled && !using, |tile| {
                tile.cursor_pointer().on_click(move |_, _, cx| {
                    let change = change.clone();
                    view.update(cx, |settings, cx| settings.change_wallpaper(change, cx));
                })
            })
        };
        let (artwork, gradients): (Vec<_>, Vec<_>) = rmac_wallpaper::BuiltInId::ALL
            .into_iter()
            .partition(|id| id.metadata().has_artwork);
        let mut photos = Vec::new();
        if let Some(rmac_wallpaper::Source::File(_)) = &source {
            let picture = div().size_full().bg(style::well_fill()).when_some(
                self.wallpaper_preview
                    .as_ref()
                    .map(|(_, image)| image.clone()),
                |picture, image| {
                    picture.child(img(image).w_full().h_full().object_fit(ObjectFit::Cover))
                },
            );
            photos.push(gallery_tile(
                "wallpaper-current-file",
                picture,
                wallpaper_source_name(&selection),
                true,
            ));
        }
        let choose_view = view.clone();
        photos.push(add_photo_tile().when(enabled, |tile| {
            tile.cursor_pointer().on_click(move |_, _, cx| {
                choose_view.update(cx, |settings, cx| settings.choose_wallpaper_file(cx));
            })
        }));
        let gallery = div()
            .v_flex()
            .children(notes)
            .child(gallery_section(
                "Lulo",
                artwork.into_iter().map(&builtin_tile).collect(),
            ))
            .child(gallery_section("Your Photos", photos))
            .child(gallery_section(
                "Gradients",
                gradients.into_iter().map(&builtin_tile).collect(),
            ));
        root.child(top)
            .child(
                div()
                    .h(px(style::SEPARATOR))
                    .flex_none()
                    .w_full()
                    .bg(style::wallpaper_rule()),
            )
            .child(gallery_scroll(gallery))
    }
}

/// A built-in's packaged thumbnail over a swatch of its own colours, which
/// shows through where the thumbnail is not installed.
fn builtin_picture(id: rmac_wallpaper::BuiltInId, dark: bool) -> Div {
    let metadata = id.metadata();
    let palette = metadata.palette_for(dark);
    div()
        .size_full()
        .bg(gpui::linear_gradient(
            135.0,
            gpui::linear_color_stop(hex(palette[0]), 0.0),
            gpui::linear_color_stop(hex(palette[3]), 1.0),
        ))
        .when_some(
            metadata.thumbnail_path(dark).filter(|path| path.is_file()),
            |picture, path| picture.child(img(path).w_full().h_full().object_fit(ObjectFit::Cover)),
        )
}

fn gallery_section(title: &'static str, tiles: Vec<Stateful<Div>>) -> Div {
    div()
        .v_flex()
        .mb(px(SECTION_GAP))
        .child(
            div()
                .mb(px(SECTION_TITLE_GAP))
                .text_size(rmac_ui::text_px(13.0))
                .line_height(px(16.0))
                .font_weight(rmac_ui::mac::BOLD)
                .text_color(style::heading_text())
                .child(title),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_x(px(THUMB_GAP))
                .children(tiles),
        )
}

fn gallery_tile(
    id: impl Into<ElementId>,
    picture: Div,
    name: SharedString,
    selected: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(THUMB_WIDTH))
        .h(px(THUMB_TILE_HEIGHT))
        .flex_none()
        .v_flex()
        .items_center()
        .gap(px(4.0))
        .child(
            div()
                .w(px(THUMB_WIDTH))
                .h(px(THUMB_HEIGHT))
                .flex_none()
                .rounded(px(
                    THUMB_RADIUS + if selected { THUMB_RING_INSET } else { 0.0 }
                ))
                .when(selected, |frame| {
                    frame
                        .border(px(THUMB_RING))
                        .border_color(rmac_ui::mac::accent())
                        .p(px(THUMB_RING_INSET - THUMB_RING))
                })
                .child(
                    div()
                        .size_full()
                        .rounded(px(THUMB_RADIUS))
                        .overflow_hidden()
                        .child(picture),
                ),
        )
        .child(
            div()
                .max_w(px(THUMB_WIDTH))
                .truncate()
                .text_size(rmac_ui::text_px(10.0))
                .line_height(px(14.0))
                .font_weight(rmac_ui::mac::MEDIUM)
                .text_color(label())
                .child(name),
        )
}

fn add_photo_tile() -> Stateful<Div> {
    div()
        .id("wallpaper-choose")
        .w(px(THUMB_WIDTH))
        .h(px(THUMB_HEIGHT))
        .flex_none()
        .v_flex()
        .items_center()
        .justify_center()
        .gap(px(4.0))
        .rounded(px(PREVIEW_RADIUS))
        .bg(style::add_photo_fill())
        .child(glyph("icons/image.svg", 24.0, secondary()))
        .child(
            div()
                .text_size(rmac_ui::text_px(10.0))
                .line_height(px(14.0))
                .font_weight(rmac_ui::mac::MEDIUM)
                .text_color(secondary())
                .child("Add Photo…"),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_reads_jpeg_png_and_bmp_but_not_webp() {
        assert!(windows_reads(Path::new(r"C:\Pictures\a.JPG")));
        assert!(windows_reads(Path::new(r"C:\Pictures\a.png")));
        assert!(windows_reads(Path::new(r"C:\Pictures\a.bmp")));
        assert!(!windows_reads(Path::new(r"C:\Pictures\a.webp")));
        assert!(!windows_reads(Path::new(r"C:\Pictures\a")));
    }

    #[test]
    fn a_gradient_is_rendered_once_into_the_cache_as_a_png() {
        let cache =
            std::env::temp_dir().join(format!("lulo-wallpaper-test-{}", std::process::id()));
        let gradient = rmac_wallpaper::BuiltInId::ALL
            .into_iter()
            .find(|id| !id.metadata().has_artwork)
            .expect("a procedural built-in");
        let selection = WallpaperSelection {
            source: Some(format!("builtin:{}", gradient.id())),
            fit: WallpaperFit::Fill,
        };
        let path = desktop_picture(&selection, true, (320, 180), &cache).unwrap();
        assert_eq!(path, cache.join("desktop-picture.png"));
        let picture = image::open(&path).unwrap();
        assert_eq!((picture.width(), picture.height()), (320, 180));
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn the_users_own_jpeg_is_handed_to_windows_as_is() {
        let folder =
            std::env::temp_dir().join(format!("lulo-wallpaper-own-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let photo = folder.join("photo.png");
        image::RgbaImage::from_pixel(4, 4, image::Rgba([10, 20, 30, 255]))
            .save(&photo)
            .unwrap();
        let selection = WallpaperSelection {
            source: Some(photo.to_string_lossy().into_owned()),
            fit: WallpaperFit::Fit,
        };
        assert_eq!(
            desktop_picture(&selection, false, (1920, 1080), &folder.join("cache")).unwrap(),
            photo
        );
        let _ = std::fs::remove_dir_all(&folder);
    }
}
