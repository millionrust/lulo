//! rmac Mission Control, App Exposé, Show Desktop, Spaces and hot corners
//! (docs/decisions/0014-mission-control.md).
//!
//! `mission-control --service` is the resident session component. niri's
//! ⌃↑ / ⌃↓ / F11 / ⌃← / ⌃→ binds run `mission-control <word>`, which forwards
//! one word to the service and exits. The service reads the focused output
//! once, maps a full-screen overlay that niri backs with the bare wallpaper
//! (layer-rule xray), and flies the windows' pictures into the Mac's
//! measured layout. It also owns the Spaces bar, Show Desktop, the
//! wallpaper click that pushes windows aside, and the hot-corner surfaces
//! configured in Desktop & Dock.

mod capture;
#[cfg(unix)]
mod ipc;
mod model;

#[cfg(all(target_os = "linux", feature = "wayland"))]
mod linux_wayland {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use gpui::{
        div, img, layer_shell::*, point, prelude::*, px, rgba, AnyElement, App, AsyncApp, Bounds,
        Context, Entity, FocusHandle, FontWeight, KeyDownEvent, MouseButton, ObjectFit, QuitMode,
        RenderImage, Role, Size, WeakEntity, Window, WindowBackgroundAppearance, WindowBounds,
        WindowHandle, WindowKind, WindowOptions,
    };
    use gpui_platform::application;
    use rmac_compositor::reveal::{self, Revealed};
    use rmac_compositor::{
        frame_to_percent, Action, Distance, OutputId, Snapshot, TileHistoryStore, TileRegion,
        WindowId, WorkspaceId,
    };
    use rmac_shell_settings::{ClickWallpaperToReveal, HotCornerSettings};
    use rmac_shell_ui::tokens;
    use uuid::Uuid;

    use crate::capture::{self, Captured};
    use crate::model::{self, Command, Corner, CornerEffect, Mode, Rect, Scene, Space};

    const NAMESPACE: &str = "rmac-mission-control";
    const CORNER_NAMESPACE: &str = "rmac-hot-corner";
    const FRAME: Duration = Duration::from_millis(8);
    const ANIMATION: Duration = Duration::from_millis(model::ANIMATION_MS);
    /// Windows glide to their new slots when the Spaces bar grows (S).
    const RELAYOUT: Duration = Duration::from_millis(250);
    /// The height rmac's menu bar reserves (rmac-menubar BAR_HEIGHT). The
    /// Mac measures its wallpaper-click reveal from below the menu bar.
    const MENU_BAR_HEIGHT: f64 = 29.0;
    /// The Dock's reservation (rmac-dock EXCLUSIVE_ZONE, WIN-02). Fill,
    /// Centre and the halves (WIN-01) measure their working area between
    /// this and the menu bar, above.
    const DOCK_HEIGHT: f64 = 89.0;
    /// niri's window-movement animation (shell.kdl: 300 ms). Under Reduce
    /// Motion the screen holds this long while windows move, then
    /// crossfades; Mission Control waits this long for windows coming back.
    const WINDOW_MOVEMENT_MS: u16 = 300;

    // Measured over the wallpaper on macOS 26 (design-lab/mission-control.html).
    const BAR_FILL: u32 = 0xFFFF_FF0D;
    const BAR_TOP_EDGE: u32 = 0xFFFF_FF30;
    const BAR_BOTTOM_EDGE: u32 = 0xFFFF_FF1C;
    const PILL_FILL: u32 = 0xFFFF_FF26;
    const ADD_FILL: u32 = 0xFFFF_FF1A;
    const SPACE_RING: u32 = 0x5697_F5FF;
    const HOVER_RING_MISSION_CONTROL: u32 = 0x5595_F1FF;
    const HOVER_RING_APP_WINDOWS: u32 = 0x4070_B5FF;
    const TITLE_FILL: u32 = 0xB6B7_BEFF;
    const TITLE_TEXT: u32 = 0x1B1B_1CFF;
    const REMOVE_FILL: u32 = 0xFFFF_FFFF;
    const REMOVE_GLYPH: u32 = 0x8787_87FF;
    const WHITE: u32 = 0xFFFF_FFFF;
    /// A window nothing on screen shows whole: the rmac content surface
    /// (docs/measured-colors-2026-09-19.md) with the app's icon (S).
    const CARD_FILL: u32 = 0x2220_25FF;
    /// rmac's window corner radius (shell.kdl geometry-corner-radius).
    const WINDOW_RADIUS: f32 = 16.0;

    /// Name and icon of one application.
    #[derive(Clone)]
    struct Item {
        name: String,
        icon: Option<PathBuf>,
    }

    /// The last picture of a Space and the windows it showed then.
    struct SpacePicture {
        windows: Vec<WindowId>,
        image: Arc<RenderImage>,
    }

    struct Service {
        compositor: rmac_compositor::State,
        catalog: Rc<Vec<rmac_apps::Application>>,
        overlay: Option<WindowHandle<Overlay>>,
        opening: bool,
        /// The + button was pressed while standing on niri's spare workspace;
        /// name the next spare one as soon as niri creates it.
        pending_add: Option<OutputId>,
        corners: HotCornerSettings,
        click_to_reveal: ClickWallpaperToReveal,
        /// Windows pushed aside by a wallpaper click. Nothing is persisted.
        revealed: Option<Revealed>,
        /// Windows an earlier run left at an edge were pulled back.
        recovered: bool,
        corner_key: Vec<(Uuid, Corner)>,
        corner_windows: Vec<WindowHandle<CornerView>>,
        space_pictures: HashMap<WorkspaceId, SpacePicture>,
        /// The last picture captured of each window, kept across captures so
        /// an occluded window (MC-01) still shows a real picture rather than
        /// only its icon.
        window_pictures: HashMap<WindowId, Arc<RenderImage>>,
        /// The frame Fill, Centre or a half (WIN-01) moved a window from,
        /// so 🌐⌃R can put it back. Persisted so it survives this service
        /// restarting (`ParkingStore`'s reasoning, crates/rmac-compositor).
        tile_history: TileHistoryStore,
    }

    impl Service {
        fn new(cx: &mut Context<Self>) -> Self {
            let mut service = Self {
                compositor: rmac_compositor::State::default(),
                catalog: Rc::new(Vec::new()),
                overlay: None,
                opening: false,
                pending_add: None,
                corners: HotCornerSettings::default(),
                click_to_reveal: ClickWallpaperToReveal::default(),
                revealed: None,
                recovered: false,
                corner_key: Vec::new(),
                corner_windows: Vec::new(),
                space_pictures: HashMap::new(),
                window_pictures: HashMap::new(),
                tile_history: TileHistoryStore::load_default(),
            };
            service.refresh_catalog(cx);
            service
        }

        /// Apply one compositor event. Returns whether outputs may have
        /// changed, and an action the model now wants run.
        fn apply(&mut self, event: rmac_compositor::Event) -> (bool, Option<Action>) {
            let change = self.compositor.apply(event);
            let mut action = None;
            if let Some(output) = self.pending_add.clone() {
                let snapshot = self.compositor.snapshot();
                if let Some(model::Added::New(add)) = model::add_space(&snapshot, &output) {
                    self.pending_add = None;
                    action = Some(add);
                }
            }
            let live: Vec<WorkspaceId> = self.compositor.workspaces.keys().copied().collect();
            self.space_pictures
                .retain(|workspace, _| live.contains(workspace));
            let live_windows: Vec<WindowId> = self.compositor.windows.keys().copied().collect();
            self.window_pictures
                .retain(|window, _| live_windows.contains(window));
            self.tile_history.prune(&live_windows);
            (change.topology, action)
        }

        /// Record `window`'s current frame as where Fill, Centre or a half
        /// (WIN-01) moved it from, unless one is already recorded: a second
        /// tiling action keeps the frame from before the first, so 🌐⌃R
        /// always lands back on the window's original size.
        fn record_tile_history(&mut self, snapshot: &Snapshot, window: WindowId) {
            let Some(percent) = tile_history_percent(snapshot, window) else {
                return;
            };
            self.tile_history.record(window, percent);
            if let Err(error) = self.tile_history.save_default() {
                eprintln!("Mission Control could not save the tile history: {error}");
            }
        }

        /// 🌐⌃R: the action that puts `window` back, or `None` when nothing
        /// is recorded for it (an honest no-op, not a simulated size).
        fn restore_window_size(&mut self, window: WindowId) -> Option<Action> {
            let frame = self.tile_history.take(window)?;
            if let Err(error) = self.tile_history.save_default() {
                eprintln!("Mission Control could not save the tile history: {error}");
            }
            Some(Action::SetWindowFrame {
                window,
                x: Distance(frame.x),
                y: Distance(frame.y),
                width: Distance(frame.width),
                height: Distance(frame.height),
            })
        }

        /// A wallpaper click: bring pushed-aside windows back, or push every
        /// window on the showing Spaces to the screen edges.
        fn wallpaper_click(&mut self) -> Vec<Action> {
            if self.revealed.is_some() {
                return self.bring_back();
            }
            if self.click_to_reveal == ClickWallpaperToReveal::Never {
                return Vec::new();
            }
            self.push_to_edges()
        }

        /// F11: push every window aside like a wallpaper click, leaving Mac
        /// slivers at the screen edges (crates/rmac-compositor/src/reveal.rs).
        /// Unlike a wallpaper click this always runs, regardless of "click
        /// wallpaper to reveal desktop". F11 again brings the windows back.
        fn show_desktop(&mut self) -> Vec<Action> {
            if self.revealed.is_some() {
                self.bring_back()
            } else {
                self.push_to_edges()
            }
        }

        /// Push every window on the showing Spaces to the screen edges.
        /// Empty when there is nothing to push.
        fn push_to_edges(&mut self) -> Vec<Action> {
            let snapshot = self.compositor.snapshot();
            let Some((revealed, actions)) =
                reveal::reveal(&snapshot, MENU_BAR_HEIGHT, reveal_sliver())
            else {
                return Vec::new();
            };
            self.revealed = Some(revealed);
            with_motion(actions)
        }

        /// Bring windows pushed aside by a wallpaper click back to where
        /// they were. Empty when none are aside.
        fn bring_back(&mut self) -> Vec<Action> {
            let Some(revealed) = self.revealed.take() else {
                return Vec::new();
            };
            with_motion(reveal::restore(&self.compositor.snapshot(), &revealed))
        }

        /// After each compositor event: bring the windows back when a window
        /// takes focus (a sliver was clicked, an app was activated, an item
        /// was opened) or the Space changes. On the first snapshot, pull
        /// back windows an earlier run of this service left at an edge.
        fn follow_reveal(&mut self, reconnected: bool) -> Vec<Action> {
            if reconnected {
                if self.revealed.is_some() {
                    return self.bring_back();
                }
                if !self.recovered {
                    self.recovered = true;
                    let snapshot = self.compositor.snapshot();
                    return reveal::recover(&snapshot, MENU_BAR_HEIGHT, reveal_sliver());
                }
                return Vec::new();
            }
            let Some(revealed) = self.revealed.as_mut() else {
                return Vec::new();
            };
            if revealed.should_restore(&self.compositor.snapshot()) {
                self.bring_back()
            } else {
                Vec::new()
            }
        }

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
                    Err(error) => {
                        eprintln!("Mission Control could not read applications: {error}")
                    }
                }
            })
            .detach();
        }

        fn item(&self, app_id: &str) -> Item {
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

    fn render_image((width, height, bgra): capture::Pixels) -> Option<Arc<RenderImage>> {
        let buffer = image::RgbaImage::from_raw(width, height, bgra)?;
        Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
    }

    /// The laid-out width of one title line in the capsule's font, so the
    /// capsule can be centred under its thumbnail and sized to its text.
    fn title_width(window: &Window, title: &str) -> f32 {
        if title.is_empty() {
            return 0.0;
        }
        let mut font = gpui::font("Inter");
        font.weight = FontWeight::MEDIUM;
        let run = gpui::TextRun {
            len: title.len(),
            font,
            color: rgba(WHITE).into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = window.text_system().shape_line(
            gpui::SharedString::from(title.to_owned()),
            px(model::TITLE_SIZE),
            &[run],
            None,
        );
        f32::from(line.width())
    }

    /// How much of each window stays on screen: the Mac leaves 12 pt, but
    /// niri will not move a floating window further than 75 px from the
    /// working area.
    fn reveal_sliver() -> f64 {
        reveal::MAC_SLIVER.max(rmac_compositor_niri::FLOATING_MIN_VISIBLE)
    }

    /// `window`'s current floating frame, as a percentage of its output's
    /// working area (WIN-01/WIN-02's menu bar and Dock reservation), for
    /// "Return to Previous Size" to restore later. `None` for a window rmac
    /// cannot place this way: not floating (every rmac window is), its
    /// output's logical size is not known yet, or it has no layout (a
    /// hidden Space).
    fn tile_history_percent(
        snapshot: &Snapshot,
        window: WindowId,
    ) -> Option<rmac_compositor::FramePercent> {
        let window = snapshot
            .windows
            .iter()
            .find(|candidate| candidate.id == window)?;
        if !window.floating {
            return None;
        }
        let workspace = snapshot
            .workspaces
            .iter()
            .find(|workspace| Some(workspace.id) == window.workspace)?;
        let output = snapshot
            .outputs
            .iter()
            .find(|output| Some(&output.id) == workspace.output.as_ref())?;
        let logical = output.logical.as_ref()?;
        let position = window.layout.tile_position_in_view?;
        let size = window.layout.tile_size;
        frame_to_percent(
            logical.size.width,
            logical.size.height,
            MENU_BAR_HEIGHT,
            DOCK_HEIGHT,
            position.x,
            position.y,
            size.width,
            size.height,
        )
    }

    /// Under Reduce Motion, hold the screen while niri moves the windows and
    /// crossfade to the result instead of showing them slide.
    fn with_motion(mut actions: Vec<Action>) -> Vec<Action> {
        if !actions.is_empty() && tokens::current().motion.reduced_motion {
            actions.insert(
                0,
                Action::CrossfadeScreen {
                    delay_ms: WINDOW_MOVEMENT_MS,
                },
            );
        }
        actions
    }

    /// Run compositor actions in order, one socket each.
    fn run_actions(actions: Vec<Action>, cx: &mut App) {
        if actions.is_empty() {
            return;
        }
        cx.spawn(async move |_cx: &mut AsyncApp| {
            for action in actions {
                if let Err(error) = rmac_compositor_niri::execute_action(&action).await {
                    eprintln!(
                        "Mission Control could not run {:?}: {error:?}",
                        action.kind()
                    );
                }
            }
        })
        .detach();
    }

    /// Notification Centre, Apps and Lock Screen belong to other services;
    /// the shortcut dispatcher installed beside this binary reaches them.
    fn dispatch_shortcut(id: &'static str) {
        let program = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join("rmac-shortcut-dispatch")))
            .filter(|path| path.is_file())
            .unwrap_or_else(|| PathBuf::from("/usr/libexec/rmac/rmac-shortcut-dispatch"));
        match std::process::Command::new(program)
            .arg(id)
            .stdin(std::process::Stdio::null())
            .spawn()
        {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(error) => eprintln!("hot corner could not run the shortcut dispatcher: {error}"),
        }
    }

    // ---------------------------------------------------------------------
    // The overlay

    enum Exit {
        Nothing,
        Focus(WindowId),
        Space(WorkspaceId),
    }

    struct Closing {
        started: Instant,
        from: f32,
        exit: Exit,
    }

    struct Overlay {
        service: WeakEntity<Service>,
        mode: Mode,
        scene: Scene,
        snapshot: Snapshot,
        spaces: Vec<Space>,
        pictures: HashMap<WindowId, Arc<RenderImage>>,
        space_pictures: HashMap<WorkspaceId, Arc<RenderImage>>,
        items: HashMap<String, Item>,
        focus: FocusHandle,
        opened: Instant,
        closing: Option<Closing>,
        expanded: bool,
        targets: Vec<Rect>,
        relayout: Option<(Instant, Vec<Rect>)>,
        hovered: Option<usize>,
        selected: Option<usize>,
        hovered_space: Option<usize>,
        ticking: bool,
        was_active: bool,
    }

    impl Overlay {
        #[allow(clippy::too_many_arguments)]
        fn new(
            service: WeakEntity<Service>,
            mode: Mode,
            scene: Scene,
            snapshot: Snapshot,
            pictures: HashMap<WindowId, Arc<RenderImage>>,
            space_pictures: HashMap<WorkspaceId, Arc<RenderImage>>,
            items: HashMap<String, Item>,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) -> Self {
            let focus = cx.focus_handle();
            focus.focus(window, cx);
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.was_active = true;
                } else if this.was_active {
                    // Another surface took the keyboard: leave, like Esc.
                    this.dismiss(Exit::Nothing, window, cx);
                }
            })
            .detach();
            let spaces = model::spaces(&snapshot, &scene.output);
            let mut overlay = Self {
                service,
                mode,
                scene,
                snapshot,
                spaces,
                pictures,
                space_pictures,
                items,
                focus,
                opened: Instant::now(),
                closing: None,
                expanded: false,
                targets: Vec::new(),
                relayout: None,
                hovered: None,
                selected: None,
                hovered_space: None,
                ticking: false,
                was_active: false,
            };
            overlay.targets = overlay.layout();
            overlay.ensure_ticking(window, cx);
            overlay
        }

        fn bar_height(&self) -> f32 {
            if self.expanded {
                model::BAR_EXPANDED
            } else {
                model::BAR_COLLAPSED
            }
        }

        fn layout(&self) -> Vec<Rect> {
            let frames: Vec<Rect> = self
                .scene
                .windows
                .iter()
                .map(|window| window.frame)
                .collect();
            let (width, height) = (self.scene.width, self.scene.height);
            match self.mode {
                Mode::MissionControl => model::layout(
                    &frames,
                    model::mission_control_area(width, height, self.bar_height()),
                    model::GAP_X,
                    model::GAP_Y,
                    0.0,
                ),
                Mode::AppWindows => model::layout(
                    &frames,
                    model::app_windows_area(width, height),
                    model::GAP_X,
                    0.0,
                    model::CAPTION_BAND,
                ),
            }
        }

        /// 0 = the windows where they are on screen, 1 = laid out.
        fn progress(&self) -> f32 {
            let over = ANIMATION.as_secs_f32();
            match &self.closing {
                Some(closing) => {
                    closing.from
                        * (1.0 - model::ease(closing.started.elapsed().as_secs_f32() / over))
                }
                None => model::ease(self.opened.elapsed().as_secs_f32() / over),
            }
        }

        fn targets_now(&self) -> Vec<Rect> {
            match &self.relayout {
                Some((started, from)) => {
                    let t = model::ease(started.elapsed().as_secs_f32() / RELAYOUT.as_secs_f32());
                    from.iter()
                        .zip(&self.targets)
                        .map(|(from, to)| from.lerp(to, t))
                        .collect()
                }
                None => self.targets.clone(),
            }
        }

        fn animating(&self) -> bool {
            self.closing.is_some()
                || self.opened.elapsed() < ANIMATION
                || self
                    .relayout
                    .as_ref()
                    .is_some_and(|(started, _)| started.elapsed() < RELAYOUT)
        }

        fn ensure_ticking(&mut self, window: &mut Window, cx: &mut Context<Self>) {
            if self.ticking {
                return;
            }
            self.ticking = true;
            cx.spawn_in(window, async move |this, cx| loop {
                cx.background_executor().timer(FRAME).await;
                let running = this
                    .update_in(cx, |this, window, cx| this.tick(window, cx))
                    .unwrap_or(false);
                if !running {
                    break;
                }
            })
            .detach();
        }

        fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
            if self
                .closing
                .as_ref()
                .is_some_and(|closing| closing.started.elapsed() >= ANIMATION)
            {
                self.finish(window, cx);
                return false;
            }
            if self
                .relayout
                .as_ref()
                .is_some_and(|(started, _)| started.elapsed() >= RELAYOUT)
            {
                self.relayout = None;
            }
            cx.notify();
            let running = self.animating();
            if !running {
                self.ticking = false;
            }
            running
        }

        /// Leave Mission Control. Choosing a Space leaves at once; anything
        /// else flies the windows back first.
        fn dismiss(&mut self, exit: Exit, window: &mut Window, cx: &mut Context<Self>) {
            if self.closing.is_some() {
                return;
            }
            let immediate = matches!(exit, Exit::Space(_));
            self.closing = Some(Closing {
                started: Instant::now(),
                from: self.progress(),
                exit,
            });
            if immediate {
                self.finish(window, cx);
            } else {
                self.ensure_ticking(window, cx);
            }
        }

        fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
            let Some(closing) = self.closing.take() else {
                return;
            };
            // Keep the overlay marked closed so a late tick does nothing.
            self.closing = Some(Closing {
                started: closing.started,
                from: 0.0,
                exit: Exit::Nothing,
            });
            window.remove_window();
            let _ = self.service.update(cx, |service, _| service.overlay = None);
            let action = match closing.exit {
                Exit::Nothing => None,
                Exit::Focus(window) => Some(Action::FocusWindow { window }),
                Exit::Space(workspace) => Some(Action::FocusWorkspace { workspace }),
            };
            run_actions(action.into_iter().collect(), cx);
        }

        fn commit(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
            if let Some(scene_window) = self.scene.windows.get(index) {
                let id = scene_window.id;
                self.dismiss(Exit::Focus(id), window, cx);
            }
        }

        fn set_expanded(&mut self, expanded: bool, window: &mut Window, cx: &mut Context<Self>) {
            if self.expanded == expanded || self.closing.is_some() {
                return;
            }
            let current = self.targets_now();
            self.expanded = expanded;
            self.targets = self.layout();
            self.relayout = Some((Instant::now(), current));
            if !expanded {
                self.hovered_space = None;
            }
            self.ensure_ticking(window, cx);
            cx.notify();
        }

        /// The Space list changed under the open overlay (+, remove, or a
        /// window moved).
        fn refresh_spaces(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
            self.spaces = model::spaces(&snapshot, &self.scene.output);
            self.snapshot = snapshot;
            self.hovered_space = None;
            cx.notify();
        }

        fn service_snapshot(&self, cx: &App) -> Option<Snapshot> {
            self.service
                .upgrade()
                .map(|service| service.read(cx).compositor.snapshot())
        }

        fn add_space(&mut self, cx: &mut Context<Self>) {
            let Some(snapshot) = self.service_snapshot(cx) else {
                return;
            };
            match model::add_space(&snapshot, &self.scene.output) {
                Some(model::Added::New(action)) => run_actions(vec![action], cx),
                Some(model::Added::PinnedCurrent(action)) => {
                    let output = self.scene.output.clone();
                    let _ = self
                        .service
                        .update(cx, |service, _| service.pending_add = Some(output));
                    run_actions(vec![action], cx);
                }
                None => {}
            }
        }

        fn remove_space(&mut self, workspace: WorkspaceId, cx: &mut Context<Self>) {
            let Some(snapshot) = self.service_snapshot(cx) else {
                return;
            };
            let actions = model::remove_space(&snapshot, &self.scene.output, workspace);
            run_actions(actions, cx);
        }

        fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
            let step = |this: &mut Self, dx: f32, dy: f32, cx: &mut Context<Self>| {
                let targets = this.targets_now();
                this.selected =
                    model::step_selection(&targets, this.selected.or(this.hovered), dx, dy);
                cx.notify();
            };
            match event.keystroke.key.as_str() {
                "escape" => self.dismiss(Exit::Nothing, window, cx),
                "enter" => match self.selected.or(self.hovered) {
                    Some(index) => self.commit(index, window, cx),
                    None => self.dismiss(Exit::Nothing, window, cx),
                },
                "left" => step(self, -1.0, 0.0, cx),
                "right" => step(self, 1.0, 0.0, cx),
                "up" => step(self, 0.0, -1.0, cx),
                "down" => step(self, 0.0, 1.0, cx),
                _ => {}
            }
            cx.stop_propagation();
        }

        // -----------------------------------------------------------------
        // Drawing

        fn window_element(
            &self,
            index: usize,
            rect: Rect,
            progress: f32,
            window: &Window,
            cx: &mut Context<Self>,
        ) -> AnyElement {
            let scene_window = &self.scene.windows[index];
            let scale = rect.width / scene_window.frame.width.max(1.0);
            let radius = WINDOW_RADIUS * scale;
            let highlighted = self.closing.is_none()
                && progress > 0.99
                && (self.hovered == Some(index) || self.selected == Some(index));
            let item = scene_window
                .app_id
                .as_ref()
                .and_then(|app_id| self.items.get(app_id));

            let mut element = div()
                .id(("mission-control-window", index))
                .role(Role::Button)
                .aria_label(
                    scene_window
                        .title
                        .clone()
                        .or_else(|| item.map(|item| item.name.clone()))
                        .unwrap_or_default(),
                )
                .absolute()
                .left(px(rect.x))
                .top(px(rect.y))
                .w(px(rect.width))
                .h(px(rect.height))
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if *hovered {
                        this.hovered = Some(index);
                        this.selected = None;
                    } else if this.hovered == Some(index) {
                        this.hovered = None;
                    }
                    cx.notify();
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.commit(index, window, cx);
                    }),
                );

            if highlighted {
                let reach = model::HOVER_GAP + model::HOVER_RING;
                element = element.child(
                    div()
                        .absolute()
                        .left(px(-reach))
                        .top(px(-reach))
                        .w(px(rect.width + 2.0 * reach))
                        .h(px(rect.height + 2.0 * reach))
                        .rounded(px(radius + reach))
                        .border(px(model::HOVER_RING))
                        .border_color(rgba(match self.mode {
                            Mode::MissionControl => HOVER_RING_MISSION_CONTROL,
                            Mode::AppWindows => HOVER_RING_APP_WINDOWS,
                        })),
                );
            }

            element = match self.pictures.get(&scene_window.id) {
                Some(picture) => element.child(
                    img(picture.clone())
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .rounded(px(radius))
                        .object_fit(ObjectFit::Fill),
                ),
                None => {
                    // Covered on screen, so no picture of its own: a window
                    // plate with the application's icon.
                    let icon = (rect.width.min(rect.height) * 0.4).clamp(24.0, 128.0);
                    let mut card = div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .rounded(px(radius))
                        .bg(rgba(CARD_FILL))
                        .flex()
                        .items_center()
                        .justify_center();
                    if let Some(path) = item.and_then(|item| item.icon.clone()) {
                        card = card.child(img(path).size(px(icon)));
                    }
                    element.child(card)
                }
            };

            if highlighted && self.mode == Mode::MissionControl {
                let title = scene_window
                    .title
                    .clone()
                    .filter(|title| !title.is_empty())
                    .or_else(|| item.map(|item| item.name.clone()))
                    .unwrap_or_default();
                let capsule = model::title_capsule_frame(
                    rect,
                    title_width(window, &title) + 2.0 * model::TITLE_PADDING,
                    self.scene.width,
                );
                element = element.child(
                    div()
                        .absolute()
                        .left(px(capsule.x - rect.x))
                        .top(px(capsule.y - rect.y))
                        .w(px(capsule.width))
                        .h(px(capsule.height))
                        .px(px(model::TITLE_PADDING))
                        .rounded(px(model::TITLE_HEIGHT / 2.0))
                        .bg(rgba(TITLE_FILL))
                        .flex()
                        .items_center()
                        .font_family("Inter")
                        .text_size(px(model::TITLE_SIZE))
                        .line_height(px(model::TITLE_HEIGHT))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgba(TITLE_TEXT))
                        .truncate()
                        .child(title),
                );
            }
            element.into_any_element()
        }

        fn caption(&self, index: usize, rect: Rect, progress: f32) -> AnyElement {
            let scene_window = &self.scene.windows[index];
            let text = scene_window
                .title
                .clone()
                .filter(|title| !title.is_empty())
                .or_else(|| {
                    scene_window
                        .app_id
                        .as_ref()
                        .and_then(|app_id| self.items.get(app_id))
                        .map(|item| item.name.clone())
                })
                .unwrap_or_default();
            div()
                .absolute()
                .left(px(rect.x))
                .top(px(rect.bottom() + model::CAPTION_GAP))
                .w(px(rect.width))
                .h(px(model::CAPTION_LINE))
                .opacity(progress)
                .font_family("Inter")
                .text_size(px(model::CAPTION_SIZE))
                .line_height(px(model::CAPTION_LINE))
                .text_color(rgba(WHITE))
                .text_center()
                .truncate()
                .child(text)
                .into_any_element()
        }

        fn space_thumbnail(
            &self,
            index: usize,
            space: &Space,
            centre: f32,
            cx: &mut Context<Self>,
        ) -> AnyElement {
            let width = model::THUMB_WIDTH;
            let height = width * self.scene.height / self.scene.width.max(1.0);
            let scale = width / self.scene.width.max(1.0);
            let workspace = space.workspace;
            let removable = self.spaces.len() > 1 && self.hovered_space == Some(index);

            let mut thumbnail = div()
                .id(("space", index))
                .role(Role::Button)
                .aria_label(space.label.clone())
                .absolute()
                .left(px(centre - width / 2.0))
                .top(px(model::THUMB_TOP))
                .w(px(width))
                .h(px(height))
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if *hovered {
                        this.hovered_space = Some(index);
                    } else if this.hovered_space == Some(index) {
                        this.hovered_space = None;
                    }
                    cx.notify();
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.dismiss(Exit::Space(workspace), window, cx);
                    }),
                );

            if space.active {
                let reach = model::THUMB_RING_GAP + model::THUMB_RING;
                thumbnail = thumbnail.child(
                    div()
                        .absolute()
                        .left(px(-reach))
                        .top(px(-reach))
                        .w(px(width + 2.0 * reach))
                        .h(px(height + 2.0 * reach))
                        .rounded(px(model::THUMB_RING_RADIUS))
                        .border(px(model::THUMB_RING))
                        .border_color(rgba(SPACE_RING)),
                );
            }

            thumbnail = match self.space_pictures.get(&workspace) {
                Some(picture) => thumbnail.child(
                    img(picture.clone())
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .rounded(px(model::THUMB_RADIUS))
                        .object_fit(ObjectFit::Fill),
                ),
                None => {
                    // No picture yet: niri shows the wallpaper through the
                    // overlay here, and each window is a plate where it sits.
                    let plates = self
                        .snapshot
                        .windows
                        .iter()
                        .filter(|window| space.windows.contains(&window.id))
                        .filter_map(|window| {
                            let position = window.layout.tile_position_in_view?;
                            let size = window.layout.tile_size;
                            let frame = Rect::new(
                                position.x as f32,
                                position.y as f32,
                                size.width as f32,
                                size.height as f32,
                            )
                            .clipped(self.scene.width, self.scene.height);
                            (!frame.is_empty()).then(|| {
                                div()
                                    .absolute()
                                    .left(px(frame.x * scale))
                                    .top(px(frame.y * scale))
                                    .w(px(frame.width * scale))
                                    .h(px(frame.height * scale))
                                    .rounded(px(WINDOW_RADIUS * scale))
                                    .bg(rgba(CARD_FILL))
                            })
                        })
                        .collect::<Vec<_>>();
                    thumbnail.child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .rounded(px(model::THUMB_RADIUS))
                            .overflow_hidden()
                            .children(plates),
                    )
                }
            };

            thumbnail = thumbnail.child(
                div()
                    .absolute()
                    .left(px(-model::THUMB_PITCH / 2.0 + width / 2.0))
                    .top(px(model::THUMB_LABEL_TOP - model::THUMB_TOP))
                    .w(px(model::THUMB_PITCH))
                    .h(px(model::THUMB_LABEL_LINE))
                    .font_family("Inter")
                    .text_size(px(model::THUMB_LABEL))
                    .line_height(px(model::THUMB_LABEL_LINE))
                    .text_color(rgba(WHITE))
                    .text_center()
                    .truncate()
                    .child(space.label.clone()),
            );

            if removable {
                let diameter = model::REMOVE_DIAMETER;
                thumbnail = thumbnail.child(
                    div()
                        .id(("remove-space", index))
                        .role(Role::Button)
                        .aria_label(format!("Remove {}", space.label))
                        .absolute()
                        .left(px(-diameter / 2.0))
                        .top(px(-diameter / 2.0))
                        .size(px(diameter))
                        .rounded(px(diameter / 2.0))
                        .bg(rgba(REMOVE_FILL))
                        .flex()
                        .items_center()
                        .justify_center()
                        .font_family("Inter")
                        .text_size(px(model::REMOVE_GLYPH * 1.6))
                        .line_height(px(diameter))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgba(REMOVE_GLYPH))
                        .child("×")
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.remove_space(workspace, cx);
                            }),
                        ),
                );
            }
            thumbnail.into_any_element()
        }

        fn space_pill(
            &self,
            index: usize,
            space: &Space,
            centre: f32,
            cx: &mut Context<Self>,
        ) -> AnyElement {
            let workspace = space.workspace;
            div()
                .absolute()
                .left(px(centre - model::THUMB_PITCH / 2.0))
                .top(px(model::PILL_TOP))
                .w(px(model::THUMB_PITCH))
                .h(px(model::PILL_HEIGHT))
                .flex()
                .justify_center()
                .child(
                    div()
                        .id(("space-pill", index))
                        .role(Role::Button)
                        .aria_label(space.label.clone())
                        .h(px(model::PILL_HEIGHT))
                        .px(px(model::PILL_PADDING))
                        .rounded(px(model::PILL_HEIGHT / 2.0))
                        .bg(rgba(PILL_FILL))
                        .flex()
                        .items_center()
                        .font_family("Inter")
                        .text_size(px(model::PILL_LABEL))
                        .line_height(px(model::PILL_HEIGHT))
                        .text_color(rgba(WHITE))
                        .whitespace_nowrap()
                        .child(space.label.clone())
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.dismiss(Exit::Space(workspace), window, cx);
                            }),
                        ),
                )
                .into_any_element()
        }

        fn spaces_bar(&self, progress: f32, cx: &mut Context<Self>) -> AnyElement {
            let height = self.bar_height();
            let width = self.scene.width;
            let centres = model::space_centres(self.spaces.len(), width);
            let spaces: Vec<AnyElement> = self
                .spaces
                .iter()
                .zip(centres)
                .enumerate()
                .map(|(index, (space, centre))| {
                    if self.expanded {
                        self.space_thumbnail(index, space, centre, cx)
                    } else {
                        self.space_pill(index, space, centre, cx)
                    }
                })
                .collect();
            let add_centre = if self.expanded {
                model::ADD_CENTRE_EXPANDED
            } else {
                model::ADD_CENTRE_COLLAPSED
            };
            let diameter = model::ADD_DIAMETER;
            let plus = model::PLUS_SIZE;
            let stroke = model::PLUS_STROKE;
            div()
                .id("spaces-bar")
                .absolute()
                .left_0()
                .top(px(-height * (1.0 - progress)))
                .w(px(width))
                .h(px(height))
                .bg(rgba(BAR_FILL))
                .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                    this.set_expanded(*hovered, window, cx);
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .w(px(width))
                        .h(px(0.5))
                        .bg(rgba(BAR_TOP_EDGE)),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(height - 0.5))
                        .left_0()
                        .w(px(width))
                        .h(px(0.5))
                        .bg(rgba(BAR_BOTTOM_EDGE)),
                )
                .children(spaces)
                .child(
                    div()
                        .id("add-space")
                        .role(Role::Button)
                        .aria_label("Add Desktop")
                        .absolute()
                        .left(px(width - model::ADD_CENTRE_FROM_RIGHT - diameter / 2.0))
                        .top(px(add_centre - diameter / 2.0))
                        .size(px(diameter))
                        .rounded(px(diameter / 2.0))
                        .bg(rgba(ADD_FILL))
                        .child(
                            div()
                                .absolute()
                                .left(px((diameter - plus) / 2.0))
                                .top(px((diameter - stroke) / 2.0))
                                .w(px(plus))
                                .h(px(stroke))
                                .rounded(px(stroke / 2.0))
                                .bg(rgba(WHITE)),
                        )
                        .child(
                            div()
                                .absolute()
                                .left(px((diameter - stroke) / 2.0))
                                .top(px((diameter - plus) / 2.0))
                                .w(px(stroke))
                                .h(px(plus))
                                .rounded(px(stroke / 2.0))
                                .bg(rgba(WHITE)),
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.add_space(cx);
                            }),
                        ),
                )
                .into_any_element()
        }
    }

    impl Render for Overlay {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let progress = self.progress();
            let targets = self.targets_now();
            let rects: Vec<Rect> = self
                .scene
                .windows
                .iter()
                .zip(&targets)
                .map(|(window, target)| window.frame.lerp(target, progress))
                .collect();
            let windows: Vec<AnyElement> = rects
                .iter()
                .enumerate()
                .map(|(index, rect)| self.window_element(index, *rect, progress, window, cx))
                .collect();
            let captions: Vec<AnyElement> = if self.mode == Mode::AppWindows {
                rects
                    .iter()
                    .enumerate()
                    .map(|(index, rect)| self.caption(index, *rect, progress))
                    .collect()
            } else {
                Vec::new()
            };
            let bar = (self.mode == Mode::MissionControl).then(|| self.spaces_bar(progress, cx));
            div()
                .id("mission-control")
                .track_focus(&self.focus)
                .role(Role::Pane)
                .aria_label(match self.mode {
                    Mode::MissionControl => "Mission Control",
                    Mode::AppWindows => "Application Windows",
                })
                .relative()
                .size_full()
                .on_key_down(cx.listener(Self::key_down))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.dismiss(Exit::Nothing, window, cx)),
                )
                .children(windows)
                .children(captions)
                .children(bar)
        }
    }

    // ---------------------------------------------------------------------
    // Hot corners

    struct CornerView {
        service: WeakEntity<Service>,
        corner: Corner,
        /// The pointer must leave the corner before it fires again.
        armed: bool,
    }

    impl Render for CornerView {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div().id("hot-corner").size_full().on_hover(cx.listener(
                |this, hovered: &bool, _, cx| {
                    if !*hovered {
                        this.armed = true;
                        return;
                    }
                    if !this.armed {
                        return;
                    }
                    this.armed = false;
                    let Some(service) = this.service.upgrade() else {
                        return;
                    };
                    let action = this.corner.action(&service.read(cx).corners);
                    match model::corner_effect(action) {
                        Some(CornerEffect::Command(command)) => {
                            handle_command(&service, command, cx)
                        }
                        Some(CornerEffect::Dispatch(id)) => dispatch_shortcut(id),
                        None => {}
                    }
                },
            ))
        }
    }

    /// Keep one 1 × 1 surface in each configured corner of every display.
    fn reconcile_corners(service: &Entity<Service>, cx: &mut App) {
        let corners = model::active_corners(&service.read(cx).corners);
        let displays = rmac_shell_layer::output_surfaces::newest_displays(cx);
        let wanted: Vec<(Uuid, Corner)> = displays
            .keys()
            .flat_map(|uuid| corners.iter().map(move |corner| (*uuid, *corner)))
            .collect();
        if wanted == service.read(cx).corner_key {
            return;
        }
        let old = service.update(cx, |service, _| {
            service.corner_key = wanted.clone();
            std::mem::take(&mut service.corner_windows)
        });
        for handle in old {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
        let mut opened = Vec::new();
        for (uuid, corner) in wanted {
            let Some(display) = displays.get(&uuid) else {
                continue;
            };
            let anchor = match corner {
                Corner::TopLeft => Anchor::TOP | Anchor::LEFT,
                Corner::TopRight => Anchor::TOP | Anchor::RIGHT,
                Corner::BottomLeft => Anchor::BOTTOM | Anchor::LEFT,
                Corner::BottomRight => Anchor::BOTTOM | Anchor::RIGHT,
            };
            let options = WindowOptions {
                titlebar: None,
                focus: false,
                show: true,
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(0.0), px(0.0)),
                    size: Size::new(px(1.0), px(1.0)),
                })),
                display_id: Some(display.id()),
                app_id: Some("dev.rmac.HotCorner".to_owned()),
                window_background: WindowBackgroundAppearance::Transparent,
                kind: WindowKind::LayerShell(LayerShellOptions {
                    namespace: CORNER_NAMESPACE.to_owned(),
                    layer: Layer::Overlay,
                    anchor,
                    // The corner pixel itself, over the menu bar and Dock.
                    exclusive_zone: Some(px(-1.0)),
                    keyboard_interactivity: KeyboardInteractivity::None,
                    ..Default::default()
                }),
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                ..Default::default()
            };
            let weak = service.downgrade();
            match cx.open_window(options, move |_, cx| {
                cx.new(|_| CornerView {
                    service: weak,
                    corner,
                    armed: true,
                })
            }) {
                Ok(handle) => opened.push(handle),
                Err(error) => eprintln!("could not open a hot corner: {error}"),
            }
        }
        service.update(cx, |service, _| service.corner_windows = opened);
    }

    // ---------------------------------------------------------------------
    // Commands

    /// Dismiss an open overlay; returns whether one was open.
    fn close_overlay(service: &Entity<Service>, cx: &mut App) -> bool {
        let Some(handle) = service.read(cx).overlay else {
            return false;
        };
        let closed = handle
            .update(cx, |view, window, cx| {
                view.dismiss(Exit::Nothing, window, cx)
            })
            .is_ok();
        if !closed {
            service.update(cx, |service, _| service.overlay = None);
        }
        closed
    }

    fn handle_command(service: &Entity<Service>, command: Command, cx: &mut App) {
        // Every other command starts from the windows where they belong.
        // Show Desktop is a toggle over the same pushed-aside state, so it
        // is excluded like a wallpaper click.
        if command != Command::WallpaperClick && command != Command::ShowDesktop {
            let back = service.update(cx, |service, _| service.bring_back());
            if !back.is_empty() {
                run_actions(back, cx);
                if let Command::MissionControl | Command::AppWindows = command {
                    // Mission Control reads the screen: let the windows land.
                    let service = service.clone();
                    cx.spawn(async move |cx: &mut AsyncApp| {
                        cx.background_executor()
                            .timer(Duration::from_millis(u64::from(WINDOW_MOVEMENT_MS)))
                            .await;
                        cx.update(|cx| handle_command(&service, command, cx));
                    })
                    .detach();
                    return;
                }
            }
        }
        match command {
            Command::MissionControl => toggle(service, Mode::MissionControl, cx),
            Command::AppWindows => toggle(service, Mode::AppWindows, cx),
            Command::ShowDesktop => {
                close_overlay(service, cx);
                show_desktop(service, cx);
            }
            Command::WallpaperClick => {
                // The overlay covers the wallpaper; a click there is its own.
                if service.read(cx).overlay.is_none() {
                    let actions = service.update(cx, |service, _| service.wallpaper_click());
                    run_actions(actions, cx);
                }
            }
            Command::NextSpace | Command::PreviousSpace => {
                close_overlay(service, cx);
                let snapshot = service.read(cx).compositor.snapshot();
                if let Some(workspace) =
                    model::neighbour_space(&snapshot, command == Command::NextSpace)
                {
                    run_actions(vec![Action::FocusWorkspace { workspace }], cx);
                }
            }
            Command::Cancel => {
                close_overlay(service, cx);
            }
            Command::Fill
            | Command::Centre
            | Command::TileLeft
            | Command::TileRight
            | Command::TileTop
            | Command::TileBottom
            | Command::RestoreSize => {
                close_overlay(service, cx);
                tile_focused_window(service, command, cx);
            }
            Command::Minimize => {
                close_overlay(service, cx);
                minimize_focused_window(service, cx);
            }
        }
    }

    /// ⌘M for every application (packaging/rmac-session/shell.kdl): the
    /// focused window goes through the same record, picture and park path
    /// as an rmac app's yellow traffic light, so its Dock tile and restore
    /// behave identically.
    fn minimize_focused_window(service: &Entity<Service>, cx: &mut App) {
        let snapshot = service.read(cx).compositor.snapshot();
        let Some(window) = snapshot.focus.window else {
            return;
        };
        cx.spawn(async move |_cx: &mut AsyncApp| {
            if let Err(error) = rmac_compositor_niri::minimize_window_in(snapshot, window).await {
                eprintln!("could not minimize the focused window: {error}");
            }
        })
        .detach();
    }

    /// Window ▸ Move & Resize, WIN-01: Fill, Centre, a half, or Return to
    /// Previous Size, applied to whichever window niri reports focused.
    /// rmac windows are always floating, so every command is a floating
    /// frame (docs/decisions/0014-mission-control.md).
    fn tile_focused_window(service: &Entity<Service>, command: Command, cx: &mut App) {
        let snapshot = service.read(cx).compositor.snapshot();
        let Some(window) = snapshot.focus.window else {
            return;
        };
        if command == Command::RestoreSize {
            let action = service.update(cx, |service, _| service.restore_window_size(window));
            if let Some(action) = action {
                run_actions(vec![action], cx);
            }
            return;
        }
        service.update(cx, |service, _| {
            service.record_tile_history(&snapshot, window)
        });
        let action = match command {
            Command::Fill => Action::FillWindow { window },
            Command::Centre => Action::CenterWindow { window },
            Command::TileLeft => Action::TileWindow {
                window,
                region: TileRegion::Left,
            },
            Command::TileRight => Action::TileWindow {
                window,
                region: TileRegion::Right,
            },
            Command::TileTop => Action::TileWindow {
                window,
                region: TileRegion::Top,
            },
            Command::TileBottom => Action::TileWindow {
                window,
                region: TileRegion::Bottom,
            },
            other => unreachable!("tile_focused_window called with {other:?}"),
        };
        run_actions(vec![action], cx);
    }

    fn toggle(service: &Entity<Service>, mode: Mode, cx: &mut App) {
        if close_overlay(service, cx) || service.read(cx).opening {
            return;
        }
        let snapshot = service.read(cx).compositor.snapshot();
        let Some(scene) = model::scene(&snapshot, mode) else {
            return;
        };
        if mode == Mode::AppWindows && scene.windows.is_empty() {
            return;
        }
        service.update(cx, |service, _| service.opening = true);
        let service = service.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            // Read the screen before the overlay covers it.
            let captured_scene = scene.clone();
            let captured = cx
                .background_executor()
                .spawn(async move { capture::capture_scene(&captured_scene) })
                .await;
            cx.update(|cx| {
                service.update(cx, |service, _| service.opening = false);
                show_overlay(&service, mode, scene, captured, cx);
            });
        })
        .detach();
    }

    fn show_overlay(
        service: &Entity<Service>,
        mode: Mode,
        scene: Scene,
        captured: Captured,
        cx: &mut App,
    ) {
        if service.read(cx).overlay.is_some() {
            return;
        }
        let snapshot = service.read(cx).compositor.snapshot();
        // Windows nothing covers right now: a fresh screenshot crop each.
        let fresh: HashMap<WindowId, Arc<RenderImage>> = captured
            .windows
            .into_iter()
            .filter_map(|(id, pixels)| render_image(pixels).map(|image| (id, image)))
            .collect();
        let desktop = captured.desktop.and_then(render_image);
        let spaces = model::spaces(&snapshot, &scene.output);
        let (space_pictures, items, pictures) = service.update(cx, |service, _| {
            if let (Some(image), Some(space)) = (
                desktop,
                spaces
                    .iter()
                    .find(|space| space.workspace == scene.workspace),
            ) {
                service.space_pictures.insert(
                    scene.workspace,
                    SpacePicture {
                        windows: space.windows.clone(),
                        image,
                    },
                );
            }
            let space_pictures: HashMap<WorkspaceId, Arc<RenderImage>> = spaces
                .iter()
                .filter_map(|space| {
                    let picture = service.space_pictures.get(&space.workspace)?;
                    (picture.windows == space.windows)
                        .then(|| (space.workspace, picture.image.clone()))
                })
                .collect();
            let items: HashMap<String, Item> = scene
                .windows
                .iter()
                .filter_map(|window| window.app_id.clone())
                .map(|app_id| {
                    let item = service.item(&app_id);
                    (app_id, item)
                })
                .collect();
            // Cache every freshly captured picture, then look every window
            // of the Space up in the cache. A window nothing covers right
            // now still shows the last picture Mission Control has of it
            // (MC-01): the Mac keeps a live layer for every window, and a
            // screenshot-based capture cannot, but the picture it took the
            // last time this window was on top beats the icon card.
            service.window_pictures.extend(fresh);
            let pictures: HashMap<WindowId, Arc<RenderImage>> = scene
                .windows
                .iter()
                .filter_map(|window| {
                    service
                        .window_pictures
                        .get(&window.id)
                        .map(|image| (window.id, image.clone()))
                })
                .collect();
            (space_pictures, items, pictures)
        });

        let displays = rmac_shell_layer::output_surfaces::newest_displays(cx);
        let display = displays
            .get(&rmac_shell_layer::stable_output_uuid(&scene.output))
            .cloned()
            .or_else(|| displays.values().next().cloned());
        let weak = service.downgrade();
        let options = WindowOptions {
            titlebar: None,
            focus: true,
            show: true,
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(0.0), px(0.0)),
                size: Size::new(px(scene.width), px(scene.height)),
            })),
            display_id: display.as_ref().map(|display| display.id()),
            app_id: Some("dev.rmac.MissionControl".to_owned()),
            window_background: WindowBackgroundAppearance::Transparent,
            kind: WindowKind::LayerShell(LayerShellOptions {
                namespace: NAMESPACE.to_owned(),
                layer: Layer::Overlay,
                anchor: Anchor::TOP | Anchor::RIGHT | Anchor::BOTTOM | Anchor::LEFT,
                // Cover the menu bar too, as macOS does.
                exclusive_zone: Some(px(-1.0)),
                keyboard_interactivity: KeyboardInteractivity::Exclusive,
                ..Default::default()
            }),
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            ..Default::default()
        };
        match cx.open_window(options, move |window, cx| {
            cx.new(|cx| {
                Overlay::new(
                    weak,
                    mode,
                    scene,
                    snapshot,
                    pictures,
                    space_pictures,
                    items,
                    window,
                    cx,
                )
            })
        }) {
            Ok(handle) => service.update(cx, |service, _| service.overlay = Some(handle)),
            Err(error) => eprintln!("could not open Mission Control: {error}"),
        }
    }

    /// F11: push every window to the screen edges, Mac-style; F11 again
    /// brings them back (`Service::show_desktop`).
    fn show_desktop(service: &Entity<Service>, cx: &mut App) {
        let actions = service.update(cx, |service, _| service.show_desktop());
        run_actions(actions, cx);
    }

    fn run_service() -> Result<(), String> {
        let listener = crate::ipc::Listener::bind()
            .map_err(|error| format!("could not bind the Mission Control socket: {error}"))?;
        let (command_tx, command_rx) = async_channel::bounded(32);
        std::thread::Builder::new()
            .name("rmac-mission-control-ipc".into())
            .spawn(move || loop {
                match listener.receive() {
                    Ok(command) => {
                        if command_tx.send_blocking(command).is_err() {
                            return;
                        }
                    }
                    Err(error) => {
                        eprintln!("Mission Control endpoint stopped: {error}");
                        std::process::exit(1);
                    }
                }
            })
            .map_err(|error| format!("could not start the Mission Control endpoint: {error}"))?;

        let app = application().with_quit_mode(QuitMode::Explicit);
        app.run(move |cx: &mut App| {
            tokens::install_appearance_watch(cx);
            let service = cx.new(Service::new);

            let (compositor_tx, compositor_rx) = async_channel::bounded(64);
            cx.background_executor()
                .spawn(async move {
                    if let Err(error) = rmac_compositor_niri::watch(compositor_tx).await {
                        eprintln!("Mission Control compositor watcher stopped: {error}");
                    }
                })
                .detach();
            let watched = service.clone();
            cx.spawn(async move |cx: &mut AsyncApp| {
                while let Ok(event) = compositor_rx.recv().await {
                    cx.update(|cx| {
                        let reconnected = matches!(event, rmac_compositor::Event::Snapshot { .. });
                        let (topology, action) =
                            watched.update(cx, |service, _| service.apply(event));
                        if let Some(action) = action {
                            run_actions(vec![action], cx);
                        }
                        let follow =
                            watched.update(cx, |service, _| service.follow_reveal(reconnected));
                        run_actions(follow, cx);
                        if topology {
                            reconcile_corners(&watched, cx);
                        }
                        let overlay = watched.read(cx).overlay;
                        if let Some(handle) = overlay {
                            let snapshot = watched.read(cx).compositor.snapshot();
                            let _ =
                                handle.update(cx, |view, _, cx| view.refresh_spaces(snapshot, cx));
                        }
                    });
                }
            })
            .detach();

            // Hot corners follow Desktop & Dock through the shell settings.
            // The document is small, so it is read on this thread.
            let configured = service.clone();
            cx.spawn(async move |cx: &mut AsyncApp| {
                let store = match rmac_shell_settings::ShellSettingsStore::from_environment() {
                    Ok(store) => store,
                    Err(error) => {
                        eprintln!("Mission Control cannot read shell settings: {error}");
                        return;
                    }
                };
                let watcher = store.watch();
                loop {
                    match store.load() {
                        Ok(snapshot) => {
                            let corners = snapshot.settings.hot_corners;
                            let click_to_reveal = snapshot.settings.click_wallpaper_to_reveal;
                            cx.update(|cx| {
                                configured.update(cx, |service, _| {
                                    service.corners = corners;
                                    service.click_to_reveal = click_to_reveal;
                                });
                                reconcile_corners(&configured, cx);
                            });
                        }
                        Err(error) => {
                            eprintln!("Mission Control cannot read shell settings: {error}")
                        }
                    }
                    let Ok(watcher) = watcher.as_ref() else {
                        eprintln!("Mission Control cannot watch shell settings");
                        return;
                    };
                    loop {
                        match watcher.recv().await {
                            Ok(rmac_shell_settings::StoreEvent::Changed) => break,
                            Ok(rmac_shell_settings::StoreEvent::WatchError(error)) => {
                                eprintln!("Mission Control shell settings watch: {error}")
                            }
                            Err(_) => return,
                        }
                    }
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
                    .ok_or_else(|| format!("unknown Mission Control command: {command}"))?;
                crate::ipc::send(command)
                    .map_err(|error| format!("Mission Control is not running: {error}"))
            }
            _ => Err(
                "usage: mission-control --service | mission-control | app-windows | show-desktop | wallpaper-click | next-space | previous-space | cancel | minimize"
                    .to_owned(),
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
    let _ = capture::capture_scene;
    eprintln!(
        "Mission Control requires Linux and: cargo run --features wayland --bin mission-control -- --service"
    );
    std::process::exit(2);
}
