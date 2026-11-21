// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 2.0 context guard for 4a7b5174 (shaderc: gl_FragData[0] read back is
//! held in a slot, as gl_FragColor is), on the GC7000 (i.MX8MP). The whole dEQP-GLES2
//! tree of the i.MX8MP (0fbe61a3) predates it, and its stage-1 promise is 0 Fail and only
//! the 34 NotSupported of tools/cts/es2-accepted.txt. 4a7b5174 changes the
//! lowering of every GLSL ES 1.00 fragment shader that writes gl_FragData[0]
//! (dEQP-GLES2 shaders.fragdata.*, shaders.builtin_variable.* and several
//! shaders.* cases use it); the answers asserted here come from GLSL ES 1.00
//! section 7.2 and dEQP-GLES2 (fragdata.test, es2fShaderFragDataTests.cpp),
//! not from the current output:
//!
//! - written with a constant, written then read and modified (`*=`, a
//!   partial write), written in a branch, written in a loop, and read back
//!   at a run-time index (`valid_dynamic_index`): compile and link, and the
//!   one colour output is stored once (the front end `compile_to_ir`, the
//!   same lowering both back ends take);
//! - `gl_FragColor` and `gl_FragData` in one shader, also in the arms of a
//!   static `if` or in a function main never calls: the compile fails;
//! - a constant index 1 fails (one draw buffer in an ES 2.0 report).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gc_es2ctx_drift_fragdata
//! ```
#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature = "report-es2"))]

use core::ffi::{c_char, c_void, CStr};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::shaderc::online::lower::compile_to_ir;
use yakogl::shaderc::online::{arena::Arena, ir, sema::Stage};

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
    name: c"gc-es2ctx-drift-fragdata".as_ptr() as *const c_char,
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

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The one ES 2.0 context, the online compiler behind it; serialises the tests.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(16, 16).is_null());
        }
    });
    // SAFETY: plain GL calls on the one context.
    unsafe { while glGetError() != GL_NO_ERROR {} }
    g
}

/// The fragment shaders that must compile and link: (name, source).
const VALID: [(&str, &str); 6] = [
    (
        "constant",
        "precision mediump float;\nvarying vec4 v_color;\nvoid main (void)\n{\n    gl_FragData[0] = vec4(0.25, 0.5, 0.75, 1.0);\n}\n",
    ),
    (
        "read_modify_write",
        "precision mediump float;\nvarying vec4 v_color;\nvoid main (void)\n{\n    gl_FragData[0] = v_color;\n    gl_FragData[0].rgb *= 0.5;\n}\n",
    ),
    (
        "read_operand",
        "precision mediump float;\nvarying vec4 v_color;\nvoid main (void)\n{\n    gl_FragData[0] = vec4(1.0, 0.5, 0.25, 1.0);\n    gl_FragData[0] = gl_FragData[0] * v_color;\n}\n",
    ),
    (
        "branch",
        "precision mediump float;\nvarying vec4 v_color;\nvoid main (void)\n{\n    if (v_color.x > 0.5)\n        gl_FragData[0] = v_color;\n    else\n        gl_FragData[0] = vec4(0.0, 1.0, 0.0, 1.0);\n    gl_FragData[0].a *= 0.5;\n}\n",
    ),
    (
        "loop",
        "precision mediump float;\nvarying vec4 v_color;\nvoid main (void)\n{\n    gl_FragData[0] = vec4(0.0);\n    for (int i = 0; i < 4; i++)\n        gl_FragData[0] += v_color * 0.25;\n}\n",
    ),
    (
        "dynamic_index",
        "precision mediump float;\nvarying vec4 v_color;\nvarying float v_index;\nvoid main (void)\n{\n    gl_FragData[0] = v_color;\n    gl_FragData[int(v_index)] = gl_FragData[int(v_index)] * 0.5;\n}\n",
    ),
];

/// A shader that uses both outputs: GLSL ES 1.00 section 7.2 makes it a
/// compile error (fragdata.test write_fragcolor_and_fragdata_*).
const BOTH: [(&str, &str); 4] = [
    ("simple", "void main (void)\n{\n    gl_FragColor = vec4(1.0);\n    gl_FragData[0] = vec4(1.0);\n}\n"),
    (
        "static_if",
        "void main (void)\n{\n    if (false)\n        gl_FragColor = vec4(1.0);\n    else\n        gl_FragData[0] = vec4(1.0);\n}\n",
    ),
    (
        "unused_func",
        "void unused (void)\n{\n    gl_FragData[0] = vec4(1.0);\n}\nvoid main (void)\n{\n    gl_FragColor = vec4(1.0);\n}\n",
    ),
    (
        "read_back_beside_fragcolor",
        "precision mediump float;\nvoid main (void)\n{\n    gl_FragColor = vec4(0.5);\n    gl_FragData[0] *= 0.5;\n}\n",
    ),
];

/// Index 1 with one draw buffer (fragdata.test invalid_assign_to_1), also
/// beside a read-back of element 0.
const INDEX_1: [(&str, &str); 2] = [
    ("assign", "void main (void)\n{\n    gl_FragData[1] = vec4(1.0);\n}\n"),
    ("read_back_then_1", "void main (void)\n{\n    gl_FragData[0] *= 0.5;\n    gl_FragData[1] = vec4(1.0);\n}\n"),
];

const VS: &str = "attribute vec4 p;\nvarying vec4 v_color;\nvarying float v_index;\nvoid main (void)\n{\n    gl_Position = p;\n    v_color = p * 0.5 + 0.5;\n    v_index = 0.0;\n}\n";

/// Compile `src` as a fragment shader through the GL API; the compile status.
fn compiles(src: &str) -> bool {
    let text = CString::new(src).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(GL_FRAGMENT_SHADER);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        glDeleteShader(s);
        ok == 1
    }
}

/// Compile and link `fs` with [`VS`]; the link status, its log on a failure.
fn links(fs: &str) -> Result<(), String> {
    let (vs, fs) = (CString::new(VS).unwrap(), CString::new(fs).unwrap());
    // SAFETY: plain GL calls; NUL-terminated sources, a log buffer of the size announced.
    unsafe {
        let mk = |ty: GLenum, t: &CString| {
            let s = glCreateShader(ty);
            let p: [*const c_char; 1] = [t.as_ptr()];
            glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            (s, ok == 1)
        };
        let ((v, vok), (f, fok)) = (mk(GL_VERTEX_SHADER, &vs), mk(GL_FRAGMENT_SHADER, &fs));
        assert!(vok, "the vertex shader compiles");
        if !fok {
            return Err("the fragment shader does not compile".into());
        }
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        let mut buf = [0 as c_char; 1024];
        let mut len: GLsizei = 0;
        glGetProgramInfoLog(p, buf.len() as GLsizei, &mut len, buf.as_mut_ptr());
        let log = CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned();
        glDeleteShader(v);
        glDeleteShader(f);
        glDeleteProgram(p);
        if ok == 1 {
            Ok(())
        } else {
            Err(log)
        }
    }
}

/// The number of stores to colour output 0 the front end lowers `src` to
/// (Err: its compile message). The lowering is the one both back ends take.
fn stores(src: &str) -> Result<usize, String> {
    let mut buf = vec![0u8; 1 << 20];
    let a = Arena::new(&mut buf);
    match compile_to_ir(&a, Stage::Fragment, src) {
        Ok(l) => Ok(l
            .module
            .instrs
            .iter()
            .filter(|o| matches!(o, ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(0), .. }))
            .count()),
        Err(e) => Err(e.msg.to_string()),
    }
}

#[test]
fn fragdata_shaders_compile_and_link() {
    let _g = rig();
    for (name, fs) in VALID {
        if let Err(log) = links(fs) {
            panic!("{name} does not link: {log}");
        }
        // SAFETY: a plain GL call.
        assert_eq!(unsafe { glGetError() }, GL_NO_ERROR, "{name}");
    }
}

/// Written then read and modified, in a branch, in a loop or at a run-time
/// index: the one colour output is stored once, at the end of main.
#[test]
fn fragdata_output_is_stored_once() {
    for (name, fs) in VALID {
        assert_eq!(stores(fs), Ok(1), "{name}");
    }
}

/// Masked and swizzled partial writes after a whole write compile, link, and
/// store once.
#[test]
fn partial_writes_compile() {
    let _g = rig();
    for body in
        ["gl_FragData[0].rgb *= 0.5;", "gl_FragData[0].a = gl_FragData[0].r;", "gl_FragData[0].xy = gl_FragData[0].yx;"]
    {
        let fs = format!(
            "precision mediump float;\nvarying vec4 v_color;\nvoid main (void)\n{{\n    gl_FragData[0] = v_color;\n    {body}\n}}\n"
        );
        assert_eq!(stores(&fs), Ok(1), "{body}");
        assert_eq!(links(&fs), Ok(()), "{body}");
    }
}

#[test]
fn fragcolor_and_fragdata_do_not_compile() {
    let _g = rig();
    for (name, fs) in BOTH {
        assert!(!compiles(fs), "{name} compiled");
        assert!(stores(fs).is_err(), "{name} lowered");
    }
}

#[test]
fn fragdata_index_1_does_not_compile() {
    let _g = rig();
    for (name, fs) in INDEX_1 {
        assert!(!compiles(fs), "{name} compiled");
        let m = stores(fs).expect_err(name);
        assert!(m.contains("one draw buffer"), "{name}: {m}");
    }
}
