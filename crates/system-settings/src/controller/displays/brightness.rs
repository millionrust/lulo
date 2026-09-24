//! The built-in display's brightness slider: a debounced write through
//! logind `SetBrightness` (`rmac-osd`), matching the audio volume sliders'
//! generation-guarded scheduling so a fast drag does not queue a write per
//! frame.

use super::*;

impl Settings {
    pub(in crate::controller) fn schedule_brightness(
        &mut self,
        value: f32,
        cx: &mut Context<Self>,
    ) {
        if self.brightness.is_none() {
            return;
        }
        self.brightness_generation = self.brightness_generation.wrapping_add(1);
        let generation = self.brightness_generation;
        let percentage = value.round().clamp(0.0, 100.0) as u8;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            cx.background_executor()
                .timer(Duration::from_millis(80))
                .await;
            let still_current = this
                .update(cx, |this: &mut Settings, _| {
                    this.brightness_generation == generation
                })
                .unwrap_or(false);
            if !still_current {
                return;
            }
            let result = cx
                .background_executor()
                .spawn(async move { rmac_osd::set_brightness(percentage) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if this.brightness_generation != generation {
                    return;
                }
                match result {
                    Ok(applied) => {
                        this.brightness = Some(applied);
                        this.brightness_error = None;
                    }
                    Err(error) => {
                        this.brightness_error =
                            Some(format!("Could not change brightness: {error}").into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
