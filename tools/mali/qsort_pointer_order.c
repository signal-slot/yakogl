// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//
// Does glibc's qsort_r move anything when the comparator is Mesa 26.2.1's
// compare_u32 (bifrost_compile.c:4268-4273), which subtracts the element
// POINTERS? The constant promotion threshold reads the "sorted" array
// (src/shaderc/online/mali/fau.rs lower_constants); if qsort_r leaves it
// as it is, the threshold is the count at max_amount - 1 in the hash
// table's slot order. Prints how many of 3200 arrays (1 to 64 counts, 50
// each) changed order.
//
//   cc -O2 -o /tmp/qs tools/mali/qsort_pointer_order.c && /tmp/qs
//
// Measured 2026-10-19 on the host pan_shc runs on (glibc 2.44):
// "arrays whose order changed: 0 of 3200".
#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
/* Mesa 26.2.1 bifrost_compile.c compare_u32: the element POINTERS */
static int compare_u32(const void *a, const void *b, void *_)
{
    const uint32_t va = (uintptr_t)a;
    const uint32_t vb = (uintptr_t)b;
    return va - vb;
}
int main(void)
{
    int moved = 0;
    for (int n = 1; n <= 64; n++) {
        for (int seed = 0; seed < 50; seed++) {
            uint32_t *a = malloc(n * sizeof *a), *b = malloc(n * sizeof *b);
            srand(n * 1000 + seed);
            for (int i = 0; i < n; i++) a[i] = b[i] = rand() % 5 + 1;
            qsort_r(a, n, sizeof *a, compare_u32, NULL);
            for (int i = 0; i < n; i++) if (a[i] != b[i]) { moved++; break; }
            free(a); free(b);
        }
    }
    printf("arrays whose order changed: %d of %d\n", moved, 64 * 50);
    return 0;
}
