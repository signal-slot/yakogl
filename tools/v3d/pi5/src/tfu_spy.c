/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * tfu_spy.c: prints the register words of every DRM_IOCTL_V3D_SUBMIT_TFU
 * Mesa makes, for the tools/v3d/pi5/gl_*.clif references of
 * tools/v3d/pi5/src/gl_shapes.py.
 *
 * Mesa's V3D_DEBUG=clif dumps only the control lists of render (CL) jobs; a
 * TFU job (glGenerateMipmap, v3dx_tfu.c) is a separate ioctl whose
 * registers are never written to the dump, and the drm-shim's SUBMIT_TFU is
 * a no-op (drm-shim/v3d_noop.c). gl_shapes.py builds this file into a shared
 * object and runs itself again with it ahead of the shim in LD_PRELOAD: this
 * ioctl() prints the struct drm_v3d_submit_tfu on stdout (fd 1, unbuffered,
 * so the line lands among gl_shapes.py's own) and hands the call on to the
 * next ioctl (the shim's), unchanged. Nothing else is touched.
 *
 * The line holds every register word Mesa fills, and the V3D 7.1 fields
 * decoded with the shifts of Mesa's src/broadcom/common/v3d_tfu.h (V3D71_*).
 * The extensions pointer is a process address and is printed only as zero or
 * not, so that two runs print the same bytes.
 *
 * Built by gl_shapes.py:
 *   cc -shared -fPIC -O2 -I third_party/mesa-26.2.1/include/drm-uapi \
 *      -o <libdir>/gl_shapes_tfu_spy.so tools/v3d/pi5/src/tfu_spy.c -ldl
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdarg.h>
#include <stdio.h>
#include <sys/ioctl.h>

#include "v3d_drm.h"

int ioctl(int fd, unsigned long request, ...)
{
	static int (*next)(int, unsigned long, ...);
	va_list ap;
	void *arg;

	va_start(ap, request);
	arg = va_arg(ap, void *);
	va_end(ap);
	if (!next)
		next = (int (*)(int, unsigned long, ...))dlsym(RTLD_NEXT, "ioctl");

	if (request == DRM_IOCTL_V3D_SUBMIT_TFU && arg) {
		const struct drm_v3d_submit_tfu *t = arg;
		const unsigned ioc = t->v71.ioc;

		dprintf(1,
			"gl_shapes: SUBMIT_TFU icfg 0x%08x iia 0x%08x iis 0x%08x ica 0x%08x "
			"iua 0x%08x ioa 0x%08x ios 0x%08x coef 0x%08x 0x%08x 0x%08x 0x%08x "
			"ioc 0x%08x bo_handles %u %u %u %u in_sync %u out_sync %u flags 0x%x "
			"extensions %s\n",
			t->icfg, t->iia, t->iis, t->ica, t->iua, t->ioa, t->ios, t->coef[0],
			t->coef[1], t->coef[2], t->coef[3], ioc, t->bo_handles[0],
			t->bo_handles[1], t->bo_handles[2], t->bo_handles[3], t->in_sync,
			t->out_sync, t->flags, t->extensions ? "set" : "0");
		dprintf(1,
			"gl_shapes: SUBMIT_TFU 7.1: ICFG IFORMAT %u OTYPE %u; IOS %ux%u; "
			"IOC DIMTW %u NUMMM %u FORMAT %u STRIDE %u\n",
			(t->icfg >> 23) & 0xf, (t->icfg >> 16) & 0x7f, t->ios & 0xffff,
			t->ios >> 16, ioc & 1, (ioc >> 4) & 0xf, (ioc >> 12) & 0xf, ioc >> 16);
	}
	return next(fd, request, arg);
}
