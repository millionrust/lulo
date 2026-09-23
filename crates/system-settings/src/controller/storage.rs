//! Mounted-volume refresh, identity revalidation, and Files opening lifecycle.

use super::*;

mod render;

impl Settings {
    pub(super) fn queue_storage_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.system_data_loading || self.storage_busy || self.storage_stream_refreshing {
            self.storage_refresh_pending = true;
            return;
        }
        self.storage_refresh_pending = false;
        self.storage_stream_refreshing = true;
        let generation = self.storage_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_mounts::volumes() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.storage_stream_refreshing = false;
                if storage_stream_snapshot_is_current(
                    generation,
                    this.storage_generation,
                    this.system_data_loading,
                    this.storage_busy,
                ) {
                    match result {
                        Ok(storage) => {
                            this.storage = storage;
                            this.storage_error = None;
                            this.storage_stream_error = None;
                            this.measure_storage_categories(cx);
                        }
                        Err(_) => {
                            this.storage_stream_error =
                                Some("Could not refresh the changed mounted-volume state".into());
                        }
                    }
                } else {
                    this.storage_refresh_pending = true;
                }
                this.run_pending_storage_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn run_pending_storage_refresh(&mut self, cx: &mut Context<Self>) {
        if self.storage_refresh_pending
            && !self.system_data_loading
            && !self.storage_busy
            && !self.storage_stream_refreshing
        {
            self.queue_storage_stream_refresh(cx);
        }
    }

    pub(super) fn refresh_storage(&mut self, cx: &mut Context<Self>) {
        if self.system_data_loading || self.storage_busy {
            return;
        }
        self.storage_busy = true;
        self.storage_generation = self.storage_generation.wrapping_add(1);
        self.storage_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_mounts::volumes() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.storage_busy = false;
                match result {
                    Ok(volumes) => {
                        this.storage = volumes;
                        this.storage_error = None;
                        this.storage_stream_error = None;
                        this.measure_storage_categories(cx);
                    }
                    Err(error) => {
                        this.storage_error =
                            Some(format!("Could not refresh storage volumes: {error}").into());
                    }
                }
                this.run_pending_storage_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// The mounted volume that holds the home folder: the one whose mount
    /// point is the longest prefix of `$HOME`.
    pub(super) fn home_volume(&self) -> Option<&rmac_mounts::Volume> {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        self.storage
            .iter()
            .filter(|volume| home.starts_with(&volume.mount.path))
            .max_by_key(|volume| volume.mount.path.components().count())
    }

    /// Measure the home volume's Storage categories on the background
    /// executor. Called when volumes refresh and when Storage opens.
    pub(super) fn measure_storage_categories(&mut self, cx: &mut Context<Self>) {
        if self.storage_categories_busy {
            return;
        }
        let Some(volume_path) = self.home_volume().map(|volume| volume.mount.path.clone()) else {
            self.storage_categories = None;
            return;
        };
        self.storage_categories_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let measured = cx
                .background_executor()
                .spawn(async move { crate::storage_categories::measure_home(volume_path) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.storage_categories_busy = false;
                this.storage_categories = measured;
                cx.notify();
            });
        })
        .detach();
    }

    /// Open one category's folder in Files.
    pub(super) fn reveal_storage_category(&mut self, folder: PathBuf, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let name = folder
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            if rmac_app_launch::open_item(folder).await.is_err() {
                let _ = this.update(cx, |this: &mut Settings, cx| {
                    this.storage_error = Some(format!("Could not open {name} in Files").into());
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(super) fn open_storage_volume(&mut self, identity: String, cx: &mut Context<Self>) {
        if self.storage_action_busy.is_some() {
            return;
        }
        let Some(mount) = self
            .storage
            .iter()
            .find(|volume| volume.mount.identity == identity)
            .map(|volume| volume.mount.clone())
        else {
            self.storage_refresh_pending = true;
            self.run_pending_storage_refresh(cx);
            return;
        };
        let name = mount.name.clone();
        self.storage_action_busy = Some(identity);
        self.storage_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = match blocking::unblock(move || rmac_mounts::revalidate(&mount)).await {
                Ok(current) => rmac_app_launch::open_item(current.path)
                    .await
                    .map_err(|_| ()),
                Err(_) => Err(()),
            };
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.storage_action_busy = None;
                if result.is_err() {
                    this.storage_error = Some(format!("Could not open {name} in Files").into());
                    this.storage_refresh_pending = true;
                }
                this.run_pending_storage_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }
}
