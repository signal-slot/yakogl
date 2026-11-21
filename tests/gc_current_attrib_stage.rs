// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): a disabled int, uint or float vertex input reads the
//! typed current value (docs/es3-cts-failures.md Appendix A, T1) through the
//! GL core's zero-stride staging (src/gl/draw.rs, `if !ga.enabled`).
//!
//! A program with `ivec4`, `uvec4` and `vec4` inputs, all arrays disabled,
//! set with glVertexAttribI4i, glVertexAttribI4ui and glVertexAttrib4f:
//!
//! * (a) the 16 bytes each stream fetches are the raw values, bit for bit
//!   (1 << 30, i32::MAX, -5, 0xffffffff, a NaN pattern 0x7fc00001), not
//!   converted to float on the way;
//! * (b) every stream has stride 0 and divisor 0, and the element is the
//!   four-component 32-bit fetch the back end uses for every 4-byte input
//!   (`attrib_data_type`: GC_DATA_TYPE_FLOAT, no normalisation: the shader's
//!   integer type is the register's bit pattern, the fetch converts nothing);
//! * an int input fed by a float-typed current value (ES 3.0 2.7: undefined)
//!   only has to draw without a GL error or a crash.
//!
//! How it runs: the real GL core and the real Vivante back end over a
//! stand-in OSAL that keeps every submitted stream (as
//! tests/gc_stream_divisor.rs).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_current_attrib_stage
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::gl::consts::*;
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::program::{
    glAttachShader, glBindAttribLocation, glCreateProgram, glGetProgramiv, glLinkProgram, glUseProgram,
};
use yakogl::gl::shader::{glCompileShader, glCreateShader, glGetShaderiv, glShaderSource};
use yakogl::gl::state::{glFinish, glGetError, glViewport};
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glVertexAttrib4f, glVertexAttribI4i, glVertexAttribI4ui};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{gc_init, gc_shutdown, yakogl_backend_vivante};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

/* ---- the stand-in OSAL: host memory, and the streams the back end submits --------- */

struct Alloc {
    mem: Box<[u64]>,
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
/// Serialises the tests: the OSAL pointer, the back end, the state cache and
/// `g_backend` are process-global.
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
        p.allocs.insert(gpu, Alloc { mem });
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor.
    let m = unsafe { *mem };
    with_platform(|p| {
        /* leaked, not freed: a static of the library that still names the
         * block must find memory (tests/gc_npot_repeat.rs) */
        match p.allocs.remove(&m.gpu) {
            Some(a) => core::mem::forget(a.mem),
            None if !m.impl_.is_null() => {
                p.faults.push(format!("gpu_free of 0x{:x}, which is no live allocation", m.gpu))
            }
            None => {}
        }
    });
    // SAFETY: as above; a freed descriptor is cleared (the OSAL contract).
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

/// The stream is kept and its fence raised; every kick completes at once.
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
        let words = words.to_vec();
        p.kicks.push(words);
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

/* ---- GL plumbing ------------------------------------------------------------------- */

/// Run `case` on the real Vivante back end over the stand-in OSAL and a
/// fresh 64x64 context. The stand-in starts empty and hands out the same
/// addresses as at the last start (the memory of earlier cases is leaked,
/// not reused), so two runs of one GL sequence get the same streams.
fn with_backend<R>(case: impl FnOnce() -> R) -> R {
    with_platform(|p| {
        for (_, a) in core::mem::take(&mut p.allocs) {
            core::mem::forget(a.mem);
        }
        *p = Platform::default();
    });
    // SAFETY: the caller holds SERIAL; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        gc_shutdown();
    }
    let faults = with_platform(|p| core::mem::take(&mut p.faults));
    let r = r.unwrap_or_else(|e| std::panic::resume_unwind(e));
    assert!(faults.is_empty(), "the back end broke the OSAL contract: {faults:?}");
    r
}

fn no_error(what: &str) {
    // SAFETY: a plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: 0x{e:x}");
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

/// A linked program of `vs` and `fs`, attribute `p` at 0, `q` at 1 and `r` at 2.
fn program(vs: &str, fs: &str) -> GLuint {
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glBindAttribLocation(p, 1, c"q".as_ptr());
        glBindAttribLocation(p, 2, c"r".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        p
    }
}

/// The streams submitted since the last call.
fn take_kicks() -> Vec<Vec<u32>> {
    with_platform(|p| core::mem::take(&mut p.kicks))
}

/* ---- the cases --------------------------------------------------------------------- */

const VS: &str = "#version 300 es\nin highp ivec4 p;\nin highp uvec4 q;\nin highp vec4 r;\n\
                  flat out highp ivec4 vi;\nflat out highp uvec4 vu;\nout highp vec4 vf;\n\
                  void main() { vi = p; vu = q; vf = r; gl_Position = vec4(0.0, 0.0, 0.5, 1.0); }\n";
const FS: &str = "#version 300 es\nprecision highp float;\nprecision highp int;\n\
                  flat in ivec4 vi;\nflat in uvec4 vu;\nin vec4 vf;\nout vec4 o;\n\
                  void main() { o = vf + vec4(vi) + vec4(vu); }\n";

/// Every register written in the streams since the last call, last write
/// winning, and the same registers' values.
fn final_state(kicks: &[Vec<u32>]) -> BTreeMap<u32, u32> {
    let mut regs = BTreeMap::new();
    for words in kicks {
        let mut i = 0usize;
        while i < words.len() {
            let w = words[i];
            match w & 0xf800_0000 {
                GC_OP_LOAD_STATE => {
                    let count = ((w >> 16) & 0x3ff) as usize;
                    let base = (w & 0xffff) * 4;
                    for k in 0..count {
                        regs.insert(base + 4 * k as u32, words[i + 1 + k]);
                    }
                    i += 1 + count;
                }
                GC_OP_DRAW_INSTANCED => i += 4,
                GC_OP_STALL => i += 2,
                _ => i += 1,
            }
        }
    }
    regs
}

/// The 16 bytes at GPU address `gpu`, as the four dwords the stream fetches.
fn fetch16(gpu: u32) -> [u32; 4] {
    with_platform(|p| {
        let (&base, a) = p.allocs.range(..=u64::from(gpu)).next_back().expect("a stream base inside no allocation");
        let off = (u64::from(gpu) - base) as usize;
        assert!(off + 16 <= a.mem.len() * 8, "stream base 0x{gpu:x} runs off its allocation");
        // SAFETY: bounds checked above; the allocation is plain host memory.
        let b = unsafe { core::slice::from_raw_parts((a.mem.as_ptr() as *const u8).add(off), 16) };
        core::array::from_fn(|k| u32::from_ne_bytes(b[4 * k..4 * k + 4].try_into().unwrap()))
    })
}

fn draw_three_inputs(setup: impl FnOnce()) -> BTreeMap<u32, u32> {
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(program(VS, FS));
        glViewport(0, 0, 64, 64);
    }
    no_error("the program");
    setup();
    no_error("the current values");
    let _ = take_kicks();
    // SAFETY: plain GL calls; every array is disabled.
    unsafe {
        glDrawArrays(GL_POINTS, 0, 3);
        glFinish();
    }
    no_error("glDrawArrays");
    final_state(&take_kicks())
}

/// The program's stream `i` carrying input `a` (the elements come in the
/// order of the bound locations p, q, r).
fn check_stream(regs: &BTreeMap<u32, u32>, i: u32, want: [u32; 4], what: &str) {
    let base = *regs.get(&nfe_vertex_streams_base_addr(i)).unwrap_or_else(|| panic!("{what}: stream {i} has no base"));
    assert_eq!(fetch16(base), want, "{what}: the 16 staged bytes are the raw dwords");
    assert_eq!(regs[&nfe_vertex_streams_control(i)], vertex_stream_stride(0), "{what}: zero stride");
    assert_eq!(regs[&nfe_vertex_streams_vertex_divisor(i)], 0, "{what}: divisor 0");
    let cfg = regs[&nfe_generic_attrib_config0(i)];
    assert_eq!(cfg & 0xf, GC_DATA_TYPE_FLOAT, "{what}: a 32-bit fetch, no conversion");
    assert_eq!(cfg & (0x3 << 12), attrib_num(4), "{what}: four components");
    assert_eq!(cfg & (0x3 << 14), attrib_normalize(GC_NORMALIZE_OFF), "{what}: not normalised");
}

#[test]
fn disabled_int_uint_float_inputs_fetch_the_raw_dwords_with_zero_stride() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    with_backend(|| {
        let regs = draw_three_inputs(|| {
            // SAFETY: plain GL calls.
            unsafe {
                glVertexAttribI4i(0, -5, 1 << 30, i32::MAX, -1);
                glVertexAttribI4ui(1, 0xffff_ffff, 0x7fc0_0001, 3, 0);
                glVertexAttrib4f(2, 0.5, -2.0, 1.0e10, 0.25);
            }
        });
        check_stream(&regs, 0, [-5i32 as u32, 1 << 30, i32::MAX as u32, u32::MAX], "ivec4 p");
        check_stream(&regs, 1, [0xffff_ffff, 0x7fc0_0001, 3, 0], "uvec4 q");
        check_stream(
            &regs,
            2,
            [0.5f32.to_bits(), (-2.0f32).to_bits(), 1.0e10f32.to_bits(), 0.25f32.to_bits()],
            "vec4 r",
        );
    });
}

/// An int input fed by glVertexAttrib4f: ES 3.0 2.7 leaves the value
/// undefined; the draw must only not fail. The value is staged with the
/// float's bits.
#[test]
fn an_int_input_fed_by_a_float_current_value_draws() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    with_backend(|| {
        let regs = draw_three_inputs(|| {
            // SAFETY: plain GL calls.
            unsafe {
                glVertexAttrib4f(0, 1.5, -2.5, 3.0, f32::NAN);
                glVertexAttribI4ui(1, 1, 2, 3, 4);
                glVertexAttrib4f(2, 1.0, 2.0, 3.0, 4.0);
            }
        });
        let base = regs[&nfe_vertex_streams_base_addr(0)];
        let _ = fetch16(base);
        assert_eq!(regs[&nfe_vertex_streams_control(0)], vertex_stream_stride(0));
        check_stream(&regs, 1, [1, 2, 3, 4], "uvec4 q");
    });
}
