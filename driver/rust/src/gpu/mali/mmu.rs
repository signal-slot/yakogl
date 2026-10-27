// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2019 Linaro, Ltd, Rob Herring
// SPDX-FileCopyrightText: 2023 Collabora ltd.
//! A Mali address space's registers and their sequences (Z2,
//! driver/docs/nanopc-t6-mali.md "Z2: AS0 and a cache flush"): enable an
//! address space on its tables, and the lock / cache flush / unlock that
//! makes a change of the tables visible to the GPU.
//!
//! Source: Linux v6.18's panthor under the MIT option of its `GPL-2.0 or
//! MIT` licence (the design page's decision 1): the register offsets and
//! fields from drivers/gpu/drm/panthor/panthor_regs.h, the sequences and
//! `mair_to_memattr` from panthor_mmu.c (Copyright 2019 Linaro, Ltd, Rob
//! Herring; 2023 Collabora ltd.), each named by its function. Upstream-
//! derived items keep panthor's names and order.
//!
//! Two differences, each said where it is: a command is followed by a
//! wait for `AS_ACTIVE` to clear (panthor waits before the next command
//! instead), and the cache flush polls `GPU_INT_RAWSTAT`
//! ([`super::flush_caches`]).

use super::{flush_caches, Flush, MaliRefusal, CACHE_CLEAN, CACHE_INV};
use crate::mmio::Mmio;

/// The MMU interrupt block (panthor_regs.h `MMU_INT_*`): bit n is a fault
/// of AS n, bit 16 + n a bus error of AS n
/// (`panthor_mmu_as_fault_mask`).
pub const MMU_INT_RAWSTAT: usize = 0x2000;
pub const MMU_INT_CLEAR: usize = 0x2004;
pub const MMU_INT_MASK: usize = 0x2008;
pub const MMU_INT_STAT: usize = 0x200c;

/// `MMU_AS(as)` = `MMU_BASE` (0x2400) + `as << MMU_AS_SHIFT` (6).
pub const fn mmu_as(as_nr: u32) -> usize {
    0x2400 + ((as_nr as usize) << 6)
}
/// Offsets in an address space's block (panthor_regs.h `AS_*`).
pub const AS_TRANSTAB: usize = 0x00;
pub const AS_MEMATTR: usize = 0x08;
pub const AS_LOCKADDR: usize = 0x10;
pub const AS_COMMAND: usize = 0x18;
pub const AS_FAULTSTATUS: usize = 0x1c;
pub const AS_FAULTADDRESS: usize = 0x20;
pub const AS_STATUS: usize = 0x28;
pub const AS_TRANSCFG: usize = 0x30;
pub const AS_FAULTEXTRA: usize = 0x38;

/// `AS_COMMAND_*`.
pub const AS_COMMAND_NOP: u32 = 0;
pub const AS_COMMAND_UPDATE: u32 = 1;
pub const AS_COMMAND_LOCK: u32 = 2;
pub const AS_COMMAND_UNLOCK: u32 = 3;
pub const AS_COMMAND_FLUSH_PT: u32 = 4;
pub const AS_COMMAND_FLUSH_MEM: u32 = 5;
/// `AS_LOCK_REGION_MIN_SIZE`: 32 KiB.
pub const AS_LOCK_REGION_MIN_SIZE: u64 = 1 << 15;
/// `AS_STATUS_AS_ACTIVE`.
pub const AS_STATUS_AS_ACTIVE: u32 = 1 << 0;

/// `AS_TRANSCFG` fields.
pub const AS_TRANSCFG_ADRMODE_UNMAPPED: u64 = 1;
pub const AS_TRANSCFG_ADRMODE_AARCH64_4K: u64 = 6;
pub const fn as_transcfg_ina_bits(x: u64) -> u64 {
    x << 6
}
pub const AS_TRANSCFG_PTW_MEMATTR_WB: u64 = 2 << 24;
pub const AS_TRANSCFG_PTW_SH_OS: u64 = 2 << 28;
pub const AS_TRANSCFG_PTW_RA: u64 = 1 << 30;

/// `AS_MEMATTR` fields, per byte.
pub const fn as_memattr_aarch64_inner_alloc_expl(w: bool, r: bool) -> u8 {
    (3 << 2) | if w { 1 } else { 0 } | if r { 2 } else { 0 }
}
pub const AS_MEMATTR_AARCH64_SH_MIDGARD_INNER: u8 = 0 << 4;
pub const AS_MEMATTR_AARCH64_SH_CPU_INNER: u8 = 1 << 4;
pub const AS_MEMATTR_AARCH64_INNER_OUTER_NC: u8 = 1 << 6;
pub const AS_MEMATTR_AARCH64_INNER_OUTER_WB: u8 = 2 << 6;

/// panthor's `AS_STATUS` wait: a read every 10 µs for 100 ms
/// (`wait_ready`, `gpu_read_relaxed_poll_timeout_atomic(..., 10, 100000)`).
pub const READY_POLL_US: u32 = 10;
pub const READY_WAIT_US: u32 = 100_000;

/// `AS_TRANSCFG` for a VM of `va_bits` (`panthor_vm_active`): page-table
/// walks write-back cached with read allocate, the AArch64 4 KiB mode, the
/// input size as `55 - va_bits`; outer shareable walks only when coherent.
pub const fn transcfg(va_bits: u32, coherent: bool) -> u64 {
    let mut t = AS_TRANSCFG_PTW_MEMATTR_WB
        | AS_TRANSCFG_PTW_RA
        | AS_TRANSCFG_ADRMODE_AARCH64_4K
        | as_transcfg_ina_bits(55 - va_bits as u64);
    if coherent {
        t |= AS_TRANSCFG_PTW_SH_OS;
    }
    t
}

/// `AS_MEMATTR` from a stage-1 `MAIR` (`mair_to_memattr`, transcribed):
/// a byte is write-back only when its inner and outer policies both are,
/// otherwise non-cacheable; write-back takes the inner allocation hints,
/// and is "Midgard inner" shareable when the GPU is not coherent.
pub const fn mair_to_memattr(mair: u64, coherent: bool) -> u64 {
    let mut memattr = 0u64;
    let mut i = 0;
    while i < 8 {
        let in_attr = (mair >> (8 * i)) as u8;
        let outer = in_attr >> 4;
        let inner = in_attr & 0xf;
        let out_attr = if (outer & 3) == 0 || (outer & 4) == 0 || (inner & 4) == 0 {
            AS_MEMATTR_AARCH64_INNER_OUTER_NC
                | AS_MEMATTR_AARCH64_SH_MIDGARD_INNER
                | as_memattr_aarch64_inner_alloc_expl(false, false)
        } else {
            let a =
                AS_MEMATTR_AARCH64_INNER_OUTER_WB | as_memattr_aarch64_inner_alloc_expl(inner & 1 != 0, inner & 2 != 0);
            if !coherent {
                a | AS_MEMATTR_AARCH64_SH_MIDGARD_INNER
            } else {
                a | AS_MEMATTR_AARCH64_SH_CPU_INNER
            }
        };
        memattr |= (out_attr as u64) << (8 * i);
        i += 1;
    }
    memattr
}

/// The `AS_LOCKADDR` value for `[start, start + size)` (`lock_region`):
/// the naturally aligned power-of-two block that holds the range, at least
/// [`AS_LOCK_REGION_MIN_SIZE`], as its base with `log2(width) - 1` in the
/// low bits. `None` for a size of 0 (panthor then locks nothing).
pub const fn lock_region(start: u64, size: u64) -> Option<u64> {
    if size == 0 {
        return None;
    }
    let end = start.wrapping_add(size);
    let differ = start ^ end.wrapping_sub(1);
    let fls = 64 - differ.leading_zeros();
    let min = AS_LOCK_REGION_MIN_SIZE.trailing_zeros();
    let width = if fls > min { fls } else { min } - 1;
    let base = start & !((1u64 << width) - 1);
    Some(base | width as u64)
}

fn write64(m: &impl Mmio, addr: usize, v: u64) {
    m.write32(addr, v as u32);
    m.write32(addr + 4, (v >> 32) as u32);
}

fn read64(m: &impl Mmio, addr: usize) -> u64 {
    u64::from(m.read32(addr)) | u64::from(m.read32(addr + 4)) << 32
}

/// One address space's registers, read with no write.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AsRegs {
    pub transtab: u64,
    pub memattr: u64,
    pub transcfg: u64,
    pub lockaddr: u64,
    pub status: u32,
    pub faultstatus: u32,
    pub faultaddress: u64,
}

impl AsRegs {
    pub fn read(m: &impl Mmio, base: usize, as_nr: u32) -> AsRegs {
        let a = base + mmu_as(as_nr);
        AsRegs {
            transtab: read64(m, a + AS_TRANSTAB),
            memattr: read64(m, a + AS_MEMATTR),
            transcfg: read64(m, a + AS_TRANSCFG),
            lockaddr: read64(m, a + AS_LOCKADDR),
            status: m.read32(a + AS_STATUS),
            faultstatus: m.read32(a + AS_FAULTSTATUS),
            faultaddress: read64(m, a + AS_FAULTADDRESS),
        }
    }
}

/// `wait_ready`: wait for `AS_STATUS.AS_ACTIVE` to clear; the polls it took.
pub fn wait_ready(
    m: &impl Mmio,
    base: usize,
    as_nr: u32,
    delay_us: &mut impl FnMut(u32),
) -> Result<u32, (MaliRefusal, u32)> {
    let a = base + mmu_as(as_nr) + AS_STATUS;
    for t in 0..=READY_WAIT_US / READY_POLL_US {
        if m.read32(a) & AS_STATUS_AS_ACTIVE == 0 {
            return Ok(t);
        }
        delay_us(READY_POLL_US);
    }
    Err((MaliRefusal::AsActiveStuck, m.read32(a)))
}

/// `write_cmd`: wait until the address space takes a command, write it,
/// then (our addition) wait for it to finish. The polls before and after.
pub fn write_cmd(
    m: &impl Mmio,
    base: usize,
    as_nr: u32,
    cmd: u32,
    delay_us: &mut impl FnMut(u32),
) -> Result<[u32; 2], (MaliRefusal, u32)> {
    let before = wait_ready(m, base, as_nr, delay_us)?;
    m.write32(base + mmu_as(as_nr) + AS_COMMAND, cmd);
    let after = wait_ready(m, base, as_nr, delay_us)?;
    Ok([before, after])
}

/// Which operation [`hw_do_operation`] stands for: it decides what the LSC
/// flush does (`mmu_hw_do_operation_locked`); the L2 is always cleaned and
/// invalidated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlushOp {
    /// `AS_COMMAND_FLUSH_MEM`: the LSC too, cleaned and invalidated.
    Mem,
    /// `AS_COMMAND_FLUSH_PT`: the L2 only (after a change of the tables).
    Pt,
}

/// What [`hw_do_operation`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Op {
    /// The `AS_LOCKADDR` written.
    pub lockaddr: u64,
    /// Polls of the LOCK command and of the UNLOCK command (before, after).
    pub lock_polls: [u32; 2],
    pub flush: Flush,
    pub unlock_polls: [u32; 2],
}

/// `mmu_hw_do_operation_locked`: lock the region (`lock_region`, then
/// `AS_COMMAND_LOCK`), wait, flush the caches through `GPU_CMD`, then
/// `AS_COMMAND_UNLOCK` (the region is not unlocked by a `GPU_CMD` flush, as
/// panthor's comment there says) and wait.
pub fn hw_do_operation(
    m: &impl Mmio,
    base: usize,
    as_nr: u32,
    iova: u64,
    size: u64,
    op: FlushOp,
    delay_us: &mut impl FnMut(u32),
) -> Result<Op, (MaliRefusal, u32)> {
    let l2 = CACHE_CLEAN | CACHE_INV;
    let lsc = match op {
        FlushOp::Mem => CACHE_CLEAN | CACHE_INV,
        FlushOp::Pt => 0,
    };
    let mut o = Op::default();
    if let Some(region) = lock_region(iova, size) {
        o.lockaddr = region;
        write64(m, base + mmu_as(as_nr) + AS_LOCKADDR, region);
        o.lock_polls = write_cmd(m, base, as_nr, AS_COMMAND_LOCK, delay_us)?;
    }
    o.flush = flush_caches(m, base, l2, lsc, 0, delay_us)?;
    o.unlock_polls = write_cmd(m, base, as_nr, AS_COMMAND_UNLOCK, delay_us)?;
    Ok(o)
}

/// What [`as_enable`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AsEnable {
    /// The whole-VA flush before the switch.
    pub flush: Op,
    pub transtab: u64,
    pub memattr: u64,
    pub transcfg: u64,
    /// Polls of the UPDATE command (before, after).
    pub update_polls: [u32; 2],
}

/// `panthor_mmu_as_enable`: flush the whole VA range (`FLUSH_MEM` over 0,
/// `~0`), write `AS_TRANSTAB`, `AS_MEMATTR`, `AS_TRANSCFG`, then
/// `AS_COMMAND_UPDATE`.
pub fn as_enable(
    m: &impl Mmio,
    base: usize,
    as_nr: u32,
    transtab: u64,
    transcfg: u64,
    memattr: u64,
    delay_us: &mut impl FnMut(u32),
) -> Result<AsEnable, (MaliRefusal, u32)> {
    let flush = hw_do_operation(m, base, as_nr, 0, !0, FlushOp::Mem, delay_us)?;
    let a = base + mmu_as(as_nr);
    write64(m, a + AS_TRANSTAB, transtab);
    write64(m, a + AS_MEMATTR, memattr);
    write64(m, a + AS_TRANSCFG, transcfg);
    let update_polls = write_cmd(m, base, as_nr, AS_COMMAND_UPDATE, delay_us)?;
    Ok(AsEnable { flush, transtab, memattr, transcfg, update_polls })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::{GPU_CMD, GPU_INT_CLEAR, GPU_INT_RAWSTAT, IRQ_CLEAN_CACHES_COMPLETED};
    use super::*;
    use crate::gpu::mali::pgtable::MAIR;
    use std::cell::RefCell;
    use std::collections::HashMap;

    pub const BASE: usize = 0xfb00_0000;

    /// The G610's GPU control and MMU blocks for the Z2 sequence: an
    /// `AS_COMMAND` keeps `AS_ACTIVE` set for `lag` reads of `AS_STATUS`, a
    /// cache flush raises `CLEAN_CACHES_COMPLETED` after `lag` reads of
    /// `GPU_INT_RAWSTAT`; a command while `AS_ACTIVE` is set is a test
    /// failure, as is a flush while the last one is still pending.
    pub struct Gpu {
        pub regs: RefCell<HashMap<usize, u32>>,
        pub writes: RefCell<Vec<(usize, u32)>>,
        pub lag: u32,
        /// AS number and reads left while it is active.
        active: RefCell<[u32; 8]>,
        flush_left: RefCell<Option<u32>>,
        /// `AS_ACTIVE` never clears.
        pub stuck: bool,
        /// What UPDATE latched: the TRANSTAB, MEMATTR, TRANSCFG of AS 0.
        pub latched: RefCell<Option<(u64, u64, u64)>>,
    }

    impl Gpu {
        pub fn new(lag: u32) -> Gpu {
            Gpu {
                regs: RefCell::new(HashMap::new()),
                writes: RefCell::new(vec![]),
                lag,
                active: RefCell::new([0; 8]),
                flush_left: RefCell::new(None),
                stuck: false,
                latched: RefCell::new(None),
            }
        }
        fn r64(&self, off: usize) -> u64 {
            let r = self.regs.borrow();
            u64::from(*r.get(&off).unwrap_or(&0)) | u64::from(*r.get(&(off + 4)).unwrap_or(&0)) << 32
        }
        /// The register writes, without the `GPU_INT_CLEAR` ones.
        pub fn sequence(&self) -> Vec<(usize, u32)> {
            self.writes.borrow().iter().copied().filter(|w| w.0 != GPU_INT_CLEAR).collect()
        }
    }

    impl Mmio for Gpu {
        fn read32(&self, addr: usize) -> u32 {
            let off = addr - BASE;
            if (0x2400..0x2600).contains(&off) && (off - 0x2400) % 0x40 == AS_STATUS {
                let n = (off - 0x2400) / 0x40;
                let mut a = self.active.borrow_mut();
                if a[n] > 0 {
                    if !self.stuck {
                        a[n] -= 1;
                    }
                    return AS_STATUS_AS_ACTIVE;
                }
                return 0;
            }
            if off == GPU_INT_RAWSTAT {
                let mut f = self.flush_left.borrow_mut();
                match *f {
                    Some(0) => {
                        *f = None;
                        *self.regs.borrow_mut().entry(GPU_INT_RAWSTAT).or_insert(0) |= IRQ_CLEAN_CACHES_COMPLETED;
                    }
                    Some(n) => *f = Some(n - 1),
                    None => {}
                }
            }
            *self.regs.borrow().get(&off).unwrap_or(&0)
        }
        fn write32(&self, addr: usize, val: u32) {
            let off = addr - BASE;
            self.writes.borrow_mut().push((off, val));
            match off {
                GPU_INT_CLEAR => {
                    *self.regs.borrow_mut().entry(GPU_INT_RAWSTAT).or_insert(0) &= !val;
                }
                GPU_CMD => {
                    assert_eq!(val & 0xff, 4, "only cache flushes here");
                    assert!(self.flush_left.borrow().is_none(), "a flush while one is pending");
                    *self.flush_left.borrow_mut() = Some(self.lag);
                }
                _ if (0x2400..0x2600).contains(&off) && (off - 0x2400) % 0x40 == AS_COMMAND => {
                    let n = (off - 0x2400) / 0x40;
                    assert_eq!(self.active.borrow()[n], 0, "AS_COMMAND {val} while AS {n} is active");
                    if val == AS_COMMAND_UPDATE && n == 0 {
                        let a = mmu_as(0);
                        *self.latched.borrow_mut() =
                            Some((self.r64(a + AS_TRANSTAB), self.r64(a + AS_MEMATTR), self.r64(a + AS_TRANSCFG)));
                    }
                    self.active.borrow_mut()[n] = self.lag;
                }
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
    fn the_non_coherent_t6_values_are_the_design_page_s() {
        // MMU_FEATURES 0x2830: 48 bits of VA
        assert_eq!(transcfg(48, false), 0x4200_01c6);
        assert_eq!(transcfg(48, true), 0x6200_01c6);
        assert_eq!(MAIR, 0xf404_ff44);
        assert_eq!(mair_to_memattr(MAIR, false), 0x4c4c_4c4c_8c4c_8f4c);
        // coherent: the write-back bytes are CPU-inner shareable
        assert_eq!(mair_to_memattr(MAIR, true), 0x4c4c_4c4c_9c4c_9f4c);
        assert_eq!(mmu_as(0), 0x2400);
        assert_eq!(mmu_as(7), 0x25c0);
    }

    #[test]
    fn lock_regions_are_naturally_aligned_powers_of_two() {
        // the whole VA, as panthor_mmu_as_enable asks
        assert_eq!(lock_region(0, !0), Some(63));
        // 64 KiB at 0x04000000: one 64 KiB block, log2 - 1 = 15
        assert_eq!(lock_region(0x0400_0000, 0x1_0000), Some(0x0400_000f));
        // one page: the 32 KiB minimum
        assert_eq!(lock_region(0x0400_1000, 0x1000), Some(0x0400_000e));
        // a range across a 64 KiB line takes the block holding both sides
        assert_eq!(lock_region(0x0400_f000, 0x2000), Some(0x0400_0010));
        assert_eq!(lock_region(0x1234_5000, 0), None);
    }

    /// The address space's offset of a write.
    fn as0(off: usize) -> usize {
        mmu_as(0) + off
    }

    #[test]
    fn as_enable_is_panthor_s_sequence() {
        let g = Gpu::new(2);
        let e = as_enable(&g, BASE, 0, 0x1040_0000, transcfg(48, false), mair_to_memattr(MAIR, false), &mut |_| {})
            .unwrap();
        assert_eq!(
            g.sequence(),
            vec![
                (as0(AS_LOCKADDR), 63),
                (as0(AS_LOCKADDR) + 4, 0),
                (as0(AS_COMMAND), AS_COMMAND_LOCK),
                (GPU_CMD, 0x3304),
                (as0(AS_COMMAND), AS_COMMAND_UNLOCK),
                (as0(AS_TRANSTAB), 0x1040_0000),
                (as0(AS_TRANSTAB) + 4, 0),
                (as0(AS_MEMATTR), 0x8c4c_8f4c),
                (as0(AS_MEMATTR) + 4, 0x4c4c_4c4c),
                (as0(AS_TRANSCFG), 0x4200_01c6),
                (as0(AS_TRANSCFG) + 4, 0),
                (as0(AS_COMMAND), AS_COMMAND_UPDATE),
            ]
        );
        assert_eq!(*g.latched.borrow(), Some((0x1040_0000, 0x4c4c_4c4c_8c4c_8f4c, 0x4200_01c6)));
        // each command waited out its AS_ACTIVE: lag reads report it set
        assert_eq!((e.flush.lock_polls, e.flush.unlock_polls, e.update_polls), ([0, 2], [0, 2], [0, 2]));
        assert_eq!(e.flush.flush.cmd, 0x3304);
        assert_eq!(g.read32(BASE + GPU_INT_RAWSTAT), 0, "the flush's completion is cleared");
    }

    #[test]
    fn a_flush_after_a_map_locks_its_range_and_spares_the_lsc() {
        let g = Gpu::new(0);
        let o = hw_do_operation(&g, BASE, 0, 0x0400_0000, 0x1_0000, FlushOp::Pt, &mut |_| {}).unwrap();
        assert_eq!(o.lockaddr, 0x0400_000f);
        assert_eq!(
            g.sequence(),
            vec![
                (as0(AS_LOCKADDR), 0x0400_000f),
                (as0(AS_LOCKADDR) + 4, 0),
                (as0(AS_COMMAND), AS_COMMAND_LOCK),
                (GPU_CMD, 0x0304),
                (as0(AS_COMMAND), AS_COMMAND_UNLOCK),
            ]
        );
    }

    #[test]
    fn an_address_space_that_stays_active_is_refused_before_its_next_command() {
        let mut g = Gpu::new(1);
        g.stuck = true;
        let mut slept = 0u64;
        let r = as_enable(&g, BASE, 0, 0x1040_0000, 0x4200_01c6, 0, &mut |us| slept += u64::from(us));
        assert_eq!(r, Err((MaliRefusal::AsActiveStuck, AS_STATUS_AS_ACTIVE)));
        // the LOCK went out; nothing after it
        assert_eq!(g.sequence().last(), Some(&(as0(AS_COMMAND), AS_COMMAND_LOCK)));
        assert_eq!(slept, u64::from(READY_WAIT_US) + u64::from(READY_POLL_US));
        assert!(g.latched.borrow().is_none());
    }

    #[test]
    fn as_regs_read_writes_nothing() {
        let g = Gpu::new(0);
        g.regs.borrow_mut().insert(as0(AS_TRANSCFG), AS_TRANSCFG_ADRMODE_UNMAPPED as u32);
        let r = AsRegs::read(&g, BASE, 0);
        assert_eq!(r.transcfg, 1);
        assert!(g.writes.borrow().is_empty());
    }
}
