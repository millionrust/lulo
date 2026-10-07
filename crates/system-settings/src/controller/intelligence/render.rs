//! Lulo Intelligence pane presentation, in the grammar of macOS 26's
//! Apple Intelligence & Siri pane: a header card, the main switch, then the
//! model group (which model, its download, its size), with the reasons and
//! privacy facts as footnotes.

use rmac_intelligence::gate::Decision;
use rmac_intelligence::manifest::Tier;

use super::*;

/// "533 MB", decimal like Finder.
pub(in crate::controller) fn megabytes(bytes: u64) -> String {
    let megabytes = bytes as f64 / 1_000_000.0;
    if megabytes >= 1000.0 {
        format!("{:.1} GB", megabytes / 1000.0)
    } else {
        format!("{megabytes:.0} MB")
    }
}

impl Settings {
    pub(in crate::controller) fn render_intelligence(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let pane = &self.intelligence;
        let mut cards = vec![header_card(
            tile26("icons/sparkles.svg", rmac_ui::mac::system_purple()),
            "Lulo Intelligence",
            "Type a request such as “turn on dark mode” or “set a timer for 10 minutes” in Spotlight, and Lulo offers to do it. The model runs on this computer; nothing you type leaves it.",
            None,
        )];
        if !pane.loaded {
            cards.push(footnote("Checking this computer…"));
            return self.pane(cards);
        }
        let decision = pane.config.decision(&pane.facts);
        let offered = !matches!(decision, Decision::NotOffered(_));
        let toggle_view = view.clone();
        cards.push(card(vec![switch_row(
            "intelligence-enabled",
            "Lulo Intelligence",
            Some(
                "Spotlight offers to carry out requests. Settings changes always ask first.".into(),
            ),
            pane.config.enabled && offered,
            offered && !pane.busy,
            move |enabled, _, cx| {
                toggle_view.update(cx, |settings, cx| {
                    settings.set_intelligence_enabled(enabled, cx)
                });
            },
        )]));
        match &decision {
            Decision::NotOffered(reason) => {
                cards.push(footnote(format!(
                    "{reason} Lulo Intelligence is not available on this computer."
                )));
            }
            Decision::Offered {
                standard, reason, ..
            } => {
                cards.push(section_header("Model"));
                let tier = self.intelligence_tier();
                let mut rows = Vec::new();
                if *standard {
                    let choices: Vec<PopupChoice> = Tier::ALL
                        .into_iter()
                        .map(|choice_tier| {
                            let choice_view = view.clone();
                            let selected = choice_tier == tier;
                            choice(choice_tier.model().display_name, selected, move |_, cx| {
                                if !selected {
                                    choice_view.update(cx, |settings, cx| {
                                        settings.choose_intelligence_tier(choice_tier, cx)
                                    });
                                }
                            })
                        })
                        .collect();
                    let current = popup_value(&choices, tier.model().display_name);
                    rows.push(popup_row(
                        "intelligence-model",
                        "Model",
                        None,
                        current,
                        choices,
                        pane.download.is_none() && !pane.busy,
                    ));
                } else {
                    rows.push(fact_row("Model", tier.model().display_name));
                }
                rows.push(self.intelligence_download_row(tier, &view));
                cards.push(card(rows));
                let status = if pane.calibrating {
                    " Checking this computer’s speed…".to_owned()
                } else {
                    String::new()
                };
                cards.push(footnote(format!(
                    "{reason}{status} The model is downloaded once, from a fixed version on Hugging Face, and checked against its fingerprint before it is used. Licence: {}.",
                    tier.model().licence
                )));
            }
        }
        if let Some(error) = &pane.error {
            cards.push(note_card(error.clone()));
        }
        self.pane(cards)
    }

    fn intelligence_download_row(&self, tier: Tier, view: &Entity<Settings>) -> AnyElement {
        let pane = &self.intelligence;
        let model = tier.model();
        let size = megabytes(model.size);
        let index = tier_index(tier);
        let (status, button): (String, AnyElement) = if let Some(download) = &pane.download {
            let percent = download.done * 100 / download.total.max(1);
            let cancel_view = view.clone();
            (
                format!(
                    "Downloading {} model… {percent}% of {size}",
                    match download.tier {
                        Tier::Tiny => "Tiny",
                        Tier::Standard => "Standard",
                    }
                ),
                push_button("intelligence-download-cancel", "Stop")
                    .on_click(move |_, _, cx| {
                        cancel_view
                            .update(cx, |settings, cx| settings.cancel_intelligence_download(cx));
                    })
                    .into_any_element(),
            )
        } else if pane.present[index] {
            let remove_view = view.clone();
            (
                format!("Downloaded · {size}"),
                push_button("intelligence-remove", "Remove Model")
                    .disabled(pane.busy)
                    .on_click(move |_, _, cx| {
                        remove_view
                            .update(cx, |settings, cx| settings.remove_intelligence_model(cx));
                    })
                    .into_any_element(),
            )
        } else {
            let partial = pane.partial[index];
            let download_view = view.clone();
            let (status, label) = if partial > 0 {
                (
                    format!(
                        "Download paused at {}% of {size}",
                        partial * 100 / model.size.max(1)
                    ),
                    "Resume",
                )
            } else {
                (format!("Not downloaded · {size}"), "Download")
            };
            (
                status,
                push_button("intelligence-download", label)
                    .disabled(pane.busy)
                    .on_click(move |_, _, cx| {
                        download_view
                            .update(cx, |settings, cx| settings.download_intelligence_model(cx));
                    })
                    .into_any_element(),
            )
        };
        value_button_row("Download", Some(status.into()), None, Some(button))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_like_finder() {
        assert_eq!(megabytes(532_517_120), "533 MB");
        assert_eq!(megabytes(1_280_835_840), "1.3 GB");
    }
}
