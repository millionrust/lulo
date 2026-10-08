//! Sound on Windows: the alert sound Lulo apps play (`rmac-sound`) and a
//! read-out of the Windows output device and its volume.
//!
//! Lulo plays its alerts through Windows' own system sounds, at the Windows
//! volume (`rmac-sound`'s Windows backend), so there is no separate alert
//! volume here, and the output volume itself is changed with Windows'
//! volume control: Settings only shows it, read when the pane opens and
//! whenever the window comes forward.

use gpui::{div, Context, Div, ParentElement as _, SharedString};
use rmac_ui::StyledExt as _;

use super::form::{
    card, choice, first_section_header, footnote, note_card, popup_row, section_header, value_row,
};
use super::WinSettings;

/// The alert sounds a user can pick, as on Lulo OS (`Settings::normalized`
/// keeps every other cue out of `alert_sound`).
const ALERTS: [rmac_sound::Cue; 3] = [
    rmac_sound::Cue::Alert,
    rmac_sound::Cue::Error,
    rmac_sound::Cue::Notification,
];

/// "42%", or "Muted".
pub(super) fn volume_label(reading: &super::host::VolumeReading) -> String {
    if reading.muted {
        "Muted".to_owned()
    } else {
        format!("{}%", reading.percent)
    }
}

impl WinSettings {
    pub(super) fn load_sound(&mut self, cx: &mut Context<Self>) {
        let task = cx
            .background_executor()
            .spawn(async { rmac_sound::load_settings().map_err(|error| error.to_string()) });
        cx.spawn(async move |this, cx| {
            let loaded = task.await;
            let _ = this.update(cx, |this, cx| {
                this.sound = Some(loaded.map_err(SharedString::from));
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn load_volume(&mut self, cx: &mut Context<Self>) {
        let host = self.host.clone();
        let task = cx
            .background_executor()
            .spawn(async move { host.output_volume() });
        cx.spawn(async move |this, cx| {
            let reading = task.await;
            let _ = this.update(cx, |this, cx| {
                this.volume = Some(reading.map_err(SharedString::from));
                cx.notify();
            });
        })
        .detach();
    }

    fn choose_alert(&mut self, cue: rmac_sound::Cue, cx: &mut Context<Self>) {
        let Some(Ok(current)) = &self.sound else {
            return;
        };
        let next = rmac_sound::Settings {
            alert_sound: cue,
            ..current.clone()
        };
        // Let the user hear it, as the Mac does on choosing an alert.
        let _ = rmac_sound::preview(cue, next.alert_volume);
        self.sound = Some(Ok(next.clone()));
        self.sound_error = None;
        cx.notify();
        let task = cx.background_executor().spawn(async move {
            rmac_sound::save_settings(&next).map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            let saved = task.await;
            let _ = this.update(cx, |this, cx| {
                if let Err(error) = saved {
                    this.sound_error =
                        Some(format!("The alert sound was not saved: {error}.").into());
                    this.load_sound(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_sound(&self, cx: &mut Context<Self>) -> Div {
        let mut cards = vec![first_section_header("Sound Effects")];
        match &self.sound {
            None => cards.push(note_card("Reading Lulo's sound settings…")),
            Some(Err(error)) => cards.push(note_card(format!(
                "Lulo's sound settings could not be read: {error}."
            ))),
            Some(Ok(settings)) => {
                let view = cx.entity();
                let choices = ALERTS
                    .into_iter()
                    .map(|cue| {
                        let view = view.clone();
                        choice(
                            cue.display_name(),
                            settings.alert_sound == cue,
                            move |_, cx| {
                                view.update(cx, |settings, cx| settings.choose_alert(cue, cx));
                            },
                        )
                    })
                    .collect();
                cards.push(card(vec![popup_row(
                    "sound-alert",
                    "Alert sound",
                    choices,
                    true,
                )]));
                cards.push(footnote(
                    "Lulo apps play their alerts through Windows' system sounds, at the Windows volume.",
                ));
            }
        }
        if let Some(error) = &self.sound_error {
            cards.push(note_card(error.clone()));
        }

        cards.push(section_header("Output"));
        match &self.volume {
            None => cards.push(card(vec![value_row("Output volume", "Reading…")])),
            Some(Err(error)) => cards.push(note_card(error.clone())),
            Some(Ok(reading)) => {
                let mut rows = Vec::new();
                if let Some(device) = &reading.device {
                    rows.push(value_row("Output device", device.clone()));
                }
                rows.push(value_row("Output volume", volume_label(reading)));
                cards.push(card(rows));
                cards.push(footnote(
                    "Change the volume and the output device with the volume control in the Windows taskbar.",
                ));
            }
        }
        div().v_flex().children(cards)
    }
}

#[cfg(test)]
mod tests {
    use super::super::host::{tests::FakeHost, Host as _};
    use super::*;

    #[test]
    fn volume_reads_as_a_percentage_or_muted() {
        let mut reading = FakeHost.output_volume().unwrap();
        assert_eq!(volume_label(&reading), "42%");
        reading.muted = true;
        assert_eq!(volume_label(&reading), "Muted");
    }

    #[test]
    fn only_the_alert_cues_are_offered() {
        for cue in ALERTS {
            let settings = rmac_sound::Settings {
                alert_sound: cue,
                ..rmac_sound::Settings::default()
            }
            .normalized();
            assert_eq!(settings.alert_sound, cue);
        }
    }
}
