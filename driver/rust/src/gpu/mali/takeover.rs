// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Take over a GPU that another kernel left running (driver/docs/
//! nanopc-t6-mali.md, "The GPU power-on"): after a crash kexec from Linux,
//! PD_GPU is still powered, panthor's CSF firmware still runs on the MCU,
//! the interrupt masks are set and the address spaces point at Linux's page
//! tables. Before the power-on touches the GPU, [`take_over`] makes it quiet
//! in this order, each step read back:
//!
//! 1. `GPU_INT_MASK`, `JOB_INT_MASK` and `MMU_INT_MASK` written 0 and read
//!    back 0, then every bit of the three `*_INT_CLEAR` written: no interrupt
//!    line is raised from here on, whatever the firmware still signals.
//! 2. The MCU halted: `MCU_CONTROL` := `DISABLE`, then `MCU_STATUS` polled
//!    until it reads `DISABLED` (panthor's `panthor_fw_stop`, panthor_fw.c
//!    v6.18:1061-1069: the same write, the same 100 ms). When it does not
//!    read `DISABLED` in time, the record says so and the reset follows
//!    anyway, as in panthor, whose `panthor_fw_stop` only logs "Failed to
//!    stop MCU" and whose reset path goes on to the soft reset
//!    (panthor_device.c:142-144): the reset is what stops a stuck MCU.
//! 3. `GPU_CMD` := `SOFT_RESET` and `RESET_COMPLETED` waited for in
//!    `GPU_INT_RAWSTAT` ([`super::soft_reset`]); when it does not rise in
//!    100 ms, `GPU_CMD` := `HARD_RESET` (panthor_regs.h:52) and the same
//!    wait. After the reset the address spaces are off: the GPU no longer
//!    walks the other kernel's page tables.
//!
//! panthor's own reset on a hang differs in order: it masks the job
//! interrupt and stops the MCU (`panthor_fw_pre_reset`, :1080-1102), and
//! only then masks the MMU interrupt (`panthor_mmu_pre_reset`, panthor_mmu.c:
//! 1805-1816) before the soft reset (panthor_device.c:141-144). It keeps the
//! GPU interrupt unmasked because it waits for the reset through it. This
//! masks all three before the MCU stop, which only removes interrupt
//! sources earlier, and polls the reset. panthor has no hard reset: a soft
//! reset that times out is an error there (panthor_gpu.c:325-330).
//!
//! The halt does not go through the firmware's global interface
//! (`GLB_HALT`, panthor's graceful path): that interface lives in the other
//! kernel's memory, which this image does not map.
//!
//! The highest GPU offset this touches is `MMU_INT_MASK` (0x2008): the
//! caller maps the GPU for at least [`MAP_MIN`] bytes.

use super::csf::{JOB_INT_CLEAR, JOB_INT_MASK, JOB_INT_RAWSTAT, MCU_CONTROL, MCU_CONTROL_DISABLE, MCU_STATUS_DISABLED};
use super::mmu::{MMU_INT_CLEAR, MMU_INT_MASK, MMU_INT_RAWSTAT};
use super::{soft_reset, MaliRefusal, Reset, CMD_HARD_RESET, GPU_INT_CLEAR, GPU_INT_MASK, GPU_INT_RAWSTAT, MCU_STATUS};
use crate::mmio::Mmio;

/// The three interrupt blocks: (`*_INT_RAWSTAT`, `*_INT_CLEAR`,
/// `*_INT_MASK`) of the GPU, the job interface and the MMU, in that order.
pub const IRQ_BLOCKS: [(usize, usize, usize); 3] = [
    (GPU_INT_RAWSTAT, GPU_INT_CLEAR, GPU_INT_MASK),
    (JOB_INT_RAWSTAT, JOB_INT_CLEAR, JOB_INT_MASK),
    (MMU_INT_RAWSTAT, MMU_INT_CLEAR, MMU_INT_MASK),
];

/// The least GPU mapping [`take_over`] needs: the page past the highest
/// register it touches (the MMU interrupt block, 0x2000..0x200c).
pub const MAP_MIN: usize = 0x3000;

const fn highest_irq_register() -> usize {
    let mut hi = 0;
    let mut i = 0;
    while i < IRQ_BLOCKS.len() {
        let (a, b, c) = IRQ_BLOCKS[i];
        let m = if a > b { a } else { b };
        let m = if m > c { m } else { c };
        if m > hi {
            hi = m;
        }
        i += 1;
    }
    hi
}
const _: () = assert!(highest_irq_register() + 4 <= MAP_MIN && MCU_STATUS + 4 <= MAP_MIN);

/// panthor's wait for the MCU to stop: 100 ms (panthor_fw.c v6.18:1066-1067).
pub const MCU_HALT_WAIT_US: u32 = 100_000;

/// How far [`take_over`] got ([`TakeOver::steps`]).
pub const STEP_MASKED: u32 = 1;
pub const STEP_HALT_WAITED: u32 = 2;
pub const STEP_RESET: u32 = 3;

/// What [`take_over`] found and did, in the order of [`IRQ_BLOCKS`] where a
/// field has three words. A field of a step not reached stays 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TakeOver {
    /// The steps done: 0 the masks did not take, [`STEP_MASKED`] masked and
    /// cleared, [`STEP_HALT_WAITED`] the MCU halt waited for (done or timed
    /// out), [`STEP_RESET`] a reset completed.
    pub steps: u32,
    /// The three `*_INT_MASK` as found.
    pub masks: [u32; 3],
    /// The three `*_INT_RAWSTAT` as found, and after the clear.
    pub raw: [u32; 3],
    pub raw_cleared: [u32; 3],
    /// `MCU_STATUS` as found.
    pub mcu_status: u32,
    /// Polls (each followed by a 1 µs wait) before `MCU_STATUS` read
    /// `DISABLED`; [`MCU_HALT_WAIT_US`] + 1 when it did not.
    pub mcu_waited_us: u32,
    /// `MCU_STATUS` did not read `DISABLED` within [`MCU_HALT_WAIT_US`]; the
    /// reset ran anyway.
    pub mcu_halt_timed_out: bool,
    /// `MCU_STATUS` at the end of the wait.
    pub mcu_status_halted: u32,
    /// The soft reset timed out and the hard reset ran; `reset` is then the
    /// hard reset's.
    pub hard: bool,
    pub reset: Reset,
    /// The three `*_INT_MASK` and `MCU_STATUS` after the reset.
    pub after_reset: [u32; 4],
}

/// The take-over (see the module). The GPU's domain is usable, its clocks
/// open and at least [`MAP_MIN`] bytes of it mapped at `base`. On a refusal,
/// `out` holds what was read up to the step that refused (`out.steps` says
/// which), and the pair is the refusal and the value it is about:
///
/// - [`MaliRefusal::IrqMaskStuck`]: a mask read back other than 0 (the
///   value: the mask's offset << 32 | the word read). Nothing else was
///   written after the three masks.
/// - [`MaliRefusal::ResetBitStuck`]: as [`super::soft_reset`]; no reset was
///   commanded.
/// - [`MaliRefusal::HardResetTimeout`]: neither reset completed (the value:
///   `GPU_INT_RAWSTAT`).
///
/// An MCU that does not halt is no refusal: [`TakeOver::mcu_halt_timed_out`].
pub fn take_over(
    m: &impl Mmio,
    base: usize,
    delay_us: &mut impl FnMut(u32),
    out: &mut TakeOver,
) -> Result<(), (MaliRefusal, u64)> {
    for (i, (raw, _, mask)) in IRQ_BLOCKS.into_iter().enumerate() {
        out.masks[i] = m.read32(base + mask);
        out.raw[i] = m.read32(base + raw);
    }
    out.mcu_status = m.read32(base + MCU_STATUS);
    for (_, _, mask) in IRQ_BLOCKS {
        m.write32(base + mask, 0);
    }
    for (_, _, mask) in IRQ_BLOCKS {
        let now = m.read32(base + mask);
        if now != 0 {
            return Err((MaliRefusal::IrqMaskStuck, (mask as u64) << 32 | u64::from(now)));
        }
    }
    for (_, clear, _) in IRQ_BLOCKS {
        m.write32(base + clear, u32::MAX);
    }
    for (i, (raw, _, _)) in IRQ_BLOCKS.into_iter().enumerate() {
        out.raw_cleared[i] = m.read32(base + raw);
    }
    out.steps = STEP_MASKED;

    m.write32(base + MCU_CONTROL, MCU_CONTROL_DISABLE);
    match wait32(m, base + MCU_STATUS, MCU_HALT_WAIT_US, delay_us, |s| s == MCU_STATUS_DISABLED) {
        Some(t) => out.mcu_waited_us = t,
        None => {
            out.mcu_waited_us = MCU_HALT_WAIT_US + 1;
            out.mcu_halt_timed_out = true;
        }
    }
    out.mcu_status_halted = m.read32(base + MCU_STATUS);
    out.steps = STEP_HALT_WAITED;

    out.reset = match soft_reset(m, base, delay_us) {
        Ok(r) => r,
        Err((MaliRefusal::ResetTimeout, _)) => {
            out.hard = true;
            super::reset_with(m, base, CMD_HARD_RESET, delay_us).map_err(|(why, raw)| {
                let why = if why == MaliRefusal::ResetTimeout { MaliRefusal::HardResetTimeout } else { why };
                (why, u64::from(raw))
            })?
        }
        Err((why, raw)) => return Err((why, u64::from(raw))),
    };
    for (i, (_, _, mask)) in IRQ_BLOCKS.into_iter().enumerate() {
        out.after_reset[i] = m.read32(base + mask);
    }
    out.after_reset[3] = m.read32(base + MCU_STATUS);
    out.steps = STEP_RESET;
    Ok(())
}

/// Poll the word at `addr` once per microsecond for at most `budget_us`;
/// the microseconds it took.
fn wait32(
    m: &impl Mmio,
    addr: usize,
    budget_us: u32,
    delay_us: &mut impl FnMut(u32),
    cond: impl Fn(u32) -> bool,
) -> Option<u32> {
    for t in 0..=budget_us {
        if cond(m.read32(addr)) {
            return Some(t);
        }
        delay_us(1);
    }
    None
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::csf::{JOB_INT_GLOBAL_IF, MCU_CONTROL_AUTO, MCU_STATUS_ENABLED};
    use super::super::{CMD_SOFT_RESET, GPU_CMD, IRQ_RESET_COMPLETED, RESET_WAIT_US};
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    const BASE: usize = 0xfb00_0000;

    /// A G610 as panthor leaves it running (the facts of
    /// driver/docs/nanopc-t6-display.md 2(a)): panthor's masks
    /// (`GPU_INTERRUPTS_MASK` panthor_gpu.c:43-47, the job mask ~0 of
    /// panthor_fw.c:1033, the MMU's fault bits), the firmware's global
    /// interrupt and an MMU fault pending, the MCU enabled. While the MCU is
    /// enabled the firmware raises the global interrupt again on each read
    /// of `JOB_INT_RAWSTAT`. `MCU_CONTROL` := `DISABLE` stops it after
    /// `halt_lag` polls (`None`: never). The model knows the reset commands
    /// by panthor's numbers, written out (panthor_regs.h v6.18:50-52: 0x101
    /// soft, 0x201 hard), not by this module's constants. A reset completes
    /// at once unless its command is in `dead`; it stops the MCU and leaves
    /// the masks and `MCU_STATUS` as `after_reset` says (`None`: masks as
    /// they were, MCU disabled). Every write is logged, and each reset
    /// command keeps the masks and `MCU_STATUS` it met.
    pub(crate) struct Running {
        pub(crate) regs: RefCell<HashMap<usize, u32>>,
        pub(crate) writes: RefCell<Vec<(usize, u32)>>,
        pub(crate) halt_lag: Option<u32>,
        halt_left: RefCell<Option<u32>>,
        pub(crate) dead: Vec<u32>,
        /// Masks whose writes do not take.
        pub(crate) stuck_mask: Option<usize>,
        /// (command, masks, `MCU_STATUS`) at each reset command.
        pub(crate) resets: RefCell<Vec<(u32, [u32; 3], u32)>>,
        /// The three masks and `MCU_STATUS` a completed reset leaves.
        pub(crate) after_reset: Option<[u32; 4]>,
    }

    /// panthor_regs.h v6.18:51-52.
    const SOFT: u32 = 0x101;
    const HARD: u32 = 0x201;

    impl Running {
        pub(crate) fn new() -> Running {
            let r = [
                (GPU_INT_MASK, 0x2_0103),
                (JOB_INT_MASK, u32::MAX),
                (MMU_INT_MASK, 0xffff),
                (JOB_INT_RAWSTAT, JOB_INT_GLOBAL_IF),
                (MMU_INT_RAWSTAT, 1),
                (MCU_CONTROL, MCU_CONTROL_AUTO),
                (MCU_STATUS, MCU_STATUS_ENABLED),
            ];
            Running {
                regs: RefCell::new(r.into_iter().collect()),
                writes: RefCell::new(vec![]),
                halt_lag: Some(3),
                halt_left: RefCell::new(None),
                dead: vec![],
                stuck_mask: None,
                resets: RefCell::new(vec![]),
                after_reset: None,
            }
        }
        fn reg(&self, off: usize) -> u32 {
            *self.regs.borrow().get(&off).unwrap_or(&0)
        }
        fn masks(&self) -> [u32; 3] {
            IRQ_BLOCKS.map(|(_, _, mask)| self.reg(mask))
        }
    }

    impl Mmio for Running {
        fn read32(&self, a: usize) -> u32 {
            let off = a - BASE;
            if off == MCU_STATUS {
                let mut left = self.halt_left.borrow_mut();
                match *left {
                    Some(0) => {
                        self.regs.borrow_mut().insert(MCU_STATUS, MCU_STATUS_DISABLED);
                        *left = None;
                    }
                    Some(n) => *left = Some(n - 1),
                    None => {}
                }
            }
            if off == JOB_INT_RAWSTAT && self.reg(MCU_STATUS) == MCU_STATUS_ENABLED {
                *self.regs.borrow_mut().entry(JOB_INT_RAWSTAT).or_insert(0) |= JOB_INT_GLOBAL_IF;
            }
            self.reg(off)
        }
        fn write32(&self, a: usize, v: u32) {
            let off = a - BASE;
            self.writes.borrow_mut().push((off, v));
            if let Some(&(raw, _, _)) = IRQ_BLOCKS.iter().find(|b| b.1 == off) {
                *self.regs.borrow_mut().entry(raw).or_insert(0) &= !v;
                return;
            }
            if Some(off) == self.stuck_mask {
                return;
            }
            if off == MCU_CONTROL && v == MCU_CONTROL_DISABLE {
                *self.halt_left.borrow_mut() = self.halt_lag;
            }
            if off == GPU_CMD && (v == SOFT || v == HARD) {
                self.resets.borrow_mut().push((v, self.masks(), self.reg(MCU_STATUS)));
                if !self.dead.contains(&v) {
                    let mut r = self.regs.borrow_mut();
                    *r.entry(GPU_INT_RAWSTAT).or_insert(0) |= IRQ_RESET_COMPLETED;
                    r.insert(MCU_STATUS, MCU_STATUS_DISABLED);
                    if let Some(after) = self.after_reset {
                        for (i, (_, _, mask)) in IRQ_BLOCKS.into_iter().enumerate() {
                            r.insert(mask, after[i]);
                        }
                        r.insert(MCU_STATUS, after[3]);
                    }
                }
                return;
            }
            assert_ne!(off, GPU_CMD, "a GPU_CMD that is no reset: {v:#x}");
            self.regs.borrow_mut().insert(off, v);
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!()
        }
        fn write8(&self, _: usize, _: u8) {
            unreachable!()
        }
    }

    fn run(g: &Running) -> (Result<(), (MaliRefusal, u64)>, TakeOver, u64) {
        let mut out = TakeOver::default();
        let mut slept = 0u64;
        let r = take_over(g, BASE, &mut |us| slept += u64::from(us), &mut out);
        (r, out, slept)
    }

    /// The index in the write log of the first write that `f` matches.
    fn first(g: &Running, f: impl Fn(&(usize, u32)) -> bool) -> usize {
        g.writes.borrow().iter().position(f).expect("no such write")
    }

    #[test]
    fn a_running_gpu_is_masked_cleared_halted_then_reset() {
        let g = Running::new();
        let (r, out, _) = run(&g);
        assert_eq!(r, Ok(()));
        assert_eq!(out.masks, [0x2_0103, u32::MAX, 0xffff]);
        assert_eq!(out.raw, [0, JOB_INT_GLOBAL_IF, 1]);
        // the firmware still runs at the clear: its global interrupt is back
        assert_eq!(out.raw_cleared, [0, JOB_INT_GLOBAL_IF, 0]);
        assert_eq!((out.mcu_status, out.mcu_waited_us, out.hard), (MCU_STATUS_ENABLED, 3, false));
        assert_eq!(
            (out.mcu_halt_timed_out, out.mcu_status_halted, out.steps),
            (false, MCU_STATUS_DISABLED, STEP_RESET)
        );
        assert_eq!(out.reset, Reset { raw_before: 0, raw_cleared: 0, waited_us: 0 });
        assert_eq!(out.after_reset, [0, 0, 0, MCU_STATUS_DISABLED]);
        // the one reset met the masks at 0 and the MCU disabled
        assert_eq!(*g.resets.borrow(), vec![(SOFT, [0; 3], MCU_STATUS_DISABLED)]);
    }

    #[test]
    fn every_mask_is_zero_before_any_clear_and_every_clear_before_the_halt() {
        let g = Running::new();
        assert_eq!(run(&g).0, Ok(()));
        let last_mask = IRQ_BLOCKS.iter().map(|b| first(&g, |w| w.0 == b.2)).max().unwrap();
        let clears: Vec<usize> = IRQ_BLOCKS.iter().map(|b| first(&g, |w| *w == (b.1, u32::MAX))).collect();
        let halt = first(&g, |w| *w == (MCU_CONTROL, MCU_CONTROL_DISABLE));
        let reset = first(&g, |w| w.0 == GPU_CMD);
        assert!(IRQ_BLOCKS.iter().all(|b| g.writes.borrow().contains(&(b.2, 0))));
        assert!(clears.iter().all(|&c| last_mask < c && c < halt), "{:x?}", g.writes.borrow());
        assert!(halt < reset);
    }

    #[test]
    fn a_mask_that_reads_back_set_stops_before_the_clear_and_the_halt() {
        let mut g = Running::new();
        g.stuck_mask = Some(JOB_INT_MASK);
        let (r, out, _) = run(&g);
        assert_eq!(r, Err((MaliRefusal::IrqMaskStuck, (JOB_INT_MASK as u64) << 32 | u64::from(u32::MAX))));
        assert_eq!(out.masks, [0x2_0103, u32::MAX, 0xffff]);
        // the three mask writes, nothing after them
        let w = g.writes.borrow();
        assert_eq!(*w, IRQ_BLOCKS.map(|b| (b.2, 0)).to_vec());
    }

    #[test]
    fn an_mcu_that_does_not_halt_is_recorded_and_reset_anyway() {
        let mut g = Running::new();
        g.halt_lag = None;
        let (r, out, slept) = run(&g);
        assert_eq!(r, Ok(()));
        assert!(out.mcu_halt_timed_out);
        assert_eq!((out.mcu_waited_us, out.mcu_status_halted), (MCU_HALT_WAIT_US + 1, MCU_STATUS_ENABLED));
        assert_eq!(slept, u64::from(MCU_HALT_WAIT_US) + 1);
        // the soft reset met the masks at 0; it is what stops the MCU
        assert_eq!(*g.resets.borrow(), vec![(SOFT, [0; 3], MCU_STATUS_ENABLED)]);
        assert_eq!((out.steps, out.after_reset[3]), (STEP_RESET, MCU_STATUS_DISABLED));
    }

    #[test]
    fn the_mcu_wait_is_panthors_100_ms() {
        // panthor_fw.c v6.18:1066-1067: 100000 us
        for (lag, timed_out) in [(100_000, false), (100_001, true)] {
            let mut g = Running::new();
            g.halt_lag = Some(lag);
            let (r, out, _) = run(&g);
            assert_eq!(r, Ok(()));
            assert_eq!(out.mcu_halt_timed_out, timed_out, "{lag}");
        }
    }

    #[test]
    fn the_reset_commands_are_panthors_numbers() {
        // GPU_CMD_DEF(1, 1) and GPU_CMD_DEF(1, 2), panthor_regs.h v6.18:50-52
        assert_eq!((CMD_SOFT_RESET, CMD_HARD_RESET), (SOFT, HARD));
    }

    #[test]
    fn the_record_keeps_what_the_reset_left() {
        let mut g = Running::new();
        g.after_reset = Some([0x11, 0x22, 0x33, 2]);
        let (r, out, _) = run(&g);
        assert_eq!(r, Ok(()));
        assert_eq!(out.after_reset, [0x11, 0x22, 0x33, 2]);
    }

    #[test]
    fn a_refusal_leaves_the_steps_it_reached() {
        let mut g = Running::new();
        g.stuck_mask = Some(MMU_INT_MASK);
        let (_, out, _) = run(&g);
        assert_eq!((out.steps, out.raw_cleared, out.mcu_waited_us), (0, [0; 3], 0));
        let mut g = Running::new();
        g.dead = vec![SOFT, HARD];
        let (_, out, _) = run(&g);
        assert_eq!((out.steps, out.mcu_waited_us, out.after_reset), (STEP_HALT_WAITED, 3, [0; 4]));
    }

    #[test]
    fn a_soft_reset_that_times_out_is_followed_by_a_hard_reset() {
        let mut g = Running::new();
        g.dead = vec![SOFT];
        let (r, out, slept) = run(&g);
        assert_eq!(r, Ok(()));
        assert!(out.hard);
        assert_eq!(slept, 3 + u64::from(RESET_WAIT_US) + 1);
        let resets: Vec<u32> = g.resets.borrow().iter().map(|r| r.0).collect();
        assert_eq!(resets, vec![SOFT, HARD]);
        // the hard reset is commanded after the bit was cleared and read clear again
        let w = g.writes.borrow();
        let hard = w.iter().position(|w| *w == (GPU_CMD, HARD)).unwrap();
        assert_eq!(w[hard - 1], (GPU_INT_CLEAR, IRQ_RESET_COMPLETED));
        assert_eq!(*w.last().unwrap(), (GPU_INT_CLEAR, IRQ_RESET_COMPLETED));
    }

    #[test]
    fn a_hard_reset_that_times_out_is_refused_with_its_own_number() {
        let mut g = Running::new();
        g.dead = vec![SOFT, HARD];
        let (r, out, _) = run(&g);
        assert_eq!(r, Err((MaliRefusal::HardResetTimeout, 0)));
        assert!(out.hard);
        assert_eq!(out.after_reset, [0; 4]);
    }

    #[test]
    fn a_gpu_already_quiet_is_written_the_same_words() {
        // a GPU that panthor had stopped: masks 0, MCU disabled
        let g = Running::new();
        for (a, v) in [(GPU_INT_MASK, 0), (JOB_INT_MASK, 0), (MMU_INT_MASK, 0), (MCU_STATUS, MCU_STATUS_DISABLED)] {
            g.regs.borrow_mut().insert(a, v);
        }
        let (r, out, _) = run(&g);
        assert_eq!(r, Ok(()));
        assert_eq!(out.mcu_waited_us, 0);
        let running = Running::new();
        assert_eq!(run(&running).0, Ok(()));
        assert_eq!(*g.writes.borrow(), *running.writes.borrow());
    }
}
