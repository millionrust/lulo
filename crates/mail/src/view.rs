use std::sync::Arc;

use gpui::{
    accesskit, div, prelude::FluentBuilder as _, px, uniform_list, AnyElement, AppContext as _,
    ClickEvent, Context, Entity, FocusHandle, Focusable as _, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Role, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window,
};
use rmac_editor::InputState;
use rmac_mail::{
    compose::ComposeKind, BodyState, Erase, MailState, Mailbox, Message, OrganizeAction,
    SearchScope, SpecialUse,
};
use rmac_mail_mime::{BlockKind, RichText};
use rmac_ui::{mac, AccessibleTextInput as _, InputEvent, TextField};

use crate::{
    compose_window, delivery::ComposeAccount, Archive, CloseWindow, Copy, Delete, Flag, Forward,
    Junk, Move, NewMessage, NextMessage, PreviousMessage, Reply, ReplyAll, Search, ToggleRead,
    ToggleThreads, Undo,
};

mod mailboxes;
mod menu;
mod tools;
mod windows;

pub use menu::close_all;
#[cfg(any(target_os = "linux", windows))]
pub use tools::viewers;
pub use tools::{open_viewer, show_viewer};

const SIDEBAR: f32 = 220.0;
const LIST: f32 = 340.0;
const TOOLBAR: f32 = 52.0;
/// Where the toolbar starts while View ▸ Hide Sidebar has moved the window
/// controls into it.
const LIGHTS_WIDTH: f32 = 84.0;
/// One row of View ▸ Use Column Layout's table.
const COLUMN_ROW: f32 = 26.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ControlMode {
    Disabled,
    Enabled,
    Selected,
}

pub struct MailView {
    pub focus: FocusHandle,
    state: MailState,
    search_input: Entity<InputState>,
    search_open: bool,
    destination_menu: bool,
    copy_destination: bool,
    accounts: Vec<ComposeAccount>,
    /// `None` under `RMAC_MAIL_FIXTURE=1` and in tests; otherwise the live
    /// sync runtime, kept only to wake the right account's worker right
    /// after an organise action queues its journal entry.
    runtime: Option<Arc<rmac_mail_runtime::Runtime>>,
    /// The conversation list, drawn as its own cached view (SPEED-09).
    list_view: Option<Entity<MailListView>>,
    /// Settings ▸ Fonts & Colours and Viewing, as last applied.
    display: rmac_mail::settings::DisplayPrefs,
    /// View ▸ Hide Sidebar / Hide Toolbar.
    sidebar_hidden: bool,
    toolbar_hidden: bool,
    /// View ▸ Use Column Layout and Show Bottom Preview.
    column_layout: bool,
    bottom_preview: bool,
    /// View ▸ Show Date and Time, Show To/Cc Label, Show Contact Photo and
    /// Highlight Conversations: what each list row shows.
    show_date_time: bool,
    show_to_cc: bool,
    show_contact_photo: bool,
    highlight_conversations: bool,
    /// Mailbox ▸ Erase Deleted Items… / Erase Junk Mail waiting for its
    /// confirmation.
    erase_confirm: Option<Erase>,
    /// Mailbox ▸ New/Rename/Delete Mailbox… or a smart mailbox sheet.
    mailbox_sheet: Option<mailboxes::MailboxSheet>,
    /// A result or failure to show in an alert (a failed save, say).
    notice: Option<SharedString>,
    /// The message viewer's scroll position, for Edit ▸ Find ▸ Jump to Top.
    viewer_scroll: ScrollHandle,
    /// Whether this window is in front, so the rows a compose window
    /// re-labels (Send / Send Again) are left alone while it is.
    active: bool,
    /// Bumped for each File ▸ Print… the portal must tie to this window.
    #[cfg(target_os = "linux")]
    print_generation: Arc<std::sync::atomic::AtomicU64>,
    /// Edit ▸ Find in the message: the find bar, its field, the current
    /// match, and whether the viewer still has to scroll to it.
    find_open: bool,
    find_input: Entity<InputState>,
    find_current: usize,
    find_scroll: bool,
    find_anchor: gpui::ScrollAnchor,
    /// View ▸ Message ▸ All Headers, and the header block it shows for one
    /// message (by id).
    all_headers: bool,
    all_headers_text: Option<(String, SharedString)>,
    /// File ▸ Quick Look Attachments…'s panel while it is open.
    quick_look: Option<rmac_quick_look::Handle>,
    /// View ▸ Show Message Size.
    show_size: bool,
    /// The `MailState::revision` last handed to the other viewers.
    shared_revision: std::cell::Cell<u64>,
}

/// The conversation list as its own GPUI view (SPEED-09). A scroll
/// re-renders only this view and the root shell; the rows it lists are
/// built once per change to `MailView` (it observes it) instead of on every
/// frame, which with 10,000 messages cloned 50,000 strings per frame.
struct MailListView {
    mail: gpui::WeakEntity<MailView>,
    rows: Option<(Arc<Vec<Row>>, usize)>,
}

impl MailListView {
    fn new(mail: &Entity<MailView>, cx: &mut Context<Self>) -> Self {
        cx.observe(mail, |this, _, cx| {
            this.rows = None;
            cx.notify();
        })
        .detach();
        Self {
            mail: mail.downgrade(),
            rows: None,
        }
    }
}

impl Render for MailListView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(mail) = self.mail.upgrade() else {
            return div().into_any_element();
        };
        let (rows, unread) = self
            .rows
            .get_or_insert_with(|| mail.read(cx).list_rows())
            .clone();
        mail.read(cx).list(rows, unread, mail.downgrade())
    }
}

#[derive(Clone)]
struct Row {
    id: String,
    sender: String,
    initials: String,
    subject: String,
    preview: String,
    date: String,
    unread: bool,
    flagged: bool,
    attachment: bool,
    thread_count: usize,
    selected: bool,
    /// View ▸ Show To/Cc Label: "To" or "Cc" when the message was sent to
    /// one of this person's addresses that way.
    to_cc: Option<&'static str>,
    /// View ▸ Highlight Conversations: part of the selected conversation.
    highlighted: bool,
    /// Message ▸ Flag's colour, while `flagged`.
    flag_colour: rmac_mail::FlagColour,
    /// View ▸ Show Message Size: "212 KB", or empty.
    size: String,
}

/// What View ▸ Use Column Layout, Show Contact Photo and the other list
/// options ask each row to draw.
#[derive(Clone, Copy)]
struct RowStyle {
    columns: bool,
    contact_photo: bool,
    /// Settings ▸ Fonts & Colours and Viewing.
    display: rmac_mail::settings::DisplayPrefs,
}

impl MailView {
    pub fn new(
        state: MailState,
        accounts: Vec<ComposeAccount>,
        runtime: Option<Arc<rmac_mail_runtime::Runtime>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search Mail"));
        let find_input = cx.new(|cx| InputState::new(window, cx).placeholder("Find"));
        cx.subscribe(&find_input, |this, _, event: &InputEvent, cx| match event {
            InputEvent::Change => {
                this.find_current = 0;
                this.find_scroll = true;
                this.sync_menu(cx);
                cx.notify();
            }
            InputEvent::PressEnter { shift, .. } => {
                this.step_find(if *shift { -1 } else { 1 }, cx);
            }
            _ => {}
        })
        .detach();
        cx.subscribe(&search_input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let query = this.search_input.read(cx).value().to_owned();
                this.state.set_search(&query);
                cx.notify();
            }
        })
        .detach();
        // Microsoft Graph accounts have no push: focusing Mail runs one
        // delta query (debounced in the runtime; IMAP accounts keep IDLE).
        if let Some(runtime) = runtime.clone() {
            cx.observe_window_activation(window, move |_, window, _| {
                if window.is_window_active() {
                    runtime.refresh_on_focus();
                }
            })
            .detach();
        }
        // Settings load off the UI thread; an open window redraws when a
        // Fonts & Colours or Viewing choice changes.
        crate::settings_view::load_applied(cx);
        cx.observe_global::<crate::settings_view::Applied>(|this, cx| {
            this.display = cx.global::<crate::settings_view::Applied>().0;
            cx.notify();
        })
        .detach();
        cx.observe_window_activation(window, |this: &mut Self, window, cx| {
            this.active = window.is_window_active();
            if this.active {
                this.sync_menu(cx);
            }
        })
        .detach();
        let mut state = state;
        state.my_addresses = accounts
            .iter()
            .map(|account| account.address.clone())
            .collect();
        let viewer_scroll = ScrollHandle::new();
        let find_anchor = gpui::ScrollAnchor::for_handle(viewer_scroll.clone());
        let mut view = Self {
            focus: cx.focus_handle(),
            state,
            search_input,
            search_open: false,
            destination_menu: false,
            copy_destination: false,
            accounts,
            runtime,
            list_view: None,
            display: rmac_mail::settings::DisplayPrefs::default(),
            sidebar_hidden: false,
            toolbar_hidden: false,
            column_layout: false,
            bottom_preview: false,
            show_date_time: false,
            show_to_cc: false,
            show_contact_photo: false,
            highlight_conversations: false,
            erase_confirm: None,
            mailbox_sheet: None,
            notice: None,
            viewer_scroll,
            active: true,
            #[cfg(target_os = "linux")]
            print_generation: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            find_open: false,
            find_input,
            find_current: 0,
            find_scroll: false,
            find_anchor,
            all_headers: false,
            all_headers_text: None,
            quick_look: None,
            show_size: false,
            shared_revision: std::cell::Cell::new(0),
        };
        view.ensure_body_loaded(cx);
        view.load_smart_mailboxes(cx);
        view
    }

    /// Replaces the mailbox/message list with a freshly loaded one, keeping
    /// the current selection, mailbox and search — called after a sync
    /// snapshot or new-mail event (`main.rs`'s background event loop, which
    /// only exists on Linux: GOA and the sync runtime are Linux session
    /// services).
    #[cfg(any(target_os = "linux", windows))]
    pub fn refresh_live(
        &mut self,
        mailboxes: Vec<Mailbox>,
        messages: Vec<Message>,
        cx: &mut Context<Self>,
    ) {
        self.state.refresh_live(mailboxes, messages);
        self.ensure_body_loaded(cx);
        self.sync_menu(cx);
        cx.notify();
    }

    /// The `Message::id` for a live notification's `(account, row_id)`
    /// pair — `rmac_mail_runtime`'s `NewMail`/`OpenMessage` events only
    /// carry the account and the storage row id, not Mail's own composite
    /// id string.
    /// Windows: the account list changed (added or removed in Settings).
    #[cfg(windows)]
    pub fn set_accounts(&mut self, accounts: Vec<ComposeAccount>, cx: &mut Context<Self>) {
        self.state.my_addresses = accounts
            .iter()
            .map(|account| account.address.clone())
            .collect();
        self.accounts = accounts;
        cx.notify();
    }

    #[cfg(any(target_os = "linux", windows))]
    pub fn message_id_for_row(&self, account: uuid::Uuid, row_id: i64) -> Option<String> {
        self.state
            .messages
            .iter()
            .find(|message| {
                message.row_id == Some(row_id)
                    && matches!(&message.mailbox, Mailbox::Real(real) if real.account == account)
            })
            .map(|message| message.id.clone())
    }

    /// Selects `id` (for example from a clicked new-mail notification),
    /// opening its mailbox first if needed.
    #[cfg(any(target_os = "linux", windows))]
    pub fn open_message(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(message) = self.state.messages.iter().find(|message| message.id == id) {
            self.state.mailbox = message.mailbox.clone();
        }
        self.state.select(id);
        self.ensure_body_loaded(cx);
        self.dispatch_persist(cx);
        self.sync_menu(cx);
        cx.notify();
    }

    /// Whether the selected message's account has an Archive mailbox at
    /// all (iCloud, for example, has none) — decides whether the toolbar's
    /// Archive button is a real action or greyed out.
    fn can_archive_selected(&self) -> bool {
        self.state.selected_message().is_some_and(|message| {
            matches!(&message.mailbox, Mailbox::Real(real) if self
                .state
                .find_special(real.account, SpecialUse::Archive)
                .is_some())
        })
    }

    fn perform(&mut self, action: OrganizeAction, cx: &mut Context<Self>) {
        if self.state.apply(action) {
            self.destination_menu = false;
            self.dispatch_persist(cx);
            self.sync_menu(cx);
            cx.notify();
        }
    }

    /// Replays whatever `Persist` the last mutating `MailState` call queued
    /// into that account's storage journal, off the GPUI thread, and wakes
    /// its sync worker so the change reaches the server promptly instead of
    /// waiting for the next IDLE timeout. A no-op for fixture data, which
    /// never produces a `Persist`.
    fn dispatch_persist(&mut self, cx: &mut Context<Self>) {
        self.share_changes(cx);
        let persists = self.state.take_persists();
        if persists.is_empty() {
            return;
        }
        let runtime = self.runtime.clone();
        cx.background_executor()
            .spawn(async move {
                let mut paths: Vec<String> = Vec::new();
                for persist in persists {
                    if !paths.contains(&persist.account_path) {
                        paths.push(persist.account_path.clone());
                    }
                    crate::live::persist(persist);
                }
                if let Some(runtime) = runtime {
                    for path in &paths {
                        runtime.sync_now(path);
                    }
                }
            })
            .detach();
    }

    /// Starts loading the selected message's real body the first time it
    /// is opened: a quick local cache check, then (only if nothing is
    /// cached — a message synced header-only) a network download, the way
    /// Mac Mail downloads on demand when you open older mail. A fixture
    /// message, or one already loading/loaded, is a no-op (MAIL-10: never
    /// parses MIME, or hits the network, for rows nobody has opened).
    fn ensure_body_loaded(&mut self, cx: &mut Context<Self>) {
        let Some(message) = self.state.selected_message() else {
            return;
        };
        if message.body_state != BodyState::NotLoaded {
            return;
        }
        self.load_body_for_selected(cx);
    }

    /// Does the actual local-cache-then-network load, regardless of the
    /// message's current `BodyState`. `ensure_body_loaded` only calls this
    /// once per message (guarded on `NotLoaded`); a Retry click after a
    /// failed download calls it directly.
    fn load_body_for_selected(&mut self, cx: &mut Context<Self>) {
        let Some(message) = self.state.selected_message() else {
            return;
        };
        let Mailbox::Real(real) = message.mailbox.clone() else {
            return;
        };
        let Some(row_id) = message.row_id else {
            return;
        };
        let id = message.id.clone();
        let runtime = self.runtime.clone();
        self.state.set_body_state(&id, BodyState::Loading);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let account = real.account;
            let mailbox_id = real.mailbox_id;
            let cached = cx
                .background_executor()
                .spawn(async move { crate::live::cached_body(account, mailbox_id, row_id) })
                .await;
            if let Some((body, attachment)) = cached {
                let _ = this.update(cx, |this, cx| {
                    this.state.set_loaded_body(&id, body, attachment);
                    cx.notify();
                });
                return;
            }
            let Some(runtime) = runtime else {
                let _ = this.update(cx, |this, cx| {
                    this.state.set_body_state(
                        &id,
                        BodyState::Failed("No network connection.".to_owned()),
                    );
                    cx.notify();
                });
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.state.set_body_state(&id, BodyState::Downloading);
                cx.notify();
            });
            let account_path = real.account_path.clone();
            let mailbox_name = real.name.clone();
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    crate::live::fetch_and_store_body(
                        runtime,
                        account,
                        account_path,
                        mailbox_name,
                        mailbox_id,
                        row_id,
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                match outcome {
                    Ok((body, attachment)) => this.state.set_loaded_body(&id, body, attachment),
                    Err(message) => this.state.set_body_state(&id, BodyState::Failed(message)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Address completion candidates for a freshly opened compose window:
    /// every distinct sender this mailbox has seen (`compose::known_recipients`).
    fn compose_candidates(&self) -> Vec<rmac_mail::compose::Recipient> {
        rmac_mail::compose::known_recipients(&self.state.messages)
    }

    fn control(
        &self,
        id: &'static str,
        glyph: &'static str,
        label: &'static str,
        mode: ControlMode,
        click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut button = div()
            .id(id)
            .role(Role::Button)
            .aria_label(label)
            .w(px(30.0))
            .h(px(28.0))
            .rounded(px(mac::radius_control()))
            .flex()
            .items_center()
            .justify_center()
            .bg(if mode == ControlMode::Selected {
                mac::control_fill()
            } else {
                mac::material_clear()
            })
            .text_color(if mode != ControlMode::Disabled {
                mac::text()
            } else {
                mac::text_tertiary()
            })
            .text_size(px(17.0))
            .child(glyph);
        if mode != ControlMode::Disabled {
            button =
                button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| click(this, cx)));
        }
        button.into_any_element()
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut bar = div()
            .h(px(TOOLBAR))
            .pl(px(if self.sidebar_hidden {
                LIGHTS_WIDTH
            } else {
                SIDEBAR + 8.0
            }))
            .pr(px(10.0))
            .flex()
            .items_center()
            .gap(px(3.0))
            .border_b_1()
            .border_color(mac::separator());
        bar = bar.child(self.control(
            "mail-filter",
            "☷",
            "Filter",
            if self.state.filter.enabled {
                ControlMode::Selected
            } else {
                ControlMode::Enabled
            },
            |this, cx| {
                this.state.filter.enabled = !this.state.filter.enabled;
                this.sync_menu(cx);
                cx.notify();
            },
            cx,
        ));
        bar = bar.child(div().w(px(LIST - 66.0)));
        bar = bar.child(self.control(
            "mail-compose",
            "▣",
            "Compose",
            ControlMode::Enabled,
            |this, cx| {
                compose_window::open(
                    ComposeKind::New,
                    None,
                    None,
                    this.accounts.clone(),
                    this.compose_candidates(),
                    cx,
                );
            },
            cx,
        ));
        for (id, glyph, label, action) in [
            ("mail-archive", "▤", "Archive", OrganizeAction::Archive),
            ("mail-trash", "⌫", "Move to Bin", OrganizeAction::Delete),
            ("mail-junk", "⊗", "Junk or Not Junk", OrganizeAction::Junk),
        ] {
            // Some accounts (iCloud, for example) have no Archive mailbox,
            // so grey the button out there instead of a silent no-op click.
            let available = action != OrganizeAction::Archive || self.can_archive_selected();
            let mode = if available {
                ControlMode::Enabled
            } else {
                ControlMode::Disabled
            };
            bar = bar.child(self.control(
                id,
                glyph,
                label,
                mode,
                move |this, cx| this.perform(action.clone(), cx),
                cx,
            ));
        }
        let has_selection = self.state.selected_message().is_some();
        for (id, glyph, label, kind) in [
            ("mail-reply", "↶", "Reply", ComposeKind::Reply),
            ("mail-reply-all", "↞", "Reply All", ComposeKind::ReplyAll),
            ("mail-forward", "↷", "Forward", ComposeKind::Forward),
        ] {
            let mode = if has_selection {
                ControlMode::Enabled
            } else {
                ControlMode::Disabled
            };
            bar = bar.child(self.control(
                id,
                glyph,
                label,
                mode,
                move |this, cx| {
                    compose_window::open(
                        kind,
                        this.state.selected_message().cloned(),
                        None,
                        this.accounts.clone(),
                        this.compose_candidates(),
                        cx,
                    );
                },
                cx,
            ));
        }
        bar = bar.child(self.control(
            "mail-flag",
            "⚑",
            "Flag",
            ControlMode::Enabled,
            |this, cx| this.perform(OrganizeAction::Flag, cx),
            cx,
        ));
        bar = bar.child(self.control(
            "mail-move",
            "▱",
            "Move or Copy to Mailbox",
            ControlMode::Enabled,
            |this, cx| {
                this.destination_menu = !this.destination_menu;
                cx.notify();
            },
            cx,
        ));
        bar = bar.child(self.control(
            "mail-search",
            "⌕",
            "Search",
            ControlMode::Enabled,
            |this, cx| {
                this.search_open = !this.search_open;
                cx.notify();
            },
            cx,
        ));
        if self.search_open {
            bar = bar.child(
                div()
                    .id("mail-search-field")
                    .role(Role::SearchInput)
                    .aria_label("Search Mail")
                    .accessible_text_input(&self.search_input, cx)
                    .w(px(170.0))
                    .h(px(28.0))
                    .rounded(px(mac::radius_control()))
                    .bg(mac::control_fill())
                    .child(
                        TextField::new(&self.search_input)
                            .appearance(false)
                            .cleanable(true)
                            .small(),
                    ),
            );
            let all = self.state.search_scope == SearchScope::AllMailboxes;
            bar = bar.child(self.control(
                "mail-search-scope",
                if all { "All" } else { "Here" },
                if all {
                    "Search All Mailboxes"
                } else {
                    "Search Current Mailbox"
                },
                ControlMode::Enabled,
                |this, cx| {
                    this.state.search_scope =
                        if this.state.search_scope == SearchScope::AllMailboxes {
                            SearchScope::CurrentMailbox
                        } else {
                            SearchScope::AllMailboxes
                        };
                    cx.notify();
                },
                cx,
            ));
        }
        bar.into_any_element()
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div()
            .id("mail-sidebar-scroll")
            .role(Role::ListBox)
            .aria_label("Mailboxes")
            .absolute()
            .top(px(48.0))
            .left(px(10.0))
            .right(px(10.0))
            .bottom(px(12.0))
            .overflow_y_scroll()
            .flex()
            .flex_col();
        let mut account = String::new();
        for mailbox in self.state.mailboxes.clone() {
            if mailbox.account() != account {
                account = mailbox.account().to_owned();
                list = list.child(
                    div()
                        .h(px(30.0))
                        .pt(px(10.0))
                        .pl(px(8.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(11.0))
                        .text_color(mac::text_secondary())
                        .child(account.clone()),
                );
            }
            let selected = self.state.mailbox == mailbox;
            let count = self.state.unread_count(&mailbox);
            let label = if count > 0 {
                format!("{}, {} unread", mailbox.label(), count)
            } else {
                mailbox.label()
            };
            list = list.child(
                div()
                    .id(format!("mailbox-{mailbox:?}"))
                    .role(Role::ListBoxOption)
                    .aria_label(label)
                    .aria_selected(selected)
                    .h(px(27.0))
                    .px(px(8.0))
                    .rounded(px(mac::radius_control()))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .bg(if selected {
                        mac::accent()
                    } else {
                        mac::material_clear()
                    })
                    .text_color(if selected { mac::white() } else { mac::text() })
                    .text_size(px(12.0))
                    .child(div().w(px(16.0)).child(mailbox_icon(&mailbox)))
                    .child(mailbox.label())
                    .child(div().flex_1())
                    .child(if count > 0 {
                        count.to_string()
                    } else {
                        String::new()
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.state.select_mailbox(mailbox.clone());
                        this.ensure_body_loaded(cx);
                        this.dispatch_persist(cx);
                        this.sync_menu(cx);
                        cx.notify();
                    })),
            );
        }
        // The panel's own 8pt inset plus the measured traffic-light origin
        // (`rmac_ui::traffic_lights_origin`), matching Finder/Notes/System
        // Settings' sidebar panels — not flush against the panel's corner.
        let lights_origin = rmac_ui::traffic_lights_origin(true) - 8.0;
        div()
            .absolute()
            .left(px(8.0))
            .top(px(8.0))
            .bottom(px(8.0))
            .w(px(SIDEBAR - 8.0))
            .rounded(px(mac::radius_large_surface()))
            .bg(mac::material_sidebar())
            .border_1()
            .border_color(mac::separator())
            .child(
                div()
                    .absolute()
                    .left(px(lights_origin))
                    .top(px(lights_origin))
                    .child(rmac_ui::traffic_lights()),
            )
            .child(list)
            .into_any_element()
    }

    fn destinations(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(selected) = self.state.selected_message() else {
            return div().into_any_element();
        };
        let account = selected.mailbox.account();
        let mut panel = div()
            .id("mail-destination-menu")
            .role(Role::Menu)
            .aria_label(if self.copy_destination {
                "Copy to Mailbox"
            } else {
                "Move to Mailbox"
            })
            .absolute()
            .top(px(48.0))
            .right(px(104.0))
            .w(px(196.0))
            .p(px(6.0))
            .rounded(px(mac::radius_card()))
            .bg(mac::material_sidebar())
            .border_1()
            .border_color(mac::separator())
            .flex()
            .flex_col();
        for mailbox in self
            .state
            .mailboxes
            .clone()
            .into_iter()
            .filter(|mailbox| mailbox.is_real() && mailbox.account() == account)
        {
            let label = format!("{} · {}", mailbox.account(), mailbox.label());
            panel = panel.child(
                div()
                    .id(format!("mail-destination-{mailbox:?}"))
                    .role(Role::MenuItem)
                    .aria_label(label.clone())
                    .h(px(27.0))
                    .px(px(8.0))
                    .rounded(px(mac::radius_control()))
                    .text_size(px(12.0))
                    .text_color(mac::text())
                    .flex()
                    .items_center()
                    .child(label)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let action = if this.copy_destination {
                            OrganizeAction::Copy(mailbox.clone())
                        } else {
                            OrganizeAction::Move(mailbox.clone())
                        };
                        this.perform(action, cx);
                    })),
            );
        }
        panel.into_any_element()
    }

    /// The rows the conversation list shows, and how many are unread.
    fn list_rows(&self) -> (Arc<Vec<Row>>, usize) {
        let visible = self.state.visible();
        let selected_thread = self
            .state
            .selected_message()
            .map(|message| message.thread_id.clone());
        let highlight = self.highlight_conversations && !self.state.threads;
        // `MailState::thread_count` scans every message; calling it once
        // per visible row would make building the list O(n²) in a 10 000
        // -message mailbox (MAIL-10). One pass here keeps it O(n).
        let mut thread_counts: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::with_capacity(self.state.messages.len());
        for message in &self.state.messages {
            *thread_counts.entry(message.thread_id.as_str()).or_insert(0) += 1;
        }
        let rows: Arc<Vec<Row>> = Arc::new(
            visible
                .iter()
                .map(|&index| {
                    let message = &self.state.messages[index];
                    let to_cc = if !self.show_to_cc {
                        None
                    } else if self.state.addressed_to_me(&message.to_addresses) {
                        Some("To")
                    } else if self.state.addressed_to_me(&message.cc_addresses) {
                        Some("Cc")
                    } else {
                        None
                    };
                    Row {
                        id: message.id.clone(),
                        sender: message.sender.clone(),
                        initials: message.initials.clone(),
                        subject: message.subject.clone(),
                        preview: message.preview.clone(),
                        date: if self.show_date_time {
                            message.date_time.clone()
                        } else {
                            message.date.clone()
                        },
                        unread: message.unread,
                        flagged: message.flagged,
                        attachment: message.attachment.is_some(),
                        thread_count: thread_counts
                            .get(message.thread_id.as_str())
                            .copied()
                            .unwrap_or(1),
                        selected: self.state.selected.as_deref() == Some(message.id.as_str()),
                        to_cc,
                        highlighted: highlight
                            && selected_thread.as_deref() == Some(message.thread_id.as_str()),
                        flag_colour: message.flag_colour(),
                        size: if self.show_size && message.size > 0 {
                            rmac_mail::size_label(message.size)
                        } else {
                            String::new()
                        },
                    }
                })
                .collect(),
        );
        let unread = visible
            .iter()
            .filter(|&&index| self.state.messages[index].unread)
            .count();
        (rows, unread)
    }

    fn list(&self, rows: Arc<Vec<Row>>, unread: usize, view: gpui::WeakEntity<Self>) -> AnyElement {
        let count = rows.len();
        let style = RowStyle {
            columns: self.column_layout,
            contact_photo: self.show_contact_photo,
            display: self.display,
        };
        div()
            .id("mail-conversations")
            .role(Role::ListBox)
            .aria_label(format!("{count} conversations, {unread} unread"))
            .size_full()
            .flex()
            .flex_col()
            .bg(mac::list())
            .border_r_1()
            .border_color(mac::separator())
            .child(
                div()
                    .h(px(64.0))
                    .px(px(15.0))
                    .pt(px(10.0))
                    .flex()
                    .flex_col()
                    .border_b_1()
                    .border_color(mac::separator())
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(20.0))
                            .text_color(mac::text())
                            .child(self.state.mailbox.label()),
                    )
                    .child(
                        // MAIL-10/ACC-32: a named, polite live region, so
                        // Orca announces new mail arriving while focus is
                        // elsewhere (reading a message, say) — the list
                        // box's own name above only gets picked up when it
                        // already holds focus. `Role::Status` alone is not
                        // enough: confirmed live on the laptop, Orca needs
                        // AccessKit's `live`/`live_atomic` too (the same fix
                        // Calculator's display and Spotlight's result
                        // announcer use).
                        div()
                            .id("mail-conversation-count")
                            .role(Role::Status)
                            .aria_label(format!("{count} conversations, {unread} unread"))
                            .a11y_synthetic_children(|builder| {
                                builder.parent_node().set_live(accesskit::Live::Polite);
                                builder.parent_node().set_live_atomic();
                            })
                            .text_size(px(11.0))
                            .text_color(mac::text_secondary())
                            .child(format!("{count} conversations, {unread} unread")),
                    ),
            )
            .child(
                uniform_list(
                    "mail-message-list",
                    rows.len(),
                    move |range, _window, _cx| {
                        range
                            .map(|index| {
                                let row = &rows[index];
                                let id = row.id.clone();
                                let view = view.clone();
                                let flag = match (row.flagged, row.flag_colour) {
                                    (false, _) => String::new(),
                                    (true, rmac_mail::FlagColour::Red) => "flagged, ".to_owned(),
                                    (true, colour) => format!("flagged, {} flag, ", colour.label()),
                                };
                                let accessible = format!(
                                    "{}{}{}, {}, {}, {}{}",
                                    if row.unread { "Unread, " } else { "" },
                                    flag,
                                    row.sender,
                                    row.subject,
                                    row.date,
                                    row.preview,
                                    if row.attachment { ", 1 attachment" } else { "" }
                                );
                                let row_element = div()
                                    .id(format!("mail-message-{}", row.id))
                                    .role(Role::ListBoxOption)
                                    .aria_label(accessible)
                                    .aria_selected(row.selected)
                                    .bg(if row.selected {
                                        mac::accent_subtle()
                                    } else if row.highlighted {
                                        mac::hover()
                                    } else {
                                        mac::material_clear()
                                    })
                                    .border_b_1()
                                    .border_color(mac::separator());
                                let row_element = if style.columns {
                                    column_row(row_element, row, style)
                                } else {
                                    card_row(row_element, row, style)
                                };
                                row_element.on_click(move |_: &ClickEvent, _, cx| {
                                    let _ = view.update(cx, |this, cx| {
                                        this.state.select(&id);
                                        this.ensure_body_loaded(cx);
                                        this.dispatch_persist(cx);
                                        this.sync_menu(cx);
                                        cx.notify();
                                    });
                                })
                            })
                            .collect()
                    },
                )
                .flex_1(),
            )
            .into_any_element()
    }

    fn rich_body(
        body: &RichText,
        display: &rmac_mail::settings::DisplayPrefs,
        find: Option<tools::FindHighlight<'_>>,
    ) -> AnyElement {
        // Viewing ▸ "Use dark backgrounds for messages" off: in Dark Mode
        // the message sits on white paper with black text, as on the Mac.
        let paper = !display.dark_message_backgrounds && mac::is_dark();
        let text = if paper { mac::black() } else { mac::text() };
        let mut content = div().flex().flex_col().gap(px(11.0));
        if paper {
            content = content
                .p(px(16.0))
                .rounded(px(mac::radius_card()))
                .bg(mac::white());
        }
        for (index, block) in body.blocks.iter().enumerate() {
            let mut line = div()
                .id(format!("mail-block-{index}"))
                .flex()
                .flex_wrap()
                .text_size(px(display.message_font_size))
                .text_color(text);
            if block.kind == BlockKind::Quote {
                // Fonts & Colours ▸ "Colour quoted text".
                let (bar, quote_text) = if display.colour_quoted_text {
                    (mac::system_blue(), mac::system_blue())
                } else if paper {
                    (mac::separator(), mac::black())
                } else {
                    (mac::separator(), mac::text_secondary())
                };
                line = line
                    .border_l_3()
                    .border_color(bar)
                    .pl(px(10.0))
                    .text_color(quote_text);
            }
            if block.kind == BlockKind::ListItem {
                line = line.child("• ");
            }
            for (span_index, span) in block.spans.iter().enumerate() {
                let style_piece = |mut piece: gpui::Div| {
                    if span.bold {
                        piece = piece.font_weight(FontWeight::BOLD);
                    }
                    if span.italic {
                        piece = piece.italic();
                    }
                    if span.underline || span.link.is_some() {
                        piece = piece.underline();
                    }
                    if span.link.is_some() {
                        piece = piece.text_color(mac::system_blue());
                    }
                    piece
                };
                // Edit ▸ Find marks every match in the span, the current
                // one strongest, and scrolls to the current one.
                let here: Vec<(usize, &rmac_mail::FindMatch)> = find
                    .as_ref()
                    .map(|find| {
                        find.matches
                            .iter()
                            .enumerate()
                            .filter(|(_, found)| found.block == index && found.span == span_index)
                            .collect()
                    })
                    .unwrap_or_default();
                if here.is_empty() {
                    line =
                        line.child(style_piece(div()).child(SharedString::from(span.text.clone())));
                    continue;
                }
                let mut at = 0;
                for (match_index, found) in here {
                    if found.range.start > at {
                        line = line.child(style_piece(div()).child(SharedString::from(
                            span.text[at..found.range.start].to_owned(),
                        )));
                    }
                    let current = find
                        .as_ref()
                        .is_some_and(|find| find.current == match_index);
                    let mut piece = style_piece(div())
                        .rounded(px(2.0))
                        .bg(if current {
                            mac::system_yellow()
                        } else {
                            mac::system_yellow().opacity(0.35)
                        })
                        .when(current, |piece| piece.text_color(mac::black()))
                        .child(SharedString::from(
                            span.text[found.range.clone()].to_owned(),
                        ))
                        .id(("mail-find-match", match_index));
                    if current {
                        piece = piece.anchor_scroll(find.as_ref().map(|find| find.anchor.clone()));
                    }
                    line = line.child(piece);
                    at = found.range.end;
                }
                if at < span.text.len() {
                    line = line.child(
                        style_piece(div()).child(SharedString::from(span.text[at..].to_owned())),
                    );
                }
            }
            content = content.child(line);
        }
        content.into_any_element()
    }

    fn viewer(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let matches = self.find_matches(cx);
        if self.find_scroll && !matches.is_empty() {
            // The current match is drawn this frame with the anchor; the
            // viewer scrolls to it on the next.
            self.find_scroll = false;
            self.find_anchor.scroll_to(window, cx);
        }
        let find_bar = self.find_open.then(|| self.find_bar(matches.len(), cx));
        let viewer = self.message_viewer(&matches, cx);
        match find_bar {
            Some(bar) => div()
                .flex_1()
                .h_full()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .child(bar)
                .child(div().flex_1().min_h(px(0.0)).flex().child(viewer))
                .into_any_element(),
            None => viewer,
        }
    }

    fn message_viewer(
        &self,
        matches: &[rmac_mail::FindMatch],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(message) = self.state.selected_message() else {
            return div()
                .flex_1()
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(mac::text_secondary())
                .child("No message selected")
                .into_any_element();
        };
        let mut body = div()
            .id("mail-viewer-scroll")
            .track_scroll(&self.viewer_scroll)
            .role(Role::Document)
            .aria_label(format!("{}, {}", message.sender, message.subject))
            .flex_1()
            .h_full()
            .overflow_y_scroll()
            .px(px(24.0))
            .py(px(20.0))
            .flex()
            .flex_col()
            .gap(px(16.0))
            .bg(mac::window())
            .child(Self::header(message));
        if let Some(headers) = self.all_headers_block() {
            body = body.child(headers);
        }
        if !message.body.blocked_remote_images.is_empty() {
            body = body.child(
                div()
                    .py(px(8.0))
                    .border_b_1()
                    .border_color(mac::separator())
                    .text_size(px(12.0))
                    .text_color(mac::text_secondary())
                    .child("This message contains remote content. Images are blocked."),
            );
        }
        match &message.body_state {
            BodyState::Loaded => {
                let find = (!matches.is_empty()).then(|| tools::FindHighlight {
                    matches,
                    current: self.find_current.min(matches.len() - 1),
                    anchor: &self.find_anchor,
                });
                body = body.child(Self::rich_body(&message.body, &self.display, find));
                if let Some(attachment) = message.attachment.clone() {
                    body = body.child(self.attachment_chip(attachment, cx));
                }
            }
            BodyState::Downloading => {
                body = body.child(
                    div()
                        .id("mail-body-downloading")
                        .role(Role::Status)
                        .aria_label("Downloading message…")
                        // ACC-32: same live-region fix as the heading above.
                        .a11y_synthetic_children(|builder| {
                            builder.parent_node().set_live(accesskit::Live::Polite);
                            builder.parent_node().set_live_atomic();
                        })
                        .text_size(px(13.0))
                        .text_color(mac::text_secondary())
                        .child("Downloading message…"),
                );
            }
            BodyState::Failed(error) => {
                let error = error.clone();
                body = body.child(
                    div()
                        .id("mail-body-failed")
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .child(
                            div()
                                .id("mail-body-failed-message")
                                .role(Role::Status)
                                .aria_label(error.clone())
                                // ACC-32: same live-region fix as the heading above.
                                .a11y_synthetic_children(|builder| {
                                    builder.parent_node().set_live(accesskit::Live::Assertive);
                                    builder.parent_node().set_live_atomic();
                                })
                                .text_size(px(13.0))
                                .text_color(mac::text_secondary())
                                .child(error),
                        )
                        .child(
                            div()
                                .id("mail-body-retry")
                                .role(Role::Button)
                                .aria_label("Retry")
                                .cursor_pointer()
                                .w(px(70.0))
                                .h(px(26.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(mac::radius_control()))
                                .bg(mac::control_fill())
                                .text_color(mac::text())
                                .text_size(px(12.0))
                                .child("Retry")
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.load_body_for_selected(cx);
                                })),
                        ),
                );
            }
            BodyState::NotLoaded | BodyState::Loading => {
                body = body.child(
                    div()
                        .id("mail-body-loading")
                        .role(Role::Status)
                        .aria_label("Loading message…")
                        // ACC-32: same live-region fix as the heading above.
                        .a11y_synthetic_children(|builder| {
                            builder.parent_node().set_live(accesskit::Live::Polite);
                            builder.parent_node().set_live_atomic();
                        })
                        .text_size(px(13.0))
                        .text_color(mac::text_secondary())
                        .child("Loading message…"),
                );
            }
        }
        body.into_any_element()
    }

    /// An attachment chip. A `.ics` invite (MAIL-8) is also a button that
    /// stages the attachment and hands it to Calendar through the OpenURI
    /// portal, which routes to Calendar because `text/calendar` is
    /// registered to `org.rmac.Calendar.desktop` (`packaging/rmac-apps`).
    fn attachment_chip(
        &self,
        attachment: rmac_mail::MessageAttachment,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_calendar_invite = rmac_mail::ics::has_ics_extension(&attachment.filename);
        let mut chip = div()
            .id("mail-attachment")
            .w(px(170.0))
            .h(px(46.0))
            .rounded(px(mac::radius_card()))
            .bg(mac::control_fill())
            .px(px(10.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .text_size(px(12.0))
            .text_color(mac::text())
            .child(if is_calendar_invite { "▦" } else { "▤" })
            .child(format!(
                "{} · {}",
                attachment.filename, attachment.size_label
            ));
        if is_calendar_invite {
            chip = chip
                .role(Role::Button)
                .aria_label(format!("Add {} to Calendar", attachment.filename))
                .cursor_pointer()
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                    let filename = attachment.filename.clone();
                    let bytes = attachment.bytes.clone();
                    cx.spawn(async move |_, _cx| {
                        let staged = rmac_mail::ics::stage_for_handoff(&filename, &bytes);
                        match staged {
                            Ok(path) => {
                                if let Err(error) = rmac_portal::open_item(&path).await {
                                    eprintln!(
                                        "rmac-mail: could not hand {filename} to Calendar: {error}"
                                    );
                                }
                            }
                            Err(error) => {
                                eprintln!(
                                    "rmac-mail: could not stage {filename} for Calendar: {error}"
                                );
                            }
                        }
                    })
                    .detach();
                }));
        }
        chip.into_any_element()
    }

    fn header(message: &Message) -> AnyElement {
        let mut details = format!("To: {}", message.to);
        if !message.cc.is_empty() {
            details.push_str(&format!("  ·  Cc: {}", message.cc));
        }
        div()
            .flex()
            .gap(px(10.0))
            .pb(px(16.0))
            .border_b_1()
            .border_color(mac::separator())
            .child(
                div()
                    .w(px(40.0))
                    .h(px(40.0))
                    .rounded_full()
                    .bg(mac::system_gray())
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(mac::white())
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(message.initials.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(15.0))
                            .text_color(mac::text())
                            .child(message.sender.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(mac::text_secondary())
                            .child(details),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(13.0))
                            .text_color(mac::text())
                            .child(message.subject.clone()),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(mac::text_secondary())
                    .child(message.date.clone()),
            )
            .into_any_element()
    }

    /// Shown instead of the message list/viewer when GOA has no mail
    /// account yet. Mail must never show fixture or sample data to a real
    /// user — this, not a fake mailbox, is what someone with no account
    /// sees (`docs/design/calendar-mail.md` §3). macOS Mail keeps its
    /// normal three-pane chrome (toolbar, sidebar) around this prompt
    /// rather than blanking the whole window, so this reuses the same
    /// `toolbar`/`sidebar` the populated window renders — both already
    /// degrade safely with zero accounts (every control just goes idle).
    fn empty_state(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .size_full()
            .relative()
            .bg(mac::window())
            .track_focus(&self.focus)
            .key_context("Mail")
            // Close must work even with no account configured yet: this is
            // the only root the empty state renders, so it needs its own
            // close-action handling — the normal three-pane root (below)
            // isn't mounted while `self.accounts` is empty.
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .on_action(
                cx.listener(|_, _: &rmac_ui::RequestClose, window, _| window.remove_window()),
            )
            // Settings… (⌘,) opens with no account too, as on the Mac: it
            // is where accounts are added. Before the 2026-10-13 live robot
            // run, the menu row and ⌘, did nothing until an account existed.
            .on_action(cx.listener(|_, _: &crate::ShowSettings, _, cx| {
                cx.defer(crate::settings_view::show);
            }))
            .on_action(
                cx.listener(|_, _: &crate::ShowAccounts, _, cx| menu::open_internet_accounts(cx)),
            )
            .child(self.toolbar(cx))
            .child(
                div()
                    .id("mail-empty-state")
                    .role(Role::Group)
                    .aria_label("No Mail Accounts")
                    .absolute()
                    .left(px(SIDEBAR + 8.0))
                    .right_0()
                    .top(px(TOOLBAR))
                    .bottom_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(14.0))
                    .child(
                        div()
                            .text_size(px(34.0))
                            .text_color(mac::text_tertiary())
                            .child("✉"),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(17.0))
                            .text_color(mac::text())
                            .child("No Mail Accounts"),
                    )
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(mac::text_secondary())
                            .child("Add an account to send and receive mail."),
                    )
                    .child(
                        div()
                            .id("mail-empty-add-account")
                            .role(Role::Button)
                            .aria_label("Add Account…")
                            .cursor_pointer()
                            .mt(px(6.0))
                            .px(px(16.0))
                            .h(px(28.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(mac::radius_control()))
                            .bg(mac::accent())
                            .text_color(mac::white())
                            .text_size(px(13.0))
                            .child("Add Account…")
                            // Windows has no Internet Accounts: Mail's own
                            // Settings ▸ Accounts adds the account there.
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                                menu::open_internet_accounts(cx)
                            })),
                    ),
            )
            .child(self.sidebar(cx))
            .into_any_element()
    }
}

impl Render for MailView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if window.is_window_active() {
            // Writing Tools work in a compose window, never on the mailbox.
            rmac_writing_tools::publish_menu_state(false, cx);
        }
        if self.accounts.is_empty() {
            return self.empty_state(cx);
        }
        self.sync_menu(cx);
        let entity = cx.entity();
        let list_view = self
            .list_view
            .get_or_insert_with(|| cx.new(|cx| MailListView::new(&entity, cx)))
            .clone();
        // A cached view is laid out from this style. While an assistive
        // technology is connected every frame rebuilds the accessibility
        // tree from prepaint, which a reused subtree skips.
        let list = if window.is_a11y_active() {
            list_view.into_any_element()
        } else {
            list_view
                .cached(gpui::StyleRefinement::default().size_full())
                .into_any_element()
        };
        let viewer = self.viewer(window, cx);
        let root = div()
            .size_full()
            .relative()
            .bg(mac::window())
            .track_focus(&self.focus)
            .key_context("Mail")
            .on_action(cx.listener(|this, _: &NewMessage, _, cx| {
                compose_window::open(
                    ComposeKind::New,
                    None,
                    None,
                    this.accounts.clone(),
                    this.compose_candidates(),
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &Reply, _, cx| {
                compose_window::open(
                    ComposeKind::Reply,
                    this.state.selected_message().cloned(),
                    None,
                    this.accounts.clone(),
                    this.compose_candidates(),
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &ReplyAll, _, cx| {
                compose_window::open(
                    ComposeKind::ReplyAll,
                    this.state.selected_message().cloned(),
                    None,
                    this.accounts.clone(),
                    this.compose_candidates(),
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &Forward, _, cx| {
                compose_window::open(
                    ComposeKind::Forward,
                    this.state.selected_message().cloned(),
                    None,
                    this.accounts.clone(),
                    this.compose_candidates(),
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &ToggleThreads, _, cx| {
                this.state.threads = !this.state.threads;
                this.sync_menu(cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleRead, _, cx| {
                let action = if this
                    .state
                    .selected_message()
                    .is_some_and(|message| message.unread)
                {
                    OrganizeAction::MarkRead
                } else {
                    OrganizeAction::MarkUnread
                };
                this.perform(action, cx);
            }))
            .on_action(
                cx.listener(|this, _: &Archive, _, cx| this.perform(OrganizeAction::Archive, cx)),
            )
            .on_action(cx.listener(|this, _: &Delete, window, cx| {
                // Edit ▸ Delete edits the search field while it has focus.
                if this
                    .search_input
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
                {
                    if let Ok(action) = cx.build_action("input::Delete", None) {
                        window.dispatch_action(action, cx);
                    }
                } else {
                    this.perform(OrganizeAction::Delete, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Junk, _, cx| this.perform(OrganizeAction::Junk, cx)))
            .on_action(cx.listener(|this, _: &Flag, _, cx| this.perform(OrganizeAction::Flag, cx)))
            .on_action(cx.listener(|this, _: &Undo, _, cx| {
                if this.state.undo() {
                    this.dispatch_persist(cx);
                    this.sync_menu(cx);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &Move, _, cx| {
                this.copy_destination = false;
                this.destination_menu = true;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Copy, _, cx| {
                this.copy_destination = true;
                this.destination_menu = true;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Search, window, cx| {
                this.search_open = true;
                this.search_input
                    .update(cx, |state, cx| state.focus(window, cx));
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &NextMessage, _, cx| {
                this.state.select_next(1);
                this.after_selection(cx);
            }))
            .on_action(cx.listener(|this, _: &PreviousMessage, _, cx| {
                this.state.select_next(-1);
                this.after_selection(cx);
            }))
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .on_action(
                cx.listener(|_, _: &rmac_ui::RequestClose, window, _| window.remove_window()),
            )
            .on_action(cx.listener(|_, _: &crate::ShowSettings, _, cx| {
                cx.defer(crate::settings_view::show);
            }))
            .when(!self.toolbar_hidden, |root| root.child(self.toolbar(cx)))
            .child(self.content(list, viewer))
            .when(!self.sidebar_hidden, |root| root.child(self.sidebar(cx)))
            .when(self.sidebar_hidden, |root| {
                // The window controls move to the top left of the window.
                let origin = rmac_ui::traffic_lights_origin(true);
                root.child(
                    div()
                        .absolute()
                        .left(px(origin))
                        .top(px(origin))
                        .child(rmac_ui::traffic_lights()),
                )
            })
            .when(self.destination_menu, |root| {
                root.child(self.destinations(cx))
            })
            .when_some(self.erase_confirm, |root, erase| {
                root.child(self.erase_alert(erase, cx))
            })
            .when_some(self.mailbox_sheet_view(cx), |root, sheet| root.child(sheet))
            .when_some(self.notice.clone(), |root, notice| {
                root.child(self.notice_alert(notice, cx))
            });
        let root = self.on_mailbox_actions(root, cx);
        self.on_menu_actions(root, cx).into_any_element()
    }
}

/// The status column every row starts with: unread dot or flag.
/// Blue for unread, the flag's own colour for a flag.
fn status_colour(row: &Row) -> gpui::Hsla {
    if !row.unread && row.flagged {
        tools::flag_hsla(row.flag_colour)
    } else {
        mac::system_blue()
    }
}

fn status_mark(row: &Row) -> &'static str {
    if row.unread {
        "●"
    } else if row.flagged {
        "⚑"
    } else {
        ""
    }
}

fn avatar(initials: &str, size: f32) -> gpui::Div {
    div()
        .flex_none()
        .w(px(size))
        .h(px(size))
        .rounded_full()
        .bg(mac::system_gray())
        .flex()
        .items_center()
        .justify_center()
        .text_color(mac::white())
        .text_size(px(size * 0.4))
        .font_weight(FontWeight::SEMIBOLD)
        .child(initials.to_owned())
}

fn to_cc_badge(label: &'static str) -> gpui::Div {
    div()
        .flex_none()
        .px(px(4.0))
        .rounded(px(mac::radius_control()))
        .bg(mac::control_fill())
        .text_size(px(9.0))
        .text_color(mac::text_secondary())
        .child(label)
}

/// The Mac's default list row: sender and date, subject, two lines of
/// preview.
fn card_row(
    element: gpui::Stateful<gpui::Div>,
    row: &Row,
    style: RowStyle,
) -> gpui::Stateful<gpui::Div> {
    element
        .h(px(style.display.list_row_height()))
        .px(px(13.0))
        .pt(px(7.0))
        .flex()
        .gap(px(7.0))
        .child(
            div()
                .w(px(9.0))
                .pt(px(9.0))
                .text_size(px(12.0))
                .text_color(status_colour(row))
                .child(status_mark(row)),
        )
        .when(style.contact_photo, |element| {
            element.child(div().pt(px(2.0)).child(avatar(&row.initials, 30.0)))
        })
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .gap(px(5.0))
                        .child(
                            div()
                                .flex()
                                .gap(px(4.0))
                                .items_center()
                                .overflow_hidden()
                                .child(
                                    div()
                                        .font_weight(if row.unread && style.display.unread_bold {
                                            FontWeight::BOLD
                                        } else {
                                            FontWeight::NORMAL
                                        })
                                        .text_size(px(style.display.list_font_size))
                                        .text_color(mac::text())
                                        .child(row.sender.clone()),
                                )
                                .when_some(row.to_cc, |element, label| {
                                    element.child(to_cc_badge(label))
                                }),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(px(10.0))
                                .text_color(mac::text_secondary())
                                .child(row.date.clone()),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(4.0))
                        .text_size(px(style.display.list_font_size - 1.0))
                        .text_color(mac::text())
                        .child(row.subject.clone())
                        .child(if row.thread_count > 1 {
                            format!("({})", row.thread_count)
                        } else {
                            String::new()
                        })
                        .child(if row.attachment { "⌁" } else { "" })
                        .when(!row.size.is_empty(), |line| {
                            line.child(
                                div()
                                    .ml_auto()
                                    .flex_none()
                                    .text_size(px(10.0))
                                    .text_color(mac::text_secondary())
                                    .child(row.size.clone()),
                            )
                        }),
                )
                .when(style.display.preview_lines > 0, |column| {
                    column.child(
                        div()
                            .text_size(px(style.display.list_font_size - 2.0))
                            .text_color(mac::text_secondary())
                            .overflow_hidden()
                            .line_clamp(usize::from(style.display.preview_lines))
                            .child(row.preview.clone()),
                    )
                }),
        )
}

/// View ▸ Use Column Layout: one line per message, in From, Subject and
/// Date Received columns.
fn column_row(
    element: gpui::Stateful<gpui::Div>,
    row: &Row,
    style: RowStyle,
) -> gpui::Stateful<gpui::Div> {
    let weight = if row.unread && style.display.unread_bold {
        FontWeight::SEMIBOLD
    } else {
        FontWeight::NORMAL
    };
    element
        .h(px(COLUMN_ROW))
        .px(px(10.0))
        .flex()
        .items_center()
        .gap(px(8.0))
        .text_size(px(style.display.list_font_size - 1.0))
        .text_color(mac::text())
        .font_weight(weight)
        .child(
            div()
                .flex_none()
                .w(px(12.0))
                .text_color(status_colour(row))
                .child(status_mark(row)),
        )
        .child(
            div()
                .flex_none()
                .w(px(12.0))
                .text_color(mac::text_secondary())
                .child(if row.attachment { "⌁" } else { "" }),
        )
        .when(style.contact_photo, |element| {
            element.child(avatar(&row.initials, 18.0))
        })
        .child(
            div()
                .flex_none()
                .w(px(170.0))
                .overflow_hidden()
                .flex()
                .gap(px(4.0))
                .items_center()
                .child(row.sender.clone())
                .when_some(row.to_cc, |element, label| {
                    element.child(to_cc_badge(label))
                }),
        )
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .child(if row.thread_count > 1 {
                    format!("{} ({})", row.subject, row.thread_count)
                } else {
                    row.subject.clone()
                }),
        )
        .when(!row.size.is_empty(), |element| {
            element.child(
                div()
                    .flex_none()
                    .w(px(64.0))
                    .text_size(px(11.0))
                    .text_color(mac::text_secondary())
                    .child(row.size.clone()),
            )
        })
        .child(
            div()
                .flex_none()
                .text_size(px(11.0))
                .text_color(mac::text_secondary())
                .child(row.date.clone()),
        )
}

fn mailbox_icon(mailbox: &Mailbox) -> &'static str {
    match mailbox {
        Mailbox::AllInboxes => "▤",
        Mailbox::Flagged => "⚑",
        Mailbox::Drafts => "✎",
        Mailbox::Sent => "➤",
        Mailbox::Real(real) => match real.special_use {
            Some(SpecialUse::Inbox) => "▤",
            Some(SpecialUse::Drafts) => "✎",
            Some(SpecialUse::Sent) => "➤",
            Some(SpecialUse::Junk) => "⊗",
            Some(SpecialUse::Trash) => "⌫",
            Some(SpecialUse::Archive) => "▥",
            None => "▱",
        },
        Mailbox::Smart(_) => "⚙",
    }
}
