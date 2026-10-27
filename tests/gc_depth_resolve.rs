// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): `ds_cache::resolve_depth`, the CPU read of a D24S8
//! surface into a depth texture's floats (docs/notes/gc-depth-resolve.md).
//!
//! The surface is row-linear at its row stride (ds_cache.rs `resolve_depth`),
//! each word depth in bits 31..8 and stencil in 7..0. The test fills a
//! surface with a known pattern through the real `select`, resolves it and
//! checks every texel against the rule written out here: `(word >> 8)` over
//! 2^24 - 1, as `f32`. The rule is the output the per-texel loop had before
//! it was rewritten to work on whole rows, so this file passing on both
//! sides of that change is the "bit-identical" check. Texels outside the
//! surface and the pitch's padding must stay as they were.
//!
//! The timing test prints the cost of one 1024 x 1024 resolve (the shadow
//! and refract textures of glmark2); it asserts nothing about time. Run it
//! with `--release` and `--nocapture`:
//!
//! ```text
//! cargo test --release --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_depth_resolve -- --nocapture
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::c_void;
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Instant;
use yakogl::backend::GlesDraw;
use yakogl::ds_retention::DepthTexels;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::ds_cache::{self, DsKey};

/// Host memory for every allocation; `select` makes exactly one per surface.
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
    // The memory is leaked on purpose: the next surface must not reuse an address.
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
const ID: u32 = 0x4000_0064;
const KEY: DsKey = DsKey { depth_id: ID, depth_gen: 1, stencil_id: ID, stencil_gen: 1, scale: 1 };

/// The rule: depth is bits 31..8 of the word, over 2^24 - 1.
fn rule(word: u32) -> f32 {
    ((word >> 8) as f64 / 16_777_215.0) as f32
}

/// xorshift32 of the index: a fixed pattern that touches every bit of a word.
fn pattern(i: usize) -> u32 {
    let mut x = (i as u32).wrapping_mul(0x9e37_79b9) | 1;
    x ^= x << 13;
    x ^= x >> 17;
    x ^ (x << 5)
}

/// Run `case` on the pixel words of a `w` x `h` surface that a draw wrote
/// depth to, and the surface's row stride in words.
fn with_surface(w: u32, h: u32, case: impl FnOnce(&mut [u32], u32)) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; the tables are statics.
    unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl::vivante::backend::yakogl_backend_vivante;
        assert_eq!(yakogl::vivante::backend::gc_init(), 0, "gc_init");
    }
    ds_cache::select(KEY, w, h, GUARD, 1, None, |_| false, || 0).expect("a surface");
    ds_cache::drawn(KEY, [true, false], [None, None]);
    /* the surface: 64-aligned width in words, 64-aligned rows (ds_cache.rs `select`) */
    let (stride, rows) = (w.div_ceil(64) * 64 * 4, h.div_ceil(64) * 64);
    let bytes = stride as usize * rows as usize;
    let mut allocs = ALLOCS.lock().unwrap_or_else(|e| e.into_inner());
    /* gc_init has made its own allocations: the surface is the one of this size */
    let mut of_size = allocs.values_mut().filter(|m| m.len() * 8 == bytes + 2 * GUARD);
    let mem = of_size.next().expect("the surface");
    assert!(of_size.next().is_none(), "one allocation of the surface's size");
    // SAFETY: the allocation holds guard + bytes + guard bytes.
    let words =
        unsafe { core::slice::from_raw_parts_mut((mem.as_mut_ptr() as *mut u8).add(GUARD) as *mut u32, bytes / 4) };
    case(words, stride / 4);
    drop(allocs);
    yakogl::vivante::backend::gc_shutdown();
}

fn resolve(dst: &mut [f32], w: u32, h: u32, pitch: u32) -> i32 {
    assert!(dst.len() >= (h * pitch) as usize);
    // SAFETY: `h` rows of `pitch` floats.
    unsafe { ds_cache::resolve_depth(ID, dst.as_mut_ptr(), w, h, pitch, GUARD, |_| false) }
}

/// Every texel of the resolve is the rule's value of the word at its place;
/// a texel outside the surface and the pitch's padding keep their sentinel.
#[test]
fn every_texel_is_the_rule_of_its_word() {
    /* a 90 x 70 surface is 128 words a row and 128 rows; some asks hold more than that */
    for (w, h, pitch) in [(90u32, 70u32, 90u32), (90, 70, 97), (128, 128, 129), (1, 1, 1), (150, 140, 160), (3, 5, 64)]
    {
        with_surface(90, 70, |words, tw| {
            for (i, v) in words.iter_mut().enumerate() {
                *v = pattern(i);
            }
            /* the extremes of the rule */
            words[0] = 0x0000_00ff;
            words[1] = 0xffff_ff00;
            words[2] = 0x8000_0000;
            let rows = words.len() as u32 / tw;
            let mut dst = vec![7.5f32; (h * pitch) as usize];
            assert_eq!(resolve(&mut dst, w, h, pitch), 0, "{w}x{h}/{pitch}");
            for y in 0..h {
                for x in 0..pitch {
                    let got = dst[(y * pitch + x) as usize];
                    let inside = x < w && x < tw && y < rows;
                    let want = if inside { rule(words[(y * tw + x) as usize]) } else { 7.5 };
                    assert_eq!(got.to_bits(), want.to_bits(), "{w}x{h}/{pitch} at ({x}, {y})");
                }
            }
        });
    }
}

/// Depth texels a draw carries (`GlesDraw::depth_texels`: the CPU wrote the
/// depth texture) become the surface's depth through `select`, the reverse
/// of the resolve: each word's bits 31..8 take the texel's 24-bit depth,
/// its stencil byte stays, the words past the texels stay, no depth is
/// restored from a clear value, and the resolve then answers the texels
/// bit for bit. The kick still using the surface is waited for first.
///
/// Revert check: without the seed the resolve answers the old words.
#[test]
fn depth_texels_become_the_surface_depth() {
    with_surface(90, 70, |words, tw| {
        for (i, v) in words.iter_mut().enumerate() {
            *v = pattern(i);
        }
        let before = words.to_vec();
        let (w, h, pitch) = (90u32, 70u32, 97u32);
        let texels: Vec<f32> =
            (0..h * pitch).map(|i| yakogl::ds_retention::unorm24_depth(pattern(i as usize + 7) & 0xff_ffff)).collect();
        let d = GlesDraw {
            depth_id: ID,
            ds_w: w,
            ds_h: h,
            depth_texels: texels.as_ptr(),
            depth_texels_pitch: pitch,
            ..GlesDraw::ZERO
        };
        let finishes = core::cell::Cell::new(0);
        // SAFETY: `texels` holds `h` rows of `pitch` floats.
        let seed = unsafe { DepthTexels::of(&d) };
        let sel = ds_cache::select(
            KEY,
            w,
            h,
            GUARD,
            2,
            seed,
            |k| k == 1,
            || {
                finishes.set(finishes.get() + 1);
                0
            },
        )
        .expect("the surface");
        assert_eq!(finishes.get(), 1, "the kick using the surface was waited for");
        assert_eq!(sel.restore[0], None);
        let rows = words.len() as u32 / tw;
        for y in 0..rows {
            for x in 0..tw {
                let (i, old) = ((y * tw + x) as usize, before[(y * tw + x) as usize]);
                let want = if x < w && y < h {
                    (yakogl::ds_retention::depth_unorm24(texels[(y * pitch + x) as usize]) << 8) | (old & 0xff)
                } else {
                    old
                };
                assert_eq!(words[i], want, "({x}, {y})");
            }
        }
        let mut dst = vec![7.5f32; (h * pitch) as usize];
        assert_eq!(resolve(&mut dst, w, h, pitch), 0);
        for y in 0..h {
            for x in 0..w {
                let at = (y * pitch + x) as usize;
                assert_eq!(dst[at].to_bits(), texels[at].to_bits(), "({x}, {y})");
            }
        }
    });
}

/// The depth object drawn in one pair's surface, then attached with another
/// stencil object at a size whose surface has another row stride: a draw
/// that carries texels takes them in the new surface, where a plain draw
/// would have to copy the old plane across strides (refused, -ENOTSUP).
/// The resolve then answers the texels.
///
/// Revert check: moving the plane before the texels refuses the draw.
#[test]
fn depth_texels_need_no_copy_from_a_surface_of_another_stride() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; the tables are statics.
    unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl::vivante::backend::yakogl_backend_vivante;
        assert_eq!(yakogl::vivante::backend::gc_init(), 0, "gc_init");
    }
    let first = DsKey { stencil_id: 0x65, ..KEY };
    ds_cache::select(first, 90, 70, GUARD, 1, None, |_| false, || 0).expect("the first surface");
    ds_cache::drawn(first, [true, false], [None, None]);
    let plain = ds_cache::select(KEY, 200, 70, GUARD, 2, None, |_| false, || 0);
    assert_eq!(plain, Err(-95), "a plain draw would copy across strides: refused");
    let (w, h) = (200u32, 70u32);
    let texels: Vec<f32> = (0..w * h).map(|i| yakogl::ds_retention::unorm24_depth(i * 97)).collect();
    let d = GlesDraw {
        depth_id: ID,
        ds_w: w,
        ds_h: h,
        depth_texels: texels.as_ptr(),
        depth_texels_pitch: w,
        ..GlesDraw::ZERO
    };
    // SAFETY: `texels` holds `h` rows of `w` floats.
    let seed = unsafe { DepthTexels::of(&d) };
    ds_cache::select(KEY, w, h, GUARD, 3, seed, |_| false, || 0).expect("the texels need no copy");
    let mut dst = vec![-1.0f32; (w * h) as usize];
    assert_eq!(resolve(&mut dst, w, h, w), 0);
    assert!(dst.iter().zip(&texels).all(|(a, b)| a.to_bits() == b.to_bits()), "the resolve answers the texels");
    yakogl::vivante::backend::gc_shutdown();
}

/// What the 1024 x 1024 resolve costs (printed, not asserted).
#[test]
fn time_one_1024_square_resolve() {
    with_surface(1024, 1024, |words, tw| {
        for (i, v) in words.iter_mut().enumerate() {
            *v = pattern(i);
        }
        let mut dst = vec![0.0f32; 1024 * 1024];
        let mut best = f64::MAX;
        let mut total = 0.0;
        const RUNS: usize = 40;
        for _ in 0..RUNS {
            let t = Instant::now();
            assert_eq!(resolve(&mut dst, 1024, 1024, 1024), 0);
            let ms = t.elapsed().as_secs_f64() * 1e3;
            best = best.min(ms);
            total += ms;
        }
        println!("resolve_depth 1024x1024: best {best:.3} ms, mean {:.3} ms over {RUNS}", total / RUNS as f64);
        assert_eq!(dst[123 * 1024 + 456].to_bits(), rule(words[(123 * tw + 456) as usize]).to_bits());
    });
}
