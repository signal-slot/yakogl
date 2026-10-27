/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Broadcom V3D 4.2 (Raspberry Pi 4) GPU driver: milestones M1..M3 of the
 * bring-up (docs/notes/pi4-v3d-bringup.md; the sequences
 * are driver/docs/rpi4-v3d.md). Power the V3D domain and identify the GPU
 * (M1), program its MMU (M2), and submit one binning + render job and wait for
 * it (M3).
 *
 * The registers are v3d_regs.h (hardware facts, named in the vocabulary of
 * Mesa's V3D simulator glue); each sequence in v3d_hw.c says what the
 * hardware needs from it, and names the Mesa simulator function that
 * programs the same thing where there is one.
 *
 * The same file builds for Zephyr and, through driver/tools/gclab-style shims,
 * for a Linux harness and a host mock test. Under Linux the kernel's genpd owns
 * the PM and ASB registers, so the harness skips v3d_power_on() (as gclab skips
 * gc_init()'s GPC/CCM part) and starts at v3d_identify().
 */
#ifndef V3D_H
#define V3D_H

#include <stdint.h>
#include <stdbool.h>
#include <zephyr/sys/sys_io.h>

/* The mapped register blocks. `pm` and `asb` are only used by v3d_power_on(). */
struct v3d_regs {
	mm_reg_t hub, core, pm, asb;
};

/*
 * The V3D clock is a VideoCore firmware clock (id 5 in the firmware's list),
 * switched through the property mailbox, which this driver does not own. The
 * caller passes the switch; it returns 0 or a negative errno.
 */
struct v3d_clock_ops {
	int (*set)(void *ctx, bool on);
	void *ctx;
};

struct v3d_identity {
	uint32_t hub_ident[4];     /* V3D_HUB_CTL_IDENT0..3, raw */
	uint32_t ctl_ident[3];     /* V3D_CTL_0_IDENT0..2, raw */
	uint32_t mmu_debug_info;   /* raw */
	uint32_t ver;              /* TVER * 10 + REV, 42 on the BCM2711 */
	uint32_t ncores;
	uint32_t qpu_count;        /* NSLC * QUPS */
	uint32_t vpm_size;         /* bytes, IDENT1.VPM_SIZE * 8192 */
	uint32_t pa_width, va_width; /* bits */
	bool has_mmu;
};

/* What the registers said when a wait ended without the bits it waited for. */
struct v3d_stall {
	uint32_t core_int_sts, hub_int_sts;
	uint32_t ct0ca, ct0ra, ct1ca, ct1ra;
	uint32_t err_stat, mmu_ctl, mmu_vio_addr;
	/* the faulting AXI client; read with the address, before the fault's
	 * HUB_INT_CLR (a read after it is not this fault's) */
	uint32_t mmu_vio_id;
};

/*
 * M1, Zephyr only: bring the V3D out of reset behind its AXI bridge: the
 * reset released under a running clock, then the bridge's master and slave
 * ports started. The graphics power island itself is the firmware's on the
 * BCM2711. Returns 0, -ENODEV if the bridge ID is wrong, -ETIMEDOUT if the
 * bridge does not acknowledge, or what the clock switch returned.
 */
int v3d_power_on(const struct v3d_regs *r, const struct v3d_clock_ops *clk);

/*
 * The opposite: the bridge's slave, then master port stopped (stop request
 * set, acknowledge rises), the clock off, the V3D held in reset. -ETIMEDOUT
 * when a port does not stop (the slave is started again when the master
 * does not).
 */
int v3d_power_off(const struct v3d_regs *r, const struct v3d_clock_ops *clk);

/*
 * A GPU reset after a stage that did not end. The BCM2711's only V3D reset
 * is the power manager's reset line, so this is a power cycle: interrupts
 * quiesced, v3d_power_off, v3d_power_on, the caches' state, the MMU on the
 * same page table, interrupts cleared. 0, or the first step's error.
 */
int v3d_reset(const struct v3d_regs *r, const struct v3d_clock_ops *clk, uint64_t pt_pa, uint64_t scratch_pa);

/* M1: read and check the identity (TVER 4, REV 2, one core, an MMU).
 * Returns 0 or -ENODEV; `id` is filled either way. */
int v3d_identify(const struct v3d_regs *r, struct v3d_identity *id);

/*
 * M2: the page table is one level of 32-bit entries, one per 4 KiB page of GPU
 * virtual address. `pt` is its CPU view, `pt_entries` its length.
 * v3d_pte_map() fills entries for [va, va + bytes) -> [pa, ...) and refuses
 * VA 0, an unaligned address, a range outside the table, or a physical page
 * number that does not fit 24 bits (returns -EINVAL, writes nothing).
 */
int v3d_pte_map(uint32_t *pt, uint32_t pt_entries, uint32_t va, uint64_t pa, uint32_t bytes,
		bool writeable);

/* M2: point the MMU at the table, turn it on, route faults to a scratch page
 * and flush. Returns 0 or -ETIMEDOUT. */
int v3d_mmu_init(const struct v3d_regs *r, uint64_t pt_pa, uint64_t scratch_pa);

/* Flush the MMU cache and TLB after the table changed. */
int v3d_mmu_flush(const struct v3d_regs *r);

/* Once after power-on: the range the texture L2 operations cover (all of it). */
void v3d_caches_init(const struct v3d_regs *r);

/* Before each job: invalidate what the GPU may hold stale (Mesa's simulator
 * glue: v3d_invalidate_caches). */
void v3d_caches_invalidate(const struct v3d_regs *r);

/* After a render the CPU will read: write back the TMU caches (Mesa's
 * simulator glue: v3d_flush_caches). */
int v3d_caches_clean(const struct v3d_regs *r);

/* Mask every core and hub interrupt and clear what is pending; M3 polls. */
void v3d_irq_quiesce(const struct v3d_regs *r);

/*
 * M3: start binning (Mesa's v3dX(simulator_submit_cl_ioctl) order). Addresses are GPU
 * virtual. The binner starts with no overflow pool (PTB_BPOS = 0).
 */
void v3d_submit_bin(const struct v3d_regs *r, uint32_t bcl_start, uint32_t bcl_end,
		    uint32_t tile_alloc, uint32_t tile_alloc_size, uint32_t tile_state);

/* The registers a failed stage's record prints (the core and hub interrupt
 * status, the control lists' addresses, the error and MMU state), read now:
 * for a stage whose end an interrupt reports rather than a wait. */
void v3d_stall_snapshot(const struct v3d_regs *r, struct v3d_stall *stall);

/* M3: start rendering (the same function's render half). */
void v3d_submit_render(const struct v3d_regs *r, uint32_t rcl_start, uint32_t rcl_end);

/*
 * M3: poll the core interrupt status until every bit of `bits` has been seen,
 * clearing what it reads. Returns 0; -ENOMEM if the binner ran out of memory
 * (OUTOMEM: M3 gives it no overflow pool, so that is its own outcome, never a
 * pass); -EFAULT on an MMU fault; -ETIMEDOUT after `timeout_us`. On any error
 * `stall`, if given, holds the registers read at that moment.
 */
int v3d_wait(const struct v3d_regs *r, uint32_t bits, uint32_t timeout_us, struct v3d_stall *stall);

/*
 * The binner's overflow pool: blocks of `block_size` bytes at GPU addresses
 * `blocks[0..nblocks)`, mapped already. `used` counts the blocks handed out
 * during one bin (the caller zeroes it before submitting).
 */
struct v3d_bin_overflow {
	const uint32_t *blocks;
	uint32_t nblocks;
	uint32_t block_size;
	uint32_t used;
};

/*
 * Wait for the binning to finish (FLDONE), handing the binner overflow memory
 * when it runs out: on OUTOMEM, the next block goes to V3D_PTB_0_BPOA/BPOS and
 * the wait goes on, as Mesa's simulator glue does (v3d_isr_core). OUTOMEM is
 * edge-signalled, and on V3D 4.x FLDONE does not come while the binner is
 * out of memory: it needs the block to finish. -ENOMEM when the pool is dry
 * (or `ov` is NULL); otherwise as v3d_wait.
 */
int v3d_wait_bin(const struct v3d_regs *r, uint32_t timeout_us, struct v3d_stall *stall,
		 struct v3d_bin_overflow *ov);

/*
 * The texture formatting unit (TFU): one conversion of an image in GPU
 * memory into another, polled. The registers are written in the order of
 * Mesa's v3dX(simulator_submit_tfu_ioctl): IIA, IIS, ICA, IUA, IOA, IOS,
 * COEF0 (COEF1..3 only with USECOEF, never set here), then ICFG | IOC, which
 * starts the job. Completion is the hub's TFUC status bit, polled with the
 * hub interrupts masked as M3 polls the core's; the conversion counter
 * (TFU_CS.CVTCT, which Mesa's simulator glue waits on) is
 * read before and after as a second witness. Addresses are GPU virtual.
 * The caller packs the fields (Mesa's v3dx_tfu.c) and owns the caches: the
 * source written back before, the destination invalidated after.
 */
struct v3d_tfu_job {
	uint32_t icfg, iia, iis, ica, iua, ioa, ios, coef0;
};

/* What one conversion left in the registers, for the verdict. */
struct v3d_tfu_status {
	uint32_t cs_before, cs_after;  /* V3D_TFU_CS before the start and at the end */
	uint32_t hub_int_sts;          /* the status that ended the wait (or the last read) */
	uint32_t us;                   /* the start to the end of the wait, polled in 1 us steps */
};

/*
 * Run one TFU job and wait for it. 0 when TFUC was seen and CVTCT moved by
 * one; -EFAULT on an MMU fault (a hub MMU status bit; the fault's registers
 * stay for the caller's dump); -EIO when TFUC was seen but CVTCT did not
 * move by one; -EBUSY when the TFU was busy before the start (nothing
 * written); -ETIMEDOUT after `timeout_us`. After -EFAULT or -ETIMEDOUT the
 * TFU is not reset here (TFU_CS.TFURST is never written); the caller resets
 * the GPU as after a stage that did not end.
 */
int v3d_tfu_run(const struct v3d_regs *r, const struct v3d_tfu_job *j, uint32_t timeout_us,
		struct v3d_tfu_status *st);

#endif
