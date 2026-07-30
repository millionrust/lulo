/// Accept a streamed snapshot only when it belongs to the current mutation
/// generation and neither initial loading nor a mutation owns the authority.
pub(super) fn snapshot_is_current(
    snapshot_generation: u64,
    current_generation: u64,
    loading: bool,
    busy: bool,
) -> bool {
    snapshot_generation == current_generation && !loading && !busy
}

/// A mutation needs an independent recovery read when another mutation is
/// active, or when initial loading cannot rely on a live stream.
pub(super) fn change_needs_followup(busy: bool, loading: bool, stream_unavailable: bool) -> bool {
    busy || (loading && stream_unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streamed_snapshots_cannot_cross_mutation_generations() {
        assert!(snapshot_is_current(4, 4, false, false));
        assert!(!snapshot_is_current(3, 4, false, false));
        assert!(!snapshot_is_current(4, 4, true, false));
        assert!(!snapshot_is_current(4, 4, false, true));
    }

    #[test]
    fn recovery_reads_are_requested_only_when_stream_state_is_insufficient() {
        assert!(!change_needs_followup(false, true, false));
        assert!(change_needs_followup(false, true, true));
        assert!(change_needs_followup(true, false, false));
        assert!(!change_needs_followup(false, false, true));
    }
}
