/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * SafeUI route 3 from the direct image (board_imx8mp_route3.h): the M7
 * loaded from this image, released through TF-A, and fed through the frame
 * ring. Design, addresses and the reasons for the order:
 * docs/notes/imx8mp-direct-boot.md, "Route 3". The ring protocol is the
 * shared header <cluster/frame_ring.h>; the A53 producer that ran on Torizon
 * on 2026-10-03 was gc_common/safeui_ring_a53.c (removed in dac31030953),
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

/* ---- the M7's map (SafeUI platform/fw_imx8mp/src/m7_mem.h) ---------------- */

#define M7_CARVEOUT_BASE 0x80000000u
#define M7_UNCACHED_BASE 0x81000000u
#define M7_UNCACHED_SIZE 0x8000u
#define M7_STATUS_ADDR   0x81000000u
#define M7_STATUS_MAGIC  0x37464153u /* 'SAF7' */
#define M7_RING_ADDR     0x81005000u
#define M7_FB_APP_ADDR   0x81408000u
#define M7_FB_STRIDE     0x400000u
/* The A53's buffers: the first above the M7's scan-out pair, one stride
 * apart (fb_app is the M7's stand-in and never ours; safeui_ring_a53.c). */
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
	struct k_timer beat;
} R;

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

/* ---- the ring ------------------------------------------------------------- */

static void beat_expiry(struct k_timer *t)
{
	ARG_UNUSED(t);
	/* one increment, no printk (interrupt context, no FPU on arm64) */
	cluster_frame_ring_heartbeat(R.ring);
}

static int route3_setup(void)
{
	int rc;

	R.window = map_nc(M7_UNCACHED_BASE, M7_UNCACHED_SIZE);
	R.fb_cpu = map_nc(RING_FB0_ADDR, (size_t)RING_FB_MAX * M7_FB_STRIDE);
	if (R.window == NULL || R.fb_cpu == NULL) {
		return -ENOMEM;
	}
	R.status = (const volatile uint32_t *)(R.window + (M7_STATUS_ADDR - M7_UNCACHED_BASE));
	R.ring = (volatile struct cluster_frame_ring *)(R.window + (M7_RING_ADDR - M7_UNCACHED_BASE));
	R.page = cluster_m7_page_of(R.ring);

	/* DDR keeps its contents over a warm reset: what the last boot's M7
	 * left must not read as this boot's. Its status magic and its page
	 * go; the M7 writes both again at its start. */
	((volatile uint32_t *)R.status)[ST_MAGIC] = 0u;
	R.page->magic = 0u;
	R.ring->magic = 0u;

	rc = m7_load();
	if (rc != 0) {
		printk("route3: the M7 image did not load: %d\n", rc);
		return -EIO;
	}
	cluster_frame_ring_init(R.ring);
	k_timer_init(&R.beat, beat_expiry, NULL);
	R.state = BOARD_ROUTE3_LOADED;
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
		k_timer_start(&R.beat, K_MSEC(CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_BEAT_MS),
			      K_MSEC(CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_BEAT_MS));
		m7_release();
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
		printk("route3: the M7's first frame on the panel within %lld ms of the release\n",
		       k_uptime_get() - R.released_ms);
	}
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
