/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * pvr_mesa_pds.c -- Mesa 26.2.1's PDS program generators
 * (third_party/mesa-26.2.1/src/imagination/vulkan/pds/pvr_pds.c), compiled
 * as they are, in the environment driver/compat/pvr_mesa.h sets up, plus the
 * two things they need from the rest of Mesa: the device information and
 * mesa_log().
 */
#include "pvr_mesa.h"

/* Mesa builds this file without -Wextra (an unused parameter in
 * pvr_pds_write_dma_address), and gcc at -O1 and above cannot see that
 * pvr_pds_compute_shader() sets usc_control_constant64 on every path that
 * reads it; neither is a code path this driver depends on. */
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wunused-parameter"
#if defined(__GNUC__) && !defined(__clang__)
#pragma GCC diagnostic ignored "-Wmaybe-uninitialized"
#endif
#include "vulkan/pds/pvr_pds.c"
#pragma GCC diagnostic pop

#include "common/device_info/bxs-4-64.h"

/* util/log.c's entry point. The generators log only on a request they
 * cannot encode, and every caller here checks its inputs first, so nothing
 * reaches it; it has nowhere to print in a driver without a console of its
 * own. */
void mesa_log(enum mesa_log_level level, const char *tag, const char *format, ...)
{
   (void)level;
   (void)tag;
   (void)format;
}

const struct pvr_device_info *pvr_mesa_dev_info(bool slc_mcu_cache_controls)
{
   static struct pvr_device_info info[2];
   static bool ready;

   if (!ready) {
      info[0] = pvr_device_info_36_53_104_796;
      info[1] = pvr_device_info_36_53_104_796;
      info[1].features.has_slc_mcu_cache_controls = true;
      ready = true;
   }
   return &info[slc_mcu_cache_controls ? 1 : 0];
}
