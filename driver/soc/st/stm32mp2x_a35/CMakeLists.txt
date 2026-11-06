# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# STM32MP257F Cortex-A35: Zephyr's generic arm64 link script, Zephyr's
# default MMU table (arch/arm64/core/mmu_default_config.c: devicetree
# regions only) and the polled console USART.

zephyr_include_directories(.)
zephyr_sources_ifdef(CONFIG_UART_STM32MP2_A35_POLLED uart_stm32mp2_a35_polled.c)

set(SOC_LINKER_SCRIPT ${ZEPHYR_BASE}/include/zephyr/arch/arm64/scripts/linker.ld CACHE INTERNAL "")
