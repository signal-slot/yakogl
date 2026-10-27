/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * replay.h: what a sample's host replay program shares (replay_common.c).
 *
 * A replay program links a sample's Rust archive, the C side that archive
 * expects (heap, printk, clock), and libyakogl.a built for one GPU with the
 * Linux OSAL (src/osal/linux.c: GPU memory is malloc'd, every submit
 * completes at once, nothing is rendered). The archive's GL calls go through
 * replay_get_proc, which is YakoGL's own eglGetProcAddress, so every call of
 * the frame sequence reaches the real GL core and back end.
 */
#ifndef HOST_REPLAY_H
#define HOST_REPLAY_H

#include <stdint.h>

/* EGL up as the samples' main.c brings it up (RGBA8888 + stencil 8, ES 2.0,
 * a window over a malloc'd XRGB8888 buffer of w x h). 0 on success. */
int replay_egl_up(uint32_t w, uint32_t h);

/* The archive's loader. Logs every name YakoGL does not answer. */
const void *replay_get_proc(const char *name);

/* One frame done: eglSwapBuffers, then glGetError, then one line
 *   replay: frame <n> <label>: rc <rc> glerror 0x<e> refused <r> incomplete <i> draws <d>
 * r is the draws the library refused in this frame (yakogl_draws_refused),
 * i the draws of this frame that sampled an incomplete texture and so read
 * (0,0,0,1) (yakogl_incomplete_texture_draws; the library logs only the
 * first such texture of a process, the counter sees every draw), d the draws
 * the archive made (counted in the wrapped glDraw*). */
void replay_frame_end(const char *label, int rc);

/* The end line "replay: summary frames=<n> failed=<f> glerrors=<g> refused=<r>
 * incomplete=<i> unresolved=<u>"; returns the exit status: 1 when a frame
 * failed, a frame ended with a GL error, a draw was refused or a draw
 * sampled an incomplete texture. Unresolved names are informational. */
int replay_finish(void);

#endif
