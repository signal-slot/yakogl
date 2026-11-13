// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The PowerVR's vertex output: what the 15-vector case needs of it today,
//! and the 64-component case ES 3.0 asks (GL_MAX_VERTEX_OUTPUT_COMPONENTS,
//! reported 60 from gl/context.rs MAX_VARYING_VECTORS * 4).
//!
//! Live half: a `#version 300 es` program of fifteen vec4 varyings (three of
//! them flat) beside gl_Position links and draws through the GL on a
//! stand-in back end, and the records the draw hands the back end are
//! the ones the output size must come from: `vs_output_size` is 65 dwords
//! (position 4 + 15 * 4 + gl_PointSize 1), gl_PointSize after the varyings.
//!
//! There is no PowerVR to read pixels back from on the host, so the pixels
//! are the USC byte model's (tests/usc_exec `run_binary`) on the very
//! records the draw was handed: the vertex program is run for the three
//! vertices, its outputs are checked against the shader's own arithmetic
//! at the offsets the record names, the fragment program is run at the
//! triangle's centre with each smooth varying iterating to the mean of the
//! three vertices and each flat one to the last (provoking) vertex, and the
//! RGBA8 pixel it writes is read back. That checks the compiler's offsets
//! and the two programs' agreement, not the ISP's iterator hardware.
//!
//! Ignored half: GL_MAX_VERTEX_OUTPUT_COMPONENTS is 64 and a vertex shader
//! writing 64 output components (sixteen vec4, only fifteen of them read by
//! the fragment shader) links and draws.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_vout64
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_vout64 -- --ignored
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::{glFinish, glGetError};
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::shaderc::ShcShader;

mod usc_exec;

/// ES 3.0's table 6.31 fragment side: fifteen vec4.
const N: usize = 15;
/// The varyings qualified `flat` in the live program.
const FLAT: [usize; 3] = [4, 9, 14];

/* ---- the program ----------------------------------------------------------------- */

/// A vertex shader writing `n` vec4 varyings `v0..v<n>` (each a different
/// swizzle of one attribute, scaled and offset, so no two are the same
/// value and all stay inside [0, 1)), and a fragment shader that reads the
/// first `reads` of them and outputs `v<pick>` (or the sum when `pick` is
/// None). `point_size` adds a gl_PointSize write.
fn sources(n: usize, reads: usize, pick: Option<usize>, point_size: bool) -> (String, String) {
    let mut vs = String::from("#version 300 es\nin highp vec4 a_pos;\nin highp vec4 a_c;\n");
    let mut fs = String::from("#version 300 es\nprecision highp float;\nlayout(location = 0) out highp vec4 o;\n");
    for i in 0..n {
        let q = if n == N && FLAT.contains(&i) { "flat" } else { "" };
        vs += &format!("{q} out highp vec4 v{i};\n");
        if i < reads {
            fs += &format!("{q} in highp vec4 v{i};\n");
        }
    }
    vs += "void main() {\n    gl_Position = a_pos;\n";
    if point_size {
        vs += "    gl_PointSize = 1.0;\n";
    }
    for i in 0..n {
        let sw = ["xyzw", "yzwx", "zwxy", "wxyz"][i % 4];
        vs += &format!("    v{i} = a_c.{sw} * 0.5 + {:.6};\n", i as f32 / 40.0);
    }
    vs += "}\n";
    fs += "void main() {\n";
    match pick {
        Some(k) => fs += &format!("    o = v{k};\n"),
        None => {
            fs += "    highp vec4 s = vec4(0.0);\n";
            for i in 0..reads {
                fs += &format!("    s += v{i};\n");
            }
            fs += "    o = s * 0.0625;\n";
        }
    }
    fs += "}\n";
    (vs, fs)
}

/// The three vertices: positions (a triangle that covers the 8 x 8 target)
/// and the attribute each varying is made of.
static POS: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
static COL: [f32; 12] = [0.10, 0.20, 0.30, 0.40, 0.50, 0.60, 0.70, 0.80, 0.90, 0.15, 0.25, 0.35];

/// What the shader computes for varying `i`, component `c`, at vertex `j`.
fn vertex_value(i: usize, c: usize, j: usize) -> f32 {
    let sw = [[0, 1, 2, 3], [1, 2, 3, 0], [2, 3, 0, 1], [3, 0, 1, 2]][i % 4];
    COL[4 * j + sw[c]] * 0.5 + i as f32 / 40.0
}

/* ---- the GL, on a stand-in back end ---------------------------------------------------- */

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
unsafe extern "C" fn b_ok() -> i32 {
    0
}

/// What the back end was handed for one application draw, and the pixel the
/// byte model makes of its two programs at the triangle's centre.
#[derive(Clone, Debug)]
struct Seen {
    vs_varyings: u8,
    fs_varyings: u8,
    vs_output_size: u8,
    vs_psize_off: u8,
    /// The vertex program's outputs of the three vertices, by dword.
    vs_out: Vec<std::collections::BTreeMap<u32, u32>>,
    /// The (location, vertex output offset) of each vertex output.
    vs_offsets: Vec<(u8, u8)>,
    /// The fragment program's pixel output 0.
    pixel: Result<[u8; 4], String>,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

fn find<'a>(s: &'a ShcShader, name: &str) -> &'a yakogl::shaderc::ShcAttrib {
    s.attribs[..s.num_attribs as usize]
        .iter()
        .find(|a| a.name.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect::<String>() == name)
        .unwrap_or_else(|| panic!("the vertex record has no attribute {name}"))
}

/// The two programs on the byte model: the vertex program for the three
/// vertices, then the fragment program at the centre.
fn model(vs: &ShcShader, fs: &ShcShader) -> (Vec<std::collections::BTreeMap<u32, u32>>, Result<[u8; 4], String>) {
    let image = |s: &ShcShader| -> Vec<u8> { s.usc_words().iter().flat_map(|w| w.to_le_bytes()).collect() };
    let (pos, col) = (find(vs, "a_pos"), find(vs, "a_c"));
    let mut outs = Vec::new();
    for j in 0..3 {
        let mut inp = usc_exec::sentinel_inputs(1.0e30);
        for c in 0..4 {
            inp.vtxin[pos.vi_reg as usize + c] = POS[4 * j + c].to_bits();
            inp.vtxin[col.vi_reg as usize + c] = COL[4 * j + c].to_bits();
        }
        outs.push(usc_exec::run_binary(&image(vs), &inp, true).unwrap_or_default());
    }
    /* the fragment program: W (4 coefficients), then 16 per vec4 varying in
     * the record's order; a smooth component iterates to the mean of the
     * three vertices, a flat one to its plane constant (coefficient 2 of its
     * four) the last vertex's value */
    let mut inp = usc_exec::sentinel_inputs(1.0e30);
    inp.coeff[0] = 1.0f32.to_bits();
    for (k, v) in fs.varyings[..fs.num_varyings as usize].iter().enumerate() {
        let vt = vs.varyings[..vs.num_varyings as usize]
            .iter()
            .find(|x| x.location == v.location)
            .expect("the vertex stage writes what the fragment stage reads");
        for c in 0..4u32 {
            let at = |j: usize| f32::from_bits(outs[j].get(&(vt.vtxout_off as u32 + c)).copied().unwrap_or(0));
            let (word, off) = if v.interp == 2 { (at(2), 2) } else { ((at(0) + at(1) + at(2)) / 3.0, 0) };
            inp.coeff[4 + 16 * k + off + 4 * c as usize] = word.to_bits();
        }
    }
    let pixel = usc_exec::run_binary(&image(fs), &inp, true)
        .and_then(|o| o.get(&0).copied().ok_or_else(|| format!("nothing in pixel output 0: {o:?}")))
        .map(|w| w.to_le_bytes());
    (outs, pixel)
}

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count == 0 || d.vs.is_null() || d.fs.is_null() {
        return 0;
    }
    let (vs, fs) = (&*d.vs, &*d.fs);
    let (vs_out, pixel) = model(vs, fs);
    SEEN.lock().unwrap().push(Seen {
        vs_varyings: vs.num_varyings,
        fs_varyings: fs.num_varyings,
        vs_output_size: vs.vs_output_size,
        vs_psize_off: vs.vs_psize_off,
        vs_out,
        vs_offsets: vs.varyings[..vs.num_varyings as usize].iter().map(|v| (v.location, v.vtxout_off)).collect(),
        pixel,
    });
    0
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
    name: c"es3-pvr-vout64".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: Some(b_draw),
    flush: Some(b_ok),
    finish: Some(b_ok),
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

/// One context for the whole file; the tests take turns on it and on the
/// compiler (one arena, not reentrant).
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    SEEN.lock().unwrap().clear();
    g
}

fn geti(p: GLenum) -> GLint {
    let mut v: GLint = -1;
    // SAFETY: plain GL call with a real output.
    unsafe { yakogl::gl::state::glGetIntegerv(p, &mut v) };
    v
}

fn text_of(log: &[c_char]) -> String {
    log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        if ok != 1 {
            let mut log = [0 as c_char; 1024];
            glGetShaderInfoLog(s, log.len() as GLsizei, core::ptr::null_mut(), log.as_mut_ptr());
            panic!("did not compile: {}\n{text:?}", text_of(&log));
        }
        s
    }
}

/// The two sources linked: Ok(program) or Err(info log).
fn program((vs, fs): &(String, String)) -> Result<GLuint, String> {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
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
        if ok == 1 {
            return Ok(p);
        }
        let mut log = [0 as c_char; 1024];
        glGetProgramInfoLog(p, log.len() as GLsizei, core::ptr::null_mut(), log.as_mut_ptr());
        glDeleteProgram(p);
        Err(text_of(&log))
    }
}

/// Draws the triangle with `p`; the draw's record is in SEEN afterwards.
fn draw(p: GLuint) {
    // SAFETY: the arrays are statics and outlive the draw.
    unsafe {
        glUseProgram(p);
        yakogl::gl::buffer::glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, COL.as_ptr() as *const c_void);
        glEnableVertexAttribArray(1);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
}

fn quantise(x: f32) -> u8 {
    (x.clamp(0.0, 1.0) * 255.0).round() as u8
}

/* ---- the live half ------------------------------------------------------------------ */

/// Fifteen vec4 varyings (three flat) and gl_PointSize: one program per
/// varying read back, each drawn on its own. The first draw's records are
/// held to the output size; every pixel is held to the arithmetic.
#[test]
fn fifteen_vec4_varyings_draw_and_the_vertex_output_is_sized_for_them() {
    let _g = context();
    let refused_before = backend::draws_refused();
    for k in 0..N {
        let p =
            program(&sources(N, N, Some(k), true)).unwrap_or_else(|e| panic!("v{k}: fifteen vec4 do not link: {e}"));
        draw(p);
        // SAFETY: plain GL calls.
        unsafe {
            assert_eq!(glGetError(), GL_NO_ERROR, "the draw of v{k}");
            glUseProgram(0);
            glDeleteProgram(p);
        }
        let seen = SEEN.lock().unwrap().pop().unwrap_or_else(|| panic!("v{k}: the back end saw no draw"));
        SEEN.lock().unwrap().clear();

        /* the output size: 4 (position) + 15 * 4 + 1 (gl_PointSize) dwords,
         * and the point size after the varyings */
        assert_eq!(seen.vs_varyings as usize, N, "v{k}: the vertex record lists the fifteen");
        assert!(seen.fs_varyings >= 1, "v{k}: the fragment record lists the one it reads");
        assert_eq!(seen.vs_output_size as usize, 4 + 4 * N + 1, "v{k}: the vertex output size in dwords");
        assert_eq!(seen.vs_psize_off as usize, 4 + 4 * N, "v{k}: gl_PointSize after the varyings");

        /* the vertex program, at the offsets the record names */
        for (j, out) in seen.vs_out.iter().enumerate() {
            assert!(!out.is_empty(), "v{k}: the vertex program ran on vertex {j}");
            for c in 0..4 {
                let pos = f32::from_bits(out[&(c as u32)]);
                assert_eq!(pos, POS[4 * j + c], "v{k}: gl_Position[{c}] of vertex {j}");
            }
            for i in 0..N {
                let off = seen.vs_offsets.iter().find(|v| v.0 as usize == i).expect("listed").1;
                for c in 0..4 {
                    let got = f32::from_bits(out[&(off as u32 + c as u32)]);
                    let want = vertex_value(i, c, j);
                    assert!((got - want).abs() < 1e-6, "v{k}: vertex {j} varying {i}[{c}] = {got}, want {want}");
                }
            }
        }

        /* the pixel at the centre */
        let px = seen.pixel.clone().unwrap_or_else(|e| panic!("v{k}: the fragment program: {e}"));
        for c in 0..4 {
            let (flat, want) = if FLAT.contains(&k) {
                (true, quantise(vertex_value(k, c, 2)))
            } else {
                (false, quantise((0..3).map(|j| vertex_value(k, c, j)).sum::<f32>() / 3.0))
            };
            if flat {
                assert_eq!(px[c], want, "v{k} (flat)[{c}]: the pixel is the last vertex's value exactly");
            } else {
                assert!(px[c].abs_diff(want) <= 1, "v{k}[{c}]: pixel {} vs the centre's {want}", px[c]);
            }
        }
    }
    assert_eq!(backend::draws_refused(), refused_before, "no draw was refused");
}

/// All fifteen read in one fragment shader (the case a draw really has): the
/// records carry fifteen on both sides and the sum reaches the pixel.
#[test]
fn fifteen_vec4_varyings_read_by_one_fragment_shader() {
    let _g = context();
    let p = program(&sources(N, N, None, true)).unwrap_or_else(|e| panic!("fifteen vec4 do not link: {e}"));
    draw(p);
    // SAFETY: plain GL calls.
    unsafe {
        assert_eq!(glGetError(), GL_NO_ERROR);
        glUseProgram(0);
        glDeleteProgram(p);
    }
    let seen = SEEN.lock().unwrap().pop().expect("the back end saw the draw");
    assert_eq!((seen.vs_varyings as usize, seen.fs_varyings as usize), (N, N));
    assert_eq!(seen.vs_output_size, 65);
    let px = seen.pixel.expect("the fragment program runs");
    for c in 0..4 {
        let sum: f32 = (0..N)
            .map(|i| {
                if FLAT.contains(&i) {
                    vertex_value(i, c, 2)
                } else {
                    (0..3).map(|j| vertex_value(i, c, j)).sum::<f32>() / 3.0
                }
            })
            .sum();
        let want = quantise(sum * 0.0625);
        assert!(px[c].abs_diff(want) <= 2, "sum[{c}]: pixel {} vs {want}", px[c]);
    }
}

/* ---- the 64-component half ---------------------------------------------------------- */

/// The raise to GL_MAX_VERTEX_OUTPUT_COMPONENTS 64: the limit reads 64 and a
/// vertex shader writing 64 output components (sixteen vec4; the fragment
/// shader reads fifteen of them, ES 3.0's GL_MAX_VARYING_VECTORS) links and
/// draws without a refusal. Nothing in the hardware, the PDS, the USC or the
/// records stops it (plan #359 pvr-vout64-study); the 60 is only the cap.
#[test]
#[ignore = "needs the PowerVR raise to MAX_VERTEX_OUTPUT_COMPONENTS 64"]
fn sixty_four_vertex_output_components_link_and_draw() {
    let _g = context();
    assert_eq!(geti(GL_MAX_VERTEX_OUTPUT_COMPONENTS), 64, "ES 3.0 table 6.31 asks 64");
    let refused_before = backend::draws_refused();
    let p =
        program(&sources(16, N, Some(0), false)).unwrap_or_else(|e| panic!("sixteen vec4 outputs do not link: {e}"));
    draw(p);
    // SAFETY: plain GL calls.
    unsafe {
        assert_eq!(glGetError(), GL_NO_ERROR, "the draw");
        glUseProgram(0);
        glDeleteProgram(p);
    }
    assert_eq!(backend::draws_refused(), refused_before, "the draw was not refused");
    let seen = SEEN.lock().unwrap().pop().expect("the back end saw the draw");
    assert_eq!(seen.vs_varyings, 16, "the vertex record lists the sixteen");
    assert_eq!(seen.fs_varyings as usize, 1, "the fragment shader reads one");
}
