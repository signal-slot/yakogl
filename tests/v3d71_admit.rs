// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `src/v3d/admit71.rs`, the 7.1 table's admission and named refusals for a
//! frame and its draws, over the merged pure modules `ident71`, `limits71`,
//! `rt71`, `attr71`, `record71` and `format71`.
//!
//! * (a) the shapes of gl_test cases 5, 58, 59, 66, 67 and 102 are admitted,
//!   written by hand from Mesa's 7.1 references `tools/v3d/pi5/gl_*.clif`
//!   (the GL calls are `tools/v3d/pi5/src/gl_shapes.py`; each description
//!   cites the dump's line), plus the mrt4 and mrt8 frames;
//! * (b) every refusal the modules have is reached once with its exact name:
//!   9 targets, 7681 px, IPREV < 10 (and a 4.2 part), a format rt71 or
//!   format71 refuses, an attribute attr71 refuses and a record record71
//!   refuses, and limits71's "no tile fits";
//! * (c) the board's hub identity (`tools/v3d/pi5/v3d_ident.txt`,
//!   `driver/docs/rpi5-v3d71-facts.md`: `HUB_IDENT1` 0x00081117,
//!   `HUB_IDENT3` 0x00020a10) is admitted.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_admit
//! ```
//! (the modules and this test use no crate feature.)
#[path = "../src/v3d/admit71.rs"]
#[allow(dead_code)]
mod admit71;
#[path = "../src/v3d/attr71.rs"]
#[allow(dead_code)]
mod attr71;
#[path = "../src/v3d/format71.rs"]
#[allow(dead_code)]
mod format71;
#[path = "../src/v3d/ident71.rs"]
#[allow(dead_code)]
mod ident71;
#[path = "../src/v3d/limits71.rs"]
#[allow(dead_code)]
mod limits71;
#[path = "../src/v3d/record71.rs"]
mod record71;
#[path = "../src/v3d/rt71.rs"]
#[allow(dead_code)]
mod rt71;

use admit71::*;
use record71::{DrawDesc, FragmentDesc, ProgramDesc, VertexDesc, BCM2712_D0};
use rt71::ClearColor;

const RGBA8: u32 = 0x8058;
const RGB8: u32 = 0x8051;
const D24S8: u32 = 0x88F0;
const D16: u32 = 0x81A5;
const S8: u32 = 0x8D48;

fn refusal(module: &'static str, reason: &str) -> Refusal {
    Refusal { module, reason: reason.to_string() }
}

// ---- frames ----------------------------------------------------------------

const KEEP: BufferUse = BufferUse { load: true, store: true, clear: false };
const CLEARED: BufferUse = BufferUse { load: false, store: true, clear: true };
const NONE: BufferUse = BufferUse { load: false, store: false, clear: false };

fn rgba8(uses: BufferUse, clear: Option<[f32; 4]>) -> ColorTarget {
    ColorTarget { gl_format: RGBA8, uses, clear_color: clear.map(ClearColor::Float), sampled: false }
}

fn frame<'a>(w: u32, h: u32, colors: &'a [ColorTarget], ds: Option<DepthStencil>) -> Frame<'a> {
    Frame { width: w, height: h, colors, depth_stencil: ds, msaa: false, double_buffer: false }
}

/// gl_test's window: an RGBA8 target and a D24S8 buffer, 64x64
/// (gl_shapes.py `window`; gl_scissor.clif:15 "tile width 64 pixels",
/// :17 "Width (in pixels): 64", :32 "Number of Render Targets: 1",
/// :40 "Internal Depth Type: depth_24").
fn window(colors: &[ColorTarget]) -> Result<FrameAdmitted, Refusal> {
    let ds = DepthStencil { gl_format: D24S8, depth: CLEARED, stencil: CLEARED };
    admit_frame(&frame(64, 64, colors, Some(ds)))
}

#[test]
fn case5_scissor() {
    // glClear of the window (black), then a scissored clear and a quad
    // (gl_scissor.clif: two jobs, :32 and :328, one render target each)
    let ok = window(&[rgba8(CLEARED, Some([0.0, 0.0, 0.0, 1.0]))]).unwrap();
    assert_eq!(ok.tile, (64, 64));
    assert_eq!(ok.color_bytes, 4);
    // the second job loads what the first stored (gl_scissor.clif's tile-buffer load)
    window(&[rgba8(KEEP, None)]).unwrap();
}

#[test]
fn case58_scisbox() {
    // gl_scisbox.clif: blue clear, scissored red clear, a green quad; the
    // same window, with an empty scissor box in the last job
    window(&[rgba8(CLEARED, Some([0.0, 0.0, 1.0, 1.0]))]).unwrap();
    window(&[rgba8(KEEP, None)]).unwrap();
}

#[test]
fn case59_writemask() {
    // gl_writemask.clif: colour, depth and stencil cleared, depth-tested and
    // stencil-tested draws, the stencil kept into the next job
    let ds = DepthStencil { gl_format: D24S8, depth: KEEP, stencil: KEEP };
    let colors = [rgba8(KEEP, None)];
    let f = frame(64, 64, &colors, Some(ds));
    assert_eq!(admit_frame(&f).unwrap().tile, (64, 64));
}

#[test]
fn case66_viewport() {
    // gl_viewport.clif: white clear, blue clear at a quarter viewport, green
    // clear scissored to a quarter, on the window
    window(&[rgba8(CLEARED, Some([1.0, 1.0, 1.0, 1.0]))]).unwrap();
}

#[test]
fn case102_colormask() {
    // gl_colormask.clif: masked clears and a yellow quad on the window
    window(&[rgba8(CLEARED, Some([1.0, 0.0, 0.0, 1.0]))]).unwrap();
}

#[test]
fn case67_fbods_frames() {
    // gl_fbods.clif, frames of case_fbods (every one: 64 pixel wide tile,
    // 1 render target: :17, :227, :485, :837, :1064, :1466, ...)
    // frame 1: colour-only FBO texture, depth then stencil test
    let tex = ColorTarget { sampled: true, ..rgba8(CLEARED, Some([0.0, 0.0, 1.0, 1.0])) };
    assert_eq!(admit_frame(&frame(64, 64, &[tex], None)).unwrap().tile, (64, 64));
    // frame 2: a GL_RGB target cleared with alpha 0.25
    let rgb = ColorTarget { gl_format: RGB8, ..rgba8(CLEARED, Some([1.0, 0.0, 0.0, 0.25])) };
    admit_frame(&frame(64, 64, &[rgb], None)).unwrap();
    // frames 3 to 4: a GL_DEPTH_COMPONENT16 renderbuffer per FBO over one texture
    let d16 = DepthStencil { gl_format: D16, depth: CLEARED, stencil: NONE };
    admit_frame(&frame(64, 64, &[tex], Some(d16))).unwrap();
    // frames 5 to 8: a GL_STENCIL_INDEX8 FBO, scissored stencil-only clear,
    // alone and after a draw (and across glFinish, loading the stencil)
    let s8 = DepthStencil { gl_format: S8, depth: NONE, stencil: CLEARED };
    admit_frame(&frame(64, 64, &[tex], Some(s8))).unwrap();
    let s8_kept = DepthStencil { gl_format: S8, depth: NONE, stencil: KEEP };
    admit_frame(&frame(64, 64, &[tex], Some(s8_kept))).unwrap();
}

#[test]
fn mrt4_and_mrt8() {
    // gl_mrt4.clif:35 "Number of Render Targets: 4", :18 "tile width 64"
    // (64x32: 16 colour bytes a pixel); gl_mrt8.clif:39 "...: 8", :22
    // "tile width 32 pixels" (32x32: 32 bytes)
    let t = rgba8(CLEARED, Some([0.0, 0.0, 1.0, 1.0]));
    let a = admit_frame(&frame(64, 64, &[t; 4], None)).unwrap();
    assert_eq!((a.tile, a.color_bytes), ((64, 32), 16));
    let a = admit_frame(&frame(64, 64, &[t; 8], None)).unwrap();
    assert_eq!((a.tile, a.color_bytes), ((32, 32), 32));
}

// ---- draws -----------------------------------------------------------------

fn cuv_attrs() -> [attr71::Attr; 3] {
    // gl_test's 40-byte cuv vertex: pos4, color4, uv2 (gl_scissor.clif:530
    // to :562: vec size 4/4/2, float, stride 40; the first attribute read by
    // both shaders (4, 4), the others by the vertex shader (4, 2))
    let a = |components, addr| attr71::Attr {
        enabled: true,
        components,
        gl_type: attr71::GL_FLOAT,
        stride: 40,
        buffer_addr: addr,
        ..attr71::Attr::disabled()
    };
    [a(4, 0x1000), a(4, 0x1010), a(2, 0x1020)]
}

fn program(code: u32) -> VertexDesc {
    VertexDesc {
        prog: ProgramDesc { code, uniforms: code + 0x100, threads: 4, single_seg: true, propagate_nans: false },
        vpm_input_size: 0,
        vpm_output_size: 5,
        min_input_segments: 1,
        min_output_segments: 0,
        ..Default::default()
    }
}

fn quad_draw(attrs: &[attr71::Attr]) -> Draw<'_> {
    let mut cs_reads = [0u8; attr71::MAX_ATTRS];
    let mut vs_reads = [0u8; attr71::MAX_ATTRS];
    cs_reads[0] = 4;
    vs_reads[..3].copy_from_slice(&[4, 4, 2]);
    Draw {
        attrs,
        cs_reads,
        vs_reads,
        cs_builtins: false,
        dummy_addr: 0x2000,
        separate_segments: false,
        programs: DrawDesc {
            fs: FragmentDesc {
                prog: ProgramDesc {
                    code: 0x1060,
                    uniforms: 0x13000,
                    threads: 4,
                    single_seg: false,
                    propagate_nans: false,
                },
                num_varyings: 6,
                ..Default::default()
            },
            vs: program(0x1090),
            cs: program(0x1108),
            point_size: false,
        },
    }
}

#[test]
fn the_gl_test_quad_is_admitted() {
    let attrs = cuv_attrs();
    let ok = admit_draw(BCM2712_D0, &quad_draw(&attrs)).unwrap();
    // gl_scissor.clif:313 "number of attribute arrays: 3"
    assert_eq!(ok.attrs.count, 3);
    assert_eq!(ok.attrs.records[0].stride, 40);
    assert!(ok.record.enable_clipping);
}

#[test]
fn draw_without_attributes_gets_the_dummy_record() {
    let mut d = quad_draw(&[]);
    d.cs_reads = [0; attr71::MAX_ATTRS];
    d.vs_reads = [0; attr71::MAX_ATTRS];
    assert_eq!(admit_draw(BCM2712_D0, &d).unwrap().attrs.count, 1);
}

// ---- (b) the refusals, each once --------------------------------------------

#[test]
fn nine_targets() {
    let t = rgba8(CLEARED, None);
    let e = admit_frame(&frame(64, 64, &[t; 9], None)).unwrap_err();
    assert_eq!(e, refusal("rt71", "v3d 7.1: more than 8 render targets"));
    assert_eq!(e.to_string(), "rt71: v3d 7.1: more than 8 render targets");
}

#[test]
fn size_7681_and_empty() {
    let t = [rgba8(CLEARED, None)];
    assert!(admit_frame(&frame(7680, 7680, &t[..0], None)).is_ok());
    for (w, h) in [(7681, 64), (64, 7681)] {
        let e = admit_frame(&frame(w, h, &t, None)).unwrap_err();
        assert_eq!(e, refusal("rt71", "v3d 7.1: a render target larger than 7680 px"));
    }
    let e = admit_frame(&frame(0, 64, &t, None)).unwrap_err();
    assert_eq!(e, refusal("rt71", "v3d 7.1: an empty render target"));
}

#[test]
fn iprev_below_10_and_other_parts() {
    // HUB_IDENT1 7.1 (TVER 7, REV 1); HUB_IDENT3 IPREV 9
    let e = admit_ident(0x0008_1117, 0x0002_0910).unwrap_err();
    assert_eq!(e, refusal("ident71", "V3D 7.1.9: this table is 7.1.10 (BCM2712 D0) only"));
    // a Pi 4: TVER 4, REV 2
    let e = admit_ident(0x0008_1124, 0x0000_1900).unwrap_err();
    assert_eq!(e, refusal("ident71", "V3D 4.2: build gpu-v3d"));
    let e = admit_ident(0x0008_1133, 0).unwrap_err();
    assert_eq!(e.module, "ident71");
    assert!(e.reason.contains("unknown version"));
}

#[test]
fn colour_formats_rt71_refuses() {
    let bad = |f| ColorTarget { gl_format: f, ..rgba8(CLEARED, None) };
    let run = |f| admit_frame(&frame(64, 64, &[bad(f)], None)).unwrap_err();
    // RGBA8_SNORM: no render-target row
    assert_eq!(run(0x8F97), refusal("rt71", "v3d 7.1: this colour format has no render-target row in Mesa's table"));
    // a depth format as a colour target
    assert_eq!(run(D24S8), refusal("rt71", "v3d 7.1: a depth or stencil format is not a colour target"));
    assert_eq!(run(0x1234), refusal("rt71", "v3d 7.1: an unknown colour format"));
}

#[test]
fn clear_kind_mismatch() {
    let t = ColorTarget { clear_color: Some(ClearColor::Int([1, 2, 3, 4])), ..rgba8(CLEARED, None) };
    let e = admit_frame(&frame(64, 64, &[t], None)).unwrap_err();
    assert_eq!(e, refusal("rt71", "v3d 7.1: the clear colour's kind does not match the target's"));
}

#[test]
fn depth_stencil_format_must_be_depth_or_stencil() {
    let ds = DepthStencil { gl_format: RGBA8, depth: CLEARED, stencil: NONE };
    let e = admit_frame(&frame(64, 64, &[rgba8(CLEARED, None)], Some(ds))).unwrap_err();
    assert_eq!(e, refusal("admit71", "a colour format is not a depth or stencil format"));
    let ds = DepthStencil { gl_format: 0x1234, depth: CLEARED, stencil: NONE };
    let e = admit_frame(&frame(64, 64, &[rgba8(CLEARED, None)], Some(ds))).unwrap_err();
    assert_eq!(e, refusal("rt71", "v3d 7.1: an unknown colour format"));
}

#[test]
fn texture_formats_format71_refuses() {
    // RGB16F: Mesa has no three-channel row
    let e = admit_texture(0x881B, 0).unwrap_err();
    assert_eq!(e, refusal("format71", "NoThreeChannelRow"));
    let e = admit_texture(0x8D48, 0).unwrap_err();
    assert_eq!(e, refusal("format71", "StencilTexturing"));
    assert!(admit_texture(RGBA8, 0).is_ok());
    // a sampled target goes through the same answer (RGBA8 has one)
    let t = ColorTarget { sampled: true, ..rgba8(CLEARED, None) };
    assert!(admit_frame(&frame(64, 64, &[t], None)).is_ok());
}

#[test]
fn no_tile_fits() {
    // eight 128-bit targets under 4x MSAA fit 8x8 exactly (512 bytes a
    // pixel); double-buffered they need a tile below 8x8, which limits71
    // answers with None
    let wide = ColorTarget { gl_format: 0x8814, ..rgba8(CLEARED, None) };
    let targets = [wide; 8];
    let mut f = frame(64, 64, &targets, None);
    f.msaa = true;
    assert_eq!(admit_frame(&f).unwrap().tile, (8, 8));
    f.double_buffer = true;
    let e = admit_frame(&f).unwrap_err();
    assert_eq!(e, refusal("limits71", "no tile size fits 128 colour bytes per pixel"));
}

#[test]
fn attributes_attr71_refuses() {
    // a program reads a disabled attribute and no constant slot is given
    let mut attrs = cuv_attrs();
    attrs[1] = attr71::Attr::disabled();
    let e = admit_draw(BCM2712_D0, &quad_draw(&attrs)).unwrap_err();
    assert_eq!(e.module, "attr71");
    assert_eq!(e.reason, attr71::Refusal71::ConstantAttribute.message());
    assert_eq!(
        e.reason,
        "V3D 7.1: a constant vertex attribute the program reads has no 16-byte slot to read from (no default-attribute BO)"
    );
    // a GL type with no 7.1 attribute type
    let mut attrs = cuv_attrs();
    attrs[0].gl_type = attr71::GL_DOUBLE;
    let e = admit_draw(BCM2712_D0, &quad_draw(&attrs)).unwrap_err();
    assert_eq!(e, refusal("attr71", "V3D 7.1: this vertex attribute type has no 7.1 attribute type"));
    // reads of an attribute that has no record
    let mut d = quad_draw(&[]);
    d.vs_reads[2] = 4;
    let e = admit_draw(BCM2712_D0, &d).unwrap_err();
    assert_eq!(e, refusal("attr71", "V3D 7.1: a program reads an attribute that has no record"));
    // separate VPM segments are not built
    let attrs = cuv_attrs();
    let mut d = quad_draw(&attrs);
    d.separate_segments = true;
    let e = admit_draw(BCM2712_D0, &d).unwrap_err();
    assert_eq!(e, refusal("attr71", "V3D 7.1: a shader with separate input and output VPM segments is not built"));
}

#[test]
fn records_record71_refuses() {
    let attrs = cuv_attrs();
    // gl_DrawID (u11 s.4 item 3)
    let mut d = quad_draw(&attrs);
    d.programs.vs.uses_draw_index = true;
    let e = admit_draw(BCM2712_D0, &d).unwrap_err();
    assert_eq!(e, refusal("record71", "the vertex shader reads gl_DrawID: refused"));
    // gl_BaseVertex in the coordinate shader
    let mut d = quad_draw(&attrs);
    d.programs.cs.uses_base_vertex = true;
    let e = admit_draw(BCM2712_D0, &d).unwrap_err();
    assert_eq!(e, refusal("record71", "the coordinate shader reads gl_BaseVertex: refused"));
    // a 7.1 part before 7.1.10 has no draw-index record
    let d = quad_draw(&attrs);
    let e = admit_draw(record71::Revision { ver: 71, rev: 5 }, &d).unwrap_err();
    assert_eq!(e, refusal("record71", "V3D 7.1.5 has no GL Shader State Record Draw Index (7.1.10 or later 7.1 only)"));
    // an unaligned code address
    let mut d = quad_draw(&attrs);
    d.programs.fs.prog.code = 0x1061;
    let e = admit_draw(BCM2712_D0, &d).unwrap_err();
    assert_eq!(e, refusal("record71", "the fragment shader's code address is not 8-byte aligned"));
}

// ---- (c) the board's identity ------------------------------------------------

#[test]
fn the_boards_hub_identity_is_admitted() {
    // driver/docs/rpi5-v3d71-facts.md:538: HUB_IDENT1 0x00081117,
    // HUB_IDENT3 0x00020a10 (v3d_ident.txt "Revision: 7.1.10.16")
    let id = admit_ident(0x0008_1117, 0x0002_0a10).unwrap();
    assert_eq!((id.ver, id.iprev, id.compat_rev), (71, 10, 2));
}
