// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The cluster sample's check, present accounting and records
//! (driver/samples/zephyr/cluster/src/main.c, on the board layer), over the
//! crate `cluster_check` (../check). C entry points, from the UI thread:
//!
//! - [`cluster_board_check`]: the checked frame against the host reference
//!   for its size; prints each probe, the summary lines and the thumbnail
//!   (host/ref/compare.py reads it), and gives the PROBES and FINGERPRINT
//!   words;
//! - [`cluster_board_frame`]: one presented animation frame's times;
//! - [`cluster_board_report`]: the status line of the interval since the
//!   last one;
//! - [`cluster_board_finish`]: the animation's line, and the PERF and APP
//!   words;
//! - [`cluster_board_mem`]: what each heap actually holds at one stage.

use core::ffi::c_void;
use core::fmt::Write;

use cluster_check::{
    check, record, thumb_blocks, write_thumb_row, Frame, FrameTimes, Present, FRAME_BUDGET_MS, PART_NAMES, THUMB,
    TOLERANCE,
};

use crate::{Pk, Single, STATE};

/// One console line through printk (Pk flushes at the newline).
macro_rules! line {
    ($($arg:tt)*) => {{
        let mut w: Pk = Pk::new();
        let _ = writeln!(w, $($arg)*);
    }};
}

/// main.c's `struct cluster_sequence`: the checked frame's sequence and the
/// window's fill values, from the check crate (the one definition, which
/// host/ref uses too).
#[repr(C)]
pub struct Sequence {
    pub frame_ticks: u32,
    pub frame_ms: [u32; 3],
    pub tick_ms: u32,
    pub guard_rows: u32,
    pub sentinel: u32,
    pub guard: u32,
}

/// main.c's `struct cluster_check_result`.
#[repr(C)]
pub struct CheckResult {
    /// record::STATUS_PASS or STATUS_FAIL: the check alone.
    pub status: u32,
    pub probes: [u32; 3],
    pub fingerprint: [u32; 3],
    /// The CHECK record of a failed check (record.rs): its tag (CHECK_*),
    /// 0 for a pass, and w1..w3.
    pub check_tag: u32,
    pub check: [u32; 3],
}

/// main.c's `struct cluster_frame_times`, in microseconds.
#[repr(C)]
pub struct CFrameTimes {
    pub draw: u32,
    pub wait: u32,
    pub show: u32,
    pub swap: u32,
    pub frame: u32,
}

/// main.c's `struct cluster_run_records`.
#[repr(C)]
pub struct RunRecords {
    pub app: [u32; 3],
    pub perf: [u32; 3],
}

/// main.c's `struct cluster_mem`: what each allocator actually holds, in
/// bytes. A heap is `now, most at once since init, of`; the carve-out adds
/// the bytes its held frees keep back. `missing` is main.c's MEM_MISSING_*:
/// a group this build has no counter for, printed as "not measured".
#[repr(C)]
pub struct Mem {
    /// The stage the numbers were read at (main.c's `enum stage`).
    pub stage: u32,
    /// The window the board gave (stride x rows); 0 before it exists.
    pub window: u32,
    pub ui: [u32; 3],
    pub malloc: [u32; 3],
    pub keep: [u32; 3],
    pub gpu: [u32; 4],
    pub missing: u32,
}

const _: () = assert!(core::mem::size_of::<Sequence>() == 32);
const _: () = assert!(core::mem::size_of::<CheckResult>() == 44);
const _: () = assert!(core::mem::size_of::<CFrameTimes>() == 20);
const _: () = assert!(core::mem::size_of::<RunRecords>() == 24);
const _: () = assert!(core::mem::size_of::<Mem>() == 64);

/// main.c's MEM_MISSING_*: the group has no counter in this build.
const MISSING_UI: u32 = 1;
const MISSING_MALLOC: u32 = 2;
const MISSING_KEEP: u32 = 4;
const MISSING_GPU: u32 = 8;

const GL_RGBA: u32 = 0x1908;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
type ReadPixels = extern "C" fn(i32, i32, i32, i32, u32, u32, *mut c_void);

/// The whole animation, and the interval since the last report.
struct Accounting {
    total: Present,
    interval: Present,
}

const NOTHING: Present = Present { presented: 0, sum: [0; 5], worst: [0; 5] };
static ACC: Single<Accounting> = Single::new(Accounting { total: NOTHING, interval: NOTHING });

/// Fill main.c's sequence from the check crate.
///
/// # Safety
/// `out` is writable.
#[no_mangle]
pub unsafe extern "C" fn cluster_board_sequence(out: *mut Sequence) {
    // SAFETY: the caller's contract.
    let out = unsafe { &mut *out };
    *out = Sequence {
        frame_ticks: cluster_check::FRAME_TICKS,
        frame_ms: [0, cluster_check::FRAME2_MS as u32, cluster_check::FRAME3_MS as u32],
        tick_ms: cluster_check::TICK_MS as u32,
        guard_rows: cluster_check::GUARD_ROWS,
        sentinel: cluster_check::SENTINEL,
        guard: cluster_check::GUARD,
    };
}

/// Check the window's first `h` rows (half 0 of main.c's window) and the
/// `guard_rows` after them. 0: the check passed; 1: it ran and failed; -1:
/// the frame could not be checked (the reason is printed). `out` is filled
/// in every case (status FAIL unless 0).
///
/// # Safety
/// `cpu` is readable for (h + guard_rows) * w words and nothing writes it
/// during the call (the GPU is idle: main.c waited for it); `out` is
/// writable.
#[no_mangle]
pub unsafe extern "C" fn cluster_board_check(
    cpu: *const u32,
    w: u32,
    h: u32,
    guard_rows: u32,
    format: u32,
    out: *mut CheckResult,
) -> i32 {
    // SAFETY: the caller's contract.
    let out = unsafe { &mut *out };
    *out = CheckResult { status: record::STATUS_FAIL, probes: [0; 3], fingerprint: [0; 3], check_tag: 0, check: [0; 3] };
    let n = (h as usize + guard_rows as usize) * w as usize;
    // SAFETY: the caller's contract.
    let pixels = unsafe { core::slice::from_raw_parts(cpu, n) };
    let frame = match Frame::new(pixels, w, h, guard_rows, format) {
        Ok(f) => f,
        Err(e) => {
            line!("cluster: check: not checked: {} ({w}x{h}, format {format:#x})", e.what());
            return -1;
        }
    };
    // SAFETY: only the UI thread touches STATE.
    let get_proc = unsafe { STATE.get() }.get_proc;
    let read_pixels: Option<ReadPixels> = get_proc.and_then(|gp| {
        let p = gp(c"glReadPixels".as_ptr());
        // SAFETY: glReadPixels' C signature, as eglGetProcAddress returns it.
        (!p.is_null()).then(|| unsafe { core::mem::transmute::<*const c_void, ReadPixels>(p) })
    });
    let Some(read_pixels) = read_pixels else {
        line!("cluster: check: not checked: glReadPixels was not resolved (no frame was drawn)");
        return -1;
    };
    let o = check(&frame, |x, y| {
        let mut px = [0u8; 4];
        read_pixels(x as i32, y as i32, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
        px
    });
    line!(
        "window {w}x{h}: {} sentinel pixels left, {} with alpha != 255, crc32 {:#010x}",
        o.sentinel_left,
        o.alpha_bad,
        o.crc32
    );
    line!("  guard rows {} of {} pixels changed", o.guard_bad, w * guard_rows);
    match o.reference {
        None => line!("probes: none for a {w}x{h} window (check/src/references.rs has no table for it)"),
        Some(r) => {
            for (i, p) in r.probes.iter().take(o.n).enumerate() {
                let (m, g) = (o.points[i], o.via_gl[i]);
                line!(
                    "  probe {i:2} ({:4},{:3}) = {:3},{:3},{:3}: host {:3},{:3},{:3}, diff {}{}{}",
                    p.x,
                    p.y,
                    m[0],
                    m[1],
                    m[2],
                    p.rgb[0],
                    p.rgb[1],
                    p.rgb[2],
                    o.diff[i],
                    if o.diff[i] > TOLERANCE { "  WRONG" } else { "" },
                    if g != m { "  glReadPixels DIFFERENT" } else { "" }
                );
            }
            line!(
                "probes: {} of {} within {TOLERANCE} of the host reference (worst {}); glReadPixels differs at {}",
                o.n as u32 - o.probes_bad,
                o.n,
                o.worst,
                o.reads_bad
            );
        }
    }
    let (bw, bh) = thumb_blocks(&frame);
    line!("thumb {bw}x{bh} blocks of {THUMB} px, window {w}x{h}");
    for by in 0..bh {
        // a 1920-wide window's row is 9 + 96 * 6 bytes: one printk line
        let mut t: Pk<640> = Pk::new();
        let _ = write_thumb_row(&mut t, &frame, by);
        t.flush();
    }
    let verdict = o.verdict();
    line!("cluster: check: {}", verdict.map_or("PASS", |f| f.what()));
    out.status = if verdict.is_none() { record::STATUS_PASS } else { record::STATUS_FAIL };
    out.probes = o.probes_words();
    out.fingerprint = o.fingerprint_words(w, h);
    if let Some((tag, words)) = o.check_words() {
        out.check_tag = tag as u32;
        out.check = words;
    }
    verdict.is_some() as i32
}

/// One presented frame. Returns 0, or 1 when the frame took longer than
/// FRAME_BUDGET_MS (main.c fails the run then).
///
/// # Safety
/// `t` is readable.
#[no_mangle]
pub unsafe extern "C" fn cluster_board_frame(t: *const CFrameTimes) -> i32 {
    // SAFETY: the caller's contract.
    let t = unsafe { &*t };
    let times = FrameTimes { draw: t.draw, wait: t.wait, show: t.show, swap: t.swap, frame: t.frame };
    // SAFETY: only the UI thread touches ACC.
    let acc = unsafe { ACC.get() };
    acc.total.add(&times);
    acc.interval.add(&times);
    (t.frame > FRAME_BUDGET_MS * 1000) as i32
}

/// " <part> <mean> / <worst>" for each part, in ms with one decimal.
fn write_parts(out: &mut impl Write, p: &Present) {
    for (i, name) in PART_NAMES.iter().enumerate() {
        let (mean, worst) = (p.mean_us(i), p.worst[i] as u64);
        let _ = write!(out, " {name} {}.{} / {}.{}", mean / 1000, mean % 1000 / 100, worst / 1000, worst % 1000 / 100);
    }
}

/// The status line of the interval since the last report (`interval_ms`
/// long, ending `t_ms` into the animation); then a new interval.
#[no_mangle]
pub extern "C" fn cluster_board_report(t_ms: u32, interval_ms: u32) {
    // SAFETY: only the UI thread touches ACC.
    let acc = unsafe { ACC.get() };
    let p = &acc.interval;
    let fps = p.fps_x10(interval_ms);
    let mut w: Pk<320> = Pk::new();
    let _ = write!(
        w,
        "cluster: t={}s: {} frames, {}.{} fps; ms mean / worst:",
        t_ms / 1000,
        p.presented,
        fps / 10,
        fps % 10
    );
    write_parts(&mut w, p);
    let _ = writeln!(w);
    acc.interval = NOTHING;
}

/// The animation's lines, and the APP and PERF words for its `elapsed_ms`.
/// `stage`, `status` and `shown_by_copy` (board_show_copies() != 0) go into
/// APP's w1. The second line is the rate normalised by the `w` x `h`
/// window (record.rs Throughput, the arithmetic `yakogl-tools record`
/// applies to the same records): fps alone does not compare across panels.
///
/// # Safety
/// `out` is writable.
#[no_mangle]
pub unsafe extern "C" fn cluster_board_finish(
    w: u32,
    h: u32,
    stage: u8,
    status: u32,
    shown_by_copy: bool,
    elapsed_ms: u32,
    out: *mut RunRecords,
) {
    // SAFETY: only the UI thread touches ACC.
    let acc = unsafe { ACC.get() };
    let p = &acc.total;
    let fps = p.fps_x10(elapsed_ms);
    let mut line: Pk<320> = Pk::new();
    let _ = write!(
        line,
        "cluster: animation: {} frames presented in {elapsed_ms} ms, {}.{} fps; ms mean / worst:",
        p.presented,
        fps / 10,
        fps % 10
    );
    write_parts(&mut line, p);
    let _ = writeln!(line);
    // SAFETY: the caller's contract.
    let out = unsafe { &mut *out };
    out.app = p.app_words(stage, status, shown_by_copy, elapsed_ms);
    out.perf = p.perf_words(elapsed_ms);
    if elapsed_ms != 0 {
        let t = record::Throughput { w, h, frames: p.presented, ms: elapsed_ms, draw_tenths: Some(out.perf[1] >> 16) };
        let mut line: Pk<320> = Pk::new();
        let _ = write!(line, "cluster: throughput: ");
        let _ = record::write_throughput(&mut line, &t);
        let _ = writeln!(line);
    }
}

/// " name now/most of total", or " name not measured" for a group main.c
/// could not read (never a 0, which would read as memory not used).
fn write_heap(out: &mut impl Write, name: &str, v: &[u32; 3], missing: bool) {
    if missing {
        let _ = write!(out, " {name} not measured");
    } else {
        let _ = write!(out, " {name} {}/{}/{}", v[0], v[1], v[2]);
    }
}

/// What the run's memory actually is at one stage: main.c reads it at every
/// `stage()` point. The image's statics are resident either way, so this is
/// what the allocators hold inside them — and a run that dies partway still
/// says how far its memory got.
///
/// # Safety
/// `m` is readable.
#[no_mangle]
pub unsafe extern "C" fn cluster_board_mem(m: *const Mem) {
    // SAFETY: the caller's contract.
    let m = unsafe { &*m };
    let mut w: Pk<320> = Pk::new();
    let _ = write!(w, "cluster: mem stage {}: ", m.stage);
    write_heap(&mut w, "ui heap", &m.ui, m.missing & MISSING_UI != 0);
    write_heap(&mut w, "malloc", &m.malloc, m.missing & MISSING_MALLOC != 0);
    write_heap(&mut w, "keep", &m.keep, m.missing & MISSING_KEEP != 0);
    if m.missing & MISSING_GPU != 0 {
        let _ = write!(w, " gpu carve-out not measured");
    } else if m.gpu[2] == 0 {
        // The OSAL has not been initialized yet (its allocator's total reads
        // 0): that is "not set up yet", not "a carve-out of nothing".
        let _ = write!(w, " gpu carve-out not set up yet");
    } else {
        let _ = write!(
            w,
            " gpu carve-out {}/{}/{} ({} held by frees)",
            m.gpu[0], m.gpu[1], m.gpu[2], m.gpu[3]
        );
    }
    if m.window == 0 {
        let _ = write!(w, "; window none yet");
    } else {
        let _ = write!(w, "; window {} bytes", m.window);
    }
    let _ = writeln!(w);
}
