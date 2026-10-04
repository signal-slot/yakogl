/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * zephyr_v3d.h - what the Zephyr V3D OSAL (zephyr_v3d.c) offers a V3D sample
 * besides the gles_osal ops: instruments only. V3D-only, so the shared
 * osal_internal.h does not grow for one OSAL.
 */
#ifndef ZEPHYR_V3D_H
#define ZEPHYR_V3D_H

#include <stdbool.h>
#include <stdint.h>

/* The V3D OSAL's GPU memory (the carve-out's allocator), in bytes: used now
 * (frees held for the open job included), the most used at once since init,
 * and the allocator's total. Any pointer may be NULL. For a sample that
 * prints it per case (v3d_deqp's E runs: a per-program leak climbs, a
 * compiler-side fault does not). */
void osal_v3d_mem_used(uint32_t *used, uint32_t *high, uint32_t *total);

/* The carve-out allocator's free space as runs, in bytes: the longest free
 * run (the largest allocation that can succeed now, alignment aside), how
 * many runs, and the free total (held frees count as used). A large copy
 * that fails with total >= its size and longest < it is fragmentation.
 * Any pointer may be NULL. */
void osal_v3d_mem_free_runs(uint32_t *longest, uint32_t *runs, uint32_t *free_total);

/* The frees the OSAL holds now (v3d_frees.h: each waits for the newest job
 * that may read it) and their bytes (whole granules). Any pointer may be
 * NULL. With osal_v3d_mem_used and osal_v3d_mem_free_runs, a sample tells
 * live memory, held frees and fragmentation apart (glmark2's per-scene
 * "glmark2-mem:" line). */
void osal_v3d_frees_held(uint32_t *n, uint32_t *bytes);

/* The carve-out allocator's search since init (v3d_granules.h): its calls,
 * the map reads they made, and the most one call made. Any pointer may be
 * NULL. A sample that prints the deltas per interval gets the reads a call. */
void osal_v3d_find_counts(uint32_t *calls, uint64_t *reads, uint32_t *reads_max);

/* The job builder's packet skip (driver/src/v3d/v3d_job.h "Packet skip",
 * CONFIG_YAKOGL_V3D_PACK_SKIP): whether it is on, and over the jobs ended
 * since start, their count, the BCL packets their draws wrote, the state
 * packets they left out, the packet groups compared with what the list held,
 * and those that differed. Any pointer may be NULL. */
void osal_v3d_pack_counts(bool *on, uint32_t *jobs, uint32_t *emitted, uint32_t *skipped,
			  uint32_t *compared, uint32_t *mismatched);

/* The V3D OSAL's jobs since init: stages past JOB_SLOW_MS that still ended
 * (slow, their result stands); stages that did not end (timed out, the
 * overflow pool dry, an MMU fault: the GPU is then taken as hung, so at
 * most 1 without a reset); jobs not submitted because the GPU was hung.
 * Any pointer may be NULL. A sample printing the deltas per case names the
 * first failing case apart from the cases that only followed it. */
void osal_v3d_job_counts(uint32_t *slow, uint32_t *failed, uint32_t *refused);

/* GPU resets since init: after a stage that did not end, the OSAL resets the
 * V3D (v3d_reset) and goes on; the job's frame is lost. Every verdict names
 * this count (a run that recovered is never a clean pass). */
uint32_t osal_v3d_recoveries(void);

/* Stages ended by an MMU fault since init (the count osal_v3d_diag_print's
 * "MMU faults:" prints), and whether the GPU is taken as hung (a reset that
 * failed: nothing is submitted since). For a sample whose verdict must name
 * them rather than leave them absent (v3d_gl_cluster). */
uint32_t osal_v3d_mmu_faults(void);
bool osal_v3d_hung(void);

/* The last job's times, in microseconds (k_cycle_get_32, converted with
 * k_cyc_to_us_floor32), and the jobs submitted since init: prep (the tile
 * state cleared, the lists checked; after the previous job in flight was
 * finished), bin (the binner started to FLDONE), render (the renderer
 * started to the moment the OSAL saw FRDONE: submit() returns with the render
 * running, and the job is finished in wait(), completed() or the next
 * submit(), so render includes any time before the OSAL was asked; with a
 * backend that waits right after submit it is the render), clean (the TMU
 * cache write-back), total (submit()'s entry to the job's completion, the
 * four and the rest). Until the job completes, render, clean and total are
 * 0.
 * The GPU's work is bin + render; the backend's CPU work around the job is
 * the swap's time minus total. For v3d_gl_cluster's demo (the 60 fps plan's
 * step 0c). jobs is the fences handed out since init (a job refused while
 * the GPU is hung takes one; a kick rejected as -EINVAL does not). A stage
 * that failed keeps its time, the stages after it stay 0, and total includes
 * the recovery; a job refused before the binner started leaves all five 0. */
struct osal_v3d_job_us {
	uint32_t jobs;
	uint32_t prep, bin, render, clean, total;
	/* 1: FRDONE was already up when the OSAL first looked (the job's end
	 * was observed late: render, clean and total are the time until the
	 * OSAL asked, not the GPU's; the render's end has no interrupt here,
	 * V3D_BIN_IRQS). 0: the OSAL waited for FRDONE, so render is the
	 * render's start (the FLDONE interrupt) to its end within the wait's
	 * poll. bin is from the bin's submit to the FLDONE interrupt either
	 * way. */
	uint32_t render_late;
};
void osal_v3d_last_job_us(struct osal_v3d_job_us *out);

/* The same five times for the last job that COMPLETED, kept when it
 * completed; jobs is that job's fence (0: none completed yet). The job a
 * submit() starts runs on after it returns and the next submit() finishes it
 * and then zeroes osal_v3d_last_job_us's times for itself, so a caller that
 * reads right after a swap sees prep and four zeros there; this one keeps the
 * finished job's numbers until another job completes. */
void osal_v3d_last_completed_job_us(struct osal_v3d_job_us *out);

/* One TFU conversion (driver/src/v3d v3d_tfu_run), for the probe in
 * v3d_gl_test only (no product code submits one): the job's fields as the
 * caller packed them, run alone between jobs, polled. 0 or v3d_tfu_run's
 * errno; -ENOTSUP when HUB_IDENT1 says the V3D has no TFU; -EIO while the
 * GPU is taken as hung. An MMU fault or a timeout prints the registers,
 * counts as a failed stage and resets the GPU (osal_v3d_recoveries). */
struct v3d_tfu_job;
struct v3d_tfu_status;
int osal_v3d_tfu(const struct v3d_tfu_job *j, struct v3d_tfu_status *st);

#endif
