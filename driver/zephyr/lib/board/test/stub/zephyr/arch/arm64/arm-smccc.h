/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* Host stub: Zephyr's SMCCC call (include/zephyr/arch/arm64/arm-smccc.h, the
 * same declaration); the test defines arm_smccc_smc as a recorder. */
#ifndef STUB_ZEPHYR_ARCH_ARM64_ARM_SMCCC_H
#define STUB_ZEPHYR_ARCH_ARM64_ARM_SMCCC_H
struct arm_smccc_res {
	unsigned long a0;
	unsigned long a1;
	unsigned long a2;
	unsigned long a3;
	unsigned long a4;
	unsigned long a5;
	unsigned long a6;
	unsigned long a7;
};
void arm_smccc_smc(unsigned long a0, unsigned long a1, unsigned long a2, unsigned long a3,
		   unsigned long a4, unsigned long a5, unsigned long a6, unsigned long a7,
		   struct arm_smccc_res *res);
#endif
