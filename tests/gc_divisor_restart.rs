// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): primitive restart in a mixed-divisor
//! glDrawElementsInstanced, with V1, the hardware per-stream divisor
//! (docs/notes/instanceid-per-draw.md, `max_instance_divisor` 0xffff).
//!
//! glEnable(GL_PRIMITIVE_RESTART_FIXED_INDEX), two arrays at
//! glVertexAttribDivisor 0 and 2, 4 instances of a GL_TRIANGLE_STRIP whose
//! element buffer holds the type's restart index (0xffff for
//! GL_UNSIGNED_SHORT, 0xff for GL_UNSIGNED_BYTE) between two pieces:
//!
//! * the stream has exactly ONE indexed DRAW_INSTANCED and it carries the 4
//!   instances (the GL core does not split the draw per instance, and no
//!   draw is refused: `gc_refused_draws` unchanged);
//! * the divisor words at 0x14680 and 0x14684 are 0 and 2;
//! * the restart is honoured (ES 3.0 2.8.1): the draw is the triangles the
//!   two strip pieces make, in GL's vertex order, and the restart index
//!   never reaches the hardware as a vertex.
//!
//! Where the restart happens: the GL core, not the front end.
//! `draw_elements` (src/gl/draw.rs) rewrites an index list that holds the
//! restart index as the independent list primitive of the mode
//! (`restart_to_lists`) and draws that, and the back end writes its index
//! group (`Group::Index`, src/vivante/dedup.rs) with
//! GC_FE_PRIMITIVE_RESTART_INDEX 0. So this test checks the indices the
//! index stream address points at (read back from the stand-in's memory),
//! not a hardware restart enable: with the rewrite there is no restart
//! index left for the hardware to find, which this test also pins (no
//! element of the stream equals the restart index of the stream's type).
//!
//! How it runs: the real GL core and the real Vivante back end over a
//! stand-in OSAL that keeps every submitted stream (as
//! tests/gc_stream_divisor.rs), and keeps the memory of a freed allocation
//! readable until the next case, so the index stream can be read after
//! glFinish.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_divisor_restart
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::gl::buffer::{glBindBuffer, glBufferData, glGenBuffers};
use yakogl::gl::consts::*;
use yakogl::gl::draw::glDrawElementsInstanced;
use yakogl::gl::program::{
    glAttachShader, glBindAttribLocation, glCreateProgram, glGetProgramiv, glLinkProgram, glUseProgram,
};
use yakogl::gl::shader::{glCompileShader, glCreateShader, glGetShaderiv, glShaderSource};
use yakogl::gl::state::{glEnable, glFinish, glGetError, glViewport};
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribDivisor, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{gc_init, gc_shutdown, yakogl_backend_vivante};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

/* ---- the stand-in OSAL: host memory, and the streams the back end submits --------- */

struct Alloc {
    mem: Box<[u64]>,
    size: usize,
}

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Alloc>,
    /// Freed allocations, kept readable until the next case (addresses are
    /// never handed out twice within one case).
    retired: BTreeMap<u64, Alloc>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    kicks: Vec<Vec<u32>>,
    faults: Vec<String>,
}

impl Platform {
    /// `n` bytes at GPU address `gpu`, from a live or a freed allocation;
    /// `None` when no allocation holds all of them.
    fn read(&self, gpu: u64, n: usize) -> Option<Vec<u8>> {
        for map in [&self.allocs, &self.retired] {
            if let Some((&base, a)) = map.range(..=gpu).next_back() {
                let off = (gpu - base) as usize;
                if off + n <= a.size {
                    // SAFETY: `mem` holds at least `size` bytes.
                    let bytes = unsafe { core::slice::from_raw_parts(a.mem.as_ptr() as *const u8, a.size) };
                    return Some(bytes[off..off + n].to_vec());
                }
            }
        }
        None
    }
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
        p.allocs.insert(gpu, Alloc { mem, size });
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor.
    let m = unsafe { *mem };
    with_platform(|p| {
        /* retired, not freed: a static of the library that still names the
         * block must find memory (tests/gc_npot_repeat.rs), and the case
         * reads the index stream after glFinish */
        match p.allocs.remove(&m.gpu) {
            Some(a) => {
                p.retired.insert(m.gpu, a);
            }
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
        for (_, a) in core::mem::take(&mut p.allocs).into_iter().chain(core::mem::take(&mut p.retired)) {
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

/// A linked program of `vs` and `fs`, attribute `p` at 0 and `q` at 1.
fn program(vs: &str, fs: &str) -> GLuint {
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glBindAttribLocation(p, 1, c"q".as_ptr());
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

/* ---- decoding the stream ------------------------------------------------------------ */

/// One DRAW_INSTANCED and the state its stream had loaded when it ran
/// (`u32::MAX`: never written in that stream).
#[derive(Debug)]
struct Draw {
    indexed: bool,
    prim: u32,
    instances: u32,
    vertices: u32,
    first: u32,
    /// The divisor words of streams 0 and 1 (0x14680, 0x14684).
    divisors: [u32; 2],
    index_base: u32,
    index_control: u32,
    restart_index: u32,
}

/// Every DRAW_INSTANCED of `kicks`, in order.
fn draws(kicks: &[Vec<u32>]) -> Vec<Draw> {
    let divisor_regs = [0, 1].map(nfe_vertex_streams_vertex_divisor);
    let mut out = Vec::new();
    for words in kicks {
        let mut state: BTreeMap<u32, u32> = BTreeMap::new();
        let get = |s: &BTreeMap<u32, u32>, reg: u32| s.get(&reg).copied().unwrap_or(u32::MAX);
        let mut i = 0usize;
        while i < words.len() {
            let w = words[i];
            match w & 0xf800_0000 {
                GC_OP_LOAD_STATE => {
                    let count = ((w >> 16) & 0x3ff) as usize;
                    let base = (w & 0xffff) * 4;
                    for k in 0..count {
                        if let Some(&v) = words.get(i + 1 + k) {
                            state.insert(base + 4 * k as u32, v);
                        }
                    }
                    i += 1 + count;
                }
                GC_OP_DRAW_INSTANCED => {
                    let count = words.get(i + 1).copied().unwrap_or(0);
                    out.push(Draw {
                        indexed: w & 0x0010_0000 != 0,
                        prim: (w >> 16) & 0xf,
                        instances: (w & 0xffff) | ((count >> 24) << 16),
                        vertices: count & 0x00ff_ffff,
                        first: words.get(i + 2).copied().unwrap_or(0),
                        divisors: divisor_regs.map(|r| get(&state, r)),
                        index_base: get(&state, GC_FE_INDEX_STREAM_BASE_ADDR),
                        index_control: get(&state, GC_FE_INDEX_STREAM_CONTROL),
                        restart_index: get(&state, GC_FE_PRIMITIVE_RESTART_INDEX),
                    });
                    i += 4;
                }
                GC_OP_STALL => i += 2,
                _ => i += 1,
            }
        }
    }
    out
}

/// The indices an indexed draw reads, from the stand-in's memory, and the
/// largest value of the stream's index type (the value a hardware restart
/// would look for).
fn indices_of(d: &Draw) -> (Vec<u32>, u32) {
    let (esz, largest) = match d.index_control & 0x3 {
        GC_INDEX_TYPE_U8 => (1usize, 0xff),
        GC_INDEX_TYPE_U16 => (2, 0xffff),
        _ => (4, u32::MAX),
    };
    let at = u64::from(d.index_base) + u64::from(d.first) * esz as u64;
    let n = d.vertices as usize * esz;
    let bytes = with_platform(|p| p.read(at, n))
        .unwrap_or_else(|| panic!("the index stream 0x{at:x} (+{n} bytes) is in no allocation"));
    let list = bytes
        .chunks_exact(esz)
        .map(|s| match esz {
            1 => u32::from(s[0]),
            2 => u32::from(u16::from_le_bytes([s[0], s[1]])),
            _ => u32::from_le_bytes([s[0], s[1], s[2], s[3]]),
        })
        .collect();
    (list, largest)
}

/* ---- the cases --------------------------------------------------------------------- */

const VS2: &str = "attribute vec4 p;\nattribute vec4 q;\nvarying vec4 v;\n\
                   void main() { v = q; gl_Position = p; }\n";
const FS: &str = "precision mediump float;\nvarying vec4 v;\nvoid main() { gl_FragColor = v; }\n";

/// Positions (attribute 0) of 4 vertices, then 2 colours (attribute 1, one
/// per 2 instances: 4 instances at divisor 2 read 2 of them).
static DATA: [f32; 24] = [
    -1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0, //
    1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0,
];

/// The strip of the cases: vertices 0..4, the restart index, then 0..3.
const STRIP: [u32; 8] = [0, 1, 2, 3, u32::MAX, 0, 1, 2];

/// The triangles GL makes of STRIP (ES 3.0 2.8.1): the first piece is
/// (0, 1, 2) and the odd (2, 1, 3), the second (0, 1, 2).
const TRIANGLES: [u32; 9] = [0, 1, 2, 2, 1, 3, 0, 1, 2];

/// GL_PRIMITIVE_RESTART_FIXED_INDEX on, two arrays at divisors 0 and 2, an
/// element buffer holding `bytes` (STRIP as type `ty`, `restart` in place of
/// its marker), then glDrawElementsInstanced(GL_TRIANGLE_STRIP) of the 8
/// indices and 4 instances.
fn run(ty: GLenum, restart: u32, bytes: &[u8]) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    with_backend(|| {
        // SAFETY: plain GL calls; DATA is a static, `bytes` outlives the calls.
        unsafe {
            glUseProgram(program(VS2, FS));
            glViewport(0, 0, 64, 64);
            glEnable(GL_PRIMITIVE_RESTART_FIXED_INDEX);
            let mut bo = [0 as GLuint; 2];
            glGenBuffers(2, bo.as_mut_ptr());
            glBindBuffer(GL_ARRAY_BUFFER, bo[0]);
            glBufferData(
                GL_ARRAY_BUFFER,
                core::mem::size_of_val(&DATA) as GLsizeiptr,
                DATA.as_ptr() as *const c_void,
                GL_STATIC_DRAW,
            );
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, bo[1]);
            glBufferData(
                GL_ELEMENT_ARRAY_BUFFER,
                bytes.len() as GLsizeiptr,
                bytes.as_ptr() as *const c_void,
                GL_STATIC_DRAW,
            );
            glEnableVertexAttribArray(0);
            glEnableVertexAttribArray(1);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null());
            glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, (16 * 4) as *const c_void);
            glVertexAttribDivisor(0, 0);
            glVertexAttribDivisor(1, 2);
        }
        no_error("the setup");
        let _ = take_kicks();
        let refused = yakogl::vivante::backend::gc_refused_draws();
        // SAFETY: plain GL calls; the indices come from the bound buffer.
        unsafe {
            glDrawElementsInstanced(GL_TRIANGLE_STRIP, STRIP.len() as GLsizei, ty, core::ptr::null(), 4);
            glFinish();
        }
        no_error("glDrawElementsInstanced");
        assert_eq!(yakogl::vivante::backend::gc_refused_draws().wrapping_sub(refused), 0, "no draw refused");

        let all = draws(&take_kicks());
        assert_eq!(all.len(), 1, "ONE DRAW_INSTANCED (not split per instance): {all:x?}");
        let d = &all[0];
        assert!(d.indexed, "the draw is indexed: {d:x?}");
        assert_eq!(d.instances, 4, "the draw carries the 4 instances: {d:x?}");
        assert_eq!(d.divisors, [0, 2], "divisor words at 0x14680, 0x14684: {d:x?}");
        assert_eq!(d.prim, GC_PRIM_TRIANGLES, "the restarted strip is drawn as its triangles: {d:x?}");
        assert_eq!(d.vertices as usize, TRIANGLES.len(), "the triangles' index count: {d:x?}");

        let (list, largest) = indices_of(d);
        assert_eq!(list, TRIANGLES, "the index stream holds the strip pieces' triangles in GL's order");
        assert!(
            !list.contains(&restart) && !list.contains(&largest),
            "no restart index (0x{restart:x}, or 0x{largest:x} of the stream's type) reaches the hardware: {list:?}"
        );
        assert_eq!(d.restart_index, 0, "the index group's GC_FE_PRIMITIVE_RESTART_INDEX: {d:x?}");
    });
}

#[test]
fn unsigned_short_restart_mixed_divisors_are_one_indexed_instanced_draw() {
    let bytes: Vec<u8> =
        STRIP.iter().flat_map(|&i| (if i == u32::MAX { 0xffff } else { i as u16 }).to_ne_bytes()).collect();
    run(GL_UNSIGNED_SHORT, 0xffff, &bytes);
}

#[test]
fn unsigned_byte_restart_mixed_divisors_are_one_indexed_instanced_draw() {
    let bytes: Vec<u8> = STRIP.iter().map(|&i| if i == u32::MAX { 0xff } else { i as u8 }).collect();
    run(GL_UNSIGNED_BYTE, 0xff, &bytes);
}
