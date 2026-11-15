/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_screen_mock: driver/zephyr/lib/board/rpi5/board_rpi5.c, unchanged,
 * over rpi4/rpi_mbox.c (BCM2712 build, also unchanged) against a mocked
 * mailbox and a model of the firmware's framebuffer tags. Both files are
 * #included, so the test reaches their state; the Zephyr symbols they need
 * are the stubs under stub/zephyr/ and the few defined here (printk and its
 * character hook, fbcon_attach, device_map, the mailbox registers).
 *
 * The Rust decisions (yd_rpi_*, yd_rpi4_*: driver/rust/src/board/rpi4.rs and
 * rpi_fb.rs) are a C model here, written against the values of those files'
 * own tests (rpi_fb.rs: allocate_answers, fb_from_the_answer,
 * bus_address_per_board; rpi4.rs: pixel_order_tag, which_windows_are_views,
 * pan_rows) and checked against them below; the prototypes come from
 * yakogl_drivers.h, so a changed signature fails the build.
 *
 * Checked: the one message the screen sends (seven framebuffer tags in
 * order, depth 32, pixel order RGB, ALLOCATE aligned to 4096); a good 32 bpp
 * answer at 1920x1080 and 3840x2160 (the granted size, not the asked one),
 * with pitch = width * 4 and a padded pitch; the bus address kept with and
 * without the top bits (board 5); the raw answer line; the mapping (address,
 * size, Normal NC); a 16 bpp answer, an unanswered tag, an ALLOCATE without
 * the response bit or with address 0, a size or pitch that does not hold the
 * rows, a virtual width other than the physical one, fewer rows than the
 * screen, an error answer and a silent firmware, each refused with a named
 * line and no screen; the pixel order RGB and BGR mapped to the window
 * format, an unanswered order refusing the window only; an oversize window
 * refused by name; a view and the pans through the shared decisions; the
 * console on the UART and the framebuffer, beside the window.
 *
 * Prints "rpi5_screen_mock: N of N checks, 0 failed"; exit 0 when none failed.
 */
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifndef CONFIG_SOC_BCM2712
#define CONFIG_SOC_BCM2712 1
#endif

/* what the Kconfig symbols are in an image: the cases change them */
static uint32_t cfg_w = 1920, cfg_h = 1080, cfg_virt = 0;
#define CONFIG_YAKOGL_BOARD_RPI5_FB_WIDTH cfg_w
#define CONFIG_YAKOGL_BOARD_RPI5_FB_HEIGHT cfg_h
#define CONFIG_YAKOGL_BOARD_RPI5_FB_VIRTUAL_HEIGHT cfg_virt

#define APPLICATION 2
#define BUILD_ASSERT(cond, msg) _Static_assert(cond, msg)

#include "rpi_mbox.c"
#include "board_rpi5.c"

/* ---- the harness ---------------------------------------------------------- */

static int checks, failed;
#define CHECK(cond, ...)                                                       \
	do {                                                                   \
		checks++;                                                      \
		if (!(cond)) {                                                 \
			failed++;                                              \
			printf("  FAIL line %d: ", __LINE__);                   \
			printf(__VA_ARGS__);                                   \
			printf("\n");                                          \
		}                                                              \
	} while (0)

/* printk: appended to the log, and sent through the character hook as
 * Zephyr's is (the UART's, then the board's replacement) */
static char logbuf[32768];
static size_t loglen;
static printk_hook_fn_t hook;
static unsigned long uart_chars, fb_chars;

void printk(const char *fmt, ...)
{
	char line[512];
	va_list ap;

	va_start(ap, fmt);
	int n = vsnprintf(line, sizeof(line), fmt, ap);

	va_end(ap);
	for (int i = 0; i < n && i < (int)sizeof(line) - 1; i++) {
		if (loglen + 1 < sizeof(logbuf)) {
			logbuf[loglen++] = line[i];
			logbuf[loglen] = '\0';
		}
		if (hook) {
			hook((unsigned char)line[i]);
		}
	}
}
void __printk_hook_install(printk_hook_fn_t fn)
{
	hook = fn;
}
printk_hook_fn_t __printk_get_hook(void)
{
	return hook;
}
static int uart_hook(int c)
{
	uart_chars++;
	return c;
}
static int fbcon_hook(int c)
{
	fb_chars++;
	return c;
}

/* fbcon.c's entry point: record the arguments, install its hook */
static struct {
	int calls;
	uint8_t *fb;
	uint32_t width, pitch, rows_px, scale;
} attach;
void fbcon_attach(uint8_t *fb, uint32_t width, uint32_t pitch, uint32_t rows_px, uint32_t scale)
{
	attach.calls++;
	attach.fb = fb;
	attach.width = width;
	attach.pitch = pitch;
	attach.rows_px = rows_px;
	attach.scale = scale;
	hook = fbcon_hook;
}

void stub_sys_init_add(int (*fn)(void), int level)
{
	(void)fn;
	(void)level;
}
int64_t k_uptime_get(void)
{
	return 0;
}

static bool has(const char *s)
{
	return strstr(logbuf, s) != NULL;
}

/* ---- the Rust decisions, as a C model ------------------------------------ */

uint32_t yd_rpi4_window_format(uint32_t code, uint32_t value, uint32_t *order)
{
	if (!(code & 0x80000000u) || (code & ~0x80000000u) != 4 || value > 1) {
		return 0;
	}
	*order = value;
	return value == 1 ? GLES_NATIVE_FORMAT_RGBA8888 : GLES_NATIVE_FORMAT_XRGB8888;
}

int yd_rpi4_window_is_view(const struct yd_rpi4_fb *fb, uint32_t w, uint32_t h)
{
	return w != 0 && h != 0 && w <= fb->width && h <= fb->rows && fb->pitch % 8 == 0 &&
	       (uint64_t)fb->pitch >= (uint64_t)w * 4;
}

int yd_rpi4_pan_row(const struct yd_rpi4_fb *fb, uint64_t win_phys, uint32_t w, uint32_t h, uint32_t stride,
		    uint32_t *row)
{
	if (w == 0 || h == 0 || w > fb->width || h > fb->height || stride != fb->pitch || fb->pitch == 0 ||
	    win_phys < fb->phys) {
		return 0;
	}
	uint64_t off = win_phys - fb->phys;

	if (off % fb->pitch) {
		return 0;
	}
	uint64_t r = off / fb->pitch;

	if (r + fb->height > fb->rows) {
		return 0;
	}
	*row = (uint32_t)r;
	return 1;
}

uint64_t yd_rpi_fb_phys(uint32_t board, uint32_t bus)
{
	uint32_t phys = board == 4 ? bus & ~0xc0000000u : bus;

	return (board == 4 || board == 5) ? phys : 0;
}

int yd_rpi_allocate_ok(uint32_t width, uint32_t rows, uint32_t pitch, uint32_t size)
{
	return width != 0 && rows != 0 && size != 0 && (uint64_t)pitch >= (uint64_t)width * 4 &&
	       (uint64_t)size >= (uint64_t)pitch * rows;
}

int yd_rpi_fb_from_answer(uint32_t board, uint32_t bus, uint32_t width, uint32_t height, uint32_t rows,
			  uint32_t pitch, uint32_t size, struct yd_rpi4_fb *fb)
{
	uint64_t phys = yd_rpi_fb_phys(board, bus);

	if (height == 0 || rows < height || !yd_rpi_allocate_ok(width, rows, pitch, size) || phys == 0) {
		return 0;
	}
	*fb = (struct yd_rpi4_fb){ .phys = phys, .width = width, .height = height, .pitch = pitch, .rows = rows };
	return 1;
}

/* ---- the mailbox and the firmware ----------------------------------------- */

static uint8_t regs[0x40] __attribute__((aligned(64)));
static uint8_t *fbmem; /* big enough for 3840x2160 and its offsets */
#define FBMEM_BYTES (40u << 20)
static unsigned long long map_phys[8];
static unsigned long map_size[8];
static unsigned int map_flags[8];
static int map_calls;
static int bad_access;
static uintptr_t buf_phys = 0x00100000u;

void device_map(mm_reg_t *virt, unsigned long long phys, unsigned long size, unsigned int flags)
{
	if (map_calls < 8) {
		map_phys[map_calls] = phys;
		map_size[map_calls] = size;
		map_flags[map_calls] = flags;
	}
	map_calls++;
	*virt = phys == 0x107c013880ull ? (mm_reg_t)regs : (mm_reg_t)fbmem;
}
uintptr_t k_mem_phys_addr(void *virt)
{
	(void)virt;
	return buf_phys;
}
int sys_cache_data_flush_range(void *addr, size_t size)
{
	(void)addr;
	(void)size;
	return 0;
}
int sys_cache_data_invd_range(void *addr, size_t size)
{
	(void)addr;
	(void)size;
	return 0;
}
void k_busy_wait(uint32_t usec)
{
	(void)usec;
}
uint64_t k_cycle_get_64(void)
{
	static uint64_t c;

	return c += 10;
}

/* the firmware model: what it grants, and what it leaves unanswered */
static struct fw {
	int silent, error;
	uint32_t grant_w, grant_h, grant_vw, grant_vh; /* 0: echo the request */
	uint32_t depth;                                /* 0: echo */
	uint32_t bus;                                  /* ALLOCATE's address */
	int alloc_acked;                               /* 0: no response bit on ALLOCATE */
	uint32_t pitch_pad;                            /* bytes added to a row */
	uint32_t size_adjust;                          /* subtracted from size */
	uint32_t pitch_override, size_override;        /* 0: none */
	uint32_t order_value;                          /* the order it answers (0 BGR, 1 RGB) */
	int order_acked;
	uint32_t unanswered_tag;                       /* a tag id left without the response bit */
	uint32_t pan_echo_y;                           /* 0: echo; else answer this row */
} fw;
static uint32_t req_tags[16], req_n, req_vals[16][2];
static int req_msgs;
static uint32_t pans, last_pan_y;
static uint32_t mail0;
static int have_mail0;
static uint32_t cur_w, cur_h, cur_vw, cur_vh;

static void firmware(void)
{
	uint32_t n = 2;

	req_msgs++;
	req_n = 0;
	while (n < buf[0] / 4 && buf[n] != 0) {
		uint32_t tag = buf[n], bytes = buf[n + 1], *v = &buf[n + 3];
		int ack = tag != fw.unanswered_tag;

		if (req_n < 16) {
			req_tags[req_n] = tag;
			req_vals[req_n][0] = bytes >= 4 ? v[0] : 0;
			req_vals[req_n][1] = bytes >= 8 ? v[1] : 0;
			req_n++;
		}
		switch (tag) {
		case 0x00048003:
			cur_w = fw.grant_w ? fw.grant_w : v[0];
			cur_h = fw.grant_h ? fw.grant_h : v[1];
			v[0] = cur_w;
			v[1] = cur_h;
			break;
		case 0x00048004:
			cur_vw = fw.grant_vw ? fw.grant_vw : v[0];
			cur_vh = fw.grant_vh ? fw.grant_vh : v[1];
			v[0] = cur_vw;
			v[1] = cur_vh;
			break;
		case 0x00048005:
			v[0] = fw.depth ? fw.depth : v[0];
			break;
		case 0x00048006:
			v[0] = fw.order_value;
			ack = ack && fw.order_acked;
			break;
		case 0x00048009: /* SET_VIRTUAL_OFFSET: echoes x, y */
			if (fw.pan_echo_y) {
				v[1] = fw.pan_echo_y;
			}
			last_pan_y = v[1];
			pans += bytes == 8 && req_n == 1; /* a request of this tag alone */
			break;
		case 0x00040001:
			v[0] = fw.bus;
			if (fw.size_override) {
				v[1] = fw.size_override;
			} else {
				v[1] = (cur_vw * 4 + fw.pitch_pad) * cur_vh - fw.size_adjust;
			}
			ack = ack && fw.alloc_acked;
			break;
		case 0x00040008:
			v[0] = fw.pitch_override ? fw.pitch_override : cur_vw * 4 + fw.pitch_pad;
			break;
		default:
			break;
		}
		if (ack) {
			buf[n + 2] = 0x80000000u | bytes;
		}
		n += 3 + bytes / 4;
	}
	buf[1] = fw.error ? 0x80000001u : STATUS_SUCCESS;
}

uint32_t sys_read32(mm_reg_t addr)
{
	uint32_t off = (uint32_t)(addr - (mm_reg_t)regs);

	if (addr < (mm_reg_t)regs || off >= sizeof(regs)) {
		bad_access++;
		return 0;
	}
	switch (off) {
	case MAIL1_STA:
		return 0;
	case MAIL0_STA:
		return have_mail0 ? 0 : MBOX_EMPTY;
	case MAIL0_RD:
		have_mail0 = 0;
		return mail0;
	default:
		bad_access++;
		return 0;
	}
}
void sys_write32(uint32_t val, mm_reg_t addr)
{
	if (addr != (mm_reg_t)regs + MAIL1_WRT) {
		bad_access++;
		return;
	}
	if (fw.silent) {
		return;
	}
	firmware();
	mail0 = val;
	have_mail0 = 1;
}

/* ---- cases ------------------------------------------------------------------ */

#define FB_BUS 0x3c100000u

static void fw_reset(void)
{
	memset(&fw, 0, sizeof(fw));
	fw.bus = FB_BUS;
	fw.alloc_acked = 1;
	fw.order_acked = 1;
	fw.order_value = 1;
	req_msgs = 0;
	req_n = 0;
	pans = 0;
	last_pan_y = UINT32_MAX;
	have_mail0 = 0;
}

/* a fresh board layer: its state, the log, the hooks, the mappings */
static void board_reset_state(void)
{
	s_uart_out = s_fb_out = NULL;
	s_cpu = NULL;
	memset(&s_fb, 0, sizeof(s_fb));
	s_format = 0;
	s_taken = s_view = s_console_moved = false;
	s_shown_y = 0;
	loglen = 0;
	logbuf[0] = '\0';
	hook = uart_hook;
	uart_chars = fb_chars = 0;
	memset(&attach, 0, sizeof(attach));
	map_calls = 0;
	cfg_w = 1920;
	cfg_h = 1080;
	cfg_virt = 0;
}

static void boot(void)
{
	/* the init hook as Zephyr runs it: rpi_mbox_init, then the screen */
	CHECK(board_rpi5_init() == 0, "the init hook must not fail");
}

static struct board_window gwin; /* the full-screen window of the last case_good */

static void case_good(const char *name, uint32_t gw, uint32_t gh, uint32_t pad, uint32_t bus)
{
	board_reset_state();
	fw_reset();
	fw.grant_w = gw;
	fw.grant_h = gh;
	fw.grant_vw = gw;
	fw.grant_vh = gh;
	fw.pitch_pad = pad;
	fw.bus = bus;
	boot();
	const uint32_t pitch = gw * 4 + pad, size = pitch * gh;

	CHECK(rpi5_screen_ready(), "%s: no screen", name);
	uint32_t w = 0, h = 0, rows = 0, p = 0;

	rpi5_screen_info(&w, &h, &rows, &p);
	CHECK(w == gw && h == gh && rows == gh && p == pitch, "%s: screen %ux%u rows %u pitch %u", name, w, h, rows,
	      p);
	CHECK(s_fb.phys == bus, "%s: phys %#llx, the bus address %#x kept", name, (unsigned long long)s_fb.phys, bus);
	CHECK(map_calls == 2 && map_phys[1] == bus && map_size[1] == size && map_flags[1] == K_MEM_ARM_NORMAL_NC,
	      "%s: framebuffer mapped at %#llx + %lu flags %u (%d maps)", name, map_phys[1], map_size[1],
	      map_flags[1], map_calls);
	char want[160];

	snprintf(want, sizeof(want), "board: rpi5 fb answer allocate 0x%08x size %u pitch %u depth 32 order code",
		 bus, size, pitch);
	CHECK(has(want), "%s: the raw answer line (%s) is missing", name, want);
	snprintf(want, sizeof(want), "board: rpi5 framebuffer %ux%u (%u rows in memory) pitch %u phys 0x%08x", gw,
		 gh, gh, pitch, bus);
	CHECK(has(want), "%s: the screen line (%s) is missing", name, want);
	CHECK(attach.calls == 1 && attach.fb == fbmem && attach.width == gw && attach.pitch == pitch &&
		      attach.rows_px == gh && attach.scale == 1,
	      "%s: console attached %d times (%u x %u, pitch %u)", name, attach.calls, attach.width, attach.rows_px,
	      attach.pitch);
	{
		unsigned long u0 = uart_chars, f0 = fb_chars;

		printk("hello\n");
		CHECK(uart_chars - u0 == 6 && fb_chars - f0 == 6,
		      "%s: the console writes to the UART (%lu) and the framebuffer (%lu)", name, uart_chars - u0,
		      fb_chars - f0);
	}
	struct board_window *win = &gwin;

	CHECK(rpi5_window(gw, gh, win) == 0 && win->phys == bus && win->cpu == fbmem && win->w == gw &&
		      win->h == gh && win->stride == pitch &&
		      win->format == (GLES_NATIVE_FORMAT_RGBA8888 | GLES_NATIVE_WINDOW_SCANOUT),
	      "%s: the full-screen window", name);
}

int main(void)
{
	fbmem = calloc(FBMEM_BYTES, 1);
	if (!fbmem) {
		puts("rpi5_screen_mock: out of memory");
		return 1;
	}
	CHECK(rpi_mbox_init() == 0, "rpi_mbox_init");
	struct board_window win;

	/* 1. the model of the decisions agrees with the Rust tests' values */
	{
		uint32_t o = 9;
		CHECK(yd_rpi4_window_format(0x80000004u, 1, &o) == GLES_NATIVE_FORMAT_RGBA8888 && o == 1,
		      "pixel order RGB");
		CHECK(yd_rpi4_window_format(0x80000004u, 0, &o) == GLES_NATIVE_FORMAT_XRGB8888 && o == 0,
		      "pixel order BGR");
		CHECK(yd_rpi4_window_format(4, 1, &o) == 0 && yd_rpi4_window_format(0x80000008u, 1, &o) == 0 &&
			      yd_rpi4_window_format(0x80000004u, 2, &o) == 0,
		      "pixel order unanswered, wrong length, no such order");
		CHECK(yd_rpi_fb_phys(5, 0xfc100000u) == 0xfc100000u && yd_rpi_fb_phys(4, 0xfc100000u) == 0x3c100000u &&
			      yd_rpi_fb_phys(5, 0) == 0 && yd_rpi_fb_phys(4, 0xc0000000u) == 0,
		      "bus addresses per board");
		CHECK(yd_rpi_allocate_ok(1920, 1080, 7680, 7680 * 1080) == 1 &&
			      yd_rpi_allocate_ok(1920, 1080, 7680, 0) == 0 &&
			      yd_rpi_allocate_ok(1920, 1080, 7676, 7680 * 1080) == 0 &&
			      yd_rpi_allocate_ok(1920, 1080, 7680, 7680 * 1080 - 1) == 0,
		      "allocate answers");
		struct yd_rpi4_fb f;
		CHECK(yd_rpi_fb_from_answer(5, 0x3c100000u, 1920, 1080, 2160, 7680, 7680 * 2160, &f) == 1 &&
			      f.phys == 0x3c100000u && f.rows == 2160 &&
			      yd_rpi_fb_from_answer(5, 0x1000, 1920, 1080, 540, 7680, 7680 * 540, &f) == 0,
		      "fb from the answer");
		struct yd_rpi4_fb v = { 0x3c100000u, 1920, 720, 7680, 1440 };
		uint32_t row = 0;
		CHECK(yd_rpi4_window_is_view(&v, 1920, 1440) == 1 && yd_rpi4_window_is_view(&v, 1921, 720) == 0 &&
			      yd_rpi4_pan_row(&v, v.phys + 7680ull * 720, 1920, 720, 7680, &row) == 1 && row == 720 &&
			      yd_rpi4_pan_row(&v, v.phys + 7680ull * 721, 1920, 720, 7680, &row) == 0,
		      "views and pan rows");
	}

	/* 2. a good 32 bpp answer: 1080p, 4K (granted over the asked size),
	 *    a padded pitch, bus addresses with and without the top bits */
	case_good("1080p", 1920, 1080, 0, FB_BUS);
	CHECK(req_msgs == 1 && req_n == 7 && req_tags[0] == 0x48003 && req_tags[1] == 0x48004 &&
		      req_tags[2] == 0x48005 && req_tags[3] == 0x48006 && req_tags[4] == 0x48009 &&
		      req_tags[5] == 0x40001 && req_tags[6] == 0x40008,
	      "one message, %u tags (first %#x last %#x) in the framebuffer group's order in %d messages", req_n,
	      req_tags[0], req_tags[6], req_msgs);
	CHECK(req_vals[0][0] == 1920 && req_vals[0][1] == 1080 && req_vals[1][0] == 1920 &&
		      req_vals[1][1] == 1080 && req_vals[2][0] == 32 && req_vals[3][0] == 1 &&
		      req_vals[4][0] == 0 && req_vals[4][1] == 0 && req_vals[5][0] == 4096,
	      "asked: size %ux%u virtual %ux%u depth %u order %u offset %u,%u align %u", req_vals[0][0],
	      req_vals[0][1], req_vals[1][0], req_vals[1][1], req_vals[2][0], req_vals[3][0], req_vals[4][0],
	      req_vals[4][1], req_vals[5][0]);
	CHECK(rpi5_show(&gwin) == 0 && pans == 0, "showing the screen's own row 0 calls no pan (%u)", pans);
	CHECK(rpi5_window(10, 10, &win) == -ENOMEM && win.w == 0 && win.cpu == NULL, "a second window is -ENOMEM");

	case_good("4K granted over the asked 1080p", 3840, 2160, 0, FB_BUS);
	CHECK(s_fb.width == 3840 && has("asked 1920x1080 rows 1080") && has("physical 3840x2160 virtual 3840x2160"),
	      "the granted size is the screen, the asked size is in the line");
	CHECK(map_size[1] == 3840ul * 2160 * 4, "a 4K screen maps %lu bytes (32 MiB class)", map_size[1]);

	case_good("padded pitch", 1920, 1080, 64, FB_BUS);
	CHECK(s_fb.pitch == 7744 && attach.pitch == 7744, "the pitch is the granted 7744: %u", s_fb.pitch);

	case_good("top bits kept", 1920, 1080, 0, 0xfc100000u);
	case_good("no top bits", 1920, 1080, 0, 0x3e000000u);

	/* 3. a 16 bpp answer is refused, named, no screen, no window */
	board_reset_state();
	fw_reset();
	fw.depth = 16;
	boot();
	CHECK(!rpi5_screen_ready() && has("refused: depth 16 is not 32 bpp") && has("the UART only"),
	      "16 bpp: refused by name, the UART only");
	CHECK(has("board: rpi5 fb answer allocate 0x3c100000") && map_calls == 1 && attach.calls == 0,
	      "16 bpp: the raw answer is logged, nothing mapped, no console in the framebuffer");
	CHECK(rpi5_window(640, 480, &win) == -ENODEV && rpi5_show(&win) == -ENODEV, "16 bpp: no window, no show");
	CHECK(uart_chars > 0 && fb_chars == 0, "16 bpp: the console stays on the UART");

	/* 4. ALLOCATE not answered: no response bit, address 0, size 0 */
	board_reset_state();
	fw_reset();
	fw.alloc_acked = 0;
	boot();
	CHECK(!rpi5_screen_ready() && has("refused: ALLOCATE was not answered"), "ALLOCATE without the response bit");
	board_reset_state();
	fw_reset();
	fw.bus = 0;
	boot();
	CHECK(!rpi5_screen_ready() && has("refused: ALLOCATE gave address 0x00000000"), "ALLOCATE with address 0");
	board_reset_state();
	fw_reset();
	fw.unanswered_tag = 0x00048005;
	boot();
	CHECK(!rpi5_screen_ready() && has("refused: SET_DEPTH was not answered"), "SET_DEPTH not answered");
	board_reset_state();
	fw_reset();
	fw.unanswered_tag = 0x00040008;
	boot();
	CHECK(!rpi5_screen_ready() && has("refused: GET_PITCH was not answered"), "GET_PITCH not answered");

	/* 5. a size or pitch that does not hold the rows */
	board_reset_state();
	fw_reset();
	fw.size_adjust = 1;
	boot();
	CHECK(!rpi5_screen_ready() && has("refused: pitch 7680 size 8294399 do not hold 1080 rows of 1920"),
	      "size one byte short of rows * pitch");
	board_reset_state();
	fw_reset();
	fw.size_override = 7680 * 540;
	boot();
	CHECK(!rpi5_screen_ready() && has("do not hold 1080 rows"), "size of half the rows");
	board_reset_state();
	fw_reset();
	fw.pitch_override = 7676;
	boot();
	CHECK(!rpi5_screen_ready() && has("pitch 7676") && has("do not hold"), "pitch shorter than a row of pixels");
	board_reset_state();
	fw_reset();
	fw.grant_vw = 1280;
	boot();
	CHECK(!rpi5_screen_ready() && has("refused: virtual width 1280 is not the physical width 1920"),
	      "virtual width other than the physical one");
	board_reset_state();
	fw_reset();
	fw.grant_vh = 540;
	boot();
	CHECK(!rpi5_screen_ready() && has("refused: 540 rows in memory under the 1080 of the screen"),
	      "fewer rows in memory than the screen");

	/* 6. the transport: an error answer, a silent firmware */
	board_reset_state();
	fw_reset();
	fw.error = 1;
	boot();
	CHECK(!rpi5_screen_ready() && has("request failed (-5)") && has("the UART only"), "an error answer is -EIO");
	board_reset_state();
	fw_reset();
	fw.silent = 1;
	boot();
	CHECK(!rpi5_screen_ready() && has("request failed (-110)"), "a silent firmware is -ETIMEDOUT");

	/* 7. the pixel order: RGB, BGR, unanswered */
	board_reset_state();
	fw_reset();
	boot();
	CHECK(rpi5_window(640, 480, &win) == 0 && (win.format & 0xffu) == GLES_NATIVE_FORMAT_RGBA8888 &&
		      has("pixel order RGB depth 32: windows are RGBA8888"),
	      "RGB: RGBA8888");
	board_reset_state();
	fw_reset();
	fw.order_value = 0;
	boot();
	CHECK(rpi5_window(640, 480, &win) == 0 &&
		      win.format == (GLES_NATIVE_FORMAT_XRGB8888 | GLES_NATIVE_WINDOW_SCANOUT) &&
		      has("pixel order BGR depth 32: windows are XRGB8888"),
	      "BGR: XRGB8888");
	board_reset_state();
	fw_reset();
	fw.order_acked = 0;
	boot();
	CHECK(rpi5_screen_ready() && has("pixel order not answered: no window"), "an unanswered order keeps the screen");
	CHECK(rpi5_window(640, 480, &win) == -ENODEV && win.w == 0 && has("window 640x480 refused: the pixel order"),
	      "an unanswered order refuses the window by name");

	/* 8. a window the screen cannot hold is refused by name */
	board_reset_state();
	fw_reset();
	boot();
	win.w = 7;
	CHECK(rpi5_window(1921, 1080, &win) == -EFBIG && win.w == 0 &&
		      has("window 1921x1080 refused: larger than the granted screen 1920x1080 (1080 rows in memory)"),
	      "wider than the screen");
	CHECK(rpi5_window(1920, 1081, &win) == -EFBIG && has("window 1920x1081 refused: larger than"),
	      "taller than the rows in memory");
	CHECK(rpi5_window(0, 10, &win) == -EINVAL && rpi5_window(10, 0, &win) == -EINVAL, "zero sizes");
	CHECK(!s_taken, "refused windows take nothing");
	board_reset_state();
	fw_reset();
	fw.grant_w = fw.grant_vw = 3840;
	fw.grant_h = fw.grant_vh = 2160;
	boot();
	CHECK(rpi5_window(3841, 2160, &win) == -EFBIG && has("larger than the granted screen 3840x2160"),
	      "oversize against a 4K grant");
	CHECK(rpi5_window(1920, 1080, &win) == 0, "a 1080p window on a 4K screen is a view");
	/* the console moved beside it: 1920 wide, 1920 pixels free */
	CHECK(attach.calls == 2 && attach.fb == fbmem + 1920 * 4 && attach.width == 1920 && attach.pitch == 15360 &&
		      attach.rows_px == 2160,
	      "the console is beside the window (%d attaches, x offset %ld)", attach.calls,
	      (long)(attach.fb - fbmem));

	/* 9. a view and the pans, through the shared decisions */
	board_reset_state();
	fw_reset();
	cfg_virt = 2160; /* two screens for a double buffer */
	boot();
	CHECK(rpi5_screen_ready() && s_fb.rows == 2160 && s_fb.height == 1080 && req_vals[1][1] == 2160,
	      "two screens: rows %u of %u, virtual height asked %u", s_fb.rows, s_fb.height, req_vals[1][1]);
	CHECK(rpi5_window(1920, 2160, &win) == 0 && win.h == 2160, "the two-screen window is a view");
	CHECK(attach.calls == 1 && s_fb_out == NULL, "no room beside it: the console is the UART only");
	{
		unsigned long f0 = fb_chars;

		printk("after\n");
		CHECK(fb_chars == f0, "the UART only: nothing goes to the framebuffer");
	}
	struct board_window half = win;
	half.h = 1080;
	half.phys = win.phys + 7680ull * 1080;
	CHECK(rpi5_show(&half) == 0 && pans == 1 && last_pan_y == 1080 && s_shown_y == 1080,
	      "the second half pans to row 1080 (%u pans, y %u)", pans, last_pan_y);
	CHECK(rpi5_show(&half) == 0 && pans == 1, "the same half again calls no pan");
	half.phys = win.phys;
	CHECK(rpi5_show(&half) == 0 && pans == 2 && last_pan_y == 0 && s_shown_y == 0, "back to row 0");
	half.phys = win.phys + 7680ull * 1081;
	CHECK(rpi5_show(&half) == -EINVAL && s_shown_y == 0, "a part reaching past the rows is refused");
	half.stride = 1920 * 4 + 8;
	half.phys = win.phys;
	CHECK(rpi5_show(&half) == -EINVAL && has("show of 1920x1080 refused: not a part of the view"),
	      "another stride is not a part of the view: refused, nothing copied");
	fw.pan_echo_y = 3;
	half = win;
	half.h = 1080;
	half.phys = win.phys + 7680ull * 1080;
	CHECK(rpi5_show(&half) == -EIO && s_shown_y == 0 && has("pan to row 1080 not taken (echo 3)"),
	      "a pan the firmware answers with another row is -EIO and the screen row stays");

	free(fbmem);
	printf("rpi5_screen_mock: %d of %d checks, %d failed\n", checks - failed, checks, failed);
	return failed ? 1 : 0;
}
