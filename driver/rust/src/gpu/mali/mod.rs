// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Mali-G610 (Valhall, CSF) of the RK3588: the kernel side's first
//! steps (feature `mali`): read the identity, soft reset, set the coherency
//! mode, power the L2 cache on (driver/docs/nanopc-t6-mali.md, "Z1").
//!
//! Source: Linux v6.18's panthor, used under the MIT option of its
//! `GPL-2.0 or MIT` licence (the design page's decision 1): register
//! offsets from drivers/gpu/drm/panthor/panthor_regs.h, the sequences from
//! panthor_gpu.c (Copyright 2018 Marty E. Plummer, 2019 Linaro, Ltd,
//! 2023 Collabora ltd.; register definitions based on ARM's
//! mali_kbase_gpu_regmap.h). Each item names its line.
//!
//! The host never powers the shader cores or the tiler: the firmware does
//! (panthor_gpu.c:356-366, "The FW takes care of powering the other
//! blocks").
//!
//! Measured on the NanoPC-T6 under Armbian (2026-10-18/19): panthor's probe
//! line "Mali-G610 id 0xa867 major 0x0 minor 0x0 status 0x5", "Features:
//! L2:0x7120306 Tiler:0x809 Mem:0x301 MMU:0x2830 AS:0xff", "shader_present=
//! 0x50005 l2_present=0x1 tiler_present=0x1"; `GPU_ID` read with the GPU
//! active: 0xa8670005.

use crate::mmio::Mmio;

/// Registers (panthor_regs.h:13-126).
pub const GPU_ID: usize = 0x0;
pub const GPU_L2_FEATURES: usize = 0x4;
pub const GPU_TILER_FEATURES: usize = 0xc;
pub const GPU_MEM_FEATURES: usize = 0x10;
pub const GPU_MMU_FEATURES: usize = 0x14;
pub const GPU_AS_PRESENT: usize = 0x18;
pub const GPU_CSF_ID: usize = 0x1c;
pub const GPU_INT_RAWSTAT: usize = 0x20;
pub const GPU_INT_CLEAR: usize = 0x24;
pub const GPU_INT_MASK: usize = 0x28;
pub const GPU_CMD: usize = 0x30;
pub const GPU_STATUS: usize = 0x34;
pub const GPU_SHADER_PRESENT: usize = 0x100;
pub const GPU_TILER_PRESENT: usize = 0x110;
pub const GPU_L2_PRESENT: usize = 0x120;
pub const L2_READY: usize = 0x160;
pub const L2_PWRON: usize = 0x1a0;
pub const L2_PWRTRANS: usize = 0x220;
pub const GPU_COHERENCY_FEATURES: usize = 0x300;
pub const GPU_COHERENCY_PROTOCOL: usize = 0x304;
pub const MCU_STATUS: usize = 0x704;

/// `GPU_IRQ_RESET_COMPLETED` (panthor_regs.h:43).
pub const IRQ_RESET_COMPLETED: u32 = 1 << 8;
/// `GPU_SOFT_RESET` = `GPU_CMD_DEF(1, 1)` (panthor_regs.h:51-53).
pub const CMD_SOFT_RESET: u32 = 1 | (1 << 8);
/// `GPU_COHERENCY_NONE` (panthor_regs.h:119): the RK3588's GPU node has no
/// `dma-coherent`, so panthor writes this (panthor_gpu.c:49-53).
pub const COHERENCY_NONE: u32 = 31;

/// The RK3588's Mali-G610 as the T6 reads it: arch 10.8, product 7.
pub const G610_GPU_ID: u32 = 0xa867_0005;

/// panthor's waits: the soft reset 100 ms (panthor_gpu.c:318), the L2
/// 20 ms (:247).
pub const RESET_WAIT_US: u32 = 100_000;
pub const L2_WAIT_US: u32 = 20_000;

/// The identity block, read with no write.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ident {
    pub gpu_id: u32,
    pub l2_features: u32,
    pub tiler_features: u32,
    pub mem_features: u32,
    pub mmu_features: u32,
    pub as_present: u32,
    pub csf_id: u32,
    pub shader_present: u64,
    pub tiler_present: u64,
    pub l2_present: u64,
    pub coherency_features: u32,
    pub status: u32,
    pub mcu_status: u32,
}

fn read64(m: &impl Mmio, addr: usize) -> u64 {
    u64::from(m.read32(addr)) | u64::from(m.read32(addr + 4)) << 32
}

/// Read the identity registers of the GPU at `base`. The caller has powered
/// PD_GPU and opened its clocks: on the RK3588 a read of a GPU register
/// with the domain off is an external abort.
pub fn ident(m: &impl Mmio, base: usize) -> Ident {
    Ident {
        gpu_id: m.read32(base + GPU_ID),
        l2_features: m.read32(base + GPU_L2_FEATURES),
        tiler_features: m.read32(base + GPU_TILER_FEATURES),
        mem_features: m.read32(base + GPU_MEM_FEATURES),
        mmu_features: m.read32(base + GPU_MMU_FEATURES),
        as_present: m.read32(base + GPU_AS_PRESENT),
        csf_id: m.read32(base + GPU_CSF_ID),
        shader_present: read64(m, base + GPU_SHADER_PRESENT),
        tiler_present: read64(m, base + GPU_TILER_PRESENT),
        l2_present: read64(m, base + GPU_L2_PRESENT),
        coherency_features: m.read32(base + GPU_COHERENCY_FEATURES),
        status: m.read32(base + GPU_STATUS),
        mcu_status: m.read32(base + MCU_STATUS),
    }
}

/// Why a step stopped. The value crosses to C, one number per cause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum MaliRefusal {
    /// `RESET_COMPLETED` did not rise within [`RESET_WAIT_US`].
    ResetTimeout = 1,
    /// `L2_PWRTRANS` stayed set for the L2 before the power-on.
    L2TransTimeout = 2,
    /// `L2_READY` did not show the L2 within [`L2_WAIT_US`].
    L2ReadyTimeout = 3,
    /// `L2_PRESENT` is 0: there is nothing to power.
    NoL2 = 4,
    /// `RESET_COMPLETED` still reads set after its clear: a completion
    /// could not be told from an old one.
    ResetBitStuck = 5,
}

/// Poll the 64-bit register at `addr` once per microsecond for at most
/// `budget_us`; the microseconds it took.
fn wait64(
    m: &impl Mmio,
    addr: usize,
    budget_us: u32,
    delay_us: &mut impl FnMut(u32),
    cond: impl Fn(u64) -> bool,
) -> Option<u32> {
    for t in 0..=budget_us {
        if cond(read64(m, addr)) {
            return Some(t);
        }
        delay_us(1);
    }
    None
}

/// Soft reset (panthor_gpu.c:302-334): clear `RESET_COMPLETED`, write
/// `GPU_SOFT_RESET`, wait for `RESET_COMPLETED` in `GPU_INT_RAWSTAT`
/// (panthor waits for the interrupt; with `GPU_INT_MASK` as found this
/// polls the raw status, which panthor's timeout path reads too, :319-323).
/// The microseconds it took.
///
/// The record says whether the completion is this reset's: `GPU_INT_RAWSTAT`
/// before the clear, after it (the command is written only when it reads
/// clear), and the polls it took.
pub fn soft_reset(m: &impl Mmio, base: usize, delay_us: &mut impl FnMut(u32)) -> Result<Reset, (MaliRefusal, u32)> {
    let raw_before = m.read32(base + GPU_INT_RAWSTAT);
    m.write32(base + GPU_INT_CLEAR, IRQ_RESET_COMPLETED);
    let raw_cleared = m.read32(base + GPU_INT_RAWSTAT);
    if raw_cleared & IRQ_RESET_COMPLETED != 0 {
        return Err((MaliRefusal::ResetBitStuck, raw_cleared));
    }
    m.write32(base + GPU_CMD, CMD_SOFT_RESET);
    for t in 0..=RESET_WAIT_US {
        if m.read32(base + GPU_INT_RAWSTAT) & IRQ_RESET_COMPLETED != 0 {
            m.write32(base + GPU_INT_CLEAR, IRQ_RESET_COMPLETED);
            return Ok(Reset { raw_before, raw_cleared, waited_us: t });
        }
        delay_us(1);
    }
    Err((MaliRefusal::ResetTimeout, m.read32(base + GPU_INT_RAWSTAT)))
}

/// What [`soft_reset`] saw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reset {
    /// `GPU_INT_RAWSTAT` before the clear and after it.
    pub raw_before: u32,
    pub raw_cleared: u32,
    /// Polls (each followed by a 1 µs wait) before `RESET_COMPLETED` rose.
    pub waited_us: u32,
}

/// What [`l2_power_on`] saw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct L2On {
    pub mask: u64,
    /// `L2_READY` before the `L2_PWRON` write, and after the wait.
    pub ready_before: u64,
    pub ready: u64,
    /// Microseconds of the transition wait and of the ready wait.
    pub waited_us: [u32; 2],
}

/// The coherency mode, then the L2 on (panthor_gpu.c:226-248 and
/// `panthor_gpu_block_power_on`, :190-218): with only one L2 (the
/// G610's `l2_present` 1), its mask is 1; wait while `L2_PWRTRANS` has it,
/// write `L2_PWRON`, wait until `L2_READY` has it.
///
/// One difference: panthor's ready condition is `(mask & val) == val`
/// (:210), which an `L2_READY` of 0 meets at once; this waits for
/// `ready & mask == mask`, the L2 actually reported ready.
pub fn l2_power_on(m: &impl Mmio, base: usize, delay_us: &mut impl FnMut(u32)) -> Result<L2On, (MaliRefusal, u64)> {
    let present = read64(m, base + GPU_L2_PRESENT);
    if present == 0 {
        return Err((MaliRefusal::NoL2, 0));
    }
    // panthor powers the first core group only (:229-243); the G610 has one
    let mask = present & present.wrapping_neg();
    m.write32(base + GPU_COHERENCY_PROTOCOL, COHERENCY_NONE);
    let mut out = L2On { mask, ..L2On::default() };
    out.waited_us[0] = wait64(m, base + L2_PWRTRANS, L2_WAIT_US, delay_us, |v| v & mask == 0)
        .ok_or((MaliRefusal::L2TransTimeout, read64(m, base + L2_PWRTRANS)))?;
    out.ready_before = read64(m, base + L2_READY);
    m.write32(base + L2_PWRON, mask as u32);
    m.write32(base + L2_PWRON + 4, (mask >> 32) as u32);
    out.waited_us[1] = wait64(m, base + L2_READY, L2_WAIT_US, delay_us, |v| v & mask == mask)
        .ok_or((MaliRefusal::L2ReadyTimeout, read64(m, base + L2_READY)))?;
    out.ready = read64(m, base + L2_READY);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    const BASE: usize = 0xfb00_0000;

    /// The G610 as panthor's probe line describes it; a soft reset completes
    /// after `lag` polls, the L2 is ready `lag` polls after its power-on.
    struct G610 {
        regs: RefCell<HashMap<usize, u32>>,
        writes: RefCell<Vec<(usize, u32)>>,
        lag: u32,
        polls: RefCell<u32>,
        reset_pending: RefCell<bool>,
        l2_on: RefCell<bool>,
        /// The L2 never becomes ready.
        stuck_l2: bool,
        /// `GPU_INT_CLEAR` does not clear.
        sticky_raw: bool,
    }

    impl G610 {
        fn new(lag: u32) -> G610 {
            let r = [
                (GPU_ID, G610_GPU_ID),
                (GPU_L2_FEATURES, 0x0712_0306),
                (GPU_TILER_FEATURES, 0x809),
                (GPU_MEM_FEATURES, 0x301),
                (GPU_MMU_FEATURES, 0x2830),
                (GPU_AS_PRESENT, 0xff),
                (GPU_SHADER_PRESENT, 0x50005),
                (GPU_TILER_PRESENT, 1),
                (GPU_L2_PRESENT, 1),
            ];
            G610 {
                regs: RefCell::new(r.into_iter().collect()),
                writes: RefCell::new(vec![]),
                lag,
                polls: RefCell::new(0),
                reset_pending: RefCell::new(false),
                l2_on: RefCell::new(false),
                stuck_l2: false,
                sticky_raw: false,
            }
        }
    }

    impl Mmio for G610 {
        fn read32(&self, addr: usize) -> u32 {
            let off = addr - BASE;
            let mut p = self.polls.borrow_mut();
            *p += 1;
            match off {
                GPU_INT_RAWSTAT => {
                    if *self.reset_pending.borrow() && *p > self.lag {
                        *self.reset_pending.borrow_mut() = false;
                        let mut r = self.regs.borrow_mut();
                        *r.entry(GPU_INT_RAWSTAT).or_insert(0) |= IRQ_RESET_COMPLETED;
                    }
                }
                L2_READY if *self.l2_on.borrow() && *p > self.lag && !self.stuck_l2 => {
                    self.regs.borrow_mut().insert(L2_READY, 1);
                }
                _ => {}
            }
            *self.regs.borrow().get(&off).unwrap_or(&0)
        }
        fn write32(&self, addr: usize, val: u32) {
            let off = addr - BASE;
            self.writes.borrow_mut().push((off, val));
            *self.polls.borrow_mut() = 0;
            match off {
                GPU_INT_CLEAR if self.sticky_raw => {}
                GPU_INT_CLEAR => {
                    *self.regs.borrow_mut().entry(GPU_INT_RAWSTAT).or_insert(0) &= !val;
                }
                GPU_CMD if val == CMD_SOFT_RESET => {
                    *self.reset_pending.borrow_mut() = true;
                    self.regs.borrow_mut().insert(L2_READY, 0);
                    *self.l2_on.borrow_mut() = false;
                }
                L2_PWRON if val & 1 != 0 => *self.l2_on.borrow_mut() = true,
                _ => {
                    self.regs.borrow_mut().insert(off, val);
                }
            }
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!()
        }
        fn write8(&self, _: usize, _: u8) {
            unreachable!()
        }
    }

    #[test]
    fn ident_reads_what_panthor_printed() {
        let g = G610::new(0);
        let i = ident(&g, BASE);
        assert_eq!(i.gpu_id, 0xa867_0005);
        assert_eq!((i.gpu_id >> 16, (i.gpu_id >> 12) & 0xf, (i.gpu_id >> 4) & 0xff, i.gpu_id & 0xf), (0xa867, 0, 0, 5));
        assert_eq!(
            (i.l2_features, i.tiler_features, i.mem_features, i.mmu_features, i.as_present),
            (0x7120306, 0x809, 0x301, 0x2830, 0xff)
        );
        assert_eq!((i.shader_present, i.l2_present, i.tiler_present), (0x50005, 1, 1));
        assert!(g.writes.borrow().is_empty());
    }

    #[test]
    fn soft_reset_clears_then_commands_then_acknowledges() {
        let g = G610::new(4);
        assert_eq!(soft_reset(&g, BASE, &mut |_| {}), Ok(Reset { raw_before: 0, raw_cleared: 0, waited_us: 4 }));
        assert_eq!(*g.writes.borrow(), vec![(GPU_INT_CLEAR, 1 << 8), (GPU_CMD, 0x101), (GPU_INT_CLEAR, 1 << 8)]);
    }

    #[test]
    fn a_completion_that_cannot_be_cleared_is_refused_before_the_command() {
        let mut g = G610::new(0);
        g.sticky_raw = true;
        g.regs.borrow_mut().insert(GPU_INT_RAWSTAT, IRQ_RESET_COMPLETED);
        assert_eq!(soft_reset(&g, BASE, &mut |_| {}), Err((MaliRefusal::ResetBitStuck, IRQ_RESET_COMPLETED)));
        assert!(g.writes.borrow().iter().all(|w| w.0 != GPU_CMD));
    }

    #[test]
    fn a_reset_that_never_completes_times_out_after_100_ms() {
        let g = G610::new(u32::MAX);
        let mut slept = 0u64;
        assert_eq!(soft_reset(&g, BASE, &mut |us| slept += u64::from(us)), Err((MaliRefusal::ResetTimeout, 0)));
        assert_eq!(slept, u64::from(RESET_WAIT_US) + 1);
    }

    #[test]
    fn l2_power_on_sets_no_coherency_then_powers_the_one_l2() {
        let g = G610::new(3);
        let on = l2_power_on(&g, BASE, &mut |_| {}).unwrap();
        // 64-bit polls: two reads each, so a lag of 3 reads is two polls
        assert_eq!(on, L2On { mask: 1, ready_before: 0, ready: 1, waited_us: [0, 2] });
        assert_eq!(*g.writes.borrow(), vec![(GPU_COHERENCY_PROTOCOL, 31), (L2_PWRON, 1), (L2_PWRON + 4, 0)]);
    }

    #[test]
    fn an_l2_ready_of_0_is_not_taken_as_ready() {
        let mut g = G610::new(0);
        g.stuck_l2 = true;
        assert_eq!(l2_power_on(&g, BASE, &mut |_| {}), Err((MaliRefusal::L2ReadyTimeout, 0)));
        let g = G610::new(0);
        g.regs.borrow_mut().insert(GPU_L2_PRESENT, 0);
        assert_eq!(l2_power_on(&g, BASE, &mut |_| {}), Err((MaliRefusal::NoL2, 0)));
        assert!(g.writes.borrow().is_empty());
    }
}
