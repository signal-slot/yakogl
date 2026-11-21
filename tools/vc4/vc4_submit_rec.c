/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * vc4_submit_rec: an LD_PRELOAD library that records what Mesa's gallium vc4
 * driver submits, for tools/vc4/drm_shim_ref.sh. It goes in front of Mesa's
 * drm-shim (libvc4_noop_drm_shim.so), whose DRM_VC4_SUBMIT_CL is a stub, and
 * writes each DRM_IOCTL_VC4_SUBMIT_CL out as raw bytes before handing the
 * ioctl on to the shim unchanged.
 *
 * VC4_SUBMIT_REC=<prefix> names the output; without it the library records
 * nothing. For the N-th submit (from 1):
 *
 *   <prefix>.submitN.struct.bin  struct drm_vc4_submit_cl as passed, with its
 *                                four user pointers (bin_cl, shader_rec,
 *                                uniforms, bo_handles) set to 0
 *   <prefix>.submitN.bcl.bin     the binner control list, bin_cl_size bytes
 *   <prefix>.submitN.shader_rec.bin  the shader records, shader_rec_size bytes
 *   <prefix>.submitN.uniforms.bin    the uniform streams, uniforms_size bytes
 *   <prefix>.submitN.bo_handles.bin  the BO handle table, 4 * bo_handle_count
 *   <prefix>.submitN.bo<H>.bin   the contents of BO handle H at submit time,
 *                                for each handle in the table: a shader BO's
 *                                QPU code as DRM_IOCTL_VC4_CREATE_SHADER_BO
 *                                passed it (the shim keeps no copy), another
 *                                BO's bytes through Mesa's own mapping of it,
 *                                up to its last non-zero byte (the rest is
 *                                zero; the .txt gives the BO's size); a BO
 *                                Mesa never mapped has no file
 *   <prefix>.submitN.txt         every field of the struct, by name, and the
 *                                BO table (index, handle, kind, size, file)
 *
 * The structure is the kernel ABI of include/drm-uapi/vc4_drm.h (Mesa's copy,
 * compiled against here). Handles and mmap offsets come from the shim, which
 * hands out the lowest free handle (drm-shim/device.c drm_shim_bo_get_handle),
 * so they repeat run to run; the user pointers do not, hence the zeroing.
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/types.h>

#include "drm-uapi/drm.h"
#include "drm-uapi/vc4_drm.h"

#define MAX_HANDLES 4096

enum bo_kind { BO_NONE, BO_PLAIN, BO_SHADER };

static struct rec_bo {
	enum bo_kind kind;
	uint32_t size;
	uint64_t mmap_offset; /* from DRM_IOCTL_VC4_MMAP_BO, 0 = none yet */
	void *map;	      /* Mesa's mapping, NULL = none */
	void *code;	      /* a shader BO's bytes */
} bos[MAX_HANDLES];

static int drm_fd = -1;
static unsigned submits;

typedef int (*ioctl_fn)(int, unsigned long, ...);
typedef void *(*mmap_fn)(void *, size_t, int, int, int, off_t);
typedef int (*munmap_fn)(void *, size_t);

static void *next(const char *name)
{
	void *f = dlsym(RTLD_NEXT, name);
	if (!f) {
		fprintf(stderr, "vc4_submit_rec: no next %s\n", name);
		abort();
	}
	return f;
}

static int write_file(const char *prefix, unsigned n, const char *what, const void *data,
		      size_t size)
{
	char path[4096];
	snprintf(path, sizeof(path), "%s.submit%u.%s.bin", prefix, n, what);
	FILE *f = fopen(path, "wb");
	if (!f || (size && fwrite(data, 1, size, f) != size) || fclose(f)) {
		fprintf(stderr, "vc4_submit_rec: cannot write %s\n", path);
		abort();
	}
	return 0;
}

static void print_surface(FILE *t, const char *name, const struct drm_vc4_submit_rcl_surface *s)
{
	fprintf(t, "%s.hindex 0x%08x\n%s.offset 0x%08x\n%s.bits 0x%04x\n%s.flags 0x%04x\n", name,
		s->hindex, name, s->offset, name, s->bits, name, s->flags);
}

static void record_submit(const struct drm_vc4_submit_cl *in)
{
	const char *prefix = getenv("VC4_SUBMIT_REC");
	if (!prefix || !*prefix)
		return;
	unsigned n = ++submits;

	struct drm_vc4_submit_cl s = *in;
	s.bin_cl = s.shader_rec = s.uniforms = s.bo_handles = 0;
	write_file(prefix, n, "struct", &s, sizeof(s));
	write_file(prefix, n, "bcl", (const void *)(uintptr_t)in->bin_cl, in->bin_cl_size);
	write_file(prefix, n, "shader_rec", (const void *)(uintptr_t)in->shader_rec,
		   in->shader_rec_size);
	write_file(prefix, n, "uniforms", (const void *)(uintptr_t)in->uniforms,
		   in->uniforms_size);
	const uint32_t *handles = (const uint32_t *)(uintptr_t)in->bo_handles;
	write_file(prefix, n, "bo_handles", handles, 4 * (size_t)in->bo_handle_count);

	char path[4096];
	snprintf(path, sizeof(path), "%s.submit%u.txt", prefix, n);
	FILE *t = fopen(path, "w");
	if (!t) {
		fprintf(stderr, "vc4_submit_rec: cannot write %s\n", path);
		abort();
	}
	fprintf(t, "# struct drm_vc4_submit_cl (include/drm-uapi/vc4_drm.h), %zu bytes; "
		   "the four pointers are 0 in .struct.bin\n", sizeof(s));
	fprintf(t, "bin_cl_size %u\nshader_rec_size %u\nshader_rec_count %u\n"
		   "uniforms_size %u\nbo_handle_count %u\n",
		in->bin_cl_size, in->shader_rec_size, in->shader_rec_count, in->uniforms_size,
		in->bo_handle_count);
	fprintf(t, "width %u\nheight %u\nmin_x_tile %u\nmin_y_tile %u\nmax_x_tile %u\n"
		   "max_y_tile %u\n",
		in->width, in->height, in->min_x_tile, in->min_y_tile, in->max_x_tile,
		in->max_y_tile);
	print_surface(t, "color_read", &in->color_read);
	print_surface(t, "color_write", &in->color_write);
	print_surface(t, "zs_read", &in->zs_read);
	print_surface(t, "zs_write", &in->zs_write);
	print_surface(t, "msaa_color_write", &in->msaa_color_write);
	print_surface(t, "msaa_zs_write", &in->msaa_zs_write);
	fprintf(t, "clear_color 0x%08x 0x%08x\nclear_z 0x%08x\nclear_s 0x%02x\nflags 0x%08x\n"
		   "seqno %llu\nperfmonid %u\nin_sync %u\nout_sync %u\n",
		in->clear_color[0], in->clear_color[1], in->clear_z, in->clear_s, in->flags,
		(unsigned long long)in->seqno, in->perfmonid, in->in_sync, in->out_sync);
	fprintf(t, "# BO table: index handle kind size file (a plain BO's file stops at its "
		   "last non-zero byte)\n");
	for (uint32_t i = 0; i < in->bo_handle_count; i++) {
		uint32_t h = handles[i];
		const struct rec_bo *bo = h < MAX_HANDLES ? &bos[h] : NULL;
		const char *kind = !bo || bo->kind == BO_NONE ? "unknown"
				   : bo->kind == BO_SHADER    ? "shader"
							      : "plain";
		const void *data = bo && bo->kind == BO_SHADER ? bo->code
				   : bo && bo->kind == BO_PLAIN ? bo->map
								: NULL;
		char what[32];
		snprintf(what, sizeof(what), "bo%u", h);
		/* a handle listed twice is written once */
		bool seen = false;
		for (uint32_t j = 0; j < i; j++)
			seen |= handles[j] == h;
		/* a plain BO is written up to its last non-zero byte (the shim's
		 * memory starts zeroed; Mesa's upload buffer is 1 MiB) */
		size_t len = data ? bo->size : 0;
		if (data && bo->kind == BO_PLAIN)
			while (len && ((const uint8_t *)data)[len - 1] == 0)
				len--;
		if (data && !seen)
			write_file(prefix, n, what, data, len);
		fprintf(t, "bo %u %u %s %u %s\n", i, h, kind, bo ? bo->size : 0,
			data ? what : "(not mapped)");
	}
	if (fclose(t)) {
		fprintf(stderr, "vc4_submit_rec: cannot write %s\n", path);
		abort();
	}
}

int ioctl(int fd, unsigned long request, ...)
{
	static ioctl_fn real;
	if (!real)
		real = (ioctl_fn)next("ioctl");
	va_list ap;
	va_start(ap, request);
	void *arg = va_arg(ap, void *);
	va_end(ap);

	if (_IOC_TYPE(request) != DRM_IOCTL_BASE)
		return real(fd, request, arg);

	void *code = NULL;
	if (request == DRM_IOCTL_VC4_SUBMIT_CL) {
		record_submit(arg);
	} else if (request == DRM_IOCTL_VC4_CREATE_SHADER_BO) {
		const struct drm_vc4_create_shader_bo *c = arg;
		code = malloc(c->size);
		if (!code)
			abort();
		memcpy(code, (const void *)(uintptr_t)c->data, c->size);
	}

	int ret = real(fd, request, arg);
	if (ret != 0) {
		free(code);
		return ret;
	}

	if (request == DRM_IOCTL_VC4_CREATE_BO) {
		const struct drm_vc4_create_bo *c = arg;
		if (c->handle < MAX_HANDLES) {
			drm_fd = fd;
			free(bos[c->handle].code);
			bos[c->handle] = (struct rec_bo){ .kind = BO_PLAIN, .size = c->size };
		}
	} else if (request == DRM_IOCTL_VC4_CREATE_SHADER_BO) {
		const struct drm_vc4_create_shader_bo *c = arg;
		if (c->handle < MAX_HANDLES) {
			drm_fd = fd;
			free(bos[c->handle].code);
			bos[c->handle] = (struct rec_bo){
				.kind = BO_SHADER, .size = c->size, .code = code
			};
			code = NULL;
		}
	} else if (request == DRM_IOCTL_VC4_MMAP_BO) {
		const struct drm_vc4_mmap_bo *m = arg;
		if (m->handle < MAX_HANDLES)
			bos[m->handle].mmap_offset = m->offset;
	} else if (request == DRM_IOCTL_GEM_CLOSE) {
		const struct drm_gem_close *c = arg;
		if (c->handle < MAX_HANDLES) {
			free(bos[c->handle].code);
			bos[c->handle] = (struct rec_bo){ 0 };
		}
	}
	free(code);
	return ret;
}

static void note_map(int fd, off_t offset, void *map)
{
	if (fd < 0 || fd != drm_fd || map == MAP_FAILED || !offset)
		return;
	for (unsigned h = 1; h < MAX_HANDLES; h++) {
		if (bos[h].kind != BO_NONE && bos[h].mmap_offset == (uint64_t)offset)
			bos[h].map = map;
	}
}

void *mmap(void *addr, size_t length, int prot, int flags, int fd, off_t offset)
{
	static mmap_fn real;
	if (!real)
		real = (mmap_fn)next("mmap");
	void *map = real(addr, length, prot, flags, fd, offset);
	note_map(fd, offset, map);
	return map;
}

void *mmap64(void *addr, size_t length, int prot, int flags, int fd, off_t offset)
{
	static mmap_fn real;
	if (!real)
		real = (mmap_fn)next("mmap64");
	void *map = real(addr, length, prot, flags, fd, offset);
	note_map(fd, offset, map);
	return map;
}

int munmap(void *addr, size_t length)
{
	static munmap_fn real;
	if (!real)
		real = (munmap_fn)next("munmap");
	for (unsigned h = 1; h < MAX_HANDLES; h++) {
		if (bos[h].map == addr)
			bos[h].map = NULL;
	}
	return real(addr, length);
}
