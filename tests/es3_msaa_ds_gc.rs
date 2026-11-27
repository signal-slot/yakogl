// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): the depth and stencil resolve of a 4x FBO's depth
//! surface, `ds_cache::resolve_plane` (step G1 of the 4x depth/stencil
//! design, docs/notes/es3-msaa-renderbuffers.md).
//!
//! A 4x FBO draws into a scale-2 surface (`ds_cache::key_of(gl, 2)`, twice
//! as wide and high): row-linear at its row stride `align64(2w) * 4`, each
//! pixel's four samples a 2x2 block of D24S8 words. The test seeds such a
//! surface through the real `select_in` and the probes `memory_of`,
//! `stride_of` and `planes_of`, with a different value in each of a pixel's
//! four words, and checks that the resolve of pixel (x, y) is the word at
//! row 2y, column 2x (sample 0): depth `(word >> 8) / (2^24 - 1)` as `f32`,
//! stencil `word & 0xff`. Elements past the surface and the pitch's padding
//! keep their sentinel. A plane that holds a whole clear (in its surface or
//! parked) is that value everywhere; a plane in no surface answers 1.
//!
//! The resolve does not depend on `msaa-fbo` (a scale-2 surface is the
//! cache's own), so the file needs only the Vivante host back end and runs
//! in every gpu-vivante row of tools/check.sh's CONFIGS:
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_msaa_ds_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante"))]

use core::ffi::c_void;
use std::collections::BTreeMap;
use std::sync::Mutex;
use yakogl::ds_retention::Plane;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::ds_cache::{self, DsKey, PlaneOut};
use yakogl::vivante::layout::Layout;

static ALLOCS: Mutex<BTreeMap<u64, Box<[u64]>>> = Mutex::new(BTreeMap::new());
static SERIAL: Mutex<()> = Mutex::new(());

unsafe extern "C" fn p_gpu_alloc(heap_base: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mut a = ALLOCS.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
    let gpu = heap_base + 0x1000 + a.len() as u64 * (1 << 24);
    let cpu = mem.as_mut_ptr() as *mut c_void;
    // SAFETY: the caller passes a writable descriptor.
    unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
    a.insert(gpu, mem);
    0
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor.
    let m = unsafe { *mem };
    // Leaked on purpose: the next surface must not reuse an address.
    if let Some(b) = ALLOCS.lock().unwrap_or_else(|e| e.into_inner()).remove(&m.gpu) {
        core::mem::forget(b);
    }
    // SAFETY: as above; a freed descriptor is cleared.
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

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

const GUARD: usize = 4096;
/// A depth-stencil renderbuffer (bit 30 clear: not a texture).
const ID: u32 = 0x0000_0071;
const KEY: DsKey = DsKey { depth_id: ID, depth_gen: 1, stencil_id: ID, stencil_gen: 1, scale: 2 };
const SENTINEL_F: f32 = -7.5;
const SENTINEL_S: u8 = 0xa5;

/// The depth rule: bits 31..8 of the word, over 2^24 - 1.
fn depth_of(word: u32) -> f32 {
    ((word >> 8) as f64 / 16_777_215.0) as f32
}

/// The stencil rule: bits 7..0.
fn stencil_of(word: u32) -> u8 {
    (word & 0xff) as u8
}

/// A fixed word for (row, column) of the surface: every word differs from
/// the other three of its 2x2 block in both planes.
fn word_at(r: usize, c: usize) -> u32 {
    let mut x = ((r * 4099 + c) as u32).wrapping_mul(0x9e37_79b9) | 1;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    /* the sample (r & 1, c & 1) in the low two bits of each plane */
    let s = ((r & 1) << 1 | (c & 1)) as u32;
    (x & !0x0300 & !0x03) | s << 8 | s
}

/// The test lock held, the OSAL and the Vivante back end set up.
fn init() -> std::sync::MutexGuard<'static, ()> {
    let g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; the tables are statics.
    unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl::vivante::backend::yakogl_backend_vivante;
        assert_eq!(yakogl::vivante::backend::gc_init(), 0, "gc_init");
    }
    g
}

fn shutdown() {
    yakogl::vivante::backend::gc_shutdown();
}

/// Select `key`'s surface for a `w` x `h` draw (the surface is `scale`
/// times that on each axis) and record a draw that `writes` and `clears`.
fn draw(key: DsKey, w: u32, h: u32, kick: u32, writes: [bool; 2], clears: [Option<u32>; 2]) {
    let s = key.scale;
    ds_cache::select_in(Layout::Linear, key, w * s, h * s, GUARD, kick, None, |_| false, || 0).expect("a surface");
    ds_cache::drawn(key, kick, false, writes, clears);
}

/// `key`'s surface words, seeded with [`word_at`]: the words and the row
/// stride in words.
fn seed(key: DsKey) -> (&'static mut [u32], usize) {
    let (mem, bytes) = ds_cache::memory_of(key).expect("a surface");
    let stride = ds_cache::stride_of(key).expect("a surface") as usize / 4;
    // SAFETY: the allocation holds guard + bytes + guard bytes, word-aligned
    // past GUARD; it lives until the test shuts the back end down (and is
    // leaked by p_gpu_free after that).
    let words = unsafe { core::slice::from_raw_parts_mut((mem.cpu as *mut u8).add(GUARD) as *mut u32, bytes / 4) };
    for (i, w) in words.iter_mut().enumerate() {
        *w = word_at(i / stride, i % stride);
    }
    (words, stride)
}

fn resolve_depth(id: u32, dst: &mut [f32], w: u32, h: u32, pitch: u32) -> i32 {
    assert!(dst.len() >= (h * pitch) as usize);
    // SAFETY: `h` rows of `pitch` floats.
    unsafe { ds_cache::resolve_plane(id, PlaneOut::Depth(dst.as_mut_ptr()), w, h, pitch, GUARD, |_| 0) }
}

fn resolve_stencil(id: u32, dst: &mut [u8], w: u32, h: u32, pitch: u32) -> i32 {
    assert!(dst.len() >= (h * pitch) as usize);
    // SAFETY: `h` rows of `pitch` bytes.
    unsafe { ds_cache::resolve_plane(id, PlaneOut::Stencil(dst.as_mut_ptr()), w, h, pitch, GUARD, |_| 0) }
}

/// (draw w, draw h, ask w, ask h, pitch): odd widths end the row's `u128`
/// middle at every remainder; the last two ask past the surface (a 40 x 40
/// draw's surface is 128 words, 64 pixels, a row and 64 pixel rows).
const CASES: [(u32, u32, u32, u32, u32); 7] = [
    (40, 40, 40, 40, 40),
    (40, 40, 37, 23, 41),
    (13, 9, 13, 9, 13),
    (13, 9, 3, 2, 5),
    (1, 1, 1, 1, 1),
    (40, 40, 70, 66, 72),
    (64, 33, 65, 33, 65),
];

/// Pixel (x, y)'s expected word, or `None` past the surface.
fn expected(words: &[u32], stride: usize, x: usize, y: usize) -> Option<u32> {
    (2 * x < stride).then(|| words.get(2 * y * stride + 2 * x).copied()).flatten()
}

#[test]
fn depth_resolve_reads_sample_0_of_every_pixel() {
    for (dw, dh, w, h, pitch) in CASES {
        let _g = init();
        draw(KEY, dw, dh, 1, [true, true], [None, None]);
        assert_eq!(ds_cache::planes_of(KEY), Some([Plane::LIVE; 2]));
        let (words, stride) = seed(KEY);
        let mut out = vec![SENTINEL_F; (h * pitch) as usize];
        assert_eq!(resolve_depth(ID, &mut out, w, h, pitch), 0, "{dw}x{dh} ask {w}x{h}/{pitch}");
        for y in 0..h as usize {
            for x in 0..pitch as usize {
                let got = out[y * pitch as usize + x];
                let want = if x < w as usize { expected(words, stride, x, y).map(depth_of) } else { None };
                assert_eq!(
                    got.to_bits(),
                    want.unwrap_or(SENTINEL_F).to_bits(),
                    "{dw}x{dh} ask {w}x{h}/{pitch}: depth at ({x}, {y})"
                );
            }
        }
        shutdown();
    }
}

#[test]
fn stencil_resolve_reads_sample_0_of_every_pixel() {
    for (dw, dh, w, h, pitch) in CASES {
        let _g = init();
        draw(KEY, dw, dh, 1, [true, true], [None, None]);
        let (words, stride) = seed(KEY);
        let mut out = vec![SENTINEL_S; (h * pitch) as usize];
        assert_eq!(resolve_stencil(ID, &mut out, w, h, pitch), 0, "{dw}x{dh} ask {w}x{h}/{pitch}");
        for y in 0..h as usize {
            for x in 0..pitch as usize {
                let got = out[y * pitch as usize + x];
                let want = if x < w as usize { expected(words, stride, x, y).map(stencil_of) } else { None };
                assert_eq!(got, want.unwrap_or(SENTINEL_S), "{dw}x{dh} ask {w}x{h}/{pitch}: stencil at ({x}, {y})");
                if x < w as usize && want.is_some() {
                    assert_eq!(got & 3, 0, "sample 0's marker at ({x}, {y})");
                }
            }
        }
        shutdown();
    }
}

/// The same surface at scale 1 reads word (y, x) for the stencil, as the
/// depth always has (tests/gc_depth_resolve.rs).
#[test]
fn stencil_resolve_of_a_single_sampled_surface_reads_word_y_x() {
    let _g = init();
    let key = DsKey { scale: 1, ..KEY };
    draw(key, 37, 11, 1, [true, true], [None, None]);
    let (words, stride) = seed(key);
    let (w, h, pitch) = (37u32, 11u32, 40u32);
    let mut out = vec![SENTINEL_S; (h * pitch) as usize];
    assert_eq!(resolve_stencil(ID, &mut out, w, h, pitch), 0);
    for y in 0..h as usize {
        for x in 0..pitch as usize {
            let want = if x < w as usize { stencil_of(words[y * stride + x]) } else { SENTINEL_S };
            assert_eq!(out[y * pitch as usize + x], want, "stencil at ({x}, {y})");
        }
    }
    shutdown();
}

#[test]
fn a_cleared_4x_plane_fills_its_clear_value() {
    let _g = init();
    let (d, s) = (0.375f32, 0x5au32);
    draw(KEY, 20, 10, 1, [false, false], [Some(d.to_bits()), Some(s)]);
    assert_eq!(ds_cache::planes_of(KEY), Some([Plane::cleared(d.to_bits()), Plane::cleared(s)]));
    /* words that are not the clear: the resolve must not read them */
    let _ = seed(KEY);
    let mut fd = vec![SENTINEL_F; 20 * 10];
    let mut fs = vec![SENTINEL_S; 20 * 10];
    assert_eq!(resolve_depth(ID, &mut fd, 20, 10, 20), 0);
    assert_eq!(resolve_stencil(ID, &mut fs, 20, 10, 20), 0);
    assert!(fd.iter().all(|v| v.to_bits() == d.to_bits()), "depth is the clear everywhere");
    assert!(fs.iter().all(|&v| v == s as u8), "stencil is the clear everywhere");
    shutdown();
}

#[test]
fn a_parked_4x_plane_fills_its_clear_value() {
    let _g = init();
    let (d, s) = (0.625f32, 0x3cu32);
    draw(KEY, 8, 8, 1, [false, false], [Some(d.to_bits()), Some(s)]);
    /* every slot to other pairs: the cleared surface, the least recently
     * used, goes and its clear values are parked */
    for i in 0..ds_cache::ENTRIES as u32 {
        let other = DsKey { depth_id: 0x200 + i, depth_gen: 1, stencil_id: 0x200 + i, stencil_gen: 1, scale: 1 };
        draw(other, 8, 8, 2 + i, [false, false], [None, None]);
    }
    assert!(ds_cache::memory_of(KEY).is_none(), "the cleared surface went to another pair");
    assert!(ds_cache::parked_len() >= 2, "both planes parked");
    let mut fd = vec![SENTINEL_F; 8 * 8];
    let mut fs = vec![SENTINEL_S; 8 * 8];
    assert_eq!(resolve_depth(ID, &mut fd, 8, 8, 8), 0);
    assert_eq!(resolve_stencil(ID, &mut fs, 8, 8, 8), 0);
    assert!(fd.iter().all(|v| v.to_bits() == d.to_bits()), "depth is the parked clear everywhere");
    assert!(fs.iter().all(|&v| v == s as u8), "stencil is the parked clear everywhere");
    shutdown();
}

#[test]
fn a_plane_in_no_surface_answers_1() {
    let _g = init();
    /* another object drawn: ID is in no surface and not parked */
    draw(DsKey { depth_id: 0x300, stencil_id: 0x300, ..KEY }, 8, 8, 1, [true, true], [None, None]);
    let mut fd = vec![SENTINEL_F; 64];
    let mut fs = vec![SENTINEL_S; 64];
    assert_eq!(resolve_depth(ID, &mut fd, 8, 8, 8), 1);
    assert_eq!(resolve_stencil(ID, &mut fs, 8, 8, 8), 1);
    assert!(fd.iter().all(|v| v.to_bits() == SENTINEL_F.to_bits()), "nothing written");
    assert!(fs.iter().all(|&v| v == SENTINEL_S), "nothing written");
    /* and one whose stencil point is empty has no stencil plane */
    let depth_only = DsKey { depth_id: 0x301, stencil_id: 0, stencil_gen: 0, ..KEY };
    draw(depth_only, 8, 8, 2, [true, false], [None, None]);
    assert_eq!(resolve_stencil(0x301, &mut fs, 8, 8, 8), 1);
    assert_eq!(resolve_depth(0x301, &mut fd, 8, 8, 8), 0);
    shutdown();
}

/// The read waits for the kick that last wrote the surface, and a failed
/// wait is the answer.
#[test]
fn the_resolve_waits_for_the_writing_kick() {
    let _g = init();
    draw(KEY, 8, 8, 9, [true, true], [None, None]);
    let mut out = vec![0u8; 64];
    let waited = core::cell::Cell::new(0);
    // SAFETY: 8 rows of 8 bytes.
    let rc = unsafe {
        ds_cache::resolve_plane(ID, PlaneOut::Stencil(out.as_mut_ptr()), 8, 8, 8, GUARD, |k| {
            waited.set(k);
            -5
        })
    };
    assert_eq!((rc, waited.get()), (-5, 9));
    shutdown();
}

#[test]
fn the_table_has_the_stencil_resolve() {
    let _g = init();
    let t = &yakogl::vivante::backend::yakogl_backend_vivante;
    let f = t.stencil_resolve.expect("stencil_resolve is set");
    let mut out = vec![SENTINEL_S; 16];
    // SAFETY: 4 rows of 4 bytes.
    assert_eq!(unsafe { f(0x400, out.as_mut_ptr(), 4, 4, 4) }, 1, "an object in no surface");
    assert!(out.iter().all(|&v| v == SENTINEL_S));
    shutdown();
}
