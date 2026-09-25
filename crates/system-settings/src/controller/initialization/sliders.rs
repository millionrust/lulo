//! Audio slider construction and subscription wiring.

use super::*;

impl Settings {
    pub(in crate::controller) fn sound_policy_volume_slider(
        cx: &mut Context<Self>,
        value: f32,
    ) -> Entity<SliderState> {
        let slider = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(100.0)
                .step(1.0)
                .default_value(value)
        });
        cx.subscribe(&slider, move |this, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(value) = event {
                this.schedule_sound_policy_volume(value.start(), cx);
                cx.notify();
            }
        })
        .detach();
        slider
    }

    pub(in crate::controller) fn audio_slider(
        cx: &mut Context<Self>,
        value: f32,
        kind: rmac_audio::DeviceKind,
    ) -> Entity<SliderState> {
        let slider = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(100.0)
                .step(1.0)
                .default_value(value)
        });
        cx.subscribe(&slider, move |this, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(value) = event {
                this.schedule_audio_volume(kind, value.start(), cx);
                cx.notify();
            }
        })
        .detach();
        slider
    }

    pub(in crate::controller) fn brightness_slider(
        cx: &mut Context<Self>,
        value: f32,
    ) -> Entity<SliderState> {
        let slider = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(100.0)
                .step(1.0)
                .default_value(value)
        });
        cx.subscribe(&slider, move |this, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(value) = event {
                this.schedule_brightness(value.start(), cx);
                cx.notify();
            }
        })
        .detach();
        slider
    }

    pub(in crate::controller) fn audio_balance_slider(
        cx: &mut Context<Self>,
        value: f32,
    ) -> Entity<SliderState> {
        let slider = cx.new(|_| {
            SliderState::new()
                .min(-100.0)
                .max(100.0)
                .step(1.0)
                .default_value(value)
        });
        cx.subscribe(&slider, move |this, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(value) = event {
                this.schedule_audio_balance(value.start(), cx);
                cx.notify();
            }
        })
        .detach();
        slider
    }

    /// Desktop & Dock's Size slider (DOCK-01).
    pub(in crate::controller) fn dock_size_slider(
        cx: &mut Context<Self>,
        value: f32,
    ) -> Entity<SliderState> {
        let slider = cx.new(|_| {
            SliderState::new()
                .min(rmac_shell_settings::MIN_DOCK_TILE_SIZE)
                .max(rmac_shell_settings::MAX_DOCK_TILE_SIZE)
                .step(1.0)
                .default_value(value)
        });
        cx.subscribe(&slider, move |this, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(value) = event {
                this.apply_dock_change(DockChange::TileSize(value.start()), cx);
            }
        })
        .detach();
        slider
    }

    /// Desktop & Dock's Magnification slider (DOCK-07): Off at 0.0, then
    /// continuous up to the existing 2.5 validation ceiling.
    pub(in crate::controller) fn dock_magnification_slider(
        cx: &mut Context<Self>,
        value: f32,
    ) -> Entity<SliderState> {
        let slider = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(2.5)
                .step(0.05)
                .default_value(value)
        });
        cx.subscribe(&slider, move |this, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(value) = event {
                this.apply_dock_change(DockChange::MagnificationLevel(value.start()), cx);
            }
        })
        .detach();
        slider
    }
}
