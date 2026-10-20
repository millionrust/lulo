//! Contacts, VIPs and rules in the viewer: Message ▸ Add Sender to
//! Contacts, Add Sender to VIPs / Remove Sender from VIPs, Apply Rules
//! (⌥⌘L), View ▸ Filter ▸ Only from VIP, and Settings ▸ Rules acting on mail
//! as it arrives.

use gpui::{App, Context, InteractiveElement as _};
use rmac_mail::Mailbox;
#[cfg(any(target_os = "linux", windows))]
use rmac_mail::SpecialUse;

use super::MailView;
use crate::{people, AddSenderToContacts, ApplyRules, FilterVip, ToggleSenderVip};

/// The real mailboxes a rule can file into, as (account, mailbox name),
/// from the first viewer (Settings has no mail state of its own).
pub fn rule_destinations(cx: &App) -> Vec<(String, String)> {
    let Some(view) = super::tools::viewers().into_iter().next() else {
        return Vec::new();
    };
    view.read(cx)
        .state
        .mailboxes
        .iter()
        .filter_map(|mailbox| match mailbox {
            Mailbox::Real(real) => Some((real.account_label.clone(), real.name.clone())),
            _ => None,
        })
        .collect()
}

impl MailView {
    /// Takes the contacts store as it loads or changes.
    pub(super) fn observe_people(&mut self, cx: &mut Context<Self>) {
        if let Some(people) = cx.try_global::<people::People>() {
            self.state.set_people(people.0.clone());
        }
        cx.observe_global::<people::People>(|this, cx| {
            let people = cx.global::<people::People>().0.clone();
            this.state.set_people(people);
            this.sync_menu(cx);
            cx.notify();
        })
        .detach();
    }

    /// Settings ▸ Rules on a message that just arrived in an Inbox.
    #[cfg(any(target_os = "linux", windows))]
    pub fn apply_rules_to_arrival(
        &mut self,
        account: uuid::Uuid,
        row_id: i64,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.state.id_for_row(account, row_id) else {
            return;
        };
        let in_inbox = self.state.messages.iter().any(|message| {
            message.id == id && message.mailbox.special_use() == Some(SpecialUse::Inbox)
        });
        if in_inbox {
            self.run_rules(vec![id], cx);
        }
    }

    fn run_rules(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        let rules = people::rules(cx);
        if self.state.apply_rules(&rules, &ids) > 0 {
            self.dispatch_persist(cx);
            self.sync_menu(cx);
            cx.notify();
        }
    }

    fn sender(&self) -> Option<(String, String)> {
        self.state
            .selected_message()
            .map(|message| (message.sender.clone(), message.sender_address.clone()))
            .filter(|(_, address)| address.contains('@'))
    }

    pub(super) fn sync_people_menu(&self, cx: &mut Context<Self>) {
        // Until the store has loaded, a change could not be merged into it.
        let sender = self.sender().filter(|_| cx.has_global::<people::People>());
        let known = sender
            .as_ref()
            .is_some_and(|(_, address)| self.state.people.knows(address));
        let vip = sender
            .as_ref()
            .is_some_and(|(_, address)| self.state.is_vip(address));
        rmac_ui::set_menu_enabled("mail::AddSenderToContacts", sender.is_some() && !known, cx);
        rmac_ui::set_menu_enabled("mail::ToggleSenderVip", sender.is_some(), cx);
        rmac_ui::set_menu_label(
            "mail::ToggleSenderVip",
            if vip {
                "Remove Sender from VIPs"
            } else {
                "Add Sender to VIPs"
            },
            cx,
        );
        rmac_ui::set_menu_enabled(
            "mail::ApplyRules",
            self.state.selected.is_some() && !people::rules(cx).is_empty(),
            cx,
        );
        rmac_ui::set_menu_checked("mail::FilterVip", self.state.filter.vip, cx);
        rmac_ui::set_menu_enabled("mail::FilterVip", self.state.filter.enabled, cx);
    }

    pub(super) fn on_people_actions(&self, root: gpui::Div, cx: &mut Context<Self>) -> gpui::Div {
        root.on_action(cx.listener(|this, _: &AddSenderToContacts, _, cx| {
            if let Some((name, address)) =
                this.sender().filter(|_| cx.has_global::<people::People>())
            {
                people::update(cx, |contacts| contacts.add(&name, &address));
            }
        }))
        .on_action(cx.listener(|this, _: &ToggleSenderVip, _, cx| {
            if let Some((name, address)) =
                this.sender().filter(|_| cx.has_global::<people::People>())
            {
                let vip = !this.state.is_vip(&address);
                people::update(cx, |contacts| contacts.set_vip(&name, &address, vip));
            }
        }))
        .on_action(cx.listener(|this, _: &ApplyRules, _, cx| {
            if let Some(id) = this.state.selected.clone() {
                this.run_rules(vec![id], cx);
            }
        }))
        .on_action(cx.listener(|this, _: &FilterVip, _, cx| {
            this.state.filter.vip = !this.state.filter.vip;
            this.sync_menu(cx);
            cx.notify();
        }))
    }
}
