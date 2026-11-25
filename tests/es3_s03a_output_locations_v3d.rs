// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The link refusal behind 54 dEQP-GLES3 S03a builtin-precision Fails on the
//! Pi 4 (V3D), pinned on the host (plan #399, issue #1261, investigation
//! s03a-link-cause).
//!
//! The dEQP shader executor writes a mat4 result as four `vec4` fragment
//! outputs (one per column, GLSL ES 3.00 has no matrix output), at locations
//! 0 to 3, so it reaches location 3. V3D answers three draw targets (`DRAW_TARGETS`,
//! src/backend.rs), and `check_fragment_output_locations` (src/gl/program.rs)
//! refuses the link: "fragment output at location 3 ends at 4, beyond the 3
//! draw buffers".
//!
//! - `mat4_output_refused_with_three_draw_targets` pins today's refusal.
//! - `mat4_output_links_with_four_draw_targets` is `#[ignore]`d: it holds once
//!   V3D has four draw targets (the mrt4 work, docs/notes/mrt4-v3d-pvr.md,
//!   issue #1261). `backend::HOST_DRAW_TARGETS` (the stand-in of
//!   testutil::backend_draws_targets) shows it on the host today. Run it with
//!   `-- --ignored`; drop the `#[ignore]` when V3D answers four.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_s03a_output_locations_v3d
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::{CStr, CString};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

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
    name: c"es3-s03a-output-locations-v3d".as_ptr() as *const c_char,
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

/// One context for the whole file; the tests take turns on it. `targets` is
/// the draw-target count the back end answers (`HOST_DRAW_TARGETS`; 0 leaves
/// the build's own, V3D's three).
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context(targets: u8) -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    backend::HOST_DRAW_TARGETS.store(targets, Ordering::Relaxed);
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    g
}

/// The fragment shader the dEQP shader executor builds for
/// `builtin_precision_matrix.matrixcompmult.highp_fragment.mat4`. GLSL ES 3.00
/// allows no matrix fragment output, so the executor writes the mat4 result as
/// four `vec4` outputs, one per column, at locations 0..=3.
const FRAGMENT: &str = "#version 300 es\n\
    precision highp float;\n\
    precision highp int;\n\
    in highp mat4 in0;\n\
    in highp mat4 in1;\n\
    layout(location = 0) out highp vec4 out0;\n\
    layout(location = 1) out highp vec4 out1;\n\
    layout(location = 2) out highp vec4 out2;\n\
    layout(location = 3) out highp vec4 out3;\n\
    void main() {\n\
        mat4 r = matrixCompMult(in0, in1);\n\
        out0 = r[0]; out1 = r[1]; out2 = r[2]; out3 = r[3];\n\
    }\n";

const VERTEX: &str = "#version 300 es\n\
    in highp vec4 a_position;\n\
    in highp mat4 a_in0;\n\
    in highp mat4 a_in1;\n\
    out highp mat4 in0;\n\
    out highp mat4 in1;\n\
    void main() { in0 = a_in0; in1 = a_in1; gl_Position = a_position; }\n";

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
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

/// Link the executor's program: (link status, info log).
fn link() -> (GLint, String) {
    let v = shader(GL_VERTEX_SHADER, VERTEX);
    let f = shader(GL_FRAGMENT_SHADER, FRAGMENT);
    // SAFETY: plain GL calls; the log buffer is NUL-terminated by the GL.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        let mut buf = [0 as c_char; 512];
        glGetProgramInfoLog(p, buf.len() as GLsizei, core::ptr::null_mut(), buf.as_mut_ptr());
        (ok, CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
    }
}

#[test]
fn mat4_output_refused_with_three_draw_targets() {
    let _g = context(0);
    let (ok, log) = link();
    assert_eq!(ok, 0, "V3D answers 3 draw targets; a mat4 output must not link: {log:?}");
    assert!(log.contains("fragment output"), "{log:?}");
    assert!(log.contains("location 3"), "the fourth column is at location 3: {log:?}");
    assert!(log.contains("ends at 4"), "{log:?}");
    assert!(log.contains("3 draw buffers"), "{log:?}");
}

/// #1261: the 54 S03a mat4 link Fails. Holds once V3D has four draw targets
/// (docs/notes/mrt4-v3d-pvr.md); the host stand-in shows it today.
#[test]
#[ignore = "V3D answers 3 draw targets until the mrt4 work (#1261, docs/notes/mrt4-v3d-pvr.md)"]
fn mat4_output_links_with_four_draw_targets() {
    let _g = context(4);
    let (ok, log) = link();
    assert_eq!(ok, 1, "with 4 draw targets the mat4 output links: {log:?}");
}
