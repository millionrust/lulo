//! Desktop Stacks: "Use Stacks" groups loose files by kind into piles that
//! expand in place when clicked. Folders stay loose, after the stacks.
//! Which kinds exist and their names follow Finder; ordering and the
//! two-item threshold are S (see design-lab/desktop.html).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{Item, ItemKind};

/// Finder's "Group Stacks By: Kind" groups, in the order they are shown.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum StackKind {
    Archives,
    Documents,
    Images,
    Movies,
    Music,
    PdfDocuments,
    Presentations,
    Screenshots,
    Spreadsheets,
    Other,
}

impl StackKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Archives => "Archives",
            Self::Documents => "Documents",
            Self::Images => "Images",
            Self::Movies => "Movies",
            Self::Music => "Music",
            Self::PdfDocuments => "PDF Documents",
            Self::Presentations => "Presentations",
            Self::Screenshots => "Screenshots",
            Self::Spreadsheets => "Spreadsheets",
            Self::Other => "Other",
        }
    }
}

/// The stack a file belongs to; `None` for folders, which never stack.
pub fn classify(name: &str, kind: ItemKind) -> Option<StackKind> {
    if kind == ItemKind::Directory {
        return None;
    }
    let extension = name
        .rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    // rmac's own screenshot names ("Screenshot 2026-09-23 at 21.08.12.png").
    if name.starts_with("Screenshot ") && matches!(extension.as_str(), "png" | "jpg" | "mov") {
        return Some(StackKind::Screenshots);
    }
    Some(match extension.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "heif" | "tif" | "tiff" | "bmp"
        | "svg" | "avif" => StackKind::Images,
        "mov" | "mp4" | "m4v" | "mkv" | "webm" | "avi" => StackKind::Movies,
        "mp3" | "m4a" | "aac" | "flac" | "wav" | "ogg" | "opus" | "aiff" => StackKind::Music,
        "pdf" => StackKind::PdfDocuments,
        "key" | "ppt" | "pptx" | "odp" => StackKind::Presentations,
        "numbers" | "xls" | "xlsx" | "ods" | "csv" | "tsv" => StackKind::Spreadsheets,
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "7z" | "rar" | "dmg" | "iso" => {
            StackKind::Archives
        }
        "txt" | "md" | "rtf" | "rtfd" | "doc" | "docx" | "odt" | "pages" | "html" | "htm"
        | "json" | "xml" | "log" => StackKind::Documents,
        _ => StackKind::Other,
    })
}

/// A kind label for Sort By › Kind: the stack name, "Folder" for folders.
pub fn kind_label(item: &Item) -> &'static str {
    classify(&item.name, item.kind).map_or("Folder", StackKind::label)
}

/// One position on a stacked desktop.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Entry {
    /// Item indices, newest first.
    Stack {
        kind: StackKind,
        members: Vec<usize>,
    },
    Item(usize),
}

/// Groups a snapshot's items. A kind with a single file stays loose (S).
pub fn group(items: &[Item]) -> Vec<Entry> {
    let mut kinds = BTreeMap::<StackKind, Vec<usize>>::new();
    let mut loose = Vec::new();
    for (index, item) in items.iter().enumerate() {
        match classify(&item.name, item.kind) {
            Some(kind) => kinds.entry(kind).or_default().push(index),
            None => loose.push(index),
        }
    }
    let mut entries = Vec::new();
    for (kind, mut members) in kinds {
        if members.len() < 2 {
            loose.extend(members);
            continue;
        }
        members.sort_by(|&left, &right| {
            items[right]
                .modified_millis
                .cmp(&items[left].modified_millis)
                .then_with(|| items[left].name.cmp(&items[right].name))
        });
        entries.push(Entry::Stack { kind, members });
    }
    loose.sort_by(|&left, &right| {
        let (left, right) = (&items[left], &items[right]);
        (left.kind != ItemKind::Directory)
            .cmp(&(right.kind != ItemKind::Directory))
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    entries.extend(loose.into_iter().map(Entry::Item));
    entries
}

/// What occupies one grid slot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Tile {
    /// A pile; `top` holds up to three member indices for its artwork.
    Stack {
        kind: StackKind,
        count: usize,
        top: Vec<usize>,
        expanded: bool,
    },
    /// An item, and the stack it was expanded out of.
    Item {
        index: usize,
        stack: Option<StackKind>,
    },
}

/// The slots in order: an expanded stack is followed by its members.
pub fn tiles(entries: &[Entry], expanded: &BTreeSet<StackKind>) -> Vec<Tile> {
    let mut tiles = Vec::new();
    for entry in entries {
        match entry {
            Entry::Stack { kind, members } => {
                let open = expanded.contains(kind);
                tiles.push(Tile::Stack {
                    kind: *kind,
                    count: members.len(),
                    top: members.iter().take(3).copied().collect(),
                    expanded: open,
                });
                if open {
                    tiles.extend(members.iter().map(|&index| Tile::Item {
                        index,
                        stack: Some(*kind),
                    }));
                }
            }
            Entry::Item(index) => tiles.push(Tile::Item {
                index: *index,
                stack: None,
            }),
        }
    }
    tiles
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn item(name: &str, kind: ItemKind, modified: u128) -> Item {
        Item {
            path: PathBuf::from("/desk").join(name),
            name: name.to_owned(),
            kind,
            size_bytes: 1,
            modified_millis: modified,
            created_millis: 0,
        }
    }

    #[test]
    fn files_are_classified_by_kind_and_folders_never_stack() {
        assert_eq!(
            classify("Photo.JPG", ItemKind::File),
            Some(StackKind::Images)
        );
        assert_eq!(
            classify("clip.mov", ItemKind::File),
            Some(StackKind::Movies)
        );
        assert_eq!(
            classify("Screenshot 2026-09-23 at 21.08.12.png", ItemKind::File),
            Some(StackKind::Screenshots)
        );
        assert_eq!(
            classify("Lease.pdf", ItemKind::File),
            Some(StackKind::PdfDocuments)
        );
        assert_eq!(
            classify("Budget.xlsx", ItemKind::File),
            Some(StackKind::Spreadsheets)
        );
        assert_eq!(classify(".bashrc", ItemKind::File), Some(StackKind::Other));
        assert_eq!(classify("notes", ItemKind::File), Some(StackKind::Other));
        assert_eq!(classify("Projects.png", ItemKind::Directory), None);
        assert_eq!(
            kind_label(&item("Projects", ItemKind::Directory, 0)),
            "Folder"
        );
    }

    #[test]
    fn stacks_hold_two_or_more_items_newest_first_then_loose_items() {
        let items = [
            item("b.png", ItemKind::File, 10),
            item("Projects", ItemKind::Directory, 1),
            item("a.png", ItemKind::File, 30),
            item("Lease.pdf", ItemKind::File, 5),
            item("c.jpg", ItemKind::File, 20),
            item("demo.mov", ItemKind::File, 2),
            item("clip.mp4", ItemKind::File, 3),
        ];
        let entries = group(&items);
        assert_eq!(
            entries,
            [
                Entry::Stack {
                    kind: StackKind::Images,
                    members: vec![2, 4, 0],
                },
                Entry::Stack {
                    kind: StackKind::Movies,
                    members: vec![6, 5],
                },
                Entry::Item(1),
                Entry::Item(3),
            ]
        );
    }

    #[test]
    fn an_expanded_stack_is_followed_by_its_members() {
        let items = [
            item("a.png", ItemKind::File, 2),
            item("b.png", ItemKind::File, 1),
            item("x.mov", ItemKind::File, 1),
            item("y.mov", ItemKind::File, 2),
            item("Folder", ItemKind::Directory, 0),
        ];
        let entries = group(&items);
        let collapsed = tiles(&entries, &BTreeSet::new());
        assert_eq!(collapsed.len(), 3);
        let expanded = tiles(&entries, &BTreeSet::from([StackKind::Images]));
        assert_eq!(
            expanded,
            [
                Tile::Stack {
                    kind: StackKind::Images,
                    count: 2,
                    top: vec![0, 1],
                    expanded: true,
                },
                Tile::Item {
                    index: 0,
                    stack: Some(StackKind::Images),
                },
                Tile::Item {
                    index: 1,
                    stack: Some(StackKind::Images),
                },
                Tile::Stack {
                    kind: StackKind::Movies,
                    count: 2,
                    top: vec![3, 2],
                    expanded: false,
                },
                Tile::Item {
                    index: 4,
                    stack: None,
                },
            ]
        );
    }
}
