/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * baremetal_config_check.c - host check of the bare-metal OSAL's board
 * registration (src/osal/baremetal.c, include/gles_osal_baremetal.h): init
 * fails closed. baremetal.c is linked in unchanged; the "register block"
 * and the "carveout" are host arrays, so an access the OSAL should not make
 * (an address from a missing board, register offset 0x10 at address 0x10)
 * ends the program with a fault instead of passing.
 *
 *   cc -std=c11 -O1 -Wall -Wextra -Werror -Iinclude -Isrc/osal \
 *      -DYAKOGL_OSAL_BAREMETAL tools/osal/baremetal_config_check.c \
 *      src/osal/baremetal.c -o build/osal-check/baremetal_config_check \
 *      && build/osal-check/baremetal_config_check
 *
 * The shapes:
 * 1. No board registered: init returns -EINVAL with one log line naming
 *    the cause, the boot hook is not called, and every other op refuses
 *    without an access (gpu_alloc, map_phys, mmu_map, gpu_write, reg_read,
 *    reg_write, the cache ops). osal_bm_board_config(NULL) is -EINVAL.
 * 2. Each required field left 0 / NULL (8 of them), a carveout base that is
 *    not 4 KiB aligned, render_pool_off at carveout_size and a carveout that
 *    wraps: init returns -EINVAL, one log line names the field, no boot.
 * 3. A boot hook that fails: init returns its code and nothing is usable.
 * 4. A complete board: init boots with reg_base; registering again is
 *    -EBUSY; reg_read / reg_write reach the register block; gpu_alloc hands
 *    out carveout_cpu + render_pool_off and maps carveout_phys +
 *    render_pool_off; a size that would round past SIZE_MAX, an allocation
 *    past the carveout and an mmu_map of SIZE_MAX bytes are refused;
 *    gpu_write / gpu_read go through the cache hooks. After shutdown the
 *    board can be registered again.
 */
#define _POSIX_C_SOURCE 200809L
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

#include "gles_osal_baremetal.h"
#include "osal_internal.h"

/* 15 (no board) + 11 x 3 (bad boards) + 2 (failed boot) + 24 (good board) */
#define PLANNED 74

static int checks, failures;

static void check(bool ok, const char *what)
{
	checks++;
	if (!ok) {
		failures++;
		printf("baremetal_config_check: FAIL %s\n", what);
	}
}

/* baremetal.c's osal_baremetal_register() calls this; nothing here uses it. */
void osal_set_ops(const struct osal_ops *ops)
{
	(void)ops;
}

/* ---- the fake board ---------------------------------------------------- */
static uint32_t regs[64];
static _Alignas(4096) uint8_t carveout[64 * 1024];
#define CARVEOUT_PHYS 0x80000000ull
#define POOL_OFF      0x2000u

static int boot_calls, boot_rc;
static uintptr_t boot_regs;
static int map_calls;
static uint64_t map_va, map_phys;
static size_t map_size;
static int clean_calls, invd_calls;

static int fake_boot(uintptr_t r)
{
	boot_calls++;
	boot_regs = r;
	return boot_rc;
}
static void fake_mmu_map(uint64_t va, uint64_t phys, size_t size, uint64_t l0flags)
{
	(void)l0flags;
	map_calls++;
	map_va = va;
	map_phys = phys;
	map_size = size;
}
static void fake_clean(void *p, size_t n)
{
	(void)p;
	(void)n;
	clean_calls++;
}
static void fake_invd(void *p, size_t n)
{
	(void)p;
	(void)n;
	invd_calls++;
}

static struct osal_bm_board good_board(void)
{
	struct osal_bm_board c = {
		.reg_base = (uintptr_t)regs,
		.carveout_phys = CARVEOUT_PHYS,
		.carveout_cpu = carveout,
		.carveout_size = sizeof(carveout),
		.render_pool_off = POOL_OFF,
		.boot = fake_boot,
		.mmu_map = fake_mmu_map,
		.cache_clean = fake_clean,
		.cache_invalidate = fake_invd,
	};
	return c;
}

/* ---- stdout capture: the OSAL logs with vprintf ------------------------- */
static char logbuf[4096];
static FILE *cap;
static int saved_fd = -1;

static void cap_begin(void)
{
	fflush(stdout);
	cap = tmpfile();
	saved_fd = dup(1);
	if (!cap || saved_fd < 0 || dup2(fileno(cap), 1) < 0) {
		fprintf(stderr, "baremetal_config_check: cannot capture stdout\n");
		_exit(2);
	}
}

static const char *cap_end(void)
{
	size_t n;

	fflush(stdout);
	dup2(saved_fd, 1);
	close(saved_fd);
	rewind(cap);
	n = fread(logbuf, 1, sizeof(logbuf) - 1, cap);
	logbuf[n] = '\0';
	fclose(cap);
	return logbuf;
}

static int lines(const char *s)
{
	int k = 0;

	for (; *s; s++)
		k += *s == '\n';
	return k;
}

/* init with `c` registered: refused with -EINVAL, exactly one log line that
 * starts with the refusal prefix and contains `why`, and no boot. 3 checks. */
static void expect_refused(const struct osal_bm_board *c, const char *why, const char *what)
{
	char msg[160];
	const char *log;
	int rc;

	osal_bm_board_config(c);
	boot_calls = 0;
	cap_begin();
	rc = osal_baremetal_ops.init(NULL);
	log = cap_end();
	snprintf(msg, sizeof(msg), "%s: init returned %d, not -22", what, rc);
	check(rc == -22, msg);
	snprintf(msg, sizeof(msg), "%s: log is not one refusal line naming \"%s\": %s", what, why, log);
	check(lines(log) == 1 && strncmp(log, "yakogl baremetal: init refused: ", 32) == 0 &&
		      strstr(log, why) != NULL,
	      msg);
	snprintf(msg, sizeof(msg), "%s: boot hook called", what);
	check(boot_calls == 0, msg);
}

int main(void)
{
	const struct osal_ops *ops = &osal_baremetal_ops;
	struct osal_gpu_mem mem;
	struct osal_bm_board c;
	uint8_t byte = 0x5a, back = 0;
	const char *log;
	uint32_t v;
	int rc;

	/* 1. nothing registered (15 checks) */
	boot_calls = 0;
	cap_begin();
	rc = ops->init(NULL);
	log = cap_end();
	check(rc == -22, "no board: init not -EINVAL");
	check(lines(log) == 1 && strstr(log, "yakogl baremetal: init refused: no board registered"),
	      "no board: not one named refusal line");
	check(boot_calls == 0, "no board: boot hook called");
	check(ops->gpu_alloc(0x1000000, 4096, 0, &mem) == -22, "no board: gpu_alloc not refused");
	check(ops->map_phys(0x1000000, 0x90000000ull, 4096, 0, &mem) == -22,
	      "no board: map_phys not refused");
	check(ops->mmu_map(0x1000000, 0x90000000ull, 4096, 0) == -22, "no board: mmu_map not refused");
	mem = (struct osal_gpu_mem){ .cpu = carveout, .gpu = 0x1000000, .size = 4096 };
	check(ops->gpu_write(&mem, 0, &byte, 1) == -22, "no board: gpu_write not refused");
	check(ops->gpu_read(&mem, 0, &back, 1) == -22, "no board: gpu_read not refused");
	cap_begin();
	v = ops->reg_read(0x10); /* address 0x10 if it were not refused */
	ops->reg_write(0x10, 1);
	log = cap_end();
	check(v == 0, "no board: reg_read not 0");
	check(lines(log) == 2 && strstr(log, "reg_read(0x10) before init refused") &&
		      strstr(log, "reg_write(0x10) before init refused"),
	      "no board: reg_read / reg_write did not log their refusal");
	ops->cache_flush(carveout, 64);
	ops->cache_invd(carveout, 64);
	check(clean_calls == 0 && invd_calls == 0 && map_calls == 0, "no board: a hook was called");
	check(ops->submit(NULL, 0) == -22, "no board: submit not refused");
	check(osal_bm_board_config(NULL) == -22, "config(NULL) not -EINVAL");
	cap_begin();
	rc = ops->init(NULL);
	cap_end();
	check(rc == -22 && boot_calls == 0, "config(NULL) made init succeed");
	check(regs[4] == 0, "no board: register 0x10 written");

	/* 2. each required field missing, and the bad shapes (11 x 3 checks) */
	c = good_board(); c.reg_base = 0;
	expect_refused(&c, "reg_base is 0", "reg_base 0");
	c = good_board(); c.carveout_phys = 0;
	expect_refused(&c, "carveout_phys is 0", "carveout_phys 0");
	c = good_board(); c.carveout_cpu = NULL;
	expect_refused(&c, "carveout_cpu is NULL", "carveout_cpu NULL");
	c = good_board(); c.carveout_size = 0;
	expect_refused(&c, "carveout_size is 0", "carveout_size 0");
	c = good_board(); c.boot = NULL;
	expect_refused(&c, "boot hook is NULL", "boot NULL");
	c = good_board(); c.mmu_map = NULL;
	expect_refused(&c, "mmu_map hook is NULL", "mmu_map NULL");
	c = good_board(); c.cache_clean = NULL;
	expect_refused(&c, "cache_clean hook is NULL", "cache_clean NULL");
	c = good_board(); c.cache_invalidate = NULL;
	expect_refused(&c, "cache_invalidate hook is NULL", "cache_invalidate NULL");
	c = good_board(); c.carveout_phys = CARVEOUT_PHYS + 0x800;
	expect_refused(&c, "not 4 KiB aligned", "unaligned carveout_phys");
	c = good_board(); c.render_pool_off = c.carveout_size;
	expect_refused(&c, "render_pool_off is not below carveout_size", "render_pool_off == size");
	c = good_board(); c.carveout_phys = UINT64_MAX & ~0xfffull;
	expect_refused(&c, "wraps the address space", "carveout wrapping");

	/* 3. a boot hook that fails (2 checks) */
	c = good_board();
	osal_bm_board_config(&c);
	boot_rc = -5;
	check(ops->init(NULL) == -5, "failed boot: init did not return the hook's code");
	check(ops->gpu_alloc(0x1000000, 4096, 0, &mem) == -22, "failed boot: gpu_alloc not refused");
	boot_rc = 0;

	/* 4. a complete board (21 checks) */
	boot_calls = 0;
	check(ops->init(NULL) == 0, "good board: init failed");
	check(boot_calls == 1 && boot_regs == (uintptr_t)regs, "good board: boot not called with reg_base");
	check(osal_bm_board_config(&c) == -16, "good board: config while initialised not -EBUSY");
	regs[2] = 0x1234abcd;
	check(ops->reg_read(8) == 0x1234abcd, "good board: reg_read(8) did not read the block");
	ops->reg_write(12, 0xcafe);
	check(regs[3] == 0xcafe, "good board: reg_write(12) did not write the block");

	memset(carveout, 0xee, sizeof(carveout));
	check(ops->gpu_alloc(0x1000000, 100, 0, &mem) == 0, "good board: gpu_alloc failed");
	check(mem.cpu == carveout + POOL_OFF, "good board: cpu is not carveout_cpu + render_pool_off");
	check(mem.phys == CARVEOUT_PHYS + POOL_OFF,
	      "good board: phys is not carveout_phys + render_pool_off");
	check(mem.size == 4096 && mem.gpu == 0x1000000, "good board: size / gpu wrong");
	check(carveout[POOL_OFF] == 0 && carveout[POOL_OFF + 4095] == 0 &&
		      carveout[POOL_OFF - 1] == 0xee && carveout[POOL_OFF + 4096] == 0xee,
	      "good board: not exactly the allocation was cleared");
	check(map_calls == 1 && map_va == 0x1000000 && map_phys == CARVEOUT_PHYS + POOL_OFF &&
		      map_size == 4096,
	      "good board: mmu_map hook not called with the allocation");

	check(ops->gpu_write(&mem, 10, &byte, 1) == 0 && carveout[POOL_OFF + 10] == 0x5a,
	      "good board: gpu_write did not write");
	check(clean_calls == 1, "good board: gpu_write did not clean the cache");
	check(ops->gpu_read(&mem, 10, &back, 1) == 0 && back == 0x5a, "good board: gpu_read did not read");
	check(invd_calls == 1, "good board: gpu_read did not invalidate the cache");
	/* R-10 (REVIEW-C-TRIAGE.md): the GL core reads impl == NULL as "no
	 * storage", so a successful alloc must hand back a non-NULL impl, and
	 * free must take it and leave the descriptor empty. */
	check(mem.impl != NULL, "good board: a successful gpu_alloc left impl NULL");
	check(ops->gpu_free(&mem) == 0, "good board: gpu_free of an allocation refused");
	check(mem.impl == NULL && mem.cpu == NULL && mem.gpu == 0 && mem.size == 0,
	      "good board: gpu_free did not clear the descriptor");

	struct osal_gpu_mem big;
	check(ops->gpu_alloc(0x2000000, SIZE_MAX, 0, &big) == -12, "good board: SIZE_MAX alloc not -ENOMEM");
	check(ops->gpu_alloc(0x2000000, SIZE_MAX - 100, 0, &big) == -12,
	      "good board: alloc rounding past SIZE_MAX not -ENOMEM");
	check(ops->gpu_alloc(0x2000000, sizeof(carveout), 0, &big) == -12,
	      "good board: alloc past the carveout not -ENOMEM");
	check(ops->mmu_map(0x3000000, 0x90000000ull, SIZE_MAX, 0) == -22,
	      "good board: mmu_map of SIZE_MAX bytes not refused");
	check(map_calls == 1, "good board: a refused call reached the mmu_map hook");

	ops->shutdown();
	check(osal_bm_board_config(&c) == 0, "after shutdown: config refused");

	printf("baremetal_config_check: %d checks run (planned %d), %d failed\n", checks, PLANNED,
	       failures);
	return failures || checks != PLANNED;
}
