/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * Host harness: run.sh compiles the board lib's ramlog.c unchanged with it.
 * Known text goes through the writer's printk hook into a host buffer that
 * stands for the DDR ring; the buffer is dumped to a file.
 *
 *   harness CASE OUT.bin     CASE: short wrap empty garbage torn
 */
#include <setjmp.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <zephyr/sys/printk-hooks.h>

#ifndef CONFIG_YAKOGL_RAMLOG_SIZE
#error CONFIG_YAKOGL_RAMLOG_SIZE
#endif
extern int (*yk_sys_init)(void);

static uint8_t ddr[CONFIG_YAKOGL_RAMLOG_SIZE];
static printk_hook_fn_t hook, prev_hook;
static int armed;
static jmp_buf torn;
static unsigned chained;

void device_map(uintptr_t *va, uintptr_t phys, size_t size, uint32_t flags)
{
	(void)phys;
	(void)flags;
	if (size != sizeof(ddr)) {
		fprintf(stderr, "map size %zu\n", size);
		exit(1);
	}
	*va = (uintptr_t)ddr;
}

printk_hook_fn_t __printk_get_hook(void) { return prev_hook; }
void __printk_hook_install(printk_hook_fn_t fn) { hook = fn; }

void barrier_dmem_fence_full(void)
{
	/* the writer stores the data byte, then fences, then head */
	if (armed) {
		armed = 0;
		longjmp(torn, 1);
	}
}

static int chain(int c)
{
	chained++;
	return c;
}

static void put(const char *s)
{
	for (; *s; s++) {
		hook((unsigned char)*s);
	}
}

int main(int argc, char **argv)
{
	if (argc != 3) {
		return 2;
	}
	const char *c = argv[1];

	if (!strcmp(c, "empty")) {
		/* never initialised: zeroed DDR */
	} else if (!strcmp(c, "garbage")) {
		unsigned x = 12345;

		for (size_t i = 0; i < sizeof(ddr); i++) {
			x = x * 1103515245u + 12345u;
			ddr[i] = x >> 16;
		}
		ddr[0] ^= 0xff; /* make sure the magic cannot match */
	} else {
		prev_hook = chain;
		yk_sys_init();
		if (!hook) {
			return 1;
		}
		if (!strcmp(c, "short")) {
			put("MP2:A EL2\nMP2:B up\n");
		} else if (!strcmp(c, "wrap")) {
			char b[32];

			for (int i = 0; i < 40; i++) {
				snprintf(b, sizeof b, "wrap line %03d\n", i);
				put(b);
			}
		} else if (!strcmp(c, "torn")) {
			put("MP2:A EL2\nMP2:B up\nMP2:C torn");
			if (!setjmp(torn)) {
				armed = 1;
				hook('X'); /* data stored, head not updated */
			}
		} else {
			return 2;
		}
		if (!chained) {
			fprintf(stderr, "writer did not chain\n");
			return 1;
		}
	}
	FILE *f = fopen(argv[2], "wb");

	if (!f || fwrite(ddr, 1, sizeof(ddr), f) != sizeof(ddr) || fclose(f)) {
		return 1;
	}
	return 0;
}
