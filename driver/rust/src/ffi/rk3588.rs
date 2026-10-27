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
    pub clock: [u32; 7],
    pub gpu_src_hz: [u32; 2],
    pub spi_gate14: u32,
    pub spi_opened: u32,
    /// RK806 `POWER_EN0`, `POWER_SLP_EN0`, `BUCK1_CONFIG`, `BUCK1_ON_VSEL`.
    pub rk806: [u32; 4],
    /// SPI2 as handed over: `CTRLR0`, `CTRLR1`, `SSIENR`, `SER`, `BAUDR`,
    /// `SR`, `RXFLR`, `VERSION`.
    pub spi_found: [u32; 8],
    pub vdd_gpu_uv: u32,
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
}

#[cfg(any(feature = "mali", test))]
const _: () = assert!(size_of::<YdT6GpuUp>() == 4 * 108);
#[cfg(any(feature = "mali", test))]
const _: () = assert!(offset_of!(YdT6GpuUp, rk806) == 4 * 17);
#[cfg(any(feature = "mali", test))]
const _: () = assert!(offset_of!(YdT6GpuUp, ident) == 4 * 61);
#[cfg(any(feature = "mali", test))]
const _: () = assert!(offset_of!(YdT6GpuUp, stop) == 4 * 104);

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
            clock: [c.sel158, c.sel159, c.gate66, c.mode_con0, c.pll_con[0], c.pll_con[1], c.pll_con[2]],
            gpu_src_hz: lo_hi(c.src_hz),
            spi_gate14: o.spi_gate14,
            spi_opened: o.spi_opened,
            rk806: o.rk806.map(u32::from),
            spi_found: o.spi_found,
            vdd_gpu_uv: o.vdd_gpu_uv,
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
