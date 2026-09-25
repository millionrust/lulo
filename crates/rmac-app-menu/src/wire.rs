//! The D-Bus encodings of the menu model.
//!
//! `org.rmac.AppMenu1` carries flat menus: `a(sa(sssbb))`, one
//! `(label, action, shortcut, enabled, separator_before)` per item, at most
//! [`V1_LIMITS`]. It stays served unchanged so a menu bar or Spotlight built
//! before submenus existed keeps working: a tree is flattened for it
//! ([`flatten_for_v1`]).
//!
//! `org.rmac.AppMenu2` carries the whole tree: `(ua(sa(sssuy)))`, a
//! revision and, per menu, its items in pre-order as
//! `(label, action, shortcut, flags, depth)`. D-Bus signatures cannot
//! recurse, so a submenu's children follow their parent one level deeper.
//! [`flags`] holds the bits; a reader ignores bits it does not know, so a
//! later version can add some without a new interface.

use std::iter::Peekable;

use crate::{
    valid_action, valid_label, CheckState, Error, Item, Menu, ABOUT_ACTION, MAX_SHORTCUT_BYTES,
};

pub type WireItem = (String, String, String, bool, bool);
pub type WireMenu = (String, Vec<WireItem>);
pub type WireMenus = Vec<WireMenu>;

pub type WireItemV2 = (String, String, String, u32, u8);
pub type WireMenuV2 = (String, Vec<WireItemV2>);
/// `(revision, menus)`: the revision grows by one each time the app
/// publishes a different menu tree.
pub type WireLayout = (u32, Vec<WireMenuV2>);

/// Item flags in `org.rmac.AppMenu2`.
pub mod flags {
    pub const ENABLED: u32 = 1 << 0;
    pub const SEPARATOR_BEFORE: u32 = 1 << 1;
    /// A checkmark. With [`MIXED`] it is the mixed-state dash.
    pub const CHECKED: u32 = 1 << 2;
    pub const MIXED: u32 = 1 << 3;
    /// The item opens a submenu; its children follow at `depth + 1`.
    pub const SUBMENU: u32 = 1 << 4;
}

/// Size limits a reader enforces before it accepts menus.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Limits {
    pub menus: usize,
    /// Items in one menu, counting every submenu item beneath it.
    pub items_per_menu: usize,
    /// 1 means flat.
    pub depth: usize,
}

/// What a version 1 reader accepts; version 1 writers never exceed it.
pub(crate) const V1_LIMITS: Limits = Limits {
    menus: 8,
    items_per_menu: 32,
    depth: 1,
};

/// Room for the Mac's own menus: Preview has eight menus beside the app
/// menu, and Text Editor's Edit menu holds thirty items with its Find
/// submenu open.
pub(crate) const V2_LIMITS: Limits = Limits {
    menus: 12,
    items_per_menu: 96,
    depth: 3,
};

pub(crate) fn validate(menus: &[Menu], limits: Limits) -> Result<(), Error> {
    if menus.is_empty() || menus.len() > limits.menus {
        return Err(Error::Protocol);
    }
    let mut actions = std::collections::BTreeSet::new();
    for menu in menus {
        if !valid_label(&menu.label)
            || menu.items.is_empty()
            || count_items(&menu.items) > limits.items_per_menu
        {
            return Err(Error::Protocol);
        }
        validate_items(&menu.items, 1, limits, &mut actions)?;
    }
    Ok(())
}

fn count_items(items: &[Item]) -> usize {
    items
        .iter()
        .map(|item| 1 + count_items(&item.children))
        .sum()
}

fn validate_items<'a>(
    items: &'a [Item],
    depth: usize,
    limits: Limits,
    actions: &mut std::collections::BTreeSet<&'a str>,
) -> Result<(), Error> {
    for item in items {
        if !valid_label(&item.label)
            || !valid_action(&item.action)
            || item.shortcut.len() > MAX_SHORTCUT_BYTES
            || item.shortcut.chars().any(char::is_control)
            || !actions.insert(item.action.as_str())
        {
            return Err(Error::Protocol);
        }
        if !item.children.is_empty() {
            if depth >= limits.depth {
                return Err(Error::Protocol);
            }
            validate_items(&item.children, depth + 1, limits, actions)?;
        }
    }
    Ok(())
}

pub(crate) fn encode_v1(menus: &[Menu]) -> WireMenus {
    menus
        .iter()
        .map(|menu| {
            (
                menu.label.clone(),
                menu.items
                    .iter()
                    .map(|item| {
                        (
                            item.label.clone(),
                            item.action.clone(),
                            item.shortcut.clone(),
                            item.enabled,
                            item.separator_before,
                        )
                    })
                    .collect(),
            )
        })
        .collect()
}

pub(crate) fn decode_v1(wire: WireMenus) -> Result<Vec<Menu>, Error> {
    let menus = wire
        .into_iter()
        .map(|(label, items)| Menu {
            label,
            items: items
                .into_iter()
                .map(
                    |(label, action, shortcut, enabled, separator_before)| Item {
                        label,
                        action,
                        shortcut,
                        enabled,
                        separator_before,
                        ..Item::default()
                    },
                )
                .collect(),
        })
        .collect::<Vec<_>>();
    validate(&menus, V1_LIMITS)?;
    Ok(menus)
}

/// The tree as a version 1 reader can show it: each submenu's items take
/// its place, set off by separators, and checkmarks are dropped. The About
/// row is left out because a version 1 menu bar draws its own. Menus and
/// items past the version 1 limits are cut off rather than refused, so an
/// older menu bar still shows the first of them.
pub(crate) fn flatten_for_v1(menus: &[Menu]) -> Vec<Menu> {
    menus
        .iter()
        .filter_map(|menu| {
            let mut items = Vec::new();
            flatten_items(&menu.items, &mut items);
            items.retain(|item| item.action != ABOUT_ACTION);
            items.truncate(V1_LIMITS.items_per_menu);
            if let Some(first) = items.first_mut() {
                first.separator_before = false;
            }
            (!items.is_empty()).then(|| Menu {
                label: menu.label.clone(),
                items,
            })
        })
        .take(V1_LIMITS.menus)
        .collect()
}

fn flatten_items(items: &[Item], out: &mut Vec<Item>) {
    let mut separate_next = false;
    for item in items {
        if item.children.is_empty() {
            out.push(Item {
                separator_before: item.separator_before || separate_next,
                checked: CheckState::Off,
                ..item.clone()
            });
            separate_next = false;
        } else {
            let start = out.len();
            flatten_items(&item.children, out);
            if let Some(first) = out.get_mut(start) {
                first.separator_before = true;
            }
            separate_next = true;
        }
    }
}

pub(crate) fn encode_v2(menus: &[Menu]) -> Vec<WireMenuV2> {
    menus
        .iter()
        .map(|menu| {
            let mut items = Vec::new();
            encode_items(&menu.items, 0, &mut items);
            (menu.label.clone(), items)
        })
        .collect()
}

fn encode_items(items: &[Item], depth: u8, out: &mut Vec<WireItemV2>) {
    for item in items {
        let mut bits = 0;
        if item.enabled {
            bits |= flags::ENABLED;
        }
        if item.separator_before {
            bits |= flags::SEPARATOR_BEFORE;
        }
        match item.checked {
            CheckState::Off => {}
            CheckState::On => bits |= flags::CHECKED,
            CheckState::Mixed => bits |= flags::CHECKED | flags::MIXED,
        }
        if !item.children.is_empty() {
            bits |= flags::SUBMENU;
        }
        out.push((
            item.label.clone(),
            item.action.clone(),
            item.shortcut.clone(),
            bits,
            depth,
        ));
        encode_items(&item.children, depth.saturating_add(1), out);
    }
}

pub(crate) fn decode_v2(wire: Vec<WireMenuV2>) -> Result<Vec<Menu>, Error> {
    let menus = wire
        .into_iter()
        .map(|(label, items)| {
            let mut items = items.into_iter().peekable();
            let decoded = decode_level(&mut items, 0)?;
            // Anything left over sat deeper than a submenu allows.
            if items.next().is_some() {
                return Err(Error::Protocol);
            }
            Ok(Menu {
                label,
                items: decoded,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    validate(&menus, V2_LIMITS)?;
    Ok(menus)
}

fn decode_level(
    items: &mut Peekable<std::vec::IntoIter<WireItemV2>>,
    depth: u8,
) -> Result<Vec<Item>, Error> {
    let mut level = Vec::new();
    while let Some(&(_, _, _, _, item_depth)) = items.peek() {
        if item_depth < depth {
            break;
        }
        if item_depth > depth {
            // A child whose parent is not marked as a submenu.
            return Err(Error::Protocol);
        }
        let Some((label, action, shortcut, bits, _)) = items.next() else {
            break;
        };
        let checked = match (bits & flags::CHECKED != 0, bits & flags::MIXED != 0) {
            (false, _) => CheckState::Off,
            (true, false) => CheckState::On,
            (true, true) => CheckState::Mixed,
        };
        let children = if bits & flags::SUBMENU != 0 {
            if usize::from(depth) + 1 >= V2_LIMITS.depth {
                return Err(Error::Protocol);
            }
            let children = decode_level(items, depth + 1)?;
            if children.is_empty() {
                return Err(Error::Protocol);
            }
            children
        } else {
            Vec::new()
        };
        level.push(Item {
            label,
            action,
            shortcut,
            enabled: bits & flags::ENABLED != 0,
            separator_before: bits & flags::SEPARATOR_BEFORE != 0,
            checked,
            children,
            badge: String::new(),
        });
    }
    Ok(level)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(label: &str, action: &str) -> Item {
        Item::new(label, action, "")
    }

    fn tree() -> Vec<Menu> {
        vec![Menu {
            label: "View".into(),
            items: vec![
                leaf("as List", "notes::ViewAsList").checked(true),
                Item::submenu(
                    "Sort By",
                    "notes::SortByMenu",
                    vec![
                        leaf("Date Edited", "notes::SortByEdited").checked(true),
                        leaf("Title", "notes::SortByTitle"),
                    ],
                )
                .separated(),
                leaf("Show Folders", "notes::ToggleFolders")
                    .separated()
                    .enabled(false),
            ],
        }]
    }

    #[test]
    fn version_2_round_trips_submenus_checkmarks_and_state() {
        let menus = tree();
        let wire = encode_v2(&menus);
        assert_eq!(wire[0].1.len(), 5);
        assert_eq!(wire[0].1[1].4, 0);
        assert_eq!(wire[0].1[2].4, 1);
        assert_ne!(wire[0].1[1].3 & flags::SUBMENU, 0);
        assert_eq!(wire[0].1[4].3 & flags::ENABLED, 0);
        assert_eq!(decode_v2(wire).unwrap(), menus);
    }

    #[test]
    fn mixed_state_survives_the_wire() {
        let mut menus = tree();
        menus[0].items[0].checked = CheckState::Mixed;
        assert_eq!(
            decode_v2(encode_v2(&menus)).unwrap()[0].items[0].checked,
            CheckState::Mixed
        );
    }

    #[test]
    fn version_2_rejects_orphans_empty_submenus_and_unknown_depths() {
        let item = |action: &str, bits, depth| {
            (
                "Row".to_owned(),
                action.to_owned(),
                String::new(),
                bits,
                depth,
            )
        };
        // A child with no submenu parent.
        let orphan = vec![(
            "Edit".to_owned(),
            vec![
                item("a::One", flags::ENABLED, 0),
                item("a::Two", flags::ENABLED, 1),
            ],
        )];
        assert_eq!(decode_v2(orphan), Err(Error::Protocol));
        // A submenu with nothing in it.
        let empty = vec![("Edit".to_owned(), vec![item("a::Menu", flags::SUBMENU, 0)])];
        assert_eq!(decode_v2(empty), Err(Error::Protocol));
        // Deeper than three levels.
        let deep = vec![(
            "Edit".to_owned(),
            vec![
                item("a::One", flags::SUBMENU, 0),
                item("a::Two", flags::SUBMENU, 1),
                item("a::Three", flags::SUBMENU, 2),
                item("a::Four", flags::ENABLED, 3),
            ],
        )];
        assert_eq!(decode_v2(deep), Err(Error::Protocol));
    }

    #[test]
    fn unknown_flag_bits_are_ignored() {
        let wire = vec![(
            "Edit".to_owned(),
            vec![(
                "Copy".to_owned(),
                "a::Copy".to_owned(),
                "⌘C".to_owned(),
                flags::ENABLED | 1 << 20,
                0,
            )],
        )];
        let menus = decode_v2(wire).unwrap();
        assert!(menus[0].items[0].enabled);
        assert_eq!(menus[0].items[0].checked, CheckState::Off);
    }

    #[test]
    fn version_1_readers_get_the_tree_flattened_between_separators() {
        let flat = flatten_for_v1(&tree());
        let rows = flat[0]
            .items
            .iter()
            .map(|item| (item.label.as_str(), item.separator_before))
            .collect::<Vec<_>>();
        assert_eq!(
            rows,
            [
                ("as List", false),
                ("Date Edited", true),
                ("Title", false),
                ("Show Folders", true),
            ]
        );
        assert!(flat[0]
            .items
            .iter()
            .all(|item| item.checked == CheckState::Off && item.children.is_empty()));
        assert_eq!(decode_v1(encode_v1(&flat)).unwrap(), flat);
    }

    #[test]
    fn version_1_readers_are_kept_within_their_limits() {
        let long = Menu {
            label: "Edit".into(),
            items: (0..40)
                .map(|index| leaf("Row", &format!("a::Row{index}")))
                .collect(),
        };
        let about = Menu {
            label: crate::APPLICATION_MENU.into(),
            items: vec![leaf("About", ABOUT_ACTION)],
        };
        let mut menus = vec![about];
        menus.extend(std::iter::repeat_n(long, 10));
        let flat = flatten_for_v1(&menus);
        // The About-only application menu is left out entirely.
        assert_eq!(flat.len(), V1_LIMITS.menus);
        assert!(flat
            .iter()
            .all(|menu| menu.items.len() == V1_LIMITS.items_per_menu));
        assert!(validate(&flat[..1], V1_LIMITS).is_ok());
    }

    #[test]
    fn version_1_decoding_still_refuses_oversized_menus() {
        let rows = (0..33)
            .map(|index| {
                (
                    "Row".to_owned(),
                    format!("a::Row{index}"),
                    String::new(),
                    true,
                    false,
                )
            })
            .collect();
        assert_eq!(
            decode_v1(vec![("Edit".to_owned(), rows)]),
            Err(Error::Protocol)
        );
    }
}
