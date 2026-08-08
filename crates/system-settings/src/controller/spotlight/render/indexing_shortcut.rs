//! Spotlight indexing and global-shortcut projection.

use super::*;

impl Settings {
    pub(super) fn append_spotlight_indexing_shortcut(
        &self,
        view: Entity<Self>,
        settings: &rmac_shell_settings::ShellSettings,
        cards: &mut Vec<Div>,
    ) {
        let configure_shortcuts_view = view;
        cards.push(section_header("Indexing"));
        cards.push(card(vec![
            value_row(
                "icons/search.svg",
                accent(),
                "Search mode".into(),
                "On demand".into(),
            ),
            value_row(
                "icons/hard-drive.svg",
                secondary(),
                "Filesystem scope".into(),
                if settings.spotlight.include_removable_mounts {
                    "Home and removable mounts".into()
                } else {
                    "Home filesystem only".into()
                },
            ),
            value_row(
                "icons/info.svg",
                secondary(),
                "Background content index".into(),
                "Not used".into(),
            ),
        ]));

        let launcher_shortcut = rmac_shortcuts::default_shortcuts()
            .into_iter()
            .find(|shortcut| shortcut.id.0 == "launcher")
            .expect("the stable launcher shortcut is registered");
        let shortcut_status: SharedString = match self.shortcut_status.as_ref() {
            Some(rmac_shortcuts::BackendStatus::Portal {
                version,
                can_configure,
            }) => format!(
                "Portal v{version}{}",
                if *can_configure && *version >= rmac_shortcuts::PORTAL_CONFIGURE_VERSION {
                    " · configurable"
                } else {
                    ""
                }
            )
            .into(),
            Some(rmac_shortcuts::BackendStatus::FallbackRequired { .. }) => {
                "niri fallback required".into()
            }
            None if self.shortcut_status_loading => "Loading…".into(),
            None => "Not reported".into(),
        };
        let shortcut_configuration_enabled =
            shortcut_configuration_available(self.shortcut_status.as_ref());
        let shortcut_configuration_detail = match self.shortcut_status.as_ref() {
            Some(rmac_shortcuts::BackendStatus::Portal {
                version,
                can_configure: true,
            }) if *version >= rmac_shortcuts::PORTAL_CONFIGURE_VERSION => {
                "Open the portal UI for every shortcut in the live rmac session"
            }
            Some(rmac_shortcuts::BackendStatus::Portal { .. }) => {
                "The active portal is older than GlobalShortcuts version 2"
            }
            Some(rmac_shortcuts::BackendStatus::FallbackRequired { .. }) => {
                "The generated niri fallback remains the shortcut authority"
            }
            None if self.shortcut_status_loading => "Waiting for the session broker",
            None => "The session broker has not reported its shortcut backend",
        };
        cards.push(section_header("Keyboard shortcut"));
        cards.push(card(vec![
            value_row(
                "icons/keyboard.svg",
                accent(),
                "Active backend".into(),
                shortcut_status,
            ),
            value_row(
                "icons/keyboard.svg",
                secondary(),
                "Portal preference".into(),
                launcher_shortcut.preferred_trigger.into(),
            ),
            value_row(
                "icons/keyboard.svg",
                secondary(),
                "niri fallback".into(),
                launcher_shortcut.niri_trigger.into(),
            ),
            row_base()
                .child(text_block(
                    "Global shortcuts".into(),
                    Some(shortcut_configuration_detail.into()),
                ))
                .child(
                    Button::new(
                        "spotlight-configure-shortcuts",
                        if self.shortcut_configuration_busy {
                            "Opening…"
                        } else {
                            "Configure…"
                        },
                    )
                    .disabled(self.shortcut_configuration_busy || !shortcut_configuration_enabled)
                    .on_click(move |_, _, cx| {
                        configure_shortcuts_view.update(cx, |settings, cx| {
                            settings.configure_global_shortcuts(cx);
                        });
                    }),
                )
                .into_any_element(),
        ]));
        if let Some(error) = self.shortcut_status_error.clone() {
            cards.push(note_card(error));
        }
        if let Some(error) = self.shortcut_configuration_error.clone() {
            cards.push(note_card(error));
        }
        cards.push(note_card(
            "The portal owns user consent and the actual trigger. Configure opens its UI through the broker's existing session; it never creates a second binding authority. The fallback is enabled only when the broker reports it is required, so one shortcut backend owns Logo/Mod+Space at a time.",
        ));
        cards.push(note_card(
            "These preferences are consumed by the launcher provider/runtime foundations. The centered GPUI overlay and full live session wiring remain D7/D8 release gates.",
        ));
    }
}
