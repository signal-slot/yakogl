/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gc_internal.h -- what the parts of the Vivante driver share.
 * Not an API: gc.h and gc_render.h are what callers outside src/vivante/ use.
 *
 *   kernel side   M1-M3 power, clocks, identity, reset, hardware init, MMU v2,
 *                 interrupts and events, the FE ring and submit: Rust
 *                 (driver/rust, gpu::vivante) behind the Zephyr shim
 *                 driver/drivers/gpu/gpu_vivante_gc_rust.c
 *                 (driver/docs/vivante-kernel-rust.md)
 *   gc_cmd.c          command emission
 *   gc_draw.c     M4/M5 the clear and triangle command streams
 *   gc_render.c       the carveout allocator and the service in gc_render.h
 *
 * struct gc_dev's fields up to dump_seen are shared with Rust, and their
 * offsets are pinned there.
 */
#ifndef GC_INTERNAL_H
#define GC_INTERNAL_H

#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>
#include <zephyr/device.h>
#include <zephyr/sys/sys_io.h>

#include "gc.h"
#include "gc_render.h"

/* ------------------------------------------------------------------ *
 * The carveout
 *
 * A fixed layout at the head of the device tree memory-region, then a bump
 * allocator for everything else. Fixed because the MMU has to be built before
 * any allocator exists: the page tables are the first thing in the carveout.
 *
 * GPU virtual addresses are 32 bits (MMU v2: a 1024-entry master TLB, each
 * entry covering 4 MiB, so 4 GiB of VA). The carveout is identity-mapped, so a
 * carveout GPU VA equals its physical address, which makes a stall dump
 * readable. Imported memory (a scan-out buffer, possibly above 4 GiB physical)
 * gets a VA from a separate window that no physical carveout address can
 * collide with.
 * ------------------------------------------------------------------ */
#define GC_PAGE_SIZE        0x1000u

#define GC_OFF_MTLB         0x000000u   /* 1024 x u32 master TLB */
#define GC_OFF_PTA          0x001000u   /* 512 x u64 page table array (secure) */
#define GC_OFF_SAFE_PAGE    0x002000u   /* the "bad page" the MMU redirects to */
#define GC_OFF_MMU_CMD      0x003000u   /* the MMU-enable stream (runs MMU off) */
#define GC_OFF_STLB         0x004000u   /* GC_STLB_COUNT x 4 KiB slave TLBs */
#define GC_STLB_COUNT       64u         /* covers 64 x 4 MiB = 256 MiB of VA */
#define GC_OFF_RING         0x044000u   /* the front end's idle loop + submits */
#define GC_RING_SIZE        0x010000u   /* 64 KiB */
#define GC_OFF_POOL         0x060000u   /* the rest: gc_render_alloc() */

/* GPU VA window for gc_render_map_phys(). Below the carveout's own physical
 * base (0x58000000 on the Verdin iMX8M Plus) and above the first 4 MiB, which
 * stays unmapped as a null-pointer guard. */
#define GC_IMPORT_VA_BASE   0x10000000u
#define GC_IMPORT_VA_SIZE   0x08000000u  /* 128 MiB */

/* ------------------------------------------------------------------ *
 * Events
 *
 * The hardware signals completion with an EVENT command in the stream: the
 * front end raises the interrupt and reports the id in INTR_ACKNOWLEDGE. There
 * are 30 ids (bits 0..29; bits 30 and 31 are the MMU exception and the AXI bus
 * error), so the service maps each one to a monotonic fence.
 * ------------------------------------------------------------------ */
#define GC_EVENT_COUNT      30u
/* All of them free. Spelled out rather than BIT_MASK(GC_EVENT_COUNT): Zephyr's
 * BIT_MASK(n) is (BIT(n) - 1), the Linux kernel's is (1 << (n % BITS_PER_LONG))
 * -- the same name for two different numbers -- and this driver is built as
 * both. See the note in tools/gclab/shim/gc_compat.h; that one cost a day. */
#define GC_EVENT_ALL        ((1u << GC_EVENT_COUNT) - 1u)

/* ------------------------------------------------------------------ *
 * Driver state shared between the translation units. One instance; the device
 * tree has a single vivante,gc node and the hardware has one 3D core.
 * ------------------------------------------------------------------ */
struct gc_dev {
	const struct device *dev;    /* the Zephyr device, or NULL off Zephyr */
	/* The identity the bring-up read, published with the register block.
	 * Carried here rather than fetched through `dev` so that a host with no
	 * Zephyr device (the Linux module in tools/gclab) can supply it too. */
	const struct gc_identity *id;
	mm_reg_t  regs;          /* the GPU register block (device memory) */
	uint8_t  *carveout;      /* CPU view, Normal non-cacheable */
	uint64_t  carveout_phys;
	size_t    carveout_size;

	/* MMU v2 */
	uint32_t *mtlb;          /* carveout + GC_OFF_MTLB */
	uint64_t *pta;           /* carveout + GC_OFF_PTA */
	uint32_t  stlb_used;     /* slave tables handed out so far */
	uint32_t  import_va;     /* bump pointer in the import VA window */
	bool      mmu_on;

	/* The allocation pool's own state lives in gc_render.c's `g_pool`
	 * (gc_render_pool.h), so that a host test can drive the same code. */

	/* the FE ring (kernel side) */
	uint32_t *ring;          /* CPU view of the ring */
	uint32_t  ring_gpu;      /* its GPU VA */
	uint32_t  ring_pos;      /* next free dword */
	uint32_t  wait_pos;      /* dword index of the WAIT of the idle loop */
	bool      fe_running;

	/* events and fences (kernel side / gc_render.c) */
	uint32_t  event_free;              /* bitmap: 1 = free */
	uint32_t  event_fence[GC_EVENT_COUNT]; /* fence waiting on each event */
	uint32_t  fence_next;              /* next fence to hand out (never 0) */
	/* Written by the interrupt, polled by gc_render_wait() outside the
	 * lock, so the compiler must not keep them in a register. */
	volatile uint32_t fence_done;      /* highest completed fence */
	volatile uint32_t irq_errors;      /* AXI bus errors + MMU exceptions seen */
	/* What a run spent its time waiting for, for a board with no console:
	 * the only way these numbers travel is a hold code. */
	volatile uint32_t fence_timeouts;  /* gc_render_wait gave up */
	uint32_t max_quiet_ms;             /* longest wait-time with nothing retiring */
	/* The highest fence a reset abandoned, and how many resets have run.
	 * A fence at or below fence_lost never produced its picture, however
	 * far fence_done has since advanced: see gc_render_recover(). */
	volatile uint32_t fence_lost;
	volatile uint32_t resets;
	uint32_t idle_resets;              /* gc_render_reset_idle between clients */
	volatile uint32_t event_busy;      /* all 30 events were in flight */
	/* Milliseconds spent inside the two waits, which is the number the
	 * counters above only stand in for: gc_event_alloc loops rather than
	 * giving up, so a submit can wait a long time for an event without
	 * event_busy ever moving. */
	volatile uint32_t wait_ms;
	/* Performance counters (gc_render_perf): when each event's kick was
	 * submitted, and what the interrupt saw. Busy time is the union of the
	 * kicks' submit..retire intervals, so two queued kicks are not counted
	 * twice; kicks run in order on the one front end. */
	uint64_t  event_t0_ns[GC_EVENT_COUNT];
	uint64_t  perf_last_done_ns;
	volatile uint64_t perf_busy_ns;
	volatile uint32_t perf_kicks_done;
	uint64_t  perf_wait_ns;
	uint32_t  perf_waits;
	uint32_t  perf_wait_sleeps;
	/* Frees of a range that was already in the free list: each one would
	 * have put two allocations on the same memory. */
	uint32_t  pool_double_frees;
	/* The previous stall dump's idle state and completed fence, so a dump
	 * can say whether anything moved since the last one. */
	uint32_t  dump_idle;
	uint32_t  dump_fence;
	bool      dump_seen;
	struct k_sem event_room;           /* signalled when an event is freed */
	struct k_mutex submit_lock;        /* one writer of the ring at a time */
	struct k_spinlock lock;
};

/*
 * Has the caller been asked to abandon a wait? A host that can interrupt a
 * thread (a Linux module: a signal) defines this before this header is
 * included; on Zephyr, where a driver wait is not interruptible, it is
 * constant false and the checks compile away.
 */
#ifndef gc_wait_aborted
#define gc_wait_aborted() false
#endif

/* A monotonic clock in nanoseconds for the performance counters. Zephyr: the
 * architectural timer (8 MHz on the i.MX8MP, 125 ns steps); a host defines
 * its own before this header (the Linux module: ktime_get_ns). */
#ifndef gc_now_ns
#define gc_now_ns() k_cyc_to_ns_floor64(k_cycle_get_64())
#endif

/* The one device. A static instance rather than dev->data: the interrupt
 * table needs its address as a compile-time constant. */
extern struct gc_dev g_gc;

static inline struct gc_dev *gc_dev_get(void)
{
	return &g_gc;
}

static inline uint32_t gc_rd(struct gc_dev *g, uint32_t off)
{
	return sys_read32(g->regs + off);
}

static inline void gc_wr(struct gc_dev *g, uint32_t off, uint32_t val)
{
	sys_write32(val, g->regs + off);
}

/* CPU view of a carveout GPU VA (identity-mapped), or NULL if it is not in
 * the carveout. Debug and command-stream patching only. */
void *gc_cpu_of(struct gc_dev *g, uint32_t gpu_va);

/* --- kernel side, M1: the device (gpu::vivante::hw and power) --- */
/* Hand the rest of the driver the register mapping and the carveout the device
 * found; called at the end of the M1 init, before gc_render_init() can run. */
void gc_dev_publish(const struct device *dev, mm_reg_t regs, uint64_t carveout_phys,
		    size_t carveout_size, const struct gc_identity *id);

/* Put the GPU back to the state a fresh reset leaves it in: isolate, soft
 * reset, and poll until every module reports idle. The bring-up runs it at
 * probe; gc_render_recover() runs the same code on a hang, which is the point
 * of it taking the register base rather than the bring-up's own struct. */
int gc_bringup_reset(mm_reg_t regs, const struct gc_identity *idp);

/* --- kernel side, M2: init, MMU, interrupt (gpu::vivante::hw, mmu, irq) --- */
uint32_t gc_rd_fe(struct gc_dev *g, uint32_t off); /* FE regs need a double read */
int  gc_hw_init(struct gc_dev *g);        /* the register writes reset leaves undone */
int  gc_mmu_init(struct gc_dev *g);       /* build the page tables (MMU still off) */
int  gc_mmu_map(struct gc_dev *g, uint32_t gpu_va, uint64_t phys, size_t size);
int  gc_mmu_enable(struct gc_dev *g);     /* the PTA sequence; needs the FE */
void gc_irq_connect(struct gc_dev *g);
int  gc_event_alloc(struct gc_dev *g, uint32_t fence, uint32_t timeout_ms);
void gc_mmu_dump(struct gc_dev *g);
void gc_idle_state_decode(uint32_t idle);  /* names the busy modules */

/* The interrupt's work, split out from the Zephyr ISR so that a host which
 * must say whether the interrupt was its own (Linux: IRQ_HANDLED vs IRQ_NONE,
 * without which a stuck line is never detected) can call it directly.
 * Returns true when this GPU raised something. */
bool gc_irq_handle(struct gc_dev *g);

/* --- kernel side, M3: the FE ring (gpu::vivante::cmdbuf) --- */
/* Start the front end on a command buffer. `prefetch` is in 64-bit words, the
 * unit the LINK and FE_COMMAND_CONTROL fields use. */
void gc_fe_start(struct gc_dev *g, uint32_t address, uint32_t prefetch);
int  gc_fe_wait_idle(struct gc_dev *g, uint32_t timeout_ms);
/* Build the MMU-enable stream at GC_OFF_MMU_CMD and return its size in bytes. */
uint32_t gc_cmd_build_pta_config(struct gc_dev *g, uint32_t pta_index);
int  gc_ring_init(struct gc_dev *g);      /* the WAIT/LINK idle loop, FE started */
int  gc_ring_submit(struct gc_dev *g, const struct gc_submit *s, uint32_t event,
		    uint32_t fence);
void gc_ring_dump(struct gc_dev *g);

/* --- command-stream writers (gc_cmd.c), used by gc_draw.c --- *
 * A cursor over a dword array. `n` counts dwords written; the writers never
 * check the limit, so a caller sizes its buffer from GC_CMD_* below. */
struct gc_cmd {
	uint32_t *buf;   /* CPU view */
	uint32_t  gpu;   /* GPU VA of buf[0] */
	uint32_t  n;     /* dwords written */
	uint32_t  max;   /* capacity in dwords; gc_cmd_room() checks it */
};

void gc_cmd_state(struct gc_cmd *c, uint32_t reg, uint32_t value);
void gc_cmd_state_fixp(struct gc_cmd *c, uint32_t reg, uint32_t value);
void gc_cmd_state_multi(struct gc_cmd *c, uint32_t reg, const uint32_t *vals, uint32_t count);
void gc_cmd_nop(struct gc_cmd *c);
void gc_cmd_end(struct gc_cmd *c);
void gc_cmd_event(struct gc_cmd *c, uint32_t event_id, bool from_pe);
void gc_cmd_semaphore(struct gc_cmd *c, uint32_t from, uint32_t to);
void gc_cmd_stall(struct gc_cmd *c, uint32_t from, uint32_t to);
void gc_cmd_link(struct gc_cmd *c, uint32_t address, uint32_t prefetch);
void gc_cmd_wait(struct gc_cmd *c, uint32_t delay);
void gc_cmd_draw(struct gc_cmd *c, uint32_t prim_type, uint32_t start, uint32_t count);
void gc_cmd_align8(struct gc_cmd *c);   /* pad with NOP to a 64-bit boundary */
bool gc_cmd_room(const struct gc_cmd *c, uint32_t dwords);

#endif /* GC_INTERNAL_H */
