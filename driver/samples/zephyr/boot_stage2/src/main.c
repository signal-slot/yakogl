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
#include <zephyr/kernel/mm.h>
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

/* The success-path log (BOOT_STAGE2_PROGRESS_LOG): the console is polled,
 * so each line holds the boot for its length at the UART's rate. Errors
 * always print (printk). Off, the arguments are still type-checked. */
#define progress(...)                                                                              \
	do {                                                                                       \
		if (IS_ENABLED(CONFIG_BOOT_STAGE2_PROGRESS_LOG)) {                                 \
			printk(__VA_ARGS__);                                                       \
		}                                                                                  \
	} while (0)

/*
 * The loader's own timeline, for the image after it to report: the system
 * counter (CNTPCT_EL0, started by SPL) at the loader's stages, written into
 * a page of this loader's RAM that nothing else uses -- the middle of its
 * 32 MiB, well past the loader and the devicetree SPL appends to it, and
 * below the top of the region, where Zephyr places device mappings. The
 * image (at 0x48000000, its own RAM elsewhere) maps the page, copies the
 * stamps and clears the magic. Layout, also in the image's board_route3.c:
 * magic, count, then u64 t[8]: entry, main, payload in memory, unpacked
 * (both CRCs checked), jump, stored CRC checked (BS2_MARK_STORED_CHECKED),
 * decoded (BS2_MARK_DECODED), zero range cleared.
 */
#define BS2_STAMP_ADDR  0x41000000u
#define BS2_STAMP_MAGIC 0x54325342u /* 'BS2T' */
#define BS2_STAMPS      8u

struct bs2_stamps {
	uint32_t magic;
	uint32_t count;
	uint64_t t[BS2_STAMPS];
};

/*
 * Kept in .bss until main() maps the page: Zephyr's arm64 MMU maps the
 * loader's own sections, not the rest of its RAM, so a store to the page
 * before the mapping faults (2026-10-29: the loader stopped at its first
 * stamp, the panel stayed dark).
 */
static uint64_t bs2_t[BS2_STAMPS];
static volatile struct bs2_stamps *bs2_st;

static inline uint64_t cntpct(void)
{
	uint64_t v;

	__asm__ volatile("isb; mrs %0, cntpct_el0" : "=r"(v) : : "memory");
	return v;
}

#ifdef CONFIG_BOOT_STAGE2_RESTART
/*
 * Every entry is a first entry (BOOT_STAGE2_RESTART). The A53 image suspends
 * to this loader's entry point (cluster/a53_power.h) and TF-A resumes core 0
 * at __start with the MMU and caches off, at the EL it gave BL33 at power-on:
 * psci_get_ns_ep_info() takes the mode from SCR_EL3.HCE, not from the caller,
 * and HCE is set because imx8mp_bl31_setup.c entered BL33 at EL2. Nothing is
 * reloaded, so the RAM is as the first run left it. Zephyr's start code
 * (arch/arm64/core/reset.S, prep_c.c) rebuilds the stack, the MMU tables
 * (.bss), the GIC and the timers, and zeroes .bss again; it does not restore
 * the initialised RAM data (arch_data_copy() is empty without XIP). That
 * range, [__data_region_start, __data_region_end): .data (timeout_list,
 * whose nodes were in the old .bss, timeslicing's slice_ticks,
 * virt_region_bitmap, the console hooks) and the device states, whose
 * `initialized` bits the first run set.
 *
 * So the first entry copies the range into noinit room (data_snapshot.ld)
 * before anything writes it, and every later entry copies it back: the
 * range is then byte for byte what SPL loaded. Which entry this is comes
 * from the range itself: bs2_data_state is 'LOAD' as linked and set to
 * 'RUN!' once the copy is taken, so it reads 'LOAD' exactly when SPL has
 * just written the file. A power-page check could not decide this (DDR
 * holds anything at power-on, and a reset keeps a stale page).
 *
 * Runs in z_arm64_el_highest_plat_init(), the first hook of the reset path:
 * after __reset_prep_c set the stack (z_interrupt_stacks, noinit) and before
 * z_prep_c() (soc_prep_hook, the .bss clear). The MMU is off, so every data
 * access is Device-nGnRnE and must be aligned: the copies are
 * arch_early_memcpy() (Zephyr's MMU-off copy, early_mem_funcs.S), the other
 * accesses aligned volatile words. The caches hold nothing for these
 * addresses: on a first entry because the loader has not run, on a restart
 * because TF-A cleaned and invalidated them before the cluster went off.
 */
#include <zephyr/linker/linker-defs.h>
#include <zephyr/arch/common/init.h>
#include <cluster/a53_power.h>

#define BS2_DATA_LOADED 0x44414f4cu /* 'LOAD': as linked */
#define BS2_DATA_RAN    0x214e5552u /* 'RUN!': the copy is taken */

/* The linker script defines these in every build; linker-defs.h declares
 * them for XIP only. */
extern char __data_region_start[];
extern char __data_region_end[];
extern char bs2_data_snapshot[]; /* data_snapshot.ld: as many bytes as the range */

static volatile uint32_t bs2_data_state = BS2_DATA_LOADED;

BUILD_ASSERT(offsetof(struct cluster_a53_power, a53_state) == 12u, "a53_power.h layout");
BUILD_ASSERT(offsetof(struct cluster_a53_power, a53_loader_in) == 72u, "a53_power.h layout");

/* The weak hook of arch/arm64/core/reset.c; no SoC or board defines it. */
void z_arm64_el_highest_plat_init(void);

void z_arm64_el_highest_plat_init(void)
{
	volatile struct cluster_a53_power *pw =
		(volatile struct cluster_a53_power *)(uintptr_t)CLUSTER_A53_POWER_ADDR;
	const size_t n = (size_t)(__data_region_end - __data_region_start);

	/* After a suspend (the page says so): the loader's entry time, and
	 * nothing else in the page. */
	if (pw->magic == CLUSTER_A53_POWER_MAGIC && pw->a53_state == CLUSTER_A53_ST_SUSPENDING) {
		pw->a53_loader_in = cntpct();
	}

	if (bs2_data_state == BS2_DATA_LOADED) {
		arch_early_memcpy(bs2_data_snapshot, __data_region_start, n);
	} else {
		arch_early_memcpy(__data_region_start, bs2_data_snapshot, n);
	}
	/* The copy is in memory before the mark that says it is. */
	__asm__ volatile("dsb sy" : : : "memory");
	bs2_data_state = BS2_DATA_RAN;
	__asm__ volatile("dsb sy" : : : "memory");
}
#endif /* CONFIG_BOOT_STAGE2_RESTART */

static void bs2_stamp(unsigned int i)
{
	bs2_t[i] = cntpct();
}

void bs2_mark(uint32_t stage)
{
	if (stage < BS2_STAMPS) {
		bs2_stamp(stage);
	}
}

static int bs2_stamp_entry(void)
{
	bs2_stamp(0);
	return 0;
}
SYS_INIT(bs2_stamp_entry, PRE_KERNEL_1, 0);

/* Map the page and write the record; the magic last. Called just before the
 * jump, after the last stamp. */
static void bs2_stamps_publish(void)
{
	uint8_t *va = NULL;

	k_mem_map_phys_bare(&va, BS2_STAMP_ADDR, CONFIG_MMU_PAGE_SIZE,
			    K_MEM_CACHE_WB | K_MEM_PERM_RW);
	if (va == NULL) {
		return;
	}
	bs2_st = (volatile struct bs2_stamps *)va;
	bs2_st->magic = 0u;
	bs2_st->count = BS2_STAMPS;
	for (unsigned int i = 0; i < BS2_STAMPS; i++) {
		bs2_st->t[i] = bs2_t[i];
	}
	bs2_st->magic = BS2_STAMP_MAGIC;
}

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
	bs2_stamp(1);
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

	progress("boot_stage2: image header at eMMC user-area sector %d (payload up to sector %d), "
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
	progress("boot_stage2: eMMC user area %u sectors, ready after %lld ms\n", disk_sectors,
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

	progress("boot_stage2: image carried in this loader (%zu bytes at %p), load 0x%lx\n", payload_bytes,
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
	progress("boot_stage2: \"%s\": %llu bytes stored (%s), %llu bytes image, entry 0x%llx, "
	       "%llu bytes zeroed at +0x%llx\n",
	       info.name, info.stored_bytes,
	       info.compression == BS2_COMPRESSION_DEFLATE ? "deflate"
	       : info.compression == BS2_COMPRESSION_LZ4   ? "lz4"
							   : "uncompressed",
	       info.image_bytes, info.entry, info.zero_bytes, info.zero_offset);

	/* What the read and the inflate run on: the MMU and both caches
	 * (SCTLR_EL1 M, C, I), and the copy bandwidth into the payload region,
	 * 1 MiB timed. Before the read: LOAD is free here in both modes (the
	 * eMMC source reads an uncompressed payload straight to LOAD, which
	 * this probe would otherwise overwrite -- it did, on 2026-10-22). */
	if (IS_ENABLED(CONFIG_BOOT_STAGE2_PROGRESS_LOG)) {
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
	/* Uncompressed: straight to the load address. Compressed: to the
	 * staging copy above the image, then decoded down to the load address. */
	uint8_t *stored = (uint8_t *)(info.compression != BS2_COMPRESSION_NONE ? STAGING : LOAD);
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
	bs2_stamp(2);

	ret = bs2_unpack(&info, (uint8_t *)stored, (uint8_t *)LOAD, STAGING - LOAD);
	if (ret) {
		printk("boot_stage2: \"%s\" refused: %s\n", info.name, bs2_strerror(ret));
		halt();
	}
	int64_t t3 = k_uptime_get();
	bs2_stamp(3);

	/* The image's .bss, for an image that skips its own clear (header
	 * version 2; bs2_header_parse checked the range is past the image and
	 * inside the region). Here the caches are on; the image's reset path
	 * would clear it with them off. */
	if (info.zero_bytes != 0u) {
		memset((void *)(LOAD + (uintptr_t)info.zero_offset), 0, (size_t)info.zero_bytes);
	}
	bs2_stamp(7);

	progress("boot_stage2: \"%s\" ok (CRC-32 %08x): read %u sectors in %lld ms, unpacked in %lld ms; "
	       "jumping to 0x%llx at %lld ms\n", info.name, info.image_crc32, nsec, t2 - t1, t3 - t2,
	       info.entry, k_uptime_get() - t0);

	/* Let the UART drain the last line: the image re-initialises it. */
	if (IS_ENABLED(CONFIG_BOOT_STAGE2_PROGRESS_LOG)) {
		k_busy_wait(5000);
	}

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
	bs2_stamp(4);
	bs2_stamps_publish();
	sys_cache_data_flush_and_invd_all();
	sys_cache_instr_invd_all();
	bs2_jump(info.entry);
}
