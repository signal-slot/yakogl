// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Writing a GC7000 command stream into a CPU-mapped buffer.
//!
//! The front end fetches 64-bit words, so every command occupies an even
//! number of 32-bit words; a LOAD_STATE with an even payload gets a pad word.
//! A writer that runs out of room sets `overflow` and drops the rest: the
//! caller checks once, at the end of a draw, takes the draw back out of the
//! frame (`backend.rs` `Mark`) and records it again in the next one.

use super::regs::*;

/// The command stream being written (`struct gc_cmd`).
pub struct GcCmd {
    /// CPU mapping of the command buffer.
    pub words: *mut u32,
    /// Words written so far.
    pub used: u32,
    /// Capacity, in words.
    pub cap: u32,
    pub overflow: bool,
}

impl GcCmd {
    pub const ZERO: GcCmd = GcCmd { words: core::ptr::null_mut(), used: 0, cap: 0, overflow: false };

    pub fn word(&mut self, v: u32) {
        if self.used >= self.cap || self.words.is_null() {
            self.overflow = true;
            return;
        }
        // SAFETY: `used` is below `cap`, the length of the mapping `words`
        // describes; the words are written one at a time, aligned.
        unsafe { self.words.add(self.used as usize).write(v) };
        self.used += 1;
    }

    /// Room, in words, for a LOAD_STATE of `count` registers (header +
    /// payload + the pad that keeps the stream 64-bit aligned).
    pub const fn state_words(count: u32) -> u32 {
        1 + count + if count & 1 != 0 { 0 } else { 1 }
    }

    pub fn pad(&mut self) {
        if self.used & 1 != 0 {
            self.word(GC_OP_NOP);
        }
    }

    /// Write `count` consecutive registers starting at `reg`. `fixp` marks
    /// the values as 16.16 fixed point (the viewport and scissor registers).
    /// One room check for the whole command, then plain stores: the per-word
    /// check of [`GcCmd::word`] reloaded `used` after every store.
    pub fn state_multi_fixp(&mut self, reg: u32, values: &[u32], fixp: bool) {
        let count = values.len() as u32;

        if count == 0 {
            return;
        }
        let n = Self::state_words(count);
        if self.words.is_null() || self.used + n > self.cap {
            self.overflow = true;
            return;
        }
        // SAFETY: `used + n` is at most `cap`, the length of the mapping
        // `words` describes (checked just above).
        unsafe {
            let p = self.words.add(self.used as usize);
            p.write(load_state(reg, count, fixp));
            for (k, v) in values.iter().enumerate() {
                p.add(1 + k).write(*v);
            }
            if count & 1 == 0 {
                p.add(1 + count as usize).write(GC_OP_NOP); /* pad to a 64-bit boundary */
            }
        }
        self.used += n;
    }

    /// Append `src`, whole commands another writer made, or nothing and the
    /// overflow when they do not fit.
    pub fn append(&mut self, src: &[u32]) {
        let n = src.len() as u32;
        if n == 0 {
            return;
        }
        if self.words.is_null() || self.used + n > self.cap {
            self.overflow = true;
            return;
        }
        // SAFETY: `used + n` is at most `cap` (checked just above); `src` is
        // another buffer than the mapping.
        unsafe { core::ptr::copy_nonoverlapping(src.as_ptr(), self.words.add(self.used as usize), src.len()) };
        self.used += n;
    }

    pub fn state_multi(&mut self, reg: u32, values: &[u32]) {
        self.state_multi_fixp(reg, values, false);
    }

    pub fn state(&mut self, reg: u32, value: u32) {
        self.state_multi_fixp(reg, &[value], false);
    }

    /// A viewport / scissor register: the value is 16.16 fixed point.
    pub fn state_fixp(&mut self, reg: u32, value: u32) {
        self.state_multi_fixp(reg, &[value], true);
    }

    /// Hold one unit until another has drained. When the front end is the one
    /// being held it takes a STALL command as well as the token.
    pub fn stall(&mut self, from: u32, to: u32) {
        self.state(GC_GL_SEMAPHORE_TOKEN, stall_token(from, to));
        if from == GC_SYNC_FE {
            self.word(GC_OP_STALL);
            self.word(stall_token(from, to));
        } else {
            self.state(GC_GL_STALL_TOKEN, stall_token(from, to));
        }
    }

    pub fn draw_instanced(&mut self, indexed: bool, prim: u32, instances: u32, vertices: u32, first: u32) {
        if self.used + 4 > self.cap {
            self.overflow = true;
            return;
        }
        self.word(draw_instanced_hdr(indexed, prim, instances & 0xffff));
        self.word(draw_instanced_count(instances >> 16, vertices));
        self.word(first);
        self.word(0);
    }
}
