//! rmac ⌘Tab application switcher (docs/decisions/0009-app-switcher.md).
//!
//! `app-switcher --service` is the resident session component. niri's
//! ⌘Tab / ⌘⇧Tab binds run `app-switcher next|previous`, which forwards one
//! word to the service and exits. The service opens an exclusive-keyboard
//! overlay, tracks ⌘ through the surface's modifier events, and activates the
//! selected application when ⌘ is released.

#[cfg(unix)]
mod ipc;
mod model;

#[cfg(all(target_os = "linux", feature = "wayland"))]
mod linux_wayland {
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::time::Duration;

    use gpui::{
        div, img, layer_shell::*, linear_color_stop, linear_gradient, point, prelude::*, px,
        rgba, AnyElement, App, AsyncApp, Bounds, Context, Entity, FocusHandle, FontWeight,
        KeyDownEvent, ModifiersChangedEvent, QuitMode, Role, Size, WeakEntity, Window,
        WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions,
    };
    use gpui_platform::application;
    use rmac_compositor::Action;
    use rmac_shell_ui::tokens;
    use uuid::Uuid;

    use crate::model::{self, Command, Layout, Recency, RunningApp, Session};

    /// A quick ⌘Tab tap switches without flashing the panel, as on macOS:
    /// the surface takes the keyboard at once but stays 1 × 1 until then.
    const REVEAL_DELAY: Duration = Duration::from_millis(120);
    /// After the surface gains focus, wait for the compositor's modifier
    /// state before treating "⌘ is up" as a release.
    const RELEASE_GRACE: Duration = Duration::from_millis(80);
    /// Never keep an invisible exclusive surface that never got the keyboard.
    const ACTIVATION_TIMEOUT: Duration = Duration::from_millis(1_000);
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

    struct Service {
        compositor: rmac_compositor::State,
        recency: Recency,
        catalog: Rc<Vec<rmac_apps::Application>>,
        open: Option<WindowHandle<SwitcherView>>,
    }

    impl Service {
        fn new(cx: &mut Context<Self>) -> Self {
            let mut service = Self {
                compositor: rmac_compositor::State::default(),
                recency: Recency::default(),
                catalog: Rc::new(Vec::new()),
                open: None,
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
                let catalog = cx
                    .background_executor()
                    .spawn(async { rmac_apps::discover() })
                    .await;
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
            self.refresh_catalog(cx);
        }

        fn item(&self, app: &RunningApp) -> Item {
            let entry = rmac_apps::find_desktop_entry(&self.catalog, &app.app_id);
            let name = entry
                .map(|entry| entry.name.clone())
                .or_else(|| rmac_apps::identity::window_title(&app.app_id).map(str::to_owned))
                .unwrap_or_else(|| fallback_name(&app.app_id));
            let icon = entry
                .and_then(|entry| entry.icon.clone())
                .filter(|path| path.is_file())
                .or_else(|| packaged_icon(&app.app_id));
            Item { name, icon }
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
        [
            PathBuf::from("/usr/share/icons/hicolor/scalable/apps").join(&file),
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../packaging/rmac-apps/icons")
                .join(&file),
        ]
        .into_iter()
        .find(|path| path.is_file())
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
                        this.arm_after_grace(window, cx);
                    }
                } else if this.was_active {
                    // Another surface took the keyboard: cancel, like Esc.
                    this.close(window, cx);
                }
            })
            .detach();
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(REVEAL_DELAY).await;
                let _ = this.update_in(cx, |this, window, cx| this.reveal(window, cx));
            })
            .detach();
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(ACTIVATION_TIMEOUT).await;
                let _ = this.update_in(cx, |this, window, cx| {
                    if !this.was_active {
                        this.close(window, cx);
                    }
                });
            })
            .detach();
            let layout = model::layout(session.apps.len(), display_width);
            Self {
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
            }
        }

        fn arm_after_grace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(RELEASE_GRACE).await;
                let _ = this.update_in(cx, |this, window, cx| {
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
            window.remove_window();
        }

        fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
            if self.closing {
                return;
            }
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
        fn quit_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
            let Some(app) = self.session.selected_app().cloned() else {
                return;
            };
            let Some(snapshot) = self.snapshot(cx) else {
                return;
            };
            let windows = rmac_compositor::windows_of_application(&snapshot, &app.app_id);
            cx.spawn(async move |_, _cx: &mut AsyncApp| {
                let mut store = rmac_compositor::ParkingStore::load_default();
                for window in &windows {
                    store.forget(*window);
                }
                for window in windows {
                    let action = Action::CloseWindow { window };
                    if let Err(error) = rmac_compositor_niri::execute_action(&action).await {
                        eprintln!("app switcher could not quit a window: {error:?}");
                    }
                }
                if let Err(error) = store.save_default() {
                    eprintln!("app switcher could not save the parking set: {error}");
                }
            })
            .detach();
            let _ = self
                .service
                .update(cx, |service, _| service.recency.forget(&app.app_id));
            self.items.remove(&app.app_id);
            if !self.session.remove(&app.app_id) {
                self.close(window, cx);
                return;
            }
            self.layout = model::layout(self.session.apps.len(), self.display_width);
            if self.revealed {
                window.resize(Size::new(px(self.layout.width), px(self.layout.height)));
            }
            cx.notify();
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

        fn tile(&self, index: usize, app: &RunningApp, colors: &Colors, cx: &mut Context<Self>) -> AnyElement {
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
                .child(icon(item, &app.app_id, layout.icon))
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
    fn icon(item: Option<&Item>, app_id: &str, size: f32) -> AnyElement {
        if let Some(path) = item.and_then(|item| item.icon.clone()) {
            return img(path)
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
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let layout = self.layout;
            let colors = Colors::current();
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
                .map(|(index, app)| self.tile(index, app, &colors, cx))
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
        let open = service.read(cx).open;
        if let Some(handle) = open {
            let handled = handle
                .update(cx, |view, window, cx| match command {
                    Command::Next => view.step(true, cx),
                    Command::Previous => view.step(false, cx),
                    Command::Cancel => view.close(window, cx),
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
            Command::Cancel => {}
        }
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
        let weak = service.downgrade();
        let options = WindowOptions {
            titlebar: None,
            focus: true,
            show: true,
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(0.0), px(0.0)),
                size: Size::new(px(HIDDEN_SIZE), px(HIDDEN_SIZE)),
            })),
            display_id: display.as_ref().map(|display| display.id()),
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
            Ok(handle) => service.update(cx, |service, _| service.open = Some(handle)),
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
                    cx.update(|cx| watched.update(cx, |service, _| service.apply(event)));
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
            _ => Err("usage: app-switcher --service | next | previous | cancel".to_owned()),
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
