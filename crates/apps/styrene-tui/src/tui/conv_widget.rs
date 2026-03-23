//! ConversationWidget — segment-based scrollable LXMF message view.
//!
//! StatefulWidget with segment height caching and visible-only rendering.

use ratatui::prelude::*;

use super::segments::Segment;
use super::theme::Theme;

/// Scroll + height cache state.
pub struct ConvState {
    pub scroll_offset: u16,
    pub user_scrolled: bool,
    pub heights: Vec<u16>,
    cached_width: u16,
    cached_count: usize,
}

impl ConvState {
    pub fn new() -> Self {
        Self {
            scroll_offset: 0,
            user_scrolled: false,
            heights: Vec::new(),
            cached_width: 0,
            cached_count: 0,
        }
    }

    pub fn scroll_up(&mut self, amount: u16) {
        self.scroll_offset = self.scroll_offset.saturating_add(amount);
        self.user_scrolled = self.scroll_offset > 0;
    }

    pub fn scroll_down(&mut self, amount: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
        if self.scroll_offset == 0 { self.user_scrolled = false; }
    }

    pub fn auto_scroll_to_bottom(&mut self) {
        if !self.user_scrolled { self.scroll_offset = 0; }
    }

    pub fn force_scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
        self.user_scrolled = false;
    }

    pub fn invalidate(&mut self) { self.cached_count = 0; }

    pub fn ensure_heights(&mut self, segments: &[Segment], width: u16, t: &dyn Theme) {
        if width != self.cached_width {
            self.heights.clear();
            self.cached_width = width;
            self.cached_count = 0;
        }
        if self.cached_count > segments.len() {
            self.heights.truncate(segments.len());
            self.cached_count = segments.len();
        }
        // Recompute last segment (may have updated)
        if !segments.is_empty() && self.cached_count == segments.len() {
            let last = segments.len() - 1;
            self.heights[last] = segments[last].height(width, t);
        }
        while self.cached_count < segments.len() {
            let h = segments[self.cached_count].height(width, t);
            if self.cached_count < self.heights.len() {
                self.heights[self.cached_count] = h;
            } else {
                self.heights.push(h);
            }
            self.cached_count += 1;
        }
    }

    pub fn total_height(&self) -> u16 {
        self.heights.iter().copied().sum()
    }
}

impl Default for ConvState {
    fn default() -> Self { Self::new() }
}

/// The scrollable conversation widget.
pub struct ConversationWidget<'a> {
    segments: &'a [Segment],
    theme: &'a dyn Theme,
}

impl<'a> ConversationWidget<'a> {
    pub fn new(segments: &'a [Segment], theme: &'a dyn Theme) -> Self {
        Self { segments, theme }
    }
}

impl<'a> StatefulWidget for ConversationWidget<'a> {
    type State = ConvState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut ConvState) {
        if area.width == 0 || area.height == 0 || self.segments.is_empty() {
            return;
        }

        state.ensure_heights(self.segments, area.width, self.theme);

        let viewport_height = area.height;
        let total_height = state.total_height();
        let max_scroll = total_height.saturating_sub(viewport_height);
        if state.scroll_offset > max_scroll { state.scroll_offset = max_scroll; }

        let top_offset = if total_height <= viewport_height {
            0
        } else {
            total_height - viewport_height - state.scroll_offset
        };

        let mut y_cursor: u16 = 0;
        for (i, segment) in self.segments.iter().enumerate() {
            let seg_height = state.heights[i];
            let seg_top = y_cursor;
            let seg_bottom = y_cursor + seg_height;
            y_cursor = seg_bottom;

            if seg_bottom <= top_offset { continue; }
            if seg_top >= top_offset + viewport_height { break; }

            if seg_top >= top_offset {
                // Fully visible — render directly
                let render_y = area.y + (seg_top - top_offset);
                let available = area.bottom().saturating_sub(render_y);
                if available == 0 { continue; }
                let seg_area = Rect {
                    x: area.x, y: render_y,
                    width: area.width,
                    height: seg_height.min(available),
                };
                segment.render(seg_area, buf, self.theme);
            } else {
                // Partially clipped at top — render to temp buffer, copy visible portion
                let clip_rows = top_offset - seg_top;
                let visible_rows = seg_height.saturating_sub(clip_rows).min(viewport_height);
                if visible_rows == 0 { continue; }

                let temp_area = Rect::new(0, 0, area.width, seg_height);
                let mut temp_buf = Buffer::empty(temp_area);
                let bg = self.theme.surface_bg();
                let fg = self.theme.fg();
                for y in 0..seg_height {
                    for x in 0..area.width {
                        let cell = &mut temp_buf[(x, y)];
                        cell.set_bg(bg);
                        cell.set_fg(fg);
                    }
                }
                segment.render(temp_area, &mut temp_buf, self.theme);

                for row in 0..visible_rows {
                    let src_y = clip_rows + row;
                    let dst_y = area.y + row;
                    if dst_y >= area.bottom() { break; }
                    for x in 0..area.width {
                        if src_y < seg_height {
                            if let Some(cell) = buf.cell_mut((area.x + x, dst_y)) {
                                *cell = temp_buf[(x, src_y)].clone();
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::segments::{DeliveryStatus};
    use crate::tui::theme::StyreneTheme;

    #[test]
    fn empty_segments_does_not_panic() {
        let segs: Vec<Segment> = vec![];
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let mut state = ConvState::new();
        ConversationWidget::new(&segs, &StyreneTheme).render(area, &mut buf, &mut state);
    }

    #[test]
    fn single_sent_segment_renders() {
        let segs = vec![
            Segment::SentMessage {
                dest_hash: "aabbccdd".into(),
                dest_name: Some("Node A".into()),
                text: "hello".into(),
                delivery_status: DeliveryStatus::Sent,
            },
        ];
        let area = Rect::new(0, 0, 80, 10);
        let mut buf = Buffer::empty(area);
        let mut state = ConvState::new();
        ConversationWidget::new(&segs, &StyreneTheme).render(area, &mut buf, &mut state);
        // Verify something was written
        let any_content = (0..10).any(|y| (0..80).any(|x| buf[(x, y)].symbol() != " "));
        assert!(any_content);
    }

    #[test]
    fn scroll_lifecycle() {
        let mut state = ConvState::new();
        state.scroll_up(5);
        assert_eq!(state.scroll_offset, 5);
        state.scroll_down(5);
        assert_eq!(state.scroll_offset, 0);
        assert!(!state.user_scrolled);
    }

    #[test]
    fn force_scroll_to_bottom_resets() {
        let mut state = ConvState::new();
        state.scroll_up(20);
        state.force_scroll_to_bottom();
        assert_eq!(state.scroll_offset, 0);
        assert!(!state.user_scrolled);
    }
}
