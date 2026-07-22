//! Renderer-neutral notification banner content, semantics, and keyboard flow.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use rmac_notifications::banner::{
    BannerSnapshot, OutputId, PauseState, PhaseSnapshot, Snapshot as StackSnapshot,
};
use rmac_notifications::{Notification, NotificationId, Priority};

pub const MAX_CARDS: usize = 500;
pub const MAX_APPLICATION_NAME_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveRegion {
    Polite,
    Assertive,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ControlId {
    Card(NotificationId),
    Button {
        notification: NotificationId,
        index: usize,
    },
    Dismiss(NotificationId),
}

impl ControlId {
    pub fn notification(self) -> NotificationId {
        match self {
            Self::Card(id) | Self::Dismiss(id) => id,
            Self::Button { notification, .. } => notification,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlRole {
    Notification,
    Action,
    Dismiss,
}

#[derive(Clone, Eq, PartialEq)]
pub struct Control {
    pub id: ControlId,
    pub role: ControlRole,
    pub label: String,
    pub accessible_label: String,
    pub activatable: bool,
}

impl fmt::Debug for Control {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Control")
            .field("id", &self.id)
            .field("role", &self.role)
            .field("label", &"<redacted>")
            .field("accessible_label", &"<redacted>")
            .field("activatable", &self.activatable)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct Card {
    pub id: NotificationId,
    pub output: OutputId,
    pub phase: PhaseSnapshot,
    pub pause: PauseState,
    pub stack_index: usize,
    pub app_name: String,
    pub title: String,
    pub body: String,
    pub priority: Priority,
    pub live_region: LiveRegion,
    pub accessible_label: String,
    pub controls: Vec<Control>,
}

impl fmt::Debug for Card {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Card")
            .field("id", &self.id)
            .field("output", &"<redacted>")
            .field("phase", &self.phase)
            .field("pause", &self.pause)
            .field("stack_index", &self.stack_index)
            .field("app_name", &"<redacted>")
            .field("title", &"<redacted>")
            .field("body", &"<redacted>")
            .field("priority", &self.priority)
            .field("live_region", &self.live_region)
            .field("accessible_label", &"<redacted>")
            .field("controls", &self.controls)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Announcement {
    pub id: NotificationId,
    pub live_region: LiveRegion,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SyncOutcome {
    pub announcements: Vec<Announcement>,
    pub focus_released: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Key {
    Tab,
    BackTab,
    ArrowDown,
    ArrowUp,
    Home,
    End,
    Enter,
    Space,
    Escape,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Intent {
    InvokeDefault(NotificationId),
    InvokeButton {
        notification: NotificationId,
        index: usize,
    },
    Dismiss(NotificationId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Effect {
    None,
    FocusChanged {
        control: ControlId,
        notification: NotificationId,
    },
    Activate {
        control: ControlId,
        intent: Intent,
    },
    ReleaseFocus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    TooManyCards,
    DuplicateBanner,
    MissingNotification,
    InvalidApplicationName,
}

/// Stateful presentation authority. Content remains retained while a banner's
/// terminal animation is present even if the service snapshot has already
/// removed its authoritative notification.
#[derive(Default)]
pub struct Presenter {
    cards: Vec<Card>,
    focused: Option<ControlId>,
    pending: Option<ControlId>,
}

impl Presenter {
    pub fn cards(&self) -> &[Card] {
        &self.cards
    }

    pub fn focused(&self) -> Option<ControlId> {
        self.focused
    }

    pub fn pending(&self) -> Option<ControlId> {
        self.pending
    }

    /// Synchronizes stable stack geometry with current notification content.
    /// `reannounce` contains replacement IDs whose portal `show-as-new` flag
    /// explicitly requested a fresh assistive announcement.
    pub fn sync(
        &mut self,
        stack: &StackSnapshot,
        notifications: &[Notification],
        application_names: &BTreeMap<String, String>,
        reannounce: &[NotificationId],
    ) -> Result<SyncOutcome, Error> {
        if stack.banners.len() > MAX_CARDS {
            return Err(Error::TooManyCards);
        }
        let mut seen = BTreeSet::new();
        if stack.banners.iter().any(|banner| !seen.insert(banner.id)) {
            return Err(Error::DuplicateBanner);
        }
        let notifications = notifications
            .iter()
            .map(|notification| (notification.id, notification))
            .collect::<BTreeMap<_, _>>();
        let previous = self
            .cards
            .iter()
            .cloned()
            .map(|card| (card.id, card))
            .collect::<BTreeMap<_, _>>();
        let previous_ids = previous.keys().copied().collect::<BTreeSet<_>>();
        let mut next = Vec::with_capacity(stack.banners.len());
        for banner in &stack.banners {
            let mut card = if let Some(notification) = notifications.get(&banner.id) {
                card(notification, banner, application_names)?
            } else if matches!(banner.phase, PhaseSnapshot::Exiting(_)) {
                previous
                    .get(&banner.id)
                    .cloned()
                    .ok_or(Error::MissingNotification)?
            } else {
                return Err(Error::MissingNotification);
            };
            apply_banner(&mut card, banner);
            next.push(card);
        }

        let controls = next
            .iter()
            .flat_map(|card| card.controls.iter().map(|control| control.id))
            .collect::<BTreeSet<_>>();
        let focus_released = self.focused.is_some_and(|focus| !controls.contains(&focus));
        if focus_released {
            self.focused = None;
        }
        if self
            .pending
            .is_some_and(|pending| !controls.contains(&pending))
        {
            self.pending = None;
        }
        let requested = reannounce.iter().copied().collect::<BTreeSet<_>>();
        let announcements = next
            .iter()
            .filter(|card| !previous_ids.contains(&card.id) || requested.contains(&card.id))
            .filter(|card| !matches!(card.phase, PhaseSnapshot::Exiting(_)))
            .map(|card| Announcement {
                id: card.id,
                live_region: card.live_region,
            })
            .collect();
        self.cards = next;
        Ok(SyncOutcome {
            announcements,
            focus_released,
        })
    }

    pub fn select(&mut self, control: ControlId) -> Effect {
        if self.pending.is_some() || !self.control_ids().contains(&control) {
            return Effect::None;
        }
        if self.focused == Some(control) {
            return Effect::None;
        }
        self.focused = Some(control);
        Effect::FocusChanged {
            control,
            notification: control.notification(),
        }
    }

    pub fn handle_key(&mut self, key: Key) -> Effect {
        if self.pending.is_some() {
            return Effect::None;
        }
        match key {
            Key::Tab | Key::ArrowDown => self.move_focus(true, false),
            Key::BackTab | Key::ArrowUp => self.move_focus(false, false),
            Key::Home => self.move_focus(true, true),
            Key::End => self.move_focus(false, true),
            Key::Enter | Key::Space => self.activate_focused(),
            Key::Escape => {
                if self.focused.take().is_some() {
                    Effect::ReleaseFocus
                } else {
                    Effect::None
                }
            }
        }
    }

    /// Activates one exact pointer/touch-selected control without changing
    /// keyboard focus. A second operation cannot start while one is pending.
    pub fn activate_control(&mut self, control: ControlId) -> Effect {
        if self.pending.is_some() {
            return Effect::None;
        }
        let Some(intent) = self.intent(control) else {
            return Effect::None;
        };
        self.pending = Some(control);
        Effect::Activate { control, intent }
    }

    /// Clears one exact completed service operation. Mismatched/stale
    /// completions cannot unlock a newer activation.
    pub fn complete(&mut self, control: ControlId) -> bool {
        if self.pending != Some(control) {
            return false;
        }
        self.pending = None;
        true
    }

    fn move_focus(&mut self, forward: bool, edge: bool) -> Effect {
        let controls = self.control_ids();
        if controls.is_empty() {
            return Effect::None;
        }
        let next = if edge {
            if forward {
                controls[0]
            } else {
                controls[controls.len() - 1]
            }
        } else if let Some(focused) = self.focused {
            let index = controls
                .iter()
                .position(|control| *control == focused)
                .unwrap_or(0);
            if forward {
                controls[(index + 1) % controls.len()]
            } else {
                controls[(index + controls.len() - 1) % controls.len()]
            }
        } else if forward {
            controls[0]
        } else {
            controls[controls.len() - 1]
        };
        self.select(next)
    }

    fn activate_focused(&mut self) -> Effect {
        let Some(control) = self.focused else {
            return Effect::None;
        };
        self.activate_control(control)
    }

    fn intent(&self, control: ControlId) -> Option<Intent> {
        let card = self
            .cards
            .iter()
            .find(|card| card.id == control.notification())?;
        let control_state = card
            .controls
            .iter()
            .find(|candidate| candidate.id == control)?;
        if !control_state.activatable {
            return None;
        }
        match control {
            ControlId::Card(id) => Some(Intent::InvokeDefault(id)),
            ControlId::Button {
                notification,
                index,
            } => Some(Intent::InvokeButton {
                notification,
                index,
            }),
            ControlId::Dismiss(id) => Some(Intent::Dismiss(id)),
        }
    }

    fn control_ids(&self) -> Vec<ControlId> {
        self.cards
            .iter()
            .flat_map(|card| card.controls.iter().map(|control| control.id))
            .collect()
    }
}

impl fmt::Debug for Presenter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Presenter")
            .field("cards", &self.cards)
            .field("focused", &self.focused)
            .field("pending", &self.pending)
            .finish()
    }
}

fn card(
    notification: &Notification,
    banner: &BannerSnapshot,
    application_names: &BTreeMap<String, String>,
) -> Result<Card, Error> {
    let app_id = notification.source.app_id().as_str();
    let app_name = application_names
        .get(app_id)
        .map(String::as_str)
        .unwrap_or(app_id);
    if app_name.trim().is_empty()
        || app_name.len() > MAX_APPLICATION_NAME_BYTES
        || app_name.chars().any(char::is_control)
    {
        return Err(Error::InvalidApplicationName);
    }
    let title = notification.content.title().to_owned();
    let body = notification.content.body().to_owned();
    let live_region = if notification.priority == Priority::Urgent {
        LiveRegion::Assertive
    } else {
        LiveRegion::Polite
    };
    let kind = if live_region == LiveRegion::Assertive {
        "Urgent notification"
    } else {
        "Notification"
    };
    let accessible_label = [kind, " from ", app_name, ". ", &title, ". ", &body].concat();
    let card_label = if title.trim().is_empty() {
        app_name.to_owned()
    } else {
        title.clone()
    };
    let mut controls = vec![Control {
        id: ControlId::Card(notification.id),
        role: ControlRole::Notification,
        label: card_label,
        accessible_label: accessible_label.clone(),
        activatable: notification.default_action.is_some(),
    }];
    controls.extend(
        notification
            .actions
            .iter()
            .enumerate()
            .filter_map(|(index, action)| {
                let label = rmac_notifications::protocol::visible_action_label(action)?;
                Some(Control {
                    id: ControlId::Button {
                        notification: notification.id,
                        index,
                    },
                    role: ControlRole::Action,
                    label: label.clone().into_owned(),
                    accessible_label: label.into_owned(),
                    activatable: true,
                })
            }),
    );
    if !notification.display.persistent {
        controls.push(Control {
            id: ControlId::Dismiss(notification.id),
            role: ControlRole::Dismiss,
            label: "Dismiss".into(),
            accessible_label: format!("Dismiss notification from {app_name}"),
            activatable: true,
        });
    }
    Ok(Card {
        id: notification.id,
        output: banner.output.clone(),
        phase: banner.phase,
        pause: banner.pause,
        stack_index: banner.stack_index,
        app_name: app_name.to_owned(),
        title,
        body,
        priority: notification.priority,
        live_region,
        accessible_label,
        controls,
    })
}

fn apply_banner(card: &mut Card, banner: &BannerSnapshot) {
    card.output = banner.output.clone();
    card.phase = banner.phase;
    card.pause = banner.pause;
    card.stack_index = banner.stack_index;
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_notifications::banner::{Config, Motion, Stack};
    use rmac_notifications::protocol::{self, PortalButton, PortalInput};
    use rmac_notifications::{DeliveryPolicy, Server, Time, TimeoutPolicy};

    fn notification(persistent: bool) -> Notification {
        let mut request = protocol::portal(PortalInput {
            app_id: "org.example.Private".into(),
            id: "message-1".into(),
            title: Some("Private title 8472".into()),
            body: Some("Private body 8472".into()),
            priority: Some("urgent".into()),
            default_action: Some("app.open".into()),
            buttons: vec![
                PortalButton {
                    label: Some("Archive Private 8472".into()),
                    action: "app.archive".into(),
                    ..PortalButton::default()
                },
                PortalButton {
                    action: "app.reply".into(),
                    purpose: Some("im.reply-with-text".into()),
                    ..PortalButton::default()
                },
            ],
            ..PortalInput::default()
        })
        .unwrap();
        request.display.persistent = persistent;
        let mut server = Server::new(10, TimeoutPolicy::default());
        let outcome = server
            .post(request, Time(0), DeliveryPolicy::default())
            .unwrap();
        let notification = server
            .active()
            .find(|notification| notification.id == outcome.id)
            .unwrap()
            .clone();
        notification
    }

    fn output() -> OutputId {
        OutputId::parse("private-output-8472").unwrap()
    }

    fn stack(notification: &Notification, motion: Motion) -> Stack {
        let mut stack = Stack::new(Config::default(), motion).unwrap();
        stack.post(notification.id, output(), None, false, Time(0));
        stack
    }

    fn names() -> BTreeMap<String, String> {
        BTreeMap::from([("org.example.Private".into(), "Private Chat 8472".into())])
    }

    #[test]
    fn cards_expose_exact_actions_and_redact_all_content_from_debug() {
        let notification = notification(false);
        let stack = stack(&notification, Motion::Reduced);
        let mut presenter = Presenter::default();
        let outcome = presenter
            .sync(&stack.snapshot(), &[notification], &names(), &[])
            .unwrap();
        assert_eq!(
            outcome.announcements,
            vec![Announcement {
                id: presenter.cards()[0].id,
                live_region: LiveRegion::Assertive,
            }]
        );
        let controls = &presenter.cards()[0].controls;
        assert_eq!(controls.len(), 4);
        assert_eq!(controls[0].id, ControlId::Card(presenter.cards()[0].id));
        assert_eq!(
            controls[1].id,
            ControlId::Button {
                notification: presenter.cards()[0].id,
                index: 0,
            }
        );
        assert_eq!(controls[1].label, "Archive Private 8472");
        assert_eq!(controls[2].label, "Reply");
        assert_eq!(controls[3].id, ControlId::Dismiss(presenter.cards()[0].id));
        let debug = format!("{presenter:?}");
        assert!(!debug.contains("Private title 8472"));
        assert!(!debug.contains("Private body 8472"));
        assert!(!debug.contains("Private Chat 8472"));
        assert!(!debug.contains("Archive Private 8472"));
        assert!(!debug.contains("private-output-8472"));
    }

    #[test]
    fn keyboard_navigation_dispatches_one_exact_busy_control() {
        let notification = notification(false);
        let id = notification.id;
        let stack = stack(&notification, Motion::Reduced);
        let mut presenter = Presenter::default();
        presenter
            .sync(&stack.snapshot(), &[notification], &names(), &[])
            .unwrap();
        assert_eq!(
            presenter.handle_key(Key::Tab),
            Effect::FocusChanged {
                control: ControlId::Card(id),
                notification: id,
            }
        );
        assert_eq!(
            presenter.handle_key(Key::Enter),
            Effect::Activate {
                control: ControlId::Card(id),
                intent: Intent::InvokeDefault(id),
            }
        );
        assert_eq!(presenter.handle_key(Key::Tab), Effect::None);
        assert!(!presenter.complete(ControlId::Dismiss(id)));
        assert!(presenter.complete(ControlId::Card(id)));
        assert_eq!(
            presenter.handle_key(Key::Tab),
            Effect::FocusChanged {
                control: ControlId::Button {
                    notification: id,
                    index: 0,
                },
                notification: id,
            }
        );
        assert_eq!(
            presenter.handle_key(Key::Space),
            Effect::Activate {
                control: ControlId::Button {
                    notification: id,
                    index: 0,
                },
                intent: Intent::InvokeButton {
                    notification: id,
                    index: 0,
                },
            }
        );
        assert!(presenter.complete(ControlId::Button {
            notification: id,
            index: 0,
        }));
        assert_eq!(
            presenter.handle_key(Key::End),
            Effect::FocusChanged {
                control: ControlId::Dismiss(id),
                notification: id,
            }
        );
        assert_eq!(presenter.handle_key(Key::Escape), Effect::ReleaseFocus);
    }

    #[test]
    fn terminal_animation_retains_content_then_releases_focus_exactly_once() {
        let notification = notification(false);
        let id = notification.id;
        let mut stack = stack(&notification, Motion::Full);
        stack.advance(Time(220));
        let mut presenter = Presenter::default();
        presenter
            .sync(&stack.snapshot(), &[notification], &names(), &[])
            .unwrap();
        presenter.select(ControlId::Card(id));

        stack.reconcile_closed(id, Time(300));
        let retained = presenter
            .sync(&stack.snapshot(), &[], &BTreeMap::new(), &[])
            .unwrap();
        assert!(!retained.focus_released);
        assert_eq!(presenter.cards()[0].title, "Private title 8472");
        assert!(matches!(
            presenter.cards()[0].phase,
            PhaseSnapshot::Exiting(_)
        ));

        stack.advance(Time(480));
        let removed = presenter
            .sync(&stack.snapshot(), &[], &BTreeMap::new(), &[])
            .unwrap();
        assert!(removed.focus_released);
        assert!(presenter.cards().is_empty());
        assert_eq!(presenter.focused(), None);
        assert!(
            !presenter
                .sync(&stack.snapshot(), &[], &BTreeMap::new(), &[])
                .unwrap()
                .focus_released
        );
    }

    #[test]
    fn sync_is_atomic_and_reannounces_only_explicit_replacements() {
        let notification = notification(true);
        let id = notification.id;
        let stack = stack(&notification, Motion::Reduced);
        let mut presenter = Presenter::default();
        presenter
            .sync(
                &stack.snapshot(),
                std::slice::from_ref(&notification),
                &names(),
                &[],
            )
            .unwrap();
        assert!(presenter.cards()[0]
            .controls
            .iter()
            .all(|control| control.role != ControlRole::Dismiss));
        assert!(presenter
            .sync(
                &stack.snapshot(),
                std::slice::from_ref(&notification),
                &names(),
                &[],
            )
            .unwrap()
            .announcements
            .is_empty());
        assert_eq!(
            presenter
                .sync(
                    &stack.snapshot(),
                    std::slice::from_ref(&notification),
                    &names(),
                    &[id],
                )
                .unwrap()
                .announcements,
            vec![Announcement {
                id,
                live_region: LiveRegion::Assertive,
            }]
        );

        let mut duplicate = stack.snapshot();
        duplicate.banners.push(duplicate.banners[0].clone());
        assert_eq!(
            presenter.sync(&duplicate, &[notification], &names(), &[]),
            Err(Error::DuplicateBanner)
        );
        assert_eq!(presenter.cards().len(), 1);

        let missing = Presenter::default().sync(&stack.snapshot(), &[], &names(), &[]);
        assert_eq!(missing, Err(Error::MissingNotification));
    }
}
