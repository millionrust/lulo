//! rmac screenshots (docs/decisions/0010-screenshots.md).
//!
//! `screenshot --service` is the resident session component. niri's ⇧⌘3,
//! ⇧⌘4 and ⇧⌘5 binds run `screenshot screen|selection|toolbar` (and the ⌃
//! clipboard variants), which forward one word to the service and exit. The
//! service draws the ⇧⌘4 crosshair readout and ⇧⌘5 toolbar on a full-output
//! overlay, captures through `grim`, plays the shutter cue, and shows the
//! floating thumbnail before saving "Screenshot … at 1.36.39 PM.png".

mod capture;
#[cfg(unix)]
mod ipc;
mod model;

#[cfg(all(target_os = "linux", feature = "wayland"))]
mod linux_wayland {
    use std::borrow::Cow;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use gpui::{
        canvas, div, fill, img, layer_shell::*, point, prelude::*, px, rgba, svg, AnyElement, App,
        AssetSource, AsyncApp, Bounds, BoxShadow, Context, CursorStyle, Entity, FocusHandle,
        FontWeight, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
        Pixels, QuitMode, Role, SharedString, Size, WeakEntity, Window, WindowBackgroundAppearance,
        WindowBounds, WindowHandle, WindowKind, WindowOptions,
    };
    use gpui_platform::application;
    use uuid::Uuid;

    use crate::capture::{self, Region};
    use crate::model::{
        self, Command, Destination, Handle, MenuEntry, MenuItem, Point, Rect, Settings, Target,
    };

    const NAMESPACE: &str = "rmac-screenshot";
    const THUMBNAIL_NAMESPACE: &str = "rmac-screenshot-thumbnail";
    const FRAME: Duration = Duration::from_millis(16);
    // App-menu material (design-lab/menus.html), drawn without the blur.
    const MENU_TINT: u32 = 0x1F1F_26F7;
    const MENU_EDGE: u32 = 0xFFFF_FF4D;
    const MENU_HAIRLINE: u32 = 0x0000_00D9;
    const MENU_TEXT: u32 = 0xFFFF_FFD9;
    const MENU_HEADER_TEXT: u32 = 0xFFFF_FF40;
    const MENU_SEPARATOR_COLOR: u32 = 0xFFFF_FF24;
    const MENU_HIGHLIGHT: u32 = 0x387B_F6FF;

    struct Assets;

    impl AssetSource for Assets {
        fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
            let bytes: Option<&'static [u8]> = match path {
                "screenshot/capture-screen.svg" => Some(include_bytes!(
                    "../../../assets/screenshot/capture-screen.svg"
                )),
                "screenshot/capture-window.svg" => Some(include_bytes!(
                    "../../../assets/screenshot/capture-window.svg"
                )),
                "screenshot/capture-selection.svg" => Some(include_bytes!(
                    "../../../assets/screenshot/capture-selection.svg"
                )),
                "screenshot/close-x.svg" => {
                    Some(include_bytes!("../../../assets/screenshot/close-x.svg"))
                }
                "menu/checkmark.svg" => Some(include_bytes!("../../../assets/menu/checkmark.svg")),
                "menu/chevron-down.svg" => {
                    Some(include_bytes!("../../../assets/menu/chevron-down.svg"))
                }
                _ => None,
            };
            Ok(bytes.map(Cow::Borrowed))
        }

        fn list(&self, _path: &str) -> gpui::Result<Vec<SharedString>> {
            Ok(Vec::new())
        }
    }

    /// The output being captured, in niri's logical layout.
    #[derive(Clone)]
    struct OutputInfo {
        name: String,
        uuid: Uuid,
        x: f64,
        y: f64,
        width: f32,
        height: f32,
    }

    impl OutputInfo {
        fn region(&self, rect: &Rect) -> Region {
            Region::Area {
                x: self.x + f64::from(rect.x),
                y: self.y + f64::from(rect.y),
                width: f64::from(rect.width),
                height: f64::from(rect.height),
            }
        }
    }

    /// One capture on its way to the file or the clipboard.
    struct Request {
        region: Region,
        output: Uuid,
        width: f32,
        height: f32,
        to_clipboard: bool,
        delay: Duration,
    }

    /// A captured file waiting behind the floating thumbnail.
    struct Pending {
        staged: PathBuf,
        directory: PathBuf,
        name: String,
    }

    /// The focused output and its visible windows (output-local, topmost
    /// first: floating over tiled, the focused window first).
    fn scene(snapshot: &rmac_compositor::Snapshot) -> Option<(OutputInfo, Vec<Rect>)> {
        let output_id = snapshot
            .focus
            .output
            .clone()
            .or_else(|| {
                snapshot
                    .workspaces
                    .iter()
                    .find(|workspace| workspace.focused)
                    .and_then(|workspace| workspace.output.clone())
            })
            .or_else(|| {
                snapshot
                    .outputs
                    .iter()
                    .find(|output| output.enabled())
                    .map(|output| output.id.clone())
            })?;
        let output = snapshot
            .outputs
            .iter()
            .find(|output| output.id == output_id)?;
        let logical = output.logical.as_ref()?;
        let info = OutputInfo {
            name: output.id.0.clone(),
            uuid: rmac_shell_layer::stable_output_uuid(&output.id),
            x: logical.position.x,
            y: logical.position.y,
            width: logical.size.width as f32,
            height: logical.size.height as f32,
        };
        let active = snapshot
            .workspaces
            .iter()
            .filter(|workspace| workspace.active && workspace.output.as_ref() == Some(&output.id))
            .map(|workspace| workspace.id)
            .collect::<Vec<_>>();
        let mut windows = snapshot
            .windows
            .iter()
            .filter(|window| window.workspace.is_some_and(|id| active.contains(&id)))
            .filter(|window| !rmac_compositor::window_is_parked(snapshot, window))
            .collect::<Vec<_>>();
        windows.sort_by_key(|window| (!window.floating, !window.focused));
        let rects = windows
            .iter()
            .filter_map(|window| rmac_compositor::window_logical_rect(snapshot, window.id))
            .map(|rect| {
                Rect::new(
                    (rect.x - info.x) as f32,
                    (rect.y - info.y) as f32,
                    rect.width as f32,
                    rect.height as f32,
                )
                .clipped(info.width, info.height)
            })
            .filter(|rect| !rect.is_empty())
            .collect();
        Some((info, rects))
    }

    struct Service {
        settings: Settings,
        clipboard: bool,
        overlay: Option<WindowHandle<Overlay>>,
        thumbnail: Option<WindowHandle<Thumbnail>>,
        busy: bool,
        sequence: u64,
    }

    impl Service {
        fn new(_cx: &mut Context<Self>) -> Self {
            Self {
                settings: Settings::load(),
                clipboard: capture::clipboard_available(),
                overlay: None,
                thumbnail: None,
                busy: false,
                sequence: 0,
            }
        }

        fn store(&mut self, settings: Settings) {
            if settings != self.settings {
                if let Err(error) = settings.save() {
                    eprintln!("could not save screenshot options: {error}");
                }
                self.settings = settings;
            }
        }

        fn start_capture(&mut self, request: Request, cx: &mut Context<Self>) {
            if self.busy {
                return;
            }
            self.busy = true;
            self.sequence = self.sequence.wrapping_add(1);
            let staged = capture::staging_path(self.sequence);
            let show_pointer = self.settings.show_pointer;
            cx.spawn(async move |this, cx: &mut AsyncApp| {
                cx.background_executor()
                    .timer(Duration::from_millis(model::SETTLE_MS) + request.delay)
                    .await;
                let region = request.region.clone();
                let result = match staged {
                    Ok(path) => {
                        cx.background_executor()
                            .spawn(async move {
                                capture::grab(&region, show_pointer, &path).map(|()| path)
                            })
                            .await
                    }
                    Err(error) => Err(error),
                };
                let _ = this.update(cx, |service, cx| {
                    service.busy = false;
                    match result {
                        Ok(path) => service.captured(path, request, cx),
                        Err(error) => eprintln!("screenshot failed: {error}"),
                    }
                });
            })
            .detach();
        }

        fn captured(&mut self, staged: PathBuf, request: Request, cx: &mut Context<Self>) {
            let _ = rmac_sound::play(rmac_sound::Cue::Screenshot);
            if request.to_clipboard {
                cx.background_executor()
                    .spawn(async move {
                        if let Err(error) = capture::copy_to_clipboard(&staged) {
                            eprintln!("could not copy the screenshot: {error}");
                        }
                        capture::discard(&staged);
                    })
                    .detach();
                return;
            }
            let destination = match self.settings.destination {
                Destination::Clipboard => Destination::Desktop,
                destination => destination,
            };
            let Some(directory) = model::destination_directory(destination) else {
                eprintln!("no folder to save the screenshot in");
                capture::discard(&staged);
                return;
            };
            let pending = Pending {
                staged,
                directory,
                name: model::file_name(&chrono::Local::now().naive_local()),
            };
            // A newer capture saves the previous thumbnail's file at once.
            self.dismiss_thumbnail(cx);
            if !self.settings.show_thumbnail {
                self.deliver(pending, false, cx);
                return;
            }
            let (width, height) = model::thumbnail_size(request.width, request.height);
            let displays = rmac_shell_layer::output_surfaces::newest_displays(cx);
            let display = displays
                .get(&request.output)
                .cloned()
                .or_else(|| displays.values().next().cloned());
            let image = pending.staged.clone();
            let weak = cx.entity().downgrade();
            let options = WindowOptions {
                titlebar: None,
                focus: false,
                show: true,
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(0.0), px(0.0)),
                    size: Size::new(px(width + model::THUMB_RIGHT), px(height)),
                })),
                display_id: display.as_ref().map(|display| display.id()),
                app_id: Some("dev.rmac.Screenshot".to_owned()),
                window_background: WindowBackgroundAppearance::Transparent,
                kind: WindowKind::LayerShell(LayerShellOptions {
                    namespace: THUMBNAIL_NAMESPACE.to_owned(),
                    layer: Layer::Overlay,
                    anchor: Anchor::BOTTOM | Anchor::RIGHT,
                    exclusive_zone: Some(px(-1.0)),
                    margin: Some((px(0.0), px(0.0), px(model::THUMB_BOTTOM), px(0.0))),
                    keyboard_interactivity: KeyboardInteractivity::None,
                    ..Default::default()
                }),
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                ..Default::default()
            };
            let mut pending = Some(pending);
            let opened = cx.open_window(options, |window, cx| {
                cx.new(|cx| {
                    Thumbnail::new(weak, pending.take(), image, (width, height), window, cx)
                })
            });
            match opened {
                Ok(handle) => self.thumbnail = Some(handle),
                Err(error) => {
                    eprintln!("could not show the screenshot thumbnail: {error}");
                    if let Some(pending) = pending {
                        self.deliver(pending, false, cx);
                    }
                }
            }
        }

        fn dismiss_thumbnail(&mut self, cx: &mut Context<Self>) {
            let Some(handle) = self.thumbnail.take() else {
                return;
            };
            if let Ok(Some(pending)) = handle.update(cx, |view, window, _| view.take(window)) {
                self.deliver(pending, false, cx);
            }
        }

        fn deliver(&mut self, pending: Pending, open: bool, cx: &mut Context<Self>) {
            cx.background_executor()
                .spawn(async move {
                    match capture::deliver(&pending.staged, &pending.directory, &pending.name) {
                        Ok(path) if open => capture::open(&path),
                        Ok(_) => {}
                        Err(error) => {
                            eprintln!("could not save the screenshot: {error}");
                            capture::discard(&pending.staged);
                        }
                    }
                })
                .detach();
        }
    }

    // ---------------------------------------------------------------------
    // ⇧⌘4 / ⇧⌘5 overlay

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Mode {
        /// ⇧⌘4: drag a selection or (Space) pick a window.
        Interactive,
        /// ⇧⌘5: the toolbar with an adjustable selection.
        Toolbar,
    }

    #[derive(Clone, Copy)]
    enum Gesture {
        Idle,
        Drawing {
            start: Point,
        },
        Moving {
            origin: Rect,
            grab: Point,
        },
        Resizing {
            origin: Rect,
            handle: Handle,
            grab: Point,
        },
    }

    enum Area {
        Screen,
        Rect(Rect),
    }

    struct Overlay {
        service: WeakEntity<Service>,
        mode: Mode,
        target: Target,
        to_clipboard: bool,
        output: OutputInfo,
        windows: Vec<Rect>,
        pointer: Option<Point>,
        gesture: Gesture,
        selection: Rect,
        hovered_window: Option<usize>,
        menu_open: bool,
        menu_hover: Option<usize>,
        settings: Settings,
        clipboard_available: bool,
        focus: FocusHandle,
        was_active: bool,
        closing: bool,
    }

    impl Overlay {
        #[allow(clippy::too_many_arguments)]
        fn new(
            service: WeakEntity<Service>,
            mode: Mode,
            to_clipboard: bool,
            output: OutputInfo,
            windows: Vec<Rect>,
            settings: Settings,
            clipboard_available: bool,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) -> Self {
            let focus = cx.focus_handle();
            focus.focus(window, cx);
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.was_active = true;
                } else if this.was_active {
                    // Another surface took the keyboard: cancel, like Esc.
                    this.cancel(window, cx);
                }
            })
            .detach();
            let target = match mode {
                Mode::Interactive => Target::Selection,
                Mode::Toolbar => settings.target,
            };
            let remembered = settings
                .last_selection
                .filter(|_| settings.remember_selection)
                .map(|rect| rect.clipped(output.width, output.height))
                .filter(|rect| !rect.is_empty());
            let selection =
                remembered.unwrap_or_else(|| model::default_selection(output.width, output.height));
            Self {
                service,
                mode,
                target,
                to_clipboard,
                output,
                windows,
                pointer: None,
                gesture: Gesture::Idle,
                selection,
                hovered_window: None,
                menu_open: false,
                menu_hover: None,
                settings,
                clipboard_available,
                focus,
                was_active: false,
                closing: false,
            }
        }

        fn local(position: gpui::Point<Pixels>) -> Point {
            Point::new(f32::from(position.x), f32::from(position.y))
        }

        fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
            if self.closing {
                return false;
            }
            self.closing = true;
            let _ = self.service.update(cx, |service, _| service.overlay = None);
            window.remove_window();
            true
        }

        fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
            if self.close(window, cx) && self.mode == Mode::Toolbar {
                // The toolbar remembers the chosen mode even when cancelled.
                let mut settings = self.settings.clone();
                settings.target = self.target;
                let _ = self
                    .service
                    .update(cx, |service, _| service.store(settings));
            }
        }

        fn capture(&mut self, area: Area, window: &mut Window, cx: &mut Context<Self>) {
            let mut settings = self.settings.clone();
            let mut delay = Duration::ZERO;
            if self.mode == Mode::Toolbar {
                settings.target = self.target;
                if let (Area::Rect(rect), Target::Selection) = (&area, self.target) {
                    settings.last_selection = Some(*rect);
                }
                delay = Duration::from_secs(u64::from(settings.timer_seconds));
            }
            let (region, width, height) = match area {
                Area::Screen => (
                    Region::Output(self.output.name.clone()),
                    self.output.width,
                    self.output.height,
                ),
                Area::Rect(rect) => (self.output.region(&rect), rect.width, rect.height),
            };
            let request = Request {
                region,
                output: self.output.uuid,
                width,
                height,
                to_clipboard: self.to_clipboard
                    || (settings.destination == Destination::Clipboard && self.clipboard_available),
                delay,
            };
            if !self.close(window, cx) {
                return;
            }
            let _ = self.service.update(cx, |service, cx| {
                service.store(settings);
                service.start_capture(request, cx);
            });
        }

        /// The Capture button and Return.
        fn capture_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
            match self.target {
                Target::Screen => self.capture(Area::Screen, window, cx),
                Target::Selection => self.capture(Area::Rect(self.selection), window, cx),
                Target::Window => {
                    if let Some(rect) = self.hovered_window.map(|index| self.windows[index]) {
                        self.capture(Area::Rect(rect), window, cx);
                    }
                }
            }
        }

        fn set_target(&mut self, target: Target, cx: &mut Context<Self>) {
            self.target = target;
            self.hovered_window = self
                .pointer
                .and_then(|pointer| model::window_at(&self.windows, pointer));
            cx.notify();
        }

        fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
            match event.keystroke.key.as_str() {
                "escape" if self.menu_open => {
                    self.menu_open = false;
                    cx.notify();
                }
                "escape" => self.cancel(window, cx),
                "space"
                    if self.mode == Mode::Interactive
                        && !matches!(self.gesture, Gesture::Drawing { .. }) =>
                {
                    let target = if self.target == Target::Window {
                        Target::Selection
                    } else {
                        Target::Window
                    };
                    self.set_target(target, cx);
                }
                "enter" if self.mode == Mode::Toolbar && !self.menu_open => {
                    self.capture_current(window, cx)
                }
                _ => {}
            }
            cx.stop_propagation();
        }

        fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
            let point = Self::local(event.position);
            let (width, height) = (self.output.width, self.output.height);
            self.pointer = Some(point);
            match self.gesture {
                Gesture::Moving { origin, grab } => {
                    self.selection =
                        origin.moved_within(point.x - grab.x, point.y - grab.y, width, height);
                }
                Gesture::Resizing {
                    origin,
                    handle,
                    grab,
                } => {
                    let rect = model::resized(
                        &origin,
                        handle,
                        point.x - grab.x,
                        point.y - grab.y,
                        width,
                        height,
                    );
                    if !rect.is_empty() {
                        self.selection = rect;
                    }
                }
                Gesture::Drawing { start } if self.mode == Mode::Toolbar => {
                    let rect = Rect::from_corners(start, point).clipped(width, height);
                    if !rect.is_empty() {
                        self.selection = rect;
                    }
                }
                _ => {}
            }
            if self.target == Target::Window {
                self.hovered_window = model::window_at(&self.windows, point);
            }
            cx.notify();
        }

        fn mouse_down(
            &mut self,
            event: &MouseDownEvent,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) {
            if self.menu_open {
                self.menu_open = false;
                cx.notify();
                return;
            }
            let point = Self::local(event.position);
            self.pointer = Some(point);
            match (self.mode, self.target) {
                (_, Target::Window) => {
                    if let Some(index) = model::window_at(&self.windows, point) {
                        let rect = self.windows[index];
                        self.capture(Area::Rect(rect), window, cx);
                        return;
                    }
                }
                (Mode::Toolbar, Target::Screen) => {
                    self.capture(Area::Screen, window, cx);
                    return;
                }
                (Mode::Interactive, _) => self.gesture = Gesture::Drawing { start: point },
                (Mode::Toolbar, Target::Selection) => {
                    self.gesture = if let Some(handle) = model::handle_at(&self.selection, point) {
                        Gesture::Resizing {
                            origin: self.selection,
                            handle,
                            grab: point,
                        }
                    } else if self.selection.contains(point) {
                        Gesture::Moving {
                            origin: self.selection,
                            grab: point,
                        }
                    } else {
                        Gesture::Drawing { start: point }
                    };
                }
            }
            cx.notify();
        }

        fn mouse_up(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
            let point = Self::local(event.position);
            let gesture = std::mem::replace(&mut self.gesture, Gesture::Idle);
            if let (Mode::Interactive, Gesture::Drawing { start }) = (self.mode, gesture) {
                let rect =
                    Rect::from_corners(start, point).clipped(self.output.width, self.output.height);
                if !rect.is_empty() {
                    self.capture(Area::Rect(rect), window, cx);
                    return;
                }
            }
            cx.notify();
        }

        fn choose(&mut self, item: MenuItem, cx: &mut Context<Self>) {
            item.apply(&mut self.settings);
            self.menu_open = false;
            let settings = self.settings.clone();
            let _ = self
                .service
                .update(cx, |service, _| service.store(settings));
            cx.notify();
        }

        fn cursor(&self) -> CursorStyle {
            match (self.mode, self.target) {
                (_, Target::Window) | (Mode::Toolbar, Target::Screen) => CursorStyle::Arrow,
                (Mode::Interactive, _) => CursorStyle::Crosshair,
                (Mode::Toolbar, Target::Selection) => {
                    if matches!(self.gesture, Gesture::Moving { .. }) {
                        return CursorStyle::ClosedHand;
                    }
                    let Some(pointer) = self.pointer else {
                        return CursorStyle::Crosshair;
                    };
                    match model::handle_at(&self.selection, pointer) {
                        Some(Handle::TopLeft | Handle::BottomRight) => {
                            CursorStyle::ResizeUpLeftDownRight
                        }
                        Some(Handle::TopRight | Handle::BottomLeft) => {
                            CursorStyle::ResizeUpRightDownLeft
                        }
                        Some(Handle::Left | Handle::Right) => CursorStyle::ResizeLeftRight,
                        Some(Handle::Top | Handle::Bottom) => CursorStyle::ResizeUpDown,
                        None if self.selection.contains(pointer) => CursorStyle::Arrow,
                        None => CursorStyle::Crosshair,
                    }
                }
            }
        }

        /// Two lines beside the crosshair: black digits over a white copy
        /// 0.5 right and 1 down.
        fn readout(&self) -> Option<AnyElement> {
            let pointer = self.pointer?;
            let start = match self.gesture {
                Gesture::Drawing { start } => Some(start),
                _ => None,
            };
            let (first, second) = model::readout(pointer, start);
            let lines = |color: u32| {
                div()
                    .flex()
                    .flex_col()
                    .whitespace_nowrap()
                    .font_family("Inter")
                    .text_size(px(model::READOUT_SIZE))
                    .line_height(px(model::READOUT_LINE))
                    .text_color(rgba(color))
                    .child(first.clone())
                    .child(second.clone())
            };
            Some(
                div()
                    .absolute()
                    .left(px(pointer.x + model::READOUT_DX))
                    .top(px(pointer.y + model::READOUT_DY))
                    .child(
                        div()
                            .absolute()
                            .left(px(model::READOUT_SHADOW_DX))
                            .top(px(model::READOUT_SHADOW_DY))
                            .child(lines(0xFFFF_FFFF)),
                    )
                    .child(div().relative().child(lines(0x0000_00FF)))
                    .into_any_element(),
            )
        }

        fn window_highlight(&self) -> Option<AnyElement> {
            let rect = self.hovered_window.map(|index| self.windows[index])?;
            Some(
                rect_div(&rect)
                    .bg(rgba(model::WINDOW_HIGHLIGHT))
                    .into_any_element(),
            )
        }

        /// ⇧⌘5's dim, dashed frame and handles, painted in one canvas.
        fn selection_frame(&self) -> AnyElement {
            let rect = self.selection;
            let (width, height) = (self.output.width, self.output.height);
            canvas(
                |_, _, _| (),
                move |_, (), window, _| {
                    let quad = |x: f32, y: f32, w: f32, h: f32, color: u32| {
                        fill(
                            Bounds {
                                origin: point(px(x), px(y)),
                                size: Size::new(px(w.max(0.0)), px(h.max(0.0))),
                            },
                            rgba(color),
                        )
                    };
                    let dim = model::DIM;
                    window.paint_quad(quad(0.0, 0.0, width, rect.y, dim));
                    window.paint_quad(quad(0.0, rect.bottom(), width, height - rect.bottom(), dim));
                    window.paint_quad(quad(0.0, rect.y, rect.x, rect.height, dim));
                    window.paint_quad(quad(
                        rect.right(),
                        rect.y,
                        width - rect.right(),
                        rect.height,
                        dim,
                    ));
                    // 1 pt edge: black, then 4 pt white dashes every 8.
                    let right = rect.right() - 1.0;
                    let bottom = rect.bottom() - 1.0;
                    for (x, y, w, h) in [
                        (rect.x, rect.y, rect.width, 1.0),
                        (rect.x, bottom, rect.width, 1.0),
                        (rect.x, rect.y, 1.0, rect.height),
                        (right, rect.y, 1.0, rect.height),
                    ] {
                        window.paint_quad(quad(x, y, w, h, 0x0000_00FF));
                    }
                    let mut offset = 0.0;
                    while offset < rect.width {
                        let dash = model::DASH.min(rect.width - offset);
                        window.paint_quad(quad(rect.x + offset, rect.y, dash, 1.0, 0xFFFF_FFFF));
                        window.paint_quad(quad(rect.x + offset, bottom, dash, 1.0, 0xFFFF_FFFF));
                        offset += 2.0 * model::DASH;
                    }
                    let mut offset = 0.0;
                    while offset < rect.height {
                        let dash = model::DASH.min(rect.height - offset);
                        window.paint_quad(quad(rect.x, rect.y + offset, 1.0, dash, 0xFFFF_FFFF));
                        window.paint_quad(quad(right, rect.y + offset, 1.0, dash, 0xFFFF_FFFF));
                        offset += 2.0 * model::DASH;
                    }
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .into_any_element()
        }

        fn handles(&self) -> Vec<AnyElement> {
            Handle::ALL
                .into_iter()
                .map(|handle| {
                    let center = handle.center(&self.selection);
                    let half = model::HANDLE_SIZE / 2.0;
                    div()
                        .absolute()
                        .left(px(center.x - half))
                        .top(px(center.y - half))
                        .size(px(model::HANDLE_SIZE))
                        .rounded_full()
                        .bg(rgba(model::HANDLE_FILL))
                        .border(px(1.0))
                        .border_color(rgba(0xFFFF_FFFF))
                        .into_any_element()
                })
                .collect()
        }

        fn toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
            let origin = model::toolbar_origin(self.output.width, self.output.height);
            let middle = model::TOOLBAR_HEIGHT / 2.0;
            let mut bar = div()
                .id("screenshot-toolbar")
                .role(Role::Toolbar)
                .aria_label("Screenshot")
                .absolute()
                .left(px(origin.x))
                .top(px(origin.y))
                .w(px(model::TOOLBAR_WIDTH))
                .h(px(model::TOOLBAR_HEIGHT))
                .rounded(px(model::TOOLBAR_RADIUS))
                .bg(rgba(model::TOOLBAR_TINT))
                .shadow(vec![
                    BoxShadow {
                        color: rgba(0x0000_0099).into(),
                        offset: point(px(0.0), px(0.0)),
                        blur_radius: px(0.0),
                        spread_radius: px(0.5),
                        inset: false,
                    },
                    BoxShadow {
                        color: rgba(0x0000_0059).into(),
                        offset: point(px(0.0), px(8.0)),
                        blur_radius: px(24.0),
                        spread_radius: px(0.0),
                        inset: false,
                    },
                ])
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .cursor(CursorStyle::Arrow);

            // Close.
            bar = bar.child(
                div()
                    .id("screenshot-close")
                    .role(Role::Button)
                    .aria_label("Close")
                    .absolute()
                    .left(px(model::CLOSE_CENTER_X - model::CLOSE_SIZE / 2.0))
                    .top(px(middle - model::CLOSE_SIZE / 2.0))
                    .size(px(model::CLOSE_SIZE))
                    .rounded_full()
                    .bg(rgba(model::TOOLBAR_GLYPH))
                    .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx)))
                    .child(
                        svg()
                            .size(px(model::CLOSE_SIZE))
                            .path("screenshot/close-x.svg")
                            .text_color(rgba(model::TOOLBAR_TINT | 0xFF)),
                    ),
            );

            for (index, target) in Target::ALL.into_iter().enumerate() {
                let center = model::TARGET_CENTERS[index];
                let selected = target == self.target;
                bar = bar.child(
                    div()
                        .id(("screenshot-target", index))
                        .role(Role::RadioButton)
                        .aria_label(target.label())
                        .aria_selected(selected)
                        .absolute()
                        .left(px(center - model::PLATE_WIDTH / 2.0))
                        .top(px(middle - model::PLATE_HEIGHT / 2.0))
                        .w(px(model::PLATE_WIDTH))
                        .h(px(model::PLATE_HEIGHT))
                        .rounded(px(model::PLATE_RADIUS))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(selected, |plate| plate.bg(rgba(model::TOOLBAR_PLATE)))
                        .on_click(cx.listener(move |this, _, _, cx| this.set_target(target, cx)))
                        .child(
                            svg()
                                .w(px(model::GLYPH_WIDTH))
                                .h(px(model::GLYPH_HEIGHT))
                                .path(target.glyph())
                                .text_color(rgba(if selected {
                                    model::TOOLBAR_GLYPH_SELECTED
                                } else {
                                    model::TOOLBAR_GLYPH
                                })),
                        ),
                );
            }

            bar = bar
                .child(
                    div()
                        .absolute()
                        .left(px(model::SEPARATOR_X))
                        .top(px(model::SEPARATOR_TOP))
                        .w(px(1.0))
                        .h(px(model::SEPARATOR_HEIGHT))
                        .bg(rgba(model::TOOLBAR_PLATE)),
                )
                .child(
                    div()
                        .id("screenshot-options")
                        .role(Role::Button)
                        .aria_label("Options")
                        .aria_expanded(self.menu_open)
                        .absolute()
                        .left(px(model::OPTIONS_X))
                        .top_0()
                        .h(px(model::TOOLBAR_HEIGHT))
                        .flex()
                        .items_center()
                        .whitespace_nowrap()
                        .font_family("Inter")
                        .text_size(px(model::OPTIONS_TEXT_SIZE))
                        .text_color(rgba(model::TOOLBAR_GLYPH))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.menu_open = !this.menu_open;
                            this.menu_hover = None;
                            cx.notify();
                        }))
                        .child("Options")
                        .child(
                            svg()
                                .ml(px(model::CHEVRON_GAP))
                                .size(px(model::CHEVRON_BOX))
                                .path("menu/chevron-down.svg")
                                .text_color(rgba(model::TOOLBAR_GLYPH)),
                        ),
                )
                .child(
                    div()
                        .id("screenshot-capture")
                        .role(Role::Button)
                        .aria_label("Capture")
                        .absolute()
                        .right(px(model::CAPTURE_INSET))
                        .top(px(model::CAPTURE_INSET))
                        .w(px(model::CAPTURE_WIDTH))
                        .h(px(model::CAPTURE_HEIGHT))
                        .rounded(px(model::CAPTURE_RADIUS))
                        .bg(rgba(model::CAPTURE_FILL))
                        .flex()
                        .items_center()
                        .justify_center()
                        .font_family("Inter")
                        .text_size(px(13.0))
                        .text_color(rgba(0xFFFF_FFFF))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.capture_current(window, cx)),
                        )
                        .child("Capture"),
                );
            bar.into_any_element()
        }

        fn menu(&self, cx: &mut Context<Self>) -> AnyElement {
            let entries = model::menu_entries(self.clipboard_available);
            let toolbar = model::toolbar_origin(self.output.width, self.output.height);
            let origin = model::menu_origin(toolbar, &entries);
            let mut panel = div()
                .id("screenshot-options-menu")
                .role(Role::Menu)
                .aria_label("Options")
                .absolute()
                .left(px(origin.x))
                .top(px(origin.y))
                .w(px(model::MENU_WIDTH))
                .py(px(model::MENU_PADDING))
                .rounded(px(model::MENU_RADIUS))
                .bg(rgba(MENU_TINT))
                .border(px(1.0))
                .border_color(rgba(MENU_EDGE))
                .shadow(vec![
                    BoxShadow {
                        color: rgba(MENU_HAIRLINE).into(),
                        offset: point(px(0.0), px(0.0)),
                        blur_radius: px(0.0),
                        spread_radius: px(0.5),
                        inset: false,
                    },
                    BoxShadow {
                        color: rgba(0x0000_0059).into(),
                        offset: point(px(0.0), px(10.0)),
                        blur_radius: px(32.0),
                        spread_radius: px(0.0),
                        inset: false,
                    },
                ])
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .cursor(CursorStyle::Arrow)
                .font_family("Inter")
                .flex()
                .flex_col();
            for (index, entry) in entries.into_iter().enumerate() {
                panel = panel.child(match entry {
                    MenuEntry::Header(title) => div()
                        .h(px(model::MENU_HEADER))
                        .pl(px(model::MENU_HEADER_X))
                        .flex()
                        .items_center()
                        .text_size(px(12.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgba(MENU_HEADER_TEXT))
                        .child(title)
                        .into_any_element(),
                    MenuEntry::Separator => div()
                        .h(px(model::MENU_SEPARATOR))
                        .px(px(model::MENU_SEPARATOR_INSET))
                        .pt(px((model::MENU_SEPARATOR - 1.0) / 2.0))
                        .child(div().h(px(1.0)).bg(rgba(MENU_SEPARATOR_COLOR)))
                        .into_any_element(),
                    MenuEntry::Item(item) => {
                        let highlighted = self.menu_hover == Some(index);
                        let checked = item.checked(&self.settings);
                        let text = if highlighted { 0xFFFF_FFFF } else { MENU_TEXT };
                        let inset = model::MENU_PADDING;
                        div()
                            .id(("screenshot-option", index))
                            .role(Role::MenuItemCheckBox)
                            .aria_label(item.label())
                            .aria_toggled(if checked {
                                gpui::Toggled::True
                            } else {
                                gpui::Toggled::False
                            })
                            .relative()
                            .h(px(model::MENU_ROW))
                            .mx(px(inset))
                            .pl(px(model::MENU_TEXT_X - inset))
                            .flex()
                            .items_center()
                            .whitespace_nowrap()
                            .rounded(px(5.0))
                            .text_size(px(13.0))
                            .text_color(rgba(text))
                            .when(highlighted, |row| row.bg(rgba(MENU_HIGHLIGHT)))
                            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                                if *hovered {
                                    this.menu_hover = Some(index);
                                } else if this.menu_hover == Some(index) {
                                    this.menu_hover = None;
                                }
                                cx.notify();
                            }))
                            .on_click(cx.listener(move |this, _, _, cx| this.choose(item, cx)))
                            .when(checked, |row| {
                                row.child(
                                    svg()
                                        .absolute()
                                        .left(px(model::MENU_CHECK_CENTER_X - inset - 8.0))
                                        .top(px((model::MENU_ROW - 16.0) / 2.0))
                                        .size(px(16.0))
                                        .path("menu/checkmark.svg")
                                        .text_color(rgba(text)),
                                )
                            })
                            .child(item.label())
                            .into_any_element()
                    }
                });
            }
            panel.into_any_element()
        }
    }

    fn rect_div(rect: &Rect) -> gpui::Div {
        div()
            .absolute()
            .left(px(rect.x))
            .top(px(rect.y))
            .w(px(rect.width))
            .h(px(rect.height))
    }

    impl Render for Overlay {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let mut root = div()
                .id("screenshot-overlay")
                .track_focus(&self.focus)
                .relative()
                .size_full()
                .role(Role::Pane)
                .aria_label("Screenshot")
                .cursor(self.cursor())
                .on_key_down(cx.listener(Self::key_down))
                .on_mouse_move(cx.listener(Self::mouse_move))
                .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
                .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up));
            if self.target == Target::Window {
                root = root.children(self.window_highlight());
            }
            match self.mode {
                Mode::Interactive => {
                    if self.target == Target::Selection {
                        if let (Gesture::Drawing { start }, Some(pointer)) =
                            (self.gesture, self.pointer)
                        {
                            let rect = Rect::from_corners(start, pointer);
                            root = root.child(
                                rect_div(&rect)
                                    .bg(rgba(model::DRAG_FILL))
                                    .border(px(1.0))
                                    .border_color(rgba(model::DRAG_EDGE)),
                            );
                        }
                        root = root.children(self.readout());
                    }
                }
                Mode::Toolbar => {
                    if self.target == Target::Selection {
                        root = root.child(self.selection_frame()).children(self.handles());
                    }
                    root = root.child(self.toolbar(cx));
                    if self.menu_open {
                        root = root.child(self.menu(cx));
                    }
                }
            }
            root
        }
    }

    fn open_overlay(
        service: &Entity<Service>,
        mode: Mode,
        to_clipboard: bool,
        output: OutputInfo,
        windows: Vec<Rect>,
        cx: &mut App,
    ) {
        if service.read(cx).overlay.is_some() {
            return;
        }
        let displays = rmac_shell_layer::output_surfaces::newest_displays(cx);
        let display = displays
            .get(&output.uuid)
            .cloned()
            .or_else(|| displays.values().next().cloned());
        let (settings, clipboard) = {
            let state = service.read(cx);
            (state.settings.clone(), state.clipboard)
        };
        let weak = service.downgrade();
        let options = WindowOptions {
            titlebar: None,
            focus: true,
            show: true,
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(0.0), px(0.0)),
                size: Size::new(px(output.width), px(output.height)),
            })),
            display_id: display.as_ref().map(|display| display.id()),
            app_id: Some("dev.rmac.Screenshot".to_owned()),
            window_background: WindowBackgroundAppearance::Transparent,
            kind: WindowKind::LayerShell(LayerShellOptions {
                namespace: NAMESPACE.to_owned(),
                layer: Layer::Overlay,
                anchor: Anchor::TOP | Anchor::RIGHT | Anchor::BOTTOM | Anchor::LEFT,
                // Cover the menu bar and the Dock too.
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
                    to_clipboard,
                    output,
                    windows,
                    settings,
                    clipboard,
                    window,
                    cx,
                )
            })
        }) {
            Ok(handle) => service.update(cx, |service, _| service.overlay = Some(handle)),
            Err(error) => eprintln!("could not open the screenshot overlay: {error}"),
        }
    }

    // ---------------------------------------------------------------------
    // Floating thumbnail

    struct Thumbnail {
        service: WeakEntity<Service>,
        pending: Option<Pending>,
        image: PathBuf,
        size: (f32, f32),
        shown_at: Instant,
        /// When the slide out began and from which offset.
        leaving: Option<(Instant, f32)>,
        /// Pointer x at the press, while the thumbnail is held.
        press: Option<f32>,
        drag: f32,
        open_after: bool,
    }

    impl Thumbnail {
        fn new(
            service: WeakEntity<Service>,
            pending: Option<Pending>,
            image: PathBuf,
            size: (f32, f32),
            window: &mut Window,
            cx: &mut Context<Self>,
        ) -> Self {
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
            Self {
                service,
                pending,
                image,
                size,
                shown_at: Instant::now(),
                leaving: None,
                press: None,
                drag: 0.0,
                open_after: false,
            }
        }

        fn hidden_offset(&self) -> f32 {
            self.size.0 + model::THUMB_RIGHT
        }

        fn progress(since: Instant) -> f32 {
            since.elapsed().as_secs_f32()
                / Duration::from_millis(model::THUMB_SLIDE_MS).as_secs_f32()
        }

        fn offset(&self) -> f32 {
            let hidden = self.hidden_offset();
            if let Some((since, from)) = self.leaving {
                return from + (hidden - from) * model::ease_out(Self::progress(since));
            }
            let entering = hidden * (1.0 - model::ease_out(Self::progress(self.shown_at)));
            entering.max(0.0) + self.drag
        }

        /// Advance the animation; false once the thumbnail has gone.
        fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
            if self.pending.is_none() {
                return false;
            }
            match self.leaving {
                Some((since, _)) if Self::progress(since) >= 1.0 => {
                    self.finish(window, cx);
                    return false;
                }
                None if self.press.is_none()
                    && self.shown_at.elapsed() >= Duration::from_millis(model::THUMB_HOLD_MS) =>
                {
                    self.leave();
                }
                _ => {}
            }
            cx.notify();
            true
        }

        fn leave(&mut self) {
            if self.leaving.is_none() {
                self.leaving = Some((Instant::now(), self.offset()));
            }
        }

        /// Close without saving; the caller delivers the file.
        fn take(&mut self, window: &mut Window) -> Option<Pending> {
            let pending = self.pending.take();
            window.remove_window();
            pending
        }

        fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
            let open = self.open_after;
            if let Some(pending) = self.take(window) {
                let _ = self.service.update(cx, |service, cx| {
                    service.thumbnail = None;
                    service.deliver(pending, open, cx);
                });
            }
        }
    }

    impl Render for Thumbnail {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let (width, height) = self.size;
            let offset = self.offset();
            window.set_input_region(Some(&[Bounds {
                origin: point(px(offset.max(0.0)), px(0.0)),
                size: Size::new(px(width), px(height)),
            }]));
            div().size_full().relative().child(
                div()
                    .id("screenshot-thumbnail")
                    .role(Role::Button)
                    .aria_label("Screenshot")
                    .absolute()
                    .left(px(offset))
                    .top_0()
                    .w(px(width))
                    .h(px(height))
                    .rounded(px(model::THUMB_RADIUS))
                    .bg(rgba(0x0000_00FF))
                    .p(px(model::THUMB_BORDER))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, _| {
                            if this.leaving.is_none() {
                                this.press = Some(f32::from(event.position.x));
                            }
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        if let Some(start) = this.press {
                            this.drag = (f32::from(event.position.x) - start).max(0.0);
                            cx.notify();
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseUpEvent, window, cx| {
                            let Some(_) = this.press.take() else {
                                return;
                            };
                            if this.drag >= model::THUMB_SWIPE {
                                // Swiped right: save now.
                                this.leave();
                            } else if this.drag < 3.0 {
                                // Clicked: save and open it.
                                this.open_after = true;
                                this.finish(window, cx);
                                return;
                            } else {
                                this.drag = 0.0;
                            }
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .relative()
                            .size_full()
                            .child(img(self.image.clone()).size_full())
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .left_0()
                                    .size_full()
                                    .border(px(0.5))
                                    .border_color(rgba(0xFFFF_FFFF)),
                            ),
                    ),
            )
        }
    }

    // ---------------------------------------------------------------------

    fn handle_command(service: &Entity<Service>, command: Command, cx: &mut App) {
        if command == Command::Cancel {
            if let Some(handle) = service.read(cx).overlay {
                let _ = handle.update(cx, |view, window, cx| view.cancel(window, cx));
            }
            return;
        }
        let (busy, open) = {
            let state = service.read(cx);
            (state.busy, state.overlay.is_some())
        };
        if busy || open {
            return;
        }
        let service = service.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let snapshot = match rmac_compositor_niri::snapshot().await {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    eprintln!("screenshot could not read the compositor: {error}");
                    return;
                }
            };
            let Some((output, windows)) = scene(&snapshot) else {
                eprintln!("screenshot found no output");
                return;
            };
            cx.update(|cx| match command {
                Command::Screen | Command::ScreenToClipboard => {
                    let clipboard_default = {
                        let state = service.read(cx);
                        state.settings.destination == Destination::Clipboard && state.clipboard
                    };
                    let request = Request {
                        region: Region::Output(output.name.clone()),
                        output: output.uuid,
                        width: output.width,
                        height: output.height,
                        to_clipboard: command == Command::ScreenToClipboard || clipboard_default,
                        delay: Duration::ZERO,
                    };
                    service.update(cx, |service, cx| service.start_capture(request, cx));
                }
                Command::Selection | Command::SelectionToClipboard => open_overlay(
                    &service,
                    Mode::Interactive,
                    command == Command::SelectionToClipboard,
                    output,
                    windows,
                    cx,
                ),
                Command::Toolbar => {
                    open_overlay(&service, Mode::Toolbar, false, output, windows, cx)
                }
                Command::Cancel => {}
            });
        })
        .detach();
    }

    fn run_service() -> Result<(), String> {
        let listener = crate::ipc::Listener::bind()
            .map_err(|error| format!("could not bind the screenshot socket: {error}"))?;
        let (command_tx, command_rx) = async_channel::bounded(32);
        std::thread::Builder::new()
            .name("rmac-screenshot-ipc".into())
            .spawn(move || loop {
                match listener.receive() {
                    Ok(command) => {
                        if command_tx.send_blocking(command).is_err() {
                            return;
                        }
                    }
                    Err(error) => {
                        eprintln!("screenshot endpoint stopped: {error}");
                        std::process::exit(1);
                    }
                }
            })
            .map_err(|error| format!("could not start the screenshot endpoint: {error}"))?;

        let app = application()
            .with_assets(Assets)
            .with_quit_mode(QuitMode::Explicit);
        app.run(move |cx: &mut App| {
            rmac_shell_ui::tokens::install_appearance_watch(cx);
            let service = cx.new(Service::new);
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
                    .ok_or_else(|| format!("unknown screenshot command: {command}"))?;
                crate::ipc::send(command)
                    .map_err(|error| format!("the screenshot service is not running: {error}"))
            }
            _ => Err("usage: screenshot --service | screen | screen-to-clipboard | selection | selection-to-clipboard | toolbar | cancel".to_owned()),
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
    // Keep the platform-independent modules linked (and warning-free) here.
    let _ = model::Command::parse;
    let _ = capture::grim_arguments;
    eprintln!(
        "Screenshots require Linux and: cargo run --features wayland --bin screenshot -- --service"
    );
    std::process::exit(2);
}
