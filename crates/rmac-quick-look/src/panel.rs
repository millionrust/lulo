//! The Quick Look panel: a floating window, sized to what it shows, with the
//! Mac's title bar (close, full screen, previous/next, name, index sheet,
//! "Open with …" or "Uncompress").

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use gpui::{
    div, img, prelude::FluentBuilder as _, px, rgb, rgba, size, svg, AnyElement, App,
    AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, WindowBackgroundAppearance,
    WindowBounds, WindowHandle,
};
use gpui_component::Root;

use crate::content::{self, Content, Picture, Summary};
use crate::metrics;

/// Wayland app id; `packaging/rmac-session/shell.kdl` gives it the measured
/// 20 pt radius and the glass blur.
pub const APP_ID: &str = "org.rmac.QuickLook";

const CLOSE_GLYPH: &str = "icons/quick-look/close.svg";
const FULL_SCREEN_GLYPH: &str = "icons/quick-look/full-screen.svg";
const PREVIOUS_GLYPH: &str = "icons/quick-look/chevron-left.svg";
const NEXT_GLYPH: &str = "icons/quick-look/chevron-right.svg";
const INDEX_GLYPH: &str = "icons/quick-look/index-sheet.svg";
const DOCUMENT_GLYPH: &str = "icons/quick-look/document.svg";
const FOLDER_GLYPH: &str = "icons/quick-look/folder.svg";

// Measured colours (design-lab/quick-look.html).
const BODY: u32 = 0x1f1f27;
const SUMMARY_BODY: u32 = 0x1e1f28;
/// Title-bar glass: rgb(30,30,45) at 86 % so niri's blur shows through (S).
const BAR: u32 = 0x1e1e2ddb;
const RIM: u32 = 0x646473;
const CIRCLE: u32 = 0x9998aa;
const BUTTON_GLYPH: u32 = 0x9999a1;
const TITLE: u32 = 0xe8e8ea;
const NAV_FILL: u32 = 0x373748;
const NAV_DIVIDER: u32 = 0x5a5a6a;
const CHEVRON: u32 = 0xebebed;
const OPEN_FILL: u32 = 0x2d2d3c;
const OPEN_TEXT: u32 = 0xe0dfe2;
const UNCOMPRESS_FILL: u32 = 0x34353c;
const TEXT_BACKGROUND: u32 = 0x1e1e1e;
const TEXT_COLOUR: u32 = 0xffffff;
const SECONDARY: u32 = 0x9a9ba5;
const INDEX_BACKGROUND: u32 = 0x1f1f28;
const SELECTION: u32 = 0x2458ca;
const FOLDER_BLUE: u32 = 0x2ea6f2;
const DOCUMENT_WHITE: u32 = 0xe6e6ea;

/// What the owner lets the panel do.
#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    /// Show "Uncompress" for archives and report it with
    /// [`Event::Uncompress`] (Files expands with its progress sheet).
    pub uncompress: bool,
}

/// Events for the window that opened the panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The panel moved to this item (arrows, index sheet).
    Current(PathBuf),
    /// The user asked to expand this archive.
    Uncompress(PathBuf),
}

enum Load {
    Loading,
    Ready(Content),
    Failed(SharedString),
}

#[derive(Clone)]
enum Thumb {
    Pending,
    Picture { picture: Picture, aspect: f32 },
    Icon { folder: bool },
}

pub struct QuickLook {
    items: Vec<PathBuf>,
    current: usize,
    state: Load,
    cancel: Arc<AtomicBool>,
    generation: u64,
    index_sheet: bool,
    thumbs: Vec<Thumb>,
    thumbs_requested: bool,
    /// While stepping through a selection the Mac keeps the first item's
    /// box: later items fit a square of its longest side.
    selection_box: Option<(f32, f32)>,
    last_content: Option<(f32, f32)>,
    open_with: Option<SharedString>,
    options: Options,
    focus: FocusHandle,
}

impl EventEmitter<Event> for QuickLook {}

impl Focusable for QuickLook {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// The open panel, as seen from the window that opened it.
#[derive(Clone)]
pub struct Handle {
    window: WindowHandle<Root>,
    view: WeakEntity<QuickLook>,
}

impl Handle {
    pub fn is_open(&self) -> bool {
        self.view.upgrade().is_some()
    }

    /// Previous (−1) or next (+1) item, wrapping like the Mac.
    pub fn step(&self, delta: isize, cx: &mut App) {
        self.with(cx, |panel, window, cx| panel.step(delta, window, cx));
    }

    /// Show another selection in the same panel.
    pub fn show(&self, items: Vec<PathBuf>, current: usize, cx: &mut App) {
        self.with(cx, move |panel, window, cx| {
            panel.show(items, current, window, cx)
        });
    }

    pub fn close(&self, cx: &mut App) {
        let _ = self
            .window
            .update(cx, |_, window, _| window.remove_window());
    }

    fn with(
        &self,
        cx: &mut App,
        update: impl FnOnce(&mut QuickLook, &mut Window, &mut Context<QuickLook>),
    ) {
        let Some(view) = self.view.upgrade() else {
            return;
        };
        let _ = self.window.update(cx, |_, window, cx| {
            view.update(cx, |panel, cx| update(panel, window, cx))
        });
    }
}

/// Open Quick Look on `items`, starting at `current`. Returns the handle and
/// the panel entity (subscribe to [`Event`], observe its release to learn
/// that it closed).
pub fn open(
    items: Vec<PathBuf>,
    current: usize,
    options: Options,
    cx: &mut App,
) -> Option<(Handle, Entity<QuickLook>)> {
    if items.is_empty() {
        return None;
    }
    let current = current.min(items.len() - 1);
    let limits = primary_limits(cx);
    let (width, height) = first_guess(&items[current], limits);
    let mut window_options =
        rmac_ui::window_options_for_app_with_title(APP_ID, "Quick Look", width, height, cx);
    window_options.window_bounds = Some(WindowBounds::centered(size(px(width), px(height)), cx));
    window_options.window_background = WindowBackgroundAppearance::Blurred;
    window_options.window_min_size = Some(size(
        px(metrics::MIN_CONTENT.0 + 2.0 * metrics::INSET),
        px(metrics::MIN_CONTENT.1 + metrics::TITLE_BAR + metrics::INSET),
    ));
    window_options.is_minimizable = false;
    let mut created = None;
    let window = cx
        .open_window(window_options, |window, cx| {
            rmac_ui::prepare_surface_window(window, cx);
            let view = cx.new(|cx| QuickLook::new(items, current, options, window, cx));
            let focus = view.read(cx).focus.clone();
            window.focus(&focus, cx);
            created = Some(view.clone());
            cx.new(|cx| rmac_ui::shell_surface_root(view, window, cx))
        })
        .ok()?;
    let view = created?;
    Some((
        Handle {
            window,
            view: view.downgrade(),
        },
        view,
    ))
}

impl QuickLook {
    fn new(
        items: Vec<PathBuf>,
        current: usize,
        options: Options,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let count = items.len();
        let mut panel = Self {
            items,
            current,
            state: Load::Loading,
            cancel: Arc::new(AtomicBool::new(false)),
            generation: 0,
            index_sheet: false,
            thumbs: vec![Thumb::Pending; count],
            thumbs_requested: false,
            selection_box: None,
            last_content: None,
            open_with: None,
            options,
            focus: cx.focus_handle(),
        };
        panel.load_current(window, cx);
        panel
    }

    pub fn current_path(&self) -> Option<&Path> {
        self.items.get(self.current).map(PathBuf::as_path)
    }

    pub fn show(
        &mut self,
        items: Vec<PathBuf>,
        current: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if items.is_empty() {
            self.close(window, cx);
            return;
        }
        self.current = current.min(items.len() - 1);
        self.thumbs = vec![Thumb::Pending; items.len()];
        self.items = items;
        self.thumbs_requested = false;
        self.index_sheet = false;
        self.selection_box = None;
        self.load_current(window, cx);
    }

    pub fn step(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.items.len();
        if count < 2 {
            return;
        }
        let next = (self.current as isize + delta).rem_euclid(count as isize) as usize;
        self.select(next, window, cx);
    }

    fn select(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.current = index.min(self.items.len().saturating_sub(1));
        self.index_sheet = false;
        self.load_current(window, cx);
    }

    fn close(&mut self, window: &mut Window, _: &mut Context<Self>) {
        self.cancel.store(true, Ordering::Release);
        window.remove_window();
    }

    fn load_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel.store(true, Ordering::Release);
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.state = Load::Loading;
        self.open_with = None;
        let Some(path) = self.items.get(self.current).cloned() else {
            return;
        };
        let limits = self
            .selection_box
            .unwrap_or_else(|| window_limits(window, cx));
        cx.emit(Event::Current(path.clone()));
        cx.notify();
        let load_path = path.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { content::load(&load_path, limits, &cancel) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(content) => this.apply(content, window, cx),
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(error) => {
                        this.state = Load::Failed(content::error_message(&error).into());
                        this.resize_to(metrics::window_for_content(metrics::TEXT_CONTENT), window);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        self.load_open_with(path, cx);
    }

    fn apply(&mut self, content: Content, window: &mut Window, cx: &mut Context<Self>) {
        let window_size = match content.natural_size() {
            None => metrics::SUMMARY_WINDOW,
            Some(natural) => {
                let limits = self
                    .selection_box
                    .unwrap_or_else(|| window_limits(window, cx));
                let shown = match (&content, self.last_content) {
                    // Text keeps the panel's size (measured stepping onto a
                    // text file in a selection).
                    (Content::Text { .. }, Some(previous)) => previous,
                    _ => metrics::fit(natural, limits),
                };
                if self.items.len() > 1 && self.selection_box.is_none() {
                    let side = shown.0.max(shown.1);
                    self.selection_box = Some((side, side));
                }
                self.last_content = Some(shown);
                metrics::window_for_content(shown)
            }
        };
        self.resize_to(window_size, window);
        let pdf = matches!(content, Content::Pdf { .. });
        self.state = Load::Ready(content);
        if pdf {
            self.render_more_pages(window, cx);
        }
    }

    fn resize_to(&self, window_size: (f32, f32), window: &mut Window) {
        if !window.is_fullscreen() {
            window.resize(size(px(window_size.0), px(window_size.1)));
        }
    }

    /// Render the rest of a PDF one page at a time, after the first page is
    /// already on screen.
    fn render_more_pages(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Load::Ready(Content::Pdf {
            path,
            pages,
            page_sizes,
            pixel_scale,
        }) = &self.state
        else {
            return;
        };
        let next = pages.len();
        if next >= page_sizes.len().min(metrics::MAX_PDF_PAGES) {
            return;
        }
        let path = path.clone();
        let page_size = page_sizes[next];
        let pixel_scale = *pixel_scale;
        let generation = self.generation;
        let cancel = self.cancel.clone();
        cx.spawn_in(window, async move |this, cx| {
            let page = cx
                .background_executor()
                .spawn(async move {
                    if cancel.load(Ordering::Acquire) {
                        None
                    } else {
                        content::render_pdf_page(&path, next, page_size, pixel_scale).ok()
                    }
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation {
                    return;
                }
                let Some(page) = page else {
                    return;
                };
                if let Load::Ready(Content::Pdf { pages, .. }) = &mut this.state {
                    if pages.len() == next {
                        pages.push(page);
                    }
                }
                cx.notify();
                this.render_more_pages(window, cx);
            });
        })
        .detach();
    }

    /// "Open with <default app>", from the XDG association.
    fn load_open_with(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if path.is_dir() {
            return;
        }
        if self.options.uncompress && rmac_archive::format_of(&path).is_some() {
            return;
        }
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let label =
                rmac_app_launch::file_association(path)
                    .await
                    .ok()
                    .and_then(|association| {
                        let default = association.default_application_id?;
                        association
                            .handlers
                            .into_iter()
                            .find(|handler| handler.id == default)
                            .map(|handler| {
                                SharedString::from(format!("Open with {}", handler.name))
                            })
                    });
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.open_with = label;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn open_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.items.get(self.current).cloned() else {
            return;
        };
        cx.spawn(async move |_, _: &mut gpui::AsyncApp| {
            if let Err(error) = rmac_app_launch::open_item(path).await {
                eprintln!("rmac Quick Look: {error}");
            }
        })
        .detach();
        self.close(window, cx);
    }

    fn uncompress_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.items.get(self.current).cloned() else {
            return;
        };
        cx.emit(Event::Uncompress(path));
        self.close(window, cx);
    }

    fn toggle_index_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.index_sheet = !self.index_sheet;
        if self.index_sheet && !self.thumbs_requested {
            self.thumbs_requested = true;
            self.load_thumbs(window, cx);
        }
        cx.notify();
    }

    /// Index-sheet pictures, one item at a time off the UI thread.
    fn load_thumbs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.items.clone();
        let cancel = self.cancel.clone();
        cx.spawn_in(window, async move |this, cx| {
            for (index, path) in items.into_iter().enumerate() {
                let cancel = cancel.clone();
                let thumb = cx
                    .background_executor()
                    .spawn(async move { load_thumb(&path, &cancel) })
                    .await;
                let keep_going = this
                    .update(cx, |this, cx| {
                        if let Some(slot) = this.thumbs.get_mut(index) {
                            *slot = thumb;
                        }
                        cx.notify();
                    })
                    .is_ok();
                if !keep_going {
                    break;
                }
            }
        })
        .detach();
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        match keystroke.key.as_str() {
            "space" if keystroke.modifiers.alt => window.toggle_fullscreen(),
            "space" | "escape" => self.close(window, cx),
            "left" if !self.index_sheet => self.step(-1, window, cx),
            "right" if !self.index_sheet => self.step(1, window, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn current_name(&self) -> SharedString {
        self.current_path()
            .and_then(Path::file_name)
            .map(|name| SharedString::from(name.to_string_lossy().into_owned()))
            .unwrap_or_else(|| SharedString::from("/"))
    }

    fn is_summary(&self) -> bool {
        !self.index_sheet && matches!(self.state, Load::Ready(Content::Summary(_)))
    }

    fn render_title_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let multi = self.items.len() > 1;
        let index = self.index_sheet;
        let summary = self.is_summary();
        let archive = matches!(
            &self.state,
            Load::Ready(Content::Summary(Summary { archive: true, .. }))
        );
        let title_x = if index {
            metrics::TITLE_X_INDEX_SHEET
        } else if multi {
            metrics::TITLE_X_AFTER_NAV
        } else {
            metrics::TITLE_X
        };
        let show_uncompress = archive && self.options.uncompress && !index;
        let open_label = self
            .open_with
            .clone()
            .filter(|_| !index && !show_uncompress);
        // Room the title leaves for the right-hand buttons (S: estimated).
        let reserved = metrics::RIGHT_MARGIN
            + if show_uncompress {
                metrics::UNCOMPRESS_WIDTH + metrics::BUTTON_GAP
            } else if open_label.is_some() {
                190.0
            } else {
                0.0
            }
            + if multi && !index {
                metrics::BUTTON_WIDTH + metrics::BUTTON_GAP
            } else {
                0.0
            };

        let mut right = div()
            .absolute()
            .right(px(metrics::RIGHT_MARGIN))
            .top(px(metrics::UNCOMPRESS_Y))
            .h(px(metrics::UNCOMPRESS_HEIGHT))
            .flex()
            .items_center()
            .gap(px(metrics::BUTTON_GAP));
        if multi && !index {
            right = right.child(
                div()
                    .id("quick-look-index-sheet")
                    .w(px(metrics::BUTTON_WIDTH))
                    .h(px(metrics::BUTTON_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(glyph(INDEX_GLYPH, 16.0, BUTTON_GLYPH))
                    .on_click(
                        cx.listener(|this, _, window, cx| this.toggle_index_sheet(window, cx)),
                    ),
            );
        }
        if show_uncompress {
            right = right.child(
                div()
                    .id("quick-look-uncompress")
                    .w(px(metrics::UNCOMPRESS_WIDTH))
                    .h(px(metrics::UNCOMPRESS_HEIGHT))
                    .rounded(px(metrics::UNCOMPRESS_HEIGHT / 2.0))
                    .bg(rgb(UNCOMPRESS_FILL))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(rgb(OPEN_TEXT))
                    .child("Uncompress")
                    .on_click(
                        cx.listener(|this, _, window, cx| this.uncompress_current(window, cx)),
                    ),
            );
        } else if let Some(label) = open_label {
            right = right.child(
                div()
                    .id("quick-look-open")
                    .h(px(metrics::BUTTON_HEIGHT))
                    .px(px(metrics::OPEN_PADDING))
                    .rounded(px(metrics::BUTTON_HEIGHT / 2.0))
                    .bg(rgb(OPEN_FILL))
                    .flex()
                    .items_center()
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(rgb(OPEN_TEXT))
                    .whitespace_nowrap()
                    .child(label)
                    .on_click(cx.listener(|this, _, window, cx| this.open_current(window, cx))),
            );
        }

        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(metrics::TITLE_BAR))
            .when(!summary, |bar| bar.bg(rgba(BAR)))
            .child(
                circle_button("quick-look-close", CLOSE_GLYPH, metrics::CLOSE_X)
                    .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
            )
            .when(!index, |bar| {
                bar.child(
                    circle_button(
                        "quick-look-full-screen",
                        FULL_SCREEN_GLYPH,
                        metrics::FULL_SCREEN_X,
                    )
                    .on_click(|_, window, _| window.toggle_fullscreen()),
                )
            })
            .when(multi && !index, |bar| {
                bar.child(
                    div()
                        .absolute()
                        .left(px(metrics::NAV_X))
                        .top(px(metrics::NAV_Y))
                        .w(px(metrics::NAV_WIDTH))
                        .h(px(metrics::NAV_HEIGHT))
                        .rounded(px(metrics::NAV_HEIGHT / 2.0))
                        .bg(rgb(NAV_FILL))
                        .flex()
                        .items_center()
                        .child(
                            div()
                                .id("quick-look-previous")
                                .w(px(metrics::NAV_DIVIDER_X))
                                .h_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(glyph(PREVIOUS_GLYPH, 16.0, CHEVRON))
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.step(-1, window, cx)),
                                ),
                        )
                        .child(div().w(px(1.0)).h(px(14.0)).bg(rgb(NAV_DIVIDER)))
                        .child(
                            div()
                                .id("quick-look-next")
                                .flex_1()
                                .h_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(glyph(NEXT_GLYPH, 16.0, CHEVRON))
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.step(1, window, cx)),
                                ),
                        ),
                )
            })
            .when(!summary, |bar| {
                bar.child(
                    div()
                        .absolute()
                        .left(px(title_x))
                        .right(px(reserved))
                        .top(px(metrics::TITLE_Y))
                        .h(px(metrics::TITLE_HEIGHT))
                        .line_height(px(metrics::TITLE_HEIGHT))
                        .text_size(rmac_ui::text_px(metrics::TITLE_SIZE))
                        .font_weight(rmac_ui::mac::BOLD)
                        .text_color(rgb(TITLE))
                        .truncate()
                        .child(self.current_name()),
                )
            })
            .child(right)
            .into_any_element()
    }

    fn render_body(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let frame = || {
            div()
                .absolute()
                .left(px(metrics::INSET))
                .right(px(metrics::INSET))
                .top(px(metrics::TITLE_BAR))
                .bottom(px(metrics::INSET))
                .rounded(px(metrics::CONTENT_RADIUS))
                .overflow_hidden()
        };
        let content_width = (f32::from(window.bounds().size.width) - 2.0 * metrics::INSET).max(1.0);
        if self.index_sheet {
            return frame()
                .bg(rgb(INDEX_BACKGROUND))
                .child(self.render_index_sheet(content_width, cx))
                .into_any_element();
        }
        match &self.state {
            Load::Loading => frame().into_any_element(),
            Load::Failed(message) => frame()
                .flex()
                .items_center()
                .justify_center()
                .px(px(24.0))
                .text_center()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(rgb(SECONDARY))
                .child(message.clone())
                .into_any_element(),
            Load::Ready(Content::Image { picture, .. }) => frame()
                .child(img(picture.source()).size_full())
                .into_any_element(),
            Load::Ready(Content::Pdf {
                pages, page_sizes, ..
            }) => frame()
                .bg(rgb(TEXT_BACKGROUND))
                .child(
                    div()
                        .id("quick-look-pdf")
                        .size_full()
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap(px(metrics::PDF_PAGE_GAP))
                        .children(pages.iter().zip(page_sizes).map(|(page, page_size)| {
                            let height = content_width * page_size.1 / page_size.0.max(1.0);
                            img(page.clone())
                                .flex_none()
                                .w(px(content_width))
                                .h(px(height))
                        })),
                )
                .into_any_element(),
            Load::Ready(Content::Text { text, .. }) => frame()
                .bg(rgb(TEXT_BACKGROUND))
                .child(
                    div()
                        .id("quick-look-text")
                        .size_full()
                        .overflow_y_scroll()
                        .px(px(metrics::TEXT_PADDING_X))
                        .pt(px(metrics::TEXT_PADDING_TOP))
                        .font_family(rmac_ui::MONO_FONT)
                        .text_size(rmac_ui::text_px(metrics::TEXT_SIZE))
                        .line_height(px(metrics::TEXT_LINE))
                        .text_color(rgb(TEXT_COLOUR))
                        .child(text.clone()),
                )
                .into_any_element(),
            Load::Ready(Content::Summary(summary)) => self.render_summary(summary),
        }
    }

    fn render_summary(&self, summary: &Summary) -> AnyElement {
        let (icon, colour) = if summary.folder {
            (FOLDER_GLYPH, FOLDER_BLUE)
        } else {
            (DOCUMENT_GLYPH, DOCUMENT_WHITE)
        };
        div()
            .absolute()
            .inset_0()
            .child(
                div()
                    .absolute()
                    .left(px(metrics::SUMMARY_ICON_X))
                    .top(px(metrics::SUMMARY_ICON_Y))
                    .child(glyph(icon, metrics::SUMMARY_ICON_SIZE, colour)),
            )
            .child(
                div()
                    .absolute()
                    .left(px(metrics::SUMMARY_TEXT_X))
                    .top(px(metrics::SUMMARY_NAME_Y))
                    .w(px(metrics::SUMMARY_TEXT_WIDTH))
                    .line_height(px(26.0))
                    .text_size(rmac_ui::text_px(metrics::SUMMARY_NAME_SIZE))
                    .font_weight(rmac_ui::mac::BOLD)
                    .text_color(rgb(TEXT_COLOUR))
                    .truncate()
                    .child(self.current_name()),
            )
            .child(summary_line(
                metrics::SUMMARY_LINE_2_Y,
                summary.detail.clone(),
            ))
            .when_some(summary.modified.clone(), |element, modified| {
                element.child(summary_line(metrics::SUMMARY_LINE_3_Y, modified))
            })
            .into_any_element()
    }

    fn render_index_sheet(&self, width: f32, cx: &mut Context<Self>) -> AnyElement {
        let aspects: Vec<f32> = self
            .thumbs
            .iter()
            .map(|thumb| match thumb {
                Thumb::Picture { aspect, .. } => *aspect,
                _ => 1.0,
            })
            .collect();
        let (cells, height) = metrics::index_layout(&aspects, width);
        div()
            .id("quick-look-index-grid")
            .size_full()
            .overflow_y_scroll()
            .child(div().relative().w_full().h(px(height)).children(
                cells.into_iter().enumerate().map(|(index, cell)| {
                    let thumb = self.thumbs.get(index).cloned().unwrap_or(Thumb::Pending);
                    div()
                        .id(("quick-look-index-item", index))
                        .absolute()
                        .left(px(cell.x))
                        .top(px(cell.y))
                        .w(px(cell.width))
                        .h(px(cell.height))
                        .flex()
                        .items_center()
                        .justify_center()
                        .map(|element| match thumb {
                            Thumb::Picture { picture, .. } => {
                                element.child(img(picture.source()).size_full())
                            }
                            Thumb::Icon { folder } => element.child(if folder {
                                glyph(FOLDER_GLYPH, 128.0, FOLDER_BLUE)
                            } else {
                                glyph(DOCUMENT_GLYPH, 128.0, DOCUMENT_WHITE)
                            }),
                            Thumb::Pending => element,
                        })
                        .when(index == self.current, |element| {
                            element.child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .border_4()
                                    .border_color(rgb(SELECTION)),
                            )
                        })
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.select(index, window, cx)),
                        )
                }),
            ))
            .into_any_element()
    }
}

impl Render for QuickLook {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let summary = self.is_summary();
        let radius = if window.is_fullscreen() {
            0.0
        } else {
            metrics::WINDOW_RADIUS
        };
        div()
            .id("quick-look")
            .key_context("QuickLook")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .size_full()
            .relative()
            .overflow_hidden()
            .rounded(px(radius))
            .bg(rgb(if summary { SUMMARY_BODY } else { BODY }))
            .border_1()
            .border_color(rgb(RIM))
            .child(self.render_body(window, cx))
            .child(self.render_title_bar(cx))
    }
}

fn glyph(path: &'static str, side: f32, colour: u32) -> gpui::Svg {
    svg()
        .path(path)
        .w(px(side))
        .h(px(side))
        .flex_none()
        .text_color(rgb(colour))
}

fn circle_button(id: &'static str, path: &'static str, x: f32) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .absolute()
        .left(px(x))
        .top(px(metrics::CIRCLE_Y))
        .w(px(metrics::CIRCLE_SIZE))
        .h(px(metrics::CIRCLE_SIZE))
        .child(glyph(path, metrics::CIRCLE_SIZE, CIRCLE))
}

fn summary_line(top: f32, text: String) -> gpui::Div {
    div()
        .absolute()
        .left(px(metrics::SUMMARY_TEXT_X))
        .top(px(top))
        .w(px(metrics::SUMMARY_TEXT_WIDTH))
        .line_height(px(16.0))
        .text_size(rmac_ui::text_px(13.0))
        .text_color(rgb(SECONDARY))
        .whitespace_nowrap()
        .child(text)
}

/// The largest content frame on the window's screen.
fn window_limits(window: &Window, cx: &App) -> (f32, f32) {
    window
        .display(cx)
        .or_else(|| cx.primary_display())
        .map(|display| {
            let visible = display.visible_bounds().size;
            metrics::content_limits((f32::from(visible.width), f32::from(visible.height)))
        })
        .unwrap_or_else(|| metrics::content_limits((1470.0, 923.0)))
}

fn primary_limits(cx: &App) -> (f32, f32) {
    cx.primary_display()
        .map(|display| {
            let visible = display.visible_bounds().size;
            metrics::content_limits((f32::from(visible.width), f32::from(visible.height)))
        })
        .unwrap_or_else(|| metrics::content_limits((1470.0, 923.0)))
}

/// A first window size from cheap facts (file type and image header), so the
/// panel rarely has to resize once its content arrives.
fn first_guess(path: &Path, limits: (f32, f32)) -> (f32, f32) {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return metrics::window_for_content(metrics::TEXT_CONTENT);
    };
    if metadata.is_dir()
        || metadata.file_type().is_symlink()
        || rmac_archive::format_of(path).is_some()
    {
        return metrics::SUMMARY_WINDOW;
    }
    match rmac_preview::render::sniff_path(path) {
        Ok(rmac_preview::document::Kind::Image(_)) => rmac_preview::render::image_dimensions(path)
            .map(|(width, height)| {
                metrics::window_for_content(metrics::fit((width as f32, height as f32), limits))
            })
            .unwrap_or_else(|| metrics::window_for_content(metrics::TEXT_CONTENT)),
        // US Letter until pdfinfo reports the real page (S).
        Ok(rmac_preview::document::Kind::Pdf) => {
            metrics::window_for_content(metrics::fit((612.0, 792.0), limits))
        }
        Err(_) => metrics::window_for_content(metrics::TEXT_CONTENT),
    }
}

fn load_thumb(path: &Path, cancel: &AtomicBool) -> Thumb {
    if cancel.load(Ordering::Acquire) {
        return Thumb::Pending;
    }
    if path.is_dir() {
        return Thumb::Icon { folder: true };
    }
    let preview = if rmac_thumbnails::is_supported(path) {
        rmac_thumbnails::generate_preview(path).ok()
    } else if rmac_thumbnails::media_kind(path).is_some() {
        rmac_thumbnails::generate_media_preview(path, cancel)
            .ok()
            .map(|media| media.preview)
    } else {
        None
    };
    match preview.and_then(|preview| {
        rmac_preview::render::image_dimensions(&preview)
            .map(|(width, height)| (preview, height as f32 / (width.max(1) as f32)))
    }) {
        Some((preview, aspect)) => Thumb::Picture {
            picture: Picture::File(preview),
            aspect,
        },
        None => Thumb::Icon { folder: false },
    }
}
