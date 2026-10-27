// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The NanoPC-T6 image's early banner: raw lines on uart2, written with
//! [`crate::ip::ns16550_tx`] before Zephyr's console exists, so a jump that
//! dies says where (driver/docs/nanopc-t6-mali.md, "Z1", "Where an image
//! dies"). Each line starts with its tag; a tag that is absent on the UART
//! means the image did not get there.
//!
//! - `T6:A` the image's first code (reset.S's `z_arm64_el2_plat_prep_c`, or
//!   the EL1 one), MMU off: the EL it was entered at and, at EL2, `HCR_EL2`
//!   found and written ([`crate::aarch64::HCR_EL2_AT_ENTRY`]), `SCTLR_EL2`,
//!   and `VBAR_EL2` found and set (the trap reporter below).
//! - `T6:A1` the end of Zephyr's `z_arm64_el1_init` (`z_arm64_el1_plat_init`),
//!   MMU off: EL1 runs and its timer register writes did not trap.
//! - `T6:A2` the first PRE_KERNEL_1 hook, MMU on (through a `device_map()`
//!   of uart2): `z_prep_c` (BSS, MMU tables) and the EARLY hooks passed.
//! - `T6:A3` the same hook after the WDT0 arming, with its result.
//! - `T6:X` an exception taken to EL2 after `T6:A`: its syndrome, then the
//!   CPU halts (WDT0, if it runs, resets the board).
//!
//! `T6:B` and `T6:C` are printed by the glue through Zephyr's console
//! (nanopc_t6.c), not here.
//!
//! Everything here must run with the MMU off and no `.data`/`.bss`
//! initialised: no statics, no `core::fmt`, no slices copied (a `memcpy` or
//! `memset` from the C library may use `dc zva`, which faults on the Device
//! memory that every access is with the MMU off). The target has
//! `+strict-align`, so the compiler emits no unaligned access either.

use crate::ip::ns16550_tx::Tx;
use crate::mmio::Mmio;

/// uart2, the debug header (Linux rk3588-base.dtsi, `serial@feb50000`;
/// Zephyr's rk3588.dtsi `uart2`, reg 0xfeb50000).
pub const UART2_PHYS: usize = 0xfeb5_0000;

/// What the first code found at EL2 and what it left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct El2Entry {
    pub hcr_found: u64,
    /// `None`: the VHE exit is configured off and `HCR_EL2` was kept.
    pub hcr_now: Option<u64>,
    pub sctlr: u64,
    pub vbar_found: u64,
    pub vbar_now: u64,
}

/// What an exception taken to EL2 left in the EL2 registers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct El2Trap {
    /// The vector's offset in the table (0x000..=0x780, a multiple of 0x80).
    pub vector: u64,
    pub esr: u64,
    pub elr: u64,
    pub far: u64,
    pub spsr: u64,
    pub hcr: u64,
}

pub fn banner_a_el2<M: Mmio>(tx: &mut Tx<'_, M>, e: &El2Entry) {
    tx.str(b"T6:A EL2 HCR_EL2 ");
    tx.hex(e.hcr_found, 16);
    match e.hcr_now {
        Some(v) => {
            tx.str(b" -> ");
            tx.hex(v, 16);
        }
        None => tx.str(b" kept (VHE exit off)"),
    }
    tx.str(b" SCTLR_EL2 ");
    tx.hex(e.sctlr, 16);
    tx.str(b" VBAR_EL2 ");
    tx.hex(e.vbar_found, 16);
    tx.str(b" -> ");
    tx.hex(e.vbar_now, 16);
    tx.str(b"\r\n");
}

/// `T6:A` entered at an EL other than 2: nothing at EL2 was changed.
pub fn banner_a_other<M: Mmio>(tx: &mut Tx<'_, M>, el: u64) {
    tx.str(b"T6:A EL");
    tx.dec(el as i64);
    tx.str(b" (not EL2: HCR_EL2 and VBAR_EL2 untouched)\r\n");
}

pub fn banner_a1<M: Mmio>(tx: &mut Tx<'_, M>) {
    tx.str(b"T6:A1 EL1 init done, MMU off\r\n");
}

pub fn banner_a2<M: Mmio>(tx: &mut Tx<'_, M>) {
    tx.str(b"T6:A2 PRE_KERNEL_1, MMU on: arming WDT0\r\n");
}

pub fn banner_a3<M: Mmio>(tx: &mut Tx<'_, M>, rc: i32, cr_now: u32, period_ms: u32) {
    tx.str(b"T6:A3 WDT0 arm rc ");
    tx.dec(i64::from(rc));
    tx.str(b" CR ");
    tx.hex(u64::from(cr_now), 8);
    tx.str(b" period_ms ");
    tx.dec(i64::from(period_ms));
    tx.str(b"\r\n");
}

pub fn banner_x<M: Mmio>(tx: &mut Tx<'_, M>, t: &El2Trap) {
    tx.str(b"T6:X EL2 exception, vector +");
    tx.hex(t.vector, 3);
    tx.str(b": ESR_EL2 ");
    tx.hex(t.esr, 16);
    tx.str(b" ELR_EL2 ");
    tx.hex(t.elr, 16);
    tx.str(b" FAR_EL2 ");
    tx.hex(t.far, 16);
    tx.str(b" SPSR_EL2 ");
    tx.hex(t.spsr, 16);
    tx.str(b" HCR_EL2 ");
    tx.hex(t.hcr, 16);
    tx.str(b"; halted\r\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aarch64::{HCR_EL2_AT_ENTRY, HCR_LINUX_VHE_HOST};
    use crate::ip::ns16550_tx::tests::{Op, Uart, BASE};
    use crate::ip::ns16550_tx::{LCR, LSR, THR};

    fn sent(f: impl FnOnce(&mut Tx<'_, Uart>)) -> (String, Uart) {
        let u = Uart::linux_left(1);
        {
            let mut tx = Tx::new(&u, BASE);
            f(&mut tx);
            assert_eq!(tx.timeouts, 0);
        }
        (String::from_utf8(u.sent()).unwrap(), u)
    }

    #[test]
    fn banner_a_at_el2_names_both_hcr_values() {
        let e = El2Entry {
            hcr_found: HCR_LINUX_VHE_HOST,
            hcr_now: Some(HCR_EL2_AT_ENTRY),
            sctlr: 0x30c5_0838,
            vbar_found: 0xffff_8000_8001_0800,
            vbar_now: 0x1000_8000,
        };
        let (s, u) = sent(|tx| banner_a_el2(tx, &e));
        assert_eq!(
            s,
            "T6:A EL2 HCR_EL2 0x0000000488000038 -> 0x0000000080000000 SCTLR_EL2 0x0000000030c50838 \
             VBAR_EL2 0xffff800080010800 -> 0x0000000010008000\r\n"
        );
        // The register sequence: LCR once, then per byte LSR until THRE and
        // one THR store; nothing else (the model refuses any other access).
        let ops = u.ops.borrow();
        assert_eq!(ops[0], Op::R(BASE + LCR, 0x03));
        let mut i = 1;
        for b in s.bytes() {
            assert_eq!(ops[i], Op::R(BASE + LSR, 0x00));
            assert_eq!(ops[i + 1], Op::R(BASE + LSR, 0x60));
            assert_eq!(ops[i + 2], Op::W(BASE + THR, u32::from(b)));
            i += 3;
        }
        assert_eq!(i, ops.len());
    }

    #[test]
    fn banner_a_with_the_exit_off_says_kept() {
        let e = El2Entry { hcr_found: HCR_LINUX_VHE_HOST, hcr_now: None, sctlr: 0, vbar_found: 0, vbar_now: 0 };
        let (s, _) = sent(|tx| banner_a_el2(tx, &e));
        assert!(s.starts_with("T6:A EL2 HCR_EL2 0x0000000488000038 kept (VHE exit off) SCTLR_EL2 "), "{s}");
    }

    #[test]
    fn the_other_lines() {
        assert_eq!(sent(|tx| banner_a_other(tx, 1)).0, "T6:A EL1 (not EL2: HCR_EL2 and VBAR_EL2 untouched)\r\n");
        assert_eq!(sent(banner_a1).0, "T6:A1 EL1 init done, MMU off\r\n");
        assert_eq!(sent(banner_a2).0, "T6:A2 PRE_KERNEL_1, MMU on: arming WDT0\r\n");
        assert_eq!(sent(|tx| banner_a3(tx, 0, 0x9, 22369)).0, "T6:A3 WDT0 arm rc 0 CR 0x00000009 period_ms 22369\r\n");
        assert_eq!(sent(|tx| banner_a3(tx, -5, 0x8, 0)).0, "T6:A3 WDT0 arm rc -5 CR 0x00000008 period_ms 0\r\n");
        // Arbitrary values: this pins the format only, not a measured trap.
        let t = El2Trap { vector: 0x400, esr: 0x6238_0000, elr: 0x1000_0f00, far: 0, spsr: 0x3c5, hcr: 0x4_8000_0008 };
        assert_eq!(
            sent(|tx| banner_x(tx, &t)).0,
            "T6:X EL2 exception, vector +0x400: ESR_EL2 0x0000000062380000 ELR_EL2 0x0000000010000f00 \
             FAR_EL2 0x0000000000000000 SPSR_EL2 0x00000000000003c5 HCR_EL2 0x0000000480000008; halted\r\n"
        );
    }
}
