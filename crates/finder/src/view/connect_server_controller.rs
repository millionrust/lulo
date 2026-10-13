//! Go ▸ Connect to Server… (⌘K) (FILES-39): the address sheet, favourite
//! and recent servers, and the sign-in step. The mounting itself is
//! `crate::connect_server`.

use std::sync::atomic::AtomicI32;

use super::*;
use crate::connect_server::{self, AddressError, Credentials, MountOutcome};

/// What the sheet is doing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ConnectPhase {
    Idle,
    /// `gio mount` is running for this address.
    Connecting(String),
    /// The server wants a name and password; the text says for what.
    SignIn(String),
    Error(String),
}

pub(super) struct ConnectSheet {
    address: gpui::Entity<InputState>,
    user: gpui::Entity<InputState>,
    password: gpui::Entity<InputState>,
    phase: ConnectPhase,
    guest: bool,
    selected_favourite: Option<usize>,
    /// The running `gio mount`'s pid, for Cancel.
    pid: Arc<AtomicI32>,
}

const CONNECT_WIDTH: f32 = 420.0;

impl FinderView {
    pub(super) fn open_connect_to_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.connect.is_some() {
            return;
        }
        self.menu_at = None;
        let address = cx.new(|cx| InputState::new(window, cx).placeholder("Server Address"));
        let user = cx.new(|cx| InputState::new(window, cx).placeholder("Name"));
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Password")
        });
        cx.subscribe_in(
            &address,
            window,
            |this: &mut Self, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    if let Some(sheet) = this.connect.as_mut() {
                        if matches!(sheet.phase, ConnectPhase::Error(_)) {
                            sheet.phase = ConnectPhase::Idle;
                        }
                    }
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => this.start_connect(window, cx),
                _ => {}
            },
        )
        .detach();
        for field in [&user, &password] {
            cx.subscribe_in(
                field,
                window,
                |this: &mut Self, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.start_connect(window, cx);
                    }
                },
            )
            .detach();
        }
        let focus = address.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.connect = Some(ConnectSheet {
            address,
            user,
            password,
            phase: ConnectPhase::Idle,
            guest: false,
            selected_favourite: None,
            pid: Arc::new(AtomicI32::new(0)),
        });
        cx.notify();
    }

    pub(super) fn close_connect_to_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(sheet) = self.connect.take() {
            connect_server::cancel(&sheet.pid);
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn set_connect_guest(&mut self, guest: bool, cx: &mut Context<Self>) {
        if let Some(sheet) = self.connect.as_mut() {
            sheet.guest = guest;
        }
        cx.notify();
    }

    fn set_connect_address(&mut self, uri: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sheet) = self.connect.as_ref() else {
            return;
        };
        let uri = uri.to_owned();
        sheet
            .address
            .update(cx, |state, cx| state.set_value(uri, window, cx));
    }

    fn pick_connect_server(
        &mut self,
        uri: String,
        favourite: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_connect_address(&uri, window, cx);
        if let Some(sheet) = self.connect.as_mut() {
            sheet.selected_favourite = favourite;
            sheet.phase = ConnectPhase::Idle;
        }
        cx.notify();
    }

    /// The "+" button: remember the typed address as a favourite.
    fn add_connect_favourite(&mut self, cx: &mut Context<Self>) {
        let Some(sheet) = self.connect.as_mut() else {
            return;
        };
        let text = sheet.address.read(cx).value().to_string();
        match connect_server::parse_address(&text) {
            Ok(uri) => {
                let _ = settings::update(
                    |settings| {
                        if !settings.favourite_servers.contains(&uri) {
                            settings.favourite_servers.push(uri.clone());
                        }
                    },
                    cx,
                );
            }
            Err(error) => {
                if let Some(sheet) = self.connect.as_mut() {
                    sheet.phase = ConnectPhase::Error(error.message());
                }
            }
        }
        cx.notify();
    }

    /// The "−" button: forget the selected favourite.
    fn remove_connect_favourite(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self
            .connect
            .as_mut()
            .and_then(|sheet| sheet.selected_favourite.take())
        else {
            return;
        };
        let _ = settings::update(
            |settings| {
                if index < settings.favourite_servers.len() {
                    settings.favourite_servers.remove(index);
                }
            },
            cx,
        );
        cx.notify();
    }

    pub(super) fn start_connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sheet) = self.connect.as_mut() else {
            return;
        };
        if matches!(sheet.phase, ConnectPhase::Connecting(_)) {
            return;
        }
        let text = sheet.address.read(cx).value().to_string();
        let uri = match connect_server::parse_address(&text) {
            Ok(uri) => uri,
            Err(error) => {
                if !matches!(error, AddressError::Empty) || !text.trim().is_empty() {
                    sheet.phase = ConnectPhase::Error(error.message());
                }
                cx.notify();
                return;
            }
        };
        // Windows reaches an SMB share by its UNC path directly.
        #[cfg(windows)]
        if let Some(path) = connect_server::unc_path(&uri) {
            self.connect = None;
            window.focus(&self.focus, cx);
            self.navigate(path, cx);
            return;
        }
        let credentials = matches!(sheet.phase, ConnectPhase::SignIn(_)).then(|| Credentials {
            guest: sheet.guest,
            user: sheet.user.read(cx).value().to_string(),
            password: sheet.password.read(cx).value().to_string(),
        });
        sheet.phase = ConnectPhase::Connecting(uri.clone());
        let pid = sheet.pid.clone();
        let _ = window;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let target = uri.clone();
            let (outcome, entries) = blocking::unblock(move || {
                let outcome = connect_server::mount(&target, credentials.as_ref(), &pid);
                let entries = if outcome == MountOutcome::Mounted {
                    gvfs_entries()
                } else {
                    Vec::new()
                };
                (outcome, entries)
            })
            .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                this.finish_connect(uri, outcome, entries, cx);
            });
        })
        .detach();
    }

    fn finish_connect(
        &mut self,
        uri: String,
        outcome: MountOutcome,
        entries: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        // Cancel closed the sheet; nothing is waiting for this result.
        let Some(sheet) = self.connect.as_mut() else {
            return;
        };
        match outcome {
            MountOutcome::Mounted => {
                self.connect = None;
                let _ = settings::update(
                    |settings| {
                        connect_server::remember(
                            &mut settings.recent_servers,
                            &uri,
                            connect_server::MAX_RECENT_SERVERS,
                        );
                    },
                    cx,
                );
                let root = gvfs_root();
                let target = root.as_ref().map(|root| {
                    connect_server::pick_mount_dir(&entries, &uri)
                        .map_or_else(|| root.clone(), |name| root.join(name))
                });
                match target {
                    Some(path) if path.is_dir() => self.navigate(path, cx),
                    _ => {
                        self.operation_notice =
                            Some("Connected. The server is listed under Go ▸ Network.".into());
                    }
                }
            }
            MountOutcome::NeedsCredentials(message) => {
                sheet.phase = ConnectPhase::SignIn(message);
            }
            MountOutcome::Failed(message) => {
                sheet.phase = ConnectPhase::Error(message);
            }
        }
        cx.notify();
    }

    pub(super) fn render_connect_to_server(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let sheet = self.connect.as_ref()?;
        let stored = settings::current();
        let connecting = matches!(sheet.phase, ConnectPhase::Connecting(_));
        let entity = cx.entity();
        let address_empty = sheet.address.read(cx).value().trim().is_empty();
        let server_row = |id: &'static str,
                          index: usize,
                          uri: &str,
                          selected: bool,
                          favourite: Option<usize>,
                          cx: &mut Context<Self>| {
            let target = uri.to_owned();
            div()
                .id((id, index))
                .role(Role::ListBoxOption)
                .aria_label(uri.to_owned())
                .h(px(GO_TO_ROW_HEIGHT))
                .flex()
                .items_center()
                .px(px(8.0))
                .rounded(px(ROW_RADIUS))
                .text_size(rmac_ui::text_px(13.0))
                .text_color(if selected {
                    selected_text(true)
                } else {
                    primary_text()
                })
                .when(selected, |row| row.bg(selection(true)))
                .cursor_pointer()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .truncate()
                        .child(uri.to_owned()),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.pick_connect_server(target.clone(), favourite, window, cx);
                }))
        };
        let favourites = stored
            .favourite_servers
            .iter()
            .enumerate()
            .map(|(index, uri)| {
                let selected = sheet.selected_favourite == Some(index);
                server_row("connect-favourite", index, uri, selected, Some(index), cx)
            })
            .collect::<Vec<_>>();
        let recents = stored
            .recent_servers
            .iter()
            .enumerate()
            .map(|(index, uri)| server_row("connect-recent", index, uri, false, None, cx))
            .collect::<Vec<_>>();
        let section_title = |text: &'static str| {
            div()
                .px(px(4.0))
                .text_size(rmac_ui::text_px(11.0))
                .text_color(rmac_ui::mac::text_secondary())
                .child(text)
        };
        let mut panel = div()
            .id("connect-to-server")
            .role(Role::Dialog)
            .aria_label("Connect to Server")
            .relative()
            .w(px(CONNECT_WIDTH))
            .v_flex()
            .gap(px(6.0))
            .p(px(12.0))
            .rounded(px(rmac_ui::mac::radius_card()))
            .bg(rmac_ui::mac::raised())
            .border_1()
            .border_color(sep())
            .shadow_lg()
            .child(
                div()
                    .text_size(rmac_ui::text_px(13.0))
                    .font_weight(rmac_ui::mac::BOLD)
                    .text_color(primary_text())
                    .child("Connect to Server"),
            )
            .child(
                div()
                    .id("connect-address")
                    .role(Role::TextInput)
                    .aria_label("Server Address")
                    .accessible_text_input(&sheet.address, cx)
                    .child(TextField::new(&sheet.address)),
            )
            .child(section_title("Favourite Servers:"))
            .child(
                div()
                    .id("connect-favourites")
                    .role(Role::ListBox)
                    .aria_label("Favourite Servers")
                    .v_flex()
                    .min_h(px(GO_TO_ROW_HEIGHT * 2.0))
                    .max_h(px(GO_TO_ROW_HEIGHT * 5.0))
                    .overflow_hidden()
                    .rounded(px(ROW_RADIUS))
                    .border_1()
                    .border_color(sep())
                    .children(favourites),
            )
            .child(
                div()
                    .flex()
                    .gap(px(4.0))
                    .child(
                        Button::new("connect-add-favourite", "+")
                            .xsmall()
                            .disabled(address_empty || connecting)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.add_connect_favourite(cx);
                            })),
                    )
                    .child(
                        Button::new("connect-remove-favourite", "−")
                            .xsmall()
                            .disabled(sheet.selected_favourite.is_none() || connecting)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.remove_connect_favourite(cx);
                            })),
                    ),
            );
        if !recents.is_empty() {
            panel = panel.child(section_title("Recent Servers:")).child(
                div()
                    .id("connect-recents")
                    .role(Role::ListBox)
                    .aria_label("Recent Servers")
                    .v_flex()
                    .max_h(px(GO_TO_ROW_HEIGHT * 3.0))
                    .overflow_hidden()
                    .children(recents),
            );
        }
        if let ConnectPhase::SignIn(message) = &sheet.phase {
            panel = panel
                .child(
                    div()
                        .px(px(4.0))
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(primary_text())
                        .child(message.clone()),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(12.0))
                        .child(
                            rmac_ui::Radio::new("connect-guest")
                                .label("Guest")
                                .selected(sheet.guest)
                                .on_change({
                                    let entity = entity.clone();
                                    move |_, _, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.set_connect_guest(true, cx);
                                        });
                                    }
                                }),
                        )
                        .child(
                            rmac_ui::Radio::new("connect-registered")
                                .label("Registered User")
                                .selected(!sheet.guest)
                                .on_change(move |_, _, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.set_connect_guest(false, cx);
                                    });
                                }),
                        ),
                );
            if !sheet.guest {
                panel = panel
                    .child(
                        div()
                            .id("connect-user")
                            .role(Role::TextInput)
                            .aria_label("Name")
                            .accessible_text_input(&sheet.user, cx)
                            .child(TextField::new(&sheet.user)),
                    )
                    .child(
                        div()
                            .id("connect-password")
                            .role(Role::TextInput)
                            .aria_label("Password")
                            .accessible_text_input(&sheet.password, cx)
                            .child(TextField::new(&sheet.password)),
                    );
            }
        }
        match &sheet.phase {
            ConnectPhase::Connecting(uri) => {
                panel = panel.child(
                    div()
                        .px(px(4.0))
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(rmac_ui::mac::text_secondary())
                        .child(format!("Connecting to {uri}…")),
                );
            }
            ConnectPhase::Error(message) => {
                panel = panel.child(
                    div()
                        .px(px(4.0))
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(rmac_ui::mac::danger())
                        .child(message.clone()),
                );
            }
            ConnectPhase::Idle | ConnectPhase::SignIn(_) => {}
        }
        panel = panel.child(
            div()
                .flex()
                .justify_end()
                .gap(px(8.0))
                .child(Button::new("connect-cancel", "Cancel").on_click(
                    cx.listener(|this, _, window, cx| this.close_connect_to_server(window, cx)),
                ))
                .child(
                    Button::new("connect-go", "Connect")
                        .primary()
                        .disabled(address_empty || connecting)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.start_connect(window, cx)),
                        ),
                ),
        );
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(TOOLBAR_HEIGHT))
                .child(panel)
                .into_any_element(),
        )
    }
}

/// The names GVFS lists under `$XDG_RUNTIME_DIR/gvfs`.
fn gvfs_entries() -> Vec<String> {
    let Some(root) = gvfs_root() else {
        return Vec::new();
    };
    std::fs::read_dir(root)
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default()
}

fn gvfs_root() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|dir| !dir.is_empty())
        .map(|dir| PathBuf::from(dir).join("gvfs"))
}
