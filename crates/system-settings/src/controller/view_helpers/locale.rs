//! System Settings locale value formatting.

/// The keyboard layouts localed reports, "layout · variant", as Keyboard's
/// Input Sources value and Language & Region's keyboard row show them.
pub(in crate::controller) fn locale_keyboard_summary(snapshot: &rmac_locale::Snapshot) -> String {
    if snapshot.x11_layout.is_empty() {
        return "Not reported".to_owned();
    }
    let mut value = snapshot.x11_layout.clone();
    if !snapshot.x11_variant.is_empty() {
        value.push_str(" · ");
        value.push_str(&snapshot.x11_variant);
    }
    value
}
