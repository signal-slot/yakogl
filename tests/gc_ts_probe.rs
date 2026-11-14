// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Case 243 `tsprobe`'s streams (`src/vivante/ts.rs`, K0 to K3) on the host
//! (docs/notes/gc-ts-probe.md, "Part (iii)").
//!
//! Each arm's four kicks run through `rs_interp` with its TS read on, over a
//! memory with the note's guarded objects, and the readback goes through
//! `ts::classify`: every arm must give `YES`, and K3 must read the map
//! `ts::k3_expected_clear` predicts. That is the design's self-consistency
//! (the words, Mesa's TS semantics as `rs_interp` models them, and
//! `layout.rs`), not a claim about the board. The words are also compared
//! with `tools/vivante/testdata/ts-clear.golden` (mode 29): the fill block
//! equal, the resolve block equal apart from the note's "golden diff".
//!
//! Run: `cargo test --test gc_ts_probe --features host-test,gpu-vivante`.

#![cfg(all(feature = "host-test", feature = "gpu-vivante"))]

use yakogl::vivante::layout::{self, Layout, SuperOrder};
use yakogl::vivante::regs::*;
use yakogl::vivante::rs_interp::{Memory, RsInterp};
use yakogl::vivante::ts::{self, *};

const GUARD: u32 = 0x1000;

// --- the golden ----------------------------------------------------------

/// The `(register, value)` writes of the golden's `LOAD_STATE` lines, in
/// order (a count of n writes registers `reg + 4i`). Lines cut short with
/// `...` keep the values printed; no RS block has one.
fn golden_writes() -> Vec<(u32, u32)> {
    let text = include_str!("../tools/vivante/testdata/ts-clear.golden");
    let mut out = Vec::new();
    for line in text.lines().filter(|l| l.starts_with("LOAD_STATE")) {
        let mut f = line.split_whitespace();
        f.next();
        let reg = u32::from_str_radix(f.next().unwrap().trim_start_matches("0x"), 16).unwrap();
        let vals = f
            .skip_while(|t| !t.starts_with("count="))
            .skip(1)
            .filter(|t| t.starts_with("0x"))
            .map(|t| u32::from_str_radix(t.trim_start_matches("0x"), 16).unwrap());
        out.extend(vals.enumerate().map(|(i, v)| (reg + 4 * i as u32, v)));
    }
    out
}

/// The golden's first block: the colour TS fill, from the first write to the
/// first `RS_SINGLE_BUFFER` 0.
fn golden_fill_block(g: &[(u32, u32)]) -> &[(u32, u32)] {
    let end = g.iter().position(|&w| w == (GC_RS_SINGLE_BUFFER, 0)).unwrap();
    &g[..=end]
}

/// The golden's last block: the TS-sourced resolve, from the last colour and
/// depth flush to the end.
fn golden_resolve_block(g: &[(u32, u32)]) -> &[(u32, u32)] {
    let start = g.iter().rposition(|&w| w == (GC_GL_FLUSH_CACHE, GC_FLUSH_COLOR | GC_FLUSH_DEPTH)).unwrap();
    &g[start..]
}

/// The golden's objects: `T` the colour TS, `S` the colour surface, `D` the
/// resolve's destination.
const GOLDEN: ProbeAddrs = ProbeAddrs { s: 0xffff_7000, t: 0xffff_6000, d: 0xffee_a000 };

fn arm(n: u32) -> ProbeArm {
    probe_arm(n).unwrap()
}

/// The registers where `ours` differs from `golden`, with our value; the two
/// must write the same registers in the same order.
fn diff(ours: &[(u32, u32)], golden: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let regs = |w: &[(u32, u32)]| w.iter().map(|p| p.0).collect::<Vec<_>>();
    assert_eq!(regs(ours), regs(golden), "the same registers in the same order");
    ours.iter().zip(golden).filter(|(a, b)| a.1 != b.1).map(|(a, _)| *a).collect()
}

#[test]
fn register_copies_are_regs_rs() {
    for (ours, theirs) in [
        (ts::GL_FLUSH_CACHE, GC_GL_FLUSH_CACHE),
        (ts::GL_SEMAPHORE_TOKEN, GC_GL_SEMAPHORE_TOKEN),
        (ts::GL_STALL_TOKEN, GC_GL_STALL_TOKEN),
        (ts::TS_FLUSH_CACHE, GC_TS_FLUSH_CACHE),
        (ts::TS_MEM_CONFIG, GC_TS_MEM_CONFIG),
        (ts::TS_COLOR_STATUS_BASE, GC_TS_COLOR_STATUS_BASE),
        (ts::TS_COLOR_SURFACE_BASE, GC_TS_COLOR_SURFACE_BASE),
        (ts::TS_COLOR_CLEAR_VALUE, GC_TS_COLOR_CLEAR_VALUE),
        (ts::TS_COLOR_AUTO_DISABLE_COUNT, GC_TS_COLOR_AUTO_DISABLE_COUNT),
        (ts::TS_COLOR_CLEAR_VALUE_EXT, GC_TS_COLOR_CLEAR_VALUE_EXT),
        (ts::RS_KICKER, GC_RS_KICKER),
        (ts::RS_CONFIG, GC_RS_CONFIG),
        (ts::RS_SOURCE_STRIDE, GC_RS_SOURCE_STRIDE),
        (ts::RS_DEST_STRIDE, GC_RS_DEST_STRIDE),
        (ts::RS_WINDOW_SIZE, GC_RS_WINDOW_SIZE),
        (ts::RS_DITHER0, rs_dither(0)),
        (ts::RS_CLEAR_CONTROL, GC_RS_CLEAR_CONTROL),
        (ts::RS_FILL_VALUE0, rs_fill_value(0)),
        (ts::RS_EXTRA_CONFIG, GC_RS_EXTRA_CONFIG),
        (ts::RS_SINGLE_BUFFER, GC_RS_SINGLE_BUFFER),
        (ts::RS_PIPE_SOURCE_ADDR0, rs_pipe_source_addr(0)),
        (ts::RS_PIPE_DEST_ADDR0, rs_pipe_dest_addr(0)),
        (ts::RS_PIPE_OFFSET0, rs_pipe_offset(0)),
        (ts::GL_FLUSH_COLOR_DEPTH, GC_FLUSH_COLOR | GC_FLUSH_DEPTH),
        (ts::STALL_RA_PE, stall_token(GC_SYNC_RA, GC_SYNC_PE)),
        (ts::TS_FLUSH, GC_TS_FLUSH),
        (ts::RS_KICKER_VALUE, GC_RS_KICKER_VALUE),
        (ts::RS_SINGLE_BUFFER_ENABLE, GC_RS_SINGLE_BUFFER_ENABLE),
        (ts::RS_CONFIG_SOURCE_TILED, GC_RS_SOURCE_TILED),
        (ts::RS_CONFIG_DEST_TILED, GC_RS_DEST_TILED),
        (ts::RS_FORMAT_A8R8G8B8, GC_RS_FORMAT_A8R8G8B8),
        (ts::RS_STRIDE_TILING, GC_RS_SOURCE_STRIDE_TILING),
        (ts::RS_STRIDE_SUPER_TILED_NEW, GC_RS_SOURCE_STRIDE_SUPER_TILED_NEW),
        (ts::TS_MEM_COLOR_FAST_CLEAR, GC_TS_MEM_CONFIG_COLOR_FAST_CLEAR),
        (ts::TS_MEM_COLOR_AUTO_DISABLE, GC_TS_MEM_CONFIG_COLOR_AUTO_DISABLE),
    ] {
        assert_eq!(ours, theirs, "{ours:#x}");
    }
    assert_eq!(ts::PROBE_TS_BYTES, ts_size(PROBE_SIDE, PROBE_SIDE, 4, false));
    assert_eq!(ts::PROBE_TS_BYTES, ts_size(PROBE_SIDE, PROBE_SIDE, 4, true));
}

#[test]
fn the_arms_are_the_notes_table() {
    let words: Vec<_> = (0..4).map(|n| (arm(n).mem_config, probe_source_stride(&arm(n), arm(n).ts_mode))).collect();
    assert_eq!(words, [(0x2, 0x400), (0x22, 0x400), (0x382, 0x2000_0400), (0x2, 0x8800_0400)]);
    assert_eq!(arm(1).auto_disable_count, Some(256));
    assert!((0..4).filter(|&n| n != 1).all(|n| arm(n).auto_disable_count.is_none()));
    assert_eq!(probe_arm(4), None);
}

#[test]
fn k1_is_the_goldens_fill_block() {
    let g = golden_writes();
    assert_eq!(probe_k1(&GOLDEN), golden_fill_block(&g));
}

#[test]
fn k2_differs_from_the_goldens_resolve_only_by_the_notes_diff() {
    let g = golden_writes();
    let golden = golden_resolve_block(&g);
    let config = (GC_RS_CONFIG, 0x0000_0686);
    for n in 0..4 {
        let mut ours = probe_k2(&arm(n), &GOLDEN);
        if n == 1 {
            // The one write the golden lacks, right after the clear values.
            let i = ours.iter().position(|w| w.0 == GC_TS_COLOR_AUTO_DISABLE_COUNT).unwrap();
            assert_eq!(ours[i - 1].0, GC_TS_COLOR_CLEAR_VALUE_EXT);
            assert_eq!(ours.remove(i), (GC_TS_COLOR_AUTO_DISABLE_COUNT, 256));
        }
        let want = match n {
            0 => vec![(GC_TS_MEM_CONFIG, 0x2), config, (GC_RS_SOURCE_STRIDE, 0x400)],
            1 => vec![(GC_TS_MEM_CONFIG, 0x22), config, (GC_RS_SOURCE_STRIDE, 0x400)],
            2 => vec![config, (GC_RS_SOURCE_STRIDE, 0x2000_0400)],
            _ => vec![(GC_TS_MEM_CONFIG, 0x2), config, (GC_RS_SOURCE_STRIDE, 0x8800_0400)],
        };
        assert_eq!(diff(&ours, golden), want, "arm {n}");
        assert_eq!(probe_k3(&arm(n), &GOLDEN), probe_k2(&arm(n), &GOLDEN), "K3 is K2 again");
    }
}

#[test]
fn k0_is_the_resolve_without_its_ts_words() {
    let g = golden_writes();
    let ts_words =
        [GC_TS_COLOR_STATUS_BASE, GC_TS_COLOR_SURFACE_BASE, GC_TS_COLOR_CLEAR_VALUE, GC_TS_COLOR_CLEAR_VALUE_EXT];
    let golden: Vec<_> = golden_resolve_block(&g).iter().copied().filter(|w| !ts_words.contains(&w.0)).collect();
    let config = (GC_RS_CONFIG, 0x0000_0686);
    for n in 0..4 {
        let stride = if n == 3 { 0x8800_0400 } else { 0x400 };
        let want = vec![(GC_TS_MEM_CONFIG, 0), config, (GC_RS_SOURCE_STRIDE, stride)];
        assert_eq!(diff(&probe_k0(&arm(n), &GOLDEN), &golden), want, "arm {n}");
    }
}

// --- the arms through rs_interp -----------------------------------------

/// The case's memory: `S`, `T` and `D`, each with a guard page on both sides
/// (one region a object, mapped at the guard's start).
struct ProbeMem {
    mem: Memory,
    a: ProbeAddrs,
}

fn dwords(b: &[u8]) -> Vec<u32> {
    b.chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect()
}

fn bytes(w: impl IntoIterator<Item = u32>) -> Vec<u8> {
    w.into_iter().flat_map(u32::to_le_bytes).collect()
}

impl ProbeMem {
    fn new() -> Self {
        let a = ProbeAddrs { s: 0x0010_1000, t: 0x0020_1000, d: 0x0030_1000 };
        let mut mem = Memory::new();
        let guarded = |inner: Vec<u32>| {
            let g = vec![PROBE_GUARD; (GUARD / 4) as usize];
            bytes(g.iter().copied().chain(inner).chain(g.iter().copied()))
        };
        mem.map(a.s - GUARD, guarded((0..PROBE_DWORDS as u32).map(probe_b).collect()));
        mem.map(a.t - GUARD, guarded(vec![0; (PROBE_TS_BYTES / 4) as usize]));
        mem.map(a.d - GUARD, guarded(vec![PROBE_DEST_INIT; PROBE_DWORDS]));
        ProbeMem { mem, a }
    }

    fn read(&self, addr: u32, n: u32) -> Vec<u32> {
        dwords(self.mem.read(addr as u64, (n * 4) as usize))
    }

    fn d(&self) -> Vec<u32> {
        self.read(self.a.d, PROBE_DWORDS as u32)
    }

    fn reset_d(&mut self) {
        self.mem.write(self.a.d as u64, &bytes(vec![PROBE_DEST_INIT; PROBE_DWORDS]));
    }

    fn surface_changed(&self) -> bool {
        self.read(self.a.s, PROBE_DWORDS as u32).iter().enumerate().any(|(i, &w)| w != probe_b(i as u32))
    }

    fn guard_changed(&self) -> u32 {
        let (s, t, d) = (self.a.s, self.a.t, self.a.d);
        [(s, PROBE_SURFACE_BYTES), (t, PROBE_TS_BYTES), (d, PROBE_SURFACE_BYTES)]
            .into_iter()
            .flat_map(|(base, size)| [self.read(base - GUARD, GUARD / 4), self.read(base + size, GUARD / 4)])
            .flatten()
            .filter(|&w| w != PROBE_GUARD)
            .count() as u32
    }
}

/// `detile(B)`: `S`'s dwords in linear order, through `layout.rs`.
fn detile_b(super_tiled: bool) -> Vec<u32> {
    let l = if super_tiled { Layout::SuperTiled } else { Layout::Tiled };
    let stride = layout::stride(l, PROBE_SIDE, 4, true);
    let mut out = Vec::with_capacity(PROBE_DWORDS);
    for y in 0..PROBE_SIDE {
        for x in 0..PROBE_SIDE {
            out.push(probe_b((layout::offset(l, x, y, 4, stride) / 4) as u32));
        }
    }
    out
}

/// What one arm reads back, step by step.
struct ArmRun {
    k0_d: Vec<u32>,
    k1_t: Vec<u32>,
    k2_d: Vec<u32>,
    k3_d: Vec<u32>,
    guard_changed: u32,
    surface_changed: bool,
}

/// K0 to K3 of `arm` in one interpreter (the GPU keeps its registers from
/// kick to kick), with the CPU's writes between them as the note gives them.
fn run_arm(arm: &ProbeArm, ts_read: bool) -> ArmRun {
    let mut m = ProbeMem::new();
    let a = m.a;
    let mut rs = RsInterp::new();
    rs.ts_source = ts_read;
    rs.run(&probe_k0(arm, &a), &mut m.mem);
    let k0_d = m.d();
    rs.run(&probe_k1(&a), &mut m.mem);
    let k1_t = m.read(a.t, PROBE_TS_DWORDS as u32);
    m.reset_d();
    rs.run(&probe_k2(arm, &a), &mut m.mem);
    let k2_d = m.d();
    let (guard_changed, surface_changed) = (m.guard_changed(), m.surface_changed());
    m.mem.write(a.t as u64, &probe_k3_ts());
    m.reset_d();
    rs.run(&probe_k3(arm, &a), &mut m.mem);
    assert_eq!(rs.jobs.len(), 4, "one RS job a step");
    assert_eq!(rs.jobs[2].ts.is_some(), ts_read, "K2 reads through the TS when TS read is on");
    assert!(rs.jobs[0].ts.is_none() && rs.jobs[1].ts.is_none(), "K0 (TS off) and K1 (a fill) do not");
    ArmRun { k0_d, k1_t, k2_d, k3_d: m.d(), guard_changed, surface_changed }
}

fn classify_run(arm: &ProbeArm, r: &ArmRun) -> Outcome {
    let expected_b = detile_b(arm.super_tiled);
    classify(&ProbeReadback {
        fence_timeouts: 0,
        guard_changed: r.guard_changed,
        surface_changed: r.surface_changed,
        expected_b: &expected_b,
        k0_d: &r.k0_d,
        k1_t: &r.k1_t,
        k2_d: &r.k2_d,
        clear: PROBE_CLEAR,
    })
}

/// Arm `n` gives `YES`, and K3 reads `k3_expected_clear`'s map with
/// `row0_map` in row 0 (low nibble first).
fn check_arm(n: u32, row0_map: u32) {
    let arm = arm(n);
    let r = run_arm(&arm, true);
    assert_eq!(classify_run(&arm, &r), Outcome::Yes, "arm {n}");
    let expected_b = detile_b(arm.super_tiled);
    assert_eq!(probe_counts(&r.k2_d, PROBE_CLEAR, &expected_b), (PROBE_DWORDS as u32) << 16);
    assert_eq!(probe_diff_w1(&r.k2_d, PROBE_CLEAR, &expected_b), 0xffff_ffff);
    assert_eq!(probe_diff_w3(&r.k1_t, r.guard_changed), 0xffff_0000);
    assert_eq!(probe_w1(Outcome::Yes, n, 4, 0), 0x404 | n << 4);

    // K3: C where the pattern's nibble is 1, B elsewhere.
    assert_eq!(layout::SUPER_ORDER, SuperOrder::Drm, "rs_interp's super-tiles are layout.rs's Drm order");
    let k3l = if arm.super_tiled { K3Layout::SuperTiledDrm } else { K3Layout::Tiled };
    let clear = k3_expected_clear(k3l, PROBE_SIDE, PROBE_SIDE, arm.ts_mode, &probe_k3_ts(), true);
    let want: Vec<u32> = clear.iter().zip(&expected_b).map(|(&c, &b)| if c { PROBE_CLEAR } else { b }).collect();
    assert_eq!(r.k3_d, want, "arm {n}: K3");
    assert_eq!(k3_row0_map(&clear, PROBE_SIDE), row0_map, "arm {n}: row-0 map");
    let cleared = clear.iter().filter(|&&c| c).count() as u32;
    assert_eq!(probe_counts(&r.k3_d, PROBE_CLEAR, &expected_b), cleared << 16 | (PROBE_DWORDS as u32 - cleared));
}

#[test]
fn arm0_tiled_128b_is_yes() {
    check_arm(0, 0x55);
}

#[test]
fn arm1_auto_disable_is_yes() {
    check_arm(1, 0x55);
}

#[test]
fn arm2_256b_is_yes() {
    check_arm(2, 0x33);
}

#[test]
fn arm3_super_tiled_is_yes() {
    // 128B granules of a Drm super-tile: pixel (8k, 0) is at byte 512k
    // (`(x & 0x38) << 4` dwords), granule 4k, always an even granule and
    // within T's 64 bytes of 0x01; so every sampled dword of row 0 is C (low
    // nibble first; high nibble first would read 0x00). The row-0 map says
    // nothing about the super-tile order here (ts-probe-classify's finding).
    check_arm(3, 0xff);
}

#[test]
fn every_arm_reads_ignored_with_ts_read_off() {
    // rs_interp's default: the TS words are tracked, not applied, so the
    // resolve reads S as K0 did. The classification then says IGNORED.
    for n in 0..4 {
        let arm = arm(n);
        let r = run_arm(&arm, false);
        assert_eq!(classify_run(&arm, &r), Outcome::Ignored, "arm {n}");
        assert_eq!(r.k3_d, detile_b(arm.super_tiled), "arm {n}: K3 ignores the TS too");
    }
}
