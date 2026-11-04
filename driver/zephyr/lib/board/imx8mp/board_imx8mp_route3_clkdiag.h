/* SPDX-License-Identifier: Apache-2.0 */
#ifndef BOARD_IMX8MP_ROUTE3_CLKDIAG_H_
#define BOARD_IMX8MP_ROUTE3_CLKDIAG_H_
/* board_route3_clkdiag.c: the A53 clock and VDD_ARM, read back, two lines. */
void board_route3_clock_report(void (*out)(const char *line));
#endif
