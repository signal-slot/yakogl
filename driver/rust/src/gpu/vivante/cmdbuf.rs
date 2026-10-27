// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The front end: starting it, the ring with its WAIT/LINK idle loop, and one
//! submit.
//!
//! The scheme is galcore's wait/link front end
//! (gc_hal_kernel_hardware_waitlink_fe.c, `gckWLFE_WaitLink`,
//! `gckWLFE_Link`, `gckWLFE_Execute`; gc_hal_kernel_command.c
//! `_CommitWaitLinkOnce`):
//! - the ring ends in WAIT then a LINK back to that WAIT, 16 bytes, LINK
//!   prefetch 2, which the front end spins on;
//! - new work is appended after it, ending in a fresh WAIT/LINK; then the old
//!   WAIT is turned into a LINK to the new work: the address word first, a
//!   barrier, the header word, a barrier -- the header last, so the front end
//!   never fetches a LINK with a stale address. The ring is Normal
//!   non-cacheable on the board, so the barrier is all (galcore also cleans
//!   the CPU cache of a cacheable ring);
//! - the front end is started with FE_COMMAND_ADDRESS, then
//!   FE_COMMAND_CONTROL (ENABLE | prefetch in 64-bit words) and, with the
//!   secure AHB, the same value in MMUv2_SEC_COMMAND_CONTROL.
//!
//! A submit links to the caller's buffer instead of copying it, and writes a
//! LINK back to the ring at the end of it (the caller leaves CMDBUF_TAIL
//! bytes). Its prologue and epilogue are galcore's pipe select
//! (`gckHARDWARE_PipeSelect`: flush, FE->PE semaphore and stall, PIPE_SELECT
//! 3D) and flush (`gckHARDWARE_Flush`: GL_FLUSH_CACHE, TS_FLUSH_CACHE), then
//! the semaphore/stall pair and an EVENT from the pixel engine
//! (`gckWLFE_Event`). The flush sets, the WAIT delay (the core clock >> 15,
//! about 30 us; galcore uses a fixed 200) and the ring arithmetic are the
//! board's: what the C gc_cmdbuf.c ran from 2026-09-20 until it was deleted.

use super::cmd::{link_header, Cmd};
use super::regs::*;
use super::{dmb_sy, hw, submit, GcCore, GcSubmit, CMDBUF_TAIL, EVENT_COUNT, OFF_RING, RING_SIZE};
use crate::env::Env;
use crate::errno::{EFAULT, EINVAL, ETIMEDOUT};
use crate::mmio::Mmio;
use crate::ylog;

pub const WAIT_CYCLES_MIN: u32 = 200;
pub const WAIT_CYCLES_MAX: u32 = 0xffff;
/// WAIT + LINK.
pub const IDLE_LOOP_WORDS: u32 = 4;
/// The most words one submit adds (prologue 12, LINK 2, epilogue 12, idle
/// loop 4; rounded up).
pub const SUBMIT_WORDS: u32 = 32;
const RING_WORDS: u32 = (RING_SIZE / 4) as u32;

// A lap of the ring must hold far more segments than events can be in
// flight, so the start of the ring has retired long before it is reused.
const _: () = assert!(RING_WORDS / SUBMIT_WORDS > 2 * EVENT_COUNT);

/// The idle loop's WAIT delay for a core clock of `hz`.
pub fn wait_cycles(hz: u32) -> u32 {
    (hz >> 15).clamp(WAIT_CYCLES_MIN, WAIT_CYCLES_MAX)
}

/// Start the front end at `address`, `prefetch` 64-bit words.
pub fn fe_start(c: &GcCore, m: &impl Mmio, address: u32, prefetch: u32) {
    let wr = |o: u32, v: u32| m.write32(c.regs + o as usize, v);
    wr(VIVS_FE_COMMAND_ADDRESS, address);
    wr(VIVS_FE_COMMAND_CONTROL, VIVS_FE_COMMAND_CONTROL_ENABLE | (prefetch & VIVS_FE_COMMAND_CONTROL_PREFETCH__MASK));
    if c.identity().is_some_and(|id| id.secure_ahb) {
        wr(
            VIVS_MMUv2_SEC_COMMAND_CONTROL,
            VIVS_MMUv2_SEC_COMMAND_CONTROL_ENABLE | (prefetch & VIVS_MMUv2_SEC_COMMAND_CONTROL_PREFETCH__MASK),
        );
    }
}

/// Wait until every module but AXI_LP reports idle, polling every 5 us.
pub fn fe_wait_idle(c: &GcCore, m: &impl Mmio, env: &impl Env, timeout_ms: u32) -> Result<(), i32> {
    let idle_mask = !VIVS_HI_IDLE_STATE_AXI_LP;
    let mut us = 0;
    while us < timeout_ms.saturating_mul(1000) {
        if m.read32(c.regs + VIVS_HI_IDLE_STATE as usize) & idle_mask == idle_mask {
            return Ok(());
        }
        env.busy_wait_us(5);
        us += 5;
    }
    ylog!(
        env,
        Err,
        "fe: not idle after {} ms (IDLE_STATE 0x{:08x})",
        timeout_ms,
        m.read32(c.regs + VIVS_HI_IDLE_STATE as usize)
    );
    Err(-ETIMEDOUT)
}

fn ring_at(c: &GcCore, pos: u32) -> (usize, u32, u32) {
    (OFF_RING + pos as usize * 4, c.ring_gpu + pos * 4, RING_WORDS - pos)
}

/// The idle loop at word `pos`; returns `pos` (its WAIT).
fn emit_idle_loop(c: &mut GcCore, pos: u32, clock_hz: u32) -> u32 {
    let mem = c.mem();
    let (off, gpu, max) = ring_at(c, pos);
    let mut s = Cmd::new(&mem, off, gpu, max);
    s.wait(wait_cycles(clock_hz));
    s.link(gpu, 2);
    c.ring_pos = pos + s.n;
    pos
}

/// Turn the WAIT at `wait_pos` into a LINK to `target`: address, barrier,
/// header, barrier.
fn replace_wait(c: &GcCore, wait_pos: u32, target: u32, prefetch: u32) {
    let mem = c.mem();
    let at = OFF_RING + wait_pos as usize * 4;
    mem.w32(at + 4, target);
    dmb_sy();
    mem.w32(at, link_header(prefetch));
    dmb_sy();
}

/// The ring and its idle loop, and the front end started on it (MMU on, at
/// the ring's GPU VA, which the identity mapping makes its physical address).
pub fn ring_init(c: &mut GcCore, m: &impl Mmio, env: &impl Env, clock_hz: u32) {
    c.ring = c.carveout.wrapping_add(OFF_RING).cast();
    c.ring_gpu = (c.carveout_phys + OFF_RING as u64) as u32;
    c.ring_pos = 0;
    c.wait_pos = emit_idle_loop(c, 0, clock_hz);
    fe_start(c, m, c.ring_gpu, 2);
    c.fe_running = true;
    ylog!(env, Inf, "fe: running the idle loop at 0x{:08x} (WAIT {} cycles)", c.ring_gpu, wait_cycles(clock_hz));
}

/// One submit (the module text).
pub fn ring_submit(
    c: &mut GcCore,
    env: &impl Env,
    s: &GcSubmit,
    event: u32,
    _fence: u32,
    clock_hz: u32,
) -> Result<(), i32> {
    if s.cmd_gpu & 7 != 0 || s.cmd_bytes & 7 != 0 {
        ylog!(env, Err, "submit: buffer 0x{:08x}+{} must be 8-byte aligned in both", s.cmd_gpu, s.cmd_bytes);
        return Err(-EINVAL);
    }
    // Wrap when the segment would not fit. The front end is parked on the
    // old WAIT near the end and moves only at the commit below.
    if c.ring_pos + SUBMIT_WORDS > RING_WORDS {
        c.ring_pos = 0;
    }
    let Some(cmd_end) = c.carveout_offset(s.cmd_gpu.wrapping_add(s.cmd_bytes)) else {
        ylog!(env, Err, "submit: command buffer 0x{:08x}+{} is not in the carveout", s.cmd_gpu, s.cmd_bytes);
        return Err(-EFAULT);
    };
    let mem = c.mem();
    let seg = c.ring_pos;
    let (off, gpu, max) = ring_at(c, seg);
    let mut w = Cmd::new(&mem, off, gpu, max);

    // Prologue: flush, FE waits for PE, select the 3D pipe (undefined after
    // a reset).
    if s.flags & submit::NO_PIPE_SEL == 0 {
        w.state(VIVS_GL_FLUSH_CACHE, VIVS_GL_FLUSH_CACHE_DEPTH | VIVS_GL_FLUSH_CACHE_COLOR);
        w.semaphore(SYNC_RECIPIENT_FE, SYNC_RECIPIENT_PE);
        w.stall(SYNC_RECIPIENT_FE, SYNC_RECIPIENT_PE);
        w.state(VIVS_GL_PIPE_SELECT, PIPE_ID_PIPE_3D);
    }
    // Out to the caller's commands; back to the word after this LINK.
    let ret_pos = seg + w.n + 2;
    w.link(s.cmd_gpu, (s.cmd_bytes + CMDBUF_TAIL) / 8);
    c.ring_pos = seg + w.n;

    // Epilogue: flush what the render dirtied, FE waits for PE, the event.
    let (off, gpu, max) = ring_at(c, ret_pos);
    let mut e = Cmd::new(&mem, off, gpu, max);
    let mut flush = 0;
    if s.flags & submit::FLUSH_COLOR != 0 {
        flush |= VIVS_GL_FLUSH_CACHE_COLOR;
    }
    if s.flags & submit::FLUSH_DEPTH != 0 {
        flush |= VIVS_GL_FLUSH_CACHE_DEPTH;
    }
    if s.flags & submit::FLUSH_TEX != 0 {
        flush |= VIVS_GL_FLUSH_CACHE_TEXTURE | VIVS_GL_FLUSH_CACHE_TEXTUREVS;
    }
    if flush == 0 {
        flush = VIVS_GL_FLUSH_CACHE_DEPTH | VIVS_GL_FLUSH_CACHE_COLOR | VIVS_GL_FLUSH_CACHE_SHADER_L1;
    }
    e.state(VIVS_GL_FLUSH_CACHE, flush);
    e.state(VIVS_TS_FLUSH_CACHE, VIVS_TS_FLUSH_CACHE_FLUSH);
    e.semaphore(SYNC_RECIPIENT_FE, SYNC_RECIPIENT_PE);
    e.stall(SYNC_RECIPIENT_FE, SYNC_RECIPIENT_PE);
    e.event(event, true);
    c.ring_pos = ret_pos + e.n;
    let new_wait = emit_idle_loop(c, c.ring_pos, clock_hz);

    // The LINK at the end of the caller's buffer, back to the epilogue:
    // address, barrier, header, barrier.
    mem.w32(cmd_end + 4, c.ring_gpu + ret_pos * 4);
    dmb_sy();
    mem.w32(cmd_end, link_header((new_wait + 4 - ret_pos) / 2));
    dmb_sy();

    // Commit.
    replace_wait(c, c.wait_pos, c.ring_gpu + seg * 4, (ret_pos - seg) / 2);
    c.wait_pos = new_wait;
    Ok(())
}

/// The stall dump: where the front end is, whether work is pending, and the
/// words around it.
pub fn ring_dump(c: &GcCore, m: &impl Mmio, env: &impl Env) {
    let dma_addr = hw::rd_fe(m, c.regs, VIVS_FE_DMA_ADDRESS);
    let debug = hw::rd_fe(m, c.regs, VIVS_FE_DMA_DEBUG_STATE);
    ylog!(
        env,
        Err,
        "fe: DMA_ADDRESS 0x{:08x} DMA_DEBUG_STATE 0x{:08x} (cmd state {}, dma state {})",
        dma_addr,
        debug,
        debug & VIVS_FE_DMA_DEBUG_STATE_CMD_STATE__MASK,
        (debug & VIVS_FE_DMA_DEBUG_STATE_CMD_DMA_STATE__MASK) >> VIVS_FE_DMA_DEBUG_STATE_CMD_DMA_STATE__SHIFT
    );
    let low = hw::rd_fe(m, c.regs, VIVS_FE_DMA_LOW);
    let high = hw::rd_fe(m, c.regs, VIVS_FE_DMA_HIGH);
    let cmd = m.read32(c.regs + VIVS_FE_COMMAND_ADDRESS as usize);
    let idle = m.read32(c.regs + VIVS_HI_IDLE_STATE as usize);
    ylog!(
        env,
        Err,
        "fe: DMA_LOW 0x{:08x} DMA_HIGH 0x{:08x} COMMAND_ADDRESS 0x{:08x} IDLE_STATE 0x{:08x}",
        low,
        high,
        cmd,
        idle
    );
    ylog!(env, Err, "ring: 0x{:08x}, pos {} dwords, idle-loop WAIT at {}", c.ring_gpu, c.ring_pos, c.wait_pos);
    if c.ring_pos <= c.wait_pos + IDLE_LOOP_WORDS {
        ylog!(
            env,
            Err,
            "ring: NOTHING PENDING -- the front end has consumed every submitted command and is in its idle loop"
        );
    } else {
        ylog!(
            env,
            Err,
            "ring: {} dwords PENDING past the idle loop (pos {}, loop ends {})",
            c.ring_pos - (c.wait_pos + IDLE_LOOP_WORDS),
            c.ring_pos,
            c.wait_pos + IDLE_LOOP_WORDS
        );
    }
    if c.ring.is_null() || dma_addr < c.ring_gpu || dma_addr >= c.ring_gpu + RING_SIZE as u32 {
        return;
    }
    let mem = c.mem();
    let at = (dma_addr - c.ring_gpu) / 4;
    let first = at.saturating_sub(4);
    for i in first..(first + 8).min(RING_WORDS) {
        let mark = if i == at { "  <- FE" } else { "" };
        ylog!(env, Err, "  ring[{}] 0x{:08x}{}", i, mem.r32(OFF_RING + i as usize * 4), mark);
    }
}
