//! [`RichTextEditor`]: an editable attributed-text view.
//!
//! It answers the same keys and Edit-menu rows as every rmac text field (the
//! "Input" key context and gpui-component's `input::` actions, re-exported
//! by `rmac_ui::input_actions`), takes keyboard and IME text through
//! [`EntityInputHandler`], and lays out only the paragraphs on screen. A
//! paragraph's layout is cached by its content version, so an edit relays
//! out just the paragraphs it changed, and nothing runs while it is idle:
//! the caret blinks only for a couple of seconds after the last keystroke
//! or click (then stays lit, as the Terminal's parked cursor does) and
//! nothing polls.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    point, px, size, App, Bounds, ClipboardItem, Context, CursorStyle, ElementId,
    ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    GlobalElementId, Hsla, InteractiveElement as _, IntoElement, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, Point, Render, Role,
    ScrollWheelEvent, SharedString, StatefulInteractiveElement as _, Style, Styled as _,
    UTF16Selection, Window,
};
use rmac_ui::input_actions as input;

use super::layout::{layout_paragraph, LayoutParams, ParagraphLayout};
use super::model::{
    clamp_size, normalize_newlines, Alignment, CharStyle, Document, DocumentAttributes, Ligatures,
    ListKind, ParagraphStyle, Rgb, MAX_LIST_LEVEL,
};

/// Undo steps kept per editor.
const MAX_UNDO: usize = 256;
/// Documents up to this size measure every paragraph on a width change, so
/// the scroll extent is exact; larger ones estimate unseen paragraphs.
const MEASURE_ALL_BYTES: usize = 64 * 1024;
/// Extra height laid out above and below the viewport.
const OVERDRAW: f32 = 200.0;
/// The insertion point's on and off phases.
const BLINK_PHASE: Duration = Duration::from_millis(530);
/// The caret stops blinking (and stays lit) this long after the last input,
/// so a focused but idle editor schedules no work at all.
const BLINK_IDLE: Duration = Duration::from_secs(2);

/// What the editor tells its owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RichTextEvent {
    /// The document changed (typing, paste, formatting, undo).
    Changed,
}

thread_local! {
    /// The styled copy of the last text this process put on the clipboard,
    /// so a paste inside rmac keeps its formatting.
    static STYLED_CLIPBOARD: RefCell<Option<(String, Document)>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditKind {
    Typing,
    Composing,
    Other,
}

struct UndoEntry {
    document: Document,
    selection: Range<usize>,
    reversed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Granularity {
    Character,
    Word,
    Paragraph,
}

#[derive(Clone)]
struct Placed {
    index: usize,
    /// Window position of the paragraph's text-column origin.
    origin: Point<Pixels>,
    layout: Rc<ParagraphLayout>,
}

/// What painting needs from the last prepaint.
struct Frame {
    placed: Vec<Placed>,
    selection: Vec<Bounds<Pixels>>,
    caret: Option<Bounds<Pixels>>,
    marked: Vec<Bounds<Pixels>>,
    bounds: Bounds<Pixels>,
}

pub struct RichTextEditor {
    focus: FocusHandle,
    document: Document,
    selection: Range<usize>,
    /// The selection's moving end is `selection.start`.
    reversed: bool,
    marked: Option<Range<usize>>,
    /// Formatting chosen with an empty selection, for the next typing.
    typing_style: Option<CharStyle>,
    default_style: CharStyle,
    undo: Vec<UndoEntry>,
    redo: Vec<UndoEntry>,
    last_edit: Option<EditKind>,
    editable: bool,
    revision: u64,

    zoom: f32,
    default_color: Hsla,
    selection_color: Hsla,
    caret_color: Hsla,
    default_family: SharedString,
    mono_family: SharedString,
    paper_color: Hsla,
    link_color: Hsla,
    padding_x: Pixels,
    page_width: Option<Pixels>,

    /// Shared with the overlay scroll bar.
    scroll: rmac_ui::ScrollPosition,
    params: Option<LayoutParams>,
    layouts: HashMap<(u64, u32), Rc<ParagraphLayout>>,
    heights: HashMap<u64, Pixels>,
    numbers: (u64, Rc<Vec<u32>>),
    placed: Vec<Placed>,
    viewport: Bounds<Pixels>,
    reveal: bool,
    goal_x: Option<Pixels>,
    dragging: Option<(Granularity, Range<usize>)>,
    /// A click that landed on a link: opened on release unless it drags.
    pending_link: Option<std::sync::Arc<str>>,
    /// Caret blink: whether the caret shows this phase, which blink task is
    /// current, and when the caret last moved.
    caret_visible: bool,
    blink_epoch: u64,
    last_activity: Instant,
    _focus_subscriptions: Vec<gpui::Subscription>,
}

impl EventEmitter<RichTextEvent> for RichTextEditor {}

impl Focusable for RichTextEditor {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl RichTextEditor {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let default_style = CharStyle::default();
        let focus = cx.focus_handle();
        let focus_subscriptions = vec![
            cx.on_focus(&focus, window, |this, _window, cx| this.restart_blink(cx)),
            cx.on_blur(&focus, window, |this, _window, cx| this.stop_blink(cx)),
        ];
        Self {
            focus,
            document: Document::empty(&default_style),
            selection: 0..0,
            reversed: false,
            marked: None,
            typing_style: None,
            default_style,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: None,
            editable: true,
            revision: 0,
            zoom: 1.0,
            default_color: rmac_ui::mac::text(),
            selection_color: rmac_ui::mac::text_selection(),
            caret_color: rmac_ui::mac::text_caret(),
            default_family: SharedString::from(rmac_ui::UI_FONT),
            mono_family: SharedString::from(rmac_ui::MONO_FONT),
            paper_color: gpui::white(),
            link_color: rmac_ui::mac::system_blue(),
            padding_x: px(10.0),
            page_width: None,
            scroll: rmac_ui::ScrollPosition::default(),
            params: None,
            layouts: HashMap::new(),
            heights: HashMap::new(),
            numbers: (u64::MAX, Rc::new(Vec::new())),
            placed: Vec::new(),
            viewport: Bounds::default(),
            reveal: false,
            goal_x: None,
            dragging: None,
            pending_link: None,
            caret_visible: true,
            blink_epoch: 0,
            last_activity: Instant::now(),
            _focus_subscriptions: focus_subscriptions,
        }
    }

    /// Show the caret and blink it until the editor has been idle for
    /// [`BLINK_IDLE`]. One timer task at a time; a newer start or a blur
    /// ends the older one.
    fn restart_blink(&mut self, cx: &mut Context<Self>) {
        self.caret_visible = true;
        self.last_activity = Instant::now();
        self.blink_epoch = self.blink_epoch.wrapping_add(1);
        let epoch = self.blink_epoch;
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(BLINK_PHASE).await;
            let keep_going = this
                .update(cx, |this, cx| {
                    if this.blink_epoch != epoch {
                        return false;
                    }
                    if this.last_activity.elapsed() >= BLINK_IDLE {
                        if !this.caret_visible {
                            this.caret_visible = true;
                            cx.notify();
                        }
                        return false;
                    }
                    this.caret_visible = !this.caret_visible;
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !keep_going {
                break;
            }
        })
        .detach();
    }

    fn stop_blink(&mut self, cx: &mut Context<Self>) {
        self.blink_epoch = self.blink_epoch.wrapping_add(1);
        self.caret_visible = true;
        cx.notify();
    }

    // ---- Owner API -------------------------------------------------------

    /// Replace the whole document (open, revert, recovery). Clears undo and
    /// puts the caret at the start; emits no change event.
    pub fn set_document(&mut self, document: Document, cx: &mut Context<Self>) {
        self.document = document;
        self.selection = 0..0;
        self.reversed = false;
        self.marked = None;
        self.typing_style = None;
        self.undo.clear();
        self.redo.clear();
        self.last_edit = None;
        self.scroll.set_top(px(0.0));
        self.revision += 1;
        self.goal_x = None;
        cx.notify();
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    pub fn text(&self) -> String {
        self.document.text()
    }

    /// The style a new document's text and Make Rich Text conversions use.
    pub fn set_default_style(&mut self, style: CharStyle) {
        self.default_style = style;
    }

    pub fn default_style(&self) -> &CharStyle {
        &self.default_style
    }

    pub fn selected_range(&self) -> Range<usize> {
        self.selection.clone()
    }

    pub fn cursor(&self) -> usize {
        self.head()
    }

    pub fn marked_range(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn is_editable(&self) -> bool {
        self.editable
    }

    pub fn set_editable(&mut self, editable: bool, cx: &mut Context<Self>) {
        if self.editable != editable {
            self.editable = editable;
            cx.notify();
        }
    }

    /// View ▸ Zoom: scales the whole document on screen.
    pub fn set_zoom(&mut self, zoom: f32, cx: &mut Context<Self>) {
        let zoom = zoom.clamp(0.25, 8.0);
        if (self.zoom - zoom).abs() > f32::EPSILON {
            self.zoom = zoom;
            cx.notify();
        }
    }

    /// Colours and faces from the owner's appearance: automatic text colour
    /// (black on paper, white on a dark background), selection and caret.
    pub fn set_appearance(
        &mut self,
        default_color: Hsla,
        selection_color: Hsla,
        caret_color: Hsla,
        cx: &mut Context<Self>,
    ) {
        if self.default_color != default_color
            || self.selection_color != selection_color
            || self.caret_color != caret_color
        {
            self.default_color = default_color;
            self.selection_color = selection_color;
            self.caret_color = caret_color;
            cx.notify();
        }
    }

    /// The paper colour behind the text: what Format ▸ Font ▸ Outline
    /// fills its glyphs with.
    pub fn set_paper_color(&mut self, paper_color: Hsla, cx: &mut Context<Self>) {
        if self.paper_color != paper_color {
            self.paper_color = paper_color;
            cx.notify();
        }
    }

    /// Left/right text inset, and Wrap to Page's column width.
    pub fn set_column(
        &mut self,
        padding_x: Pixels,
        page_width: Option<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.padding_x != padding_x || self.page_width != page_width {
            self.padding_x = padding_x;
            self.page_width = page_width;
            cx.notify();
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    /// Select `range` (UTF-8 bytes) and scroll it into view.
    pub fn select_range(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        let range = self.document.clamp_range(range);
        self.set_selection(range, false, cx);
    }

    /// Replace `range` with plain text through the undoable path (Find and
    /// Replace, Transformations, Spelling, assistive technology).
    pub fn replace_range(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        if !self.editable {
            return;
        }
        let range = self.document.clamp_range(range);
        self.insert(range, text, EditKind::Other, cx);
    }

    /// Replace every one of `ranges` (sorted, not overlapping) with `text`
    /// as one undo step: Find's Replace All.
    pub fn replace_ranges(&mut self, ranges: &[Range<usize>], text: &str, cx: &mut Context<Self>) {
        if !self.editable || ranges.is_empty() {
            return;
        }
        self.begin_edit(EditKind::Other);
        for range in ranges.iter().rev() {
            let range = self.document.clamp_range(range.clone());
            let style = self.document.style_of_char_at(range.start);
            self.document.replace_text(range, text, &style);
        }
        let caret = self.document.clamp_offset(self.selection.start);
        self.selection = caret..caret;
        self.reversed = false;
        self.typing_style = None;
        self.finish_edit(cx);
    }

    /// The character style at the selection (its first character), or the
    /// typing style at the caret: what Format ▸ Font's checkmarks show.
    pub fn style_at_selection(&self) -> CharStyle {
        if let Some(style) = &self.typing_style {
            return style.clone();
        }
        if self.selection.is_empty() {
            self.document.style_at(self.selection.start)
        } else {
            self.document.style_of_char_at(self.selection.start)
        }
    }

    pub fn paragraph_style_at_selection(&self) -> ParagraphStyle {
        self.document.paragraph_style_at(self.selection.start)
    }

    /// Format ▸ Font ▸ Bold (⌘B): on unless the selection's first
    /// character is already bold, as NSFontManager decides.
    pub fn toggle_bold(&mut self, cx: &mut Context<Self>) {
        let on = !self.style_at_selection().bold;
        self.change_char_style(cx, move |style| style.bold = on);
    }

    pub fn toggle_italic(&mut self, cx: &mut Context<Self>) {
        let on = !self.style_at_selection().italic;
        self.change_char_style(cx, move |style| style.italic = on);
    }

    pub fn toggle_underline(&mut self, cx: &mut Context<Self>) {
        let on = !self.style_at_selection().underline;
        self.change_char_style(cx, move |style| style.underline = on);
    }

    pub fn toggle_strikethrough(&mut self, cx: &mut Context<Self>) {
        let on = !self.style_at_selection().strikethrough;
        self.change_char_style(cx, move |style| style.strikethrough = on);
    }

    /// Format ▸ Font ▸ Bigger / Smaller: every character's own size moves
    /// by `delta` points.
    pub fn change_size(&mut self, delta: f32, cx: &mut Context<Self>) {
        self.change_char_style(cx, move |style| style.size = clamp_size(style.size + delta));
    }

    /// The Fonts panel's family; `None` is the document's default face.
    pub fn set_family(&mut self, family: Option<std::sync::Arc<str>>, cx: &mut Context<Self>) {
        self.change_char_style(cx, move |style| style.family = family.clone());
    }

    /// The Fonts panel's size, in points.
    pub fn set_size(&mut self, size: f32, cx: &mut Context<Self>) {
        let size = clamp_size(size);
        self.change_char_style(cx, move |style| style.size = size);
    }

    /// The Colours panel's text colour; `None` is automatic.
    pub fn set_text_color(&mut self, color: Option<Rgb>, cx: &mut Context<Self>) {
        self.change_char_style(cx, move |style| style.color = color);
    }

    /// Format ▸ Font ▸ Highlight.
    pub fn set_highlight(&mut self, color: Option<Rgb>, cx: &mut Context<Self>) {
        self.change_char_style(cx, move |style| style.highlight = color);
    }

    /// Format ▸ Font ▸ Paste Style and Styles… ▸ Apply: the copied
    /// character style, whole, except that each character keeps its own
    /// link (a link is content, not style).
    pub fn apply_char_style(&mut self, copied: CharStyle, cx: &mut Context<Self>) {
        self.change_char_style(cx, move |style| {
            let link = style.link.take();
            *style = copied.clone();
            style.link = link;
        });
    }

    pub fn set_alignment(&mut self, alignment: Alignment, cx: &mut Context<Self>) {
        self.change_paragraph_style(cx, move |style| style.alignment = alignment);
    }

    pub fn set_list(&mut self, list: Option<ListKind>, cx: &mut Context<Self>) {
        self.set_list_marker(list, cx);
    }

    pub fn set_line_spacing(&mut self, spacing: f32, cx: &mut Context<Self>) {
        let spacing = spacing.clamp(0.5, 4.0);
        self.change_paragraph_style(cx, move |style| style.line_spacing = spacing);
    }

    /// Format ▸ Text ▸ Paste Ruler.
    pub fn apply_paragraph_style(&mut self, ruler: ParagraphStyle, cx: &mut Context<Self>) {
        self.change_paragraph_style(cx, move |style| *style = ruler);
    }

    /// Format ▸ Font ▸ Outline: on unless the selection is already
    /// outlined.
    pub fn toggle_outline(&mut self, cx: &mut Context<Self>) {
        let on = !self.style_at_selection().outline;
        self.change_char_style(cx, move |style| style.outline = on);
    }

    /// Format ▸ Font ▸ Kern ▸ Use Default (`None`) / Use None (`Some(0)`).
    pub fn set_kern(&mut self, kern: Option<f32>, cx: &mut Context<Self>) {
        self.change_char_style(cx, move |style| style.kern = kern);
    }

    /// Format ▸ Font ▸ Kern ▸ Tighten / Loosen: each character's own
    /// spacing moves by `delta` points (NSTextView's tighten/loosenKerning).
    pub fn adjust_kern(&mut self, delta: f32, cx: &mut Context<Self>) {
        self.change_char_style(cx, move |style| {
            let kern = (style.kern.unwrap_or(0.0) + delta).clamp(-20.0, 50.0);
            style.kern = Some(kern);
        });
    }

    /// Format ▸ Font ▸ Ligatures.
    pub fn set_ligatures(&mut self, ligatures: Ligatures, cx: &mut Context<Self>) {
        self.change_char_style(cx, move |style| style.ligatures = ligatures);
    }

    /// Format ▸ Font ▸ Baseline ▸ Use Default: no superscript and no
    /// offset.
    pub fn reset_baseline(&mut self, cx: &mut Context<Self>) {
        self.change_char_style(cx, |style| {
            style.superscript = 0;
            style.baseline_offset = 0.0;
        });
    }

    /// Format ▸ Font ▸ Baseline ▸ Superscript (+1) / Subscript (-1): each
    /// character's level moves one step, as NSTextView's superscript: and
    /// subscript: do.
    pub fn adjust_superscript(&mut self, delta: i8, cx: &mut Context<Self>) {
        self.change_char_style(cx, move |style| {
            style.superscript = style.superscript.saturating_add(delta).clamp(-8, 8);
        });
    }

    /// Format ▸ Font ▸ Baseline ▸ Raise / Lower: the offset moves by
    /// `delta` points.
    pub fn adjust_baseline(&mut self, delta: f32, cx: &mut Context<Self>) {
        self.change_char_style(cx, move |style| {
            style.baseline_offset = (style.baseline_offset + delta).clamp(-100.0, 100.0);
        });
    }

    /// Format ▸ Font ▸ Character Shape ▸ Traditional Form.
    pub fn toggle_traditional(&mut self, cx: &mut Context<Self>) {
        let on = !self.style_at_selection().traditional;
        self.change_char_style(cx, move |style| style.traditional = on);
    }

    /// Edit ▸ Link…: link the selection to `target`, or remove its link
    /// (`None`). With nothing selected, the address is inserted as linked
    /// text, as TextEdit does.
    pub fn set_link(&mut self, target: Option<std::sync::Arc<str>>, cx: &mut Context<Self>) {
        if !self.editable {
            return;
        }
        if self.selection.is_empty() {
            let Some(target) = target else {
                return;
            };
            let mut style = self.style_at_selection();
            style.link = Some(target.clone());
            self.begin_edit(EditKind::Other);
            let inserted = self
                .document
                .replace_text(self.selection.clone(), &target, &style);
            self.selection = inserted.end..inserted.end;
            self.reversed = false;
            self.typing_style = None;
            self.finish_edit(cx);
            return;
        }
        self.change_char_style(cx, move |style| style.link = target.clone());
    }

    /// The link at the selection's start, for Edit ▸ Link…'s field.
    pub fn link_at_selection(&self) -> Option<std::sync::Arc<str>> {
        self.style_at_selection().link
    }

    /// Format ▸ List… with a level: the selected paragraphs' list marker,
    /// keeping their levels (none ends the list and its nesting).
    pub fn set_list_marker(&mut self, list: Option<ListKind>, cx: &mut Context<Self>) {
        self.change_paragraph_style(cx, move |style| {
            style.list = list;
            if list.is_none() {
                style.list_level = 0;
            }
        });
    }

    /// Tab / ⇧Tab at a list item's start: nest it one level deeper or
    /// bring it out one level.
    pub fn change_list_level(&mut self, delta: i8, cx: &mut Context<Self>) {
        self.change_paragraph_style(cx, move |style| {
            if style.list.is_some() {
                style.list_level =
                    (style.list_level as i8 + delta).clamp(0, MAX_LIST_LEVEL as i8) as u8;
            }
        });
    }

    /// Hyphenation and properties (Format ▸ Allow Hyphenation, File ▸ Show
    /// Properties), as one undoable change.
    pub fn set_document_attributes(
        &mut self,
        attributes: DocumentAttributes,
        cx: &mut Context<Self>,
    ) {
        if !self.editable || *self.document.attributes() == attributes {
            return;
        }
        self.begin_edit(EditKind::Other);
        self.document.set_attributes(attributes);
        self.finish_edit(cx);
    }

    // ---- Edits -----------------------------------------------------------

    fn head(&self) -> usize {
        if self.reversed {
            self.selection.start
        } else {
            self.selection.end
        }
    }

    fn anchor(&self) -> usize {
        if self.reversed {
            self.selection.end
        } else {
            self.selection.start
        }
    }

    fn snapshot(&self) -> UndoEntry {
        UndoEntry {
            document: self.document.clone(),
            selection: self.selection.clone(),
            reversed: self.reversed,
        }
    }

    fn begin_edit(&mut self, kind: EditKind) {
        let coalesce = matches!(
            (self.last_edit, kind),
            (Some(EditKind::Typing), EditKind::Typing)
                | (Some(EditKind::Composing), EditKind::Composing)
                | (Some(EditKind::Composing), EditKind::Typing)
        );
        if !coalesce || self.undo.is_empty() {
            let entry = self.snapshot();
            self.undo.push(entry);
            if self.undo.len() > MAX_UNDO {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last_edit = Some(kind);
    }

    fn finish_edit(&mut self, cx: &mut Context<Self>) {
        self.restart_blink(cx);
        self.revision += 1;
        self.reveal = true;
        self.goal_x = None;
        cx.emit(RichTextEvent::Changed);
        cx.notify();
    }

    /// Insert `text` over `range` in the typing style. Returns the inserted
    /// range.
    fn insert(
        &mut self,
        range: Range<usize>,
        text: &str,
        kind: EditKind,
        cx: &mut Context<Self>,
    ) -> Range<usize> {
        let style = self.typing_style.clone().unwrap_or_else(|| {
            if range.is_empty() {
                let mut style = self.document.style_at(range.start);
                // Typing just after a link does not extend it.
                let (index, local) = self.document.locate(range.start);
                let paragraph = self.document.paragraph(index);
                if style.link.is_some()
                    && (local >= paragraph.len()
                        || paragraph.style_of_char_at(local).link != style.link)
                {
                    style.link = None;
                }
                style
            } else {
                self.document.style_of_char_at(range.start)
            }
        });
        self.begin_edit(kind);
        let text = normalize_newlines(text);
        let inserted = self.document.replace_text(range, &text, &style);
        if !text.is_empty() {
            self.typing_style = None;
        }
        self.selection = inserted.end..inserted.end;
        self.reversed = false;
        self.finish_edit(cx);
        inserted
    }

    fn change_char_style(&mut self, cx: &mut Context<Self>, change: impl Fn(&mut CharStyle)) {
        if !self.editable {
            return;
        }
        if self.selection.is_empty() {
            let mut style = self.style_at_selection();
            change(&mut style);
            self.typing_style = Some(style);
            cx.notify();
            return;
        }
        self.begin_edit(EditKind::Other);
        self.document
            .update_char_style(self.selection.clone(), &change);
        self.finish_edit(cx);
    }

    fn change_paragraph_style(
        &mut self,
        cx: &mut Context<Self>,
        change: impl Fn(&mut ParagraphStyle),
    ) {
        if !self.editable {
            return;
        }
        let before = self.document.clone();
        let range = self.selection.clone();
        self.document.update_paragraph_style(range, &change);
        if self.document == before {
            return;
        }
        let after = std::mem::replace(&mut self.document, before);
        self.begin_edit(EditKind::Other);
        self.document = after;
        self.finish_edit(cx);
    }

    fn set_selection(&mut self, range: Range<usize>, reversed: bool, cx: &mut Context<Self>) {
        let changed = self.selection != range || self.reversed != reversed;
        self.selection = range;
        self.reversed = reversed && !self.selection.is_empty();
        if changed {
            self.typing_style = None;
            self.last_edit = None;
        }
        self.marked = None;
        self.reveal = true;
        self.restart_blink(cx);
        cx.notify();
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.goal_x = None;
        let offset = self.document.clamp_offset(offset);
        self.set_selection(offset..offset, false, cx);
    }

    /// Extend the selection's moving end to `offset`.
    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.goal_x = None;
        let anchor = self.anchor();
        let offset = self.document.clamp_offset(offset);
        if offset < anchor {
            self.set_selection(offset..anchor, true, cx);
        } else {
            self.set_selection(anchor..offset, false, cx);
        }
    }

    fn delete_range(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        if !self.editable || range.is_empty() {
            return;
        }
        self.typing_style = None;
        self.insert(range, "", EditKind::Other, cx);
    }

    // ---- Geometry --------------------------------------------------------

    fn list_numbers(&mut self) -> Rc<Vec<u32>> {
        if self.numbers.0 != self.revision
            || self.numbers.1.len() != self.document.paragraph_count()
        {
            self.numbers = (self.revision, Rc::new(self.document.list_numbers()));
        }
        self.numbers.1.clone()
    }

    fn ensure_layout(
        &mut self,
        index: usize,
        number: u32,
        params: &LayoutParams,
        window: &Window,
    ) -> Rc<ParagraphLayout> {
        let paragraph = self.document.paragraph(index);
        let key = (paragraph.version(), number);
        if let Some(layout) = self.layouts.get(&key) {
            return layout.clone();
        }
        let layout = Rc::new(layout_paragraph(paragraph, number, params, window));
        self.heights.insert(paragraph.version(), layout.height);
        self.layouts.insert(key, layout.clone());
        layout
    }

    fn estimated_height(&self, index: usize, params: &LayoutParams) -> Pixels {
        let paragraph = self.document.paragraph(index);
        let size = paragraph.style_of_char_at(0).size * params.zoom;
        let per_line = (f32::from(params.width) / (size * 0.5)).max(1.0);
        let lines = (paragraph.text().chars().count() as f32 / per_line)
            .ceil()
            .max(1.0);
        px((lines * size * 1.25 * paragraph.style().line_spacing).ceil())
    }

    fn column(&self, bounds: Bounds<Pixels>) -> (Pixels, Pixels) {
        let full = (bounds.size.width - self.padding_x * 2.0).max(px(40.0));
        match self.page_width {
            Some(page) if page < full => {
                let left = bounds.left() + (bounds.size.width - page) / 2.0;
                (left, page)
            }
            _ => (bounds.left() + self.padding_x, full),
        }
    }

    /// Lay out what is visible, keep the caret in view when asked, and
    /// compute what paint draws.
    fn prepare_frame(&mut self, bounds: Bounds<Pixels>, window: &mut Window) -> Frame {
        self.viewport = bounds;
        let (left, width) = self.column(bounds);
        let params = LayoutParams {
            width,
            zoom: self.zoom,
            default_color: self.default_color,
            default_family: self.default_family.clone(),
            mono_family: self.mono_family.clone(),
            paper_color: self.paper_color,
            link_color: self.link_color,
            hyphenation: self.document.attributes().hyphenation,
        };
        if self.params.as_ref() != Some(&params) {
            self.layouts.clear();
            self.heights.clear();
            self.params = Some(params.clone());
        }
        let numbers = self.list_numbers();
        let count = self.document.paragraph_count();
        let measure_all = self.document.len() <= MEASURE_ALL_BYTES;
        let view_height = bounds.size.height;
        let overdraw = px(OVERDRAW);
        let (caret_index, caret_local) = self.document.locate(self.head());

        let mut placed = Vec::new();
        let mut content_height = px(0.0);
        for _pass in 0..3 {
            placed.clear();
            let mut y = px(0.0);
            let mut caret_span = None;
            let scroll_top = self.scroll.top();
            for index in 0..count {
                let version = self.document.paragraph(index).version();
                let mut height = match self.heights.get(&version) {
                    Some(height) => *height,
                    None if measure_all => {
                        self.ensure_layout(index, numbers[index], &params, window)
                            .height
                    }
                    None => self.estimated_height(index, &params),
                };
                let visible =
                    y + height > scroll_top - overdraw && y < scroll_top + view_height + overdraw;
                let caret_here = index == caret_index && self.reveal;
                if visible || caret_here {
                    let layout = self.ensure_layout(index, numbers[index], &params, window);
                    height = layout.height;
                    if caret_here {
                        let (_, top, line_height) = layout.caret(caret_local);
                        caret_span = Some((y + top, line_height));
                    }
                    if y + height > scroll_top - overdraw && y < scroll_top + view_height + overdraw
                    {
                        placed.push(Placed {
                            index,
                            origin: point(left, bounds.top() + y - scroll_top),
                            layout,
                        });
                    }
                }
                y += height;
            }
            let max_scroll = (y - view_height).max(px(0.0));
            let mut target = self.scroll.top().clamp(px(0.0), max_scroll);
            if let Some((top, line_height)) = caret_span {
                if top < target {
                    target = top;
                } else if top + line_height > target + view_height {
                    target = (top + line_height - view_height).min(max_scroll);
                }
            }
            content_height = y;
            if target == self.scroll.top() {
                break;
            }
            self.scroll.set_top(target);
        }
        self.reveal = false;
        self.scroll
            .set_content_size(size(bounds.size.width, content_height.max(view_height)));

        // Keep only what is on screen; heights stay for scrolling.
        let keep: HashSet<(u64, u32)> = placed
            .iter()
            .map(|p| (self.document.paragraph(p.index).version(), numbers[p.index]))
            .collect();
        self.layouts.retain(|key, _| keep.contains(key));
        if self.heights.len() > count * 2 + 64 {
            let live: HashSet<u64> = self
                .document
                .paragraphs()
                .iter()
                .map(|p| p.version())
                .collect();
            self.heights.retain(|version, _| live.contains(version));
        }

        let mut selection = Vec::new();
        let mut caret = None;
        let mut marked = Vec::new();
        let range = self.selection.clone();
        let focused = self.focus.is_focused(window);
        for placed_paragraph in &placed {
            let paragraph_range = self.document.paragraph_range(placed_paragraph.index);
            let origin = placed_paragraph.origin;
            let layout = &placed_paragraph.layout;
            let to_bounds = |(x1, y1, x2, y2): (Pixels, Pixels, Pixels, Pixels)| {
                Bounds::from_corners(
                    point(origin.x + x1, origin.y + y1),
                    point(origin.x + x2, origin.y + y2),
                )
            };
            if !range.is_empty()
                && range.start <= paragraph_range.end
                && range.end >= paragraph_range.start
            {
                let local_start = range.start.max(paragraph_range.start) - paragraph_range.start;
                let local_end = range.end.min(paragraph_range.end) - paragraph_range.start;
                let through_end = range.end > paragraph_range.end;
                selection.extend(
                    layout
                        .selection_rects(local_start..local_end, through_end)
                        .into_iter()
                        .map(to_bounds),
                );
            }
            if let Some(marked_range) = &self.marked {
                if marked_range.start <= paragraph_range.end
                    && marked_range.end >= paragraph_range.start
                {
                    let local_start =
                        marked_range.start.max(paragraph_range.start) - paragraph_range.start;
                    let local_end =
                        marked_range.end.min(paragraph_range.end) - paragraph_range.start;
                    for (x1, _, x2, y2) in layout.selection_rects(local_start..local_end, false) {
                        marked.push(Bounds::from_corners(
                            point(origin.x + x1, origin.y + y2 - px(2.0)),
                            point(origin.x + x2, origin.y + y2 - px(1.0)),
                        ));
                    }
                }
            }
            if range.is_empty()
                && focused
                && self.editable
                && self.caret_visible
                && placed_paragraph.index == caret_index
            {
                let (x, top, height) = layout.caret(caret_local);
                caret = Some(Bounds::new(
                    point(origin.x + x, origin.y + top),
                    size(px(1.0), height),
                ));
            }
        }
        self.placed = placed.clone();
        Frame {
            placed,
            selection,
            caret,
            marked,
            bounds,
        }
    }

    /// The document offset under a window position.
    fn offset_at(&self, position: Point<Pixels>) -> usize {
        let Some(first) = self.placed.first() else {
            return 0;
        };
        let target = self
            .placed
            .iter()
            .find(|placed| position.y < placed.origin.y + placed.layout.height)
            .unwrap_or_else(|| self.placed.last().unwrap_or(first));
        let local = target.layout.offset_for_point(
            position.x - target.origin.x,
            (position.y - target.origin.y).max(px(0.0)),
        );
        self.document.paragraph_start(target.index) + local
    }

    fn layout_for(&mut self, index: usize, window: &Window) -> Option<Rc<ParagraphLayout>> {
        let params = self.params.clone()?;
        let numbers = self.list_numbers();
        Some(self.ensure_layout(index, numbers[index], &params, window))
    }

    /// The offset `lines` visual lines above (negative) or below `from`,
    /// keeping the column TextEdit remembers across vertical moves.
    fn vertical_target(&mut self, from: usize, lines: i32, window: &Window) -> usize {
        let (mut index, local) = self.document.locate(from);
        let Some(mut layout) = self.layout_for(index, window) else {
            return from;
        };
        let mut line = layout.line_for_offset(local);
        let x = *self.goal_x.get_or_insert_with(|| layout.x_for(line, local));
        let count = self.document.paragraph_count();
        let mut remaining = lines;
        while remaining != 0 {
            if remaining < 0 {
                if line > 0 {
                    line -= 1;
                } else if index > 0 {
                    index -= 1;
                    let Some(previous) = self.layout_for(index, window) else {
                        return from;
                    };
                    layout = previous;
                    line = layout.lines.len() - 1;
                } else {
                    return 0;
                }
                remaining += 1;
            } else {
                if line + 1 < layout.lines.len() {
                    line += 1;
                } else if index + 1 < count {
                    index += 1;
                    let Some(next) = self.layout_for(index, window) else {
                        return from;
                    };
                    layout = next;
                    line = 0;
                } else {
                    return self.document.len();
                }
                remaining -= 1;
            }
        }
        self.document.paragraph_start(index) + layout.offset_for_x(line, x)
    }

    /// Start and end of the visual line holding `offset`.
    fn line_bounds(&mut self, offset: usize, window: &Window) -> (usize, usize) {
        let (index, local) = self.document.locate(offset);
        let start = self.document.paragraph_start(index);
        match self.layout_for(index, window) {
            Some(layout) => {
                let line = layout.line_for_offset(local);
                (
                    start + layout.lines[line].start,
                    start + layout.line_caret_end(line),
                )
            }
            None => {
                let range = self.document.paragraph_range(index);
                (range.start, range.end)
            }
        }
    }

    fn page_lines(&self) -> i32 {
        let line = px(16.0 * self.zoom);
        ((self.viewport.size.height / line) as i32 - 1).max(1)
    }

    // ---- Actions ---------------------------------------------------------

    fn move_left(&mut self, _: &input::MoveLeft, _: &mut Window, cx: &mut Context<Self>) {
        let target = if self.selection.is_empty() {
            self.document.previous_boundary(self.head())
        } else {
            self.selection.start
        };
        self.move_to(target, cx);
    }

    fn move_right(&mut self, _: &input::MoveRight, _: &mut Window, cx: &mut Context<Self>) {
        let target = if self.selection.is_empty() {
            self.document.next_boundary(self.head())
        } else {
            self.selection.end
        };
        self.move_to(target, cx);
    }

    fn move_up(&mut self, _: &input::MoveUp, window: &mut Window, cx: &mut Context<Self>) {
        let from = self.selection.start;
        let target = self.vertical_target(from, -1, window);
        self.move_vertically(target, cx);
    }

    fn move_down(&mut self, _: &input::MoveDown, window: &mut Window, cx: &mut Context<Self>) {
        let from = self.selection.end;
        let target = self.vertical_target(from, 1, window);
        self.move_vertically(target, cx);
    }

    fn move_vertically(&mut self, target: usize, cx: &mut Context<Self>) {
        let goal = self.goal_x;
        self.set_selection(target..target, false, cx);
        self.goal_x = goal;
    }

    fn page_up(&mut self, _: &input::MovePageUp, window: &mut Window, cx: &mut Context<Self>) {
        let lines = self.page_lines();
        let target = self.vertical_target(self.head(), -lines, window);
        self.move_vertically(target, cx);
    }

    fn page_down(&mut self, _: &input::MovePageDown, window: &mut Window, cx: &mut Context<Self>) {
        let lines = self.page_lines();
        let target = self.vertical_target(self.head(), lines, window);
        self.move_vertically(target, cx);
    }

    fn select_left(&mut self, cx: &mut Context<Self>) {
        let target = self.document.previous_boundary(self.head());
        self.goal_x = None;
        self.select_to(target, cx);
    }

    fn select_right(&mut self, cx: &mut Context<Self>) {
        let target = self.document.next_boundary(self.head());
        self.goal_x = None;
        self.select_to(target, cx);
    }

    fn select_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.vertical_target(self.head(), -1, window);
        let goal = self.goal_x;
        self.select_to(target, cx);
        self.goal_x = goal;
    }

    fn select_down(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.vertical_target(self.head(), 1, window);
        let goal = self.goal_x;
        self.select_to(target, cx);
        self.goal_x = goal;
    }

    fn move_home(&mut self, _: &input::MoveHome, window: &mut Window, cx: &mut Context<Self>) {
        let (start, _) = self.line_bounds(self.selection.start, window);
        self.move_to(start, cx);
    }

    fn move_end(&mut self, _: &input::MoveEnd, window: &mut Window, cx: &mut Context<Self>) {
        let (_, end) = self.line_bounds(self.selection.end, window);
        self.move_to(end, cx);
    }

    fn move_line_start(
        &mut self,
        _: &input::MoveToStartOfLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (start, _) = self.line_bounds(self.selection.start, window);
        self.move_to(start, cx);
    }

    fn move_line_end(
        &mut self,
        _: &input::MoveToEndOfLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (_, end) = self.line_bounds(self.selection.end, window);
        self.move_to(end, cx);
    }

    fn move_to_start(&mut self, _: &input::MoveToStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn move_to_end(&mut self, _: &input::MoveToEnd, _: &mut Window, cx: &mut Context<Self>) {
        let end = self.document.len();
        self.move_to(end, cx);
    }

    fn move_previous_word(
        &mut self,
        _: &input::MoveToPreviousWord,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.document.previous_word_start(self.selection.start);
        self.move_to(target, cx);
    }

    fn move_next_word(
        &mut self,
        _: &input::MoveToNextWord,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.document.next_word_end(self.selection.end);
        self.move_to(target, cx);
    }

    fn select_line_start(
        &mut self,
        _: &input::SelectToStartOfLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (start, _) = self.line_bounds(self.head(), window);
        self.select_to(start, cx);
    }

    fn select_line_end(
        &mut self,
        _: &input::SelectToEndOfLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (_, end) = self.line_bounds(self.head(), window);
        self.select_to(end, cx);
    }

    fn select_to_start(
        &mut self,
        _: &input::SelectToStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(0, cx);
    }

    fn select_to_end(&mut self, _: &input::SelectToEnd, _: &mut Window, cx: &mut Context<Self>) {
        let end = self.document.len();
        self.select_to(end, cx);
    }

    fn select_previous_word(
        &mut self,
        _: &input::SelectToPreviousWordStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.document.previous_word_start(self.head());
        self.select_to(target, cx);
    }

    fn select_next_word(
        &mut self,
        _: &input::SelectToNextWordEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.document.next_word_end(self.head());
        self.select_to(target, cx);
    }

    fn select_all(&mut self, _: &input::SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        let end = self.document.len();
        self.goal_x = None;
        self.set_selection(0..end, false, cx);
    }

    fn backspace(&mut self, _: &input::Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if !self.editable {
            return;
        }
        if self.selection.is_empty() {
            let head = self.head();
            let (index, local) = self.document.locate(head);
            // ⌫ at the start of a list item ends the list there first.
            if local == 0 && self.document.paragraph(index).style().list.is_some() {
                self.set_list(None, cx);
                return;
            }
            let previous = self.document.previous_boundary(head);
            self.delete_range(previous..head, cx);
        } else {
            self.delete_range(self.selection.clone(), cx);
        }
    }

    fn delete(&mut self, _: &input::Delete, _: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            let head = self.head();
            let next = self.document.next_boundary(head);
            self.delete_range(head..next, cx);
        } else {
            self.delete_range(self.selection.clone(), cx);
        }
    }

    fn delete_to_line_start(
        &mut self,
        _: &input::DeleteToBeginningOfLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selection.is_empty() {
            let head = self.head();
            let (start, _) = self.line_bounds(head, window);
            let start = if start == head {
                self.document.previous_boundary(head)
            } else {
                start
            };
            self.delete_range(start..head, cx);
        } else {
            self.delete_range(self.selection.clone(), cx);
        }
    }

    fn delete_to_line_end(
        &mut self,
        _: &input::DeleteToEndOfLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selection.is_empty() {
            let head = self.head();
            let (_, end) = self.line_bounds(head, window);
            let end = if end == head {
                self.document.next_boundary(head)
            } else {
                end
            };
            self.delete_range(head..end, cx);
        } else {
            self.delete_range(self.selection.clone(), cx);
        }
    }

    fn delete_previous_word(
        &mut self,
        _: &input::DeleteToPreviousWordStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selection.is_empty() {
            let head = self.head();
            let start = self.document.previous_word_start(head);
            self.delete_range(start..head, cx);
        } else {
            self.delete_range(self.selection.clone(), cx);
        }
    }

    fn delete_next_word(
        &mut self,
        _: &input::DeleteToNextWordEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selection.is_empty() {
            let head = self.head();
            let end = self.document.next_word_end(head);
            self.delete_range(head..end, cx);
        } else {
            self.delete_range(self.selection.clone(), cx);
        }
    }

    fn enter(&mut self, _: &input::Enter, _: &mut Window, cx: &mut Context<Self>) {
        if !self.editable {
            return;
        }
        // Return in an empty list item ends the list, as in TextEdit.
        if self.selection.is_empty() {
            let (index, _) = self.document.locate(self.head());
            let paragraph = self.document.paragraph(index);
            if paragraph.is_empty() && paragraph.style().list.is_some() {
                // A nested empty item comes out a level first.
                if paragraph.style().list_level > 0 {
                    self.change_list_level(-1, cx);
                } else {
                    self.set_list(None, cx);
                }
                return;
            }
        }
        self.insert(self.selection.clone(), "\n", EditKind::Typing, cx);
    }

    /// Whether Tab / ⇧Tab change list levels here: the caret at a list
    /// item's start, or a selection across list items.
    fn at_list_item_start(&self) -> bool {
        let (index, local) = self.document.locate(self.selection.start);
        let in_list = self.document.paragraph(index).style().list.is_some();
        if self.selection.is_empty() {
            in_list && local == 0
        } else {
            in_list
                && self
                    .document
                    .paragraph_indices(self.selection.clone())
                    .len()
                    > 1
        }
    }

    fn tab(&mut self, _: &input::IndentInline, _: &mut Window, cx: &mut Context<Self>) {
        if !self.editable {
            return;
        }
        if self.at_list_item_start() {
            self.change_list_level(1, cx);
            return;
        }
        self.insert(self.selection.clone(), "\t", EditKind::Typing, cx);
    }

    fn copy(&mut self, _: &input::Copy, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_selection(cx);
    }

    fn copy_selection(&mut self, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            return;
        }
        let text = self.document.slice(self.selection.clone());
        let fragment = self.document.fragment(self.selection.clone());
        // Other apps paste the RTF or HTML flavour; rmac pastes the exact
        // fragment kept here.
        let rtf = String::from_utf8(super::rtf::write(&fragment)).unwrap_or_default();
        let html = super::html::write(&fragment);
        let metadata = super::clipboard::encode_formats(&[
            (super::clipboard::RTF_MIME, rtf.as_str()),
            (super::clipboard::HTML_MIME, html.as_str()),
        ]);
        STYLED_CLIPBOARD.with(|clipboard| {
            *clipboard.borrow_mut() = Some((text.clone(), fragment));
        });
        cx.write_to_clipboard(ClipboardItem::new_string_with_metadata(text, metadata));
    }

    fn cut(&mut self, _: &input::Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.editable || self.selection.is_empty() {
            return;
        }
        self.copy_selection(cx);
        self.delete_range(self.selection.clone(), cx);
    }

    fn paste(&mut self, _: &input::Paste, _: &mut Window, cx: &mut Context<Self>) {
        if !self.editable {
            return;
        }
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        let Some(text) = item.text() else {
            return;
        };
        let styled = STYLED_CLIPBOARD
            .with(|clipboard| {
                clipboard
                    .borrow()
                    .as_ref()
                    .filter(|(copied, _)| *copied == text)
                    .map(|(_, fragment)| fragment.clone())
            })
            .or_else(|| {
                // Another app's rich copy: its RTF flavour. Most writers
                // end the last paragraph with `\par`, which the plain text
                // does not have.
                let rtf = super::clipboard::format(item.metadata()?, super::clipboard::RTF_MIME)?;
                let document = super::rtf::parse(rtf.as_bytes())?;
                let len = document.len();
                let document = if document.text().ends_with('\n') && !text.ends_with('\n') {
                    document.fragment(0..len - 1)
                } else {
                    document
                };
                (!document.is_empty()).then_some(document)
            });
        match styled {
            Some(fragment) => {
                let range = self.selection.clone();
                self.begin_edit(EditKind::Other);
                let inserted = self.document.replace_fragment(range, &fragment, true);
                self.typing_style = None;
                self.selection = inserted.end..inserted.end;
                self.reversed = false;
                self.finish_edit(cx);
            }
            None => {
                self.insert(self.selection.clone(), &text, EditKind::Other, cx);
            }
        }
    }

    fn undo_action(&mut self, _: &input::Undo, _: &mut Window, cx: &mut Context<Self>) {
        self.undo(cx);
    }

    fn redo_action(&mut self, _: &input::Redo, _: &mut Window, cx: &mut Context<Self>) {
        self.redo(cx);
    }

    /// Edit ▸ Undo.
    pub fn undo(&mut self, cx: &mut Context<Self>) {
        if !self.editable {
            return;
        }
        if let Some(entry) = self.undo.pop() {
            let current = self.snapshot();
            self.redo.push(current);
            self.restore(entry, cx);
        }
    }

    /// Edit ▸ Redo.
    pub fn redo(&mut self, cx: &mut Context<Self>) {
        if !self.editable {
            return;
        }
        if let Some(entry) = self.redo.pop() {
            let current = self.snapshot();
            self.undo.push(current);
            self.restore(entry, cx);
        }
    }

    fn restore(&mut self, entry: UndoEntry, cx: &mut Context<Self>) {
        self.document = entry.document;
        self.selection = self.document.clamp_range(entry.selection);
        self.reversed = entry.reversed;
        self.marked = None;
        self.typing_style = None;
        self.last_edit = None;
        self.finish_edit(cx);
    }

    fn show_character_palette(
        &mut self,
        _: &input::ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    // ---- Pointer ---------------------------------------------------------

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        let offset = self.offset_at(event.position);
        self.goal_x = None;
        // A plain click on linked text opens the link on release, as in
        // TextEdit (web and mail addresses only).
        self.pending_link = None;
        if event.click_count <= 1 && !event.modifiers.shift {
            self.pending_link = self
                .char_at(event.position)
                .and_then(|at| self.document.style_of_char_at(at).link)
                .filter(|link| is_openable_link(link));
        }
        let (granularity, range) = match event.click_count {
            0 | 1 => (Granularity::Character, offset..offset),
            2 => (Granularity::Word, self.document.word_range_at(offset)),
            _ => {
                let index = self.document.locate(offset).0;
                (Granularity::Paragraph, self.document.paragraph_range(index))
            }
        };
        if event.modifiers.shift && granularity == Granularity::Character {
            self.select_to(offset, cx);
            let anchor = self.anchor();
            self.dragging = Some((Granularity::Character, anchor..anchor));
        } else {
            self.set_selection(range.clone(), false, cx);
            self.dragging = Some((granularity, range));
        }
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some((granularity, origin)) = self.dragging.clone() else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.dragging = None;
            return;
        }
        // Dragging past the top or bottom scrolls, a step per move.
        if event.position.y < self.viewport.top() {
            self.scroll.set_top(self.scroll.top() - px(16.0));
        } else if event.position.y > self.viewport.bottom() {
            self.scroll.set_top(self.scroll.top() + px(16.0));
        }
        let offset = self.offset_at(event.position);
        let target = match granularity {
            Granularity::Character => offset..offset,
            Granularity::Word => self.document.word_range_at(offset),
            Granularity::Paragraph => {
                let index = self.document.locate(offset).0;
                self.document.paragraph_range(index)
            }
        };
        let (range, reversed) = if target.start < origin.start {
            (target.start..origin.end, true)
        } else {
            (origin.start..target.end.max(origin.end), false)
        };
        let changed = self.selection != range;
        self.selection = range;
        self.reversed = reversed && !self.selection.is_empty();
        if changed {
            self.pending_link = None;
            self.typing_style = None;
            self.last_edit = None;
            cx.notify();
        }
    }

    /// ⇧←/⇧→/⇧↑/⇧↓: gpui-component binds these in the "Input" context to
    /// actions it keeps private, so they reach the editor unhandled as key
    /// presses (GPUI delivers a key to listeners when no binding's action
    /// was handled).
    fn on_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = &event.keystroke.modifiers;
        if !modifiers.shift
            || modifiers.control
            || modifiers.alt
            || modifiers.platform
            || modifiers.function
        {
            return;
        }
        match event.keystroke.key.as_str() {
            "left" => self.select_left(cx),
            "right" => self.select_right(cx),
            "up" => self.select_up(window, cx),
            "down" => self.select_down(window, cx),
            // ⇧Tab at a list item's start brings it out a level.
            "tab" if self.editable && self.at_list_item_start() => self.change_list_level(-1, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.dragging = None;
        if let Some(link) = self.pending_link.take() {
            cx.open_url(&link);
        }
    }

    fn on_mouse_up_out(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.dragging = None;
        self.pending_link = None;
    }

    /// The character under a window position, if any.
    fn char_at(&self, position: Point<Pixels>) -> Option<usize> {
        let target = self.placed.iter().find(|placed| {
            position.y >= placed.origin.y && position.y < placed.origin.y + placed.layout.height
        })?;
        let local = target
            .layout
            .char_at_point(position.x - target.origin.x, position.y - target.origin.y)?;
        Some(self.document.paragraph_start(target.index) + local)
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(px(16.0 * self.zoom));
        if delta.y != px(0.0) {
            self.scroll.set_top(self.scroll.top() - delta.y);
            cx.stop_propagation();
            cx.notify();
        }
    }
}

impl EntityInputHandler for RichTextEditor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.document.range_from_utf16(&range_utf16);
        adjusted_range.replace(self.document.range_to_utf16(&range));
        Some(self.document.slice(range))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.document.range_to_utf16(&self.selection),
            reversed: self.reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked
            .as_ref()
            .map(|range| self.document.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked = None;
        self.last_edit = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.editable {
            return;
        }
        let range = range_utf16
            .map(|range| self.document.range_from_utf16(&range))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection.clone());
        let kind = if self.marked.take().is_some() {
            EditKind::Composing
        } else {
            EditKind::Typing
        };
        self.insert(range, text, kind, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.editable {
            return;
        }
        let range = range_utf16
            .map(|range| self.document.range_from_utf16(&range))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection.clone());
        let inserted = self.insert(range, new_text, EditKind::Composing, cx);
        self.marked = (!inserted.is_empty()).then_some(inserted.clone());
        if let Some(selected) = new_selected_range_utf16 {
            let start = inserted.start + utf8_offset_in(new_text, selected.start);
            let end = inserted.start + utf8_offset_in(new_text, selected.end);
            self.selection = start.min(end)..start.max(end);
            self.reversed = false;
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let start = self.document.offset_from_utf16(range_utf16.start);
        let (index, local) = self.document.locate(start);
        let placed = self.placed.iter().find(|placed| placed.index == index)?;
        let (x, top, height) = placed.layout.caret(local);
        Some(Bounds::new(
            point(placed.origin.x + x, placed.origin.y + top),
            size(px(1.0), height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let offset = self.offset_at(point);
        Some(self.document.offset_to_utf16(offset))
    }

    fn accepts_text_input(&self, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
        self.editable
    }
}

/// UTF-16 offset inside `text` → UTF-8 offset.
/// Links a click opens: web and mail addresses. Anything else in a
/// document (a local file, a custom scheme) stays inert.
fn is_openable_link(link: &str) -> bool {
    let lower = link.trim_start().to_ascii_lowercase();
    ["http://", "https://", "mailto:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
}

fn utf8_offset_in(text: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (index, character) in text.char_indices() {
        if units >= utf16 {
            return index;
        }
        units += character.len_utf16();
    }
    text.len()
}

impl rmac_ui::EditableText for RichTextEditor {
    fn editable_text(&self) -> String {
        self.document.text()
    }

    fn editable_selection(&self) -> Range<usize> {
        self.selection.clone()
    }

    fn select_editable_range(
        &mut self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_range(range, cx);
    }

    fn replace_editable_range(
        &mut self,
        range: Range<usize>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_range(range, text, cx);
    }

    fn focus_editable(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }
}

impl Render for RichTextEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = gpui::div()
            .id("rich-text-editor")
            .role(Role::MultilineTextInput)
            .key_context(input::KEY_CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::move_left))
            .on_action(cx.listener(Self::move_right))
            .on_action(cx.listener(Self::move_up))
            .on_action(cx.listener(Self::move_down))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::move_home))
            .on_action(cx.listener(Self::move_end))
            .on_action(cx.listener(Self::move_line_start))
            .on_action(cx.listener(Self::move_line_end))
            .on_action(cx.listener(Self::move_to_start))
            .on_action(cx.listener(Self::move_to_end))
            .on_action(cx.listener(Self::move_previous_word))
            .on_action(cx.listener(Self::move_next_word))
            .on_action(cx.listener(Self::select_line_start))
            .on_action(cx.listener(Self::select_line_end))
            .on_action(cx.listener(Self::select_to_start))
            .on_action(cx.listener(Self::select_to_end))
            .on_action(cx.listener(Self::select_previous_word))
            .on_action(cx.listener(Self::select_next_word))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_to_line_start))
            .on_action(cx.listener(Self::delete_to_line_end))
            .on_action(cx.listener(Self::delete_previous_word))
            .on_action(cx.listener(Self::delete_next_word))
            .on_action(cx.listener(Self::enter))
            .on_action(cx.listener(Self::tab))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::undo_action))
            .on_action(cx.listener(Self::redo_action))
            .on_action(cx.listener(Self::show_character_palette))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up_out))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .on_key_down(cx.listener(Self::on_key_down))
            // Painted inside this (focused) node, so the shift-arrow
            // listeners it registers by name are on the dispatch path.
            .child(RichTextElement {
                editor: cx.entity(),
            });
        rmac_ui::overlay_scrollbar(root, &self.scroll)
    }
}

/// Paints the visible paragraphs, selection and caret, and registers the
/// editor as the window's text input while focused.
struct RichTextElement {
    editor: Entity<RichTextEditor>,
}

impl IntoElement for RichTextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Element for RichTextElement {
    type RequestLayoutState = ();
    type PrepaintState = Option<Frame>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = gpui::relative(1.).into();
        style.size.height = gpui::relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        Some(
            self.editor
                .update(cx, |editor, _cx| editor.prepare_frame(bounds, window)),
        )
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        frame: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(frame) = frame.take() else {
            return;
        };
        let (focus, selection_color, caret_color, focused) = {
            let editor = self.editor.read(cx);
            (
                editor.focus.clone(),
                editor.selection_color,
                editor.caret_color,
                editor.focus.is_focused(window),
            )
        };
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );
        let selection_color = if focused {
            selection_color
        } else {
            selection_color.opacity(0.5)
        };
        let mask = gpui::ContentMask {
            bounds: frame.bounds,
        };
        window.with_content_mask(Some(mask), |window| {
            for placed in &frame.placed {
                for line in &placed.layout.lines {
                    for piece in &line.pieces {
                        let origin = point(
                            placed.origin.x + piece.x,
                            placed.origin.y + line.top + line.ascent
                                - piece.line.ascent
                                - piece.rise,
                        );
                        let line_height = piece.line.ascent + piece.line.descent;
                        let _ = piece.line.paint_background(
                            origin,
                            line_height,
                            gpui::TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    }
                }
            }
            for quad in &frame.selection {
                window.paint_quad(gpui::fill(*quad, selection_color));
            }
            for placed in &frame.placed {
                if let (Some(marker), Some(first)) =
                    (&placed.layout.marker, placed.layout.lines.first())
                {
                    let origin = point(
                        placed.origin.x + marker.x,
                        placed.origin.y + first.top + first.ascent - marker.line.ascent,
                    );
                    let line_height = marker.line.ascent + marker.line.descent;
                    let _ = marker.line.paint(
                        origin,
                        line_height,
                        gpui::TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
                for line in &placed.layout.lines {
                    for piece in &line.pieces {
                        let origin = point(
                            placed.origin.x + piece.x,
                            placed.origin.y + line.top + line.ascent
                                - piece.line.ascent
                                - piece.rise,
                        );
                        let line_height = piece.line.ascent + piece.line.descent;
                        let Some(fill) = &piece.fill else {
                            let _ = piece.line.paint(
                                origin,
                                line_height,
                                gpui::TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                            continue;
                        };
                        // Outline: the glyphs in the text colour, nudged
                        // around their place, then filled with the paper.
                        let stroke = px(0.75);
                        for (dx, dy) in [
                            (stroke, px(0.0)),
                            (-stroke, px(0.0)),
                            (px(0.0), stroke),
                            (px(0.0), -stroke),
                        ] {
                            let _ = piece.line.paint(
                                point(origin.x + dx, origin.y + dy),
                                line_height,
                                gpui::TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        }
                        let _ = fill.paint(
                            origin,
                            line_height,
                            gpui::TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    }
                }
            }
            for quad in &frame.marked {
                window.paint_quad(gpui::fill(*quad, caret_color));
            }
            if let Some(caret) = frame.caret {
                window.paint_quad(gpui::fill(caret, caret_color));
            }
        });
    }
}
