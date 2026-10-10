// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Unit tests of the Vivante backend, driven through its backend-contract
//! entry points.
//!
//! The command stream is the product here, so the tests decode it: [`Rig`]
//! installs an OSAL whose `submit` keeps the words, and [`Stream::reg`] reads
//! one register back out of the LOAD_STATE commands. What the registers should
//! hold comes from Mesa's etnaviv driver and is written down in
//! VIVANTE_NOTES.md; these tests pin what this backend decides -- the frame
//! boundaries, the arena snapshots, the descriptors, the state a draw derives
//! from `GlesDraw` -- not whether the GPU agrees, which no host test can say.
//!
//! testutil's mock OSAL has no submit/wait/completed (its mock backend never
//! kicks), so the rig installs a table of its own while holding a
//! [`TestGl`](crate::testutil::TestGl) guard: that serialises these tests with
//! every other test that touches the process-global OSAL pointer, and puts
//! testutil's table back before the guard is dropped.

use super::super::ds_cache;
use super::*;
use crate::ds_retention::{self, Plane};
use crate::osal::OsalOps;
use crate::shaderc::{ShcVivIo, SHC_VIV_CONST_LITERAL, SHC_VIV_CONST_TEXTURE_DEPTH, SHC_VIV_CONST_UNIFORM};
use crate::testutil::TestGl;
use crate::util::{EINVAL, ENOMEM};
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::vec::Vec;
use std::{boxed::Box, format, string::String, vec};

/* ---- the platform ------------------------------------------------------- */

struct Alloc {
    mem: Box<[u64]>,
    size: usize,
}

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Alloc>,
    bump: [u64; 3],
    fence: i64,
    /// One entry per submit: the command stream's words.
    kicks: Vec<Vec<u32>>,
    faults: Vec<String>,
    /// Fail the next allocation when non-zero, counting down.
    fail_alloc_in: u32,
    /// Every OSAL_QUERY_GC_FRAME_OPEN the backend sent, with the kicks
    /// submitted when it sent it: (open, kicks so far).
    frame_open: Vec<(u32, usize)>,
    /// OSAL_QUERY_CLOCK_NS: (the next answer, the step each answer adds);
    /// `None` answers -ENOTSUP, as an OSAL without a clock does.
    clock: Option<(u64, u64)>,
    /// Every OSAL_QUERY_DRAW_STATS report, in order.
    draw_stats: Vec<crate::draw_steps::OsalDrawStats>,
    /// Hand out addresses 64 bytes past a page, as the GC7000 OSAL's 64-byte
    /// alignment can (src/osal/zephyr_gc.c GC_OSAL_ALIGN); the reservation
    /// stays page-sized, so nothing else moves.
    off_page: bool,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

fn heap_slot(base: u64) -> Option<usize> {
    match base {
        GC_HEAP_DATA => Some(0),
        GC_HEAP_SHADER => Some(1),
        GC_HEAP_COMMAND => Some(2),
        _ => None,
    }
}

unsafe extern "C" fn p_gpu_alloc(heap_base: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let Some(h) = heap_slot(heap_base) else { return -EINVAL };
    with_platform(|p| {
        if p.fail_alloc_in > 0 {
            p.fail_alloc_in -= 1;
            if p.fail_alloc_in == 0 {
                return -ENOMEM;
            }
        }
        if size == 0 || size > 1 << 30 {
            return -ENOMEM;
        }
        let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
        /* page-aligned, as a real allocator's are: the frame's command buffer
         * relies on an even capacity for its closing pad word */
        let skew = if p.off_page { 64 } else { 0 };
        let gpu = heap_base + 0x1000 + p.bump[h] + skew;
        p.bump[h] += (size as u64 + skew).div_ceil(4096) * 4096;
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
        if m.impl_.is_null() {
            return;
        }
        if p.allocs.remove(&m.gpu).is_none() {
            p.faults.push(format!("gpu_free of 0x{:x}, which is no live allocation", m.gpu));
        }
    });
    // SAFETY: as above; a freed descriptor is cleared (the OSAL contract).
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}

unsafe extern "C" fn p_cache_flush(cpu: *mut c_void, n: usize) {
    with_platform(|p| {
        let inside = p.allocs.values().any(|a| {
            let base = a.mem.as_ptr() as usize;
            (cpu as usize) >= base && cpu as usize + n <= base + a.size
        });
        if !inside && n != 0 {
            p.faults.push(format!("cache_flush of {n} bytes at {cpu:p} outside every allocation"));
        }
    });
}

unsafe extern "C" fn p_cache_invd(_cpu: *mut c_void, _n: usize) {}

unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<GcKick>() {
        with_platform(|p| p.faults.push(format!("submit of a {len}-byte descriptor")));
        return -(EINVAL as i64);
    }
    // SAFETY: the backend passes a live GcKick of that size.
    let k = unsafe { *(kick as *const GcKick) };
    with_platform(|p| {
        if k.magic != GC_KICK_MAGIC {
            p.faults.push(format!("submit with magic 0x{:x}", k.magic));
            return -(EINVAL as i64);
        }
        if k.stream_bytes % 8 != 0 {
            p.faults.push(format!("a {}-byte stream: the front end fetches 64-bit words", k.stream_bytes));
        }
        let Some(a) = p.allocs.get(&k.stream_gpu) else {
            p.faults.push(format!("the kick's stream 0x{:x} is no allocation", k.stream_gpu));
            return -(EINVAL as i64);
        };
        if k.stream_bytes as usize + GC_KICK_TAIL_BYTES as usize > a.size {
            p.faults.push(format!("a {}-byte stream leaves the OSAL no tail", k.stream_bytes));
        }
        // SAFETY: the allocation holds at least `stream_bytes` bytes (checked).
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
    if what == crate::osal::OSAL_QUERY_CLOCK_NS && !out.is_null() && out_sz == 8 {
        return match with_platform(|p| p.clock.as_mut().map(|c| (c.0, c.0 += c.1).0)) {
            Some(ns) => {
                // SAFETY: a u64 out-parameter of the size given.
                unsafe { *(out as *mut u64) = ns };
                0
            }
            None => -crate::util::ENOTSUP,
        };
    }
    if what == crate::osal::OSAL_QUERY_DRAW_STATS {
        if out.is_null() || out_sz != core::mem::size_of::<crate::draw_steps::OsalDrawStats>() {
            with_platform(|p| p.faults.push(format!("a {out_sz}-byte draw stats report")));
            return -EINVAL;
        }
        // SAFETY: the library's report of the size checked above.
        let d = unsafe { *(out as *const crate::draw_steps::OsalDrawStats) };
        with_platform(|p| p.draw_stats.push(d));
        return 0;
    }
    if what != crate::osal::OSAL_QUERY_GC_FRAME_OPEN || out.is_null() || out_sz != 4 {
        return -EINVAL;
    }
    // SAFETY: the backend passes a u32 of the size given.
    let v = unsafe { *(out as *const u32) };
    with_platform(|p| {
        let kicks = p.kicks.len();
        p.frame_open.push((v, kicks))
    });
    0
}

#[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
extern "C" {
    /// testutil's log trampoline (a variadic function to Rust): the lines land
    /// in the TestGl recorder.
    fn yakogl_test_log(fmt: *const core::ffi::c_char, ...);
}
#[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
const RIG_LOG: Option<unsafe extern "C" fn(*const core::ffi::c_char, ...)> = Some(yakogl_test_log);
#[cfg(not(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64"))))]
const RIG_LOG: Option<unsafe extern "C" fn(*const core::ffi::c_char, ...)> = None;

static RIG_OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(p_gpu_alloc),
    gpu_free: Some(p_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(p_cache_flush),
    cache_invd: Some(p_cache_invd),
    submit: Some(p_submit),
    wait: Some(p_wait),
    completed: Some(p_completed),
    reg_read: None,
    reg_write: None,
    query: Some(p_query),
    log: RIG_LOG,
    wait_vblank: None,
    lock: None,
    unlock: None,
    thread_id: None,
};

/* ---- the rig ------------------------------------------------------------ */

/// One submitted command stream, decoded.
struct Stream(Vec<u32>);

/// Walk the commands, calling `f(reg, value, word index)` for every register a
/// LOAD_STATE writes.
fn walk(words: &[u32], mut f: impl FnMut(u32, u32, usize)) {
    let mut i = 0usize;

    while i < words.len() {
        let w = words[i];
        if w & 0xf800_0000 == GC_OP_LOAD_STATE {
            let count = ((w >> 16) & 0x3ff) as usize;
            let base = (w & 0xffff) * 4;
            for k in 0..count {
                if let Some(v) = words.get(i + 1 + k) {
                    f(base + 4 * k as u32, *v, i);
                }
            }
            i += 1 + count;
            continue;
        }
        i += match w & 0xf800_0000 {
            GC_OP_DRAW_INSTANCED => 4,
            GC_OP_STALL => 2,
            _ => 1, /* NOP and the pad words */
        };
    }
}

impl Stream {
    /// The last value written to `reg`, and how many times it was written.
    fn writes(&self, reg: u32) -> (Option<u32>, usize) {
        let (mut last, mut n) = (None, 0);

        walk(&self.0, |r, v, _| {
            if r == reg {
                last = Some(v);
                n += 1;
            }
        });
        (last, n)
    }

    /// The last value written to `reg`.
    fn reg(&self, reg: u32) -> u32 {
        self.writes(reg).0.unwrap_or_else(|| panic!("register 0x{reg:x} was never written"))
    }

    /// Where in the stream `reg` was last written.
    fn at(&self, reg: u32) -> usize {
        let mut at = None;

        walk(&self.0, |r, _, i| {
            if r == reg {
                at = Some(i);
            }
        });
        at.unwrap_or_else(|| panic!("register 0x{reg:x} was never written"))
    }

    /// The DRAW_INSTANCED commands: (indexed, prim, instances, vertices, first).
    fn draws(&self) -> Vec<(bool, u32, u32, u32, u32)> {
        let mut out = Vec::new();
        let mut i = 0usize;

        while i < self.0.len() {
            let w = self.0[i];
            if w & 0xf800_0000 == GC_OP_DRAW_INSTANCED {
                let count = self.0[i + 1];
                out.push((
                    w & 0x0010_0000 != 0,
                    (w >> 16) & 0xf,
                    (w & 0xffff) | ((count >> 24) << 16),
                    count & 0x00ff_ffff,
                    self.0[i + 2],
                ));
                i += 4;
                continue;
            }
            i += match w & 0xf800_0000 {
                GC_OP_LOAD_STATE => 1 + ((w >> 16) & 0x3ff) as usize,
                GC_OP_STALL => 2,
                _ => 1,
            };
        }
        out
    }

    /// The 256-byte texture descriptor the stream points slot `slot` at.
    fn descriptor(&self, slot: u32) -> Vec<u32> {
        let gpu = self.reg(nte_descriptor_addr(slot)) as u64;

        assert_ne!(gpu, 0, "no descriptor for slot {slot}");
        with_platform(|p| {
            let (base, a) = p.allocs.range(..=gpu).next_back().expect("an allocation");
            let off = (gpu - base) as usize;
            assert!(off + GC_TEXDESC_BYTES <= a.size, "the descriptor runs off the arena");
            // SAFETY: 256 bytes inside that allocation (checked just above).
            unsafe {
                core::slice::from_raw_parts((a.mem.as_ptr() as *const u8).add(off) as *const u32, GC_TEXDESC_BYTES / 4)
            }
            .to_vec()
        })
    }
}

struct Rig {
    prev_osal: *const OsalOps,
    _gl: TestGl,
    vs: Box<ShcShader>,
    fs: Box<ShcShader>,
    /// A buffer in the data heap, for the draws below.
    vbo: OsalGpuMem,
}

const TARGET_GPU: u64 = GC_HEAP_DATA + 0x10_0000;

impl Rig {
    fn new() -> Rig {
        let gl = TestGl::new(4, 4);
        *PLATFORM.lock().unwrap_or_else(|e| e.into_inner()) = Some(Platform::default());
        // SAFETY: tests are serialised by the TestGl guard.
        let prev_osal = unsafe {
            let prev = crate::osal::g_osal;
            crate::osal::osal_set_ops(&RIG_OSAL);
            prev
        };
        gc_shutdown();
        assert_eq!(gc_init(), 0);

        /* a vertex shader reading one attribute into register 0, writing
         * position from register 1 and one varying from register 2, and a
         * fragment shader that reads that varying */
        let mut vs = ShcShader::ZERO;
        vs.temps = 2;
        vs.viv.num_in = 1;
        vs.viv.in_[0] = ShcVivIo { reg: 0, slot: 0, components: 4, interp: 0, name: [0; 32] };
        vs.viv.num_out = 2;
        vs.viv.out[0] = ShcVivIo { reg: 1, slot: 0, components: 4, interp: 0, name: [0; 32] };
        vs.viv.out[1] = ShcVivIo { reg: 2, slot: 1, components: 4, interp: 0, name: [0; 32] };
        vs.viv.vs_pos_out_reg = 1;
        let mut fs = ShcShader::ZERO;
        fs.temps = 3;
        fs.viv.num_in = 1;
        fs.viv.in_[0] = ShcVivIo { reg: 1, slot: 1, components: 4, interp: 0, name: [0; 32] };
        fs.viv.vs_reg_of_in[0] = 2;
        fs.viv.num_out = 1;
        fs.viv.ps_color_out_reg = 3;

        let mut vbo = OsalGpuMem::ZERO;
        assert_eq!(osal::gpu_alloc(GC_HEAP_DATA, 4096, OSAL_MEM_DATA, &mut vbo), 0);
        Rig { prev_osal, _gl: gl, vs: Box::new(vs), fs: Box::new(fs), vbo }
    }

    /// A 64x64 RGBA8888 target, one vec4 attribute out of the rig's buffer,
    /// `count` sequential vertices as a triangle list.
    fn draw(&self, count: u32) -> GlesDraw {
        let mut d = GlesDraw::ZERO;
        d.target_gpu = TARGET_GPU;
        d.target_w = 64;
        d.target_h = 64;
        d.target_format = GLES_SURF_RGBA8888;
        /* slot 0 mirrors the target, as the GL core builds it (backend.h) */
        d.targets[0] = GlesDrawTarget { gpu: TARGET_GPU, w: 64, h: 64, format: GLES_SURF_RGBA8888 };
        d.viewport = [0, 0, 64, 64];
        d.depth_range = [0.0, 1.0];
        d.color_mask = 0xf;
        d.color_write = true;
        d.line_width = 1.0;
        d.prim = GLES_PRIM_TRIANGLES;
        d.index_count = count;
        d.vs = &*self.vs;
        d.fs = &*self.fs;
        d.vs_usc_gpu = GC_HEAP_SHADER + 0x1000;
        d.fs_usc_gpu = GC_HEAP_SHADER + 0x2000;
        d.num_attribs = 1;
        d.attribs[0] = GlesAttrib {
            vi_reg: 0,
            components: 4,
            format: GLES_ATTRIBF_FLOAT32,
            normalized: 0,
            stride: 16,
            offset: 0,
            buffer_gpu: self.vbo.gpu,
            cpu: core::ptr::null(),
            cpu_bytes: 0,
            per_instance: 0,
            divisor: 0,
        };
        d
    }

    fn submit(&self, d: &GlesDraw) -> Stream {
        // SAFETY: `d` is a live draw for the length of the call.
        assert_eq!(unsafe { gc_draw(d) }, 0);
        assert_eq!(gc_flush(), 0);
        self.last_stream()
    }

    fn last_stream(&self) -> Stream {
        with_platform(|p| {
            assert!(p.faults.is_empty(), "{:?}", p.faults);
            Stream(p.kicks.last().cloned().expect("a submitted stream"))
        })
    }

    fn kicks(&self) -> usize {
        with_platform(|p| p.kicks.len())
    }

    fn faults(&self) -> Vec<String> {
        with_platform(|p| core::mem::take(&mut p.faults))
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        gc_shutdown();
        // SAFETY: serialised by the guard; the previous table is a static.
        unsafe { crate::osal::osal_set_ops(self.prev_osal) };
        *PLATFORM.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// The lines a run logged, and whether any repeats: the per-draw log rule
/// (osal `glog_once!`): at most one line per cause, however many draws.
fn repeated_lines(lines: &[String]) -> Vec<(String, usize)> {
    let mut seen: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for l in lines {
        *seen.entry(l.as_str()).or_default() += 1;
    }
    seen.into_iter().filter(|(_, n)| *n > 1).map(|(l, n)| (l.to_string(), n)).collect()
}

/// 100 draws the backend refuses (a vertex shader with more inputs than
/// the draw binds elements) log at most one line per cause, the repeats
/// counted: the per-draw log rule (osal `glog_once!`).
#[test]
fn a_draw_refused_100_times_logs_each_cause_once() {
    let rig = Rig::new();
    let mut vs = *rig.vs;
    vs.viv.num_in = 2;
    vs.viv.in_[1] = ShcVivIo { reg: 3, slot: 1, components: 4, interp: 0, name: [0; 32] };
    let mut d = rig.draw(3);
    d.vs = &vs;
    rig._gl.logs();
    let (_, repeats0) = crate::osal::log_repeats();
    for _ in 0..100 {
        // SAFETY: a live draw for the length of the call.
        assert!(unsafe { gc_draw(&d) } < 0, "refused");
    }
    let _ = gc_flush();
    let lines = rig._gl.logs();
    assert_eq!(repeated_lines(&lines), [], "{lines:?}");
    assert!(lines.len() <= 3, "a few causes, not a line per draw: {lines:?}");
    assert!(crate::osal::log_repeats().1.wrapping_sub(repeats0) >= 99, "the refusal ran 100 times: {lines:?}");
}

/* ---- the constant file ------------------------------------------------- */

/// A constant that names a uniform dword past its stage's block reads 0 and
/// is logged once, not on every draw: a line per draw was 2.7 s of the
/// i.MX8MP cluster's frame 2 (the online compiler described a register's
/// padding lanes that way; tests/gc_const_map.rs pins the compiler side).
/// At most one line in 100 draws, and the other draws counted as repeats
/// (which shows the line's site was reached: zero lines alone would not).
#[test]
fn a_constant_past_the_block_is_logged_once_not_per_draw() {
    static CONTENTS: [u8; 4] = [
        crate::shaderc::SHC_VIV_CONST_UNIFORM,
        crate::shaderc::SHC_VIV_CONST_UNIFORM,
        0,
        crate::shaderc::SHC_VIV_CONST_UNIFORM,
    ];
    static DATA: [u32; 4] = [0, 1, 0, 7];
    let rig = Rig::new();
    let mut fs = *rig.fs;
    fs.viv.const_contents = CONTENTS.as_ptr();
    fs.viv.const_data = DATA.as_ptr();
    fs.viv.const_count = 4;
    fs.uniform_dwords = 2;
    let block = [1.0f32.to_bits(), 2.0f32.to_bits()];
    let mut d = rig.draw(3);
    d.fs = &fs;
    d.fs_uniform_cpu = block.as_ptr() as _;
    d.fs_uniform_dwords = 2;
    rig._gl.logs();
    let (_, repeats0) = crate::osal::log_repeats();
    for _ in 0..100 {
        // SAFETY: a live draw for the length of the call.
        assert_eq!(unsafe { gc_draw(&d) }, 0);
    }
    assert_eq!(gc_flush(), 0);
    let lines: Vec<String> = rig._gl.logs().into_iter().filter(|l| l.contains("uniform dword")).collect();
    let (_, repeats) = crate::osal::log_repeats();
    assert!(lines.len() <= 1, "{lines:?}");
    assert!(repeats.wrapping_sub(repeats0) >= 99, "the line's site was reached 100 times: {}", repeats - repeats0);
    if let Some(l) = lines.first() {
        assert!(l.contains("fragment uniform dword 7 missing, block holds 2"), "{lines:?}");
    }
}

/* ---- the draw's steps (draw_steps.rs) ------------------------------------ */

/// The draw-stats reports the OSAL received since the last call.
fn draw_stats_taken() -> Vec<crate::draw_steps::OsalDrawStats> {
    with_platform(|p| core::mem::take(&mut p.draw_stats))
}

/// Each part of a GC7000 draw is counted under its step and the draw is
/// reported to the OSAL once: with a clock that moves 1000 ns a read, every
/// lap is 1000 ns, so a step's time is its laps. The i.MX8MP cluster's
/// frame 2 spent 2.75 s inside glDraw* with about none of it on the GPU
/// (2026-10-05); this is the instrument that names the step.
#[test]
fn a_gc_draw_reports_each_of_its_parts_under_its_step() {
    use crate::draw_steps::Step::*;
    let rig = Rig::new();
    with_platform(|p| p.clock = Some((1_000_000, 1000)));
    let d = rig.draw(3);
    draw_stats_taken();
    // SAFETY: a live draw for the length of the call.
    assert_eq!(unsafe { gc_draw(&d) }, 0);
    let got = draw_stats_taken();
    assert_eq!(got.len(), 1, "one report for one draw: {got:?}");
    let r = got[0];
    assert_eq!(r.untimed, 0);
    /* GcRest: before the surface, before the tiled copies, at the end;
     * GcEmit: the (absent) clear and the draw; GcUniforms: the blocks and
     * the UBO copies; GcVertex: the vertex input and the indices */
    for (s, laps) in
        [(GcRest, 3), (GcDs, 1), (GcTiledCopy, 1), (GcFrame, 1), (GcUniforms, 2), (GcVertex, 2), (GcEmit, 2)]
    {
        assert_eq!(r.ns[s as usize], laps * 1000, "{s:?}: {r:?}");
    }
    for s in [GlProgram, GlUniforms, GlAttribs, GlTextures, GlState] {
        assert_eq!(r.ns[s as usize], 0, "{s:?}: the GL core did not run: {r:?}");
    }
    assert_eq!(gc_flush(), 0);
}

/// A step's time is its own: a draw timed inside another timer's step is
/// taken out of that step, and the scope is reported once, by the
/// outermost timer (a pending clear the GL core draws on its own nests
/// that way).
#[test]
fn a_nested_draw_is_reported_once_and_not_counted_twice() {
    use crate::draw_steps::{Step, Timer};
    let rig = Rig::new();
    with_platform(|p| p.clock = Some((1_000_000, 1000)));
    let d = rig.draw(3);
    draw_stats_taken();
    {
        let mut outer = Timer::start();
        // SAFETY: a live draw for the length of the call.
        assert_eq!(unsafe { gc_draw(&d) }, 0);
        assert_eq!(draw_stats_taken(), [], "the nested draw does not report on its own");
        outer.lap(Step::GlState);
    }
    let got = draw_stats_taken();
    assert_eq!(got.len(), 1, "{got:?}");
    /* after the outer start the clock is read 14 times: the draw's start,
     * its 12 laps, and the outer lap; the draw's own 12 laps are 12000 ns
     * of the 14000 the outer lap spans */
    assert_eq!(got[0].ns[Step::GlState as usize], 2000, "{got:?}");
    assert_eq!(got[0].ns.iter().sum::<u64>(), 14 * 1000, "the outer span counted once: {got:?}");
    assert_eq!(gc_flush(), 0);
}

/// Without a clock nothing is measured: the report carries no time and
/// counts every lap as untimed, so the reader can tell it from zero.
#[test]
fn without_a_clock_every_lap_is_reported_untimed() {
    let rig = Rig::new();
    with_platform(|p| p.clock = None);
    draw_stats_taken();
    // SAFETY: a live draw for the length of the call.
    assert_eq!(unsafe { gc_draw(&rig.draw(3)) }, 0);
    let got = draw_stats_taken();
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!((got[0].ns, got[0].untimed), ([0; crate::draw_steps::STEPS], 12), "every lap untimed");
    assert_eq!(gc_flush(), 0);
}

/* ---- the frame arena ---------------------------------------------------- */

/// A draw whose client arrays would not fit what is left of the frame arena
/// submits the frame first and goes into the next one, instead of being
/// dropped: dEQP texture.vertex's 127 x 127 grids (~450 KB of client
/// arrays and indices each, several per frame) came back as the clear
/// colour on the i.MX8MP (2085847f).
/// The backend tells the OSAL when it records a frame
/// (OSAL_QUERY_GC_FRAME_OPEN): 1 when a draw opens one, 0 when it is
/// submitted, and nothing for a flush or finish with no frame. Without it
/// the Zephyr OSAL held every free for the next kick, which does not exist
/// until something draws again: dEQP texture.size's next case uploads
/// before its first draw, and the i.MX8MP ran out of the carveout with
/// 32 MiB held that way (tools/cts/RESULTS.md).
#[test]
fn the_backend_tells_the_osal_when_a_frame_is_open() {
    let rig = Rig::new();
    let taken = |p: &mut Platform| core::mem::take(&mut p.frame_open);
    with_platform(taken);
    let k0 = rig.kicks();
    // SAFETY: a live draw for the length of the call.
    assert_eq!(unsafe { gc_draw(&rig.draw(3)) }, 0);
    // SAFETY: as above.
    assert_eq!(unsafe { gc_draw(&rig.draw(3)) }, 0);
    assert_eq!(with_platform(taken), [(1, k0)], "one frame opened by the first draw, none by the second");
    assert_eq!(gc_flush(), 0);
    assert_eq!(with_platform(taken), [(0, k0)], "closed as it is submitted");
    assert_eq!(rig.kicks(), k0 + 1);
    assert_eq!(gc_flush(), 0);
    assert_eq!(gc_finish(), 0);
    assert_eq!(with_platform(taken), [], "no frame, nothing to say");
    let _ = rig.submit(&rig.draw(3));
    assert_eq!(with_platform(taken), [(1, k0 + 1), (0, k0 + 1)]);
}

/// One instance of an instanced draw the GL core draws instance by instance
/// (GlesDraw::instance_id, a divisor past 1): the FE starts the instance ID
/// at 0 for every draw, so a program that reads gl_InstanceID is refused
/// (and counted, gc_refused_draws), not drawn with 0. Instance 0, and a
/// program that does not read it, are drawn. Revert check: without the
/// refusal the first draw goes in.
#[test]
fn an_instance_drawn_on_its_own_that_reads_gl_instance_id_is_refused() {
    let rig = Rig::new();
    let mut vs = *rig.vs;
    vs.vs_instance_id_reg1 = 3;
    let base = gc_refused_draws();
    let mut d = rig.draw(3);
    d.vs = &vs;
    d.instance_id = 2;
    // SAFETY: a live draw.
    assert_ne!(unsafe { gc_draw(&d) }, 0, "reads gl_InstanceID, instance 2");
    d.instance_id = 0;
    // SAFETY: a live draw.
    assert_eq!(unsafe { gc_draw(&d) }, 0, "instance 0");
    let mut e = rig.draw(3);
    e.instance_id = 2;
    // SAFETY: a live draw.
    assert_eq!(unsafe { gc_draw(&e) }, 0, "does not read gl_InstanceID");
    assert_eq!(gc_refused_draws().wrapping_sub(base), 1);
}

/// ES 3.0 lets colour attachments differ in size (4.4.4.2); the render area
/// is their intersection. The pixel engine clips to RT 0 alone, so a later
/// target smaller than RT 0 in either dimension is refused by name (review
/// R-02) and counted; one at least as large is drawn. Revert check: without
/// the refusal the first draw goes in.
#[test]
fn a_later_target_smaller_than_the_first_is_refused() {
    let rig = Rig::new();
    let base = gc_refused_draws();
    let mut d = rig.draw(3);
    d.num_targets = 2;
    d.targets[1] = GlesDrawTarget { gpu: TARGET_GPU + 0x10_0000, w: 64, h: 32, format: d.target_format };
    // SAFETY: a live draw.
    let smaller = unsafe { gc_draw(&d) };
    d.targets[1].h = 64;
    // SAFETY: a live draw.
    let same = unsafe { gc_draw(&d) };
    d.targets[1].w = 128;
    // SAFETY: a live draw.
    let larger = unsafe { gc_draw(&d) };
    assert_eq!((smaller, same, larger), (-crate::util::ENOTSUP, 0, 0));
    assert_eq!(gc_refused_draws().wrapping_sub(base), 1);
    let logs = rig._gl.logs();
    assert!(logs.iter().any(|l| l.contains("smaller than the first")), "{logs:?}");
}

/// glDrawBuffers({GL_NONE, GL_COLOR_ATTACHMENT1}) (review R-09): hardware
/// target 0 is always slot 0 (hw_targets), so a draw whose slot 0 the GL
/// core left without a target is refused by name, counted, and not drawn
/// into attachment 0; with slot 0 written it is drawn. Revert check:
/// without the refusal the first draw goes in.
#[test]
fn a_draw_whose_draw_buffer_0_is_none_is_refused_by_name() {
    let rig = Rig::new();
    let base = gc_refused_draws();
    let mut d = rig.draw(3);
    d.num_targets = 2;
    d.targets[0] = GlesDrawTarget { gpu: 0, w: d.target_w, h: d.target_h, format: d.target_format };
    d.targets[1] = GlesDrawTarget { gpu: d.target_gpu, w: d.target_w, h: d.target_h, format: d.target_format };
    // SAFETY: a live draw.
    assert_eq!(unsafe { gc_draw(&d) }, -crate::util::ENOTSUP);
    assert_eq!(gc_refused_draws().wrapping_sub(base), 1);
    let logs = rig._gl.logs();
    assert!(logs.iter().any(|l| l.contains("draw buffer 0 is GL_NONE")), "{logs:?}");
    d.num_targets = 1;
    d.targets[0].gpu = d.target_gpu;
    // SAFETY: a live draw.
    assert_eq!(unsafe { gc_draw(&d) }, 0);
}

#[test]
fn a_draw_that_does_not_fit_the_arena_goes_into_the_next_frame() {
    let rig = Rig::new();
    let big = vec![0u8; 400 * 1024];
    let mut d = rig.draw(3);
    d.attribs[0].buffer_gpu = 0;
    d.attribs[0].cpu = big.as_ptr() as *const c_void;
    d.attribs[0].cpu_bytes = big.len() as u32;
    let before = rig.kicks();
    let refused0 = gc_refused_draws();
    for k in 0..3 {
        // SAFETY: `d` and the array it points at live across the call.
        assert_eq!(unsafe { gc_draw(&d) }, 0, "draw {k}: accepted, into the next frame if need be");
    }
    assert!(rig.kicks() > before, "the third 400 KB draw ended the frame before it (1 MiB arena)");
    assert_eq!(gc_refused_draws(), refused0, "the three accepted draws are not counted as refused");
    assert_eq!(gc_flush(), 0);
    assert!(rig.faults().is_empty());
}

/// A draw whose copies are larger than an EMPTY arena gets a frame of its
/// own with memory of its own for them, and is drawn: dEQP
/// texture.vertex.2d draws 127 x 127 quads of four vertices each from client
/// arrays, 1,225,804 bytes, and all 45 cases came back black on the
/// i.MX8MP because it was refused with GL_OUT_OF_MEMORY (gl_test 129's grid
/// pass, check 12, ed71ce69). Only a pool with no such block refuses it, and
/// that is counted.
#[test]
fn a_draw_larger_than_the_arena_gets_memory_of_its_own() {
    let rig = Rig::new();
    let small = vec![7u8; 4096];
    let mut s = rig.draw(3);
    s.attribs[0].buffer_gpu = 0;
    s.attribs[0].cpu = small.as_ptr() as *const c_void;
    s.attribs[0].cpu_bytes = small.len() as u32;
    // SAFETY: `s` and its array live across the call.
    assert_eq!(unsafe { gc_draw(&s) }, 0, "a small draw opens the frame");

    let huge: Vec<u8> = (0..1100 * 1024).map(|i| (i * 7 + 3) as u8).collect();
    let mut d = rig.draw(3);
    d.attribs[0].buffer_gpu = 0;
    d.attribs[0].cpu = huge.as_ptr() as *const c_void;
    d.attribs[0].cpu_bytes = huge.len() as u32;
    let before = rig.kicks();
    let refused0 = gc_refused_draws();
    // SAFETY: as above.
    assert_eq!(unsafe { gc_draw(&d) }, 0, "larger than an empty arena: drawn");
    assert_eq!(rig.kicks(), before + 1, "the frame before it was submitted: it has a frame of its own");
    assert_eq!(gc_flush(), 0);
    let st = rig.last_stream();
    assert_eq!(st.draws().len(), 1);
    let at = st.reg(nfe_vertex_streams_base_addr(0)) as u64;
    assert_eq!(gpu_bytes(at, huge.len()), huge, "the whole array, copied intact outside the arena");
    assert_eq!(gc_refused_draws(), refused0, "not refused");

    /* the pool cannot give such a block: refused, counted, logged */
    let bigger: Vec<u8> = vec![1u8; 3 * 1024 * 1024];
    d.attribs[0].cpu = bigger.as_ptr() as *const c_void;
    d.attribs[0].cpu_bytes = bigger.len() as u32;
    with_platform(|p| p.fail_alloc_in = 1);
    // SAFETY: as above.
    assert_eq!(unsafe { gc_draw(&d) }, -ENOMEM, "no memory for it: refused");
    assert_eq!(gc_refused_draws(), refused0 + 1, "and counted: a dropped draw is never silent");
    with_platform(|p| p.fail_alloc_in = 0);
    assert_eq!(gc_flush(), 0);
    assert!(rig.faults().is_empty());
}

/* ---- the command writer ------------------------------------------------- */

/// Does every command of `words` end inside it? A LOAD_STATE cut off by a
/// full buffer claims words the stream does not have.
fn whole_commands(words: &[u32]) -> bool {
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        i += match w & 0xf800_0000 {
            GC_OP_LOAD_STATE => 1 + ((w >> 16) & 0x3ff) as usize,
            GC_OP_DRAW_INSTANCED => 4,
            GC_OP_STALL => 2,
            _ => 1,
        };
    }
    i == words.len()
}

/// A frame whose command buffer fills up mid-draw takes that draw back out
/// and records it again at the start of the next frame. Before, the writer
/// only noted the overflow: the draw was lost with nothing counting it, and
/// its last LOAD_STATE could end without its payload (dEQP
/// color_clear.long_masked_rgba: up to 500 masked, scissored clears in one
/// frame, and only its alpha channel showed the lost one).
#[test]
fn a_draw_that_does_not_fit_the_command_buffer_goes_into_the_next_frame() {
    let rig = Rig::new();
    let d = rig.draw(3);
    let one = rig.submit(&d).0.len();
    let n = 3 * GC_CMD_WORDS / one + 1;
    for _ in 0..n {
        // SAFETY: `d` is a live draw for the length of the call.
        assert_eq!(unsafe { gc_draw(&d) }, 0);
    }
    assert_eq!(gc_flush(), 0);
    assert!(rig.faults().is_empty());
    let kicks = with_platform(|p| p.kicks.clone());
    assert!(kicks.len() > 2, "{n} draws of {one} words fill more than one frame");
    let drawn: usize = kicks[1..].iter().map(|k| Stream(k.clone()).draws().len()).sum();
    assert_eq!(drawn, n, "every draw is in some frame, none lost to a full buffer");
    for (i, k) in kicks.iter().enumerate() {
        assert!(whole_commands(k), "kick {i}: a command runs past the end of the stream");
    }
}

#[test]
fn a_load_state_carries_its_payload_and_a_pad_to_the_64_bit_boundary() {
    let mut words = [0u32; 16];
    let mut c = GcCmd { words: words.as_mut_ptr(), used: 0, cap: 16, overflow: false };

    /* the front end fetches 64-bit words, so a command is an even number of
     * them: a header and one value already are, a header and two values need
     * a pad */
    c.state(0x1400, 0xdead_beef);
    assert_eq!(c.used, 2);
    assert_eq!(words[0], GC_OP_LOAD_STATE | (1 << 16) | (0x1400 >> 2));
    assert_eq!(words[1], 0xdead_beef);

    c.state_multi(0x1400, &[1, 2]);
    assert_eq!(c.used, 6);
    assert_eq!(words[2], GC_OP_LOAD_STATE | (2 << 16) | (0x1400 >> 2));
    assert_eq!((words[3], words[4], words[5]), (1, 2, GC_OP_NOP));

    /* the fixed-point bit is the one difference of a viewport register */
    c.state_fixp(0x0a00, 0x10000);
    assert_ne!(words[6] & 0x0400_0000, 0);
}

#[test]
fn a_writer_that_runs_out_of_room_says_so_and_writes_nothing_more() {
    let mut words = [0u32; 4];
    let mut c = GcCmd { words: words.as_mut_ptr(), used: 0, cap: 4, overflow: false };

    c.state(0x1400, 1); /* 2 words */
    assert!(!c.overflow);
    c.state_multi(0x1400, &[1, 2]); /* 4 more: no room */
    assert!(c.overflow);
    assert_eq!(c.used, 2, "the command was dropped whole, not half-written");
}

/* ---- frames ------------------------------------------------------------- */

#[test]
fn a_frame_holds_every_draw_to_one_target_and_a_new_target_closes_it() {
    let rig = Rig::new();
    let d = rig.draw(3);

    // SAFETY: `d` is live.
    unsafe {
        assert_eq!(gc_draw(&d), 0);
        assert_eq!(gc_draw(&d), 0);
    }
    assert_eq!(rig.kicks(), 0, "nothing is submitted until the frame closes");

    let mut other = rig.draw(3);
    other.target_gpu = TARGET_GPU + 0x10_0000;
    // SAFETY: `other` is live.
    assert_eq!(unsafe { gc_draw(&other) }, 0);
    assert_eq!(rig.kicks(), 1, "a new target ended the first frame");
    assert_eq!(rig.last_stream().draws().len(), 2, "both draws are in it");
    assert_eq!(gc_flush(), 0);
    assert_eq!(rig.kicks(), 2);
    assert!(rig.faults().is_empty());
}

#[test]
fn a_fresh_stream_resets_the_gpu_state_before_its_first_draw() {
    let rig = Rig::new();
    let s = rig.submit(&rig.draw(3));

    assert_eq!(s.reg(GC_GL_PIPE_SELECT), GC_PIPE_3D);
    assert_eq!(s.reg(GC_GL_API_MODE), GC_API_MODE_OPENGL);
    assert_eq!(s.reg(GC_NTE_DESCRIPTOR_CONTROL), GC_NTE_DESCRIPTOR_ENABLE);
    assert_eq!(s.reg(GC_VS_ICACHE_INVALIDATE), GC_VS_ICACHE_INVALIDATE_ALL);
    /* the whole attribute array is cleared: some parts come out of reset with
     * attributes enabled, and the front end needs the edge to disable them */
    assert_eq!(s.writes(nfe_generic_attrib_config0(31)).0, Some(0));
}

#[test]
fn the_draw_command_carries_the_primitive_and_the_counts() {
    let rig = Rig::new();
    let mut d = rig.draw(9);
    d.prim = GLES_PRIM_TRI_STRIP;
    d.instance_count = 4;

    assert_eq!(rig.submit(&d).draws(), [(false, GC_PRIM_TRIANGLE_STRIP, 4, 9, 0)]);
}

#[test]
fn an_indexed_draw_names_its_index_buffer_and_width() {
    let rig = Rig::new();
    let mut d = rig.draw(6);
    d.index_buf_gpu = rig.vbo.gpu + 0x800;
    d.index_size = 2;
    let s = rig.submit(&d);

    assert_eq!(s.reg(GC_FE_INDEX_STREAM_BASE_ADDR), (rig.vbo.gpu + 0x800) as u32);
    assert_eq!(s.reg(GC_FE_INDEX_STREAM_CONTROL), GC_INDEX_TYPE_U16);
    assert!(s.draws()[0].0, "the header says indexed");
}

#[test]
fn staged_indices_and_attributes_are_snapshotted_into_the_frame_arena() {
    /* the GL core's staging is reused by the next draw, and this frame's
     * stream is still pending */
    let rig = Rig::new();
    let idx: [u16; 3] = [0, 1, 2];
    let verts = [1.0f32; 8];
    let mut d = rig.draw(3);
    d.index_cpu = idx.as_ptr() as *const c_void;
    d.index_bytes = 6;
    d.attribs[0].cpu = verts.as_ptr() as *const c_void;
    d.attribs[0].cpu_bytes = 32;
    d.attribs[0].buffer_gpu = 0;
    let s = rig.submit(&d);

    let idx_gpu = s.reg(GC_FE_INDEX_STREAM_BASE_ADDR) as u64;
    let vtx_gpu = s.reg(nfe_vertex_streams_base_addr(0)) as u64;
    assert_ne!(idx_gpu, 0);
    assert_ne!(vtx_gpu, 0);
    assert_ne!(idx_gpu, vtx_gpu);
    with_platform(|p| {
        for gpu in [idx_gpu, vtx_gpu] {
            assert!(
                p.allocs.range(..=gpu).next_back().is_some_and(|(base, a)| gpu < base + a.size as u64),
                "0x{gpu:x} is in no allocation"
            );
        }
    });
}

/* ---- the state a draw derives ------------------------------------------- */

#[test]
fn the_colour_surface_is_the_targets_and_the_pixel_engine_is_told_its_stride() {
    let rig = Rig::new();
    let s = rig.submit(&rig.draw(3));

    assert_eq!(s.reg(pe_pipe_color_addr(0)), TARGET_GPU as u32);
    assert_eq!(s.reg(pe_pipe_color_addr(1)), TARGET_GPU as u32, "one pipe: both registers name it");
    assert_eq!(s.reg(GC_PE_COLOR_STRIDE), 64 * 4);
    /* an RGBA8888 target is written A8R8G8B8, and nothing masks or blends, so
     * the pixel engine may overwrite whole pixels */
    assert_eq!(s.reg(GC_PE_COLOR_FORMAT) & 0xf, GC_PE_FORMAT_A8R8G8B8);
    assert_ne!(s.reg(GC_PE_COLOR_FORMAT) & GC_PE_COLOR_FORMAT_OVERWRITE, 0);
    /* and the depth buffer the backend allocated for it, padded to 64x64 */
    assert_eq!(s.reg(GC_PE_DEPTH_STRIDE), 64 * 4);
}

/// A framebuffer with no colour attachment (glmark2's shadow pass, a depth
/// texture alone; `backend::DEPTH_ONLY_TARGETS`): the GL core sends
/// `target_gpu` 0 with colour writes off. The pixel engine still needs a
/// colour surface address, so it gets a small one of the backend's own with
/// no components and a stride of 0, as Mesa's etnaviv does for nr_cbufs == 0
/// (etnaviv_state.c:397-407, the 64x64 dummy of etnaviv_screen.c:1144-1151,
/// Mesa 26.2.1); the depth surface is the draw's as always.
#[test]
fn a_draw_with_no_colour_target_names_a_dummy_surface_and_writes_no_colour() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.target_gpu = 0;
    d.targets[0].gpu = 0;
    d.color_mask = 0;
    d.color_write = false;
    d.depth_test = true;
    d.depth_write = true;
    d.depth_id = 0x4000_0001;
    (d.ds_w, d.ds_h) = (64, 64);
    let s = rig.submit(&d);

    let addr = s.reg(pe_pipe_color_addr(0)) as u64;
    assert_ne!(addr, 0, "never address 0");
    assert_eq!(s.reg(pe_pipe_color_addr(1)) as u64, addr, "one pipe: both registers name it");
    with_platform(|p| {
        assert!(
            p.allocs.range(..=addr).next_back().is_some_and(|(base, a)| addr + 64 * 64 * 4 <= base + a.size as u64),
            "0x{addr:x}: the dummy surface is an allocation of at least 64x64x4 bytes"
        );
    });
    assert_eq!(s.reg(GC_PE_COLOR_STRIDE), 0);
    assert_eq!((s.reg(GC_PE_COLOR_FORMAT) >> 8) & 0xf, 0, "no components: no colour is written");
    assert_ne!(s.reg(pe_pipe_depth_addr(0)), 0, "the depth surface is the draw's");
    assert_eq!(s.draws().len(), 1, "the draw is recorded, not refused");
}

#[test]
fn an_odd_width_target_is_written_at_the_gl_cores_pair_padded_pitch() {
    /* dEQP fbo.render's NPOT targets are 127, 111 and 55 wide; the GL core
     * stores every colour surface at whole pixel pairs (tex_pitch), and the
     * pixel engine must write the rows there or each lands a pixel short */
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.target_w = 127;
    d.target_h = 95;
    d.viewport = [0, 0, 127, 95];
    let s = rig.submit(&d);
    assert_eq!(s.reg(GC_PE_COLOR_STRIDE), 128 * 4, "127 pixels stored as 128");
    /* an even width is its own pitch, as before */
    d.target_w = 64;
    d.target_h = 64;
    d.viewport = [0, 0, 64, 64];
    let s = rig.submit(&d);
    assert_eq!(s.reg(GC_PE_COLOR_STRIDE), 64 * 4);
}

/// A window that is a view into a wider scan-out buffer (an 800x600 window
/// in a 1280-pixel framebuffer, `GlesDraw::target_pitch`): the pixel engine
/// is told the view's pitch, read back from the submitted stream's
/// PE_COLOR_STRIDE, and a multisampled draw's resolve writes the target at
/// that pitch too (RS_DEST_STRIDE); the scratch surface keeps its own.
#[test]
fn a_view_window_puts_its_pitch_into_the_pixel_engines_and_the_resolves_stride() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    (d.target_w, d.target_h, d.target_pitch) = (800, 600, 1280);
    d.viewport = [0, 0, 800, 600];
    let s = rig.submit(&d);
    assert_eq!(s.reg(GC_PE_COLOR_STRIDE), 1280 * 4, "the view's pitch, not 800 * 4");

    /* 4x is a test knob (gc_debug_msaa) */
    #[cfg(feature = "test-knobs")]
    {
        d.samples = 4;
        gc_debug_set_msaa(1);
        let on = rig.submit(&d);
        gc_debug_set_msaa(0);
        let ms_pitch = (2 * 800u32).div_ceil(64) * 64 * 4;
        assert_eq!(on.reg(GC_PE_COLOR_STRIDE), ms_pitch, "the scratch surface's own pitch");
        assert_eq!(on.reg(GC_RS_DEST_STRIDE), 1280 * 4, "the resolve writes the view at its pitch");
    }

    /* the same target packed */
    d.samples = 1;
    d.target_pitch = 0;
    let p = rig.submit(&d);
    assert_eq!(p.reg(GC_PE_COLOR_STRIDE), 800 * 4);
}

/// PE_COLOR_FORMAT for a float target is etnaviv's word: FORMAT_EXT(fmt) with
/// FORMAT's write mask (bit 4), never FORMAT_EXT's (bit 31), then the colour
/// mask and OVERWRITE (etnaviv_state.c:281-291, merged with the blend state's
/// bits as etnaviv_emit.c:538-544 does). Bit 31 in place of bit 4 told the PE
/// to keep the FORMAT_EXT it had, so an RGBA16F target was drawn in some
/// other format (gl_test 44, #6). A 32-bit format goes through FORMAT, and
/// neither mask bit is set.
#[test]
fn a_float_target_goes_through_the_extended_format_field() {
    let rig = Rig::new();
    for (fmt, want, what) in [
        (GLES_SURF_RGBA16F, 0x1301_0f10u32, "RGBA16F: FORMAT_EXT 0x13"),
        (GLES_SURF_R16F, 0x1101_0f10, "R16F: FORMAT_EXT 0x11"),
        (GLES_SURF_R32F, 0x1401_0f10, "R32F: FORMAT_EXT 0x14"),
        (GLES_SURF_RGBA8888, 0x0001_0f06, "RGBA8888: FORMAT 6 (A8R8G8B8)"),
        (GLES_SURF_RGB565, 0x0001_0f04, "RGB565: FORMAT 4"),
    ] {
        let mut d = rig.draw(3);
        d.target_format = fmt;
        d.targets[0].format = fmt;
        let f = rig.submit(&d).reg(GC_PE_COLOR_FORMAT);
        assert_eq!(f, want, "{what}: 0x{f:08x}");
        assert_eq!(f & GC_PE_COLOR_FORMAT_FORMAT_EXT_MASK, 0, "{what}: FORMAT_EXT is written");
    }
    let mut d = rig.draw(3);
    d.target_format = GLES_SURF_RGBA16F;
    d.targets[0].format = GLES_SURF_RGBA16F;
    let f = rig.submit(&d).reg(GC_PE_COLOR_FORMAT);
    assert_eq!(f & 0xf, 0, "FORMAT itself is left at 0, under its mask");
    assert_eq!((f >> 24) & 0x7f, GC_PE_FORMAT_A16B16G16R16F);
}

/// The PE_COLOR_FORMAT bits regs.rs names are the vendored rnndb header's
/// (driver/src/vivante/hw/state_3d.xml.h, Mesa's src/etnaviv/hw): the bit-4
/// write mask was once written down as bit 31, which is FORMAT_EXT's own.
#[test]
fn the_pe_color_format_bits_are_the_rnndb_headers() {
    const XML: &str = include_str!("../../driver/src/vivante/hw/state_3d.xml.h");
    let define = |name: &str| -> u32 {
        let line = XML
            .lines()
            .find(|l| l.split_whitespace().nth(1) == Some(name))
            .unwrap_or_else(|| panic!("{name} is in state_3d.xml.h"));
        let v = line.split_whitespace().nth(2).expect("a value");
        u32::from_str_radix(v.trim_start_matches("0x"), 16).expect("hex")
    };
    assert_eq!(GC_PE_COLOR_FORMAT, define("VIVS_PE_COLOR_FORMAT"));
    assert_eq!(GC_PE_COLOR_FORMAT_FORMAT_MASK, define("VIVS_PE_COLOR_FORMAT_FORMAT_MASK"));
    assert_eq!(GC_PE_COLOR_FORMAT_FORMAT_EXT_MASK, define("VIVS_PE_COLOR_FORMAT_FORMAT_EXT_MASK"));
    assert_eq!(GC_PE_COLOR_FORMAT_OVERWRITE, define("VIVS_PE_COLOR_FORMAT_OVERWRITE"));
    assert_eq!(GC_PE_COLOR_FORMAT_SUPER_TILED, define("VIVS_PE_COLOR_FORMAT_SUPER_TILED"));
    assert_eq!(GC_PE_COLOR_FORMAT_SUPER_TILED_NEW, define("VIVS_PE_COLOR_FORMAT_SUPER_TILED_NEW"));
    assert_eq!(pe_color_format_format_ext(0x7f), define("VIVS_PE_COLOR_FORMAT_FORMAT_EXT__MASK"));
    assert_eq!(pe_color_format_components(0xf), define("VIVS_PE_COLOR_FORMAT_COMPONENTS__MASK"));
}

#[test]
fn a_32_bit_float_target_selects_the_uif32_colour_output_mode() {
    /* etnaviv's translate_output_mode (etnaviv_translate.h:446) tests the
     * component width BEFORE the pure-integer test, so a 32-bit float target
     * takes UIF32 and every narrower one NORMAL. This backend wrote NORMAL for
     * all of them until the state audit against etnaviv. */
    for (fmt, want, what) in [
        (GLES_SURF_R32F, GC_PS_OUTPUT_MODE_UIF32, "R32F is 32 bits wide"),
        (GLES_SURF_RGBA16F, GC_PS_OUTPUT_MODE_NORMAL, "16-bit float is NORMAL"),
        (GLES_SURF_RGBA8888, GC_PS_OUTPUT_MODE_NORMAL, "8-bit unorm is NORMAL"),
    ] {
        let rig = Rig::new();
        let mut d = rig.draw(3);
        d.target_format = fmt;
        let ext = rig.submit(&d).reg(GC_PS_CONTROL_EXT);
        assert_eq!(ext & 0x7, want, "{what}");
    }
}

#[test]
fn the_blender_is_programmed_from_the_gl_state_and_left_off_for_one_zero() {
    let rig = Rig::new();
    let mut d = rig.draw(3);

    /* ONE / ZERO with ADD computes what no blending computes, and a draw with
     * the blender enabled and those factors writes nothing at all on this
     * part (measured 2026-09-21) */
    d.blend = true;
    d.blend_src_rgb = SHC_BF_ONE as u8;
    d.blend_dst_rgb = SHC_BF_ZERO as u8;
    d.blend_src_alpha = SHC_BF_ONE as u8;
    d.blend_dst_alpha = SHC_BF_ZERO as u8;
    assert_eq!(rig.submit(&d).reg(GC_PE_ALPHA_CONFIG), 0);

    d.blend_src_rgb = SHC_BF_SRC_ALPHA as u8;
    d.blend_dst_rgb = SHC_BF_INV_SRC_ALPHA as u8;
    d.blend_color = [1.0, 0.5, 0.0, 1.0];
    let s = rig.submit(&d);
    let a = s.reg(GC_PE_ALPHA_CONFIG);
    assert_ne!(a & GC_PE_ALPHA_BLEND_ENABLE_COLOR, 0);
    assert_eq!((a >> 4) & 0xf, GC_BLEND_SRC_ALPHA);
    assert_eq!((a >> 8) & 0xf, GC_BLEND_ONE_MINUS_SRC_ALPHA);
    assert_ne!(a & GC_PE_ALPHA_BLEND_SEPARATE_ALPHA, 0, "the alpha factors differ from the colour ones");
    /* the constant colour, in bytes and in halves */
    assert_eq!((s.reg(GC_PE_ALPHA_BLEND_COLOR) >> 16) & 0xff, 255, "red");
    assert_eq!(s.reg(GC_PE_ALPHA_COLOR_EXT0) & 0xffff, f2half(1.0));
    /* and no OVERWRITE while the blender reads the destination */
    assert_eq!(s.reg(GC_PE_COLOR_FORMAT) & GC_PE_COLOR_FORMAT_OVERWRITE, 0);
}

#[test]
fn the_colour_mask_is_pixel_engine_state_here_not_a_shader_variant() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.color_mask = 0b0101;
    let s = rig.submit(&d);

    assert_eq!((s.reg(GC_PE_COLOR_FORMAT) >> 8) & 0xf, 0b0101);
    assert_eq!(s.reg(GC_PE_COLOR_FORMAT) & GC_PE_COLOR_FORMAT_OVERWRITE, 0, "a masked draw keeps the other channels");
}

#[test]
fn the_viewport_and_the_scissor_follow_a_top_down_target() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.viewport = [0, 0, 64, 32];
    d.scissor = true;
    d.scissor_rect = [8, 4, 16, 8];
    d.flip_y = true;
    let s = rig.submit(&d);

    /* the y scale is mirrored and the centre reflected in the target's height */
    assert_eq!(s.reg(GC_PA_VIEWPORT_SCALE_Y), f2fixp(-16.0));
    assert_eq!(s.reg(GC_PA_VIEWPORT_OFFSET_Y), f2fixp(64.0 - 16.0));
    /* the scissor is the viewport narrowed by the box, mirrored, in 16.16 */
    assert_eq!(s.reg(GC_SE_SCISSOR_LEFT) >> 16, 8);
    assert_eq!(s.reg(GC_SE_SCISSOR_TOP) >> 16, 64 - 12, "GL's y counts from the bottom");
    assert_eq!(s.reg(GC_SE_SCISSOR_RIGHT) >> 16, 24);
}

/// A viewport smaller than the target must bound the draw even with the
/// scissor test OFF.
///
/// This hardware has a guard band: it does not clip rasterisation to the
/// viewport by itself, and etnaviv therefore derives a scissor rectangle from
/// the viewport and intersects the GL scissor into it rather than the other
/// way round. A driver that programs the scissor only when the application
/// enabled the scissor test leaves the viewport unenforced, and the draw
/// covers the whole target -- which draws a picture and does not fail.
///
/// Written because the board reported ~1984 covered pixels for a draw in a
/// 32x32 viewport on a 64x64 target, which is the shape of exactly that bug.
#[test]
fn a_viewport_bounds_the_draw_with_the_scissor_test_off() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.viewport = [0, 0, 32, 32];
    d.scissor = false;
    d.flip_y = false;
    let s = rig.submit(&d);

    assert_eq!(s.reg(GC_SE_SCISSOR_LEFT) >> 16, 0);
    assert_eq!(s.reg(GC_SE_SCISSOR_TOP) >> 16, 0);
    assert_eq!(s.reg(GC_SE_SCISSOR_RIGHT) >> 16, 32, "the viewport's right edge, not the target's");
    assert_eq!(s.reg(GC_SE_SCISSOR_BOTTOM) >> 16, 32, "the viewport's bottom edge, not the target's");
}

/// The same with an offset viewport, so that a rectangle that merely happens
/// to start at the origin cannot pass.
#[test]
fn an_offset_viewport_bounds_the_draw_on_all_four_sides() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.viewport = [16, 8, 32, 16];
    d.scissor = false;
    d.flip_y = false;
    let s = rig.submit(&d);

    assert_eq!(s.reg(GC_SE_SCISSOR_LEFT) >> 16, 16);
    assert_eq!(s.reg(GC_SE_SCISSOR_TOP) >> 16, 8);
    assert_eq!(s.reg(GC_SE_SCISSOR_RIGHT) >> 16, 48);
    assert_eq!(s.reg(GC_SE_SCISSOR_BOTTOM) >> 16, 24);
}

#[test]
fn the_cull_face_is_mirrored_for_the_target_that_is_not_flipped() {
    /* The rasteriser's screen space runs y down, which is where the viewport
     * flip puts the window surface; an FBO keeps GL's y-up and arrives upside
     * down, so its winding is the one that needs correcting. The golden
     * stream settled it: for the same GL calls into an FBO, Mesa emits
     * CULL_FACE_CCW where this backend emitted CW. */
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.cull = GLES_CULL_CCW;
    d.flip_y = true;
    assert_eq!(rig.submit(&d).reg(GC_PA_CONFIG) & 0x300, GC_PA_CONFIG_CULL_FACE_CCW, "the window surface");
    d.flip_y = false;
    assert_eq!(rig.submit(&d).reg(GC_PA_CONFIG) & 0x300, GC_PA_CONFIG_CULL_FACE_CW, "an FBO");
    d.cull = GLES_CULL_NONE;
    assert_eq!(rig.submit(&d).reg(GC_PA_CONFIG) & 0x300, GC_PA_CONFIG_CULL_FACE_OFF, "nothing to mirror");
}

#[test]
fn the_scissor_margin_is_the_scissors_and_not_the_clips() {
    /* Mesa rounds the two rectangles outwards by different amounts, and both
     * of these used to be the clip's 0xffff -- almost a whole pixel of extra
     * fraction, which let a fragment at x = 64 past a test meant to stop at
     * 64, and on the last row that is a write past the surface. */
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.scissor = true;
    d.scissor_rect = [0, 0, 64, 64];
    let s = rig.submit(&d);

    assert_eq!(s.reg(GC_SE_SCISSOR_RIGHT), (64 << 16) + 0x1119);
    assert_eq!(s.reg(GC_SE_SCISSOR_BOTTOM), (64 << 16) + 0x1111);
    /* the clip keeps its own, wider, margin */
    assert_eq!(s.reg(GC_SE_CLIP_RIGHT), (64 << 16) + 0xffff);
    assert_eq!(s.reg(GC_SE_CLIP_BOTTOM), (64 << 16) + 0xffff);
}

/// Flat shading as Mesa selects it: PA_CONFIG's SHADE_MODEL is FLAT when a
/// fragment input is flat; smooth for a program with no flat input (every
/// ES 1.00 one).
#[test]
fn a_flat_varying_selects_the_flat_shade_model() {
    let rig = Rig::new();
    let mut flat_fs = *rig.fs;
    flat_fs.viv.in_[0].interp = GC_INTERP_FLAT as u8;
    let mut d = rig.draw(3);
    d.fs = &flat_fs;
    let model = |r: &Stream| r.reg(GC_PA_CONFIG) & GC_PA_CONFIG_SHADE_MODEL_SMOOTH;

    let flat = rig.submit(&d);
    let smooth = rig.submit(&rig.draw(3));
    assert_eq!(model(&flat), GC_PA_CONFIG_SHADE_MODEL_FLAT, "a flat input: flat (measured on silicon, case 89)");
    assert_eq!(model(&smooth), GC_PA_CONFIG_SHADE_MODEL_SMOOTH, "no flat input: smooth");
    assert_ne!(flat.reg(GC_PA_SYSTEM_MODE) & GC_PA_SYSTEM_MODE_PROVOKING_VERTEX_LAST, 0, "still the last vertex");
}

/// Fifteen vec4 varyings -- all a vertex shader's 16 io rows carry beside
/// its position (SHC_VIV_MAX_IO), 60 components, the ES 3.0 minimum for
/// MAX_FRAGMENT_INPUT_COMPONENTS -- program every one of them. The
/// interpolation map held 32 components (`[u32; 4]` at four bits each), so
/// a program with nine or more vec4 varyings indexed past it and panicked
/// inside the draw. The hardware's GL_HALTI5_SHADER_ATTRIBUTES has 16 words
/// (etnaviv state.xml.h), and this part 16 varyings (hwdb `varyings_count`).
#[test]
fn fifteen_vec4_varyings_are_programmed_not_a_panic() {
    let rig = Rig::new();
    let mut vs = *rig.vs;
    let mut fs = *rig.fs;
    vs.viv.num_out = 16;
    vs.viv.out[0] = ShcVivIo { reg: 1, slot: 0, components: 4, interp: 0, name: [0; 32] };
    fs.viv.num_in = 15;
    for i in 0..15 {
        vs.viv.out[i + 1] = ShcVivIo { reg: 2 + i as u8, slot: 1 + i as u8, components: 4, interp: 0, name: [0; 32] };
        fs.viv.in_[i] = ShcVivIo { reg: 1 + i as u8, slot: 1 + i as u8, components: 4, interp: 0, name: [0; 32] };
        fs.viv.vs_reg_of_in[i] = 2 + i as u8;
    }
    fs.viv.in_[14].interp = GC_INTERP_FLAT as u8;
    vs.temps = 17;
    fs.temps = 16;
    let mut d = rig.draw(3);
    d.vs = &vs;
    d.fs = &fs;
    let s = rig.submit(&d);
    assert_eq!(s.reg(GC_GL_VARYING_TOTAL_COMPONENTS), 60);
    /* the last varying's four components, 56..59, flat: word 7, low half */
    assert_eq!(s.reg(gl_halti5_shader_attributes(7)) & 0xffff, 0x2222, "components 56..59 are flat");
}

#[test]
fn the_provoking_vertex_is_the_last_one() {
    /* GL's is, and a flat-shaded varying takes its value from it; Mesa sets
     * this unconditionally and this backend did not, so the golden diff read
     * 0x10 against 0x11. */
    let rig = Rig::new();
    let mode = rig.submit(&rig.draw(3)).reg(GC_PA_SYSTEM_MODE);

    assert_ne!(mode & GC_PA_SYSTEM_MODE_PROVOKING_VERTEX_LAST, 0);
    assert_ne!(mode & GC_PA_SYSTEM_MODE_HALF_PIXEL_CENTER, 0);
}

#[test]
fn early_depth_is_never_enabled_and_the_rasteriser_never_writes_z() {
    /* every colour surface here is linear, which is what Mesa's
     * early_z_allowed refuses; the two halves of the pipe used to disagree */
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.depth_test = true;
    d.depth_write = true;
    d.depth_func = GLES_CMP_LEQUAL;
    let s = rig.submit(&d);

    assert_eq!(s.reg(GC_RA_EARLY_DEPTH) & GC_RA_EARLY_DEPTH_TEST_ENABLE, 0);
    assert_ne!(s.reg(GC_RA_EARLY_DEPTH) & GC_RA_EARLY_DEPTH_WRITE_DISABLE, 0);
    assert_eq!(s.reg(GC_PE_DEPTH_CONFIG) & GC_PE_DEPTH_EARLY_Z, 0);
    /* the depth test itself is the GL's, and the surface is D24S8 */
    assert_eq!((s.reg(GC_PE_DEPTH_CONFIG) >> 8) & 0x7, GLES_CMP_LEQUAL);
    assert_ne!(s.reg(GC_PE_DEPTH_CONFIG) & GC_PE_DEPTH_WRITE_ENABLE, 0);
    assert_ne!(s.reg(GC_PE_DEPTH_CONFIG) & GC_PE_DEPTH_FORMAT_D24S8, 0);
}

/* ---- the shader half ---------------------------------------------------- */

#[test]
fn the_two_stages_are_paired_by_register() {
    let rig = Rig::new();
    let s = rig.submit(&rig.draw(3));

    /* VS_OUTPUT[0] is the position register, [1] the register that feeds the
     * fragment shader's first varying (vs_reg_of_in[0] = 2) */
    let out = s.reg(vs_halti5_output(0));
    assert_eq!(out & 0xff, 1);
    assert_eq!((out >> 8) & 0xff, 2);
    /* the vertex shader's one input register, from the one vertex element */
    assert_eq!(s.reg(vs_halti5_input(0)) & 0xff, 0);
    assert_eq!(s.reg(GC_VS_OUTPUT_COUNT), 2, "position and one varying");
    assert_eq!(s.reg(GC_PS_INPUT_COUNT) & 0x1f, 2, "the varying plus position");
    assert_eq!(s.reg(GC_PS_OUTPUT_REG), 3);
    /* the fragment stage's temps cover its varyings even when it has fewer */
    assert_eq!(s.reg(GC_PS_TEMP_REGISTER_CONTROL) & 0x3f, 3);
}

#[test]
fn the_shader_code_is_pointed_at_and_prefetched_after_the_constants() {
    /* the two stages share one constant file and the hardware steers a write
     * by the shader state written before it: a prefetch in between would send
     * the fragment stage's constants to the vertex stage */
    let rig = Rig::new();
    let s = rig.submit(&rig.draw(3));

    assert_eq!(s.reg(GC_VS_INST_ADDR), (GC_HEAP_SHADER + 0x1000) as u32);
    assert_eq!(s.reg(GC_PS_INST_ADDR), (GC_HEAP_SHADER + 0x2000) as u32);
    assert!(s.at(GC_PS_INST_ADDR) < s.at(GC_PS_UNIFORM_BASE), "the icache block comes first");
    assert!(s.at(GC_PS_UNIFORM_BASE) < s.at(GC_VS_ICACHE_PREFETCH), "then the constants, then the prefetch");
    assert!(s.at(GC_VS_ICACHE_PREFETCH) < s.at(GC_PS_ICACHE_PREFETCH));
}

#[test]
fn a_uniform_block_is_written_into_the_constant_file_of_its_stage() {
    let rig = Rig::new();
    let mut vs = *rig.vs;
    /* two constant registers: one literal, one dword of the uniform block */
    static CONTENTS: [u8; 2] = [SHC_VIV_CONST_LITERAL, SHC_VIV_CONST_UNIFORM];
    static DATA: [u32; 2] = [0x1234_5678, 1];
    vs.viv.const_contents = CONTENTS.as_ptr();
    vs.viv.const_data = DATA.as_ptr();
    vs.viv.const_count = 2;
    let block: [u32; 2] = [0xaaaa_aaaa, 0xbbbb_bbbb];

    let mut d = rig.draw(3);
    d.vs = &vs;
    d.vs_uniform_cpu = block.as_ptr() as *const c_void;
    d.vs_uniform_dwords = 2;
    let s = rig.submit(&d);

    assert_eq!(s.reg(GC_VS_UNIFORM_BASE), 0);
    assert_eq!(s.reg(GC_PS_UNIFORM_BASE), 1, "the fragment window starts after the vertex stage's registers");
    assert_eq!(s.reg(GC_SH_HALTI5_UNIFORMS_VS), 0x1234_5678, "the literal");
    assert_eq!(s.reg(GC_SH_HALTI5_UNIFORMS_VS + 4), 0xbbbb_bbbb, "dword 1 of the block");
}

/// A vertex constant file of 275 registers (1,100 dwords), dEQP
/// uniform_api.random.3's, is written whole: registers 256..274 too, in
/// LOAD_STATEs whose ten-bit count holds them, with the fragment window after
/// it. Until 2026-10-01 the upload stopped at 1,024 dwords, one LOAD_STATE
/// whose count of 1024 does not fit the field, and the shader read registers
/// nothing had written: the case drew black on the i.MX8MP.
#[test]
fn a_vertex_constant_file_past_256_registers_is_written_whole() {
    const N: usize = 1100;
    static CONTENTS: [u8; N] = [SHC_VIV_CONST_LITERAL; N];
    static DATA: [u32; N] = {
        let mut a = [0u32; N];
        let mut i = 0;
        while i < N {
            a[i] = 0x5000_0000 + i as u32;
            i += 1;
        }
        a
    };
    let rig = Rig::new();
    let mut vs = *rig.vs;
    vs.viv.const_contents = CONTENTS.as_ptr();
    vs.viv.const_data = DATA.as_ptr();
    vs.viv.const_count = N as u16;
    let mut d = rig.draw(3);
    d.vs = &vs;
    let s = rig.submit(&d);

    assert_eq!(s.reg(GC_PS_UNIFORM_BASE), 275, "the fragment window starts after all 275 registers");
    for (i, want) in DATA.iter().enumerate() {
        assert_eq!(s.reg(GC_SH_HALTI5_UNIFORMS_VS + 4 * i as u32), *want, "dword {i} (register {})", i / 4);
    }
    /* every LOAD_STATE into the vertex window: a count the field holds, and
     * no FIXP bit */
    let window = GC_SH_HALTI5_UNIFORMS_VS..GC_SH_HALTI5_UNIFORMS_PS;
    walk(&s.0, |reg, _, at| {
        if window.contains(&reg) {
            let h = s.0[at];
            assert_eq!(h & 0x0400_0000, 0, "no FIXP on a constant");
            assert!((1..1024).contains(&((h >> 16) & 0x3ff)), "count {:#x}", h);
        }
    });
}

/// A stage that reads a uniform array at a run-time index loads it from a
/// copy of its block in memory (select.rs LoadUniformIdx, Mesa's shape): the
/// SHC_VIV_CONST_UBO_ADDR slot carries that copy's address, the copy holds
/// the block, and it has room for a four-dword load of its last dword.
#[test]
fn a_uniform_block_read_with_load_is_copied_to_memory_and_its_address_filled() {
    let rig = Rig::new();
    let mut fs = *rig.fs;
    static CONTENTS: [u8; 2] = [SHC_VIV_CONST_UNIFORM, crate::shaderc::SHC_VIV_CONST_UBO_ADDR];
    static DATA: [u32; 2] = [0, 0];
    fs.viv.const_contents = CONTENTS.as_ptr();
    fs.viv.const_data = DATA.as_ptr();
    fs.viv.const_count = 2;
    let block: [u32; 5] = [0x11, 0x22, 0x33, 0x44, 0x55];

    let mut d = rig.draw(3);
    d.fs = &fs;
    d.fs_uniform_cpu = block.as_ptr() as *const c_void;
    d.fs_uniform_dwords = 5;
    let s = rig.submit(&d);

    /* the vertex stage has no constants here: the fragment window is at 0 */
    assert_eq!(s.reg(GC_SH_HALTI5_UNIFORMS_PS), 0x11, "the block's own dword 0 is still in the file");
    let addr = s.reg(GC_SH_HALTI5_UNIFORMS_PS + 4) as u64;
    assert_ne!(addr, 0, "the UBO_ADDR slot names the copy");
    let copy = gpu_bytes(addr, 5 * 4 + 16);
    let words: std::vec::Vec<u32> = copy.chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    assert_eq!(&words[..5], &block, "the copy holds the block");
    assert_eq!(&words[5..], &[0, 0, 0, 0], "and zeros where the last element's load reads past it");
    assert!(rig.faults().is_empty());
}

/* ---- depth/stencil surfaces --------------------------------------------- */

fn depth_addr(s: &Stream) -> u32 {
    s.reg(pe_pipe_depth_addr(0))
}

/// A draw with neither a depth nor a stencil attachment (ids 0) still gets a
/// surface to address, and every such draw shares it, grown to the larger
/// target: the single `ensure_depth` buffer of before (ds_cache.rs).
#[test]
fn without_an_identity_every_target_shares_one_depth_surface() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    let a = depth_addr(&rig.submit(&d));
    d.target_gpu = TARGET_GPU + 0x10_0000;
    let b = rig.submit(&d);
    assert_eq!(depth_addr(&b), a, "one surface for both targets");
    assert_eq!(b.reg(GC_PE_DEPTH_STRIDE), 64 * 4, "a 64-wide target's rows");
}

/// With an identity each depth/stencil object has a surface of its own: two
/// framebuffers sharing a colour buffer keep separate depth (dEQP
/// fbo.render.shared_colorbuffer*), the same identity finds its surface again,
/// and a re-specified one (a new generation) gets a fresh one.
#[test]
fn each_depth_stencil_identity_has_its_own_surface() {
    use super::super::ds_cache::{set_key_override, DsKey};
    let rig = Rig::new();
    let d = rig.draw(3);
    let key = |id: u32, gen: u32| DsKey { depth_id: id, depth_gen: gen, stencil_id: id, stencil_gen: gen, scale: 1 };
    set_key_override(Some(key(1, 1)));
    let a = depth_addr(&rig.submit(&d));
    set_key_override(Some(key(2, 1)));
    let b = depth_addr(&rig.submit(&d));
    set_key_override(Some(key(1, 1)));
    let a_again = depth_addr(&rig.submit(&d));
    set_key_override(Some(key(1, 2)));
    let a_respecified = depth_addr(&rig.submit(&d));
    set_key_override(None);
    assert_ne!(a, b, "two identities, two surfaces, the same colour target");
    assert_eq!(a_again, a, "an identity finds its surface again");
    assert_ne!(a_respecified, a, "a new generation is new storage");
    assert!(rig.faults().is_empty());
}

/// A surface a pending kick still uses is never handed to another identity:
/// with every slot used by the open frame, one more identity is refused
/// instead of taking a surface the frame is drawing with.
#[test]
fn a_depth_surface_the_open_frame_uses_is_not_reused() {
    use super::super::ds_cache::{set_key_override, DsKey};
    let rig = Rig::new();
    let d = rig.draw(3);
    let mut refused = None;
    let past = ds_cache::ENTRIES as u32 + 1;
    for id in 1..=past {
        set_key_override(Some(DsKey { depth_id: id, depth_gen: 1, stencil_id: id, stencil_gen: 1, scale: 1 }));
        // SAFETY: `d` is a live draw for the length of the call.
        let rc = unsafe { gc_draw(&d) };
        if rc != 0 {
            refused = Some(id);
            break;
        }
    }
    set_key_override(None);
    assert_eq!(refused, Some(past), "every surface is in the open frame; the identity past them waits");
    assert_eq!(gc_flush(), 0);
    set_key_override(Some(DsKey { depth_id: past, depth_gen: 1, stencil_id: past, stencil_gen: 1, scale: 1 }));
    // SAFETY: as above.
    let rc = unsafe { gc_draw(&d) };
    set_key_override(None);
    assert_eq!(rc, 0, "once the frame has completed, a surface is free again");
    assert_eq!(gc_flush(), 0);
}

/// gl_PointCoord.t' = t * scale + offset (select.rs point_coord): identity on
/// a target this backend draws mirrored, t' = 1 - t on one it does not -- a
/// y-up pbuffer since orientation step 1, where dEQP
/// builtin_variable.pointcoord broke on the i.MX8MP.
#[test]
fn the_point_coord_transform_follows_the_targets_orientation() {
    let rig = Rig::new();
    let mut fs = *rig.fs;
    static CONTENTS: [u8; 2] =
        [crate::shaderc::SHC_VIV_CONST_PNTC_Y_SCALE, crate::shaderc::SHC_VIV_CONST_PNTC_Y_OFFSET];
    static DATA: [u32; 2] = [0, 0];
    fs.viv.const_contents = CONTENTS.as_ptr();
    fs.viv.const_data = DATA.as_ptr();
    fs.viv.const_count = 2;
    let pair = |flip: bool| {
        let mut d = rig.draw(3);
        d.fs = &fs;
        d.flip_y = flip;
        let s = rig.submit(&d);
        (f32::from_bits(s.reg(GC_SH_HALTI5_UNIFORMS_PS)), f32::from_bits(s.reg(GC_SH_HALTI5_UNIFORMS_PS + 4)))
    };
    let (mirrored, upright) = (pair(true), pair(false));
    let want_mirrored = if mirrors(true) { (1.0, 0.0) } else { (-1.0, 1.0) };
    let want_upright = if mirrors(false) { (1.0, 0.0) } else { (-1.0, 1.0) };
    assert_eq!((mirrored, upright), (want_mirrored, want_upright));
    assert_ne!(mirrored, upright, "the two orientations differ");
}

/// A colour format with no alpha reads alpha 1: a draw into it leaves the
/// alpha byte alone (the clear wrote 1 there), and a clear still writes all
/// four channels.
#[test]
fn a_draw_into_a_format_without_alpha_leaves_the_alpha_byte_alone() {
    let rig = Rig::new();
    let comps = |s: &Stream| (s.reg(GC_PE_COLOR_FORMAT) >> 8) & 0xf;
    let mut d = rig.draw(3);
    d.target_alpha_bits = 0;
    let draw = comps(&rig.submit(&d));
    let mut c = rig.draw(0);
    c.target_alpha_bits = 0;
    c.do_clear = true;
    c.clear_color_mask = 0xf;
    let clear = comps(&rig.submit(&c));
    c.clear_color_mask = 0b0011;
    let masked = comps(&rig.submit(&c));
    d.target_alpha_bits = 8;
    let rgba = comps(&rig.submit(&d));
    assert_eq!(
        (draw, clear, masked, rgba),
        (0x7, 0xf, 0b1011, 0xf),
        "(draw into RGB, clear of RGB, a masked clear of RGB also writes its alpha, draw into RGBA)"
    );

    /* and a blend that reads destination alpha reads the constant 1 there */
    let mut b = rig.draw(3);
    b.target_alpha_bits = 0;
    b.blend = true;
    b.blend_src_rgb = SHC_BF_DST_ALPHA as u8;
    b.blend_dst_rgb = SHC_BF_INV_DST_ALPHA as u8;
    b.blend_src_alpha = SHC_BF_SRC_ALPHA_SATURATE as u8;
    b.blend_dst_alpha = SHC_BF_ZERO as u8;
    let cfg = rig.submit(&b).reg(GC_PE_ALPHA_CONFIG);
    let fields = pe_alpha_src_func_color(0xf) | pe_alpha_dst_func_color(0xf) | pe_alpha_src_func_alpha(0xf);
    assert_eq!(
        cfg & fields,
        pe_alpha_src_func_color(GC_BLEND_ONE)
            | pe_alpha_dst_func_color(GC_BLEND_ZERO)
            | pe_alpha_src_func_alpha(GC_BLEND_ZERO),
        "DST_ALPHA -> ONE, ONE_MINUS_DST_ALPHA -> ZERO, SRC_ALPHA_SATURATE -> ZERO"
    );
}

/* ---- textures ----------------------------------------------------------- */

fn bind(gpu: u64, w: u32, h: u32, levels: u8) -> GlesTextureBind {
    let mut t = GlesTextureBind::ZERO;
    t.unit = 0;
    t.format = GLES_TEXFMT_RGBA8;
    t.base_gpu = gpu;
    t.width = w;
    t.height = h;
    t.stride_texels = w;
    t.bytes = w * h * 4;
    t.mip_levels = levels;
    t.mip_filter = u8::from(levels > 1);
    t
}

#[test]
fn a_descriptor_names_every_level_on_a_64_byte_boundary_at_one_stride() {
    /* the descriptor carries one LINEAR_STRIDE for the whole chain and
     * fetches a level from a 64-byte boundary, which is what
     * `mip_level_align` makes the GL core lay out */
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.num_textures = 1;
    d.textures[0] = bind(rig.vbo.gpu, 4, 4, 3);
    let s = rig.submit(&d);
    let desc = s.descriptor(0);

    assert_eq!(desc[GC_TEXDESC_LINEAR_STRIDE as usize / 4], 16, "4 texels of 4 bytes");
    assert_eq!(desc[GC_TEXDESC_SIZE as usize / 4], texdesc_size_width(4) | texdesc_size_height(4));
    assert_eq!((desc[GC_TEXDESC_BASELOD as usize / 4] >> 8) & 0xf, 2, "three levels: 0..2");
    let lod = |i: u32| desc[texdesc_lod_addr(i) as usize / 4];
    assert_eq!(lod(0) as u64, rig.vbo.gpu);
    assert_eq!(lod(1), lod(0) + 64, "level 0 is 4 rows of 16 bytes, already aligned");
    assert_eq!(lod(2), lod(1) + 64, "level 1 is 2 rows of the chain's stride, rounded up to 64");
    /* the sampler's own LOD range, not the chain's length */
    assert_eq!(s.reg(nte_descriptor_samp_lod_minmax(0)), samp_lod_max(GC_SAMP_LOD_FP58_MAX));
    /* and the slot's descriptor cache entry is invalidated: the arena puts
     * this draw's descriptors at a fresh address every time */
    assert_eq!(s.reg(GC_NTE_DESCRIPTOR_INVALIDATE), GC_NTE_INVALIDATE_UNK29);
}

/// GL_OES_texture_npot on the GC7000, glmark2 terrain's case: glGenerateMipmap
/// of an 800x600 RGBA texture under this backend's table lays out its ten
/// levels (800x600 down to 1x1) at the chain's one 800-texel pitch, each on
/// a 64-byte boundary, and the descriptor of the complete, REPEAT-wrapped,
/// LINEAR_MIPMAP_LINEAR texture names exactly those levels: SIZE 800x600,
/// BASELOD 0..9, LOD_ADDR i = the GL core's level i, the sampler REPEAT on
/// both axes with a linear mip filter.
///
/// The hardware's own NPOT support (etnaviv asks for chipMinorFeatures1
/// NON_POWER_OF_TWO, which this part's recorded words do not show) is not a
/// host question: docs/es2-cts-failures.md says what the board must show.
#[test]
fn a_generated_npot_chain_is_named_level_by_level() {
    use crate::gl::consts::*;
    use crate::gl::texture::{
        glBindTexture, glDeleteTextures, glGenTextures, glGenerateMipmap, glTexImage2D, glTexParameteri,
    };
    let rig = Rig::new();
    /* the GL core reads this backend's layout flags and allocates through
     * the rig's platform, as in the product; TestGl's drop resets the table */
    // SAFETY: serialised by the rig's TestGl.
    unsafe { crate::backend::g_backend = &yakogl_backend_vivante };
    let (w, h) = (800u32, 600u32);
    let texels = vec![0x40u8; (w * h * 4) as usize];
    let mut name = 0;
    // SAFETY: plain GL calls on the rig's context with live pointers.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(GL_TEXTURE_2D, name);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGBA as i32,
            w as i32,
            h as i32,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            texels.as_ptr() as *const c_void,
        );
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as i32);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as i32);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_REPEAT as i32);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_REPEAT as i32);
        glGenerateMipmap(GL_TEXTURE_2D);
    }
    let t = &rig._gl.ctx().objs().textures[name as usize];
    assert_eq!(rig._gl.error(), GL_NO_ERROR, "glGenerateMipmap of 800x600");
    assert_eq!((t.chain_levels, t.levels, t.level_defined, t.level_bad), (10, 10, 0x3ff, 0));
    assert_eq!(t.level_pitch(1), 800, "one pitch for the chain");
    /* level i starts after the rows of the levels above it, 3200 bytes each
     * (a multiple of 64, so no padding) */
    let heights = [600u32, 300, 150, 75, 37, 18, 9, 4, 2, 1];
    let mut rows = 0;
    for (i, lh) in heights.iter().enumerate() {
        assert_eq!(t.level_off[i], rows * 3200, "level {i}");
        assert_eq!((h >> i).max(1), *lh);
        rows += lh;
    }
    let b = crate::gl::draw::texture_bind_2d(t);
    assert_eq!((b.mip_levels, b.mip_filter, b.stride_texels), (10, 2, 800));
    let mut d = rig.draw(3);
    d.num_textures = 1;
    d.textures[0] = b;
    let s = rig.submit(&d);
    let desc = s.descriptor(0);
    assert_eq!(desc[GC_TEXDESC_SIZE as usize / 4], texdesc_size_width(800) | texdesc_size_height(600));
    assert_eq!(desc[GC_TEXDESC_LINEAR_STRIDE as usize / 4], 3200);
    assert_eq!((desc[GC_TEXDESC_BASELOD as usize / 4] >> 8) & 0xf, 9, "ten levels: 0..9");
    for i in 0..10u32 {
        assert_eq!(
            desc[texdesc_lod_addr(i) as usize / 4] as u64,
            t.mem.gpu + t.level_off[i as usize] as u64,
            "level {i}: the descriptor and the GL core agree"
        );
    }
    let c0 = s.reg(nte_descriptor_samp_ctrl0(0));
    assert_eq!(
        c0 & (samp_uwrap(0xffff_ffff) | samp_vwrap(0xffff_ffff) | samp_mip(0xffff_ffff)),
        samp_uwrap(GC_TEX_WRAP_REPEAT) | samp_vwrap(GC_TEX_WRAP_REPEAT) | samp_mip(GC_TEX_FILTER_LINEAR)
    );
    /* freed while the rig's OSAL, which allocated it, is the one installed */
    // SAFETY: a live name.
    unsafe { glDeleteTextures(1, &name) };
}

#[test]
fn a_mipmapped_cubes_levels_are_where_the_gl_core_put_them() {
    /* dEQP's texturecube: a 256 x 256 cube with nine levels. The GL core
     * keeps level i as all six faces at (256 >> i), 64-aligned
     * (gl/texture.rs chain_layout), and each LOD_ADDR must name exactly
     * that. Stepping one face per level put level 1 at 256 * 256 * 4 --
     * inside level 0's second face -- and the fragment case failed on the
     * i.MX8MP while the one-level vertex case passed (2026-09-24). */
    let rig = Rig::new();
    let (w, levels) = (256u32, 9u8);
    let Some((off, n, _, _)) = crate::gl::texture::chain_layout(
        w,
        w,
        6,
        4,
        false,
        yakogl_backend_vivante.mip_level_align,
        !yakogl_backend_vivante.native_layered_textures,
    ) else {
        panic!("the GL core lays out a 256 cube");
    };
    assert_eq!(n, levels, "the GL core's chain for a 256 cube");
    let mut d = rig.draw(3);
    d.num_textures = 1;
    d.textures[0] = bind(rig.vbo.gpu, w, w, levels);
    d.textures[0].type_ = GLES_TEXTYPE_CUBE;
    d.textures[0].layers = 6;
    let s = rig.submit(&d);
    let desc = s.descriptor(0);
    for i in 0..levels as u32 {
        assert_eq!(
            desc[texdesc_lod_addr(i) as usize / 4] as u64,
            rig.vbo.gpu + off[i as usize] as u64,
            "level {i}: the descriptor and the GL core disagree about where it starts"
        );
    }
    /* a one-layer chain is unchanged by the fix */
    assert_eq!(off[1], 6 * w * w * 4, "level 1 follows all six faces of level 0");
}

/// A 16x16 RGBA8 cube with five levels as the GL core stores it for this
/// backend (`chain_layout`), texel (x, y) of face k at level l holding
/// `k << 24 | l << 16 | y << 8 | x`, plus `salt` in every texel.
fn linear_cube(salt: u32) -> (Vec<u32>, GlesTextureBind) {
    let Some((off, n, bytes, pitch)) = crate::gl::texture::chain_layout(
        16,
        16,
        6,
        4,
        false,
        yakogl_backend_vivante.mip_level_align,
        !yakogl_backend_vivante.native_layered_textures,
    ) else {
        panic!("the GL core lays out a 16 cube");
    };
    assert_eq!(n, 5);
    let mut mem = vec![0u32; bytes as usize / 4];
    for l in 0..5u32 {
        let lw = 16 >> l;
        for k in 0..6u32 {
            for y in 0..lw {
                for x in 0..lw {
                    let at = off[l as usize] + (k * lw + y) * pitch * 4 + x * 4;
                    mem[at as usize / 4] = (k << 24 | l << 16 | y << 8 | x) + salt;
                }
            }
        }
    }
    let mut t = bind(GC_HEAP_DATA + 0x80_0000, 16, 16, 5);
    t.type_ = GLES_TEXTYPE_CUBE;
    t.layers = 6;
    t.stride_texels = pitch;
    t.bytes = bytes;
    t.cpu = mem.as_ptr() as *const c_void;
    (mem, t)
}

/// The bytes of the allocation holding GPU address `gpu`, from there on.
fn gpu_bytes(gpu: u64, n: usize) -> Vec<u8> {
    with_platform(|p| {
        let (base, a) = p.allocs.range(..=gpu).next_back().expect("an allocation");
        let off = (gpu - base) as usize;
        assert!(off + n <= a.size, "0x{gpu:x} + {n} runs off its allocation");
        // SAFETY: inside that allocation (checked just above).
        unsafe { core::slice::from_raw_parts((a.mem.as_ptr() as *const u8).add(off), n) }.to_vec()
    })
}

#[test]
fn a_mipmapped_cube_is_sampled_from_a_tiled_copy_of_its_faces() {
    /* gl_test 113 on the i.MX8MP: the texture unit steps from face to face by
     * the level's 4x4-tile-padded size whatever the addressing mode, so a
     * mipmapped cube is sampled from a tiled copy (tiled_copy.rs). Every
     * LOD_ADDR names the copy's level, SLICE is level 0's padded face, the
     * addressing is TILED, and each texel sits in its tile. */
    let rig = Rig::new();
    let (_mem, t) = linear_cube(0);
    let mut d = rig.draw(3);
    d.num_textures = 1;
    d.textures[0] = t;
    let s = rig.submit(&d);
    let desc = s.descriptor(0);

    assert_eq!(
        (desc[GC_TEXDESC_CONFIG0 as usize / 4] >> 20) & 0x3,
        GC_TEX_ADDRESSING_TILED,
        "CONFIG0 0x{:08x}",
        desc[GC_TEXDESC_CONFIG0 as usize / 4]
    );
    assert_eq!(desc[GC_TEXDESC_SLICE as usize / 4], 16 * 16 * 4, "level 0's face");
    assert_eq!(desc[GC_TEXDESC_LINEAR_STRIDE as usize / 4], 16 * 4);
    let lod = |i: u32| desc[texdesc_lod_addr(i) as usize / 4] as u64;
    assert_ne!(lod(0), t.base_gpu, "the copy, not the GL core's storage");
    let face = [1024u64, 256, 64, 64, 64];
    let copy = gpu_bytes(lod(0), 8832);
    for l in 0..5u32 {
        assert_eq!(lod(l), lod(0) + [0, 6144, 7680, 8064, 8448][l as usize], "level {l}");
        let lw = 16u32 >> l;
        let tw = lw.next_multiple_of(4);
        for k in 0..6u32 {
            for y in 0..lw {
                for x in 0..lw {
                    let tile = (y / 4) * (tw / 4) + x / 4;
                    let at = (lod(l) - lod(0) + k as u64 * face[l as usize]) as usize
                        + ((tile * 16 + (y % 4) * 4 + x % 4) * 4) as usize;
                    let v = u32::from_le_bytes(copy[at..at + 4].try_into().unwrap());
                    assert_eq!(v, k << 24 | l << 16 | y << 8 | x, "level {l} face {k} ({x}, {y})");
                }
            }
        }
    }
}

#[test]
fn the_tiled_copy_is_rebuilt_when_the_texels_change_and_only_then() {
    let rig = Rig::new();
    let (mut mem, t) = linear_cube(0);
    let mut d = rig.draw(3);
    d.num_textures = 1;
    d.textures[0] = t;
    /* two frames first: the second opens the other frame of the pair */
    let first = rig.submit(&d).descriptor(0)[texdesc_lod_addr(0) as usize / 4];
    let again = rig.submit(&d).descriptor(0)[texdesc_lod_addr(0) as usize / 4];
    assert_eq!(again, first, "the same texels: the same copy");
    let live = with_platform(|p| p.allocs.len());
    assert_eq!(rig.submit(&d).descriptor(0)[texdesc_lod_addr(0) as usize / 4], first);
    assert_eq!(with_platform(|p| p.allocs.len()), live, "and nothing new allocated");

    /* a CPU write (the GL core bumps content_gen) */
    mem[0] = 0xdead_beef;
    d.textures[0].content_gen += 1;
    let lod0 = rig.submit(&d).descriptor(0)[texdesc_lod_addr(0) as usize / 4] as u64;
    let v = u32::from_le_bytes(gpu_bytes(lod0, 4)[..].try_into().unwrap());
    assert_eq!(v, 0xdead_beef, "the new texels are in the copy the draw samples");
    assert_eq!(with_platform(|p| p.allocs.len()), live, "the stale copy was freed, not kept");
}

#[test]
fn a_render_into_the_cube_still_in_the_open_frame_is_finished_before_the_copy() {
    /* the copy is read from memory by the CPU: a draw into the cube recorded
     * in the open frame has not written it yet, so the frame goes first */
    let rig = Rig::new();
    let (_mem, t) = linear_cube(0);
    let plain = rig.draw(3);
    // SAFETY: `plain` is a live draw for the length of the call.
    assert_eq!(unsafe { gc_draw(&plain) }, 0);
    assert_eq!(rig.kicks(), 0);
    let mut d = rig.draw(3);
    d.num_textures = 1;
    d.textures[0] = t;
    d.textures[0].target_kick = gc_kick_id(); /* the open frame renders into it */
    // SAFETY: as above.
    assert_eq!(unsafe { gc_draw(&d) }, 0);
    assert_eq!(rig.kicks(), 1, "the open frame was submitted and waited for first");
    assert_eq!(gc_flush(), 0);
    assert_eq!(rig.kicks(), 2);

    /* a render into it by a kick that has completed needs no wait */
    d.textures[0].target_kick = gc_kick_id() - 1;
    d.textures[0].content_gen += 1;
    // SAFETY: as above.
    assert_eq!(unsafe { gc_draw(&d) }, 0);
    assert_eq!(rig.kicks(), 2);
    assert_eq!(gc_flush(), 0);
    assert!(rig.faults().is_empty());
}

/// A one-level cube whose faces are not whole 4x4 tiles is sampled from the
/// tiled copy too: the texture unit steps faces by align4(w) * align4(h)
/// texels and the GL core stores one as stride * h, which for a 6x5 face is
/// 6 * 5 against 8 * 8 (dEQP filtering.cube's *_npot on the i.MX8MP).
/// The copies hold at most a budget of GPU memory between them: making a new
/// one frees the least recently used idle ones first, so a run of large
/// mipmapped cubes does not leave every one of them allocated (dEQP
/// texture.size.cube.512x512_rgba4444_mipmap ended the i.MX8MP run).
#[test]
fn the_tiled_copies_stay_under_their_memory_budget() {
    let rig = Rig::new();
    let live_bytes = || with_platform(|p| p.allocs.values().map(|a| a.size).sum::<usize>());
    let base = live_bytes();
    /* 256x256 RGBA8 cubes, 9 levels: ~2 MiB each, copy included */
    let n = 256u32;
    let mut mems = std::vec::Vec::new();
    for k in 0..12u64 {
        let Some((_, _, bytes, pitch)) = crate::gl::texture::chain_layout(
            n,
            n,
            6,
            4,
            false,
            yakogl_backend_vivante.mip_level_align,
            !yakogl_backend_vivante.native_layered_textures,
        ) else {
            panic!("a 256 cube lays out");
        };
        let mem = vec![0u32; bytes as usize / 4];
        let mut t = bind(GC_HEAP_DATA + 0x100_0000 * (k + 1), n, n, 9);
        t.type_ = GLES_TEXTYPE_CUBE;
        t.layers = 6;
        t.stride_texels = pitch;
        t.bytes = bytes;
        t.cpu = mem.as_ptr() as *const c_void;
        mems.push(mem);
        let mut d = rig.draw(3);
        d.num_textures = 1;
        d.textures[0] = t;
        let _ = rig.submit(&d);
    }
    let held = live_bytes() - base;
    assert!(held <= 16 << 20, "the copies hold {held} bytes, over the 16 MiB budget");
    assert!(rig.faults().is_empty());
}

#[test]
fn a_one_level_npot_cube_is_sampled_from_the_tiled_copy() {
    let rig = Rig::new();
    let (w, h) = (6u32, 5u32);
    let mut mem = vec![0u32; (w * h * 6) as usize];
    for k in 0..6u32 {
        for y in 0..h {
            for x in 0..w {
                mem[((k * h + y) * w + x) as usize] = k << 24 | y << 8 | x;
            }
        }
    }
    let mut t = bind(GC_HEAP_DATA + 0x80_0000, w, h, 1);
    t.type_ = GLES_TEXTYPE_CUBE;
    t.layers = 6;
    t.stride_texels = w;
    t.bytes = w * h * 6 * 4;
    t.cpu = mem.as_ptr() as *const c_void;
    let mut d = rig.draw(3);
    d.num_textures = 1;
    d.textures[0] = t;
    let desc = rig.submit(&d).descriptor(0);
    assert_eq!((desc[GC_TEXDESC_CONFIG0 as usize / 4] >> 20) & 0x3, GC_TEX_ADDRESSING_TILED);
    assert_eq!(desc[GC_TEXDESC_SLICE as usize / 4], 8 * 8 * 4, "a face is two tiles by two");
    let lod0 = desc[texdesc_lod_addr(0) as usize / 4] as u64;
    assert_ne!(lod0, t.base_gpu, "the copy, not the storage");
    let copy = gpu_bytes(lod0, 6 * 8 * 8 * 4);
    for k in 0..6u32 {
        for y in 0..h {
            for x in 0..w {
                let at = (k * 256) as usize + ((((y / 4) * 2 + x / 4) * 16 + (y % 4) * 4 + x % 4) * 4) as usize;
                let v = u32::from_le_bytes(copy[at..at + 4].try_into().unwrap());
                assert_eq!(v, k << 24 | y << 8 | x, "face {k} ({x}, {y})");
            }
        }
    }
}

/// The surface cache reads the GL core's identity fields (9fed3d5a): two
/// draws into one colour target with different depth objects get different
/// surfaces, the same object gets its own back.
#[test]
fn the_depth_stencil_identity_comes_from_the_draw() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    let with = |d: &mut GlesDraw, id: u32| {
        d.depth_id = id;
        d.depth_gen = 1;
        d.stencil_id = id;
        d.stencil_gen = 1;
    };
    with(&mut d, 7);
    let a = depth_addr(&rig.submit(&d));
    with(&mut d, 8);
    let b = depth_addr(&rig.submit(&d));
    with(&mut d, 7);
    let a_again = depth_addr(&rig.submit(&d));
    assert_ne!(a, b);
    assert_eq!(a, a_again);
}

#[test]
fn a_one_level_cube_and_a_mipmapped_2d_texture_are_sampled_as_they_are() {
    let rig = Rig::new();
    let (_mem, mut t) = linear_cube(0);
    t.mip_levels = 1;
    let mut d = rig.draw(3);
    d.num_textures = 2;
    d.textures[0] = t;
    let (mem2d, mut t2) = linear_cube(0);
    t2.type_ = GLES_TEXTYPE_2D;
    t2.layers = 1;
    t2.cpu = mem2d.as_ptr() as *const c_void;
    t2.unit = 1;
    d.textures[1] = t2;
    let s = rig.submit(&d);
    for slot in 0..2 {
        let desc = s.descriptor(slot);
        assert_eq!(desc[texdesc_lod_addr(0) as usize / 4] as u64, t.base_gpu, "slot {slot}: its own storage");
        assert_eq!((desc[GC_TEXDESC_CONFIG0 as usize / 4] >> 20) & 0x3, GC_TEX_ADDRESSING_LINEAR, "slot {slot}");
    }
}

/// A mipmapped 2D texture the VERTEX stage samples is read from a tiled
/// copy (Mesa's layout for every sampler; golden mode 26), and the same
/// texture in the fragment stage alone keeps its linear storage.
#[test]
fn a_vertex_stage_2d_texture_is_sampled_from_a_tiled_copy() {
    let rig = Rig::new();
    let (mem2d, mut t) = linear_cube(0);
    t.type_ = GLES_TEXTYPE_2D;
    t.layers = 1;
    t.cpu = mem2d.as_ptr() as *const c_void;
    let addressing = |desc: &[u32]| (desc[GC_TEXDESC_CONFIG0 as usize / 4] >> 20) & 0x3;
    let lod0 = |desc: &[u32]| desc[texdesc_lod_addr(0) as usize / 4] as u64;
    let vs_draw = || {
        let mut d = rig.draw(3);
        d.num_vs_textures = 1;
        d.vs_textures[0] = t;
        d
    };

    let mut fs = rig.draw(3);
    fs.num_textures = 1;
    fs.textures[0] = t;
    let fs_only = rig.submit(&fs).descriptor(0);
    assert_eq!(
        (addressing(&fs_only), lod0(&fs_only)),
        (GC_TEX_ADDRESSING_LINEAR, t.base_gpu),
        "fragment stage only: no copy is made for it"
    );
    let on = rig.submit(&vs_draw()).descriptor(0);
    assert_eq!(addressing(&on), GC_TEX_ADDRESSING_TILED, "CONFIG0 0x{:08x}", on[GC_TEXDESC_CONFIG0 as usize / 4]);
    assert_ne!(lod0(&on), t.base_gpu, "vertex stage: the copy");
    /* level 0 of the copy holds texel (x, y) in its 4x4 tile: 16 wide is
     * four tiles across */
    let copy = gpu_bytes(lod0(&on), 16 * 16 * 4);
    for (x, y) in [(0u32, 0u32), (5, 6), (15, 15)] {
        let tile = (y / 4) * 4 + x / 4;
        let at = ((tile * 16 + (y % 4) * 4 + x % 4) * 4) as usize;
        assert_eq!(u32::from_le_bytes(copy[at..at + 4].try_into().unwrap()), y << 8 | x, "({x}, {y})");
    }
}

/// An RGBA16F 8x8 texture with its bytes, as gl_test 44's render target is:
/// texel (x, y) holds the halves (x, y, salt, 0x3c00).
fn half_texture(gpu: u64, salt: u16) -> (Vec<u16>, GlesTextureBind) {
    let (w, h) = (8u32, 8u32);
    let mut mem = vec![0u16; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let at = ((y * w + x) * 4) as usize;
            mem[at..at + 4].copy_from_slice(&[x as u16, y as u16, salt, 0x3c00]);
        }
    }
    let mut t = bind(gpu, w, h, 1);
    t.format = GLES_TEXFMT_RGBA16F;
    t.bytes = w * h * 8;
    t.cpu = mem.as_ptr() as *const c_void;
    (mem, t)
}

/// A texture with 64-bit texels is sampled from a tiled copy at a page
/// boundary, never from its linear storage, in the fragment stage too: on
/// the i.MX8MP the texture unit misread a LINEAR RGBA16F texture at a
/// 64-byte-aligned base (its first 64 bytes as 0, the edge texels of other
/// rows at a 64-byte stride; gl_test 44, #6), and Mesa samples such a
/// texture only TILED in a page-aligned buffer. The OSAL here hands out
/// addresses 64 bytes past a page, as the GC7000 one can.
#[test]
fn an_rgba16f_texture_is_sampled_tiled_from_a_page() {
    let rig = Rig::new();
    with_platform(|p| p.off_page = true);
    let (mem, t) = half_texture(GC_HEAP_DATA + 0x80_0040, 7);
    let mut d = rig.draw(3);
    d.num_textures = 1;
    d.textures[0] = t;
    let desc = rig.submit(&d).descriptor(0);
    let c0 = desc[GC_TEXDESC_CONFIG0 as usize / 4];
    let lod0 = desc[texdesc_lod_addr(0) as usize / 4] as u64;

    assert_eq!((c0 >> 20) & 0x3, GC_TEX_ADDRESSING_TILED, "CONFIG0 0x{c0:08x}");
    assert_ne!(lod0, t.base_gpu, "the copy, not the GL core's storage");
    assert_eq!(lod0 % 4096, 0, "the copy starts on a page: 0x{lod0:x}");
    assert_eq!(desc[GC_TEXDESC_LINEAR_STRIDE as usize / 4], 8 * 8, "a level-0 row of 8 texels of 8 bytes");
    assert_eq!(desc[GC_TEXDESC_SLICE as usize / 4], 8 * 8 * 8);
    /* every texel in its 4x4 tile, all eight bytes */
    let copy = gpu_bytes(lod0, 8 * 8 * 8);
    for y in 0..8u32 {
        for x in 0..8u32 {
            let tile = (y / 4) * 2 + x / 4;
            let at = ((tile * 16 + (y % 4) * 4 + x % 4) * 8) as usize;
            let src = ((y * 8 + x) * 4) as usize;
            let got: Vec<u16> = copy[at..at + 8].chunks(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
            assert_eq!(got, mem[src..src + 4], "texel ({x}, {y})");
        }
    }
    assert!(rig.faults().is_empty(), "{:?}", rig.faults());

    /* an RGBA8 texture at the same kind of address keeps its linear storage */
    let rgba = vec![0u32; 64];
    let mut t8 = bind(GC_HEAP_DATA + 0x90_0040, 8, 8, 1);
    t8.cpu = rgba.as_ptr() as *const c_void;
    d.textures[0] = t8;
    let desc = rig.submit(&d).descriptor(0);
    assert_eq!((desc[GC_TEXDESC_CONFIG0 as usize / 4] >> 20) & 0x3, GC_TEX_ADDRESSING_LINEAR);
    assert_eq!(desc[texdesc_lod_addr(0) as usize / 4] as u64, t8.base_gpu);
}

/// gl_test 44's round trip through the mock backend: a clear and a draw into
/// an RGBA16F texture, each followed by a draw sampling it. A render into the
/// texture still in the open frame is submitted and finished before the copy
/// is read from memory, and each render's texels are what the next draw
/// samples -- the copy is rebuilt from the storage, not kept.
#[test]
fn a_render_into_an_rgba16f_texture_reaches_the_draw_that_samples_it() {
    let rig = Rig::new();
    let (mut mem, t) = half_texture(GC_HEAP_DATA + 0x80_0000, 1);
    let sample = |t: GlesTextureBind| {
        let mut d = rig.draw(3);
        d.num_textures = 1;
        d.textures[0] = t;
        d
    };
    let texel0 = |s: &Stream| {
        let lod0 = s.descriptor(0)[texdesc_lod_addr(0) as usize / 4] as u64;
        let b = gpu_bytes(lod0, 8);
        [0, 2, 4, 6].map(|i| u16::from_le_bytes([b[i], b[i + 1]]))
    };

    /* the "clear" is a render into it in the open frame */
    let mut into = rig.draw(3);
    into.target_gpu = t.base_gpu;
    into.target_format = GLES_SURF_RGBA16F;
    into.targets[0] = GlesDrawTarget { gpu: t.base_gpu, w: 8, h: 8, format: GLES_SURF_RGBA16F };
    into.target_w = 8;
    into.target_h = 8;
    into.viewport = [0, 0, 8, 8];
    // SAFETY: `into` is a live draw for the length of the call.
    assert_eq!(unsafe { gc_draw(&into) }, 0);
    let kicks = rig.kicks();
    /* what the PE would have written: 2.0, 0.5, 0.25, 1.0 */
    mem[..4].copy_from_slice(&[0x4000, 0x3800, 0x3400, 0x3c00]);
    let mut tc = t;
    tc.target_kick = gc_kick_id();
    let s = rig.submit(&sample(tc));
    assert!(rig.kicks() > kicks, "the render into the texture was submitted before the copy");
    assert_eq!(texel0(&s), [0x4000, 0x3800, 0x3400, 0x3c00], "the clear's texel is sampled");

    /* a draw into it, by a kick that has completed */
    mem[..4].copy_from_slice(&[0x2c00, 0x3b00, 0x2c00, 0x3c00]);
    tc.target_kick = gc_kick_id() - 1;
    let s = rig.submit(&sample(tc));
    assert_eq!(texel0(&s), [0x2c00, 0x3b00, 0x2c00, 0x3c00], "the draw's texel, not the clear's");
    assert!(rig.faults().is_empty(), "{:?}", rig.faults());
}

/// INT_FILTER (SAMP_CTRL0 bit 23) on a UNORM, non-3D texture's sampler in
/// either stage, as Mesa's texture_use_int_filter sets it (golden mode 26),
/// never on a float format or a 3D texture.
#[test]
fn a_unorm_sampler_takes_int_filter() {
    let rig = Rig::new();
    let ctrl0 = |t: GlesTextureBind, vs: bool| {
        let mut d = rig.draw(3);
        if vs {
            d.num_vs_textures = 1;
            d.vs_textures[0] = t;
        } else {
            d.num_textures = 1;
            d.textures[0] = t;
        }
        rig.submit(&d).reg(nte_descriptor_samp_ctrl0(0))
    };
    let rgba = bind(rig.vbo.gpu, 4, 4, 1);
    let mut half = rgba;
    half.format = GLES_TEXFMT_RGBA16F;
    let mut vol = rgba;
    vol.type_ = GLES_TEXTYPE_3D;
    vol.layers = 2;

    assert_ne!(ctrl0(rgba, false) & GC_SAMP_INT_FILTER, 0, "RGBA8, fragment stage");
    assert_ne!(ctrl0(rgba, true) & GC_SAMP_INT_FILTER, 0, "RGBA8, vertex stage");
    assert_eq!(ctrl0(half, false) & GC_SAMP_INT_FILTER, 0, "RGBA16F: not UNORM");
    assert_eq!(ctrl0(vol, false) & GC_SAMP_INT_FILTER, 0, "a 3D texture");
}

/// An sRGB texture's sampler: SAMP_CTRL1 is Mesa's UNK1 with SRGB (0x4,
/// state_3d.xml.h:1984; etnaviv_texture_desc.c:246-247), every other format
/// UNK1 alone; INT_FILTER off (Mesa's texture_use_int_filter leaves an sRGB
/// format out, etnaviv_format.c:317-318); the descriptor's texel format the
/// RGBA8 one (etnaviv translates through util_format_linear,
/// etnaviv_format.c:278).
#[test]
fn an_srgb_sampler_decodes_and_takes_no_int_filter() {
    let rig = Rig::new();
    let regs = |t: GlesTextureBind| {
        let mut d = rig.draw(3);
        d.num_textures = 1;
        d.textures[0] = t;
        let s = rig.submit(&d);
        (s.reg(nte_descriptor_samp_ctrl0(0)), s.reg(nte_descriptor_samp_ctrl1(0)))
    };
    let rgba = bind(rig.vbo.gpu, 4, 4, 1);
    let mut srgb = rgba;
    srgb.format = GLES_TEXFMT_SRGB8_A8;
    let (c0, c1) = regs(srgb);
    assert_eq!(c1, GC_SAMP_CTRL1_UNK1 | GC_SAMP_CTRL1_SRGB, "SRGB decode");
    assert_eq!(c0 & GC_SAMP_INT_FILTER, 0, "no integer filter for sRGB");
    let (r0, r1) = regs(rgba);
    assert_eq!(r1, GC_SAMP_CTRL1_UNK1, "RGBA8: no decode");
    assert_eq!(c0 | GC_SAMP_INT_FILTER, r0, "SAMP_CTRL0 otherwise the RGBA8 one");
}

/// An ETC1 binding as the GL core makes one since 43f99f47: level-major,
/// each level `layers` block grids at its own size on a 64-byte boundary,
/// `bytes` = level 0 of every face. `cpu` is set, as the draw fill sets it.
fn etc1(gpu: u64, w: u32, levels: u8, layers: u16, cpu: &[u8]) -> GlesTextureBind {
    let mut t = bind(gpu, w, w, levels);
    t.format = GLES_TEXFMT_ETC2_RGB;
    t.layers = layers;
    t.bytes = w.div_ceil(4) * w.div_ceil(4) * 8 * layers as u32;
    t.cpu = cpu.as_ptr() as *const c_void;
    t
}

#[test]
fn an_etc1_chain_is_addressed_tiled_at_the_gl_cores_level_offsets() {
    /* 16x16 ETC1, five levels: 4x4, 2x2, 1x1, 1x1, 1x1 blocks of 8 bytes
     * at 0, 128, 192, 256, 320 (worked by hand; gl/texture.rs pins the same
     * offsets in an_etc1_mip_chain_is_accepted_level_by_level_and_is_complete) */
    let rig = Rig::new();
    let mem = vec![0u8; 512];
    let mut d = rig.draw(3);
    d.num_textures = 1;
    d.textures[0] = etc1(rig.vbo.gpu, 16, 5, 1, &mem);
    let desc = rig.submit(&d).descriptor(0);
    let c0 = desc[GC_TEXDESC_CONFIG0 as usize / 4];
    assert_eq!((c0 >> 20) & 0x3, GC_TEX_ADDRESSING_TILED, "CONFIG0 0x{c0:08x}");
    assert_eq!(c0 & texdesc_format(0x1f), texdesc_format(GC_TEX_FORMAT_ETC1), "CONFIG0 0x{c0:08x}");
    assert_eq!(desc[GC_TEXDESC_LINEAR_STRIDE as usize / 4], 4 * 8, "four blocks a row");
    assert_eq!(desc[GC_TEXDESC_SLICE as usize / 4], 4 * 4 * 8, "one block grid");
    for (l, off) in [0u64, 128, 192, 256, 320].iter().enumerate() {
        assert_eq!(desc[texdesc_lod_addr(l as u32) as usize / 4] as u64, rig.vbo.gpu + off, "level {l}");
    }
}

#[test]
fn an_etc1_cube_is_sampled_where_the_gl_core_put_it_not_from_a_tiled_copy() {
    /* 8x8 ETC1 cube, four levels: six 2x2-block faces (192 bytes), then six
     * single blocks per level at 192, 256, 320 (by hand; gl/texture.rs pins
     * the same in etc1_cube_faces_and_levels_are_stored_level_by_level). A
     * face is a block grid, which is the texture unit's padded face step, so
     * the storage is sampled as it is: the tiled copy moves 4-byte texels
     * and would have scrambled the blocks (gles_texfmt_bytes answers 4 for a
     * block format). */
    let rig = Rig::new();
    let mem = vec![0u8; 512];
    let mut d = rig.draw(3);
    d.num_textures = 1;
    d.textures[0] = etc1(rig.vbo.gpu, 8, 4, 6, &mem);
    d.textures[0].type_ = GLES_TEXTYPE_CUBE;
    let desc = rig.submit(&d).descriptor(0);
    assert_eq!((desc[GC_TEXDESC_CONFIG0 as usize / 4] >> 20) & 0x3, GC_TEX_ADDRESSING_TILED);
    assert_eq!(desc[GC_TEXDESC_LINEAR_STRIDE as usize / 4], 2 * 8);
    assert_eq!(desc[GC_TEXDESC_SLICE as usize / 4], 2 * 2 * 8, "one face: the step to the next");
    for (l, off) in [0u64, 192, 256, 320].iter().enumerate() {
        assert_eq!(desc[texdesc_lod_addr(l as u32) as usize / 4] as u64, rig.vbo.gpu + off, "level {l}");
    }
}

#[test]
fn a_draw_whose_descriptor_does_not_fit_fails_instead_of_losing_the_texture() {
    /* The frame arena is a bump allocator. When a descriptor does not fit,
     * `build_descriptor` returns 0 -- and the draw used to be emitted anyway,
     * with the texture slot never programmed. That renders black with no GL
     * error and nothing naming the arena, which is indistinguishable from a
     * texture that was uploaded wrong; twenty gl_test cases failed that way
     * for an afternoon before the cause was found to be the arena.
     *
     * A draw that cannot have its texture must fail, so the GL core can raise
     * GL_OUT_OF_MEMORY, rather than draw something wrong. */
    let rig = Rig::new();
    let mut d = rig.draw(3);

    d.num_textures = 1;
    d.textures[0] = bind(rig.vbo.gpu, 4, 4, 1);

    /* The first draw opens the frame, which resets the arena; fill it only
     * afterwards, so the second draw is the one with nowhere to put its
     * descriptor. */
    // SAFETY: a plain draw through the backend entry point.
    assert_eq!(unsafe { gc_draw(&d) }, 0, "the first draw fits and must succeed");

    /* room for something, but not for a 256-byte descriptor */
    crate::vivante::backend::squeeze_frame_arena_for_tests(64);

    /* Since the arena check before a draw (staging_need), a frame that cannot
     * take the draw's copies and descriptors is submitted first, and the draw
     * goes into the next frame WITH its texture -- better than either
     * failing or drawing without it. The refusal in emit_textures stays for
     * whatever the estimate does not cover. */
    let before = rig.kicks();
    // SAFETY: a plain draw through the backend entry point.
    let rc = unsafe { gc_draw(&d) };
    assert_eq!(rc, 0, "the draw goes into the next frame instead of failing");
    assert_eq!(rig.kicks(), before + 1, "the squeezed frame was submitted first");
    assert_eq!(gc_flush(), 0);
    let mut wrote = false;
    walk(&rig.last_stream().0, |r, _, _| wrote |= r == nte_descriptor_addr(0));
    assert!(wrote, "and the new frame programs the texture");
}

#[test]
fn a_texture_the_pixel_engine_wrote_is_sampled_in_its_own_channel_order() {
    /* the GL core converts a texture to the pixel engine's B,G,R,A the first
     * time a draw renders into it and says so in `bgra`; the descriptor
     * follows `bgra`, and nothing else */
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.num_textures = 1;
    d.textures[0] = bind(rig.vbo.gpu, 4, 4, 1);
    let format_of = |s: &Stream| (s.descriptor(0)[GC_TEXDESC_CONFIG0 as usize / 4] >> 13) & 0x1f;

    assert_eq!(format_of(&rig.submit(&d)), GC_TEX_FORMAT_A8B8G8R8, "the GL core wrote it: R,G,B,A");
    d.textures[0].bgra = 1;
    assert_eq!(format_of(&rig.submit(&d)), GC_TEX_FORMAT_A8R8G8B8, "the pixel engine wrote it: B,G,R,A");
    /* the same storage rendered into, then said to be R,G,B,A (a new upload
     * into a reused block): R,G,B,A */
    let mut into = rig.draw(3);
    into.target_gpu = rig.vbo.gpu;
    rig.submit(&into);
    d.textures[0].bgra = 0;
    assert_eq!(format_of(&rig.submit(&d)), GC_TEX_FORMAT_A8B8G8R8, "what the binding says, not where a render went");
}

/// A CPU upload into storage at an address the pixel engine once wrote (a
/// reused pool block): the GL core says R,G,B,A (`bgra` 0) and that is what
/// is sampled. The render-target ring this backend used to keep said
/// B,G,R,A here, red and blue swapped (hazard 2; dEQP fbo.render on the
/// i.MX8MP, order-dependent).
#[test]
fn a_cpu_upload_at_a_rendered_address_is_sampled_as_the_cpu_wrote_it() {
    let rig = Rig::new();
    let mut into = rig.draw(3);
    into.target_gpu = rig.vbo.gpu;
    rig.submit(&into);
    let mut d = rig.draw(3);
    d.num_textures = 1;
    d.textures[0] = bind(rig.vbo.gpu, 4, 4, 1);
    assert_eq!(d.textures[0].bgra, 0, "the GL core: the CPU's R,G,B,A");
    let s = rig.submit(&d);
    assert_eq!((s.descriptor(0)[GC_TEXDESC_CONFIG0 as usize / 4] >> 13) & 0x1f, GC_TEX_FORMAT_A8B8G8R8);
}

/// A texture whose GL base format has no alpha (the GL core's `no_alpha`:
/// GL_RGB, GL_LUMINANCE, RGB565) samples alpha 1 through the descriptor's
/// swizzle, as Mesa's descriptor for a GL_RGB texture does (golden mode 26,
/// CONFIG1 0x08521000); one with alpha reads its alpha channel.
#[test]
fn a_texture_with_no_alpha_samples_alpha_one() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.num_textures = 1;
    d.textures[0] = bind(rig.vbo.gpu, 4, 4, 1);
    let swizzle_a = |s: &Stream| (s.descriptor(0)[GC_TEXDESC_CONFIG1 as usize / 4] >> 20) & 0x7;

    assert_eq!(swizzle_a(&rig.submit(&d)), GC_SWIZZLE_A, "GL_RGBA: its own alpha");
    d.textures[0].no_alpha = 1;
    assert_eq!(swizzle_a(&rig.submit(&d)), GC_SWIZZLE_1, "GL_RGB: alpha 1");
}

/// A depth texture under GL_OES_depth_texture (`depth_luminance`) samples as
/// luminance, (d, d, d, 1): its R32F descriptor's green and blue swizzles
/// read red. Without the flag (ES 3.0's "treated as RED") they are the
/// identity, and the format and alpha are the same either way.
#[test]
fn a_luminance_depth_texture_swizzles_red_into_green_and_blue() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.num_textures = 1;
    d.textures[0] = bind(rig.vbo.gpu, 4, 4, 1);
    d.textures[0].format = GLES_TEXFMT_R32F;
    d.textures[0].no_alpha = 1;
    let config1 = |s: &Stream| s.descriptor(0)[GC_TEXDESC_CONFIG1 as usize / 4];
    let rgba = |c: u32| ((c >> 8) & 7, (c >> 12) & 7, (c >> 16) & 7, (c >> 20) & 7);
    let red = config1(&rig.submit(&d));
    assert_eq!(rgba(red), (GC_SWIZZLE_R, GC_SWIZZLE_G, GC_SWIZZLE_B, GC_SWIZZLE_1));
    d.textures[0].depth_luminance = 1;
    let lum = config1(&rig.submit(&d));
    assert_eq!(rgba(lum), (GC_SWIZZLE_R, GC_SWIZZLE_R, GC_SWIZZLE_R, GC_SWIZZLE_1));
    assert_eq!(lum & !(0xff << 12), red & !(0xff << 12), "only the green and blue swizzles differ");
}

/// L8, A8 and LA8 stored narrow (gc_debug_narrow_la) are described with the
/// texture unit's own formats (Mesa's A8_UNORM / L8_UNORM / L8A8_UNORM:
/// TEXTURE_FORMAT_A8, L8, A8L8, no swizzle) and their pitch in bytes of
/// their own texel size.
#[test]
fn narrow_luminance_alpha_textures_use_the_units_own_formats() {
    let rig = Rig::new();
    for (texfmt, fmt, bpp) in [
        (GLES_TEXFMT_L8, GC_TEX_FORMAT_L8, 1u32),
        (GLES_TEXFMT_A8, GC_TEX_FORMAT_A8, 1),
        (GLES_TEXFMT_LA8, GC_TEX_FORMAT_A8L8, 2),
    ] {
        let mut d = rig.draw(3);
        d.num_textures = 1;
        d.textures[0] = bind(rig.vbo.gpu, 6, 4, 1);
        d.textures[0].format = texfmt;
        let desc = rig.submit(&d).descriptor(0);
        assert_eq!((desc[GC_TEXDESC_CONFIG0 as usize / 4] >> 13) & 0x1f, fmt, "texfmt {texfmt}");
        assert_eq!(desc[GC_TEXDESC_LINEAR_STRIDE as usize / 4], 6 * bpp, "texfmt {texfmt}: its own texel size");
    }
}

#[test]
fn the_vertex_stages_samplers_start_after_the_fragment_stages() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.num_textures = 1;
    d.textures[0] = bind(rig.vbo.gpu, 4, 4, 1);
    d.num_vs_textures = 1;
    d.vs_textures[0] = bind(rig.vbo.gpu + 0x400, 2, 2, 1);
    let s = rig.submit(&d);

    assert_eq!(s.reg(GC_VS_SAMPLER_BASE), 1, "one fragment sampler comes first");
    assert_ne!(s.reg(nte_descriptor_addr(0)), s.reg(nte_descriptor_addr(1)));
    assert_eq!(s.descriptor(1)[GC_TEXDESC_SIZE as usize / 4], texdesc_size_width(2) | texdesc_size_height(2));
}

/// Nine to sixteen vertex samplers (gc_debug_vs_units16), beside 0, 1 and 16
/// fragment ones: VS_SAMPLER_BASE is the fragment count, vertex sampler k is
/// slot base + k with its own descriptor (each a different size, so a slot
/// that named another sampler's descriptor shows), every slot the draw wrote
/// is invalidated once, and nothing is written past slot 31, the end of the
/// 32-slot array the two stages share (Mesa etnaviv_texture.c:429-439,
/// `etna_vs_sampler_base`: the vertex stage "right after them in the shared
/// 32-entry array"). The backend does not read the switch: it programs what
/// the draw carries, and the switch decides what the GL core lets through.
#[test]
fn nine_to_sixteen_vertex_samplers_take_the_slots_after_the_fragment_ones() {
    let rig = Rig::new();
    for fs in [0usize, 1, 16] {
        for vs in 9..=16usize {
            let mut d = rig.draw(3);
            d.num_textures = fs as u8;
            for (k, t) in d.textures.iter_mut().enumerate().take(fs) {
                *t = bind(rig.vbo.gpu, 4, 4, 1);
                t.unit = k as u8;
            }
            d.num_vs_textures = vs as u8;
            for (k, t) in d.vs_textures.iter_mut().enumerate().take(vs) {
                /* width k + 1: sampler k's descriptor is recognisable */
                *t = bind(rig.vbo.gpu + 0x400 * k as u64, k as u32 + 1, 2, 1);
                t.unit = k as u8;
            }
            let s = rig.submit(&d);
            let what = std::format!("{fs} fragment + {vs} vertex samplers");
            assert_eq!(s.reg(GC_VS_SAMPLER_BASE), fs as u32, "{what}");
            for k in 0..vs {
                let slot = (fs + k) as u32;
                assert_eq!(
                    s.descriptor(slot)[GC_TEXDESC_SIZE as usize / 4],
                    texdesc_size_width(k as u32 + 1) | texdesc_size_height(2),
                    "{what}: vertex sampler {k} in slot {slot}"
                );
            }
            let (mut addr_slots, mut invalidated) = (0u64, std::vec::Vec::new());
            walk(&s.0, |r, v, _| {
                if (nte_descriptor_addr(0)..nte_descriptor_addr(128)).contains(&r) {
                    addr_slots |= 1 << ((r - nte_descriptor_addr(0)) / 4);
                }
                if r == GC_NTE_DESCRIPTOR_INVALIDATE {
                    invalidated.push(v & 0x3f);
                }
            });
            let want = (1u64 << (fs + vs)) - 1;
            assert_eq!(addr_slots, want, "{what}: slots 0..{} and no other", fs + vs - 1);
            invalidated.sort_unstable();
            assert_eq!(invalidated, (0..(fs + vs) as u32).collect::<std::vec::Vec<_>>(), "{what}: one invalidate each");
        }
    }
}

#[test]
fn a_cube_maps_descriptor_names_the_type_and_the_step_to_the_next_face() {
    /* upstream 976c632: the texture unit walks the six faces itself, so the
     * descriptor gets the cube type, one face's size and SLICE as the step
     * from one face to the next. Mesa's descriptor for a 6 x 8x8 cube reads
     * TYPE 5, SIZE 8x8 and SLICE = stride * height. */
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.num_textures = 1;
    d.textures[0] = bind(rig.vbo.gpu, 8, 8, 1);
    d.textures[0].type_ = GLES_TEXTYPE_CUBE;
    d.textures[0].layers = 6;
    let desc = rig.submit(&d).descriptor(0);

    assert_eq!(desc[GC_TEXDESC_CONFIG0 as usize / 4] & 0x7, GC_TEX_TYPE_CUBE_MAP);
    assert_eq!(desc[GC_TEXDESC_SIZE as usize / 4], texdesc_size_width(8) | texdesc_size_height(8));
    assert_eq!(desc[GC_TEXDESC_SLICE as usize / 4], 8 * 4 * 8, "a face is 8 rows of 32 bytes");
    /* a cube is not a volume: textureSize(samplerCube) has no third component */
    assert_eq!(desc[GC_TEXDESC_3D_CONFIG as usize / 4], texdesc_3d_depth(1));
    assert_eq!(desc[GC_TEXDESC_CONFIG1 as usize / 4] & GC_TEXDESC_TEXTURE_ARRAY, 0);
}

#[test]
fn a_2d_array_is_a_volume_with_the_array_bit_set() {
    /* Mesa's translate_texture_target gives an array the 3D type as well, and
     * CONFIG1's TEXTURE_ARRAY bit is the only thing that tells the two apart:
     * an array's layers must not filter into each other. */
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.num_textures = 1;
    d.textures[0] = bind(rig.vbo.gpu, 4, 4, 1);
    d.textures[0].type_ = GLES_TEXTYPE_2D_ARRAY;
    d.textures[0].layers = 3;
    let desc = rig.submit(&d).descriptor(0);

    assert_eq!(desc[GC_TEXDESC_CONFIG0 as usize / 4] & 0x7, GC_TEX_TYPE_3D);
    assert_ne!(desc[GC_TEXDESC_CONFIG1 as usize / 4] & GC_TEXDESC_TEXTURE_ARRAY, 0);
    assert_eq!(desc[GC_TEXDESC_3D_CONFIG as usize / 4], texdesc_3d_depth(3));
    assert_eq!(desc[GC_TEXDESC_VOLUME as usize / 4], 0x180, "log2(3) in 8.8 fixed point");
    assert_eq!(desc[GC_TEXDESC_SLICE as usize / 4], 4 * 4 * 4, "the step to the next layer");
}

#[test]
fn a_3d_texture_is_the_same_volume_without_the_array_bit() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.num_textures = 1;
    d.textures[0] = bind(rig.vbo.gpu, 4, 4, 1);
    d.textures[0].type_ = GLES_TEXTYPE_3D;
    d.textures[0].layers = 3;
    let desc = rig.submit(&d).descriptor(0);

    assert_eq!(desc[GC_TEXDESC_CONFIG0 as usize / 4] & 0x7, GC_TEX_TYPE_3D);
    assert_eq!(desc[GC_TEXDESC_CONFIG1 as usize / 4] & GC_TEXDESC_TEXTURE_ARRAY, 0, "slices do filter together");
    assert_eq!(desc[GC_TEXDESC_3D_CONFIG as usize / 4], texdesc_3d_depth(3));
}

#[test]
fn a_block_formats_rows_are_rows_of_blocks() {
    /* upstream 75e5746: a 16x8 ETC1 texture is four blocks by two. Taking the
     * pitch from stride_texels made it twice too wide and the sampler read the
     * next row of blocks halfway through this one; taking SLICE from the texel
     * height made it four times too far. Mesa writes stride 0x20, SLICE 0x40. */
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.num_textures = 1;
    let mut t = bind(rig.vbo.gpu, 16, 8, 1);
    t.format = GLES_TEXFMT_ETC2_RGB;
    t.bytes = 4 * 2 * 8;
    d.textures[0] = t;
    let desc = rig.submit(&d).descriptor(0);

    assert_eq!(desc[GC_TEXDESC_LINEAR_STRIDE as usize / 4], 4 * 8, "four blocks of 8 bytes");
    assert_eq!(desc[GC_TEXDESC_SLICE as usize / 4], 4 * 8 * 2, "two rows of blocks");
    /* an ETC1 texture has no alpha to read and GL says it samples as 1: Mesa
     * writes SWIZZLE_A = ONE for the same texture */
    assert_eq!((desc[GC_TEXDESC_CONFIG1 as usize / 4] >> 20) & 0x7, GC_SWIZZLE_1);
}

#[test]
fn an_rgba_block_format_keeps_its_own_alpha() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.num_textures = 1;
    let mut t = bind(rig.vbo.gpu, 16, 8, 1);
    t.format = GLES_TEXFMT_ETC2_RGBA;
    t.bytes = 4 * 2 * 16;
    d.textures[0] = t;
    let desc = rig.submit(&d).descriptor(0);

    assert_eq!(desc[GC_TEXDESC_LINEAR_STRIDE as usize / 4], 4 * 16, "four blocks of 16 bytes");
    assert_eq!((desc[GC_TEXDESC_CONFIG1 as usize / 4] >> 20) & 0x7, GC_SWIZZLE_A);
}

#[test]
fn texture_size_reports_the_layers_of_an_array_and_one_for_everything_else() {
    /* textureSize()'s third component reaches the shader as a constant
     * register the backend fills from the bind; it was hard-coded to 1 back
     * when every texture arrived as one flat 2D image. */
    let rig = Rig::new();
    let mut vs = *rig.vs;
    static CONTENTS: [u8; 1] = [SHC_VIV_CONST_TEXTURE_DEPTH];
    static DATA: [u32; 1] = [0]; /* sampler binding 0 */
    vs.viv.const_contents = CONTENTS.as_ptr();
    vs.viv.const_data = DATA.as_ptr();
    vs.viv.const_count = 1;

    let depth_for = |type_: u8, layers: u16| {
        let mut d = rig.draw(3);
        d.vs = &vs;
        d.num_vs_textures = 1;
        d.vs_textures[0] = bind(rig.vbo.gpu, 4, 4, 1);
        d.vs_textures[0].type_ = type_;
        d.vs_textures[0].layers = layers;
        rig.submit(&d).reg(GC_SH_HALTI5_UNIFORMS_VS)
    };

    assert_eq!(depth_for(GLES_TEXTYPE_2D_ARRAY, 7), 7);
    assert_eq!(depth_for(GLES_TEXTYPE_3D, 7), 7);
    assert_eq!(depth_for(GLES_TEXTYPE_CUBE, 6), 1, "a cube map has no third component");
    assert_eq!(depth_for(GLES_TEXTYPE_2D, 1), 1);
}

#[test]
fn the_stencil_faces_swap_with_the_same_rule_the_cull_does() {
    /* upstream 9091596/bd4d440: which half of the pixel engine's two-sided
     * state is "front" is decided by glFrontFace against the target's y flip,
     * exactly as the cull direction is. Sending GL's front state to the PE's
     * front half unconditionally gave a back-facing triangle the front
     * operation whenever exactly one of the two held. */
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.stencil = true;
    d.stencil_func = GLES_CMP_EQUAL;
    d.stencil_back_func = GLES_CMP_NOTEQUAL;
    d.stencil_ref = 1;
    d.stencil_back_ref = 2;
    d.stencil_cmp_mask = 0x0f;
    d.stencil_back_cmp_mask = 0xf0;
    d.stencil_write_mask = 0x11;
    d.stencil_back_write_mask = 0x22;

    /* (write mask 24..31, compare mask 16..23, reference 8..15) */
    let front_of = |s: &Stream| {
        let cfg = s.reg(GC_PE_STENCIL_CONFIG);
        ((cfg >> 24) & 0xff, (cfg >> 16) & 0xff, (cfg >> 8) & 0xff)
    };
    /* the back half: compare mask 0..7, write mask 8..15 */
    let back_of = |s: &Stream| {
        let cfg = s.reg(GC_PE_STENCIL_CONFIG_EXT2);
        ((cfg >> 8) & 0xff, cfg & 0xff)
    };

    /* neither: GL's front is the PE's front */
    d.front_cw = false;
    d.flip_y = false;
    let s = rig.submit(&d);
    assert_eq!(front_of(&s), (0x11, 0x0f, 1), "write mask, compare mask, ref");
    assert_eq!(back_of(&s), (0x22, 0xf0));

    /* both: they cancel, so still GL's front */
    d.front_cw = true;
    d.flip_y = true;
    assert_eq!(front_of(&rig.submit(&d)), (0x11, 0x0f, 1), "both cancel");

    /* exactly one: the faces swap, and the masks go with them */
    d.front_cw = true;
    d.flip_y = false;
    let s = rig.submit(&d);
    assert_eq!(front_of(&s), (0x22, 0xf0, 2), "GL's back state in the PE's front half");
    assert_eq!(back_of(&s), (0x11, 0x0f));
}

#[test]
fn s_and_t_wrap_independently_and_a_one_texel_axis_clamps() {
    /* upstream 239d7f5: the S mode went on both axes, so a texture with S
     * clamped and T repeating was clamped in T too. And a one-texel axis has
     * one texel, so every mode maps every coordinate to it; declaring clamp
     * is what gl_test case 24's unset 1x1 texture needed, which was the one
     * textured case that came back black. */
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.num_textures = 1;
    let wrap_of = |s: &Stream| {
        let c0 = s.reg(nte_descriptor_samp_ctrl0(0));
        (c0 & 0x7, (c0 >> 3) & 0x7)
    };

    d.textures[0] = bind(rig.vbo.gpu, 4, 4, 1);
    for (ws, wt, u, v) in [
        (GLES_WRAP_CLAMP_TO_EDGE, GLES_WRAP_REPEAT, GC_TEX_WRAP_CLAMP_TO_EDGE, GC_TEX_WRAP_REPEAT),
        (GLES_WRAP_REPEAT, GLES_WRAP_CLAMP_TO_EDGE, GC_TEX_WRAP_REPEAT, GC_TEX_WRAP_CLAMP_TO_EDGE),
        /* GL_MIRRORED_REPEAT used to be silently GL_REPEAT */
        (GLES_WRAP_MIRRORED_REPEAT, GLES_WRAP_REPEAT, GC_TEX_WRAP_MIRROR_REPEAT, GC_TEX_WRAP_REPEAT),
        (GLES_WRAP_CLAMP_TO_EDGE, GLES_WRAP_MIRRORED_REPEAT, GC_TEX_WRAP_CLAMP_TO_EDGE, GC_TEX_WRAP_MIRROR_REPEAT),
    ] {
        d.textures[0].wrap_s = ws;
        d.textures[0].wrap_t = wt;
        assert_eq!(wrap_of(&rig.submit(&d)), (u, v), "wrap {ws}/{wt}");
    }

    /* a 1x1 texture that asked for repeat on both axes */
    d.textures[0] = bind(rig.vbo.gpu, 1, 1, 1);
    assert_eq!(
        wrap_of(&rig.submit(&d)),
        (GC_TEX_WRAP_CLAMP_TO_EDGE, GC_TEX_WRAP_CLAMP_TO_EDGE),
        "one texel: clamp is what repeat means there"
    );
}

#[test]
fn a_clear_covers_the_surface_whatever_the_viewport_is() {
    /* glClear is a full-screen quad here, in clip space, and the viewport
     * transform maps clip space onto the viewport rectangle -- so a clear put
     * through the application's viewport clears only that rectangle. Measured
     * on the board before this was fixed: a 32x32 viewport on a 64x64 surface
     * left 3072 of 4096 pixels untouched, first at (32,0), on both pairs with
     * the scissor off and glGetError 0 (cts-runner, clear_viewport.c).
     *
     * ES 2.0 3.7.3 lists the scissor test, the colour mask and the write
     * masks as what affects a clear. Not the viewport: a viewport transforms
     * vertices and a clear has none. The quad is this backend's business.
     *
     * The clear quad is emitted first, so the FIRST write of each viewport
     * register is the clear's and the second is the draw's -- the same
     * reading the depth-range test below uses. */
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.viewport = [16, 8, 32, 16];
    d.scissor = false;
    d.flip_y = false;
    d.do_clear = true;
    let s = rig.submit(&d);

    let first = |reg: u32| {
        let mut v = None;
        walk(&s.0, |r, w, _| {
            if r == reg && v.is_none() {
                v = Some(w);
            }
        });
        v.expect("the register is written")
    };
    /* 64x64 target: the clear's transform is the whole surface, half extent
     * 32 about the centre 32 -- not the viewport's 16 about 32. */
    assert_eq!(first(GC_PA_VIEWPORT_SCALE_X), f2fixp(32.0), "the clear spans the surface, not the viewport");
    assert_eq!(first(GC_PA_VIEWPORT_SCALE_Y), f2fixp(32.0));
    assert_eq!(first(GC_PA_VIEWPORT_OFFSET_X), f2fixp(32.0));
    assert_eq!(first(GC_PA_VIEWPORT_OFFSET_Y), f2fixp(32.0));

    /* and the draw behind it still gets the application's viewport */
    assert_eq!(s.reg(GC_PA_VIEWPORT_SCALE_X), f2fixp(16.0), "the draw keeps the viewport");
    assert_eq!(s.reg(GC_PA_VIEWPORT_OFFSET_X), f2fixp(32.0));
}

/// The scissor still bounds a clear -- through the clipping rectangle, which
/// is where ES 2.0 3.7.3 puts it, and not through the viewport transform.
#[test]
fn a_scissored_clear_is_still_bounded_by_its_box() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.viewport = [0, 0, 64, 64];
    d.scissor = true;
    d.scissor_rect = [8, 4, 16, 8];
    d.flip_y = false;
    d.do_clear = true;
    /* The GL core sets this for a clear the background object cannot carry
     * out -- which is exactly the scissored one. Without it the clear takes
     * the cheap path and covers the target, and this test would be asserting
     * the wrong clear. */
    d.clear_quad = true;
    let s = rig.submit(&d);

    let first = |reg: u32| {
        let mut v = None;
        walk(&s.0, |r, w, _| {
            if r == reg && v.is_none() {
                v = Some(w);
            }
        });
        v.expect("the register is written")
    };
    assert_eq!(first(GC_SE_SCISSOR_LEFT) >> 16, 8);
    assert_eq!(first(GC_SE_SCISSOR_TOP) >> 16, 4);
    assert_eq!(first(GC_SE_SCISSOR_RIGHT) >> 16, 24);
    assert_eq!(first(GC_SE_SCISSOR_BOTTOM) >> 16, 12);
    /* and the transform is still the whole surface: the scissor does the
     * bounding, the viewport transform does not */
    assert_eq!(first(GC_PA_VIEWPORT_SCALE_X), f2fixp(32.0));
}

/// A scissor box wholly OUTSIDE the target -- above, below, left, right, or
/// far enough right that `x << 16` would wrap the SE's 16.16 field -- must
/// come out as an empty rectangle inside the target, never an inverted or an
/// unclamped one; a box overlapping by one pixel must be exactly that pixel,
/// on the right row after the y flip. For a scissored clear (the quad) and a
/// draw, with and without the flip. The PowerVR's version of this (a region
/// clip inverted after the flip) wedged its firmware; dEQP's scissored
/// color_clear cases draw random boxes, some wholly outside.
#[test]
fn a_scissor_box_outside_the_target_is_empty_and_never_inverted() {
    let rig = Rig::new();
    let first = |s: &Stream, reg: u32| {
        let mut v = None;
        walk(&s.0, |r, w, _| {
            if r == reg && v.is_none() {
                v = Some(w);
            }
        });
        v.expect("the register is written")
    };
    /* (x, y, w, h) in GL window coordinates, bottom-up; 64 x 64 target */
    let outside = [
        ("above", [0, 64, 16, 16]),
        ("below", [0, -20, 16, 16]),
        ("left", [-20, 0, 16, 16]),
        ("right", [64, 0, 16, 16]),
        ("far right", [70_000, 0, 16, 16]),
    ];
    for flip in [false, true] {
        for quad in [true, false] {
            let what = |name: &str| format!("{name}, {}, flip {flip}", if quad { "clear quad" } else { "draw" });
            let rect = |b: [i32; 4]| {
                let mut d = rig.draw(if quad { 0 } else { 3 });
                d.flip_y = flip;
                d.scissor = true;
                d.scissor_rect = b;
                if quad {
                    d.do_clear = true;
                    d.clear_quad = true;
                }
                let s = rig.submit(&d);
                [
                    first(&s, GC_SE_SCISSOR_LEFT) >> 16,
                    first(&s, GC_SE_SCISSOR_TOP) >> 16,
                    first(&s, GC_SE_SCISSOR_RIGHT) >> 16,
                    first(&s, GC_SE_SCISSOR_BOTTOM) >> 16,
                ]
            };
            for (name, b) in outside {
                let [l, t, r, bo] = rect(b);
                assert!(l <= r && t <= bo, "{}: inverted [{l},{t})-({r},{bo})", what(name));
                assert!(r <= 64 && bo <= 64, "{}: past the target [{l},{t})-({r},{bo})", what(name));
                assert!(l == r || t == bo, "{}: not empty [{l},{t})-({r},{bo})", what(name));
            }
            /* one pixel: GL (63, 63), the top-right corner */
            let [l, t, r, bo] = rect([63, 63, 5, 5]);
            let row = if flip { 0 } else { 63 };
            assert_eq!([l, t, r, bo], [63, row, 64, row + 1], "{}", what("one-pixel overlap"));
        }
    }
}

#[test]
fn a_clear_quad_is_not_put_through_the_applications_depth_range() {
    /* upstream 8466fc6: the quad already carries GL's window-space clear
     * depth, which is in 0..1; running it through glDepthRangef as well maps
     * it twice. The clear quad is emitted first, so the first write of the
     * two viewport registers is its own and the second is the draw's. */
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.depth_range = [0.25, 0.75];
    d.do_clear = true;
    d.do_clear_depth = true;
    d.clear_depth = 0.5;
    let s = rig.submit(&d);

    let first = |reg: u32| {
        let mut v = None;

        walk(&s.0, |r, w, _| {
            if r == reg && v.is_none() {
                v = Some(f32::from_bits(w));
            }
        });
        v.expect("the register is written")
    };
    /* scale = far - near, offset = near */
    assert_eq!((first(GC_PA_VIEWPORT_SCALE_Z), first(GC_PA_VIEWPORT_OFFSET_Z)), (1.0, 0.0), "the clear quad: 0..1");
    /* and the draw behind it keeps the application's range */
    assert_eq!(
        (f32::from_bits(s.reg(GC_PA_VIEWPORT_SCALE_Z)), f32::from_bits(s.reg(GC_PA_VIEWPORT_OFFSET_Z))),
        (0.5, 0.25),
        "the draw: 0.25..0.75"
    );
}

#[test]
fn depth_is_not_written_while_the_depth_test_is_off() {
    /* GL 4.1.5: with the test off the buffer is not written, whatever
     * glDepthMask says. */
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.depth_write = true;
    d.depth_test = false;
    assert_eq!(rig.submit(&d).reg(GC_PE_DEPTH_CONFIG) & GC_PE_DEPTH_WRITE_ENABLE, 0);
    d.depth_test = true;
    assert_ne!(rig.submit(&d).reg(GC_PE_DEPTH_CONFIG) & GC_PE_DEPTH_WRITE_ENABLE, 0);
}

#[test]
fn multisampling_is_off_and_says_so() {
    /* 4x is present and switched off (GC_MSAA_ENABLE): the resolve engine
     * never wrote the target on this part. A draw that asks for samples gets
     * the single-sampled state and no scratch surface. */
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.samples = 4;
    d.target_w = 64; /* a multiple of 16, so only the switch is stopping it */
    let s = rig.submit(&d);

    assert_eq!(s.reg(GC_GL_MULTI_SAMPLE_CONFIG), GC_MSAA_SAMPLES_NONE | msaa_enables(0xf));
    assert_eq!(s.reg(pe_pipe_color_addr(0)), d.target_gpu as u32, "straight into the target");
    /* alpha to coverage off, every sample written */
    assert_eq!(s.reg(GC_PS_MSAA_CONFIG), !(GC_PS_MSAA_ALPHA_TO_COVERAGE | GC_PS_MSAA_ALPHA_TO_COVERAGE_MASK));
}

/* ---- clears ------------------------------------------------------------- */

#[test]
fn a_clear_is_a_quad_drawn_with_the_backends_own_shaders() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.do_clear = true;
    d.clear_rgba = [1.0, 0.0, 0.0, 1.0];
    d.do_clear_depth = true;
    d.clear_depth = 1.0;
    let s = rig.submit(&d);

    let draws = s.draws();
    assert_eq!(draws.len(), 2, "the clear quad and the draw itself");
    assert_eq!(draws[0], (false, GC_PRIM_TRIANGLE_STRIP, 1, 4, 0));
    /* both draws point the instruction cache somewhere: the quad at the
     * backend's own shaders, the draw at the program's */
    assert_eq!(s.writes(GC_VS_INST_ADDR).1, 2);

    /* a clear-only kick has no geometry of its own */
    let mut only = rig.draw(0);
    only.do_clear = true;
    assert_eq!(rig.submit(&only).draws().len(), 1);
}

/// gl_PointCoord, as Mesa's etnaviv programs it: its row's two components
/// are POINTCOORD_X/Y in GL_VARYING_COMPONENT_USE, SH_SPECIALS' PS_PCOORD_IN
/// is the offset of its first component, and PA_CONFIG's POINT_SPRITE_ENABLE
/// is on. A program without it writes no GL_VARYING_COMPONENT_USE, except on
/// the process's first draw (the GPU keeps what a previous client left) and
/// the first draw after a point-coordinate program: both write all four
/// words GENERIC, so no varying reads a point coordinate it did not ask for.
#[test]
fn gl_point_coord_is_programmed_as_etnaviv_does() {
    let rig = Rig::new();
    let uses = [
        GC_GL_VARYING_COMPONENT_USE0,
        GC_GL_VARYING_COMPONENT_USE1,
        GC_GL_VARYING_COMPONENT_USE2,
        GC_GL_VARYING_COMPONENT_USE3,
    ];
    let written = |st: &Stream| {
        let mut v = std::vec::Vec::new();
        walk(&st.0, |r, w, _| {
            if uses.contains(&r) {
                v.push((r, w));
            }
        });
        v
    };
    /* a fresh backend's first draw writes all four GENERIC, whatever a
     * previous client left in the register; a plain draw after it writes
     * none */
    let first = rig.submit(&rig.draw(3));
    assert_eq!(written(&first), uses.map(|r| (r, 0)), "the process's first draw: GENERIC");
    assert_eq!((first.reg(GC_GL_HALTI5_SH_SPECIALS) >> 8) & 0x7f, GC_SH_SPECIALS_NO_PCOORD);
    let second = rig.submit(&rig.draw(3));
    assert!(written(&second).is_empty(), "never again for it: {:x?}", written(&second));

    let mut fs = *rig.fs;
    fs.viv.num_in = 2;
    fs.viv.in_[0].components = 4;
    crate::util::set_name(&mut fs.viv.in_[0].name, b"v");
    fs.viv.vs_reg_of_in[0] = 1;
    fs.viv.in_[1].components = 2;
    fs.viv.in_[1].interp = 0;
    crate::util::set_name(&mut fs.viv.in_[1].name, crate::shaderc::SHC_VIV_POINT_COORD.as_bytes());
    fs.viv.vs_reg_of_in[1] = SHC_VIV_NO_REG;
    let mut d = rig.draw(3);
    d.fs = &fs;
    d.prim = GLES_PRIM_POINTS;
    let s = rig.submit(&d);
    assert_eq!((s.reg(GC_GL_HALTI5_SH_SPECIALS) >> 8) & 0x7f, 4, "PS_PCOORD_IN: after the vec4");
    assert_eq!(
        written(&s),
        [(uses[0], (2 << 8) | (3 << 10)), (uses[1], 0), (uses[2], 0), (uses[3], 0)],
        "components 4 and 5 are the point coordinate"
    );
    assert_ne!(s.reg(GC_PA_CONFIG) & GC_PA_CONFIG_POINT_SPRITE_ENABLE, 0);

    /* the draw after it: all four words back to GENERIC */
    let after = rig.submit(&rig.draw(3));
    assert_eq!(written(&after), uses.map(|r| (r, 0)), "the transition back");
    assert_eq!((after.reg(GC_GL_HALTI5_SH_SPECIALS) >> 8) & 0x7f, GC_SH_SPECIALS_NO_PCOORD);
    assert_eq!(after.reg(GC_PA_CONFIG) & GC_PA_CONFIG_POINT_SPRITE_ENABLE, 0);

    /* and from then on nothing again */
    let plain = rig.submit(&rig.draw(3));
    assert!(written(&plain).is_empty(), "left alone: {:x?}", written(&plain));
}

/// The audit of a clear fused into a draw: the clear quad takes NONE of the
/// draw's own state, whatever it is. The draw here is hostile in every field
/// the emitters read -- blending on, colour mask off, depth LESS without
/// writes, stencil NEVER / KEEP with masks 0x0f, CW culling, polygon offset,
/// a 4x4 viewport and a squeezed depth range -- and the quad (the first write
/// of each register) must still be the clear: no blending, every channel,
/// depth ALWAYS with writes, stencil ALWAYS / REPLACE under the clear's own
/// mask, no cull, no offset, the whole target, depth 0..1.
#[test]
fn a_fused_clear_takes_none_of_the_draws_own_state() {
    let rig = Rig::new();
    let first = |s: &Stream, reg: u32| {
        let mut v = None;
        walk(&s.0, |r, w, _| {
            if r == reg && v.is_none() {
                v = Some(w);
            }
        });
        v.expect("the register is written")
    };
    let mut d = rig.draw(3);
    d.do_clear = true;
    d.clear_rgba = [0.25, 0.5, 0.75, 1.0];
    d.do_clear_depth = true;
    d.clear_depth = 0.5;
    d.do_clear_stencil = true;
    d.clear_stencil = 7;
    d.clear_stencil_write_mask = 0xff;
    /* the draw's own state, hostile */
    d.blend = true;
    d.blend_src_rgb = 2;
    d.color_mask = 0;
    d.depth_test = true;
    d.depth_write = false;
    d.depth_func = GLES_CMP_LESS;
    d.stencil = true;
    d.stencil_func = GLES_CMP_NEVER;
    d.stencil_back_func = GLES_CMP_NEVER;
    d.stencil_write_mask = 0x0f;
    d.stencil_back_write_mask = 0x0f;
    d.stencil_cmp_mask = 0x0f;
    d.stencil_back_cmp_mask = 0x0f;
    d.cull = GLES_CULL_CW;
    d.polygon_offset = true;
    d.polygon_offset_factor = 2.0;
    d.polygon_offset_units = 4.0;
    d.viewport = [1, 1, 4, 4];
    d.depth_range = [0.25, 0.75];
    let s = rig.submit(&d);

    assert_eq!(first(&s, GC_PE_ALPHA_CONFIG), 0, "no blending");
    assert_eq!((first(&s, GC_PE_COLOR_FORMAT) >> 8) & 0xf, 0xf, "every channel");
    let depth = first(&s, GC_PE_DEPTH_CONFIG);
    assert_eq!((depth >> 8) & 0x7, GLES_CMP_ALWAYS, "depth ALWAYS");
    assert_ne!(depth & GC_PE_DEPTH_WRITE_ENABLE, 0, "depth written");
    let op = first(&s, GC_PE_STENCIL_OP);
    assert_eq!(op & 0x7, GLES_CMP_ALWAYS, "stencil ALWAYS, front");
    assert_eq!((op >> 16) & 0x7, GLES_CMP_ALWAYS, "stencil ALWAYS, back");
    assert_eq!((op >> 4) & 0x7, GC_STENCIL_OP_REPLACE, "REPLACE on pass");
    let cfg = first(&s, GC_PE_STENCIL_CONFIG);
    assert_eq!((cfg >> 24) & 0xff, 0xff, "the clear's write mask");
    assert_eq!((cfg >> 8) & 0xff, 7, "the clear value as the reference");
    assert_eq!((cfg >> 16) & 0xff, 0xff, "compare mask 0xff, not the draw's 0x0f");
    assert_eq!(first(&s, GC_PA_CONFIG) & 0x300, GC_PA_CONFIG_CULL_FACE_OFF, "no cull");
    assert_eq!(first(&s, GC_SE_DEPTH_SCALE), 0, "no polygon offset");
    assert_eq!(first(&s, GC_PA_VIEWPORT_SCALE_X), f2fixp(32.0), "the whole 64-wide target");
    assert_eq!(f32::from_bits(first(&s, GC_PA_VIEWPORT_SCALE_Z)), 1.0, "depth 0..1");
}

/// A sampler the fragment shader declares with nothing usable bound gets the
/// black texel in its slot, exactly as an incomplete binding does: the GL
/// core leaves such a sampler to the backend and the GC7000 wrote nothing,
/// so the slot kept the previous draw's texture (gl_test 61, check 5
/// "nothing bound: opaque black", read 0xff0000ff on the i.MX8MP).
#[test]
fn a_declared_sampler_with_nothing_bound_reads_the_black_texel() {
    static SAMPLER: [crate::shaderc::ShcUniform; 1] = [crate::shaderc::ShcUniform {
        dword_off: crate::shaderc::SHC_UNIFORM_NO_BLOCK,
        gl_type: 0x8b5e, /* GL_SAMPLER_2D */
        binding: 0,
        ..crate::shaderc::ShcUniform::ZERO
    }];
    let rig = Rig::new();
    /* every register write, with the descriptor ADDRESSES normalised: each
     * submit is its own frame, whose arena puts the descriptor elsewhere */
    let desc0 = nte_descriptor_addr(0);
    let all = |st: &Stream| {
        let mut v = std::vec::Vec::new();
        walk(&st.0, |r, w, _| v.push((r, if r == desc0 { 0 } else { w })));
        v
    };
    /* past the process's first draw, which writes GL_VARYING_COMPONENT_USE
     * once and would make the first stream compared differ for that alone */
    let _ = rig.submit(&rig.draw(3));
    /* a shader declaring sampler 0, drawn with nothing bound */
    let mut fs = *rig.fs;
    fs.uniforms = SAMPLER.as_ptr();
    fs.num_uniforms = 1;
    let mut d = rig.draw(3);
    d.fs = &fs;
    let unbound = rig.submit(&d);
    /* the same draw with an incomplete binding for slot 0: the black texel */
    let mut e = rig.draw(3);
    e.fs = &fs;
    e.num_textures = 1;
    e.textures[0] = GlesTextureBind { unit: 0, incomplete: 1, ..GlesTextureBind::ZERO };
    let incomplete = rig.submit(&e);
    assert_eq!(all(&unbound), all(&incomplete), "nothing bound is programmed as the incomplete texture");
    /* and the slot is written at all */
    let mut wrote = false;
    walk(&unbound.0, |r, _, _| wrote |= r == desc0);
    assert!(wrote, "slot 0 gets a descriptor");
    assert_eq!(unbound.reg(GC_VS_SAMPLER_BASE), 1, "the vertex slots after the declared one");
}

/// Draw buffers (MRT step 2). A draw given four targets programs RT1..3 in
/// the 8-RT bank as etnaviv does -- address, stride and format, the draw's
/// colour mask and blend -- PS_OUTPUT_REG names each target's register from
/// the shader's output rows, PS_CONTROL counts and saturates all four; and a
/// one-target draw after it is back to RT_COUNT(0) with no bank write.
#[test]
fn draw_buffers_program_the_8_rt_bank() {
    let rig = Rig::new();
    let bank = |st: &Stream| {
        let mut v = std::vec::Vec::new();
        walk(&st.0, |r, w, _| {
            if (0x14800..0x14a00).contains(&r) {
                v.push((r, w));
            }
        });
        v
    };
    let mut fs = *rig.fs;
    fs.viv.ps_color_out_reg = 3;
    fs.viv.num_out = 4;
    for (i, reg) in [3u8, 4, 5, 6].iter().enumerate() {
        fs.viv.out[i] = ShcVivIo { reg: *reg, slot: 4 + i as u8, components: 4, interp: 0, name: [0; 32] };
    }
    let mut d = rig.draw(3);
    d.fs = &fs;
    d.num_targets = 4;
    for rt in 1..4usize {
        d.targets[rt] =
            GlesDrawTarget { gpu: TARGET_GPU + 0x10_0000 * rt as u64, w: 64, h: 64, format: d.target_format };
    }
    let _ = rig.submit(&rig.draw(3)); /* past the first draw's one-off writes */

    let mut one = d;
    one.num_targets = 1;
    let on = rig.submit(&d);
    let after = rig.submit(&one);

    let (fmt, _) = pe_color_format(d.target_format);
    let pitch = 64 * gles_surf_bytes_per_pixel(d.target_format);
    for rt in 1..4u32 {
        assert_eq!(on.reg(pe_rt_addr_8_pipe(rt, 0)), (TARGET_GPU + 0x10_0000 * rt as u64) as u32, "RT{rt} address");
        assert_eq!(on.reg(pe_rt_config_8(rt)), rt_config_stride(pitch) | rt_config_format(fmt), "RT{rt} config");
        assert_eq!(
            on.reg(pe_halti5_rt_colormask(rt)),
            pe_halti5_rt_colormask_components(0xf) | GC_PE_HALTI5_RT_COLORMASK_OVERWRITE,
            "RT{rt} mask"
        );
        assert_eq!(on.reg(pe_halti5_rt_alpha_config(rt)), 0, "RT{rt}: no blend, as RT0");
    }
    assert_eq!(on.reg(GC_PS_OUTPUT_REG), 3 | 4 << 8 | 5 << 16 | 6 << 24, "each target's register");
    let ctl = on.reg(GC_PS_CONTROL);
    assert_eq!((ctl >> 8) & 0x7, 3, "RT_COUNT(3): four targets");
    assert_eq!(ctl & 0x1e, 0x1e, "all four unorm targets saturated");

    assert!(bank(&after).is_empty(), "a one-target draw writes no bank: {:x?}", bank(&after));
    assert_eq!((after.reg(GC_PS_CONTROL) >> 8) & 0x7, 0, "and RT_COUNT(0) disables the rest");
}

/// A draw buffer the GL core carries with no target (`gpu` 0: a GL_NONE
/// draw buffer between two attachments, glDrawBuffers(ATT0, NONE, ATT2)) is
/// not a render target here. With targets (A, none, C) the stream programs
/// two: RT1 is C -- its address, stride and format -- and takes the shader's
/// output for slot 2, RT_COUNT(1) counts two, only RT0 and RT1 are saturated,
/// and nothing at all is written for a third target. The middle slot's
/// output is stored nowhere, as Mesa's etnaviv does for a null colour buffer.
#[test]
fn a_draw_buffer_with_no_target_is_skipped_and_the_next_moves_down() {
    let rig = Rig::new();
    let mut fs = *rig.fs;
    fs.viv.ps_color_out_reg = 3;
    fs.viv.num_out = 3;
    for (i, reg) in [3u8, 4, 5].iter().enumerate() {
        fs.viv.out[i] = ShcVivIo { reg: *reg, slot: 4 + i as u8, components: 4, interp: 0, name: [0; 32] };
    }
    const C_GPU: u64 = TARGET_GPU + 0x20_0000;
    let mut d = rig.draw(3);
    d.fs = &fs;
    d.num_targets = 3;
    d.targets[1] = GlesDrawTarget::ZERO;
    /* C is wider than RT0 (a smaller one is refused, see
     * a_later_target_smaller_than_the_first_is_refused), so its stride
     * differs from RT0's */
    d.targets[2] = GlesDrawTarget { gpu: C_GPU, w: 128, h: 128, format: d.target_format };
    let _ = rig.submit(&rig.draw(3)); /* past the first draw's one-off writes */

    let holed = rig.submit(&d);

    let written = |st: &Stream, reg: u32| {
        let mut n = 0;
        walk(&st.0, |r, _, _| n += usize::from(r == reg));
        n
    };
    let (fmt, _) = pe_color_format(d.target_format);
    let pitch = 128 * gles_surf_bytes_per_pixel(d.target_format);
    assert_eq!(holed.reg(pe_rt_addr_8_pipe(1, 0)), C_GPU as u32, "RT1 is C, not the empty slot");
    assert_eq!(
        holed.reg(pe_rt_config_8(1)),
        rt_config_stride(pitch) | rt_config_format(fmt),
        "RT1: C's stride and format"
    );
    for rt in 2..4u32 {
        assert_eq!(written(&holed, pe_rt_addr_8_pipe(rt, 0)), 0, "no address for RT{rt}");
        assert_eq!(written(&holed, pe_rt_config_8(rt)), 0, "no config for RT{rt}");
        assert_eq!(written(&holed, pe_halti5_rt_colormask(rt)), 0, "no mask for RT{rt}");
    }
    let addr_regs: std::vec::Vec<u32> =
        (1..8u32).flat_map(|rt| (0..2u32).map(move |p| pe_rt_addr_8_pipe(rt, p))).collect();
    let mut zero = std::vec::Vec::new();
    walk(&holed.0, |r, w, _| {
        if addr_regs.contains(&r) && w == 0 {
            zero.push(r);
        }
    });
    assert!(zero.is_empty(), "no render target programmed at address 0: {zero:x?}");
    assert_eq!(holed.reg(GC_PS_OUTPUT_REG), 3 | 5 << 8, "RT1 takes slot 2's output register (5), not slot 1's (4)");
    let ctl = holed.reg(GC_PS_CONTROL);
    assert_eq!((ctl >> 8) & 0x7, 1, "RT_COUNT(1): two targets");
    assert_eq!(ctl & 0x1e, 0x06, "RT0 and RT1 saturated, no third");
}

/// A draw into the pair (depth object, stencil object), each `(id, gen)`,
/// that tests and writes both planes.
fn pair_draw(rig: &Rig, depth: (u32, u32), stencil: (u32, u32)) -> GlesDraw {
    let mut d = rig.draw(3);
    (d.depth_id, d.depth_gen, d.stencil_id, d.stencil_gen) = (depth.0, depth.1, stencil.0, stencil.1);
    (d.depth_test, d.depth_write, d.depth_func) = (true, true, GLES_CMP_LESS);
    (d.stencil, d.stencil_func, d.stencil_dppass) = (true, GLES_CMP_ALWAYS, GLES_SOP_REPLACE);
    (d.stencil_write_mask, d.stencil_back_write_mask, d.stencil_cmp_mask) = (0xff, 0xff, 0xff);
    d
}

fn ds_key(depth: (u32, u32), stencil: (u32, u32)) -> ds_cache::DsKey {
    ds_cache::DsKey { depth_id: depth.0, depth_gen: depth.1, stencil_id: stencil.0, stencil_gen: stencil.1, scale: 1 }
}

/// The D24S8 words of a pair's surface, as the pixel engine left them: the
/// mock GPU draws nothing, so a test writes what a draw would have.
fn ds_words(key: ds_cache::DsKey) -> &'static mut [u32] {
    let (mem, bytes) = ds_cache::memory_of(key).expect("the pair has a surface");
    // SAFETY: the surface's CPU mapping (host memory in the rig), past the
    // guard, `bytes` long; no kick runs while a test reads it.
    unsafe { core::slice::from_raw_parts_mut((mem.cpu as *mut u8).add(GC_DEPTH_GUARD_BYTES) as *mut u32, bytes / 4) }
}

/// A shared depth: two framebuffers share the depth
/// renderbuffer 5 and each has a stencil renderbuffer of its own (6, 7).
/// Each pair has its own surface; the depth moves to whichever is drawn
/// and each stencil stays where it was written, so swapping the stencil
/// back and forth keeps both stencils and the latest depth.
///
/// Revert check: keyed by the depth object alone the pairs share one
/// surface, and stencil 6 reads 0x22 after (5, 7) was drawn.
#[test]
fn a_shared_depth_follows_its_object_and_each_stencil_keeps_its_own() {
    let rig = Rig::new();
    let (a, b) = (ds_key((5, 1), (6, 1)), ds_key((5, 1), (7, 1)));
    let a_addr = depth_addr(&rig.submit(&pair_draw(&rig, (5, 1), (6, 1))));
    ds_words(a).fill(0xaaaa_aa11); /* depth 0xaaaaaa, stencil 0x11 */
    let b_addr = depth_addr(&rig.submit(&pair_draw(&rig, (5, 1), (7, 1))));
    assert_ne!(b_addr, a_addr, "a surface for each pair");
    assert!(ds_words(b).iter().all(|w| w >> 8 == 0xaa_aaaa), "the shared depth was copied in");
    assert_eq!(ds_cache::planes_of(a), Some([Plane::UNDEFINED, Plane::LIVE]), "6's stencil stays in A");
    ds_words(b).iter_mut().for_each(|w| *w = 0xbbbb_bb22); /* B's draw: depth 0xbbbbbb, stencil 0x22 */
    assert_eq!(depth_addr(&rig.submit(&pair_draw(&rig, (5, 1), (6, 1)))), a_addr);
    assert!(ds_words(a).iter().all(|&w| w == 0xbbbb_bb11), "the latest depth, and stencil 6's own 0x11");
    assert_eq!(ds_cache::planes_of(b), Some([Plane::UNDEFINED, Plane::LIVE]), "7's stencil stays in B");
    assert_eq!(depth_addr(&rig.submit(&pair_draw(&rig, (5, 1), (7, 1)))), b_addr);
    assert!(ds_words(b).iter().all(|&w| w & 0xff == 0x22), "stencil 7 kept its 0x22");
    assert!(rig.faults().is_empty());
}

/// A shared stencil: two framebuffers share the stencil
/// renderbuffer 6 with depth objects of their own (5, 8). The stencil moves
/// with its object and each depth stays with its own.
#[test]
fn a_shared_stencil_follows_its_object_and_each_depth_keeps_its_own() {
    let rig = Rig::new();
    let (a, b) = (ds_key((5, 1), (6, 1)), ds_key((8, 1), (6, 1)));
    rig.submit(&pair_draw(&rig, (5, 1), (6, 1)));
    ds_words(a).fill(0x1111_1133);
    rig.submit(&pair_draw(&rig, (8, 1), (6, 1)));
    assert!(ds_words(b).iter().all(|&w| w & 0xff == 0x33), "the shared stencil was copied in");
    ds_words(b).iter_mut().for_each(|w| *w = 0x2222_2244);
    rig.submit(&pair_draw(&rig, (5, 1), (6, 1)));
    assert!(ds_words(a).iter().all(|&w| w == 0x1111_1144), "depth 5's own, and the stencil's latest 0x44");
    assert!(rig.faults().is_empty());
}

/// A new generation of the depth object is new storage (its contents are
/// gone), and stencil-only framebuffers are keyed by their stencil object.
#[test]
fn a_respecified_depth_and_stencil_only_pairs_get_surfaces_of_their_own() {
    let rig = Rig::new();
    let a = depth_addr(&rig.submit(&pair_draw(&rig, (5, 1), (6, 1))));
    let respecified = depth_addr(&rig.submit(&pair_draw(&rig, (5, 2), (6, 1))));
    let stencil_only = depth_addr(&rig.submit(&pair_draw(&rig, (0, 0), (6, 2))));
    let other_stencil = depth_addr(&rig.submit(&pair_draw(&rig, (0, 0), (8, 1))));
    assert_ne!(respecified, a, "new depth storage");
    assert_ne!(stencil_only, other_stencil, "stencil-only framebuffers by their stencil object");
}

/// Pair `i` of the cycling tests: its own colour target and one D24S8
/// object (100 + i) on both points.
fn cycled(rig: &Rig, i: u32) -> GlesDraw {
    let mut d = pair_draw(rig, (100 + i, 1), (100 + i, 1));
    d.target_gpu = TARGET_GPU + 0x10_0000 * u64::from(i + 1);
    d
}

/// The depth/stencil line the OSAL prints at close
/// (`gc_backend_ds_report`), counted from a clean start.
fn gc_ds_line() -> String {
    let mut buf = [0 as core::ffi::c_char; 160];
    // SAFETY: buf is writable for its length.
    unsafe { ds_cache::gc_backend_ds_report(buf.as_mut_ptr(), buf.len()) };
    // SAFETY: the report NUL-terminates inside buf.
    unsafe { core::ffi::CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned()
}

/// One pair more than there are surfaces: when they all hold nothing GL keeps
/// (depth tested, nothing written), the one past them takes one and the line
/// counts it as given up; when they all hold a plane, it is refused and
/// counted as all live. Where the peak alone reads full, the given-up count
/// is what says an application uses more pairs than there are surfaces.
#[test]
fn the_gc_ds_line_counts_surfaces_given_up_and_refusals() {
    let rig = Rig::new();
    ds_cache::release_all();
    ds_cache::reset_counts();
    assert_eq!(gc_ds_line(), "ds surfaces: none allocated, ds given up 0, ds refused 0 (all live 0, no surface 0)");
    let quiet = |i: u32| {
        let mut d = cycled(&rig, i);
        d.depth_write = false;
        d.stencil = false;
        d
    };
    for i in 0..E {
        rig.submit(&quiet(i));
    }
    let full = gc_ds_line();
    let peak = format!("ds surfaces peak {E}/{E}, ds bytes peak ");
    assert!(full.starts_with(&peak), "{full}");
    assert!(full.ends_with(", ds given up 0, ds refused 0 (all live 0, no surface 0)"), "{full}");
    rig.submit(&quiet(E));
    assert!(gc_ds_line().ends_with(", ds given up 1, ds refused 0 (all live 0, no surface 0)"), "{}", gc_ds_line());

    ds_cache::release_all();
    ds_cache::reset_counts();
    for i in 0..E {
        rig.submit(&cycled(&rig, 10 + i));
    }
    let mut past = cycled(&rig, 10 + E);
    past.do_clear = false;
    // SAFETY: a live draw for the call.
    assert_eq!(unsafe { gc_draw(&past) }, -ENOMEM, "all eight hold a plane GL keeps: refused");
    assert!(gc_ds_line().ends_with(", ds given up 0, ds refused 1 (all live 1, no surface 0)"), "{}", gc_ds_line());
}

/// `depth_resolve` reads the depth a kick left for an object as the
/// texture's floats: bits 31..8 of each D24S8 word (the stencil byte
/// ignored), row-linear at the surface's row stride (measured, ds_cache.rs
/// `resolve_depth`). A whole clear is its
/// value everywhere, here and once parked; an object nothing holds answers
/// 1 and leaves the texels.
#[test]
fn depth_resolve_reads_the_linear_depth_as_floats() {
    let rig = Rig::new();
    let id = 0x4000_0064; /* a texture's id form */
    rig.submit(&pair_draw(&rig, (id, 1), (id, 1)));
    let key = ds_key((id, 1), (id, 1));
    let words = ds_words(key);
    let tw = ds_cache::stride_of(key).expect("a surface") / 4;
    assert!(tw >= 8 && tw.is_multiple_of(64), "{tw}");
    let at = |x: u32, y: u32| (y * tw + x) as usize;
    words.fill(0);
    words[at(5, 2)] = 0x8000_00ab; /* (5, 2): depth 2^23, stencil 0xab */
    words[at(1, 6)] = 0xffff_ff00; /* (1, 6): depth 1.0 */
    let mut t = [7.0f32; 9 * 8];
    // SAFETY: 8 rows of 9 floats.
    assert_eq!(unsafe { gc_depth_resolve(id, t.as_mut_ptr(), 8, 8, 9) }, 0);
    assert_eq!(t[2 * 9 + 5], 8_388_608.0 / 16_777_215.0);
    assert_eq!(t[6 * 9 + 1], 1.0);
    assert_eq!(t[0], 0.0);
    assert_eq!(t[8], 7.0, "the pitch's padding is not written");
    // SAFETY: as above.
    assert_eq!(unsafe { gc_depth_resolve(id + 1, t.as_mut_ptr(), 8, 8, 9) }, 1, "nothing held for it");
    /* a whole clear */
    let mut clear = pair_draw(&rig, (id, 1), (id, 1));
    (clear.index_count, clear.do_clear_depth, clear.clear_depth) = (0, true, 0.25);
    (clear.do_clear_stencil, clear.clear_stencil) = (true, 3); /* nothing live left: the surface may go */
    rig.submit(&clear);
    // SAFETY: as above.
    assert_eq!(unsafe { gc_depth_resolve(id, t.as_mut_ptr(), 8, 8, 9) }, 0);
    assert_eq!((t[0], t[7 * 9 + 7], t[8]), (0.25, 0.25, 7.0));
    /* parked: the surface let go, its clear value kept */
    for i in 0..ds_cache::ENTRIES as u32 {
        rig.submit(&cycled(&rig, i));
    }
    assert!(ds_cache::memory_of(key).is_none(), "the cleared surface went to another pair");
    t = [7.0; 72];
    // SAFETY: as above.
    assert_eq!(unsafe { gc_depth_resolve(id, t.as_mut_ptr(), 8, 8, 9) }, 0);
    assert_eq!((t[0], t[70]), (0.25, 0.25));
    assert!(rig.faults().is_empty());
}

/// The table's size, as a pair index: the first pair past it.
const E: u32 = ds_cache::ENTRIES as u32;

/// Draw pairs 0..E (the table's size), each writing depth and stencil into
/// a surface of its own, and mark pair 0's.
fn full_table_drawn(rig: &Rig) {
    for i in 0..E {
        rig.submit(&cycled(rig, i));
    }
    ds_words(ds_key((100, 1), (100, 1))).fill(0x5a5a_5a07);
}

/// The refusal (src/ds_retention.rs): with every surface holding a live
/// plane, the first pair past the table's first depth-tested draw is refused (-ENOMEM, which
/// the GL core reports as GL_OUT_OF_MEMORY) with one named line, and pair
/// 0's depth and stencil are intact and drawn with again.
///
/// Revert check: with the old least-recently-used replacement the draw past
/// the table returns 0 and pair 0's surface is gone.
#[test]
fn a_pair_past_the_table_is_refused_and_the_first_keeps_its_depth_and_stencil() {
    let rig = Rig::new();
    full_table_drawn(&rig);
    let first = ds_key((100, 1), (100, 1));
    let first_addr = ds_cache::memory_of(first).map(|(m, _)| m.gpu);
    // SAFETY: a live draw for the call.
    assert_eq!(unsafe { gc_draw(&cycled(&rig, E)) }, -ENOMEM, "the pair past the table is refused");
    let logs = rig._gl.logs();
    let refused: Vec<_> = logs.iter().filter(|l| l.contains(ds_retention::REFUSED)).collect();
    assert_eq!(refused.len(), 1, "{logs:?}");
    let (object, all) = (format!("object {:#x}", 100 + E), format!("all {E}"));
    assert!(refused[0].contains(&object) && refused[0].contains(&all), "{refused:?}");
    assert!(ds_words(first).iter().all(|&w| w == 0x5a5a_5a07), "pair 0's depth and stencil are intact");
    rig.submit(&cycled(&rig, 0));
    assert_eq!(ds_cache::memory_of(first).map(|(m, _)| m.gpu), first_addr, "pair 0 draws with its own surface");
    assert!(rig.faults().is_empty());
}

/// Deleting pair 0's object frees its surface for the pair past the table; a whole clear
/// of pair 1 with no draw after it lets its surface go too (the clear
/// values parked), and when pair 1 is drawn again its new surface gets them
/// back through a clear recorded before the draw.
#[test]
fn a_deleted_or_cleared_pair_lets_its_surface_go() {
    let rig = Rig::new();
    full_table_drawn(&rig);
    gc_attachment_gone(100);
    assert!(ds_cache::memory_of(ds_key((100, 1), (100, 1))).is_none(), "object 100's surface is freed");
    rig.submit(&cycled(&rig, E));
    let mut clear = cycled(&rig, 1);
    (clear.index_count, clear.do_clear_depth, clear.clear_depth) = (0, true, 0.25);
    (clear.do_clear_stencil, clear.clear_stencil) = (true, 3);
    rig.submit(&clear);
    let cleared = [Plane::cleared(0.25f32.to_bits()), Plane::cleared(3)];
    assert_eq!(ds_cache::planes_of(ds_key((101, 1), (101, 1))), Some(cleared));
    rig.submit(&cycled(&rig, E + 1)); /* takes pair 1's surface */
    assert_eq!(ds_cache::parked_len(), 2, "pair 1's depth and stencil clear values");
    gc_attachment_gone(102);
    let mut test_only = cycled(&rig, 1);
    (test_only.depth_write, test_only.stencil) = (false, false);
    let s = rig.submit(&test_only);
    assert_eq!(s.draws().len(), 2, "the restoring clear, then the draw: {:?}", s.draws());
    assert_eq!(ds_cache::planes_of(ds_key((101, 1), (101, 1))), Some(cleared), "still only the clear");
    assert_eq!(ds_cache::parked_len(), 0);
    assert!(rig.faults().is_empty());
}

/// A clear writes only the draw buffers `clear_target_mask` names:
/// glClearBuffer(GL_COLOR, 1) leaves RT0 alone, glClear with every draw
/// buffer GL_NONE (0x80) writes none, and 0 (glClear) writes them all. RT1's
/// rows are at the pair-padded pitch, as RT0's (a1343e19): a 63-wide target
/// is 64 pixels a row.
#[test]
fn a_clear_writes_only_the_draw_buffers_its_mask_names() {
    let rig = Rig::new();
    let mut d = rig.draw(0);
    d.target_w = 63;
    d.viewport = [0, 0, 63, 64];
    d.do_clear = true;
    d.clear_color_mask = 0xf;
    d.num_targets = 2;
    d.targets[1] = GlesDrawTarget { gpu: TARGET_GPU + 0x10_0000, w: 63, h: 64, format: d.target_format };
    let rt0 = |s: &Stream| (s.reg(GC_PE_COLOR_FORMAT) >> 8) & 0xf;
    let rt1 = |s: &Stream| (s.reg(pe_halti5_rt_colormask(1)) >> 4) & 0xf;
    let mut seen = std::vec::Vec::new();
    for mask in [0u8, 0b10, 0b01, 0x80] {
        d.clear_target_mask = mask;
        let st = rig.submit(&d);
        seen.push((mask, rt0(&st), rt1(&st), st.reg(pe_rt_config_8(1))));
    }
    let (fmt, _) = pe_color_format(d.target_format);
    let cfg = rt_config_stride(64 * gles_surf_bytes_per_pixel(d.target_format)) | rt_config_format(fmt);
    assert_eq!(
        seen,
        [(0, 0xf, 0xf, cfg), (0b10, 0, 0xf, cfg), (0b01, 0xf, 0, cfg), (0x80, 0, 0, cfg)],
        "(mask, RT0 channels, RT1 channels, RT1 config)"
    );
}

/// A stencil clear fused into a draw is written through the stencil write
/// mask of its own glClear (`clear_stencil_write_mask`), and the draw behind
/// it through the draw's: gl_test 9 frame 5, where the clear under 0xff took
/// the draw's 0xfe and kept bit 0. 0 means "not given" and falls back to the
/// draw's mask, as before the field.
#[test]
fn a_fused_stencil_clear_uses_its_own_write_mask() {
    let rig = Rig::new();
    let masks = |s: &Stream| {
        let mut v = std::vec::Vec::new();
        walk(&s.0, |r, w, _| {
            if r == GC_PE_STENCIL_CONFIG {
                v.push((w >> 24) & 0xff);
            }
        });
        v
    };
    let mut d = rig.draw(3);
    d.stencil = true;
    d.stencil_write_mask = 0xfe;
    d.stencil_back_write_mask = 0xfe;
    d.do_clear_stencil = true;
    d.clear_stencil = 0;
    d.clear_stencil_write_mask = 0xff;
    let s = rig.submit(&d);
    assert_eq!(masks(&s), [0xff, 0xfe], "the clear quad first with the glClear's mask, then the draw with its own");

    d.clear_stencil_write_mask = 0;
    assert_eq!(masks(&rig.submit(&d)), [0xfe, 0xfe], "not given: the draw's mask, as before");
}

/// A colour clear writes the channels of its own glClear's mask, through the
/// same PE_COLOR_FORMAT COMPONENTS field a masked draw uses (all four, the
/// behaviour before it, is what dEQP's masked color_clear cases fail).
#[test]
fn a_masked_clear_writes_only_its_channels() {
    let rig = Rig::new();
    let mut d = rig.draw(0);
    d.do_clear = true;
    d.clear_color_mask = 0b1001;
    let comps = |s: &Stream| (s.reg(GC_PE_COLOR_FORMAT) >> 8) & 0xf;
    let overwrite = |s: &Stream| s.reg(GC_PE_COLOR_FORMAT) & GC_PE_COLOR_FORMAT_OVERWRITE;

    let on = rig.submit(&d);

    let mut whole = d;
    whole.clear_color_mask = 0xf;
    let on_whole = rig.submit(&whole);
    let mut unset = d;
    unset.clear_color_mask = 0;
    let on_unset = rig.submit(&unset);
    /* fused into a draw whose own mask is different */
    let mut fused = rig.draw(3);
    fused.do_clear = true;
    fused.clear_color_mask = 0b0110;
    fused.color_mask = 0xf;
    let on_fused = rig.submit(&fused);

    assert_eq!(comps(&on), 0b1001, "R and A only");
    assert_eq!(overwrite(&on), 0, "a masked clear keeps the other channels");
    assert_eq!(comps(&on_whole), 0xf);
    assert_ne!(overwrite(&on_whole), 0, "an unmasked clear keeps the fast path");
    assert_eq!(comps(&on_unset), 0xf, "0 is 'not given': every channel");
    let mut firsts = std::vec::Vec::new();
    walk(&on_fused.0, |r, w, _| {
        if r == GC_PE_COLOR_FORMAT {
            firsts.push((w >> 8) & 0xf);
        }
    });
    assert_eq!(firsts.first(), Some(&0b0110), "the clear quad, first: the clear's own mask");
    assert_eq!(comps(&on_fused), 0xf, "the draw behind it: the draw's mask");
}

#[test]
fn a_clear_without_a_colour_writes_no_channel() {
    let rig = Rig::new();
    let mut d = rig.draw(0);
    d.do_clear_depth = true;
    d.clear_depth = 0.5;
    let s = rig.submit(&d);

    assert_eq!((s.reg(GC_PE_COLOR_FORMAT) >> 8) & 0xf, 0, "colour masked off");
    assert_ne!(s.reg(GC_PE_DEPTH_CONFIG) & GC_PE_DEPTH_WRITE_ENABLE, 0);
    assert_eq!((s.reg(GC_PE_DEPTH_CONFIG) >> 8) & 0x7, GLES_CMP_ALWAYS, "a clear passes every fragment");
}

/* ---- fences ------------------------------------------------------------- */

#[test]
fn a_kick_is_pending_until_its_fence_completes() {
    let rig = Rig::new();
    let d = rig.draw(3);

    /* the id is the kick being recorded, as the GL core stamps it */
    let id = gc_kick_id();
    assert_eq!(id, 1);
    assert!(!gc_kick_pending(id), "nothing is recorded yet");
    // SAFETY: `d` is a live draw for the length of the call.
    assert_eq!(unsafe { gc_draw(&d) }, 0);
    assert_eq!(gc_kick_id(), id, "a draw goes into the open frame");
    assert!(gc_kick_pending(id), "the open frame is not even submitted");
    assert_eq!(gc_flush(), 0);
    assert_eq!(gc_kick_id(), id + 1, "the next frame is the one recorded now");
    /* the rig's completed() moves with every submit, so nothing is pending
     * once it has run; a fence ahead of it is */
    with_platform(|p| p.fence = 0);
    assert!(gc_kick_pending(id));
    assert!(!gc_kick_pending(id + 1), "nothing recorded in the next frame yet");
    assert!(!gc_kick_pending(id + 2), "a kick that does not exist yet");
    assert!(!gc_kick_pending(0));
    with_platform(|p| p.fence = 1);
    assert!(!gc_kick_pending(id));
    assert_eq!(gc_finish(), 0);
}

#[test]
fn a_texture_the_open_frame_samples_is_not_written_under_it() {
    /* The GL core decides whether glTexSubImage2D may write a texture's
     * storage in place from kick_pending(last_use), and stamps last_use with
     * kick_id() at the draw. With kick_id() answering the last SUBMITTED
     * kick, a draw recorded into the open frame was stamped with the frame
     * before it; once that one completed, the write went in place and the
     * draw, submitted later, sampled the new texels. */
    let rig = Rig::new();
    let d = rig.draw(3);
    rig.submit(&d); /* kick 1 runs and completes (the rig's fence) */
    // SAFETY: `d` is a live draw for the length of the call.
    assert_eq!(unsafe { gc_draw(&d) }, 0);
    let stamp = gc_kick_id(); /* what the GL core stamps this draw's texture with */
    assert!(gc_kick_pending(stamp), "the draw that samples the texture has not run");
    assert_eq!(gc_flush(), 0);
    assert!(!gc_kick_pending(stamp), "and once it has, the texture is free");
}

#[test]
fn the_counters_report_the_kicks_and_the_waits() {
    let rig = Rig::new();
    let d = rig.draw(3);
    let (mut kicks, mut waits) = (0u32, 0u32);

    for _ in 0..GC_FRAMES {
        rig.submit(&d);
    }
    // SAFETY: both are writable.
    unsafe { gc_counters(&mut kicks, &mut waits) };
    assert_eq!((kicks, waits), (GC_FRAMES as u32, 0));
    /* GC_FRAMES streams in flight, so the next submit reuses the first
     * one's buffers and has to wait for its fence */
    with_platform(|p| p.fence = 0);
    rig.submit(&d);
    // SAFETY: as above.
    unsafe { gc_counters(&mut kicks, &mut waits) };
    assert_eq!((kicks, waits), (GC_FRAMES as u32 + 1, 1));
}

/// A swap waits for the frame before the one it submits, never for that
/// one: the CPU records frame N + 1 while the GPU draws N. It used to wait
/// for the kick just submitted, which made every swap a glFinish (gc_slint
/// on the Verdin, 2026-10-01: 68 ms a frame, 48 of them in that wait).
#[test]
fn a_swap_waits_for_the_previous_frame_not_the_one_it_submits() {
    let rig = Rig::new();
    let d = rig.draw(3);
    let (mut kicks, mut waits) = (0u32, 0u32);

    /* nothing completes from here on: every wait would be counted */
    rig.submit(&d);
    with_platform(|p| p.fence = 0);
    assert_eq!(gc_finish_previous(), 0);
    // SAFETY: both are writable.
    unsafe { gc_counters(&mut kicks, &mut waits) };
    assert_eq!((kicks, waits), (1, 0), "the first swap has no frame before it to wait for");

    rig.submit(&d);
    with_platform(|p| p.fence = 0);
    assert_eq!(gc_finish_previous(), 0);
    // SAFETY: as above.
    unsafe { gc_counters(&mut kicks, &mut waits) };
    assert_eq!((kicks, waits), (2, 1), "the second swap waits for the first frame's kick");

    /* a swap with nothing new drawn waits for the frame before, once */
    assert_eq!(gc_finish_previous(), 0);
    // SAFETY: as above.
    unsafe { gc_counters(&mut kicks, &mut waits) };
    assert_eq!((kicks, waits), (2, 2));
}

#[test]
fn the_heaps_are_the_ones_the_gl_core_allocates_in() {
    assert_eq!(gc_heap_base(GLES_HEAP_DATA), GC_HEAP_DATA);
    assert_eq!(gc_heap_base(GLES_HEAP_SHADER), GC_HEAP_SHADER);
    assert_eq!(gc_heap_base(GLES_HEAP_PROGRAM), GC_HEAP_SHADER);
}

#[test]
fn init_that_cannot_allocate_fails_instead_of_recording_into_nothing() {
    let gl = TestGl::new(4, 4);
    *PLATFORM.lock().unwrap_or_else(|e| e.into_inner()) = Some(Platform::default());
    // SAFETY: tests are serialised by the TestGl guard.
    let prev = unsafe {
        let prev = crate::osal::g_osal;
        crate::osal::osal_set_ops(&RIG_OSAL);
        prev
    };
    gc_shutdown();
    with_platform(|p| p.fail_alloc_in = 2);
    assert_eq!(gc_init(), -1);
    let d = GlesDraw::ZERO;
    // SAFETY: `d` is live.
    assert_eq!(unsafe { gc_draw(&d) }, -1, "a draw into a backend that did not start");
    gc_shutdown();
    // SAFETY: serialised; the previous table is a static.
    unsafe { crate::osal::osal_set_ops(prev) };
    *PLATFORM.lock().unwrap_or_else(|e| e.into_inner()) = None;
    drop(gl);
}

/// The vertex/instance id injection is programmed on every draw: on for a
/// shader that takes an id, and explicitly OFF for one that does not. It used
/// to be written only when on, so a draw after one that used gl_VertexID --
/// in the same stream or, the GPU keeping its state, in the next client --
/// had ids written into a register of its own.
#[test]
fn the_id_injection_is_written_on_every_draw_and_off_when_unused() {
    let mut rig = Rig::new();
    rig.vs.viv.vs_id_in_reg = 3;
    let with_id = rig.submit(&rig.draw(3));
    assert_ne!(with_id.reg(GC_FE_HALTI5_ID_CONFIG) & GC_ID_CONFIG_VERTEX_ID_ENABLE, 0, "on for the id shader");

    rig.vs.viv.vs_id_in_reg = SHC_VIV_NO_REG;
    let without = rig.submit(&rig.draw(3));
    assert_eq!(without.writes(GC_FE_HALTI5_ID_CONFIG), (Some(0), 1), "written, and off, for a shader with no id");
}

/// A mat4 attribute arrives from the GL core as four elements, one per
/// column, into registers vi_reg .. vi_reg + 3 (gl/draw.rs
/// `vertex_inputs`), while the shader's io rows list it once. Every element
/// goes to the register it names -- not, as the rule for elements past the
/// io rows had it, to spare temporaries above the shader's -- and each is a
/// well-formed four-component, 16-byte element. The temporaries programmed
/// cover the highest register an element writes, even one the shader's own
/// count leaves out.
#[test]
fn a_matrix_attributes_columns_are_elements_into_their_own_registers() {
    let mut rig = Rig::new();
    rig.vs.viv.num_in = 2;
    rig.vs.viv.in_[1] = ShcVivIo { reg: 1, slot: 1, components: 4, interp: 0, name: [0; 32] };
    rig.vs.temps = 4; /* the compiler's count stops at t3; column 3 is t4 */
    let mut d = rig.draw(3);
    d.num_attribs = 5;
    for k in 0..5usize {
        d.attribs[k] = d.attribs[0];
        d.attribs[k].vi_reg = k as u8;
    }
    let s = rig.submit(&d);

    let map = s.reg(vs_halti5_input(0)).to_le_bytes();
    assert_eq!(map, [0, 1, 2, 3], "elements 0..3 into registers 0..3");
    assert_eq!(s.reg(vs_halti5_input(1)) & 0xff, 4, "element 4 into register 4");
    assert_eq!(s.reg(GC_VS_INPUT_COUNT) & 0xf, 5, "five inputs");
    assert_eq!(s.reg(GC_VS_TEMP_REGISTER_CONTROL), 5, "t0..t4, although the shader said four");
    for k in 0..5u32 {
        assert_eq!(s.reg(nfe_generic_attrib_config0(k)) & (0x3 << 12), attrib_num(4), "element {k}: four components");
        assert_eq!(s.reg(nfe_generic_attrib_config1(k)) & 0xff, 16, "element {k}: sixteen bytes");
    }
}

/// The flip study's arm (i), off by default: a top-down target drawn
/// bottom-up. The colour surface starts at its last row with a negative
/// stride, and the viewport and scissor are GL's, unmirrored -- the state an
/// FBO gets. With the switch off, the same draw is the mirrored viewport it
/// always was (the test above), so nothing changes unless a test asks.
#[cfg(feature = "test-knobs")]
#[test]
fn the_flip_study_draws_a_top_down_target_bottom_up_only_when_asked() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.viewport = [0, 0, 64, 32];
    d.scissor = true;
    d.scissor_rect = [8, 4, 16, 8];
    d.flip_y = true;
    let pitch = 64 * 4;

    let off = rig.submit(&d);
    assert_eq!(off.reg(GC_PE_COLOR_STRIDE), pitch, "off: the ordinary stride");
    assert_eq!(off.reg(pe_pipe_color_addr(0)), TARGET_GPU as u32, "off: from row 0");
    assert_eq!(off.reg(GC_PA_VIEWPORT_SCALE_Y), f2fixp(-16.0), "off: the mirrored viewport");

    gc_debug_set_flip_experiment(1);
    let on = rig.submit(&d);
    gc_debug_set_flip_experiment(0);
    assert_eq!(on.reg(GC_PE_COLOR_STRIDE), (pitch as i32).wrapping_neg() as u32, "a negative stride");
    assert_eq!(on.reg(pe_pipe_color_addr(0)), TARGET_GPU as u32 + 63 * pitch, "from the last row");
    assert_eq!(on.reg(pe_pipe_color_addr(1)), TARGET_GPU as u32 + 63 * pitch);
    assert_eq!(on.reg(GC_PA_VIEWPORT_SCALE_Y), f2fixp(16.0), "GL's viewport, not mirrored");
    assert_eq!(on.reg(GC_PA_VIEWPORT_OFFSET_Y), f2fixp(16.0));
    assert_eq!(on.reg(GC_SE_SCISSOR_TOP) >> 16, 4, "GL's scissor, not mirrored");

    /* an FBO (no flip_y) is untouched by the switch */
    d.flip_y = false;
    gc_debug_set_flip_experiment(1);
    let fbo = rig.submit(&d);
    gc_debug_set_flip_experiment(0);
    assert_eq!(fbo.reg(GC_PE_COLOR_STRIDE), pitch);
    assert_eq!(fbo.reg(pe_pipe_color_addr(0)), TARGET_GPU as u32);
}

/// The flip study's arm (ii), off by default: a frame ends with 100
/// resolve-engine copies of its target into the scratch surface with
/// RS_CONFIG_FLIP, then one plain copy back.
#[cfg(feature = "test-knobs")]
#[test]
fn the_flip_study_ends_a_frame_with_flipped_resolve_copies_only_when_asked() {
    let rig = Rig::new();
    let d = rig.draw(3);
    let kicks = |s: &Stream| s.writes(GC_RS_KICKER).1;
    let flipped = |s: &Stream| {
        let mut n = 0;
        walk(&s.0, |r, v, _| n += (r == GC_RS_CONFIG && v & GC_RS_CONFIG_FLIP != 0) as usize);
        n
    };

    let off = rig.submit(&d);
    assert_eq!((kicks(&off), flipped(&off)), (0, 0), "off: no copies");

    gc_debug_set_flip_experiment(2);
    let on = rig.submit(&d);
    gc_debug_set_flip_experiment(0);
    assert_eq!(kicks(&on), 101, "100 flipped copies and one back");
    assert_eq!(flipped(&on), 100);
    assert_eq!(on.reg(GC_RS_WINDOW_SIZE), rs_window_width(64) | rs_window_height(64));
    assert_eq!(on.reg(rs_pipe_dest_addr(0)), TARGET_GPU as u32, "the last copy lands back in the target");
}

/// 4x multisampling, on only while gc_debug_msaa asks: the scratch surface
/// supertiled at twice the target's size (padded to 64), the PE told so and
/// never told OVERWRITE, and the frame's resolve reading a TILED source with
/// the stride times four and the supertile bits, over a window of SOURCE
/// samples (etnaviv etnaviv_resource.c / etnaviv_state.c / etnaviv_rs.c on a
/// CACHE128B256BPERLINE core). Off, a 4x draw is the single-sampled one it
/// always was, with no resolve.
/// gc_debug_msaa = 3 adds etnaviv's three 4x settings this backend did not
/// make -- PE_LOGIC_OP UNK24 = 5, one more fragment input and temporary --
/// and 1 does not, so the board can run the two as arms of one session. The
/// scratch probe answers only once a scratch surface exists.
#[cfg(feature = "test-knobs")]
#[test]
fn msaa_bit_one_adds_etnaviv_s_small_msaa_settings_and_the_probe_reads_the_scratch() {
    let rig = Rig::new();
    let mut probe = [0u32; 4];
    let mut d = rig.draw(3);
    d.samples = 4;
    let unk24 = |r: &Stream| (r.reg(GC_PE_LOGIC_OP) >> 24) & 0x7;
    let inputs = |r: &Stream| r.reg(GC_PS_INPUT_COUNT) & 0x1f;

    gc_debug_set_msaa(1);
    let plain = rig.submit(&d);
    gc_debug_set_msaa(3);
    let extras = rig.submit(&d);
    let single = rig.submit(&rig.draw(3));
    // SAFETY: four writable u32s.
    let probed = unsafe { gc_debug_msaa_scratch(probe.as_mut_ptr()) };
    gc_debug_set_msaa(0);

    assert_eq!(unk24(&plain), 0, "bit 0 alone: as before");
    assert_eq!(unk24(&extras), 5, "bit 1: PE_LOGIC_OP UNK24 = 5");
    assert_eq!(inputs(&extras), inputs(&plain) + 1, "bit 1: one more fragment input");
    assert_eq!(unk24(&single), 0, "a single-sampled draw never takes them");
    assert_eq!(inputs(&single), inputs(&plain), "nor the extra input");
    assert_eq!(probed, 0, "a 4x draw made the scratch surface, so the probe answers");
    let mut map = [7u8; 1024];
    // SAFETY: 1024 writable bytes.
    let chunks = unsafe { gc_debug_msaa_scratch_map(map.as_mut_ptr(), map.len() as u32) };
    assert_eq!(chunks as u32, probe[0] / 256, "one flag per 256 bytes of the scratch");
    assert!(map[..chunks as usize].iter().all(|f| *f <= 1), "each flag is 0 or 1");
    assert!(probe[0] >= 2 * 64 * 2 * 64 * 4, "the scratch is at least 128 x 128 samples: {} bytes", probe[0]);
}

/// gc_debug_msaa bit 2 (7) writes etnaviv's per-draw PS_MSAA_CONFIG --
/// coverage 16/16, the sample mask enabled with all four samples, their
/// write masks cleared -- and 3 leaves the context word's, which writes none
/// of those fields.
#[cfg(feature = "test-knobs")]
#[test]
fn msaa_bit_two_writes_the_sample_mask_and_coverage_as_etnaviv_does() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.samples = 4;
    gc_debug_set_msaa(3);
    let three = rig.submit(&d).reg(GC_PS_MSAA_CONFIG);
    gc_debug_set_msaa(7);
    let seven = rig.submit(&d).reg(GC_PS_MSAA_CONFIG);
    gc_debug_set_msaa(0);
    assert_eq!(three, !(GC_PS_MSAA_ALPHA_TO_COVERAGE | GC_PS_MSAA_ALPHA_TO_COVERAGE_MASK), "3: the context word");
    assert_eq!(seven & GC_PS_MSAA_SAMPLE_MASK_FIELD, 0xf, "all four samples");
    assert_ne!(seven & GC_PS_MSAA_SAMPLE_MASK_ENABLE, 0);
    assert_eq!((seven & GC_PS_MSAA_SAMPLE_COVERAGE_VALUE_FIELD) >> 20, 16, "coverage 16/16");
    assert_ne!(seven & GC_PS_MSAA_SAMPLE_COVERAGE, 0);
    for m in [
        GC_PS_MSAA_SAMPLE_MASK_MASK,
        GC_PS_MSAA_SAMPLE_MASK_ENABLE_MASK,
        GC_PS_MSAA_SAMPLE_COVERAGE_VALUE_MASK,
        GC_PS_MSAA_SAMPLE_COVERAGE_MASK,
        GC_PS_MSAA_SAMPLE_COVERAGE_INVERT_MASK,
    ] {
        assert_eq!(seven & m, 0, "write mask {m:#x} cleared, so the field is written");
    }
    assert_eq!(seven & GC_PS_MSAA_SAMPLE_COVERAGE_INVERT, 0, "not inverted");
}

#[cfg(feature = "test-knobs")]
#[test]
fn a_multisampled_draw_is_supertiled_and_resolved_like_etnaviv_only_when_on() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.samples = 4;
    let (w, bpp) = (64u32, 4u32);
    let ms_pitch = (2 * w).div_ceil(64) * 64 * bpp; /* 128 samples -> 512 bytes */

    let off = rig.submit(&d);
    assert_eq!(off.writes(GC_RS_KICKER).1, 0, "off: no resolve");
    assert_eq!(off.reg(GC_PE_COLOR_STRIDE), w * bpp);
    assert_ne!(off.reg(GC_PE_COLOR_FORMAT) & GC_PE_COLOR_FORMAT_OVERWRITE, 0, "off: the plain target");

    gc_debug_set_msaa(1);
    let on = rig.submit(&d);
    gc_debug_set_msaa(0);
    let fmt = on.reg(GC_PE_COLOR_FORMAT);
    assert_ne!(fmt & GC_PE_COLOR_FORMAT_SUPER_TILED, 0);
    assert_ne!(fmt & GC_PE_COLOR_FORMAT_SUPER_TILED_NEW, 0);
    assert_eq!(fmt & GC_PE_COLOR_FORMAT_OVERWRITE, 0, "never OVERWRITE on a multisampled target");
    assert_eq!(on.reg(GC_PE_COLOR_STRIDE), ms_pitch);
    assert_ne!(on.reg(pe_pipe_color_addr(0)), TARGET_GPU as u32, "drawn into the scratch surface");

    let cfg = on.reg(GC_RS_CONFIG);
    assert_eq!(
        cfg & (GC_RS_SOURCE_TILED | GC_RS_DOWNSAMPLE_X | GC_RS_DOWNSAMPLE_Y),
        GC_RS_SOURCE_TILED | GC_RS_DOWNSAMPLE_X | GC_RS_DOWNSAMPLE_Y
    );
    assert_eq!(
        on.reg(GC_RS_SOURCE_STRIDE),
        (ms_pitch << 2) | GC_RS_SOURCE_STRIDE_TILING | GC_RS_SOURCE_STRIDE_SUPER_TILED_NEW
    );
    assert_eq!(on.reg(GC_RS_DEST_STRIDE), w * bpp, "into the linear target");
    assert_eq!(on.reg(GC_RS_WINDOW_SIZE), rs_window_width(2 * w) | rs_window_height(2 * 64), "in source samples");
    assert_eq!(on.reg(rs_pipe_dest_addr(0)), TARGET_GPU as u32);
    assert_eq!(on.reg(rs_pipe_source_addr(0)), on.reg(pe_pipe_color_addr(0)), "the resolve reads what the PE wrote");

    /* a height the resolve cannot take stays single-sampled, never resolved past the target */
    let mut odd = rig.draw(3);
    odd.samples = 4;
    odd.target_h = 62;
    gc_debug_set_msaa(1);
    let fallback = rig.submit(&odd);
    gc_debug_set_msaa(0);
    assert_eq!(fallback.writes(GC_RS_KICKER).1, 0);
    assert_eq!(fallback.reg(GC_PE_COLOR_STRIDE), w * bpp);
}

/// A draw's descriptor writes are Mesa's order (the i.MX8MP stale vertex
/// descriptor fix): every descriptor address of the draw, then every
/// invalidate in slot order, then emit_draw's RA-PE stall before the draw --
/// no flush or stall between them. A draw with fragment units 0 and 1 and
/// vertex unit 0 (slot 2) is recorded from its first NTE_DESCRIPTOR_ADDR
/// write to the draw command, keeping only the addresses, the invalidates,
/// the descriptor flush, GL_FLUSH_CACHE and the stall tokens. A draw with no
/// texture writes no invalidate.
#[test]
fn descriptor_addresses_come_first_then_every_invalidate() {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Ev {
        Addr(u32),
        Inval(u32),
        DescFlush,
        Flush(u32),
        Stall(u32, u32),
    }
    use Ev::*;
    let events = |s: &Stream| -> Vec<Ev> {
        let slot_of = |f: &dyn Fn(u32) -> bool| (0..32u32).find(|&k| f(k));
        let mut v = Vec::new();
        let mut i = 0usize;
        while i < s.0.len() {
            let w = s.0[i];
            match w & 0xf800_0000 {
                GC_OP_LOAD_STATE => {
                    let count = ((w >> 16) & 0x3ff) as usize;
                    let base = (w & 0xffff) * 4;
                    for k in 0..count {
                        let (r, val) = (base + 4 * k as u32, s.0[i + 1 + k]);
                        let ev = if let Some(slot) = slot_of(&|k| r == nte_descriptor_addr(k)) {
                            Some(Addr(slot))
                        } else if r == GC_NTE_DESCRIPTOR_INVALIDATE {
                            let slot = slot_of(&|k| val == GC_NTE_INVALIDATE_UNK29 | nte_invalidate_idx(k));
                            Some(Inval(slot.expect("an invalidate names a slot")))
                        } else if r == GC_NTE_DESCRIPTOR_FLUSH {
                            Some(DescFlush)
                        } else if r == GC_GL_FLUSH_CACHE {
                            Some(Flush(val))
                        } else if r == GC_GL_SEMAPHORE_TOKEN {
                            let pair = [GC_SYNC_FE, GC_SYNC_RA]
                                .iter()
                                .flat_map(|&a| [GC_SYNC_PE].map(move |b| (a, b)))
                                .find(|&(a, b)| val == stall_token(a, b));
                            let (a, b) = pair.expect("a stall this backend emits");
                            Some(Stall(a, b))
                        } else {
                            None
                        };
                        if let Some(ev) = ev {
                            if matches!(ev, Addr(_)) || !v.is_empty() {
                                v.push(ev);
                            }
                        }
                    }
                    i += 1 + count;
                }
                GC_OP_DRAW_INSTANCED => return v,
                GC_OP_STALL => i += 2,
                _ => i += 1,
            }
        }
        panic!("no draw in the stream")
    };
    let rig = Rig::new();
    let tex = |unit: u8| GlesTextureBind {
        unit,
        format: crate::backend::GLES_TEXFMT_RGBA8,
        base_gpu: 0x4000_0000 + 0x1_0000 * unit as u64,
        width: 1,
        height: 1,
        stride_texels: 1,
        bytes: 4,
        mip_levels: 1,
        ..GlesTextureBind::ZERO
    };
    let mut d = rig.draw(3);
    d.num_textures = 2;
    d.textures[0] = tex(0);
    d.textures[1] = tex(1);
    d.num_vs_textures = 1;
    d.vs_textures[0] = tex(0);
    let got = events(&rig.submit(&d));
    assert_eq!(
        got,
        [Addr(0), Addr(1), Addr(2), Inval(0), Inval(1), Inval(2), Stall(GC_SYNC_RA, GC_SYNC_PE)],
        "all addresses, then all invalidates, then emit_draw's stall"
    );

    /* a draw that writes no descriptor writes no invalidate either */
    let mut invals = 0;
    walk(&rig.submit(&rig.draw(3)).0, |r, _, _| invals += usize::from(r == GC_NTE_DESCRIPTOR_INVALIDATE));
    assert_eq!(invals, 0);
}

/// The synchronisation skeleton of a frame, step by step against etnaviv
/// (issues #9 and #25, the GC7000's nondeterministic missing colour writes;
/// the comparison is in VIVANTE_NOTES.md, "Draw, resolve, read back: our sync
/// sequence against etnaviv's"). gl_test case 10's failing frame is a clear
/// and then a draw in one stream, read back after glFinish; this records the
/// same shape and keeps only the cache flushes, the semaphore/stall pairs and
/// the draws:
///
/// - the stream opens with the descriptor-cache flush of the GPU state reset;
/// - every draw, the clear quad included, flushes colour, depth, texture and
///   shader L1, then the vertex texture cache on its own, then waits RA for
///   PE, then flushes the tile-status cache -- so a draw never starts while
///   the previous draw's colour is in the PE cache unflushed;
/// - the shader and sampler state waits FE for PE, and the draw command
///   waits RA for PE;
/// - every SEMAPHORE_TOKEN is followed at once by its STALL for the same
///   pair (FE: the STALL command; RA: the STALL_TOKEN state). A token with no
///   stall would leave a semaphore pending that a later stall consumes early;
/// - the stream ends with the last draw: the final colour flush, the FE/PE
///   pair and the EVENT are the OSAL's epilogue (kick.rs), as in etnaviv,
///   where the kernel appends them (etnaviv_buffer_queue).
#[test]
fn a_frame_flushes_and_stalls_as_etnaviv_does() {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Ev {
        Flush(u32),
        TsFlush,
        Stall(u32, u32),
        Draw,
    }
    use Ev::*;
    let pair_of = |token: u32| {
        [GC_SYNC_FE, GC_SYNC_RA]
            .iter()
            .flat_map(|&a| [GC_SYNC_PE].map(move |b| (a, b)))
            .find(|&(a, b)| token == stall_token(a, b))
            .expect("a semaphore pair this backend emits")
    };
    let events = |s: &Stream| -> Vec<Ev> {
        let mut v = Vec::new();
        let mut i = 0usize;
        while i < s.0.len() {
            let w = s.0[i];
            match w & 0xf800_0000 {
                GC_OP_LOAD_STATE if w == load_state(GC_GL_SEMAPHORE_TOKEN, 1, false) => {
                    /* the token, and the stall for the same pair as the very
                     * next command: one value, so no pad word between them */
                    let token = s.0[i + 1];
                    let (a, b) = pair_of(token);
                    let stall = if a == GC_SYNC_FE { GC_OP_STALL } else { load_state(GC_GL_STALL_TOKEN, 1, false) };
                    assert_eq!(
                        s.0.get(i + 2..i + 4),
                        Some(&[stall, token][..]),
                        "the semaphore at word {i} has its stall"
                    );
                    v.push(Stall(a, b));
                    i += 4;
                }
                GC_OP_LOAD_STATE => {
                    let count = ((w >> 16) & 0x3ff) as usize;
                    let base = (w & 0xffff) * 4;
                    for k in 0..count {
                        let (r, val) = (base + 4 * k as u32, s.0[i + 1 + k]);
                        if r == GC_GL_FLUSH_CACHE {
                            v.push(Flush(val));
                        } else if r == GC_TS_FLUSH_CACHE {
                            assert_eq!(val, GC_TS_FLUSH);
                            v.push(TsFlush);
                        } else if r == GC_GL_STALL_TOKEN || r == GC_GL_SEMAPHORE_TOKEN {
                            panic!("a stall token at word {i} outside a semaphore/stall pair");
                        }
                    }
                    i += 1 + count;
                }
                GC_OP_DRAW_INSTANCED => {
                    v.push(Draw);
                    i += 4;
                }
                GC_OP_STALL => panic!("a STALL command at word {i} with no semaphore before it"),
                _ => i += 1, /* NOP and the pad words */
            }
        }
        v
    };
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.do_clear = true;
    let got = events(&rig.submit(&d));

    let per_draw = [
        Flush(GC_FLUSH_COLOR | GC_FLUSH_DEPTH | GC_FLUSH_TEXTURE | GC_FLUSH_SHADER_L1),
        Flush(GC_FLUSH_TEXTUREVS),
        Stall(GC_SYNC_RA, GC_SYNC_PE),
        TsFlush,
        Stall(GC_SYNC_FE, GC_SYNC_PE),
        Stall(GC_SYNC_RA, GC_SYNC_PE),
        Draw,
    ];
    let mut want = vec![Flush(GC_FLUSH_DESCRIPTOR)];
    want.extend_from_slice(&per_draw); /* the clear quad */
    want.extend_from_slice(&per_draw); /* the draw */
    assert_eq!(got, want, "the clear, then the draw, each behind its flushes; nothing after the last draw");
}

/// gc_debug_msaa bit 3 (9): the 4x draw's viewport and clip are in the
/// scratch surface's samples, twice the target on each axis. Without it they
/// stay in target pixels (the board has not said which this core wants, issue
/// #8); a single-sampled draw is never scaled.
#[cfg(feature = "test-knobs")]
#[test]
fn msaa_bit_three_scales_the_viewport_and_clip_to_the_scratch_samples() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.samples = 4;
    let single = rig.submit(&rig.draw(3));

    gc_debug_set_msaa(1);
    let plain = rig.submit(&d);
    gc_debug_set_msaa(9);
    let scaled = rig.submit(&d);
    let scaled_single = rig.submit(&rig.draw(3));
    gc_debug_set_msaa(0);

    let scaled_regs = [
        GC_PA_VIEWPORT_SCALE_X,
        GC_PA_VIEWPORT_SCALE_Y,
        GC_PA_VIEWPORT_OFFSET_X,
        GC_PA_VIEWPORT_OFFSET_Y,
        GC_SE_SCISSOR_RIGHT,
        GC_SE_SCISSOR_BOTTOM,
        GC_SE_CLIP_RIGHT,
        GC_SE_CLIP_BOTTOM,
    ];
    for r in scaled_regs {
        assert_eq!(plain.reg(r), single.reg(r), "bit 0 alone leaves register {r:#x} in target pixels");
        assert_eq!(scaled_single.reg(r), single.reg(r), "a single-sampled draw is never scaled ({r:#x})");
    }
    for r in [GC_PA_VIEWPORT_SCALE_X, GC_PA_VIEWPORT_SCALE_Y, GC_PA_VIEWPORT_OFFSET_X, GC_PA_VIEWPORT_OFFSET_Y] {
        assert_eq!(scaled.reg(r) as i32, 2 * plain.reg(r) as i32, "viewport register {r:#x} is twice as large");
    }
    for r in [GC_SE_CLIP_RIGHT, GC_SE_CLIP_BOTTOM] {
        let (p, s) = (plain.reg(r) - GC_SE_CLIP_MARGIN, scaled.reg(r) - GC_SE_CLIP_MARGIN);
        assert_eq!(s, 2 * p, "clip register {r:#x} is twice as far from the origin");
    }
    assert_eq!(scaled.reg(GC_SE_SCISSOR_LEFT), plain.reg(GC_SE_SCISSOR_LEFT), "the origin stays at 0");
}

/* ---- occlusion queries: GL_OCCLUSION_QUERY_ADDR / _CONTROL ---- */

/// The register writes of a stream, in order, of the two query registers:
/// (register, value).
fn oq_writes(s: &Stream) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    walk(&s.0, |r, v, _| {
        if r == GC_GL_OCCLUSION_QUERY_ADDR || r == GC_GL_OCCLUSION_QUERY_CONTROL {
            out.push((r, v));
        }
    });
    out
}

/// The address the stream arms for `handle`'s slot `slot`.
fn oq_slot(handle: u32, slot: u32) -> u32 {
    (oq_mem().gpu + u64::from(handle * 64 + slot) * 8) as u32
}

/// etnaviv's resume and suspend around counted draws: the first counted
/// draw arms the handle's slot 0, a second one of the same handle writes
/// nothing, an uncounted draw suspends (CONTROL) and arms nothing, the next
/// counted draw arms slot 1, and the stream ends with the suspend.
///
/// Deliberate breaks: drop the `oq_arm` call in `emit_draw` and no ADDR is
/// written; drop the `oq_suspend` in `frame_submit` and the stream ends
/// armed (the last count would never be stored).
#[test]
fn counted_draws_arm_a_slot_and_every_change_and_the_stream_end_suspend() {
    let rig = Rig::new();
    let plain = rig.draw(3);
    let s0 = rig.submit(&plain);
    assert!(oq_writes(&s0).is_empty(), "no query: no query register (the ES 2.0 stream is unchanged)");
    assert!(oq_mem().cpu.is_null(), "no slots until a query starts");

    assert_eq!(gc_query_begin(9), 0);
    let mut counted = rig.draw(3);
    counted.occlusion_query = 9;
    // SAFETY: live draws for the calls.
    unsafe {
        for d in [&counted, &counted, &plain, &counted] {
            assert_eq!(gc_draw(d), 0);
        }
    }
    assert_eq!(gc_flush(), 0);
    let s = rig.last_stream();
    let c = (GC_GL_OCCLUSION_QUERY_CONTROL, GC_OQ_CONTROL_VALUE);
    assert_eq!(
        oq_writes(&s),
        [(GC_GL_OCCLUSION_QUERY_ADDR, oq_slot(9, 0)), c, (GC_GL_OCCLUSION_QUERY_ADDR, oq_slot(9, 1)), c],
        "arm slot 0, suspend before the plain draw, arm slot 1, suspend at the end"
    );
    let draws_at: Vec<usize> = {
        let mut v = Vec::new();
        let mut i = 0;
        while i < s.0.len() {
            let w = s.0[i];
            if w & 0xf800_0000 == GC_OP_DRAW_INSTANCED {
                v.push(i);
            }
            i += match w & 0xf800_0000 {
                GC_OP_LOAD_STATE => 1 + ((w >> 16) & 0x3ff) as usize,
                GC_OP_DRAW_INSTANCED => 4,
                GC_OP_STALL => 2,
                _ => 1,
            };
        }
        v
    };
    assert!(s.at(GC_GL_OCCLUSION_QUERY_CONTROL) > *draws_at.last().unwrap(), "the last suspend follows the last draw");
    assert!(whole_commands(&s.0));
}

/// A clear inside the query is not counted: the clear quad suspends the
/// counter, and the counted draw after it arms the next slot. The result is
/// the sum of the slots the query filled.
///
/// Deliberate break: arm `d.gl.occlusion_query` for the clear quad too and
/// the clear arms slot 0 instead of suspending.
#[test]
fn a_clear_suspends_and_the_result_sums_the_slots() {
    let rig = Rig::new();
    assert_eq!(gc_query_begin(3), 0);
    let mut counted = rig.draw(3);
    counted.occlusion_query = 3;
    let mut cleared = counted;
    (cleared.do_clear, cleared.clear_color_mask) = (true, 0xf);
    // SAFETY: live draws for the calls.
    unsafe {
        assert_eq!(gc_draw(&counted), 0);
        assert_eq!(gc_draw(&cleared), 0); /* the clear quad, then the draw */
    }
    assert_eq!(gc_flush(), 0);
    let c = (GC_GL_OCCLUSION_QUERY_CONTROL, GC_OQ_CONTROL_VALUE);
    assert_eq!(
        oq_writes(&rig.last_stream()),
        [(GC_GL_OCCLUSION_QUERY_ADDR, oq_slot(3, 0)), c, (GC_GL_OCCLUSION_QUERY_ADDR, oq_slot(3, 1)), c]
    );
    /* the pixel engine stored 0 and 5 */
    let at = (oq_mem().cpu as *mut u64).wrapping_add(3 * 64);
    // SAFETY: inside the slots.
    unsafe {
        at.write(0);
        at.add(1).write(5);
        at.add(2).write(77); /* past the slots the query filled: not read */
    }
    let mut n = 0u64;
    // SAFETY: `n` is a live u64.
    assert_eq!(unsafe { gc_query_result(3, &mut n) }, 0);
    assert_eq!(n, 5);
    assert_eq!(gc_query_begin(3), 0);
    // SAFETY: as above.
    assert_eq!(unsafe { gc_query_result(3, &mut n) }, 0);
    assert_eq!(n, 0, "begin zeroes the slots and the count");
    assert!(rig.faults().is_empty());
}

/// A counted draw that does not fit the frame is taken back and recorded in
/// the next, with the counter as it was before it: the frame it was taken
/// out of holds plain draws only and ends with no query register written;
/// the next frame arms the handle once and suspends it once.
///
/// Deliberate break: leave `oq` out of `Mark::restore` and the first frame
/// ends with a CONTROL write (the handle still marked armed, its ADDR rolled
/// back with the draw).
#[test]
fn a_counted_draw_taken_back_leaves_the_counter_as_it_was() {
    let rig = Rig::new();
    assert_eq!(gc_query_begin(4), 0);
    let plain = rig.draw(3);
    let mut counted = plain;
    counted.occlusion_query = 4;
    let room = || {
        let g = g();
        let w = &g.frames[g.cur].w;
        w.cap - w.used
    };
    let draw = |d: &GlesDraw| {
        // SAFETY: `d` is live.
        assert_eq!(unsafe { gc_draw(d) }, 0);
    };
    /* what one draw of each kind takes in the middle of a frame */
    draw(&plain);
    let r0 = room();
    draw(&plain);
    let dp = r0 - room();
    let r1 = room();
    draw(&counted);
    let dc = r1 - room() + OQ_SUSPEND_WORDS; /* and the room it keeps */
    assert_eq!(gc_flush(), 0);
    let kicks0 = rig.kicks();
    /* fill a frame with plain draws until a counted draw no longer fits */
    draw(&plain);
    while room() >= dp + dc {
        draw(&plain);
    }
    if room() >= dc {
        draw(&plain);
    }
    assert!(room() < dc && rig.kicks() == kicks0, "the frame is still open, too full for a counted draw");
    draw(&counted);
    assert_eq!(rig.kicks(), kicks0 + 1, "the counted draw ended the frame");
    assert_eq!(oq_writes(&rig.last_stream()), [], "the frame it was taken out of writes no query register");
    assert_eq!(gc_flush(), 0);
    let s = rig.last_stream();
    assert_eq!(s.draws().len(), 1);
    assert_eq!(
        oq_writes(&s),
        [(GC_GL_OCCLUSION_QUERY_ADDR, oq_slot(4, 1)), (GC_GL_OCCLUSION_QUERY_CONTROL, GC_OQ_CONTROL_VALUE)],
        "the next frame arms slot 1 (slot 0 was the first frame's) and suspends it"
    );
    assert!(whole_commands(&s.0));
    assert!(rig.faults().is_empty());
}

/// More suspends than a handle has slots: the query stops arming and its
/// result is "some passed" (the conservative answer), logged once.
#[test]
fn a_query_out_of_slots_answers_some_passed() {
    let rig = Rig::new();
    assert_eq!(gc_query_begin(2), 0);
    let mut d = rig.draw(3);
    d.occlusion_query = 2;
    for _ in 0..65 {
        // SAFETY: `d` is live.
        assert_eq!(unsafe { gc_draw(&d) }, 0);
        assert_eq!(gc_flush(), 0);
    }
    assert!(oq_writes(&rig.last_stream()).is_empty(), "the 65th stream arms nothing");
    let mut n = 0u64;
    // SAFETY: `n` is a live u64.
    assert_eq!(unsafe { gc_query_result(2, &mut n) }, 0);
    assert_eq!(n, 1, "all 64 slots read 0, and the overflow answers 1");
    assert!(rig.faults().is_empty());
}
