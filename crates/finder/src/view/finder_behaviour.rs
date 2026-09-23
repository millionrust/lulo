//! Finder behaviours Files previously lacked: type-to-select, marquee
//! selection in icon view, and spring-loaded folders during a drag.

use std::time::Instant;

use super::*;

/// An in-progress icon-view rubber band, in grid-content coordinates so it
/// scrolls with the icons.
pub(super) struct Marquee {
    start: Point<Pixels>,
    current: Point<Pixels>,
    /// Selection held before the drag when ⌘ or ⇧ extends it.
    base: BTreeSet<usize>,
}

#[derive(Default)]
pub(super) struct TypeSelect {
    prefix: String,
    last: Option<Instant>,
}

#[derive(Default)]
pub(super) struct SpringLoading {
    target: Option<PathBuf>,
    generation: u64,
}

/// The printable text a key press contributes to type-to-select, if any.
/// Commands, Space (Quick Look) and control keys never type-select.
pub(super) fn type_select_text(event: &KeyDownEvent) -> Option<String> {
    let modifiers = &event.keystroke.modifiers;
    if modifiers.platform || modifiers.control || modifiers.alt || modifiers.function {
        return None;
    }
    let text = event.keystroke.key_char.as_deref()?;
    (!text.is_empty() && text != " " && !text.chars().any(char::is_control))
        .then(|| text.to_lowercase())
}

/// AppKit's type-select match: the first name (in name order) that is not
/// alphabetically before the typed prefix, or the last name if all are.
pub(super) fn type_select_match<'a>(
    prefix: &str,
    names: impl Iterator<Item = (usize, &'a str)>,
) -> Option<usize> {
    let mut best: Option<(usize, String)> = None;
    let mut last: Option<(usize, String)> = None;
    for (index, name) in names {
        let name = name.to_lowercase();
        if name.as_str() >= prefix && best.as_ref().is_none_or(|(_, b)| name < *b) {
            best = Some((index, name.clone()));
        }
        if last.as_ref().is_none_or(|(_, l)| name > *l) {
            last = Some((index, name));
        }
    }
    best.or(last).map(|(index, _)| index)
}

impl FinderView {
    /// Icon-view cell size: 128 × 116 at the 64 pt default, scaling with the
    /// icon-size slider.
    pub(super) fn icon_cell(&self) -> (f32, f32) {
        (
            self.icon_size + ICON_CELL_EXTRA_WIDTH,
            self.icon_size + ICON_CELL_EXTRA_HEIGHT,
        )
    }

    pub(super) fn icon_columns(&self) -> usize {
        let width = f32::from(self.icon_scroll.bounds().size.width) - ICON_GRID_LEFT;
        let (cell, _) = self.icon_cell();
        if width.is_finite() && width > cell {
            (width / cell).floor() as usize
        } else {
            1
        }
    }

    fn grid_point(&self, position: Point<Pixels>) -> Point<Pixels> {
        let bounds = self.icon_scroll.bounds();
        let offset = self.icon_scroll.offset();
        Point::new(
            position.x - bounds.origin.x - offset.x,
            position.y - bounds.origin.y - offset.y,
        )
    }

    pub(super) fn marquee_rect(&self) -> Option<gpui::Bounds<Pixels>> {
        let marquee = self.marquee.as_ref()?;
        let left = marquee.start.x.min(marquee.current.x);
        let top = marquee.start.y.min(marquee.current.y);
        let right = marquee.start.x.max(marquee.current.x);
        let bottom = marquee.start.y.max(marquee.current.y);
        (right - left > px(2.0) || bottom - top > px(2.0)).then(|| {
            gpui::Bounds::new(
                Point::new(left, top),
                gpui::size(right - left, bottom - top),
            )
        })
    }

    pub(super) fn begin_marquee(&mut self, position: Point<Pixels>, extend: bool) {
        if !extend {
            self.selected.clear();
            self.anchor = None;
        }
        let start = self.grid_point(position);
        self.marquee = Some(Marquee {
            start,
            current: start,
            base: self.selected.clone(),
        });
    }

    pub(super) fn update_marquee(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.marquee.is_none() {
            return;
        }
        let current = self.grid_point(position);
        if let Some(marquee) = self.marquee.as_mut() {
            marquee.current = current;
        }
        let Some(rect) = self.marquee_rect() else {
            cx.notify();
            return;
        };
        let query = if self.search_summary.is_some() {
            String::new()
        } else {
            self.query.read(cx).value().to_lowercase()
        };
        let columns = self.icon_columns();
        let (cell_width, cell_height) = self.icon_cell();
        let icon_size = self.icon_size;
        let mut selected = self
            .marquee
            .as_ref()
            .map(|m| m.base.clone())
            .unwrap_or_default();
        let visible = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| query.is_empty() || entry.name.to_lowercase().contains(&query))
            .map(|(index, _)| index);
        for (position, index) in visible.enumerate() {
            let column = (position % columns) as f32;
            let row = (position / columns) as f32;
            // The icon plus its label, as Finder hit-tests a marquee.
            let item = gpui::Bounds::new(
                Point::new(
                    px(ICON_GRID_LEFT + column * cell_width + (cell_width - icon_size) / 2.0),
                    px(ICON_GRID_TOP + row * cell_height),
                ),
                gpui::size(px(icon_size), px(icon_size + ICON_LABEL_GAP + 17.0)),
            );
            if item.intersects(&rect) {
                selected.insert(index);
            }
        }
        self.anchor = selected.iter().next_back().copied();
        self.selected = selected;
        cx.notify();
    }

    pub(super) fn end_marquee(&mut self, cx: &mut Context<Self>) {
        if self.marquee.take().is_some() {
            cx.notify();
        }
    }

    /// Select the closest name to what has been typed within the last second.
    pub(super) fn type_select(&mut self, text: &str, visible: &[usize], cx: &mut Context<Self>) {
        let now = Instant::now();
        if self
            .type_select
            .last
            .is_none_or(|last| now.duration_since(last) > TYPE_SELECT_TIMEOUT)
        {
            self.type_select.prefix.clear();
        }
        self.type_select.last = Some(now);
        self.type_select.prefix.push_str(text);
        let prefix = self.type_select.prefix.clone();
        let names = visible.iter().filter_map(|&index| {
            self.entries
                .get(index)
                .map(|entry| (index, entry.name.as_ref()))
        });
        if let Some(index) = type_select_match(&prefix, names) {
            self.select_single(index);
            cx.notify();
        }
    }

    /// Spring-loaded folders: hovering a dragged item over a folder for
    /// 0.7 s opens it, so the drop can continue deeper.
    pub(super) fn spring_hover(&mut self, path: PathBuf, inside: bool, cx: &mut Context<Self>) {
        if !inside {
            if self.spring.target.as_ref() == Some(&path) {
                self.spring.target = None;
                self.spring.generation = self.spring.generation.wrapping_add(1);
            }
            return;
        }
        if self.spring.target.as_ref() == Some(&path) {
            return;
        }
        self.spring.target = Some(path.clone());
        self.spring.generation = self.spring.generation.wrapping_add(1);
        let generation = self.spring.generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            cx.background_executor().timer(SPRING_LOADING_DELAY).await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                if this.spring.generation != generation || !cx.has_active_drag() {
                    return;
                }
                this.spring.target = None;
                this.navigate(path, cx);
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::type_select_match;

    #[test]
    fn type_select_picks_the_closest_following_name() {
        let names = [
            (0, "Applications"),
            (1, "Desktop"),
            (2, "Documents"),
            (3, "Music"),
        ];
        assert_eq!(type_select_match("d", names.into_iter()), Some(1));
        assert_eq!(type_select_match("do", names.into_iter()), Some(2));
        assert_eq!(type_select_match("e", names.into_iter()), Some(3));
        assert_eq!(type_select_match("z", names.into_iter()), Some(3));
    }
}
