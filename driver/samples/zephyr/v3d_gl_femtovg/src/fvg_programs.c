/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* femtovg's 30 programs through GL: see fvg_programs.h. */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>
#include <stdbool.h>
#include <stdio.h>
#include <string.h>

#include <GLES2/gl2.h>

#include "fvg_programs.h"
#include "gles_shaderc.h" /* shc_online_keep_stats */
#include "fvg_glsl.h" /* fvg_main_vs, fvg_main_fs: femtovg's shader text */

/* femtovg's ShaderType numbers (renderer/params.rs) and names */
static const char *const fvg_type_name[10] = {
	"FillGradient", "FillImage", "Stencil", "FillImageGradient", "FilterImage",
	"FillColor", "TextureCopyUnclipped", NULL, "FillGradientConic", "FillImageGradientConic",
};

static char src_buf[16384];

/* The source femtovg's MainProgram::new passes to glShaderSource. */
static const char *fvg_source(bool vs, int type, bool glyph, bool no_discard)
{
	int n;
	if (vs) {
		n = snprintf(src_buf, sizeof(src_buf), "#version 100\n#define EDGE_AA 1\n%s", fvg_main_vs);
	} else {
		n = snprintf(src_buf, sizeof(src_buf),
			     "#version 100\n#define EDGE_AA 1\n#define SELECT_SHADER %d\n%s%s\n%s", type,
			     glyph ? "#define ENABLE_GLYPH_TEXTURE" : "",
			     no_discard ? "\n#define NO_DISCARD" : "", fvg_main_fs);
	}
	return n > 0 && (size_t)n < sizeof(src_buf) ? src_buf : NULL;
}

/* The info log with its line breaks as '|', so one row is one line. */
static void one_line(char *s)
{
	size_t n = strlen(s);
	while (n && (s[n - 1] == '\n' || s[n - 1] == ' '))
		s[--n] = 0;
	for (char *p = s; *p; p++)
		if (*p == '\n')
			*p = '|';
}

/* Build one program as femtovg's Program::new does: compile both stages,
 * attach, bind the attributes, link, then detach and delete the shaders.
 * The program stays alive (*out), as femtovg keeps all 30 at once: what the
 * compiler holds for 30 live programs is what femtovg's canvas needs.
 * 0 when it links, else the log (compile or link) and no program. */
static int fvg_program(int type, bool glyph, bool no_discard, char *log, size_t logsz, GLuint *out)
{
	GLuint sh[2] = { 0, 0 }, prog = 0;
	int rc = 0;
	log[0] = 0;
	for (int s = 0; s < 2 && rc == 0; s++) {
		const char *src = fvg_source(s == 0, type, glyph, no_discard);
		if (!src) {
			snprintf(log, logsz, "(the sample's source buffer is too small)");
			return -1;
		}
		GLint ok = 0;
		sh[s] = glCreateShader(s == 0 ? GL_VERTEX_SHADER : GL_FRAGMENT_SHADER);
		glShaderSource(sh[s], 1, &src, NULL);
		glCompileShader(sh[s]);
		glGetShaderiv(sh[s], GL_COMPILE_STATUS, &ok);
		if (!ok) {
			int k = snprintf(log, logsz, "%s compile: ", s == 0 ? "vs" : "fs");
			glGetShaderInfoLog(sh[s], (GLsizei)(logsz - (size_t)k), NULL, log + k);
			rc = -1;
		}
	}
	if (rc == 0) {
		GLint ok = 0;
		prog = glCreateProgram();
		glAttachShader(prog, sh[0]);
		glAttachShader(prog, sh[1]);
		/* program.rs Program::new binds these before the link */
		glBindAttribLocation(prog, 0, "vertex");
		glBindAttribLocation(prog, 1, "tcoord");
		glLinkProgram(prog);
		glGetProgramiv(prog, GL_LINK_STATUS, &ok);
		if (!ok) {
			int k = snprintf(log, logsz, "link: ");
			glGetProgramInfoLog(prog, (GLsizei)(logsz - (size_t)k), NULL, log + k);
			rc = -1;
		}
	}
	for (int s = 0; s < 2; s++) {
		if (sh[s]) {
			if (prog)
				glDetachShader(prog, sh[s]);
			glDeleteShader(sh[s]);
		}
	}
	if (rc == 0) {
		*out = prog;
	} else if (prog) {
		glDeleteProgram(prog);
	}
	one_line(log);
	return rc;
}

void fvg_survey_run(struct fvg_survey *sv)
{
	/* renderer/opengl.rs new_from_context: (glyph, no_discard) in this order,
	 * each a generate_shader_program_variants array indexed by ShaderType */
	static const bool sets[4][2] = { { true, false }, { false, false }, { true, true }, { false, true } };
	char log[FVG_LOG_KEEP];
	GLuint live[FVG_PROGRAMS];
	unsigned nlive = 0;
	int64_t t0 = k_uptime_get();

	memset(sv, 0, sizeof(*sv));
	for (int set = 0; set < 4; set++) {
		bool glyph = sets[set][0], nd = sets[set][1];
		for (int type = 0; type < 10; type++) {
			/* the None entries: 7 never; Stencil, FilterImage and
			 * TextureCopyUnclipped only without the glyph texture */
			if (type == 7 || (glyph && (type == 2 || type == 4 || type == 6)))
				continue;
			bool prog_nd = type == 6 ? false : nd; /* "has no stroke test to drop" */
			GLuint prog = 0;
			int64_t tp = k_uptime_get();
			printk("  program %2u %s%s%s: ", sv->built + sv->failed + 1, fvg_type_name[type],
			       glyph ? " glyph" : "", prog_nd ? " nodiscard" : "");
			if (fvg_program(type, glyph, prog_nd, log, sizeof(log), &prog) == 0) {
				printk("ok (%lld ms)\n", (long long)(k_uptime_get() - tp));
				sv->built++;
				if (nlive < FVG_PROGRAMS)
					live[nlive++] = prog;
				continue;
			}
			sv->failed++;
			printk("FAIL: %s\n", log);
			if (!sv->first[0])
				snprintf(sv->first, sizeof(sv->first), "%s%s%s: %s", fvg_type_name[type],
					 glyph ? " glyph" : "", prog_nd ? " nodiscard" : "", log);
			unsigned r = 0;
			while (r < sv->nrows && strcmp(sv->rows[r].log, log) != 0)
				r++;
			if (r == sv->nrows && sv->nrows < ARRAY_SIZE(sv->rows)) {
				strncpy(sv->rows[r].log, log, FVG_LOG_KEEP - 1);
				sv->nrows++;
			}
			if (r < sv->nrows)
				sv->rows[r].n++;
		}
	}
	fvg_keep_print("with the survey's programs alive");
	/* femtovg's own programs are built next: give back everything */
	for (unsigned i = 0; i < nlive; i++)
		glDeleteProgram(live[i]);
	sv->ms = k_uptime_get() - t0;
}

void fvg_survey_print(const struct fvg_survey *sv)
{
	printk("femtovg programs: %u of %u link (%lld ms)\n", sv->built, sv->built + sv->failed,
	       (long long)sv->ms);
	for (unsigned r = 0; r < sv->nrows; r++)
		printk("  %2u x %s\n", sv->rows[r].n, sv->rows[r].log);
	if (sv->first[0])
		printk("  femtovg stops at %s\n", sv->first);
}

void fvg_keep_print(const char *when)
{
	size_t in_use = 0, peak = 0;
	int rc = shc_online_keep_stats(&in_use, &peak);
	if (rc != 0) {
		printk("keep: shc_online_keep_stats %d %s\n", rc, when);
		return;
	}
#ifdef CONFIG_YAKOGL_SHADERC_KEEP_BYTES
	printk("keep used %zu (peak %zu) of %d bytes %s\n", in_use, peak, CONFIG_YAKOGL_SHADERC_KEEP_BYTES, when);
#else
	printk("keep used %zu (peak %zu) bytes (the crate's built-in region) %s\n", in_use, peak, when);
#endif
}
