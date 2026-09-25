//! Software Update: the Mac-style Lulo OS item, Other Updates, the More Info
//! and Automatic Updates sheets, and the download-then-restart lifecycle.
//!
//! "Update Now" never installs packages under the running session. It
//! simulates the selection with `ONLY_TRUSTED`, downloads exactly that plan
//! with `ONLY_DOWNLOAD`, and triggers PackageKit's offline update, which
//! `pk-offline-update` installs from `system-update.target` on the next
//! restart. Neither step needs a polkit password (docs/software-update.md
//! "Authorization"), so no authentication agent is involved.

use super::*;

mod render;

/// The dispatch socket the menu bar listens on to restart through its
/// quit-all path (`rmac_shortcuts::power_key::RESTART_TO_UPDATE_SHORTCUT`).
const RESTART_TO_UPDATE: &str = rmac_shortcuts::power_key::RESTART_TO_UPDATE_SHORTCUT;

/// The pane item an Update Now belongs to, so its progress shows there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum UpdateTarget {
    LuloOs,
    Other,
    /// A More Info sheet selection.
    Selection,
}

/// The More Info sheet: a row per item (Lulo OS first), each ticked unless
/// listed in `unticked`, and the row whose details are shown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct InfoSheet {
    pub(super) selected: String,
    pub(super) unticked: std::collections::BTreeSet<String>,
}

impl Settings {
    pub(super) fn finish_update_status(
        &mut self,
        result: std::result::Result<rmac_updates::Snapshot, rmac_updates::Error>,
    ) {
        self.updates_loading = false;
        self.updates_busy = false;
        match result {
            Ok(snapshot) => {
                self.set_update_snapshot(snapshot);
                self.updates_error = None;
                self.updates_stream_error = None;
            }
            Err(error) => {
                self.updates_error = Some(format!("Could not check for updates: {error}").into());
            }
        }
    }

    /// Adopt an authoritative snapshot and tell the menu bar how many items
    /// it lists ("System Settings…, 1 update").
    fn set_update_snapshot(&mut self, snapshot: rmac_updates::Snapshot) {
        let status = rmac_updates::UpdateStatus {
            updates: rmac_updates::Catalog::from_snapshot(&snapshot).item_count(),
            restart_required: snapshot.offline.triggered,
        };
        if let Some(path) = rmac_updates::UpdateStatus::default_path() {
            if rmac_updates::UpdateStatus::load(&path) != status {
                if let Err(error) = status.save(&path) {
                    eprintln!("rmac-system-settings: could not record the update count: {error}");
                }
            }
        }
        if let Some(sheet) = &mut self.updates_info_sheet {
            let keys = info_sheet_keys(&snapshot);
            sheet.unticked.retain(|key| keys.contains(key));
            if !keys.contains(&sheet.selected) {
                sheet.selected = keys.first().cloned().unwrap_or_default();
            }
        }
        self.updates = Some(snapshot);
    }

    pub(super) fn queue_update_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.updates_loading || self.updates_busy || self.updates_stream_refreshing {
            self.updates_refresh_pending = true;
            return;
        }
        self.updates_refresh_pending = false;
        self.updates_stream_refreshing = true;
        let generation = self.updates_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = rmac_updates_linux::snapshot(rmac_updates::Request::cached()).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.updates_stream_refreshing = false;
                if update_stream_snapshot_is_current(
                    generation,
                    this.updates_generation,
                    this.updates_loading,
                    this.updates_busy,
                ) {
                    match result {
                        Ok(snapshot) => {
                            this.set_update_snapshot(snapshot);
                            this.updates_error = None;
                            this.updates_stream_error = None;
                            this.updates_plan = None;
                        }
                        Err(error) => {
                            this.updates_stream_error = Some(
                                format!("Could not refresh changed package state: {error}").into(),
                            );
                        }
                    }
                } else {
                    this.updates_refresh_pending = true;
                }
                this.run_pending_update_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn run_pending_update_refresh(&mut self, cx: &mut Context<Self>) {
        if self.updates_refresh_pending
            && !self.updates_loading
            && !self.updates_busy
            && !self.updates_stream_refreshing
        {
            self.queue_update_stream_refresh(cx);
        }
    }

    pub(super) fn refresh_update_status(&mut self, cx: &mut Context<Self>) {
        if self.updates_loading || self.updates_busy {
            return;
        }
        self.updates_busy = true;
        self.updates_generation = self.updates_generation.wrapping_add(1);
        self.updates_error = None;
        self.updates_plan = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = rmac_updates_linux::snapshot(rmac_updates::Request::refresh()).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_update_status(result);
                this.run_pending_update_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Update Now: refresh, resolve `selection` (plus whatever is already
    /// waiting for restart), and simulate it. A plan that removes, replaces
    /// or downgrades packages waits for confirmation; any other plan
    /// downloads straight away, as the Mac's Update Now does.
    pub(super) fn start_update(
        &mut self,
        selection: Vec<String>,
        target: UpdateTarget,
        cx: &mut Context<Self>,
    ) {
        if self.updates_loading || self.updates_busy || selection.is_empty() {
            return;
        }
        let Some(snapshot) = self.updates.as_ref() else {
            return;
        };
        if !snapshot.install_supported || snapshot.truncated {
            return;
        }
        let cancellation = rmac_updates::Cancellation::default();
        self.updates_busy = true;
        self.updates_preparing = true;
        self.updates_installing = false;
        self.updates_target = Some(target);
        self.updates_info_sheet = None;
        self.updates_generation = self.updates_generation.wrapping_add(1);
        self.updates_error = None;
        self.updates_plan = None;
        self.updates_progress = None;
        self.updates_cancellation = Some(cancellation.clone());
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = rmac_updates_linux::prepare_selection(selection, cancellation).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.updates_busy = false;
                this.updates_preparing = false;
                this.updates_cancellation = None;
                match result {
                    // The fresh set only resolved the selection; the pane
                    // keeps its snapshot with sizes and notes until the
                    // download's own recovery read replaces it.
                    Ok((_snapshot, plan)) => {
                        this.updates_error = None;
                        this.updates_stream_error = None;
                        if plan.has_destructive_changes() {
                            this.updates_plan = Some(plan);
                        } else {
                            this.download_update(plan, cx);
                        }
                    }
                    Err(error) => {
                        this.updates_target = None;
                        this.updates_error =
                            Some(format!("Could not prepare the update: {error}").into());
                    }
                }
                this.run_pending_update_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn cancel_update_plan(&mut self, cx: &mut Context<Self>) {
        if !self.updates_busy {
            self.updates_plan = None;
            self.updates_target = None;
            cx.notify();
        }
    }

    pub(super) fn cancel_update_operation(&mut self, cx: &mut Context<Self>) {
        if let Some(cancellation) = &self.updates_cancellation {
            cancellation.cancel();
            cx.notify();
        }
    }

    /// The confirmation dialog's "Download" for a plan with removals.
    pub(super) fn confirm_update_plan(&mut self, cx: &mut Context<Self>) {
        if self.updates_busy {
            return;
        }
        if let Some(plan) = self.updates_plan.take() {
            self.download_update(plan, cx);
        }
    }

    /// Download the reviewed plan and schedule it for the next restart.
    fn download_update(&mut self, plan: rmac_updates::InstallPlan, cx: &mut Context<Self>) {
        let cancellation = rmac_updates::Cancellation::default();
        let (progress_sender, progress_receiver) = async_channel::bounded(8);
        self.updates_busy = true;
        self.updates_preparing = false;
        self.updates_installing = true;
        self.updates_generation = self.updates_generation.wrapping_add(1);
        self.updates_error = None;
        self.updates_progress = Some(rmac_updates::InstallProgress::default());
        self.updates_cancellation = Some(cancellation.clone());
        cx.notify();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(progress) = progress_receiver.recv().await {
                if this
                    .update(cx, |this: &mut Settings, cx| {
                        if this.updates_installing {
                            this.updates_progress = Some(progress);
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result =
                rmac_updates_linux::prepare_offline(plan, cancellation, progress_sender).await;
            let recovery = rmac_updates_linux::snapshot(rmac_updates::Request::cached()).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.updates_busy = false;
                this.updates_installing = false;
                this.updates_cancellation = None;
                this.updates_target = None;
                match (result, recovery) {
                    (Ok(_), Ok(snapshot)) => {
                        this.set_update_snapshot(snapshot);
                        this.updates_error = None;
                        this.updates_stream_error = None;
                    }
                    (Ok(_), Err(error)) => {
                        this.updates = None;
                        this.updates_error = Some(
                            format!(
                                "The update was downloaded, but its state could not be confirmed: {error}"
                            )
                            .into(),
                        );
                    }
                    (Err(error), Ok(snapshot)) => {
                        this.set_update_snapshot(snapshot);
                        this.updates_error =
                            Some(format!("Could not download the update: {error}").into());
                        this.updates_stream_error = None;
                    }
                    (Err(error), Err(recovery_error)) => {
                        this.updates = None;
                        this.updates_error = Some(
                            format!(
                                "Could not download the update: {error}. The current package state could not be confirmed: {recovery_error}"
                            )
                            .into(),
                        );
                    }
                }
                this.run_pending_update_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Restart Now: the menu bar asks every app to quit, as its Restart
    /// does, then restarts; `pk-offline-update` installs the update.
    pub(super) fn restart_to_update(&mut self, cx: &mut Context<Self>) {
        let id = rmac_shortcuts::ShortcutId(RESTART_TO_UPDATE.into());
        if let Err(error) = rmac_shortcuts::dispatch(&id) {
            self.updates_error = Some(
                format!("Could not restart: {error}. Restart from the Lulo menu instead.").into(),
            );
        }
        cx.notify();
    }

    pub(super) fn open_update_info_sheet(&mut self, selected: String, cx: &mut Context<Self>) {
        if self.updates_busy {
            return;
        }
        self.updates_info_sheet = Some(InfoSheet {
            selected,
            unticked: Default::default(),
        });
        cx.notify();
    }

    pub(super) fn close_update_info_sheet(&mut self, cx: &mut Context<Self>) {
        self.updates_info_sheet = None;
        cx.notify();
    }

    pub(super) fn select_update_info_row(&mut self, key: String, cx: &mut Context<Self>) {
        if let Some(sheet) = &mut self.updates_info_sheet {
            sheet.selected = key;
            cx.notify();
        }
    }

    pub(super) fn toggle_update_info_row(&mut self, key: String, cx: &mut Context<Self>) {
        if let Some(sheet) = &mut self.updates_info_sheet {
            if !sheet.unticked.remove(&key) {
                sheet.unticked.insert(key.clone());
            }
            sheet.selected = key;
            cx.notify();
        }
    }

    /// The More Info sheet's Update Now: every ticked item.
    pub(super) fn update_info_selection(&mut self, cx: &mut Context<Self>) {
        let (Some(sheet), Some(snapshot)) = (&self.updates_info_sheet, &self.updates) else {
            return;
        };
        let selection = info_sheet_keys(snapshot)
            .into_iter()
            .filter(|key| !sheet.unticked.contains(key))
            .collect::<Vec<_>>();
        self.start_update(selection, UpdateTarget::Selection, cx);
    }

    pub(super) fn open_update_auto_sheet(&mut self, cx: &mut Context<Self>) {
        self.updates_auto_sheet = true;
        self.updates_auto_error = None;
        cx.notify();
    }

    pub(super) fn close_update_auto_sheet(&mut self, cx: &mut Context<Self>) {
        self.updates_auto_sheet = false;
        cx.notify();
    }

    /// Apply and persist one Automatic Updates switch at once, as the Mac
    /// does; the daily `rmac-update-check` run reads the file.
    pub(super) fn set_automatic_updates(
        &mut self,
        change: impl FnOnce(&mut rmac_updates::AutomaticUpdates),
        cx: &mut Context<Self>,
    ) {
        let mut next = self.updates_auto;
        change(&mut next);
        let saved = rmac_updates::AutomaticUpdates::default_path()
            .ok_or_else(|| std::io::Error::other("no configuration directory"))
            .and_then(|path| next.save(&path));
        match saved {
            Ok(()) => {
                self.updates_auto = next;
                self.updates_auto_error = None;
            }
            Err(error) => {
                self.updates_auto_error =
                    Some(format!("Could not save Automatic Updates: {error}").into());
            }
        }
        cx.notify();
    }
}

/// The More Info sheet's rows: the Lulo OS item, then each other update.
pub(super) fn info_sheet_keys(snapshot: &rmac_updates::Snapshot) -> Vec<String> {
    let catalog = rmac_updates::Catalog::from_snapshot(snapshot);
    catalog
        .lulo_os
        .as_ref()
        .map(|_| rmac_updates::LULO_OS_ITEM.to_owned())
        .into_iter()
        .chain(catalog.other.iter().map(|update| update.package_id.clone()))
        .collect()
}
