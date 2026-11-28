// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A reader for the firmware's HWPerf packet stream (docs/notes/pvr-hwperf.md,
//! "(b) Events, packet layout, clock" and "(d) Diff sketch").
//!
//! The firmware, with HWPerf on, writes one packet per event into a buffer the
//! host reads: when a TA or 3D task started and finished on the hardware, when
//! its background task (the CCB processing) and its IRQ task ran, when it
//! checked and updated a kick's fences. The stamps are the firmware's own, so
//! they split the time between a post and a fence that the host's reads
//! cannot (docs/notes/pvr-fence-wait.md, docs/notes/am62p-slow-terrain.md).
//!
//! A pure decoder over a byte slice ([`Reader`]), a per-kick summary
//! ([`summarize`]) and the drain of the firmware's buffer ([`drain`],
//! [`drain_log`], `yd_pvr_hwperf_drain`). The firmware init turns HWPerf on
//! with `fwinit::FWINIT_HWPERF` (Kconfig `CONFIG_YAKOGL_DRV_GPU_PVR_HWPERF`,
//! off by default): the buffer, its size and [`PROBE_FILTER`]. Without it the
//! drain finds a size of 0 and reads nothing. There is no allocator: the
//! reader borrows the slice and the summary writes into a slice the caller
//! gives.
//!
//! Layout. Every number here comes from the vendored DDK 25.2 headers
//! (`third_party/ti-img-rogue-ddk-25.2`, `UPSTREAM`: rogueddk 25.2@6850647),
//! the ones our firmware was built from. Paths below are relative to its
//! `include/` directory. The packets are little-endian (the firmware's and the
//! host's byte order).
//!
//! Clock. The firmware stamps with `RGX_CR_TIMER`, "it increments by 1 every
//! 256 GPU clock ticks" (`rogue/rgx_fwif_km.h:1898-1901`). Whether the
//! firmware writes that raw value into `ui64Timestamp` or a converted one is
//! not in the headers (the note's (b), "[knowledge]"); the reader keeps the
//! stamp as it is, in ticks, and [`ticks_to_ns`] converts a difference at a
//! given core clock.

use super::fwif::{
    rd32, wr32, FW_CONFIG_HEAP_ALLOC_GRANULARITY, FW_HEAP_SIZE, HWPERFDATA_DROP_COUNT, HWPERFDATA_RIDX,
    HWPERFDATA_SIZE, HWPERFDATA_SIZE_FIELD, HWPERFDATA_WIDX, HWPERFDATA_WRAP_COUNT, OSINIT_HWPERF_DATA,
    OSINIT_HWPERF_RUNTIME_CFG, RUNTIME_HWPERFCFG_BUF,
};
use super::km::YdPvrKm;

/// Signatures of a valid packet, `ui32Sig`: 'HWP2', 'HWPA', 'HWPB', 'HWPC'
/// (`rgx_hwperf_common.h:270`, `:274`, `:278`, `:282`;
/// `HWPERF_PACKET_ISVALID`, `:284-288`).
pub const SIG_V2: u32 = 0x4857_5032;
pub const SIG_V2A: u32 = 0x4857_5041;
pub const SIG_V2B: u32 = 0x4857_5042;
pub const SIG_V2C: u32 = 0x4857_5043;

/// `sizeof(RGX_HWPERF_V2_PACKET_HDR)`: four `u32` and a `u64`
/// (`rgx_hwperf_common.h:295-301`).
pub const HDR_SIZE: usize = 24;
/// `RGX_HWPERF_SIZE_MASK`, the size bits of `ui32Size`
/// (`rgx_hwperf_common.h:308`).
pub const SIZE_MASK: u32 = 0xFFFF;
/// `RGX_HWPERF_MAX_PACKET_SIZE`, 3 KB (`rgx_hwperf_common.h:315`).
pub const MAX_PACKET_SIZE: usize = 0xC00;
/// `PVRSRVTL_PACKET_ALIGNMENT`: every packet size is a multiple of it
/// (`pvrsrv_tlcommon.h:64`; `RGX_HWPERF_MAKE_SIZE_FIXED`,
/// `rgx_hwperf_common.h:326-329`).
pub const PACKET_ALIGN: usize = 8;
/// `RGX_HWPERF_TYPEID_EVENT_MASK`, the event type bits of `eTypeId`
/// (`rgx_hwperf_common.h:356`).
pub const TYPEID_EVENT_MASK: u32 = 0x7FFF;

/// Event types (`RGX_HWPERF_EVENT_TYPE`, `rgx_hwperf_common.h:82-170`): the
/// ones docs/notes/pvr-hwperf.md (b) names.
pub mod ev {
    /// `RGX_HWPERF_FW_BGSTART`, "Background task processing start" (`:89`).
    pub const FW_BGSTART: u32 = 0x01;
    /// `RGX_HWPERF_FW_BGEND` (`:90`).
    pub const FW_BGEND: u32 = 0x02;
    /// `RGX_HWPERF_FW_IRQSTART` (`:91`).
    pub const FW_IRQSTART: u32 = 0x03;
    /// `RGX_HWPERF_FW_IRQEND` (`:93`).
    pub const FW_IRQEND: u32 = 0x04;
    /// `RGX_HWPERF_FW_DBGSTART`, `_DBGEND` (`:94-95`); the FW range ends
    /// at 0x06 (`:97`).
    pub const FW_DBGSTART: u32 = 0x05;
    pub const FW_DBGEND: u32 = 0x06;
    /// `RGX_HWPERF_HW_PMOOM_TAPAUSE` (`:102`).
    pub const HW_PMOOM_TAPAUSE: u32 = 0x07;
    /// `RGX_HWPERF_HW_TAKICK`, `_TAFINISHED` (`:104-105`).
    pub const HW_TAKICK: u32 = 0x08;
    pub const HW_TAFINISHED: u32 = 0x09;
    /// `RGX_HWPERF_HW_3DTQKICK` (`:106`).
    pub const HW_3DTQKICK: u32 = 0x0A;
    /// `RGX_HWPERF_HW_3DKICK`, `_3DFINISHED` (`:107-108`).
    pub const HW_3DKICK: u32 = 0x0B;
    pub const HW_3DFINISHED: u32 = 0x0C;
    /// `RGX_HWPERF_HW_CDMKICK`, `_CDMFINISHED` (`:109-110`).
    pub const HW_CDMKICK: u32 = 0x0D;
    pub const HW_CDMFINISHED: u32 = 0x0E;
    /// `RGX_HWPERF_HW_TLAKICK`, `_TLAFINISHED` (`:111-112`).
    pub const HW_TLAKICK: u32 = 0x0F;
    pub const HW_TLAFINISHED: u32 = 0x10;
    /// `RGX_HWPERF_HW_3DSPMKICK` (`:113`), the partial render's 3D.
    pub const HW_3DSPMKICK: u32 = 0x11;
    /// `RGX_HWPERF_HW_3DTQFINISHED` (`:120`).
    pub const HW_3DTQFINISHED: u32 = 0x17;
    /// `RGX_HWPERF_HW_3DSPMFINISHED` (`:121`).
    pub const HW_3DSPMFINISHED: u32 = 0x18;
    /// `RGX_HWPERF_HW_PMOOM_TARESUME` (`:123-124`); HW range 0 ends here
    /// (`:127`).
    pub const HW_PMOOM_TARESUME: u32 = 0x19;
    /// `RGX_HWPERF_CLKS_CHG`, `_GPU_STATE_CHG` (`:130-131`).
    pub const CLKS_CHG: u32 = 0x1A;
    pub const GPU_STATE_CHG: u32 = 0x1B;
    /// `RGX_HWPERF_PWR_CHG` (`:141`).
    pub const PWR_CHG: u32 = 0x23;
    /// `RGX_HWPERF_HW_TDMKICK`, `_TDMFINISHED` (`:146-147`), HW range 1
    /// (`:144-150`, through `NULLKICK` 0x2A).
    pub const HW_TDMKICK: u32 = 0x28;
    pub const HW_TDMFINISHED: u32 = 0x29;
    pub const HW_NULLKICK: u32 = 0x2A;
    /// `RGX_HWPERF_CSW_START`, `_FINISHED` (`:154-155`).
    pub const CSW_START: u32 = 0x30;
    pub const CSW_FINISHED: u32 = 0x31;
    /// `RGX_HWPERF_UFO`, "FW UFO Check / Update" (`:161`).
    pub const UFO: u32 = 0x38;
    /// `RGX_HWPERF_FWACT` (`:162`).
    pub const FWACT: u32 = 0x39;
    /// `RGX_HWPERF_MAX_TYPE`: types are below it, a filter bit each
    /// (`:170`).
    pub const MAX_TYPE: u32 = 0x40;
}

/// `RGX_HWPERF_EVENT_MASK_VALUE(type)`, the filter bit of an event type
/// (`rogue/rgx_hwperf.h:347`, `1 << type`). For the later firmware init
/// (`RGXFWIF_HWPERFDATA.ui64HWPerfFilter`, docs/notes/pvr-hwperf.md (a)).
pub const fn mask_value(ty: u32) -> u64 {
    1u64 << (ty & (ev::MAX_TYPE - 1))
}

/// Every type from `first` to `last` (the DDK's `MASK_RANGE_IMPL`,
/// `rogue/rgx_hwperf.h:381-386`).
const fn mask_range(first: u32, last: u32) -> u64 {
    ((1u64 << (last - first + 1)) - 1) << first
}

/// The event filter the firmware init writes with `FWINIT_HWPERF`
/// (docs/notes/pvr-hwperf.md (b)): every HW kick and finish
/// (`RGX_HWPERF_EVENT_MASK_HW_KICKFINISH`, the periodic counter event left
/// out), the firmware's background, IRQ and debug tasks (`_FW_SED`), the
/// UFO checks and updates (`_FW_UFO`), the context stores (`_FW_CSW`), the
/// power and clock changes (`_ALL_PWR`) and `FWACT`. Pinned to that
/// expression of the DDK's masks (`fwif::HWPERF_PROBE_FILTER`).
pub const PROBE_FILTER: u64 = ((mask_range(ev::HW_PMOOM_TAPAUSE, ev::HW_PMOOM_TARESUME)
    | mask_range(ev::HW_TDMKICK, ev::HW_NULLKICK))
    & !mask_value(HW_PERIODIC))
    | mask_range(ev::FW_BGSTART, ev::FW_DBGEND)
    | mask_value(ev::UFO)
    | mask_value(ev::CSW_START)
    | mask_value(ev::CSW_FINISHED)
    | mask_value(ev::CLKS_CHG)
    | mask_value(ev::GPU_STATE_CHG)
    | mask_value(ev::PWR_CHG)
    | mask_value(ev::FWACT);

/// `RGX_HWPERF_HW_PERIODIC` (`rgx_hwperf_common.h:114-115`), the 1 ms
/// counter event, left out of [`PROBE_FILTER`].
const HW_PERIODIC: u32 = 0x12;

/// Data masters (`RGX_HWPERF_DM`, `rogue/rgx_hwperf.h:156-163`): the `eDM`
/// of a FW or UFO packet.
pub mod dm {
    pub const GP: u32 = 0;
    pub const DM_2D: u32 = 1;
    pub const TA: u32 = 2;
    pub const DM_3D: u32 = 3;
    pub const CDM: u32 = 4;
    pub const RTU: u32 = 5;
    pub const SHG: u32 = 6;
    pub const TDM: u32 = 7;
}

/// `sizeof(RGX_HWPERF_HW_DATA)` up to its counter stream: 15 `u32`, so
/// `offsetof(..., aui32CountBlksStream)` is 60 (`rgx_hwperf_common.h:636-669`,
/// the assert at `:672`).
pub const HW_DATA_SIZE: usize = 60;
/// `sizeof(RGX_HWPERF_FW_DATA)`: six `u32` (`rgx_hwperf_common.h:610-617`).
pub const FW_DATA_SIZE: usize = 24;
/// `offsetof(RGX_HWPERF_UFO_DATA, aui32StreamData)`: eight `u32`
/// (`rgx_hwperf_common.h:960-983`).
pub const UFO_DATA_SIZE: usize = 32;

/// `RGX_HWPERF_UFO_EV` (`rgx_hwperf_common.h:929-938`), the `eEvType` of a
/// UFO packet.
pub mod ufo_ev {
    pub const UPDATE: u32 = 0;
    pub const CHECK_SUCCESS: u32 = 1;
    pub const PRCHECK_SUCCESS: u32 = 2;
    pub const CHECK_FAIL: u32 = 3;
    pub const PRCHECK_FAIL: u32 = 4;
    pub const FORCE_UPDATE: u32 = 5;
}

/// `RGX_HWPERF_UFO_STREAMSIZE_MASK`/`_SHIFT` and `_STREAMOFFSET_MASK`
/// (`rgx_hwperf_common.h:585-590`): `ui32StreamInfo` is the element count in
/// bits 31..16 and the stream's byte offset in the payload in bits 15..0.
pub const UFO_STREAMSIZE_SHIFT: u32 = 16;
pub const UFO_STREAMOFFSET_MASK: u32 = 0xFFFF;

fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    let w = b.get(off..off.checked_add(4)?)?;
    Some(u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
}

fn u64_at(b: &[u8], off: usize) -> Option<u64> {
    Some(u32_at(b, off)? as u64 | (u32_at(b, off + 4)? as u64) << 32)
}

/// `RGX_HWPERF_V2_PACKET_HDR` (`rgx_hwperf_common.h:295-301`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hdr {
    /// `ui32Sig`, one of the `SIG_*`.
    pub sig: u32,
    /// `ui32Size & SIZE_MASK`: the whole packet, header included.
    pub size: u32,
    /// `eTypeId & TYPEID_EVENT_MASK`, one of [`ev`].
    pub ty: u32,
    /// `eTypeId` as written (thread, stream, OSID bits, `:357-362`).
    pub type_id: u32,
    /// `ui32Ordinal`, the packet's sequence number (a gap is a dropped
    /// packet, docs/notes/pvr-hwperf.md (b)).
    pub ordinal: u32,
    /// `ui64Timestamp`, in timer ticks.
    pub ts: u64,
}

/// The fields of `RGX_HWPERF_HW_DATA` (`rgx_hwperf_common.h:636-669`) that
/// tie a HW event to a kick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HwData {
    /// `ui32DMCyc` (offset 0).
    pub dm_cyc: u32,
    /// `ui32FrameNum` (4).
    pub frame_num: u32,
    /// `ui32DMContext` (12), the firmware render context.
    pub dm_context: u32,
    /// `ui32WorkTarget` (16), the HWRT data for a TA or 3D.
    pub work_target: u32,
    /// `ui32ExtJobRef` (20), `ui32IntJobRef` (24): the client-CCB command
    /// header's job refs (`rogue/rgx_fwif_km.h:1395-1403`); 0 today, our
    /// command headers are zeroed (docs/notes/pvr-hwperf.md (b)).
    pub ext_job_ref: u32,
    pub int_job_ref: u32,
    /// `ui32TimeCorrIndex` (28).
    pub time_corr_index: u32,
}

/// `RGX_HWPERF_FW_DATA` (`rgx_hwperf_common.h:610-617`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FwData {
    /// `eDM` (offset 0), one of [`dm`].
    pub dm: u32,
    /// `ui32TimeCorrIndex` (16).
    pub time_corr_index: u32,
}

/// `RGX_HWPERF_UFO_DATA` (`rgx_hwperf_common.h:960-983`) and its first
/// stream element (`RGX_HWPERF_UFO_DATA_ELEMENT`, `:941-957`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UfoData {
    /// `eEvType` (offset 0), one of [`ufo_ev`].
    pub ev: u32,
    /// `ui32ExtJobRef` (12), `ui32IntJobRef` (16).
    pub ext_job_ref: u32,
    pub int_job_ref: u32,
    /// `ui32DMContext` (20).
    pub dm_context: u32,
    /// `eDM` (28), one of [`dm`].
    pub dm: u32,
    /// Elements in the stream (`ui32StreamInfo` bits 31..16).
    pub count: u32,
    /// The first element's `ui32FWAddr` and value: `ui32Value` for a check,
    /// `ui32NewValue` for an update (0, 0 without an element).
    pub addr: u32,
    pub value: u32,
}

/// What a packet carries after its header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// A HW event (`HWPERF_PACKET_IS_HW_TYPE`, `rgx_hwperf_common.h:176-180`).
    Hw(HwData),
    /// A FW event (`HWPERF_PACKET_IS_FW_TYPE`, `:183-185`).
    Fw(FwData),
    /// [`ev::UFO`].
    Ufo(UfoData),
    /// A type this reader does not decode (CSW, power, FWACT, or one it does
    /// not know): kept by its header only.
    Other,
}

/// One decoded packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Packet<'a> {
    /// Byte offset of the packet in the slice.
    pub off: usize,
    pub hdr: Hdr,
    pub event: Event,
    /// The bytes after the header (`hdr.size - HDR_SIZE`).
    pub payload: &'a [u8],
}

/// Why the reader stopped. Each names the byte offset of the packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// `ui32Sig` is none of the four signatures.
    BadSig { off: usize, sig: u32 },
    /// The size is below the header, above [`MAX_PACKET_SIZE`], not a
    /// multiple of [`PACKET_ALIGN`], or too small for the type's payload.
    BadSize { off: usize, size: u32 },
    /// The packet runs past the end of the slice (a header cut short, or a
    /// size larger than what is left).
    Truncated { off: usize, need: usize, have: usize },
}

fn is_hw(ty: u32) -> bool {
    (ev::HW_PMOOM_TAPAUSE..=ev::HW_PMOOM_TARESUME).contains(&ty) || (ev::HW_TDMKICK..=ev::HW_NULLKICK).contains(&ty)
}

fn is_fw(ty: u32) -> bool {
    (ev::FW_BGSTART..=ev::FW_DBGEND).contains(&ty)
}

fn decode(hdr: &Hdr, off: usize, p: &[u8]) -> Result<Event, Error> {
    let short = |min: usize| -> Result<(), Error> {
        if p.len() < min {
            Err(Error::BadSize { off, size: hdr.size })
        } else {
            Ok(())
        }
    };
    // Every read below is in range after the length checks; `unwrap_or(0)`
    // only spells that out without a panic path.
    let w = |o: usize| u32_at(p, o).unwrap_or(0);
    if is_hw(hdr.ty) {
        short(HW_DATA_SIZE)?;
        return Ok(Event::Hw(HwData {
            dm_cyc: w(0),
            frame_num: w(4),
            dm_context: w(12),
            work_target: w(16),
            ext_job_ref: w(20),
            int_job_ref: w(24),
            time_corr_index: w(28),
        }));
    }
    if is_fw(hdr.ty) {
        short(FW_DATA_SIZE)?;
        return Ok(Event::Fw(FwData { dm: w(0), time_corr_index: w(16) }));
    }
    if hdr.ty == ev::UFO {
        short(UFO_DATA_SIZE)?;
        let info = w(24);
        let count = info >> UFO_STREAMSIZE_SHIFT;
        let soff = (info & UFO_STREAMOFFSET_MASK) as usize;
        let mut u = UfoData {
            ev: w(0),
            ext_job_ref: w(12),
            int_job_ref: w(16),
            dm_context: w(20),
            dm: w(28),
            count,
            addr: 0,
            value: 0,
        };
        if count > 0 {
            // A check success is {addr, value}; a check fail
            // {addr, value, required}; an update {addr, old, new}
            // (`rgx_hwperf_common.h:941-957`).
            let val_at = match u.ev {
                ufo_ev::UPDATE | ufo_ev::FORCE_UPDATE => 8,
                _ => 4,
            };
            let (Some(a), Some(v)) = (u32_at(p, soff), u32_at(p, soff + val_at)) else {
                return Err(Error::BadSize { off, size: hdr.size });
            };
            u.addr = a;
            u.value = v;
        }
        return Ok(Event::Ufo(u));
    }
    Ok(Event::Other)
}

/// Walks the packets of one contiguous stretch of the HWPerf buffer (the
/// `[RIdx, WIdx)` range, or each of the two ranges after a wrap,
/// docs/notes/pvr-hwperf.md (c)). Yields each packet; on a malformed one it
/// yields the [`Error`] once and then stops, since the size of a bad packet
/// cannot be trusted to find the next one.
pub struct Reader<'a> {
    buf: &'a [u8],
    off: usize,
    done: bool,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Reader { buf, off: 0, done: false }
    }

    /// Bytes consumed so far (the next packet's offset).
    pub fn offset(&self) -> usize {
        self.off
    }

    fn next_packet(&mut self) -> Result<Packet<'a>, Error> {
        let off = self.off;
        let rest = &self.buf[off..];
        if rest.len() < HDR_SIZE {
            return Err(Error::Truncated { off, need: HDR_SIZE, have: rest.len() });
        }
        let sig = u32_at(rest, 0).unwrap_or(0);
        if !matches!(sig, SIG_V2 | SIG_V2A | SIG_V2B | SIG_V2C) {
            return Err(Error::BadSig { off, sig });
        }
        let size = u32_at(rest, 4).unwrap_or(0) & SIZE_MASK;
        let n = size as usize;
        if !(HDR_SIZE..=MAX_PACKET_SIZE).contains(&n) || !n.is_multiple_of(PACKET_ALIGN) {
            return Err(Error::BadSize { off, size });
        }
        if rest.len() < n {
            return Err(Error::Truncated { off, need: n, have: rest.len() });
        }
        let type_id = u32_at(rest, 8).unwrap_or(0);
        let hdr = Hdr {
            sig,
            size,
            ty: type_id & TYPEID_EVENT_MASK,
            type_id,
            ordinal: u32_at(rest, 12).unwrap_or(0),
            ts: u64_at(rest, 16).unwrap_or(0),
        };
        let payload = &rest[HDR_SIZE..n];
        let event = decode(&hdr, off, payload)?;
        self.off = off + n;
        Ok(Packet { off, hdr, event, payload })
    }
}

impl<'a> Iterator for Reader<'a> {
    type Item = Result<Packet<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done || self.off >= self.buf.len() {
            return None;
        }
        let r = self.next_packet();
        if r.is_err() {
            self.done = true;
        }
        Some(r)
    }
}

/// The engine a HW kick/finish pair runs on. Each has its own start and end
/// types; a finish closes the oldest open start of its lane (the firmware
/// runs one task a lane at a time).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    /// TA (geometry): `TAKICK` .. `TAFINISHED`.
    Ta,
    /// 3D (fragment): `3DKICK` .. `3DFINISHED`.
    ThreeD,
    /// Partial render 3D: `3DSPMKICK` .. `3DSPMFINISHED`.
    ThreeDSpm,
    /// 3D transfer: `3DTQKICK` .. `3DTQFINISHED`.
    ThreeDTq,
    /// Compute: `CDMKICK` .. `CDMFINISHED`.
    Cdm,
    /// TLA: `TLAKICK` .. `TLAFINISHED`.
    Tla,
    /// Transfer: `TDMKICK` .. `TDMFINISHED`.
    Tdm,
}

const LANES: usize = 7;

impl Lane {
    /// The lane a type starts (`Some((lane, true))`) or finishes
    /// (`Some((lane, false))`).
    pub fn of(ty: u32) -> Option<(Lane, bool)> {
        Some(match ty {
            ev::HW_TAKICK => (Lane::Ta, true),
            ev::HW_TAFINISHED => (Lane::Ta, false),
            ev::HW_3DKICK => (Lane::ThreeD, true),
            ev::HW_3DFINISHED => (Lane::ThreeD, false),
            ev::HW_3DSPMKICK => (Lane::ThreeDSpm, true),
            ev::HW_3DSPMFINISHED => (Lane::ThreeDSpm, false),
            ev::HW_3DTQKICK => (Lane::ThreeDTq, true),
            ev::HW_3DTQFINISHED => (Lane::ThreeDTq, false),
            ev::HW_CDMKICK => (Lane::Cdm, true),
            ev::HW_CDMFINISHED => (Lane::Cdm, false),
            ev::HW_TLAKICK => (Lane::Tla, true),
            ev::HW_TLAFINISHED => (Lane::Tla, false),
            ev::HW_TDMKICK => (Lane::Tdm, true),
            ev::HW_TDMFINISHED => (Lane::Tdm, false),
            _ => return None,
        })
    }

    fn idx(self) -> usize {
        self as usize
    }

    /// Every lane, in [`Summary::gpu_ticks`]'s order.
    pub const ALL: [Lane; LANES] =
        [Lane::Ta, Lane::ThreeD, Lane::ThreeDSpm, Lane::ThreeDTq, Lane::Cdm, Lane::Tla, Lane::Tdm];

    /// The lane's name in the drain's record line ([`drain`]).
    pub fn name(self) -> &'static str {
        match self {
            Lane::Ta => "ta",
            Lane::ThreeD => "3d",
            Lane::ThreeDSpm => "3dspm",
            Lane::ThreeDTq => "3dtq",
            Lane::Cdm => "cdm",
            Lane::Tla => "tla",
            Lane::Tdm => "tdm",
        }
    }
}

/// One task on the GPU: its start and finish stamps and what the firmware
/// last did before it started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Task {
    pub lane: Lane,
    /// The start packet's job refs and HWRT (see [`HwData`]).
    pub ext_job_ref: u32,
    pub int_job_ref: u32,
    pub work_target: u32,
    /// Start and finish stamps, in ticks.
    pub start: u64,
    pub finish: u64,
    /// The stamp of the last `FW_BGSTART` before the start (the firmware's
    /// take-up of the post, docs/notes/pvr-hwperf.md (b)), if any.
    pub bg: Option<u64>,
    /// The stamp of the previous task's finish on the same lane, if any.
    pub prev_finish: Option<u64>,
}

impl Task {
    /// The GPU time of the task: finish less start.
    pub fn gpu_ticks(&self) -> u64 {
        self.finish.saturating_sub(self.start)
    }

    /// The time from the kick to the start: from the firmware's last
    /// background-task start before it (its take-up of the KCCB command),
    /// or `None` without one in the stream.
    pub fn kick_to_start_ticks(&self) -> Option<u64> {
        Some(self.start.saturating_sub(self.bg?))
    }
}

/// Totals over a stream.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// Packets read.
    pub packets: u32,
    /// Packets missing by the ordinals (the sum of every gap).
    pub dropped: u32,
    /// Packets of a type this reader keeps by the header only.
    pub other: u32,
    /// Tasks whose start and finish were both seen.
    pub tasks: u32,
    /// Tasks that did not fit in the output slice (counted in the sums).
    pub tasks_lost: u32,
    /// A finish with no open start on its lane, a start with no finish by the
    /// end, and a start that found [`MAX_OPEN`] already open.
    pub unmatched_finish: u32,
    pub open_at_end: u32,
    pub open_overflow: u32,
    /// GPU ticks summed over the tasks, per [`Lane`] in its order.
    pub gpu_ticks: [u64; LANES],
    /// Kick-to-start ticks summed over the tasks that had a `BGSTART` before
    /// them, and how many those were.
    pub kick_to_start_ticks: u64,
    pub kick_to_start_n: u32,
    /// Per [`Lane`]: the tasks, and the kick-to-start ticks and their count.
    pub lane_tasks: [u32; LANES],
    pub lane_kick_to_start_ticks: [u64; LANES],
    pub lane_kick_to_start_n: [u32; LANES],
    /// Ticks from the first to the last packet's stamp.
    pub span_ticks: u64,
    /// The error that stopped the reader, if one did.
    pub error: Option<Error>,
}

impl Summary {
    pub fn gpu_ticks_of(&self, lane: Lane) -> u64 {
        self.gpu_ticks[lane.idx()]
    }
}

/// Starts a lane can hold before their finishes (the firmware runs one task a
/// lane, so more than one is a stream the reader does not understand).
pub const MAX_OPEN: usize = 4;

#[derive(Clone, Copy)]
struct Open {
    ext: u32,
    int: u32,
    wt: u32,
    ts: u64,
    bg: Option<u64>,
}

/// Read every packet of `buf`, pair each lane's start with its finish, write
/// the tasks in finish order into `out`, and return the totals. Stops at the
/// first malformed packet and names it in [`Summary::error`]; what came before
/// it is summarised.
pub fn summarize(buf: &[u8], out: &mut [Task]) -> (Summary, usize) {
    let mut s = Summary::default();
    let mut open: [[Option<Open>; MAX_OPEN]; LANES] = [[None; MAX_OPEN]; LANES];
    let mut prev_finish: [Option<u64>; LANES] = [None; LANES];
    let mut last_bg: Option<u64> = None;
    let mut last_ord: Option<u32> = None;
    let mut first_ts: Option<u64> = None;
    let mut n = 0usize;

    for r in Reader::new(buf) {
        let p = match r {
            Ok(p) => p,
            Err(e) => {
                s.error = Some(e);
                break;
            }
        };
        s.packets += 1;
        if let Some(o) = last_ord {
            s.dropped += p.hdr.ordinal.wrapping_sub(o).wrapping_sub(1);
        }
        last_ord = Some(p.hdr.ordinal);
        first_ts.get_or_insert(p.hdr.ts);
        s.span_ticks = p.hdr.ts.saturating_sub(first_ts.unwrap_or(p.hdr.ts));

        match p.event {
            Event::Fw(_) if p.hdr.ty == ev::FW_BGSTART => last_bg = Some(p.hdr.ts),
            Event::Hw(h) => {
                let Some((lane, start)) = Lane::of(p.hdr.ty) else {
                    continue;
                };
                let q = &mut open[lane.idx()];
                if start {
                    match q.iter_mut().find(|e| e.is_none()) {
                        Some(slot) => {
                            *slot = Some(Open {
                                ext: h.ext_job_ref,
                                int: h.int_job_ref,
                                wt: h.work_target,
                                ts: p.hdr.ts,
                                bg: last_bg,
                            })
                        }
                        None => s.open_overflow += 1,
                    }
                } else {
                    // The oldest open start of the lane: slots fill from 0, and
                    // closing one shifts the rest down.
                    let Some(o) = q[0] else {
                        s.unmatched_finish += 1;
                        continue;
                    };
                    q.copy_within(1.., 0);
                    q[MAX_OPEN - 1] = None;
                    let t = Task {
                        lane,
                        ext_job_ref: o.ext,
                        int_job_ref: o.int,
                        work_target: o.wt,
                        start: o.ts,
                        finish: p.hdr.ts,
                        bg: o.bg,
                        prev_finish: prev_finish[lane.idx()],
                    };
                    prev_finish[lane.idx()] = Some(p.hdr.ts);
                    s.tasks += 1;
                    s.gpu_ticks[lane.idx()] += t.gpu_ticks();
                    s.lane_tasks[lane.idx()] += 1;
                    if let Some(k) = t.kick_to_start_ticks() {
                        s.kick_to_start_ticks += k;
                        s.kick_to_start_n += 1;
                        s.lane_kick_to_start_ticks[lane.idx()] += k;
                        s.lane_kick_to_start_n[lane.idx()] += 1;
                    }
                    match out.get_mut(n) {
                        Some(slot) => {
                            *slot = t;
                            n += 1;
                        }
                        None => s.tasks_lost += 1,
                    }
                }
            }
            Event::Other => s.other += 1,
            _ => {}
        }
    }
    s.open_at_end = open.iter().flatten().filter(|e| e.is_some()).count() as u32;
    (s, n)
}

/// GPU clocks a timer tick (`rogue/rgx_fwif_km.h:1900-1901`).
pub const CLOCKS_PER_TICK: u64 = 256;

/// Nanoseconds of `ticks` timer ticks at a core clock of `core_hz`
/// (`deltaCR * 256 / GPUfreq`, `rogue/rgx_fwif_km.h:1903-1909`); 0 at a clock
/// of 0.
pub fn ticks_to_ns(ticks: u64, core_hz: u64) -> u64 {
    if core_hz == 0 {
        return 0;
    }
    ((ticks as u128 * CLOCKS_PER_TICK as u128 * 1_000_000_000) / core_hz as u128) as u64
}

// ---- the drain (docs/notes/pvr-hwperf.md (c)) ---------------------------------

/// What [`drain`] read: the summary of the stretch and the firmware's words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drained {
    pub summary: Summary,
    /// Where the stretch began (the caller's `from`) and the firmware's
    /// `ui32HWPerfWIdx`, `ui32HWPerfWrapCount`, `ui32HWPerfSize` and
    /// `ui32HWPerfDropCount` as read.
    pub from: u32,
    pub widx: u32,
    pub wrap: u32,
    pub size: u32,
    pub fw_drops: u32,
}

/// Firmware-heap offset of a firmware address (either data window,
/// `YdPvrKm::heap_fwaddr`: the window's base ORed with the offset).
fn fw_heap_off(fwaddr: u32) -> u32 {
    fwaddr & (FW_HEAP_SIZE - 1)
}

fn add(a: &mut Summary, b: &Summary) {
    a.packets += b.packets;
    a.dropped += b.dropped;
    a.other += b.other;
    a.tasks += b.tasks;
    a.tasks_lost += b.tasks_lost;
    a.unmatched_finish += b.unmatched_finish;
    a.open_at_end += b.open_at_end;
    a.open_overflow += b.open_overflow;
    for i in 0..LANES {
        a.gpu_ticks[i] += b.gpu_ticks[i];
        a.lane_tasks[i] += b.lane_tasks[i];
        a.lane_kick_to_start_ticks[i] += b.lane_kick_to_start_ticks[i];
        a.lane_kick_to_start_n[i] += b.lane_kick_to_start_n[i];
    }
    a.kick_to_start_ticks += b.kick_to_start_ticks;
    a.kick_to_start_n += b.kick_to_start_n;
    a.span_ticks += b.span_ticks;
    a.error = a.error.or(b.error);
}

/// Read the packets the firmware wrote since byte `from` of its HWPerf
/// buffer, summarise them ([`summarize`], tasks into `out`), and hand the
/// space back to the firmware: `ui32HWPerfRIdx` is set to the write index
/// reached ([`store_ridx`]). The buffer and its control are found the way the
/// firmware finds them, through the OS init's `sHwPerfData` and
/// `sHwPerfRuntimeCfg.sHWPerfBuf`, so the context needs no field of its own.
/// `None`, with nothing read or written, when the firmware init did not set
/// HWPerf up (`FWINIT_HWPERF` clear: a size of 0).
///
/// The caller keeps `widx` as the next `from` (it equals the `RIdx` written),
/// so each call covers one stretch (a scene) and the firmware only drops
/// packets (`fw_drops`) when one stretch alone fills the buffer. A stretch
/// past a wrap mark is summarised as its two ranges, added: a task cut by the
/// wrap counts as open in one and unmatched in the other, and the ordinal gap
/// across it is not counted.
pub fn drain(k: &YdPvrKm, from: u32, out: &mut [Task]) -> Option<Drained> {
    let osi = k.heap_cpu(k.fw_config_off + FW_CONFIG_HEAP_ALLOC_GRANULARITY);
    // SAFETY: the OS init's slot of the config region (built by fwinit).
    let (hp_fa, rc_fa) = unsafe { (rd32(osi, OSINIT_HWPERF_DATA), rd32(osi, OSINIT_HWPERF_RUNTIME_CFG)) };
    if hp_fa == 0 || rc_fa == 0 {
        return None;
    }
    let hp = k.heap_cpu(fw_heap_off(hp_fa));
    // The firmware writes the indices and the drop count in memory.
    k.invd(hp, HWPERFDATA_SIZE);
    let rc = k.heap_cpu(fw_heap_off(rc_fa));
    // SAFETY: the HWPerf data (40 bytes) and runtime configuration fwinit
    // allocated.
    let (size, widx, wrap, fw_drops, buf_fa) = unsafe {
        (
            rd32(hp, HWPERFDATA_SIZE_FIELD),
            rd32(hp, HWPERFDATA_WIDX),
            rd32(hp, HWPERFDATA_WRAP_COUNT),
            rd32(hp, HWPERFDATA_DROP_COUNT),
            rd32(rc, RUNTIME_HWPERFCFG_BUF),
        )
    };
    if size == 0 || buf_fa == 0 {
        return None;
    }
    let buf = k.heap_cpu(fw_heap_off(buf_fa));
    let (from_c, widx_c) = (from.min(size), widx.min(size));
    let wrap_end = if wrap > from_c && wrap <= size { wrap } else { size };
    let ranges = if widx_c >= from_c { [(from_c, widx_c), (0, 0)] } else { [(from_c, wrap_end), (0, widx_c)] };
    let mut summary = Summary::default();
    let mut n = 0usize;
    for (a, b) in ranges {
        if b <= a {
            continue;
        }
        let p = buf.wrapping_add(a as usize);
        let len = (b - a) as usize;
        k.invd(p, len);
        // SAFETY: inside the buffer of `size` bytes fwinit allocated.
        let bytes = unsafe { core::slice::from_raw_parts(p as *const u8, len) };
        let (s, m) = summarize(bytes, out.get_mut(n..).unwrap_or(&mut []));
        n += m;
        add(&mut summary, &s);
    }
    // A write index past the buffer is not the firmware's: leave RIdx.
    if widx <= size {
        store_ridx(k, hp, widx);
    }
    Some(Drained { summary, from, widx, wrap, size, fw_drops })
}

/// Write `ui32HWPerfRIdx` = `ridx` in the HWPerf data at `hp`, without
/// touching the firmware's `WIdx` and `WrapCount` beside it.
///
/// The three indices share one 12-byte struct and so one cache line
/// (`rogue/rgx_fwif_km.h:1670-1674`). Through a cached view, our write would
/// dirty the line and its write-back (a `flush`, or an eviction) would put
/// our stale copy of `WIdx` back over the firmware's (the FWCCB's lesson,
/// `ccb.rs:329-333`). The view is not cached: `pvr_fw_boot` maps the whole
/// carveout Normal non-cacheable (`K_MEM_ARM_NORMAL_NC`, the attribute
/// `dma_alloc_coherent` gives on arm64, `driver/src/pvr_fw_boot.c`), and
/// the HWPerf knob does not build without that attribute (the `#error`
/// beside `pvr_fw_hwperf_drain`). So the route is the one every other
/// firmware-shared word of ours takes (`wr32`, as the FWCCB's
/// `read_offset`), less the clean: one 32-bit volatile store, which writes
/// those four bytes and no others to memory, and no `flush` of the line
/// after it. (Through a cached view no store from the CPU is safe here: an
/// eviction writes the whole line back. Hence the build guard rather than a
/// cache operation.)
///
/// The barrier before it orders the reads of the packets before the
/// firmware may overwrite them; the one after it makes the store visible
/// before the caller goes on (the firmware polls `RIdx` when it next writes a
/// packet; it needs no doorbell).
fn store_ridx(k: &YdPvrKm, hp: *mut u8, ridx: u32) {
    k.fence();
    // SAFETY: `hp` is the 40-byte HWPerf data fwinit allocated (the caller's
    // read of it); offset 0 is `ui32HWPerfRIdx`, a word only the host writes.
    unsafe { wr32(hp, HWPERFDATA_RIDX, ridx) };
    k.fence();
}

/// [`drain`] and its record lines in the driver's log: one totals line, then
/// one line per lane (TA and 3D always, the others when they ran), each
/// field `name=value`, ticks raw and in ns at `core_hz` ([`ticks_to_ns`]).
/// Returns the write index reached (the next `from`), or -ENODEV.
pub fn drain_log(k: &YdPvrKm, from: u32, core_hz: u32) -> i32 {
    let Some(d) = drain(k, from, &mut []) else {
        crate::km_log!(k, Wrn, "pvr-hwperf: off (no buffer; CONFIG_YAKOGL_DRV_GPU_PVR_HWPERF)");
        return -crate::errno::ENODEV;
    };
    let s = &d.summary;
    let hz = u64::from(core_hz);
    crate::km_log!(
        k,
        Inf,
        "pvr-hwperf: from={} widx={} wrap={} size={} fw_drops={} packets={} dropped={} other={} tasks={} \
         unmatched={} open={} overflow={} span_ticks={} span_ns={} err={:?}",
        d.from,
        d.widx,
        d.wrap,
        d.size,
        d.fw_drops,
        s.packets,
        s.dropped,
        s.other,
        s.tasks,
        s.unmatched_finish,
        s.open_at_end,
        s.open_overflow,
        s.span_ticks,
        ticks_to_ns(s.span_ticks, hz),
        s.error
    );
    for lane in Lane::ALL {
        let i = lane.idx();
        if s.lane_tasks[i] == 0 && !matches!(lane, Lane::Ta | Lane::ThreeD) {
            continue;
        }
        crate::km_log!(
            k,
            Inf,
            "pvr-hwperf lane={} tasks={} gpu_ticks={} gpu_ns={} k2s_n={} k2s_ticks={} k2s_ns={}",
            lane.name(),
            s.lane_tasks[i],
            s.gpu_ticks[i],
            ticks_to_ns(s.gpu_ticks[i], hz),
            s.lane_kick_to_start_n[i],
            s.lane_kick_to_start_ticks[i],
            ticks_to_ns(s.lane_kick_to_start_ticks[i], hz)
        );
    }
    d.widx.min(i32::MAX as u32) as i32
}

/// # Safety
/// `k` is the C driver's `struct yd_pvr_km` (see [`super::km::km`]), the
/// firmware structures built.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_hwperf_drain(k: *mut YdPvrKm, from: u32, core_clock_hz: u32) -> i32 {
    // SAFETY: the caller's contract.
    drain_log(unsafe { super::km::km(k) }, from, core_clock_hz)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Appends packets to a buffer laid out as the firmware would.
    struct Buf(Vec<u8>, u32);

    impl Buf {
        fn new() -> Self {
            Buf(Vec::new(), 1)
        }

        fn raw(&mut self, sig: u32, size: u32, type_id: u32, ordinal: u32, ts: u64, payload: &[u8]) {
            for w in [sig, size, type_id, ordinal] {
                self.0.extend_from_slice(&w.to_le_bytes());
            }
            self.0.extend_from_slice(&ts.to_le_bytes());
            self.0.extend_from_slice(payload);
        }

        fn pkt(&mut self, ty: u32, ts: u64, payload: &[u32]) {
            let mut p: Vec<u8> = payload.iter().flat_map(|w| w.to_le_bytes()).collect();
            while !p.len().is_multiple_of(PACKET_ALIGN) {
                p.push(0);
            }
            let ord = self.1;
            self.1 += 1;
            // Stream and thread bits above the event type, as the firmware
            // sets them (`rgx_hwperf_common.h:357-362`): the reader masks them.
            self.raw(SIG_V2B, (HDR_SIZE + p.len()) as u32, ty | 0x0001_8000, ord, ts, &p);
        }

        fn hw(&mut self, ty: u32, ts: u64, ext: u32, wt: u32) {
            let mut d = [0u32; HW_DATA_SIZE / 4];
            d[1] = 7; // ui32FrameNum
            d[3] = 0x1000; // ui32DMContext
            d[4] = wt;
            d[5] = ext;
            d[6] = ext + 100;
            self.pkt(ty, ts, &d);
        }

        fn fw(&mut self, ty: u32, ts: u64, dm: u32) {
            self.pkt(ty, ts, &[dm, 0, 0, 0, 3, 0]);
        }

        fn ufo(&mut self, ts: u64, ufo_ev: u32, dm: u32, tuple: &[u32]) {
            let n = if ufo_ev == ufo_ev::CHECK_SUCCESS { 2 } else { 3 };
            let count = (tuple.len() / n) as u32;
            let mut d = vec![ufo_ev, 0, 0, 0, 0, 0x1000, count << 16 | UFO_DATA_SIZE as u32, dm];
            d.extend_from_slice(tuple);
            self.pkt(ev::UFO, ts, &d);
        }
    }

    /// One mip kick: the firmware takes up the post, TA runs, the geometry
    /// fence is written, 3D runs, the fragment fence is written.
    fn kick(b: &mut Buf, t0: u64, seq: u32) {
        b.fw(ev::FW_BGSTART, t0, dm::GP);
        b.fw(ev::FW_BGEND, t0 + 5, dm::GP);
        b.hw(ev::HW_TAKICK, t0 + 30, seq, 0xA000);
        b.hw(ev::HW_TAFINISHED, t0 + 80, seq, 0xA000);
        b.ufo(t0 + 82, ufo_ev::UPDATE, dm::TA, &[0x7000, seq - 1, seq]);
        b.ufo(t0 + 84, ufo_ev::CHECK_SUCCESS, dm::DM_3D, &[0x7000, seq]);
        b.hw(ev::HW_3DKICK, t0 + 90, seq, 0xA000);
        b.hw(ev::HW_3DFINISHED, t0 + 290, seq, 0xA000);
        b.ufo(t0 + 292, ufo_ev::UPDATE, dm::DM_3D, &[0x7040, seq - 1, seq]);
    }

    #[test]
    fn well_formed_stream() {
        let mut b = Buf::new();
        kick(&mut b, 1000, 1);
        kick(&mut b, 5000, 2);
        b.pkt(ev::PWR_CHG, 6000, &[1, 2]);

        let pkts: Vec<_> = Reader::new(&b.0).collect::<Result<_, _>>().unwrap();
        assert_eq!(pkts.len(), 19);
        let ta = &pkts[2];
        assert_eq!(ta.off, 2 * (HDR_SIZE + FW_DATA_SIZE));
        assert_eq!(ta.hdr.ty, ev::HW_TAKICK);
        assert_eq!(ta.hdr.type_id, ev::HW_TAKICK | 0x0001_8000);
        assert_eq!(ta.hdr.ordinal, 3);
        assert_eq!(ta.hdr.ts, 1030);
        assert_eq!(ta.hdr.size as usize, HDR_SIZE + HW_DATA_SIZE + 4);
        assert_eq!(
            ta.event,
            Event::Hw(HwData {
                dm_cyc: 0,
                frame_num: 7,
                dm_context: 0x1000,
                work_target: 0xA000,
                ext_job_ref: 1,
                int_job_ref: 101,
                time_corr_index: 0
            })
        );
        assert_eq!(pkts[0].event, Event::Fw(FwData { dm: dm::GP, time_corr_index: 3 }));
        let Event::Ufo(u) = pkts[4].event else { panic!("{:?}", pkts[4]) };
        assert_eq!((u.ev, u.dm, u.count, u.addr, u.value), (ufo_ev::UPDATE, dm::TA, 1, 0x7000, 1));
        let Event::Ufo(u) = pkts[5].event else { panic!() };
        assert_eq!((u.ev, u.dm, u.addr, u.value), (ufo_ev::CHECK_SUCCESS, dm::DM_3D, 0x7000, 1));
        assert_eq!(pkts[18].event, Event::Other);

        let mut tasks = [Task {
            lane: Lane::Ta,
            ext_job_ref: 0,
            int_job_ref: 0,
            work_target: 0,
            start: 0,
            finish: 0,
            bg: None,
            prev_finish: None,
        }; 8];
        let (s, n) = summarize(&b.0, &mut tasks);
        assert_eq!(n, 4);
        assert_eq!(s.packets, 19);
        assert_eq!(s.dropped, 0);
        assert_eq!(s.other, 1);
        assert_eq!(s.tasks, 4);
        assert_eq!((s.unmatched_finish, s.open_at_end, s.open_overflow, s.tasks_lost), (0, 0, 0, 0));
        assert_eq!(s.error, None);
        assert_eq!(s.span_ticks, 5000);
        assert_eq!(s.gpu_ticks_of(Lane::Ta), 2 * 50);
        assert_eq!(s.gpu_ticks_of(Lane::ThreeD), 2 * 200);
        // Each TA starts 30 ticks after the BGSTART before it; each 3D 90.
        assert_eq!((s.kick_to_start_ticks, s.kick_to_start_n), (2 * 30 + 2 * 90, 4));

        assert_eq!(tasks[0].lane, Lane::Ta);
        assert_eq!((tasks[0].start, tasks[0].finish, tasks[0].bg), (1030, 1080, Some(1000)));
        assert_eq!(tasks[0].prev_finish, None);
        assert_eq!(tasks[1].lane, Lane::ThreeD);
        assert_eq!(tasks[1].gpu_ticks(), 200);
        assert_eq!(tasks[1].kick_to_start_ticks(), Some(90));
        assert_eq!((tasks[2].lane, tasks[2].ext_job_ref, tasks[2].int_job_ref), (Lane::Ta, 2, 102));
        assert_eq!(tasks[2].work_target, 0xA000);
        assert_eq!(tasks[2].prev_finish, Some(1080));
        assert_eq!(tasks[3].prev_finish, Some(1290));

        // A slice too small keeps the totals and counts what it lost.
        let (s2, n2) = summarize(&b.0, &mut tasks[..1]);
        assert_eq!((n2, s2.tasks, s2.tasks_lost), (1, 4, 3));
        assert_eq!(s2.gpu_ticks, s.gpu_ticks);
    }

    #[test]
    fn ordinal_gap_and_unpaired() {
        let mut b = Buf::new();
        b.hw(ev::HW_3DFINISHED, 10, 0, 0); // a finish before any start
        b.1 += 2; // two packets the firmware could not place
        b.hw(ev::HW_TAKICK, 20, 0, 0); // never finished
        let (s, n) = summarize(&b.0, &mut []);
        assert_eq!((s.packets, s.dropped, n), (2, 2, 0));
        assert_eq!((s.unmatched_finish, s.open_at_end), (1, 1));
        assert_eq!(s.kick_to_start_n, 0);
    }

    #[test]
    fn truncated_packet() {
        let mut b = Buf::new();
        kick(&mut b, 1000, 1);
        let whole = b.0.len();
        b.hw(ev::HW_TAKICK, 2000, 2, 0);
        let cut = whole + HDR_SIZE + 10;
        let it: Vec<_> = Reader::new(&b.0[..cut]).collect();
        assert_eq!(it.len(), 10);
        assert!(it[..9].iter().all(|r| r.is_ok()));
        assert_eq!(it[9], Err(Error::Truncated { off: whole, need: HDR_SIZE + HW_DATA_SIZE + 4, have: HDR_SIZE + 10 }));

        // A header cut short.
        let it: Vec<_> = Reader::new(&b.0[..whole + 12]).collect();
        assert_eq!(it.last(), Some(&Err(Error::Truncated { off: whole, need: HDR_SIZE, have: 12 })));

        // The summary keeps what came before the cut.
        let (s, _) = summarize(&b.0[..cut], &mut []);
        assert_eq!((s.packets, s.tasks), (9, 2));
        assert!(matches!(s.error, Some(Error::Truncated { off, .. }) if off == whole));

        // A HW packet whose size leaves no room for RGX_HWPERF_HW_DATA.
        let mut c = Buf::new();
        c.pkt(ev::HW_3DKICK, 5, &[0, 0]);
        assert_eq!(Reader::new(&c.0).next(), Some(Err(Error::BadSize { off: 0, size: (HDR_SIZE + 8) as u32 })));
    }

    #[test]
    fn unknown_type() {
        let mut b = Buf::new();
        // A type no header defines (0x3A, below RGX_HWPERF_LAST_TYPE), and
        // one past the known range: kept by header, skipped by size.
        b.pkt(0x3A, 100, &[1, 2, 3, 4, 5]);
        b.pkt(0x7F00, 110, &[]);
        b.hw(ev::HW_TAKICK, 120, 9, 0);
        b.hw(ev::HW_TAFINISHED, 170, 9, 0);
        let pkts: Vec<_> = Reader::new(&b.0).collect::<Result<_, _>>().unwrap();
        assert_eq!(pkts.len(), 4);
        assert_eq!((pkts[0].hdr.ty, pkts[0].event, pkts[0].payload.len()), (0x3A, Event::Other, 24));
        assert_eq!((pkts[1].hdr.ty, pkts[1].event, pkts[1].payload.len()), (0x7F00, Event::Other, 0));
        let (s, _) = summarize(&b.0, &mut []);
        assert_eq!((s.packets, s.other, s.tasks, s.gpu_ticks_of(Lane::Ta)), (4, 2, 1, 50));
        assert_eq!(s.kick_to_start_n, 0);
    }

    #[test]
    fn bad_signature_and_size() {
        let mut b = Buf::new();
        b.fw(ev::FW_BGSTART, 1, dm::GP);
        let at = b.0.len();
        b.raw(0x1234_5678, 32, ev::FW_BGEND, 2, 2, &[0; 8]);
        b.fw(ev::FW_BGEND, 3, dm::GP);
        let it: Vec<_> = Reader::new(&b.0).collect();
        assert_eq!(it.len(), 2, "the reader stops at a bad packet");
        assert_eq!(it[1], Err(Error::BadSig { off: at, sig: 0x1234_5678 }));

        for size in [8u32, 28, MAX_PACKET_SIZE as u32 + 8] {
            let mut c = Buf::new();
            c.raw(SIG_V2, size, ev::FW_BGEND, 1, 0, &[0; 8]);
            assert_eq!(Reader::new(&c.0).next(), Some(Err(Error::BadSize { off: 0, size })));
        }
        // The high bits of ui32Size are not the size.
        let mut c = Buf::new();
        c.raw(SIG_V2C, 0xABCD_0000 | 24, ev::PWR_CHG, 1, 0, &[]);
        assert_eq!(Reader::new(&c.0).next().unwrap().unwrap().hdr.size, 24);
    }

    // ---- the drain and its read index over a fake firmware heap ----------------

    use super::super::fwif::{FW_CONFIG_HEAP_SIZE, RISCV_SHARED_UNCACHED_DATA0_BASE};
    use super::super::km::YdPvrKmOps;
    use core::cell::{Cell, RefCell};
    use core::ffi::c_void;
    use std::boxed::Box;
    use std::vec::Vec;

    /// What the fake's hooks saw: every `flush` range and the fences.
    #[derive(Default)]
    struct Hooks {
        flushes: RefCell<Vec<(usize, usize)>>,
        fences: Cell<u32>,
    }

    unsafe extern "C" fn rec_flush(ctx: *mut c_void, a: *const c_void, n: usize) {
        // SAFETY: ctx is the rig's `Hooks`.
        unsafe { &*(ctx as *const Hooks) }.flushes.borrow_mut().push((a as usize, n));
    }
    unsafe extern "C" fn no_range(_: *mut c_void, _: *const c_void, _: usize) {}
    unsafe extern "C" fn rec_fence(ctx: *mut c_void) {
        // SAFETY: as above.
        let h = unsafe { &*(ctx as *const Hooks) };
        h.fences.set(h.fences.get() + 1);
    }
    unsafe extern "C" fn no_wait(_: *mut c_void, _: u32) {}
    unsafe extern "C" fn no_log(_: *mut c_void, _: u32, _: *const u8, _: usize) {}

    /// Heap offsets of the fake's objects in the main region.
    const HP: u32 = 0x100;
    const RC: u32 = 0x200;
    const BUF: u32 = 0x1000;
    const MAIN: u32 = 0x1_0000;

    /// A firmware heap with the OS init, the HWPerf data, the runtime
    /// configuration and a buffer of `size` bytes where fwinit puts them,
    /// and a producer that writes packets the way the firmware does
    /// (docs/notes/pvr-hwperf.md (c)): at `WIdx`, wrapping to 0 with
    /// `WrapCount` at the end of the data when a packet does not fit, never
    /// up to `RIdx`, counting what it cannot place in `DropCount`.
    struct Rig {
        mem: Vec<u64>,
        k: Box<YdPvrKm>,
        hooks: Box<Hooks>,
        _ops: Box<YdPvrKmOps>,
        size: u32,
        wraps: u32,
    }

    impl Rig {
        /// `size` 0: HWPerf off (fwinit leaves the OS init's words 0).
        fn new(size: u32) -> Rig {
            let total = (MAIN + FW_CONFIG_HEAP_ALLOC_GRANULARITY + 0x100) as usize;
            let mut mem = std::vec![0u64; total / 8];
            let hooks: Box<Hooks> = Box::default();
            let ops = Box::new(YdPvrKmOps {
                ctx: &*hooks as *const Hooks as *mut c_void,
                flush: rec_flush,
                invd: no_range,
                fence: rec_fence,
                busy_wait_us: no_wait,
                log: no_log,
            });
            // SAFETY: all-zero is the documented state before the first call.
            let mut k: Box<YdPvrKm> = Box::new(unsafe { core::mem::zeroed() });
            k.cpu = mem.as_mut_ptr() as *mut u8;
            k.size = total as u64;
            k.ops = &*ops;
            k.fw_main_size = MAIN;
            k.fw_config_off = FW_HEAP_SIZE - FW_CONFIG_HEAP_SIZE;
            let mut r = Rig { mem, k, hooks, _ops: ops, size, wraps: 0 };
            if size != 0 {
                assert!(BUF + size <= MAIN);
                let fa = |off: u32| RISCV_SHARED_UNCACHED_DATA0_BASE | off;
                let osi = MAIN + FW_CONFIG_HEAP_ALLOC_GRANULARITY;
                r.put(osi + OSINIT_HWPERF_DATA as u32, fa(HP));
                r.put(osi + OSINIT_HWPERF_RUNTIME_CFG as u32, fa(RC));
                r.put(HP + HWPERFDATA_SIZE_FIELD as u32, size);
                r.put(RC + RUNTIME_HWPERFCFG_BUF as u32, fa(BUF));
            }
            r
        }

        fn bytes(&mut self) -> &mut [u8] {
            // SAFETY: the Vec's own bytes.
            unsafe { core::slice::from_raw_parts_mut(self.mem.as_mut_ptr() as *mut u8, self.mem.len() * 8) }
        }

        fn put(&mut self, carveout_off: u32, v: u32) {
            let o = carveout_off as usize;
            self.bytes()[o..o + 4].copy_from_slice(&v.to_le_bytes());
        }

        fn get(&self, carveout_off: u32) -> u32 {
            // SAFETY: inside the carveout.
            unsafe { rd32(self.mem.as_ptr() as *const u8, carveout_off as usize) }
        }

        fn ridx(&self) -> u32 {
            self.get(HP + HWPERFDATA_RIDX as u32)
        }

        fn widx(&self) -> u32 {
            self.get(HP + HWPERFDATA_WIDX as u32)
        }

        fn drops(&self) -> u32 {
            self.get(HP + HWPERFDATA_DROP_COUNT as u32)
        }

        /// The firmware writes one packet; false when it had to drop it.
        fn fw_put(&mut self, pkt: &[u8]) -> bool {
            let (r, mut w, n) = (self.ridx(), self.widx(), pkt.len() as u32);
            let fits = if w >= r {
                if w + n < self.size {
                    true
                } else if n < r {
                    // The data ends at w; go on at 0.
                    self.put(HP + HWPERFDATA_WRAP_COUNT as u32, w);
                    self.wraps += 1;
                    w = 0;
                    true
                } else {
                    false
                }
            } else {
                w + n < r
            };
            if !fits {
                let d = self.drops();
                self.put(HP + HWPERFDATA_DROP_COUNT as u32, d + 1);
                return false;
            }
            let at = (BUF + w) as usize;
            self.bytes()[at..at + pkt.len()].copy_from_slice(pkt);
            self.put(HP + HWPERFDATA_WIDX as u32, w + n);
            true
        }

        /// `kicks` TA tasks, each a kick and a finish packet, from job ref
        /// `seq` on.
        fn fw_kicks(&mut self, b: &mut Buf, seq: u32, kicks: u32) {
            for j in seq..seq + kicks {
                for (ty, dt) in [(ev::HW_TAKICK, 0), (ev::HW_TAFINISHED, 50)] {
                    b.0.clear();
                    b.hw(ty, 1000 * u64::from(j) + dt, j, 0xA000);
                    let p = b.0.clone();
                    self.fw_put(&p);
                }
            }
        }
    }

    const EMPTY_TASK: Task = Task {
        lane: Lane::Ta,
        ext_job_ref: 0,
        int_job_ref: 0,
        work_target: 0,
        start: 0,
        finish: 0,
        bg: None,
        prev_finish: None,
    };

    /// (1) After a drain the read index is the write index reached, written
    /// with one store between two fences and no clean of the line that holds
    /// the firmware's `WIdx`; the firmware's words are left as they were.
    #[test]
    fn drain_writes_read_index_back() {
        let mut r = Rig::new(4096);
        let mut b = Buf::new();
        r.fw_kicks(&mut b, 1, 3);
        let w = r.widx();
        assert_eq!((w, r.ridx()), (6 * 88, 0));

        let d = drain(&r.k, 0, &mut []).unwrap();
        assert_eq!((d.widx, d.summary.packets, d.summary.tasks), (w, 6, 3));
        assert_eq!(r.ridx(), w);
        assert_eq!((r.widx(), r.drops()), (w, 0), "WIdx and DropCount untouched");
        assert!(r.hooks.flushes.borrow().is_empty(), "no flush: {:?}", r.hooks.flushes.borrow());
        assert_eq!(r.hooks.fences.get(), 2);

        // The next stretch, from the index the caller kept.
        r.fw_kicks(&mut b, 4, 2);
        let d = drain(&r.k, d.widx, &mut []).unwrap();
        assert_eq!((d.from, d.summary.packets, d.summary.tasks), (w, 4, 2));
        assert_eq!(r.ridx(), w + 4 * 88);

        // A write index past the buffer is not believed: RIdx stays.
        r.put(HP + HWPERFDATA_WIDX as u32, 8192);
        let before = r.ridx();
        let _ = drain(&r.k, before, &mut []);
        assert_eq!(r.ridx(), before);
    }

    /// (2) A run that writes the buffer over many times: with the read index
    /// handed back after each drain, the firmware wraps instead of dropping,
    /// and every task comes out once, in order. The buffer is not a multiple
    /// of a task (two 88-byte packets), so the wrap mark is below the end.
    #[test]
    fn wrapped_buffer_drains_in_order_without_loss() {
        let size = 176 * 23 + 40;
        let mut r = Rig::new(size);
        let mut b = Buf::new();
        let mut out = std::vec![EMPTY_TASK; 64];
        let (mut seq, mut from, mut seen, mut packets) = (1u32, 0u32, Vec::new(), 0u32);
        for burst in 0..60u32 {
            // 1 to 20 tasks a stretch: never a whole buffer between drains.
            let kicks = 1 + burst * 7 % 20;
            r.fw_kicks(&mut b, seq, kicks);
            seq += kicks;
            let d = drain(&r.k, from, &mut out).unwrap();
            let s = d.summary;
            assert_eq!((s.error, s.dropped, s.unmatched_finish, s.open_at_end), (None, 0, 0, 0), "burst {burst}");
            assert_eq!(d.fw_drops, 0, "burst {burst}");
            assert_eq!(r.ridx(), d.widx);
            seen.extend(out[..s.tasks as usize].iter().map(|t| t.ext_job_ref));
            packets += s.packets;
            from = d.widx;
        }
        assert!(r.wraps >= 10, "the run wrapped {} times", r.wraps);
        assert_ne!(r.get(HP + HWPERFDATA_WRAP_COUNT as u32), size, "a wrap mark below the end");
        assert_eq!(seen, (1..seq).collect::<Vec<_>>());
        assert_eq!(packets, 2 * (seq - 1));
        assert_eq!(r.drops(), 0);

        // Without the read index (the drain before this change), the same
        // producer fills the buffer once and drops the rest.
        let mut q = Rig::new(size);
        q.fw_kicks(&mut b, 1, seq - 1);
        assert!(q.drops() > 0 && q.wraps == 0);
    }

    /// (3) With HWPerf off the drain reads nothing and writes nothing: the
    /// heap is byte for byte as before, and no hook ran.
    #[test]
    fn hwperf_off_writes_nothing() {
        let mut r = Rig::new(0);
        // Words where an enabled heap would have its indices, to see they stay.
        r.put(HP + HWPERFDATA_RIDX as u32, 0x5A5A_5A5A);
        r.put(HP + HWPERFDATA_WIDX as u32, 0x40);
        let before = r.mem.clone();
        assert!(drain(&r.k, 0, &mut []).is_none());
        assert!(r.mem == before);
        assert!(r.hooks.flushes.borrow().is_empty());
        assert_eq!(r.hooks.fences.get(), 0);

        // The size word 0 with the pointers set (the init refused the
        // buffer): also nothing.
        let mut r = Rig::new(4096);
        r.put(HP + HWPERFDATA_SIZE_FIELD as u32, 0);
        r.put(HP + HWPERFDATA_WIDX as u32, 0x40);
        let before = r.mem.clone();
        assert!(drain(&r.k, 0, &mut []).is_none());
        assert!(r.mem == before);
        assert_eq!(r.hooks.fences.get(), 0);
    }

    #[test]
    fn masks_and_clock() {
        assert_eq!(mask_value(ev::HW_TAKICK), 1 << 8);
        assert_eq!(mask_value(ev::UFO), 1 << 0x38);
        // 800 MHz: one tick is 256 / 800e6 s = 320 ns.
        assert_eq!(ticks_to_ns(1, 800_000_000), 320);
        assert_eq!(ticks_to_ns(2812, 800_000_000), 899_840);
        assert_eq!(ticks_to_ns(5, 0), 0);
    }
}
