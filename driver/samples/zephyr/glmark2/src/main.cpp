/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * glmark2: glmark2-es2 on the board layer (driver/docs/glmark2.md). The
 * benchmark is upstream's, compiled unchanged: its option parser, scenes,
 * main loop, score, CanvasGeneric and GLStateEGL (EGL through YakoGL). This
 * file is the part a glmark2 flavour supplies per window system, plus the
 * run's end:
 *
 *   NativeStateBoard   glmark2's NativeState over yakogl_board.h: the native
 *                      window is a struct gles_native_window on
 *                      board_window(W, H); flip() is board_show(); the
 *                      watchdog is fed at every main-loop step.
 *   run()              the sequence of upstream's main.cpp (Options, Log,
 *                      CanvasGeneric over the two states, the scenes, the
 *                      banner, the benchmark loop, the score line), with the
 *                      arguments built in (CONFIG_GLMARK2_SIZE,
 *                      CONFIG_GLMARK2_ARGS).
 *   RecordingLoop      upstream's MainLoop, which prints every scene's line
 *                      itself; it also counts the scenes by outcome, because
 *                      upstream's score leaves a scene that failed or was
 *                      unsupported out of the mean (main-loop.cpp:115-120)
 *                      and so cannot tell "all ran" from "some ran".
 *   the frame's cost   after each scene's line, one "glmark2-frame:" line:
 *                      the mean microseconds per frame spent in glmark2's own
 *                      GL calls (clear, draw, update), in eglSwapBuffers (the
 *                      kick and the fence wait: YakoGL's swap waits for the
 *                      frame), in board_show, and the rest of glmark2's frame
 *                      time; on the V3D also the GPU job times of the last
 *                      job that completed during each frame, as the OSAL
 *                      kept them (osal_v3d_last_completed_job_us: a swap
 *                      submits its frame's job and returns with it running,
 *                      so that is normally the previous frame's), "absent"
 *                      when no job completed in the scene; on the PowerVR
 *                      the last completed kick's times (pvr_render_last_kick_us:
 *                      submit, MMU invalidate, post to geometry and fragment
 *                      fence seen with their error bars, fence to fence, the
 *                      wait), "absent" when the driver was built without
 *                      CONFIG_YAKOGL_DRV_GPU_PVR_KICK_TIMES; "absent" on the
 *                      GC7000. Measured with the cycle counter (k_cycle_get_64).
 *   the draw steps     after it, one "glmark2-steps:" line: the mean
 *                      microseconds per frame of each step the library times
 *                      inside a draw (src/draw_steps.rs, totalled by the
 *                      OSAL: osal_draw_stats), over the scene's frames;
 *                      "absent" when no draw reported any. The GL-call time
 *                      outside them (the backend's own draw, uploads,
 *                      glGenerateMipmap, the other calls) is the frame
 *                      line's gl less the steps' sum.
 *   the GPU memory     one "glmark2-mem:" line before each entry's setup
 *                      and one at its end: the OSAL's own line
 *                      (osal_mem_line: "osal(v3d) mem:", "osal(zephyr)
 *                      mem:", "osal(gc) mem:"), what is allocated, the frees
 *                      it holds and the largest allocation that can still
 *                      succeed. Live memory, held frees and fragmentation
 *                      read apart.
 *
 * The verdict: pass when no scene failed and the score is not 0. A scene
 * glmark2 reports unsupported (an extension it asks for is not claimed) is
 * its own outcome, never folded into pass or fail: it is counted in the
 * summary line and the record, and a pass says how many there were. It is stored as a kind
 * GLMARK2 record (driver/rust/src/board/record.rs, glmark2_words: the score,
 * the status, the scene counts, the size, and which benchmark-list entries did
 * not run: the first failed one with why its setup failed, from glmark2's
 * first error line, and the first two unsupported ones; each is also printed
 * as a "glmark2: entry N <scene>:" line; and the GL core's counters, the GL
 * errors raised and the draws that sampled an incomplete texture, 4 bits
 * each, also printed as a "glmark2: GL errors raised" line; they do not
 * change the verdict), the run's last record, which the i.MX8MP keeps
 * in its SNVS (it has no UART); board_finish then keeps it as the verdict.
 *
 * The first GL errors by name (issue #814): the GL core keeps the first
 * eight with the entry point that raised them and a tag, which this file sets
 * to the benchmark-list entry at each entry's setup (yakogl_gl_error_tag).
 * Each is printed as a "glmark2: GL error K of N:" line. A runner without a
 * console asks for one of them with the selector YK_RECORD_GL_ERROR_SEL | K
 * (the i.MX8MP's SNVS, tools/board.sh SEL): the run then stores a kind
 * GL_ERROR record for error K after GLMARK2, which becomes the run's last
 * record (record.rs, GL_ERROR: the code, the entry, the FNV-1a of the entry
 * point's name, every error raised as a 32-bit count).
 *
 * The per-scene values for a runner without a console (the i.MX8MP,
 * docs/notes/imx8mp-gm-record.md): with the selector YK_RECORD_GM_SEL | t <<
 * YK_RECORD_GM_SEL_T_SHIFT | n the benchmark list is cut to one page's
 * entries (an F page: entries 3n..3n+2; a P0 or P1 page: entry n), and after
 * GLMARK2 the run stores one kind GM_PAGE record, its last: the F page holds
 * each scene's FPS (the integer of upstream's result line) and frame count,
 * P0 and P1 the scene's picture value (P1 also its FPS and frames). tools/
 * board.sh imx8mp gm runs the pages and turns them back into the console's
 * result, glmark2-frame: and glmark2-picture: lines.
 *
 * The frame-breakdown record on the GC7000 (the i.MX8MP, docs/notes/
 * imx8mp-frame-floor.md, "The frame-breakdown record"): with the selector
 * YK_RECORD_PERF_SEL | page (experiment 0, page 0..3) each scene's frames
 * are also counted for that page (the frame, the swap, the display, the
 * GPU; src/gm_perf_page.h), one "glmark2-perf:" line per scene, and after
 * GLMARK2 the run stores the last scene's page as a kind PERF record, its
 * last. Meant for an image of one scene (CONFIG_GLMARK2_ARGS="-b <scene>").
 * Without the selector, and on the other GPUs, nothing of it runs.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>

#include <cerrno>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <stdexcept>
#include <string>
#include <vector>

/* upstream glmark2 (GLMARK2_SRC/src) */
#include "benchmark-collection.h"
#include "canvas-generic.h"
#include "gl-state-egl.h"
#include "log.h"
#include "main-loop.h"
#include "native-state.h"
#include "options.h"
#include "results-file.h"
#include "scene-collection.h"

#include <yakogl_board.h>
#include <yakogl_record.h>
extern "C" {
#include <yakogl_drivers.h> /* yd_record_glmark2_words, yd_record_glmark2_why, yd_record_gm_page_words */
}
#include "osal_internal.h" /* YakoGL src/osal: osal_mem_line (its own extern "C") */
extern "C" {
/* the GL core's counters for the record (issue #13), totals since start */
uint32_t yakogl_gl_errors_raised(void);         /* src/gl/context.rs */
uint32_t yakogl_gl_errors_recorded(void);       /* src/gl/context.rs */
uint32_t yakogl_incomplete_texture_draws(void); /* src/gl/draw.rs */
/* the first GL errors, by name (issue #814): src/gl/context.rs */
void yakogl_gl_error_tag(uint32_t tag);
uint32_t yakogl_gl_errors_kept(void);
int yakogl_gl_error_first(uint32_t i, uint32_t *code, uint32_t *tag, char *name, size_t cap);
}
#if defined(CONFIG_YAKOGL_GPU_V3D)
extern "C" {
#include "zephyr_v3d.h" /* YakoGL src/osal: osal_v3d_last_completed_job_us */
}
#elif defined(CONFIG_YAKOGL_DRV_GPU_PVR)
extern "C" {
#include <pvr_render.h>     /* pvr_render_last_kick_us, pvr_render_scene_begin */
#include <yakogl_drivers.h> /* struct yd_pvr_kick_us */
#include "pvr_backend.h"    /* pvr_backend_set_option, PVR_BACKEND_OPT_ONE_IN_FLIGHT */
}
#elif defined(CONFIG_YAKOGL_DRV_GPU_VIVANTE)
/* the frame-breakdown record (YK_RECORD_PERF_SEL) */
#define GM_PERF 1
#include "gm_perf_page.h"
extern "C" {
#include <gc_render.h> /* gc_render_perf: the driver's counters and HI_PROFILE */
/* the board layer's VSYNC wait (board_imx8mp.c; the GC7000 OSAL's .wait_vblank) */
int yakogl_zephyr_wait_vblank(int64_t timeout_us);
}
#endif

#ifndef __cpp_exceptions
#error "glmark2 is built with USE_EXCEPTIONS: CONFIG_CPP_EXCEPTIONS=y"
#endif

namespace {

/* ---- the frame's cost ---------------------------------------------------- */

/* Cycle totals over one scene's frames (reset when the scene has set up). */
struct FrameCost {
    uint64_t frames = 0;
    /* the OSAL's draw-step totals when the scene's frames started
     * (osal_draw_stats); steps_known false: none reported yet */
    bool steps_known = false;
    uint64_t steps0[OSAL_DRAW_STEPS] = {};
    uint32_t untimed0 = 0;
    uint64_t gl = 0, swap = 0, show = 0, frame = 0; /* cycles */
#if defined(CONFIG_YAKOGL_GPU_V3D)
    /* jobs completed during the scene's frames; the times of the last one
     * completed in each frame that completed any (job_frames of them) */
    uint64_t jobs = 0, multi_job_frames = 0, job_frames = 0;
    uint64_t prep_us = 0, bin_us = 0;
    /* render and total only from jobs whose end the OSAL waited for
     * (render_late 0): a job whose FRDONE was up before the OSAL looked has
     * an end stamp of that look, which is not the GPU's time */
    uint64_t exact_frames = 0, render_us = 0, job_total_us = 0;
    uint32_t done_before = 0; /* the last completed fence seen */
    /* the jobs completed during the swap of frame JOBS_LINE_FRAME (the
     * "glmark2-jobs:" line); jobs_at 0: that frame did not run */
    static constexpr uint64_t JOBS_LINE_FRAME = 10;
    static constexpr unsigned JOBS_LINE_MAX = 8;
    uint64_t jobs_at = 0;
    unsigned jobs_n = 0;
    struct osal_v3d_recent_job_us jobs_ring[JOBS_LINE_MAX] = {};
#elif defined(CONFIG_YAKOGL_DRV_GPU_PVR)
    /* The PowerVR render service's last completed kick after each swap
     * (pvr_render_last_kick_us; gpu::pvr::kick_times), summed over the
     * frames that completed one (kick_frames). A fence time is "bounded"
     * when the last read that missed it was at most KICK_BOUND_US before
     * the read that saw it; the others are upper bounds. */
    int kick_rc = 0; /* the reader's last answer (-ENOTSUP: not built) */
    uint64_t kicks = 0, multi_kick_frames = 0, kick_frames = 0;
    uint64_t submit_us = 0, mmu_us = 0, mmu_syncs = 0;
    uint64_t geom_frames = 0, kick_geom_us = 0, geom_err_us = 0, geom_bounded = 0;
    uint64_t kick_frag_us = 0, frag_err_us = 0, frag_bounded = 0;
    uint64_t prev_frames = 0, frag_frag_us = 0;
    /* The scene's first frame that completes a kick: its frag->frag
     * measures from the previous scene's last kick, across this scene's
     * setup (seconds), so it is left out (conditionals 0:0 read 2,421 us at
     * a 1,943 us frame with it, 2026-10-04). */
    bool first_kick_frame = true;
    uint64_t wait_frames = 0, wait_us = 0;
    uint32_t done_before = 0, lost = 0;
#endif
};
#if defined(CONFIG_YAKOGL_DRV_GPU_PVR)
constexpr uint32_t KICK_BOUND_US = 100;
#endif
FrameCost s_cost;

uint64_t cyc_to_us(uint64_t cyc)
{
    return cyc * 1000000u / sys_clock_hw_cycles_per_sec();
}

/* ---- the picture value --------------------------------------------------- */

/* What a scene's last frame looked like, as a few numbers that two boards can
 * compare with a tolerance (driver/docs/glmark2.md, "The picture value"):
 * five probe pixels, a crc16 over a 40x30 grid of pixels, the grid's mean
 * colour, and whether every sampled pixel is black. Read back with
 * glReadPixels after the scene's last update and before its last swap, so the
 * pixels are that frame's. The host harness (tools/hostharness/glmark2/
 * host_main.cpp) has the same code. */
struct Picture {
    static constexpr int GRID_X = 40, GRID_Y = 30, PROBES = 5;
    bool valid = false;
    int width = 0, height = 0;
    uint32_t probe[PROBES] = {}; /* RGBA, R in the top byte */
    uint16_t crc = 0;            /* crc16 of the grid's RGB, exact */
    uint16_t crc_q = 0;          /* the same with each channel >> 4 */
    uint32_t mean[3] = {};       /* mean R, G, B over the grid */
    bool black = true;           /* every sampled pixel has R = G = B = 0 */
};

/* CRC-16/CCITT-FALSE (poly 0x1021, init 0xffff), one byte at a time. */
uint16_t crc16_byte(uint16_t crc, uint8_t b)
{
    crc ^= static_cast<uint16_t>(b << 8);
    for (int i = 0; i < 8; i++) {
        crc = (crc & 0x8000u) ? static_cast<uint16_t>((crc << 1) ^ 0x1021u)
                              : static_cast<uint16_t>(crc << 1);
    }
    return crc;
}

/* Row y of the window (0 is the top row) as RGBA bytes, by glReadPixels. One
 * row per call keeps the buffer small; glReadPixels counts rows from the
 * bottom. False when GL raised an error. The row is cleared first: a read
 * after lost GPU work writes no rows and raises no error, and the previous
 * row's bytes must not pass as that picture (it reads as black). */
bool gl_picture_row(int w, int h, int y, std::vector<uint8_t> &row)
{
    row.assign(static_cast<size_t>(w) * 4u, 0);
    glReadPixels(0, h - 1 - y, w, 1, GL_RGBA, GL_UNSIGNED_BYTE, row.data());
    return glGetError() == GL_NO_ERROR;
}

/* `picture_row(y, row)` fills row y (RGBA bytes) and says whether it could. */
template <typename RowReader>
Picture capture_picture(int w, int h, RowReader picture_row)
{
    Picture p;
    p.width = w;
    p.height = h;
    if (w < Picture::GRID_X || h < Picture::GRID_Y) {
        return p;
    }
    std::vector<uint8_t> row;
    /* an error the scene left is not the readback's */
    while (glGetError() != GL_NO_ERROR) {
    }
    /* the probes: the centre, then the four quarter points */
    const int px[Picture::PROBES] = {w / 2, w / 4, w * 3 / 4, w / 4, w * 3 / 4};
    const int py[Picture::PROBES] = {h / 2, h / 4, h / 4, h * 3 / 4, h * 3 / 4};
    for (int i = 0; i < Picture::PROBES; i++) {
        if (!picture_row(py[i], row)) {
            return p;
        }
        const uint8_t *q = &row[static_cast<size_t>(px[i]) * 4u];
        p.probe[i] = static_cast<uint32_t>(q[0]) << 24 | static_cast<uint32_t>(q[1]) << 16 |
                     static_cast<uint32_t>(q[2]) << 8 | q[3];
        p.black = p.black && !q[0] && !q[1] && !q[2];
    }
    /* the grid: the centres of 40x30 equal cells, row by row, left to right */
    uint16_t crc = 0xffffu, crc_q = 0xffffu;
    uint64_t sum[3] = {};
    for (int gy = 0; gy < Picture::GRID_Y; gy++) {
        if (!picture_row((2 * gy + 1) * h / (2 * Picture::GRID_Y), row)) {
            return p;
        }
        for (int gx = 0; gx < Picture::GRID_X; gx++) {
            const uint8_t *q = &row[static_cast<size_t>((2 * gx + 1) * w / (2 * Picture::GRID_X)) * 4u];
            for (int c = 0; c < 3; c++) {
                crc = crc16_byte(crc, q[c]);
                crc_q = crc16_byte(crc_q, static_cast<uint8_t>(q[c] >> 4));
                sum[c] += q[c];
            }
            p.black = p.black && !q[0] && !q[1] && !q[2];
        }
    }
    p.crc = crc;
    p.crc_q = crc_q;
    for (int c = 0; c < 3; c++) {
        p.mean[c] = static_cast<uint32_t>(sum[c] / (Picture::GRID_X * Picture::GRID_Y));
    }
    p.valid = true;
    return p;
}

#if defined(CONFIG_GLMARK2_FRAME_DUMP)
/* ---- the frame dump (CONFIG_GLMARK2_FRAME_DUMP) ---------------------------- */

/* The scene's last frame as text on the console, for a picture of what the
 * board drew (driver/docs/glmark2.md, "The frame dump"; decoded by
 * tools/cts/glmark2-fb.py):
 *   glmark2-fb: <scene> <w>x<h> begin
 *   glmark2-fb: <y> <crc16 of the row's RGB, hex> <base64 of the row's RGB888>
 *   glmark2-fb: end crc32 <hex>
 * the crc32 (IEEE) runs over all rows' RGB bytes, top row first. A row has its
 * own number and crc16, so a UART that lost or garbled a line costs that row. */
uint32_t crc32_update(uint32_t crc, const uint8_t *p, size_t n)
{
    while (n--) {
        crc ^= *p++;
        for (int i = 0; i < 8; i++) {
            crc = (crc >> 1) ^ (0xedb88320u & (0u - (crc & 1u)));
        }
    }
    return crc;
}

void frame_dump(const char *scene, int w, int h)
{
    static const char b64[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    std::vector<uint8_t> row, rgb(static_cast<size_t>(w) * 3u);
    std::string text;
    uint32_t crc = 0xffffffffu;
    printk("glmark2-fb: %s %dx%d begin\n", scene, w, h);
    for (int y = 0; y < h; y++) {
        /* The dump takes minutes at the console's speed, far longer than
         * CONFIG_YAKOGL_BOARD_ALIVE_S: without a feed per row the board
         * watchdog stops being fed and resets the SoC part way through. */
        board_watchdog_feed();
        if (!gl_picture_row(w, h, y, row)) {
            printk("glmark2-fb: end absent (the readback failed at row %d)\n", y);
            return;
        }
        uint16_t c16 = 0xffffu;
        for (int x = 0; x < w; x++) {
            for (int c = 0; c < 3; c++) {
                rgb[static_cast<size_t>(x) * 3u + c] = row[static_cast<size_t>(x) * 4u + c];
            }
        }
        for (uint8_t b : rgb) {
            c16 = crc16_byte(c16, b);
        }
        crc = crc32_update(crc, rgb.data(), rgb.size());
        text.clear();
        for (size_t i = 0; i < rgb.size(); i += 3) {
            const size_t left = rgb.size() - i;
            const uint32_t v = static_cast<uint32_t>(rgb[i]) << 16 |
                               (left > 1 ? static_cast<uint32_t>(rgb[i + 1]) << 8 : 0u) |
                               (left > 2 ? rgb[i + 2] : 0u);
            text += b64[v >> 18 & 63u];
            text += b64[v >> 12 & 63u];
            text += left > 1 ? b64[v >> 6 & 63u] : '=';
            text += left > 2 ? b64[v & 63u] : '=';
        }
        printk("glmark2-fb: %d %04x %s\n", y, static_cast<unsigned>(c16), text.c_str());
    }
    printk("glmark2-fb: end crc32 %08x\n", static_cast<unsigned>(~crc));
}
#endif

/* The picture line's text after "glmark2-picture: <scene>: ". */
void picture_text(const Picture &p, char *out, size_t n)
{
    if (!p.valid) {
        snprintf(out, n, "absent (the readback failed)");
        return;
    }
    snprintf(out, n,
             "%dx%d; probes c %08x, tl %08x, tr %08x, bl %08x, br %08x; crc16 %04x, crc16q %04x; "
             "mean %u %u %u; black %s",
             p.width, p.height, static_cast<unsigned>(p.probe[0]), static_cast<unsigned>(p.probe[1]),
             static_cast<unsigned>(p.probe[2]), static_cast<unsigned>(p.probe[3]),
             static_cast<unsigned>(p.probe[4]), static_cast<unsigned>(p.crc),
             static_cast<unsigned>(p.crc_q), static_cast<unsigned>(p.mean[0]),
             static_cast<unsigned>(p.mean[1]), static_cast<unsigned>(p.mean[2]),
             p.black ? "yes" : "no");
}

/* ---- the native state: the board's window and screen -------------------- */

class NativeStateBoard : public NativeState
{
public:
    explicit NativeStateBoard(const struct board_window &win) : win_(win)
    {
        nw_.phys = win.phys;
        nw_.width = win.w;
        nw_.height = win.h;
        nw_.stride_bytes = win.stride;
        nw_.format = win.format;
    }

    bool init_display() override { return true; }

    /* EGL_DEFAULT_DISPLAY: the board layer hands each OSAL its platform */
    void *display() override { return nullptr; }

    bool create_window(WindowProperties const &p) override
    {
        /* The window is the board's buffer, sized once from
         * CONFIG_GLMARK2_SIZE before the options are parsed; a different
         * size (--size or --fullscreen in CONFIG_GLMARK2_ARGS) cannot be
         * honoured. */
        if (p.fullscreen || p.width != static_cast<int>(win_.w) ||
            p.height != static_cast<int>(win_.h)) {
            Log::error("glmark2: the board window is %ux%u; %dx%d%s was asked\n",
                       win_.w, win_.h, p.width, p.height,
                       p.fullscreen ? " (fullscreen)" : "");
            return false;
        }
        created_ = true;
        return true;
    }

    void *window(WindowProperties &p) override
    {
        if (!created_) {
            p = WindowProperties();
            return nullptr;
        }
        p = WindowProperties(win_.w, win_.h, false, 0, {});
        return &nw_;
    }

    void visible(bool) override {}

    /* MainLoop::step() asks this once per step, on- and off-screen. */
    bool should_quit() override
    {
        board_watchdog_feed();
        return false;
    }

    /* After eglSwapBuffers (CanvasGeneric::update, FrameEndSwap). A window
     * EGL double-buffers (EGL_BACK_BUFFER, the default where the platform
     * can show a buffer: gles_osal.h, present) is shown by the swap itself;
     * only a window rendered in place (EGL_SINGLE_BUFFER) is shown here. */
    void flip() override
    {
        const uint64_t t0 = k_cycle_get_64();
        int rc = 0;

        if (!egl_shows_known_) {
            EGLint rb = EGL_SINGLE_BUFFER;
            EGLDisplay d = eglGetCurrentDisplay();
            EGLSurface su = eglGetCurrentSurface(EGL_DRAW);

            egl_shows_ = d != EGL_NO_DISPLAY && su != EGL_NO_SURFACE &&
                         eglQuerySurface(d, su, EGL_RENDER_BUFFER, &rb) && rb == EGL_BACK_BUFFER;
            egl_shows_known_ = true;
            printk("glmark2: the window is %s\n",
                   egl_shows_ ? "double-buffered by EGL (shown by the swap)" : "rendered in place (shown here)");
        }
        if (!egl_shows_) {
            rc = board_show(&win_);
        }

        s_cost.show += k_cycle_get_64() - t0;

        if (rc != 0 && rc != show_rc_) {
            /* once per distinct error; the frames go on */
            printk("glmark2: board_show = %d (the window is not on the screen)\n", rc);
        }
        show_rc_ = rc;
        shown_ = shown_ || rc == 0;
    }

    bool shown() const { return shown_; }

private:
    struct board_window win_;
    struct gles_native_window nw_ {};
    /* Whether eglSwapBuffers shows the window (EGL_BACK_BUFFER): read from
     * the current draw surface at the first flip, when the surface exists. */
    bool egl_shows_ = false;
    bool egl_shows_known_ = false;
    int show_rc_ = 0;
    bool shown_ = false;
    bool created_ = false;
};

/* ---- the EGL state, timing the swap -------------------------------------- */

/* Upstream's GLStateEGL, with eglSwapBuffers timed (and, on the V3D, the
 * job times the OSAL kept for the frame's last job). */
class GLStateTimed : public GLStateEGL
{
public:
    void swap() override
    {
        const uint64_t t0 = k_cycle_get_64();
        GLStateEGL::swap();
        s_cost.swap += k_cycle_get_64() - t0;
#if defined(CONFIG_YAKOGL_GPU_V3D)
        struct osal_v3d_job_us j;
        osal_v3d_last_completed_job_us(&j);
        const uint32_t n = j.jobs - s_cost.done_before;
        s_cost.done_before = j.jobs;
        s_cost.jobs += n;
        if (s_cost.frames + 1 == FrameCost::JOBS_LINE_FRAME) {
            /* the jobs this swap completed: the last n of the OSAL's ring */
            struct osal_v3d_recent_job_us all[OSAL_V3D_RECENT_JOBS];
            const unsigned have = osal_v3d_recent_jobs_us(all, OSAL_V3D_RECENT_JOBS);
            unsigned take = n < have ? n : have;
            if (take > FrameCost::JOBS_LINE_MAX) {
                take = FrameCost::JOBS_LINE_MAX;
            }
            for (unsigned i = 0; i < take; i++) {
                s_cost.jobs_ring[i] = all[have - take + i];
            }
            s_cost.jobs_n = take;
            s_cost.jobs_at = s_cost.frames + 1;
        }
        s_cost.multi_job_frames += n > 1u;
        if (n != 0u) {
            s_cost.job_frames++;
            s_cost.prep_us += j.prep;
            s_cost.bin_us += j.bin;
            if (j.render_late == 0u) {
                s_cost.exact_frames++;
                s_cost.render_us += j.render;
                s_cost.job_total_us += j.total;
            }
        }
#elif defined(CONFIG_YAKOGL_DRV_GPU_PVR)
        struct yd_pvr_kick_us k {};
        FrameCost &c = s_cost;
        c.kick_rc = pvr_render_last_kick_us(&k);
        if (c.kick_rc == 0) {
            const uint32_t n = k.done - c.done_before;
            c.done_before = k.done;
            c.lost = k.lost;
            c.kicks += n;
            c.multi_kick_frames += n > 1u;
            if (n != 0u) {
                c.kick_frames++;
                c.submit_us += k.submit_us;
                c.mmu_us += k.mmu_us;
                c.mmu_syncs += k.mmu_synced;
                c.kick_frag_us += k.kick_frag_us;
                c.frag_err_us += k.frag_err_us;
                c.frag_bounded += k.frag_err_us <= KICK_BOUND_US;
                if (k.flags & YD_PVR_KICK_US_GEOM_SEEN) {
                    c.geom_frames++;
                    c.kick_geom_us += k.kick_geom_us;
                    c.geom_err_us += k.geom_err_us;
                    c.geom_bounded += k.geom_err_us <= KICK_BOUND_US;
                }
                if ((k.flags & YD_PVR_KICK_US_PREV) && !c.first_kick_frame) {
                    c.prev_frames++;
                    c.frag_frag_us += k.frag_frag_us;
                }
                c.first_kick_frame = false;
                if (k.flags & YD_PVR_KICK_US_WAITED) {
                    c.wait_frames++;
                    c.wait_us += k.wait_us;
                }
            }
        }
#endif
    }
};

/* ---- glmark2's error lines while a scene sets up ------------------------- */

/* The stream Log::init hands every log line to (upstream log.cc, extra_out):
 * while armed, it keeps why the first "Error: " line says a setup failed
 * (yd_record_glmark2_why). The upstream lines still go to std::cerr. */
class SetupErrors : public std::streambuf
{
public:
    void arm()
    {
        armed_ = true;
        why_ = YK_RECORD_GLMARK2_WHY_NO_ERROR;
        n_ = 0;
    }
    /* Ends the window: a last line without its newline counts too. */
    uint32_t disarm()
    {
        line_end();
        armed_ = false;
        return why_;
    }

protected:
    int_type overflow(int_type c) override
    {
        if (c == traits_type::eof()) {
            return traits_type::not_eof(c);
        }
        if (c == '\n') {
            line_end();
        } else if (n_ < sizeof(line_)) {
            line_[n_++] = static_cast<char>(c);
        }
        return c;
    }

private:
    void line_end()
    {
        if (armed_ && why_ == YK_RECORD_GLMARK2_WHY_NO_ERROR && n_ != 0) {
            why_ = yd_record_glmark2_why(line_, n_);
        }
        n_ = 0;
    }

    bool armed_ = false;
    uint32_t why_ = YK_RECORD_GLMARK2_WHY_NO_ERROR;
    char line_[160];
    size_t n_ = 0;
};

SetupErrors s_setup_errors;
std::ostream s_setup_log(&s_setup_errors);

/* ---- the first GL errors, by name (issue #814) -------------------------- */

/* FNV-1a, 32 bits, of an entry point's name: record.rs fnv1a32, which the
 * decoder matches against the GL headers' names. */
uint32_t fnv1a32(const char *s)
{
    uint32_t h = 2166136261u;
    for (; *s != '\0'; s++) {
        h = (h ^ static_cast<uint8_t>(*s)) * 16777619u;
    }
    return h;
}

/* One line per GL error the GL core kept: its code, the entry point and the
 * benchmark-list entry that was running. */
void print_first_gl_errors(uint32_t raised)
{
    const uint32_t kept = yakogl_gl_errors_kept();
    for (uint32_t i = 0; i < kept; i++) {
        uint32_t code = 0, tag = 0;
        char name[48];
        if (yakogl_gl_error_first(i, &code, &tag, name, sizeof(name)) == 0) {
            printk("glmark2: GL error %u of %u: 0x%x in %s, entry %u\n", i + 1, raised, code, name, tag);
        }
    }
}

/* The selector YK_RECORD_GL_ERROR_SEL | K: store GL error K (0-based) as a
 * kind GL_ERROR record, tag K, after the GLMARK2 verdict, so a runner that
 * reads only the last record (the i.MX8MP) reads it. Without that selector
 * nothing is stored. */
void store_gl_error_record(uint32_t raised)
{
    const int sel = board_selector();
    if (sel < 0 || (static_cast<uint32_t>(sel) & ~0xffu) != YK_RECORD_GL_ERROR_SEL) {
        return;
    }
    const uint32_t k = static_cast<uint32_t>(sel) & 0xffu;
    const uint32_t kept = yakogl_gl_errors_kept();
    uint32_t code = 0, tag = YK_RECORD_GL_ERROR_TAG_NONE, hash = 0;
    char name[48];
    if (k < kept && yakogl_gl_error_first(k, &code, &tag, name, sizeof(name)) == 0) {
        hash = fnv1a32(name);
        if (tag > YK_RECORD_GL_ERROR_TAG_NONE) {
            tag = YK_RECORD_GL_ERROR_TAG_NONE;
        }
    }
    board_result_tag(YK_RECORD_KIND_GL_ERROR, static_cast<uint8_t>(k),
                     (code & 0xffffu) | tag << YK_RECORD_GL_ERROR_TAG_SHIFT |
                         kept << YK_RECORD_GL_ERROR_KEPT_SHIFT,
                     hash, raised);
}

/* ---- the per-scene page (YK_RECORD_GM_SEL) ------------------------------- */

/* The page the runner's selector asks for (record.rs, kind GM_PAGE): its tag
 * (the selector's low byte) and the list entries [first, end) the run is cut
 * to. Not asked without a GM selector, nor for GM_SEL_RATE (the cluster's). */
struct GmPageAsk {
    bool asked = false;
    uint32_t tag = 0;
    size_t first = 0, end = 0;
};

GmPageAsk gm_page_asked(size_t listed)
{
    GmPageAsk p;
    const int sel = board_selector();
    if (sel < 0 || (static_cast<uint32_t>(sel) & ~0xffu) != YK_RECORD_GM_SEL) {
        return p;
    }
    p.tag = static_cast<uint32_t>(sel) & 0xffu;
    const uint32_t t = p.tag >> YK_RECORD_GM_SEL_T_SHIFT, n = p.tag & YK_RECORD_GM_SEL_N_MASK;
    if (t == YK_RECORD_GM_SEL_F) {
        p.first = static_cast<size_t>(n) * YK_RECORD_GM_PAGE_SCENES;
        p.end = p.first + YK_RECORD_GM_PAGE_SCENES;
    } else if (t == YK_RECORD_GM_SEL_P0 || t == YK_RECORD_GM_SEL_P1) {
        p.first = n;
        p.end = static_cast<size_t>(n) + 1u;
    } else {
        return p;
    }
    p.asked = true;
    p.end = p.end < listed ? p.end : listed;
    return p;
}

/* After the verdict: the page, as the run's last record, so a runner that
 * keeps one record (the i.MX8MP) reads it. `scenes` are the cut list's, in
 * order: up to three for an F page, one for a P page. */
void store_gm_page(const GmPageAsk &page, const std::vector<struct yd_gm_scene> &scenes)
{
    const bool f = (page.tag >> YK_RECORD_GM_SEL_T_SHIFT) == YK_RECORD_GM_SEL_F;
    const uint32_t n = static_cast<uint32_t>(f ? scenes.size() : (scenes.empty() ? 0u : 1u));
    uint32_t w[3];
    if (yd_record_gm_page_words(page.tag, scenes.data(), n, w) != 0) {
        printk("glmark2: GM page 0x%02x: %u scenes do not fit it, not stored\n", page.tag, n);
        return;
    }
    board_result_tag(YK_RECORD_KIND_GM_PAGE, static_cast<uint8_t>(page.tag), w[0], w[1], w[2]);
    printk("glmark2: GM page 0x%02x stored: %08x %08x %08x\n", page.tag, w[0], w[1], w[2]);
}

#if defined(GM_PERF)
/* ---- the frame-breakdown record (YK_RECORD_PERF_SEL) --------------------- */

/* The counters page 1 takes differences of: the GC7000 OSAL's waits for
 * kicks and its time in submits, the driver's 1 ms sleeps in
 * gc_render_wait, GPU busy time and completed kicks. Each field keeps its
 * source's width, so a difference is right across a 32-bit wrap. */
struct GcCounts {
    uint64_t wait_us = 0, busy_us = 0, submit_us = 0;
    uint32_t waits = 0, sleeps = 0, kicks = 0;

    static GcCounts now()
    {
        GcCounts c;
        struct gc_render_perf p {};
        osal_gc_wait_stats(&c.waits, nullptr, &c.wait_us, nullptr);
        osal_gc_gpu_stats(&c.submit_us, &c.busy_us, &c.kicks);
        gc_render_perf(&p);
        c.sleeps = p.wait_sleeps;
        return c;
    }

    GcCounts operator-(const GcCounts &o) const
    {
        GcCounts d;
        d.wait_us = wait_us - o.wait_us;
        d.busy_us = busy_us - o.busy_us;
        d.submit_us = submit_us - o.submit_us;
        d.waits = waits - o.waits;
        d.sleeps = sleeps - o.sleeps;
        d.kicks = kicks - o.kicks;
        return d;
    }
};

/* The page the selector asks for and the running scene's counts. */
struct PerfPage {
    bool asked = false;
    uint32_t page = 0;
    struct gm_perf_scene s {};
    uint64_t last_t0 = 0; /* the previous frame's t0; 0 before the scene's first */
    /* page 3: HI_PROFILE at the previous frame's start (TOTAL, IDLE,
     * READ_BYTES8, WRITE_BYTES8), hi_known false before the scene's first */
    bool hi_known = false;
    uint32_t hi[4] = {};
    /* page 1: the counters when the scene's frames started, and what the
     * last frame's picture readback (glReadPixels waits for the GPU) added */
    GcCounts c0, readback;
    /* page 2: the frame-to-frame times (gm_perf_page.h, GM_PERF_BUCKET_US) */
    uint32_t hist[GM_PERF_BUCKETS] = {};
    /* the last scene's page, stored after GLMARK2 */
    bool ready = false;
    uint32_t w[4] = {};
};
PerfPage s_perf;

/* The selector, read once before the benchmark loop: YK_RECORD_PERF_SEL |
 * page, experiment 0 (the tag's bits 3..7) sets s_perf.asked. False for
 * another experiment or a page this image does not have: the run is refused,
 * not run without the page. True otherwise, asked or not. */
bool perf_read_selector()
{
    const int sel = board_selector();
    if (sel < 0 || (static_cast<uint32_t>(sel) & ~0xffu) != YK_RECORD_PERF_SEL) {
        return true;
    }
    const uint32_t tag = static_cast<uint32_t>(sel) & 0xffu;
    if (tag >= GM_PERF_PAGES) {
        printk("glmark2: PERF selector 0x%x: experiment %u page %u is not one of this image "
               "(experiment 0, pages 0..%u)\n",
               static_cast<unsigned int>(sel), tag >> 3, tag & 7u, GM_PERF_PAGES - 1u);
        return false;
    }
    s_perf.asked = true;
    s_perf.page = tag;
    printk("glmark2: PERF page %u asked (the frame-breakdown record of the last scene)\n", tag);
    return true;
}

/* Before a scene's first frame. */
void perf_scene_begin()
{
    PerfPage &p = s_perf;
    p.s = gm_perf_scene{};
    p.last_t0 = 0;
    p.hi_known = false;
    memset(p.hist, 0, sizeof(p.hist));
    p.readback = GcCounts();
    p.c0 = GcCounts::now();
}

/* Page 3, before a frame's t0 (the register reads stay out of t0..t2): the
 * 32-bit HI_PROFILE counters wrap in about 5 s at 800 MHz, so they are read
 * every frame and the differences added. */
void perf_hi_sample()
{
    PerfPage &p = s_perf;
    struct gc_render_perf g {};
    gc_render_perf(&g);
    const uint32_t hi[4] = {g.hi_total_cycles, g.hi_idle_cycles, g.hi_read_bytes8, g.hi_write_bytes8};
    if (p.hi_known) {
        p.s.hi_total_cycles += hi[0] - p.hi[0];
        p.s.hi_idle_cycles += hi[1] - p.hi[1];
        p.s.read_bytes += static_cast<uint64_t>(hi[2] - p.hi[2]) * 8u;
        p.s.write_bytes += static_cast<uint64_t>(hi[3] - p.hi[3]) * 8u;
    }
    memcpy(p.hi, hi, sizeof(hi));
    p.hi_known = true;
}

/* At a frame's t0: the interval from the previous frame's t0. */
void perf_frame_start(uint64_t t0)
{
    PerfPage &p = s_perf;
    if (p.last_t0 != 0) {
        const uint64_t us = cyc_to_us(t0 - p.last_t0);
        const uint64_t b = us / GM_PERF_BUCKET_US;
        p.s.intervals++;
        p.s.interval_us += us;
        p.hist[b < GM_PERF_BUCKETS ? b : GM_PERF_BUCKETS - 1u]++;
    }
    p.last_t0 = t0;
}

/* T, the panel's period, for page 2: four VSYNC waits of the board layer
 * (yakogl_zephyr_wait_vblank; the latch thread owns the LCDIF's latch, so
 * the sample asks it rather than calling imx8mp_display_set_fb), timed, the
 * last three averaged: the first may end at the last show's latch. Measured
 * after the scene's last frame, so it is in no frame. 0 when the board has
 * no VSYNC wait or one did not come within 100 ms. */
uint32_t perf_period_us()
{
    uint64_t t[4];
    for (int i = 0; i < 4; i++) {
        const int rc = yakogl_zephyr_wait_vblank(100000);
        if (rc != 0) {
            printk("glmark2: PERF page 2: the VSYNC wait answered %d, T not measured\n", rc);
            return 0;
        }
        t[i] = k_cycle_get_64();
    }
    return static_cast<uint32_t>(cyc_to_us(t[3] - t[0]) / 3u);
}

/* After a scene that ran: its page, one line, kept for the record. */
void perf_scene_end(const char *scene)
{
    PerfPage &p = s_perf;
    const FrameCost &c = s_cost;
    p.s.frames = c.frames;
    p.s.frame_us = cyc_to_us(c.frame);
    p.s.gl_us = cyc_to_us(c.gl);
    p.s.swap_us = cyc_to_us(c.swap);
    p.s.show_us = cyc_to_us(c.show);
    const GcCounts d = GcCounts::now() - p.c0 - p.readback;
    p.s.wait_us = d.wait_us;
    p.s.waits = d.waits;
    p.s.sleeps = d.sleeps;
    p.s.busy_us = d.busy_us;
    p.s.kicks = d.kicks;
    p.s.submit_us = d.submit_us;
    if (p.page == 2u) {
        p.s.period_us = perf_period_us();
        gm_perf_bins(p.hist, GM_PERF_BUCKETS, GM_PERF_BUCKET_US, p.s.period_us, p.s.bins);
    }
    p.s.ticks_per_sec = CONFIG_SYS_CLOCK_TICKS_PER_SEC;
    uint32_t w[4];
    if (gm_perf_page_words(p.page, &p.s, w) != 0) {
        return; /* not reached: perf_page_asked took pages 0..3 only */
    }
    memcpy(p.w, w, sizeof(w));
    p.ready = true;
    printk("glmark2-perf: %s: page %u: %08x %08x %08x %08x\n", scene, p.page, w[0], w[1], w[2], w[3]);
}

/* After the verdict: the last scene's page as the run's last record, so a
 * runner that keeps one record (the i.MX8MP) reads it. */
void store_perf_page()
{
    const PerfPage &p = s_perf;
    if (!p.asked) {
        return;
    }
    if (!p.ready) {
        printk("glmark2: PERF page %u: no scene ran, not stored\n", p.page);
        return;
    }
    board_result_tag(YK_RECORD_KIND_PERF, static_cast<uint8_t>(p.page), p.w[1], p.w[2], p.w[3]);
    printk("glmark2: PERF page %u stored: %08x %08x %08x\n", p.page, p.w[1], p.w[2], p.w[3]);
}
#endif /* GM_PERF */

/* ---- the main loop, counting outcomes ----------------------------------- */

/* The counts and, for the record, which list entries did not run: the first
 * failed one with why, the first two unsupported ones; and each scene's
 * values as kind GM_PAGE carries them, in run order. */
struct Outcomes {
    unsigned int ran = 0, failed = 0, unsupported = 0;
    uint32_t failed_entry = YK_RECORD_GLMARK2_ENTRY_NONE;
    uint32_t why = YK_RECORD_GLMARK2_WHY_NO_ERROR;
    uint32_t unsupported_entries[2] = {YK_RECORD_GLMARK2_ENTRY_NONE, YK_RECORD_GLMARK2_ENTRY_NONE};
    std::vector<struct yd_gm_scene> scenes;
};

class RecordingLoop : public MainLoop
{
public:
    /* `entry_base`: the index in the full benchmark list of `benchmarks`'
     * first entry (a GM page runs a cut of it), so entries keep their
     * numbers in the lines and the record. */
    RecordingLoop(Canvas &canvas, const std::vector<Benchmark *> &benchmarks, size_t entry_base,
                  Outcomes &out)
        : MainLoop(canvas, benchmarks), out_(out), entry_base_(entry_base)
    {
    }

    /* Upstream's MainLoop::draw (main-loop.cpp:129-138), timed: the GL calls
     * of the scene, then the canvas update (swap and flip, timed inside). */
    void draw() override
    {
#if defined(GM_PERF)
        if (s_perf.asked && s_perf.page == 3u) {
            perf_hi_sample();
        }
#endif
        const uint64_t t0 = k_cycle_get_64();
#if defined(GM_PERF)
        if (s_perf.asked) {
            perf_frame_start(t0);
        }
#endif
        canvas_.clear();
        scene_->draw();
        scene_->update();
        const uint64_t t1 = k_cycle_get_64();
        /* The scene's last frame (update() ended it and stopped the scene's
         * own clock): read it back before the swap. Its cycles are left out
         * of the frame's, so the picture is not in the timing. */
        uint64_t readback = 0;
        if (!scene_->running() && scene_setup_status_ == SceneSetupStatusSuccess) {
#if defined(GM_PERF)
            const GcCounts rb0 = s_perf.asked ? GcCounts::now() : GcCounts();
#endif
            picture_ = capture_picture(
                canvas_.width(), canvas_.height(), [&](int y, std::vector<uint8_t> &row) {
                    return gl_picture_row(canvas_.width(), canvas_.height(), y, row);
                });
#if defined(GM_PERF)
            if (s_perf.asked) {
                s_perf.readback = GcCounts::now() - rb0;
            }
#endif
#if defined(CONFIG_GLMARK2_FRAME_DUMP)
            if (picture_.valid) {
                frame_dump(scene_->info_string().c_str(), canvas_.width(), canvas_.height());
            }
#endif
            readback = k_cycle_get_64() - t1;
        }
#if defined(GM_PERF)
        const uint64_t present0 = s_cost.swap + s_cost.show;
#endif
        canvas_.update();
        const uint64_t t2 = k_cycle_get_64();
#if defined(GM_PERF)
        /* page 2 v4: the frame's present, the swap (which shows the window
         * EGL double-buffers) and the flip */
        if (s_perf.asked && cyc_to_us(s_cost.swap + s_cost.show - present0) >= 1000u) {
            s_perf.s.slow_presents++;
        }
#endif
        s_cost.gl += t1 - t0;
        s_cost.frame += t2 - t0 - readback;
        s_cost.frames++;
    }

    /* The scene's setup is the window in which an error line says why. */
    void before_scene_setup() override
    {
        /* the GL errors from here to the next entry's setup are this entry's */
        yakogl_gl_error_tag(entry());
        print_mem("before setup");
        s_setup_errors.arm();
    }
    void after_scene_setup() override { setup_why_ = s_setup_errors.disarm(); }

    /* After the scene set up: its frames start here. */
    void log_scene_info() override
    {
        MainLoop::log_scene_info();
#if defined(CONFIG_YAKOGL_DRV_GPU_PVR)
        pvr_render_scene_begin(); /* the grow line's scene (GROW_SCENE) */
#endif
        picture_ = Picture();
        const FrameCost fresh;
#if defined(CONFIG_YAKOGL_GPU_V3D) || defined(CONFIG_YAKOGL_DRV_GPU_PVR)
        const uint32_t done_before = s_cost.done_before;
        s_cost = fresh;
        s_cost.done_before = done_before;
#else
        s_cost = fresh;
#endif
        s_cost.steps_known = osal_draw_stats(s_cost.steps0, &s_cost.untimed0);
#if defined(GM_PERF)
        if (s_perf.asked) {
            perf_scene_begin();
        }
#endif
    }

    void log_scene_result() override
    {
#if defined(GM_PERF)
        /* first: the page's counters end with the scene's last frame */
        if (s_perf.asked && scene_setup_status_ == SceneSetupStatusSuccess && s_cost.frames != 0) {
            perf_scene_end(scene_->info_string().c_str());
        }
#endif
        MainLoop::log_scene_result(); /* upstream's own line, unchanged */
        print_cost();
        print_steps();
        print_picture();
        print_mem("end");
        keep_scene();
        const uint32_t entry = this->entry();
        switch (scene_setup_status_) {
        case SceneSetupStatusSuccess:
            out_.ran++;
            break;
        case SceneSetupStatusUnsupported:
            if (out_.unsupported < 2u) {
                out_.unsupported_entries[out_.unsupported] = entry;
            }
            out_.unsupported++;
            printk("glmark2: entry %u %s: unsupported\n", entry, scene_->info_string().c_str());
            break;
        default:
            if (out_.failed == 0u) {
                out_.failed_entry = entry;
                out_.why = setup_why_;
            }
            out_.failed++;
            printk("glmark2: entry %u %s: failed, why %u\n", entry, scene_->info_string().c_str(),
                   setup_why_);
            break;
        }
    }

private:
    /* The entry's index in the full benchmark list, option-only entries
     * counted. */
    uint32_t entry() const
    {
        return static_cast<uint32_t>(entry_base_ + static_cast<size_t>(bench_iter_ - benchmarks_.begin()));
    }

    /* The scene's values for kind GM_PAGE: the FPS integer upstream's result
     * line prints (main-loop.cpp:178, from the same Scene::stats()), or the
     * did-not-run code; the frames; the last frame's picture value (0 x 0
     * when the readback failed). A scene that set up but drew no frame has
     * no frame time: fps 0. */
    void keep_scene()
    {
        struct yd_gm_scene s {};
        switch (scene_setup_status_) {
        case SceneSetupStatusSuccess: {
            const double t = scene_->stats().average_frame_time;
            s.fps = t > 0.0 ? static_cast<unsigned>(ceil(1.0 / t)) : 0u;
            s.frames = static_cast<uint32_t>(s_cost.frames);
            break;
        }
        case SceneSetupStatusUnsupported:
            s.fps = YK_RECORD_GM_FPS_UNSUPPORTED;
            break;
        default:
            s.fps = YK_RECORD_GM_FPS_FAILED;
            break;
        }
        if (picture_.valid) {
            for (int i = 0; i < Picture::PROBES; i++) {
                s.probe[i] = picture_.probe[i];
            }
            s.crc16 = picture_.crc;
            s.crc16q = picture_.crc_q;
            for (int c = 0; c < 3; c++) {
                s.mean[c] = picture_.mean[c];
            }
            s.black = picture_.black ? 1u : 0u;
            s.w = static_cast<uint32_t>(picture_.width);
            s.h = static_cast<uint32_t>(picture_.height);
        }
        out_.scenes.push_back(s);
    }

    /* The OSAL's GPU memory at a scene change, one line: before the
     * entry's setup (what the previous scene's teardown left) and at its
     * end. The OSAL words it (osal_mem_line, its own prefix per OSAL): what
     * is allocated, the frees it holds and the largest allocation that can
     * still succeed, so live memory, held frees and fragmentation read
     * apart (the Pi 4's round 2: terrain's 415 refused tiled copies with 0
     * frees held). */
    void print_mem(const char *when)
    {
        char line[256];
        osal_mem_line(line, sizeof(line));
        printk("glmark2-mem: entry %u %s: %s\n", static_cast<unsigned int>(entry()), when, line);
    }

    /* One line per scene that ran: the last frame's picture value. */
    void print_picture()
    {
        if (scene_setup_status_ != SceneSetupStatusSuccess) {
            return;
        }
        char text[256];
        picture_text(picture_, text, sizeof(text));
        printk("glmark2-picture: %s: %s\n", scene_->info_string().c_str(), text);
    }

    /* The library's draw steps over the scene's frames (src/draw_steps.rs
     * Step order; the GC7000's own steps are 0 on the other GPUs). */
    void print_steps()
    {
        static_assert(OSAL_DRAW_STEPS == 12u, "a name for each step of src/draw_steps.rs");
        static const char *const name[OSAL_DRAW_STEPS] = {
            "program", "uniforms", "attribs", "textures", "state", "gc rest",
            "gc ds", "gc tiled copy", "gc frame", "gc uniforms", "gc vertex", "gc emit",
        };
        const FrameCost &c = s_cost;
        if (c.frames == 0) {
            return;
        }
        uint64_t now[OSAL_DRAW_STEPS] = {};
        uint32_t untimed = 0;
        printk("glmark2-steps: %s: ", scene_->info_string().c_str());
        if (!osal_draw_stats(now, &untimed)) {
            printk("absent (no draw reported its steps)\n");
            return;
        }
        uint64_t sum = 0;
        printk("us per frame:");
        for (unsigned int i = 0; i < OSAL_DRAW_STEPS; i++) {
            const uint64_t d = now[i] - (c.steps_known ? c.steps0[i] : 0u);
            sum += d;
            printk("%s %s %llu", i == 0 ? "" : ",", name[i], (unsigned long long)(d / 1000u / c.frames));
        }
        printk("; sum %llu; untimed laps %u\n", (unsigned long long)(sum / 1000u / c.frames),
               untimed - (c.steps_known ? c.untimed0 : 0u));
    }

    void print_cost()
    {
        const FrameCost &c = s_cost;
        if (c.frames == 0) {
            return; /* a scene that did not run has no frames to describe */
        }
        const uint64_t n = c.frames;
        const uint64_t gl = cyc_to_us(c.gl) / n, swap = cyc_to_us(c.swap) / n,
                       show = cyc_to_us(c.show) / n, frame = cyc_to_us(c.frame) / n;
        const uint64_t inside = gl + swap + show;
        printk("glmark2-frame: %s: %llu frames; us per frame: gl %llu, swap %llu, show %llu, "
               "other %llu, frame %llu",
               scene_->info_string().c_str(), (unsigned long long)n, (unsigned long long)gl,
               (unsigned long long)swap, (unsigned long long)show,
               (unsigned long long)(frame > inside ? frame - inside : 0),
               (unsigned long long)frame);
#if defined(CONFIG_YAKOGL_GPU_V3D)
        if (c.job_frames == 0) {
            printk("; v3d completed jobs: absent (none completed in the scene)\n");
        } else {
            const uint64_t f = c.job_frames;
            printk("; v3d last completed job per frame: prep %llu, bin %llu",
                   (unsigned long long)(c.prep_us / f), (unsigned long long)(c.bin_us / f));
            if (c.exact_frames == 0) {
                printk(", render absent, total absent (the end seen late in all %llu)",
                       (unsigned long long)f);
            } else {
                printk(", render %llu, total %llu (end waited for in %llu of %llu)",
                       (unsigned long long)(c.render_us / c.exact_frames),
                       (unsigned long long)(c.job_total_us / c.exact_frames),
                       (unsigned long long)c.exact_frames, (unsigned long long)f);
            }
            printk(" (jobs %llu over %llu frames, frames with more than one %llu)\n",
                   (unsigned long long)c.jobs, (unsigned long long)f,
                   (unsigned long long)c.multi_job_frames);
        }
        print_jobs(c);
#elif defined(CONFIG_YAKOGL_DRV_GPU_PVR)
        print_kicks(c);
#else
        printk("; gpu job times: absent\n");
#endif
    }

#if defined(CONFIG_YAKOGL_GPU_V3D)
    /* One line per scene, after its glmark2-frame line, for the frame whose
     * swap is number JOBS_LINE_FRAME (osal_v3d_recent_jobs_us):
     *   glmark2-jobs: <scene>: frame N: J job(s): <job>; <job>; ...
     * <job> = "total T render R[~] idle I[ tfu K/Tus]" in us: the job's
     * submit-to-completion time, its render, and the GPU idle gap before its
     * bin (the end of the previous job's GPU work, TFU jobs included, to this
     * job's bin submit); "~" marks a render whose end the OSAL saw late (the
     * job's FRDONE was already up), "tfu" the K TFU jobs (glGenerateMipmap)
     * run since the previous job completed, T us in all. Jobs listed are
     * those completed by that swap (the frame's last render finishes at the
     * next submit, so the list can carry the previous frame's last job). Not
     * parsed by glmark2-ms.py. */
    void print_jobs(const FrameCost &c)
    {
        typedef unsigned long long ull;
        if (c.jobs_at == 0) {
            printk("glmark2-jobs: %s: frame %u: absent (the scene ran fewer frames)\n",
                   scene_->info_string().c_str(), (unsigned)FrameCost::JOBS_LINE_FRAME);
            return;
        }
        printk("glmark2-jobs: %s: frame %llu: %u job(s):", scene_->info_string().c_str(), (ull)c.jobs_at,
               c.jobs_n);
        for (unsigned i = 0; i < c.jobs_n; i++) {
            const osal_v3d_recent_job_us &j = c.jobs_ring[i];
            printk("%s total %u render %u%s idle %u", i == 0 ? "" : ";", j.total, j.render,
                   j.render_late ? "~" : "", j.idle);
            if (j.tfu_jobs != 0) {
                printk(" tfu %u/%uus", j.tfu_jobs, j.tfu);
            }
        }
        printk("\n");
    }
#endif

#if defined(CONFIG_YAKOGL_DRV_GPU_PVR)
    /* The per-kick line: means over the frames that completed a kick; each
     * fence time with its mean error bar and how many were bounded. */
    static void print_kicks(const FrameCost &c)
    {
        typedef unsigned long long ull;
        if (c.kick_rc == -ENOTSUP) {
            printk("; pvr kick times: absent (built without CONFIG_YAKOGL_DRV_GPU_PVR_KICK_TIMES)\n");
            return;
        }
        if (c.kick_frames == 0) {
            printk("; pvr kick times: absent (no kick completed in the scene, rc %d)\n", c.kick_rc);
            return;
        }
        const uint64_t f = c.kick_frames;
        printk("; pvr last completed kick per frame: submit %llu (mmu %llu, syncs %llu)",
               (ull)(c.submit_us / f), (ull)(c.mmu_us / f), (ull)c.mmu_syncs);
        if (c.geom_frames == 0) {
            printk(", kick->geom absent (never seen)");
        } else {
            printk(", kick->geom %llu err %llu (bounded %llu of %llu)",
                   (ull)(c.kick_geom_us / c.geom_frames), (ull)(c.geom_err_us / c.geom_frames),
                   (ull)c.geom_bounded, (ull)c.geom_frames);
        }
        printk(", kick->frag %llu err %llu (bounded %llu of %llu)", (ull)(c.kick_frag_us / f),
               (ull)(c.frag_err_us / f), (ull)c.frag_bounded, (ull)f);
        if (c.prev_frames == 0) {
            printk(", frag->frag absent");
        } else {
            printk(", frag->frag %llu (%llu, the scene's first left out)",
                   (ull)(c.frag_frag_us / c.prev_frames), (ull)c.prev_frames);
        }
        if (c.wait_frames == 0) {
            printk(", wait absent (no wait named the kick)");
        } else {
            printk(", wait %llu (%llu)", (ull)(c.wait_us / c.wait_frames), (ull)c.wait_frames);
        }
        printk(" (kicks %llu over %llu frames, frames with more than one %llu, lost %u; bound %u us)\n",
               (ull)c.kicks, (ull)f, (ull)c.multi_kick_frames, c.lost, KICK_BOUND_US);
    }
#endif

    Outcomes &out_;
    size_t entry_base_;
    Picture picture_; /* the running scene's last frame */
    uint32_t setup_why_ = YK_RECORD_GLMARK2_WHY_NO_ERROR;
};

/* ---- the arguments ------------------------------------------------------- */

/* argv for Options::parse_args: the program name, --size, then
 * CONFIG_GLMARK2_ARGS split at spaces (no quoting: no glmark2 option value
 * needs a space). The strings live as long as the run. */
std::vector<std::string> s_args;
std::vector<char *> s_argv;

void build_args()
{
    s_args.push_back("glmark2-es2");
    s_args.push_back("--size");
    s_args.push_back(CONFIG_GLMARK2_SIZE);
    const std::string extra(CONFIG_GLMARK2_ARGS);
    size_t i = 0;
    while (i < extra.size()) {
        size_t j = extra.find(' ', i);
        if (j == std::string::npos) {
            j = extra.size();
        }
        if (j > i) {
            s_args.push_back(extra.substr(i, j - i));
        }
        i = j + 1;
    }
    for (auto &a : s_args) {
        s_argv.push_back(a.data());
    }
    s_argv.push_back(nullptr);
}

int parse_size(const char *s, uint32_t *w, uint32_t *h)
{
    char *end;
    unsigned long a = strtoul(s, &end, 10);

    if (*end != 'x') {
        return -EINVAL;
    }
    unsigned long b = strtoul(end + 1, &end, 10);
    if (*end != '\0' || a == 0 || b == 0 || a > 8192 || b > 8192) {
        return -EINVAL;
    }
    *w = a;
    *h = b;
    return 0;
}

/* ---- the run ------------------------------------------------------------- */

int run()
{
    uint32_t w, h;
    struct board_window win;

    printk("glmark2: upstream %s (%s), size %s, args \"%s\"\n", GLMARK2_COMMIT,
           GLMARK_VERSION, CONFIG_GLMARK2_SIZE, CONFIG_GLMARK2_ARGS);
    /* a GL error before the first entry's setup (EGL, the canvas) is no entry's */
    yakogl_gl_error_tag(YK_RECORD_GL_ERROR_TAG_NONE);
    if (parse_size(CONFIG_GLMARK2_SIZE, &w, &h) != 0) {
        printk("glmark2: CONFIG_GLMARK2_SIZE \"%s\" is not WxH\n", CONFIG_GLMARK2_SIZE);
        return -EINVAL;
    }
    int rc = board_window(w, h, &win);
    if (rc != 0) {
        printk("glmark2: board_window(%u, %u) = %d\n", w, h, rc);
        return rc;
    }
    /* packed, or a view into the screen (CONFIG_YAKOGL_BOARD_SCANOUT_VIEW):
     * whole pixels, at least w; EGL checks the rest of the pitch rule. A
     * 565 window (CONFIG_YAKOGL_BOARD_WINDOW_RGB565) has 2 bytes a pixel. */
    const uint32_t bpp =
        (win.format & ~(GLES_NATIVE_WINDOW_BOTTOM_UP | GLES_NATIVE_WINDOW_SCANOUT)) ==
                GLES_NATIVE_FORMAT_RGB565
            ? 2u
            : 4u;
    if (win.stride < w * bpp || win.stride % bpp != 0u) {
        printk("glmark2: board_window gave stride %u, not whole pixels of at least %u\n",
               win.stride, w * bpp);
        return -EINVAL;
    }
    printk("glmark2: window %ux%u, stride %u, %u bytes a pixel%s\n", w, h, win.stride, bpp,
           win.stride == w * bpp ? "" : " (a view into the screen)");

    build_args();
    if (!Options::parse_args(static_cast<int>(s_argv.size()) - 1, s_argv.data())) {
        printk("glmark2: the arguments were refused\n");
        return -EINVAL;
    }
    if (Options::show_help || Options::show_version || Options::list_scenes ||
        Options::validate) {
        printk("glmark2: --help/--version/--list-scenes/--validate is not a benchmark run\n");
        return -EINVAL;
    }
    Log::init("glmark2-es2", Options::show_debug, &s_setup_log);
    if (!ResultsFile::init(Options::results_file)) {
        printk("glmark2: --results-file needs a file system\n");
        return -EINVAL;
    }

    NativeStateBoard native(win);
    GLStateTimed gl;
    CanvasGeneric canvas(native, gl, Options::size.first, Options::size.second);
    canvas.offscreen(Options::offscreen);
    canvas.visual_config(Options::visual_config);

    SceneCollection scenes(canvas);
    scenes.register_scenes();

    if (!canvas.init()) {
        printk("glmark2: the canvas did not come up (EGL/GL)\n");
        return -EIO;
    }
#if defined(CONFIG_GLMARK2_PVR_ONE_KICK_IN_FLIGHT)
    /* A measurement arm: every swap waits for its own kick. */
    pvr_backend_set_option(PVR_BACKEND_OPT_ONE_IN_FLIGHT, 1);
    printk("glmark2: PVR_BACKEND_OPT_ONE_IN_FLIGHT=1 (one kick in flight; a measurement, not a score)\n");
#endif

    ResultsFile &results = ResultsFile::get();
    results.begin();
    results.begin_info();
    Log::info("=======================================================\n");
    Log::info("    glmark2 %s\n", GLMARK_VERSION);
    Log::info("=======================================================\n");
    canvas.print_info();
    Log::info("=======================================================\n");
    results.end_info();
    canvas.visible(true);

    BenchmarkCollection collection;
    collection.populate_from_options();
    if (collection.needs_decoration()) {
        /* show-fps / title draw text over every frame (MainLoopDecoration):
         * extra GPU work the published runs do not have */
        printk("glmark2: the show-fps/title options are not supported in this image\n");
        return -ENOTSUP;
    }
    const size_t listed = collection.benchmarks().size();
    /* a GM page runs only its entries (docs/notes/imx8mp-gm-record.md) */
    const GmPageAsk page = gm_page_asked(listed);
    std::vector<Benchmark *> list = collection.benchmarks();
    if (page.asked) {
        if (page.first >= page.end) {
            printk("glmark2: GM page 0x%02x names no entry of the %u-entry list\n", page.tag,
                   static_cast<unsigned int>(listed));
            return -EINVAL;
        }
        list.assign(collection.benchmarks().begin() + static_cast<std::ptrdiff_t>(page.first),
                    collection.benchmarks().begin() + static_cast<std::ptrdiff_t>(page.end));
        printk("glmark2: GM page 0x%02x: entries %u..%u of %u only\n", page.tag,
               static_cast<unsigned int>(page.first), static_cast<unsigned int>(page.end - 1u),
               static_cast<unsigned int>(listed));
    }
#if defined(GM_PERF)
    /* the frame-breakdown record: counted from the first scene on */
    if (!perf_read_selector()) {
        return -EINVAL;
    }
#endif
    Outcomes out;
    unsigned int score;
    {
        RecordingLoop loop(canvas, list, page.asked ? page.first : 0u, out);
        while (loop.step()) {
        }
        score = loop.score();
    }
    Log::info("=======================================================\n");
    Log::info("                                  glmark2 Score: %u \n", score);
    Log::info("=======================================================\n");
    results.end();

    /* An entry with an empty scene name only sets options and never reaches
     * log_scene_result, so the outcomes are what is compared. */
    const unsigned int seen = out.ran + out.failed + out.unsupported;
    printk("glmark2: score %u: %u scenes ran, %u failed, %u unsupported (%u list "
           "entries); window %ux%u%s, shown %s, GPU resets %u\n",
           score, out.ran, out.failed, out.unsupported, static_cast<unsigned int>(listed),
           w, h, Options::offscreen ? " off-screen" : "", native.shown() ? "yes" : "no",
           board_gpu_resets());
    const bool pass = seen != 0 && out.ran != 0 && out.failed == 0 && score != 0;
    /* What the GL core counted over the whole run: glmark2 never calls
     * glGetError, so a GL error it hits is otherwise only a console line,
     * and the i.MX8MP has no console (issue #13). Recorded is the part
     * glGetError could have returned (the first error masks the rest). */
    const uint32_t gl_errors = yakogl_gl_errors_raised();
    const uint32_t incomplete_draws = yakogl_incomplete_texture_draws();
    printk("glmark2: GL errors raised %u, draws that sampled an incomplete texture %u (GL "
           "errors recorded %u)\n",
           gl_errors, incomplete_draws, yakogl_gl_errors_recorded());
    print_first_gl_errors(gl_errors);
    if (pass && out.unsupported != 0) {
        printk("glmark2: no scene failed; %u unsupported, left out of the score (not a pass or "
               "a fail)\n",
               out.unsupported);
    }
    struct yd_glmark2_outcome o {};
    o.score = score;
    o.flags = (Options::offscreen ? YK_RECORD_GLMARK2_OFFSCREEN : 0u) | YK_RECORD_GLMARK2_GL;
    o.status = pass ? YK_RECORD_STATUS_PASS : YK_RECORD_STATUS_FAIL;
    o.ran = out.ran;
    o.failed = out.failed;
    o.unsupported = out.unsupported;
    o.failed_entry = out.failed_entry;
    o.why = out.why;
    o.unsupported_entries[0] = out.unsupported_entries[0];
    o.unsupported_entries[1] = out.unsupported_entries[1];
    o.w = w;
    o.h = h;
    o.gl_errors = gl_errors;
    o.incomplete_draws = incomplete_draws;
    uint32_t rec[3];
    yd_record_glmark2_words(&o, rec);
    board_result(YK_RECORD_KIND_GLMARK2, rec[0], rec[1], rec[2]);
    store_gl_error_record(gl_errors);
    if (page.asked) {
        store_gm_page(page, out.scenes);
    }
#if defined(GM_PERF)
    store_perf_page();
#endif
    return pass ? 0 : -EIO;
}

} // namespace

int main(void)
{
    int rc;

    try {
        rc = run();
    } catch (const std::exception &e) {
        /* Options::parse_args and the scenes throw on bad option values */
        printk("glmark2: exception: %s\n", e.what());
        rc = -EIO;
    }
    board_finish("glmark2", rc);
}
