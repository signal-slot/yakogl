// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The PowerVR's native 3D texture (`pvr_native_3d`, `gl::native_vol`) and
//! the ES 3.0 level and LOD parameters (dEQP-GLES3 texture.mipmap.3d
//! base_level, max_level, min_lod, max_lod): the copy is the volume as from
//! GL_TEXTURE_BASE_LEVEL (`native_vol::view`), its descriptor is the base
//! level's size with the levels counted from it and capped by
//! GL_TEXTURE_MAX_LEVEL, and the sampler clamps the LOD to GL_TEXTURE_MIN_LOD
//! and GL_TEXTURE_MAX_LOD (ES 3.0 3.8.9, 3.8.10). A frame that draws one
//! volume from several base levels binds a copy for each, and a copy a kick
//! may still read is never written again. The host OSAL renders nothing:
//! these are the computed copies and state words, not pictures.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_3d_lod
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use core::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicU32, Ordering};
use yakogl::backend::{
    self, GlesDraw, GlesHeap, GlesTextureBind, YakoglBackend, GLES_TEXFMT_RGBA8, GLES_TEXTYPE_2D,
    GLES_TEXTYPE_3D_TWIDDLED, GLES_WRAP_REPEAT,
};
use yakogl::gl::consts::*;
use yakogl::gl::context::{self, GlTexture};
use yakogl::gl::names::{self, Kind};
use yakogl::gl::native_vol::{self, copy_bytes, level_texel_off, physical_extent, twiddle3d, twiddle_extent, View};
use yakogl::gl::state::{glGetError, glPixelStorei};
use yakogl::gl::texture::{
    glBindTexture, glDeleteTextures, glGenTextures, glTexParameterf, glTexParameteri, glTexStorage3D, glTexSubImage3D,
};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::pvr::backend::texture_descriptor;

/* ---- a GL context on the host: host memory, and a stand-in back end with
 * the PowerVR's texture layout (a 3D texture's slices in a grid) and the
 * PowerVR's kick numbering, which the test moves by hand ---- */

/// The last kick submitted; the one being recorded is the next
/// (`pvr_backend_kick_id`).
static LAST_FENCE: AtomicU32 = AtomicU32::new(0);
/// The last kick completed.
static COMPLETED: AtomicU32 = AtomicU32::new(0);
static FLUSHES: AtomicU32 = AtomicU32::new(0);
static FINISHES: AtomicU32 = AtomicU32::new(0);

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
unsafe extern "C" fn b_draw(_d: *const GlesDraw) -> i32 {
    0
}
/// Submits the open frame.
unsafe extern "C" fn b_flush() -> i32 {
    FLUSHES.fetch_add(1, Ordering::SeqCst);
    LAST_FENCE.fetch_add(1, Ordering::SeqCst);
    0
}
/// Waits for every submitted kick.
unsafe extern "C" fn b_finish() -> i32 {
    FINISHES.fetch_add(1, Ordering::SeqCst);
    COMPLETED.store(LAST_FENCE.load(Ordering::SeqCst), Ordering::SeqCst);
    0
}
unsafe extern "C" fn b_kick_id() -> u32 {
    LAST_FENCE.load(Ordering::SeqCst) + 1
}
/// The PowerVR's rule (`pvr_backend_kick_pending`): any kick after the last
/// completed one, the open frame included.
unsafe extern "C" fn b_kick_pending(id: u32) -> bool {
    id != 0 && id > COMPLETED.load(Ordering::SeqCst)
}

/* The two tables are built from zero (every hook absent, every flag clear)
 * with the fields this file uses set, rather than written out in full: a
 * field another change adds to YakoglBackend or OsalOps then needs no edit
 * here (CLAUDE.md, "Two green pull requests, red main"). A zero that is not
 * a valid value for a new field fails the constant's evaluation, at build
 * time. */
static OSAL: OsalOps = {
    // SAFETY: OsalOps is Options of fn pointers, for which all-zero is None.
    let mut o: OsalOps = unsafe { core::mem::zeroed() };
    o.gpu_alloc = Some(m_gpu_alloc);
    o.gpu_free = Some(m_gpu_free);
    o.cache_flush = Some(m_cache);
    o.cache_invd = Some(m_cache);
    o
};

static GRID: YakoglBackend = {
    // SAFETY: YakoglBackend is a pointer, Options of fn pointers, bools and
    // integers, for which all-zero is a valid value (null, None, false, 0).
    let mut b: YakoglBackend = unsafe { core::mem::zeroed() };
    b.name = c"es3-pvr-3d-lod-grid".as_ptr() as *const c_char;
    b.init = Some(b_init);
    b.draw = Some(b_draw);
    b.flush = Some(b_flush);
    b.finish = Some(b_finish);
    b.kick_id = Some(b_kick_id);
    b.kick_pending = Some(b_kick_pending);
    b.heap_base = Some(b_heap_base);
    b.max_instance_divisor = 1;
    b
};

/// The one GL context of this file, current, with [`GRID`] as the back end
/// and no kick in flight.
fn gl_context() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    static ONCE: std::sync::Once = std::sync::Once::new();
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the tables are statics; the lock serialises the GL users.
    unsafe {
        ONCE.call_once(|| {
            osal_set_ops(&OSAL);
            backend::g_backend = &GRID;
            assert!(!context::gl_context_create(8, 8).is_null());
        });
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        while glGetError() != GL_NO_ERROR {}
    }
    COMPLETED.store(LAST_FENCE.load(Ordering::SeqCst), Ordering::SeqCst);
    g
}

/// Texel (x, y, z) of level `l`: its coordinates and level, so no two
/// texels of a volume here are the same.
fn texel(l: u32, x: u32, y: u32, z: u32) -> [u8; 4] {
    [x as u8, y as u8, z as u8, 0x80 | l as u8]
}

/// The volume of this file: 16x8x4 RGBA8, its whole chain (five levels,
/// 16x8x4 .. 1x1x1) stored and uploaded, LINEAR_MIPMAP_LINEAR.
const W: u32 = 16;
const H: u32 = 8;
const D: u32 = 4;
const CHAIN: u32 = 5;

fn volume() -> GLuint {
    let mut name = 0;
    // SAFETY: room for one name; each level's pixels hold its texels.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(GL_TEXTURE_3D, name);
        glTexStorage3D(GL_TEXTURE_3D, CHAIN as GLsizei, GL_RGBA8, W as GLsizei, H as GLsizei, D as GLsizei);
        for l in 0..CHAIN {
            let (lw, lh, ld) = twiddle_extent(W, H, D, l);
            let mut px = Vec::new();
            for z in 0..ld {
                for y in 0..lh {
                    for x in 0..lw {
                        px.extend_from_slice(&texel(l, x, y, z));
                    }
                }
            }
            glTexSubImage3D(
                GL_TEXTURE_3D,
                l as GLint,
                0,
                0,
                0,
                lw as GLsizei,
                lh as GLsizei,
                ld as GLsizei,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                px.as_ptr() as *const c_void,
            );
        }
        glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
    name
}

fn tex(name: GLuint) -> &'static GlTexture {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, name).expect("a texture name");
    c.texture_object(slot).expect("a texture object")
}

fn parami(pname: GLenum, v: GLint) {
    // SAFETY: plain GL call on the bound volume.
    unsafe { glTexParameteri(GL_TEXTURE_3D, pname, v) };
}

fn paramf(pname: GLenum, v: GLfloat) {
    // SAFETY: plain GL call on the bound volume.
    unsafe { glTexParameterf(GL_TEXTURE_3D, pname, v) };
}

fn delete(name: GLuint) {
    // SAFETY: a name this test made.
    unsafe { glDeleteTextures(1, &name) };
}

/// The bind the draw gives a native 3D texture after its copy
/// (`gl::draw`): the texture's level 0 size and depth, the levels counted
/// from level 0 (`native_vol::sampled_levels`), the copy's address and bytes,
/// the filters as `gl::draw::sampling_fields` has them.
fn bind_of(t: &GlTexture, (gpu, bytes): (u64, u32)) -> GlesTextureBind {
    let levels = native_vol::sampled_levels(t);
    GlesTextureBind {
        format: GLES_TEXFMT_RGBA8,
        type_: GLES_TEXTYPE_3D_TWIDDLED,
        layers: t.layers,
        base_gpu: gpu,
        width: t.width,
        height: t.height,
        stride_texels: t.width,
        bytes,
        linear_filter: u8::from(t.mag_filter == GL_LINEAR),
        min_linear: u8::from(matches!(t.min_filter, GL_LINEAR | GL_LINEAR_MIPMAP_NEAREST | GL_LINEAR_MIPMAP_LINEAR)),
        mip_filter: if levels > 1 { 2 } else { 0 },
        mip_levels: levels as u8,
        wrap_s: GLES_WRAP_REPEAT,
        wrap_t: GLES_WRAP_REPEAT,
        wrap_r: GLES_WRAP_REPEAT,
        ..GlesTextureBind::ZERO
    }
}

/// The fields of a descriptor the level and LOD parameters move:
/// (width - 1, height - 1, depth - 1, num_mip_levels, mipmaps_present,
/// baselevel, minlod, maxlod, mipfilter), from texstate.xml IMAGE_WORD0,
/// IMAGE_WORD1 and SAMPLER_WORD0.
fn fields(w: &[u32]) -> [u64; 9] {
    let iw0 = w[0] as u64 | (w[1] as u64) << 32;
    let iw1 = w[2] as u64 | (w[3] as u64) << 32;
    let s = w[8] as u64 | (w[9] as u64) << 32;
    [
        (iw0 >> 34) & 0x3fff,
        (iw0 >> 48) & 0x3fff,
        (iw1 >> 4) & 0x7ff,
        iw1 & 0xf,
        (iw1 >> 15) & 1,
        iw1 >> 60,
        (s >> 13) & 0x3ff,
        (s >> 23) & 0x3ff,
        (s >> 40) & 1,
    ]
}

/// The copy's bytes. The host OSAL's GPU address is its CPU pointer.
fn bytes_of((gpu, bytes): (u64, u32)) -> &'static [u8] {
    // SAFETY: the copy is `bytes` long and lives while the test runs.
    unsafe { core::slice::from_raw_parts(gpu as *const u8, bytes as usize) }
}

/// Every texel of every level the copy holds is GL level base + k's, at
/// level k of a chain over the base level's extent.
fn assert_copy_from_base(c: (u64, u32), v: &View) {
    let out = bytes_of(c);
    let (w, h, d) = v.extent;
    let phys = physical_extent(w, h, d);
    assert_eq!(c.1, copy_bytes(phys, 4, v.held), "base {}: the chain over the base level's extent", v.base);
    for k in 0..v.held {
        let ext = twiddle_extent(w, h, d, k);
        for z in 0..ext.2 {
            for y in 0..ext.1 {
                for x in 0..ext.0 {
                    let at = ((level_texel_off(phys, k) + twiddle3d(x, y, z, ext)) * 4) as usize;
                    assert_eq!(
                        out[at..at + 4],
                        texel(v.base + k, x, y, z),
                        "base {}: copy level {k} ({x}, {y}, {z}) is GL level {}",
                        v.base,
                        v.base + k
                    );
                }
            }
        }
    }
}

/// GL_TEXTURE_BASE_LEVEL: the view starts at the base level (its size, the
/// levels from it to the end of the chain), and without a mipmap filter it
/// is the base level alone (ES 3.0 3.8.9: level_base is sampled). A base
/// level past the chain is the last level (an immutable texture's clamp,
/// 3.8.10).
#[test]
fn the_view_starts_at_the_base_level() {
    let _g = gl_context();
    let name = volume();
    let v = native_vol::view(tex(name));
    assert_eq!((v.base, v.extent, v.held, v.levels, v.mip_filter), (0, (16, 8, 4), 5, 5, 2), "the defaults");
    assert_eq!((v.min_lod, v.max_lod), (-1000.0, 1000.0));
    for (base, extent) in [(1, (8, 4, 2)), (2, (4, 2, 1)), (3, (2, 1, 1)), (4, (1, 1, 1))] {
        parami(GL_TEXTURE_BASE_LEVEL, base);
        let v = native_vol::view(tex(name));
        let n = CHAIN - base as u32;
        assert_eq!((v.base, v.extent, v.held, v.levels), (base as u32, extent, n, n), "base {base}");
        assert_eq!(v.mip_filter, if n > 1 { 2 } else { 0 }, "base {base}: one level mixes nothing");
    }
    parami(GL_TEXTURE_BASE_LEVEL, 9);
    assert_eq!(native_vol::view(tex(name)).base, CHAIN - 1, "clamped to the last level");
    parami(GL_TEXTURE_BASE_LEVEL, 2);
    parami(GL_TEXTURE_MIN_FILTER, GL_LINEAR as GLint);
    let v = native_vol::view(tex(name));
    assert_eq!((v.base, v.extent, v.held, v.levels, v.mip_filter), (2, (4, 2, 1), 1, 1, 0), "no mipmap filter");
    delete(name);
}

/// GL_TEXTURE_MAX_LEVEL caps the levels the sampler walks, counted from the
/// base level (q = min(p, MAX_LEVEL)), but not the levels the copy holds:
/// draws that change only MAX_LEVEL share one copy.
#[test]
fn max_level_caps_the_walk_from_the_base_level_and_not_the_copy() {
    let _g = gl_context();
    let name = volume();
    for (base, max, levels) in [(0, 0, 1), (0, 2, 3), (1, 2, 2), (1, 3, 3), (2, 2, 1), (2, 1, 1), (0, 1000, 5)] {
        parami(GL_TEXTURE_BASE_LEVEL, base);
        parami(GL_TEXTURE_MAX_LEVEL, max);
        let v = native_vol::view(tex(name));
        assert_eq!((v.held, v.levels), (CHAIN - base as u32, levels), "base {base} max {max}");
        assert_eq!(v.mip_filter, if levels > 1 { 2 } else { 0 }, "base {base} max {max}");
    }
    parami(GL_TEXTURE_BASE_LEVEL, 1);
    parami(GL_TEXTURE_MAX_LEVEL, 3);
    let n0 = native_vol::copies();
    let a = native_vol::copy(tex(name), 0, 4).expect("a copy");
    parami(GL_TEXTURE_MAX_LEVEL, 2);
    let b = native_vol::copy(tex(name), 0, 3).expect("a copy");
    assert_eq!((a, native_vol::copies() - n0), (b, 1), "the same copy for another MAX_LEVEL");
    let w = texture_descriptor(&bind_of(tex(name), b));
    assert_eq!(fields(&w), [7, 3, 1, 2, 1, 0, 0, 64, 1], "8x4x2, two levels, LOD 0 to 1");
    parami(GL_TEXTURE_MAX_LEVEL, 1000);
    delete(name);
}

/// The copy for each base level holds GL levels base.. at levels 0.. of a
/// chain over the base level's extent, and its descriptor is that level's
/// size and depth, the levels from it, baselevel 0 and the LOD range of
/// those levels.
#[test]
fn each_base_level_copies_its_levels_and_describes_its_size() {
    let _g = gl_context();
    let name = volume();
    for base in 0..CHAIN {
        parami(GL_TEXTURE_BASE_LEVEL, base as GLint);
        let t = tex(name);
        let v = native_vol::view(t);
        let c = native_vol::copy(t, 0, native_vol::sampled_levels(t)).expect("a copy");
        assert_copy_from_base(c, &v);
        assert_eq!(native_vol::bound_view(c.0), Some(v), "base {base}: the view the descriptor reads");
        let n = u64::from(CHAIN - base);
        let (w, h, d) = (u64::from(v.extent.0), u64::from(v.extent.1), u64::from(v.extent.2));
        let f = fields(&texture_descriptor(&bind_of(t, c)));
        let mips = u64::from(n > 1);
        assert_eq!(f, [w - 1, h - 1, d - 1, n, mips, 0, 0, (n - 1) * 64, mips], "base {base}");
    }
    /* no mipmap filter: the base level alone, one level, LOD 0 */
    parami(GL_TEXTURE_BASE_LEVEL, 1);
    parami(GL_TEXTURE_MIN_FILTER, GL_LINEAR as GLint);
    let t = tex(name);
    let v = native_vol::view(t);
    let c = native_vol::copy(t, 0, 1).expect("a copy");
    assert_copy_from_base(c, &v);
    assert_eq!(c.1, 8 * 4 * 4 * 4, "8x4x2 padded to 8x4x4, RGBA8");
    assert_eq!(fields(&texture_descriptor(&bind_of(t, c))), [7, 3, 1, 1, 0, 0, 0, 0, 0]);
    delete(name);
}

/// GL_TEXTURE_MIN_LOD and GL_TEXTURE_MAX_LOD reach SAMPLER_WORD0's minlod
/// and maxlod in u4.6 (64 per level), inside the LOD range the levels
/// allow: a negative bound is 0, a minimum past the maximum is the maximum,
/// and the defaults (-1000, 1000) are the levels' range as before.
#[test]
fn min_and_max_lod_are_the_samplers_clamps() {
    let _g = gl_context();
    let name = volume();
    let lod = |min: f32, max: f32| {
        paramf(GL_TEXTURE_MIN_LOD, min);
        paramf(GL_TEXTURE_MAX_LOD, max);
        let t = tex(name);
        let c = native_vol::copy(t, 0, native_vol::sampled_levels(t)).expect("a copy");
        let f = fields(&texture_descriptor(&bind_of(t, c)));
        (f[6], f[7])
    };
    assert_eq!(lod(-1000.0, 1000.0), (0, 4 * 64), "the defaults: the whole chain");
    assert_eq!(lod(1.0, 1000.0), (64, 4 * 64));
    assert_eq!(lod(1.5, 2.25), (96, 144));
    assert_eq!(lod(-2.0, 3.0), (0, 192));
    assert_eq!(lod(0.0, 0.0), (0, 0), "dEQP max_lod 0: level 0 alone");
    assert_eq!(lod(3.0, 1.0), (64, 64), "a minimum past the maximum");
    assert_eq!(lod(10.0, 1000.0), (4 * 64, 4 * 64), "past the last level: the last level");
    assert_eq!(lod(-5.0, -1.0), (0, 0), "a LOD at most 0 magnifies");
    /* counted from the base level: base 2 has three levels, LOD 0 to 2 */
    parami(GL_TEXTURE_BASE_LEVEL, 2);
    assert_eq!(lod(-1000.0, 1000.0), (0, 2 * 64));
    assert_eq!(lod(0.5, 1.75), (32, 112));
    parami(GL_TEXTURE_BASE_LEVEL, 0);
    /* one level with min and mag filters apart: LOD 1.0 at most (the
     * TPU's choice on the clamped LOD), and MAX_LOD below it */
    parami(GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
    assert_eq!(lod(-1000.0, 1000.0), (0, 64));
    assert_eq!(lod(-1000.0, 0.5), (0, 32));
    assert_eq!(lod(-1000.0, -1.0), (0, 0), "MAX_LOD below 0: magnified");
    delete(name);
}

/// Only the native 3D descriptor reads the view: a stride 2D bind at the
/// same address keeps LOD 0 to its levels and its own size, whatever the
/// texture's base level and LOD range, and a native 3D bind no copy has
/// recorded is described as the bind says.
#[test]
fn other_binds_do_not_read_the_view() {
    let _g = gl_context();
    let name = volume();
    parami(GL_TEXTURE_BASE_LEVEL, 1);
    paramf(GL_TEXTURE_MIN_LOD, 1.5);
    paramf(GL_TEXTURE_MAX_LOD, 2.0);
    let t = tex(name);
    let c = native_vol::copy(t, 0, native_vol::sampled_levels(t)).expect("a copy");
    let b = bind_of(t, c);
    let s = fields(&texture_descriptor(&GlesTextureBind { type_: GLES_TEXTYPE_2D, ..b }));
    assert_eq!((s[0], s[1], s[6], s[7]), (15, 7, 0, 4 * 64), "a stride 2D: its own size, LOD 0 to 4");
    let none = fields(&texture_descriptor(&GlesTextureBind { base_gpu: 0x80_0000_9000, ..b }));
    assert_eq!(none, [15, 7, 3, 5, 1, 0, 0, 4 * 64, 1], "no copy recorded: the bind's level 0 and levels");
    delete(name);
}

/// One frame, one volume, a draw per base level (dEQP's base_level cells):
/// each base level gets its own copy, a base level asked for again gets its
/// kept copy, and no copy handed to the open frame is written again.
#[test]
fn a_frame_of_base_levels_binds_a_copy_for_each() {
    let _g = gl_context();
    let name = volume();
    let mut copies = Vec::new();
    for base in 0..CHAIN {
        parami(GL_TEXTURE_BASE_LEVEL, base as GLint);
        let t = tex(name);
        let c = native_vol::copy(t, 0, native_vol::sampled_levels(t)).expect("a copy");
        copies.push((c, native_vol::view(t)));
    }
    for (i, (c, _)) in copies.iter().enumerate() {
        assert!(copies[..i].iter().all(|(o, _)| o.0 != c.0), "base {i}: its own copy");
    }
    for (c, v) in &copies {
        assert_copy_from_base(*c, v);
    }
    parami(GL_TEXTURE_BASE_LEVEL, 1);
    let n0 = native_vol::copies();
    let t = tex(name);
    assert_eq!(native_vol::copy(t, 0, native_vol::sampled_levels(t)), Some(copies[1].0), "kept");
    assert_eq!(native_vol::copies(), n0, "not made again");
    parami(GL_TEXTURE_BASE_LEVEL, 0);
    delete(name);
}

/// The texels change while the open frame may read the copy: the new texels
/// go to another copy and the old one keeps the bytes the frame was
/// recorded with. Once its kicks are done its memory is used again.
#[test]
fn a_stale_copy_a_kick_may_read_is_not_written_again() {
    let _g = gl_context();
    let name = volume();
    let t = tex(name);
    let a = native_vol::copy(t, 0, native_vol::sampled_levels(t)).expect("a copy");
    let before = bytes_of(a).to_vec();
    let px = [9u8, 9, 9, 9];
    // SAFETY: one RGBA8 texel of level 0.
    unsafe {
        glTexSubImage3D(GL_TEXTURE_3D, 0, 0, 0, 0, 1, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_ptr() as *const c_void)
    };
    let t = tex(name);
    let b = native_vol::copy(t, 0, native_vol::sampled_levels(t)).expect("a copy");
    assert_ne!(a.0, b.0, "the open frame reads the first copy: another one");
    assert_eq!(bytes_of(a), &before[..], "the first copy keeps its bytes");
    assert_eq!(bytes_of(b)[..4], px, "the second has the new texel (twiddle index 0)");
    assert_eq!(native_vol::bound_view(a.0), None, "the stale copy is out of the lookup");
    /* the frame is submitted and done: the next stale copy is written over */
    // SAFETY: back-end calls of this file's stand-in.
    unsafe {
        b_flush();
        b_flush();
        b_finish();
    }
    let px2 = [7u8, 7, 7, 7];
    // SAFETY: one RGBA8 texel of level 0.
    unsafe {
        glTexSubImage3D(GL_TEXTURE_3D, 0, 0, 0, 0, 1, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px2.as_ptr() as *const c_void)
    };
    let t = tex(name);
    let c = native_vol::copy(t, 0, native_vol::sampled_levels(t)).expect("a copy");
    assert_eq!(c.0, b.0, "no kick reads it any more: written in place");
    assert_eq!(bytes_of(c)[..4], px2);
    delete(name);
}

/// The draw lands in the kick after the one it was recorded in when the
/// back end ends the open frame for it: that kick counts as reading the copy
/// too, until it is done.
#[test]
fn the_kick_after_the_hand_out_reads_the_copy_too() {
    let _g = gl_context();
    let name = volume();
    let t = tex(name);
    let a = native_vol::copy(t, 0, native_vol::sampled_levels(t)).expect("a copy");
    /* the frame it was handed out in is submitted and completes; the draw
     * itself went to the next one, still open */
    // SAFETY: back-end calls of this file's stand-in.
    unsafe {
        b_flush();
        b_finish();
    }
    let px = [5u8, 5, 5, 5];
    // SAFETY: one RGBA8 texel of level 0.
    unsafe {
        glTexSubImage3D(GL_TEXTURE_3D, 0, 0, 0, 0, 1, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_ptr() as *const c_void)
    };
    let t = tex(name);
    let b = native_vol::copy(t, 0, native_vol::sampled_levels(t)).expect("a copy");
    assert_ne!(a.0, b.0, "the next kick may read the first copy");
    delete(name);
}

/// More base-level copies in flight than the cache keeps: the open frame
/// is submitted and waited for before a copy a kick reads is written over,
/// never written while it is read.
#[test]
fn a_full_cache_of_copies_in_flight_waits_before_it_writes() {
    let _g = gl_context();
    let names: Vec<GLuint> = (0..17).map(|_| volume()).collect();
    let (f0, w0) = (FLUSHES.load(Ordering::SeqCst), FINISHES.load(Ordering::SeqCst));
    let copy_of = |i: usize| {
        let name = names[i];
        // SAFETY: plain GL call.
        unsafe { glBindTexture(GL_TEXTURE_3D, name) };
        parami(GL_TEXTURE_BASE_LEVEL, (i as u32 % CHAIN) as GLint);
        let t = tex(name);
        let v = native_vol::view(t);
        let c = native_vol::copy(t, 0, native_vol::sampled_levels(t)).expect("a copy");
        assert_copy_from_base(c, &v);
    };
    /* sixteen kept copies, all handed to the open frame: room for each */
    for i in 0..16 {
        copy_of(i);
    }
    assert_eq!((FLUSHES.load(Ordering::SeqCst), FINISHES.load(Ordering::SeqCst)), (f0, w0), "no wait yet");
    /* one more */
    copy_of(16);
    assert_eq!(FLUSHES.load(Ordering::SeqCst), f0 + 1, "the open frame was submitted");
    assert_eq!(FINISHES.load(Ordering::SeqCst), w0 + 1, "and waited for");
    for name in names {
        delete(name);
    }
}
