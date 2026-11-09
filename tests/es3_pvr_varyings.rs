// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_MAX_VARYING_VECTORS 15 on the PowerVR: ES 3.0's minimum, carried end
//! to end by the online compiler and the back end (gl/context.rs
//! MAX_VARYING_VECTORS says where each bound is).
//!
//! The first half asks the compiler itself: fifteen vec4 varyings compile,
//! link and lay out as the records say they do (fifteen listed in each
//! stage, 64 dwords of vertex output, W iterated), also with
//! gl_FragCoord.z and gl_PointCoord read beside them -- the fullest
//! coefficient layout, whose last iterator starts at coefficient 240 of the
//! 252 the `ditrp` encodes -- and with a flat one last. The second half is
//! the GL: the four limits read 15 / 60, a program of fifteen vec4 varyings
//! links and draws (no error, no refused draw, the records the draw hands
//! the back end carry the fifteen), and an ES 1.00 program of sixteen does
//! not link, by the name of the limit. Under `report-es2` the answer stays 8
//! (tests/es2_report_guard.rs); this file is not built there.
//!
//! The dEQP cases this answers on the host (API verdicts only, HOST-ONLY):
//! implementation_limits.max_varying_vectors, max_varying_components,
//! max_fragment_input_components and the state_query.integers forms of
//! them; max_vertex_output_components still asks 64 of our 60.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_varyings
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context::MAX_VARYING_VECTORS;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// The fifteen of ES 3.0's table 6.31 (GL_MAX_VARYING_VECTORS minimum).
const N: usize = 15;

/* ---- the bounds the cap rests on ---------------------------------------------- */

const _: () = assert!(MAX_VARYING_VECTORS == N);
/* the records (both stages) and the PDS coefficient program hold the fifteen */
const _: () = assert!(N <= yakogl::pvr::ffi::PVR_PDS_COEFF_MAX_VARYINGS as usize);
const _: () = assert!(N <= 16); /* ShcShader::varyings, abi.rs widths, chain.rs comps */
/* The `ditrp` coefficient offset: 6 bits in units of 4 (isa_tables.rs
 * `coff`, Shr2), so a varying's set must start below 252. With W (4),
 * gl_FragCoord.z (4) and gl_PointCoord (8) ahead of them, the fifteenth
 * vec4 starts at 16 + 14 * 16 = 240; a sixteenth would start at 256. */
const COFF_LIMIT: usize = 63 * 4;
const _: () = assert!(16 + (N - 1) * 16 < COFF_LIMIT);
const _: () = assert!(16 + N * 16 >= COFF_LIMIT);

/* ---- the compiler, asked directly ---------------------------------------------- */

/// The compiler keeps its output in one arena that every compile resets,
/// and is not reentrant: the direct compiles and the GL's (through the
/// context below) take turns on this one lock.
static COMPILING: Mutex<()> = Mutex::new(());

fn text_of(log: &[c_char]) -> String {
    log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

fn compile(stage: u32, src: &str) -> Result<ShcShader, String> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let text = format!("{src}\0");
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(stage, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(out)
    } else {
        Err(text_of(&log))
    }
}

/// Both stages compiled and linked: (vertex record, fragment record).
fn build(vs: &str, fs: &str) -> Result<(ShcShader, ShcShader), String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    yakogl::shaderc::online::backend::forget_compiled();
    let vso = compile(SHC_STAGE_VERTEX, vs).map_err(|m| format!("vertex: {m}"))?;
    let mut fso = compile(SHC_STAGE_FRAGMENT, fs).map_err(|m| format!("fragment: {m}"))?;
    let link = yakogl::shaderc::online::SHC_ONLINE.link.expect("the online back end links");
    let mut log = [0 as c_char; 1024];
    // SAFETY: two distinct compiled shaders and a log of the size announced.
    let rc = unsafe { link(&vso, &mut fso, log.as_mut_ptr(), log.len()) };
    if rc != 0 {
        return Err(format!("link: {}", text_of(&log)));
    }
    Ok((vso, fso))
}

/// Which GLSL ES the sources are written in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Es {
    V100,
    V300,
}

/// A vertex shader writing `n` vec4 varyings `v0..v<n>` from one attribute
/// (each a different swizzle and scale, so no two are the same value) and
/// a fragment shader summing them all. `qual_last` qualifies the last of
/// them (e.g. `flat`); `extra` is read into the fragment colour beside them.
fn sources(es: Es, n: usize, qual_last: &str, extra: &str) -> (String, String) {
    let (mut vs, mut fs, out, in_, color) = match es {
        Es::V300 => (
            String::from("#version 300 es\nin highp vec4 a_pos;\nin highp vec4 a_c;\n"),
            String::from("#version 300 es\nprecision highp float;\nlayout(location = 0) out highp vec4 o;\n"),
            "out",
            "in",
            "o",
        ),
        Es::V100 => (
            String::from("attribute highp vec4 a_pos;\nattribute highp vec4 a_c;\n"),
            String::from("precision highp float;\n"),
            "varying",
            "varying",
            "gl_FragColor",
        ),
    };
    for i in 0..n {
        let q = if i + 1 == n { qual_last } else { "" };
        vs += &format!("{q} {out} highp vec4 v{i};\n");
        fs += &format!("{q} {in_} highp vec4 v{i};\n");
    }
    vs += "void main() {\n    gl_Position = a_pos;\n    gl_PointSize = 1.0;\n";
    for i in 0..n {
        let sw = ["xyzw", "yzwx", "zwxy", "wxyz"][i % 4];
        vs += &format!("    v{i} = a_c.{sw} * {}.0;\n", i + 1);
    }
    vs += "}\n";
    fs += "void main() {\n    highp vec4 s = vec4(0.0);\n";
    for i in 0..n {
        fs += &format!("    s += v{i};\n");
    }
    fs += &format!("    {color} = s{extra};\n}}\n");
    (vs, fs)
}

/// The varyings of a record: (location, components, vertex-output offset).
fn varyings(s: &ShcShader) -> Vec<(u8, u8, u8)> {
    s.varyings[..s.num_varyings as usize].iter().map(|v| (v.location, v.components, v.vtxout_off)).collect()
}

/// Fifteen vec4 at offsets 4, 8, ..., 60 after the position's four dwords.
fn fifteen_vec4_layout() -> Vec<(u8, u8, u8)> {
    (0..N as u8).map(|i| (i, 4, 4 + 4 * i)).collect()
}

#[test]
fn fifteen_vec4_varyings_compile_link_and_fill_the_records() {
    let (vs, fs) = sources(Es::V300, N, "", "");
    let (v, f) = build(&vs, &fs).unwrap_or_else(|e| panic!("fifteen vec4 varyings do not build: {e}"));
    assert_eq!(v.num_varyings as usize, N, "the vertex record lists the fifteen");
    assert_eq!(f.num_varyings as usize, N, "the fragment record lists the fifteen");
    assert_eq!(varyings(&v), fifteen_vec4_layout(), "the vertex outputs, packed after gl_Position");
    assert_eq!(varyings(&f), fifteen_vec4_layout(), "the fragment side agrees on every offset");
    /* 4 (position) + 60 (varyings) + 1 (gl_PointSize) */
    assert_eq!(v.vs_output_size, 65, "the vertex output size in dwords");
    assert_eq!(v.vs_psize_off, 64, "gl_PointSize after the varyings");
    assert_eq!(f.uses_w, 1, "W is iterated for the perspective divide");
    assert!(f.usc_dw > 0 && !f.usc.is_null(), "a fragment program was built");
}

#[test]
fn fifteen_vec4_beside_frag_coord_z_and_point_coord_build() {
    /* W, Z and the point coordinate ahead of the varyings: the last
     * iterator's coefficients start at 240 */
    let (vs, fs) = sources(Es::V300, N, "", " + vec4(gl_PointCoord, gl_FragCoord.z, 0.0)");
    let (v, f) = build(&vs, &fs).unwrap_or_else(|e| panic!("the fullest coefficient layout does not build: {e}"));
    assert_eq!((v.num_varyings as usize, f.num_varyings as usize), (N, N));
    assert_eq!((f.uses_w, f.uses_z), (1, 1), "W and Z iterated");
    assert_eq!(varyings(&f), fifteen_vec4_layout());
}

#[test]
fn fifteen_vec4_with_a_flat_one_build() {
    let (vs, fs) = sources(Es::V300, N, "flat", "");
    let (v, f) = build(&vs, &fs).unwrap_or_else(|e| panic!("fifteen with a flat last does not build: {e}"));
    assert_eq!((v.num_varyings as usize, f.num_varyings as usize), (N, N));
    assert_eq!(f.varyings[N - 1].interp, v.varyings[N - 1].interp, "the flat one is flat on both sides");
}

/// GLSL ES 1.00's `varying` count is checked against the limit at the link
/// (reflect::varying_budget, Appendix A packing): fifteen vec4 link -- the
/// corpus pair es100/varyings15, refused at 8 until now -- and sixteen are
/// refused by the limit's name. (An ES 3.00 program's count is not checked
/// there: `varying_budget` returns early for ES 3.00 sources, and the ABI's
/// table of 16 is what refuses more.)
#[test]
fn es100_fifteen_link_and_sixteen_are_refused_by_the_limit() {
    let (vs, fs) = sources(Es::V100, N, "", "");
    let (v, f) = build(&vs, &fs).unwrap_or_else(|e| panic!("fifteen ES 1.00 varyings do not build: {e}"));
    assert_eq!((v.num_varyings as usize, f.num_varyings as usize), (N, N));
    let (vs, fs) = sources(Es::V100, N + 1, "", "");
    let err = build(&vs, &fs).err().expect("sixteen ES 1.00 varyings are refused");
    assert!(
        err.starts_with("link:") && err.contains("MAX_VARYING_VECTORS = 15"),
        "refused at the link by the limit's name and value: {err}"
    );
}

/* ---- the GL, on a stand-in back end -------------------------------------------- */

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

/// What the back end was handed for each application draw: the two
/// records' varying counts and the vertex output size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Seen {
    vs_varyings: u8,
    fs_varyings: u8,
    vs_output_size: u8,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count == 0 || d.vs.is_null() || d.fs.is_null() {
        return 0;
    }
    let (vs, fs) = (&*d.vs, &*d.fs);
    SEEN.lock().unwrap().push(Seen {
        vs_varyings: vs.num_varyings,
        fs_varyings: fs.num_varyings,
        vs_output_size: vs.vs_output_size,
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
    name: c"es3-pvr-varyings".as_ptr() as *const c_char,
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

/// One context for the whole file; the tests take turns on it (and on the
/// compiler: `COMPILING`).
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    use yakogl::gl::state::{glFinish, glGetError};
    let g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
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

mod gl {
    use super::*;
    use std::ffi::CString;
    use yakogl::gl::draw::glDrawArrays;
    use yakogl::gl::program::*;
    use yakogl::gl::shader::*;
    use yakogl::gl::state::glGetError;
    use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};

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

    /// The program of `n` varyings, linked: Ok(program) or Err(info log).
    fn program(es: Es, n: usize) -> Result<GLuint, String> {
        let (vs, fs) = sources(es, n, "", "");
        let (v, f) = (shader(GL_VERTEX_SHADER, &vs), shader(GL_FRAGMENT_SHADER, &fs));
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

    fn draw_triangle() {
        static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
        static COL: [f32; 12] = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2];
        // SAFETY: the arrays are statics and outlive the draw.
        unsafe {
            yakogl::gl::buffer::glBindBuffer(GL_ARRAY_BUFFER, 0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, COL.as_ptr() as *const c_void);
            glEnableVertexAttribArray(1);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            yakogl::gl::state::glFinish();
        }
    }

    #[test]
    fn the_four_limits_read_fifteen_vectors_and_sixty_components() {
        let _g = context();
        assert_eq!(geti(GL_MAX_VARYING_VECTORS), 15, "GL_MAX_VARYING_VECTORS: ES 3.0 table 6.31 asks 15");
        assert_eq!(geti(GL_MAX_VARYING_COMPONENTS), 60, "GL_MAX_VARYING_COMPONENTS: 15 * 4, ES 3.0 asks 60");
        assert_eq!(geti(GL_MAX_FRAGMENT_INPUT_COMPONENTS), 60, "GL_MAX_FRAGMENT_INPUT_COMPONENTS: ES 3.0 asks 60");
        assert_eq!(
            geti(GL_MAX_VERTEX_OUTPUT_COMPONENTS),
            60,
            "GL_MAX_VERTEX_OUTPUT_COMPONENTS: 15 * 4 (ES 3.0 asks 64)"
        );
        // SAFETY: plain GL call.
        assert_eq!(unsafe { glGetError() }, GL_NO_ERROR);
    }

    #[test]
    fn a_program_of_fifteen_vec4_varyings_links_and_draws() {
        let _g = context();
        let refused_before = backend::draws_refused();
        let p = program(Es::V300, N).unwrap_or_else(|e| panic!("fifteen vec4 varyings do not link: {e}"));
        // SAFETY: plain GL call.
        unsafe { glUseProgram(p) };
        draw_triangle();
        // SAFETY: plain GL call.
        assert_eq!(unsafe { glGetError() }, GL_NO_ERROR, "the draw");
        assert_eq!(backend::draws_refused(), refused_before, "the draw was not refused");
        let seen = SEEN.lock().unwrap().clone();
        assert_eq!(
            seen,
            vec![Seen { vs_varyings: N as u8, fs_varyings: N as u8, vs_output_size: 65 }],
            "the records the back end was handed carry the fifteen (and gl_PointSize)"
        );
        // SAFETY: plain GL calls.
        unsafe {
            glUseProgram(0);
            glDeleteProgram(p);
        }
    }

    /// ES 1.00 through the GL: fifteen `varying vec4` link (gl_test case 97
    /// "varyings15", refused on this GPU until now) and a sixteenth is
    /// refused at the link, the info log naming the limit.
    #[test]
    fn es100_fifteen_varyings_link_and_a_sixteenth_is_refused_by_the_limit_s_name() {
        let _g = context();
        let p = program(Es::V100, N).unwrap_or_else(|e| panic!("fifteen ES 1.00 varyings do not link: {e}"));
        // SAFETY: plain GL call.
        unsafe { glDeleteProgram(p) };
        let log = match program(Es::V100, N + 1) {
            Ok(p) => {
                // SAFETY: plain GL call.
                unsafe { glDeleteProgram(p) };
                panic!("sixteen ES 1.00 varyings linked past MAX_VARYING_VECTORS = 15")
            }
            Err(log) => log,
        };
        assert!(log.contains("MAX_VARYING_VECTORS = 15"), "the info log names the limit: {log}");
        // SAFETY: plain GL call.
        assert_eq!(unsafe { glGetError() }, GL_NO_ERROR, "a failed link is not an error");
    }
}
