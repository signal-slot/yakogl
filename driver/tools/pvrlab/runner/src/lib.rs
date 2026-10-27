// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The gl_test runner: what `driver/tools/pvrlab/gl_test.c`'s `gl_test_run`
//! does around the cases, in Rust. The case bodies stay C and are called one
//! at a time through FFI; this crate owns
//!
//! - which cases run: the regression set ([`SET_ORDER`]), the list a caller
//!   stored (`gl_test_set_list`, [`List`]) or one case, from `gl_case`
//!   ([`select`]); the skip rules of a walk ([`shader_skip_reason`], the
//!   caller's skip list [`listed`]),
//! - the line of each case: `skipped`, `SKIP`, `PASS` / `FAIL` / `TIMEOUT`
//!   ([`skip_line`], [`single_skip_line`], [`verdict_line`]),
//! - the record: the tally ([`Tally`]) and the C globals a board's record
//!   reads from it (`gl_test_passed` ... `gl_test_first_skip`, in `ffi`),
//!   and the `gl_test: all:` line ([`all_line`]).
//!
//! The C side keeps the bring-up, the panel modes, the harness context and
//! the cases; it answers the runner's questions about the build (the shader
//! set it was compiled with, a case's name) and lends it the console
//! (`gl_test_log`). There is one walker: `gl_test_run` calls
//! `gl_test_runner` after its bring-up, in every build that links gl_test.c
//! (the Zephyr images through libyakogl.a's `gl-test-runner` feature, the
//! host harness the same way, panthor-gltest as a crate dependency).
//!
//! The text of every line is the one the C walker printed, byte for byte,
//! because `tools/board.sh`'s recipes parse it; the host tests below pin it.
//! A case moves to Rust later by replacing its `case_*` function and its
//! `case 'n'` arm of the C switch; nothing here changes.
//!
//! What the runner needs of the C side is the [`Suite`] trait, so the rules
//! run on the host against a fake suite; the real one (`ffi::Ffi`) exists only
//! outside the tests.
#![cfg_attr(not(test), no_std)]
#![deny(unsafe_op_in_unsafe_fn)]

use core::fmt::Write;

/// `gl_case` of the regression set.
pub const SET: i32 = 7;
/// `gl_case` of the list `gl_test_set_list` stored: negative because every
/// real case number is not; 7 is the set.
pub const LIST: i32 = -1;
/// The most cases a stored list holds (the rest of a longer one is dropped).
pub const LIST_MAX: usize = 256;

/// The regression set, case 7's walk; the cases that take over the context
/// pool (17, 18) run last. Here once: the C walker is gone, and a harness
/// that runs each case in a process of its own (tools/mali/panthor
/// panthor-gltest) reads it through `gl_test_set_cases` rather than keeping
/// a copy that could drift. tools/cts/check-es100-skips.py reads it from
/// this file.
pub static SET_ORDER: [i32; 50] = [
    1, 2, 3, 4, 5, 6, 8, 9, 10, 22, 11, 34, 12, 23, 13, 40, 14, 24, 58, 59, 60, 61, 15, 16, 19, 20, 41, 42, 43, 46, 47,
    48, 49, 45, 44, 172, 173, 181, 182, 183, 188, 189, 190, 191, 192, 197, 199, 21, 17, 18,
];

/// A case's rc, as `run_case` returns it: 0 passed.
pub const RC_PASS: i32 = 0;
/// The case ran and failed.
pub const RC_FAIL: i32 = -1;
/// No such case (the numbering has gaps); a harness defect in a walk.
pub const RC_NO_CASE: i32 = -2;
/// Skipped by name: by the walk (the shader set has no form of the case, the
/// caller's skip list) or by the case itself ("this build cannot run my
/// subject", e.g. case 16 without a 4x config). Neither a pass nor a failure.
pub const RC_SKIPPED: i32 = -3;
/// A fence wait timed out during the case (PowerVR): its checks are not a
/// verdict. A failure.
pub const RC_TIMEOUT: i32 = -4;

/// The cases a walk takes for a `gl_case`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection<'a> {
    /// The regression set or the stored list, on one context.
    Walk(&'a [i32]),
    /// One case alone.
    One(i32),
}

/// What `gl_case` names: 7 the regression set, [`LIST`] the list the caller
/// stored, anything else one case of that number.
pub fn select(gl_case: i32, list: &List) -> Selection<'_> {
    match gl_case {
        SET => Selection::Walk(&SET_ORDER),
        LIST => Selection::Walk(list.cases()),
        c => Selection::One(c),
    }
}

/// A caller-supplied list of cases to run on one context, in the given order
/// (`gl_test_set_list`).
///
/// It exists because the suite's two modes are not interchangeable and the
/// difference is where a defect hides: `all` gives each case its own process,
/// while the regression set walks its cases on ONE context, so a case that
/// corrupts state only fails in the second. Bisecting such a case from the
/// outside is impossible, so the walk has to be reachable with an arbitrary
/// list. Stored rather than passed so that `gl_test_run` keeps the signature
/// every sample and harness calls it with.
pub struct List {
    cases: [i32; LIST_MAX],
    n: usize,
}

impl List {
    pub const fn new() -> Self {
        Self { cases: [0; LIST_MAX], n: 0 }
    }

    /// Store `cases` (the first [`LIST_MAX`] of them) and return how many
    /// were stored. The `gl_case` that walks them is [`LIST`].
    pub fn set(&mut self, cases: &[i32]) -> usize {
        self.n = cases.len().min(LIST_MAX);
        self.cases[..self.n].copy_from_slice(&cases[..self.n]);
        self.n
    }

    pub fn cases(&self) -> &[i32] {
        &self.cases[..self.n]
    }
}

impl Default for List {
    fn default() -> Self {
        Self::new()
    }
}

/// The shader set gl_test.c was compiled with (`gl_test_shader_set`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShaderSet {
    /// The GLSL ES 1.00 set (`gl_test_shaders_es100.h`, `GL_TEST_ES100_SET`):
    /// what an in-process compiler or a precompiled table serves. Off: the
    /// desktop set, which only a table build (the Mesa reference) links.
    pub es100: bool,
    /// No compiler in the image, the precompiled table the whole back end
    /// (`CONFIG_YAKOGL_SHADERC_PRECOMPILED_ONLY`).
    pub precompiled_only: bool,
}

/// `gl_test_shader_set`'s bits.
pub const SHADER_SET_ES100: u32 = 1;
pub const SHADER_SET_PRECOMPILED_ONLY: u32 = 2;

impl ShaderSet {
    pub const fn from_flags(f: u32) -> Self {
        Self { es100: f & SHADER_SET_ES100 != 0, precompiled_only: f & SHADER_SET_PRECOMPILED_ONLY != 0 }
    }
}

/// The cases whose shaders are their own sources: a table build has no rows
/// for them.
const OWN_SOURCES: [i32; 13] = [181, 182, 183, 184, 185, 188, 189, 190, 191, 192, 197, 199, 209];
/// The cases that write part of a program as text while they run (vsunits,
/// shprobe, fshprobe, churn's fragment shader, varyorder's pair): the
/// precompiled list cannot name it, so an image without the compiler misses
/// the table.
const RUN_TIME_TEXT: [i32; 5] = [90, 149, 150, 166, 209];

/// Why the shader set cannot serve case `c`, or `None`. A shader set that has
/// no form of some case's shaders skips it BY NAME: the ES 1.00 set has no
/// twin for the `#version 300 es` sources, because an ES 2.0 application
/// cannot write one. That is a property of the dialect, not a gap in the
/// compiler, so those cases must never read as a FAIL and must never pass
/// silently either. They are outside an ES 2.0 compiler, but a library that
/// reports ES 3.0 compiles them, so there they run: the set's answer is
/// asked of the current context, and with none it stands.
pub fn shader_skip_reason<S: Suite + ?Sized>(s: &mut S, c: i32) -> Option<&'static str> {
    let set = s.shader_set();

    if !set.es100 {
        // the desktop set is a table build's, and the table has no rows for
        // a case's own sources
        if OWN_SOURCES.contains(&c) {
            return Some("its own sources: a table build has no rows for them");
        }
        // alpha to coverage draws the sample-kill variant's blend variant
        // (the colour mask): the run-time compiler's, no table row
        if c == 198 {
            return Some("alpha to coverage needs the sample-kill variant: a table build has no row for it");
        }
        // an sRGB target's fragment variants are the run-time compiler's
        if c == 204 {
            return Some("an sRGB target needs the sRGB class's variants: a table build has no row for them");
        }
        // one program per source in a table: nothing to lower two ways
        if c == 195 {
            return Some("the if lowering is the run-time compiler's: a table build has one program per source");
        }
    }
    if set.precompiled_only && RUN_TIME_TEXT.contains(&c) {
        return Some("builds its shader text at run time: needs the in-process compiler");
    }
    let why = s.shader_set_lacks(c)?;

    if s.es2_or_no_context() {
        Some(why)
    } else {
        None
    }
}

/// Whether the caller's skip list names case `c`. The list is decimal
/// numbers ("3" or "3,13"); anything that is not a number is skipped over.
///
/// A case that hangs a kick costs every later case the two seconds each hung
/// kick waits, so one unfixed case can stop the set from finishing at all
/// and hide the state of the thirty that follow it. Skipping it is how the
/// rest stay readable while it is being worked on; a skipped case counts as
/// neither a pass nor a failure.
pub fn listed(list: &str, c: i32) -> bool {
    let b = list.as_bytes();
    let mut i = 0;

    while i < b.len() {
        let mut v: i32 = 0;
        let mut digits = false;

        while i < b.len() && (b[i] == b' ' || b[i] == b',') {
            i += 1;
        }
        while i < b.len() && b[i].is_ascii_digit() {
            v = v.wrapping_mul(10).wrapping_add(i32::from(b[i] - b'0'));
            digits = true;
            i += 1;
        }
        if digits && v == c {
            return true;
        }
        if !digits && i < b.len() {
            i += 1; // skip anything that is not a number
        }
    }
    false
}

/// The longest console line the runner prints; a longer one is cut.
pub const LINE_MAX: usize = 256;

/// A fixed buffer for one line (no allocator here).
pub struct Line {
    buf: [u8; LINE_MAX],
    len: usize,
}

impl Line {
    pub const fn new() -> Self {
        Self { buf: [0; LINE_MAX], len: 0 }
    }

    pub fn as_str(&self) -> &str {
        // Only `write_str` fills it, from a `&str`, cut at a byte count: a cut
        // inside a character leaves the valid prefix.
        match core::str::from_utf8(&self.buf[..self.len]) {
            Ok(s) => s,
            Err(e) => core::str::from_utf8(&self.buf[..e.valid_up_to()]).unwrap_or(""),
        }
    }
}

impl Default for Line {
    fn default() -> Self {
        Self::new()
    }
}

impl Write for Line {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let n = s.len().min(self.buf.len() - self.len);

        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

/// What a walk found, for a caller with no console (the i.MX8MP's SNVS
/// record): how many of its cases ran and passed, the first two that failed
/// (and where the first sits in the order), and how many were skipped by name
/// and which first. A skip is neither a pass nor a failure: `total` counts
/// the cases that RAN, so `passed == total` is a clean set even where some
/// cases skip by design, and the skip count keeps a board that skips what it
/// should run visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tally {
    /// Cases that ran: the list's length less the skips.
    pub total: u32,
    pub passed: u32,
    pub skipped: u32,
    pub first_fail: i32,
    pub first_fail_at: i32,
    pub second_fail: i32,
    pub first_skip: i32,
    /// Failures (a timeout is also one).
    pub fails: u32,
    pub timed_out: u32,
}

impl Tally {
    /// The count for a list of `n` cases, before any ran.
    pub const fn begin(n: u32) -> Self {
        Self {
            total: n,
            passed: 0,
            skipped: 0,
            first_fail: -1,
            first_fail_at: -1,
            second_fail: -1,
            first_skip: -1,
            fails: 0,
            timed_out: 0,
        }
    }

    /// Count case `c`, at index `at` of the list, by its rc: 0 passed; -3
    /// skipped by name (by the walk or by the case itself), which is out of
    /// the cases run and into the skips; anything else failed (-4 also timed
    /// out; -2, "no such case", is a harness defect and fails too).
    pub fn case(&mut self, c: i32, at: u32, rc: i32) {
        match rc {
            RC_PASS => self.passed += 1,
            RC_SKIPPED => {
                self.total = self.total.saturating_sub(1);
                self.skipped += 1;
                if self.first_skip < 0 {
                    self.first_skip = c;
                }
            }
            _ => {
                if rc == RC_TIMEOUT {
                    self.timed_out += 1;
                }
                self.fails += 1;
                if self.first_fail < 0 {
                    self.first_fail = c;
                    self.first_fail_at = at as i32;
                } else if self.second_fail < 0 {
                    self.second_fail = c;
                }
            }
        }
    }

    /// Whether the walk found no failure (`gl_test_run`'s rc is then 0).
    pub fn clean(&self) -> bool {
        self.fails == 0
    }
}

/// What the runner needs of the C side: the build's facts, the case bodies,
/// the hooks and the console.
pub trait Suite {
    /// The shader set gl_test.c was compiled with.
    fn shader_set(&mut self) -> ShaderSet;
    /// The generated shader header's reason the set has no form of case
    /// `c`'s shaders (`GL_TEST_SHADERS_SKIP_REASON`), or `None`.
    fn shader_set_lacks(&mut self, c: i32) -> Option<&'static str>;
    /// No current context, or one whose `GL_VERSION` is OpenGL ES 2.x.
    fn es2_or_no_context(&mut self) -> bool;
    /// The caller's skip list (`gl_test_skip`); "" for none.
    fn skip_list(&mut self) -> &str;
    /// The case's name, for its lines.
    fn name(&mut self, c: i32) -> &'static str;
    /// The case body: its rc, and whether it ran to its end (a verdict line
    /// is owed; a case that skipped itself, or does not exist, stepped out
    /// before that and said why itself).
    fn run(&mut self, c: i32) -> (i32, bool);
    /// Tell the harness a case's result (`gl_test_case_result`).
    fn result(&mut self, c: i32, rc: i32);
    /// After a case of a walk: the harness context is current again.
    fn restore_context(&mut self, c: i32);
    /// One console line, `\n` included.
    fn log(&mut self, line: &str);
    /// The shader back end's cost of the run: precompiled hits, compiler runs.
    fn shc_stats(&mut self) -> (u32, u32);
    /// The tally so far, for the C globals a board's record reads; called
    /// when a walk begins and after each of its cases, as the C walker
    /// updated them.
    fn publish(&mut self, t: &Tally);
}

/// `gl_test: case N skipped[: reason]`: a case a walk skipped by name.
pub fn skip_line(c: i32, why: Option<&str>) -> Line {
    let mut l = Line::new();
    let _ = writeln!(l, "gl_test: case {c} skipped{}{}", if why.is_some() { ": " } else { "" }, why.unwrap_or(""));
    l
}

/// `gl_test: case N <name>: SKIP (reason)`: a single case the shader set skips.
pub fn single_skip_line(c: i32, name: &str, why: &str) -> Line {
    let mut l = Line::new();
    let _ = writeln!(l, "gl_test: case {c} {name}: SKIP ({why})");
    l
}

/// `gl_test: case N <name>: PASS|FAIL|TIMEOUT`: rc as the case body returns it.
pub fn verdict_line(c: i32, name: &str, rc: i32) -> Line {
    let verdict = match rc {
        RC_TIMEOUT => "TIMEOUT",
        RC_PASS => "PASS",
        _ => "FAIL",
    };
    let mut l = Line::new();
    let _ = writeln!(l, "gl_test: case {c} {name}: {verdict}");
    l
}

/// The `gl_test: all:` line: what the run cost the shader back end is in it
/// (look-ups the precompiled shaders answered and compiler runs, 0 in an
/// image without the one or the other), so the console proves the tier.
pub fn all_line(t: &Tally, hits: u32, runs: u32) -> Line {
    let mut l = Line::new();
    let _ = writeln!(
        l,
        "gl_test: all: {} of {} cases failed ({} passed; {} timed out; first failure case {} at index {}; \
         second failure case {}; {} skipped, first skip case {}); precompiled hits {}, compiler runs {}",
        t.fails,
        t.total,
        t.passed,
        t.timed_out,
        t.first_fail,
        t.first_fail_at,
        t.second_fail,
        t.skipped,
        t.first_skip,
        hits,
        runs
    );
    l
}

/// The case body and, when it ran to its end, its verdict line; its rc.
fn run_one<S: Suite + ?Sized>(s: &mut S, c: i32) -> i32 {
    let (rc, ended) = s.run(c);

    if ended {
        let name = s.name(c);

        s.log(verdict_line(c, name, rc).as_str());
    }
    rc
}

/// Walk `list` on one context, in order: the regression set or a caller's
/// list. One walk, two callers: they must stay the same code, because the
/// whole point of the list is to reproduce what the set does to a case.
/// Returns the tally; the caller's rc is 0 when it is [`Tally::clean`].
pub fn walk<S: Suite + ?Sized>(s: &mut S, list: &[i32]) -> Tally {
    let mut t = Tally::begin(list.len() as u32);

    s.publish(&t);
    for (i, &c) in list.iter().enumerate() {
        let why = shader_skip_reason(s, c);

        if why.is_some() || listed(s.skip_list(), c) {
            s.log(skip_line(c, why).as_str());
            t.case(c, i as u32, RC_SKIPPED);
            s.publish(&t);
            s.result(c, RC_SKIPPED);
            continue;
        }
        let rc = run_one(s, c);

        s.result(c, rc);
        // a case that skipped itself (-3) ran far enough to touch the
        // context: the context is restored for it too
        t.case(c, i as u32, rc);
        s.publish(&t);
        s.restore_context(c);
    }
    let (hits, runs) = s.shc_stats();

    s.log(all_line(&t, hits, runs).as_str());
    t
}

/// One case alone (no tally, no report to the harness): its rc. The shader
/// set's skip says why here too, in the single case's own form.
pub fn single<S: Suite + ?Sized>(s: &mut S, c: i32) -> i32 {
    if let Some(why) = shader_skip_reason(s, c) {
        let name = s.name(c);

        s.log(single_skip_line(c, name, why).as_str());
        return RC_SKIPPED;
    }
    run_one(s, c)
}

/// The runner: the cases `gl_case` names ([`select`]), run; `gl_test_run`'s
/// rc (0 when a walk found no failure, else -1; a single case's own rc).
pub fn run<S: Suite + ?Sized>(s: &mut S, gl_case: i32, list: &List) -> i32 {
    match select(gl_case, list) {
        Selection::Walk(cases) => {
            if walk(s, cases).clean() {
                0
            } else {
                -1
            }
        }
        Selection::One(c) => single(s, c),
    }
}

/// The C ABI: what gl_test.c and its callers call, and the suite gl_test.c
/// is (`Ffi`). Only outside the tests: the symbols are gl_test.c's.
#[cfg(not(test))]
pub mod ffi {
    use super::{run, List, ShaderSet, Suite, Tally, LINE_MAX, LIST, LIST_MAX, SET_ORDER};
    use core::cell::UnsafeCell;
    use core::ffi::{c_char, c_int, c_uint, CStr};

    unsafe extern "C" {
        /// The case body (gl_test.c `run_case`): its rc, and `*ended` = 1
        /// when it ran to its end and a verdict line is owed.
        fn gl_test_case(c: c_int, pretransform: c_int, debug: c_int, ended: *mut c_int) -> c_int;
        /// The case's name, a string literal; "?" for a number with none.
        fn gl_test_case_name(c: c_int) -> *const c_char;
        /// The shader set the build compiled in: `SHADER_SET_*` bits.
        fn gl_test_shader_set() -> c_uint;
        /// The generated header's reason (a string literal) or NULL.
        fn gl_test_shader_set_lacks(c: c_int) -> *const c_char;
        /// gl_test.c's weak hook; a harness may override it.
        fn gl_test_case_result(c: c_int, rc: c_int);
        /// The harness context current again after a case of a walk.
        fn gl_test_restore_context(c: c_int);
        /// One console line, as gl_test.c's pr_info prints it.
        fn gl_test_log(line: *const c_char);
        fn gl_test_shc_stats(hits: *mut c_uint, runs: *mut c_uint);
        /// The library's; the version the current context reports.
        fn glGetString(name: c_uint) -> *const u8;
    }

    const GL_VERSION: c_uint = 0x1F02;

    /// Cases the caller wants left out of a walk, as a decimal list ("3" or
    /// "3,13"; [`super::listed`]). A C global the i.MX8MP's sample sets
    /// before `gl_test_run` (`extern const char *gl_test_skip;`).
    #[no_mangle]
    pub static mut gl_test_skip: *const c_char = core::ptr::null();

    /// The record of the last walk ([`Tally`]), as the C globals the boards'
    /// records read (`extern unsigned int gl_test_passed, ...;`): the cases
    /// that ran, passed and were skipped, the first two failures (and the
    /// first's index) and the first skip. Written when a walk begins and
    /// after each of its cases.
    #[no_mangle]
    pub static mut gl_test_passed: c_uint = 0;
    #[no_mangle]
    pub static mut gl_test_total: c_uint = 0;
    #[no_mangle]
    pub static mut gl_test_skipped: c_uint = 0;
    #[no_mangle]
    pub static mut gl_test_first_fail: c_int = -1;
    #[no_mangle]
    pub static mut gl_test_first_fail_at: c_int = -1;
    #[no_mangle]
    pub static mut gl_test_second_fail: c_int = -1;
    #[no_mangle]
    pub static mut gl_test_first_skip: c_int = -1;

    /// The list `gl_test_set_list` stored. One per image, like the C static
    /// it replaces; the suite runs on one thread (`gl_test_run`'s contract),
    /// and no case stores a list.
    struct Stored(UnsafeCell<List>);

    // SAFETY: read and written on the suite's thread only (above).
    unsafe impl Sync for Stored {}

    static STORED: Stored = Stored(UnsafeCell::new(List::new()));

    /// Store the list and return the `gl_case` value that walks it, writing
    /// the number actually stored to `*stored` (the list is bounded, [`LIST_MAX`]).
    /// Returning the sentinel rather than exporting it keeps the caller from
    /// spelling either constant itself, so the two cannot drift apart.
    ///
    /// # Safety
    ///
    /// `cases` points at `n` ints (or `n` is 0); `stored` is NULL or a valid
    /// out-pointer; on the suite's thread, not during a run.
    #[no_mangle]
    pub unsafe extern "C" fn gl_test_set_list(cases: *const c_int, n: c_uint, stored: *mut c_uint) -> c_int {
        let n = if cases.is_null() { 0 } else { (n as usize).min(LIST_MAX) };
        // SAFETY: `n` ints at `cases`, by the caller; the stored list is
        // this thread's (Stored).
        let kept = unsafe { (*STORED.0.get()).set(core::slice::from_raw_parts(cases, n)) };

        if !stored.is_null() {
            // SAFETY: a valid out-pointer, by the caller.
            unsafe { *stored = kept as c_uint };
        }
        LIST
    }

    /// The regression set, for a harness that runs each case in a process
    /// of its own: `*cases` points at [`SET_ORDER`], the count is returned.
    ///
    /// # Safety
    ///
    /// `cases` is a valid out-pointer.
    #[no_mangle]
    pub unsafe extern "C" fn gl_test_set_cases(cases: *mut *const c_int) -> c_uint {
        // SAFETY: a valid out-pointer, by the caller; SET_ORDER is static.
        unsafe { *cases = SET_ORDER.as_ptr() };
        SET_ORDER.len() as c_uint
    }

    /// The runner's entry, called by `gl_test_run` after its bring-up and
    /// the harness context: the cases `gl_case` names (7 the regression
    /// set, what `gl_test_set_list` returned the stored list, else one
    /// case), run; `gl_test_run`'s rc.
    ///
    /// # Safety
    ///
    /// On the thread that owns the GPU, after `gl_test_run`'s bring-up, with
    /// the harness context current, as the C walker ran.
    #[no_mangle]
    pub unsafe extern "C" fn gl_test_runner(gl_case: c_int, pretransform: c_int, debug: c_int) -> c_int {
        let mut s = Ffi { pretransform, debug };
        // SAFETY: the stored list is this thread's and is not written during
        // a run (Stored).
        let list = unsafe { &*STORED.0.get() };

        run(&mut s, gl_case, list)
    }

    /// A string literal of gl_test.c (or NULL): `&'static str`, "" if not
    /// UTF-8.
    ///
    /// # Safety
    ///
    /// `p` is NULL or a NUL-terminated string that lives as long as the image.
    unsafe fn literal(p: *const c_char) -> Option<&'static str> {
        if p.is_null() {
            return None;
        }
        // SAFETY: by the caller.
        Some(unsafe { CStr::from_ptr(p) }.to_str().unwrap_or(""))
    }

    /// The real suite: gl_test.c, linked into the image.
    pub struct Ffi {
        pub pretransform: c_int,
        pub debug: c_int,
    }

    impl Suite for Ffi {
        fn shader_set(&mut self) -> ShaderSet {
            // SAFETY: a plain function of gl_test.c.
            ShaderSet::from_flags(unsafe { gl_test_shader_set() })
        }

        fn shader_set_lacks(&mut self, c: i32) -> Option<&'static str> {
            // SAFETY: the reason is NULL or a string literal of the
            // generated header.
            unsafe { literal(gl_test_shader_set_lacks(c)) }
        }

        fn es2_or_no_context(&mut self) -> bool {
            // SAFETY: the library's glGetString, on the suite's thread; it
            // returns NULL with no current context, else a static string.
            let v = unsafe { glGetString(GL_VERSION) };

            if v.is_null() {
                return true;
            }
            // SAFETY: a NUL-terminated string the library owns.
            unsafe { CStr::from_ptr(v.cast()) }.to_bytes().starts_with(b"OpenGL ES 2.")
        }

        fn skip_list(&mut self) -> &str {
            // SAFETY: the sample's string (a Kconfig literal) that it set
            // before gl_test_run, or NULL; read on the suite's thread.
            unsafe { literal(gl_test_skip) }.unwrap_or("")
        }

        fn name(&mut self, c: i32) -> &'static str {
            // SAFETY: a string literal of gl_test.c, never NULL.
            unsafe { literal(gl_test_case_name(c)) }.unwrap_or("?")
        }

        fn run(&mut self, c: i32) -> (i32, bool) {
            let mut ended: c_int = 0;
            // SAFETY: the case, on the thread that owns the harness context;
            // a valid out-pointer.
            let rc = unsafe { gl_test_case(c, self.pretransform, self.debug, &mut ended) };

            (rc, ended != 0)
        }

        fn result(&mut self, c: i32, rc: i32) {
            // SAFETY: gl_test.c's hook (weak; a harness may override it).
            unsafe { gl_test_case_result(c, rc) };
        }

        fn restore_context(&mut self, c: i32) {
            // SAFETY: on the thread that owns the harness context.
            unsafe { gl_test_restore_context(c) };
        }

        fn log(&mut self, line: &str) {
            // The C side takes a NUL-terminated string: the line is copied
            // into a buffer that has room for the NUL.
            let mut buf = [0u8; LINE_MAX + 1];
            let n = line.len().min(LINE_MAX);

            buf[..n].copy_from_slice(&line.as_bytes()[..n]);
            // SAFETY: `buf` is NUL-terminated (its last byte is 0).
            unsafe { gl_test_log(buf.as_ptr().cast()) };
        }

        fn shc_stats(&mut self) -> (u32, u32) {
            let (mut hits, mut runs) = (0, 0);

            // SAFETY: two valid out-pointers.
            unsafe { gl_test_shc_stats(&mut hits, &mut runs) };
            (hits, runs)
        }

        fn publish(&mut self, t: &Tally) {
            // SAFETY: plain stores into this crate's exported counters, on
            // the suite's thread; the C side reads them after gl_test_run.
            unsafe {
                gl_test_total = t.total;
                gl_test_passed = t.passed;
                gl_test_skipped = t.skipped;
                gl_test_first_fail = t.first_fail;
                gl_test_first_fail_at = t.first_fail_at;
                gl_test_second_fail = t.second_fail;
                gl_test_first_skip = t.first_skip;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::string::String;
    use std::vec::Vec;

    /// A suite whose cases are a table of rcs.
    #[derive(Default)]
    struct Fake {
        set: ShaderSet,
        /// The generated header's list: (case, reason).
        lacks: Vec<(i32, &'static str)>,
        es3: bool,
        skip_list: String,
        /// (case, rc) the body returns; absent: 0.
        rcs: Vec<(i32, i32)>,
        /// Cases whose body steps out early (no verdict line).
        early: Vec<i32>,
        log: String,
        order: Vec<String>,
        published: Vec<Tally>,
    }

    impl Suite for Fake {
        fn shader_set(&mut self) -> ShaderSet {
            self.set
        }
        fn shader_set_lacks(&mut self, c: i32) -> Option<&'static str> {
            self.lacks.iter().find(|(k, _)| *k == c).map(|(_, r)| *r)
        }
        fn es2_or_no_context(&mut self) -> bool {
            !self.es3
        }
        fn skip_list(&mut self) -> &str {
            &self.skip_list
        }
        fn name(&mut self, c: i32) -> &'static str {
            match c {
                1 => "uniform",
                16 => "msaa",
                44 => "es3hdr",
                _ => "?",
            }
        }
        fn run(&mut self, c: i32) -> (i32, bool) {
            self.order.push(std::format!("run {c}"));
            let rc = self.rcs.iter().find(|(k, _)| *k == c).map_or(0, |(_, r)| *r);

            (rc, !self.early.contains(&c))
        }
        fn result(&mut self, c: i32, rc: i32) {
            self.order.push(std::format!("result {c} {rc}"));
        }
        fn restore_context(&mut self, c: i32) {
            self.order.push(std::format!("restore {c}"));
        }
        fn log(&mut self, line: &str) {
            self.log.push_str(line);
        }
        fn shc_stats(&mut self) -> (u32, u32) {
            (7, 2)
        }
        fn publish(&mut self, t: &Tally) {
            self.published.push(*t);
        }
    }

    fn es100() -> ShaderSet {
        ShaderSet { es100: true, precompiled_only: false }
    }

    // ---- selection --------------------------------------------------------

    #[test]
    fn seven_is_the_set_the_sentinel_the_stored_list_and_a_number_one_case() {
        let mut list = List::new();

        assert_eq!(select(7, &list), Selection::Walk(&SET_ORDER[..]));
        assert_eq!(select(LIST, &list), Selection::Walk(&[]));
        assert_eq!(select(16, &list), Selection::One(16));
        assert_eq!(list.set(&[17, 19]), 2);
        assert_eq!(select(LIST, &list), Selection::Walk(&[17, 19]));
    }

    #[test]
    fn the_set_has_fifty_cases_and_the_pool_takers_run_last() {
        assert_eq!(SET_ORDER.len(), 50);
        assert_eq!(&SET_ORDER[47..], &[21, 17, 18]);
        assert!(!SET_ORDER.contains(&7) && !SET_ORDER.contains(&LIST));
    }

    #[test]
    fn a_long_list_is_cut_at_the_bound_and_says_how_many_it_kept() {
        let long: Vec<i32> = (0..300).collect();
        let mut list = List::new();

        assert_eq!(list.set(&long), LIST_MAX);
        assert_eq!(list.cases(), &long[..LIST_MAX]);
        assert_eq!(list.set(&[]), 0);
        assert!(list.cases().is_empty());
    }

    // ---- the skip rules ---------------------------------------------------

    #[test]
    fn the_callers_skip_list_is_decimal_numbers_with_junk_skipped_over() {
        assert!(listed("3", 3));
        assert!(listed("3,13", 13));
        assert!(listed(" 3, 13 ", 3));
        assert!(!listed("3,13", 1));
        assert!(!listed("", 3));
        assert!(listed("x3", 3));
        assert!(!listed("31", 3));
    }

    #[test]
    fn a_table_build_skips_the_cases_whose_shaders_it_has_no_rows_for() {
        let mut s = Fake::default(); // the desktop set

        assert_eq!(shader_skip_reason(&mut s, 181), Some("its own sources: a table build has no rows for them"));
        assert!(shader_skip_reason(&mut s, 198).unwrap().starts_with("alpha to coverage"));
        assert!(shader_skip_reason(&mut s, 204).unwrap().starts_with("an sRGB target"));
        assert!(shader_skip_reason(&mut s, 195).unwrap().starts_with("the if lowering"));
        assert_eq!(shader_skip_reason(&mut s, 1), None);
        let mut s = Fake { set: es100(), ..Fake::default() };

        assert_eq!(shader_skip_reason(&mut s, 181), None);
    }

    #[test]
    fn an_image_without_a_compiler_skips_the_cases_that_write_shader_text() {
        let set = ShaderSet { es100: true, precompiled_only: true };
        let mut s = Fake { set, ..Fake::default() };

        assert_eq!(
            shader_skip_reason(&mut s, 149),
            Some("builds its shader text at run time: needs the in-process compiler")
        );
        assert_eq!(shader_skip_reason(&mut s, 1), None);
        let mut s = Fake { set: es100(), ..Fake::default() };

        assert_eq!(shader_skip_reason(&mut s, 149), None);
    }

    #[test]
    fn the_es100_sets_skip_stands_on_an_es2_context_and_not_on_an_es3_one() {
        let lacks = std::vec![(41, "ES 3.00 source, outside the ES 2.0 compiler")];
        let mut s = Fake { set: es100(), lacks: lacks.clone(), ..Fake::default() };

        assert_eq!(shader_skip_reason(&mut s, 41), Some("ES 3.00 source, outside the ES 2.0 compiler"));
        assert_eq!(shader_skip_reason(&mut s, 42), None);
        let mut s = Fake { set: es100(), lacks, es3: true, ..Fake::default() };

        assert_eq!(shader_skip_reason(&mut s, 41), None);
    }

    // ---- the walk and its lines -------------------------------------------

    #[test]
    fn a_clean_walk_prints_the_c_walkers_lines() {
        let mut s = Fake { set: es100(), ..Fake::default() };
        let t = walk(&mut s, &[1, 16]);

        assert!(t.clean());
        assert_eq!(
            s.log,
            "gl_test: case 1 uniform: PASS\ngl_test: case 16 msaa: PASS\n\
             gl_test: all: 0 of 2 cases failed (2 passed; 0 timed out; first failure case -1 at index -1; \
             second failure case -1; 0 skipped, first skip case -1); precompiled hits 7, compiler runs 2\n"
        );
    }

    #[test]
    fn a_case_runs_reports_counts_and_restores_the_context_in_that_order() {
        let mut s = Fake { set: es100(), ..Fake::default() };

        walk(&mut s, &[5]);
        assert_eq!(s.order, ["run 5", "result 5 0", "restore 5"]);
        // the record before the case and after it
        assert_eq!(s.published.len(), 2);
        assert_eq!((s.published[0].total, s.published[0].passed), (1, 0));
        assert_eq!((s.published[1].total, s.published[1].passed), (1, 1));
    }

    #[test]
    fn a_walk_skips_by_name_before_the_body_and_says_why_when_it_knows() {
        let lacks = std::vec![(44, "ES 3.00 source, outside the ES 2.0 compiler")];
        let mut s = Fake { set: es100(), lacks, skip_list: "13".into(), ..Fake::default() };
        let t = walk(&mut s, &[44, 13, 1]);

        assert!(t.clean());
        assert_eq!((t.total, t.passed, t.skipped, t.first_skip), (1, 1, 2, 44));
        assert!(s.log.starts_with(
            "gl_test: case 44 skipped: ES 3.00 source, outside the ES 2.0 compiler\n\
             gl_test: case 13 skipped\n\
             gl_test: case 1 uniform: PASS\n\
             gl_test: all: 0 of 1 cases failed (1 passed;"
        ));
        assert_eq!(s.order, ["result 44 -3", "result 13 -3", "run 1", "result 1 0", "restore 1"]);
    }

    #[test]
    fn the_imx8mp_default_sets_shape_counts_as_the_c_walker_did() {
        // 49 listed, case 16 skips itself, one other case (44) fails
        // (tests/gltest_set_tally.c's first scene, which this replaces)
        let list: Vec<i32> = (0..49)
            .map(|i| {
                if i == 23 {
                    16
                } else if i == 34 {
                    44
                } else {
                    i + 100
                }
            })
            .collect();
        let rcs = std::vec![(16, RC_SKIPPED), (44, RC_FAIL)];
        let mut s = Fake { set: es100(), rcs, early: std::vec![16], ..Fake::default() };
        let t = walk(&mut s, &list);

        assert_eq!((t.passed, t.total, t.skipped, t.first_skip), (47, 48, 1, 16));
        assert_eq!((t.fails, t.first_fail, t.first_fail_at, t.second_fail), (1, 44, 34, -1));
        assert!(!s.log.contains("case 16 msaa"), "a case that skipped itself printed its own reason");
        assert!(s.log.contains("gl_test: case 44 es3hdr: FAIL\n"));
        assert!(s.log.ends_with(
            "gl_test: all: 1 of 48 cases failed (47 passed; 0 timed out; first failure case 44 at index 34; \
             second failure case -1; 1 skipped, first skip case 16); precompiled hits 7, compiler runs 2\n"
        ));
        // the case that skipped itself ran far enough to touch the context
        assert!(s.order.contains(&"restore 16".into()));
    }

    #[test]
    fn a_clean_set_with_a_skip_has_passed_equal_to_run() {
        let mut s = Fake { set: es100(), rcs: std::vec![(16, RC_SKIPPED)], ..Fake::default() };
        let t = walk(&mut s, &[1, 16, 2]);

        assert_eq!(t.passed, t.total);
        assert!(t.clean());
    }

    #[test]
    fn a_timeout_a_fail_and_no_such_case_all_fail_the_first_two_named() {
        let rcs = std::vec![(10, RC_TIMEOUT), (2, RC_FAIL), (99, RC_NO_CASE), (17, RC_SKIPPED), (18, RC_SKIPPED)];
        let mut s = Fake { set: es100(), rcs, early: std::vec![99], ..Fake::default() };
        let t = walk(&mut s, &[10, 2, 99, 17, 18]);

        assert!(!t.clean());
        assert_eq!((t.fails, t.timed_out, t.first_fail, t.second_fail), (3, 1, 10, 2));
        assert_eq!((t.total, t.skipped, t.first_skip, t.passed), (3, 2, 17, 0));
        assert!(s.log.contains("gl_test: case 10 ?: TIMEOUT\n"));
        assert!(s.log.contains("gl_test: case 2 ?: FAIL\n"));
        assert!(!s.log.contains("case 99"), "no such case has no verdict line");
    }

    #[test]
    fn an_empty_list_is_clean() {
        let mut s = Fake { set: es100(), ..Fake::default() };

        assert!(walk(&mut s, &[]).clean());
        assert!(s.log.starts_with("gl_test: all: 0 of 0 cases failed (0 passed;"));
    }

    #[test]
    fn the_runner_returns_gl_test_runs_rc() {
        let mut list = List::new();

        list.set(&[1, 2]);
        let mut s = Fake { set: es100(), ..Fake::default() };

        assert_eq!(run(&mut s, LIST, &list), 0);
        let mut s = Fake { set: es100(), rcs: std::vec![(2, RC_FAIL)], ..Fake::default() };

        assert_eq!(run(&mut s, LIST, &list), -1);
        let mut s = Fake { set: es100(), rcs: std::vec![(2, RC_FAIL)], ..Fake::default() };

        assert_eq!(run(&mut s, 2, &list), RC_FAIL);
    }

    // ---- a single case ----------------------------------------------------

    #[test]
    fn a_single_case_prints_its_verdict_and_reports_nothing() {
        let mut s = Fake { set: es100(), rcs: std::vec![(1, RC_FAIL)], ..Fake::default() };

        assert_eq!(single(&mut s, 1), RC_FAIL);
        assert_eq!(s.log, "gl_test: case 1 uniform: FAIL\n");
        assert_eq!(s.order, ["run 1"]);
        assert!(s.published.is_empty());
    }

    #[test]
    fn a_single_case_the_shader_set_skips_says_why_in_its_own_form() {
        let lacks = std::vec![(44, "ES 3.00 source, outside the ES 2.0 compiler")];
        let mut s = Fake { set: es100(), lacks, skip_list: "1".into(), ..Fake::default() };

        assert_eq!(single(&mut s, 44), RC_SKIPPED);
        assert_eq!(s.log, "gl_test: case 44 es3hdr: SKIP (ES 3.00 source, outside the ES 2.0 compiler)\n");
        assert!(s.order.is_empty());
        // the caller's skip list is the walk's, not a single case's
        assert_eq!(single(&mut s, 1), RC_PASS);
    }

    #[test]
    fn a_long_line_is_cut_on_a_character_boundary() {
        let why = "é".repeat(LINE_MAX);
        let l = skip_line(1, Some(&why));

        assert!(l.as_str().len() <= LINE_MAX);
        assert!(l.as_str().starts_with("gl_test: case 1 skipped: é"));
    }
}
