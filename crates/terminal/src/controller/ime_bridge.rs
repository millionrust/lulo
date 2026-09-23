//! GPUI platform text-input bridge for the active terminal session.

use std::ops::Range;
use std::sync::Arc;

use super::{ImeComposition, TerminalView, MAX_IME_TEXT_BYTES, PAD_TOP, PAD_X};
use crate::ime::{
    byte_range_for_utf16, replace_buffer as replace_ime_buffer, utf16_len, ImeEditError,
};
use crate::keyboard::encode_text_input;
use crate::session::SessionWriteError;
use alacritty_terminal::grid::Scroll;
use gpui::{px, Bounds, Context, EntityInputHandler, Pixels, Point, UTF16Selection, Window};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

impl TerminalView {
    fn reject_ime(&mut self, message: &'static str, cx: &mut Context<Self>) {
        self.ime = None;
        self.operation_error = Some(message.into());
        cx.notify();
    }

    fn commit_text_input(&mut self, session_id: u64, text: &str, cx: &mut Context<Self>) {
        if text.is_empty() {
            cx.notify();
            return;
        }
        if text.len() > MAX_IME_TEXT_BYTES {
            self.reject_ime(
                "Text input exceeds Terminal's 16 KiB composition safety limit; nothing was sent.",
                cx,
            );
            return;
        }
        if text.chars().any(char::is_control) {
            self.reject_ime(
                "Terminal refused non-text control data from the input method.",
                cx,
            );
            return;
        }
        if self.modal_open() {
            self.reject_ime(
                "Terminal did not send text while a confirmation was open.",
                cx,
            );
            return;
        }
        if self.tabs[self.active].id != session_id {
            self.reject_ime(
                "Text composition belonged to another terminal tab; nothing was sent.",
                cx,
            );
            return;
        }
        let mode = match self.active_terminal_mode() {
            Ok(mode) => mode,
            Err(error) => {
                self.operation_error = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        let bytes = encode_text_input(text, mode);
        match self.tabs[self.active].write(&bytes) {
            Ok(()) => {
                if let Ok(mut terminal) = self.tabs[self.active].term.lock() {
                    terminal.scroll_display(Scroll::Bottom);
                }
                self.tabs[self.active].ui.selection = None;
            }
            Err(SessionWriteError::State) => {
                self.operation_error = Some(SessionWriteError::State.to_string().into());
            }
            Err(SessionWriteError::Exited | SessionWriteError::Write) => {}
        }
        cx.notify();
    }
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let Some(composition) = self
            .ime
            .as_ref()
            .filter(|composition| composition.session_id == self.tabs[self.active].id)
        else {
            if range_utf16.is_empty() && range_utf16.start == 0 {
                *adjusted_range = Some(0..0);
                return Some(String::new());
            }
            return None;
        };
        let bytes = byte_range_for_utf16(&composition.buffer.text, range_utf16.clone())?;
        *adjusted_range = Some(range_utf16);
        Some(composition.buffer.text[bytes].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        if self.modal_open() || !self.tabs[self.active].accepts_input() {
            return None;
        }
        let range = self
            .ime
            .as_ref()
            .filter(|composition| composition.session_id == self.tabs[self.active].id)
            .map_or(0..0, |composition| {
                composition.buffer.selection_utf16.clone()
            });
        Some(UTF16Selection {
            range,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let composition = self
            .ime
            .as_ref()
            .filter(|composition| composition.session_id == self.tabs[self.active].id)?;
        Some(0..utf16_len(&composition.buffer.text))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(composition) = self.ime.take() else {
            return;
        };
        self.commit_text_input(composition.session_id, &composition.buffer.text, cx);
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let composition = self.ime.take();
        let session_id = composition
            .as_ref()
            .map_or(self.tabs[self.active].id, |composition| {
                composition.session_id
            });
        if session_id != self.tabs[self.active].id {
            self.reject_ime(
                "Text composition belonged to another terminal tab; nothing was sent.",
                cx,
            );
            return;
        }
        if let Some(range) = range_utf16 {
            let valid = if let Some(composition) = composition.as_ref() {
                byte_range_for_utf16(&composition.buffer.text, range).is_some()
            } else {
                range.is_empty() && range.start == 0
            };
            if !valid {
                self.reject_ime(
                    "Terminal refused an invalid text-composition replacement.",
                    cx,
                );
                return;
            }
        }
        self.commit_text_input(session_id, text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.modal_open() {
            self.reject_ime(
                "Terminal did not begin text composition while a confirmation was open.",
                cx,
            );
            return;
        }
        if !self.tabs[self.active].accepts_input() {
            self.ime = None;
            cx.notify();
            return;
        }
        if new_text.chars().any(char::is_control) {
            self.reject_ime(
                "Terminal refused non-text control data from the input method.",
                cx,
            );
            return;
        }

        let active_id = self.tabs[self.active].id;
        if self
            .ime
            .as_ref()
            .is_some_and(|composition| composition.session_id != active_id)
        {
            self.reject_ime(
                "Text composition belonged to another terminal tab; nothing was sent.",
                cx,
            );
            return;
        }
        let current = self.ime.as_ref().map(|composition| &composition.buffer);
        match replace_ime_buffer(current, range_utf16, new_text, new_selected_range_utf16) {
            Ok(buffer) if buffer.text.is_empty() => {
                self.ime = None;
                cx.notify();
            }
            Ok(buffer) => {
                let term = Arc::clone(&self.tabs[self.active].term);
                let Ok(mut term) = term.lock() else {
                    self.reject_ime(
                        "Terminal state is unavailable; composed text was not accepted.",
                        cx,
                    );
                    return;
                };
                term.scroll_display(Scroll::Bottom);
                drop(term);
                self.ime = Some(ImeComposition {
                    session_id: active_id,
                    buffer,
                });
                cx.notify();
            }
            Err(ImeEditError::TooLarge) => self.reject_ime(
                "Text input exceeds Terminal's 16 KiB composition safety limit; nothing was sent.",
                cx,
            ),
            Err(ImeEditError::InvalidRange) => self.reject_ime(
                "Terminal refused an invalid text-composition replacement.",
                cx,
            ),
        }
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let (row, column) = self.active_cursor_viewport_cell()?;
        let (prefix_cells, range_cells) = if let Some(composition) = self
            .ime
            .as_ref()
            .filter(|composition| composition.session_id == self.tabs[self.active].id)
        {
            let prefix_bytes =
                byte_range_for_utf16(&composition.buffer.text, 0..range_utf16.start)?;
            let range_bytes = byte_range_for_utf16(&composition.buffer.text, range_utf16.clone())?;
            (
                UnicodeWidthStr::width(&composition.buffer.text[prefix_bytes]),
                UnicodeWidthStr::width(&composition.buffer.text[range_bytes]).max(1),
            )
        } else if range_utf16.is_empty() && range_utf16.start == 0 {
            (0, 1)
        } else {
            return None;
        };

        let linear_cell = column.saturating_add(prefix_cells);
        let candidate_row = row
            .saturating_add(linear_cell / self.cols.max(1))
            .min(self.rows.saturating_sub(1));
        let candidate_column = linear_cell % self.cols.max(1);
        let available_columns = self.cols.saturating_sub(candidate_column).max(1);
        Some(Bounds::new(
            gpui::point(
                element_bounds.left() + px(PAD_X + candidate_column as f32 * self.cell_w),
                element_bounds.top() + px(PAD_TOP + candidate_row as f32 * self.line_h),
            ),
            gpui::size(
                px(range_cells.min(available_columns) as f32 * self.cell_w),
                px(self.line_h),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let composition = self
            .ime
            .as_ref()
            .filter(|composition| composition.session_id == self.tabs[self.active].id)?;
        let (cursor_row, cursor_column) = self.active_cursor_viewport_cell()?;
        let row = (((f32::from(point.y) - self.terminal_content_top()) / self.line_h).floor()
            as i32)
            .clamp(0, self.rows.saturating_sub(1) as i32) as usize;
        let column = (((f32::from(point.x) - PAD_X) / self.cell_w).floor() as i32)
            .clamp(0, self.cols.saturating_sub(1) as i32) as usize;
        let cursor_linear = cursor_row
            .saturating_mul(self.cols)
            .saturating_add(cursor_column);
        let target_linear = row.saturating_mul(self.cols).saturating_add(column);
        let target_cells = target_linear.saturating_sub(cursor_linear);

        let mut cells = 0;
        let mut utf16_offset = 0;
        for character in composition.buffer.text.chars() {
            if cells >= target_cells {
                break;
            }
            cells += UnicodeWidthChar::width(character).unwrap_or(0);
            utf16_offset += character.len_utf16();
        }
        Some(utf16_offset)
    }
}
