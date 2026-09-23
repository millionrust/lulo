//! The portal's `parent_window` identifier.
//!
//! GPUI 0.2 cannot import a foreign toplevel (xdg-foreign `zxdg_importer_v2`)
//! or reparent to an X11 window, so the handle is parsed, bounded, and kept for
//! diagnostics and future transient parenting; the panel itself opens as a
//! modal `xdg_dialog_v1` window centred on the active output. See ADR 0012.

pub const MAX_HANDLE_BYTES: usize = 256;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ParentWindow {
    #[default]
    None,
    /// `wayland:<xdg-foreign exported handle>`
    Wayland(String),
    /// `x11:<hexadecimal XID>`
    X11(u32),
}

impl ParentWindow {
    pub fn parse(value: &str) -> Self {
        if value.len() > MAX_HANDLE_BYTES {
            return Self::None;
        }
        if let Some(handle) = value.strip_prefix("wayland:") {
            let valid = !handle.is_empty()
                && handle
                    .bytes()
                    .all(|byte| byte.is_ascii_graphic() && byte != b'/');
            return if valid {
                Self::Wayland(handle.to_owned())
            } else {
                Self::None
            };
        }
        if let Some(xid) = value.strip_prefix("x11:") {
            let xid = xid.trim_start_matches("0x");
            return u32::from_str_radix(xid, 16)
                .ok()
                .filter(|xid| *xid != 0)
                .map_or(Self::None, Self::X11);
        }
        Self::None
    }
}
