//! Shell ▸ New Command… (⇧⌘N), New Remote Connection… (⇧⌘K), Edit Title
//! (⇧⌘I) and Show/Hide Inspector (⌘I) — mocked in `design-lab/` as
//! `terminal-new-command.html`, `terminal-new-remote-connection.html`,
//! `terminal-edit-title.html` and `terminal-inspector.html`.
//!
//! The first three are modal sheets (`modal_open()` covers all three, so
//! the grid stops accepting keystrokes while one is open, mirroring the
//! close/paste reviews); the Inspector is a non-modal panel the user can
//! leave open while working, like the Find bar.

use super::*;

/// Shell ▸ New Command… (⇧⌘N): runs a typed command in a fresh window,
/// reusing the same `-e PROGRAM ARGS…` convention as `-e` on the command
/// line and Shell ▸ New Window with Same Command.
pub(super) struct NewCommandSheet {
    pub(super) command: Entity<InputState>,
    pub(super) run_in_shell: bool,
    pub(super) error: bool,
}

/// Shell ▸ New Remote Connection… (⇧⌘K): a simplified stand-in for the
/// Mac's Service/Server list — there is no Bonjour/SSH-server discovery on
/// Linux to populate one — that still really connects: Connect runs
/// `ssh [-l USER] HOST` in a fresh window, exactly what choosing a server
/// and clicking Connect does on the Mac.
pub(super) struct RemoteConnectionSheet {
    pub(super) host: Entity<InputState>,
    pub(super) user: Entity<InputState>,
    pub(super) error: bool,
}

/// Shell ▸ Edit Title (⇧⌘I): overrides the automatic tab/window title.
pub(super) struct EditTitleSheet {
    pub(super) title: Entity<InputState>,
}

/// Shell ▸ Open… (⌘O): a path typed directly, or chosen with the portal's
/// file chooser — a folder opens a new window there, a file runs it (as a
/// script through the shell, or directly if it is itself executable).
pub(super) struct OpenShellSheet {
    pub(super) path: Entity<InputState>,
    pub(super) error: Option<&'static str>,
}

/// Shell ▸ Edit Background Colour (⌥⌘I): a hex colour that overrides the
/// active profile's background for new windows, like `font_size`'s own
/// "Use Settings as Default" escape hatch.
pub(super) struct BackgroundColourSheet {
    pub(super) hex: Entity<InputState>,
    pub(super) error: bool,
}

impl TerminalView {
    pub(super) fn open_new_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        self.menu_at = None;
        self.picker_open = false;
        let command = cx.new(|cx| InputState::new(window, cx).placeholder("Command"));
        let focus = command.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.pending_new_command = Some(NewCommandSheet {
            command,
            run_in_shell: true,
            error: false,
        });
        cx.notify();
    }

    pub(super) fn cancel_new_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_new_command = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(super) fn commit_new_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sheet) = self.pending_new_command.as_ref() else {
            return;
        };
        let text = sheet.command.read(cx).value().to_string();
        let run_in_shell = sheet.run_in_shell;
        let exec = crate::cli::command_to_exec(&text, run_in_shell);
        let Some(exec) = exec else {
            if let Some(sheet) = self.pending_new_command.as_mut() {
                sheet.error = true;
            }
            cx.notify();
            return;
        };
        self.pending_new_command = None;
        window.focus(&self.focus, cx);
        open_window_with_same_command(exec, cx);
    }

    pub(super) fn open_new_remote_connection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.modal_open() {
            return;
        }
        self.menu_at = None;
        self.picker_open = false;
        let host = cx.new(|cx| InputState::new(window, cx).placeholder("Server"));
        let user = cx.new(|cx| InputState::new(window, cx).placeholder("User"));
        let focus = host.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.pending_remote_connection = Some(RemoteConnectionSheet {
            host,
            user,
            error: false,
        });
        cx.notify();
    }

    pub(super) fn cancel_new_remote_connection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_remote_connection = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(super) fn commit_new_remote_connection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(sheet) = self.pending_remote_connection.as_ref() else {
            return;
        };
        let host = sheet.host.read(cx).value().trim().to_string();
        let user = sheet.user.read(cx).value().trim().to_string();
        if host.is_empty() {
            if let Some(sheet) = self.pending_remote_connection.as_mut() {
                sheet.error = true;
            }
            cx.notify();
            return;
        }
        let mut args = vec!["ssh".to_string()];
        if !user.is_empty() {
            args.push("-l".to_string());
            args.push(user);
        }
        args.push(host);
        self.pending_remote_connection = None;
        window.focus(&self.focus, cx);
        let mut arguments = vec!["-e".to_string()];
        arguments.extend(args);
        if !rmac_ui::open_another_window(arguments, cx) {
            self.operation_error = Some("Terminal could not open a new window.".into());
        }
        cx.notify();
    }

    pub(super) fn open_edit_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        self.menu_at = None;
        self.picker_open = false;
        let current = self.tabs[self.active].tab_title().unwrap_or_default();
        let title = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("Title");
            state.set_value(current, window, cx);
            state
        });
        let focus = title.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.pending_edit_title = Some(EditTitleSheet { title });
        cx.notify();
    }

    pub(super) fn cancel_edit_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_edit_title = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(super) fn commit_edit_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sheet) = self.pending_edit_title.take() else {
            return;
        };
        let text = sheet.title.read(cx).value().to_string();
        let trimmed = text.trim();
        self.tabs[self.active].set_manual_title(if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        });
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Shell ▸ Show/Hide Inspector (⌘I).
    pub(super) fn toggle_inspector(&mut self, cx: &mut Context<Self>) {
        self.inspector_open = !self.inspector_open;
        cx.notify();
    }

    fn sheet_panel(id: &'static str, label: &'static str) -> Stateful<Div> {
        div()
            .id(id)
            .role(Role::Dialog)
            .aria_label(label)
            .relative()
            .v_flex()
            .gap(px(10.0))
            .p(px(16.0))
            .rounded(px(rmac_ui::mac::radius_card()))
            .bg(rmac_ui::mac::raised())
            .border_1()
            .border_color(rmac_ui::mac::separator())
            .shadow_lg()
    }

    fn sheet_overlay(panel: impl IntoElement) -> impl IntoElement {
        div()
            .absolute()
            .inset_0()
            .flex()
            .justify_center()
            .items_center()
            .bg(rmac_ui::mac::scrim())
            .child(panel)
    }

    /// Shell ▸ New Command… (499 × 136 on the Mac): a command field, "Run
    /// command inside a shell", Cancel · Run.
    pub(super) fn render_new_command(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let sheet = self.pending_new_command.as_ref()?;
        let run_in_shell = sheet.run_in_shell;
        let view = cx.entity();
        let panel = Self::sheet_panel("terminal-new-command", "New Command")
            .key_context("TerminalNewCommand")
            .on_action(cx.listener(|this, _: &CancelNewCommand, window, cx| {
                this.cancel_new_command(window, cx);
            }))
            .w(px(460.0))
            .child(
                div()
                    .id("new-command-field")
                    .role(Role::TextInput)
                    .aria_label("Command")
                    .accessible_text_input(&sheet.command, cx)
                    .child(TextField::new(&sheet.command)),
            )
            .child(
                Checkbox::new("new-command-shell")
                    .checked(run_in_shell)
                    .label("Run command inside a shell")
                    .on_change(move |value, _, cx| {
                        let value = *value;
                        view.update(cx, |this, cx| {
                            if let Some(sheet) = this.pending_new_command.as_mut() {
                                sheet.run_in_shell = value;
                                cx.notify();
                            }
                        });
                    }),
            )
            .when(sheet.error, |panel| {
                panel.child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(rmac_ui::mac::danger())
                        .child("Type a command to run."),
                )
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        rmac_ui::dialog_button(
                            "new-command-cancel",
                            "Cancel",
                            rmac_ui::DialogButtonKind::Normal,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.cancel_new_command(window, cx);
                        })),
                    )
                    .child(
                        rmac_ui::dialog_button(
                            "new-command-run",
                            "Run",
                            rmac_ui::DialogButtonKind::Primary,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.commit_new_command(window, cx);
                        })),
                    ),
            );
        Some(Self::sheet_overlay(panel))
    }

    /// Shell ▸ New Remote Connection… (422 × 432 on the Mac): a simplified
    /// Server/User form (no SSH-server discovery exists on Linux) with
    /// Cancel · Connect.
    pub(super) fn render_new_remote_connection(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let sheet = self.pending_remote_connection.as_ref()?;
        let panel = Self::sheet_panel("terminal-new-remote-connection", "New Remote Connection")
            .key_context("TerminalRemote")
            .on_action(
                cx.listener(|this, _: &CancelNewRemoteConnection, window, cx| {
                    this.cancel_new_remote_connection(window, cx);
                }),
            )
            .w(px(360.0))
            .child(
                div()
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(rmac_ui::mac::text_secondary())
                    .child("Connect over SSH:"),
            )
            .child(
                div()
                    .id("new-remote-host")
                    .role(Role::TextInput)
                    .aria_label("Server")
                    .accessible_text_input(&sheet.host, cx)
                    .child(TextField::new(&sheet.host)),
            )
            .child(
                div()
                    .id("new-remote-user")
                    .role(Role::TextInput)
                    .aria_label("User")
                    .accessible_text_input(&sheet.user, cx)
                    .child(TextField::new(&sheet.user)),
            )
            .when(sheet.error, |panel| {
                panel.child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(rmac_ui::mac::danger())
                        .child("Type a server to connect to."),
                )
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        rmac_ui::dialog_button(
                            "new-remote-cancel",
                            "Cancel",
                            rmac_ui::DialogButtonKind::Normal,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.cancel_new_remote_connection(window, cx);
                        })),
                    )
                    .child(
                        rmac_ui::dialog_button(
                            "new-remote-connect",
                            "Connect",
                            rmac_ui::DialogButtonKind::Primary,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.commit_new_remote_connection(window, cx);
                        })),
                    ),
            );
        Some(Self::sheet_overlay(panel))
    }

    /// Shell ▸ Edit Title (⇧⌘I): one field, Cancel · Done.
    pub(super) fn render_edit_title(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let sheet = self.pending_edit_title.as_ref()?;
        let panel = Self::sheet_panel("terminal-edit-title", "Edit Title")
            .key_context("TerminalEditTitle")
            .on_action(cx.listener(|this, _: &CancelEditTitle, window, cx| {
                this.cancel_edit_title(window, cx);
            }))
            .w(px(360.0))
            .child(
                div()
                    .id("edit-title-field")
                    .role(Role::TextInput)
                    .aria_label("Title")
                    .accessible_text_input(&sheet.title, cx)
                    .child(TextField::new(&sheet.title)),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        rmac_ui::dialog_button(
                            "edit-title-cancel",
                            "Cancel",
                            rmac_ui::DialogButtonKind::Normal,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.cancel_edit_title(window, cx);
                        })),
                    )
                    .child(
                        rmac_ui::dialog_button(
                            "edit-title-done",
                            "Done",
                            rmac_ui::DialogButtonKind::Primary,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.commit_edit_title(window, cx);
                        })),
                    ),
            );
        Some(Self::sheet_overlay(panel))
    }

    /// Shell ▸ Show Inspector (⌘I): a non-modal panel docked to the
    /// right — the window/tab title and the shell's current foreground
    /// process, the same live data the tab strip and the close review
    /// already read (`job_state`/`tab_title`), not a new polling source.
    pub(super) fn render_inspector(&self) -> Option<impl IntoElement> {
        if !self.inspector_open {
            return None;
        }
        let session = &self.tabs[self.active];
        let title = session
            .tab_title()
            .unwrap_or_else(|| "Terminal".to_string());
        let process = session.foreground_job_name().unwrap_or_else(|| "—".into());
        let shell_state = if session.accepts_input() {
            "Running"
        } else {
            "Not Running"
        };
        Some(
            div()
                .id("terminal-inspector")
                .role(Role::Group)
                .aria_label("Inspector")
                .absolute()
                .top(px(40.0))
                .right(px(12.0))
                .w(px(220.0))
                .v_flex()
                .gap(px(8.0))
                .p(px(12.0))
                .rounded(px(rmac_ui::mac::radius_card()))
                .bg(rmac_ui::mac::raised())
                .border_1()
                .border_color(rmac_ui::mac::separator())
                .shadow_lg()
                .text_size(rmac_ui::text_px(12.0))
                .child(div().font_weight(FontWeight::SEMIBOLD).child("Inspector"))
                .child(
                    div()
                        .text_color(rmac_ui::mac::text_secondary())
                        .child(format!("Title: {title}")),
                )
                .child(
                    div()
                        .text_color(rmac_ui::mac::text_secondary())
                        .child(format!("Size: {} × {}", self.cols, self.rows)),
                )
                .child(
                    div()
                        .text_color(rmac_ui::mac::text_secondary())
                        .child(format!("Shell: {shell_state}")),
                )
                .child(
                    div()
                        .text_color(rmac_ui::mac::text_secondary())
                        .child(format!("Process: {process}")),
                ),
        )
    }

    /// Shell ▸ Open… (⌘O): a folder path opens a new window there; a file
    /// path runs it in one (directly if it is itself executable, through
    /// `/bin/sh` otherwise) — the plain double-click convention this
    /// desktop already uses for scripts, since there is no `.term`/`.command`
    /// document format to open instead.
    pub(super) fn open_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        self.menu_at = None;
        self.picker_open = false;
        let path = cx.new(|cx| InputState::new(window, cx).placeholder("Path"));
        let focus = path.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.pending_open_shell = Some(OpenShellSheet { path, error: None });
        cx.notify();
    }

    pub(super) fn cancel_open_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_open_shell = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Shell ▸ Open…'s "Choose…": the portal's own Open panel, filling the
    /// typed-path field rather than committing immediately, so the user
    /// can still see (and edit) what was chosen before opening it.
    pub(super) fn choose_open_shell_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_open_shell.is_none() {
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            let chosen = rmac_portal::choose_terminal_open_target().await;
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(sheet) = this.pending_open_shell.as_ref() else {
                    return;
                };
                match chosen {
                    Ok(Some(path)) => {
                        let text = path.to_string_lossy().into_owned();
                        sheet.path.update(cx, |state, cx| {
                            state.set_value(text, window, cx);
                        });
                        if let Some(sheet) = this.pending_open_shell.as_mut() {
                            sheet.error = None;
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        if let Some(sheet) = this.pending_open_shell.as_mut() {
                            sheet.error = Some("Could not open the file chooser.");
                            let _ = error;
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn commit_open_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sheet) = self.pending_open_shell.as_ref() else {
            return;
        };
        let typed = sheet.path.read(cx).value().trim().to_string();
        if typed.is_empty() {
            if let Some(sheet) = self.pending_open_shell.as_mut() {
                sheet.error = Some("Type a path, or choose one, to open.");
            }
            cx.notify();
            return;
        }
        let path = std::path::PathBuf::from(&typed);
        let Some(restore) = open_target_restore_window(&path, self.profile) else {
            if let Some(sheet) = self.pending_open_shell.as_mut() {
                sheet.error = Some("Terminal could not find that path.");
            }
            cx.notify();
            return;
        };
        self.pending_open_shell = None;
        window.focus(&self.focus, cx);
        match crate::cli::restore_flag(&restore) {
            Some(flag) if rmac_ui::open_another_window(vec![flag.clone()], cx) => {}
            _ => {
                self.operation_error = Some("Terminal could not open a new window.".into());
            }
        }
        cx.notify();
    }

    /// Shell ▸ Edit Background Colour (⌥⌘I): a hex field, pre-filled with
    /// whatever this window is drawing right now (the override if it set
    /// one, otherwise the active profile's own colour).
    pub(super) fn open_edit_background_colour(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.modal_open() {
            return;
        }
        self.menu_at = None;
        self.picker_open = false;
        let current = self
            .background_override
            .unwrap_or_else(|| profiles::active().bg);
        let hex = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("RRGGBB");
            state.set_value(format!("{current:06x}"), window, cx);
            state
        });
        let focus = hex.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.pending_background_colour = Some(BackgroundColourSheet { hex, error: false });
        cx.notify();
    }

    pub(super) fn cancel_edit_background_colour(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_background_colour = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(super) fn commit_edit_background_colour(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(sheet) = self.pending_background_colour.as_ref() else {
            return;
        };
        let typed = sheet
            .hex
            .read(cx)
            .value()
            .trim()
            .trim_start_matches('#')
            .to_string();
        let Ok(colour) = u32::from_str_radix(&typed, 16).map(|value| value & 0x00ff_ffff) else {
            if let Some(sheet) = self.pending_background_colour.as_mut() {
                sheet.error = true;
            }
            cx.notify();
            return;
        };
        if typed.len() != 6 {
            if let Some(sheet) = self.pending_background_colour.as_mut() {
                sheet.error = true;
            }
            cx.notify();
            return;
        }
        self.background_override = Some(colour);
        if let Err(error) = profiles::save_background_override(colour) {
            self.persistence_error = Some(error.to_string().into());
        }
        self.pending_background_colour = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Shell ▸ Open…'s 360 × 150-ish sheet: a path field, "Choose…", Cancel
    /// · Open.
    pub(super) fn render_open_shell(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let sheet = self.pending_open_shell.as_ref()?;
        let panel = Self::sheet_panel("terminal-open-shell", "Open")
            .key_context("TerminalOpenShell")
            .on_action(cx.listener(|this, _: &CancelOpenShell, window, cx| {
                this.cancel_open_shell(window, cx);
            }))
            .w(px(420.0))
            .child(
                div()
                    .id("open-shell-field")
                    .role(Role::TextInput)
                    .aria_label("Path")
                    .accessible_text_input(&sheet.path, cx)
                    .child(TextField::new(&sheet.path)),
            )
            .when_some(sheet.error, |panel, error| {
                panel.child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(rmac_ui::mac::danger())
                        .child(error),
                )
            })
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        rmac_ui::dialog_button(
                            "open-shell-choose",
                            "Choose…",
                            rmac_ui::DialogButtonKind::Normal,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.choose_open_shell_path(window, cx);
                        })),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(8.0))
                            .child(
                                rmac_ui::dialog_button(
                                    "open-shell-cancel",
                                    "Cancel",
                                    rmac_ui::DialogButtonKind::Normal,
                                )
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        this.cancel_open_shell(window, cx);
                                    },
                                )),
                            )
                            .child(
                                rmac_ui::dialog_button(
                                    "open-shell-open",
                                    "Open",
                                    rmac_ui::DialogButtonKind::Primary,
                                )
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        this.commit_open_shell(window, cx);
                                    },
                                )),
                            ),
                    ),
            );
        Some(Self::sheet_overlay(panel))
    }

    /// Shell ▸ Edit Background Colour's small sheet: a hex field, Cancel ·
    /// Set.
    pub(super) fn render_edit_background_colour(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let sheet = self.pending_background_colour.as_ref()?;
        let typed = sheet
            .hex
            .read(cx)
            .value()
            .trim()
            .trim_start_matches('#')
            .to_string();
        let preview = (typed.len() == 6)
            .then(|| u32::from_str_radix(&typed, 16).ok())
            .flatten()
            .unwrap_or_else(|| {
                self.background_override
                    .unwrap_or_else(|| profiles::active().bg)
            });
        let panel = Self::sheet_panel("terminal-edit-background-colour", "Edit Background Colour")
            .key_context("TerminalBackgroundColour")
            .on_action(
                cx.listener(|this, _: &CancelEditBackgroundColour, window, cx| {
                    this.cancel_edit_background_colour(window, cx);
                }),
            )
            .w(px(320.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .id("background-colour-swatch")
                            .role(Role::Image)
                            .aria_label("Preview")
                            .w(px(24.0))
                            .h(px(24.0))
                            .rounded(px(rmac_ui::mac::radius_control()))
                            .border_1()
                            .border_color(rmac_ui::mac::separator())
                            .bg(Hsla::from(gpui::rgb(preview))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .id("background-colour-field")
                            .role(Role::TextInput)
                            .aria_label("Background colour (hex)")
                            .accessible_text_input(&sheet.hex, cx)
                            .child(TextField::new(&sheet.hex)),
                    ),
            )
            .when(sheet.error, |panel| {
                panel.child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(rmac_ui::mac::danger())
                        .child("Type a 6-digit hex colour, like 1E1E1E."),
                )
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        rmac_ui::dialog_button(
                            "background-colour-cancel",
                            "Cancel",
                            rmac_ui::DialogButtonKind::Normal,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.cancel_edit_background_colour(window, cx);
                        })),
                    )
                    .child(
                        rmac_ui::dialog_button(
                            "background-colour-set",
                            "Set",
                            rmac_ui::DialogButtonKind::Primary,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.commit_edit_background_colour(window, cx);
                        })),
                    ),
            );
        Some(Self::sheet_overlay(panel))
    }
}

/// Shell ▸ Open…: what opening `path` should do, as a one-tab
/// [`crate::session_restore::RestoreWindow`] — a folder just sets the new
/// window's working directory; a file runs it (directly if executable,
/// through `/bin/sh` otherwise), starting in its own parent folder. `None`
/// if `path` does not exist.
fn open_target_restore_window(
    path: &std::path::Path,
    profile: usize,
) -> Option<crate::session_restore::RestoreWindow> {
    let metadata = std::fs::metadata(path).ok()?;
    let tab = if metadata.is_dir() {
        crate::session_restore::RestoreTab {
            cwd: Some(path.to_path_buf()),
            program: None,
            args: Vec::new(),
            profile,
            scrollback: String::new(),
        }
    } else {
        let executable = is_executable(&metadata, path);
        let parent = path.parent().map(std::path::Path::to_path_buf);
        if executable {
            crate::session_restore::RestoreTab {
                cwd: parent,
                program: Some(path.to_string_lossy().into_owned()),
                args: Vec::new(),
                profile,
                scrollback: String::new(),
            }
        } else {
            crate::session_restore::RestoreTab {
                cwd: parent,
                program: Some(shell_runner()),
                args: shell_run_args(path),
                profile,
                scrollback: String::new(),
            }
        }
    };
    Some(crate::session_restore::RestoreWindow { tabs: vec![tab] })
}

/// Unix: the executable bit. Windows has no such bit; a `.exe`/`.bat`/`.cmd`
/// extension is "executable" there (what `cmd.exe` would run directly).
#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata, _path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(windows)]
fn is_executable(_metadata: &std::fs::Metadata, path: &std::path::Path) -> bool {
    let _ = _metadata;
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("exe")
                || extension.eq_ignore_ascii_case("bat")
                || extension.eq_ignore_ascii_case("cmd")
        })
}

/// What runs a non-executable file: `/bin/sh` on Unix, the same default
/// shell Terminal starts (`powershell.exe`) on Windows.
#[cfg(unix)]
fn shell_runner() -> String {
    "/bin/sh".to_string()
}

#[cfg(windows)]
fn shell_runner() -> String {
    "powershell.exe".to_string()
}

#[cfg(unix)]
fn shell_run_args(path: &std::path::Path) -> Vec<String> {
    vec![path.to_string_lossy().into_owned()]
}

#[cfg(windows)]
fn shell_run_args(path: &std::path::Path) -> Vec<String> {
    vec!["-File".to_string(), path.to_string_lossy().into_owned()]
}

#[cfg(test)]
mod open_target_tests {
    use super::open_target_restore_window;

    #[test]
    fn a_missing_path_opens_nothing() {
        assert_eq!(
            open_target_restore_window(std::path::Path::new("/does/not/exist-xyz"), 0),
            None
        );
    }

    #[test]
    fn a_directory_just_sets_the_new_windows_cwd() {
        let dir = std::env::temp_dir();
        let restore = open_target_restore_window(&dir, 2).expect("exists");
        assert_eq!(restore.tabs.len(), 1);
        assert_eq!(restore.tabs[0].cwd.as_deref(), Some(dir.as_path()));
        assert_eq!(restore.tabs[0].program, None);
        assert_eq!(restore.tabs[0].profile, 2);
    }

    #[cfg(unix)]
    #[test]
    fn a_non_executable_file_runs_through_a_plain_shell() {
        let path = std::env::temp_dir().join(format!(
            "rmac-terminal-open-target-test-{}",
            std::process::id()
        ));
        std::fs::write(&path, b"#!/bin/sh\necho hi\n").unwrap();
        let restore = open_target_restore_window(&path, 0).expect("exists");
        assert_eq!(restore.tabs[0].program.as_deref(), Some("/bin/sh"));
        assert_eq!(
            restore.tabs[0].args,
            vec![path.to_string_lossy().into_owned()]
        );
        std::fs::remove_file(&path).ok();
    }

    #[cfg(unix)]
    #[test]
    fn an_executable_file_runs_directly() {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!(
            "rmac-terminal-open-target-test-exec-{}",
            std::process::id()
        ));
        std::fs::write(&path, b"#!/bin/sh\necho hi\n").unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        let restore = open_target_restore_window(&path, 0).expect("exists");
        assert_eq!(
            restore.tabs[0].program.as_deref(),
            Some(path.to_str().unwrap())
        );
        assert!(restore.tabs[0].args.is_empty());
        std::fs::remove_file(&path).ok();
    }

    #[cfg(windows)]
    #[test]
    fn a_non_executable_file_runs_through_powershell() {
        let path = std::env::temp_dir().join(format!(
            "rmac-terminal-open-target-test-{}.txt",
            std::process::id()
        ));
        std::fs::write(&path, b"echo hi\n").unwrap();
        let restore = open_target_restore_window(&path, 0).expect("exists");
        assert_eq!(restore.tabs[0].program.as_deref(), Some("powershell.exe"));
        assert_eq!(
            restore.tabs[0].args,
            vec!["-File".to_string(), path.to_string_lossy().into_owned()]
        );
        std::fs::remove_file(&path).ok();
    }

    #[cfg(windows)]
    #[test]
    fn an_exe_file_runs_directly() {
        let path = std::env::temp_dir().join(format!(
            "rmac-terminal-open-target-test-exec-{}.exe",
            std::process::id()
        ));
        std::fs::write(&path, b"not a real PE, only the extension matters here\n").unwrap();
        let restore = open_target_restore_window(&path, 0).expect("exists");
        assert_eq!(
            restore.tabs[0].program.as_deref(),
            Some(path.to_str().unwrap())
        );
        assert!(restore.tabs[0].args.is_empty());
        std::fs::remove_file(&path).ok();
    }
}
