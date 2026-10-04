/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * femtovg's 30 programs, built through GL before femtovg builds them itself.
 *
 * femtovg compiles all its programs when a canvas is created
 * (renderer/opengl.rs, new_from_context), whether a frame uses them or not,
 * and a caller then learns only that the renderer did not start
 * (fvg_draw_frame returns 1; Slint's FemtoVGOpenGLRenderer::new returns an
 * error). This builds the same programs from femtovg's own shader text
 * (main-vs.glsl and main-fs.glsl, embedded by fvg_programs.cmake as
 * fvg_glsl.h), in femtovg's order and with its source layout (program.rs,
 * MainProgram::new), and keeps each link failure's log, a histogram keyed on
 * the whole log, and the program femtovg stops at: the first in its order
 * that fails. Like femtovg, it keeps every program alive while it builds the
 * next (the shaders are deleted after each link, the programs are not), so a
 * limit on what the compiler holds for live programs shows here, by name,
 * as it would in femtovg's canvas creation. Every program is deleted again
 * afterwards.
 *
 * Used by v3d_gl_femtovg; v3d_gl_cluster used it too until Step 5 deleted
 * it (Slint's renderer draws through the same no_std femtovg).
 */
#ifndef FVG_PROGRAMS_H
#define FVG_PROGRAMS_H

#include <stdint.h>

#define FVG_LOG_KEEP 160
#define FVG_PROGRAMS 30

struct fvg_fail_row {
	char log[FVG_LOG_KEEP];
	unsigned n;
};

struct fvg_survey {
	unsigned built, failed;
	struct fvg_fail_row rows[FVG_PROGRAMS];
	unsigned nrows;
	char first[FVG_LOG_KEEP + 64]; /* the program femtovg stops at, and why */
	int64_t ms;
};

/* Build the 30 programs on the current context; print each failure. */
void fvg_survey_run(struct fvg_survey *sv);

/* "femtovg programs: N of 30 link (T ms)", the histogram, the first stop. */
void fvg_survey_print(const struct fvg_survey *sv);

/* "keep used N (peak P) of M bytes <when>": what the in-process compiler
 * holds for the shaders and programs alive now (shc_online_keep_stats), and
 * the region's size (CONFIG_YAKOGL_SHADERC_KEEP_BYTES when the image lends
 * it). */
void fvg_keep_print(const char *when);

#endif
