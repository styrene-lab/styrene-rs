//! Styrene splash screen
#![allow(dead_code)]
// — CRT noise convergence on the ⬡ mesh sigil.
//!
//! Each character unlocks frame by frame (center-out weighted).
//! Before unlock: CRT noise glyph. After unlock: final character.

use std::time::Duration;
use ratatui::prelude::*;

use super::theme::Theme;

pub const FRAME_INTERVAL_MS: u64 = 45;
pub const TOTAL_FRAMES: u32 = 38;
pub const HOLD_FRAMES: u32 = 8;

const NOISE_CHARS: &[char] = &[
    '▓', '▒', '░', '█', '▄', '▀', '▌', '▐', '▊', '▋', '▍', '▎', '▏', '◆', '■',
    '┼', '╬', '╪', '╫', '┤', '├', '┬', '┴', '╱', '╲', '│', '─', '⬡', '◇',
];

struct SimpleRng { s: u32 }
impl SimpleRng {
    fn new(seed: u32) -> Self { Self { s: seed } }
    fn next(&mut self) -> f64 {
        self.s = self.s.wrapping_mul(1664525).wrapping_add(1013904223) & 0x7fffffff;
        self.s as f64 / 0x7fffffff as f64
    }
    fn choice_char(&mut self, chars: &[char]) -> char {
        let idx = (self.next() * chars.len() as f64) as usize;
        chars[idx.min(chars.len() - 1)]
    }
}

// Styrene ASCII logo — mesh hexagon sigil + wordmark
const SIGIL: &[&str] = &[
    r"      ___   ___      ",
    r"     /   \ /   \     ",
    r"    | ⬡   X   ⬡ |    ",
    r"     \___/ \___/     ",
    r"    /   \ /   \      ",
    r"   | ⬡   X   ⬡ |     ",
    r"    \___/ \___/      ",
    r"       S T Y R E N E ",
    r"  mesh communications",
    r"                     ",
];

const MARK: &[&str] = &[
    " ███████╗████████╗██╗   ██╗██████╗ ███████╗███╗   ██╗███████╗",
    " ██╔════╝╚══██╔══╝╚██╗ ██╔╝██╔══██╗██╔════╝████╗  ██║██╔════╝",
    " ███████╗   ██║    ╚████╔╝ ██████╔╝█████╗  ██╔██╗ ██║█████╗  ",
    " ╚════██║   ██║     ╚██╔╝  ██╔══██╗██╔══╝  ██║╚██╗██║██╔══╝  ",
    " ███████║   ██║      ██║   ██║  ██║███████╗██║ ╚████║███████╗ ",
    " ╚══════╝   ╚═╝      ╚═╝   ╚═╝  ╚═╝╚══════╝╚═╝  ╚═══╝╚══════╝",
];

pub struct SplashScreen {
    pub frame: u32,
    pub hold_count: u32,
    art: Vec<Vec<char>>,
    unlock_frames: Vec<Vec<u32>>,
    art_width: u16,
    art_height: u16,
    term_width: u16,
    term_height: u16,
}

impl SplashScreen {
    pub fn new(term_width: u16, term_height: u16) -> Option<Self> {
        if term_width < 40 || term_height < 12 { return None; }

        // Build combined art: sigil + spacer + mark
        let mut art_lines: Vec<&str> = SIGIL.to_vec();
        art_lines.push("");
        art_lines.extend_from_slice(MARK);

        let art_width = art_lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
        let art_height = art_lines.len();

        let art: Vec<Vec<char>> = art_lines.iter()
            .map(|l| {
                let mut chars: Vec<char> = l.chars().collect();
                chars.resize(art_width, ' ');
                chars
            })
            .collect();

        // Compute center
        let cy = art_height as f64 / 2.0;
        let cx = art_width as f64 / 2.0;
        let max_dist = (cx * cx + cy * cy).sqrt().max(1.0);

        // Assign unlock frames — center-biased
        let mut rng = SimpleRng::new(42);
        let unlock_frames: Vec<Vec<u32>> = (0..art_height)
            .map(|y| {
                (0..art_width).map(|x| {
                    let dy = y as f64 - cy;
                    let dx = x as f64 - cx;
                    let dist = (dx * dx + dy * dy).sqrt();
                    let norm = dist / max_dist;
                    // Center unlocks first, edges unlock last
                    let jitter = rng.next() * 6.0;
                    ((norm * (TOTAL_FRAMES as f64 - 8.0)) + jitter) as u32
                }).collect()
            })
            .collect();

        Some(Self {
            frame: 0,
            hold_count: 0,
            art,
            unlock_frames,
            art_width: art_width as u16,
            art_height: art_height as u16,
            term_width,
            term_height,
        })
    }

    pub fn frame_interval() -> Duration {
        Duration::from_millis(FRAME_INTERVAL_MS)
    }

    pub fn tick(&mut self) {
        self.frame += 1;
        if self.frame >= TOTAL_FRAMES {
            self.hold_count += 1;
        }
    }

    pub fn force_done(&mut self) {
        self.frame = TOTAL_FRAMES + HOLD_FRAMES + 1;
        self.hold_count = HOLD_FRAMES + 1;
    }

    pub fn ready_to_dismiss(&self) -> bool {
        self.frame >= TOTAL_FRAMES
    }

    pub fn draw(&self, f: &mut Frame, t: &dyn Theme) {
        let area = f.area();
        let offset_x = area.width.saturating_sub(self.art_width) / 2;
        let offset_y = area.height.saturating_sub(self.art_height) / 2;

        // Fill bg
        let bg_block = ratatui::widgets::Block::default()
            .style(Style::default().bg(t.bg()));
        f.render_widget(bg_block, area);

        // Render art character by character
        let mut rng = SimpleRng::new(self.frame.wrapping_mul(7919));
        let buf = f.buffer_mut();

        for (y, row) in self.art.iter().enumerate() {
            for (x, &ch) in row.iter().enumerate() {
                let unlock = self.unlock_frames[y][x];
                let draw_x = offset_x + x as u16;
                let draw_y = offset_y + y as u16;
                if draw_x >= area.right() || draw_y >= area.bottom() { continue; }

                let (symbol, color) = if self.frame >= unlock {
                    // Unlocked — show final character
                    let col = if matches!(ch, '⬡' | '█' | '╗' | '╚' | '╔' | '╝') {
                        t.accent()
                    } else if ch == ' ' {
                        t.bg()
                    } else {
                        t.fg()
                    };
                    (ch.to_string(), col)
                } else {
                    // Locked — show noise glyph
                    let noise = rng.choice_char(NOISE_CHARS);
                    let intensity = self.frame as f64 / unlock as f64;
                    let col = if intensity > 0.7 {
                        t.accent_muted()
                    } else if intensity > 0.4 {
                        t.border()
                    } else {
                        t.dim()
                    };
                    (noise.to_string(), col)
                };

                if let Some(cell) = buf.cell_mut((draw_x, draw_y)) {
                    cell.set_symbol(&symbol);
                    cell.set_fg(color);
                    cell.set_bg(t.bg());
                }
            }
        }

        // Status line below art
        if self.ready_to_dismiss() {
            let hint = " press any key ";
            let hint_x = offset_x + self.art_width.saturating_sub(hint.len() as u16) / 2;
            let hint_y = offset_y + self.art_height + 1;
            if hint_y < area.bottom() {
                for (i, ch) in hint.chars().enumerate() {
                    let cx = hint_x + i as u16;
                    if cx < area.right() {
                        if let Some(cell) = buf.cell_mut((cx, hint_y)) {
                            cell.set_symbol(&ch.to_string());
                            cell.set_fg(t.muted());
                            cell.set_bg(t.bg());
                        }
                    }
                }
            }
        }
    }
}
