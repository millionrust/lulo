//! rmac ⌘Tab application switcher (docs/decisions/0009-app-switcher.md).
//!
//! `app-switcher --service` is the resident session component. niri's
//! ⌘Tab / ⌘⇧Tab binds run `app-switcher next|previous`, which forwards one
//! word to the service and exits. The service opens an exclusive-keyboard
//! overlay, tracks ⌘ through the surface's modifier events, and activates the
//! selected application when ⌘ is released.

#[cfg_attr(not(all(target_os = "linux", feature = "wayland")), allow(dead_code))]
mod force_quit;
#[cfg(all(target_os = "linux", feature = "wayland"))]
mod force_quit_window;
#[cfg(unix)]
mod ipc;
mod model;

#[cfg(all(target_os = "linux", feature = "wayland"))]
pub(crate) mod linux_wayland {
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use gpui::{
        div, layer_shell::*, linear_color_stop, linear_gradient, point, prelude::*, px, rgba,
        AnyElement, App, AsyncApp, Bounds, Context, Entity, FocusHandle, FontWeight, KeyDownEvent,
        ModifiersChangedEvent, QuitMode, Role, Size, WeakEntity, Window,
        WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions,
    };
    use gpui_platform::application;
    use rmac_compositor::Action;
    use rmac_shell_ui::tokens;
    use uuid::Uuid;

    use crate::force_quit_window::{self, ForceQuitView};
    use crate::model::{self, Command, Layout, Recency, RunningApp, Session};

    /// A quick ⌘Tab tap switches without flashing the panel, as on macOS:
    /// the surface takes the keyboard at once but stays 1 × 1 until it knows
    /// ⌘ is still held (the compositor's modifier state, which follows the
    /// keyboard enter at once) or, failing that, for this long.
    const REVEAL_DELAY: Duration = Duration::from_millis(120);
    /// After the surface gains focus, wait for the compositor's modifier
    /// state before treating "⌘ is up" as a release. niri sends it in the
    /// same flush as the keyboard enter, so it has always been dispatched
    /// well within one frame; the grace only covers a slow event loop. A
    /// quick ⌘Tab tap pays it once (SPEED-11: it was 80 ms, then 16).
    const RELEASE_GRACE: Duration = Duration::from_millis(4);
    /// Never keep an invisible exclusive surface that never got the keyboard.
    /// The surface took 750 ms to get the keyboard in the reference laptop's
    /// nested journey session and sometimes over 1 s, when ⌘Tab then did
    /// nothing at all. Leave a slow low-spec first frame room to arrive.
    const ACTIVATION_TIMEOUT: Duration = Duration::from_millis(3_000);
    /// How long after a close the desktop-entry catalog is re-read.
    const CATALOG_REFRESH_DELAY: Duration = Duration::from_millis(1_000);
    /// How long after hiding the kept surface asks for the configure its
    /// next map needs (after its resize back to 1 × 1 has been sent).
    const KEPT_CONFIGURE_DELAY: Duration = Duration::from_millis(50);
    const QUIT_READBACK_TIMEOUT: Duration = Duration::from_millis(500);
    const QUIT_READBACK_INTERVAL: Duration = Duration::from_millis(25);
    const HIDDEN_SIZE: f32 = 1.0;
    const NAMESPACE: &str = "rmac-app-switcher";

    // Measured on macOS 26 dark mode (design-lab/switcher-osd.html): the
    // panel reads #1A1C21 over #0E1015, i.e. 34,36,41 at 60 % over the blur.
    const DARK_TINT: u32 = 0x2224_2999;
    const DARK_RIM: u32 = 0xFFFF_FF2E;
    const DARK_PLATE_TOP: u32 = 0xFFFF_FF4A;
    const DARK_PLATE_BOTTOM: u32 = 0xFFFF_FF52;
    const DARK_TEXT: u32 = 0xFFFF_FFFF;
    const LIGHT_TINT: u32 = 0xF2F2_F5B3;
    const LIGHT_RIM: u32 = 0xFFFF_FF99;
    const LIGHT_PLATE_TOP: u32 = 0x0000_001A;
    const LIGHT_PLATE_BOTTOM: u32 = 0x0000_0021;
    const LIGHT_TEXT: u32 = 0x0000_00E6;
    const LABEL_SIZE: f32 = 13.0;

    /// What the switcher draws for one application.
    #[derive(Clone)]
    struct Item {
        name: String,
        icon: Option<PathBuf>,
    }

    pub(crate) struct Service {
        compositor: rmac_compositor::State,
        recency: Recency,
        catalog: Rc<Vec<rmac_apps::Application>>,
        open: Option<WindowHandle<SwitcherView>>,
        /// The last switcher surface, unmapped, and the display it is on:
        /// the next ⌘Tab maps it again instead of creating a surface and a
        /// swapchain (SPEED-11).
        kept: Option<(WindowHandle<SwitcherView>, Option<gpui::DisplayId>)>,
        force_quit: Option<WindowHandle<ForceQuitView>>,
    }

    impl Service {
        fn new(cx: &mut Context<Self>) -> Self {
            let mut service = Self {
                compositor: rmac_compositor::State::default(),
                recency: Recency::default(),
                catalog: Rc::new(Vec::new()),
                open: None,
                kept: None,
                force_quit: None,
            };
            service.refresh_catalog(cx);
            service
        }

        fn apply(&mut self, event: rmac_compositor::Event) {
            self.compositor.apply(event);
            let focused_app = self
                .compositor
                .focus
                .window
                .and_then(|window| self.compositor.windows.get(&window))
                .and_then(|window| window.app_id.clone());
            if let Some(app_id) = focused_app {
                self.recency.observe_focus(&app_id);
            }
        }

        /// Names and icons come from the desktop-entry catalog. It is read
        /// off the main thread at start and again after each use, so a newly
        /// installed application is named correctly on the next ⌘Tab.
        fn refresh_catalog(&mut self, cx: &mut Context<Self>) {
            cx.spawn(async move |this, cx: &mut AsyncApp| {
                // `rmac_apps::discover()` can shell out to `gsettings` to
                // read the active icon theme; GPUI's background executor
                // is not safe to spawn child processes from (LINUX-HW-07).
                let catalog = blocking::unblock(rmac_apps::discover).await;
                match catalog {
                    Ok(catalog) => {
                        let _ = this.update(cx, |service, _| service.catalog = Rc::new(catalog));
                    }
                    Err(error) => eprintln!("app switcher could not read applications: {error}"),
                }
            })
            .detach();
        }

        fn closed(&mut self, cx: &mut Context<Self>) {
            self.open = None;
            // Re-read the catalog once the switch has landed: right after a
            // commit it competed with the chosen app's first frame.
            cx.spawn(async move |this, cx: &mut AsyncApp| {
                cx.background_executor().timer(CATALOG_REFRESH_DELAY).await;
                let _ = this.update(cx, |service, cx| service.refresh_catalog(cx));
            })
            .detach();
        }

        fn item(&self, app: &RunningApp) -> Item {
            self.item_for(&app.app_id)
        }

        fn item_for(&self, app_id: &str) -> Item {
            let entry = rmac_apps::find_desktop_entry(&self.catalog, app_id);
            let name = entry
                .map(|entry| entry.name.clone())
                .or_else(|| rmac_apps::identity::window_title(app_id).map(str::to_owned))
                .unwrap_or_else(|| fallback_name(app_id));
            let icon = entry
                .and_then(|entry| entry.icon.clone())
                .filter(|path| path.is_file())
                .or_else(|| packaged_icon(app_id));
            Item { name, icon }
        }

        /// Force Quit's list and icons, from the live window set.
        fn force_quit_entries(&self) -> (Vec<crate::force_quit::Entry>, BTreeMap<String, PathBuf>) {
            let snapshot = self.compositor.snapshot();
            let entries = crate::force_quit::entries(
                &snapshot,
                |app_id| self.item_for(app_id).name,
                crate::force_quit::process_stopped,
            );
            let icons = entries
                .iter()
                .filter_map(|entry| {
                    self.item_for(&entry.app_id)
                        .icon
                        .map(|icon| (entry.app_id.clone(), icon))
                })
                .collect();
            (entries, icons)
        }

        /// Keep an open Force Quit list in step with the windows.
        fn refresh_force_quit(&mut self, cx: &mut Context<Self>) {
            let Some(handle) = self.force_quit else {
                return;
            };
            let (entries, icons) = self.force_quit_entries();
            if handle
                .update(cx, |view, _, cx| view.refresh(entries, icons, cx))
                .is_err()
            {
                self.force_quit = None;
            }
        }

        pub(crate) fn force_quit_closed(&mut self, cx: &mut Context<Self>) {
            self.force_quit = None;
            self.refresh_catalog(cx);
        }

        fn focused_output(&self) -> Option<Uuid> {
            let snapshot = &self.compositor;
            snapshot
                .focus
                .workspace
                .and_then(|id| snapshot.workspaces.get(&id))
                .or_else(|| {
                    snapshot
                        .workspaces
                        .values()
                        .find(|workspace| workspace.focused)
                })
                .and_then(|workspace| workspace.output.as_ref())
                .map(rmac_shell_layer::stable_output_uuid)
        }
    }

    /// "org.mozilla.firefox" → "Firefox" for apps without a desktop entry.
    fn fallback_name(app_id: &str) -> String {
        let base = app_id
            .trim_end_matches(".desktop")
            .rsplit('.')
            .next()
            .unwrap_or(app_id);
        let mut characters = base.chars();
        match characters.next() {
            Some(first) => first.to_uppercase().chain(characters).collect(),
            None => app_id.to_owned(),
        }
    }

    fn packaged_icon(app_id: &str) -> Option<PathBuf> {
        let file = format!("{}.svg", app_id.trim_end_matches(".desktop"));
        let installed = PathBuf::from("/usr/share/icons/hicolor/scalable/apps").join(&file);
        // The source-tree fallback exists only in debug builds, so a release
        // binary never embeds or probes the build checkout's path (SR-15).
        #[cfg(debug_assertions)]
        let candidates = [
            installed,
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../packaging/rmac-apps/icons")
                .join(&file),
        ];
        #[cfg(not(debug_assertions))]
        let candidates = [installed];
        candidates.into_iter().find(|path| path.is_file())
    }

    struct SwitcherView {
        service: WeakEntity<Service>,
        session: Session,
        items: BTreeMap<String, Item>,
        layout: Layout,
        display_width: f32,
        focus: FocusHandle,
        revealed: bool,
        /// The grace period after focus has passed: ⌘ up now means release.
        armed: bool,
        /// A modifier event has shown ⌘ held since the surface got focus.
        saw_command: bool,
        was_active: bool,
        closing: bool,
        pending_quit: bool,
        display_id: Option<gpui::DisplayId>,
        /// Counts opens of this (kept) surface; timers of an earlier open
        /// do nothing.
        generation: u64,
    }

    impl SwitcherView {
        fn new(
            service: WeakEntity<Service>,
            session: Session,
            items: BTreeMap<String, Item>,
            display_width: f32,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) -> Self {
            let focus = cx.focus_handle();
            focus.focus(window, cx);
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    if !this.was_active {
                        this.was_active = true;
                        if window.modifiers().platform {
                            // ⌘ is still held: show the panel now.
                            this.saw_command = true;
                            this.reveal(window, cx);
                        }
                        this.arm_after_grace(window, cx);
                    }
                } else if this.was_active {
                    // Another surface took the keyboard: cancel, like Esc.
                    this.close(window, cx);
                }
            })
            .detach();
            let layout = model::layout(session.apps.len(), display_width);
            let mut view = Self {
                service,
                session,
                items,
                layout,
                display_width,
                focus,
                revealed: false,
                armed: false,
                saw_command: false,
                was_active: false,
                closing: false,
                pending_quit: false,
                display_id: window.display(cx).map(|display| display.id()),
                generation: 0,
            };
            view.start(window, cx);
            view
        }

        /// The timers of one open: reveal after REVEAL_DELAY, and give up
        /// if the surface never gets the keyboard.
        fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
            self.generation = self.generation.wrapping_add(1);
            let generation = self.generation;
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(REVEAL_DELAY).await;
                let _ = this.update_in(cx, |this, window, cx| {
                    if this.generation == generation {
                        this.reveal(window, cx);
                    }
                });
            })
            .detach();
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(ACTIVATION_TIMEOUT).await;
                let _ = this.update_in(cx, |this, window, cx| {
                    if this.generation == generation && !this.was_active {
                        this.close(window, cx);
                    }
                });
            })
            .detach();
        }

        /// Open the kept surface again for a new ⌘Tab.
        fn reopen(
            &mut self,
            session: Session,
            items: BTreeMap<String, Item>,
            display_width: f32,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) {
            self.layout = model::layout(session.apps.len(), display_width);
            self.session = session;
            self.items = items;
            self.display_width = display_width;
            self.revealed = false;
            self.armed = false;
            self.saw_command = false;
            self.was_active = false;
            self.closing = false;
            self.pending_quit = false;
            self.focus.focus(window, cx);
            self.start(window, cx);
            cx.notify();
        }

        fn arm_after_grace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
            let generation = self.generation;
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(RELEASE_GRACE).await;
                let _ = this.update_in(cx, |this, window, cx| {
                    if this.generation != generation || this.closing {
                        return;
                    }
                    this.armed = true;
                    if !window.modifiers().platform {
                        // ⌘ was already up when the surface got the keyboard:
                        // a quick ⌘Tab tap switches straight away.
                        this.commit(window, cx);
                    }
                });
            })
            .detach();
        }

        fn reveal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
            if self.closing || self.revealed {
                return;
            }
            self.revealed = true;
            window.resize(Size::new(px(self.layout.width), px(self.layout.height)));
            cx.notify();
        }

        fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
            self.session.step(forward);
            cx.notify();
        }

        fn modifiers_changed(
            &mut self,
            event: &ModifiersChangedEvent,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) {
            if event.modifiers.platform {
                self.saw_command = true;
                // ⌘ is held after ⌘Tab: this is not a quick tap, so show
                // the panel at once rather than after REVEAL_DELAY.
                self.reveal(window, cx);
            } else if self.armed || self.saw_command {
                self.commit(window, cx);
            }
        }

        fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
            let modifiers = event.keystroke.modifiers;
            match event.keystroke.key.as_str() {
                // niri normally consumes ⌘Tab for its bind and sends `next`;
                // these cover a compositor that forwards the key instead.
                "tab" => self.step(!modifiers.shift, cx),
                "`" if modifiers.platform => self.step(false, cx),
                "left" => self.step(false, cx),
                "right" => self.step(true, cx),
                "escape" => self.close(window, cx),
                "." if modifiers.platform => self.close(window, cx),
                "enter" => self.commit(window, cx),
                "q" if modifiers.platform => self.quit_selected(window, cx),
                "h" if modifiers.platform => self.hide_selected(cx),
                _ => {}
            }
            cx.stop_propagation();
        }

        fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
            if self.closing {
                return;
            }
            self.closing = true;
            let _ = self.service.update(cx, |service, cx| service.closed(cx));
            // Keep the surface, unmapped, for the next ⌘Tab: it takes no
            // input or focus and draws nothing until mapped again.
            // Its swapchain was allocated at the panel's size when it opened
            // (`reserve_window_drawable`), so shrinking a revealed panel back
            // to 1 × 1 for the next ⌘Tab rebuilds nothing.
            let handle = window.window_handle();
            if !gpui_linux::set_layer_window_mapped(handle, false) {
                window.remove_window();
                return;
            }
            gpui_linux::trace_mark("switcher_hidden");
            if self.revealed {
                window.resize(Size::new(px(HIDDEN_SIZE), px(HIDDEN_SIZE)));
            }
            let kept = handle.downcast::<Self>();
            let display_id = self.display_id;
            let replaced = self
                .service
                .update(cx, |service, _| {
                    std::mem::replace(&mut service.kept, kept.map(|kept| (kept, display_id)))
                })
                .ok()
                .flatten();
            if let Some((old, _)) = replaced {
                // Only one surface is kept (a display change made a new one).
                let _ = old.update(cx, |_, window, _| window.remove_window());
            }
            let generation = self.generation;
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(KEPT_CONFIGURE_DELAY).await;
                let _ = this.update_in(cx, |this, window, _| {
                    if this.generation == generation && this.closing {
                        // Ask now for the configure the next map needs.
                        gpui_linux::request_layer_window_configure(window.window_handle());
                    }
                });
            })
            .detach();
        }

        fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
            if self.closing {
                return;
            }
            gpui_linux::trace_mark("switcher_commit");
            let selected = self.session.selected_app().cloned();
            let snapshot = self
                .service
                .upgrade()
                .map(|service| service.read(cx).compositor.snapshot());
            self.close(window, cx);
            if let (Some(app), Some(snapshot)) = (selected, snapshot) {
                activate(app, snapshot, cx);
            }
        }

        fn snapshot(&self, cx: &App) -> Option<rmac_compositor::Snapshot> {
            self.service
                .upgrade()
                .map(|service| service.read(cx).compositor.snapshot())
        }

        /// ⌘Q while the switcher is open quits the selected application and
        /// keeps the switcher up for the rest.
        fn quit_selected(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
            if self.pending_quit {
                return;
            }
            let Some(app) = self.session.selected_app().cloned() else {
                return;
            };
            let Some(service) = self.service.upgrade() else {
                return;
            };
            let Some(handle) = service.read(cx).open else {
                return;
            };
            let Some(snapshot) = self.snapshot(cx) else {
                return;
            };
            let windows = rmac_compositor::windows_of_application(&snapshot, &app.app_id);
            self.pending_quit = true;
            cx.spawn(async move |_, cx: &mut AsyncApp| {
                for window in windows.iter().copied() {
                    let action = Action::CloseWindow { window };
                    if let Err(error) = rmac_compositor_niri::execute_action(&action).await {
                        eprintln!("app switcher could not quit a window: {error:?}");
                    }
                }

                // A close request can be refused by an app's own guard (for
                // example, Text Editor's unsaved-document sheet). Reconcile
                // only after niri reports which windows actually survived.
                let deadline = Instant::now() + QUIT_READBACK_TIMEOUT;
                let mut latest_snapshot = None;
                loop {
                    match rmac_compositor_niri::snapshot().await {
                        Ok(snapshot) => {
                            let closed = windows.iter().all(|target| {
                                !snapshot.windows.iter().any(|window| window.id == *target)
                            });
                            latest_snapshot = Some(snapshot);
                            if closed || Instant::now() >= deadline {
                                break;
                            }
                        }
                        Err(error) => {
                            eprintln!("app switcher could not read windows after quit: {error}");
                            break;
                        }
                    }
                    cx.background_executor().timer(QUIT_READBACK_INTERVAL).await;
                }
                let Some(snapshot) = latest_snapshot else {
                    let _ = handle.update(cx, |view, _, cx| {
                        view.pending_quit = false;
                        cx.notify();
                    });
                    return;
                };
                let remaining = rmac_compositor::windows_of_application(&snapshot, &app.app_id);
                let remaining_ids = remaining
                    .iter()
                    .copied()
                    .collect::<std::collections::BTreeSet<_>>();
                let mut store = rmac_compositor::ParkingStore::load_default();
                for window in &windows {
                    if !remaining_ids.contains(window) {
                        store.forget(*window);
                    }
                }
                if let Err(error) = store.save_default() {
                    eprintln!("app switcher could not save the parking set: {error}");
                }

                let readback = snapshot.clone();
                let (apps, items) = service.update(cx, |service, _| {
                    if remaining.is_empty() {
                        service.recency.forget(&app.app_id);
                    }
                    let apps = service.recency.applications(&readback);
                    let items = apps
                        .iter()
                        .map(|app| (app.app_id.clone(), service.item(app)))
                        .collect();
                    (apps, items)
                });
                let _ = handle.update(cx, |view, window, cx| {
                    view.pending_quit = false;
                    if !view.session.replace_apps(apps) {
                        view.close(window, cx);
                        return;
                    }
                    view.items = items;
                    view.layout = model::layout(view.session.apps.len(), view.display_width);
                    if view.revealed {
                        window.resize(Size::new(px(view.layout.width), px(view.layout.height)));
                    }
                    cx.notify();
                });
            })
            .detach();
        }

        /// ⌘H hides the selected application (parks its windows) and keeps
        /// it listed, as macOS does.
        fn hide_selected(&mut self, cx: &mut Context<Self>) {
            let Some(app) = self.session.selected_app().cloned() else {
                return;
            };
            let Some(snapshot) = self.snapshot(cx) else {
                return;
            };
            let windows = rmac_compositor::application_windows(&snapshot, &app.app_id);
            cx.spawn(async move |_, _cx: &mut AsyncApp| {
                let mut store = rmac_compositor::ParkingStore::load_default();
                store.prune(&snapshot);
                store.record_from(&snapshot, &windows);
                for window in windows {
                    let action = Action::MinimizeWindow { window };
                    if let Err(error) = rmac_compositor_niri::execute_action(&action).await {
                        eprintln!("app switcher could not hide a window: {error:?}");
                    }
                }
                if let Err(error) = store.save_default() {
                    eprintln!("app switcher could not save the parking set: {error}");
                }
            })
            .detach();
            self.session.mark_hidden(&app.app_id);
            cx.notify();
        }

        fn tile(
            &self,
            index: usize,
            app: &RunningApp,
            colors: &Colors,
            scale_factor: f32,
            cx: &mut Context<Self>,
        ) -> AnyElement {
            let layout = self.layout;
            let item = self.items.get(&app.app_id);
            let selected = index == self.session.selected;
            let plate = layout.icon - 2.0 * layout.plate_inset;
            div()
                .id(("switcher-app", index))
                .relative()
                .flex_none()
                .size(px(layout.icon))
                .on_mouse_move(cx.listener(move |this, _, _, cx| {
                    if this.session.selected != index {
                        this.session.select(index);
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.session.select(index);
                    this.commit(window, cx);
                }))
                .when(selected, |tile| {
                    tile.child(
                        div()
                            .absolute()
                            .top(px(layout.plate_inset))
                            .left(px(layout.plate_inset))
                            .size(px(plate))
                            .rounded(px(layout.plate_radius))
                            .bg(linear_gradient(
                                180.0,
                                linear_color_stop(rgba(colors.plate_top), 0.0),
                                linear_color_stop(rgba(colors.plate_bottom), 1.0),
                            )),
                    )
                })
                .child(icon(item, &app.app_id, layout.icon, scale_factor, cx))
                .when(selected, |tile| {
                    tile.child(
                        div()
                            .absolute()
                            .top(px(layout.label_top - layout.padding))
                            .left(px(-layout.icon / 2.0))
                            .w(px(layout.icon * 2.0))
                            .h(px(model::LABEL_LINE))
                            .flex()
                            .justify_center()
                            .whitespace_nowrap()
                            .font_family("Inter")
                            .text_size(px(LABEL_SIZE))
                            .line_height(px(model::LABEL_LINE))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgba(colors.text))
                            .child(
                                item.map(|item| item.name.clone())
                                    .unwrap_or_else(|| fallback_name(&app.app_id)),
                            ),
                    )
                })
                .into_any_element()
        }
    }

    struct Colors {
        tint: u32,
        rim: u32,
        plate_top: u32,
        plate_bottom: u32,
        text: u32,
    }

    impl Colors {
        fn current() -> Self {
            if tokens::is_dark() {
                Self {
                    tint: DARK_TINT,
                    rim: DARK_RIM,
                    plate_top: DARK_PLATE_TOP,
                    plate_bottom: DARK_PLATE_BOTTOM,
                    text: DARK_TEXT,
                }
            } else {
                Self {
                    tint: LIGHT_TINT,
                    rim: LIGHT_RIM,
                    plate_top: LIGHT_PLATE_TOP,
                    plate_bottom: LIGHT_PLATE_BOTTOM,
                    text: LIGHT_TEXT,
                }
            }
        }
    }

    /// The application's artwork fills the 128 frame; macOS-grid artwork
    /// draws its squircle at 104 inside it. Without artwork, a plain plate
    /// carries the name's initial.
    fn icon(
        item: Option<&Item>,
        app_id: &str,
        size: f32,
        scale_factor: f32,
        cx: &mut Context<SwitcherView>,
    ) -> AnyElement {
        if let Some(path) = item.and_then(|item| item.icon.clone()) {
            return rmac_shell_ui::svg_icon(path, size, scale_factor, cx)
                .absolute()
                .top_0()
                .left_0()
                .size(px(size))
                .into_any_element();
        }
        let plate = size * 104.0 / 128.0;
        let inset = (size - plate) / 2.0;
        let initial = item
            .map(|item| item.name.clone())
            .unwrap_or_else(|| fallback_name(app_id))
            .chars()
            .next()
            .map(String::from)
            .unwrap_or_default();
        div()
            .absolute()
            .top(px(inset))
            .left(px(inset))
            .size(px(plate))
            .rounded(px(plate * 0.225))
            .bg(rgba(0x8E8E_93FF))
            .flex()
            .items_center()
            .justify_center()
            .font_family("Inter")
            .text_size(px(plate * 0.5))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgba(0xFFFF_FFFF))
            .child(initial)
            .into_any_element()
    }

    impl Render for SwitcherView {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let layout = self.layout;
            let colors = Colors::current();
            let scale_factor = window.scale_factor();
            let selected_name = self
                .session
                .selected_app()
                .map(|app| {
                    self.items
                        .get(&app.app_id)
                        .map(|item| item.name.clone())
                        .unwrap_or_else(|| fallback_name(&app.app_id))
                })
                .unwrap_or_default();
            let tiles = self
                .session
                .apps
                .iter()
                .enumerate()
                .map(|(index, app)| self.tile(index, app, &colors, scale_factor, cx))
                .collect::<Vec<_>>();
            div()
                .id("app-switcher")
                .track_focus(&self.focus)
                .relative()
                .size_full()
                .role(Role::Status)
                .aria_label(selected_name)
                .on_key_down(cx.listener(Self::key_down))
                .on_modifiers_changed(cx.listener(Self::modifiers_changed))
                .when(self.revealed, |panel| {
                    panel
                        .rounded(px(layout.radius))
                        .bg(rgba(colors.tint))
                        .border(px(1.0))
                        .border_color(rgba(colors.rim))
                        .child(
                            div()
                                .absolute()
                                .left(px(layout.padding))
                                .top(px(layout.padding))
                                .flex()
                                .gap(px(layout.gap))
                                .children(tiles),
                        )
                })
        }
    }

    /// Bring the chosen application forward (see
    /// [`model::activation_actions`]). Restored windows leave the parking set.
    fn activate(app: RunningApp, snapshot: rmac_compositor::Snapshot, cx: &mut App) {
        cx.spawn(async move |_cx: &mut AsyncApp| {
            let mut store = rmac_compositor::ParkingStore::load_default();
            store.prune(&snapshot);
            let actions = model::activation_actions(&snapshot, &app, |window| store.origin(window));
            let mut restored = Vec::new();
            for action in &actions {
                if let Action::RestoreWindow { window, .. } = action {
                    restored.push(*window);
                }
                if let Err(error) = rmac_compositor_niri::execute_action(action).await {
                    eprintln!("app switcher could not activate {}: {error:?}", app.app_id);
                }
            }
            gpui_linux::trace_mark("switcher_activated");
            if !restored.is_empty() {
                for window in restored {
                    store.forget(window);
                }
                if let Err(error) = store.save_default() {
                    eprintln!("app switcher could not save the parking set: {error}");
                }
            }
        })
        .detach();
    }

    fn handle_command(service: &Entity<Service>, command: Command, cx: &mut App) {
        if command == Command::ForceQuit {
            open_force_quit(service, cx);
            return;
        }
        let open = service.read(cx).open;
        if let Some(handle) = open {
            let handled = handle
                .update(cx, |view, window, cx| match command {
                    Command::Next => view.step(true, cx),
                    Command::Previous => view.step(false, cx),
                    Command::Cancel | Command::ForceQuit => view.close(window, cx),
                })
                .is_ok();
            if handled {
                return;
            }
            service.update(cx, |service, _| service.open = None);
        }
        match command {
            Command::Next => open_switcher(service, false, cx),
            Command::Previous => open_switcher(service, true, cx),
            Command::Cancel | Command::ForceQuit => {}
        }
    }

    /// ⌥⌘⎋ and the logo menu's Force Quit…: bring the open window forward,
    /// or open it on the app that was frontmost.
    fn open_force_quit(service: &Entity<Service>, cx: &mut App) {
        let (existing, snapshot) = {
            let state = service.read(cx);
            (state.force_quit, state.compositor.snapshot())
        };
        if let Some(handle) = existing {
            if handle.update(cx, |_, _, _| ()).is_ok() {
                let window = snapshot
                    .windows
                    .iter()
                    .find(|window| window.app_id.as_deref() == Some(crate::force_quit::APP_ID))
                    .map(|window| window.id);
                if let Some(window) = window {
                    cx.spawn(async move |_cx: &mut AsyncApp| {
                        let action = Action::FocusWindow { window };
                        if let Err(error) = rmac_compositor_niri::execute_action(&action).await {
                            eprintln!("could not bring Force Quit forward: {error:?}");
                        }
                    })
                    .detach();
                }
                return;
            }
            service.update(cx, |service, _| service.force_quit = None);
        }
        let (entries, icons) = service.read(cx).force_quit_entries();
        let frontmost = crate::force_quit::frontmost_app(&snapshot);
        let list = crate::force_quit::List::new(entries, frontmost.as_deref());
        let handle = force_quit_window::open(service, list, icons, cx);
        service.update(cx, |service, _| service.force_quit = handle);
    }

    fn open_switcher(service: &Entity<Service>, backwards: bool, cx: &mut App) {
        let (session, items, output) = {
            let state = service.read(cx);
            let snapshot = state.compositor.snapshot();
            let Some(session) = Session::open(state.recency.applications(&snapshot), backwards)
            else {
                return;
            };
            let items = session
                .apps
                .iter()
                .map(|app| (app.app_id.clone(), state.item(app)))
                .collect::<BTreeMap<_, _>>();
            (session, items, state.focused_output())
        };
        let displays = rmac_shell_layer::output_surfaces::newest_displays(cx);
        let display = output
            .and_then(|uuid| displays.get(&uuid).cloned())
            .or_else(|| displays.values().next().cloned());
        let display_width = display
            .as_ref()
            .map_or(1280.0, |display| f32::from(display.bounds().size.width));
        let display_id = display.as_ref().map(|display| display.id());
        let kept = service.update(cx, |service, _| service.kept.take());
        if let Some((handle, kept_display)) = kept {
            if kept_display == display_id
                && gpui_linux::set_layer_window_mapped(handle.into(), true)
            {
                gpui_linux::trace_mark("switcher_reshown");
                let reopened = handle.update(cx, |view, window, cx| {
                    view.reopen(session.clone(), items.clone(), display_width, window, cx)
                });
                if reopened.is_ok() {
                    service.update(cx, |service, _| service.open = Some(handle));
                    return;
                }
            }
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
        let weak = service.downgrade();
        let options = WindowOptions {
            titlebar: None,
            focus: true,
            show: true,
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(0.0), px(0.0)),
                size: Size::new(px(HIDDEN_SIZE), px(HIDDEN_SIZE)),
            })),
            display_id,
            app_id: Some("dev.rmac.AppSwitcher".to_owned()),
            window_background: WindowBackgroundAppearance::Transparent,
            // No anchor: the compositor centres the surface on the output,
            // which is where macOS puts the switcher.
            kind: WindowKind::LayerShell(LayerShellOptions {
                namespace: NAMESPACE.to_owned(),
                layer: Layer::Overlay,
                keyboard_interactivity: KeyboardInteractivity::Exclusive,
                ..Default::default()
            }),
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            ..Default::default()
        };
        match cx.open_window(options, move |window, cx| {
            cx.new(|cx| SwitcherView::new(weak, session, items, display_width, window, cx))
        }) {
            Ok(handle) => {
                // Allocate the swapchain at the widest panel this display can
                // show now, while the surface is 1 × 1: revealing it then only
                // changes the `wp_viewport` crop, in the same frame, instead
                // of rebuilding the swapchain (SPEED-11).
                let panel = model::layout(1, display_width);
                gpui_linux::reserve_window_drawable(
                    handle.into(),
                    Size::new(px(display_width), px(panel.height)),
                );
                // Kept surfaces are matched on the display asked for here
                // (the window may not know its output yet).
                let _ = handle.update(cx, |view, _, _| view.display_id = display_id);
                service.update(cx, |service, _| service.open = Some(handle));
            }
            Err(error) => eprintln!("could not open the app switcher: {error}"),
        }
    }

    fn run_service() -> Result<(), String> {
        let listener = crate::ipc::Listener::bind()
            .map_err(|error| format!("could not bind the app switcher socket: {error}"))?;
        let (command_tx, command_rx) = async_channel::bounded(32);
        std::thread::Builder::new()
            .name("rmac-app-switcher-ipc".into())
            .spawn(move || loop {
                match listener.receive() {
                    Ok(command) => {
                        if command_tx.send_blocking(command).is_err() {
                            return;
                        }
                    }
                    Err(error) => {
                        eprintln!("app switcher endpoint stopped: {error}");
                        std::process::exit(1);
                    }
                }
            })
            .map_err(|error| format!("could not start the app switcher endpoint: {error}"))?;

        let app = application().with_quit_mode(QuitMode::Explicit);
        app.run(move |cx: &mut App| {
            tokens::install_appearance_watch(cx);
            let service = cx.new(Service::new);

            let (compositor_tx, compositor_rx) = async_channel::bounded(64);
            cx.background_executor()
                .spawn(async move {
                    if let Err(error) = rmac_compositor_niri::watch(compositor_tx).await {
                        eprintln!("app switcher compositor watcher stopped: {error}");
                    }
                })
                .detach();
            let watched = service.clone();
            cx.spawn(async move |cx: &mut AsyncApp| {
                while let Ok(event) = compositor_rx.recv().await {
                    cx.update(|cx| {
                        watched.update(cx, |service, cx| {
                            service.apply(event);
                            service.refresh_force_quit(cx);
                        })
                    });
                }
            })
            .detach();

            cx.spawn(async move |cx: &mut AsyncApp| {
                while let Ok(command) = command_rx.recv().await {
                    cx.update(|cx| handle_command(&service, command, cx));
                }
            })
            .detach();
        });
        Ok(())
    }

    pub fn run() -> Result<(), String> {
        let arguments = std::env::args().skip(1).collect::<Vec<_>>();
        match arguments.as_slice() {
            [service] if service == "--service" => run_service(),
            [command] => {
                let command = Command::parse(command)
                    .ok_or_else(|| format!("unknown app switcher command: {command}"))?;
                crate::ipc::send(command)
                    .map_err(|error| format!("app switcher is not running: {error}"))
            }
            _ => Err(
                "usage: app-switcher --service | next | previous | cancel | force-quit".to_owned(),
            ),
        }
    }
}

#[cfg(all(target_os = "linux", feature = "wayland"))]
fn main() {
    if let Err(error) = linux_wayland::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(not(all(target_os = "linux", feature = "wayland")))]
fn main() {
    // Keep the platform-independent model linked (and warning-free) here.
    let _ = model::Command::parse;
    eprintln!("The app switcher requires Linux and: cargo run --features wayland --bin app-switcher -- --service");
    std::process::exit(2);
}
