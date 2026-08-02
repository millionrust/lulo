use super::*;

fn dark_snapshot() -> Snapshot {
    Snapshot {
        available: true,
        color_scheme: ColorScheme::PreferDark,
        accent_color: AccentColor::new(0.0, 0.48, 1.0),
        contrast: Contrast::Higher,
        motion: MotionPreference::Reduced,
        capabilities: Capabilities {
            color_scheme: true,
            accent_color: true,
            contrast: true,
            reduced_motion: true,
        },
        detail: None,
    }
}

#[test]
fn text_scales_are_ordered_and_bounded() {
    assert_eq!(TextScale::Standard.factor(), 1.0);
    assert!(TextScale::Large.factor() > TextScale::Standard.factor());
    assert!(TextScale::ExtraLarge.factor() > TextScale::Large.factor());
    assert!(TextScale::ExtraLarge.factor() <= 1.3);
}

#[test]
fn accent_color_rejects_non_finite_and_out_of_range_components() {
    assert!(AccentColor::new(0.0, 0.5, 1.0).is_some());
    assert!(AccentColor::new(-0.1, 0.5, 1.0).is_none());
    assert!(AccentColor::new(0.0, 1.1, 1.0).is_none());
    assert!(AccentColor::new(0.0, f64::NAN, 1.0).is_none());
}

#[test]
fn state_keeps_last_known_good_snapshot_during_source_loss() {
    let snapshot = dark_snapshot();
    let mut state = AppearanceState::default();
    assert!(state.apply(Event::Snapshot(snapshot.clone())));
    assert!(state.apply(Event::Unavailable(Error::new(
        "watch appearance settings",
        "portal stopped"
    ))));
    assert_eq!(state.snapshot, snapshot);
    assert!(state.source_error.is_some());
}

#[test]
fn repeated_events_do_not_request_an_unnecessary_redraw() {
    let snapshot = dark_snapshot();
    let mut state = AppearanceState {
        snapshot: snapshot.clone(),
        source_error: None,
    };
    assert!(!state.apply(Event::Snapshot(snapshot)));
}

#[test]
fn fake_controller_updates_authoritative_state() {
    let (source, controller) = FakeAppearanceSource::new(Snapshot::default());
    let snapshot = dark_snapshot();
    controller.set_snapshot(snapshot.clone());
    assert_eq!(source.current(), snapshot);
}
