/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * VideoCore IV 3D (Raspberry Pi Zero 2 W, BCM2837 / RP3A0) driver core: the
 * register sequences of item 5 of docs/notes/pre-study-rpi-zero-2w.md
 * section 8, as the Pi 4's driver/src/v3d is for the V3D 4.2. Nothing here
 * draws and nothing here has run on silicon: tools/vc4/hw_mock_test.c runs
 * every sequence against a register model on the host.
 *
 * Each sequence is read from Linux v6.18 drivers/gpu/drm/vc4/ (and the power
 * drivers named in vc4_power.c), for the hardware's behaviour only: offsets,
 * bits and the order of the writes. The code is ours; each function names
 * the Linux function it follows and says where it differs.
 *
 * Unlike driver/src/v3d (which calls Zephyr's sys_read32/sys_write32 through
 * a host shim), every register access here goes through struct vc4_io, so
 * the core has no Zephyr dependency at all and the mock records each access
 * by block. The Zephyr glue (later, with the bcm2837 SoC of item 4) is a
 * vc4_io whose functions are sys_read32/sys_write32 on the mapped block.
 *
 * Addresses handed to the V3D (control lists, overflow memory) are bus
 * addresses: vc4_bus() of the ARM physical address (vc4_regs.h).
 */
#ifndef VC4_H
#define VC4_H

#include <stdbool.h>
#include <stdint.h>

#include "vc4_regs.h"

/* One register block: 32-bit accesses at byte offsets from its base.
 * delay_us may be NULL for a block whose sequences never wait (the V3D's). */
struct vc4_io {
	uint32_t (*read32)(void *ctx, uint32_t off);
	void (*write32)(void *ctx, uint32_t off, uint32_t val);
	void (*delay_us)(void *ctx, uint32_t us);
	void *ctx;
};

/* The bus address the V3D uses for ARM physical `phys` (below
 * VC4_DMA_WINDOW). */
static inline uint32_t vc4_bus(uint32_t phys)
{
	return phys + VC4_BUS_ALIAS;
}

/* ---- identity ---------------------------------------------------------- */

struct vc4_ident {
	uint32_t ident0, ident1, ident2; /* raw */
	uint32_t rev, nslc, qups, tups, nsem, vpm_kb; /* IDENT1's fields */
};

/*
 * Read V3D_IDENT0, IDENT1 and IDENT2 (all three, in that order, so the
 * record holds them whatever the verdict) and accept only IDENT0 ==
 * V3D_EXPECTED_IDENT0 (technology version 2, "V3D"): 0, or -ENODEV. Writes
 * nothing. Linux: the check in vc4_v3d_bind (vc4_v3d.c:461-466), the fields
 * as vc4_v3d_debugfs_ident prints them (:105-116).
 */
int vc4_identify(const struct vc4_io *v3d, struct vc4_ident *id);

/*
 * The first set-up after power-on: vc4_identify, then (only when the
 * identity is right) V3D_VPMBASE = 0, the binner's overflow pointer
 * zeroed, every driver interrupt cleared and FLDONE | FRDONE enabled.
 * OUTOMEM stays disabled until there is bin memory (vc4_bin_mem_enable).
 * Linux: vc4_v3d_bind (:461-474) with vc4_v3d_runtime_resume (:384-399):
 * vc4_v3d_init_hw (:162-171), the BPOA/BPOS reset (:471-472), vc4_irq_prepare
 * (vc4_irq.c:255-258) and vc4_irq_enable (:272-275). Linux runs the resume's
 * VPMBASE and INTENA writes before the identity check; we check first so
 * that a block that is not a VC4 V3D is never written.
 * 0, or vc4_identify's -ENODEV with nothing written.
 */
int vc4_init(const struct vc4_io *v3d, struct vc4_ident *id);

/* Mask and clear every driver interrupt (vc4_irq_disable, vc4_irq.c:289-293). */
void vc4_irq_disable(const struct vc4_io *v3d);

/* ---- caches ------------------------------------------------------------ */

/* Before a bin job: the L2 cache, then the TMU0/1, uniform and instruction
 * caches of all four slices (vc4_flush_caches, vc4_gem.c:436-452). */
void vc4_flush_caches_bin(const struct vc4_io *v3d);

/* Before a render job: the L2 cache and only the two TMU caches; a render
 * list writes no instructions or uniforms (vc4_flush_texture_caches,
 * vc4_gem.c:454-465). */
void vc4_flush_caches_render(const struct vc4_io *v3d);

/* ---- control lists ----------------------------------------------------- */

/*
 * Start control-list thread `thread` (VC4_THREAD_BIN or VC4_THREAD_RENDER)
 * on [start, end) (bus addresses): CTnCA, then CTnEA, whose write starts it
 * (submit_cl, vc4_gem.c:372-382). The guide allows a CTnCA write only to a
 * stopped thread, so CTnCS is read first and a running thread (CTRUN) is
 * refused with nothing written: -EBUSY. Linux does not read CTnCS; it relies
 * on FLDONE/FRDONE having come. -EINVAL for another thread number.
 */
int vc4_submit_cl(const struct vc4_io *v3d, uint32_t thread, uint32_t start, uint32_t end);

/* ---- bin memory -------------------------------------------------------- */

/*
 * The binner keeps only 28 bits of the tile-alloc and overflow addresses and
 * takes the top 4 from the tile state buffer's, so the tile state, the tile
 * alloc and the overflow memory live in one buffer that does not cross a
 * 256 MB boundary of the bus (vc4_v3d.c:215-236, the check at :270-272). The
 * buffer is 16 MiB in 512 KiB slices (:240, :291); a slice is either a job's
 * tile state + tile alloc or the binner's overflow memory.
 */
#define VC4_BIN_SIZE      (16u * 1024u * 1024u)
#define VC4_BIN_SLOT_SIZE (512u * 1024u)
#define VC4_BIN_MAX_SLOTS 32u
#define VC4_BIN_ALIGN     4096u

/*
 * Pick the bin buffer inside a free range of ARM physical memory: the lowest
 * address >= free_phys, VC4_BIN_ALIGN aligned, such that [phys, phys + size)
 * lies inside the free range, inside the bus alias's window (VC4_DMA_WINDOW)
 * and inside one 256 MB block. A pure function. 0 and *phys, or -ERANGE when
 * no such address exists, or -EINVAL when size is 0, not a whole number of
 * slices, or more than VC4_BIN_MAX_SLOTS slices. Linux allocates and retries
 * (bin_bo_alloc, vc4_v3d.c:247-310); with a fixed memory map we choose.
 */
int vc4_bin_range_pick(uint64_t free_phys, uint64_t free_size, uint32_t size, uint32_t *phys);

struct vc4_bin_mem {
	uint32_t phys;     /* ARM physical address of the buffer */
	uint32_t size;
	uint32_t nslots;
	uint32_t used;     /* one bit per slice in use */
	uint32_t overflow; /* the slice BPOA points at now, as its bit, or 0 */
};

/* Take a buffer vc4_bin_range_pick accepted: 0, or -EINVAL when it is not
 * one (crosses 256 MB, outside the window, not whole slices). */
int vc4_bin_mem_init(struct vc4_bin_mem *m, uint32_t phys, uint32_t size);

/* The lowest free slice (vc4_v3d_get_bin_slot, vc4_v3d.c:173-213, without
 * its wait for a render to free one): its index, or -ENOMEM. */
int vc4_bin_slot_get(struct vc4_bin_mem *m);
void vc4_bin_slots_put(struct vc4_bin_mem *m, uint32_t mask);

/* The bus address of slice `slot`. */
static inline uint32_t vc4_bin_slot_bus(const struct vc4_bin_mem *m, uint32_t slot)
{
	return vc4_bus(m->phys + slot * VC4_BIN_SLOT_SIZE);
}

/* Bin memory exists now: enable OUTOMEM (bin_bo_alloc, vc4_v3d.c:299-303:
 * "potentially from an already-pending-but-masked interrupt"). */
void vc4_bin_mem_enable(const struct vc4_io *v3d);

/* ---- one frame: bin, then render --------------------------------------- */

enum vc4_job_state {
	VC4_JOB_IDLE = 0,
	VC4_JOB_BINNING,
	VC4_JOB_RENDERING,
	VC4_JOB_DONE,
	VC4_JOB_FAILED,
};

struct vc4_job {
	uint32_t ct0ca, ct0ea; /* binner list (bus); equal: nothing to bin */
	uint32_t ct1ca, ct1ea; /* render list (bus) */
	enum vc4_job_state state;
	uint32_t last_ct0ca, last_ct1ca; /* the hang check's last reading */
	uint32_t bin_slots;  /* slices to free when the job ends (its tile
			      * state + alloc, and overflow slices replaced
			      * while it ran) */
};

/*
 * Start a job (vc4_submit_next_bin_job, vc4_gem.c:472-515): the full cache
 * clear, then the binner list on thread 0. A job with an empty binner list
 * goes straight to the render list (:497-504 through vc4_move_job_to_render,
 * :541-553): the render clear and thread 1. The thread's CTnCS is read
 * before anything is written: -EBUSY with nothing written when it runs.
 * -EINVAL when the job is not IDLE. The hang check's readings start at 0, as
 * Linux's zeroed job does, so the first check after a start is progress.
 */
int vc4_job_submit(const struct vc4_io *v3d, struct vc4_job *job);

/* What one interrupt did (vc4_irq's return value, a mask). */
#define VC4_EV_OOM            VC4_BIT(0) /* OUTOMEM masked: call vc4_overflow_give */
#define VC4_EV_RENDER_STARTED VC4_BIT(1) /* FLDONE: the render list started */
#define VC4_EV_FRAME_DONE     VC4_BIT(2) /* FRDONE: the job is DONE */
#define VC4_EV_SPURIOUS       VC4_BIT(3) /* a done bit for a stage not running */
#define VC4_EV_BUSY           VC4_BIT(4) /* FLDONE, but thread 1 still runs: FAILED */

/*
 * The interrupt handler's decisions (vc4_irq, vc4_irq.c:202-242): read
 * INTCTL and write the same bits back (FLDONE and FRDONE clear; OUTOMEM
 * stays set while its cause lasts); OUTOMEM is then masked with INTDIS and
 * reported for vc4_overflow_give (Linux's work item); FLDONE moves the job
 * to the render list (the render cache clear and thread 1, as
 * vc4_irq_finish_bin_job -> vc4_move_job_to_render does, :116-136);
 * FRDONE completes it (vc4_irq_finish_render_job, :155-200). The bits are
 * handled in Linux's order: OUTOMEM, FLDONE, FRDONE.
 */
uint32_t vc4_irq(const struct vc4_io *v3d, struct vc4_job *job);

/*
 * Hand the binner a new overflow slice (vc4_overflow_mem_work,
 * vc4_irq.c:60-114): the slice it had goes to the job (freed when the job
 * ends) or, with no job, back to the pool; then BPOA = the slice's bus
 * address and BPOS = its size, which removes OUTOMEM's cause, and only then
 * OUTOMEM cleared in INTCTL and enabled again in INTENA.
 * Linux writes the whole bin buffer's size to BPOS (:107); we write the
 * slice's, so the binner can never be pointed past the slice it was given
 * (the note's section 2 says what the board must confirm).
 * 0, or -ENOMEM with OUTOMEM left masked and nothing written when no slice
 * is free (Linux logs and leaves it masked too, :77-81); the binner then
 * stalls and the hang check resets.
 */
int vc4_overflow_give(const struct vc4_io *v3d, struct vc4_bin_mem *m, struct vc4_job *job);

/* The job ended (DONE or after a reset): its slices go back to the pool. */
void vc4_job_retire(struct vc4_bin_mem *m, struct vc4_job *job);

/* ---- hang and reset ---------------------------------------------------- */

enum vc4_hang {
	VC4_HANG_IDLE = 0,  /* no stage running: stop watching */
	VC4_HANG_PROGRESS,  /* a running thread's CTnCA moved: check again later */
	VC4_HANG_HUNG,      /* neither moved since the last check: reset */
};

/*
 * The periodic hang check (vc4_hangcheck_elapsed, vc4_gem.c:326-370): read
 * CT0CA and CT1CA; the job made progress when the thread of its running
 * stage moved since the last check (the readings are kept in the job).
 * The caller calls it periodically while a job runs; Linux's period is
 * 100 ms (vc4_queue_hangcheck, vc4_gem.c:41-46).
 */
enum vc4_hang vc4_hangcheck(const struct vc4_io *v3d, struct vc4_job *job);

/* ---- power and clock --------------------------------------------------- */

/* The V3D's power and clock, as one switch (two implementations in
 * vc4_power.c). on/off return 0 or a negative errno. */
struct vc4_power {
	int (*on)(void *ctx);
	int (*off)(void *ctx);
	void *ctx;
};

enum vc4_reset_kind {
	VC4_RESET_POWER_CYCLE = 0, /* Linux's reset (vc4_reset, vc4_gem.c:289-313) */
	VC4_RESET_THREAD,          /* the guide's CTRSTA, per control thread */
};

/*
 * The reset to use: Linux's power cycle whenever there is a power switch
 * that can turn the V3D off and on, unless the caller asks for the thread
 * reset (`prefer_thread`, e.g. a board session that measures it); the
 * thread reset when there is no such switch (`pw` NULL or without off/on).
 * Linux never writes CTRSTA; no source read says it recovers a hung binner,
 * so it is the fallback, not the default.
 */
enum vc4_reset_kind vc4_reset_choose(const struct vc4_power *pw, bool prefer_thread);

/*
 * Reset after VC4_HANG_HUNG and cancel the job (its slices and the
 * overflow slice go back to `m`; the job is FAILED).
 *
 * VC4_RESET_POWER_CYCLE, as vc4_reset (vc4_gem.c:289-313) does it through
 * runtime PM: vc4_irq_disable (vc4_v3d_runtime_suspend, vc4_v3d.c:377), the
 * power off, the power on, VPMBASE = 0 and FLDONE | FRDONE enabled
 * (vc4_v3d_runtime_resume, :390-396), then vc4_irq_reset's clear and enable
 * of all three (vc4_irq.c:343-352). The power switch's error stops the
 * sequence and is returned.
 *
 * VC4_RESET_THREAD: CTRSTA written to the CTnCS of the job's running thread
 * (binner when binning, renderer when rendering; guide p. 85, Table 49:
 * "stops the control thread and resets all bits in this register"), the
 * overflow pointer zeroed (BPOA, BPOS = 0, as vc4_v3d_bind :471-472, so the
 * freed slice is not reused), then the same clear and enable as above.
 * 0, or the power switch's error; -EINVAL for a thread reset of an IDLE job.
 */
int vc4_reset(const struct vc4_io *v3d, const struct vc4_power *pw, enum vc4_reset_kind kind,
	      struct vc4_job *job, struct vc4_bin_mem *m);

/*
 * Implementation 1, the firmware (the device tree's way on the Zero 2 W:
 * the v3d node has power-domains = <&power RPI_POWER_DOMAIN_V3D> and clocks =
 * <&firmware_clocks 5>, bcm2835-rpi-common.dtsi:24-27). `tag` sends one
 * property tag and waits: on the board it is rpi_mbox_tag of
 * driver/zephyr/lib/board/rpi4/rpi_mbox.c (built for the BCM2837, mailbox at
 * VC4_MBOX_ARM_BASE), whose buffer layout this reuses rather than building
 * another.
 *   on:  SET_DOMAIN_STATE {11, 1} (raspberrypi-power.c:51-72, the firmware's
 *        domain = binding index + 1, :119-120); then, when `rate_hz` asks,
 *        the V3D clock's rate (VC4_FW_RATE_MAX: GET_MAX_CLOCK_RATE and
 *        SET_CLOCK_RATE to it, as clk-raspberrypi.c:357-400 does for the
 *        V3D's `maximize` clock, :137-140); then SET_CLOCK_STATE {5, on}
 *        (raspberrypi_fw_prepare, clk-raspberrypi.c:289-304), which is what
 *        Linux's clk_prepare_enable in vc4_v3d_runtime_resume sends.
 *   off: SET_CLOCK_STATE {5, off} (raspberrypi_fw_unprepare, :306-319), then
 *        SET_DOMAIN_STATE {11, 0}.
 * The clock tags carry three words {id, value, 0} as Linux's
 * raspberrypi_firmware_prop (clk-raspberrypi.c:184-188).
 */
typedef int (*vc4_fw_tag_fn)(uint32_t tag, uint32_t *val, uint32_t words);
#define VC4_FW_RATE_MAX 0xffffffffu
struct vc4_power_fw {
	vc4_fw_tag_fn tag;
	uint32_t rate_hz;  /* 0: leave the firmware's rate; VC4_FW_RATE_MAX; or Hz */
	uint32_t set_hz;   /* out: the rate asked for (0 when none) */
};
int vc4_power_fw_on(void *fw);
int vc4_power_fw_off(void *fw);
/* SET_CLOCK_STATE for the V3D clock alone (also the clock of the PM/ASB
 * sequence: on the Pi boards the pm node's "v3d" clock is firmware clock 5,
 * bcm2835-rpi-common.dtsi:16-21). `fw` is a struct vc4_power_fw. */
int vc4_fw_clock(void *fw, bool on);
/*
 * GET_DOMAIN_STATE {11, ~0}: 0 and *on, or -ENOTSUP when the firmware left
 * the state word as sent (it skips a tag it does not know; Linux's probe of
 * the interface, raspberrypi-power.c:137-159), or the tag's error. The day-
 * one reading of Q3 (is the island on when our image starts).
 */
int vc4_power_fw_state(void *fw, bool *on);

/*
 * Implementation 2, the PM and ASB registers (bcm2835-power.c), the fallback
 * when the firmware path does not bring the block up. `clock` switches the
 * V3D clock (vc4_fw_clock on the Pi boards).
 *   on:  the bridge's ID checked (bcm2835_power_probe, :638-639); the GRAFX
 *        island powered (bcm2835_power_power_on, :217-285, left alone when
 *        PM_POWUP is already set, :230-232), then the V3D behind its bridge
 *        (bcm2835_asb_power_on, :287-340): clock on, 1 us, clock off, reset
 *        released, clock on, bridge master then slave started.
 *   off: bcm2835_asb_power_off (:342-371): slave, then master stopped, clock
 *        off, reset asserted; then bcm2835_power_power_off (:197-215):
 *        functional and electrical isolation, power switches open.
 * Waits are polls with a 1 us delay, bounded as Linux bounds them (POWOK
 * 3 us per inrush step, MRDONE 1 us, bridge ACK 1 us). -ENODEV for a wrong
 * bridge ID (nothing written), -ETIMEDOUT with Linux's unwinding, or the
 * clock's error.
 */
struct vc4_power_pm {
	const struct vc4_io *pm, *asb;
	int (*clock)(void *ctx, bool on);
	void *clock_ctx;
};
int vc4_power_pm_on(void *pm);
int vc4_power_pm_off(void *pm);

#endif
