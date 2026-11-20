/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * zephyr_mali_board.h - the board's half of the Zephyr Mali OSAL
 * (src/osal/zephyr_mali.c). The OSAL owns the GPU's memory and the ops; what
 * depends on the SoC is here, one implementation per board, linked into
 * every CONFIG_YAKOGL_GPU_MALI image of that board:
 *
 *   NanoPC-T6 (RK3588)      driver/zephyr/lib/board/nanopc_t6/nanopc_t6_mali.c
 *   Verdin iMX95 (i.MX 95)  driver/zephyr/lib/board/verdin_imx95/verdin_imx95_mali.c
 *
 * The kernel side the ops forward to is board::nanopc_t6_osal through the
 * yd_t6_osal_* entries of yakogl_drivers.h, the same on every board. Only
 * the bring-up is the SoC's: the T6's board_up calls yd_t6_osal_up (the
 * RK3588's CRU, PMU, PMIC and QoS), the Verdin iMX95's yd_imx95_osal_up
 * (SCMI to the System Manager); from GPU_ID on both run the same code.
 */
#ifndef YAKOGL_ZEPHYR_MALI_BOARD_H
#define YAKOGL_ZEPHYR_MALI_BOARD_H

#include <stddef.h>
#include <stdint.h>

#include "osal_internal.h" /* struct osal_zephyr_mali_init */

struct yd_t6_osal_mem;   /* yakogl_drivers.h */
struct yd_t6_gpu_up;
struct yd_t6_submit_up;

/* The SoC's part of the OSAL's init, called once by z_init after the GPU's
 * memory is zeroed and made uncached: map the SoC's blocks, power the GPU on,
 * boot the CSF firmware `fw` and start the group over `mem`. `clean`,
 * `delay_us` and `on_handover` are the OSAL's (the cache clean, the delay
 * that feeds the board's watchdog, the hand-over hook); `z1` and `o` are
 * filled with the records of where a refusal stopped. 0 when the group is
 * up; a negative errno otherwise. A board with no bring-up yet returns
 * -ENOTSUP after one console line that names what is missing. */
int osal_zephyr_mali_board_up(const struct osal_zephyr_mali_init *fw,
			      const struct yd_t6_osal_mem *mem,
			      int (*clean)(uintptr_t addr, size_t len),
			      void (*delay_us)(uint32_t us),
			      void (*on_handover)(const struct yd_t6_gpu_up *h),
			      struct yd_t6_gpu_up *z1, struct yd_t6_submit_up *o);

/* The board's window memory: the carve-out's last
 * CONFIG_YAKOGL_MALI_WINDOW_BYTES, which the OSAL's allocator never hands
 * out and the GPU reaches through map_phys. Its physical address, the CPU's
 * view (Normal non-cacheable, zeroed once: the OSAL's memory is made so at
 * the first of this call and init, so neither the CPU nor the GPU needs a
 * cache operation for it) and its size. Callable before eglInitialize
 * (board_window()); main only. -ENOTSUP when the option is 0, or the
 * error of making the memory uncached. */
int osal_zephyr_mali_window_mem(uintptr_t *phys, void **cpu, size_t *bytes);

/* "Still making progress": the board's watchdog feed, called by the OSAL in
 * every delay (its waits poll) and around the bring-up. Any context. */
void osal_zephyr_mali_board_feed(void);

#endif /* YAKOGL_ZEPHYR_MALI_BOARD_H */
