// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The result record without a console on the i.MX 95: the four words of
//! `board::record` in the battery-backed module's general purpose registers
//! (BBNSM GPR4..7), written over SCMI to the System Manager's BBM protocol
//! (NXP vendor protocol 0x81), read back from Linux after the reset
//! (docs/standalone-boot.md, "Toradex Verdin iMX95", "The record without a
//! console").
//!
//! The SM (imx-sm-toradex 3aa8a77a, `sm/rpc/scmi/rpc_scmi_bbm.c`):
//! BBM_GPR_SET (0x3) takes `index, value` and needs EXCLUSIVE on the GPR,
//! BBM_GPR_GET (0x4) takes `index` and returns `value`, needing GET; an index
//! at or past `SM_NUM_GPR` (8 on the i.MX 95, `dev_sm_bbm.h:57`) is
//! NOT_FOUND. Toradex's verdin-imx95 configuration gives the AP-NS agent
//! GPR4..7 at ALL (`configs/verdin-imx95.cfg:756-759`, generated
//! `configs/verdin-imx95/config_scmi.h:453-456`) and none of GPR0..3, which
//! the SM's own board code keeps its shutdown record in
//! (`boards/tdx-common/sm/brd_sm_tdx_common.c:52-53`, read and cleared at
//! every SM boot). The registers are the BBNSM's (`dev_sm_bbm.c:158-210`,
//! `BBNSM_GprSetValue`), in the always-on battery-backed domain: nothing in
//! the SM clears GPR4..7, so the words outlive the A55's and the SM's
//! resets. Whether they outlive the PMIC reset WDOG3's fault takes is the
//! board's to say (pre-study U2, B3).
//!
//! The store keeps the record's rule (`board::record`): the magic word
//! cleared first, then w1..w3, then w0, so a reader that finds the magic
//! can trust the rest, and a store cut off half way never shows the old
//! magic over new words. Linux reads the four words with BBM_GPR_GET
//! (tools/imx95_bbm_record.sh, the same messages through the kernel's SCMI
//! raw interface).

use super::sm::{self, Fault};
use crate::firmware::scmi::{self, Transport};

/// NXP's BBM vendor protocol (Linux `SCMI_PROTOCOL_IMX_BBM`; the SM's
/// `SCMI_PROTOCOL_BBM`, rpc_scmi.h:84).
pub const PROTO_BBM: u8 = 0x81;
/// BBM_GPR_SET: `index, value`.
pub const BBM_GPR_SET: u8 = 0x3;
/// BBM_GPR_GET: `index` -> `value`.
pub const BBM_GPR_GET: u8 = 0x4;
/// The number of GPRs the SM has on the i.MX 95 (`DEV_SM_NUM_GPR`); the low
/// half of PROTOCOL_ATTRIBUTES' word.
pub const NUM_GPR: u32 = 8;
/// The first of the four GPRs that hold the record: w0 in GPR4, w3 in GPR7.
pub const RECORD_GPR: u32 = 4;
/// The record's words.
pub const RECORD_WORDS: usize = 4;

/// Why the record was not stored or read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error {
    /// The GPR whose message failed.
    pub gpr: u32,
    /// BBM_GPR_SET or BBM_GPR_GET.
    pub msg: u8,
    pub fault: Fault,
}

fn set<T: Transport>(t: &mut T, gpr: u32, value: u32) -> Result<(), Error> {
    sm::call(t, PROTO_BBM, BBM_GPR_SET, &[gpr, value], 0).map(|_| ()).map_err(|fault| Error {
        gpr,
        msg: BBM_GPR_SET,
        fault,
    })
}

fn get<T: Transport>(t: &mut T, gpr: u32) -> Result<u32, Error> {
    sm::call(t, PROTO_BBM, BBM_GPR_GET, &[gpr], 1).map(|r| r.get(0)).map_err(|fault| Error {
        gpr,
        msg: BBM_GPR_GET,
        fault,
    })
}

/// The store's messages, in order: (GPR, value). The magic's GPR is cleared
/// first and written last.
pub const fn store_order(w: &[u32; RECORD_WORDS]) -> [(u32, u32); RECORD_WORDS + 1] {
    [(RECORD_GPR, 0), (RECORD_GPR + 1, w[1]), (RECORD_GPR + 2, w[2]), (RECORD_GPR + 3, w[3]), (RECORD_GPR, w[0])]
}

/// Store the record `w` in GPR4..7 ([`store_order`]). Stops at the first
/// message the SM does not answer as asked; GPR4 then holds 0 (or the old
/// magic, when the first message failed), never the new magic over old
/// words.
pub fn store<T: Transport>(t: &mut T, w: &[u32; RECORD_WORDS]) -> Result<(), Error> {
    for (gpr, value) in store_order(w) {
        set(t, gpr, value)?;
    }
    Ok(())
}

/// Read the record from GPR4..7: w0..w3.
pub fn load<T: Transport>(t: &mut T) -> Result<[u32; RECORD_WORDS], Error> {
    let mut w = [0u32; RECORD_WORDS];
    for (i, x) in w.iter_mut().enumerate() {
        *x = get(t, RECORD_GPR + i as u32)?;
    }
    Ok(w)
}

/// The header of a BBM command with `token` (the Linux reader's messages
/// carry their own token; ours carry [`sm::TOKEN`]).
pub const fn header(msg: u8, token: u16) -> u32 {
    scmi::header(PROTO_BBM, msg, token)
}
