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
#ifdef CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_A53_RESTART
#include <cluster/a53_power.h>
#else
BUILD_ASSERT(!DT_PROP_OR(DT_NODELABEL(display0), zephyr_deferred_init, 0),
	     "a53-restart.overlay without CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_A53_RESTART: "
	     "nothing would initialise the display chain");
#endif

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
#define ST_LCDIF_FB    23 /* CTRLDESCL_LOW0_4 as the M7 read it back */
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
	atomic_t m7_started; /* m7_start() ran (it runs once) */
	bool panel_checked;  /* the first show made sure the panel runs */
	struct k_timer beat;
	volatile struct cluster_boot_timeline *tl; /* in the window, once mapped */
#ifdef CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_A53_RESTART
	bool going_down;       /* DOWN taken: nothing more is published */
	bool restart_published; /* a restart's first frame is out */
	unsigned int buf_base; /* window n is ring buffer buf_base + n */
#endif
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
	/* the Slint sample's stages 1..9 (examples/slint/src/main.c,
	 * enum stage); 5..7 (the check, the animation) are not stamped */
	static const int at[] = {
		CLUSTER_TL_A53_APP_WINDOW, CLUSTER_TL_A53_APP_EGL, CLUSTER_TL_A53_APP_FRAME1,
		CLUSTER_TL_A53_APP_SWAPPED, -1, -1, -1, CLUSTER_TL_A53_APP_BUILT,
		CLUSTER_TL_A53_APP_RENDERED};
	const uint32_t n = where & ~YK_RECORD_WHERE_STAGE;

	if ((where & YK_RECORD_WHERE_STAGE) != 0u && n >= 1u && n <= ARRAY_SIZE(at) &&
	    at[n - 1u] >= 0) {
		board_route3_tl_mark((enum cluster_tl_a53)at[n - 1u]);
	}
}

static int tl_entry(void)
{
	board_route3_tl_mark(CLUSTER_TL_A53_ENTRY);
#ifdef CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_A53_RESTART
	/* The first board hook: a restart is known before anything else of
	 * the board's runs (the WDOG1 arming at 1, the early clocks at 5, the
	 * panel console at CONSOLE_INIT_PRIORITY, the devices). */
	board_route3_restart_detect(tl_early[CLUSTER_TL_A53_ENTRY]);
#endif
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

#if CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_TIMELINE_REPORT_MS > 0 || \
	defined(CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_A53_RESTART)
/*
 * The report goes out on a UART, written here character by character: the
 * image keeps the UART console off so that nothing on the way to the first
 * picture waits for a serial line, and this runs long after it.
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
#endif

/*
 * A line of the application's onto the timeline UART, off its thread: the
 * polled UART takes about 10 ms a line, so it is written from the system work
 * queue. A line that comes while the last is still going out is dropped.
 */
#if CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_TIMELINE_REPORT_MS > 0 || \
	defined(CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_A53_RESTART)
static char app_line[160];
static atomic_t app_line_busy;

static void app_line_work_fn(struct k_work *work)
{
	ARG_UNUSED(work);
	tl_uart_str(app_line);
	atomic_set(&app_line_busy, 0);
}
static K_WORK_DEFINE(app_line_work, app_line_work_fn);

void board_route3_report_line(const char *line)
{
	if (atomic_set(&app_line_busy, 1) != 0) {
		return;
	}
	strncpy(app_line, line, sizeof(app_line) - 1u);
	app_line[sizeof(app_line) - 1u] = '\0';
	k_work_submit(&app_line_work);
}
#endif

#if CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_TIMELINE_REPORT_MS > 0
static void tl_uart_line(const char *who, const char *what, uint64_t t)
{
	char buf[96];

	snprintk(buf, sizeof(buf), "TL %s %llu %s\n", who, (unsigned long long)t, what);
	tl_uart_str(buf);
}

/*
 * The colour of the frames as this core drew them: channel sums over every
 * 16th pixel of every 16th row of the first three shows, before they are
 * published (the M7 blends into the same buffer after). The first picture
 * looked red/blue swapped on a slow-motion video (2026-10-29); this says
 * whether frame 1 left this core that way.
 */
static uint64_t tl_px[3][3];
static uint32_t tl_desc5_first; /* LCDIF CTRLDESCL0_5 at the first show */

/*
 * The measurement report: both cores' stamps on the timeline UART, one
 * line per stage, "TL <core> <ticks> <name>" between "TL begin hz=<Hz>"
 * and "TL end" (read on the laptop, /var/tmp/yk-spawn/tl-table.py). The
 * cluster keeps running: the report is a work item on the system work
 * queue, so the polled UART (about 90 ms for the whole table) never holds
 * a frame, and the panel is never taken from the M7.
 */
static void tl_report(struct k_work *work)
{
	static const char *const a53[] = {
		"entry", "panel bring-up start", "panel on (dark)", "kernel up",
		"window (M7 loaded)", "first board_show", "M7 released", "M7 up",
		"first shown", "app: windows", "app: EGL", "app: frame 1",
		"app: swapped", "loader entry", "loader main", "loader payload in RAM",
		"loader unpacked", "loader jump", "loader stored CRC ok", "loader decoded",
		"loader .bss zeroed", "app: built", "app: rendered (CPU)"};
	static const char *const m7[] = {
		"main", "safety layer ready", "loop", "first A53 frame",
		"first composite", "first flip latched", "backlight on", "layer pre-rendered", "frame 1 layer rendered"};
	volatile struct cluster_boot_timeline *tl = R.tl;
	char buf[64];

	ARG_UNUSED(work);
	if (tl == NULL || tl->magic != CLUSTER_BOOT_TIMELINE_MAGIC) {
		tl_uart_str("\nTL none: no timeline page\n");
		return;
	}
	snprintk(buf, sizeof(buf), "\nTL begin hz=%u\n", tl->cntfrq != 0u ? tl->cntfrq : cntfrq());
	tl_uart_str(buf);
	for (unsigned int i = 0; i < ARRAY_SIZE(a53); i++) {
		tl_uart_line("A53", a53[i], tl->a53[i]);
	}
	for (unsigned int i = 0; i < ARRAY_SIZE(m7); i++) {
		tl_uart_line("M7", m7[i], tl->m7[i]);
	}
	for (unsigned int n = 0; n < 3u; n++) {
		snprintk(buf, sizeof(buf), "TL px %u r %llu g %llu b %llu\n", n + 1u,
			 (unsigned long long)tl_px[n][0], (unsigned long long)tl_px[n][1],
			 (unsigned long long)tl_px[n][2]);
		tl_uart_str(buf);
	}
	snprintk(buf, sizeof(buf), "TL lcdif desc5 first 0x%08x now 0x%08x\n", tl_desc5_first,
		 imx8mp_display_layer_control());
	tl_uart_str(buf);
	tl_uart_str("TL end\n");
}

static K_WORK_DEFINE(tl_report_work, tl_report);

static void tl_sample(const struct board_window *win, unsigned int n)
{
	const volatile uint8_t *base = (const volatile uint8_t *)win->cpu;

	for (uint32_t y = 0; y < win->h; y += 16u) {
		const volatile uint32_t *row = (const volatile uint32_t *)(base + (size_t)y * win->stride);

		for (uint32_t x = 0; x < win->w; x += 16u) {
			const uint32_t v = row[x];

			tl_px[n][0] += (v >> 16) & 0xffu;
			tl_px[n][1] += (v >> 8) & 0xffu;
			tl_px[n][2] += v & 0xffu;
		}
	}
}
#endif

#ifdef CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_A53_RESTART
/*
 * The restart's report, once, on the timeline UART (tl_uart_str): the power
 * page's stamps from the M7's DOWN request to this image's first frame, as
 * milliseconds after the request and after the stamp before, between
 * "TLR begin" and "TLR end". A stamp nobody wrote reads "-". From the system
 * work queue, a while after the first frame (the polled UART must not hold a
 * frame), like tl_report.
 */
static void tl_restart_report(struct k_work *work)
{
	static const char *const what[] = {
		"M7 DOWN request", "A53 DOWN taken", "A53 SYSTEM_SUSPEND", "M7 A53 SCU off",
		"M7 UP request", "loader entry", "image entry", "first frame"};
	const volatile struct cluster_a53_power *pw = board_route3_power_page();
	const uint32_t hz = cntfrq();
	uint64_t t[ARRAY_SIZE(what)];
	uint64_t base = 0u, prev = 0u;
	char buf[112];

	ARG_UNUSED(work);
	if (pw == NULL || hz == 0u) {
		tl_uart_str("\nTLR none: no power page\n");
		return;
	}
	t[0] = pw->m7_down_req;
	t[1] = pw->a53_down_seen;
	t[2] = pw->a53_suspend;
	t[3] = pw->m7_scu_off;
	t[4] = pw->m7_up_req;
	t[5] = pw->a53_loader_in;
	t[6] = pw->a53_image_in;
	t[7] = pw->a53_first_frame;
	for (unsigned int i = 0; i < ARRAY_SIZE(t) && base == 0u; i++) {
		base = t[i];
	}
	snprintk(buf, sizeof(buf), "\nTLR begin hz=%u restarts=%u state=%u suspend_rc=%d\n", hz,
		 pw->restarts, pw->a53_state, pw->suspend_rc);
	tl_uart_str(buf);
	for (unsigned int i = 0; i < ARRAY_SIZE(t); i++) {
		if (t[i] == 0u) {
			snprintk(buf, sizeof(buf), "TLR - %s\n", what[i]);
		} else {
			/* signed: the M7's and the A53's stamps are one counter,
			 * but nothing orders an M7 stamp after an A53 one */
			const int64_t at = (int64_t)(t[i] - base) * 1000000 / (int64_t)hz;
			const int64_t step = prev != 0u ? (int64_t)(t[i] - prev) * 1000000 / (int64_t)hz : 0;

			snprintk(buf, sizeof(buf), "TLR %llu %s: %lld.%03lld ms (+%lld.%03lld)\n",
				 (unsigned long long)t[i], what[i], (long long)(at / 1000),
				 (long long)((at < 0 ? -at : at) % 1000), (long long)(step / 1000),
				 (long long)((step < 0 ? -step : step) % 1000));
			prev = t[i];
		}
		tl_uart_str(buf);
	}
	tl_uart_str("TLR end\n");
}

static K_WORK_DELAYABLE_DEFINE(tl_restart_work, tl_restart_report);
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

#ifdef CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_A53_RESTART
/*
 * Which pair of the four ring buffers a restart renders into: one that holds
 * nothing the M7 may still read. The last boot published into its own pair,
 * and until the M7 has seen this boot's first frame it may still be scanning
 * (`lcdif_fb`), showing (`shown`) or holding as the newest frame one of those
 * buffers -- the M7 keeps an A53 frame on the panel until it declares the
 * A53 gone, CLUSTER_FRAME_RING_TIMEOUT_MS after the last beat, and a restart
 * can come sooner. Cold boots use pair 0 as before.
 */
static unsigned int warm_buffer_pair(void)
{
	const unsigned int w = CONFIG_YAKOGL_BOARD_IMX8MP_WINDOWS;
	const uint32_t produced = R.ring->produced;
	const uint32_t shown = R.ring->shown;
	const uint32_t scanned = R.status[ST_LCDIF_FB];
	uint32_t busy = 0u, scan = 0u;

	BUILD_ASSERT(2u * CONFIG_YAKOGL_BOARD_IMX8MP_WINDOWS <= RING_FB_MAX,
		     "a restart renders into the pair the last boot did not");
	for (unsigned int i = 0; i < RING_FB_MAX; i++) {
		const uint32_t pa = RING_FB0_ADDR + i * M7_FB_STRIDE;

		for (unsigned int k = 0; k < CLUSTER_FRAME_RING_SLOTS; k++) {
			const uint32_t slot_pa = (uint32_t)R.ring->slot[k].pa;
			const uint32_t seq = R.ring->slot[k].seq;

			if (slot_pa == pa && produced != 0u &&
			    (seq == produced || (shown != 0u && seq == shown))) {
				busy |= BIT(i);
			}
		}
		if (scanned >= pa && scanned < pa + M7_FB_STRIDE) {
			scan |= BIT(i);
		}
	}
	busy |= scan;
	if ((busy & BIT_MASK(w)) == 0u) {
		return 0u;
	}
	if ((busy & (BIT_MASK(w) << w)) == 0u) {
		return w;
	}
	/* both pairs hold something: keep off the one being scanned */
	return (scan & BIT_MASK(w)) != 0u ? w : 0u;
}

/*
 * A restart's setup: the M7 runs, its ring is ours from the last boot. Map,
 * check its map once more (m7_check, which reads only), and take the ring as
 * it is: its counters, its session, its heartbeat count. Written here: nothing.
 * The heartbeat starts with the first publish (board_route3_show): a beat
 * before it would make the M7 take the last boot's newest frame for this
 * boot's, and blend its layer into it a second time.
 */
static int route3_warm_setup(void)
{
	if (!cluster_frame_ring_attached(R.ring)) {
		printk("route3: restart, but no ring (magic 0x%08x): publishing nothing\n",
		       R.ring->magic);
		R.state = BOARD_ROUTE3_MISMATCH;
		return 0;
	}
	R.buf_base = warm_buffer_pair();
	k_timer_init(&R.beat, beat_expiry, NULL);
	atomic_set(&R.m7_started, 1); /* the M7 runs: never m7_start() */
	R.panel_checked = true;       /* the panel is the M7's */
	R.lit = true;                 /* and lit by it */
	R.released_ms = k_uptime_get();
	board_route3_tl_mark(CLUSTER_TL_A53_WINDOW);
	R.state = BOARD_ROUTE3_STARTING;
	m7_check();
	if (R.state == BOARD_ROUTE3_STARTING) {
		/* the entry saw 'SAF7' and a moving heartbeat; never wait for
		 * the M7 here (the wait ends in m7_give_up's SiP STOP) */
		printk("route3: restart, but the M7 status block reads 0x%08x: publishing nothing\n",
		       R.status[ST_MAGIC]);
		R.state = BOARD_ROUTE3_MISMATCH;
		return 0;
	}
	printk("route3: restart: ring produced %u shown %u session 0x%08x, buffers %u..%u\n",
	       R.ring->produced, R.ring->shown, R.ring->session, R.buf_base,
	       R.buf_base + CONFIG_YAKOGL_BOARD_IMX8MP_WINDOWS - 1u);
	return 0;
}

void board_route3_quiesce(void)
{
	R.going_down = true;
	k_timer_stop(&R.beat);
}
#endif

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
#ifdef CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_A53_RESTART
	if (board_route3_warm()) {
		return route3_warm_setup();
	}
#endif
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
	 * flip all on the critical path. Even before the panel runs: the
	 * M7 touches the LCDIF and PWM3 (MEDIAMIX, a bus fault while it is
	 * off) only to flip, and on a handed-over start it flips nothing
	 * before our first frame, which board_route3_show() publishes only
	 * once the panel runs. Released after the panel instead, its first
	 * render of the safety layer (33 ms) stood between the panel and
	 * the first picture (2026-10-29, v12 timeline).
	 */
	if (atomic_cas(&R.m7_started, 0, 1)) {
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
#ifdef CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_A53_RESTART
	n += R.buf_base; /* 0 on a cold boot */
#endif
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

#ifdef CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_A53_RESTART
	if (R.going_down) {
		return 0; /* DOWN taken: the A53 side is on its way off */
	}
#endif
	board_route3_tl_mark(CLUSTER_TL_A53_FRAME1_DRAWN); /* the first show only */
#if CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_TIMELINE_REPORT_MS > 0
	{
		static unsigned int shows;

		if (shows < 3u) {
			tl_sample(win, shows++);
		}
		if (shows == 1u && imx8mp_display_is_running()) {
			tl_desc5_first = imx8mp_display_layer_control();
		}
	}
#endif
	if (!R.panel_checked && (R.state == BOARD_ROUTE3_LOADED || R.state == BOARD_ROUTE3_STARTING ||
				 R.state == BOARD_ROUTE3_UP)) {
		/*
		 * Nothing is published before the panel runs: the M7 flips
		 * (touching MEDIAMIX) only once there is a frame. The panel
		 * console's bring-up may still be on its thread; wait for it.
		 * Without it (or if it failed), the panel from cold on this
		 * frame. The backlight stays off either way: the M7 turns PWM3
		 * on itself right before its first flip of a frame it has
		 * composited (lcdif_m7_backlight_ensure_on); lit here, the
		 * A53's bare frame stood on the panel for the whole M7 boot
		 * (2026-10-22, the first frames wrong).
		 */
		R.panel_checked = true;
		(void)gc_panel_console_cold_wait(K_SECONDS(2));
		if (!imx8mp_display_is_running()) {
			rc = imx8mp_display_on(win->phys, win->stride);
			if (rc != 0) {
				printk("route3: no panel (%d); stopping the M7\n", rc);
				m7_give_up();
				return rc;
			}
		}
#ifdef CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_A53_RESTART
		/*
		 * The bridge is written by now whichever way the panel came up
		 * (the console's cold bring-up, which disables this line itself,
		 * or this fallback): I2C2 is the M7's from here, so its line is
		 * off in this core's GIC before the first publish -- the M7
		 * starts I2C2 only after a published frame is on its panel (desk
		 * audit D7; gc_panel_console.c says why). Idempotent.
		 */
		irq_disable(DT_IRQ(DT_NODELABEL(i2c2), irq));
#endif
	}
	switch (R.state) {
	case BOARD_ROUTE3_LOADED:
		(void)publish(win);
		if (atomic_cas(&R.m7_started, 0, 1)) {
			m7_start();
		}
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
#ifdef CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_A53_RESTART
	if (board_route3_warm() && !R.restart_published) {
		/* The restart's first frame is in the ring: now the beat, which
		 * tells the M7 the producer is back (app_frame.c,
		 * producer_is_beating), so the newest frame it then takes is
		 * this one. */
		volatile struct cluster_a53_power *pw = board_route3_power_page();

		R.restart_published = true;
		if (pw != NULL) {
			pw->a53_first_frame = cntpct();
		}
		cluster_frame_ring_heartbeat(R.ring);
		k_timer_start(&R.beat, K_MSEC(CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_BEAT_MS),
			      K_MSEC(CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_BEAT_MS));
		(void)k_work_schedule(&tl_restart_work,
				      K_MSEC(MAX(CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_TIMELINE_REPORT_MS,
						 1000)));
	}
#endif
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
	static bool tl_reported;

#ifdef CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_A53_RESTART
	tl_reported = tl_reported || board_route3_warm(); /* its own report */
#endif
	if (!tl_reported && R.lit && R.state == BOARD_ROUTE3_UP &&
	    k_uptime_get() - R.first_shown_ms >= CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_TIMELINE_REPORT_MS) {
		tl_reported = true;
		(void)k_work_submit(&tl_report_work);
	}
#endif
#ifdef CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_A53_RESTART
	if (R.state == BOARD_ROUTE3_UP) {
		board_route3_restart_arm(); /* once */
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
