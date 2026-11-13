// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The reported varying limits can be used: a GPU-neutral host test through
//! the public GL API. Features: `gpu-powervr` or `gpu-vivante`, with
//! `host-test` and `shaderc-online`, not `report-es2` (it is not built
//! there). The file is written to run unchanged on `gpu-v3d` once
//! v3d-varyings-raise lands (its cfg already admits it); the commands below
//! list only the two builds that report 15 today.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_varyings_reachable
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_varyings_reachable
//! ```
//!
//! tests/es3_pvr_varyings.rs holds the PowerVR's own coverage (the record
//! layout of fifteen vec4, the W/Z coefficient offsets, ES 1.00 at the
//! limit); this file does not repeat it. What is new: the limit is
//! *queried*, not hard-coded, and the same shapes are asked of whichever GPU
//! is built -- MAX_VARYING_VECTORS vec4, a flat/smooth/centroid mix, scalars,
//! vec2s and vec3s packed up to MAX_VARYING_COMPONENTS, gl_PointCoord beside
//! the maximum, and one vector past the limit.
//!
//! "Every component reaches the output": the host has no executor for the
//! PowerVR or GC7000 shaders (the kick is not run, tests/es3_msaa_gc.rs), so
//! there is no pixel to read back. The check is on the records the draw hands
//! the back end: both stages list each declared varying with all its
//! components, flat as flat, at distinct in-range vertex-output dwords, and
//! agree slot for slot. A readback of the values is a board step (#313,
//! #316). The records carry no centroid bit, so centroid is only checked to
//! link and draw.
#![cfg(all(
    feature = "host-test",
    feature = "shaderc-online",
    any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "gpu-v3d"),
    not(feature = "report-es2")
))]
use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::glGetError;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/// The compiler keeps its output in one arena that every compile resets and
/// is not reentrant: the tests take turns on this lock.
static COMPILING: Mutex<()> = Mutex::new(());

fn text_of(log: &[c_char]) -> String {
    log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

/* ---- a stand-in back end -------------------------------------------------------- */

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

/// What the back end was handed for one application draw: per stage the
/// (components, vertex-output offset, interp) of each varying, and the
/// vertex output size.
#[derive(Clone, Debug)]
struct Seen {
    vs: Vec<(u8, u8, u8)>,
    fs: Vec<(u8, u8, u8)>,
    vs_output_size: u8,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count == 0 || d.vs.is_null() || d.fs.is_null() {
        return 0;
    }
    let (vs, fs) = (&*d.vs, &*d.fs);
    let list = |s: &yakogl::shaderc::ShcShader| -> Vec<(u8, u8, u8)> {
        s.varyings[..s.num_varyings as usize].iter().map(|v| (v.components, v.vtxout_off, v.interp)).collect()
    };
    SEEN.lock().unwrap().push(Seen { vs: list(vs), fs: list(fs), vs_output_size: vs.vs_output_size });
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
    name: c"es3-varyings-reachable".as_ptr() as *const c_char,
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

static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    use yakogl::gl::state::glFinish;
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

fn geti(p: GLenum) -> usize {
    let mut v: GLint = -1;
    // SAFETY: plain GL call with a real output.
    unsafe { yakogl::gl::state::glGetIntegerv(p, &mut v) };
    assert!(v > 0, "query {p:#x} gave {v}");
    v as usize
}

/* ---- generated programs ---------------------------------------------------------- */

/// One varying of a generated program: qualifier, GLSL type, components.
#[derive(Clone)]
struct Var {
    qual: &'static str,
    ty: &'static str,
    comps: usize,
}

fn var(qual: &'static str, ty: &'static str) -> Var {
    let comps = match ty {
        "float" => 1,
        "vec2" => 2,
        "vec3" => 3,
        _ => 4,
    };
    Var { qual, ty, comps }
}

fn n_of(ty: &'static str, qual: &'static str, n: usize) -> Vec<Var> {
    (0..n).map(|_| var(qual, ty)).collect()
}

/// `#version 300 es` sources: the vertex shader writes every component of
/// every varying from the attribute `a_c` (a different scale each); the
/// fragment shader reads every component into the output. `frag_extra` is
/// added to the output (e.g. gl_PointCoord).
fn sources(vars: &[Var], frag_extra: &str) -> (String, String) {
    let mut vs = String::from("#version 300 es\nin highp vec4 a_pos;\nin highp vec4 a_c;\n");
    let mut fs = String::from("#version 300 es\nprecision highp float;\nlayout(location = 0) out highp vec4 o;\n");
    for (i, v) in vars.iter().enumerate() {
        vs += &format!("{} out highp {} v{i};\n", v.qual, v.ty);
        fs += &format!("{} in highp {} v{i};\n", v.qual, v.ty);
    }
    vs += "void main() {\n    gl_Position = a_pos;\n    gl_PointSize = 1.0;\n";
    fs += "void main() {\n    highp vec4 s = vec4(0.0);\n";
    let mut k = 0usize;
    for (i, v) in vars.iter().enumerate() {
        let sw = ["xyzw", "yzwx", "zwxy", "wxyz"][i % 4];
        let rhs = if v.comps == 1 {
            format!("a_c.{} * {}.0", &sw[..1], i + 1)
        } else {
            format!("(a_c.{sw} * {}.0).{}", i + 1, &sw[..v.comps])
        };
        vs += &format!("    v{i} = {rhs};\n");
        for c in 0..v.comps {
            let comp = if v.comps == 1 { String::new() } else { format!(".{}", &"xyzw"[c..c + 1]) };
            fs += &format!("    s.{} += v{i}{comp};\n", &"xyzw"[k % 4..k % 4 + 1]);
            k += 1;
        }
    }
    vs += "}\n";
    fs += &format!("    o = s{frag_extra};\n}}\n");
    (vs, fs)
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
        if ok != 1 {
            let mut log = [0 as c_char; 1024];
            glGetShaderInfoLog(s, log.len() as GLsizei, core::ptr::null_mut(), log.as_mut_ptr());
            glDeleteShader(s);
            return Err(format!("did not compile: {}", text_of(&log)));
        }
        Ok(s)
    }
}

/// The program of `vars`, linked: Ok(program) or Err(the compile or link log).
fn program(vars: &[Var], frag_extra: &str) -> Result<GLuint, String> {
    let (vs, fs) = sources(vars, frag_extra);
    let v = shader(GL_VERTEX_SHADER, &vs)?;
    let f = shader(GL_FRAGMENT_SHADER, &fs)?;
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

fn draw(mode: GLenum) -> Seen {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    static COL: [f32; 12] = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2];
    let refused = backend::draws_refused();
    SEEN.lock().unwrap().clear();
    // SAFETY: the arrays are statics and outlive the draw.
    unsafe {
        yakogl::gl::buffer::glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, COL.as_ptr() as *const c_void);
        glEnableVertexAttribArray(1);
        glDrawArrays(mode, 0, 3);
        yakogl::gl::state::glFinish();
        assert_eq!(glGetError(), GL_NO_ERROR, "the draw");
    }
    assert_eq!(backend::draws_refused(), refused, "the draw was not refused");
    let seen = SEEN.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "one draw reached the back end");
    seen[0].clone()
}

/// Link `vars`, draw, and check that every declared component is carried:
/// both records list each varying with its component count and interp, at
/// distinct in-range vertex-output dwords, the two stages agreeing.
fn link_draw_and_check(vars: &[Var], frag_extra: &str, mode: GLenum) {
    let p = program(vars, frag_extra).unwrap_or_else(|e| panic!("{} varyings do not link: {e}", vars.len()));
    // SAFETY: plain GL call.
    unsafe { glUseProgram(p) };
    let seen = draw(mode);
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(0);
        glDeleteProgram(p);
    }
    let want: usize = vars.iter().map(|v| v.comps).sum();
    for (stage, list) in [("vertex", &seen.vs), ("fragment", &seen.fs)] {
        assert_eq!(list.len(), vars.len(), "the {stage} record lists every varying");
        let mut taken = vec![false; seen.vs_output_size as usize + 4];
        let mut got = 0;
        for (v, (c, off, interp)) in vars.iter().zip(list.iter()) {
            assert_eq!(*c as usize, v.comps, "{stage}: components of a {}", v.ty);
            assert_eq!(*interp == 2, v.qual == "flat", "{stage}: interp {interp} for qualifier '{}'", v.qual);
            for k in 0..*c as usize {
                let slot = *off as usize + k;
                assert!(slot < seen.vs_output_size as usize, "{stage}: slot {slot} past the output size");
                assert!(!taken[slot], "{stage}: two components share vertex output dword {slot}");
                taken[slot] = true;
                got += 1;
            }
        }
        assert_eq!(got, want, "{stage}: every component has a slot");
    }
    assert_eq!(seen.vs, seen.fs, "both stages agree on every slot");
}

/* ---- the tests ------------------------------------------------------------------- */

#[test]
fn the_limits_are_consistent() {
    let _g = context();
    let (vecs, comps) = (geti(GL_MAX_VARYING_VECTORS), geti(GL_MAX_VARYING_COMPONENTS));
    assert!(vecs >= 15, "ES 3.0's minimum is 15 vectors; got {vecs}");
    assert_eq!(comps, vecs * 4, "components are four per vector");
    assert_eq!(geti(GL_MAX_FRAGMENT_INPUT_COMPONENTS), comps);
}

#[cfg_attr(
    feature = "gpu-vivante",
    ignore = "the GC7000 code generator refuses a vertex shader with 15 vec4 outputs: Abi(LoadBalancingTempsUnmeasured { num_out: 15, temps: 17, temps_max: 16 }), online/vivante abi.rs:250 (SHADERC_NOTES.md:461)"
)]
#[test]
fn max_varying_vectors_vec4_link_and_every_component_is_carried() {
    let _g = context();
    link_draw_and_check(&n_of("vec4", "", geti(GL_MAX_VARYING_VECTORS)), "", GL_TRIANGLES);
}

#[cfg_attr(
    feature = "gpu-vivante",
    ignore = "the GC7000 code generator refuses a vertex shader with 15 vec4 outputs: Abi(LoadBalancingTempsUnmeasured { num_out: 15, temps: 17, temps_max: 16 }), online/vivante abi.rs:250 (SHADERC_NOTES.md:461)"
)]
#[test]
fn flat_smooth_centroid_mix_at_the_limit() {
    let _g = context();
    let n = geti(GL_MAX_VARYING_VECTORS);
    let quals = ["", "flat", "centroid", "smooth", "flat", "centroid"];
    let vars: Vec<Var> = (0..n).map(|i| var(quals[i % quals.len()], "vec4")).collect();
    link_draw_and_check(&vars, "", GL_TRIANGLES);
}

/// 5 vec4 + 10 vec2 + 20 float: 60 components, packed into 15 rows by the
/// Appendix A rules (ES 3.00 section 11.1.2.1), a set that fits the queried
/// MAX_VARYING_COMPONENTS exactly when that is 60.
#[cfg_attr(
    any(feature = "gpu-powervr", feature = "gpu-vivante"),
    ignore = "scalar/vec2/vec3 varyings are not packed into rows: PowerVR's records hold 16 varyings (shaderc ShcShader::varyings, 'more varyings than the record holds'), GC7000's link gives each its own register (Select TooManyIo, online/vivante link::by_register)"
)]
#[test]
fn scalars_and_vec2_pack_up_to_max_varying_components() {
    let _g = context();
    let mut vars = n_of("vec4", "", 5);
    vars.extend(n_of("vec2", "", 10));
    vars.extend(n_of("float", "", 20));
    assert!(vars.iter().map(|v| v.comps).sum::<usize>() <= geti(GL_MAX_VARYING_COMPONENTS));
    link_draw_and_check(&vars, "", GL_TRIANGLES);
}

#[cfg_attr(
    any(feature = "gpu-powervr", feature = "gpu-vivante"),
    ignore = "scalar/vec2/vec3 varyings are not packed into rows: PowerVR's records hold 16 varyings (shaderc ShcShader::varyings, 'more varyings than the record holds'), GC7000's link gives each its own register (Select TooManyIo, online/vivante link::by_register)"
)]
#[test]
fn thirty_vec2_and_fifteen_vec3_plus_fifteen_float_pack_to_sixty_components() {
    let _g = context();
    link_draw_and_check(&n_of("vec2", "", 30), "", GL_TRIANGLES);
    let mut v = n_of("vec3", "", 15);
    v.extend(n_of("float", "", 15));
    link_draw_and_check(&v, "", GL_TRIANGLES);
}

#[cfg_attr(
    feature = "gpu-vivante",
    ignore = "the GC7000 code generator refuses a vertex shader with 15 vec4 outputs: Abi(LoadBalancingTempsUnmeasured { num_out: 15, temps: 17, temps_max: 16 }), online/vivante abi.rs:250 (SHADERC_NOTES.md:461)"
)]
#[test]
fn point_coord_beside_the_maximum_varyings() {
    let _g = context();
    let n = geti(GL_MAX_VARYING_VECTORS);
    link_draw_and_check(&n_of("vec4", "", n), " + vec4(gl_PointCoord, 0.0, 0.0)", GL_POINTS);
}

#[test]
fn one_vector_past_the_limit_does_not_link() {
    let _g = context();
    let n = geti(GL_MAX_VARYING_VECTORS) + 1;
    match program(&n_of("vec4", "", n), "") {
        Ok(p) => {
            // SAFETY: plain GL call.
            unsafe { glDeleteProgram(p) };
            panic!("{n} vec4 varyings linked past MAX_VARYING_VECTORS");
        }
        /* ES 3.00 section 2.12.7 makes this a link error; PowerVR refuses
         * earlier, at glCompileShader (see the report): either way no
         * program exists. */
        Err(log) => assert!(!log.is_empty(), "the log says why"),
    }
    // SAFETY: plain GL call.
    assert_eq!(unsafe { glGetError() }, GL_NO_ERROR, "a failed link is not an error");
}
