// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The frame builder's 2D texture against the board: the draw Mesa 25.0.7
//! made on the NanoPC-T6 for `texture2D(u_tex, v_uv)` over an 8x8 RGBA8
//! texture, nearest or linear, clamped or repeated (tools/mali/board-dump/
//! tex-2026-10-19.txt, cut from 0dce654a8:tools/mali/board-ref/2026-10-19-texture/
//! mesa), built again with the texture at the trace's own address and row
//! stride.
//!
//! What is compared, per mode: the fragment shader's sampler, its texture
//! descriptor and that descriptor's plane, entries 3 (samplers) and 4
//! (textures) of the fragment resource table, the fragment shader program
//! descriptor, and the draw flags RUN_IDVS reads (r56-r58). Every byte where
//! ours may differ from the dump is listed with its cause, the dump is
//! checked to hold what the cause says, and every other byte is equal.

use super::arena::Arena;
use super::cs::{Instr, Stream};
use super::frame::*;
use super::genxml::v10::*;
use super::trace;
use std::collections::BTreeMap;

const TEX: &str = include_str!("../../tools/mali/board-dump/tex-2026-10-19.txt");

/* the trace's addresses: the descriptor buffer, the render target, the
 * texture's texels ("Plane 0: Pointer") and the heap */
const DESC: u64 = 0x7fff_fffd_4000;
const TARGET: u64 = 0x7fff_fffe_4000;
const TEXELS: u64 = 0x7fff_fffe_a000;
const SAMPLES: u64 = 0x7fff_ffff_f000;
const HEAP_DESC: u64 = 0x7fff_ffff_b000;
const GEOMETRY: u64 = 0x7fff_fffe_b000;
/// The texture's row stride in the trace ("Row stride: 64"): Mesa's linear
/// layout aligns a row to 64 bytes (pan_mod.c:705-760).
const ROW_STRIDE: u32 = 64;

/// The registers each RUN_IDVS of a stream reads (every MOVE32 and MOVE48
/// before it, per register).
fn regs_at_idvs(words: &[u64]) -> Vec<BTreeMap<u32, u32>> {
    let mut r = BTreeMap::new();
    let mut out = Vec::new();
    for &w in words {
        match Instr::decode(w) {
            Ok(Instr::Move32(m)) => {
                r.insert(m.destination, m.immediate);
            }
            Ok(Instr::Move48(m)) => {
                r.insert(m.destination, m.immediate as u32);
                r.insert(m.destination + 1, (m.immediate >> 32) as u32);
            }
            Ok(Instr::RunIdvs(_)) => out.push(r.clone()),
            _ => {}
        }
    }
    out
}

fn meta(stage: ShaderStage) -> ShaderMeta<'static> {
    let fragment = stage == ShaderStage::FRAGMENT;
    ShaderMeta {
        no_psiz_offset: None,
        stage,
        register_allocation: ShaderRegisterAllocation::_32_PER_THREAD,
        /* the trace's fragment shader: r60 and r61 ("R48-R63: 0x3000") */
        preload: if fragment { 0x3000 } else { 0 },
        sysvals: &[],
        push: &[],
        varyings: 0,
        varying_bytes: 0,
        secondary: None,
        colour_outputs: u32::from(fragment),
        samples_texture: fragment,
        /* the trace's fragment shader samples with an implicit LOD:
         * "Requires helper threads: true" */
        requires_helper_threads: fragment,
        can_discard: false,
        writes_depth: false,
        writes_stencil: false,
        writes_coverage: false,
        writes_global: false,
        ubo: &[],
    }
}

/// tex.c's sampler state for a mode.
fn texture_of(mode: &str) -> SampledTexture {
    let linear = mode.starts_with("linear");
    let wrap = if mode.ends_with("repeat") { WrapMode::REPEAT } else { WrapMode::CLAMP_TO_EDGE };
    let mut level = [TextureLevel::default(); MAX_TEXTURE_LEVELS];
    level[0] = TextureLevel { gpu: TEXELS, row_stride: ROW_STRIDE };
    SampledTexture {
        width: 8,
        height: 8,
        level,
        levels: 1,
        faces: 1,
        format: Format::RGBA8_UNORM.0 << 12 | RgbComponentOrder::RGBA.0,
        clump: ClumpFormat::RAW32,
        block_bytes: 0,
        swizzle: SWIZZLE_RGBA,
        min_nearest: !linear,
        mag_nearest: !linear,
        mipmap: MipmapMode::NONE,
        wrap_s: wrap,
        wrap_t: wrap,
    }
}

struct Built {
    desc: Vec<u8>,
    spds: Vec<u8>,
    words: Vec<u64>,
    layout: Layout,
    fs_spd: u64,
}

const SPDS: u64 = 0x7fff_ffff_d000;

/// tex.c's frame: one draw of six vertices with the mode's texture, the
/// descriptor arena at the trace's address.
fn build(tex: SampledTexture) -> Built {
    build_draw(Some(tex), None, false)
}

/// The same frame with a texture for each stage that samples one, into an
/// FBO, or (`flip_y`) into the window surface, whose draws flip the
/// viewport and the front face as the GL core asks for a window.
fn build_draw(tex: Option<SampledTexture>, vs_tex: Option<SampledTexture>, flip_y: bool) -> Built {
    let (mut desc_m, mut code_m, mut spd_m, mut ctx_m) =
        (vec![0u8; 0x4000], vec![0u8; 0x800], vec![0u8; 0x100], vec![0u8; 0x200]);
    let mut words = vec![0u64; 512];
    let (layout, fs_spd);
    {
        let mut ctx_a = Arena::new("context", HEAP_DESC, &mut ctx_m).unwrap();
        let heap = tiler_heap_descriptor(&mut ctx_a, 0x8000_00a0_0000, 2 << 20).unwrap();
        let ctx = Context {
            heap: heap.gpu,
            geometry_buffer: GEOMETRY,
            geometry_buffer_size: GEOMETRY_BUFFER_BYTES,
            sample_positions: SAMPLES,
            tiler_max_levels: 8,
        };
        let mut desc = Arena::new("desc", DESC, &mut desc_m).unwrap();
        let mut code = Arena::new("code", 0x7fff_ffff_c000, &mut code_m).unwrap();
        let mut spds = Arena::new("spds", SPDS, &mut spd_m).unwrap();
        let bin = [0u8; 128];
        let vs_meta = ShaderMeta { samples_texture: vs_tex.is_some(), ..meta(ShaderStage::VERTEX) };
        let fs_meta = ShaderMeta { samples_texture: tex.is_some(), ..meta(ShaderStage::FRAGMENT) };
        let vs = upload_program(&mut code, &mut spds, &bin, vs_meta).unwrap();
        let fs = upload_program(&mut code, &mut spds, &bin, fs_meta).unwrap();
        fs_spd = fs.spd;
        let attributes = [VertexAttribute {
            buffer: VertexBuffer { gpu: 0x7fff_ffec_c000, size: 96 },
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
            vertex_count: 6,
            instance_count: 1,
            indices: None,
            attributes: &attributes,
            viewport: Viewport { x: 0, y: 0, width: 64, height: 64, near: 0.0, far: 1.0 },
            scissor: None,
            line_width: 1.0,
            flip_y,
            blend_color: [0.0; 4],
            vs_uniforms: &[],
            fs_uniforms: &[],
            zs: DepthStencilState::OFF,
            colour_mask: 0xf,
            front_cw: false,
            texture: tex,
            vs_texture: vs_tex,
            blend: None,
            polygon_offset: None,
            occlusion_query: None,
            alpha_to_coverage: false,
            blend_shader: None,
        };
        let target =
            Target { gpu: TARGET, width: 64, height: 64, row_stride: 256, format: TargetFormat::Rgba8888, samples: 1 };
        let mut cs = Stream::new(&mut words, 92);
        let mut open = begin_frame(&mut desc, &mut cs).unwrap();
        add_draw(&mut open, &mut desc, &mut cs, &ctx, &target, false, &draw).unwrap();
        layout = open.layout;
        end_frame(&open, &mut desc, &mut cs, &ctx, &target, [0.0, 0.0, 1.0, 1.0], None).unwrap();
        let n = cs.words().len();
        words.truncate(n);
    }
    Built { desc: desc_m, spds: spd_m, words, layout, fs_spd }
}

/// One place where our bytes may differ from the dump's.
struct Difference {
    block: &'static str,
    at: usize,
    dump: Vec<u8>,
    ours: Vec<u8>,
    cause: &'static str,
}

fn compare(name: &'static str, ours: &[u8], dump: &[u8], diffs: &[Difference]) -> Result<(), String> {
    let mut expected = dump.to_vec();
    for d in diffs.iter().filter(|d| d.block == name) {
        let span = d.at..d.at + d.dump.len();
        assert_eq!(dump[span.clone()], d.dump[..], "{name} +{:#x}: the dump is not what the cause says", d.at);
        expected[span].copy_from_slice(&d.ours);
    }
    if expected == ours {
        Ok(())
    } else {
        let at: Vec<usize> = (0..ours.len()).filter(|&i| expected[i] != ours[i]).collect();
        Err(format!("{name}: bytes {at:x?}\n  dump  {dump:02x?}\n  ours  {ours:02x?}"))
    }
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

fn check_mode(mode: &'static str) {
    let tex = texture_of(mode);
    let b = build(tex);
    let m = trace::memory_of(TEX, mode);
    let ours = |gpu: u64, len: usize| b.desc[(gpu - DESC) as usize..(gpu - DESC) as usize + len].to_vec();
    let mesa_regs = regs_at_idvs(&trace::cs_words(TEX, mode).iter().map(|w| w.word).collect::<Vec<_>>());
    let our_regs = regs_at_idvs(&b.words);
    assert_eq!((mesa_regs.len(), our_regs.len()), (1, 1), "one RUN_IDVS each");
    let reg64 = |r: &BTreeMap<u32, u32>, n: u32| u64::from(r[&n]) | u64::from(r[&(n + 1)]) << 32;
    let (mr, or) = (&mesa_regs[0], &our_regs[0]);

    /* Mesa's blocks, followed from the fragment SRT register (d4, the
     * table count in its low bits) and the SPD register (d20) */
    let srt = reg64(mr, 4) & !63;
    let mesa_table = trace::bytes(&m, srt, 7 * Resource::LENGTH);
    let entry = |t: &[u8], i: usize| Resource::unpack(t[16 * i..16 * i + 16].try_into().unwrap()).unwrap();
    let (m_sampler, m_texture) = (entry(&mesa_table, 3), entry(&mesa_table, 4));
    let m_tex_bytes = trace::bytes(&m, m_texture.address, Texture::LENGTH);
    let m_tex = Texture::unpack(m_tex_bytes[..].try_into().unwrap()).unwrap();
    let m_plane = trace::bytes(&m, m_tex.surfaces, GenericPlane::LENGTH);
    let m_spd = trace::bytes(&m, reg64(mr, 20), ShaderProgram::LENGTH);

    /* ours, from the builder's layout */
    let our_table = ours(b.layout.fs_resources.expect("a fragment resource table").gpu, 7 * Resource::LENGTH);
    let (o_sampler, o_texture) = (entry(&our_table, 3), entry(&our_table, 4));
    let o_tex_block = b.layout.fs_texture.expect("a texture descriptor");
    let o_plane_block = b.layout.fs_plane.expect("a plane");
    assert_eq!(o_texture.address, o_tex_block.gpu);
    let o_spd = b.spds[(b.fs_spd - SPDS) as usize..(b.fs_spd - SPDS) as usize + ShaderProgram::LENGTH].to_vec();

    let nearest = !mode.starts_with("linear");
    let mut diffs = vec![
        Difference {
            block: "Texture",
            at: 0x10,
            dump: m_tex.surfaces.to_le_bytes().to_vec(),
            ours: o_plane_block.gpu.to_le_bytes().to_vec(),
            cause: "the plane's address: Mesa keeps it in the sampler view's own buffer, ours is in the draw's",
        },
        Difference {
            block: "Plane",
            at: 0x18,
            dump: (ROW_STRIDE * 8).to_le_bytes().to_vec(),
            ours: vec![0; 4],
            cause: "slice stride: Mesa 25.0.7 writes the level's size; 26.2.1 writes 0 for a single-sampled 2D image \
                    (pan_texture.c:416-420)",
        },
        Difference {
            block: "Fragment SPD",
            at: 0x08,
            dump: m_spd[8..16].to_vec(),
            ours: o_spd[8..16].to_vec(),
            cause: "the binary's address",
        },
    ];
    if nearest {
        let s = trace::bytes(&m, m_sampler.address, Sampler::LENGTH);
        diffs.push(Difference {
            block: "Sampler",
            at: 0x02,
            dump: vec![s[2]],
            ours: vec![s[2] & !0x20],
            cause: "Round to nearest even: Mesa 25.0.7 keeps it for a nearest sampler; 26.2.1 clears it when \
                    minification and magnification are nearest (pan_cmdstream.c:243-245)",
        });
    }
    let mut failures = Vec::new();
    /* entries 3 and 4: one descriptor each */
    for (k, (mo, oo)) in [(3, (m_sampler, o_sampler)), (4, (m_texture, o_texture))] {
        if (mo.contains_descriptors, mo.size) != (oo.contains_descriptors, oo.size) {
            failures.push(format!("table entry {k}: Mesa {mo:?}, ours {oo:?}"));
        }
    }
    let blocks = [
        ("Sampler", ours(o_sampler.address, Sampler::LENGTH), trace::bytes(&m, m_sampler.address, Sampler::LENGTH)),
        ("Texture", ours(o_tex_block.gpu, Texture::LENGTH), m_tex_bytes.clone()),
        ("Plane", ours(o_plane_block.gpu, GenericPlane::LENGTH), m_plane.clone()),
        ("Fragment SPD", o_spd.clone(), m_spd.clone()),
    ];
    for (name, o, d) in &blocks {
        if let Err(e) = compare(name, o, d, &diffs) {
            failures.push(e);
        }
    }
    /* the plane names the trace's texels and row stride */
    assert_eq!((u64_at(&m_plane, 8), u32::from_le_bytes(m_plane[16..20].try_into().unwrap())), (TEXELS, ROW_STRIDE));
    /* the draw flags: the tiler flags (r56), DCD 0 and 1 (r57, r58) */
    for reg in [56, 57, 58] {
        if mr[&reg] != or[&reg] {
            failures.push(format!("r{reg}: Mesa {:#x}, ours {:#x}", mr[&reg], or[&reg]));
        }
    }
    eprintln!("mali texture draw vs the board ({mode}): {} differences named", diffs.len());
    for d in &diffs {
        eprintln!("  {} +{:#x}: {}", d.block, d.at, d.cause);
    }
    assert!(failures.is_empty(), "{mode}:\n{}", failures.join("\n"));
}

#[test]
fn the_nearest_clamped_texture_is_the_board_s() {
    check_mode("nearest_clamp");
}

#[test]
fn the_linear_clamped_texture_is_the_board_s() {
    check_mode("linear_clamp");
}

#[test]
fn the_nearest_repeated_texture_is_the_board_s() {
    check_mode("nearest_repeat");
}

#[test]
fn the_linear_repeated_texture_is_the_board_s() {
    check_mode("linear_repeat");
}

/// A fragment shader that samples and a draw without a texture (or the
/// other way round) is refused before any descriptor is written.
#[test]
fn a_texture_and_its_shader_must_agree() {
    let mut desc_m = vec![0u8; 0x4000];
    let (mut code_m, mut spd_m) = (vec![0u8; 0x800], vec![0u8; 0x100]);
    let mut code = Arena::new("code", 0x7fff_ffff_c000, &mut code_m).unwrap();
    let mut spds = Arena::new("spds", SPDS, &mut spd_m).unwrap();
    let bin = [0u8; 128];
    let vs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::VERTEX)).unwrap();
    let fs_tex = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::FRAGMENT)).unwrap();
    let fs_plain = upload_program(
        &mut code,
        &mut spds,
        &bin,
        ShaderMeta { samples_texture: false, ..meta(ShaderStage::FRAGMENT) },
    )
    .unwrap();
    let attributes = [VertexAttribute {
        buffer: VertexBuffer { gpu: 0x7fff_ffec_c000, size: 96 },
        offset: 0,
        stride: 16,
        components: 4,
        instance_divisor: 0,
    }];
    let ctx = Context {
        heap: HEAP_DESC,
        geometry_buffer: GEOMETRY,
        geometry_buffer_size: GEOMETRY_BUFFER_BYTES,
        sample_positions: SAMPLES,
        tiler_max_levels: 8,
    };
    let target =
        Target { gpu: TARGET, width: 64, height: 64, row_stride: 256, format: TargetFormat::Rgba8888, samples: 1 };
    let mut words = vec![0u64; 512];
    let mut desc = Arena::new("desc", DESC, &mut desc_m).unwrap();
    let mut cs = Stream::new(&mut words, 92);
    let mut open = begin_frame(&mut desc, &mut cs).unwrap();
    let draw = |fs, texture| DrawCall {
        sample_mask: 0xf,
        vs: &vs,
        fs,
        primitive: Topology::Triangles,
        first_vertex: 0,
        vertex_count: 6,
        instance_count: 1,
        indices: None,
        attributes: &attributes,
        viewport: Viewport { x: 0, y: 0, width: 64, height: 64, near: 0.0, far: 1.0 },
        scissor: None,
        line_width: 1.0,
        flip_y: false,
        blend_color: [0.0; 4],
        vs_uniforms: &[],
        fs_uniforms: &[],
        zs: DepthStencilState::OFF,
        colour_mask: 0xf,
        front_cw: false,
        texture,
        vs_texture: None,
        blend: None,
        polygon_offset: None,
        occlusion_query: None,
        alpha_to_coverage: false,
        blend_shader: None,
    };
    let with_level0 = |gpu: u64, row_stride: u32| {
        let mut t = texture_of("nearest_clamp");
        t.level[0] = TextureLevel { gpu, row_stride };
        t
    };
    let (misaligned, narrow) = (with_level0(TEXELS + 32, ROW_STRIDE), with_level0(TEXELS, 28));
    for (d, want) in [
        (draw(&fs_tex, None), FrameError::Invalid(INVALID_TEXTURE)),
        (draw(&fs_plain, Some(texture_of("nearest_clamp"))), FrameError::Invalid(INVALID_TEXTURE)),
        (draw(&fs_tex, Some(misaligned)), FrameError::Invalid(INVALID_TEXTURE_ALIGN)),
        (draw(&fs_tex, Some(narrow)), FrameError::NotBuilt(NOT_BUILT_TEXTURE_SIZE)),
    ] {
        let got = add_draw(&mut open, &mut desc, &mut cs, &ctx, &target, false, &d).map(|_| ());
        assert_eq!(got.map_err(|e| format!("{e}")), Err(format!("{want}")));
    }
    assert_eq!(open.draws, 0, "nothing was added");
}

const MIP: &str = include_str!("../../tools/mali/board-dump/mip-2026-10-19.txt");

/// mip.c's draw (tools/mali/board-dump/mip-2026-10-19.txt): a 32x32 chain of
/// six levels, built with the levels where Mesa put them (each plane's
/// pointer and row stride read from the dump: the descriptors are pinned
/// here, the GL core's layout by the backend's tests and the board), and the
/// mode's filters. The sampler, the texture descriptor and its six planes
/// are the dump's but for the named causes.
fn check_mip(mode: &'static str) {
    let m = trace::memory_of(MIP, mode);
    let mesa_regs = regs_at_idvs(&trace::cs_words(MIP, mode).iter().map(|w| w.word).collect::<Vec<_>>());
    assert_eq!(mesa_regs.len(), 1, "one RUN_IDVS");
    let reg64 = |r: &BTreeMap<u32, u32>, n: u32| u64::from(r[&n]) | u64::from(r[&(n + 1)]) << 32;
    let mr = &mesa_regs[0];
    let mesa_table = trace::bytes(&m, reg64(mr, 4) & !63, 7 * Resource::LENGTH);
    let entry = |t: &[u8], i: usize| Resource::unpack(t[16 * i..16 * i + 16].try_into().unwrap()).unwrap();
    let (m_sampler, m_texture) = (entry(&mesa_table, 3), entry(&mesa_table, 4));
    let m_tex_bytes = trace::bytes(&m, m_texture.address, Texture::LENGTH);
    let m_tex = Texture::unpack(m_tex_bytes[..].try_into().unwrap()).unwrap();
    assert_eq!((m_tex.width, m_tex.height, m_tex.levels), (32, 32, 6));
    let m_planes = trace::bytes(&m, m_tex.surfaces, 6 * GenericPlane::LENGTH);
    let mut level = [TextureLevel::default(); MAX_TEXTURE_LEVELS];
    for (l, lv) in level.iter_mut().enumerate().take(6) {
        let p = GenericPlane::unpack(m_planes[32 * l..32 * l + 32].try_into().unwrap()).unwrap();
        *lv = TextureLevel { gpu: p.pointer, row_stride: p.row_stride };
    }
    let (min_nearest, mag_nearest, mipmap) = match mode {
        "nearest" => (true, true, MipmapMode::NEAREST),
        "linear_nearest" => (false, false, MipmapMode::NEAREST),
        _ => (false, false, MipmapMode::TRILINEAR),
    };
    let tex = SampledTexture {
        width: 32,
        height: 32,
        level,
        levels: 6,
        faces: 1,
        format: Format::RGBA8_UNORM.0 << 12 | RgbComponentOrder::RGBA.0,
        clump: ClumpFormat::RAW32,
        block_bytes: 0,
        swizzle: SWIZZLE_RGBA,
        min_nearest,
        mag_nearest,
        mipmap,
        wrap_s: WrapMode::REPEAT,
        wrap_t: WrapMode::REPEAT,
    };
    let b = build(tex);
    let ours = |gpu: u64, len: usize| b.desc[(gpu - DESC) as usize..(gpu - DESC) as usize + len].to_vec();
    let our_table = ours(b.layout.fs_resources.expect("a fragment resource table").gpu, 7 * Resource::LENGTH);
    let (o_sampler, o_texture) = (entry(&our_table, 3), entry(&our_table, 4));
    let o_planes = b.layout.fs_plane.expect("the planes");
    assert_eq!(o_planes.len, 6 * GenericPlane::LENGTH, "a plane a level");
    let mut diffs = vec![Difference {
        block: "Texture",
        at: 0x10,
        dump: m_tex.surfaces.to_le_bytes().to_vec(),
        ours: o_planes.gpu.to_le_bytes().to_vec(),
        cause: "the planes' address: Mesa keeps them in the sampler view's own buffer, ours are in the draw's",
    }];
    let slice = u32::from_le_bytes(m_planes[0x18..0x1c].try_into().unwrap());
    for l in 0..6 {
        diffs.push(Difference {
            block: "Planes",
            at: 32 * l + 0x18,
            dump: slice.to_le_bytes().to_vec(),
            ours: vec![0; 4],
            cause: "slice stride: Mesa 25.0.7 writes the chain's size into every level; 26.2.1 writes 0 for a \\
                    single-sampled 2D image (pan_texture.c:416-420)",
        });
    }
    if mode == "nearest" {
        let s = trace::bytes(&m, m_sampler.address, Sampler::LENGTH);
        diffs.push(Difference {
            block: "Sampler",
            at: 0x02,
            dump: vec![s[2]],
            ours: vec![s[2] & !0x20],
            cause: "Round to nearest even: Mesa 25.0.7 keeps it for a nearest sampler; 26.2.1 clears it when \\
                    minification and magnification are nearest (pan_cmdstream.c:243-245)",
        });
    }
    let mut failures = Vec::new();
    for (k, (mo, oo)) in [(3, (m_sampler, o_sampler)), (4, (m_texture, o_texture))] {
        if (mo.contains_descriptors, mo.size) != (oo.contains_descriptors, oo.size) {
            failures.push(format!("table entry {k}: Mesa {mo:?}, ours {oo:?}"));
        }
    }
    let blocks = [
        ("Sampler", ours(o_sampler.address, Sampler::LENGTH), trace::bytes(&m, m_sampler.address, Sampler::LENGTH)),
        ("Texture", ours(o_texture.address, Texture::LENGTH), m_tex_bytes.clone()),
        ("Planes", ours(o_planes.gpu, 6 * GenericPlane::LENGTH), m_planes.clone()),
    ];
    for (name, o, d) in &blocks {
        if let Err(e) = compare(name, o, d, &diffs) {
            failures.push(e);
        }
    }
    eprintln!("mali mip chain vs the board ({mode}): {} differences named", diffs.len());
    assert!(failures.is_empty(), "{mode}:\n{}", failures.join("\n"));
}

#[test]
fn a_nearest_mipmapped_chain_is_the_board_s() {
    check_mip("nearest");
}

#[test]
fn a_linear_mipmap_nearest_chain_is_the_board_s() {
    check_mip("linear_nearest");
}

#[test]
fn a_trilinear_chain_is_the_board_s() {
    check_mip("trilinear");
}

const FMT: &str = include_str!("../../tools/mali/board-dump/fmt-2026-10-19.txt");

/// fmt.c's draw (tools/mali/board-dump/fmt-2026-10-19.txt): one texture of
/// a texel format, built with its level where Mesa put it (the plane's
/// pointer and row stride read from the dump) and the format the backend
/// gives the GL core's storage of it (backend.rs `texel_format`): the
/// sampler, the texture descriptor and its plane are the dump's but for
/// the named causes.
fn check_format(mode: &'static str) {
    let m = trace::memory_of(FMT, mode);
    let mesa_regs = regs_at_idvs(&trace::cs_words(FMT, mode).iter().map(|w| w.word).collect::<Vec<_>>());
    assert_eq!(mesa_regs.len(), 1, "one RUN_IDVS");
    let reg64 = |r: &BTreeMap<u32, u32>, n: u32| u64::from(r[&n]) | u64::from(r[&(n + 1)]) << 32;
    let mesa_table = trace::bytes(&m, reg64(&mesa_regs[0], 4) & !63, 7 * Resource::LENGTH);
    let entry = |t: &[u8], i: usize| Resource::unpack(t[16 * i..16 * i + 16].try_into().unwrap()).unwrap();
    let (m_sampler, m_texture) = (entry(&mesa_table, 3), entry(&mesa_table, 4));
    let m_tex_bytes = trace::bytes(&m, m_texture.address, Texture::LENGTH);
    let m_tex = Texture::unpack(m_tex_bytes[..].try_into().unwrap()).unwrap();
    let m_plane = trace::bytes(&m, m_tex.surfaces, GenericPlane::LENGTH);
    let p = GenericPlane::unpack(m_plane[..].try_into().unwrap()).unwrap();
    let rgba8 = |o: RgbComponentOrder| Format::RGBA8_UNORM.0 << 12 | o.0;
    let (w, h, format, clump, block_bytes, swizzle) = match mode {
        "etc1" => (16, 8, rgba8(RgbComponentOrder::RGB1), ClumpFormat::ETC2_RGB8, 8, SWIZZLE_RGB1),
        "etc2rgba" => (8, 8, rgba8(RgbComponentOrder::RGBA), ClumpFormat::ETC2_RGBA8, 16, SWIZZLE_RGBA),
        _ => (4, 4, rgba8(RgbComponentOrder::RGBA) | 1 << 20, ClumpFormat::RAW32, 0, SWIZZLE_RGBA),
    };
    let mut level = [TextureLevel::default(); MAX_TEXTURE_LEVELS];
    level[0] = TextureLevel { gpu: p.pointer, row_stride: p.row_stride };
    let tex = SampledTexture {
        width: w,
        height: h,
        level,
        levels: 1,
        faces: 1,
        format,
        clump,
        block_bytes,
        swizzle,
        min_nearest: true,
        mag_nearest: true,
        mipmap: MipmapMode::NONE,
        wrap_s: WrapMode::CLAMP_TO_EDGE,
        wrap_t: WrapMode::CLAMP_TO_EDGE,
    };
    let b = build(tex);
    let ours = |gpu: u64, len: usize| b.desc[(gpu - DESC) as usize..(gpu - DESC) as usize + len].to_vec();
    let our_table = ours(b.layout.fs_resources.expect("a fragment resource table").gpu, 7 * Resource::LENGTH);
    let (o_sampler, o_texture) = (entry(&our_table, 3), entry(&our_table, 4));
    let o_plane = b.layout.fs_plane.expect("the plane");
    let s = trace::bytes(&m, m_sampler.address, Sampler::LENGTH);
    let diffs = [
        Difference {
            block: "Texture",
            at: 0x10,
            dump: m_tex.surfaces.to_le_bytes().to_vec(),
            ours: o_plane.gpu.to_le_bytes().to_vec(),
            cause: "the plane's address: Mesa keeps it in the sampler view's own buffer, ours is in the draw's",
        },
        Difference {
            block: "Plane",
            at: 0x18,
            dump: m_plane[0x18..0x1c].to_vec(),
            ours: vec![0; 4],
            cause: "slice stride: Mesa 25.0.7 writes the level's size; 26.2.1 writes 0 for a single-sampled 2D image \\
                    (pan_texture.c:416-420)",
        },
        Difference {
            block: "Sampler",
            at: 0x02,
            dump: vec![s[2]],
            ours: vec![s[2] & !0x20],
            cause: "Round to nearest even: Mesa 25.0.7 keeps it for a nearest sampler; 26.2.1 clears it when \\
                    minification and magnification are nearest (pan_cmdstream.c:243-245)",
        },
    ];
    let mut failures = Vec::new();
    for (k, (mo, oo)) in [(3, (m_sampler, o_sampler)), (4, (m_texture, o_texture))] {
        if (mo.contains_descriptors, mo.size) != (oo.contains_descriptors, oo.size) {
            failures.push(format!("table entry {k}: Mesa {mo:?}, ours {oo:?}"));
        }
    }
    let blocks = [
        ("Sampler", ours(o_sampler.address, Sampler::LENGTH), s.clone()),
        ("Texture", ours(o_texture.address, Texture::LENGTH), m_tex_bytes.clone()),
        ("Plane", ours(o_plane.gpu, GenericPlane::LENGTH), m_plane.clone()),
    ];
    for (name, o, d) in &blocks {
        if let Err(e) = compare(name, o, d, &diffs) {
            failures.push(e);
        }
    }
    eprintln!("mali texel format vs the board ({mode}): {} differences named", diffs.len());
    assert!(failures.is_empty(), "{mode}:\n{}", failures.join("\n"));
}

#[test]
fn an_etc1_texture_is_the_board_s() {
    check_format("etc1");
}

#[test]
fn an_etc2_rgba8_texture_is_the_board_s() {
    check_format("etc2rgba");
}

#[test]
fn an_srgb8_alpha8_texture_is_the_board_s() {
    check_format("srgb");
}

const VTX: &str = include_str!("../../tools/mali/board-dump/vtxgrid-2026-10-20.txt");

/// gl_test case 197's draw (tools/mali/board-dump/vtxgrid-2026-10-20.txt):
/// a 16x16 GL_RGB chain of five levels, NEAREST_MIPMAP_NEAREST and clamped,
/// sampled by the vertex shader (`texture2DLod` in vtxgrid_vs.vert), built
/// with the levels where Mesa put them and the storage the backend gives
/// the GL core's GL_RGB (RGBA8 read through the RGB1 swizzle,
/// backend.rs `sampled_texture`). Entries 3 (samplers) and 4 (textures) of
/// the vertex resource table, which both IDVS programs read, the sampler,
/// the texture descriptor and its five planes are the dump's but for the
/// named causes.
#[test]
fn a_vertex_shader_s_texture_is_the_board_s() {
    let m = trace::memory_of(VTX, "vtxgrid");
    let mesa_regs = regs_at_idvs(&trace::cs_words(VTX, "vtxgrid").iter().map(|w| w.word).collect::<Vec<_>>());
    assert_eq!(mesa_regs.len(), 1, "one RUN_IDVS");
    let reg64 = |r: &BTreeMap<u32, u32>, n: u32| u64::from(r[&n]) | u64::from(r[&(n + 1)]) << 32;
    /* the vertex SRT (d0, the table count in its low bits) */
    let mesa_table = trace::bytes(&m, reg64(&mesa_regs[0], 0) & !63, 7 * Resource::LENGTH);
    let entry = |t: &[u8], i: usize| Resource::unpack(t[16 * i..16 * i + 16].try_into().unwrap()).unwrap();
    let (m_sampler, m_texture) = (entry(&mesa_table, 3), entry(&mesa_table, 4));
    let m_tex_bytes = trace::bytes(&m, m_texture.address, Texture::LENGTH);
    let m_tex = Texture::unpack(m_tex_bytes[..].try_into().unwrap()).unwrap();
    assert_eq!((m_tex.width, m_tex.height, m_tex.levels), (16, 16, 5));
    let m_planes = trace::bytes(&m, m_tex.surfaces, 5 * GenericPlane::LENGTH);
    let mut level = [TextureLevel::default(); MAX_TEXTURE_LEVELS];
    for (l, lv) in level.iter_mut().enumerate().take(5) {
        let p = GenericPlane::unpack(m_planes[32 * l..32 * l + 32].try_into().unwrap()).unwrap();
        *lv = TextureLevel { gpu: p.pointer, row_stride: p.row_stride };
    }
    let tex = SampledTexture {
        width: 16,
        height: 16,
        level,
        levels: 5,
        faces: 1,
        format: Format::RGBA8_UNORM.0 << 12 | RgbComponentOrder::RGBA.0,
        clump: ClumpFormat::RAW32,
        block_bytes: 0,
        swizzle: SWIZZLE_RGB1,
        min_nearest: true,
        mag_nearest: true,
        mipmap: MipmapMode::NEAREST,
        wrap_s: WrapMode::CLAMP_TO_EDGE,
        wrap_t: WrapMode::CLAMP_TO_EDGE,
    };
    let b = build_draw(None, Some(tex), false);
    let ours = |gpu: u64, len: usize| b.desc[(gpu - DESC) as usize..(gpu - DESC) as usize + len].to_vec();
    let our_table = ours(b.layout.vs_resources.expect("a vertex resource table").gpu, 7 * Resource::LENGTH);
    let (o_sampler, o_texture) = (entry(&our_table, 3), entry(&our_table, 4));
    let o_planes = b.layout.vs_plane.expect("the planes");
    assert_eq!(o_texture.address, b.layout.vs_texture.expect("a texture descriptor").gpu);
    assert_eq!(o_planes.len, 5 * GenericPlane::LENGTH, "a plane a level");
    assert!(b.layout.fs_texture.is_none(), "the fragment shader samples nothing here");
    let s = trace::bytes(&m, m_sampler.address, Sampler::LENGTH);
    let mut diffs = vec![
        Difference {
            block: "Texture",
            at: 0x10,
            dump: m_tex.surfaces.to_le_bytes().to_vec(),
            ours: o_planes.gpu.to_le_bytes().to_vec(),
            cause: "the planes' address: Mesa keeps them in the sampler view's own buffer, ours are in the draw's",
        },
        Difference {
            block: "Texture",
            at: 0x01,
            dump: vec![m_tex_bytes[1]],
            ours: vec![m_tex_bytes[1] & !0x40],
            cause: "the format's component order: Mesa's GL_RGB view is RGBA8 UNORM in the RGB1 order (its \
                    R8G8B8X8 storage), ours RGBA8 UNORM in the RGBA order; both read alpha as 1 through the \
                    swizzle RGB1 (2696), equal in both",
        },
        Difference {
            block: "Sampler",
            at: 0x02,
            dump: vec![s[2]],
            ours: vec![s[2] & !0x20],
            cause: "Round to nearest even: Mesa 25.0.7 keeps it for a nearest sampler; 26.2.1 clears it when \
                    minification and magnification are nearest (pan_cmdstream.c:243-245)",
        },
    ];
    /* the dump's order and swizzle are what the cause says */
    assert_eq!((m_tex.format & 0xff, m_tex.swizzle), (RgbComponentOrder::RGB1.0, SWIZZLE_RGB1));
    let slice = u32::from_le_bytes(m_planes[0x18..0x1c].try_into().unwrap());
    for l in 0..5 {
        diffs.push(Difference {
            block: "Planes",
            at: 32 * l + 0x18,
            dump: slice.to_le_bytes().to_vec(),
            ours: vec![0; 4],
            cause: "slice stride: Mesa 25.0.7 writes the chain's size into every level; 26.2.1 writes 0 for a \
                    single-sampled 2D image (pan_texture.c:416-420)",
        });
    }
    let mut failures = Vec::new();
    for (k, (mo, oo)) in [(3, (m_sampler, o_sampler)), (4, (m_texture, o_texture))] {
        if (mo.contains_descriptors, mo.size) != (oo.contains_descriptors, oo.size) {
            failures.push(format!("table entry {k}: Mesa {mo:?}, ours {oo:?}"));
        }
    }
    let blocks = [
        ("Sampler", ours(o_sampler.address, Sampler::LENGTH), s.clone()),
        ("Texture", ours(o_texture.address, Texture::LENGTH), m_tex_bytes.clone()),
        ("Planes", ours(o_planes.gpu, 5 * GenericPlane::LENGTH), m_planes.clone()),
    ];
    for (name, o, d) in &blocks {
        if let Err(e) = compare(name, o, d, &diffs) {
            failures.push(e);
        }
    }
    eprintln!("mali vertex texture vs the board (vtxgrid): {} differences named", diffs.len());
    for d in &diffs {
        eprintln!("  {} +{:#x}: {}", d.block, d.at, d.cause);
    }
    assert!(failures.is_empty(), "vtxgrid:\n{}", failures.join("\n"));
}

const CUBE: &str = include_str!("../../tools/mali/board-dump/cube-2026-10-20.txt");

/// gl_test case 113's first draw (tools/mali/board-dump/cube-2026-10-20.txt,
/// docs/notes/mali-cube-textures.md): a 16x16 RGBA8 cube map of five
/// levels, NEAREST_MIPMAP_NEAREST and clamped, sampled by the vertex shader
/// (`textureCubeLod` in cubemap.vert), built with face 0's levels where Mesa
/// put them. Entries 3 (samplers) and 4 (textures) of the vertex resource
/// table, the sampler, the texture descriptor (`Dimension: Cube`, one
/// face's extent, `Levels: 5`, `Array size: 1`) and its 30 planes in
/// face-major order are the dump's but for the named causes: the planes'
/// address; the slice stride, 1984 in 25.0.7, 0 in 26.2.1 (the shim's
/// cube_vs trace, every plane) and ours; the sampler's round-to-nearest-even
/// bit; and the pointers of faces 1 to 5, since Mesa's storage is
/// layer-major (one face's whole chain, 1984 bytes, then the next face)
/// where the GL core's is level-major (a level's six faces stacked on one
/// 64-byte pitch, `chain_layout_npot`): the dump's face pointer is `face *
/// 1984` past face 0's, ours `face * rows * 64`. Every other byte is equal:
/// the plane order, each plane's size, row stride, extent and format, the
/// texture words.
#[test]
fn a_cube_map_s_texture_is_the_board_s() {
    let m = trace::memory_of(CUBE, "cubemap");
    let mesa_regs = regs_at_idvs(&trace::cs_words(CUBE, "cubemap").iter().map(|w| w.word).collect::<Vec<_>>());
    assert_eq!(mesa_regs.len(), 1, "one RUN_IDVS");
    let reg64 = |r: &BTreeMap<u32, u32>, n: u32| u64::from(r[&n]) | u64::from(r[&(n + 1)]) << 32;
    /* the vertex SRT (d0, the table count in its low bits) */
    let mesa_table = trace::bytes(&m, reg64(&mesa_regs[0], 0) & !63, 7 * Resource::LENGTH);
    let entry = |t: &[u8], i: usize| Resource::unpack(t[16 * i..16 * i + 16].try_into().unwrap()).unwrap();
    let (m_sampler, m_texture) = (entry(&mesa_table, 3), entry(&mesa_table, 4));
    let m_tex_bytes = trace::bytes(&m, m_texture.address, Texture::LENGTH);
    let m_tex = Texture::unpack(m_tex_bytes[..].try_into().unwrap()).unwrap();
    assert_eq!(m_tex.dimension, TextureDimension::CUBE);
    assert_eq!((m_tex.width, m_tex.height, m_tex.levels, m_tex.array_size), (16, 16, 5, 1));
    const PLANES: usize = 30;
    let m_planes = trace::bytes(&m, m_tex.surfaces, PLANES * GenericPlane::LENGTH);
    let plane_at = |k: usize| GenericPlane::unpack(m_planes[32 * k..32 * k + 32].try_into().unwrap()).unwrap();
    let mut level = [TextureLevel::default(); MAX_TEXTURE_LEVELS];
    for (l, lv) in level.iter_mut().enumerate().take(5) {
        let p = plane_at(l);
        *lv = TextureLevel { gpu: p.pointer, row_stride: p.row_stride };
    }
    /* the dump's order is face-major: plane 5 is face 1, level 0, one
     * face's chain (1024 + 512 + 256 + 128 + 64 = 1984 bytes) past face 0 */
    const FACE_STRIDE: u64 = 1984;
    assert_eq!(plane_at(5).pointer, plane_at(0).pointer + FACE_STRIDE);
    assert_eq!((plane_at(5).width, plane_at(5).height, plane_at(5).size), (16, 16, 1024));
    let tex = SampledTexture {
        width: 16,
        height: 16,
        level,
        levels: 5,
        faces: 6,
        format: Format::RGBA8_UNORM.0 << 12 | RgbComponentOrder::RGBA.0,
        clump: ClumpFormat::RAW32,
        block_bytes: 0,
        swizzle: SWIZZLE_RGBA,
        min_nearest: true,
        mag_nearest: true,
        mipmap: MipmapMode::NEAREST,
        wrap_s: WrapMode::CLAMP_TO_EDGE,
        wrap_t: WrapMode::CLAMP_TO_EDGE,
    };
    let b = build_draw(None, Some(tex), false);
    let ours = |gpu: u64, len: usize| b.desc[(gpu - DESC) as usize..(gpu - DESC) as usize + len].to_vec();
    let our_table = ours(b.layout.vs_resources.expect("a vertex resource table").gpu, 7 * Resource::LENGTH);
    let (o_sampler, o_texture) = (entry(&our_table, 3), entry(&our_table, 4));
    let o_planes = b.layout.vs_plane.expect("the planes");
    assert_eq!(o_texture.address, b.layout.vs_texture.expect("a texture descriptor").gpu);
    assert_eq!(o_planes.len, PLANES * GenericPlane::LENGTH, "a plane a face and level");
    let s = trace::bytes(&m, m_sampler.address, Sampler::LENGTH);
    let mut diffs = vec![
        Difference {
            block: "Texture",
            at: 0x10,
            dump: m_tex.surfaces.to_le_bytes().to_vec(),
            ours: o_planes.gpu.to_le_bytes().to_vec(),
            cause: "the planes' address: Mesa keeps them in the sampler view's own buffer, ours are in the draw's",
        },
        Difference {
            block: "Sampler",
            at: 0x02,
            dump: vec![s[2]],
            ours: vec![s[2] & !0x20],
            cause: "Round to nearest even: Mesa 25.0.7 keeps it for a nearest sampler; 26.2.1 clears it when \
                    minification and magnification are nearest (pan_cmdstream.c:243-245)",
        },
    ];
    let slice = u32::from_le_bytes(m_planes[0x18..0x1c].try_into().unwrap());
    assert_eq!(slice, FACE_STRIDE as u32, "25.0.7's slice stride is the layer stride");
    for k in 0..PLANES {
        diffs.push(Difference {
            block: "Planes",
            at: 32 * k + 0x18,
            dump: slice.to_le_bytes().to_vec(),
            ours: vec![0; 4],
            cause: "slice stride: Mesa 25.0.7 writes the layer stride (one face's chain) into every plane; 26.2.1 \
                    writes 0 for a single-sampled 2D or cube image (pan_texture.c:416-421; the shim's cube_vs \
                    trace, 0 in all 30 planes)",
        });
        let (face, l) = (k / 5, k % 5);
        if face == 0 {
            continue;
        }
        let rows = u64::from(tex.extent(l as u32).1);
        diffs.push(Difference {
            block: "Planes",
            at: 32 * k + 0x08,
            dump: (level[l].gpu + face as u64 * FACE_STRIDE).to_le_bytes().to_vec(),
            ours: (level[l].gpu + face as u64 * rows * 64).to_le_bytes().to_vec(),
            cause: "a face's pointer: Mesa's storage is layer-major (face f at f * 1984, one face's chain), the GL \
                    core's level-major (face f at f * rows of the level on the 64-byte pitch); each plane names \
                    its own pointer, so the unit reads either (docs/notes/mali-cube-textures.md (c))",
        });
    }
    let mut failures = Vec::new();
    for (k, (mo, oo)) in [(3, (m_sampler, o_sampler)), (4, (m_texture, o_texture))] {
        if (mo.contains_descriptors, mo.size) != (oo.contains_descriptors, oo.size) {
            failures.push(format!("table entry {k}: Mesa {mo:?}, ours {oo:?}"));
        }
    }
    let blocks = [
        ("Sampler", ours(o_sampler.address, Sampler::LENGTH), s.clone()),
        ("Texture", ours(o_texture.address, Texture::LENGTH), m_tex_bytes.clone()),
        ("Planes", ours(o_planes.gpu, PLANES * GenericPlane::LENGTH), m_planes.clone()),
    ];
    for (name, o, d) in &blocks {
        if let Err(e) = compare(name, o, d, &diffs) {
            failures.push(e);
        }
    }
    /* the texture words as the note reads them (docs/notes/mali-cube-textures.md (a)) */
    let o_tex = ours(o_texture.address, Texture::LENGTH);
    assert_eq!(u32::from_le_bytes(o_tex[0..4].try_into().unwrap()), 0x2ec0_0002, "Type 2, Dimension Cube, RGBA8");
    assert_eq!(u32::from_le_bytes(o_tex[8..12].try_into().unwrap()), 0x0004_0688, "swizzle RGBA, 5 levels");
    assert_eq!(u32::from_le_bytes(o_tex[12..16].try_into().unwrap()), 0x0400_0000, "maximum LOD 4.0");
    assert_eq!(&o_tex[24..32], &[0u8; 8], "array size 1, depth 1");
    eprintln!("mali cube map vs the board (cubemap): {} differences named", diffs.len());
    assert!(failures.is_empty(), "cubemap:\n{}", failures.join("\n"));
}

/// A cube map's planes and pointers: face `f` of level `l` at `f * rows *
/// row_stride` past face 0's (the GL core's stacked faces), 30 planes in
/// face-major order, one face's extent in the descriptor; a 2D texture's
/// `faces` is 1 and its planes are the levels alone.
#[test]
fn a_cube_map_s_planes_are_face_major() {
    let mut level = [TextureLevel::default(); MAX_TEXTURE_LEVELS];
    /* a 16x16 cube as the GL core lays it out: level l's six faces of
     * (16 >> l) rows each on a 64-byte pitch, every level on 64 bytes */
    let offsets = [0u64, 6144, 9216, 10752, 11520];
    for (l, lv) in level.iter_mut().enumerate().take(5) {
        *lv = TextureLevel { gpu: TEXELS + offsets[l], row_stride: 64 };
    }
    let tex = SampledTexture { width: 16, height: 16, faces: 6, level, levels: 5, ..texture_of("nearest_clamp") };
    assert_eq!(tex.texture(0).dimension, TextureDimension::CUBE);
    for f in 0..6u32 {
        for l in 0..5u32 {
            let rows = 16u64 >> l;
            let p = tex.plane(f, l);
            assert_eq!(p.pointer, TEXELS + offsets[l as usize] + u64::from(f) * rows * 64, "face {f} level {l}");
            assert_eq!((p.size, p.row_stride, p.width, p.height), (64 * rows as u32, 64, 16 >> l, 16 >> l));
            assert_eq!(p.slice_stride, 0);
        }
    }
    let b = build_draw(None, Some(tex), false);
    let planes = b.layout.vs_plane.expect("the planes");
    assert_eq!(planes.len, 30 * GenericPlane::LENGTH);
    let at = |k: usize| {
        let o = (planes.gpu - DESC) as usize + 32 * k;
        GenericPlane::unpack(b.desc[o..o + 32].try_into().unwrap()).unwrap()
    };
    assert_eq!(at(5).pointer, TEXELS + 16 * 64, "plane 5 is face 1, level 0");
    assert_eq!(at(29).pointer, TEXELS + 11520 + 5 * 64, "plane 29 is face 5, level 4");
    let flat = texture_of("nearest_clamp");
    assert_eq!(flat.texture(0).dimension, TextureDimension::_2D);
    assert_eq!(build(flat).layout.fs_plane.expect("the planes").len, GenericPlane::LENGTH);
}

/// A vertex shader that samples and a draw without a vertex texture (or
/// the other way round) is refused, as for the fragment stage.
#[test]
fn a_vertex_texture_and_its_shader_must_agree() {
    let mut desc_m = vec![0u8; 0x4000];
    let (mut code_m, mut spds_m) = (vec![0u8; 0x800], vec![0u8; 0x100]);
    let mut code = Arena::new("code", 0x7fff_ffff_c000, &mut code_m).unwrap();
    let mut spds = Arena::new("spds", SPDS, &mut spds_m).unwrap();
    let bin = [0u8; 128];
    let vs_tex =
        upload_program(&mut code, &mut spds, &bin, ShaderMeta { samples_texture: true, ..meta(ShaderStage::VERTEX) })
            .unwrap();
    let vs_plain = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::VERTEX)).unwrap();
    let fs = upload_program(
        &mut code,
        &mut spds,
        &bin,
        ShaderMeta { samples_texture: false, ..meta(ShaderStage::FRAGMENT) },
    )
    .unwrap();
    let attributes = [VertexAttribute {
        buffer: VertexBuffer { gpu: 0x7fff_ffec_c000, size: 96 },
        offset: 0,
        stride: 16,
        components: 4,
        instance_divisor: 0,
    }];
    let ctx = Context {
        heap: HEAP_DESC,
        geometry_buffer: GEOMETRY,
        geometry_buffer_size: GEOMETRY_BUFFER_BYTES,
        sample_positions: SAMPLES,
        tiler_max_levels: 8,
    };
    let target =
        Target { gpu: TARGET, width: 64, height: 64, row_stride: 256, format: TargetFormat::Rgba8888, samples: 1 };
    let mut words = vec![0u64; 512];
    let mut desc = Arena::new("desc", DESC, &mut desc_m).unwrap();
    let mut cs = Stream::new(&mut words, 92);
    let mut open = begin_frame(&mut desc, &mut cs).unwrap();
    let draw = |vs, vs_texture| DrawCall {
        sample_mask: 0xf,
        vs,
        fs: &fs,
        primitive: Topology::Triangles,
        first_vertex: 0,
        vertex_count: 6,
        instance_count: 1,
        indices: None,
        attributes: &attributes,
        viewport: Viewport { x: 0, y: 0, width: 64, height: 64, near: 0.0, far: 1.0 },
        scissor: None,
        line_width: 1.0,
        flip_y: false,
        blend_color: [0.0; 4],
        vs_uniforms: &[],
        fs_uniforms: &[],
        zs: DepthStencilState::OFF,
        colour_mask: 0xf,
        front_cw: false,
        texture: None,
        vs_texture,
        blend: None,
        polygon_offset: None,
        occlusion_query: None,
        alpha_to_coverage: false,
        blend_shader: None,
    };
    let mut misaligned = texture_of("nearest_clamp");
    misaligned.level[0] = TextureLevel { gpu: TEXELS + 32, row_stride: ROW_STRIDE };
    for (d, want) in [
        (draw(&vs_tex, None), FrameError::Invalid(INVALID_TEXTURE)),
        (draw(&vs_plain, Some(texture_of("nearest_clamp"))), FrameError::Invalid(INVALID_TEXTURE)),
        (draw(&vs_tex, Some(misaligned)), FrameError::Invalid(INVALID_TEXTURE_ALIGN)),
    ] {
        let got = add_draw(&mut open, &mut desc, &mut cs, &ctx, &target, false, &d).map(|_| ());
        assert_eq!(got.map_err(|e| format!("{e}")), Err(format!("{want}")));
    }
    assert_eq!(open.draws, 0, "nothing was added");
}

/* ---- case 169's depth textures (tools/mali/board-dump/dstex-2026-10-20.txt) */

const DSTEX: &str = include_str!("../../tools/mali/board-dump/dstex-2026-10-20.txt");

/// Case 169's two depth textures as this backend binds them, each at the
/// trace's own texel address and row stride (the layout is what is
/// compared; the address is the GL core's R32F storage on the board):
/// `ramp`, the 4x1 GL_DEPTH_COMPONENT texture the CPU uploaded, NEAREST,
/// CLAMP_TO_EDGE; `rendered`, the 64x64 GL_DEPTH_STENCIL texture a frame
/// rendered into, NEAREST, REPEAT (GL's default wraps). The GL core keeps
/// both as R32F texels (texture.rs; the rendered one written by
/// `mali_depth_resolve`), which `texel_format` samples as Mesa's R32_FLOAT
/// (`R32F`, R001 spelt RGB1), the row the backend binds for
/// GLES_TEXFMT_R32F; `luminance` is the ES 2.0 surface's (d, d, d, 1)
/// (`GlesTextureBind::depth_luminance`, SWIZZLE_RRR1), else the ES 3.0
/// (d, 0, 0, 1) read through the identity swizzle.
fn depth_texture_of(source: &str, luminance: bool) -> SampledTexture {
    let (width, height, pointer, row_stride, wrap) = match source {
        "ramp" => (4, 1, 0x7fff_fffe_a000, 64, WrapMode::CLAMP_TO_EDGE),
        _ => (64, 64, 0x7fff_ffec_0000, 256, WrapMode::REPEAT),
    };
    let mut level = [TextureLevel::default(); MAX_TEXTURE_LEVELS];
    level[0] = TextureLevel { gpu: pointer, row_stride };
    SampledTexture {
        width,
        height,
        level,
        levels: 1,
        faces: 1,
        format: Format::R32F.0 << 12 | RgbComponentOrder::RGB1.0,
        clump: ClumpFormat::RAW32,
        block_bytes: 0,
        swizzle: if luminance { SWIZZLE_RRR1 } else { SWIZZLE_RGBA },
        min_nearest: true,
        mag_nearest: true,
        mipmap: MipmapMode::NONE,
        wrap_s: wrap,
        wrap_t: wrap,
    }
}

/// Our draw with the depth texture of `source` against the board's frame
/// that samples it: the sampler, the texture descriptor and its plane, the
/// resource table's entries 3 and 4, the fragment SPD and the draw flags.
/// Mesa samples the depth storage itself (Z16 UNORM / Z24X8 UNORM in
/// place, swizzle RRR1); ours samples the R32F texels the GL core holds, so
/// the format word differs by design, and, for the ES 3.0 read, the
/// swizzle: every differing byte is named, every other byte is equal.
fn check_depth(source: &'static str, luminance: bool) {
    let tex = depth_texture_of(source, luminance);
    /* the case samples over the window (a pbuffer): a flipped draw, whose
     * front face is CCW in the DCD flags (r57 bit 16) as the board's */
    let b = build_draw(Some(tex), None, true);
    let m = trace::memory_of(DSTEX, source);
    let ours = |gpu: u64, len: usize| b.desc[(gpu - DESC) as usize..(gpu - DESC) as usize + len].to_vec();
    let mesa_regs = regs_at_idvs(&trace::cs_words(DSTEX, source).iter().map(|w| w.word).collect::<Vec<_>>());
    let our_regs = regs_at_idvs(&b.words);
    assert_eq!((mesa_regs.len(), our_regs.len()), (1, 1), "one RUN_IDVS each");
    let reg64 = |r: &BTreeMap<u32, u32>, n: u32| u64::from(r[&n]) | u64::from(r[&(n + 1)]) << 32;
    let (mr, or) = (&mesa_regs[0], &our_regs[0]);

    let srt = reg64(mr, 4) & !63;
    let mesa_table = trace::bytes(&m, srt, 7 * Resource::LENGTH);
    let entry = |t: &[u8], i: usize| Resource::unpack(t[16 * i..16 * i + 16].try_into().unwrap()).unwrap();
    let (m_sampler, m_texture) = (entry(&mesa_table, 3), entry(&mesa_table, 4));
    let m_tex_bytes = trace::bytes(&m, m_texture.address, Texture::LENGTH);
    let m_tex = Texture::unpack(m_tex_bytes[..].try_into().unwrap()).unwrap();
    let m_plane_bytes = trace::bytes(&m, m_tex.surfaces, GenericPlane::LENGTH);
    let m_plane = GenericPlane::unpack(m_plane_bytes[..].try_into().unwrap()).unwrap();
    let m_spd = trace::bytes(&m, reg64(mr, 20), ShaderProgram::LENGTH);
    let m_sampler_bytes = trace::bytes(&m, m_sampler.address, Sampler::LENGTH);

    let our_table = ours(b.layout.fs_resources.expect("a fragment resource table").gpu, 7 * Resource::LENGTH);
    let (o_sampler, o_texture) = (entry(&our_table, 3), entry(&our_table, 4));
    let o_tex_block = b.layout.fs_texture.expect("a texture descriptor");
    let o_plane_block = b.layout.fs_plane.expect("a plane");
    assert_eq!(o_texture.address, o_tex_block.gpu);
    let o_tex_bytes = ours(o_tex_block.gpu, Texture::LENGTH);
    let o_tex = Texture::unpack(o_tex_bytes[..].try_into().unwrap()).unwrap();
    let o_plane_bytes = ours(o_plane_block.gpu, GenericPlane::LENGTH);
    let o_plane = GenericPlane::unpack(o_plane_bytes[..].try_into().unwrap()).unwrap();
    let o_spd = b.spds[(b.fs_spd - SPDS) as usize..(b.fs_spd - SPDS) as usize + ShaderProgram::LENGTH].to_vec();

    /* the fields the differing bytes hold, checked whole */
    let (mesa_format, mesa_clump) = match source {
        "ramp" => (Format::Z16_UNORM, ClumpFormat::RAW16),
        _ => (Format::Z24X8_UNORM, ClumpFormat::RAW32),
    };
    assert_eq!(m_tex.format, mesa_format.0 << 12 | RgbComponentOrder::RGBA.0, "Mesa's depth format");
    assert_eq!(o_tex.format, Format::R32F.0 << 12 | RgbComponentOrder::RGB1.0, "our R32F");
    assert_eq!(m_tex.swizzle, SWIZZLE_RRR1, "Mesa's luminance read: 2560 (RRR1)");
    assert_eq!(o_tex.swizzle, if luminance { SWIZZLE_RRR1 } else { SWIZZLE_RGBA });
    assert_eq!((m_plane.clump_format, o_plane.clump_format), (mesa_clump, ClumpFormat::RAW32));
    /* the rest of the descriptors' fields are the same */
    let same_tex = Texture { format: m_tex.format, swizzle: m_tex.swizzle, surfaces: m_tex.surfaces, ..o_tex };
    assert_eq!(same_tex, m_tex, "the texture descriptor's other fields");
    let same_plane = GenericPlane { clump_format: m_plane.clump_format, slice_stride: m_plane.slice_stride, ..o_plane };
    assert_eq!(same_plane, m_plane, "the plane's other fields");
    assert_eq!((m_plane.row_stride, m_plane.size), (tex.level[0].row_stride, tex.level[0].row_stride * tex.height));

    let mut diffs = vec![
        Difference {
            block: "Texture",
            at: 0x1,
            dump: m_tex_bytes[1..4].to_vec(),
            ours: o_tex_bytes[1..4].to_vec(),
            cause: "the format (bits 10-31): Mesa samples the depth storage itself (Z16 UNORM / Z24X8 UNORM, RGBA \
                    order); ours samples the GL core's R32F texels (R32F, R001 spelt RGB1) that the CPU upload or \
                    `mali_depth_resolve` wrote",
        },
        Difference {
            block: "Texture",
            at: 0x10,
            dump: m_tex.surfaces.to_le_bytes().to_vec(),
            ours: o_plane_block.gpu.to_le_bytes().to_vec(),
            cause: "the plane's address: Mesa keeps it in the sampler view's own buffer, ours is in the draw's",
        },
        Difference {
            block: "Plane",
            at: 0x18,
            dump: m_plane.slice_stride.to_le_bytes().to_vec(),
            ours: vec![0; 4],
            cause: "slice stride: Mesa 25.0.7 writes the level's size; 26.2.1 writes 0 for a single-sampled 2D image \
                    (pan_texture.c:416-420)",
        },
        Difference {
            block: "Fragment SPD",
            at: 0x08,
            dump: m_spd[8..16].to_vec(),
            ours: o_spd[8..16].to_vec(),
            cause: "the binary's address",
        },
        Difference {
            block: "Sampler",
            at: 0x02,
            dump: vec![m_sampler_bytes[2]],
            ours: vec![m_sampler_bytes[2] & !0x20],
            cause: "Round to nearest even: Mesa 25.0.7 keeps it for a nearest sampler; 26.2.1 clears it when \
                    minification and magnification are nearest (pan_cmdstream.c:243-245)",
        },
    ];
    if !luminance {
        diffs.push(Difference {
            block: "Texture",
            at: 0x8,
            dump: m_tex_bytes[8..10].to_vec(),
            ours: o_tex_bytes[8..10].to_vec(),
            cause: "the swizzle (bits 64-75): Mesa's ES 3.1 context still reads GL_LUMINANCE's (d, d, d, 1), \
                    RRR1; the ES 3.0 surface of ours reads (d, 0, 0, 1) through the identity swizzle over R001 \
                    (`depth_luminance` 0: the case's own expectation, which Mesa's channels fail)",
        });
    }
    if source == "ramp" {
        diffs.push(Difference {
            block: "Plane",
            at: 0x3,
            dump: vec![m_plane_bytes[3]],
            ours: vec![o_plane_bytes[3]],
            cause: "the clump format (bits 24-31): Z16 UNORM's RAW16 against R32F's RAW32 (the 4x1 ramp, 8 bytes \
                    of Z16 or 16 of R32F, on one 64-byte row either way)",
        });
    }
    let mut failures = Vec::new();
    for (k, (mo, oo)) in [(3, (m_sampler, o_sampler)), (4, (m_texture, o_texture))] {
        if (mo.contains_descriptors, mo.size) != (oo.contains_descriptors, oo.size) {
            failures.push(format!("table entry {k}: Mesa {mo:?}, ours {oo:?}"));
        }
    }
    let blocks = [
        ("Sampler", ours(o_sampler.address, Sampler::LENGTH), m_sampler_bytes.clone()),
        ("Texture", o_tex_bytes.clone(), m_tex_bytes.clone()),
        ("Plane", o_plane_bytes.clone(), m_plane_bytes.clone()),
        ("Fragment SPD", o_spd.clone(), m_spd.clone()),
    ];
    for (name, o, d) in &blocks {
        if let Err(e) = compare(name, o, d, &diffs) {
            failures.push(e);
        }
    }
    for reg in [56, 57, 58] {
        if mr[&reg] != or[&reg] {
            failures.push(format!("r{reg}: Mesa {:#x}, ours {:#x}", mr[&reg], or[&reg]));
        }
    }
    eprintln!(
        "mali depth texture draw vs the board ({source}, luminance {luminance}): {} differences named",
        diffs.len()
    );
    for d in &diffs {
        eprintln!("  {} +{:#x}: {}", d.block, d.at, d.cause);
    }
    assert!(failures.is_empty(), "{source}:\n{}", failures.join("\n"));
}

#[test]
fn the_uploaded_depth_ramp_is_the_board_s() {
    check_depth("ramp", true);
    check_depth("ramp", false);
}

#[test]
fn the_rendered_depth_texture_is_the_board_s() {
    check_depth("rendered", true);
    check_depth("rendered", false);
}
