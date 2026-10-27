/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* shim.h - shim.c's entry points (README.md). */
#ifndef GM2_SHIM_H
#define GM2_SHIM_H

#ifdef __cplusplus
extern "C" {
#endif

/* what shared-library.o calls after run.sh's objcopy --redefine-sym */
void *gm2_dlopen(const char *name, int flags);
void *gm2_dlsym(void *handle, const char *symbol);
int gm2_dlclose(void *handle);
char *gm2_dlerror(void);

/* glDrawArrays + glDrawElements calls glmark2 made so far */
unsigned long gm2_draws(void);

#ifdef __cplusplus
}
#endif

#endif
