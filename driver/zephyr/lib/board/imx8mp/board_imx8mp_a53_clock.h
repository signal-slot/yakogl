/* SPDX-License-Identifier: Apache-2.0 */
#ifndef BOARD_IMX8MP_A53_CLOCK_H_
#define BOARD_IMX8MP_A53_CLOCK_H_
/* board_imx8mp_a53_clock.c: 0 when the A53 runs at CONFIG_..._A53_MHZ,
 * -ETIMEDOUT when the PLL did not lock (the core is then on 800 MHz),
 * -EINVAL for a rate it does not know, 1 when it did not run. */
int board_imx8mp_a53_clock_result(void);
#endif
