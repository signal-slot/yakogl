// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.

//! The 7.1 table's hub-identity check (docs/notes/v3d71-gl-table.md, section 4).
//!
//! Pure over plain register words, no `crate::` import. `HUB_IDENT1` gives
//! `TVER` (3:0) and `REV` (7:4), `ver = 10 * TVER + REV`
//! (driver/src/v3d/v3d_hw.c, `v3d_identify`). `HUB_IDENT3` gives `IPREV`
//! (15:8) and Mesa's `compat_rev` (23:16) (driver/docs/rpi5-v3d71-facts.md).
//! The table serves 7.1.10 (BCM2712 D0) only.

/// The lowest IPREV the 7.1 table accepts.
pub const MIN_IPREV: u32 = 10;

/// The decoded hub identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HubIdent {
    pub tver: u32,
    pub rev: u32,
    /// `10 * tver + rev`: 42 for the Pi 4, 71 for the Pi 5.
    pub ver: u32,
    pub iprev: u32,
    pub compat_rev: u32,
}

/// Why the 7.1 table refuses to start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A V3D 4.2 (Pi 4): the other table serves it.
    V3d42 { iprev: u32 },
    /// A 7.1 older than 7.1.10.
    OldIprev { iprev: u32 },
    /// Neither 4.2 nor 7.1.
    UnknownVersion { tver: u32, rev: u32, ver: u32 },
}

impl core::fmt::Display for Refusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Refusal::V3d42 { .. } => write!(f, "V3D 4.2: build gpu-v3d"),
            Refusal::OldIprev { iprev } => write!(f, "V3D 7.1.{iprev}: this table is 7.1.10 (BCM2712 D0) only"),
            Refusal::UnknownVersion { tver, rev, ver } => {
                write!(f, "V3D {tver}.{rev} (ver {ver}): unknown version, this table is 7.1.10 (BCM2712 D0) only")
            }
        }
    }
}

/// Decode `HUB_IDENT1` and `HUB_IDENT3`.
pub fn decode(hub_ident1: u32, hub_ident3: u32) -> HubIdent {
    let tver = hub_ident1 & 0xf;
    let rev = (hub_ident1 >> 4) & 0xf;
    HubIdent { tver, rev, ver: tver * 10 + rev, iprev: (hub_ident3 >> 8) & 0xff, compat_rev: (hub_ident3 >> 16) & 0xff }
}

/// Accept only 7.1 with IPREV >= 10.
pub fn check(id: &HubIdent) -> Result<(), Refusal> {
    match id.ver {
        71 if id.iprev >= MIN_IPREV => Ok(()),
        71 => Err(Refusal::OldIprev { iprev: id.iprev }),
        42 => Err(Refusal::V3d42 { iprev: id.iprev }),
        ver => Err(Refusal::UnknownVersion { tver: id.tver, rev: id.rev, ver }),
    }
}

/// Decode and check in one step.
pub fn check_words(hub_ident1: u32, hub_ident3: u32) -> Result<HubIdent, Refusal> {
    let id = decode(hub_ident1, hub_ident3);
    check(&id).map(|()| id)
}
