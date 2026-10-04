//! File ▸ Rename…/Move To…/Revert To ▸ Last Saved/Page Setup… and
//! Application ▸ Quit and Keep Windows (TXT-MENU-001/002/003/004/006).

use super::*;

impl EditorView {
    /// File ▸ Rename… — only a saved, path-backed document has a name to
    /// change; an Untitled window uses Save As instead, as on the Mac.
    pub(super) fn rename_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.path.is_none() || self.file_action_blocked() {
            return;
        }
        let name = self.filename().to_string();
        self.rename_input.update(cx, |input, cx| {
            input.set_value(name.clone(), window, cx);
            input.focus(window, cx);
            input.set_selected_range(0..name.len(), cx);
        });
        self.rename_error = None;
        self.rename_open = true;
        cx.notify();
    }

    pub(super) fn cancel_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.rename_open = false;
        self.rename_error = None;
        self.input.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    pub(super) fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.rename_open || self.rename_busy {
            return;
        }
        let Some(current) = self.path.clone() else {
            return;
        };
        let name = self.rename_input.read(cx).text().to_string();
        let name = name.trim();
        if !is_valid_document_name(name) {
            self.rename_error = Some("Choose a valid document name.".into());
            cx.notify();
            return;
        }
        let Some(destination) = current.parent().map(|parent| parent.join(name)) else {
            return;
        };
        if destination == current {
            self.rename_open = false;
            self.input.update(cx, |state, cx| state.focus(window, cx));
            cx.notify();
            return;
        }
        self.rename_busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let moved = cx
                .background_executor()
                .spawn({
                    let destination = destination.clone();
                    async move { std::fs::rename(&current, &destination) }
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.rename_busy = false;
                match moved {
                    Ok(()) => {
                        this.path = Some(destination);
                        this.rename_open = false;
                        this.rename_error = None;
                        this.reset_document_watch();
                        this.record_current_document(cx);
                        this.input.update(cx, |state, cx| state.focus(window, cx));
                    }
                    Err(_) => {
                        this.rename_error = Some("The document could not be renamed.".into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// File ▸ Move To… — the Mac's folder-picker move, backed by the same
    /// desktop portal `Open…` uses with `directories: true`.
    pub(super) fn move_to_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(current) = self.path.clone() else {
            return;
        };
        if self.move_busy || self.file_action_blocked() {
            return;
        }
        self.move_busy = true;
        cx.notify();
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Move".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let picker = receiver.await;
            let Ok(Ok(Some(mut folders))) = picker else {
                let _ = this.update_in(cx, |this, _, cx| {
                    this.move_busy = false;
                    cx.notify();
                });
                return;
            };
            let Some(folder) = folders.pop() else {
                let _ = this.update_in(cx, |this, _, cx| {
                    this.move_busy = false;
                    cx.notify();
                });
                return;
            };
            let Some(name) = current.file_name().map(std::ffi::OsStr::to_owned) else {
                let _ = this.update_in(cx, |this, _, cx| {
                    this.move_busy = false;
                    cx.notify();
                });
                return;
            };
            let destination = folder.join(&name);
            if destination == current {
                let _ = this.update_in(cx, |this, _, cx| {
                    this.move_busy = false;
                    cx.notify();
                });
                return;
            }
            let moved = cx
                .background_executor()
                .spawn({
                    let destination = destination.clone();
                    async move { std::fs::rename(&current, &destination) }
                })
                .await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.move_busy = false;
                match moved {
                    Ok(()) => {
                        this.path = Some(destination);
                        this.reset_document_watch();
                        this.record_current_document(cx);
                    }
                    Err(_) => {
                        this.alert = Some(ActiveAlert::Error {
                            title: "Could not move the document.",
                            message: "The chosen folder could not be written to.".into(),
                        });
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// File ▸ Revert To ▸ Last Saved — only a path-backed document with
    /// unsaved changes has anything to revert; "Browse All Versions…" is
    /// omitted (Linux has no Time Machine-style version store).
    pub(super) fn revert_to_last_saved(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.path.is_none()
            || !self.dirty
            || self.file_action_blocked()
            || self.rtf_runs.is_some()
        {
            return;
        }
        self.alert = Some(ActiveAlert::ConfirmRevert);
        cx.notify();
    }

    pub(super) fn perform_revert(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.path.clone() {
            self.load_document_path(path, "The document could not be reverted.", window, cx);
        }
    }

    /// File ▸ Page Setup… — a persistent default page description for
    /// Export as PDF (TE-04); the Linux print portal negotiates its own
    /// page settings each time, so this does not also reopen that dialog.
    pub(super) fn open_page_setup(&mut self, cx: &mut Context<Self>) {
        if self.file_action_blocked() {
            return;
        }
        self.page_setup_before = Some((self.page_setup_letter, self.page_setup_landscape));
        self.page_setup_open = true;
        cx.notify();
    }

    pub(super) fn cancel_page_setup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((letter, landscape)) = self.page_setup_before.take() {
            self.page_setup_letter = letter;
            self.page_setup_landscape = landscape;
        }
        self.page_setup_open = false;
        self.input.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    pub(super) fn close_page_setup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.page_setup_before = None;
        self.page_setup_open = false;
        self.input.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    pub(super) fn set_page_setup_paper(&mut self, letter: bool, cx: &mut Context<Self>) {
        self.page_setup_letter = letter;
        cx.notify();
    }

    pub(super) fn set_page_setup_orientation(&mut self, landscape: bool, cx: &mut Context<Self>) {
        self.page_setup_landscape = landscape;
        cx.notify();
    }

    /// The `PageLayout` Export as PDF renders with, from this document's
    /// own Page Setup choice.
    pub(super) fn page_layout(&self) -> rmac_print::PageLayout {
        page_layout_for(self.page_setup_letter, self.page_setup_landscape)
    }

    /// Application ▸ Quit and Keep Windows (⌥⌘Q) — unlike a plain Quit,
    /// this records every saved document window this process has open so
    /// the next launch reopens them, regardless of whether they had unsaved
    /// changes (those already come back through the recovery store). Each
    /// window still closes through its own unsaved-changes guard.
    pub(super) fn quit_and_keep_windows(&mut self, cx: &mut Context<Self>) {
        let paths = startup::open_document_paths(cx);
        if let Err(error) = recovery_state::save_kept_session(&paths) {
            eprintln!("Text Editor could not record windows to keep ({error})");
        }
        rmac_ui::quit_application(cx);
    }
}

/// File ▸ Rename…'s name field: non-empty, no path separator, and not a
/// directory reference.
fn is_valid_document_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/')
}

/// File ▸ Page Setup…'s paper size (A4/US Letter) and orientation
/// (Portrait/Landscape) as the `PageLayout` Export as PDF renders with.
fn page_layout_for(letter: bool, landscape: bool) -> rmac_print::PageLayout {
    let (mut width_mm, mut height_mm) = if letter {
        (215.9, 279.4)
    } else {
        (210.0, 297.0)
    };
    if landscape {
        std::mem::swap(&mut width_mm, &mut height_mm);
    }
    rmac_print::PageLayout {
        width_mm,
        height_mm,
        ..rmac_print::PageLayout::default()
    }
}

#[cfg(test)]
mod tests {
    use super::{is_valid_document_name, page_layout_for};

    #[test]
    fn document_names_reject_empty_dot_and_path_separators() {
        assert!(is_valid_document_name("notes.txt"));
        assert!(is_valid_document_name("My Document"));
        assert!(!is_valid_document_name(""));
        assert!(!is_valid_document_name("."));
        assert!(!is_valid_document_name(".."));
        assert!(!is_valid_document_name("a/b"));
    }

    #[test]
    fn page_setup_chooses_a4_or_letter_in_the_requested_orientation() {
        let a4_portrait = page_layout_for(false, false);
        assert_eq!(
            (a4_portrait.width_mm, a4_portrait.height_mm),
            (210.0, 297.0)
        );

        let a4_landscape = page_layout_for(false, true);
        assert_eq!(
            (a4_landscape.width_mm, a4_landscape.height_mm),
            (297.0, 210.0)
        );

        let letter_portrait = page_layout_for(true, false);
        assert_eq!(
            (letter_portrait.width_mm, letter_portrait.height_mm),
            (215.9, 279.4)
        );

        let letter_landscape = page_layout_for(true, true);
        assert_eq!(
            (letter_landscape.width_mm, letter_landscape.height_mm),
            (279.4, 215.9)
        );

        // Only the page geometry changes; margins and DPI keep the
        // renderer's own defaults.
        let default_layout = rmac_print::PageLayout::default();
        assert_eq!(a4_portrait.margin_top_mm, default_layout.margin_top_mm);
        assert_eq!(a4_portrait.dpi, default_layout.dpi);
    }
}
