/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The GPU's interrupt on the STM32MP257F-DK as this image sees it
 * (gc_board_irq_connected, drivers/gpu/gpu_vivante_gc_rust.c: called once
 * gc_irq_connect has run IRQ_CONNECT and irq_enable). Plan #349, issues #7
 * and #1281.
 *
 * The GPU is GIC SPI 183, level high (ST's Linux tree; the SoC's dtsi), on
 * the GIC-400 whose distributor is at 0x4ac10000. The image runs
 * non-secure at EL1, so every distributor register reads its non-secure
 * view: GICD_IGROUPRn is RAZ/WI from there whatever the group, and every
 * other bit of a group 0 (secure) interrupt is RAZ/WI as well. After the
 * connect has written the priority, the trigger and the enable, a line in
 * group 1 reads back enable=1, prio=0xa0 (IRQ_DEFAULT_PRIORITY) and a
 * target with this CPU interface's bit; a line that TF-A or OP-TEE left in
 * group 0 reads back all zeros (route=raz). So the line tells "no interrupt
 * from the GPU" (route=ours, pend=0 after a fence) apart from "the interrupt
 * is not routed to us". One printk: the line stays whole on the console.
 *
 *   gc_irq: spi=183 intid=215 enable= cfg= target= self= prio= group=
 *     pend= active= gicd_ctlr= typer= secext= gicc_ctlr= pmr= route=
 *
 * cfg is the line's two GICD_ICFGR bits (0 level, 2 edge); self is this CPU
 * interface's mask (GICD_ITARGETSR0, banked); gicd_ctlr, gicc_ctlr and pmr
 * are the non-secure views (gicc_ctlr bit 9 is EOImodeNS: set, every
 * interrupt would stay active after its first EOI). route is the first of:
 * raz (enable, prio and target all read 0: group 0, or no such line),
 * dist-off, cpuif-off, disabled, other-cpu, masked (prio not below pmr),
 * ours.
 *
 * Reads only: the GIC distributor and CPU interface, nothing of the GPU.
 */
#include <zephyr/kernel.h>
#include <zephyr/devicetree.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/sys_io.h>
#include <zephyr/dt-bindings/interrupt-controller/arm-gic.h>
#include <zephyr/drivers/interrupt_controller/gic.h>

#define MP2_GPU_NODE DT_NODELABEL(gpu)
#define MP2_GPU_SPI  183u

/* The devicetree's irq cell of a GIC_SPI is already the INTID (Zephyr's
 * gen_defines adds the 32 SGIs and PPIs). */
BUILD_ASSERT(DT_IRQ(MP2_GPU_NODE, type) == GIC_SPI &&
		     DT_IRQN(MP2_GPU_NODE) == GIC_SPI_INT_BASE + MP2_GPU_SPI,
	     "the DK's GPU is GIC SPI 183, INTID 215 (ST's Linux tree, #1281)");
BUILD_ASSERT((DT_IRQ(MP2_GPU_NODE, flags) & IRQ_TYPE_EDGE) == 0,
	     "SPI 183 is level-sensitive (IRQ_TYPE_LEVEL_HIGH in ST's tree)");
BUILD_ASSERT(GIC_DIST_BASE == 0x4ac10000UL, "the GIC-400's distributor on the STM32MP257F");

#define GICD_CTLR_ENABLE  BIT(0)
#define GICC_CTLR_ENABLE  BIT(0) /* EnableGrp1 in the non-secure view */
#define GICD_TYPER_SECEXT BIT(10)

void gc_board_irq_connected(unsigned int irq);
void gc_board_irq_connected(unsigned int irq)
{
	uint32_t word = (irq / 32u) * 4u;
	uint32_t bit = BIT(irq % 32u);
	uint32_t enable = (sys_read32(GICD_ISENABLERn + word) & bit) ? 1u : 0u;
	uint32_t group = (sys_read32(GICD_IGROUPRn + word) & bit) ? 1u : 0u;
	uint32_t pend = (sys_read32(GICD_ISPENDRn + word) & bit) ? 1u : 0u;
	uint32_t active = (sys_read32(GICD_ISACTIVERn + word) & bit) ? 1u : 0u;
	uint32_t cfg = (sys_read32(GICD_ICFGRn + (irq / 16u) * 4u) >> ((irq % 16u) * 2u)) & 3u;
	uint8_t target = sys_read8(GICD_ITARGETSRn + irq);
	uint8_t self = sys_read8(GICD_ITARGETSRn);
	uint8_t prio = sys_read8(GICD_IPRIORITYRn + irq);
	uint32_t gicd_ctlr = sys_read32(GICD_CTLR);
	uint32_t typer = sys_read32(GICD_TYPER);
	uint32_t gicc_ctlr = sys_read32(GICC_CTLR);
	uint32_t pmr = sys_read32(GICC_PMR) & 0xffu;
	const char *route;

	if (enable == 0u && prio == 0u && target == 0u) {
		route = "raz";
	} else if ((gicd_ctlr & GICD_CTLR_ENABLE) == 0u) {
		route = "dist-off";
	} else if ((gicc_ctlr & GICC_CTLR_ENABLE) == 0u) {
		route = "cpuif-off";
	} else if (enable == 0u) {
		route = "disabled";
	} else if ((target & self) == 0u) {
		route = "other-cpu";
	} else if (prio >= pmr) {
		route = "masked";
	} else {
		route = "ours";
	}
	printk("gc_irq: spi=%u intid=%u enable=%u cfg=%u target=0x%02x self=0x%02x prio=0x%02x "
	       "group=%u pend=%u active=%u gicd_ctlr=0x%x typer=0x%x secext=%u gicc_ctlr=0x%x "
	       "pmr=0x%02x route=%s\n",
	       irq - GIC_SPI_INT_BASE, irq, enable, cfg, target, self, prio, group, pend, active,
	       gicd_ctlr, typer, (typer & GICD_TYPER_SECEXT) ? 1u : 0u, gicc_ctlr, pmr, route);
}
