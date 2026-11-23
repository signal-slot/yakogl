// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! dEQP-GLES3 `functional.shaders.texture_functions` on the PowerVR online
//! back end: the shaders that the board configuration once refused (class A:
//! textureGrad/textureOffset-style variants, class C: textureGrad,
//! textureGradOffset and textureProjGrad on cube, 3D and 2D-array samplers,
//! the "Usc refusal", class D) compile, link and draw. commits f9744f26 and
//! 92b93c23 fixed them; the refusals reproduce only before f9744f26 with
//! native 2D arrays on and native 3D off. Nothing pinned them.
//!
//! Each shader is dEQP's own shape: one sampler `s`, one look-up whose
//! result goes to the output (fragment stage) or to a varying (vertex
//! stage), with the plain shader of the other stage. A case is run for every
//! setting of the three layout switches (`pvr_native_cube`,
//! `pvr_native_array`, `pvr_native_3d`), off and on, through the GL API on
//! the host: the shader compiles, the program links, and a draw with a
//! complete texture of the sampler's kind bound reaches the back end (the
//! draw is where the GL core builds the program's draw-time variants and
//! the hidden layer-count rows). The host cannot render: the verdicts are
//! the compile, the link and the draw's acceptance, HOST-ONLY like
//! `tools/cts/host-deqp-run.sh`'s.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_texfn
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use core::ffi::{c_char, c_void, CStr};
use std::ffi::CString;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one --- */

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

/// Draws the back end received.
static DRAWS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    assert!(!d.vs.is_null() && !d.fs.is_null(), "the draw carries both stages' records");
    DRAWS.fetch_add(1, Ordering::SeqCst);
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
    name: c"es3-pvr-texfn".as_ptr() as *const c_char,
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
    max_samples: 0,
};

/// One context for the whole file; the tests take turns on it. The layout
/// switches are process-wide, so the lock also serialises their settings.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(64, 64).is_null());
        }
    });
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glUseProgram(0);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

/// The three switches, set for the closure and back at their defaults after
/// it, panic or not. The caller holds [`context`]'s lock.
fn with_switches<T>(cube: bool, array: bool, vol: bool, f: impl FnOnce() -> T) -> T {
    struct Back;
    impl Drop for Back {
        fn drop(&mut self) {
            yakogl::shaderc::pvr_set_native_cube(yakogl::shaderc::PVR_NATIVE_CUBE_DEFAULT);
            yakogl::shaderc::pvr_set_native_array(yakogl::shaderc::PVR_NATIVE_ARRAY_DEFAULT);
            yakogl::shaderc::pvr_set_native_3d(yakogl::shaderc::PVR_NATIVE_3D_DEFAULT);
        }
    }
    let _back = Back;
    yakogl::shaderc::pvr_set_native_cube(i32::from(cube));
    yakogl::shaderc::pvr_set_native_array(i32::from(array));
    yakogl::shaderc::pvr_set_native_3d(i32::from(vol));
    f()
}

/* ---- the GL side -------------------------------------------------------- */

fn shader(ty: GLenum, text: &str) -> Result<GLuint, String> {
    let c = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source and a log buffer.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [c.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        if ok == 1 {
            return Ok(s);
        }
        let mut log = [0 as c_char; 1024];
        glGetShaderInfoLog(s, 1024, core::ptr::null_mut(), log.as_mut_ptr());
        glDeleteShader(s);
        Err(format!("compile: {}", CStr::from_ptr(log.as_ptr()).to_string_lossy()))
    }
}

/// Both stages compiled and linked: the program, or the refusal's text.
fn program(vs_src: &str, fs_src: &str) -> Result<GLuint, String> {
    let vs = shader(GL_VERTEX_SHADER, vs_src).map_err(|e| format!("vertex {e}"))?;
    let fs = shader(GL_FRAGMENT_SHADER, fs_src).map_err(|e| format!("fragment {e}"))?;
    // SAFETY: plain GL calls on handles this test owns.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, vs);
        glAttachShader(p, fs);
        glLinkProgram(p);
        glDeleteShader(vs);
        glDeleteShader(fs);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        if ok == 1 {
            return Ok(p);
        }
        let mut log = [0 as c_char; 2048];
        glGetProgramInfoLog(p, 2048, core::ptr::null_mut(), log.as_mut_ptr());
        glDeleteProgram(p);
        Err(format!("link: {}", CStr::from_ptr(log.as_ptr()).to_string_lossy()))
    }
}

/// What a sampler type needs of its texture: the bind target, the sized
/// format (integer samplers an integer one, shadow samplers a depth one) and
/// the storage call's shape.
struct Kind {
    target: GLenum,
    format: GLenum,
    shadow: bool,
}

fn kind_of(sampler: &str) -> Kind {
    let shadow = sampler.ends_with("Shadow");
    let format = if shadow {
        GL_DEPTH_COMPONENT16
    } else if sampler.starts_with('i') {
        GL_RGBA8I
    } else if sampler.starts_with('u') {
        GL_RGBA8UI
    } else {
        GL_RGBA8
    };
    let target = if sampler.contains("Cube") {
        GL_TEXTURE_CUBE_MAP
    } else if sampler.contains("2DArray") {
        GL_TEXTURE_2D_ARRAY
    } else if sampler.contains("3D") {
        GL_TEXTURE_3D
    } else {
        GL_TEXTURE_2D
    };
    Kind { target, format, shadow }
}

/// Draw `p` (sampler `s` on unit 0) with a complete one-level 8-texel
/// texture of `kind` bound, positions from a client array. The draw must be
/// accepted and reach the back end.
fn draw_with_texture(p: GLuint, kind: &Kind) -> Result<(), String> {
    static POS: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0];
    // SAFETY: plain GL calls on a linked program of this context; every
    // pointer stays alive through the draw.
    unsafe {
        let mut tex: GLuint = 0;
        glActiveTexture(GL_TEXTURE0);
        glGenTextures(1, &mut tex);
        glBindTexture(kind.target, tex);
        match kind.target {
            GL_TEXTURE_3D | GL_TEXTURE_2D_ARRAY => glTexStorage3D(kind.target, 1, kind.format, 8, 8, 4),
            _ => glTexStorage2D(kind.target, 1, kind.format, 8, 8),
        }
        glTexParameteri(kind.target, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(kind.target, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        if kind.shadow {
            glTexParameteri(kind.target, GL_TEXTURE_COMPARE_MODE, GL_COMPARE_REF_TO_TEXTURE as GLint);
        }
        glUseProgram(p);
        let loc = glGetAttribLocation(p, c"a_pos".as_ptr());
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        if loc >= 0 {
            glVertexAttribPointer(loc as u32, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
            glEnableVertexAttribArray(loc as u32);
        }
        let before = DRAWS.load(Ordering::SeqCst);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        let e = glGetError();
        if loc >= 0 {
            glDisableVertexAttribArray(loc as u32);
        }
        glUseProgram(0);
        glBindTexture(kind.target, 0);
        glDeleteTextures(1, &tex);
        // Drain anything the cleanup raised.
        while glGetError() != GL_NO_ERROR {}
        if e != GL_NO_ERROR {
            return Err(format!("draw: GL error {e:#x}"));
        }
        if DRAWS.load(Ordering::SeqCst) != before + 1 {
            return Err("draw: the back end did not receive it".into());
        }
    }
    Ok(())
}

/* ---- the shaders ------------------------------------------------------- */

const HEAD: &str = "#version 300 es\nprecision highp float;\nprecision highp int;\nprecision highp sampler2D;\n\
                    precision highp sampler3D;\nprecision highp samplerCube;\nprecision highp sampler2DArray;\n\
                    precision highp sampler2DShadow;\nprecision highp samplerCubeShadow;\n\
                    precision highp sampler2DArrayShadow;\nprecision highp isampler2D;\nprecision highp isampler3D;\n\
                    precision highp isamplerCube;\nprecision highp isampler2DArray;\nprecision highp usampler2D;\n\
                    precision highp usampler3D;\nprecision highp usamplerCube;\nprecision highp usampler2DArray;\n";

/// The uniforms the look-ups draw on: a coordinate (and its projective
/// form), two gradients, a level.
const UNIFORMS: &str = "uniform vec4 c;\nuniform vec4 gx;\nuniform vec4 gy;\nuniform float l;\n";

/// `s` declared as `sampler`, the look-up `e` in the fragment stage.
fn fragment(sampler: &str, e: &str) -> String {
    format!("{HEAD}uniform {sampler} s;\n{UNIFORMS}layout(location=0) out vec4 o;\nvoid main()\n{{\n    o = vec4({e});\n}}\n")
}

/// ... and in the vertex stage, through a varying.
fn vertex(sampler: &str, e: &str) -> String {
    format!(
        "{HEAD}uniform {sampler} s;\n{UNIFORMS}in vec4 a_pos;\nout vec4 v;\nvoid main()\n{{\n    gl_Position = a_pos;\n    v = vec4({e});\n}}\n"
    )
}

const VS_PLAIN: &str =
    "#version 300 es\nprecision highp float;\nin vec4 a_pos;\nvoid main()\n{\n    gl_Position = a_pos;\n}\n";
const FS_PLAIN: &str =
    "#version 300 es\nprecision highp float;\nlayout(location=0) out vec4 o;\nvoid main()\n{\n    o = vec4(1.0);\n}\n";

/// Every case: (sampler, look-up with `s`, `c`, `gx`, `gy`, `l`). Offsets
/// are constants, as ES 3.00 8.8 requires. The look-ups dEQP's
/// texture_functions group makes of each sampler: grad, gradoffset,
/// projgrad, projgradoffset, offset, lodoffset and projlodoffset, wherever
/// ES 3.00 has the overload. A sample of the 2D forms stays in as the
/// control: it was never refused.
const CASES: &[(&str, &str)] = &[
    /* sampler2D: the control */
    ("sampler2D", "textureGrad(s, c.xy, gx.xy, gy.xy)"),
    ("sampler2D", "textureGradOffset(s, c.xy, gx.xy, gy.xy, ivec2(3, -2))"),
    ("sampler2D", "textureProjGrad(s, c.xyz, gx.xy, gy.xy)"),
    ("sampler2D", "textureProjGrad(s, c, gx.xy, gy.xy)"),
    ("sampler2D", "textureProjGradOffset(s, c.xyz, gx.xy, gy.xy, ivec2(-1, 4))"),
    ("sampler2D", "textureOffset(s, c.xy, ivec2(1, -3))"),
    ("sampler2D", "textureLodOffset(s, c.xy, l, ivec2(-8, 7))"),
    /* samplerCube: no offset forms exist */
    ("samplerCube", "textureGrad(s, c.xyz, gx.xyz, gy.xyz)"),
    ("samplerCube", "textureLod(s, c.xyz, l)"),
    ("isamplerCube", "vec4(textureGrad(s, c.xyz, gx.xyz, gy.xyz))"),
    ("usamplerCube", "vec4(textureGrad(s, c.xyz, gx.xyz, gy.xyz))"),
    /* sampler3D */
    ("sampler3D", "textureGrad(s, c.xyz, gx.xyz, gy.xyz)"),
    ("sampler3D", "textureGradOffset(s, c.xyz, gx.xyz, gy.xyz, ivec3(3, -2, 1))"),
    ("sampler3D", "textureProjGrad(s, c, gx.xyz, gy.xyz)"),
    ("sampler3D", "textureProjGradOffset(s, c, gx.xyz, gy.xyz, ivec3(-4, 2, 5))"),
    ("sampler3D", "textureOffset(s, c.xyz, ivec3(2, -3, 1))"),
    ("sampler3D", "textureLodOffset(s, c.xyz, l, ivec3(-8, 7, -2))"),
    ("sampler3D", "textureProjLodOffset(s, c, l, ivec3(1, 2, -3))"),
    ("isampler3D", "vec4(textureGradOffset(s, c.xyz, gx.xyz, gy.xyz, ivec3(1, 2, 3)))"),
    ("usampler3D", "vec4(textureProjGrad(s, c, gx.xyz, gy.xyz))"),
    /* sampler2DArray */
    ("sampler2DArray", "textureGrad(s, c.xyz, gx.xy, gy.xy)"),
    ("sampler2DArray", "textureGradOffset(s, c.xyz, gx.xy, gy.xy, ivec2(3, -2))"),
    ("sampler2DArray", "textureOffset(s, c.xyz, ivec2(-3, 5))"),
    ("sampler2DArray", "textureLodOffset(s, c.xyz, l, ivec2(-8, 7))"),
    ("isampler2DArray", "vec4(textureGradOffset(s, c.xyz, gx.xy, gy.xy, ivec2(2, 1)))"),
    ("usampler2DArray", "vec4(textureGrad(s, c.xyz, gx.xy, gy.xy))"),
    /* shadow forms: 2D, cube and 2D array only (no 3D shadow in ES 3.00) */
    ("sampler2DShadow", "textureGrad(s, c.xyz, gx.xy, gy.xy)"),
    ("sampler2DShadow", "textureGradOffset(s, c.xyz, gx.xy, gy.xy, ivec2(3, -2))"),
    ("sampler2DShadow", "textureProjGrad(s, c, gx.xy, gy.xy)"),
    ("sampler2DShadow", "textureProjGradOffset(s, c, gx.xy, gy.xy, ivec2(-1, 4))"),
    ("sampler2DShadow", "textureOffset(s, c.xyz, ivec2(1, -3))"),
    ("sampler2DShadow", "textureLodOffset(s, c.xyz, l, ivec2(-8, 7))"),
    ("samplerCubeShadow", "textureGrad(s, c, gx.xyz, gy.xyz)"),
    ("sampler2DArrayShadow", "textureGrad(s, c, gx.xy, gy.xy)"),
    ("sampler2DArrayShadow", "textureGradOffset(s, c, gx.xy, gy.xy, ivec2(3, -2))"),
];

/// Fragment-only forms: the implicit-derivative look-ups and the bias, which
/// ES 3.00 allows in a fragment shader alone.
const FRAGMENT_ONLY: &[(&str, &str)] = &[
    ("samplerCube", "texture(s, c.xyz, l)"),
    ("sampler3D", "textureOffset(s, c.xyz, ivec3(4, -1, 2), l)"),
    ("sampler3D", "textureProjOffset(s, c, ivec3(4, -1, 2), l)"),
    ("sampler2DArray", "textureOffset(s, c.xyz, ivec2(3, -2), l)"),
    ("sampler2DArray", "texture(s, c.xyz)"),
    ("sampler2DArrayShadow", "texture(s, c)"),
    ("sampler2DShadow", "textureProjOffset(s, c, ivec2(1, 2))"),
];

/// One case under one setting of the switches, in one stage: `Ok`, or what
/// refused.
fn run_case(sampler: &str, e: &str, vertex_stage: bool, cube: bool, array: bool, vol: bool) -> Result<(), String> {
    with_switches(cube, array, vol, || {
        let p = if vertex_stage {
            program(&vertex(sampler, e), FS_PLAIN)
        } else {
            program(VS_PLAIN, &fragment(sampler, e))
        }?;
        let r = draw_with_texture(p, &kind_of(sampler));
        // SAFETY: plain GL call on a program this test owns.
        unsafe { glDeleteProgram(p) };
        r
    })
}

/// Every case, every stage it has, all eight settings of the switches. All
/// failures are collected, so a regression lists every shader and setting it
/// touches.
fn sweep(cases: &[(&str, &str)], vertex_too: bool) -> Vec<String> {
    let mut failed = Vec::new();
    let _g = context();
    for &(sampler, e) in cases {
        for vertex_stage in if vertex_too { vec![false, true] } else { vec![false] } {
            for bits in 0..8u32 {
                let (cube, array, vol) = (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0);
                if let Err(why) = run_case(sampler, e, vertex_stage, cube, array, vol) {
                    failed.push(format!(
                        "{} stage, {sampler} s: {e}; native cube {cube}, array {array}, 3d {vol}: {why}",
                        if vertex_stage { "vertex" } else { "fragment" }
                    ));
                }
            }
        }
    }
    failed
}

/// Classes A, C and D: grad, gradoffset, projgrad, offset and lodoffset on
/// 2D, cube, 3D and 2D-array samplers and their shadow forms compile, link
/// and draw in both stages, with native cube, arrays and 3D each off and on.
#[test]
fn texture_functions_grad_and_offset_compile_link_and_draw() {
    let failed = sweep(CASES, true);
    assert!(failed.is_empty(), "{} refused:\n{}", failed.len(), failed.join("\n"));
}

/// The fragment-only forms of the same samplers (implicit derivatives, the
/// bias).
#[test]
fn texture_functions_fragment_only_forms_compile_link_and_draw() {
    let failed = sweep(FRAGMENT_ONLY, false);
    assert!(failed.is_empty(), "{} refused:\n{}", failed.len(), failed.join("\n"));
}

/// The case tables cover the three kinds of sampler the refusals were on, in
/// grad, offset and projective forms: a guard against a table edit that
/// drops a class.
#[test]
fn the_tables_cover_cube_3d_and_array() {
    let has = |sampler: &str, f: &str| CASES.iter().any(|(s, e)| s.contains(sampler) && e.contains(f));
    for sampler in ["Cube", "3D", "2DArray"] {
        assert!(has(sampler, "textureGrad("), "{sampler}: textureGrad");
    }
    for sampler in ["3D", "2DArray"] {
        assert!(has(sampler, "textureGradOffset("), "{sampler}: textureGradOffset");
        assert!(has(sampler, "textureOffset("), "{sampler}: textureOffset");
        assert!(has(sampler, "textureLodOffset("), "{sampler}: textureLodOffset");
    }
    assert!(has("3D", "textureProjGrad("), "3D: textureProjGrad");
    assert!(has("3D", "textureProjGradOffset("), "3D: textureProjGradOffset");
    assert!(has("Shadow", "textureGrad("), "shadow: textureGrad");
}
