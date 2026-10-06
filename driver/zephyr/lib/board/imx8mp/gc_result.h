/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gc_result.h -- a 128-bit result record that survives the SoC reset.
 *
 * A kexec'd Zephyr image on the Verdin iMX8M Plus has no UART, and the SoC
 * reset that takes the board back to Linux clears DDR and OCRAM. The four SNVS
 * low-power general purpose registers (LPGPR0..3, 0x30370090..0x9C) are in the
 * battery-backed domain and are the one place that keeps a value across that
 * reset. Neither U-Boot 2024.07 nor NXP TF-A lf-6.6.52-2.2.1 touches them, and
 * the board runs no OP-TEE. Linux reads them back through the snvs_lpgpr nvmem
 * device, /sys/bus/nvmem/devices/30370000.snvs:snvs-lpgpr0/nvmem.
 *
 * The record uses the same layout on every board. Only the transport differs:
 *
 *   w0  0x594B0000 | kind << 8 | seq    'YK' magic, written LAST
 *   w1..w3                             depend on the kind, see below
 *
 * Words 1..3 go first, then a barrier, then word 0. A reader that finds the
 * magic can trust what is under it. The reader zeroes all four words before
 * each jump, so a zero word 0 means "nothing was written this run" and a stale
 * record cannot pass for a fresh one.
 */
#ifndef GC_RESULT_H
#define GC_RESULT_H

#include <stdint.h>

/* Every number below is an alias of the record's one definition,
 * driver/rust/src/board/record.rs, through the header generated from it.
 * The words of each kind are described here for this board's readers; the
 * values are record.rs's. */
#include "yakogl_record.h"

#define GC_RESULT_MAGIC YK_RECORD_MAGIC

enum gc_result_kind {
	GC_RESULT_CASE  = YK_RECORD_KIND_CASE, /* one gl_test case */
	GC_RESULT_APP   = YK_RECORD_KIND_APP, /* an application run: Slint, Qt, ... */
	GC_RESULT_FAULT = YK_RECORD_KIND_FAULT, /* the fatal handler ran */
	GC_RESULT_PICTURE = YK_RECORD_KIND_PICTURE, /* a rendered picture, for comparing boards */
	GC_RESULT_PROGRESS = YK_RECORD_KIND_PROGRESS, /* how far a run got; any later record replaces it */
	GC_RESULT_PROBES = YK_RECORD_KIND_PROBES,   /* an application's probe points, for comparing boards */
	GC_RESULT_SWEEP = YK_RECORD_KIND_SWEEP,    /* 48 gl_test cases' results, two bits each */
	GC_RESULT_DEQP = YK_RECORD_KIND_DEQP,     /* a dEQP run's running tally */
	GC_RESULT_DEQP_CASES = YK_RECORD_KIND_DEQP_CASES, /* a short dEQP run, each case's own outcome */
	GC_RESULT_DEQP_FAILS = YK_RECORD_KIND_DEQP_FAILS, /* six failing cases' names, a page at a time */
	GC_RESULT_DEQP_STATUS = YK_RECORD_KIND_DEQP_STATUS, /* the first unusual status, as text */
	GC_RESULT_PERF = YK_RECORD_KIND_PERF,       /* an application's frame-time breakdown, a page at a time */
	GC_RESULT_WDOG = YK_RECORD_KIND_WDOG,       /* the WDOG1 self-arm: what it found and did */
	GC_RESULT_STAGE = YK_RECORD_KIND_STAGE,      /* a boot or sample stage reached (a breadcrumb) */
	GC_RESULT_FINGERPRINT = YK_RECORD_KIND_FINGERPRINT, /* the window's CRC-32 (record.rs) */
	GC_RESULT_DISPLAY = YK_RECORD_KIND_DISPLAY, /* the display chain scanning a window (record.rs) */
};

/* kind WDOG (gc_wdog_arm.c, CONFIG_GC_BOARD_WDOG_SELF_ARM): w0's low byte is
 * the tag, not a sequence:
 *   1 armed: a breadcrumb, replaced by any later record of the run
 *   2 refused: the image reset the SoC at once (a verdict)
 *   3 proof image: armed and no longer fed; WDOG1 must reset the SoC
 *     60 s after w3 (a verdict, rewritten when the sample has finished)
 *   4 proof image FAILED: still alive at the software backstop (a verdict)
 * w1 = WCR as found | WCR read back << 16;
 * w2 = flags (YD_IMX_WDOG_*: 1 was running, 2 cleared PDE, 4 root enabled,
 *      8 gate opened) | refusal (YD_IMX_WDOG_REFUSED_*) << 8 | WMCR as
 *      found << 16;
 * w3 = uptime in ms when the arming finished (tags 1..3), when the backstop
 *      fired (tag 4).
 * WCR as found tells the entries apart: WDE (bit 2) clear = nobody started
 * it; WT (bits 15:8) 0x77 = 60 s (U-Boot's autostart default); 0x3b = 30 s
 * (Linux under systemd, a kexec entry). */
#define GC_RESULT_WDOG_ARMED       YK_RECORD_WDOG_ARMED
#define GC_RESULT_WDOG_REFUSED     YK_RECORD_WDOG_REFUSED
#define GC_RESULT_WDOG_PROOF       YK_RECORD_WDOG_PROOF
#define GC_RESULT_WDOG_PROOF_ALIVE YK_RECORD_WDOG_PROOF_ALIVE

/* kind STAGE (gc_result_stage(); driver/docs/imx8mp-netboot-plan.md, "Board
 * run 2026-10-03"): a breadcrumb under gc_result_breadcrumb's rule (dropped
 * once a result record is stored, replaced by any later record), so the last
 * one left by a run that died names the stage it died in. It never touches
 * the sequence number: an image that reaches its verdict writes exactly the
 * records it wrote without them. w0's low byte is the stage, not a sequence:
 * a mark is written as a stage BEGINS (or, for "returned", as it ends);
 * w1 = the stage's value (a return code as a two's complement int32, or as
 * listed; 0 when none); w2 = uptime in ms; w3 = how many times the WDOG1
 * self-arm's 1 s timer has fed (gc_wdog_arm.c; 0 without the self-arm and
 * before POST_KERNEL). A w3 that stopped growing while w2 did not says the
 * timer interrupt was gone.
 *   0x11 PRE_KERNEL_1: media_early_clocks returned (CCM roots and gates)
 *   0x12 PRE_KERNEL_1: the CCM clock-control driver returned
 *   0x13 panel console stage 1 entered: next is its reachability check,
 *        then (if reachable) its first LCDIF read (gc_panel_console.c)
 *   0x14 panel console stage 1 returned; w1 = 1 took over a picture, 0 the
 *        LCDIF answered but showed nothing to adopt, 0x100 | why the LCDIF
 *        was not read (YD_LCDIF_UNREACH_*: 1 MEDIAMIX off, 2 MEDIA_APB
 *        root off, 4 CCGR93 closed, 8 the LCDIF APB clock off or in reset)
 *   0x15 PRE_KERNEL_1 done (PSCI, UART, the IRQ guard returned)
 *   0x16 PRE_KERNEL_2: the system clock driver returned
 *   0x20 POST_KERNEL began; the self-arm's feeder is started
 *   0x21 POST_KERNEL: GPIO up                   (priority 40 returned)
 *   0x22 POST_KERNEL: PWM and I2C up            (50)
 *   0x23 POST_KERNEL: media blk-ctrl and its power domains up (60, 61)
 *   0x24 POST_KERNEL: the DSI bridge device up  (75)
 *   0x25 POST_KERNEL: the display device returned (80)
 *   0x26 panel console stage 2 entered: next is imx8mp_display_on()
 *   0x27 panel console stage 2 returned; w1 = imx8mp_display_on()'s rc
 *   0x30 GPU device init entered (vivante,gc gc_init; blocks mapped)
 *   0x31..0x37 its power-up, entering sub-step 0x30 + n of
 *        gpu::vivante::power::step: 1 clock roots, 2 gates, 3 GPUMIX
 *        (GPU_SHARE_LOGIC) power-up, 4 the GPUMIX ADB400 handshake wait,
 *        5 GPU_3D power-up, 7 the settle delay
 *   0x38 the identity read entered (the first GPU register access)
 *   0x39 the GPU reset entered
 *   0x3a GPU device init returned; w1 = rc
 *   0x40 the sample's main() entered
 *   0x41 the GPU device is ready; w1 = 1 yes, 0 no
 *   0x43 gc_render_init returned; w1 = rc (its own steps in between are
 *        kind PROGRESS 0x100 | n records)
 *   0x44 gc_render_alloc returned; w1 = rc
 *   0x45 draw entered: next is eglInitialize
 *   0x46 the EGL surface and context are current; w1 = 1 yes, 0 no
 *   0x47 the program is built; w1 = 1 linked, 0 not
 *   0x48 eglSwapBuffers returned (the frame's fence); w1 = glGetError()
 *   0x4e imx8mp_display_on(window) returned; w1 = rc
 *   0x4f the sample ended without a result record; w1 = 1 pass, 0 fail
 * Every stage but 0x4f means the run died after that point. */
#define GC_RESULT_STAGE_MEDIA_EARLY_CLOCKS YK_RECORD_STAGE_MEDIA_EARLY_CLOCKS
#define GC_RESULT_STAGE_CCM               YK_RECORD_STAGE_CCM
#define GC_RESULT_STAGE_CONSOLE_ENTER     YK_RECORD_STAGE_CONSOLE_ENTER
#define GC_RESULT_STAGE_CONSOLE_DONE      YK_RECORD_STAGE_CONSOLE_DONE
#define GC_RESULT_STAGE_PRE_KERNEL_1_DONE YK_RECORD_STAGE_PRE_KERNEL_1_DONE
#define GC_RESULT_STAGE_SYS_CLOCK         YK_RECORD_STAGE_SYS_CLOCK
#define GC_RESULT_STAGE_POST_KERNEL       YK_RECORD_STAGE_POST_KERNEL
#define GC_RESULT_STAGE_GPIO              YK_RECORD_STAGE_GPIO
#define GC_RESULT_STAGE_PWM_I2C           YK_RECORD_STAGE_PWM_I2C
#define GC_RESULT_STAGE_MEDIA             YK_RECORD_STAGE_MEDIA
#define GC_RESULT_STAGE_BRIDGE            YK_RECORD_STAGE_BRIDGE
#define GC_RESULT_STAGE_DISPLAY           YK_RECORD_STAGE_DISPLAY
#define GC_RESULT_STAGE_CONSOLE_COLD      YK_RECORD_STAGE_CONSOLE_COLD
#define GC_RESULT_STAGE_CONSOLE_COLD_DONE YK_RECORD_STAGE_CONSOLE_COLD_DONE
#define GC_RESULT_STAGE_GPU_INIT          YK_RECORD_STAGE_GPU_INIT
#define GC_RESULT_STAGE_GPU_POWER         YK_RECORD_STAGE_GPU_POWER /* + gpu::vivante::power::step */
#define GC_RESULT_STAGE_GPU_IDENTIFY      YK_RECORD_STAGE_GPU_IDENTIFY
#define GC_RESULT_STAGE_GPU_RESET         YK_RECORD_STAGE_GPU_RESET
#define GC_RESULT_STAGE_GPU_INIT_DONE     YK_RECORD_STAGE_GPU_INIT_DONE
#define GC_RESULT_STAGE_MAIN              YK_RECORD_STAGE_MAIN
#define GC_RESULT_STAGE_GPU_READY         YK_RECORD_STAGE_GPU_READY
#define GC_RESULT_STAGE_RENDER_INIT_DONE  YK_RECORD_STAGE_RENDER_INIT_DONE
#define GC_RESULT_STAGE_ALLOC_DONE        YK_RECORD_STAGE_ALLOC_DONE
#define GC_RESULT_STAGE_DRAW              YK_RECORD_STAGE_DRAW
#define GC_RESULT_STAGE_EGL_CURRENT       YK_RECORD_STAGE_EGL_CURRENT
#define GC_RESULT_STAGE_PROGRAM           YK_RECORD_STAGE_PROGRAM
#define GC_RESULT_STAGE_SWAPPED           YK_RECORD_STAGE_SWAPPED
#define GC_RESULT_STAGE_DISPLAY_ON        YK_RECORD_STAGE_DISPLAY_ON
#define GC_RESULT_STAGE_ENDED             YK_RECORD_STAGE_ENDED

/* kind CASE, w1 = case << 16 | first bad check (255 = none) << 8 | status;
 * w2 = got, w3 = want, both 0xAABBGGRR as gl_test reads them back. On a pass,
 * w2 = checks made and w3 = 0. */
enum gc_result_case_status {
	GC_RESULT_CASE_PASS       = YK_RECORD_CASE_PASS,
	GC_RESULT_CASE_CHECK      = YK_RECORD_CASE_CHECK, /* a numbered check failed */
	GC_RESULT_CASE_NO_CHECK   = YK_RECORD_CASE_NO_CHECK, /* failed, but no check said so */
	GC_RESULT_CASE_UNFINISHED = YK_RECORD_CASE_UNFINISHED, /* the suite did not finish in time */
	GC_RESULT_CASE_NO_DEVICE  = YK_RECORD_CASE_NO_DEVICE, /* the GPU or render service did not come up */
	GC_RESULT_CASE_SET_FAIL   = YK_RECORD_CASE_SET_FAIL, /* the regression set: case = its first failure,
					* w2 = cases passed, w3 = cases run */
};

/* kind APP: w1 = stage << 8 | status (1 pass, 2 fail), w2 = frames presented,
 * w3 = defined by the application. kind FAULT: w1 = ESR, w2 = ELR, w3 = FAR,
 * the low 32 bits of each. */

/* kind PICTURE: w1 = the pixel at (W/2, H/2), GL rows bottom-up, as the bytes
 * glReadPixels(GL_RGBA, GL_UNSIGNED_BYTE) returns read as a little-endian word
 * (R | G << 8 | B << 16 | A << 24). w2 = CRC-32/IEEE (crc32_ieee, zlib's
 * crc32) over those four bytes of each sample at x = W/2, y = i * H / 24 for
 * i = 3..21 in ascending order: the rows a +-0.8 clip-space triangle covers,
 * so the clear colour does not enter it. w3 = W << 16 | H. Two boards' CRCs
 * can only agree when W and H do. */
/* kind PROBES (agreed with am62p-board, 2026-09-23): five probe points.
 *   w0 = 0x594B0600 | status        status in the low byte (1 pass, 2 fail),
 *                                   where the other kinds keep a sequence
 *   w1 = p0 | p1 << 16              each point as RGB565:
 *   w2 = p2 | p3 << 16                (R >> 3) << 11 | (G >> 2) << 5 | B >> 3
 *   w3 = p4 | crc << 16             crc = CRC-16/CCITT-FALSE (poly 0x1021, init
 *                                   0xffff, no reflection, no final xor; Python
 *                                   binascii.crc_hqx(data, 0xffff)) over the 15
 *                                   bytes R,G,B of p0..p4 at full 8 bits
 * The RGB565 words say which point differs; the CRC keeps the comparison exact.
 * gc_slint's five, in 1024x600 window coordinates, top-down: p0 bg-top-left
 * (40,20), p1 face-left (214,330), p2 face-right (810,330), p3 bg-bottom
 * (300,590), p4 track (47,330). */

/* kind SWEEP: gc_gl_test's sweep of 48 consecutive case numbers, written
 * after every case so that a run which hangs half way still says how far it
 * got. w0's low byte = cases finished; w1..w3 = two bits per case, case
 * first + i at bits 2*(i % 16) of word 1 + i / 16, where `first` is the
 * image's CONFIG_GC_GL_TEST_SWEEP_FIRST: 0 not reached, 1 PASS, 2 FAIL,
 * 3 skipped or no such case.
 *
 * With a selector (below) the sweep image runs one case alone and ends with
 * that case's kind CASE record instead (the first bad check, got and want).
 * Selector GC_RESULT_SWEEP_SEL_LEAKS runs the whole range, and selector
 * GC_RESULT_SWEEP_SEL_LIST + k the k-th list of the image's
 * CONFIG_GC_GL_TEST_SWEEP_LISTS; both end with the counters record: kind
 * SWEEP with w0's low byte GC_RESULT_SWEEP_TAG_LEAKS,
 * w1 = lost GPU work (yakogl_gpu_work_lost) << 16 | GPU resets << 8 |
 *      fence timeouts (each saturating),
 * w2 = which of first+32..first+47 passed << 16 | the first case that left a
 *      GL error unchecked, as an offset from first (0xff none) << 8 | how
 *      many cases did,
 * w3 = which of first..first+31 passed.
 * A case neither passed nor skipped (gc_gl_test's CONFIG_GC_GL_TEST_SKIP,
 * case 7) nor left out of the list failed. */
#define GC_RESULT_SWEEP_CASES    YK_RECORD_SWEEP_CASES
#define GC_RESULT_SWEEP_SEL_LEAKS YK_RECORD_SWEEP_SEL_LEAKS
#define GC_RESULT_SWEEP_SEL_LIST  YK_RECORD_SWEEP_SEL_LIST
#define GC_RESULT_SWEEP_TAG_LEAKS YK_RECORD_SWEEP_TAG_LEAKS

/* kind DEQP: a dEQP run's tally, rewritten after every case result, so a run
 * that hangs or faults still says how far it got. w0's low byte = seq;
 * w1 = passed | failed << 16; w2 = not supported | any other status << 16;
 * w3 = cases begun | the first failing case's name check << 16 (below;
 * 0xffff while none failed). begun greater than
 * passed + failed + not supported + other means the case at index begun - 1
 * never reached a result. */
/* kind DEQP_CASES: a dEQP run of at most 32 cases. w0's low byte = cases
 * begun; w1..w2 = two bits per case in the order dEQP RAN them, case i at bits
 * 2*(i % 16) of word 1 + i / 16: 0 no result (not begun, or begun and never
 * finished: the last begun one hung), 1 Pass, 2 Fail, 3 NotSupported or any
 * other status. w3 = the first failing case's name check << 16 | GPU resets
 * (recoveries, 4 bits) << 12 | fences that timed out (4 bits) << 8 | that
 * case's index (0xff while none failed; name check 0xffff then). The counters
 * tell a Fail from a kick that was lost and recovered (the read-back then
 * reads a target nothing drew); 0 on a board whose driver has none. An image
 * built with the deqp sample's CONFIG_DEQP_SNVS_ADDRESS packs w3 differently
 * (the target's address bits and the lost work; see that option's help). A
 * run that grows past 32 cases switches to kind DEQP.
 *
 * The order: dEQP runs a group's cases sorted by NAME (TestNode::addChild,
 * tcuTestCase.cpp, inserts every child in name order), not in the order the
 * group's source adds them. Index i is the i-th case dEQP ran. The name check
 * is CRC-16/CCITT-FALSE (poly 0x1021, init 0xffff) of the case's full name as
 * the log's #beginTestCaseResult line gives it, taken from the same log lines
 * as its result, so a reader matches it against the candidates' names rather
 * than trusting any assumed order. */
#define GC_RESULT_DEQP_CASES_MAX YK_RECORD_DEQP_CASES_MAX

/* kind DEQP_FAILS: names every failure of a big dEQP run, six per boot. The
 * reader's selector GC_RESULT_DEQP_SEL_FAILS | page << 8 | table index runs
 * the same filter as the index alone, and the record is w0's low byte = Fails
 * seen (saturating at 255); w1..w3 = the name checks (CRC-16/CCITT-FALSE of
 * the full name, as in kind DEQP_CASES) of the Fails numbered 6 * page ..
 * 6 * page + 5 in the order dEQP ran them, two to a word, the earlier in the
 * low half; 0xffff where there is none. Run pages 0, 1, ... until
 * 6 * page >= Fails seen. "Fail" here is any status other than Pass and
 * NotSupported, so a case that crashed, timed out or ended the run is named
 * too (the tally, kind DEQP, counts those as "other"). */
#define GC_RESULT_DEQP_SEL_FAILS YK_RECORD_DEQP_SEL_FAILS
/* Selector GC_RESULT_DEQP_SEL_PROBE | table index: the same filter, and at
 * the first result that is not Pass or NotSupported the sample reads five
 * pixels of the current framebuffer (the case's own picture, still there
 * when its result line is logged) and writes them as a kind PROBES record
 * with status 2: p0 the centre (W/2, H/2), p1..p4 the quarter points
 * (W/4, H/4), (3W/4, H/4), (W/4, 3H/4), (3W/4, 3H/4), GL rows (bottom-up),
 * W = H = CONFIG_DEQP_SURFACE_SIZE. What a failing case drew, for a log
 * that only carries its images as PNG. */
#define GC_RESULT_DEQP_SEL_PROBE YK_RECORD_DEQP_SEL_PROBE
/* kind DEQP_STATUS: selector GC_RESULT_DEQP_SEL_STATUS | table index runs the filter and, at the first
 * result that is not Pass, Fail or NotSupported (QualityWarning, CompatibilityWarning, ResourceError,
 * InternalError, Crash, Timeout, ...), records its status string: w0's low byte = the index of that case
 * in the run (saturating); w1..w3 = the first 12 bytes of the string, little-endian, zero-padded. */
#define GC_RESULT_DEQP_SEL_STATUS YK_RECORD_DEQP_SEL_STATUS
#define GC_RESULT_DEQP_FAILS_PER_PAGE YK_RECORD_DEQP_FAILS_PER_PAGE

/* kind PERF: selector GC_RESULT_PERF_SEL | exp << 3 | page runs the
 * application with performance experiment `exp` (application-defined; 0 =
 * none) and ends with page `page` of its measurement instead of its usual
 * record: w0's low byte = the selector's low byte; w1..w3 = six 16-bit
 * values, the first in the low half of w1. The pages were gc_slint's
 * (deleted in Step 1 of driver/docs/one-method.md; c8436f0d has its README). */
#define GC_RESULT_PERF_SEL YK_RECORD_PERF_SEL

#define GC_RESULT_PICTURE_ROWS   YK_RECORD_PICTURE_ROWS
#define GC_RESULT_PICTURE_FIRST  YK_RECORD_PICTURE_FIRST
#define GC_RESULT_PICTURE_LAST   YK_RECORD_PICTURE_LAST

/* kind PROGRESS: w1 = where, w2 = uptime in ms, w3 = how many progress
 * records this run has written. A run that wedges and is reset by the
 * watchdog leaves the last one, which is where it stopped. `where`:
 *   0x0A0        the image is running (PRE_KERNEL_1)
 *   0x0A1        the kernel is up and applications are starting
 *   0x100 | n    the GPU driver's init reached step n (gc_render.h)
 *   0x200 | n    the sample reached its own stage n
 */
#define GC_RESULT_WHERE_IMAGE      YK_RECORD_WHERE_IMAGE
#define GC_RESULT_WHERE_KERNEL     YK_RECORD_WHERE_KERNEL
#define GC_RESULT_WHERE_GPU_STEP   YK_RECORD_WHERE_GPU_STEP
#define GC_RESULT_WHERE_STAGE      YK_RECORD_WHERE_STAGE

/*
 * The selector: the one thing that travels the other way. Before a jump the
 * reader may write w0 = 0 and w1 = 0x53450000 | index ('SE' in the high half)
 * into the cleared record; the image reads it at PRE_KERNEL_1, before it
 * writes any record of its own, so one image can run whichever entry of a
 * table compiled into it the index names (the deqp sample's CONFIG_DEQP_CASE_TABLE).
 * gc_result_selector() returns the index, or -1 when no selector was written.
 */
#define GC_RESULT_SELECTOR_MAGIC YK_RECORD_SELECTOR_MAGIC

#ifdef CONFIG_GC_COMMON_RESULT_SNVS
int gc_result_selector(void);

/* Store one record. The last one stored before the reset is the one read. */
void gc_result_write(unsigned int kind, uint32_t w1, uint32_t w2, uint32_t w3);

/* The same, with `tag` in w0's low byte instead of the sequence number (the
 * PROBES record's status). */
void gc_result_write_tag(unsigned int kind, uint8_t tag, uint32_t w1, uint32_t w2,
			 uint32_t w3);

/* Store a PROGRESS record, unless a result record has already been stored:
 * a verdict is never overwritten by a breadcrumb. */
void gc_result_progress(uint32_t where);

/* Store a breadcrumb of another kind (`tag` in w0's low byte) under the same
 * rule as gc_result_progress: dropped once a result record is stored, and
 * replaced by any later record. */
void gc_result_breadcrumb(unsigned int kind, uint8_t tag, uint32_t w1, uint32_t w2,
			  uint32_t w3);

/* Store a kind STAGE breadcrumb (above): `stage` in w0's low byte, `value`
 * in w1, the uptime and the self-arm's feed count in w2 and w3. */
void gc_result_stage(uint8_t stage, uint32_t value);

static inline void gc_result_case(unsigned int gl_case, int bad_check, unsigned int status,
				  uint32_t got, uint32_t want)
{
	uint32_t check = (bad_check < 0 || bad_check > 254) ? 255u : (uint32_t)bad_check;

	gc_result_write(GC_RESULT_CASE,
			((uint32_t)gl_case & 0xffffu) << 16 | check << 8 | (status & 0xffu),
			got, want);
}
#else
static inline void gc_result_write(unsigned int kind, uint32_t w1, uint32_t w2, uint32_t w3)
{
	(void)kind; (void)w1; (void)w2; (void)w3;
}
static inline void gc_result_case(unsigned int gl_case, int bad_check, unsigned int status,
				  uint32_t got, uint32_t want)
{
	(void)gl_case; (void)bad_check; (void)status; (void)got; (void)want;
}
static inline void gc_result_progress(uint32_t where)
{
	(void)where;
}
static inline void gc_result_breadcrumb(unsigned int kind, uint8_t tag, uint32_t w1,
					uint32_t w2, uint32_t w3)
{
	(void)kind; (void)tag; (void)w1; (void)w2; (void)w3;
}
static inline void gc_result_stage(uint8_t stage, uint32_t value)
{
	(void)stage; (void)value;
}
static inline int gc_result_selector(void)
{
	return -1;
}
static inline void gc_result_write_tag(unsigned int kind, uint8_t tag, uint32_t w1,
				       uint32_t w2, uint32_t w3)
{
	(void)kind; (void)tag; (void)w1; (void)w2; (void)w3;
}
#endif

#endif /* GC_RESULT_H */
