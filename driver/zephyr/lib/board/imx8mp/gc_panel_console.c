/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The panel console: printk() rendered onto the DSI panel. See
 * gc_panel_console.h for why this board has no other console at all.
 *
 * The plan, in four steps:
 *
 *   1. Read the LCDIFv3 that Linux left running and believe what it says: the
 *      scan-out address, the stride, the width, the height and the pixel
 *      format. Nothing is assumed about the panel.
 *   2. Put a framebuffer of our own, same geometry, in front of it with the
 *      one handshake the Linux driver uses for a page flip. If that handshake
 *      does not complete, put the address back and draw into the buffer Linux
 *      left instead -- the picture is the only channel we have, so the code
 *      must never be able to lose it.
 *   3. Render characters with an 8x16 bitmap font and scroll by moving the
 *      picture up one text line.
 *   4. Clean the data cache over every pixel written, because the LCDIF reads
 *      the framebuffer straight out of DDR and knows nothing about our caches.
 *
 * Timing, clocks, the pixel format, the DSI bridge and the panel are never
 * touched on that path. The only registers written are the two halves of the
 * scan-out address and the shadow-load bit.
 *
 * That works on the kexec crash route, where Linux never ran
 * device_shutdown() and the LCDIF is still scanning. On the normal route it
 * does not: device_shutdown() turns the backlight off and stops the LCDIF, so
 * there is no picture to take over. For that case a second stage runs at
 * POST_KERNEL, after the I2C and GPIO drivers the DSI bridge needs, and calls
 * imx8mp_display_on() (src/display/) to bring the whole chain up on a
 * framebuffer of its own before installing the printk hook. Anything printed
 * before that point is lost; there is no way round it, because the bridge
 * sequence needs I2C and millisecond sleeps and PRE_KERNEL_1 has neither.
 *
 * Register map and the flip sequence: mainline Linux
 * drivers/gpu/drm/mxsfb/lcdif_regs.h and lcdif_kms.c. Every offset and field
 * below carries the name it has there.
 *
 * Compile-tested only. No i.MX8MP board has run this.
 */

#include <zephyr/kernel.h>
#include <zephyr/cache.h>
#include <zephyr/init.h>
#include <zephyr/kernel/mm.h>
#include <zephyr/kernel/internal/mm.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/libc-hooks.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/printk-hooks.h>
#include <zephyr/sys/util.h>
#include <zephyr/version.h>

#include <errno.h>
#include <string.h>

#include "gc_font8x16.h"
#include "gc_panel_console.h"
#include "gc_result.h"
#ifdef CONFIG_SAFEUI_DISPLAY_IMX8MP
#include "imx8mp_display.h"
#endif
/* The reachability check before any LCDIF access (Rust
 * soc::imx8mp::media::lcdif_unreachable) and, with LCDIF_RUST, the LCDIF
 * reads and the flip below (driver/rust/src/ip/lcdifv3.rs, from IMX8MPRM
 * Rev. 3 13.3), all on this file's own mappings: no device is ready at
 * PRE_KERNEL_1. */
#include <yakogl_drivers.h>

/* Filled in by gc_sample.cmake so a photograph identifies the run. */
#ifndef GC_SAMPLE_NAME
#define GC_SAMPLE_NAME "gc sample"
#endif
#ifndef GC_BUILD_STAMP
#define GC_BUILD_STAMP "unknown build"
#endif

/* ------------------------------------------------------------------ *
 * The LCDIFv3 register map
 *
 * All names, offsets and fields are from mainline Linux
 * drivers/gpu/drm/mxsfb/lcdif_regs.h ("V8 register set"), which is the
 * register set of the i.MX8MP LCDIFv3 -- the controller Linux's device tree
 * calls lcd-controller@32e80000 and hands to the DSI bridge.
 * ------------------------------------------------------------------ */

#define LCDC_V8_DISP_PARA          0x010  /* lcdif_regs.h LCDC_V8_DISP_PARA */
#define LCDC_V8_CTRLDESCL0_1       0x200  /* lcdif_regs.h LCDC_V8_CTRLDESCL0_1 */
#define LCDC_V8_CTRLDESCL0_3       0x208  /* lcdif_regs.h LCDC_V8_CTRLDESCL0_3 */
#define LCDC_V8_CTRLDESCL_LOW0_4   0x20c  /* lcdif_regs.h LCDC_V8_CTRLDESCL_LOW0_4 */
#define LCDC_V8_CTRLDESCL_HIGH0_4  0x210  /* lcdif_regs.h LCDC_V8_CTRLDESCL_HIGH0_4 */
#define LCDC_V8_CTRLDESCL0_5       0x214  /* lcdif_regs.h LCDC_V8_CTRLDESCL0_5 */

/* DISP_PARA: the display is on. lcdif_regs.h DISP_PARA_DISP_ON. */
#define DISP_PARA_DISP_ON              BIT(31)

/* CTRLDESCL0_1: the layer size in pixels.
 * lcdif_regs.h CTRLDESCL0_1_HEIGHT/_WIDTH (31:16 and 15:0). */
#define CTRLDESCL0_1_HEIGHT(reg)       (((reg) >> 16) & 0xffffu)
#define CTRLDESCL0_1_WIDTH(reg)        ((reg) & 0xffffu)

/* CTRLDESCL0_3: the stride in bytes. lcdif_regs.h CTRLDESCL0_3_PITCH_MASK
 * is GENMASK(15, 0); bits 22:16 are the undocumented AXI burst sizes, which
 * this code neither reads nor writes. */
#define CTRLDESCL0_3_PITCH(reg)        ((reg) & 0xffffu)

/* CTRLDESCL_HIGH0_4: bits 35:32 of the scan-out address.
 * lcdif_regs.h CTRLDESCL_HIGH0_4_ADDR_HIGH_MASK is GENMASK(3, 0). */
#define CTRLDESCL_HIGH0_4_ADDR_HIGH_MASK 0xfu

/* CTRLDESCL0_5: the layer enable, the shadow-load handshake and the pixel
 * format. lcdif_regs.h CTRLDESCL0_5_EN, _SHADOW_LOAD_EN, _BPP_MASK and the
 * _BPP_* codes (bits 27:24). */
#define CTRLDESCL0_5_EN                BIT(31)
#define CTRLDESCL0_5_SHADOW_LOAD_EN    BIT(30)
#define CTRLDESCL0_5_BPP_MASK          GENMASK(27, 24)
#define CTRLDESCL0_5_BPP_16_RGB565     (0x4u << 24)
#define CTRLDESCL0_5_BPP_32_ARGB8888   (0x9u << 24)
#define CTRLDESCL0_5_BPP_32_ABGR8888   (0xau << 24)

/* lcdif_regs.h LCDIF_MIN_XRES / LCDIF_MIN_YRES. */
#define LCDIF_MIN_XRES                 120
#define LCDIF_MIN_YRES                 120
/* Our own upper bound, not the hardware's: it only has to reject nonsense
 * read back from a controller that is not in the state we expect. */
#define PANEL_MAX_RES                  4096

/*
 * How many times to read CTRLDESCL0_5 waiting for the hardware to take the
 * shadow load. The Linux driver waits on the same bit with a 50 ms timeout
 * (lcdif_disable_controller), one frame being about 16 ms; this runs at
 * PRE_KERNEL_1, before the system timer exists, so it counts register reads
 * instead of microseconds. Each read crosses the peripheral bus, so 500k of
 * them is far longer than a frame and still bounded.
 */
#define SHADOW_POLL_READS              500000u

/* ------------------------------------------------------------------ */

struct panel_console {
	uint8_t  *fb;        /* pixel (0,0) of the surface we draw into */
	uint32_t  width;
	uint32_t  height;
	uint32_t  pitch;     /* bytes per scan line */
	uint32_t  bytes_pp;
	uint32_t  scale;     /* glyph zoom, 1 = one font pixel per screen pixel */
	uint32_t  fg;
	uint32_t  bg;
	uint16_t  cols;
	uint16_t  rows;
	uint16_t  col;
	uint16_t  row;
	bool      ready;
	bool      rb_swapped; /* ABGR: the panel wants B in bits 23:16 */
	struct k_spinlock lock;
};

static struct panel_console p;

#ifdef CONFIG_GC_COMMON_PANEL_CONSOLE_OWN_FB
/*
 * Our own scan-out buffer. __noinit keeps it out of the image (it is cleared
 * before the LCDIF is pointed at it) and out of the boot-time zeroing of .bss,
 * which for four megabytes is worth avoiding. It lives in the kernel's cached
 * linear mapping, so no mapping call is needed -- one less thing that can fail
 * on the way to having a console.
 */
static uint8_t panel_fb[CONFIG_GC_COMMON_PANEL_CONSOLE_FB_KB * 1024]
	__aligned(CONFIG_MMU_PAGE_SIZE) __noinit;
#endif

/* ------------------------------------------------------------------ *
 * Pixels
 * ------------------------------------------------------------------ */

static uint32_t pack(uint32_t r, uint32_t g, uint32_t b)
{
	if (p.bytes_pp == 2) {
		/* CTRLDESCL0_5_BPP_16_RGB565 */
		return ((r & 0xf8u) << 8) | ((g & 0xfcu) << 3) | (b >> 3);
	}
	/* 32 bpp: the alpha byte is opaque either way, and the two orders
	 * differ only in which of R and B sits in bits 23:16. */
	return 0xff000000u | (r << 16) | (g << 8) | b;
}

static uint32_t swap_rb(uint32_t argb)
{
	return (argb & 0xff00ff00u) | ((argb & 0x00ff0000u) >> 16) |
	       ((argb & 0x000000ffu) << 16);
}

static inline void put_pixel(uint8_t *row, uint32_t x, uint32_t colour)
{
	if (p.bytes_pp == 2) {
		((uint16_t *)row)[x] = (uint16_t)colour;
	} else {
		((uint32_t *)row)[x] = colour;
	}
}

/* Fill whole scan lines [y0, y0 + n) and clean them out of the data cache. */
static void fill_rows(uint32_t y0, uint32_t n, uint32_t colour)
{
	if (y0 >= p.height) {
		return;
	}
	n = MIN(n, p.height - y0);

	for (uint32_t y = y0; y < y0 + n; y++) {
		uint8_t *row = p.fb + (size_t)y * p.pitch;

		for (uint32_t x = 0; x < p.width; x++) {
			put_pixel(row, x, colour);
		}
	}
	sys_cache_data_flush_range(p.fb + (size_t)y0 * p.pitch, (size_t)n * p.pitch);
}

/*
 * One character cell. The cache clean is per glyph row rather than per line of
 * text so that a printk() without a trailing newline -- the last thing a hang
 * prints -- still reaches the panel.
 */
static void draw_glyph(uint32_t x0, uint32_t y0, uint8_t ch)
{
	const uint8_t *glyph = gc_font8x16[ch - GC_FONT_FIRST];
	uint32_t s = p.scale;

	for (uint32_t gy = 0; gy < GC_FONT_H; gy++) {
		uint8_t bits = glyph[gy];

		for (uint32_t sy = 0; sy < s; sy++) {
			uint8_t *row = p.fb + (size_t)(y0 + gy * s + sy) * p.pitch;

			for (uint32_t gx = 0; gx < GC_FONT_W; gx++) {
				uint32_t colour = (bits & (0x80u >> gx)) ? p.fg : p.bg;

				for (uint32_t sx = 0; sx < s; sx++) {
					put_pixel(row, x0 + gx * s + sx, colour);
				}
			}
			sys_cache_data_flush_range(row + (size_t)x0 * p.bytes_pp,
						   (size_t)GC_FONT_W * s * p.bytes_pp);
		}
	}
}

/* ------------------------------------------------------------------ *
 * The text grid
 * ------------------------------------------------------------------ */

static void scroll(void)
{
	uint32_t band = GC_FONT_H * p.scale;      /* one text line, in pixels */
	uint32_t keep = (uint32_t)(p.rows - 1) * band;

	memmove(p.fb, p.fb + (size_t)band * p.pitch, (size_t)keep * p.pitch);
	fill_rows(keep, p.height - keep, p.bg);
	sys_cache_data_flush_range(p.fb, (size_t)p.height * p.pitch);
	p.row = p.rows - 1;
}

static void newline(void)
{
	p.col = 0;
	if (p.row + 1 < p.rows) {
		p.row++;
	} else {
		scroll();
	}
}

static void put_char(uint8_t ch)
{
	if (p.col >= p.cols) {
		newline();
	}
	draw_glyph((uint32_t)p.col * GC_FONT_W * p.scale,
		   (uint32_t)p.row * GC_FONT_H * p.scale, ch);
	p.col++;
}

static int panel_console_out(int character)
{
	k_spinlock_key_t key;
	uint8_t ch = (uint8_t)character;

	if (!p.ready) {
		return character;
	}

	key = k_spin_lock(&p.lock);
	switch (ch) {
	case '\n':
		newline();
		break;
	case '\r':
		p.col = 0;
		break;
	case '\t':
		do {
			put_char(' ');
		} while ((p.col & 7u) != 0u && p.col < p.cols);
		break;
	default:
		if (ch < GC_FONT_FIRST || ch > GC_FONT_LAST) {
			ch = '?';   /* control codes and anything above ASCII */
		}
		put_char(ch);
		break;
	}
	k_spin_unlock(&p.lock, key);

	return character;
}

/* ------------------------------------------------------------------ *
 * Bring-up
 * ------------------------------------------------------------------ */

/*
 * Point the layer at `phys` and ask the hardware to take it.
 *
 * This is lcdif_plane_primary_atomic_update() followed by
 * lcdif_crtc_atomic_flush() in lcdif_kms.c: write the two halves of the
 * address, then set CTRLDESCL0_5_SHADOW_LOAD_EN. The hardware clears that bit
 * when it has latched the new address at the next vblank, which is also how
 * lcdif_disable_controller() waits for a frame to finish.
 */
static int flip_to(mm_reg_t regs, uint64_t phys)
{
#ifdef CONFIG_SAFEUI_DISPLAY_IMX8MP_LCDIF_RUST
	return yd_lcdif_flip_polled(regs, phys, SHADOW_POLL_READS);
#else
	uint32_t desc5;

	sys_write32((uint32_t)phys, regs + LCDC_V8_CTRLDESCL_LOW0_4);
	sys_write32((uint32_t)(phys >> 32) & CTRLDESCL_HIGH0_4_ADDR_HIGH_MASK,
		    regs + LCDC_V8_CTRLDESCL_HIGH0_4);

	desc5 = sys_read32(regs + LCDC_V8_CTRLDESCL0_5);
	sys_write32(desc5 | CTRLDESCL0_5_SHADOW_LOAD_EN, regs + LCDC_V8_CTRLDESCL0_5);

	for (uint32_t i = 0; i < SHADOW_POLL_READS; i++) {
		if ((sys_read32(regs + LCDC_V8_CTRLDESCL0_5) &
		     CTRLDESCL0_5_SHADOW_LOAD_EN) == 0u) {
			return 0;
		}
	}
	return -ETIMEDOUT;
#endif
}

/* Read the scan-out configuration Linux left behind. Returns false when the
 * LCDIF is not scanning anything this code knows how to draw into. */
static bool read_scanout(mm_reg_t regs, uint64_t *phys)
{
#ifdef CONFIG_SAFEUI_DISPLAY_IMX8MP_LCDIF_RUST
	struct yd_lcdif_scanout s;

	if (!yd_lcdif_read_scanout(regs, &s)) {
		return false;
	}
	p.bytes_pp = s.bytes_pp;   /* before pack(), which packs for it */
	switch (s.bpp_code << 24) {
	case CTRLDESCL0_5_BPP_32_ARGB8888:
	case CTRLDESCL0_5_BPP_16_RGB565:
		p.fg = pack(0xff, 0xff, 0xff);
		p.bg = pack(0x00, 0x20, 0x80);
		break;
	case CTRLDESCL0_5_BPP_32_ABGR8888:
		p.rb_swapped = true;
		p.fg = swap_rb(pack(0xff, 0xff, 0xff));
		p.bg = swap_rb(pack(0x00, 0x20, 0x80));
		break;
	default:
		return false;
	}
	p.width = s.width;
	p.height = s.height;
	p.pitch = s.pitch;
	if (p.width < LCDIF_MIN_XRES || p.width > PANEL_MAX_RES ||
	    p.height < LCDIF_MIN_YRES || p.height > PANEL_MAX_RES) {
		return false;
	}
	*phys = s.phys;
	return true;
#else
	uint32_t desc5 = sys_read32(regs + LCDC_V8_CTRLDESCL0_5);
	uint32_t desc1, desc3;

	if ((desc5 & CTRLDESCL0_5_EN) == 0u) {
		return false;
	}
	if ((sys_read32(regs + LCDC_V8_DISP_PARA) & DISP_PARA_DISP_ON) == 0u) {
		return false;
	}

	switch (desc5 & CTRLDESCL0_5_BPP_MASK) {
	case CTRLDESCL0_5_BPP_32_ARGB8888:
		p.bytes_pp = 4;
		p.fg = pack(0xff, 0xff, 0xff);
		p.bg = pack(0x00, 0x20, 0x80);
		break;
	case CTRLDESCL0_5_BPP_32_ABGR8888:
		p.bytes_pp = 4;
		p.rb_swapped = true;
		p.fg = swap_rb(pack(0xff, 0xff, 0xff));
		p.bg = swap_rb(pack(0x00, 0x20, 0x80));
		break;
	case CTRLDESCL0_5_BPP_16_RGB565:
		p.bytes_pp = 2;
		p.fg = pack(0xff, 0xff, 0xff);
		p.bg = pack(0x00, 0x20, 0x80);
		break;
	default:
		return false;   /* 24 bpp and the YUV formats: not worth the code */
	}

	desc1 = sys_read32(regs + LCDC_V8_CTRLDESCL0_1);
	desc3 = sys_read32(regs + LCDC_V8_CTRLDESCL0_3);
	p.width  = CTRLDESCL0_1_WIDTH(desc1);
	p.height = CTRLDESCL0_1_HEIGHT(desc1);
	p.pitch  = CTRLDESCL0_3_PITCH(desc3);

	if (p.width < LCDIF_MIN_XRES || p.width > PANEL_MAX_RES ||
	    p.height < LCDIF_MIN_YRES || p.height > PANEL_MAX_RES ||
	    p.pitch < p.width * p.bytes_pp) {
		return false;
	}

	*phys = ((uint64_t)(sys_read32(regs + LCDC_V8_CTRLDESCL_HIGH0_4) &
			    CTRLDESCL_HIGH0_4_ADDR_HIGH_MASK) << 32) |
		sys_read32(regs + LCDC_V8_CTRLDESCL_LOW0_4);
	return true;
#endif
}

/* Work the text grid out from the surface geometry. False means the panel
 * is too small for even one character, which is our cue to give up before
 * anything has been written to the controller. */
static bool grid_from_geometry(void)
{
	p.cols = (uint16_t)(p.width / (GC_FONT_W * p.scale));
	p.rows = (uint16_t)(p.height / (GC_FONT_H * p.scale));
	return p.cols != 0u && p.rows != 0u;
}

static void install_hook(const char *surface)
{
	p.ready = true;

	__printk_hook_install(panel_console_out);
	__stdout_hook_install(panel_console_out);

	printk("%s  %s  built %s  zephyr %s\n", GC_SAMPLE_NAME, CONFIG_BOARD_TARGET,
	       GC_BUILD_STAMP, KERNEL_VERSION_STRING);
	printk("panel console: %ux%u, pitch %u, %u bpp, %ux%u chars, on %s\n",
	       p.width, p.height, p.pitch, p.bytes_pp * 8, p.cols, p.rows, surface);
}

/*
 * Stage one, at PRE_KERNEL_1: take over a picture that is already there.
 * Nothing here needs the kernel, so the console exists from the earliest
 * moment it can.
 */
/* What stage 1 did, for its kind STAGE 0x14 breadcrumb (gc_result.h). */
#define TAKE_OVER_NONE        0u     /* reachable, but nothing to adopt */
#define TAKE_OVER_DONE        1u     /* the console draws on the panel */
#define TAKE_OVER_UNREACHABLE 0x100u /* | YD_LCDIF_UNREACH_*: not read at all */

/*
 * Is the LCDIF answering? It sits inside MEDIAMIX, and a register read of a
 * powered-down or unclocked MEDIAMIX never completes: the core stalls until
 * a watchdog resets the SoC. That is the U-Boot entry (the 2026-10-03
 * netboot: the last record was the breadcrumb before the first read below,
 * kind STAGE 0x13, and WDOG1 had never been fed). The crash-kexec entry
 * finds Linux's display running and passes. The check reads only the GPC
 * and the CCM, which always answer, and MEDIA_BLK_CTRL only once those say
 * it does -- the same rule the display driver applies with media_powered()
 * before its own LCDIF reads (src/display/imx8mp_display.c), with the
 * clocks added, made here on this file's own mappings because no device is
 * ready at PRE_KERNEL_1.
 */
static uint32_t lcdif_unreachable(void)
{
	mm_reg_t gpc, ccm, blk;

	device_map(&gpc, CONFIG_GC_COMMON_PANEL_CONSOLE_GPC_BASE, 0x10000, K_MEM_CACHE_NONE);
	device_map(&ccm, CONFIG_GC_COMMON_PANEL_CONSOLE_CCM_BASE, 0x10000, K_MEM_CACHE_NONE);
	device_map(&blk, CONFIG_GC_COMMON_PANEL_CONSOLE_MEDIA_BLK_BASE, 0x10000,
		   K_MEM_CACHE_NONE);
	return yd_imx8mp_lcdif_unreachable(gpc, ccm, blk);
}

static uint32_t panel_console_take_over(void)
{
	const char *surface = NULL;
	mm_reg_t regs;
	uint64_t linux_phys = 0;
	size_t need;
	uint32_t why = lcdif_unreachable();

	if (why != 0u) {
		/* Nothing to adopt: there is no running display behind a
		 * domain that is off. Stage two (panel_console_cold_init)
		 * brings the chain up itself. */
		return TAKE_OVER_UNREACHABLE | why;
	}
	device_map(&regs, CONFIG_GC_COMMON_PANEL_CONSOLE_LCDIF_BASE, 0x1000,
		   K_MEM_CACHE_NONE);

	p.scale = CONFIG_GC_COMMON_PANEL_CONSOLE_SCALE;
	if (!read_scanout(regs, &linux_phys)) {
		return TAKE_OVER_NONE;   /* no picture to write on; stage two may make one */
	}
	need = (size_t)p.pitch * p.height;

	/* Work the text grid out before anything is written to the
	 * controller. Giving up after the flip would leave the panel showing
	 * a buffer nothing draws into, which is worse than never having
	 * started. */
	if (!grid_from_geometry()) {
		return TAKE_OVER_NONE;
	}

#ifdef CONFIG_GC_COMMON_PANEL_CONSOLE_OWN_FB
	if (need <= sizeof(panel_fb)) {
		p.fb = panel_fb;
		fill_rows(0, p.height, p.bg);
		if (flip_to(regs, k_mem_phys_addr(panel_fb)) == 0) {
			surface = "our own buffer";
		} else {
			/* The hardware never took the new address. Put the old
			 * one back -- an LCDIF left pointing at a buffer it has
			 * not latched is the one way this code could cost us
			 * the only console -- and draw where Linux was drawing
			 * instead. */
			(void)flip_to(regs, linux_phys);
			p.fb = NULL;
		}
	}
#endif
	if (surface == NULL) {
		uintptr_t base = ROUND_DOWN(linux_phys, CONFIG_MMU_PAGE_SIZE);
		size_t offset = (size_t)(linux_phys - base);
		size_t len = ROUND_UP(offset + need, CONFIG_MMU_PAGE_SIZE);
		uint8_t *virt = NULL;

		k_mem_map_phys_bare(&virt, base, len, K_MEM_CACHE_WB | K_MEM_PERM_RW);
		if (virt == NULL) {
			return TAKE_OVER_NONE;
		}
		p.fb = virt + offset;
		fill_rows(0, p.height, p.bg);
		surface = "the buffer Linux left";
	}

	install_hook(surface);
	printk("lcdif 0x%08x was already scanning; the buffer Linux left is at 0x%llx\n",
	       (unsigned int)CONFIG_GC_COMMON_PANEL_CONSOLE_LCDIF_BASE,
	       (unsigned long long)linux_phys);
	gc_panel_console_stage(0, 0xff0000);
	return TAKE_OVER_DONE;
}

/* The take-over between two kind STAGE breadcrumbs (gc_result.h): its LCDIF
 * reads are the first MEDIAMIX access of the boot, made only when
 * lcdif_unreachable() says they will be answered. */
static int panel_console_init(void)
{
	uint32_t done;

	gc_result_stage(GC_RESULT_STAGE_CONSOLE_ENTER, 0u);
	done = panel_console_take_over();
	gc_result_stage(GC_RESULT_STAGE_CONSOLE_DONE, done);
	return 0;
}

SYS_INIT(panel_console_init, PRE_KERNEL_1, CONFIG_CONSOLE_INIT_PRIORITY);

#if defined(CONFIG_GC_COMMON_PANEL_CONSOLE_COLD_BRINGUP) && \
	defined(CONFIG_SAFEUI_DISPLAY_IMX8MP) && \
	defined(CONFIG_GC_COMMON_PANEL_CONSOLE_OWN_FB)
/*
 * Stage two, at POST_KERNEL after the display device: if stage one found
 * nothing, turn the panel on ourselves and draw into our own buffer.
 */
static int panel_console_cold_init(void)
{
	const struct imx8mp_display_info *info;
	uint32_t width = 0, height = 0, pitch;
	int rc;

	if (p.ready) {
		return 0;   /* stage one already has a console */
	}
	imx8mp_display_geometry(&width, &height);
	pitch = width * 4u;
	if (width == 0u || height == 0u ||
	    (size_t)pitch * height > sizeof(panel_fb)) {
		printk("panel console: %ux%u XRGB8888 does not fit in %zu bytes\n",
		       width, height, sizeof(panel_fb));
		return 0;
	}

	/* XRGB8888 is what imx8mp_display_on() programs into the LCDIF. */
	p.scale = CONFIG_GC_COMMON_PANEL_CONSOLE_SCALE;
	p.width = width;
	p.height = height;
	p.pitch = pitch;
	p.bytes_pp = 4;
	p.fg = pack(0xff, 0xff, 0xff);
	p.bg = pack(0x10, 0x18, 0x20);
	if (!grid_from_geometry()) {
		return 0;
	}
	p.fb = panel_fb;
	fill_rows(0, p.height, p.bg);

	gc_result_stage(GC_RESULT_STAGE_CONSOLE_COLD, 0u);
	rc = imx8mp_display_on(k_mem_phys_addr(panel_fb), pitch);
	gc_result_stage(GC_RESULT_STAGE_CONSOLE_COLD_DONE, (uint32_t)rc);
	if (rc) {
		p.fb = NULL;
		return 0;   /* the log the display driver wrote is all we get */
	}

	install_hook("our own buffer, display brought up by Zephyr");
	info = imx8mp_display_info();
	printk("display: %s, pixel clock %u Hz (%u.%03u fps), DSI %u Hz, PLL p%u m%u s%u\n",
	       info->adopted ? "adopted" : "cold bring-up", info->pixel_clock_hz,
	       info->refresh_mhz / 1000u, info->refresh_mhz % 1000u,
	       info->dsi_hs_clock_hz, info->pll_p, info->pll_m, info->pll_s);
	return 0;
}

SYS_INIT(panel_console_cold_init, POST_KERNEL,
	 CONFIG_GC_COMMON_PANEL_CONSOLE_COLD_PRIORITY);
#endif

/* ------------------------------------------------------------------ */

/*
 * Stage markers: a colour bar in one of eight slots along the bottom edge.
 * They tell a person looking at the panel how far a boot got when the text
 * itself cannot be trusted yet, and they need nothing but the buffer.
 */
#define STAGE_BAR_H 40u

void gc_panel_console_stage(unsigned int slot, uint32_t rgb)
{
	uint32_t colour;
	uint32_t w, x0, y0;

	if (!p.ready || slot >= 8u) {
		return;
	}
	colour = pack((rgb >> 16) & 0xffu, (rgb >> 8) & 0xffu, rgb & 0xffu);
	if (p.rb_swapped) {
		colour = swap_rb(colour);
	}
	w = p.width / 8u;
	x0 = slot * w;
	y0 = p.height - STAGE_BAR_H;
	for (uint32_t y = y0; y < p.height; y++) {
		uint8_t *row = p.fb + (size_t)y * p.pitch;

		for (uint32_t x = x0; x < x0 + w; x++) {
			put_pixel(row, x, colour);
		}
		sys_cache_data_flush_range(row + (size_t)x0 * p.bytes_pp,
					   (size_t)w * p.bytes_pp);
	}
}

/* The kernel's own init levels as bars: 1 orange = PRE_KERNEL_2 began,
 * 2 yellow = POST_KERNEL began, 3 green = APPLICATION began, which means every
 * POST_KERNEL driver init, the GPU's included, returned. Slot 0 (red) is set by
 * the console itself, 4.. by the sample. */
static int stage_pre2(void)  { gc_panel_console_stage(1, 0xff8000); return 0; }
static int stage_post(void)  { gc_panel_console_stage(2, 0xffff00); return 0; }
static int stage_app(void)   { gc_panel_console_stage(3, 0x00ff00); return 0; }
SYS_INIT(stage_pre2, PRE_KERNEL_2, 0);
SYS_INIT(stage_post, POST_KERNEL, 0);
SYS_INIT(stage_app, APPLICATION, 0);

bool gc_panel_console_ready(void)
{
	return p.ready;
}

uint16_t gc_panel_console_cols(void)
{
	return p.ready ? p.cols : 0;
}

uint16_t gc_panel_console_rows(void)
{
	return p.ready ? p.rows : 0;
}

bool gc_panel_console_surface(struct gc_panel_surface *s)
{
	if (!p.ready || s == NULL) {
		return false;
	}
	s->fb = p.fb;
	s->width = p.width;
	s->height = p.height;
	s->pitch = p.pitch;
	s->bytes_pp = p.bytes_pp;
	return true;
}
