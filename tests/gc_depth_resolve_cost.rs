// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): what `ds_cache::resolve_depth` costs on the host for the
//! depth sizes of glmark2's shadow and refract scenes (both 1600x1200, a
//! D24S8 surface of 1600 x 1216 rows at 6400 B a row; the 1024 x 1024 of
//! `gc_depth_resolve.rs` is not what these scenes render), as ns per resolved
//! texel. `docs/notes/gc-shadow-refract-cause.md` scales it to the A53.
//!
//! Timing only, so every test is `#[ignore]`: nothing here is asserted about
//! time (the last texel of the first resolve is checked against the rule, so
//! a loop that got fast by being wrong shows). Besides `resolve_depth`, two
//! host references run on the same bytes: a plain `memcpy` of the surface's
//! bytes (the floor of any read of it) and the byte-wise loop `resolve_row`
//! replaced (four `u8` loads a word), which is what the board ran before
//! resolve-wide. Run with `--release`:
//!
//! ```text
//! cargo test --release --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_depth_resolve_cost -- --ignored --nocapture --test-threads=1
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::c_void;
use std::collections::BTreeMap;
use std::hint::black_box;
use std::sync::Mutex;
use std::time::Instant;
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
const ID: u32 = 0x4000_00c8;
const KEY: DsKey = DsKey { depth_id: ID, depth_gen: 1, stencil_id: ID, stencil_gen: 1, scale: 1 };
const RUNS: usize = 30;

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

/// Best and mean of `RUNS` runs of `f`, in ms.
fn time(mut f: impl FnMut()) -> (f64, f64) {
    let (mut best, mut total) = (f64::MAX, 0.0);
    for _ in 0..RUNS {
        let t = Instant::now();
        f();
        let ms = t.elapsed().as_secs_f64() * 1e3;
        best = best.min(ms);
        total += ms;
    }
    (best, total / RUNS as f64)
}

/// Time a `w` x `h` resolve and the two references, print ns a texel.
fn cost(scene: &str, w: u32, h: u32) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; the tables are statics.
    unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl::vivante::backend::yakogl_backend_vivante;
        assert_eq!(yakogl::vivante::backend::gc_init(), 0, "gc_init");
    }
    ds_cache::select(KEY, w, h, GUARD, 1, None, |_| false, || 0).expect("a surface");
    ds_cache::drawn(KEY, 1, false, [true, false], [None, None]);
    let (stride, rows) = (w.div_ceil(64) * 64 * 4, h.div_ceil(64) * 64);
    let bytes = stride as usize * rows as usize;
    let mut allocs = ALLOCS.lock().unwrap_or_else(|e| e.into_inner());
    let mut of_size = allocs.values_mut().filter(|m| m.len() == (bytes + 2 * GUARD).div_ceil(8));
    let mem = of_size.next().expect("the surface");
    // SAFETY: the allocation holds guard + bytes + guard bytes.
    let px = unsafe { core::slice::from_raw_parts_mut((mem.as_mut_ptr() as *mut u8).add(GUARD), bytes) };
    for (i, c) in px.chunks_exact_mut(4).enumerate() {
        c.copy_from_slice(&pattern(i).to_ne_bytes());
    }
    let texels = (w * h) as f64;
    let mut dst = vec![0.0f32; (w * h) as usize];
    let mut resolve = |dst: &mut [f32]| {
        // SAFETY: `h` rows of `w` floats.
        let rc = unsafe { ds_cache::resolve_depth(ID, dst.as_mut_ptr(), w, h, w, GUARD, |_| 0) };
        assert_eq!(rc, 0);
    };
    resolve(&mut dst);
    let at = (h as usize - 1) * (stride as usize / 4) + w as usize - 1;
    let last = u32::from_ne_bytes(px[at * 4..at * 4 + 4].try_into().unwrap());
    assert_eq!(dst[(w * h - 1) as usize].to_bits(), rule(last).to_bits(), "{scene}: the last texel");
    let (best, mean) = time(|| resolve(&mut dst));

    let mut copy = vec![0u8; bytes];
    let (cbest, _) = time(|| {
        copy.copy_from_slice(black_box(&*px));
        black_box(&mut copy);
    });
    let (bbest, _) = time(|| {
        for y in 0..h as usize {
            let row = &px[y * stride as usize..][..w as usize * 4];
            for (o, s) in dst[y * w as usize..][..w as usize].iter_mut().zip(row.chunks_exact(4)) {
                let word = (s[0] as u32) | (s[1] as u32) << 8 | (s[2] as u32) << 16 | (s[3] as u32) << 24;
                *o = rule(word);
            }
        }
        black_box(&mut dst);
    });
    let ns = |ms: f64| ms * 1e6 / texels;
    println!(
        "{scene} {w}x{h} ({} B read, {} B written): resolve_depth best {best:.3} ms mean {mean:.3} ms = {:.3} ns/texel; \
         memcpy of the surface best {cbest:.3} ms = {:.3} ns/texel; byte-wise loop best {bbest:.3} ms = {:.3} ns/texel",
        w as usize * h as usize * 4,
        w as usize * h as usize * 4,
        ns(best),
        ns(cbest),
        ns(bbest),
    );
    drop(allocs);
    yakogl::vivante::backend::gc_shutdown();
}

/// shadow: the depth texture of `DepthRenderTarget`, 1600x1200
/// (docs/notes/gc-slow-shadow-refract.md, "shadow", K2).
#[test]
#[ignore = "timing: run with --ignored --release"]
fn shadow_1600x1200_resolve_cost() {
    cost("shadow", 1600, 1200);
}

/// refract: the `DistanceRenderTarget` depth, 1600x1200 ("refract", K2).
#[test]
#[ignore = "timing: run with --ignored --release"]
fn refract_1600x1200_resolve_cost() {
    cost("refract", 1600, 1200);
}
