// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The NanoPC-T6 (RK3588) board glue's Rust side, for
//! driver/zephyr/lib/board/nanopc_t6/nanopc_t6.c: the WDT0 self-arm
//! (soc::rk3588 gates, ip::dw_wdt), its feed, the GIC CPU interface reset,
//! the GIC Active state clear and PSCI SYSTEM_RESET (aarch64), the feeder's
//! decision
//! (board::nanopc_t6), and the early banner with the EL2 entry fix
//! (board::nanopc_t6_early, aarch64). No device, no env: the shim calls these with
//! its mappings and logs from what comes back. Declarations:
//! include/yakogl_drivers.h ("RK3588 board glue").

use crate::errno::{EINVAL, EIO};
use crate::ip::dw_wdt::{self, Armed, Refused};
use crate::mmio::{Hw, Mmio};
use crate::soc::rk3588;
use core::mem::{offset_of, size_of};

/// `flags` of [`YdDwWdtArm`].
pub const YD_DW_WDT_WAS_RUNNING: u32 = 1 << 0;
pub const YD_DW_WDT_PCLK_OPENED: u32 = 1 << 1;
pub const YD_DW_WDT_TCLK_OPENED: u32 = 1 << 2;

/// `refusal` of [`YdDwWdtArm`] for a CRU gate that stays closed (the
/// [`dw_wdt::Refusal`] numbers are 1..=7).
pub const YD_DW_WDT_REFUSED_GATE: u32 = 16;

/// `struct yd_dw_wdt_arm`: what the arming found and left.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdDwWdtArm {
    pub comp_type: u32,
    pub param1: u32,
    pub version: u32,
    /// `WDT_CR` and `WDT_TORR` as found, before anything was written.
    pub cr_found: u32,
    pub torr_found: u32,
    /// `WDT_CR`, `WDT_TORR` and `WDT_CCVR` read back after arming; on a
    /// refusal `cr_now` is the read the refusal is about
    /// ([`Refused::read_back`]; the CLKGATE_CON value for a gate).
    pub cr_now: u32,
    pub torr_now: u32,
    pub ccvr_now: u32,
    pub flags: u32,
    /// 0, or the [`dw_wdt::Refusal`] number, or [`YD_DW_WDT_REFUSED_GATE`].
    pub refusal: u32,
    /// The period now counting, rounded down; 0 on a refusal.
    pub period_ms: u32,
}

const _: () = assert!(size_of::<YdDwWdtArm>() == 44);
const _: () = assert!(offset_of!(YdDwWdtArm, comp_type) == 0);
const _: () = assert!(offset_of!(YdDwWdtArm, cr_found) == 12);
const _: () = assert!(offset_of!(YdDwWdtArm, cr_now) == 20);
const _: () = assert!(offset_of!(YdDwWdtArm, flags) == 32);
const _: () = assert!(offset_of!(YdDwWdtArm, refusal) == 36);
const _: () = assert!(offset_of!(YdDwWdtArm, period_ms) == 40);

/// Open WDT0's two CRU gates (the APB clock first: the block's registers
/// need it), then [`dw_wdt::arm`] with `top` on xin24m. 0, `-EIO` when the
/// arming refused (the reason in `out.refusal`).
///
/// `out` is written one field at a time, each a volatile 32-bit store, never
/// as a whole: this also runs with the MMU off (`yd_t6_early_arm`), where
/// every access is to Device memory, and a whole-struct store may become a
/// `memset` or `memcpy` call, whose `dc zva` or unaligned accesses fault
/// there.
pub fn dw_wdt_arm_with(m: &impl Mmio, wdt_base: usize, cru_base: usize, top: u8, out: &mut YdDwWdtArm) -> i32 {
    fn set(field: &mut u32, v: u32) {
        // SAFETY: a valid, aligned u32.
        unsafe { core::ptr::write_volatile(field, v) }
    }
    set(&mut out.comp_type, 0);
    set(&mut out.param1, 0);
    set(&mut out.version, 0);
    set(&mut out.cr_found, 0);
    set(&mut out.torr_found, 0);
    set(&mut out.cr_now, 0);
    set(&mut out.torr_now, 0);
    set(&mut out.ccvr_now, 0);
    set(&mut out.flags, 0);
    set(&mut out.refusal, 0);
    set(&mut out.period_ms, 0);
    let mut flags = 0;
    for (gate, flag) in [(rk3588::PCLK_WDT0, YD_DW_WDT_PCLK_OPENED), (rk3588::TCLK_WDT0, YD_DW_WDT_TCLK_OPENED)] {
        match rk3588::gate_open(m, cru_base, gate) {
            Ok(true) => flags |= flag,
            Ok(false) => {}
            Err(now) => {
                set(&mut out.flags, flags);
                set(&mut out.cr_now, now);
                set(&mut out.refusal, YD_DW_WDT_REFUSED_GATE);
                return -EIO;
            }
        }
    }
    match dw_wdt::arm(m, wdt_base, top, rk3588::TCLK_WDT0_HZ) {
        Ok(Armed { cr_found, torr_found, param1, version, cr_now, torr_now, ccvr_now, was_running, period_ms }) => {
            set(&mut out.comp_type, dw_wdt::COMP_TYPE_VALUE);
            set(&mut out.param1, param1);
            set(&mut out.version, version);
            set(&mut out.cr_found, cr_found);
            set(&mut out.torr_found, torr_found);
            set(&mut out.cr_now, cr_now);
            set(&mut out.torr_now, torr_now);
            set(&mut out.ccvr_now, ccvr_now);
            if was_running {
                flags |= YD_DW_WDT_WAS_RUNNING;
            }
            set(&mut out.flags, flags);
            set(&mut out.period_ms, period_ms);
            0
        }
        Err(Refused { why, comp_type, param1, cr_found, torr_found, read_back }) => {
            set(&mut out.comp_type, comp_type);
            set(&mut out.param1, param1);
            set(&mut out.cr_found, cr_found);
            set(&mut out.torr_found, torr_found);
            set(&mut out.cr_now, read_back);
            if cr_found & dw_wdt::cr::WDT_EN != 0 {
                flags |= YD_DW_WDT_WAS_RUNNING;
            }
            set(&mut out.flags, flags);
            set(&mut out.refusal, why as u32);
            -EIO
        }
    }
}

/// [`dw_wdt_arm_with`] after the argument checks of [`yd_dw_wdt_arm`].
///
/// # Safety
/// Both bases address the blocks as device memory; `out` is null or valid.
unsafe fn dw_wdt_arm_checked(wdt_base: usize, cru_base: usize, top: u32, out: *mut YdDwWdtArm) -> i32 {
    if out.is_null() || top > u32::from(dw_wdt::TOP_MAX) {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    let hw = unsafe { Hw::new() };
    // SAFETY: the caller's contract.
    unsafe { dw_wdt_arm_with(&hw, wdt_base, cru_base, top as u8, &mut *out) }
}

/// Arm WDT0 (`wdt_base`, the shim's `device_map()` of 0xfeaf0000) for the
/// period of `top` (0..=15: 2^(16 + top) cycles of 24 MHz), through the CRU
/// at `cru_base` (its mapping of 0xfd7c0000, at least 0x1000 bytes). The
/// image's own arming is `yd_t6_early_arm`; this is the glue's second try
/// after a refusal.
///
/// # Safety
/// Both bases are mapped device memory covering the blocks; `out` is valid.
#[no_mangle]
pub unsafe extern "C" fn yd_dw_wdt_arm(wdt_base: usize, cru_base: usize, top: u32, out: *mut YdDwWdtArm) -> i32 {
    // SAFETY: the caller's contract.
    unsafe { dw_wdt_arm_checked(wdt_base, cru_base, top, out) }
}

/// Restart WDT0's counter.
///
/// # Safety
/// `wdt_base` is the mapping [`yd_dw_wdt_arm`] was given.
#[no_mangle]
pub unsafe extern "C" fn yd_dw_wdt_feed(wdt_base: usize) {
    // SAFETY: the caller's contract.
    let hw = unsafe { Hw::new() };
    dw_wdt::feed(&hw, wdt_base);
}

/// The feeder's verdict for the tick at uptime `now_ms`
/// ([`crate::board::nanopc_t6::feed_verdict`]): 0 feed, otherwise the
/// `YD_T6_FEED_*` cause that stops the feeding.
#[no_mangle]
pub extern "C" fn yd_t6_feed_verdict(
    now_ms: i64,
    alive_at_ms: i64,
    released: bool,
    deadline_ms: u32,
    alive_ms: u32,
) -> u32 {
    crate::board::nanopc_t6::feed_verdict(now_ms, alive_at_ms, released, deadline_ms, alive_ms) as u32
}

/// `flags` of [`YdGicCpuifReset`].
pub const YD_GIC_CPUIF_RESET: u32 = 1 << 0;
pub const YD_GIC_EOIMODE_CLEARED: u32 = 1 << 1;

/// `struct yd_gic_cpuif_reset`: what [`crate::aarch64::gic_cpuif_reset`]
/// found and did.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdGicCpuifReset {
    /// `ICC_SRE_EL1` as found.
    pub sre: u64,
    /// `ICC_CTLR_EL1`, `ICC_AP1R0_EL1` and `ICC_BPR1_EL1` as found; 0 when
    /// SRE was clear.
    pub ctlr_found: u64,
    pub ap1r0_found: u64,
    pub bpr1_found: u64,
    /// The number of `ICC_AP1Rn_EL1` written 0 (`ICC_BPR1_EL1` is written 0
    /// with them).
    pub ap1r_cleared: u32,
    /// [`YD_GIC_CPUIF_RESET`]: SRE was set and the registers were reset;
    /// [`YD_GIC_EOIMODE_CLEARED`]: EOImode was 1 and is now 0.
    pub flags: u32,
}

const _: () = assert!(size_of::<YdGicCpuifReset>() == 40);
const _: () = assert!(offset_of!(YdGicCpuifReset, ctlr_found) == 8);
const _: () = assert!(offset_of!(YdGicCpuifReset, ap1r0_found) == 16);
const _: () = assert!(offset_of!(YdGicCpuifReset, bpr1_found) == 24);
const _: () = assert!(offset_of!(YdGicCpuifReset, ap1r_cleared) == 32);
const _: () = assert!(offset_of!(YdGicCpuifReset, flags) == 36);

impl From<crate::aarch64::GicCpuifReset> for YdGicCpuifReset {
    fn from(r: crate::aarch64::GicCpuifReset) -> Self {
        let mut flags = 0;
        if r.sre & crate::aarch64::ICC_SRE_EL1_SRE != 0 {
            flags |= YD_GIC_CPUIF_RESET;
        }
        if r.eoimode_cleared {
            flags |= YD_GIC_EOIMODE_CLEARED;
        }
        YdGicCpuifReset {
            sre: r.sre,
            ctlr_found: r.ctlr,
            ap1r0_found: r.ap1r0,
            bpr1_found: r.bpr1,
            ap1r_cleared: r.ap1r_cleared,
            flags,
        }
    }
}

/// This CPU's GICv3 CPU interface back to its reset EOI mode with no active
/// priority ([`crate::aarch64::gic_cpuif_reset`]); what it found and did in
/// `*out`.
///
/// # Safety
/// At EL1, on the boot CPU, before Zephyr's GICv3 init and before interrupts
/// are enabled; `out` is valid for writes.
#[cfg(target_arch = "aarch64")]
#[no_mangle]
pub unsafe extern "C" fn yd_gic_cpuif_reset(out: *mut YdGicCpuifReset) {
    // SAFETY: the caller's contract is SysRegs::new's.
    let mut regs = unsafe { crate::aarch64::SysRegs::new() };
    let r = crate::aarch64::gic_cpuif_reset(&mut regs);
    // SAFETY: the caller's contract.
    unsafe { out.write(r.into()) };
}

/// `flags` of [`YdGicActiveReset`].
pub const YD_GIC_RDIST_FOUND: u32 = 1 << 0;

/// `struct yd_gic_active_reset`: what [`crate::aarch64::gic_active_reset`]
/// found and did.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdGicActiveReset {
    /// This CPU's `GICR_TYPER`; the last one read when none matched.
    pub gicr_typer: u64,
    pub gicd_typer: u32,
    /// `MPIDR_EL1`'s affinity as `GICR_TYPER[63:32]` holds it.
    pub aff: u32,
    /// The `GICR_TYPER` read in the walk.
    pub frames: u32,
    /// RD_base less the region's base, of the one found; 0 when none.
    pub rd_offset: u32,
    /// `GICR_ISACTIVER0` as found; 0 when no redistributor matched.
    pub isactiver0: u32,
    pub spi_words: u32,
    pub eppi_words: u32,
    pub espi_words: u32,
    pub words_cleared: u32,
    /// The INTIDs Active as found, and the first of them.
    pub active: u32,
    pub intids: [u32; crate::aarch64::GIC_ACTIVE_LISTED],
    /// The INTIDs still Active after the clear.
    pub still_active: u32,
    /// [`YD_GIC_RDIST_FOUND`].
    pub flags: u32,
}

const _: () = assert!(size_of::<YdGicActiveReset>() == 88);
const _: () = assert!(offset_of!(YdGicActiveReset, gicd_typer) == 8);
const _: () = assert!(offset_of!(YdGicActiveReset, isactiver0) == 24);
const _: () = assert!(offset_of!(YdGicActiveReset, active) == 44);
const _: () = assert!(offset_of!(YdGicActiveReset, intids) == 48);
const _: () = assert!(offset_of!(YdGicActiveReset, still_active) == 80);
const _: () = assert!(offset_of!(YdGicActiveReset, flags) == 84);

impl YdGicActiveReset {
    /// The record of `r`; `gicr` is the region base `rd_offset` counts from.
    pub fn from_reset(r: crate::aarch64::GicActiveReset, gicr: usize) -> Self {
        let (flags, rd_offset) = match r.rdist.rd_base {
            Some(rd) => (YD_GIC_RDIST_FOUND, (rd - gicr) as u32),
            None => (0, 0),
        };
        YdGicActiveReset {
            gicr_typer: r.rdist.typer,
            gicd_typer: r.gicd_typer,
            aff: r.aff,
            frames: r.rdist.frames,
            rd_offset,
            isactiver0: r.isactiver0,
            spi_words: r.spi_words,
            eppi_words: r.eppi_words,
            espi_words: r.espi_words,
            words_cleared: r.words_cleared,
            active: r.active,
            intids: r.intids,
            still_active: r.still_active,
            flags,
        }
    }
}

/// Every interrupt left Active at the distributor `gicd` and at this CPU's
/// redistributor deactivated ([`crate::aarch64::gic_active_reset`]; the
/// redistributor by `MPIDR_EL1` in the region `gicr`, `gicr_size`,
/// `gicr_stride`); what it found and did in `*out`.
///
/// # Safety
/// At EL1, on the boot CPU, before Zephyr's GICv3 init and before interrupts
/// are enabled; `gicd` (64 KiB) and the region are the GIC node's `reg`
/// entries and mapped as device memory; `out` is valid for writes.
#[cfg(target_arch = "aarch64")]
#[no_mangle]
pub unsafe extern "C" fn yd_gic_active_reset(
    gicd: usize,
    gicr: usize,
    gicr_size: usize,
    gicr_stride: usize,
    out: *mut YdGicActiveReset,
) {
    // SAFETY: the caller's contract: every address gic_active_reset reaches
    // is inside the two mapped ranges (gicr_find stays in the region).
    let hw = unsafe { Hw::new() };
    let aff = crate::aarch64::mpidr_gicr_affinity(crate::aarch64::read_mpidr_el1());
    let region = crate::aarch64::GicrRegion { base: gicr, size: gicr_size, stride: gicr_stride };
    let r = crate::aarch64::gic_active_reset(&hw, gicd, region, aff);
    // SAFETY: the caller's contract.
    unsafe { out.write(YdGicActiveReset::from_reset(r, gicr)) };
}

/// PSCI `SYSTEM_RESET` over SMC. Returns only when the firmware did not
/// reset, with the PSCI return code.
///
/// # Safety
/// The board's firmware implements PSCI over SMC.
#[cfg(target_arch = "aarch64")]
#[no_mangle]
pub unsafe extern "C" fn yd_psci_system_reset() -> i32 {
    // SAFETY: the caller's contract.
    unsafe { crate::aarch64::psci_system_reset() }
}

/// `flags` of the early entry (the C registration's `x0`): write
/// [`crate::aarch64::HCR_EL2_AT_ENTRY`] to `HCR_EL2` before Zephyr's EL2 init
/// reads it (CONFIG_YAKOGL_NANOPC_T6_VHE_EXIT).
pub const YD_T6_EARLY_VHE_EXIT: u64 = 1 << 0;

/// The image's first code and the EL2 trap reporter
/// (board::nanopc_t6_early has the lines and why).
///
/// `yd_t6_early_entry` is what nanopc_t6.c registers as Zephyr's
/// `z_arm64_el2_plat_prep_c` and `z_arm64_el1_plat_prep_c` (arch/arm64/core/
/// reset.S, zephyr af17c0c8: weak, called from `__reset_prep_c` at the EL the
/// image was entered at, before any C code: "Stack is not yet available",
/// "x23, x24 and x25 must be preserved"). `__start` has already loaded `x24`
/// with the top of `z_interrupt_stacks`, which `__reset_prep_c` makes the
/// stack right after the hook. The entry first turns off the MMU and the
/// caches when U-Boot's `go` left them on, with no stack and no load or
/// store (board::nanopc_t6_go, its text `t6_go_off_asm`; with them already
/// off, as after kexec, it changes nothing); then it makes `x24` the stack,
/// keeps the return address in `x22` (callee-saved, unused by `__start` and
/// `__reset_prep_c`), and calls [`yd_t6_early_c`] with the flags in `x0`
/// and what the first part found in `x1`..`x4`.
///
/// `yd_t6_el2_vectors` is the EL2 vector table [`yd_t6_early_c`] installs:
/// every one of the 16 entries passes its offset to [`yd_t6_el2_trap`] on a
/// stack of its own and never returns.
///
/// [`yd_t6_early_arm`] is the image's WDT0 arming, from Zephyr's
/// `z_arm64_el1_plat_init`, MMU off: before `z_prep_c` and every init hook.
#[cfg(target_arch = "aarch64")]
mod early {
    use super::YdDwWdtArm;
    use crate::aarch64;
    use crate::board::nanopc_t6_early::{self as banner, El2Entry, El2Trap, UART2_PHYS};
    use crate::board::nanopc_t6_go::{self as go, t6_go_off_asm};
    use crate::ip::ns16550_tx::Tx;
    use crate::mmio::Hw;

    const TRAP_STACK: usize = 1024;

    /// The trap reporter's stack: its own, so a trap does not overwrite the
    /// frames of the code it stopped.
    #[repr(C, align(16))]
    struct Stack([u8; TRAP_STACK]);
    static mut YD_T6_EL2_TRAP_STACK: Stack = Stack([0; TRAP_STACK]);

    core::arch::global_asm!(
        ".pushsection .text.yd_t6_early_entry, \"ax\"",
        ".global yd_t6_early_entry",
        ".type yd_t6_early_entry, %function",
        ".balign 4",
        "yd_t6_early_entry:",
        t6_go_off_asm!(),
        "    mov x22, x30",
        "    mov sp, x24",
        "    bl {early}",
        "    mov x30, x22",
        "    ret",
        ".size yd_t6_early_entry, . - yd_t6_early_entry",
        ".popsection",
        ".pushsection .text.yd_t6_el2_vectors, \"ax\"",
        ".global yd_t6_el2_vectors",
        ".type yd_t6_el2_vectors, %function",
        ".balign 2048",
        "yd_t6_el2_vectors:",
        ".irp off, 0x000, 0x080, 0x100, 0x180, 0x200, 0x280, 0x300, 0x380, 0x400, 0x480, 0x500, 0x580, 0x600, 0x680, 0x700, 0x780",
        ".balign 128",
        "    mov x0, #\\off",
        "    b .Lyd_t6_el2_common",
        ".endr",
        ".Lyd_t6_el2_common:",
        "    adrp x9, {stack}",
        "    add x9, x9, :lo12:{stack}",
        "    add sp, x9, #{size}",
        "    bl {trap}",
        ".Lyd_t6_el2_halt:",
        "    wfe",
        "    b .Lyd_t6_el2_halt",
        ".size yd_t6_el2_vectors, . - yd_t6_el2_vectors",
        ".popsection",
        early = sym yd_t6_early_c,
        trap = sym yd_t6_el2_trap,
        stack = sym YD_T6_EL2_TRAP_STACK,
        size = const TRAP_STACK,
    );

    fn vectors() -> u64 {
        let v: u64;
        // SAFETY: an address computation.
        unsafe {
            core::arch::asm!(
                "adrp {0}, yd_t6_el2_vectors",
                "add {0}, {0}, :lo12:yd_t6_el2_vectors",
                out(reg) v,
                options(nomem, nostack, preserves_flags),
            );
        }
        v
    }

    /// `T6:A`, and at EL2: `VBAR_EL2` to the trap reporter first (any EL2
    /// exception after this is reported), then, with
    /// [`super::YD_T6_EARLY_VHE_EXIT`], `HCR_EL2` to
    /// [`aarch64::HCR_EL2_AT_ENTRY`]. Then `T6:G` when the entry turned off
    /// the MMU and caches `go` left on: `go_sctlr` the `SCTLR_EL2` it found
    /// (0 when it found them off), `start`..`end` the range it cleaned,
    /// `left` the `SCTLR_EL2` it read back after the turn-off. Entered at EL1,
    /// `T6:G EL1` when `SCTLR_EL1` has the MMU or a cache on.
    ///
    /// # Safety
    /// Only from `yd_t6_early_entry`, as reset.S's plat_prep_c hook: MMU off,
    /// uart2 as the previous stage left it, `.bss` not yet used.
    #[no_mangle]
    pub unsafe extern "C" fn yd_t6_early_c(flags: u64, go_sctlr: u64, start: u64, end: u64, left: u64) {
        // SAFETY: with the MMU off every address is the physical one, and
        // UART2_PHYS is uart2's 32-bit registers.
        let hw = unsafe { Hw::new() };
        let mut tx = Tx::new(&hw, UART2_PHYS);
        let el = aarch64::current_el();
        if el != 2 {
            banner::banner_a_other(&mut tx, el);
            if el == 1 {
                let sctlr_el1: u64;
                // SAFETY: at EL1; reading the register has no side effect.
                unsafe {
                    core::arch::asm!("mrs {}, sctlr_el1", out(reg) sctlr_el1, options(nomem, nostack, preserves_flags));
                }
                go::print_el1(&mut tx, sctlr_el1);
            }
            return;
        }
        let vbar_found = aarch64::read_vbar_el2();
        // SAFETY: at EL2; the table is 2 KiB aligned and its code runs with
        // the EL2 MMU off (it reads only EL2 registers and uart2).
        unsafe { aarch64::write_vbar_el2(vectors()) };
        let hcr_found = aarch64::read_hcr_el2();
        let hcr_now = if flags & super::YD_T6_EARLY_VHE_EXIT != 0 {
            // SAFETY: at EL2, before Zephyr's z_arm64_el2_init and before
            // anything runs below EL2.
            unsafe { aarch64::write_hcr_el2(aarch64::HCR_EL2_AT_ENTRY) };
            Some(aarch64::read_hcr_el2())
        } else {
            None
        };
        let e = El2Entry {
            hcr_found,
            hcr_now,
            sctlr: aarch64::read_sctlr_el2(),
            vbar_found,
            vbar_now: aarch64::read_vbar_el2(),
        };
        banner::banner_a_el2(&mut tx, &e);
        go::print_go(&mut tx, go_sctlr, left, start, end);
    }

    /// `T6:X`, then halt.
    ///
    /// # Safety
    /// Only from `yd_t6_el2_vectors`, at EL2 with its MMU off.
    #[no_mangle]
    pub unsafe extern "C" fn yd_t6_el2_trap(vector: u64) -> ! {
        let t = El2Trap {
            vector,
            esr: aarch64::read_esr_el2(),
            elr: aarch64::read_elr_el2(),
            far: aarch64::read_far_el2(),
            spsr: aarch64::read_spsr_el2(),
            hcr: aarch64::read_hcr_el2(),
        };
        // SAFETY: the EL2 MMU is off: UART2_PHYS is uart2.
        let hw = unsafe { Hw::new() };
        banner::banner_x(&mut Tx::new(&hw, UART2_PHYS), &t);
        loop {
            // SAFETY: waits; WDT0, if it runs, resets the board.
            unsafe { core::arch::asm!("wfe", options(nomem, nostack, preserves_flags)) };
        }
    }

    /// `T6:A1`, `T6:A2`, [`super::yd_dw_wdt_arm`] on WDT0 and the CRU at
    /// their physical bases `wdt_phys` and `cru_phys`, `T6:A3`:
    /// from Zephyr's `z_arm64_el1_plat_init`, the end of `z_arm64_el1_init`,
    /// which reset.S runs before `z_prep_c` and so before every init hook of
    /// every level (nanopc_t6.c, "Order"). Returns what `yd_dw_wdt_arm`
    /// would; `out` gets the record.
    ///
    /// # Safety
    /// At EL1 with the MMU off (before `z_prep_c`): every address is the
    /// physical one, every access is to Device memory. The bases are WDT0's
    /// and the CRU's; `out` is null or valid, outside `.bss` (which
    /// `z_prep_c` zeroes after this).
    #[no_mangle]
    pub unsafe extern "C" fn yd_t6_early_arm(wdt_phys: usize, cru_phys: usize, top: u32, out: *mut YdDwWdtArm) -> i32 {
        // SAFETY: the MMU is off: UART2_PHYS is uart2.
        let hw = unsafe { Hw::new() };
        let mut tx = Tx::new(&hw, UART2_PHYS);
        banner::banner_a1(&mut tx);
        banner::banner_a2(&mut tx);
        // SAFETY: the caller's contract.
        let rc = unsafe { super::dw_wdt_arm_checked(wdt_phys, cru_phys, top, out) };
        let (cr_now, period_ms) = if out.is_null() {
            (0, 0)
        } else {
            // SAFETY: as above; volatile reads of what was just written.
            unsafe {
                (
                    core::ptr::read_volatile(core::ptr::addr_of!((*out).cr_now)),
                    core::ptr::read_volatile(core::ptr::addr_of!((*out).period_ms)),
                )
            }
        };
        banner::banner_a3(&mut tx, rc, cr_now, period_ms);
        rc
    }
}

/// `struct yd_t6_gpu_up`: what [`crate::board::nanopc_t6_gpu::gpu_up`]
/// read, in 32-bit words (a 64-bit value is lo, hi). The `*_regs` arrays
/// are `PWR_GATE`, `STATUS`, `BUS_IDLE_REQ`, `BUS_IDLE_ACK`, `BUS_IDLE_ST`,
/// `REPAIR_STATUS`; `clock` is `CLKSEL_CON(158)`, `CLKSEL_CON(159)`,
/// `CLKGATE_CON(66)`, `MODE_CON0`, the selected PLL's `PLL_CON` 0..2;
/// `ident` is `GPU_ID`, `L2_FEATURES`, `TILER_FEATURES`, `MEM_FEATURES`,
/// `MMU_FEATURES`, `AS_PRESENT`, `CSF_ID`, `SHADER_PRESENT` lo/hi,
/// `TILER_PRESENT` lo/hi, `L2_PRESENT` lo/hi, `COHERENCY_FEATURES`,
/// `GPU_STATUS`, `MCU_STATUS`.
#[cfg(any(feature = "mali", test))]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdT6GpuUp {
    pub pd_before: [u32; 6],
    /// `CLKSEL_CON(158)`, `(159)`, `CLKGATE_CON(66)`, `MODE_CON0`, the
    /// selected PLL's `PLL_CON0..2` and `PLL_CON6`, as found.
    pub clock: [u32; 8],
    pub gpu_src_hz: [u32; 2],
    pub spi_gate14: u32,
    pub spi_opened: u32,
    /// RK806 `POWER_EN0`, `POWER_SLP_EN0`, `BUCK1_CONFIG`, `BUCK1_ON_VSEL`.
    pub rk806: [u32; 4],
    /// SPI2 as handed over: `CTRLR0`, `CTRLR1`, `SSIENR`, `SER`, `BAUDR`,
    /// `SR`, `RXFLR`, `VERSION`.
    pub spi_found: [u32; 8],
    pub vdd_gpu_uv: u32,
    /// The GPU clock's PLL: read powered before any write, the microseconds
    /// its lock took, `PLL_CON6` after.
    pub pll_on: [u32; 3],
    pub pd_opened: u32,
    pub pd_was_on: u32,
    pub pd_waited_us: [u32; 3],
    pub pd_after: [u32; 6],
    pub qos: [[u32; 5]; 4],
    pub ident: [u32; 16],
    /// `GPU_INT_RAWSTAT` before the clear, after it, and the polls.
    pub reset: [u32; 3],
    pub ident_after_reset: [u32; 16],
    pub l2_mask: [u32; 2],
    pub l2_ready_before: [u32; 2],
    pub l2_ready: [u32; 2],
    pub l2_waited_us: [u32; 2],
    /// `YD_T6_GPU_STOP_*`, the refusal number of that step, and the value it
    /// is about (lo, hi).
    pub stop: u32,
    pub refusal: u32,
    pub detail: [u32; 2],
    /// Appended: the clock set ([`crate::soc::rk3588::ClockSet`]: kept, the
    /// CPLL's rate, the words written to `CLKSEL_CON(158)`/`(159)`, those
    /// registers after it, the source's rate after it) and the supply enable.
    pub clock_kept: u32,
    pub clock_cpll_hz: [u32; 2],
    pub clock_wrote: [u32; 2],
    pub clock_sel: [u32; 2],
    pub clock_set_hz: [u32; 2],
    pub supply_enabled: u32,
    /// `POWER_EN0` and `BUCK1_ON_VSEL` after the enable.
    pub supply_after: [u32; 2],
    pub supply_waited_us: u32,
    /// Appended: the GPU QoS policy applied after the power-on
    /// ([`rk3588::QosPolicy::id`]: 1 reset, 2 linux-zero; 0 when the run
    /// stopped before it) and `MODE`, `BANDWIDTH`, `SATURATION` of
    /// `qos_gpu_m0` read back.
    pub qos_policy: u32,
    pub qos_words: [u32; 3],
    /// Appended: the take-over of a GPU found powered
    /// ([`crate::gpu::mali::takeover::TakeOver`]): 1 when it ran (0: PD_GPU
    /// read off at hand-over, or the run stopped before the step); the
    /// steps it got through (`TakeOver::steps`: 1 masked and cleared, 2 the
    /// MCU halt waited for, 3 a reset completed; the words of a step not
    /// reached are 0); `GPU_INT_MASK`, `JOB_INT_MASK`, `MMU_INT_MASK` as
    /// found; the three `*_INT_RAWSTAT` as found and after the clear;
    /// `MCU_STATUS` as found, the polls of the halt wait, 1 when it timed
    /// out, `MCU_STATUS` at its end; 1 when the soft reset timed out and the
    /// hard reset ran; that reset's `GPU_INT_RAWSTAT` before the clear,
    /// after it, and its polls; the three masks and `MCU_STATUS` after the
    /// reset.
    pub take_over_ran: u32,
    pub take_over_steps: u32,
    pub take_over_masks: [u32; 3],
    pub take_over_raw: [u32; 3],
    pub take_over_raw_cleared: [u32; 3],
    pub take_over_mcu: [u32; 4],
    pub take_over_hard: u32,
    pub take_over_reset: [u32; 3],
    pub take_over_after: [u32; 4],
}

#[cfg(any(feature = "mali", test))]
const _: () = assert!(size_of::<YdT6GpuUp>() == 4 * 152);
#[cfg(any(feature = "mali", test))]
const _: () = assert!(offset_of!(YdT6GpuUp, rk806) == 4 * 18);
#[cfg(any(feature = "mali", test))]
const _: () = assert!(offset_of!(YdT6GpuUp, ident) == 4 * 65);
#[cfg(any(feature = "mali", test))]
const _: () = assert!(offset_of!(YdT6GpuUp, stop) == 4 * 108);
#[cfg(any(feature = "mali", test))]
const _: () = assert!(offset_of!(YdT6GpuUp, clock_kept) == 4 * 112);
#[cfg(any(feature = "mali", test))]
const _: () = assert!(offset_of!(YdT6GpuUp, clock_cpll_hz) == 4 * 113);
#[cfg(any(feature = "mali", test))]
const _: () = assert!(offset_of!(YdT6GpuUp, clock_wrote) == 4 * 115);
#[cfg(any(feature = "mali", test))]
const _: () = assert!(offset_of!(YdT6GpuUp, clock_sel) == 4 * 117);
#[cfg(any(feature = "mali", test))]
const _: () = assert!(offset_of!(YdT6GpuUp, clock_set_hz) == 4 * 119);
#[cfg(any(feature = "mali", test))]
const _: () = assert!(offset_of!(YdT6GpuUp, supply_enabled) == 4 * 121);
#[cfg(any(feature = "mali", test))]
const _: () = assert!(offset_of!(YdT6GpuUp, supply_after) == 4 * 122);
#[cfg(any(feature = "mali", test))]
const _: () = assert!(offset_of!(YdT6GpuUp, supply_waited_us) == 4 * 124);
#[cfg(any(feature = "mali", test))]
const _: () = assert!(offset_of!(YdT6GpuUp, qos_policy) == 4 * 125);
#[cfg(any(feature = "mali", test))]
const _: () = assert!(offset_of!(YdT6GpuUp, qos_words) == 4 * 126);
#[cfg(any(feature = "mali", test))]
const _: () = assert!(offset_of!(YdT6GpuUp, take_over_ran) == 4 * 129);
#[cfg(any(feature = "mali", test))]
const _: () = assert!(offset_of!(YdT6GpuUp, take_over_after) == 4 * 148);

#[cfg(any(feature = "mali", test))]
fn lo_hi(v: u64) -> [u32; 2] {
    [v as u32, (v >> 32) as u32]
}

#[cfg(any(feature = "mali", test))]
fn pd_words(r: &rk3588::PdGpuRegs) -> [u32; 6] {
    [r.pwr_gate, r.status, r.idle_req, r.idle_ack, r.idle_st, r.repair]
}

#[cfg(any(feature = "mali", test))]
fn ident_words(i: &crate::gpu::mali::Ident) -> [u32; 16] {
    let [s0, s1] = lo_hi(i.shader_present);
    let [t0, t1] = lo_hi(i.tiler_present);
    let [l0, l1] = lo_hi(i.l2_present);
    [
        i.gpu_id,
        i.l2_features,
        i.tiler_features,
        i.mem_features,
        i.mmu_features,
        i.as_present,
        i.csf_id,
        s0,
        s1,
        t0,
        t1,
        l0,
        l1,
        i.coherency_features,
        i.status,
        i.mcu_status,
    ]
}

#[cfg(any(feature = "mali", test))]
impl From<&crate::board::nanopc_t6_gpu::GpuUp> for YdT6GpuUp {
    fn from(o: &crate::board::nanopc_t6_gpu::GpuUp) -> YdT6GpuUp {
        let c = &o.clock;
        let t = o.take_over.unwrap_or_default();
        YdT6GpuUp {
            pd_before: pd_words(&o.pd_before),
            clock: [c.sel158, c.sel159, c.gate66, c.mode_con0, c.pll_con[0], c.pll_con[1], c.pll_con[2], c.pll_con6],
            gpu_src_hz: lo_hi(c.src_hz),
            spi_gate14: o.spi_gate14,
            spi_opened: o.spi_opened,
            rk806: o.rk806.map(u32::from),
            spi_found: o.spi_found,
            vdd_gpu_uv: o.vdd_gpu_uv,
            pll_on: [u32::from(o.pll_on.was_on), o.pll_on.waited_us, o.pll_on.con6],
            pd_opened: o.pd_on.opened,
            pd_was_on: u32::from(o.pd_on.was_on),
            pd_waited_us: o.pd_on.waited_us,
            pd_after: pd_words(&o.pd_after),
            qos: o.qos,
            ident: ident_words(&o.ident),
            reset: [o.reset.raw_before, o.reset.raw_cleared, o.reset.waited_us],
            ident_after_reset: ident_words(&o.ident_after_reset),
            l2_mask: lo_hi(o.l2.mask),
            l2_ready_before: lo_hi(o.l2.ready_before),
            l2_ready: lo_hi(o.l2.ready),
            l2_waited_us: o.l2.waited_us,
            stop: o.stop as u32,
            refusal: o.refusal,
            detail: lo_hi(o.detail),
            clock_kept: u32::from(o.clock_set.kept),
            clock_cpll_hz: lo_hi(o.clock_set.cpll_hz),
            clock_wrote: o.clock_set.wrote,
            clock_sel: o.clock_set.sel,
            clock_set_hz: lo_hi(o.clock_set.src_hz),
            supply_enabled: u32::from(o.supply_enabled),
            supply_after: o.supply_after.map(u32::from),
            supply_waited_us: o.supply_waited_us,
            qos_policy: o.qos_policy,
            qos_words: [o.qos[0][1], o.qos[0][2], o.qos[0][3]],
            take_over_ran: u32::from(o.take_over.is_some()),
            take_over_steps: t.steps,
            take_over_masks: t.masks,
            take_over_raw: t.raw,
            take_over_raw_cleared: t.raw_cleared,
            take_over_mcu: [t.mcu_status, t.mcu_waited_us, u32::from(t.mcu_halt_timed_out), t.mcu_status_halted],
            take_over_hard: u32::from(t.hard),
            take_over_reset: [t.reset.raw_before, t.reset.raw_cleared, t.reset.waited_us],
            take_over_after: t.after_reset,
        }
    }
}

/// The bases of a C entry that runs Z1 ([`yd_t6_gpu_up`], [`yd_t6_fw_up`],
/// [`yd_t6_submit`], [`yd_t6_osal_up`]), or `None` when the caller mapped
/// fewer than [`crate::board::nanopc_t6_gpu::GPU_MAP_MIN`] bytes of the GPU
/// at `gpu` (`gpu_len`): the take-over of a GPU found powered writes up to
/// its MMU interrupt block, past a shorter map. The entry then answers
/// `-EINVAL` before it touches anything. Each entry gets its bases here and
/// nowhere else (`every_z1_entry_takes_its_bases_from_t6_bases`).
#[cfg(feature = "mali")]
fn t6_bases(
    cru: usize,
    pmu: usize,
    spi2: usize,
    qos: usize,
    gpu: usize,
    gpu_len: usize,
) -> Option<crate::board::nanopc_t6_gpu::Bases> {
    (gpu_len >= crate::board::nanopc_t6_gpu::GPU_MAP_MIN).then_some(crate::board::nanopc_t6_gpu::Bases {
        cru,
        pmu,
        spi2,
        qos,
        gpu,
    })
}

/// The GPU QoS policy [`yd_t6_gpu_up`] applies, as [`rk3588::QosPolicy::id`].
#[cfg(feature = "mali")]
static T6_GPU_QOS_POLICY: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(crate::board::nanopc_t6_gpu::GPU_QOS_POLICY.id());

/// The policy [`T6_GPU_QOS_POLICY`] holds, for Z1, Z3 and Z4 alike.
#[cfg(feature = "mali")]
fn t6_gpu_qos_policy() -> rk3588::QosPolicy {
    rk3588::QosPolicy::from_id(T6_GPU_QOS_POLICY.load(core::sync::atomic::Ordering::Relaxed))
        .unwrap_or(crate::board::nanopc_t6_gpu::GPU_QOS_POLICY)
}

/// Choose the GPU QoS policy `yd_t6_gpu_up`, `yd_t6_fw_up` and `yd_t6_submit` apply after the power-on:
/// `YD_T6_QOS_RESET` (1) or `YD_T6_QOS_LINUX_ZERO` (2, the default). Call it
/// before `yd_t6_gpu_up`; the record names the policy that ran. `-EINVAL`
/// (the choice unchanged) for any other id.
#[cfg(feature = "mali")]
#[no_mangle]
pub extern "C" fn yd_t6_gpu_qos_policy(id: u32) -> i32 {
    if rk3588::QosPolicy::from_id(id).is_none() {
        return -EINVAL;
    }
    T6_GPU_QOS_POLICY.store(id, core::sync::atomic::Ordering::Relaxed);
    0
}

/// The NanoPC-T6's GPU power-on ([`crate::board::nanopc_t6_gpu::gpu_up`]):
/// 0 when every step ran, `-EIO` when one stopped (`out.stop` says which),
/// `-EINVAL` for a null `out`, `delay_us` or `on_handover`, or a `gpu_len`
/// below `YD_T6_GPU_MAP_MIN` ([`t6_bases`]); `on_handover` is
/// called once with the hand-over state, before the first PMU or GPU write. The bases are the shim's
/// `device_map()`s of the CRU (0xfd7c0000, 0x1000), the PMU (0xfd8d8000),
/// SPI2 (0xfeb20000), `qos_gpu_m0` (0xfdf35000, 0x800) and the GPU
/// (0xfb000000, `gpu_len` bytes, at least `YD_T6_GPU_MAP_MIN` =
/// [`crate::board::nanopc_t6_gpu::GPU_MAP_MIN`]: the take-over of a
/// GPU found powered reaches its MMU interrupt block); `delay_us` busy-waits.
///
/// # Safety
/// The bases are mapped device memory covering those blocks, the GPU's for
/// `gpu_len` bytes; `out` is valid.
#[cfg(feature = "mali")]
#[no_mangle]
pub unsafe extern "C" fn yd_t6_gpu_up(
    cru: usize,
    pmu: usize,
    spi2: usize,
    qos: usize,
    gpu: usize,
    gpu_len: usize,
    delay_us: Option<extern "C" fn(u32)>,
    on_handover: Option<extern "C" fn(*const YdT6GpuUp)>,
    out: *mut YdT6GpuUp,
) -> i32 {
    let Some(b) = t6_bases(cru, pmu, spi2, qos, gpu, gpu_len) else { return -EINVAL };
    let (Some(delay_us), Some(on_handover)) = (delay_us, on_handover) else { return -EINVAL };
    if out.is_null() {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    let hw = unsafe { Hw::new() };
    let qos = t6_gpu_qos_policy();
    let o = crate::board::nanopc_t6_gpu::gpu_up_with(qos, &hw, b, &mut |us| delay_us(us), &mut |h| {
        let h = YdT6GpuUp::from(h);
        on_handover(&h);
    });
    // SAFETY: the caller's contract.
    unsafe { *out = YdT6GpuUp::from(&o) };
    if o.stop == crate::board::nanopc_t6_gpu::Stop::None {
        0
    } else {
        -EIO
    }
}

/// `struct yd_t6_mmu_up`: what [`crate::board::nanopc_t6_mmu::mmu_up`]
/// read, in 32-bit words (a 64-bit value is lo, hi). An `as` array is
/// `TRANSTAB`, `MEMATTR`, `TRANSCFG`, `LOCKADDR` (lo, hi each), `STATUS`,
/// `FAULTSTATUS`, `FAULTADDRESS` (lo, hi); an `irq` array is
/// `MMU_INT_RAWSTAT`, `MMU_INT_MASK`, `GPU_INT_RAWSTAT`, `GPU_INT_MASK`; a
/// `flush` array is the `GPU_CMD` written, `GPU_INT_RAWSTAT` before the
/// clear and after it, and the polls; an `op` array is the `AS_LOCKADDR`
/// written (lo, hi), the LOCK polls (before, after), the `flush`, the UNLOCK
/// polls.
#[cfg(any(feature = "mali", test))]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdT6MmuUp {
    pub found: [u32; 12],
    pub found_irq: [u32; 4],
    pub flush: [u32; 4],
    pub tables_pa: [u32; 2],
    pub backing_pa: [u32; 2],
    pub tables_used: u32,
    /// The walk of the probe VA: the VA, the descriptor at each level, the
    /// levels read, the PA it gives (0 when none).
    pub walk_va: [u32; 2],
    pub walk_desc: [[u32; 2]; 4],
    pub walk_levels: u32,
    pub walk_pa: [u32; 2],
    pub clean_rc: i32,
    pub enable_op: [u32; 10],
    /// What the enable wrote: `TRANSTAB`, `MEMATTR`, `TRANSCFG`; the UPDATE
    /// polls (before, after).
    pub enable_regs: [[u32; 2]; 3],
    pub update_polls: [u32; 2],
    pub after: [u32; 12],
    pub after_irq: [u32; 4],
    pub flush_pt: [u32; 10],
    /// `YD_T6_MMU_STOP_*`, the refusal number of that step, and the value it
    /// is about (lo, hi).
    pub stop: u32,
    pub refusal: u32,
    pub detail: [u32; 2],
}

#[cfg(any(feature = "mali", test))]
const _: () = assert!(size_of::<YdT6MmuUp>() == 4 * 87);
#[cfg(any(feature = "mali", test))]
const _: () = assert!(offset_of!(YdT6MmuUp, walk_va) == 4 * 25);
#[cfg(any(feature = "mali", test))]
const _: () = assert!(offset_of!(YdT6MmuUp, enable_op) == 4 * 39);
#[cfg(any(feature = "mali", test))]
const _: () = assert!(offset_of!(YdT6MmuUp, after) == 4 * 57);
#[cfg(any(feature = "mali", test))]
const _: () = assert!(offset_of!(YdT6MmuUp, stop) == 4 * 83);

#[cfg(any(feature = "mali", test))]
fn as_words(r: &crate::gpu::mali::mmu::AsRegs) -> [u32; 12] {
    let [t0, t1] = lo_hi(r.transtab);
    let [m0, m1] = lo_hi(r.memattr);
    let [c0, c1] = lo_hi(r.transcfg);
    let [l0, l1] = lo_hi(r.lockaddr);
    let [f0, f1] = lo_hi(r.faultaddress);
    [t0, t1, m0, m1, c0, c1, l0, l1, r.status, r.faultstatus, f0, f1]
}

#[cfg(any(feature = "mali", test))]
fn flush_words(f: &crate::gpu::mali::Flush) -> [u32; 4] {
    [f.cmd, f.raw_before, f.raw_cleared, f.waited_us]
}

#[cfg(any(feature = "mali", test))]
fn op_words(o: &crate::gpu::mali::mmu::Op) -> [u32; 10] {
    let [l0, l1] = lo_hi(o.lockaddr);
    let [c, b, a, w] = flush_words(&o.flush);
    [l0, l1, o.lock_polls[0], o.lock_polls[1], c, b, a, w, o.unlock_polls[0], o.unlock_polls[1]]
}

#[cfg(any(feature = "mali", test))]
impl From<&crate::board::nanopc_t6_mmu::MmuUp> for YdT6MmuUp {
    fn from(o: &crate::board::nanopc_t6_mmu::MmuUp) -> YdT6MmuUp {
        let e = &o.enable;
        YdT6MmuUp {
            found: as_words(&o.found),
            found_irq: o.found_irq,
            flush: flush_words(&o.flush),
            tables_pa: lo_hi(o.tables_pa),
            backing_pa: lo_hi(o.backing_pa),
            tables_used: o.tables_used,
            walk_va: lo_hi(crate::board::nanopc_t6_mmu::PROBE_VA),
            walk_desc: o.walk.desc.map(lo_hi),
            walk_levels: o.walk.levels,
            walk_pa: lo_hi(o.walk.pa.unwrap_or(0)),
            clean_rc: o.clean_rc,
            enable_op: op_words(&e.flush),
            enable_regs: [lo_hi(e.transtab), lo_hi(e.memattr), lo_hi(e.transcfg)],
            update_polls: e.update_polls,
            after: as_words(&o.after),
            after_irq: o.after_irq,
            flush_pt: op_words(&o.flush_pt),
            stop: o.stop as u32,
            refusal: o.refusal,
            detail: lo_hi(o.detail),
        }
    }
}

/// Z2 on the NanoPC-T6 ([`crate::board::nanopc_t6_mmu::mmu_up`]), after
/// [`yd_t6_gpu_up`] returned 0: 0 when every step ran, `-EIO` when one
/// stopped (`out.stop` says which), `-EINVAL` for a null or misaligned
/// argument. `gpu` maps the GPU (0xfb000000) for at least 0x3000 bytes;
/// `mmu_features` is `ident[4]` of the power-on; `pool` is `pool_pages`
/// 4 KiB pages of the image's memory at physical `tables_pa`, written by
/// this call; `backing_pa` is 64 KiB the firmware section maps to (not
/// touched here); `clean` cleans the data cache over a range to the point
/// of coherency, 0 on success; `delay_us` busy-waits.
///
/// # Safety
/// `gpu` is mapped device memory covering the block; `pool` is valid for
/// writes of `pool_pages` * 4096 bytes and is the memory at `tables_pa`;
/// `out` is valid.
#[cfg(feature = "mali")]
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn yd_t6_mmu_up(
    gpu: usize,
    mmu_features: u32,
    pool: *mut u8,
    pool_pages: u32,
    tables_pa: u64,
    backing_pa: u64,
    clean: Option<extern "C" fn(usize, usize) -> i32>,
    delay_us: Option<extern "C" fn(u32)>,
    out: *mut YdT6MmuUp,
) -> i32 {
    use crate::gpu::mali::pgtable::{Table, PAGE};
    let (Some(clean), Some(delay_us)) = (clean, delay_us) else { return -EINVAL };
    if out.is_null() || pool.is_null() || pool_pages == 0 || !(pool as usize).is_multiple_of(PAGE as usize) {
        return -EINVAL;
    }
    // SAFETY: the caller's contract; 4 KiB aligned covers u64's alignment.
    let pages = unsafe { core::slice::from_raw_parts_mut(pool.cast::<Table>(), pool_pages as usize) };
    // SAFETY: the caller's contract.
    let hw = unsafe { Hw::new() };
    let o = crate::board::nanopc_t6_mmu::mmu_up(
        &hw,
        gpu,
        mmu_features,
        pages,
        tables_pa,
        backing_pa,
        &mut |a, n| clean(a, n),
        &mut |us| delay_us(us),
    );
    // SAFETY: the caller's contract.
    unsafe { *out = YdT6MmuUp::from(&o) };
    if o.stop == crate::board::nanopc_t6_mmu::Stop::None {
        0
    } else {
        -EIO
    }
}

/// `struct yd_t6_fw_backing`: the memory one loaded firmware section maps
/// to ([`crate::board::nanopc_t6_sections::Backing`]): physical address, the
/// CPU's address of it, size in bytes.
#[cfg(any(feature = "mali", test))]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdT6FwBacking {
    pub pa: u64,
    pub cpu: usize,
    pub size: u64,
}

#[cfg(all(any(feature = "mali", test), target_pointer_width = "64"))]
const _: () = assert!(size_of::<YdT6FwBacking>() == 24);

/// Z3's run ([`crate::board::nanopc_t6_fw::fw_up`]) behind the C entry,
/// over any [`Mmio`]: the arguments as [`yd_t6_fw_up`] documents them.
#[cfg(feature = "mali")]
#[allow(clippy::too_many_arguments)]
fn fw_up_run(
    m: &impl Mmio,
    b: crate::board::nanopc_t6_gpu::Bases,
    blob: &[u8],
    backings: &[crate::board::nanopc_t6_sections::Backing],
    pool: &mut [crate::gpu::mali::pgtable::Table],
    tables_pa: u64,
    timer_hz: u32,
    clean: extern "C" fn(usize, usize) -> i32,
    delay_us: extern "C" fn(u32),
    on_handover: extern "C" fn(*const YdT6GpuUp),
    z1: &mut crate::board::nanopc_t6_gpu::GpuUp,
) -> crate::board::nanopc_t6_fw::FwUp {
    crate::board::nanopc_t6_fw::fw_up_with(
        t6_gpu_qos_policy(),
        m,
        b,
        blob,
        backings,
        pool,
        tables_pa,
        timer_hz,
        &|a, n| clean(a, n),
        &mut |us| delay_us(us),
        &mut |h| {
            let h = YdT6GpuUp::from(h);
            on_handover(&h);
        },
        z1,
    )
}

/// Z3 on the NanoPC-T6 ([`crate::board::nanopc_t6_fw::fw_up`]): the
/// firmware image `blob` (`mali_csffw.bin`, `blob_len` bytes) parsed and
/// placed on `backings[0..n_backings]` (one per loaded section, in the
/// file's order), then Z1's power-on, Z2's AS0 over every section, the load
/// with a `clean` of each section, the MCU's start, the global
/// configuration and one `PING`. 0 when every step ran (`out->step` is
/// `YD_T6_FW_STEP_NONE`), `-EIO` when one stopped (`out->step` says which),
/// `-EINVAL` for a null or misaligned argument or a `gpu_len` below
/// `YD_T6_GPU_MAP_MIN`. The bases are those of
/// [`yd_t6_gpu_up`], `gpu` mapped for `gpu_len` = 0x280000 bytes (the doorbells);
/// `pool` is `pool_pages` 4 KiB pages at physical `tables_pa`, written by
/// this call; `timer_hz` is CNTFRQ_EL0; `clean` cleans the data cache over
/// a range to the point of coherency, 0 on success; `on_handover` as
/// [`yd_t6_gpu_up`]'s. `z1` receives Z1's whole record as [`yd_t6_gpu_up`]
/// writes its `out` (all zero when the run stopped before Z1), so a probe
/// can explain a stop at `YD_T6_FW_STEP_GPU`.
///
/// # Safety
/// The bases are mapped device memory; `blob` is valid for `blob_len` bytes
/// and `backings` for `n_backings` entries; each backing's `cpu` is the
/// memory at its `pa` for `size` bytes, writable by this call (the shared
/// section's mapped uncached); `pool` is valid for writes of `pool_pages` *
/// 4096 bytes and is the memory at `tables_pa`; `z1` and `out` are valid.
#[cfg(feature = "mali")]
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn yd_t6_fw_up(
    cru: usize,
    pmu: usize,
    spi2: usize,
    qos: usize,
    gpu: usize,
    gpu_len: usize,
    blob: *const u8,
    blob_len: usize,
    backings: *const YdT6FwBacking,
    n_backings: u32,
    pool: *mut u8,
    pool_pages: u32,
    tables_pa: u64,
    timer_hz: u32,
    clean: Option<extern "C" fn(usize, usize) -> i32>,
    delay_us: Option<extern "C" fn(u32)>,
    on_handover: Option<extern "C" fn(*const YdT6GpuUp)>,
    z1: *mut YdT6GpuUp,
    out: *mut YdT6FwUp,
) -> i32 {
    use crate::board::nanopc_t6_sections::Backing;
    use crate::gpu::mali::pgtable::{Table, PAGE};
    let Some(b) = t6_bases(cru, pmu, spi2, qos, gpu, gpu_len) else { return -EINVAL };
    let (Some(clean), Some(delay_us), Some(on_handover)) = (clean, delay_us, on_handover) else { return -EINVAL };
    if out.is_null()
        || z1.is_null()
        || blob.is_null()
        || backings.is_null()
        || pool.is_null()
        || pool_pages == 0
        || n_backings as usize > crate::gpu::mali::fw::MAX_SECTIONS
        || !(pool as usize).is_multiple_of(PAGE as usize)
    {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    let (blob, given) = unsafe {
        (core::slice::from_raw_parts(blob, blob_len), core::slice::from_raw_parts(backings, n_backings as usize))
    };
    let mut placed = [Backing::default(); crate::gpu::mali::fw::MAX_SECTIONS];
    for (p, k) in placed.iter_mut().zip(given) {
        *p = Backing { pa: k.pa, cpu: k.cpu, size: k.size };
    }
    // SAFETY: the caller's contract; 4 KiB aligned covers u64's alignment.
    let pages = unsafe { core::slice::from_raw_parts_mut(pool.cast::<Table>(), pool_pages as usize) };
    // SAFETY: the caller's contract.
    let hw = unsafe { Hw::new() };
    let mut g = crate::board::nanopc_t6_gpu::GpuUp::default();
    let o = fw_up_run(
        &hw,
        b,
        blob,
        &placed[..given.len()],
        pages,
        tables_pa,
        timer_hz,
        clean,
        delay_us,
        on_handover,
        &mut g,
    );
    // SAFETY: the caller's contract.
    unsafe {
        *z1 = YdT6GpuUp::from(&g);
        *out = o;
    }
    if o.step == crate::board::nanopc_t6_fw::STEP_NONE {
        0
    } else {
        -EIO
    }
}

/// `struct yd_t6_fw_up` is [`crate::board::nanopc_t6_fw::FwUp`] as it is.
#[cfg(any(feature = "mali", test))]
pub type YdT6FwUp = crate::board::nanopc_t6_fw::FwUp;

/// `struct yd_t6_submit_up` is [`crate::board::nanopc_t6_submit::SubmitUp`]
/// as it is.
#[cfg(any(feature = "mali", test))]
pub type YdT6SubmitUp = crate::board::nanopc_t6_submit::SubmitUp;

/// The regions of the group VM `struct yd_t6_submit_mem` backs, in the
/// order of [`crate::board::nanopc_t6_submit::REGIONS`].
#[cfg(any(feature = "mali", test))]
pub const YD_T6_SUBMIT_REGIONS: usize = crate::board::nanopc_t6_submit::REGIONS.len();

/// `struct yd_t6_submit_mem`: the memory Z4 adds to Z3's. `queue` is the
/// queue's AS0 memory (the user pages, then CSG 0's suspend buffers),
/// mapped at `YD_T6_SUBMIT_QUEUE_MCU_VA`, its `cpu` uncached; `regions` is
/// one backing per region of the group VM, in the header's order
/// (`YD_T6_SUBMIT_REGION_*`), each at least its region's size, its `cpu`
/// uncached; `vm_pool` is `vm_pool_pages` 4 KiB pages at physical
/// `vm_tables_pa`, the group VM's tables, written by the call.
#[cfg(any(feature = "mali", test))]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct YdT6SubmitMem {
    pub queue: YdT6FwBacking,
    pub regions: [YdT6FwBacking; YD_T6_SUBMIT_REGIONS],
    pub vm_pool: *mut u8,
    pub vm_pool_pages: u32,
    pub vm_tables_pa: u64,
}

#[cfg(all(any(feature = "mali", test), target_pointer_width = "64"))]
const _: () = assert!(size_of::<YdT6SubmitMem>() == 192);

/// Z4's run ([`crate::board::nanopc_t6_submit::submit_up`]) behind the C
/// entry, over any [`Mmio`]: the arguments as [`yd_t6_submit`] documents
/// them, `mem` and `frame` as `submit_up` takes them.
#[cfg(feature = "mali")]
#[allow(clippy::too_many_arguments)]
fn submit_run(
    m: &impl Mmio,
    b: crate::board::nanopc_t6_gpu::Bases,
    blob: &[u8],
    backings: &[crate::board::nanopc_t6_sections::Backing],
    pool: &mut [crate::gpu::mali::pgtable::Table],
    tables_pa: u64,
    timer_hz: u32,
    mem: crate::board::nanopc_t6_submit::Z4Mem<'_>,
    frame: &crate::board::nanopc_t6_submit::Frame<'_>,
    clean: extern "C" fn(usize, usize) -> i32,
    delay_us: extern "C" fn(u32),
    on_handover: extern "C" fn(*const YdT6GpuUp),
    z1: &mut crate::board::nanopc_t6_gpu::GpuUp,
) -> YdT6SubmitUp {
    crate::board::nanopc_t6_submit::submit_up_with(
        t6_gpu_qos_policy(),
        m,
        b,
        blob,
        backings,
        pool,
        tables_pa,
        timer_hz,
        mem,
        frame,
        &|a, n| clean(a, n),
        &mut |us| delay_us(us),
        &mut |h| {
            let h = YdT6GpuUp::from(h);
            on_handover(&h);
        },
        z1,
    )
}

/// Z4 on the NanoPC-T6 ([`crate::board::nanopc_t6_submit::submit_up`] with
/// the generated frame, `nanopc_t6_submit::GENERATED`): Z3 as
/// [`yd_t6_fw_up`] runs it, with the same arguments, then the queue's
/// memory added to AS0, the group VM built from `mem->regions` and enabled
/// in address space 1, the tiler heap made, CSG 0 / CS 0 started, the
/// frame written at its VAs, one job submitted and waited for (5 s), the
/// group stopped and terminated, the target read back. 0 when every step
/// ran (`out->step` is `YD_T6_SUBMIT_STEP_NONE`), `-EIO` when one stopped
/// (`out->step` says which; Z3's own record is `out->fw`, Z1's is `z1` as
/// [`yd_t6_fw_up`] writes it), `-EINVAL` for a null or misaligned argument
/// or a `gpu_len` below `YD_T6_GPU_MAP_MIN`.
///
/// # Safety
/// As [`yd_t6_fw_up`]'s; and `mem` is valid, each backing in it the memory
/// at its `pa` for `size` bytes, mapped uncached, writable by this call and
/// used by nothing else while it runs, `mem->vm_pool` valid for writes of
/// `vm_pool_pages` * 4096 bytes and the memory at `vm_tables_pa`.
#[cfg(feature = "mali")]
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn yd_t6_submit(
    cru: usize,
    pmu: usize,
    spi2: usize,
    qos: usize,
    gpu: usize,
    gpu_len: usize,
    blob: *const u8,
    blob_len: usize,
    backings: *const YdT6FwBacking,
    n_backings: u32,
    pool: *mut u8,
    pool_pages: u32,
    tables_pa: u64,
    timer_hz: u32,
    mem: *const YdT6SubmitMem,
    clean: Option<extern "C" fn(usize, usize) -> i32>,
    delay_us: Option<extern "C" fn(u32)>,
    on_handover: Option<extern "C" fn(*const YdT6GpuUp)>,
    z1: *mut YdT6GpuUp,
    out: *mut YdT6SubmitUp,
) -> i32 {
    use crate::board::nanopc_t6_sections::Backing;
    use crate::board::nanopc_t6_submit::{GroupBacking, McuBacking, Z4Mem, GENERATED, QUEUE_MCU_VA};
    use crate::gpu::mali::pgtable::{Table, PAGE};
    let Some(b) = t6_bases(cru, pmu, spi2, qos, gpu, gpu_len) else { return -EINVAL };
    let (Some(clean), Some(delay_us), Some(on_handover)) = (clean, delay_us, on_handover) else { return -EINVAL };
    if out.is_null()
        || z1.is_null()
        || blob.is_null()
        || backings.is_null()
        || pool.is_null()
        || mem.is_null()
        || pool_pages == 0
        || n_backings as usize > crate::gpu::mali::fw::MAX_SECTIONS
        || !(pool as usize).is_multiple_of(PAGE as usize)
    {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    let z4 = unsafe { *mem };
    if z4.vm_pool.is_null() || z4.vm_pool_pages == 0 || !(z4.vm_pool as usize).is_multiple_of(PAGE as usize) {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    let (blob, given) = unsafe {
        (core::slice::from_raw_parts(blob, blob_len), core::slice::from_raw_parts(backings, n_backings as usize))
    };
    let mut placed = [Backing::default(); crate::gpu::mali::fw::MAX_SECTIONS];
    for (p, k) in placed.iter_mut().zip(given) {
        *p = Backing { pa: k.pa, cpu: k.cpu, size: k.size };
    }
    // SAFETY: the caller's contract; 4 KiB aligned covers u64's alignment.
    let (pages, vm_pool) = unsafe {
        (
            core::slice::from_raw_parts_mut(pool.cast::<Table>(), pool_pages as usize),
            core::slice::from_raw_parts_mut(z4.vm_pool.cast::<Table>(), z4.vm_pool_pages as usize),
        )
    };
    // SAFETY: the caller's contract: each region's backing is this run's,
    // uncached, the memory at its physical address.
    let vm = z4.regions.map(|r| unsafe { GroupBacking::new(r.pa, r.cpu, r.size) });
    let queue = McuBacking { va: QUEUE_MCU_VA, pa: z4.queue.pa, cpu: z4.queue.cpu, size: z4.queue.size };
    let mem = Z4Mem { queue, vm: &vm, vm_pool, vm_tables_pa: z4.vm_tables_pa };
    // SAFETY: the caller's contract.
    let hw = unsafe { Hw::new() };
    let mut g = crate::board::nanopc_t6_gpu::GpuUp::default();
    let o = submit_run(
        &hw,
        b,
        blob,
        &placed[..given.len()],
        pages,
        tables_pa,
        timer_hz,
        mem,
        &GENERATED,
        clean,
        delay_us,
        on_handover,
        &mut g,
    );
    // SAFETY: the caller's contract.
    unsafe {
        *z1 = YdT6GpuUp::from(&g);
        *out = o;
    }
    if o.step == crate::board::nanopc_t6_submit::STEP_NONE {
        0
    } else {
        -EIO
    }
}

/// `struct yd_t6_osal_mem`: what the Zephyr Mali OSAL
/// (src/osal/zephyr_mali.c) runs on, every backing mapped uncached for the
/// CPU and used by nothing else: `fw` the arena the firmware's sections
/// are placed in ([`crate::board::nanopc_t6_osal::place_sections`]); `pool`
/// AS0's table pages (`pool_pages` 4 KiB pages at physical `tables_pa`);
/// `queue` the queue's AS0 memory (user pages, suspend buffers); `ring`,
/// `heap_pool`, `heap_chunks` and `carve` the group VM's four regions
/// (`YD_T6_OSAL_*_VA`); `vm_pool` the group VM's table pages;
/// `heap_initial_chunks` the tiler heap's chunks at the start (the others
/// of `heap_chunks` are its growth); `carve_reserved` the bytes at
/// `carve`'s end no `yd_t6_osal_alloc` gets (the board's window buffer,
/// reached through `yd_t6_osal_map_phys`): 0, or whole 4 KiB pages that
/// leave at least one page to hand out.
#[cfg(any(feature = "mali", test))]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct YdT6OsalMem {
    pub fw: YdT6FwBacking,
    pub pool: *mut u8,
    pub pool_pages: u32,
    pub tables_pa: u64,
    pub queue: YdT6FwBacking,
    pub ring: YdT6FwBacking,
    pub heap_pool: YdT6FwBacking,
    pub heap_chunks: YdT6FwBacking,
    pub carve: YdT6FwBacking,
    pub vm_pool: *mut u8,
    pub vm_pool_pages: u32,
    pub vm_tables_pa: u64,
    pub heap_initial_chunks: u32,
    pub carve_reserved: u32,
}

#[cfg(all(any(feature = "mali", test), target_pointer_width = "64"))]
const _: () = assert!(size_of::<YdT6OsalMem>() == 200);

/// `struct osal_gpu_mem` (include/gles_osal.h), which this crate cannot
/// import from the library: the same fields at the same offsets.
#[cfg(any(feature = "mali", test))]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OsalGpuMem {
    pub cpu: *mut core::ffi::c_void,
    pub gpu: u64,
    pub phys: u64,
    pub size: usize,
    pub impl_: *mut core::ffi::c_void,
}

#[cfg(all(any(feature = "mali", test), target_pointer_width = "64"))]
const _: () = assert!(size_of::<OsalGpuMem>() == 40);

/// The one OSAL of the image and the delay its waits poll with. The OSAL
/// contract is one thread (include/gles_osal.h, "threads"): the library
/// calls the ops one at a time, so the cell is never reached twice at once.
#[cfg(feature = "mali")]
struct OsalCell(core::cell::UnsafeCell<(crate::board::nanopc_t6_osal::Osal, Option<extern "C" fn(u32)>)>);

// SAFETY: see above; nothing but the yd_t6_osal_* entries touches it.
#[cfg(feature = "mali")]
unsafe impl Sync for OsalCell {}

#[cfg(feature = "mali")]
static OSAL: OsalCell = OsalCell(core::cell::UnsafeCell::new((crate::board::nanopc_t6_osal::Osal::EMPTY, None)));

/// The OSAL and its delay.
///
/// # Safety
/// Called from the yd_t6_osal_* entries only, which the library calls from
/// its one thread, never two at once (the cell's doc).
#[cfg(feature = "mali")]
unsafe fn osal() -> &'static mut (crate::board::nanopc_t6_osal::Osal, Option<extern "C" fn(u32)>) {
    // SAFETY: the caller's contract.
    unsafe { &mut *OSAL.0.get() }
}

#[cfg(feature = "mali")]
fn osal_hw() -> Hw {
    // SAFETY: the addresses the OSAL touches are the GPU's registers (the
    // `gpu` mapping yd_t6_osal_up was given) and the uncached memory of
    // its `struct yd_t6_osal_mem`.
    unsafe { Hw::new() }
}

/// Bring the Mali OSAL up ([`crate::board::nanopc_t6_osal::Osal::up`]):
/// the firmware's sections placed in `mem->fw`, Z1 to Z3 over them and
/// `mem->pool` (the arguments as [`yd_t6_fw_up`] documents them), then the
/// group over `mem`'s regions. 0 when up (or already up), `-EIO` when a
/// step stopped (`out->step` says which, as for [`yd_t6_submit`]; Z1's
/// whole record is `z1`, as [`yd_t6_fw_up`] gives it),
/// `-EINVAL` for a null or misaligned argument, a `gpu_len` below
/// `YD_T6_GPU_MAP_MIN`, a firmware that does not
/// parse or an `mem->fw` too small for its sections. `delay_us` is kept
/// for the waits of the later calls.
///
/// # Safety
/// As [`yd_t6_submit`]'s for the register mappings, `blob`, `clean`,
/// `delay_us`, `on_handover` and `z1`; every backing of `mem` the memory at its
/// `pa` for `size` bytes, mapped uncached, used by this OSAL alone for as
/// long as the image runs, `mem->pool` and `mem->vm_pool` valid for writes
/// of their pages and the memory at `tables_pa` and `vm_tables_pa`. One
/// thread only (include/gles_osal.h).
#[cfg(feature = "mali")]
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn yd_t6_osal_up(
    cru: usize,
    pmu: usize,
    spi2: usize,
    qos: usize,
    gpu: usize,
    gpu_len: usize,
    blob: *const u8,
    blob_len: usize,
    timer_hz: u32,
    mem: *const YdT6OsalMem,
    clean: Option<extern "C" fn(usize, usize) -> i32>,
    delay_us: Option<extern "C" fn(u32)>,
    on_handover: Option<extern "C" fn(*const YdT6GpuUp)>,
    z1: *mut YdT6GpuUp,
    out: *mut YdT6SubmitUp,
) -> i32 {
    use crate::board::nanopc_t6_osal::{place_sections, OsalMem};
    use crate::board::nanopc_t6_sections::Backing;
    use crate::board::nanopc_t6_submit::{GroupBacking, McuBacking, SubmitUp, QUEUE_MCU_VA};
    use crate::gpu::mali::pgtable::{Table, PAGE};
    let Some(b) = t6_bases(cru, pmu, spi2, qos, gpu, gpu_len) else { return -EINVAL };
    let (Some(clean), Some(delay), Some(on_handover)) = (clean, delay_us, on_handover) else { return -EINVAL };
    if out.is_null() || z1.is_null() || blob.is_null() || mem.is_null() {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    let (z, blob) = unsafe { (*mem, core::slice::from_raw_parts(blob, blob_len)) };
    let aligned = |p: *mut u8| !p.is_null() && (p as usize).is_multiple_of(PAGE as usize);
    if !aligned(z.pool) || !aligned(z.vm_pool) || z.pool_pages == 0 || z.vm_pool_pages == 0 {
        return -EINVAL;
    }
    let arena = Backing { pa: z.fw.pa, cpu: z.fw.cpu, size: z.fw.size };
    let Some((backings, n)) = place_sections(blob, arena) else { return -EINVAL };
    // SAFETY: the caller's contract; 4 KiB aligned covers u64's alignment.
    let (pool, vm_pool) = unsafe {
        (
            core::slice::from_raw_parts_mut(z.pool.cast::<Table>(), z.pool_pages as usize),
            core::slice::from_raw_parts_mut(z.vm_pool.cast::<Table>(), z.vm_pool_pages as usize),
        )
    };
    // SAFETY: the caller's contract: each backing the OSAL's alone,
    // uncached, the memory at its physical address.
    let back = |b: YdT6FwBacking| unsafe { GroupBacking::new(b.pa, b.cpu, b.size) };
    let mem = OsalMem {
        queue: McuBacking { va: QUEUE_MCU_VA, pa: z.queue.pa, cpu: z.queue.cpu, size: z.queue.size },
        ring: back(z.ring),
        heap_pool: back(z.heap_pool),
        heap_chunks: back(z.heap_chunks),
        carve: back(z.carve),
        vm_pool,
        vm_tables_pa: z.vm_tables_pa,
        heap_initial_chunks: z.heap_initial_chunks,
        carve_reserved: u64::from(z.carve_reserved),
    };
    // SAFETY: one thread (the contract).
    let s = unsafe { osal() };
    s.1 = Some(delay);
    let mut o = SubmitUp::default();
    let mut g = crate::board::nanopc_t6_gpu::GpuUp::default();
    let r = s.0.up(
        &osal_hw(),
        b,
        blob,
        &backings[..n],
        pool,
        z.tables_pa,
        timer_hz,
        mem,
        &|a, n| clean(a, n),
        &mut |us| delay(us),
        &mut |h| {
            let h = YdT6GpuUp::from(h);
            on_handover(&h);
        },
        &mut g,
        &mut o,
    );
    // SAFETY: the caller's contract.
    unsafe {
        *z1 = YdT6GpuUp::from(&g);
        *out = o;
    }
    if r.is_ok() {
        0
    } else {
        -EIO
    }
}

#[cfg(feature = "mali")]
fn fill_mem(out: &mut OsalGpuMem, a: &crate::board::nanopc_t6_osal::Alloc) {
    *out = OsalGpuMem {
        cpu: a.cpu as *mut core::ffi::c_void,
        gpu: a.gpu,
        phys: a.phys,
        size: a.size,
        impl_: a.handle as *mut core::ffi::c_void,
    };
}

/// `gpu_alloc` ([`crate::board::nanopc_t6_osal::Osal::alloc_reclaiming`]): 0 and
/// `*out`, or a negative errno.
///
/// # Safety
/// `out` is valid for a write; after [`yd_t6_osal_up`], one thread only.
#[cfg(feature = "mali")]
#[no_mangle]
pub unsafe extern "C" fn yd_t6_osal_alloc(size: usize, out: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller's contract.
    let (s, out) = unsafe { (osal(), out.as_mut()) };
    let Some(out) = out else { return -EINVAL };
    let Some(delay) = s.1 else { return -EINVAL };
    match s.0.alloc_reclaiming(&osal_hw(), size, &mut |us| delay(us)) {
        Ok(a) => {
            fill_mem(out, &a);
            0
        }
        Err(e) => e,
    }
}

/// `OSAL_QUERY_MALI_FRAME_OPEN`
/// ([`crate::board::nanopc_t6_osal::Osal::frame_open`]): whether the
/// backend records a frame.
///
/// # Safety
/// After [`yd_t6_osal_up`], one thread only.
#[cfg(feature = "mali")]
#[no_mangle]
pub unsafe extern "C" fn yd_t6_osal_frame_open(open: bool) {
    // SAFETY: the caller's contract.
    unsafe { osal() }.0.frame_open(open);
}

/// `gpu_free` ([`crate::board::nanopc_t6_osal::Osal::free`]): `*mem`
/// cleared either way; 0 for an empty one.
///
/// # Safety
/// As [`yd_t6_osal_alloc`]; `mem` a descriptor this OSAL filled.
#[cfg(feature = "mali")]
#[no_mangle]
pub unsafe extern "C" fn yd_t6_osal_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller's contract.
    let (s, mem) = unsafe { (osal(), mem.as_mut()) };
    let Some(m) = mem else { return -EINVAL };
    let Some(delay) = s.1 else { return -EINVAL };
    let rc = if m.impl_.is_null() { 0 } else { s.0.free(&osal_hw(), m.gpu, m.impl_ as usize, &mut |us| delay(us)) };
    *m = OsalGpuMem { cpu: core::ptr::null_mut(), gpu: 0, phys: 0, size: 0, impl_: core::ptr::null_mut() };
    rc
}

/// `map_phys` ([`crate::board::nanopc_t6_osal::Osal::window`]): 0 and
/// `*out`, its `impl` the window's handle
/// ([`crate::board::nanopc_t6_osal::WINDOW`]: non-null as the GL library
/// needs it, and [`yd_t6_osal_free`] frees nothing for it), or a negative
/// errno.
///
/// # Safety
/// As [`yd_t6_osal_alloc`].
#[cfg(feature = "mali")]
#[no_mangle]
pub unsafe extern "C" fn yd_t6_osal_map_phys(phys: u64, size: usize, out: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller's contract.
    let (s, out) = unsafe { (osal(), out.as_mut()) };
    let Some(out) = out else { return -EINVAL };
    match s.0.window(phys, size) {
        Ok(a) => {
            fill_mem(out, &a);
            0
        }
        Err(e) => e,
    }
}

/// `submit` of an `osal_mali_kick` ([`crate::board::nanopc_t6_osal::Osal::submit`]):
/// the fence, or a negative errno.
///
/// # Safety
/// After [`yd_t6_osal_up`], one thread only.
#[cfg(feature = "mali")]
#[no_mangle]
pub unsafe extern "C" fn yd_t6_osal_submit(stream_gpu: u64, stream_bytes: u32) -> i64 {
    // SAFETY: the caller's contract.
    let s = unsafe { osal() };
    let Some(delay) = s.1 else { return -i64::from(EINVAL) };
    s.0.submit(&osal_hw(), stream_gpu, stream_bytes, &mut |us| delay(us))
}

/// `wait` ([`crate::board::nanopc_t6_osal::Osal::wait`]): 0, 1 on a
/// timeout, or a negative errno.
///
/// # Safety
/// As [`yd_t6_osal_submit`].
#[cfg(feature = "mali")]
#[no_mangle]
pub unsafe extern "C" fn yd_t6_osal_wait(fence: i64, timeout_us: i64) -> i32 {
    // SAFETY: the caller's contract.
    let s = unsafe { osal() };
    let Some(delay) = s.1 else { return -EINVAL };
    s.0.wait(&osal_hw(), fence, timeout_us, &mut |us| delay(us))
}

/// `completed` ([`crate::board::nanopc_t6_osal::Osal::completed`]).
///
/// # Safety
/// As [`yd_t6_osal_submit`].
#[cfg(feature = "mali")]
#[no_mangle]
pub unsafe extern "C" fn yd_t6_osal_completed() -> i64 {
    // SAFETY: the caller's contract.
    let s = unsafe { osal() };
    let Some(delay) = s.1 else { return -i64::from(EINVAL) };
    s.0.completed(&osal_hw(), &mut |us| delay(us))
}

/// `shutdown` ([`crate::board::nanopc_t6_osal::Osal::shutdown`]): its
/// [`crate::board::nanopc_t6_osal::Shutdown::code`] (0, `-EIO` or
/// `-ETIMEDOUT`), or `-EINVAL` when there was nothing to end (not up,
/// already shut down, or dead).
///
/// # Safety
/// As [`yd_t6_osal_submit`].
#[cfg(feature = "mali")]
#[no_mangle]
pub unsafe extern "C" fn yd_t6_osal_shutdown() -> i32 {
    // SAFETY: the caller's contract.
    let s = unsafe { osal() };
    let Some(delay) = s.1 else { return -EINVAL };
    s.0.shutdown(&osal_hw(), &mut |us| delay(us)).map_or(-EINVAL, |d| d.code())
}

/// `OSAL_QUERY_MALI_GPU`'s answer into `*out` (`struct osal_mali_gpu`):
/// 0, or `-EINVAL` before [`yd_t6_osal_up`] or for a null `out`.
///
/// # Safety
/// `out` null or valid for a write; one thread only.
#[cfg(feature = "mali")]
#[no_mangle]
pub unsafe extern "C" fn yd_t6_osal_gpu(out: *mut crate::board::nanopc_t6_osal::OsalMaliGpu) -> i32 {
    // SAFETY: the caller's contract.
    let (s, out) = unsafe { (osal(), out.as_mut()) };
    match (s.0.gpu_info(), out) {
        (Some(g), Some(out)) => {
            *out = g;
            0
        }
        _ => -EINVAL,
    }
}

/// `OSAL_QUERY_MALI_TILER_HEAP`'s answer into `*out` (`struct
/// osal_mali_heap`): 0, or `-EINVAL` as [`yd_t6_osal_gpu`].
///
/// # Safety
/// As [`yd_t6_osal_gpu`].
#[cfg(feature = "mali")]
#[no_mangle]
pub unsafe extern "C" fn yd_t6_osal_heap(out: *mut crate::gpu::mali::heap::OsalMaliHeap) -> i32 {
    // SAFETY: the caller's contract.
    let (s, out) = unsafe { (osal(), out.as_mut()) };
    match (s.0.heap(), out) {
        (Some(h), Some(out)) => {
            *out = h;
            0
        }
        _ => -EINVAL,
    }
}

/// The OSAL's counts for its log: kicks submitted and completed, pages in
/// use now and at most, frees held and refused; the sched refusal that
/// stopped it (`dead_refusal` 0 for none) with its value; the group's
/// restarts and the refusal that caused the last (0 for none) with its
/// value; tiler OOMs answered with a chunk and with none, and the heap's
/// chunks now. 0, or `-EINVAL` for a null `out`.
///
/// # Safety
/// As [`yd_t6_osal_gpu`].
#[cfg(feature = "mali")]
#[no_mangle]
pub unsafe extern "C" fn yd_t6_osal_counts(out: *mut YdT6OsalCounts) -> i32 {
    // SAFETY: the caller's contract.
    let (s, out) = unsafe { (osal(), out.as_mut()) };
    let Some(out) = out else { return -EINVAL };
    let o = &s.0;
    let (submitted, completed) = o.fences();
    let (dead_refusal, dead_value) = o.dead().map_or((0, 0), |(r, v)| (r as u32, v));
    let (last_refusal, last_value) = o.last_error().map_or((0, 0), |(r, v)| (r as u32, v));
    *out = YdT6OsalCounts {
        submitted,
        completed,
        live_pages: o.live_pages as u32,
        live_pages_high: o.live_pages_high as u32,
        held: o.held() as u32,
        refused_frees: o.refused_frees,
        dead_refusal,
        dead_value,
        restarts: o.restarts,
        last_refusal,
        last_value,
        oom_chunks: o.oom_chunks,
        oom_no_chunk: o.oom_no_chunk,
        heap_chunks: o.heap_chunks().len() as u32,
        unended: u32::from(o.unended()),
        down: u32::from(o.is_down()),
        memory_kept: u32::from(o.memory_kept()),
        stops_refused: o.stops_refused,
    };
    0
}

/// `struct yd_t6_osal_counts` ([`yd_t6_osal_counts`]).
#[cfg(any(feature = "mali", test))]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdT6OsalCounts {
    pub submitted: u64,
    pub completed: u64,
    pub live_pages: u32,
    pub live_pages_high: u32,
    pub held: u32,
    pub refused_frees: u32,
    pub dead_refusal: u32,
    pub dead_value: u32,
    pub restarts: u32,
    pub last_refusal: u32,
    pub last_value: u32,
    pub oom_chunks: u32,
    pub oom_no_chunk: u32,
    pub heap_chunks: u32,
    pub unended: u32,
    pub down: u32,
    pub memory_kept: u32,
    pub stops_refused: u32,
}

#[cfg(any(feature = "mali", test))]
const _: () = assert!(size_of::<YdT6OsalCounts>() == 80);

#[cfg(all(test, feature = "mali"))]
mod fw_tests {
    use super::*;
    use crate::board::nanopc_t6_fw::tests::{backings, B, T6, TABLES_PA};
    use crate::board::nanopc_t6_fw::{self as fwb, FwUp};
    use crate::gpu::mali::csf::RK3588_TIMER_HZ;
    use crate::gpu::mali::fw::synth;
    use crate::gpu::mali::pgtable::ENTRIES;

    extern "C" fn clean_ok(_: usize, _: usize) -> i32 {
        0
    }
    thread_local! {
        static MODEL: core::cell::Cell<*const T6> = const { core::cell::Cell::new(core::ptr::null()) };
    }
    /// The model's clean: it boots only from code it saw cleaned.
    extern "C" fn clean_model(a: usize, n: usize) -> i32 {
        // SAFETY: set by the test below to a T6 that outlives the run.
        MODEL.with(|m| unsafe { &*m.get() }.clean(a, n))
    }
    extern "C" fn no_delay(_: u32) {}
    extern "C" fn no_handover(_: *const YdT6GpuUp) {}

    #[test]
    fn the_c_header_has_the_z3_numbers_and_layout() {
        const H: &str = include_str!("../../include/yakogl_drivers.h");
        let def = |name: &str| -> u32 {
            let l = H.lines().find(|l| l.starts_with(&format!("#define {name} "))).expect(name);
            l.split_whitespace().nth(2).expect(name).parse().unwrap_or_else(|_| panic!("{name}"))
        };
        for (name, v) in [
            ("YD_T6_FW_STEP_NONE", fwb::STEP_NONE),
            ("YD_T6_FW_STEP_PARSE", fwb::STEP_PARSE),
            ("YD_T6_FW_STEP_PLACE", fwb::STEP_PLACE),
            ("YD_T6_FW_STEP_GPU", fwb::STEP_GPU),
            ("YD_T6_FW_STEP_MMU", fwb::STEP_MMU),
            ("YD_T6_FW_STEP_BOOT", fwb::STEP_BOOT),
            ("YD_T6_FW_STEP_CLEAN", fwb::STEP_CLEAN),
            ("YD_T6_FW_STEP_VERSION", fwb::STEP_VERSION),
        ] {
            assert_eq!(def(name), v, "{name}");
        }
        let pins = [
            ("sizeof(struct yd_t6_fw_backing)", size_of::<YdT6FwBacking>()),
            ("offsetof(struct yd_t6_fw_backing, cpu)", offset_of!(YdT6FwBacking, cpu)),
            ("offsetof(struct yd_t6_fw_backing, size)", offset_of!(YdT6FwBacking, size)),
            ("sizeof(struct yd_t6_fw_up)", size_of::<FwUp>()),
            ("offsetof(struct yd_t6_fw_up, image_version)", offset_of!(FwUp, image_version)),
            ("offsetof(struct yd_t6_fw_up, gpu_id)", offset_of!(FwUp, gpu_id)),
            ("offsetof(struct yd_t6_fw_up, tables_used)", offset_of!(FwUp, tables_used)),
            ("offsetof(struct yd_t6_fw_up, clean_rc)", offset_of!(FwUp, clean_rc)),
            ("offsetof(struct yd_t6_fw_up, version)", offset_of!(FwUp, version)),
            ("offsetof(struct yd_t6_fw_up, csg0)", offset_of!(FwUp, csg0)),
            ("offsetof(struct yd_t6_fw_up, core_en_mask)", offset_of!(FwUp, core_en_mask)),
            ("offsetof(struct yd_t6_fw_up, acked)", offset_of!(FwUp, acked)),
            ("offsetof(struct yd_t6_fw_up, glb_ack)", offset_of!(FwUp, glb_ack)),
            ("offsetof(struct yd_t6_fw_up, supply_enabled)", offset_of!(FwUp, supply_enabled)),
            ("offsetof(struct yd_t6_fw_up, supply_after)", offset_of!(FwUp, supply_after)),
            ("offsetof(struct yd_t6_fw_up, supply_waited_us)", offset_of!(FwUp, supply_waited_us)),
        ];
        for (what, v) in pins {
            let pin = format!("_Static_assert({what} == {v},");
            assert!(H.contains(&pin), "{pin}");
        }
    }

    #[test]
    fn the_entry_runs_the_mcu_model_through_to_the_record() {
        let blob = synth::small();
        let t6 = T6::handed_over(blob.clone());
        let bk = backings(&blob);
        let mut pool = vec![[0u64; ENTRIES]; 16];
        MODEL.with(|m| m.set(&t6));
        let o = fw_up_run(
            &t6,
            B,
            &blob,
            &bk,
            &mut pool,
            TABLES_PA,
            RK3588_TIMER_HZ,
            clean_model,
            no_delay,
            no_handover,
            &mut Default::default(),
        );
        assert_eq!((o.step, o.stop, o.refusal, o.detail), (fwb::STEP_NONE, 0, 0, [0, 0]));
        assert_eq!((o.version, o.image_version, o.sections), (0x0105_0000, 0x0105_0000, 4));
        assert_eq!((o.ping_acked, o.mcu_status, o.tables_used), (1, 1, 7));
        assert_eq!((o.gpu_id, o.flushed, o.clean_rc), (0xa867_0005, 4, 0));
        assert_eq!((o.glb_input, o.glb_output), (0x0400_2000, 0x0400_6000));
        // the backings' order is the file's: a short list is the placement's refusal
        let o = fw_up_run(
            &t6,
            B,
            &blob,
            &bk[..3],
            &mut pool,
            TABLES_PA,
            RK3588_TIMER_HZ,
            clean_ok,
            no_delay,
            no_handover,
            &mut Default::default(),
        );
        assert_eq!((o.step, o.detail), (fwb::STEP_PLACE, [3, 0]));
    }

    #[test]
    fn the_entry_applies_the_build_time_qos_policy_z1_reads() {
        let blob = synth::small();
        let bk = backings(&blob);
        let before = T6_GPU_QOS_POLICY.load(core::sync::atomic::Ordering::Relaxed);
        for p in [rk3588::QosPolicy::Reset, rk3588::QosPolicy::LinuxZero] {
            assert_eq!(yd_t6_gpu_qos_policy(p.id()), 0);
            let t6 = T6::handed_over(blob.clone());
            let mut pool = vec![[0u64; ENTRIES]; 16];
            let mut z1 = crate::board::nanopc_t6_gpu::GpuUp::default();
            MODEL.with(|m| m.set(&t6));
            let o = fw_up_run(
                &t6,
                B,
                &blob,
                &bk,
                &mut pool,
                TABLES_PA,
                RK3588_TIMER_HZ,
                clean_model,
                no_delay,
                no_handover,
                &mut z1,
            );
            assert_eq!(o.step, fwb::STEP_NONE, "{p:?}");
            assert_eq!(z1.qos_policy, p.id());
            assert_eq!(YdT6GpuUp::from(&z1).qos_policy, p.id());
        }
        yd_t6_gpu_qos_policy(before);
    }

    #[test]
    fn the_entry_refuses_null_and_misaligned_arguments() {
        let mut out = FwUp::default();
        let mut g = YdT6GpuUp::default();
        let z1: *mut YdT6GpuUp = &mut g;
        let mut pool = vec![0u8; 8192];
        let bk = [YdT6FwBacking::default()];
        let blob = [0u8; 4];
        let off = pool.as_mut_ptr().align_offset(4096);
        let call = |gpu_len: usize,
                    blob: *const u8,
                    pool: *mut u8,
                    clean: Option<extern "C" fn(usize, usize) -> i32>,
                    out: *mut FwUp| unsafe {
            yd_t6_fw_up(
                0,
                0,
                0,
                0,
                0,
                gpu_len,
                blob,
                4,
                bk.as_ptr(),
                1,
                pool,
                1,
                0,
                1,
                clean,
                Some(no_delay),
                Some(no_handover),
                z1,
                out,
            )
        };
        let p = unsafe { pool.as_mut_ptr().add(off) };
        const MAP: usize = crate::board::nanopc_t6_gpu::GPU_MAP_MIN;
        assert_eq!(call(MAP, blob.as_ptr(), p, Some(clean_ok), core::ptr::null_mut()), -EINVAL);
        assert_eq!(call(MAP, core::ptr::null(), p, Some(clean_ok), &mut out), -EINVAL);
        assert_eq!(call(MAP, blob.as_ptr(), p, None, &mut out), -EINVAL);
        assert_eq!(call(MAP, blob.as_ptr(), unsafe { p.add(8) }, Some(clean_ok), &mut out), -EINVAL);
        // every argument in order: the run starts and refuses the 4-byte
        // blob (STEP_PARSE) before it touches anything; so the same call
        // with one byte less of the GPU mapped is refused by the map alone
        assert_eq!(call(MAP, blob.as_ptr(), p, Some(clean_ok), &mut out), -EIO);
        assert_eq!(out.step, fwb::STEP_PARSE);
        assert_eq!(call(MAP - 1, blob.as_ptr(), p, Some(clean_ok), &mut out), -EINVAL);
    }

    #[test]
    fn t6_bases_refuses_a_gpu_map_below_gpu_map_min() {
        use crate::board::nanopc_t6_gpu::GPU_MAP_MIN;
        assert!(t6_bases(1, 2, 3, 4, 5, 0).is_none());
        assert!(t6_bases(1, 2, 3, 4, 5, GPU_MAP_MIN - 1).is_none());
        let b = t6_bases(1, 2, 3, 4, 5, GPU_MAP_MIN).expect("GPU_MAP_MIN is enough");
        assert_eq!((b.cru, b.pmu, b.spi2, b.qos, b.gpu), (1, 2, 3, 4, 5));
    }

    /// Every C entry here that runs Z1 (one that takes `on_handover`) takes
    /// the GPU map's length right after `gpu`, gets its bases from
    /// [`t6_bases`] before anything else, and the header declares the
    /// length; nothing else in this file builds the bases. An entry added
    /// later is found by the same rule, not from a list.
    #[test]
    fn every_z1_entry_takes_its_bases_from_t6_bases() {
        const SRC: &str = include_str!("rk3588.rs");
        const H: &str = include_str!("../../include/yakogl_drivers.h");
        let entry = ["pub unsafe extern ", "\"C\" fn "].concat();
        let first = "let Some(b) = t6_bases(cru, pmu, spi2, qos, gpu, gpu_len) else { return -EINVAL };";
        let mut names = vec![];
        for chunk in SRC.split(entry.as_str()).skip(1) {
            let (sig, body) = chunk.split_once(" {\n").expect("a signature");
            if !sig.contains("on_handover: Option<") {
                continue;
            }
            let name = &sig[..sig.find('(').expect("a name")];
            assert!(sig.contains("    gpu: usize,\n    gpu_len: usize,\n"), "{name}: no gpu_len after gpu");
            let body = &body[..body.find("\n}\n").expect("an end")];
            let code: Vec<&str> =
                body.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("//")).collect();
            let at = code.iter().position(|l| *l == first);
            // only `use` lines before it
            assert!(at.is_some_and(|i| code[..i].iter().all(|l| l.starts_with("use "))), "{name}: {code:?}");
            let decl = &H[H.find(&format!("int {name}(")).expect(name)..];
            let decl = &decl[..decl.find(");").expect(name)];
            assert!(decl.contains("uintptr_t gpu,") && decl.contains("size_t gpu_len,"), "{name}: {decl}");
            names.push(name);
        }
        assert!(names.contains(&"yd_t6_gpu_up") && names.len() > 1, "{names:?}");
        assert_eq!(SRC.matches(["nanopc_t6_gpu::Bases", " {"].concat().as_str()).count(), 1);
    }

    /// A header value: a decimal or `0x` hex literal, with or without a
    /// `u` / `ull` suffix.
    fn def64(name: &str) -> u64 {
        const H: &str = include_str!("../../include/yakogl_drivers.h");
        let l = H.lines().find(|l| l.starts_with(&format!("#define {name} "))).expect(name);
        let v = l.split_whitespace().nth(2).expect(name).trim_end_matches("ull").trim_end_matches('u');
        let parsed = match v.strip_prefix("0x") {
            Some(h) => u64::from_str_radix(h, 16),
            None => v.parse(),
        };
        parsed.unwrap_or_else(|_| panic!("{name}: {v}"))
    }

    #[test]
    fn the_c_header_has_the_z4_numbers_and_layout() {
        use crate::board::nanopc_t6_frame as gen;
        use crate::board::nanopc_t6_submit::{self as sub, FrameRefusal, SubmitUp};
        const H: &str = include_str!("../../include/yakogl_drivers.h");
        for (name, v) in [
            ("YD_T6_SUBMIT_STEP_NONE", sub::STEP_NONE),
            ("YD_T6_SUBMIT_STEP_FRAME", sub::STEP_FRAME),
            ("YD_T6_SUBMIT_STEP_FW", sub::STEP_FW),
            ("YD_T6_SUBMIT_STEP_IFACE", sub::STEP_IFACE),
            ("YD_T6_SUBMIT_STEP_IDENT", sub::STEP_IDENT),
            ("YD_T6_SUBMIT_STEP_QUEUE_MEM", sub::STEP_QUEUE_MEM),
            ("YD_T6_SUBMIT_STEP_AS0_MAP", sub::STEP_AS0_MAP),
            ("YD_T6_SUBMIT_STEP_CLEAN", sub::STEP_CLEAN),
            ("YD_T6_SUBMIT_STEP_AS0_FLUSH", sub::STEP_AS0_FLUSH),
            ("YD_T6_SUBMIT_STEP_VM_MAP", sub::STEP_VM_MAP),
            ("YD_T6_SUBMIT_STEP_VM_ENABLE", sub::STEP_VM_ENABLE),
            ("YD_T6_SUBMIT_STEP_HEAP", sub::STEP_HEAP),
            ("YD_T6_SUBMIT_STEP_START", sub::STEP_START),
            ("YD_T6_SUBMIT_STEP_SUBMIT", sub::STEP_SUBMIT),
            ("YD_T6_SUBMIT_STEP_WAIT", sub::STEP_WAIT),
            ("YD_T6_SUBMIT_STEP_STOP", sub::STEP_STOP),
            ("YD_T6_SUBMIT_STEP_TERMINATE", sub::STEP_TERMINATE),
            ("YD_T6_SUBMIT_STEP_DRAIN", sub::STEP_DRAIN),
            ("YD_T6_FRAME_RANGE", FrameRefusal::Range as u32),
            ("YD_T6_FRAME_MEMORY", FrameRefusal::Memory as u32),
            ("YD_T6_FRAME_EMPTY", FrameRefusal::Empty as u32),
            ("YD_T6_FRAME_UNALIGNED", FrameRefusal::Unaligned as u32),
            ("YD_T6_FRAME_OUTSIDE", FrameRefusal::Outside as u32),
            ("YD_T6_FRAME_OVERLAP", FrameRefusal::Overlap as u32),
            ("YD_T6_FRAME_TARGET", FrameRefusal::Target as u32),
            ("YD_T6_FRAME_MAPPING", FrameRefusal::Mapping as u32),
            ("YD_T6_SUBMIT_QUEUE_MCU_VA", sub::QUEUE_MCU_VA),
            ("YD_T6_SUBMIT_REGIONS", sub::REGIONS.len() as u32),
            ("YD_T6_SUBMIT_REGION_QUEUE", sub::REGION_QUEUE as u32),
            ("YD_T6_SUBMIT_REGION_ARENAS", sub::REGION_ARENAS as u32),
            ("YD_T6_SUBMIT_REGION_GEOMETRY", sub::REGION_GEOMETRY as u32),
            ("YD_T6_SUBMIT_REGION_TARGET", sub::REGION_TARGET as u32),
            ("YD_T6_SUBMIT_REGION_HEAP_POOL", sub::REGION_HEAP_POOL as u32),
            ("YD_T6_SUBMIT_REGION_HEAP_CHUNK", sub::REGION_HEAP_CHUNK as u32),
            ("YD_T6_SUBMIT_CS_WORDS", sub::CS.len() as u32),
            ("YD_T6_SUBMIT_WIDTH", sub::WIDTH),
            ("YD_T6_SUBMIT_HEIGHT", sub::HEIGHT),
        ] {
            assert_eq!(def64(name), u64::from(v), "{name}");
        }
        assert_eq!(def64("YD_T6_SUBMIT_QUEUE_MCU_MIN"), sub::QUEUE_MCU_MIN);
        assert_eq!(def64("YD_T6_SUBMIT_CS_VA"), sub::CS_VA);
        assert_eq!(def64("YD_T6_SUBMIT_DIGEST"), sub::DIGEST);
        // each region's VA and least size, in the header's order
        let names = ["QUEUE", "ARENAS", "GEOMETRY", "TARGET", "HEAP_POOL", "HEAP_CHUNK"];
        for (r, n) in sub::REGIONS.iter().zip(names) {
            assert_eq!(def64(&format!("YD_T6_SUBMIT_{n}_VA")), r.va, "{n}");
            assert_eq!(def64(&format!("YD_T6_SUBMIT_{n}_SIZE")), r.size, "{n}");
        }
        assert_eq!(
            (def64("YD_T6_SUBMIT_HEAP_POOL_VA"), def64("YD_T6_SUBMIT_HEAP_CHUNK_VA")),
            (sub::HEAP_CTX_VA, gen::HEAP_FIRST_CHUNK_VA)
        );
        assert_eq!((def64("YD_T6_SUBMIT_TARGET_VA"), def64("YD_T6_SUBMIT_ARENAS_VA")), (gen::TARGET_VA, gen::CTX_VA));

        // the layouts: a _Static_assert per field of both structs and their sizes
        let mut pins = vec![
            ("sizeof(struct yd_t6_submit_mem)".to_string(), size_of::<YdT6SubmitMem>()),
            ("offsetof(struct yd_t6_submit_mem, queue)".into(), offset_of!(YdT6SubmitMem, queue)),
            ("offsetof(struct yd_t6_submit_mem, regions)".into(), offset_of!(YdT6SubmitMem, regions)),
            ("offsetof(struct yd_t6_submit_mem, vm_pool)".into(), offset_of!(YdT6SubmitMem, vm_pool)),
            ("offsetof(struct yd_t6_submit_mem, vm_pool_pages)".into(), offset_of!(YdT6SubmitMem, vm_pool_pages)),
            ("offsetof(struct yd_t6_submit_mem, vm_tables_pa)".into(), offset_of!(YdT6SubmitMem, vm_tables_pa)),
            ("sizeof(struct yd_t6_submit_up)".into(), size_of::<SubmitUp>()),
        ];
        macro_rules! field {
            ($($f:ident),*) => { $( pins.push((
                format!("offsetof(struct yd_t6_submit_up, {})", stringify!($f)), offset_of!(SubmitUp, $f))); )* };
        }
        field!(
            step,
            refusal,
            detail,
            fw,
            as_present,
            l2_features,
            as0_tables_used,
            vm_tables_used,
            suspend_size,
            protm_suspend_size,
            heap_ctx,
            heap_chunk,
            heap_chunk_size,
            heap_stride,
            csg_req,
            csg_ack,
            cs_req,
            cs_ack,
            csg_ack_us,
            cs_ack_us,
            latest_flush,
            sync_updates,
            insert,
            extract_at_done,
            seqno,
            sync_status,
            wait_us,
            status_cmd_ptr,
            info,
            stop_us,
            stop_refusal,
            terminate_us,
            terminate_refusal,
            cleared,
            red,
            blue,
            fill,
            other,
            pixels,
            digest,
            extract_drained,
            extract_at_end,
            drain_us,
            drain_refusal,
            drain_sync_updates,
            drain_cs_ack,
            drain_cs_irq_acks,
            drain_idle_acks
        );
        assert_eq!(pins.len(), 7 + 48);
        for (what, v) in &pins {
            let pin = format!("_Static_assert({what} == {v},");
            assert!(H.contains(&pin), "{pin}");
        }
        // one assert per field and one for the size, each naming yd_t6_submit
        let n = H.lines().filter(|l| l.starts_with("_Static_assert(") && l.contains("yd_t6_submit")).count();
        assert!(n >= 49, "{n} _Static_assert lines name yd_t6_submit");
    }

    #[test]
    fn the_c_header_has_the_osal_numbers_and_layout() {
        use crate::board::nanopc_t6_osal as osal;
        const H: &str = include_str!("../../include/yakogl_drivers.h");
        let def64 = |name: &str| -> u64 {
            let l = H.lines().find(|l| l.starts_with(&format!("#define {name} "))).expect(name);
            let v = l.split_whitespace().nth(2).expect(name);
            let v = v.trim_end_matches("ull").trim_end_matches('u');
            match v.strip_prefix("0x") {
                Some(h) => u64::from_str_radix(h, 16),
                None => v.parse(),
            }
            .unwrap_or_else(|_| panic!("{name}"))
        };
        for (name, v) in [
            ("YD_T6_OSAL_QUEUE_VA", osal::QUEUE_VA),
            ("YD_T6_OSAL_QUEUE_SIZE", crate::board::nanopc_t6_submit::QUEUE_VM_BYTES),
            ("YD_T6_OSAL_HEAP_POOL_VA", osal::HEAP_POOL_VA),
            ("YD_T6_OSAL_HEAP_POOL_SIZE", crate::board::nanopc_t6_submit::HEAP_POOL_SIZE),
            ("YD_T6_OSAL_HEAP_CHUNK_VA", osal::HEAP_CHUNK_VA),
            ("YD_T6_OSAL_HEAP_CHUNK", u64::from(osal::HEAP_CHUNK_SIZE)),
            ("YD_T6_OSAL_CARVE_VA", osal::CARVE_VA),
            ("YD_T6_OSAL_CARVE_MAX", (osal::MAX_CARVE_PAGES as u64) * crate::gpu::mali::pgtable::PAGE),
            ("YD_T6_OSAL_VM_MAP_RESERVED", u64::from(osal::VM_MAP_RESERVED)),
        ] {
            assert_eq!(def64(name), v, "{name}");
        }
        let mut pins = vec![
            ("sizeof(struct yd_t6_osal_mem)".to_string(), size_of::<YdT6OsalMem>()),
            ("sizeof(struct yd_t6_osal_counts)".into(), size_of::<YdT6OsalCounts>()),
        ];
        macro_rules! field {
            ($t:ident, $c:literal, $($f:ident),*) => { $( pins.push((
                format!("offsetof(struct {}, {})", $c, stringify!($f)), offset_of!($t, $f))); )* };
        }
        field!(
            YdT6OsalMem,
            "yd_t6_osal_mem",
            fw,
            pool,
            pool_pages,
            tables_pa,
            queue,
            ring,
            heap_pool,
            heap_chunks,
            carve,
            vm_pool,
            vm_pool_pages,
            vm_tables_pa,
            heap_initial_chunks,
            carve_reserved
        );
        field!(
            YdT6OsalCounts,
            "yd_t6_osal_counts",
            submitted,
            completed,
            live_pages,
            live_pages_high,
            held,
            refused_frees,
            dead_refusal,
            dead_value,
            restarts,
            last_refusal,
            last_value,
            oom_chunks,
            oom_no_chunk,
            heap_chunks,
            unended,
            down,
            memory_kept,
            stops_refused
        );
        for (what, v) in &pins {
            let pin = format!("_Static_assert({what} == {v},");
            assert!(H.contains(&pin), "{pin}");
        }
        // the library's struct osal_gpu_mem (include/gles_osal.h)
        assert_eq!(
            [
                offset_of!(OsalGpuMem, cpu),
                offset_of!(OsalGpuMem, gpu),
                offset_of!(OsalGpuMem, phys),
                offset_of!(OsalGpuMem, size),
                offset_of!(OsalGpuMem, impl_)
            ],
            [0, 8, 16, 24, 32]
        );
        let lib = include_str!("../../../../include/gles_osal.h");
        assert!(lib.contains("struct osal_gpu_mem {\n\tvoid     *cpu;"), "gles_osal.h's struct osal_gpu_mem");
    }

    #[test]
    fn the_z4_entry_runs_the_generated_frame_over_the_model_to_the_predicted_record() {
        use crate::board::nanopc_t6_submit::tests::{QUEUE, VM_TABLES_PA, Z4};
        use crate::board::nanopc_t6_submit::{self as sub, Z4Mem, GENERATED};
        let blob = synth::small();
        let z = Z4::generated(blob.clone());
        let bk = backings(&blob);
        let mut pool = vec![[0u64; ENTRIES]; 16];
        let mut vm_pool = vec![[0u64; ENTRIES]; 16];
        let vm = z.backings();
        let mem = Z4Mem { queue: QUEUE, vm: &vm, vm_pool: &mut vm_pool, vm_tables_pa: VM_TABLES_PA };
        MODEL.with(|m| m.set(&z.t6));
        let o = submit_run(
            &z,
            B,
            &blob,
            &bk,
            &mut pool,
            TABLES_PA,
            RK3588_TIMER_HZ,
            mem,
            &GENERATED,
            clean_model,
            no_delay,
            no_handover,
            &mut Default::default(),
        );
        assert_eq!((o.step, o.refusal, o.detail), (sub::STEP_NONE, 0, 0));
        assert_eq!((o.fw.step, o.fw.ping_acked, o.as0_tables_used, o.vm_tables_used), (0, 1, 8, 10));
        assert_eq!((o.heap_ctx, o.heap_chunk), (sub::HEAP_CTX_VA, def64("YD_T6_SUBMIT_HEAP_CHUNK_VA")));
        assert_eq!((o.seqno, o.sync_status, o.red, o.blue, o.pixels), (1, 0, 2048, 2048, 4096));
        assert_eq!(o.digest, sub::DIGEST);
    }

    #[test]
    fn the_z4_entry_refuses_null_and_misaligned_arguments() {
        use crate::board::nanopc_t6_submit::SubmitUp;
        let mut out = SubmitUp::default();
        let mut g = YdT6GpuUp::default();
        let z1: *mut YdT6GpuUp = &mut g;
        let mut pool = vec![0u8; 8192 * 2];
        let bk = [YdT6FwBacking::default()];
        let blob = [0u8; 4];
        let off = pool.as_mut_ptr().align_offset(4096);
        let p = unsafe { pool.as_mut_ptr().add(off) };
        let mem = |vm_pool: *mut u8, pages: u32| YdT6SubmitMem {
            queue: YdT6FwBacking::default(),
            regions: [YdT6FwBacking::default(); YD_T6_SUBMIT_REGIONS],
            vm_pool,
            vm_pool_pages: pages,
            vm_tables_pa: 0,
        };
        const MAP: usize = crate::board::nanopc_t6_gpu::GPU_MAP_MIN;
        let call = |gpu_len: usize, blob: *const u8, pool: *mut u8, mem: *const YdT6SubmitMem, out: *mut SubmitUp| unsafe {
            yd_t6_submit(
                0,
                0,
                0,
                0,
                0,
                gpu_len,
                blob,
                4,
                bk.as_ptr(),
                1,
                pool,
                1,
                0,
                1,
                mem,
                Some(clean_ok),
                Some(no_delay),
                Some(no_handover),
                z1,
                out,
            )
        };
        let good = mem(unsafe { p.add(4096) }, 1);
        assert_eq!(call(MAP, blob.as_ptr(), p, &good, core::ptr::null_mut()), -EINVAL);
        assert_eq!(call(MAP, core::ptr::null(), p, &good, &mut out), -EINVAL);
        assert_eq!(call(MAP, blob.as_ptr(), p, core::ptr::null(), &mut out), -EINVAL);
        assert_eq!(call(MAP, blob.as_ptr(), unsafe { p.add(8) }, &good, &mut out), -EINVAL);
        // the group VM's pool: null, empty, misaligned
        assert_eq!(call(MAP, blob.as_ptr(), p, &mem(core::ptr::null_mut(), 1), &mut out), -EINVAL);
        assert_eq!(call(MAP, blob.as_ptr(), p, &mem(unsafe { p.add(4096) }, 0), &mut out), -EINVAL);
        assert_eq!(call(MAP, blob.as_ptr(), p, &mem(unsafe { p.add(4100) }, 1), &mut out), -EINVAL);
        // with every pointer in order the run starts and the frame's zero
        // backings are refused before anything is touched: -EIO, STEP_FRAME
        assert_eq!(call(MAP, blob.as_ptr(), p, &good, &mut out), -EIO);
        assert_eq!(
            (out.step, out.refusal),
            (crate::board::nanopc_t6_submit::STEP_FRAME, crate::board::nanopc_t6_submit::FrameRefusal::Memory as u32)
        );
        // the same call with one byte less of the GPU mapped: refused by
        // the map alone
        assert_eq!(call(MAP - 1, blob.as_ptr(), p, &good, &mut out), -EINVAL);
    }
}

/// The display adopt of step D2 ([`crate::board::nanopc_t6_display`]):
/// `struct yd_t6_display_bases`, `struct yd_t6_display_adopt` and `struct
/// yd_t6_display_status` are the board module's own `#[repr(C)]` records,
/// and so is step D3's `struct yd_t6_screen`.
pub use crate::board::nanopc_t6_display::{
    Adopt as YdT6DisplayAdopt, Bases as YdT6DisplayBases, Screen as YdT6Screen, Status as YdT6DisplayStatus,
};

#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<YdT6DisplayBases>() == 64);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(offset_of!(YdT6DisplayBases, vop_mmu) == 8);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(offset_of!(YdT6DisplayBases, hdmi) == 56);
const _: () = assert!(size_of::<YdT6DisplayAdopt>() == 4 * 48);
const _: () = assert!(offset_of!(YdT6DisplayAdopt, pd_vop) == 4 * 4);
const _: () = assert!(offset_of!(YdT6DisplayAdopt, version) == 4 * 11);
const _: () = assert!(offset_of!(YdT6DisplayAdopt, timing) == 4 * 16);
const _: () = assert!(offset_of!(YdT6DisplayAdopt, mmu_status) == 4 * 27);
const _: () = assert!(offset_of!(YdT6DisplayAdopt, running) == 4 * 31);
const _: () = assert!(offset_of!(YdT6DisplayAdopt, ovl_layer_sel) == 4 * 36);
const _: () = assert!(offset_of!(YdT6DisplayAdopt, fb) == 4 * 39);
const _: () = assert!(offset_of!(YdT6DisplayAdopt, shown) == 4 * 44);
const _: () = assert!(size_of::<YdT6DisplayStatus>() == 4 * 38);
const _: () = assert!(offset_of!(YdT6DisplayStatus, frames) == 4 * 7);
const _: () = assert!(offset_of!(YdT6DisplayStatus, vop) == 4 * 9);
const _: () = assert!(offset_of!(YdT6DisplayStatus, soc_status1) == 4 * 24);
const _: () = assert!(offset_of!(YdT6DisplayStatus, vo1_usable) == 4 * 26);
const _: () = assert!(offset_of!(YdT6DisplayStatus, monitor) == 4 * 30);
const _: () = assert!(offset_of!(YdT6DisplayStatus, cru_gate60) == 4 * 37);
const _: () = assert!(size_of::<YdT6Screen>() == 48);
const _: () = assert!(offset_of!(YdT6Screen, held) == 8);
const _: () = assert!(offset_of!(YdT6Screen, state) == 24);
const _: () = assert!(offset_of!(YdT6Screen, n_held) == 28);
const _: () = assert!(offset_of!(YdT6Screen, width) == 32);
const _: () = assert!(offset_of!(YdT6Screen, stride) == 40);
const _: () = assert!(offset_of!(YdT6Screen, flips) == 44);

/// Draw the adopt's pattern ([`crate::board::nanopc_t6_display::draw_pattern`])
/// into `fb`: `height` rows of `stride_bytes`, `width` pixels each. 0, or
/// `-EINVAL` (nothing written) for a null or misaligned `fb`, a stride that
/// is not whole pixels or shorter than a line, or a zero size.
///
/// # Safety
/// `fb` is valid for writes of `stride_bytes * height` bytes.
#[no_mangle]
pub unsafe extern "C" fn yd_t6_display_pattern(fb: *mut u32, width: u32, height: u32, stride_bytes: u32) -> i32 {
    if fb.is_null() || !(fb as usize).is_multiple_of(4) || !stride_bytes.is_multiple_of(4) || width == 0 || height == 0
    {
        return -EINVAL;
    }
    let stride_px = stride_bytes as usize / 4;
    // SAFETY: the caller's contract.
    let px = unsafe { core::slice::from_raw_parts_mut(fb, stride_px * height as usize) };
    if crate::board::nanopc_t6_display::draw_pattern(px, width as usize, height as usize, stride_px) {
        0
    } else {
        -EINVAL
    }
}

/// The adopt ([`crate::board::nanopc_t6_display::adopt`]) on the blocks of
/// `b`, showing the buffer at physical `fb_pa` (drawn and cleaned by the
/// caller). 0 when Esmart0 shows it, `-EIO` when a step stopped
/// (`out.stop` says which), `-EINVAL` for a null argument.
///
/// # Safety
/// Every base of `*b` is mapped device memory covering what the module
/// reads and writes (`VOP_MAP` bytes of the VOP, 0x100 of each `vop_mmu`
/// set, a page of each other block); `b` and `out` are valid.
#[no_mangle]
pub unsafe extern "C" fn yd_t6_display_adopt(
    b: *const YdT6DisplayBases,
    fb_pa: u64,
    delay_us: Option<extern "C" fn(u32)>,
    out: *mut YdT6DisplayAdopt,
) -> i32 {
    let Some(delay_us) = delay_us else { return -EINVAL };
    if b.is_null() || out.is_null() {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    let (hw, b) = unsafe { (Hw::new(), &*b) };
    let w = crate::board::nanopc_t6_display::probe_window(fb_pa);
    let a = crate::board::nanopc_t6_display::adopt(&hw, b, &w, &mut |us| delay_us(us));
    // SAFETY: the caller's contract.
    unsafe { *out = a };
    if a.stop == crate::board::nanopc_t6_display::Stop::None as u32 {
        0
    } else {
        -EIO
    }
}

/// Section 5's read-backs ([`crate::board::nanopc_t6_display::status`]):
/// the frame check over `wait_us`, `frames` frame starts timed with
/// `now_us` (µs), the registers. 0, `-EIO` when a frame wait or the check
/// refused (`out.refusal`), `-EINVAL` for a null argument.
///
/// # Safety
/// As [`yd_t6_display_adopt`].
#[no_mangle]
pub unsafe extern "C" fn yd_t6_display_status(
    b: *const YdT6DisplayBases,
    wait_us: u32,
    frames: u32,
    now_us: Option<extern "C" fn() -> u64>,
    delay_us: Option<extern "C" fn(u32)>,
    out: *mut YdT6DisplayStatus,
) -> i32 {
    let (Some(now_us), Some(delay_us)) = (now_us, delay_us) else { return -EINVAL };
    if b.is_null() || out.is_null() {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    let (hw, b) = unsafe { (Hw::new(), &*b) };
    let s = crate::board::nanopc_t6_display::status(&hw, b, wait_us, frames, &mut || now_us(), &mut |us| delay_us(us));
    // SAFETY: the caller's contract.
    unsafe { *out = s };
    if s.refusal == 0 {
        0
    } else {
        -EIO
    }
}

/// Show the buffer at physical `phys` on the screen `s`
/// ([`crate::board::nanopc_t6_display::show`]): the first call is the adopt
/// with this buffer as its window (record in `adopt`), a later call with
/// another address a flip that never waits. 0; `-ENOTSUP` for a window the
/// VOP cannot scan or one that is not the first show's size and stride
/// (nothing read or written); `-ENODEV` when the adopt stopped, now or at
/// the first show (`adopt.stop` names the step of this call's adopt);
/// `-EINVAL` for a null argument.
///
/// # Safety
/// As [`yd_t6_display_adopt`]; `s` is valid, all zero before the first
/// call and then only this module's; `adopt` is valid. One thread calls
/// this and [`yd_t6_screen_busy`] for one `s`.
#[no_mangle]
pub unsafe extern "C" fn yd_t6_screen_show(
    b: *const YdT6DisplayBases,
    s: *mut YdT6Screen,
    phys: u64,
    width: u32,
    height: u32,
    stride_bytes: u32,
    delay_us: Option<extern "C" fn(u32)>,
    adopt: *mut YdT6DisplayAdopt,
) -> i32 {
    let Some(delay_us) = delay_us else { return -EINVAL };
    if b.is_null() || s.is_null() || adopt.is_null() {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    let (hw, b, s, adopt) = unsafe { (Hw::new(), &*b, &mut *s, &mut *adopt) };
    crate::board::nanopc_t6_display::show(&hw, b, s, phys, width, height, stride_bytes, &mut |us| delay_us(us), adopt)
}

/// Which of the `n` buffers at `phys[]` the screen `s` may still scan
/// ([`crate::board::nanopc_t6_display::busy`]): bit k for `phys[k]`, of the
/// first 32. Too large is possible, too small never; never waits. A null
/// `b` or `s`, or a null `phys` with `n` not 0, answers every bit: this
/// call cannot name a buffer free.
///
/// # Safety
/// As [`yd_t6_screen_show`]; `phys` is valid for `n` reads.
#[no_mangle]
pub unsafe extern "C" fn yd_t6_screen_busy(
    b: *const YdT6DisplayBases,
    s: *mut YdT6Screen,
    phys: *const u64,
    n: usize,
) -> u32 {
    if n == 0 {
        return 0;
    }
    if b.is_null() || s.is_null() || phys.is_null() {
        return u32::MAX;
    }
    // SAFETY: the caller's contract.
    let (hw, b, s, addrs) = unsafe { (Hw::new(), &*b, &mut *s, core::slice::from_raw_parts(phys, n)) };
    crate::board::nanopc_t6_display::busy(&hw, b, s, addrs)
}

/// Step D8, the display brought up from cold
/// ([`crate::board::nanopc_t6_display_cold`]): `struct
/// yd_t6_display_cold_bases` and `struct yd_t6_display_cold` are the board
/// module's own `#[repr(C)]` records.
pub use crate::board::nanopc_t6_display_cold::{Cold as YdT6DisplayCold, ColdBases as YdT6DisplayColdBases};

#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<YdT6DisplayColdBases>() == 88);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(offset_of!(YdT6DisplayColdBases, vop_grf) == 64);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(offset_of!(YdT6DisplayColdBases, phy) == 80);
const _: () = assert!(size_of::<YdT6DisplayCold>() == 4 * 5);
const _: () = assert!(offset_of!(YdT6DisplayCold, error) == 4);
const _: () = assert!(offset_of!(YdT6DisplayCold, detail) == 16);

/// The display chain from cold ([`crate::board::nanopc_t6_display_cold::cold`])
/// on the blocks of `b`, showing the buffer at physical `fb_pa` (1024x768
/// XRGB8888, stride 4096, drawn and cleaned by the caller) at the top left
/// of the mode, which is 1024x768 at 60 Hz, or 1920x1080 at 60 Hz with
/// `YD_T6_DISPLAY_COLD_MODE_1920X1080`. `flags` are
/// `YD_T6_DISPLAY_COLD_*`; `stop_after` 0 runs to the end, N stops after
/// stage N. Every line goes to `env`'s log as it happens. 0 when the chain
/// ran through, `-ECANCELED` when it stopped after `stop_after` as asked,
/// `-EIO` when a stage failed (`out` names it), `-EINVAL` for a null
/// argument, an unknown flag or a `stop_after` past the last stage
/// (nothing read).
///
/// # Safety
/// Every base of `*b` is mapped device memory covering what the chain
/// reads and writes: as [`yd_t6_display_adopt`]'s, with the CRU mapping
/// also covering the PMU1 CRU at +0x30000 and the HDMI controller's up to
/// 0x5000; vop_grf and vo1_grf a page each, the PHY 0x2000 bytes. `b`,
/// `env` and `out` are valid.
#[no_mangle]
pub unsafe extern "C" fn yd_t6_display_cold(
    b: *const YdT6DisplayColdBases,
    env: *const crate::env::YdEnv,
    flags: u32,
    stop_after: u32,
    fb_pa: u64,
    out: *mut YdT6DisplayCold,
) -> i32 {
    if b.is_null() || env.is_null() || out.is_null() {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    let (hw, b, env) = unsafe { (Hw::new(), &*b, &*env) };
    let Ok(c) = crate::board::nanopc_t6_display_cold::cold(&hw, b, env, flags, stop_after, fb_pa) else {
        return -EINVAL;
    };
    // SAFETY: the caller's contract.
    unsafe { *out = c };
    match c.error {
        0 => 0,
        e if e == crate::board::nanopc_t6_display_cold::ColdError::Stopped as u32 => -crate::errno::ECANCELED,
        _ => -EIO,
    }
}

#[cfg(test)]
mod display_tests {
    use super::*;
    use crate::board::nanopc_t6_display::Stop;

    #[test]
    fn the_c_header_has_the_d2_numbers_and_layout() {
        const H: &str = include_str!("../../include/yakogl_drivers.h");
        let def = |name: &str| -> u32 {
            let l = H.lines().find(|l| l.starts_with(&format!("#define {name} "))).expect(name);
            l.split_whitespace().nth(2).expect(name).parse().unwrap_or_else(|_| panic!("{name}"))
        };
        for (name, v) in [
            ("YD_T6_DISPLAY_STOP_NONE", Stop::None),
            ("YD_T6_DISPLAY_STOP_VOP_DOMAIN", Stop::VopDomain),
            ("YD_T6_DISPLAY_STOP_VOP_CLOCK", Stop::VopClock),
            ("YD_T6_DISPLAY_STOP_VERSION", Stop::Version),
            ("YD_T6_DISPLAY_STOP_STANDBY", Stop::Standby),
            ("YD_T6_DISPLAY_STOP_VP_STATE", Stop::VpState),
            ("YD_T6_DISPLAY_STOP_MODE", Stop::Mode),
            ("YD_T6_DISPLAY_STOP_ROUTE", Stop::Route),
            ("YD_T6_DISPLAY_STOP_ESMART_POWER", Stop::EsmartPower),
            ("YD_T6_DISPLAY_STOP_TAKE_OVER", Stop::TakeOver),
            ("YD_T6_DISPLAY_STOP_IOMMU", Stop::Iommu),
            ("YD_T6_DISPLAY_STOP_OVERLAY", Stop::Overlay),
            ("YD_T6_DISPLAY_STOP_WINDOW", Stop::Window),
            ("YD_T6_DISPLAY_STOP_FRAME", Stop::Frame),
            ("YD_T6_DISPLAY_STOP_SHOWN", Stop::Shown),
        ] {
            assert_eq!(def(name), v as u32, "{name}");
        }
        {
            use crate::board::nanopc_t6_display::{SCREEN_NEW, SCREEN_SHOWN, SCREEN_STOPPED};
            for (name, v) in [
                ("YD_T6_SCREEN_NEW", SCREEN_NEW),
                ("YD_T6_SCREEN_SHOWN", SCREEN_SHOWN),
                ("YD_T6_SCREEN_STOPPED", SCREEN_STOPPED),
            ] {
                assert_eq!(def(name), v, "{name}");
            }
        }
        let pins = [
            ("sizeof(struct yd_t6_display_bases)", size_of::<YdT6DisplayBases>()),
            ("offsetof(struct yd_t6_display_bases, vop_mmu)", offset_of!(YdT6DisplayBases, vop_mmu)),
            ("offsetof(struct yd_t6_display_bases, hdmi)", offset_of!(YdT6DisplayBases, hdmi)),
            ("sizeof(struct yd_t6_display_adopt)", size_of::<YdT6DisplayAdopt>()),
            ("offsetof(struct yd_t6_display_adopt, pd_vop)", offset_of!(YdT6DisplayAdopt, pd_vop)),
            ("offsetof(struct yd_t6_display_adopt, version)", offset_of!(YdT6DisplayAdopt, version)),
            ("offsetof(struct yd_t6_display_adopt, timing)", offset_of!(YdT6DisplayAdopt, timing)),
            ("offsetof(struct yd_t6_display_adopt, mmu_status)", offset_of!(YdT6DisplayAdopt, mmu_status)),
            ("offsetof(struct yd_t6_display_adopt, running)", offset_of!(YdT6DisplayAdopt, running)),
            ("offsetof(struct yd_t6_display_adopt, ovl_layer_sel)", offset_of!(YdT6DisplayAdopt, ovl_layer_sel)),
            ("offsetof(struct yd_t6_display_adopt, fb)", offset_of!(YdT6DisplayAdopt, fb)),
            ("offsetof(struct yd_t6_display_adopt, shown)", offset_of!(YdT6DisplayAdopt, shown)),
            ("sizeof(struct yd_t6_display_status)", size_of::<YdT6DisplayStatus>()),
            ("offsetof(struct yd_t6_display_status, frames)", offset_of!(YdT6DisplayStatus, frames)),
            ("offsetof(struct yd_t6_display_status, vop)", offset_of!(YdT6DisplayStatus, vop)),
            ("offsetof(struct yd_t6_display_status, soc_status1)", offset_of!(YdT6DisplayStatus, soc_status1)),
            ("offsetof(struct yd_t6_display_status, vo1_usable)", offset_of!(YdT6DisplayStatus, vo1_usable)),
            ("offsetof(struct yd_t6_display_status, monitor)", offset_of!(YdT6DisplayStatus, monitor)),
            ("offsetof(struct yd_t6_display_status, cru_gate60)", offset_of!(YdT6DisplayStatus, cru_gate60)),
            ("sizeof(struct yd_t6_screen)", size_of::<YdT6Screen>()),
            ("offsetof(struct yd_t6_screen, last)", offset_of!(YdT6Screen, last)),
            ("offsetof(struct yd_t6_screen, held)", offset_of!(YdT6Screen, held)),
            ("offsetof(struct yd_t6_screen, state)", offset_of!(YdT6Screen, state)),
            ("offsetof(struct yd_t6_screen, n_held)", offset_of!(YdT6Screen, n_held)),
            ("offsetof(struct yd_t6_screen, width)", offset_of!(YdT6Screen, width)),
            ("offsetof(struct yd_t6_screen, height)", offset_of!(YdT6Screen, height)),
            ("offsetof(struct yd_t6_screen, stride)", offset_of!(YdT6Screen, stride)),
            ("offsetof(struct yd_t6_screen, flips)", offset_of!(YdT6Screen, flips)),
        ];
        for (what, v) in pins {
            let pin = format!("_Static_assert({what} == {v},");
            assert!(H.contains(&pin), "{pin}");
        }
    }

    #[test]
    fn the_c_header_has_the_d8_numbers_and_layout() {
        use crate::board::nanopc_t6_display_cold::{flags, ColdError};
        use crate::soc::rk3588_display_chain::stage;
        const H: &str = include_str!("../../include/yakogl_drivers.h");
        let def = |name: &str| -> u32 {
            let l = H.lines().find(|l| l.starts_with(&format!("#define {name} "))).expect(name);
            // the value, before the comment: a number or "(1u << N)"
            let v = l[format!("#define {name} ").len()..].split("/*").next().expect(name).trim();
            if let Some(hex) = v.strip_prefix("0x").and_then(|s| s.strip_suffix('u')) {
                return u32::from_str_radix(hex, 16).unwrap_or_else(|_| panic!("{name}"));
            }
            match v.strip_prefix("(1u << ").and_then(|s| s.strip_suffix(')')) {
                Some(shift) => 1 << shift.parse::<u32>().unwrap_or_else(|_| panic!("{name}")),
                None => v.parse().unwrap_or_else(|_| panic!("{name}")),
            }
        };
        // the physical bases: the chain's, HDMI0's controller, PHY and GRF
        {
            use crate::ip::{dw_hdmi_qp, hdptx};
            use crate::soc::rk3588_display_chain::RK3588 as P;
            let phys = |name: &str| def(&format!("YD_T6_DISPLAY_{name}_PHYS")) as usize;
            assert_eq!(phys("VOP"), P.vop);
            assert_eq!((phys("VOP_MMU") + 0xe00, phys("VOP_MMU") + 0xf00), (P.vop_mmu[0], P.vop_mmu[1]));
            assert_eq!((phys("PMU"), phys("CRU")), (P.pmu, P.cru));
            assert_eq!((phys("SYS_GRF"), phys("VOP_GRF"), phys("VO1_GRF")), (P.sys_grf, P.vop_grf, P.vo1_grf));
            assert_eq!((phys("HDMI0"), P.hdmi), (dw_hdmi_qp::TX0_BASE, dw_hdmi_qp::TX0_BASE));
            assert_eq!((phys("HDPTX0"), phys("HDPTX0_GRF")), (hdptx::PHY0.phy, hdptx::PHY0.grf));
            assert_eq!(P.phy, hdptx::PHY0);
        }
        assert_eq!(def("YD_T6_DISPLAY_COLD_POWER_CYCLE"), flags::POWER_CYCLE);
        assert_eq!(def("YD_T6_DISPLAY_COLD_FORCE_COLD"), flags::FORCE_COLD);
        assert_eq!(def("YD_T6_DISPLAY_COLD_MODE_1920X1080"), flags::MODE_1920X1080);
        for n in stage::START..=stage::VIDEO {
            assert_eq!(def(&format!("YD_T6_DISPLAY_COLD_STAGE_{}", stage::name(n))), n);
        }
        for (name, v) in [
            ("NONE", ColdError::None),
            ("STOPPED", ColdError::Stopped),
            ("MODE", ColdError::Mode),
            ("PHY", ColdError::Phy),
            ("VOP", ColdError::Vop),
            ("FOUND_RUNNING", ColdError::FoundRunning),
            ("STANDBY", ColdError::Standby),
            ("DISPLAY", ColdError::Display),
            ("GRF", ColdError::Grf),
            ("GATE", ColdError::Gate),
            ("DCLK_MUX", ColdError::DclkMux),
            ("HDMI", ColdError::Hdmi),
            ("IOMMU", ColdError::Iommu),
            ("VIDEO", ColdError::Video),
            ("VOP_UNREADABLE", ColdError::VopUnreadable),
            ("UNCLOCKED", ColdError::Unclocked),
        ] {
            assert_eq!(def(&format!("YD_T6_DISPLAY_COLD_ERROR_{name}")), v as u32, "{name}");
        }
        let pins = [
            ("sizeof(struct yd_t6_display_cold_bases)", size_of::<YdT6DisplayColdBases>()),
            ("offsetof(struct yd_t6_display_cold_bases, vop_grf)", offset_of!(YdT6DisplayColdBases, vop_grf)),
            ("offsetof(struct yd_t6_display_cold_bases, vo1_grf)", offset_of!(YdT6DisplayColdBases, vo1_grf)),
            ("offsetof(struct yd_t6_display_cold_bases, phy)", offset_of!(YdT6DisplayColdBases, phy)),
            ("sizeof(struct yd_t6_display_cold)", size_of::<YdT6DisplayCold>()),
            ("offsetof(struct yd_t6_display_cold, error)", offset_of!(YdT6DisplayCold, error)),
            ("offsetof(struct yd_t6_display_cold, refusal)", offset_of!(YdT6DisplayCold, refusal)),
            ("offsetof(struct yd_t6_display_cold, index)", offset_of!(YdT6DisplayCold, index)),
            ("offsetof(struct yd_t6_display_cold, detail)", offset_of!(YdT6DisplayCold, detail)),
        ];
        for (what, v) in pins {
            let pin = format!("_Static_assert({what} == {v},");
            assert!(H.contains(&pin), "{pin}");
        }
    }

    #[test]
    fn the_cold_entry_refuses_a_null_argument_before_anything_is_touched() {
        let mut out = YdT6DisplayCold { stage: 7, ..YdT6DisplayCold::default() };
        let live_b = core::ptr::NonNull::<YdT6DisplayColdBases>::dangling().as_ptr().cast_const();
        let live_env = core::ptr::NonNull::<crate::env::YdEnv>::dangling().as_ptr().cast_const();
        // SAFETY: each call is refused before it dereferences anything.
        unsafe {
            assert_eq!(yd_t6_display_cold(core::ptr::null(), live_env, 1, 0, 0, &mut out), -EINVAL);
            assert_eq!(yd_t6_display_cold(live_b, core::ptr::null(), 1, 0, 0, &mut out), -EINVAL);
            assert_eq!(yd_t6_display_cold(live_b, live_env, 1, 0, 0, core::ptr::null_mut()), -EINVAL);
        }
        assert_eq!(out.stage, 7, "the record is not written");
    }

    #[test]
    fn null_or_misshapen_arguments_are_refused_before_anything_is_touched() {
        extern "C" fn no_delay(_: u32) {}
        extern "C" fn no_clock() -> u64 {
            0
        }
        let mut a = YdT6DisplayAdopt::default();
        let mut s = YdT6DisplayStatus::default();
        let b = core::ptr::null::<YdT6DisplayBases>();
        // SAFETY: each call is refused before it dereferences anything.
        unsafe {
            assert_eq!(yd_t6_display_adopt(b, 0, Some(no_delay), &mut a), -EINVAL);
            assert_eq!(yd_t6_display_status(b, 0, 0, Some(no_clock), Some(no_delay), &mut s), -EINVAL);
            assert_eq!(yd_t6_display_pattern(core::ptr::null_mut(), 4, 4, 16), -EINVAL);
        }
        // the screen: nothing is touched without the bases, the screen and
        // the adopt's record; no buffer is named free without them
        let mut sc = YdT6Screen::default();
        let live = core::ptr::NonNull::<YdT6DisplayBases>::dangling().as_ptr().cast_const();
        let addrs = [0x1000u64, 0x2000];
        // SAFETY: each call is refused before it dereferences anything.
        unsafe {
            assert_eq!(yd_t6_screen_show(b, &mut sc, 0x1000, 1024, 768, 4096, Some(no_delay), &mut a), -EINVAL);
            assert_eq!(
                yd_t6_screen_show(live, core::ptr::null_mut(), 0x1000, 1024, 768, 4096, Some(no_delay), &mut a),
                -EINVAL
            );
            assert_eq!(yd_t6_screen_show(live, &mut sc, 0x1000, 1024, 768, 4096, None, &mut a), -EINVAL);
            assert_eq!(
                yd_t6_screen_show(live, &mut sc, 0x1000, 1024, 768, 4096, Some(no_delay), core::ptr::null_mut()),
                -EINVAL
            );
            assert_eq!(yd_t6_screen_busy(b, &mut sc, addrs.as_ptr(), 2), u32::MAX);
            assert_eq!(yd_t6_screen_busy(live, core::ptr::null_mut(), addrs.as_ptr(), 2), u32::MAX);
            assert_eq!(yd_t6_screen_busy(live, &mut sc, core::ptr::null(), 2), u32::MAX);
            assert_eq!(yd_t6_screen_busy(b, core::ptr::null_mut(), core::ptr::null(), 0), 0);
        }
        assert_eq!((sc, a), (YdT6Screen::default(), YdT6DisplayAdopt::default()));
        let mut fb = [7u32; 16];
        // SAFETY: fb holds 4 rows of 16 bytes.
        unsafe {
            assert_eq!(yd_t6_display_pattern(fb.as_mut_ptr(), 4, 4, 15), -EINVAL);
            assert_eq!(yd_t6_display_pattern(fb.as_mut_ptr(), 5, 4, 16), -EINVAL);
            assert!(fb.iter().all(|p| *p == 7));
            assert_eq!(yd_t6_display_pattern(fb.as_mut_ptr(), 4, 4, 16), 0);
        }
        assert_eq!(fb[0], crate::board::nanopc_t6_display::WHITE);
        assert_eq!(fb[5], crate::board::nanopc_t6_display::RED);
    }
}

#[cfg(test)]
mod mmu_tests {
    use super::*;
    use crate::board::nanopc_t6_mmu::{mmu_up, Stop};
    use crate::gpu::mali::mmu::tests::{Gpu, BASE};
    use crate::gpu::mali::pgtable::{PtRefusal, ENTRIES};
    use crate::gpu::mali::MaliRefusal;

    #[test]
    fn the_c_header_has_the_z2_numbers_and_layout() {
        const H: &str = include_str!("../../include/yakogl_drivers.h");
        let def = |name: &str| -> u32 {
            let l = H.lines().find(|l| l.starts_with(&format!("#define {name} "))).expect(name);
            l.split_whitespace().nth(2).expect(name).parse().unwrap_or_else(|_| panic!("{name}"))
        };
        for (name, v) in [
            ("YD_T6_MMU_STOP_NONE", Stop::None as u32),
            ("YD_T6_MMU_STOP_FEATURES", Stop::Features as u32),
            ("YD_T6_MMU_STOP_FLUSH", Stop::Flush as u32),
            ("YD_T6_MMU_STOP_TABLES", Stop::Tables as u32),
            ("YD_T6_MMU_STOP_WALK", Stop::Walk as u32),
            ("YD_T6_MMU_STOP_CLEAN", Stop::Clean as u32),
            ("YD_T6_MMU_STOP_ENABLE", Stop::Enable as u32),
            ("YD_T6_MMU_STOP_READ_BACK", Stop::ReadBack as u32),
            ("YD_T6_MMU_STOP_FAULT", Stop::Fault as u32),
            ("YD_T6_MMU_STOP_FLUSH_PT", Stop::FlushPt as u32),
            ("YD_MALI_FLUSH_TIMEOUT", MaliRefusal::FlushTimeout as u32),
            ("YD_MALI_FLUSH_BIT_STUCK", MaliRefusal::FlushBitStuck as u32),
            ("YD_MALI_AS_ACTIVE_STUCK", MaliRefusal::AsActiveStuck as u32),
            ("YD_MALI_PT_BAD_POOL", PtRefusal::BadPool as u32),
            ("YD_MALI_PT_UNALIGNED", PtRefusal::Unaligned as u32),
            ("YD_MALI_PT_OUT_OF_RANGE", PtRefusal::OutOfRange as u32),
            ("YD_MALI_PT_POOL_FULL", PtRefusal::PoolFull as u32),
            ("YD_MALI_PT_ALREADY_MAPPED", PtRefusal::AlreadyMapped as u32),
            ("YD_MALI_PT_FOREIGN_TABLE", PtRefusal::ForeignTable as u32),
        ] {
            assert_eq!(def(name), v, "{name}");
        }
        let pins = [
            ("sizeof(struct yd_t6_mmu_up)", size_of::<YdT6MmuUp>()),
            ("offsetof(struct yd_t6_mmu_up, walk_va)", offset_of!(YdT6MmuUp, walk_va)),
            ("offsetof(struct yd_t6_mmu_up, enable_op)", offset_of!(YdT6MmuUp, enable_op)),
            ("offsetof(struct yd_t6_mmu_up, after)", offset_of!(YdT6MmuUp, after)),
            ("offsetof(struct yd_t6_mmu_up, stop)", offset_of!(YdT6MmuUp, stop)),
        ];
        for (what, v) in pins {
            let pin = format!("_Static_assert({what} == {v},");
            assert!(H.contains(&pin), "{pin}");
        }
    }

    #[test]
    fn the_words_c_prints_are_the_run_s_values() {
        let g = Gpu::new(0);
        let mut pool = vec![[0u64; ENTRIES]; 8];
        let o = mmu_up(&g, BASE, 0x2830, &mut pool, 0x1040_0000, 0x1050_0000, &mut |_, _| 0, &mut |_| {});
        let w = YdT6MmuUp::from(&o);
        assert_eq!((w.stop, w.tables_used, w.walk_levels), (0, 4, 4));
        assert_eq!(w.walk_va, [0x0400_1234, 0]);
        assert_eq!(w.walk_pa, [0x1050_1234, 0]);
        assert_eq!(w.walk_desc[3], [0x1050_1f47, 0x0060_0000]);
        assert_eq!(w.flush[0], 0x3304);
        assert_eq!(w.enable_regs, [[0x1040_0000, 0], [0x8c4c_8f4c, 0x8f8f_8f8f], [0x4200_01c6, 0]]);
        assert_eq!(&w.after[..6], &[0x1040_0000, 0, 0x8c4c_8f4c, 0x8f8f_8f8f, 0x4200_01c6, 0]);
        assert_eq!(&w.enable_op[..2], &[63, 0]);
        assert_eq!(&w.flush_pt[..2], &[0x0400_000f, 0]);
        assert_eq!(w.flush_pt[4], 0x0304);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::nanopc_t6::Feed;
    use crate::ip::dw_wdt::tests::{Wdt, BASE, T6_CR, T6_PARAM1, T6_TORR, T6_VERSION};
    use crate::ip::dw_wdt::{Refusal, COMP_TYPE};
    use std::cell::RefCell;

    /// WDT0 and CLKGATE_CON(15) behind one Mmio.
    struct Board {
        wdt: Wdt,
        con15: RefCell<u32>,
        cru_writes: RefCell<Vec<u32>>,
    }
    const CRU: usize = rk3588::CRU_BASE;
    impl Mmio for Board {
        fn read32(&self, addr: usize) -> u32 {
            if addr == CRU + 0x83c {
                return *self.con15.borrow();
            }
            self.wdt.read32(addr)
        }
        fn write32(&self, addr: usize, val: u32) {
            if addr == CRU + 0x83c {
                self.cru_writes.borrow_mut().push(val);
                let mask = val >> 16;
                let mut r = self.con15.borrow_mut();
                *r = (*r & !mask) | (val & mask);
                return;
            }
            self.wdt.write32(addr, val);
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!()
        }
        fn write8(&self, _: usize, _: u8) {
            unreachable!()
        }
    }
    fn board(con15: u32) -> Board {
        Board { wdt: Wdt::t6(), con15: RefCell::new(con15), cru_writes: RefCell::new(vec![]) }
    }

    #[test]
    fn armbian_s_state_arms_without_touching_the_cru() {
        let b = board(0x2ff8);
        let mut out = YdDwWdtArm::default();
        assert_eq!(dw_wdt_arm_with(&b, BASE, CRU, 13, &mut out), 0);
        assert!(b.cru_writes.borrow().is_empty());
        assert_eq!(
            out,
            YdDwWdtArm {
                comp_type: 0x4457_0120,
                param1: T6_PARAM1,
                version: T6_VERSION,
                cr_found: T6_CR,
                torr_found: T6_TORR,
                cr_now: 0x09,
                torr_now: 0xdd,
                ccvr_now: (1 << 29) - 1,
                flags: 0,
                refusal: 0,
                period_ms: 22369,
            }
        );
    }

    #[test]
    fn closed_gates_are_opened_apb_clock_first() {
        let b = board(0x2ffb);
        let mut out = YdDwWdtArm::default();
        assert_eq!(dw_wdt_arm_with(&b, BASE, CRU, 13, &mut out), 0);
        assert_eq!(*b.cru_writes.borrow(), vec![0x0001_0000, 0x0002_0000]);
        assert_eq!(out.flags, YD_DW_WDT_PCLK_OPENED | YD_DW_WDT_TCLK_OPENED);
    }

    #[test]
    fn a_refusal_is_minus_eio_with_its_number() {
        let b = board(0x2ff8);
        b.wdt.regs.borrow_mut()[COMP_TYPE / 4] = 0;
        let mut out = YdDwWdtArm::default();
        assert_eq!(dw_wdt_arm_with(&b, BASE, CRU, 13, &mut out), -EIO);
        assert_eq!((out.refusal, out.period_ms), (Refusal::NotDwWdt as u32, 0));
        assert!(b.wdt.writes().is_empty());
    }

    /// The record is written field by field (it may be Device memory) into
    /// the glue's `__noinit` s_arm, which holds whatever RAM held at boot:
    /// every field must be written on every path, none left as found.
    #[test]
    fn every_field_is_written_over_what_was_there() {
        let stale = YdDwWdtArm {
            comp_type: !0,
            param1: !0,
            version: !0,
            cr_found: !0,
            torr_found: !0,
            cr_now: !0,
            torr_now: !0,
            ccvr_now: !0,
            flags: !0,
            refusal: !0,
            period_ms: !0,
        };
        let mut fresh = YdDwWdtArm::default();
        let mut out = stale;
        assert_eq!(dw_wdt_arm_with(&board(0x2ff8), BASE, CRU, 13, &mut fresh), 0);
        assert_eq!(dw_wdt_arm_with(&board(0x2ff8), BASE, CRU, 13, &mut out), 0);
        assert_eq!(out, fresh);

        let refused = |out: &mut YdDwWdtArm| {
            let b = board(0x2ff8);
            b.wdt.regs.borrow_mut()[COMP_TYPE / 4] = 0;
            dw_wdt_arm_with(&b, BASE, CRU, 13, out)
        };
        let (mut fresh, mut out) = (YdDwWdtArm::default(), stale);
        assert_eq!(refused(&mut fresh), -EIO);
        assert_eq!(refused(&mut out), -EIO);
        assert_eq!(out, fresh);

        // a CRU gate that stays closed (the CRU ignores the write)
        struct Stuck(Board);
        impl Mmio for Stuck {
            fn read32(&self, addr: usize) -> u32 {
                self.0.read32(addr)
            }
            fn write32(&self, addr: usize, val: u32) {
                if addr != CRU + 0x83c {
                    self.0.write32(addr, val);
                }
            }
            fn read8(&self, _: usize) -> u8 {
                unreachable!()
            }
            fn write8(&self, _: usize, _: u8) {
                unreachable!()
            }
        }
        let (mut fresh, mut out) = (YdDwWdtArm::default(), stale);
        assert_eq!(dw_wdt_arm_with(&Stuck(board(0x2ffb)), BASE, CRU, 13, &mut fresh), -EIO);
        assert_eq!(fresh.refusal, YD_DW_WDT_REFUSED_GATE);
        assert_eq!(dw_wdt_arm_with(&Stuck(board(0x2ffb)), BASE, CRU, 13, &mut out), -EIO);
        assert_eq!(out, fresh);
    }

    #[test]
    fn the_c_header_has_the_same_numbers() {
        const H: &str = include_str!("../../include/yakogl_drivers.h");
        let def = |name: &str| -> u32 {
            let l = H.lines().find(|l| l.starts_with(&format!("#define {name} "))).expect(name);
            let v = l.split_whitespace().nth(2).expect(name);
            let v = v.trim_start_matches('(').trim_end_matches(')').trim_end_matches('u');
            if let Some(shift) = l.split("<<").nth(1) {
                let n: String = shift.trim().chars().take_while(char::is_ascii_digit).collect();
                1 << n.parse::<u32>().unwrap()
            } else {
                v.parse().unwrap_or_else(|_| panic!("{name}: {v}"))
            }
        };
        for (name, v) in [
            ("YD_DW_WDT_WAS_RUNNING", YD_DW_WDT_WAS_RUNNING),
            ("YD_DW_WDT_PCLK_OPENED", YD_DW_WDT_PCLK_OPENED),
            ("YD_DW_WDT_TCLK_OPENED", YD_DW_WDT_TCLK_OPENED),
            ("YD_DW_WDT_REFUSED_NOT_DW_WDT", Refusal::NotDwWdt as u32),
            ("YD_DW_WDT_REFUSED_TOP_FIXED", Refusal::TopNotProgrammable as u32),
            ("YD_DW_WDT_REFUSED_ARGUMENT", Refusal::BadArgument as u32),
            ("YD_DW_WDT_REFUSED_NOT_ENABLED", Refusal::NotEnabled as u32),
            ("YD_DW_WDT_REFUSED_RMOD", Refusal::RmodReadBack as u32),
            ("YD_DW_WDT_REFUSED_TOP_READ_BACK", Refusal::TopReadBack as u32),
            ("YD_DW_WDT_REFUSED_COUNTER", Refusal::CounterReadBack as u32),
            ("YD_DW_WDT_REFUSED_GATE", YD_DW_WDT_REFUSED_GATE),
            ("YD_T6_FEED_YES", Feed::Yes as u32),
            ("YD_T6_FEED_RELEASED", Feed::Released as u32),
            ("YD_T6_FEED_LIFETIME_OVER", Feed::LifetimeOver as u32),
            ("YD_T6_FEED_NO_PROGRESS", Feed::NoProgress as u32),
            ("YD_T6_EARLY_VHE_EXIT", YD_T6_EARLY_VHE_EXIT as u32),
        ] {
            assert_eq!(def(name), v, "{name}");
        }
        assert!(H.contains("_Static_assert(sizeof(struct yd_dw_wdt_arm) == 44,"));
    }

    #[test]
    fn z1_record_appends_the_take_over_and_the_header_names_its_numbers() {
        use crate::board::nanopc_t6_gpu::{GpuUp, Stop};
        use crate::gpu::mali::takeover::TakeOver;
        use crate::gpu::mali::{MaliRefusal, Reset};
        const H: &str = include_str!("../../include/yakogl_drivers.h");
        let def = |name: &str| -> u32 {
            let l = H.lines().find(|l| l.starts_with(&format!("#define {name} "))).expect(name);
            l.split_whitespace().nth(2).expect(name).parse().unwrap_or_else(|_| panic!("{name}"))
        };
        for (name, v) in [
            ("YD_T6_GPU_STOP_TAKE_OVER", Stop::TakeOver as u32),
            ("YD_MALI_RESET_BIT_STUCK", MaliRefusal::ResetBitStuck as u32),
            ("YD_MALI_IRQ_MASK_STUCK", MaliRefusal::IrqMaskStuck as u32),
            ("YD_MALI_HARD_RESET_TIMEOUT", MaliRefusal::HardResetTimeout as u32),
            ("YD_T6_TAKE_OVER_MASKED", crate::gpu::mali::takeover::STEP_MASKED),
            ("YD_T6_TAKE_OVER_HALT_WAITED", crate::gpu::mali::takeover::STEP_HALT_WAITED),
            ("YD_T6_TAKE_OVER_RESET", crate::gpu::mali::takeover::STEP_RESET),
        ] {
            assert_eq!(def(name), v, "{name}");
        }
        for (what, v) in [
            ("sizeof(struct yd_t6_gpu_up)", size_of::<YdT6GpuUp>()),
            ("offsetof(struct yd_t6_gpu_up, take_over_ran)", offset_of!(YdT6GpuUp, take_over_ran)),
            ("offsetof(struct yd_t6_gpu_up, take_over_after)", offset_of!(YdT6GpuUp, take_over_after)),
        ] {
            let pin = format!("_Static_assert({what} == {v},");
            assert!(H.contains(&pin), "{pin}");
        }
        let map = H.lines().find(|l| l.starts_with("#define YD_T6_GPU_MAP_MIN ")).expect("YD_T6_GPU_MAP_MIN");
        let map = map.split_whitespace().nth(2).and_then(|v| v.strip_prefix("0x")).expect("a hex value");
        assert_eq!(usize::from_str_radix(map, 16), Ok(crate::board::nanopc_t6_gpu::GPU_MAP_MIN));
        // a run that found PD_GPU off carries none
        assert_eq!(YdT6GpuUp::from(&GpuUp::default()).take_over_ran, 0);
        let t = TakeOver {
            steps: 3,
            masks: [4, 5, 6],
            raw: [7, 8, 9],
            raw_cleared: [10, 11, 12],
            mcu_status: 13,
            mcu_waited_us: 14,
            mcu_halt_timed_out: true,
            mcu_status_halted: 16,
            hard: true,
            reset: Reset { raw_before: 18, raw_cleared: 19, waited_us: 20 },
            after_reset: [21, 22, 23, 24],
        };
        let r = YdT6GpuUp::from(&GpuUp { take_over: Some(t), ..GpuUp::default() });
        assert_eq!(r.take_over_ran, 1);
        let words: Vec<u32> = [
            &[r.take_over_steps][..],
            &r.take_over_masks,
            &r.take_over_raw,
            &r.take_over_raw_cleared,
            &r.take_over_mcu,
            &[r.take_over_hard],
            &r.take_over_reset,
            &r.take_over_after,
        ]
        .concat();
        // in the order of the fields, each flag as 1
        let want: Vec<u32> = [3].into_iter().chain(4..=14).chain([1, 16, 1]).chain(18..=24).collect();
        assert_eq!(words, want);
    }

    #[test]
    fn z1_record_appends_the_qos_policy_and_its_words() {
        use crate::board::nanopc_t6_gpu::GpuUp;
        const H: &str = include_str!("../../include/yakogl_drivers.h");
        assert_eq!(size_of::<YdT6GpuUp>(), 4 * 152);
        assert_eq!(offset_of!(YdT6GpuUp, qos_policy), 4 * 125);
        assert_eq!(offset_of!(YdT6GpuUp, qos_words), 4 * 126);
        for what in ["YD_T6_QOS_RESET      1 ", "YD_T6_QOS_LINUX_ZERO 2 "] {
            assert!(H.contains(&format!("#define {what}")), "{what}");
        }
        assert_eq!(rk3588::QosPolicy::Reset.id(), 1);
        assert_eq!(rk3588::QosPolicy::LinuxZero.id(), 2);
        assert_eq!(rk3588::QosPolicy::from_id(0), None);
        assert_eq!(rk3588::QosPolicy::from_id(3), None);
        // a run that stopped before the step carries no policy
        let none = YdT6GpuUp::from(&GpuUp::default());
        assert_eq!((none.qos_policy, none.qos_words), (0, [0; 3]));
        for p in [rk3588::QosPolicy::Reset, rk3588::QosPolicy::LinuxZero] {
            let mut g = GpuUp { qos_policy: p.id(), ..GpuUp::default() };
            g.qos[0] = [0x8000_0000, p.values()[0], p.values()[1], p.values()[2], 0];
            // the other blocks differ, so the record is shown to read m0
            g.qos[1] = [9; 5];
            let r = YdT6GpuUp::from(&g);
            assert_eq!(r.qos_policy, p.id());
            assert_eq!(r.qos_words, p.values());
        }
    }
}

#[cfg(test)]
mod gic_tests {
    use super::*;
    use crate::aarch64::GicCpuifReset;

    #[test]
    fn the_c_header_has_the_gic_record() {
        const H: &str = include_str!("../../include/yakogl_drivers.h");
        assert!(H.contains("#define YD_GIC_CPUIF_RESET     (1u << 0)"));
        assert!(H.contains("#define YD_GIC_EOIMODE_CLEARED (1u << 1)"));
        assert_eq!((YD_GIC_CPUIF_RESET, YD_GIC_EOIMODE_CLEARED), (1 << 0, 1 << 1));
        assert!(H.contains("_Static_assert(sizeof(struct yd_gic_cpuif_reset) == 40,"));
        assert!(H.contains("_Static_assert(offsetof(struct yd_gic_cpuif_reset, ap1r_cleared) == 32,"));
    }

    #[test]
    fn the_record_carries_what_was_found() {
        let r = GicCpuifReset {
            sre: 0x7,
            ctlr: 0x8402,
            ap1r0: 1 << 20,
            bpr1: 7,
            eoimode_cleared: true,
            ap1r_cleared: 1,
            bpr1_written: true,
        };
        assert_eq!(
            YdGicCpuifReset::from(r),
            YdGicCpuifReset {
                sre: 0x7,
                ctlr_found: 0x8402,
                ap1r0_found: 1 << 20,
                bpr1_found: 7,
                ap1r_cleared: 1,
                flags: YD_GIC_CPUIF_RESET | YD_GIC_EOIMODE_CLEARED,
            }
        );
        // SRE clear: nothing reset, no flag
        let off = GicCpuifReset { sre: 0x6, ..GicCpuifReset::default() };
        assert_eq!(YdGicCpuifReset::from(off), YdGicCpuifReset { sre: 0x6, ..YdGicCpuifReset::default() });
    }

    #[test]
    fn the_c_header_has_the_active_record() {
        const H: &str = include_str!("../../include/yakogl_drivers.h");
        assert!(H.contains("#define YD_GIC_RDIST_FOUND   (1u << 0)"));
        assert!(H.contains(&format!("#define YD_GIC_ACTIVE_LISTED {}", crate::aarch64::GIC_ACTIVE_LISTED)));
        assert!(H.contains("_Static_assert(sizeof(struct yd_gic_active_reset) == 88,"));
        assert!(H.contains("_Static_assert(offsetof(struct yd_gic_active_reset, intids) == 48,"));
        assert!(H.contains("_Static_assert(offsetof(struct yd_gic_active_reset, flags) == 84,"));
    }

    #[test]
    fn the_active_record_carries_what_was_found() {
        use crate::aarch64::{GicActiveReset, GicrFound};
        let gicr = 0xfe68_0000;
        let r = GicActiveReset {
            gicd_typer: 0xf,
            aff: 0x200,
            rdist: GicrFound { rd_base: Some(gicr + 0x4_0000), typer: 0x200_0000_0200, frames: 3 },
            isactiver0: 1 << 27,
            spi_words: 15,
            eppi_words: 0,
            espi_words: 0,
            words_cleared: 16,
            active: 2,
            intids: [27, 511, 0, 0, 0, 0, 0, 0],
            still_active: 0,
        };
        let c = YdGicActiveReset::from_reset(r, gicr);
        assert_eq!((c.flags, c.rd_offset, c.frames, c.gicr_typer), (YD_GIC_RDIST_FOUND, 0x4_0000, 3, 0x200_0000_0200));
        assert_eq!((c.gicd_typer, c.aff, c.isactiver0, c.spi_words, c.words_cleared), (0xf, 0x200, 1 << 27, 15, 16));
        assert_eq!((c.active, c.intids, c.still_active), (2, [27, 511, 0, 0, 0, 0, 0, 0], 0));
        // no redistributor: no flag, no offset
        let none = GicActiveReset { rdist: GicrFound { rd_base: None, typer: 0x10, frames: 4 }, ..r };
        let c = YdGicActiveReset::from_reset(none, gicr);
        assert_eq!((c.flags, c.rd_offset, c.frames, c.gicr_typer), (0, 0, 4, 0x10));
    }
}
