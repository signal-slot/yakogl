// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! What an AArch64 image entered from Linux needs from the CPU itself: the
//! GICv3 CPU interface put back into its reset EOI mode, and the way back,
//! PSCI `SYSTEM_RESET`.
//!
//! **EOI mode.** Linux booted at EL2 runs the GICv3 CPU interface in split
//! EOI/deactivate mode (`ICC_CTLR_EL1.EOImode` = 1; Linux v6.18
//! drivers/irqchip/irq-gic-v3.c:1237-1243, read for the fact only) and a
//! kexec does not restore it. Zephyr's GICv3 driver never writes
//! `ICC_CTLR_EL1` and ends an interrupt with `ICC_EOIR1_EL1` only, so in that
//! mode the first interrupt stays active and the system timer ticks once
//! (measured on the AM62P and the i.MX8MP: driver/tools/kexec-zephyr/
//! README.md, "What a previous Linux leaves behind"; the C precedents are
//! driver/soc/ti/k3/am62px/soc_a53.c and driver/zephyr/lib/board/imx8mp/
//! gc_board_gic.c). The NanoPC-T6's Armbian starts all CPUs at EL2 (board
//! dmesg, 2026-10-18), so it is in that mode too.
//!
//! **PSCI.** `SYSTEM_RESET` is function 0x84000009 through SMC (PSCI 0.2 and
//! later; the T6's firmware reports PSCI 1.1 and its device tree says
//! `method = "smc"`, board dmesg and /proc/device-tree/psci, 2026-10-18).
//! Linux reboots the T6 this way: psci_sys_reset has priority 129
//! (drivers/firmware/psci/psci.c:309-330), above dw_wdt's restart handler
//! (128, drivers/watchdog/dw_wdt.c:653).

/// **EL2 as a VHE Linux leaves it.** A Linux that runs its kernel at EL2
/// with the Virtualization Host Extensions (the T6's Armbian: dmesg "CPU
/// features: detected: Virtualization Host Extensions", 2026-10-19) has
/// `HCR_EL2` = `HCR_HOST_VHE_FLAGS` = RW | TGE | E2H | AMO | IMO | FMO (Linux
/// v6.18 arch/arm64/include/asm/kvm_arm.h:106), and its kexec enters the new
/// image at EL2 with that value: no EL2 vectors are set up for a VHE host
/// (machine_kexec.c:123-126), the relocation code turns the MMU off with
/// `msr sctlr_el1`, which E2H redirects to `SCTLR_EL2`, and branches to the
/// entry (relocate_kernel.S:84-99, cpu-reset.S's comment says the same).
/// Linux copes as the next kernel because its entry writes a whole new
/// `HCR_EL2` (head.S:302, `init_el2_hcr HCR_HOST_NVHE_FLAGS`). Zephyr's
/// `z_arm64_el2_init` (arch/arm64/core/reset.c, zephyr af17c0c8) instead
/// reads `HCR_EL2`, clears IMO, AMO and TGE and sets RW: E2H and FMO stay
/// set. It then writes `CPTR_EL2` and `CNTHCTL_EL2` in their E2H=0 layouts.
/// With E2H=1, `CNTHCTL_EL2` bits 0 and 1 are the EL0 controls and EL1's
/// access to the physical timer and counter is bits 11 and 10 (Linux's KVM
/// shifts them by 10 under VHE: kvm/arch_timer.c:829-846), so they stay 0
/// and EL1's `CNTP_*` accesses trap to EL2. `z_arm64_el1_init` writes
/// `CNTP_CVAL_EL0` before any C code of the image runs: the trap goes to
/// Linux's `VBAR_EL2`, a kernel virtual address with the EL2 MMU off, and
/// the CPU stops there with nothing printed. FMO=1 also sends EL1's
/// `ICC_CTLR_EL1` accesses to the virtual interface, so the EOI mode reset
/// below would not reach the physical one. U-Boot leaves E2H clear, which is
/// why Zephyr's roc_rk3588_pc (U-Boot `go`) never meets this.
///
/// The fix is Linux's: write a whole `HCR_EL2` before Zephyr's EL2 init
/// reads it, [`HCR_EL2_AT_ENTRY`] (driver/zephyr/lib/board/nanopc_t6,
/// `z_arm64_el2_plat_prep_c`).
pub const HCR_FMO: u64 = 1 << 3;
pub const HCR_IMO: u64 = 1 << 4;
pub const HCR_AMO: u64 = 1 << 5;
pub const HCR_TGE: u64 = 1 << 27;
pub const HCR_RW: u64 = 1 << 31;
pub const HCR_E2H: u64 = 1 << 34;

/// Linux v6.18 `HCR_HOST_VHE_FLAGS` (kvm_arm.h:106): what a kexec from a VHE
/// host leaves in `HCR_EL2`.
pub const HCR_LINUX_VHE_HOST: u64 = HCR_RW | HCR_TGE | HCR_E2H | HCR_AMO | HCR_IMO | HCR_FMO;

/// The `HCR_EL2` written at entry, before Zephyr's `z_arm64_el2_init`: EL1 is
/// AArch64 and nothing else. E2H 0 gives `CPTR_EL2` and `CNTHCTL_EL2` the
/// layout Zephyr writes; TGE, IMO, FMO and AMO 0 route EL1's exceptions,
/// interrupts and GIC CPU interface accesses to EL1 and the physical
/// interface.
pub const HCR_EL2_AT_ENTRY: u64 = HCR_RW;

/// `ICC_SRE_EL1.SRE`: the system-register interface is enabled.
pub const ICC_SRE_EL1_SRE: u64 = 1 << 0;
/// `ICC_CTLR_EL1.EOImode`.
pub const ICC_CTLR_EL1_EOIMODE: u64 = 1 << 1;

/// PSCI 0.2 `SYSTEM_RESET` (SMC32 function ID).
pub const PSCI_SYSTEM_RESET: u64 = 0x8400_0009;

/// The `ICC_CTLR_EL1` value to write, or `None` when nothing is to be
/// written: the system-register interface is off (the CPU interface has not
/// been touched since reset, so it is in reset mode) or EOImode is already 0.
pub const fn icc_ctlr_reset_mode(sre: u64, ctlr: u64) -> Option<u64> {
    if sre & ICC_SRE_EL1_SRE == 0 || ctlr & ICC_CTLR_EL1_EOIMODE == 0 {
        return None;
    }
    Some(ctlr & !ICC_CTLR_EL1_EOIMODE)
}

/// Put this CPU's GICv3 CPU interface into the reset EOI mode. Returns the
/// `ICC_CTLR_EL1` value found when it changed it, 0 when nothing was
/// written (0 is never a value it changes: EOImode was set in it).
///
/// # Safety
/// Runs at EL1 on the CPU whose interface is meant, before interrupts are
/// enabled.
#[cfg(target_arch = "aarch64")]
pub unsafe fn gic_eoi_mode_reset() -> u64 {
    let sre: u64;
    let ctlr: u64;
    // SAFETY: reading the two system registers has no side effect.
    unsafe {
        core::arch::asm!("mrs {}, S3_0_C12_C12_5", out(reg) sre, options(nomem, nostack, preserves_flags));
    }
    if sre & ICC_SRE_EL1_SRE == 0 {
        return 0;
    }
    // SAFETY: as above; ICC_CTLR_EL1 is only accessible with SRE set.
    unsafe {
        core::arch::asm!("mrs {}, S3_0_C12_C12_4", out(reg) ctlr, options(nomem, nostack, preserves_flags));
    }
    let Some(new) = icc_ctlr_reset_mode(sre, ctlr) else {
        return 0;
    };
    // SAFETY: the caller's contract; the write is followed by an isb, as
    // Zephyr's barrier_isync_fence_full().
    unsafe {
        core::arch::asm!("msr S3_0_C12_C12_4, {}", "isb", in(reg) new, options(nostack, preserves_flags));
    }
    ctlr
}

/// PSCI `SYSTEM_RESET`. Returns only when the firmware did not reset: the
/// PSCI return code (negative: `NOT_SUPPORTED` -1, ...).
///
/// # Safety
/// The firmware implements PSCI over SMC (the caller knows the board).
#[cfg(target_arch = "aarch64")]
pub unsafe fn psci_system_reset() -> i32 {
    let mut x0 = PSCI_SYSTEM_RESET;
    // SAFETY: the caller's contract. SMCCC: x0..x3 may be clobbered.
    unsafe {
        core::arch::asm!(
            "smc #0",
            inout("x0") x0,
            out("x1") _,
            out("x2") _,
            out("x3") _,
            options(nostack),
        );
    }
    x0 as i32
}

/// `CurrentEL` (0..=3).
#[cfg(target_arch = "aarch64")]
pub fn current_el() -> u64 {
    let v: u64;
    // SAFETY: reading CurrentEL has no side effect.
    unsafe {
        core::arch::asm!("mrs {}, CurrentEL", out(reg) v, options(nomem, nostack, preserves_flags));
    }
    (v >> 2) & 3
}

/// Readers of the EL2 registers the early banner prints. Only at EL2.
macro_rules! el2_readers {
    ($($name:ident $reg:literal),* $(,)?) => {$(
        #[doc = concat!("`", $reg, "`. Only at EL2.")]
        #[cfg(target_arch = "aarch64")]
        pub fn $name() -> u64 {
            let v: u64;
            // SAFETY: at EL2 (the caller's state); reading the register has
            // no side effect.
            unsafe {
                core::arch::asm!(concat!("mrs {}, ", $reg), out(reg) v, options(nomem, nostack, preserves_flags));
            }
            v
        }
    )*};
}
el2_readers!(
    read_hcr_el2 "hcr_el2",
    read_sctlr_el2 "sctlr_el2",
    read_vbar_el2 "vbar_el2",
    read_esr_el2 "esr_el2",
    read_elr_el2 "elr_el2",
    read_far_el2 "far_el2",
    read_spsr_el2 "spsr_el2",
);

/// Write `HCR_EL2`, then `isb`.
///
/// # Safety
/// At EL2, before anything below EL2 runs that depends on the old value.
#[cfg(target_arch = "aarch64")]
pub unsafe fn write_hcr_el2(v: u64) {
    // SAFETY: the caller's contract.
    unsafe {
        core::arch::asm!("msr hcr_el2, {}", "isb", in(reg) v, options(nostack, preserves_flags));
    }
}

/// Write `VBAR_EL2`, then `isb`.
///
/// # Safety
/// At EL2; `v` is a 2 KiB aligned vector table that runs with the EL2 MMU in
/// the state it is in.
#[cfg(target_arch = "aarch64")]
pub unsafe fn write_vbar_el2(v: u64) {
    // SAFETY: the caller's contract.
    unsafe {
        core::arch::asm!("msr vbar_el2, {}", "isb", in(reg) v, options(nostack, preserves_flags));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entry_hcr_drops_every_vhe_host_bit_but_rw() {
        // The bits Zephyr's z_arm64_el2_init leaves alone (it clears IMO,
        // AMO, TGE and sets RW): E2H and FMO.
        let zephyr_keeps = |hcr: u64| (hcr & !(HCR_IMO | HCR_AMO | HCR_TGE)) | HCR_RW;
        assert_eq!(zephyr_keeps(HCR_LINUX_VHE_HOST), HCR_RW | HCR_E2H | HCR_FMO);
        assert_eq!(zephyr_keeps(HCR_EL2_AT_ENTRY), HCR_RW);
        assert_eq!(HCR_LINUX_VHE_HOST, 0x4_8800_0038);
        assert_eq!(HCR_EL2_AT_ENTRY, 0x8000_0000);
    }

    #[test]
    fn eoi_mode_is_cleared_only_when_set_and_reachable() {
        // Linux's split mode: EOImode 1 (ICC_CTLR_EL1_EOImode_drop)
        assert_eq!(icc_ctlr_reset_mode(0x7, 0x0000_0402), Some(0x0000_0400));
        assert_eq!(icc_ctlr_reset_mode(0x7, 0x0000_0400), None);
        // SRE off: the CPU interface is as it came out of reset
        assert_eq!(icc_ctlr_reset_mode(0x6, 0x0000_0402), None);
    }
}
