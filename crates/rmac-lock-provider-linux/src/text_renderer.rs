//! Bounded Unicode rasterization for PAM prompt presentation.

use std::collections::VecDeque;
use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};

use chrono::{DateTime, Local};
use cosmic_text::{
    Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Weight, Wrap,
};

use crate::paint::{TextRaster, TextRasterError};
use crate::prompt_label::{AccountLabel, PromptKey, PromptLabel, PromptText};
use crate::surface::BufferLayout;

const MAX_CACHED_RASTERS: usize = 8;

pub(crate) struct LockTextRenderer {
    font_system: FontSystem,
    swash_cache: SwashCache,
    account: Option<AccountLabel>,
    prompt: Option<PromptLabel>,
    clock: ClockLabels,
    rasters: VecDeque<(TextRole, RasterKey, TextRaster)>,
}

impl Default for LockTextRenderer {
    fn default() -> Self {
        let clock = ClockLabels::now();
        Self {
            // Font discovery happens while the Wayland connection is being
            // prepared, before the compositor receives a lock request.
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
            account: None,
            prompt: None,
            clock,
            rasters: VecDeque::new(),
        }
    }
}

impl LockTextRenderer {
    pub(crate) fn set_account(&mut self, account: AccountLabel) {
        self.account = Some(account);
        self.rasters
            .retain(|(role, _, _)| *role != TextRole::Account);
    }

    /// Refresh minute-granularity date and time labels. The Wayland pump calls
    /// this before rendering pending frames so the lock screen stays current
    /// without a second timer thread.
    pub(crate) fn refresh_clock(&mut self) -> bool {
        let clock = ClockLabels::now();
        if self.clock.minute == clock.minute {
            return false;
        }
        self.clock = clock;
        self.rasters
            .retain(|(role, _, _)| !matches!(role, TextRole::Clock | TextRole::Date));
        true
    }

    pub(crate) fn update(
        &mut self,
        prompt: Option<PromptText<'_>>,
        authentication_failed: bool,
        authenticating: bool,
    ) -> bool {
        let desired_key = prompt
            .map(PromptText::key)
            .or_else(|| authentication_failed.then_some(PromptKey::AuthenticationFailure))
            .or_else(|| authenticating.then_some(PromptKey::Authenticating));
        if self.prompt.as_ref().map(PromptLabel::key) == desired_key {
            return false;
        }
        self.prompt = prompt
            .map(PromptLabel::from_prompt)
            .or_else(|| authentication_failed.then(PromptLabel::authentication_failure))
            .or_else(|| authenticating.then(PromptLabel::authenticating));
        self.swash_cache = SwashCache::new();
        self.rasters
            .retain(|(role, _, _)| *role != TextRole::Prompt);
        true
    }

    pub(crate) fn rasters(&mut self, layout: BufferLayout) -> Result<LockTextRasters, Error> {
        Ok(LockTextRasters {
            clock: self.label_raster(layout, TextRole::Clock)?,
            date: self.label_raster(layout, TextRole::Date)?,
            account: self.account_raster(layout)?,
            prompt: self.prompt_raster(layout)?,
        })
    }

    fn label_raster(
        &mut self,
        layout: BufferLayout,
        role: TextRole,
    ) -> Result<Option<TextRaster>, Error> {
        let text = match role {
            TextRole::Clock => self.clock.time.as_str(),
            TextRole::Date => self.clock.date.as_str(),
            _ => return Err(Error::InvalidRaster),
        };
        let key = RasterKey::new(layout);
        if let Some((_, _, raster)) = self
            .rasters
            .iter()
            .find(|(candidate_role, candidate, _)| *candidate_role == role && *candidate == key)
        {
            return Ok(Some(raster.clone()));
        }
        let result = catch_unwind(AssertUnwindSafe(|| {
            rasterize(
                &mut self.font_system,
                &mut self.swash_cache,
                layout,
                text,
                role,
            )
        }))
        .map_err(|_| Error::Panicked)??;
        self.cache(role, key, result.clone());
        Ok(Some(result))
    }

    fn account_raster(&mut self, layout: BufferLayout) -> Result<Option<TextRaster>, Error> {
        let Some(label) = self.account.as_ref() else {
            return Ok(None);
        };
        let key = RasterKey::new(layout);
        if let Some((_, _, raster)) = self
            .rasters
            .iter()
            .find(|(role, candidate, _)| *role == TextRole::Account && *candidate == key)
        {
            return Ok(Some(raster.clone()));
        }

        let result = {
            let font_system = &mut self.font_system;
            let swash_cache = &mut self.swash_cache;
            label.expose(|text| {
                catch_unwind(AssertUnwindSafe(|| {
                    rasterize(font_system, swash_cache, layout, text, TextRole::Account)
                }))
                .map_err(|_| Error::Panicked)?
            })
        }?;
        self.cache(TextRole::Account, key, result.clone());
        Ok(Some(result))
    }

    fn prompt_raster(&mut self, layout: BufferLayout) -> Result<Option<TextRaster>, Error> {
        let Some(label) = self.prompt.as_ref() else {
            return Ok(None);
        };
        let key = RasterKey::new(layout);
        if let Some((_, _, raster)) = self
            .rasters
            .iter()
            .find(|(role, candidate, _)| *role == TextRole::Prompt && *candidate == key)
        {
            return Ok(Some(raster.clone()));
        }

        let result = {
            let font_system = &mut self.font_system;
            let swash_cache = &mut self.swash_cache;
            label.expose(|text| {
                catch_unwind(AssertUnwindSafe(|| {
                    rasterize(font_system, swash_cache, layout, text, TextRole::Prompt)
                }))
                .map_err(|_| Error::Panicked)?
            })
        }?;
        self.cache(TextRole::Prompt, key, result.clone());
        Ok(Some(result))
    }

    fn cache(&mut self, role: TextRole, key: RasterKey, raster: TextRaster) {
        if self.rasters.len() >= MAX_CACHED_RASTERS {
            self.rasters.pop_front();
        }
        self.rasters.push_back((role, key, raster));
    }
}

pub(crate) struct LockTextRasters {
    clock: Option<TextRaster>,
    date: Option<TextRaster>,
    account: Option<TextRaster>,
    prompt: Option<TextRaster>,
}

impl LockTextRasters {
    pub(crate) fn clock(&self) -> Option<&TextRaster> {
        self.clock.as_ref()
    }

    pub(crate) fn date(&self) -> Option<&TextRaster> {
        self.date.as_ref()
    }

    pub(crate) fn account(&self) -> Option<&TextRaster> {
        self.account.as_ref()
    }

    pub(crate) fn prompt(&self) -> Option<&TextRaster> {
        self.prompt.as_ref()
    }
}

impl fmt::Debug for LockTextRasters {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LockTextRasters(<redacted>)")
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TextRole {
    Clock,
    Date,
    Account,
    Prompt,
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
    role: TextRole,
) -> Result<TextRaster, Error> {
    let scale = layout.scale();
    let maximum_width = match role {
        TextRole::Clock => 720,
        TextRole::Date | TextRole::Account | TextRole::Prompt => 560,
    };
    let width = layout
        .width()
        .saturating_sub(48_u32.saturating_mul(scale))
        .max(1)
        .min(maximum_width * scale);
    let logical_height = match role {
        TextRole::Clock => 124,
        TextRole::Date => 42,
        TextRole::Account => 40,
        TextRole::Prompt => 56,
    };
    let height = layout.height().max(1).min(logical_height * scale);
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
    let (font_size, line_height) = match role {
        TextRole::Clock => (96.0, 112.0),
        TextRole::Date => (24.0, 34.0),
        TextRole::Account => (21.0, 29.0),
        TextRole::Prompt => (16.0, 24.0),
    };
    let metrics = Metrics::new(font_size * scale, line_height * scale);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(font_system, Some(width as f32), Some(height as f32));
    buffer.set_wrap(font_system, Wrap::WordOrGlyph);
    let attrs = Attrs::new()
        .family(Family::Name("Inter"))
        .weight(if role == TextRole::Clock {
            Weight::LIGHT
        } else {
            Weight::NORMAL
        });
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
    let origin_y = match role {
        TextRole::Clock => {
            let center = u64::from(layout.height()) * 18 / 100;
            center.saturating_sub(u64::from(height) / 2) as i64
        }
        TextRole::Date => {
            let center = u64::from(layout.height()) * 10 / 100;
            center.saturating_sub(u64::from(height) / 2) as i64
        }
        TextRole::Account => {
            let center = u64::from(layout.height()) * 76 / 100;
            center.saturating_sub(u64::from(height) / 2) as i64
        }
        TextRole::Prompt => {
            let panel_center = u64::from(layout.height()) * 84 / 100;
            panel_center.saturating_add(u64::from(26 * layout.scale())) as i64
        }
    };
    TextRaster::new(origin_x, origin_y, width, height, alpha).map_err(Error::Raster)
}

struct ClockLabels {
    minute: i64,
    time: String,
    date: String,
}

impl ClockLabels {
    fn now() -> Self {
        Self::from_datetime(Local::now())
    }

    fn from_datetime(now: DateTime<Local>) -> Self {
        Self {
            minute: now.timestamp().div_euclid(60),
            time: now.format("%-I:%M").to_string(),
            date: now.format("%A, %-d %B").to_string(),
        }
    }
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
            renderer.set_account(AccountLabel::new("jacob").unwrap());
            assert!(renderer.update(
                Some(PromptText::new(prompt.id(), prompt.kind(), text)),
                false,
                false,
            ));
            let first = renderer.rasters(layout()).unwrap();
            let second = renderer.rasters(layout()).unwrap();
            assert_eq!(first.account(), second.account());
            assert_eq!(first.prompt(), second.prompt());
            assert_ne!(first.account(), first.prompt());
            let panel_center = i64::from(layout().height()) * 84 / 100;
            assert!(first.clock().is_some());
            assert!(first.date().is_some());
            assert!(first.account().unwrap().origin_y() > 0);
            assert!(first.account().unwrap().bottom() < panel_center);
            assert!(first.prompt().unwrap().origin_y() > panel_center);
            assert_eq!(renderer.rasters.len(), 4);
            assert!(renderer.update(None, true, false));
            let failure = renderer.rasters(layout()).unwrap();
            assert_eq!(first.account(), failure.account());
            assert_ne!(first.prompt(), failure.prompt());
            assert_eq!(renderer.rasters.len(), 4);
            assert!(renderer.update(None, false, true));
            let authenticating = renderer.rasters(layout()).unwrap();
            assert_eq!(first.account(), authenticating.account());
            assert_ne!(failure.prompt(), authenticating.prompt());
            assert_eq!(renderer.rasters.len(), 4);
            assert_eq!(format!("{renderer:?}"), "LockTextRenderer(<redacted>)");
            assert_eq!(format!("{first:?}"), "LockTextRasters(<redacted>)");
            assert_eq!(
                format!("{:?}", first.account().unwrap()),
                "TextRaster(<redacted>)"
            );
        });
    }
}
