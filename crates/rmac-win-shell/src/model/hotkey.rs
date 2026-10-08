//! Spotlight's hotkey on Windows, and what happens when another app holds
//! it. Alt+Space comes first: Alt is the key in the ⌘ position on a PC
//! keyboard. Windows gives a hotkey to the first app that registers it,
//! and Alt+Space is a favourite (the ChatGPT app, PowerToys Run), so when
//! it is taken Lulo falls back down a fixed list, never taking a hotkey
//! from the app that holds it. Win+Space is not a registrable hotkey (it
//! is Windows' input-language switch), so Lulo reads it through its
//! low-level keyboard hook, and only on a PC with one keyboard layout,
//! where that switch has nothing to do.

/// A hotkey Spotlight can open on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Hotkey {
    AltSpace,
    WinSpace,
    CtrlAltSpace,
    AltShiftSpace,
}

/// How a hotkey is claimed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Claim {
    /// `RegisterHotKey`: refused when another app has it.
    Register,
    /// The low-level keyboard hook (Win+Space).
    Hook,
}

impl Hotkey {
    /// The fallback order.
    pub const ORDER: [Hotkey; 4] = [
        Hotkey::AltSpace,
        Hotkey::WinSpace,
        Hotkey::CtrlAltSpace,
        Hotkey::AltShiftSpace,
    ];

    /// As Windows writes it.
    pub fn label(self) -> &'static str {
        match self {
            Hotkey::AltSpace => "Alt+Space",
            Hotkey::WinSpace => "Win+Space",
            Hotkey::CtrlAltSpace => "Ctrl+Alt+Space",
            Hotkey::AltShiftSpace => "Alt+Shift+Space",
        }
    }

    pub fn claim(self) -> Claim {
        match self {
            Hotkey::WinSpace => Claim::Hook,
            _ => Claim::Register,
        }
    }

    /// A small number for the registry record of the notice already shown.
    pub fn code(self) -> u32 {
        match self {
            Hotkey::AltSpace => 1,
            Hotkey::WinSpace => 2,
            Hotkey::CtrlAltSpace => 3,
            Hotkey::AltShiftSpace => 4,
        }
    }

    /// Whether this is a fallback (the first choice was taken).
    pub fn is_fallback(self) -> bool {
        self != Hotkey::AltSpace
    }
}

/// The first hotkey of the fallback order that can be claimed: `try_claim`
/// registers it (or installs the hook) and says whether that worked. Win+
/// Space is skipped when the PC has more than one keyboard layout, whose
/// switch it is.
pub fn choose(
    keyboard_layouts: usize,
    mut try_claim: impl FnMut(Hotkey) -> bool,
) -> Option<Hotkey> {
    Hotkey::ORDER.into_iter().find(|&hotkey| {
        if hotkey == Hotkey::WinSpace && keyboard_layouts > 1 {
            return false;
        }
        try_claim(hotkey)
    })
}

/// Whether to show the one-time notice: a fallback is in effect and the
/// user has not been told about this one yet (`noticed` is the code last
/// recorded).
pub fn should_notice(hotkey: Hotkey, noticed: Option<u32>) -> bool {
    hotkey.is_fallback() && noticed != Some(hotkey.code())
}

/// The notice's text.
pub fn notice(hotkey: Hotkey) -> (String, String) {
    (
        format!("Spotlight opens with {}", hotkey.label()),
        format!(
            "Another app uses {}. You can also click the magnifying glass in the menu bar.",
            Hotkey::AltSpace.label()
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alt_space_wins_when_it_is_free() {
        let mut tried = Vec::new();
        let chosen = choose(1, |hotkey| {
            tried.push(hotkey);
            true
        });
        assert_eq!(chosen, Some(Hotkey::AltSpace));
        assert_eq!(tried, [Hotkey::AltSpace]);
        assert!(!should_notice(Hotkey::AltSpace, None));
    }

    #[test]
    fn a_taken_alt_space_falls_back_to_win_space_on_one_layout() {
        let chosen = choose(1, |hotkey| hotkey != Hotkey::AltSpace);
        assert_eq!(chosen, Some(Hotkey::WinSpace));
        assert_eq!(chosen.unwrap().claim(), Claim::Hook);
        assert!(should_notice(Hotkey::WinSpace, None));
        assert!(!should_notice(
            Hotkey::WinSpace,
            Some(Hotkey::WinSpace.code())
        ));
        // A different fallback later is told about again.
        assert!(should_notice(
            Hotkey::CtrlAltSpace,
            Some(Hotkey::WinSpace.code())
        ));
    }

    #[test]
    fn several_layouts_keep_win_space_for_switching_them() {
        let mut tried = Vec::new();
        let chosen = choose(2, |hotkey| {
            tried.push(hotkey);
            hotkey == Hotkey::CtrlAltSpace || hotkey == Hotkey::WinSpace
        });
        assert_eq!(chosen, Some(Hotkey::CtrlAltSpace));
        assert!(!tried.contains(&Hotkey::WinSpace));
    }

    #[test]
    fn nothing_free_leaves_the_menu_bar_icon() {
        assert_eq!(choose(1, |_| false), None);
        let (title, detail) = notice(Hotkey::WinSpace);
        assert_eq!(title, "Spotlight opens with Win+Space");
        assert!(detail.contains("Alt+Space"));
    }
}
