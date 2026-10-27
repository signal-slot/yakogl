/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * femtovg's 30 programs, built through GL before femtovg builds them itself.
 *
 * femtovg compiles all its programs when a canvas is created
 * (renderer/opengl.rs, new_from_context), whether a frame uses them or not,
 * and a caller then learns only that the renderer did not start
 * (femtovg_draw returns 1; Slint's FemtoVGOpenGLRenderer::new returns an
 * error). This builds the same programs from femtovg's own shader text
 * (main-vs.glsl and main-fs.glsl of the tree the staticlib builds, embedded
 * by ../CMakeLists.txt as fvg_glsl.h), in femtovg's order and with its
 * source layout (program.rs, MainProgram::new), and keeps each link
 * failure's log, a histogram keyed on the whole log, and the program
 * femtovg stops at: the first in its order that fails. Like femtovg, it keeps every program alive while it builds the
 * next (the shaders are deleted after each link, the programs are not), so a
 * limit on what the compiler holds for live programs shows here, by name,
 * as it would in femtovg's canvas creation. Every program is deleted again
 * afterwards.
 */
#ifndef FEMTOVG_PROGRAMS_H
#define FEMTOVG_PROGRAMS_H

#include <stdint.h>

#define PROGRAMS_LOG_KEEP 160
#define PROGRAMS_N 30

struct programs_fail_row {
	char log[PROGRAMS_LOG_KEEP];
	unsigned n;
};

struct programs_survey {
	unsigned built, failed;
	struct programs_fail_row rows[PROGRAMS_N];
	unsigned nrows;
	char first[PROGRAMS_LOG_KEEP + 64]; /* the program femtovg stops at, and why */
	int64_t ms;
};

/* Build the 30 programs on the current context; print each failure. */
void programs_run(struct programs_survey *sv);

/* "femtovg programs: N of 30 link (T ms)", the histogram, the first stop. */
void programs_print(const struct programs_survey *sv);

/* "keep used N (peak P) of M bytes <when>": what the in-process compiler
 * holds for the shaders and programs alive now (shc_online_keep_stats), and
 * the region's size (CONFIG_YAKOGL_SHADERC_KEEP_BYTES when the image lends
 * it). */
void programs_keep_print(const char *when);

#endif
