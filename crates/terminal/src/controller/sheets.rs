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
            .on_action(cx.listener(|this, _: &CancelNewRemoteConnection, window, cx| {
                this.cancel_new_remote_connection(window, cx);
            }))
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
        let title = session.tab_title().unwrap_or_else(|| "Terminal".to_string());
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
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Inspector"),
                )
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
}
