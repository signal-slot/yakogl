# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Reuses the TI K3 common code from the Zephyr tree.
set(K3_COMMON ${ZEPHYR_BASE}/soc/ti/k3/common)

zephyr_include_directories(${K3_COMMON}/peripherals ${CMAKE_CURRENT_LIST_DIR})
zephyr_sources(${K3_COMMON}/peripherals/ctrl_partitions.c)

if(CONFIG_SOC_SERIES_AM62PX_A53)
  # soc_a53.c replaces ${K3_COMMON}/cortex_a/soc.c: same partition unlock,
  # plus the GIC CPU-interface reset needed after a kexec from Linux.
  zephyr_sources(soc_a53.c)
  zephyr_sources_ifdef(CONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT soc_a53_wdt.c)
  set(SOC_LINKER_SCRIPT ${ZEPHYR_BASE}/include/zephyr/arch/arm64/scripts/linker.ld CACHE INTERNAL "")
endif()
# The MCU R5F cluster (soc_r5*.c, boot_r5.S, rsc_table.c, linker_r5.ld) is
# not carried here: it belongs to the platform firmware.
