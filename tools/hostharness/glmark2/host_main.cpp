/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * host_main.cpp - glmark2-es2 on the host against YakoGL, the Linux OSAL
 * (src/osal/linux.c: GPU memory is malloc'd, every submit completes at once,
 * nothing is rendered) and shim.c (README.md).
 *
 * Upstream's main.cpp with a window system of our own: the native window is
 * a struct gles_native_window over a malloc'd XRGB8888 buffer, which linux.c
 * maps at its host address. Upstream's own main cannot be used: its window
 * systems are X11, Wayland, DRM, ... and the null one has no window.
 *
 *   glmark2-host --gm2-list          the default benchmark list, one per line
 *   glmark2-host <glmark2 options>   upstream's options (run.sh passes
 *                                    "-b :nframes=N -b <entry>")
 *
 * Around every scene, on stderr with the library's lines:
 *   gm2: scene-begin <entry index>
 *   gm2: scene-end <info string>: <ok|failed|unsupported> frames=F draws=D refused=R
 * draws counts glDrawArrays/glDrawElements calls, refused the draws the
 * library refused (yakogl_draws_refused, src/backend.rs) during the scene.
 */
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <iostream>

/* upstream glmark2 (GLMARK2_SRC/src) */
#include "benchmark-collection.h"
#include "canvas-generic.h"
#include "default-benchmarks.h"
#include "gl-state-egl.h"
#include "log.h"
#include "main-loop.h"
#include "native-state.h"
#include "options.h"
#include "results-file.h"
#include "scene-collection.h"

#include <EGL/eglplatform.h> /* YakoGL's: struct gles_native_window */

#include "shim.h"

extern "C" unsigned int yakogl_draws_refused(void);

namespace {

class NativeStateHost : public NativeState
{
public:
    ~NativeStateHost() override { std::free(buf_); }

    bool init_display() override { return true; }

    /* EGL_DEFAULT_DISPLAY */
    void *display() override { return nullptr; }

    bool create_window(WindowProperties const &p) override
    {
        if (p.width <= 0 || p.height <= 0 || p.width > 8192 || p.height > 8192) {
            return false;
        }
        std::free(buf_);
        const size_t stride = static_cast<size_t>(p.width) * 4u;
        buf_ = std::aligned_alloc(4096, (stride * p.height + 4095) & ~size_t(4095));
        if (!buf_) {
            return false;
        }
        std::memset(buf_, 0, stride * p.height);
        nw_.phys = reinterpret_cast<uintptr_t>(buf_);
        nw_.width = p.width;
        nw_.height = p.height;
        nw_.stride_bytes = stride;
        nw_.format = GLES_NATIVE_FORMAT_XRGB8888;
        props_ = WindowProperties(p.width, p.height, false, 0, {});
        return true;
    }

    void *window(WindowProperties &p) override
    {
        p = buf_ ? props_ : WindowProperties();
        return buf_ ? &nw_ : nullptr;
    }

    void visible(bool) override {}
    bool should_quit() override { return false; }
    void flip() override {}

private:
    void *buf_ = nullptr;
    struct gles_native_window nw_ {};
    WindowProperties props_;
};

class RecordingLoop : public MainLoop
{
public:
    RecordingLoop(Canvas &canvas, const std::vector<Benchmark *> &benchmarks)
        : MainLoop(canvas, benchmarks)
    {
    }

    unsigned failed = 0;

    void draw() override
    {
        MainLoop::draw();
        frames_++;
    }

    void before_scene_setup() override
    {
        std::fprintf(stderr, "gm2: scene-begin %u\n",
                     static_cast<unsigned>(bench_iter_ - benchmarks_.begin()));
        frames_ = 0;
        draws_ = gm2_draws();
        refused_ = yakogl_draws_refused();
    }

    void log_scene_result() override
    {
        MainLoop::log_scene_result();
        const char *st = scene_setup_status_ == SceneSetupStatusSuccess       ? "ok"
                         : scene_setup_status_ == SceneSetupStatusUnsupported ? "unsupported"
                                                                              : "failed";
        failed += scene_setup_status_ == SceneSetupStatusFailure;
        std::fprintf(stderr, "gm2: scene-end %s: %s frames=%u draws=%lu refused=%u\n",
                     scene_->info_string().c_str(), st, frames_, gm2_draws() - draws_,
                     yakogl_draws_refused() - refused_);
    }

private:
    unsigned frames_ = 0;
    unsigned long draws_ = 0;
    unsigned refused_ = 0;
};

} // namespace

int main(int argc, char *argv[])
{
    if (argc == 2 && !std::strcmp(argv[1], "--gm2-list")) {
        for (const auto &b : DefaultBenchmarks::get()) {
            std::printf("%s\n", b.c_str());
        }
        return 0;
    }
    /* one stream with the library's and shim.c's stderr lines, in order */
    std::setvbuf(stdout, nullptr, _IONBF, 0);
    std::cout << std::unitbuf;

    if (!Options::parse_args(argc, argv)) {
        return 1;
    }
    Log::init("glmark2-es2", Options::show_debug);
    if (!ResultsFile::init(Options::results_file)) {
        return 1;
    }
    if (Options::show_help || Options::show_version || Options::list_scenes || Options::validate) {
        std::fprintf(stderr, "glmark2-host: not a benchmark run\n");
        return 2;
    }

    NativeStateHost native;
    GLStateEGL gl;
    CanvasGeneric canvas(native, gl, Options::size.first, Options::size.second);
    canvas.offscreen(Options::offscreen);
    canvas.visual_config(Options::visual_config);

    SceneCollection scenes(canvas);
    scenes.register_scenes();

    if (!canvas.init()) {
        std::fprintf(stderr, "gm2: canvas-init-failed\n");
        return 1;
    }
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
    unsigned failed;
    {
        RecordingLoop loop(canvas, collection.benchmarks());
        while (loop.step()) {
        }
        Log::info("                                  glmark2 Score: %u \n", loop.score());
        failed = loop.failed;
    }
    results.end();
    return failed ? 3 : 0;
}
