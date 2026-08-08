//! Bounded dialog-stack and live-region semantics for Files.

use std::collections::HashSet;

pub const MAX_ACCESSIBLE_DIALOGS: usize = 8;
pub const MAX_ACCESSIBLE_DIALOG_ACTIONS: usize = 16;
pub const MAX_ACCESSIBLE_DIALOG_OPTIONS: usize = 4_096;
pub const MAX_ACCESSIBLE_DOCUMENT_BYTES: usize = 64 * 1024;
pub const MAX_ACCESSIBLE_TEXT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_ACCESSIBLE_LABEL_BYTES: usize = 4 * 1024;
pub const MAX_ACCESSIBLE_OPTION_ID_BYTES: usize = 512;
pub const MAX_ACCESSIBLE_GALLERY_ITEMS: usize = 4_096;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum DialogKind {
    GetInfo,
    Conflict,
    Recovery,
    TrashRecovery,
    PermanentDelete,
    OpenWith,
    QuickLook,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogActionKind {
    Normal,
    Default,
    Destructive,
    Toggle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibleDialogAction {
    pub id: String,
    pub name: String,
    pub kind: DialogActionKind,
    pub enabled: bool,
    pub busy: bool,
    pub checked: Option<bool>,
}

impl AccessibleDialogAction {
    pub fn new(id: impl Into<String>, name: impl Into<String>, kind: DialogActionKind) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            kind,
            enabled: true,
            busy: false,
            checked: None,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.enabled = !disabled;
        self
    }

    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        if busy {
            self.enabled = false;
        }
        self
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibleDialogOption {
    pub stable_id: String,
    pub name: String,
    pub selected: bool,
    pub is_default: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogFocus {
    Action(usize),
    Option(usize),
    Document,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LivePoliteness {
    Polite,
    Assertive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressUnit {
    Items,
    Bytes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccessibleProgress {
    pub current: u64,
    pub total: u64,
    pub unit: ProgressUnit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibleLiveRegion {
    pub id: String,
    pub text: String,
    pub politeness: LivePoliteness,
    pub progress: Option<AccessibleProgress>,
    pub actions: Vec<AccessibleDialogAction>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibleDialog {
    pub kind: DialogKind,
    pub title: String,
    pub description: String,
    pub actions: Vec<AccessibleDialogAction>,
    pub options: Vec<AccessibleDialogOption>,
    pub initial_focus: DialogFocus,
    /// Visible document text, currently used only by Quick Look text previews.
    pub document_text: Option<String>,
    /// Dialog-local loading or error feedback announced when it changes.
    pub status: Option<AccessibleLiveRegion>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibleGalleryItem {
    pub stable_id: String,
    pub name: String,
    pub description: String,
    pub selected: bool,
    pub is_directory: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibleGallery {
    /// Filmstrip items in the same filtered order as the visible renderer.
    pub items: Vec<AccessibleGalleryItem>,
    /// Active large preview, indexed into `items`.
    pub active_item: Option<usize>,
    pub selection_count: usize,
    pub preview_name: Option<String>,
    pub preview_description: String,
    pub actions: Vec<AccessibleDialogAction>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilesAccessibilitySnapshot {
    /// Present only while the distinct Gallery view is active.
    pub gallery: Option<AccessibleGallery>,
    /// Dialogs in visual stacking order. The last entry is the active modal.
    pub dialogs: Vec<AccessibleDialog>,
    pub active_dialog: Option<usize>,
    /// Root-window feedback in the same order it is rendered.
    pub live_regions: Vec<AccessibleLiveRegion>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessibilityProjectionError {
    DialogLimit,
    DuplicateDialog,
    ActionLimit,
    OptionLimit,
    InvalidText,
    InvalidAction,
    DuplicateAction,
    InvalidOptions,
    InvalidFocus,
    InvalidProgress,
    GalleryLimit,
    InvalidGallery,
    TextLimit,
}

/// Validate and retain the exact renderer/controller projection. Adapters pass
/// only already-visible text and stable option IDs, never hidden operation
/// records, launch specifications, or query text.
pub fn project_files_accessibility(
    gallery: Option<AccessibleGallery>,
    dialogs: Vec<AccessibleDialog>,
    live_regions: Vec<AccessibleLiveRegion>,
) -> Result<FilesAccessibilitySnapshot, AccessibilityProjectionError> {
    if dialogs.len() > MAX_ACCESSIBLE_DIALOGS {
        return Err(AccessibilityProjectionError::DialogLimit);
    }
    let mut budget = TextBudget::default();
    if let Some(gallery) = &gallery {
        validate_gallery(gallery, &mut budget)?;
    }
    let mut kinds = HashSet::with_capacity(dialogs.len());
    for dialog in &dialogs {
        if !kinds.insert(dialog.kind) {
            return Err(AccessibilityProjectionError::DuplicateDialog);
        }
        validate_text(&dialog.title, &mut budget, false)?;
        validate_text(&dialog.description, &mut budget, true)?;
        validate_actions(&dialog.actions, &mut budget)?;
        validate_options(&dialog.options, &mut budget)?;
        match dialog.initial_focus {
            DialogFocus::Action(index) if index < dialog.actions.len() => {}
            DialogFocus::Option(index) if index < dialog.options.len() => {}
            DialogFocus::Document if dialog.document_text.is_some() => {}
            _ => return Err(AccessibilityProjectionError::InvalidFocus),
        }
        if let Some(document) = &dialog.document_text {
            if document.len() > MAX_ACCESSIBLE_DOCUMENT_BYTES {
                return Err(AccessibilityProjectionError::TextLimit);
            }
            if !document
                .chars()
                .all(|character| !character.is_control() || matches!(character, '\n' | '\r' | '\t'))
            {
                return Err(AccessibilityProjectionError::InvalidText);
            }
            budget.add(document)?;
        }
        if let Some(status) = &dialog.status {
            validate_live_region(status, &mut budget)?;
        }
    }
    for live_region in &live_regions {
        validate_live_region(live_region, &mut budget)?;
    }
    Ok(FilesAccessibilitySnapshot {
        gallery,
        active_dialog: dialogs.len().checked_sub(1),
        dialogs,
        live_regions,
    })
}

fn validate_gallery(
    gallery: &AccessibleGallery,
    budget: &mut TextBudget,
) -> Result<(), AccessibilityProjectionError> {
    if gallery.items.len() > MAX_ACCESSIBLE_GALLERY_ITEMS {
        return Err(AccessibilityProjectionError::GalleryLimit);
    }
    let mut ids = HashSet::with_capacity(gallery.items.len());
    let mut selected_count = 0usize;
    for item in &gallery.items {
        if !valid_id(&item.stable_id, MAX_ACCESSIBLE_OPTION_ID_BYTES) || !valid_label(&item.name) {
            return Err(AccessibilityProjectionError::InvalidGallery);
        }
        validate_text(&item.description, budget, false)?;
        if !ids.insert(item.stable_id.as_str()) {
            return Err(AccessibilityProjectionError::InvalidGallery);
        }
        selected_count = selected_count.saturating_add(usize::from(item.selected));
        budget.add(&item.stable_id)?;
        budget.add(&item.name)?;
    }
    if selected_count != gallery.selection_count
        || gallery
            .active_item
            .is_some_and(|index| index >= gallery.items.len() || !gallery.items[index].selected)
        || gallery.active_item.is_none() != gallery.preview_name.is_none()
    {
        return Err(AccessibilityProjectionError::InvalidGallery);
    }
    if let Some(name) = &gallery.preview_name {
        validate_text(name, budget, false)?;
    }
    validate_text(&gallery.preview_description, budget, false)?;
    validate_actions(&gallery.actions, budget)
}

fn validate_actions(
    actions: &[AccessibleDialogAction],
    budget: &mut TextBudget,
) -> Result<(), AccessibilityProjectionError> {
    if actions.is_empty() || actions.len() > MAX_ACCESSIBLE_DIALOG_ACTIONS {
        return Err(AccessibilityProjectionError::ActionLimit);
    }
    let mut ids = HashSet::with_capacity(actions.len());
    for action in actions {
        if !valid_id(&action.id, MAX_ACCESSIBLE_LABEL_BYTES)
            || !valid_label(&action.name)
            || action.busy && action.enabled
            || action.checked.is_some() && action.kind != DialogActionKind::Toggle
            || action.kind == DialogActionKind::Toggle && action.checked.is_none()
        {
            return Err(AccessibilityProjectionError::InvalidAction);
        }
        if !ids.insert(action.id.as_str()) {
            return Err(AccessibilityProjectionError::DuplicateAction);
        }
        budget.add(&action.id)?;
        budget.add(&action.name)?;
    }
    Ok(())
}

fn validate_options(
    options: &[AccessibleDialogOption],
    budget: &mut TextBudget,
) -> Result<(), AccessibilityProjectionError> {
    if options.len() > MAX_ACCESSIBLE_DIALOG_OPTIONS {
        return Err(AccessibilityProjectionError::OptionLimit);
    }
    let mut ids = HashSet::with_capacity(options.len());
    let mut selected = 0usize;
    for option in options {
        if !valid_id(&option.stable_id, MAX_ACCESSIBLE_OPTION_ID_BYTES)
            || !valid_label(&option.name)
        {
            return Err(AccessibilityProjectionError::InvalidOptions);
        }
        if !ids.insert(option.stable_id.as_str()) {
            return Err(AccessibilityProjectionError::InvalidOptions);
        }
        selected = selected.saturating_add(usize::from(option.selected));
        budget.add(&option.stable_id)?;
        budget.add(&option.name)?;
    }
    if !options.is_empty() && selected != 1 {
        return Err(AccessibilityProjectionError::InvalidOptions);
    }
    Ok(())
}

fn validate_live_region(
    live_region: &AccessibleLiveRegion,
    budget: &mut TextBudget,
) -> Result<(), AccessibilityProjectionError> {
    if !valid_id(&live_region.id, MAX_ACCESSIBLE_LABEL_BYTES) {
        return Err(AccessibilityProjectionError::InvalidText);
    }
    validate_text(&live_region.text, budget, true)?;
    budget.add(&live_region.id)?;
    if let Some(progress) = live_region.progress {
        if progress.total == 0 || progress.current > progress.total {
            return Err(AccessibilityProjectionError::InvalidProgress);
        }
    }
    if !live_region.actions.is_empty() {
        validate_actions(&live_region.actions, budget)?;
    }
    Ok(())
}

fn validate_text(
    value: &str,
    budget: &mut TextBudget,
    multiline: bool,
) -> Result<(), AccessibilityProjectionError> {
    let valid_controls = |character: char| {
        !character.is_control() || multiline && matches!(character, '\n' | '\r' | '\t')
    };
    if value.trim().is_empty()
        || value.len() > MAX_ACCESSIBLE_TEXT_BYTES
        || !value.chars().all(valid_controls)
    {
        return Err(AccessibilityProjectionError::InvalidText);
    }
    budget.add(value)
}

fn valid_label(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= MAX_ACCESSIBLE_LABEL_BYTES
        && !value.chars().any(char::is_control)
}

fn valid_id(value: &str, limit: usize) -> bool {
    !value.is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}

#[derive(Default)]
struct TextBudget {
    bytes: usize,
}

impl TextBudget {
    fn add(&mut self, value: &str) -> Result<(), AccessibilityProjectionError> {
        self.bytes = self.bytes.saturating_add(value.len());
        if self.bytes > MAX_ACCESSIBLE_TEXT_BYTES {
            Err(AccessibilityProjectionError::TextLimit)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(id: &str, name: &str, kind: DialogActionKind) -> AccessibleDialogAction {
        AccessibleDialogAction::new(id, name, kind)
    }

    fn dialog(kind: DialogKind, title: &str) -> AccessibleDialog {
        AccessibleDialog {
            kind,
            title: title.to_string(),
            description: "A truthful description".to_string(),
            actions: vec![
                action("cancel", "Cancel", DialogActionKind::Normal),
                action("confirm", "Continue", DialogActionKind::Default),
            ],
            options: Vec::new(),
            initial_focus: DialogFocus::Action(0),
            document_text: None,
            status: None,
        }
    }

    #[test]
    fn dialog_stack_preserves_safe_focus_destructive_state_and_active_modal() {
        let mut delete = dialog(DialogKind::PermanentDelete, "Delete Item Permanently?");
        delete.actions[1] = action("delete", "Delete", DialogActionKind::Destructive);
        let open_with = AccessibleDialog {
            kind: DialogKind::OpenWith,
            title: "Open With".to_string(),
            description: "Choose an application for “report.txt” (text/plain)".to_string(),
            actions: vec![
                action("cancel", "Cancel", DialogActionKind::Normal),
                action("open", "Open", DialogActionKind::Default),
                action(
                    "make-default",
                    "Always use this application",
                    DialogActionKind::Toggle,
                )
                .checked(false),
            ],
            options: vec![
                AccessibleDialogOption {
                    stable_id: "editor.desktop".to_string(),
                    name: "Editor".to_string(),
                    selected: true,
                    is_default: false,
                },
                AccessibleDialogOption {
                    stable_id: "viewer.desktop".to_string(),
                    name: "Viewer".to_string(),
                    selected: false,
                    is_default: true,
                },
            ],
            initial_focus: DialogFocus::Option(0),
            document_text: None,
            status: None,
        };

        let snapshot =
            project_files_accessibility(None, vec![delete, open_with], Vec::new()).unwrap();
        assert_eq!(snapshot.active_dialog, Some(1));
        assert_eq!(snapshot.dialogs[0].initial_focus, DialogFocus::Action(0));
        assert_eq!(
            snapshot.dialogs[0].actions[1].kind,
            DialogActionKind::Destructive
        );
        assert_eq!(snapshot.dialogs[1].initial_focus, DialogFocus::Option(0));
        assert!(snapshot.dialogs[1].options[0].selected);
        assert!(snapshot.dialogs[1].options[1].is_default);
    }

    #[test]
    fn live_regions_retain_render_order_priority_progress_and_cancel_state() {
        let snapshot = project_files_accessibility(
            None,
            Vec::new(),
            vec![
                AccessibleLiveRegion {
                    id: "operation-notice".to_string(),
                    text: "Copy completed".to_string(),
                    politeness: LivePoliteness::Polite,
                    progress: None,
                    actions: vec![action("dismiss", "Dismiss", DialogActionKind::Normal)],
                },
                AccessibleLiveRegion {
                    id: "operation-error".to_string(),
                    text: "Copy failed safely".to_string(),
                    politeness: LivePoliteness::Assertive,
                    progress: None,
                    actions: vec![action("review", "Review", DialogActionKind::Default)],
                },
                AccessibleLiveRegion {
                    id: "transfer-progress".to_string(),
                    text: "Copying — 2 of 4 items".to_string(),
                    politeness: LivePoliteness::Polite,
                    progress: Some(AccessibleProgress {
                        current: 2,
                        total: 4,
                        unit: ProgressUnit::Items,
                    }),
                    actions: vec![action("cancel", "Cancel", DialogActionKind::Normal)],
                },
            ],
        )
        .unwrap();

        assert_eq!(snapshot.live_regions.len(), 3);
        assert_eq!(snapshot.live_regions[0].politeness, LivePoliteness::Polite);
        assert_eq!(
            snapshot.live_regions[1].politeness,
            LivePoliteness::Assertive
        );
        assert_eq!(snapshot.live_regions[2].progress.unwrap().current, 2);
    }

    #[test]
    fn invalid_focus_duplicates_busy_actions_and_oversized_documents_fail_closed() {
        let mut invalid = dialog(DialogKind::Conflict, "Conflict");
        invalid.initial_focus = DialogFocus::Action(9);
        assert_eq!(
            project_files_accessibility(None, vec![invalid], Vec::new()),
            Err(AccessibilityProjectionError::InvalidFocus)
        );

        let duplicates = vec![
            dialog(DialogKind::Recovery, "One"),
            dialog(DialogKind::Recovery, "Two"),
        ];
        assert_eq!(
            project_files_accessibility(None, duplicates, Vec::new()),
            Err(AccessibilityProjectionError::DuplicateDialog)
        );

        let mut busy = dialog(DialogKind::Conflict, "Conflict");
        busy.actions[1].busy = true;
        assert_eq!(
            project_files_accessibility(None, vec![busy], Vec::new()),
            Err(AccessibilityProjectionError::InvalidAction)
        );

        let mut preview = dialog(DialogKind::QuickLook, "Preview");
        preview.document_text = Some("x".repeat(MAX_ACCESSIBLE_DOCUMENT_BYTES + 1));
        preview.initial_focus = DialogFocus::Document;
        assert_eq!(
            project_files_accessibility(None, vec![preview], Vec::new()),
            Err(AccessibilityProjectionError::TextLimit)
        );
    }

    #[test]
    fn gallery_preserves_filmstrip_preview_selection_and_actions() {
        let gallery = AccessibleGallery {
            items: vec![
                AccessibleGalleryItem {
                    stable_id: "gallery-item-0".into(),
                    name: "Pictures".into(),
                    description: "Folder · 3 items".into(),
                    selected: false,
                    is_directory: true,
                },
                AccessibleGalleryItem {
                    stable_id: "gallery-item-1".into(),
                    name: "Sunset.jpg".into(),
                    description: "JPEG image · 2 MB".into(),
                    selected: true,
                    is_directory: false,
                },
            ],
            active_item: Some(1),
            selection_count: 1,
            preview_name: Some("Sunset.jpg".into()),
            preview_description: "JPEG image · 2 MB · Today".into(),
            actions: vec![action("gallery-open", "Open", DialogActionKind::Default)],
        };

        let snapshot = project_files_accessibility(Some(gallery), Vec::new(), Vec::new()).unwrap();
        let gallery = snapshot.gallery.unwrap();
        assert_eq!(gallery.active_item, Some(1));
        assert_eq!(gallery.selection_count, 1);
        assert!(gallery.items[0].is_directory);
        assert!(gallery.items[1].selected);
        assert_eq!(gallery.actions[0].name, "Open");
    }

    #[test]
    fn gallery_rejects_duplicate_items_and_preview_selection_mismatch() {
        let mut gallery = AccessibleGallery {
            items: vec![AccessibleGalleryItem {
                stable_id: "gallery-item-0".into(),
                name: "One".into(),
                description: "Text document".into(),
                selected: false,
                is_directory: false,
            }],
            active_item: Some(0),
            selection_count: 0,
            preview_name: Some("One".into()),
            preview_description: "Text document".into(),
            actions: vec![action("gallery-open", "Open", DialogActionKind::Default)],
        };
        assert_eq!(
            project_files_accessibility(Some(gallery.clone()), Vec::new(), Vec::new()),
            Err(AccessibilityProjectionError::InvalidGallery)
        );

        gallery.active_item = None;
        gallery.preview_name = None;
        gallery.items.push(gallery.items[0].clone());
        assert_eq!(
            project_files_accessibility(Some(gallery), Vec::new(), Vec::new()),
            Err(AccessibilityProjectionError::InvalidGallery)
        );
    }
}
