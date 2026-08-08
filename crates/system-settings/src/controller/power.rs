//! Battery and power-profile settings authority.

mod render;

use super::*;

fn power_profile_row(profile: rmac_power::PowerProfile, selected: bool, disabled: bool) -> ListRow {
    let foreground = if selected { on_accent() } else { label() };
    let secondary_foreground = if selected { on_accent() } else { secondary() };
    ListRow::new(
        ElementId::from(SharedString::from(format!(
            "power-profile-{}",
            profile.id()
        ))),
        div()
            .w_full()
            .flex()
            .items_center()
            .gap_3()
            .child(tile("icons/power.svg", secondary_foreground, 22.0))
            .child(
                div()
                    .flex_1()
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(foreground)
                    .child(profile.label()),
            )
            .when(selected, |row| {
                row.child(glyph("icons/check.svg", 14.0, on_accent()))
            }),
    )
    .selected(selected)
    .disabled(disabled)
    .h(px(44.0))
    .px_3()
}

impl Settings {
    pub(super) fn finish_power_update(
        &mut self,
        result: std::result::Result<rmac_power::Snapshot, rmac_power::Error>,
        cx: &mut Context<Self>,
    ) {
        let refresh_pending = std::mem::take(&mut self.power_refresh_pending);
        self.power_loading = false;
        self.power_busy = false;
        match result {
            Ok(snapshot) => {
                self.power = snapshot;
                self.power_error = None;
                self.power_stream_error = None;
            }
            Err(error) => {
                self.power_error = Some(format!("Could not update Battery: {error}").into());
            }
        }
        if refresh_pending {
            self.refresh_power(cx);
        }
    }

    pub(super) fn finish_power_stream_update(
        &mut self,
        result: std::result::Result<rmac_power::Snapshot, rmac_power::Error>,
    ) {
        match result {
            Ok(snapshot) => {
                self.power = snapshot;
                self.power_stream_error = None;
            }
            Err(_) => {
                self.power_stream_error =
                    Some("Live battery state could not be refreshed from UPower".into());
            }
        }
    }

    pub(super) fn refresh_power(&mut self, cx: &mut Context<Self>) {
        if self.power_loading || self.power_busy {
            return;
        }
        self.power_generation = self.power_generation.wrapping_add(1);
        self.power_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_power::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_power_update(result, cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn set_power_profile(
        &mut self,
        profile: rmac_power::PowerProfile,
        cx: &mut Context<Self>,
    ) {
        if self.power_loading
            || self.power_busy
            || !self.power.profiles.available
            || !self.power.profiles.supported.contains(&profile)
        {
            return;
        }
        self.power_generation = self.power_generation.wrapping_add(1);
        self.power_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { apply_power_profile(profile) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_power_update(result, cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn set_charge_threshold(
        &mut self,
        threshold: rmac_power::ChargeThreshold,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let is_current = self.power.battery.as_ref().is_some_and(|battery| {
            battery.charge_threshold == threshold && battery.charge_threshold.can_change()
        });
        if self.power_loading || self.power_busy || !is_current || threshold.enabled == enabled {
            return;
        }
        self.power_generation = self.power_generation.wrapping_add(1);
        self.power_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let update = cx
                .background_executor()
                .spawn(async move { apply_charge_threshold(&threshold, enabled) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if let Some(snapshot) = update.recovery {
                    this.power = snapshot;
                }
                this.finish_power_update(update.result, cx);
                cx.notify();
            });
        })
        .detach();
    }
}
