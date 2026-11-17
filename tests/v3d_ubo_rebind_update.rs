// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Rebinding and updating full uniform blocks between draws, through the GL
//! API on the host V3D build. A fragment program reads 12 blocks of 16384
//! bytes (8 members of `vec4 [128]` each: `array_len` is a u8 today). The
//! blocks are mapped with `glUniformBlockBinding` to binding points spread
//! over 0..`GL_MAX_UNIFORM_BUFFER_BINDINGS`-1 (24 on the V3D), 24 buffers of
//! 16384 bytes are bound to all 24 points, and the program is drawn
//!
//! 1. as bound,
//! 2. after every point is bound to another buffer (`glBindBufferBase`),
//! 3. after the first and the last dword of one bound buffer are changed
//!    with `glBufferSubData` and the blocks are mapped to other points.
//!
//! The host cannot render, so a draw is checked the way `es3_ubo` checks it:
//! by the stage's uniform dwords the back end is handed (copied inside the
//! draw call). Every dword of the 12 blocks must be the one of the buffer
//! bound at the block's point at that draw. The last test pins what the
//! submission model does today with the dwords of an earlier draw when a
//! buffer is updated and drawn again: the draw hands over the stage's own
//! uniform storage, which the next draw rewrites in place.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_ubo_rebind_update
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3::*;
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

/// The fragment stage of one draw: its uniform rows (name, dword offset,
/// dwords), a copy of the block's dwords taken inside the draw call, and
/// the address and size the draw handed over.
#[derive(Clone, Default)]
struct Drawn {
    rows: Vec<(String, usize, usize)>,
    block: Vec<u32>,
    cpu: usize,
    dwords: usize,
}

static DRAWN: Mutex<Option<Drawn>> = Mutex::new(None);

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let Some(sh) = d.fs.as_ref() else { return 0 };
    let rows = sh
        .uniform_table()
        .iter()
        .map(|u| {
            let n: Vec<u8> = u.name.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
            (String::from_utf8_lossy(&n).into_owned(), usize::from(u.dword_off), usize::from(u.dwords))
        })
        .collect();
    let dwords = usize::from(d.fs_uniform_dwords);
    let block = if d.fs_uniform_cpu.is_null() {
        Vec::new()
    } else {
        core::slice::from_raw_parts(d.fs_uniform_cpu as *const u32, dwords).to_vec()
    };
    *DRAWN.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(Drawn { rows, block, cpu: d.fs_uniform_cpu as usize, dwords });
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
    name: c"v3d-ubo-rebind".as_ptr() as *const c_char,
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
};

/// One context for the whole file; the tests take turns on it.
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

/* ------------------------------------------------------------------ test -- */

const BLOCKS: usize = 12;
const MEMBERS: usize = 8;
const ENTRIES: usize = 128;
/// Dwords of one block (16384 bytes) and of one member.
const BLOCK_DWORDS: usize = MEMBERS * ENTRIES * 4;
const MEMBER_DWORDS: usize = ENTRIES * 4;

/// Binding points the 12 blocks are mapped to: 1, 3, .. 23 (odd).
fn odd(k: usize) -> usize {
    2 * k + 1
}
/// A second mapping: 22, 20, .. 0 (even, reversed).
fn even_rev(k: usize) -> usize {
    2 * (BLOCKS - 1 - k)
}

/// The dword `d` of buffer `b`, when it was filled at generation `gen`:
/// distinct per buffer, dword and generation.
fn word(b: usize, d: usize, gen: u32) -> u32 {
    (gen << 28) | ((b as u32) << 16) | d as u32
}

fn buffer_data(b: usize, gen: u32) -> Vec<u32> {
    (0..BLOCK_DWORDS).map(|d| word(b, d, gen)).collect()
}

fn program_source() -> String {
    let mut s = String::from("#version 300 es\nprecision highp float;\n");
    for k in 0..BLOCKS {
        s += &format!("layout(std140) uniform B{k} {{ ");
        for m in 0..MEMBERS {
            s += &format!("highp vec4 b{k}m{m}[{ENTRIES}]; ");
        }
        s += "};\n";
    }
    s += "layout(location = 0) out highp vec4 o;\nvoid main (void)\n{\n    highp vec4 s = vec4(0.0);\n";
    for k in 0..BLOCKS {
        // the first and the last dword of the block, and one entry between
        s += &format!("    s += b{k}m0[0] + b{k}m{l}[{e}] + b{k}m3[77];\n", l = MEMBERS - 1, e = ENTRIES - 1);
    }
    s + "    o = s;\n}\n"
}

const VS: &str = "#version 300 es\nin highp vec4 a_position;\nvoid main (void)\n{\n    gl_Position = a_position;\n}\n";

fn shader(ty: GLenum, text: &str) -> GLuint {
    let c = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source and a log buffer.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [c.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        if ok != 1 {
            let mut log = [0 as c_char; 1024];
            glGetShaderInfoLog(s, 1024, core::ptr::null_mut(), log.as_mut_ptr());
            panic!("compile: {}", core::ffi::CStr::from_ptr(log.as_ptr()).to_string_lossy());
        }
        s
    }
}

/// The linked program, its 24 buffers filled at generation 1 (buffer `b`
/// holds `buffer_data(b, 1)`), and the position attribute set up.
struct Rig {
    prog: GLuint,
    bufs: [GLuint; 24],
    /// The generation each buffer holds, as the test last wrote it.
    gen: [u32; 24],
    /// The block index of `B{k}` in the program.
    index: [GLuint; BLOCKS],
}

impl Rig {
    unsafe fn new() -> Rig {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, &program_source()));
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        if ok != 1 {
            let mut log = [0 as c_char; 1024];
            glGetProgramInfoLog(p, 1024, core::ptr::null_mut(), log.as_mut_ptr());
            panic!("link: {}", core::ffi::CStr::from_ptr(log.as_ptr()).to_string_lossy());
        }
        glUseProgram(p);
        let mut index = [0; BLOCKS];
        for (k, i) in index.iter_mut().enumerate() {
            let name = CString::new(format!("B{k}")).unwrap();
            *i = glGetUniformBlockIndex(p, name.as_ptr());
            assert_ne!(*i, GL_INVALID_INDEX, "block B{k}");
            let mut size: GLint = 0;
            glGetActiveUniformBlockiv(p, *i, GL_UNIFORM_BLOCK_DATA_SIZE, &mut size);
            assert_eq!(size as usize, BLOCK_DWORDS * 4, "B{k} data size");
        }
        let mut bufs = [0; 24];
        glGenBuffers(24, bufs.as_mut_ptr());
        let mut rig = Rig { prog: p, bufs, gen: [1; 24], index };
        for (b, &buf) in rig.bufs.iter().enumerate() {
            let d = buffer_data(b, 1);
            glBindBuffer(GL_UNIFORM_BUFFER, buf);
            glBufferData(GL_UNIFORM_BUFFER, (d.len() * 4) as GLsizeiptr, d.as_ptr() as *const c_void, GL_DYNAMIC_DRAW);
        }
        rig.gen = [1; 24];
        assert_eq!(glGetError(), GL_NO_ERROR);
        rig
    }

    /// Map block `B{k}` to point `at(k)`.
    unsafe fn map(&self, at: fn(usize) -> usize) {
        for k in 0..BLOCKS {
            glUniformBlockBinding(self.prog, self.index[k], at(k) as GLuint);
        }
        assert_eq!(glGetError(), GL_NO_ERROR);
    }

    /// Bind buffer `buf(p)` to every point `p`.
    unsafe fn bind(&self, buf: impl Fn(usize) -> usize) {
        for p in 0..24 {
            glBindBufferBase(GL_UNIFORM_BUFFER, p as GLuint, self.bufs[buf(p)]);
        }
        assert_eq!(glGetError(), GL_NO_ERROR);
    }

    /// Change the first and the last dword of buffer `b` to generation `gen`
    /// values, with `glBufferSubData` (the rest keeps its generation).
    unsafe fn update_ends(&mut self, b: usize, gen: u32) {
        glBindBuffer(GL_UNIFORM_BUFFER, self.bufs[b]);
        let first = word(b, 0, gen);
        let last = word(b, BLOCK_DWORDS - 1, gen);
        glBufferSubData(GL_UNIFORM_BUFFER, 0, 4, &first as *const u32 as *const c_void);
        glBufferSubData(
            GL_UNIFORM_BUFFER,
            ((BLOCK_DWORDS - 1) * 4) as GLintptr,
            4,
            &last as *const u32 as *const c_void,
        );
        assert_eq!(glGetError(), GL_NO_ERROR);
    }

    /// One indexed draw; what the back end was handed.
    unsafe fn draw(&self) -> Drawn {
        static POS: [f32; 16] = [-1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0];
        static IDX: [u16; 6] = [0, 1, 2, 2, 1, 3];
        let loc = glGetAttribLocation(self.prog, c"a_position".as_ptr());
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(loc as u32, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(loc as u32);
        *DRAWN.lock().unwrap() = None;
        glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, IDX.as_ptr() as *const c_void);
        glDisableVertexAttribArray(loc as u32);
        assert_eq!(glGetError(), GL_NO_ERROR);
        DRAWN.lock().unwrap().take().expect("the draw reached the back end")
    }

    /// Every dword of every block of `d` is the one of buffer
    /// `buf(point(k))` (at that buffer's generation `gens`), where `point`
    /// maps block `k` to its point.
    fn check(&self, d: &Drawn, point: fn(usize) -> usize, buf: impl Fn(usize) -> usize, gens: &[u32; 24], what: &str) {
        for k in 0..BLOCKS {
            let b = buf(point(k));
            for m in 0..MEMBERS {
                let name = format!("b{k}m{m}");
                let &(_, off, n) = d.rows.iter().find(|r| r.0 == name).unwrap_or_else(|| {
                    panic!("{what}: no row {name} in {:?}", d.rows.iter().map(|r| &r.0).collect::<Vec<_>>())
                });
                assert_eq!(n, MEMBER_DWORDS, "{what}: {name} dwords");
                let got = &d.block[off..off + n];
                for (i, w) in got.iter().enumerate() {
                    let src = m * MEMBER_DWORDS + i;
                    // the ends carry the generation of an update, the rest the one the buffer was filled with
                    let g = if src == 0 || src == BLOCK_DWORDS - 1 { gens[b] } else { 1 };
                    assert_eq!(*w, word(b, src, g), "{what}: block B{k} (point {}, buffer {b}) dword {src}", point(k));
                }
            }
        }
    }
}

#[test]
fn the_block_limits_allow_the_rig() {
    let _g = context();
    // SAFETY: plain GL calls; one value each.
    unsafe {
        let mut v: GLint = 0;
        glGetIntegerv(GL_MAX_UNIFORM_BUFFER_BINDINGS, &mut v);
        assert_eq!(v, 24);
        glGetIntegerv(GL_MAX_UNIFORM_BLOCK_SIZE, &mut v);
        assert_eq!(v, 16384);
        glGetIntegerv(GL_MAX_FRAGMENT_UNIFORM_BLOCKS, &mut v);
        assert_eq!(v, BLOCKS as GLint);
    }
}

/// Twelve blocks at odd points, then every point rebound to another buffer,
/// then the blocks mapped to other points after an update of both ends of one
/// buffer: each draw saw its own values.
#[test]
fn each_draw_sees_the_bindings_and_data_of_its_time() {
    let _g = context();
    // SAFETY: plain GL calls on the one context; the buffers outlive the draws.
    unsafe {
        let mut rig = Rig::new();
        let gens = rig.gen;

        // draw 1: point p holds buffer p
        rig.map(odd);
        rig.bind(|p| p);
        let d1 = rig.draw();
        assert!(d1.dwords >= BLOCKS * BLOCK_DWORDS, "stage dwords {}", d1.dwords);
        rig.check(&d1, odd, |p| p, &gens, "draw 1");

        // draw 2: the same mapping, point p now holds buffer (p + 5) % 24
        rig.bind(|p| (p + 5) % 24);
        let d2 = rig.draw();
        rig.check(&d2, odd, |p| (p + 5) % 24, &gens, "draw 2");

        // draw 3: block 4's buffer (point 9 -> buffer 14) gets new ends, and
        // the blocks move to the even points
        let target = 14;
        rig.update_ends(target, 2);
        rig.gen[target] = 2;
        rig.map(even_rev);
        let d3 = rig.draw();
        rig.check(&d3, even_rev, |p| (p + 5) % 24, &rig.gen, "draw 3");
        // block 5 reads point even_rev(5) = 12, buffer 17, which no update touched
        assert_eq!(even_rev(5), 12);

        // draw 4: back to the odd points; buffer 14 is bound at point 9
        rig.map(odd);
        let d4 = rig.draw();
        rig.check(&d4, odd, |p| (p + 5) % 24, &rig.gen, "draw 4");
        assert_eq!(d4.block[block_start(&d4, 4)], word(target, 0, 2), "block B4 first dword is the update");

        glDeleteBuffers(24, rig.bufs.as_ptr());
        glDeleteProgram(rig.prog);
    }
}

fn block_start(d: &Drawn, k: usize) -> usize {
    d.rows.iter().find(|r| r.0 == format!("b{k}m0")).expect("row").1
}

/// Pin today's behaviour: a buffer updated after a draw does not change the
/// dwords the draw handed over until the next draw rewrites the stage's
/// storage in place. Whatever the submission model guarantees later, a draw
/// that has been handed to the back end must never see a LATER update; this
/// test records the two facts it can see on the host:
/// the copy taken inside the draw is its own (never changed by the update),
/// and the stage's storage is rewritten by the next draw when that draw
/// has the same size (the address is reused).
#[test]
fn a_later_update_does_not_reach_an_earlier_draws_data() {
    let _g = context();
    // SAFETY: plain GL calls on the one context; the memory read after the
    // later draw is the stage's own uniform storage, which stays allocated
    // while the context lives (that it is read at all is what is pinned).
    unsafe {
        let mut rig = Rig::new();
        rig.map(odd);
        rig.bind(|p| p);
        let d1 = rig.draw();
        let before = d1.block.clone();

        // update the first and the last dword of buffer 1 (block 0 reads it)
        rig.update_ends(1, 2);
        rig.gen[1] = 2;
        // 1) no draw yet: the data of the earlier draw is untouched
        assert_eq!(d1.block, before, "the copy of draw 1 changed");
        let live = core::slice::from_raw_parts(d1.cpu as *const u32, d1.dwords);
        assert_eq!(live, &before[..], "the storage of draw 1 changed before the next draw");

        // 2) the next draw sees the update; the earlier draw's copy does not
        let d2 = rig.draw();
        rig.check(&d2, odd, |p| p, &rig.gen, "draw 2");
        assert_eq!(d1.block, before, "the copy of draw 1 changed");
        let (first, last) = (block_start(&d2, 0), block_start(&d2, 0) + BLOCK_DWORDS - 1);
        assert_eq!(d2.block[first], word(1, 0, 2));
        assert_eq!(d2.block[last], word(1, BLOCK_DWORDS - 1, 2));
        assert_eq!(before[first], word(1, 0, 1));
        assert_eq!(before[last], word(1, BLOCK_DWORDS - 1, 1));

        // 3) the storage: pinned as it is today -- the same address, so the
        // earlier draw's dwords are overwritten in place by the later one
        assert_eq!(d2.cpu, d1.cpu, "the stage storage moved between draws (the model changed: update this pin)");
        let live = core::slice::from_raw_parts(d1.cpu as *const u32, d1.dwords);
        assert_eq!(live[first], word(1, 0, 2), "the storage of draw 1 is rewritten in place by draw 2");

        glDeleteBuffers(24, rig.bufs.as_ptr());
        glDeleteProgram(rig.prog);
    }
}
