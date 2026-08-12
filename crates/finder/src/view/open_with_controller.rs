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
        let mut selected = self
            .selected
            .iter()
            .filter_map(|&index| self.entries.get(index));
        let Some(entry) = selected.next() else {
            return;
        };
        if selected.next().is_some() || entry.is_dir {
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

    pub(super) fn move_open_with_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(picker) = self.open_with.as_mut() else {
            return;
        };
        let Some(association) = picker.association.as_ref() else {
            return;
        };
        if picker.busy || association.handlers.is_empty() {
            return;
        }
        picker.selected = picker
            .selected
            .saturating_add_signed(delta)
            .min(association.handlers.len() - 1);
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

    pub(super) fn toggle_open_with_default(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = self.open_with.as_mut() else {
            return;
        };
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
        let Some(association) = picker.association.as_ref() else {
            return;
        };
        let Some(application) = association.handlers.get(picker.selected) else {
            return;
        };
        if picker.busy {
            return;
        }
        let path = picker.path.clone();
        let mime_type = association.mime_type.clone();
        let application_id = application.id.clone();
        let application_name = sanitize_dialog_name(&application.name);
        let make_default = picker.make_default
            && association.default_application_id.as_deref() != Some(application.id.as_str());
        picker.busy = true;
        picker.error = None;
        self.operation_error = None;
        cx.notify();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = rmac_app_launch::open_file_with(
                path.clone(),
                mime_type.clone(),
                application_id.clone(),
                make_default,
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
                            if make_default {
                                format!(
                                    "{application_name} is now the default for {mime_type} files"
                                )
                            } else {
                                format!("Opened with {application_name}")
                            }
                            .into(),
                        );
                    }
                    Err(error) => {
                        if error.default_changed {
                            if let Some(association) = picker.association.as_mut() {
                                association.default_application_id = Some(application_id.clone());
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
