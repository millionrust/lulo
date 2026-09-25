//! An editable text field for shell surfaces: one paragraph that wraps at a
//! fixed width and grows to its widest line, as Finder's rename field does.
//! Text arrives through GPUI's platform input handler, so dead keys and IME
//! composition work; editing keys follow AppKit (⌥ by word, ⌘ to either end,
//! ⌘A/⌘C/⌘X/⌘V). The editing model is [`crate::text_edit::TextEdit`].
//!
//! The owner listens for [`TextFieldEvent`]s: Return and Tab submit, Escape
//! cancels, and losing focus (to another element or another window) blurs.

use std::ops::Range;

use gpui::{
    accesskit, div, fill, point, prelude::*, px, rgba, size, A11ySubtreeBuilder, App, Bounds,
    ClipboardItem, Context, ElementId, ElementInputHandler, Entity, EntityInputHandler,
    EventEmitter, FocusHandle, Focusable, GlobalElementId, Hsla, InspectorElementId, KeyDownEvent,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point,
    Role, SharedString, Style, Subscription, TextAlign, TextRun, UTF16Selection, UnderlineStyle,
    Window, WrappedLine,
};

use crate::text_edit::{self, Motion, TextEdit};
use crate::tokens;

/// The caret's width (S).
const CARET_WIDTH: f32 = 1.0;

/// How a field looks. Colours are 0xRRGGBBAA.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextFieldStyle {
    pub text_size: f32,
    pub line_height: f32,
    /// Text wraps at this width; the field is as wide as its widest line.
    pub wrap_width: f32,
    pub padding_x: f32,
    pub radius: f32,
    pub background: u32,
    pub text: u32,
    pub selection: u32,
    pub caret: u32,
    pub centered: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextFieldEvent {
    /// Return or Tab.
    Submit,
    /// Escape.
    Cancel,
    /// Focus moved elsewhere or the window was deactivated.
    Blur,
}

pub struct TextField {
    id: SharedString,
    name: SharedString,
    focus: FocusHandle,
    edit: TextEdit,
    style: TextFieldStyle,
    painted: Option<Painted>,
    selecting: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TextFieldEvent> for TextField {}

impl Focusable for TextField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TextField {
    /// A field with `selection` (UTF-8 byte offsets) selected. `name` is
    /// what assistive technology announces for it.
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        text: &str,
        selection: Range<usize>,
        style: TextFieldStyle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        let subscriptions = vec![
            cx.on_blur(&focus, window, |_, _, cx| cx.emit(TextFieldEvent::Blur)),
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() && this.focus.is_focused(window) {
                    cx.emit(TextFieldEvent::Blur);
                }
            }),
        ];
        Self {
            id: id.into(),
            name: name.into(),
            focus,
            edit: TextEdit::new(text_edit::single_paragraph(text), selection),
            style,
            painted: None,
            selecting: false,
            _subscriptions: subscriptions,
        }
    }

    pub fn text(&self) -> &str {
        self.edit.text()
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let modifiers = &keystroke.modifiers;
        let (command, option, shift) = (modifiers.platform, modifiers.alt, modifiers.shift);
        let key = keystroke.key.as_str();
        match key {
            "enter" | "tab" if !command && !option => cx.emit(TextFieldEvent::Submit),
            "escape" => cx.emit(TextFieldEvent::Cancel),
            "left" | "right" | "up" | "down" | "home" | "end" => {
                let forward = matches!(key, "right" | "down" | "end");
                let motion = match (key, forward) {
                    ("up" | "down" | "home" | "end", false) => Motion::Start,
                    ("up" | "down" | "home" | "end", true) => Motion::End,
                    (_, false) if command => Motion::Start,
                    (_, true) if command => Motion::End,
                    (_, false) if option => Motion::WordLeft,
                    (_, true) if option => Motion::WordRight,
                    (_, false) => Motion::Left,
                    (_, true) => Motion::Right,
                };
                self.edit.move_by(motion, shift);
            }
            "backspace" | "delete" => {
                let motion = match (key == "delete", command, option) {
                    (false, true, _) => Motion::Start,
                    (false, _, true) => Motion::WordLeft,
                    (false, _, _) => Motion::Left,
                    (true, true, _) => Motion::End,
                    (true, _, true) => Motion::WordRight,
                    (true, _, _) => Motion::Right,
                };
                if !self.edit.delete(motion) {
                    window.play_system_bell();
                }
            }
            "a" if command => self.edit.select_all(),
            "c" if command => {
                if !self.edit.selection().is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(
                        self.edit.selected_text().to_owned(),
                    ));
                }
            }
            "x" if command => {
                if let Some(taken) = self.edit.cut() {
                    cx.write_to_clipboard(ClipboardItem::new_string(taken));
                }
            }
            "v" if command => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.edit.replace(None, &text_edit::single_paragraph(&text));
                }
            }
            // Everything else, printable keys included, goes on to the
            // platform input handler.
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        window.focus(&self.focus, cx);
        let index = self.index_for_point(event.position);
        match event.click_count {
            2 => self.edit.select_word_at(index),
            count if count >= 3 => self.edit.select_all(),
            _ if event.modifiers.shift => self.edit.select_to(index),
            _ => self.edit.set_cursor(index),
        }
        self.selecting = event.click_count < 2;
        cx.notify();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting && event.pressed_button == Some(MouseButton::Left) {
            let index = self.index_for_point(event.position);
            self.edit.select_to(index);
            cx.notify();
        }
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }

    fn index_for_point(&self, position: Point<Pixels>) -> usize {
        match &self.painted {
            Some(painted) => painted.shaped.index_for_point(painted.bounds, position),
            None => self.edit.head(),
        }
    }

    fn shape(&self, window: &Window) -> Shaped {
        let text = SharedString::from(self.edit.text().to_owned());
        let color: Hsla = rgba(self.style.text).into();
        let run = TextRun {
            len: text.len(),
            font: window.text_style().font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = match self.edit.marked() {
            Some(marked) => vec![
                TextRun {
                    len: marked.start,
                    ..run.clone()
                },
                TextRun {
                    len: marked.end - marked.start,
                    underline: Some(UnderlineStyle {
                        color: Some(color),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: text.len() - marked.end,
                    ..run
                },
            ],
            None => vec![run],
        };
        let line = window
            .text_system()
            .shape_text(
                text,
                px(self.style.text_size),
                &runs,
                Some(px(self.style.wrap_width)),
                None,
            )
            .ok()
            .and_then(|lines| lines.into_iter().next())
            .unwrap_or_default();
        Shaped::new(line, self.style.line_height, self.style.centered)
    }
}

/// Character offset of byte offset `byte` in `text` (rounded down to a
/// character boundary), for translating [`TextEdit`]'s byte-offset
/// selection into the character offsets AccessKit's text positions use.
fn char_offset(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    text[..byte].chars().count()
}

/// Publish `text` as one AccessKit text run under this node (the field
/// never holds a newline: Return submits instead of inserting one), with
/// `anchor`/`focus` as character offsets into it. Mirrors
/// `rmac_ui::accessibility::AccessibleTextInput` for gpui-component's
/// `InputState` fields, which this crate cannot depend on (ADR 0015 keeps
/// shell surfaces off the app-level `gpui-component` dependency graph).
fn accessible_text_children(
    text: String,
    anchor: usize,
    focus: usize,
) -> impl FnOnce(&mut A11ySubtreeBuilder) + 'static {
    move |builder| {
        let id = builder.synthetic_node_id(("text-run", 0usize));
        let mut node = accesskit::Node::new(accesskit::Role::TextRun);
        node.set_value(text.clone());
        node.set_character_lengths(text.chars().map(|c| c.len_utf8() as u8).collect::<Vec<_>>());
        let mut word_starts = Vec::new();
        let mut previous_space = true;
        for (index, character) in text.chars().enumerate().take(256) {
            let space = character.is_whitespace();
            if index == 0 || (previous_space && !space) {
                word_starts.push(index as u8);
            }
            previous_space = space;
        }
        node.set_word_starts(word_starts);
        builder.push_child(id, node);
        let position = |character_index: usize| accesskit::TextPosition {
            node: id,
            character_index,
        };
        builder
            .parent_node()
            .set_text_selection(accesskit::TextSelection {
                anchor: position(anchor),
                focus: position(focus),
            });
    }
}

impl Render for TextField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let style = self.style;
        let is_focused = self.focus.is_focused(window);
        let text = self.edit.text().to_owned();
        let selection = self.edit.selection();
        let head = self.edit.head();
        let (anchor_byte, focus_byte) = if head == selection.start {
            (selection.end, selection.start)
        } else {
            (selection.start, selection.end)
        };
        let (anchor_char, focus_char) = (
            char_offset(&text, anchor_byte),
            char_offset(&text, focus_byte),
        );
        div()
            .id(ElementId::Name(self.id.clone()))
            .role(Role::TextInput)
            .aria_label(self.name.clone())
            .aria_value(SharedString::from(text.clone()))
            .a11y_synthetic_children(accessible_text_children(text, anchor_char, focus_char))
            .track_focus(&self.focus)
            .cursor_text()
            .px(px(style.padding_x))
            .rounded(px(style.radius))
            .bg(rgba(style.background))
            // A visible keyboard-focus ring, matching rmac-ui's
            // `mac::focus_ring_shadow()` (previously `track_focus` made this
            // field a tab stop with no visible indicator of that at all).
            .when(is_focused, |el| {
                el.shadow(vec![gpui::BoxShadow::new(
                    px(0.0),
                    px(0.0),
                    rgba(tokens::focus_ring()).into(),
                )
                .spread_radius(px(tokens::focus_ring_width()))])
            })
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .child(FieldText { field: cx.entity() })
    }
}

impl EntityInputHandler for TextField {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.edit.range_from_utf16(&range_utf16);
        actual_range.replace(self.edit.range_to_utf16(&range));
        self.edit.text().get(range).map(str::to_owned)
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.edit.range_to_utf16(&self.edit.selection()),
            reversed: self.edit.is_reversed(),
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.edit
            .marked()
            .map(|range| self.edit.range_to_utf16(&range))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.edit.unmark();
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16.map(|range| self.edit.range_from_utf16(&range));
        self.edit
            .replace(range, &text_edit::single_paragraph(new_text));
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16.map(|range| self.edit.range_from_utf16(&range));
        let selected = new_selected_range_utf16.map(|range| {
            text_edit::utf16_to_utf8(new_text, range.start)
                ..text_edit::utf16_to_utf8(new_text, range.end)
        });
        self.edit.replace_and_mark(range, new_text, selected);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let painted = self.painted.as_ref()?;
        let range = self.edit.range_from_utf16(&range_utf16);
        let start = painted.shaped.position(element_bounds, range.start);
        let end = painted.shaped.position(element_bounds, range.end);
        let width = if end.y == start.y && end.x > start.x {
            end.x - start.x
        } else {
            px(CARET_WIDTH)
        };
        Some(Bounds::new(
            start,
            size(width, px(painted.shaped.line_height)),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let painted = self.painted.as_ref()?;
        let index = painted.shaped.index_for_point(painted.bounds, point);
        Some(self.edit.offset_to_utf16(index))
    }
}

struct Painted {
    bounds: Bounds<Pixels>,
    shaped: Shaped,
}

/// One visual line of the wrapped paragraph: byte range and x extent in
/// the unwrapped layout.
#[derive(Clone, Copy, Debug)]
struct Span {
    start: usize,
    end: usize,
    start_x: f32,
    end_x: f32,
}

impl Span {
    fn width(&self) -> f32 {
        self.end_x - self.start_x
    }
}

struct Shaped {
    line: WrappedLine,
    spans: Vec<Span>,
    width: f32,
    line_height: f32,
    centered: bool,
}

impl Shaped {
    fn new(line: WrappedLine, line_height: f32, centered: bool) -> Self {
        let layout = &line.unwrapped_layout;
        let mut spans = Vec::new();
        let (mut start, mut start_x) = (0, 0.0);
        for boundary in line.wrap_boundaries() {
            let Some(glyph) = layout
                .runs
                .get(boundary.run_ix)
                .and_then(|run| run.glyphs.get(boundary.glyph_ix))
            else {
                continue;
            };
            let x = f32::from(glyph.position.x);
            spans.push(Span {
                start,
                end: glyph.index,
                start_x,
                end_x: x,
            });
            (start, start_x) = (glyph.index, x);
        }
        spans.push(Span {
            start,
            end: layout.len,
            start_x,
            end_x: f32::from(layout.width),
        });
        let width = spans.iter().map(Span::width).fold(0.0, f32::max);
        Self {
            line,
            spans,
            width,
            line_height,
            centered,
        }
    }

    fn height(&self) -> f32 {
        self.line_height * self.spans.len() as f32
    }

    /// The line's left edge inside the field, matching how GPUI aligns
    /// wrapped lines when it paints them.
    fn indent(&self, span: &Span, field_width: f32) -> f32 {
        if self.centered {
            (field_width - span.width()) / 2.0
        } else {
            0.0
        }
    }

    fn x_in(&self, row: usize, index: usize, field_width: f32) -> f32 {
        let span = &self.spans[row];
        let x = f32::from(self.line.unwrapped_layout.x_for_index(index));
        self.indent(span, field_width) + x - span.start_x
    }

    fn row_of(&self, index: usize) -> usize {
        self.spans
            .iter()
            .rposition(|span| span.start <= index)
            .unwrap_or(0)
    }

    fn position(&self, bounds: Bounds<Pixels>, index: usize) -> Point<Pixels> {
        let row = self.row_of(index);
        let width = f32::from(bounds.size.width);
        point(
            bounds.left() + px(self.x_in(row, index, width)),
            bounds.top() + px(row as f32 * self.line_height),
        )
    }

    fn index_for_point(&self, bounds: Bounds<Pixels>, position: Point<Pixels>) -> usize {
        let y = f32::from(position.y - bounds.top());
        let row = ((y / self.line_height).floor().max(0.0) as usize).min(self.spans.len() - 1);
        let span = &self.spans[row];
        let width = f32::from(bounds.size.width);
        let x = f32::from(position.x - bounds.left()) - self.indent(span, width) + span.start_x;
        self.line
            .unwrapped_layout
            .closest_index_for_x(px(x))
            .clamp(span.start, span.end)
    }

    fn selection_quads(
        &self,
        bounds: Bounds<Pixels>,
        range: &Range<usize>,
        color: Hsla,
    ) -> Vec<PaintQuad> {
        let width = f32::from(bounds.size.width);
        self.spans
            .iter()
            .enumerate()
            .filter_map(|(row, span)| {
                let (start, end) = (range.start.max(span.start), range.end.min(span.end));
                (start < end).then(|| {
                    let top = bounds.top() + px(row as f32 * self.line_height);
                    fill(
                        Bounds::from_corners(
                            point(bounds.left() + px(self.x_in(row, start, width)), top),
                            point(
                                bounds.left() + px(self.x_in(row, end, width)),
                                top + px(self.line_height),
                            ),
                        ),
                        color,
                    )
                })
            })
            .collect()
    }
}

/// The field's text, selection and caret, and its platform input handler.
struct FieldText {
    field: Entity<TextField>,
}

struct FieldPaint {
    selection: Vec<PaintQuad>,
    caret: Option<PaintQuad>,
}

impl IntoElement for FieldText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for FieldText {
    type RequestLayoutState = Option<Shaped>;
    type PrepaintState = FieldPaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let shaped = self.field.read(cx).shape(window);
        let mut style = Style::default();
        style.size.width = px(shaped.width.max(CARET_WIDTH).ceil()).into();
        style.size.height = px(shaped.height()).into();
        (window.request_layout(style, [], cx), Some(shaped))
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        shaped: &mut Self::RequestLayoutState,
        _: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let field = self.field.read(cx);
        let Some(shaped) = shaped.as_ref() else {
            return FieldPaint {
                selection: Vec::new(),
                caret: None,
            };
        };
        let selection = field.edit.selection();
        if selection.is_empty() {
            let caret = shaped.position(bounds, field.edit.head());
            FieldPaint {
                selection: Vec::new(),
                caret: Some(fill(
                    Bounds::new(caret, size(px(CARET_WIDTH), px(shaped.line_height))),
                    rgba(field.style.caret),
                )),
            }
        } else {
            FieldPaint {
                selection: shaped.selection_quads(
                    bounds,
                    &selection,
                    rgba(field.style.selection).into(),
                ),
                caret: None,
            }
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        shaped: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.field.read(cx).focus.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.field.clone()),
            cx,
        );
        for quad in prepaint.selection.drain(..) {
            window.paint_quad(quad);
        }
        let Some(shaped) = shaped.take() else {
            return;
        };
        let align = if shaped.centered {
            TextAlign::Center
        } else {
            TextAlign::Left
        };
        // A failed paint leaves the field blank for a frame; the next
        // render repaints it.
        shaped
            .line
            .paint(
                bounds.origin,
                px(shaped.line_height),
                align,
                Some(bounds),
                window,
                cx,
            )
            .ok();
        if focus.is_focused(window) {
            if let Some(caret) = prepaint.caret.take() {
                window.paint_quad(caret);
            }
        }
        self.field.update(cx, |field, _| {
            field.painted = Some(Painted { bounds, shaped });
        });
    }
}
