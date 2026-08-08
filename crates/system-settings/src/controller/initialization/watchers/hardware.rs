//! Bluetooth, audio, and power startup watchers.

use super::*;

impl Settings {
    pub(super) fn start_hardware_watchers(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_bluetooth::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_bluetooth_update(result);
                cx.notify();
            });
        })
        .detach();

        let (bluetooth_updates, bluetooth_update_rx) = async_channel::bounded(1);
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_bluetooth::watch(bluetooth_updates).await;
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = bluetooth_update_rx.recv().await {
                match event {
                    rmac_bluetooth::WatchEvent::Changed => {
                        let generation = match this.update(cx, |this: &mut Settings, cx| {
                            this.bluetooth_stream_error = None;
                            cx.notify();
                            (!this.bluetooth_busy && !this.bluetooth_loading)
                                .then_some(this.bluetooth_generation)
                        }) {
                            Ok(generation) => generation,
                            Err(_) => break,
                        };
                        let Some(generation) = generation else {
                            continue;
                        };
                        let result = cx
                            .background_executor()
                            .spawn(async { rmac_bluetooth::snapshot() })
                            .await;
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                if bluetooth_stream_snapshot_is_current(
                                    generation,
                                    this.bluetooth_generation,
                                    this.bluetooth_busy,
                                    this.bluetooth_loading,
                                ) {
                                    this.finish_bluetooth_stream_update(result);
                                    cx.notify();
                                }
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    rmac_bluetooth::WatchEvent::Unavailable => {
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                this.bluetooth_stream_error = Some(
                                    "Live Bluetooth updates are temporarily unavailable while BlueZ reconnects"
                                        .into(),
                                );
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        })
        .detach();

        let (audio_updates, audio_update_rx) = async_channel::bounded(1);
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_audio::watch(audio_updates).await;
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = audio_update_rx.recv().await {
                match event {
                    rmac_audio::WatchEvent::Changed => {
                        let generation = match this.update(cx, |this: &mut Settings, cx| {
                            if this.audio_busy || this.audio_loading {
                                if audio_change_needs_followup(
                                    this.audio_busy,
                                    this.audio_loading,
                                    this.audio_stream_error.is_some(),
                                ) {
                                    this.audio_refresh_pending = true;
                                }
                                None
                            } else {
                                this.audio_stream_error = None;
                                cx.notify();
                                Some(this.audio_generation)
                            }
                        }) {
                            Ok(generation) => generation,
                            Err(_) => break,
                        };
                        let Some(generation) = generation else {
                            continue;
                        };
                        let result = cx
                            .background_executor()
                            .spawn(async { rmac_audio::snapshot() })
                            .await;
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                if audio_stream_snapshot_is_current(
                                    generation,
                                    this.audio_generation,
                                    this.audio_busy,
                                    this.audio_loading,
                                ) {
                                    this.finish_audio_stream_update(result, cx);
                                    cx.notify();
                                }
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    rmac_audio::WatchEvent::Unavailable => {
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                this.audio_stream_error = Some(
                                    "Live audio updates are temporarily unavailable while PipeWire reconnects"
                                        .into(),
                                );
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        })
        .detach();

        let (power_updates, power_update_rx) = async_channel::bounded(1);
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_power::watch(power_updates).await;
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = power_update_rx.recv().await {
                match event {
                    rmac_power::WatchEvent::Changed => {
                        let generation = match this.update(cx, |this: &mut Settings, cx| {
                            if this.power_busy || this.power_loading {
                                if power_change_needs_followup(
                                    this.power_busy,
                                    this.power_loading,
                                    this.power_stream_error.is_some(),
                                ) {
                                    this.power_refresh_pending = true;
                                }
                                None
                            } else {
                                this.power_stream_error = None;
                                cx.notify();
                                Some(this.power_generation)
                            }
                        }) {
                            Ok(generation) => generation,
                            Err(_) => break,
                        };
                        let Some(generation) = generation else {
                            continue;
                        };
                        let result = cx
                            .background_executor()
                            .spawn(async { rmac_power::snapshot() })
                            .await;
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                if power_stream_snapshot_is_current(
                                    generation,
                                    this.power_generation,
                                    this.power_busy,
                                    this.power_loading,
                                ) {
                                    this.finish_power_stream_update(result);
                                    cx.notify();
                                }
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    rmac_power::WatchEvent::Unavailable => {
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                this.power_stream_error = Some(
                                    "Live battery updates are temporarily unavailable while UPower reconnects"
                                        .into(),
                                );
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        })
        .detach();
    }
}
