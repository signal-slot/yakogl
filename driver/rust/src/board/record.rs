// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The result record: the one definition (driver/docs/one-method.md,
//! section 3). The board layer's writer (driver/zephyr/lib/board/, through
//! the `yd_record_*` entry points of `ffi::board`), the generated C header
//! `driver/zephyr/lib/board/include/yakogl_record.h` and the host decoder all
//! come from this file, so a writer and a reader cannot drift.
//!
//! This file uses `core` only and names nothing else of its crate, so a host
//! tool compiles the same file with `#[path = ".../record.rs"] mod record;`
//! instead of keeping a second copy.
//!
//! # Format
//!
//! Four 32-bit words. `w0 = 0x594B0000 | kind << 8 | low` ('YK'), then
//! `w1..w3`, which the kind defines (the [`KINDS`] table's `doc`). `low` is a
//! sequence number, except for the kinds whose [`Low`] is `Tag`: the decoder
//! needs that list, so it is part of the format. A store writes `w1..w3`
//! first and `w0` last, so a reader that finds the magic can trust the rest.
//!
//! Every board prints each record it stores as one console line,
//! [`LINE_PREFIX`] and the four words as 8 lower-case hex digits, one space
//! apart ([`write_line`], [`parse_line`]):
//!
//! ```text
//! yk-record: 594b040e ff403f80 35706c4e 04000258
//! ```
//!
//! The last line of a run is its verdict. The i.MX8MP also keeps the last
//! record in the SNVS LPGPR0..3 (it survives the SoC reset), where the
//! runner may leave a selector before a run ([`SELECTOR_MAGIC`]).
//!
//! # Breadcrumbs
//!
//! PROGRESS, WDOG tag 1 (armed) and STAGE records are breadcrumbs: dropped
//! once a result record was stored and replaced by any later record; the
//! tagged ones do not move the sequence. A run whose last record is a
//! breadcrumb died after that point ([`classify`]). The rules are
//! [`Writer`]'s.

use core::fmt;

/// 'YK' in w0's high half.
pub const MAGIC: u32 = 0x594B_0000;
/// The selector the runner leaves in a cleared i.MX8MP record before a run:
/// w0 = 0, w1 = `SELECTOR_MAGIC | index` ('SE').
pub const SELECTOR_MAGIC: u32 = 0x5345_0000;
/// The console line's prefix; the four words follow, each `%08x`, one space
/// apart.
pub const LINE_PREFIX: &str = "yk-record: ";

/// What w0's low byte holds for a kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Low {
    /// The writer's sequence number (one per stored result or PROGRESS).
    Seq,
    /// A value of the kind's own (a status, a tag, a count); the sequence
    /// number is not touched.
    Tag,
}

/// One row of [`KINDS`].
#[derive(Clone, Copy, Debug)]
pub struct Kind {
    pub value: u8,
    /// The C name's suffix: `YK_RECORD_KIND_<name>`.
    pub name: &'static str,
    pub low: Low,
    /// What w1..w3 (and a tag) hold.
    pub doc: &'static str,
}

macro_rules! kinds {
    ($($name:ident = $v:literal, $low:ident, $doc:literal;)*) => {
        /// The kind numbers, as constants.
        pub mod kind {
            $(pub const $name: u8 = $v;)*
        }
        /// Every kind, in number order.
        pub const KINDS: &[Kind] = &[
            $(Kind { value: $v, name: stringify!($name), low: Low::$low, doc: $doc },)*
        ];
    };
}

kinds! {
    CASE = 1, Seq,
        "one gl_test case. w1 = case << 16 | first bad check (255 none) << 8 | status \
         (CASE_*); w2 = got, w3 = want (0xAABBGGRR as gl_test reads them back); on a pass \
         w2 = checks made, w3 = 0. CASE_SET (the regression set or a list, its summary): \
         w1 = the first failing case (0xffff none) << 16 | cases skipped (saturating at \
         255) << 8 | CASE_SET; w2 = the second failing case (0xffff none) << 16 | cases \
         passed; w3 = the first skipped case (0xffff none) << 16 | cases run (skips not \
         counted); a pass when passed == run. CASE_SET_FAIL (and a CASE_PASS of case 7) \
         is the form earlier images wrote: case = the first failure, w2 = cases passed, \
         w3 = cases run, a by-name skip counted as a failure";
    APP = 2, Seq,
        "an application run. w1 = flags (APP_*) | stage << 8 | status (1 pass, 2 fail); \
         w2 = frames presented; w3 = the application's (board_finish: the GPU resets; \
         the cluster sample: the milliseconds the frames were presented in). Flag \
         APP_SHOWN_BY_COPY: board_show copied the window to the screen at least once \
         (a window the display cannot scan in place)";
    FAULT = 3, Tag,
        "the fatal handler ran. w1 = ESR, w2 = ELR, w3 = FAR (low 32 bits each); the tag \
         says how far the application got (dEQP: cases begun; gc_board: its sequence)";
    PICTURE = 4, Seq,
        "a rendered picture. w1 = the pixel at (W/2, H/2), GL rows bottom-up, the \
         glReadPixels RGBA bytes as a little-endian word; w2 = CRC-32/IEEE over those four \
         bytes of the samples at x = W/2, y = i * H / 24, i = 3..21 ascending; \
         w3 = W << 16 | H. CRCs compare only when W and H agree";
    PROGRESS = 5, Seq,
        "a breadcrumb: how far a run got. w1 = where (WHERE_*), w2 = uptime in ms, \
         w3 = PROGRESS records written this run";
    PROBES = 6, Tag,
        "five probe points; tag = status (1 pass, 2 fail). w1 = p0 | p1 << 16, \
         w2 = p2 | p3 << 16, w3 = p4 | crc << 16; each point RGB565, crc = CRC-16/CCITT-FALSE \
         over the 15 bytes R,G,B of p0..p4 at 8 bits";
    SWEEP = 7, Tag,
        "48 gl_test cases, two bits each (0 not reached, 1 pass, 2 fail, 3 skipped), case \
         first + i at bits 2*(i % 16) of word 1 + i / 16; tag = cases finished. Tag \
         SWEEP_TAG_LEAKS: the counters, w1 = lost work << 16 | GPU resets << 8 | fence \
         timeouts, w2 = pass bits of first+32..47 << 16 | first leaker offset (0xff none) \
         << 8 | leakers, w3 = pass bits of first..first+31";
    DEQP = 8, Seq,
        "a dEQP run's tally. w1 = passed | failed << 16; w2 = not supported | other << 16; \
         w3 = cases begun | the first failing case's name check << 16 (0xffff none)";
    DEQP_CASES = 9, Tag,
        "a dEQP run of at most 32 cases; tag = cases begun. w1..w2 = two bits per case in \
         run order (0 none, 1 Pass, 2 Fail, 3 other); w3 = first fail's name check << 16 | \
         GPU resets (4 bits) << 12 | fence timeouts (4 bits) << 8 | its index (0xff none)";
    DEQP_FAILS = 10, Tag,
        "six failing cases' name checks, a page at a time; tag = Fails seen (saturating). \
         w1..w3 = two CRC-16 name checks each, the earlier in the low half, 0xffff none";
    DEQP_STATUS = 11, Tag,
        "the first unusual dEQP status; tag = that case's index. w1..w3 = the first 12 \
         bytes of the status string, little-endian, zero-padded";
    PERF = 12, Tag,
        "a page of an application's frame-time breakdown; tag = the selector's low byte \
         (experiment << 3 | page). w1..w3 = six 16-bit values, the first in w1's low half";
    WDOG = 13, Tag,
        "the i.MX8MP WDOG1 self-arm; tag = WDOG_* (ARMED is a breadcrumb). w1 = WCR as \
         found | WCR read back << 16; w2 = flags | refusal << 8 | WMCR as found << 16; \
         w3 = uptime in ms when armed (tags 1..3) or at the backstop (tag 4)";
    STAGE = 14, Tag,
        "a breadcrumb: a boot or sample stage began; tag = the stage (STAGE_*). \
         w1 = the stage's value (a return code as int32), w2 = uptime in ms, w3 = the \
         WDOG1 self-arm's feeds";
    FINGERPRINT = 15, Seq,
        "the whole window's checksum, not a verdict. w1 = CRC-32/IEEE of the window, \
         rows top-down, 4 bytes per pixel as in memory; w2 = frames drawn; w3 = W << 16 | H";
    // 16 = SET and 17 = PX1 are reserved for imx8mp/gltest-repro-k16 and vivante/gc-ab-knobs.
    GLMARK2 = 18, Seq,
        "a glmark2-es2 run, the run's verdict. Flags = w2 >> 28 (GLMARK2_OFFSCREEN, \
         GLMARK2_SCENES); status = w2 bits 24..27, 1 pass (no scene failed, the score is not 0), \
         2 fail; unsupported scenes are their own count, in neither; w3 = W << 16 | H. With \
         GLMARK2_SCENES (glmark2_words): w1 = score (16 bits, saturating) | the first failed \
         entry << 16 | why it failed (GLMARK2_WHY_*) << 22 | the first unsupported entry << \
         26; w2 = flags << 28 | status << 24 | ran << 18 | failed << 12 | unsupported << 6 | \
         the second unsupported entry; counts saturate at 63, an entry is the index in the \
         benchmark list (GLMARK2_ENTRY_NONE none). With GLMARK2_GL as well (2026-10-12, \
         issue #13): w3 = W (12 bits) | H << 12 (12 bits) | the GL errors the entry points \
         raised (yakogl_gl_errors_raised) << 24 | the draws that sampled an incomplete \
         texture (yakogl_incomplete_texture_draws) << 28, 4 bits each, saturating at 15 \
         (15 or more); without GLMARK2_GL the counters were not recorded. Without \
         GLMARK2_SCENES (before 2026-10-04): w1 = score; w2 = flags << 28 | status << 24 | \
         ran << 16 | failed << 8 | unsupported, 8 bits each";
    // 19 = KICK_DIFF is reserved for vivante/gc-ab-knobs.
    GPU_STOP = 20, Tag,
        "the GPU driver ended the run: a kick it cannot complete (the PowerVR render \
         service's stop, gpu::pvr::freelist::stop); a failure. tag = the cause (GPU_STOP_*); \
         w1 = the fence a wait found it on (0: a submit did); w2 = GPU recoveries so far; \
         w3 = freelist ID << 24 | KiB the cause names (saturating at 24 bits)";
    CHECK = 21, Tag,
        "why an application's checked frame or run failed; a failure, written after \
         PROBES as the run's verdict. tag = what failed first (CHECK_*). CHECK_PROBES: \
         w1 = the first failing probe's index << 24 | x << 12 | y (window pixels, y down), \
         w2 = its R << 16 | G << 8 | B in memory | probes failing << 24 (saturating), \
         w3 = the reference's R << 16 | G << 8 | B | that probe's largest channel \
         difference << 24. CHECK_UNDRAWN, _ALPHA, _GUARD, _READPIXELS: w1 = how many pixels \
         (or probes, for _READPIXELS), w2 = w3 = 0. CHECK_RUN: the check passed and the run \
         failed later; w1 = the run's return code as int8 | the cause (CHECK_RUN_*) << 8 | \
         draws the GL core refused (yakogl_draws_refused) << 12 | compiler runs in the \
         failing frame << 20 (6 bits) | its slowest step (CHECK_RUN_STEP_*) << 26 \
         (4 bits) | its iteration's GPU waits that timed out << 30 (2 bits), w2 = the animation frame (1 = the first after the check; 0 none; 8 \
         bits) | that step's ms in the frame << 8 (12 bits) | the GPU's busy ms over its \
         iteration << 20 (12 bits), w3 = the online compiler's keep high-water in KiB (11 \
         bits) | the cause's detail in ms << 11 (13 bits: how long the wait or the frame \
         took) | the time its iteration was blocked on the GPU (fence waits and submits \
         waiting for an event), 16 ms units, << 24; every field \
         saturates one below its largest value, which means not measured";
    DISPLAY = 22, Seq,
        "the i.MX8MP display chain as it scans the window (gc_gl_triangle, \
         CONFIG_GC_GL_TRIANGLE_DISPLAY_RECORD), written after PICTURE once the window has \
         been scanned about a second, so it replaces PICTURE as the run's record. w1 = the \
         pixel clock in kHz (the CCM root as read back when adopted) | DISPLAY_ADOPTED; \
         w2 = the DSI lane rate in kHz (the DSIM PLL as programmed; 0 unknown); w3 = the \
         SN65DSI84's IRQ_STAT (0xE5) now, latched since the link check (0xff not read) | \
         IRQ_STAT one ms after the video started << 8 (0xff the check failed) | the \
         display health (IMX8MP_DISPLAY_HEALTH_*) << 16 | the LCDIF's CTRLDESCL0_5 BPP code \
         << 24 | its pitch in KiB << 28 (saturating at 15)";
    GL_ERROR = 23, Tag,
        "one of the first GL errors a run raised, by name (issue #814); not a verdict of its \
         own: written after the run's verdict when the runner's selector asks for it \
         (GL_ERROR_SEL | k: driver/samples/zephyr/glmark2), so a runner that keeps only the \
         last record (the i.MX8MP) reads it. tag = k, the error's index (0 the first). w1 = \
         the GL error code (16 bits) | the client's tag when it was raised (glmark2: the \
         benchmark-list entry; GL_ERROR_TAG_NONE none or before the first) << \
         GL_ERROR_TAG_SHIFT (8 bits) | how many errors the GL core kept (at most 8) << \
         GL_ERROR_KEPT_SHIFT; w2 = the FNV-1a (32 bits, fnv1a32) of the name of the entry \
         point that raised it, 0 when k is not below the kept count (code 0, tag none); \
         w3 = every GL error the entry points raised (yakogl_gl_errors_raised, 32 bits)";
}

/// Kind numbers taken on another branch and not defined here yet (16 SET,
/// 17 PX1, 19 KICK_DIFF: imx8mp/gltest-repro-k16, vivante/gc-ab-knobs). The
/// test that the kinds are 1..=N leaves these out; the merge that brings
/// them in empties this list.
pub const RESERVED_KINDS: &[u8] = &[16, 17, 19];

/// The row of a kind number, if it is one.
pub fn kind_info(value: u8) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.value == value)
}

/// One row of [`STAGES`].
#[derive(Clone, Copy, Debug)]
pub struct Stage {
    pub value: u8,
    /// The C name's suffix (`YK_RECORD_STAGE_<name>`), or `None` for the GPU
    /// power-up sub-steps (`STAGE_GPU_POWER + step`).
    pub name: Option<&'static str>,
    /// The runner's token: `died_after=<label>`.
    pub label: &'static str,
}

const fn st(value: u8, name: &'static str, label: &'static str) -> Stage {
    Stage { value, name: Some(name), label }
}
const fn sub(value: u8, label: &'static str) -> Stage {
    Stage { value, name: None, label }
}

/// The STAGE breadcrumbs (kind 14): a mark is written as a stage begins (or,
/// for a "returned" stage, as it ends), so the last mark of a dead run names
/// the stage it died in. 0x4f ENDED is the only one that is not a death.
/// The i.MX8MP writes them (gc_result.c, CONFIG_GC_COMMON_RESULT_STAGES).
pub const STAGES: &[Stage] = &[
    st(0x11, "MEDIA_EARLY_CLOCKS", "media_early_clocks"), // PRE_KERNEL_1: CCM roots and gates returned
    st(0x12, "CCM", "ccm_driver"),                        // the CCM clock-control driver returned
    st(0x13, "CONSOLE_ENTER", "console_lcdif_read"),      // panel console stage 1 entered
    st(0x14, "CONSOLE_DONE", "console_stage1_done"),      // returned: w1 1 adopted, 0 nothing, 0x100 | why unread
    st(0x15, "PRE_KERNEL_1_DONE", "pre_kernel_1_done"),
    st(0x16, "SYS_CLOCK", "sys_clock"), // PRE_KERNEL_2: the system clock returned
    st(0x20, "POST_KERNEL", "post_kernel"),
    st(0x21, "GPIO", "gpio"),
    st(0x22, "PWM_I2C", "pwm_i2c"),
    st(0x23, "MEDIA", "media_domains"),
    st(0x24, "BRIDGE", "dsi_bridge"),
    st(0x25, "DISPLAY", "display_device"),
    st(0x26, "CONSOLE_COLD", "display_on_cold"), // panel console stage 2: next is imx8mp_display_on()
    st(0x27, "CONSOLE_COLD_DONE", "display_on_cold_done"), // w1 = its rc
    st(0x30, "GPU_INIT", "gpu_init"),            // also STAGE_GPU_POWER, the base of 0x31..0x37
    sub(0x31, "gpu_power_roots"),
    sub(0x32, "gpu_power_gates"),
    sub(0x33, "gpu_power_gpumix"),
    sub(0x34, "gpu_power_gpumix_adb"),
    sub(0x35, "gpu_power_gpu3d"),
    sub(0x37, "gpu_power_settle"),
    st(0x38, "GPU_IDENTIFY", "gpu_identify"),
    st(0x39, "GPU_RESET", "gpu_reset"),
    st(0x3a, "GPU_INIT_DONE", "gpu_init_done"), // w1 = rc
    st(0x40, "MAIN", "main"),
    st(0x41, "GPU_READY", "gpu_ready"),               // w1 = 1 yes, 0 no
    st(0x43, "RENDER_INIT_DONE", "render_init_done"), // w1 = rc
    st(0x44, "ALLOC_DONE", "alloc_done"),             // w1 = rc
    st(0x45, "DRAW", "draw_egl_init"),
    st(0x46, "EGL_CURRENT", "egl_current"), // w1 = 1 yes, 0 no
    st(0x47, "PROGRAM", "program"),         // w1 = 1 linked
    st(0x48, "SWAPPED", "swapped"),         // w1 = glGetError()
    st(0x4e, "DISPLAY_ON", "display_on"),   // w1 = rc
    st(0x4f, "ENDED", "ended"),             // ended without a result: w1 = 1 pass, 0 fail
];

/// The stage that is not a death: the sample ended without a result record.
pub const STAGE_ENDED: u8 = 0x4f;
/// The base of the GPU power-up sub-steps (0x31..0x37 = base + step).
pub const STAGE_GPU_POWER: u8 = 0x30;

/// The row of a stage number, if it is one.
pub fn stage_info(value: u8) -> Option<&'static Stage> {
    STAGES.iter().find(|s| s.value == value)
}

macro_rules! values {
    ($($name:ident = $v:expr;)*) => {
        $(pub const $name: u32 = $v;)*
        /// The constants above by name, for the C header (`YK_RECORD_<name>`).
        pub const VALUES: &[(&str, u32)] = &[$((stringify!($name), $v),)*];
    };
}

values! {
    // kind CASE: the status in w1's low byte.
    CASE_PASS = 1;
    CASE_CHECK = 2;
    CASE_NO_CHECK = 3;
    CASE_UNFINISHED = 4;
    CASE_NO_DEVICE = 5;
    CASE_SET_FAIL = 6;
    CASE_SET = 7;
    // kind CASE_SET: "no such case" in a case field.
    CASE_SET_NONE = 0xffff;
    // kind APP and PROBES: the status.
    STATUS_PASS = 1;
    STATUS_FAIL = 2;
    // kind APP: w1's flags.
    APP_SHOWN_BY_COPY = 0x10000;
    // kind WDOG: the tag.
    WDOG_ARMED = 1;
    WDOG_REFUSED = 2;
    WDOG_PROOF = 3;
    WDOG_PROOF_ALIVE = 4;
    // kind PROGRESS: where.
    WHERE_IMAGE = 0x0A0;
    WHERE_KERNEL = 0x0A1;
    WHERE_GPU_STEP = 0x100;
    WHERE_STAGE = 0x200;
    // kind PICTURE: the sampled rows.
    PICTURE_ROWS = 24;
    PICTURE_FIRST = 3;
    PICTURE_LAST = 21;
    // kind SWEEP.
    SWEEP_CASES = 48;
    SWEEP_SEL_LEAKS = 0x100;
    SWEEP_SEL_LIST = 0x200;
    SWEEP_TAG_LEAKS = 0xff;
    // dEQP.
    DEQP_CASES_MAX = 32;
    DEQP_SEL_FAILS = 0x8000;
    DEQP_SEL_PROBE = 0x4000;
    DEQP_SEL_STATUS = 0x2000;
    DEQP_FAILS_PER_PAGE = 6;
    // kind PERF: the selector.
    PERF_SEL = 0x1000;
    // kind GLMARK2: w2's flags (bits 28..31).
    GLMARK2_OFFSCREEN = 0x1;
    GLMARK2_SCENES = 0x2;
    GLMARK2_GL = 0x4;
    // kind GLMARK2 with GLMARK2_GL: w3's size fields (12 bits each) and the
    // two GL core counters (4 bits each, 15 = 15 or more).
    GLMARK2_GL_SIZE_MAX = 0xfff;
    GLMARK2_GL_COUNT_MAX = 15;
    GLMARK2_GL_ERRORS_SHIFT = 24;
    GLMARK2_GL_INCOMPLETE_SHIFT = 28;
    // kind GLMARK2 with GLMARK2_SCENES: a benchmark-list entry (6 bits).
    GLMARK2_ENTRY_NONE = 0x3f;
    // kind GLMARK2 with GLMARK2_SCENES: why the first failed entry failed,
    // from the first "Error: " line glmark2 logged while setting it up
    // (glmark2_why).
    GLMARK2_WHY_NO_ERROR = 0;
    GLMARK2_WHY_VERTEX_SHADER = 1;
    GLMARK2_WHY_FRAGMENT_SHADER = 2;
    GLMARK2_WHY_LINK = 3;
    GLMARK2_WHY_FILE = 4;
    GLMARK2_WHY_OTHER = 15;
    // kind GL_ERROR: the selector that asks for error k (GL_ERROR_SEL | k)
    // and w1's fields.
    GL_ERROR_SEL = 0x800;
    GL_ERROR_TAG_SHIFT = 16;
    GL_ERROR_TAG_NONE = 0xff;
    GL_ERROR_KEPT_SHIFT = 24;
    // kind GPU_STOP: the cause (gpu::pvr::freelist STOP_*).
    // 1 is not a cause: a refused grow is survived
    // (docs/notes/pvr-render-pool.md).
    GPU_STOP_AT_MAXIMUM = 2;
    GPU_STOP_GROW_TABLE_FULL = 3;
    GPU_STOP_KICK_TIMEOUT = 4;
    // kind DISPLAY: w1's flag (the LCDIF was already scanning: the window was
    // flipped onto a running chain).
    DISPLAY_ADOPTED = 0x8000_0000;
    // kind DISPLAY: w3's fields.
    DISPLAY_IRQ_NOT_READ = 0xff;
    DISPLAY_HEALTH_SHIFT = 16;
    DISPLAY_BPP_SHIFT = 24;
    DISPLAY_PITCH_SHIFT = 28;
    // kind CHECK: the tag, in the check's order (cluster/check Failure).
    CHECK_PROBES = 1;
    CHECK_UNDRAWN = 2;
    CHECK_ALPHA = 3;
    CHECK_GUARD = 4;
    CHECK_READPIXELS = 5;
    CHECK_NO_REFERENCE = 6;
    CHECK_RUN = 7;
    // kind CHECK, tag CHECK_RUN: the cause in w1 bits 16..23.
    CHECK_RUN_FENCE_TIMEOUT = 1;
    CHECK_RUN_OVER_BUDGET = 2;
    CHECK_RUN_DRAW = 3;
    CHECK_RUN_EGL = 4;
    CHECK_RUN_GL_ERROR = 5;
    CHECK_RUN_NO_FRAME = 6;
    CHECK_RUN_OTHER = 7;
    // kind CHECK, tag CHECK_RUN: each field's shift and width as its largest
    // value, which means "not measured"; one below it saturates
    // (check_run_words). w1:
    CHECK_RUN_CAUSE_SHIFT = 8;
    CHECK_RUN_CAUSE_MAX = 0xf;
    CHECK_RUN_REFUSED_SHIFT = 12;
    CHECK_RUN_REFUSED_MAX = 0xff;
    CHECK_RUN_RUNS_SHIFT = 20;
    CHECK_RUN_RUNS_MAX = 0x3f;
    CHECK_RUN_STEP_SHIFT = 26;
    CHECK_RUN_STEP_MAX = 0xf;
    CHECK_RUN_TIMEOUTS_SHIFT = 30;
    CHECK_RUN_TIMEOUTS_MAX = 0x3;
    // w2:
    CHECK_RUN_FRAME_MAX = 0xff;
    CHECK_RUN_STEP_MS_SHIFT = 8;
    CHECK_RUN_STEP_MS_MAX = 0xfff;
    CHECK_RUN_BUSY_SHIFT = 20;
    CHECK_RUN_BUSY_MAX = 0xfff;
    // w3:
    CHECK_RUN_KEEP_MAX = 0x7ff;
    CHECK_RUN_DETAIL_SHIFT = 11;
    CHECK_RUN_DETAIL_MAX = 0x1fff;
    CHECK_RUN_BLOCKED_SHIFT = 24;
    CHECK_RUN_BLOCKED_MAX = 0xff;
    // the steps of a frame, the slowest of which CHECK_RUN names: the
    // GL calls the cluster times other than glDraw*, glDraw* outside the
    // steps the library times, and those steps (src/draw_steps.rs Step,
    // numbered from CHECK_RUN_STEP_GL_PROGRAM)
    CHECK_RUN_STEP_GL_CALLS = 0;
    CHECK_RUN_STEP_GL_DRAW_REST = 1;
    CHECK_RUN_STEP_GL_PROGRAM = 2;
    CHECK_RUN_STEP_GL_UNIFORMS = 3;
    CHECK_RUN_STEP_GL_ATTRIBS = 4;
    CHECK_RUN_STEP_GL_TEXTURES = 5;
    CHECK_RUN_STEP_GL_STATE = 6;
    CHECK_RUN_STEP_GC_REST = 7;
    CHECK_RUN_STEP_GC_DS = 8;
    CHECK_RUN_STEP_GC_TILED_COPY = 9;
    CHECK_RUN_STEP_GC_FRAME = 10;
    CHECK_RUN_STEP_GC_UNIFORMS = 11;
    CHECK_RUN_STEP_GC_VERTEX = 12;
    CHECK_RUN_STEP_GC_EMIT = 13;
    CHECK_RUN_STEPS = 14;
}

/// The step names, by CHECK_RUN_STEP_* number.
pub const CHECK_RUN_STEP_NAMES: [&str; CHECK_RUN_STEPS as usize] = [
    "the other timed GL calls",
    "glDraw* outside the timed steps",
    "the GL core's program and variants",
    "the GL core's uniforms",
    "the GL core's attributes",
    "the GL core's textures",
    "the GL core's fixed-function state",
    "the GC7000 draw's bookkeeping",
    "the GC7000 depth/stencil surface",
    "the GC7000 tiled copies",
    "the GC7000 frame submit and open",
    "the GC7000 uniforms",
    "the GC7000 vertex input and indices",
    "the GC7000 command stream",
];

/* every step number (0..CHECK_RUN_STEPS) stays below the saturated value,
 * CHECK_RUN_STEP_MAX - 1 */
const _: () = assert!(CHECK_RUN_STEPS < CHECK_RUN_STEP_MAX);

/// One CHECK_RUN field: `v` saturated one below `max`, `max` itself (not
/// measured) passed through.
const fn check_run_field(v: u32, max: u32) -> u32 {
    if v == max {
        max
    } else if v < max - 1 {
        v
    } else {
        max - 1
    }
}

/// What a failed run's CHECK_RUN carries (record.rs's doc of kind CHECK).
/// A field the run did not measure is its CHECK_RUN_*_MAX.
pub struct CheckRun {
    pub rc: i32,
    pub cause: u32,
    pub refused: u32,
    pub runs: u32,
    /// The frame's slowest step (CHECK_RUN_STEP_*).
    pub step: u32,
    pub frame: u32,
    /// That step's time in the frame.
    pub step_ms: u32,
    /// The GPU's busy time over the frame's iteration (the driver's per-kick
    /// submit-to-done time, gc_render_perf).
    pub busy_ms: u32,
    pub keep_kib: u32,
    pub detail_ms: u32,
    /// The time the frame's iteration was blocked on the GPU (fence waits
    /// and submits waiting for an event), in 16 ms units.
    pub blocked_16ms: u32,
    /// The waits for a kick that timed out (the OSAL's 2 s bound).
    pub timeouts: u32,
}

/// kind CHECK, tag CHECK_RUN's w1..w3 (the C side packs the same with the
/// header's constants, cluster main.c).
pub const fn check_run_words(r: &CheckRun) -> [u32; 3] {
    [
        (r.rc as u32 & 0xff)
            | check_run_field(r.cause, CHECK_RUN_CAUSE_MAX) << CHECK_RUN_CAUSE_SHIFT
            | check_run_field(r.refused, CHECK_RUN_REFUSED_MAX) << CHECK_RUN_REFUSED_SHIFT
            | check_run_field(r.runs, CHECK_RUN_RUNS_MAX) << CHECK_RUN_RUNS_SHIFT
            | check_run_field(r.step, CHECK_RUN_STEP_MAX) << CHECK_RUN_STEP_SHIFT
            | check_run_field(r.timeouts, CHECK_RUN_TIMEOUTS_MAX) << CHECK_RUN_TIMEOUTS_SHIFT,
        check_run_field(r.frame, CHECK_RUN_FRAME_MAX)
            | check_run_field(r.step_ms, CHECK_RUN_STEP_MS_MAX) << CHECK_RUN_STEP_MS_SHIFT
            | check_run_field(r.busy_ms, CHECK_RUN_BUSY_MAX) << CHECK_RUN_BUSY_SHIFT,
        check_run_field(r.keep_kib, CHECK_RUN_KEEP_MAX)
            | check_run_field(r.detail_ms, CHECK_RUN_DETAIL_MAX) << CHECK_RUN_DETAIL_SHIFT
            | check_run_field(r.blocked_16ms, CHECK_RUN_BLOCKED_MAX) << CHECK_RUN_BLOCKED_SHIFT,
    ]
}

/// kind CHECK's w1..w3 for a failing probe: its index, window position, the
/// colour read and the reference's, how many probes failed and its largest
/// channel difference. Fields wider than their bits saturate.
pub fn check_probe_words(index: u32, x: u32, y: u32, got: [u8; 3], want: [u8; 3], failing: u32, diff: u8) -> [u32; 3] {
    let rgb = |c: [u8; 3]| (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32;
    [
        index.min(0xff) << 24 | x.min(0xfff) << 12 | y.min(0xfff),
        rgb(got) | failing.min(0xff) << 24,
        rgb(want) | (diff as u32) << 24,
    ]
}

/// w0 for a kind and a low byte.
pub const fn w0(kind: u8, low: u8) -> u32 {
    MAGIC | (kind as u32) << 8 | low as u32
}

/// kind CASE's w1. A bad check outside 0..=254 is "none" (255).
pub const fn case_w1(gl_case: u32, bad_check: i32, status: u32) -> u32 {
    let check = if bad_check < 0 || bad_check > 254 { 255 } else { bad_check as u32 };
    (gl_case & 0xffff) << 16 | check << 8 | (status & 0xff)
}

/// A set summary (kind CASE, status CASE_SET), as the walker counts it.
/// The case fields are `None` when there is no such case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SetSummary {
    pub passed: u32,
    /// cases run: the listed ones minus the skipped ones
    pub run: u32,
    pub skipped: u32,
    pub first_fail: Option<u32>,
    pub second_fail: Option<u32>,
    pub first_skip: Option<u32>,
}

impl SetSummary {
    /// w1..w3. Counts and case numbers saturate in their fields (a case
    /// number at 0xfffe, so it never reads as none).
    pub fn words(&self) -> [u32; 3] {
        let case = |c: Option<u32>| c.map_or(CASE_SET_NONE, |c| c.min(CASE_SET_NONE - 1));
        [
            case(self.first_fail) << 16 | self.skipped.min(0xff) << 8 | CASE_SET,
            case(self.second_fail) << 16 | self.passed.min(0xffff),
            case(self.first_skip) << 16 | self.run.min(0xffff),
        ]
    }

    /// The summary in w1..w3 of a CASE_SET record.
    pub fn from_words(w1: u32, w2: u32, w3: u32) -> Self {
        let case = |v: u32| if v >> 16 == CASE_SET_NONE { None } else { Some(v >> 16) };
        SetSummary {
            passed: w2 & 0xffff,
            run: w3 & 0xffff,
            skipped: (w1 >> 8) & 0xff,
            first_fail: case(w1),
            second_fail: case(w2),
            first_skip: case(w3),
        }
    }

    /// Cases that failed (run, not passed).
    pub fn failed(&self) -> u32 {
        self.run.saturating_sub(self.passed)
    }

    /// Whether the fields agree with each other: no more passes than runs,
    /// a first failure exactly when one failed, a second when two did, and a
    /// first skip exactly when one was skipped.
    pub fn consistent(&self) -> bool {
        let f = self.failed();
        self.passed <= self.run
            && self.first_fail.is_some() == (f >= 1)
            && self.second_fail.is_some() == (f >= 2)
            && self.first_skip.is_some() == (self.skipped >= 1)
    }

    /// The set's verdict: it ran something and every case it ran passed.
    pub fn passed_all(&self) -> bool {
        self.run > 0 && self.passed == self.run
    }
}

/// A colour as RGB565.
pub const fn rgb565(rgb: [u8; 3]) -> u16 {
    ((rgb[0] as u16 >> 3) << 11) | ((rgb[1] as u16 >> 2) << 5) | (rgb[2] as u16 >> 3)
}

/// kind PROBES' w1..w3 for five points (R, G, B at 8 bits).
pub fn probes_words(p: &[[u8; 3]; 5]) -> [u32; 3] {
    let mut bytes = [0u8; 15];
    for (i, px) in p.iter().enumerate() {
        bytes[3 * i..3 * i + 3].copy_from_slice(px);
    }
    let crc = crc16_ccitt_false(&bytes) as u32;
    let c = |i: usize| rgb565(p[i]) as u32;
    [c(0) | c(1) << 16, c(2) | c(3) << 16, c(4) | crc << 16]
}

/// `W << 16 | H`, the size word of PICTURE and FINGERPRINT.
pub const fn size_word(w: u32, h: u32) -> u32 {
    (w & 0xffff) << 16 | (h & 0xffff)
}

/// What kind DISPLAY carries ([`display_words`]); `struct yd_record_display`
/// in yakogl_drivers.h, which the C writer fills.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Display {
    /// 1: the LCDIF was already scanning when the window was put on it.
    pub adopted: u32,
    pub pixel_clock_hz: u32,
    /// The DSI lane rate (0 unknown).
    pub lane_rate_hz: u32,
    /// The bridge's IRQ_STAT read now; [`DISPLAY_IRQ_NOT_READ`] when it
    /// could not be read.
    pub irq_now: u32,
    /// IRQ_STAT one ms after the video started (0xff: the check failed).
    pub irq_at_start: u32,
    /// IMX8MP_DISPLAY_HEALTH_* bits.
    pub health: u32,
    /// CTRLDESCL0_5 bits 27:24.
    pub bpp_code: u32,
    /// CTRLDESCL0_3's pitch, bytes.
    pub pitch: u32,
}

/// kind DISPLAY's w1..w3.
pub const fn display_words(d: &Display) -> [u32; 3] {
    let pitch_kib = if d.pitch >> 10 > 15 { 15 } else { d.pitch >> 10 };
    [
        ((d.pixel_clock_hz / 1000) & !DISPLAY_ADOPTED) | if d.adopted != 0 { DISPLAY_ADOPTED } else { 0 },
        d.lane_rate_hz / 1000,
        (d.irq_now & 0xff)
            | (d.irq_at_start & 0xff) << 8
            | (d.health & 0xff) << DISPLAY_HEALTH_SHIFT
            | (d.bpp_code & 0xf) << DISPLAY_BPP_SHIFT
            | pitch_kib << DISPLAY_PITCH_SHIFT,
    ]
}

/// The SN65DSI84's IRQ_STAT bits (data sheet Table 7-10), named.
fn write_irq_stat(out: &mut impl fmt::Write, v: u32) -> fmt::Result {
    const BITS: [(u32, &str); 7] = [
        (7, "SYNCH_ERR"),
        (6, "CRC_ERR"),
        (5, "UNC_ECC_ERR"),
        (4, "COR_ECC_ERR"),
        (3, "LLP_ERR"),
        (2, "SOT_BIT_ERR"),
        (0, "PLL_UNLOCK"),
    ];
    write!(out, "0x{v:02x}")?;
    if v == 0 {
        return out.write_str(" (clean)");
    }
    let mut first = true;
    for (b, n) in BITS {
        if v & (1 << b) != 0 {
            out.write_str(if first { " (" } else { " " })?;
            out.write_str(n)?;
            first = false;
        }
    }
    if !first {
        out.write_str(")")?;
    }
    Ok(())
}

fn describe_display(out: &mut impl fmt::Write, w1: u32, w2: u32, w3: u32) -> fmt::Result {
    let pix = w1 & !DISPLAY_ADOPTED;
    write!(
        out,
        "{}: pixel clock {pix} kHz, DSI lane rate {w2} kHz",
        if w1 & DISPLAY_ADOPTED != 0 { "adopted" } else { "cold bring-up" }
    )?;
    // the headroom over pixel x 24 / 4 lanes (the Verdin's link), per mille
    // (ip::samsung_dsim::LINK_MARGIN_PERMILLE: 25 needed, 0 broke the picture)
    if pix != 0 && w2 != 0 {
        let tenths = (i64::from(w2) * 4 * 10_000 / (i64::from(pix) * 24)) - 10_000;
        write!(out, " (headroom {}.{} per mille at 4 lanes)", tenths / 10, (tenths % 10).abs())?;
    }
    out.write_str("; bridge IRQ_STAT now ")?;
    match w3 & 0xff {
        DISPLAY_IRQ_NOT_READ => out.write_str("not read")?,
        v => write_irq_stat(out, v)?,
    }
    out.write_str(", at the start ")?;
    match (w3 >> 8) & 0xff {
        0xff => out.write_str("not checked")?,
        v => write_irq_stat(out, v)?,
    }
    let bpp = (w3 >> DISPLAY_BPP_SHIFT) & 0xf;
    write!(
        out,
        "; health 0x{:02x}; LCDIF format {bpp} ({}) pitch {} KiB",
        (w3 >> DISPLAY_HEALTH_SHIFT) & 0xff,
        match bpp {
            4 => "RGB565",
            8 => "RGB888",
            9 => "ARGB8888",
            10 => "ABGR8888",
            _ => "?",
        },
        w3 >> DISPLAY_PITCH_SHIFT
    )
}

/// A glmark2 run's outcome, as kind GLMARK2 stores it ([`glmark2_words`]).
/// The entries are indices in the benchmark list (the default list:
/// [`GLMARK2_DEFAULT_LIST`]); [`GLMARK2_ENTRY_NONE`] when there is none.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glmark2Outcome {
    pub score: u32,
    /// GLMARK2_OFFSCREEN or 0; [`glmark2_words`] adds GLMARK2_SCENES.
    pub flags: u32,
    pub status: u32,
    pub ran: u32,
    pub failed: u32,
    pub unsupported: u32,
    pub failed_entry: u32,
    /// GLMARK2_WHY_* for `failed_entry`.
    pub why: u32,
    pub unsupported_entries: [u32; 2],
    pub w: u32,
    pub h: u32,
    /// The GL errors the entry points raised (yakogl_gl_errors_raised);
    /// stored only when `flags` has GLMARK2_GL.
    pub gl_errors: u32,
    /// The draws that sampled an incomplete texture
    /// (yakogl_incomplete_texture_draws); stored only when `flags` has
    /// GLMARK2_GL.
    pub incomplete_draws: u32,
}

/// kind GLMARK2's w1..w3 with GLMARK2_SCENES: the score saturating at
/// 0xffff, the counts at 63, an entry above 62 stored as none. With
/// GLMARK2_GL in `o.flags` (the caller read the two GL core counters), w3
/// carries them beside a 12-bit size; a size that does not fit in 12 bits
/// is not narrowed: GLMARK2_GL is dropped and w3 keeps the full size, so
/// the counters read "not recorded" rather than a wrong size.
pub const fn glmark2_words(o: &Glmark2Outcome) -> [u32; 3] {
    const fn sat(x: u32, max: u32) -> u32 {
        if x > max {
            max
        } else {
            x
        }
    }
    const fn entry(x: u32) -> u32 {
        if x >= GLMARK2_ENTRY_NONE {
            GLMARK2_ENTRY_NONE
        } else {
            x
        }
    }
    let w1 = sat(o.score, 0xffff)
        | entry(o.failed_entry) << 16
        | (o.why & 0xf) << 22
        | entry(o.unsupported_entries[0]) << 26;
    let gl = o.flags & GLMARK2_GL != 0 && o.w <= GLMARK2_GL_SIZE_MAX && o.h <= GLMARK2_GL_SIZE_MAX;
    let flags = if gl { o.flags } else { o.flags & !GLMARK2_GL };
    let w2 = ((flags | GLMARK2_SCENES) & 0xf) << 28
        | (o.status & 0xf) << 24
        | sat(o.ran, 63) << 18
        | sat(o.failed, 63) << 12
        | sat(o.unsupported, 63) << 6
        | entry(o.unsupported_entries[1]);
    let w3 = if gl {
        o.w | o.h << 12
            | sat(o.gl_errors, GLMARK2_GL_COUNT_MAX) << GLMARK2_GL_ERRORS_SHIFT
            | sat(o.incomplete_draws, GLMARK2_GL_COUNT_MAX) << GLMARK2_GL_INCOMPLETE_SHIFT
    } else {
        size_word(o.w, o.h)
    };
    [w1, w2, w3]
}

/// kind GLMARK2's window size `(W, H)` from w2 and w3, in any layout.
pub const fn glmark2_size(w2: u32, w3: u32) -> (u32, u32) {
    if (w2 >> 28) & GLMARK2_GL != 0 {
        (w3 & GLMARK2_GL_SIZE_MAX, (w3 >> 12) & GLMARK2_GL_SIZE_MAX)
    } else {
        (w3 >> 16, w3 & 0xffff)
    }
}

/// kind GLMARK2's GL core counters `[GL errors raised, draws that sampled
/// an incomplete texture]` (each saturating at GLMARK2_GL_COUNT_MAX, which
/// means that many or more), or None when the record does not carry them
/// (no GLMARK2_GL: an image before 2026-10-12, or a size wider than 12 bits).
pub const fn glmark2_gl_counts(w2: u32, w3: u32) -> Option<[u32; 2]> {
    if (w2 >> 28) & GLMARK2_GL == 0 {
        return None;
    }
    Some([(w3 >> GLMARK2_GL_ERRORS_SHIFT) & 0xf, (w3 >> GLMARK2_GL_INCOMPLETE_SHIFT) & 0xf])
}

/// kind GLMARK2's `[score, ran, failed, unsupported]` from w1 and w2, in
/// either layout (GLMARK2_SCENES or the one before it).
pub const fn glmark2_counts(w1: u32, w2: u32) -> [u32; 4] {
    if (w2 >> 28) & GLMARK2_SCENES != 0 {
        [w1 & 0xffff, (w2 >> 18) & 0x3f, (w2 >> 12) & 0x3f, (w2 >> 6) & 0x3f]
    } else {
        [w1, (w2 >> 16) & 0xff, (w2 >> 8) & 0xff, w2 & 0xff]
    }
}

/// The GLMARK2_WHY_* of one line glmark2 logged while setting a scene up:
/// its own error lines (upstream scene.cpp:314-335; model.cpp:61, 384, 399,
/// 704; libmatrix/shader-source.cc:72) start with "Error: " and name the
/// step.
/// Lines that are not errors give GLMARK2_WHY_NO_ERROR.
pub fn glmark2_why(line: &[u8]) -> u32 {
    let Some(m) = line.strip_prefix(b"Error: ") else {
        return GLMARK2_WHY_NO_ERROR;
    };
    let starts = |p: &[u8]| m.starts_with(p);
    if starts(b"Failed to add vertex shader") {
        GLMARK2_WHY_VERTEX_SHADER
    } else if starts(b"Failed to add fragment shader") {
        GLMARK2_WHY_FRAGMENT_SHADER
    } else if starts(b"Failed to link program") {
        GLMARK2_WHY_LINK
    } else if starts(b"Failed to open") || starts(b"Could not open") || m.windows(8).any(|w| w == b"3ds file") {
        GLMARK2_WHY_FILE
    } else {
        GLMARK2_WHY_OTHER
    }
}

fn glmark2_why_name(why: u32) -> &'static str {
    match why {
        GLMARK2_WHY_NO_ERROR => "no error logged",
        GLMARK2_WHY_VERTEX_SHADER => "vertex shader",
        GLMARK2_WHY_FRAGMENT_SHADER => "fragment shader",
        GLMARK2_WHY_LINK => "link",
        GLMARK2_WHY_FILE => "a file",
        GLMARK2_WHY_OTHER => "another error",
        _ => "?",
    }
}

/// glmark2's default benchmark list at the commit the sample pins
/// (driver/samples/zephyr/glmark2/CMakeLists.txt GLMARK2_COMMIT 22c527cb,
/// src/default-benchmarks.h), for naming an entry of a run that used it
/// (CONFIG_GLMARK2_ARGS without -b).
pub const GLMARK2_DEFAULT_LIST: [&str; 33] = [
    "build:use-vbo=false",
    "build:use-vbo=true",
    "texture:texture-filter=nearest",
    "texture:texture-filter=linear",
    "texture:texture-filter=mipmap",
    "shading:shading=gouraud",
    "shading:shading=blinn-phong-inf",
    "shading:shading=phong",
    "shading:shading=cel",
    "bump:bump-render=high-poly",
    "bump:bump-render=normals",
    "bump:bump-render=height",
    "effect2d:kernel=0,1,0;1,-4,1;0,1,0;",
    "effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;",
    "pulsar:quads=5:texture=false:light=false",
    "desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true",
    "desktop:windows=4:effect=shadow",
    "buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=false",
    "buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=subdata:interleave=false",
    "buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=true",
    "ideas:speed=duration",
    "jellyfish",
    "terrain",
    "shadow",
    "refract",
    "conditionals:vertex-steps=0:fragment-steps=0",
    "conditionals:vertex-steps=0:fragment-steps=5",
    "conditionals:vertex-steps=5:fragment-steps=0",
    "function:fragment-steps=5:fragment-complexity=low",
    "function:fragment-steps=5:fragment-complexity=medium",
    "loop:vertex-steps=5:fragment-steps=5:fragment-loop=false",
    "loop:vertex-steps=5:fragment-steps=5:fragment-uniform=false",
    "loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true",
];

/// The GL core counters of a GLMARK2 record, in the words of the sample's
/// console line ("glmark2: GL errors raised N, draws that sampled an
/// incomplete texture M"); absent counters are said to be absent, never 0.
fn write_glmark2_gl_counts(out: &mut impl fmt::Write, c: Option<[u32; 2]>) -> fmt::Result {
    let Some([errors, incomplete]) = c else {
        return out.write_str("; GL errors raised and draws that sampled an incomplete texture: not recorded");
    };
    fn n(out: &mut impl fmt::Write, v: u32) -> fmt::Result {
        if v >= GLMARK2_GL_COUNT_MAX {
            write!(out, "{v} or more")
        } else {
            write!(out, "{v}")
        }
    }
    out.write_str("; GL errors raised ")?;
    n(out, errors)?;
    out.write_str(", draws that sampled an incomplete texture ")?;
    n(out, incomplete)
}

/// `entry N (<name in the default list>)`, or `none`.
fn write_glmark2_entry(out: &mut impl fmt::Write, e: u32) -> fmt::Result {
    if e == GLMARK2_ENTRY_NONE {
        return out.write_str("none");
    }
    match GLMARK2_DEFAULT_LIST.get(e as usize) {
        Some(n) => write!(out, "entry {e} (default list: {n})"),
        None => write!(out, "entry {e}"),
    }
}

/// CRC-16/CCITT-FALSE: poly 0x1021, init 0xffff, no reflection, no final
/// xor (Python `binascii.crc_hqx(data, 0xffff)`). PROBES and the dEQP name
/// checks.
pub fn crc16_ccitt_false(data: &[u8]) -> u16 {
    let mut c: u16 = 0xffff;
    for &b in data {
        c ^= (b as u16) << 8;
        for _ in 0..8 {
            c = if c & 0x8000 != 0 { (c << 1) ^ 0x1021 } else { c << 1 };
        }
    }
    c
}

/// CRC-32/IEEE continued over `data` from a previous result (start with 0):
/// zlib's `crc32(crc, data)`, Zephyr's `crc32_ieee_update`. PICTURE and
/// FINGERPRINT.
pub fn crc32_ieee_update(crc: u32, data: &[u8]) -> u32 {
    let mut c = !crc;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    !c
}

/// The writer's rules: the sequence number and the breadcrumb rule. The C
/// side keeps one per image (`struct yd_record_writer`, the same layout,
/// pinned on both sides) and stores what the calls return in its sink.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Writer {
    pub seq: u8,
    /// 1 once a result record was stored.
    pub have_result: u8,
    pub pad: [u8; 2],
    pub progress_count: u32,
}

impl Writer {
    pub const fn new() -> Self {
        Writer { seq: 0, have_result: 0, pad: [0; 2], progress_count: 0 }
    }

    /// A result record with the next sequence number. Always stored.
    pub fn result(&mut self, kind: u8, w1: u32, w2: u32, w3: u32) -> [u32; 4] {
        self.have_result = 1;
        self.seq = self.seq.wrapping_add(1);
        [w0(kind, self.seq), w1, w2, w3]
    }

    /// A result record with `tag` in the low byte. Always stored.
    pub fn result_tag(&mut self, kind: u8, tag: u8, w1: u32, w2: u32, w3: u32) -> [u32; 4] {
        self.have_result = 1;
        [w0(kind, tag), w1, w2, w3]
    }

    /// A PROGRESS breadcrumb, or `None` once a result was stored.
    pub fn progress(&mut self, where_: u32, uptime_ms: u32) -> Option<[u32; 4]> {
        if self.have_result != 0 {
            return None;
        }
        self.seq = self.seq.wrapping_add(1);
        self.progress_count = self.progress_count.wrapping_add(1);
        Some([w0(kind::PROGRESS, self.seq), where_, uptime_ms, self.progress_count])
    }

    /// Another breadcrumb (`tag` in the low byte, the sequence untouched),
    /// or `None` once a result was stored.
    pub fn breadcrumb(&mut self, kind: u8, tag: u8, w1: u32, w2: u32, w3: u32) -> Option<[u32; 4]> {
        if self.have_result != 0 {
            return None;
        }
        Some([w0(kind, tag), w1, w2, w3])
    }
}

/// What a run that left this record did: the runner's verdict token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// No 'YK' magic in w0.
    NoMagic,
    /// A result record.
    Verdict,
    /// A breadcrumb: the run died after this point.
    DiedAfter(Crumb),
    /// STAGE 0x4f: the sample ended without a result record.
    EndedNoPicture,
    /// FINGERPRINT: the whole window's checksum, written beside a verdict
    /// record and never a verdict itself; a run whose last record is one
    /// left no verdict.
    Fingerprint,
}

/// The point a breadcrumb names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Crumb {
    Progress(u32),
    WdogArmed,
    Stage(u8),
}

/// The class of a record (the breadcrumb kinds are listed in the module
/// documentation).
pub fn classify(w: [u32; 4]) -> Class {
    if w[0] & 0xffff_0000 != MAGIC {
        return Class::NoMagic;
    }
    let (k, low) = ((w[0] >> 8) as u8, w[0] as u8);
    match k {
        kind::PROGRESS => Class::DiedAfter(Crumb::Progress(w[1])),
        kind::WDOG if low as u32 == WDOG_ARMED => Class::DiedAfter(Crumb::WdogArmed),
        kind::STAGE if low == STAGE_ENDED => Class::EndedNoPicture,
        kind::STAGE => Class::DiedAfter(Crumb::Stage(low)),
        kind::FINGERPRINT => Class::Fingerprint,
        _ => Class::Verdict,
    }
}

/// PROGRESS' where as a label: image_start, kernel_up, render_step_<name>,
/// sample_stage_<n>, progress_0x<hex>.
pub fn write_where(out: &mut impl fmt::Write, where_: u32) -> fmt::Result {
    // gc_render.h, enum gc_render_step.
    const GPU_STEPS: [&str; 9] = ["", "enter", "dev_ready", "carveout", "hw", "mmu", "irq", "mmu_on", "ring"];
    match (where_, where_ >> 8) {
        (WHERE_IMAGE, _) => out.write_str("image_start"),
        (WHERE_KERNEL, _) => out.write_str("kernel_up"),
        (_, 1) => match GPU_STEPS.get((where_ & 0xff) as usize) {
            Some(n) if !n.is_empty() => write!(out, "render_step_{n}"),
            _ => write!(out, "render_step_{}", where_ & 0xff),
        },
        (_, 2) => write!(out, "sample_stage_{}", where_ & 0xff),
        _ => write!(out, "progress_0x{where_:03x}"),
    }
}

/// A stage as its label, `stage_0x<hex>` when it has no row.
pub fn write_stage(out: &mut impl fmt::Write, stage: u8) -> fmt::Result {
    match stage_info(stage) {
        Some(s) => out.write_str(s.label),
        None => write!(out, "stage_0x{stage:02x}"),
    }
}

impl fmt::Display for Class {
    /// The runner's token: `verdict`, `died_after=<label>`,
    /// `ended_no_picture`, `fingerprint` or `no_magic`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Class::NoMagic => f.write_str("no_magic"),
            Class::Verdict => f.write_str("verdict"),
            Class::EndedNoPicture => f.write_str("ended_no_picture"),
            Class::Fingerprint => f.write_str("fingerprint"),
            Class::DiedAfter(c) => {
                f.write_str("died_after=")?;
                match c {
                    Crumb::Progress(w) => write_where(f, w),
                    Crumb::WdogArmed => f.write_str("wdog_armed"),
                    Crumb::Stage(s) => write_stage(f, s),
                }
            }
        }
    }
}

/// The console line of a record, without the newline.
pub fn write_line(out: &mut impl fmt::Write, w: [u32; 4]) -> fmt::Result {
    write!(out, "{LINE_PREFIX}{:08x} {:08x} {:08x} {:08x}", w[0], w[1], w[2], w[3])
}

/// The record of a console line: [`LINE_PREFIX`] anywhere in the line (a
/// capture may put a time stamp before it), then four words of exactly 8 hex
/// digits. `None` for any other line.
pub fn parse_line(line: &str) -> Option<[u32; 4]> {
    let at = line.find(LINE_PREFIX)?;
    let mut it = line[at + LINE_PREFIX.len()..].split_ascii_whitespace();
    let mut w = [0u32; 4];
    for x in w.iter_mut() {
        let s = it.next()?;
        if s.len() != 8 {
            return None;
        }
        *x = u32::from_str_radix(s, 16).ok()?;
    }
    Some(w)
}

/// FNV-1a, 32 bits (offset basis 0x811c9dc5, prime 0x01000193): kind
/// GL_ERROR's w2, the hash of an entry point's name, which a decoder matches
/// against the GL headers' names.
pub const fn fnv1a32(data: &[u8]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    let mut i = 0;
    while i < data.len() {
        h = (h ^ data[i] as u32).wrapping_mul(0x0100_0193);
        i += 1;
    }
    h
}

/// The name of a GL error code, or "?".
fn gl_error_name(code: u32) -> &'static str {
    match code {
        0x500 => "GL_INVALID_ENUM",
        0x501 => "GL_INVALID_VALUE",
        0x502 => "GL_INVALID_OPERATION",
        0x505 => "GL_OUT_OF_MEMORY",
        0x506 => "GL_INVALID_FRAMEBUFFER_OPERATION",
        _ => "?",
    }
}

/// kind GL_ERROR after its prefix: error `k`, its code, entry point hash and
/// tag, and the run's count.
fn describe_gl_error(out: &mut impl fmt::Write, k: u8, w1: u32, w2: u32, w3: u32) -> fmt::Result {
    let (code, tag, kept) = (w1 & 0xffff, (w1 >> GL_ERROR_TAG_SHIFT) & 0xff, w1 >> GL_ERROR_KEPT_SHIFT);
    if u32::from(k) >= kept {
        return write!(out, "error {k}: not kept ({kept} kept); GL errors raised {w3}");
    }
    write!(
        out,
        "error {k} of {kept} kept: 0x{code:x} ({}) in the entry point with FNV-1a 0x{w2:08x}, ",
        gl_error_name(code)
    )?;
    if tag == GL_ERROR_TAG_NONE {
        out.write_str("no tag")?;
    } else {
        write!(out, "tag {tag} (glmark2: ")?;
        write_glmark2_entry(out, tag)?;
        out.write_str(")")?;
    }
    write!(out, "; GL errors raised {w3}")
}

fn status_name(s: u32) -> &'static str {
    match s {
        STATUS_PASS => "pass",
        STATUS_FAIL => "fail",
        _ => "?",
    }
}

/// An animation's rate, normalised by the window's pixels so that runs on
/// panels of different sizes compare: the frames and their milliseconds
/// from APP (w2, w3), the window from FINGERPRINT (w3), the mean draw from
/// PERF page 0 (w2's high half, 0.1 ms). Nothing here is a new field; it is
/// arithmetic over what the three records hold (the cluster sample stores
/// them in that order: FINGERPRINT, PERF, APP).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Throughput {
    pub w: u32,
    pub h: u32,
    pub frames: u32,
    pub ms: u32,
    /// The mean draw in 0.1 ms; `None` without a PERF page.
    pub draw_tenths: Option<u32>,
}

/// `(v + d / 2) / d`, 0 for d == 0.
fn div_round(v: u64, d: u64) -> u64 {
    (v + d / 2).checked_div(d).unwrap_or(0)
}

impl Throughput {
    fn px(&self) -> u64 {
        self.w as u64 * self.h as u64
    }

    /// Frames per second, times 10.
    pub fn fps_x10(&self) -> u64 {
        div_round(self.frames as u64 * 10_000, self.ms as u64)
    }

    /// Mpx/s = w * h * fps / 1e6, times 10.
    pub fn mpx_per_s_x10(&self) -> u64 {
        div_round(self.px() * self.frames as u64 * 10_000, self.ms as u64 * 1_000_000)
    }

    /// ms per Mpx = the mean draw / (w * h / 1e6), times 100.
    pub fn ms_per_mpx_x100(&self) -> Option<u64> {
        self.draw_tenths.map(|d| div_round(d as u64 * 10_000_000, self.px()))
    }

    /// From a run's records: the last PERF page 0 directly followed by an
    /// APP record, with the last FINGERPRINT before them for the size. None
    /// when the run has no such pair (APP alone is not enough: other writers,
    /// board_finish among them, put the GPU resets in its w3), or the APP
    /// counts no milliseconds.
    pub fn from_records(recs: &[[u32; 4]]) -> Option<Self> {
        let is = |w: &[u32; 4], k: u8| w[0] & 0xffff_0000 == MAGIC && (w[0] >> 8) as u8 == k;
        let i = (0..recs.len().saturating_sub(1))
            .rev()
            .find(|&i| is(&recs[i], kind::PERF) && recs[i][0] as u8 == 0 && is(&recs[i + 1], kind::APP))?;
        let fp = recs[..i].iter().rev().find(|w| is(w, kind::FINGERPRINT))?;
        let (perf, app) = (recs[i], recs[i + 1]);
        let t = Throughput {
            w: fp[3] >> 16,
            h: fp[3] & 0xffff,
            frames: app[2],
            ms: app[3],
            // PERF page 0 as the cluster packs it (check/src/lib.rs,
            // Present::perf_words): w2 = frame worst | draw mean << 16
            draw_tenths: Some(perf[2] >> 16),
        };
        (t.ms != 0 && t.px() != 0).then_some(t)
    }
}

/// `WxH: F frames in M ms, X.X fps, Y.Y Mpx/s; draw D.D ms mean, Z.ZZ ms/Mpx`
/// (the draw part only with a PERF page).
pub fn write_throughput(out: &mut impl fmt::Write, t: &Throughput) -> fmt::Result {
    let (fps, mpx) = (t.fps_x10(), t.mpx_per_s_x10());
    write!(
        out,
        "{}x{}: {} frames in {} ms, {}.{} fps, {}.{} Mpx/s",
        t.w,
        t.h,
        t.frames,
        t.ms,
        fps / 10,
        fps % 10,
        mpx / 10,
        mpx % 10
    )?;
    if let (Some(d), Some(m)) = (t.draw_tenths, t.ms_per_mpx_x100()) {
        write!(out, "; draw {}.{} ms mean, {}.{:02} ms/Mpx", d / 10, d % 10, m / 100, m % 100)?;
    }
    Ok(())
}

/// A CASE_SET case field: the number, or "none".
fn write_case_or_none(out: &mut impl fmt::Write, c: Option<u32>) -> fmt::Result {
    match c {
        Some(c) => write!(out, "{c}"),
        None => out.write_str("none"),
    }
}

/// A record in words, for a reader: one line naming every field. The dEQP
/// name checks are printed as hex; mapping them to case names needs the case
/// list and is the host tool's.
pub fn describe(out: &mut impl fmt::Write, w: [u32; 4]) -> fmt::Result {
    let [w0, w1, w2, w3] = w;
    if w0 & 0xffff_0000 != MAGIC {
        return write!(out, "NO RECORD (w0 = {w0:08x})");
    }
    let (k, low) = ((w0 >> 8) as u8, w0 as u8);
    let lo = |x: u32| x & 0xffff;
    let hi = |x: u32| x >> 16;
    match kind_info(k) {
        Some(i) => write!(out, "{} ({k}) {} {low}: ", i.name, if i.low == Low::Seq { "seq" } else { "tag" })?,
        None => return write!(out, "kind {k} low {low}: w1 {w1:08x} w2 {w2:08x} w3 {w3:08x} (not a known kind)"),
    }
    match k {
        kind::CASE if w1 & 0xff == CASE_SET => {
            let s = SetSummary::from_words(w1, w2, w3);
            write!(
                out,
                "set status {CASE_SET} (set {}) passed {} of {} run; skipped {}, first skip case ",
                if s.passed_all() { "passed" } else { "failed" },
                s.passed,
                s.run,
                s.skipped
            )?;
            write_case_or_none(out, s.first_skip)?;
            write!(out, "; failing {}: first case ", s.failed())?;
            write_case_or_none(out, s.first_fail)?;
            out.write_str(", second case ")?;
            write_case_or_none(out, s.second_fail)?;
            if s.failed() > 2 {
                write!(out, ", {} more not in the record", s.failed() - 2)?;
            }
            if !s.consistent() {
                out.write_str(" (INCONSISTENT fields)")?;
            }
            Ok(())
        }
        kind::CASE => write!(
            out,
            "case {} check {} status {} ({}) got 0x{w2:08x} want 0x{w3:08x}",
            hi(w1),
            (w1 >> 8) & 0xff,
            w1 & 0xff,
            match w1 & 0xff {
                CASE_PASS => "pass",
                CASE_CHECK => "check failed",
                CASE_NO_CHECK => "failed, no check",
                CASE_UNFINISHED => "unfinished",
                CASE_NO_DEVICE => "no device",
                CASE_SET_FAIL => "set failed",
                _ => "?",
            }
        ),
        kind::APP => write!(
            out,
            "stage {} status {} ({}){} frames {w2} app 0x{w3:08x}",
            (w1 >> 8) & 0xff,
            w1 & 0xff,
            status_name(w1 & 0xff),
            if w1 & APP_SHOWN_BY_COPY != 0 { " shown by copy" } else { "" }
        ),
        kind::FAULT => write!(out, "esr 0x{w1:08x} (EC 0x{:02x}) elr 0x{w2:08x} far 0x{w3:08x}", w1 >> 26),
        kind::PICTURE => write!(out, "centre 0x{w1:08x} crc32 0x{w2:08x} size {}x{}", hi(w3), lo(w3)),
        kind::PROGRESS => {
            write!(out, "where 0x{w1:03x} (")?;
            write_where(out, w1)?;
            write!(out, ") uptime {w2} ms count {w3}; nothing after this point finished")
        }
        kind::PROBES => write!(
            out,
            "status {} p0 {:04x} p1 {:04x} p2 {:04x} p3 {:04x} p4 {:04x} crc16 {:04x} (RGB565)",
            status_name(low as u32),
            lo(w1),
            hi(w1),
            lo(w2),
            hi(w2),
            lo(w3),
            hi(w3)
        ),
        kind::SWEEP if low as u32 == SWEEP_TAG_LEAKS => write!(
            out,
            "counters: lost work {} GPU resets {} fence timeouts {}; first leaker offset {} leakers {}; \
             pass bits {:04x} {w3:08x}",
            hi(w1),
            (w1 >> 8) & 0xff,
            w1 & 0xff,
            (w2 >> 8) & 0xff,
            w2 & 0xff,
            hi(w2)
        ),
        kind::SWEEP => write!(out, "cases finished {low}, two bits per case {w1:08x} {w2:08x} {w3:08x}"),
        kind::DEQP => write!(
            out,
            "pass {} fail {} notsup {} other {} begun {} first fail name check {:04x}",
            lo(w1),
            hi(w1),
            lo(w2),
            hi(w2),
            lo(w3),
            hi(w3)
        ),
        kind::DEQP_CASES => {
            let (mut p, mut f, mut o) = (0, 0, 0);
            for i in 0..(low as usize).min(32) {
                match ([w1, w2][i / 16] >> (2 * (i % 16))) & 3 {
                    1 => p += 1,
                    2 => f += 1,
                    3 => o += 1,
                    _ => {}
                }
            }
            write!(
                out,
                "begun {low}: pass {p} fail {f} other {o}; first fail index {} name check {:04x}; \
                 GPU resets {} fence timeouts {}",
                w3 & 0xff,
                hi(w3),
                (w3 >> 12) & 15,
                (w3 >> 8) & 15
            )
        }
        kind::DEQP_FAILS => write!(
            out,
            "fails seen {low}: name checks {:04x} {:04x} {:04x} {:04x} {:04x} {:04x}",
            lo(w1),
            hi(w1),
            lo(w2),
            hi(w2),
            lo(w3),
            hi(w3)
        ),
        kind::DEQP_STATUS => {
            write!(out, "case index {low} status '")?;
            for word in [w1, w2, w3] {
                for b in word.to_le_bytes() {
                    if b != 0 {
                        out.write_char(if b.is_ascii_graphic() || b == b' ' { b as char } else { '?' })?;
                    }
                }
            }
            out.write_str("'")
        }
        kind::PERF => write!(
            out,
            "experiment {} page {}: {} {} {} {} {} {}",
            low >> 3,
            low & 7,
            lo(w1),
            hi(w1),
            lo(w2),
            hi(w2),
            lo(w3),
            hi(w3)
        ),
        kind::WDOG => write!(
            out,
            "{} at uptime {w3} ms; WCR found 0x{:04x} now 0x{:04x}; flags 0x{:02x} refusal {} WMCR 0x{:04x}",
            match low as u32 {
                WDOG_ARMED => "armed (breadcrumb)",
                WDOG_REFUSED => "REFUSED: the image reset at once",
                WDOG_PROOF => "proof: armed and not fed",
                WDOG_PROOF_ALIVE => "proof FAILED: alive at the backstop",
                _ => "unknown tag",
            },
            lo(w1),
            hi(w1),
            w2 & 0xff,
            (w2 >> 8) & 0xff,
            hi(w2)
        ),
        kind::STAGE => {
            write!(out, "0x{low:02x} ")?;
            write_stage(out, low)?;
            write!(out, " value {} (0x{w1:08x}) uptime {w2} ms WDOG1 feeds {w3}; ", w1 as i32)?;
            out.write_str(if low == STAGE_ENDED {
                "the sample ended without a result record"
            } else {
                "a breadcrumb: the run died after this point"
            })
        }
        kind::FINGERPRINT => write!(out, "window crc32 0x{w1:08x} frames {w2} size {}x{}", hi(w3), lo(w3)),
        kind::GLMARK2 => {
            let scenes = (w2 >> 28) & GLMARK2_SCENES != 0;
            let [score, ran, failed, unsupported] = glmark2_counts(w1, w2);
            let (width, height) = glmark2_size(w2, w3);
            write!(
                out,
                "score {score} status {} ({}) scenes ran {ran} failed {failed} unsupported {unsupported} size {width}x{height} {}",
                (w2 >> 24) & 0xf,
                status_name((w2 >> 24) & 0xf),
                if (w2 >> 28) & GLMARK2_OFFSCREEN != 0 { "off-screen" } else { "on-screen" }
            )?;
            if !scenes {
                return Ok(());
            }
            out.write_str("; first failed: ")?;
            write_glmark2_entry(out, (w1 >> 16) & 0x3f)?;
            if (w1 >> 16) & 0x3f != GLMARK2_ENTRY_NONE {
                let why = (w1 >> 22) & 0xf;
                write!(out, ", why {why} ({})", glmark2_why_name(why))?;
            }
            out.write_str("; unsupported: ")?;
            write_glmark2_entry(out, w1 >> 26)?;
            out.write_str(", ")?;
            write_glmark2_entry(out, w2 & 0x3f)?;
            write_glmark2_gl_counts(out, glmark2_gl_counts(w2, w3))
        }
        kind::GPU_STOP => write!(
            out,
            "cause {low} ({}) fence {w1} GPU recoveries {w2} list {} KiB {}",
            match low as u32 {
                GPU_STOP_GROW_TABLE_FULL => "a freelist grow past the host's table",
                GPU_STOP_KICK_TIMEOUT => "a kick that did not complete in the stop's time",
                GPU_STOP_AT_MAXIMUM => "out of memory with the freelist at its maximum",
                _ => "?",
            },
            w3 >> 24,
            w3 & 0xff_ffff
        ),
        kind::CHECK if low as u32 == CHECK_PROBES => write!(
            out,
            "probes: probe {} at ({},{}) reads {},{},{}, the reference {},{},{} (diff {}); {} probes failing",
            w1 >> 24,
            (w1 >> 12) & 0xfff,
            w1 & 0xfff,
            (w2 >> 16) & 0xff,
            (w2 >> 8) & 0xff,
            w2 & 0xff,
            (w3 >> 16) & 0xff,
            (w3 >> 8) & 0xff,
            w3 & 0xff,
            w3 >> 24,
            w2 >> 24
        ),
        kind::CHECK => match low as u32 {
            CHECK_UNDRAWN => write!(out, "undrawn: {w1} pixels keep the sentinel"),
            CHECK_ALPHA => write!(out, "alpha: {w1} pixels with alpha != 255"),
            CHECK_GUARD => write!(out, "guard: {w1} guard-row pixels changed"),
            CHECK_READPIXELS => write!(out, "readpixels: glReadPixels differs from memory at {w1} probes"),
            CHECK_NO_REFERENCE => out.write_str("no reference: no probe table for this window size"),
            CHECK_RUN => {
                let f = |w: u32, shift: u32, max: u32| (w >> shift) & max;
                let cause = match f(w1, CHECK_RUN_CAUSE_SHIFT, CHECK_RUN_CAUSE_MAX) {
                    CHECK_RUN_FENCE_TIMEOUT => "the frame's fence did not signal",
                    CHECK_RUN_OVER_BUDGET => "a frame over the budget",
                    CHECK_RUN_DRAW => "the UI's draw failed",
                    CHECK_RUN_EGL => "an EGL call failed",
                    CHECK_RUN_GL_ERROR => "a GL error",
                    CHECK_RUN_NO_FRAME => "no frame presented",
                    CHECK_RUN_OTHER => "another failure",
                    _ => "?",
                };
                write!(
                    out,
                    "run: the check passed, the run failed later: {cause} at frame {} ({} ms; ",
                    f(w2, 0, CHECK_RUN_FRAME_MAX),
                    f(w3, CHECK_RUN_DETAIL_SHIFT, CHECK_RUN_DETAIL_MAX)
                )?;
                match (
                    f(w1, CHECK_RUN_STEP_SHIFT, CHECK_RUN_STEP_MAX),
                    f(w2, CHECK_RUN_STEP_MS_SHIFT, CHECK_RUN_STEP_MS_MAX),
                ) {
                    (CHECK_RUN_STEP_MAX, _) => out.write_str("its steps not measured")?,
                    (st, ms) => {
                        let name = CHECK_RUN_STEP_NAMES.get(st as usize).copied().unwrap_or("step ?");
                        match ms {
                            CHECK_RUN_STEP_MS_MAX => write!(out, "most in {name}, its time not measured")?,
                            ms => write!(out, "most in {name}, {ms} ms")?,
                        }
                    }
                }
                match f(w2, CHECK_RUN_BUSY_SHIFT, CHECK_RUN_BUSY_MAX) {
                    CHECK_RUN_BUSY_MAX => out.write_str(", GPU busy not measured")?,
                    ms => write!(out, ", GPU busy {ms} ms")?,
                }
                match (
                    f(w3, CHECK_RUN_BLOCKED_SHIFT, CHECK_RUN_BLOCKED_MAX),
                    f(w1, CHECK_RUN_TIMEOUTS_SHIFT, CHECK_RUN_TIMEOUTS_MAX),
                ) {
                    (CHECK_RUN_BLOCKED_MAX, _) => out.write_str(", blocked on the GPU not measured")?,
                    (w, CHECK_RUN_TIMEOUTS_MAX) => write!(out, ", about {} ms blocked on the GPU", w * 16)?,
                    (w, t) => write!(out, ", about {} ms blocked on the GPU, {t} waits timed out", w * 16)?,
                }
                match f(w1, CHECK_RUN_RUNS_SHIFT, CHECK_RUN_RUNS_MAX) {
                    CHECK_RUN_RUNS_MAX => out.write_str(", compiler runs not counted")?,
                    n => write!(out, ", {n} compiler runs")?,
                }
                write!(
                    out,
                    "), rc {}; draws refused {}; ",
                    (w1 & 0xff) as u8 as i8,
                    f(w1, CHECK_RUN_REFUSED_SHIFT, CHECK_RUN_REFUSED_MAX)
                )?;
                match f(w3, 0, CHECK_RUN_KEEP_MAX) {
                    CHECK_RUN_KEEP_MAX => out.write_str("no keep region"),
                    k => write!(out, "keep high-water {k} KiB"),
                }
            }
            _ => write!(out, "what {low} (?) {w1:08x} {w2:08x} {w3:08x}"),
        },
        kind::DISPLAY => describe_display(out, w1, w2, w3),
        kind::GL_ERROR => describe_gl_error(out, low, w1, w2, w3),
        _ => Ok(()),
    }
}

/// The C header, `driver/zephyr/lib/board/include/yakogl_record.h`: every
/// number of this file under a `YK_RECORD_` name. A test of the crate
/// compares the committed file with this output (`board::tests`).
pub fn write_c_header(out: &mut impl fmt::Write) -> fmt::Result {
    out.write_str(
        "/* SPDX-License-Identifier: Apache-2.0 */\n\
         /* GENERATED from driver/rust/src/board/record.rs by its crate's test\n\
         \x20* (YK_RECORD_BLESS=1 cargo test -p yakogl-drivers board). DO NOT EDIT.\n\
         \x20*\n\
         \x20* The result record (driver/docs/one-method.md, section 3): four words,\n\
         \x20* w0 = YK_RECORD_MAGIC | kind << 8 | low, then w1..w3 as the kind\n\
         \x20* defines them (record.rs, KINDS). `low` is a sequence number, or the\n\
         \x20* kind's own value for the kinds marked \"tag\" below. Each stored record\n\
         \x20* is printed as YK_RECORD_LINE_FMT. */\n\
         #ifndef YAKOGL_RECORD_H\n\
         #define YAKOGL_RECORD_H\n\n",
    )?;
    writeln!(out, "#define YK_RECORD_MAGIC 0x{MAGIC:08x}u")?;
    writeln!(out, "#define YK_RECORD_SELECTOR_MAGIC 0x{SELECTOR_MAGIC:08x}u")?;
    writeln!(out, "#define YK_RECORD_LINE_PREFIX \"{LINE_PREFIX}\"")?;
    writeln!(out, "#define YK_RECORD_LINE_FMT YK_RECORD_LINE_PREFIX \"%08x %08x %08x %08x\\n\"")?;
    out.write_str("\n/* kinds */\n")?;
    for k in KINDS {
        writeln!(
            out,
            "#define YK_RECORD_KIND_{} {}u /* low byte: {} */",
            k.name,
            k.value,
            if k.low == Low::Seq { "seq" } else { "tag" }
        )?;
    }
    out.write_str("\n/* kind STAGE: the stage in the low byte */\n")?;
    writeln!(out, "#define YK_RECORD_STAGE_GPU_POWER 0x{STAGE_GPU_POWER:02x}u /* + gpu::vivante::power::step */")?;
    for s in STAGES {
        if let Some(n) = s.name {
            writeln!(out, "#define YK_RECORD_STAGE_{n} 0x{:02x}u", s.value)?;
        }
    }
    out.write_str("\n")?;
    for (n, v) in VALUES {
        writeln!(out, "#define YK_RECORD_{n} 0x{v:x}u")?;
    }
    out.write_str("\n#endif /* YAKOGL_RECORD_H */\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DISPLAY: the record the fixed Verdin image should leave (69.3 MHz,
    /// window flipped onto the chain the panel console brought up, a clean
    /// link) and the 71.1 MHz control, both decoded.
    #[test]
    fn display_words_and_their_decode() {
        let fixed = Display {
            adopted: 1,
            pixel_clock_hz: 69_300_000,
            lane_rate_hz: 426_600_000,
            irq_now: 0,
            irq_at_start: 0,
            health: 0x3f,
            bpp_code: 9,
            pitch: 5120,
        };
        assert_eq!(display_words(&fixed), [0x8001_0eb4, 0x0006_8268, 0x593f_0000]);
        let b = text(|b| describe(b, [w0(kind::DISPLAY, 0x0f), 0x8001_0eb4, 0x0006_8268, 0x593f_0000]));
        assert_eq!(
            b.as_str(),
            "DISPLAY (22) seq 15: adopted: pixel clock 69300 kHz, DSI lane rate 426600 kHz (headroom 25.9 per \
             mille at 4 lanes); bridge IRQ_STAT now 0x00 (clean), at the start 0x00 (clean); health 0x3f; LCDIF \
             format 9 (ARGB8888) pitch 5 KiB"
        );
        let control = Display { pixel_clock_hz: 71_100_000, irq_now: 0x88, ..fixed };
        let w = display_words(&control);
        let b = text(|b| describe(b, [w0(kind::DISPLAY, 0x0f), w[0], w[1], w[2]]));
        assert!(b.as_str().contains("(headroom 0.0 per mille at 4 lanes)"), "{}", b.as_str());
        assert!(b.as_str().contains("now 0x88 (SYNCH_ERR LLP_ERR)"), "{}", b.as_str());
        // the fields saturate and an unread IRQ_STAT is its own value
        let w = display_words(&Display { irq_now: DISPLAY_IRQ_NOT_READ, irq_at_start: 0xff, pitch: 1 << 20, ..fixed });
        assert_eq!((w[2] & 0xff, (w[2] >> 8) & 0xff, w[2] >> 28), (0xff, 0xff, 15));
        assert_eq!(classify([w0(kind::DISPLAY, 1), w[0], w[1], w[2]]), Class::Verdict);
    }

    /// CHECK_RUN: each field lands at its bits, saturates one below its
    /// "not measured" value, and that value passes through unchanged.
    #[test]
    fn check_run_words_keep_not_measured_apart_from_saturated() {
        let r = CheckRun {
            rc: -116,
            cause: CHECK_RUN_OVER_BUDGET,
            refused: 0,
            runs: 0,
            step: CHECK_RUN_STEP_GC_FRAME,
            frame: 2,
            step_ms: 2782,
            busy_ms: 2600,
            keep_kib: 464,
            detail_ms: 2787,
            blocked_16ms: 170,
            timeouts: 1,
        };
        let w = check_run_words(&r);
        assert_eq!(w[0] & 0xff, (-116i32 as u32) & 0xff);
        assert_eq!(((w[0] >> 8) & 0xf, (w[0] >> 20) & 0x3f, (w[0] >> 26) & 0xf, w[0] >> 30), (2, 0, 10, 1));
        assert_eq!((w[1] & 0xff, (w[1] >> 8) & 0xfff, w[1] >> 20), (2, 2782, 2600));
        assert_eq!((w[2] & 0x7ff, (w[2] >> 11) & 0x1fff, w[2] >> 24), (464, 2787, 170));
        let big = check_run_words(&CheckRun { frame: 900, step_ms: 9000, runs: 500, keep_kib: 5000, ..r });
        assert_eq!((big[1] & 0xff, (big[1] >> 8) & 0xfff), (0xfe, 0xffe), "saturated, not absent");
        assert_eq!(((big[0] >> 20) & 0x3f, big[2] & 0x7ff), (0x3e, 0x7fe));
        let none = check_run_words(&CheckRun {
            runs: CHECK_RUN_RUNS_MAX,
            step: CHECK_RUN_STEP_MAX,
            step_ms: CHECK_RUN_STEP_MS_MAX,
            busy_ms: CHECK_RUN_BUSY_MAX,
            blocked_16ms: CHECK_RUN_BLOCKED_MAX,
            timeouts: CHECK_RUN_TIMEOUTS_MAX,
            ..r
        });
        assert_eq!(((none[1] >> 8) & 0xfff, none[1] >> 20), (CHECK_RUN_STEP_MS_MAX, CHECK_RUN_BUSY_MAX));
        assert_eq!(((none[0] >> 20) & 0x3f, (none[0] >> 26) & 0xf, none[0] >> 30, none[2] >> 24), (0x3f, 0xf, 3, 0xff));
    }

    /// A fixed buffer for `fmt::Write` (the crate has no allocator).
    struct Buf {
        b: [u8; 512],
        n: usize,
    }
    impl Buf {
        fn as_str(&self) -> &str {
            core::str::from_utf8(&self.b[..self.n]).unwrap()
        }
    }
    impl fmt::Write for Buf {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            let e = self.n + s.len();
            if e > self.b.len() {
                return Err(fmt::Error);
            }
            self.b[self.n..e].copy_from_slice(s.as_bytes());
            self.n = e;
            Ok(())
        }
    }
    fn text(f: impl FnOnce(&mut Buf) -> fmt::Result) -> Buf {
        let mut b = Buf { b: [0; 512], n: 0 };
        f(&mut b).unwrap();
        b
    }
    fn shown(d: impl fmt::Display) -> Buf {
        text(|b| fmt::write(b, format_args!("{d}")))
    }

    #[test]
    fn kinds_are_one_to_twenty_three_and_the_tag_list_is_the_formats() {
        // 1..=23 in order, each number once, defined here or reserved
        let mut want = (1..=23u8).filter(|v| !RESERVED_KINDS.contains(v));
        for k in KINDS {
            assert_eq!(Some(k.value), want.next(), "{}", k.name);
        }
        assert_eq!(want.next(), None, "a kind is missing");
        assert_eq!(KINDS.len() + RESERVED_KINDS.len(), 23);
        assert_eq!(kind::STAGE, 14);
        assert_eq!(kind::FINGERPRINT, 15);
        assert_eq!(kind::GLMARK2, 18);
        assert_eq!(kind::GPU_STOP, 20);
        assert_eq!(kind::CHECK, 21);
        assert_eq!(kind::DISPLAY, 22);
        assert_eq!(kind::GL_ERROR, 23);
        // one-method.md section 3's list, plus STAGE and FAULT's tag.
        let tags = [
            kind::FAULT,
            kind::PROBES,
            kind::SWEEP,
            kind::DEQP_CASES,
            kind::DEQP_FAILS,
            kind::DEQP_STATUS,
            kind::PERF,
            kind::WDOG,
            kind::STAGE,
            kind::GPU_STOP,
            kind::CHECK,
            kind::GL_ERROR,
        ];
        for k in KINDS {
            assert_eq!(k.low == Low::Tag, tags.contains(&k.value), "{}", k.name);
        }
    }

    #[test]
    fn stages_unique_and_sub_steps_in_place() {
        for (i, a) in STAGES.iter().enumerate() {
            for b in &STAGES[i + 1..] {
                assert_ne!(a.value, b.value);
                assert_ne!(a.label, b.label);
                assert!(a.name.is_none() || a.name != b.name);
            }
            if a.name.is_none() {
                assert!(a.value > STAGE_GPU_POWER && a.value <= STAGE_GPU_POWER + 7);
            }
        }
        assert_eq!(stage_info(STAGE_ENDED).unwrap().name, Some("ENDED"));
        assert_eq!(stage_info(STAGE_GPU_POWER).unwrap().name, Some("GPU_INIT"));
    }

    /// The old Python decoder's self-test cases, the two board records of
    /// 2026-10-03 first (docs/notes/imx8mp-netboot.md).
    #[test]
    fn classify_known_records() {
        let cases: &[([u32; 4], &str)] = &[
            ([0x594B0D01, 0x773F773F, 0x00000001, 0x00002E30], "died_after=wdog_armed"),
            ([0x594B040E, 0xFF403F80, 0x35706C4E, 0x04000258], "verdict"),
            ([0x594B0D03, 0x773F773F, 0x00000001, 0x00002E30], "verdict"),
            ([0x594B0D02, 0x0000773F, 0x00000300, 0x00000010], "verdict"),
            ([0x594B0E13, 0, 11830, 0], "died_after=console_lcdif_read"),
            ([0x594B0E33, 0, 12100, 1], "died_after=gpu_power_gpumix"),
            ([0x594B0E3A, 0xFFFFFFED, 12200, 1], "died_after=gpu_init_done"),
            ([0x594B0E4F, 0, 15000, 4], "ended_no_picture"),
            ([0x594B0E7F, 0, 1, 0], "died_after=stage_0x7f"),
            ([0x594B0503, 0x104, 13000, 3], "died_after=render_step_hw"),
            ([0x594B0502, 0x0A1, 12900, 2], "died_after=kernel_up"),
            ([0x594B0101, 0x00070101, 1, 0], "verdict"),
            ([0x00000000, 0x53450003, 0, 0], "no_magic"),
        ];
        for (w, want) in cases {
            assert_eq!(shown(classify(*w)).as_str(), *want, "{w:08x?}");
        }
        // FINGERPRINT's doc says "not a verdict": its own class, never Verdict
        let fp = [w0(kind::FINGERPRINT, 1), 0x1a2b_3c4d, 2, size_word(1024, 600)];
        assert_eq!(classify(fp), Class::Fingerprint);
        assert_eq!(shown(classify(fp)).as_str(), "fingerprint");
        // every other kind with the magic is a verdict or a breadcrumb, never Fingerprint
        for k in KINDS.iter().filter(|k| k.value != kind::FINGERPRINT) {
            assert_ne!(classify([w0(k.value, 2), 0, 0, 0]), Class::Fingerprint, "kind {}", k.name);
        }
        for s in STAGES {
            let got = shown(classify([w0(kind::STAGE, s.value), 0, 0, 0]));
            if s.value == STAGE_ENDED {
                assert_eq!(got.as_str(), "ended_no_picture");
            } else {
                assert_eq!(&got.as_str()["died_after=".len()..], s.label);
            }
        }
    }

    /// The baseline PICTURE of the i.MX8MP triangle (kexec and netboot,
    /// 2026-10-03), as its image writes it: two PROGRESS (image, kernel),
    /// eleven render steps, then the picture, sequence 0x0e. Breadcrumbs of
    /// other kinds in between do not move the sequence.
    #[test]
    fn writer_reproduces_the_baseline_picture() {
        let mut w = Writer::new();
        assert_eq!(w.progress(WHERE_IMAGE, 0).unwrap()[0], 0x594B0501);
        assert!(w.breadcrumb(kind::WDOG, WDOG_ARMED as u8, 0x773F773F, 1, 0x2E30).is_some());
        assert!(w.breadcrumb(kind::STAGE, 0x20, 0, 5, 1).is_some());
        assert_eq!(w.progress(WHERE_KERNEL, 1).unwrap()[0], 0x594B0502);
        for step in 1..=11 {
            w.progress(WHERE_GPU_STEP | step, 2).unwrap();
        }
        let r = w.result(kind::PICTURE, 0xFF403F80, 0x35706C4E, size_word(1024, 600));
        assert_eq!(r, [0x594B040E, 0xFF403F80, 0x35706C4E, 0x04000258]);
        assert!(w.progress(WHERE_STAGE | 1, 3).is_none(), "a verdict is never replaced by a breadcrumb");
        assert!(w.breadcrumb(kind::STAGE, STAGE_ENDED, 1, 0, 0).is_none());
        assert_eq!(w.result_tag(kind::PROBES, 1, 0, 0, 0)[0], 0x594B0601);
        assert_eq!(w.result(kind::APP, 1, 0, 0)[0], 0x594B020F);
    }

    #[test]
    fn line_round_trip_and_garbage() {
        let w = [0x594B040E, 0xFF403F80, 0x35706C4E, 0x04000258];
        let l = text(|b| write_line(b, w));
        assert_eq!(l.as_str(), "yk-record: 594b040e ff403f80 35706c4e 04000258");
        assert_eq!(parse_line(l.as_str()), Some(w));
        assert_eq!(parse_line("[12:00:01.5] yk-record: 594b040e ff403f80 35706c4e 04000258\r"), Some(w));
        assert_eq!(parse_line("yk-record: 594b040e ff403f80 35706c4e"), None);
        assert_eq!(parse_line("yk-record: 594b040e ff403f80 35706c4e 0400025"), None);
        assert_eq!(parse_line("sample: PASS"), None);
    }

    #[test]
    fn crcs_check_values() {
        assert_eq!(crc16_ccitt_false(b"123456789"), 0x29B1);
        assert_eq!(crc32_ieee_update(0, b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32_ieee_update(crc32_ieee_update(0, b"1234"), b"56789"), 0xCBF4_3926);
    }

    /// CASE_SET: the i.MX8MP default set as the walker now counts it (49
    /// listed, case 16 skipped by name, one case failing; 44 stands in for
    /// the case the earlier records could not name), its words, decode and
    /// round trip; then two failures and more, and a clean set with a skip.
    #[test]
    fn set_summary_words_and_their_decode() {
        let s = SetSummary {
            passed: 47,
            run: 48,
            skipped: 1,
            first_fail: Some(44),
            second_fail: None,
            first_skip: Some(16),
        };
        let w = s.words();
        assert_eq!(w, [0x002c_0107, 0xffff_002f, 0x0010_0030]);
        assert_eq!(SetSummary::from_words(w[0], w[1], w[2]), s);
        assert!(s.consistent() && !s.passed_all());
        let b = text(|b| describe(b, [w0(kind::CASE, 14), w[0], w[1], w[2]]));
        assert_eq!(
            b.as_str(),
            "CASE (1) seq 14: set status 7 (set failed) passed 47 of 48 run; skipped 1, first skip case 16; \
             failing 1: first case 44, second case none"
        );
        let four = SetSummary { passed: 44, second_fail: Some(2), ..s };
        let w = four.words();
        let b = text(|b| describe(b, [w0(kind::CASE, 14), w[0], w[1], w[2]]));
        assert!(
            b.as_str().ends_with("failing 4: first case 44, second case 2, 2 more not in the record"),
            "{}",
            b.as_str()
        );
        let clean = SetSummary { passed: 48, first_fail: None, ..s };
        assert!(clean.consistent() && clean.passed_all());
        let w = clean.words();
        assert_eq!(w, [0xffff_0107, 0xffff_0030, 0x0010_0030]);
        let b = text(|b| describe(b, [w0(kind::CASE, 14), w[0], w[1], w[2]]));
        assert!(
            b.as_str().contains("(set passed) passed 48 of 48 run; skipped 1, first skip case 16"),
            "{}",
            b.as_str()
        );
        // fields that disagree are said to: a failure named while all passed,
        // a skip count without its case, nothing run
        assert!(!SetSummary { first_fail: Some(3), ..clean }.consistent());
        assert!(!SetSummary { first_skip: None, ..clean }.consistent());
        assert!(!SetSummary { passed: 0, run: 0, first_fail: None, ..clean }.passed_all());
        let w = SetSummary { first_skip: None, ..clean }.words();
        assert!(text(|b| describe(b, [w0(kind::CASE, 1), w[0], w[1], w[2]]))
            .as_str()
            .ends_with("(INCONSISTENT fields)"));
        // saturation: a case number never reads as none, counts stop at their fields
        let big = SetSummary { skipped: 300, first_fail: Some(0x1_0000), ..s }.words();
        assert_eq!((big[0] >> 16, (big[0] >> 8) & 0xff), (0xfffe, 0xff));
    }

    #[test]
    fn packers() {
        assert_eq!(case_w1(7, 1, CASE_PASS), 0x0007_0101);
        assert_eq!(case_w1(7, -1, CASE_CHECK), 0x0007_ff02);
        assert_eq!(case_w1(7, 300, CASE_CHECK), 0x0007_ff02);
        assert_eq!(rgb565([0xff, 0xff, 0xff]), 0xffff);
        assert_eq!(rgb565([13, 17, 24]), (1 << 11) | (4 << 5) | 3);
        let p = [[13, 17, 24]; 5];
        let w = probes_words(&p);
        assert_eq!(w[0], 0x0883_0883);
        assert_eq!(w[2] >> 16, crc16_ccitt_false(&[13, 17, 24].repeat(5)) as u32);
        assert_eq!(size_word(1024, 600), 0x0400_0258);
        // GLMARK2 with GLMARK2_SCENES: the i.MX8MP's 2026-10-04 counts (score
        // 39, 30 ran, 1 failed, 2 unsupported) with SYNTHETIC entries (22
        // failed in its fragment shader, 17 and 19 unsupported: a packing
        // example, not a reading of any run), on-screen at 800x600
        let o = Glmark2Outcome {
            score: 39,
            flags: 0,
            status: STATUS_FAIL,
            ran: 30,
            failed: 1,
            unsupported: 2,
            failed_entry: 22,
            why: GLMARK2_WHY_FRAGMENT_SHADER,
            unsupported_entries: [17, 19],
            w: 800,
            h: 600,
            gl_errors: 11,
            incomplete_draws: 3,
        };
        assert_eq!(
            glmark2_words(&o),
            [0x4496_0027, 0x2278_1093, 0x0320_0258],
            "without GLMARK2_GL the counters are not stored"
        );
        // with GLMARK2_GL: the 11 GL errors terrain raised before
        // GL_OES_texture_npot (tools/hostharness/glmark2/FINDINGS.md) and a
        // count past the field (40 draws), on-screen at 800x600
        let gl = Glmark2Outcome { flags: GLMARK2_GL, incomplete_draws: 40, ..o };
        let w = glmark2_words(&gl);
        assert_eq!(w, [0x4496_0027, 0x6278_1093, 0xfb25_8320]);
        assert_eq!(glmark2_size(w[1], w[2]), (800, 600));
        assert_eq!(glmark2_gl_counts(w[1], w[2]), Some([11, GLMARK2_GL_COUNT_MAX]), "40 saturates at 15");
        assert_eq!(glmark2_counts(w[0], w[1]), [39, 30, 1, 2], "the scene counts are unchanged");
        assert_eq!(glmark2_gl_counts(0x2278_1093, 0x0320_0258), None, "a record without GLMARK2_GL");
        assert_eq!(glmark2_size(0x2278_1093, 0x0320_0258), (800, 600));
        // a size that does not fit in 12 bits keeps the full size word and
        // drops the counters (not recorded), never a narrowed size
        let wide = Glmark2Outcome { w: 4096, ..gl };
        let w = glmark2_words(&wide);
        assert_eq!(w[1] >> 28, GLMARK2_SCENES, "GLMARK2_GL dropped");
        assert_eq!(w[2], size_word(4096, 600));
        assert_eq!((glmark2_size(w[1], w[2]), glmark2_gl_counts(w[1], w[2])), ((4096, 600), None));
        let zero = glmark2_words(&Glmark2Outcome { gl_errors: 0, incomplete_draws: 0, ..gl });
        assert_eq!(glmark2_gl_counts(zero[1], zero[2]), Some([0, 0]), "0 counted is not absent");
        let none = Glmark2Outcome {
            score: 70000,
            flags: GLMARK2_OFFSCREEN,
            status: STATUS_PASS,
            ran: 64,
            failed: 0,
            unsupported: 0,
            failed_entry: GLMARK2_ENTRY_NONE,
            why: GLMARK2_WHY_NO_ERROR,
            unsupported_entries: [GLMARK2_ENTRY_NONE, 200],
            w: 1920,
            h: 1080,
            gl_errors: 0,
            incomplete_draws: 0,
        };
        assert_eq!(
            glmark2_words(&none),
            [0xfc3f_ffff, 0x31fc_003f, 0x0780_0438],
            "score and counts saturate; an entry above 62 is none"
        );
        // why: upstream's own error lines (scene.cpp:314, 326, 335; model.cpp:384, 704)
        assert_eq!(glmark2_why(b"Error: Failed to add vertex shader from file a.vert:\n"), GLMARK2_WHY_VERTEX_SHADER);
        assert_eq!(glmark2_why(b"Error: Failed to add fragment shader from file b.frag:"), GLMARK2_WHY_FRAGMENT_SHADER);
        assert_eq!(glmark2_why(b"Error: Failed to link program created from files a and b:  x"), GLMARK2_WHY_LINK);
        assert_eq!(glmark2_why(b"Error: Could not open 3ds file 'horse.3ds'"), GLMARK2_WHY_FILE);
        assert_eq!(glmark2_why(b"Error: Failed to open 'x.png'"), GLMARK2_WHY_FILE);
        assert_eq!(glmark2_why(b"Error: GL_FRAMEBUFFER is not complete"), GLMARK2_WHY_OTHER);
        assert_eq!(glmark2_why(b"Warning: Failed to add vertex shader"), GLMARK2_WHY_NO_ERROR, "not an error line");
        assert_eq!(glmark2_why(b"[terrain] FPS: 0"), GLMARK2_WHY_NO_ERROR);
    }

    /// The two cluster rows of 2026-10-03, from their records' words: the
    /// Pi 4 (1920x720, 8395 frames in 60012 ms, draw 5.0) and the AM62P
    /// (1024x600, 5936 frames in 60016 ms, draw 9.9).
    #[test]
    fn throughput_of_two_panels() {
        let fp = |w: u32, h: u32| [w0(kind::FINGERPRINT, 1), 0, 3, size_word(w, h)];
        let perf = |draw: u32| [w0(kind::PERF, 0), 0, draw << 16 | 0x00aa, 0];
        let app = |frames: u32, ms: u32| [w0(kind::APP, 3), 7 << 8 | 1, frames, ms];
        let probes = [w0(kind::PROBES, 1), 0, 0, 0];
        let pi = [fp(1920, 720), perf(50), app(8395, 60012), probes];
        let t = Throughput::from_records(&pi).unwrap();
        assert_eq!((t.fps_x10(), t.mpx_per_s_x10(), t.ms_per_mpx_x100()), (1399, 1934, Some(362)));
        let line = text(|b| write_throughput(b, &t));
        assert_eq!(
            line.as_str(),
            "1920x720: 8395 frames in 60012 ms, 139.9 fps, 193.4 Mpx/s; draw 5.0 ms mean, 3.62 ms/Mpx"
        );
        let am = [fp(1024, 600), perf(99), app(5936, 60016), probes];
        let t = Throughput::from_records(&am).unwrap();
        let line = text(|b| write_throughput(b, &t));
        assert_eq!(
            line.as_str(),
            "1024x600: 5936 frames in 60016 ms, 98.9 fps, 60.8 Mpx/s; draw 9.9 ms mean, 16.11 ms/Mpx"
        );
        // An APP without the PERF page before it (board_finish's: w3 is the
        // GPU resets) is not a rate; neither is a run without a FINGERPRINT,
        // nor zero milliseconds.
        assert_eq!(Throughput::from_records(&[fp(1024, 600), app(5936, 0x0)]), None);
        assert_eq!(Throughput::from_records(&[perf(99), app(5936, 60016)]), None);
        assert_eq!(Throughput::from_records(&[fp(1024, 600), perf(99), app(5936, 0)]), None);
        assert_eq!(Throughput::from_records(&[fp(1024, 600), [w0(kind::PERF, 1), 0, 99, 0], app(5936, 60016)]), None);
    }

    #[test]
    fn describe_every_kind() {
        for k in KINDS {
            let d = text(|b| describe(b, [w0(k.value, 1), 0, 0, 0]));
            assert!(d.as_str().starts_with(k.name), "{}", d.as_str());
        }
        let d = text(|b| describe(b, [0x594B040E, 0xFF403F80, 0x35706C4E, 0x04000258]));
        assert_eq!(d.as_str(), "PICTURE (4) seq 14: centre 0xff403f80 crc32 0x35706c4e size 1024x600");
        // the layout before GLMARK2_SCENES: the i.MX8MP's common-set GM row, 2026-10-04 10:21
        let d = text(|b| describe(b, [0x594B120E, 0x0000_0027, 0x021E_0102, 0x0320_0258]));
        assert_eq!(
            d.as_str(),
            "GLMARK2 (18) seq 14: score 39 status 2 (fail) scenes ran 30 failed 1 unsupported 2 size 800x600 on-screen"
        );
        let d = text(|b| describe(b, [w0(kind::GLMARK2, 3), 697, 0x021f_0002, size_word(800, 600)]));
        assert_eq!(
            d.as_str(),
            "GLMARK2 (18) seq 3: score 697 status 2 (fail) scenes ran 31 failed 0 unsupported 2 size 800x600 on-screen"
        );
        // with GLMARK2_SCENES
        let d = text(|b| describe(b, [w0(kind::GLMARK2, 14), 0x4496_0027, 0x2278_1093, 0x0320_0258]));
        assert_eq!(
            d.as_str(),
            "GLMARK2 (18) seq 14: score 39 status 2 (fail) scenes ran 30 failed 1 unsupported 2 size 800x600 on-screen; \
             first failed: entry 22 (default list: terrain), why 2 (fragment shader); \
             unsupported: entry 17 (default list: \
             buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=false), \
             entry 19 (default list: \
             buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=true); \
             GL errors raised and draws that sampled an incomplete texture: not recorded"
        );
        // with GLMARK2_GL (glmark2_words' packing example): absent, zero and
        // saturated read apart
        let d = text(|b| describe(b, [w0(kind::GLMARK2, 14), 0x4496_0027, 0x6278_1093, 0xfb25_8320]));
        assert!(
            d.as_str().ends_with(
                "size 800x600 on-screen; first failed: entry 22 (default list: terrain), why 2 (fragment shader); \
                 unsupported: entry 17 (default list: \
                 buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=false), \
                 entry 19 (default list: \
                 buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=true); \
                 GL errors raised 11, draws that sampled an incomplete texture 15 or more"
            ),
            "{}",
            d.as_str()
        );
        let d = text(|b| describe(b, [w0(kind::GLMARK2, 1), 0xfc3f_ffff, 0x71fc_003f, 0x002d_0500]));
        assert!(
            d.as_str().ends_with(
                "size 1280x720 off-screen; first failed: none; unsupported: none, none; \
                 GL errors raised 0, draws that sampled an incomplete texture 0"
            ),
            "{}",
            d.as_str()
        );
        let d = text(|b| describe(b, [w0(kind::GLMARK2, 1), 0xfc3f_ffff, 0x31fc_003f, 0x0780_0438]));
        assert_eq!(
            d.as_str(),
            "GLMARK2 (18) seq 1: score 65535 status 1 (pass) scenes ran 63 failed 0 unsupported 0 size 1920x1080 off-screen; \
             first failed: none; unsupported: none, none; \
             GL errors raised and draws that sampled an incomplete texture: not recorded"
        );
        assert_eq!(classify([w0(kind::GLMARK2, 3), 697, 0, 0]), Class::Verdict, "the run's verdict");
        // the AM62P stop probe's record (d9e225eb, 2026-10-04)
        let d = text(|b| describe(b, [0x594B1402, 0x7897, 0, 0x0100_0000]));
        assert_eq!(
            d.as_str(),
            "GPU_STOP (20) tag 2: cause 2 (out of memory with the freelist at its maximum) fence 30871 GPU recoveries 0 list 1 KiB 0"
        );
        assert_eq!(classify([w0(kind::GPU_STOP, 2), 30871, 0, 0x0100_0000]), Class::Verdict, "the run's verdict");
        let d = text(|b| describe(b, [0, 0x53450003, 0, 0]));
        assert_eq!(d.as_str(), "NO RECORD (w0 = 00000000)");
        // APP: the stage, the status and the copy flag apart
        let d = text(|b| describe(b, [0x594B0203, APP_SHOWN_BY_COPY | 7 << 8 | 1, 3600, 60_000]));
        assert_eq!(d.as_str(), "APP (2) seq 3: stage 7 status 1 (pass) shown by copy frames 3600 app 0x0000ea60");
        let d = text(|b| describe(b, [0x594B0203, 7 << 8 | 2, 0, 0]));
        assert_eq!(d.as_str(), "APP (2) seq 3: stage 7 status 2 (fail) frames 0 app 0x00000000");
    }

    /// GL_ERROR (issue #814): the hash is FNV-1a (the published test
    /// vectors), each field decodes, and an error not kept says so.
    #[test]
    fn gl_error_words_and_their_decode() {
        assert_eq!(fnv1a32(b""), 0x811c_9dc5);
        assert_eq!(fnv1a32(b"a"), 0xe40c_292c);
        assert_eq!(fnv1a32(b"foobar"), 0xbf9c_f968);
        assert_eq!(fnv1a32(b"glTexImage2D"), 0xebee_a496);
        let w1 = 0x505 | 22 << GL_ERROR_TAG_SHIFT | 8 << GL_ERROR_KEPT_SHIFT;
        assert_eq!(w1, 0x0816_0505);
        let d = text(|b| describe(b, [w0(kind::GL_ERROR, 0), w1, fnv1a32(b"glTexImage2D"), 37]));
        assert_eq!(
            d.as_str(),
            "GL_ERROR (23) tag 0: error 0 of 8 kept: 0x505 (GL_OUT_OF_MEMORY) in the entry point with FNV-1a \
             0xebeea496, tag 22 (glmark2: entry 22 (default list: terrain)); GL errors raised 37"
        );
        let d = text(|b| describe(b, [w0(kind::GL_ERROR, 0), 0x01ff_0500, 0x1234_5678, 1]));
        assert_eq!(
            d.as_str(),
            "GL_ERROR (23) tag 0: error 0 of 1 kept: 0x500 (GL_INVALID_ENUM) in the entry point with FNV-1a \
             0x12345678, no tag; GL errors raised 1"
        );
        let d = text(|b| describe(b, [w0(kind::GL_ERROR, 3), 0x02ff_0000, 0, 2]));
        assert_eq!(d.as_str(), "GL_ERROR (23) tag 3: error 3: not kept (2 kept); GL errors raised 2");
        // the selector does not take another kind's bits
        for sel in [DEQP_SEL_FAILS, DEQP_SEL_PROBE, DEQP_SEL_STATUS, PERF_SEL, SWEEP_SEL_LEAKS, SWEEP_SEL_LIST] {
            assert_eq!(GL_ERROR_SEL & sel, 0, "0x{sel:x}");
        }
        assert_eq!(classify([w0(kind::GL_ERROR, 0), w1, 0, 37]), Class::Verdict);
    }
}
