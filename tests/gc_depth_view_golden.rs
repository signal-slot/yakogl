// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): the depth view's texture-unit words
//! (src/vivante/depth_sample.rs mode 1, docs/notes/gc-depth-sample-wire.md
//! outcome (a): the D24S8 surface sampled in place as TEXTURE_FORMAT_D24X8)
//! compared with Mesa's words for a depth texture sampled the same way, in
//! `tools/vivante/testdata/depth-view.golden` (cs_golden mode 31, Mesa 26.2.1
//! etnaviv on the GC7000UL drm-shim row).
//!
//! The golden's scene, glmark2 shadow's shape: a 64x64 GL_DEPTH_COMPONENT /
//! GL_UNSIGNED_INT texture, NEAREST and CLAMP_TO_EDGE, the only attachment of
//! an FBO; a depth clear and one GL_LESS quad into it; then a quad into an
//! RGBA8 target that samples it from unit 0 (`texture2D(u_depth, uv).x`).
//! Mesa samples the depth surface itself (the descriptor's LOD 0 is the
//! draw's PE_PIPE_DEPTH_ADDR): there is no copy, as in outcome (a).
//!
//! Ours: the binding the GL core gives that texture (R32F texels, ES 2.0
//! luminance, `depth_id` set) turned into the view by
//! `depth_sample::view_binding` in the knob's mode (`gc_debug_set_depth_tpu
//! (DEPTH_TPU_D24X8)`, read back through `depth_sample::mode`), over a 64x64
//! `ds_cache` surface (256-byte rows, 64 rows). CONFIG0-2 are
//! `texture::format_words` of the view, the words `build_descriptor` writes;
//! the other descriptor words and the sampler words are the arithmetic
//! `build_descriptor`, `emit_slot` and `write_sampler` (src/vivante/
//! texture.rs) apply to the view's fields, written out below field by field.
//!
//! Compared: slot 0's NTE_DESCRIPTOR_TX_CTRL, SAMP_CTRL0, SAMP_CTRL1,
//! SAMP_LOD_MINMAX, SAMP_LOD_BIAS and SAMP_ANISOTROPY at the sampling draw,
//! and every word of its 256-byte descriptor (the golden's --tex-desc
//! section). Every field that differs is named in [`EXPECTED`] with its
//! reason, and the list is exact both ways: a bit that differs outside a
//! listed field fails, and so does a listed field that no longer differs. No
//! listed field is the D24X8 view's own choice of format, swizzle, filter or size: those
//! fields equal Mesa's. The layout is (ADDRESSING, HALIGN): the view is
//! LINEAR by design, and whether the texture unit decodes D24X8 from a LINEAR
//! surface is the board's question (gl_test `dtpu` checks 4 and 5), which no
//! golden can answer.
//!
//! Pure: no GL context, no OSAL. Built without `report-es2`; nothing here
//! depends on the report.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test gc_depth_view_golden
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature = "test-knobs"))]

use std::collections::BTreeMap;
use std::sync::Mutex;
use yakogl::backend::{gles_texfmt_bytes, GlesTextureBind, GLES_TEXFMT_R32F, GLES_TEXTYPE_2D, GLES_WRAP_CLAMP_TO_EDGE};
use yakogl::vivante::depth_sample::{
    gc_debug_set_depth_tpu, mode, view_binding, DepthView, DEPTH_TPU_D24X8, DEPTH_TPU_OFF, GC_TEXFMT_DEPTH_VIEW,
    GC_TEX_FORMAT_D24X8,
};
use yakogl::vivante::regs::*;
use yakogl::vivante::texture::{format_words, gc_texfmt};

const GOLDEN: &str = include_str!("../tools/vivante/testdata/depth-view.golden");

/// The texture and surface of the golden's scene.
const W: u32 = 64;
const H: u32 = 64;
/// Where our surface lies (any 64-aligned address: it is masked).
const SURF: u64 = 0x0012_3400;

/// NTE_DESCRIPTOR_TX_CTRL's fields (Mesa state_3d.xml.h; regs.rs has no
/// name for them, nothing of ours writes them).
const TX_CTRL_TS_MODE: u32 = 0x0000_0001;
const TX_CTRL_TS_ENABLE: u32 = 0x0000_0002;
const TX_CTRL_COMPRESSION: u32 = 0x0000_0020;
/// SAMP_CTRL0's WWRAP field (bits 6-8).
const SAMP_CTRL0_WWRAP: u32 = 0x0000_01c0;
/// TEXDESC_ASTC0 (texdesc_3d.xml.h:120) and its three UNK bytes.
const TEXDESC_ASTC0: u32 = 0x58;
const ASTC0_UNK_C: u32 = 0x0c0c_0c00;
/// TS_SAMPLER_SURFACE_BASE[0] (state_3d.xml.h), the sampler's tile-status
/// surface: regs.rs has no name for it.
const TS_SAMPLER_SURFACE_BASE0: u32 = 0x01a80;

/// The knob is process-global and libtest runs tests on parallel threads.
static SERIAL: Mutex<()> = Mutex::new(());

/* ---- Mesa's words: the golden -------------------------------------------------------- */

struct Golden {
    /// Each register's value at the sampling draw (the second DRAW_INSTANCED).
    regs: BTreeMap<u32, u32>,
    /// PE_PIPE_DEPTH_ADDR[0] at the depth draw (the first).
    depth_addr: u32,
    /// Slot 0's descriptor, offset -> word (the words --tex-desc prints; the
    /// ones it leaves out are 0).
    desc: BTreeMap<u32, u32>,
}

impl Golden {
    fn reg(&self, r: u32) -> u32 {
        *self.regs.get(&r).unwrap_or_else(|| panic!("the golden writes no {r:#x} before its sampling draw"))
    }
    fn desc(&self, off: u32) -> u32 {
        self.desc.get(&off).copied().unwrap_or(0)
    }
}

/// `cs_decode.py`'s print: `LOAD_STATE <addr> <name> count=<n> <values>`
/// (a run's registers 4 bytes apart; a long run's print ends in `...`, and
/// the values it leaves out are left out here too), then the --tex-desc
/// section's `+0x<off> <name> 0x<word>` lines under `==== texture
/// descriptor, slot <n> (at ...) ====`.
fn golden() -> Golden {
    let hex = |t: &str| u32::from_str_radix(t.trim_start_matches("0x"), 16).unwrap();
    let mut regs = BTreeMap::new();
    let (mut draws, mut depth_addr, mut at_draw) = (0, None, None);
    let mut slot = None;
    let mut desc = BTreeMap::new();
    for l in GOLDEN.lines() {
        if let Some(rest) = l.strip_prefix("==== texture descriptor, slot ") {
            slot = rest.split_whitespace().next().map(|s| s.parse::<u32>().unwrap());
            continue;
        }
        if let Some(rest) = l.trim_start().strip_prefix("+0x") {
            if slot == Some(0) {
                let mut it = rest.split_whitespace();
                let off = hex(it.next().unwrap());
                let w = hex(it.nth(1).unwrap());
                desc.insert(off, w);
            }
            continue;
        }
        if l.starts_with("DRAW_INSTANCED") {
            draws += 1;
            if draws == 1 {
                depth_addr = regs.get(&pe_pipe_depth_addr(0)).copied();
            } else if draws == 2 {
                at_draw = Some(regs.clone());
            }
            continue;
        }
        if !l.starts_with("LOAD_STATE") {
            continue;
        }
        let mut it = l.split_whitespace().skip(1);
        let reg = hex(it.next().unwrap());
        let vals = it.skip_while(|t| !t.starts_with("count=")).skip(1).filter(|t| *t != "FIXP");
        for (k, t) in vals.take_while(|t| t.starts_with("0x")).enumerate() {
            regs.insert(reg + 4 * k as u32, hex(t));
        }
    }
    assert_eq!(draws, 2, "the golden has the depth draw and the sampling draw");
    assert!(!desc.is_empty(), "the golden has slot 0's descriptor");
    Golden { regs: at_draw.unwrap(), depth_addr: depth_addr.expect("the depth draw has a PE_PIPE_DEPTH_ADDR[0]"), desc }
}

/* ---- our words ------------------------------------------------------------------------- */

/// The binding the GL core gives glmark2's (and the golden's) depth texture
/// on the ES 2.0 surface once the depth answer said yes: R32F texels (stale,
/// the view replaces them), luminance, no compare (16) of a fixed-point depth
/// (8), NEAREST, clamped, one level, the depth's GL id in `depth_id`.
fn texture() -> GlesTextureBind {
    GlesTextureBind {
        unit: 0,
        format: GLES_TEXFMT_R32F,
        type_: GLES_TEXTYPE_2D,
        layers: 1,
        base_gpu: 0x0080_0000,
        width: W,
        height: H,
        stride_texels: W,
        bytes: W * H * 4,
        mip_levels: 1,
        wrap_s: GLES_WRAP_CLAMP_TO_EDGE,
        wrap_t: GLES_WRAP_CLAMP_TO_EDGE,
        no_alpha: 1,
        compare_op: 16 | 8,
        depth_luminance: 1,
        depth_id: (1 << 30) | 1,
        ..GlesTextureBind::default()
    }
}

/// `ds_cache`'s surface for the texture: 64-pixel aligned rows and stride.
fn surface() -> DepthView {
    DepthView { gpu: SURF, stride: W.next_multiple_of(64) * 4, rows: H.next_multiple_of(64) }
}

struct Ours {
    regs: BTreeMap<u32, u32>,
    desc: BTreeMap<u32, u32>,
}

/// The view's words, with the knob in mode 1 (put back to off after).
fn ours() -> Ours {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let was = gc_debug_set_depth_tpu(DEPTH_TPU_D24X8);
    let m = mode();
    gc_debug_set_depth_tpu(was);
    assert_eq!(m, DEPTH_TPU_D24X8, "the knob reads back as mode 1");
    let v = view_binding(&texture(), m, Some(surface())).expect("the view takes a 64x64 texture on its surface");
    assert_eq!(v.format, GC_TEXFMT_DEPTH_VIEW);
    assert_eq!(v.mip_levels, 1);

    /* texture.rs build_descriptor: no tiled copy (one level, 2D, 4-byte:
     * tiled_copy::lookup finds none) and not a block format, so `tiled` is
     * false; one level at base_gpu, LINEAR_STRIDE stride_texels * bytes,
     * SLICE stride * height, one layer */
    let fmt = gc_texfmt(v.format);
    assert!(!fmt.compressed && fmt.decoded);
    let [config0, config1, config2] = format_words(&v, false);
    let stride = v.stride_texels * gles_texfmt_bytes(v.format);
    let log2_fixp88 = |x: u32| {
        assert!(x.is_power_of_two(), "the scene's size is a power of two");
        x.ilog2() << 8
    };
    let mut desc = BTreeMap::new();
    desc.insert(texdesc_lod_addr(0), v.base_gpu as u32);
    desc.insert(GC_TEXDESC_CONFIG0, config0);
    desc.insert(GC_TEXDESC_SIZE, texdesc_size_width(v.width) | texdesc_size_height(v.height));
    desc.insert(GC_TEXDESC_LINEAR_STRIDE, stride);
    desc.insert(GC_TEXDESC_CONFIG1, config1);
    desc.insert(GC_TEXDESC_BASELOD, texdesc_baselod_base(0) | texdesc_baselod_max(v.mip_levels as u32 - 1));
    desc.insert(GC_TEXDESC_CONFIG2, config2);
    desc.insert(
        GC_TEXDESC_LOG_SIZE_EXT,
        texdesc_log_width(log2_fixp88(v.width)) | texdesc_log_height(log2_fixp88(v.height)),
    );
    desc.insert(GC_TEXDESC_VOLUME, log2_fixp88(1));
    desc.insert(GC_TEXDESC_SLICE, stride * v.height);
    desc.insert(GC_TEXDESC_3D_CONFIG, texdesc_3d_depth(1));
    desc.retain(|_, w| *w != 0);

    /* texture.rs emit_slot and write_sampler: a 2D lookup's W wrap is U's;
     * filters from the binding; MIP NONE for one level; UNK21; INT_FILTER as
     * the class says; no tile status; LOD range 0..0xfff; no bias, no
     * anisotropy */
    let wrap = |w: u8| match w {
        GLES_WRAP_CLAMP_TO_EDGE => GC_TEX_WRAP_CLAMP_TO_EDGE,
        _ => panic!("the scene clamps"),
    };
    let filter = |linear: u8| if linear != 0 { GC_TEX_FILTER_LINEAR } else { GC_TEX_FILTER_NEAREST };
    let (u, vw) = (wrap(v.wrap_s), wrap(v.wrap_t));
    let samp_ctrl0 = samp_uwrap(u)
        | samp_vwrap(vw)
        | samp_wwrap(u)
        | samp_min(filter(v.min_linear))
        | samp_mip(GC_TEX_FILTER_NONE)
        | samp_mag(filter(v.linear_filter))
        | GC_SAMP_CTRL0_UNK21
        | if fmt.int_filter { GC_SAMP_INT_FILTER } else { 0 };
    let mut regs = BTreeMap::new();
    regs.insert(nte_descriptor_tx_ctrl(0), 0);
    regs.insert(nte_descriptor_samp_ctrl0(0), samp_ctrl0);
    regs.insert(nte_descriptor_samp_ctrl1(0), GC_SAMP_CTRL1_UNK1);
    regs.insert(nte_descriptor_samp_lod_minmax(0), samp_lod_max(GC_SAMP_LOD_FP58_MAX) | samp_lod_min(0));
    regs.insert(nte_descriptor_samp_lod_bias(0), 0);
    regs.insert(nte_descriptor_samp_anisotropy(0), 0);
    Ours { regs, desc }
}

/* ---- the comparison ----------------------------------------------------------------- */

/// What a compared word is: a sampler register or a descriptor offset.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Word {
    Reg(u32),
    Desc(u32),
}

/// One expected difference: the field of `word` (its bits) in which our word and Mesa's
/// differ, and why.
struct Expected {
    word: Word,
    name: &'static str,
    bits: u32,
    why: &'static str,
}

/// Every field in which the view's words differ from the golden's. None is a
/// defect of depth_sample.rs: two are the layout the view is by design, one
/// is tile status (not built), and three are differences of every texture
/// this back end samples, not of the view.
const EXPECTED: &[Expected] = &[
    Expected {
        word: Word::Desc(0x00),
        name: "LOD_ADDR[0]",
        bits: u32::MAX,
        why: "an address: the shim's allocator against this test's surface. What compares is that \
              each is its depth surface (Mesa's is the depth draw's PE_PIPE_DEPTH_ADDR[0], ours the \
              DepthView's gpu: checked by name below)",
    },
    Expected {
        word: Word::Desc(GC_TEXDESC_CONFIG0),
        name: "CONFIG0 ADDRESSING",
        bits: texdesc_addressing_mode(3),
        why: "Mesa's depth resource is SUPER_TILED (etnaviv_resource.c:572-576; the golden's \
              PE_DEPTH_CONFIG has SUPER_TILED), so its descriptor is TILED (0); ds_cache's surface is \
              row-linear, so the view is LINEAR (3) by design (gc-depth-sample-wire.md (a).3). \
              Whether D24X8 decodes a LINEAR surface is dtpu checks 4/5 on the board",
    },
    Expected {
        word: Word::Desc(GC_TEXDESC_CONFIG1),
        name: "CONFIG1 HALIGN",
        bits: texdesc_halign(GC_TEXTURE_HALIGN_SUPER_TILED),
        why: "the same layout: Mesa writes the super-tiled resource's halign (2, \
              etnaviv_texture_desc.c:186); a LINEAR view has none (0). Outcome (b).1's copy \
              (depth_sample::copy_sampler_words) is the one that takes Mesa's 2",
    },
    Expected {
        word: Word::Reg(nte_descriptor_tx_ctrl(0)),
        name: "TX_CTRL",
        bits: TX_CTRL_TS_MODE | TX_CTRL_TS_ENABLE | TX_CTRL_COMPRESSION,
        why: "Mesa samples the depth through its tile status (TS_ENABLE, TS_MODE 1, COMPRESSION, \
              TS_INDEX 0; with TS_SAMPLER_*[0] at the depth's TS and surface); this back end has no \
              tile status, its write_sampler writes 0 for every slot. TS is gc-tiled-wire.md W7-W9, \
              gated on the X2's MMU-off probe",
    },
    Expected {
        word: Word::Reg(nte_descriptor_samp_ctrl0(0)),
        name: "SAMP_CTRL0 WWRAP",
        bits: SAMP_CTRL0_WWRAP,
        why: "Mesa writes GL's wrap_r (REPEAT, 0; etnaviv_texture_desc.c:108); texture.rs's \
              emit_slot gives a 2D lookup U's wrap (CLAMP, 2), as for every 2D texture \
              (tests/gc_npot_repeat.rs pins it). A 2D lookup has no third coordinate",
    },
    Expected {
        word: Word::Desc(TEXDESC_ASTC0),
        name: "ASTC0",
        bits: ASTC0_UNK_C,
        why: "Mesa writes ASTC0's three UNK bytes (0xc each) into every descriptor, ASTC or not \
              (etnaviv_texture_desc.c:194-197); texture.rs's build_descriptor writes no ASTC0 for \
              any texture. Not of the view",
    },
];

fn expected_bits(w: Word) -> u32 {
    EXPECTED.iter().filter(|e| e.word == w).fold(0, |m, e| m | e.bits)
}

/// The sampler registers compared.
fn sampler_regs() -> [(u32, &'static str); 6] {
    [
        (nte_descriptor_tx_ctrl(0), "TX_CTRL[0]"),
        (nte_descriptor_samp_ctrl0(0), "SAMP_CTRL0[0]"),
        (nte_descriptor_samp_ctrl1(0), "SAMP_CTRL1[0]"),
        (nte_descriptor_samp_lod_minmax(0), "SAMP_LOD_MINMAX[0]"),
        (nte_descriptor_samp_lod_bias(0), "SAMP_LOD_BIAS[0]"),
        (nte_descriptor_samp_anisotropy(0), "SAMP_ANISOTROPY[0]"),
    ]
}

/// The view's sampler and descriptor words are Mesa's, but for the fields
/// [`EXPECTED`] names; and every field it names does differ.
#[test]
fn depth_view_words_match_mesas_but_the_listed_bits() {
    let g = golden();
    let o = ours();
    let mut wrong = Vec::new();
    let mut check = |w: Word, name: String, ours: u32, theirs: u32| {
        /* outside the listed field the words are equal; inside it they differ */
        let field = expected_bits(w);
        let diff = ours ^ theirs;
        if diff & !field != 0 || (field != 0 && diff & field == 0) {
            wrong.push(format!(
                "{name}: ours {ours:#010x}, Mesa {theirs:#010x}, differ in {diff:#010x}, listed field {field:#010x}"
            ));
        }
    };
    for (r, name) in sampler_regs() {
        check(Word::Reg(r), name.into(), o.regs[&r], g.reg(r));
    }
    let offs: std::collections::BTreeSet<u32> = g.desc.keys().chain(o.desc.keys()).copied().collect();
    for off in (0..GC_TEXDESC_BYTES as u32).step_by(4) {
        let (ours, theirs) = (o.desc.get(&off).copied().unwrap_or(0), g.desc(off));
        if offs.contains(&off) || expected_bits(Word::Desc(off)) != 0 {
            check(Word::Desc(off), format!("descriptor +{off:#04x}"), ours, theirs);
        }
    }
    assert!(wrong.is_empty(), "the depth view's words against depth-view.golden:\n{}", wrong.join("\n"));
    for e in EXPECTED {
        assert!(!e.name.is_empty() && !e.why.is_empty());
        if let Word::Reg(r) = e.word {
            assert!(sampler_regs().iter().any(|(x, _)| *x == r), "listed {} is not compared", e.name);
        }
    }
}

/// The fields outcome (a) rests on, one by one, so a failure says which:
/// Mesa samples the depth surface itself as D24X8, 2D, swizzle (R, R, R, 1),
/// NEAREST with no mip and no INT_FILTER, no compare; and so does the view.
#[test]
fn mesa_samples_the_depth_surface_itself_as_d24x8() {
    let g = golden();
    let o = ours();
    let c0 = g.desc(GC_TEXDESC_CONFIG0);
    assert_eq!((c0 >> 13) & 0x1f, GC_TEX_FORMAT_D24X8, "Mesa: FORMAT D24X8 (0x11)");
    assert_eq!(c0 & 7, GC_TEX_TYPE_2D, "Mesa: TYPE 2D");
    assert_eq!((c0 >> 20) & 3, GC_TEX_ADDRESSING_TILED, "Mesa: TILED (its depth is super-tiled)");
    let oc0 = o.desc[&GC_TEXDESC_CONFIG0];
    assert_eq!((oc0 >> 13) & 0x1f, GC_TEX_FORMAT_D24X8, "ours: FORMAT D24X8");
    assert_eq!((oc0 >> 20) & 3, GC_TEX_ADDRESSING_LINEAR, "ours: LINEAR");

    let sw = texdesc_swizzle_r(GC_SWIZZLE_R)
        | texdesc_swizzle_g(GC_SWIZZLE_R)
        | texdesc_swizzle_b(GC_SWIZZLE_R)
        | texdesc_swizzle_a(GC_SWIZZLE_1);
    let c1 = g.desc(GC_TEXDESC_CONFIG1);
    assert_eq!(c1, sw | texdesc_halign(GC_TEXTURE_HALIGN_SUPER_TILED), "Mesa: (R, R, R, 1), no FORMAT_EXT");
    assert_eq!(o.desc[&GC_TEXDESC_CONFIG1], sw, "ours: (R, R, R, 1), no FORMAT_EXT, HALIGN 0");

    /* the surface itself, no copy */
    assert_eq!(g.desc(texdesc_lod_addr(0)), g.depth_addr, "Mesa's LOD 0 is the depth draw's surface");
    assert_eq!(g.reg(TS_SAMPLER_SURFACE_BASE0), g.depth_addr, "and so is its sampler TS surface");
    assert_eq!(o.desc[&texdesc_lod_addr(0)] as u64, surface().gpu, "ours is the DepthView's address");
    for lod in 1..16 {
        assert_eq!(g.desc(texdesc_lod_addr(lod)), 0, "Mesa: one level");
    }
    assert_eq!(g.desc(GC_TEXDESC_LINEAR_STRIDE), surface().stride, "the 64-wide surface's 256-byte rows");
    assert_eq!(g.desc(GC_TEXDESC_SLICE), surface().stride * H);

    let s0 = g.reg(nte_descriptor_samp_ctrl0(0));
    assert_eq!(s0 & GC_SAMP_INT_FILTER, 0, "Mesa: no INT_FILTER for a depth format");
    assert_eq!(s0 & 0x001f_0000, 0, "Mesa: no compare, DEPTH_STENCIL_MODE depth");
    assert_eq!((s0 >> 11) & 3, GC_TEX_FILTER_NONE, "Mesa: MIP NONE");
    assert_eq!((s0 >> 9) & 3, GC_TEX_FILTER_NEAREST, "Mesa: MIN NEAREST");
    assert_eq!((s0 >> 13) & 3, GC_TEX_FILTER_NEAREST, "Mesa: MAG NEAREST");
    assert!(!gc_texfmt(GC_TEXFMT_DEPTH_VIEW).int_filter, "ours: no INT_FILTER");
}

/// With the knob off there is no view: mode 0 refuses to build one.
#[test]
fn mode_off_builds_no_view() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let was = gc_debug_set_depth_tpu(DEPTH_TPU_OFF);
    let m = mode();
    gc_debug_set_depth_tpu(was);
    assert_eq!(m, DEPTH_TPU_OFF);
    assert!(view_binding(&texture(), m, Some(surface())).is_err());
}
