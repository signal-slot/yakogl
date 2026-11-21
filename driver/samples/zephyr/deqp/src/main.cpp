/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * dEQP on Zephyr: the tcu::App main loop of framework/platform/tcuMain.cpp,
 * with the arguments built in (there is no command line), the test data from
 * a memory archive and the log turned into one result line per case
 * (deqp_log.c). EGL/GLES are YakoGL, linked in directly.
 */
#include "tcuApp.hpp"
#include "tcuCommandLine.hpp"
#include "tcuPlatform.hpp"
#include "tcuTestLog.hpp"
#include "tcuTestPackage.hpp"
#include "deUniquePtr.hpp"
#include "deqp_archive.hpp"
#include "qpDebugOut.h"
#include <cstdarg>
#include <cstdio>
#include <cstring>
#include <exception>

extern "C" {
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/util.h>
#include "sample_glue.h"
#ifdef CONFIG_YAKOGL_BOARD_STM32MP2
#include "yakogl_board.h"
#endif
#ifdef CONFIG_YAKOGL_BOARD_VERDIN_IMX95
/* src/mali_board.c: the Verdin iMX95's end (the board layer's sample_glue.c
 * has pvr_sample_finish, which does not end through board_finish) */
FUNC_NORETURN void mali_deqp_finish(int rc);
#endif
#ifdef CONFIG_YAKOGL_GPU_POWERVR
void pvr_set_native_cube(int on);
extern int pvr_native_cube;   /* src/shaderc/mod.rs: the switch itself */
void pvr_set_native_array(int on);
extern int pvr_native_array;  /* src/shaderc/mod.rs: the 2D-array switch */
void pvr_set_native_3d(int on);
extern int pvr_native_3d;     /* src/shaderc/mod.rs: the 3D-texture switch */
#endif
void deqp_log_summary(void);
}

tcu::Platform *createPlatform(void);

/*
 * dEQP ends a run it cannot continue with tcu::die() -> qpDiev(): it prints
 * the message with type MESSAGETYPE_ERROR, then calls exitProcess(). Its
 * non-Windows arm is fflush(NULL) and raise(SIGKILL). picolibc's tinystdio
 * fflush() does not accept NULL: it reads stream->flush (offset 0x18). So on
 * Zephyr a die() was a data abort at FAR 0x18, with no summary line
 * (AM62P, dEQP-GLES3.functional.lifetime.*, 2026-09-26: the group's init
 * fails TCU_CHECK(isOk()) on a program that asks for transform feedback).
 * raise() would not have ended anything either.
 *
 * This hook sees the fatal message before exitProcess() runs and ends the
 * image itself. It prints dEQP's message, a "deqp: fatal:" line and the
 * usual summary, then takes Zephyr's fatal path (k_panic). That is where
 * dEQP's own assertion failures already go on Zephyr (deAssertFail ->
 * abort()), and sample_glue.c's handler writes the FAULT record there and
 * resets. exitProcess() is never reached.
 *
 * MESSAGETYPE_ERROR is 1 in qpDebugOut.c's private enum (INFO 0, ERROR 1,
 * NONFATAL_ERROR 2). Only qpDief() and qpDiev() use it, and both call
 * exitProcess() next. Every other type is printed by dEQP as before.
 */
static bool deqp_fatal_redirect(int type, const char *format, va_list args)
{
    static const int message_type_error = 1;

    if (type != message_type_error)
        return true; /* dEQP prints it itself */
    printk("FATAL ERROR: ");
    vprintk(format, args);
    printk("\ndeqp: fatal: dEQP ended the run (die()); later cases did not run\n");
    deqp_log_summary();
    k_panic();
    return false; /* not reached */
}

#ifdef CONFIG_GC_COMMON_RESULT_SNVS
extern "C" int gc_result_selector(void);

/* The --deqp-case argument: DEQP_CASE, or the table entry the reader's
 * selector names (Kconfig DEQP_CASE_TABLE). */
static const char *case_argument(void)
{
    static char arg[2048]; /* a batch: comma-separated patterns */
    const char *table = CONFIG_DEQP_CASE_TABLE;
    int want = gc_result_selector();

    if (want >= 0)
        want &= 0xff; /* the rest names a page of failures (gc_result.h) */

    if (want >= 0) {
        for (int i = 0; *table; i++) {
            const char *end = strchr(table, ';');
            size_t n = end ? (size_t)(end - table) : strlen(table);

            if (i == want && n > 0 && n < sizeof(arg) - sizeof("--deqp-case=")) {
                snprintf(arg, sizeof(arg), "--deqp-case=%.*s", (int)n, table);
                printk("deqp: selector %d\n", want);
                return arg;
            }
            if (!end)
                break;
            table = end + 1;
        }
    }
    return "--deqp-case=" CONFIG_DEQP_CASE;
}
#elif defined(CONFIG_DEQP_CASE_SLOT)
/*
 * The patch slot (Kconfig DEQP_CASE_SLOT; the STM32MP257F-DK, whose board
 * layer has no selector): a block in the image that a host tool writes
 * after the build, before the runner stages the image
 * (tools/cts/mp2-deqp-slot.py; the layout is pinned there and must stay the
 * same). Little-endian, in .rodata and so in zephyr.bin:
 *   magic[24]  "yakogl-deqp-case-slot/1", NUL-terminated (offset 0)
 *   size       uint32: DEQP_CASE_SLOT_SIZE, the filter's room (24)
 *   selector   int32: -1 none, else the index into DEQP_CASE_TABLE (28)
 *   length     uint32: 0 none, else the filter's length in bytes (32)
 *   filter     DEQP_CASE_SLOT_SIZE bytes (36): a --deqp-case value (patterns
 *              separated by ','), NUL-terminated
 * The filter wins over the selector, the selector over DEQP_CASE. A slot
 * that does not hold together (no NUL at filter[length], an index past the
 * table) is named on the console and runs DEQP_CASE.
 */
struct deqp_case_slot {
    char magic[24];
    uint32_t size;
    int32_t selector;
    uint32_t length;
    char filter[CONFIG_DEQP_CASE_SLOT_SIZE];
};
static_assert(offsetof(struct deqp_case_slot, filter) == 36,
              "the slot layout tools/cts/mp2-deqp-slot.py writes");

__attribute__((used)) static const struct deqp_case_slot deqp_slot = {
    "yakogl-deqp-case-slot/1", CONFIG_DEQP_CASE_SLOT_SIZE, -1, 0, {0}};

/* The --deqp-case argument: the slot's filter, the DEQP_CASE_TABLE entry its
 * selector names, or DEQP_CASE. */
static const char *case_argument(void)
{
    static char arg[sizeof("--deqp-case=") + MAX(2048, CONFIG_DEQP_CASE_SLOT_SIZE)];
    const struct deqp_case_slot *slot = &deqp_slot;
    const char *table = CONFIG_DEQP_CASE_TABLE;

    /* the compiler must read the bytes the host tool wrote, not fold the
     * initializer above */
    __asm__ volatile("" : "+r"(slot));
    int want = slot->selector;
    uint32_t len = slot->length;

    if (len > 0) {
        if (len < sizeof(slot->filter) && slot->filter[len] == '\0' &&
            strlen(slot->filter) == len) {
            snprintf(arg, sizeof(arg), "--deqp-case=%s", slot->filter);
            printk("deqp: slot filter, %u bytes\n", (unsigned)len);
            return arg;
        }
        printk("deqp: slot filter length %u does not hold together, running DEQP_CASE\n",
               (unsigned)len);
        return "--deqp-case=" CONFIG_DEQP_CASE;
    }
    if (want >= 0) {
        for (int i = 0; *table; i++) {
            const char *end = strchr(table, ';');
            size_t n = end ? (size_t)(end - table) : strlen(table);

            if (i == want && n > 0 && n < sizeof(arg) - sizeof("--deqp-case=")) {
                snprintf(arg, sizeof(arg), "--deqp-case=%.*s", (int)n, table);
                printk("deqp: selector %d\n", want);
                return arg;
            }
            if (!end)
                break;
            table = end + 1;
        }
        printk("deqp: slot selector %d names no DEQP_CASE_TABLE entry, running DEQP_CASE\n",
               want);
    }
    return "--deqp-case=" CONFIG_DEQP_CASE;
}
#endif

/*
 * The four-draw-target probe (docs/notes/mrt4-v3d-pvr.md section 3, cases:
 * docs/notes/mrt4-deqp-cases.md): the library's test-knob setters, defined
 * with CONFIG_YAKOGL_TEST_KNOBS (DEQP_TEST_KNOBS selects it). Both symbols
 * default n, so a default image compiles out the declaration and the call
 * and never references the setters.
 */
#if defined(CONFIG_V3D_DEQP_MRT4)
extern "C" int v3d_debug_set_draw_targets(int n);
#endif
#if defined(CONFIG_PVR_DEQP_MRT4)
extern "C" int pvr_debug_set_draw_targets(int n);
#endif

extern "C" int main(void)
{
#if defined(CONFIG_GC_COMMON_RESULT_SNVS) || defined(CONFIG_DEQP_CASE_SLOT)
    const char *argv[] = {
        "deqp",
        case_argument(),
#else
    static const char *argv[] = {
        "deqp",
        "--deqp-case=" CONFIG_DEQP_CASE,
#endif
        "--deqp-surface-type=pbuffer",
        "--deqp-surface-width=" STRINGIFY(CONFIG_DEQP_SURFACE_SIZE),
        "--deqp-surface-height=" STRINGIFY(CONFIG_DEQP_SURFACE_SIZE),
        "--deqp-log-filename=deqp.qpa",
#ifdef CONFIG_DEQP_ECHO_QPA
        "--deqp-log-images=enable",
        "--deqp-log-shader-sources=enable",
#else
        "--deqp-log-images=disable",
        "--deqp-log-shader-sources=disable",
#endif
        "--deqp-watchdog=disable",
#ifdef CONFIG_DEQP_GL_CONFIG_NAME
        /* last, so an empty name drops it from argc below (dEQP rejects a
         * repeated option, and a config called "" does not exist) */
        "--deqp-gl-config-name=" CONFIG_DEQP_GL_CONFIG_NAME,
#endif
    };
    int rc = 0;

    qpRedirectOut(nullptr, deqp_fatal_redirect);
#if defined(CONFIG_GC_COMMON_RESULT_SNVS) || defined(CONFIG_DEQP_CASE_SLOT)
    printk("deqp: case filter '%s'\n", argv[1] + sizeof("--deqp-case=") - 1);
#else
    printk("deqp: case filter '%s'\n", CONFIG_DEQP_CASE);
#endif
#ifdef CONFIG_YAKOGL_GPU_POWERVR
    /* the A/B switches: -1 leaves the library's default, and says so */
    if (CONFIG_DEQP_PVR_NATIVE_CUBE >= 0)
        pvr_set_native_cube(CONFIG_DEQP_PVR_NATIVE_CUBE);
    printk("deqp: pvr_native_cube=%d (CONFIG_DEQP_PVR_NATIVE_CUBE=%d%s)\n", pvr_native_cube,
           CONFIG_DEQP_PVR_NATIVE_CUBE, CONFIG_DEQP_PVR_NATIVE_CUBE >= 0 ? "" : ": the library's default");
    if (CONFIG_DEQP_PVR_NATIVE_ARRAY >= 0)
        pvr_set_native_array(CONFIG_DEQP_PVR_NATIVE_ARRAY);
    printk("deqp: pvr_native_array=%d (CONFIG_DEQP_PVR_NATIVE_ARRAY=%d%s)\n", pvr_native_array,
           CONFIG_DEQP_PVR_NATIVE_ARRAY, CONFIG_DEQP_PVR_NATIVE_ARRAY >= 0 ? "" : ": the library's default");
    if (CONFIG_DEQP_PVR_NATIVE_3D >= 0)
        pvr_set_native_3d(CONFIG_DEQP_PVR_NATIVE_3D);
    printk("deqp: pvr_native_3d=%d (CONFIG_DEQP_PVR_NATIVE_3D=%d%s)\n", pvr_native_3d,
           CONFIG_DEQP_PVR_NATIVE_3D, CONFIG_DEQP_PVR_NATIVE_3D >= 0 ? "" : ": the library's default");
#endif
#if defined(CONFIG_V3D_DEQP_MRT4)
    /* four draw targets before the first context, so GL_MAX_DRAW_BUFFERS and
     * GL_MAX_COLOR_ATTACHMENTS answer 4 (docs/notes/mrt4-deqp-cases.md); the
     * setter returns the previous count */
    printk("deqp: v3d_debug_set_draw_targets(4) -> %d (CONFIG_V3D_DEQP_MRT4=y)\n",
           v3d_debug_set_draw_targets(4));
#endif
#if defined(CONFIG_PVR_DEQP_MRT4)
    printk("deqp: pvr_debug_set_draw_targets(4) -> %d (CONFIG_PVR_DEQP_MRT4=y)\n",
           pvr_debug_set_draw_targets(4));
#endif
    try
    {
        int argc = (int)(sizeof(argv) / sizeof(argv[0]));
#ifdef CONFIG_DEQP_GL_CONFIG_NAME
        if (sizeof(CONFIG_DEQP_GL_CONFIG_NAME) == 1)
            argc--;
#endif
        tcu::CommandLine cmdLine(argc, (char **)argv);
        MemArchive archive;
        tcu::TestLog log(cmdLine.getLogFileName(), cmdLine.getLogFlags());
        de::UniquePtr<tcu::Platform> platform(createPlatform());
        de::UniquePtr<tcu::App> app(new tcu::App(*platform, archive, log, cmdLine));

        while (app->iterate())
        {
        }
        const tcu::TestRunStatus &r = app->getResult();
        printk("deqp: done: %d passed, %d failed, %d not supported, %d warnings, complete=%d\n",
               r.numPassed, r.numFailed, r.numNotSupported, r.numWarnings, (int)r.isComplete);
        rc = (!r.isComplete || r.numFailed) ? -1 : 0;
    }
    catch (const std::exception &e)
    {
        printk("deqp: exception: %s\n", e.what());
        rc = -2;
    }
    deqp_log_summary();
#ifdef CONFIG_YAKOGL_BOARD_STM32MP2
    /* the STM32MP257F-DK: "deqp: PASS"/"FAIL", the yk-record line and PSCI
     * SYSTEM_RESET back into ST's Linux (board_stm32mp2.c) */
    board_finish("deqp", rc);
#elif defined(CONFIG_YAKOGL_BOARD_VERDIN_IMX95)
    /* the Verdin iMX95: the OSAL's close line, "deqp: OK|FAILED (rc=N)",
     * then board_finish(): the yk-record line and PSCI SYSTEM_RESET */
    mali_deqp_finish(rc);
#else
    return pvr_sample_finish(rc);
#endif
}
