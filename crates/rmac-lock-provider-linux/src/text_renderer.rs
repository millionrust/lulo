//! Bounded Unicode rasterization for PAM prompt presentation.

use std::collections::VecDeque;
use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};

use cosmic_text::{
    Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Weight, Wrap,
};

use crate::paint::{TextRaster, TextRasterError};
use crate::prompt_label::{PromptKey, PromptLabel, PromptText};
use crate::surface::BufferLayout;

const MAX_CACHED_RASTERS: usize = 8;

pub(crate) struct LockTextRenderer {
    font_system: FontSystem,
    swash_cache: SwashCache,
    label: Option<PromptLabel>,
    rasters: VecDeque<(RasterKey, TextRaster)>,
}

impl Default for LockTextRenderer {
    fn default() -> Self {
        Self {
            // Font discovery happens while the Wayland connection is being
            // prepared, before the compositor receives a lock request.
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
            label: None,
            rasters: VecDeque::new(),
        }
    }
}

impl LockTextRenderer {
    pub(crate) fn update(
        &mut self,
        prompt: Option<PromptText<'_>>,
        authentication_failed: bool,
    ) -> bool {
        let desired_key = prompt
            .map(PromptText::key)
            .or_else(|| authentication_failed.then_some(PromptKey::AuthenticationFailure));
        if self.label.as_ref().map(PromptLabel::key) == desired_key {
            return false;
        }
        self.label = prompt
            .map(PromptLabel::from_prompt)
            .or_else(|| authentication_failed.then(PromptLabel::authentication_failure));
        self.swash_cache = SwashCache::new();
        self.rasters.clear();
        true
    }

    pub(crate) fn raster(&mut self, layout: BufferLayout) -> Result<Option<TextRaster>, Error> {
        let Some(label) = self.label.as_ref() else {
            return Ok(None);
        };
        let key = RasterKey::new(layout);
        if let Some((_, raster)) = self.rasters.iter().find(|(candidate, _)| *candidate == key) {
            return Ok(Some(raster.clone()));
        }

        let result = {
            let font_system = &mut self.font_system;
            let swash_cache = &mut self.swash_cache;
            label.expose(|text| {
                catch_unwind(AssertUnwindSafe(|| {
                    rasterize(font_system, swash_cache, layout, text)
                }))
                .map_err(|_| Error::Panicked)?
            })
        }?;
        if self.rasters.len() >= MAX_CACHED_RASTERS {
            self.rasters.pop_front();
        }
        self.rasters.push_back((key, result.clone()));
        Ok(Some(result))
    }
}

impl fmt::Debug for LockTextRenderer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LockTextRenderer(<redacted>)")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RasterKey {
    width: u32,
    height: u32,
    scale: u32,
}

impl RasterKey {
    fn new(layout: BufferLayout) -> Self {
        Self {
            width: layout.width(),
            height: layout.height(),
            scale: layout.scale(),
        }
    }
}

fn rasterize(
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    layout: BufferLayout,
    text: &str,
) -> Result<TextRaster, Error> {
    let scale = layout.scale();
    let width = layout
        .width()
        .saturating_sub(48_u32.saturating_mul(scale))
        .max(1)
        .min(560_u32.saturating_mul(scale));
    let height = layout.height().max(1).min(56_u32.saturating_mul(scale));
    let length = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .ok_or(Error::InvalidRaster)?;
    let mut alpha = vec![0_u8; length];

    let scale = scale as f32;
    let metrics = Metrics::new(18.0 * scale, 26.0 * scale);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(font_system, Some(width as f32), Some(height as f32));
    buffer.set_wrap(font_system, Wrap::WordOrGlyph);
    let attrs = Attrs::new()
        .family(Family::Name("Inter"))
        .weight(Weight::NORMAL);
    buffer.set_rich_text(
        font_system,
        [(text, attrs.clone())],
        &attrs,
        Shaping::Advanced,
        Some(Align::Center),
    );
    buffer.shape_until_scroll(font_system, true);
    buffer.draw(
        font_system,
        swash_cache,
        Color::rgb(255, 255, 255),
        |x, y, pixel_width, pixel_height, color| {
            for row in 0..pixel_height {
                for column in 0..pixel_width {
                    let Some(local_x) = x.checked_add_unsigned(column) else {
                        continue;
                    };
                    let Some(local_y) = y.checked_add_unsigned(row) else {
                        continue;
                    };
                    let (Ok(local_x), Ok(local_y)) =
                        (u32::try_from(local_x), u32::try_from(local_y))
                    else {
                        continue;
                    };
                    if local_x >= width || local_y >= height {
                        continue;
                    }
                    let Some(index) = usize::try_from(local_y)
                        .ok()
                        .and_then(|row| row.checked_mul(width as usize))
                        .and_then(|row| row.checked_add(local_x as usize))
                    else {
                        continue;
                    };
                    if let Some(alpha) = alpha.get_mut(index) {
                        *alpha = (*alpha).max(color.a());
                    }
                }
            }
        },
    );
    if !alpha.iter().any(|alpha| *alpha != 0) {
        return Err(Error::FontUnavailable);
    }

    let origin_x = i64::from(layout.width().saturating_sub(width) / 2);
    let panel_center_y = u64::from(layout.height()) * 58 / 100;
    let label_bottom_gap = u64::from(34_u32.saturating_mul(layout.scale()));
    let origin_y = panel_center_y
        .saturating_sub(label_bottom_gap)
        .saturating_sub(u64::from(height)) as i64;
    TextRaster::new(origin_x, origin_y, width, height, alpha).map_err(Error::Raster)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Error {
    InvalidRaster,
    FontUnavailable,
    Raster(TextRasterError),
    Panicked,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("lock prompt text rendering failed")
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pam_broker::conversation_channel;
    use crate::pam_conversation::{Conversation as _, Request};
    use crate::surface::SurfaceSet;
    use rmac_lock_provider::OutputId;
    use std::thread;
    use std::time::Duration;

    fn layout() -> BufferLayout {
        let output = OutputId::new(1).unwrap();
        let mut surfaces = SurfaceSet::new();
        surfaces.add_output(output).unwrap();
        surfaces.configure(output, 1, 640, 480).unwrap();
        surfaces.begin_render(output).unwrap().layout()
    }

    #[test]
    fn installed_fonts_rasterize_and_cache_a_redacted_unicode_prompt() {
        let (mut conversation, ui) = conversation_channel();
        thread::spawn(move || conversation.respond(Request::EchoOff(c"Пароль: رمز")));
        let pending = ui.prompt_timeout(Duration::from_secs(2)).unwrap().unwrap();
        let prompt = pending.prompt();
        prompt.text(|text| {
            let mut renderer = LockTextRenderer::default();
            assert!(renderer.update(
                Some(PromptText::new(prompt.id(), prompt.kind(), text)),
                false,
            ));
            let first = renderer.raster(layout()).unwrap().unwrap();
            let second = renderer.raster(layout()).unwrap().unwrap();
            assert_eq!(first, second);
            assert_eq!(renderer.rasters.len(), 1);
            assert_eq!(format!("{renderer:?}"), "LockTextRenderer(<redacted>)");
            assert_eq!(format!("{first:?}"), "TextRaster(<redacted>)");
        });
    }
}
