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
 *        gl_errors=E incomplete_draws=I
 * (one line) draws counts glDrawArrays/glDrawElements calls, refused the draws the
 * library refused (yakogl_draws_refused, src/backend.rs) during the scene,
 * gl_errors the GL errors its entry points raised (yakogl_gl_errors_raised,
 * src/gl/context.rs) and incomplete_draws the draws that sampled an
 * incomplete texture (yakogl_incomplete_texture_draws, src/gl/draw.rs):
 * the counters the board's GLMARK2 record carries.
 *
 * After it, for a scene that ran, the line the board sample prints too: the
 * last frame's picture value (driver/docs/glmark2.md, "The picture value").
 */
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <iostream>
#include <vector>

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
extern "C" unsigned int yakogl_gl_errors_raised(void);
extern "C" unsigned int yakogl_incomplete_texture_draws(void);

namespace {

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

    /* Row y (0 is the top) of the window as RGBA bytes: the XRGB8888 buffer
     * the frame is rendered into in place. glReadPixels is not used: on the
     * host's non-V3D back ends the window has no CPU view in the library
     * (no map_phys), so it raises GL_OUT_OF_MEMORY, which the gate reads as
     * a scene's GL error. The bytes are the same ones the board's glReadPixels
     * returns (alpha 0xff for an XRGB window). */
    bool window_row(int y, std::vector<uint8_t> &row) const
    {
        if (!buf_ || y < 0 || y >= nw_.height) {
            return false;
        }
        const uint8_t *src = static_cast<const uint8_t *>(buf_) + static_cast<size_t>(y) * nw_.stride_bytes;
        row.resize(static_cast<size_t>(nw_.width) * 4u);
        for (int x = 0; x < nw_.width; x++) {
            row[x * 4 + 0] = src[x * 4 + 2];
            row[x * 4 + 1] = src[x * 4 + 1];
            row[x * 4 + 2] = src[x * 4 + 0];
            row[x * 4 + 3] = 0xff;
        }
        return true;
    }

private:
    void *buf_ = nullptr;
    struct gles_native_window nw_ {};
    WindowProperties props_;
};

class RecordingLoop : public MainLoop
{
public:
    RecordingLoop(Canvas &canvas, const std::vector<Benchmark *> &benchmarks, const NativeStateHost &native)
        : MainLoop(canvas, benchmarks), native_(native)
    {
    }

    unsigned failed = 0;

    /* Upstream's MainLoop::draw (main-loop.cpp:129-138) with the scene's
     * last frame read back before its swap, as the board sample's draw does
     * (there with glReadPixels, and the readback is kept out of the frame
     * timing). The probes and the grid are the same code. */
    void draw() override
    {
        canvas_.clear();
        scene_->draw();
        scene_->update();
        if (!scene_->running() && scene_setup_status_ == SceneSetupStatusSuccess) {
            glFinish(); /* the frame is in the window before it is read */
            picture_ = capture_picture(canvas_.width(), canvas_.height(),
                                       [&](int y, std::vector<uint8_t> &row) { return native_.window_row(y, row); });
        }
        canvas_.update();
        frames_++;
    }

    void before_scene_setup() override
    {
        std::fprintf(stderr, "gm2: scene-begin %u\n",
                     static_cast<unsigned>(bench_iter_ - benchmarks_.begin()));
        frames_ = 0;
        picture_ = Picture();
        draws_ = gm2_draws();
        refused_ = yakogl_draws_refused();
        gl_errors_ = yakogl_gl_errors_raised();
        incomplete_ = yakogl_incomplete_texture_draws();
    }

    void log_scene_result() override
    {
        MainLoop::log_scene_result();
        const char *st = scene_setup_status_ == SceneSetupStatusSuccess       ? "ok"
                         : scene_setup_status_ == SceneSetupStatusUnsupported ? "unsupported"
                                                                              : "failed";
        failed += scene_setup_status_ == SceneSetupStatusFailure;
        if (scene_setup_status_ == SceneSetupStatusSuccess) {
            char text[256];
            picture_text(picture_, text, sizeof(text));
            std::fprintf(stderr, "glmark2-picture: %s: %s\n", scene_->info_string().c_str(), text);
        }
        std::fprintf(stderr,
                     "gm2: scene-end %s: %s frames=%u draws=%lu refused=%u gl_errors=%u "
                     "incomplete_draws=%u\n",
                     scene_->info_string().c_str(), st, frames_, gm2_draws() - draws_,
                     yakogl_draws_refused() - refused_, yakogl_gl_errors_raised() - gl_errors_,
                     yakogl_incomplete_texture_draws() - incomplete_);
    }

private:
    const NativeStateHost &native_;
    Picture picture_;
    unsigned frames_ = 0;
    unsigned long draws_ = 0;
    unsigned refused_ = 0;
    unsigned gl_errors_ = 0;
    unsigned incomplete_ = 0;
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
        RecordingLoop loop(canvas, collection.benchmarks(), native);
        while (loop.step()) {
        }
        Log::info("                                  glmark2 Score: %u \n", loop.score());
        failed = loop.failed;
    }
    results.end();
    return failed ? 3 : 0;
}
