// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP), ES 2.0 context: a draw at the maximum texture units
//! stays inside the 32 sampler slots (guard for b1e22a8d).
//!
//! b1e22a8d makes `emit_textures_inner` (src/vivante/texture.rs) refuse a draw
//! (-ENOTSUP) whose vertex stage ends past slot 31; the vertex stage starts at
//! `vs_base = fs_count + fs_wide`. Only 128-bit bindings make `fs_wide` non-zero
//! and an ES 2.0 context binds none (a `cfg(test)` hook opens those classes), so
//! the highest slot is `fs_count + vs_count - 1 <= 31`. Here one GLSL ES 1.00
//! program uses every fragment and every vertex unit, 2D and cube samplers
//! mixed (as dEQP-GLES2 functional.texture.units.* and
//! functional.implementation_limits do), and the stream is decoded for slots.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gc_es2ctx_sampler_slots
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_es2ctx_sampler_slots
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

/* ---- the stand-in OSAL: host memory, and the streams the back end submits --------- */

struct Alloc {
    mem: Box<[u64]>,
    #[allow(dead_code)]
    size: usize,
}

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Alloc>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    kicks: Vec<Vec<u32>>,
    faults: Vec<String>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);
/// Serialises the tests: the OSAL pointer, the back end and `g_backend` are process-global.
static SERIAL: Mutex<()> = Mutex::new(());

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

unsafe extern "C" fn p_gpu_alloc(heap_base: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    if size == 0 || size > 1 << 30 {
        return -12;
    }
    with_platform(|p| {
        let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
        let at = p.bump.entry(heap_base).or_insert(0);
        let gpu = heap_base + 0x1000 + *at;
        *at += (size as u64).div_ceil(4096) * 4096;
        let cpu = mem.as_mut_ptr() as *mut c_void;
        // SAFETY: the caller passes a writable descriptor.
        unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
        p.allocs.insert(gpu, Alloc { mem, size });
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor.
    let m = unsafe { *mem };
    with_platform(|p| {
        if !m.impl_.is_null() && p.allocs.remove(&m.gpu).is_none() {
            p.faults.push(format!("gpu_free of 0x{:x}, which is no live allocation", m.gpu));
        }
    });
    // SAFETY: as above; a freed descriptor is cleared (the OSAL contract).
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<GcKick>() {
        with_platform(|p| p.faults.push(format!("submit of a {len}-byte descriptor")));
        return -22;
    }
    // SAFETY: the back end passes a live GcKick of that size.
    let k = unsafe { *(kick as *const GcKick) };
    with_platform(|p| {
        if k.magic != GC_KICK_MAGIC {
            p.faults.push(format!("submit with magic 0x{:x}", k.magic));
            return -22;
        }
        let Some(a) = p.allocs.get(&k.stream_gpu) else {
            p.faults.push(format!("the kick's stream 0x{:x} is no allocation", k.stream_gpu));
            return -22;
        };
        // SAFETY: the allocation holds at least `stream_bytes` bytes.
        let words: &[u32] =
            unsafe { core::slice::from_raw_parts(a.mem.as_ptr() as *const u32, k.stream_bytes as usize / 4) };
        p.kicks.push(words.to_vec());
        p.fence += 1;
        p.fence
    })
}

unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}

unsafe extern "C" fn p_completed() -> i64 {
    with_platform(|p| p.fence)
}

unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    use yakogl::osal::{OSAL_QUERY_DRAW_STATS, OSAL_QUERY_GC_FRAME_OPEN};
    if (what == OSAL_QUERY_GC_FRAME_OPEN && out_sz == 4 && !out.is_null()) || what == OSAL_QUERY_DRAW_STATS {
        return 0;
    }
    -22
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(p_gpu_alloc),
    gpu_free: Some(p_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(p_cache),
    cache_invd: Some(p_cache),
    submit: Some(p_submit),
    wait: Some(p_wait),
    completed: Some(p_completed),
    reg_read: None,
    reg_write: None,
    query: Some(p_query),
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/* ---- decoding a submitted stream ------------------------------------------------------ */

/// Every `(register, value)` a LOAD_STATE of the stream writes, in order.
fn writes(words: &[u32]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        if w & 0xf800_0000 == GC_OP_LOAD_STATE {
            let count = ((w >> 16) & 0x3ff) as usize;
            let base = (w & 0xffff) * 4;
            for k in 0..count {
                if let Some(v) = words.get(i + 1 + k) {
                    out.push((base + 4 * k as u32, *v));
                }
            }
            i += 1 + count;
            continue;
        }
        i += match w & 0xf800_0000 {
            GC_OP_DRAW_INSTANCED => 4,
            GC_OP_STALL => 2,
            _ => 1,
        };
    }
    out
}

/* ---- a fresh context, back end and OSAL per case -------------------------------------- */

/// Run `case` with the Vivante back end up on the stand-in OSAL and a fresh
/// 64x64 context (so the default texture objects are pristine, as at the
/// start of a CTS case group).
fn on_gc7000(case: impl FnOnce()) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl::vivante::backend::yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(yakogl::vivante::backend::gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        yakogl::vivante::backend::gc_shutdown();
    }
    let faults = with_platform(|p| {
        /* The memory is leaked, not freed, and the addresses keep counting up:
         * a static of the library that still names an earlier case's block
         * (a cache keyed by GPU address) must find memory, not a hole or a
         * different case's data. tests/es2_npot.rs leaks its blocks the same way. */
        for (_, a) in core::mem::take(&mut p.allocs) {
            core::mem::forget(a.mem);
        }
        core::mem::take(&mut p.faults)
    });
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
    assert!(faults.is_empty(), "the back end broke the OSAL contract: {faults:?}");
}

/* ---- GL plumbing (the CTS's calls, as in tests/es2_npot.rs) ---------------------------- */

fn no_error(what: &str) {
    // SAFETY: a plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: 0x{e:x}, the CTS expects GL_NO_ERROR (expectError)");
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
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

fn program(vs: &str, fs: &str) -> GLuint {
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        p
    }
}

/* ---- the draw at the maximum units ---------------------------------------------------- */

fn geti(pname: GLenum) -> u32 {
    let mut v: GLint = -1;
    // SAFETY: a plain GL query into a local.
    unsafe { glGetIntegerv(pname, &mut v) };
    no_error("glGetIntegerv");
    u32::try_from(v).unwrap_or_else(|_| panic!("pname 0x{pname:x} answered {v}"))
}

/// Unit `u` holds a cube map when odd, a 2D texture when even: both stages
/// sample a unit with the same type (an ES 2.0 draw with two types on one unit
/// is INVALID_OPERATION).
fn is_cube(u: u32) -> bool {
    u % 2 == 1
}

fn tex_param(target: GLenum, pname: GLenum, v: GLenum) {
    // SAFETY: a plain GL call.
    unsafe { glTexParameteri(target, pname, v as GLint) };
}

const FACES: [GLenum; 6] = [
    GL_TEXTURE_CUBE_MAP_POSITIVE_X,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_X,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Y,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Y,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Z,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Z,
];

/// A complete 4x4 texture (NEAREST, no mips) of the unit's type on unit `u`.
fn bind_unit(u: u32) {
    let texels = [0x80u8; 4 * 4 * 4];
    // SAFETY: plain GL calls; `texels` holds a 4x4 RGBA image.
    unsafe {
        glActiveTexture(GL_TEXTURE0 + u);
        let mut t: GLuint = 0;
        glGenTextures(1, &mut t);
        let target = if is_cube(u) { GL_TEXTURE_CUBE_MAP } else { GL_TEXTURE_2D };
        glBindTexture(target, t);
        tex_param(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
        tex_param(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
        let faces: &[GLenum] = if is_cube(u) { &FACES } else { &[GL_TEXTURE_2D] };
        for &face in faces {
            glTexImage2D(face, 0, GL_RGBA as GLint, 4, 4, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels.as_ptr().cast());
        }
    }
    no_error("bind_unit");
}

/// The GLSL ES 1.00 sources: `n_fs` fragment samplers `f0..`, `n_vs` vertex
/// samplers `v0..`, unit `i` of either stage a cube sampler when `is_cube(i)`.
fn sources(n_fs: u32, n_vs: u32) -> (String, String) {
    let decl = |prefix: &str, n: u32| -> String {
        (0..n)
            .map(|i| format!("uniform {} {prefix}{i};\n", if is_cube(i) { "samplerCube" } else { "sampler2D" }))
            .collect()
    };
    let vs_sum: String = (0..n_vs)
        .map(|i| {
            if is_cube(i) {
                format!("  c += textureCubeLod(v{i}, vec3(0.5, 0.5, 1.0), 0.0);\n")
            } else {
                format!("  c += texture2DLod(v{i}, vec2(0.5), 0.0);\n")
            }
        })
        .collect();
    let fs_sum: String = (0..n_fs)
        .map(|i| {
            if is_cube(i) {
                format!("  s += textureCube(f{i}, vec3(0.5, 0.5, 1.0));\n")
            } else {
                format!("  s += texture2D(f{i}, vec2(0.5));\n")
            }
        })
        .collect();
    let vs = format!(
        "attribute vec4 p;\nvarying vec4 c;\n{}void main() {{\n  c = vec4(0.0);\n{vs_sum}  c *= 0.0625;\n  gl_Position = p;\n}}\n",
        decl("v", n_vs)
    );
    let fs = format!(
        "precision mediump float;\nvarying vec4 c;\n{}void main() {{\n  vec4 s = c;\n{fs_sum}  gl_FragColor = s * 0.0625;\n}}\n",
        decl("f", n_fs)
    );
    (vs, fs)
}

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// What one draw's stream programmed.
struct Slots {
    /// Every sampler slot (of 0..64) whose descriptor address was written.
    used: Vec<u32>,
    vs_base: Option<u32>,
}

/// One quad with `n_fs` fragment and `n_vs` vertex samplers at units 0.. of
/// each stage: GL_NO_ERROR, a kick, and the slots the stream wrote.
fn draw_at(n_fs: u32, n_vs: u32) -> Slots {
    let (vs, fs) = sources(n_fs, n_vs);
    for u in 0..n_fs.max(n_vs) {
        bind_unit(u);
    }
    let kicks0 = with_platform(|p| p.kicks.len());
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 64, 64);
        let p = program(&vs, &fs);
        glUseProgram(p);
        for (prefix, n) in [("f", n_fs), ("v", n_vs)] {
            for i in 0..n {
                let name = CString::new(format!("{prefix}{i}")).unwrap();
                let loc = glGetUniformLocation(p, name.as_ptr());
                assert!(loc >= 0, "{name:?} has no location");
                glUniform1i(loc, i as GLint);
            }
        }
        no_error("glUniform1i");
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
    }
    no_error("glDrawArrays at the maximum units: the draw is accepted, not refused");
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
    no_error("glFinish");
    let stream = with_platform(|p| {
        assert!(p.kicks.len() > kicks0, "the draw produced no kick (refused?)");
        p.kicks.last().cloned().unwrap()
    });
    let w = writes(&stream);
    let used = (0..64).filter(|&s| w.iter().any(|(r, _)| *r == nte_descriptor_addr(s))).collect();
    let vs_base = w.iter().rev().find(|(r, _)| *r == GC_VS_SAMPLER_BASE).map(|(_, v)| *v);
    Slots { used, vs_base }
}

#[test]
fn es2_maximum_units_draw_stays_within_slot_31() {
    on_gc7000(|| {
        let n_fs = geti(GL_MAX_TEXTURE_IMAGE_UNITS);
        let n_vs = geti(GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS);
        let n_comb = geti(GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS);
        eprintln!("MAX_TEXTURE_IMAGE_UNITS {n_fs}, MAX_VERTEX_TEXTURE_IMAGE_UNITS {n_vs}, MAX_COMBINED {n_comb}");
        /* the ES 2.0 minimums (Table 6.20): 8, 0, 8 */
        assert!(n_fs >= 8 && n_comb >= 8);
        assert!(n_comb >= n_fs.max(n_vs), "every unit either stage can name exists");

        let s = draw_at(n_fs, n_vs);
        let highest = *s.used.last().expect("no sampler slot written");
        eprintln!("slots used {:?}, VS_SAMPLER_BASE {:?}, highest slot {highest}", s.used, s.vs_base);
        assert!(highest <= 31, "highest sampler slot {highest} is past the 32-entry array");
        /* no 128-bit binding in an ES 2.0 context: the vertex stage starts
         * right after the fragment units and the slots are exactly 0..n_fs+n_vs */
        assert_eq!(s.vs_base, Some(n_fs));
        assert_eq!(s.used, (0..n_fs + n_vs).collect::<Vec<_>>());
    });
}

/// The fragment stage alone, every unit (functional.texture.units.*'s
/// shape), and the vertex stage alone when it has units.
#[test]
fn es2_one_stage_at_the_maximum_is_accepted() {
    on_gc7000(|| {
        let n_fs = geti(GL_MAX_TEXTURE_IMAGE_UNITS);
        let n_vs = geti(GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS);
        let s = draw_at(n_fs, 0);
        assert_eq!(s.used, (0..n_fs).collect::<Vec<_>>());
        if n_vs > 0 {
            let s = draw_at(1, n_vs);
            assert_eq!(s.vs_base, Some(1));
            assert_eq!(*s.used.last().unwrap(), n_vs, "1 fragment slot, then {n_vs} vertex slots");
        }
    });
}
