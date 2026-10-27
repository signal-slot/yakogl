// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! draw_steps.rs stops timing once the OSAL has answered -ENOTSUP to a draw
//! stats report (F1 of docs/notes/gl-draw-fixed-cost.md): after that first
//! report a `Timer` makes no clock read and no report. An OSAL that keeps
//! totals is still reported to and its laps counted, and installing another
//! OSAL asks again.
//!
//! The OSAL here is a query hook that counts clock reads and reports.

use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
use yakogl::draw_steps::{OsalDrawStats, Step, Timer};
use yakogl::osal::{osal_set_ops, OsalOps, OSAL_QUERY_CLOCK_NS, OSAL_QUERY_DRAW_STATS};

/// The tests share the process-wide OSAL and the counters below.
static SERIAL: Mutex<()> = Mutex::new(());

static CLOCK_READS: AtomicU32 = AtomicU32::new(0);
static REPORTS: AtomicU32 = AtomicU32::new(0);
static CLOCK: AtomicU64 = AtomicU64::new(0);
static LAST_STEP_NS: AtomicU64 = AtomicU64::new(0);

const ENOTSUP: i32 = 95;

unsafe extern "C" fn q(what: u32, out: *mut c_void, out_sz: usize, keeps: bool) -> i32 {
    if what == OSAL_QUERY_CLOCK_NS && out_sz == 8 {
        CLOCK_READS.fetch_add(1, Ordering::Relaxed);
        let ns = CLOCK.fetch_add(1000, Ordering::Relaxed);
        // SAFETY: a u64 out-parameter of the size given.
        unsafe { *(out as *mut u64) = ns };
        return 0;
    }
    if what == OSAL_QUERY_DRAW_STATS && out_sz == core::mem::size_of::<OsalDrawStats>() {
        REPORTS.fetch_add(1, Ordering::Relaxed);
        if !keeps {
            return -ENOTSUP;
        }
        // SAFETY: the library's report of the size checked above.
        let d = unsafe { *(out as *const OsalDrawStats) };
        LAST_STEP_NS.store(d.ns[Step::GlState as usize], Ordering::Relaxed);
        return 0;
    }
    -ENOTSUP
}

unsafe extern "C" fn q_no_totals(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    // SAFETY: forwarded as given.
    unsafe { q(what, out, out_sz, false) }
}

unsafe extern "C" fn q_totals(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    // SAFETY: forwarded as given.
    unsafe { q(what, out, out_sz, true) }
}

const NONE: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: None,
    gpu_free: None,
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: None,
    cache_invd: None,
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
static NO_TOTALS: OsalOps = OsalOps { query: Some(q_no_totals), ..NONE };
static TOTALS: OsalOps = OsalOps { query: Some(q_totals), ..NONE };

fn install(ops: &'static OsalOps) {
    // SAFETY: serialised by SERIAL; the tables are statics. A draw with
    // no OSAL in between forgets an earlier answer, as another table would.
    unsafe {
        osal_set_ops(core::ptr::null());
        draw(0);
        osal_set_ops(ops);
    }
    CLOCK_READS.store(0, Ordering::Relaxed);
    REPORTS.store(0, Ordering::Relaxed);
}

/// One draw's worth of timer use: a start, `laps` laps, a drop.
fn draw(laps: u32) {
    let mut t = Timer::start();
    for _ in 0..laps {
        t.lap(Step::GlState);
    }
}

fn counts() -> (u32, u32) {
    (CLOCK_READS.load(Ordering::Relaxed), REPORTS.load(Ordering::Relaxed))
}

#[test]
fn after_the_first_enotsup_report_there_are_no_clock_reads_or_reports() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    install(&NO_TOTALS);
    draw(5);
    /* the first draw is timed (start + 5 laps) and its report is refused */
    assert_eq!(counts(), (6, 1));
    for _ in 0..10 {
        draw(5);
    }
    assert_eq!(counts(), (6, 1), "no further clock read or report");
    /* a nested scope is silent too */
    let mut outer = Timer::start();
    draw(3);
    outer.lap(Step::GlProgram);
    drop(outer);
    assert_eq!(counts(), (6, 1));
}

#[test]
fn an_osal_that_keeps_totals_is_timed_and_reported_every_draw() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    install(&TOTALS);
    CLOCK.store(0, Ordering::Relaxed);
    for _ in 0..3 {
        draw(2);
    }
    assert_eq!(counts(), (9, 3), "start + 2 laps and one report, three times");
    /* the clock moves 1000 ns a read: each lap is 1000 ns under its step */
    assert_eq!(LAST_STEP_NS.load(Ordering::Relaxed), 2000);
}

#[test]
fn installing_another_osal_asks_again() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    install(&NO_TOTALS);
    draw(1);
    draw(1);
    assert_eq!(counts(), (2, 1), "off after the first answer");
    install(&TOTALS);
    CLOCK.store(0, Ordering::Relaxed);
    draw(2);
    assert_eq!(counts(), (3, 1), "an OSAL that keeps totals is timed again");
    assert_eq!(LAST_STEP_NS.load(Ordering::Relaxed), 2000);
    /* and back: the earlier table is asked once more, not remembered */
    install(&NO_TOTALS);
    draw(1);
    assert_eq!(counts(), (2, 1));
}
