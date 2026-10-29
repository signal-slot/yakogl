// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A 4x frame split into two kicks against Mesa 26.2.1 on the board:
//! gl_test case 214 splitkick (tools/cts/log/2026-10-28-splitref/README.md,
//! issue #32). Mesa keeps the four samples of a multisampled surface in
//! memory, one plane a sample: kick 1 writes them back there (Writeback
//! MSAA Layered), kick 2 draws them back with a pre-frame draw that runs
//! once a sample and fetches its own sample of a 4-sample texture over the
//! same planes, and writes them back again. The resolve to one sample comes
//! after the last kick.
//!
//! The frame builder does this with a [`SampleStore`]. Each kick is built
//! at its own pool address from the README (kick 1 0x7fffffea8000, kick 2
//! 0x7ffffffc0000), with the store at Mesa's 4x buffer and the preload
//! program at Mesa's address, and compared with the README's words:
//! sections 4.1 (framebuffer parameters), 4.2 (render target), 4.6 (the
//! pre-frame DCD and every block it names) and 5 (what differs between the
//! kicks). Every word is equal but the pointers into the descriptor pool,
//! each listed with its cause; a pointer is followed on both sides and the
//! block it names is compared in turn.
//!
//! The resolve is ours, not Mesa's (Mesa blits with four TEX_FETCHes and
//! an average in a third frame, section 4): a frame with no draws whose
//! pre-frame draw is kick 2's and whose render target averages the four
//! samples into the single-sampled target ([`resolve_frame`]).

use super::arena::Arena;
use super::cs::Stream;
use super::frame::*;
use super::genxml::v10::*;

/* the README's addresses: each kick's descriptor pool, Mesa's 4x buffer
 * (RT 0's writeback base), the single-sampled pbuffer, the preload program,
 * the sample-position table (FBD w4:w5 0x7ffffffff200 is its rotated-4x
 * block, pattern 2 at 256 bytes a pattern) */
const KICK_1_POOL: u64 = 0x7fff_ffea_8000;
const KICK_2_POOL: u64 = 0x7fff_fffc_0000;
const STORE: u64 = 0x7fff_fffd_0000;
const PBUFFER: u64 = 0x7fff_fffe_0000;
const PRELOAD_BINARY: u64 = 0x7fff_ffff_e180;
const SAMPLES: u64 = 0x7fff_ffff_f000;
const HEAP_DESC: u64 = 0x7fff_ffff_b000;
const GEOMETRY: u64 = 0x7fff_fffe_7000;

/// One sample's plane: 64 rows of 256 bytes (RT 0 w11, plane +0x18).
pub(super) const SURFACE_STRIDE: u32 = 0x4000;

/// The case's 4x RGB8 pbuffer, 64x64 (section 1): B, G, R, X bytes (RT 0's
/// swizzle BGR0, the texture's BGR1).
const TARGET: Target =
    Target { gpu: PBUFFER, width: 64, height: 64, row_stride: 256, format: TargetFormat::Xrgb8888, samples: 4 };

/* section 4.1 */
pub(super) const KICK_1_FBD: [u32; 16] = [
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0xfffff200, 0x00007fff, 0x00000000, 0x00000000, 0x003f003f,
    0x00000000, 0x003f003f, 0x08001292, 0x00010000, 0x00000000, 0xffea8400, 0x00007fff,
];
pub(super) const KICK_2_FBD: [u32; 16] = [
    0x00000002, 0x00000000, 0x00000000, 0x00000000, 0xfffff200, 0x00007fff, 0xfffc08c0, 0x00007fff, 0x003f003f,
    0x00000000, 0x003f003f, 0x08001292, 0x00010000, 0x00000000, 0xfffc0400, 0x00007fff,
];
/* section 4.2 */
pub(super) const KICK_1_RT: [u32; 16] = [
    0x04000000, 0x880ab299, 0, 0, 0, 0, 0, 0, 0xfffd0000, 0x00007fff, 0x00000100, 0x00004000, 0xff000000, 0xff000000,
    0xff000000, 0xff000000,
];
pub(super) const KICK_2_RT: [u32; 16] =
    [0x04000000, 0x080ab299, 0, 0, 0, 0, 0, 0, 0xfffd0000, 0x00007fff, 0x00000100, 0x00004000, 0, 0, 0, 0];
/* section 4.6: the pre-frame-0 DCD at kick 2's pool +0x8C0 */
pub(super) const PRELOAD_DCD: [u32; 32] = [
    0x00080a03, 0x0001ffff, 0, 0, 0, 0, 0x00000000, 0x3f800000, 0, 0, 0xfffc0c20, 0x00007fff, 0xfffc0c01, 0x00007fff,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xfffc0b84, 0x00007fff, 0xfffc0bc0, 0x00007fff, 0xfffc0100, 0x00007fff, 0, 0,
];
/* the blocks it names (section 4.6's table), in kick 2's pool */
const RESOURCE_TABLE: [u32; 16] = [
    0xfffc0b20, 0x01007fff, 0x00000020, 0, 0xfffc0b40, 0x01007fff, 0x00000020, 0, 0xfffc0b00, 0x01007fff, 0x00000020,
    0, 0xfffc0a40, 0x01007fff, 0x00000020, 0,
];
const ATTRIBUTE: [u32; 8] = [0x2dc04015, 0x00000001, 0, 0, 0x00000010, 0, 0, 0];
const BUFFER: [u32; 8] = [0x00000019, 0x00000040, 0xfffc0880, 0x00007fff, 0, 0, 0, 0];
const CORNERS: [u32; 16] = [
    0x00000000, 0x00000000, 0x00000000, 0x3f800000, 0x42800000, 0x00000000, 0x00000000, 0x3f800000, 0x00000000,
    0x42800000, 0x00000000, 0x3f800000, 0x42800000, 0x42800000, 0x00000000, 0x3f800000,
];
const SAMPLER: [u32; 8] = [0x1c299901, 0, 0, 0, 0, 0, 0, 0];
pub(super) const TEXTURE: [u32; 8] = [0x2ec05022, 0x003f003f, 0x00000688, 0x00004000, 0xfffc0a80, 0x00007fff, 0, 0];
pub(super) const PLANE: [u32; 8] =
    [0x0200021a, 0x00010000, 0xfffd0000, 0x00007fff, 0x00000100, 0x00000000, 0x00004000, 0x003f003f];
const SPD: [u32; 8] = [0x80000128, 0x00003000, 0xffffe180, 0x00007fff, 0, 0, 0, 0];
const BLEND: [u32; 4] = [0x00000a00, 0xf0122122, 0x00000019, 0x000ed000];
const ZSD: [u32; 8] = [0x024f24f7, 0xffffffff, 0, 0, 0xe4800000, 0, 0, 0];

fn meta(stage: ShaderStage) -> ShaderMeta<'static> {
    ShaderMeta {
        no_psiz_offset: None,
        stage,
        register_allocation: ShaderRegisterAllocation::_32_PER_THREAD,
        preload: 0,
        sysvals: &[],
        push: &[],
        varyings: 0,
        varying_bytes: 0,
        secondary: None,
        colour_outputs: u32::from(stage == ShaderStage::FRAGMENT),
        samples_texture: false,
        requires_helper_threads: false,
        can_discard: false,
        writes_depth: false,
        writes_stencil: false,
        writes_coverage: false,
        writes_global: false,
        ubo: &[],
    }
}

/// What one frame of the split is.
#[derive(Clone, Copy, PartialEq)]
enum Kick {
    /// Kick 1: cleared black, the triangle, the samples kept.
    First,
    /// Kick 2: the samples drawn back, the triangle, the samples kept.
    Second,
    /// Our resolve: the samples drawn back, no draw, averaged into the
    /// target.
    Resolve,
    /// A frame that starts from the target's image while the store holds
    /// nothing of it (`SampleStore::reload` off): the target drawn back
    /// into every sample, the triangle, the samples kept.
    Unkept,
    /// The same frame without a store: averaged into the target.
    NoStore,
}

/// The single-sampled colour preload program's address in [`Kick::Unkept`]
/// and [`Kick::NoStore`] (any address but the per-sample program's).
const PRELOAD_1X_BINARY: u64 = 0x7fff_ffff_e100;

/// A built kick: its descriptor pool's bytes from `pool`, and its FBD's,
/// TLS's and tiler context's addresses.
struct Built {
    pool: u64,
    bytes: Vec<u8>,
    fbd: u64,
    tls: u64,
    tiler: Option<u64>,
}

impl Built {
    fn words(&self, gpu: u64, n: usize) -> Vec<u32> {
        let at = (gpu - self.pool) as usize;
        self.bytes[at..at + 4 * n].as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect()
    }
}

fn build(pool: u64, kick: Kick) -> Built {
    let (mut desc_m, mut code_m, mut spd_m, mut ctx_m) =
        (vec![0u8; 0x4000], vec![0u8; 0x800], vec![0u8; 0x100], vec![0u8; 0x200]);
    let mut words = vec![0u64; 512];
    let mut ctx_a = Arena::new("context", HEAP_DESC, &mut ctx_m).unwrap();
    let heap = tiler_heap_descriptor(&mut ctx_a, 0x8000_00a0_0000, 2 << 20).unwrap();
    let ctx = Context {
        heap: heap.gpu,
        geometry_buffer: GEOMETRY,
        geometry_buffer_size: GEOMETRY_BUFFER_BYTES,
        sample_positions: SAMPLES,
        tiler_max_levels: 8,
    };
    let (fbd, tls, tiler);
    {
        let mut desc = Arena::new("desc", pool, &mut desc_m).unwrap();
        let mut code = Arena::new("code", 0x7fff_ffff_c000, &mut code_m).unwrap();
        let mut spds = Arena::new("spds", 0x7fff_ffff_d000, &mut spd_m).unwrap();
        let bin = [0u8; 128];
        let vs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::VERTEX)).unwrap();
        let fs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::FRAGMENT)).unwrap();
        let attributes = [VertexAttribute {
            buffer: VertexBuffer { gpu: 0x7fff_ffec_c000, size: 48 },
            offset: 0,
            stride: 16,
            components: 4,
            instance_divisor: 0,
        }];
        let draw = DrawCall {
            sample_mask: 0xf,
            vs: &vs,
            fs: &fs,
            primitive: Topology::Triangles,
            first_vertex: 0,
            vertex_count: 3,
            instance_count: 1,
            indices: None,
            attributes: &attributes,
            viewport: Viewport { x: 0, y: 0, width: 64, height: 64, near: 0.0, far: 1.0 },
            scissor: None,
            line_width: 1.0,
            flip_y: true,
            blend_color: [0.0; 4],
            vs_uniforms: &[],
            fs_uniforms: &[],
            zs: DepthStencilState::OFF,
            colour_mask: 0xf,
            front_cw: false,
            cull: Cull::None,
            textures: &[],
            vs_texture: None,
            occlusion_query: None,
            alpha_to_coverage: false,
            blend_shader: None,
            blend: None,
            polygon_offset: None,
        };
        let mut cs = Stream::new(&mut words, 92);
        /* the store holds the samples from kick 2 on (kick 1 is cleared) */
        let reload = matches!(kick, Kick::Second | Kick::Resolve);
        let store = SampleStore { gpu: STORE, surface_stride: SURFACE_STRIDE, reload, resolve: false };
        /* the program's preload word (SPD w1 0x3000: r60, r61); the 1x
         * program's is the same */
        let preload = FrameShader { binary: PRELOAD_BINARY, preload: 0x3000 };
        let layout = if kick == Kick::Resolve {
            resolve_frame(&mut desc, &mut cs, &ctx, &TARGET, store, preload).unwrap().layout
        } else {
            let mut open = begin_frame(&mut desc, &mut cs).unwrap();
            open.sample_store = (kick != Kick::NoStore).then_some(store);
            open.colour_preload = match kick {
                Kick::Second => Some(preload),
                Kick::Unkept | Kick::NoStore => Some(FrameShader { binary: PRELOAD_1X_BINARY, ..preload }),
                _ => None,
            };
            add_draw(&mut open, &mut desc, &mut cs, &ctx, &TARGET, false, &draw).unwrap();
            let clear = if kick == Kick::First { [0.0, 0.0, 0.0, 1.0] } else { [0.0; 4] };
            end_frame(&open, &mut desc, &mut cs, &ctx, &TARGET, clear, None).unwrap().layout
        };
        (fbd, tls, tiler) = (layout.fbd.unwrap().gpu, layout.tls.unwrap().gpu, layout.tiler.map(|b| b.gpu));
    }
    Built { pool, bytes: desc_m, fbd, tls, tiler }
}

/// A word of ours that is not Mesa's, with its cause: (index, Mesa's
/// word, ours, cause).
type Named = (usize, u32, u32, &'static str);

/// FBD w11, a 4x frame's tile: Mesa 26.2.1 also bounds the tile by the
/// depth tile budget whether or not the frame has a depth buffer
/// (`pan_zsbuf_bytes_per_pixel`, pan_desc.h:242-259: 4 bytes a sample;
/// pan_desc.c:617-625), half the G610's 16 KiB Z tile buffer
/// (`MODEL_TB_SIZES(32768, 16384)`, pan_model.c:88; pan_props.h:61-72):
/// 8192 >> log2(16) = 512 pixels, and the colour allocation 16 * 512 =
/// 8192. The builder follows Mesa 25.0.7, which has no such bound: 1024
/// pixels and 16384 bytes for a 4x RGBA8 frame (frame.rs `tile_size`, the
/// board's 25.0.7 dump that frame_msaa_tests.rs mesa-msrtt pins).
const TILE_26_2: Named = (
    11,
    0x08001292,
    0x10001492,
    "FBD w11 Effective Tile Size log2 10 (1024) and Color Buffer Allocation 16 (16384): Mesa 25.0.7's 4x tile, \
     not 26.2.1's (9, 512 and 8, 8192), which also bounds it by the depth tile budget",
);

/// Ours against Mesa's words, but the 64-bit pointers at the word indices
/// `pointers` (low word first), returned as (ours, Mesa's) for the caller
/// to follow, and the `named` words, which must be Mesa's and ours as named.
fn compare(name: &str, ours: &[u32], mesa: &[u32], pointers: &[usize], named: &[Named]) -> Vec<(u64, u64)> {
    assert_eq!(ours.len(), mesa.len());
    let ptr = |w: &[u32], i: usize| u64::from(w[i]) | u64::from(w[i + 1]) << 32;
    let mut expected = mesa.to_vec();
    for &i in pointers {
        expected[i..i + 2].copy_from_slice(&ours[i..i + 2]);
    }
    for &(i, theirs, our, cause) in named {
        assert_eq!(mesa[i], theirs, "{name} w{i}: Mesa's word is not what the cause says: {cause}");
        expected[i] = our;
        eprintln!("mali split kick vs Mesa: {name} w{i} {theirs:#010x} -> {our:#010x}: {cause}");
    }
    let differ: Vec<usize> = (0..ours.len()).filter(|&i| ours[i] != expected[i]).collect();
    assert!(differ.is_empty(), "{name}: words {differ:?} differ\n  mesa {mesa:08x?}\n  ours {ours:08x?}");
    pointers.iter().map(|&i| (ptr(ours, i), ptr(mesa, i))).collect()
}

/// The tiler context pointer (FBD w14:w15) is the one FBD word that differs:
/// Mesa's tiler context is at its pool +0x400, after the tiler OOM context
/// and the three incremental-rendering FBDs it prepares for every batch
/// with a draw (pan_csf.c:1032-1051; README section 4.6's last paragraph),
/// which the builder does not build (frame.rs module documentation). Ours
/// is the frame's own tiler context, or 0 for a frame without a draw.
fn check_fbd(name: &str, b: &Built, mesa: &[u32; 16]) -> FramebufferParameters {
    let ours = b.words(b.fbd, 16);
    let p = compare(name, &ours, mesa, &[14], &[TILE_26_2]);
    assert_eq!(p[0].0, b.tiler.unwrap_or(0), "{name}: our tiler pointer is the frame's tiler context");
    /* the 16 words of padding after it, zero on both sides */
    assert_eq!(b.words(b.fbd + 64, 16), [0; 16], "{name}: FBD padding");
    FramebufferParameters::unpack(&b.bytes[(b.fbd - b.pool) as usize..][..64].try_into().unwrap()).unwrap()
}

/// Kick 1 (sections 4.1, 4.2): the clear and the first triangle in a 4x
/// tile buffer of the rotated grid, no frame shader, and a render target
/// that writes the four samples back as four planes of the store (Writeback
/// MSAA Layered, surface stride 0x4000) with clean tiles written and the
/// clear colour black.
#[test]
fn a_split_4x_frame_s_first_kick_keeps_the_four_samples() {
    let b = build(KICK_1_POOL, Kick::First);
    let p = check_fbd("kick 1 FBD", &b, &KICK_1_FBD);
    assert_eq!(p.frame_shader_dcds, 0);
    let rt = b.words(b.fbd + 128, 16);
    compare("kick 1 RT", &rt, &KICK_1_RT, &[], &[]);
    /* the frame's own TLS at pool +0x100, as Mesa's (kick 2's DCD w28) */
    assert_eq!(b.tls, KICK_1_POOL + 0x100);
}

/// Kick 2 (sections 4.1, 4.2, 5): the same FBD but for pre frame 0
/// (Intersect) and the frame shader DCDs, and the same render target but
/// for no clean-tile write and no clear colour: the four planes are drawn
/// back, not cleared, and written back to the same store.
#[test]
fn the_second_kick_reloads_and_keeps_the_four_samples() {
    let b = build(KICK_2_POOL, Kick::Second);
    /* w6:w7, the DCD array, is followed in the pre-frame test */
    let ours = b.words(b.fbd, 16);
    let p = compare("kick 2 FBD", &ours, &KICK_2_FBD, &[6, 14], &[TILE_26_2]);
    assert_eq!(p[1].0, b.tiler.unwrap(), "kick 2: our tiler pointer is the frame's tiler context");
    let rt = b.words(b.fbd + 128, 16);
    compare("kick 2 RT", &rt, &KICK_2_RT, &[], &[]);
    /* section 5's table: the words kick 1 and kick 2 differ in, ours too */
    let k1 = build(KICK_1_POOL, Kick::First);
    let (w1, w2) = (k1.words(k1.fbd, 16), b.words(b.fbd, 16));
    let differ: Vec<usize> = (0..16).filter(|&i| w1[i] != w2[i]).collect();
    assert_eq!(differ, [0, 6, 7, 14], "FBD words kick 1 and kick 2 differ in");
    let (r1, r2) = (k1.words(k1.fbd + 128, 16), rt);
    let differ: Vec<usize> = (0..16).filter(|&i| r1[i] != r2[i]).collect();
    assert_eq!(differ, [1, 12, 13, 14, 15], "RT words kick 1 and kick 2 differ in");
}

/// Kick 2's pre-frame-0 DCD and every block it names (section 4.6): Flags 0
/// 0x00080a03 (multisampled, once a sample, clean fragment writes, early
/// Z/S forced), sample mask 0xffff; a 4-sample 2D texture (w3 0x4000) whose
/// plane is the store (0x10000 bytes, slice stride 0x4000). Its pointers
/// into the pool are ours (the builder allocates the blocks in its own
/// order: frame.rs `preload_dcds`, `colour_preload_dcd`); each is followed.
/// The TLS (the frame's, pool +0x100), the program and the store are at
/// Mesa's addresses and equal.
#[test]
fn the_second_kick_s_pre_frame_draw_is_mesa_s() {
    let b = build(KICK_2_POOL, Kick::Second);
    check_pre_frame("kick 2", &b);
}

/// The pre-frame DCD array of a frame with a sample store's preload
/// against section 4.6.
fn check_pre_frame(name: &str, b: &Built) {
    let fbd = b.words(b.fbd, 16);
    let dcds = u64::from(fbd[6]) | u64::from(fbd[7]) << 32;
    let dcd = b.words(dcds, 32);
    /* pre frame 1 and the post frame: zero, their modes Never */
    assert_eq!(b.words(dcds + 128, 64), [0; 64], "{name}: DCDs 1 and 2");
    let p = compare(&format!("{name} DCD 0"), &dcd, &PRELOAD_DCD, &[10, 12, 24, 26], &[]);
    let (zsd, blend, table, spd) = (p[0], p[1], p[2], p[3]);
    /* the counts in the pointers' low bits: one blend, four tables */
    assert_eq!((blend.0 & 63, table.0 & 63), (1, 4), "{name}: blend and table counts");
    compare(&format!("{name} ZSD"), &b.words(zsd.0, 8), &ZSD, &[], &[]);
    compare(&format!("{name} blend"), &b.words(blend.0 & !63, 4), &BLEND, &[], &[]);
    let t = compare(&format!("{name} resources"), &b.words(table.0 & !63, 16), &RESOURCE_TABLE, &[0, 4, 8, 12], &[]);
    /* the resource words keep the count bit (0x0100 of w1) above the
     * address: compare() replaced the whole pointer, so check it here */
    let rt = b.words(table.0 & !63, 16);
    for k in 0..4 {
        assert_eq!(rt[4 * k + 1] >> 16, 0x0100, "{name}: resource {k} contains descriptors");
    }
    let addr = |k: usize| t[k].0 & ((1 << 56) - 1);
    compare(&format!("{name} attribute"), &b.words(addr(0), 8), &ATTRIBUTE, &[], &[]);
    let buf = compare(&format!("{name} buffer"), &b.words(addr(1), 8), &BUFFER, &[2], &[]);
    compare(&format!("{name} corners"), &b.words(buf[0].0, 16), &CORNERS, &[], &[]);
    compare(&format!("{name} sampler"), &b.words(addr(2), 8), &SAMPLER, &[], &[]);
    let plane = compare(&format!("{name} texture"), &b.words(addr(3), 8), &TEXTURE, &[4], &[]);
    compare(&format!("{name} plane"), &b.words(plane[0].0, 8), &PLANE, &[], &[]);
    /* the SPD names the program at Mesa's address (SPD w2:w3); the TLS
     * (DCD w28:w29) was compared as it is: the frame's, pool +0x100 on both
     * sides */
    compare(&format!("{name} SPD"), &b.words(spd.0, 8), &SPD, &[], &[]);
    eprintln!(
        "mali split kick vs Mesa ({name}): pool pointers followed: FBD w6 (DCDs), w14 (tiler); DCD w10 (ZSD), w12 \
         (blend), w24 (resources), w26 (SPD); resource entries' addresses; buffer w2; texture w4"
    );
}

/// Kick 2 against Mesa 26.2.1's on the drm-shim
/// (tools/mali/testdata/splitkick_kick2.expect.txt, the trace's decoded
/// fields; the shim prints no raw descriptor words): every field that says
/// the samples are reloaded reads as the shim's, as it does as the board's
/// (the tests above). The fields the shim's tiled layout moves (block
/// format, clump ordering, row strides) are not compared: ours is the
/// board's linear one.
#[test]
fn the_second_kick_s_pre_frame_draw_reads_as_the_shim_s() {
    let shim = include_str!("../../tools/mali/testdata/splitkick_kick2.expect.txt");
    let b = build(KICK_2_POOL, Kick::Second);
    let block = |gpu: u64, n: usize| b.bytes[(gpu - b.pool) as usize..][..n].to_vec();
    let ptr = |w: &[u32], i: usize| u64::from(w[i]) | u64::from(w[i + 1]) << 32;
    let fbd = b.words(b.fbd, 16);
    let dcd_words = b.words(ptr(&fbd, 6), 32);
    let draw = Draw::unpack(&block(ptr(&fbd, 6), Draw::LENGTH).try_into().unwrap()).unwrap();
    let table = ptr(&dcd_words, 24) & !63;
    let tex_at = ptr(&b.words(table, 16), 12) & ((1 << 56) - 1);
    let tex = Texture::unpack(&block(tex_at, Texture::LENGTH).try_into().unwrap()).unwrap();
    let plane = GenericPlane::unpack(&block(tex.surfaces, GenericPlane::LENGTH).try_into().unwrap()).unwrap();
    /* the union members a fragment SPD and an RGBA8 render target hold */
    let spd_bytes = block(ptr(&dcd_words, 26), ShaderProgram::LENGTH).try_into().unwrap();
    let spd = ShaderProgram::unpack_as(&spd_bytes, &["Fragment coverage bitmask type", "Preload.R48-R63"]).unwrap();
    let rt_bytes = block(b.fbd + 128, RgbRenderTarget::LENGTH).try_into().unwrap();
    let rt = RgbRenderTarget::unpack_as(&rt_bytes, &["Writeback Format"]).unwrap();
    let params = FramebufferParameters::unpack(&block(b.fbd, 64).try_into().unwrap()).unwrap();
    let (f0, f1) = (draw.flags_0, draw.flags_1);
    let early = |k: PixelKill| if k == PixelKill::FORCE_EARLY { "Force Early" } else { "not Force Early" };
    let fields: Vec<(&str, String)> = vec![
        ("Pre Frame 0", if params.pre_frame_0 == PrePostFrameShaderMode::INTERSECT { "Intersect" } else { "?" }.into()),
        ("Sample Count", params.sample_count.to_string()),
        ("Allow Forward Pixel To Kill", f0.allow_forward_pixel_to_kill.to_string()),
        ("Allow Forward Pixel To Be Killed", f0.allow_forward_pixel_to_be_killed.to_string()),
        ("Pixel Kill Operation", early(f0.pixel_kill_operation).into()),
        ("ZS Update Operation", early(f0.zs_update_operation).into()),
        ("Clean Fragment Write", f0.clean_fragment_write.to_string()),
        ("Evaluate per-sample", f0.evaluate_per_sample.to_string()),
        ("Multisample Enable", f0.multisample_enable.to_string()),
        ("Sample Mask", format!("{:#x}", f1.sample_mask)),
        ("Render Target Mask", format!("{:#x}", f1.render_target_mask)),
        ("Minimum Z", format!("{:.6}", draw.minimum_z)),
        ("Maximum Z", format!("{:.6}", draw.maximum_z)),
        ("Blend count", draw.blend_count.to_string()),
        ("Sample count", tex.sample_count.to_string()),
        ("Levels", tex.levels.to_string()),
        ("Size", format!("{:#x}", plane.size)),
        ("Slice stride", format!("{:#x}", plane.slice_stride)),
        ("R48-R63", format!("{:#x}", spd.preload.r48_r63)),
        ("Binary", format!("{:#x}", spd.binary)),
        ("Writeback MSAA", if rt.writeback_msaa == Msaa::LAYERED { "Layered" } else { "?" }.into()),
        ("Clean Tile Write Enable", rt.clean_tile_write_enable.to_string()),
        ("Surface Stride", format!("{:#x}", rt.writeback_buffer.surface_stride)),
    ];
    let differ: Vec<String> = fields
        .iter()
        .filter(|(label, ours)| !shim.contains(&format!(" {label}: {ours}\n")))
        .map(|(label, ours)| format!("{label}: {ours}"))
        .collect();
    assert!(differ.is_empty(), "fields of ours the shim's kick 2 does not print: {differ:?}");
}

/// Our resolve after the last kick: no draw and no tiler, the same
/// pre-frame draw as kick 2 (the four samples drawn back from the store),
/// run on every tile (pre frame 0 Always, as no draw marks a tile), and a
/// render target that averages the four samples into the single-sampled
/// target with every tile written. Not Mesa's: Mesa resolves in a third
/// frame with a draw of four TEX_FETCHes it averages in the shader into a
/// Writeback MSAA Single target (section 4, ctx-0.0002); ours lets the
/// writeback average, as the builder's 4x frames without a store already do
/// (frame_msaa_tests.rs, mesa-msrtt).
#[test]
fn the_resolve_averages_the_four_samples_into_the_target() {
    let b = build(KICK_2_POOL, Kick::Resolve);
    assert_eq!(b.tiler, None);
    let fbd = b.words(b.fbd, 16);
    /* kick 2's FBD but for: w0 pre frame 0 Always (1); w14:w15 no tiler */
    let mut want = KICK_2_FBD;
    want[0] = PrePostFrameShaderMode::ALWAYS.0;
    want[14] = 0;
    want[15] = 0;
    compare("resolve FBD", &fbd, &want, &[6], &[TILE_26_2]);
    /* kick 1's RT but for: w1 Writeback MSAA Average (1 at bits 12-13, not
     * Layered's 3); w8:w9 the target, not the store; w11 no surface stride;
     * w12-w15 no clear colour. Clean tiles are written (w1 bit 31) */
    let rt = b.words(b.fbd + 128, 16);
    let lo = PBUFFER as u32;
    let hi = (PBUFFER >> 32) as u32;
    let want = [0x04000000, 0x880a9299, 0, 0, 0, 0, 0, 0, lo, hi, 0x00000100, 0, 0, 0, 0, 0];
    compare("resolve RT", &rt, &want, &[], &[]);
    check_pre_frame("resolve", &b);
}

/// A 4x frame that starts from the target's image while the store holds
/// nothing of it (the backend's first frame of a surface that was not
/// cleared): its pre-frame draw is the one a frame without a store makes,
/// the target read back once into every sample (1x texture, no per-sample
/// bits; frame_preload_tests.rs pins it against Mesa), and its render
/// target writes the samples to the store as kick 1's does, but for the
/// clean-tile write and the clear colour (the frame is preloaded, kick 2's).
/// Every other byte of the two pools is equal.
#[test]
fn a_frame_before_the_store_holds_samples_preloads_the_target_and_keeps_them() {
    let (unkept, plain) = (build(KICK_2_POOL, Kick::Unkept), build(KICK_2_POOL, Kick::NoStore));
    assert_eq!((unkept.fbd, unkept.tls, unkept.tiler), (plain.fbd, plain.tls, plain.tiler));
    let rt_at = (unkept.fbd + 128 - unkept.pool) as usize;
    let differ: Vec<usize> = (0..unkept.bytes.len())
        .filter(|&i| unkept.bytes[i] != plain.bytes[i] && !(rt_at..rt_at + 64).contains(&i))
        .collect();
    assert!(differ.is_empty(), "pool bytes other than RT 0's differ: {differ:x?}");
    let fbd = unkept.words(unkept.fbd, 16);
    assert_eq!(fbd[0], PrePostFrameShaderMode::INTERSECT.0, "pre frame 0 Intersect");
    let dcd = unkept.words(u64::from(fbd[6]) | u64::from(fbd[7]) << 32, 2);
    assert_eq!(dcd[0] & (1 << 11 | 1 << 19), 0, "not multisampled, not per sample");
    compare("unkept RT", &unkept.words(unkept.fbd + 128, 16), &KICK_2_RT, &[], &[]);
}

/// A store the frame cannot keep its samples in is refused by name, and so
/// is a resolve with nothing to resolve (no preload, or a preload of the
/// target rather than of the store).
#[test]
fn a_sample_store_that_cannot_hold_the_samples_is_refused() {
    let (mut desc_m, mut ctx_m) = (vec![0u8; 0x4000], vec![0u8; 0x200]);
    let mut words = vec![0u64; 512];
    let mut ctx_a = Arena::new("context", HEAP_DESC, &mut ctx_m).unwrap();
    let heap = tiler_heap_descriptor(&mut ctx_a, 0x8000_00a0_0000, 2 << 20).unwrap();
    let ctx = Context {
        heap: heap.gpu,
        geometry_buffer: GEOMETRY,
        geometry_buffer_size: GEOMETRY_BUFFER_BYTES,
        sample_positions: SAMPLES,
        tiler_max_levels: 8,
    };
    let mut desc = Arena::new("desc", KICK_2_POOL, &mut desc_m).unwrap();
    let mut cs = Stream::new(&mut words, 92);
    let mut open = begin_frame(&mut desc, &mut cs).unwrap();
    let store = SampleStore { gpu: STORE, surface_stride: SURFACE_STRIDE, reload: true, resolve: false };
    let refused = Err(FrameError::Invalid(INVALID_SAMPLE_STORE));
    let preload = Some(FrameShader { binary: PRELOAD_BINARY, preload: 0x3000 });
    let cases = [
        (Target { samples: 1, ..TARGET }, store, None),
        (TARGET, SampleStore { gpu: STORE + 32, ..store }, None),
        (TARGET, SampleStore { surface_stride: SURFACE_STRIDE + 32, ..store }, None),
        (TARGET, SampleStore { surface_stride: SURFACE_STRIDE - 64, ..store }, None),
        (TARGET, SampleStore { resolve: true, ..store }, None),
        (TARGET, SampleStore { resolve: true, reload: false, ..store }, preload),
    ];
    for (k, (target, s, p)) in cases.into_iter().enumerate() {
        open.sample_store = Some(s);
        open.colour_preload = p;
        let r = end_frame(&open, &mut desc, &mut cs, &ctx, &target, [0.0; 4], None).map(|_| ());
        assert_eq!(r, refused, "case {k}");
    }
}

/// Section 4.6's preload program, the board's bytes as little-endian words
/// (GPU VA 0x7fffffffe180).
pub(super) const PRELOAD_PROGRAM: [u64; 14] = [
    0x0854_8004_1200_003d,
    0x6890_c000_c00c_0040,
    0x0090_c100_c00c_0041,
    0x0110_c200_0000_1fc0,
    0x00b4_c200_1042_cc7d,
    0x00bd_c200_00c0_c042,
    0x00bd_c200_0042_c0c0,
    0x0110_c318_0010_00c0,
    0x0091_c400_0000_0043,
    0x0091_c500_0000_00c0,
    0x4925_4036_53c0_0844,
    0x787f_4008_b23c_00f0,
    0x0110_f000_0000_00c0,
    0x002f_c110_0000_f1c0,
];

/// The words of the "words" array of a tools/mali/testdata `*.expect.json`
/// (one region).
fn expect_words(json: &str) -> Vec<u64> {
    let at = json.find("\"words\":").expect("a words array");
    let list = &json[at..];
    let list = &list[list.find('[').unwrap() + 1..list.find(']').unwrap()];
    list.split(',').map(|w| u64::from_str_radix(w.trim().trim_matches('"'), 16).unwrap()).collect()
}

/// The per-sample colour preload program the backend compiles at init
/// (`preload::colour_samples`, which the second kick's pre-frame draw
/// runs): Mesa 26.2.1's words on the drm-shim
/// (tools/mali/testdata/splitkick_preload.expect.json: `drm_shim_ref.sh run
/// <work> splitkick.vert splitkick.frag <out> --samples 4 --second-frame`,
/// the program at 0x7fffffffe180, the address the board's has), which are
/// the board's words too (section 4.6). It preloads r60 (the coverage) and
/// r61 (the sample id) in 32 work registers, as the shim's and the board's
/// SPD say (w1 0x3000, `32 Per Thread`).
#[test]
fn the_per_sample_preload_program_is_mesa_s() {
    let shim = expect_words(include_str!("../../tools/mali/testdata/splitkick_preload.expect.json"));
    assert_eq!(shim, PRELOAD_PROGRAM, "the drm-shim's program is the board's");
    let (mut ours, preload, wrc) = crate::shaderc::online::store::with_scratch(|s| {
        let mut out = vec![0u64; 64];
        let p = crate::shaderc::online::mali::preload::colour_samples(&mut out, s).expect("the preload compiles");
        (p.code.to_vec(), p.preload, p.work_reg_count)
    })
    .expect("the scratch arena is free");
    while ours.last() == Some(&0) {
        ours.pop();
    }
    assert_eq!(ours, PRELOAD_PROGRAM, "ours against Mesa's\n  ours {ours:016x?}\n  mesa {PRELOAD_PROGRAM:016x?}");
    assert_eq!((preload >> 48, wrc), (0x3000, 32), "preload r60 r61, 32 work registers");
}
