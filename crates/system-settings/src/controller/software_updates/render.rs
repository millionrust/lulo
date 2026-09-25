//! Software Update presentation, measured from macOS 26.2's pane
//! (design-lab/software-update.html): the Lulo OS card with its release
//! notes, "Also Available" Other Updates, Installed, and Automatic Updates.

use super::*;

mod dialog;

/// The OS icon is 32 at (8, 10); the text starts at x 46.
const ITEM_ICON: f32 = 32.0;
const ITEM_ICON_X: f32 = 8.0;
const ITEM_TEXT_GAP: f32 = 6.0;
/// The primary card's ⓘ is 15, 12 from the card's right edge.
const ITEM_INFO: f32 = 15.0;
const ITEM_BUTTON_GAP: f32 = 9.0;
/// Release notes: 16 between blocks.
const NOTES_BLOCK_GAP: f32 = 16.0;

fn lulo_icon() -> AnyElement {
    tile("icons/lulo.svg", hsl(0xff8a1e), ITEM_ICON).into_any_element()
}

fn other_updates_icon() -> AnyElement {
    tile("icons/refresh-cw.svg", hsl(0x0a84ff), ITEM_ICON).into_any_element()
}

/// The primary card's 15 pt circled i.
fn item_info_button(
    id: &'static str,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .size(px(ITEM_INFO))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .role(gpui::Role::Button)
        .aria_label("More Info")
        .tooltip(|window, cx| rmac_ui::tooltip_view("More Info", window, cx))
        .on_click(move |_, window, cx| on_click(window, cx))
        .child(glyph("icons/info.svg", ITEM_INFO, secondary()))
        .into_any_element()
}

/// An item's header row: icon, 13 pt title over the 11 pt subtitle, then
/// its buttons and ⓘ.
fn item_header(icon: AnyElement, title: String, subtitle: String, buttons: Vec<AnyElement>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(ITEM_TEXT_GAP))
        .min_h(px(style::LARGE_ROW_HEIGHT))
        .pl(px(ITEM_ICON_X))
        .pr(px(12.0))
        .py(px(10.0))
        .child(icon)
        .child(large_text(title, Some(subtitle_text(subtitle))))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(ITEM_BUTTON_GAP))
                .children(buttons),
        )
}

/// The release notes: the lead paragraph at 13 pt, then each bold 11 pt
/// heading with its 11 pt secondary body, 16 apart.
fn release_notes(blocks: &[rmac_updates::NotesBlock]) -> Div {
    let mut column = div()
        .v_flex()
        .gap(px(NOTES_BLOCK_GAP))
        .px(px(style::ROW_PADDING))
        .py(px(style::ROW_PADDING));
    let mut index = 0;
    while index < blocks.len() {
        match &blocks[index] {
            rmac_updates::NotesBlock::Paragraph(text) if index == 0 => {
                column = column.child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .line_height(px(16.0))
                        .text_color(label())
                        .child(text.clone()),
                );
                index += 1;
            }
            rmac_updates::NotesBlock::Heading(heading) => {
                let mut section = div().v_flex().child(
                    div()
                        .text_size(rmac_ui::text_px(11.0))
                        .line_height(px(14.0))
                        .font_weight(rmac_ui::mac::BOLD)
                        .text_color(label())
                        .child(heading.clone()),
                );
                index += 1;
                if let Some(rmac_updates::NotesBlock::Paragraph(body)) = blocks.get(index) {
                    section = section.child(notes_body(body.clone()));
                    index += 1;
                }
                column = column.child(section);
            }
            rmac_updates::NotesBlock::Paragraph(text) => {
                column = column.child(notes_body(text.clone()));
                index += 1;
            }
        }
    }
    column
}

fn notes_body(text: String) -> Div {
    div()
        .text_size(rmac_ui::text_px(11.0))
        .line_height(px(14.0))
        .text_color(secondary())
        .child(text)
}

/// What the notes say when the archive carries none (a component-only
/// update, or a release published before notes existed).
pub(super) fn package_list_notes(packages: &[rmac_updates::Update]) -> rmac_updates::ReleaseNotes {
    let list = packages
        .iter()
        .map(|update| {
            format!(
                "{} {}",
                update.name,
                rmac_updates::display_version(&update.version)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    rmac_updates::ReleaseNotes {
        blocks: vec![rmac_updates::NotesBlock::Paragraph(format!(
            "This update includes {list}."
        ))],
    }
}

fn card_footnote(text: impl Into<SharedString>) -> Div {
    div()
        .px(px(style::ROW_PADDING))
        .py(px(11.0))
        .text_size(rmac_ui::text_px(11.0))
        .line_height(px(14.0))
        .text_color(secondary())
        .child(text.into())
}

impl Settings {
    fn update_progress_row(&self) -> Option<AnyElement> {
        if self.updates_preparing {
            return Some(
                div()
                    .px(px(style::ROW_PADDING))
                    .pb(px(12.0))
                    .child(Progress::indeterminate().label("Verifying the update…"))
                    .into_any_element(),
            );
        }
        if !self.updates_installing {
            return None;
        }
        let progress = self.updates_progress.clone().unwrap_or_default();
        let mut text = progress.phase.label().to_string();
        if let Some(package) = &progress.current_package {
            text.push_str(&format!(" · {package}"));
        }
        if let Some(remaining) = progress.remaining_seconds {
            text.push_str(&format!(
                " · about {}",
                format_power_duration(u64::from(remaining))
            ));
        }
        let bar = match progress.percentage {
            Some(percentage) => Progress::new(f32::from(percentage) / 100.0),
            None => Progress::indeterminate(),
        }
        .label(text);
        Some(
            div()
                .px(px(style::ROW_PADDING))
                .pb(px(12.0))
                .child(bar)
                .into_any_element(),
        )
    }

    /// Update Now / Restart Now / Cancel for one item.
    fn item_buttons(
        &self,
        id: &'static str,
        ready: bool,
        selection: Vec<String>,
        target: UpdateTarget,
        can_install: bool,
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        let view = cx.entity();
        let running_here = self.updates_target == Some(target);
        if running_here && (self.updates_preparing || self.updates_installing) {
            let progress = self.updates_progress.clone().unwrap_or_default();
            let cancel_requested = self
                .updates_cancellation
                .as_ref()
                .is_some_and(rmac_updates::Cancellation::is_cancelled);
            let can_cancel = self.updates_preparing || progress.allow_cancel;
            return vec![push_button(
                SharedString::from(format!("{id}-cancel")),
                if cancel_requested {
                    "Cancelling…"
                } else {
                    "Cancel"
                },
            )
            .busy(cancel_requested)
            .disabled(!can_cancel || cancel_requested)
            .on_click(move |_, _, cx| {
                view.update(cx, |settings, cx| settings.cancel_update_operation(cx));
            })
            .into_any_element()];
        }
        if ready {
            return vec![
                push_button(SharedString::from(format!("{id}-restart")), "Restart Now")
                    .disabled(self.updates_busy)
                    .on_click(move |_, _, cx| {
                        view.update(cx, |settings, cx| settings.restart_to_update(cx));
                    })
                    .into_any_element(),
            ];
        }
        vec![
            push_button(SharedString::from(format!("{id}-update")), "Update Now")
                .disabled(self.updates_busy || !can_install)
                .on_click(move |_, _, cx| {
                    let selection = selection.clone();
                    view.update(cx, |settings, cx| {
                        settings.start_update(selection, target, cx)
                    });
                })
                .into_any_element(),
        ]
    }

    pub(in crate::controller) fn software_update_body(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let installed = card(vec![fact_row(
            "Installed",
            self.sysinfo.operating_system.clone(),
        )]);
        let auto_view = view.clone();
        let automatic = card(vec![row_base()
            .child(text_block("Automatic Updates".into(), None))
            .child(trailing_value(self.updates_auto.summary()))
            .child(info_button(
                "software-update-automatic-info",
                "Automatic Updates",
                move |_, cx| {
                    auto_view.update(cx, |settings, cx| settings.open_update_auto_sheet(cx));
                },
            ))
            .into_any_element()]);
        let legal = card(vec![div()
            .px(px(style::ROW_PADDING))
            .py(px(style::ROW_PADDING))
            .text_size(rmac_ui::text_px(11.0))
            .line_height(px(14.0))
            .text_color(secondary())
            .child(
                "Updates come only from the signed Lulo OS and Ubuntu archives. Each package's \
                 licence is in /usr/share/doc.",
            )
            .into_any_element()]);
        let mut body = div().v_flex();
        for message in [&self.updates_error, &self.updates_stream_error]
            .into_iter()
            .flatten()
        {
            body = body.child(note_card(message.clone()));
        }

        if self.updates_loading && self.updates.is_none() {
            return body
                .child(
                    Progress::indeterminate()
                        .label("Checking for updates…")
                        .mb_3(),
                )
                .child(installed)
                .child(automatic);
        }

        let Some(snapshot) = &self.updates else {
            let refresh_view = view.clone();
            return body
                .child(
                    EmptyState::new("Software Update is unavailable")
                        .message("PackageKit, the system's update service, did not respond")
                        .error(true),
                )
                .child(installed)
                .child(automatic)
                .child(footer_buttons(vec![push_button(
                    "refresh-update-status",
                    "Try Again",
                )
                .busy(self.updates_busy)
                .disabled(self.updates_busy)
                .on_click(move |_, _, cx| {
                    refresh_view.update(cx, |settings, cx| settings.refresh_update_status(cx));
                })
                .into_any_element()]));
        };

        let catalog = rmac_updates::Catalog::from_snapshot(snapshot);
        let can_install = snapshot.install_supported && !snapshot.truncated;

        if catalog.is_empty() {
            let status = if self.updates_busy {
                "Checking for updates…".to_string()
            } else {
                "Lulo OS is up to date".to_string()
            };
            body = body.child(card(vec![item_header(
                lulo_icon(),
                status,
                self.sysinfo.operating_system.clone(),
                Vec::new(),
            )
            .into_any_element()]));
        }

        let mut first_card = true;
        if let Some(lulo) = &catalog.lulo_os {
            first_card = false;
            let ids = lulo.package_ids();
            let ready = snapshot.offline.ready(&ids);
            let subtitle = if ready {
                "Ready to install — restart to finish".to_string()
            } else {
                lulo.subtitle()
            };
            let info_view = view.clone();
            let mut buttons = self.item_buttons(
                "software-update-lulo",
                ready,
                vec![rmac_updates::LULO_OS_ITEM.to_owned()],
                UpdateTarget::LuloOs,
                can_install,
                cx,
            );
            buttons.push(item_info_button(
                "software-update-lulo-info",
                move |_, cx| {
                    info_view.update(cx, |settings, cx| {
                        settings.open_update_info_sheet(rmac_updates::LULO_OS_ITEM.to_owned(), cx)
                    });
                },
            ));
            let notes = snapshot
                .release_notes
                .clone()
                .unwrap_or_else(|| package_list_notes(&lulo.packages));
            let mut group =
                group().child(item_header(lulo_icon(), lulo.title(), subtitle, buttons));
            if matches!(
                self.updates_target,
                Some(UpdateTarget::LuloOs | UpdateTarget::Selection)
            ) {
                if let Some(progress) = self.update_progress_row() {
                    group = group.child(progress);
                }
            }
            group = group
                .child(row_separator())
                .child(release_notes(&notes.blocks))
                .child(row_separator())
                .child(card_footnote(if ready {
                    "Lulo OS will install this update while it restarts."
                } else {
                    "Once downloaded, this update will be installed when you restart."
                }));
            body = body.child(group);
        }

        if !catalog.other.is_empty() {
            if !first_card {
                body = body.child(section_header("Also Available"));
            }
            let ids = catalog.other_ids();
            let ready = snapshot.offline.ready(&ids);
            let subtitle = if ready {
                "Ready to install — restart to finish".to_string()
            } else {
                let mut text = catalog.other_summary().unwrap_or_default();
                if let Some(size) = catalog.other_size(snapshot) {
                    text.push_str(&format!(" — {}", rmac_updates::format_size(size)));
                }
                text
            };
            let first_other = catalog
                .other
                .first()
                .map(|update| update.package_id.clone())
                .unwrap_or_default();
            let info_view = view.clone();
            let mut buttons = self.item_buttons(
                "software-update-other",
                ready,
                ids,
                UpdateTarget::Other,
                can_install,
                cx,
            );
            buttons.push(item_info_button(
                "software-update-other-info",
                move |_, cx| {
                    let first = first_other.clone();
                    info_view.update(cx, |settings, cx| {
                        settings.open_update_info_sheet(first, cx)
                    });
                },
            ));
            let mut group = group().child(item_header(
                other_updates_icon(),
                "Other Updates".into(),
                subtitle,
                buttons,
            ));
            let progress_here = self.updates_target == Some(UpdateTarget::Other)
                || (self.updates_target == Some(UpdateTarget::Selection) && first_card);
            if progress_here {
                if let Some(progress) = self.update_progress_row() {
                    group = group.child(progress);
                }
            }
            body = body.child(group);
        }

        body = body.child(installed).child(automatic);
        if let Some(error) = &self.updates_auto_error {
            body = body.child(footnote(error.clone()));
        }
        if !snapshot.install_supported && !catalog.is_empty() {
            body = body.child(footnote(
                snapshot
                    .install_unavailable_reason
                    .clone()
                    .unwrap_or_else(|| {
                        "The PackageKit backend cannot install updates on this system.".into()
                    }),
            ));
        }
        if snapshot.truncated {
            body = body.child(footnote(
                "More updates are available than this list can show, so they can't be reviewed \
                 or installed here.",
            ));
        }
        if catalog.blocked > 0 {
            body = body.child(footnote(if catalog.blocked == 1 {
                "1 update is held back by the package manager and won't be installed.".to_string()
            } else {
                format!(
                    "{} updates are held back by the package manager and won't be installed.",
                    catalog.blocked
                )
            }));
        }
        body.child(legal)
    }
}
