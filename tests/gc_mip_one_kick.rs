// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): a GPU mipmap chain (src/vivante/mipmap.rs) is one kick,
//! not one kick a level, and its stream holds a kick boundary's flushes
//! between the levels.
//!
//! One kick a level made glmark2's terrain 52 kicks a frame through the
//! back end's four stream slots (docs/notes/gc-slow-terrain.md, rank 2;
//! gc-slow-shadow-refract.md, cause 3). Here the real Vivante back end
//! (`gc_init`) runs over a stand-in OSAL that keeps every allocation in host
//! memory and every submitted stream, and is asked for the chain of a
//! 256x256 and a 1024x1024 RGBA8 texture as the GL core asks it
//! (`generate_mipmap`). The chain must be one kick holding one draw a level,
//! in order: level l's draw targets level l and samples level l - 1, and
//! before every level's draw but the first the stream holds what the end of
//! one kick and the start of the next made (`backend::emit_target_boundary`):
//! the pixel engine's colour cache written back, then the front end held
//! until the pixel engine has drained (a FE->PE semaphore and STALL), then
//! the texture cache invalidated. What the levels hold is
//! tests/gc_mipmap_gpu.rs's (the model runs the same one-kick streams).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_mip_one_kick
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::c_void;
use std::collections::BTreeMap;
use std::sync::Mutex;
use yakogl::backend::{GlesTextureBind, GLES_TEXFMT_RGBA8, GLES_TEXTYPE_2D};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{gc_flush, gc_init, gc_shutdown, yakogl_backend_vivante};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

/* ---- the stand-in OSAL: host memory, and the streams the back end submits --------- */

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, (Box<[u64]>, usize)>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    kicks: Vec<Vec<u32>>,
    /// Per kick: the texture descriptors its draws point at, read at submit
    /// time (the back end reuses a frame's arena GC_FRAMES kicks on).
    descriptors: Vec<BTreeMap<u32, Vec<u32>>>,
    faults: Vec<String>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);
/// Serialises the tests: the OSAL pointer and the back end are process-global.
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
        p.allocs.insert(gpu, (mem, size));
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

/// `n` words of the stand-in's memory at GPU address `gpu`.
fn read_words(p: &Platform, gpu: u64, n: usize) -> Option<Vec<u32>> {
    let (base, (mem, size)) = p.allocs.range(..=gpu).next_back()?;
    let off = (gpu - base) as usize;
    if off + n * 4 > *size {
        return None;
    }
    // SAFETY: inside the allocation (checked just above).
    Some(unsafe { core::slice::from_raw_parts((mem.as_ptr() as *const u8).add(off) as *const u32, n) }.to_vec())
}

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
        let Some(words) = read_words(p, k.stream_gpu, k.stream_bytes as usize / 4) else {
            p.faults.push(format!("the kick's stream 0x{:x} is no allocation", k.stream_gpu));
            return -22;
        };
        let mut descs = BTreeMap::new();
        for d in draws(&words) {
            if let Some(&at) = d.regs.get(&nte_descriptor_addr(0)) {
                if let Some(desc) = read_words(p, u64::from(at), GC_TEXDESC_BYTES / 4) {
                    descs.insert(at, desc);
                }
            }
        }
        p.kicks.push(words);
        p.descriptors.push(descs);
        p.fence += 1;
        p.fence
    })
}

unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}

/// Every kick completes at once.
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

/* ---- the stream, decoded ------------------------------------------------------------ */

/// One command of a stream, as far as the chain's checks need it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    /// A LOAD_STATE's register and value.
    State(u32, u32),
    /// A front-end STALL and its token.
    Stall(u32),
    Draw,
}

fn ops(words: &[u32]) -> Vec<Op> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        match w & 0xf800_0000 {
            GC_OP_LOAD_STATE => {
                let count = ((w >> 16) & 0x3ff) as usize;
                let base = (w & 0xffff) * 4;
                for k in 0..count {
                    if let Some(&v) = words.get(i + 1 + k) {
                        out.push(Op::State(base + 4 * k as u32, v));
                    }
                }
                i += 1 + count;
            }
            GC_OP_DRAW_INSTANCED => {
                out.push(Op::Draw);
                i += 4;
            }
            GC_OP_STALL => {
                out.push(Op::Stall(words.get(i + 1).copied().unwrap_or(0)));
                i += 2;
            }
            _ => i += 1,
        }
    }
    out
}

/// One draw of a stream, with the register file as the stream had loaded it.
struct Draw {
    regs: BTreeMap<u32, u32>,
}

fn draws(words: &[u32]) -> Vec<Draw> {
    let mut regs = BTreeMap::new();
    let mut out = Vec::new();
    for op in ops(words) {
        match op {
            Op::State(r, v) => {
                regs.insert(r, v);
            }
            Op::Draw => out.push(Draw { regs: regs.clone() }),
            Op::Stall(_) => {}
        }
    }
    out
}

/// The commands between draw `n - 1` and draw `n` (from 0).
fn between(ops: &[Op], n: usize) -> &[Op] {
    let at: Vec<usize> = ops.iter().enumerate().filter(|(_, o)| **o == Op::Draw).map(|(i, _)| i).collect();
    &ops[at[n - 1] + 1..at[n]]
}

/* ---- the cases ---------------------------------------------------------------------- */

/// The chain layout the GL core keeps for this back end: level l is
/// max(w >> l, 1) x max(h >> l, 1) rows of `w` texels, on a 64-byte boundary
/// after the level before. (offset, width, height) a level.
fn layout(w: u32, h: u32, levels: u32) -> Vec<(u32, u32, u32)> {
    let mut off = 0u32;
    (0..levels)
        .map(|l| {
            off = (off + 63) & !63;
            let (lw, lh) = ((w >> l).max(1), (h >> l).max(1));
            let at = off;
            off += w * lh * 4;
            (at, lw, lh)
        })
        .collect()
}

fn chain_case(size: u32) {
    let levels = 32 - size.leading_zeros();
    let lv = layout(size, size, levels);
    let bytes = lv.last().map(|&(off, _, lh)| off + size * lh * 4).unwrap() as usize;

    // SAFETY: the caller holds SERIAL; the table is a static.
    unsafe { osal_set_ops(&OSAL) };
    assert_eq!(gc_init(), 0, "gc_init");
    let mut storage = OsalGpuMem::ZERO;
    // SAFETY: a writable descriptor.
    assert_eq!(unsafe { p_gpu_alloc(0x4000_0000, bytes, 0, &mut storage) }, 0);
    let base = storage.gpu;
    let b = GlesTextureBind {
        unit: 0,
        format: GLES_TEXFMT_RGBA8,
        type_: GLES_TEXTYPE_2D,
        layers: 1,
        base_gpu: base,
        width: size,
        height: size,
        stride_texels: size,
        bytes: size * size * 4,
        linear_filter: 1,
        min_linear: 1,
        mip_filter: 2,
        mip_levels: levels as u8,
        bgra: 1,
        cpu: storage.cpu as *const c_void,
        ..GlesTextureBind::ZERO
    };
    let generate = yakogl_backend_vivante.generate_mipmap.expect("the Vivante back end answers generate_mipmap");

    /* twice: the second chain on a back end that has recorded one */
    for run in 1..=2 {
        let what = format!("{size}x{size} run {run}");
        let kicks0 = with_platform(|p| p.kicks.len());
        // SAFETY: a valid binding for the call.
        assert_eq!(unsafe { generate(&b, levels) }, 1, "{what}: the GPU path");
        assert_eq!(gc_flush(), 0, "{what}: nothing left open");
        let (kicks, descs, faults) = with_platform(|p| {
            (p.kicks[kicks0..].to_vec(), p.descriptors[kicks0..].to_vec(), core::mem::take(&mut p.faults))
        });
        assert!(faults.is_empty(), "{what}: {faults:?}");
        assert_eq!(kicks.len(), 1, "{what}: one kick for the {levels}-level chain");
        let (stream, descs) = (&kicks[0], &descs[0]);
        let d = draws(stream);
        assert_eq!(d.len(), levels as usize - 1, "{what}: one draw a level above 0");

        for (i, draw) in d.iter().enumerate() {
            let l = i + 1;
            let r = |reg: u32| draw.regs.get(&reg).copied().unwrap_or_else(|| panic!("{what} level {l}: 0x{reg:x}"));
            let (off, lw, lh) = lv[l];
            assert_eq!(u64::from(r(pe_pipe_color_addr(0))), base + u64::from(off), "{what}: level {l}'s target");
            assert_eq!(r(GC_PE_COLOR_STRIDE), size * 4, "{what}: level {l} at the chain's pitch");
            let vp = r(GC_PA_VIEWPORT_SCALE_X) as i32 as f32 / 65536.0;
            assert_eq!(vp, lw as f32 / 2.0, "{what}: level {l}'s viewport ({lw}x{lh})");
            let desc = descs.get(&r(nte_descriptor_addr(0))).unwrap_or_else(|| panic!("{what} level {l}: descriptor"));
            let src = u64::from(desc[texdesc_lod_addr(0) as usize / 4]);
            assert_eq!(src, base + u64::from(lv[l - 1].0), "{what}: level {l} samples level {}", l - 1);
        }

        /* between two levels: PE written back, FE held until PE drained,
         * then the texture cache invalidated before the next draw */
        let o = ops(stream);
        let fe_pe = stall_token(GC_SYNC_FE, GC_SYNC_PE);
        for n in 1..d.len() {
            let gap = between(&o, n);
            let pos = |p: &dyn Fn(&Op) -> bool| gap.iter().position(p);
            let color = pos(&|o| matches!(o, Op::State(GC_GL_FLUSH_CACHE, v) if v & GC_FLUSH_COLOR != 0));
            let sem = pos(&|o| *o == Op::State(GC_GL_SEMAPHORE_TOKEN, fe_pe));
            let stall = pos(&|o| *o == Op::Stall(fe_pe));
            let tex =
                gap.iter().rposition(|o| matches!(o, Op::State(GC_GL_FLUSH_CACHE, v) if v & GC_FLUSH_TEXTURE != 0));
            let (Some(color), Some(sem), Some(stall), Some(tex)) = (color, sem, stall, tex) else {
                panic!("{what}: before level {}'s draw, a flush or the stall is missing: {gap:x?}", n + 1);
            };
            assert!(color < sem && sem < stall && stall < tex, "{what}: level {}: the order {gap:x?}", n + 1);
        }
        println!("{what}: {levels} levels, one kick of {} words", stream.len());
    }

    gc_shutdown();
    with_platform(|p| {
        /* leaked, not freed: a static of the library may still name a block */
        for (_, (mem, _)) in core::mem::take(&mut p.allocs) {
            core::mem::forget(mem);
        }
    });
}

#[test]
fn a_256_chain_is_one_kick_with_the_flushes_between_its_levels() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    chain_case(256);
}

#[test]
fn a_1024_chain_is_one_kick_with_the_flushes_between_its_levels() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    chain_case(1024);
}
