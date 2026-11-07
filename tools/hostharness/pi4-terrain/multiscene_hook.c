/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * multiscene.sh's swap hook: after every eglSwapBuffers the V3D back end's
 * cumulative counters, one line on stderr,
 *
 *   gm2ms: swap <n> chains <c> queued <q> full <f> refused <14 values> copies <16 values>
 *
 * (chains: v3d_rt_images' TFU mip chains; refused, in the order of
 * src/v3d/backend.rs MIP_REFUSAL_NAMES; copies, in the order of
 * COPY_REASON_NAMES). multiscene.sh links it with -Wl,--wrap=eglGetProcAddress:
 * glmark2 takes eglSwapBuffers through eglGetProcAddress (shim.c's lookup),
 * so the pointer it gets here is this file's. Nothing else of the harness or
 * the library is changed.
 */
#include <EGL/egl.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#define MIP_REFUSALS 14 /* V3D_MIP_REFUSALS */
#define COPY_REASONS 16 /* V3D_COPY_REASONS */

extern void v3d_mip_counters(uint32_t *refused, uint32_t *queued, uint32_t *queue_full);
extern void v3d_copy_counters(uint32_t *copies);
extern void v3d_rt_images(uint32_t *live, uint64_t *bytes, uint32_t *direct, uint32_t *cpu_writes,
			  uint32_t *no_memory, uint32_t *chains, uint32_t *chains_returned);
extern __eglMustCastToProperFunctionPointerType __real_eglGetProcAddress(const char *name);

static EGLBoolean (*real_swap)(EGLDisplay, EGLSurface);
static unsigned long swaps;

static EGLBoolean hook_swap(EGLDisplay dpy, EGLSurface surf)
{
	uint32_t refused[MIP_REFUSALS], copies[COPY_REASONS], queued = 0, full = 0, chains = 0;
	EGLBoolean r = real_swap(dpy, surf);

	v3d_mip_counters(refused, &queued, &full);
	v3d_copy_counters(copies);
	v3d_rt_images(NULL, NULL, NULL, NULL, NULL, &chains, NULL);
	fprintf(stderr, "gm2ms: swap %lu chains %u queued %u full %u refused", ++swaps, chains, queued, full);
	for (int i = 0; i < MIP_REFUSALS; i++)
		fprintf(stderr, " %u", refused[i]);
	fprintf(stderr, " copies");
	for (int i = 0; i < COPY_REASONS; i++)
		fprintf(stderr, " %u", copies[i]);
	fputc('\n', stderr);
	return r;
}

__eglMustCastToProperFunctionPointerType __wrap_eglGetProcAddress(const char *name)
{
	__eglMustCastToProperFunctionPointerType f = __real_eglGetProcAddress(name);

	if (f && !strcmp(name, "eglSwapBuffers")) {
		real_swap = (EGLBoolean (*)(EGLDisplay, EGLSurface))f;
		return (__eglMustCastToProperFunctionPointerType)hook_swap;
	}
	return f;
}
