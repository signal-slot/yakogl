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
 *                      CONFIG_SAFEUI_GPU_PVR_KICK_TIMES; "absent" on the
 *                      GC7000. Measured with the cycle counter (k_cycle_get_64).
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
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>

#include <cerrno>
#include <cstdint>
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
#include <yakogl_drivers.h> /* yd_record_glmark2_words, yd_record_glmark2_why */
}
#include "osal_internal.h" /* YakoGL src/osal: osal_mem_line (its own extern "C") */
extern "C" {
/* the GL core's counters for the record (issue #13), totals since start */
uint32_t yakogl_gl_errors_raised(void);         /* src/gl/context.rs */
uint32_t yakogl_gl_errors_recorded(void);       /* src/gl/context.rs */
uint32_t yakogl_incomplete_texture_draws(void); /* src/gl/draw.rs */
}
#if defined(CONFIG_YAKOGL_GPU_V3D)
extern "C" {
#include "zephyr_v3d.h" /* YakoGL src/osal: osal_v3d_last_completed_job_us */
}
#elif defined(CONFIG_SAFEUI_GPU_PVR)
extern "C" {
#include <pvr_render.h>     /* pvr_render_last_kick_us, pvr_render_scene_begin */
#include <yakogl_drivers.h> /* struct yd_pvr_kick_us */
#include "pvr_backend.h"    /* pvr_backend_set_option, PVR_BACKEND_OPT_ONE_IN_FLIGHT */
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
#elif defined(CONFIG_SAFEUI_GPU_PVR)
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
#if defined(CONFIG_SAFEUI_GPU_PVR)
constexpr uint32_t KICK_BOUND_US = 100;
#endif
FrameCost s_cost;

uint64_t cyc_to_us(uint64_t cyc)
{
    return cyc * 1000000u / sys_clock_hw_cycles_per_sec();
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

    /* After eglSwapBuffers (CanvasGeneric::update, FrameEndSwap). */
    void flip() override
    {
        const uint64_t t0 = k_cycle_get_64();
        int rc = board_show(&win_);

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
#elif defined(CONFIG_SAFEUI_GPU_PVR)
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

/* ---- the main loop, counting outcomes ----------------------------------- */

/* The counts and, for the record, which list entries did not run: the first
 * failed one with why, the first two unsupported ones. */
struct Outcomes {
    unsigned int ran = 0, failed = 0, unsupported = 0;
    uint32_t failed_entry = YK_RECORD_GLMARK2_ENTRY_NONE;
    uint32_t why = YK_RECORD_GLMARK2_WHY_NO_ERROR;
    uint32_t unsupported_entries[2] = {YK_RECORD_GLMARK2_ENTRY_NONE, YK_RECORD_GLMARK2_ENTRY_NONE};
};

class RecordingLoop : public MainLoop
{
public:
    RecordingLoop(Canvas &canvas, const std::vector<Benchmark *> &benchmarks, Outcomes &out)
        : MainLoop(canvas, benchmarks), out_(out)
    {
    }

    /* Upstream's MainLoop::draw (main-loop.cpp:129-138), timed: the GL calls
     * of the scene, then the canvas update (swap and flip, timed inside). */
    void draw() override
    {
        const uint64_t t0 = k_cycle_get_64();
        canvas_.clear();
        scene_->draw();
        scene_->update();
        const uint64_t t1 = k_cycle_get_64();
        canvas_.update();
        const uint64_t t2 = k_cycle_get_64();
        s_cost.gl += t1 - t0;
        s_cost.frame += t2 - t0;
        s_cost.frames++;
    }

    /* The scene's setup is the window in which an error line says why. */
    void before_scene_setup() override
    {
        print_mem("before setup");
        s_setup_errors.arm();
    }
    void after_scene_setup() override { setup_why_ = s_setup_errors.disarm(); }

    /* After the scene set up: its frames start here. */
    void log_scene_info() override
    {
        MainLoop::log_scene_info();
#if defined(CONFIG_SAFEUI_GPU_PVR)
        pvr_render_scene_begin(); /* the grow line's scene (GROW_SCENE) */
#endif
        const FrameCost fresh;
#if defined(CONFIG_YAKOGL_GPU_V3D) || defined(CONFIG_SAFEUI_GPU_PVR)
        const uint32_t done_before = s_cost.done_before;
        s_cost = fresh;
        s_cost.done_before = done_before;
#else
        s_cost = fresh;
#endif
    }

    void log_scene_result() override
    {
        MainLoop::log_scene_result(); /* upstream's own line, unchanged */
        print_cost();
        print_mem("end");
        /* the entry's index in the benchmark list, option-only entries counted */
        const uint32_t entry = static_cast<uint32_t>(bench_iter_ - benchmarks_.begin());
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
        printk("glmark2-mem: entry %u %s: %s\n", static_cast<unsigned int>(bench_iter_ - benchmarks_.begin()),
               when, line);
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
#elif defined(CONFIG_SAFEUI_GPU_PVR)
        print_kicks(c);
#else
        printk("; gpu job times: absent\n");
#endif
    }

#if defined(CONFIG_SAFEUI_GPU_PVR)
    /* The per-kick line: means over the frames that completed a kick; each
     * fence time with its mean error bar and how many were bounded. */
    static void print_kicks(const FrameCost &c)
    {
        typedef unsigned long long ull;
        if (c.kick_rc == -ENOTSUP) {
            printk("; pvr kick times: absent (built without CONFIG_SAFEUI_GPU_PVR_KICK_TIMES)\n");
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
        (win.format & ~GLES_NATIVE_WINDOW_BOTTOM_UP) == GLES_NATIVE_FORMAT_RGB565 ? 2u : 4u;
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
#if defined(CONFIG_SAFEUI_GPU_PVR)
    /* The swap drops the window's depth/stencil by default (EGL 1.5
     * 3.10.1); CONFIG_GLMARK2_PVR_KEEP_ZS is the opt-out. Printed either
     * way, so a run says which arm it was. */
    pvr_backend_set_option(PVR_BACKEND_OPT_SWAP_DROPS_ZS, IS_ENABLED(CONFIG_GLMARK2_PVR_KEEP_ZS) ? 0 : 1);
    printk("glmark2: PVR_BACKEND_OPT_SWAP_DROPS_ZS=%d (%s)\n", IS_ENABLED(CONFIG_GLMARK2_PVR_KEEP_ZS) ? 0 : 1,
           IS_ENABLED(CONFIG_GLMARK2_PVR_KEEP_ZS) ? "kept: CONFIG_GLMARK2_PVR_KEEP_ZS" : "the default");
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
    Outcomes out;
    unsigned int score;
    {
        RecordingLoop loop(canvas, collection.benchmarks(), out);
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
