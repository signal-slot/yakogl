// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Events, fences and the interrupt's work.
//!
//! An EVENT in the stream makes the GPU raise its interrupt with the event's
//! bit set in HI_INTR_ACKNOWLEDGE, which a read clears; bit 31 is an AXI bus
//! error, bit 30 an MMU exception, bits 29:0 the 30 event ids (rnndb
//! INTR_VEC / MMU_EXCEPTION / AXI_BUS_ERROR; galcore
//! gc_hal_kernel_hardware.c `gckHARDWARE_Interrupt` reads it, a zero read is
//! "not our interrupt", and gc_hal_kernel_event.c `gckEVENT_Notify` decodes
//! the two error bits the same way). The driver maps each event id to a
//! monotonic fence; events may retire out of order, so the completed fence
//! only moves forward. (galcore reserves event 29 to resume its front end
//! after an MMU flush; this driver never flushes the MMU, so all 30 ids are
//! fences.)
//!
//! The kernel objects stay in C: the shim holds the spinlock around
//! [`claim`] and [`handle`], and gives the event semaphore once per event
//! [`handle`] freed.

use super::regs::*;
use super::{GcCore, EVENT_COUNT};
use crate::errno::EIO;
use crate::mmio::Mmio;

/// Claim a free event for `fence` (caller holds the lock). `Ok(None)` when
/// all 30 are in flight; `Err(-EIO)` for a corrupt bitmap offering an id the
/// hardware does not have (the id is taken out of the bitmap).
pub fn claim(c: &mut GcCore, fence: u32) -> Result<Option<u32>, i32> {
    if c.event_free == 0 {
        return Ok(None);
    }
    let id = c.event_free.trailing_zeros();
    c.event_free &= !(1 << id);
    if id >= EVENT_COUNT {
        return Err(-EIO);
    }
    c.event_fence[id as usize] = fence;
    Ok(Some(id))
}

/// What one interrupt did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Handled {
    /// This GPU raised it (INTR_ACKNOWLEDGE was not zero).
    pub ours: bool,
    /// Events freed: the shim gives the event semaphore this many times.
    pub freed: u32,
}

/// The interrupt's work (caller holds the lock). `now_ns` is the
/// performance clock at the interrupt.
pub fn handle(c: &mut GcCore, m: &impl Mmio, now_ns: u64) -> Handled {
    let mut intr = m.read32(c.regs + VIVS_HI_INTR_ACKNOWLEDGE as usize);
    if intr == 0 {
        return Handled::default();
    }
    if intr & VIVS_HI_INTR_ACKNOWLEDGE_AXI_BUS_ERROR != 0 {
        c.bump_irq_errors();
        intr &= !VIVS_HI_INTR_ACKNOWLEDGE_AXI_BUS_ERROR;
    }
    if intr & VIVS_HI_INTR_ACKNOWLEDGE_MMU_EXCEPTION != 0 {
        c.bump_irq_errors();
        intr &= !VIVS_HI_INTR_ACKNOWLEDGE_MMU_EXCEPTION;
    }
    let mut freed = 0;
    while intr != 0 {
        let id = intr.trailing_zeros();
        intr &= !(1 << id);
        if id >= EVENT_COUNT {
            continue;
        }
        let i = id as usize;
        let fence = c.event_fence[i];
        c.event_fence[i] = 0;
        c.event_free |= 1 << id;
        if fence != 0 && (fence.wrapping_sub(c.fence_done()) as i32) > 0 {
            c.set_fence_done(fence);
        }
        if fence != 0 {
            // Busy time is the union of the kicks' submit..retire spans.
            let t0 = c.event_t0_ns[i].max(c.perf_last_done_ns);
            if now_ns > t0 {
                c.perf_busy_ns += now_ns - t0;
            }
            c.perf_last_done_ns = c.perf_last_done_ns.max(now_ns);
            c.perf_kicks_done += 1;
        }
        freed += 1;
    }
    Handled { ours: true, freed }
}
