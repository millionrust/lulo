//! The banner host: runs the notification service, feeds its events to the
//! authority-bound banner session, opens one banner surface per output that
//! has banners, plays each banner's sound, and carries out the dismiss,
//! expiry and action requests the session emits.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::{AnyWindowHandle, App, AppContext as _, AsyncApp, Context, SharedString, Task};
use rmac_notifications::banner::{Config, PhaseSnapshot, PlacementPolicy, Schedule};
use rmac_notifications::{Notification, NotificationId, Time};
use rmac_notifications_linux::banner::{
    self as session, BannerSession, HostCommand, ServiceRequest, SoundCue, SoundPlaybackError,
    SoundPlayer, Update,
};
use rmac_notifications_linux::service::{RuntimeEvent, ServiceHandle};
use rmac_notifications_runtime::presentation::{ControlId, ControlRole, LiveRegion};

use crate::model::{catalog_entries, ApplicationCatalog, ApplicationIdentity};
use crate::surface;

/// macOS 26 banner geometry and motion, measured on the owner's Mac
/// (design-lab/notification-banners.html).
pub(crate) mod metrics {
    /// The card: 344 wide, 16 under the menu bar and 16 from the screen edge.
    pub const CARD_WIDTH: f32 = 344.0;
    pub const TOP: f32 = 16.0;
    pub const RIGHT: f32 = 16.0;
    /// Space between banners of different apps on screen together (not yet
    /// captured; the Center card gap).
    pub const GAP: f32 = 8.0;
    /// Slide-in from just past the screen edge: ≈ 600 ms, decelerating
    /// (fits ease-out cubic to the captured frames).
    pub const ENTER_MS: u64 = 600;
    /// Slide-out back past the edge, accelerating (gone within ≈ 250 ms).
    pub const EXIT_MS: u64 = 250;
    /// The card starts and ends fully off screen.
    pub const SLIDE: f32 = CARD_WIDTH + RIGHT;
}

/// The banner stack policy. A newer banner from the same application takes
/// the older one's place (measured); banners from different applications
/// stack, newest on top, up to three.
fn config() -> Config {
    Config {
        max_visible_per_output: 3,
        enter_ms: metrics::ENTER_MS,
        exit_ms: metrics::EXIT_MS,
        top_inset_px: metrics::TOP as u16,
        trailing_inset_px: metrics::RIGHT as u16,
        gap_px: metrics::GAP as u16,
        width_px: metrics::CARD_WIDTH as u16,
    }
}

/// Resolved identities kept for recent banners; older ones are pruned.
const MAX_IDENTITIES: usize = 512;

#[derive(Clone)]
struct Identity {
    app_id: String,
    /// The application's stack key: its desktop ID, or an `exe:`/`app:`
    /// fallback when no installed application matches.
    key: String,
    visible: ApplicationIdentity,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Phase {
    Enter,
    Rest,
    Exit,
}

#[derive(Clone, Copy)]
struct Motion {
    phase: Phase,
    since: Instant,
}

/// Everything a banner surface needs to draw one card.
pub(crate) struct CardView {
    pub(crate) id: NotificationId,
    pub(crate) title: SharedString,
    pub(crate) body: SharedString,
    pub(crate) icon: Option<PathBuf>,
    pub(crate) initial: SharedString,
    pub(crate) accessible_label: SharedString,
    pub(crate) assertive: bool,
    pub(crate) offset: f32,
    pub(crate) hovered: bool,
    pub(crate) dismissible: bool,
    /// Alert style: no timeout, so its buttons stay visible.
    pub(crate) persistent: bool,
    pub(crate) actions: Vec<(ControlId, SharedString)>,
    pub(crate) busy: Option<ControlId>,
    pub(crate) options_open: bool,
}

pub(crate) struct BannerHost {
    session: BannerSession,
    service: Option<ServiceHandle>,
    catalog: ApplicationCatalog,
    launch: BTreeMap<String, rmac_apps::LaunchSpec>,
    identities: BTreeMap<NotificationId, Identity>,
    identity_order: VecDeque<NotificationId>,
    compositor: rmac_compositor::State,
    epoch: Instant,
    motions: BTreeMap<NotificationId, Motion>,
    options_open: BTreeSet<NotificationId>,
    wake: Option<Task<()>>,
    sounds: SoundPlayer,
    surfaces: BTreeMap<String, AnyWindowHandle>,
}

/// Keeps the host alive for the whole process.
struct Host(#[allow(dead_code)] gpui::Entity<BannerHost>);

impl gpui::Global for Host {}

pub(crate) fn start(cx: &mut App) {
    let host = cx.new(BannerHost::new);
    cx.set_global(Host(host));
}

impl BannerHost {
    fn new(cx: &mut Context<Self>) -> Self {
        let session = match BannerSession::new(
            config(),
            PlacementPolicy::ActiveOutput,
            &rmac_appearance::Snapshot::default(),
        ) {
            Ok(session) => session,
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        };

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let (service, events) = match rmac_notifications_linux::service::serve().await {
                Ok(served) => served,
                Err(error) => {
                    eprintln!("{error}");
                    std::process::exit(1);
                }
            };
            let handle = service.clone();
            if this
                .update(cx, move |this, _| this.service = Some(handle))
                .is_err()
            {
                return;
            }
            // This is the service's one receiver. Each event first updates
            // Center history off the D-Bus dispatch path, then moves into the
            // banner session, which never blocks this loop: service requests
            // run as their own tasks because their events arrive here.
            while let Ok(event) = events.recv().await {
                let history = service.history().clone();
                let (event, outcome) = blocking::unblock(move || {
                    let outcome = history.record(&event);
                    (event, outcome)
                })
                .await;
                if let Some(outcome) = outcome {
                    if !outcome.persisted {
                        eprintln!("notification service failed (History)");
                    }
                    if let Err(error) = service.emit_indicator(outcome.indicator).await {
                        eprintln!("{error}");
                    }
                }
                if this
                    .update(cx, |this, cx| this.apply_event(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let (compositor_tx, compositor_rx) = async_channel::bounded(64);
        cx.background_executor()
            .spawn(async move {
                if let Err(error) = rmac_compositor_niri::watch(compositor_tx).await {
                    eprintln!("notification banners: compositor watcher stopped: {error}");
                }
            })
            .detach();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            while let Ok(event) = compositor_rx.recv().await {
                if this
                    .update(cx, |this, cx| this.apply_compositor(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let (appearance_tx, appearance_rx) = async_channel::bounded(8);
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_appearance_portal::watch(appearance_tx).await;
            })
            .detach();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            while let Ok(event) = appearance_rx.recv().await {
                let rmac_appearance::Event::Snapshot(appearance) = event else {
                    continue;
                };
                if this
                    .update(cx, |this, cx| {
                        let now = this.now();
                        let result = this.session.apply_appearance(&appearance, now);
                        this.handle(result, cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let loaded = blocking::unblock(|| {
                rmac_apps::discover().map(|applications| {
                    let mut launch = BTreeMap::new();
                    for application in &applications {
                        launch.insert(application.id.clone(), application.launch.clone());
                        if let Some(alias) = application.id.strip_suffix(".desktop") {
                            launch
                                .entry(alias.to_owned())
                                .or_insert_with(|| application.launch.clone());
                        }
                    }
                    (
                        ApplicationCatalog::new(catalog_entries(applications)),
                        launch,
                    )
                })
            })
            .await;
            if let Ok((catalog, launch)) = loaded {
                let _ = this.update(cx, |this, _| {
                    this.catalog = catalog;
                    this.launch = launch;
                });
            }
        })
        .detach();

        Self {
            session,
            service: None,
            catalog: ApplicationCatalog::default(),
            launch: BTreeMap::new(),
            identities: BTreeMap::new(),
            identity_order: VecDeque::new(),
            compositor: rmac_compositor::State::default(),
            epoch: Instant::now(),
            motions: BTreeMap::new(),
            options_open: BTreeSet::new(),
            wake: None,
            sounds: SoundPlayer::new(),
            surfaces: BTreeMap::new(),
        }
    }

    fn now(&self) -> Time {
        Time(u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX))
    }

    fn apply_event(&mut self, event: RuntimeEvent, cx: &mut Context<Self>) {
        let now = self.now();
        let mut replaced = Vec::new();
        if let RuntimeEvent::Posted {
            outcome,
            notification: Some(notification),
            ..
        } = &event
        {
            if outcome.delivery.banner {
                let identity = self.identify(notification);
                // A newer banner from the same application takes the place
                // of an older one still on screen. Alerts stay until acted on.
                replaced = self
                    .session
                    .frame()
                    .cards
                    .iter()
                    .filter(|card| card.id != notification.id)
                    .filter(|card| !matches!(card.phase, PhaseSnapshot::Exiting(_)))
                    .filter(|card| {
                        card.controls
                            .iter()
                            .any(|control| control.role == ControlRole::Dismiss)
                    })
                    .filter(|card| {
                        self.identities
                            .get(&card.id)
                            .is_some_and(|other| other.key == identity.key)
                    })
                    .map(|card| card.id)
                    .collect::<Vec<_>>();
                self.remember(notification.id, identity);
                self.publish_names(now, cx);
            }
        }
        let result = self.session.apply_event(event, now);
        self.handle(result, cx);
        for id in replaced {
            let result = self.session.replace(id, now);
            self.handle(result, cx);
        }
    }

    fn apply_compositor(&mut self, event: rmac_compositor::Event, cx: &mut Context<Self>) {
        let _ = self.compositor.apply(event);
        let snapshot = self.compositor.snapshot();
        let now = self.now();
        let result = self.session.apply_compositor(&snapshot, now);
        self.handle(result, cx);
    }

    /// The same application name and icon Notification Center shows.
    fn identify(&self, notification: &Notification) -> Identity {
        let app_id = notification.source.app_id().as_str().to_owned();
        let origin = self
            .service
            .as_ref()
            .and_then(|service| service.history().origin(notification.id))
            .unwrap_or_default();
        let (key, visible) = self.catalog.resolve_origin(&app_id, &origin);
        Identity {
            app_id,
            key,
            visible,
        }
    }

    fn remember(&mut self, id: NotificationId, identity: Identity) {
        if self.identities.insert(id, identity).is_none() {
            self.identity_order.push_back(id);
        }
        while self.identity_order.len() > MAX_IDENTITIES {
            let Some(oldest) = self.identity_order.pop_front() else {
                break;
            };
            let on_screen = self
                .session
                .frame()
                .cards
                .iter()
                .any(|card| card.id == oldest);
            if on_screen {
                self.identity_order.push_back(oldest);
                break;
            }
            self.identities.remove(&oldest);
        }
    }

    /// Gives the session each sender's visible name, which it uses for the
    /// accessible label and for cards without a title.
    fn publish_names(&mut self, now: Time, cx: &mut Context<Self>) {
        let names = self
            .identities
            .values()
            .filter(|identity| {
                let name = identity.visible.name.as_ref();
                !name.trim().is_empty()
                    && name.len()
                        <= rmac_notifications_runtime::presentation::MAX_APPLICATION_NAME_BYTES
                    && !name.chars().any(char::is_control)
            })
            .map(|identity| (identity.app_id.clone(), identity.visible.name.to_string()))
            .collect::<BTreeMap<_, _>>();
        let result = self.session.set_application_names(names, now);
        self.handle(result, cx);
    }

    fn handle(&mut self, result: Result<Update, session::Error>, cx: &mut Context<Self>) {
        match result {
            Ok(update) => self.run(update, cx),
            Err(error) => eprintln!("{error}"),
        }
    }

    fn run(&mut self, update: Update, cx: &mut Context<Self>) {
        for command in update.commands {
            match command {
                HostCommand::Service(request) => self.execute(request, cx),
                // Banners never take keyboard focus, so there is no previous
                // focus to capture or restore.
                HostCommand::Redraw
                | HostCommand::CapturePreviousFocus
                | HostCommand::RestorePreviousFocus => {}
            }
        }
        for cue in update.sounds {
            self.play(cue, cx);
        }
        self.arm(update.schedule, cx);
        self.sync_motion();
        let cards = self
            .session
            .frame()
            .cards
            .iter()
            .map(|card| card.id)
            .collect::<BTreeSet<_>>();
        self.options_open.retain(|id| cards.contains(id));
        self.sync_surfaces(cx);
        cx.notify();
    }

    fn execute(&mut self, request: ServiceRequest, cx: &mut Context<Self>) {
        let Some(service) = self.service.clone() else {
            return;
        };
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = session::execute_service(&service, request, None).await;
            let _ = this.update(cx, |this, cx| {
                let now = this.now();
                let update = match result {
                    Ok(completion) => this.session.complete_service(completion, now),
                    Err(error) => {
                        eprintln!("{error}");
                        this.session.fail_service(error, now)
                    }
                };
                this.handle(update, cx);
            });
        })
        .detach();
    }

    fn play(&mut self, cue: SoundCue, cx: &mut Context<Self>) {
        let player = self.sounds.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            match player.play(&cue).await {
                // A cue that arrives while another is still playing is
                // simply skipped, as when several banners land at once.
                Ok(()) | Err(SoundPlaybackError::Busy) => {}
                Err(error) => {
                    eprintln!("{error}");
                    let _ = this.update(cx, |this, cx| {
                        let now = this.now();
                        let result = this.session.fail_sound(&cue, error, now);
                        this.handle(result, cx);
                    });
                }
            }
        })
        .detach();
    }

    /// One timer for the next enter/exit boundary or timeout; replacing it
    /// cancels the previous one.
    fn arm(&mut self, schedule: Schedule, cx: &mut Context<Self>) {
        self.wake = schedule.wake_at.map(|at| {
            let delay = Duration::from_millis(at.0.saturating_sub(self.now().0));
            cx.spawn(async move |this, cx: &mut AsyncApp| {
                cx.background_executor().timer(delay).await;
                let _ = this.update(cx, |this, cx| {
                    let now = this.now();
                    let result = this.session.advance(now);
                    this.handle(result, cx);
                });
            })
        });
    }

    /// Records when each card entered its current phase, which drives its
    /// slide.
    fn sync_motion(&mut self) {
        let now = Instant::now();
        let mut next = BTreeMap::new();
        for card in self.session.frame().cards {
            let phase = match card.phase {
                PhaseSnapshot::Entering => Phase::Enter,
                PhaseSnapshot::Visible => Phase::Rest,
                PhaseSnapshot::Exiting(_) => Phase::Exit,
            };
            let motion = match self.motions.get(&card.id) {
                Some(motion) if motion.phase == phase => *motion,
                _ => Motion { phase, since: now },
            };
            next.insert(card.id, motion);
        }
        self.motions = next;
    }

    /// Opens a surface for each output that has banners and closes the rest.
    fn sync_surfaces(&mut self, cx: &mut Context<Self>) {
        let wanted = self
            .session
            .frame()
            .cards
            .iter()
            .map(|card| card.output.as_str().to_owned())
            .collect::<BTreeSet<_>>();
        let stale = self
            .surfaces
            .keys()
            .filter(|output| !wanted.contains(*output))
            .cloned()
            .collect::<Vec<_>>();
        for output in stale {
            if let Some(handle) = self.surfaces.remove(&output) {
                // The surface may be the window whose click ended its last
                // banner, so close it after the current event.
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                });
            }
        }
        let host = cx.entity();
        for output in wanted {
            if self.surfaces.contains_key(&output) {
                continue;
            }
            if let Some(handle) = surface::open(output.clone(), host.clone(), cx) {
                self.surfaces.insert(output, handle);
            }
        }
    }

    /// Horizontal slide of one card, in points to the right of its place.
    fn offset(&self, id: NotificationId) -> f32 {
        let Some(motion) = self.motions.get(&id) else {
            return 0.0;
        };
        let elapsed = motion.since.elapsed().as_secs_f32() * 1_000.0;
        match motion.phase {
            Phase::Rest => 0.0,
            Phase::Enter => {
                let progress = (elapsed / metrics::ENTER_MS as f32).clamp(0.0, 1.0);
                metrics::SLIDE * (1.0 - progress).powi(3)
            }
            Phase::Exit => {
                let progress = (elapsed / metrics::EXIT_MS as f32).clamp(0.0, 1.0);
                metrics::SLIDE * progress.powi(3)
            }
        }
    }

    pub(crate) fn animating(&self) -> bool {
        self.motions
            .values()
            .any(|motion| motion.phase != Phase::Rest)
    }

    /// This output's cards, newest first.
    pub(crate) fn cards(&self, output: &str) -> Vec<CardView> {
        let frame = self.session.frame();
        let busy = self.session.pending_control();
        let mut cards = frame
            .cards
            .iter()
            .filter(|card| card.output.as_str() == output)
            .collect::<Vec<_>>();
        cards.sort_by_key(|card| card.stack_index);
        cards
            .into_iter()
            .map(|card| {
                let identity = self.identities.get(&card.id);
                let name: SharedString = identity
                    .map(|identity| identity.visible.name.clone())
                    .unwrap_or_else(|| card.app_name.clone().into());
                let title: SharedString = if card.title.trim().is_empty() {
                    name.clone()
                } else {
                    card.title.clone().into()
                };
                let initial = name
                    .chars()
                    .next()
                    .map(|character| character.to_uppercase().to_string())
                    .unwrap_or_else(|| "•".into());
                let dismissible = card
                    .controls
                    .iter()
                    .any(|control| control.role == ControlRole::Dismiss);
                CardView {
                    id: card.id,
                    title,
                    body: card.body.clone().into(),
                    icon: identity.and_then(|identity| identity.visible.icon.clone()),
                    initial: initial.into(),
                    accessible_label: card.accessible_label.clone().into(),
                    assertive: card.live_region == LiveRegion::Assertive,
                    offset: self.offset(card.id),
                    hovered: card.pause.hovered,
                    dismissible,
                    persistent: !dismissible,
                    actions: card
                        .controls
                        .iter()
                        .filter(|control| control.role == ControlRole::Action)
                        .map(|control| (control.id, control.label.clone().into()))
                        .collect(),
                    busy: busy.filter(|control| control.notification() == card.id),
                    options_open: self.options_open.contains(&card.id),
                }
            })
            .collect()
    }

    pub(crate) fn set_hovered(
        &mut self,
        id: NotificationId,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        if !hovered {
            self.options_open.remove(&id);
        }
        let now = self.now();
        // A card that already left the stack is inert.
        if let Ok(update) = self.session.set_hovered(id, hovered, now) {
            self.run(update, cx);
        }
    }

    pub(crate) fn activate(&mut self, control: ControlId, cx: &mut Context<Self>) {
        let now = self.now();
        let result = self.session.activate_control(control, now);
        self.handle(result, cx);
    }

    pub(crate) fn toggle_options(&mut self, id: NotificationId, cx: &mut Context<Self>) {
        if !self.options_open.remove(&id) {
            self.options_open.insert(id);
        }
        cx.notify();
    }

    /// A click on the card body opens the sending application, as on the
    /// Mac: through its default action when it offered one, otherwise by
    /// focusing (or launching) the application and clearing the banner.
    pub(crate) fn click(&mut self, id: NotificationId, cx: &mut Context<Self>) {
        let card = self
            .session
            .frame()
            .cards
            .iter()
            .find(|card| card.id == id)
            .map(|card| {
                let default = card
                    .controls
                    .iter()
                    .any(|control| control.id == ControlId::Card(id) && control.activatable);
                let dismissible = card
                    .controls
                    .iter()
                    .any(|control| control.role == ControlRole::Dismiss);
                (default, dismissible)
            });
        let Some((default, dismissible)) = card else {
            return;
        };
        if default {
            self.activate(ControlId::Card(id), cx);
            return;
        }
        self.open_application(id, cx);
        if dismissible {
            self.activate(ControlId::Dismiss(id), cx);
        }
    }

    fn open_application(&self, id: NotificationId, cx: &mut Context<Self>) {
        let Some(identity) = self.identities.get(&id) else {
            return;
        };
        let desktop_id = identity
            .key
            .strip_suffix(".desktop")
            .unwrap_or(&identity.key)
            .to_owned();
        let snapshot = self.compositor.snapshot();
        let window = snapshot
            .windows
            .iter()
            .filter(|window| window.app_id.as_deref() == Some(desktop_id.as_str()))
            .max_by_key(|window| {
                window
                    .focus_timestamp
                    .map(|stamp| (stamp.seconds, stamp.nanoseconds))
            })
            .map(|window| window.id);
        if let Some(window) = window {
            cx.spawn(async move |_, _: &mut AsyncApp| {
                let action = rmac_compositor::Action::FocusWindow { window };
                if let Err(error) = rmac_compositor_niri::execute_action(&action).await {
                    eprintln!("notification banners: could not focus the application: {error:?}");
                }
            })
            .detach();
            return;
        }
        let Some(spec) = self.launch.get(&identity.key).cloned() else {
            return;
        };
        cx.spawn(async move |_, _: &mut AsyncApp| {
            if let Err(error) = rmac_app_launch::launch(spec).await {
                eprintln!("notification banners: could not open the application: {error:?}");
            }
        })
        .detach();
    }
}
