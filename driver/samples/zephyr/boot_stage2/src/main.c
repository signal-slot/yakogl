/*
 * SPDX-License-Identifier: Apache-2.0
 *
 * The second-stage loader (docs/notes/am62p-direct-boot.md "The second
 * stage", docs/notes/imx8mp-direct-boot.md). The boot loader's last stage
 * (the AM62P's A53 SPL, the i.MX8MP's TF-A) starts this image in the slot
 * U-Boot proper had, which is too small for a GL image. The loader takes the
 * real image either from its own .rodata (BOOT_STAGE2_SOURCE_EMBEDDED: the
 * compressed payload is linked into this image at build time) or from a raw
 * sector range of the eMMC user area (BOOT_STAGE2_SOURCE_EMMC), checks it,
 * puts it at its load address and jumps to it with the MMU and caches off,
 * at EL1.
 *
 * This file is the Zephyr side only: the payload's source (the eMMC through
 * Zephyr's SD stack and the TI AM654 SDHCI driver, or the linked bytes), the
 * log, quiescing the controller and the cache maintenance. The header check,
 * the CRC-32s, the inflate and the jump are Rust (loader/, include/boot_stage2.h).
 */

#include <zephyr/kernel.h>
#include <zephyr/cache.h>
#include <zephyr/devicetree.h>
#include <zephyr/irq.h>
#include <zephyr/sys/printk.h>
#include <string.h>
#ifdef CONFIG_BOOT_STAGE2_SOURCE_EMMC
#include <zephyr/drivers/sdhc.h>
#include <zephyr/storage/disk_access.h>
#endif

#include "boot_stage2.h"

#define SECTOR_BYTES 512u
#define HDR_SECTOR   CONFIG_BOOT_STAGE2_SECTOR
#define END_SECTOR   CONFIG_BOOT_STAGE2_END_SECTOR
#define LOAD         ((uintptr_t)CONFIG_BOOT_STAGE2_LOAD)
#define STAGING      ((uintptr_t)CONFIG_BOOT_STAGE2_STAGING)
#define REGION_END   ((uintptr_t)CONFIG_BOOT_STAGE2_REGION_END)
#define MAX_STORED   ((uint64_t)(END_SECTOR - HDR_SECTOR - 1) * SECTOR_BYTES)

#define PAYLOAD_NODE DT_NODELABEL(stage2_payload)

/* The Kconfig addresses must lie in the identity-mapped region the overlay
 * declares, and the image must end where the staging copy begins. */
BUILD_ASSERT(DT_REG_ADDR(PAYLOAD_NODE) == CONFIG_BOOT_STAGE2_LOAD, "region starts at the load address");
BUILD_ASSERT(DT_REG_ADDR(PAYLOAD_NODE) + DT_REG_SIZE(PAYLOAD_NODE) == CONFIG_BOOT_STAGE2_REGION_END,
	     "region ends at BOOT_STAGE2_REGION_END");
BUILD_ASSERT(CONFIG_BOOT_STAGE2_LOAD < CONFIG_BOOT_STAGE2_STAGING, "staging above the load address");
BUILD_ASSERT(CONFIG_SRAM_BASE_ADDRESS + CONFIG_SRAM_SIZE * 1024ull <= CONFIG_BOOT_STAGE2_LOAD,
	     "the loader's own RAM ends below the image");

#ifdef CONFIG_BOOT_STAGE2_SOURCE_EMMC
#define SDHC_NODE DT_NODELABEL(main_sdhci0)
BUILD_ASSERT((uint64_t)CONFIG_BOOT_STAGE2_STAGING + MAX_STORED <= CONFIG_BOOT_STAGE2_REGION_END,
	     "the largest payload fits between staging and the region's end");
BUILD_ASSERT(END_SECTOR > HDR_SECTOR + 1, "room for a payload");

static uint8_t hdr[SECTOR_BYTES] __aligned(64);
#else
/* payload.S (CMake, -DBOOT_STAGE2_PAYLOAD): the stage2 .img, header first. */
extern const uint8_t bs2_payload_start[];
extern const uint8_t bs2_payload_end[];
#endif

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

#ifdef CONFIG_BOOT_STAGE2_SOURCE_EMMC
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
#endif

int main(void)
{
	struct bs2_info info;
	int64_t t0 = k_uptime_get();
	int ret;

#ifdef CONFIG_BOOT_STAGE2_SOURCE_EMMC
	const struct bs2_limits limits = {
		.load = LOAD,
		.max_stored = MAX_STORED,
		.max_image = STAGING - LOAD,
	};
	uint32_t disk_sectors = 0;

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
#else
	const size_t payload_bytes = (size_t)(bs2_payload_end - bs2_payload_start);
	const struct bs2_limits limits = {
		.load = LOAD,
		.max_stored = payload_bytes > SECTOR_BYTES ? payload_bytes - SECTOR_BYTES : 0,
		.max_image = STAGING - LOAD,
	};

	printk("boot_stage2: image carried in this loader (%zu bytes at %p), load 0x%lx\n", payload_bytes,
	       (const void *)bs2_payload_start, (unsigned long)LOAD);
	if (payload_bytes <= SECTOR_BYTES) {
		printk("boot_stage2: no payload linked in (BOOT_STAGE2_PAYLOAD at build time)\n");
		halt();
	}
	ret = bs2_header_parse(bs2_payload_start, SECTOR_BYTES, &limits, &info);
	if (ret) {
		printk("boot_stage2: the linked payload's header is refused: %s\n", bs2_strerror(ret));
		halt();
	}
#endif
	printk("boot_stage2: \"%s\": %llu bytes stored (%s), %llu bytes image, entry 0x%llx\n",
	       info.name, info.stored_bytes,
	       info.compression == BS2_COMPRESSION_DEFLATE ? "deflate" : "uncompressed",
	       info.image_bytes, info.entry);

	/* What the read and the inflate run on: the MMU and both caches
	 * (SCTLR_EL1 M, C, I), and the copy bandwidth into the payload region,
	 * 1 MiB timed. Before the read: LOAD is free here in both modes (the
	 * eMMC source reads an uncompressed payload straight to LOAD, which
	 * this probe would otherwise overwrite -- it did, on 2026-10-22). */
	{
		uint64_t sctlr;
		__asm__ volatile("mrs %0, sctlr_el1" : "=r"(sctlr));
		uint32_t c0 = k_cycle_get_32();
		memcpy((void *)LOAD, (const void *)(LOAD + (1u << 20)), 1u << 20);
		uint32_t c1 = k_cycle_get_32();
		printk("boot_stage2: SCTLR_EL1 M=%u C=%u I=%u; memcpy 1 MiB in %u us\n",
		       (unsigned)(sctlr & 1), (unsigned)((sctlr >> 2) & 1), (unsigned)((sctlr >> 12) & 1),
		       (unsigned)(((uint64_t)(c1 - c0) * 1000000u) / sys_clock_hw_cycles_per_sec()));
	}
	int64_t t1 = k_uptime_get();
#ifdef CONFIG_BOOT_STAGE2_SOURCE_EMMC
	/* Uncompressed: straight to the load address. Deflate: to the staging
	 * copy above the image, then inflated down to the load address. */
	uint8_t *stored = (uint8_t *)(info.compression == BS2_COMPRESSION_DEFLATE ? STAGING : LOAD);
	uint32_t nsec = (uint32_t)((info.stored_bytes + SECTOR_BYTES - 1) / SECTOR_BYTES);

	if (read_sectors(stored, HDR_SECTOR + 1, nsec)) {
		halt();
	}
#else
	/* The stored bytes follow the header inside this image; bs2_unpack
	 * copies or inflates them to the load address. */
	const uint8_t *stored = bs2_payload_start + SECTOR_BYTES;
	uint32_t nsec = 0;
#endif
	int64_t t2 = k_uptime_get();

	ret = bs2_unpack(&info, (uint8_t *)stored, (uint8_t *)LOAD, STAGING - LOAD);
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
#ifdef CONFIG_BOOT_STAGE2_SOURCE_EMMC
	irq_disable(DT_IRQN(SDHC_NODE));
	(void)sdhc_hw_reset(DEVICE_DT_GET(SDHC_NODE));
#endif
	(void)irq_lock();

	/* Everything the image consists of is in the data cache or memory:
	 * clean and invalidate all of it to the point of coherency, then the
	 * instruction cache; bs2_jump turns the MMU and caches off. */
	sys_cache_data_flush_and_invd_all();
	sys_cache_instr_invd_all();
	bs2_jump(info.entry);
}
