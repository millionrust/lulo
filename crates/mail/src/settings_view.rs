//! Mail ▸ Settings… (⌘,) window (MAIL-8): General, Accounts, Junk Mail,
//! Fonts & Colours, Viewing, Composing and Signatures, one window at a
//! time — mirrors `crates/notes/src/settings_window.rs`'s single-instance
//! pattern. Every control writes straight through to
//! `rmac_mail::settings::{load, save}` so a value survives the window
//! closing. The Accounts pane is deliberately thin: it only ever lists
//! what ACC-2/ACC-3 (`rmac-accounts`, System Settings ▸ Internet Accounts)
//! already configured, rather than duplicating that D-Bus/OAuth machinery
//! inside Mail.

use gpui::{
    div, prelude::FluentBuilder as _, px, App, AppContext as _, Context, FocusHandle, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, WindowHandle,
};
use rmac_mail::settings::{self, ComposeFormat, JunkMailAction, MailSettings, Signature};
use rmac_ui::{mac, Button, Checkbox, Root, StyledExt as _};

use crate::delivery::{self, ComposeAccount};

const WIDTH: f32 = 560.0;
const HEIGHT: f32 = 420.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pane {
    General,
    Accounts,
    JunkMail,
    FontsAndColours,
    Viewing,
    Composing,
    Signatures,
}

impl Pane {
    const ALL: [Self; 7] = [
        Self::General,
        Self::Accounts,
        Self::JunkMail,
        Self::FontsAndColours,
        Self::Viewing,
        Self::Composing,
        Self::Signatures,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Accounts => "Accounts",
            Self::JunkMail => "Junk Mail",
            Self::FontsAndColours => "Fonts & Colours",
            Self::Viewing => "Viewing",
            Self::Composing => "Composing",
            Self::Signatures => "Signatures",
        }
    }
}

thread_local! {
    static OPEN: std::cell::Cell<Option<WindowHandle<Root>>> = const { std::cell::Cell::new(None) };
}

/// Opens Mail ▸ Settings…, or brings the one already open to the front.
pub fn show(cx: &mut App) {
    if let Some(handle) = OPEN.with(std::cell::Cell::get) {
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            return;
        }
    }
    // Not `window_options_for_app`: Settings would then inherit whatever
    // size the main Mail window last saved under the same app_id (the
    // UIA-06/UIA-09 window-geometry-key bug).
    let options = rmac_ui::window_options_for_panel(rmac_ui::app_id::MAIL, WIDTH, HEIGHT, cx);
    let opened = cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        window.set_window_title("Mail Settings");
        let view = cx.new(SettingsView::new);
        let focus = view.read(cx).focus.clone();
        window.focus(&focus, cx);
        cx.new(|cx| Root::new(view, window, cx))
    });
    match opened {
        Ok(handle) => {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            OPEN.with(|open| open.set(Some(handle)));
        }
        Err(error) => eprintln!("rmac-mail: could not open Settings: {error}"),
    }
}

struct SettingsView {
    focus: FocusHandle,
    pane: Pane,
    settings: MailSettings,
    save_error: Option<SharedString>,
    /// The real accounts GOA knows about right now (MAIL-4): Accounts and
    /// Signatures read this, never a fixed address, so Settings reflects
    /// however many mail accounts (zero, one or several) the person
    /// actually has.
    accounts: Vec<ComposeAccount>,
}

impl SettingsView {
    fn new(cx: &mut Context<Self>) -> Self {
        let (settings, error) = settings::load();
        Self {
            focus: cx.focus_handle(),
            pane: Pane::General,
            settings,
            save_error: error.map(SharedString::from),
            accounts: delivery::accounts(),
        }
    }

    fn persist(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = settings::save(&self.settings) {
            self.save_error = Some(SharedString::from(error.to_string()));
        } else {
            self.save_error = None;
        }
        cx.notify();
    }

    fn tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .h(px(44.0))
            .px(px(10.0))
            .flex()
            .items_center()
            .gap(px(4.0))
            .border_b_1()
            .border_color(mac::separator())
            .children(Pane::ALL.into_iter().enumerate().map(|(index, pane)| {
                let selected = self.pane == pane;
                // The strip itself carries the real tab semantics (role and
                // selected state) the way Finder's own window-tab strip does
                // (`crates/finder/src/view/chrome_presentation/menus_tabs.rs`):
                // `Button::selected` is purely a visual highlight with no
                // accessibility signal of its own, so without this wrapper
                // AT-SPI never reported which Settings pane was showing.
                div()
                    .id(("mail-settings-tab", index))
                    .role(Role::Tab)
                    .aria_label(pane.label())
                    .aria_selected(selected)
                    .child(
                        Button::new(("mail-settings-tab-button", index), pane.label())
                            .small()
                            .selected(selected)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.pane = pane;
                                cx.notify();
                            })),
                    )
            }))
            .into_any_element()
    }

    fn row(&self, child: impl IntoElement) -> impl IntoElement {
        div().py(px(4.0)).child(child)
    }

    fn general_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let general = self.settings.general.clone();
        div()
            .v_flex()
            .gap(px(2.0))
            .child(
                self.row(
                    Checkbox::new("mail-settings-notify")
                        .label("Notify me when new mail arrives")
                        .checked(general.notify_new_mail)
                        .on_change(cx.listener(|this, value: &bool, _, cx| {
                            this.settings.general.notify_new_mail = *value;
                            this.persist(cx);
                        })),
                ),
            )
            .child(
                self.row(
                    Checkbox::new("mail-settings-sound")
                        .label("Play a sound when new mail arrives")
                        .checked(general.play_sound_for_new_mail)
                        .on_change(cx.listener(|this, value: &bool, _, cx| {
                            this.settings.general.play_sound_for_new_mail = *value;
                            this.persist(cx);
                        })),
                ),
            )
            .child(
                self.row(
                    Checkbox::new("mail-settings-remove-downloads")
                        .label("Remove unedited downloads when Mail quits")
                        .checked(general.remove_unedited_downloads)
                        .on_change(cx.listener(|this, value: &bool, _, cx| {
                            this.settings.general.remove_unedited_downloads = *value;
                            this.persist(cx);
                        })),
                ),
            )
            .child(
                div()
                    .pt(px(8.0))
                    .text_size(px(12.0))
                    .text_color(mac::text_secondary())
                    .child(format!("Downloads folder: {}", general.downloads_folder)),
            )
    }

    /// Mail stores no account state of its own: this reads nothing from
    /// disk and only points at where accounts actually live
    /// (`rmac-accounts`, surfaced in System Settings ▸ Internet Accounts
    /// and the Mail/Calendar "Add Account…" sheet, ACC-3).
    fn accounts_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .v_flex()
            .gap(px(10.0))
            .child(
                div()
                    .text_color(mac::text_secondary())
                    .text_size(px(12.0))
                    .child(
                        "Mail accounts are added and removed from System Settings ▸ \
                         Internet Accounts, so every app that uses them stays in sync.",
                    ),
            )
            .child(
                div()
                    .v_flex()
                    .gap(px(4.0))
                    .children(self.accounts.iter().map(|account| {
                        div()
                            .p(px(8.0))
                            .rounded(px(mac::radius_card()))
                            .bg(mac::control_fill())
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(px(13.0))
                                    .child(account.address.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(mac::text_secondary())
                                    .child(account.provider.clone()),
                            )
                    })),
            )
            .child(
                Button::new(
                    "mail-settings-open-internet-accounts",
                    "Open Internet Accounts…",
                )
                .small()
                .on_click(cx.listener(|_, _, _, cx| {
                    cx.spawn(async move |_, _| {
                        if let Err(error) = std::process::Command::new("rmac-system-settings")
                            .arg("--pane")
                            .arg("internet-accounts")
                            .spawn()
                        {
                            eprintln!("rmac-mail: could not open System Settings: {error}");
                        }
                    })
                    .detach();
                })),
            )
    }

    fn junk_mail_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let junk = self.settings.junk.clone();
        div()
            .v_flex()
            .gap(px(2.0))
            .child(
                self.row(
                    Checkbox::new("mail-settings-junk-enabled")
                        .label("Enable junk mail filtering")
                        .checked(junk.filter_enabled)
                        .on_change(cx.listener(|this, value: &bool, _, cx| {
                            this.settings.junk.filter_enabled = *value;
                            this.persist(cx);
                        })),
                ),
            )
            .children(
                JunkMailAction::ALL
                    .into_iter()
                    .enumerate()
                    .map(|(index, action)| {
                        self.row(
                            Checkbox::new(("mail-settings-junk-action", index))
                                .label(action.label())
                                .disabled(!junk.filter_enabled)
                                .checked(junk.action == action)
                                .on_change(cx.listener(move |this, value: &bool, _, cx| {
                                    if *value {
                                        this.settings.junk.action = action;
                                        this.persist(cx);
                                    }
                                })),
                        )
                    }),
            )
    }

    fn stepper(
        &self,
        id: &'static str,
        label: &'static str,
        value: u8,
        range: std::ops::RangeInclusive<u8>,
        on_change: impl Fn(&mut Self, u8, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (min, max) = (*range.start(), *range.end());
        let on_change = std::rc::Rc::new(on_change);
        let decrement = on_change.clone();
        let increment = on_change;
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(div().text_size(px(13.0)).child(label))
            .child(
                Button::new(format!("{id}-decrement"), "−")
                    .small()
                    .disabled(value <= min)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        decrement(this, value.saturating_sub(1).max(min), cx);
                    })),
            )
            .child(
                div()
                    .w(px(24.0))
                    .text_size(px(13.0))
                    .child(value.to_string()),
            )
            .child(
                Button::new(format!("{id}-increment"), "+")
                    .small()
                    .disabled(value >= max)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        increment(this, (value + 1).min(max), cx);
                    })),
            )
    }

    fn fonts_and_colours_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let fonts = self.settings.fonts_and_colours.clone();
        div()
            .v_flex()
            .gap(px(10.0))
            .child(self.stepper(
                "mail-settings-list-font",
                "Message list font size:",
                fonts.message_list_font_size,
                9..=24,
                |this, value, cx| {
                    this.settings.fonts_and_colours.message_list_font_size = value;
                    this.persist(cx);
                },
                cx,
            ))
            .child(self.stepper(
                "mail-settings-message-font",
                "Message font size:",
                fonts.message_font_size,
                9..=24,
                |this, value, cx| {
                    this.settings.fonts_and_colours.message_font_size = value;
                    this.persist(cx);
                },
                cx,
            ))
            .child(
                self.row(
                    Checkbox::new("mail-settings-fixed-width")
                        .label("Use fixed-width font for plain text messages")
                        .checked(fonts.fixed_width_for_plain_text)
                        .on_change(cx.listener(|this, value: &bool, _, cx| {
                            this.settings.fonts_and_colours.fixed_width_for_plain_text = *value;
                            this.persist(cx);
                        })),
                ),
            )
    }

    fn viewing_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let viewing = self.settings.viewing.clone();
        let block_remote = self.settings.privacy.block_all_remote_content;
        div()
            .v_flex()
            .gap(px(10.0))
            .child(self.stepper(
                "mail-settings-preview-lines",
                "Preview lines in the message list:",
                viewing.preview_lines,
                0..=5,
                |this, value, cx| {
                    this.settings.viewing.preview_lines = value;
                    this.persist(cx);
                },
                cx,
            ))
            .child(
                self.row(
                    Checkbox::new("mail-settings-show-to-cc")
                        .label("Show To/Cc labels in the message list")
                        .checked(viewing.show_to_cc_in_list)
                        .on_change(cx.listener(|this, value: &bool, _, cx| {
                            this.settings.viewing.show_to_cc_in_list = *value;
                            this.persist(cx);
                        })),
                ),
            )
            .child(
                self.row(
                    Checkbox::new("mail-settings-load-remote")
                        .label("Load remote content in messages")
                        .checked(viewing.load_remote_content)
                        .on_change(cx.listener(|this, value: &bool, _, cx| {
                            this.settings.viewing.load_remote_content = *value;
                            this.persist(cx);
                        })),
                ),
            )
            .child(
                self.row(
                    Checkbox::new("mail-settings-block-remote")
                        .label("Block all remote content, even when a message asks for it")
                        .checked(block_remote)
                        .on_change(cx.listener(|this, value: &bool, _, cx| {
                            this.settings.privacy.block_all_remote_content = *value;
                            this.persist(cx);
                        })),
                ),
            )
    }

    fn composing_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let composing = self.settings.composing.clone();
        div()
            .v_flex()
            .gap(px(2.0))
            .child(
                div()
                    .text_size(px(13.0))
                    .pb(px(4.0))
                    .child("Message format:"),
            )
            .children(
                ComposeFormat::ALL
                    .into_iter()
                    .enumerate()
                    .map(|(index, format)| {
                        self.row(
                            Checkbox::new(("mail-settings-format", index))
                                .label(format.label())
                                .checked(composing.format == format)
                                .on_change(cx.listener(move |this, value: &bool, _, cx| {
                                    if *value {
                                        this.settings.composing.format = format;
                                        this.persist(cx);
                                    }
                                })),
                        )
                    }),
            )
            .child(
                self.row(
                    Checkbox::new("mail-settings-quote-original")
                        .label("Quote the text of the original message when replying")
                        .checked(composing.quote_original_when_replying)
                        .on_change(cx.listener(|this, value: &bool, _, cx| {
                            this.settings.composing.quote_original_when_replying = *value;
                            this.persist(cx);
                        })),
                ),
            )
            .child(
                self.row(
                    Checkbox::new("mail-settings-spelling")
                        .label("Check spelling while typing")
                        .checked(composing.check_spelling_while_typing)
                        .on_change(cx.listener(|this, value: &bool, _, cx| {
                            this.settings.composing.check_spelling_while_typing = *value;
                            this.persist(cx);
                        })),
                ),
            )
    }

    fn add_signature(&mut self, cx: &mut Context<Self>) {
        let id = self.settings.signatures.next_id();
        let ordinal = self.settings.signatures.signatures.len() + 1;
        self.settings.signatures.signatures.push(Signature {
            id,
            name: format!("Signature {ordinal}"),
            body: "Jacob Samas\nLulo OS".to_owned(),
        });
        self.persist(cx);
    }

    fn delete_signature(&mut self, id: &str, cx: &mut Context<Self>) {
        self.settings
            .signatures
            .signatures
            .retain(|signature| signature.id != id);
        self.settings
            .signatures
            .default_for_account
            .retain(|_, default_id| default_id.as_str() != id);
        self.persist(cx);
    }

    fn set_default_signature(
        &mut self,
        account_address: &str,
        id: &str,
        is_default: bool,
        cx: &mut Context<Self>,
    ) {
        if is_default {
            self.settings
                .signatures
                .default_for_account
                .insert(account_address.to_owned(), id.to_owned());
        } else if self
            .settings
            .signatures
            .default_for_account
            .get(account_address)
            .map(String::as_str)
            == Some(id)
        {
            self.settings
                .signatures
                .default_for_account
                .remove(account_address);
        }
        self.persist(cx);
    }

    fn signatures_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .v_flex()
            .gap(px(8.0))
            .child(
                Button::new("mail-settings-add-signature", "Add Signature")
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| this.add_signature(cx))),
            )
            .child(div().v_flex().gap(px(6.0)).children(
                self.settings.signatures.signatures.iter().map(|signature| {
                    let id_for_delete = signature.id.clone();
                    div()
                        .p(px(8.0))
                        .rounded(px(mac::radius_card()))
                        .bg(mac::control_fill())
                        .v_flex()
                        .gap(px(4.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .child(
                                    div()
                                        .flex_1()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_size(px(13.0))
                                        .child(signature.name.clone()),
                                )
                                .child(
                                    Button::new(
                                        format!("mail-settings-signature-delete-{}", signature.id),
                                        "Delete",
                                    )
                                    .small()
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.delete_signature(&id_for_delete, cx);
                                        },
                                    )),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(mac::text_secondary())
                                .child(signature.body.clone()),
                        )
                        .child(
                            div()
                                .v_flex()
                                .gap(px(2.0))
                                .children(self.accounts.iter().map(|account| {
                                    let checked = self
                                        .settings
                                        .signatures
                                        .default_for_account
                                        .get(&account.address)
                                        .map(String::as_str)
                                        == Some(signature.id.as_str());
                                    let id_for_default = signature.id.clone();
                                    let address = account.address.clone();
                                    Checkbox::new(format!(
                                        "mail-settings-signature-default-{}-{}",
                                        signature.id, account.address
                                    ))
                                    .label(format!("Use for {}", account.address))
                                    .checked(checked)
                                    .on_change(cx.listener(move |this, value: &bool, _, cx| {
                                        this.set_default_signature(
                                            &address,
                                            &id_for_default,
                                            *value,
                                            cx,
                                        );
                                    }))
                                })),
                        )
                }),
            ))
            .when(self.settings.signatures.signatures.is_empty(), |parent| {
                parent.child(
                    div()
                        .text_size(px(12.0))
                        .text_color(mac::text_secondary())
                        .child("No signatures yet. Add one to insert it automatically on compose."),
                )
            })
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pane = self.pane;
        div()
            .size_full()
            .v_flex()
            .track_focus(&self.focus)
            .key_context("Mail")
            .on_action(cx.listener(|_, _: &rmac_ui::RequestClose, window, _| {
                window.remove_window();
            }))
            .on_action(cx.listener(|_, _: &crate::ShowSettings, window, _| {
                window.activate_window();
            }))
            .bg(mac::window())
            .text_color(mac::text())
            .child(rmac_ui::title_bar_content(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(13.0))
                    .child("Mail Settings"),
            ))
            .child(self.tabs(cx))
            .when_some(self.save_error.clone(), |parent, error| {
                parent.child(
                    div()
                        .px(px(16.0))
                        .py(px(6.0))
                        .text_size(px(12.0))
                        .text_color(mac::danger())
                        .child(format!("Settings could not be saved: {error}")),
                )
            })
            .child(
                div()
                    .id("mail-settings-pane")
                    .flex_1()
                    .overflow_y_scroll()
                    .px(px(16.0))
                    .py(px(12.0))
                    .map(|parent| match pane {
                        Pane::General => parent.child(self.general_pane(cx)),
                        Pane::Accounts => parent.child(self.accounts_pane(cx)),
                        Pane::JunkMail => parent.child(self.junk_mail_pane(cx)),
                        Pane::FontsAndColours => parent.child(self.fonts_and_colours_pane(cx)),
                        Pane::Viewing => parent.child(self.viewing_pane(cx)),
                        Pane::Composing => parent.child(self.composing_pane(cx)),
                        Pane::Signatures => parent.child(self.signatures_pane(cx)),
                    }),
            )
    }
}
