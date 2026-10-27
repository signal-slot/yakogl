/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * yakogl_board.h -- the board layer: everything a sample needs besides EGL
 * and GL (driver/docs/one-method.md, section 2).
 *
 * The Raspberry Pi 4 (V3D), the Verdin AM62P (PowerVR) and the Verdin
 * iMX8M Plus (GC7000) run a sample the same way. Only five things differ per
 * board: the boot method, the display bring-up, the result sink, the
 * watchdog and the GPU backend. The first four live behind this header, one
 * implementation per board under driver/zephyr/lib/board/<board>/; the GPU
 * backend is YakoGL's. A sample that includes this header and EGL/GL builds
 * for all three boards without an #ifdef.
 *
 * Kconfig (driver/zephyr/lib/board/Kconfig):
 *   CONFIG_YAKOGL_BOARD             this layer; the board is chosen by the
 *                                   SoC (YAKOGL_BOARD_AM62P, _IMX8MP, _RPI4)
 *   CONFIG_YAKOGL_BOARD_LIFETIME_S  the watchdog bound: the board resets at
 *                                   most this long (plus one watchdog period)
 *                                   after boot, whatever the sample does
 *   CONFIG_YAKOGL_BOARD_ALIVE_S     board_watchdog_feed()'s window
 *   CONFIG_YAKOGL_BOARD_HOLD_S      board_finish(): hold after a pass
 *   CONFIG_YAKOGL_BOARD_HOLD_FAIL_S board_finish(): hold after a fail
 *   CONFIG_YAKOGL_BOARD_RECORD_SCREEN  Pi 4: the console ring frozen on the
 *                                   framebuffer at the end (the record screen)
 *   CONFIG_YAKOGL_BOARD_ANIMATE_S   read by samples: how long to animate after
 *                                   the checked frames
 *
 * The watchdog is armed by the board layer from SYS_INIT, before main() and
 * before the GPU and display drivers' init, so a hang there is covered too.
 * There is no arm call.
 *
 * The record (driver/docs/one-method.md, section 3): four 32-bit words,
 *   w0 = 0x594B0000 | kind << 8 | low,  w1, w2, w3,
 * `low` being a sequence number or, for the kinds whose definition says so,
 * a tag. The kinds and their words are defined once, in
 * driver/rust/src/board/record.rs; yakogl_record.h (generated from it) gives
 * the C names (YK_RECORD_KIND_*). Every record a board stores is also printed
 * on its console through printk, so no log level can drop it:
 *   "yk-record: %08x %08x %08x %08x\n"
 * The last such line of a run is its verdict. Where a record is kept:
 *   i.MX8MP  the SNVS LPGPR0..3 (survives the SoC reset) + the line
 *   AM62P    the line only (UART)
 *   Pi 4     the line (UART), and the last line of the record screen
 *
 * Contexts: "main" means a thread (main() or one it started). "any" means
 * also a SYS_INIT at PRE_KERNEL_1 or later, an ISR or the fatal handler.
 * None of these functions may be called before PRE_KERNEL_1.
 *
 * EGL: eglGetDisplay(EGL_DEFAULT_DISPLAY) on every board. The Pi 4's V3D
 * OSAL needs the firmware clock switch; the board layer hands it over
 * (osal_zephyr_v3d_default_platform, rpi4/board_rpi4.c), so a sample passes
 * no board argument.
 *
 * Rust reaches the same symbols with `extern "C"`.
 */
#ifndef YAKOGL_BOARD_H
#define YAKOGL_BOARD_H

#include <stddef.h>
#include <stdint.h>
#include <zephyr/toolchain.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * A window: a GPU-visible buffer of w x h pixels whose rows are `stride`
 * bytes apart: w * 4, or, with CONFIG_YAKOGL_BOARD_SCANOUT_VIEW, the scan-out
 * buffer's own pitch when the window is a view into it (YakoGL takes any
 * even whole-pixel pitch of at least the width for a window, src/gl/egl.rs
 * native_window_pitch; hand `stride` to gles_native_window.stride_bytes as
 * is). A sample that reads the window back walks rows of `stride` bytes.
 *   phys    physical address, for struct gles_native_window.phys
 *   cpu     the CPU's view of the same memory (for read-back and checks).
 *           Normal non-cacheable on every board, so neither the sample nor
 *           the GPU needs a cache flush or invalidate for it:
 *             AM62P    an NC alias of the static buffer (k_mem_map_phys_bare)
 *             i.MX8MP  the carve-out's NC mapping (gc_render.c)
 *             Pi 4     an NC alias of the static buffer, or the
 *                      framebuffer's NC mapping for a view (rpi_mbox.c)
 *   format  the pixel layout the screen shows (copy it into
 *           gles_native_window.format as is): GLES_NATIVE_FORMAT_XRGB8888
 *           on the Verdins; on the Pi 4 the firmware framebuffer's pixel
 *           order as the firmware answered it (RGB: RGBA8888, BGR:
 *           XRGB8888; printed at boot); plus GLES_NATIVE_WINDOW_BOTTOM_UP
 *           when this board's display scans the rows bottom first (then GL
 *           draws y-up, as into an FBO); plus GLES_NATIVE_WINDOW_SCANOUT
 *           on a scan-out view (below): the window is the buffer the
 *           display scans, so a sample creates its EGL surface with
 *           EGL_RENDER_BUFFER = EGL_SINGLE_BUFFER (or PIPELINED); under the
 *           default EGL_BACK_BUFFER eglCreateWindowSurface refuses it
 *           (EGL_BAD_MATCH), as a frame would show while it is drawn
 *
 * A window may be split by the sample into screen-sized parts (two halves
 * for a double buffer: the same stride, `phys` and `cpu` moved by whole
 * rows); board_show() takes such a part as it takes the whole window. A
 * sample that does so tells EGL with GLES_NATIVE_WINDOW_PIPELINED in the
 * native window's format, and then a swap may return with its frame still
 * rendering (the half shown is the other one). Without the flag a swap
 * completes the frame, so a sample that shows its one window right after
 * eglSwapBuffers shows a finished picture (eglplatform.h).
 */
struct board_window {
	uintptr_t phys;
	void *cpu;
	uint32_t w, h, stride, format;
};

/*
 * CONFIG_YAKOGL_BOARD_SCANOUT_VIEW (default n, until all three boards have
 * passed with it): a window smaller than the screen is a VIEW into the
 * buffer the display scans, at its (0,0) and with its pitch, and
 * board_show() of it copies nothing:
 *   Pi 4     a view into the firmware framebuffer when w <= its width and h
 *            <= its rows in memory (the pitch is the framebuffer's; with
 *            CONFIG_YAKOGL_BOARD_RPI4_FB_VIRTUAL_HEIGHT two screens, which
 *            hold a double buffer); board_show() of the part at row y pans
 *            the screen to y (no call when it starts there already) and
 *            moves the console beside or under the window
 *   i.MX8MP  a view into a panel-sized carve-out buffer (the LCDIF's PITCH
 *            is the panel's, a multiple of 64 bytes as the LCDIF needs,
 *            ip/lcdifv3.rs); board_show() points the LCDIF at it
 *   AM62P    unchanged, because there is no copy to remove: board_show() is
 *            a DSS application-plane flip by address with the window's own
 *            stride (common/sample_glue.c plane_init, VID_ROW_INC), and the
 *            AM62P cluster run (image 7aa7e6f2, 2026-10-03) measured
 *            board_show at 0.0 ms mean and worst over 5971 frames
 * A window of the screen's size is scanned in place on the Verdins with or
 * without it (the DSS flip; the LCDIF pointed at the carve-out). Without
 * it, or when the view does not fit, a smaller window is a buffer of its
 * own and board_show() copies it; every copy is counted
 * (board_show_copies, the APP record's YK_RECORD_APP_SHOWN_BY_COPY).
 */

/*
 * Get the window, w x h. Called once per run, from main. A sample may ask
 * for more rows than it hands EGL (guard rows after the picture, which the
 * GPU must leave alone, or a second panel-sized part): rows are packed, so
 * gles_native_window.height may be smaller than h with the same phys and
 * stride. Where the memory comes from is the board's business:
 *   AM62P    a static buffer in the image (no-init, cache-line aligned)
 *   i.MX8MP  the GPU carve-out (gc_render_alloc)
 *   Pi 4     a view into the firmware framebuffer when the window fits it
 *            as CONFIG_YAKOGL_BOARD_SCANOUT_VIEW says; any other window is
 *            a static buffer in the image, which board_show() copies
 * Returns 0 and fills *out; -EINVAL for a zero size or one larger than the
 * board can give; -ENOMEM when the memory is taken (a second call);
 * -ENODEV when the GPU carve-out is not up. On error *out is zeroed.
 */
int board_window(uint32_t w, uint32_t h, struct board_window *out);

/*
 * Put the window on the screen with its top-left corner at the screen's
 * (0, 0). Call it after the frame in it is complete (its fence signalled, or
 * eglWaitGL); main only. A view or a part of it, and on the Verdins a
 * window of the panel's size, is shown in place: the display is pointed at
 * it (the DSS or LCDIF flip, the Pi 4's pan), nothing is copied, and the
 * change takes effect at the display's next frame. Any other window (the
 * 1024x600 Slint window on a bigger panel without the view) is the
 * fallback: it is copied into a buffer of the screen's size, the rest of
 * the screen keeps what it had (the console); the first copy prints
 * "board: board_show copies ..." once, and board_show_copies() counts them
 * (board_finish's APP record then carries YK_RECORD_APP_SHOWN_BY_COPY).
 * Returns 0; -ENOTSUP when the board has no screen or cannot show this
 * window (a bottom-up window on a display that cannot flip, a window larger
 * than the screen); -ENODEV when the display did not come up; -EIO when
 * the pan or flip was not taken; another negative errno from the display
 * driver. A check reads the window, not the screen, so a sample goes on
 * after an error here.
 */
int board_show(const struct board_window *win);

/*
 * Which of the n buffers at phys[] (n <= 32) the display may still scan:
 * bit k for phys[k]. On a board whose board_show() does not wait for the
 * display to take the buffer (the AM62P: the DSS takes a new address at its
 * next frame end, and a later show replaces one not taken yet), the buffer
 * scanned and the one waiting are busy; one replaced before it was taken is
 * not. Too large is possible, too small never. Never waits. Context: the
 * thread that calls board_show(). Only on a board with
 * CONFIG_YAKOGL_BOARD_DISPLAY_BUSY; it is the OSAL's display_busy there.
 */
uint32_t board_display_busy(const uint64_t *phys, size_t n);

/*
 * How many board_show() calls copied the window (the fallback above) in
 * this run; 0 when every show was in place (a flip, a pan, a view). Context:
 * main.
 */
uint32_t board_show_copies(void);

/*
 * Store a result record: kind (YK_RECORD_KIND_*), w1..w3 as that kind
 * defines them; w0's low byte is the next sequence number. A result is never
 * replaced by a later breadcrumb (board_progress), only by another result.
 * Context: any. Never fails. Before the sink is mapped (PRE_KERNEL_1 0 on
 * the i.MX8MP) the store is skipped; the line is printed whenever printk
 * works.
 */
void board_result(unsigned int kind, uint32_t w1, uint32_t w2, uint32_t w3);

/*
 * The same with `tag` in w0's low byte instead of the sequence number, for
 * the kinds whose low byte is a tag (PROBES: the status; WDOG; SWEEP's
 * counters; DEQP_CASES, DEQP_STATUS, DEQP_FAILS; PERF; FAULT). Context: any.
 */
void board_result_tag(unsigned int kind, uint8_t tag, uint32_t w1, uint32_t w2, uint32_t w3);

/*
 * Store a breadcrumb (kind PROGRESS): w1 = where, w2 = uptime in ms, w3 =
 * how many breadcrumbs this run wrote. Dropped once a result was stored, so
 * a verdict is never replaced by a breadcrumb; a run that died leaves its
 * last breadcrumb, which says where it stopped. `where` as kind PROGRESS
 * defines it (YK_RECORD_WHERE_*; YK_RECORD_WHERE_STAGE | n is the sample's
 * own stage n). Context: any.
 */
void board_progress(uint32_t where);

/*
 * The index the runner asked this run for, read before the first record is
 * stored: the i.MX8MP SNVS selector (w0 = 0, w1 = 0x5345 << 16 | index).
 * -1 when none was written, and always -1 on the AM62P and the Pi 4 (a run
 * is chosen by its image there). Context: main (valid from PRE_KERNEL_1 on).
 */
int board_selector(void);

/*
 * "Still making progress." Once a sample has called this, the watchdog
 * feeder (a kernel timer) feeds only while the calls keep coming at most
 * CONFIG_YAKOGL_BOARD_ALIVE_S apart; after a longer gap the feeding stops and
 * the watchdog resets the board within one period. A sample that never calls
 * it is fed until the lifetime. The feeder stops at
 * CONFIG_YAKOGL_BOARD_LIFETIME_S whatever the sample says.
 * Context: any; cheap (one store).
 */
void board_watchdog_feed(void);

/*
 * How many times the GPU driver reset a wedged GPU during this run (0 on a
 * driver without recovery). Context: main.
 */
uint32_t board_gpu_resets(void);

/*
 * End the run. Every path of a sample ends here, failures included; main
 * only. The same steps on every board:
 *  1. The verdict: rc == 0 is a pass, unless board_gpu_resets() != 0 (a
 *     recovered hang is never a pass).
 *  2. Print "<name>: PASS" or "<name>: FAIL rc=<rc>". Store a kind APP
 *     record (w1 = status: 1 pass, 2 fail, with YK_RECORD_APP_SHOWN_BY_COPY
 *     when board_show_copies() != 0; w2 = 0; w3 = the GPU resets)
 *     when the run stored no result record yet, or when the GPU was reset.
 *     Otherwise the sample's own CASE/PICTURE/PROBES record is the verdict
 *     and is kept (it has the detail).
 *  3. Hold CONFIG_YAKOGL_BOARD_HOLD_S (pass) or _HOLD_FAIL_S (fail),
 *     feeding the watchdog, so the screen can be read and photographed.
 *  4. Reset the board: sys_reboot on both Verdins; on the Pi 4 the record
 *     screen is frozen and the feeding stops, so the PM watchdog resets it.
 * Does not return.
 */
FUNC_NORETURN void board_finish(const char *name, int rc);

#ifdef __cplusplus
}
#endif

#endif /* YAKOGL_BOARD_H */
