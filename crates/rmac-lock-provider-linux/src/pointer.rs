//! Platform-neutral hit testing for lock-screen pointer controls.

use crate::keyboard::DecodedKey;
use crate::paint::{prompt_can_submit, PromptGeometry, PromptVisual};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PointerTarget {
    Submit,
    SelectPrevious,
    SelectNext,
}

impl PointerTarget {
    pub(crate) fn into_key(self) -> DecodedKey {
        match self {
            Self::Submit => DecodedKey::Submit,
            Self::SelectPrevious => DecodedKey::SelectPrevious,
            Self::SelectNext => DecodedKey::SelectNext,
        }
    }
}

#[derive(Default)]
pub(crate) struct PointerGesture {
    pressed: Option<PointerTarget>,
}

impl PointerGesture {
    pub(crate) fn press(&mut self, target: Option<PointerTarget>) {
        self.pressed = target;
    }

    pub(crate) fn release(&mut self, target: Option<PointerTarget>) -> Option<DecodedKey> {
        self.pressed
            .take()
            .filter(|pressed| Some(*pressed) == target)
            .map(PointerTarget::into_key)
    }

    pub(crate) fn clear(&mut self) {
        self.pressed = None;
    }
}

impl std::fmt::Debug for PointerGesture {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PointerGesture(<redacted>)")
    }
}

pub(crate) fn hit_test(
    width: u32,
    height: u32,
    prompt: PromptVisual,
    x: f64,
    y: f64,
) -> Option<PointerTarget> {
    if width == 0
        || height == 0
        || !x.is_finite()
        || !y.is_finite()
        || x < 0.0
        || y < 0.0
        || x >= f64::from(width)
        || y >= f64::from(height)
    {
        return None;
    }
    let x = x as i64;
    let y = y as i64;
    let center_x = i64::from(width / 2);
    let geometry = PromptGeometry::new(width, height, 1);
    if inside_circle(
        x,
        y,
        geometry.submit_x,
        geometry.center_y,
        geometry.submit_radius,
    ) && prompt_can_submit(prompt)
    {
        return Some(PointerTarget::Submit);
    }
    if matches!(prompt, PromptVisual::Radio { .. })
        && (x - center_x).abs() <= 19
        && (y - geometry.center_y).abs() <= 14
    {
        return Some(if x < center_x {
            PointerTarget::SelectPrevious
        } else {
            PointerTarget::SelectNext
        });
    }
    None
}

fn inside_circle(x: i64, y: i64, center_x: i64, center_y: i64, radius: i64) -> bool {
    let dx = x - center_x;
    let dy = y - center_y;
    dx * dx + dy * dy <= radius * radius
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submit_target_matches_the_visible_accent_and_rejects_binary_prompts() {
        // 800x600: pill centre=(400,490), half-width=90, submit arrow=(476,490).
        // An empty field has no arrow, as on the Mac.
        assert_eq!(
            hit_test(800, 600, PromptVisual::secret(0), 476.0, 490.0),
            None
        );
        assert_eq!(
            hit_test(800, 600, PromptVisual::secret(4), 476.0, 490.0),
            Some(PointerTarget::Submit)
        );
        assert_eq!(hit_test(800, 600, PromptVisual::Binary, 476.0, 490.0), None);
        assert_eq!(hit_test(800, 600, PromptVisual::Hidden, 476.0, 490.0), None);
        assert_eq!(
            hit_test(800, 600, PromptVisual::Authenticating, 476.0, 490.0),
            None
        );
        assert!(matches!(
            PointerTarget::Submit.into_key(),
            DecodedKey::Submit
        ));
    }

    #[test]
    fn radio_halves_select_and_release_outside_cannot_activate() {
        let radio = PromptVisual::Radio { selected: false };
        assert_eq!(
            hit_test(800, 600, radio, 390.0, 490.0),
            Some(PointerTarget::SelectPrevious)
        );
        assert_eq!(
            hit_test(800, 600, radio, 410.0, 490.0),
            Some(PointerTarget::SelectNext)
        );
        for (x, y) in [(-1.0, 0.0), (f64::NAN, 2.0), (800.0, 10.0)] {
            assert_eq!(hit_test(800, 600, radio, x, y), None);
        }

        let mut gesture = PointerGesture::default();
        gesture.press(Some(PointerTarget::SelectPrevious));
        assert!(gesture.release(Some(PointerTarget::SelectNext)).is_none());
        gesture.press(Some(PointerTarget::SelectPrevious));
        gesture.clear();
        assert!(gesture
            .release(Some(PointerTarget::SelectPrevious))
            .is_none());
        gesture.press(Some(PointerTarget::SelectNext));
        assert!(matches!(
            gesture.release(Some(PointerTarget::SelectNext)),
            Some(DecodedKey::SelectNext)
        ));
        assert_eq!(format!("{gesture:?}"), "PointerGesture(<redacted>)");
    }
}
