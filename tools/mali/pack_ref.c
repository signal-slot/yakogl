/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * pack_ref: Mesa's own packers for the Mali-G610's descriptors, as the
 * reference src/mali/genxml/v10.rs is compared with.
 *
 * It packs each case of v10_cases.inc (written by
 * `tools/mali/gen_pack_rs.py c-cases`, the same values as the Rust side's
 * src/mali/genxml/v10_cases.rs) with Mesa's generated v10_pack.h
 * (third_party/mesa-26.2.1/src/panfrost/genxml/gen_pack.py on v10.xml)
 * and prints one line per case:
 *
 *     case <id> <bytes in hex>
 *
 * and last `cases <n> (planned <PACK_REF_CASES>)`. Built without NDEBUG,
 * so Mesa's own asserts (a minus(1) of 0, a shr with low bits set, a value
 * wider than its field) stop the run on a case they would refuse.
 * src/mali/genxml/mesa_tests.rs builds and runs it; nothing here is
 * product code.
 */

#include <stdio.h>

#include "v10_pack.h"

static int cases_run;

static void print_case(const char *id, const void *packed, size_t n)
{
   const unsigned char *b = packed;

   printf("case %s ", id);
   for (size_t i = 0; i < n; i++)
      printf("%02x", b[i]);
   printf("\n");
   cases_run++;
}

#include "v10_cases.inc"

int main(void)
{
   run_cases();
   printf("cases %d (planned %d)\n", cases_run, PACK_REF_CASES);
   return cases_run == PACK_REF_CASES ? 0 : 1;
}
