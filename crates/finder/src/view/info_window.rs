//! Get Info (⌘I): one non-modal window per item, laid out as macOS 26
//! Finder's (tests/robot/mac/Finder.json, "item/File/Get Info"): the header,
//! the Tags field, then the collapsible General, More Info, Name &
//! Extension, Comments, Open with, Preview and Sharing & Permissions
//! sections (FILES-15, FILES-18, FILES-19).
//!
//! Every change is written off the UI thread: the colour tag and comment
//! are extended attributes (NTFS streams on Windows), Locked clears the
//! write permission, Open with stores a per-file choice that Files honours
//! when it opens the file, Change All… sets the XDG default for the type,
//! and the privilege pop-ups change the POSIX mode bits.

use std::sync::atomic::AtomicU8;

use super::file_metadata::{
    apply_file_tag, file_tags, read_file_comment, tag_colour, tag_display_name,
    write_file_comment,
};
use super::search_info_controller::{
    format_folder_size, info_artwork, info_block, info_body, info_card, info_header, info_rows,
    info_title_strip, scan_folder_size, MAX_FOLDER_INFO_ENTRIES,
};
use super::*;
use rmac_ui::{Checkbox, PopUpButton, PopupMenuItem};

/// The disclosure sections of an Info window, in Finder's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InfoSection {
    General,
    MoreInfo,
    NameExtension,
    Comments,
    OpenWith,
    Preview,
    Permissions,
}

impl InfoSection {
    fn bit(self) -> u8 {
        1 << (self as u8)
    }

    pub(super) fn title(self) -> &'static str {
        match self {
            Self::General => "General:",
            Self::MoreInfo => "More Info:",
            Self::NameExtension => "Name & Extension:",
            Self::Comments => "Comments:",
            Self::OpenWith => "Open with:",
            Self::Preview => "Preview:",
            Self::Permissions => "Sharing & Permissions:",
        }
    }
}

/// Which sections are collapsed. Like Finder, a section closed in one Info
/// window stays closed in the next one opened.
static COLLAPSED_SECTIONS: AtomicU8 = AtomicU8::new(0);

pub(super) fn section_open(section: InfoSection) -> bool {
    COLLAPSED_SECTIONS.load(Ordering::Relaxed) & section.bit() == 0
}

fn toggle_section(section: InfoSection) {
    COLLAPSED_SECTIONS.fetch_xor(section.bit(), Ordering::Relaxed);
}

/// A section's disclosure heading. Clicking it (or its accessible action)
/// opens or closes the section in every Info window.
pub(super) fn info_section_header(section: InfoSection) -> Stateful<Div> {
    let open = section_open(section);
    div()
        .id(SharedString::from(format!("info-section-{}", section as u8)))
        .role(Role::Button)
        .aria_label(section.title().trim_end_matches(':').to_owned())
        .aria_expanded(open)
        .h(px(INFO_SECTION_HEADER))
        .flex()
        .items_center()
        .gap(px(4.0))
        .text_size(rmac_ui::text_px(INFO_SECTION_TEXT))
        .text_color(label())
        .cursor_pointer()
        .child(icon(
            if open {
                "icons/chevron-down.svg"
            } else {
                "icons/chevron-right.svg"
            },
            10.0,
            secondary_text(),
        ))
        .child(section.title())
        .on_click(move |_, window, _| {
            toggle_section(section);
            window.refresh();
        })
        .on_a11y_action(AccessibleAction::Click, move |_, window, _| {
            toggle_section(section);
            window.refresh();
        })
}

/// One ruled, collapsible Info section; `rows` only build while it is open.
pub(super) fn info_disclosure<I: IntoElement>(
    section: InfoSection,
    rows: impl FnOnce() -> Vec<I>,
) -> Div {
    let block = info_block().child(info_section_header(section));
    if section_open(section) {
        block.children(rows())
    } else {
        block
    }
}

/// The General section: kind, size, location and dates.
pub(super) fn info_general(details: &[(&'static str, String)]) -> Div {
    info_disclosure(InfoSection::General, || {
        info_rows(
            details,
            &["Kind", "Size", "Where", "Created", "Modified"],
            INFO_LABEL_RIGHT - INFO_SECTION_INSET,
        )
    })
}

/// The read-only Sharing & Permissions rows the Inspector shows.
pub(super) fn info_permissions(details: &[(&'static str, String)]) -> Div {
    info_disclosure(InfoSection::Permissions, || {
        info_rows(
            details,
            &["Owner", "Group", "Permissions"],
            INFO_PERMISSIONS_LABEL,
        )
    })
}

/// One row of Finder's Sharing & Permissions table.
#[cfg_attr(not(unix), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Privilege {
    ReadWrite,
    ReadOnly,
    WriteOnly,
    NoAccess,
}

#[cfg_attr(not(unix), allow(dead_code))]
impl Privilege {
    const ALL: [Privilege; 4] = [
        Self::ReadWrite,
        Self::ReadOnly,
        Self::WriteOnly,
        Self::NoAccess,
    ];

    /// The privilege one `rwx` triplet grants.
    pub(super) fn from_bits(bits: u32) -> Self {
        match (bits & 0o4 != 0, bits & 0o2 != 0) {
            (true, true) => Self::ReadWrite,
            (true, false) => Self::ReadOnly,
            (false, true) => Self::WriteOnly,
            (false, false) => Self::NoAccess,
        }
    }

    pub(super) fn label(self, is_dir: bool) -> &'static str {
        match self {
            Self::ReadWrite => "Read & Write",
            Self::ReadOnly => "Read only",
            Self::WriteOnly if is_dir => "Write only (Drop Box)",
            Self::WriteOnly => "Write only",
            Self::NoAccess => "No Access",
        }
    }
}

/// `mode` with the triplet at `shift` (6 owner, 3 group, 0 everyone) set to
/// `privilege`. A folder's search bit follows read or write access, as
/// Finder sets it; a file keeps its execute bit.
#[cfg_attr(not(unix), allow(dead_code))]
pub(super) fn with_privilege(mode: u32, shift: u32, privilege: Privilege, is_dir: bool) -> u32 {
    let old = (mode >> shift) & 0o7;
    let mut bits = match privilege {
        Privilege::ReadWrite => 0o6,
        Privilege::ReadOnly => 0o4,
        Privilege::WriteOnly => 0o2,
        Privilege::NoAccess => 0o0,
    };
    if is_dir {
        if bits != 0 {
            bits |= 0o1;
        }
    } else {
        bits |= old & 0o1;
    }
    (mode & !(0o7 << shift)) | (bits << shift)
}

/// Finder's summary line above the permissions table.
pub(super) fn access_summary(read: bool, write: bool) -> &'static str {
    match (read, write) {
        (true, true) => "You can read and write",
        (true, false) => "You can only read",
        (false, true) => "You can only write",
        (false, false) => "You have no access",
    }
}

/// Locked: no write permission for anyone (the read-only attribute on
/// Windows), so the item cannot be changed until it is unlocked.
fn set_locked(path: &Path, locked: bool) -> Result<(), String> {
    let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
    let mut permissions = metadata.permissions();
    #[cfg(unix)]
    {
        let mode = permissions.mode();
        // Unlocking gives back only the owner's write access, never write
        // access for everyone, which `set_readonly(false)` would grant.
        permissions.set_mode(if locked { mode & !0o222 } else { mode | 0o200 });
    }
    #[cfg(not(unix))]
    permissions.set_readonly(locked);
    std::fs::set_permissions(path, permissions).map_err(|error| error.to_string())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), String> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|error| error.to_string())
}

/// Who may do what with one item, read when the window opens and after
/// every change.
#[derive(Clone, Debug, Default, PartialEq)]
struct Access {
    locked: bool,
    can_read: bool,
    can_write: bool,
    #[cfg(unix)]
    unix: Option<UnixAccess>,
}

#[cfg(unix)]
#[derive(Clone, Debug, PartialEq)]
struct UnixAccess {
    mode: u32,
    owner: String,
    group: String,
    /// The current user owns the item, so may change its privileges.
    mine: bool,
}

fn read_access(path: &Path) -> Access {
    let Ok(metadata) = std::fs::metadata(path) else {
        return Access::default();
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let mode = metadata.mode() & 0o7777;
        // SAFETY: geteuid has no preconditions and cannot fail.
        let euid = unsafe { libc::geteuid() };
        let uid = metadata.uid();
        let gid = metadata.gid();
        Access {
            locked: mode & 0o222 == 0,
            can_read: rustix::fs::access(path, rustix::fs::Access::READ_OK).is_ok(),
            can_write: rustix::fs::access(path, rustix::fs::Access::WRITE_OK).is_ok(),
            unix: Some(UnixAccess {
                mode,
                owner: super::filesystem_helpers::account_name(uid),
                group: super::filesystem_helpers::group_account_name(gid),
                mine: uid == euid,
            }),
        }
    }
    #[cfg(not(unix))]
    {
        let locked = metadata.permissions().readonly();
        Access {
            locked,
            can_read: true,
            can_write: !locked,
        }
    }
}

/// Get Info ▸ Open with: the type's compatible applications and this
/// file's own choice (Linux; other platforms leave the section out).
#[derive(Clone, Debug, PartialEq)]
struct OpenWithInfo {
    association: rmac_apps::FileAssociation,
    /// The application this file opens with: its own choice, else the
    /// type's default.
    chosen: Option<String>,
    confirm_change_all: bool,
    busy: bool,
}

impl OpenWithInfo {
    fn chosen_name(&self) -> String {
        let Some(chosen) = &self.chosen else {
            return "None".to_owned();
        };
        self.association
            .handlers
            .iter()
            .find(|handler| &handler.id == chosen)
            .map(|handler| {
                if self.association.default_application_id.as_ref() == Some(chosen) {
                    format!("{} (default)", handler.name)
                } else {
                    handler.name.clone()
                }
            })
            .unwrap_or_else(|| "None".to_owned())
    }

    fn can_change_all(&self) -> bool {
        !self.busy
            && self.chosen.is_some()
            && self.chosen != self.association.default_application_id
    }
}

pub(super) struct InfoWindow {
    entry: Entry,
    details: Vec<(&'static str, String)>,
    folder_scan_cancel: Option<Arc<AtomicBool>>,
    thumbnail: Option<PathBuf>,
    name_input: Option<gpui::Entity<InputState>>,
    comment_input: Option<gpui::Entity<InputState>>,
    saved_comment: String,
    tag: Option<&'static str>,
    access: Access,
    open_with: Option<OpenWithInfo>,
    error: Option<SharedString>,
    owner: gpui::WeakEntity<FinderView>,
    pub(super) focus: FocusHandle,
}

fn stored_tag(entry: &Entry) -> Option<&'static str> {
    let tag = entry.tag.as_ref()?;
    file_tags().iter().copied().find(|candidate| *candidate == tag.as_ref())
}

impl InfoWindow {
    pub(super) fn new(
        entry: Entry,
        thumbnail: Option<PathBuf>,
        owner: gpui::WeakEntity<FinderView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let title = format!("{} Info", entry.name);
        window.set_window_title(&title);
        let editable = entry.application.is_none();
        let name_input = editable.then(|| {
            let input =
                cx.new(|cx| InputState::new(window, cx).default_value(entry.name.to_string()));
            cx.subscribe_in(
                &input,
                window,
                |this, input, event: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        this.commit_name(input.clone(), window, cx);
                    }
                },
            )
            .detach();
            input
        });
        let saved_comment = entry
            .comment
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default();
        let comment_input = editable.then(|| {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .multi_line(true)
                    .rows(2)
                    .default_value(saved_comment.clone())
            });
            cx.subscribe_in(&input, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    this.commit_comment(cx);
                }
            })
            .detach();
            input
        });
        let focus = cx.focus_handle();
        let mut info = Self {
            details: info_details(&entry),
            tag: stored_tag(&entry),
            access: read_access(&entry.path),
            entry,
            folder_scan_cancel: None,
            thumbnail,
            name_input,
            comment_input,
            saved_comment,
            open_with: None,
            error: None,
            owner,
            focus,
        };
        info.start_folder_scan(cx);
        info.load_open_with(cx);
        info
    }

    fn start_folder_scan(&mut self, cx: &mut Context<Self>) {
        if let Some(cancel) = self.folder_scan_cancel.take() {
            cancel.store(true, Ordering::Release);
        }
        if !self.entry.is_dir {
            return;
        }
        let path = self.entry.path.clone();
        let scanned_path = path.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let pending = cancel.clone();
        self.folder_scan_cancel = Some(cancel.clone());
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let scanned =
                cx.background_executor()
                    .spawn(async move {
                        scan_folder_size(&scanned_path, &cancel, MAX_FOLDER_INFO_ENTRIES)
                    })
                    .await;
            let _ = this.update(cx, |this, cx| {
                if this.entry.path != path || pending.load(Ordering::Acquire) {
                    return;
                }
                let Some((_, value)) = this.details.iter_mut().find(|(key, _)| *key == "Size")
                else {
                    return;
                };
                *value = scanned
                    .map(format_folder_size)
                    .unwrap_or_else(|| "Unavailable".to_owned());
                cx.notify();
            });
        })
        .detach();
    }

    /// Run one filesystem change away from the UI thread, then re-read
    /// what the window shows and report a failure in the window.
    fn change(
        &mut self,
        cx: &mut Context<Self>,
        job: impl FnOnce(&Path) -> Result<(), String> + Send + 'static,
    ) {
        let path = self.entry.path.clone();
        self.error = None;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let job_path = path.clone();
            let (result, access, tag, comment) = cx
                .background_executor()
                .spawn(async move {
                    let result = job(&job_path);
                    (
                        result,
                        read_access(&job_path),
                        super::item_operations::file_tag_label(&job_path),
                        read_file_comment(&job_path),
                    )
                })
                .await;
            let _ = this.update(cx, |this: &mut InfoWindow, cx| {
                if this.entry.path != path {
                    return;
                }
                this.access = access;
                this.entry.tag = tag;
                this.tag = stored_tag(&this.entry);
                this.entry.comment = comment;
                if let Err(error) = result {
                    this.error = Some(format!("The change could not be made: {error}").into());
                }
                this.details = file_info(&this.entry);
                if this.entry.is_dir {
                    this.details.insert(1, ("Size", "Calculating…".to_owned()));
                    this.start_folder_scan(cx);
                }
                let (tag, comment) = (this.entry.tag.clone(), this.entry.comment.clone());
                let _ = this.owner.update(cx, |owner, cx| {
                    owner.update_entry_metadata(&path, tag, comment, cx)
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle_tag(&mut self, tag: &'static str, cx: &mut Context<Self>) {
        let next = (self.tag != Some(tag)).then_some(tag);
        self.tag = next;
        cx.notify();
        self.change(cx, move |path| apply_file_tag(path, next));
    }

    fn commit_comment(&mut self, cx: &mut Context<Self>) {
        let Some(input) = &self.comment_input else {
            return;
        };
        let comment = input.read(cx).value().trim().to_owned();
        if comment == self.saved_comment {
            return;
        }
        self.saved_comment = comment.clone();
        self.change(cx, move |path| write_file_comment(path, &comment));
    }

    fn toggle_locked(&mut self, locked: bool, cx: &mut Context<Self>) {
        self.access.locked = locked;
        cx.notify();
        self.change(cx, move |path| set_locked(path, locked));
    }

    #[cfg(unix)]
    fn set_privilege(&mut self, shift: u32, privilege: Privilege, cx: &mut Context<Self>) {
        let Some(unix) = self.access.unix.as_ref().filter(|unix| unix.mine) else {
            return;
        };
        let mode = with_privilege(unix.mode, shift, privilege, self.entry.is_dir);
        if mode == unix.mode {
            return;
        }
        self.change(cx, move |path| set_mode(path, mode));
    }

    fn load_open_with(&mut self, cx: &mut Context<Self>) {
        if self.entry.is_dir || self.entry.application.is_some() || !cfg!(target_os = "linux") {
            return;
        }
        let path = self.entry.path.clone();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let association = rmac_app_launch::file_association(path.clone()).await;
            let chosen_path = path.clone();
            let own_choice =
                blocking::unblock(move || super::file_metadata::read_open_with(&chosen_path))
                    .await;
            let _ = this.update(cx, |this: &mut InfoWindow, cx| {
                if this.entry.path != path {
                    return;
                }
                let Ok(association) = association else {
                    return;
                };
                let chosen = own_choice
                    .filter(|id| association.handlers.iter().any(|handler| &handler.id == id))
                    .or_else(|| association.default_application_id.clone());
                this.open_with = Some(OpenWithInfo {
                    association,
                    chosen,
                    confirm_change_all: false,
                    busy: false,
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// Choosing an application in the pop-up makes this one file open with
    /// it, as on the Mac; the type's default is untouched.
    fn choose_open_with(&mut self, application_id: String, cx: &mut Context<Self>) {
        let Some(open_with) = self.open_with.as_mut() else {
            return;
        };
        let default = open_with.association.default_application_id.clone();
        open_with.chosen = Some(application_id.clone());
        open_with.confirm_change_all = false;
        cx.notify();
        let stored = (Some(&application_id) != default.as_ref()).then_some(application_id);
        self.change(cx, move |path| {
            super::file_metadata::write_open_with(path, stored.as_deref())
        });
    }

    fn change_all(&mut self, cx: &mut Context<Self>) {
        let Some(open_with) = self.open_with.as_mut() else {
            return;
        };
        let Some(application_id) = open_with.chosen.clone() else {
            return;
        };
        open_with.busy = true;
        open_with.confirm_change_all = false;
        let mime_type = open_with.association.mime_type.clone();
        let path = self.entry.path.clone();
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result =
                rmac_app_launch::set_default_application(mime_type, application_id.clone()).await;
            if result.is_ok() {
                // The type's default now matches, so the file needs no
                // choice of its own.
                let clear_path = path.clone();
                let _ = blocking::unblock(move || {
                    super::file_metadata::write_open_with(&clear_path, None)
                })
                .await;
            }
            let _ = this.update(cx, |this: &mut InfoWindow, cx| {
                if let Some(open_with) = this.open_with.as_mut() {
                    open_with.busy = false;
                    match result {
                        Ok(()) => {
                            open_with.association.default_application_id = Some(application_id)
                        }
                        Err(error) => this.error = Some(error.to_string().into()),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn commit_name(
        &mut self,
        input: gpui::Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = self.entry.path.clone();
        let new_name = input.read(cx).value().to_string();
        let renamed = self
            .owner
            .update(cx, |owner, cx| owner.rename_path_to(&path, &new_name, cx));
        match renamed {
            Ok(Some((destination, completion))) => {
                let window_handle = window.window_handle();
                cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
                    if completion.recv().await != Ok(true) {
                        return;
                    }
                    let _ = cx.update_window(window_handle, |_, window, cx| {
                        let _ = this.update(cx, |this: &mut InfoWindow, cx| {
                            if let Some(entry) = entry_for(&destination) {
                                this.entry = entry;
                                this.details = info_details(&this.entry);
                                this.start_folder_scan(cx);
                                let name = this.entry.name.clone();
                                window.set_window_title(&format!("{name} Info"));
                                input.update(cx, |state, cx| state.set_value(name, window, cx));
                                cx.notify();
                            }
                        });
                    });
                })
                .detach();
            }
            Err(_) => window.remove_window(),
            Ok(None) => {}
        }
    }

    /// Finder's Tags field: the applied tag as a token (or the "Add Tags…"
    /// placeholder), with the seven colours to choose from under it.
    fn render_tags(&self, cx: &mut Context<Self>) -> Div {
        let field = div()
            .id("info-tags")
            .role(Role::Group)
            .aria_label("Tags")
            .h(px(24.0))
            .px(px(6.0))
            .flex()
            .items_center()
            .rounded(px(rmac_ui::mac::radius_control()))
            .border_1()
            .border_color(header_divider())
            .bg(rmac_ui::mac::window())
            .text_size(rmac_ui::text_px(INFO_ROW_TEXT))
            .child(match self.tag {
                Some(tag) => div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .px(px(6.0))
                    .h(px(18.0))
                    .rounded_full()
                    .bg(stripe())
                    .text_color(label())
                    .child(div().size(px(8.0)).rounded_full().bg(tag_colour(tag)))
                    .child(tag_display_name(tag))
                    .into_any_element(),
                None => div()
                    .text_color(secondary_text())
                    .child("Add Tags…")
                    .into_any_element(),
            });
        let dots = div()
            .flex()
            .gap(px(6.0))
            .pt(px(6.0))
            .children(file_tags().iter().map(|&tag| {
                let applied = self.tag == Some(tag);
                div()
                    .id(SharedString::from(format!("info-tag-{tag}")))
                    .role(Role::CheckBox)
                    .aria_label(tag_display_name(tag))
                    .aria_toggled(if applied {
                        Toggled::True
                    } else {
                        Toggled::False
                    })
                    .size(px(16.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .when(applied, |dot| dot.border_2().border_color(label()))
                    .cursor_pointer()
                    .child(div().size(px(11.0)).rounded_full().bg(tag_colour(tag)))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_tag(tag, cx)))
            }));
        div()
            .px(px(INFO_SECTION_INSET))
            .pb(px(10.0))
            .child(field)
            .child(dots)
    }

    fn render_more_info(&self) -> Div {
        let last_opened = if self.entry.last_opened_absolute.is_empty() {
            "--".to_owned()
        } else {
            self.entry.last_opened_absolute.to_string()
        };
        let details = [("Last opened", last_opened)];
        info_disclosure(InfoSection::MoreInfo, || {
            info_rows(&details, &["Last opened"], INFO_PERMISSIONS_LABEL)
        })
    }

    fn render_open_with(&self, cx: &mut Context<Self>) -> Option<Div> {
        let open_with = self.open_with.as_ref()?;
        let entity = cx.entity().downgrade();
        let handlers = open_with
            .association
            .handlers
            .iter()
            .map(|handler| (handler.id.clone(), handler.name.clone()))
            .collect::<Vec<_>>();
        let default = open_with.association.default_application_id.clone();
        let chosen = open_with.chosen.clone();
        let popup = PopUpButton::new("info-open-with", open_with.chosen_name())
            .accessible_name("Open with")
            .w_full()
            .disabled(open_with.busy || handlers.is_empty())
            .dropdown_menu(move |menu, _, _| {
                handlers.iter().fold(menu, |menu, (id, name)| {
                    let entity = entity.clone();
                    let choice = id.clone();
                    let label = if default.as_ref() == Some(id) {
                        format!("{name} (default)")
                    } else {
                        name.clone()
                    };
                    menu.item(
                        PopupMenuItem::new(label)
                            .checked(chosen.as_ref() == Some(id))
                            .on_click(move |_, _, cx| {
                                let _ = entity.update(cx, |this: &mut InfoWindow, cx| {
                                    this.choose_open_with(choice.clone(), cx)
                                });
                            }),
                    )
                })
            });
        let confirm = open_with.confirm_change_all.then(|| {
            let name = open_with.chosen_name();
            div()
                .id("info-change-all-confirm")
                .role(Role::AlertDialog)
                .aria_label("Change All")
                .mt(px(6.0))
                .p(px(8.0))
                .v_flex()
                .gap(px(6.0))
                .rounded(px(rmac_ui::mac::radius_control()))
                .bg(stripe())
                .text_size(rmac_ui::text_px(INFO_ROW_TEXT))
                .text_color(label())
                .child(format!(
                    "Are you sure you want to change all similar documents to open with the application “{}”?",
                    name.trim_end_matches(" (default)")
                ))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(6.0))
                        .child(Button::new("info-change-all-cancel", "Cancel").small().on_click(
                            cx.listener(|this, _, _, cx| {
                                if let Some(open_with) = this.open_with.as_mut() {
                                    open_with.confirm_change_all = false;
                                }
                                cx.notify();
                            }),
                        ))
                        .child(
                            Button::new("info-change-all-continue", "Continue")
                                .small()
                                .primary()
                                .on_click(cx.listener(|this, _, _, cx| this.change_all(cx))),
                        ),
                )
        });
        let can_change_all = open_with.can_change_all();
        Some(info_disclosure(InfoSection::OpenWith, move || {
            vec![
                div().pt(px(2.0)).child(popup).into_any_element(),
                div()
                    .pt(px(4.0))
                    .text_size(rmac_ui::text_px(INFO_ROW_TEXT))
                    .text_color(label())
                    .child("Use this application to open all documents like this one.")
                    .into_any_element(),
                div()
                    .pt(px(6.0))
                    .child(
                        Button::new("info-change-all", "Change All…")
                            .small()
                            .disabled(!can_change_all)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(open_with) = this.open_with.as_mut() {
                                    open_with.confirm_change_all = true;
                                }
                                cx.notify();
                            })),
                    )
                    .into_any_element(),
            ]
            .into_iter()
            .chain(confirm.map(IntoElement::into_any_element))
            .collect()
        }))
    }

    fn render_sharing(&self, cx: &mut Context<Self>) -> Div {
        let summary = access_summary(self.access.can_read, self.access.can_write);
        #[cfg(unix)]
        let table = self.access.unix.as_ref().map(|unix| {
            let is_dir = self.entry.is_dir;
            let row = |id: &'static str, name: String, shift: u32| {
                let privilege = Privilege::from_bits((unix.mode >> shift) & 0o7);
                let entity = cx.entity().downgrade();
                div()
                    .flex()
                    .items_center()
                    .h(px(24.0))
                    .text_size(rmac_ui::text_px(INFO_ROW_TEXT))
                    .text_color(label())
                    .child(div().flex_1().min_w(px(0.0)).truncate().child(name))
                    .child(
                        PopUpButton::new(id, privilege.label(is_dir))
                            .accessible_name("Privilege")
                            .w(px(128.0))
                            .disabled(!unix.mine)
                            .dropdown_menu(move |menu, _, _| {
                                Privilege::ALL.iter().fold(menu, |menu, &choice| {
                                    let entity = entity.clone();
                                    menu.item(
                                        PopupMenuItem::new(choice.label(is_dir))
                                            .checked(choice == privilege)
                                            .on_click(move |_, _, cx| {
                                                let _ = entity.update(
                                                    cx,
                                                    |this: &mut InfoWindow, cx| {
                                                        this.set_privilege(shift, choice, cx)
                                                    },
                                                );
                                            }),
                                    )
                                })
                            }),
                    )
            };
            let owner = if unix.mine {
                format!("{} (Me)", unix.owner)
            } else {
                unix.owner.clone()
            };
            div()
                .v_flex()
                .pt(px(4.0))
                .child(
                    div()
                        .flex()
                        .h(px(18.0))
                        .text_size(rmac_ui::text_px(11.0))
                        .text_color(secondary_text())
                        .border_b_1()
                        .border_color(header_divider())
                        .child(div().flex_1().child("Name"))
                        .child(div().w(px(128.0)).child("Privilege")),
                )
                .child(row("info-privilege-owner", owner, 6))
                .child(row("info-privilege-group", unix.group.clone(), 3))
                .child(row("info-privilege-everyone", "everyone".to_owned(), 0))
        });
        #[cfg(not(unix))]
        let table: Option<Div> = {
            let _ = cx;
            None
        };
        info_disclosure(InfoSection::Permissions, move || {
            let mut rows = vec![div()
                .pt(px(2.0))
                .text_size(rmac_ui::text_px(INFO_ROW_TEXT))
                .text_color(label())
                .child(summary)];
            rows.extend(table);
            rows
        })
    }

    /// Finder's information panel, hosted in its own resizable app window.
    fn render_info(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let e = &self.entry;
        let details = &self.details;
        let title = format!("{} Info", e.name);
        let header = info_header(
            info_artwork(e, self.thumbnail.as_ref()),
            e.name.clone(),
            (!e.is_dir).then(|| e.size.clone()),
            format!("Modified: {}", e.modified),
        );
        let editable = e.application.is_none();

        let general = info_general(details).when(editable && section_open(InfoSection::General), |block| {
            block.child(
                div().pt(px(6.0)).pl(px(INFO_LABEL_RIGHT - INFO_SECTION_INSET - 13.0)).child(
                    Checkbox::new("info-locked")
                        .label("Locked")
                        .checked(self.access.locked)
                        .on_change(cx.listener(|this, checked: &bool, _, cx| {
                            this.toggle_locked(*checked, cx)
                        })),
                ),
            )
        });

        // Finder's Name & Extension field: edit and press Return to rename.
        let name_field = self.name_input.as_ref().map(|input| {
            info_disclosure(InfoSection::NameExtension, || {
                vec![div()
                    .id("info-name")
                    .role(Role::Group)
                    .aria_label("Name & Extension")
                    .child(TextField::new(input).small())]
            })
        });

        let comments = self.comment_input.as_ref().map(|input| {
            info_disclosure(InfoSection::Comments, || {
                vec![div()
                    .id("info-comments")
                    .role(Role::Group)
                    .aria_label("Comments")
                    .child(TextField::new(input).small())]
            })
        });

        let preview = self.thumbnail.as_ref().map(|thumbnail| {
            info_disclosure(InfoSection::Preview, || {
                vec![div()
                    .h(px(INFO_PREVIEW_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        img(thumbnail.clone())
                            .max_w(gpui::relative(1.0))
                            .max_h(px(INFO_PREVIEW_HEIGHT))
                            .rounded(px(rmac_ui::mac::radius_control())),
                    )]
            })
        });

        let error = self.error.clone().map(|error| {
            div()
                .px(px(INFO_SECTION_INSET))
                .pb(px(6.0))
                .text_size(rmac_ui::text_px(INFO_ROW_TEXT))
                .text_color(rmac_ui::theme::current().colors.system_red.hsla())
                .child(error)
        });

        let card = info_card(&title, &self.focus)
            .child(info_title_strip(&title))
            .child(
                info_body()
                    .child(header)
                    .when(editable, |body| body.child(self.render_tags(cx)))
                    .children(error)
                    .child(general)
                    .child(self.render_more_info())
                    .children(name_field)
                    .children(comments)
                    .children(self.render_open_with(cx))
                    .children(preview)
                    .child(self.render_sharing(cx)),
            );

        card.on_key_down(cx.listener(|_, event: &KeyDownEvent, window, cx| {
            if event.keystroke.key.as_str() == "escape" {
                cx.stop_propagation();
                window.remove_window();
            }
        }))
    }
}

impl Drop for InfoWindow {
    fn drop(&mut self) {
        if let Some(cancel) = &self.folder_scan_cancel {
            cancel.store(true, Ordering::Release);
        }
    }
}

impl gpui::Focusable for InfoWindow {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for InfoWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_info(cx)
    }
}

impl FinderView {
    /// An Info window changed an item's tag or comment: show it in this
    /// window's rows at once, without listing the folder again.
    pub(super) fn update_entry_metadata(
        &mut self,
        path: &Path,
        tag: Option<SharedString>,
        comment: Option<SharedString>,
        cx: &mut Context<Self>,
    ) {
        let mut changed = false;
        let rows = self
            .entries
            .iter_mut()
            .chain(self.root_entries.iter_mut())
            .chain(self.child_entries.values_mut().flatten());
        for entry in rows.filter(|entry| entry.path == path) {
            entry.tag = tag.clone();
            entry.comment = comment.clone();
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }
}

pub(super) fn info_details(entry: &Entry) -> Vec<(&'static str, String)> {
    let mut details = file_info(entry);
    if entry.is_dir {
        details.insert(1, ("Size", "Calculating…".to_owned()));
    }
    details
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privileges_read_from_and_write_to_each_triplet() {
        assert_eq!(Privilege::from_bits(0o7), Privilege::ReadWrite);
        assert_eq!(Privilege::from_bits(0o5), Privilege::ReadOnly);
        assert_eq!(Privilege::from_bits(0o2), Privilege::WriteOnly);
        assert_eq!(Privilege::from_bits(0o1), Privilege::NoAccess);
        // A file keeps its execute bits.
        assert_eq!(with_privilege(0o755, 3, Privilege::NoAccess, false), 0o715);
        assert_eq!(with_privilege(0o644, 0, Privilege::ReadWrite, false), 0o646);
        assert_eq!(with_privilege(0o744, 6, Privilege::ReadOnly, false), 0o544);
        // A folder can be searched whenever it can be read or written.
        assert_eq!(with_privilege(0o700, 0, Privilege::ReadOnly, true), 0o705);
        assert_eq!(with_privilege(0o755, 0, Privilege::WriteOnly, true), 0o753);
        assert_eq!(with_privilege(0o755, 0, Privilege::NoAccess, true), 0o750);
        assert_eq!(Privilege::WriteOnly.label(true), "Write only (Drop Box)");
        assert_eq!(Privilege::WriteOnly.label(false), "Write only");
    }

    #[test]
    fn access_summary_uses_finder_wording() {
        assert_eq!(access_summary(true, true), "You can read and write");
        assert_eq!(access_summary(true, false), "You can only read");
        assert_eq!(access_summary(false, true), "You can only write");
        assert_eq!(access_summary(false, false), "You have no access");
    }

    #[test]
    fn sections_collapse_for_every_window_and_reopen() {
        assert!(section_open(InfoSection::Comments));
        toggle_section(InfoSection::Comments);
        assert!(!section_open(InfoSection::Comments));
        assert!(section_open(InfoSection::General));
        toggle_section(InfoSection::Comments);
        assert!(section_open(InfoSection::Comments));
        let titles = [
            InfoSection::General,
            InfoSection::MoreInfo,
            InfoSection::NameExtension,
            InfoSection::Comments,
            InfoSection::OpenWith,
            InfoSection::Preview,
            InfoSection::Permissions,
        ]
        .map(InfoSection::title);
        assert_eq!(titles[6], "Sharing & Permissions:");
    }

    #[test]
    fn locking_removes_write_access_and_unlocking_restores_only_the_owners() {
        let path = std::env::temp_dir().join(format!("rmac-lock-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"x").unwrap();
        set_locked(&path, true).unwrap();
        assert!(read_access(&path).locked);
        assert!(std::fs::metadata(&path).unwrap().permissions().readonly());
        set_locked(&path, false).unwrap();
        assert!(!read_access(&path).locked);
        #[cfg(unix)]
        {
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o200, 0o200);
            assert_eq!(mode & 0o002, 0);
        }
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn privileges_change_the_mode_of_an_owned_file() {
        let path = std::env::temp_dir().join(format!("rmac-mode-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"x").unwrap();
        set_mode(&path, 0o640).unwrap();
        let access = read_access(&path);
        let unix = access.unix.unwrap();
        assert!(unix.mine);
        assert_eq!(unix.mode, 0o640);
        assert_eq!(Privilege::from_bits(unix.mode & 0o7), Privilege::NoAccess);
        set_mode(&path, with_privilege(unix.mode, 0, Privilege::ReadOnly, false)).unwrap();
        assert_eq!(read_access(&path).unix.unwrap().mode, 0o644);
        std::fs::remove_file(path).unwrap();
    }
}
