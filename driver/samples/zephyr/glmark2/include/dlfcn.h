/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * dlfcn.h for the glmark2 image: glmark2's SharedLibrary (shared-library.cpp)
 * opens libEGL / libGLESv2 with dlopen and takes every entry point with
 * dlsym. In the image YakoGL is linked in, so "opening" a library always
 * succeeds and dlsym is eglGetProcAddress, which answers for every EGL and GL
 * entry point YakoGL has (src/gl/egl.rs, the PROCS table). src/platform.c.
 */
#ifndef GLMARK2_DLFCN_H
#define GLMARK2_DLFCN_H

#ifdef __cplusplus
extern "C" {
#endif

#define RTLD_LAZY     0x0001
#define RTLD_NOW      0x0002
#define RTLD_GLOBAL   0x0100
#define RTLD_LOCAL    0x0000
#define RTLD_NODELETE 0x1000

void *dlopen(const char *filename, int flags);
void *dlsym(void *handle, const char *symbol);
int dlclose(void *handle);
char *dlerror(void);

#ifdef __cplusplus
}
#endif

#endif /* GLMARK2_DLFCN_H */
