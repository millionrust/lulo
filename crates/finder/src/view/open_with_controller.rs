use super::*;

impl FinderView {
    pub(super) fn request_open_with(&mut self, cx: &mut Context<Self>) {
        if self.applications_view {
            self.operation_error = Some("Applications open directly".into());
            cx.notify();
            return;
        }
        if self.trash_view {
            self.operation_error = Some("Restore the item before choosing an application".into());
            cx.notify();
            return;
        }
        let Some(entry) = self.selected_entry() else {
            return;
        };
        if self.selection_count() != 1 || entry.is_dir {
            self.operation_error =
                Some("Select one file to choose which application opens it".into());
            cx.notify();
            return;
        }

        let path = entry.path.clone();
        self.menu_at = None;
        self.operation_error = None;
        self.open_with = Some(OpenWithPicker {
            path: path.clone(),
            association: None,
            selected: 0,
            make_default: false,
            busy: false,
            error: None,
            browse: None,
        });
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = rmac_app_launch::file_association(path.clone()).await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                let Some(picker) = this.open_with.as_mut().filter(|picker| picker.path == path)
                else {
                    return;
                };
                match result {
                    Ok(association) => {
                        picker.association = Some(association);
                        picker.selected = 0;
                    }
                    Err(error) => {
                        this.open_with = None;
                        this.operation_error =
                            Some(format!("Could not load compatible applications: {error}").into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn close_open_with(&mut self, cx: &mut Context<Self>) {
        if self.open_with.as_ref().is_some_and(|picker| picker.busy) {
            return;
        }
        self.open_with = None;
        cx.notify();
    }

    /// "Choose Application…": load the full catalog so the user can force-open
    /// the file with an application that never declared its type, the same
    /// override macOS's own Open With "Other…" browse allows.
    pub(super) fn request_choose_application(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = self.open_with.as_mut() else {
            return;
        };
        if picker.busy || picker.browse.is_some() {
            return;
        }
        picker.browse = Some(OpenWithBrowse::Loading);
        picker.selected = 0;
        picker.error = None;
        let path = picker.path.clone();
        cx.notify();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = rmac_app_launch::all_applications().await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                let Some(picker) = this.open_with.as_mut().filter(|picker| picker.path == path)
                else {
                    return;
                };
                match result {
                    Ok(mut applications) => {
                        applications.sort_by(|left, right| {
                            left.name.to_lowercase().cmp(&right.name.to_lowercase())
                        });
                        picker.browse = Some(OpenWithBrowse::Ready(applications));
                        picker.selected = 0;
                    }
                    Err(error) => {
                        picker.browse = None;
                        this.operation_error =
                            Some(format!("Could not load the application catalog: {error}").into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Back out of the "Choose Application…" browse to the ordinary picker.
    pub(super) fn cancel_choose_application(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = self.open_with.as_mut() else {
            return;
        };
        if picker.busy || picker.browse.is_none() {
            return;
        }
        picker.browse = None;
        picker.selected = 0;
        picker.error = None;
        cx.notify();
    }

    pub(super) fn move_open_with_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(picker) = self.open_with.as_mut() else {
            return;
        };
        if picker.busy {
            return;
        }
        let len = match &picker.browse {
            Some(OpenWithBrowse::Ready(applications)) => applications.len(),
            Some(OpenWithBrowse::Loading) => 0,
            None => picker
                .association
                .as_ref()
                .map_or(0, |association| association.handlers.len()),
        };
        if len == 0 {
            return;
        }
        picker.selected = picker.selected.saturating_add_signed(delta).min(len - 1);
        cx.notify();
    }

    pub(super) fn choose_open_with(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(picker) = self.open_with.as_mut() else {
            return;
        };
        let Some(association) = picker.association.as_ref() else {
            return;
        };
        if !picker.busy && index < association.handlers.len() {
            picker.selected = index;
            picker.error = None;
            cx.notify();
        }
    }

    pub(super) fn choose_browse_application(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(picker) = self.open_with.as_mut() else {
            return;
        };
        let Some(OpenWithBrowse::Ready(applications)) = picker.browse.as_ref() else {
            return;
        };
        if !picker.busy && index < applications.len() {
            picker.selected = index;
            picker.error = None;
            cx.notify();
        }
    }

    pub(super) fn toggle_open_with_default(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = self.open_with.as_mut() else {
            return;
        };
        if picker.browse.is_some() {
            // A forced, uncategorized open can't be recorded as an XDG
            // default; see the note next to the hidden toggle in the render.
            return;
        }
        let Some(association) = picker.association.as_ref() else {
            return;
        };
        let Some(application) = association.handlers.get(picker.selected) else {
            return;
        };
        if !picker.busy
            && association.default_application_id.as_deref() != Some(application.id.as_str())
        {
            picker.make_default = !picker.make_default;
            picker.error = None;
            cx.notify();
        }
    }

    pub(super) fn confirm_open_with(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = self.open_with.as_mut() else {
            return;
        };
        if picker.busy {
            return;
        }

        let chosen = match &picker.browse {
            Some(OpenWithBrowse::Ready(applications)) => {
                let Some(application) = applications.get(picker.selected) else {
                    return;
                };
                let Some(association) = picker.association.as_ref() else {
                    return;
                };
                ChosenApplication {
                    mime_type: association.mime_type.clone(),
                    application_id: application.id.clone(),
                    application_name: sanitize_dialog_name(&application.name),
                    make_default: false,
                    force: true,
                }
            }
            Some(OpenWithBrowse::Loading) => return,
            None => {
                let Some(association) = picker.association.as_ref() else {
                    return;
                };
                let Some(application) = association.handlers.get(picker.selected) else {
                    return;
                };
                ChosenApplication {
                    mime_type: association.mime_type.clone(),
                    application_id: application.id.clone(),
                    application_name: sanitize_dialog_name(&application.name),
                    make_default: picker.make_default
                        && association.default_application_id.as_deref()
                            != Some(application.id.as_str()),
                    force: false,
                }
            }
        };

        let path = picker.path.clone();
        picker.busy = true;
        picker.error = None;
        self.operation_error = None;
        cx.notify();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = rmac_app_launch::open_file_with(
                path.clone(),
                chosen.mime_type.clone(),
                chosen.application_id.clone(),
                chosen.make_default,
                chosen.force,
            )
            .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                let Some(picker) = this.open_with.as_mut().filter(|picker| picker.path == path)
                else {
                    return;
                };
                picker.busy = false;
                match result {
                    Ok(()) => {
                        this.open_with = None;
                        this.operation_notice = Some(
                            if chosen.make_default {
                                format!(
                                    "{} is now the default for {} files",
                                    chosen.application_name, chosen.mime_type
                                )
                            } else {
                                format!("Opened with {}", chosen.application_name)
                            }
                            .into(),
                        );
                    }
                    Err(error) => {
                        if error.default_changed {
                            if let Some(association) = picker.association.as_mut() {
                                association.default_application_id =
                                    Some(chosen.application_id.clone());
                            }
                            picker.make_default = false;
                        }
                        picker.error = Some(format!("Could not open the file: {error}").into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

struct ChosenApplication {
    mime_type: String,
    application_id: String,
    application_name: String,
    make_default: bool,
    force: bool,
}
