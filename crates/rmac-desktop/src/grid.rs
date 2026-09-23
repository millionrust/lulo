//! The desktop icon grid as macOS 26 lays it out: columns filled top to
//! bottom from the top-right corner, measured on the owner's Mac
//! (design-lab/desktop.html). Positions are stored from the right edge so a
//! resolution change keeps icons right-aligned, as Finder does.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Top of the first row's icon box (AX frame y 41 under a 33 pt bar).
pub const ICON_TOP: f32 = 41.0;
/// Gap from the first column's icon box to the screen's right edge.
pub const ICON_RIGHT: f32 = 34.0;
/// Gap from the icon box to the label's text box (cap band 115.5–124).
pub const LABEL_GAP: f32 = 6.0;
/// Label wrap width (S) and line count.
pub const LABEL_MAX_WIDTH: f32 = 100.0;
pub const LABEL_LINES: usize = 2;

/// Finder's desktop "Show View Options" values.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct ViewOptions {
    pub icon_size: f32,
    pub grid_spacing: f32,
    pub text_size: f32,
}

impl Default for ViewOptions {
    /// The owner's Mac: iconSize 64, gridSpacing 54, textSize 12.
    fn default() -> Self {
        Self {
            icon_size: 64.0,
            grid_spacing: 54.0,
            text_size: 12.0,
        }
    }
}

impl ViewOptions {
    pub const ICON_SIZES: std::ops::RangeInclusive<f32> = 16.0..=128.0;
    pub const GRID_SPACINGS: std::ops::RangeInclusive<f32> = 1.0..=100.0;
    pub const TEXT_SIZES: std::ops::RangeInclusive<f32> = 10.0..=16.0;

    pub fn normalized(self) -> Self {
        let clamp = |value: f32, range: std::ops::RangeInclusive<f32>, fallback: f32| {
            if value.is_finite() {
                value.clamp(*range.start(), *range.end())
            } else {
                fallback
            }
        };
        let default = Self::default();
        Self {
            icon_size: clamp(self.icon_size, Self::ICON_SIZES, default.icon_size).round(),
            grid_spacing: clamp(self.grid_spacing, Self::GRID_SPACINGS, default.grid_spacing)
                .round(),
            text_size: clamp(self.text_size, Self::TEXT_SIZES, default.text_size).round(),
        }
    }

    /// Distance between neighbouring icons: 112 at the defaults (measured
    /// row pitch). How Finder scales it with the other settings is S.
    pub fn pitch(&self) -> f32 {
        self.icon_size + self.grid_spacing - 6.0
    }

    /// Label line height for the text size (15 at 12 pt).
    pub fn label_line(&self) -> f32 {
        (self.text_size * 1.25).round()
    }
}

/// A grid cell: column 0 is the rightmost, row 0 the top.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct Slot {
    pub column: u16,
    pub row: u16,
}

/// Where an icon box sits: its right edge's distance from the screen's
/// right edge, and its top.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Placement {
    pub from_right: f32,
    pub top: f32,
}

impl Placement {
    pub fn is_valid(&self) -> bool {
        self.from_right.is_finite() && self.top.is_finite()
    }
}

/// The screen the grid is laid out on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub width: f32,
    pub height: f32,
    /// Height kept clear at the bottom (the Dock's exclusive zone).
    pub bottom_reserved: f32,
    pub options: ViewOptions,
}

impl Grid {
    pub fn new(width: f32, height: f32, bottom_reserved: f32, options: ViewOptions) -> Self {
        Self {
            width: width.max(1.0),
            height: height.max(1.0),
            bottom_reserved: bottom_reserved.max(0.0),
            options: options.normalized(),
        }
    }

    fn cell_height(&self) -> f32 {
        self.options.icon_size + LABEL_GAP + self.options.label_line() * LABEL_LINES as f32
    }

    /// Rows whose icon and two label lines fit above the reserved area.
    pub fn rows(&self) -> u16 {
        let usable = self.height - self.bottom_reserved - ICON_TOP - self.cell_height();
        ((usable / self.options.pitch()).floor().max(0.0) as u16).saturating_add(1)
    }

    /// Columns whose icon box fits on screen.
    pub fn columns(&self) -> u16 {
        let usable = self.width - ICON_RIGHT - self.options.icon_size;
        ((usable / self.options.pitch()).floor().max(0.0) as u16).saturating_add(1)
    }

    pub fn slot_placement(&self, slot: Slot) -> Placement {
        let pitch = self.options.pitch();
        Placement {
            from_right: ICON_RIGHT + f32::from(slot.column) * pitch,
            top: ICON_TOP + f32::from(slot.row) * pitch,
        }
    }

    /// Screen x of the icon box's left edge.
    pub fn left(&self, placement: Placement) -> f32 {
        self.width - placement.from_right - self.options.icon_size
    }

    pub fn placement_at(&self, left: f32, top: f32) -> Placement {
        Placement {
            from_right: self.width - left - self.options.icon_size,
            top,
        }
    }

    /// Keeps an icon box fully on screen, below the menu bar.
    pub fn clamp(&self, placement: Placement) -> Placement {
        let icon = self.options.icon_size;
        let max_right = (self.width - icon).max(0.0);
        let max_top = (self.height - self.bottom_reserved - icon).max(ICON_TOP);
        Placement {
            from_right: if placement.from_right.is_finite() {
                placement.from_right.clamp(0.0, max_right)
            } else {
                ICON_RIGHT
            },
            top: if placement.top.is_finite() {
                placement.top.clamp(ICON_TOP.min(max_top), max_top)
            } else {
                ICON_TOP
            },
        }
    }

    /// The slot whose icon position is nearest, within the grid.
    pub fn nearest_slot(&self, placement: Placement) -> Slot {
        let pitch = self.options.pitch();
        let column = ((placement.from_right - ICON_RIGHT) / pitch).round();
        let row = ((placement.top - ICON_TOP) / pitch).round();
        Slot {
            column: column.clamp(0.0, f32::from(self.columns().saturating_sub(1))) as u16,
            row: row.clamp(0.0, f32::from(self.rows().saturating_sub(1))) as u16,
        }
    }

    /// Slot at a column-major index: down the rightmost column first.
    pub fn slot_at(&self, index: usize) -> Slot {
        let rows = usize::from(self.rows().max(1));
        Slot {
            column: u16::try_from(index / rows).unwrap_or(u16::MAX),
            row: (index % rows) as u16,
        }
    }

    fn slot_occupied(&self, slot: Slot, placements: &[Placement]) -> bool {
        let centre = self.slot_placement(slot);
        let half = self.options.pitch() / 2.0;
        placements.iter().any(|placement| {
            (placement.from_right - centre.from_right).abs() < half
                && (placement.top - centre.top).abs() < half
        })
    }

    /// The first slot, in column order, that no icon covers.
    pub fn first_free(&self, placements: &[Placement]) -> Placement {
        let mut index = 0;
        loop {
            let slot = self.slot_at(index);
            if !self.slot_occupied(slot, placements) || slot.column == u16::MAX {
                return self.slot_placement(slot);
            }
            index += 1;
        }
    }

    /// Consecutive slots for an arranged (sorted) desktop.
    pub fn arrange(&self, count: usize) -> Vec<Placement> {
        (0..count)
            .map(|index| self.slot_placement(self.slot_at(index)))
            .collect()
    }

    /// Unsorted desktop: an item keeps its saved position (pulled back on
    /// screen); a new one takes the first free slot.
    pub fn layout<'a>(
        &self,
        names: impl IntoIterator<Item = &'a str>,
        saved: &BTreeMap<String, Placement>,
    ) -> Vec<Placement> {
        let names = names.into_iter().collect::<Vec<_>>();
        let mut placed = vec![None; names.len()];
        let mut taken = Vec::with_capacity(names.len());
        for (index, name) in names.iter().enumerate() {
            if let Some(placement) = saved.get(*name).filter(|placement| placement.is_valid()) {
                let placement = self.clamp(*placement);
                placed[index] = Some(placement);
                taken.push(placement);
            }
        }
        placed
            .into_iter()
            .map(|placement| {
                placement.unwrap_or_else(|| {
                    let placement = self.first_free(&taken);
                    taken.push(placement);
                    placement
                })
            })
            .collect()
    }

    /// "Clean Up": every icon moves to the nearest free slot. Icons nearest
    /// the top-right claim their slots first, so a tidy desktop is unchanged.
    pub fn clean_up(&self, placements: &[Placement]) -> Vec<Placement> {
        let slots = self.slot_count(placements.len());
        let candidates = (0..slots)
            .map(|index| self.slot_at(index))
            .collect::<Vec<_>>();
        let mut order = (0..placements.len()).collect::<Vec<_>>();
        order.sort_by(|&left, &right| {
            let key = |index: usize| {
                let slot = self.nearest_slot(placements[index]);
                let target = self.slot_placement(slot);
                (
                    slot.column,
                    slot.row,
                    distance(placements[index], target).to_bits(),
                )
            };
            key(left).cmp(&key(right))
        });
        let mut used = vec![false; candidates.len()];
        let mut result = vec![self.slot_placement(Slot { column: 0, row: 0 }); placements.len()];
        for index in order {
            let placement = placements[index];
            let best = candidates
                .iter()
                .enumerate()
                .filter(|(slot_index, _)| !used[*slot_index])
                .min_by(|(_, left), (_, right)| {
                    distance(placement, self.slot_placement(**left))
                        .total_cmp(&distance(placement, self.slot_placement(**right)))
                        .then_with(|| left.cmp(right))
                })
                .map(|(slot_index, slot)| (slot_index, *slot));
            if let Some((slot_index, slot)) = best {
                used[slot_index] = true;
                result[index] = self.slot_placement(slot);
            }
        }
        result
    }

    /// Enough slots for every item, spilling into extra columns.
    fn slot_count(&self, items: usize) -> usize {
        let on_screen = usize::from(self.rows()) * usize::from(self.columns());
        let rows = usize::from(self.rows().max(1));
        on_screen.max(items.div_ceil(rows) * rows)
    }
}

fn distance(left: Placement, right: Placement) -> f32 {
    let x = left.from_right - right.from_right;
    let y = left.top - right.top;
    (x * x + y * y).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mac() -> Grid {
        // The owner's 1470 × 956 screen; the Dock's zone is not reserved on
        // the Mac's desktop grid, which stops above the screen bottom.
        Grid::new(1470.0, 956.0, 0.0, ViewOptions::default())
    }

    #[test]
    fn first_column_matches_the_measured_mac() {
        let grid = mac();
        assert_eq!(grid.options.pitch(), 112.0);
        let tops = (0..6)
            .map(|row| grid.slot_placement(Slot { column: 0, row }).top)
            .collect::<Vec<_>>();
        assert_eq!(tops, [41.0, 153.0, 265.0, 377.0, 489.0, 601.0]);
        let first = grid.slot_placement(Slot { column: 0, row: 0 });
        assert_eq!(grid.left(first), 1372.0);
        assert_eq!(grid.rows(), 8);
        assert_eq!(grid.columns(), 13);
    }

    #[test]
    fn slots_fill_down_the_rightmost_column_first() {
        let grid = mac();
        assert_eq!(grid.slot_at(0), Slot { column: 0, row: 0 });
        assert_eq!(grid.slot_at(7), Slot { column: 0, row: 7 });
        assert_eq!(grid.slot_at(8), Slot { column: 1, row: 0 });
        let arranged = grid.arrange(9);
        assert_eq!(arranged[8].from_right, ICON_RIGHT + 112.0);
        assert_eq!(arranged[8].top, ICON_TOP);
    }

    #[test]
    fn layout_keeps_saved_positions_and_fills_gaps_for_new_items() {
        let grid = mac();
        let mut saved = BTreeMap::new();
        saved.insert(
            "a".to_owned(),
            grid.slot_placement(Slot { column: 0, row: 0 }),
        );
        saved.insert(
            "b".to_owned(),
            Placement {
                from_right: 400.0,
                top: 300.0,
            },
        );
        let placements = grid.layout(["a", "new", "b", "other"], &saved);
        assert_eq!(
            placements[0],
            grid.slot_placement(Slot { column: 0, row: 0 })
        );
        assert_eq!(
            placements[1],
            grid.slot_placement(Slot { column: 0, row: 1 })
        );
        assert_eq!(
            placements[2],
            Placement {
                from_right: 400.0,
                top: 300.0
            }
        );
        assert_eq!(
            placements[3],
            grid.slot_placement(Slot { column: 0, row: 2 })
        );
    }

    #[test]
    fn saved_positions_off_screen_are_pulled_back() {
        let grid = mac();
        let mut saved = BTreeMap::new();
        saved.insert(
            "far".to_owned(),
            Placement {
                from_right: 5000.0,
                top: -20.0,
            },
        );
        let placement = grid.layout(["far"], &saved)[0];
        assert_eq!(placement.from_right, 1470.0 - 64.0);
        assert_eq!(placement.top, ICON_TOP);
    }

    #[test]
    fn clean_up_snaps_to_the_nearest_free_slot() {
        let grid = mac();
        let tidy = grid.arrange(3);
        assert_eq!(grid.clean_up(&tidy), tidy);
        let messy = [
            Placement {
                from_right: 40.0,
                top: 50.0,
            },
            // Nearest to the same slot as the first; it takes the next one.
            Placement {
                from_right: 30.0,
                top: 70.0,
            },
            Placement {
                from_right: 260.0,
                top: 380.0,
            },
        ];
        let cleaned = grid.clean_up(&messy);
        assert_eq!(cleaned[0], grid.slot_placement(Slot { column: 0, row: 0 }));
        assert_eq!(cleaned[1], grid.slot_placement(Slot { column: 0, row: 1 }));
        assert_eq!(cleaned[2], grid.slot_placement(Slot { column: 2, row: 3 }));
    }

    #[test]
    fn clean_up_spills_into_extra_columns_when_full() {
        let grid = Grid::new(300.0, 300.0, 0.0, ViewOptions::default());
        let slots = usize::from(grid.rows()) * usize::from(grid.columns());
        let placements = vec![grid.slot_placement(Slot { column: 0, row: 0 }); slots + 2];
        let cleaned = grid.clean_up(&placements);
        let mut unique = cleaned
            .iter()
            .map(|placement| (placement.from_right.to_bits(), placement.top.to_bits()))
            .collect::<Vec<_>>();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), slots + 2);
    }

    #[test]
    fn view_options_are_clamped() {
        let options = ViewOptions {
            icon_size: 500.0,
            grid_spacing: f32::NAN,
            text_size: 3.0,
        }
        .normalized();
        assert_eq!(options.icon_size, 128.0);
        assert_eq!(options.grid_spacing, 54.0);
        assert_eq!(options.text_size, 10.0);
    }
}
