use super::*;

#[test]
fn source_sets_merge_without_losing_independent_services() {
    let mut sources = Sources::audio();
    sources.merge(Sources::system_bus());
    assert_eq!(sources, Sources::all());
    assert!(!sources.is_empty());
    assert!(Sources::empty().is_empty());
}

#[test]
fn source_sets_scope_transport_failures() {
    assert!(!Sources::system_bus().audio);
    assert_eq!(
        Sources::audio(),
        Sources {
            audio: true,
            ..Sources::empty()
        }
    );
}
