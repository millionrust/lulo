//! The Emoji & Symbols panel: a search field, the emoji in a scrolling grid
//! under Frequently Used and the Mac's categories, a category bar along the
//! bottom, and the name of the emoji under the pointer.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{
    div, prelude::FluentBuilder as _, px, uniform_list, AnyElement, App, AppContext as _, Context, Entity, Focusable as _,
    FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, ParentElement as _,
    Render, Role, ScrollStrategy, SharedString, StatefulInteractiveElement as _, Styled as _,
    UniformListScrollHandle, Window,
};
use rmac_emoji::{Category, Entry, Recents};
use rmac_ui::{mac, InputEvent, InputState, SearchField};

pub const WIDTH: f32 = 344.0;
pub const HEIGHT: f32 = 392.0;
const COLUMNS: usize = 8;
const CELL: f32 = 38.0;
const ROW_HEIGHT: f32 = 40.0;
const SEARCH_RESULTS: usize = 400;

/// What the panel hands back to `main` when it closes.
#[derive(Default)]
pub struct Outcome {
    pub glyph: Option<String>,
}

enum Row {
    Header(SharedString),
    Message(SharedString),
    /// Emoji with their index in the flat selection order.
    Cells(Vec<(usize, &'static Entry)>),
}

/// A bar button: Frequently Used (`None`) or a category, and the first row
/// of its section.
#[derive(Clone, Copy)]
struct Section {
    category: Option<Category>,
    first_row: usize,
}

struct Layout {
    rows: Vec<Row>,
    sections: Vec<Section>,
    flat: Vec<&'static Entry>,
    /// Row and column of each flat index.
    place: Vec<(usize, usize)>,
}

pub struct EmojiView {
    search: Entity<InputState>,
    recents: Recents,
    recents_path: Option<PathBuf>,
    layout: Rc<Layout>,
    selected: usize,
    scroll: UniformListScrollHandle,
    hovered: Option<&'static Entry>,
    /// The skin-tone strip shown for this emoji (secondary click).
    tones: Option<&'static Entry>,
    outcome: Rc<RefCell<Outcome>>,
    was_active: bool,
}

impl EmojiView {
    pub fn new(outcome: Rc<RefCell<Outcome>>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.rebuild(cx);
                this.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
            }
        })
        .detach();
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.was_active = true;
            } else if this.was_active {
                window.remove_window();
                cx.notify();
            }
        })
        .detach();
        search.read(cx).focus_handle(cx).focus(window, cx);
        let recents_path = Recents::default_path();
        let recents = recents_path
            .as_deref()
            .map(Recents::load)
            .unwrap_or_default();
        let layout = Rc::new(build_layout("", &recents));
        Self {
            search,
            recents,
            recents_path,
            layout,
            selected: 0,
            scroll: UniformListScrollHandle::new(),
            hovered: None,
            tones: None,
            outcome,
            was_active: false,
        }
    }

    fn query(&self, cx: &App) -> String {
        self.search.read(cx).value().to_string()
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        self.layout = Rc::new(build_layout(&self.query(cx), &self.recents));
        self.selected = 0;
        self.tones = None;
        cx.notify();
    }

    /// Insert `glyph`: count it in Frequently Used, remember it for `main`
    /// and close the panel.
    fn pick(&mut self, glyph: &str, window: &mut Window) {
        self.recents.record(glyph);
        if let Some(path) = &self.recents_path {
            if let Err(error) = self.recents.save(path) {
                eprintln!("rmac-emoji: could not save Frequently Used: {error}");
            }
        }
        self.outcome.borrow_mut().glyph = Some(glyph.to_owned());
        window.remove_window();
    }

    fn move_selection(&mut self, rows: isize, columns: isize, cx: &mut Context<Self>) {
        let layout = self.layout.clone();
        if layout.flat.is_empty() {
            return;
        }
        let target = if columns != 0 {
            let next = self.selected as isize + columns;
            next.clamp(0, layout.flat.len() as isize - 1) as usize
        } else {
            let (row, column) = layout.place[self.selected];
            let mut row = row as isize;
            loop {
                row += rows.signum();
                if row < 0 || row as usize >= layout.rows.len() {
                    return;
                }
                if let Row::Cells(cells) = &layout.rows[row as usize] {
                    let column = column.min(cells.len() - 1);
                    break cells[column].0;
                }
            }
        };
        self.selected = target;
        self.tones = None;
        self.hovered = layout.flat.get(target).copied();
        if let Some(&(row, _)) = layout.place.get(target) {
            self.scroll.scroll_to_item(row, ScrollStrategy::Nearest);
        }
        cx.notify();
    }

    fn jump_to(&mut self, section: Section, cx: &mut Context<Self>) {
        self.scroll
            .scroll_to_item_strict(section.first_row, ScrollStrategy::Top);
        cx.notify();
    }

    /// The bar button whose section holds the top row of the list.
    fn current_section(&self) -> usize {
        let offset = self.scroll.0.borrow().base_handle.offset().y;
        let top = ((-f32::from(offset)) / ROW_HEIGHT).max(0.0) as usize;
        self.layout
            .sections
            .iter()
            .rposition(|section| section.first_row <= top)
            .unwrap_or(0)
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.keystroke.modifiers;
        if modifiers.platform || modifiers.control || modifiers.alt {
            return;
        }
        let handled = match event.keystroke.key.as_str() {
            "escape" => {
                if self.tones.take().is_none() {
                    window.remove_window();
                }
                true
            }
            "enter" => {
                if let Some(entry) = self.layout.flat.get(self.selected).copied() {
                    self.pick(entry.glyph, window);
                }
                true
            }
            "left" if self.query(cx).is_empty() => {
                self.move_selection(0, -1, cx);
                true
            }
            "right" if self.query(cx).is_empty() => {
                self.move_selection(0, 1, cx);
                true
            }
            "down" => {
                self.move_selection(1, 0, cx);
                true
            }
            "up" => {
                self.move_selection(-1, 0, cx);
                true
            }
            _ => false,
        };
        if handled {
            cx.stop_propagation();
        }
    }
}

fn build_layout(query: &str, recents: &Recents) -> Layout {
    let mut rows = Vec::new();
    let mut sections = Vec::new();
    let mut flat: Vec<&'static Entry> = Vec::new();
    let mut place = Vec::new();

    let mut push_cells = |entries: Vec<&'static Entry>, rows: &mut Vec<Row>| {
        for chunk in entries.chunks(COLUMNS) {
            let row = rows.len();
            let cells = chunk
                .iter()
                .enumerate()
                .map(|(column, entry)| {
                    let index = flat.len();
                    flat.push(entry);
                    place.push((row, column));
                    (index, *entry)
                })
                .collect();
            rows.push(Row::Cells(cells));
        }
    };

    if query.trim().is_empty() {
        let used: Vec<&'static Entry> = recents
            .glyphs()
            .filter_map(rmac_emoji::find)
            .collect();
        if !used.is_empty() {
            sections.push(Section {
                category: None,
                first_row: rows.len(),
            });
            rows.push(Row::Header("Frequently Used".into()));
            push_cells(used, &mut rows);
        }
        for category in Category::ALL {
            sections.push(Section {
                category: Some(category),
                first_row: rows.len(),
            });
            rows.push(Row::Header(category.label().into()));
            push_cells(rmac_emoji::in_category(category).collect(), &mut rows);
        }
    } else {
        let hits = rmac_emoji::search(query, SEARCH_RESULTS);
        if hits.is_empty() {
            rows.push(Row::Message("No Results".into()));
        } else {
            rows.push(Row::Header("Search Results".into()));
            push_cells(hits, &mut rows);
        }
    }
    Layout {
        rows,
        sections,
        flat,
        place,
    }
}

fn bar_glyph(category: Option<Category>) -> &'static str {
    match category {
        None => "🕘",
        Some(Category::People) => "😀",
        Some(Category::Nature) => "🐻",
        Some(Category::Food) => "🍔",
        Some(Category::Activity) => "⚽",
        Some(Category::Travel) => "🚗",
        Some(Category::Objects) => "💡",
        Some(Category::Symbols) => "🔣",
        Some(Category::Flags) => "🏁",
    }
}

fn bar_label(category: Option<Category>) -> &'static str {
    category.map_or("Frequently Used", Category::label)
}

impl Render for EmojiView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let layout = self.layout.clone();
        let selected = self.selected;
        let searching = !self.query(cx).trim().is_empty();

        let rows_view = view.clone();
        let rows_layout = layout.clone();
        let list = uniform_list("emoji-grid", layout.rows.len(), move |range, _window, _cx| {
            range
                .map(|index| render_row(&rows_layout.rows[index], selected, &rows_view))
                .collect::<Vec<_>>()
        })
        .track_scroll(&self.scroll)
        .flex_1()
        .min_h(px(0.0));

        let current = self.current_section();
        let bar = (!searching).then(|| {
            div()
                .flex_none()
                .h(px(36.0))
                .px(px(8.0))
                .flex()
                .items_center()
                .justify_between()
                .border_t_1()
                .border_color(mac::separator())
                .children(layout.sections.iter().enumerate().map(|(index, section)| {
                    let section = *section;
                    let view = view.clone();
                    div()
                        .id(SharedString::from(format!("emoji-section-{index}")))
                        .role(Role::Button)
                        .aria_label(bar_label(section.category))
                        .w(px(32.0))
                        .h(px(28.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.0))
                        .text_size(px(16.0))
                        .when(index == current, |button| button.bg(mac::control_fill()))
                        .hover(|button| button.bg(mac::hover()))
                        .on_click(move |_, _, cx| {
                            view.update(cx, |this, cx| this.jump_to(section, cx));
                        })
                        .child(bar_glyph(section.category))
                }))
        });

        let caption: AnyElement = match self.tones {
            Some(entry) => div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .children(
                    std::iter::once(entry.glyph)
                        .chain(entry.variants())
                        .enumerate()
                        .map(|(index, glyph)| {
                            let view = view.clone();
                            div()
                                .id(SharedString::from(format!("emoji-tone-{index}")))
                                .role(Role::Button)
                                .aria_label(format!("{} skin tone {index}", entry.name))
                                .w(px(30.0))
                                .h(px(26.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(6.0))
                                .text_size(px(20.0))
                                .hover(|button| button.bg(mac::hover()))
                                .on_click(move |_, window, cx| {
                                    view.update(cx, |this, _| this.pick(glyph, window));
                                })
                                .child(glyph)
                        }),
                )
                .into_any_element(),
            None => div()
                .text_size(px(12.0))
                .text_color(mac::text_secondary())
                .child(
                    self.hovered
                        .or_else(|| layout.flat.get(selected).copied())
                        .map(|entry| entry.name)
                        .unwrap_or(""),
                )
                .into_any_element(),
        };

        div()
            .size_full()
            .rounded(px(mac::radius_popover()))
            .overflow_hidden()
            .bg(mac::material_popover())
            .border_1()
            .border_color(mac::separator())
            .flex()
            .flex_col()
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key(event, window, cx);
            }))
            .child(
                div()
                    .flex_none()
                    .p(px(10.0))
                    .child(SearchField::new(&self.search).small().w_full()),
            )
            .child(
                div()
                    .id("emoji-list")
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .flex_col()
                    .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
                    .child(list),
            )
            .child(
                div()
                    .flex_none()
                    .h(px(28.0))
                    .px(px(14.0))
                    .flex()
                    .items_center()
                    .child(caption),
            )
            .children(bar)
    }
}

fn render_row(row: &Row, selected: usize, view: &Entity<EmojiView>) -> AnyElement {
    match row {
        Row::Header(label) => div()
            .h(px(ROW_HEIGHT))
            .px(px(14.0))
            .pb(px(4.0))
            .flex()
            .items_end()
            .text_size(px(12.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(mac::text_tertiary())
            .child(label.clone())
            .into_any_element(),
        Row::Message(label) => div()
            .h(px(ROW_HEIGHT))
            .px(px(14.0))
            .flex()
            .items_center()
            .text_size(px(13.0))
            .text_color(mac::text_secondary())
            .child(label.clone())
            .into_any_element(),
        Row::Cells(cells) => div()
            .h(px(ROW_HEIGHT))
            .px(px(14.0))
            .flex()
            .items_center()
            .children(cells.iter().map(|&(index, entry)| {
                let pick_view = view.clone();
                let tone_view = view.clone();
                let hover_view = view.clone();
                let glyph = entry.glyph;
                div()
                    .id(SharedString::from(format!("emoji-{index}")))
                    .role(Role::Button)
                    .aria_label(entry.name)
                    .w(px(CELL))
                    .h(px(CELL))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.0))
                    .text_size(px(26.0))
                    .when(index == selected, |cell| cell.bg(mac::accent()))
                    .hover(|cell| cell.bg(mac::hover()))
                    .on_hover(move |hovered, _, cx| {
                        let hovered = *hovered;
                        hover_view.update(cx, |this, cx| {
                            if hovered {
                                this.hovered = Some(entry);
                                cx.notify();
                            } else if this.hovered.is_some_and(|other| other.glyph == glyph) {
                                this.hovered = None;
                                cx.notify();
                            }
                        });
                    })
                    .on_click(move |_, window, cx| {
                        pick_view.update(cx, |this, _| this.pick(glyph, window));
                    })
                    .on_mouse_down(MouseButton::Right, move |_, _, cx| {
                        tone_view.update(cx, |this, cx| {
                            this.tones = entry.has_variants().then_some(entry);
                            cx.notify();
                        });
                    })
                    .child(glyph)
            }))
            .into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_layout_lists_every_category_in_order() {
        let layout = build_layout("", &Recents::default());
        let categories: Vec<_> = layout.sections.iter().map(|s| s.category).collect();
        assert_eq!(categories.len(), Category::ALL.len());
        assert_eq!(categories[0], Some(Category::People));
        assert!(layout
            .sections
            .windows(2)
            .all(|pair| pair[0].first_row < pair[1].first_row));
        assert_eq!(layout.flat.len(), layout.place.len());
    }

    #[test]
    fn frequently_used_comes_first_once_something_was_chosen() {
        let mut recents = Recents::default();
        recents.record("🎉");
        let layout = build_layout("", &recents);
        assert_eq!(layout.sections[0].category, None);
        assert_eq!(layout.flat[0].glyph, "🎉");
    }

    #[test]
    fn a_search_replaces_the_sections_with_results() {
        let layout = build_layout("heart", &Recents::default());
        assert!(layout.sections.is_empty());
        assert!(!layout.flat.is_empty());
        assert!(matches!(layout.rows[0], Row::Header(_)));
        let empty = build_layout("zzzzqqq", &Recents::default());
        assert!(empty.flat.is_empty());
        assert!(matches!(empty.rows[0], Row::Message(_)));
    }

    #[test]
    fn grid_places_match_the_rows() {
        let layout = build_layout("", &Recents::default());
        for (index, &(row, column)) in layout.place.iter().enumerate() {
            let Row::Cells(cells) = &layout.rows[row] else {
                panic!("flat index {index} is not in a cell row");
            };
            assert_eq!(cells[column].0, index);
        }
    }
}
