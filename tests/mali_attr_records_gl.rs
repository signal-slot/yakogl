// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The draws `NOT_BUILT_ATTRIBUTE_RECORDS` used to refuse (item attrrec-gl-pin,
//! plan #406), drawn through the GL core on the real Mali back end over a mock
//! of its OSAL, and the attribute and buffer descriptors they build read back
//! from the mock's memory (`src/mali/frame_attrrec_tests.rs` pins the same
//! words at frame level). The shapes:
//!
//! - every location the vertex shader reads has an enabled array: one
//!   ATTRIBUTE and one Buffer per record, the array's;
//! - a location the shader reads with no array enabled reads Mesa's
//!   current-value element (`st_setup_current`): ATTRIBUTE `RGBA32F` 1D at
//!   stride 0, offset 0, and a Simple Buffer of 16 bytes holding GL's
//!   initial (0, 0, 0, 1). The core stages that value itself for a vec4, so
//!   the frame's own element (shared upload, a location nothing stages) is
//!   not reached through GL; an unstaged vec2 is staged at stride 0 with
//!   two components, which the back end builds as the same RGBA32F element;
//! - the refusals the records keep (`NOT_BUILT_ATTRIBUTE`,
//!   `INVALID_ATTRIBUTE_RECORDS`, `NOT_BUILT_ATTRIBUTES`) are not reachable
//!   through GL (the core converts every array to float32 and has 16
//!   locations), so their texts are pinned.
//!
//! The expected words are v10.xml's (Mesa 26.2.1 src/panfrost/genxml/v10.xml:
//! 947-965 Attribute, 1056-1060 Buffer): Descriptor Type Attribute 5, Buffer 9;
//! Attribute Type 1D 1; Buffer Type Simple 1; table `ATTRIBUTE_BUFFER` 2;
//! Pixel Format `RGBA32F` 0xbf; the descriptors sit in entries 1 (Attribute)
//! and 2 (Attribute buffer) of the vertex stage's resource table.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_attr_records_gl
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend;
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glDisableVertexAttribArray, glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::mali::backend::NOT_BUILT_ATTRIBUTE;
use yakogl::osal::mali::{OsalMaliGpu, OsalMaliHeap, OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP};
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
/// Each buffer's CPU address and size, so the test can read the descriptors.
static BUFFERS: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

unsafe extern "C" fn m_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let n = ALLOCS.fetch_add(1, Ordering::Relaxed) + 1;
    assert!(size <= 16 << 20);
    let cpu = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice()).as_mut_ptr().cast::<c_void>();
    BUFFERS.lock().unwrap().push((cpu as usize, size));
    *out = OsalGpuMem { cpu, gpu: 0x40_0000_0000 + n * (16 << 20), phys: 0, size, impl_: n as *mut c_void };
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
unsafe extern "C" fn m_submit(_kick: *const c_void, _len: usize) -> i64 {
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

/// The log lines since the last `context()`.
static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The OSAL's log hook: the library only calls it as `log("%s", line)`, so a
/// two-pointer function takes its place (the same calling convention for
/// these arguments).
unsafe extern "C" fn m_log(_fmt: *const c_char, line: *const c_char) {
    let line = core::ffi::CStr::from_ptr(line).to_string_lossy().into_owned();
    eprintln!("osal log: {line}");
    LOG.lock().unwrap_or_else(|e| e.into_inner()).push(line);
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

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The GL context and an 8x8 RGBA8 framebuffer object bound.
fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        backend::HOST_DRAW_TARGETS.store(1, Ordering::Relaxed);
        // SAFETY: static tables and one context; the lock above serialises
        // the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &yakogl::mali::backend::yakogl_backend_mali;
            assert_eq!(yakogl::mali::backend::mali_init(), 0);
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
            let (mut fb, mut rb) = (0, 0);
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 8, 8);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        }
    });
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clear();
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(0.5, 0.25, 0.75, 0.4);
        glClear(GL_COLOR_BUFFER_BIT);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

/* ---- reading the draw's descriptors back ----------------------------------- */

/// The mock's GPU address of buffer `i` (`m_alloc`).
fn gpu_base(i: usize) -> u64 {
    0x40_0000_0000 + (i as u64 + 1) * (16 << 20)
}

/// `len` bytes at GPU address `gpu`, when a buffer of the mock holds them.
fn at_gpu(gpu: u64, len: usize) -> Option<Vec<u8>> {
    let bufs = BUFFERS.lock().unwrap();
    bufs.iter().enumerate().find_map(|(i, &(cpu, size))| {
        let off = usize::try_from(gpu.checked_sub(gpu_base(i))?).ok()?;
        if off.checked_add(len)? > size {
            return None;
        }
        // SAFETY: the buffer is a leaked allocation of `size` bytes and
        // nothing draws while the test reads it.
        Some(unsafe { std::slice::from_raw_parts((cpu + off) as *const u8, len) }.to_vec())
    })
}

const TABLE_ENTRIES: usize = 7;
const TABLE_BYTES: usize = TABLE_ENTRIES * 16;
const ENTRY_ATTRIBUTE: usize = 1;
const ENTRY_BUFFER: usize = 2;
const ENTRY_SAMPLER: usize = 3;
const ENTRY_TEXTURE: usize = 4;

/// A resource table: each entry's (address, size in bytes).
type Table = [(u64, u64); TABLE_ENTRIES];

/// The table a 112-byte chunk holds, if it is shaped like a vertex stage's
/// with attributes: every entry empty or `Contains descriptors` inside a
/// buffer of the mock, entries 1 and 2 of one size, a multiple of 32 bytes,
/// and a sampler entry (every draw has one).
fn table_shaped(c: &[u8]) -> Option<Table> {
    let mut t = [(0, 0); TABLE_ENTRIES];
    for (i, e) in t.iter_mut().enumerate() {
        let lo = u64::from_le_bytes(c[i * 16..i * 16 + 8].try_into().unwrap());
        let size = u64::from_le_bytes(c[i * 16 + 8..i * 16 + 16].try_into().unwrap());
        if (lo, size) == (0, 0) {
            continue;
        }
        let address = lo & ((1 << 56) - 1);
        if lo >> 56 != 1 || size == 0 || size % 32 != 0 || size > 32 * 64 || i > ENTRY_TEXTURE {
            return None;
        }
        at_gpu(address, size as usize)?;
        *e = (address, size);
    }
    (t[ENTRY_ATTRIBUTE].1 != 0 && t[ENTRY_ATTRIBUTE].1 == t[ENTRY_BUFFER].1 && t[ENTRY_SAMPLER].1 != 0).then_some(t)
}

/// Every table-shaped chunk of the mock's buffers, 64-byte aligned; they are
/// cleared when `zero`.
fn tables(zero: bool) -> Vec<Table> {
    let bufs: Vec<(usize, usize)> = BUFFERS.lock().unwrap().clone();
    let mut found = Vec::new();
    for (cpu, size) in bufs {
        for off in (0..size.saturating_sub(TABLE_BYTES - 1)).step_by(64) {
            // SAFETY: as in `at_gpu`.
            let chunk = unsafe { std::slice::from_raw_parts_mut((cpu + off) as *mut u8, TABLE_BYTES) };
            if let Some(t) = table_shaped(chunk) {
                found.push(t);
                if zero {
                    chunk.fill(0);
                }
            }
        }
    }
    found
}

/// The eight 32-bit words of each 32-byte descriptor at `(address, size)`.
fn words(e: (u64, u64)) -> Vec<[u32; 8]> {
    let bytes = at_gpu(e.0, e.1 as usize).unwrap();
    bytes
        .as_chunks::<32>()
        .0
        .iter()
        .map(|d| core::array::from_fn(|w| u32::from_le_bytes(d[4 * w..4 * w + 4].try_into().unwrap())))
        .collect()
}

/// The vertex stage's (Attribute, Buffer) words of the draw just made. The
/// draw may leave more than one table (the same descriptors); they must agree.
fn vertex_stage(label: &str) -> (Vec<[u32; 8]>, Vec<[u32; 8]>) {
    let found = tables(false);
    assert!(!found.is_empty(), "{label}: no vertex table with attributes");
    let all: Vec<_> = found.iter().map(|t| (words(t[ENTRY_ATTRIBUTE]), words(t[ENTRY_BUFFER]))).collect();
    for s in &all[1..] {
        assert_eq!(s, &all[0], "{label}: the vertex tables differ");
    }
    all.into_iter().next().unwrap()
}

/* ---- Mesa's words (v10.xml) ---------------------------------------------- */

/// Descriptor Type Attribute 5, Attribute Type 1D 1 (bit 4), table
/// ATTRIBUTE_BUFFER 2 is word 1; Pixel Format RGBA32F 0xbf, component order
/// RGBA 0 (bits 12 and up of the format field at bit 10).
const ATTR_RGBA32F_1D: u32 = 5 | 1 << 4 | 1 << 8 | ((0xbf << 12) << 10);
/// RG32F 0xaf with component order RGB1 0x10.
const ATTR_RG32F_1D: u32 = 5 | 1 << 4 | 1 << 8 | ((0xaf << 12 | 0x10) << 10);
/// R32F 0x8f?: not used; a float scalar array is not drawn here.
/// Descriptor Type Buffer 9, Buffer Type Simple 1.
const BUFFER_SIMPLE: u32 = 9 | 1 << 4;

/// Attribute words of a 1D element of `format` word `fmt`, buffer `index`,
/// stride `stride`.
fn attr_words(fmt: u32, index: u32, stride: u32) -> [u32; 8] {
    [fmt, 2, 0, index, stride, 0, 0, 0]
}

fn buffer_words(size: u32, gpu: u64) -> [u32; 8] {
    [BUFFER_SIMPLE, size, gpu as u32, (gpu >> 32) as u32, 0, 0, 0, 0]
}

/// The bytes of one Simple Buffer's contents.
fn contents(b: &[u32; 8]) -> Vec<u8> {
    let gpu = u64::from(b[2]) | u64::from(b[3]) << 32;
    at_gpu(gpu, b[1] as usize).unwrap_or_else(|| panic!("buffer at {gpu:#x} of {} bytes is not in the mock", b[1]))
}

fn floats(bytes: &[u8]) -> Vec<f32> {
    bytes.as_chunks::<4>().0.iter().map(|&b| f32::from_le_bytes(b)).collect()
}

/* ---- the GL calls --------------------------------------------------------- */

/// Covers the whole target.
static FULL: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
/// A vec2 per vertex.
static UV: [f32; 6] = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6];
/// Four bytes per vertex.
static BYTES: [u8; 12] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];

const FS: &str = "#version 100\nvarying mediump vec4 v_color;\nvoid main() { gl_FragColor = v_color; }\n";

fn compile(ty: GLenum, text: &str) -> GLuint {
    let src = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [src.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "the shader does not compile:\n{text}");
        s
    }
}

/// Link `vs` + `FS` with every attribute of `binds` at its location.
fn link(label: &str, vs: &str, binds: &[(&str, GLuint)]) -> GLuint {
    let (v, f) = (compile(GL_VERTEX_SHADER, vs), compile(GL_FRAGMENT_SHADER, FS));
    // SAFETY: plain GL calls; the names are NUL-terminated.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        for (n, l) in binds {
            let n = CString::new(*n).unwrap();
            glBindAttribLocation(p, *l, n.as_ptr());
        }
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        let log = LOG.lock().unwrap_or_else(|e| e.into_inner()).clone();
        assert_eq!(ok, 1, "{label}: the program does not link; log {log:?}");
        assert_eq!(glGetError(), GL_NO_ERROR, "{label}: GL error after the link");
        p
    }
}

/// One array to enable for a draw.
struct Array {
    loc: GLuint,
    size: GLint,
    ty: GLenum,
    stride: GLsizei,
    data: *const c_void,
}

/// What a draw did: its `glGetError`, the draws refused and the refusal
/// lines logged.
struct Drew {
    error: GLenum,
    refused: u32,
    lines: Vec<String>,
}

/// Draw one triangle with `p`, `arrays` enabled and every other of the 16
/// generic arrays disabled; the descriptor tables are cleared first.
fn draw(p: GLuint, arrays: &[Array]) -> Drew {
    let before = backend::draws_refused();
    tables(true);
    // SAFETY: plain GL calls; the arrays are statics that outlive the draw.
    let error = unsafe {
        glUseProgram(p);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        for i in 0..16 {
            glDisableVertexAttribArray(i);
        }
        for a in arrays {
            glVertexAttribPointer(a.loc, a.size, a.ty, 0, a.stride, a.data);
            glEnableVertexAttribArray(a.loc);
        }
        while glGetError() != GL_NO_ERROR {}
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
        let e = glGetError();
        for a in arrays {
            glDisableVertexAttribArray(a.loc);
        }
        e
    };
    let log = LOG.lock().unwrap_or_else(|e| e.into_inner()).clone();
    Drew {
        error,
        refused: backend::draws_refused() - before,
        lines: log.into_iter().filter(|l| l.contains("refus")).collect(),
    }
}

fn pos(loc: GLuint) -> Array {
    Array { loc, size: 4, ty: GL_FLOAT, stride: 0, data: FULL.as_ptr().cast() }
}

fn uv(loc: GLuint) -> Array {
    Array { loc, size: 2, ty: GL_FLOAT, stride: 0, data: UV.as_ptr().cast() }
}

/// The draw is built: no GL error, no refusal.
fn built(label: &str, d: &Drew) {
    assert_eq!(d.refused, 0, "{label}: the draw was refused; refusal lines {:?}", d.lines);
    assert_eq!(d.error, GL_NO_ERROR, "{label}: GL error after the draw");
}

/// A vertex shader reading `a_pos` (vec4), `a_uv` (vec2) and `a_extra`
/// (vec4), `a_extra` added to the colour.
const VS: &str = "#version 100\nattribute vec4 a_pos;\nattribute vec2 a_uv;\nattribute vec4 a_extra;\n\
    varying mediump vec4 v_color;\nvoid main() {\n    gl_Position = a_pos;\n\
    v_color = vec4(a_uv, 0.0, 1.0) + a_extra;\n}\n";

/// The same with `a_uv` a vec4: what Mesa's current-value element (always
/// four floats) is for every type of an unstaged attribute.
const VS4: &str = "#version 100\nattribute vec4 a_pos;\nattribute vec4 a_uv;\nattribute vec4 a_extra;\n\
    varying mediump vec4 v_color;\nvoid main() {\n    gl_Position = a_pos;\n\
    v_color = a_uv + a_extra;\n}\n";

/// Every location read has an enabled array: three records, the arrays'.
#[test]
fn three_staged_arrays_build_three_records() {
    let _g = context();
    let p = link("staged", VS, &[("a_pos", 0), ("a_uv", 1), ("a_extra", 2)]);
    static EXTRA: [f32; 12] = [0.5; 12];
    let extra = Array { loc: 2, size: 4, ty: GL_FLOAT, stride: 0, data: EXTRA.as_ptr().cast() };
    let d = draw(p, &[pos(0), uv(1), extra]);
    built("staged", &d);
    let (a, b) = vertex_stage("staged");
    assert_eq!((a.len(), b.len()), (3, 3), "one Attribute and one Buffer per record");
    assert_eq!(a[0], attr_words(ATTR_RGBA32F_1D, 0, 16));
    assert_eq!(a[1], attr_words(ATTR_RG32F_1D, 1, 8));
    assert_eq!(a[2], attr_words(ATTR_RGBA32F_1D, 2, 16));
    for (i, (len, data)) in [(48, FULL.as_slice()), (24, UV.as_slice()), (48, EXTRA.as_slice())].into_iter().enumerate()
    {
        assert_eq!((b[i][0], b[i][1]), (BUFFER_SIMPLE, len), "Buffer {i}");
        assert_eq!(b[i][4..], [0; 4], "Buffer {i}: the reserved words");
        assert_eq!(floats(&contents(&b[i])), data, "Buffer {i}: the array's bytes");
    }
    // SAFETY: plain GL call.
    unsafe { glDeleteProgram(p) };
}

/// A location the shader reads whose array is not enabled reads the current
/// value element: (0, 0, 0, 1) as four floats at stride 0.
#[test]
fn a_read_location_with_no_array_reads_the_initial_current_value() {
    let _g = context();
    let p = link("current", VS, &[("a_pos", 0), ("a_uv", 1), ("a_extra", 2)]);
    let d = draw(p, &[pos(0), uv(1)]);
    built("current", &d);
    let (a, b) = vertex_stage("current");
    assert_eq!((a.len(), b.len()), (3, 3));
    assert_eq!(a[0], attr_words(ATTR_RGBA32F_1D, 0, 16), "the staged vec4");
    assert_eq!(a[1], attr_words(ATTR_RG32F_1D, 1, 8), "the staged vec2");
    assert_eq!(a[2], attr_words(ATTR_RGBA32F_1D, 2, 0), "the current-value Attribute: stride 0");
    let gpu = u64::from(b[2][2]) | u64::from(b[2][3]) << 32;
    assert_eq!(b[2], buffer_words(16, gpu), "the current-value Buffer: Simple, 16 bytes");
    assert_eq!(floats(&contents(&b[2])), [0.0, 0.0, 0.0, 1.0], "GL's initial current value");
    // SAFETY: plain GL call.
    unsafe { glDeleteProgram(p) };
}

/// Two unread-by-array locations, neither the last nor the first. The
/// core stages each one's current value itself (a four-float element of its
/// own), so the frame-level shape of one shared upload is not reached
/// through GL; each Buffer is a Simple one of 16 bytes holding the value.
#[test]
fn two_unstaged_locations_read_the_initial_current_value_each() {
    let _g = context();
    let p = link("two current", VS4, &[("a_pos", 3), ("a_uv", 7), ("a_extra", 12)]);
    let d = draw(p, &[pos(3)]);
    built("two current", &d);
    let (a, b) = vertex_stage("two current");
    assert_eq!((a.len(), b.len()), (3, 3), "records count the read locations, not their numbers");
    assert_eq!(a[0], attr_words(ATTR_RGBA32F_1D, 0, 16));
    for i in 1..3 {
        assert_eq!(a[i], attr_words(ATTR_RGBA32F_1D, i as u32, 0), "Attribute {i}");
        let gpu = u64::from(b[i][2]) | u64::from(b[i][3]) << 32;
        assert_eq!(b[i], buffer_words(16, gpu), "Buffer {i}");
        assert_eq!(floats(&contents(&b[i])), [0.0, 0.0, 0.0, 1.0], "Buffer {i}");
    }
    // SAFETY: plain GL call.
    unsafe { glDeleteProgram(p) };
}

/// A record that no location names does not arise through GL: the compiler
/// numbers a record per location read, so a location the shader never reads
/// takes no record (it is inactive). The unread attribute's array, enabled,
/// changes nothing: its record is not there.
#[test]
fn an_enabled_array_of_an_unread_attribute_takes_no_record() {
    let _g = context();
    let vs = "#version 100\nattribute vec4 a_pos;\nattribute vec2 a_dead;\nvarying mediump vec4 v_color;\n\
        void main() {\n    gl_Position = a_pos;\n    v_color = vec4(1.0);\n}\n";
    let p = link("unread", vs, &[("a_pos", 0), ("a_dead", 1)]);
    let d = draw(p, &[pos(0), uv(1)]);
    built("unread", &d);
    let (a, b) = vertex_stage("unread");
    assert_eq!((a.len(), b.len()), (1, 1), "a_pos's record only");
    assert_eq!(a[0], attr_words(ATTR_RGBA32F_1D, 0, 16));
    // SAFETY: plain GL call.
    unsafe { glDeleteProgram(p) };
}

/// An unstaged vec2 attribute: Mesa's current-value element is four floats
/// whatever the attribute's type, RGBA32F. The core stages the disabled
/// array's current value with the vec2's two components at stride 0; the
/// Mali back end builds it as Mesa's element (word 0 0x2fc00115, not RG32F's
/// 0x2bc04115), a Simple Buffer of 16 bytes holding (0, 0, 0, 1).
#[test]
fn an_unstaged_vec2_reads_mesas_rgba32f_current_value() {
    let _g = context();
    let p = link("vec2 current", VS, &[("a_pos", 0), ("a_uv", 1), ("a_extra", 2)]);
    let extra = Array { loc: 2, size: 4, ty: GL_FLOAT, stride: 0, data: FULL.as_ptr().cast() };
    let d = draw(p, &[pos(0), extra]);
    built("vec2 current", &d);
    let (a, b) = vertex_stage("vec2 current");
    assert_eq!(a[1], attr_words(ATTR_RGBA32F_1D, 1, 0));
    assert_eq!(a[2], attr_words(ATTR_RGBA32F_1D, 2, 16), "the staged vec4 array is unchanged");
    let gpu = u64::from(b[1][2]) | u64::from(b[1][3]) << 32;
    assert_eq!(b[1], buffer_words(16, gpu), "the current-value Buffer: Simple, 16 bytes");
    assert_eq!(floats(&contents(&b[1])), [0.0, 0.0, 0.0, 1.0], "GL's initial current value");
    // SAFETY: plain GL call.
    unsafe { glDeleteProgram(p) };
}

/// A byte array is converted to float32 by the core (the back end reports no
/// native attribute format), so the draw is built with the float32 words of
/// the converted data: the back end's `NOT_BUILT_ATTRIBUTE` is not reached
/// through GL.
#[test]
fn a_byte_array_reaches_the_back_end_as_float32() {
    let _g = context();
    let p = link("bytes", VS, &[("a_pos", 0), ("a_uv", 1), ("a_extra", 2)]);
    let bytes = Array { loc: 2, size: 4, ty: GL_UNSIGNED_BYTE, stride: 0, data: BYTES.as_ptr().cast() };
    let d = draw(p, &[pos(0), uv(1), bytes]);
    built("bytes", &d);
    let (a, b) = vertex_stage("bytes");
    assert_eq!(a.len(), 3);
    assert_eq!(a[2][0], ATTR_RGBA32F_1D, "converted to RGBA32F");
    assert_eq!(a[2][4], 16, "the converted array's stride");
    assert_eq!(b[2][0], BUFFER_SIMPLE);
    // SAFETY: plain GL call.
    unsafe { glDeleteProgram(p) };
}

/// A mat2 attribute: two columns at locations 1 and 2, one record each.
#[test]
fn a_mat2_attribute_builds_a_record_per_column() {
    let _g = context();
    let vs = "#version 100\nattribute vec4 a_pos;\nattribute mat2 a_m;\nvarying mediump vec4 v_color;\n\
        void main() {\n    gl_Position = a_pos;\n    v_color = vec4(a_m[0], a_m[1]);\n}\n";
    let p = link("mat2", vs, &[("a_pos", 0), ("a_m", 1)]);
    static COLS: [f32; 12] = [0.5; 12];
    let (c0, c1) = (
        Array { loc: 1, size: 2, ty: GL_FLOAT, stride: 0, data: COLS.as_ptr().cast() },
        Array { loc: 2, size: 2, ty: GL_FLOAT, stride: 0, data: COLS.as_ptr().cast() },
    );
    let d = draw(p, &[pos(0), c0, c1]);
    built("mat2", &d);
    let (a, b) = vertex_stage("mat2");
    assert_eq!((a.len(), b.len()), (3, 3));
    assert_eq!(a[1], attr_words(ATTR_RG32F_1D, 1, 8));
    assert_eq!(a[2], attr_words(ATTR_RG32F_1D, 2, 8));
    // SAFETY: plain GL call.
    unsafe { glDeleteProgram(p) };
}

/// The refusals the attribute records keep are the back end's texts: none of
/// them is reachable through GL (the core converts every format and limits
/// the locations to 16), so their texts are pinned here, and the frame-level
/// tests (`src/mali/frame_attrrec_tests.rs`) draw them.
#[test]
fn the_attribute_refusal_texts_are_kept() {
    assert_eq!(
        NOT_BUILT_ATTRIBUTE,
        "mali: a vertex attribute other than float32 data (staged by the GL core, or in a buffer object) is not built yet"
    );
    assert_eq!(
        yakogl::mali::backend::INVALID_ATTRIBUTE_RECORDS,
        "mali: the vertex shader's attribute records name one record from two locations"
    );
    assert_eq!(
        yakogl::mali::frame::NOT_BUILT_ATTRIBUTES,
        "mali frame: a draw with more than 16 vertex attributes, or one of other than 1 to 4 32-bit floats, is not built yet"
    );
}
