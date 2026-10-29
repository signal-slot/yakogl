/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Route 3 from the direct image (board_imx8mp_route3.h): the M7
 * loaded from this image, released through TF-A, and fed through the frame
 * ring. Design, addresses and the reasons for the order:
 * docs/notes/imx8mp-direct-boot.md, "Route 3". The ring protocol is the
 * shared header <cluster/frame_ring.h>; the A53 producer that ran on Torizon
 * on 2026-10-03 was gc_common/ring_a53.c (removed in dac31030953),
 * whose map, status checks and buffer rule this file keeps.
 *
 * The order, which the M7 firmware dictates (its first tick reads the LCDIF,
 * a bus fault while MEDIAMIX is off):
 *   first board_window()  map the carve-out pieces, load the M7 image (held:
 *                         IOMUXC_GPR22 CPUWAIT), set the ring up
 *   first board_show()    the panel on this frame (the C cold path, the
 *                         backlight still off), publish frame 1, the
 *                         heartbeat, then the release (SiP 0xC2000005,
 *                         x1 = 0); the M7 lights the backlight when it
 *                         adopts the panel, with its layer on the frame
 *   later shows           publish; wait until the M7's `shown` reaches the
 *                         frame (the previous window is then free)
 * The A53 never writes an LCDIF register after the first show, unless the M7
 * is declared ABSENT (no 'SAF7' in YAKOGL_BOARD_IMX8MP_ROUTE3_M7_WAIT_MS):
 * then it is stopped again, the ring dropped, and the caller flips.
 */
#include <zephyr/kernel.h>
#include <zephyr/drivers/uart.h>
#include <zephyr/kernel/mm.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/sys_io.h>
#include <zephyr/sys/util.h>
#include <errno.h>
#include <string.h>

#include <EGL/eglplatform.h>

#include "yakogl_board.h"
#include "imx8mp_display.h"
#include "board_imx8mp_route3.h"
#include "gc_panel_console.h"
#include "yakogl_record.h"

/* ---- the M7's map (the M7 firmware's m7_mem.h) ---------------- */

#define M7_CARVEOUT_BASE 0x80000000u
#define M7_UNCACHED_BASE 0x81000000u
#define M7_UNCACHED_SIZE 0x8000u
#define M7_STATUS_ADDR   0x81000000u
#define M7_STATUS_MAGIC  0x37464153u /* 'SAF7' */
#define M7_RING_ADDR     0x81005000u
#define M7_FB_APP_ADDR   0x81408000u
#define M7_FB_STRIDE     0x400000u
/* The A53's buffers: the first above the M7's scan-out pair, one stride
 * apart (fb_app is the M7's stand-in and never ours; ring_a53.c). */
#define RING_FB0_ADDR    (M7_FB_APP_ADDR + 3u * M7_FB_STRIDE) /* 0x82008000 */
#define RING_FB_MAX      4u
#define RING_W           1280u
#define RING_H           800u
#define RING_FORMAT_XRGB8888 0x7u
/* What an M7 segment may be loaded into: its code, data, status and console,
 * below fb_app (nothing of ours is there). */
#define M7_LOAD_LIMIT    M7_FB_APP_ADDR

/* struct m7_status word offsets (append-only; route3_boot.sh pins 19, 29, 46) */
#define ST_MAGIC       0
#define ST_VERSION     1
#define ST_FB_APP      9
#define ST_FB_W        12
#define ST_FB_H        13
#define ST_LCDIF_FLAGS 19
#define ST_RING_ADDR   29
#define ST_RING_VERDICT 31
#define ST_BUILD_ID    46
#define ST_VERSION_WITH_PAGE 10u

/* struct m7_control (at 0x8100_0800), word offsets, append-only: the
 * session handover is word 12, after `inplace` (the M7 firmware's m7_mem.h). */
#define M7_CTRL_ADDR         0x81000800u
#define CTRL_A53_SESSION     12

/* ---- the release (the board's BL31, lf-6.6.52-2.2.1, disassembled) -------- */

#define IMX_SIP_RPROC        0xC2000005u /* IMX_SIP_SRC in TF-A */
#define IMX_SIP_RPROC_START  0x0u        /* clears GPR22 CPUWAIT, returns 0 */
#define IMX_SIP_RPROC_STARTED 0x1u       /* returns !(GPR22 & 1) */
#define IMX_SIP_RPROC_STOP   0x2u
#define IOMUXC_GPR_BASE      0x30340000u
#define IOMUXC_GPR22         0x58u
#define GPR22_CM7_CPUWAIT    BIT(0)
#define CCM_BASE             0x30380000u
#define CCM_TARGET_ROOT_M7   0x8080u     /* core slice 1: m7_core */
#define CCM_TARGET_ROOT_EN   BIT(28)

/* The M7's TCMs as the A53 sees them (imx_rproc_att_imx8mn). */
#define M7_ITCM_DEV  0x00000000u
#define M7_ITCM_SYS  0x007E0000u
#define M7_DTCM_DEV  0x20000000u
#define M7_DTCM_SYS  0x00800000u
#define M7_TCM_SIZE  0x00020000u

#define WAIT_POLL_MS       1
#define SHOWN_TIMEOUT_MS   200

/* The stripped M7 ELF (m7_image.py), linked in by m7_blob.S. */
extern const uint8_t yakogl_m7_image[];
extern const uint8_t yakogl_m7_image_end[];

static struct {
	int state;
	uint8_t *window;                    /* the uncached window, NC */
	const volatile uint32_t *status;
	volatile struct cluster_frame_ring *ring;
	volatile struct cluster_m7_page *page;
	uint8_t *fb_cpu;                    /* the A53 buffers, NC */
	uint32_t entry;
	uint32_t shown_timeouts;
	bool lit;                           /* imx8mp_display_backlight_on() called */
	int64_t released_ms;
	int64_t first_shown_ms;
	struct k_timer beat;
	volatile struct cluster_boot_timeline *tl; /* in the window, once mapped */
} R;

/* ---- the cold-start timeline (<cluster/boot_timeline.h>) -------------------- */

static uint64_t tl_early[CLUSTER_BOOT_TIMELINE_SLOTS];

static inline uint64_t cntpct(void)
{
	uint64_t v;

	__asm__ volatile("isb; mrs %0, cntpct_el0" : "=r"(v) : : "memory");
	return v;
}

static inline uint32_t cntfrq(void)
{
	uint64_t v;

	__asm__ volatile("mrs %0, cntfrq_el0" : "=r"(v));
	return (uint32_t)v;
}

void board_route3_tl_mark(enum cluster_tl_a53 stage)
{
	if ((unsigned int)stage >= CLUSTER_BOOT_TIMELINE_SLOTS || tl_early[stage] != 0u) {
		return;
	}
	tl_early[stage] = cntpct();
	if (R.tl != NULL) {
		R.tl->a53[stage] = tl_early[stage];
	}
}

/* The page for this boot: cleared, the A53's stamps so far, the magic last
 * (the M7 writes only m7[], and only after its release). */
static void tl_attach(uint8_t *window)
{
	volatile struct cluster_boot_timeline *tl =
		(volatile struct cluster_boot_timeline *)(window + (CLUSTER_BOOT_TIMELINE_ADDR -
								    M7_UNCACHED_BASE));
	volatile uint32_t *w = (volatile uint32_t *)tl;

	for (size_t i = 0; i < sizeof(*tl) / 4u; i++) {
		w[i] = 0u;
	}
	tl->version = CLUSTER_BOOT_TIMELINE_VERSION;
	tl->cntfrq = cntfrq();
	for (unsigned int i = 0; i < CLUSTER_BOOT_TIMELINE_SLOTS; i++) {
		tl->a53[i] = tl_early[i];
	}
	barrier_dmem_fence_full();
	tl->magic = CLUSTER_BOOT_TIMELINE_MAGIC;
	R.tl = tl;
}

/* The application's progress marks (board_progress), stage n of
 * YK_RECORD_WHERE_STAGE | n: the Slint sample's window, EGL, frame 1 and
 * swapped marks (examples/slint/src/main.c). */
void board_progress_hook(uint32_t where)
{
	static const enum cluster_tl_a53 at[] = {
		CLUSTER_TL_A53_APP_WINDOW, CLUSTER_TL_A53_APP_EGL, CLUSTER_TL_A53_APP_FRAME1,
		CLUSTER_TL_A53_APP_SWAPPED};
	const uint32_t n = where & ~YK_RECORD_WHERE_STAGE;

	if ((where & YK_RECORD_WHERE_STAGE) != 0u && n >= 1u && n <= ARRAY_SIZE(at)) {
		board_route3_tl_mark(at[n - 1u]);
	}
}

static int tl_entry(void)
{
	board_route3_tl_mark(CLUSTER_TL_A53_ENTRY);
	return 0;
}
SYS_INIT(tl_entry, PRE_KERNEL_1, 0);

static int tl_kernel_up(void)
{
	board_route3_tl_mark(CLUSTER_TL_A53_KERNEL_UP);
	return 0;
}
SYS_INIT(tl_kernel_up, APPLICATION, 0);

static void *map_nc(uintptr_t phys, size_t size)
{
	uint8_t *cpu = NULL;

#ifdef K_MEM_ARM_NORMAL_NC
	k_mem_map_phys_bare(&cpu, phys, size, K_MEM_ARM_NORMAL_NC | K_MEM_PERM_RW);
#else
	k_mem_map_phys_bare(&cpu, phys, size, K_MEM_CACHE_WB | K_MEM_PERM_RW);
#endif
	if (cpu == NULL) {
		printk("route3: mapping 0x%lx+%zu failed (KERNEL_VM_SIZE?)\n", (unsigned long)phys,
		       size);
	}
	return cpu;
}

/*
 * The second-stage loader's own stamps (boot_stage2/src/main.c, "the
 * loader's own timeline"): a page of the loader's RAM, dead once this image
 * runs. Copied into the timeline's LOADER slots, then the magic cleared so a
 * later boot by another route does not read this one's.
 */
#define BS2_STAMP_ADDR  0x41000000u
#define BS2_STAMP_MAGIC 0x54325342u /* 'BS2T' */
#define BS2_STAMPS      8u

static void tl_loader(void)
{
	static const enum cluster_tl_a53 at[] = {
		CLUSTER_TL_A53_LOADER_ENTRY,     CLUSTER_TL_A53_LOADER_MAIN,
		CLUSTER_TL_A53_LOADER_READ,      CLUSTER_TL_A53_LOADER_UNPACKED,
		CLUSTER_TL_A53_LOADER_JUMP,      CLUSTER_TL_A53_LOADER_STORED_OK,
		CLUSTER_TL_A53_LOADER_DECODED,   CLUSTER_TL_A53_LOADER_ZEROED};
	volatile uint32_t *w = map_nc(BS2_STAMP_ADDR, CONFIG_MMU_PAGE_SIZE);

	if (w == NULL || R.tl == NULL) {
		return;
	}
	if (w[0] == BS2_STAMP_MAGIC && w[1] == BS2_STAMPS) {
		volatile uint64_t *ts = (volatile uint64_t *)(w + 2);

		for (unsigned int i = 0; i < ARRAY_SIZE(at); i++) {
			tl_early[at[i]] = ts[i];
			R.tl->a53[at[i]] = ts[i];
		}
		w[0] = 0u;
	}
	k_mem_unmap_phys_bare((uint8_t *)w, CONFIG_MMU_PAGE_SIZE);
}

static uintptr_t map_dev(uintptr_t phys, size_t size)
{
	mm_reg_t va = 0;

	device_map(&va, phys, size, K_MEM_CACHE_NONE);
	return (uintptr_t)va;
}

/* ---- loading -------------------------------------------------------------- */

struct elf32_ehdr {
	uint8_t ident[16];
	uint16_t type, machine;
	uint32_t version, entry, phoff, shoff, flags;
	uint16_t ehsize, phentsize, phnum, shentsize, shnum, shstrndx;
};
struct elf32_phdr {
	uint32_t type, offset, vaddr, paddr, filesz, memsz, flags, align;
};
BUILD_ASSERT(sizeof(struct elf32_ehdr) == 52 && sizeof(struct elf32_phdr) == 32);

/* One segment into a TCM, through a device mapping and 32-bit stores (the
 * TCMs are ECC-protected: no partial-word writes). */
static int load_tcm(uint32_t sys, uint32_t off, const uint8_t *src, uint32_t filesz,
		    uint32_t memsz)
{
	uintptr_t va = map_dev(sys, M7_TCM_SIZE);
	uint32_t words = DIV_ROUND_UP(memsz, 4u);

	if (va == 0u) {
		return -ENOMEM;
	}
	for (uint32_t i = 0; i < words; i++) {
		uint32_t v = 0u;

		for (uint32_t b = 0; b < 4u; b++) {
			uint32_t at = i * 4u + b;

			if (at < filesz) {
				v |= (uint32_t)src[at] << (8u * b);
			}
		}
		sys_write32(v, va + off + i * 4u);
	}
	return 0;
}

static int load_ddr(uint32_t paddr, const uint8_t *src, uint32_t filesz, uint32_t memsz)
{
	uintptr_t base = ROUND_DOWN(paddr, CONFIG_MMU_PAGE_SIZE);
	size_t len = ROUND_UP((size_t)paddr + memsz, CONFIG_MMU_PAGE_SIZE) - base;
	uint8_t *cpu = map_nc(base, len);

	if (cpu == NULL) {
		return -ENOMEM;
	}
	memcpy(cpu + (paddr - base), src, filesz);
	memset(cpu + (paddr - base) + filesz, 0, memsz - filesz);
	k_mem_unmap_phys_bare(cpu, len);
	return 0;
}

static int m7_load(void)
{
	const uint8_t *img = yakogl_m7_image;
	size_t size = (size_t)(yakogl_m7_image_end - yakogl_m7_image);
	struct elf32_ehdr eh;
	unsigned int loads = 0;

	if (size < sizeof(eh)) {
		return -EINVAL;
	}
	memcpy(&eh, img, sizeof(eh));
	if (memcmp(eh.ident, "\177ELF\001\001", 6) != 0 || eh.machine != 40u ||
	    eh.phentsize != sizeof(struct elf32_phdr) ||
	    (size_t)eh.phoff + (size_t)eh.phnum * sizeof(struct elf32_phdr) > size) {
		printk("route3: the M7 image is not an ARM ELF32\n");
		return -EINVAL;
	}
	for (unsigned int i = 0; i < eh.phnum; i++) {
		struct elf32_phdr ph;
		int rc;

		memcpy(&ph, img + eh.phoff + i * sizeof(ph), sizeof(ph));
		if (ph.type != 1u || ph.memsz == 0u) {
			continue;
		}
		if (ph.filesz > ph.memsz || (size_t)ph.offset + ph.filesz > size) {
			return -EINVAL;
		}
		if (ph.paddr >= M7_ITCM_DEV && ph.paddr + ph.memsz <= M7_ITCM_DEV + M7_TCM_SIZE) {
			rc = load_tcm(M7_ITCM_SYS, ph.paddr - M7_ITCM_DEV, img + ph.offset, ph.filesz,
				      ph.memsz);
		} else if (ph.paddr >= M7_DTCM_DEV &&
			   ph.paddr + ph.memsz <= M7_DTCM_DEV + M7_TCM_SIZE) {
			rc = load_tcm(M7_DTCM_SYS, ph.paddr - M7_DTCM_DEV, img + ph.offset, ph.filesz,
				      ph.memsz);
		} else if (ph.paddr >= M7_CARVEOUT_BASE &&
			   (uint64_t)ph.paddr + ph.memsz <= M7_LOAD_LIMIT) {
			rc = load_ddr(ph.paddr, img + ph.offset, ph.filesz, ph.memsz);
		} else {
			printk("route3: M7 segment at 0x%08x+0x%x is outside its TCMs and code "
			       "region\n", ph.paddr, ph.memsz);
			return -EINVAL;
		}
		if (rc != 0) {
			return rc;
		}
		loads++;
	}
	R.entry = eh.entry;
	printk("route3: M7 image loaded: %u segments, entry 0x%08x, %zu bytes\n", loads, eh.entry,
	       size);
	return loads != 0u ? 0 : -EINVAL;
}

/* ---- the release -------------------------------------------------------- */

static uint64_t sip_rproc(uint64_t op)
{
	register uint64_t x0 __asm__("x0") = IMX_SIP_RPROC;
	register uint64_t x1 __asm__("x1") = op;

	__asm__ volatile("smc #0"
			 : "+r"(x0), "+r"(x1)
			 :
			 : "x2", "x3", "x4", "x5", "x6", "x7", "x8", "x9", "x10", "x11", "x12",
			   "x13", "x14", "x15", "x16", "x17", "memory");
	return x0;
}

static void m7_release(void)
{
	uintptr_t ccm = map_dev(CCM_BASE, 0x10000);
	uint64_t rc;

	/* Linux enables the m7_core clock before its SMC; TF-A does not touch
	 * it. The M7's own SoC init moves it to SYS_PLL1 800 MHz. */
	if (ccm != 0u) {
		uint32_t root = sys_read32(ccm + CCM_TARGET_ROOT_M7);

		if ((root & CCM_TARGET_ROOT_EN) == 0u) {
			sys_write32(root | CCM_TARGET_ROOT_EN, ccm + CCM_TARGET_ROOT_M7);
			printk("route3: the M7 core clock root was off (0x%08x); enabled\n", root);
		}
	}
	/* every image byte in DDR and the TCMs before the core runs */
	barrier_dsync_fence_full();
	rc = sip_rproc(IMX_SIP_RPROC_START);
	if (rc != 0u) {
		uintptr_t gpr = map_dev(IOMUXC_GPR_BASE, 0x1000);

		printk("route3: SiP M7 start returned 0x%llx; clearing GPR22 CPUWAIT\n",
		       (unsigned long long)rc);
		if (gpr != 0u) {
			sys_write32(sys_read32(gpr + IOMUXC_GPR22) & ~GPR22_CM7_CPUWAIT,
				    gpr + IOMUXC_GPR22);
		}
	}
	R.released_ms = k_uptime_get();
	R.state = BOARD_ROUTE3_STARTING;
	printk("route3: M7 released (started=%llu)\n",
	       (unsigned long long)sip_rproc(IMX_SIP_RPROC_STARTED));
}

/* The M7 has written its status block: check its map against ours once. */
static void m7_check(void)
{
	const volatile uint32_t *s = R.status;

	if (R.state != BOARD_ROUTE3_STARTING || s[ST_MAGIC] != M7_STATUS_MAGIC) {
		return;
	}
	if (s[ST_VERSION] < ST_VERSION_WITH_PAGE || s[ST_FB_APP] != M7_FB_APP_ADDR ||
	    s[ST_RING_ADDR] != M7_RING_ADDR || s[ST_FB_W] != RING_W || s[ST_FB_H] != RING_H) {
		printk("route3: the M7 runs another map: status v%u fb_app 0x%08x ring 0x%08x "
		       "%ux%u; publishing stops\n",
		       s[ST_VERSION], s[ST_FB_APP], s[ST_RING_ADDR], s[ST_FB_W], s[ST_FB_H]);
		R.ring->magic = 0u;
		R.state = BOARD_ROUTE3_MISMATCH;
		return;
	}
	R.state = BOARD_ROUTE3_UP;
	board_route3_tl_mark(CLUSTER_TL_A53_M7_UP);
	printk("route3: M7 up at +%lld ms: build 0x%08x, status v%u, lcdif_flags 0x%x\n",
	       k_uptime_get() - R.released_ms, s[ST_BUILD_ID], s[ST_VERSION], s[ST_LCDIF_FLAGS]);
}

static void m7_give_up(void)
{
	k_timer_stop(&R.beat);
	R.ring->magic = 0u;
	(void)sip_rproc(IMX_SIP_RPROC_STOP);
	R.state = BOARD_ROUTE3_ABSENT;
	/* nobody lit the panel: this image shows its own frames from now on */
	imx8mp_display_backlight_on();
	R.lit = true;
	printk("route3: no M7 status block after %d ms: stopped it, this image flips\n",
	       CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_M7_WAIT_MS);
}

#if CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_TIMELINE_REPORT_MS > 0
/*
 * The report also goes out on the console UART, written here character by
 * character: the image keeps the UART console off so that nothing on the
 * way to the first picture waits for a serial line, and this runs after it.
 * The panel console shows the same lines. One machine-readable line per
 * stage: "TL <core> <slot> <ticks> <name>".
 */
#if DT_NODE_HAS_STATUS(DT_ALIAS(yakogl_timeline_uart), okay)
static const struct device *const tl_uart = DEVICE_DT_GET(DT_ALIAS(yakogl_timeline_uart));
#else
static const struct device *const tl_uart = DEVICE_DT_GET(DT_CHOSEN(zephyr_console));
#endif

static void tl_uart_str(const char *s)
{
	if (!device_is_ready(tl_uart)) {
		return;
	}
	for (; *s != '\0'; s++) {
		if (*s == '\n') {
			uart_poll_out(tl_uart, '\r');
		}
		uart_poll_out(tl_uart, *s);
	}
}

static void tl_uart_line(const char *who, const char *what, uint64_t t)
{
	char buf[96];

	snprintk(buf, sizeof(buf), "TL %s %llu %s\n", who, (unsigned long long)t, what);
	tl_uart_str(buf);
}

static void tl_line(const char *who, const char *what, uint64_t t, uint64_t t0, uint32_t hz)
{
	if (t == 0u) {
		printk("  %s %-22s      -\n", who, what);
		return;
	}
	/* microseconds since SPL started the counter, and since the A53's entry */
	const uint64_t abs_us = t * 1000000u / hz;
	const int64_t rel_us = t0 != 0u ? (int64_t)((int64_t)t - (int64_t)t0) * 1000000 / hz : 0;

	printk("  %s %-22s %6llu.%03llu ms  (+%lld.%03lld)\n", who, what, abs_us / 1000u,
	       abs_us % 1000u, rel_us / 1000, (rel_us < 0 ? -rel_us : rel_us) % 1000);
}

/*
 * The measurement build's ending: stop the M7, put the panel console's own
 * buffer on the panel, print both cores' stamps, and stay there. The
 * panel shows the table; the same stamps go out on the UART (below).
 */
static void tl_report(void)
{
	static const char *const a53[] = {
		"entry", "panel bring-up start", "panel on (dark)", "kernel up",
		"window (M7 loaded)", "first board_show", "M7 released", "M7 up",
		"first shown", "app: windows", "app: EGL", "app: frame 1",
		"app: swapped", "loader entry", "loader main", "loader payload in RAM",
		"loader unpacked", "loader jump", "loader stored CRC ok", "loader decoded",
		"loader .bss zeroed"};
	static const char *const m7[] = {
		"main", "safety layer ready", "loop", "first A53 frame",
		"first composite", "first flip latched", "backlight on", "layer pre-rendered", "frame 1 layer rendered"};
	volatile struct cluster_boot_timeline *tl = R.tl;
	struct gc_panel_surface s;
	uint32_t hz = cntfrq();
	uint64_t t0;

	k_timer_stop(&R.beat);
	(void)sip_rproc(IMX_SIP_RPROC_STOP);
	if (gc_panel_console_surface(&s)) {
		(void)imx8mp_display_set_fb(k_mem_phys_addr(s.fb));
	}
	imx8mp_display_backlight_on();
	if (tl == NULL || tl->magic != CLUSTER_BOOT_TIMELINE_MAGIC) {
		printk("route3 timeline: no page\n");
	} else {
		hz = tl->cntfrq != 0u ? tl->cntfrq : hz;
		t0 = tl->a53[CLUSTER_TL_A53_ENTRY];

		printk("\nroute3 timeline (system counter %u Hz, ms since SPL started it; +ms since "
		       "the A53 entry)\n", hz);
		for (unsigned int i = 0; i < ARRAY_SIZE(a53); i++) {
			tl_line("A53", a53[i], tl->a53[i], t0, hz);
		}
		for (unsigned int i = 0; i < ARRAY_SIZE(m7); i++) {
			tl_line("M7 ", m7[i], tl->m7[i], t0, hz);
		}
		/* The UART last: the panel already holds the record if the
		 * serial line is not there. */
		{
			char buf[64];

			snprintk(buf, sizeof(buf), "\nTL begin hz=%u\n", hz);
			tl_uart_str(buf);
		}
		for (unsigned int i = 0; i < ARRAY_SIZE(a53); i++) {
			tl_uart_line("A53", a53[i], tl->a53[i]);
		}
		for (unsigned int i = 0; i < ARRAY_SIZE(m7); i++) {
			tl_uart_line("M7", m7[i], tl->m7[i]);
		}
		tl_uart_str("TL end\n");
	}
	for (;;) {
		board_watchdog_feed();
		k_sleep(K_MSEC(100));
	}
}
#endif

/* ---- the ring ------------------------------------------------------------- */

static void beat_expiry(struct k_timer *t)
{
	ARG_UNUSED(t);
	/* one increment, no printk (interrupt context, no FPU on arm64) */
	cluster_frame_ring_heartbeat(R.ring);
}

/* This boot's ring session (frame_ring.h, `session`): any non-zero value
 * the last boot did not use -- the counter's low word is as good as any. */
static uint32_t new_session(uint32_t stale)
{
	uint32_t s = (uint32_t)cntpct() ^ (uint32_t)(cntpct() >> 32);

	if (s == 0u || s == stale) {
		s = stale + 1u;
	}
	return s != 0u ? s : 1u;
}

/*
 * Hand the M7 this boot's ring session (it then believes the ring on its first
 * look instead of waiting for the beat to move), beat once so the ring reads
 * alive at that look, start the heartbeat, and release the core.
 */
static void m7_start(void)
{
	volatile uint32_t *ctrl =
		(volatile uint32_t *)(R.window + (M7_CTRL_ADDR - M7_UNCACHED_BASE));

	ctrl[CTRL_A53_SESSION] = R.ring->session;
	cluster_frame_ring_heartbeat(R.ring);
	k_timer_start(&R.beat, K_MSEC(CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_BEAT_MS),
		      K_MSEC(CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_BEAT_MS));
	m7_release();
	board_route3_tl_mark(CLUSTER_TL_A53_M7_RELEASED);
}

static int route3_setup(void)
{
	uint32_t stale_session;

	int rc;

	R.window = map_nc(M7_UNCACHED_BASE, M7_UNCACHED_SIZE);
	R.fb_cpu = map_nc(RING_FB0_ADDR, (size_t)RING_FB_MAX * M7_FB_STRIDE);
	if (R.window == NULL || R.fb_cpu == NULL) {
		return -ENOMEM;
	}
	R.status = (const volatile uint32_t *)(R.window + (M7_STATUS_ADDR - M7_UNCACHED_BASE));
	R.ring = (volatile struct cluster_frame_ring *)(R.window + (M7_RING_ADDR - M7_UNCACHED_BASE));
	R.page = cluster_m7_page_of(R.ring);
	BUILD_ASSERT(CLUSTER_BOOT_TIMELINE_ADDR + sizeof(struct cluster_boot_timeline) <=
			     M7_UNCACHED_BASE + M7_UNCACHED_SIZE,
		     "the timeline page is in the M7's non-cacheable window");
	tl_attach(R.window);
	tl_loader();

	/* DDR keeps its contents over a warm reset: what the last boot's M7
	 * left must not read as this boot's. Its status magic and its page
	 * go; the M7 writes both again at its start. */
	((volatile uint32_t *)R.status)[ST_MAGIC] = 0u;
	R.page->magic = 0u;
	stale_session = R.ring->session;
	R.ring->magic = 0u;

	rc = m7_load();
	if (rc != 0) {
		printk("route3: the M7 image did not load: %d\n", rc);
		return -EIO;
	}
	cluster_frame_ring_init(R.ring);
	R.ring->session = new_session(stale_session);
	k_timer_init(&R.beat, beat_expiry, NULL);
	R.state = BOARD_ROUTE3_LOADED;
	board_route3_tl_mark(CLUSTER_TL_A53_WINDOW);

	/*
	 * The M7 now, not after frame 1: its start, its first render of the
	 * safety layer and its wait for our frame then run while this core
	 * initialises the GPU and draws -- 2026-10-29's timeline had the M7
	 * started 463 ms after it was loaded, and its ~290 ms to the first
	 * flip all on the critical path. Safe once the panel chain is
	 * running (the M7's first tick reads the LCDIF, a bus fault while
	 * MEDIAMIX is off): the panel console's cold bring-up has done that
	 * at POST_KERNEL. Otherwise the first show brings the panel up and
	 * releases it, as before.
	 */
	if (imx8mp_display_is_running()) {
		m7_start();
	}
	return 0;
}

int board_route3_window(unsigned int n, uint32_t w, uint32_t h, struct board_window *out)
{
	if (R.state == BOARD_ROUTE3_OFF) {
		int rc = route3_setup();

		if (rc != 0) {
			R.state = BOARD_ROUTE3_ABSENT;
			return rc;
		}
	}
	if (w != RING_W || h < RING_H || (size_t)h * w * 4u > M7_FB_STRIDE) {
		printk("route3: a %ux%u window; the M7 composites 1280x800 only\n", w, h);
		return -EINVAL;
	}
	if (n >= MIN(RING_FB_MAX, (unsigned int)CONFIG_YAKOGL_BOARD_IMX8MP_WINDOWS)) {
		return -ENOMEM;
	}
	*out = (struct board_window){
		.phys = RING_FB0_ADDR + n * M7_FB_STRIDE,
		.cpu = R.fb_cpu + (size_t)n * M7_FB_STRIDE,
		.w = w,
		.h = h,
		.stride = w * 4u,
		.format = GLES_NATIVE_FORMAT_XRGB8888,
	};
	printk("route3: window %u is ring buffer 0x%08lx\n", n, (unsigned long)out->phys);
	return 0;
}

static uint32_t publish(const struct board_window *win)
{
	const struct cluster_frame f = {
		.pa = win->phys,
		.width = RING_W,
		.height = RING_H,
		.stride_bytes = win->stride,
		.format = RING_FORMAT_XRGB8888,
		.app_alive = 1u,
	};

	return cluster_frame_ring_publish(R.ring, &f);
}

int board_route3_show(const struct board_window *win)
{
	uint32_t seq;
	int64_t t0;
	int rc;

	board_route3_tl_mark(CLUSTER_TL_A53_FRAME1_DRAWN); /* the first show only */
	switch (R.state) {
	case BOARD_ROUTE3_LOADED:
		/* The panel from cold on this frame; the M7 adopts it later. The
		 * backlight stays off (YAKOGL_DRV_DISPLAY_IMX8MP_BACKLIGHT_ON_FIRST_SHOW):
		 * this frame has no safety layer, and the M7 turns PWM3 on itself
		 * when it adopts the LCDIF (lcdif_m7_backlight_ensure_on), right
		 * before its first flip of a frame it has composited. Lit here,
		 * the A53's bare frame stood on the panel for the whole M7 boot
		 * (2026-10-22, the first frames wrong). */
		rc = imx8mp_display_on(win->phys, win->stride);
		if (rc != 0) {
			return rc;
		}
		(void)publish(win);
		m7_start();
		return 0;
	case BOARD_ROUTE3_STARTING:
	case BOARD_ROUTE3_UP:
		break;
	case BOARD_ROUTE3_MISMATCH:
		return 0; /* the M7 owns the panel; nothing of ours goes there */
	default:
		return -EAGAIN;
	}

	seq = publish(win);
	t0 = k_uptime_get();
	/* The previous window holds frame seq - 1: it is ours again once the
	 * M7's `shown` has moved past it (frame_ring.h). Until the M7 is up,
	 * the wait is for the M7 itself, bounded by the M7 wait. */
	while (cluster_frame_ring_shown(R.ring) < seq) {
		m7_check();
		if (R.state == BOARD_ROUTE3_STARTING &&
		    k_uptime_get() - R.released_ms >= CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_M7_WAIT_MS) {
			m7_give_up();
			return -EAGAIN;
		}
		if (R.state == BOARD_ROUTE3_MISMATCH) {
			return 0;
		}
		if (R.state == BOARD_ROUTE3_UP && k_uptime_get() - t0 >= SHOWN_TIMEOUT_MS) {
			if (R.shown_timeouts++ == 0u) {
				printk("route3: frame %u not shown after %d ms (verdict %u); going on\n",
				       seq, SHOWN_TIMEOUT_MS, R.status[ST_RING_VERDICT]);
			}
			break;
		}
		board_watchdog_feed();
		k_sleep(K_MSEC(WAIT_POLL_MS));
	}
	m7_check();
	if (!R.lit && R.state == BOARD_ROUTE3_UP && cluster_frame_ring_shown(R.ring) != 0u) {
		/* The M7 has shown a frame, so it has lit the panel. Only the
		 * display driver's 5 s fallback is left to cancel: PWM3 is on, so
		 * this writes nothing. */
		imx8mp_display_backlight_on();
		R.lit = true;
		R.first_shown_ms = k_uptime_get();
		board_route3_tl_mark(CLUSTER_TL_A53_FIRST_SHOWN);
		printk("route3: the M7's first frame on the panel within %lld ms of the release\n",
		       k_uptime_get() - R.released_ms);
	}
#if CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_TIMELINE_REPORT_MS > 0
	if (R.lit && R.state == BOARD_ROUTE3_UP &&
	    k_uptime_get() - R.first_shown_ms >= CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_TIMELINE_REPORT_MS) {
		tl_report(); /* does not return */
	}
#endif
	return 0;
}

int board_route3_state(void)
{
	return R.state;
}

bool board_route3_vehicle(struct cluster_vehicle_state *out)
{
	if (R.page == NULL || R.state < BOARD_ROUTE3_STARTING || R.state == BOARD_ROUTE3_ABSENT ||
	    !cluster_m7_page_ok(R.page)) {
		return false;
	}
	return cluster_vehicle_read(R.page, out);
}
