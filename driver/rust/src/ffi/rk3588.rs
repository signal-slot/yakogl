// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The NanoPC-T6 (RK3588) board glue's Rust side, for
//! driver/zephyr/lib/board/nanopc_t6/nanopc_t6.c: the WDT0 self-arm
//! (soc::rk3588 gates, ip::dw_wdt), its feed, the GIC EOI mode reset and
//! PSCI SYSTEM_RESET (aarch64), the feeder's decision
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
pub fn dw_wdt_arm_with(m: &impl Mmio, wdt_base: usize, cru_base: usize, top: u8, out: &mut YdDwWdtArm) -> i32 {
    *out = YdDwWdtArm::default();
    for (gate, flag) in [(rk3588::PCLK_WDT0, YD_DW_WDT_PCLK_OPENED), (rk3588::TCLK_WDT0, YD_DW_WDT_TCLK_OPENED)] {
        match rk3588::gate_open(m, cru_base, gate) {
            Ok(true) => out.flags |= flag,
            Ok(false) => {}
            Err(now) => {
                out.cr_now = now;
                out.refusal = YD_DW_WDT_REFUSED_GATE;
                return -EIO;
            }
        }
    }
    match dw_wdt::arm(m, wdt_base, top, rk3588::TCLK_WDT0_HZ) {
        Ok(Armed { cr_found, torr_found, param1, version, cr_now, torr_now, ccvr_now, was_running, period_ms }) => {
            out.comp_type = dw_wdt::COMP_TYPE_VALUE;
            out.param1 = param1;
            out.version = version;
            out.cr_found = cr_found;
            out.torr_found = torr_found;
            out.cr_now = cr_now;
            out.torr_now = torr_now;
            out.ccvr_now = ccvr_now;
            if was_running {
                out.flags |= YD_DW_WDT_WAS_RUNNING;
            }
            out.period_ms = period_ms;
            0
        }
        Err(Refused { why, comp_type, param1, cr_found, torr_found, read_back }) => {
            out.comp_type = comp_type;
            out.param1 = param1;
            out.cr_found = cr_found;
            out.torr_found = torr_found;
            out.cr_now = read_back;
            if cr_found & dw_wdt::cr::WDT_EN != 0 {
                out.flags |= YD_DW_WDT_WAS_RUNNING;
            }
            out.refusal = why as u32;
            -EIO
        }
    }
}

/// Arm WDT0 (`wdt_base`, the shim's `device_map()` of 0xfeaf0000) for the
/// period of `top` (0..=15: 2^(16 + top) cycles of 24 MHz), through the CRU
/// at `cru_base` (its mapping of 0xfd7c0000, at least 0x1000 bytes).
///
/// # Safety
/// Both bases are mapped device memory covering the blocks; `out` is valid.
#[no_mangle]
pub unsafe extern "C" fn yd_dw_wdt_arm(wdt_base: usize, cru_base: usize, top: u32, out: *mut YdDwWdtArm) -> i32 {
    if out.is_null() || top > u32::from(dw_wdt::TOP_MAX) {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    let hw = unsafe { Hw::new() };
    // SAFETY: the caller's contract.
    unsafe { dw_wdt_arm_with(&hw, wdt_base, cru_base, top as u8, &mut *out) }
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

/// This CPU's GICv3 CPU interface into the reset EOI mode
/// ([`crate::aarch64::gic_eoi_mode_reset`]): the `ICC_CTLR_EL1` value found
/// when it was changed, 0 when nothing was written.
///
/// # Safety
/// At EL1, on the boot CPU, before interrupts are enabled.
#[cfg(target_arch = "aarch64")]
#[no_mangle]
pub unsafe extern "C" fn yd_gic_eoi_mode_reset() -> u64 {
    // SAFETY: the caller's contract.
    unsafe { crate::aarch64::gic_eoi_mode_reset() }
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
/// stack right after the hook; the entry makes it the stack now, keeps the
/// return address in `x22` (callee-saved, unused by `__start` and
/// `__reset_prep_c`), and calls [`yd_t6_early_c`] with the flags in `x0`.
///
/// `yd_t6_el2_vectors` is the EL2 vector table [`yd_t6_early_c`] installs:
/// every one of the 16 entries passes its offset to [`yd_t6_el2_trap`] on a
/// stack of its own and never returns.
#[cfg(target_arch = "aarch64")]
mod early {
    use crate::aarch64;
    use crate::board::nanopc_t6_early::{self as banner, El2Entry, El2Trap, UART2_PHYS};
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
    /// [`aarch64::HCR_EL2_AT_ENTRY`].
    ///
    /// # Safety
    /// Only from `yd_t6_early_entry`, as reset.S's plat_prep_c hook: MMU off,
    /// uart2 as the previous stage left it, `.bss` not yet used.
    #[no_mangle]
    pub unsafe extern "C" fn yd_t6_early_c(flags: u64) {
        // SAFETY: with the MMU off every address is the physical one, and
        // UART2_PHYS is uart2's 32-bit registers.
        let hw = unsafe { Hw::new() };
        let mut tx = Tx::new(&hw, UART2_PHYS);
        let el = aarch64::current_el();
        if el != 2 {
            banner::banner_a_other(&mut tx, el);
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

    /// `T6:A1`, from Zephyr's `z_arm64_el1_plat_init` (the end of
    /// `z_arm64_el1_init`).
    ///
    /// # Safety
    /// At EL1 with the MMU off (before `z_prep_c`).
    #[no_mangle]
    pub unsafe extern "C" fn yd_t6_early_el1() {
        // SAFETY: the MMU is off: UART2_PHYS is uart2.
        let hw = unsafe { Hw::new() };
        banner::banner_a1(&mut Tx::new(&hw, UART2_PHYS));
    }

    /// `T6:A2` through `uart`, the glue's `device_map()` of uart2.
    ///
    /// # Safety
    /// `uart` maps uart2's registers as device memory.
    #[no_mangle]
    pub unsafe extern "C" fn yd_t6_banner_a2(uart: usize) {
        // SAFETY: the caller's contract.
        let hw = unsafe { Hw::new() };
        banner::banner_a2(&mut Tx::new(&hw, uart));
    }

    /// `T6:A3` through `uart`: the WDT0 arming's result.
    ///
    /// # Safety
    /// As [`yd_t6_banner_a2`].
    #[no_mangle]
    pub unsafe extern "C" fn yd_t6_banner_a3(uart: usize, rc: i32, cr_now: u32, period_ms: u32) {
        // SAFETY: the caller's contract.
        let hw = unsafe { Hw::new() };
        banner::banner_a3(&mut Tx::new(&hw, uart), rc, cr_now, period_ms);
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
}

#[cfg(any(feature = "mali", test))]
const _: () = assert!(size_of::<YdT6GpuUp>() == 4 * 125);
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
        }
    }
}

/// The NanoPC-T6's GPU power-on ([`crate::board::nanopc_t6_gpu::gpu_up`]):
/// 0 when every step ran, `-EIO` when one stopped (`out.stop` says which),
/// `-EINVAL` for a null `out`, `delay_us` or `on_handover`; `on_handover` is
/// called once with the hand-over state, before the first PMU or GPU write. The bases are the shim's
/// `device_map()`s of the CRU (0xfd7c0000, 0x1000), the PMU (0xfd8d8000),
/// SPI2 (0xfeb20000), `qos_gpu_m0` (0xfdf35000, 0x800) and the GPU
/// (0xfb000000, 0x1000); `delay_us` busy-waits.
///
/// # Safety
/// The bases are mapped device memory covering those blocks; `out` is valid.
#[cfg(feature = "mali")]
#[no_mangle]
pub unsafe extern "C" fn yd_t6_gpu_up(
    cru: usize,
    pmu: usize,
    spi2: usize,
    qos: usize,
    gpu: usize,
    delay_us: Option<extern "C" fn(u32)>,
    on_handover: Option<extern "C" fn(*const YdT6GpuUp)>,
    out: *mut YdT6GpuUp,
) -> i32 {
    let (Some(delay_us), Some(on_handover)) = (delay_us, on_handover) else { return -EINVAL };
    if out.is_null() {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    let hw = unsafe { Hw::new() };
    let b = crate::board::nanopc_t6_gpu::Bases { cru, pmu, spi2, qos, gpu };
    let o = crate::board::nanopc_t6_gpu::gpu_up(&hw, b, &mut |us| delay_us(us), &mut |h| {
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
    crate::board::nanopc_t6_fw::fw_up(
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
/// `-EINVAL` for a null or misaligned argument. The bases are those of
/// [`yd_t6_gpu_up`], `gpu` mapped for 0x280000 bytes (the doorbells);
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
    let b = crate::board::nanopc_t6_gpu::Bases { cru, pmu, spi2, qos, gpu };
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
    crate::board::nanopc_t6_submit::submit_up(
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
/// [`yd_t6_fw_up`] writes it), `-EINVAL` for a null or misaligned argument.
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
    let b = crate::board::nanopc_t6_gpu::Bases { cru, pmu, spi2, qos, gpu };
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
    fn the_entry_refuses_null_and_misaligned_arguments() {
        let mut out = FwUp::default();
        let mut g = YdT6GpuUp::default();
        let z1: *mut YdT6GpuUp = &mut g;
        let mut pool = vec![0u8; 8192];
        let bk = [YdT6FwBacking::default()];
        let blob = [0u8; 4];
        let off = pool.as_mut_ptr().align_offset(4096);
        let call = |blob: *const u8,
                    pool: *mut u8,
                    clean: Option<extern "C" fn(usize, usize) -> i32>,
                    out: *mut FwUp| unsafe {
            yd_t6_fw_up(
                0,
                0,
                0,
                0,
                0,
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
        assert_eq!(call(blob.as_ptr(), p, Some(clean_ok), core::ptr::null_mut()), -EINVAL);
        assert_eq!(call(core::ptr::null(), p, Some(clean_ok), &mut out), -EINVAL);
        assert_eq!(call(blob.as_ptr(), p, None, &mut out), -EINVAL);
        assert_eq!(call(blob.as_ptr(), unsafe { p.add(8) }, Some(clean_ok), &mut out), -EINVAL);
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
            drain_cs_ack
        );
        assert_eq!(pins.len(), 7 + 46);
        for (what, v) in &pins {
            let pin = format!("_Static_assert({what} == {v},");
            assert!(H.contains(&pin), "{pin}");
        }
        // one assert per field and one for the size, each naming yd_t6_submit
        let n = H.lines().filter(|l| l.starts_with("_Static_assert(") && l.contains("yd_t6_submit")).count();
        assert!(n >= 47, "{n} _Static_assert lines name yd_t6_submit");
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
        let call = |blob: *const u8, pool: *mut u8, mem: *const YdT6SubmitMem, out: *mut SubmitUp| unsafe {
            yd_t6_submit(
                0,
                0,
                0,
                0,
                0,
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
        assert_eq!(call(blob.as_ptr(), p, &good, core::ptr::null_mut()), -EINVAL);
        assert_eq!(call(core::ptr::null(), p, &good, &mut out), -EINVAL);
        assert_eq!(call(blob.as_ptr(), p, core::ptr::null(), &mut out), -EINVAL);
        assert_eq!(call(blob.as_ptr(), unsafe { p.add(8) }, &good, &mut out), -EINVAL);
        // the group VM's pool: null, empty, misaligned
        assert_eq!(call(blob.as_ptr(), p, &mem(core::ptr::null_mut(), 1), &mut out), -EINVAL);
        assert_eq!(call(blob.as_ptr(), p, &mem(unsafe { p.add(4096) }, 0), &mut out), -EINVAL);
        assert_eq!(call(blob.as_ptr(), p, &mem(unsafe { p.add(4100) }, 1), &mut out), -EINVAL);
        // with every pointer in order the run starts and the frame's zero
        // backings are refused before anything is touched: -EIO, STEP_FRAME
        assert_eq!(call(blob.as_ptr(), p, &good, &mut out), -EIO);
        assert_eq!(
            (out.step, out.refusal),
            (crate::board::nanopc_t6_submit::STEP_FRAME, crate::board::nanopc_t6_submit::FrameRefusal::Memory as u32)
        );
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
}
