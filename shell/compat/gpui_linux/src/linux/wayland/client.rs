use std::{
    cell::{RefCell, RefMut},
    collections::HashSet,
    hash::Hash,
    io::Write,
    os::fd::{AsRawFd, BorrowedFd},
    path::PathBuf,
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

use ashpd::WindowIdentifier;
use calloop::{
    EventLoop, LoopHandle, RegistrationToken,
    timer::{TimeoutAction, Timer},
};
use calloop_wayland_source::WaylandSource;
use collections::HashMap;
use filedescriptor::Pipe;
use gpui_util::ResultExt as _;
use http_client::Url;
use smallvec::SmallVec;
use wayland_backend::client::ObjectId;
use wayland_backend::protocol::WEnum;
use wayland_client::event_created_child;
use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_callback::{self, WlCallback};
use wayland_client::protocol::wl_data_device_manager::DndAction;
use wayland_client::protocol::wl_data_offer::WlDataOffer;
use wayland_client::protocol::wl_pointer::AxisSource;
use wayland_client::protocol::{
    wl_data_device, wl_data_device_manager, wl_data_offer, wl_data_source, wl_output, wl_region,
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, delegate_noop,
    protocol::{
        wl_buffer, wl_compositor, wl_keyboard, wl_pointer, wl_registry, wl_seat, wl_shm,
        wl_shm_pool, wl_surface, wl_touch,
    },
};
use wayland_protocols::wp::pointer_gestures::zv1::client::{
    zwp_pointer_gesture_pinch_v1, zwp_pointer_gestures_v1,
};
use wayland_protocols::wp::primary_selection::zv1::client::zwp_primary_selection_offer_v1::{
    self, ZwpPrimarySelectionOfferV1,
};
use wayland_protocols::wp::primary_selection::zv1::client::{
    zwp_primary_selection_device_manager_v1, zwp_primary_selection_device_v1,
    zwp_primary_selection_source_v1,
};
use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_v3::{
    ContentHint, ContentPurpose,
};
use wayland_protocols::wp::text_input::zv3::client::{
    zwp_text_input_manager_v3, zwp_text_input_v3,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols::xdg::activation::v1::client::{xdg_activation_token_v1, xdg_activation_v1};
use wayland_protocols::xdg::decoration::zv1::client::{
    zxdg_decoration_manager_v1, zxdg_toplevel_decoration_v1,
};
use wayland_protocols::xdg::shell::client::{
    xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base,
};
use wayland_protocols::xdg::system_bell::v1::client::xdg_system_bell_v1;
use wayland_protocols::{
    wp::cursor_shape::v1::client::{wp_cursor_shape_device_v1, wp_cursor_shape_manager_v1},
    xdg::dialog::v1::client::xdg_wm_dialog_v1::{self, XdgWmDialogV1},
};
use wayland_protocols::{
    wp::fractional_scale::v1::client::{wp_fractional_scale_manager_v1, wp_fractional_scale_v1},
    xdg::dialog::v1::client::xdg_dialog_v1::XdgDialogV1,
};
use wayland_protocols_plasma::blur::client::{org_kde_kwin_blur, org_kde_kwin_blur_manager};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};
use xkbcommon::xkb::ffi::XKB_KEYMAP_FORMAT_TEXT_V1;
use xkbcommon::xkb::{self, KEYMAP_COMPILE_NO_FLAGS, Keycode};

use super::{
    display::WaylandDisplay,
    window::{ImeInput, WaylandWindowStatePtr},
};

use crate::linux::{
    DOUBLE_CLICK_INTERVAL, LinuxClient, LinuxCommon, LinuxKeyboardLayout, PIPE_READ_TIMEOUT,
    SCROLL_LINES, capslock_from_xkb, cursor_style_to_icon_names, get_xkb_compose_state,
    is_within_click_distance, keystroke_from_xkb, keystroke_underlying_dead_key,
    modifiers_from_xkb, open_uri_internal, read_fd_with_timeout, reveal_path_internal,
    wayland::{
        clipboard::{Clipboard, DataOffer, FILE_LIST_MIME_TYPE, TEXT_MIME_TYPES},
        cursor::Cursor,
        serial::{SerialKind, SerialTracker},
        to_shape,
        window::WaylandWindow,
    },
    xdg_desktop_portal::{Event as XDPEvent, XDPEventSource},
};
use gpui::{
    AnyWindowHandle, Bounds, Capslock, CursorStyle, DevicePixels, DisplayId, FileDropEvent,
    ForegroundExecutor, KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, ModifiersChangedEvent,
    MouseButton, MouseDownEvent, MouseExitEvent, MouseMoveEvent, MouseUpEvent, NavigationDirection,
    Pixels, PlatformDisplay, PlatformInput, PlatformKeyboardLayout, PlatformWindow, Point,
    ScrollDelta, ScrollWheelEvent, SharedString, Size, TouchPhase, WindowButtonLayout, WindowKind,
    WindowParams, point, profiler, px, size,
};
use gpui_wgpu::{CompositorGpuHint, GpuContext};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_dmabuf_feedback_v1, zwp_linux_dmabuf_v1,
};

/// Used to convert evdev scancode to xkb scancode
const MIN_KEYCODE: u32 = 8;

const UNKNOWN_KEYBOARD_LAYOUT_NAME: SharedString = SharedString::new_static("unknown");
const XDG_ACTIVATION_TOKEN_ENV_VAR: &str = "XDG_ACTIVATION_TOKEN";

fn take_startup_activation_token_from_environment() -> Option<String> {
    let startup_activation_token = std::env::var(XDG_ACTIVATION_TOKEN_ENV_VAR)
        .ok()
        .filter(|token| !token.is_empty());
    // The token must be removed from the environment so it isn't inherited by child
    // processes we spawn, per the xdg-activation spec: https://wayland.app/protocols/xdg-activation-v1
    // SAFETY: This runs during Wayland platform initialization before GPUI starts
    // concurrent environment access or spawning child processes.
    unsafe { std::env::remove_var(XDG_ACTIVATION_TOKEN_ENV_VAR) };
    startup_activation_token
}

#[derive(Clone)]
pub struct Globals {
    pub qh: QueueHandle<WaylandClientStatePtr>,
    pub activation: Option<xdg_activation_v1::XdgActivationV1>,
    pub compositor: wl_compositor::WlCompositor,
    pub cursor_shape_manager: Option<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1>,
    pub data_device_manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    pub primary_selection_manager:
        Option<zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1>,
    pub wm_base: xdg_wm_base::XdgWmBase,
    pub shm: wl_shm::WlShm,
    pub seat: wl_seat::WlSeat,
    pub viewporter: Option<wp_viewporter::WpViewporter>,
    pub fractional_scale_manager:
        Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    pub decoration_manager: Option<zxdg_decoration_manager_v1::ZxdgDecorationManagerV1>,
    pub layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    pub blur_manager: Option<org_kde_kwin_blur_manager::OrgKdeKwinBlurManager>,
    pub text_input_manager: Option<zwp_text_input_manager_v3::ZwpTextInputManagerV3>,
    pub gesture_manager: Option<zwp_pointer_gestures_v1::ZwpPointerGesturesV1>,
    pub dialog: Option<xdg_wm_dialog_v1::XdgWmDialogV1>,
    pub system_bell: Option<xdg_system_bell_v1::XdgSystemBellV1>,
    pub executor: ForegroundExecutor,
}

impl Globals {
    fn new(
        globals: GlobalList,
        executor: ForegroundExecutor,
        qh: QueueHandle<WaylandClientStatePtr>,
        seat: wl_seat::WlSeat,
    ) -> Self {
        let dialog_v = XdgWmDialogV1::interface().version;
        Globals {
            activation: globals.bind(&qh, 1..=1, ()).ok(),
            compositor: globals
                .bind(
                    &qh,
                    wl_surface::REQ_SET_BUFFER_SCALE_SINCE
                        ..=wl_surface::EVT_PREFERRED_BUFFER_SCALE_SINCE,
                    (),
                )
                .unwrap(),
            cursor_shape_manager: globals.bind(&qh, 1..=1, ()).ok(),
            data_device_manager: globals
                .bind(
                    &qh,
                    WL_DATA_DEVICE_MANAGER_VERSION..=WL_DATA_DEVICE_MANAGER_VERSION,
                    (),
                )
                .ok(),
            primary_selection_manager: globals.bind(&qh, 1..=1, ()).ok(),
            shm: globals.bind(&qh, 1..=1, ()).unwrap(),
            seat,
            wm_base: globals.bind(&qh, 1..=5, ()).unwrap(),
            viewporter: globals.bind(&qh, 1..=1, ()).ok(),
            fractional_scale_manager: globals.bind(&qh, 1..=1, ()).ok(),
            decoration_manager: globals.bind(&qh, 1..=1, ()).ok(),
            layer_shell: globals.bind(&qh, 1..=5, ()).ok(),
            blur_manager: globals.bind(&qh, 1..=1, ()).ok(),
            text_input_manager: globals.bind(&qh, 1..=1, ()).ok(),
            gesture_manager: globals.bind(&qh, 1..=3, ()).ok(),
            dialog: globals.bind(&qh, dialog_v..=dialog_v, ()).ok(),
            system_bell: globals.bind(&qh, 1..=1, ()).ok(),
            executor,
            qh,
        }
    }
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Hash)]
pub struct InProgressOutput {
    name: Option<String>,
    scale: Option<i32>,
    position: Option<Point<DevicePixels>>,
    size: Option<Size<DevicePixels>>,
    subpixel: Option<wl_output::Subpixel>,
}

impl InProgressOutput {
    fn complete(&self) -> Option<Output> {
        if let Some((position, size)) = self.position.zip(self.size) {
            let scale = self.scale.unwrap_or(1);
            Some(Output {
                name: self.name.clone(),
                scale,
                bounds: Bounds::new(position, size),
                subpixel: self.subpixel,
            })
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct Output {
    pub name: Option<String>,
    pub scale: i32,
    pub bounds: Bounds<DevicePixels>,
    pub subpixel: Option<wl_output::Subpixel>,
}

pub(crate) struct WaylandClientState {
    connection: Connection,
    serial_tracker: SerialTracker,
    globals: Globals,
    pub gpu_context: GpuContext,
    pub compositor_gpu: Option<CompositorGpuHint>,
    wl_seat: wl_seat::WlSeat, // TODO: Multi seat support
    wl_pointer: Option<wl_pointer::WlPointer>,
    wl_touch: Option<wl_touch::WlTouch>,
    touch_ids: HashSet<i32>,
    touch_suppressed: bool,
    touch: Option<TouchContact>,
    touch_generation: u64,
    touch_hold_token: Option<RegistrationToken>,
    pinch_gesture: Option<zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1>,
    pinch_scale: f32,
    wl_keyboard: Option<wl_keyboard::WlKeyboard>,
    cursor_shape_device: Option<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1>,
    data_device: Option<wl_data_device::WlDataDevice>,
    primary_selection: Option<zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1>,
    text_input: Option<zwp_text_input_v3::ZwpTextInputV3>,
    pre_edit_text: Option<String>,
    ime_pre_edit: Option<String>,
    composing: bool,
    // Surface to Window mapping
    windows: HashMap<ObjectId, WaylandWindowStatePtr>,
    // Output to scale mapping
    outputs: HashMap<ObjectId, Output>,
    in_progress_outputs: HashMap<ObjectId, InProgressOutput>,
    wl_outputs: HashMap<ObjectId, wl_output::WlOutput>,
    keyboard_layout: LinuxKeyboardLayout,
    keymap_state: Option<xkb::State>,
    compose_state: Option<xkb::compose::State>,
    drag: DragState,
    staged_file_drag: Option<StagedFileDrag>,
    file_drag_source: Option<FileDragSource>,
    press_serial: Option<u32>,
    click: ClickState,
    repeat: KeyRepeat,
    pub modifiers: Modifiers,
    pub capslock: Capslock,
    axis_source: AxisSource,
    pub mouse_location: Option<Point<Pixels>>,
    continuous_scroll_delta: Option<Point<Pixels>>,
    discrete_scroll_delta: Option<Point<f32>>,
    vertical_modifier: f32,
    horizontal_modifier: f32,
    scroll_event_received: bool,
    /// rmac: touchpad momentum. Finger scroll velocity in logical px/ms, the
    /// time of the last finger scroll, a pending axis stop, and a generation
    /// that cancels a running glide.
    scroll_velocity: Point<f32>,
    last_finger_scroll: Option<Instant>,
    axis_stop_pending: bool,
    momentum_generation: u64,
    enter_token: Option<()>,
    button_pressed: Option<MouseButton>,
    pending_window_move: Option<PendingWindowMove>,
    mouse_focused_window: Option<WaylandWindowStatePtr>,
    keyboard_focused_window: Option<WaylandWindowStatePtr>,
    pub(crate) loop_handle: LoopHandle<'static, WaylandClientStatePtr>,
    cursor_style: Option<CursorStyle>,
    cursor_hidden_window: Option<WaylandWindowStatePtr>,
    clipboard: Clipboard,
    data_offers: Vec<DataOffer<WlDataOffer>>,
    primary_data_offer: Option<DataOffer<ZwpPrimarySelectionOfferV1>>,
    cursor: Cursor,
    pending_activation: Option<PendingActivation>,
    startup_activation_token: Option<String>,
    event_loop: Option<EventLoop<'static, WaylandClientStatePtr>>,
    pub common: LinuxCommon,
    ime_enabled: Option<bool>,
}

pub struct DragState {
    data_offer: Option<wl_data_offer::WlDataOffer>,
    window: Option<WaylandWindowStatePtr>,
    position: Point<Pixels>,
    action: Option<DndAction>,
    paths_ready: bool,
    drop_pending: bool,
}

struct StagedFileDrag {
    window: WaylandWindowStatePtr,
    serial: u32,
    paths: Vec<PathBuf>,
}

struct FileDragSource {
    source: wl_data_source::WlDataSource,
    uri_list: Vec<u8>,
    gnome_files: Vec<u8>,
    window: WaylandWindowStatePtr,
    position: Point<Pixels>,
}

thread_local! {
    static FILE_DRAG_CLIENT: RefCell<Weak<RefCell<WaylandClientState>>> = RefCell::default();
}

/// Stage an item drag while its button or touch contact is held. The normal
/// GPUI drag continues inside the window; crossing its edge starts Wayland DnD.
pub fn stage_external_file_drag(paths: Vec<PathBuf>) -> bool {
    FILE_DRAG_CLIENT.with(|slot| {
        let Some(client) = slot.borrow().upgrade() else {
            return false;
        };
        let mut state = client.borrow_mut();
        if state.file_drag_source.is_some() {
            return true;
        }
        let press = state
            .touch
            .as_ref()
            .filter(|touch| touch.gesture == TouchGesture::PointerDrag)
            .map(|touch| (touch.window.clone(), touch.serial, touch.position))
            .or_else(|| {
                (state.button_pressed == Some(MouseButton::Left)).then_some(())?;
                Some((
                    state.mouse_focused_window.clone()?,
                    state.press_serial?,
                    state.mouse_location?,
                ))
            });
        let Some((window, serial, position)) = press else {
            return false;
        };
        if paths.is_empty() {
            return false;
        }
        state.staged_file_drag = Some(StagedFileDrag {
            window,
            serial,
            paths,
        });
        state.try_start_staged_file_drag(position, false);
        state.file_drag_source.is_some()
    })
}

/// Begin Wayland DnD while a surface still owns the pointer or touch grab.
/// Desktop surfaces cover the whole output, so they choose the handoff when
/// the pointer approaches another window rather than waiting for an edge.
pub fn begin_external_file_drag(paths: Vec<PathBuf>) -> bool {
    if stage_external_file_drag(paths) {
        return true;
    }
    FILE_DRAG_CLIENT.with(|slot| {
        let Some(client) = slot.borrow().upgrade() else {
            return false;
        };
        let mut state = client.borrow_mut();
        let position = state
            .touch
            .as_ref()
            .map(|touch| touch.position)
            .or(state.mouse_location)
            .unwrap_or_default();
        state.try_start_staged_file_drag(position, true);
        state.file_drag_source.is_some()
    })
}

pub fn external_file_drag_active() -> bool {
    FILE_DRAG_CLIENT.with(|slot| {
        slot.borrow()
            .upgrade()
            .is_some_and(|client| client.borrow().file_drag_source.is_some())
    })
}

/// The compositor's selected action for the current external file drop.
/// Unknown actions copy, which avoids removing files offered by older clients.
pub fn file_drop_should_copy() -> bool {
    FILE_DRAG_CLIENT.with(|slot| {
        slot.borrow()
            .upgrade()
            .is_none_or(|client| client.borrow().drag.action != Some(DndAction::Move))
    })
}

fn file_drag_payload(paths: &[PathBuf], copy: bool) -> Option<(Vec<u8>, Vec<u8>)> {
    let urls: Vec<_> = paths
        .iter()
        .filter_map(|path| Url::from_file_path(path).ok())
        .map(|url| url.to_string())
        .collect();
    if urls.is_empty() {
        return None;
    }
    let uri_list = format!("{}\r\n", urls.join("\r\n")).into_bytes();
    let gnome_files = format!(
        "{}\n{}\n",
        if copy { "copy" } else { "cut" },
        urls.join("\n")
    )
    .into_bytes();
    Some((uri_list, gnome_files))
}

#[cfg(test)]
mod file_drag_tests {
    use super::*;

    #[test]
    fn uri_list_encodes_names_and_gnome_move_hint() {
        let (uris, gnome) = file_drag_payload(
            &[
                PathBuf::from("/tmp/one two.txt"),
                PathBuf::from("/tmp/café.txt"),
            ],
            false,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(uris).unwrap(),
            "file:///tmp/one%20two.txt\r\nfile:///tmp/caf%C3%A9.txt\r\n"
        );
        assert_eq!(
            String::from_utf8(gnome).unwrap(),
            "cut\nfile:///tmp/one%20two.txt\nfile:///tmp/caf%C3%A9.txt\n"
        );
        assert!(file_drag_payload(&[], false).is_none());
        let (_, gnome_copy) = file_drag_payload(&[PathBuf::from("/tmp/one")], true).unwrap();
        assert_eq!(gnome_copy, b"copy\nfile:///tmp/one\n");
    }
}

impl WaylandClientState {
    fn try_start_staged_file_drag(&mut self, position: Point<Pixels>, force: bool) {
        let Some(staged) = self.staged_file_drag.as_ref() else {
            return;
        };
        let bounds = staged.window.window_geometry();
        // Start while the press grab still belongs to the source surface.
        // Waiting until the pointer is outside can make the compositor reject
        // start_drag before it ever sends an offer to the target.
        const EDGE_INSET: f32 = 64.0;
        let x = position.x.as_f32();
        let y = position.y.as_f32();
        let left = bounds.origin.x.as_f32();
        let top = bounds.origin.y.as_f32();
        let right = left + bounds.size.width.as_f32();
        let bottom = top + bounds.size.height.as_f32();
        if !force
            && x >= left + EDGE_INSET
            && x < right - EDGE_INSET
            && y >= top + EDGE_INSET
            && y < bottom - EDGE_INSET
        {
            return;
        }
        let Some(manager) = self.globals.data_device_manager.as_ref() else {
            return;
        };
        let Some(device) = self.data_device.as_ref() else {
            return;
        };
        let copy = self.modifiers.alt;
        let Some((uri_list, gnome_files)) = file_drag_payload(&staged.paths, copy) else {
            return;
        };
        let source = manager.create_data_source(&self.globals.qh, ());
        source.offer(FILE_LIST_MIME_TYPE.to_owned());
        source.offer("x-special/gnome-copied-files".to_owned());
        source.set_actions(if copy {
            DndAction::Copy
        } else {
            DndAction::Copy | DndAction::Move | DndAction::Ask
        });
        device.start_drag(Some(&source), &staged.window.surface(), None, staged.serial);
        if let Err(error) = self.connection.flush() {
            log::warn!("Wayland file drag start flush failed: {error}");
        }
        self.file_drag_source = Some(FileDragSource {
            source,
            uri_list,
            gnome_files,
            window: staged.window.clone(),
            position,
        });
        self.staged_file_drag = None;
    }
}

pub struct ClickState {
    last_mouse_button: Option<MouseButton>,
    last_click: Instant,
    last_location: Point<Pixels>,
    current_count: usize,
}

/// Keep the press serial until movement proves this was a title-bar drag.
struct PendingWindowMove {
    window: WaylandWindowStatePtr,
    position: Point<Pixels>,
    serial: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TouchGesture {
    Pending,
    Scroll,
    PointerDrag,
    ContextMenu,
}

struct TouchContact {
    id: i32,
    window: WaylandWindowStatePtr,
    start: Point<Pixels>,
    position: Point<Pixels>,
    serial: u32,
    started: Instant,
    gesture: TouchGesture,
    drag_region: bool,
    files_window: bool,
    click_count: usize,
}

const TOUCH_MOVE_THRESHOLD: Pixels = px(8.0);
const TOUCH_HOLD: Duration = Duration::from_millis(500);
const FILE_DRAG_HOLD: Duration = Duration::from_millis(160);

/// A quick swipe scrolls. A title bar always drags; a Files item can be
/// dragged after a short deliberate hold. GPUI does not expose per-element
/// drag hit testing to the platform backend, so Files uses a window heuristic.
fn classify_touch_motion(
    start: Point<Pixels>,
    position: Point<Pixels>,
    elapsed: Duration,
    drag_region: bool,
    files_window: bool,
) -> TouchGesture {
    let delta = position - start;
    if delta.x.abs() <= TOUCH_MOVE_THRESHOLD && delta.y.abs() <= TOUCH_MOVE_THRESHOLD {
        TouchGesture::Pending
    } else if drag_region || (files_window && elapsed >= FILE_DRAG_HOLD) {
        TouchGesture::PointerDrag
    } else {
        TouchGesture::Scroll
    }
}

fn classify_touch_hold(gesture: TouchGesture, elapsed: Duration) -> TouchGesture {
    if gesture == TouchGesture::Pending && elapsed >= TOUCH_HOLD {
        TouchGesture::ContextMenu
    } else {
        gesture
    }
}

fn next_click_count(
    previous_button: Option<MouseButton>,
    previous_position: Point<Pixels>,
    previous_count: usize,
    elapsed: Duration,
    button: MouseButton,
    position: Point<Pixels>,
) -> usize {
    if elapsed < DOUBLE_CLICK_INTERVAL
        && previous_button == Some(button)
        && is_within_click_distance(previous_position, position)
    {
        previous_count + 1
    } else {
        1
    }
}

#[cfg(test)]
mod touch_tests {
    use super::*;

    #[test]
    fn gesture_threshold_and_drag_regions() {
        let start = point(px(10.0), px(20.0));
        let near = point(px(18.0), px(20.0));
        let moved = point(px(19.0), px(20.0));
        assert_eq!(
            classify_touch_motion(start, near, Duration::ZERO, false, false),
            TouchGesture::Pending
        );
        assert_eq!(
            classify_touch_motion(start, moved, Duration::ZERO, false, false),
            TouchGesture::Scroll
        );
        assert_eq!(
            classify_touch_motion(start, moved, Duration::ZERO, true, false),
            TouchGesture::PointerDrag
        );
        assert_eq!(
            classify_touch_motion(start, moved, Duration::from_millis(159), false, true),
            TouchGesture::Scroll
        );
        assert_eq!(
            classify_touch_motion(start, moved, FILE_DRAG_HOLD, false, true),
            TouchGesture::PointerDrag
        );
    }

    #[test]
    fn hold_only_opens_context_menu_without_movement() {
        assert_eq!(
            classify_touch_hold(TouchGesture::Pending, TOUCH_HOLD - Duration::from_millis(1)),
            TouchGesture::Pending
        );
        assert_eq!(
            classify_touch_hold(TouchGesture::Pending, TOUCH_HOLD),
            TouchGesture::ContextMenu
        );
        assert_eq!(
            classify_touch_hold(TouchGesture::Scroll, TOUCH_HOLD),
            TouchGesture::Scroll
        );
    }

    #[test]
    fn double_tap_uses_click_interval_and_distance() {
        let position = point(px(40.0), px(50.0));
        assert_eq!(
            next_click_count(
                Some(MouseButton::Left),
                position,
                1,
                DOUBLE_CLICK_INTERVAL - Duration::from_millis(1),
                MouseButton::Left,
                position
            ),
            2
        );
        assert_eq!(
            next_click_count(
                Some(MouseButton::Left),
                position,
                1,
                DOUBLE_CLICK_INTERVAL,
                MouseButton::Left,
                position
            ),
            1
        );
        assert_eq!(
            next_click_count(
                Some(MouseButton::Right),
                position,
                1,
                Duration::ZERO,
                MouseButton::Left,
                position
            ),
            1
        );
        assert_eq!(
            next_click_count(
                Some(MouseButton::Left),
                position,
                1,
                Duration::ZERO,
                MouseButton::Left,
                point(px(200.0), px(50.0))
            ),
            1
        );
    }
}

const WINDOW_MOVE_THRESHOLD: Pixels = px(4.0);

fn moved_past_window_drag_threshold(start: Point<Pixels>, current: Point<Pixels>) -> bool {
    let delta = current - start;
    delta.x.abs() > WINDOW_MOVE_THRESHOLD || delta.y.abs() > WINDOW_MOVE_THRESHOLD
}

#[cfg(test)]
mod window_move_tests {
    use super::*;

    #[test]
    fn clicks_with_small_pointer_jitter_remain_clicks() {
        let press = point(px(100.0), px(80.0));
        assert!(!moved_past_window_drag_threshold(
            press,
            point(px(104.0), px(76.0))
        ));
        assert!(moved_past_window_drag_threshold(
            press,
            point(px(104.1), px(80.0))
        ));
    }
}

pub(crate) struct KeyRepeat {
    characters_per_second: u32,
    delay: Duration,
    current_id: u64,
    current_keycode: Option<xkb::Keycode>,
}

pub(crate) enum PendingActivation {
    /// URI to open in the web browser.
    Uri(String),
    /// Path to open in the file explorer.
    Path(PathBuf),
    /// A window from ourselves to raise.
    Window(ObjectId),
}

impl WaylandClientState {
    fn cancel_touch_hold(&mut self) {
        if let Some(token) = self.touch_hold_token.take() {
            self.loop_handle.remove(token);
        }
    }

    fn count_click(&mut self, button: MouseButton, position: Point<Pixels>) -> usize {
        self.click.current_count = next_click_count(
            self.click.last_mouse_button,
            self.click.last_location,
            self.click.current_count,
            self.click.last_click.elapsed(),
            button,
            position,
        );
        self.click.last_click = Instant::now();
        self.click.last_mouse_button = Some(button);
        self.click.last_location = position;
        self.click.current_count
    }

    fn consume_startup_activation_token(&mut self, surface: &wl_surface::WlSurface) {
        let Some(startup_activation_token) = self.startup_activation_token.take() else {
            return;
        };
        let Some(activation) = self.globals.activation.as_ref() else {
            return;
        };
        activation.activate(startup_activation_token, surface);
    }
}

/// This struct is required to conform to Rust's orphan rules, so we can dispatch on the state but hand the
/// window to GPUI.
#[derive(Clone)]
pub struct WaylandClientStatePtr(Weak<RefCell<WaylandClientState>>);

impl WaylandClientStatePtr {
    pub fn get_client(&self) -> Rc<RefCell<WaylandClientState>> {
        self.0
            .upgrade()
            .expect("The pointer should always be valid when dispatching in wayland")
    }

    pub fn get_serial(&self, kind: SerialKind) -> u32 {
        self.0.upgrade().unwrap().borrow().serial_tracker.get(kind)
    }

    pub fn set_pending_activation(&self, window: ObjectId) {
        self.0.upgrade().unwrap().borrow_mut().pending_activation =
            Some(PendingActivation::Window(window));
    }

    pub fn enable_ime(&self) {
        let client = self.get_client();
        let mut state = client.borrow_mut();
        state.ime_enabled = Some(true);
        let Some(text_input) = state.text_input.take() else {
            return;
        };

        text_input.enable();
        text_input.set_content_type(ContentHint::None, ContentPurpose::Normal);
        if let Some(window) = state.keyboard_focused_window.clone() {
            drop(state);
            if let Some(area) = window.get_ime_area() {
                text_input.set_cursor_rectangle(
                    f32::from(area.origin.x) as i32,
                    f32::from(area.origin.y) as i32,
                    f32::from(area.size.width) as i32,
                    f32::from(area.size.height) as i32,
                );
            }
            state = client.borrow_mut();
        }
        text_input.commit();
        state.text_input = Some(text_input);
    }

    pub fn disable_ime(&self) {
        let client = self.get_client();
        let mut state = client.borrow_mut();
        state.ime_enabled = Some(false);
        state.composing = false;
        if let Some(text_input) = &state.text_input {
            text_input.disable();
            text_input.commit();
        }
    }

    pub fn ime_enabled(&self) -> Option<bool> {
        let client = self.get_client();
        client.borrow().ime_enabled
    }

    pub fn update_ime_position(&self, bounds: Bounds<Pixels>) {
        let client = self.get_client();
        let state = client.borrow_mut();
        if state.text_input.is_none() || state.pre_edit_text.is_some() {
            return;
        }

        let text_input = state.text_input.as_ref().unwrap();
        text_input.set_cursor_rectangle(
            bounds.origin.x.as_f32() as i32,
            bounds.origin.y.as_f32() as i32,
            bounds.size.width.as_f32() as i32,
            bounds.size.height.as_f32() as i32,
        );
        text_input.commit();
    }

    pub fn handle_keyboard_layout_change(&self) {
        let client = self.get_client();
        let mut state = client.borrow_mut();
        let changed = if let Some(keymap_state) = &state.keymap_state {
            let layout_idx = keymap_state.serialize_layout(xkbcommon::xkb::STATE_LAYOUT_EFFECTIVE);
            let keymap = keymap_state.get_keymap();
            let layout_name = keymap.layout_get_name(layout_idx);
            let changed = layout_name != state.keyboard_layout.name();
            if changed {
                state.keyboard_layout = LinuxKeyboardLayout::new(layout_name.to_string().into());
            }
            changed
        } else {
            let changed = UNKNOWN_KEYBOARD_LAYOUT_NAME != state.keyboard_layout.name();
            if changed {
                state.keyboard_layout = LinuxKeyboardLayout::new(UNKNOWN_KEYBOARD_LAYOUT_NAME);
            }
            changed
        };

        if changed && let Some(mut callback) = state.common.callbacks.keyboard_layout_change.take()
        {
            drop(state);
            callback();
            state = client.borrow_mut();
            state.common.callbacks.keyboard_layout_change = Some(callback);
        }
    }

    pub fn drop_window(&self, surface_id: &ObjectId) {
        let client = self.get_client();
        let mut state = client.borrow_mut();
        let closed_window = state.windows.remove(surface_id).unwrap();
        if state
            .touch
            .as_ref()
            .is_some_and(|contact| contact.window.ptr_eq(&closed_window))
        {
            state.touch = None;
            state.cancel_touch_hold();
            state.touch_generation = state.touch_generation.wrapping_add(1);
        }
        if let Some(window) = state.mouse_focused_window.take()
            && !window.ptr_eq(&closed_window)
        {
            state.mouse_focused_window = Some(window);
        }
        if let Some(window) = state.keyboard_focused_window.take()
            && !window.ptr_eq(&closed_window)
        {
            state.keyboard_focused_window = Some(window);
        }
        if let Some(window) = state.cursor_hidden_window.take()
            && !window.ptr_eq(&closed_window)
        {
            state.cursor_hidden_window = Some(window);
        }
    }
}

impl WaylandClientState {
    fn hide_cursor_until_mouse_moves(&mut self) {
        if self.cursor_hidden_window.is_some() {
            return;
        }
        let Some(focused_window) = self.mouse_focused_window.clone() else {
            // No surface to apply the hidden cursor to.
            return;
        };
        let Some(wl_pointer) = self.wl_pointer.clone() else {
            // Seat lost its pointer capability; nothing to hide.
            return;
        };
        let serial = self.serial_tracker.get(SerialKind::MouseEnter);
        wl_pointer.set_cursor(serial, None, 0, 0);
        self.cursor_hidden_window = Some(focused_window);
    }

    fn restore_cursor_after_hide(&mut self) {
        if self.cursor_hidden_window.take().is_none() {
            return;
        }
        let Some(style) = self.cursor_style else {
            return;
        };
        let serial = self.serial_tracker.get(SerialKind::MouseEnter);
        if let Some(cursor_shape_device) = &self.cursor_shape_device {
            cursor_shape_device.set_shape(serial, to_shape(style));
            return;
        }
        let Some(focused_window) = self.mouse_focused_window.clone() else {
            log::warn!(
                "wayland: no focused surface to restore cursor style {:?} after hide; cursor may stay invisible",
                style
            );
            return;
        };
        let Some(wl_pointer) = self.wl_pointer.clone() else {
            log::warn!(
                "wayland: no wl_pointer to restore cursor style {:?} after hide; cursor may stay invisible",
                style
            );
            return;
        };
        let scale = focused_window.primary_output_scale();
        self.cursor.set_icon(
            &wl_pointer,
            serial,
            cursor_style_to_icon_names(style),
            scale,
        );
    }
}

#[derive(Clone)]
pub struct WaylandClient(Rc<RefCell<WaylandClientState>>);

impl Drop for WaylandClient {
    fn drop(&mut self) {
        let mut state = self.0.borrow_mut();
        state.windows.clear();

        if let Some(wl_pointer) = &state.wl_pointer {
            wl_pointer.release();
        }
        if let Some(cursor_shape_device) = &state.cursor_shape_device {
            cursor_shape_device.destroy();
        }
        if let Some(data_device) = &state.data_device {
            data_device.release();
        }
        if let Some(text_input) = &state.text_input {
            text_input.destroy();
        }
    }
}

const WL_DATA_DEVICE_MANAGER_VERSION: u32 = 3;

fn wl_seat_version(version: u32) -> u32 {
    // We rely on the wl_pointer.frame event
    const WL_SEAT_MIN_VERSION: u32 = 5;
    const WL_SEAT_MAX_VERSION: u32 = 9;

    if version < WL_SEAT_MIN_VERSION {
        panic!(
            "wl_seat below required version: {} < {}",
            version, WL_SEAT_MIN_VERSION
        );
    }

    version.clamp(WL_SEAT_MIN_VERSION, WL_SEAT_MAX_VERSION)
}

fn wl_output_version(version: u32) -> u32 {
    const WL_OUTPUT_MIN_VERSION: u32 = 2;
    const WL_OUTPUT_MAX_VERSION: u32 = 4;

    if version < WL_OUTPUT_MIN_VERSION {
        panic!(
            "wl_output below required version: {} < {}",
            version, WL_OUTPUT_MIN_VERSION
        );
    }

    version.clamp(WL_OUTPUT_MIN_VERSION, WL_OUTPUT_MAX_VERSION)
}

// rmac: sysexits.h's EX_UNAVAILABLE. Distinct from a normal panic's exit
// code (101) and from the clean-shutdown exit (0) that ends this process
// when its last window closes, so the systemd unit's Restart=on-success does
// not treat "no compositor to connect to" as the same case it exists to
// re-arm for, and OnFailure handling can tell the two apart.
const RMAC_NO_COMPOSITOR_EXIT_CODE: i32 = 69;

impl WaylandClient {
    pub(crate) fn new() -> Self {
        super::frame_trace::init();
        let startup_activation_token = take_startup_activation_token_from_environment();
        // rmac: connecting can fail for a completely ordinary reason -- this
        // process respawning in the brief window after the compositor exits
        // during logout or a session switch, before the session's units are
        // torn down (systemd Restart=on-success re-arms the process when its
        // last window closes, which niri exiting also looks like). That is
        // an expected shutdown race, not a bug worth a panic and a
        // backtrace: report it in one line and exit distinctly instead.
        let conn = Connection::connect_to_env().unwrap_or_else(|error| {
            eprintln!("gpui: no Wayland compositor to connect to: {error}");
            std::process::exit(RMAC_NO_COMPOSITOR_EXIT_CODE);
        });

        let (globals, event_queue) = registry_queue_init::<WaylandClientStatePtr>(&conn).unwrap();
        let qh = event_queue.handle();

        let mut seat: Option<wl_seat::WlSeat> = None;
        #[allow(clippy::mutable_key_type)]
        let mut in_progress_outputs = HashMap::default();
        #[allow(clippy::mutable_key_type)]
        let mut wl_outputs: HashMap<ObjectId, wl_output::WlOutput> = HashMap::default();
        globals.contents().with_list(|list| {
            for global in list {
                match &global.interface[..] {
                    "wl_seat" => {
                        seat = Some(globals.registry().bind::<wl_seat::WlSeat, _, _>(
                            global.name,
                            wl_seat_version(global.version),
                            &qh,
                            (),
                        ));
                    }
                    "wl_output" => {
                        let output = globals.registry().bind::<wl_output::WlOutput, _, _>(
                            global.name,
                            wl_output_version(global.version),
                            &qh,
                            (),
                        );
                        in_progress_outputs.insert(output.id(), InProgressOutput::default());
                        wl_outputs.insert(output.id(), output);
                    }
                    _ => {}
                }
            }
        });

        let event_loop = EventLoop::<WaylandClientStatePtr>::try_new().unwrap();

        let (common, main_receiver, wake_receiver) = LinuxCommon::new(event_loop.get_signal());

        let handle = event_loop.handle();
        handle
            .insert_source(main_receiver, {
                let handle = handle.clone();
                move |event, _, _: &mut WaylandClientStatePtr| {
                    if let calloop::channel::Event::Msg(runnable) = event {
                        handle.insert_idle(|_| {
                            let location = runnable.metadata().location;
                            let spawned = runnable.metadata().spawned;
                            profiler::update_running_task(spawned, location);
                            runnable.run();
                            profiler::save_task_timing();
                        });
                    }
                }
            })
            .unwrap();

        handle
            .insert_source(
                wake_receiver,
                |event, _, client: &mut WaylandClientStatePtr| {
                    if let calloop::channel::Event::Msg(()) = event {
                        client.get_client().borrow_mut().common.handle_system_wake();
                    }
                },
            )
            .unwrap();

        let compositor_gpu = detect_compositor_gpu();
        let gpu_context = Rc::new(RefCell::new(None));

        let seat = seat.unwrap();
        let globals = Globals::new(
            globals,
            common.foreground_executor.clone(),
            qh.clone(),
            seat.clone(),
        );

        let data_device = globals
            .data_device_manager
            .as_ref()
            .map(|data_device_manager| data_device_manager.get_data_device(&seat, &qh, ()));

        let primary_selection = globals
            .primary_selection_manager
            .as_ref()
            .map(|primary_selection_manager| primary_selection_manager.get_device(&seat, &qh, ()));

        let cursor = Cursor::new(&conn, &globals, 24);

        handle
            .insert_source(XDPEventSource::new(&common.background_executor), {
                move |event, _, client| match event {
                    XDPEvent::WindowAppearance(appearance) => {
                        if let Some(client) = client.0.upgrade() {
                            let mut client = client.borrow_mut();

                            client.common.appearance = appearance;

                            for window in client.windows.values_mut() {
                                window.set_appearance(appearance);
                            }
                        }
                    }
                    XDPEvent::ButtonLayout(layout_str) => {
                        if let Some(client) = client.0.upgrade() {
                            let layout = WindowButtonLayout::parse(&layout_str)
                                .log_err()
                                .unwrap_or_else(WindowButtonLayout::linux_default);
                            let mut client = client.borrow_mut();
                            client.common.button_layout = layout;

                            for window in client.windows.values_mut() {
                                window.set_button_layout();
                            }
                        }
                    }
                    XDPEvent::CursorTheme(theme) => {
                        if let Some(client) = client.0.upgrade() {
                            let mut client = client.borrow_mut();
                            client.cursor.set_theme(theme);
                        }
                    }
                    XDPEvent::CursorSize(size) => {
                        if let Some(client) = client.0.upgrade() {
                            let mut client = client.borrow_mut();
                            client.cursor.set_size(size);
                        }
                    }
                }
            })
            .unwrap();

        let state = Rc::new(RefCell::new(WaylandClientState {
            connection: conn.clone(),
            serial_tracker: SerialTracker::new(),
            globals,
            gpu_context,
            compositor_gpu,
            wl_seat: seat,
            wl_pointer: None,
            wl_touch: None,
            touch_ids: HashSet::new(),
            touch_suppressed: false,
            touch: None,
            touch_generation: 0,
            touch_hold_token: None,
            wl_keyboard: None,
            pinch_gesture: None,
            pinch_scale: 1.0,
            cursor_shape_device: None,
            data_device,
            primary_selection,
            text_input: None,
            pre_edit_text: None,
            ime_pre_edit: None,
            composing: false,
            outputs: HashMap::default(),
            in_progress_outputs,
            wl_outputs,
            windows: HashMap::default(),
            common,
            keyboard_layout: LinuxKeyboardLayout::new(UNKNOWN_KEYBOARD_LAYOUT_NAME),
            keymap_state: None,
            compose_state: None,
            drag: DragState {
                data_offer: None,
                window: None,
                position: Point::default(),
                action: None,
                paths_ready: false,
                drop_pending: false,
            },
            staged_file_drag: None,
            file_drag_source: None,
            press_serial: None,
            click: ClickState {
                last_click: Instant::now(),
                last_mouse_button: None,
                last_location: Point::default(),
                current_count: 0,
            },
            repeat: KeyRepeat {
                characters_per_second: 16,
                delay: Duration::from_millis(500),
                current_id: 0,
                current_keycode: None,
            },
            modifiers: Modifiers {
                shift: false,
                control: false,
                alt: false,
                function: false,
                platform: false,
            },
            capslock: Capslock { on: false },
            scroll_event_received: false,
            scroll_velocity: point(0.0, 0.0),
            last_finger_scroll: None,
            axis_stop_pending: false,
            momentum_generation: 0,
            axis_source: AxisSource::Wheel,
            mouse_location: None,
            continuous_scroll_delta: None,
            discrete_scroll_delta: None,
            vertical_modifier: -1.0,
            horizontal_modifier: -1.0,
            button_pressed: None,
            pending_window_move: None,
            mouse_focused_window: None,
            keyboard_focused_window: None,
            loop_handle: handle.clone(),
            enter_token: None,
            cursor_style: None,
            cursor_hidden_window: None,
            clipboard: Clipboard::new(conn.clone(), handle.clone()),
            data_offers: Vec::new(),
            primary_data_offer: None,
            cursor,
            pending_activation: None,
            startup_activation_token,
            event_loop: Some(event_loop),
            ime_enabled: None,
        }));
        FILE_DRAG_CLIENT.with(|slot| *slot.borrow_mut() = Rc::downgrade(&state));

        WaylandSource::new(conn, event_queue)
            .insert(handle)
            .unwrap();

        Self(state)
    }
}

impl LinuxClient for WaylandClient {
    fn keyboard_layout(&self) -> Box<dyn PlatformKeyboardLayout> {
        Box::new(self.0.borrow().keyboard_layout.clone())
    }

    fn displays(&self) -> Vec<Rc<dyn PlatformDisplay>> {
        self.0
            .borrow()
            .outputs
            .iter()
            .map(|(id, output)| {
                Rc::new(WaylandDisplay {
                    id: id.clone(),
                    name: output.name.clone(),
                    bounds: output.bounds.to_pixels(output.scale as f32),
                }) as Rc<dyn PlatformDisplay>
            })
            .collect()
    }

    fn display(&self, id: DisplayId) -> Option<Rc<dyn PlatformDisplay>> {
        self.0
            .borrow()
            .outputs
            .iter()
            .find_map(|(object_id, output)| {
                (object_id.protocol_id() as u64 == u64::from(id)).then(|| {
                    Rc::new(WaylandDisplay {
                        id: object_id.clone(),
                        name: output.name.clone(),
                        bounds: output.bounds.to_pixels(output.scale as f32),
                    }) as Rc<dyn PlatformDisplay>
                })
            })
    }

    fn primary_display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        None
    }

    #[cfg(feature = "screen-capture")]
    fn screen_capture_sources(
        &self,
    ) -> futures::channel::oneshot::Receiver<anyhow::Result<Vec<Rc<dyn gpui::ScreenCaptureSource>>>>
    {
        // TODO: Get screen capture working on wayland. Be sure to try window resizing as that may
        // be tricky.
        //
        // start_scap_default_target_source()
        let (sources_tx, sources_rx) = futures::channel::oneshot::channel();
        sources_tx
            .send(Err(anyhow::anyhow!(
                "Wayland screen capture not yet implemented."
            )))
            .ok();
        sources_rx
    }

    fn open_window(
        &self,
        handle: AnyWindowHandle,
        params: WindowParams,
    ) -> anyhow::Result<Box<dyn PlatformWindow>> {
        let mut state = self.0.borrow_mut();

        // Popups name their parent explicitly. Other kinds are parented to the focused window.
        let (parent, popup_grab) = match &params.kind {
            WindowKind::AnchoredPopup(options) => {
                let parent = state
                    .windows
                    .values()
                    .find(|window| window.handle() == options.parent)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("popup parent window not found"))?;
                // A popup grab must reference a press event or the compositor declines it and
                // immediately dismisses the popup, so use the most recent press serial, or no
                // grab before any press.
                let popup_grab = options.grab.then(|| {
                    let serial = state
                        .serial_tracker
                        .get(SerialKind::MousePress)
                        .max(state.serial_tracker.get(SerialKind::KeyPress));
                    (serial != 0).then(|| (serial, state.wl_seat.clone()))
                });
                (Some(parent), popup_grab.flatten())
            }
            _ => (state.keyboard_focused_window.clone(), None),
        };

        let target_output = params.display_id.and_then(|display_id| {
            let target_protocol_id: u64 = display_id.into();
            state
                .wl_outputs
                .iter()
                .find(|(id, _)| id.protocol_id() as u64 == target_protocol_id)
                .map(|(_, output)| output.clone())
        });

        let appearance = state.common.appearance;
        let compositor_gpu = state.compositor_gpu.take();

        let (window, surface_id) = WaylandWindow::new(
            handle,
            state.globals.clone(),
            state.gpu_context.clone(),
            compositor_gpu,
            WaylandClientStatePtr(Rc::downgrade(&self.0)),
            params,
            appearance,
            parent,
            popup_grab,
            target_output,
        )?;

        if window.0.toplevel().is_some() {
            state.consume_startup_activation_token(&window.0.surface());
        }
        state.windows.insert(surface_id, window.0.clone());

        Ok(Box::new(window))
    }

    fn set_cursor_style(&self, style: CursorStyle) {
        let mut state = self.0.borrow_mut();

        let need_update = state.cursor_style != Some(style)
            && (state.mouse_focused_window.is_none()
                || state
                    .mouse_focused_window
                    .as_ref()
                    .is_some_and(|w| !w.is_blocked()));

        if !need_update {
            return;
        }

        state.cursor_style = Some(style);

        // Don't clobber the invisible cursor; restore reads back from `cursor_style`.
        if state.cursor_hidden_window.is_some() {
            return;
        }

        let serial = state.serial_tracker.get(SerialKind::MouseEnter);
        if let Some(cursor_shape_device) = &state.cursor_shape_device {
            cursor_shape_device.set_shape(serial, to_shape(style));
        } else if let Some(focused_window) = &state.mouse_focused_window {
            // cursor-shape-v1 isn't supported, set the cursor using a surface.
            let wl_pointer = state
                .wl_pointer
                .clone()
                .expect("window is focused by pointer");
            let scale = focused_window.primary_output_scale();
            state.cursor.set_icon(
                &wl_pointer,
                serial,
                cursor_style_to_icon_names(style),
                scale,
            );
        }
    }

    fn hide_cursor_until_mouse_moves(&self) {
        self.0.borrow_mut().hide_cursor_until_mouse_moves();
    }

    fn is_cursor_visible(&self) -> bool {
        self.0.borrow().cursor_hidden_window.is_none()
    }

    fn open_uri(&self, uri: &str) {
        let mut state = self.0.borrow_mut();
        if let (Some(activation), Some(window)) = (
            state.globals.activation.clone(),
            state.mouse_focused_window.clone(),
        ) {
            state.pending_activation = Some(PendingActivation::Uri(uri.to_string()));
            let token = activation.get_activation_token(&state.globals.qh, ());
            let serial = state.serial_tracker.get(SerialKind::MousePress);
            token.set_serial(serial, &state.wl_seat);
            token.set_surface(&window.surface());
            token.commit();
        } else {
            let executor = state.common.background_executor.clone();
            open_uri_internal(executor, uri, None);
        }
    }

    fn reveal_path(&self, path: PathBuf) {
        let mut state = self.0.borrow_mut();
        if let (Some(activation), Some(window)) = (
            state.globals.activation.clone(),
            state.mouse_focused_window.clone(),
        ) {
            state.pending_activation = Some(PendingActivation::Path(path));
            let token = activation.get_activation_token(&state.globals.qh, ());
            let serial = state.serial_tracker.get(SerialKind::MousePress);
            token.set_serial(serial, &state.wl_seat);
            token.set_surface(&window.surface());
            token.commit();
        } else {
            let executor = state.common.background_executor.clone();
            reveal_path_internal(executor, path, None);
        }
    }

    fn with_common<R>(&self, f: impl FnOnce(&mut LinuxCommon) -> R) -> R {
        f(&mut self.0.borrow_mut().common)
    }

    fn run(&self) {
        let mut event_loop = self
            .0
            .borrow_mut()
            .event_loop
            .take()
            .expect("App is already running");

        event_loop
            .run(
                None,
                &mut WaylandClientStatePtr(Rc::downgrade(&self.0)),
                // rmac: every wake-up of this loop (input, a Wayland event, a
                // task or a timer) may have made a parked window dirty; check
                // them here instead of on a timer, so an idle app sleeps.
                |client| {
                    let Some(client) = client.0.upgrade() else {
                        return;
                    };
                    let parked: Vec<WaylandWindowStatePtr> = client
                        .borrow()
                        .windows
                        .values()
                        .filter(|window| window.is_parked())
                        .cloned()
                        .collect();
                    for window in parked {
                        window.check_parked();
                    }
                },
            )
            .log_err();
    }

    fn write_to_primary(&self, item: gpui::ClipboardItem) {
        let mut state = self.0.borrow_mut();
        let (Some(primary_selection_manager), Some(primary_selection)) = (
            state.globals.primary_selection_manager.clone(),
            state.primary_selection.clone(),
        ) else {
            return;
        };
        if state.mouse_focused_window.is_some() || state.keyboard_focused_window.is_some() {
            state.clipboard.set_primary(item);
            let serial = state.serial_tracker.get_latest();
            let data_source = primary_selection_manager.create_source(&state.globals.qh, ());
            for mime_type in TEXT_MIME_TYPES {
                data_source.offer(mime_type.to_string());
            }
            data_source.offer(state.clipboard.self_mime());
            primary_selection.set_selection(Some(&data_source), serial);
        }
    }

    fn write_to_clipboard(&self, item: gpui::ClipboardItem) {
        let mut state = self.0.borrow_mut();
        let (Some(data_device_manager), Some(data_device)) = (
            state.globals.data_device_manager.clone(),
            state.data_device.clone(),
        ) else {
            return;
        };
        if state.mouse_focused_window.is_some() || state.keyboard_focused_window.is_some() {
            state.clipboard.set(item);
            let serial = state.serial_tracker.get_latest();
            let data_source = data_device_manager.create_data_source(&state.globals.qh, ());
            for mime_type in TEXT_MIME_TYPES {
                data_source.offer(mime_type.to_string());
            }
            data_source.offer(state.clipboard.self_mime());
            data_device.set_selection(Some(&data_source), serial);
        }
    }

    fn read_from_primary(&self) -> Option<gpui::ClipboardItem> {
        self.0.borrow_mut().clipboard.read_primary()
    }

    fn read_from_clipboard(&self) -> Option<gpui::ClipboardItem> {
        self.0.borrow_mut().clipboard.read()
    }

    fn active_window(&self) -> Option<AnyWindowHandle> {
        self.0
            .borrow_mut()
            .keyboard_focused_window
            .as_ref()
            .map(|window| window.handle())
    }

    fn window_stack(&self) -> Option<Vec<AnyWindowHandle>> {
        None
    }

    fn compositor_name(&self) -> &'static str {
        "Wayland"
    }

    fn window_identifier(&self) -> impl Future<Output = Option<WindowIdentifier>> + Send + 'static {
        async fn inner(surface: Option<wl_surface::WlSurface>) -> Option<WindowIdentifier> {
            if let Some(surface) = surface {
                ashpd::WindowIdentifier::from_wayland(&surface).await
            } else {
                None
            }
        }

        let client_state = self.0.borrow();
        let active_window = client_state.keyboard_focused_window.as_ref();
        inner(active_window.map(|aw| aw.surface()))
    }
}

struct DmabufProbeState {
    device: Option<u64>,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for DmabufProbeState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, ()> for DmabufProbeState {
    fn event(
        _: &mut Self,
        _: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
        _: zwp_linux_dmabuf_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwp_linux_dmabuf_feedback_v1::ZwpLinuxDmabufFeedbackV1, ()> for DmabufProbeState {
    fn event(
        state: &mut Self,
        _: &zwp_linux_dmabuf_feedback_v1::ZwpLinuxDmabufFeedbackV1,
        event: zwp_linux_dmabuf_feedback_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_linux_dmabuf_feedback_v1::Event::MainDevice { device } = event
            && let Ok(bytes) = <[u8; 8]>::try_from(device.as_slice())
        {
            state.device = Some(u64::from_ne_bytes(bytes));
        }
    }
}

fn detect_compositor_gpu() -> Option<CompositorGpuHint> {
    let connection = Connection::connect_to_env().ok()?;
    let (globals, mut event_queue) = registry_queue_init::<DmabufProbeState>(&connection).ok()?;
    let queue_handle = event_queue.handle();

    let dmabuf: zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1 =
        globals.bind(&queue_handle, 4..=4, ()).ok()?;
    let feedback = dmabuf.get_default_feedback(&queue_handle, ());

    let mut state = DmabufProbeState { device: None };

    event_queue.roundtrip(&mut state).ok()?;

    feedback.destroy();
    dmabuf.destroy();

    crate::linux::compositor_gpu_hint_from_dev_t(state.device?)
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();

        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => match &interface[..] {
                "wl_seat" => {
                    if let Some(wl_pointer) = state.wl_pointer.take() {
                        wl_pointer.release();
                    }
                    if let Some(wl_touch) = state.wl_touch.take() {
                        wl_touch.release();
                    }
                    state.touch = None;
                    state.touch_ids.clear();
                    state.cancel_touch_hold();
                    state.touch_generation = state.touch_generation.wrapping_add(1);
                    if let Some(wl_keyboard) = state.wl_keyboard.take() {
                        wl_keyboard.release();
                    }
                    state.wl_seat.release();
                    state.wl_seat = registry.bind::<wl_seat::WlSeat, _, _>(
                        name,
                        wl_seat_version(version),
                        qh,
                        (),
                    );
                }
                "wl_output" => {
                    let output = registry.bind::<wl_output::WlOutput, _, _>(
                        name,
                        wl_output_version(version),
                        qh,
                        (),
                    );

                    state
                        .in_progress_outputs
                        .insert(output.id(), InProgressOutput::default());
                    state.wl_outputs.insert(output.id(), output);
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name: _ } => {
                // TODO: handle global removal
            }
            _ => {}
        }
    }
}

delegate_noop!(WaylandClientStatePtr: ignore xdg_activation_v1::XdgActivationV1);
delegate_noop!(WaylandClientStatePtr: ignore xdg_system_bell_v1::XdgSystemBellV1);
delegate_noop!(WaylandClientStatePtr: ignore wl_compositor::WlCompositor);
delegate_noop!(WaylandClientStatePtr: ignore wp_cursor_shape_device_v1::WpCursorShapeDeviceV1);
delegate_noop!(WaylandClientStatePtr: ignore wp_cursor_shape_manager_v1::WpCursorShapeManagerV1);
delegate_noop!(WaylandClientStatePtr: ignore wl_data_device_manager::WlDataDeviceManager);
delegate_noop!(WaylandClientStatePtr: ignore zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1);
delegate_noop!(WaylandClientStatePtr: ignore wl_shm::WlShm);
delegate_noop!(WaylandClientStatePtr: ignore wl_shm_pool::WlShmPool);
delegate_noop!(WaylandClientStatePtr: ignore wl_buffer::WlBuffer);
delegate_noop!(WaylandClientStatePtr: ignore wl_region::WlRegion);
delegate_noop!(WaylandClientStatePtr: ignore wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1);
delegate_noop!(WaylandClientStatePtr: ignore zxdg_decoration_manager_v1::ZxdgDecorationManagerV1);
delegate_noop!(WaylandClientStatePtr: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);
delegate_noop!(WaylandClientStatePtr: ignore xdg_positioner::XdgPositioner);
delegate_noop!(WaylandClientStatePtr: ignore org_kde_kwin_blur_manager::OrgKdeKwinBlurManager);
delegate_noop!(WaylandClientStatePtr: ignore zwp_text_input_manager_v3::ZwpTextInputManagerV3);
delegate_noop!(WaylandClientStatePtr: ignore org_kde_kwin_blur::OrgKdeKwinBlur);
delegate_noop!(WaylandClientStatePtr: ignore wp_viewporter::WpViewporter);
delegate_noop!(WaylandClientStatePtr: ignore wp_viewport::WpViewport);

impl Dispatch<WlCallback, ObjectId> for WaylandClientStatePtr {
    fn event(
        state: &mut WaylandClientStatePtr,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        surface_id: &ObjectId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = state.get_client();
        let mut state = client.borrow_mut();
        let Some(window) = get_window(&mut state, surface_id) else {
            return;
        };
        drop(state);

        if let wl_callback::Event::Done { .. } = event {
            window.frame();
        }
    }
}

pub(crate) fn get_window(
    state: &mut RefMut<WaylandClientState>,
    surface_id: &ObjectId,
) -> Option<WaylandWindowStatePtr> {
    state.windows.get(surface_id).cloned()
}

impl Dispatch<wl_surface::WlSurface, ()> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        surface: &wl_surface::WlSurface,
        event: <wl_surface::WlSurface as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();

        let Some(window) = get_window(&mut state, &surface.id()) else {
            return;
        };
        #[allow(clippy::mutable_key_type)]
        let outputs = state.outputs.clone();
        drop(state);

        window.handle_surface_event(event, outputs);
    }
}

impl Dispatch<wl_output::WlOutput, ()> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        output: &wl_output::WlOutput,
        event: <wl_output::WlOutput as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();

        let Some(in_progress_output) = state.in_progress_outputs.get_mut(&output.id()) else {
            return;
        };

        match event {
            wl_output::Event::Name { name } => {
                in_progress_output.name = Some(name);
            }
            wl_output::Event::Scale { factor } => {
                in_progress_output.scale = Some(factor);
            }
            wl_output::Event::Geometry { x, y, subpixel, .. } => {
                in_progress_output.position = Some(point(DevicePixels(x), DevicePixels(y)));
                if let WEnum::Value(subpixel) = subpixel {
                    in_progress_output.subpixel = Some(subpixel);
                }
            }
            wl_output::Event::Mode { width, height, .. } => {
                in_progress_output.size = Some(size(DevicePixels(width), DevicePixels(height)))
            }
            wl_output::Event::Done => {
                if let Some(complete) = in_progress_output.complete() {
                    state.outputs.insert(output.id(), complete);
                }
                state.in_progress_outputs.remove(&output.id());
            }
            _ => {}
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ObjectId> for WaylandClientStatePtr {
    fn event(
        state: &mut Self,
        _: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        surface_id: &ObjectId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = state.get_client();
        let mut state = client.borrow_mut();
        let Some(window) = get_window(&mut state, surface_id) else {
            return;
        };
        drop(state);
        window.handle_xdg_surface_event(event);
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ObjectId> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: <xdg_toplevel::XdgToplevel as Proxy>::Event,
        surface_id: &ObjectId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();
        let Some(window) = get_window(&mut state, surface_id) else {
            return;
        };

        drop(state);
        let should_close = window.handle_toplevel_event(event);

        if should_close {
            // The close logic will be handled in drop_window()
            window.close();
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ObjectId> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        _: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: <zwlr_layer_surface_v1::ZwlrLayerSurfaceV1 as Proxy>::Event,
        surface_id: &ObjectId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();
        let Some(window) = get_window(&mut state, surface_id) else {
            return;
        };

        drop(state);
        let should_close = window.handle_layersurface_event(event);

        if should_close {
            // Close logic will be handled in drop_window()
            window.close();
        }
    }
}

impl Dispatch<xdg_popup::XdgPopup, ObjectId> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        _: &xdg_popup::XdgPopup,
        event: <xdg_popup::XdgPopup as Proxy>::Event,
        surface_id: &ObjectId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();
        let Some(window) = get_window(&mut state, surface_id) else {
            return;
        };

        drop(state);
        let should_close = window.handle_popup_event(event);

        if should_close {
            // The close logic will be handled in drop_window()
            window.close();
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for WaylandClientStatePtr {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: <xdg_wm_base::XdgWmBase as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_activation_token_v1::XdgActivationTokenV1, ()> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        token: &xdg_activation_token_v1::XdgActivationTokenV1,
        event: <xdg_activation_token_v1::XdgActivationTokenV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();

        if let xdg_activation_token_v1::Event::Done { token } = event {
            let executor = state.common.background_executor.clone();
            match state.pending_activation.take() {
                Some(PendingActivation::Uri(uri)) => open_uri_internal(executor, &uri, Some(token)),
                Some(PendingActivation::Path(path)) => {
                    reveal_path_internal(executor, path, Some(token))
                }
                Some(PendingActivation::Window(window)) => {
                    let Some(window) = get_window(&mut state, &window) else {
                        return;
                    };
                    let activation = state.globals.activation.as_ref().unwrap();
                    activation.activate(token, &window.surface());
                }
                None => log::error!("activation token received with no pending activation"),
            }
        }

        token.destroy();
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for WaylandClientStatePtr {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        {
            let client = state.get_client();
            let mut state = client.borrow_mut();
            if capabilities.contains(wl_seat::Capability::Keyboard) {
                let keyboard = seat.get_keyboard(qh, ());

                if let Some(text_input) = state.text_input.take() {
                    text_input.destroy();
                    state.ime_pre_edit = None;
                    state.composing = false;
                }

                state.text_input = state
                    .globals
                    .text_input_manager
                    .as_ref()
                    .map(|text_input_manager| text_input_manager.get_text_input(seat, qh, ()));

                if let Some(wl_keyboard) = &state.wl_keyboard {
                    wl_keyboard.release();
                }

                state.wl_keyboard = Some(keyboard);
            }
            if capabilities.contains(wl_seat::Capability::Pointer) {
                let pointer = seat.get_pointer(qh, ());

                if let Some(cursor_shape_device) = state.cursor_shape_device.take() {
                    cursor_shape_device.destroy();
                }

                state.cursor_shape_device = state
                    .globals
                    .cursor_shape_manager
                    .as_ref()
                    .map(|cursor_shape_manager| cursor_shape_manager.get_pointer(&pointer, qh, ()));

                state.pinch_gesture = state.globals.gesture_manager.as_ref().map(
                    |gesture_manager: &zwp_pointer_gestures_v1::ZwpPointerGesturesV1| {
                        gesture_manager.get_pinch_gesture(&pointer, qh, ())
                    },
                );

                if let Some(wl_pointer) = &state.wl_pointer {
                    wl_pointer.release();
                }

                state.wl_pointer = Some(pointer);
            } else if let Some(pointer) = state.wl_pointer.take() {
                pointer.release();
            }
            if capabilities.contains(wl_seat::Capability::Touch) {
                if state.wl_touch.is_none() {
                    state.wl_touch = Some(seat.get_touch(qh, ()));
                }
            } else if let Some(touch) = state.wl_touch.take() {
                touch.release();
                state.touch_ids.clear();
                state.touch_suppressed = false;
                state.cancel_touch_hold();
                state.touch_generation = state.touch_generation.wrapping_add(1);
                if let Some(contact) = state.touch.take() {
                    drop(state);
                    end_touch_contact(&client, contact, true);
                }
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();
        match event {
            wl_keyboard::Event::RepeatInfo { rate, delay } => {
                state.repeat.characters_per_second = rate as u32;
                state.repeat.delay = Duration::from_millis(delay as u64);
            }
            wl_keyboard::Event::Keymap {
                format: WEnum::Value(format),
                fd,
                size,
                ..
            } => {
                if format != wl_keyboard::KeymapFormat::XkbV1 {
                    log::error!("Received keymap format {:?}, expected XkbV1", format);
                    return;
                }
                let xkb_context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
                let keymap = unsafe {
                    xkb::Keymap::new_from_fd(
                        &xkb_context,
                        fd,
                        size as usize,
                        XKB_KEYMAP_FORMAT_TEXT_V1,
                        KEYMAP_COMPILE_NO_FLAGS,
                    )
                    .log_err()
                    .flatten()
                    .expect("Failed to create keymap")
                };
                state.keymap_state = Some(xkb::State::new(&keymap));
                state.compose_state = get_xkb_compose_state(&xkb_context);
                drop(state);

                this.handle_keyboard_layout_change();
            }
            wl_keyboard::Event::Enter { surface, .. } => {
                state.keyboard_focused_window = get_window(&mut state, &surface.id());
                state.enter_token = Some(());

                if let Some(window) = state.keyboard_focused_window.clone() {
                    drop(state);
                    window.set_focused(true);
                }
            }
            wl_keyboard::Event::Leave { surface, .. } => {
                let keyboard_focused_window = get_window(&mut state, &surface.id());
                state.keyboard_focused_window = None;
                state.enter_token.take();
                // Prevent keyboard events from repeating after opening e.g. a file chooser and closing it quickly
                state.repeat.current_id += 1;
                state.restore_cursor_after_hide();

                if let Some(window) = keyboard_focused_window {
                    if let Some(ref mut compose) = state.compose_state {
                        compose.reset();
                    }
                    state.pre_edit_text.take();
                    drop(state);
                    window.handle_ime(ImeInput::DeleteText);
                    window.set_focused(false);
                }
            }
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                let focused_window = state.keyboard_focused_window.clone();

                let keymap_state = state.keymap_state.as_mut().unwrap();
                let old_layout =
                    keymap_state.serialize_layout(xkbcommon::xkb::STATE_LAYOUT_EFFECTIVE);
                keymap_state.update_mask(mods_depressed, mods_latched, mods_locked, 0, 0, group);
                state.modifiers = modifiers_from_xkb(keymap_state);
                let keymap_state = state.keymap_state.as_mut().unwrap();
                state.capslock = capslock_from_xkb(keymap_state);

                let input = PlatformInput::ModifiersChanged(ModifiersChangedEvent {
                    modifiers: state.modifiers,
                    capslock: state.capslock,
                });
                drop(state);

                if let Some(focused_window) = focused_window {
                    focused_window.handle_input(input);
                }

                if group != old_layout {
                    this.handle_keyboard_layout_change();
                }
            }
            wl_keyboard::Event::Key {
                serial,
                key,
                state: WEnum::Value(key_state),
                ..
            } => {
                state.serial_tracker.update(SerialKind::KeyPress, serial);

                let focused_window = state.keyboard_focused_window.clone();
                let Some(focused_window) = focused_window else {
                    return;
                };

                let keymap_state = state.keymap_state.as_ref().unwrap();
                let keycode = Keycode::from(key + MIN_KEYCODE);
                let keysym = keymap_state.key_get_one_sym(keycode);

                match key_state {
                    wl_keyboard::KeyState::Pressed if !keysym.is_modifier_key() => {
                        let mut keystroke =
                            keystroke_from_xkb(keymap_state, state.modifiers, keycode);
                        if let Some(mut compose) = state.compose_state.take() {
                            compose.feed(keysym);
                            match compose.status() {
                                xkb::Status::Composing => {
                                    keystroke.key_char = None;
                                    state.pre_edit_text =
                                        compose.utf8().or(keystroke_underlying_dead_key(keysym));
                                    let pre_edit = state.pre_edit_text.clone().unwrap_or_default();
                                    drop(state);
                                    focused_window.handle_ime(ImeInput::SetMarkedText(pre_edit));
                                    state = client.borrow_mut();
                                }

                                xkb::Status::Composed => {
                                    state.pre_edit_text.take();
                                    keystroke.key_char = compose.utf8();
                                    if let Some(keysym) = compose.keysym() {
                                        keystroke.key = xkb::keysym_get_name(keysym);
                                    }
                                }
                                xkb::Status::Cancelled => {
                                    let pre_edit = state.pre_edit_text.take();
                                    let new_pre_edit = keystroke_underlying_dead_key(keysym);
                                    state.pre_edit_text = new_pre_edit.clone();
                                    drop(state);
                                    if let Some(pre_edit) = pre_edit {
                                        focused_window.handle_ime(ImeInput::InsertText(pre_edit));
                                    }
                                    if let Some(current_key) = new_pre_edit {
                                        focused_window
                                            .handle_ime(ImeInput::SetMarkedText(current_key));
                                    }
                                    compose.feed(keysym);
                                    state = client.borrow_mut();
                                }
                                _ => {}
                            }
                            state.compose_state = Some(compose);
                        }
                        let input = PlatformInput::KeyDown(KeyDownEvent {
                            keystroke: keystroke.clone(),
                            is_held: false,
                            prefer_character_input: false,
                        });

                        state.repeat.current_id += 1;
                        state.repeat.current_keycode = Some(keycode);

                        let rate = state.repeat.characters_per_second;
                        let repeat_interval = Duration::from_secs(1) / rate.max(1);
                        let id = state.repeat.current_id;
                        state
                            .loop_handle
                            .insert_source(Timer::from_duration(state.repeat.delay), {
                                let input = PlatformInput::KeyDown(KeyDownEvent {
                                    keystroke,
                                    is_held: true,
                                    prefer_character_input: false,
                                });
                                move |event_timestamp, _metadata, this| {
                                    let client = this.get_client();
                                    let state = client.borrow();
                                    let is_repeating = id == state.repeat.current_id
                                        && state.repeat.current_keycode.is_some()
                                        && state.keyboard_focused_window.is_some();

                                    if !is_repeating || rate == 0 {
                                        return TimeoutAction::Drop;
                                    }

                                    let focused_window =
                                        state.keyboard_focused_window.as_ref().unwrap().clone();

                                    drop(state);
                                    focused_window.handle_input(input.clone());

                                    // If the new scheduled time is in the past the event will repeat as soon as possible
                                    TimeoutAction::ToInstant(event_timestamp + repeat_interval)
                                }
                            })
                            .unwrap();

                        drop(state);
                        focused_window.handle_input(input);
                    }
                    wl_keyboard::KeyState::Released if !keysym.is_modifier_key() => {
                        let input = PlatformInput::KeyUp(KeyUpEvent {
                            keystroke: keystroke_from_xkb(keymap_state, state.modifiers, keycode),
                        });

                        if state.repeat.current_keycode == Some(keycode) {
                            state.repeat.current_keycode = None;
                        }

                        drop(state);
                        focused_window.handle_input(input);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<zwp_text_input_v3::ZwpTextInputV3, ()> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        text_input: &zwp_text_input_v3::ZwpTextInputV3,
        event: <zwp_text_input_v3::ZwpTextInputV3 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();
        match event {
            zwp_text_input_v3::Event::Enter { .. } => {
                drop(state);
                this.enable_ime();
            }
            zwp_text_input_v3::Event::Leave { .. } => {
                drop(state);
                this.disable_ime();
            }
            zwp_text_input_v3::Event::CommitString { text } => {
                state.composing = false;
                let Some(window) = state.keyboard_focused_window.clone() else {
                    return;
                };

                if let Some(commit_text) = text {
                    drop(state);
                    // IBus Intercepts keys like `a`, `b`, but those keys are needed for vim mode.
                    // We should only send ASCII characters to Zed, otherwise a user could remap a letter like `か` or `相`.
                    if commit_text.len() == 1 {
                        window.handle_input(PlatformInput::KeyDown(KeyDownEvent {
                            keystroke: Keystroke {
                                modifiers: Modifiers::default(),
                                key: commit_text.clone(),
                                key_char: Some(commit_text),
                            },
                            is_held: false,
                            prefer_character_input: false,
                        }));
                    } else {
                        window.handle_ime(ImeInput::InsertText(commit_text));
                    }
                }
            }
            zwp_text_input_v3::Event::PreeditString { text, .. } => {
                state.composing = true;
                state.ime_pre_edit = text;
            }
            zwp_text_input_v3::Event::Done { serial } => {
                let last_serial = state.serial_tracker.get(SerialKind::InputMethod);
                state.serial_tracker.update(SerialKind::InputMethod, serial);
                let Some(window) = state.keyboard_focused_window.clone() else {
                    return;
                };

                if let Some(text) = state.ime_pre_edit.take() {
                    drop(state);
                    window.handle_ime(ImeInput::SetMarkedText(text));
                    if let Some(area) = window.get_ime_area() {
                        text_input.set_cursor_rectangle(
                            f32::from(area.origin.x) as i32,
                            f32::from(area.origin.y) as i32,
                            f32::from(area.size.width) as i32,
                            f32::from(area.size.height) as i32,
                        );
                        if last_serial == serial {
                            text_input.commit();
                        }
                    }
                } else {
                    state.composing = false;
                    drop(state);
                    window.handle_ime(ImeInput::DeleteText);
                }
            }
            _ => {}
        }
    }
}

fn touch_click(
    client: &Rc<RefCell<WaylandClientState>>,
    window: &WaylandWindowStatePtr,
    button: MouseButton,
    position: Point<Pixels>,
) {
    let mut state = client.borrow_mut();
    state.momentum_generation = state.momentum_generation.wrapping_add(1);
    let click_count = state.count_click(button, position);
    let modifiers = state.modifiers;
    drop(state);
    window.handle_input(PlatformInput::MouseDown(MouseDownEvent {
        button,
        position,
        modifiers,
        click_count,
        first_mouse: false,
    }));
    window.handle_input(PlatformInput::MouseUp(MouseUpEvent {
        button,
        position,
        modifiers,
        click_count,
    }));
}

fn end_touch_contact(
    client: &Rc<RefCell<WaylandClientState>>,
    contact: TouchContact,
    cancelled: bool,
) {
    let state = client.borrow();
    let modifiers = state.modifiers;
    match contact.gesture {
        TouchGesture::Pending if !cancelled => {
            drop(state);
            let button = if classify_touch_hold(contact.gesture, contact.started.elapsed())
                == TouchGesture::ContextMenu
            {
                MouseButton::Right
            } else {
                MouseButton::Left
            };
            touch_click(client, &contact.window, button, contact.position);
        }
        TouchGesture::PointerDrag => {
            drop(state);
            contact
                .window
                .handle_input(PlatformInput::MouseUp(MouseUpEvent {
                    button: MouseButton::Left,
                    position: contact.position,
                    modifiers,
                    click_count: contact.click_count,
                }));
        }
        TouchGesture::Scroll => {
            drop(state);
            contact
                .window
                .handle_input(PlatformInput::ScrollWheel(ScrollWheelEvent {
                    position: contact.position,
                    delta: ScrollDelta::Pixels(point(px(0.0), px(0.0))),
                    modifiers,
                    touch_phase: TouchPhase::Ended,
                }));
            if !cancelled {
                start_momentum(
                    &mut client.borrow_mut(),
                    Some((contact.window.clone(), contact.position)),
                );
            }
        }
        _ => drop(state),
    }
    let state = client.borrow();
    let pointer = state
        .mouse_focused_window
        .as_ref()
        .filter(|window| window.ptr_eq(&contact.window))
        .and_then(|_| state.mouse_location);
    let modifiers = state.modifiers;
    let pressed_button = state.button_pressed;
    drop(state);
    if let Some(position) = pointer {
        contact
            .window
            .handle_input(PlatformInput::MouseMove(MouseMoveEvent {
                position,
                pressed_button,
                modifiers,
            }));
    } else {
        contact
            .window
            .handle_input(PlatformInput::MouseExited(MouseExitEvent {
                position: contact.position,
                pressed_button: None,
                modifiers,
            }));
        contact.window.set_hovered(false);
    }
}

impl Dispatch<wl_touch::WlTouch, ()> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        _: &wl_touch::WlTouch,
        event: wl_touch::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        match event {
            wl_touch::Event::Down {
                serial,
                surface,
                id,
                x,
                y,
                ..
            } => {
                let position = point(px(x as f32), px(y as f32));
                let mut state = client.borrow_mut();
                state.touch_ids.insert(id);
                if state.touch_ids.len() > 1 {
                    state.touch_suppressed = true;
                    state.cancel_touch_hold();
                    state.touch_generation = state.touch_generation.wrapping_add(1);
                    let old = state.touch.take();
                    drop(state);
                    if let Some(old) = old {
                        end_touch_contact(&client, old, true);
                    }
                    // Multi-finger gestures, including pinch, are ignored.
                    return;
                }
                if state.touch_suppressed {
                    return;
                }
                let Some(window) = get_window(&mut state, &surface.id()) else {
                    return;
                };
                state.serial_tracker.update(SerialKind::MousePress, serial);
                state.momentum_generation = state.momentum_generation.wrapping_add(1);
                state.last_finger_scroll = None;
                state.scroll_velocity = point(0.0, 0.0);
                state.touch_generation = state.touch_generation.wrapping_add(1);
                let generation = state.touch_generation;
                let modifiers = state.modifiers;
                let loop_handle = state.loop_handle.clone();
                drop(state);
                window.set_hovered(true);
                window.handle_input(PlatformInput::MouseMove(MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers,
                }));
                let drag_region =
                    window.hit_test_window_control() == Some(gpui::WindowControlArea::Drag);
                let files_window = window.is_files_window();
                client.borrow_mut().touch = Some(TouchContact {
                    id,
                    window,
                    start: position,
                    position,
                    serial,
                    started: Instant::now(),
                    gesture: TouchGesture::Pending,
                    drag_region,
                    files_window,
                    click_count: 1,
                });
                let token = loop_handle
                    .insert_source(
                        Timer::from_duration(TOUCH_HOLD),
                        move |_, _, this: &mut WaylandClientStatePtr| {
                            let client = this.get_client();
                            let mut state = client.borrow_mut();
                            if state.touch_generation != generation || state.touch_suppressed {
                                return TimeoutAction::Drop;
                            }
                            let Some(contact) = state.touch.as_mut() else {
                                return TimeoutAction::Drop;
                            };
                            let gesture =
                                classify_touch_hold(contact.gesture, contact.started.elapsed());
                            if gesture != TouchGesture::ContextMenu {
                                return TimeoutAction::Drop;
                            }
                            contact.gesture = gesture;
                            let window = contact.window.clone();
                            let position = contact.position;
                            state.touch_hold_token = None;
                            drop(state);
                            touch_click(&client, &window, MouseButton::Right, position);
                            TimeoutAction::Drop
                        },
                    )
                    .expect("touch hold timer registration failed");
                client.borrow_mut().touch_hold_token = Some(token);
            }
            wl_touch::Event::Motion { id, x, y, .. } => {
                let position = point(px(x as f32), px(y as f32));
                let mut state = client.borrow_mut();
                if state.touch_suppressed {
                    return;
                }
                let modifiers = state.modifiers;
                let Some(contact) = state.touch.as_mut().filter(|contact| contact.id == id) else {
                    return;
                };
                let previous = contact.position;
                contact.position = position;
                let mut phase = TouchPhase::Moved;
                let mut begin_drag = None;
                if contact.gesture == TouchGesture::Pending {
                    let next = classify_touch_motion(
                        contact.start,
                        position,
                        contact.started.elapsed(),
                        contact.drag_region,
                        contact.files_window,
                    );
                    if next != TouchGesture::Pending {
                        contact.gesture = next;
                        if next == TouchGesture::Scroll {
                            phase = TouchPhase::Started;
                        }
                        if next == TouchGesture::PointerDrag {
                            begin_drag = Some((contact.start, contact.serial, contact.drag_region));
                        }
                    }
                }
                let gesture = contact.gesture;
                let window = contact.window.clone();
                let delta = point(previous.x - position.x, previous.y - position.y);
                if gesture == TouchGesture::Scroll {
                    state.momentum_generation = state.momentum_generation.wrapping_add(1);
                    track_finger_velocity(&mut state, delta);
                }
                if begin_drag.is_some() {
                    let count = state.count_click(MouseButton::Left, previous);
                    if let Some(contact) = state.touch.as_mut() {
                        contact.click_count = count;
                    }
                }
                if gesture != TouchGesture::Pending {
                    state.cancel_touch_hold();
                }
                let click_count = state
                    .touch
                    .as_ref()
                    .map_or(1, |contact| contact.click_count);
                drop(state);
                if let Some((start, serial, title_drag)) = begin_drag {
                    window.handle_input(PlatformInput::MouseDown(MouseDownEvent {
                        button: MouseButton::Left,
                        position: start,
                        modifiers,
                        click_count,
                        first_mouse: false,
                    }));
                    if title_drag {
                        window.start_window_move_with_serial(serial);
                    }
                }
                match gesture {
                    TouchGesture::Scroll => {
                        window.handle_input(PlatformInput::ScrollWheel(ScrollWheelEvent {
                            position,
                            delta: ScrollDelta::Pixels(delta),
                            modifiers,
                            touch_phase: phase,
                        }))
                    }
                    TouchGesture::PointerDrag | TouchGesture::Pending => {
                        window.handle_input(PlatformInput::MouseMove(MouseMoveEvent {
                            position,
                            pressed_button: (gesture == TouchGesture::PointerDrag)
                                .then_some(MouseButton::Left),
                            modifiers,
                        }));
                        if gesture == TouchGesture::PointerDrag {
                            client
                                .borrow_mut()
                                .try_start_staged_file_drag(position, false);
                        }
                    }
                    TouchGesture::ContextMenu => {}
                }
            }
            wl_touch::Event::Up { id, .. } => {
                let mut state = client.borrow_mut();
                state.staged_file_drag = None;
                state.touch_ids.remove(&id);
                state.cancel_touch_hold();
                state.touch_generation = state.touch_generation.wrapping_add(1);
                let contact = state.touch.take().filter(|contact| contact.id == id);
                if state.touch_ids.is_empty() {
                    state.touch_suppressed = false;
                }
                drop(state);
                if let Some(contact) = contact {
                    end_touch_contact(&client, contact, false);
                }
            }
            wl_touch::Event::Cancel => {
                let mut state = client.borrow_mut();
                state.staged_file_drag = None;
                state.touch_ids.clear();
                state.touch_suppressed = false;
                state.cancel_touch_hold();
                state.touch_generation = state.touch_generation.wrapping_add(1);
                let contact = state.touch.take();
                drop(state);
                if let Some(contact) = contact {
                    end_touch_contact(&client, contact, true);
                }
            }
            _ => {}
        }
    }
}

fn linux_button_to_gpui(button: u32) -> Option<MouseButton> {
    // These values are coming from <linux/input-event-codes.h>.
    const BTN_LEFT: u32 = 0x110;
    const BTN_RIGHT: u32 = 0x111;
    const BTN_MIDDLE: u32 = 0x112;
    const BTN_SIDE: u32 = 0x113;
    const BTN_EXTRA: u32 = 0x114;
    const BTN_FORWARD: u32 = 0x115;
    const BTN_BACK: u32 = 0x116;

    Some(match button {
        BTN_LEFT => MouseButton::Left,
        BTN_RIGHT => MouseButton::Right,
        BTN_MIDDLE => MouseButton::Middle,
        BTN_BACK | BTN_SIDE => MouseButton::Navigate(NavigationDirection::Back),
        BTN_FORWARD | BTN_EXTRA => MouseButton::Navigate(NavigationDirection::Forward),
        _ => return None,
    })
}

impl Dispatch<wl_pointer::WlPointer, ()> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        wl_pointer: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();

        match event {
            wl_pointer::Event::Enter {
                serial,
                surface,
                surface_x,
                surface_y,
                ..
            } => {
                let position = point(px(surface_x as f32), px(surface_y as f32));
                state.serial_tracker.update(SerialKind::MouseEnter, serial);
                state.mouse_location = Some(position);
                state.button_pressed = None;
                state.pending_window_move = None;

                if let Some(window) = get_window(&mut state, &surface.id()) {
                    state.mouse_focused_window = Some(window.clone());

                    if state.enter_token.is_some() {
                        state.enter_token = None;
                    }
                    state.restore_cursor_after_hide();
                    if let Some(style) = state.cursor_style {
                        if let Some(cursor_shape_device) = &state.cursor_shape_device {
                            cursor_shape_device.set_shape(serial, to_shape(style));
                        } else {
                            let scale = window.primary_output_scale();
                            state.cursor.set_icon(
                                wl_pointer,
                                serial,
                                cursor_style_to_icon_names(style),
                                scale,
                            );
                        }
                    }
                    let modifiers = state.modifiers;
                    drop(state);
                    window.set_hovered(true);
                    // No Motion follows Enter unless the pointer keeps moving, so synthesize
                    // a MouseMove to establish hover at the entry position.
                    window.handle_input(PlatformInput::MouseMove(MouseMoveEvent {
                        position,
                        pressed_button: None,
                        modifiers,
                    }));
                }
            }
            wl_pointer::Event::Leave { .. } => {
                state.momentum_generation = state.momentum_generation.wrapping_add(1);
                if state.button_pressed == Some(MouseButton::Left) {
                    state.try_start_staged_file_drag(point(px(-1.0), px(-1.0)), false);
                }
                if let Some(focused_window) = state.mouse_focused_window.clone() {
                    let input = PlatformInput::MouseExited(MouseExitEvent {
                        position: state.mouse_location.unwrap(),
                        pressed_button: state.button_pressed,
                        modifiers: state.modifiers,
                    });
                    state.mouse_focused_window = None;
                    state.mouse_location = None;
                    state.button_pressed = None;
                    state.pending_window_move = None;
                    state.cursor_hidden_window = None;

                    drop(state);
                    focused_window.handle_input(input);
                    focused_window.set_hovered(false);
                }
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                if state.mouse_focused_window.is_none() {
                    return;
                }
                state.mouse_location = Some(point(px(surface_x as f32), px(surface_y as f32)));
                state.restore_cursor_after_hide();

                if let Some(window) = state.mouse_focused_window.clone() {
                    let start_move = state.pending_window_move.take().and_then(|pending| {
                        if state.button_pressed == Some(MouseButton::Left)
                            && pending.window.ptr_eq(&window)
                            && moved_past_window_drag_threshold(
                                pending.position,
                                state.mouse_location.unwrap(),
                            )
                        {
                            Some(pending.serial)
                        } else {
                            state.pending_window_move = Some(pending);
                            None
                        }
                    });
                    if window.is_blocked() {
                        let default_style = CursorStyle::Arrow;
                        if state.cursor_style != Some(default_style) {
                            let serial = state.serial_tracker.get(SerialKind::MouseEnter);
                            state.cursor_style = Some(default_style);

                            if let Some(cursor_shape_device) = &state.cursor_shape_device {
                                cursor_shape_device.set_shape(serial, to_shape(default_style));
                            } else {
                                // cursor-shape-v1 isn't supported, set the cursor using a surface.
                                let wl_pointer = state
                                    .wl_pointer
                                    .clone()
                                    .expect("window is focused by pointer");
                                let scale = window.primary_output_scale();
                                state.cursor.set_icon(
                                    &wl_pointer,
                                    serial,
                                    cursor_style_to_icon_names(default_style),
                                    scale,
                                );
                            }
                        }
                    }
                    if state
                        .keyboard_focused_window
                        .as_ref()
                        .is_some_and(|keyboard_window| window.ptr_eq(keyboard_window))
                    {
                        state.enter_token = None;
                    }
                    let input = PlatformInput::MouseMove(MouseMoveEvent {
                        position: state.mouse_location.unwrap(),
                        pressed_button: state.button_pressed,
                        modifiers: state.modifiers,
                    });
                    drop(state);
                    if let Some(serial) = start_move {
                        window.start_window_move_with_serial(serial);
                    }
                    window.handle_input(input);
                    client.borrow_mut().try_start_staged_file_drag(
                        point(px(surface_x as f32), px(surface_y as f32)),
                        false,
                    );
                }
            }
            wl_pointer::Event::Button {
                serial,
                button,
                state: WEnum::Value(button_state),
                ..
            } => {
                // Record presses only. Requests referencing this serial (popup grabs,
                // interactive moves) are declined when given a release serial.
                if button_state == wl_pointer::ButtonState::Pressed {
                    state.momentum_generation = state.momentum_generation.wrapping_add(1);
                    state.serial_tracker.update(SerialKind::MousePress, serial);
                    if button == 0x110 {
                        state.press_serial = Some(serial);
                    }
                }
                let button = linux_button_to_gpui(button);
                let Some(button) = button else { return };
                if state.mouse_focused_window.is_none() {
                    return;
                }
                match button_state {
                    wl_pointer::ButtonState::Pressed => {
                        if let Some(window) = state.keyboard_focused_window.clone() {
                            if state.composing && state.text_input.is_some() {
                                drop(state);
                                // text_input_v3 don't have something like a reset function
                                this.disable_ime();
                                this.enable_ime();
                                window.handle_ime(ImeInput::UnmarkText);
                                state = client.borrow_mut();
                            } else if let (Some(text), Some(compose)) =
                                (state.pre_edit_text.take(), state.compose_state.as_mut())
                            {
                                compose.reset();
                                drop(state);
                                window.handle_ime(ImeInput::InsertText(text));
                                state = client.borrow_mut();
                            }
                        }
                        let position = state.mouse_location.unwrap();
                        state.count_click(button, position);

                        state.button_pressed = Some(button);
                        state.pending_window_move = None;

                        if let Some(window) = state.mouse_focused_window.clone() {
                            let position = state.mouse_location.unwrap();
                            let input = PlatformInput::MouseDown(MouseDownEvent {
                                button,
                                position,
                                modifiers: state.modifiers,
                                click_count: state.click.current_count,
                                first_mouse: state.enter_token.take().is_some(),
                            });
                            drop(state);
                            if button == gpui::MouseButton::Left
                                && window.hit_test_window_control()
                                    == Some(gpui::WindowControlArea::Drag)
                            {
                                client.borrow_mut().pending_window_move = Some(PendingWindowMove {
                                    window: window.clone(),
                                    position,
                                    serial,
                                });
                            }
                            window.handle_input(input);
                        }
                    }
                    wl_pointer::ButtonState::Released => {
                        state.button_pressed = None;
                        state.staged_file_drag = None;
                        state.press_serial = None;
                        state.pending_window_move = None;

                        if let Some(window) = state.mouse_focused_window.clone() {
                            let input = PlatformInput::MouseUp(MouseUpEvent {
                                button,
                                position: state.mouse_location.unwrap(),
                                modifiers: state.modifiers,
                                click_count: state.click.current_count,
                            });
                            drop(state);
                            window.handle_input(input);
                        }
                    }
                    _ => {}
                }
            }

            // Axis Events
            wl_pointer::Event::AxisSource {
                axis_source: WEnum::Value(axis_source),
            } => {
                state.axis_source = axis_source;
            }
            wl_pointer::Event::AxisStop { .. } if state.axis_source == AxisSource::Finger => {
                // The fingers left the touchpad; the frame decides whether the
                // scroll glides on (rmac, docs/decisions/0013).
                state.axis_stop_pending = true;
                state.scroll_event_received = true;
            }
            wl_pointer::Event::Axis {
                axis: WEnum::Value(axis),
                value,
                ..
            } => {
                if state.axis_source == AxisSource::Wheel {
                    return;
                }
                let axis = if state.modifiers.shift {
                    wl_pointer::Axis::HorizontalScroll
                } else {
                    axis
                };
                let axis_modifier = match axis {
                    wl_pointer::Axis::VerticalScroll => state.vertical_modifier,
                    wl_pointer::Axis::HorizontalScroll => state.horizontal_modifier,
                    _ => 1.0,
                };
                state.scroll_event_received = true;
                let scroll_delta = state
                    .continuous_scroll_delta
                    .get_or_insert(point(px(0.0), px(0.0)));
                let modifier = 3.0;
                match axis {
                    wl_pointer::Axis::VerticalScroll => {
                        scroll_delta.y += px(value as f32 * modifier * axis_modifier);
                    }
                    wl_pointer::Axis::HorizontalScroll => {
                        scroll_delta.x += px(value as f32 * modifier * axis_modifier);
                    }
                    _ => unreachable!(),
                }
            }
            wl_pointer::Event::AxisDiscrete {
                axis: WEnum::Value(axis),
                discrete,
            } => {
                state.scroll_event_received = true;
                let axis = if state.modifiers.shift {
                    wl_pointer::Axis::HorizontalScroll
                } else {
                    axis
                };
                let axis_modifier = match axis {
                    wl_pointer::Axis::VerticalScroll => state.vertical_modifier,
                    wl_pointer::Axis::HorizontalScroll => state.horizontal_modifier,
                    _ => 1.0,
                };

                let scroll_delta = state.discrete_scroll_delta.get_or_insert(point(0.0, 0.0));
                match axis {
                    wl_pointer::Axis::VerticalScroll => {
                        scroll_delta.y += discrete as f32 * axis_modifier * SCROLL_LINES;
                    }
                    wl_pointer::Axis::HorizontalScroll => {
                        scroll_delta.x += discrete as f32 * axis_modifier * SCROLL_LINES;
                    }
                    _ => unreachable!(),
                }
            }
            wl_pointer::Event::AxisValue120 {
                axis: WEnum::Value(axis),
                value120,
            } => {
                state.scroll_event_received = true;
                let axis = if state.modifiers.shift {
                    wl_pointer::Axis::HorizontalScroll
                } else {
                    axis
                };
                let axis_modifier = match axis {
                    wl_pointer::Axis::VerticalScroll => state.vertical_modifier,
                    wl_pointer::Axis::HorizontalScroll => state.horizontal_modifier,
                    _ => unreachable!(),
                };

                let scroll_delta = state.discrete_scroll_delta.get_or_insert(point(0.0, 0.0));
                let wheel_percent = value120 as f32 / 120.0;
                match axis {
                    wl_pointer::Axis::VerticalScroll => {
                        scroll_delta.y += wheel_percent * axis_modifier * SCROLL_LINES;
                    }
                    wl_pointer::Axis::HorizontalScroll => {
                        scroll_delta.x += wheel_percent * axis_modifier * SCROLL_LINES;
                    }
                    _ => unreachable!(),
                }
            }
            wl_pointer::Event::Frame if state.scroll_event_received => {
                state.scroll_event_received = false;
                let continuous = state.continuous_scroll_delta.take();
                let discrete = state.discrete_scroll_delta.take();
                let stopped = std::mem::take(&mut state.axis_stop_pending);
                if continuous.is_some() || discrete.is_some() {
                    // New scrolling cancels a glide in progress.
                    state.momentum_generation = state.momentum_generation.wrapping_add(1);
                }
                if let Some(continuous) = continuous
                    && state.axis_source == AxisSource::Finger
                {
                    track_finger_velocity(&mut state, continuous);
                }
                if stopped {
                    start_momentum(&mut state, None);
                }
                if let Some(continuous) = continuous {
                    if let Some(window) = state.mouse_focused_window.clone() {
                        let input = PlatformInput::ScrollWheel(ScrollWheelEvent {
                            position: state.mouse_location.unwrap(),
                            delta: ScrollDelta::Pixels(continuous),
                            modifiers: state.modifiers,
                            touch_phase: TouchPhase::Moved,
                        });
                        drop(state);
                        window.handle_input(input);
                    }
                } else if let Some(discrete) = discrete
                    && let Some(window) = state.mouse_focused_window.clone()
                {
                    let input = PlatformInput::ScrollWheel(ScrollWheelEvent {
                        position: state.mouse_location.unwrap(),
                        delta: ScrollDelta::Lines(discrete),
                        modifiers: state.modifiers,
                        touch_phase: TouchPhase::Moved,
                    });
                    drop(state);
                    window.handle_input(input);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<zwp_pointer_gestures_v1::ZwpPointerGesturesV1, ()> for WaylandClientStatePtr {
    fn event(
        _this: &mut Self,
        _: &zwp_pointer_gestures_v1::ZwpPointerGesturesV1,
        _: <zwp_pointer_gestures_v1::ZwpPointerGesturesV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The gesture manager doesn't generate events
    }
}

impl Dispatch<zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1, ()>
    for WaylandClientStatePtr
{
    fn event(
        this: &mut Self,
        _: &zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1,
        event: <zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use gpui::PinchEvent;

        let client = this.get_client();
        let mut state = client.borrow_mut();

        let Some(window) = state.mouse_focused_window.clone() else {
            return;
        };

        match event {
            zwp_pointer_gesture_pinch_v1::Event::Begin {
                serial: _,
                time: _,
                surface: _,
                fingers: _,
            } => {
                state.pinch_scale = 1.0;
                let input = PlatformInput::Pinch(PinchEvent {
                    position: state.mouse_location.unwrap_or(point(px(0.0), px(0.0))),
                    delta: 0.0,
                    modifiers: state.modifiers,
                    phase: TouchPhase::Started,
                });
                drop(state);
                window.handle_input(input);
            }
            zwp_pointer_gesture_pinch_v1::Event::Update { time: _, scale, .. } => {
                let new_absolute_scale = scale as f32;
                let previous_scale = state.pinch_scale;
                let zoom_delta = new_absolute_scale - previous_scale;
                state.pinch_scale = new_absolute_scale;

                let input = PlatformInput::Pinch(PinchEvent {
                    position: state.mouse_location.unwrap_or(point(px(0.0), px(0.0))),
                    delta: zoom_delta,
                    modifiers: state.modifiers,
                    phase: TouchPhase::Moved,
                });
                drop(state);
                window.handle_input(input);
            }
            zwp_pointer_gesture_pinch_v1::Event::End {
                serial: _,
                time: _,
                cancelled: _,
            } => {
                state.pinch_scale = 1.0;
                let input = PlatformInput::Pinch(PinchEvent {
                    position: state.mouse_location.unwrap_or(point(px(0.0), px(0.0))),
                    delta: 0.0,
                    modifiers: state.modifiers,
                    phase: TouchPhase::Ended,
                });
                drop(state);
                window.handle_input(input);
            }
            _ => {}
        }
    }
}

impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, ObjectId> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        _: &wp_fractional_scale_v1::WpFractionalScaleV1,
        event: <wp_fractional_scale_v1::WpFractionalScaleV1 as Proxy>::Event,
        surface_id: &ObjectId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();

        let Some(window) = get_window(&mut state, surface_id) else {
            return;
        };

        drop(state);
        window.handle_fractional_scale_event(event);
    }
}

impl Dispatch<zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1, ObjectId>
    for WaylandClientStatePtr
{
    fn event(
        this: &mut Self,
        _: &zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1,
        event: zxdg_toplevel_decoration_v1::Event,
        surface_id: &ObjectId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();
        let Some(window) = get_window(&mut state, surface_id) else {
            return;
        };

        drop(state);
        window.handle_toplevel_decoration_event(event);
    }
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        _: &wl_data_device::WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();

        match event {
            // Clipboard
            wl_data_device::Event::DataOffer { id: data_offer } => {
                state.data_offers.push(DataOffer::new(data_offer));
                if state.data_offers.len() > 2 {
                    // At most we store a clipboard offer and a drag and drop offer.
                    state.data_offers.remove(0).inner.destroy();
                }
            }
            wl_data_device::Event::Selection { id: data_offer } => {
                if let Some(offer) = data_offer {
                    let offer = state
                        .data_offers
                        .iter()
                        .find(|wrapper| wrapper.inner.id() == offer.id());
                    let offer = offer.cloned();
                    state.clipboard.set_offer(offer);
                } else {
                    state.clipboard.set_offer(None);
                }
            }

            // Drag and drop
            wl_data_device::Event::Enter {
                serial,
                surface,
                x,
                y,
                id: data_offer,
            } => {
                state.serial_tracker.update(SerialKind::DataDevice, serial);
                state.drag.action = None;
                if let Some(data_offer) = data_offer {
                    data_offer.accept(serial, Some(FILE_LIST_MIME_TYPE.to_owned()));
                    let Some(drag_window) = get_window(&mut state, &surface.id()) else {
                        return;
                    };

                    data_offer.set_actions(
                        DndAction::Copy | DndAction::Move | DndAction::Ask,
                        if state.modifiers.alt {
                            DndAction::Copy
                        } else {
                            DndAction::Move
                        },
                    );
                    state.drag.data_offer = Some(data_offer.clone());
                    state.drag.window = Some(drag_window.clone());
                    state.drag.position = Point::new(x.into(), y.into());
                    state.drag.paths_ready = false;
                    state.drag.drop_pending = false;
                    let offer_id = data_offer.id();

                    let pipe = Pipe::new().unwrap();
                    data_offer.receive(FILE_LIST_MIME_TYPE.to_string(), unsafe {
                        BorrowedFd::borrow_raw(pipe.write.as_raw_fd())
                    });
                    let fd = pipe.read;
                    drop(pipe.write);

                    let read_task = state.common.background_executor.spawn(async {
                        let buffer = read_fd_with_timeout(fd, PIPE_READ_TIMEOUT)?;
                        let text = String::from_utf8(buffer)?;
                        anyhow::Ok(text)
                    });

                    let this = this.clone();
                    state
                        .common
                        .foreground_executor
                        .spawn(async move {
                            let file_list = match read_task.await {
                                Ok(list) => list,
                                Err(err) => {
                                    log::error!("error reading drag and drop pipe: {err:?}");
                                    String::new()
                                }
                            };

                            let paths: SmallVec<[_; 2]> = file_list
                                .lines()
                                .filter_map(|path| Url::parse(path).log_err())
                                .filter_map(|url| match url.to_file_path() {
                                    Ok(url) => Some(url),
                                    Err(()) => {
                                        log::error!("Failed turn {url:?} into a file path");
                                        None
                                    }
                                })
                                .collect();
                            let client = this.get_client();
                            let mut state = client.borrow_mut();
                            if !state
                                .drag
                                .data_offer
                                .as_ref()
                                .is_some_and(|offer| offer.id() == offer_id)
                            {
                                return; // The pointer already left this offer.
                            }
                            // Prevent dropping text from other programs.
                            if paths.is_empty() {
                                state.drag.data_offer.take().unwrap().destroy();
                                state.drag.window = None;
                                state.drag.paths_ready = false;
                                state.drag.drop_pending = false;
                                return;
                            }
                            let position = state.drag.position;
                            let drop_pending = state.drag.drop_pending;
                            state.drag.paths_ready = true;
                            let source_window = state
                                .file_drag_source
                                .as_ref()
                                .map(|source| (source.window.clone(), source.position));
                            if drop_pending {
                                let offer = state.drag.data_offer.take().unwrap();
                                offer.finish();
                                offer.destroy();
                                state.drag.window = None;
                                state.drag.paths_ready = false;
                                state.drag.drop_pending = false;
                            }

                            drop(state);
                            // GPUI keeps its old in-window drag value until a
                            // release. Clear it before a second window in this
                            // process receives ExternalPaths.
                            if let Some((source_window, source_position)) = source_window {
                                source_window.handle_input(PlatformInput::MouseUp(MouseUpEvent {
                                    button: MouseButton::Left,
                                    position: source_position,
                                    modifiers: Modifiers::default(),
                                    click_count: 1,
                                }));
                            }
                            drag_window.handle_input(PlatformInput::FileDrop(
                                FileDropEvent::Entered {
                                    position,
                                    paths: gpui::ExternalPaths(paths),
                                },
                            ));
                            if drop_pending {
                                drag_window.handle_input(PlatformInput::FileDrop(
                                    FileDropEvent::Submit { position },
                                ));
                            }
                        })
                        .detach();
                }
            }
            wl_data_device::Event::Motion { x, y, .. } => {
                let Some(drag_window) = state.drag.window.clone() else {
                    return;
                };
                let position = Point::new(x.into(), y.into());
                state.drag.position = position;
                if !state.drag.paths_ready {
                    return;
                }

                let input = PlatformInput::FileDrop(FileDropEvent::Pending { position });
                drop(state);
                drag_window.handle_input(input);
            }
            wl_data_device::Event::Leave => {
                if state.drag.drop_pending {
                    return; // Submit after the pending URI read completes.
                }
                let Some(drag_window) = state.drag.window.clone() else {
                    return;
                };
                let paths_ready = state.drag.paths_ready;
                state.drag.data_offer.take().unwrap().destroy();
                state.drag.window = None;
                state.drag.paths_ready = false;
                state.drag.action = None;

                if !paths_ready {
                    return;
                }
                drop(state);
                drag_window.handle_input(PlatformInput::FileDrop(FileDropEvent::Exited {}));
            }
            wl_data_device::Event::Drop => {
                let Some(drag_window) = state.drag.window.clone() else {
                    return;
                };
                if !state.drag.paths_ready {
                    state.drag.drop_pending = true;
                    return;
                }
                let offer = state.drag.data_offer.take().unwrap();
                offer.finish();
                offer.destroy();
                state.drag.window = None;
                state.drag.paths_ready = false;

                let input = PlatformInput::FileDrop(FileDropEvent::Submit {
                    position: state.drag.position,
                });
                drop(state);
                drag_window.handle_input(input);
            }
            _ => {}
        }
    }

    event_created_child!(WaylandClientStatePtr, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, ()),
    ]);
}

impl Dispatch<wl_data_offer::WlDataOffer, ()> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        data_offer: &wl_data_offer::WlDataOffer,
        event: wl_data_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();

        match event {
            wl_data_offer::Event::Offer { mime_type } => {
                if let Some(offer) = state
                    .data_offers
                    .iter_mut()
                    .find(|wrapper| wrapper.inner.id() == data_offer.id())
                {
                    offer.add_mime_type(mime_type);
                }
            }
            wl_data_offer::Event::Action {
                dnd_action: WEnum::Value(action),
            } => {
                state.drag.action = Some(action);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_data_source::WlDataSource, ()> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        data_source: &wl_data_source::WlDataSource,
        event: wl_data_source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let state = client.borrow();

        if state
            .file_drag_source
            .as_ref()
            .is_some_and(|drag| drag.source.id() == data_source.id())
        {
            match event {
                wl_data_source::Event::Send { mime_type, fd } => {
                    let drag = state.file_drag_source.as_ref().unwrap();
                    let payload = match mime_type.as_str() {
                        FILE_LIST_MIME_TYPE => drag.uri_list.clone(),
                        "x-special/gnome-copied-files" => drag.gnome_files.clone(),
                        _ => Vec::new(),
                    };
                    std::thread::spawn(move || {
                        let mut file = std::fs::File::from(fd);
                        if let Err(error) = file.write_all(&payload) {
                            log::warn!("Wayland file drag send failed: {error}");
                        }
                    });
                }
                wl_data_source::Event::Cancelled | wl_data_source::Event::DndFinished => {
                    let drag = state.file_drag_source.as_ref().unwrap();
                    let window = drag.window.clone();
                    let position = drag.position;
                    let modifiers = state.modifiers;
                    let click_count = state.click.current_count;
                    data_source.destroy();
                    drop(state);
                    window.handle_input(PlatformInput::MouseUp(MouseUpEvent {
                        button: MouseButton::Left,
                        position,
                        modifiers,
                        click_count,
                    }));
                    client.borrow_mut().file_drag_source = None;
                }
                _ => {}
            }
            return;
        }

        match event {
            wl_data_source::Event::Send { mime_type, fd } => {
                state.clipboard.send(mime_type, fd);
            }
            wl_data_source::Event::Cancelled => {
                data_source.destroy();
            }
            _ => {}
        }
    }
}

impl Dispatch<zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1, ()>
    for WaylandClientStatePtr
{
    fn event(
        this: &mut Self,
        _: &zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1,
        event: zwp_primary_selection_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();

        match event {
            zwp_primary_selection_device_v1::Event::DataOffer { offer } => {
                let old_offer = state.primary_data_offer.replace(DataOffer::new(offer));
                if let Some(old_offer) = old_offer {
                    old_offer.inner.destroy();
                }
            }
            zwp_primary_selection_device_v1::Event::Selection { id: data_offer } => {
                if data_offer.is_some() {
                    let offer = state.primary_data_offer.clone();
                    state.clipboard.set_primary_offer(offer);
                } else {
                    state.clipboard.set_primary_offer(None);
                }
            }
            _ => {}
        }
    }

    event_created_child!(WaylandClientStatePtr, zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1, [
        zwp_primary_selection_device_v1::EVT_DATA_OFFER_OPCODE => (zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1, ()),
    ]);
}

impl Dispatch<zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1, ()>
    for WaylandClientStatePtr
{
    fn event(
        this: &mut Self,
        _data_offer: &zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1,
        event: zwp_primary_selection_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();

        if let zwp_primary_selection_offer_v1::Event::Offer { mime_type } = event
            && let Some(offer) = state.primary_data_offer.as_mut()
        {
            offer.add_mime_type(mime_type);
        }
    }
}

impl Dispatch<zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1, ()>
    for WaylandClientStatePtr
{
    fn event(
        this: &mut Self,
        selection_source: &zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1,
        event: zwp_primary_selection_source_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let state = client.borrow_mut();

        match event {
            zwp_primary_selection_source_v1::Event::Send { mime_type, fd } => {
                state.clipboard.send_primary(mime_type, fd);
            }
            zwp_primary_selection_source_v1::Event::Cancelled => {
                selection_source.destroy();
            }
            _ => {}
        }
    }
}

impl Dispatch<XdgWmDialogV1, ()> for WaylandClientStatePtr {
    fn event(
        _: &mut Self,
        _: &XdgWmDialogV1,
        _: <XdgWmDialogV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<XdgDialogV1, ()> for WaylandClientStatePtr {
    fn event(
        _state: &mut Self,
        _proxy: &XdgDialogV1,
        _event: <XdgDialogV1 as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

/// rmac: macOS-like momentum after a touchpad scroll, using the same decay as
/// `rmac_ui::scroll` (FEEL_SPEC §D.4).
const MOMENTUM_DECAY_MS: f32 = 325.0;
const MOMENTUM_TICK: Duration = Duration::from_millis(8);
/// Below this lift-off speed (px/ms) the scroll simply stops, like a slow
/// deliberate drag on a Mac trackpad.
const MOMENTUM_MIN_VELOCITY: f32 = 0.08;
/// The glide ends once it moves less than this per tick.
const MOMENTUM_STOP_PX: f32 = 0.25;

fn track_finger_velocity(state: &mut WaylandClientState, delta: Point<Pixels>) {
    let now = Instant::now();
    let delta = point(f32::from(delta.x), f32::from(delta.y));
    let elapsed = state
        .last_finger_scroll
        .map(|last| now.duration_since(last).as_secs_f32() * 1000.0);
    let instant_velocity = |dt: f32| point(delta.x / dt, delta.y / dt);
    state.scroll_velocity = match elapsed {
        // Blend with recent motion so one uneven frame doesn't decide the fling.
        Some(dt) if dt <= 100.0 => {
            let current = instant_velocity(dt.max(4.0));
            point(
                0.6 * current.x + 0.4 * state.scroll_velocity.x,
                0.6 * current.y + 0.4 * state.scroll_velocity.y,
            )
        }
        _ => instant_velocity(16.0),
    };
    state.last_finger_scroll = Some(now);
}

/// The optional fixed target is a lifted touchscreen contact. A touchpad
/// glide instead follows the current pointer focus and location.
fn start_momentum(
    state: &mut WaylandClientState,
    touch_target: Option<(WaylandWindowStatePtr, Point<Pixels>)>,
) {
    let lifted_while_moving = state
        .last_finger_scroll
        .is_some_and(|last| last.elapsed() <= Duration::from_millis(60));
    let velocity = state.scroll_velocity;
    state.last_finger_scroll = None;
    state.scroll_velocity = point(0.0, 0.0);
    if !lifted_while_moving
        || velocity.x.hypot(velocity.y) < MOMENTUM_MIN_VELOCITY
        || (touch_target.is_none() && state.mouse_focused_window.is_none())
    {
        return;
    }
    state.momentum_generation = state.momentum_generation.wrapping_add(1);
    let generation = state.momentum_generation;
    let mut velocity = velocity;
    let mut last_tick = Instant::now();
    let _ = state.loop_handle.insert_source(
        Timer::from_duration(MOMENTUM_TICK),
        move |_, _, this: &mut WaylandClientStatePtr| {
            let client = this.get_client();
            let state = client.borrow();
            if state.momentum_generation != generation {
                return TimeoutAction::Drop;
            }
            let now = Instant::now();
            let dt = now.duration_since(last_tick).as_secs_f32() * 1000.0;
            last_tick = now;
            let decay = (-dt / MOMENTUM_DECAY_MS).exp();
            velocity = point(velocity.x * decay, velocity.y * decay);
            let step = point(velocity.x * dt, velocity.y * dt);
            if step.x.hypot(step.y) < MOMENTUM_STOP_PX {
                return TimeoutAction::Drop;
            }
            let (window, position) = if let Some((window, position)) = &touch_target {
                if !state
                    .windows
                    .values()
                    .any(|candidate| candidate.ptr_eq(window))
                {
                    return TimeoutAction::Drop;
                }
                (window.clone(), *position)
            } else {
                let (Some(window), Some(position)) =
                    (state.mouse_focused_window.clone(), state.mouse_location)
                else {
                    return TimeoutAction::Drop;
                };
                (window, position)
            };
            let modifiers = state.modifiers;
            drop(state);
            window.handle_input(PlatformInput::ScrollWheel(ScrollWheelEvent {
                position,
                delta: ScrollDelta::Pixels(point(px(step.x), px(step.y))),
                modifiers,
                touch_phase: TouchPhase::Moved,
            }));
            TimeoutAction::ToDuration(MOMENTUM_TICK)
        },
    );
}
