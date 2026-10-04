/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * The Verdin AM62P second-stage loader (docs/direct-boot.md, "The second
 * stage"). The A53 SPL loads this image from the eMMC boot partition's
 * u-boot.img slot (1.49 MiB, too small for a GL image) at 0x80800000. It
 * reads the real image from a raw sector range of the eMMC user area, checks
 * it, puts it at its load address and jumps to it with the MMU and caches
 * off, at EL1.
 *
 * This file is the Zephyr side only: the eMMC (Zephyr's SD stack and the TI
 * AM654 SDHCI driver, through the disk API), the log, quiescing the
 * controller and the cache maintenance. The header check, the CRC-32s, the
 * inflate and the jump are Rust (loader/, include/boot_stage2.h).
 */

#include <zephyr/kernel.h>
#include <zephyr/cache.h>
#include <zephyr/devicetree.h>
#include <zephyr/drivers/sdhc.h>
#include <zephyr/irq.h>
#include <zephyr/storage/disk_access.h>
#include <zephyr/sys/printk.h>

#include "boot_stage2.h"

#define SECTOR_BYTES 512u
#define HDR_SECTOR   CONFIG_BOOT_STAGE2_SECTOR
#define END_SECTOR   CONFIG_BOOT_STAGE2_END_SECTOR
#define LOAD         ((uintptr_t)CONFIG_BOOT_STAGE2_LOAD)
#define STAGING      ((uintptr_t)CONFIG_BOOT_STAGE2_STAGING)
#define REGION_END   ((uintptr_t)CONFIG_BOOT_STAGE2_REGION_END)
#define MAX_STORED   ((uint64_t)(END_SECTOR - HDR_SECTOR - 1) * SECTOR_BYTES)

#define PAYLOAD_NODE DT_NODELABEL(stage2_payload)
#define SDHC_NODE    DT_NODELABEL(main_sdhci0)

/* The Kconfig addresses must lie in the identity-mapped region the overlay
 * declares, and the image must end where the staging copy begins. */
BUILD_ASSERT(DT_REG_ADDR(PAYLOAD_NODE) == CONFIG_BOOT_STAGE2_LOAD, "region starts at the load address");
BUILD_ASSERT(DT_REG_ADDR(PAYLOAD_NODE) + DT_REG_SIZE(PAYLOAD_NODE) == CONFIG_BOOT_STAGE2_REGION_END,
	     "region ends at BOOT_STAGE2_REGION_END");
BUILD_ASSERT(CONFIG_BOOT_STAGE2_LOAD < CONFIG_BOOT_STAGE2_STAGING, "staging above the load address");
BUILD_ASSERT((uint64_t)CONFIG_BOOT_STAGE2_STAGING + MAX_STORED <= CONFIG_BOOT_STAGE2_REGION_END,
	     "the largest payload fits between staging and the region's end");
BUILD_ASSERT(CONFIG_SRAM_BASE_ADDRESS + CONFIG_SRAM_SIZE * 1024ull <= CONFIG_BOOT_STAGE2_LOAD,
	     "the loader's own RAM ends below the image");
BUILD_ASSERT(END_SECTOR > HDR_SECTOR + 1, "room for a payload");

static uint8_t hdr[SECTOR_BYTES] __aligned(64);

static FUNC_NORETURN void halt(void)
{
	printk("boot_stage2: stopped; power-cycle (or USB recovery mode) to continue\n");
	for (;;) {
		k_sleep(K_FOREVER);
	}
}

FUNC_NORETURN void bs2_panic(const char *file, size_t file_len, uint32_t line)
{
	printk("boot_stage2: Rust panic at %.*s:%u\n", (int)file_len, file, line);
	halt();
}

static int read_sectors(uint8_t *buf, uint32_t first, uint32_t count)
{
	while (count) {
		uint32_t n = MIN(count, (uint32_t)CONFIG_BOOT_STAGE2_CHUNK_SECTORS);
		int ret = disk_access_read(CONFIG_BOOT_STAGE2_DISK, buf, first, n);

		if (ret) {
			printk("boot_stage2: read of sectors %u..%u failed: %d\n", first, first + n - 1, ret);
			return ret;
		}
		buf += (size_t)n * SECTOR_BYTES;
		first += n;
		count -= n;
	}
	return 0;
}

int main(void)
{
	const struct bs2_limits limits = {
		.load = LOAD,
		.max_stored = MAX_STORED,
		.max_image = STAGING - LOAD,
	};
	struct bs2_info info;
	uint32_t disk_sectors = 0;
	int64_t t0 = k_uptime_get();
	int ret;

	printk("boot_stage2: image header at eMMC user-area sector %d (payload up to sector %d), "
	       "load 0x%lx\n", HDR_SECTOR, END_SECTOR - 1, (unsigned long)LOAD);

	ret = disk_access_init(CONFIG_BOOT_STAGE2_DISK);
	if (ret) {
		printk("boot_stage2: eMMC \"%s\" init failed: %d\n", CONFIG_BOOT_STAGE2_DISK, ret);
		halt();
	}
	ret = disk_access_ioctl(CONFIG_BOOT_STAGE2_DISK, DISK_IOCTL_GET_SECTOR_COUNT, &disk_sectors);
	if (ret || disk_sectors < END_SECTOR) {
		printk("boot_stage2: eMMC user area has %u sectors (%d), fewer than %d\n", disk_sectors,
		       ret, END_SECTOR);
		halt();
	}
	printk("boot_stage2: eMMC user area %u sectors, ready after %lld ms\n", disk_sectors,
	       k_uptime_get() - t0);

	if (read_sectors(hdr, HDR_SECTOR, 1)) {
		halt();
	}
	ret = bs2_header_parse(hdr, sizeof(hdr), &limits, &info);
	if (ret) {
		printk("boot_stage2: header at sector %d refused: %s\n", HDR_SECTOR, bs2_strerror(ret));
		halt();
	}
	printk("boot_stage2: \"%s\": %llu bytes stored (%s), %llu bytes image, entry 0x%llx\n",
	       info.name, info.stored_bytes,
	       info.compression == BS2_COMPRESSION_DEFLATE ? "deflate" : "uncompressed",
	       info.image_bytes, info.entry);

	/* Uncompressed: straight to the load address. Deflate: to the staging
	 * copy above the image, then inflated down to the load address. */
	uint8_t *stored = (uint8_t *)(info.compression == BS2_COMPRESSION_DEFLATE ? STAGING : LOAD);
	uint32_t nsec = (uint32_t)((info.stored_bytes + SECTOR_BYTES - 1) / SECTOR_BYTES);
	int64_t t1 = k_uptime_get();

	if (read_sectors(stored, HDR_SECTOR + 1, nsec)) {
		halt();
	}
	int64_t t2 = k_uptime_get();

	ret = bs2_unpack(&info, stored, (uint8_t *)LOAD, STAGING - LOAD);
	if (ret) {
		printk("boot_stage2: \"%s\" refused: %s\n", info.name, bs2_strerror(ret));
		halt();
	}
	int64_t t3 = k_uptime_get();

	printk("boot_stage2: \"%s\" ok (CRC-32 %08x): read %u sectors in %lld ms, unpacked in %lld ms; "
	       "jumping to 0x%llx at %lld ms\n", info.name, info.image_crc32, nsec, t2 - t1, t3 - t2,
	       info.entry, t3 - t0);

	/* Let the UART drain the last line: the image re-initialises it. */
	k_busy_wait(5000);

	/* Quiesce what this image started: the SDHC (its interrupt and the
	 * controller, reset), then interrupts off; the timers are stopped by
	 * bs2_jump. The image's own GIC set-up clears the distributor. */
	irq_disable(DT_IRQN(SDHC_NODE));
	(void)sdhc_hw_reset(DEVICE_DT_GET(SDHC_NODE));
	(void)irq_lock();

	/* Everything the image consists of is in the data cache or memory:
	 * clean and invalidate all of it to the point of coherency, then the
	 * instruction cache; bs2_jump turns the MMU and caches off. */
	sys_cache_data_flush_and_invd_all();
	sys_cache_instr_invd_all();
	bs2_jump(info.entry);
}
