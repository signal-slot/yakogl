// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Case 246 `tsprobe` (docs/notes/gc-ts-probe.md, "Part (iii)"): the
//! `test-knobs` hook `gc_debug_ts_probe` that runs the probe's four RS-only
//! kicks on the board, and the unit tests of the probe's streams and
//! readback.
//!
//! The probe's streams, readback and classification are pure functions in
//! [`super::ts`], which stays self-contained (no `crate::` or `super::`
//! path, `tests/gc_ts_words.rs`); the hook allocates through the OSAL and
//! submits its own kicks, so it lives here.

#[allow(unused_imports)]
use super::ts::*;

#[cfg(feature = "test-knobs")]
pub use self::hook::gc_debug_ts_probe;

#[cfg(feature = "test-knobs")]
mod hook {
    use super::*;
    use crate::osal::{self, OsalGpuMem, OSAL_MEM_DATA};
    use crate::vivante::backend;
    use crate::vivante::kick::{GcKick, GC_KICK_MAGIC, GC_KICK_TAIL_BYTES};
    use crate::vivante::regs::load_state;
    use crate::vivante::state::cache_flush_raw;
    use core::ffi::c_void;

    /// The guard on each side of an object, and the objects' alignment.
    const GUARD: usize = 0x1000;
    /// `backend.rs`'s `GC_HEAP_COMMAND` (private there): the heap of every
    /// command stream the back end submits.
    const HEAP_COMMAND: u64 = 0x6000_0000;
    /// Words of the probe's command buffer: K2, the longest step, is 64.
    const CMD_WORDS: usize = 256;
    /// A fence that has not signalled after this long is a hang
    /// (gc-ts-probe.md: 2 s, as the image's `s_hang_case`).
    const WAIT_US: i64 = 2_000_000;
    const N: usize = PROBE_DWORDS;

    /// One guarded object: `size` bytes at a 0x1000-aligned GPU address
    /// below 4 GiB, with at least [`GUARD`] bytes of allocation either side.
    struct Obj {
        mem: OsalGpuMem,
        off: usize,
        size: usize,
    }

    impl Obj {
        fn new(heap: u64, size: usize) -> Result<Obj, i32> {
            let mut o = Obj { mem: OsalGpuMem::ZERO, off: 0, size };
            if osal::gpu_alloc(heap, size + 3 * GUARD, OSAL_MEM_DATA, &mut o.mem) != 0 {
                return Err(PROBE_ERR_ALLOC);
            }
            if o.mem.cpu.is_null() || !(o.mem.cpu as usize).is_multiple_of(4) {
                return Err(PROBE_ERR_CPU);
            }
            let at = (o.mem.gpu + GUARD as u64).next_multiple_of(GUARD as u64);
            o.off = (at - o.mem.gpu) as usize;
            if at + (size + GUARD) as u64 > u32::MAX as u64 {
                return Err(PROBE_ERR_ADDR);
            }
            Ok(o)
        }

        fn gpu(&self) -> u32 {
            (self.mem.gpu + self.off as u64) as u32
        }

        /// The whole allocation as dwords, as the CPU sees it now.
        fn all(&mut self) -> &mut [u32] {
            if let Some(f) = osal::ops().and_then(|o| o.cache_invd) {
                // SAFETY: the allocation's own mapping and size.
                unsafe { f(self.mem.cpu, self.mem.size) };
            }
            // SAFETY: the allocation is `mem.size` bytes long, mapped at
            // `mem.cpu` (4-aligned, checked in `new`), and no other view of
            // it is live: every caller drops the slice before the next kick.
            unsafe { core::slice::from_raw_parts_mut(self.mem.cpu as *mut u32, self.mem.size / 4) }
        }

        /// The object's dwords.
        fn words(&mut self) -> &mut [u32] {
            let (off, size) = (self.off / 4, self.size / 4);
            &mut self.all()[off..off + size]
        }

        /// The guards' dwords that are not [`PROBE_GUARD`].
        fn guard_changed(&mut self) -> u32 {
            let (off, size, g) = (self.off / 4, self.size / 4, GUARD / 4);
            let all = self.all();
            all[off - g..off].iter().chain(&all[off + size..off + size + g]).filter(|&&w| w != PROBE_GUARD).count()
                as u32
        }

        /// Hand the CPU's writes to the GPU.
        fn flush(&self) {
            // SAFETY: the allocation's own mapping and size.
            unsafe { cache_flush_raw(self.mem.cpu, self.mem.size) };
        }
    }

    impl Drop for Obj {
        fn drop(&mut self) {
            osal::gpu_free(&mut self.mem);
        }
    }

    /// Submit the writes `build` makes as one kick of `cmd` and wait for it:
    /// the OSAL submit and fence wait every back-end kick takes
    /// (`frame_submit`, `wait_fence`), with the wait bounded. `Ok(true)` when
    /// the fence signalled, `Ok(false)` on a timeout or a wait error.
    fn kick(cmd: &mut Obj, build: impl FnOnce(ProbeSink)) -> Result<bool, i32> {
        let (Some(submit), Some(wait)) = (osal::ops().and_then(|o| o.submit), osal::ops().and_then(|o| o.wait)) else {
            return Err(PROBE_ERR_OSAL);
        };
        let cap = CMD_WORDS - GC_KICK_TAIL_BYTES as usize / 4;
        let words = cmd.words();
        let mut n = 0usize;
        build(&mut |reg, value| {
            /* one register a LOAD_STATE: header and value, 64 bits, so the
             * stream is always an even number of words */
            assert!(n + 2 <= cap, "the probe's stream outgrew its buffer");
            words[n] = load_state(reg, 1, false);
            words[n + 1] = value;
            n += 2;
        });
        cmd.flush();
        let kick = GcKick { magic: GC_KICK_MAGIC, stream_bytes: (n * 4) as u32, stream_gpu: cmd.gpu() as u64 };
        // SAFETY: `kick` is a live GcKick of the size passed.
        let fence = unsafe { submit(&kick as *const GcKick as *const c_void, core::mem::size_of::<GcKick>()) };
        if fence < 0 {
            return Err(PROBE_ERR_SUBMIT);
        }
        // SAFETY: a plain wait on the fence the OSAL just returned.
        Ok(unsafe { wait(fence, WAIT_US) } == 0)
    }

    /// Case 246's arm `arm` (gc-ts-probe.md "Part (iii)"): K0 to K2, and K3
    /// when K2 read `YES`, each one kick of the probe's own, waited on with a
    /// 2 s bound; no kick after a timeout. `progress(step)` is called before
    /// each step (1 set-up done, 2 K0, 3 K1, 4 K2, 5 K3, 6 the readback), so
    /// that the case can write `gc_result_progress(0x200 | arm << 4 | step)`
    /// before the kick that may hang. Fills `*out` (`struct tsp_readback`,
    /// `out_bytes` its size) and returns the [`Outcome`] code, or a
    /// `PROBE_ERR_*` for a step that could not run (`*out` untouched). Waits
    /// for the back end's own kicks first and touches none of its state.
    ///
    /// # Safety
    /// `out` is null or points at `out_bytes` writable bytes; `progress` is
    /// null or a function safe to call with any `u32`.
    #[no_mangle]
    pub unsafe extern "C" fn gc_debug_ts_probe(
        arm: u32,
        out: *mut TspReadback,
        out_bytes: u32,
        progress: Option<unsafe extern "C" fn(step: u32)>,
    ) -> i32 {
        if out.is_null() || out_bytes as usize != core::mem::size_of::<TspReadback>() {
            return PROBE_ERR_ARGS;
        }
        let Some(a) = probe_arm(arm) else { return PROBE_ERR_ARM };
        let step = |s: u32| {
            if let Some(f) = progress {
                // SAFETY: the caller's contract.
                unsafe { f(s) }
            }
        };
        match run(arm, &a, &step) {
            Ok((outcome, rb)) => {
                // SAFETY: non-null and of the struct's size (checked above).
                unsafe { out.write_unaligned(rb) };
                outcome as i32
            }
            Err(e) => e,
        }
    }

    /// Lengths of the reads that happened (0: the step did not run).
    #[derive(Clone, Copy, Default)]
    struct Lens {
        k0: usize,
        k1: usize,
        k2: usize,
        k3: usize,
    }

    /// `scratch` holds `detile(B)`, K0's, K2's and K3's `D`, 4096 dwords each.
    fn readback(sc: &[u32], k1_t: &[u32], l: Lens, base: ArmReads, arm: &ProbeArm) -> (Outcome, TspReadback) {
        let r = ArmReads {
            expected_b: &sc[..N],
            k0_d: &sc[N..N + l.k0],
            k1_t: &k1_t[..l.k1],
            k2_d: &sc[2 * N..2 * N + l.k2],
            k3_d: &sc[3 * N..3 * N + l.k3],
            ..base
        };
        probe_readback(&r, arm)
    }

    fn run(n: u32, arm: &ProbeArm, step: &dyn Fn(u32)) -> Result<(Outcome, TspReadback), i32> {
        if osal::ops().and_then(|o| o.submit).is_none() || osal::ops().and_then(|o| o.wait).is_none() {
            return Err(PROBE_ERR_OSAL);
        }
        if backend::finish() != 0 {
            return Err(PROBE_ERR_FINISH);
        }
        let heap = backend::heap_data();
        let bytes = PROBE_SURFACE_BYTES as usize;
        let mut s = Obj::new(heap, bytes)?;
        let mut t = Obj::new(heap, PROBE_TS_BYTES as usize)?;
        let mut d = Obj::new(heap, bytes)?;
        /* CPU-only scratch: detile(B), K0's D, K2's D, K3's D */
        let mut scratch = Obj::new(heap, 4 * bytes)?;
        let mut cmd = Obj::new(HEAP_COMMAND, CMD_WORDS * 4)?;
        let a = ProbeAddrs { s: s.gpu(), t: t.gpu(), d: d.gpu() };

        for o in [&mut s, &mut t, &mut d] {
            o.all().fill(PROBE_GUARD);
        }
        for (i, w) in s.words().iter_mut().enumerate() {
            *w = probe_b(i as u32);
        }
        t.words().fill(0);
        d.words().fill(PROBE_DEST_INIT);
        for o in [&s, &t, &d] {
            o.flush();
        }
        let s_changed = |s: &mut Obj| s.words().iter().enumerate().any(|(i, &w)| w != probe_b(i as u32));
        let mut base = ArmReads {
            arm: n,
            steps_done: 0,
            fence_timeouts: 0,
            guard_changed: 0,
            surface_changed: false,
            expected_b: &[],
            k0_d: &[],
            k1_t: &[],
            k2_d: &[],
            k3_d: &[],
        };
        let mut l = Lens::default();
        let mut k1_t = [0u32; PROBE_TS_DWORDS];
        let mut interleaved = false;
        step(1);

        /* K0, the control resolve; K1, the TS fill; K2, the TS-sourced
         * resolve. A timeout ends the arm: no later kick runs. */
        let mut timed_out = false;
        step(2);
        if kick(&mut cmd, |w| probe_k0_into(w, arm, &a))? {
            base.steps_done += 1;
            l.k0 = N;
            scratch.words()[N..2 * N].copy_from_slice(d.words());
            /* arm 3: whichever super-tile order K0 matches is detile(B) */
            let k0 = &scratch.words()[N..2 * N];
            let fits = |il: bool| (0..N as u32).all(|i| k0[i as usize] == probe_expected_b(probe_layout(arm, il), i));
            interleaved = arm.super_tiled && !fits(false) && fits(true);
            step(3);
            if kick(&mut cmd, |w| probe_k1_into(w, &a))? {
                base.steps_done += 1;
                l.k1 = PROBE_TS_DWORDS;
                k1_t.copy_from_slice(&t.words()[..PROBE_TS_DWORDS]);
                base.surface_changed |= s_changed(&mut s);
                d.words().fill(PROBE_DEST_INIT);
                d.flush();
                step(4);
                if kick(&mut cmd, |w| probe_k2_into(w, arm, &a))? {
                    base.steps_done += 1;
                    l.k2 = N;
                    scratch.words()[2 * N..3 * N].copy_from_slice(d.words());
                    base.surface_changed |= s_changed(&mut s);
                } else {
                    timed_out = true;
                }
            } else {
                timed_out = true;
            }
        } else {
            timed_out = true;
        }
        base.fence_timeouts = timed_out as u32;
        let layout = probe_layout(arm, interleaved);
        for (i, w) in scratch.words()[..N].iter_mut().enumerate() {
            *w = probe_expected_b(layout, i as u32);
        }
        base.guard_changed = s.guard_changed() + t.guard_changed() + d.guard_changed();

        /* K3, the pattern resolve, only after a YES: T as probe_k3_ts, D
         * reset, K2's stream again */
        if readback(scratch.words(), &k1_t, l, base, arm).0 == Outcome::Yes {
            let ts = probe_k3_ts();
            for (w, b) in t.words().iter_mut().zip(ts.as_chunks::<4>().0) {
                *w = u32::from_le_bytes(*b);
            }
            t.flush();
            d.words().fill(PROBE_DEST_INIT);
            d.flush();
            step(5);
            if kick(&mut cmd, |w| probe_k2_into(w, arm, &a))? {
                base.steps_done += 1;
                l.k3 = N;
                scratch.words()[3 * N..4 * N].copy_from_slice(d.words());
                base.guard_changed = s.guard_changed() + t.guard_changed() + d.guard_changed();
            } else {
                base.fence_timeouts += 1;
            }
        }
        step(6);
        Ok(readback(scratch.words(), &k1_t, l, base, arm))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vivante::layout::{self, Layout, SuperOrder};
    use std::vec;
    use std::vec::Vec;

    const ARMS: u32 = 4;

    /// `tsp_classify` of gl_test_probe_decode.inc, transcribed.
    fn tsp_classify(r: &TspReadback) -> u32 {
        if r.timeouts != 0 {
            0xf
        } else if r.guard_changed != 0 || r.s_changed != 0 {
            1
        } else if r.k0_ok == 0 {
            2
        } else if r.t_first_bad != TSP_NOT_FOUND {
            3
        } else if r.n_clear == PROBE_DWORDS as u32 {
            4
        } else if r.n_detile == PROBE_DWORDS as u32 {
            5
        } else if r.n_dead == PROBE_DWORDS as u32 {
            6
        } else {
            7
        }
    }

    fn detile(arm: &ProbeArm, interleaved: bool) -> Vec<u32> {
        (0..PROBE_DWORDS as u32).map(|i| probe_expected_b(probe_layout(arm, interleaved), i)).collect()
    }

    #[test]
    fn the_record_is_the_c_struct() {
        assert_eq!(core::mem::size_of::<TspReadback>(), 19 * 4, "struct tsp_readback: 19 uint32_t");
        assert_eq!(core::mem::align_of::<TspReadback>(), 4);
    }

    #[test]
    fn expected_b_is_layout_rs() {
        for n in 0..ARMS {
            let arm = probe_arm(n).unwrap();
            for (il, order) in [(false, SuperOrder::Drm), (true, SuperOrder::Interleaved)] {
                let l = if arm.super_tiled { Layout::SuperTiled } else { Layout::Tiled };
                for i in 0..PROBE_DWORDS as u32 {
                    let (x, y) = (i % 64, i / 64);
                    let off = layout::offset_in(l, order, x, y, 4, 256) / 4;
                    assert_eq!(probe_expected_b(probe_layout(&arm, il), i), probe_b(off as u32), "arm {n} {x},{y}");
                }
            }
        }
    }

    #[test]
    fn row0_without_allocation_is_k3_expected_clear() {
        let ts = probe_k3_ts();
        for l in [K3Layout::Tiled, K3Layout::SuperTiledDrm, K3Layout::SuperTiledInterleaved] {
            for mode in [TS_MODE_128B, TS_MODE_256B] {
                for low in [true, false] {
                    let clear = k3_expected_clear(l, PROBE_SIDE, PROBE_SIDE, mode, &ts, low);
                    assert_eq!(
                        k3_expected_row0(l, mode, &ts, low),
                        k3_row0_map(&clear, PROBE_SIDE),
                        "{l:?} {mode} {low}"
                    );
                }
            }
        }
        /* the note's maps: 0x55 / 0xaa in 128B, 0x33 / 0xcc in 256B */
        assert_eq!(k3_expected_row0(K3Layout::Tiled, TS_MODE_128B, &ts, true), 0x55);
        assert_eq!(k3_expected_row0(K3Layout::Tiled, TS_MODE_128B, &ts, false), 0xaa);
        assert_eq!(k3_expected_row0(K3Layout::Tiled, TS_MODE_256B, &ts, true), 0x33);
        assert_eq!(k3_expected_row0(K3Layout::Tiled, TS_MODE_256B, &ts, false), 0xcc);
        let a0 = probe_arm(0).unwrap();
        assert_eq!(k3_nibble_order(0x55, &a0), K3_LOW_NIBBLE_FIRST);
        assert_eq!(k3_nibble_order(0xaa, &a0), 0);
        assert_eq!(k3_nibble_order(0x0f, &a0), K3_NO_MATCH);
    }

    #[test]
    fn crc_is_zlibs() {
        /* zlib.crc32(b"\x01\x00\x00\x00\x02\x00\x00\x00") */
        assert_eq!(probe_crc32(&[1, 2]), 0x0381_177c);
        assert_eq!(probe_crc32(&[]), 0);
    }

    #[test]
    fn readback_agrees_with_the_c_classification() {
        let arm = probe_arm(0).unwrap();
        let b = detile(&arm, false);
        let fill = vec![PROBE_FILL; PROBE_TS_DWORDS];
        let mut bad_fill = fill.clone();
        bad_fill[5] = 0;
        let c = vec![PROBE_CLEAR; PROBE_DWORDS];
        let dead = vec![PROBE_DEST_INIT; PROBE_DWORDS];
        let mut mixed = c.clone();
        mixed[7] = 0x1234_5678;
        let base = ArmReads {
            arm: 0,
            steps_done: 3,
            fence_timeouts: 0,
            guard_changed: 0,
            surface_changed: false,
            expected_b: &b,
            k0_d: &b,
            k1_t: &fill,
            k2_d: &c,
            k3_d: &[],
        };
        let cases: [(ArmReads, Outcome); 8] = [
            (base, Outcome::Yes),
            (ArmReads { k2_d: &b, ..base }, Outcome::Ignored),
            (ArmReads { k2_d: &dead, ..base }, Outcome::NoWrite),
            (ArmReads { k2_d: &mixed, ..base }, Outcome::Mixed),
            (ArmReads { k1_t: &bad_fill, ..base }, Outcome::FillMiss),
            (ArmReads { k0_d: &dead, ..base }, Outcome::NoControl),
            (ArmReads { guard_changed: 2, ..base }, Outcome::Guard),
            (ArmReads { fence_timeouts: 1, steps_done: 1, k1_t: &[], k2_d: &[], ..base }, Outcome::Hang),
        ];
        for (r, want) in cases {
            let (o, rb) = probe_readback(&r, &arm);
            assert_eq!(o, want);
            assert_eq!(tsp_classify(&rb), o as u32, "{want:?}: the C twin classifies the record alike");
        }
        let (_, rb) = probe_readback(&ArmReads { k2_d: &mixed, ..base }, &arm);
        assert_eq!((rb.first_idx, rb.first_val, rb.n_clear), (7, 0x1234_5678, 4095));
        assert_eq!(rb.crc, probe_crc32(&mixed));
        let (_, rb) = probe_readback(&ArmReads { k1_t: &bad_fill, ..base }, &arm);
        assert_eq!(rb.t_first_bad, 5);
    }

    #[test]
    fn a_step_fits_the_hooks_buffer() {
        let a = ProbeAddrs { s: 0x1000, t: 0x2000, d: 0x3000 };
        for n in 0..ARMS {
            let arm = probe_arm(n).unwrap();
            for w in [probe_k0(&arm, &a), probe_k1(&a), probe_k2(&arm, &a)] {
                assert!(2 * w.len() + 2 <= 256, "arm {n}: {} writes", w.len());
            }
        }
    }

    /// The hook on a host OSAL whose GPU is `rs_interp` with Mesa's TS
    /// semantics: the board's code path, the design's answers.
    #[cfg(feature = "test-knobs")]
    mod hook_tests {
        use super::*;
        use crate::osal::{OsalGpuMem, OsalOps};
        use crate::testutil::TestGl;
        use crate::vivante::kick::GcKick;
        use crate::vivante::regs::{load_state, GC_OP_LOAD_STATE};
        use crate::vivante::rs_interp::{Memory, RsInterp};
        use core::ffi::c_void;
        use std::boxed::Box;
        use std::sync::Mutex;

        #[derive(Default)]
        struct Gpu {
            allocs: Vec<(u64, Box<[u8]>)>,
            next: u64,
            rs: RsInterp,
            /// Kicks submitted, and each one's decoded writes.
            streams: Vec<Vec<(u32, u32)>>,
            /// The kick (counted from 1) whose fence never signals.
            hang_at: Option<usize>,
            /// A header that is not one LOAD_STATE of one register.
            odd_words: u32,
        }

        static GPU: Mutex<Option<Gpu>> = Mutex::new(None);
        static STEPS: Mutex<Vec<u32>> = Mutex::new(Vec::new());

        fn with<R>(f: impl FnOnce(&mut Gpu) -> R) -> R {
            f(GPU.lock().unwrap_or_else(|e| e.into_inner()).as_mut().expect("the rig's GPU"))
        }

        unsafe extern "C" fn m_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
            with(|g| {
                let mut b = vec![0x5au8; size].into_boxed_slice();
                let gpu = g.next;
                g.next += (size as u64).next_multiple_of(0x1000) + 0x1000;
                let cpu = b.as_mut_ptr() as *mut c_void;
                g.allocs.push((gpu, b));
                // SAFETY: the caller's out-parameter.
                unsafe { *out = OsalGpuMem { cpu, gpu, phys: 0, size, impl_: core::ptr::null_mut() } };
                0
            })
        }

        unsafe extern "C" fn m_free(mem: *mut OsalGpuMem) -> i32 {
            // SAFETY: the caller's descriptor.
            let m = unsafe { &mut *mem };
            with(|g| g.allocs.retain(|(gpu, _)| *gpu != m.gpu || m.cpu.is_null()));
            *m = OsalGpuMem::ZERO;
            0
        }

        unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}

        unsafe extern "C" fn m_submit(desc: *const c_void, len: usize) -> i64 {
            assert_eq!(len, core::mem::size_of::<GcKick>());
            // SAFETY: the back end's descriptor of that size.
            let k = unsafe { *(desc as *const GcKick) };
            with(|g| {
                assert_eq!(k.stream_bytes % 8, 0, "an even number of words");
                let (base, b) = g
                    .allocs
                    .iter()
                    .find(|(gpu, b)| k.stream_gpu >= *gpu && k.stream_gpu < gpu + b.len() as u64)
                    .expect("the stream is an allocation");
                let o = (k.stream_gpu - base) as usize;
                let words: Vec<u32> = b[o..o + k.stream_bytes as usize]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|w| u32::from_le_bytes(*w))
                    .collect();
                let mut writes = Vec::new();
                for p in words.as_chunks::<2>().0 {
                    if p[0] & 0xf800_0000 == GC_OP_LOAD_STATE && (p[0] >> 16) & 0x3ff == 1 {
                        writes.push(((p[0] & 0xffff) * 4, p[1]));
                    } else {
                        g.odd_words += 1;
                    }
                }
                g.streams.push(writes.clone());
                if g.hang_at != Some(g.streams.len()) {
                    let mut mem = Memory::new();
                    for (gpu, b) in &g.allocs {
                        mem.map(*gpu as u32, b.to_vec());
                    }
                    g.rs.run(&writes, &mut mem);
                    for (gpu, b) in g.allocs.iter_mut() {
                        b.copy_from_slice(mem.region(*gpu as u32));
                    }
                }
                g.streams.len() as i64
            })
        }

        unsafe extern "C" fn m_wait(fence: i64, timeout_us: i64) -> i32 {
            assert_eq!(timeout_us, 2_000_000, "the probe's wait is bounded");
            with(|g| (g.hang_at == Some(fence as usize)) as i32)
        }

        unsafe extern "C" fn m_completed() -> i64 {
            with(|g| g.streams.len() as i64)
        }

        unsafe extern "C" fn m_step(s: u32) {
            STEPS.lock().unwrap_or_else(|e| e.into_inner()).push(s);
        }

        const OPS: OsalOps = OsalOps {
            init: None,
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
            query: None,
            log: None,
            wait_vblank: None,
            present: None,
            lock: None,
            unlock: None,
            thread_id: None,
            display_busy: None,
        };
        static GPU_OSAL: OsalOps = OPS;
        static NO_SUBMIT_OSAL: OsalOps = OsalOps { submit: None, ..OPS };

        /// The serialising guard and [`GPU_OSAL`] with a fresh GPU.
        struct Rig {
            prev: *const OsalOps,
            _gl: TestGl,
        }

        impl Rig {
            fn new(ts_source: bool, hang_at: Option<usize>) -> Rig {
                let gl = TestGl::new(4, 4);
                let mut rs = RsInterp::new();
                rs.ts_source = ts_source;
                *GPU.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(Gpu { next: 0x1000_0800, rs, hang_at, ..Gpu::default() });
                STEPS.lock().unwrap_or_else(|e| e.into_inner()).clear();
                // SAFETY: tests are serialised by the TestGl guard.
                let prev = unsafe {
                    let prev = crate::osal::g_osal;
                    crate::osal::osal_set_ops(&GPU_OSAL);
                    prev
                };
                Rig { prev, _gl: gl }
            }

            fn steps(&self) -> Vec<u32> {
                STEPS.lock().unwrap_or_else(|e| e.into_inner()).clone()
            }
        }

        impl Drop for Rig {
            fn drop(&mut self) {
                // SAFETY: as in `new`.
                unsafe { crate::osal::osal_set_ops(self.prev) };
                *GPU.lock().unwrap_or_else(|e| e.into_inner()) = None;
            }
        }

        fn probe(arm: u32) -> (i32, TspReadback) {
            let mut rb = TspReadback::default();
            // SAFETY: a TspReadback of the size passed; `m_step` takes any u32.
            let rc =
                unsafe { gc_debug_ts_probe(arm, &mut rb, core::mem::size_of::<TspReadback>() as u32, Some(m_step)) };
            (rc, rb)
        }

        #[test]
        fn every_arm_reads_yes_on_mesas_ts_semantics() {
            for n in 0..ARMS {
                let r = Rig::new(true, None);
                let arm = probe_arm(n).unwrap();
                let (rc, rb) = probe(n);
                assert_eq!(rc, Outcome::Yes as i32, "arm {n}: {rb:?}");
                assert_eq!(tsp_classify(&rb), rc as u32, "arm {n}: the C twin agrees");
                assert_eq!(
                    (rb.arm, rb.steps_done, rb.timeouts, rb.guard_changed, rb.s_changed, rb.k0_ok, rb.t_first_bad),
                    (n, 4, 0, 0, 0, 1, TSP_NOT_FOUND),
                    "arm {n}"
                );
                assert_eq!((rb.n_clear, rb.n_detile, rb.n_dead, rb.first_idx), (4096, 0, 0, TSP_NOT_FOUND), "arm {n}");
                assert_eq!(rb.crc, probe_crc32(&[PROBE_CLEAR; PROBE_DWORDS]));
                let want_map = k3_expected_row0(probe_layout(&arm, false), arm.ts_mode, &probe_k3_ts(), true);
                assert_eq!(
                    (rb.k3_present, rb.k3_map, rb.k3_nibble_order),
                    (1, want_map, K3_LOW_NIBBLE_FIRST),
                    "arm {n}"
                );
                assert_eq!(rb.k3_n_clear + rb.k3_n_detile, 4096, "arm {n}: K3 is C or B everywhere");
                assert_eq!(r.steps(), [1, 2, 3, 4, 5, 6], "arm {n}");

                /* the four kicks are K0, K1, K2 and K2 again, one LOAD_STATE a
                 * register, at the objects' addresses */
                let (streams, odd, live) = with(|g| (g.streams.clone(), g.odd_words, g.allocs.len()));
                assert_eq!((streams.len(), odd, live), (4, 0, 0), "arm {n}: four kicks, nothing left allocated");
                let reg = |s: &[(u32, u32)], r: u32| s.iter().rev().find(|w| w.0 == r).unwrap().1;
                let a = ProbeAddrs {
                    s: reg(&streams[2], TS_COLOR_SURFACE_BASE),
                    t: reg(&streams[2], TS_COLOR_STATUS_BASE),
                    d: reg(&streams[2], RS_PIPE_DEST_ADDR0),
                };
                assert!([a.s, a.t, a.d].iter().all(|x| x % 0x1000 == 0), "{a:?}");
                assert_eq!(streams[0], probe_k0(&arm, &a));
                assert_eq!(streams[1], probe_k1(&a));
                assert_eq!(streams[2], probe_k2(&arm, &a));
                assert_eq!(streams[3], probe_k3(&arm, &a));
                assert_eq!(load_state(RS_CONFIG, 1, false), 0x0801_0581, "the note's hdr()");
            }
        }

        #[test]
        fn a_ts_the_rs_ignores_reads_ignored_and_runs_no_k3() {
            let r = Rig::new(false, None);
            let (rc, rb) = probe(0);
            assert_eq!(rc, Outcome::Ignored as i32);
            assert_eq!(tsp_classify(&rb), rc as u32);
            assert_eq!((rb.steps_done, rb.n_detile, rb.n_clear, rb.k3_present), (3, 4096, 0, 0));
            assert_eq!(r.steps(), [1, 2, 3, 4, 6]);
            assert_eq!(with(|g| g.streams.len()), 3);
        }

        #[test]
        fn a_hang_stops_the_arm() {
            let r = Rig::new(true, Some(2));
            let (rc, rb) = probe(1);
            assert_eq!(rc, 0xf, "HANG");
            assert_eq!(tsp_classify(&rb), 0xf);
            assert_eq!((rb.steps_done, rb.timeouts, rb.k0_ok, rb.k3_present), (1, 1, 1, 0));
            assert_eq!(with(|g| g.streams.len()), 2, "no kick after the timeout");
            assert_eq!(r.steps(), [1, 2, 3, 6]);
        }

        #[test]
        fn the_hook_refuses_what_it_cannot_run() {
            let _r = Rig::new(true, None);
            let mut rb = TspReadback::default();
            let size = core::mem::size_of::<TspReadback>() as u32;
            // SAFETY: null and short records are refused before any write.
            unsafe {
                assert_eq!(gc_debug_ts_probe(0, core::ptr::null_mut(), size, None), PROBE_ERR_ARGS);
                assert_eq!(gc_debug_ts_probe(0, &mut rb, size - 4, None), PROBE_ERR_ARGS);
                assert_eq!(gc_debug_ts_probe(4, &mut rb, size, None), PROBE_ERR_ARM);
                crate::osal::osal_set_ops(&NO_SUBMIT_OSAL);
                assert_eq!(gc_debug_ts_probe(0, &mut rb, size, None), PROBE_ERR_OSAL);
                crate::osal::osal_set_ops(&GPU_OSAL);
            }
            assert_eq!(rb, TspReadback::default(), "a refusal writes nothing");
            assert_eq!(with(|g| g.streams.len()), 0);
        }
    }
}
