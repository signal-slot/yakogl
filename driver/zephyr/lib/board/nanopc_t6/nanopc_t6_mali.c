/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The Mali-G610's CSF firmware for a NanoPC-T6 GL image
 * (CONFIG_YAKOGL_GPU_MALI): what every such image embeds, so each sample
 * (gl_test, deqp) does not carry its own copy.
 *
 * - The firmware is driver/src/fw/mali_csffw_blob.h (made by
 *   driver/tools/mkmalifw.sh from the board's mali_csffw.bin; gitignored,
 *   never committed). The build stops at the include without it.
 * - It is the Mali OSAL's default platform (src/osal/zephyr_mali.c, weak
 *   there): the GL core's init(NULL), from eglInitialize or gl_test.c's Mali
 *   arm, takes it, as the Pi 4's board layer gives the V3D OSAL its default.
 * - Its size goes to the console before main(), with the SHA-256 that
 *   mkmalifw.sh took of the file it made the header from. That digest is a
 *   constant of the header, not computed here over the embedded bytes, and
 *   the line says so: a record names the file the image was built with.
 */
#include <zephyr/init.h>
#include <zephyr/sys/printk.h>

#include "osal_internal.h" /* struct osal_zephyr_mali_init */
#include "fw/mali_csffw_blob.h"

static const struct osal_zephyr_mali_init t6_mali_fw = {
	.fw = mali_csffw_blob,
	.fw_len = MALI_CSFFW_SIZE,
};

const struct osal_zephyr_mali_init *osal_zephyr_mali_default_platform(void)
{
	return &t6_mali_fw;
}

static int t6_mali_fw_report(void)
{
	printk("t6: CSF firmware %u bytes embedded, built from a file with sha256 %s "
	       "(mkmalifw.sh's digest, not computed at run time)\n",
	       (unsigned int)MALI_CSFFW_SIZE, MALI_CSFFW_SHA256);
	return 0;
}
SYS_INIT(t6_mali_fw_report, APPLICATION, 0);
