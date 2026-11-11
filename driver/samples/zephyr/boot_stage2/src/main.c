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
#ifdef CONFIG_BOOT_STAGE2_BOOT0_ON_REQUEST
#include <zephyr/drivers/pm_cpu_ops.h>
#include <zephyr/drivers/uart.h>
#include <zephyr/sd/sd_spec.h>
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

/* An uncompressed segment straight to its load address: the whole sectors,
 * then the last partial one through the header buffer, so nothing past the
 * segment's image is written. */
static int read_in_place(uint8_t *dst, uint32_t first, uint64_t bytes)
{
	uint32_t whole = (uint32_t)(bytes / SECTOR_BYTES);
	size_t tail = (size_t)(bytes % SECTOR_BYTES);

	if (read_sectors(dst, first, whole)) {
		return -1;
	}
	if (tail) {
		if (read_sectors(hdr, first + whole, 1)) {
			return -1;
		}
		memcpy(dst + (size_t)whole * SECTOR_BYTES, hdr, tail);
	}
	return 0;
}
#endif

#ifdef CONFIG_BOOT_STAGE2_BOOT0_ON_REQUEST
/*
 * The way back to the other boot chain without a button (the Verdin AM62P:
 * Toradex's chain in boot0, this one in boot1). A 'T' on the console while
 * the loader runs makes it point the eMMC's boot operation at boot partition
 * 1 again and reset the SoC, so the ROM's next read is the other chain.
 *
 * One byte of the EXT_CSD is written, PARTITION_CONFIG (179), with CMD6
 * SWITCH in its write-byte mode, and nothing else ever: no boot write
 * protection, no RST_n function, no other field. 0x48 is BOOT_ACK kept,
 * BOOT_PARTITION_ENABLE = 1 (boot0), PARTITION_ACCESS = 0 (user area); the
 * value this chain runs under is 0x50 (enable = 2, boot1). Coming back is
 * `mmc bootpart enable 2 1 /dev/mmcblk0` in Linux.
 */
#define EXT_CSD_PARTITION_CONFIG 179u
#define PARTITION_CONFIG_BOOT0   0x48u
#define MMC_SWITCH_WRITE_BYTE    3u
#define MMC_STATUS_SWITCH_ERROR  BIT(7)
#define MMC_STATUS_READY         BIT(8)
#define MMC_STATUS_STATE(s)      (((s) >> 9) & 0xfu)
#define MMC_STATE_TRAN           4u

static uint8_t ext_csd[512] __aligned(64);

/* Whether a 'T' has arrived on the console. Never waits: it takes what the
 * UART's receiver holds now and returns. */
static bool boot0_requested(void)
{
	const struct device *uart = DEVICE_DT_GET(DT_CHOSEN(zephyr_console));
	unsigned char c;
	bool seen = false;

	if (!device_is_ready(uart)) {
		return false;
	}
	for (int i = 0; i < 256 && uart_poll_in(uart, &c) == 0; i++) {
		seen = seen || c == 'T';
	}
	return seen;
}

static int ext_csd_read(const struct device *sdhc)
{
	struct sdhc_command cmd = {
		.opcode = MMC_SEND_EXT_CSD,
		.response_type = SD_RSP_TYPE_R1,
		.timeout_ms = CONFIG_SD_CMD_TIMEOUT,
	};
	struct sdhc_data data = {
		.block_size = sizeof(ext_csd),
		.blocks = 1,
		.data = ext_csd,
		.timeout_ms = CONFIG_SD_DATA_TIMEOUT,
	};

	return sdhc_request(sdhc, &cmd, &data);
}

/* PARTITION_CONFIG = boot0, checked by reading it back. 0 when the card
 * holds the new value. */
static int boot0_select(const struct device *sdhc)
{
	struct sdhc_command sw = {
		.opcode = SD_SWITCH,
		.arg = (MMC_SWITCH_WRITE_BYTE << 24) | (EXT_CSD_PARTITION_CONFIG << 16) |
		       (PARTITION_CONFIG_BOOT0 << 8),
		.response_type = SD_RSP_TYPE_R1b,
		.timeout_ms = 1000,
	};
	struct sdhc_command st = {
		.opcode = SD_SEND_STATUS,
		.arg = (uint32_t)CONFIG_MMC_RCA << 16,
		.response_type = SD_RSP_TYPE_R1,
		.timeout_ms = CONFIG_SD_CMD_TIMEOUT,
	};
	uint8_t before;
	int ret;

	ret = ext_csd_read(sdhc);
	if (ret) {
		printk("boot_stage2: boot0 request: EXT_CSD read failed: %d\n", ret);
		return ret;
	}
	before = ext_csd[EXT_CSD_PARTITION_CONFIG];
	ret = sdhc_request(sdhc, &sw, NULL);
	if (ret) {
		printk("boot_stage2: boot0 request: CMD6 failed: %d\n", ret);
		return ret;
	}
	/* The card holds DAT0 low while it programs the byte, then says in its
	 * status whether it took it. */
	for (int ms = 0;; ms++) {
		if (ms == 2000) {
			printk("boot_stage2: boot0 request: the card stayed busy after CMD6\n");
			return -ETIMEDOUT;
		}
		if (sdhc_card_busy(sdhc) == 0) {
			st.response[0] = 0;
			ret = sdhc_request(sdhc, &st, NULL);
			if (ret == 0 && (st.response[0] & MMC_STATUS_READY) &&
			    MMC_STATUS_STATE(st.response[0]) == MMC_STATE_TRAN) {
				break;
			}
		}
		k_busy_wait(1000);
	}
	if (st.response[0] & MMC_STATUS_SWITCH_ERROR) {
		printk("boot_stage2: boot0 request: the card refused the switch (status 0x%08x)\n",
		       st.response[0]);
		return -EIO;
	}
	memset(ext_csd, 0, sizeof(ext_csd));
	ret = ext_csd_read(sdhc);
	if (ret) {
		printk("boot_stage2: boot0 request: EXT_CSD read-back failed: %d\n", ret);
		return ret;
	}
	printk("boot_stage2: boot0 request: PARTITION_CONFIG 0x%02x -> 0x%02x (wanted 0x%02x)\n", before,
	       ext_csd[EXT_CSD_PARTITION_CONFIG], PARTITION_CONFIG_BOOT0);
	return ext_csd[EXT_CSD_PARTITION_CONFIG] == PARTITION_CONFIG_BOOT0 ? 0 : -EIO;
}

/* Called where the eMMC is up and nothing is half done. Returns when no 'T'
 * came or the switch failed (the boot then goes on as usual). */
static void boot0_on_request(void)
{
	if (!boot0_requested()) {
		return;
	}
	printk("boot_stage2: 'T' on the console: eMMC boot partition 1 (boot0) for the next start\n");
	if (boot0_select(DEVICE_DT_GET(SDHC_NODE)) != 0) {
		printk("boot_stage2: boot0 request failed; booting this chain as usual\n");
		return;
	}
	printk("boot_stage2: resetting the SoC (PSCI SYSTEM_RESET)\n");
	k_busy_wait(20000); /* the line, out of the UART */
	(void)pm_system_reset(SYS_COLD_RESET);
	printk("boot_stage2: the reset did not happen; power-cycle to start from boot0\n");
	halt();
}
#else
static inline void boot0_on_request(void)
{
}
#endif

int main(void)
{
	struct bs2_info info;
	int64_t t0 = k_uptime_get();
	int ret;

#ifdef CONFIG_BOOT_STAGE2_SOURCE_EMMC
	/* A compressed segment is read to STAGING + its stored offset: the
	 * whole range is the staging buffer, and no segment may overlap it. */
	const struct bs2_limits limits = {
		.load = LOAD,
		.region_end = REGION_END,
		.max_stored = MAX_STORED,
		.staging = STAGING,
		.staging_bytes = MAX_STORED,
	};
	uint32_t disk_sectors = 0;

	printk("boot_stage2: image header at eMMC user-area sector %d (payload up to sector %d), "
	       "load 0x%lx, staging 0x%lx\n", HDR_SECTOR, END_SECTOR - 1, (unsigned long)LOAD,
	       (unsigned long)STAGING);

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
	boot0_on_request();

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
	/* Nothing is staged: the stored bytes are read from this image's
	 * .rodata, so the staging range is free for segments. */
	const struct bs2_limits limits = {
		.load = LOAD,
		.region_end = REGION_END,
		.max_stored = payload_bytes > SECTOR_BYTES ? payload_bytes - SECTOR_BYTES : 0,
		.staging = STAGING,
		.staging_bytes = 0,
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
	printk("boot_stage2: \"%s\": header version %u, %u segment(s), %llu bytes stored, entry 0x%llx\n",
	       info.name, info.version, info.count, info.stored_bytes, info.entry);
	for (uint32_t i = 0; i < info.count; i++) {
		const struct bs2_segment *s = &info.segments[i];

		printk("boot_stage2:   [%u] 0x%llx..0x%llx: %llu bytes image (%s, %llu stored at +0x%llx), "
		       "footprint %llu\n", i, s->load, s->load + s->image_bytes, s->image_bytes,
		       s->compression == BS2_COMPRESSION_DEFLATE ? "deflate" : "uncompressed",
		       s->stored_bytes, s->stored_offset, s->footprint);
	}

	/* What the read and the inflate run on: the MMU and both caches
	 * (SCTLR_EL1 M, C, I), and the copy bandwidth into the payload region,
	 * 1 MiB timed. Before the read: LOAD is free here in both modes (the
	 * eMMC source reads an uncompressed segment 0 straight to LOAD, which
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
	int64_t t_read = 0, t_unpack = 0;
	uint32_t nsec_total = 0;

	for (uint32_t i = 0; i < info.count; i++) {
		const struct bs2_segment *s = &info.segments[i];
		int64_t t1 = k_uptime_get();
#ifdef CONFIG_BOOT_STAGE2_SOURCE_EMMC
		/* Uncompressed: straight to the load address. Deflate: to the
		 * staging buffer at the segment's stored offset (no segment
		 * overlaps it: the header check), then inflated to the load
		 * address. */
		const uint32_t first = HDR_SECTOR + 1 + (uint32_t)(s->stored_offset / SECTOR_BYTES);
		const uint32_t nsec = (uint32_t)((s->stored_bytes + SECTOR_BYTES - 1) / SECTOR_BYTES);
		uint8_t *stored;

		if (s->compression == BS2_COMPRESSION_DEFLATE) {
			stored = (uint8_t *)(STAGING + (uintptr_t)s->stored_offset);
			if (read_sectors(stored, first, nsec)) {
				halt();
			}
		} else {
			stored = (uint8_t *)(uintptr_t)s->load;
			if (read_in_place(stored, first, s->stored_bytes)) {
				halt();
			}
		}
		nsec_total += nsec;
#else
		/* The stored bytes follow the header inside this image;
		 * bs2_segment_unpack copies or inflates them to the load address. */
		const uint8_t *stored = bs2_payload_start + SECTOR_BYTES + (size_t)s->stored_offset;
#endif
		int64_t t2 = k_uptime_get();

		ret = bs2_segment_unpack(&info, i, stored);
		if (ret) {
			printk("boot_stage2: \"%s\" segment %u refused: %s\n", info.name, i, bs2_strerror(ret));
			halt();
		}
		t_read += t2 - t1;
		t_unpack += k_uptime_get() - t2;
	}
	int64_t t3 = k_uptime_get();

	/* once more, with everything read: the last moment the eMMC is up */
	boot0_on_request();
	printk("boot_stage2: \"%s\" ok (%u segment(s), segment 0 CRC-32 %08x): read %u sectors in %lld ms, "
	       "unpacked in %lld ms; jumping to 0x%llx at %lld ms\n", info.name, info.count,
	       info.segments[0].image_crc32, nsec_total, t_read, t_unpack, info.entry, t3 - t0);

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
