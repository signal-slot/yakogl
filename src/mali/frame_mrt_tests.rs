// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A frame of several colour targets against Mesa 26.2.1 on the drm-shim:
//! gl_test case 55's two-output program into two 64x64 RGBA8 targets
//! (tools/mali/testdata/fsout_55_es3mrt.trace, `gl_ref.py --mrt 2`) and its
//! three-output twin, case 106 (fsout_106_mrt3.trace, `--mrt 3`); the
//! commands are in docs/notes/mali-fragment-outputs.md. The program words
//! of those references (`.expect.json`) are the compiler's (item fsout-55);
//! this file compares what the frame builds for them: the descriptors
//! pandecode decoded, and the RUN_IDVS and RUN_FRAGMENT words it printed.
//!
//! The same:
//! - the blend descriptor array: one 16-byte descriptor a target, the
//!   descriptor of target `i` Mesa's `Blend RT i` byte for byte (its
//!   decoded fields packed again), and `BLEND_DESC` (d50) tagged with the
//!   target count (`...182` for two, `...1C3` for three);
//! - DCD1 (r58): Mesa's word, `Render Target Mask` 0x3 / 0x7;
//! - the framebuffer parameters: `Render Target Count` 2 / 3, `Color Buffer
//!   Allocation` 8192 / 12288, `Effective Tile Size` 1024, and every field
//!   but those named below;
//! - the FBD pointer's render target count;
//! - per render target: the tile-buffer offset (0, 4096, 8192), formats,
//!   swizzle, dithering, clean tile writes and the clear colour.
//!
//! What differs, each named: Mesa's targets are AFBC, ours linear
//! (`Writeback Block Format` AFBC, its `Header`, `Row stride`, `Body
//! offset`, `YUV Transform`, `Compression mode` and `Writeback Float
//! Format`, against our `Linear` with the target's base and row stride),
//! and with AFBC Mesa keeps a CRC: its FBD has the ZS/CRC extension (`Has
//! ZS CRC Extension`, `CRC Read Enable`, `CRC Write Enable`, `Empty Tile
//! Write Enable`, and the FBD pointer's ZS/CRC bit), ours, without a
//! depth/stencil attachment, none. The addresses (the tiler context, the
//! descriptors) are where each builder put its blocks.

use super::arena::Arena;
use super::cs::{Instr, Stream};
use super::frame::*;
use super::genxml::v10::*;
use std::collections::BTreeMap;

const MRT2: &str = include_str!("../../tools/mali/testdata/fsout_55_es3mrt.trace");
const MRT3: &str = include_str!("../../tools/mali/testdata/fsout_106_mrt3.trace");
const DESC: u64 = 0x7fff_fffd_0000;
/// The targets' memory: 64 rows of 256 bytes each, back to back.
const TARGET_GPU: u64 = 0x7fff_ffe0_0000;

/// One decoded descriptor: pandecode's `Key: value` lines, each key the
/// path of the headers above it (`Equation/RGB/A`).
type Fields = BTreeMap<String, String>;

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The fields of the first block whose header line starts with `header`,
/// at or after the first line that starts with `after`.
fn block(trace: &str, after: &str, header: &str) -> Fields {
    let lines: Vec<&str> = trace.lines().collect();
    let from = lines.iter().position(|l| l.trim_start().starts_with(after)).unwrap_or_else(|| panic!("no {after}"));
    let at = from
        + lines[from..]
            .iter()
            .position(|l| l.trim_start().starts_with(header))
            .unwrap_or_else(|| panic!("no {header}"));
    let base = indent(lines[at]);
    let mut path: Vec<(usize, String)> = Vec::new();
    let mut f = Fields::new();
    for l in lines[at + 1..].iter().take_while(|l| indent(l) > base && !l.trim().is_empty()) {
        let s = l.trim();
        while path.last().is_some_and(|(i, _)| *i >= indent(l)) {
            path.pop();
        }
        if let Some(h) = s.strip_suffix(':') {
            path.push((indent(l), h.to_string()));
            continue;
        }
        let (k, v) = s.split_once(": ").unwrap_or_else(|| panic!("{s}: not a field"));
        let key = path.iter().map(|(_, h)| h.as_str()).chain([k]).collect::<Vec<_>>().join("/");
        assert!(f.insert(key, v.to_string()).is_none(), "{s} twice");
    }
    f
}

fn take(f: &mut Fields, k: &str) -> String {
    f.remove(k).unwrap_or_else(|| panic!("no {k}"))
}

fn flag(f: &mut Fields, k: &str) -> bool {
    match take(f, k).as_str() {
        "true" => true,
        "false" => false,
        v => panic!("{k}: {v}"),
    }
}

fn num(f: &mut Fields, k: &str) -> u64 {
    let v = take(f, k);
    match v.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16),
        None => v.parse(),
    }
    .unwrap_or_else(|_| panic!("{k}: {v} not a number"))
}

fn done(f: &Fields, what: &str) {
    assert!(f.is_empty(), "{what}: fields not read: {f:?}");
}

/// The last word a `MOVE48 d<reg>` (or `MOVE32 r<reg>`) of the trace sets,
/// from its printed bytes.
fn cs_word(trace: &str, mnemonic: &str) -> u64 {
    let l = trace.lines().rev().find(|l| l.contains(mnemonic)).unwrap_or_else(|| panic!("no {mnemonic}"));
    let b: Vec<u8> = l.split_whitespace().take(8).map(|h| u8::from_str_radix(h, 16).expect("a printed byte")).collect();
    let w = u64::from_le_bytes(b.try_into().expect("eight bytes"));
    if mnemonic.starts_with("MOVE48") {
        w & 0xffff_ffff_ffff
    } else {
        w & 0xffff_ffff
    }
}

/// Mesa's blend descriptor of one target from its decoded fields, packed
/// with our packer (`Shader/PC` is the same bits as the fixed-function
/// part, read back separately).
fn mesa_blend(mut f: Fields) -> ([u8; 16], u32) {
    let f = &mut f;
    let mut func = |p: &str| {
        let a = match take(f, &format!("Equation/{p}/A")).as_str() {
            "Src" => BlendOperandA::SRC,
            v => panic!("A {v}"),
        };
        let b = match take(f, &format!("Equation/{p}/B")).as_str() {
            "Src" => BlendOperandB::SRC,
            v => panic!("B {v}"),
        };
        let c = match take(f, &format!("Equation/{p}/C")).as_str() {
            "Zero" => BlendOperandC::ZERO,
            v => panic!("C {v}"),
        };
        BlendFunction {
            a,
            negate_a: flag(f, &format!("Equation/{p}/Negate A")),
            b,
            negate_b: flag(f, &format!("Equation/{p}/Negate B")),
            c,
            invert_c: flag(f, &format!("Equation/{p}/Invert C")),
        }
    };
    let (rgb, alpha) = (func("RGB"), func("Alpha"));
    let mode = match take(f, "Internal/Mode").as_str() {
        "Opaque" => BlendMode::OPAQUE,
        "Fixed-Function" => BlendMode::FIXED_FUNCTION,
        v => panic!("mode {v}"),
    };
    let memory_format = match take(f, "Internal/Fixed-Function/Conversion/Format (v7)").as_str() {
        "RGBA8 TB RGBA" => Format::RGBA8_TB.0 << 12 | RgbComponentOrder::RGBA.0,
        v => panic!("format {v}"),
    };
    let b = Blend {
        load_destination: flag(f, "Load Destination"),
        alpha_to_one: flag(f, "Alpha To One"),
        enable: flag(f, "Enable"),
        srgb: flag(f, "sRGB"),
        round_to_fb_precision: flag(f, "Round to FB precision"),
        blend_constant: num(f, "Blend Constant") as u32,
        equation: BlendEquation { rgb, alpha, color_mask: num(f, "Equation/Color Mask") as u32 },
        internal: InternalBlend {
            mode,
            fixed_function: BlendFixedFunction {
                num_comps: num(f, "Internal/Fixed-Function/Num Comps") as u32,
                rt: num(f, "Internal/Fixed-Function/RT") as u32,
                conversion: InternalConversion {
                    memory_format,
                    raw: flag(f, "Internal/Fixed-Function/Conversion/Raw"),
                },
            },
            ..Default::default()
        },
    };
    let pc = num(f, "Internal/Shader/PC") as u32;
    done(f, "blend");
    (b.pack().expect("Mesa's blend packs"), pc)
}

fn meta(stage: ShaderStage, colour_outputs: u32) -> ShaderMeta<'static> {
    ShaderMeta {
        stage,
        register_allocation: ShaderRegisterAllocation::_32_PER_THREAD,
        preload: 0,
        sysvals: &[],
        push: &[],
        varyings: 0,
        varying_bytes: 0,
        secondary: None,
        no_psiz_offset: None,
        colour_outputs,
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

fn target(i: u64) -> Target {
    Target {
        gpu: TARGET_GPU + i * 0x4000,
        width: 64,
        height: 64,
        row_stride: 256,
        format: TargetFormat::Rgba8888,
        samples: 1,
    }
}

/// What a frame of `n` targets built: the descriptor arena, the RUN_IDVS
/// registers and the FBD pointer.
struct Built {
    desc: Vec<u8>,
    regs: BTreeMap<u32, u32>,
    frame: Frame,
}

/// gl_ref.py's MRT frame: `n` RGBA8 targets cleared to (0, 0, 1, 1), one
/// quad of a program writing outputs `outputs` (bit i: target i).
fn build(n: u64, outputs: u32) -> Result<Built, FrameError> {
    let (mut desc_m, mut code_m, mut spd_m, mut ctx_m) =
        (vec![0u8; 0x4000], vec![0u8; 0x800], vec![0u8; 0x100], vec![0u8; 0x200]);
    let mut words = vec![0u64; 512];
    let (len, frame);
    {
        let mut ctx_a = Arena::new("context", 0x7fff_ffff_b000, &mut ctx_m).unwrap();
        let heap = tiler_heap_descriptor(&mut ctx_a, 0x8000_00a0_0000, 2 << 20).unwrap();
        let ctx = Context {
            heap: heap.gpu,
            geometry_buffer: 0x7fff_fffe_b000,
            geometry_buffer_size: GEOMETRY_BUFFER_BYTES,
            sample_positions: 0x7fff_ffff_f000,
            tiler_max_levels: 8,
        };
        let mut desc = Arena::new("desc", DESC, &mut desc_m).unwrap();
        let mut code = Arena::new("code", 0x7fff_ffff_c000, &mut code_m).unwrap();
        let mut spds = Arena::new("spds", 0x7fff_ffff_d000, &mut spd_m).unwrap();
        let bin = [0u8; 128];
        let vs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::VERTEX, 0)).unwrap();
        let fs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::FRAGMENT, outputs)).unwrap();
        let attributes = [VertexAttribute {
            buffer: VertexBuffer { gpu: 0x7fff_ffec_c000, size: 48 },
            offset: 0,
            stride: 8,
            components: 2,
            instance_divisor: 0,
        }];
        let draw = DrawCall {
            zs: DepthStencilState::OFF,
            colour_mask: 0xf,
            front_cw: false,
            cull: Cull::None,
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
            flip_y: false,
            blend_color: [0.0; 4],
            blend: None,
            polygon_offset: None,
            sample_mask: 0xf,
            vs_uniforms: &[],
            fs_uniforms: &[],
            textures: &[],
            vs_texture: None,
            occlusion_query: None,
            alpha_to_coverage: false,
            blend_shader: None,
        };
        let more: Vec<Target> = (1..n).map(target).collect();
        let mut cs = Stream::new(&mut words, 92);
        let mut open = begin_frame_targets(&mut desc, &mut cs, &more)?;
        add_draw(&mut open, &mut desc, &mut cs, &ctx, &target(0), false, &draw)?;
        frame = end_frame(&open, &mut desc, &mut cs, &ctx, &target(0), [0.0, 0.0, 1.0, 1.0], None)?;
        len = cs.words().len();
    }
    let mut regs = BTreeMap::new();
    for &w in &words[..len] {
        match Instr::decode(w) {
            Ok(Instr::Move32(m)) => {
                regs.insert(m.destination, m.immediate);
            }
            Ok(Instr::Move48(m)) => {
                regs.insert(m.destination, m.immediate as u32);
                regs.insert(m.destination + 1, (m.immediate >> 32) as u32);
            }
            Ok(Instr::RunIdvs(_)) => break,
            _ => {}
        }
    }
    Ok(Built { desc: desc_m, regs, frame })
}

impl Built {
    fn at(&self, gpu: u64, len: usize) -> &[u8] {
        let o = (gpu - DESC) as usize;
        &self.desc[o..o + len]
    }
    fn d(&self, r: u32) -> u64 {
        u64::from(self.regs[&r]) | u64::from(self.regs[&(r + 1)]) << 32
    }
}

/// The frame of `n` targets against Mesa's `trace`.
fn compare(trace: &str, n: u64) {
    let outputs = (1u32 << n) - 1;
    let ours = build(n, outputs).expect("the MRT frame builds");

    /* BLEND_DESC: n descriptors, the count in the tag */
    let theirs_d50 = cs_word(trace, "MOVE48 d50,");
    assert_eq!(theirs_d50 & 63, n, "Mesa's BLEND_DESC count");
    let d50 = ours.d(50);
    assert_eq!(d50 & 63, n, "our BLEND_DESC count");
    for i in 0..n {
        let (theirs, pc) = mesa_blend(block(trace, "Blend RT 0:", &format!("Blend RT {i}:")));
        let b = ours.at((d50 & !63) + i * Blend::LENGTH as u64, Blend::LENGTH);
        assert_eq!(b, theirs, "the blend descriptor of target {i}");
        /* `Shader/PC` is bits 36-63, the conversion's: the same word */
        let hi = u32::from_le_bytes(b[12..16].try_into().unwrap());
        assert_eq!(hi & !15, pc, "target {i}: the descriptor's top word");
    }

    /* DCD1: the sample mask and the targets the shader writes */
    let r58 = cs_word(trace, "MOVE32 r58,") as u32;
    assert_eq!(ours.regs[&58], r58, "DCD1");
    let dcd1 = DcdFlags1::unpack(&r58.to_le_bytes()).unwrap();
    assert_eq!(dcd1.render_target_mask, outputs);

    /* the FBD pointer: its target count (Mesa's has the ZS/CRC bit too) */
    let theirs_fbd = cs_word(trace, "MOVE48 d40,").to_le_bytes();
    let theirs_fbd = FramebufferPointer::unpack(&theirs_fbd).unwrap();
    let ours_fbd = FramebufferPointer::unpack(&ours.frame.fbd_pointer.to_le_bytes()).unwrap();
    assert_eq!(ours_fbd.render_target_count, theirs_fbd.render_target_count);
    assert_eq!(ours_fbd.render_target_count, n as u32);
    assert!(theirs_fbd.zs_crc_extension_present && !ours_fbd.zs_crc_extension_present, "the named difference");

    /* the framebuffer parameters, field by field */
    let fbd = ours.frame.layout.fbd.expect("an FBD");
    let p = FramebufferParameters::unpack(ours.at(fbd.gpu, 64).try_into().unwrap()).unwrap();
    let mut f = block(trace, "Framebuffer @", "Parameters:");
    let never = |v: String| assert_eq!(v, "Never");
    never(take(&mut f, "Pre Frame 0"));
    never(take(&mut f, "Pre Frame 1"));
    never(take(&mut f, "Post Frame"));
    assert_eq!(p.pre_frame_0, PrePostFrameShaderMode::NEVER);
    assert_eq!(p.pre_frame_1, PrePostFrameShaderMode::NEVER);
    assert_eq!(p.post_frame, PrePostFrameShaderMode::NEVER);
    assert_eq!(take(&mut f, "Downscale geometry"), "None");
    assert_eq!(p.downscale_geometry, Downscale::NONE);
    assert_eq!(take(&mut f, "Sample Pattern"), "Single-sampled");
    assert_eq!(p.sample_pattern, SamplePattern::SINGLE_SAMPLED);
    assert_eq!(take(&mut f, "Tie-Break Rule"), "MINUS_180_IN_0_OUT");
    assert_eq!(p.tie_break_rule, TieBreakRule::MINUS_180_IN_0_OUT);
    assert_eq!(take(&mut f, "Z Internal Format"), "D24");
    assert_eq!(p.z_internal_format, ZInternalFormat::D24);
    assert_eq!(take(&mut f, "Z Clear"), "0.000000");
    assert_eq!(p.z_clear, 0.0);
    for (k, v) in [
        ("Internal layer index", u64::from(p.internal_layer_index)),
        ("Frame argument", p.frame_argument),
        ("Sample Locations", p.sample_locations),
        ("Frame Shader DCDs", p.frame_shader_dcds),
        ("Width", u64::from(p.width)),
        ("Height", u64::from(p.height)),
        ("Bound Min X", u64::from(p.bound_min_x)),
        ("Bound Min Y", u64::from(p.bound_min_y)),
        ("Bound Max X", u64::from(p.bound_max_x)),
        ("Bound Max Y", u64::from(p.bound_max_y)),
        ("Sample Count", u64::from(p.sample_count)),
        ("Effective Tile Size", u64::from(p.effective_tile_size)),
        ("Render Target Count", u64::from(p.render_target_count)),
        ("Color Buffer Allocation", u64::from(p.color_buffer_allocation)),
        ("S Clear", u64::from(p.s_clear)),
    ] {
        assert_eq!(num(&mut f, k), v, "FBD {k}");
    }
    for (k, v) in [
        ("First provoking vertex", p.first_provoking_vertex),
        ("Point sprite coord origin max Y", p.point_sprite_coord_origin_max_y),
        ("Blend suppress Inf", p.blend_suppress_inf),
        ("Blend suppress NaN", p.blend_suppress_nan),
        ("Blend suppress denorm", p.blend_suppress_denorm),
        ("S Write Enable", p.s_write_enable),
        ("S Preload Enable", p.s_preload_enable),
        ("S Unload Enable", p.s_unload_enable),
        ("Z Write Enable", p.z_write_enable),
        ("Z Preload Enable", p.z_preload_enable),
        ("Z Unload Enable", p.z_unload_enable),
        ("Empty Tile Read Enable", p.empty_tile_read_enable),
    ] {
        assert_eq!(flag(&mut f, k), v, "FBD {k}");
    }
    /* the named differences: Mesa's AFBC targets keep a CRC */
    for k in ["Has ZS CRC Extension", "Empty Tile Write Enable", "CRC Read Enable", "CRC Write Enable"] {
        assert!(flag(&mut f, k), "Mesa's {k}");
    }
    assert!(!p.has_zs_crc_extension && !p.empty_tile_write_enable && !p.crc_read_enable && !p.crc_write_enable);
    /* where each builder put its tiler context */
    take(&mut f, "Tiler");
    done(&f, "FBD parameters");

    /* the render targets, after the 64 bytes of the ZS/CRC extension room */
    let mut offset = 0;
    for i in 0..n {
        let at = fbd.gpu + 128 + i * RgbRenderTarget::LENGTH as u64;
        let mut f = block(trace, "Color Render Targets @", &format!("AFBC RGB Color Render Target {i}:"));
        let colour = |f: &mut Fields, c: usize| num(f, &format!("Clear/Color {c}")) as u32;
        let clear = RtClear {
            color_0: colour(&mut f, 0),
            color_1: colour(&mut f, 1),
            color_2: colour(&mut f, 2),
            color_3: colour(&mut f, 3),
        };
        assert_eq!(take(&mut f, "Writeback Mode"), "Compat");
        assert_eq!(take(&mut f, "Internal Format"), "R8G8B8A8");
        assert_eq!(take(&mut f, "Writeback Format"), "R8G8B8A8");
        assert_eq!(take(&mut f, "Writeback MSAA"), "Single");
        assert_eq!(take(&mut f, "Swizzle"), "1672 (RGBA)");
        let mesa_offset = num(&mut f, "Internal Buffer Offset") as u32;
        assert_eq!(mesa_offset, offset, "target {i}: the tile-buffer offset");
        let expected = RgbRenderTarget {
            writeback_mode: WritebackMode::COMPAT,
            internal_buffer_offset: mesa_offset,
            yuv_enable: flag(&mut f, "YUV Enable"),
            dithered_clear: flag(&mut f, "Dithered Clear"),
            internal_format: ColorBufferInternalFormat::R8G8B8A8,
            write_enable: flag(&mut f, "Write Enable"),
            writeback_format: ColorFormat::R8G8B8A8,
            writeback_msaa: Msaa::SINGLE,
            srgb: flag(&mut f, "sRGB"),
            dithering_enable: flag(&mut f, "Dithering Enable"),
            swizzle: 1672,
            clean_tile_write_enable: flag(&mut f, "Clean Tile Write Enable"),
            clear,
            /* the named difference: linear, the target's memory, where
             * Mesa's is AFBC (below) */
            writeback_block_format: BlockFormat::LINEAR,
            writeback_buffer: RtBuffer { base: target(i).gpu, row_stride: 256, surface_stride: 0 },
            ..Default::default()
        };
        assert_eq!(ours.at(at, RgbRenderTarget::LENGTH), expected.pack().unwrap(), "render target {i}");
        assert_eq!(take(&mut f, "Writeback Block Format"), "AFBC");
        for k in [
            "Writeback Float Format",
            "YUV Transform",
            "Split block",
            "Wide block",
            "Reverse issue order",
            "Front buffer",
            "Alpha hint",
            "Compression mode",
            "Header",
            "Row stride",
            "Body offset",
        ] {
            take(&mut f, k);
        }
        done(&f, "render target");
        offset += 4 * 1024;
    }
}

/// Case 55's frame: two RGBA8 targets.
#[test]
fn two_targets_are_mesa_s() {
    compare(MRT2, 2);
}

/// Case 106's frame: three RGBA8 targets.
#[test]
fn three_targets_are_mesa_s() {
    compare(MRT3, 3);
}

/// A program that writes past the frame's targets, or not output 0, is
/// refused by name; so is what a frame of several targets does not build.
#[test]
fn what_several_targets_do_not_build_is_refused_by_name() {
    let refused = |r: Result<Built, FrameError>| match r {
        Err(FrameError::NotBuilt(why)) => why,
        Err(e) => panic!("{e}"),
        Ok(_) => panic!("built"),
    };
    assert_eq!(refused(build(2, 0x7)), NOT_BUILT_FS_OUTPUTS);
    assert_eq!(refused(build(2, 0x2)), NOT_BUILT_FS_OUTPUTS);
    assert_eq!(refused(build(1, 0x3)), NOT_BUILT_FS_OUTPUTS);
    assert_eq!(refused(build(9, 0x1)), NOT_BUILT_MRT);
    /* a program that writes target 0 only, into a frame of two: built,
     * the mask is the shader's */
    let one = build(2, 0x1).expect("built");
    assert_eq!(DcdFlags1::unpack(&one.regs[&58].to_le_bytes()).unwrap().render_target_mask, 1);
    assert_eq!(one.d(50) & 63, 2);

    let mut desc_m = vec![0u8; 0x1000];
    let mut words = vec![0u64; 64];
    let mut desc = Arena::new("desc", DESC, &mut desc_m).unwrap();
    let mut cs = Stream::new(&mut words, 92);
    let ctx = Context {
        heap: 0,
        geometry_buffer: 0,
        geometry_buffer_size: GEOMETRY_BUFFER_BYTES,
        sample_positions: 0x7fff_ffff_f000,
        tiler_max_levels: 8,
    };
    let small = Target { width: 32, ..target(1) };
    let open = begin_frame_targets(&mut desc, &mut cs, &[small]).unwrap();
    let e = end_frame(&open, &mut desc, &mut cs, &ctx, &target(0), [0.0; 4], None);
    assert!(matches!(e, Err(FrameError::Invalid(INVALID_MRT_SIZE))));
    let mut open = begin_frame_targets(&mut desc, &mut cs, &[target(1)]).unwrap();
    open.colour_preload = Some(FrameShader { binary: 0x7fff_ffff_c000, preload: 0 });
    let e = end_frame(&open, &mut desc, &mut cs, &ctx, &target(0), [0.0; 4], None);
    assert!(matches!(e, Err(FrameError::NotBuilt(NOT_BUILT_MRT))));
    let four = Target { samples: 4, ..target(0) };
    let open = begin_frame_targets(&mut desc, &mut cs, &[Target { samples: 4, ..target(1) }]).unwrap();
    let e = end_frame(&open, &mut desc, &mut cs, &ctx, &four, [0.0; 4], None);
    assert!(matches!(e, Err(FrameError::NotBuilt(NOT_BUILT_MRT))));
}
