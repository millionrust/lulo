//! The contacts store and Mail's rules as GPUI globals, loaded and saved off
//! the UI thread. Every viewer observes [`People`]; new mail and Message ▸
//! Apply Rules read [`Rules`].

use std::sync::Arc;

use gpui::{App, Global};
use rmac_mail::rules::{self, Rule};

/// The local contacts (`rmac-contacts`).
pub struct People(pub rmac_contacts::Contacts);

impl Global for People {}

/// Mail ▸ Settings ▸ Rules, in order.
pub struct Rules(pub Arc<Vec<Rule>>);

impl Global for Rules {}

/// Loads contacts and rules in the background once at launch.
pub fn load(cx: &mut App) {
    cx.spawn(async move |cx| {
        let (people, rules) = cx
            .background_executor()
            .spawn(async {
                let people = rmac_contacts::load().unwrap_or_else(|error| {
                    eprintln!("rmac-mail: could not read contacts: {error}");
                    rmac_contacts::Contacts::default()
                });
                let rules = rules::load().unwrap_or_else(|error| {
                    eprintln!("rmac-mail: could not read rules: {error}");
                    Vec::new()
                });
                (people, rules)
            })
            .await;
        cx.update(|cx| {
            cx.set_global(Rules(Arc::new(rules)));
            cx.set_global(People(people));
        });
    })
    .detach();
}

/// The rules as last loaded or saved (none before the load finishes).
pub fn rules(cx: &App) -> Arc<Vec<Rule>> {
    cx.try_global::<Rules>()
        .map(|rules| rules.0.clone())
        .unwrap_or_default()
}

/// Replaces the rules and saves them in the background.
pub fn set_rules(next: Vec<Rule>, cx: &mut App) {
    let saved = next.clone();
    cx.set_global(Rules(Arc::new(next)));
    cx.background_executor()
        .spawn(async move {
            if let Err(error) = rules::save(&saved) {
                eprintln!("rmac-mail: could not save rules: {error}");
            }
        })
        .detach();
}

/// The contacts as last loaded or changed.
pub fn contacts(cx: &App) -> rmac_contacts::Contacts {
    cx.try_global::<People>()
        .map(|people| people.0.clone())
        .unwrap_or_default()
}

/// Changes the contacts with `change`; when it reports a change, every
/// viewer follows and the store is saved in the background.
pub fn update(cx: &mut App, change: impl FnOnce(&mut rmac_contacts::Contacts) -> bool) {
    let mut people = contacts(cx);
    if !change(&mut people) {
        return;
    }
    let saved = people.clone();
    cx.set_global(People(people));
    cx.background_executor()
        .spawn(async move {
            if let Err(error) = rmac_contacts::save(&saved) {
                eprintln!("rmac-mail: could not save contacts: {error}");
            }
        })
        .detach();
}
