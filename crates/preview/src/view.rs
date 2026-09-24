//! The Preview window: unified toolbar, thumbnail sidebar, the document
//! (an image, or PDF pages in continuous scroll), search and Get Info.
//!
//! ## Markup and signing (not implemented — design notes)
//!
//! Mac Preview's Markup (Show Markup Toolbar ⇧⌘A) and Signature tools are
//! deliberately out of scope here; they are the largest remaining Preview
//! gap and need their own pass. Sketch of how they would fit this module:
//!
//! - **Data model**: a new `Annotation` enum (Highlight, Underline,
//!   StrikeThrough, Rectangle, Oval, Line, Arrow, Text, Note, Signature),
//!   each carrying a page index and a unit-rect or point path, kept in
//!   `Slot` next to `text` — a fourth `SlotState`-adjacent field, e.g.
//!   `annotations: Vec<Annotation>`, undo/redo as a simple command stack.
//! - **Persisting them**: rmac has no PDF *writer* (`rmac_print::render_pdf`
//!   only rasterises plain text; `crate::pdfwriter` only wraps one raster
//!   image). Baking annotations into the saved PDF would need a proper
//!   incremental-update PDF writer (new page content streams plus `/Annots`
//!   objects) — realistically a small vendored writer or a `lopdf`-style
//!   dependency, not a from-scratch format like `pdfwriter`'s.
//! - **Toolbar**: a second capsule row under the title bar (Show Markup
//!   Toolbar ⇧⌘A toggles it), tool buttons mirroring `metrics::` capsule
//!   sizing, a colour/line-width popover.
//! - **Drawing**: the free-form pen tool needs point-sampled mouse capture
//!   like `text_mouse_down`/`text_mouse_move` already do for selection, but
//!   accumulating a path instead of a text range.
//! - **Signature**: Preview offers trackpad drawing, camera capture, and a
//!   typed cursive font; only the typed-text path is feasible without new
//!   camera/trackpad-gesture plumbing, so that would ship first.
//!
//! None of this is started; `crate::pdfwriter`'s image-in-PDF writer is the
//! one piece already built that a later pass could extend toward it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{
    div, img, point, prelude::FluentBuilder as _, px, rgb, rgba, svg, AnyElement, AppContext as _,
    ClickEvent, ClipboardItem, Context, Entity, FocusHandle, Focusable as _, FontWeight, Image,
    ImageFormat, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement as _, Render, RenderImage, ScrollHandle,
    ScrollWheelEvent, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    WindowControlArea,
};
use rmac_preview::document::{self, Kind};
use rmac_preview::layout::{self, Rect, Rotation, ThumbItem};
use rmac_preview::metrics::{self, dark, light};
use rmac_preview::poppler::{self, Match, TextPage};
use rmac_preview::zoom::{self, ContentKind, Zoom};
use rmac_ui::{mac, InputEvent, InputState};

use crate::{
    ActualSize, CloseWindow, Copy, Find, FindNext, FindPrevious, GoToPage, HideSidebar, NextItem,
    PreviousItem, PrintDocument, RotateLeft, RotateRight, SelectAll, ShowInspector, ShowThumbnails,
    ZoomIn, ZoomOut, ZoomToFit,
};
use rmac_preview::render::{self, Content, Loaded};

/// pdftoppm processes allowed at once (a low-end PC has few cores).
const MAX_RENDERS: usize = 2;
/// Full-size page bitmaps and sidebar thumbnails kept per document.
const MAX_PAGE_BITMAPS: usize = 8;
const MAX_THUMBNAILS: usize = 80;
/// Arrow-key scroll step.
const LINE_SCROLL: f32 = 40.0;

static NEXT_WINDOW_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[derive(Clone, Copy)]
struct Palette {
    window: u32,
    control_fill: u32,
    control_edge: u32,
    divider: u32,
    glyph: u32,
    subtitle: u32,
    placeholder: u32,
    document: u32,
    sidebar: u32,
    sidebar_edge: u32,
    selection: u32,
    thumb_label: u32,
    inspector: u32,
    inspector_separator: u32,
    card: u32,
    card_separator: u32,
    card_label: u32,
    card_value: u32,
    find: u32,
}

const DARK: Palette = Palette {
    window: dark::WINDOW,
    control_fill: dark::CONTROL_FILL,
    control_edge: dark::CONTROL_EDGE,
    divider: dark::DIVIDER,
    glyph: dark::GLYPH,
    subtitle: dark::SUBTITLE,
    placeholder: dark::PLACEHOLDER,
    document: dark::DOCUMENT,
    sidebar: dark::SIDEBAR,
    sidebar_edge: dark::SIDEBAR_EDGE,
    selection: dark::SELECTION,
    thumb_label: dark::THUMB_LABEL,
    inspector: dark::INSPECTOR,
    inspector_separator: dark::INSPECTOR_SEPARATOR,
    card: dark::CARD,
    card_separator: dark::CARD_SEPARATOR,
    card_label: dark::CARD_LABEL,
    card_value: dark::CARD_VALUE,
    find: dark::FIND_HIGHLIGHT,
};

const LIGHT: Palette = Palette {
    window: light::WINDOW,
    control_fill: light::CONTROL_FILL,
    control_edge: light::CONTROL_EDGE,
    divider: light::DIVIDER,
    glyph: light::GLYPH,
    subtitle: light::SUBTITLE,
    placeholder: light::PLACEHOLDER,
    document: light::DOCUMENT,
    sidebar: light::SIDEBAR,
    sidebar_edge: light::SIDEBAR_EDGE,
    selection: light::SELECTION,
    thumb_label: light::THUMB_LABEL,
    inspector: light::INSPECTOR,
    inspector_separator: light::INSPECTOR_SEPARATOR,
    card: light::CARD,
    card_separator: light::CARD_SEPARATOR,
    card_label: light::CARD_LABEL,
    card_value: light::CARD_VALUE,
    find: light::FIND_HIGHLIGHT,
};

fn palette() -> Palette {
    if mac::window().l > 0.5 {
        LIGHT
    } else {
        DARK
    }
}

/// A sidebar item: label, size in points and its thumbnail when rendered.
type SidebarEntry = (String, (f32, f32), Option<Arc<RenderImage>>);

enum SlotState {
    Loading,
    Failed(SharedString),
    Ready(Loaded),
}

struct PageBitmap {
    image: Arc<RenderImage>,
    pixel_width: u32,
    rotation: Rotation,
}

enum TextState {
    NotLoaded,
    Loading,
    Ready(Arc<Vec<TextPage>>),
    Failed(SharedString),
}

/// A position in a PDF's extracted text: a page, a word on it, and a
/// character insertion index within that word (0..=chars). Ordered in
/// reading order, so a selection is just its lower and upper `TextPos`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct TextPos {
    page: usize,
    word: usize,
    char: usize,
}

/// A selection's two ends in reading order, whichever the drag direction was.
fn ordered(anchor: TextPos, focus: TextPos) -> (TextPos, TextPos) {
    if anchor <= focus {
        (anchor, focus)
    } else {
        (focus, anchor)
    }
}

/// One open document.
struct Slot {
    id: u64,
    path: PathBuf,
    name: String,
    state: SlotState,
    rotation: Rotation,
    zoom: Zoom,
    /// The image as shown (rotated), and whether a rebuild is running.
    display: Option<(Rotation, Arc<RenderImage>)>,
    display_pending: bool,
    pages: HashMap<usize, PageBitmap>,
    thumbs: HashMap<usize, (Rotation, Arc<RenderImage>)>,
    pending: HashSet<(usize, bool)>,
    current_page: usize,
    text: TextState,
}

impl Slot {
    fn new(id: u64, path: PathBuf) -> Self {
        Self {
            id,
            name: document::display_name(&path),
            path,
            state: SlotState::Loading,
            rotation: Rotation::default(),
            zoom: Zoom::Fit,
            display: None,
            display_pending: false,
            pages: HashMap::new(),
            thumbs: HashMap::new(),
            pending: HashSet::new(),
            current_page: 0,
            text: TextState::NotLoaded,
        }
    }

    fn loaded(&self) -> Option<&Loaded> {
        match &self.state {
            SlotState::Ready(loaded) => Some(loaded),
            _ => None,
        }
    }

    fn kind(&self) -> Option<Kind> {
        self.loaded().map(|loaded| loaded.kind)
    }

    fn content_kind(&self) -> ContentKind {
        match self.kind() {
            Some(Kind::Pdf) => ContentKind::Pdf,
            _ => ContentKind::Image,
        }
    }

    /// Page (or image) sizes in points, after the page's /Rotate and the
    /// viewer rotation.
    fn page_sizes(&self) -> Vec<(f32, f32)> {
        match self.loaded().map(|loaded| &loaded.content) {
            Some(Content::Pdf(info)) => info
                .pages
                .iter()
                .map(|page| self.rotation.apply(page.displayed()))
                .collect(),
            Some(Content::Image(image)) => {
                vec![self
                    .rotation
                    .apply((image.size.0 as f32, image.size.1 as f32))]
            }
            None => Vec::new(),
        }
    }

    fn fit_scale(&self, viewport: (f32, f32)) -> f32 {
        let sizes = self.page_sizes();
        match self.content_kind() {
            ContentKind::Pdf => {
                let widest = sizes.iter().map(|size| size.0).fold(0.0, f32::max);
                zoom::fit_width(widest, viewport.0)
            }
            ContentKind::Image => sizes
                .first()
                .map(|size| zoom::fit_contain(*size, viewport))
                .unwrap_or(1.0),
        }
    }

    fn release_images(&mut self, garbage: &mut Vec<Arc<RenderImage>>) {
        if let Some((_, image)) = self.display.take() {
            garbage.push(image);
        }
        garbage.extend(self.pages.drain().map(|(_, bitmap)| bitmap.image));
    }
}

#[derive(Default)]
struct Search {
    /// The query the matches belong to.
    query: String,
    matches: Vec<Match>,
    current: Option<usize>,
    /// A query waiting for the text to be extracted.
    waiting: Option<String>,
}

pub(crate) struct PreviewView {
    pub(crate) focus: FocusHandle,
    slots: Vec<Slot>,
    selected: usize,
    next_id: u64,
    /// Images beside a single opened image, for Go ▸ Next / Previous Item.
    folder: Option<Vec<PathBuf>>,
    sidebar: bool,
    inspector: bool,
    scroll: ScrollHandle,
    sidebar_scroll: ScrollHandle,
    search_input: Entity<InputState>,
    search: Search,
    menu: Option<rmac_ui::ContextMenuState>,
    in_flight: usize,
    garbage: Vec<Arc<RenderImage>>,
    /// The document column measured on the last frame.
    viewport: (f32, f32),
    title: String,
    /// Drag-selected PDF text (Edit ▸ Copy, double-click a word, triple-click
    /// a line, ⌘A). `None` outside a PDF and when nothing is selected.
    text_selection: Option<(TextPos, TextPos)>,
    /// True while the left button is held dragging a text selection.
    text_selecting: bool,
    go_to_page_input: Entity<InputState>,
    go_to_page_open: bool,
    recent_documents: Vec<PathBuf>,
    /// This window's stable identity for the print portal transaction.
    window_generation: u64,
    /// Bumped whenever the selected document changes, so a print or export
    /// started before that never lands on the newer document (or vice
    /// versa). Shared with the async print/export task as `current`.
    document_generation: Arc<std::sync::atomic::AtomicU64>,
    print_busy: bool,
}

impl PreviewView {
    pub(crate) fn new(paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search")
                .clean_on_escape()
        });
        cx.subscribe(
            &search_input,
            |this, _, event: &InputEvent, cx| match event {
                InputEvent::PressEnter { shift, .. } => {
                    let query = this.search_input.read(cx).value().to_string();
                    if query.trim().is_empty() {
                        this.clear_search(cx);
                    } else if query == this.search.query && !this.search.matches.is_empty() {
                        this.step_match(if *shift { -1 } else { 1 }, cx);
                    } else {
                        this.run_search(query, cx);
                    }
                }
                InputEvent::Change if this.search_input.read(cx).value().trim().is_empty() => {
                    this.clear_search(cx);
                }
                _ => {}
            },
        )
        .detach();
        let go_to_page_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Page number")
                .clean_on_escape()
        });
        cx.subscribe(&go_to_page_input, |this, _, event: &InputEvent, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.submit_go_to_page(cx);
            }
        })
        .detach();
        let slots: Vec<Slot> = paths
            .into_iter()
            .enumerate()
            .map(|(index, path)| Slot::new(index as u64, path))
            .collect();
        let next_id = slots.len() as u64;
        let mut view = Self {
            focus: cx.focus_handle(),
            sidebar: slots.len() > 1,
            slots,
            selected: 0,
            next_id,
            folder: None,
            inspector: false,
            scroll: ScrollHandle::new(),
            sidebar_scroll: ScrollHandle::new(),
            search_input,
            search: Search::default(),
            menu: None,
            in_flight: 0,
            garbage: Vec::new(),
            viewport: (0.0, 0.0),
            title: String::new(),
            text_selection: None,
            text_selecting: false,
            go_to_page_input,
            go_to_page_open: false,
            recent_documents: load_recent_documents(),
            window_generation: NEXT_WINDOW_GENERATION
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            document_generation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            print_busy: false,
        };
        for index in 0..view.slots.len() {
            view.start_load(index, cx);
        }
        view
    }

    fn slot(&self) -> Option<&Slot> {
        self.slots.get(self.selected)
    }

    fn slot_mut(&mut self) -> Option<&mut Slot> {
        self.slots.get_mut(self.selected)
    }

    fn start_load(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(slot) = self.slots.get(index) else {
            return;
        };
        let (id, path) = (slot.id, slot.path.clone());
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { render::load(&path) })
                .await;
            let _ = this.update(cx, |view, cx| {
                let Some(slot) = view.slots.iter_mut().find(|slot| slot.id == id) else {
                    return;
                };
                let opened = result.is_ok();
                slot.state = match result {
                    Ok(loaded) => SlotState::Ready(loaded),
                    Err(error) => SlotState::Failed(error.into()),
                };
                // A PDF with several pages opens with its thumbnails, like
                // Preview; a single image or one-page PDF does not.
                let pages = slot.loaded().map(Loaded::page_count).unwrap_or(0);
                if view.slots.len() == 1 && pages > 1 {
                    view.sidebar = true;
                }
                if opened {
                    record_recent_document(slot.path.clone(), cx);
                }
                view.ensure_text(cx);
                cx.notify();
            });
        })
        .detach();
    }

    // ---- selection and navigation -------------------------------------

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if index == self.selected || index >= self.slots.len() {
            return;
        }
        if let Some(old) = self.slots.get_mut(self.selected) {
            // Only the shown document keeps full-size textures.
            old.release_images(&mut self.garbage);
            old.pending.clear();
        }
        self.selected = index;
        self.clear_search(cx);
        self.text_selection = None;
        self.text_selecting = false;
        self.scroll.set_offset(point(px(0.0), px(0.0)));
        self.ensure_text(cx);
        self.document_generation
            .fetch_add(1, std::sync::atomic::Ordering::Release);
        cx.notify();
    }

    fn step_item(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.slots.len() > 1 {
            if let Some(index) = document::step(self.selected, self.slots.len(), delta) {
                self.select(index, cx);
            }
            return;
        }
        let Some(slot) = self.slot() else { return };
        match slot.kind() {
            Some(Kind::Pdf) => {
                let pages = slot.loaded().map(Loaded::page_count).unwrap_or(0);
                if let Some(page) = document::step(slot.current_page, pages, delta) {
                    self.go_to_page(page, cx);
                }
            }
            _ => self.step_folder(delta, cx),
        }
    }

    /// Go ▸ Next / Previous Item for a single image: the images beside it,
    /// in Finder order.
    fn step_folder(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(current) = self.slot().map(|slot| slot.path.clone()) else {
            return;
        };
        let folder = self
            .folder
            .get_or_insert_with(|| render::folder_images(&current));
        let Some(index) = folder.iter().position(|path| *path == current) else {
            return;
        };
        let Some(target) = document::step(index, folder.len(), delta) else {
            return;
        };
        let path = folder[target].clone();
        let id = self.next_id;
        self.next_id += 1;
        if let Some(old) = self.slots.get_mut(self.selected) {
            old.release_images(&mut self.garbage);
            *old = Slot::new(id, path);
        }
        self.inspector_refresh();
        self.text_selection = None;
        self.text_selecting = false;
        self.document_generation
            .fetch_add(1, std::sync::atomic::Ordering::Release);
        self.start_load(self.selected, cx);
        cx.notify();
    }

    fn inspector_refresh(&mut self) {
        self.search = Search::default();
    }

    fn go_to_page(&mut self, page: usize, cx: &mut Context<Self>) {
        let Some(slot) = self.slot() else { return };
        let scale = slot.zoom.resolve(slot.fit_scale(self.viewport));
        let layout = layout::continuous(&slot.page_sizes(), scale, self.viewport.0);
        let top = layout::scroll_to_page(&layout.pages, page);
        let x = -f32::from(self.scroll.offset().x);
        self.scroll.set_offset(point(px(-x), px(-top)));
        if let Some(slot) = self.slot_mut() {
            slot.current_page = page;
        }
        cx.notify();
    }

    // ---- Go to Page (⌥⌘G) -------------------------------------------------

    fn open_go_to_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.slot().and_then(Slot::kind) != Some(Kind::Pdf) {
            return;
        }
        self.go_to_page_open = true;
        let current = self.slot().map(|slot| slot.current_page + 1).unwrap_or(1);
        self.go_to_page_input.update(cx, |state, cx| {
            state.set_value(current.to_string(), window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    fn close_go_to_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.go_to_page_open = false;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn submit_go_to_page(&mut self, cx: &mut Context<Self>) {
        let value = self.go_to_page_input.read(cx).value().to_string();
        let pages = self
            .slot()
            .and_then(Slot::loaded)
            .map(Loaded::page_count)
            .unwrap_or(0);
        if let Ok(page) = value.trim().parse::<usize>() {
            if page >= 1 && page <= pages {
                self.go_to_page(page - 1, cx);
            }
        }
        self.go_to_page_open = false;
        cx.notify();
    }

    fn scroll_by(&mut self, dx: f32, dy: f32, cx: &mut Context<Self>) {
        let offset = self.scroll.offset();
        let max = self.scroll.max_offset();
        let x = (-f32::from(offset.x) + dx).clamp(0.0, f32::from(max.x).max(0.0));
        let y = (-f32::from(offset.y) + dy).clamp(0.0, f32::from(max.y).max(0.0));
        self.scroll.set_offset(point(px(-x), px(-y)));
        cx.notify();
    }

    // ---- zoom and rotation ----------------------------------------------

    fn set_zoom(&mut self, zoom_to: impl Fn(&Slot, f32) -> Zoom, cx: &mut Context<Self>) {
        let viewport = self.viewport;
        let Some(slot) = self.slots.get_mut(self.selected) else {
            return;
        };
        if slot.loaded().is_none() {
            return;
        }
        let fit = slot.fit_scale(viewport);
        let old = slot.zoom.resolve(fit);
        slot.zoom = zoom_to(slot, old);
        let new = slot.zoom.resolve(fit);
        let offset = self.scroll.offset();
        let x = zoom::anchored_scroll(-f32::from(offset.x), viewport.0, old, new);
        let y = zoom::anchored_scroll(-f32::from(offset.y), viewport.1, old, new);
        self.scroll.set_offset(point(px(-x), px(-y)));
        cx.notify();
    }

    fn zoom_in(&mut self, cx: &mut Context<Self>) {
        self.set_zoom(
            |slot, scale| Zoom::Scale(zoom::zoom_in(slot.content_kind(), scale)),
            cx,
        );
    }

    fn zoom_out(&mut self, cx: &mut Context<Self>) {
        self.set_zoom(
            |slot, scale| Zoom::Scale(zoom::zoom_out(slot.content_kind(), scale)),
            cx,
        );
    }

    fn actual_size(&mut self, cx: &mut Context<Self>) {
        self.set_zoom(|_, _| Zoom::Scale(1.0), cx);
    }

    fn zoom_to_fit(&mut self, cx: &mut Context<Self>) {
        self.set_zoom(|_, _| Zoom::Fit, cx);
    }

    fn rotate(&mut self, right: bool, cx: &mut Context<Self>) {
        let Some(slot) = self.slots.get_mut(self.selected) else {
            return;
        };
        if slot.loaded().is_none() {
            return;
        }
        slot.rotation = if right {
            slot.rotation.right()
        } else {
            slot.rotation.left()
        };
        // Page bitmaps are re-rendered at the new orientation; until then
        // the old ones are dropped rather than drawn sideways.
        self.garbage
            .extend(slot.pages.drain().map(|(_, bitmap)| bitmap.image));
        self.garbage
            .extend(slot.thumbs.drain().map(|(_, (_, image))| image));
        slot.pending.clear();
        cx.notify();
    }

    // ---- sidebar, inspector ---------------------------------------------

    fn set_sidebar(&mut self, shown: bool, cx: &mut Context<Self>) {
        self.sidebar = shown;
        cx.notify();
    }

    fn toggle_inspector(&mut self, cx: &mut Context<Self>) {
        self.inspector = !self.inspector;
        cx.notify();
    }

    fn open_sidebar_menu(
        &mut self,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu = Some(rmac_ui::ContextMenuState::open(
            event.position(),
            &self.focus,
            window,
            cx,
        ));
        cx.notify();
    }

    // ---- copy -------------------------------------------------------------

    /// Edit ▸ Copy: the selected PDF text, or the whole image.
    fn copy(&mut self, cx: &mut Context<Self>) {
        let Some(slot) = self.slot() else { return };
        if slot.kind() == Some(Kind::Pdf) {
            if let (Some((anchor, focus)), TextState::Ready(pages)) =
                (self.text_selection, &slot.text)
            {
                let (from, to) = ordered(anchor, focus);
                let text = poppler::selected_text(
                    pages,
                    (from.page, from.word, from.char),
                    (to.page, to.word, to.char),
                );
                if !text.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }
            return;
        }
        let Some(Content::Image(image)) = slot.loaded().map(|loaded| &loaded.content) else {
            return;
        };
        let pixels = image.pixels.clone();
        let rotation = slot.rotation;
        cx.spawn(async move |_, cx| {
            let encoded = cx
                .background_executor()
                .spawn(async move { render::encode_png(&render::rotate(&pixels, rotation)) })
                .await;
            match encoded {
                Ok(bytes) => cx.update(|cx| {
                    cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(
                        ImageFormat::Png,
                        bytes,
                    )))
                }),
                Err(error) => eprintln!("rmac-preview: copy failed: {error}"),
            }
        })
        .detach();
    }

    // ---- print --------------------------------------------------------------

    /// File ▸ Print… (⌘P) for a PDF: the document's own bytes go straight to
    /// the print portal — nothing is re-rendered, so what prints matches
    /// what's on screen exactly. Printing an image isn't implemented yet
    /// (see `crate::pdfwriter` for the building block a later pass would use
    /// to wrap one in a page first, the same way `Export as PDF…` would).
    #[cfg(target_os = "linux")]
    fn print_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.print_busy {
            return;
        }
        let Some(slot) = self.slot() else { return };
        if slot.kind() != Some(Kind::Pdf) {
            eprintln!("rmac-preview: printing an image isn't supported yet");
            return;
        }
        let path = slot.path.clone();
        let title = slot.name.clone();
        let raw_window =
            raw_window_handle::HasWindowHandle::window_handle(window).map(|handle| handle.as_raw());
        let raw_display = raw_window_handle::HasDisplayHandle::display_handle(window)
            .map(|handle| handle.as_raw());
        let (raw_window, raw_display) = match (raw_window, raw_display) {
            (Ok(raw_window), Ok(raw_display)) => (raw_window, raw_display),
            _ => {
                eprintln!("rmac-preview: printing requires the current exported window");
                return;
            }
        };
        let document_generation = self
            .document_generation
            .load(std::sync::atomic::Ordering::Acquire);
        let request = rmac_print_linux::PreparedPrintDocument {
            window: raw_window,
            display: raw_display,
            window_generation: self.window_generation,
            document_generation,
            current_document_generation: self.document_generation.clone(),
            title,
            pdf: Vec::new(),
        };
        self.print_busy = true;
        cx.spawn_in(window, async move |this, cx| {
            let pdf = cx
                .background_executor()
                .spawn(async move { std::fs::read(&path) })
                .await;
            let outcome = match pdf {
                Ok(pdf) => rmac_print_linux::print_prepared_document(
                    rmac_print_linux::PreparedPrintDocument { pdf, ..request },
                )
                .await
                .map_err(|error| error.to_string()),
                Err(error) => Err(format!(
                    "the document could not be read to print it: {error}"
                )),
            };
            let _ = this.update_in(cx, |this, _, cx| {
                this.print_busy = false;
                if let Err(error) = outcome {
                    eprintln!("rmac-preview: could not print: {error}");
                }
                cx.notify();
            });
        })
        .detach();
    }

    #[cfg(not(target_os = "linux"))]
    fn print_document(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        eprintln!("rmac-preview: printing is implemented for the supported Linux session");
    }

    // ---- PDF text selection -------------------------------------------------

    /// Edit ▸ Select All for a PDF: the whole document's extracted text.
    fn select_all(&mut self, cx: &mut Context<Self>) {
        let Some(slot) = self.slot() else { return };
        if slot.kind() != Some(Kind::Pdf) {
            return;
        }
        let TextState::Ready(pages) = &slot.text else {
            return;
        };
        let Some(last_page) = pages.len().checked_sub(1) else {
            return;
        };
        let last_word = pages[last_page].words.len().saturating_sub(1);
        let last_chars = pages[last_page]
            .words
            .get(last_word)
            .map(|word| word.text.chars().count())
            .unwrap_or(0);
        self.text_selection = Some((
            TextPos {
                page: 0,
                word: 0,
                char: 0,
            },
            TextPos {
                page: last_page,
                word: last_word,
                char: last_chars,
            },
        ));
        cx.notify();
    }

    /// Maps a window-space point onto the current PDF's page geometry: the
    /// page index and that page's unit coordinates (0‥1), already displayed
    /// (i.e. after the viewer's rotation).
    fn screen_to_page_point(
        &self,
        position: gpui::Point<gpui::Pixels>,
    ) -> Option<(usize, (f32, f32))> {
        let slot = self.slot()?;
        if slot.kind() != Some(Kind::Pdf) {
            return None;
        }
        let left = metrics::document_left(self.sidebar);
        let scale = slot.zoom.resolve(slot.fit_scale(self.viewport));
        let sizes = slot.page_sizes();
        let layout = layout::continuous(&sizes, scale, self.viewport.0);
        let scroll_x = -f32::from(self.scroll.offset().x);
        let scroll_y = -f32::from(self.scroll.offset().y);
        let doc_x = f32::from(position.x) - left + scroll_x;
        let doc_y = f32::from(position.y) - metrics::TOOLBAR_HEIGHT + scroll_y;
        layout::point_to_page(&layout.pages, (doc_x, doc_y))
    }

    /// The word/character position under a window-space point, in the raw
    /// (unrotated) text-extraction space `poppler::hit_test` works in.
    fn hit_test_text(&self, position: gpui::Point<gpui::Pixels>) -> Option<TextPos> {
        let (page, unit) = self.screen_to_page_point(position)?;
        let slot = self.slot()?;
        let TextState::Ready(pages) = &slot.text else {
            return None;
        };
        let raw = slot.rotation.inverse().apply_unit_rect(layout::UnitRect {
            x0: unit.0,
            y0: unit.1,
            x1: unit.0,
            y1: unit.1,
        });
        let (word, char) = poppler::hit_test(pages, page, (raw.x0, raw.y0))?;
        Some(TextPos { page, word, char })
    }

    /// ⌘-click a plain `http(s)://` URL in the extracted text (see the
    /// module notes for why only literal URL text is followed, not the
    /// document's own `/Link` annotations).
    fn link_at(&self, position: gpui::Point<gpui::Pixels>) -> Option<String> {
        let (page, unit) = self.screen_to_page_point(position)?;
        let slot = self.slot()?;
        let TextState::Ready(pages) = &slot.text else {
            return None;
        };
        let raw = slot.rotation.inverse().apply_unit_rect(layout::UnitRect {
            x0: unit.0,
            y0: unit.1,
            x1: unit.0,
            y1: unit.1,
        });
        let text_page = pages.get(page)?;
        poppler::find_links(text_page)
            .into_iter()
            .find(|(_, rect, _)| {
                raw.x0 >= rect.x0 && raw.x0 <= rect.x1 && raw.y0 >= rect.y0 && raw.y0 <= rect.y1
            })
            .map(|(_, _, uri)| uri)
    }

    fn open_link(&mut self, uri: String, cx: &mut Context<Self>) {
        cx.spawn(async move |_, _cx| {
            if let Err(error) = rmac_portal::open_uri(&uri).await {
                eprintln!("rmac-preview: could not open the link: {error}");
            }
        })
        .detach();
    }

    fn text_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }
        if event.modifiers.platform {
            if let Some(uri) = self.link_at(event.position) {
                self.open_link(uri, cx);
                return;
            }
        }
        let Some(pos) = self.hit_test_text(event.position) else {
            self.text_selection = None;
            cx.notify();
            return;
        };
        window.focus(&self.focus, cx);
        match event.click_count {
            2 => {
                let Some(slot) = self.slot() else { return };
                let TextState::Ready(pages) = &slot.text else {
                    return;
                };
                let chars = pages
                    .get(pos.page)
                    .and_then(|page| page.words.get(pos.word))
                    .map(|word| word.text.chars().count())
                    .unwrap_or(0);
                self.text_selection = Some((
                    TextPos {
                        page: pos.page,
                        word: pos.word,
                        char: 0,
                    },
                    TextPos {
                        page: pos.page,
                        word: pos.word,
                        char: chars,
                    },
                ));
                self.text_selecting = false;
            }
            count if count >= 3 => {
                let Some(slot) = self.slot() else { return };
                let TextState::Ready(pages) = &slot.text else {
                    return;
                };
                let Some(text_page) = pages.get(pos.page) else {
                    return;
                };
                let (first, last) = poppler::line_bounds(text_page, pos.word);
                let last_chars = text_page
                    .words
                    .get(last)
                    .map(|word| word.text.chars().count())
                    .unwrap_or(0);
                self.text_selection = Some((
                    TextPos {
                        page: pos.page,
                        word: first,
                        char: 0,
                    },
                    TextPos {
                        page: pos.page,
                        word: last,
                        char: last_chars,
                    },
                ));
                self.text_selecting = false;
            }
            _ => {
                self.text_selection = Some((pos, pos));
                self.text_selecting = true;
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn text_mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if !self.text_selecting {
            return;
        }
        let Some(pos) = self.hit_test_text(event.position) else {
            return;
        };
        if let Some((anchor, _)) = self.text_selection {
            self.text_selection = Some((anchor, pos));
            cx.notify();
        }
    }

    fn text_mouse_up(&mut self, cx: &mut Context<Self>) {
        if self.text_selecting {
            self.text_selecting = false;
            cx.notify();
        }
    }

    /// Selection highlight rectangles per page, already rotated for display.
    fn selection_highlights(&self, slot: &Slot) -> HashMap<usize, Vec<(layout::UnitRect, u32)>> {
        let mut map: HashMap<usize, Vec<(layout::UnitRect, u32)>> = HashMap::new();
        let Some((anchor, focus)) = self.text_selection else {
            return map;
        };
        let TextState::Ready(pages) = &slot.text else {
            return map;
        };
        let (from, to) = ordered(anchor, focus);
        if from == to {
            return map;
        }
        let color = (palette().selection << 8) | 0x66;
        for page_index in from.page..=to.page.min(pages.len().saturating_sub(1)) {
            let Some(text_page) = pages.get(page_index) else {
                continue;
            };
            let from_bound = (page_index == from.page).then_some((from.word, from.char));
            let to_bound = (page_index == to.page).then_some((to.word, to.char));
            let rects = poppler::selection_rects(text_page, from_bound, to_bound);
            if rects.is_empty() {
                continue;
            }
            map.entry(page_index).or_default().extend(
                rects
                    .into_iter()
                    .map(|rect| (slot.rotation.apply_unit_rect(rect), color)),
            );
        }
        map
    }

    // ---- search -----------------------------------------------------------

    fn clear_search(&mut self, cx: &mut Context<Self>) {
        self.search = Search::default();
        cx.notify();
    }

    fn run_search(&mut self, query: String, cx: &mut Context<Self>) {
        let Some(slot) = self.slots.get_mut(self.selected) else {
            return;
        };
        if slot.kind() != Some(Kind::Pdf) {
            return;
        }
        match &slot.text {
            TextState::Ready(pages) => {
                self.search = Search {
                    matches: poppler::search(pages, &query),
                    query,
                    current: None,
                    waiting: None,
                };
                self.step_match(1, cx);
            }
            TextState::Loading => self.search.waiting = Some(query),
            TextState::Failed(error) => {
                eprintln!("rmac-preview: PDF text is unavailable: {error}");
            }
            TextState::NotLoaded => {
                self.search.waiting = Some(query);
                self.ensure_text(cx);
            }
        }
        cx.notify();
    }

    /// Load the current slot's word/position text (once) so PDF search, text
    /// selection, and plain-URL link detection all have it ready.
    fn ensure_text(&mut self, cx: &mut Context<Self>) {
        let Some(slot) = self.slots.get_mut(self.selected) else {
            return;
        };
        if slot.kind() != Some(Kind::Pdf) || !matches!(slot.text, TextState::NotLoaded) {
            return;
        }
        slot.text = TextState::Loading;
        let (id, path) = (slot.id, slot.path.clone());
        cx.spawn(async move |this, cx| {
            let text = cx
                .background_executor()
                .spawn(async move { render::extract_text(&path) })
                .await;
            let _ = this.update(cx, |view, cx| {
                let Some(slot) = view.slots.iter_mut().find(|slot| slot.id == id) else {
                    return;
                };
                slot.text = match text {
                    Ok(pages) => TextState::Ready(Arc::new(pages)),
                    Err(error) => TextState::Failed(error.into()),
                };
                if let Some(query) = view.search.waiting.take() {
                    view.run_search(query, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn step_match(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.search.matches.len();
        if count == 0 {
            self.search.current = None;
            cx.notify();
            return;
        }
        let next = match self.search.current {
            None => 0,
            Some(current) => (current as isize + delta).rem_euclid(count as isize) as usize,
        };
        self.search.current = Some(next);
        self.reveal_match(next, cx);
    }

    fn reveal_match(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(found) = self.search.matches.get(index) else {
            return;
        };
        let Some(slot) = self.slot() else { return };
        let scale = slot.zoom.resolve(slot.fit_scale(self.viewport));
        let layout = layout::continuous(&slot.page_sizes(), scale, self.viewport.0);
        let Some(page) = layout.pages.get(found.page) else {
            return;
        };
        let Some(rect) = found.rects.first() else {
            return;
        };
        let rect = slot.rotation.apply_unit_rect(*rect);
        let x = (page.x + rect.x0 * page.width - self.viewport.0 / 2.0).max(0.0);
        let y = (page.y + rect.y0 * page.height - self.viewport.1 / 3.0).max(0.0);
        let current_x = -f32::from(self.scroll.offset().x);
        let x = if page.x + rect.x1 * page.width <= current_x + self.viewport.0
            && page.x + rect.x0 * page.width >= current_x
        {
            current_x
        } else {
            x
        };
        self.scroll.set_offset(point(px(-x), px(-y)));
        cx.notify();
    }

    // ---- lazy rendering ---------------------------------------------------

    /// Queue whatever the current frame needs: the rotated image, visible
    /// page bitmaps at the current scale, then visible sidebar thumbnails.
    fn schedule(&mut self, window: &Window, cx: &mut Context<Self>) {
        let scale_factor = window.scale_factor();
        let viewport = self.viewport;
        let sidebar = self.sidebar;
        let sidebar_top = -f32::from(self.sidebar_scroll.offset().y);
        let sidebar_height = f32::from(window.viewport_size().height) - metrics::TOOLBAR_HEIGHT;
        let scroll_top = -f32::from(self.scroll.offset().y);
        let single = self.slots.len() == 1;
        let Some(slot) = self.slots.get_mut(self.selected) else {
            return;
        };
        let Some(content) = slot.loaded().map(|loaded| loaded.content.clone()) else {
            return;
        };
        match &content {
            Content::Image(image) => {
                if !slot.display_pending
                    && slot.display.as_ref().map(|(rotation, _)| *rotation) != Some(slot.rotation)
                {
                    slot.display_pending = true;
                    let (id, rotation, pixels) = (slot.id, slot.rotation, image.pixels.clone());
                    cx.spawn(async move |this, cx| {
                        let shown = cx
                            .background_executor()
                            .spawn(async move {
                                render::to_render_image(render::rotate(&pixels, rotation))
                            })
                            .await;
                        let _ = this.update(cx, |view, cx| {
                            let Some(slot) = view.slots.iter_mut().find(|slot| slot.id == id)
                            else {
                                view.garbage.push(shown);
                                return;
                            };
                            slot.display_pending = false;
                            if let Some((_, old)) = slot.display.replace((rotation, shown)) {
                                view.garbage.push(old);
                            }
                            cx.notify();
                        });
                    })
                    .detach();
                }
            }
            Content::Pdf(info) => {
                let info = info.clone();
                let sizes = slot.page_sizes();
                let scale = slot.zoom.resolve(slot.fit_scale(viewport));
                let layout = layout::continuous(&sizes, scale, viewport.0);
                let visible = layout::visible_pages(&layout.pages, scroll_top, viewport.1, 1);
                let mut wanted: Vec<(usize, bool, f32)> = Vec::new();
                for page in visible.clone() {
                    let rect = layout.pages[page];
                    let needed = (rect.width * scale_factor).round() as u32;
                    let stale = match slot.pages.get(&page) {
                        None => true,
                        Some(bitmap) => {
                            bitmap.rotation != slot.rotation
                                || needed > bitmap.pixel_width + bitmap.pixel_width / 10
                                || needed < bitmap.pixel_width / 2
                        }
                    };
                    if stale {
                        wanted.push((page, false, scale * scale_factor));
                    }
                }
                if sidebar && single {
                    let (items, _) = layout::thumbnail_items(&sizes);
                    for (page, item) in items.iter().enumerate() {
                        if item.top + item.height < sidebar_top
                            || item.top > sidebar_top + sidebar_height
                        {
                            continue;
                        }
                        if !slot.thumbs.contains_key(&page) {
                            let pixel_scale = item.thumb.0 / sizes[page].0 * scale_factor;
                            wanted.push((page, true, pixel_scale));
                        }
                    }
                }
                // Keep caches bounded: drop the bitmaps farthest from view.
                if slot.pages.len() > MAX_PAGE_BITMAPS {
                    let centre = (visible.start + visible.end) / 2;
                    let mut keys: Vec<usize> = slot.pages.keys().copied().collect();
                    keys.sort_by_key(|page| std::cmp::Reverse(page.abs_diff(centre)));
                    for page in keys.into_iter().take(slot.pages.len() - MAX_PAGE_BITMAPS) {
                        if let Some(bitmap) = slot.pages.remove(&page) {
                            self.garbage.push(bitmap.image);
                        }
                    }
                }
                if slot.thumbs.len() > MAX_THUMBNAILS {
                    let centre = slot.current_page;
                    let mut keys: Vec<usize> = slot.thumbs.keys().copied().collect();
                    keys.sort_by_key(|page| std::cmp::Reverse(page.abs_diff(centre)));
                    for page in keys.into_iter().take(slot.thumbs.len() - MAX_THUMBNAILS) {
                        if let Some((_, image)) = slot.thumbs.remove(&page) {
                            self.garbage.push(image);
                        }
                    }
                }
                for (page, thumb, pixel_scale) in wanted {
                    if self.in_flight >= MAX_RENDERS {
                        break;
                    }
                    if !slot.pending.insert((page, thumb)) {
                        continue;
                    }
                    self.in_flight += 1;
                    let (id, path, rotation) = (slot.id, slot.path.clone(), slot.rotation);
                    let page_size = info.pages[page].displayed();
                    cx.spawn(async move |this, cx| {
                        let rendered = cx
                            .background_executor()
                            .spawn(async move {
                                render::render_page(&path, page, page_size, pixel_scale, rotation)
                            })
                            .await;
                        let _ = this.update(cx, |view, cx| {
                            view.in_flight = view.in_flight.saturating_sub(1);
                            let Some(slot) = view.slots.iter_mut().find(|slot| slot.id == id)
                            else {
                                return;
                            };
                            if !slot.pending.remove(&(page, thumb)) || slot.rotation != rotation {
                                // Cancelled by a rotation or a document switch.
                                cx.notify();
                                return;
                            }
                            match rendered {
                                Ok(pixels) => {
                                    let pixel_width = pixels.width();
                                    let image = render::to_render_image(pixels);
                                    if thumb {
                                        if let Some((_, old)) =
                                            slot.thumbs.insert(page, (rotation, image))
                                        {
                                            view.garbage.push(old);
                                        }
                                    } else if let Some(old) = slot.pages.insert(
                                        page,
                                        PageBitmap {
                                            image,
                                            pixel_width,
                                            rotation,
                                        },
                                    ) {
                                        view.garbage.push(old.image);
                                    }
                                }
                                Err(error) => {
                                    eprintln!("rmac-preview: page {} failed: {error}", page + 1);
                                    if !thumb && slot.pages.is_empty() && page == 0 {
                                        slot.state = SlotState::Failed(error.into());
                                    }
                                }
                            }
                            cx.notify();
                        });
                    })
                    .detach();
                }
            }
        }
    }

    // ---- key handling -------------------------------------------------------

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.search_input.focus_handle(cx).is_focused(window) {
            return;
        }
        if self.go_to_page_input.focus_handle(cx).is_focused(window) {
            if event.keystroke.key == "escape" {
                self.close_go_to_page(window, cx);
                cx.stop_propagation();
            }
            return;
        }
        let modifiers = event.keystroke.modifiers;
        if modifiers.platform || modifiers.control || modifiers.alt {
            return;
        }
        let is_pdf = self.slot().and_then(Slot::kind) == Some(Kind::Pdf);
        let page = (self.viewport.1 - LINE_SCROLL).max(LINE_SCROLL);
        match event.keystroke.key.as_str() {
            "left" | "up" if !is_pdf => self.step_item(-1, cx),
            "right" | "down" if !is_pdf => self.step_item(1, cx),
            "up" => self.scroll_by(0.0, -LINE_SCROLL, cx),
            "down" => self.scroll_by(0.0, LINE_SCROLL, cx),
            "left" if f32::from(self.scroll.max_offset().x) > 0.0 => {
                self.scroll_by(-LINE_SCROLL, 0.0, cx)
            }
            "right" if f32::from(self.scroll.max_offset().x) > 0.0 => {
                self.scroll_by(LINE_SCROLL, 0.0, cx)
            }
            "left" => self.step_item(-1, cx),
            "right" => self.step_item(1, cx),
            "pageup" => self.scroll_by(0.0, -page, cx),
            "pagedown" => self.scroll_by(0.0, page, cx),
            "space" if modifiers.shift => self.scroll_by(0.0, -page, cx),
            "space" => self.scroll_by(0.0, page, cx),
            "home" => self.scroll_by(0.0, f32::MIN / 2.0, cx),
            "end" => self.scroll_by(0.0, f32::MAX / 2.0, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    // ---- rendering --------------------------------------------------------

    fn title_and_subtitle(&self) -> (String, Option<String>) {
        let Some(slot) = self.slot() else {
            return ("Preview".into(), None);
        };
        let total: usize = self
            .slots
            .iter()
            .map(|slot| slot.loaded().map(Loaded::page_count).unwrap_or(1))
            .sum();
        let pdf = match slot.loaded().map(|loaded| &loaded.content) {
            Some(Content::Pdf(info)) => Some((slot.current_page, info.pages.len())),
            _ => None,
        };
        (
            slot.name.clone(),
            document::subtitle(self.slots.len(), total, pdf),
        )
    }

    fn render_toolbar(
        &self,
        palette: Palette,
        width: f32,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (light_x, light_y) = metrics::TRAFFIC_LIGHT_CENTER;
        let hit_width = mac::traffic_light_hit_width();
        let hit_height = mac::traffic_light_hit_height();
        let is_pdf = self.slot().and_then(Slot::kind) == Some(Kind::Pdf);
        let ready = self.slot().is_some_and(|slot| slot.loaded().is_some());
        let group = metrics::right_group(width, is_pdf);
        let (title, subtitle) = self.title_and_subtitle();
        let title_left = if self.sidebar {
            metrics::TITLE_LEFT_WITH_SIDEBAR
        } else {
            metrics::TITLE_LEFT
        };
        let glyph = |path: &'static str, size: f32, color: u32| {
            svg().path(path).size(px(size)).text_color(rgb(color))
        };
        let capsule = |id: &'static str, left: f32, width: f32| {
            div()
                .id(id)
                .absolute()
                .left(px(left))
                .top(px(metrics::CONTROL_TOP))
                .w(px(width))
                .h(px(metrics::CONTROL_HEIGHT))
                .rounded(px(metrics::CONTROL_HEIGHT / 2.0))
                .bg(rgb(palette.control_fill))
                .border_1()
                .border_color(rgb(palette.control_edge))
        };
        let button = |id: &'static str, icon: &'static str, enabled: bool| {
            div()
                .id(id)
                .w(px(metrics::ZOOM_BUTTON_WIDTH))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .when(!enabled, |button| button.opacity(0.35))
                .when(enabled, |button| button.active(|style| style.opacity(0.6)))
                .child(glyph(icon, metrics::GLYPH_SIZE, palette.glyph))
        };
        let divider = || {
            div()
                .w(px(1.0))
                .h(px(metrics::DIVIDER_HEIGHT))
                .bg(rgb(palette.divider))
        };
        let scale_state = self.slot().map(|slot| (slot.zoom, slot.content_kind()));
        let at_actual =
            matches!(scale_state, Some((Zoom::Scale(scale), _)) if (scale - 1.0).abs() < 1e-4);

        // Sidebar toggle: the glyph toggles, the chevron opens the view menu.
        let toggle_left = metrics::SIDEBAR_TOGGLE_LEFT
            + if self.sidebar {
                metrics::SIDEBAR_TOGGLE_SHOWN_SHIFT
            } else {
                0.0
            };
        let sidebar_toggle = div()
            .absolute()
            .left(px(toggle_left))
            .top(px(metrics::CONTROL_TOP - 0.5))
            .w(px(metrics::SIDEBAR_TOGGLE_WIDTH))
            .h(px(metrics::CONTROL_HEIGHT))
            .rounded(px(metrics::CONTROL_HEIGHT / 2.0))
            .when(!self.sidebar, |toggle| {
                toggle
                    .bg(rgb(palette.control_fill))
                    .border_1()
                    .border_color(rgb(palette.control_edge))
            })
            .flex()
            .items_center()
            .child(
                div()
                    .id("preview-sidebar-toggle")
                    .pl(px(7.5))
                    .h_full()
                    .flex()
                    .items_center()
                    .active(|style| style.opacity(0.6))
                    .child(glyph(
                        "icons/preview/sidebar.svg",
                        metrics::SIDEBAR_GLYPH_SIZE,
                        palette.glyph,
                    ))
                    .on_click(cx.listener(|this, _, _, cx| {
                        let shown = !this.sidebar;
                        this.set_sidebar(shown, cx);
                    })),
            )
            .child(
                div()
                    .id("preview-sidebar-menu")
                    .pl(px(3.0))
                    .pr(px(8.0))
                    .h_full()
                    .flex()
                    .items_center()
                    .active(|style| style.opacity(0.6))
                    .child(glyph(
                        "icons/preview/chevron-down.svg",
                        metrics::CHEVRON_SIZE,
                        palette.glyph,
                    ))
                    .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                        this.open_sidebar_menu(event, window, cx);
                    })),
            );

        let title_block = div()
            .absolute()
            .left(px(title_left))
            .top_0()
            .w(px((group.zoom - title_left - 12.0).max(0.0)))
            .h(px(metrics::TOOLBAR_HEIGHT))
            .text_color(rgb(palette.glyph))
            .map(|block| match subtitle {
                None => block.flex().items_center().child(
                    div()
                        .w_full()
                        .truncate()
                        .text_size(px(metrics::TITLE_SINGLE_SIZE))
                        .font_weight(FontWeight::BOLD)
                        .child(SharedString::from(title)),
                ),
                Some(subtitle) => block
                    .child(
                        div()
                            .absolute()
                            .top(px(metrics::TITLE_TOP))
                            .w_full()
                            .h(px(metrics::TITLE_LINE))
                            .line_height(px(metrics::TITLE_LINE))
                            .truncate()
                            .text_size(px(metrics::TITLE_SIZE))
                            .font_weight(FontWeight::BOLD)
                            .child(SharedString::from(title)),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(metrics::SUBTITLE_TOP))
                            .w_full()
                            .h(px(metrics::SUBTITLE_LINE))
                            .line_height(px(metrics::SUBTITLE_LINE))
                            .truncate()
                            .text_size(px(metrics::SUBTITLE_SIZE))
                            .text_color(rgb(palette.subtitle))
                            .child(SharedString::from(subtitle)),
                    ),
            });

        let zoom_group = capsule("preview-zoom", group.zoom, metrics::ZOOM_GROUP_WIDTH)
            .flex()
            .items_center()
            .child(
                button("preview-zoom-out", "icons/preview/zoom-out.svg", ready)
                    .on_click(cx.listener(|this, _, _, cx| this.zoom_out(cx))),
            )
            .child(divider())
            .child(
                button(
                    "preview-actual-size",
                    "icons/preview/zoom-actual.svg",
                    ready && !at_actual,
                )
                .on_click(cx.listener(|this, _, _, cx| this.actual_size(cx))),
            )
            .child(divider())
            .child(
                button("preview-zoom-in", "icons/preview/zoom-in.svg", ready)
                    .on_click(cx.listener(|this, _, _, cx| this.zoom_in(cx))),
            );

        // Preview's toolbar rotate button turns left; ⌥-click turns right.
        let rotate = capsule("preview-rotate", group.rotate, metrics::CONTROL_HEIGHT)
            .flex()
            .items_center()
            .justify_center()
            .when(!ready, |button| button.opacity(0.35))
            .active(|style| style.opacity(0.6))
            .child(glyph(
                "icons/preview/rotate-left.svg",
                metrics::GLYPH_SIZE,
                palette.glyph,
            ))
            .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                this.rotate(event.modifiers().alt, cx);
            }));

        let info = capsule("preview-info", group.info, metrics::CONTROL_HEIGHT)
            .flex()
            .items_center()
            .justify_center()
            .when(self.inspector, |button| {
                button
                    .bg(rgb(palette.selection))
                    .border_color(rgb(palette.selection))
            })
            .when(!ready, |button| button.opacity(0.35))
            .active(|style| style.opacity(0.6))
            .child(glyph(
                "icons/preview/info.svg",
                metrics::GLYPH_SIZE,
                if self.inspector {
                    0xFFFFFF
                } else {
                    palette.glyph
                },
            ))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_inspector(cx)));

        let search = is_pdf.then(|| {
            let text_failed = self
                .slot()
                .is_some_and(|slot| matches!(slot.text, TextState::Failed(_)));
            let count = if text_failed {
                Some("Unavailable".to_owned())
            } else if self.search.matches.is_empty() {
                (!self.search.query.is_empty()).then(|| "Not found".to_owned())
            } else {
                self.search
                    .current
                    .map(|current| format!("{} of {}", current + 1, self.search.matches.len()))
            };
            capsule("preview-search", group.search, metrics::SEARCH_WIDTH)
                .child(
                    div()
                        .absolute()
                        .left(px(metrics::SEARCH_GLYPH_LEFT - 1.0))
                        .top(px((metrics::CONTROL_HEIGHT - 2.0 - 16.0) / 2.0))
                        .child(glyph("icons/preview/search.svg", 16.0, palette.placeholder)),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(metrics::SEARCH_TEXT_LEFT - 1.0 - 8.0))
                        .right(px(if count.is_some() { 64.0 } else { 6.0 }))
                        .top_0()
                        .bottom_0()
                        .flex()
                        .items_center()
                        .text_size(px(13.0))
                        .child(
                            rmac_ui::SearchField::new(&self.search_input)
                                .appearance(false)
                                .small(),
                        ),
                )
                .when_some(count, |field, count| {
                    field.child(
                        div()
                            .absolute()
                            .right(px(10.0))
                            .top_0()
                            .bottom_0()
                            .flex()
                            .items_center()
                            .text_size(px(11.0))
                            .text_color(rgb(palette.placeholder))
                            .child(SharedString::from(count)),
                    )
                })
        });

        div()
            .absolute()
            .top_0()
            .left_0()
            .w_full()
            .h(px(metrics::TOOLBAR_HEIGHT))
            .child(
                div()
                    .id("preview-drag")
                    .absolute()
                    .size_full()
                    .window_control_area(WindowControlArea::Drag),
            )
            .child(
                div()
                    .absolute()
                    .left(px(light_x - hit_width / 2.0))
                    .top(px(light_y - hit_height / 2.0))
                    .child(rmac_ui::traffic_lights_active(window.is_window_active())),
            )
            .child(sidebar_toggle)
            .child(title_block)
            .child(zoom_group)
            .child(rotate)
            .child(info)
            .children(search)
    }

    fn render_sidebar(
        &self,
        palette: Palette,
        height: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // Items: documents when several are open, otherwise the pages.
        let several = self.slots.len() > 1;
        let (entries, selected): (Vec<SidebarEntry>, usize) = if several {
            (
                self.slots
                    .iter()
                    .map(|slot| {
                        let size = slot.page_sizes().first().copied().unwrap_or((120.0, 120.0));
                        (slot.name.clone(), size, image_thumbnail(slot))
                    })
                    .collect(),
                self.selected,
            )
        } else if let Some(slot) = self.slot() {
            match slot.kind() {
                Some(Kind::Pdf) => (
                    slot.page_sizes()
                        .into_iter()
                        .enumerate()
                        .map(|(page, size)| {
                            (
                                (page + 1).to_string(),
                                size,
                                slot.thumbs.get(&page).map(|(_, image)| image.clone()),
                            )
                        })
                        .collect(),
                    slot.current_page,
                ),
                _ => (
                    vec![(
                        slot.name.clone(),
                        slot.page_sizes().first().copied().unwrap_or((120.0, 120.0)),
                        image_thumbnail(slot),
                    )],
                    0,
                ),
            }
        } else {
            (Vec::new(), 0)
        };
        let sizes: Vec<(f32, f32)> = entries.iter().map(|(_, size, _)| *size).collect();
        let (items, total) = layout::thumbnail_items(&sizes);
        let children = entries
            .into_iter()
            .zip(items)
            .enumerate()
            .map(|(index, ((label, _, image), item))| {
                self.render_thumbnail(
                    palette,
                    index,
                    index == selected,
                    label,
                    image,
                    item,
                    several,
                    cx,
                )
            })
            .collect::<Vec<_>>();
        div()
            .absolute()
            .left(px(metrics::SIDEBAR_INSET))
            .top(px(metrics::SIDEBAR_INSET))
            .w(px(metrics::SIDEBAR_WIDTH))
            .h(px(height - 2.0 * metrics::SIDEBAR_INSET))
            .rounded(px(metrics::SIDEBAR_RADIUS))
            .bg(rgb(palette.sidebar))
            .border_1()
            .border_color(rgb(palette.sidebar_edge))
            .overflow_hidden()
            .child(
                div()
                    .id("preview-thumbnails")
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(metrics::TOOLBAR_HEIGHT - metrics::SIDEBAR_INSET))
                    .bottom_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.sidebar_scroll)
                    .on_scroll_wheel(cx.listener(|_, _: &ScrollWheelEvent, _, cx| cx.notify()))
                    .child(div().relative().w_full().h(px(total)).children(children)),
            )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_thumbnail(
        &self,
        palette: Palette,
        index: usize,
        selected: bool,
        label: String,
        image: Option<Arc<RenderImage>>,
        item: ThumbItem,
        documents: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Positions are relative to the sidebar panel (its left edge is 8).
        let selection_left = metrics::THUMB_SELECTION_LEFT - metrics::SIDEBAR_INSET - 1.0;
        let thumb_left = metrics::THUMB_LEFT - metrics::SIDEBAR_INSET - 1.0
            + (layout::THUMB_WIDTH - item.thumb.0) / 2.0;
        div()
            .id(("preview-thumb", index))
            .absolute()
            .left(px(selection_left))
            .top(px(item.top))
            .w(px(metrics::THUMB_SELECTION_WIDTH))
            .h(px(item.height))
            .rounded(px(metrics::THUMB_SELECTION_RADIUS))
            .when(selected, |item| item.bg(rgb(palette.selection)))
            .child(
                div()
                    .absolute()
                    .left(px(thumb_left - selection_left))
                    .top(px(layout::THUMB_PAD))
                    .w(px(item.thumb.0))
                    .h(px(item.thumb.1))
                    .rounded(px(3.0))
                    .overflow_hidden()
                    .bg(rgb(0xFFFFFF))
                    .when_some(image, |thumb, image| thumb.child(img(image).size_full())),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(layout::THUMB_PAD + item.thumb.1))
                    .h(px(layout::THUMB_LABEL))
                    .px(px(4.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(metrics::THUMB_LABEL_SIZE))
                    .text_color(rgb(if selected {
                        0xFFFFFF
                    } else {
                        palette.thumb_label
                    }))
                    .child(div().truncate().child(SharedString::from(label))),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                if documents {
                    this.select(index, cx);
                } else if this.slot().and_then(Slot::kind) == Some(Kind::Pdf) {
                    this.go_to_page(index, cx);
                }
            }))
            .into_any_element()
    }

    fn render_document(
        &mut self,
        palette: Palette,
        left: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let viewport = self.viewport;
        let scroll_top = -f32::from(self.scroll.offset().y);
        let is_pdf = self.slot().and_then(Slot::kind) == Some(Kind::Pdf);
        let body = match self.slots.get(self.selected) {
            None => self.render_empty_state(palette, cx),
            Some(slot) => match &slot.state {
                SlotState::Loading => div().into_any_element(),
                SlotState::Failed(error) => message(error.as_ref(), palette),
                SlotState::Ready(loaded) => match &loaded.content {
                    Content::Image(_) => {
                        let size = slot.page_sizes()[0];
                        let scale = slot.zoom.resolve(slot.fit_scale(viewport));
                        let shown = (size.0 * scale, size.1 * scale);
                        let origin = layout::centred_origin(shown, viewport);
                        div()
                            .relative()
                            .w(px(shown.0.max(viewport.0)))
                            .h(px(shown.1.max(viewport.1)))
                            .when_some(slot.display.clone(), |content, (_, image)| {
                                content.child(
                                    img(image)
                                        .absolute()
                                        .left(px(origin.0))
                                        .top(px(origin.1))
                                        .w(px(shown.0))
                                        .h(px(shown.1)),
                                )
                            })
                            .into_any_element()
                    }
                    Content::Pdf(_) => {
                        let scale = slot.zoom.resolve(slot.fit_scale(viewport));
                        let layout = layout::continuous(&slot.page_sizes(), scale, viewport.0);
                        let visible =
                            layout::visible_pages(&layout.pages, scroll_top, viewport.1, 1);
                        let current = layout::current_page(&layout.pages, scroll_top, viewport.1);
                        let mut highlights = self.page_highlights(slot, palette);
                        for (page, rects) in self.selection_highlights(slot) {
                            highlights.entry(page).or_default().extend(rects);
                        }
                        let pages = visible.map(|page| {
                            let rect = layout.pages[page];
                            render_page(
                                rect,
                                slot.pages.get(&page).map(|bitmap| bitmap.image.clone()),
                                highlights.get(&page).cloned().unwrap_or_default(),
                            )
                        });
                        let element = div()
                            .relative()
                            .w(px(layout.width))
                            .h(px(layout.height.max(viewport.1)))
                            .children(pages)
                            .into_any_element();
                        if let Some(slot) = self.slots.get_mut(self.selected) {
                            slot.current_page = current;
                        }
                        element
                    }
                },
            },
        };
        div()
            .id("preview-document")
            .absolute()
            .left(px(left))
            .top(px(metrics::TOOLBAR_HEIGHT))
            .w(px(viewport.0))
            .h(px(viewport.1))
            .bg(rgb(palette.document))
            .overflow_scroll()
            .track_scroll(&self.scroll)
            .on_scroll_wheel(cx.listener(|_, _: &ScrollWheelEvent, _, cx| cx.notify()))
            .when(is_pdf, |document| {
                document
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
                            this.text_mouse_down(event, window, cx);
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        this.text_mouse_move(event, cx);
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseUpEvent, _, cx| this.text_mouse_up(cx)),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseUpEvent, _, cx| this.text_mouse_up(cx)),
                    )
            })
            .child(body)
            .into_any_element()
    }

    /// The Open panel's placeholder: an explanation plus recently opened
    /// documents, if any — an in-window stand-in for File ▸ Open Recent,
    /// which the D-Bus menu bar cannot show as a dynamic submenu.
    fn render_empty_state(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        if self.recent_documents.is_empty() {
            return message(
                "No document is open. Choose File ▸ Open… to open images or PDF documents.",
                palette,
            );
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(12.0))
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(rgb(palette.subtitle))
                    .child("No document is open. Choose File ▸ Open… or a recent document:"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .max_w(px(420.0))
                    .children(
                        self.recent_documents
                            .iter()
                            .enumerate()
                            .map(|(index, path)| {
                                let name = document::display_name(path);
                                div()
                                    .id(("preview-recent", index))
                                    .px(px(10.0))
                                    .py(px(4.0))
                                    .rounded(px(6.0))
                                    .text_size(px(12.0))
                                    .text_color(rgb(palette.glyph))
                                    .truncate()
                                    .active(|style| style.opacity(0.6))
                                    .child(SharedString::from(name))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.open_recent(index, cx);
                                    }))
                            }),
                    ),
            )
            .into_any_element()
    }

    fn open_recent(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(path) = self.recent_documents.get(index).cloned() else {
            return;
        };
        if self.slots.is_empty() {
            let id = self.next_id;
            self.next_id += 1;
            self.slots.push(Slot::new(id, path));
            self.selected = 0;
            self.start_load(0, cx);
            cx.notify();
        }
    }

    /// Search highlight rectangles per page, in page-relative unit space
    /// after the viewer rotation; the current match is marked.
    fn page_highlights(
        &self,
        slot: &Slot,
        palette: Palette,
    ) -> HashMap<usize, Vec<(layout::UnitRect, u32)>> {
        let mut map: HashMap<usize, Vec<(layout::UnitRect, u32)>> = HashMap::new();
        for (index, found) in self.search.matches.iter().enumerate() {
            let alpha = if Some(index) == self.search.current {
                0xAA
            } else {
                0x55
            };
            let color = (palette.find << 8) | alpha;
            for rect in &found.rects {
                map.entry(found.page)
                    .or_default()
                    .push((slot.rotation.apply_unit_rect(*rect), color));
            }
        }
        map
    }

    fn render_inspector(
        &self,
        palette: Palette,
        width: f32,
        height: f32,
    ) -> Option<impl IntoElement> {
        if !self.inspector {
            return None;
        }
        let slot = self.slot()?;
        let loaded = slot.loaded()?;
        let dash = || "-".to_owned();
        let date = |time: Option<std::time::SystemTime>| {
            time.map(document::format_system_time).unwrap_or_default()
        };
        let mut cards: Vec<Vec<(&'static str, String)>> = vec![vec![
            ("File Name", slot.name.clone()),
            ("Document Type", loaded.kind.document_type().to_owned()),
        ]];
        match &loaded.content {
            Content::Image(image) => {
                cards[0].extend([
                    ("File Size", document::format_file_size(loaded.facts.size)),
                    ("Creation Date", date(loaded.facts.created)),
                    ("Modification Date", date(loaded.facts.modified)),
                ]);
                let (width, height) = if slot.rotation.swaps_axes() {
                    (image.size.1, image.size.0)
                } else {
                    image.size
                };
                cards.push(vec![
                    ("Image Size", document::format_pixels(width, height)),
                    ("Colour Model", image.colour_model.to_owned()),
                ]);
            }
            Content::Pdf(info) => {
                let page = info
                    .pages
                    .get(slot.current_page)
                    .map(|page| {
                        let (width, height) = slot.rotation.apply(page.displayed());
                        document::format_page_size_cm(width, height)
                    })
                    .unwrap_or_else(dash);
                cards.push(vec![
                    ("PDF Version", info.version.clone().unwrap_or_else(dash)),
                    ("Page Count", info.pages.len().to_string()),
                    ("Page Size", page),
                ]);
                let iso = |value: &Option<String>| {
                    value
                        .as_deref()
                        .and_then(document::format_iso_date)
                        .unwrap_or_default()
                };
                cards.push(vec![
                    ("Title", info.title.clone().unwrap_or_else(dash)),
                    ("Author", info.author.clone().unwrap_or_else(dash)),
                    ("Subject", info.subject.clone().unwrap_or_else(dash)),
                    ("PDF Producer", info.producer.clone().unwrap_or_else(dash)),
                    ("Content Creator", info.creator.clone().unwrap_or_else(dash)),
                    ("Creation Date", iso(&info.creation_date)),
                    ("Modification Date", iso(&info.modification_date)),
                ]);
            }
        }
        let card_width =
            metrics::INSPECTOR_WIDTH - metrics::INSPECTOR_CARD_LEFT - metrics::INSPECTOR_CARD_RIGHT;
        let cards = cards.into_iter().map(|rows| {
            let count = rows.len();
            div()
                .w(px(card_width))
                .rounded(px(metrics::INSPECTOR_CARD_RADIUS))
                .bg(rgb(palette.card))
                .overflow_hidden()
                .children(
                    rows.into_iter()
                        .enumerate()
                        .map(move |(index, (label, value))| {
                            div()
                                .h(px(metrics::INSPECTOR_ROW))
                                .mx(px(metrics::INSPECTOR_TEXT_INSET))
                                .flex()
                                .items_center()
                                .gap(px(12.0))
                                .when(index + 1 < count, |row| {
                                    row.border_b_1().border_color(rgb(palette.card_separator))
                                })
                                .child(
                                    div()
                                        .flex_none()
                                        .text_size(px(metrics::INSPECTOR_LABEL_SIZE))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(palette.card_label))
                                        .child(label),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .justify_end()
                                        .text_size(px(metrics::INSPECTOR_LABEL_SIZE))
                                        .text_color(rgb(palette.card_value))
                                        .child(div().truncate().child(SharedString::from(value))),
                                )
                        }),
                )
        });
        Some(
            div()
                .absolute()
                .left(px(width - metrics::INSPECTOR_WIDTH))
                .top(px(metrics::TOOLBAR_HEIGHT))
                .w(px(metrics::INSPECTOR_WIDTH))
                .h(px(height - metrics::TOOLBAR_HEIGHT))
                .bg(rgb(palette.inspector))
                .border_l_1()
                .border_color(rgb(palette.inspector_separator))
                .pt(px(metrics::INSPECTOR_TOP))
                .pl(px(metrics::INSPECTOR_CARD_LEFT - 1.0))
                .flex()
                .flex_col()
                .gap(px(metrics::INSPECTOR_CARD_GAP))
                .children(cards),
        )
    }

    /// Go ▸ Go to Page… (⌥⌘G): a small centred field over the document.
    fn render_go_to_page(&self, palette: Palette, width: f32, height: f32) -> impl IntoElement {
        let pages = self
            .slot()
            .and_then(Slot::loaded)
            .map(Loaded::page_count)
            .unwrap_or(0);
        div()
            .id("preview-go-to-page")
            .absolute()
            .left(px((width - 220.0) / 2.0))
            .top(px(height / 3.0))
            .w(px(220.0))
            .rounded(px(10.0))
            .bg(rgb(palette.card))
            .border_1()
            .border_color(rgb(palette.card_separator))
            .shadow_lg()
            .p(px(10.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.glyph))
                    .child(SharedString::from(format!("Go to page (1–{pages})"))),
            )
            .child(rmac_ui::TextField::new(&self.go_to_page_input).small())
            .into_any_element()
    }
}

/// Recents shown in the empty-window state: capped so a long shared history
/// (Files and other apps add to the same store) never floods one small list.
const MAX_RECENTS_SHOWN: usize = 10;

fn is_openable_document(path: &Path) -> bool {
    let name = path.to_string_lossy();
    name.to_ascii_lowercase().ends_with(".pdf") || document::has_image_extension(&name)
}

/// The documents Preview (and any other rmac app) has opened lately, newest
/// first, filtered to what Preview can itself open.
fn load_recent_documents() -> Vec<PathBuf> {
    let Ok(store) = rmac_recent_documents::Store::from_environment() else {
        return Vec::new();
    };
    let Ok(snapshot) = store.load() else {
        return Vec::new();
    };
    snapshot
        .paths
        .into_iter()
        .filter(|path| is_openable_document(path))
        .take(MAX_RECENTS_SHOWN)
        .collect()
}

/// Records an opened document in the shared Recents store, off the render
/// thread (it touches disk).
fn record_recent_document(path: PathBuf, cx: &mut Context<PreviewView>) {
    cx.background_executor()
        .spawn(async move {
            if let Ok(store) = rmac_recent_documents::Store::from_environment() {
                let _ = store.record(&path);
            }
        })
        .detach();
}

fn image_thumbnail(slot: &Slot) -> Option<Arc<RenderImage>> {
    match slot.loaded().map(|loaded| &loaded.content) {
        Some(Content::Image(image)) => Some(image.thumbnail.clone()),
        Some(Content::Pdf(_)) => slot.thumbs.get(&0).map(|(_, image)| image.clone()),
        None => None,
    }
}

fn message(text: &str, palette: Palette) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .p(px(24.0))
        .text_size(px(13.0))
        .text_color(rgb(palette.subtitle))
        .child(SharedString::from(text.to_owned()))
        .into_any_element()
}

fn render_page(
    rect: Rect,
    image: Option<Arc<RenderImage>>,
    highlights: Vec<(layout::UnitRect, u32)>,
) -> AnyElement {
    div()
        .absolute()
        .left(px(rect.x))
        .top(px(rect.y))
        .w(px(rect.width))
        .h(px(rect.height))
        .bg(rgb(0xFFFFFF))
        .when_some(image, |page, image| page.child(img(image).size_full()))
        .children(highlights.into_iter().map(move |(unit, color)| {
            div()
                .absolute()
                .left(px(unit.x0 * rect.width))
                .top(px(unit.y0 * rect.height))
                .w(px((unit.x1 - unit.x0) * rect.width))
                .h(px((unit.y1 - unit.y0) * rect.height))
                .rounded(px(2.0))
                .bg(rgba(color))
        }))
        .into_any_element()
}

impl Render for PreviewView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for image in self.garbage.drain(..) {
            cx.drop_image(image, Some(window));
        }
        let palette = palette();
        let size = window.viewport_size();
        let (width, height) = (f32::from(size.width), f32::from(size.height));
        let left = metrics::document_left(self.sidebar);
        let right = if self.inspector && self.slot().is_some_and(|slot| slot.loaded().is_some()) {
            metrics::INSPECTOR_WIDTH
        } else {
            0.0
        };
        self.viewport = (
            (width - left - right).max(1.0),
            (height - metrics::TOOLBAR_HEIGHT).max(1.0),
        );
        self.schedule(window, cx);

        let (title, _) = self.title_and_subtitle();
        let native = rmac_ui::native_window_title(&title, "Preview");
        if native != self.title {
            window.set_window_title(&native);
            self.title = native;
        }

        let menu = self.menu.as_ref().map(|state| {
            let check = |on: bool| {
                if on {
                    rmac_ui::MenuCheck::On
                } else {
                    rmac_ui::MenuCheck::None
                }
            };
            rmac_ui::ContextMenu::new(state.position())
                .checked_item("Hide Sidebar", check(!self.sidebar), Box::new(HideSidebar))
                .checked_item("Thumbnails", check(self.sidebar), Box::new(ShowThumbnails))
                .render(state)
        });

        let document = self.render_document(palette, left, cx);
        div()
            .id("preview")
            .track_focus(&self.focus)
            .key_context("Preview")
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key_down(event, window, cx);
            }))
            .on_action(cx.listener(|this, _: &Copy, _, cx| this.copy(cx)))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| this.select_all(cx)))
            .on_action(cx.listener(|this, _: &GoToPage, window, cx| {
                this.open_go_to_page(window, cx);
            }))
            .on_action(cx.listener(|this, _: &PrintDocument, window, cx| {
                this.print_document(window, cx);
            }))
            .on_action(cx.listener(|this, _: &Find, window, cx| {
                if this.slot().and_then(Slot::kind) == Some(Kind::Pdf) {
                    this.search_input
                        .update(cx, |input, cx| input.focus(window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &FindNext, _, cx| this.step_match(1, cx)))
            .on_action(cx.listener(|this, _: &FindPrevious, _, cx| this.step_match(-1, cx)))
            .on_action(cx.listener(|this, _: &HideSidebar, _, cx| this.set_sidebar(false, cx)))
            .on_action(cx.listener(|this, _: &ShowThumbnails, _, cx| this.set_sidebar(true, cx)))
            .on_action(cx.listener(|this, _: &ActualSize, _, cx| this.actual_size(cx)))
            .on_action(cx.listener(|this, _: &ZoomToFit, _, cx| this.zoom_to_fit(cx)))
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.zoom_in(cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.zoom_out(cx)))
            .on_action(cx.listener(|this, _: &PreviousItem, _, cx| this.step_item(-1, cx)))
            .on_action(cx.listener(|this, _: &NextItem, _, cx| this.step_item(1, cx)))
            .on_action(cx.listener(|this, _: &ShowInspector, _, cx| this.toggle_inspector(cx)))
            .on_action(cx.listener(|this, _: &RotateLeft, _, cx| this.rotate(false, cx)))
            .on_action(cx.listener(|this, _: &RotateRight, _, cx| this.rotate(true, cx)))
            .on_action(cx.listener(|this, _: &rmac_ui::DismissMenu, window, cx| {
                if rmac_ui::ContextMenuState::dismiss(&mut this.menu, window, cx) {
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .on_action(
                cx.listener(|_, _: &rmac_ui::RequestClose, window, _| window.remove_window()),
            )
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(palette.window))
            .child(document)
            .when(self.sidebar, |root| {
                root.child(self.render_sidebar(palette, height, cx))
            })
            .children(self.render_inspector(palette, width, height))
            .child(self.render_toolbar(palette, width, window, cx))
            .children(menu)
            .when(self.go_to_page_open, |root| {
                root.child(self.render_go_to_page(palette, width, height))
            })
    }
}
