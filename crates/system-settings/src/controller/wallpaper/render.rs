//! Wallpaper settings presentation, laid out like macOS 26
//! (design-lab/settings.html): a fixed top strip with the current
//! wallpaper's preview beside a group holding its name, fit and target
//! display, a full-width rule, then the gallery scrolling under it in
//! titled sections.

use super::*;
use gpui::Stateful;

/// Measured on macOS 26.2 (AX frames, window 723 wide): the top strip is
/// inset 16 on every side; preview 160 × 100 radius 4, the group 10 after
/// it; the rule is 1 pt across the whole detail column, 16 under the strip.
const TOP_INSET: f32 = 16.0;
const PREVIEW_WIDTH: f32 = 160.0;
const PREVIEW_HEIGHT: f32 = 100.0;
const PREVIEW_RADIUS: f32 = 4.0;
const PREVIEW_GAP: f32 = 10.0;
/// Gallery: content inset 20, section titles 13 bold with the tiles 9
/// under them; tiles 108 wide on the Mac's 117.5 pitch (117 here so four
/// fit the 460 pt column), a 108 × 72 picture (radius 2) over a 10 pt
/// medium name, 100 tall in all; 20 between a section's last tile and the
/// next title.
const GALLERY_INSET: f32 = 20.0;
const THUMB_WIDTH: f32 = 108.0;
const THUMB_HEIGHT: f32 = 72.0;
const THUMB_RADIUS: f32 = 2.0;
const THUMB_TILE_HEIGHT: f32 = 100.0;
const THUMB_GAP: f32 = 9.0;
const SECTION_TITLE_GAP: f32 = 9.0;
const SECTION_GAP: f32 = 20.0;
/// The selected tile's ring: 2 pt, the picture inset 3 inside it.
const THUMB_RING: f32 = 2.0;
const THUMB_RING_INSET: f32 = 3.0;

impl Settings {
    /// The whole Wallpaper detail column. Unlike the other panes only the
    /// gallery scrolls; the preview strip stays put above the rule.
    pub(in crate::controller) fn render_wallpaper(&self, cx: &Context<Self>) -> Stateful<Div> {
        let view = cx.entity();
        let unfocus_view = view.clone();
        let root = div()
            .id("wallpaper-pane")
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .v_flex()
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                unfocus_view.update(cx, |settings, cx| {
                    if settings.sidebar_focused {
                        settings.sidebar_focused = false;
                        cx.notify();
                    }
                });
            });
        let gallery_scroll = |content: Div| {
            div()
                .id(rmac_system_settings::accessibility::DETAIL_ID)
                .flex_1()
                .min_h(px(0.0))
                .w_full()
                .overflow_y_scroll()
                .child(
                    content
                        .w_full()
                        .px(px(GALLERY_INSET))
                        .pt(px(GALLERY_INSET))
                        .pb(px(GALLERY_INSET)),
                )
        };

        if self.shell_settings_loading && self.shell_settings.is_none() {
            return root.child(gallery_scroll(
                div().child(note_card("Loading the authoritative wallpaper settings…")),
            ));
        }
        let Some(snapshot) = self.shell_settings.as_ref() else {
            // The only state that needs a manual retry: the settings store
            // could not be read at all, so there is nothing live to follow.
            let retry_view = view.clone();
            return root.child(gallery_scroll(
                div()
                    .child(note_card(
                        "The versioned Lulo OS shell-settings authority is unavailable. Wallpaper choices remain unchanged.",
                    ))
                    .child(footer_buttons(vec![push_button(
                        "wallpaper-refresh",
                        "Try Again",
                    )
                    .disabled(self.shell_settings_loading || self.shell_settings_busy)
                    .on_click(move |_, _, cx| {
                        retry_view.update(cx, |settings, cx| {
                            settings.refresh_shell_settings(true, cx)
                        });
                    })
                    .into_any_element()])),
            ));
        };
        let wallpaper = &snapshot.settings.wallpaper;
        let (selection, owns_selection) = wallpaper_selection(wallpaper, &self.wallpaper_target);
        let enabled = !self.shell_settings_busy;
        let dark = style::dark();
        let source = rmac_wallpaper::parse_source(selection.source.as_deref()).ok();
        let current_builtin = match &source {
            Some(rmac_wallpaper::Source::BuiltIn(id)) => Some(*id),
            _ => None,
        };

        // Target display: every saved override plus every enabled output.
        let mut output_ids = std::collections::BTreeSet::new();
        output_ids.extend(wallpaper.per_output.keys().cloned());
        if let WallpaperTarget::Output(output) = &self.wallpaper_target {
            output_ids.insert(output.clone());
        }
        output_ids.extend(
            self.dock_compositor
                .outputs
                .values()
                .filter(|output| output.enabled())
                .map(|output| output.id.0.clone()),
        );
        let default_view = view.clone();
        let mut targets = vec![choice(
            "All Displays",
            self.wallpaper_target == WallpaperTarget::Default,
            move |_, cx| {
                default_view.update(cx, |settings, cx| {
                    settings.select_wallpaper_target(WallpaperTarget::Default, cx)
                });
            },
        )];
        for output_id in output_ids {
            let target = WallpaperTarget::Output(output_id.clone());
            let selected = self.wallpaper_target == target;
            let live = self
                .dock_compositor
                .outputs
                .values()
                .any(|output| output.enabled() && output.id.0 == output_id);
            let name = self
                .dock_compositor
                .outputs
                .get(&rmac_compositor::OutputId(output_id.clone()))
                .map(|output| {
                    format!("{} {}", output.make, output.model)
                        .trim()
                        .to_owned()
                })
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| output_id.clone());
            let target_view = view.clone();
            targets.push(choice(
                if live {
                    name
                } else {
                    format!("{name} (Offline)")
                },
                selected,
                move |_, cx| {
                    target_view.update(cx, |settings, cx| {
                        settings.select_wallpaper_target(target.clone(), cx)
                    });
                },
            ));
        }
        let current_target = popup_value(&targets, "All Displays");

        // The preview: the rendered selection, or, for a built-in whose
        // large artwork is not installed, the same thumbnail-over-swatch
        // the gallery shows. Only a chosen file with nothing to show says so.
        let preview = div()
            .w(px(PREVIEW_WIDTH))
            .h(px(PREVIEW_HEIGHT))
            .flex_none()
            .rounded(px(PREVIEW_RADIUS))
            .overflow_hidden()
            .bg(style::well_fill());
        let preview = match (self.wallpaper_preview.clone(), current_builtin) {
            (Some(image), _) if self.wallpaper_preview_error.is_none() => {
                preview.child(img(image).w_full().h_full().object_fit(ObjectFit::Cover))
            }
            (_, Some(id)) => preview.child(builtin_picture(id, dark)),
            (image, None) => preview
                .when_some(image, |preview, image| {
                    preview.child(img(image).w_full().h_full().object_fit(ObjectFit::Cover))
                })
                .when(self.wallpaper_preview.is_none(), |preview| {
                    preview.flex().items_center().justify_center().child(
                        div()
                            .text_size(rmac_ui::text_px(11.0))
                            .text_color(secondary())
                            .child(if self.wallpaper_preview_loading {
                                "Preparing preview…"
                            } else {
                                "Preview unavailable"
                            }),
                    )
                }),
        };
        let source_name = wallpaper_source_name(&selection);
        let summary = card(vec![
            wallpaper_fit_row(view.clone(), source_name.clone(), selection.fit, enabled),
            popup_row(
                "wallpaper-target",
                "Show on",
                Some(match &self.wallpaper_target {
                    WallpaperTarget::Default => "Default for every display".into(),
                    WallpaperTarget::Output(_) if owns_selection => {
                        "Custom choice for this display".into()
                    }
                    WallpaperTarget::Output(_) => "Inherited from the default".into(),
                }),
                current_target,
                targets,
                enabled,
            ),
        ])
        .mb(px(0.0));
        let use_default = matches!(self.wallpaper_target, WallpaperTarget::Output(_)).then(|| {
            let use_default_view = view.clone();
            div().mt(px(PREVIEW_GAP)).flex().justify_end().child(
                push_button("wallpaper-use-default", "Use Default")
                    .disabled(!enabled || !owns_selection)
                    .on_click(move |_, _, cx| {
                        use_default_view.update(cx, |settings, cx| {
                            let target = settings.wallpaper_target.clone();
                            settings.apply_wallpaper_change(target, WallpaperChange::UseDefault, cx)
                        });
                    }),
            )
        });
        let top = div()
            .flex_none()
            .flex()
            .items_start()
            .gap(px(PREVIEW_GAP))
            .p(px(TOP_INSET))
            .child(preview)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .child(summary)
                    .children(use_default),
            );

        // Notes that need the user: a chosen file that cannot be shown, and
        // outputs whose state cannot be confirmed.
        let mut notes = Vec::new();
        if current_builtin.is_none() {
            for error in [
                self.wallpaper_preview_error.clone(),
                self.wallpaper_preview_watch_error.clone(),
            ]
            .into_iter()
            .flatten()
            {
                notes.push(note_card(rmac_ui::user_error_message(
                    rmac_ui::ErrorSurface::Settings,
                    error.as_ref(),
                    false,
                )));
            }
        }
        if let WallpaperTarget::Output(output) = &self.wallpaper_target {
            let live = self
                .dock_compositor
                .outputs
                .values()
                .any(|candidate| candidate.enabled() && candidate.id.0 == *output);
            if !live {
                notes.push(note_card(
                    "This output is currently unplugged or disabled. Its override remains authoritative and will return when the same stable niri output ID reappears.",
                ));
            }
        }
        if self.dock_compositor.connection != rmac_compositor::ConnectionState::Connected {
            notes.push(note_card(
                "niri is not connected in this process. Saved per-output choices remain editable, but live output availability cannot be confirmed.",
            ));
        }

        // The gallery: the Lulo artwork, the user's own picture, then the
        // procedural gradients, each a titled run of tiles.
        let builtin_tile = |id: rmac_wallpaper::BuiltInId| {
            let using = current_builtin == Some(id);
            let tile_view = view.clone();
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
                    tile_view.update(cx, |settings, cx| {
                        let target = settings.wallpaper_target.clone();
                        settings.apply_wallpaper_change(target, change, cx);
                    });
                })
            })
        };
        let (artwork, gradients): (Vec<_>, Vec<_>) = rmac_wallpaper::BuiltInId::ALL
            .into_iter()
            .partition(|id| id.metadata().has_artwork);

        let mut photos = Vec::new();
        if let Some(rmac_wallpaper::Source::File(_)) = &source {
            let picture = div().size_full().bg(style::well_fill()).when_some(
                self.wallpaper_preview.clone(),
                |picture, image| {
                    picture.child(img(image).w_full().h_full().object_fit(ObjectFit::Cover))
                },
            );
            photos.push(gallery_tile(
                "wallpaper-current-file",
                picture,
                source_name,
                true,
            ));
        }
        let choose_view = view.clone();
        photos.push(
            add_photo_tile().when(enabled && !self.shell_settings_loading, |tile| {
                tile.cursor_pointer().on_click(move |_, _, cx| {
                    choose_view.update(cx, |settings, cx| settings.choose_wallpaper_file(cx));
                })
            }),
        );

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

/// A built-in's picture: its packaged thumbnail over a swatch of its own
/// colours, so the swatch shows through when the image is not installed.
fn builtin_picture(id: rmac_wallpaper::BuiltInId, dark: bool) -> Div {
    let metadata = id.metadata();
    let palette = metadata.palette_for(dark);
    div()
        .size_full()
        .bg(gpui::linear_gradient(
            135.0,
            gpui::linear_color_stop(hsl(palette[0]), 0.0),
            gpui::linear_color_stop(hsl(palette[3]), 1.0),
        ))
        .when_some(metadata.thumbnail_path(dark), |picture, path| {
            picture.child(img(path).w_full().h_full().object_fit(ObjectFit::Cover))
        })
}

/// A titled run of gallery tiles that wraps onto as many lines as it needs.
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
                .gap_y(px(SECTION_TITLE_GAP))
                .children(tiles),
        )
}

/// One gallery tile: the picture with the Mac's selection ring, and its
/// name centred under it.
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
                        .border_color(accent())
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

/// "Your Photos" › Add Photo…: a 72 pt well with a picture glyph and the
/// label inside it, the Mac's way to use a picture of your own.
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
