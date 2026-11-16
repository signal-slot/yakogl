// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glBindBufferRange feeding a full 16384-byte uniform block on the V3D: the
//! range starts at an offset inside a larger buffer, and the QPU reads the
//! first and last dword of the *range*, not of the buffer.
//!
//! The harness is `tests/v3d_ubo_large_sim.rs`'s, with the GL layer in front
//! of it. A fragment shader reads a std140 block of eight `vec4 m[128]`
//! members (16384 bytes) into `gl_FragDepth`. The program is linked and drawn
//! through the GL API on a mock back end that keeps the fragment stage's
//! block as the draw hands it over (`fill_uniform_blocks` in
//! src/gl/program.rs has copied the bound range into it); the QPU
//! interpreter then runs the same shader with a uniform stream built from
//! that block. The buffer's dword `k` holds `k + 0.5` (exact in f32), so the
//! depth names the buffer dword the shader read.
//!
//! Pinned, not specified by ES 3.0 (which leaves the reads of a range shorter
//! than the block undefined): a short range, see `short_range_*`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_ubo_buffer_range
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

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
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target};
use yakogl::shaderc::online::v3d::select::TLB_Z_PER_PIXEL;
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Varying, LANES};
use yakogl::shaderc::online::{lower, sema};

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

/// The fragment stage's block of the last draw, as handed to the back end.
static DRAWN: Mutex<Option<Vec<u32>>> = Mutex::new(None);

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let block = if d.fs_uniform_cpu.is_null() {
        Vec::new()
    } else {
        core::slice::from_raw_parts(d.fs_uniform_cpu as *const u32, usize::from(d.fs_uniform_dwords)).to_vec()
    };
    *DRAWN.lock().unwrap_or_else(|e| e.into_inner()) = Some(block);
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
    name: c"v3d-ubo-range".as_ptr() as *const c_char,
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
    /* as the V3D's table: the GL core hands over the host array */
    uniforms_host_only: true,
    ds_write: None,
    stencil_resolve: None,
};

/// One context for the whole file, and the compiler's one arena: the tests
/// take turns.
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

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/* ---- the shaders and the sim ------------------------------------------------ */

const TARGET: Target = Target { vpm_size: 65536 };
const VS: &str = "#version 300 es\nin highp vec4 a_position;\nvoid main (void)\n{\n    gl_Position = a_position;\n}\n";
/// Members per block and entries per member: 8 x 128 vec4 = 4096 dwords =
/// 16384 bytes (`UniformRow::array_len` is a u8, so no member holds 1024).
const MEMBERS: usize = 8;
const LEN: usize = 128;
const BLOCK_BYTES: usize = MEMBERS * LEN * 16;
const BLOCK_DWORDS: usize = BLOCK_BYTES / 4;
const BUFFER_BYTES: usize = 3 * BLOCK_BYTES;

/// A fragment shader of the blocks `names` (each eight `vec4 <name><m>[128]`
/// members, the name in lower case, so every member name is unique across
/// blocks) writing `depth`.
fn fs(names: &[&str], depth: &str) -> String {
    let blocks: String = names
        .iter()
        .map(|n| {
            let members: String =
                (0..MEMBERS).map(|m| format!("highp vec4 {}{m}[{LEN}]; ", n.to_lowercase())).collect();
            format!("uniform {n} {{ {members}}};\n")
        })
        .collect();
    format!(
        "#version 300 es\n{blocks}layout(location = 0) out mediump vec4 o_color;\nvoid main (void)\n{{\n    o_color = vec4(1.0);\n    gl_FragDepth = {depth};\n}}\n"
    )
}

/// The member read of block dword `dword`, as `<member>[<entry>].<comp>`
/// of the block named in lower case `n`.
fn read(n: &str, dword: usize) -> String {
    let (entry, comp) = (dword / 4, "xyzw".as_bytes()[dword % 4] as char);
    format!("{n}{}[{}].{comp}", entry / LEN, entry % LEN)
}

/// The value of buffer dword `k`.
fn value(k: usize) -> f32 {
    k as f32 + 0.5
}

/// The depth lane 0 writes for `fs` when the QPU reads `block`.
fn sim_depth(fs: &str, block: &[u32]) -> Result<f32, String> {
    let mut mem = vec![0u8; 64 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(VS).expect("arena");
    let fs_ = a.alloc_str(fs).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| e.msg.to_string()).expect("the VS lowers");
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs_).map_err(|e| format!("lower: {}", e.msg))?;
    let p =
        compile_program(&v.module, &f.module, Prim::Triangles, TARGET, &a).map_err(|e| format!("compile: {e:?}"))?;
    let mut stream = Vec::new();
    for u in p.fs.code.uniforms.iter() {
        stream.push(match u.contents {
            0 => u.data,
            1 => *block.get(u.data as usize).ok_or(format!(
                "stream dword {} past the draw's block of {}",
                u.data,
                block.len()
            ))?,
            c => return Err(format!("uniform contents {c} (UNIFA / UBO address) not modelled by this test")),
        });
    }
    let zero = [0u32; LANES];
    let one = [1.0f32.to_bits(); LANES];
    let varyings = p.fs.input_slots.iter().map(|_| Varying { vary: zero, c: zero }).collect();
    let inputs = FsInputs { w: one, w_centroid: one, z: zero, x: zero, y: zero, back_facing: [false; LANES], varyings };
    let env = Env { threads: p.fs.threads, ..Env::default() };
    let o = sim::run(p.fs.code.words, &stream, &Inputs::Fragment(Box::new(inputs)), &env)
        .map_err(|e| format!("sim: {e}"))?;
    let w = o.tlb.iter().find(|w| w.config == Some(TLB_Z_PER_PIXEL)).ok_or("no depth write")?;
    Ok(f32::from_bits(w.value[0].ok_or("undefined depth")?))
}

fn shader(ty: GLenum, src: &str) -> GLuint {
    // SAFETY: plain GL calls; the source outlives them.
    unsafe {
        let s = glCreateShader(ty);
        let c = CString::new(src).unwrap();
        let p = c.as_ptr();
        glShaderSource(s, 1, &p, core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "shader compile\n{src}");
        s
    }
}

/// A linked, current program of `fs_src` with block `names[k]` bound to
/// uniform-buffer binding point `k`.
///
/// # Safety
/// Plain GL calls on the current context.
unsafe fn program(fs_src: &str, names: &[&str]) -> GLuint {
    let (vs, fs) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, fs_src));
    let p = glCreateProgram();
    glAttachShader(p, vs);
    glAttachShader(p, fs);
    glLinkProgram(p);
    glDeleteShader(vs);
    glDeleteShader(fs);
    let mut ok: GLint = 0;
    glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
    assert_eq!(ok, 1, "link\n{fs_src}");
    glUseProgram(p);
    for (k, n) in names.iter().enumerate() {
        let name = CString::new(*n).unwrap();
        let idx = glGetUniformBlockIndex(p, name.as_ptr());
        assert_ne!(idx, GL_INVALID_INDEX, "block {n}");
        let mut size: GLint = 0;
        glGetActiveUniformBlockiv(p, idx, GL_UNIFORM_BLOCK_DATA_SIZE, &mut size);
        assert_eq!(size as usize, BLOCK_BYTES, "GL_UNIFORM_BLOCK_DATA_SIZE of {n}");
        glUniformBlockBinding(p, idx, k as u32);
    }
    assert_eq!(err(), GL_NO_ERROR);
    p
}

/// A buffer of `bytes` bytes whose dword `k` is `value(k)`.
///
/// # Safety
/// Plain GL calls on the current context.
unsafe fn buffer(bytes: usize) -> GLuint {
    let data: Vec<u32> = (0..bytes / 4).map(|k| value(k).to_bits()).collect();
    let mut b = 0;
    glGenBuffers(1, &mut b);
    glBindBuffer(GL_UNIFORM_BUFFER, b);
    glBufferData(GL_UNIFORM_BUFFER, bytes as GLsizeiptr, data.as_ptr() as *const c_void, GL_STATIC_DRAW);
    b
}

/// Binds `size` bytes of `buf` at `offset` to binding point `index`.
///
/// # Safety
/// Plain GL calls on the current context.
unsafe fn bind(index: u32, buf: GLuint, offset: usize, size: usize) {
    glBindBufferRange(GL_UNIFORM_BUFFER, index, buf, offset as GLintptr, size as GLsizeiptr);
    assert_eq!(err(), GL_NO_ERROR, "glBindBufferRange({index}, offset {offset}, size {size})");
}

/// One draw of the current program; the fragment block it handed over.
///
/// # Safety
/// Plain GL calls on the current context.
unsafe fn draw(p: GLuint) -> Vec<u32> {
    static POS: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0];
    let loc = glGetAttribLocation(p, c"a_position".as_ptr());
    glBindBuffer(GL_ARRAY_BUFFER, 0);
    glVertexAttribPointer(loc as u32, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
    glEnableVertexAttribArray(loc as u32);
    *DRAWN.lock().unwrap() = None;
    glDrawArrays(GL_TRIANGLES, 0, 3);
    glDisableVertexAttribArray(loc as u32);
    assert_eq!(err(), GL_NO_ERROR, "the draw");
    DRAWN.lock().unwrap().take().expect("the draw reached the back end")
}

/// GL_UNIFORM_BUFFER_OFFSET_ALIGNMENT (a divisor of the block size).
fn alignment() -> usize {
    let mut a: GLint = 0;
    // SAFETY: plain GL call; one value.
    unsafe { glGetIntegerv(GL_UNIFORM_BUFFER_OFFSET_ALIGNMENT, &mut a) };
    assert!(a > 1 && BLOCK_BYTES.is_multiple_of(a as usize), "alignment {a}");
    a as usize
}

/// `Big`'s block dword `dword` of a range at byte `offset` of a fresh buffer
/// of `BUFFER_BYTES`, as the QPU reads it after a draw.
fn depth_of_range(offset: usize, size: usize, dword: usize) -> f32 {
    let s = fs(&["Big"], &read("big", dword));
    // SAFETY: plain GL calls on handles this case owns.
    unsafe {
        let buf = buffer(BUFFER_BYTES);
        let p = program(&s, &["Big"]);
        bind(0, buf, offset, size);
        let block = draw(p);
        glDeleteProgram(p);
        glDeleteBuffers(1, &buf);
        sim_depth(&s, &block).unwrap_or_else(|why| panic!("offset {offset}, dword {dword}: {why}"))
    }
}

/// The first and last dword of a full-size range at byte `offset`.
fn check_range(offset: usize) {
    for dword in [0, 1, BLOCK_DWORDS / 2, BLOCK_DWORDS - 1] {
        let got = depth_of_range(offset, BLOCK_BYTES, dword);
        let want = value(offset / 4 + dword);
        assert_eq!(got, want, "dword {dword} of the range at byte {offset} (buffer dword {})", offset / 4 + dword);
    }
}

/* ---- (a) one block, a range at an offset ------------------------------------- */

#[test]
fn range_at_offset_zero() {
    let _g = context();
    check_range(0);
}

#[test]
fn range_at_one_alignment_unit() {
    let _g = context();
    check_range(alignment());
}

#[test]
fn range_at_k_alignment_units() {
    let _g = context();
    let a = alignment();
    for k in [2, 3, 7, 64] {
        check_range(k * a);
    }
}

#[test]
fn range_at_one_block_into_the_buffer() {
    let _g = context();
    check_range(BLOCK_BYTES);
}

/// The last range that fits: it ends at the buffer's last byte.
#[test]
fn range_ending_at_the_buffer_end() {
    let _g = context();
    check_range(BUFFER_BYTES - BLOCK_BYTES);
}

/// Dwords on the members' boundaries (each member is 512 dwords) read their
/// own buffer dwords too, behind an offset.
#[test]
fn range_member_boundaries() {
    let _g = context();
    let off = 3 * alignment();
    for dword in [511, 512, 1535, 1536, 2047, 2048, 3583, 3584] {
        assert_eq!(depth_of_range(off, BLOCK_BYTES, dword), value(off / 4 + dword), "dword {dword}");
    }
}

/* ---- (b) two blocks, two ranges of one buffer -------------------------------- */

/// Two blocks of 16384 bytes in one program, `A` and `B`, bound to two ranges
/// of one buffer (at byte offsets `off_a` and `off_b`), each read at its own
/// first and last dword.
fn check_two(off_a: usize, off_b: usize) {
    let last = BLOCK_DWORDS - 1;
    let reads = [("a", 0, off_a), ("a", last, off_a), ("b", 0, off_b), ("b", last, off_b)];
    // SAFETY: plain GL calls on handles this case owns.
    unsafe {
        let buf = buffer(BUFFER_BYTES);
        for (n, dword, off) in reads {
            let s = fs(&["A", "B"], &read(n, dword));
            let p = program(&s, &["A", "B"]);
            bind(0, buf, off_a, BLOCK_BYTES);
            bind(1, buf, off_b, BLOCK_BYTES);
            let block = draw(p);
            let got = sim_depth(&s, &block).unwrap_or_else(|why| panic!("{n} dword {dword}: {why}"));
            assert_eq!(got, value(off / 4 + dword), "block {n} dword {dword} with A at {off_a}, B at {off_b}");
            glDeleteProgram(p);
        }
        glDeleteBuffers(1, &buf);
    }
}

#[test]
fn two_blocks_two_ranges_of_one_buffer() {
    let _g = context();
    check_two(0, BLOCK_BYTES);
    check_two(BLOCK_BYTES, 2 * BLOCK_BYTES);
    check_two(2 * BLOCK_BYTES, 0);
}

/// Ranges that overlap: both blocks see the buffer's own dwords.
#[test]
fn two_blocks_overlapping_ranges() {
    let _g = context();
    let a = alignment();
    check_two(a, 5 * a);
}

/// A vertex-free sanity check of the harness itself: one block, two programs
/// and two bindings of the same buffer give different dwords (the offset is
/// what the depth follows, not the buffer).
#[test]
fn rebinding_moves_the_range() {
    let _g = context();
    let a = alignment();
    let s = fs(&["Big"], &read("big", 0));
    // SAFETY: plain GL calls on handles this case owns.
    unsafe {
        let buf = buffer(BUFFER_BYTES);
        let p = program(&s, &["Big"]);
        for off in [0, a, BLOCK_BYTES, 3 * a] {
            bind(0, buf, off, BLOCK_BYTES);
            let block = draw(p);
            assert_eq!(sim_depth(&s, &block).unwrap(), value(off / 4), "offset {off}");
        }
        glDeleteProgram(p);
        glDeleteBuffers(1, &buf);
    }
}

/* ---- (c) a range shorter than the block -------------------------------------- */

/// A range of 8192 bytes for a 16384-byte block. ES 3.0 leaves what the
/// shader reads past the range undefined; it forbids a crash.
///
/// What this driver does: no error at bind or draw, and the draw goes ahead.
/// `fill_uniform_blocks` (src/gl/program.rs:754) clamps the range to the
/// buffer, and `copy_block_member` (src/gl/program.rs:810, `let Some(v) =
/// read(at) else { continue }`) skips every dword the range does not hold, so
/// the program's array keeps whatever it held: zero on a program that never
/// drew a larger range. The dwords inside the range are exact.
#[test]
fn short_range_zero_fills_the_tail_of_a_fresh_program() {
    let _g = context();
    let (a, half) = (alignment(), BLOCK_BYTES / 2);
    let off = 3 * a;
    let half_dw = half / 4;
    assert_eq!(depth_of_range(off, half, half_dw - 1), value(off / 4 + half_dw - 1), "the range's last dword");
    assert_eq!(depth_of_range(off, half, 0), value(off / 4), "the range's first dword");
    assert_eq!(depth_of_range(off, half, half_dw), 0.0, "the first dword past the range");
    assert_eq!(depth_of_range(off, half, BLOCK_DWORDS - 1), 0.0, "the block's last dword");
}

/// The tail of a short range is the program's previous contents, not a
/// zero fill: a full range drawn first leaves its values behind a later
/// short one (the same two lines as above). Undefined by the GL, so allowed;
/// pinned so a change to a zero fill is seen.
#[test]
fn short_range_keeps_the_previous_draws_tail() {
    let _g = context();
    let off = alignment();
    let s = fs(&["Big"], &read("big", BLOCK_DWORDS - 1));
    // SAFETY: plain GL calls on handles this case owns.
    unsafe {
        let buf = buffer(BUFFER_BYTES);
        let p = program(&s, &["Big"]);
        bind(0, buf, BLOCK_BYTES, BLOCK_BYTES);
        let full = draw(p);
        assert_eq!(sim_depth(&s, &full).unwrap(), value(BLOCK_DWORDS + BLOCK_DWORDS - 1));
        bind(0, buf, off, BLOCK_BYTES / 2);
        let short = draw(p);
        assert_eq!(
            sim_depth(&s, &short).unwrap(),
            value(BLOCK_DWORDS + BLOCK_DWORDS - 1),
            "the tail is the earlier draw's"
        );
        glDeleteProgram(p);
        glDeleteBuffers(1, &buf);
    }
}

/// A range that runs past the end of the buffer is clamped the same way
/// (src/gl/program.rs:753-754): no error, the part inside the buffer is read
/// and the rest is the program's previous contents (zero here).
#[test]
fn range_past_the_buffer_end_is_clamped() {
    let _g = context();
    let off = BUFFER_BYTES - BLOCK_BYTES / 2;
    assert_eq!(depth_of_range(off, BLOCK_BYTES, 0), value(off / 4));
    assert_eq!(depth_of_range(off, BLOCK_BYTES, BLOCK_DWORDS / 2 - 1), value(BUFFER_BYTES / 4 - 1));
    assert_eq!(depth_of_range(off, BLOCK_BYTES, BLOCK_DWORDS / 2), 0.0);
}

/* ---- (d) an unaligned offset -------------------------------------------------- */

#[test]
fn unaligned_offset_is_invalid_value_and_binds_nothing() {
    let _g = context();
    let a = alignment();
    // SAFETY: plain GL calls on handles this case owns.
    unsafe {
        let buf = buffer(BUFFER_BYTES);
        bind(2, buf, a, BLOCK_BYTES);
        for off in [1, a - 1, a + 1, 2 * a + a / 2, BLOCK_BYTES + 1] {
            glBindBufferRange(GL_UNIFORM_BUFFER, 2, buf, off as GLintptr, BLOCK_BYTES as GLsizeiptr);
            assert_eq!(err(), GL_INVALID_VALUE, "offset {off} (alignment {a})");
            /* the earlier binding stands */
            let (mut start, mut size): (GLint64, GLint64) = (-1, -1);
            glGetInteger64i_v(GL_UNIFORM_BUFFER_START, 2, &mut start);
            glGetInteger64i_v(GL_UNIFORM_BUFFER_SIZE, 2, &mut size);
            assert_eq!((start as usize, size as usize), (a, BLOCK_BYTES), "binding changed by the offset {off}");
        }
        /* the other invalid arguments of ES 3.0 2.10.1.1 */
        glBindBufferRange(GL_UNIFORM_BUFFER, 2, buf, -(a as GLintptr), BLOCK_BYTES as GLsizeiptr);
        assert_eq!(err(), GL_INVALID_VALUE, "negative offset");
        glBindBufferRange(GL_UNIFORM_BUFFER, 2, buf, 0, 0);
        assert_eq!(err(), GL_INVALID_VALUE, "size 0");
        let mut n: GLint = 0;
        glGetIntegerv(GL_MAX_UNIFORM_BUFFER_BINDINGS, &mut n);
        glBindBufferRange(GL_UNIFORM_BUFFER, n as GLuint, buf, 0, 4);
        assert_eq!(err(), GL_INVALID_VALUE, "index at GL_MAX_UNIFORM_BUFFER_BINDINGS");
        glDeleteBuffers(1, &buf);
    }
}

/// A refused offset leaves the draw reading the earlier binding.
#[test]
fn draw_after_a_refused_offset_reads_the_earlier_range() {
    let _g = context();
    let a = alignment();
    let s = fs(&["Big"], &read("big", BLOCK_DWORDS - 1));
    // SAFETY: plain GL calls on handles this case owns.
    unsafe {
        let buf = buffer(BUFFER_BYTES);
        let p = program(&s, &["Big"]);
        bind(0, buf, 2 * a, BLOCK_BYTES);
        glBindBufferRange(GL_UNIFORM_BUFFER, 0, buf, (3 * a + 1) as GLintptr, BLOCK_BYTES as GLsizeiptr);
        assert_eq!(err(), GL_INVALID_VALUE);
        let block = draw(p);
        assert_eq!(sim_depth(&s, &block).unwrap(), value(2 * a / 4 + BLOCK_DWORDS - 1));
        glDeleteProgram(p);
        glDeleteBuffers(1, &buf);
    }
}
