//! Mail ▸ Quit and Keep Windows (⌥⌘Q): Mail quits, and the next launch
//! opens the same viewer windows on the same mailbox and message, as macOS
//! restores an app's windows. Compose windows need nothing here: their
//! drafts are already saved in Drafts.

use gpui::{App, Entity};
use rmac_mail::MailState;
use serde::{Deserialize, Serialize};

use super::{open_viewer, viewers, MailView};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptViewer {
    /// The mailbox's account (its sidebar group) and name.
    pub account: String,
    pub mailbox: String,
    pub selected: Option<String>,
}

fn path() -> Option<std::path::PathBuf> {
    crate::delivery::schedule_root().map(|root| root.join("kept-windows.json"))
}

/// Writes down every open viewer, then quits.
pub fn quit_and_keep_windows(cx: &mut App) {
    let kept: Vec<KeptViewer> = viewers()
        .iter()
        .map(|view| {
            let state = &view.read(cx).state;
            KeptViewer {
                account: state.mailbox.account().to_owned(),
                mailbox: state.mailbox.label(),
                selected: state.selected.clone(),
            }
        })
        .collect();
    let task = cx.background_executor().spawn(async move {
        if let (Some(path), Ok(bytes)) = (path(), serde_json::to_vec(&kept)) {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(path, bytes);
        }
    });
    cx.spawn(async move |cx| {
        task.await;
        cx.update(|cx| cx.quit());
    })
    .detach();
}

/// The windows the last Quit and Keep Windows kept, read once at launch.
pub fn take_kept_windows() -> Vec<KeptViewer> {
    let Some(path) = path() else {
        return Vec::new();
    };
    let kept = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let _ = std::fs::remove_file(path);
    kept
}

/// Puts `first` (the launch's viewer) back where the first kept window was
/// and opens one more viewer for each other kept window.
pub fn restore(first: &Entity<MailView>, kept: Vec<KeptViewer>, cx: &mut App) {
    let mut kept = kept.into_iter();
    let Some(head) = kept.next() else {
        return;
    };
    first.update(cx, |view, cx| view.show_kept(&head, cx));
    for viewer in kept {
        let (state, accounts, runtime) = {
            let view = first.read(cx);
            let mut state =
                MailState::new(view.state.mailboxes.clone(), view.state.messages.clone());
            state.my_addresses = view.state.my_addresses.clone();
            (state, view.accounts.clone(), view.runtime.clone())
        };
        if let Some(opened) = open_viewer(state, accounts, runtime, cx) {
            opened.update(cx, |view, cx| view.show_kept(&viewer, cx));
        }
    }
}

impl MailView {
    fn show_kept(&mut self, kept: &KeptViewer, cx: &mut gpui::Context<Self>) {
        let mailbox = self
            .state
            .mailboxes
            .iter()
            .find(|mailbox| mailbox.account() == kept.account && mailbox.label() == kept.mailbox)
            .cloned();
        if let Some(mailbox) = mailbox {
            self.state.select_mailbox(mailbox);
        }
        if let Some(id) = &kept.selected {
            if self.state.messages.iter().any(|message| &message.id == id) {
                self.state.selected = Some(id.clone());
            }
        }
        self.after_selection(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::KeptViewer;

    #[test]
    fn kept_windows_round_trip() {
        let kept = vec![KeptViewer {
            account: "Favourites".into(),
            mailbox: "All Inboxes".into(),
            selected: Some("anna".into()),
        }];
        let bytes = serde_json::to_vec(&kept).unwrap();
        let back: Vec<KeptViewer> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back, kept);
    }
}
