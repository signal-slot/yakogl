// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A true `bool` uniform reaches the Mali-G610 as Mesa 26.2.1 uploads it:
//! the word all ones (issue #11306).
//!
//! Mesa stores a true bool uniform as `ctx->Const.UniformBooleanTrue`, ~0
//! with native integers (st_extensions.c:1356), and its fragment code
//! compares that word, or its low half, with a compare's mask as it is
//! (`ICMP_OR.v2u16.eq.m1 r1, r1^.h00, u3.w0.h00`). The online compiler
//! draws those words. The GL core stored the word 1, so on the board every
//! dEQP shader case whose expected value is a true `bool` / `bvecN`
//! (`ref_out0`) compared the mask 0xffff with 0x0001 and failed.
//!
//! The test draws `deqp_intval_reslib_bvec_fs` (byte-equal to Mesa's words,
//! `tests/mali_deqp_intval.rs`) through the GL core on the Mali back end
//! over a mock of its OSAL, with `ref_out0` set as `gl_ref.py` set it for
//! the reference (true, false, true), once by `glUniform3i` (1, 0, 1) and
//! once by `glUniform3f` (2.0, 0.0, -0.5), and compares the fragment
//! stage's push block (the FAU words the GPU reads) with the reference's.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_bool_uniform
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali"))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Mutex;
use yakogl::backend;
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glDisableVertexAttribArray, glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::mali::cs::Instr;
use yakogl::osal::mali::{OsalMaliGpu, OsalMaliHeap, OsalMaliKick, OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::util::ENOTSUP;

/* ---- the Mali over a mock OSAL ------------------------------------------ */

/// The OSAL's answers: the board's (panthor-frame on the T6).
const GPU: OsalMaliGpu = OsalMaliGpu::T6;
const HEAP: OsalMaliHeap =
    OsalMaliHeap { ctx_gpu: 0x8000_0001_5000, first_chunk_gpu: 0x8000_00a0_0000, chunk_size: 2 << 20, reserved: 0 };

/// Buffers made so far: each one's GPU VA is its number times 16 MiB, far
/// from any CPU address. Nothing is given back (a few MiB for the run).
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static FENCES: AtomicI64 = AtomicI64::new(0);
/// Every buffer: (GPU VA, CPU address, size).
static BUFFERS: Mutex<Vec<(u64, usize, usize)>> = Mutex::new(Vec::new());
/// Every submit: the stream's GPU VA and length in bytes.
static KICKS: Mutex<Vec<(u64, usize)>> = Mutex::new(Vec::new());

unsafe extern "C" fn m_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let n = ALLOCS.fetch_add(1, Ordering::Relaxed) + 1;
    assert!(size <= 16 << 20);
    let cpu = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice()).as_mut_ptr().cast::<c_void>();
    let gpu = 0x40_0000_0000 + n * (16 << 20);
    BUFFERS.lock().unwrap().push((gpu, cpu as usize, size));
    *out = OsalGpuMem { cpu, gpu, phys: 0, size, impl_: n as *mut c_void };
    0
}
unsafe extern "C" fn m_free(mem: *mut OsalGpuMem) -> i32 {
    *mem = OsalGpuMem::ZERO;
    0
}
unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn m_init(_arg: *mut c_void) -> i32 {
    0
}
unsafe extern "C" fn m_submit(kick: *const c_void, len: usize) -> i64 {
    assert_eq!(len, core::mem::size_of::<OsalMaliKick>());
    let k = &*kick.cast::<OsalMaliKick>();
    KICKS.lock().unwrap().push((k.stream_gpu, k.stream_bytes as usize));
    FENCES.fetch_add(1, Ordering::Relaxed) + 1
}
unsafe extern "C" fn m_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}
unsafe extern "C" fn m_completed() -> i64 {
    FENCES.load(Ordering::Relaxed)
}
unsafe extern "C" fn m_query(what: u32, out: *mut c_void, sz: usize) -> i32 {
    match what {
        OSAL_QUERY_MALI_GPU if sz == core::mem::size_of::<OsalMaliGpu>() => *out.cast::<OsalMaliGpu>() = GPU,
        OSAL_QUERY_MALI_TILER_HEAP if sz == core::mem::size_of::<OsalMaliHeap>() => *out.cast::<OsalMaliHeap>() = HEAP,
        _ => return -ENOTSUP,
    }
    0
}

/// The OSAL's log hook: the library only calls it as `log("%s", line)`, so a
/// two-pointer function takes its place (the same calling convention for
/// these arguments).
unsafe extern "C" fn m_log(_fmt: *const c_char, line: *const c_char) {
    eprintln!("osal log: {}", core::ffi::CStr::from_ptr(line).to_string_lossy());
}

static OSAL: OsalOps = OsalOps {
    init: Some(m_init),
    shutdown: None,
    gpu_alloc: Some(m_alloc),
    gpu_free: Some(m_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(m_cache),
    cache_invd: Some(m_cache),
    submit: Some(m_submit),
    wait: Some(m_wait),
    completed: Some(m_completed),
    reg_read: None,
    reg_write: None,
    query: Some(m_query),
    // SAFETY: see `m_log`.
    log: Some(unsafe {
        core::mem::transmute::<
            unsafe extern "C" fn(*const c_char, *const c_char),
            unsafe extern "C" fn(*const c_char, ...),
        >(m_log)
    }),
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/// `len` bytes of the buffer holding GPU address `gpu`.
fn mem(gpu: u64, len: usize) -> Vec<u8> {
    let b = BUFFERS.lock().unwrap();
    let &(base, cpu, _) = b
        .iter()
        .find(|&&(base, _, size)| gpu >= base && gpu + len as u64 <= base + size as u64)
        .unwrap_or_else(|| panic!("no buffer holds 0x{gpu:x}+{len}"));
    // SAFETY: the buffer is leaked, `size` bytes long, and the range is inside it.
    unsafe { core::slice::from_raw_parts((cpu + (gpu - base) as usize) as *const u8, len).to_vec() }
}

/// The fragment stage's FAU register (`frame::regs::FAU_FRAGMENT`): the push
/// block's address, its length in 64-bit slots in bits 56-63.
const FAU_FRAGMENT: usize = 12;

/// The fragment push block of each RUN_IDVS of the kicks since `from`, read
/// as the GPU would: the stream's register moves are followed.
fn fragment_pushes(from: usize) -> Vec<Vec<u32>> {
    let kicks = KICKS.lock().unwrap()[from..].to_vec();
    let mut out = Vec::new();
    for (gpu, bytes) in kicks {
        let raw = mem(gpu, bytes);
        let mut regs = [0u32; 128];
        for w in raw.as_chunks::<8>().0 {
            match Instr::decode(u64::from_le_bytes(*w)) {
                Ok(Instr::Move48(m)) => {
                    regs[m.destination as usize] = m.immediate as u32;
                    regs[m.destination as usize + 1] = (m.immediate >> 32) as u32;
                }
                Ok(Instr::Move32(m)) => regs[m.destination as usize] = m.immediate,
                Ok(Instr::RunIdvs(_)) => {
                    let f = u64::from(regs[FAU_FRAGMENT]) | u64::from(regs[FAU_FRAGMENT + 1]) << 32;
                    let words = 2 * (f >> 56) as usize;
                    let push = mem(f & 0x00ff_ffff_ffff_ffff, 4 * words);
                    out.push(push.as_chunks::<4>().0.iter().map(|b| u32::from_le_bytes(*b)).collect());
                }
                _ => {}
            }
        }
    }
    out
}

/* ---- the reference ---------------------------------------------------------- */

fn testdata(name: &str) -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata").join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// The `fau` words of the reference's `fs.fragment` region.
fn reference_fau(json: &str) -> Vec<u32> {
    let fs = &json[json.find("\"fs.fragment\"").expect("an fs.fragment region")..];
    let fau = &fs[fs.find("\"fau\": [").expect("a fau list") + 8..];
    fau[..fau.find(']').expect("the end of the fau list")]
        .split(',')
        .map(|w| u32::from_str_radix(w.trim().trim_matches('"'), 16).expect("a hex word"))
        .collect()
}

/* ---- the draw ---------------------------------------------------------------- */

const W: GLsizei = 64;

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

fn ok(what: &str) {
    // SAFETY: plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: glGetError 0x{e:x}");
}

/// One triangle of the program `p`, then a flush; its fragment push block.
fn draw(p: GLuint) -> Vec<u32> {
    static POS: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    static IN0: [f32; 9] = [1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0];
    let from = KICKS.lock().unwrap().len();
    let before = backend::draws_refused();
    // SAFETY: the client arrays are statics; the draw takes the vertices
    // before it returns.
    unsafe {
        let pos = glGetAttribLocation(p, c"dEQP_Position".as_ptr()) as GLuint;
        let in0 = glGetAttribLocation(p, c"a_in0".as_ptr()) as GLuint;
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(pos, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glVertexAttribPointer(in0, 3, GL_FLOAT, 0, 0, IN0.as_ptr() as *const c_void);
        glEnableVertexAttribArray(pos);
        glEnableVertexAttribArray(in0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glDisableVertexAttribArray(pos);
        glDisableVertexAttribArray(in0);
        glFinish();
    }
    ok("draw");
    assert_eq!(backend::draws_refused(), before, "the draw was refused");
    let pushes = fragment_pushes(from);
    assert_eq!(pushes.len(), 1, "one draw");
    pushes.into_iter().next().unwrap()
}

#[test]
fn a_true_bool_uniform_is_pushed_as_mesa_s_word() {
    let name = "deqp_intval_reslib_bvec_fs";
    let want = reference_fau(&testdata(&format!("{name}.expect.json")));
    backend::HOST_DRAW_TARGETS.store(1, Ordering::Relaxed);
    // SAFETY: static tables and one context; this is the only test of the
    // file.
    unsafe {
        osal_set_ops(&OSAL);
        backend::g_backend = &yakogl::mali::backend::yakogl_backend_mali;
        assert_eq!(yakogl::mali::backend::mali_init(), 0);
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert!(!yakogl::gl::context::gl_context_create(W as u32, W as u32).is_null());
        let v = shader(GL_VERTEX_SHADER, &testdata(&format!("{name}.vert")));
        let f = shader(GL_FRAGMENT_SHADER, &testdata(&format!("{name}.frag")));
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut linked: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut linked);
        assert_eq!(linked, 1);
        glUseProgram(p);
        glViewport(0, 0, W, W);
        let at = glGetUniformLocation(p, c"ref_out0".as_ptr());
        glUniform3i(at, 1, 0, 1);
        ok("glUniform3i");
        assert_eq!(draw(p), want, "glUniform3i(1, 0, 1): the push block is Mesa's FAU");
        glUniform3i(at, 0, 1, 0);
        glUniform3f(at, 2.0, 0.0, -0.5);
        ok("glUniform3f");
        assert_eq!(draw(p), want, "glUniform3f(2.0, 0.0, -0.5): the push block is Mesa's FAU");
        let mut back = [7; 3];
        glGetUniformiv(p, at, back.as_mut_ptr());
        assert_eq!(back, [1, 0, 1], "glGetUniformiv still reads GL_TRUE as 1");
    }
}
