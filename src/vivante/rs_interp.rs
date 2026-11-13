// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A host-test model of the GC7000 resolve engine (RS): it runs the RS jobs
//! of a captured stream over a model of GPU memory
//! (docs/notes/gc-tiled-wire.md, "RS interpreter"). **Test helper only**:
//! compiled with the `host-test` feature, in no product image.
//!
//! It takes the `(register, value)` state writes a `GcCmd` stream decodes to
//! (as `tiled_copy.rs`'s tests collect them) and, on each `GC_RS_KICKER`
//! write, performs the job the RS registers then describe. It works through
//! [`layout::offset`], so it shows that a stream's strides and tiling bits
//! agree with `layout.rs`. It does not show what the hardware does.
//!
//! # The subset modelled (Mesa 26.2.1 `etna_compile_rs_state`)
//!
//! - **Layouts.** A side is LINEAR when its `RS_CONFIG` `SOURCE_TILED` /
//!   `DEST_TILED` bit is clear, TILED (4x4) when it is set and the stride has
//!   no `TILING` bit, SUPER_TILED when it is set and the stride has `TILING`
//!   (with `SUPER_TILED_NEW` on this CACHE128B256BPERLINE part). That is
//!   `etna_compile_rs_state` (`etnaviv_rs.c:80-83`, `:95-135`):
//!   `COND(src_tiled, *_TILED)` in `RS_CONFIG` (`ETNA_LAYOUT_BIT_TILE`),
//!   `COND(src_super, *_STRIDE_TILING)` and, on CACHE128B256BPERLINE,
//!   `COND(src_super, *_SUPER_TILED_NEW)` in the stride
//!   (`ETNA_LAYOUT_BIT_SUPER`); the destination alike. It is also what the
//!   words of
//!   Mesa's own streams: the TS fill's `RS_CONFIG` 0x00004606 with
//!   `RS_DEST_STRIDE` 0x100 (a TILED side, no stride bit), the resolve's
//!   `RS_SOURCE_STRIDE` 0xa8000400 (super-tiled: `TILING`, `TS_MODE`,
//!   `SUPER_TILED_NEW`), `tools/vivante/README.md` (the ts-golden table).
//!   A stride with a tiling bit on a side whose `RS_CONFIG` bit is clear, or
//!   `SUPER_TILED_NEW` without `TILING`, is no Mesa word: the job panics.
//! - **Strides.** A tiled side's stride is written times 4 (one row of 4x4
//!   tiles, `source_stride_shift` 2); the interpreter divides it back to the
//!   padded linear row that [`layout::offset`] takes.
//! - **Copy.** Source and destination formats of the same size; the pixels
//!   are copied bit for bit, with `SWAP_RB` exchanging bytes 0 and 2 of a
//!   32-bit pixel. Formats: A8R8G8B8 and X8R8G8B8 (4 bytes), A4R4G4B4 and
//!   R5G6B5 (2 bytes; no `SWAP_RB`).
//! - **Fill.** `RS_CLEAR_CONTROL` mode != 0: the window of the destination is
//!   written with the 16-byte pattern of `RS_FILL_VALUE[0..4]` (little
//!   endian), byte `p` of the pattern only where clear bit `p` is set. Byte
//!   `k` of the pixel in window column `x` is pattern byte
//!   `(x * bpp + k) % 16` (an assumption; Mesa writes one value into all four
//!   fill words for a 32-bit format, so for its masks the position does not
//!   matter). The source is not read.
//! - **Window.** `RS_WINDOW_SIZE` w x h, starting at the side's base address
//!   (`RS_PIPE_SOURCE_ADDR[0]` / `RS_PIPE_DEST_ADDR[0]`, else the legacy
//!   `RS_SOURCE_ADDR` / `RS_DEST_ADDR`); pixel `(x, y)` of the window is at
//!   `base + layout::offset(layout, x, y, bpp, stride)`.
//! - **TS source** (off until [`RsInterp::ts_source`] is set; for the items
//!   W8 and ts-probe-host). The TS registers' writes are always tracked. With
//!   the switch on, a source whose base is the colour (or depth) TS surface
//!   base, while `TS_MEM_CONFIG` has `COLOR_FAST_CLEAR` (or
//!   `DEPTH_FAST_CLEAR`), reads each pixel through its tile's 4-bit entry:
//!   entry `n` covers bytes `[n * T, (n + 1) * T)` of the source in memory
//!   order (T = 128, or 256 with the stride's `TS_MODE` bit), entry `n` is
//!   nibble `n % 2` (low first) of status byte `n / 2`. The clear code
//!   ([`TS_CLEAR_CODE`]) reads the clear value, 0 reads memory, any other
//!   code (compression) panics.
//!
//! Not modelled (a job using them panics): `DOWNSAMPLE_X/Y`, `FLIP`, a
//! non-zero `RS_PIPE_OFFSET`, `RS_EXTRA_CONFIG` other than 0, the
//! multi-pipe `MULTI` stride bit, a 64-bit clear value.

use super::layout::{self, Layout};
use super::regs::*;
use std::collections::BTreeMap;
use std::vec::Vec;

// Registers and fields regs.rs does not have yet (item w1-regs adds them;
// values from Mesa's hw/state_3d.xml.h, docs/notes/gc-tiled-wire.md W1).
// TODO(w1-regs): switch to regs.rs `GC_RS_DEST_STRIDE_TILING`.
const RS_DEST_STRIDE_TILING: u32 = 0x8000_0000;
// TODO(w1-regs): switch to regs.rs `GC_RS_DEST_STRIDE_SUPER_TILED_NEW`.
const RS_DEST_STRIDE_SUPER_TILED_NEW: u32 = 0x0800_0000;
// TODO(w1-regs): switch to the regs.rs name of VIVS_RS_SOURCE_STRIDE_TS_MODE.
const RS_STRIDE_TS_MODE: u32 = 0x2000_0000;
// TODO(w1-regs): switch to the regs.rs name of VIVS_RS_*_STRIDE_MULTI.
const RS_STRIDE_MULTI: u32 = 0x4000_0000;
// TODO(w1-regs): switch to the regs.rs name of VIVS_RS_*_STRIDE_STRIDE__MASK.
const RS_STRIDE_MASK: u32 = 0x0003_ffff;
// TODO(w1-regs): switch to regs.rs `GC_TS_MEM_CONFIG_DEPTH_FAST_CLEAR`.
const TS_MEM_CONFIG_DEPTH_FAST_CLEAR: u32 = 0x0000_0001;
// TODO(w1-regs): switch to regs.rs `GC_TS_MEM_CONFIG_COLOR_FAST_CLEAR`.
const TS_MEM_CONFIG_COLOR_FAST_CLEAR: u32 = 0x0000_0002;
// TODO(w1-regs): switch to regs.rs `GC_TS_COLOR_STATUS_BASE`.
const TS_COLOR_STATUS_BASE: u32 = 0x0000_1658;
// TODO(w1-regs): switch to regs.rs `GC_TS_COLOR_SURFACE_BASE`.
const TS_COLOR_SURFACE_BASE: u32 = 0x0000_165c;
// TODO(w1-regs): switch to regs.rs `GC_TS_COLOR_CLEAR_VALUE`.
const TS_COLOR_CLEAR_VALUE: u32 = 0x0000_1660;
// TODO(w1-regs): switch to regs.rs `GC_TS_DEPTH_STATUS_BASE`.
const TS_DEPTH_STATUS_BASE: u32 = 0x0000_1664;
// TODO(w1-regs): switch to regs.rs `GC_TS_DEPTH_SURFACE_BASE`.
const TS_DEPTH_SURFACE_BASE: u32 = 0x0000_1668;
// TODO(w1-regs): switch to regs.rs `GC_TS_DEPTH_CLEAR_VALUE`.
const TS_DEPTH_CLEAR_VALUE: u32 = 0x0000_166c;

/// The 4-bit TS entry of a cleared tile: the nibble of `ts::TS_CLEAR_VALUE`
/// (0x11111111, etnaviv_screen.c:884-886).
pub const TS_CLEAR_CODE: u8 = 0x1;

/// GPU memory as the interpreter sees it: byte buffers at GPU addresses.
#[derive(Default)]
pub struct Memory {
    regions: Vec<(u32, Vec<u8>)>,
}

impl Memory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Place `bytes` at GPU address `gpu`. Panics if it overlaps a region
    /// already mapped.
    pub fn map(&mut self, gpu: u32, bytes: Vec<u8>) {
        let end = gpu as u64 + bytes.len() as u64;
        for (base, b) in &self.regions {
            let (b0, b1) = (*base as u64, *base as u64 + b.len() as u64);
            assert!(end <= b0 || gpu as u64 >= b1, "rs_interp: {gpu:#x}+{:#x} overlaps {b0:#x}..{b1:#x}", bytes.len());
        }
        self.regions.push((gpu, bytes));
    }

    /// The region mapped at exactly `gpu`.
    pub fn region(&self, gpu: u32) -> &[u8] {
        &self.regions.iter().find(|(b, _)| *b == gpu).expect("rs_interp: no region at that address").1
    }

    /// The region mapped at exactly `gpu`, writable.
    pub fn region_mut(&mut self, gpu: u32) -> &mut [u8] {
        &mut self.regions.iter_mut().find(|(b, _)| *b == gpu).expect("rs_interp: no region at that address").1
    }

    fn locate(&self, addr: u64, len: usize) -> (usize, usize) {
        for (i, (base, b)) in self.regions.iter().enumerate() {
            let base = *base as u64;
            if addr >= base && addr + len as u64 <= base + b.len() as u64 {
                return (i, (addr - base) as usize);
            }
        }
        panic!("rs_interp: {addr:#x}+{len} is outside every mapped region");
    }

    /// `len` bytes at GPU address `addr`, inside one region.
    pub fn read(&self, addr: u64, len: usize) -> &[u8] {
        let (i, o) = self.locate(addr, len);
        &self.regions[i].1[o..o + len]
    }

    /// Write `bytes` at GPU address `addr`, inside one region.
    pub fn write(&mut self, addr: u64, bytes: &[u8]) {
        let (i, o) = self.locate(addr, bytes.len());
        self.regions[i].1[o..o + bytes.len()].copy_from_slice(bytes);
    }
}

/// One side of an RS job, decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Side {
    pub addr: u32,
    pub layout: Layout,
    /// The padded linear row in bytes, as [`layout::offset`] takes it (a
    /// tiled side's register value divided by 4).
    pub stride: u32,
    /// The RS format code (`GC_RS_FORMAT_*`).
    pub format: u32,
}

/// The fill of an RS job: `RS_CLEAR_CONTROL`'s clear bits and the four fill
/// words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fill {
    pub bits: u16,
    pub value: [u32; 4],
}

/// A source read through tile status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TsSource {
    pub status_base: u32,
    pub clear_value: u32,
    /// Bytes of source per 4-bit entry: 128, or 256 with `TS_MODE`.
    pub tile_bytes: u32,
}

/// One RS job as the registers describe it at the kick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RsJob {
    pub src: Side,
    pub dst: Side,
    pub w: u32,
    pub h: u32,
    pub swap_rb: bool,
    pub fill: Option<Fill>,
    pub ts: Option<TsSource>,
}

/// The interpreter: the RS and TS registers as last written, and the jobs it
/// has run.
#[derive(Default)]
pub struct RsInterp {
    regs: BTreeMap<u32, u32>,
    /// Read sources through tile status (see the module doc). Off by
    /// default: the TS registers are tracked but not applied.
    pub ts_source: bool,
    /// Every job run so far, in order.
    pub jobs: Vec<RsJob>,
}

/// The RS format's bytes per pixel.
fn bpp_of(format: u32) -> u32 {
    match format {
        GC_RS_FORMAT_A8R8G8B8 | GC_RS_FORMAT_X8R8G8B8 => 4,
        GC_RS_FORMAT_A4R4G4B4 | GC_RS_FORMAT_R5G6B5 => 2,
        f => panic!("rs_interp: RS format {f:#x} not modelled"),
    }
}

impl RsInterp {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply every write of `writes` in order, running a job at each kick.
    pub fn run(&mut self, writes: &[(u32, u32)], mem: &mut Memory) {
        for &(r, v) in writes {
            self.write(r, v, mem);
        }
    }

    /// One state write; a write of `GC_RS_KICKER` runs the job.
    pub fn write(&mut self, reg: u32, value: u32, mem: &mut Memory) {
        if reg == GC_RS_KICKER {
            let job = self.job();
            execute(&job, mem);
            self.jobs.push(job);
        } else {
            self.regs.insert(reg, value);
        }
    }

    fn reg(&self, r: u32) -> u32 {
        *self.regs.get(&r).unwrap_or_else(|| panic!("rs_interp: register {r:#06x} not written before the kick"))
    }

    fn reg_or(&self, r: u32, default: u32) -> u32 {
        self.regs.get(&r).copied().unwrap_or(default)
    }

    /// The job the registers describe now (what a kick would run).
    pub fn job(&self) -> RsJob {
        let config = self.reg(GC_RS_CONFIG);
        assert_eq!(config & (GC_RS_DOWNSAMPLE_X | GC_RS_DOWNSAMPLE_Y), 0, "rs_interp: downsampling not modelled");
        assert_eq!(config & GC_RS_CONFIG_FLIP, 0, "rs_interp: FLIP not modelled");
        let ss = self.reg(GC_RS_SOURCE_STRIDE);
        let ds = self.reg(GC_RS_DEST_STRIDE);
        let src_addr = self.regs.get(&rs_pipe_source_addr(0)).copied().unwrap_or_else(|| self.reg(GC_RS_SOURCE_ADDR));
        let dst_addr = self.regs.get(&rs_pipe_dest_addr(0)).copied().unwrap_or_else(|| self.reg(GC_RS_DEST_ADDR));
        for i in 0..2 {
            assert_eq!(self.reg_or(rs_pipe_offset(i), 0), 0, "rs_interp: RS_PIPE_OFFSET[{i}] not modelled");
        }
        assert_eq!(self.reg_or(GC_RS_EXTRA_CONFIG, 0), 0, "rs_interp: RS_EXTRA_CONFIG not modelled");
        let src = side(src_addr, config & GC_RS_SOURCE_TILED != 0, ss, config & 0x1f);
        let dst = side(dst_addr, config & GC_RS_DEST_TILED != 0, ds, (config >> 8) & 0x1f);
        let win = self.reg(GC_RS_WINDOW_SIZE);
        let clear = self.reg_or(GC_RS_CLEAR_CONTROL, 0);
        let fill = (clear & !GC_RS_CLEAR_CONTROL_BITS_ALL != 0).then(|| Fill {
            bits: (clear & GC_RS_CLEAR_CONTROL_BITS_ALL) as u16,
            value: core::array::from_fn(|i| self.reg_or(rs_fill_value(i as u32), 0)),
        });
        let ts = if self.ts_source && fill.is_none() { self.ts_of(src_addr, ss) } else { None };
        RsJob { src, dst, w: win & 0xffff, h: win >> 16, swap_rb: config & GC_RS_SWAP_RB != 0, fill, ts }
    }

    /// The TS a source at `addr` reads through, if a fast-clear TS is set
    /// up for it.
    fn ts_of(&self, addr: u32, stride: u32) -> Option<TsSource> {
        let mem_config = self.reg_or(GC_TS_MEM_CONFIG, 0);
        let tile_bytes = if stride & RS_STRIDE_TS_MODE != 0 { 256 } else { 128 };
        [
            (TS_MEM_CONFIG_COLOR_FAST_CLEAR, TS_COLOR_SURFACE_BASE, TS_COLOR_STATUS_BASE, TS_COLOR_CLEAR_VALUE),
            (TS_MEM_CONFIG_DEPTH_FAST_CLEAR, TS_DEPTH_SURFACE_BASE, TS_DEPTH_STATUS_BASE, TS_DEPTH_CLEAR_VALUE),
        ]
        .into_iter()
        .find(|&(on, surface, _, _)| mem_config & on != 0 && self.regs.get(&surface) == Some(&addr))
        .map(|(_, _, status, clear)| TsSource {
            status_base: self.reg(status),
            clear_value: self.reg_or(clear, 0),
            tile_bytes,
        })
    }
}

/// Decode one side from its RS_CONFIG tiled bit and its stride register.
fn side(addr: u32, tiled: bool, stride: u32, format: u32) -> Side {
    assert_eq!(stride & RS_STRIDE_MULTI, 0, "rs_interp: multi-pipe strides not modelled");
    // The source and destination strides share the bit positions.
    let tiling = stride & RS_DEST_STRIDE_TILING != 0;
    let new = stride & RS_DEST_STRIDE_SUPER_TILED_NEW != 0;
    assert!(!new || tiling, "rs_interp: SUPER_TILED_NEW without TILING in stride {stride:#x}");
    let layout = match (tiled, tiling) {
        (false, false) => Layout::Linear,
        (true, false) => Layout::Tiled,
        (true, true) => Layout::SuperTiled,
        (false, true) => panic!("rs_interp: stride {stride:#x} is tiled, RS_CONFIG says linear"),
    };
    let value = stride & RS_STRIDE_MASK;
    let stride = if layout == Layout::Linear {
        value
    } else {
        assert!(value.is_multiple_of(4), "rs_interp: tiled stride {value:#x} is not a multiple of 4");
        value / 4
    };
    Side { addr, layout, stride, format }
}

/// Run `job` over `mem`.
pub fn execute(job: &RsJob, mem: &mut Memory) {
    let bpp = bpp_of(job.dst.format);
    let n = bpp as usize;
    let at = |s: &Side, x: u32, y: u32| s.addr as u64 + layout::offset(s.layout, x, y, bpp, s.stride) as u64;
    if let Some(f) = job.fill {
        let mut pattern = [0u8; 16];
        for (i, v) in f.value.iter().enumerate() {
            pattern[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
        }
        for y in 0..job.h {
            for x in 0..job.w {
                let a = at(&job.dst, x, y);
                let mut px = mem.read(a, n).to_vec();
                for (k, b) in px.iter_mut().enumerate() {
                    let p = (x as usize * n + k) % 16;
                    if f.bits & (1 << p) != 0 {
                        *b = pattern[p];
                    }
                }
                mem.write(a, &px);
            }
        }
        return;
    }
    assert_eq!(bpp_of(job.src.format), bpp, "rs_interp: a copy between formats of different sizes");
    assert!(!job.swap_rb || bpp == 4, "rs_interp: SWAP_RB on a 16-bit format not modelled");
    for y in 0..job.h {
        for x in 0..job.w {
            let a = at(&job.src, x, y);
            let mut px = match job.ts.as_ref().map(|t| (t, ts_entry(t, mem, a - job.src.addr as u64))) {
                Some((t, TS_CLEAR_CODE)) => t.clear_value.to_le_bytes()[..n].to_vec(),
                Some((_, 0)) | None => mem.read(a, n).to_vec(),
                Some((_, e)) => panic!("rs_interp: TS entry {e:#x} (compressed) not modelled"),
            };
            if job.swap_rb {
                px.swap(0, 2);
            }
            mem.write(at(&job.dst, x, y), &px);
        }
    }
}

/// The 4-bit TS entry of the tile holding source byte `offset`.
fn ts_entry(t: &TsSource, mem: &Memory, offset: u64) -> u8 {
    let tile = offset / t.tile_bytes as u64;
    let byte = mem.read(t.status_base as u64 + tile / 2, 1)[0];
    (byte >> (4 * (tile % 2))) & 0xf
}

#[cfg(test)]
mod tests {
    use super::*;

    const FMT: u32 = GC_RS_FORMAT_A8R8G8B8;

    fn pattern(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 7 + i / 251) as u8).collect()
    }

    /// The RS register writes of one copy or fill, in `emit_rs`'s order.
    #[allow(clippy::too_many_arguments)]
    fn job_writes(
        config: u32,
        src: u32,
        src_stride: u32,
        dst: u32,
        dst_stride: u32,
        w: u32,
        h: u32,
        clear: u32,
        fill: u32,
    ) -> Vec<(u32, u32)> {
        let mut v = vec![
            (GC_RS_CONFIG, config),
            (GC_RS_SOURCE_STRIDE, src_stride),
            (GC_RS_DEST_STRIDE, dst_stride),
            (rs_pipe_source_addr(0), src),
            (rs_pipe_dest_addr(0), dst),
            (rs_pipe_offset(0), 0),
            (rs_pipe_offset(1), 0),
            (GC_RS_WINDOW_SIZE, rs_window_width(w) | rs_window_height(h)),
            (rs_dither(0), 0xffff_ffff),
            (rs_dither(1), 0xffff_ffff),
            (GC_RS_CLEAR_CONTROL, clear),
        ];
        v.extend((0..4).map(|i| (rs_fill_value(i), fill)));
        v.extend([
            (GC_RS_EXTRA_CONFIG, 0),
            (GC_RS_SINGLE_BUFFER, GC_RS_SINGLE_BUFFER_ENABLE),
            (GC_RS_KICKER, GC_RS_KICKER_VALUE),
            (GC_RS_SINGLE_BUFFER, 0),
        ]);
        v
    }

    fn fmt() -> u32 {
        rs_source_format(FMT) | rs_dest_format(FMT)
    }

    /// The RS stride word of a side: times 4 when tiled, with `TILING` and
    /// `SUPER_TILED_NEW` when super-tiled (etna_compile_rs_state).
    fn stride_word(l: Layout, stride: u32) -> u32 {
        match l {
            Layout::Linear => stride,
            Layout::Tiled => stride << 2,
            Layout::SuperTiled => (stride << 2) | RS_DEST_STRIDE_TILING | RS_DEST_STRIDE_SUPER_TILED_NEW,
        }
    }

    /// Lay `lin` (rows `w * 4` bytes) into `l` through `layout::offset`.
    fn lay(l: Layout, lin: &[u8], w: u32, h: u32, stride: u32, size: usize) -> Vec<u8> {
        let mut out = vec![0u8; size];
        for y in 0..h {
            for x in 0..w {
                let o = layout::offset(l, x, y, 4, stride);
                let i = ((y * w + x) * 4) as usize;
                out[o..o + 4].copy_from_slice(&lin[i..i + 4]);
            }
        }
        out
    }

    #[test]
    fn a_tiled_64x8_surface_comes_back_linear() {
        let (w, h) = (64u32, 8u32);
        let stride = layout::stride(Layout::Tiled, w, 4, true);
        let lin = pattern((w * h * 4) as usize);
        let tiled = lay(Layout::Tiled, &lin, w, h, stride, layout::size(Layout::Tiled, w, h, 4, true));
        assert_ne!(tiled, lin, "the tiled bytes differ from the linear ones");
        let (s, d) = (0x0010_0000u32, 0x0020_0000u32);
        let mut mem = Memory::new();
        mem.map(s, tiled);
        mem.map(d, vec![0; lin.len()]);
        let mut rs = RsInterp::new();
        rs.run(
            &job_writes(fmt() | GC_RS_SOURCE_TILED, s, stride_word(Layout::Tiled, stride), d, w * 4, w, h, 0, 0),
            &mut mem,
        );
        assert_eq!(rs.jobs.len(), 1);
        assert_eq!(rs.jobs[0].src.layout, Layout::Tiled);
        assert_eq!(rs.jobs[0].src.stride, stride, "the tiled stride divided back by 4");
        assert_eq!(rs.jobs[0].dst.layout, Layout::Linear);
        assert_eq!(mem.region(d), &lin[..]);
    }

    #[test]
    fn linear_to_tiled_and_back_round_trips() {
        let (w, h) = (48u32, 12u32);
        let stride = layout::stride(Layout::Tiled, w, 4, true);
        let lin = pattern((w * h * 4) as usize);
        let size = layout::size(Layout::Tiled, w, h, 4, true);
        let (a, t, b) = (0x0010_0000u32, 0x0020_0000u32, 0x0030_0000u32);
        let mut mem = Memory::new();
        mem.map(a, lin.clone());
        mem.map(t, vec![0; size]);
        mem.map(b, vec![0; lin.len()]);
        let ts = stride_word(Layout::Tiled, stride);
        let mut writes = job_writes(fmt() | GC_RS_DEST_TILED, a, w * 4, t, ts, w, h, 0, 0);
        writes.extend(job_writes(fmt() | GC_RS_SOURCE_TILED, t, ts, b, w * 4, w, h, 0, 0));
        let mut rs = RsInterp::new();
        rs.run(&writes, &mut mem);
        assert_eq!(rs.jobs.len(), 2);
        assert_eq!(mem.region(t), &lay(Layout::Tiled, &lin, w, h, stride, size)[..], "the tiled copy is layout.rs's");
        assert_eq!(mem.region(b), &lin[..], "and comes back linear");
    }

    #[test]
    fn a_fill_writes_the_value_in_the_window_only() {
        let (w, h, stride) = (32u32, 8u32, 32 * 4);
        let d = 0x0040_0000u32;
        let mut mem = Memory::new();
        mem.map(d, vec![0xaa; (stride * h) as usize]);
        let mut rs = RsInterp::new();
        let clear = GC_RS_CLEAR_CONTROL_MODE_ENABLED1 | GC_RS_CLEAR_CONTROL_BITS_ALL;
        rs.run(&job_writes(fmt(), 0, 0, d, stride, 16, 4, clear, 0x1234_5678), &mut mem);
        assert!(rs.jobs[0].fill.is_some());
        let got = mem.region(d);
        for y in 0..h {
            for x in 0..w {
                let o = (y * stride + x * 4) as usize;
                let want = if x < 16 && y < 4 { 0x1234_5678u32.to_le_bytes() } else { [0xaa; 4] };
                assert_eq!(got[o..o + 4], want, "({x}, {y})");
            }
        }

        // Depth only of a D24S8 word (README mode 12's 0xeeee): byte 0, the
        // stencil, keeps the 0x78 of the first fill.
        let mut rs = RsInterp::new();
        let clear = GC_RS_CLEAR_CONTROL_MODE_ENABLED1 | 0xeeee;
        rs.run(&job_writes(fmt(), 0, 0, d, stride, 16, 4, clear, 0x8000_0000), &mut mem);
        assert_eq!(mem.region(d)[..4], [0x78, 0, 0, 0x80]);
    }

    #[test]
    fn a_super_tiled_source_round_trips() {
        let (w, h) = (128u32, 64u32);
        let stride = layout::stride(Layout::SuperTiled, w, 4, true);
        let size = layout::size(Layout::SuperTiled, w, h, 4, true);
        let lin = pattern((w * h * 4) as usize);
        let st = stride_word(Layout::SuperTiled, stride);
        let (a, s, b) = (0x0010_0000u32, 0x0020_0000u32, 0x0030_0000u32);
        let mut mem = Memory::new();
        mem.map(a, lin.clone());
        mem.map(s, vec![0; size]);
        mem.map(b, vec![0; lin.len()]);
        let mut writes = job_writes(fmt() | GC_RS_DEST_TILED, a, w * 4, s, st, w, h, 0, 0);
        writes.extend(job_writes(fmt() | GC_RS_SOURCE_TILED, s, st, b, w * 4, w, h, 0, 0));
        let mut rs = RsInterp::new();
        rs.run(&writes, &mut mem);
        assert_eq!(rs.jobs[1].src.layout, Layout::SuperTiled);
        assert_eq!(mem.region(s), &lay(Layout::SuperTiled, &lin, w, h, stride, size)[..], "layout.rs's super-tiles");
        assert_eq!(mem.region(b), &lin[..]);
    }

    #[test]
    fn swap_rb_exchanges_bytes_0_and_2() {
        let (a, b) = (0x0010_0000u32, 0x0020_0000u32);
        let mut mem = Memory::new();
        mem.map(a, pattern(16 * 4 * 4));
        mem.map(b, vec![0; 16 * 4 * 4]);
        let mut rs = RsInterp::new();
        rs.run(&job_writes(fmt() | GC_RS_SWAP_RB, a, 64, b, 64, 16, 4, 0, 0), &mut mem);
        let (src, dst) = (mem.region(a).to_vec(), mem.region(b));
        for (s, d) in src.chunks(4).zip(dst.chunks(4)) {
            assert_eq!(d, [s[2], s[1], s[0], s[3]]);
        }
    }

    #[test]
    fn a_ts_source_reads_the_clear_value_in_cleared_tiles_once_switched_on() {
        assert_eq!(crate::vivante::ts::TS_CLEAR_VALUE & 0xf, TS_CLEAR_CODE as u32);
        let (w, h) = (64u32, 4u32);
        let stride = layout::stride(Layout::Tiled, w, 4, true);
        let size = layout::size(Layout::Tiled, w, h, 4, true); // 1 KiB: 8 tiles of 128 B
        let (s, ts, d) = (0x0010_0000u32, 0x0011_0000u32, 0x0020_0000u32);
        let src = pattern(size);
        let mut mem = Memory::new();
        mem.map(s, src.clone());
        // Tiles 0, 3 and 4 cleared (low nibble first).
        mem.map(ts, vec![0x01, 0x10, 0x01, 0x00]);
        mem.map(d, vec![0; (w * h * 4) as usize]);
        let mut writes = vec![
            (GC_TS_MEM_CONFIG, TS_MEM_CONFIG_COLOR_FAST_CLEAR),
            (TS_COLOR_STATUS_BASE, ts),
            (TS_COLOR_SURFACE_BASE, s),
            (TS_COLOR_CLEAR_VALUE, 0xc0ff_ee00),
        ];
        writes.extend(job_writes(fmt() | GC_RS_SOURCE_TILED, s, stride << 2, d, w * 4, w, h, 0, 0));

        let mut rs = RsInterp::new();
        rs.run(&writes, &mut mem);
        assert_eq!(rs.jobs[0].ts, None, "off by default");
        let plain = mem.region(d).to_vec();

        let mut rs = RsInterp::new();
        rs.ts_source = true;
        rs.run(&writes, &mut mem);
        assert_eq!(rs.jobs[0].ts, Some(TsSource { status_base: ts, clear_value: 0xc0ff_ee00, tile_bytes: 128 }));
        let got = mem.region(d);
        for y in 0..h {
            for x in 0..w {
                let tile = layout::offset(Layout::Tiled, x, y, 4, stride) / 128;
                let o = ((y * w + x) * 4) as usize;
                let want = if [0, 3, 4].contains(&tile) {
                    0xc0ff_ee00u32.to_le_bytes()
                } else {
                    plain[o..o + 4].try_into().unwrap()
                };
                assert_eq!(got[o..o + 4], want, "({x}, {y}) in tile {tile}");
            }
        }
    }

    #[test]
    #[should_panic(expected = "RS_CONFIG says linear")]
    fn a_tiled_stride_on_a_linear_side_is_refused() {
        let mut mem = Memory::new();
        mem.map(0x1000, vec![0; 4096]);
        mem.map(0x10000, vec![0; 4096]);
        RsInterp::new()
            .run(&job_writes(fmt(), 0x1000, (64 << 2) | RS_DEST_STRIDE_TILING, 0x10000, 64, 16, 4, 0, 0), &mut mem);
    }
}
