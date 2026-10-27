/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * What glmark2 takes from an operating system and the image gives instead
 * (include/dlfcn.h, include/glmark2_platform.h):
 *
 *   dlopen/dlsym  YakoGL is linked in: the two libraries glmark2 opens
 *                 (gl-state-egl.cpp: libEGL, then libGLESv2) are the image
 *                 itself, and a symbol is what eglGetProcAddress answers.
 *                 Any other library does not exist, as dlopen says for a
 *                 missing file.
 *   data files    the table glmark2_assets.S holds (CMakeLists.txt).
 *   isatty        "not a terminal" for every descriptor (include/unistd.h).
 */
#include <zephyr/kernel.h>
#include <errno.h>
#include <string.h>

#include <EGL/egl.h>

#include "dlfcn.h"
#include "unistd.h" /* include/unistd.h: isatty */
#include "glmark2_platform.h"

/* ---- dlopen / dlsym ------------------------------------------------------ */

static int s_lib; /* the handle: one address for "the image" */
static char s_dlerror[96];
static bool s_dlerror_set;

static bool is_gl_library(const char *name)
{
	/* the names gl-state-egl.cpp tries: libEGL.so[.1], libGLESv2.so[.2] */
	return strncmp(name, "libEGL.so", 9) == 0 || strncmp(name, "libGLESv2.so", 12) == 0;
}

void *dlopen(const char *filename, int flags)
{
	ARG_UNUSED(flags);
	if (filename == NULL || !is_gl_library(filename)) {
		snprintk(s_dlerror, sizeof(s_dlerror), "%s: not in this image",
			 filename ? filename : "(main program)");
		s_dlerror_set = true;
		return NULL;
	}
	return &s_lib;
}

void *dlsym(void *handle, const char *symbol)
{
	if (handle != &s_lib) {
		snprintk(s_dlerror, sizeof(s_dlerror), "dlsym: not a handle of dlopen");
		s_dlerror_set = true;
		return NULL;
	}
	void *f = (void *)eglGetProcAddress(symbol);

	if (f == NULL) {
		/* glad asks for every entry point of every version and extension;
		 * one YakoGL lacks stays NULL, as with a real libGLESv2 */
		snprintk(s_dlerror, sizeof(s_dlerror), "%s: undefined symbol", symbol);
		s_dlerror_set = true;
	}
	return f;
}

int dlclose(void *handle)
{
	return handle == &s_lib ? 0 : -1;
}

char *dlerror(void)
{
	if (!s_dlerror_set) {
		return NULL;
	}
	s_dlerror_set = false;
	return s_dlerror;
}

/* ---- the data files ------------------------------------------------------ */

struct glmark2_asset {
	const char *name;
	const void *data;
	size_t size;
};
/* glmark2_assets.S */
extern const struct glmark2_asset glmark2_asset_table[];
extern const size_t glmark2_asset_table_count;

int glmark2_asset_find(const char *path, const void **data, size_t *size)
{
	for (size_t i = 0; i < glmark2_asset_table_count; i++) {
		if (strcmp(glmark2_asset_table[i].name, path) == 0) {
			*data = glmark2_asset_table[i].data;
			*size = glmark2_asset_table[i].size;
			return 0;
		}
	}
	return -1;
}

size_t glmark2_asset_count(void)
{
	return glmark2_asset_table_count;
}

const char *glmark2_asset_name(size_t i)
{
	return i < glmark2_asset_table_count ? glmark2_asset_table[i].name : "";
}

/* ---- isatty (include/unistd.h) ------------------------------------------- */

int isatty(int fd)
{
	ARG_UNUSED(fd);
	errno = ENOTTY;
	return 0;
}

unsigned int glmark2_num_cpus(void)
{
	return arch_num_cpus();
}
