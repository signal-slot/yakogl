/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * zephyr_shaderc_scratch.c - the memory the in-process shader compiler works
 * in, on Zephyr.
 *
 * The compiler is no_std and allocates nothing: it is handed one region and
 * bump-allocates its tokens, AST, IR and register allocation inside it. The
 * region is CALLER-LENT, so its size is a policy the product sets
 * (CONFIG_YAKOGL_SHADERC_SCRATCH_BYTES) rather than a constant the library
 * carries. A shader too large for it is then refused by name, with the shader
 * and both numbers in the message -- a failure someone can read and act on,
 * instead of an image that silently does not fit.
 *
 * This file is the Zephyr side of that: one static, lent once at boot. It is
 * separate from the two OSAL backends (zephyr.c, zephyr_gc.c) because it is
 * the same on both and neither should have to remember to call it; SYS_INIT
 * is what makes "before the first compile" true without an ordering argument
 * -- nothing can compile a shader before the application starts.
 *
 * Compiled only when CONFIG_YAKOGL_SHADERC_ONLINE is set. The offline build
 * has no compiler to lend to.
 */

#include <zephyr/init.h>
#include <zephyr/kernel.h>
#include <zephyr/logging/log.h>

LOG_MODULE_REGISTER(yakogl_shaderc, CONFIG_YAKOGL_LOG_LEVEL);

/* src/shaderc/online/store.rs. The length is checked there against
 * MIN_SCRATCH_BYTES, so a region too small to say anything useful about any
 * shader is refused rather than accepted and blamed on every shader in turn.
 * Returns 0, or -EINVAL. */
extern int shc_online_lend_scratch(unsigned char *buf, size_t len);

/* The same for the region compiled shaders are kept in
 * (CONFIG_YAKOGL_SHADERC_KEEP_BYTES): checked against MIN_KEEP_BYTES and
 * 8-byte alignment. Returns 0, -EINVAL, or -EBUSY once a shader is kept. */
extern int shc_online_lend_keep(unsigned char *buf, size_t len);

/* Not in .bss by accident: 8-byte aligned because the arena hands out
 * pointers into it for types the compiler aligns to 8. */
static unsigned char yakogl_shaderc_scratch[CONFIG_YAKOGL_SHADERC_SCRATCH_BYTES]
	__aligned(8);
static unsigned char yakogl_shaderc_keep[CONFIG_YAKOGL_SHADERC_KEEP_BYTES]
	__aligned(8);

static int yakogl_shaderc_scratch_init(void)
{
	int rc = shc_online_lend_scratch(yakogl_shaderc_scratch,
					 sizeof(yakogl_shaderc_scratch));

	if (rc != 0) {
		/* The only way here is a region the compiler will not accept,
		 * and the size came from Kconfig. Loud and early beats every
		 * later shader failing for a reason that points at the shader.
		 */
		LOG_ERR("shader compiler refused the %zu byte scratch region: %d",
			sizeof(yakogl_shaderc_scratch), rc);
		return rc;
	}

	rc = shc_online_lend_keep(yakogl_shaderc_keep, sizeof(yakogl_shaderc_keep));
	if (rc != 0) {
		LOG_ERR("shader compiler refused the %zu byte keep region: %d",
			sizeof(yakogl_shaderc_keep), rc);
		return rc;
	}

	LOG_INF("shader compiler scratch: %zu bytes, keep: %zu bytes",
		sizeof(yakogl_shaderc_scratch), sizeof(yakogl_shaderc_keep));
	return 0;
}

SYS_INIT(yakogl_shaderc_scratch_init, APPLICATION, 0);
