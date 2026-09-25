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

    /// Keyboard's Key repeat rate slider (SET-101): index 0 = Slow ... last
    /// = Fast, `KEYBOARD_RATES`' own order. `index` positions the thumb;
    /// [`crate::input::keyboard_rate_slider_index`] finds the nearest one
    /// for the system's real rate.
    pub(in crate::controller) fn keyboard_repeat_rate_slider(
        cx: &mut Context<Self>,
        index: f32,
    ) -> Entity<SliderState> {
        let max = (KEYBOARD_RATES.len() - 1) as f32;
        let slider = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(max)
                .step(1.0)
                .default_value(index)
        });
        cx.subscribe(&slider, move |this, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(value) = event {
                let index = value.start().round().clamp(0.0, max) as usize;
                if let Some((_, change)) = KEYBOARD_RATES.get(index) {
                    let change = *change;
                    this.apply_input_change(change, cx);
                }
            }
        })
        .detach();
        slider
    }

    /// Keyboard's Delay until repeat slider (SET-101): index 0 = Long ...
    /// last = Short, the Mac's own reversed order over `KEYBOARD_DELAYS`.
    pub(in crate::controller) fn keyboard_repeat_delay_slider(
        cx: &mut Context<Self>,
        index: f32,
    ) -> Entity<SliderState> {
        let last = KEYBOARD_DELAYS.len() - 1;
        let max = last as f32;
        let slider = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(max)
                .step(1.0)
                .default_value(index)
        });
        cx.subscribe(&slider, move |this, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(value) = event {
                let index = value.start().round().clamp(0.0, max) as usize;
                if let Some((_, change)) = KEYBOARD_DELAYS.get(last - index) {
                    let change = *change;
                    this.apply_input_change(change, cx);
                }
            }
        })
        .detach();
        slider
    }
}
