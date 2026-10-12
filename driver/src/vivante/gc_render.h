/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gc_render.h -- the Vivante GC7000 render service: everything a frame needs
 * that is not command-stream generation.
 *
 * It owns the GPU carveout (allocation and the MMU v2 mapping), the FE ring
 * buffer, the event/fence mechanism and the stall dump. It is the Vivante
 * sibling of src/pvr_render.h, and it is deliberately smaller: this GPU has no
 * firmware, no tiling and no render-pass bookkeeping, so a frame is one command
 * buffer that the front end fetches.
 *
 * Who calls it:
 *   - YakoGL's Vivante OSAL forwards gpu_alloc / submit / wait / completed
 *     here. The GL backend writes a command buffer into memory from
 *     gc_render_alloc() and hands its GPU address and size to
 *     gc_render_submit().
 *   - The bring-up samples (gc_fence, gc_clear, gc_triangle) use it directly.
 *
 * Nothing in this header refers to a register or a command opcode: the caller
 * only has to know "GPU-visible memory" and "a command buffer".
 *
 * Milestones M2..M5 of tools/cts/log/board-sessions/imx8mp-gc-bringup.md.
 * Compile-tested only; no
 * i.MX8MP board has run this yet.
 */
#ifndef GC_RENDER_H
#define GC_RENDER_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>
#include <zephyr/device.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * A block of GPU-visible memory.
 *
 * `gpu` is a 32-bit GPU virtual address: the MMU v2 address space is 32 bits
 * wide (one master TLB of 1024 entries, each covering 4 MiB). The carveout is
 * identity-mapped, so gpu == phys for carveout allocations; code must still use
 * `gpu` for anything the GPU reads, because an imported scan-out buffer above
 * 4 GiB gets a VA that is not its physical address.
 *
 * `cpu` points into the carveout's Normal non-cacheable CPU mapping, so a CPU
 * write is visible to the GPU without a cache flush. It is NULL for memory
 * imported with gc_render_map_phys().
 */
struct gc_bo {
	void     *cpu;   /* CPU view, or NULL for imported memory */
	uint32_t  gpu;   /* GPU virtual address */
	uint64_t  phys;  /* physical address (debug, and for imports) */
	size_t    size;  /* rounded-up size in bytes */
};

/* Command buffers and shader code must start on these boundaries; see
 * gpu::vivante::cmdbuf (the FE fetches 64-bit words) and gc_draw.c (the shader
 * instruction cache fetches 256-bit instructions). */
#define GC_ALIGN_CMDBUF   64u
#define GC_ALIGN_SHADER   64u
#define GC_ALIGN_DEFAULT  64u

/*
 * One submit: a command buffer the front end runs to completion.
 *
 * The buffer holds only the caller's own commands. The service wraps it with
 * the prologue and epilogue the hardware needs (pipe select, the cache flushes,
 * the FE/PE semaphore pair and the EVENT command that raises the fence), so the
 * caller must NOT emit an EVENT, an END or a LINK itself.
 */
struct gc_submit {
	uint32_t cmd_gpu;    /* GPU VA of the command buffer (GC_ALIGN_CMDBUF) */
	uint32_t cmd_bytes;  /* length in bytes; a multiple of 8 */
	uint32_t flags;      /* GC_SUBMIT_* */
};

/* The service links to the caller's buffer instead of copying it, so a large
 * draw stream costs nothing per frame. In exchange it writes a LINK back into
 * the ring in the eight bytes at `cmd_bytes`: the allocation has to be at
 * least cmd_bytes + GC_CMDBUF_TAIL long. Add it to every gc_render_alloc()
 * of a command buffer. */
#define GC_CMDBUF_TAIL  8u

/* What the epilogue has to write back before the fence is allowed to fire.
 * Leaving out a flush the render needed is the classic "the picture is right
 * but stale" bug, so naming NO cache means "all of them": colour, depth and
 * the level-1 shader cache. Name one or more only to save the others. */
enum gc_submit_flags {
	GC_SUBMIT_FLUSH_ALL   = 0,      /* name for "no flush bit set": see above */
	GC_SUBMIT_FLUSH_COLOR = 1 << 0, /* only the pixel engine's colour cache */
	GC_SUBMIT_FLUSH_DEPTH = 1 << 1,
	GC_SUBMIT_FLUSH_TEX   = 1 << 2,
	GC_SUBMIT_NO_PIPE_SEL = 1 << 3, /* the caller knows the 3D pipe is selected */
};

/* One-time initialisation. Runs the hardware init the reset left undone,
 * builds the MMU v2 page tables, starts the front end on its idle loop and
 * connects the interrupt. Idempotent; returns 0 or a negative errno.
 * `dev` is the vivante,gc device (DEVICE_DT_GET_ONE(vivante_gc)). */
int gc_render_init(const struct device *dev);

/*
 * The same initialisation without a Zephyr device.
 *
 * `gc_render_init()` uses its `dev` for exactly two things -- device_is_ready()
 * and gc_alive() -- and everything after that works on the one `struct gc_dev`.
 * A host that is not Zephyr (the Linux module in tools/gclab, which powers and
 * clocks the GPU through the kernel's genpd and clk drivers and then attaches
 * with gc_bringup_attach()) has already established both facts for itself and
 * has no Zephyr device to offer, so it calls this instead. The caller must have
 * published the register block and the carveout first (gc_dev_publish()).
 *
 * Idempotent, like the wrapper: a second call returns 0 without redoing
 * anything.
 */
struct gc_dev;
int gc_render_init_dev(struct gc_dev *g);

/*
 * How far gc_render_init() got, reported as it happens.
 *
 * On a board with no console a hang and a fault are the same event from
 * outside: nothing feeds the watchdog and the board comes back at a fixed
 * interval either way. Bisecting that through the return time costs one board
 * run per step. The driver does not know where a record could be kept, so
 * this is a weak no-op here and a sample that has somewhere durable to put it
 * overrides it (gc_common does, into the page the SoC reset does not clear).
 */
enum gc_render_step {
	GC_RENDER_STEP_ENTER = 1,
	GC_RENDER_STEP_DEV_READY,      /* device_is_ready and gc_alive passed */
	GC_RENDER_STEP_CARVEOUT,       /* map_carveout returned */
	GC_RENDER_STEP_HW,             /* gc_hw_init returned */
	GC_RENDER_STEP_MMU,            /* gc_mmu_init returned */
	GC_RENDER_STEP_IRQ,            /* gc_irq_connect returned */
	GC_RENDER_STEP_MMU_ON,         /* gc_mmu_enable returned */
	GC_RENDER_STEP_RING,           /* gc_ring_init returned; init is done */
};
void gc_render_mark(unsigned int step);

/* True once gc_render_init() has succeeded: the FE is running its idle loop
 * and submits will be fetched. */
bool gc_render_ready(void);

/* Allocate `size` bytes from the carveout, aligned to `align` (0 = default),
 * zeroed, and mapped in the GPU MMU. Returns 0 or -ENOMEM. */
int gc_render_alloc(size_t size, size_t align, struct gc_bo *out);

/* Return a block to the pool, for a later gc_render_alloc() to hand out
 * again. The block keeps its identity mapping, so reuse costs only the
 * zeroing. Blocks that turn out to be adjacent are merged; there is no
 * coalescing beyond that, and the free list holds a fixed number of blocks
 * (a full list logs and leaks rather than failing).
 *
 * The caller must know the GPU is done with the memory -- wait for the fence
 * of the last submit that read it. This matters more than it used to: a
 * freed block is handed straight out again, so freeing one the GPU is still
 * reading now corrupts a picture instead of merely leaking. YakoGL's OSAL
 * (yakogl/src/osal/zephyr_gc.c) holds its frees until the fence for exactly
 * that reason. */
void gc_render_free(const struct gc_bo *bo);

/* Return a block whose readers are known to be done: the caller has already
 * waited for the kick that was being recorded when the block was released
 * (the Zephyr OSAL's own deferral, src/osal/zephyr_gc.c). It goes back to the
 * pool at once. gc_render_free() would stamp it again with the next kick and
 * hold it one kick longer: on Zephyr every free was deferred twice, and
 * dEQP texture.size ran out of the carveout with 40 of its 43 live MiB held
 * that way (tools/cts/RESULTS.md, "What the 43 MiB are"). */
void gc_render_free_retired(const struct gc_bo *bo);

/* Map memory the caller owns -- a scan-out buffer outside the carveout -- into
 * the GPU MMU at a fresh VA. Consumes page-table entries but no carveout pages;
 * out->cpu is NULL. `phys` and `size` are rounded out to 4 KiB pages.
 *
 * A range that is already inside the carveout is returned as it stands, with
 * its identity VA and its CPU view: it is mapped, and a second CPU mapping of
 * it would be a cached alias of non-cacheable pages. */
int gc_render_map_phys(uint64_t phys, size_t size, struct gc_bo *out);

/* Cache maintenance for buffers that are NOT in the carveout: the carveout's
 * own mapping is Normal non-cacheable, so these are no-ops on it, but a caller
 * that hands in a cached scan-out buffer needs them. `flush` before the GPU
 * reads CPU writes, `invd` before the CPU reads GPU writes. */
void gc_render_cache_flush(void *cpu, size_t n);
void gc_render_cache_invd(void *cpu, size_t n);

/*
 * Submit one command buffer. On success *fence_out is a fence id that
 * gc_render_wait() accepts.
 *
 * Fences are a monotonically increasing 32-bit sequence, not the hardware event
 * id: the hardware has only 30 events and recycles them, so a raw event id
 * cannot be compared or stored. The service maps one fence to one event and
 * frees the event when the interrupt arrives; a submit blocks while no event is
 * free. Fence 0 is never handed out and means "nothing submitted yet".
 */
int gc_render_submit(const struct gc_submit *s, uint32_t *fence_out);

/*
 * Wait for a fence. Returns 0 when it completed, -ETIMEDOUT on the timeout
 * (the stall dump has already been logged), -EIO after an AXI bus error or an
 * MMU exception. timeout_ms == 0 polls once.
 *
 * A timeout no longer ends the run. It resets the GPU.
 *
 * This used to say the opposite, and the reasoning was sound at the time:
 * reclaiming a timed-out kick's hardware event slot is unsafe while the GPU
 * may still be running that kick, because a late interrupt from the old
 * stream would retire a new fence early and the OSAL, draining its deferred
 * frees against gc_render_completed(), would hand back memory the GPU was
 * still reading -- silent corruption instead of a slowdown. The conclusion
 * drawn was that recovery needs a reset "which this driver does not have".
 *
 * That last part was wrong. gc_bringup_reset() has been in the driver since
 * milestone 1 and every boot runs it; it was simply never reached from here.
 * gc_render_wait() now calls gc_render_recover() on a timeout, and after the
 * reset there is no old stream left to send a late interrupt, so every event
 * slot comes back safely.
 *
 * The cost of the old behaviour was measured, not imagined: a dEQP sweep on
 * the i.MX8MP wedged after 1,431 cases with twelve events in flight and
 * nothing retiring, and clearing it needed a module reload nobody on the team
 * had the rights to perform.
 *
 * A recovered wait returns **-ECANCELED**, not 0 and not -ETIMEDOUT: the kick
 * was thrown away, so there is no picture, and a GL layer should surface that
 * as a lost context. -ETIMEDOUT now means the narrower thing it should always
 * have meant -- the fence did not complete AND the recovery could not put the
 * GPU back, so the device is unusable.
 */
/* A timeout_ms of GC_WAIT_FOREVER has no deadline: gc_render_wait() returns
 * when the fence retires, the GPU faults, the GPU retires nothing for the hang
 * bound (gc_render_set_hang_ms(); it then recovers and returns -ECANCELED, or
 * -ETIMEDOUT if the recovery failed), or -- on a host that can interrupt a
 * thread -- the caller is asked to stop, which it reports as -EINTR. Every
 * other value is a bound in wall-clock milliseconds and is honoured exactly,
 * uninterrupted, so a caller that must finish its work (releasing memory the
 * GPU may still be reading) can rely on it.
 *
 * "Wall-clock" is not the default-off part. Before c1f0123 the Linux harness
 * counted msleep(1) iterations, each 20..32 ms at CONFIG_HZ=100, so every
 * bounded wait ran ~30x its stated value; the release path's "1000 ms" wait
 * for a killed client took ~32 s. It now takes 1 s, whatever hang_ms says. */
#define GC_WAIT_FOREVER 0xffffffffu

/* The pool's use now, in bytes: allocated (live), handed out once at least
 * (the bump past the base, the pool's high-water), its size, lost for good
 * (dropped), and the largest block an allocation can take now (the largest
 * free-list block or the room past the bump, alignment aside). Any pointer
 * may be NULL. For the OSAL's memory line (osal_mem_line). */
void gc_render_pool_usage(size_t *live, size_t *bumped, size_t *total, size_t *dropped, size_t *largest);

/* Put the pool back to its freshly-loaded state: the bump at the base, no free
 * blocks. Safe only when nothing this service allocated is still referenced --
 * i.e. between clients, which is what the chardev calls it for. */
void gc_render_pool_reset(void);

/* A GC_WAIT_FOREVER wait that sees no fence retire for this long treats the GPU
 * as hung: stall dump, recovery, -ECANCELED. 0 = never (the old behaviour).
 * Wall-clock milliseconds. 5000 since the regression set ran with it on the
 * i.MX8MP (2026-09-23, f110fd13eebc): the longest stretch any waiter saw with
 * work outstanding and nothing retiring, over m3, m4, gl_test 25/26 (the ~200 s
 * cases) and FemtoVG, was 21 ms (max_quiet_ms), so the bound has ~240x headroom;
 * a wedged client then gets -ECANCELED after 5 s instead of blocking until
 * something kills it. 0 = never (the gcgpu module parameter hang_ms). */
#ifndef GC_HANG_MS_DEFAULT
#define GC_HANG_MS_DEFAULT 5000u
#endif
void gc_render_set_hang_ms(uint32_t ms);
/* The longest interval, in wall-clock ms, that a deadline-free waiter saw with
 * work outstanding and no fence retiring: the quantity hang_ms is compared
 * against, so a default for it is chosen from this and not from a guess. */
uint32_t gc_render_max_quiet_ms(void);

/* The defer queue's full path: how often a free found it full, how many of
 * those found the oldest stamp not yet submitted (the wait cannot help), how
 * many blocks were leaked, and the longest wait inside a free. Occupancy and
 * the peak are gc_render_defer_stats(). */
struct gc_defer_full_stats {
	uint32_t full;
	uint32_t full_unsubmitted;
	uint32_t leaked;
	uint32_t max_wait_ms;
};
void gc_render_defer_full_stats(struct gc_defer_full_stats *out);

/* Turn the deferred free off, to reproduce the use-after-free deliberately
 * (gl_test case 20 after case 40 on the i.MX8MP). There is no call to turn it
 * on: deferring is the path, not an option. */
void gc_render_defer_disable(bool off);

/* Occupancy of the deferred-free queue: live entries, the high-water mark that
 * sizes GC_DEFER_SLOTS, and how many pushes it had to refuse (each of which
 * leaked a block rather than freeing it early). */
void gc_render_defer_stats(unsigned *live, unsigned *peak, unsigned *refused);

int gc_render_wait(uint32_t fence, uint32_t timeout_ms);

/*
 * Reset the GPU and bring the render service back up on it: the same
 * gc_bringup_reset() every boot runs, then the register, page-table, MMU and
 * ring initialisation, then the bookkeeping that retires the work the reset
 * threw away. `why` labels the log line.
 *
 * gc_render_wait() calls this on a timeout, so a caller rarely needs to; it is
 * exported for a test that wants to provoke a recovery, and for a client with
 * some other reason to believe the GPU has stopped.
 *
 * Returns 0 when the GPU is usable again, or a negative errno when it is not,
 * in which case the GPU is left stopped rather than half-initialised.
 *
 * Work submitted before the reset is retired in two different senses, because
 * two different questions are being asked about it. gc_render_completed()
 * advances over it -- the GPU is certainly not reading that memory any more,
 * which is what the OSAL's deferred free needs to know. gc_render_wait() on
 * one of those fences returns -ECANCELED -- the kick never produced its
 * result, which is what a caller needs to know. Merging the two would trade a
 * visible hang for invisible wrong output.
 */
int gc_render_recover(const char *why);

/* Reset an IDLE GPU between clients (no fence outstanding, else -EBUSY): the
 * same reset and re-init as the recovery, without abandoning anything and
 * without the recovery's log wording. Containment for register state one
 * client leaves behind for the next; see gc_render.c. */
int gc_render_reset_idle(void);

/* How many times gc_render_recover() has reset the GPU. Zero on a healthy
 * run, and the number a board with no console reports through a hold code. */
uint32_t gc_render_resets(void);

/* The highest fence the GPU has completed, without waiting (0 = none). */
uint32_t gc_render_completed(void);

/*
 * What a run spent its time waiting for. On a board with no console these are
 * the difference between "the suite is slow" and knowing which wait it is
 * slow in: fence_timeouts counts the 2 s waits gc_render_wait gave up on, and
 * event_busy the 1 s waits for one of the thirty hardware events to come free.
 * wait_ms is the time actually spent in both, which is what the other two only
 * stand in for: gc_event_alloc loops rather than giving up, so a submit can
 * wait a long time for an event without event_busy ever moving.
 */
void gc_render_counters(uint32_t *fence_timeouts, uint32_t *event_busy,
			uint32_t *wait_ms);

/*
 * Where a frame's time goes, for a performance measurement. Monotonic
 * totals since the service started; a reader takes differences.
 *   gpu_busy_ns   the union of every kick's submit..retire interval (the
 *                 interrupt's view of when the GPU had work)
 *   kicks_done    kicks retired
 *   wait_ns       time spent inside gc_render_wait() (any caller), waits the
 *                 number of waits that found the fence not yet done, and
 *                 wait_sleeps the 1 ms sleeps they took
 *   hi_*          the GPU's own free-running 32-bit counters, read now:
 *                 HI_PROFILE TOTAL_CYCLES, IDLE_CYCLES, READ_BYTES8 and
 *                 WRITE_BYTES8 (units of 8 bytes)
 */
struct gc_render_perf {
	uint64_t gpu_busy_ns;
	uint64_t wait_ns;
	uint32_t kicks_done;
	uint32_t waits;
	uint32_t wait_sleeps;
	uint32_t hi_total_cycles;
	uint32_t hi_idle_cycles;
	uint32_t hi_read_bytes8;
	uint32_t hi_write_bytes8;
	/* MC_PROFILE PE_READ, SH_READ and RA_READ: whatever signal
	 * gc_render_perf_select() put in MC_PROFILE_CONFIG0/1 (etnaviv
	 * perfmon's PE, SH and RA domains), free-running 32-bit */
	uint32_t mc_pe;
	uint32_t mc_sh;
	uint32_t mc_ra;
};
void gc_render_perf(struct gc_render_perf *out);
/* Write MC_PROFILE_CONFIG0 and CONFIG1 (hw/state_hi.xml.h field values). */
void gc_render_perf_select(uint32_t config0, uint32_t config1);

/* Log everything the hardware can say about why it is stuck: the FE's
 * current address and the last command it fetched, the idle state per module,
 * the DMA debug state, the MMU status and exception addresses, and the event
 * table. `why` labels the dump. Safe to call any time; used by
 * gc_render_wait() on a timeout. */
void gc_render_stall_dump(const char *why);

#ifdef __cplusplus
}
#endif

#endif /* GC_RENDER_H */
