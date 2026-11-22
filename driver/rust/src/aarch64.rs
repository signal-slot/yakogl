// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! What an AArch64 image entered from Linux needs from the CPU itself: the
//! GICv3 CPU interface put back into its reset EOI mode with no active
//! priority left ([`gic_cpuif_reset`]), and the way back, PSCI
//! `SYSTEM_RESET`.
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
//!
//! **The two entry states.** Through Linux's kexec the image starts at EL2
//! with `HCR_EL2` = [`HCR_LINUX_VHE_HOST`] (E2H set), the MMU off and the GICv3
//! CPU interface in split EOI mode. Through U-Boot's `go` (U-Boot v2026.07, the
//! cited tag; the facts are in the tests below) it starts at EL2 with E2H clear
//! and AMO set, the EL2 MMU and both caches on, and the CPU interface as
//! TF-A left it, because U-Boot does not touch it on the RK3588. The entry
//! write of [`HCR_EL2_AT_ENTRY`] and the CPU interface reset are right for
//! both.

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
/// `ICC_CTLR_EL1` accesses to the virtual interface, so the CPU interface
/// reset below would not reach the physical one. U-Boot leaves E2H clear,
/// which is why Zephyr's roc_rk3588_pc (U-Boot `go`) never meets this.
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
/// `ICC_CTLR_EL1.PRIbits`, bits 10:8: the number of priority bits less one
/// (Linux v6.18 include/linux/irqchip/arm-gic-v3.h:556-557, read by
/// irq-gic-v3.c:119-129 `gic_get_pribits`).
pub const ICC_CTLR_EL1_PRIBITS_SHIFT: u32 = 8;
pub const ICC_CTLR_EL1_PRIBITS_MASK: u64 = 0x7 << ICC_CTLR_EL1_PRIBITS_SHIFT;

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

/// The number of priority bits the CPU interface implements, from
/// `ICC_CTLR_EL1.PRIbits` (1..=8).
pub const fn icc_pribits(ctlr: u64) -> u32 {
    ((ctlr & ICC_CTLR_EL1_PRIBITS_MASK) >> ICC_CTLR_EL1_PRIBITS_SHIFT) as u32 + 1
}

/// How many `ICC_AP1Rn_EL1` the CPU interface implements for `pribits`, as
/// Linux v6.18 clears them (irq-gic-v3.c:1264-1276, `gic_cpu_sys_reg_init`):
/// 4 or 5 bits AP1R0, 6 bits AP1R0..1, 7 or 8 bits AP1R0..3, fewer than 4
/// none. A register outside the count is not implemented, and an access to
/// it is UNDEFINED: the count is what keeps the clear from trapping.
pub const fn icc_ap1r_count(pribits: u32) -> usize {
    match pribits {
        4 | 5 => 1,
        6 => 2,
        7 | 8 => 4,
        _ => 0,
    }
}

/// A GICv3 CPU interface register the entry reset reads or writes. All are
/// the EL1 ones, Group 1 and common: Group 0's `ICC_AP0Rn_EL1` is left alone
/// ([`gic_cpuif_reset`] says why).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icc {
    /// `ICC_SRE_EL1`, S3_0_C12_C12_5.
    Sre,
    /// `ICC_CTLR_EL1`, S3_0_C12_C12_4.
    Ctlr,
    /// `ICC_BPR1_EL1`, S3_0_C12_C12_3.
    Bpr1,
    /// `ICC_AP1R0_EL1` .. `ICC_AP1R3_EL1`, S3_0_C12_C9_n.
    Ap1r0,
    Ap1r1,
    Ap1r2,
    Ap1r3,
}

/// This CPU's GICv3 CPU interface registers: the CPU's own on aarch64
/// (`SysRegs`), a model in the host tests.
pub trait IccRegs {
    fn read(&mut self, r: Icc) -> u64;
    /// The write alone; [`IccRegs::isb`] makes it take effect.
    fn write(&mut self, r: Icc, v: u64);
    fn isb(&mut self);
}

/// What [`gic_cpuif_reset`] found and did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GicCpuifReset {
    /// `ICC_SRE_EL1` as found. With SRE clear nothing else was read or
    /// written, and the fields below are 0.
    pub sre: u64,
    /// `ICC_CTLR_EL1`, `ICC_AP1R0_EL1` and `ICC_BPR1_EL1` as found, before
    /// any write. `ap1r0` is 0 when the interface implements no AP1R.
    pub ctlr: u64,
    pub ap1r0: u64,
    pub bpr1: u64,
    /// `ICC_CTLR_EL1` was written with EOImode 0 ([`icc_ctlr_reset_mode`]).
    pub eoimode_cleared: bool,
    /// The number of `ICC_AP1Rn_EL1` written 0 ([`icc_ap1r_count`]).
    pub ap1r_cleared: u32,
    /// `ICC_BPR1_EL1` was written 0.
    pub bpr1_written: bool,
}

/// Put this CPU's GICv3 CPU interface back into the state Zephyr's GICv3
/// driver assumes, before that driver's init and before interrupts are
/// enabled. A kernel before us (Linux's kexec, a crash kexec above all) can
/// leave three things that the driver's init never writes (zephyr af17c0c8
/// drivers/interrupt_controller/intc_gicv3.c:489-542 `gicv3_cpuif_init`
/// writes `ICC_SRE_EL1`, `ICC_PMR_EL1` and `ICC_IGRPEN1_EL1` only):
///
/// - `ICC_CTLR_EL1.EOImode` 1, split EOI/deactivate (the module's "EOI
///   mode").
/// - A bit set in `ICC_AP1Rn_EL1`: an interrupt the kernel had taken and not
///   ended when it handed over, the crashing CPU's own on a crash kexec. The
///   running priority stays raised, and no interrupt of that priority or a
///   lower one is taken, the system timer's included: the image hangs at its
///   first sleep. Written 0, as Linux's own init does for this hand-over
///   (Linux v6.18 drivers/irqchip/irq-gic-v3.c:1264-1278), for as many
///   registers as `ICC_CTLR_EL1.PRIbits` says exist ([`icc_ap1r_count`]).
/// - `ICC_BPR1_EL1` changed from its reset value. Written 0, as Linux does:
///   "Writing a zero to BPR restores is reset value" (irq-gic-v3.c:1229-1235).
///
/// The order is Linux's: `ICC_BPR1_EL1`, `ICC_CTLR_EL1` and an `isb`
/// (`gic_write_ctlr`, arch/arm64/include/asm/arch_gicv3.h:90-94), the
/// `ICC_AP1Rn_EL1` from the highest down, an `isb` (irq-gic-v3.c:1229-1278).
///
/// Not written: `ICC_AP0Rn_EL1`. Linux clears them only when Group 0 is
/// reachable from Non-secure EL1 (irq-gic-v3.c:1245-1262 under
/// `gic_has_group0`, :131-155, which probes `ICC_PMR_EL1` because with
/// `SCR_EL3.FIQ` set Group 0 belongs to EL3). The image is not EL3 and has
/// no Group 0 interrupt: Zephyr in Non-secure EL1 makes every interrupt Group
/// 1 (intc_gicv3.c:53-58, `IGROUPR_VAL`), and so does Linux (irq-gic-v3.c:989-
/// 990 for the SPIs, :1342-1343 for the SGIs and PPIs), so the kernel before
/// us cannot have left a Group 0 priority active either. Leaving them alone
/// spares a probe and an access that could trap to EL3.
///
/// Nothing is read or written past `ICC_SRE_EL1` when its SRE bit is clear:
/// the `ICC_*_EL1` registers are then not reachable through system register
/// accesses (Linux: irq-gic-v3.c:1188-1198, "SRE is disabled at EL2. We're
/// going to die painfully"). A CPU interface not used since reset has
/// nothing active.
///
/// The image runs this at EL1 (the T6's EARLY hook, after reset.S's
/// `z_arm64_el2_init` and the `eret` to EL1, zephyr af17c0c8
/// arch/arm64/core/reset.S:234-248). With the VHE exit `HCR_EL2` is
/// [`HCR_EL2_AT_ENTRY`] there, FMO and IMO clear, so the accesses reach the
/// physical interface, not the virtual one.
pub fn gic_cpuif_reset<R: IccRegs>(r: &mut R) -> GicCpuifReset {
    let sre = r.read(Icc::Sre);
    let mut out = GicCpuifReset { sre, ..GicCpuifReset::default() };
    if sre & ICC_SRE_EL1_SRE == 0 {
        return out;
    }
    out.ctlr = r.read(Icc::Ctlr);
    let n = icc_ap1r_count(icc_pribits(out.ctlr));
    if n > 0 {
        out.ap1r0 = r.read(Icc::Ap1r0);
    }
    out.bpr1 = r.read(Icc::Bpr1);
    r.write(Icc::Bpr1, 0);
    out.bpr1_written = true;
    if let Some(new) = icc_ctlr_reset_mode(sre, out.ctlr) {
        r.write(Icc::Ctlr, new);
        r.isb();
        out.eoimode_cleared = true;
    }
    // Linux's order, the highest first; each write names its register, so no
    // other one can be reached by an index.
    if n >= 4 {
        r.write(Icc::Ap1r3, 0);
        r.write(Icc::Ap1r2, 0);
    }
    if n >= 2 {
        r.write(Icc::Ap1r1, 0);
    }
    if n >= 1 {
        r.write(Icc::Ap1r0, 0);
    }
    out.ap1r_cleared = n as u32;
    r.isb();
    out
}

/// The CPU's own `ICC_*_EL1` registers.
#[cfg(target_arch = "aarch64")]
pub struct SysRegs(());

#[cfg(target_arch = "aarch64")]
impl SysRegs {
    /// # Safety
    /// At EL1 on the CPU whose interface is meant, before interrupts are
    /// enabled; the caller reads and writes only what [`gic_cpuif_reset`]
    /// does, which checks SRE and the AP1R count first.
    pub unsafe fn new() -> Self {
        SysRegs(())
    }
}

#[cfg(target_arch = "aarch64")]
macro_rules! icc_mrs {
    ($enc:literal) => {{
        let v: u64;
        // SAFETY: SysRegs::new's contract; a read has no side effect.
        unsafe {
            core::arch::asm!(concat!("mrs {}, ", $enc), out(reg) v, options(nomem, nostack, preserves_flags));
        }
        v
    }};
}

#[cfg(target_arch = "aarch64")]
macro_rules! icc_msr {
    ($enc:literal, $v:expr) => {{
        // SAFETY: SysRegs::new's contract.
        unsafe {
            core::arch::asm!(concat!("msr ", $enc, ", {}"), in(reg) $v, options(nostack, preserves_flags));
        }
    }};
}

#[cfg(target_arch = "aarch64")]
impl IccRegs for SysRegs {
    fn read(&mut self, r: Icc) -> u64 {
        match r {
            Icc::Sre => icc_mrs!("S3_0_C12_C12_5"),
            Icc::Ctlr => icc_mrs!("S3_0_C12_C12_4"),
            Icc::Bpr1 => icc_mrs!("S3_0_C12_C12_3"),
            Icc::Ap1r0 => icc_mrs!("S3_0_C12_C9_0"),
            Icc::Ap1r1 => icc_mrs!("S3_0_C12_C9_1"),
            Icc::Ap1r2 => icc_mrs!("S3_0_C12_C9_2"),
            Icc::Ap1r3 => icc_mrs!("S3_0_C12_C9_3"),
        }
    }

    fn write(&mut self, r: Icc, v: u64) {
        match r {
            Icc::Sre => icc_msr!("S3_0_C12_C12_5", v),
            Icc::Ctlr => icc_msr!("S3_0_C12_C12_4", v),
            Icc::Bpr1 => icc_msr!("S3_0_C12_C12_3", v),
            Icc::Ap1r0 => icc_msr!("S3_0_C12_C9_0", v),
            Icc::Ap1r1 => icc_msr!("S3_0_C12_C9_1", v),
            Icc::Ap1r2 => icc_msr!("S3_0_C12_C9_2", v),
            Icc::Ap1r3 => icc_msr!("S3_0_C12_C9_3", v),
        }
    }

    fn isb(&mut self) {
        // SAFETY: a barrier.
        unsafe {
            core::arch::asm!("isb", options(nostack, preserves_flags));
        }
    }
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

    /// `ICC_AP1Rn_EL1` by n.
    const ICC_AP1R: [Icc; 4] = [Icc::Ap1r0, Icc::Ap1r1, Icc::Ap1r2, Icc::Ap1r3];

    /// A CPU interface as the rules [`gic_cpuif_reset`] cites treat it:
    /// with SRE clear only `ICC_SRE_EL1` is reachable, and an `ICC_AP1Rn_EL1`
    /// past the PRIbits count is not implemented. Either access panics here
    /// where the board would trap.
    struct Cpu {
        sre: u64,
        ctlr: u64,
        bpr1: u64,
        ap1r: [u64; 4],
        writes: Vec<(Icc, u64)>,
        /// Writes since the last isb.
        unsynced: usize,
    }

    impl Cpu {
        fn new(sre: u64, ctlr: u64, bpr1: u64, ap1r: [u64; 4]) -> Self {
            Cpu { sre, ctlr, bpr1, ap1r, writes: Vec::new(), unsynced: 0 }
        }

        fn reg(&mut self, r: Icc) -> &mut u64 {
            if r != Icc::Sre {
                assert_ne!(self.sre & ICC_SRE_EL1_SRE, 0, "{r:?} with SRE clear");
            }
            let n = icc_ap1r_count(icc_pribits(self.ctlr));
            match r {
                Icc::Sre => &mut self.sre,
                Icc::Ctlr => &mut self.ctlr,
                Icc::Bpr1 => &mut self.bpr1,
                _ => {
                    let i = ICC_AP1R.iter().position(|&a| a == r).unwrap();
                    assert!(i < n, "{r:?} is not implemented with PRIbits {}", icc_pribits(self.ctlr));
                    &mut self.ap1r[i]
                }
            }
        }
    }

    impl IccRegs for Cpu {
        fn read(&mut self, r: Icc) -> u64 {
            *self.reg(r)
        }
        fn write(&mut self, r: Icc, v: u64) {
            *self.reg(r) = v;
            self.writes.push((r, v));
            self.unsynced += 1;
        }
        fn isb(&mut self) {
            self.unsynced = 0;
        }
    }

    // The T6's ICC_CTLR_EL1 as a kexec from Armbian left it (board record
    // of 2026-10-19, driver/docs/nanopc-t6-mali.md "ICC_CTLR_EL1 0x8402 had
    // EOImode 1"): A3V, PRIbits 4 (5 bits), EOImode 1.
    const T6_LINUX_CTLR: u64 = 0x8402;

    #[test]
    fn the_t6_has_five_priority_bits_and_one_ap1r() {
        assert_eq!(icc_pribits(T6_LINUX_CTLR), 5);
        assert_eq!(icc_ap1r_count(5), 1);
        // Linux's switch, bit count by bit count; fewer than 4 writes none
        let counts: Vec<usize> = (1..=8).map(icc_ap1r_count).collect();
        assert_eq!(counts, [0, 0, 0, 1, 1, 2, 4, 4]);
        assert_eq!(icc_pribits(0x0700), 8);
        assert_eq!(icc_pribits(0x0000), 1);
    }

    #[test]
    fn a_crash_kexec_left_active_priority_and_bpr1_are_cleared() {
        // A crash on the CPU while it handled an interrupt: one bit of
        // AP1R0 set (which one follows the interrupt's priority; any bit
        // raises the running priority). BPR1 left at 7, the kind of value
        // Linux's comment calls "large enough to prevent any pre-emptive
        // interrupts" (irq-gic-v3.c:1229-1233).
        let mut cpu = Cpu::new(0x7, T6_LINUX_CTLR, 7, [1 << 20, 0, 0, 0]);
        let got = gic_cpuif_reset(&mut cpu);
        assert_eq!(cpu.ap1r[0], 0, "the active priority stays raised");
        assert_eq!(cpu.bpr1, 0);
        assert_eq!(cpu.ctlr, 0x8400);
        assert_eq!(cpu.unsynced, 0, "no isb after the last write");
        // Linux's order: BPR1, CTLR (with its isb), the AP1Rn
        assert_eq!(cpu.writes, [(Icc::Bpr1, 0), (Icc::Ctlr, 0x8400), (Icc::Ap1r0, 0)]);
        // the record holds what was found
        assert_eq!(
            got,
            GicCpuifReset {
                sre: 0x7,
                ctlr: T6_LINUX_CTLR,
                ap1r0: 1 << 20,
                bpr1: 7,
                eoimode_cleared: true,
                ap1r_cleared: 1,
                bpr1_written: true,
            }
        );
    }

    #[test]
    fn every_implemented_ap1r_and_no_other_is_written() {
        for pribits in 1..=8u32 {
            let ctlr = u64::from(pribits - 1) << ICC_CTLR_EL1_PRIBITS_SHIFT;
            let mut cpu = Cpu::new(0x7, ctlr, 0, [!0; 4]);
            // reaching past the count panics in the model
            let got = gic_cpuif_reset(&mut cpu);
            let n = icc_ap1r_count(pribits);
            assert_eq!(got.ap1r_cleared as usize, n);
            for i in 0..4 {
                assert_eq!(cpu.ap1r[i], if i < n { 0 } else { !0 }, "PRIbits {pribits} AP1R{i}");
            }
            assert_eq!(got.ap1r0, if n > 0 { !0 } else { 0 });
            assert!(!got.eoimode_cleared);
            assert_eq!(cpu.unsynced, 0);
            // the highest first, as Linux
            let aps: Vec<Icc> = cpu.writes.iter().map(|w| w.0).filter(|r| ICC_AP1R.contains(r)).collect();
            let want: Vec<Icc> = ICC_AP1R[..n].iter().rev().copied().collect();
            assert_eq!(aps, want);
        }
    }

    #[test]
    fn with_sre_clear_only_sre_is_read() {
        // the model panics on any other access
        let mut cpu = Cpu::new(0x6, T6_LINUX_CTLR, 7, [1, 0, 0, 0]);
        let got = gic_cpuif_reset(&mut cpu);
        assert_eq!(got, GicCpuifReset { sre: 0x6, ..GicCpuifReset::default() });
        assert!(cpu.writes.is_empty());
    }

    #[test]
    fn a_clean_interface_is_written_the_same_values() {
        // U-Boot's go with SRE on and nothing active: the writes are the
        // reset values the registers already hold, EOImode is not touched
        let mut cpu = Cpu::new(0x7, 0x0400, 0, [0; 4]);
        let got = gic_cpuif_reset(&mut cpu);
        assert_eq!(cpu.writes, [(Icc::Bpr1, 0), (Icc::Ap1r0, 0)]);
        assert_eq!((cpu.ctlr, cpu.bpr1, cpu.ap1r[0]), (0x0400, 0, 0));
        assert!(!got.eoimode_cleared && got.bpr1_written && got.ap1r_cleared == 1);
    }

    #[test]
    fn eoi_mode_is_cleared_only_when_set_and_reachable() {
        // Linux's split mode: EOImode 1 (ICC_CTLR_EL1_EOImode_drop)
        assert_eq!(icc_ctlr_reset_mode(0x7, 0x0000_0402), Some(0x0000_0400));
        assert_eq!(icc_ctlr_reset_mode(0x7, 0x0000_0400), None);
        // SRE off: the CPU interface is as it came out of reset
        assert_eq!(icc_ctlr_reset_mode(0x6, 0x0000_0402), None);
    }

    // U-Boot v2026.07 (GPL-2.0; facts and file:line only), `go` on the RK3588
    // where TF-A hands BL33 over at EL2:
    // - arch/arm/cpu/armv8/start.S:127-130: at EL2 it reads HCR_EL2 and, with
    //   E2H clear, ORs in AMO (HCR_EL2_AMO_EL2, arch/arm/include/asm/system.h:87)
    //   and writes it back. E2H set would take the label-1 branch (start.S:128)
    //   and write no HCR_EL2. No other HCR_EL2 write on the path to `go`: the
    //   armv8_switch_to_el2 macros (arch/arm/include/asm/macro.h:293-314) are
    //   not reached, the CPU is already at EL2.
    // - cmd/boot.c:18-22, 41: `go` is a plain call of the entry, no cleanup.
    const UBOOT_HCR_AT_GO: u64 = HCR_RW | HCR_AMO;
    // - SCTLR_EL2: MMU on (arch/arm/cpu/armv8/cache_v8.c:815-817 mmu_enable,
    //   called from dcache_enable at :880-887), D-cache on (:894), I-cache on
    //   (:1172-1175), all from enable_caches (cache_v8.c:1224-1227, reached from
    //   common/board_r.c:108-113 initr_caches; arch/arm/mach-rockchip/board.c:209
    //   is the ARM32-only override, #if !CONFIG_ARM64). The reserved bits are
    //   illustrative; M, C and I are the facts.
    const UBOOT_SCTLR_EL2_AT_GO: u64 = 0x30c5_0830 | (1 << 0) | (1 << 2) | (1 << 12);
    // - GICv3 / ICC_*: start.S:301-319 gic_init_secure* run in lowlevel_init
    //   only under CONFIG_GICV2/CONFIG_GICV3, and arch/arm/lib/gic_64.S:123-126
    //   (the ICC_SRE_EL3 write) is EL3 code; GICV3 is a bare option
    //   (arch/arm/Kconfig:129) that only ARCH_VERSAL selects (:1294) and
    //   configs/nanopc-t6-rk3588_defconfig sets no GIC option. No ICC_* write
    //   for EL1 anywhere in U-Boot's armv8 boot path: ICC_SRE_EL1 and
    //   ICC_CTLR_EL1 are as TF-A left them (reset: SRE 0, EOImode 0; an Arm
    //   architecture fact, not U-Boot's).

    #[test]
    fn uboot_go_state_needs_no_eoi_mode_write() {
        // SRE off as out of reset: not reachable, nothing written
        assert_eq!(icc_ctlr_reset_mode(0x0, 0x0000_0400), None);
        // SRE on (a TF-A or driver that enabled it) with EOImode already 0
        assert_eq!(icc_ctlr_reset_mode(0x7, 0x0000_0400), None);
        assert_eq!(icc_ctlr_reset_mode(0x1, 0x0000_0000), None);
        // and a U-Boot ICC_CTLR_EL1 with other bits set is never altered
        assert_eq!(icc_ctlr_reset_mode(0x7, 0x0000_0001), None);
    }

    #[test]
    fn the_entry_write_is_right_from_uboots_hcr() {
        assert_eq!(UBOOT_HCR_AT_GO & HCR_E2H, 0);
        assert_eq!(UBOOT_HCR_AT_GO, 0x8000_0020);
        // The write is the whole register, so E2H clear is not special: the
        // result is the same as from Linux's VHE value.
        assert_eq!(HCR_EL2_AT_ENTRY & !HCR_RW, 0);
        // Zephyr's EL2 init after the write sees RW only (AMO from U-Boot
        // is dropped, as it would clear it itself): nothing for E2H to undo.
        let zephyr_keeps = |hcr: u64| (hcr & !(HCR_IMO | HCR_AMO | HCR_TGE)) | HCR_RW;
        assert_eq!(zephyr_keeps(HCR_EL2_AT_ENTRY), HCR_RW);
        // Without the write, U-Boot's value would already be right too: the
        // exit changes nothing Zephyr's init does not, so it is a no-op in effect.
        assert_eq!(zephyr_keeps(UBOOT_HCR_AT_GO), zephyr_keeps(HCR_EL2_AT_ENTRY));
        // The EL2 MMU is on at U-Boot's go, unlike Linux's kexec.
        assert_eq!(UBOOT_SCTLR_EL2_AT_GO & 0x1005, 0x1005);
    }
}
