// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! CPU time spent inside each GL / EGL entry point, for start-up work: how
//! much of an application's first frame is the GL implementation and how
//! much the application (Qt Quick 3D's 7.5 s first frame on the AM62P,
//! 2026-10-22, with one core saturated and the GPU 6 % busy).
//!
//! Every `gl_trace!` (the first statement of every entry point) opens a
//! [`Guard`]; its drop adds the wall time since to that entry point's row
//! and, for the outermost entry point only, to the total. The clock is the
//! AArch64 generic timer (CNTVCT_EL0); elsewhere it reads 0 and nothing is
//! counted. [`yakogl_gl_timing_row`] hands the rows to whoever prints them
//! (the Qt module at its first present).

use core::ffi::{c_char, CStr};
use core::sync::atomic::{AtomicPtr, AtomicU32, AtomicU64, Ordering};

const ROWS: usize = 128;

static NAMES: [AtomicPtr<c_char>; ROWS] = [const { AtomicPtr::new(core::ptr::null_mut()) }; ROWS];
static TICKS: [AtomicU64; ROWS] = [const { AtomicU64::new(0) }; ROWS];
static CALLS: [AtomicU32; ROWS] = [const { AtomicU32::new(0) }; ROWS];
static TOTAL: AtomicU64 = AtomicU64::new(0);
static DEPTH: AtomicU32 = AtomicU32::new(0);

#[inline]
fn now() -> u64 {
    #[cfg(all(target_arch = "aarch64", not(feature = "host-test")))]
    {
        let t: u64;
        // SAFETY: a read of the virtual counter, allowed at EL0 and EL1.
        unsafe { core::arch::asm!("mrs {}, cntvct_el0", out(reg) t, options(nomem, nostack)) };
        t
    }
    #[cfg(not(all(target_arch = "aarch64", not(feature = "host-test"))))]
    {
        0
    }
}

fn freq() -> u64 {
    #[cfg(all(target_arch = "aarch64", not(feature = "host-test")))]
    {
        let f: u64;
        // SAFETY: a read of the counter frequency register.
        unsafe { core::arch::asm!("mrs {}, cntfrq_el0", out(reg) f, options(nomem, nostack)) };
        f
    }
    #[cfg(not(all(target_arch = "aarch64", not(feature = "host-test"))))]
    {
        0
    }
}

/// The row of `name`: its slot in an open-addressed table keyed by the
/// string's address (each entry point's name is one static string).
fn row(name: &'static CStr) -> Option<usize> {
    let p = name.as_ptr() as *mut c_char;
    let mut i = (p as usize >> 3) % ROWS;
    for _ in 0..ROWS {
        let cur = NAMES[i].load(Ordering::Relaxed);
        if cur == p {
            return Some(i);
        }
        if cur.is_null() {
            match NAMES[i].compare_exchange(core::ptr::null_mut(), p, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return Some(i),
                Err(other) if other == p => return Some(i),
                Err(_) => {}
            }
        }
        i = (i + 1) % ROWS;
    }
    None
}

pub struct Guard {
    name: &'static CStr,
    t0: u64,
    outer: bool,
}

impl Guard {
    #[inline]
    pub fn enter(name: &'static CStr) -> Guard {
        let outer = DEPTH.fetch_add(1, Ordering::Relaxed) == 0;
        Guard { name, t0: now(), outer }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        let dt = now().wrapping_sub(self.t0);
        DEPTH.fetch_sub(1, Ordering::Relaxed);
        if dt == 0 && self.t0 == 0 {
            return;
        }
        if let Some(i) = row(self.name) {
            TICKS[i].fetch_add(dt, Ordering::Relaxed);
            CALLS[i].fetch_add(1, Ordering::Relaxed);
        }
        if self.outer {
            TOTAL.fetch_add(dt, Ordering::Relaxed);
        }
    }
}

/// Row `i` (0 .. 128): its entry point's name (null for an empty row), the
/// calls and the microseconds; `i` = -1 gives the total of the outermost
/// entry points (name null). Returns 0, or -1 past the table.
///
/// # Safety
/// The out pointers must be null or valid for a write.
#[no_mangle]
pub unsafe extern "C" fn yakogl_gl_timing_row(i: i32, name: *mut *const c_char, calls: *mut u32, us: *mut u64) -> i32 {
    let f = freq().max(1);
    let (n, c, t) = if i < 0 {
        (core::ptr::null(), 0, TOTAL.load(Ordering::Relaxed))
    } else if (i as usize) < ROWS {
        let k = i as usize;
        (
            NAMES[k].load(Ordering::Relaxed) as *const c_char,
            CALLS[k].load(Ordering::Relaxed),
            TICKS[k].load(Ordering::Relaxed),
        )
    } else {
        return -1;
    };
    // SAFETY: the caller's pointers (contract above).
    unsafe {
        if !name.is_null() {
            *name = n;
        }
        if !calls.is_null() {
            *calls = c;
        }
        if !us.is_null() {
            *us = t.saturating_mul(1_000_000) / f;
        }
    }
    0
}
