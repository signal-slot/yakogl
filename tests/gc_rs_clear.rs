// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000: the RS fill of a full-target glClear, `src/vivante/rs_clear.rs`.
//!
//! The reference is Mesa 26.2.1 etnaviv, transcribed below with its own
//! register addresses (`hw/state_3d.xml.h`, `hw/state.xml.h`, not
//! `src/vivante/regs.rs`): `etna_clear_rs` (`etnaviv_rs.c:504-581`),
//! `etna_rs_gen_clear_cmd` (`:284-335`), `etna_compile_rs_state` (`:68-187`),
//! `etna_submit_rs_state`'s RS_NEW_BASEADDR branch (`:221-257`), the
//! coalescing of `etnaviv_emit.h:170-245` and `etna_stall`
//! (`etnaviv_emit.c:55-77`), for a linear surface with no tile status on the
//! GC7000UL (one pixel pipe, SINGLE_BUFFER).
//!
//! The golden modes (`tools/vivante/README.md`, `cs_golden.c`): a 64x64
//! RGBA8 FBO with a 64x64 D24S8 renderbuffer. Modes 8 and 12 clear colour
//! and depth (not stencil) to depth 1.0 and 0.5, mode 15 depth alone to 0.5,
//! mode 14's third frame depth alone to 0.0. The README records Mesa's depth
//! fill as `RS_CLEAR_CONTROL count=5 0x0001eeee 0xffffff00` (mode 8) and
//! `... 0x80000000` (mode 12), and mode 15 as a 44-word submit: the 42 words
//! here plus the TS_FLUSH_CACHE (2 words) Mesa adds for its TS-backed depth
//! surface. Mesa's golden surface is tiled; ours is linear, so RS_CONFIG
//! has no DEST_TILED bit and RS_DEST_STRIDE is the plain byte stride.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_rs_clear
//! ```

#[path = "../src/vivante/regs.rs"]
mod regs;

#[path = "../src/vivante/rs_clear.rs"]
#[allow(dead_code)]
mod rs_clear;

use rs_clear::*;

/* ---- Mesa, transcribed ---------------------------------------------------- */

/// state_3d.xml.h / state.xml.h addresses.
const RS_KICKER: u32 = 0x1600;
const RS_CONFIG: u32 = 0x1604;
const RS_SOURCE_STRIDE: u32 = 0x160c;
const RS_DEST_STRIDE: u32 = 0x1614;
const RS_WINDOW_SIZE: u32 = 0x1620;
const RS_DITHER0: u32 = 0x1630;
const RS_CLEAR_CONTROL: u32 = 0x163c;
const RS_FILL_VALUE0: u32 = 0x1640;
const RS_EXTRA_CONFIG: u32 = 0x16a0;
const RS_SINGLE_BUFFER: u32 = 0x16b8;
const RS_PIPE_SOURCE_ADDR0: u32 = 0x16c0;
const RS_PIPE_DEST_ADDR0: u32 = 0x16e0;
const RS_PIPE_OFFSET0: u32 = 0x1700;
const GL_SEMAPHORE_TOKEN: u32 = 0x3808;
const GL_FLUSH_CACHE: u32 = 0x380c;
const GL_STALL_TOKEN: u32 = 0x3c00;
const RS_FORMAT_A4R4G4B4: u32 = 1;
const RS_FORMAT_A8R8G8B8: u32 = 6;
const SYNC_RA: u32 = 5;
const SYNC_PE: u32 = 7;
/// Mesa's coalesce pad (`etna_coalesce_end`).
const MESA_PAD: u32 = 0xdead_beef;

fn load_state(reg: u32, count: u32) -> u32 {
    0x0800_0000 | (count << 16) | (reg >> 2)
}

/// `etna_cmd_stream` with `etna_coalesce`.
#[derive(Default)]
struct Mesa {
    s: Vec<u32>,
    start: usize,
    last_reg: u32,
}

impl Mesa {
    fn set_state(&mut self, reg: u32, v: u32) {
        self.s.push(load_state(reg, 1));
        self.s.push(v);
    }
    fn stall(&mut self, from: u32, to: u32) {
        self.set_state(GL_SEMAPHORE_TOKEN, from | (to << 8));
        self.set_state(GL_STALL_TOKEN, from | (to << 8));
    }
    fn coalesce_start(&mut self) {
        self.start = self.s.len();
        self.last_reg = 0;
    }
    fn coalesce_end(&mut self) {
        let end = self.s.len();
        let size = end - self.start;
        if size != 0 {
            self.s[self.start - 1] |= (size as u32) << 16;
        }
        if end % 2 == 1 {
            self.s.push(MESA_PAD);
        }
    }
    fn emit(&mut self, reg: u32, v: u32) {
        if self.last_reg != 0 {
            if self.last_reg + 4 != reg {
                self.coalesce_end();
                self.s.push(load_state(reg, 0));
                self.start = self.s.len();
            }
        } else {
            self.s.push(load_state(reg, 0));
            self.start = self.s.len();
        }
        self.last_reg = reg;
        self.s.push(v);
    }
    /// `etna_coalsence_emit_reloc`: nothing without a bo.
    fn emit_reloc(&mut self, reg: u32, bo: Option<u32>) {
        if let Some(a) = bo {
            self.emit(reg, a);
        }
    }

    /// `etna_rs_gen_clear_cmd` + `etna_submit_rs_state`, linear destination.
    fn rs_clear(&mut self, addr: u32, pw: u32, ph: u32, bits_per_pixel: u32, clear_value: u64, clear_bits: u32) {
        let format = match bits_per_pixel {
            16 => RS_FORMAT_A4R4G4B4,
            32 => RS_FORMAT_A8R8G8B8,
            _ => unreachable!(),
        };
        assert_eq!(pw & 15, 0, "Mesa aborts");
        let config = format | (format << 8); /* no tiling, no downsample */
        let source_stride = 0; /* rs_state.source_stride unset; TS_MODE(0) */
        let dest_stride = pw * bits_per_pixel / 8;
        let window = pw | (ph << 16);
        let clear_control = (clear_bits & 0xffff) | 0x0001_0000; /* MODE_ENABLED1 */
        let fill = [clear_value as u32, (clear_value >> 32) as u32, clear_value as u32, (clear_value >> 32) as u32];

        self.coalesce_start();
        self.emit(RS_CONFIG, config);
        self.emit(RS_SOURCE_STRIDE, source_stride);
        self.emit(RS_DEST_STRIDE, dest_stride);
        self.emit_reloc(RS_PIPE_SOURCE_ADDR0, None);
        self.emit_reloc(RS_PIPE_DEST_ADDR0, Some(addr));
        self.emit(RS_PIPE_OFFSET0, 0);
        self.emit(RS_PIPE_OFFSET0 + 4, 0);
        self.emit(RS_WINDOW_SIZE, window);
        self.emit(RS_DITHER0, 0xffff_ffff);
        self.emit(RS_DITHER0 + 4, 0xffff_ffff);
        self.emit(RS_CLEAR_CONTROL, clear_control);
        for (i, v) in fill.iter().enumerate() {
            self.emit(RS_FILL_VALUE0 + 4 * i as u32, *v);
        }
        self.emit(RS_EXTRA_CONFIG, 0);
        self.emit(RS_SINGLE_BUFFER, 1);
        self.emit(RS_KICKER, 0xbeeb_beeb);
        self.emit(RS_SINGLE_BUFFER, 0);
        self.coalesce_end();
    }
}

/// A colour target for the transcription: (addr, padded w, h, bits a pixel,
/// `etna_clear_blit_pack_rgba`'s 64-bit value).
type MesaColour = (u32, u32, u32, u32, u64);
/// A depth target: (addr, padded w, h, bits a pixel, the 32-bit clear word,
/// clear bits).
type MesaZs = (u32, u32, u32, u32, u32, u32);

/// `etna_clear_rs` without tile status.
fn mesa_clear(colour: Option<MesaColour>, zs: Option<MesaZs>) -> Vec<u32> {
    let mut m = Mesa::default();
    m.set_state(GL_FLUSH_CACHE, 3);
    m.stall(SYNC_RA, SYNC_PE);
    if let Some((a, w, h, bpp, v)) = colour {
        m.rs_clear(a, w, h, bpp, v, 0xffff);
    }
    if colour.is_some() && zs.is_some() {
        m.set_state(GL_FLUSH_CACHE, 3);
    }
    if let Some((a, w, h, bpp, v, bits)) = zs {
        m.rs_clear(a, w, h, bpp, v as u64, bits);
    }
    m.stall(SYNC_RA, SYNC_PE);
    m.s
}

/// Ours equals Mesa's word for word; the only words allowed to differ are
/// pads, which must sit at the same places.
fn assert_mesa(ours: &[u32], mesa: &[u32]) {
    assert_eq!(ours.len(), mesa.len(), "length\nours {ours:08x?}\nmesa {mesa:08x?}");
    for (i, (o, m)) in ours.iter().zip(mesa).enumerate() {
        if *m == MESA_PAD {
            assert_eq!(*o, PAD, "word {i}: Mesa pads here");
        } else {
            assert_eq!(o, m, "word {i}\nours {ours:08x?}\nmesa {mesa:08x?}");
        }
    }
}

/// (register, value) pairs of a stream of LOAD_STATEs, pads dropped.
fn decode(words: &[u32]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let h = words[i];
        assert_eq!(h >> 27, 1, "word {i} {h:08x} is not a LOAD_STATE");
        let count = (h >> 16) & 0x3ff;
        let reg = (h & 0xffff) << 2;
        for k in 0..count {
            out.push((reg + 4 * k, words[i + 1 + k as usize]));
        }
        i += 1 + count as usize;
        if i % 2 == 1 {
            i += 1;
        }
    }
    out
}

/* ---- the golden modes -------------------------------------------------- */

const A: u32 = 0x4000_8000; /* any address; the golden's is the shim's */

fn rgba8_64(pixel: u32) -> ColourClear {
    ColourClear { surface: Surface { addr: A, padded_width: 64, padded_height: 64, bpp: 4 }, pixel, write_mask: 0xf }
}

fn d24s8_64(depth: f32, clear_stencil: bool) -> DepthStencilClear {
    DepthStencilClear {
        surface: Surface { addr: A + 0x4000, padded_width: 64, padded_height: 64, bpp: 4 },
        has_stencil: true,
        clear_depth: true,
        clear_stencil,
        depth,
        stencil: 0,
        depth_write: true,
        stencil_write_mask: 0xff,
    }
}

fn clear(colour: Option<ColourClear>, ds: Option<DepthStencilClear>) -> Clear {
    Clear { width: 64, height: 64, scissor: None, colour, depth_stencil: ds }
}

/// Mode 15 by hand: glClearDepthf(0.5), glClear(GL_DEPTH_BUFFER_BIT).
#[test]
fn golden_mode_15_by_hand() {
    let w = rs_clear(&clear(None, Some(d24s8_64(0.5, false)))).unwrap();
    let d = A + 0x4000;
    #[rustfmt::skip]
    let want: [u32; 42] = [
        0x0801_0e03, 0x0000_0003,               /* GL_FLUSH_CACHE COLOR|DEPTH */
        0x0801_0e02, 0x0000_0705,               /* GL_SEMAPHORE_TOKEN RA->PE */
        0x0801_0f00, 0x0000_0705,               /* GL_STALL_TOKEN RA->PE */
        0x0801_0581, 0x0000_0606,               /* RS_CONFIG A8R8G8B8, linear */
        0x0801_0583, 0x0000_0000,               /* RS_SOURCE_STRIDE: no source */
        0x0801_0585, 0x0000_0100,               /* RS_DEST_STRIDE 64 * 4 */
        0x0801_05b8, d,                         /* RS_PIPE_DEST_ADDR[0] */
        0x0802_05c0, 0, 0, PAD,                 /* RS_PIPE_OFFSET[0..1] */
        0x0801_0588, 0x0040_0040,               /* RS_WINDOW_SIZE 64x64 */
        0x0802_058c, 0xffff_ffff, 0xffff_ffff, PAD, /* RS_DITHER[0..1] */
        0x0805_058f, 0x0001_eeee,               /* RS_CLEAR_CONTROL count=5: depth bits */
        0x8000_0000, 0, 0x8000_0000, 0,         /* RS_FILL_VALUE[0..3] */
        0x0801_05a8, 0x0000_0000,               /* RS_EXTRA_CONFIG */
        0x0801_05ae, 0x0000_0001,               /* RS_SINGLE_BUFFER on */
        0x0801_0580, 0xbeeb_beeb,               /* RS_KICKER */
        0x0801_05ae, 0x0000_0000,               /* RS_SINGLE_BUFFER off */
        0x0801_0e02, 0x0000_0705,
        0x0801_0f00, 0x0000_0705,
    ];
    assert_eq!(w.as_slice(), &want[..]);
    /* Mesa's 44-word submit for this mode is these 42 plus its
     * TS_FLUSH_CACHE (a TS-backed depth surface), which ours does not have */
    assert_eq!(w.len + 2, 44);
    assert_mesa(w.as_slice(), &mesa_clear(None, Some((d, 64, 64, 32, 0x8000_0000, 0xeeee))));
}

/// The README's golden lines: `RS_CLEAR_CONTROL count=5 0x0001eeee <word>`.
fn clear_control_run(words: &[u32]) -> Vec<&[u32]> {
    let hdr = load_state(RS_CLEAR_CONTROL, 5);
    words.windows(6).filter(|w| w[0] == hdr).map(|w| &w[1..]).collect()
}

#[test]
fn golden_modes_8_and_12_colour_and_depth() {
    for (depth, word) in [(1.0f32, 0xffff_ff00u32), (0.5, 0x8000_0000)] {
        let pixel = 0xff00_0000; /* glClearColor(0, 0, 0, 1), A8R8G8B8 */
        let w = rs_clear(&clear(Some(rgba8_64(pixel)), Some(d24s8_64(depth, false)))).unwrap();
        assert_mesa(
            w.as_slice(),
            &mesa_clear(
                Some((A, 64, 64, 32, (pixel as u64) << 32 | pixel as u64)),
                Some((A + 0x4000, 64, 64, 32, word, 0xeeee)),
            ),
        );
        let runs = clear_control_run(w.as_slice());
        assert_eq!(runs.len(), 2, "a colour fill and a depth fill");
        assert_eq!(runs[0], &[0x0001_ffff, pixel, pixel, pixel, pixel][..]);
        assert_eq!(runs[1], &[0x0001_eeee, word, 0, word, 0][..], "README mode 8 / 12");
        /* the flush between the two fills (etnaviv_rs.c:572-574) */
        assert_eq!(w.len, 2 + 4 + FILL_WORDS + 2 + FILL_WORDS + 4);
        assert_eq!(w.len, MAX_WORDS);
    }
}

/// Mode 14's third frame: glClearDepthf(0), glClear(GL_DEPTH_BUFFER_BIT).
#[test]
fn golden_mode_14_depth_zero() {
    let w = rs_clear(&clear(None, Some(d24s8_64(0.0, false)))).unwrap();
    assert_eq!(clear_control_run(w.as_slice()), vec![&[0x0001_eeee, 0, 0, 0, 0][..]]);
    assert_mesa(w.as_slice(), &mesa_clear(None, Some((A + 0x4000, 64, 64, 32, 0, 0xeeee))));
}

/* ---- the other planes ---------------------------------------------------- */

#[test]
fn stencil_bits_follow_mesa_s8z24() {
    let mut ds = d24s8_64(0.25, true);
    ds.stencil = 0x5a;
    let word = d24s8_word(0.25, 0x5a);
    assert_eq!(word, (0x40_0000 << 8) | 0x5a); /* lrint(0.25 * 0xffffff) = 0x400000 (4194303.75) */
    let w = rs_clear(&clear(None, Some(ds))).unwrap();
    assert_eq!(clear_control_run(w.as_slice()), vec![&[0x0001_ffff, word, 0, word, 0][..]]);

    /* stencil alone: the stencil byte */
    ds.clear_depth = false;
    let w = rs_clear(&clear(None, Some(ds))).unwrap();
    assert_eq!(clear_control_run(w.as_slice())[0][0], 0x0001_1111);
    assert_mesa(w.as_slice(), &mesa_clear(None, Some((A + 0x4000, 64, 64, 32, word, 0x1111))));
}

#[test]
fn no_stencil_plane_writes_the_whole_word() {
    /* X8Z24: 4 bytes, no stencil */
    let mut ds = d24s8_64(0.5, true);
    ds.has_stencil = false;
    let w = rs_clear(&clear(None, Some(ds))).unwrap();
    assert_mesa(w.as_slice(), &mesa_clear(None, Some((A + 0x4000, 64, 64, 32, 0x8000_0000, 0xffff))));

    /* Z16: A4R4G4B4, the word doubled (translate_clear_depth_stencil) */
    ds.surface.bpp = 2;
    let w = rs_clear(&clear(None, Some(ds))).unwrap();
    assert_eq!(z16_word(0.5), 0x8000_8000); /* lrint(32767.5) = 32768 */
    assert_mesa(w.as_slice(), &mesa_clear(None, Some((A + 0x4000, 64, 64, 16, 0x8000_8000, 0xffff))));
    assert_eq!(decode(w.as_slice()).iter().find(|(r, _)| *r == RS_CONFIG), Some(&(RS_CONFIG, 0x0101)));
}

#[test]
fn a_16_bit_colour_is_doubled() {
    let mut col = rgba8_64(0xf00f);
    col.surface.bpp = 2;
    col.surface.padded_width = 32;
    col.surface.padded_height = 8;
    let w = rs_clear(&Clear { width: 30, height: 7, scissor: None, colour: Some(col), depth_stencil: None }).unwrap();
    assert_mesa(w.as_slice(), &mesa_clear(Some((A, 32, 8, 16, 0xf00f_f00f_f00f_f00f)), None));
}

#[test]
fn shadow_sized_targets_match_mesa() {
    /* glmark2's 800x600 canvas (padded 800x600) and shadow's 1600x1200 pass */
    for (w, h) in [(800u32, 600u32), (1600, 1200), (256, 256), (16, 4)] {
        let col = ColourClear {
            surface: Surface { addr: A, padded_width: w, padded_height: h, bpp: 4 },
            pixel: 0x8040_2010,
            write_mask: 0xf,
        };
        let mut ds = d24s8_64(1.0, true);
        ds.surface.padded_width = w;
        ds.surface.padded_height = h;
        let words = rs_clear(&Clear { width: w, height: h, scissor: None, colour: Some(col), depth_stencil: Some(ds) })
            .unwrap();
        assert_mesa(
            words.as_slice(),
            &mesa_clear(Some((A, w, h, 32, 0x8040_2010_8040_2010)), Some((A + 0x4000, w, h, 32, 0xffff_ff00, 0xffff))),
        );
    }
}

/// The colour fill writes what `gc_draw_clear` (gc_draw.c:217-259, board
/// PASS) writes, but for the source: Mesa leaves RS_PIPE_SOURCE_ADDR alone
/// and RS_SOURCE_STRIDE 0, the bring-up clear points both at the target.
#[test]
fn colour_fill_is_gc_draw_clear_but_for_the_source() {
    let argb = 0xff20_4060;
    let w = rs_clear(&clear(Some(rgba8_64(argb)), None)).unwrap();
    let stride = 64 * 4;
    #[rustfmt::skip]
    let gc_draw_clear: Vec<(u32, u32)> = vec![
        (GL_FLUSH_CACHE, 3), (GL_SEMAPHORE_TOKEN, 0x705), (GL_STALL_TOKEN, 0x705),
        (RS_CONFIG, 0x606), (RS_SOURCE_STRIDE, stride), (RS_DEST_STRIDE, stride),
        (RS_PIPE_SOURCE_ADDR0, A), (RS_PIPE_DEST_ADDR0, A),
        (RS_PIPE_OFFSET0, 0), (RS_PIPE_OFFSET0 + 4, 0), (RS_WINDOW_SIZE, 64 | (64 << 16)),
        (RS_DITHER0, !0), (RS_DITHER0 + 4, !0), (RS_CLEAR_CONTROL, 0x0001_ffff),
        (RS_FILL_VALUE0, argb), (RS_FILL_VALUE0 + 4, argb), (RS_FILL_VALUE0 + 8, argb), (RS_FILL_VALUE0 + 12, argb),
        (RS_EXTRA_CONFIG, 0), (RS_SINGLE_BUFFER, 1), (RS_KICKER, 0xbeeb_beeb), (RS_SINGLE_BUFFER, 0),
        (GL_SEMAPHORE_TOKEN, 0x705), (GL_STALL_TOKEN, 0x705),
    ];
    let ours = decode(w.as_slice());
    let theirs: Vec<(u32, u32)> = gc_draw_clear
        .into_iter()
        .filter(|(r, _)| *r != RS_PIPE_SOURCE_ADDR0)
        .map(|(r, v)| if r == RS_SOURCE_STRIDE { (r, 0) } else { (r, v) })
        .collect();
    assert_eq!(ours, theirs);
}

/* ---- the depth word ---------------------------------------------------- */

#[test]
fn depth_word_is_lrint_half_to_even() {
    let reference = |z: f32| -> u32 {
        let z = z as f64;
        if z <= 0.0 || z.is_nan() {
            0
        } else if z >= 1.0 {
            0xffff_ff00
        } else {
            ((z * 16_777_215.0).round_ties_even() as u32) << 8
        }
    };
    assert_eq!(d24s8_word(1.0, 0), 0xffff_ff00);
    assert_eq!(d24s8_word(0.5, 0), 0x8000_0000, "8388607.5 rounds to even");
    assert_eq!(d24s8_word(0.0, 0), 0);
    assert_eq!(d24s8_word(-1.0, 0), 0);
    assert_eq!(d24s8_word(2.0, 7), 0xffff_ff07);
    assert_eq!(d24s8_word(f32::NAN, 0), 0);
    let n = 1u32 << 20;
    for i in 0..=n {
        let z = i as f32 / n as f32;
        assert_eq!(d24s8_word(z, 0), reference(z), "z = {z}");
    }
    /* every f32 next to the halfway points of the first codes */
    for k in 0..4096u32 {
        let z = ((k as f64 + 0.5) / 16_777_215.0) as f32;
        for z in [f32::from_bits(z.to_bits() - 1), z, f32::from_bits(z.to_bits() + 1)] {
            assert_eq!(d24s8_word(z, 0), reference(z), "z = {z:e}");
        }
    }
}

/* ---- where the quad stays ------------------------------------------------ */

#[test]
fn a_scissor_that_does_not_cover_the_target_keeps_the_quad() {
    let mut c = clear(Some(rgba8_64(0)), Some(d24s8_64(1.0, false)));
    c.scissor = Some((0, 0, 64, 63));
    assert_eq!(rs_clear(&c), Err(NotRs::Scissored));
    c.scissor = Some((1, 0, 64, 64));
    assert_eq!(rs_clear(&c), Err(NotRs::Scissored));
    c.scissor = Some((0, 0, 64, 64));
    assert!(rs_clear(&c).is_ok(), "a box of the whole target is no scissor");
    c.scissor = Some((-8, -8, 100, 100));
    assert!(rs_clear(&c).is_ok());
}

#[test]
fn masks_keep_the_quad() {
    for mask in [0x0u8, 0x7, 0xe, 0xb] {
        let mut col = rgba8_64(0);
        col.write_mask = mask;
        assert_eq!(rs_clear(&clear(Some(col), None)), Err(NotRs::ColourMasked), "mask {mask:#x}");
    }
    let mut ds = d24s8_64(1.0, false);
    ds.depth_write = false;
    assert_eq!(rs_clear(&clear(Some(rgba8_64(0)), Some(ds))), Err(NotRs::DepthMasked));
    let mut ds = d24s8_64(1.0, true);
    ds.stencil_write_mask = 0x0f;
    assert_eq!(rs_clear(&clear(None, Some(ds))), Err(NotRs::StencilMasked));
    /* a stencil mask does not matter when stencil is not cleared */
    ds.clear_stencil = false;
    assert!(rs_clear(&clear(None, Some(ds))).is_ok());
}

#[test]
fn odd_sizes_keep_the_quad() {
    for (w, h) in [(72u32, 64u32), (64, 62), (8, 4), (16, 2), (0, 64), (64, 0), (0x1_0000, 4)] {
        let mut col = rgba8_64(0);
        col.surface.padded_width = w;
        col.surface.padded_height = h;
        let c = Clear { width: w, height: h, scissor: None, colour: Some(col), depth_stencil: None };
        assert_eq!(rs_clear(&c), Err(NotRs::Unaligned), "{w}x{h}");
        let mut ds = d24s8_64(1.0, true);
        ds.surface.padded_width = w;
        ds.surface.padded_height = h;
        let c = Clear { width: w, height: h, scissor: None, colour: Some(rgba8_64(0)), depth_stencil: Some(ds) };
        assert_eq!(rs_clear(&c), Err(NotRs::Unaligned), "depth {w}x{h}");
    }
}

#[test]
fn other_pixel_sizes_and_empty_clears_keep_the_quad() {
    for bpp in [1u32, 3, 8, 16] {
        let mut col = rgba8_64(0);
        col.surface.bpp = bpp;
        assert_eq!(rs_clear(&clear(Some(col), None)), Err(NotRs::Bpp), "colour {bpp}");
    }
    let mut ds = d24s8_64(1.0, false);
    ds.surface.bpp = 2; /* a stencil byte needs 4 bytes a pixel */
    assert_eq!(rs_clear(&clear(None, Some(ds))), Err(NotRs::Bpp));

    assert_eq!(rs_clear(&clear(None, None)), Err(NotRs::Nothing));
    let mut ds = d24s8_64(1.0, true);
    ds.clear_depth = false;
    ds.has_stencil = false; /* stencil asked of a surface without one */
    assert_eq!(rs_clear(&clear(None, Some(ds))), Err(NotRs::Nothing));
}
