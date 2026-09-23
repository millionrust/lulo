//! Bounded Unicode rasterization for PAM prompt presentation.

use std::collections::VecDeque;
use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};

use chrono::{DateTime, Local};
use cosmic_text::{
    Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Weight, Wrap,
};
use zeroize::Zeroizing;

use crate::paint::{layout as lock_layout, LockTexts, TextRaster, TextRasterError};
use crate::prompt_label::{AccountLabel, PromptKey, PromptLabel, PromptText};
use crate::surface::BufferLayout;

// Six roles per output layout; twelve entries keep two differently sized
// outputs from evicting each other on every repaint.
const MAX_CACHED_RASTERS: usize = 12;
/// The empty password pill's placeholder on a Mac without Touch ID.
const PLACEHOLDER: &str = "Enter Password";

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
            .retain(|(role, _, _)| !matches!(role, TextRole::Avatar | TextRole::Account));
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
            avatar: self.avatar_raster(layout)?,
            account: self.account_raster(layout)?,
            placeholder: self.label_raster(layout, TextRole::Placeholder)?,
            prompt: self.prompt_raster(layout)?,
        })
    }

    fn avatar_raster(&mut self, layout: BufferLayout) -> Result<Option<TextRaster>, Error> {
        let Some(label) = self.account.as_ref() else {
            return Ok(None);
        };
        let key = RasterKey::new(layout);
        if let Some((_, _, raster)) = self
            .rasters
            .iter()
            .find(|(role, candidate, _)| *role == TextRole::Avatar && *candidate == key)
        {
            return Ok(Some(raster.clone()));
        }

        let result = {
            let font_system = &mut self.font_system;
            let swash_cache = &mut self.swash_cache;
            label.expose(|text| {
                let monogram = Zeroizing::new(
                    text.chars()
                        .next()
                        .into_iter()
                        .flat_map(char::to_uppercase)
                        .take(2)
                        .collect::<String>(),
                );
                catch_unwind(AssertUnwindSafe(|| {
                    rasterize(
                        font_system,
                        swash_cache,
                        layout,
                        monogram.as_str(),
                        TextRole::Avatar,
                    )
                }))
                .map_err(|_| Error::Panicked)?
            })
        }?;
        self.cache(TextRole::Avatar, key, result.clone());
        Ok(Some(result))
    }

    fn label_raster(
        &mut self,
        layout: BufferLayout,
        role: TextRole,
    ) -> Result<Option<TextRaster>, Error> {
        let text = match role {
            TextRole::Clock => self.clock.time.as_str(),
            TextRole::Date => self.clock.date.as_str(),
            TextRole::Placeholder => PLACEHOLDER,
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
        // The pill's "Enter Password" placeholder already asks for the
        // password; repeating PAM's generic "Password:" under it would not
        // happen on a Mac.
        if label.is_generic_password() {
            return Ok(None);
        }
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
    avatar: Option<TextRaster>,
    account: Option<TextRaster>,
    placeholder: Option<TextRaster>,
    prompt: Option<TextRaster>,
}

impl LockTextRasters {
    pub(crate) fn texts(&self) -> LockTexts<'_> {
        LockTexts {
            clock: self.clock.as_ref(),
            date: self.date.as_ref(),
            avatar: self.avatar.as_ref(),
            account: self.account.as_ref(),
            placeholder: self.placeholder.as_ref(),
            prompt: self.prompt.as_ref(),
        }
    }

    #[cfg(test)]
    pub(crate) fn clock(&self) -> Option<&TextRaster> {
        self.clock.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn date(&self) -> Option<&TextRaster> {
        self.date.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn avatar(&self) -> Option<&TextRaster> {
        self.avatar.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn account(&self) -> Option<&TextRaster> {
        self.account.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn prompt(&self) -> Option<&TextRaster> {
        self.prompt.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn placeholder(&self) -> Option<&TextRaster> {
        self.placeholder.as_ref()
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
    Avatar,
    Account,
    Placeholder,
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
        TextRole::Avatar => lock_layout::AVATAR_DIAMETER,
        TextRole::Placeholder => lock_layout::FIELD_WIDTH,
        TextRole::Date | TextRole::Account | TextRole::Prompt => 560,
    };
    let width = layout
        .width()
        .saturating_sub(48_u32.saturating_mul(scale))
        .max(1)
        .min(maximum_width * scale);
    // design-lab/lock.html, all S. Single-line rasters are exactly one line
    // box tall so centring the box centres the text.
    let (font_size, line_height, weight) = match role {
        TextRole::Clock => (112.0, 134, Weight::SEMIBOLD),
        TextRole::Date => (22.0, 28, Weight::SEMIBOLD),
        TextRole::Avatar => (24.0, 30, Weight::MEDIUM),
        TextRole::Account => (15.0, 20, Weight::SEMIBOLD),
        TextRole::Placeholder => (13.0, 18, Weight::NORMAL),
        TextRole::Prompt => (12.0, 16, Weight::NORMAL),
    };
    // PAM guidance may wrap to two lines under the pill.
    let logical_height = if role == TextRole::Prompt {
        2 * line_height
    } else {
        line_height
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
    let metrics = Metrics::new(font_size * scale, line_height as f32 * scale);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(font_system, Some(width as f32), Some(height as f32));
    buffer.set_wrap(font_system, Wrap::WordOrGlyph);
    let attrs = Attrs::new()
        .family(Family::Name(rmac_design::UI_FONT))
        .weight(weight);
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
    let output_height = layout.height();
    let output_scale = layout.scale();
    let center_y = match role {
        TextRole::Date => lock_layout::from_top(output_scale, lock_layout::DATE_CENTER_FROM_TOP),
        TextRole::Clock => lock_layout::from_top(output_scale, lock_layout::CLOCK_CENTER_FROM_TOP),
        TextRole::Avatar => lock_layout::from_bottom(
            output_height,
            output_scale,
            lock_layout::AVATAR_CENTER_FROM_BOTTOM,
        ),
        TextRole::Account => lock_layout::from_bottom(
            output_height,
            output_scale,
            lock_layout::ACCOUNT_CENTER_FROM_BOTTOM,
        ),
        TextRole::Placeholder | TextRole::Prompt => lock_layout::from_bottom(
            output_height,
            output_scale,
            lock_layout::FIELD_CENTER_FROM_BOTTOM,
        ),
    };
    let origin_y = if role == TextRole::Prompt {
        // Guidance hangs a fixed gap below the pill.
        center_y
            + i64::from(
                (lock_layout::FIELD_HEIGHT / 2 + lock_layout::GUIDANCE_GAP)
                    .saturating_mul(output_scale.max(1)),
            )
    } else {
        center_y - i64::from(height / 2)
    }
    .max(0);
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
            // "Tuesday 23 September", the owner's en-AU order.
            date: now.format("%A %-d %B").to_string(),
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
            let panel_center = crate::paint::PromptGeometry::new(
                layout().width(),
                layout().height(),
                layout().scale(),
            )
            .center_y;
            assert!(first.clock().is_some());
            assert!(first.placeholder().is_some());
            assert!(first.date().is_some());
            assert!(first.avatar().is_some());
            assert!(first.account().unwrap().origin_y() > 0);
            assert!(first.account().unwrap().bottom() < panel_center);
            assert!(first.prompt().unwrap().origin_y() > panel_center);
            assert_eq!(renderer.rasters.len(), 6);
            assert!(renderer.update(None, true, false));
            let failure = renderer.rasters(layout()).unwrap();
            assert_eq!(first.account(), failure.account());
            assert_ne!(first.prompt(), failure.prompt());
            assert_eq!(renderer.rasters.len(), 6);
            assert!(renderer.update(None, false, true));
            let authenticating = renderer.rasters(layout()).unwrap();
            assert_eq!(first.account(), authenticating.account());
            assert_ne!(failure.prompt(), authenticating.prompt());
            assert_eq!(renderer.rasters.len(), 6);
            assert_eq!(format!("{renderer:?}"), "LockTextRenderer(<redacted>)");
            assert_eq!(format!("{first:?}"), "LockTextRasters(<redacted>)");
            assert_eq!(
                format!("{:?}", first.account().unwrap()),
                "TextRaster(<redacted>)"
            );
        });
    }
}
