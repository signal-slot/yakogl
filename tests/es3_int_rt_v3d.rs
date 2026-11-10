// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 4.2's half of ES 3.0's integer and float colour render targets
//! (int-rt-v3d), on the host: the TLB render-target rows the back end hands
//! the job builder for every colour surface (`src/v3d/state.rs`
//! `rt_format`, Mesa's `v3dx_format_table.c` and `v3dx_rcl.c`), the clear
//! words the tile buffer starts from (`clear_words`, `v3d_tlb_clear`), the
//! surfaces the GL core may draw into (`backend::INTEGER_RT_FORMATS`), and
//! the compiler's TLB writes of an integer fragment output: 32-bit words
//! with Mesa's 32-bit config (`select.rs` `tlb_config_32b`,
//! vir_emit_tlb_color_write's 32-bit branch), run on the QPU interpreter.
//! The draw path (the typed job, its strides, clears and refusals) is in
//! `src/v3d/backend_tests.rs`, which drives the back end against its fake
//! job builder.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_int_rt_v3d
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use std::sync::Mutex;
use yakogl::backend::{self, *};
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target};
use yakogl::shaderc::online::v3d::select::{tlb_config_32b, tlb_config_f16};
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, LANES};
use yakogl::shaderc::online::{lower, sema};
use yakogl::v3d::ffi::*;
use yakogl::v3d::state;

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

/* ---- the render-target rows --------------------------------------------- */

/// Every surface of ES 3.0 table 3.13's integer rows, as the GL core numbers
/// them (`GLES_SURF_INT_FIRST..=GLES_SURF_INT_LAST`).
fn integer_surfaces() -> impl Iterator<Item = u8> {
    GLES_SURF_INT_FIRST..=GLES_SURF_INT_LAST
}

/// The GL core may draw every integer surface into the V3D
/// (`INTEGER_RT_FORMATS` has all nineteen bits), and the sRGB one too
/// (`SRGB_RT`, docs/es3/srgb.md 5.1).
#[test]
fn every_integer_surface_is_drawable() {
    assert_eq!(backend::INTEGER_RT_FORMATS, (1 << 19) - 1);
    for s in integer_surfaces() {
        assert!(backend::rt_format_drawable(s), "{}", gles_surf_name(s));
        assert!(state::rt_format(s).is_some(), "{}: a TLB row", gles_surf_name(s));
    }
    assert!(backend::rt_format_drawable(GLES_SURF_SRGB8_A8));
    assert!(state::rt_format(GLES_SURF_SRGB8_A8).is_some());
}

/// Each row is Mesa's (v3dx_format_table.c `FORMAT(...)` rt column, the
/// internal type and bpp of `v3dX(get_internal_type_bpp_for_output_format)`
/// :220-341, the INT clamp of `clamp_for_format_and_type` v3dx_rcl.c:396-407):
/// the output image format of the format's own name, the internal type of
/// its channel width and signedness, the bpp from the channels' total width
/// (RGB10_A2UI held as 16UI), the INT clamp, and the GL core's bytes a pixel.
#[test]
fn integer_rows_follow_mesas_format_table() {
    /* (surface, output image format, internal type) */
    let rows: [(u8, u32, u32); 19] = [
        (GLES_SURF_R8I, V3D_OUTPUT_IMAGE_FORMAT_R8I, V3D_INTERNAL_TYPE_8I),
        (GLES_SURF_R8UI, V3D_OUTPUT_IMAGE_FORMAT_R8UI, V3D_INTERNAL_TYPE_8UI),
        (GLES_SURF_R16I, V3D_OUTPUT_IMAGE_FORMAT_R16I, V3D_INTERNAL_TYPE_16I),
        (GLES_SURF_R16UI, V3D_OUTPUT_IMAGE_FORMAT_R16UI, V3D_INTERNAL_TYPE_16UI),
        (GLES_SURF_R32I, V3D_OUTPUT_IMAGE_FORMAT_R32I, V3D_INTERNAL_TYPE_32I),
        (GLES_SURF_R32UI, V3D_OUTPUT_IMAGE_FORMAT_R32UI, V3D_INTERNAL_TYPE_32UI),
        (GLES_SURF_RG8I, V3D_OUTPUT_IMAGE_FORMAT_RG8I, V3D_INTERNAL_TYPE_8I),
        (GLES_SURF_RG8UI, V3D_OUTPUT_IMAGE_FORMAT_RG8UI, V3D_INTERNAL_TYPE_8UI),
        (GLES_SURF_RG16I, V3D_OUTPUT_IMAGE_FORMAT_RG16I, V3D_INTERNAL_TYPE_16I),
        (GLES_SURF_RG16UI, V3D_OUTPUT_IMAGE_FORMAT_RG16UI, V3D_INTERNAL_TYPE_16UI),
        (GLES_SURF_RG32I, V3D_OUTPUT_IMAGE_FORMAT_RG32I, V3D_INTERNAL_TYPE_32I),
        (GLES_SURF_RG32UI, V3D_OUTPUT_IMAGE_FORMAT_RG32UI, V3D_INTERNAL_TYPE_32UI),
        (GLES_SURF_RGBA8I, V3D_OUTPUT_IMAGE_FORMAT_RGBA8I, V3D_INTERNAL_TYPE_8I),
        (GLES_SURF_RGBA8UI, V3D_OUTPUT_IMAGE_FORMAT_RGBA8UI, V3D_INTERNAL_TYPE_8UI),
        (GLES_SURF_RGBA16I, V3D_OUTPUT_IMAGE_FORMAT_RGBA16I, V3D_INTERNAL_TYPE_16I),
        (GLES_SURF_RGBA16UI, V3D_OUTPUT_IMAGE_FORMAT_RGBA16UI, V3D_INTERNAL_TYPE_16UI),
        (GLES_SURF_RGBA32I, V3D_OUTPUT_IMAGE_FORMAT_RGBA32I, V3D_INTERNAL_TYPE_32I),
        (GLES_SURF_RGBA32UI, V3D_OUTPUT_IMAGE_FORMAT_RGBA32UI, V3D_INTERNAL_TYPE_32UI),
        (GLES_SURF_RGB10_A2UI, V3D_OUTPUT_IMAGE_FORMAT_RGB10_A2UI, V3D_INTERNAL_TYPE_16UI),
    ];
    for (surf, output, ty) in rows {
        let name = gles_surf_name(surf);
        let r = state::rt_format(surf).expect(name);
        let l = gles_surf_int_layout(surf).expect(name);
        assert_eq!((r.output, r.internal_type), (output, ty), "{name}");
        assert_eq!(r.clamp, V3D_RENDER_TARGET_CLAMP_INT, "{name}: clamp to the target's range");
        assert_eq!(r.bytes, gles_surf_bytes_per_pixel(surf), "{name}: bytes a pixel");
        /* the tile buffer's width: the channels at their width (16 for the
         * 10-bit ones), rounded up to 32, 64 or 128 bits */
        let per = if l.bits == 10 { 16 } else { u32::from(l.bits) };
        let held = u32::from(l.channels) * per;
        let bpp = if held <= 32 {
            V3D_INTERNAL_BPP_32
        } else if held <= 64 {
            V3D_INTERNAL_BPP_64
        } else {
            V3D_INTERNAL_BPP_128
        };
        assert_eq!(r.internal_bpp, bpp, "{name}: internal bpp for {held} bits held");
        assert!(state::typed(surf), "{name}: a typed job");
    }
}

/// The float rows: RGBA16F and R16F are 16F at 64 bpp (R16F too, Mesa's
/// v3dx_format_table.c:263-270), R32F is 32F at 32 bpp; none clamps. The
/// ES 2.0 three stay untyped.
#[test]
fn float_rows_and_the_es2_formats() {
    let f = |s| state::rt_format(s).unwrap();
    assert_eq!((f(GLES_SURF_RGBA16F).output, f(GLES_SURF_RGBA16F).internal_type), (18, V3D_INTERNAL_TYPE_16F));
    assert_eq!((f(GLES_SURF_RGBA16F).internal_bpp, f(GLES_SURF_RGBA16F).bytes), (V3D_INTERNAL_BPP_64, 8));
    assert_eq!((f(GLES_SURF_R16F).output, f(GLES_SURF_R16F).internal_type), (20, V3D_INTERNAL_TYPE_16F));
    assert_eq!((f(GLES_SURF_R16F).internal_bpp, f(GLES_SURF_R16F).bytes), (V3D_INTERNAL_BPP_64, 2));
    assert_eq!((f(GLES_SURF_R32F).output, f(GLES_SURF_R32F).internal_type), (11, V3D_INTERNAL_TYPE_32F));
    assert_eq!((f(GLES_SURF_R32F).internal_bpp, f(GLES_SURF_R32F).bytes), (V3D_INTERNAL_BPP_32, 4));
    /* RGB10_A2 (Mesa :71): the TLB's RGB10_A2 over a 16F tile buffer, 4 bytes */
    let r = f(GLES_SURF_RGB10_A2);
    assert_eq!((r.output, r.internal_type), (V3D_OUTPUT_IMAGE_FORMAT_RGB10_A2, V3D_INTERNAL_TYPE_16F));
    assert_eq!((r.internal_bpp, r.bytes), (V3D_INTERNAL_BPP_64, 4));
    for s in [GLES_SURF_RGBA16F, GLES_SURF_R16F, GLES_SURF_R32F, GLES_SURF_RGB10_A2] {
        assert_eq!(f(s).clamp, V3D_RENDER_TARGET_CLAMP_NONE);
        assert!(state::typed(s));
    }
    for s in [GLES_SURF_RGBA8888, GLES_SURF_XRGB8888, GLES_SURF_RGB565] {
        assert!(!state::typed(s), "{}: the ES 2.0 job, byte for byte as before", gles_surf_name(s));
        assert_eq!(f(s).internal_type, V3D_INTERNAL_TYPE_8);
        assert_eq!(f(s).internal_bpp, V3D_INTERNAL_BPP_32);
    }
}

/// The clear words per internal type (`state::clear_words`, as
/// `v3d_tlb_clear` packs them, v3dx_draw.c:1706-1769): the raw integers of
/// glClearBuffer{i,ui}v narrowed to a byte or a half a channel, kept whole
/// at 32 bits; floats as halves; the ES 2.0 formats their RGBA8 word.
#[test]
fn clear_words_pack_per_internal_type() {
    let ints = [0x7f_u32, 0x80, -1i32 as u32, 0x1_0002].map(f32::from_bits);
    assert_eq!(state::clear_words(GLES_SURF_RGBA8UI, ints), [0x02ff_807f, 0, 0, 0]);
    assert_eq!(state::clear_words(GLES_SURF_R8I, ints), [0x02ff_807f, 0, 0, 0]);
    assert_eq!(state::clear_words(GLES_SURF_RGBA16I, ints), [0x0080_007f, 0x0002_ffff, 0, 0]);
    assert_eq!(state::clear_words(GLES_SURF_RGB10_A2UI, ints), [0x0080_007f, 0x0002_ffff, 0, 0]);
    assert_eq!(state::clear_words(GLES_SURF_RG32I, ints), ints.map(f32::to_bits));
    assert_eq!(state::clear_words(GLES_SURF_RGBA16F, [1.0, 2.0, 0.5, -1.0]), [0x4000_3c00, 0xbc00_3800, 0, 0]);
    /* a fixed-point surface over the 16F tile buffer clamps the clear to [0, 1], NaN to 0 */
    assert_eq!(state::clear_words(GLES_SURF_RGB10_A2, [2.0, -1.0, f32::NAN, 0.5]), [0x0000_3c00, 0x3800_0000, 0, 0]);
    assert_eq!(state::clear_words(GLES_SURF_R32F, [0.25, 0.0, 0.0, 1.0]), [0x3e80_0000, 0, 0, 0x3f80_0000]);
    assert_eq!(state::clear_words(GLES_SURF_RGBA8888, [0.0, 1.0, 0.0, 1.0]), [0xff00_ff00, 0, 0, 0]);
}

/* ---- the compiler: integer outputs to the TLB ----------------------------- */

const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";

/// What the fragment shader wrote to the TLB for one pixel (lane 0): each
/// write's config (`Some` for a TLBU write) and value.
fn run_fs(fs: &str) -> Vec<(Option<u32>, u32)> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(VS).expect("arena");
    let fs = a.alloc_str(fs).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| e.msg.to_string()).expect("the VS lowers");
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).map_err(|e| e.msg.to_string()).expect("the FS lowers");
    let p = compile_program(&v.module, &f.module, Prim::Triangles, Target { vpm_size: 65536 }, &a)
        .unwrap_or_else(|e| panic!("the program compiles: {e:?}"));
    let uniforms: Vec<u32> =
        p.fs.code
            .uniforms
            .iter()
            .map(|u| match u.contents {
                0 => u.data,
                c => panic!("uniform contents {c} not modelled by this test"),
            })
            .collect();
    let inputs = FsInputs {
        w: [1.0f32.to_bits(); LANES],
        w_centroid: [1.0f32.to_bits(); LANES],
        z: [0; LANES],
        x: [0; LANES],
        y: [0; LANES],
        back_facing: [false; LANES],
        varyings: Vec::new(),
    };
    assert_eq!(p.fs.num_inputs, 0, "the shaders here read no varying");
    let env = Env { threads: p.fs.threads, ..Env::default() };
    let o = sim::run(p.fs.code.words, &uniforms, &Inputs::Fragment(Box::new(inputs)), &env)
        .unwrap_or_else(|e| panic!("the fragment program runs: {e}"));
    o.tlb.iter().map(|w| (w.config, w.value[0].expect("a defined colour word"))).collect()
}

/// An `ivec4` output is four 32-bit words to the TLB as they are, the first
/// through TLBU with the 32-bit config for RT 0, vec4 (`tlb_config_32b`: per
/// pixel, RT 7 - 0, type F32 (0 << 6: Mesa 26.2.1 nir_to_vir.c:1974-1977
/// writes every `f32_color_rb` target, integer ones included, with
/// TLB_TYPE_F32_COLOR and never TLB_TYPE_I32_COLOR), size 4 - 1), the rest
/// through TLB: no f16 conversion, so -2 stays 0xfffffffe and 0x7fffffff
/// whole.
#[test]
fn an_ivec4_output_writes_four_raw_words() {
    let fs = "#version 300 es\nprecision highp int;\nprecision highp float;\nout ivec4 o;\n\
              void main() { o = ivec4(1, -2, 3, 0x7fffffff); }\n";
    let w = run_fs(fs);
    assert_eq!(tlb_config_32b(0, 4), 0xffff_ff3f);
    assert_eq!(w, [(Some(0xffff_ff3f), 1), (None, 0xffff_fffe), (None, 3), (None, 0x7fff_ffff)], "config | r, g, b, a");
}

/// A `uvec2` output writes two words (size 2 - 1), an `int` one; a `uint`
/// of 0xffffffff is the word itself.
#[test]
fn narrower_integer_outputs_write_their_components() {
    let fs = "#version 300 es\nprecision highp int;\nprecision highp float;\nout uvec2 o;\n\
              void main() { o = uvec2(0xffffffffu, 7u); }\n";
    assert_eq!(run_fs(fs), [(Some(tlb_config_32b(0, 2)), 0xffff_ffff), (None, 7)]);
    assert_eq!(tlb_config_32b(0, 2), 0xffff_ff3d);
    let fs = "#version 300 es\nprecision highp int;\nprecision highp float;\nout int o;\nvoid main() { o = -5; }\n";
    assert_eq!(run_fs(fs), [(Some(tlb_config_32b(0, 1)), -5i32 as u32)]);
    assert_eq!(tlb_config_32b(0, 1), 0xffff_ff3c);
}

/// A float output is what it was: two f16 pairs through TLB with no config
/// (RT 0's f16 vec4 config is the TLB's default, `tlb_config_f16`).
#[test]
fn a_float_output_still_writes_f16_pairs() {
    let fs = "#version 300 es\nprecision highp float;\nout vec4 o;\nvoid main() { o = vec4(1.0, 0.5, -2.0, 0.0); }\n";
    assert_eq!(tlb_config_f16(0, 4), !0);
    assert_eq!(run_fs(fs), [(None, 0x3800_3c00), (None, 0x0000_c000)]);
}

/// Two outputs of different kinds, in RT order: RT 0's float as f16 pairs,
/// then RT 1's `uvec4` as four words with the 32-bit config naming RT 1
/// (7 - 1 in bits 3..5).
#[test]
fn an_integer_output_beside_a_float_one_names_its_render_target() {
    let fs = "#version 300 es\nprecision highp int;\nprecision highp float;\n\
              layout(location = 0) out vec4 c;\nlayout(location = 1) out uvec4 u;\n\
              void main() { c = vec4(1.0); u = uvec4(10u, 20u, 30u, 40u); }\n";
    let w = run_fs(fs);
    assert_eq!(tlb_config_32b(1, 4), 0xffff_ff37);
    assert_eq!(
        w,
        [(None, 0x3c00_3c00), (None, 0x3c00_3c00), (Some(0xffff_ff37), 10), (None, 20), (None, 30), (None, 40)]
    );
}

/* ---- a scissored and a masked integer clear, through the GL API ------------- */

/// glClearBuffer{i,ui}v on an integer renderbuffer under glScissor and
/// glColorMask, through the real GL core and the real V3D back end (the job
/// builder stood in for, as in tests/v3d_copy_reasons.rs; the texture layout
/// code is not reached by a renderbuffer). The job builder records what the
/// back end hands it: each clear must arrive as a draw of the integer clear
/// program (32-bit TLB words, the raw clear words as the fragment block, the
/// GL's colour mask and scissor box as the quad's state), never as a refusal,
/// and the whole-target clear stays the TLB's. The host has no tile buffer, so
/// the cleared texels are checked in tests/v3d_clear.rs against a model of the
/// quad; here the draw that produces them is.
mod gl_api {
    use core::ffi::{c_char, c_int, c_void, CStr};
    use std::collections::BTreeMap;
    use std::ffi::CString;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::{Mutex, MutexGuard, Once, OnceLock};
    use yakogl::backend::{GLES_SURF_RGBA32UI, GLES_SURF_RGBA8888, GLES_SURF_SRGB8_A8};
    use yakogl::gl::consts::*;
    use yakogl::gl::context;
    use yakogl::gl::draw::glDrawArrays;
    use yakogl::gl::es3::*;
    use yakogl::gl::framebuffer::*;
    use yakogl::gl::program::*;
    use yakogl::gl::shader::*;
    use yakogl::gl::state::*;
    use yakogl::gl::texture::*;
    use yakogl::gl::types::*;
    use yakogl::gl::vertex::*;
    use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
    use yakogl::v3d::backend::{v3d_refused_draws, yakogl_backend_v3d};
    use yakogl::v3d::ffi::*;
    use yakogl::v3d::kick::V3dKick;
    use yakogl::v3d::state;

    /* ---- the job builder: records, draws nothing ---- */

    #[derive(Clone, Debug)]
    struct SeenDraw {
        st: V3dJobState,
        /// The fragment block: the clear colour as raw words.
        fs_storage: Vec<u32>,
        fs_uniforms: Vec<V3dUniform>,
        count: u32,
    }

    #[derive(Default)]
    struct Seen {
        jobs: Vec<V3dJobTarget>,
        clears: Vec<V3dJobClear>,
        draws: Vec<SeenDraw>,
        log: Vec<String>,
    }

    static SEEN: Mutex<Seen> =
        Mutex::new(Seen { jobs: Vec::new(), clears: Vec::new(), draws: Vec::new(), log: Vec::new() });

    fn seen<R>(f: impl FnOnce(&mut Seen) -> R) -> R {
        f(&mut SEEN.lock().unwrap_or_else(|e| e.into_inner()))
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_job_begin(
        j: *mut V3dJob,
        t: *const V3dJobTarget,
        bcl: *mut V3dCl,
        rcl: *mut V3dCl,
        ind: *mut V3dCl,
    ) -> bool {
        // SAFETY: the backend passes its live job, target and lists.
        let (j, t) = unsafe { (&mut *j, *t) };
        *j = V3dJob { bcl, rcl, ind, t, draws: 0, defaults: 0, broken: false, pack: V3dJobPack::ZERO };
        // SAFETY: as above.
        unsafe {
            (*bcl).used = 16;
            (*ind).used = 256;
        }
        seen(|s| s.jobs.push(t));
        t.width != 0 && t.height != 0
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
        // SAFETY: the backend passes its live job and a draw whose pointers live for the call.
        let (j, d) = unsafe { (&mut *j, &*d) };
        j.draws += 1;
        /// A slice the draw names, empty when it names none (an application
        /// draw with no fragment block hands a null pointer and a zero count).
        unsafe fn words<T: Clone>(p: *const T, n: u32) -> Vec<T> {
            if p.is_null() || n == 0 {
                return Vec::new();
            }
            // SAFETY: the caller's slice, as long as the draw says.
            unsafe { core::slice::from_raw_parts(p, n as usize) }.to_vec()
        }
        // SAFETY: the job's BCL is live; the draw's slices are as long as it says.
        unsafe {
            (*j.bcl).used += 120;
            let rec = SeenDraw {
                st: *d.st,
                fs_storage: words(d.fs_storage.words, d.fs_storage.count),
                fs_uniforms: words((*d.fs).uniforms, (*d.fs).num_uniforms),
                count: d.count,
            };
            seen(|s| s.draws.push(rec));
        }
        V3D_JOB_OK
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_job_end(
        j: *mut V3dJob,
        c: *const V3dJobClear,
        _tile_list_start: *mut u32,
        _tile_list_end: *mut u32,
    ) -> bool {
        // SAFETY: the backend passes its live job and clear; its lists are live.
        unsafe {
            seen(|s| s.clears.push(*c));
            (*(*j).bcl).used += 1;
            (*(*j).rcl).used = 200;
        }
        true
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_frame_tiling(width: u32, height: u32, t: *mut V3dTiling) {
        // SAFETY: the backend's out-parameter, passed on.
        unsafe { v3d_frame_tiling_bpp(width, height, 1, false, 0, t) }
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_frame_tiling_rts(width: u32, height: u32, rts: u32, t: *mut V3dTiling) {
        // SAFETY: the backend's out-parameter, passed on.
        unsafe { v3d_frame_tiling_bpp(width, height, rts, false, 0, t) }
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_frame_tiling_ms(width: u32, height: u32, rts: u32, msaa: bool, t: *mut V3dTiling) {
        // SAFETY: the backend's out-parameter, passed on.
        unsafe { v3d_frame_tiling_bpp(width, height, rts, msaa, 0, t) }
    }

    /// The tile-size table of v3d_frame.c, as the backend's unit tests fake it.
    #[no_mangle]
    unsafe extern "C" fn v3d_frame_tiling_bpp(
        width: u32,
        height: u32,
        rts: u32,
        msaa: bool,
        max_bpp: u32,
        t: *mut V3dTiling,
    ) {
        const SIZES: [(u32, u32); 7] = [(64, 64), (64, 32), (32, 32), (32, 16), (16, 16), (16, 8), (8, 8)];
        let idx = match rts {
            0 | 1 => 0,
            2 => 1,
            _ => 2,
        } + if msaa { 2 } else { 0 }
            + max_bpp.min(2) as usize;
        let (tw, th) = SIZES[idx];
        let tiles = V3dTiling {
            tile_w: tw,
            tile_h: th,
            tiles_x: width.div_ceil(tw),
            tiles_y: height.div_ceil(th),
            supertile_w: 1,
            supertile_h: 1,
            frame_w_supertiles: width.div_ceil(tw),
            frame_h_supertiles: height.div_ceil(th),
        };
        // SAFETY: the backend's out-parameter.
        unsafe { *t = tiles };
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_frame_alloc_sizes(t: *const V3dTiling, _d: u32, _page: u32, ta: *mut u32, ts: *mut u32) {
        // SAFETY: the backend's parameters.
        unsafe {
            let n = (*t).tiles_x * (*t).tiles_y;
            *ta = n * 128 + 8192;
            *ts = n * 256;
        }
    }

    extern "C" {
        fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
    }

    /// `driver/src/v3d/v3d_tex.c` and the Mesa tiling it includes, as a shared
    /// object of this test's own, reached through the handle only (RTLD_LOCAL).
    fn lib() -> *mut c_void {
        static L: OnceLock<usize> = OnceLock::new();
        *L.get_or_init(|| {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_copy_reasons");
            std::fs::create_dir_all(&dir).expect("the scratch directory");
            let so = dir.join(format!("v3d_tex-{}.so", std::process::id()));
            /* the compiler's own temporary files next to the object */
            let st = Command::new("cc")
                .current_dir(&root)
                .env("TMPDIR", &dir)
                .args(["-std=c11", "-O1", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror"])
                .args(["-Idriver/src/v3d", "-Idriver/src/v3d/generated"])
                .args(["-Ithird_party/mesa-26.2.1/src/broadcom", "-Ithird_party/mesa-26.2.1/src"])
                .arg("-Ithird_party/mesa-26.2.1/include")
                .args(["driver/src/v3d/v3d_tex.c", "driver/src/v3d/v3d_tiling_mesa.c", "-lm", "-o"])
                .arg(&so)
                .status()
                .expect("cc runs");
            assert!(st.success(), "driver/src/v3d/v3d_tex.c builds");
            let path = CString::new(so.to_str().expect("a utf-8 path")).unwrap();
            // SAFETY: a path of a library this test just built; RTLD_NOW.
            let h = unsafe { dlopen(path.as_ptr(), 2) };
            assert!(!h.is_null(), "the layout code loads");
            let _ = std::fs::remove_file(&so);
            h as usize
        }) as *mut c_void
    }

    fn sym(name: &str) -> *mut c_void {
        let c = CString::new(name).unwrap();
        // SAFETY: a handle `lib` made and a NUL-terminated name.
        let p = unsafe { dlsym(lib(), c.as_ptr()) };
        assert!(!p.is_null(), "{name} in v3d_tex.c");
        p
    }

    /// The trampolines: the library's `extern "C"` texture functions
    /// (src/v3d/ffi.rs), each handed to the real C.
    macro_rules! trampoline {
        ($name:ident ( $($arg:ident : $ty:ty),* ) $(-> $ret:ty)?) => {
            #[no_mangle]
            unsafe extern "C" fn $name($($arg: $ty),*) $(-> $ret)? {
                // SAFETY: the C function's own signature, as src/v3d/ffi.rs declares it.
                let f: unsafe extern "C" fn($($ty),*) $(-> $ret)? =
                    unsafe { std::mem::transmute(sym(stringify!($name))) };
                // SAFETY: the library's call, passed on.
                unsafe { f($($arg),*) }
            }
        };
    }

    /* The signatures are src/v3d/ffi.rs's, kept as written (see
     * tests/depth_resolve_v3d.rs: rustfmt releases wrap these differently). */
    #[rustfmt::skip]
    mod trampolines {
        use super::*;

        trampoline!(v3d_tex_layout(width: u32, height: u32, levels: u32, cpp: u32, l: *mut V3dTexLayout) -> bool);
        trampoline!(v3d_tex_layout_msaa(width: u32, height: u32, cpp: u32, l: *mut V3dTexLayout) -> bool);
        trampoline!(v3d_tex_store_level(image: *mut u8, l: *const V3dTexLayout, level: u32, src: *const u8, stride: u32));
        trampoline!(v3d_tex_tfu_pack(l: *const V3dTexLayout, ttype: u32, src: u32, stride: u32, dst: u32,
                                     j: *mut V3dTexTfuJob) -> bool);
        trampoline!(v3d_tex_tfu_pack_mipmap(l: *const V3dTexLayout, ttype: u32, image: u32, j: *mut V3dTexTfuJob) -> bool);
        trampoline!(v3d_tex_layer_stride(l: *const V3dTexLayout) -> u32);
        trampoline!(v3d_tex_layers_size(l: *const V3dTexLayout, layers: u32, out: *mut u32) -> bool);
        trampoline!(v3d_tex_store_box(image: *mut u8, l: *const V3dTexLayout, level: u32, x: u32, y: u32, w: u32, h: u32,
                                      src: *const u8, stride: u32) -> bool);
        trampoline!(v3d_tex_load_box(image: *const u8, l: *const V3dTexLayout, level: u32, x: u32, y: u32, w: u32, h: u32,
                                     dst: *mut u8, stride: u32) -> bool);
        trampoline!(v3d_tex_state_from_layout(s: *mut V3dTexState, l: *const V3dTexLayout));
        trampoline!(v3d_tex_pack_sampler(out: *mut u8, s: *const V3dSampler) -> bool);
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_tex_pack_state(out: *mut u8, s: *const V3dTexState) -> bool {
        // SAFETY: the C function's signature.
        let f: unsafe extern "C" fn(*mut u8, *const V3dTexState) -> bool =
            unsafe { std::mem::transmute(sym("v3d_tex_pack_state")) };
        // SAFETY: the library's call, passed on; `s` is its live state.
        unsafe { f(out, s) }
    }

    /* ---- the platform: memory that answers, jobs that finish at once ---- */

    struct Alloc {
        mem: Box<[u64]>,
        size: usize,
    }
    struct Platform {
        next: u64,
        allocs: BTreeMap<u64, Alloc>,
        submitted: usize,
    }
    static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

    fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
        let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
        f(g.get_or_insert_with(|| Platform { next: 0, allocs: BTreeMap::new(), submitted: 0 }))
    }

    unsafe extern "C" fn p_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
        with_platform(|p| {
            if size == 0 || size > 256 << 20 {
                return -12;
            }
            let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
            let gpu = 0x0100_0000 + p.next;
            p.next += (size as u64).div_ceil(4096) * 4096;
            let cpu = mem.as_mut_ptr() as *mut c_void;
            // SAFETY: the caller passes a writable descriptor.
            unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
            p.allocs.insert(gpu, Alloc { mem, size });
            0
        })
    }
    unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
        // SAFETY: the caller passes its descriptor.
        let m = unsafe { *mem };
        if !m.impl_.is_null() {
            with_platform(|p| p.allocs.remove(&m.gpu));
        }
        0
    }
    unsafe extern "C" fn p_gpu_write(mem: *mut OsalGpuMem, off: usize, src: *const c_void, n: usize) -> i32 {
        // SAFETY: the caller passes its descriptor.
        let m = unsafe { *mem };
        if m.cpu.is_null() || off.checked_add(n).is_none_or(|e| e > m.size) {
            return -22;
        }
        // SAFETY: inside the allocation (checked above).
        unsafe { core::ptr::copy_nonoverlapping(src as *const u8, (m.cpu as *mut u8).add(off), n) };
        0
    }
    unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}
    unsafe extern "C" fn p_submit(_kick: *const c_void, len: usize) -> i64 {
        if len != size_of::<V3dKick>() {
            return -22;
        }
        with_platform(|p| {
            p.submitted += 1;
            p.submitted as i64
        })
    }
    unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
        0
    }
    unsafe extern "C" fn p_completed() -> i64 {
        with_platform(|p| p.submitted as i64)
    }
    unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
        use yakogl::osal::{OSAL_QUERY_V3D_BIN_POOL, OSAL_QUERY_V3D_CPU_OF_GPU, OSAL_QUERY_V3D_VPM_BYTES};
        if what == OSAL_QUERY_V3D_VPM_BYTES && out_sz == 4 {
            // SAFETY: a u32 out-parameter.
            unsafe { *(out as *mut u32) = 65536 };
            return 0;
        }
        if what == OSAL_QUERY_V3D_CPU_OF_GPU && out_sz == size_of::<OsalV3dCpuOf>() {
            // SAFETY: the backend's in/out struct of that size.
            let q = unsafe { &mut *(out as *mut OsalV3dCpuOf) };
            return with_platform(|p| match p.allocs.range(..=q.gpu).next_back() {
                Some((base, a)) if q.gpu + q.bytes <= base + a.size as u64 => {
                    // SAFETY: inside the allocation.
                    q.cpu = unsafe { (a.mem.as_ptr() as *mut u8).add((q.gpu - base) as usize) } as *mut c_void;
                    0
                }
                _ => -22,
            });
        }
        if what == OSAL_QUERY_V3D_BIN_POOL {
            return -61;
        }
        -22
    }
    /// The library logs every line as `log("%s", line)` (`osal::log_str`).
    unsafe extern "C" fn p_log2(_fmt: *const c_char, line: *const c_char) {
        // SAFETY: the library passes a NUL-terminated line.
        let s = unsafe { CStr::from_ptr(line) }.to_string_lossy().into_owned();
        seen(|x| x.log.push(s));
    }
    unsafe extern "C" fn p_init(_platform_arg: *mut c_void) -> i32 {
        0
    }

    type LogFn = unsafe extern "C" fn(*const c_char, ...);

    fn osal() -> &'static OsalOps {
        // SAFETY: see p_log2: the library only ever calls it with ("%s", line).
        let log = unsafe { core::mem::transmute::<unsafe extern "C" fn(*const c_char, *const c_char), LogFn>(p_log2) };
        Box::leak(Box::new(OsalOps {
            init: Some(p_init),
            shutdown: None,
            gpu_alloc: Some(p_gpu_alloc),
            gpu_free: Some(p_gpu_free),
            mmu_map: None,
            map_phys: None,
            gpu_write: Some(p_gpu_write),
            gpu_read: None,
            cache_flush: Some(p_cache),
            cache_invd: Some(p_cache),
            submit: Some(p_submit),
            wait: Some(p_wait),
            completed: Some(p_completed),
            reg_read: None,
            reg_write: None,
            query: Some(p_query),
            log: Some(log),
            wait_vblank: None,
            present: None,
            lock: None,
            unlock: None,
            thread_id: None,
            display_busy: None,
        }))
    }

    /* ---- the context ---- */

    static LOCK: Mutex<()> = Mutex::new(());
    static ONCE: Once = Once::new();

    /// The one context and back end of the module; the tests take turns and
    /// each starts with nothing recorded and nothing pending.
    fn rig() -> MutexGuard<'static, ()> {
        let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        ONCE.call_once(|| {
            // SAFETY: statics; the lock serialises every GL call of the module.
            unsafe {
                osal_set_ops(osal());
                yakogl::backend::g_backend = &yakogl_backend_v3d;
                assert_eq!(yakogl::backend::init(), 0, "{:?}", seen(|s| s.log.clone()));
                assert!(!context::gl_context_create(32, 32).is_null(), "{:?}", seen(|s| s.log.clone()));
            }
        });
        // SAFETY: plain GL calls.
        unsafe {
            glFinish();
            while glGetError() != GL_NO_ERROR {}
        }
        seen(|s| *s = Seen::default());
        g
    }

    const W: GLsizei = 24;
    const H: GLsizei = 16;

    fn unsigned(format: GLenum) -> bool {
        matches!(format, GL_RGBA8UI | GL_R32UI | GL_RGBA32UI)
    }

    /// An integer renderbuffer in a framebuffer, with the scissor box and the
    /// colour mask the call names; `words` cleared into it with
    /// glClearBufferuiv (or iv for a signed format) and flushed. Returns what
    /// the job builder recorded.
    fn clear(format: GLenum, scissor: Option<[GLint; 4]>, mask: [bool; 4], words: [u32; 4]) -> Seen {
        let (mut rb, mut fb) = (0, 0);
        // SAFETY: plain GL calls with room for one name each; the clear values are four words.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, format, W, H);
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
            let log = seen(|s| s.log.clone());
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{log:?}");
            glViewport(0, 0, W, H);
            if let Some([x, y, w, h]) = scissor {
                glEnable(GL_SCISSOR_TEST);
                glScissor(x, y, w, h);
            }
            glColorMask(mask[0] as u8, mask[1] as u8, mask[2] as u8, mask[3] as u8);
            if unsigned(format) {
                glClearBufferuiv(GL_COLOR, 0, words.as_ptr());
            } else {
                glClearBufferiv(GL_COLOR, 0, words.as_ptr() as *const GLint);
            }
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", seen(|s| s.log.clone()));
            glFinish();
            glColorMask(1, 1, 1, 1);
            glDisable(GL_SCISSOR_TEST);
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fb);
            glDeleteRenderbuffers(1, &rb);
        }
        seen(core::mem::take)
    }

    /// Does the draw write its colour as 32-bit TLB words (the integer
    /// program), not f16 pairs (the float one)?
    fn writes_32bit_words(q: &SeenDraw) -> bool {
        q.fs_uniforms.contains(&V3dUniform { contents: V3D_QUNIFORM_CONSTANT, data: 0xffff_ff3f })
    }

    /// Raw clear words with an all-ones-ish word and a signalling-NaN pattern.
    const WORDS: [u32; 4] = [0x8000_0001, 0xffff_fffe, 0x7f80_0001, 0x0102_0304];

    /// The GL core drops the alpha bit of the mask for a target with no alpha
    /// (R32I): the mask the quad is given.
    fn written(format: GLenum, mask: u32) -> u32 {
        if format == GL_R32I {
            mask & 7
        } else {
            mask
        }
    }

    fn formats() -> [(GLenum, u32); 3] {
        [
            (GL_RGBA8UI, V3D_OUTPUT_IMAGE_FORMAT_RGBA8UI),
            (GL_R32I, V3D_OUTPUT_IMAGE_FORMAT_R32I),
            (GL_RGBA32UI, V3D_OUTPUT_IMAGE_FORMAT_RGBA32UI),
        ]
    }

    /// A clear under glScissor is one draw of the integer program inside the
    /// box, not a refusal; the TLB clears nothing and loads the colour.
    #[test]
    fn a_scissored_integer_clear_is_drawn() {
        let _g = rig();
        for (f, image_format) in formats() {
            let s = clear(f, Some([3, 2, 9, 7]), [true; 4], WORDS);
            assert_eq!(s.draws.len(), 1, "{f:#x}: the quad, {:?}", s.log);
            assert_eq!((s.jobs.len(), s.clears.len()), (1, 1), "{f:#x}");
            assert!(s.jobs[0].typed && s.jobs[0].rt_format[0] == image_format, "{f:#x}");
            assert!(s.clears[0].load_rt0, "{f:#x}: the colour is loaded, the quad writes it");
            let q = &s.draws[0];
            assert_eq!(q.count, 6, "{f:#x}");
            assert!(writes_32bit_words(q), "{f:#x}: 32-bit words, not f16 pairs: {:?}", q.fs_uniforms);
            assert_eq!(q.fs_storage, WORDS.to_vec(), "{f:#x}: the raw words, as bits");
            assert_eq!(q.st.colormask, written(f, 0xf), "{f:#x}");
            assert!(q.st.scissor, "{f:#x}");
            assert_eq!(
                [q.st.scissor_minx, q.st.scissor_miny, q.st.scissor_maxx, q.st.scissor_maxy],
                [3, 2, 12, 9],
                "{f:#x}: the box, an FBO's rows"
            );
        }
    }

    /// A clear under glColorMask is one draw of the integer program with the
    /// GL's mask as COLOR_WRITE_MASKS and no scissor.
    #[test]
    fn a_masked_integer_clear_is_drawn() {
        let _g = rig();
        for (f, image_format) in formats() {
            for mask in [[true, false, true, false], [false, true, false, true], [false, false, true, true]] {
                let bits: u32 = mask.iter().enumerate().map(|(i, m)| u32::from(*m) << i).sum();
                let s = clear(f, None, mask, WORDS);
                assert_eq!(s.draws.len(), 1, "{f:#x} mask {bits:#x}: the quad, {:?}", s.log);
                assert!(s.jobs[0].typed && s.jobs[0].rt_format[0] == image_format);
                assert!(s.clears[0].load_rt0, "{f:#x} mask {bits:#x}");
                let q = &s.draws[0];
                assert!(writes_32bit_words(q), "{f:#x} mask {bits:#x}");
                assert_eq!(q.fs_storage, WORDS.to_vec(), "{f:#x} mask {bits:#x}");
                assert_eq!(q.st.colormask, written(f, bits), "{f:#x}: the GL mask");
                assert!(!q.st.scissor, "{f:#x} mask {bits:#x}: a background clear has no scissor");
            }
        }
    }

    /// Scissor and mask together: one quad with both.
    #[test]
    fn a_scissored_and_masked_integer_clear_is_one_quad() {
        let _g = rig();
        for (f, _) in formats() {
            let s = clear(f, Some([0, 4, 10, 5]), [false, true, true, false], WORDS);
            assert_eq!(s.draws.len(), 1, "{f:#x}: {:?}", s.log);
            let q = &s.draws[0];
            assert!(writes_32bit_words(q));
            assert_eq!(q.st.colormask, written(f, 0b0110), "{f:#x}");
            assert!(q.st.scissor, "{f:#x}");
            assert_eq!([q.st.scissor_minx, q.st.scissor_maxx], [0, 10], "{f:#x}");
        }
    }

    /// The whole-target clear under the full mask is still the TLB's: a clear
    /// word, no draw.
    #[test]
    fn a_whole_integer_clear_stays_the_tlbs() {
        let _g = rig();
        for (f, _) in formats() {
            let s = clear(f, None, [true; 4], WORDS);
            assert!(s.draws.is_empty(), "{f:#x}: no quad: {:?}", s.log);
            assert_eq!(s.clears.len(), 1, "{f:#x}");
            assert!(!s.clears[0].load_rt0, "{f:#x}: cleared, not loaded");
        }
    }

    /* ---- the sRGB render target (docs/es3/srgb.md 5.1, 5.4, 9 S6) ------------ */

    /// `SRGB8_ALPHA8` through the same GL core and V3D back end: a draw into
    /// it, a clear of it (glClear and glClearBufferfv), a clear the TLB
    /// cannot do (scissored, masked), and the sRGB attachment beside an RGBA8
    /// or an RGBA32UI one. What the job builder is handed is the whole of the
    /// V3D's sRGB render side (the TLB does the encode on the board): the
    /// job is typed with RT n's output image format `SRGB8_ALPHA8` (0, Mesa's
    /// `v3dx_format_table.c:67`, internal type 16F at 64 bpp, clamp NORM:
    /// `state::rt_format`), the clear words are the linear colour clamped to
    /// [0, 1] in halves (`state::clear_words`), the fragment shader writes
    /// f16 pairs with no TLB config of its own, and a clear quad is the float
    /// program with the colour as four f32 words. Nothing of it is refused
    /// but the scissored or masked clear of sRGB beside another attachment,
    /// which is the several-attachment refusal every class gets
    /// (`a_scissored_clear_of_srgb_beside_another_is_the_mrt_refusal_today`).
    mod srgb {
        use super::*;

        /// The clear colour of these tests: 1.5 is out of range (the sRGB
        /// surface is fixed-point, ES 3.0 4.2.3 clamps its clear).
        const COLOUR: [f32; 4] = [0.5, 1.5, 0.2, 0.5];

        /// `COLOUR` as the TLB's clear of the sRGB surface: linear halves,
        /// 1.5 clamped to 1.0 (0x3c00), 0.5 is 0x3800, 0.2 is 0x3266.
        const SRGB_CLEAR_WORDS: [u32; 4] = [0x3c00_3800, 0x3800_3266, 0, 0];

        /// A framebuffer of one `W` x `H` renderbuffer per format on colour
        /// attachments 0.., all of them draw buffers, bound, complete.
        struct Fbo {
            fb: GLuint,
            rbs: Vec<GLuint>,
            tex: GLuint,
        }

        impl Drop for Fbo {
            fn drop(&mut self) {
                // SAFETY: names this test made.
                unsafe {
                    glBindFramebuffer(GL_FRAMEBUFFER, 0);
                    glDeleteFramebuffers(1, &self.fb);
                    glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
                    if self.tex != 0 {
                        glDeleteTextures(1, &self.tex);
                    }
                }
            }
        }

        fn fbo(formats: &[GLenum]) -> Fbo {
            let n = formats.len();
            let (mut fb, mut rbs) = (0, vec![0; n]);
            // SAFETY: plain GL calls with room for the names.
            unsafe {
                glGenFramebuffers(1, &mut fb);
                glBindFramebuffer(GL_FRAMEBUFFER, fb);
                glGenRenderbuffers(n as GLsizei, rbs.as_mut_ptr());
                let mut bufs = Vec::new();
                for (i, &f) in formats.iter().enumerate() {
                    glBindRenderbuffer(GL_RENDERBUFFER, rbs[i]);
                    glRenderbufferStorage(GL_RENDERBUFFER, f, W, H);
                    glFramebufferRenderbuffer(
                        GL_FRAMEBUFFER,
                        GL_COLOR_ATTACHMENT0 + i as GLenum,
                        GL_RENDERBUFFER,
                        rbs[i],
                    );
                    bufs.push(GL_COLOR_ATTACHMENT0 + i as GLenum);
                }
                glDrawBuffers(n as GLsizei, bufs.as_ptr());
                assert_eq!(glGetError(), GL_NO_ERROR, "{formats:#x?}: {:?}", seen(|s| s.log.clone()));
                assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{formats:#x?}");
                glViewport(0, 0, W, H);
            }
            Fbo { fb, rbs, tex: 0 }
        }

        /// A `W` x `H` SRGB8_ALPHA8 texture level as colour attachment 0 (the
        /// texture layout code is reached: a render image in the TMU's layout).
        fn tex_fbo() -> Fbo {
            let (mut fb, mut tex) = (0, 0);
            // SAFETY: plain GL calls with room for the names; null pixels only allocate.
            unsafe {
                glGenTextures(1, &mut tex);
                glBindTexture(GL_TEXTURE_2D, tex);
                glTexImage2D(
                    GL_TEXTURE_2D,
                    0,
                    GL_SRGB8_ALPHA8 as GLint,
                    W,
                    H,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    core::ptr::null(),
                );
                glGenFramebuffers(1, &mut fb);
                glBindFramebuffer(GL_FRAMEBUFFER, fb);
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
                assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", seen(|s| s.log.clone()));
                assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
                glViewport(0, 0, W, H);
            }
            Fbo { fb, rbs: Vec::new(), tex }
        }

        fn shader(ty: GLenum, src: &str) -> GLuint {
            let c = CString::new(src).unwrap();
            // SAFETY: plain GL calls; the string lives for the call.
            unsafe {
                let s = glCreateShader(ty);
                let srcs = [c.as_ptr()];
                glShaderSource(s, 1, srcs.as_ptr(), core::ptr::null());
                glCompileShader(s);
                let mut ok: GLint = 0;
                glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
                assert_eq!(ok, 1, "compile {ty:#x}: {src}\n{:?}", seen(|s| s.log.clone()));
                s
            }
        }

        /// The current program: `a_position` at attribute 0 and one `out` per
        /// entry of `outs` ("vec4" or "uvec4"), each written a constant.
        fn program(outs: &[&str]) -> GLuint {
            let mut fs = String::from("#version 300 es\nprecision highp float;\nprecision highp int;\n");
            for (i, t) in outs.iter().enumerate() {
                fs += &format!("layout(location = {i}) out {t} o{i};\n");
            }
            fs += "void main() {\n";
            for (i, t) in outs.iter().enumerate() {
                fs += &match *t {
                    "vec4" => format!("  o{i} = vec4(0.5, 1.5, 0.2, 0.5);\n"),
                    _ => format!("  o{i} = uvec4(1u, 2u, 3u, 4u);\n"),
                };
            }
            fs += "}\n";
            let v = shader(
                GL_VERTEX_SHADER,
                "#version 300 es\nin vec4 a_position;\nvoid main() { gl_Position = a_position; }\n",
            );
            let f = shader(GL_FRAGMENT_SHADER, &fs);
            let name = CString::new("a_position").unwrap();
            // SAFETY: plain GL calls on handles just made.
            unsafe {
                let p = glCreateProgram();
                glAttachShader(p, v);
                glAttachShader(p, f);
                glBindAttribLocation(p, 0, name.as_ptr());
                glLinkProgram(p);
                let mut ok: GLint = 0;
                glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
                assert_eq!(ok, 1, "link: {:?}", seen(|s| s.log.clone()));
                glUseProgram(p);
                glDeleteShader(v);
                glDeleteShader(f);
                p
            }
        }

        /// One triangle over the whole target.
        static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

        fn draw_triangle() {
            // SAFETY: plain GL calls; TRI is static.
            unsafe {
                glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
                glEnableVertexAttribArray(0);
                glDrawArrays(GL_TRIANGLES, 0, 3);
            }
        }

        /// What a scene left: the job builder's record, the draws the back
        /// end refused during it (`v3d_refused_draws`) and the GL error.
        struct Run {
            seen: Seen,
            refused: u32,
            error: GLenum,
        }

        /// Run `scene` on a fresh record and finish it; the GL state it set is
        /// put back.
        fn run(scene: impl FnOnce()) -> Run {
            let refused0 = v3d_refused_draws();
            seen(|s| *s = Seen::default());
            scene();
            // SAFETY: plain GL calls.
            let error = unsafe {
                let e = glGetError();
                glFinish();
                let e = if e == GL_NO_ERROR { glGetError() } else { e };
                glUseProgram(0);
                glDisable(GL_BLEND);
                glDisable(GL_SCISSOR_TEST);
                glColorMask(1, 1, 1, 1);
                e
            };
            Run { seen: seen(core::mem::take), refused: v3d_refused_draws().wrapping_sub(refused0), error }
        }

        /// The one typed job of a run, its RTs' output image formats being
        /// `rt_format` (the TLB rows: `rt_format[n]` is RT n's, Mesa's
        /// `v3dx_format_table.c` column 2) and no draw refused.
        fn the_typed_job(r: &Run, rt_format: &[u32]) -> V3dJobTarget {
            assert_eq!(r.error, GL_NO_ERROR, "{:?}", r.seen.log);
            assert_eq!(r.refused, 0, "no draw refused: {:?}", r.seen.log);
            assert_eq!(r.seen.jobs.len(), 1, "one job: {:?}", r.seen.log);
            let j = r.seen.jobs[0];
            assert!(j.typed, "a typed job");
            assert_eq!(j.rts.max(1) as usize, rt_format.len(), "RTs of the job");
            assert_eq!(&j.rt_format[..rt_format.len()], rt_format);
            assert!(!j.msaa && !j.rgb565);
            j
        }

        /// The sRGB surface's TLB row the job builder types RT n with
        /// (`driver/src/v3d/v3d_job.c` `rt_internal` reads the same table):
        /// output SRGB8_ALPHA8, 16F at 64 bpp, clamp NORM, 4 bytes a pixel.
        #[test]
        fn the_srgb_row_is_mesas() {
            let row = state::rt_format(GLES_SURF_SRGB8_A8).expect("a TLB row");
            assert_eq!(row.output, V3D_OUTPUT_IMAGE_FORMAT_SRGB8_ALPHA8);
            assert_eq!(V3D_OUTPUT_IMAGE_FORMAT_SRGB8_ALPHA8, 0, "v3d_packet.xml:227");
            assert_eq!(row.internal_type, V3D_INTERNAL_TYPE_16F);
            assert_eq!(row.internal_bpp, V3D_INTERNAL_BPP_64);
            assert_eq!(row.clamp, V3D_RENDER_TARGET_CLAMP_NORM);
            assert_eq!(row.bytes, 4);
            assert!(state::typed(GLES_SURF_SRGB8_A8));
            assert_eq!(state::clear_words(GLES_SURF_SRGB8_A8, COLOUR), SRGB_CLEAR_WORDS);
        }

        /// (1) A draw into an SRGB8_ALPHA8 renderbuffer: not refused, one
        /// typed job with RT 0 in the SRGB8_ALPHA8 image format, one draw of
        /// three vertices whose fragment shader writes f16 pairs (no 32-bit
        /// TLB config: the TLB converts to the 16F tile buffer and encodes at
        /// the store, docs/es3/srgb.md 5.3) under the full colour mask. With
        /// GL_BLEND on, the same draw blends in the job's fixed function
        /// (`blend_enabled`: the tile buffer is linear 16F, 5.2).
        #[test]
        fn a_draw_into_srgb_is_a_typed_16f_job_writing_halves() {
            let _g = rig();
            let f = fbo(&[GL_SRGB8_ALPHA8]);
            let r = run(|| {
                program(&["vec4"]);
                draw_triangle();
            });
            the_typed_job(&r, &[V3D_OUTPUT_IMAGE_FORMAT_SRGB8_ALPHA8]);
            assert_eq!(r.seen.draws.len(), 1, "the triangle: {:?}", r.seen.log);
            let q = &r.seen.draws[0];
            assert_eq!(q.count, 3);
            assert!(!writes_32bit_words(q), "f16 pairs, the TLB's default config: {:?}", q.fs_uniforms);
            assert_eq!(q.st.colormask, 0xf);
            assert!(!q.st.blend_enabled);
            let r = run(|| {
                program(&["vec4"]);
                // SAFETY: plain GL calls.
                unsafe {
                    glEnable(GL_BLEND);
                    glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
                }
                draw_triangle();
            });
            the_typed_job(&r, &[V3D_OUTPUT_IMAGE_FORMAT_SRGB8_ALPHA8]);
            assert_eq!(r.seen.draws.len(), 1, "{:?}", r.seen.log);
            let q = &r.seen.draws[0];
            assert!(q.st.blend_enabled, "the blend is the TLB's, in the linear tile buffer");
            assert_eq!((q.st.rgb_src, q.st.rgb_dst), (V3D_BLEND_FACTOR_SRC_ALPHA, V3D_BLEND_FACTOR_INV_SRC_ALPHA));
            drop(f);
        }

        /// (1) The same draw into an SRGB8_ALPHA8 texture level (dEQP
        /// `fbo.render.*srgb8_alpha8*` renders into textures): typed, RT 0
        /// SRGB8_ALPHA8, not refused; and no render image (`uif_address`): a
        /// render image stands for RGBA8 textures only, the sRGB class is
        /// sampled through the tiled copy, whose texture state decodes
        /// (`src/v3d/backend.rs`, the `RtImage` list).
        #[test]
        fn a_draw_into_an_srgb_texture_level_is_the_same_typed_job() {
            let _g = rig();
            let f = tex_fbo();
            let r = run(|| {
                program(&["vec4"]);
                draw_triangle();
            });
            let j = the_typed_job(&r, &[V3D_OUTPUT_IMAGE_FORMAT_SRGB8_ALPHA8]);
            assert_eq!(r.seen.draws.len(), 1, "{:?}", r.seen.log);
            assert!(!writes_32bit_words(&r.seen.draws[0]));
            assert_eq!(j.uif_address, 0, "no render image: the sRGB class is sampled through the tiled copy");
            drop(f);
        }

        /// (2) glClear of an SRGB8_ALPHA8 renderbuffer is the TLB's: no quad,
        /// RT 0 cleared (not loaded) from the linear colour clamped to [0, 1]
        /// as halves (`state::clear_words`, Mesa's v3dx_draw.c:1739-1743 for
        /// internal type 16F): 1.5 is 0x3c00, no CPU encode anywhere.
        #[test]
        fn glclear_of_srgb_is_the_tlbs_clamped_linear_halves() {
            let _g = rig();
            let f = fbo(&[GL_SRGB8_ALPHA8]);
            let r = run(|| {
                // SAFETY: plain GL calls.
                unsafe {
                    glClearColor(COLOUR[0], COLOUR[1], COLOUR[2], COLOUR[3]);
                    glClear(GL_COLOR_BUFFER_BIT);
                }
            });
            the_typed_job(&r, &[V3D_OUTPUT_IMAGE_FORMAT_SRGB8_ALPHA8]);
            assert!(r.seen.draws.is_empty(), "no quad: {:?}", r.seen.log);
            assert_eq!(r.seen.clears.len(), 1);
            let c = &r.seen.clears[0];
            assert!(!c.load_rt0, "cleared, not loaded");
            assert_eq!(c.clear_rt[0], SRGB_CLEAR_WORDS, "linear halves, clamped");
            drop(f);
        }

        /// (2) glClearBufferfv(GL_COLOR, 0, ...) of the same: the same clear
        /// words (the GL core hands the back end the float colour, 3.2).
        #[test]
        fn glclearbufferfv_of_srgb_is_the_same_tlb_clear() {
            let _g = rig();
            let f = fbo(&[GL_SRGB8_ALPHA8]);
            let r = run(|| {
                // SAFETY: plain GL call; four floats.
                unsafe { glClearBufferfv(GL_COLOR, 0, COLOUR.as_ptr()) };
            });
            the_typed_job(&r, &[V3D_OUTPUT_IMAGE_FORMAT_SRGB8_ALPHA8]);
            assert!(r.seen.draws.is_empty(), "no quad: {:?}", r.seen.log);
            assert_eq!(r.seen.clears.len(), 1);
            assert!(!r.seen.clears[0].load_rt0);
            assert_eq!(r.seen.clears[0].clear_rt[0], SRGB_CLEAR_WORDS);
            drop(f);
        }

        /// (2) A clear the TLB cannot do, scissored or colour-masked, of an
        /// sRGB target alone: the float clear quad (six vertices, f16 pairs,
        /// the colour as four f32 words unclamped: the TLB's NORM clamp of the
        /// row takes 1.5 to 1 on entry), RT 0 loaded, the GL's box and mask.
        /// Not refused (the several-attachment refusal needs two RTs).
        #[test]
        fn a_scissored_or_masked_clear_of_srgb_alone_is_the_float_quad() {
            let _g = rig();
            let f = fbo(&[GL_SRGB8_ALPHA8]);
            /// (name, scissor box, colour mask)
            type Scene = (&'static str, Option<[GLint; 4]>, [bool; 4]);
            let scenes: [Scene; 3] = [
                ("scissored", Some([3, 2, 9, 7]), [true; 4]),
                ("masked", None, [true, false, true, false]),
                ("scissored and masked", Some([0, 4, 10, 5]), [false, true, true, false]),
            ];
            for (name, scissor, mask) in scenes {
                let r = run(|| {
                    // SAFETY: plain GL calls; four floats.
                    unsafe {
                        if let Some([x, y, w, h]) = scissor {
                            glEnable(GL_SCISSOR_TEST);
                            glScissor(x, y, w, h);
                        }
                        glColorMask(mask[0] as u8, mask[1] as u8, mask[2] as u8, mask[3] as u8);
                        glClearBufferfv(GL_COLOR, 0, COLOUR.as_ptr());
                    }
                });
                the_typed_job(&r, &[V3D_OUTPUT_IMAGE_FORMAT_SRGB8_ALPHA8]);
                assert_eq!(r.seen.draws.len(), 1, "{name}: the quad: {:?}", r.seen.log);
                assert_eq!(r.seen.clears.len(), 1, "{name}");
                assert!(r.seen.clears[0].load_rt0, "{name}: loaded, the quad writes it");
                let q = &r.seen.draws[0];
                assert_eq!(q.count, 6, "{name}");
                assert!(!writes_32bit_words(q), "{name}: the float program, f16 pairs");
                assert_eq!(q.fs_storage, COLOUR.map(f32::to_bits).to_vec(), "{name}: the colour, four f32 words");
                let bits: u32 = mask.iter().enumerate().map(|(i, m)| u32::from(*m) << i).sum();
                assert_eq!(q.st.colormask, bits, "{name}: the GL's mask");
                assert_eq!(q.st.scissor, scissor.is_some(), "{name}");
                if let Some([x, y, w, h]) = scissor {
                    let b = [q.st.scissor_minx, q.st.scissor_miny, q.st.scissor_maxx, q.st.scissor_maxy];
                    assert_eq!(b, [x as u32, y as u32, (x + w) as u32, (y + h) as u32], "{name}: the box");
                }
            }
            drop(f);
        }

        /// An attachment pair of (3): its name, the two formats, the fragment
        /// output type of each, and the output image format of each RT.
        type Pair = (&'static str, [GLenum; 2], [&'static str; 2], [u32; 2]);

        /// The attachment pairs of (3): sRGB in front of and behind an RGBA8
        /// and an RGBA32UI attachment, with their fragment output types.
        fn pairs() -> [Pair; 4] {
            const S: u32 = V3D_OUTPUT_IMAGE_FORMAT_SRGB8_ALPHA8;
            [
                ("sRGB+RGBA8", [GL_SRGB8_ALPHA8, GL_RGBA8], ["vec4", "vec4"], [S, V3D_OUTPUT_IMAGE_FORMAT_RGBA8]),
                ("RGBA8+sRGB", [GL_RGBA8, GL_SRGB8_ALPHA8], ["vec4", "vec4"], [V3D_OUTPUT_IMAGE_FORMAT_RGBA8, S]),
                (
                    "sRGB+RGBA32UI",
                    [GL_SRGB8_ALPHA8, GL_RGBA32UI],
                    ["vec4", "uvec4"],
                    [S, V3D_OUTPUT_IMAGE_FORMAT_RGBA32UI],
                ),
                (
                    "RGBA32UI+sRGB",
                    [GL_RGBA32UI, GL_SRGB8_ALPHA8],
                    ["uvec4", "vec4"],
                    [V3D_OUTPUT_IMAGE_FORMAT_RGBA32UI, S],
                ),
            ]
        }

        /// (3) A draw into an sRGB attachment beside an RGBA8 or an RGBA32UI
        /// one, in either order: not refused (`extra_targets` takes any
        /// surface with a TLB row; its comment that the sRGB surface has none
        /// is stale), one typed job of two RTs in their own image formats, the
        /// integer RT written with the 32-bit TLB config naming it and the
        /// sRGB RT with f16 pairs.
        #[test]
        fn a_draw_into_srgb_beside_another_attachment_is_one_typed_job() {
            let _g = rig();
            for (name, formats, outs, rows) in pairs() {
                let f = fbo(&formats);
                let r = run(|| {
                    program(&outs);
                    draw_triangle();
                });
                the_typed_job(&r, &rows);
                assert_eq!(r.seen.draws.len(), 1, "{name}: {:?}", r.seen.log);
                let q = &r.seen.draws[0];
                assert_eq!(q.count, 3, "{name}");
                for (rt, out) in outs.iter().enumerate() {
                    let cfg = V3dUniform {
                        contents: V3D_QUNIFORM_CONSTANT,
                        data: yakogl::shaderc::online::v3d::select::tlb_config_32b(rt as u32, 4),
                    };
                    assert_eq!(q.fs_uniforms.contains(&cfg), *out == "uvec4", "{name}: RT {rt}: {:?}", q.fs_uniforms);
                }
                drop(f);
            }
        }

        /// (3) The whole-target clear of sRGB beside RGBA8 (glClear) and
        /// beside RGBA32UI (glClearBufferfv and glClearBufferuiv, one job) is
        /// the TLB's: no quad, each RT cleared from its own words, the sRGB
        /// one's the clamped linear halves, the RGBA8 one's the RGBA8 word and
        /// the integer one's the raw words.
        #[test]
        fn a_whole_clear_of_srgb_beside_another_attachment_is_the_tlbs() {
            let _g = rig();
            let f = fbo(&[GL_SRGB8_ALPHA8, GL_RGBA8]);
            let r = run(|| {
                // SAFETY: plain GL calls.
                unsafe {
                    glClearColor(COLOUR[0], COLOUR[1], COLOUR[2], COLOUR[3]);
                    glClear(GL_COLOR_BUFFER_BIT);
                }
            });
            the_typed_job(&r, &[V3D_OUTPUT_IMAGE_FORMAT_SRGB8_ALPHA8, V3D_OUTPUT_IMAGE_FORMAT_RGBA8]);
            assert!(r.seen.draws.is_empty(), "no quad: {:?}", r.seen.log);
            assert_eq!(r.seen.clears.len(), 1);
            let c = &r.seen.clears[0];
            assert!(!c.load_rt0 && !c.load_rt[0], "both cleared, neither loaded");
            assert_eq!(c.clear_rt[0], SRGB_CLEAR_WORDS);
            assert_eq!(c.clear_rt[1], state::clear_words(GLES_SURF_RGBA8888, COLOUR), "RGBA8's own word");
            drop(f);
            let f = fbo(&[GL_RGBA32UI, GL_SRGB8_ALPHA8]);
            let r = run(|| {
                // SAFETY: plain GL calls; four words, four floats.
                unsafe {
                    glClearBufferuiv(GL_COLOR, 0, WORDS.as_ptr());
                    glClearBufferfv(GL_COLOR, 1, COLOUR.as_ptr());
                }
            });
            the_typed_job(&r, &[V3D_OUTPUT_IMAGE_FORMAT_RGBA32UI, V3D_OUTPUT_IMAGE_FORMAT_SRGB8_ALPHA8]);
            assert!(r.seen.draws.is_empty(), "no quad: {:?}", r.seen.log);
            assert_eq!(r.seen.clears.len(), 1, "one job clears both");
            let c = &r.seen.clears[0];
            assert!(!c.load_rt0 && !c.load_rt[0]);
            assert_eq!(c.clear_rt[0], state::clear_words(GLES_SURF_RGBA32UI, WORDS.map(f32::from_bits)));
            assert_eq!(c.clear_rt[0], WORDS, "the raw words");
            assert_eq!(c.clear_rt[1], SRGB_CLEAR_WORDS);
            drop(f);
        }

        /// A scissored (or masked) colour clear of sRGB beside another
        /// attachment is the several-attachment clear quad, as for every
        /// class (`src/v3d/clear_mrt.rs`, `draw_clear_quad`;
        /// docs/notes/v3d-mrt-clear.md; until v3d-mrt-clear the back end
        /// refused it by name, "a scissored or masked colour clear of several
        /// colour attachments"): no refusal, no error, both RTs loaded, one
        /// quad whose program writes both as f16 pairs (both are float class)
        /// with the GL's box, the colour as f32 words once per RT in its
        /// fragment block, into a job whose RTs are still [SRGB8_ALPHA8, RGBA8].
        #[test]
        fn a_scissored_clear_of_srgb_beside_another_is_the_several_attachment_quad() {
            let _g = rig();
            let f = fbo(&[GL_SRGB8_ALPHA8, GL_RGBA8]);
            let r = run(|| {
                // SAFETY: plain GL calls.
                unsafe {
                    glEnable(GL_SCISSOR_TEST);
                    glScissor(3, 2, 9, 7);
                    glClearColor(COLOUR[0], COLOUR[1], COLOUR[2], COLOUR[3]);
                    glClear(GL_COLOR_BUFFER_BIT);
                }
            });
            the_typed_job(&r, &[V3D_OUTPUT_IMAGE_FORMAT_SRGB8_ALPHA8, V3D_OUTPUT_IMAGE_FORMAT_RGBA8]);
            assert_eq!(r.seen.clears.len(), 1);
            assert!(r.seen.clears[0].load_rt0 && r.seen.clears[0].load_rt[0], "loaded, the quad writes them");
            assert_eq!(r.seen.draws.len(), 1, "one quad for both: {:?}", r.seen.log);
            let q = &r.seen.draws[0];
            assert_eq!(q.count, 6);
            assert!(!writes_32bit_words(q), "both RTs are float class");
            assert_eq!(q.fs_storage, COLOUR.map(f32::to_bits).repeat(2), "the colour once per RT");
            assert!(q.st.scissor);
            assert_eq!([q.st.scissor_minx, q.st.scissor_miny, q.st.scissor_maxx, q.st.scissor_maxy], [3, 2, 12, 9]);
            drop(f);
        }
    }
}
