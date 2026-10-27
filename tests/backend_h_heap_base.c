/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* backend.h's yakogl_heap_base() against the backend contract of
 * src/backend.rs: `heap_base` is optional, and a table without it has no
 * heap layout, which backend::heap_base() reads as 0. The C inline called
 * the pointer unchecked, so on the Mali (src/mali/backend.rs leaves it
 * NULL) gl_test's osal_gpu_alloc(yakogl_heap_base(GLES_HEAP_DATA), ...)
 * jumped to address 0 (cases 18, 21, 171, 172, 173 on the NanoPC-T6). The
 * inline is reachable from C only, so the pin is a C program. tools/check.sh
 * runs it ("run     backend.h yakogl_heap_base"):
 *   mkdir -p build && cc -std=gnu11 -Wall -Wextra -Werror -Iinclude -Isrc -Isrc/osal \
 *     tests/backend_h_heap_base.c -o build/backend_h_heap_base && build/backend_h_heap_base
 */
#include <stdio.h>
#include "backend.h"

static struct yakogl_backend table;
const struct yakogl_backend *g_backend = &table;

#define PLANNED 3

static unsigned run, failed;

static void expect(const char *what, uint64_t got, uint64_t want)
{
	printf("%-52s got 0x%llx want 0x%llx %s\n", what, (unsigned long long)got, (unsigned long long)want,
	       got == want ? "ok" : "FAIL");
	run++;
	failed += got != want;
}

static uint64_t heaps(enum gles_heap h)
{
	return h == GLES_HEAP_DATA ? 0x100000000ull : 0x200000000ull;
}

int main(void)
{
	/* a backend with no heap layout (the Mali's table): 0, as the Rust side */
	table.heap_base = NULL;
	expect("no heap_base: GLES_HEAP_DATA", yakogl_heap_base(GLES_HEAP_DATA), 0);
	/* a backend with one: its answer, per heap */
	table.heap_base = heaps;
	expect("heap_base: GLES_HEAP_DATA", yakogl_heap_base(GLES_HEAP_DATA), 0x100000000ull);
	expect("heap_base: GLES_HEAP_SHADER", yakogl_heap_base(GLES_HEAP_SHADER), 0x200000000ull);
	printf("backend_h_heap_base: %u checks run (planned %u), %u failed\n", run, PLANNED, failed);
	return failed || run != PLANNED;
}
