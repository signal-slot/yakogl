/* SPDX-License-Identifier: Apache-2.0 */
/*
 * eglplatform.h - YakoGL's EGL platform types.
 *
 * There is no window system on the targets this library runs on (Zephyr,
 * bare metal). A native window is a description of a scan-out (or any other
 * GPU-mappable) pixel buffer the application owns; the native display is the
 * OSAL's platform argument (NULL selects the platform default).
 *
 * Threads: ONE thread issues every EGL and GL call of the process. That
 * thread may create any number of contexts and make them current in turn.
 * The library keeps no per-thread state (EGL error, bound API, current
 * context), so a call from a second thread, or two calls at once, is not
 * supported. Where the platform can tell threads apart (gles_osal.h,
 * thread_id), a call from another thread while a context is current is
 * refused: an EGL call returns its failure value and that thread's
 * eglGetError gives EGL_BAD_ACCESS; a GL call does nothing and that
 * thread's glGetError gives GL_INVALID_OPERATION. The first refusal is
 * logged once. The current thread's state is not touched.
 */
#ifndef __eglplatform_h_
#define __eglplatform_h_

#include <KHR/khrplatform.h>

#ifndef EGLAPI
#define EGLAPI
#endif
#ifndef EGLAPIENTRY
#define EGLAPIENTRY
#endif
#define EGLAPIENTRYP EGLAPIENTRY *

/* Pixel byte order of a native window buffer. */
#define GLES_NATIVE_FORMAT_RGBA8888 1u   /* R,G,B,A in memory (as FBO textures) */
#define GLES_NATIVE_FORMAT_XRGB8888 2u   /* B,G,R,X in memory (DRM XRGB8888 scan-out) */
#define GLES_NATIVE_FORMAT_RGB565   3u   /* 16-bit R5G6B5, red in the high bits (DRM RGB565) */
/* OR'd into a WINDOW's format: the display scans this buffer bottom row first
 * (the AM62P DSS with a negative VID_ROW_INC), so GL draws it y-up as it draws
 * an FBO and gl_FragCoord needs no mirroring. Refused on a native pixmap. */
#define GLES_NATIVE_WINDOW_BOTTOM_UP 0x80000000u
/* OR'd into a WINDOW's format: the application double-buffers this window
 * itself (two halves of one board window, each its own surface, the half
 * shown is never the half being drawn: driver/samples/zephyr/cluster), so
 * eglSwapBuffers may return with the frame it submitted still rendering and
 * wait only for the frame before it. Without it a window is shown as it is,
 * so eglSwapBuffers completes the frame before it returns: showing a buffer
 * the GPU is still writing put a black band of the last-drawn rows on the
 * i.MX8MP's panel in every glmark2 scene (2026-10-16). */
#define GLES_NATIVE_WINDOW_PIPELINED 0x40000000u

/* A native window: `phys` is the buffer's physical address, mapped for the
 * GPU by eglCreateWindowSurface; rows must be tightly packed
 * (stride_bytes == width * 4). The buffer stays owned by the application. */
struct gles_native_window {
	khronos_uint64_t phys;
	khronos_uint32_t width, height;
	khronos_uint32_t stride_bytes;
	khronos_uint32_t format;        /* GLES_NATIVE_FORMAT_* */
};

typedef struct gles_native_window *EGLNativeWindowType;

/* eglCreateImage target for a native buffer (the EGL_KHR_image_pixmap token):
 * the client buffer is a struct gles_native_window describing it. */
#ifndef EGL_NATIVE_PIXMAP_KHR
#define EGL_NATIVE_PIXMAP_KHR 0x30B0
#endif
typedef void *EGLNativeDisplayType;   /* the OSAL platform argument, or NULL */
typedef void *EGLNativePixmapType;    /* pixmaps are not supported */

/* EGL 1.2 names */
typedef EGLNativeDisplayType NativeDisplayType;
typedef EGLNativePixmapType  NativePixmapType;
typedef EGLNativeWindowType  NativeWindowType;

typedef khronos_int32_t EGLint;

/* egl.h builds its handle constants with this */
#if defined(__cplusplus)
#define EGL_CAST(type, value) (static_cast<type>(value))
#else
#define EGL_CAST(type, value) ((type)(value))
#endif

#endif /* __eglplatform_h_ */
