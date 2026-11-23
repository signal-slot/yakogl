// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The record screen: the last lines of the console drawn over the picture
//! when a run ends, the last `yk-record:` line at the bottom. A board whose
//! console nobody reads (the NanoPC-T6 on its capture unit,
//! driver/docs/nanopc-t6-display.md) says its result on the screen this way.
//!
//! The board's printk hook hands every character to [`Ring::put`]; at the
//! run's end the board draws the ring with [`Ring::draw`] into the buffer the
//! display scans. The C side reaches both through `ffi::board`
//! (`yd_record_screen_put`, `yd_record_screen_draw`); the shim is
//! driver/zephyr/lib/board/common/record_screen.c.

/// Lines the ring keeps: the line being written and the `LINES - 1` before it.
pub const LINES: usize = 48;
/// Characters kept of a line; the rest of a longer line is dropped.
pub const COLS: usize = 158;

/// A glyph's cell in pixels: one byte per row, top row first, bit 7 the
/// leftmost pixel (driver/zephyr/lib/board/common/gc_font8x16.h).
pub const FONT_W: usize = 8;
pub const FONT_H: usize = 16;
/// The font's first character and its count: printable ASCII.
pub const FONT_FIRST: u8 = 0x20;
pub const FONT_GLYPHS: usize = 95;
pub type Font = [[u8; FONT_H]; FONT_GLYPHS];

/// The text's and the background's XRGB8888 pixel.
pub const FG: u32 = 0x00ff_ffff;
pub const BG: u32 = 0x0020_2020;

/// How a record line starts (common/record.c prints it).
const RECORD: &[u8] = b"yk-record: ";
/// The bottom line of a run that printed no record.
const NO_RECORD: &[u8] = b"yk-record: none";

/// The last console lines: the C side's `struct yd_record_screen`. All zero
/// is the empty ring.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ring {
    text: [[u8; COLS]; LINES],
    /// The characters in use of each line of `text`.
    len: [u8; LINES],
    /// The last complete line that started with `yk-record: `.
    record: [u8; COLS],
    record_len: u8,
    /// The line of `text` being written.
    next: u8,
    /// Complete lines kept before `next`: at most `LINES - 1`.
    count: u8,
    pad: u8,
}

// a line's length and a line's number are a u8
const _: () = assert!(COLS <= u8::MAX as usize);
const _: () = assert!(LINES <= u8::MAX as usize);
// the layout include/yakogl_drivers.h pins
const _: () = assert!(LINES == 48 && COLS == 158);
const _: () = assert!(core::mem::offset_of!(Ring, len) == 7584);
const _: () = assert!(core::mem::offset_of!(Ring, record) == 7632);
const _: () = assert!(core::mem::offset_of!(Ring, record_len) == 7790);

impl Default for Ring {
    fn default() -> Self {
        Self::new()
    }
}

impl Ring {
    pub const fn new() -> Self {
        Self { text: [[0; COLS]; LINES], len: [0; LINES], record: [0; COLS], record_len: 0, next: 0, count: 0, pad: 0 }
    }

    /// One console character. `\n` ends the line, `\r` is dropped, a line's
    /// characters after the first [`COLS`] are dropped.
    pub fn put(&mut self, c: u8) {
        let n = self.next as usize % LINES;
        let len = (self.len[n] as usize).min(COLS);
        match c {
            b'\r' => {}
            b'\n' => {
                if self.text[n][..len].starts_with(RECORD) {
                    self.record = self.text[n];
                    self.record_len = len as u8;
                }
                let next = (n + 1) % LINES;
                self.next = next as u8;
                self.len[next] = 0;
                if (self.count as usize) < LINES - 1 {
                    self.count += 1;
                }
            }
            _ if len < COLS => {
                self.text[n][len] = c;
                self.len[n] = len as u8 + 1;
            }
            _ => {}
        }
    }

    /// The line `back` lines before the one being written (0: that one).
    fn line(&self, back: usize) -> &[u8] {
        let i = (self.next as usize % LINES + LINES - back % LINES) % LINES;
        &self.text[i][..(self.len[i] as usize).min(COLS)]
    }

    /// Draw the record screen over the `width` x `height` picture in `px`
    /// (rows `stride` pixels apart, top row first): every pixel [`BG`], then
    /// in [`FG`] `head` on the first text row, under it the newest console
    /// lines that fit (a line longer than the screen is wide continues on
    /// the next row; the line being written counts when it has text), and
    /// on the last rows the last record line, or `yk-record: none`.
    /// Returns the console lines drawn. 0 and nothing written when the
    /// picture holds no three text rows of one character or `px` is too
    /// short for it.
    pub fn draw(&self, px: &mut [u32], stride: usize, width: usize, height: usize, font: &Font, head: &[u8]) -> usize {
        let (cols, rows) = (width / FONT_W, height / FONT_H);
        if cols == 0 || rows < 3 || stride < width {
            return 0;
        }
        match stride.checked_mul(height - 1).and_then(|n| n.checked_add(width)) {
            Some(n) if n <= px.len() => {}
            _ => return 0,
        }
        for y in 0..height {
            px[y * stride..y * stride + width].fill(BG);
        }
        let rows_of = |l: &[u8]| l.len().div_ceil(cols).max(1);
        let record = match self.record_len as usize {
            0 => NO_RECORD,
            n => &self.record[..n.min(COLS)],
        };
        let record_rows = rows_of(record).min(rows - 2);
        let budget = rows - 1 - record_rows;
        let first = usize::from(self.line(0).is_empty());
        let (mut lines, mut used) = (0, 0);
        for back in first..=(self.count as usize).min(LINES - 1) {
            let r = rows_of(self.line(back));
            if used + r > budget {
                break;
            }
            used += r;
            lines += 1;
        }
        let mut text = |row: usize, max_rows: usize, l: &[u8]| {
            for (i, &c) in l.iter().take(max_rows * cols).enumerate() {
                let c = if (FONT_FIRST..FONT_FIRST + FONT_GLYPHS as u8).contains(&c) { c } else { b'?' };
                let at = (row + i / cols) * FONT_H * stride + i % cols * FONT_W;
                for (gy, bits) in font[(c - FONT_FIRST) as usize].iter().enumerate() {
                    for gx in (0..FONT_W).filter(|gx| bits & (0x80 >> gx) != 0) {
                        px[at + gy * stride + gx] = FG;
                    }
                }
            }
        };
        text(0, 1, head);
        let mut row = 1;
        for back in (first..first + lines).rev() {
            let l = self.line(back);
            text(row, rows_of(l), l);
            row += rows_of(l);
        }
        text(rows - record_rows, record_rows, record);
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A font whose glyph of character c has c in its first row and nothing
    /// under it: the first pixel row of a cell reads back as the character.
    fn font() -> Font {
        let mut f = [[0; FONT_H]; FONT_GLYPHS];
        for (i, g) in f.iter_mut().enumerate() {
            g[0] = FONT_FIRST + i as u8;
        }
        f
    }

    fn ring(text: &str) -> Ring {
        let mut r = Ring::new();
        text.bytes().for_each(|c| r.put(c));
        r
    }

    const PAD: u32 = 0xdead_beef;

    /// The text rows of the screen `r` draws, read back from the pixels;
    /// every pixel is FG or BG, the ones between the rows' ends stay PAD.
    fn screen(r: &Ring, cols: usize, rows: usize, head: &str) -> (usize, Vec<String>) {
        // a width and a height that are no whole cells, a stride wider than the width
        let (width, height, stride) = (cols * FONT_W + 3, rows * FONT_H + 5, cols * FONT_W + 7);
        let mut px = vec![PAD; stride * (height - 1) + width];
        let lines = r.draw(&mut px, stride, width, height, &font(), head.as_bytes());
        let mut out = Vec::new();
        for y in 0..height {
            for x in 0..stride.min(px.len() - y * stride) {
                let p = px[y * stride + x];
                assert!(if x < width { p == FG || p == BG } else { p == PAD }, "pixel {x},{y} is {p:#x}");
                // a glyph of this font has pixels in its first row only
                assert!(p != FG || (y % FONT_H == 0 && y / FONT_H < rows && x / FONT_W < cols), "FG at {x},{y}");
            }
        }
        for row in 0..rows {
            let mut s = String::new();
            for col in 0..cols {
                let at = row * FONT_H * stride + col * FONT_W;
                let c = (0..FONT_W).fold(0u8, |c, gx| c << 1 | u8::from(px[at + gx] == FG));
                s.push(if c == 0 { ' ' } else { c as char });
            }
            out.push(s.trim_end().to_string());
        }
        (lines, out)
    }

    #[test]
    fn the_newest_lines_under_the_head_and_the_record_at_the_bottom() {
        let r = ring("one\r\ntwo\nyk-record: 01 02\nthree\n\nfive\n");
        let (lines, s) = screen(&r, 20, 9, "head");
        assert_eq!(lines, 6);
        assert_eq!(s, ["head", "one", "two", "yk-record: 01 02", "three", "", "five", "", "yk-record: 01 02"]);
        // fewer rows: the oldest lines go first; the record stays
        let (lines, s) = screen(&r, 20, 5, "head");
        assert_eq!(lines, 3);
        assert_eq!(s, ["head", "three", "", "five", "yk-record: 01 02"]);
        let (lines, s) = screen(&r, 20, 3, "a head longer than the screen");
        assert_eq!(lines, 1);
        assert_eq!(s, ["a head longer than t", "five", "yk-record: 01 02"]);
    }

    #[test]
    fn a_long_line_continues_on_the_next_row_and_is_shown_whole_or_not_at_all() {
        let r = ring("short\n0123456789abcdefghij\nlast\n");
        // the record line takes two rows of 8 characters
        let (lines, s) = screen(&r, 8, 8, "h");
        assert_eq!(lines, 3);
        assert_eq!(s, ["h", "short", "01234567", "89abcdef", "ghij", "last", "yk-recor", "d: none"]);
        // the long line's three rows do not fit over "last": the rows stay empty
        let (lines, s) = screen(&r, 8, 6, "h");
        assert_eq!(lines, 1);
        assert_eq!(s, ["h", "last", "", "", "yk-recor", "d: none"]);
    }

    #[test]
    fn the_line_being_written_is_shown_and_other_bytes_are_question_marks() {
        let r = ring("done\nno new\tline \x7f");
        let (lines, s) = screen(&r, 20, 4, "h");
        assert_eq!(lines, 2);
        assert_eq!(s, ["h", "done", "no new?line ?", "yk-record: none"]);
    }

    #[test]
    fn the_ring_keeps_the_last_lines_and_the_last_record() {
        let mut text = String::from("yk-record: early\n");
        for i in 0..100 {
            text += &format!("line {i}\n");
        }
        let r = ring(&text);
        let (lines, s) = screen(&r, 20, LINES + 10, "h");
        assert_eq!(lines, LINES - 1);
        assert_eq!(s[1], format!("line {}", 100 - (LINES - 1)));
        assert_eq!(s[LINES - 1], "line 99");
        assert_eq!(s[LINES + 9], "yk-record: early");
        // a later record replaces it; one that is not at the line's start does not
        let r = ring(&(text + "yk-record: late\nsee yk-record: no\n"));
        assert_eq!(screen(&r, 20, 4, "h").1[3], "yk-record: late");
    }

    #[test]
    fn a_line_keeps_its_first_cols_characters() {
        let long: String = (0..COLS + 20).map(|i| (b'a' + (i % 26) as u8) as char).collect();
        let r = ring(&format!("{long}\nnext\n"));
        let (lines, s) = screen(&r, COLS + 20, 4, "h");
        assert_eq!(lines, 2);
        assert_eq!(s[1], long[..COLS]);
        assert_eq!(s[2], "next");
    }

    #[test]
    fn a_picture_too_small_or_a_short_buffer_is_left_alone() {
        let r = ring("line\n");
        let f = font();
        for (stride, width, height, len) in [
            (64, 64, 2 * FONT_H + 15, 64 * 48),    // two text rows
            (64, FONT_W - 1, 3 * FONT_H, 64 * 48), // no column
            (63, 64, 3 * FONT_H, 64 * 48),         // rows closer than the width
            (64, 64, 3 * FONT_H, 64 * 47 + 63),    // one pixel short
            (usize::MAX / 2, 64, 3 * FONT_H, 64),  // the size overflows
        ] {
            let mut px = vec![PAD; len];
            assert_eq!(r.draw(&mut px, stride, width, height, &f, b"h"), 0);
            assert!(px.iter().all(|&p| p == PAD), "{stride} {width} {height} {len}");
        }
        // the smallest one that is drawn
        let mut px = vec![PAD; 64 * 47 + 64];
        assert_eq!(r.draw(&mut px, 64, 64, 3 * FONT_H, &f, b"h"), 1);
    }
}
