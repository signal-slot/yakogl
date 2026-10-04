/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * pvr_backend.h - the boundary between the GL core (gl/draw.rs) and the PowerVR
 * command generator. gl/draw.rs resolves the current GL context into one
 * `pvr_draw` and calls pvr_backend_draw(); the backend records the draw into
 * the current frame's command streams (VDM/PPP/PDS in GPU memory, via OSAL)
 * and pvr_backend_flush() submits the frame as one GEOM+FRAG kick.
 *
 * One kick per frame (not per draw) keeps the on-chip depth buffer and the
 * colour tiles coherent across the draw calls of a frame. When a frame has
 * to be split (draw table / arena limits) or does not start with a clear,
 * the next kick loads the colour back through a full-screen texel-fetch
 * quad and the depth/stencil through the ISP Z load/store unit (a packed
 * D24S8 buffer per target, pvr_draw.ds_persist).
 *
 * This keeps the GL core independent of PowerVR command encoding, and keeps the
 * (silicon-proven) command generator independent of GL. All GPU memory is
 * allocated through the OSAL; the backend never calls an OS API directly.
 */
#ifndef PVR_BACKEND_H
#define PVR_BACKEND_H

#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>
#include "../backend.h"

#ifdef __cplusplus
extern "C" {
#endif

/* The PowerVR implementation of the backend contract (backend.h). */
extern const struct yakogl_backend yakogl_backend_pvr;

int  pvr_backend_init(void);
void pvr_backend_shutdown(void);
int  pvr_backend_draw(const struct gles_draw *draw);
int  pvr_backend_flush(void);
int  pvr_backend_finish(void);
int  pvr_backend_finish_previous(void);
uint32_t pvr_backend_kick_id(void);
void pvr_backend_counters(uint32_t *kicks, uint32_t *waits);
void pvr_backend_workload(uint32_t *draws, uint32_t *tiles);
/* What the kicks carried beyond the application's draws: full-screen quads the
 * driver added (colour reload / clear) and kicks a frame was split into. */
void pvr_backend_overhead(uint32_t *quads, uint32_t *splits);
bool pvr_backend_kick_pending(uint32_t kick_id);
/* The newest kick that may read GPU memory freed now: last_submitted + 1
 * while a frame is being recorded, last_submitted otherwise. */
uint32_t pvr_backend_free_fence(uint32_t last_submitted);
/* The frame sets the next backend init builds (CONFIG_YAKOGL_PVR_FRAME_SETS;
 * default 4 x (1024 KiB general + 768 KiB pds)): 0, -EBUSY once the backend
 * runs with another, -EINVAL outside 1..4 sets, 16..4096 KiB general,
 * 8..1024 KiB pds. */
int pvr_backend_set_frame_arenas(uint32_t sets, uint32_t gen_kib, uint32_t pds_kib);
/* The backend's owner lines ("pool owner backend ..."), returns their KiB. */
uint64_t pvr_backend_pool_owners(void);
/* The GL core's owner lines ("pool owner gl ..."), returns their KiB. */
uint64_t yakogl_pool_owners(void);
/* Fence waits that timed out (the kick still incomplete after WAIT_MS). */
uint32_t pvr_backend_wait_timeouts(void);
/* Draws and clears refused (GL_OUT_OF_MEMORY) because their depth/stencil
 * buffer could not be allocated, or its contents were lost that way. */
uint32_t pvr_backend_ds_refused(void);
/* The depth/stencil line a run ends with, into buf (cap bytes, NUL-terminated,
 * truncated): "ds slots peak N/8, ds bytes peak M, ds refused R (slots full F,
 * no buffer B)", or "ds slots: none allocated, ..." when no buffer ever was. */
void pvr_backend_ds_report(char *buf, size_t cap);
uint64_t pvr_backend_heap_base(enum gles_heap heap);

/* Bring-up options (defaults: all 0). */
enum pvr_backend_option {
	/* 1: vertex positions are already in screen space (x,y pixels, z, w=1)
	 * and the TA must not apply the viewport transform (PPP pretransform).
	 * Used to reproduce the silicon test rig through the GL path. */
	PVR_BACKEND_OPT_PRETRANSFORM = 0,
	/* The TA clip mode: PVR_DEPTH_CLIP_* below. Unset (0) is FRONT_REAR --
	 * near and far clipping, as GL ES 2.0 2.11 asks for it.
	 *
	 * It was off for a while because FRONT_REAR appeared to over-clip
	 * (dEQP's clip_z cases rendered a sliver of the reference). The volume
	 * turned out to be selected by CR_PPP_CTRL bit 0, which we inherited
	 * clear from Mesa -- a VULKAN driver, and Vulkan clips 0 <= z <= w.
	 * With bit 0 set (PPP_CTRL 0x203, measured by z_probe) the TA clips
	 * GL's -w <= z <= w and FRONT_REAR is simply right. See
	 * driver/docs/gpu-recipes/depth-clip-convention.md. */
	PVR_BACKEND_OPT_DEPTH_CLIP,
	/* CR_PPP_CTRL value override (0 = the default for the current mode). */
	PVR_BACKEND_OPT_PPP_CTRL,
	/* 1: never merge adjacent attributes into one DMA stream (one DDMAD per
	 * attribute); 0: merge attributes that are contiguous in memory and in
	 * their vi registers (fewer DMAs, the layout the test rig used). */
	PVR_BACKEND_OPT_NO_COALESCE,
	/* 1: log every generated PPP/VDM/PDS word (bring-up comparison). */
	PVR_BACKEND_OPT_DEBUG,
	/* Raw TA_STATE_TERMINATE0 override for the trailing PPP block (0 = computed
	 * from the target size). Bring-up bisection knob. */
	PVR_BACKEND_OPT_TERM0,
	/* RETIRED (was NO_SUBMIT, the A/B hand-off to the self-test rig, which
	 * went with pvrgpu.ko): the slot keeps its number so the later options
	 * keep theirs; setting it changes nothing. */
	PVR_BACKEND_OPT_RETIRED_NO_SUBMIT,
	/* 1: after every kick, log a summary of the target's stored depth/stencil
	 * buffer (bring-up aid for the ISP Z load/store path). */
	PVR_BACKEND_OPT_DS_DUMP,
	/* Primitives one kick may tile before the frame is split (0 = the
	 * built-in budget). A test can set it low to exercise the split on a
	 * workload small enough for any parameter buffer. */
	PVR_BACKEND_OPT_PRIM_BUDGET,
	/* Bring-up: the step between one colour attachment's source position in
	 * the shader's output registers and the next (0 = the format's own
	 * register count). */
	PVR_BACKEND_OPT_MRT_STRIDE,
	/* 1: sample every texture from level 0, whatever chain it has (the
	 * sampler's level count is forced to one). An A/B for a texture that
	 * renders flat where it should show a minified pattern. */
	PVR_BACKEND_OPT_FORCE_MIP0,
	/* 1: the ISP takes glPolygonOffset's constant in integer depth units
	 * (backend.rs PVR_BACKEND_OPT_DBIAS_INT and pvr_dbias_int; default 1
	 * since the AM62P A/B, RESULTS 1i; 0 is the old encoding).
	 * Missing here until 2026-09-23, so no C client could set it. */
	PVR_BACKEND_OPT_DBIAS_INT,
	/* PROBE ONLY: 1 = colour target 0 written TWIDDLE_2D (backend.rs
	 * PVR_BACKEND_OPT_PBE_TWIDDLE; set by cube_probe.c, retired with pvrgpu.ko,
	 * in the history at 8b53557f). */
	PVR_BACKEND_OPT_PBE_TWIDDLE,
	/* MEASUREMENT: 1 = one kick in flight: the swap waits for its own kick
	 * too (backend.rs PVR_BACKEND_OPT_ONE_IN_FLIGHT; pvr-fence-wait.md). */
	PVR_BACKEND_OPT_ONE_IN_FLIGHT,
	/* 1 = the swap drops the window's depth/stencil: the swapped frame's
	 * kick stores none (backend.rs PVR_BACKEND_OPT_SWAP_DROPS_ZS). */
	PVR_BACKEND_OPT_SWAP_DROPS_ZS,
	PVR_BACKEND_OPT_COUNT
};

/* PVR_BACKEND_OPT_DEPTH_CLIP values. 1 and 2 are the ones z_probe and
 * gl_test case 62 already pass, so they keep meaning what they meant when
 * those measurements were taken; 0 selects the default, which is why turning
 * clipping OFF is an explicit value rather than zero. */
#define PVR_DEPTH_CLIP_FRONT_REAR        1u  /* near and far, GL's volume */
#define PVR_DEPTH_CLIP_FRONT_REAR_DEPTH  2u  /* clips nothing on its own (measured) */
#define PVR_DEPTH_CLIP_OFF               3u  /* no near/far clipping; bring-up only */
void pvr_backend_set_option(enum pvr_backend_option opt, uint32_t value);

#ifdef __cplusplus
}
#endif

#endif /* PVR_BACKEND_H */
