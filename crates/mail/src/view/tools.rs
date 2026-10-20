//! The second half of Mac Mail's menus (MROBOT-03 follow-up): File ▸ New
//! Viewer Window and Quick Look Attachments…, Edit ▸ Find inside a
//! message, View ▸ Message ▸ All Headers and Message ▸ Flag's colours.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{
    div, px, AnyElement, AnyWindowHandle, App, AppContext as _, ClickEvent, Context, Entity, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window,
};
use rmac_mail::{FindMatch, FlagColour, MailState};
use rmac_ui::{app_id::MAIL, mac, AccessibleTextInput as _, Root, TextField};

use super::MailView;
use crate::delivery::ComposeAccount;

thread_local! {
    /// Every open message viewer, in the order they opened: the first is
    /// the one Window ▸ Message Viewer brings forward.
    static VIEWERS: RefCell<Vec<(AnyWindowHandle, WeakEntity<MailView>)>> =
        const { RefCell::new(Vec::new()) };
}

/// Opens a message viewer window over `state`. Every viewer is listed for
/// the sync events and for the changes another viewer makes.
pub fn open_viewer(
    state: MailState,
    accounts: Vec<ComposeAccount>,
    runtime: Option<Arc<rmac_mail_runtime::Runtime>>,
    cx: &mut App,
) -> Option<Entity<MailView>> {
    let mut options = rmac_ui::window_options_for_app(MAIL, 1200.0, 750.0, cx);
    options.window_min_size = Some(gpui::size(px(860.0), px(540.0)));
    let mut created = None;
    let opened = cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        rmac_ui::fit_to_display_after_first_frame(window, cx);
        let view = cx.new(|cx| {
            rmac_ui::observe_window_state(MAIL, window, cx);
            MailView::new(state, accounts, runtime, window, cx)
        });
        created = Some(view.clone());
        let focus = view.read(cx).focus.clone();
        window.focus(&focus, cx);
        cx.new(|cx| Root::new(view, window, cx))
    });
    match opened {
        Ok(handle) => {
            let view = created?;
            VIEWERS.with(|viewers| viewers.borrow_mut().push((handle.into(), view.downgrade())));
            Some(view)
        }
        Err(error) => {
            eprintln!("rmac-mail: could not open a window: {error}");
            None
        }
    }
}

/// The viewers still open, first opened first.
pub fn viewers() -> Vec<Entity<MailView>> {
    VIEWERS.with(|viewers| {
        let mut viewers = viewers.borrow_mut();
        viewers.retain(|(_, view)| view.upgrade().is_some());
        viewers
            .iter()
            .filter_map(|(_, view)| view.upgrade())
            .collect()
    })
}

/// Window ▸ Message Viewer (⌘0): brings the first open viewer to the front.
pub fn show_viewer(cx: &mut App) {
    let handle = VIEWERS.with(|viewers| {
        viewers
            .borrow()
            .iter()
            .find(|(_, view)| view.upgrade().is_some())
            .map(|(handle, _)| *handle)
    });
    if let Some(handle) = handle {
        let _ = handle.update(cx, |_, window, _| window.activate_window());
    }
}

/// The colour a flag shows in the list and the Flag menu.
pub(super) fn flag_hsla(colour: FlagColour) -> Hsla {
    match colour {
        FlagColour::Red => mac::system_red(),
        FlagColour::Orange => mac::system_orange(),
        FlagColour::Yellow => mac::system_yellow(),
        FlagColour::Green => mac::system_green(),
        FlagColour::Blue => mac::system_blue(),
        FlagColour::Purple => mac::system_purple(),
        FlagColour::Grey => mac::system_gray(),
    }
}

/// The action name of Message ▸ Flag ▸ `colour`.
pub(super) fn flag_action(colour: FlagColour) -> &'static str {
    match colour {
        FlagColour::Red => "mail::FlagRed",
        FlagColour::Orange => "mail::FlagOrange",
        FlagColour::Yellow => "mail::FlagYellow",
        FlagColour::Green => "mail::FlagGreen",
        FlagColour::Blue => "mail::FlagBlue",
        FlagColour::Purple => "mail::FlagPurple",
        FlagColour::Grey => "mail::FlagGrey",
    }
}

/// Writes an attachment where Quick Look can read it: a folder of this
/// Mail process's own, readable by this person only.
fn stage_attachment(name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let folder = std::env::temp_dir().join(format!("lulo-mail-quick-look-{}", std::process::id()));
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(&folder)?;
    let path = folder.join(name);
    write_private(&path, bytes)?;
    Ok(path)
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use rmac_storage::Backend as _;
    rmac_storage::FileSystem.write_atomic(path, bytes)
}

/// What the viewer draws for Edit ▸ Find: every match, and which one is
/// current.
pub(super) struct FindHighlight<'a> {
    pub matches: &'a [FindMatch],
    pub current: usize,
    pub anchor: &'a gpui::ScrollAnchor,
}

impl MailView {
    /// File ▸ New Viewer Window (⌥⌘N): another viewer over the same mail.
    pub(super) fn new_viewer(&mut self, cx: &mut Context<Self>) {
        let mut state = MailState::new(self.state.mailboxes.clone(), self.state.messages.clone());
        state.my_addresses = self.state.my_addresses.clone();
        let accounts = self.accounts.clone();
        let runtime = self.runtime.clone();
        cx.defer(move |cx| {
            open_viewer(state, accounts, runtime, cx);
        });
    }

    /// Hands what changed here to the other viewers, so a message read,
    /// moved or erased in one window is the same in all of them.
    pub(super) fn share_changes(&self, cx: &mut Context<Self>) {
        if self.state.revision == self.shared_revision.get() {
            return;
        }
        self.shared_revision.set(self.state.revision);
        let me = cx.entity_id();
        let others: Vec<Entity<MailView>> = viewers()
            .into_iter()
            .filter(|view| view.entity_id() != me)
            .collect();
        if others.is_empty() {
            return;
        }
        let mailboxes = self.state.mailboxes.clone();
        let messages = self.state.messages.clone();
        cx.defer(move |cx| {
            for other in others {
                other.update(cx, |view, cx| {
                    view.state.refresh_live(mailboxes.clone(), messages.clone());
                    view.shared_revision.set(view.state.revision);
                    view.sync_menu(cx);
                    cx.notify();
                });
            }
        });
    }

    /// File ▸ Quick Look Attachments… (⌘Y): the selected message's
    /// attachment in Quick Look, or Quick Look closed if it is open.
    pub(super) fn quick_look(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.quick_look.take() {
            if handle.is_open() {
                handle.close(cx);
                return;
            }
        }
        let Some(attachment) = self
            .state
            .selected_message()
            .filter(|message| message.body_state == rmac_mail::BodyState::Loaded)
            .and_then(|message| message.attachment.clone())
        else {
            return;
        };
        let name = rmac_mail::safe_file_stem(&attachment.filename);
        let staged = cx
            .background_executor()
            .spawn(async move { stage_attachment(&name, &attachment.bytes) });
        cx.spawn(async move |this, cx| {
            let staged = staged.await;
            let _ = this.update(cx, |this, cx| match staged {
                Ok(path) => {
                    let options = rmac_quick_look::Options::default();
                    if let Some((handle, _)) = rmac_quick_look::open(vec![path], 0, options, cx) {
                        this.quick_look = Some(handle);
                    }
                }
                Err(error) => {
                    this.notice =
                        Some(format!("Mail couldn’t show the attachment: {error}.").into());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The matches for the find bar's text in the selected message.
    pub(super) fn find_matches(&self, cx: &App) -> Vec<FindMatch> {
        if !self.find_open {
            return Vec::new();
        }
        let query = self.find_input.read(cx).value().to_string();
        self.state
            .selected_message()
            .filter(|message| message.body_state == rmac_mail::BodyState::Loaded)
            .map(|message| rmac_mail::find_in_body(&message.body, &query))
            .unwrap_or_default()
    }

    /// Edit ▸ Find ▸ Find… (⌘F): the find bar over the message, focused.
    pub(super) fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_open = true;
        self.find_input
            .update(cx, |state, cx| state.focus(window, cx));
        self.find_scroll = true;
        self.sync_menu(cx);
        cx.notify();
    }

    pub(super) fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_open = false;
        window.focus(&self.focus, cx);
        self.sync_menu(cx);
        cx.notify();
    }

    /// Edit ▸ Find ▸ Find Next (⌘G) and Find Previous (⇧⌘G), wrapping
    /// round as the Mac does.
    pub(super) fn step_find(&mut self, step: isize, cx: &mut Context<Self>) {
        let count = self.find_matches(cx).len();
        if count == 0 {
            return;
        }
        let current = self.find_current.min(count - 1) as isize;
        self.find_current = (current + step).rem_euclid(count as isize) as usize;
        self.find_scroll = true;
        cx.notify();
    }

    /// Edit ▸ Find ▸ Jump to Selection (⌘J): back to the current match.
    pub(super) fn jump_to_find(&mut self, cx: &mut Context<Self>) {
        if !self.find_matches(cx).is_empty() {
            self.find_scroll = true;
            cx.notify();
        }
    }

    /// The find bar above the message: the search field, "2 of 5", the
    /// previous/next buttons and Done.
    pub(super) fn find_bar(&self, matches: usize, cx: &mut Context<Self>) -> AnyElement {
        let query_empty = self.find_input.read(cx).value().is_empty();
        let status: SharedString = if query_empty {
            SharedString::default()
        } else if matches == 0 {
            "Not found".into()
        } else {
            format!("{} of {matches}", self.find_current.min(matches - 1) + 1).into()
        };
        let step_button = |id: &'static str,
                           glyph: &'static str,
                           label: &'static str,
                           step: isize| {
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(label)
                .w(px(24.0))
                .h(px(22.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(mac::radius_control()))
                .bg(mac::control_fill())
                .text_color(if matches > 0 {
                    mac::text()
                } else {
                    mac::text_tertiary()
                })
                .child(glyph)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.step_find(step, cx)))
        };
        let previous = step_button("mail-find-previous", "‹", "Previous", -1);
        let next = step_button("mail-find-next", "›", "Next", 1);
        div()
            .id("mail-find-bar")
            .role(Role::Toolbar)
            .aria_label("Find")
            .key_context("MailFind")
            .h(px(34.0))
            .flex_none()
            .px(px(12.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .border_b_1()
            .border_color(mac::separator())
            .bg(mac::window())
            .child(
                div()
                    .id("mail-find-field")
                    .role(Role::SearchInput)
                    .aria_label("Find in Message")
                    .accessible_text_input(&self.find_input, cx)
                    .w(px(220.0))
                    .h(px(24.0))
                    .rounded(px(mac::radius_control()))
                    .bg(mac::control_fill())
                    .child(
                        TextField::new(&self.find_input)
                            .appearance(false)
                            .cleanable(true)
                            .small(),
                    ),
            )
            .child(previous)
            .child(next)
            .child(
                div()
                    .id("mail-find-status")
                    .role(Role::Status)
                    .aria_label(if status.is_empty() {
                        SharedString::from("Find")
                    } else {
                        status.clone()
                    })
                    .flex_1()
                    .text_size(px(11.0))
                    .text_color(mac::text_secondary())
                    .child(status),
            )
            .child(
                div()
                    .id("mail-find-done")
                    .role(Role::Button)
                    .aria_label("Done")
                    .px(px(10.0))
                    .h(px(22.0))
                    .flex()
                    .items_center()
                    .rounded(px(mac::radius_control()))
                    .bg(mac::control_fill())
                    .text_size(px(12.0))
                    .text_color(mac::text())
                    .child("Done")
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| this.close_find(window, cx)),
                    ),
            )
            .into_any_element()
    }

    /// View ▸ Message ▸ All Headers (⇧⌘H): shows or hides every header of
    /// the message above its body.
    pub(super) fn toggle_all_headers(&mut self, cx: &mut Context<Self>) {
        self.all_headers = !self.all_headers;
        self.load_all_headers(cx);
        self.sync_menu(cx);
        cx.notify();
    }

    /// Reads the selected message's header block for All Headers, from its
    /// cached source when Mail has it.
    pub(super) fn load_all_headers(&mut self, cx: &mut Context<Self>) {
        if !self.all_headers {
            return;
        }
        let Some(message) = self.state.selected_message().cloned() else {
            return;
        };
        if self
            .all_headers_text
            .as_ref()
            .is_some_and(|(id, _)| *id == message.id)
        {
            return;
        }
        let raw = self.load_raw_source(cx);
        cx.spawn(async move |this, cx| {
            let source = match raw.await {
                Some(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                None => rmac_mail::message_source(&message),
            };
            let headers = rmac_mail::header_block(&source);
            let _ = this.update(cx, |this, cx| {
                if this.state.selected.as_deref() == Some(message.id.as_str()) {
                    this.all_headers_text = Some((message.id, headers.into()));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The header block All Headers shows for the selected message.
    pub(super) fn all_headers_block(&self) -> Option<AnyElement> {
        if !self.all_headers {
            return None;
        }
        let selected = self.state.selected.as_deref()?;
        let (id, text) = self.all_headers_text.as_ref()?;
        (id == selected).then(|| {
            div()
                .id("mail-all-headers")
                .role(Role::Group)
                .aria_label("All Headers")
                .pb(px(12.0))
                .border_b_1()
                .border_color(mac::separator())
                .font_family(rmac_ui::MONO_FONT)
                .text_size(px(11.0))
                .text_color(mac::text_secondary())
                .child(text.clone())
                .into_any_element()
        })
    }

    /// Message ▸ Flag ▸ `colour`.
    pub(super) fn flag_with(&mut self, colour: FlagColour, cx: &mut Context<Self>) {
        self.perform(rmac_mail::OrganizeAction::FlagColour(colour), cx);
    }

    /// The Flag menu's checkmark and the menu rows only a viewer can use.
    pub(super) fn sync_tools_menu(&self, cx: &mut Context<Self>) {
        use rmac_ui::{set_menu_checked as check, set_menu_enabled as enable};
        let selected = self.state.selected_message();
        let flagged = selected.filter(|message| message.flagged);
        for colour in FlagColour::MENU {
            let action = flag_action(colour);
            check(
                action,
                flagged.is_some_and(|message| message.flag_colour() == colour),
                cx,
            );
            enable(
                action,
                selected.is_some_and(|message| message.mailbox.is_real()),
                cx,
            );
        }
        check("mail::ShowAllHeaders", self.all_headers, cx);
        enable("mail::ShowAllHeaders", selected.is_some(), cx);
        check("mail::ToggleMessageSize", self.show_size, cx);
        enable(
            "mail::QuickLookAttachments",
            selected.is_some_and(|message| {
                message.attachment.is_some() && message.body_state == rmac_mail::BodyState::Loaded
            }),
            cx,
        );
        enable("mail::NewViewerWindow", true, cx);
        let has_matches = !self.find_matches(cx).is_empty();
        if self.active {
            enable("mail::FindInMessage", selected.is_some(), cx);
            enable("mail::FindNext", has_matches, cx);
            enable("mail::FindPrevious", has_matches, cx);
            enable("mail::JumpToSelection", has_matches, cx);
            // Set Priority and the compose-only Edit rows belong to a
            // compose window in front.
            for action in [
                "mail::PriorityLow",
                "mail::PriorityNormal",
                "mail::PriorityHigh",
                "mail::AddLink",
                "mail::RemoveLink",
                "mail::ShowSpellingAndGrammar",
                "mail::CheckDocumentNow",
            ] {
                enable(action, false, cx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_flag_colour_has_its_own_action() {
        let mut actions: Vec<&str> = FlagColour::MENU.into_iter().map(flag_action).collect();
        actions.sort_unstable();
        actions.dedup();
        assert_eq!(actions.len(), 7);
        assert_eq!(flag_action(FlagColour::MENU[0]), "mail::FlagOrange");
    }

    #[test]
    fn staged_attachments_keep_their_name_and_bytes() {
        let path = stage_attachment("Menu.pdf", b"%PDF-1.4").unwrap();
        assert_eq!(path.file_name().unwrap(), "Menu.pdf");
        assert_eq!(std::fs::read(&path).unwrap(), b"%PDF-1.4");
        let _ = std::fs::remove_file(path);
    }
}
