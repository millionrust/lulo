//! Executable visual contract for the rmac component system.

mod gallery;
mod specimens;

use gallery::ComponentGallery;
#[cfg(test)]
use rmac_ui::gallery::{COMPONENT_SPECS, PREVIEW_SCALES};

fn main() {
    rmac_ui::boot("Component Gallery", 1280.0, 860.0, ComponentGallery::new);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specimen_count_is_stable_and_nontrivial() {
        let count = COMPONENT_SPECS
            .iter()
            .map(|spec| spec.states.len())
            .sum::<usize>();
        assert_eq!(count, 98);
    }

    #[test]
    fn scale_navigation_is_bounded() {
        assert_eq!(PREVIEW_SCALES.first().unwrap().label, "100%");
        assert_eq!(PREVIEW_SCALES.last().unwrap().label, "200%");
    }
}
