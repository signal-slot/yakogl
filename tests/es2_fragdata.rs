// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! dEQP-GLES2.functional.shaders.fragdata.*, replayed on the host.
//!
//! The cases (VK-GL-CTS modules/gles2/functional/es2fShaderFragDataTests.cpp
//! and data/gles2/shaders/fragdata.test) compile one vertex and one fragment
//! shader and either draw (`valid_*`) or expect a compile failure
//! (`invalid_assign_to_1`, `write_fragcolor_and_fragdata_*`).
//! tools/cts/log/investigations/es2-fragdata.md has the whole path.
//!
//! Two layers:
//!
//! - The top of this file compiles the case shaders with the front end
//!   (`compile_to_ir`). It runs in any build with `shaderc-online`.
//! - `mod gl_api` replays each case through the public GL entry points on a
//!   host context (a do-nothing OSAL and back end), as the board images run
//!   it: `report-es2` and `shaderc-online`. This is where the
//!   'exactly one draw buffer' decision of `invalid_assign_to_1` is pinned:
//!   the shader library asks glGetIntegerv(GL_MAJOR_VERSION) and
//!   glGetIntegerv(GL_MAX_DRAW_BUFFERS) before it compiles anything
//!   (glsShaderLibraryCase.cpp:1041-1052). The back end draws nothing, so the
//!   pixel check of the three `valid_*` cases (all green) stays a board
//!   result; the host sees the compile and link status, no GL error through
//!   the draw, and the draw reaching the back end.
//!
//! ```text
//! cargo test --no-default-features \
//!     --features host-test,gpu-powervr,shaderc-online,report-es2 --test es2_fragdata
//! cargo test --no-default-features \
//!     --features host-test,gpu-vivante,shaderc-online,report-es2 --test es2_fragdata
//! cargo test --no-default-features \
//!     --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test es2_fragdata
//! ```
#![cfg(feature = "shaderc-online")]

use yakogl::shaderc::online::lower::compile_to_ir;
use yakogl::shaderc::online::{arena::Arena, ir, sema::Stage};

/// The vertex shader of the three valid_* cases (es2fShaderFragDataTests.cpp:125).
const VALID_VS: &str =
    "attribute highp vec4 a_position;\nattribute highp float a_index;\nattribute highp vec4 a_color;\n\
varying mediump float v_index;\nvarying mediump vec4 v_color;\n\
void main (void)\n{\n    gl_Position = a_position;\n    v_color = a_color;\n    v_index = a_index;\n}\n";

/// The vertex shader of every negative case (fragdata.test).
const NEG_VS: &str = "void main (void)\n{\n    gl_Position = vec4(1.0);\n}\n";

/// The fragment shader of `invalid_assign_to_1` (fragdata.test:2-19).
const ASSIGN_TO_1_FS: &str = "void main (void)\n{\n    gl_FragData[1] = vec4(1.0);\n}\n";

/// The fragment shaders of `write_fragcolor_and_fragdata_*` (fragdata.test:21-78).
const BOTH_OUTPUTS_FS: [(&str, &str); 3] = [
    (
        "write_fragcolor_and_fragdata_simple",
        "void main (void)\n{\n    gl_FragColor = vec4(1.0);\n    gl_FragData[0] = vec4(1.0);\n}\n",
    ),
    (
        "write_fragcolor_and_fragdata_static_if",
        "void main (void)\n{\n    if (false)\n        gl_FragColor = vec4(1.0);\n    else\n        gl_FragData[0] = vec4(1.0);\n}\n",
    ),
    (
        "write_fragcolor_and_fragdata_unused_func",
        "void unused (void)\n{\n    gl_FragData[0] = vec4(1.0);\n}\nvoid main (void)\n{\n    gl_FragColor = vec4(1.0);\n}\n",
    ),
];

/// genSources' fragment shader (es2fShaderFragDataTests.cpp:137).
fn valid_fs(index: &str) -> String {
    format!(
        "varying mediump vec4 v_color;\nvarying mediump float v_index;\nuniform mediump int u_index;\n\
         void main (void)\n{{\n    gl_FragData[{index}] = v_color;\n}}\n"
    )
}

/// Compile; Ok is the number of stores to colour output 0, Err the message
/// glGetShaderInfoLog would carry.
fn compile(stage: Stage, src: &str) -> Result<usize, String> {
    let mut buf = vec![0u8; 1 << 20];
    let a = Arena::new(&mut buf);
    match compile_to_ir(&a, stage, src) {
        Ok(l) => Ok(l
            .module
            .instrs
            .iter()
            .filter(|o| matches!(o, ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(0), .. }))
            .count()),
        Err(e) => Err(e.msg.to_string()),
    }
}

fn ok(stage: Stage, src: &str) -> usize {
    compile(stage, src).unwrap_or_else(|m| panic!("expected success for {src:?}, got: {m}"))
}

fn refused(stage: Stage, src: &str) -> String {
    match compile(stage, src) {
        Ok(_) => panic!("expected a compile failure for {src:?}"),
        Err(m) => m,
    }
}

#[test]
fn valid_static_index() {
    ok(Stage::Vertex, VALID_VS);
    assert_eq!(ok(Stage::Fragment, &valid_fs("0")), 1);
}

#[test]
fn valid_uniform_index() {
    ok(Stage::Vertex, VALID_VS);
    assert_eq!(ok(Stage::Fragment, &valid_fs("u_index")), 1);
}

#[test]
fn valid_dynamic_index() {
    ok(Stage::Vertex, VALID_VS);
    assert_eq!(ok(Stage::Fragment, &valid_fs("int(v_index)")), 1);
}

/// fragdata.test:2. The case is NotSupported on a context whose
/// GL_MAX_DRAW_BUFFERS query answers more than 1; with the query refused
/// (report-es2) it runs, and the compile must fail.
#[test]
fn invalid_assign_to_1() {
    ok(Stage::Vertex, NEG_VS);
    let m = refused(Stage::Fragment, ASSIGN_TO_1_FS);
    assert!(m.contains("one draw buffer"), "{m}");
}

/// The shader-side half of the 'exactly one draw buffer' premise: in an ES 2.0
/// report the highest index gl_MaxDrawBuffers allows is 0.
#[cfg(feature = "report-es2")]
#[test]
fn gl_max_draw_buffers_is_one_in_the_es2_report() {
    let src = "void main (void)\n{\n    gl_FragData[gl_MaxDrawBuffers - 1] = vec4(1.0);\n}\n";
    assert_eq!(ok(Stage::Fragment, src), 1);
    let src = "void main (void)\n{\n    gl_FragData[gl_MaxDrawBuffers] = vec4(1.0);\n}\n";
    refused(Stage::Fragment, src);
}

/// fragdata.test:21-78 -- ES 1.00 section 7.2: a shader may not write both,
/// also when the writes are in the two arms of a static `if` or one is in a
/// function main never calls.
#[test]
fn write_fragcolor_and_fragdata() {
    for (name, fs) in BOTH_OUTPUTS_FS {
        let m = compile(Stage::Fragment, fs);
        assert!(m.is_err(), "{name}: compiled");
    }
}

/// The same cases through the public GL API on a host context, in the
/// configuration of the dEQP-GLES2 board images.
#[cfg(all(feature = "host-test", feature = "report-es2"))]
mod gl_api {
    use super::{valid_fs, ASSIGN_TO_1_FS, BOTH_OUTPUTS_FS, NEG_VS, VALID_VS};
    use core::ffi::{c_char, c_void};
    use std::ffi::{CStr, CString};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Mutex, MutexGuard, Once};
    use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
    use yakogl::gl::buffer::*;
    use yakogl::gl::consts::*;
    use yakogl::gl::draw::*;
    use yakogl::gl::framebuffer::*;
    use yakogl::gl::program::*;
    use yakogl::gl::shader::*;
    use yakogl::gl::state::*;
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

    /// Draws that reached the back end, and the index count of the last one.
    static DRAWS: AtomicU32 = AtomicU32::new(0);
    static LAST_INDEX_COUNT: AtomicU32 = AtomicU32::new(0);

    unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
        DRAWS.fetch_add(1, Ordering::SeqCst);
        LAST_INDEX_COUNT.store((*d).index_count, Ordering::SeqCst);
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
        lock: None,
        unlock: None,
        thread_id: None,
    };

    static BACKEND: YakoglBackend = YakoglBackend {
        name: c"es2-fragdata".as_ptr() as *const c_char,
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
    };

    /// The render target dEQP gets on the boards is larger; the cases take
    /// min(target, 128) for the viewport, which is the whole of this one.
    const TARGET: GLint = 8;

    /// One context for the whole file; the tests take turns on it.
    static LOCK: Mutex<()> = Mutex::new(());
    static ONCE: Once = Once::new();

    /// The shared setup: the context, current, with no error pending.
    fn context() -> MutexGuard<'static, ()> {
        let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        ONCE.call_once(|| {
            // SAFETY: the tables are statics; the lock above serialises the tests.
            unsafe {
                osal_set_ops(&OSAL);
                yakogl::backend::g_backend = &BACKEND;
                yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
                assert!(!yakogl::gl::context::gl_context_create(TARGET as u32, TARGET as u32).is_null());
            }
        });
        // SAFETY: a plain GL call on the one context.
        unsafe { while glGetError() != GL_NO_ERROR {} }
        g
    }

    /* ---- helpers --------------------------------------------------------------- */

    fn err() -> GLenum {
        // SAFETY: a plain GL call.
        unsafe { glGetError() }
    }

    /// What a glu::ShaderProgram reports: each stage's GL_COMPILE_STATUS with
    /// its info log, and GL_LINK_STATUS. Linked even when a stage failed, as
    /// glu does; the program is left for the caller to use and delete.
    struct Program {
        id: GLuint,
        vs: (bool, String),
        fs: (bool, String),
        linked: bool,
    }

    fn shader(kind: GLenum, src: &str) -> (GLuint, bool, String) {
        // SAFETY: the source is live for the calls; the log buffer is as long as it is told.
        unsafe {
            let s = glCreateShader(kind);
            let p = [src.as_ptr().cast::<c_char>()];
            let l = [src.len() as GLint];
            glShaderSource(s, 1, p.as_ptr(), l.as_ptr());
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            let mut log = [0u8; 512];
            let mut n: GLsizei = 0;
            glGetShaderInfoLog(s, log.len() as GLsizei, &mut n, log.as_mut_ptr().cast());
            (s, ok != 0, String::from_utf8_lossy(&log[..n.max(0) as usize]).into_owned())
        }
    }

    fn program(vs_src: &str, fs_src: &str) -> Program {
        let (vs, vs_ok, vs_log) = shader(GL_VERTEX_SHADER, vs_src);
        let (fs, fs_ok, fs_log) = shader(GL_FRAGMENT_SHADER, fs_src);
        // SAFETY: plain GL calls on names made above.
        unsafe {
            let id = glCreateProgram();
            glAttachShader(id, vs);
            glAttachShader(id, fs);
            glLinkProgram(id);
            let mut linked: GLint = 0;
            glGetProgramiv(id, GL_LINK_STATUS, &mut linked);
            glDeleteShader(vs);
            glDeleteShader(fs);
            assert_eq!(err(), GL_NO_ERROR, "building the program raised an error");
            Program { id, vs: (vs_ok, vs_log), fs: (fs_ok, fs_log), linked: linked != 0 }
        }
    }

    /// The verdict glsShaderLibraryCase.cpp gives an `expect compile_fail`
    /// case: Ok when a stage failed to compile, the fail reason otherwise.
    fn expect_compile_fail(p: &Program) -> Result<(), &'static str> {
        let all_compiles_ok = p.vs.0 && p.fs.0;
        if all_compiles_ok && !p.linked {
            Err("expected compilation to fail, but shaders compiled and link failed.")
        } else if all_compiles_ok {
            Err("expected compilation to fail, but shaders compiled correctly.")
        } else {
            Ok(())
        }
    }

    fn get_string(name: GLenum) -> String {
        // SAFETY: glGetString returns a NUL-terminated static string.
        let s = unsafe { CStr::from_ptr(glGetString(name).cast::<c_char>()) }.to_string_lossy().into_owned();
        assert_eq!(err(), GL_NO_ERROR, "glGetString(0x{name:x})");
        s
    }

    /* ---- the shader library's two requirements (invalid_assign_to_1) ------------ */

    /// glsShaderLibraryCase.cpp:1040-1042: `require only_glsl_es_100_support`
    /// skips the case, as a pass, when `glu::IsES3Compatible` (gluContextInfo.cpp:
    /// 64-73: GL_MAJOR_VERSION raises no error). The ES 2.0 report refuses
    /// GL_MAJOR_VERSION, so the case runs. :1044-1052: `require
    /// exactly_one_draw_buffer` is NotSupported only when GL_MAX_DRAW_BUFFERS
    /// raises no error AND answers more than 1; the ES 2.0 report refuses it,
    /// so the case goes on to compile. Neither draw-buffer extension (which
    /// would bring the query back) is claimed.
    #[test]
    fn invalid_assign_to_1_requirements_let_the_case_run() {
        let _g = context();
        let mut v: GLint = 0;
        // SAFETY: room for one value each (both are refused and write nothing).
        unsafe {
            glGetIntegerv(GL_MAJOR_VERSION, &mut v);
            assert_ne!(err(), GL_NO_ERROR, "IsES3Compatible: true, so the case would be skipped as a pass");
            let mut max_draw_buffers: GLint = 0;
            glGetIntegerv(GL_MAX_DRAW_BUFFERS, &mut max_draw_buffers);
            let e = err();
            assert!(
                !(e == GL_NO_ERROR && max_draw_buffers > 1),
                "NotSupported: GL_MAX_DRAW_BUFFERS = {max_draw_buffers} with no error"
            );
            assert_eq!(e, GL_INVALID_ENUM, "GL_MAX_DRAW_BUFFERS in an ES 2.0 context");
        }
        let ext = get_string(GL_EXTENSIONS);
        for name in ["GL_EXT_draw_buffers", "GL_NV_draw_buffers"] {
            assert!(!ext.split(' ').any(|e| e == name), "{name} claimed: GL_MAX_DRAW_BUFFERS would have to answer");
        }
    }

    /* ---- the negative cases of data/gles2/shaders/fragdata.test ------------------ */

    /// fragdata.test:2-19, `invalid_assign_to_1`: GLSL ES 1.00 7.2 and 7.4,
    /// gl_FragData has gl_MaxDrawBuffers (1) elements, and the constant index
    /// 1 is out of range: the fragment shader must not compile.
    #[test]
    fn invalid_assign_to_1() {
        let _g = context();
        let p = program(NEG_VS, ASSIGN_TO_1_FS);
        assert!(p.vs.0, "the vertex shader is valid: {:?}", p.vs.1);
        assert!(!p.fs.0, "gl_FragData[1] compiled");
        assert!(!p.fs.1.is_empty(), "refused with no info log");
        assert_eq!(expect_compile_fail(&p), Ok(()));
        // SAFETY: a plain GL call.
        unsafe { glDeleteProgram(p.id) };
    }

    /// fragdata.test:21-78: GLSL ES 1.00 7.2, "a shader may assign values to
    /// either gl_FragColor or gl_FragData, but not both", where any
    /// assignment in the text counts, taken or not, called or not.
    #[test]
    fn write_fragcolor_and_fragdata() {
        let _g = context();
        for (name, fs) in BOTH_OUTPUTS_FS {
            let p = program(NEG_VS, fs);
            assert!(p.vs.0, "{name}: the vertex shader is valid: {:?}", p.vs.1);
            assert!(!p.fs.0, "{name}: compiled");
            assert_eq!(expect_compile_fail(&p), Ok(()), "{name}");
            // SAFETY: a plain GL call.
            unsafe { glDeleteProgram(p.id) };
        }
    }

    /* ---- the three indexing cases of es2fShaderFragDataTests.cpp ----------------- */

    fn location(f: unsafe extern "C" fn(GLuint, *const GLchar) -> GLint, p: GLuint, name: &str) -> GLint {
        let c = CString::new(name).unwrap();
        // SAFETY: a NUL-terminated name, live for the call.
        unsafe { f(p, c.as_ptr()) }
    }

    /// FragDataIndexingCase::iterate (es2fShaderFragDataTests.cpp:150-205)
    /// up to the read-back: build the program (a failure is Fail for the
    /// static index, NotSupported otherwise, :170-176), clear, viewport,
    /// u_index = 0, draw the two-triangle quad from client arrays as glu::draw
    /// does (an attribute the program does not use has location -1 and is
    /// skipped), expect no error (:187), read the pixels (:195-196).
    fn indexing_case(index: &str) {
        let _g = context();
        let p = program(VALID_VS, &valid_fs(index));
        assert!(p.vs.0, "vertex shader: {:?}", p.vs.1);
        assert!(p.fs.0, "gl_FragData[{index}]: {:?}", p.fs.1);
        assert!(p.linked, "gl_FragData[{index}]: link failed");

        let positions: [f32; 8] = [-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0];
        let colors: [f32; 16] = [0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
        let index_values: [f32; 4] = [0.0; 4];
        let indices: [u8; 6] = [0, 1, 2, 2, 1, 3];
        let arrays: [(&str, GLint, *const f32); 3] = [
            ("a_position", 2, positions.as_ptr()),
            ("a_color", 4, colors.as_ptr()),
            ("a_index", 1, index_values.as_ptr()),
        ];

        let draws = DRAWS.load(Ordering::SeqCst);
        // SAFETY: plain GL calls; the client arrays outlive the draw, which reads them.
        unsafe {
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
            glClearColor(1.0, 0.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
            glViewport(0, 0, TARGET.min(128), TARGET.min(128));
            glUseProgram(p.id);
            glUniform1i(location(glGetUniformLocation, p.id, "u_index"), 0);
            assert_eq!(err(), GL_NO_ERROR, "setting u_index");
            let mut enabled = Vec::new();
            for (name, size, data) in arrays {
                let loc = location(glGetAttribLocation, p.id, name);
                if loc < 0 {
                    continue;
                }
                glEnableVertexAttribArray(loc as GLuint);
                glVertexAttribPointer(loc as GLuint, size, GL_FLOAT, 0, 0, data.cast());
                enabled.push(loc as GLuint);
            }
            assert!(!enabled.is_empty(), "a_position has no location");
            glDrawElements(GL_TRIANGLES, indices.len() as GLsizei, GL_UNSIGNED_BYTE, indices.as_ptr().cast());
            assert_eq!(err(), GL_NO_ERROR, "Rendering failed");
            for loc in enabled {
                glDisableVertexAttribArray(loc);
            }
            let mut pixels = vec![0u8; (TARGET * TARGET * 4) as usize];
            glReadPixels(0, 0, TARGET, TARGET, GL_RGBA, GL_UNSIGNED_BYTE, pixels.as_mut_ptr().cast());
            assert_eq!(err(), GL_NO_ERROR, "Reading pixels failed");
            glUseProgram(0);
            glDeleteProgram(p.id);
        }
        assert!(DRAWS.load(Ordering::SeqCst) > draws, "gl_FragData[{index}]: the draw did not reach the back end");
        assert_eq!(LAST_INDEX_COUNT.load(Ordering::SeqCst), 6, "the quad's six indices");
    }

    /// `valid_static_index`: `gl_FragData[0]`. A compile failure is a Fail.
    #[test]
    fn valid_static_index() {
        indexing_case("0");
    }

    /// `valid_uniform_index`: `gl_FragData[u_index]`, u_index set to 0. With
    /// one draw buffer every index but 0 is out of range (undefined, GLSL ES
    /// 1.00 4.1.9), and the compiler writes the one colour output.
    #[test]
    fn valid_uniform_index() {
        indexing_case("u_index");
    }

    /// `valid_dynamic_index`: `gl_FragData[int(v_index)]`, v_index 0 at every
    /// vertex.
    #[test]
    fn valid_dynamic_index() {
        indexing_case("int(v_index)");
    }
}
