/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * STM32MP2 USART (the STM32H7 register layout) as a polled console for the
 * Cortex-A35, on the line setup the previous boot stage left: the DK's
 * console USART2 is enabled at 115200 8N1 by TF-A and U-Boot before
 * Zephyr runs. Nothing here writes CR1..CR3, BRR or PRESC, and nothing
 * touches the RCC or the pins, so the driver needs no clock tree; on a
 * USART nobody enabled it prints nothing (and poll_out stops waiting).
 *
 * Register offsets and status bits: Linux's STM32H7 USART description
 * (drivers/tty/serial/stm32-usart.c, stm32h7_info: isr 0x1c, icr 0x20,
 * rdr 0x24, tdr 0x28; drivers/tty/serial/stm32-usart.h: USART_SR_TXE,
 * USART_SR_RXNE, USART_SR_ORE, USART_SR_FE, USART_SR_PE, USART_ICR_*),
 * the same layout as RM0457's USART chapter.
 */
#define DT_DRV_COMPAT st_stm32h7_uart

#include <errno.h>
#include <zephyr/arch/cpu.h>
#include <zephyr/device.h>
#include <zephyr/devicetree.h>
#include <zephyr/drivers/uart.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/sys_io.h>
#include <zephyr/sys/util.h>

#define USART_ISR 0x1c
#define USART_ICR 0x20
#define USART_RDR 0x24
#define USART_TDR 0x28

#define USART_ISR_PE   BIT(0)
#define USART_ISR_FE   BIT(1)
#define USART_ISR_NE   BIT(2)
#define USART_ISR_ORE  BIT(3)
#define USART_ISR_RXNE BIT(5) /* RXFNE with the FIFO on: the same bit */
#define USART_ISR_TXE  BIT(7) /* TXFNF with the FIFO on: the same bit */

#define USART_ISR_ERRORS (USART_ISR_PE | USART_ISR_FE | USART_ISR_NE | USART_ISR_ORE)

/* At 115200 baud one character takes about 87 us; a USART that is not
 * clocked or not enabled never raises TXE, and the console must not hang
 * the boot on it.
 */
#define TXE_POLL_LIMIT 1000000

struct stm32mp2_uart_config {
	DEVICE_MMIO_ROM;
};

struct stm32mp2_uart_data {
	DEVICE_MMIO_RAM;
};

static inline mm_reg_t reg(const struct device *dev, uint32_t off)
{
	return DEVICE_MMIO_GET(dev) + off;
}

static int stm32mp2_uart_poll_in(const struct device *dev, unsigned char *c)
{
	uint32_t isr = sys_read32(reg(dev, USART_ISR));

	if (isr & USART_ISR_ERRORS) {
		/* The ICR clear bits sit at the ISR positions of the flags. */
		sys_write32(isr & USART_ISR_ERRORS, reg(dev, USART_ICR));
	}
	if (!(isr & USART_ISR_RXNE)) {
		return -1;
	}
	*c = (unsigned char)sys_read32(reg(dev, USART_RDR));
	return 0;
}

static void stm32mp2_uart_poll_out(const struct device *dev, unsigned char c)
{
	for (int i = 0; i < TXE_POLL_LIMIT; i++) {
		if (sys_read32(reg(dev, USART_ISR)) & USART_ISR_TXE) {
			break;
		}
	}
	sys_write32(c, reg(dev, USART_TDR));
}

static int stm32mp2_uart_err_check(const struct device *dev)
{
	uint32_t isr = sys_read32(reg(dev, USART_ISR));
	int err = 0;

	if (isr & USART_ISR_ORE) {
		err |= UART_ERROR_OVERRUN;
	}
	if (isr & USART_ISR_PE) {
		err |= UART_ERROR_PARITY;
	}
	if (isr & USART_ISR_FE) {
		err |= UART_ERROR_FRAMING;
	}
	if (isr & USART_ISR_NE) {
		err |= UART_ERROR_NOISE;
	}
	if (isr & USART_ISR_ERRORS) {
		sys_write32(isr & USART_ISR_ERRORS, reg(dev, USART_ICR));
	}
	return err;
}

static int stm32mp2_uart_init(const struct device *dev)
{
	DEVICE_MMIO_MAP(dev, K_MEM_CACHE_NONE);
	return 0;
}

static DEVICE_API(uart, stm32mp2_uart_api) = {
	.poll_in = stm32mp2_uart_poll_in,
	.poll_out = stm32mp2_uart_poll_out,
	.err_check = stm32mp2_uart_err_check,
};

#define STM32MP2_UART_INIT(n)                                                                      \
	static const struct stm32mp2_uart_config stm32mp2_uart_config_##n = {                      \
		DEVICE_MMIO_ROM_INIT(DT_DRV_INST(n)),                                              \
	};                                                                                         \
	static struct stm32mp2_uart_data stm32mp2_uart_data_##n;                                   \
	DEVICE_DT_INST_DEFINE(n, stm32mp2_uart_init, NULL, &stm32mp2_uart_data_##n,                \
			      &stm32mp2_uart_config_##n, PRE_KERNEL_1,                             \
			      CONFIG_SERIAL_INIT_PRIORITY, &stm32mp2_uart_api);

DT_INST_FOREACH_STATUS_OKAY(STM32MP2_UART_INIT)
