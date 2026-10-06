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
#ifdef CONFIG_YAKOGL_GPU_POWERVR
#include "pvr/pvr_backend.h"   /* pvr_backend_set_option, PVR_BACKEND_OPT_* */
void pvr_debug_set_masked_clear(int on);
void pvr_set_native_cube(int on);
extern int pvr_native_cube;   /* src/shaderc/mod.rs: the switch itself */
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
#endif

extern "C" int main(void)
{
#ifdef CONFIG_GC_COMMON_RESULT_SNVS
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
#ifdef CONFIG_GC_COMMON_RESULT_SNVS
    printk("deqp: case filter '%s'\n", argv[1] + sizeof("--deqp-case=") - 1);
#else
    printk("deqp: case filter '%s'\n", CONFIG_DEQP_CASE);
#endif
#ifdef CONFIG_YAKOGL_GPU_POWERVR
    /* on by default (board-verified); n is the off arm of an A/B */
    pvr_debug_set_masked_clear(IS_ENABLED(CONFIG_DEQP_PVR_MASKED_CLEAR));
    printk("deqp: pvr_debug_masked_clear=%d (clears honour glColorMask)\n",
           (int)IS_ENABLED(CONFIG_DEQP_PVR_MASKED_CLEAR));
    /* the A/B switches: -1 leaves the library's default, and says so */
    if (CONFIG_DEQP_PVR_NATIVE_CUBE >= 0)
        pvr_set_native_cube(CONFIG_DEQP_PVR_NATIVE_CUBE);
    printk("deqp: pvr_native_cube=%d (CONFIG_DEQP_PVR_NATIVE_CUBE=%d%s)\n", pvr_native_cube,
           CONFIG_DEQP_PVR_NATIVE_CUBE, CONFIG_DEQP_PVR_NATIVE_CUBE >= 0 ? "" : ": the library's default");
    if (CONFIG_DEQP_PVR_DBIAS_INT >= 0)
        pvr_backend_set_option(PVR_BACKEND_OPT_DBIAS_INT, (uint32_t)CONFIG_DEQP_PVR_DBIAS_INT);
    printk("deqp: PVR_BACKEND_OPT_DBIAS_INT=%d (%s)\n", CONFIG_DEQP_PVR_DBIAS_INT,
           CONFIG_DEQP_PVR_DBIAS_INT >= 0 ? "set" : "-1: untouched");
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
    return pvr_sample_finish(rc);
}
