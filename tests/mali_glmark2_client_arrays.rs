// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2-es2's build scene with `use-vbo=false` on the Mali back end,
//! through the GL API on the host: vertex arrays and the index array live in
//! client memory (no buffer bound), glVertexAttribPointer takes client
//! pointers, and the draw is glDrawArrays or glDrawElements with a
//! GL_UNSIGNED_SHORT index pointer.
//!
//! The real Mali back end runs over a mock OSAL; no GPU runs, so no pixel is
//! drawn and the default framebuffer cannot be read back with a meaningful
//! colour. What the test reads back instead is what the GPU would be handed:
//! the draw's attribute records and the bytes behind them (the staged client
//! arrays, or the buffer object's storage), vertex by vertex, against the
//! arrays the test gave GL, and the index words. `glReadPixels` is called
//! after each draw and must raise no error (its values are not checked).
//! Every GL call is followed by glGetError() == GL_NO_ERROR, and `mali_draw`
//! must accept every draw.
//!
//! Cases: interleaved position+normal; separate position and normal arrays;
//! glDrawArrays(GL_TRIANGLES); glDrawElements with client u16 indices
//! (also with a non-zero first index and a sparse index set); a frame that
//! switches between a VBO-bound draw and a client-array draw.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_client_arrays
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali"))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::mali::{OsalMaliGpu, OsalMaliHeap, OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::util::ENOTSUP;

/* ---- the Mali over a mock OSAL ------------------------------------------ */

const GPU: OsalMaliGpu = OsalMaliGpu::T6;
const HEAP: OsalMaliHeap =
    OsalMaliHeap { ctx_gpu: 0x8000_0001_5000, first_chunk_gpu: 0x8000_00a0_0000, chunk_size: 2 << 20, reserved: 0 };

static FENCES: AtomicI64 = AtomicI64::new(0);

/// Buffers made so far: (GPU VA, CPU address, bytes). Each VA is its number
/// times 16 MiB, far from any CPU address.
static BUFFERS: Mutex<Vec<(u64, usize, usize)>> = Mutex::new(Vec::new());

unsafe extern "C" fn m_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    assert!(size <= 16 << 20);
    let cpu = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice()).as_mut_ptr().cast::<c_void>();
    let mut b = BUFFERS.lock().unwrap();
    let gpu = 0x40_0000_0000 + (b.len() as u64 + 1) * (16 << 20);
    b.push((gpu, cpu as usize, size));
    *out = OsalGpuMem { cpu, gpu, phys: 0, size, impl_: cpu };
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

/// The CPU address of `n` bytes at GPU VA `gpu`, inside one buffer.
fn cpu_of(gpu: u64, n: usize) -> Option<usize> {
    let b = BUFFERS.lock().unwrap();
    let &(g, cpu, size) = b.iter().find(|&&(g, _, s)| (g..g + s as u64).contains(&gpu))?;
    let off = (gpu - g) as usize;
    (off + n <= size).then_some(cpu + off)
}

unsafe extern "C" fn m_submit(kick: *const c_void, len: usize) -> i64 {
    let _ = (kick, len);
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

/// The OSAL's log hook: the library only calls it as `log("%s", line)`.
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
    // SAFETY: see `m_log`: the same calling convention for these arguments.
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

/* ---- what the back end was given ---------------------------------------- */

/// One attribute record of a draw, with the bytes behind it copied out (the
/// core reuses its staging for the next draw).
#[derive(Clone, Debug)]
struct Attr {
    vi_reg: u8,
    components: u8,
    stride: u32,
    offset: u32,
    staged: bool,
    data: Vec<u8>,
}

#[derive(Clone, Debug)]
struct Seen {
    index_count: u32,
    index_size: u8,
    indices: Vec<u8>,
    attribs: Vec<Attr>,
    mali: i32,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let dr = &*d;
    let mali = yakogl::mali::backend::mali_draw(d);
    let mut attribs = Vec::new();
    for a in &dr.attribs[..dr.num_attribs as usize] {
        let (staged, data) = if !a.cpu.is_null() {
            (true, core::slice::from_raw_parts(a.cpu.cast::<u8>(), a.cpu_bytes as usize).to_vec())
        } else {
            let n = a.buffer_bytes as usize;
            let cpu = cpu_of(a.buffer_gpu, n).expect("the attribute's buffer");
            (false, core::slice::from_raw_parts(cpu as *const u8, n).to_vec())
        };
        attribs.push(Attr {
            vi_reg: a.vi_reg,
            components: a.components,
            stride: a.stride,
            offset: a.offset,
            staged,
            data,
        });
    }
    let n = dr.index_count as usize * dr.index_size as usize;
    let indices = if dr.index_count == 0 || (dr.index_cpu.is_null() && dr.index_buf_gpu == 0) {
        Vec::new()
    } else if !dr.index_cpu.is_null() {
        core::slice::from_raw_parts(dr.index_cpu.cast::<u8>(), n).to_vec()
    } else {
        let cpu = cpu_of(dr.index_buf_gpu, n).expect("the index buffer");
        core::slice::from_raw_parts(cpu as *const u8, n).to_vec()
    };
    SEEN.lock().unwrap().push(Seen { index_count: dr.index_count, index_size: dr.index_size, indices, attribs, mali });
    mali
}

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

const N: GLsizei = 32;

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        backend::HOST_DRAW_TARGETS.store(1, Ordering::Relaxed);
        // SAFETY: the Mali table copied bit for bit with only `draw`
        // replaced; the OSAL and the copy are leaked; the lock serialises
        // the tests.
        unsafe {
            osal_set_ops(&OSAL);
            let mut b: YakoglBackend = core::ptr::read(&yakogl::mali::backend::yakogl_backend_mali);
            b.draw = Some(b_draw);
            backend::g_backend = Box::leak(Box::new(b));
            assert_eq!(yakogl::mali::backend::mali_init(), 0);
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(N as u32, N as u32).is_null());
            glViewport(0, 0, N, N);
        }
    });
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn ok(what: &str) {
    // SAFETY: plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: glGetError 0x{e:x}");
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
        let mut log = [0u8; 512];
        let mut n: GLsizei = 0;
        glGetShaderInfoLog(s, log.len() as GLsizei, &mut n, log.as_mut_ptr().cast());
        assert_eq!(ok, 1, "{text:?} did not compile: {}", String::from_utf8_lossy(&log[..n as usize]));
        s
    }
}

/// A lit program in build's style: position and normal attributes (0, 1), a
/// varying diffuse term, a uniform colour.
fn lit_program() -> GLuint {
    let vs = shader(
        GL_VERTEX_SHADER,
        "attribute vec3 a_pos;\nattribute vec3 a_nrm;\nvarying float d;\n\
         void main() { gl_Position = vec4(a_pos, 1.0); d = max(a_nrm.z, 0.0); }\n",
    );
    let fs = shader(
        GL_FRAGMENT_SHADER,
        "precision mediump float;\nuniform vec4 u_col;\nvarying float d;\n\
         void main() { gl_FragColor = u_col * d; }\n",
    );
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, vs);
        glAttachShader(p, fs);
        glBindAttribLocation(p, 0, c"a_pos".as_ptr());
        glBindAttribLocation(p, 1, c"a_nrm".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link");
        glUseProgram(p);
        glUniform4f(glGetUniformLocation(p, c"u_col".as_ptr()), 0.25, 0.5, 0.75, 1.0);
        p
    }
}

/* ---- the geometry: a four-vertex quad, position and normal -------------- */

const POS: [[f32; 3]; 4] = [[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [-1.0, 1.0, 0.0], [1.0, 1.0, 0.5]];
const NRM: [[f32; 3]; 4] = [[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.5, 0.5, 0.7071]];
/// Six vertices for glDrawArrays: two triangles of the quad.
const TRI: [usize; 6] = [0, 1, 2, 2, 1, 3];
static IDX: [u16; 6] = [0, 1, 2, 2, 1, 3];

fn flat(v: &[[f32; 3]]) -> Vec<f32> {
    v.iter().flatten().copied().collect()
}

/// The floats attribute `a` fetches for vertex `v`.
fn fetch(a: &Attr, v: usize) -> Vec<f32> {
    let at = a.offset as usize + v * a.stride as usize;
    (0..a.components as usize)
        .map(|c| {
            let b = &a.data[at + 4 * c..at + 4 * c + 4];
            f32::from_le_bytes([b[0], b[1], b[2], b[3]])
        })
        .collect()
}

fn last_draw() -> Seen {
    SEEN.lock().unwrap().pop().expect("the draw reached the back end")
}

/// Every vertex the draw fetches (`order`, the vertex numbers it visits)
/// carries `pos`/`nrm` of the source vertex `src[order[i]]`.
fn check_attribs(s: &Seen, order: &[usize], src: &[usize], what: &str) {
    assert_eq!(s.mali, 0, "{what}: mali_draw refused");
    assert_eq!(s.attribs.len(), 2, "{what}: attributes");
    for (a, table) in [(0u8, &POS), (1, &NRM)] {
        let rec = s.attribs.iter().find(|r| r.vi_reg == a || r.components == 3).and_then(|_| {
            let mut v: Vec<&Attr> = s.attribs.iter().collect();
            v.sort_by_key(|r| r.vi_reg);
            v.get(a as usize).copied()
        });
        let rec = rec.unwrap();
        assert_eq!(rec.components, 3, "{what}: attribute {a} components");
        for (i, &o) in order.iter().enumerate() {
            let want = table[src[o]].to_vec();
            assert_eq!(fetch(rec, o), want, "{what}: attribute {a}, fetch {i} (vertex {o})");
        }
    }
}

/// glReadPixels after a draw raises no error (no GPU ran: no value check).
fn read_back(what: &str) {
    let mut px = [0u8; 4];
    // SAFETY: one RGBA8 pixel into `px`.
    unsafe { glReadPixels(4, 4, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast()) };
    ok(what);
}

/// Client-memory arrays: `interleaved` is one array of 6 floats a vertex,
/// else two arrays.
fn client_arrays(interleaved: bool) {
    let (pos, nrm) = (flat(&POS), flat(&NRM));
    let inter: Vec<f32> = (0..4).flat_map(|i| POS[i].into_iter().chain(NRM[i])).collect();
    // SAFETY: the vectors outlive the draw, which `glFinish` completes.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
        if interleaved {
            let base = inter.as_ptr();
            glVertexAttribPointer(0, 3, GL_FLOAT, 0, 24, base.cast());
            glVertexAttribPointer(1, 3, GL_FLOAT, 0, 24, base.add(3).cast());
        } else {
            glVertexAttribPointer(0, 3, GL_FLOAT, 0, 0, pos.as_ptr().cast());
            glVertexAttribPointer(1, 3, GL_FLOAT, 0, 0, nrm.as_ptr().cast());
        }
        glEnableVertexAttribArray(0);
        glEnableVertexAttribArray(1);
    }
    ok("client attribute pointers");
    // The quad's four vertices as arrays: glDrawElements over IDX.
    // SAFETY: IDX is static.
    unsafe { glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, IDX.as_ptr().cast()) };
    ok("glDrawElements");
    unsafe { glFinish() };
    ok("glFinish");
    let s = last_draw();
    let what = format!("client u16 indices, interleaved {interleaved}");
    assert_eq!(s.index_count, 6, "{what}");
    assert_eq!(s.index_size, 2, "{what}");
    let got: Vec<u16> = s.indices.chunks(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
    assert_eq!(got, IDX, "{what}: the index words");
    assert!(s.attribs.iter().all(|a| a.staged), "{what}: client arrays are staged");
    let order: Vec<usize> = IDX.iter().map(|&i| i as usize).collect();
    check_attribs(&s, &order, &[0, 1, 2, 3], &what);
    read_back("glReadPixels");
}

#[test]
fn client_interleaved_draw_elements() {
    let _g = context();
    lit_program();
    client_arrays(true);
}

#[test]
fn client_separate_draw_elements() {
    let _g = context();
    lit_program();
    client_arrays(false);
}

/// glDrawArrays(GL_TRIANGLES) over six expanded vertices, separate arrays,
/// then the same arrays with a non-zero `first`.
#[test]
fn client_draw_arrays() {
    let _g = context();
    lit_program();
    let pos: Vec<f32> = TRI.iter().flat_map(|&i| POS[i]).collect();
    let nrm: Vec<f32> = TRI.iter().flat_map(|&i| NRM[i]).collect();
    // SAFETY: the vectors outlive the draw, which `glFinish` completes.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 3, GL_FLOAT, 0, 0, pos.as_ptr().cast());
        glVertexAttribPointer(1, 3, GL_FLOAT, 0, 0, nrm.as_ptr().cast());
        glEnableVertexAttribArray(0);
        glEnableVertexAttribArray(1);
        ok("attribute pointers");
        glDrawArrays(GL_TRIANGLES, 0, 6);
        ok("glDrawArrays");
        glFinish();
    }
    ok("glFinish");
    let s = last_draw();
    assert_eq!(s.index_count, 6);
    assert!(s.attribs.iter().all(|a| a.staged));
    let order: Vec<usize> = (0..6).collect();
    check_attribs(&s, &order, &TRI, "glDrawArrays first 0");
    read_back("glReadPixels");

    // The same arrays, drawing the second triangle only (first = 3, count 3).
    // SAFETY: as above.
    unsafe {
        glDrawArrays(GL_TRIANGLES, 3, 3);
        ok("glDrawArrays first 3");
        glFinish();
    }
    let s = last_draw();
    assert_eq!(s.mali, 0);
    assert_eq!(s.index_count, 3);
    // The core stages vertices first.. so that vertex `first` lands at 0.
    check_attribs(&s, &[0, 1, 2], &TRI[3..], "glDrawArrays first 3");
}

/// A sparse client index set: only vertices 1 and 3 and 2 are used, the
/// array of four is bigger than the indices reach from the front.
#[test]
fn client_indices_skip_vertex_zero() {
    let _g = context();
    lit_program();
    let (pos, nrm) = (flat(&POS), flat(&NRM));
    static SPARSE: [u16; 3] = [1, 3, 2];
    // SAFETY: the vectors and SPARSE outlive the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 3, GL_FLOAT, 0, 0, pos.as_ptr().cast());
        glVertexAttribPointer(1, 3, GL_FLOAT, 0, 0, nrm.as_ptr().cast());
        glEnableVertexAttribArray(0);
        glEnableVertexAttribArray(1);
        glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, SPARSE.as_ptr().cast());
        ok("glDrawElements");
        glFinish();
    }
    let s = last_draw();
    let got: Vec<u16> = s.indices.chunks(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
    assert_eq!(got, SPARSE);
    check_attribs(&s, &[1, 3, 2], &[0, 1, 2, 3], "sparse indices");
}

/// One frame that alternates a VBO-bound draw and a client-array draw (build's
/// `use-vbo` true and false share the program): the buffer's bytes are
/// fetched in place, then the client arrays, then the buffer again, with no
/// error and no leftover binding.
#[test]
fn vbo_then_client_then_vbo_in_one_frame() {
    let _g = context();
    lit_program();
    let inter: Vec<f32> = (0..4).flat_map(|i| POS[i].into_iter().chain(NRM[i])).collect();
    let (mut vbo, mut ibo) = (0, 0);
    // SAFETY: buffers made and filled from live vectors/statics.
    unsafe {
        glGenBuffers(1, &mut vbo);
        glBindBuffer(GL_ARRAY_BUFFER, vbo);
        glBufferData(GL_ARRAY_BUFFER, (inter.len() * 4) as isize, inter.as_ptr().cast(), GL_STATIC_DRAW);
        glGenBuffers(1, &mut ibo);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ibo);
        glBufferData(GL_ELEMENT_ARRAY_BUFFER, 12, IDX.as_ptr().cast(), GL_STATIC_DRAW);
    }
    ok("buffers");
    let order: Vec<usize> = IDX.iter().map(|&i| i as usize).collect();
    let vbo_draw = |round: usize| {
        // SAFETY: bound buffers, byte offsets.
        unsafe {
            glBindBuffer(GL_ARRAY_BUFFER, vbo);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ibo);
            glVertexAttribPointer(0, 3, GL_FLOAT, 0, 24, core::ptr::null());
            glVertexAttribPointer(1, 3, GL_FLOAT, 0, 24, 12usize as *const c_void);
            glEnableVertexAttribArray(0);
            glEnableVertexAttribArray(1);
            glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, core::ptr::null());
            glFinish();
        }
        ok("VBO draw");
        let s = last_draw();
        let what = format!("VBO draw {round}");
        assert!(s.attribs.iter().all(|a| !a.staged), "{what}: fetched in place");
        assert_eq!(s.indices.chunks(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect::<Vec<_>>(), IDX);
        check_attribs(&s, &order, &[0, 1, 2, 3], &what);
        read_back("glReadPixels");
    };
    vbo_draw(0);
    // Client arrays next: the bindings dropped, the pointers client memory.
    client_arrays(true);
    vbo_draw(1);
    client_arrays(false);
    // SAFETY: names made above.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
        glDeleteBuffers(1, &vbo);
        glDeleteBuffers(1, &ibo);
    }
    ok("delete buffers");
    assert_eq!(backend::draws_refused(), 0);
}
