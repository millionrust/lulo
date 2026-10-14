//! Battery and power-profile settings authority.

mod render;

use super::*;

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
        self.load_power_extras(cx);
    }

    /// Read the saved Low Power Mode and the screen on/off record off the
    /// UI thread, after each battery update.
    pub(super) fn load_power_extras(&mut self, cx: &mut Context<Self>) {
        let snapshot = self.power.clone();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (mode, events) = cx
                .background_executor()
                .spawn(async move {
                    (
                        rmac_power::low_power::current_mode(&snapshot),
                        rmac_power::screen_usage::events(),
                    )
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if this.low_power_mode != mode || this.screen_events != events {
                    this.low_power_mode = mode;
                    this.screen_events = events;
                    this.notify_if_current_pane(&["Battery"], cx);
                }
            });
        })
        .detach();
    }

    /// Save the Low Power Mode choice and carry it out for the current
    /// power source.
    pub(super) fn set_low_power_mode(
        &mut self,
        mode: rmac_power::low_power::LowPowerMode,
        cx: &mut Context<Self>,
    ) {
        if self.power_loading || self.power_busy || self.low_power_mode == mode {
            return;
        }
        self.low_power_mode = mode;
        self.power_generation = self.power_generation.wrapping_add(1);
        self.power_busy = true;
        cx.notify();
        let snapshot = self.power.clone();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rmac_power::low_power::save_mode(mode).map_err(|error| {
                        rmac_power::Error::from_message("save Low Power Mode", error.to_string())
                    })?;
                    rmac_power::low_power::apply(mode, &snapshot)?;
                    rmac_power::snapshot()
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_power_update(result, cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn set_battery_history_long(&mut self, long: bool, cx: &mut Context<Self>) {
        if self.battery_history_long != long {
            self.battery_history_long = long;
            cx.notify();
        }
    }

    pub(super) fn set_battery_health_sheet(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.battery_health_sheet != open {
            self.battery_health_sheet = open;
            cx.notify();
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
