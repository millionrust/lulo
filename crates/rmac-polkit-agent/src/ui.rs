//! The password dialog, drawn at design-lab/authorization.html's numbers: a
//! layer-shell overlay centred on the focused output, above every window,
//! holding the keyboard exclusively while it is open. One dialog at a time;
//! the coordinator queues the rest.

use std::f32::consts::TAU;
use std::path::PathBuf;
use std::time::Duration;

use gpui::layer_shell::{KeyboardInteractivity, Layer, LayerShellOptions};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, img, point, px, size, Animation, AnimationExt as _, AnyElement, App, AppContext as _,
    AsyncApp, Bounds, ClickEvent, Context, FocusHandle, FontWeight, Global,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, QuitMode, Render, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, WindowBackgroundAppearance,
    WindowBounds, WindowHandle, WindowKind, WindowOptions,
};
use rmac_ui::DialogButtonKind;

use crate::dbus;
use crate::helper::HelperConfig;
use crate::request::{Coordinator, Dialog, FromUi, Responder, System, ToUi};
use crate::secret::Secret;

/// design-lab/authorization.html (all S).
mod metrics {
    pub const PANEL_WIDTH: f32 = 260.0;
    /// Transparent room on each side for the wrong-password shake.
    pub const SHAKE_ROOM: f32 = 8.0;
    pub const PAD_TOP: f32 = 20.0;
    pub const PAD_X: f32 = 12.0;
    pub const PAD_BOTTOM: f32 = 14.0;
    pub const ICON: f32 = 64.0;
    pub const BADGE: f32 = 28.0;
    pub const TITLE_GAP: f32 = 12.0;
    pub const TITLE_SIZE: f32 = 13.0;
    pub const TITLE_LINE: f32 = 16.0;
    pub const BODY_GAP: f32 = 4.0;
    pub const BODY_SIZE: f32 = 11.0;
    pub const BODY_LINE: f32 = 14.0;
    pub const FIELDS_GAP: f32 = 14.0;
    pub const FIELD_HEIGHT: f32 = 24.0;
    pub const FIELD_SPACING: f32 = 8.0;
    pub const FIELD_TEXT: f32 = 13.0;
    pub const MENU_ROW: f32 = 22.0;
    pub const INFO_GAP: f32 = 6.0;
    pub const BUTTONS_GAP: f32 = 16.0;
    pub const BUTTON_HEIGHT: f32 = 28.0;
    pub const BUTTON_SPACING: f32 = 8.0;
    pub const BULLET: f32 = 6.0;
    pub const BULLET_PITCH: f32 = 9.0;
    pub const SHAKE_AMPLITUDE: f32 = 8.0;
    pub const SHAKE_CYCLES: f32 = 3.0;
    pub const SHAKE_MS: u64 = 400;
    /// Average advance used to estimate wrapped lines, generous so text
    /// never clips: 13 pt semibold and 11 pt regular Inter.
    pub const TITLE_ADVANCE: f32 = 7.4;
    pub const BODY_ADVANCE: f32 = 6.2;

    pub const fn content_width() -> f32 {
        PANEL_WIDTH - 2.0 * PAD_X
    }
}

const NAMESPACE: &str = "rmac-polkit-agent";
const APP_ID: &str = "org.rmac.PolkitAgent";

fn estimated_lines(text: &str, advance: f32) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    let width = text.chars().count() as f32 * advance;
    (width / metrics::content_width()).ceil().max(1.0)
}

fn instruction(dialog: &Dialog) -> &'static str {
    if dialog.administrator_needed {
        "Enter an administrator’s name and password to allow this."
    } else {
        "Enter your password to allow this."
    }
}

/// A theme icon polkit named, when it exists as a PNG (raster only: the
/// name was already checked to be a plain file name).
fn icon_path(name: &str) -> Option<PathBuf> {
    let candidates = [
        format!("/usr/share/icons/hicolor/48x48/apps/{name}.png"),
        format!("/usr/share/icons/hicolor/64x64/apps/{name}.png"),
        format!("/usr/share/icons/hicolor/32x32/apps/{name}.png"),
        format!("/usr/share/pixmaps/{name}.png"),
    ];
    candidates
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Field {
    User,
    Password,
}

pub struct AuthDialog {
    dialog: Dialog,
    responder: Responder,
    title: SharedString,
    badge: Option<PathBuf>,
    selected: usize,
    menu_open: bool,
    secret: Secret,
    placeholder: SharedString,
    echo: bool,
    info: Option<(SharedString, bool)>,
    busy: bool,
    shakes: u32,
    user_focus: FocusHandle,
    password_focus: FocusHandle,
}

impl AuthDialog {
    fn new(
        dialog: Dialog,
        responder: Responder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let password_focus = cx.focus_handle();
        password_focus.focus(window, cx);
        let title = SharedString::from(format!("{} wants to make changes.", dialog.app_name));
        let badge = dialog.icon_name.as_deref().and_then(icon_path);
        Self {
            dialog,
            responder,
            title,
            badge,
            selected: 0,
            menu_open: false,
            secret: Secret::new(),
            placeholder: "Password".into(),
            echo: false,
            info: None,
            busy: false,
            shakes: 0,
            user_focus: cx.focus_handle(),
            password_focus,
        }
    }

    fn several_identities(&self) -> bool {
        self.dialog.identities.len() > 1
    }

    /// The panel's height for the current content.
    fn panel_height(&self) -> f32 {
        use metrics::*;
        let mut height = PAD_TOP + ICON + TITLE_GAP;
        height += TITLE_LINE * estimated_lines(&self.title, TITLE_ADVANCE);
        height += BODY_GAP;
        height += BODY_LINE * estimated_lines(&self.dialog.message, BODY_ADVANCE);
        height += BODY_LINE * estimated_lines(instruction(&self.dialog), BODY_ADVANCE);
        height += FIELDS_GAP + FIELD_HEIGHT + FIELD_SPACING + FIELD_HEIGHT;
        if self.menu_open {
            height += FIELD_SPACING + MENU_ROW * self.dialog.identities.len() as f32;
        }
        if let Some((info, _)) = &self.info {
            height += INFO_GAP + BODY_LINE * estimated_lines(info, BODY_ADVANCE);
        }
        height + BUTTONS_GAP + BUTTON_HEIGHT + PAD_BOTTOM
    }

    fn window_size(&self) -> gpui::Size<gpui::Pixels> {
        size(
            px(metrics::PANEL_WIDTH + 2.0 * metrics::SHAKE_ROOM),
            px(self.panel_height().ceil()),
        )
    }

    fn relayout(&self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = self.window_size();
        if window.bounds().size != wanted {
            window.resize(wanted);
        }
        cx.notify();
    }

    fn focused(&self, window: &Window) -> Field {
        if self.user_focus.is_focused(window) {
            Field::User
        } else {
            Field::Password
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.menu_open = false;
        self.busy = true;
        self.info = None;
        let secret = self.secret.take();
        self.responder.send(FromUi::Submit {
            identity: self.selected,
            secret,
        });
        self.relayout(window, cx);
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.secret.clear();
        self.responder.send(FromUi::Cancel);
        cx.notify();
    }

    fn select(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.dialog.identities.len() {
            return;
        }
        self.menu_open = false;
        if index != self.selected {
            self.selected = index;
            self.info = None;
            self.responder.send(FromUi::SelectIdentity(index));
        }
        self.password_focus.focus(window, cx);
        self.relayout(window, cx);
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        cx.stop_propagation();
        match keystroke.key.as_str() {
            "escape" => {
                if self.menu_open {
                    self.menu_open = false;
                    self.relayout(window, cx);
                } else {
                    self.cancel(cx);
                }
            }
            "enter" | "kp_enter" => {
                if self.menu_open {
                    self.select(self.selected, window, cx);
                } else {
                    self.submit(window, cx);
                }
            }
            "tab" => {
                if self.several_identities() && self.focused(window) == Field::Password {
                    self.user_focus.focus(window, cx);
                } else {
                    self.menu_open = false;
                    self.password_focus.focus(window, cx);
                }
                self.relayout(window, cx);
            }
            "up" | "down" if self.focused(window) == Field::User => {
                let count = self.dialog.identities.len();
                let next = if keystroke.key == "up" {
                    (self.selected + count - 1) % count
                } else {
                    (self.selected + 1) % count
                };
                self.selected = next;
                self.responder.send(FromUi::SelectIdentity(next));
                cx.notify();
            }
            "space" if self.focused(window) == Field::User => {
                self.menu_open = !self.menu_open;
                self.relayout(window, cx);
            }
            "backspace" if self.focused(window) == Field::Password => {
                if modifiers.alt || modifiers.platform || modifiers.control {
                    self.secret.clear();
                } else {
                    self.secret.backspace();
                }
                cx.notify();
            }
            _ => {
                if self.focused(window) != Field::Password
                    || modifiers.control
                    || modifiers.platform
                    || modifiers.function
                {
                    return;
                }
                if let Some(text) = keystroke.key_char.as_deref() {
                    // A full field ignores more input, as macOS does.
                    let _ = self.secret.push_str(text);
                    cx.notify();
                }
            }
        }
    }

    fn handle(&mut self, message: ToUi, window: &mut Window, cx: &mut Context<Self>) {
        match message {
            ToUi::Prompt {
                placeholder, echo, ..
            } => {
                self.placeholder = placeholder.into();
                self.echo = echo;
                cx.notify();
            }
            ToUi::Info { text, error, .. } => {
                self.info = (!text.is_empty()).then(|| (text.into(), error));
                self.relayout(window, cx);
            }
            ToUi::Busy { busy, .. } => {
                self.busy = busy;
                cx.notify();
            }
            ToUi::Retry { .. } => {
                self.secret.clear();
                self.busy = false;
                self.shakes += 1;
                self.password_focus.focus(window, cx);
                cx.notify();
            }
            ToUi::Open { .. } | ToUi::Close { .. } => {}
        }
    }

    fn lock_icon(&self) -> AnyElement {
        let tokens = rmac_ui::theme::current();
        let glyph = tokens.colors.text_secondary.hsla();
        div()
            .relative()
            .size(px(metrics::ICON))
            .flex_none()
            .rounded(px(rmac_ui::mac::radius_card()))
            .bg(rmac_ui::mac::icon_plate())
            .child(
                // Shackle: a ring whose lower half the body covers.
                div()
                    .absolute()
                    .left(px(21.0))
                    .top(px(12.0))
                    .size(px(22.0))
                    .rounded_full()
                    .border_4()
                    .border_color(glyph),
            )
            .child(
                div()
                    .absolute()
                    .left(px(16.0))
                    .top(px(26.0))
                    .w(px(32.0))
                    .h(px(26.0))
                    .rounded(px(rmac_ui::mac::radius_control()))
                    .bg(glyph),
            )
            .when_some(self.badge.clone(), |icon, badge| {
                icon.child(
                    img(badge)
                        .absolute()
                        .right(px(-6.0))
                        .bottom(px(-6.0))
                        .size(px(metrics::BADGE)),
                )
            })
            .into_any_element()
    }

    fn field_shell(&self, focused: bool) -> gpui::Div {
        let tokens = rmac_ui::theme::current();
        div()
            .w(px(metrics::content_width()))
            .h(px(metrics::FIELD_HEIGHT))
            .flex()
            .items_center()
            .px(px(7.0))
            .rounded(px(rmac_ui::mac::radius_control()))
            .bg(rmac_ui::mac::field_fill())
            .border_1()
            .border_color(if focused {
                tokens.colors.focus_ring.hsla()
            } else {
                tokens.colors.separator.hsla()
            })
            .text_size(px(metrics::FIELD_TEXT))
    }

    fn user_field(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let tokens = rmac_ui::theme::current();
        let name = self
            .dialog
            .identities
            .get(self.selected)
            .map(|identity| identity.display_name.clone())
            .unwrap_or_default();
        let several = self.several_identities();
        let focused = several && self.user_focus.is_focused(window);
        let mut field = self
            .field_shell(focused)
            .id("user-name")
            .role(if several {
                Role::ComboBox
            } else {
                Role::TextInput
            })
            .aria_label("User Name")
            .text_color(tokens.colors.text.hsla())
            .child(div().flex_1().truncate().child(name));
        if several {
            field = field
                .track_focus(&self.user_focus)
                .child(
                    div()
                        .text_color(tokens.colors.text_secondary.hsla())
                        .child("⌃⌄"),
                )
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.menu_open = !this.menu_open;
                    this.user_focus.focus(window, cx);
                    this.relayout(window, cx);
                }));
        }
        field.into_any_element()
    }

    fn identity_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = rmac_ui::theme::current();
        let rows = self
            .dialog
            .identities
            .iter()
            .enumerate()
            .map(|(index, identity)| {
                let selected = index == self.selected;
                div()
                    .id(("identity", index))
                    .role(Role::ListBoxOption)
                    .aria_label(SharedString::from(identity.display_name.clone()))
                    .aria_selected(selected)
                    .h(px(metrics::MENU_ROW))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .rounded(px(rmac_ui::mac::radius_menu_item()))
                    .text_size(px(metrics::FIELD_TEXT))
                    .text_color(if selected {
                        tokens.colors.on_accent.hsla()
                    } else {
                        tokens.colors.text.hsla()
                    })
                    .when(selected, |row| row.bg(tokens.colors.accent.hsla()))
                    .child(identity.display_name.clone())
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.select(index, window, cx);
                    }))
            })
            .collect::<Vec<_>>();
        div()
            .id("identities")
            .mt(px(metrics::FIELD_SPACING))
            .w(px(metrics::content_width()))
            .role(Role::ListBox)
            .aria_label("Administrators")
            .rounded(px(rmac_ui::mac::radius_menu()))
            .bg(rmac_ui::mac::material_popover())
            .children(rows)
            .into_any_element()
    }

    fn password_field(&self, window: &Window) -> AnyElement {
        let tokens = rmac_ui::theme::current();
        let focused = self.password_focus.is_focused(window);
        let count = self.secret.character_count();
        let content: AnyElement = if count == 0 {
            div()
                .text_color(tokens.colors.text_tertiary.hsla())
                .child(self.placeholder.clone())
                .into_any_element()
        } else if self.echo {
            // Only a PAM_PROMPT_ECHO_ON answer (never a password) is drawn
            // as text.
            let shown = self
                .secret
                .expose(|bytes| String::from_utf8_lossy(bytes).into_owned());
            div()
                .text_color(tokens.colors.text.hsla())
                .truncate()
                .child(shown)
                .into_any_element()
        } else {
            let visible = count
                .min(((metrics::content_width() - 20.0) / metrics::BULLET_PITCH).floor() as usize);
            div()
                .flex()
                .items_center()
                .gap(px(metrics::BULLET_PITCH - metrics::BULLET))
                .children((0..visible).map(|_| {
                    div()
                        .size(px(metrics::BULLET))
                        .rounded_full()
                        .bg(tokens.colors.text.hsla())
                }))
                .into_any_element()
        };
        self.field_shell(focused)
            .id("password")
            .track_focus(&self.password_focus)
            .role(if self.echo {
                Role::TextInput
            } else {
                Role::PasswordInput
            })
            .aria_label(self.placeholder.clone())
            .child(content)
            .into_any_element()
    }
}

impl Render for AuthDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = rmac_ui::theme::current();
        let mut body = Vec::new();
        if !self.dialog.message.is_empty() {
            body.push(div().child(self.dialog.message.clone()));
        }
        body.push(div().child(instruction(&self.dialog)));

        let cancel = rmac_ui::dialog_button("cancel", "Cancel", DialogButtonKind::Normal)
            .flex_1()
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.cancel(cx)));
        let confirm = rmac_ui::dialog_button(
            "confirm",
            SharedString::from(self.dialog.confirm_label.clone()),
            DialogButtonKind::Primary,
        )
        .flex_1()
        .disabled(self.busy)
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.submit(window, cx)));

        let panel = div()
            .id("authorization")
            .role(Role::Dialog)
            .aria_label(self.title.clone())
            .relative()
            .w(px(metrics::PANEL_WIDTH))
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .pt(px(metrics::PAD_TOP))
            .px(px(metrics::PAD_X))
            .pb(px(metrics::PAD_BOTTOM))
            .rounded(px(rmac_ui::mac::radius_window()))
            .bg(rmac_ui::mac::sheet())
            .border_1()
            .border_color(tokens.colors.separator.hsla())
            .text_color(tokens.colors.text.hsla())
            .font_family(rmac_ui::UI_FONT)
            .on_key_down(cx.listener(Self::key_down))
            .child(self.lock_icon())
            .child(
                div()
                    .mt(px(metrics::TITLE_GAP))
                    .w(px(metrics::content_width()))
                    .text_center()
                    .text_size(px(metrics::TITLE_SIZE))
                    .line_height(px(metrics::TITLE_LINE))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.title.clone()),
            )
            .child(
                div()
                    .mt(px(metrics::BODY_GAP))
                    .w(px(metrics::content_width()))
                    .text_center()
                    .text_size(px(metrics::BODY_SIZE))
                    .line_height(px(metrics::BODY_LINE))
                    .text_color(tokens.colors.text_secondary.hsla())
                    .children(body),
            )
            .child(
                div()
                    .mt(px(metrics::FIELDS_GAP))
                    .child(self.user_field(window, cx)),
            )
            .when(self.menu_open, |panel| panel.child(self.identity_menu(cx)))
            .child(
                div()
                    .mt(px(metrics::FIELD_SPACING))
                    .child(self.password_field(window)),
            )
            .when_some(self.info.clone(), |panel, (info, error)| {
                panel.child(
                    div()
                        .id("info")
                        .role(Role::Alert)
                        .aria_label(info.clone())
                        .mt(px(metrics::INFO_GAP))
                        .w(px(metrics::content_width()))
                        .text_center()
                        .text_size(px(metrics::BODY_SIZE))
                        .line_height(px(metrics::BODY_LINE))
                        .text_color(if error {
                            rmac_ui::mac::system_red()
                        } else {
                            tokens.colors.text_secondary.hsla()
                        })
                        .child(info),
                )
            })
            .child(div().flex_1())
            .child(
                div()
                    .mt(px(metrics::BUTTONS_GAP))
                    .w(px(metrics::content_width()))
                    .flex()
                    .gap(px(metrics::BUTTON_SPACING))
                    .child(cancel)
                    .child(confirm),
            );

        let shakes = self.shakes;
        div()
            .size_full()
            .px(px(metrics::SHAKE_ROOM))
            .child(if shakes == 0 {
                panel.into_any_element()
            } else {
                panel
                    .with_animation(
                        ("shake", shakes),
                        Animation::new(Duration::from_millis(metrics::SHAKE_MS)),
                        |panel, progress| {
                            let phase = progress * metrics::SHAKE_CYCLES * TAU;
                            panel.left(px(metrics::SHAKE_AMPLITUDE * phase.sin()))
                        },
                    )
                    .into_any_element()
            })
    }
}

#[derive(Default)]
struct Host {
    open: Option<(String, WindowHandle<AuthDialog>)>,
}

impl Global for Host {}

fn window_options(dialog_size: gpui::Size<gpui::Pixels>) -> WindowOptions {
    WindowOptions {
        titlebar: None,
        focus: true,
        show: true,
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(0.0), px(0.0)),
            size: dialog_size,
        })),
        app_id: Some(APP_ID.to_owned()),
        window_background: WindowBackgroundAppearance::Transparent,
        // No anchor: the compositor centres the surface on the focused
        // output. Overlay is above every window, fullscreen ones included.
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
    }
}

fn close_open(cx: &mut App) {
    if let Some((_, handle)) = cx.global_mut::<Host>().open.take() {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    }
}

fn handle(message: ToUi, cx: &mut App) {
    match message {
        ToUi::Open { dialog, responder } => {
            close_open(cx);
            let cookie = dialog.cookie.clone();
            let failed = responder.clone();
            let mut initial = None;
            let opened = cx.open_window(
                window_options(size(px(metrics::PANEL_WIDTH), px(320.0))),
                |window, cx| {
                    window.set_window_title("Authentication");
                    rmac_ui::prepare_surface_window(window, cx);
                    let view = cx.new(|cx| AuthDialog::new(dialog, responder, window, cx));
                    initial = Some(view.read(cx).window_size());
                    view
                },
            );
            match opened {
                Ok(handle) => {
                    if let Some(wanted) = initial {
                        let _ = handle.update(cx, |_, window, _| window.resize(wanted));
                    }
                    cx.global_mut::<Host>().open = Some((cookie, handle));
                }
                Err(error) => {
                    eprintln!("could not open the authentication dialog: {error}");
                    failed.send(FromUi::Cancel);
                }
            }
        }
        ToUi::Close { cookie } => {
            let matches = cx
                .global::<Host>()
                .open
                .as_ref()
                .is_some_and(|(open, _)| *open == cookie);
            if matches {
                close_open(cx);
            }
        }
        ToUi::Prompt { ref cookie, .. }
        | ToUi::Info { ref cookie, .. }
        | ToUi::Busy { ref cookie, .. }
        | ToUi::Retry { ref cookie } => {
            let handle = cx
                .global::<Host>()
                .open
                .as_ref()
                .filter(|(open, _)| open == cookie)
                .map(|(_, handle)| *handle);
            if let Some(handle) = handle {
                let _ = handle.update(cx, |view, window, cx| view.handle(message, window, cx));
            }
        }
    }
}

/// Run the agent: D-Bus on its own thread, the dialog on GPUI's.
pub fn run() -> Result<(), String> {
    crate::harden_process();
    let (ui_tx, ui_rx) = async_channel::unbounded();
    let coordinator = Coordinator::new(ui_tx, HelperConfig::system(), Box::new(System));
    std::thread::Builder::new()
        .name("polkit-agent-bus".into())
        .spawn(move || {
            let result = async_io::block_on(async {
                let connection = zbus::Connection::system()
                    .await
                    .map_err(|error| format!("system bus: {error}"))?;
                let session = dbus::session_id(&connection)
                    .await
                    .ok_or_else(|| "no graphical login session to serve".to_owned())?;
                dbus::serve(&connection, dbus::session_subject(&session), coordinator)
                    .await
                    .map_err(|error| format!("polkit registration: {error}"))
            });
            match result {
                Ok(()) => eprintln!("the system bus connection closed"),
                Err(error) => eprintln!("rmac-polkit-agent: {error}"),
            }
            std::process::exit(1);
        })
        .map_err(|error| error.to_string())?;

    rmac_ui::application()
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx: &mut App| {
            rmac_ui::init_application(cx);
            cx.set_global(Host::default());
            cx.spawn(async move |cx: &mut AsyncApp| {
                while let Ok(message) = ui_rx.recv().await {
                    cx.update(|cx| handle(message, cx));
                }
            })
            .detach();
        });
    Ok(())
}
