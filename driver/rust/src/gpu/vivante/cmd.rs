// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Front-end command words: the encoder the ring and the MMU-enable stream
//! are written with.
//!
//! The encodings are rnndb `cmdstream.xml.h` ([`super::regs`]): every
//! command is one or two 32-bit words in a 64-bit slot, so an odd-length one
//! is padded with a NOP and sizes given to the hardware are in 64-bit words.
//! LOAD_STATE (opcode 1) is the only way to write a state from the stream: a
//! header with the first state's offset divided by four and a count, then the
//! values. galcore emits the same words (gc_hal_kernel_hardware.c,
//! gckHARDWARE_PipeSelect / _FlushCache / gckHARDWARE_Event;
//! gc_hal_kernel_hardware_waitlink_fe.c, gckWLFE_WaitLink / gckWLFE_Link /
//! gckWLFE_End).
//!
//! `driver/src/vivante/gc_cmd.c` stays: the 3D composition code (`gc_draw.c`)
//! is written in it. This is the same encoder for the kernel side's own
//! streams, and the host tests hold the two to the same words.

use super::regs::*;
use super::Mem;

/// A cursor over GPU-visible words at `off` (bytes into the carveout).
/// `n` counts the words emitted; words past `max` are counted and not
/// stored, as `gc_cmd.c`'s `put()` does, so a caller sizes from the count.
pub struct Cmd<'a> {
    mem: &'a Mem,
    off: usize,
    /// GPU VA of word 0.
    pub gpu: u32,
    pub n: u32,
    pub max: u32,
}

impl<'a> Cmd<'a> {
    pub fn new(mem: &'a Mem, off: usize, gpu: u32, max: u32) -> Cmd<'a> {
        Cmd { mem, off, gpu, n: 0, max }
    }

    pub fn room(&self, words: u32) -> bool {
        self.n + words <= self.max
    }

    fn put(&mut self, w: u32) {
        if self.n < self.max {
            self.mem.w32(self.off + self.n as usize * 4, w);
        }
        self.n += 1;
    }

    /// LOAD_STATE of `vals.len()` consecutive states from `reg`, padded to
    /// 64 bits.
    pub fn state_multi(&mut self, reg: u32, vals: &[u32]) {
        let count = vals.len() as u32;
        self.put(
            VIV_FE_LOAD_STATE_HEADER_OP_LOAD_STATE
                | ((count << VIV_FE_LOAD_STATE_HEADER_COUNT__SHIFT) & VIV_FE_LOAD_STATE_HEADER_COUNT__MASK)
                | ((reg >> VIV_FE_LOAD_STATE_HEADER_OFFSET__SHR) & VIV_FE_LOAD_STATE_HEADER_OFFSET__MASK),
        );
        for &v in vals {
            self.put(v);
        }
        if (count + 1) & 1 != 0 {
            self.put(VIV_FE_NOP_HEADER_OP_NOP);
        }
    }

    pub fn state(&mut self, reg: u32, val: u32) {
        self.state_multi(reg, &[val]);
    }

    pub fn nop(&mut self) {
        self.put(VIV_FE_NOP_HEADER_OP_NOP);
        self.put(0);
    }

    pub fn end(&mut self) {
        self.put(VIV_FE_END_HEADER_OP_END);
        self.put(0);
    }

    /// The fence: a LOAD_STATE of GL_EVENT with the id and the unit that
    /// raises it (FROM_PE: when the pixel engine has retired everything).
    pub fn event(&mut self, id: u32, from_pe: bool) {
        let src = if from_pe { VIVS_GL_EVENT_FROM_PE } else { VIVS_GL_EVENT_FROM_FE };
        self.state(VIVS_GL_EVENT, (id & VIVS_GL_EVENT_EVENT_ID__MASK) | src);
    }

    pub fn semaphore(&mut self, from: u32, to: u32) {
        self.state(
            VIVS_GL_SEMAPHORE_TOKEN,
            (from & VIVS_GL_SEMAPHORE_TOKEN_FROM__MASK)
                | ((to << VIVS_GL_SEMAPHORE_TOKEN_TO__SHIFT) & VIVS_GL_SEMAPHORE_TOKEN_TO__MASK),
        );
    }

    pub fn stall(&mut self, from: u32, to: u32) {
        self.put(VIV_FE_STALL_HEADER_OP_STALL);
        self.put(
            (from & VIV_FE_STALL_TOKEN_FROM__MASK)
                | ((to << VIV_FE_STALL_TOKEN_TO__SHIFT) & VIV_FE_STALL_TOKEN_TO__MASK),
        );
    }

    /// `prefetch` in 64-bit words.
    pub fn link(&mut self, address: u32, prefetch: u32) {
        self.put(link_header(prefetch));
        self.put(address);
    }

    pub fn wait(&mut self, delay: u32) {
        self.put(VIV_FE_WAIT_HEADER_OP_WAIT | (delay & VIV_FE_WAIT_HEADER_DELAY__MASK));
        self.put(0);
    }

    pub fn align8(&mut self) {
        while self.n & 1 != 0 {
            self.put(VIV_FE_NOP_HEADER_OP_NOP);
        }
    }
}

/// A LINK header word.
pub const fn link_header(prefetch: u32) -> u32 {
    VIV_FE_LINK_HEADER_OP_LINK | (prefetch & VIV_FE_LINK_HEADER_PREFETCH__MASK)
}
