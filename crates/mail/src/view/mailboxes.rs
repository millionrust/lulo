//! Mailbox ▸ New Mailbox…, New Smart Mailbox…, Edit Smart Mailbox…,
//! Duplicate Smart Mailbox, Rename Mailbox…, Delete Mailbox… and Export
//! Mailbox…, with the sheets and alerts Mac Mail shows for them.
//!
//! A live account's change goes to its `rmac-mail-storage` cache at once
//! and from there to the server (IMAP CREATE/RENAME/DELETE, or Microsoft
//! Graph mailFolders) on the sync worker's next pass, which this wakes.
//! Fixture data (`RMAC_MAIL_FIXTURE=1`) changes in memory only. Smart
//! mailboxes are saved searches kept in Mail's data folder.

use std::sync::Arc;

use gpui::{
    div, px, AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Role, SharedString, Styled as _, Window,
};
use rmac_editor::InputState;
use rmac_mail::smart::{
    self, leaf_name, renamed_path, SmartCondition, SmartField, SmartMailbox, MAX_CONDITIONS,
};
use rmac_mail::{Mailbox, RealMailbox};
use rmac_ui::{
    mac, AccessibleTextInput as _, DialogButtonKind, InputEvent, PopUpButton, PopupMenuItem,
    TextField,
};
use uuid::Uuid;

use super::MailView;

/// Fixture mailboxes use "/" between levels, like Graph and most IMAP
/// servers.
const FIXTURE_DELIMITER: char = '/';

/// What a smart mailbox condition row tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowKind {
    From,
    To,
    Subject,
    EntireMessage,
    Unread,
    Flagged,
    Attachments,
    DateReceived,
}

impl RowKind {
    const ALL: [RowKind; 8] = [
        RowKind::From,
        RowKind::To,
        RowKind::Subject,
        RowKind::EntireMessage,
        RowKind::Unread,
        RowKind::Flagged,
        RowKind::Attachments,
        RowKind::DateReceived,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::From => "From",
            Self::To => "Any Recipient",
            Self::Subject => "Subject",
            Self::EntireMessage => "Entire Message",
            Self::Unread => "Message is Unread",
            Self::Flagged => "Message is Flagged",
            Self::Attachments => "Contains Attachments",
            Self::DateReceived => "Date Received",
        }
    }

    fn of(condition: &SmartCondition) -> (Self, String) {
        match condition {
            SmartCondition::Contains { field, text } => (
                match field {
                    SmartField::From => Self::From,
                    SmartField::To => Self::To,
                    SmartField::Subject => Self::Subject,
                    SmartField::EntireMessage => Self::EntireMessage,
                },
                text.clone(),
            ),
            SmartCondition::Unread => (Self::Unread, String::new()),
            SmartCondition::Flagged => (Self::Flagged, String::new()),
            SmartCondition::HasAttachment => (Self::Attachments, String::new()),
            SmartCondition::ReceivedWithinDays { days } => (Self::DateReceived, days.to_string()),
        }
    }

    fn condition(self, text: &str) -> Option<SmartCondition> {
        let contains = |field| SmartCondition::Contains {
            field,
            text: text.trim().to_owned(),
        };
        Some(match self {
            Self::From => contains(SmartField::From),
            Self::To => contains(SmartField::To),
            Self::Subject => contains(SmartField::Subject),
            Self::EntireMessage => contains(SmartField::EntireMessage),
            Self::Unread => SmartCondition::Unread,
            Self::Flagged => SmartCondition::Flagged,
            Self::Attachments => SmartCondition::HasAttachment,
            Self::DateReceived => SmartCondition::ReceivedWithinDays {
                days: text.trim().parse().ok().filter(|days| *days > 0)?,
            },
        })
    }

    fn takes_text(self) -> bool {
        !matches!(self, Self::Unread | Self::Flagged | Self::Attachments)
    }
}

pub(super) struct SmartRow {
    kind: RowKind,
    text: Entity<InputState>,
}

pub(super) enum MailboxSheet {
    New {
        name: Entity<InputState>,
        account: Uuid,
        /// The mailbox it goes inside; `None` for the account's top level.
        parent: Option<i64>,
    },
    Rename {
        name: Entity<InputState>,
        target: Mailbox,
    },
    Delete {
        target: Mailbox,
    },
    Smart {
        id: u64,
        name: Entity<InputState>,
        match_all: bool,
        rows: Vec<SmartRow>,
    },
}

impl MailView {
    /// Mail runs on fixture data (`RMAC_MAIL_FIXTURE=1`): nothing is saved.
    fn fixture_mode(&self) -> bool {
        self.accounts
            .iter()
            .any(|account| account.path.starts_with("/fixture/"))
    }

    fn smart_mailboxes_path() -> Option<std::path::PathBuf> {
        crate::delivery::data_root().map(|root| root.join(smart::SMART_MAILBOXES_FILE))
    }

    /// Loads the saved smart mailboxes off the UI thread at launch.
    pub(super) fn load_smart_mailboxes(&mut self, cx: &mut Context<Self>) {
        if self.fixture_mode() {
            return;
        }
        let Some(path) = Self::smart_mailboxes_path() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let list = cx
                .background_executor()
                .spawn(async move { smart::load_smart_mailboxes(&path) })
                .await;
            if list.is_empty() {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                this.state.set_smart_mailboxes(list);
                this.sync_menu(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn save_smart_mailboxes(&self, cx: &mut Context<Self>) {
        if self.fixture_mode() {
            return;
        }
        let Some(path) = Self::smart_mailboxes_path() else {
            return;
        };
        let list = self.state.smart_mailboxes();
        cx.background_executor()
            .spawn(async move {
                if let Err(error) = smart::save_smart_mailboxes(&path, &list) {
                    eprintln!("rmac-mail: could not save smart mailboxes: {error}");
                }
            })
            .detach();
    }

    /// The account New Mailbox… starts in: the open mailbox's, else the
    /// selected message's, else the first account's.
    fn current_account(&self) -> Option<RealMailbox> {
        let from = |mailbox: &Mailbox| match mailbox {
            Mailbox::Real(real) => Some(real.clone()),
            _ => None,
        };
        from(&self.state.mailbox)
            .or_else(|| {
                self.state
                    .selected_message()
                    .and_then(|message| from(&message.mailbox))
            })
            .or_else(|| self.state.mailboxes.iter().find_map(from))
    }

    pub(super) fn sync_mailbox_menu(&self, cx: &mut Context<Self>) {
        use rmac_ui::set_menu_enabled as enable;
        let smart = matches!(self.state.mailbox, Mailbox::Smart(_));
        let sheet_open = self.mailbox_sheet.is_some();
        enable(
            "mail::NewMailbox",
            !sheet_open && self.current_account().is_some(),
            cx,
        );
        enable("mail::NewSmartMailbox", !sheet_open, cx);
        enable("mail::EditSmartMailbox", !sheet_open && smart, cx);
        enable("mail::DuplicateSmartMailbox", !sheet_open && smart, cx);
        let editable = !sheet_open && self.state.editable_mailbox().is_some();
        enable("mail::RenameMailbox", editable, cx);
        enable("mail::DeleteMailbox", editable, cx);
        enable(
            "mail::ExportMailbox",
            !sheet_open && self.state.mailbox.is_real(),
            cx,
        );
    }

    fn text_input(
        &self,
        value: &str,
        placeholder: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        let value = value.to_owned();
        let placeholder = placeholder.to_owned();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .default_value(value)
        });
        cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.confirm_mailbox_sheet(window, cx);
            }
        })
        .detach();
        input
    }

    fn focus_and_select(input: &Entity<InputState>, window: &mut Window, cx: &mut Context<Self>) {
        input.update(cx, |state, cx| state.focus(window, cx));
        // Like the Mac, a name to edit starts selected.
        window.on_next_frame(|window, cx| {
            if let Ok(action) = cx.build_action("input::SelectAll", None) {
                window.dispatch_action(action, cx);
            }
        });
    }

    fn open_sheet(&mut self, sheet: MailboxSheet, cx: &mut Context<Self>) {
        self.mailbox_sheet = Some(sheet);
        self.sync_menu(cx);
        cx.notify();
    }

    fn close_mailbox_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mailbox_sheet = None;
        window.focus(&self.focus, cx);
        self.sync_menu(cx);
        cx.notify();
    }

    fn new_mailbox(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(place) = self.current_account() else {
            return;
        };
        let name = self.text_input("", "Mailbox Name", window, cx);
        Self::focus_and_select(&name, window, cx);
        self.open_sheet(
            MailboxSheet::New {
                name,
                account: place.account,
                parent: None,
            },
            cx,
        );
    }

    fn rename_mailbox(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.state.editable_mailbox().cloned() else {
            return;
        };
        let current = match &target {
            Mailbox::Real(real) => leaf_name(&real.name, FIXTURE_DELIMITER).to_owned(),
            other => other.label(),
        };
        let name = self.text_input(&current, "Mailbox Name", window, cx);
        Self::focus_and_select(&name, window, cx);
        self.open_sheet(MailboxSheet::Rename { name, target }, cx);
    }

    fn delete_mailbox(&mut self, cx: &mut Context<Self>) {
        if let Some(target) = self.state.editable_mailbox().cloned() {
            self.open_sheet(MailboxSheet::Delete { target }, cx);
        }
    }

    fn smart_sheet(&mut self, smart: SmartMailbox, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.text_input(&smart.name, "Smart Mailbox Name", window, cx);
        let conditions = if smart.conditions.is_empty() {
            vec![SmartCondition::default_row()]
        } else {
            smart.conditions.clone()
        };
        let rows = conditions
            .iter()
            .map(|condition| {
                let (kind, text) = RowKind::of(condition);
                SmartRow {
                    kind,
                    text: self.text_input(&text, "", window, cx),
                }
            })
            .collect();
        Self::focus_and_select(&name, window, cx);
        self.open_sheet(
            MailboxSheet::Smart {
                id: smart.id,
                name,
                match_all: smart.match_all,
                rows,
            },
            cx,
        );
    }

    fn new_smart_mailbox(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let smart = SmartMailbox {
            id: self.state.next_smart_id(),
            name: self.state.next_smart_name(),
            match_all: false,
            conditions: Vec::new(),
        };
        self.smart_sheet(smart, window, cx);
    }

    fn edit_smart_mailbox(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Mailbox::Smart(smart) = self.state.mailbox.clone() {
            self.smart_sheet(smart, window, cx);
        }
    }

    fn duplicate_smart_mailbox(&mut self, cx: &mut Context<Self>) {
        let Mailbox::Smart(smart) = &self.state.mailbox else {
            return;
        };
        if let Some(copy) = self.state.duplicate_smart_mailbox(smart.id) {
            self.state.select_mailbox(Mailbox::Smart(copy));
            self.save_smart_mailboxes(cx);
            self.after_selection(cx);
        }
    }

    /// Tab moves between the sheet's text fields, as in a Mac sheet.
    fn next_sheet_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let fields: Vec<Entity<InputState>> = match &self.mailbox_sheet {
            Some(MailboxSheet::New { name, .. } | MailboxSheet::Rename { name, .. }) => {
                vec![name.clone()]
            }
            Some(MailboxSheet::Smart { name, rows, .. }) => std::iter::once(name.clone())
                .chain(
                    rows.iter()
                        .filter(|row| row.kind.takes_text())
                        .map(|row| row.text.clone()),
                )
                .collect(),
            _ => Vec::new(),
        };
        if fields.is_empty() {
            return;
        }
        let current = fields
            .iter()
            .position(|field| field.read(cx).focus_handle(cx).is_focused(window));
        let next = current.map_or(0, |index| (index + 1) % fields.len());
        fields[next].update(cx, |state, cx| state.focus(window, cx));
    }

    /// The sheet's default button (Return): OK, Save or Delete.
    pub(super) fn confirm_mailbox_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sheet) = self.mailbox_sheet.take() else {
            return;
        };
        match sheet {
            MailboxSheet::New {
                name,
                account,
                parent,
            } => {
                let leaf = name.read(cx).value().trim().to_owned();
                if leaf.is_empty() || leaf.contains(FIXTURE_DELIMITER) {
                    self.mailbox_sheet = Some(MailboxSheet::New {
                        name,
                        account,
                        parent,
                    });
                    cx.notify();
                    return;
                }
                self.create_mailbox(account, parent, leaf, cx);
            }
            MailboxSheet::Rename { name, target } => {
                let leaf = name.read(cx).value().trim().to_owned();
                if leaf.is_empty() || leaf.contains(FIXTURE_DELIMITER) {
                    self.mailbox_sheet = Some(MailboxSheet::Rename { name, target });
                    cx.notify();
                    return;
                }
                match target {
                    Mailbox::Smart(smart) => {
                        self.state.save_smart_mailbox(SmartMailbox {
                            name: leaf,
                            ..smart
                        });
                        self.save_smart_mailboxes(cx);
                    }
                    Mailbox::Real(real) => self.rename_real(real, leaf, cx),
                    _ => {}
                }
            }
            MailboxSheet::Delete { target } => match target {
                Mailbox::Smart(smart) => {
                    self.state.remove_smart_mailbox(smart.id);
                    self.save_smart_mailboxes(cx);
                }
                Mailbox::Real(real) => self.delete_real(real, cx),
                _ => {}
            },
            MailboxSheet::Smart {
                id,
                name,
                match_all,
                rows,
            } => {
                let name_text = name.read(cx).value().trim().to_owned();
                let conditions: Vec<SmartCondition> = rows
                    .iter()
                    .filter_map(|row| row.kind.condition(&row.text.read(cx).value()))
                    .collect();
                if name_text.is_empty() {
                    self.mailbox_sheet = Some(MailboxSheet::Smart {
                        id,
                        name,
                        match_all,
                        rows,
                    });
                    cx.notify();
                    return;
                }
                let smart = SmartMailbox {
                    id,
                    name: name_text,
                    match_all,
                    conditions,
                };
                self.state.save_smart_mailbox(smart.clone());
                self.state.select_mailbox(Mailbox::Smart(smart));
                self.save_smart_mailboxes(cx);
            }
        }
        window.focus(&self.focus, cx);
        self.after_selection(cx);
    }

    fn create_mailbox(
        &mut self,
        account: Uuid,
        parent: Option<i64>,
        leaf: String,
        cx: &mut Context<Self>,
    ) {
        let parents = self.state.mailbox_parents(account);
        let Some(sibling) = parents.first().cloned() else {
            return;
        };
        if self.fixture_mode() {
            let parent_name = parent.and_then(|id| {
                parents
                    .iter()
                    .find(|real| real.mailbox_id == id)
                    .map(|real| real.name.clone())
            });
            let name = match parent_name {
                Some(parent) => format!("{parent}{FIXTURE_DELIMITER}{leaf}"),
                None => leaf,
            };
            if parents.iter().any(|real| real.name == name) {
                self.notice =
                    Some("That name is already taken. Please choose a different name.".into());
                return;
            }
            let mailbox_id = parents
                .iter()
                .map(|real| real.mailbox_id)
                .max()
                .unwrap_or(0)
                + 1;
            self.state.add_real_mailbox(RealMailbox {
                mailbox_id,
                name,
                special_use: None,
                ..sibling
            });
            return;
        }
        self.change_live(
            sibling.account,
            sibling.account_path,
            rmac_mail_storage::MailboxChange::Create { parent, name: leaf },
            true,
            cx,
        );
    }

    fn rename_real(&mut self, real: RealMailbox, leaf: String, cx: &mut Context<Self>) {
        if self.fixture_mode() {
            let renamed = renamed_path(&real.name, &leaf, FIXTURE_DELIMITER);
            if self
                .state
                .mailbox_parents(real.account)
                .iter()
                .any(|other| other.name == renamed && other.mailbox_id != real.mailbox_id)
            {
                self.notice =
                    Some("That name is already taken. Please choose a different name.".into());
                return;
            }
            self.state.rename_real_mailbox(
                real.account,
                real.mailbox_id,
                &renamed,
                FIXTURE_DELIMITER,
            );
            return;
        }
        self.change_live(
            real.account,
            real.account_path,
            rmac_mail_storage::MailboxChange::Rename {
                mailbox_id: real.mailbox_id,
                name: leaf,
            },
            true,
            cx,
        );
    }

    fn delete_real(&mut self, real: RealMailbox, cx: &mut Context<Self>) {
        // Shown gone at once; a live account's cache follows.
        self.state
            .remove_real_mailbox(real.account, real.mailbox_id, FIXTURE_DELIMITER);
        if self.fixture_mode() {
            return;
        }
        self.change_live(
            real.account,
            real.account_path,
            rmac_mail_storage::MailboxChange::Delete {
                mailbox_id: real.mailbox_id,
            },
            false,
            cx,
        );
    }

    /// Applies a folder change to a live account's cache off the UI thread,
    /// wakes its sync worker to send it, then reloads the mailbox list and
    /// (`open`) shows the changed mailbox.
    fn change_live(
        &mut self,
        account: Uuid,
        account_path: String,
        change: rmac_mail_storage::MailboxChange,
        open: bool,
        cx: &mut Context<Self>,
    ) {
        let accounts = self.accounts.clone();
        let runtime: Option<Arc<rmac_mail_runtime::Runtime>> = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    let changed = crate::live::change_mailbox(account, change)?;
                    if let Some(runtime) = runtime {
                        runtime.sync_now(&account_path);
                    }
                    Ok::<_, String>((changed, crate::live::load(&accounts)))
                })
                .await;
            let _ = this.update(cx, |this, cx| match outcome {
                Ok((mailbox_id, (mailboxes, messages))) => {
                    this.state.refresh_live(mailboxes, messages);
                    let changed = this.state.mailboxes.iter().find(|mailbox| {
                        matches!(mailbox, Mailbox::Real(real)
                            if real.account == account && real.mailbox_id == mailbox_id)
                    });
                    match changed.cloned() {
                        Some(mailbox) if open => this.state.select_mailbox(mailbox),
                        _ if !this.state.mailboxes.contains(&this.state.mailbox) => {
                            this.state.select_mailbox(Mailbox::AllInboxes);
                        }
                        _ => {}
                    }
                    this.after_selection(cx);
                }
                Err(message) => {
                    this.notice = Some(message.into());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Mailbox ▸ Export Mailbox…: the open mailbox as one mbox file.
    fn export_mailbox(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Mailbox::Real(real) = self.state.mailbox.clone() else {
            return;
        };
        let messages: Vec<rmac_mail::Message> = self
            .state
            .messages
            .iter()
            .filter(|message| message.mailbox == self.state.mailbox)
            .cloned()
            .collect();
        let leaf = leaf_name(&real.name, FIXTURE_DELIMITER).to_owned();
        let file = format!("{}.mbox", rmac_mail::safe_file_stem(&leaf));
        let chosen = cx.prompt_for_new_path(&super::menu::save_directory(), Some(&file));
        let runtime = self.runtime.clone();
        let fixture = self.fixture_mode();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = chosen.await else {
                return;
            };
            let written = cx
                .background_executor()
                .spawn(async move {
                    let sources = if fixture {
                        messages
                            .iter()
                            .map(|message| rmac_mail::message_source(message).into_bytes())
                            .collect()
                    } else {
                        crate::live::mailbox_sources(
                            runtime,
                            real.account,
                            &real.account_path,
                            &real.name,
                            &messages,
                        )
                    };
                    super::menu::write_file(&path, &smart::mbox(&sources))
                })
                .await;
            if let Err(error) = written {
                let _ = this.update(cx, |this, cx| {
                    this.notice =
                        Some(format!("Mail couldn’t export the mailbox: {error}.").into());
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(super) fn on_mailbox_actions(&self, root: gpui::Div, cx: &mut Context<Self>) -> gpui::Div {
        use crate::{
            DeleteMailbox, DuplicateSmartMailbox, EditSmartMailbox, ExportMailbox,
            MailboxSheetCancel, MailboxSheetNextField, NewMailbox, NewSmartMailbox, RenameMailbox,
        };
        root.on_action(cx.listener(|this, _: &NewMailbox, window, cx| this.new_mailbox(window, cx)))
            .on_action(cx.listener(|this, _: &NewSmartMailbox, window, cx| {
                this.new_smart_mailbox(window, cx)
            }))
            .on_action(cx.listener(|this, _: &EditSmartMailbox, window, cx| {
                this.edit_smart_mailbox(window, cx)
            }))
            .on_action(cx.listener(|this, _: &DuplicateSmartMailbox, _, cx| {
                this.duplicate_smart_mailbox(cx)
            }))
            .on_action(
                cx.listener(|this, _: &RenameMailbox, window, cx| this.rename_mailbox(window, cx)),
            )
            .on_action(cx.listener(|this, _: &DeleteMailbox, _, cx| this.delete_mailbox(cx)))
            .on_action(
                cx.listener(|this, _: &ExportMailbox, window, cx| this.export_mailbox(window, cx)),
            )
            .on_action(cx.listener(|this, _: &MailboxSheetCancel, window, cx| {
                this.close_mailbox_sheet(window, cx)
            }))
            .on_action(cx.listener(|this, _: &MailboxSheetNextField, window, cx| {
                this.next_sheet_field(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &crate::MailboxSheetConfirm, window, cx| {
                    this.confirm_mailbox_sheet(window, cx)
                }),
            )
    }

    fn text_box(
        &self,
        id: impl Into<SharedString>,
        label: &str,
        input: &Entity<InputState>,
        width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id: SharedString = id.into();
        div()
            .id(id)
            .role(Role::TextInput)
            .aria_label(label.to_owned())
            .accessible_text_input(input, cx)
            .w(px(width))
            .h(px(24.0))
            .rounded(px(mac::radius_control()))
            .bg(mac::control_fill())
            .child(TextField::new(input).appearance(false).small())
            .into_any_element()
    }

    fn sheet_card(
        &self,
        label: &'static str,
        width: f32,
        body: Vec<AnyElement>,
        ok: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id("mail-mailbox-sheet")
            .key_context("MailMailboxSheet")
            .role(Role::Dialog)
            .aria_label(label)
            .absolute()
            .top(px(super::TOOLBAR))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .w(px(width))
                    .p(px(20.0))
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .rounded(px(mac::radius_card()))
                    .bg(mac::raised())
                    .border_1()
                    .border_color(mac::separator())
                    .shadow_lg()
                    .text_size(px(13.0))
                    .text_color(mac::text())
                    .children(body)
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.0))
                            .child(
                                rmac_ui::dialog_button(
                                    "mail-mailbox-cancel",
                                    "Cancel",
                                    DialogButtonKind::Normal,
                                )
                                .on_click(cx.listener(
                                    |this, _, window, cx| this.close_mailbox_sheet(window, cx),
                                )),
                            )
                            .child(
                                rmac_ui::dialog_button(
                                    "mail-mailbox-ok",
                                    ok,
                                    DialogButtonKind::Primary,
                                )
                                .on_click(cx.listener(
                                    |this, _, window, cx| this.confirm_mailbox_sheet(window, cx),
                                )),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn location_popup(
        &self,
        account: Uuid,
        parent: Option<i64>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        // Every account's top level, then its mailboxes.
        let mut choices: Vec<(Uuid, Option<i64>, String)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for mailbox in &self.state.mailboxes {
            if let Mailbox::Real(real) = mailbox {
                if seen.insert(real.account) {
                    choices.push((real.account, None, real.account_label.clone()));
                }
                choices.push((
                    real.account,
                    Some(real.mailbox_id),
                    format!("    {}", mailbox.label()),
                ));
            }
        }
        let current = choices
            .iter()
            .find(|(owner, id, _)| *owner == account && *id == parent)
            .map(|(_, _, label)| label.trim().to_owned())
            .unwrap_or_default();
        PopUpButton::new("mail-new-mailbox-location", current)
            .accessible_name("Location")
            .w(px(220.0))
            .dropdown_menu(move |menu, _, _| {
                choices.iter().fold(menu, |menu, (owner, id, label)| {
                    let view = view.clone();
                    let (owner, id) = (*owner, *id);
                    let checked = owner == account && id == parent;
                    menu.item(PopupMenuItem::new(label.clone()).checked(checked).on_click(
                        move |_, _, cx| {
                            view.update(cx, |this, cx| {
                                if let Some(MailboxSheet::New {
                                    account, parent, ..
                                }) = &mut this.mailbox_sheet
                                {
                                    *account = owner;
                                    *parent = id;
                                }
                                cx.notify();
                            });
                        },
                    ))
                })
            })
            .into_any_element()
    }

    fn row_popup(&self, index: usize, kind: RowKind, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity();
        PopUpButton::new(format!("mail-smart-kind-{index}"), kind.label())
            .accessible_name(format!("Condition {}", index + 1))
            .w(px(170.0))
            .dropdown_menu(move |menu, _, _| {
                RowKind::ALL.iter().fold(menu, |menu, &choice| {
                    let view = view.clone();
                    menu.item(
                        PopupMenuItem::new(choice.label())
                            .checked(choice == kind)
                            .on_click(move |_, _, cx| {
                                view.update(cx, |this, cx| {
                                    if let Some(MailboxSheet::Smart { rows, .. }) =
                                        &mut this.mailbox_sheet
                                    {
                                        if let Some(row) = rows.get_mut(index) {
                                            row.kind = choice;
                                        }
                                    }
                                    cx.notify();
                                });
                            }),
                    )
                })
            })
            .into_any_element()
    }

    fn add_row(&mut self, at: usize, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.text_input("", "", window, cx);
        if let Some(MailboxSheet::Smart { rows, .. }) = &mut self.mailbox_sheet {
            if rows.len() < MAX_CONDITIONS {
                rows.insert(
                    (at + 1).min(rows.len()),
                    SmartRow {
                        kind: RowKind::From,
                        text,
                    },
                );
            }
        }
        cx.notify();
    }

    fn remove_row(&mut self, at: usize, cx: &mut Context<Self>) {
        if let Some(MailboxSheet::Smart { rows, .. }) = &mut self.mailbox_sheet {
            if rows.len() > 1 && at < rows.len() {
                rows.remove(at);
            }
        }
        cx.notify();
    }

    fn match_popup(&self, match_all: bool, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity();
        PopUpButton::new("mail-smart-match", if match_all { "all" } else { "any" })
            .accessible_name("Match")
            .w(px(64.0))
            .dropdown_menu(move |menu, _, _| {
                [("any", false), ("all", true)]
                    .into_iter()
                    .fold(menu, |menu, (label, value)| {
                        let view = view.clone();
                        menu.item(
                            PopupMenuItem::new(label)
                                .checked(value == match_all)
                                .on_click(move |_, _, cx| {
                                    view.update(cx, |this, cx| {
                                        if let Some(MailboxSheet::Smart { match_all, .. }) =
                                            &mut this.mailbox_sheet
                                        {
                                            *match_all = value;
                                        }
                                        cx.notify();
                                    });
                                }),
                        )
                    })
            })
            .into_any_element()
    }

    /// The open mailbox sheet or alert, drawn over the window.
    pub(super) fn mailbox_sheet_view(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let sheet = self.mailbox_sheet.as_ref()?;
        Some(match sheet {
            MailboxSheet::New {
                name,
                account,
                parent,
            } => {
                let body = vec![
                    div()
                        .font_weight(gpui::FontWeight::BOLD)
                        .child("New Mailbox")
                        .into_any_element(),
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(div().w(px(64.0)).child("Location:"))
                        .child(self.location_popup(*account, *parent, cx))
                        .into_any_element(),
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(div().w(px(64.0)).child("Name:"))
                        .child(self.text_box("mail-new-mailbox-name", "Name", name, 220.0, cx))
                        .into_any_element(),
                ];
                self.sheet_card("New Mailbox", 360.0, body, "OK", cx)
            }
            MailboxSheet::Rename { name, target } => {
                let body = vec![
                    div()
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(format!("Rename the mailbox “{}”", target.label()))
                        .into_any_element(),
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(div().w(px(48.0)).child("Name:"))
                        .child(self.text_box("mail-rename-mailbox-name", "Name", name, 240.0, cx))
                        .into_any_element(),
                ];
                self.sheet_card("Rename Mailbox", 360.0, body, "OK", cx)
            }
            MailboxSheet::Delete { target } => {
                let (title, message) = match target {
                    Mailbox::Smart(smart) => (
                        format!(
                            "Are you sure you want to delete the smart mailbox “{}”?",
                            smart.name
                        ),
                        "The messages it shows stay in their own mailboxes.",
                    ),
                    other => (
                        format!(
                            "Are you sure you want to delete the mailbox “{}”?",
                            other.label()
                        ),
                        "The messages in it, and any mailboxes inside it, will be deleted. You can’t undo this action.",
                    ),
                };
                // Delete is the default button, as in Mac Mail's alert.
                rmac_ui::alert(
                    title,
                    message,
                    vec![
                        rmac_ui::dialog_button(
                            "mail-delete-mailbox-cancel",
                            "Cancel",
                            DialogButtonKind::Normal,
                        )
                        .on_click(
                            cx.listener(|this, _, window, cx| this.close_mailbox_sheet(window, cx)),
                        )
                        .into_any_element(),
                        rmac_ui::dialog_button(
                            "mail-delete-mailbox-confirm",
                            "Delete",
                            DialogButtonKind::Destructive,
                        )
                        .on_click(
                            cx.listener(|this, _, window, cx| {
                                this.confirm_mailbox_sheet(window, cx)
                            }),
                        )
                        .into_any_element(),
                    ],
                )
                .into_any_element()
            }
            MailboxSheet::Smart {
                name,
                match_all,
                rows,
                ..
            } => {
                let mut body = vec![
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child("Smart Mailbox Name:")
                        .child(self.text_box(
                            "mail-smart-name",
                            "Smart Mailbox Name",
                            name,
                            240.0,
                            cx,
                        ))
                        .into_any_element(),
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child("Contains messages that match")
                        .child(self.match_popup(*match_all, cx))
                        .child("of the following conditions:")
                        .into_any_element(),
                ];
                let removable = rows.len() > 1;
                let addable = rows.len() < MAX_CONDITIONS;
                for (index, row) in rows.iter().enumerate() {
                    let field: AnyElement = match row.kind {
                        RowKind::Unread | RowKind::Flagged | RowKind::Attachments => {
                            div().w(px(200.0)).into_any_element()
                        }
                        RowKind::DateReceived => div()
                            .w(px(200.0))
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child("is in the last")
                            .child(self.text_box(
                                format!("mail-smart-days-{index}"),
                                "Days",
                                &row.text,
                                48.0,
                                cx,
                            ))
                            .child("days")
                            .into_any_element(),
                        _ => div()
                            .w(px(200.0))
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child("contains")
                            .child(self.text_box(
                                format!("mail-smart-text-{index}"),
                                "Value",
                                &row.text,
                                136.0,
                                cx,
                            ))
                            .into_any_element(),
                    };
                    body.push(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(self.row_popup(index, row.kind, cx))
                            .child(field)
                            .child(
                                rmac_ui::dialog_button(
                                    SharedString::from(format!("mail-smart-remove-{index}")),
                                    "−",
                                    DialogButtonKind::Normal,
                                )
                                .disabled(!removable)
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.remove_row(index, cx)),
                                ),
                            )
                            .child(
                                rmac_ui::dialog_button(
                                    SharedString::from(format!("mail-smart-add-{index}")),
                                    "+",
                                    DialogButtonKind::Normal,
                                )
                                .disabled(!addable)
                                .on_click(cx.listener(
                                    move |this, _, window, cx| this.add_row(index, window, cx),
                                )),
                            )
                            .into_any_element(),
                    );
                }
                self.sheet_card("Smart Mailbox", 560.0, body, "OK", cx)
            }
        })
    }
}
