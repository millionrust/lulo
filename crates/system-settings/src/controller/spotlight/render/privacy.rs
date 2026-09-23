//! Spotlight file privacy, removable scope, and exclusions projection.

use super::*;

impl Settings {
    pub(super) fn append_spotlight_privacy(
        &self,
        view: Entity<Self>,
        settings: &rmac_shell_settings::ShellSettings,
        cards: &mut Vec<Div>,
    ) {
        let enabled = !self.shell_settings_busy;
        let files = spotlight_provider_policy(settings, rmac_launcher_providers::FILES_PROVIDER);
        let choose_view = view.clone();
        cards.push(section_header("File privacy and scope"));
        let private_view = view.clone();
        let removable_view = view.clone();
        cards.push(card(vec![
            row_base()
                .child(text_block(
                    "Allow private file results".into(),
                    Some("Admit local filenames and recent-document paths to Search".into()),
                ))
                .child(
                    Toggle::new("spotlight-private-files")
                        .checked(files.allow_private_content)
                        .disabled(!enabled)
                        .on_click(move |value, _, cx| {
                            private_view.update(cx, |settings, cx| {
                                settings.apply_spotlight_change(
                                    SpotlightChange::ProviderPrivateContent {
                                        id: rmac_launcher_providers::FILES_PROVIDER.into(),
                                        allowed: *value,
                                    },
                                    cx,
                                )
                            });
                        }),
                )
                .into_any_element(),
            row_base()
                .child(text_block(
                    "Include removable mounts".into(),
                    Some("Allow on-demand file search to cross filesystem boundaries".into()),
                ))
                .child(
                    Toggle::new("spotlight-removable-mounts")
                        .checked(settings.spotlight.include_removable_mounts)
                        .disabled(!enabled)
                        .on_click(move |value, _, cx| {
                            removable_view.update(cx, |settings, cx| {
                                settings.apply_spotlight_change(
                                    SpotlightChange::IncludeRemovableMounts(*value),
                                    cx,
                                )
                            });
                        }),
                )
                .into_any_element(),
        ]));
        cards.push(note_card(
            "File search is local and on demand. rmac does not build a perpetual content index, and no built-in provider requests network access.",
        ));

        cards.push(
            div()
                .flex()
                .items_center()
                .justify_between()
                .px(px(style::ROW_PADDING))
                .pt(px(style::SECTION_TOP))
                .pb(px(style::SECTION_BOTTOM))
                .child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .font_weight(rmac_ui::mac::BOLD)
                        .text_color(style::heading_text())
                        .child("Excluded folders"),
                )
                .child(
                    Button::new("spotlight-add-exclusion", "Add Folder…")
                        .disabled(!enabled)
                        .on_click(move |_, _, cx| {
                            choose_view
                                .update(cx, |settings, cx| settings.choose_search_exclusion(cx));
                        }),
                ),
        );
        if settings.spotlight.excluded_paths.is_empty() {
            cards.push(note_card(
                "No folders are excluded. Add a folder to prune it before filename traversal and recent-document admission.",
            ));
        } else {
            let exclusion_rows = settings
                .spotlight
                .excluded_paths
                .iter()
                .enumerate()
                .map(|(index, path)| {
                    let remove_view = view.clone();
                    let remove_path = path.clone();
                    row_base()
                        .child(text_block(
                            PathBuf::from(path)
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_else(|| path.clone())
                                .into(),
                            Some(path.clone().into()),
                        ))
                        .child(
                            Button::new(
                                ElementId::from(SharedString::from(format!(
                                    "spotlight-remove-exclusion-{index}"
                                ))),
                                "Remove",
                            )
                            .disabled(!enabled)
                            .on_click(move |_, _, cx| {
                                remove_view.update(cx, |settings, cx| {
                                    settings.apply_spotlight_change(
                                        SpotlightChange::RemoveExclusion(remove_path.clone()),
                                        cx,
                                    )
                                });
                            }),
                        )
                        .into_any_element()
                })
                .collect();
            cards.push(card(exclusion_rows));
        }
    }
}
