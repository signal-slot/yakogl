// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): where the two G32R32 planes of a 128-bit texture sit
//! for 1x1, 1xN and Nx1, odd NPOT sizes, a full NPOT mip chain, widths the
//! 4x4 tile pads, and the largest size the back end advertises
//! (docs/notes/gc-128bit-texture-sampling.md, docs/notes/gc-npot-repeat.md,
//! docs/notes/gc-tiled-targets.md; Mesa 26.2.1 etnaviv).
//!
//! A level's plane is `align4(w) * align4(h) * 8` bytes (4x4 tiles of 8-byte
//! G32R32 texels), plane 1 starts one plane into the level, a level starts on
//! 64 bytes, and the descriptor's stride is `align4(w) * 8`. The padding of a
//! tile a level does not fill is not specified; only real texels are read.
//!
//! - The tests that always run read the layout out of the public API
//!   (`tiled_layout`, `second_plane_offset`, `copy_bpp`) for every size,
//!   including GL_MAX_TEXTURE_SIZE (read from `glGetIntegerv`): there only
//!   the address arithmetic is checked, in u64 against the u32 the
//!   descriptor holds, with no allocation (a 4096x4096 RGBA32F level 0 is
//!   256 MB of texels, plus the same again as the upload buffer).
//! - The draw tests go through the GL API on the stand-in OSAL
//!   (tests/es3_wide128_layers_gc.rs): upload, draw, read both planes'
//!   descriptors, both planes' sampler words (REPEAT on S and T) and every
//!   texel of both planes out of the copy. The largest size is drawn as
//!   Nx1 and 1xN (`GL_MAX_TEXTURE_SIZE` texels, a few hundred KB), which
//!   reaches the same plane arithmetic without a gigabyte allocation. The
//!   sampled mask hook (`backend::TEST_EXTRA_SAMPLED`) is `#[cfg(test)]`, so
//!   no integration test can add the classes: until GC_SAMPLED_TEXFMTS holds
//!   them (item gc-w128-flip) the draw tests find them closed, say so on
//!   stderr and check the closed state; from the flip on they run for real,
//!   the state being found at run time.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_sizes_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::*;
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
use yakogl::vivante::texture::wide128_sampled;
use yakogl::vivante::tiled_copy::{copy_bpp, second_plane_offset, tiled_layout};

const COMMON_3D: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/driver/src/vivante/hw/common_3d.xml.h"));

/// The value of `#define <name>` in the vendored common_3d.xml.h.
fn rnndb(name: &str) -> u32 {
    let line = COMMON_3D
        .lines()
        .find(|l| l.starts_with("#define") && l.split_whitespace().nth(1) == Some(name))
        .unwrap_or_else(|| panic!("{name} is in common_3d.xml.h"));
    let v = line.split_whitespace().nth(2).expect("a value");
    u32::from_str_radix(v.trim_start_matches("0x"), 16).expect("hex")
}

const WIDE: [(&str, u8, u32, &str); 3] = [
    ("RGBA32F", GLES_TEXFMT_RGBA32F, GL_RGBA32F, "TEXTURE_FORMAT_EXT_G32R32F"),
    ("RGBA32UI", GLES_TEXFMT_RGBA32UI, GL_RGBA32UI, "TEXTURE_FORMAT_EXT_G32R32I"),
    ("RGBA32I", GLES_TEXFMT_RGBA32I, GL_RGBA32I, "TEXTURE_FORMAT_EXT_G32R32I"),
];

/// Is the class in the back end's sampled mask (so the GL core stores it)?
fn open(format: u8) -> bool {
    (sampled_texfmts() as u64) & (gles_texfmt_bit(format) as u64) != 0
}

fn align4(v: u32) -> u32 {
    (v + 3) & !3
}

fn level_dims(w: u32, h: u32, l: u32) -> (u32, u32) {
    ((w >> l).max(1), (h >> l).max(1))
}

/* ---- the stand-in OSAL (as tests/gc_tiled_copy_reuse.rs) ---------------- */

struct Alloc {
    mem: Box<[u64]>,
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

/// `n` bytes of the stand-in's memory at GPU address `gpu`.
fn read_gpu(gpu: u64, n: usize) -> Vec<u8> {
    with_platform(|p| {
        let (base, a) = p.allocs.range(..=gpu).next_back().unwrap_or_else(|| panic!("no allocation at 0x{gpu:x}"));
        let off = (gpu - base) as usize;
        assert!(off + n <= a.size, "0x{gpu:x}+{n} runs off the allocation at 0x{base:x} ({} bytes)", a.size);
        // SAFETY: inside the allocation (checked just above).
        unsafe { core::slice::from_raw_parts((a.mem.as_ptr() as *const u8).add(off), n) }.to_vec()
    })
}

fn words_at(gpu: u64, n: usize) -> Vec<u32> {
    let b = read_gpu(gpu, n * 4);
    let (words, _) = b.as_chunks::<4>();
    words.iter().map(|c| u32::from_le_bytes(*c)).collect()
}

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

/// Run `case` with the Vivante back end up on the stand-in OSAL and a fresh
/// 64x64 context.
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
        /* leaked, not freed, as in gc_npot_repeat.rs: addresses keep counting up */
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

/* ---- GL plumbing -------------------------------------------------------- */

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

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// The words of slot `slot`'s descriptor in the last kick, if it wrote one.
fn descriptor(stream: &[(u32, u32)], slot: u32) -> Option<Vec<u32>> {
    let a = stream.iter().rev().find(|(r, _)| *r == nte_descriptor_addr(slot))?.1;
    (a != 0).then(|| words_at(u64::from(a), GC_TEXDESC_BYTES / 4))
}

/// One quad sampling unit 0 with program `p`: the last kick's writes.
fn draw(p: GLuint) -> Vec<(u32, u32)> {
    let kicks0 = with_platform(|p| p.kicks.len());
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 64, 64);
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
    }
    no_error("draw");
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    no_error("glFinish");
    let stream = with_platform(|p| {
        assert!(p.kicks.len() > kicks0, "the quad produced no kick");
        p.kicks.last().cloned().unwrap()
    });
    writes(&stream)
}

/// GL_MAX_TEXTURE_SIZE as this back end's context reports it.
fn max_texture_size() -> GLint {
    let mut max: GLint = 0;
    on_gc7000(|| {
        // SAFETY: a plain GL query into a live local.
        unsafe { glGetIntegerv(GL_MAX_TEXTURE_SIZE, &mut max) };
        no_error("glGetIntegerv");
    });
    max
}

/* ---- the layouts under test --------------------------------------------- */

/// Levels of a full chain: `floor(log2(max(w, h))) + 1`.
fn levels_of(w: u32, h: u32) -> u32 {
    32 - w.max(h).leading_zeros()
}

fn bind(class: u8, w: u32, h: u32, levels: u32) -> GlesTextureBind {
    GlesTextureBind {
        format: class,
        type_: GLES_TEXTYPE_2D,
        width: w,
        height: h,
        layers: 1,
        mip_levels: levels as _,
        ..Default::default()
    }
}

/// Sizes of the item: 1x1, 1xN, Nx1, odd NPOT, widths the tile pads
/// (not a multiple of 4), multiples of 4 as the control, and non-square
/// ones.
const SIZES: [(u32, u32); 17] = [
    (1, 1),
    (1, 2),
    (1, 9),
    (2, 1),
    (33, 1),
    (3, 5),
    (5, 3),
    (17, 9),
    (9, 17),
    (5, 4),
    (6, 4),
    (7, 4),
    (4, 4),
    (8, 3),
    (13, 6),
    (127, 65),
    (64, 1),
];

/// The padded plane layout of `w`x`h`, in u64 so that no step wraps: per level
/// `(offset of plane 0, size of one plane)`, and the end of the copy.
fn layout64(w: u32, h: u32, levels: u32) -> (Vec<(u64, u64)>, u64) {
    let mut at = 0u64;
    let mut out = Vec::new();
    for l in 0..levels {
        at = (at + 63) & !63;
        let (lw, lh) = level_dims(w, h, l);
        let plane = u64::from(align4(lw)) * u64::from(align4(lh)) * 8;
        out.push((at, plane));
        at += 2 * plane;
    }
    (out, at)
}

fn the_layout_of(w: u32, h: u32, levels: u32, class: &(&str, u8, u32, &str)) {
    let t = bind(class.1, w, h, levels);
    let what = format!("{} {w}x{h}", class.0);
    assert_eq!(copy_bpp(&t), 8, "{what}: a plane's texel");
    let (off, total) = tiled_layout(w, h, 1, levels, 16);
    let (want, end64) = layout64(w, h, levels);
    assert!(end64 <= u64::from(u32::MAX), "{what}: the copy ({end64} bytes) fits the 32-bit addresses");
    assert_eq!(total as u64, end64, "{what}: the copy ends after the last level's two planes");
    for l in 0..levels {
        let (lw, lh) = level_dims(w, h, l);
        let (at, plane) = want[l as usize];
        assert_eq!(off[l as usize] as u64, at, "{what} level {l}: offset");
        assert_eq!(off[l as usize] % GC_TEXDESC_LOD_ALIGN, 0, "{what} level {l}: 64-byte level");
        assert_eq!(second_plane_offset(&t, l) as u64, plane, "{what} level {l}: plane 1 is one plane in");
        assert_eq!(plane % 128, 0, "{what} level {l}: a plane is whole 4x4 tiles of 8-byte texels");
        assert!(plane >= u64::from(lw) * u64::from(lh) * 8, "{what} level {l}: the plane holds the texels");
        /* plane 1 and the end of the level are inside the allocation */
        let p1 = at + plane;
        assert!(p1 < total as u64 && p1 + plane <= total as u64, "{what} level {l}: plane 1 is inside the copy");
        if l + 1 < levels {
            assert!(off[l as usize + 1] as u64 >= p1 + plane, "{what} level {l}: before the next level");
        }
    }
}

/// 1x1, 1xN, Nx1, odd and tile-padded sizes, each alone and with its full
/// chain down to 1x1, for the three classes: plane 1 is one padded plane into
/// the level at every level, levels are 64-byte aligned and do not overlap.
#[test]
fn the_planes_of_odd_and_padded_sizes_sit_one_plane_apart() {
    for class in &WIDE {
        for (w, h) in SIZES {
            the_layout_of(w, h, 1, class);
            the_layout_of(w, h, levels_of(w, h), class);
        }
    }
}

/// A 1x1 level is a whole 4x4 tile per plane (128 bytes): the levels from 4x4
/// down are all 128 bytes.
#[test]
fn the_levels_below_a_tile_pad_to_one_tile_per_plane() {
    let t = bind(GLES_TEXFMT_RGBA32F, 17, 9, levels_of(17, 9));
    let planes: Vec<usize> = (0..levels_of(17, 9)).map(|l| second_plane_offset(&t, l)).collect();
    /* 17x9 -> 20x12; 8x4 -> 8x4; 4x2, 2x1 and 1x1 -> 4x4 */
    assert_eq!(planes, [20 * 12 * 8, 8 * 4 * 8, 128, 128, 128]);
}

/// The tiled stride the descriptor takes, per width: the row is `align4(w)`
/// texels of 8 bytes.
#[test]
fn the_stride_is_the_padded_width_in_plane_texels() {
    for (w, want) in
        [(1u32, 32u32), (2, 32), (3, 32), (4, 32), (5, 64), (6, 64), (7, 64), (9, 96), (17, 160), (33, 288)]
    {
        assert_eq!(align4(w) * 8, want, "width {w}");
    }
}

/// GL_MAX_TEXTURE_SIZE as the back end reports it, as the largest square
/// texture and the Nx1 and 1xN ones: only address arithmetic, nothing is
/// allocated. No step overflows u32 and plane 1's offset and end are inside
/// the copy. (A 4096x4096 RGBA32F level 0 is 256 MB of texels, 128 MB per
/// plane; the 13-level chain is a bit under 358 MB.)
#[test]
fn the_largest_size_does_not_overflow_the_plane_arithmetic() {
    let max = max_texture_size();
    assert!(max >= 2048, "GL_MAX_TEXTURE_SIZE {max}");
    let m = max as u32;
    for class in &WIDE {
        for (w, h) in [(m, m), (m, 1), (1, m), (m, m - 1), (m - 1, m)] {
            let levels = levels_of(w, h);
            let t = bind(class.1, w, h, levels);
            let (want, end64) = layout64(w, h, levels);
            let (off, total) = tiled_layout(w, h, 1, levels, 16);
            let what = format!("{} {w}x{h}", class.0);
            assert_eq!(total as u64, end64, "{what}: the copy's size");
            assert!(end64 <= u64::from(u32::MAX), "{what}: {end64} bytes fit a 32-bit address");
            for l in 0..levels {
                let (at, plane) = want[l as usize];
                assert_eq!(off[l as usize] as u64, at, "{what} level {l}");
                let p1 = second_plane_offset(&t, l) as u64;
                assert_eq!(p1, plane, "{what} level {l}");
                assert!(at + p1 + plane <= end64, "{what} level {l}: plane 1 ends inside the copy");
                assert!(at + p1 <= u64::from(u32::MAX), "{what} level {l}: plane 1's address");
            }
        }
        /* the square one: level 0 alone is two planes of align4(max)^2 * 8 */
        let plane = u64::from(align4(m)) * u64::from(align4(m)) * 8;
        let t = bind(class.1, m, m, 1);
        assert_eq!(second_plane_offset(&t, 0) as u64, plane, "{}: the square plane", class.0);
        assert_eq!(tiled_layout(m, m, 1, 1, 16).1 as u64, 2 * plane, "{}: the square copy", class.0);
    }
}

/* ---- the draws ---------------------------------------------------------- */

fn program(sampler: &str) -> GLuint {
    let vs = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
    let fs = format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\nprecision highp {sampler};\nuniform {sampler} s;\nout vec4 o;\nvoid main() {{ o = vec4(texelFetch(s, ivec2(0), 0)); }}\n"
    );
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, &fs));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        p
    }
}

fn sampler_of(class: u8) -> &'static str {
    match class {
        GLES_TEXFMT_RGBA32F => "sampler2D",
        GLES_TEXFMT_RGBA32I => "isampler2D",
        _ => "usampler2D",
    }
}

/// The four words of texel (x, y) of `level`: a hash, the finite floats
/// [1, 2) for RGBA32F, any word otherwise.
fn texel(float: bool, level: u32, x: u32, y: u32) -> [u32; 4] {
    let mut out = [0u32; 4];
    for (c, o) in out.iter_mut().enumerate() {
        let mut h = (level + 1).wrapping_mul(0x9e37_79b1)
            ^ (x + 1).wrapping_mul(0x85eb_ca6b)
            ^ (y + 1).wrapping_mul(0xc2b2_ae35)
            ^ (c as u32 + 1).wrapping_mul(0x27d4_eb2f);
        h ^= h >> 15;
        h = h.wrapping_mul(0x2c1b_3c6d);
        h ^= h >> 12;
        *o = if float { 0x3f80_0000 | (h & 0x007f_ffff) } else { h };
    }
    out
}

/// A `w`x`h` texture of `class`, `levels` levels, NEAREST, REPEAT on S and T.
fn upload(class: &(&str, u8, u32, &str), w: u32, h: u32, levels: u32) {
    let float = class.1 == GLES_TEXFMT_RGBA32F;
    let (ty, fmt) = match class.1 {
        GLES_TEXFMT_RGBA32F => (GL_FLOAT, GL_RGBA),
        GLES_TEXFMT_RGBA32I => (GL_INT, GL_RGBA_INTEGER),
        _ => (GL_UNSIGNED_INT, GL_RGBA_INTEGER),
    };
    // SAFETY: plain GL calls; each image holds the `lw * lh` texels of its level.
    unsafe {
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        let mut t: GLuint = 0;
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        let min = if levels > 1 { GL_NEAREST_MIPMAP_NEAREST } else { GL_NEAREST };
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, min as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_REPEAT as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_REPEAT as GLint);
        for l in 0..levels {
            let (lw, lh) = level_dims(w, h, l);
            let mut img = Vec::with_capacity((lw * lh * 16) as usize);
            for y in 0..lh {
                for x in 0..lw {
                    img.extend(texel(float, l, x, y).iter().flat_map(|v| v.to_le_bytes()));
                }
            }
            glTexImage2D(
                GL_TEXTURE_2D,
                l as GLint,
                class.2 as GLint,
                lw as GLsizei,
                lh as GLsizei,
                0,
                fmt,
                ty,
                img.as_ptr().cast(),
            );
        }
    }
    no_error("upload");
}

/// The last value `stream` wrote to `reg`.
fn last(stream: &[(u32, u32)], reg: u32) -> u32 {
    stream.iter().rev().find(|(r, _)| *r == reg).unwrap_or_else(|| panic!("no write of register 0x{reg:x}")).1
}

/// SAMP_CTRL0's UWRAP, VWRAP and WWRAP fields (bits 0-8), and their values
/// (tests/gc_npot_repeat.rs).
const WRAP_BITS: u32 = 0x1ff;
const WRAP_REPEAT: u32 = 0;
const WRAP_CLAMP: u32 = 2;

/// The descriptor pair, the sampler words and the texels of both planes of one
/// draw against the model: every real texel of every level.
fn check(class: &(&str, u8, u32, &str), (w, h): (u32, u32), levels: u32, stream: &[(u32, u32)]) {
    let what = format!("{} {w}x{h} ({levels} levels)", class.0);
    let float = class.1 == GLES_TEXFMT_RGBA32F;
    let d0 = descriptor(stream, 0).unwrap_or_else(|| panic!("{what}: no descriptor in slot 0"));
    let d1 = descriptor(stream, 1).unwrap_or_else(|| panic!("{what}: no companion descriptor in slot 1"));
    for (p, d) in [&d0, &d1].into_iter().enumerate() {
        assert_eq!(
            d[GC_TEXDESC_CONFIG0 as usize / 4] & (0x7 | (3 << 20)),
            texdesc_type(GC_TEX_TYPE_2D) | texdesc_addressing_mode(GC_TEX_ADDRESSING_TILED),
            "{what}: plane {p}: 2D, tiled"
        );
        assert_eq!(d[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f, rnndb(class.3), "{what}: plane {p}: {}", class.3);
        assert_eq!(
            d[GC_TEXDESC_SIZE as usize / 4],
            texdesc_size_width(w) | texdesc_size_height(h),
            "{what}: plane {p}: the real size, not the padded one"
        );
        assert_eq!(d[GC_TEXDESC_LINEAR_STRIDE as usize / 4], align4(w) * 8, "{what}: plane {p}: stride");
        assert_eq!(d[GC_TEXDESC_SLICE as usize / 4], align4(w) * align4(h) * 8, "{what}: plane {p}: slice");
        assert_eq!(
            d[GC_TEXDESC_BASELOD as usize / 4],
            texdesc_baselod_base(0) | texdesc_baselod_max(levels - 1),
            "{what}: plane {p}: the level count"
        );
    }
    /* everything but the sixteen level addresses is the same in both */
    for i in 16..d0.len() {
        assert_eq!(d0[i], d1[i], "{what}: descriptor word {i}: the companion is a copy");
    }
    /* REPEAT on both planes' sampler, and the rest of the sampler words equal */
    /* an axis one texel long clamps (equivalent, and the only safe thing:
     * texture.rs); W of a 2D lookup follows U */
    let wrap = |one_texel: bool| if one_texel { WRAP_CLAMP } else { WRAP_REPEAT };
    let (u, v) = (wrap(w <= 1), wrap(h <= 1));
    for slot in 0..2 {
        assert_eq!(
            last(stream, nte_descriptor_samp_ctrl0(slot)) & WRAP_BITS,
            u | (v << 3) | (u << 6),
            "{what}: slot {slot}: SAMP_CTRL0 UWRAP/VWRAP/WWRAP: REPEAT, but CLAMP on a 1-texel axis"
        );
    }
    for reg in [
        nte_descriptor_samp_ctrl0,
        nte_descriptor_samp_ctrl1,
        nte_descriptor_samp_lod_minmax,
        nte_descriptor_samp_lod_bias,
    ] {
        assert_eq!(last(stream, reg(0)), last(stream, reg(1)), "{what}: sampler register 0x{:x}", reg(0));
    }
    assert_eq!(d0[0] as u64 % 4096, 0, "{what}: the copy starts on a page");
    for l in 0..16u32 {
        let (a0, a1) = (d0[texdesc_lod_addr(l) as usize / 4], d1[texdesc_lod_addr(l) as usize / 4]);
        if l >= levels {
            assert_eq!(a0, a1, "{what}: level {l} is unused in both");
            continue;
        }
        let (lw, lh) = level_dims(w, h, l);
        let plane = align4(lw) * align4(lh) * 8;
        assert_eq!(a0 as usize % GC_TEXDESC_LOD_ALIGN, 0, "{what}: level {l}: 64-byte level");
        assert_eq!(a1, a0 + plane, "{what}: level {l}: plane 1 one padded plane into the level");
        if l + 1 < levels {
            let next = d0[texdesc_lod_addr(l + 1) as usize / 4];
            assert!(next >= a0 + 2 * plane, "{what}: level {l}: two planes before level {}", l + 1);
        }
        let (p0, p1) = (words_at(u64::from(a0), plane as usize / 4), words_at(u64::from(a1), plane as usize / 4));
        for y in 0..lh {
            for x in 0..lw {
                let tile = (y / 4) * (align4(lw) / 4) + x / 4;
                let at = (tile * 16 + (y % 4) * 4 + x % 4) as usize * 2;
                let want = texel(float, l, x, y);
                assert_eq!([p0[at], p0[at + 1]], want[..2], "{what}: level {l}: R,G of ({x}, {y}) in plane 0");
                assert_eq!([p1[at], p1[at + 1]], want[2..], "{what}: level {l}: B,A of ({x}, {y}) in plane 1");
            }
        }
    }
}

/// One size, one class: the draw and its checks, or the closed state while
/// the class is not in the sampled mask.
fn draw_size(class: &(&str, u8, u32, &str), w: u32, h: u32, levels: u32) {
    if !open(class.1) {
        eprintln!(
            "{} {w}x{h}: not in GC_SAMPLED_TEXFMTS; the plane addresses are not exercised (gc-w128-flip)",
            class.0
        );
        assert!(!wide128_sampled(&bind(class.1, w, h, levels)));
        return;
    }
    on_gc7000(|| {
        upload(class, w, h, levels);
        let stream = draw(program(sampler_of(class.1)));
        let d = descriptor(&stream, 0).unwrap_or_else(|| panic!("{} {w}x{h}: no descriptor in slot 0", class.0));
        if d[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f != rnndb(class.3) {
            eprintln!("{} {w}x{h}: the GL core does not store the class as planes; not checked", class.0);
            return;
        }
        check(class, (w, h), levels, &stream);
    });
}

/// Every size of the list with its full chain, RGBA32F, REPEAT on S and T:
/// 1x1, 1xN, Nx1, odd NPOT, tile-padded widths. Both planes' descriptor
/// words, sampler words and texels.
#[test]
fn rgba32f_of_odd_sizes_has_two_planes_and_repeat_on_both() {
    for (w, h) in SIZES {
        draw_size(&WIDE[0], w, h, levels_of(w, h));
    }
}

/// The integer classes on sizes that pad, are 1 wide, or are NPOT.
#[test]
fn rgba32ui_and_rgba32i_of_odd_sizes() {
    for (w, h) in [(1u32, 1u32), (33, 1), (1, 9), (3, 5), (17, 9), (5, 4), (127, 65)] {
        draw_size(&WIDE[1], w, h, levels_of(w, h));
        draw_size(&WIDE[2], w, h, levels_of(w, h));
    }
}

/// A single level of an NPOT texture (no chain): plane 1 starts at the
/// padded plane, not at the texel count.
#[test]
fn a_single_level_npot_texture_pads_plane_zero() {
    for (w, h) in [(3u32, 5u32), (17, 9), (33, 1), (127, 65)] {
        draw_size(&WIDE[0], w, h, 1);
    }
}

/// The largest extent as Nx1 and 1xN (GL_MAX_TEXTURE_SIZE texels, with the
/// full chain). The square size would be 128 MB per plane and is the
/// arithmetic test's: nothing here allocates more than a few hundred KB.
#[test]
fn the_largest_extent_in_one_dimension_is_two_planes() {
    let m = max_texture_size() as u32;
    draw_size(&WIDE[0], m, 1, levels_of(m, 1));
    draw_size(&WIDE[0], 1, m, levels_of(1, m));
    draw_size(&WIDE[1], m - 1, 1, 1);
}

/// The class is closed or open as a whole: all three agree, so that a flip of
/// one class alone shows here.
#[test]
fn the_state_found_is_all_or_nothing() {
    let n = WIDE.iter().filter(|c| open(c.1)).count();
    eprintln!("RGBA32 classes in GC_SAMPLED_TEXFMTS: {n} of 3 ({})", if n == 0 { "closed" } else { "open" });
    assert!(n == 0 || n == 3, "{n} of the three 128-bit classes are sampled");
}
