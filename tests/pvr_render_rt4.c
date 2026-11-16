/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * The PowerVR render layer at four colour targets (RC14,
 * docs/es3-cts-failures.md); built and run by tests/pvr_render_rt4.rs.
 *
 * driver/src/pvr_render.c builds the static end-of-tile PDS program the
 * firmware runs for every render (PVR_RENDER_EOT_PDS_CODE_OFF) and the
 * CR_EVENT_PIXEL_PDS_INFO every render's end-of-tile program runs with from
 * pvr_render_eot_pds_pairs(pvr_render_eot_four_rt): static_areas_init and
 * pvr_render_static_get. This file makes the same calls (pvr_render.c
 * itself includes the Zephyr and DDK headers and is not built on the host)
 * and checks:
 *   - off (the default): the three-pair program and info word, the bytes
 *     every 1, 2 and 3 target render ran with at PVR_RENDER_MAX_RT 3;
 *   - on: a four-pair program whose fourth DOUTW moves the fourth PBE state
 *     (shared registers 6, 7), the one the four-target end-of-tile program
 *     (eot4, driver/src/pvr_usc_eot_blob.h) reads;
 *   - a render's PDS DATA segment for that program: the fourth target's emit
 *     words in the fourth pair's constants, the four-target program's DOUTU.
 * The words come from Mesa 26.2.1's pvr_pds_generate_pixel_event
 * (third_party/mesa-26.2.1/src/imagination/vulkan/pds/pvr_pds.c:535-607):
 * DATA = DOUTU src0 (2 dwords), 2 x pairs emit words, pairs DOUTW control
 * words (destination 2i, the last with LAST_EN); CODE = one DOUTW per pair
 * then DOUTU with END.
 *
 * Prints "pvr_render_rt4: <n> checks run (planned <n>), 0 failed"; with
 * -DDUMP it prints the words instead (to re-pin them, with a reason).
 */
#include <stdio.h>
#include <stdint.h>
#include <string.h>

#include "pvr_render.h"
#include "pvr_render_regs.h"
#include "pvr_pds_pixel_event_gen.h"
#include "pvr_usc_eot_blob.h"

#define ARRAY_LEN(a) (sizeof(a) / sizeof((a)[0]))

static unsigned checks, failed;

static void expect(const char *what, uint64_t got, uint64_t want)
{
   checks++;
   if (got != want) {
      failed++;
      printf("FAIL %s: 0x%llx, want 0x%llx\n", what, (unsigned long long)got,
             (unsigned long long)want);
   }
}

static void expect_words(const char *what, const uint32_t *got, uint32_t got_n,
                         const uint32_t *want, uint32_t want_n)
{
   checks++;
   if (got_n != want_n || memcmp(got, want, want_n * 4u) != 0) {
      failed++;
      printf("FAIL %s: %u words, want %u:", what, got_n, want_n);
      for (uint32_t i = 0; i < got_n; i++)
         printf(" %08x", got[i]);
      printf("\n");
   }
}

/* The static program and info word pvr_render.c builds for the switch's
 * value (static_areas_init: placeholder emit words, the one-target DOUTU). */
struct eot_static {
   uint32_t pairs, info, code_dw, code[PVR_RENDER_MAX_RT + 2];
};

static struct eot_static eot_static(bool four_rt, uint64_t doutu)
{
   uint32_t emit[2 * PVR_RENDER_MAX_RT] = { 0 };
   uint32_t d[3 * PVR_RENDER_MAX_RT + 2], dsz = 0;
   struct eot_static s;

   memset(&s, 0, sizeof(s));
   memset(d, 0, sizeof(d));
   s.pairs = pvr_render_eot_pds_pairs(four_rt);
   s.info = pvr_cr_event_pixel_pds_info(s.pairs);
   if (pvr_pds_generate_pixel_event_tq(doutu, emit, s.pairs, 0, d, s.code, &dsz, &s.code_dw))
      s.code_dw = 0;
   return s;
}

/* A render's PDS DATA segment for `pairs`, the first `targets` pairs set. */
static uint32_t eot_data(uint32_t pairs, uint32_t targets, uint64_t doutu, uint32_t *d)
{
   uint32_t emit[2 * PVR_RENDER_MAX_RT] = { 0 }, c[PVR_RENDER_MAX_RT + 2], dsz = 0, csz = 0;

   for (uint32_t i = 0; i < 2 * targets; i++)
      emit[i] = 0xE0000000u | (i / 2) << 8 | (i & 1);
   memset(d, 0, (3 * PVR_RENDER_MAX_RT + 2) * 4);
   if (pvr_pds_generate_pixel_event_tq(doutu, emit, pairs, 0, d, c, &dsz, &csz))
      return 0;
   return dsz;
}

#ifdef DUMP
static void dump(const char *what, const uint32_t *w, uint32_t n)
{
   printf("%s (%u):", what, n);
   for (uint32_t i = 0; i < n; i++)
      printf(" 0x%08x,", w[i]);
   printf("\n");
}
#endif

int main(void)
{
   enum { PLANNED = 27 };
   /* The static program's DOUTU is the one-target program's; its value is
    * not in the CODE segment, so any value gives the same code. */
   const uint64_t doutu1 = pvr_pds_pe_build_doutu_src0(0x1000, PVR_USC_EOT_TEMPS,
                                                        PVR_PDS_PE_SAMPLE_RATE_INSTANCE, 0);
   const uint64_t doutu4 = pvr_pds_pe_build_doutu_src0(0x4000, PVR_USC_EOT_TEMPS,
                                                        PVR_PDS_PE_SAMPLE_RATE_INSTANCE, 0);
   /* The committed words. code3 and info3 are the static program and info
    * word of every render before RC14 (pvr_render.c generated them for
    * PVR_RENDER_MAX_RT 3 pairs; info3 is also render_regs_check's
    * "CR_EVENT_PIXEL_PDS_INFO 3 RT"). */
   static const uint32_t code3[] = { 0xf0080101u, 0xf0090201u, 0xf00a0301u, 0xf4000002u };
   static const uint32_t code4[] = { 0xf00a0101u, 0xf00b0201u, 0xf00c0301u, 0xf00d0401u,
                                     0xf4000002u };
   static const uint32_t info3 = 0x00000203u, info4 = 0x00000204u;
   static const uint32_t data3[] = { 0x00004000u, 0x00000000u, 0xe0000000u, 0xe0000001u,
                                     0xe0000100u, 0xe0000101u, 0xe0000200u, 0xe0000201u,
                                     0x10000002u, 0x10004002u, 0x90008002u };
   static const uint32_t data4[] = { 0x00004000u, 0x00000000u, 0xe0000000u, 0xe0000001u,
                                     0xe0000100u, 0xe0000101u, 0xe0000200u, 0xe0000201u,
                                     0xe0000300u, 0xe0000301u, 0x10000002u, 0x10004002u,
                                     0x10008002u, 0x9000c002u };
   uint32_t d3[3 * PVR_RENDER_MAX_RT + 2], d4[3 * PVR_RENDER_MAX_RT + 2], n3, n4;
   struct eot_static off = eot_static(false, doutu1), on = eot_static(true, doutu1);

   n3 = eot_data(3, 3, doutu4, d3);
   n4 = eot_data(4, 4, doutu4, d4);
#ifdef DUMP
   printf("info3 0x%08x info4 0x%08x\n", off.info, on.info);
   dump("code3", off.code, off.code_dw);
   dump("code4", on.code, on.code_dw);
   dump("data3", d3, n3);
   dump("data4", d4, n4);
   return 0;
#endif

   /* The counts. */
   expect("PVR_RENDER_MAX_RT", PVR_RENDER_MAX_RT, 4);
   expect("PVR_RENDER_EOT_MAX_RT", PVR_RENDER_EOT_MAX_RT, PVR_RENDER_MAX_RT);
   expect("PVR_RENDER_EOT_PDS_PAIRS", PVR_RENDER_EOT_PDS_PAIRS, 3);
   expect("an end-of-tile program per count up to four", PVR_USC_EOT_MAX_RT, PVR_RENDER_MAX_RT);
   expect("the four-target program (eot4) is uploaded", pvr_usc_eot_n_words[4] != NULL, 1);
   expect("eot4: four 4-dword emits", pvr_usc_eot_n_dwords[4], 16);

   /* Off: today's bytes, for every 1, 2 and 3 target render. */
   expect("pairs off", off.pairs, 3);
   expect("info off (every count 1..4 runs with it)", off.info, info3);
   expect_words("static code off", off.code, off.code_dw, code3, ARRAY_LEN(code3));
   expect("code off: three DOUTW and a DOUTU", off.code_dw, 4);
   expect_words("a three-target render's DATA", d3, n3, data3, ARRAY_LEN(data3));
   expect("DATA off: 2 + 6 + 3 dwords", n3, 11);

   /* On: the four-pair program, for every render. */
   expect("pairs on", on.pairs, 4);
   expect("info on", on.info, info4);
   expect("info on = the info word of four pairs", on.info, pvr_cr_event_pixel_pds_info(4));
   expect("info on: const_size 4 units (14 dwords)", on.info & 0x3fu, 4);
   expect_words("static code on", on.code, on.code_dw, code4, ARRAY_LEN(code4));
   expect("code on: four DOUTW and a DOUTU", on.code_dw, 5);
   expect_words("a four-target render's DATA", d4, n4, data4, ARRAY_LEN(data4));
   expect("DATA on: 2 + 8 + 4 dwords", n4, 14);
   /* The fourth PBE state: the fourth target's emit words in the fourth
    * pair's constants, and its DOUTW control word the last one (LAST_EN,
    * bit 31), destination shared register 6: the third's + 2 in the
    * destination field at bit 13 (PVR_ROGUE_PDSINST_DOUT_FIELDS_DOUTW_SRC1_AO_SHIFT and _LAST_EN,
    * pvr_rogue_pds_defs.h:1336, :1354). */
   expect("DATA on: the four-target program's DOUTU", (uint64_t)d4[1] << 32 | d4[0], doutu4);
   expect("DATA on: fourth emit word 0", d4[2 + 6], 0xE0000300u);
   expect("DATA on: fourth emit word 1", d4[2 + 7], 0xE0000301u);
   expect("DATA on: pairs 0..2 as off", memcmp(&d4[2], &d3[2], 6 * 4) == 0, 1);
   expect("DATA on: the fourth DOUTW is the third's, two registers on",
          d4[10 + 3] & ~0x80000000u, d4[10 + 2] + (2u << 13));
   expect("DATA on: the fourth DOUTW is the last", d4[10 + 3] >> 31, 1);
   expect("CODE on differs from off (the program moves more)",
          memcmp(on.code, off.code, 4 * 4) != 0, 1);

   printf("pvr_render_rt4: %u checks run (planned %u), %u failed\n", checks, (unsigned)PLANNED,
          failed);
   return failed || checks != PLANNED;
}
