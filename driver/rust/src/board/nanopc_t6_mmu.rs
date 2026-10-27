// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Z2 on the NanoPC-T6 (driver/docs/nanopc-t6-mali.md, "Z2: AS0 and a
//! cache flush"), after Z1's power-on ([`super::nanopc_t6_gpu::gpu_up`]):
//! read AS0 and the interrupt state as the reset left them, one cache flush
//! round trip on its own, AS0's tables built in the image's pool with the
//! firmware's shared-interface section mapped, a CPU walk of a known VA,
//! the tables cleaned to the point of coherency, AS0 enabled as panthor
//! enables the MCU's VM, its registers read back, and the flush panthor
//! does after a map. Every value is kept in [`MmuUp`]; the first step that
//! refuses ends the run there.
//!
//! Nothing here makes the GPU walk the tables: without the MCU running no
//! GPU agent reads through AS0. The first walk is the MCU's first fetch at
//! Z3; a wrong table shows there as an AS0 fault.

use crate::gpu::mali::mmu::{self, AsEnable, AsRegs, FlushOp, Op};
use crate::gpu::mali::pgtable::{self, Prot, Table, Tables, Walk};
use crate::gpu::mali::{self, Flush, GPU_INT_MASK, GPU_INT_RAWSTAT};
use crate::mmio::Mmio;

/// The MCU's shared-interface section: 64 KiB at MCU VA 0x04000000,
/// writable, cached (the board's `mali_csffw.bin`; the design page's "The
/// firmware (Z3)"). Mapped as panthor maps a writable, cached section
/// without the execute flag.
pub const FW_SHARED_VA: u64 = 0x0400_0000;
pub const FW_SHARED_SIZE: u64 = 0x1_0000;
pub const FW_SHARED_PROT: Prot = Prot { write: true, exec: false, cached: true };
/// The VA the run walks: page 1 of the section, offset 0x234.
pub const PROBE_VA: u64 = FW_SHARED_VA + 0x1234;
/// The address space: AS0 is the MCU's (panthor_mmu.c `panthor_vm_active`:
/// the MCU VM always takes slot 0).
pub const AS_MCU: u32 = 0;
/// The `AS_MEMATTR` the run writes: panthor's live word on the board (AS0
/// and AS1, the design page's "MMU (Z2)"). Bytes 0-3, the attributes a PTE
/// selects, are [`mmu::mair_to_memattr`] over io-pgtable's `MAIR` for the
/// non-coherent case; bytes 4-7 no PTE selects.
pub const MEMATTR_LIVE: u64 = 0x8f8f_8f8f_8c4c_8f4c;

/// Which step stopped the run. The value crosses to C (`YD_T6_MMU_STOP_*`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum Stop {
    /// Every step ran and every read-back is what was written.
    #[default]
    None = 0,
    /// `MMU_FEATURES` does not give 48 bits of VA and 40 of PA (`detail`).
    Features = 1,
    /// The cache flush round trip did not complete (`refusal`: the
    /// [`mali::MaliRefusal`]).
    Flush = 2,
    /// The tables could not be built (`refusal`: the [`pgtable::PtRefusal`]).
    Tables = 3,
    /// The CPU walk of [`PROBE_VA`] does not land on the backing.
    Walk = 4,
    /// The cache clean of the tables failed (`detail`: its rc).
    Clean = 5,
    /// The AS0 enable sequence stopped (`refusal`: the [`mali::MaliRefusal`]).
    Enable = 6,
    /// AS0 reads back other than written (`refusal` 1 `TRANSTAB`, 2
    /// `MEMATTR`, 3 `TRANSCFG`; `detail` the value read).
    ReadBack = 7,
    /// AS0 is still active, or reports a fault, or `MMU_INT_RAWSTAT` has a
    /// bit (`refusal` 1, 2, 3; `detail` the value).
    Fault = 8,
    /// The flush after the map stopped (`refusal`: the [`mali::MaliRefusal`]).
    FlushPt = 9,
}

/// Everything the run read, in order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MmuUp {
    /// AS0 as found, and `MMU_INT_RAWSTAT`, `MMU_INT_MASK`,
    /// `GPU_INT_RAWSTAT`, `GPU_INT_MASK`.
    pub found: AsRegs,
    pub found_irq: [u32; 4],
    /// The flush round trip on its own (L2 and LSC cleaned and invalidated).
    pub flush: Flush,
    pub tables_pa: u64,
    pub backing_pa: u64,
    pub tables_used: u32,
    pub walk: Walk,
    pub clean_rc: i32,
    pub enable: AsEnable,
    pub after: AsRegs,
    pub after_irq: [u32; 4],
    pub flush_pt: Op,
    pub stop: Stop,
    pub refusal: u32,
    pub detail: u64,
}

impl MmuUp {
    fn stopped(&mut self, stop: Stop, refusal: u32, detail: u64) {
        self.stop = stop;
        self.refusal = refusal;
        self.detail = detail;
    }
}

fn irq(m: &impl Mmio, gpu: usize) -> [u32; 4] {
    [
        m.read32(gpu + mmu::MMU_INT_RAWSTAT),
        m.read32(gpu + mmu::MMU_INT_MASK),
        m.read32(gpu + GPU_INT_RAWSTAT),
        m.read32(gpu + GPU_INT_MASK),
    ]
}

/// The run (see the module). `gpu` is the GPU's mapping (at least 0x3000
/// bytes: the MMU block is at 0x2000); `mmu_features` is what Z1's
/// identity read; `pool` is the image's table pages at physical
/// `tables_pa`; `backing_pa` is the 64 KiB the section maps to. `clean`
/// cleans the data cache over `(address, length)` to the point of
/// coherency and returns 0 (the GPU's walk is not coherent with the CPU's
/// caches: the T6's GPU node has no `dma-coherent`). `delay_us` waits.
#[allow(clippy::too_many_arguments)]
pub fn mmu_up(
    m: &impl Mmio,
    gpu: usize,
    mmu_features: u32,
    pool: &mut [Table],
    tables_pa: u64,
    backing_pa: u64,
    clean: &mut impl FnMut(usize, usize) -> i32,
    delay_us: &mut impl FnMut(u32),
) -> MmuUp {
    let mut o = MmuUp { tables_pa, backing_pa, ..MmuUp::default() };
    o.found = AsRegs::read(m, gpu, AS_MCU);
    o.found_irq = irq(m, gpu);
    if mmu_features & 0xff != pgtable::VA_BITS || (mmu_features >> 8) & 0xff < pgtable::PA_BITS {
        o.stopped(Stop::Features, 0, u64::from(mmu_features));
        return o;
    }

    let cln_inv = mali::CACHE_CLEAN | mali::CACHE_INV;
    match mali::flush_caches(m, gpu, cln_inv, cln_inv, 0, delay_us) {
        Ok(f) => o.flush = f,
        Err((why, raw)) => {
            o.stopped(Stop::Flush, why as u32, u64::from(raw));
            return o;
        }
    }

    let (root, used, walk, range) = {
        let mut t = match Tables::new(pool, tables_pa) {
            Ok(t) => t,
            Err(why) => {
                o.stopped(Stop::Tables, why as u32, tables_pa);
                return o;
            }
        };
        if let Err(why) = t.map(FW_SHARED_VA, backing_pa, FW_SHARED_SIZE, FW_SHARED_PROT) {
            o.tables_used = t.used() as u32;
            o.stopped(Stop::Tables, why as u32, FW_SHARED_VA);
            return o;
        }
        let used = t.used_pages();
        let range = (used.as_ptr() as usize, core::mem::size_of_val(used));
        (t.root_pa(), t.used() as u32, t.walk(PROBE_VA), range)
    };
    o.tables_used = used;
    o.walk = walk;
    if walk.pa != Some(backing_pa + (PROBE_VA - FW_SHARED_VA)) {
        o.stopped(Stop::Walk, walk.levels, walk.pa.unwrap_or(0));
        return o;
    }
    o.clean_rc = clean(range.0, range.1);
    if o.clean_rc != 0 {
        o.stopped(Stop::Clean, 0, o.clean_rc as u64);
        return o;
    }

    let va_bits = mmu_features & 0xff;
    let transcfg = mmu::transcfg(va_bits, false);
    let memattr = MEMATTR_LIVE;
    match mmu::as_enable(m, gpu, AS_MCU, root, transcfg, memattr, delay_us) {
        Ok(e) => o.enable = e,
        Err((why, v)) => {
            o.stopped(Stop::Enable, why as u32, u64::from(v));
            o.after = AsRegs::read(m, gpu, AS_MCU);
            o.after_irq = irq(m, gpu);
            return o;
        }
    }
    o.after = AsRegs::read(m, gpu, AS_MCU);
    o.after_irq = irq(m, gpu);
    for (i, (got, want)) in
        [(o.after.transtab, root), (o.after.memattr, memattr), (o.after.transcfg, transcfg)].into_iter().enumerate()
    {
        if got != want {
            o.stopped(Stop::ReadBack, i as u32 + 1, got);
            return o;
        }
    }
    for (i, v) in
        [o.after.status & mmu::AS_STATUS_AS_ACTIVE, o.after.faultstatus, o.after_irq[0]].into_iter().enumerate()
    {
        if v != 0 {
            o.stopped(Stop::Fault, i as u32 + 1, u64::from(v));
            return o;
        }
    }

    match mmu::hw_do_operation(m, gpu, AS_MCU, FW_SHARED_VA, FW_SHARED_SIZE, FlushOp::Pt, delay_us) {
        Ok(op) => o.flush_pt = op,
        Err((why, v)) => o.stopped(Stop::FlushPt, why as u32, u64::from(v)),
    }
    o
}

/// The PTE of [`PROBE_VA`]'s page for a backing at `backing_pa`: what the
/// design page predicts the walk prints.
pub const fn probe_pte(backing_pa: u64) -> u64 {
    pgtable::page_desc(backing_pa + 0x1000, FW_SHARED_PROT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::mali::mmu::tests::{Gpu, BASE};
    use crate::gpu::mali::mmu::{mmu_as, AS_COMMAND, AS_COMMAND_LOCK, AS_COMMAND_UNLOCK, AS_COMMAND_UPDATE};
    use crate::gpu::mali::GPU_CMD;
    use pgtable::{PtRefusal, ENTRIES};

    const TABLES_PA: u64 = 0x1040_0000;
    const BACKING_PA: u64 = 0x1050_0000;

    fn run(g: &Gpu, pool: &mut [Table], cleans: &mut Vec<(usize, usize)>) -> MmuUp {
        mmu_up(
            g,
            BASE,
            0x2830,
            pool,
            TABLES_PA,
            BACKING_PA,
            &mut |a, n| {
                cleans.push((a, n));
                0
            },
            &mut |_| {},
        )
    }

    #[test]
    fn from_a_reset_gpu_as0_comes_up_on_the_section_s_tables() {
        let g = Gpu::new(1);
        let mut pool = vec![[0u64; ENTRIES]; 8];
        let mut cleans = vec![];
        let o = run(&g, &mut pool, &mut cleans);
        assert_eq!((o.stop, o.refusal, o.detail), (Stop::None, 0, 0));
        assert_eq!(o.flush.cmd, 0x3304);
        assert_eq!(o.tables_used, 4);
        assert_eq!(o.walk.pa, Some(0x1050_1234));
        assert_eq!(o.walk.desc[3], 0x0060_0000_1050_1f47);
        assert_eq!(o.walk.desc[3], probe_pte(BACKING_PA));
        // the four used pages, and only them, cleaned before the enable
        assert_eq!(cleans, vec![(pool.as_ptr() as usize, 4 * 4096)]);
        assert_eq!(
            (o.after.transtab, o.after.memattr, o.after.transcfg),
            (TABLES_PA, 0x8f8f_8f8f_8c4c_8f4c, 0x4200_01c6)
        );
        assert_eq!(o.enable.flush.lockaddr, 63);
        assert_eq!(o.flush_pt.lockaddr, 0x0400_000f);
        assert_eq!(o.flush_pt.flush.cmd, 0x0304);
        // the GPU_CMD and AS_COMMAND writes, in order: the round trip, the
        // enable, the flush after the map
        let cmds: Vec<(usize, u32)> =
            g.sequence().into_iter().filter(|w| w.0 == GPU_CMD || w.0 == mmu_as(0) + AS_COMMAND).collect();
        let a = mmu_as(0) + AS_COMMAND;
        assert_eq!(
            cmds,
            vec![
                (GPU_CMD, 0x3304),
                (a, AS_COMMAND_LOCK),
                (GPU_CMD, 0x3304),
                (a, AS_COMMAND_UNLOCK),
                (a, AS_COMMAND_UPDATE),
                (a, AS_COMMAND_LOCK),
                (GPU_CMD, 0x0304),
                (a, AS_COMMAND_UNLOCK),
            ]
        );
    }

    #[test]
    fn the_live_memattr_holds_the_derived_attributes_a_pte_selects() {
        let derived = mmu::mair_to_memattr(pgtable::MAIR, false);
        assert_eq!(derived, 0x4c4c_4c4c_8c4c_8f4c);
        assert_eq!(MEMATTR_LIVE & 0xffff_ffff, derived & 0xffff_ffff);
    }

    #[test]
    fn z3_s_firmware_sections_take_nine_table_pages() {
        // the sections of mali_csffw.bin (the design page's "The firmware
        // (Z3)"), protected mode left out: the probe's 16-page pool holds them
        let mut pool = vec![[0u64; ENTRIES]; 16];
        let mut t = Tables::new(&mut pool, TABLES_PA).unwrap();
        let mut pa = BACKING_PA;
        for (va, size) in [
            (0x0000_0000, 0x1_0000),
            (0x0040_0000, 0x1_0000),
            (0x0041_0000, 0x1_0000),
            (0x0080_0000, 0x2_0000),
            (0x0100_0000, 0x4_0000),
            (0x0200_0000, 0x4_0000),
            (FW_SHARED_VA, FW_SHARED_SIZE),
        ] {
            t.map(va, pa, size, FW_SHARED_PROT).unwrap();
            pa += size;
        }
        assert_eq!(t.used(), 9);
    }

    #[test]
    fn another_mmu_shape_stops_before_any_write() {
        let g = Gpu::new(0);
        let mut pool = vec![[0u64; ENTRIES]; 8];
        let o = mmu_up(&g, BASE, 0x2828, &mut pool, TABLES_PA, BACKING_PA, &mut |_, _| 0, &mut |_| {});
        assert_eq!((o.stop, o.detail), (Stop::Features, 0x2828));
        assert!(g.writes.borrow().is_empty());
    }

    #[test]
    fn a_pool_too_small_or_a_failed_clean_stops_before_as0_is_touched() {
        let g = Gpu::new(0);
        let mut pool = vec![[0u64; ENTRIES]; 3];
        let mut cleans = vec![];
        let o = run(&g, &mut pool, &mut cleans);
        assert_eq!((o.stop, o.refusal), (Stop::Tables, PtRefusal::PoolFull as u32));
        assert!(cleans.is_empty());
        let g = Gpu::new(0);
        let mut pool = vec![[0u64; ENTRIES]; 8];
        let o = mmu_up(&g, BASE, 0x2830, &mut pool, TABLES_PA, BACKING_PA, &mut |_, _| -134, &mut |_| {});
        assert_eq!((o.stop, o.clean_rc), (Stop::Clean, -134));
        assert!(g.writes.borrow().iter().all(|w| !(0x2400..0x2440).contains(&w.0)), "AS0 written");
    }

    #[test]
    fn a_register_that_does_not_keep_its_value_is_named() {
        // AS_MEMATTR reads back 0: the run says which register and what it read
        struct Lossy(Gpu);
        impl Mmio for Lossy {
            fn read32(&self, a: usize) -> u32 {
                if a == BASE + mmu_as(0) + mmu::AS_MEMATTR || a == BASE + mmu_as(0) + mmu::AS_MEMATTR + 4 {
                    return 0;
                }
                self.0.read32(a)
            }
            fn write32(&self, a: usize, v: u32) {
                self.0.write32(a, v)
            }
            fn read8(&self, _: usize) -> u8 {
                unreachable!()
            }
            fn write8(&self, _: usize, _: u8) {
                unreachable!()
            }
        }
        let g = Lossy(Gpu::new(0));
        let mut pool = vec![[0u64; ENTRIES]; 8];
        let o = mmu_up(&g, BASE, 0x2830, &mut pool, TABLES_PA, BACKING_PA, &mut |_, _| 0, &mut |_| {});
        assert_eq!((o.stop, o.refusal, o.detail), (Stop::ReadBack, 2, 0));
    }

    #[test]
    fn a_fault_after_the_update_is_reported() {
        let g = Gpu::new(0);
        g.regs.borrow_mut().insert(mmu::MMU_INT_RAWSTAT, 1);
        let mut pool = vec![[0u64; ENTRIES]; 8];
        let mut cleans = vec![];
        let o = run(&g, &mut pool, &mut cleans);
        assert_eq!((o.stop, o.refusal, o.detail), (Stop::Fault, 3, 1));
        assert_eq!(o.found_irq[0], 1);
    }
}
