// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A host-test interpreter of the resolve engine (RS) over a captured
//! stream (docs/notes/gc-tiled-wire.md, W2). Compiled with `host-test` only.
//!
//! It takes `(register, value)` state writes, as a `GcCmd` stream decodes to,
//! and a [`Mem`] (GPU address ranges over byte buffers). On each
//! `GC_RS_KICKER` write it does the RS job with Mesa's semantics for this
//! subset:
//!
//! - source and destination LINEAR, TILED (4x4) or SUPER_TILED, through
//!   [`layout::offset`]. `RS_CONFIG` SOURCE_TILED/DEST_TILED says tiled; with
//!   the stride's TILING bit as well the surface is super-tiled (assumption:
//!   etnaviv sets the stride bit only with SUPER_TILED, `etnaviv_rs.c:108-135`).
//!   A tiled stride is written times 4 (a tile row), so the row stride is the
//!   field divided by 4;
//! - window `w` x `h` from `RS_WINDOW_SIZE`, addresses from pipe 0;
//! - a 16- or 32-bit pixel bit copy (the width of the source format), with
//!   `SWAP_RB` for 32 bits;
//! - fill: `RS_CLEAR_CONTROL` mode non-zero writes no source; a destination
//!   byte whose position `k` in its 16-byte group has bit `k` of the clear
//!   bits set gets byte `k` of the fill value (four fill registers);
//! - an optional TS source ([`TsSource`], off by default; the TS registers
//!   are not decoded yet): a source tile whose 4-bit entry is the clear code
//!   reads the clear value instead of memory.
//!
//! Not modelled: downsample, flip, dither, multi-pipe, pipe offsets. A kick
//! with DOWNSAMPLE or FLIP set panics rather than guess.

use super::layout::{self, Layout};
use super::regs::*;
use std::collections::HashMap;

// TODO: use the regs.rs name once w1-regs lands it (DEST_STRIDE_TILING; the
// dest stride shares the source stride's bit positions).
const STRIDE_VALUE_MASK: u32 = 0x0003_ffff;

/// GPU memory: ranges of bytes at GPU addresses.
#[derive(Default)]
pub struct Mem {
    regions: Vec<(u64, Vec<u8>)>,
}

impl Mem {
    pub fn new() -> Self {
        Self::default()
    }

    /// A zeroed range of `len` bytes at `base`.
    pub fn add(&mut self, base: u64, len: usize) {
        self.add_bytes(base, vec![0; len]);
    }

    pub fn add_bytes(&mut self, base: u64, bytes: Vec<u8>) {
        self.regions.push((base, bytes));
    }

    /// The bytes of the range that starts at `base`.
    pub fn bytes(&self, base: u64) -> &[u8] {
        &self.regions.iter().find(|(b, _)| *b == base).expect("no region at that address").1
    }

    pub fn bytes_mut(&mut self, base: u64) -> &mut [u8] {
        &mut self.regions.iter_mut().find(|(b, _)| *b == base).expect("no region at that address").1
    }

    fn find(&self, addr: u64, n: usize) -> (usize, usize) {
        let i = self
            .regions
            .iter()
            .position(|(b, v)| addr >= *b && addr + n as u64 <= *b + v.len() as u64)
            .unwrap_or_else(|| panic!("RS access of {n} bytes at {addr:#x} outside every region"));
        (i, (addr - self.regions[i].0) as usize)
    }

    pub fn read(&self, addr: u64, out: &mut [u8]) {
        let (i, o) = self.find(addr, out.len());
        out.copy_from_slice(&self.regions[i].1[o..o + out.len()]);
    }

    pub fn write(&mut self, addr: u64, data: &[u8]) {
        let (i, o) = self.find(addr, data.len());
        self.regions[i].1[o..o + data.len()].copy_from_slice(data);
    }
}

/// The tile status of the RS source. Set by the caller until the TS registers
/// are decoded (TODO: fill it from the TS and RS_EXTRA_CONFIG registers
/// w1-regs adds).
#[derive(Clone, Copy, Debug)]
pub struct TsSource {
    /// GPU address of the 4-bit-per-tile status buffer (low nibble first).
    pub base: u64,
    /// Bytes of surface one entry covers (`ts::tile_size`: 128 or 256).
    pub tile_bytes: u32,
    /// The entry value that means "cleared" (a nibble of `ts::TS_CLEAR_VALUE`).
    pub clear_code: u8,
    /// The pixel a cleared tile reads, as the source format holds it.
    pub clear_value: u32,
}

/// The interpreter: the register file survives between kicks, as the
/// hardware's does.
#[derive(Default)]
pub struct RsInterp {
    regs: HashMap<u32, u32>,
    /// The TS of the source, if any.
    pub ts_source: Option<TsSource>,
    /// RS jobs done.
    pub kicks: usize,
}

impl RsInterp {
    pub fn new() -> Self {
        Self::default()
    }

    fn reg(&self, r: u32) -> u32 {
        self.regs.get(&r).copied().unwrap_or(0)
    }

    /// Apply `writes` in order; each `GC_RS_KICKER` write does one job.
    pub fn run(&mut self, writes: &[(u32, u32)], mem: &mut Mem) {
        for &(r, v) in writes {
            self.regs.insert(r, v);
            if r == GC_RS_KICKER {
                self.kick(mem);
                self.kicks += 1;
            }
        }
    }

    // The layout and the row stride in bytes of one side.
    fn surface(tiled: bool, stride_reg: u32) -> (Layout, u32) {
        let field = stride_reg & STRIDE_VALUE_MASK;
        if !tiled {
            return (Layout::Linear, field);
        }
        let layout = if stride_reg & GC_RS_SOURCE_STRIDE_TILING != 0 { Layout::SuperTiled } else { Layout::Tiled };
        (layout, field / 4)
    }

    fn kick(&self, mem: &mut Mem) {
        let config = self.reg(GC_RS_CONFIG);
        assert_eq!(
            config & (GC_RS_DOWNSAMPLE_X | GC_RS_DOWNSAMPLE_Y | GC_RS_FLIP),
            0,
            "RS interpreter: no downsample or flip"
        );
        let win = self.reg(GC_RS_WINDOW_SIZE);
        let (w, h) = (win & 0xffff, win >> 16);
        let (dl, ds) = Self::surface(config & GC_RS_DEST_TILED != 0, self.reg(GC_RS_DEST_STRIDE));
        let dst = self.reg(rs_pipe_dest_addr(0)) as u64;
        let sfmt = config & 0x1f;
        let bpp = if sfmt == GC_RS_FORMAT_A4R4G4B4 || sfmt == GC_RS_FORMAT_R5G6B5 { 2 } else { 4 };
        let control = self.reg(GC_RS_CLEAR_CONTROL);
        if control & 0xffff_0000 != 0 {
            let bits = control & GC_RS_CLEAR_CONTROL_BITS_ALL;
            let fill: Vec<u8> = (0..4).flat_map(|i| self.reg(rs_fill_value(i)).to_le_bytes()).collect();
            for y in 0..h {
                for x in 0..w {
                    let o = layout::offset(dl, x, y, bpp, ds) as u64;
                    for k in 0..bpp as u64 {
                        let p = ((o + k) & 15) as usize;
                        if bits >> p & 1 != 0 {
                            mem.write(dst + o + k, &fill[p..p + 1]);
                        }
                    }
                }
            }
            return;
        }
        let (sl, ss) = Self::surface(config & GC_RS_SOURCE_TILED != 0, self.reg(GC_RS_SOURCE_STRIDE));
        let src = self.reg(rs_pipe_source_addr(0)) as u64;
        let swap = config & GC_RS_SWAP_RB != 0 && bpp == 4;
        for y in 0..h {
            for x in 0..w {
                let so = layout::offset(sl, x, y, bpp, ss) as u64;
                let mut px = [0u8; 4];
                match self.ts_clear(mem, so) {
                    Some(v) => px = v.to_le_bytes(),
                    None => mem.read(src + so, &mut px[..bpp as usize]),
                }
                if swap {
                    px.swap(0, 2);
                }
                mem.write(dst + layout::offset(dl, x, y, bpp, ds) as u64, &px[..bpp as usize]);
            }
        }
    }

    // The clear value if the source byte offset `so` lies in a cleared tile.
    fn ts_clear(&self, mem: &Mem, so: u64) -> Option<u32> {
        let ts = self.ts_source?;
        let idx = so / ts.tile_bytes as u64;
        let mut b = [0u8; 1];
        mem.read(ts.base + idx / 2, &mut b);
        let entry = (b[0] >> (4 * (idx & 1))) & 0xf;
        (entry == ts.clear_code).then_some(ts.clear_value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: u64 = 0x10_0000;
    const DST: u64 = 0x20_0000;

    fn pattern(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 7 + i / 251) as u8).collect()
    }

    // The words of one RS copy; per side None = linear, Some(super_tiled).
    fn copy_words(w: u32, h: u32, src: Option<bool>, dst: Option<bool>, stride: u32) -> Vec<(u32, u32)> {
        let fmt = GC_RS_FORMAT_A8R8G8B8;
        let side = |t: Option<bool>| match t {
            None => stride,
            Some(sup) => {
                let sup_bits = GC_RS_SOURCE_STRIDE_TILING | GC_RS_SOURCE_STRIDE_SUPER_TILED_NEW;
                stride * 4 | if sup { sup_bits } else { 0 }
            }
        };
        let mut c = rs_source_format(fmt) | rs_dest_format(fmt);
        if src.is_some() {
            c |= GC_RS_SOURCE_TILED;
        }
        if dst.is_some() {
            c |= GC_RS_DEST_TILED;
        }
        vec![
            (GC_RS_CONFIG, c),
            (GC_RS_SOURCE_STRIDE, side(src)),
            (GC_RS_DEST_STRIDE, side(dst)),
            (rs_pipe_source_addr(0), SRC as u32),
            (rs_pipe_dest_addr(0), DST as u32),
            (GC_RS_WINDOW_SIZE, rs_window_width(w) | rs_window_height(h)),
            (GC_RS_CLEAR_CONTROL, 0),
            (GC_RS_KICKER, GC_RS_KICKER_VALUE),
        ]
    }

    fn laid(layout: Layout, lin: &[u8], w: u32, h: u32, stride: u32) -> Vec<u8> {
        let mut out = vec![0u8; (stride * layout::aligned_height(layout, h, true)) as usize];
        layout::linear_to_layout(layout, layout::SUPER_ORDER, &mut out, stride, lin, w * 4, w, h, 4);
        out
    }

    #[test]
    fn tiled_source_comes_back_linear() {
        let (w, h) = (64u32, 8u32);
        let stride = layout::stride(Layout::Tiled, w, 4, true);
        let lin = pattern((w * h * 4) as usize);
        let mut mem = Mem::new();
        mem.add_bytes(SRC, laid(Layout::Tiled, &lin, w, h, stride));
        mem.add(DST, (stride * h) as usize);
        let mut rs = RsInterp::new();
        rs.run(&copy_words(w, h, Some(false), None, stride), &mut mem);
        assert_eq!(rs.kicks, 1);
        assert_eq!(mem.bytes(DST), &lin[..]);
    }

    #[test]
    fn linear_to_tiled_round_trip() {
        let (w, h) = (64u32, 8u32);
        let stride = layout::stride(Layout::Tiled, w, 4, true);
        let lin = pattern((stride * h) as usize);
        let mut mem = Mem::new();
        mem.add_bytes(SRC, lin.clone());
        mem.add(DST, lin.len());
        mem.add(0x30_0000, lin.len());
        let mut rs = RsInterp::new();
        rs.run(&copy_words(w, h, None, Some(false), stride), &mut mem);
        assert_eq!(mem.bytes(DST), &laid(Layout::Tiled, &lin, w, h, stride)[..]);
        // and back: tiled -> linear into a third buffer
        let mut back = copy_words(w, h, Some(false), None, stride);
        back[3].1 = DST as u32;
        back[4].1 = 0x30_0000;
        rs.run(&back, &mut mem);
        assert_eq!(rs.kicks, 2);
        assert_eq!(mem.bytes(0x30_0000), &lin[..]);
    }

    #[test]
    fn fill_writes_the_window_only() {
        let (w, h, stride) = (64u32, 16u32, 256u32);
        let mut mem = Mem::new();
        mem.add_bytes(DST, vec![0xaa; (stride * h) as usize]);
        let mut words = vec![
            (GC_RS_CONFIG, rs_source_format(GC_RS_FORMAT_A8R8G8B8) | rs_dest_format(GC_RS_FORMAT_A8R8G8B8)),
            (GC_RS_DEST_STRIDE, stride),
            (rs_pipe_dest_addr(0), DST as u32),
            (GC_RS_WINDOW_SIZE, rs_window_width(32) | rs_window_height(8)),
            (GC_RS_CLEAR_CONTROL, GC_RS_CLEAR_CONTROL_MODE_ENABLED1 | GC_RS_CLEAR_CONTROL_BITS_ALL),
        ];
        words.extend((0..4).map(|i| (rs_fill_value(i), 0x1122_3344)));
        words.push((GC_RS_KICKER, GC_RS_KICKER_VALUE));
        RsInterp::new().run(&words, &mut mem);
        let out = mem.bytes(DST);
        for y in 0..h as usize {
            for x in 0..w as usize {
                let px = &out[y * stride as usize + x * 4..][..4];
                let inside = x < 32 && y < 8;
                assert_eq!(px, if inside { [0x44, 0x33, 0x22, 0x11] } else { [0xaa; 4] }, "({x},{y})");
            }
        }
    }

    #[test]
    fn super_tiled_source_round_trip() {
        let (w, h) = (64u32, 64u32);
        let stride = layout::stride(Layout::SuperTiled, w, 4, true);
        let lin = pattern((w * h * 4) as usize);
        let mut mem = Mem::new();
        mem.add_bytes(SRC, laid(Layout::SuperTiled, &lin, w, h, stride));
        mem.add(DST, (stride * h) as usize);
        RsInterp::new().run(&copy_words(w, h, Some(true), None, stride), &mut mem);
        assert_eq!(mem.bytes(DST), &lin[..]);
    }

    #[test]
    fn swap_rb_and_ts_clear_tiles() {
        let (w, h) = (16u32, 4u32);
        let stride = layout::stride(Layout::Linear, w, 4, true);
        let lin = pattern((stride * h) as usize);
        let mut mem = Mem::new();
        mem.add_bytes(SRC, lin.clone());
        mem.add(DST, lin.len());
        // 128-byte TS entries over a 256-byte surface: entry 0 cleared, entry 1 not
        mem.add_bytes(0x40_0000, vec![0x01]);
        let mut words = copy_words(w, h, None, None, stride);
        words[0].1 |= GC_RS_SWAP_RB;
        let mut rs = RsInterp::new();
        rs.run(&words, &mut mem);
        assert_eq!(mem.bytes(DST)[..4], [lin[2], lin[1], lin[0], lin[3]]);
        rs.ts_source = Some(TsSource { base: 0x40_0000, tile_bytes: 128, clear_code: 1, clear_value: 0x0102_0304 });
        rs.run(&words, &mut mem);
        let out = mem.bytes(DST);
        // bytes 0..128 are entry 0 (cleared): the value, red and blue swapped
        assert_eq!(out[..4], [0x02, 0x03, 0x04, 0x01]);
        assert_eq!(out[128..132], [lin[130], lin[129], lin[128], lin[131]]);
    }
}
