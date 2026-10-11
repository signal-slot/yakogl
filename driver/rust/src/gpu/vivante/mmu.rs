// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GC7000's MMU v2: two levels of 1024 entries, a master TLB whose
//! entry covers 4 MiB and holds a slave table's address, and slave tables
//! whose entries map 4 KiB pages; 32-bit GPU virtual addresses.
//!
//! The entry formats are galcore's (gc_hal_kernel_mmu.c `_SetPage`,
//! `_ConstructDynamicStlb`, gc_hal_kernel_mmu.h): a slave entry is the
//! page's physical bits 31:12, its bits 39:32 in entry bits 11:4, bit 2
//! writable, bit 0 present; an unused slave entry holds bit 1 (exception),
//! so a stray access faults instead of reading memory; a master entry is the
//! slave table's address with bit 0 present. galcore builds 1 KiB-mode
//! tables by default (gcdENABLE_MMU_1KMODE: 256 master entries of 16 MiB);
//! its 4 KiB mode -- master shift 22, 1024 entries at both levels, PTA mode
//! bit 0 clear -- is the one used here, the mode the board has run. Two
//! choices are ours, not galcore's: an unused *master* entry also holds the
//! exception bit (galcore leaves it 0), and the safe page is a whole 4 KiB
//! page of 0xdead55aa (galcore: 256 zero bytes), recognisable in a result
//! buffer.
//!
//! The enable is galcore's secure-mode setup on a part with the secure AHB
//! (gc_hal_kernel_hardware.c `gckHARDWARE_SetMMU` and the MMUv2 PTA path):
//! the page-table-array (PTA) address and enable, both safe addresses with
//! their bits 39:32, the array's entry 0 (master table address, 4 KiB mode)
//! in memory, then a command stream that sets the PTA index -- the index is
//! a state, not reachable over the AHB, so the front end has to execute it,
//! with the MMU still off, from a physical address -- and only then the
//! enable bit in MMUv2_SEC_CONTROL. Registers: rnndb ([`super::regs`]).
//!
//! The layout (identity-mapped carveout, the import window, one context,
//! never unmapped, so no per-submit flush) is this driver's own
//! (tools/cts/log/board-sessions/imx8mp-gc-driver-notes.md "M2: the MMU").
//! galcore's stream also
//! programs two more states (0x3848, 0x3878) and a semaphore/stall before
//! its END (gc_hal_kernel_hardware_func.c `_ProgramMMUStates`); the board
//! has run without them.

use super::cmd::Cmd;
use super::regs::*;
use super::{
    cmdbuf, GcCore, IMPORT_VA_BASE, OFF_MMU_CMD, OFF_MTLB, OFF_POOL, OFF_PTA, OFF_SAFE_PAGE, OFF_STLB, PAGE_SIZE,
    STLB_COUNT,
};
use crate::env::Env;
use crate::errno::{EINVAL, ENOMEM, ENOSPC, ERANGE};
use crate::mmio::Mmio;
use crate::ylog;

pub const PTE_PRESENT: u32 = 1 << 0;
pub const PTE_EXCEPTION: u32 = 1 << 1;
pub const PTE_WRITEABLE: u32 = 1 << 2;
pub const MTLB_SHIFT: u32 = 22;
pub const MTLB_MASK: u32 = 0xffc0_0000;
pub const STLB_SHIFT: u32 = 12;
pub const STLB_MASK: u32 = 0x003f_f000;
pub const ENTRIES: u32 = 1024;
pub const SAFE_PATTERN: u32 = 0xdead_55aa;

/// A slave entry for physical page `pa` (40-bit physical space).
pub const fn stlb_entry(pa: u64) -> u32 {
    (pa & 0xffff_f000) as u32 | ((((pa >> 32) & 0xff) as u32) << 4) | PTE_PRESENT | PTE_WRITEABLE
}

fn stlb_off(i: u32) -> usize {
    OFF_STLB + i as usize * PAGE_SIZE as usize
}

/// The slave table (its index in the pool) for master slot `mtlb_idx`,
/// taken from the pool the first time that 4 MiB is touched.
fn stlb_for(c: &mut GcCore, mtlb_idx: u32) -> Option<u32> {
    let mem = c.mem();
    let entry = mem.r32(OFF_MTLB + mtlb_idx as usize * 4);
    if entry & PTE_PRESENT != 0 {
        let phys = u64::from(entry & !(PAGE_SIZE - 1));
        return Some(((phys - (c.carveout_phys + OFF_STLB as u64)) / u64::from(PAGE_SIZE)) as u32);
    }
    if c.stlb_used >= STLB_COUNT {
        return None;
    }
    let i = c.stlb_used;
    c.stlb_used += 1;
    for e in 0..ENTRIES as usize {
        mem.w32(stlb_off(i) + e * 4, PTE_EXCEPTION);
    }
    // Only 32 bits of the slave table's address fit a master entry: the
    // tables live in the carveout, which mmu_init checked is below 4 GiB.
    mem.w32(OFF_MTLB + mtlb_idx as usize * 4, (c.carveout_phys + stlb_off(i) as u64) as u32 | PTE_PRESENT);
    Some(i)
}

/// Map `size` bytes at GPU VA `gpu_va` to physical `phys`, 4 KiB aligned.
pub fn map(c: &mut GcCore, env: &impl Env, gpu_va: u32, phys: u64, size: usize) -> Result<(), i32> {
    if (u64::from(gpu_va) | (phys & 0xffff_ffff) | size as u64) & u64::from(PAGE_SIZE - 1) != 0 {
        ylog!(env, Err, "mmu: map 0x{:08x} <- 0x{:x} ({}) is not 4 KiB aligned", gpu_va, phys, size);
        return Err(-EINVAL);
    }
    let mem = c.mem();
    let mut off = 0usize;
    while off < size {
        let va = gpu_va.wrapping_add(off as u32);
        let pa = phys + off as u64;
        let Some(i) = stlb_for(c, (va & MTLB_MASK) >> MTLB_SHIFT) else {
            ylog!(env, Err, "mmu: out of slave tables at VA 0x{:08x} ({} used)", va, c.stlb_used);
            return Err(-ENOMEM);
        };
        mem.w32(stlb_off(i) + ((va & STLB_MASK) >> STLB_SHIFT) as usize * 4, stlb_entry(pa));
        off += PAGE_SIZE as usize;
    }
    Ok(())
}

/// Build the tables with the MMU off: every master entry faults, the safe
/// page holds its pattern, the carveout is identity-mapped.
pub fn init(c: &mut GcCore, env: &impl Env) -> Result<(), i32> {
    let top = c.carveout_phys + c.carveout_size as u64;
    if top > 0x1_0000_0000 {
        ylog!(
            env,
            Err,
            "mmu: the carveout ends at 0x{:x}; the page tables and the identity mapping need it below 4 GiB",
            top
        );
        return Err(-ERANGE);
    }
    if c.carveout_size < OFF_POOL + PAGE_SIZE as usize {
        ylog!(
            env,
            Err,
            "mmu: the carveout is {} KiB; the fixed layout needs {} KiB",
            c.carveout_size / 1024,
            (OFF_POOL + PAGE_SIZE as usize) / 1024
        );
        return Err(-ENOSPC);
    }
    let mem = c.mem();
    // SAFETY (pointer arithmetic only): inside the carveout, checked above.
    c.mtlb = c.carveout.wrapping_add(OFF_MTLB).cast();
    c.pta = c.carveout.wrapping_add(OFF_PTA).cast();
    c.stlb_used = 0;
    c.import_va = IMPORT_VA_BASE;
    for i in 0..ENTRIES as usize {
        mem.w32(OFF_MTLB + i * 4, PTE_EXCEPTION);
    }
    for i in 0..(PAGE_SIZE / 4) as usize {
        mem.w32(OFF_SAFE_PAGE + i * 4, SAFE_PATTERN);
    }
    map(c, env, c.carveout_phys as u32, c.carveout_phys, c.carveout_size)?;
    ylog!(
        env,
        Inf,
        "mmu: MTLB at 0x{:x}, carveout 0x{:x}+{}K identity-mapped in {} slave tables",
        c.carveout_phys + OFF_MTLB as u64,
        c.carveout_phys,
        c.carveout_size / 1024,
        c.stlb_used
    );
    Ok(())
}

/// The stream that selects PTA entry `index`, at OFF_MMU_CMD; its size in
/// bytes.
pub fn build_pta_config(c: &GcCore, index: u32) -> u32 {
    let mem = c.mem();
    let mut s = Cmd::new(&mem, OFF_MMU_CMD, (c.carveout_phys + OFF_MMU_CMD as u64) as u32, PAGE_SIZE / 4);
    s.state(VIVS_MMUv2_PTA_CONFIG, index & VIVS_MMUv2_PTA_CONFIG_INDEX__MASK);
    s.end();
    s.align8();
    s.n * 4
}

/// Turn the MMU on (the module text's order).
pub fn enable(c: &mut GcCore, m: &impl Mmio, env: &impl Env) -> Result<(), i32> {
    let regs = c.regs;
    let rd = |o: u32| m.read32(regs + o as usize);
    let wr = |o: u32, v: u32| m.write32(regs + o as usize, v);
    let safe = c.carveout_phys + OFF_SAFE_PAGE as u64;
    let pta = c.carveout_phys + OFF_PTA as u64;
    let mtlb = c.carveout_phys + OFF_MTLB as u64;

    if rd(VIVS_MMUv2_SEC_CONTROL) & VIVS_MMUv2_SEC_CONTROL_ENABLE != 0 {
        ylog!(env, Wrn, "mmu: already enabled; the reset did not clear it");
        c.mmu_on = true;
        return Ok(());
    }
    wr(VIVS_MMUv2_PTA_ADDRESS_LOW, pta as u32);
    wr(VIVS_MMUv2_PTA_ADDRESS_HIGH, (pta >> 32) as u32);
    wr(VIVS_MMUv2_PTA_CONTROL, VIVS_MMUv2_PTA_CONTROL_ENABLE);
    wr(VIVS_MMUv2_NONSEC_SAFE_ADDR_LOW, safe as u32);
    wr(VIVS_MMUv2_SEC_SAFE_ADDR_LOW, safe as u32);
    let hi = (safe >> 32) as u32;
    wr(
        VIVS_MMUv2_SAFE_ADDRESS_CONFIG,
        (hi & VIVS_MMUv2_SAFE_ADDRESS_CONFIG_NON_SEC_SAFE_ADDR_HIGH__MASK)
            | ((hi << VIVS_MMUv2_SAFE_ADDRESS_CONFIG_SEC_SAFE_ADDR_HIGH__SHIFT)
                & VIVS_MMUv2_SAFE_ADDRESS_CONFIG_SEC_SAFE_ADDR_HIGH__MASK),
    );
    c.mem().w64(OFF_PTA, mtlb | u64::from(VIVS_MMUv2_CONFIGURATION_MODE_MODE4_K));

    let bytes = build_pta_config(c, 0);
    cmdbuf::fe_start(c, m, (c.carveout_phys + OFF_MMU_CMD as u64) as u32, bytes / 8);
    if let Err(e) = cmdbuf::fe_wait_idle(c, m, env, 100) {
        ylog!(env, Err, "mmu: the PTA-config stream did not finish; the MMU stays off");
        return Err(e);
    }
    wr(VIVS_MMUv2_SEC_CONTROL, VIVS_MMUv2_SEC_CONTROL_ENABLE);
    c.mmu_on = true;
    ylog!(env, Inf, "mmu: on (PTA 0x{:x} entry 0 -> MTLB 0x{:x}, safe page 0x{:x})", pta, mtlb, safe);
    Ok(())
}

/// The secure-mode fault registers, decoded (rnndb MMU_EXCEPTION codes).
pub fn dump(c: &GcCore, m: &impl Mmio, env: &impl Env) {
    const REASON: [&str; 3] = ["slave not present", "page not present", "write violation"];
    let rd = |o: u32| m.read32(c.regs + o as usize);
    let status = rd(VIVS_MMUv2_SEC_STATUS);
    let addr = rd(VIVS_MMUv2_SEC_EXCEPTION_ADDR);
    let ctrl = rd(VIVS_MMUv2_SEC_CONTROL);
    ylog!(env, Err, "mmu: SEC_STATUS 0x{:08x} SEC_CONTROL 0x{:08x} fault addr 0x{:08x}", status, ctrl, addr);
    for i in 0..4 {
        let code = (status >> (i * 4)) & 3;
        if code != 0 {
            ylog!(env, Err, "mmu {}: {} at 0x{:08x}", i, REASON[code as usize - 1], addr);
        }
    }
}
