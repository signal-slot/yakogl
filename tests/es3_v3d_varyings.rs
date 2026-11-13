// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D varyings, end to end on the host, for plan #359's criterion c3: a
//! program of as many vec4 varyings as GL_MAX_VARYING_VECTORS says, smooth,
//! flat and centroid mixed, each carrying a distinct per-vertex value.
//!
//! The host has no V3D pixels: the back end's host rig sees jobs and records,
//! not a framebuffer. What it has is the QPU interpreter of the compiler
//! (`sim::run`), which runs the compiled fragment program on 16 lanes and
//! records its TLB writes -- the colour the pixel gets. So a "draw" here is
//! the GL core linking the program (the real limit and link rules), the V3D
//! compiler compiling it, and the interpreter running its fragment program
//! for a 16-pixel quad of one triangle with the inputs the rasteriser
//! hands LDVARY: a smooth or centroid varying is the barycentric mix of the
//! three vertices' values, a flat one the provoking (last) vertex's. The
//! vertex stage is compiled, not run (its VPM layout is the compiler tests').
//!
//! The fragment shader sums every varying, each swizzled and weighted by its
//! index, into one colour, so a dropped, swapped or mis-flagged input moves
//! the pixel. Run with W = 1 and again with W = 2 and the centroid W = 4 (the
//! hardware's perspective factors): a smooth varying is `vary * W`, a
//! centroid one `vary * W_centroid`, a flat one its C coefficient alone.
//!
//! * `the_limit_in_varyings_*`: live, at the limit GL reports (8 today).
//! * `fifteen_vectors_*`, `sixty_four_output_components_*`: ES 3.0's minimum
//!   (tables 6.31 and 6.32); `#[ignore]`d until the V3D reports and carries
//!   it (plan #359 v3d-varyings-raise). Run them with `-- --ignored`. Past
//!   32 inputs the `FsProg` flag words (`flat_shade_flags`,
//!   `centroid_flags`, one u32) cannot name the input; the 15-vector test
//!   checks the low 32 bits and the pixels (select.rs:2286 treats inputs
//!   past 31 as smooth, so the flat ones among them will fail it), and the flag words themselves
//!   of inputs 32-59 are the compiler item's to add.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_varyings
//! ```
#![cfg(all(feature = "host-test", feature = "gpu-v3d", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::glGetIntegerv;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target, V3dProgram};
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Varying, LANES};
use yakogl::shaderc::online::{lower, sema};

/* ---- a context on the host: the smallest OSAL and back end that make one -- */

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u8; size].into_boxed_slice());
    let p = mem.as_mut_ptr();
    *out = OsalGpuMem { cpu: p as *mut c_void, gpu: p as u64, phys: p as u64, size, impl_: p as *mut c_void };
    0
}
unsafe extern "C" fn m_gpu_free(_mem: *mut OsalGpuMem) -> i32 {
    0
}
unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_heap: GlesHeap) -> u64 {
    0x1000_0000
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(m_gpu_alloc),
    gpu_free: Some(m_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(m_cache),
    cache_invd: Some(m_cache),
    submit: None,
    wait: None,
    completed: None,
    reg_read: None,
    reg_write: None,
    query: None,
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-v3d-varyings".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: None,
    flush: None,
    finish: None,
    finish_previous: None,
    kick_id: None,
    kick_pending: None,
    heap_base: Some(b_heap_base),
    counters: None,
    texture_released: None,
    surface_swapped: None,
    texture_uploaded: None,
    max_instance_divisor: 1,
    wait_kick: None,
    samples_texel_copies: false,
    attachment_gone: None,
    depth_resolve: None,
    query_begin: None,
    query_end: None,
    query_result: None,
    generate_mipmap: None,
    return_mipmap_chain: None,
    linear_row_align: 0,
    uniforms_host_only: false,
    ds_write: None,
    stencil_resolve: None,
};

static LIBRARY: Mutex<()> = Mutex::new(());
static CONTEXT: Once = Once::new();

/// The context, made once; the guard keeps the tests of this file off the
/// library and the compiler (single-threaded by contract) one at a time.
fn context() -> MutexGuard<'static, ()> {
    let guard = LIBRARY.lock().unwrap_or_else(|e| e.into_inner());
    CONTEXT.call_once(|| {
        // SAFETY: both tables are statics, and the guard above is held.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &BACKEND;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    guard
}

fn geti(p: GLenum) -> usize {
    let mut v: GLint = -1;
    // SAFETY: plain GL call with a real output.
    unsafe { glGetIntegerv(p, &mut v) };
    usize::try_from(v).expect("a limit")
}

/* ---- the sources: n vec4 varyings, smooth / flat / centroid mixed -------------- */

/// The qualifier of varying `i`: smooth, flat, centroid in turn, so the
/// upper inputs (vectors 8-14) hold all three kinds.
fn qual(i: usize) -> &'static str {
    ["smooth", "flat", "centroid"][i % 3]
}

const SWIZZLE: [&str; 4] = ["xyzw", "yzwx", "zwxy", "wxyz"];

fn vertex_source(n: usize) -> String {
    let mut vs = String::from("#version 300 es\nin highp vec4 a_pos;\nin highp vec4 a_c;\n");
    for i in 0..n {
        vs += &format!("{} out highp vec4 v{i};\n", qual(i));
    }
    vs += "void main() {\n    gl_Position = a_pos;\n";
    for i in 0..n {
        vs += &format!("    v{i} = a_c.{} * {}.0;\n", SWIZZLE[i % 4], i + 1);
    }
    vs + "}\n"
}

/// The sum of the varyings, each rotated and weighted by `i + 1`.
fn fragment_source(n: usize) -> String {
    let mut fs = String::from("#version 300 es\nprecision highp float;\nlayout(location = 0) out highp vec4 o;\n");
    for i in 0..n {
        fs += &format!("{} in highp vec4 v{i};\n", qual(i));
    }
    fs += "void main() {\n    highp vec4 s = vec4(0.0);\n";
    for i in 0..n {
        fs += &format!("    s += v{i}.{} * {}.0;\n", SWIZZLE[i % 4], i + 1);
    }
    fs + "    o = s;\n}\n"
}

/* ---- the GL: limits and link ----------------------------------------------------- */

fn text_of(log: &[c_char]) -> String {
    log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

fn shader(ty: GLenum, text: &str) -> Result<GLuint, String> {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        if ok == 1 {
            return Ok(s);
        }
        let mut log = [0 as c_char; 1024];
        glGetShaderInfoLog(s, log.len() as GLsizei, core::ptr::null_mut(), log.as_mut_ptr());
        Err(text_of(&log))
    }
}

/// The program linked through the GL core: Ok(()) or the info log.
fn gl_link(vs: &str, fs: &str) -> Result<(), String> {
    let v = shader(GL_VERTEX_SHADER, vs).map_err(|m| format!("vertex: {m}"))?;
    let f = shader(GL_FRAGMENT_SHADER, fs).map_err(|m| format!("fragment: {m}"))?;
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"a_pos".as_ptr());
        glBindAttribLocation(p, 1, c"a_c".as_ptr());
        glLinkProgram(p);
        glDeleteShader(v);
        glDeleteShader(f);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        let mut log = [0 as c_char; 1024];
        glGetProgramInfoLog(p, log.len() as GLsizei, core::ptr::null_mut(), log.as_mut_ptr());
        glDeleteProgram(p);
        if ok == 1 {
            Ok(())
        } else {
            Err(text_of(&log))
        }
    }
}

/* ---- the compiler and the interpreter ---------------------------------------------- */

const TARGET: Target = Target { vpm_size: 65536 };

/// The vertex `j`'s value of component `k` of varying `i`: distinct for
/// every (vertex, varying, component), small integers.
fn vertex_value(j: usize, i: usize, k: usize) -> f32 {
    (3 * (4 * i + k) + j + 1) as f32
}

/// Lane `l`'s barycentric coordinates in the triangle (dyadic, so the
/// interpolation is exact): b1 and b2 in eighths, b0 the rest.
fn bary(l: usize) -> [f32; 3] {
    let (b1, b2) = ((l % 4) as f32 / 8.0, (l / 4) as f32 / 8.0);
    [1.0 - b1 - b2, b1, b2]
}

/// The value of varying `i` component `k` on lane `l` as the rasteriser
/// hands it to LDVARY (before the W multiply): the barycentric mix, or the
/// provoking (last) vertex's for a flat varying.
fn interpolated(l: usize, i: usize, k: usize) -> f32 {
    if qual(i) == "flat" {
        return vertex_value(2, i, k);
    }
    let b = bary(l);
    (0..3).map(|j| b[j] * vertex_value(j, i, k)).sum()
}

/// The colour lane `l` should get with the perspective factors `w` and
/// `wc`: the shader's sum on the values the hardware hands it.
fn expected(l: usize, n: usize, w: f32, wc: f32) -> [f32; 4] {
    let mut s = [0.0f32; 4];
    for i in 0..n {
        let scale = match qual(i) {
            "flat" => 1.0,
            "centroid" => wc,
            _ => w,
        };
        let sw = SWIZZLE[i % 4].as_bytes();
        for (c, o) in s.iter_mut().enumerate() {
            let k = b"xyzw".iter().position(|&ch| ch == sw[c]).unwrap();
            *o += interpolated(l, i, k) * scale * (i + 1) as f32;
        }
    }
    s
}

/// Compile the program of `n` varyings and run its fragment program on the
/// quad: the colour of each lane, handed to `check` with the program.
fn draw_quad<R>(n: usize, w: f32, wc: f32, check: impl FnOnce(&V3dProgram<'_>, [[f32; 4]; LANES]) -> R) -> R {
    let mut mem = vec![0u8; 32 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(&vertex_source(n)).expect("arena");
    let fs = a.alloc_str(&fragment_source(n)).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).unwrap_or_else(|e| panic!("the VS lowers: {}", e.msg));
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).unwrap_or_else(|e| panic!("the FS lowers: {}", e.msg));
    let p = compile_program(&v.module, &f.module, Prim::Triangles, TARGET, &a)
        .unwrap_or_else(|e| panic!("the program compiles: {e:?}"));
    assert_eq!(p.fs.num_inputs as usize, 4 * n, "every component of every varying is an input");
    assert_eq!(p.fs.input_slots.len(), 4 * n);
    /* one word per ldvary, in input order: (VARYING_SLOT_VAR0 + location, component) */
    let varyings: Vec<Varying> =
        p.fs.input_slots
            .iter()
            .map(|&(slot, k)| {
                let i = usize::from(slot - 32);
                assert!(i < n && k < 4, "input ({slot}, {k}) of a program of {n} varyings");
                let v = |l: usize| interpolated(l, i, usize::from(k)).to_bits();
                /* a flat varying is C alone (select.rs emit_flat_varying): the provoking
                 * vertex's value in C, and a garbage vary that must not be read */
                if qual(i) == "flat" {
                    Varying { vary: [99.0f32.to_bits(); LANES], c: core::array::from_fn(v) }
                } else {
                    Varying { vary: core::array::from_fn(v), c: [0; LANES] }
                }
            })
            .collect();
    let inputs = FsInputs {
        w: [w.to_bits(); LANES],
        w_centroid: [wc.to_bits(); LANES],
        z: [0.5f32.to_bits(); LANES],
        x: [0; LANES],
        y: [0; LANES],
        back_facing: [false; LANES],
        varyings,
    };
    let env = Env { threads: p.fs.threads, ..Env::default() };
    let stream: Vec<u32> =
        p.fs.code
            .uniforms
            .iter()
            .map(|u| match u.contents {
                0 => u.data,
                c => panic!("uniform contents {c} not modelled by this test"),
            })
            .collect();
    let out = sim::run(p.fs.code.words, &stream, &Inputs::Fragment(Box::new(inputs)), &env)
        .unwrap_or_else(|e| panic!("the fragment program runs: {e}"));
    assert_eq!(out.msf, [1; LANES], "no lane discarded");
    /* the colour: the TLB writes without a configuration; the colour goes out as
     * f16 pairs (low half the even component), two writes for a vec4 */
    let words: Vec<&sim::TlbWrite> = out.tlb.iter().filter(|t| t.config.is_none()).collect();
    assert_eq!(words.len(), 2, "one vec4 colour as two packed words: {:#x?}", out.tlb);
    let px = core::array::from_fn(|l| {
        core::array::from_fn(|c| {
            let w = words[c / 2].value[l].unwrap_or_else(|| panic!("lane {l}: undefined"));
            f16_to_f32((w >> (16 * (c % 2))) as u16)
        })
    });
    check(&p, px)
}

/// The f16 `h` as an f32 (finite values and zero; the sums here are far from the ends).
fn f16_to_f32(h: u16) -> f32 {
    let (sign, exp, man) = (if h & 0x8000 != 0 { -1.0 } else { 1.0 }, i32::from(h >> 10 & 0x1f), f32::from(h & 0x3ff));
    assert!(exp != 31, "an f16 infinity or NaN: {h:#x}");
    sign * if exp == 0 { man * 2f32.powi(-24) } else { (1.0 + man / 1024.0) * 2f32.powi(exp - 15) }
}

/// The colour is f16: half a unit in the eleventh bit, 2^-11 relative, and
/// the sums are exact in f32 or close to it.
fn close(got: f32, want: f32) -> bool {
    (got - want).abs() <= 1.5e-3 * want.abs().max(1.0)
}

/// Both perspective cases: W = 1 (every mode is the interpolated value) and
/// W = 2, W_centroid = 4 (smooth doubles, centroid quadruples, flat stays).
fn check_pixels(n: usize) {
    for (w, wc) in [(1.0, 1.0), (2.0, 4.0)] {
        draw_quad(n, w, wc, |p, px| {
            for (l, got) in px.iter().enumerate() {
                let want = expected(l, n, w, wc);
                for c in 0..4 {
                    assert!(
                        close(got[c], want[c]),
                        "{n} varyings, W {w} / {wc}: lane {l} component {c}: got {} want {}",
                        got[c],
                        want[c]
                    );
                }
            }
            /* the flag bits of the first 32 inputs: bit m is input m in ldvary order */
            for (m, &(slot, _)) in p.fs.input_slots.iter().enumerate().take(32) {
                let q = qual(usize::from(slot - 32));
                assert_eq!(p.fs.flat_shade_flags >> m & 1, u32::from(q == "flat"), "flat flag of input {m} ({q})");
                assert_eq!(
                    p.fs.centroid_flags >> m & 1,
                    u32::from(q == "centroid"),
                    "centroid flag of input {m} ({q})"
                );
            }
        });
    }
}

/* ---- the tests ----------------------------------------------------------------------- */

/// Live: a program of GL_MAX_VARYING_VECTORS vec4 varyings (8 today), mixed
/// qualifiers, links through the GL core, compiles, and shades the quad:
/// smooth and centroid the barycentric mix, flat the last vertex's value.
#[test]
fn the_limit_in_varyings_links_and_shades() {
    let _g = context();
    let n = geti(GL_MAX_VARYING_VECTORS);
    assert!(n >= 8, "the V3D reports at least the first cut's 8, not {n}");
    gl_link(&vertex_source(n), &fragment_source(n)).unwrap_or_else(|m| panic!("{n} varyings do not link: {m}"));
    check_pixels(n);
}

/// Live: one vector past the limit is refused by the link (the rule the
/// raise moves).
#[test]
fn one_past_the_limit_does_not_link() {
    let _g = context();
    let n = geti(GL_MAX_VARYING_VECTORS);
    assert!(gl_link(&vertex_source(n + 1), &fragment_source(n + 1)).is_err(), "{} varyings link", n + 1);
}

/// ES 3.0 table 6.31: 15 vectors, 60 components, reported, linked and drawn,
/// with the flat and centroid varyings among vectors 8-14 (inputs 32-59).
#[test]
#[ignore = "needs v3d-varyings-raise: 15 vectors"]
fn fifteen_vectors_link_and_shade() {
    let _g = context();
    assert_eq!(geti(GL_MAX_VARYING_VECTORS), 15);
    assert_eq!(geti(GL_MAX_VARYING_COMPONENTS), 60);
    assert_eq!(geti(GL_MAX_FRAGMENT_INPUT_COMPONENTS), 60);
    gl_link(&vertex_source(15), &fragment_source(15)).unwrap_or_else(|m| panic!("15 varyings do not link: {m}"));
    /* vectors 8..15 hold smooth, flat and centroid */
    assert!((8..15).any(|i| qual(i) == "flat") && (8..15).any(|i| qual(i) == "centroid"));
    check_pixels(15);
}

/// ES 3.0 table 6.32: 64 vertex output components. The vertex shader writes
/// gl_Position (4) and fifteen vec4 varyings (60) and links.
#[test]
#[ignore = "needs v3d-varyings-raise: 64 output components"]
fn sixty_four_output_components_link() {
    let _g = context();
    assert_eq!(geti(GL_MAX_VERTEX_OUTPUT_COMPONENTS), 64);
    gl_link(&vertex_source(15), &fragment_source(15))
        .unwrap_or_else(|m| panic!("64 output components do not link: {m}"));
}
