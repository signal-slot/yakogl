/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * The DOUTI words of the REAL coefficient generator
 * (driver/src/pvr_pds_render_gen.c) for one flat and one linear varying;
 * built and run by tests/es3_pvr_flatshade.rs. Prints
 * "flat_vertex2=1 flat_vertex0=0 gouraud=1" when the flat varying is
 * iterated from the last vertex (GL's provoking vertex) and the linear one
 * is still GOURAUD.
 */
#include <stdio.h>
#include <stdint.h>

#include "pvr_mesa.h"
#include "pvr_pds_render_gen.h"

static uint32_t douti(enum ROGUE_PDSINST_DOUTI_SHADEMODEL m, enum ROGUE_PDSINST_DOUTI_SIZE size,
                      bool persp, uint32_t off)
{
   const struct ROGUE_PDSINST_DOUT_FIELDS_DOUTI_SRC v = {
      .shademodel = m,
      .size = size,
      .perspective = persp,
      .f32_offset = off,
      .f16_offset = off,
   };
   uint32_t w;

   ROGUE_PDSINST_DOUT_FIELDS_DOUTI_SRC_pack(&w, &v);
   return w;
}

static int has(const uint32_t *d, uint32_t n, uint32_t w)
{
   for (uint32_t i = 0; i < n; i++)
      if (d[i] == w)
         return 1;
   return 0;
}

int main(void)
{
   /* No Z, W or point coordinate: the first varying (vtxout 4) is at 0. */
   const struct pvr_pds_coeff_varying v[2] = {
      { 3, PVR_PDS_COEFF_FLAT, 4 },
      { 2, PVR_PDS_COEFF_NPC, 7 },
   };
   uint32_t data[128] = { 0 }, code[128] = { 0 };
   struct pvr_pds_coeff_info info;
   uint32_t n;

   if (pvr_pds_render_coeff_program(0, 0, 0, v, 2, data, code, &info)) {
      puts("refused");
      return 1;
   }
   n = info.data_size_dw;
   printf("flat_vertex2=%d flat_vertex0=%d gouraud=%d\n",
          has(data, n, douti(ROGUE_PDSINST_DOUTI_SHADEMODEL_FLAT_VERTEX2, ROGUE_PDSINST_DOUTI_SIZE_3D, false, 0)),
          has(data, n, douti(ROGUE_PDSINST_DOUTI_SHADEMODEL_FLAT_VERTEX0, ROGUE_PDSINST_DOUTI_SIZE_3D, false, 0)),
          has(data, n, douti(ROGUE_PDSINST_DOUTI_SHADEMODEL_GOURUAD, ROGUE_PDSINST_DOUTI_SIZE_2D, false, 6)));
   return 0;
}
