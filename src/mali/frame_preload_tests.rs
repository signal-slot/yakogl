// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The frame builder's colour preload against the board: the second frame
//! of preload.c's `strip` mode, which Mesa 25.0.7 starts from the first
//! frame's image with its pre-frame draw (tools/mali/board-dump/
//! preload-2026-10-19.txt, cut from 0dce654a8:tools/mali/board-ref/2026-10-19-preload/
//! mesa), built again with the target and the preload program at the
//! trace's own addresses. The same on a 4x view of that target (gl_test
//! case 208 msaapre, tools/mali/board-dump/msaa-preload-2026-10-23.txt):
//! Mesa's pre-frame draws there are the single-sampled ones.
//!
//! What is compared: the framebuffer descriptor's frame shader fields and
//! its render target, the pre-frame DCD and every descriptor it names
//! (depth/stencil, blend, shader program descriptor, the four resource
//! table entries, attribute, attribute buffer and its four corners,
//! sampler, texture, plane). Each address is followed to the block it names
//! on both sides, and ours is then given Mesa's address; every byte left
//! is equal but the ones listed with their cause, and the dump is checked
//! to hold what each cause says.

use super::arena::Arena;
use super::cs::Stream;
use super::frame::*;
use super::genxml::v10::*;
use super::trace;

const DUMP: &str = include_str!("../../tools/mali/board-dump/preload-2026-10-19.txt");
const SOURCE: &str = "mesa-strip";

/* the trace's addresses: the descriptor buffer (the FBD at its start), the
 * render target ("Plane 0: Pointer"), the preload program ("Binary"), the
 * sample positions and the heap */
const DESC: u64 = 0x7fff_fffd_4000;
const TARGET: u64 = 0x7fff_fffe_4000;
const BINARY: u64 = 0x7fff_ffff_e100;
const SAMPLES: u64 = 0x7fff_ffff_f000;
const HEAP_DESC: u64 = 0x7fff_ffff_b000;
const GEOMETRY: u64 = 0x7fff_fffe_b000;
/* preload_zs.c's (tools/mali/board-dump/preload-zs-2026-10-19.txt): the
 * descriptor buffer and the D24S8 ("ZS Writeback Base") */
const ZS_DESC: u64 = 0x7fff_fffd_0000;
const ZS: u64 = 0x7fff_fffe_0000;
const FORMATS_DUMP: &str = include_str!("../../tools/mali/board-dump/preload-formats-2026-10-19.txt");
const ZS_DUMP: &str = include_str!("../../tools/mali/board-dump/preload-zs-2026-10-19.txt");
/* msaapre.c's (tools/mali/board-dump/msaa-preload-2026-10-23.txt): a 4x
 * view of preload_zs.c's target and D24S8, the descriptor buffer at ZS_DESC */
const MSAA_DUMP: &str = include_str!("../../tools/mali/board-dump/msaa-preload-2026-10-23.txt");

fn meta(stage: ShaderStage) -> ShaderMeta<'static> {
    let fragment = stage == ShaderStage::FRAGMENT;
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
        colour_outputs: u32::from(fragment),
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

/// The frame: one draw, started from the old image by the preload program
/// at the trace's address, the descriptor arena at `desc_base`. With `zs`,
/// the draw is depth-tested (LESS, writes on) on the trace's D24S8, whose
/// old depth and stencil the Z/S preload program draws back. Returns the
/// descriptor arena's bytes and the FBD's and the TLS's addresses.
fn build(desc_base: u64, target: Target, zs: Option<ZsMode>) -> (Vec<u8>, u64, u64) {
    let (mut desc_m, mut code_m, mut spd_m, mut ctx_m) =
        (vec![0u8; 0x4000], vec![0u8; 0x800], vec![0u8; 0x100], vec![0u8; 0x200]);
    let mut words = vec![0u64; 512];
    let (fbd, tls);
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
        let mut desc = Arena::new("desc", desc_base, &mut desc_m).unwrap();
        let mut code = Arena::new("code", 0x7fff_ffff_c000, &mut code_m).unwrap();
        let mut spds = Arena::new("spds", 0x7fff_ffff_d000, &mut spd_m).unwrap();
        let bin = [0u8; 128];
        let vs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::VERTEX)).unwrap();
        let fs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::FRAGMENT)).unwrap();
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
            viewport: Viewport {
                x: 0,
                y: 0,
                width: target.width as i32,
                height: target.height as i32,
                near: 0.0,
                far: 1.0,
            },
            scissor: None,
            line_width: 1.0,
            /* a window's layouts are the pbuffer's, top-down */
            flip_y: target.format != TargetFormat::Rgba8888,
            blend_color: [0.0; 4],
            vs_uniforms: &[],
            fs_uniforms: &[],
            zs: if zs.is_some() {
                DepthStencilState { depth_func: Func::LESS, depth_write: true, ..DepthStencilState::OFF }
            } else {
                DepthStencilState::OFF
            },
            colour_mask: 0xf,
            front_cw: false,
            texture: None,
            vs_texture: None,
            occlusion_query: None,
            alpha_to_coverage: false,
            blend_shader: None,
            blend: None,
            polygon_offset: None,
        };
        let mut cs = Stream::new(&mut words, 92);
        let mut open = begin_frame(&mut desc, &mut cs).unwrap();
        /* the program's own preload word ("R48-R63: 0x3000": r60, r61) */
        open.colour_preload = Some(FrameShader { binary: BINARY, preload: 0x3000 });
        let attachment =
            zs.map(|m| ZsAttachment { gpu: ZS, row_stride: 256, clear_depth: m.clear_depth, clear_stencil: 0 });
        if let Some(m) = zs {
            let shader = FrameShader { binary: m.binary, preload: 0x3000 };
            open.zs_preload = Some(ZsPreload { shader, z: m.z, s: m.s, cleared: m.z != m.s });
        }
        add_draw(&mut open, &mut desc, &mut cs, &ctx, &target, zs.is_some(), &draw).unwrap();
        (fbd, tls) = (open.layout.fbd.unwrap().gpu, open.layout.tls.unwrap().gpu);
        end_frame(&open, &mut desc, &mut cs, &ctx, &target, [0.0; 4], attachment).unwrap();
    }
    (desc_m, fbd, tls)
}

/// The framebuffer parameters a 4x target moves: the sample count and
/// pattern, the tile size and the colour buffer allocation.
fn samples(p: &FramebufferParameters) -> (u32, SamplePattern, u32, u32) {
    (p.sample_count, p.sample_pattern, p.effective_tile_size, p.color_buffer_allocation)
}

/// One place where our bytes may differ from the dump's.
struct Difference {
    block: String,
    at: usize,
    dump: Vec<u8>,
    ours: Vec<u8>,
    cause: &'static str,
}

/// Mesa's bytes, with each listed difference's bytes replaced by ours,
/// against ours.
fn compare(name: &str, ours: &[u8], dump: &[u8], diffs: &[Difference]) -> Result<(), String> {
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

/// The trace's RGBA8 FBO (preload.c, preload_zs.c).
const RGBA_TARGET: Target =
    Target { gpu: TARGET, width: 64, height: 64, row_stride: 256, format: TargetFormat::Rgba8888, samples: 1 };

#[test]
fn the_colour_preload_s_pre_frame_draw_is_the_board_s() {
    check_colour(DUMP, SOURCE, DESC, RGBA_TARGET);
}

/// preload_fmt.c's window frames (tools/mali/board-dump/
/// preload-formats-2026-10-19.txt): the target read back through its own
/// layout's texture format and blended back through its own conversion.
#[test]
fn the_colour_preload_of_xrgb8888_and_rgb565_is_the_board_s() {
    let target = |gpu, row_stride, format| Target { gpu, width: 64, height: 48, row_stride, format, samples: 1 };
    check_colour(
        FORMATS_DUMP,
        "mesa-xrgb8888",
        0x7fff_fffd_5000,
        target(0x7fff_fffe_5000, 256, TargetFormat::Xrgb8888),
    );
    check_colour(FORMATS_DUMP, "mesa-rgb565", 0x7fff_fffd_6000, target(0x7fff_fffe_6000, 128, TargetFormat::Rgb565));
}

/// One colour preload frame against the board's: the trace `source` of
/// `dump`, its descriptor buffer at `desc`, the target `target`. What is
/// compared is the module documentation's list.
fn check_colour(dump: &str, source: &str, desc: u64, target: Target) {
    let (desc_m, fbd, tls) = build(desc, target, None);
    let m = trace::memory_of(dump, source);
    let ours = |gpu: u64, len: usize| desc_m[(gpu - desc) as usize..(gpu - desc) as usize + len].to_vec();
    let mesa = |gpu: u64, len: usize| trace::bytes(&m, gpu, len);
    let mut failures = Vec::new();
    let mut check = |name: &str, o: Vec<u8>, d: Vec<u8>, diffs: &[Difference]| {
        if let Err(e) = compare(name, &o, &d, diffs) {
            failures.push(e);
        }
    };

    /* the FBD: Mesa's is the descriptor buffer's start, ours the frame's */
    let (o_params, m_params) = (
        FramebufferParameters::unpack(&ours(fbd, FramebufferParameters::LENGTH)[..].try_into().unwrap()).unwrap(),
        FramebufferParameters::unpack(&mesa(desc, FramebufferParameters::LENGTH)[..].try_into().unwrap()).unwrap(),
    );
    let modes = |p: &FramebufferParameters| (p.pre_frame_0, p.pre_frame_1, p.post_frame);
    assert_eq!(
        modes(&m_params),
        (PrePostFrameShaderMode::INTERSECT, PrePostFrameShaderMode::NEVER, PrePostFrameShaderMode::NEVER)
    );
    assert_eq!(modes(&o_params), modes(&m_params), "the frame shader modes");
    assert_eq!(samples(&o_params), samples(&m_params), "the frame's samples and tile");
    assert_eq!(m_params.sample_count, target.samples);
    /* the render target, 128 bytes on (no ZS/CRC extension on either) */
    let rt_at = 128;
    check(
        "Render target",
        ours(fbd + rt_at, RgbRenderTarget::LENGTH),
        mesa(desc + rt_at, RgbRenderTarget::LENGTH),
        &[],
    );

    /* the pre-frame DCD and the blocks it names */
    let (o_dcd_at, m_dcd_at) = (o_params.frame_shader_dcds, m_params.frame_shader_dcds);
    let unpack_dcd = |b: Vec<u8>| Draw::unpack(&b[..].try_into().unwrap()).unwrap();
    let (mut o_dcd, m_dcd) = (unpack_dcd(ours(o_dcd_at, Draw::LENGTH)), unpack_dcd(mesa(m_dcd_at, Draw::LENGTH)));
    /* pre frame 1 and the post frame: Mesa allocates the three, uses one */
    check("DCDs 1 and 2", ours(o_dcd_at + 128, 256), mesa(m_dcd_at + 128, 256), &[]);
    check(
        "Depth/stencil",
        ours(o_dcd.depth_stencil, DepthStencil::LENGTH),
        mesa(m_dcd.depth_stencil, DepthStencil::LENGTH),
        &[],
    );
    let blend = mesa(m_dcd.blend, Blend::LENGTH);
    let diffs = [Difference {
        block: "Blend".into(),
        at: 0x0f,
        dump: vec![blend[15]],
        ours: vec![blend[15] & !1],
        cause: "bit 24 of the fixed-function conversion word: Mesa 25.0.7 sets it (its pandecode prints Register \
                Format F32 there); 26.2.1's v10 conversion has no field above bit 22 (v10.xml:1026-1029), its \
                preload sets a register format only before v9 (pan_fb_preload.c:141-145), and 26.2.1 under \
                drm-shim (tools/mali/drm_shim_ref.sh with gl_ref.py --second-frame) writes the word as \
                0x000ed000 (its pandecode: Shader PC 970752)",
    }];
    check("Blend", ours(o_dcd.blend, Blend::LENGTH), blend, &diffs);
    let mut named: Vec<Difference> = diffs.into();
    /* the SPD names the program at the trace's address on both sides */
    let (o_spd, m_spd) = (o_dcd.shader.shader, m_dcd.shader.shader);
    check("SPD", ours(o_spd, ShaderProgram::LENGTH), mesa(m_spd, ShaderProgram::LENGTH), &[]);
    /* the resource table, its count (4) in the address's low bits */
    assert_eq!((o_dcd.shader.resources & 63, m_dcd.shader.resources & 63), (4, 4), "four resource tables");
    let (o_table, m_table) = (o_dcd.shader.resources & !63, m_dcd.shader.resources & !63);
    let entry = |b: Vec<u8>, i: usize| Resource::unpack(&b[16 * i..16 * i + 16].try_into().unwrap()).unwrap();
    let (o_t, m_t) = (ours(o_table, 4 * Resource::LENGTH), mesa(m_table, 4 * Resource::LENGTH));
    for k in 0..4 {
        let (mut o, mo) = (entry(o_t.clone(), k), entry(m_t.clone(), k));
        o.address = mo.address;
        check(&format!("Resource {k}"), o.pack().unwrap().to_vec(), m_t[16 * k..16 * k + 16].to_vec(), &[]);
    }
    let addr = |t: &[u8], k: usize| entry(t.to_vec(), k).address;
    check("Attribute", ours(addr(&o_t, 0), Attribute::LENGTH), mesa(addr(&m_t, 0), Attribute::LENGTH), &[]);
    /* the buffer's address at +8 (genxml's Buffer has no unpack) */
    let (mut o_buf, m_buf) = (ours(addr(&o_t, 1), Buffer::LENGTH), mesa(addr(&m_t, 1), Buffer::LENGTH));
    let at8 = |b: &[u8]| u64::from_le_bytes(b[8..16].try_into().unwrap());
    check("Corners", ours(at8(&o_buf), 64), mesa(at8(&m_buf), 64), &[]);
    o_buf[8..16].copy_from_slice(&m_buf[8..16]);
    check("Buffer", o_buf, m_buf, &[]);
    check("Sampler", ours(addr(&o_t, 2), Sampler::LENGTH), mesa(addr(&m_t, 2), Sampler::LENGTH), &[]);
    let unpack_tex = |b: Vec<u8>| Texture::unpack(&b[..].try_into().unwrap()).unwrap();
    let (mut o_tex, m_tex) =
        (unpack_tex(ours(addr(&o_t, 3), Texture::LENGTH)), unpack_tex(mesa(addr(&m_t, 3), Texture::LENGTH)));
    let diffs = [Difference {
        block: "Plane".into(),
        at: 0x18,
        dump: (target.row_stride * target.height).to_le_bytes().to_vec(),
        ours: vec![0; 4],
        cause: "slice stride: Mesa 25.0.7 writes the level's size; 26.2.1 writes 0 for a single-sampled 2D image \
                (pan_texture.c:416-420)",
    }];
    check("Plane", ours(o_tex.surfaces, GenericPlane::LENGTH), mesa(m_tex.surfaces, GenericPlane::LENGTH), &diffs);
    named.extend(diffs);
    o_tex.surfaces = m_tex.surfaces;
    check("Texture", o_tex.pack().unwrap().to_vec(), mesa(addr(&m_t, 3), Texture::LENGTH), &[]);

    /* the DCD itself, with Mesa's addresses: the frame's own TLS on both
     * sides */
    assert_eq!(o_dcd.shader.thread_storage, tls, "the frame's TLS");
    o_dcd.depth_stencil = m_dcd.depth_stencil;
    o_dcd.blend = m_dcd.blend;
    o_dcd.shader.resources = m_dcd.shader.resources;
    o_dcd.shader.shader = m_dcd.shader.shader;
    o_dcd.shader.thread_storage = m_dcd.shader.thread_storage;
    check("DCD 0", o_dcd.pack().unwrap().to_vec(), mesa(m_dcd_at, Draw::LENGTH), &[]);

    eprintln!("mali colour preload vs the board ({source}): {} difference(s) named", named.len());
    for d in &named {
        eprintln!("  {} +{:#x}: {}", d.block, d.at, d.cause);
    }
    assert!(failures.is_empty(), "{source}:\n{}", failures.join("\n"));
}

/// One of preload_zs.c's modes: the planes Mesa preloads, the address of
/// its Z/S preload program in the trace, and the frame's depth clear value
/// (`fb->zs.clear_value`: 1.0 where the frame cleared the depth, else 0).
#[derive(Clone, Copy)]
struct ZsMode {
    z: bool,
    s: bool,
    binary: u64,
    clear_depth: f32,
}

/// preload_zs.c's second frame (`source`, its trace in the excerpt): the
/// colour preload in DCD 0 and the depth/stencil preload in DCD 1 against
/// the board, each block followed as above, and the FBD's frame shader
/// modes, clear values and ZS/CRC extension (the tiles written clean only
/// when the frame clears the plane it does not preload).
fn check_zs(dump: &str, source: &str, target: Target, mode: ZsMode) {
    let (desc_m, fbd, tls) = build(ZS_DESC, target, Some(mode));
    let m = trace::memory_of(dump, source);
    let ours = |gpu: u64, len: usize| desc_m[(gpu - ZS_DESC) as usize..(gpu - ZS_DESC) as usize + len].to_vec();
    let mesa = |gpu: u64, len: usize| trace::bytes(&m, gpu, len);
    let mut failures = Vec::new();
    let mut check = |name: &str, o: Vec<u8>, d: Vec<u8>, diffs: &[Difference]| {
        if let Err(e) = compare(name, &o, &d, diffs) {
            failures.push(e);
        }
    };
    let unpack_params = |b: Vec<u8>| FramebufferParameters::unpack(&b[..].try_into().unwrap()).unwrap();
    let (o_params, m_params) = (
        unpack_params(ours(fbd, FramebufferParameters::LENGTH)),
        unpack_params(mesa(ZS_DESC, FramebufferParameters::LENGTH)),
    );
    let modes = |p: &FramebufferParameters| (p.pre_frame_0, p.pre_frame_1, p.post_frame);
    assert_eq!(
        modes(&m_params),
        (PrePostFrameShaderMode::INTERSECT, PrePostFrameShaderMode::EARLY_ZS_ALWAYS, PrePostFrameShaderMode::NEVER)
    );
    assert_eq!(modes(&o_params), modes(&m_params), "the frame shader modes");
    assert_eq!(samples(&o_params), samples(&m_params), "the frame's samples and tile");
    assert_eq!(m_params.sample_count, target.samples);
    assert_eq!(
        (o_params.z_clear, o_params.s_clear, o_params.z_write_enable),
        (m_params.z_clear, m_params.s_clear, m_params.z_write_enable)
    );
    /* the ZS/CRC extension, after the FBD's 128 bytes */
    let ext = [Difference {
        block: "ZS/CRC extension".into(),
        at: 0x30,
        dump: ZS.to_le_bytes().to_vec(),
        ours: vec![0; 8],
        cause: "S target base: Mesa 25.0.7 also writes the ZS base there with the S block format No Write \
                (unused); Mesa 26.2.1 merges no S part for a D24S8 view (as frame_zs_tests.rs names it)",
    }];
    check(
        "ZS/CRC extension",
        ours(fbd + 128, ZsCrcExtension::LENGTH),
        mesa(ZS_DESC + 128, ZsCrcExtension::LENGTH),
        &ext,
    );
    let unpack_dcd = |b: Vec<u8>| Draw::unpack(&b[..].try_into().unwrap()).unwrap();
    let (o_at, m_at) = (o_params.frame_shader_dcds, m_params.frame_shader_dcds);
    check("DCD 2", ours(o_at + 256, 128), mesa(m_at + 256, 128), &[]);
    let entry = |b: &[u8], i: usize| Resource::unpack(&b[16 * i..16 * i + 16].try_into().unwrap()).unwrap();
    let mut named: Vec<Difference> = ext.into();
    let planes = usize::from(mode.z) + usize::from(mode.s);
    for (k, textures) in [(0usize, 1usize), (1, planes)] {
        let at = 128 * k as u64;
        let (mut o_dcd, m_dcd) = (unpack_dcd(ours(o_at + at, Draw::LENGTH)), unpack_dcd(mesa(m_at + at, Draw::LENGTH)));
        check(
            &format!("DCD {k} ZSD"),
            ours(o_dcd.depth_stencil, DepthStencil::LENGTH),
            mesa(m_dcd.depth_stencil, DepthStencil::LENGTH),
            &[],
        );
        if k == 0 {
            let blend = mesa(m_dcd.blend, Blend::LENGTH);
            let diffs = [Difference {
                block: "DCD 0 blend".into(),
                at: 0x0f,
                dump: vec![blend[15]],
                ours: vec![blend[15] & !1],
                cause: "bit 24 of the fixed-function conversion word, Mesa 25.0.7 against 26.2.1 (as the colour \
                        preload's test names it)",
            }];
            check("DCD 0 blend", ours(o_dcd.blend, Blend::LENGTH), blend, &diffs);
            named.extend(diffs);
        }
        check(
            &format!("DCD {k} SPD"),
            ours(o_dcd.shader.shader, ShaderProgram::LENGTH),
            mesa(m_dcd.shader.shader, ShaderProgram::LENGTH),
            &[],
        );
        let (o_t, m_t) = (ours(o_dcd.shader.resources & !63, 64), mesa(m_dcd.shader.resources & !63, 64));
        for e in 0..4 {
            let (mut o, mo) = (entry(&o_t, e), entry(&m_t, e));
            o.address = mo.address;
            check(&format!("DCD {k} resource {e}"), o.pack().unwrap().to_vec(), m_t[16 * e..16 * e + 16].to_vec(), &[]);
        }
        check(
            &format!("DCD {k} attribute"),
            ours(entry(&o_t, 0).address, Attribute::LENGTH),
            mesa(entry(&m_t, 0).address, Attribute::LENGTH),
            &[],
        );
        let (mut o_buf, m_buf) =
            (ours(entry(&o_t, 1).address, Buffer::LENGTH), mesa(entry(&m_t, 1).address, Buffer::LENGTH));
        let at8 = |b: &[u8]| u64::from_le_bytes(b[8..16].try_into().unwrap());
        check(&format!("DCD {k} corners"), ours(at8(&o_buf), 64), mesa(at8(&m_buf), 64), &[]);
        o_buf[8..16].copy_from_slice(&m_buf[8..16]);
        check(&format!("DCD {k} buffer"), o_buf, m_buf, &[]);
        check(
            &format!("DCD {k} sampler"),
            ours(entry(&o_t, 2).address, Sampler::LENGTH),
            mesa(entry(&m_t, 2).address, Sampler::LENGTH),
            &[],
        );
        for x in 0..textures {
            let (o_tb, m_tb) = (entry(&o_t, 3).address + 32 * x as u64, entry(&m_t, 3).address + 32 * x as u64);
            let unpack_tex = |b: Vec<u8>| Texture::unpack(&b[..].try_into().unwrap()).unwrap();
            let (mut o_tex, m_tex) = (unpack_tex(ours(o_tb, Texture::LENGTH)), unpack_tex(mesa(m_tb, Texture::LENGTH)));
            let name = format!("DCD {k} plane {x}");
            let plane = mesa(m_tex.surfaces, GenericPlane::LENGTH);
            let diffs = [Difference {
                block: name.clone(),
                at: 0x18,
                dump: plane[0x18..0x1c].to_vec(),
                ours: vec![0; 4],
                cause: "slice stride: Mesa 25.0.7 writes the level's size; 26.2.1 writes 0 for a single-sampled 2D \
                        image (pan_texture.c:416-420)",
            }];
            check(&name, ours(o_tex.surfaces, GenericPlane::LENGTH), plane, &diffs);
            named.extend(diffs);
            o_tex.surfaces = m_tex.surfaces;
            check(&format!("DCD {k} texture {x}"), o_tex.pack().unwrap().to_vec(), mesa(m_tb, Texture::LENGTH), &[]);
        }
        assert_eq!(o_dcd.shader.thread_storage, tls, "the frame's TLS");
        o_dcd.depth_stencil = m_dcd.depth_stencil;
        o_dcd.blend = m_dcd.blend;
        o_dcd.shader.resources = m_dcd.shader.resources;
        o_dcd.shader.shader = m_dcd.shader.shader;
        o_dcd.shader.thread_storage = m_dcd.shader.thread_storage;
        check(&format!("DCD {k}"), o_dcd.pack().unwrap().to_vec(), mesa(m_at + at, Draw::LENGTH), &[]);
    }
    eprintln!("mali Z/S preload vs the board ({source}): {} difference(s) named", named.len());
    for d in &named {
        eprintln!("  {} +{:#x}: {}", d.block, d.at, d.cause);
    }
    assert!(failures.is_empty(), "{source}:\n{}", failures.join("\n"));
}

#[test]
fn the_depth_and_stencil_preload_is_the_board_s() {
    let mode = ZsMode { z: true, s: true, binary: 0x7fff_ffff_e180, clear_depth: 0.0 };
    check_zs(ZS_DUMP, "mesa-depth", RGBA_TARGET, mode);
    check_zs(ZS_DUMP, "mesa-stencil", RGBA_TARGET, mode);
}

#[test]
fn the_depth_only_preload_is_the_board_s() {
    let mode = ZsMode { z: true, s: false, binary: 0x7fff_ffff_e000, clear_depth: 0.0 };
    check_zs(ZS_DUMP, "mesa-depth_sclear", RGBA_TARGET, mode);
}

#[test]
fn the_stencil_only_preload_is_the_board_s() {
    let mode = ZsMode { z: false, s: true, binary: 0x7fff_ffff_e080, clear_depth: 1.0 };
    check_zs(ZS_DUMP, "mesa-stencil_zclear", RGBA_TARGET, mode);
}

/// msaapre.c's `msrtt` mode, gl_test case 208's second frame: a 4x view of
/// a single-sampled RGBA8 texture drawn over without a clear. Mesa reads
/// the image's sample count, not the view's (`pan_preload_is_ms`,
/// pan_fb_preload.c:162-179): the pre-frame draw is the 1x one, byte for
/// byte (its program at the 1x trace's address, a 2D texture of 1 sample,
/// multisampling and per-sample shading off, sample mask 0xFFFF), in a
/// frame of 4 samples whose render target averages them.
#[test]
fn the_colour_preload_of_a_4x_target_is_the_board_s() {
    check_colour(MSAA_DUMP, "mesa-msrtt", ZS_DESC, Target { samples: 4, ..RGBA_TARGET });
}

/// msaapre.c's `msrttz` mode: the same 4x view with its D24S8 attached on
/// DEPTH, the second frame depth-tested against the first's depth without
/// a clear. Mesa preloads the colour and the depth with the 1x programs
/// (the depth-only one at preload_zs.c's address); the ZS target averages.
#[test]
fn the_depth_preload_of_a_4x_target_is_the_board_s() {
    let mode = ZsMode { z: true, s: false, binary: 0x7fff_ffff_e000, clear_depth: 0.0 };
    check_zs(MSAA_DUMP, "mesa-msrttz", Target { samples: 4, ..RGBA_TARGET }, mode);
}
