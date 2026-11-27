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
//! This module is the host-testable half only: a pure decoder over a byte
//! slice ([`Reader`]) and a per-kick summary ([`summarize`]). Nothing here
//! turns HWPerf on in the firmware or reads its buffer; that is the firmware
//! init and the drain of the note's section (d), a later item. There is no
//! allocator: the reader borrows the slice and the summary writes into a slice
//! the caller gives.
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
                    if let Some(k) = t.kick_to_start_ticks() {
                        s.kick_to_start_ticks += k;
                        s.kick_to_start_n += 1;
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
