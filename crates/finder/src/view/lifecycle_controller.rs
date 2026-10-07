use super::*;

impl Drop for FinderView {
    fn drop(&mut self) {
        if let Some(transfer) = &self.transfer {
            transfer.cancel.store(true, Ordering::Release);
        }
        if let Some(undo) = &self.undo_operation {
            undo.cancel.store(true, Ordering::Release);
        }
        #[cfg(any(target_os = "linux", test))]
        if let Some(trash) = &self.trash_operation {
            trash.cancel.store(true, Ordering::Release);
        }
        if let Some(cancel) = &self.search_cancel {
            cancel.store(true, Ordering::Release);
        }
    }
}

impl FinderView {
    pub(super) fn record_operation_failures(
        &mut self,
        failures: Vec<file_ops::Failure>,
        cx: &mut Context<Self>,
    ) {
        if !failures.is_empty() {
            let _ = rmac_sound::play(rmac_sound::Cue::Error);
        }
        self.operation_error = failures.first().map(|first| {
            if failures.len() == 1 {
                first.to_string().into()
            } else {
                format!("{} (and {} more failures)", first, failures.len() - 1).into()
            }
        });
        cx.notify();
    }

    pub(super) fn begin_search(&mut self) -> (u64, Arc<AtomicBool>) {
        self.cancel_search();
        self.search_generation = self.search_generation.wrapping_add(1);
        self.operation_error = None;
        self.search_summary = None;
        self.search_relevance_order = false;
        let cancel = Arc::new(AtomicBool::new(false));
        self.search_cancel = Some(cancel.clone());
        (self.search_generation, cancel)
    }

    pub(super) fn cancel_search(&mut self) {
        if let Some(cancel) = self.search_cancel.take() {
            cancel.store(true, Ordering::Release);
        }
        self.search_generation = self.search_generation.wrapping_add(1);
    }
}
