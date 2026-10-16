/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * yakogl_drivers.h -- the C view of libyakogl_drivers.a (driver/rust/), for
 * the Zephyr shims in driver/drivers/<class>/.
 *
 * Hand-written. Every struct here has a #[repr(C)] twin in
 * driver/rust/src/env.rs or driver/rust/src/ffi.rs; the _Static_asserts at
 * the end pin the layout the Rust side asserts too (core::mem::offset_of!),
 * so either side moving a field stops the build. LP64 only (aarch64).
 *
 * Conventions: `cfg` points into dev->config (devicetree values), `data` into
 * dev->data (all mutable state; the Rust code has no statics), `env` at the
 * shim's struct yd_env. Errors are negative errno. Calls on one instance must
 * be serialised by the shim. Only integers and pointers cross: the Rust code
 * is built softfloat and runs at PRE_KERNEL_1 and in ISRs.
 */
#ifndef YAKOGL_DRIVERS_H
#define YAKOGL_DRIVERS_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* The OS services a driver borrows. `bus` / `line` are indices the shim
 * defines for its own device. A service the device does not have is a stub
 * returning -ENOTSUP. Log levels are Zephyr's (1 ERR .. 4 DBG); `buf` is not
 * NUL-terminated. */
struct yd_env {
	void *ctx;
	void (*busy_wait_us)(void *ctx, uint32_t us);
	void (*sleep_ms)(void *ctx, uint32_t ms);
	void (*log)(void *ctx, uint32_t level, const uint8_t *buf, size_t len);
	int (*i2c_write)(void *ctx, uint32_t bus, const uint8_t *buf, size_t len);
	int (*i2c_write_read)(void *ctx, uint32_t bus, const uint8_t *wbuf, size_t wlen,
			      uint8_t *rbuf, size_t rlen);
	int (*gpio_set)(void *ctx, uint32_t line, uint32_t value);
};

/* Called by the Rust panic handler: log, flush, k_panic(). Never returns.
 * Defined by the Zephyr glue (driver/drivers/common/yd_zephyr.c). */
void yd_panic(const uint8_t *file, size_t file_len, uint32_t line) __attribute__((noreturn));

/* ---- i.MX PWM (ip::imx_pwm; yakogl,imx-pwm) ---------------------------- */

struct yd_imx_pwm_config {
	uint32_t ccm_root;   /* CCM ROOT[] index of the PWM clock root (ccm-root) */
	uint32_t ccm_gate;   /* CCM CCGR[] index of its clock gate (ccm-gate) */
};

struct yd_imx_pwm_data {
	uintptr_t pwm_base;  /* device_map() of the PWM block */
	uintptr_t ccm_base;  /* device_map() of the CCM */
	uint32_t rate_hz;    /* get_cycles_per_sec */
	uint32_t period;     /* the last request, for a debugger */
	uint32_t pulse;
	uint32_t flags;
};

int yd_imx_pwm_init(const struct yd_imx_pwm_config *cfg, struct yd_imx_pwm_data *data,
		    const struct yd_env *env);
int yd_imx_pwm_set_cycles(const struct yd_imx_pwm_config *cfg, struct yd_imx_pwm_data *data,
			  const struct yd_env *env, uint32_t channel, uint32_t period,
			  uint32_t pulse, uint32_t flags);
int yd_imx_pwm_get_cycles_per_sec(const struct yd_imx_pwm_config *cfg,
				  const struct yd_imx_pwm_data *data, uint32_t channel,
				  uint64_t *cycles);
/* Enabled with a clock selected, read from the hardware (health reports). */
bool yd_imx_pwm_is_enabled(const struct yd_imx_pwm_data *data);

/* ---- the result record writer (board::record; lib/board/common/record.c) --
 * The rules of the record (driver/rust/src/board/record.rs, Writer): the
 * sequence number, and breadcrumbs dropped once a result was stored. The C
 * side keeps one writer per image, zero-initialised, calls one of these and
 * stores out[0..3] in its sink when the call returns 1 (0: dropped). The
 * kinds are yakogl_record.h's YK_RECORD_KIND_*. Any context: no state but
 * the writer, no FP. */
struct yd_record_writer {
	uint8_t seq;
	uint8_t have_result;   /* 1 once a result record was stored */
	uint8_t pad[2];
	uint32_t progress_count;
};

int yd_record_result(struct yd_record_writer *w, uint32_t kind, uint32_t w1, uint32_t w2,
		     uint32_t w3, uint32_t out[4]);
int yd_record_result_tag(struct yd_record_writer *w, uint32_t kind, uint32_t tag, uint32_t w1,
			 uint32_t w2, uint32_t w3, uint32_t out[4]);
int yd_record_progress(struct yd_record_writer *w, uint32_t where, uint32_t uptime_ms,
		       uint32_t out[4]);
int yd_record_breadcrumb(struct yd_record_writer *w, uint32_t kind, uint32_t tag, uint32_t w1,
			 uint32_t w2, uint32_t w3, uint32_t out[4]);

/* kind GLMARK2's w1..w3 (record.rs glmark2_words, Glmark2Outcome): the
 * entries are indices in glmark2's benchmark list, YK_RECORD_GLMARK2_ENTRY_NONE
 * when there is none; why is a YK_RECORD_GLMARK2_WHY_*, which
 * yd_record_glmark2_why gives for one line glmark2 logged. */
struct yd_glmark2_outcome {
	uint32_t score;
	uint32_t flags;        /* YK_RECORD_GLMARK2_OFFSCREEN, YK_RECORD_GLMARK2_GL */
	uint32_t status;       /* YK_RECORD_STATUS_PASS / _FAIL */
	uint32_t ran, failed, unsupported;
	uint32_t failed_entry; /* the first failed entry */
	uint32_t why;          /* for failed_entry */
	uint32_t unsupported_entries[2];
	uint32_t w, h;
	/* stored only with YK_RECORD_GLMARK2_GL in flags: the GL core's
	 * yakogl_gl_errors_raised() and yakogl_incomplete_texture_draws() */
	uint32_t gl_errors;
	uint32_t incomplete_draws;
};

void yd_record_glmark2_words(const struct yd_glmark2_outcome *o, uint32_t out[3]);
uint32_t yd_record_glmark2_why(const char *line, size_t n);

/* kind DISPLAY's w1..w3 (record.rs display_words, Display): the i.MX8MP
 * display chain while it scans a window. */
struct yd_record_display {
	uint32_t adopted;        /* 1: the LCDIF was already scanning */
	uint32_t pixel_clock_hz;
	uint32_t lane_rate_hz;   /* the DSI lane rate, 0 unknown */
	uint32_t irq_now;        /* the bridge's 0xE5 now, YK_RECORD_DISPLAY_IRQ_NOT_READ */
	uint32_t irq_at_start;   /* 0xE5 one ms after the video started, 0xff unchecked */
	uint32_t health;         /* IMX8MP_DISPLAY_HEALTH_* */
	uint32_t bpp_code;       /* CTRLDESCL0_5 bits 27:24 */
	uint32_t pitch;          /* bytes */
};

void yd_record_display_words(const struct yd_record_display *d, uint32_t out[3]);

/* The i.MX8MP samples' hold before the SoC reset (src/board/imx8mp.rs,
 * through src/ffi/board.rs), for driver/zephyr/lib/board/imx8mp/gc_sample.c.
 * Returns the seconds; *code (may be NULL) the code the hold carries, -1 none. */
struct yd_imx8mp_hold {
	uint32_t recorded;   /* 1: the verdict is in the result record */
	uint32_t failed;
	int32_t code;        /* the sample's own code, 0..63, or -1 */
	uint32_t gpu_resets;
	uint32_t pass_s, fail_s, step_s;
	uint32_t reset_base, reset_max;
};

uint32_t yd_imx8mp_hold_seconds(const struct yd_imx8mp_hold *h, int32_t *code);

/* The Raspberry Pi 4's framebuffer decisions (src/board/rpi4.rs, through
 * src/ffi/board.rs), for driver/zephyr/lib/board/rpi4/board_rpi4.c. */
struct yd_rpi4_fb {
	uint64_t phys;
	uint32_t width;  /* the screen's */
	uint32_t height; /* the screen's */
	uint32_t pitch;  /* bytes per row */
	uint32_t rows;   /* rows in memory: the virtual height */
};
/* The window format (GLES_NATIVE_FORMAT_*) for the pixel-order tag's response
 * code word and value, *order the order (0 BGR, 1 RGB); 0 when unanswered. */
uint32_t yd_rpi4_window_format(uint32_t code, uint32_t value, uint32_t *order);
/* 1: a w x h window can be a view into the framebuffer (SCANOUT_VIEW). */
int yd_rpi4_window_is_view(const struct yd_rpi4_fb *fb, uint32_t w, uint32_t h);
/* 1 and *row: the window at win_phys is a part of the view, shown from that
 * row (a pan when the screen is elsewhere); 0: it copies. */
int yd_rpi4_pan_row(const struct yd_rpi4_fb *fb, uint64_t win_phys, uint32_t w, uint32_t h,
		    uint32_t stride, uint32_t *row);

/* ---- i.MX WDOG self-arm (ip::imx_wdog; gc_board gc_wdog_arm.c) ----------
 * One call at PRE_KERNEL_1: the WDOG clock root and gate on, then WDOG1 armed
 * for timeout_ms (whole 0.5 s steps, 0.5 s .. 128 s) from whatever state it
 * was in, serviced and read back. 0, -EIO when the WDOG refused (refusal says
 * why), -EINVAL for a bad argument. */

#define YD_IMX_WDOG_WAS_RUNNING  (1u << 0) /* WCR.WDE was set: an earlier stage started it */
#define YD_IMX_WDOG_CLEARED_PDE  (1u << 1) /* WMCR.PDE was set and is now clear */
#define YD_IMX_WDOG_ROOT_ENABLED (1u << 2) /* the CCM WDOG root was off and is now on */
#define YD_IMX_WDOG_GATE_OPENED  (1u << 3) /* the CCM WDOG1 gate was closed and is now open */

#define YD_IMX_WDOG_REFUSED_TIMEOUT       1 /* not a whole 0.5 s step in 0.5 .. 128 s */
#define YD_IMX_WDOG_REFUSED_PDE_STUCK     2 /* WMCR.PDE read back set after clearing */
#define YD_IMX_WDOG_REFUSED_NOT_ENABLED   3 /* WCR.WDE read back clear */
#define YD_IMX_WDOG_REFUSED_WT_READ_BACK  4 /* WCR.WT read back other than written */
#define YD_IMX_WDOG_REFUSED_WDT_READ_BACK 5 /* WCR.WDT asked for, read back clear */

struct yd_imx_wdog_arm {
	uint16_t wcr_found;   /* WCR as found, before any write */
	uint16_t wmcr_found;  /* WMCR as found */
	uint16_t wcr_now;     /* WCR read back (on a refusal: the read it is about) */
	uint16_t _pad;
	uint32_t flags;       /* YD_IMX_WDOG_* */
	uint32_t refusal;     /* 0 or YD_IMX_WDOG_REFUSED_* */
	uint32_t timeout_ms;  /* from the read-back WT; 0 on a refusal */
};

int yd_imx_wdog_arm(uintptr_t wdog_base, uintptr_t ccm_base, uint32_t timeout_ms,
		    uint32_t ext_reset, struct yd_imx_wdog_arm *out);

/* ---- TI SN65DSI83/84 (ip::sn65dsi8x; yakogl,sn65dsi8x) ------------------
 * I2C bus 0 of the env is the bridge; GPIO line 0 its EN pin, line 1 the
 * display adapter's reset (logical, 1 = asserted). */

#define YD_SN65DSI8X_LVDS_24BPP   (1u << 0) /* CHA_24BPP_MODE */
#define YD_SN65DSI8X_FORMAT1      (1u << 1) /* CHA_24BPP_FORMAT1 ("JEIDA") */
#define YD_SN65DSI8X_TERM_100_OHM (1u << 2) /* CHA_LVDS_TERM = 0 */
#define YD_SN65DSI8X_HS_NEG       (1u << 3) /* LVDS-side polarities */
#define YD_SN65DSI8X_VS_NEG       (1u << 4)
#define YD_SN65DSI8X_DE_NEG       (1u << 5)
#define YD_SN65DSI8X_HAS_EN       (1u << 6) /* env line 0 exists */
#define YD_SN65DSI8X_HAS_RESET    (1u << 7) /* env line 1 exists */

/* yd_sn65dsi8x_health() bits, read from the part. */
#define YD_SN65DSI8X_HEALTH_ANSWERS     (1u << 0) /* the ID reads "58ISD   " */
#define YD_SN65DSI8X_HEALTH_PLL_EN_STAT (1u << 1) /* 0x0A.7 */
#define YD_SN65DSI8X_HEALTH_LINK_CLEAN  (1u << 2) /* 0xE5 reads 0 */
#define YD_SN65DSI8X_HEALTH_SINGLE_LINK (1u << 3) /* 0x18.4 reads 1 */

struct yd_sn65dsi8x_config {
	uint32_t flags;      /* YD_SN65DSI8X_* */
	uint32_t lanes;      /* DSI data lanes, 1..4 */
};

struct yd_sn65dsi8x_mode {
	uint32_t pixel_clock_hz;
	uint16_t hactive, vactive, hsync, vsync, hbp, vbp, hfp, vfp;
};

struct yd_sn65dsi8x_data {
	uint8_t id[9];          /* CSR 0x00..0x08 */
	uint8_t lvds_clk_range; /* 0x0A bits 3:1 */
	uint8_t dsi_clk_range;  /* 0x12 */
	uint8_t dsi_clk_div;    /* 0x0B bits 7:3 (divide - 1) */
	uint8_t lvds_fmt;       /* 0x18, the bits the driver owns */
	uint8_t irq_stat;       /* 0xE5 after the last link check */
	uint8_t started;
	uint8_t id_tries;
	uint32_t dsi_clk_khz;
	uint32_t lvds_clk_khz;
};

/* Data sheet Init seq 3-7 (EN cycle, ID, CSRs, PLL, soft reset), with the
 * host's DSI clock lane already in HS. lane_rate_hz: DSI bits/s per lane, 0
 * = pixel clock * 24 / lanes. */
int yd_sn65dsi8x_start(const struct yd_sn65dsi8x_config *cfg, struct yd_sn65dsi8x_data *data,
		       const struct yd_env *env, const struct yd_sn65dsi8x_mode *mode,
		       uint32_t lane_rate_hz);
/* Once video flows: 5 ms, IRQ_STAT <- 0xFF, 1 ms, read. Returns IRQ_STAT
 * (0 = clean) or a negative errno. */
int yd_sn65dsi8x_check_link(struct yd_sn65dsi8x_data *data, const struct yd_env *env);
/* EN low for the reset time. */
int yd_sn65dsi8x_stop(const struct yd_sn65dsi8x_config *cfg, struct yd_sn65dsi8x_data *data,
		      const struct yd_env *env);
/* Reads only. */
uint32_t yd_sn65dsi8x_health(const struct yd_env *env);
/* IRQ_STAT (0xE5) as it stands, read only: 0..255, or a negative errno. */
int yd_sn65dsi8x_irq_stat(const struct yd_env *env);

/* ---- PowerVR rgxlayer (gpu::pvr::rgxlayer) ------------------------------ *
 * The TI/Img DDK 25.2's rgxlayer.h, implemented in Rust under the vendored
 * rgxstartstop.c / rgxfwimageutils.c. The functions themselves are declared by
 * rgxlayer.h (DDK names and types); this is the context their `hPrivate`
 * points at. driver/compat/pvr_rgxlayer_check.c pins the DDK side (types,
 * error codes, the core's words below against the configuration headers). */

/* Register access hooks for a host check (addresses absolute: regs + offset).
 * NULL in struct yd_pvr_layer on the board. */
struct yd_pvr_bus {
	void *ctx;
	uint32_t (*read32)(void *ctx, uintptr_t addr);
	void (*write32)(void *ctx, uintptr_t addr, uint32_t val);
	void (*barrier)(void *ctx, uintptr_t addr);
};

/* hPrivate. Must stay writable (RGXSetPoweredState writes `powered`). */
struct yd_pvr_layer {
	uintptr_t regs;                 /* device_map() of the GPU registers */
	const struct yd_env *env;       /* busy wait, log (required) */
	const struct yd_pvr_bus *bus;   /* NULL: the hardware */
	uint64_t kernel_pc;             /* RGXAcquireKernelMMUPC */
	uint64_t fw_code_devva;         /* RGXAcquireBootCodeAddr */
	uint64_t fw_data_devva;         /* RGXAcquireBootDataAddr */
	uint32_t core_clock_hz;         /* RGXWaitCycles; 0: microseconds only */
	uint32_t poll_timeout_us;       /* RGXPollReg32/64; 0: 500000 */
	uint32_t poll_step_us;          /* 0: 50 */
	uint32_t powered;               /* RGXSetPoweredState */
};

/* The core's feature and erratum words (rgx_bvnc_defs_km.h bit positions, the
 * features of configs/rgxconfig_km_36.V.104.796.h, the errata of
 * cores/rgxcore_km_36.53.104.796.h); gpu::pvr::rgxlayer::FEATURES / ERNS_BRNS
 * hold the same numbers (a Rust test reads these two lines). */
#define YD_PVR_RGX_FEATURES 0x0000039a1d79af35ULL
#define YD_PVR_RGX_ERNS_BRNS 0x000000000008005aULL

/* ---- PowerVR kernel side (gpu::pvr::km, mem, mmu) ----------------------- *
 * Ours, against the vendored DDK types (docs/source-form.md step 7,
 * driver/docs/pvr-kernel-rust.md). One struct yd_pvr_km per GPU, owned by
 * driver/src/pvr_fw_boot.c; every entry point takes it. */

/* What the kernel side borrows from the OS. */
struct yd_pvr_km_ops {
	void *ctx;
	void (*flush)(void *ctx, const void *addr, size_t len); /* clean to memory */
	void (*invd)(void *ctx, const void *addr, size_t len);  /* invalidate */
	void (*fence)(void *ctx);                                /* barrier_dmem_fence_full() */
	void (*busy_wait_us)(void *ctx, uint32_t us);
	/* one line for the GPU driver's log (1 ERR .. 4 DBG), not NUL-terminated */
	void (*log)(void *ctx, uint32_t level, const uint8_t *buf, size_t len);
};

struct yd_pvr_span {
	uint64_t va;
	uint32_t span;
	uint32_t pad;
};

#define YD_PVR_PLACE_MAX 32
#define YD_PVR_FL_MAX 6
#define YD_PVR_FL_GROWS_MAX 64
/* gpu::pvr::render::STOP_WAIT_MS: a fence still for this long stops the
 * render service with the grow knob on (GPU_STOP cause 4). */
#define YD_PVR_STOP_WAIT_MS 10000

/* One freelist as the host knows it (gpu::pvr::freelist). */
struct yd_pvr_fl {
	uint32_t id;
	uint32_t fw_addr;               /* its RGXFWIF_FREELIST */
	uint32_t heap_off;
	uint32_t grow_threshold;         /* % of its pages kept as the firmware's ready pages; 0: none */
	uint32_t *arr;                  /* the PFN array */
	uint64_t backing_phys;          /* the first backing page */
	uint32_t npages, maxpages;      /* listed (the active stack), array size */
	uint32_t ready, grow;           /* ready pages, pages per grow */
	uint32_t backed, reserve;       /* backing pages listed so far / still unlisted */
	uint32_t grows;                 /* grows served */
	uint32_t pool_kib;              /* KiB taken from the render pool for on-demand grows */
};

struct yd_pvr_km {
	/* filled by the driver before the first call */
	uint8_t *cpu;                   /* the carveout, mapped */
	uint64_t phys;                  /* its physical base */
	uint64_t size;                  /* its size */
	const struct yd_pvr_km_ops *ops;
	const struct yd_pvr_bus *bus;   /* NULL: the hardware */
	uintptr_t regs;                 /* the GPU registers, mapped */
	uint32_t fw_main_size;          /* PVR_FW_HEAP_MAIN_PHYS */
	uint32_t fw_config_off;         /* PVR_FW_HEAP_CONFIG_OFFSET (a heap offset) */
	uint32_t pt_area_off;           /* PVR_PT_AREA_OFFSET */
	uint32_t pt_area_end;           /* PVR_PT_AREA_END */
	uint32_t pool_off;              /* PVR_RENDER_POOL_OFFSET */
	/* state; the driver sets pt_next = pt_area_off, pool_next = pool_off */
	uint32_t heap_next;             /* firmware-heap bump (heap offset) */
	uint32_t pt_next;               /* page-table bump (carveout offset) */
	uint32_t pool_next;             /* render-pool bump (carveout offset) */
	uint32_t pc_off;                /* the page catalogue (0: not built) */
	uint32_t mmu_dirty;
	uint32_t place_n;
	struct yd_pvr_span place[YD_PVR_PLACE_MAX];
	/* the CCBs (gpu::pvr::ccb): heap offsets, set when they are built */
	uint32_t kccb_ctl_off, kccb_cmds_off, kccb_rtn_off;
	uint32_t fwccb_ctl_off, fwccb_cmds_off;
	uint32_t kccb_posts;
	/* the freelists (gpu::pvr::freelist) */
	uint32_t pm_bump;               /* PM-META protect heap cursor */
	uint32_t fl_reserve_mult;       /* grows a list keeps of its own (no pool); the driver sets 4 */
	uint32_t fl_pool_pages, fl_pool_used, fl_pool_peak;
	uint32_t fl_no_grow;            /* grow requests refused */
	uint32_t fl_grow_reqs, fl_grows_done;
	uint32_t nfls;
	uint32_t fl_ready_threshold;     /* grow_threshold of the next growable list built (0: none) */
	uint64_t fl_pool_phys;
	struct yd_pvr_fl fls[YD_PVR_FL_MAX];
	/* the FWIF structures (gpu::pvr::fwinit): heap offsets */
	uint32_t sysinit_off;
	uint32_t tracebuf_off;          /* RGXFWIF_TRACEBUF */
	uint32_t trace_buf_off;         /* its thread-0 ring */
	uint32_t hwrinfobuf_off;
	/* the grows' pages and the firmware's notices (gpu::pvr::freelist) */
	uint32_t fl_grow_pfn[YD_PVR_FL_MAX][YD_PVR_FL_GROWS_MAX]; /* each grow's first PFN, per list */
	uint32_t fl_spare_pfn[YD_PVR_FL_MAX]; /* a refilled one-grow reserve (0: none) */
	uint32_t fl_refill_refused_kib;  /* KiB the refills asked for and did not get */
	uint32_t fl_refills_refused;
	uint32_t fw_stats[7];            /* UPDATE_STATS notices by element (0: other) */
	/* why the render service stopped taking kicks (gpu::pvr::freelist
	 * STOP_*; 0: it has not), the list and KiB the cause names, the fence a
	 * wait found it on, and whether the wait said so */
	uint32_t stop_cause, stop_fl, stop_kib, stop_fence, stop_said;
	/* the FWCCB check (CONFIG_SAFEUI_GPU_PVR_FWCCB_CHECK, gpu::pvr::ccb):
	 * the switch, the last write offset read (and whether there is one),
	 * slots read without the command magic and those filled within the
	 * bound, and times the write offset went back */
	uint32_t fwccb_check, fwccb_last_wo, fwccb_wo_seen;
	uint32_t fwccb_empty, fwccb_filled, fwccb_wo_back;
	/* CONFIG_SAFEUI_GPU_PVR_FW_UNCACHED (gpu::pvr::mem heap_fwaddr): 1 hands
	 * the firmware the uncached window for the host's objects */
	uint32_t fw_uncached;
	uint32_t fl_probe_serve_max;    /* probe: refuse grows after this many served (0: off) */
	/* the render pool's top region (gpu::pvr::mem bo_alloc_top, gpu::pvr::pool):
	 * GENERAL requests of more than pool_top_above bytes are placed from the
	 * carveout's end downward (0: off); the bytes handed out from the top
	 * now and at most */
	uint32_t pool_top_above;
	uint32_t pool_top, pool_top_hw;
	uint32_t pad;
};
/* The driver's pool_top_above: 256 KiB
 * (docs/notes/pvr-render-pool.md, the top region). */
#define YD_PVR_POOL_TOP_ABOVE (256u * 1024u)

/* What the driver chooses for the FWIF structures (gpu::pvr::fwinit). */
#define YD_PVR_FWINIT_ASSERT_ON_HWR (1u << 0) /* halt, do not reset, on a HWR */
#define YD_PVR_FWINIT_CTXSWITCH_OFF (1u << 1) /* no context switching */
struct yd_pvr_fwinit {
	uint32_t pm_latency_ms;         /* RGXFWIF_RUNTIME_CFG.ui32ActivePMLatencyms */
	uint32_t core_clock_hz;         /* the core (and SOC) clock the firmware is told */
	uint32_t flags;                 /* YD_PVR_FWINIT_* */
	/* 0: the DDK structure's size (the driver); the host check's former sizes otherwise */
	uint32_t ccb_ctl_size;
	uint32_t tracebuf_size;
	uint32_t gpu_util_size;
};

/* struct pvr_gpu_bo (driver/src/pvr_types.h), the same layout. */
struct yd_pvr_bo {
	uint64_t gpu;
	uint8_t *cpu;
	uint32_t heap_off;
};

/* gpu::pvr::mem: the firmware heap, the page-table pages, the render pool. */
uint32_t yd_pvr_heap_alloc(struct yd_pvr_km *km, uint32_t size, uint32_t align);
uint8_t *yd_pvr_heap_cpu(const struct yd_pvr_km *km, uint32_t off);
uint32_t yd_pvr_heap_fwaddr(const struct yd_pvr_km *km, uint32_t off);
uint64_t yd_pvr_heap_gpu(const struct yd_pvr_km *km, uint32_t off);
uint64_t yd_pvr_heap_phys(const struct yd_pvr_km *km, uint32_t off);
struct yd_pvr_bo yd_pvr_bo_alloc(struct yd_pvr_km *km, uint64_t heap_base, uint32_t *heap_bump,
				 uint32_t size, uint64_t l0flags);
struct yd_pvr_bo yd_pvr_bo_alloc_pm(struct yd_pvr_km *km, uint64_t heap_base, uint32_t *heap_bump,
				    uint32_t size);
bool yd_pvr_bo_rollback(struct yd_pvr_km *km, uint32_t *heap_bump, uint32_t heap_off,
			const uint8_t *cpu, uint32_t size);
struct yd_pvr_bo yd_pvr_bo_place(struct yd_pvr_km *km, uint64_t heap_base, uint32_t start_off,
				 uint32_t size, uint64_t l0flags);
void yd_pvr_pool_align(struct yd_pvr_km *km, uint32_t align);
uint64_t yd_pvr_bo_phys(const struct yd_pvr_km *km, const struct yd_pvr_bo *bo);

/* gpu::pvr::pool: the render pool's free lists, one per heap the render
 * service serves (GENERAL, PDS, USC, enum pvr_render_heap order), and the
 * pool's summary lines. The struct is the render service's (zero before the
 * first call) and opaque here; `bumps` are its three device-VA cursors. */
#define YD_PVR_POOL_BYTES 6304
struct yd_pvr_pool {
	uint64_t opaque[YD_PVR_POOL_BYTES / 8];
};
/* 0, -EINVAL (a bad heap or size), -ENOMEM (exhausted: the bump's line and
 * the summary at ERR are logged) */
int yd_pvr_pool_alloc(struct yd_pvr_pool *pool, struct yd_pvr_km *km, uint32_t *const bumps[3],
		      uint32_t heap, uint32_t size, struct yd_pvr_bo *out);
void yd_pvr_pool_free(struct yd_pvr_pool *pool, struct yd_pvr_km *km, uint32_t *const bumps[3],
		      const struct yd_pvr_bo *bo, uint32_t size, bool rollback);
/* the summary at INF: the run-end report */
void yd_pvr_pool_report(struct yd_pvr_pool *pool, struct yd_pvr_km *km, uint32_t *const bumps[3]);
/* out[0]: bytes the physical cursor handed out; out[1]: bytes free in the lists */
void yd_pvr_pool_usage(struct yd_pvr_pool *pool, struct yd_pvr_km *km, uint32_t *const bumps[3], uint64_t *out);
/* dropped[4] (KiB per heap, then no heap), slots_peak[3], ovf_peak[3],
 * ovf_now[3]; any pointer may be NULL */
void yd_pvr_pool_counters(const struct yd_pvr_pool *pool, uint32_t *dropped, uint32_t *slots_peak,
			  uint32_t *slots_cap, uint32_t *ovf_peak, uint32_t *ovf_now, uint32_t *bad);

/* gpu::pvr::mmu: the page tables (rgxmmudefs_km.h formats). */
void yd_pvr_mmu_init(struct yd_pvr_km *km);
void yd_pvr_mmu_map(struct yd_pvr_km *km, uint64_t va, uint64_t phys, uint64_t size, uint64_t l0flags);
void yd_pvr_mmu_map_fw_heap(struct yd_pvr_km *km);
bool yd_pvr_mmu_take_dirty(struct yd_pvr_km *km);
uint64_t yd_pvr_mmu_lookup(const struct yd_pvr_km *km, uint64_t va);
uint32_t yd_pvr_mmu_audit(const struct yd_pvr_km *km, const uint64_t *lo, const uint64_t *hi, uint32_t n);
uint64_t yd_pvr_mmu_pc_phys(const struct yd_pvr_km *km);

/* gpu::pvr::ccb: the kernel CCB (post: slot; cmd is sizeof(RGXFWIF_KCCB_CMD)
 * bytes), the doorbell, the slot probe (print only), the firmware CCB. */
uint32_t yd_pvr_kccb_post(struct yd_pvr_km *km, const uint8_t *cmd);
void yd_pvr_kccb_doorbell(const struct yd_pvr_km *km);
void yd_pvr_kccb_probe_slot(const struct yd_pvr_km *km, uint32_t slot, const char *why);
int yd_pvr_fwccb_service(struct yd_pvr_km *km);

/* gpu::pvr::freelist: the grow pool, a freelist (returns the heap offset of
 * its RGXFWIF_FREELIST), the PFN-array audit. */
void yd_pvr_fl_pool_init(struct yd_pvr_km *km, uint32_t *gen_b, uint32_t pages);
uint32_t yd_pvr_fl_build(struct yd_pvr_km *km, uint32_t *gen_b, uint32_t npages, uint32_t maxpages,
			 uint32_t grow, uint32_t ready, uint32_t fl_id);
/* CONFIG_SAFEUI_GPU_PVR_FL_READY_PAGES: on (non-zero) builds the next growable
 * lists with Mesa's global grow threshold (gpu::pvr::freelist::ready_pages). */
void yd_pvr_fl_set_ready_pages(struct yd_pvr_km *km, uint32_t on);
uint32_t yd_pvr_fl_audit(const struct yd_pvr_km *km);

/* gpu::pvr::fwinit: the structures the firmware reads at boot (the image
 * loaded, km->heap_next where the host's objects start), and the wait for its
 * bFirmwareStarted (0, -ETIMEDOUT after 5 s with the trace and assert dumped). */
void yd_pvr_fwinit_build(struct yd_pvr_km *km, const struct yd_pvr_fwinit *cfg);
int yd_pvr_fw_wait_started(struct yd_pvr_km *km);

/* ---- the render service's firmware side (gpu::pvr::render) ------------- *
 * The render context, the render-target datasets, the client CCBs, the
 * submit and the fence wait; owned by driver/src/pvr_render.c (the static
 * PDS/USC areas, the pool and the Zephyr-facing API stay there). */

#define YD_PVR_HWRT_SETS 32

/* One render-target dataset (an HWRT data and its buffers). */
struct yd_pvr_hwrt {
	uint32_t used, w, h, samples;
	uint32_t hcmn_off, hrt_off, hrt_fwaddr, pad;
	struct yd_pvr_bo tpc, vheap, rgnhdr, macrotile, mlist;
	uint32_t tpc_alloc, rgn_alloc;  /* bytes behind tpc / rgnhdr */
	uint32_t fw_built;              /* the HWRT data structures exist */
	uint32_t last_seq;              /* the last kick that used it */
	uint32_t next;                  /* the pair member the next render takes (even slot) */
	uint32_t pad2;
};

/* The run-time switches (pvr_render.c has what each is for), 0/1 but
 * mmu_ack_reping (ms). */
struct yd_pvr_render_cfg {
	uint32_t combined_kick, wait_rering, wait_repost, mmu_ack, mmu_gate;
	uint32_t mmu_ack_probe, mmu_ack_reping, hwrt_refuse, hwrt_cleanup, hwrt_reinit;
	uint32_t verbose;               /* PVR_RENDER_VERBOSE */
	uint32_t fl_grow_on_demand;     /* serve a freelist grow from the render pool */
};

struct yd_pvr_render_ops {
	void *ctx;
	/* give a render-pool block back (the pool's free lists) */
	void (*free_bo)(void *ctx, const struct yd_pvr_bo *bo, uint32_t size);
};

/* gpu::pvr::kick_times (feature pvr-kick-times): one kick's stamps in
 * ARM generic-timer ticks, 0 = not yet. A "seen" time comes with the last
 * read that did not see the fence (the error bar). */
#define YD_PVR_KT_RING 8
struct yd_pvr_kick_stamp {
	uint64_t t_submit, t_posted;
	uint64_t t_geom_miss, t_geom, t_frag_miss, t_frag;
	uint64_t t_wait;                /* the first wait for its fence */
	uint64_t mmu_ticks;             /* the pre-kick MMU cache invalidate */
	uint32_t seq;                   /* 0: free */
	uint32_t mmu_synced;
};

struct yd_pvr_kick_times {
	uint32_t hz, done, lost, pad;
	uint64_t prev_frag;
	struct yd_pvr_kick_stamp last;
	struct yd_pvr_kick_stamp ring[YD_PVR_KT_RING];
};

/* Where the application's current scene began (gpu::pvr::render, feature
 * pvr-grow-scene): scenes marked, the last kick submitted then, the clock. */
struct yd_pvr_grow_scene {
	uint32_t n, seq0;
	uint64_t t0;
};

/* The last completed kick in microseconds (yd_pvr_render_last_kick_us). */
#define YD_PVR_KICK_US_GEOM_SEEN (1u << 0)  /* kick_geom_us, geom_err_us valid */
#define YD_PVR_KICK_US_PREV (1u << 1)       /* frag_frag_us valid */
#define YD_PVR_KICK_US_WAITED (1u << 2)     /* wait_us valid */
struct yd_pvr_kick_us {
	uint32_t seq;
	uint32_t submit_us;             /* submit entry to the kick posted */
	uint32_t mmu_us, mmu_synced;
	uint32_t kick_geom_us, geom_err_us;   /* the event lies in [v - err, v] */
	uint32_t kick_frag_us, frag_err_us;
	uint32_t frag_frag_us;          /* the previous kick's fragment fence to this one's */
	uint32_t wait_us;               /* the first wait for it to the fence seen */
	uint32_t flags;
	uint32_t done, lost;
};

struct yd_pvr_render {
	struct yd_pvr_render_cfg cfg;
	const struct yd_pvr_render_ops *ops;
	uint32_t inited;
	uint32_t gen_b;                 /* the general heap's device-VA cursor */
	uint32_t gfl_off, gfl2_off, fl_off;   /* the freelists, set by the C */
	uint32_t fmc_off, rc_off;
	uint32_t gcccb_off, gcccbctl_off, gstate_off, gufo_off;
	uint32_t fcccb_off, fcccbctl_off, fstate_off, fufo_off;
	uint32_t gwo, fwo;              /* client CCB write offsets */
	uint32_t mc_sync_off, mc_seq;
	uint32_t seq;                   /* the completion fence sequence */
	uint32_t hwr_seen;
	int32_t last_hs;                /* the dataset the last kick used, -1 none */
	uint64_t dbg_vdm;
	struct yd_pvr_bo vdm_cs;
	uint32_t reposts, hwrt_busy_reuse, stall_dumped, stall_dumped_after_refusal;
	uint32_t mc_unseen, mc_io_armed, mc_io_want, mc_io_slot;
	uint32_t last_kick_n, last_frag_seq;
	uint32_t last_frag_cmd[80];
	uint8_t last_kick[2][160];
	struct yd_pvr_hwrt hwrt[YD_PVR_HWRT_SETS];
	struct yd_pvr_kick_times kick_times;   /* zero without pvr-kick-times */
	struct yd_pvr_grow_scene grow_scene;   /* zero without pvr-grow-scene */
};

/* kick: a struct pvr_render_kick (driver/src/pvr_render.h; 208 bytes). */
void yd_pvr_render_ctx_init(struct yd_pvr_km *km, struct yd_pvr_render *r);
int yd_pvr_render_submit(struct yd_pvr_km *km, struct yd_pvr_render *r, const void *kick,
			 uint32_t *fence_out);
int yd_pvr_render_wait(struct yd_pvr_km *km, struct yd_pvr_render *r, uint32_t fence,
		       uint32_t timeout_ms, const char *who);
uint32_t yd_pvr_render_completed(const struct yd_pvr_km *km, const struct yd_pvr_render *r);
/* the firmware's HWR counter (recoveries since it booted) */
uint32_t yd_pvr_render_hwr_counter(const struct yd_pvr_km *km);
/* the run-end line of freelist grows and GPU recoveries since `since` */
void yd_pvr_render_grow_report(struct yd_pvr_km *km, uint32_t since);
/* The driver's owner lines (statics: page-rounded bytes per heap [general,
 * pds, usc] of the C side's one-time objects, nstatics blocks); returns KiB. */
uint64_t yd_pvr_render_pool_owners(struct yd_pvr_km *km, const struct yd_pvr_render *r,
				   const uint32_t *statics, uint32_t nstatics);
/* 0; -ENOTSUP built without pvr-kick-times; -ENODATA before the first kick
 * completed. */
int yd_pvr_render_last_kick_us(const struct yd_pvr_render *r, struct yd_pvr_kick_us *out);
/* The application's scene starts now (pvr-grow-scene; nothing without it). */
void yd_pvr_render_scene_begin(struct yd_pvr_render *r);
/* Grow requests, served, refused, out-of-memory notices, FWCCB slots read
 * empty and filled, re-posts, stop cause, partial renders
 * (gpu::pvr::render::fl_counters). */
#define YD_PVR_FL_COUNTERS 9
void yd_pvr_render_fl_counters(const struct yd_pvr_km *km, const struct yd_pvr_render *r, uint32_t *out);
/* The host check's MH tag table (rgxmhdefs_km.h encoding names, in order). */
bool yd_pvr_mh_tag(uint32_t i, const uint8_t **name, size_t *len);

/* gpu::pvr::fwif: the host check's pin table (C expression, value) and the
 * alignment-check table the firmware is given. */
bool yd_pvr_fwif_pin(uint32_t i, const uint8_t **expr, size_t *len, uint64_t *val);
bool yd_pvr_fwif_align_check(uint32_t i, uint32_t *val);

/* ---- i.MX 8M Plus display power domains (soc::imx8mp::media;
 *      yakogl,imx8mp-media-blk-ctrl) -------------------------------------- */

#define YD_IMX8MP_MEDIA_PD_LCDIF1    0u
#define YD_IMX8MP_MEDIA_PD_MIPI_DSI1 1u

/* yd_imx8mp_media_status() bits: GPC, CCM and ANATOP read live,
 * MEDIA_BLK_CTRL as last sampled while MEDIAMIX was up. */
#define YD_MEDIA_ST_MEDIAMIX_MAPPED     (1u << 0)
#define YD_MEDIA_ST_MIPI_PHY1_MAPPED    (1u << 1)
#define YD_MEDIA_ST_MEDIAMIX_PUP_CLEAN  (1u << 2)  /* no trigger pending or failed */
#define YD_MEDIA_ST_MIPI_PHY1_PUP_CLEAN (1u << 3)
#define YD_MEDIA_ST_ADB_REQ             (1u << 4)
#define YD_MEDIA_ST_ADB_ACK             (1u << 5)
#define YD_MEDIA_ST_PCR_CLEAR           (1u << 6)
#define YD_MEDIA_ST_POWERED_FROM_OFF    (1u << 7)  /* PSR was 1 at our power-up */
#define YD_MEDIA_ST_MEDIA_AXI_ROOT      (1u << 8)
#define YD_MEDIA_ST_MEDIA_APB_ROOT      (1u << 9)
#define YD_MEDIA_ST_MEDIA_GATE          (1u << 10)
#define YD_MEDIA_ST_DISP1_PIX_ROOT      (1u << 11)
#define YD_MEDIA_ST_PHY1_REF_ROOT       (1u << 12)
#define YD_MEDIA_ST_VIDEO_PLL1_RUNNING  (1u << 13)
#define YD_MEDIA_ST_SYS_PLL1_RUNNING    (1u << 14)
#define YD_MEDIA_ST_SYS_PLL2_RUNNING    (1u << 15)
#define YD_MEDIA_ST_BLK_BUS             (1u << 16) /* clocked and out of reset */
#define YD_MEDIA_ST_BLK_LCDIF1          (1u << 17)
#define YD_MEDIA_ST_BLK_MIPI_DSI1       (1u << 18)
#define YD_MEDIA_ST_BLK_PHY             (1u << 19) /* GPR_MIPI_M_RESETN */
#define YD_MEDIA_ST_BLK_HURRY           (1u << 20)
#define YD_MEDIA_ST_BLK_SAMPLED         (1u << 21)
/* bits 31:24: the last operation: step 27:24 (1 bus roots, 2 gate,
 * 3 MEDIAMIX, 4 BLK bus, 5 ADB ack, 6 hurry, 7 MIPI_PHY1, 8 block clocks,
 * 9 on, 10 reset pulse, 11 off, 12 VIDEO_PLL1, 13 pixel root, 14 PHY ref),
 * failed 28, error 31:29 (1 timeout, 2 I/O, 3 invalid, 4 other). */
#define YD_MEDIA_ST_LAST_SHIFT          24

/* Whether an LCDIF1 register access would be answered now (soc::imx8mp::
 * media::lcdif_unreachable): 0, or why not. Reads the GPC, the CCM and,
 * only when those say it answers, MEDIA_BLK_CTRL; writes nothing. For a
 * caller with its own mappings, at any init level. */
#define YD_LCDIF_UNREACH_MEDIAMIX  (1u << 0) /* MEDIAMIX not on (GPC) */
#define YD_LCDIF_UNREACH_APB_ROOT  (1u << 1) /* MEDIA_APB_CLK_ROOT disabled */
#define YD_LCDIF_UNREACH_GATE      (1u << 2) /* CCGR93 MEDIA closed */
#define YD_LCDIF_UNREACH_LCDIF_APB (1u << 3) /* MEDIA_BLK_CTRL: LCDIF APB clock off or in reset */
uint32_t yd_imx8mp_lcdif_unreachable(uintptr_t gpc_base, uintptr_t ccm_base, uintptr_t blk_base);

struct yd_imx8mp_media_data {
	uintptr_t blk_base;    /* device_map() of MEDIA_BLK_CTRL, GPC, CCM, CCM_ANALOG */
	uintptr_t gpc_base;
	uintptr_t ccm_base;
	uintptr_t anatop_base;
	uint32_t users;        /* sub-domains on (bit = YD_IMX8MP_MEDIA_PD_*) */
	uint32_t sampled;
	uint32_t last;
	uint32_t reserved;
};

int yd_imx8mp_media_pd_on(struct yd_imx8mp_media_data *data, const struct yd_env *env,
			  uint32_t pd);
int yd_imx8mp_media_pd_off(struct yd_imx8mp_media_data *data, const struct yd_env *env,
			   uint32_t pd);
int yd_imx8mp_media_pd_reset(struct yd_imx8mp_media_data *data, const struct yd_env *env,
			     uint32_t pd);
/* VIDEO_PLL1 to video_pll1_hz (0: leave it), the pixel clock root nearest
 * want_hz; the rate in *out_hz. */
int yd_imx8mp_media_set_pixel_clock(struct yd_imx8mp_media_data *data, const struct yd_env *env,
				    uint32_t want_hz, uint32_t video_pll1_hz, uint32_t *out_hz);
/* What the pixel clock root produces now; 0 when unknown. */
uint32_t yd_imx8mp_media_pixel_clock(const struct yd_imx8mp_media_data *data);
/* MEDIA_MIPI_PHY1_REF = 24 MHz / n at most hz; the rate, 0 on error. */
uint32_t yd_imx8mp_media_set_phy_ref_clock(struct yd_imx8mp_media_data *data, uint32_t hz);
uint32_t yd_imx8mp_media_status(const struct yd_imx8mp_media_data *data);
/* Roots: ENABLE only; gates: domain 0 on. Registers written, or -errno. */
int yd_imx8mp_ccm_early_clocks(uintptr_t ccm_base, const uint32_t *roots, size_t n_roots,
			       const uint32_t *gates, size_t n_gates);

/* ---- i.MX 8M Plus HDMI display (soc::imx8mp::hdmi_chain; yakogl,imx8mp-hdmi)
 * LCDIF3 -> PVI -> DesignWare HDMI TX -> HDMI PHY, brought up from cold or,
 * without FORCE_COLD, adopted when found scanning. driver/docs/x2-hdmi-cold.md. */

#define YD_IMX8MP_HDMI_HSYNC_HIGH   (1u << 0)  /* sync polarities at the connector */
#define YD_IMX8MP_HDMI_VSYNC_HIGH   (1u << 1)
#define YD_IMX8MP_HDMI_DE_HIGH      (1u << 2)
#define YD_IMX8MP_HDMI_PXCK_RISING  (1u << 3)  /* LCDIF CTRL[INV_PXCK] */
#define YD_IMX8MP_HDMI_FORCE_COLD   (1u << 4)  /* cold even when LCDIF3 scans */
#define YD_IMX8MP_HDMI_POWER_CYCLE  (1u << 5)  /* HDMIMIX + PHY off first if on */
#define YD_IMX8MP_HDMI_PIPE_SEL_SET (1u << 6)  /* write RTX_CLK_CTL1[HTX_PIPE_CLK_SEL] */
#define YD_IMX8MP_HDMI_PIPE_SEL_ONE (1u << 7)  /* ... to 1 (else 0) */
#define YD_IMX8MP_HDMI_PHY_SEL_SET  (1u << 8)  /* write RTX_CLK_CTL1[HTXPHY_CLK_SEL] */
#define YD_IMX8MP_HDMI_PHY_SEL_ONE  (1u << 9)
#define YD_IMX8MP_HDMI_CONF0_SET    (1u << 10) /* write phy_conf0 */
#define YD_IMX8MP_HDMI_PHYRSTZ_SET  (1u << 11) /* write mc_phyrstz */
#define YD_IMX8MP_HDMI_TEST_FORCED  (1u << 12) /* staging: controller's forced video */
#define YD_IMX8MP_HDMI_TEST_BGND    (1u << 13) /* staging: LCDIF background colour */

/* yd_imx8mp_hdmi_health() bits; bit 0 is IMX8MP_DISPLAY_HEALTH_LCDIF_RUNNING. */
#define YD_IMX8MP_HDMI_H_LCDIF_RUNNING  (1u << 0)
#define YD_IMX8MP_HDMI_H_HDMIMIX_UP     (1u << 8)
#define YD_IMX8MP_HDMI_H_BLK_ENABLED    (1u << 9)
#define YD_IMX8MP_HDMI_H_PHY_PLL_LOCK   (1u << 10)
#define YD_IMX8MP_HDMI_H_PHY_READY      (1u << 11)
#define YD_IMX8MP_HDMI_H_HPD            (1u << 12)
#define YD_IMX8MP_HDMI_H_TMDS_CLOCK     (1u << 13)
#define YD_IMX8MP_HDMI_H_PIXEL_CLOCK    (1u << 14)
#define YD_IMX8MP_HDMI_H_PVI_ON         (1u << 15)
#define YD_IMX8MP_HDMI_H_PVI_FIFO_ERROR (1u << 16)
#define YD_IMX8MP_HDMI_H_RX_SENSE       (1u << 17)

/* Stages (stop_after; data.stage): 1 start, 2 power, 3 block control,
 * 4 PHY PLL locked, 5 controller, 6 PVI, 7 LCDIF3 scanning, 8 video check. */

struct yd_imx8mp_hdmi_config {
	uint32_t pixel_clock_hz;
	uint32_t hactive, hfront, hsync, hback;
	uint32_t vactive, vfront, vsync, vback;
	uint32_t flags;        /* YD_IMX8MP_HDMI_* */
	uint32_t max_ppm;      /* pixel clock error accepted from the integer PLL */
	uint32_t desc3_extra;  /* LCDIF CTRLDESCL0_3 bits above PITCH (0 = RM only) */
	uint32_t noc_hurry;    /* TX_CONTROL0[LCDIF_NOC_HURRY], 0..7 */
	uint32_t panic_low;    /* LCDIF PANIC0_THRES, both 0 = not set */
	uint32_t panic_high;
	uint32_t stop_after;   /* 0 = run to the end */
	uint32_t test_rgb;     /* TEST_BGND colour, 0xRRGGBB */
	uint8_t phy_conf0;     /* with CONF0_SET */
	uint8_t phyrstz;       /* with PHYRSTZ_SET */
	uint8_t reserved[2];
};

struct yd_imx8mp_hdmi_data {
	uintptr_t lcdif;       /* device_map() of LCDIF3, the TX controller, the PHY, */
	uintptr_t hdmi;        /* the HDMI block control, the PVI, GPC, CCM */
	uintptr_t phy;
	uintptr_t blk;
	uintptr_t pvi;
	uintptr_t gpc;
	uintptr_t ccm;
	uint64_t fb;           /* the buffer we scan */
	uint64_t prev_fb;      /* adopted: the previous owner's buffer and layout */
	uint32_t prev_desc3;
	uint32_t prev_desc5;
	uint32_t width, height, pitch;
	uint32_t tmds_hz;      /* the PLL's rate (= pixel clock at 8 bpc) */
	int32_t err_ppm;
	uint32_t pll_pms;      /* P | M << 8 | S << 16 */
	uint32_t refresh_mhz;
	uint32_t stage;        /* reached, or failed in */
	int32_t error;
	uint8_t on;
	uint8_t adopted;
	uint8_t reserved[2];
};

/* Start scanning fb_phys (XRGB8888, pitch bytes per line, 0 = width * 4).
 * -ECANCELED after stop_after; data->stage says where it stopped or failed. */
int yd_imx8mp_hdmi_on(const struct yd_imx8mp_hdmi_config *cfg, struct yd_imx8mp_hdmi_data *data,
		      const struct yd_env *env, uint64_t fb_phys, uint32_t pitch);
/* Flip: the address and a shadow load, waited for (up to two frames). */
int yd_imx8mp_hdmi_set_fb(struct yd_imx8mp_hdmi_data *data, const struct yd_env *env,
			  uint64_t fb_phys);
/* Adopted: give the previous owner's buffer back. Cold: stop scanning, stay
 * powered. */
void yd_imx8mp_hdmi_off(struct yd_imx8mp_hdmi_data *data, const struct yd_env *env);
/* Read only; touches a block only when its domain, clock and reset allow. */
uint32_t yd_imx8mp_hdmi_health(const struct yd_imx8mp_hdmi_data *data);
uint64_t yd_imx8mp_hdmi_scanout(const struct yd_imx8mp_hdmi_data *data);
bool yd_imx8mp_hdmi_is_running(const struct yd_imx8mp_hdmi_data *data);
uint32_t yd_imx8mp_hdmi_layer_control(const struct yd_imx8mp_hdmi_data *data);

/* ---- i.MX 8M Plus LCDIFv3 (ip::lcdifv3; yakogl,imx8mp-lcdif) and the
 * Samsung MIPI DSI host (ip::samsung_dsim; yakogl,samsung-dsim): the Verdin's
 * LCDIF1 -> DSIM -> SN65DSI84 chain, and any LCDIFv3 adopted as it runs.
 * driver/docs/imx8mp-display-cold.md. */

#define YD_LCDIF_PXCK_RISING (1u << 0)  /* CTRL[INV_PXCK]: consumer samples on the rising edge */
#define YD_LCDIF_HSYNC_LOW   (1u << 1)
#define YD_LCDIF_VSYNC_LOW   (1u << 2)
#define YD_LCDIF_DE_LOW      (1u << 3)
#define YD_LCDIF_ADOPT_ONLY  (1u << 4)  /* never configure: adopt a running layer or nothing */
#define YD_LCDIF_PANIC       (1u << 5)  /* panic_low / panic_high are set */

/* INT_STATUS_D0 bits (yd_lcdif_isr) */
#define YD_LCDIF_IRQ_VSYNC    (1u << 0)
#define YD_LCDIF_IRQ_UNDERRUN (1u << 1)
#define YD_LCDIF_IRQ_VS_BLANK (1u << 2)

struct yd_lcdif_config {
	uint32_t flags;        /* YD_LCDIF_* */
	uint32_t hactive;      /* the size the layer runs / is expected to run */
	uint32_t vactive;
	uint32_t desc3_extra;  /* CTRLDESCL0_3 bits above PITCH (risk R1; 0 = RM / SDK) */
	uint16_t panic_low;    /* PANIC0_THRES, 128-bit FIFO units, with YD_LCDIF_PANIC */
	uint16_t panic_high;
};

struct yd_lcdif_mode {
	uint32_t pixel_clock_hz;
	uint32_t hactive, hfront, hsync, hback;
	uint32_t vactive, vfront, vsync, vback;
};

struct yd_lcdif_data {
	uintptr_t base;        /* device_map() of the LCDIF */
	uint64_t fb;           /* the buffer scanned now */
	uint64_t prev_fb;      /* adopted: the previous owner's buffer and layout */
	uint32_t prev_desc3;
	uint32_t prev_desc5;
	uint32_t our_pitch;
	uint32_t pitch;
	uint32_t width, height;
	uint32_t irq_status;   /* INT_STATUS_D0 of the last interrupt */
	uint8_t on;
	uint8_t adopted;
	uint8_t held;          /* adopted: the previous owner's layer captured */
	uint8_t ours;          /* adopted: our pitch and format in place */
};

struct yd_lcdif_scanout {
	uint32_t width, height, pitch, bytes_pp;
	uint32_t bpp_code;     /* CTRLDESCL0_5[BPP]: 0x4 RGB565, 0x9 ARGB8888, 0xA ABGR8888 */
	uint32_t reserved;
	uint64_t phys;
};

/* Timing and layer, display and layer off (-ENODEV with YD_LCDIF_ADOPT_ONLY). */
int yd_lcdif_configure(const struct yd_lcdif_config *cfg, struct yd_lcdif_data *data,
		       const struct yd_lcdif_mode *mode, uint64_t fb_phys, uint32_t pitch);
/* Panic thresholds, the VS_BLANK interrupt, DISP_ON, the layer; waits for the
 * first shadow load (-ETIMEDOUT: no pixel clock). */
int yd_lcdif_enable(const struct yd_lcdif_config *cfg, struct yd_lcdif_data *data,
		    const struct yd_env *env, bool vblank_irq);
/* Take over a running layer of cfg's size; fb_phys 0 writes nothing. */
int yd_lcdif_adopt(const struct yd_lcdif_config *cfg, struct yd_lcdif_data *data,
		   const struct yd_env *env, uint64_t fb_phys, uint32_t pitch);
/* Flip (adopted: the first buffer brings our pitch and XRGB8888). */
int yd_lcdif_set_fb(struct yd_lcdif_data *data, const struct yd_env *env, uint64_t fb_phys);
/* Adopted: hand the previous owner's layer back. Cold: stop, display off. */
int yd_lcdif_release(struct yd_lcdif_data *data, const struct yd_env *env);
/* Stateless, on a mapping alone (the panel console at PRE_KERNEL_1). */
bool yd_lcdif_is_running(uintptr_t base);
uint32_t yd_lcdif_layer_control(uintptr_t base);
bool yd_lcdif_read_scanout(uintptr_t base, struct yd_lcdif_scanout *out);
/* A flip of a running layer on a mapping alone, waited for with the timer. */
int yd_lcdif_flip(uintptr_t base, const struct yd_env *env, uint64_t fb_phys);
/* A flip whose wait counts register reads (no timer yet). */
int yd_lcdif_flip_polled(uintptr_t base, uint64_t fb_phys, uint32_t max_reads);
/* The interrupt: INT_STATUS_D0 read and cleared (YD_LCDIF_IRQ_*). */
uint32_t yd_lcdif_isr(struct yd_lcdif_data *data);

#define YD_SAMSUNG_DSIM_BURST (1u << 0)

/* ip::samsung_dsim::LINK_MARGIN_PERMILLE: the lane rate must be at least
 * pixel clock x 24 / lanes x (1000 + this) / 1000, the room for every line's
 * packet overhead (the board's measured bound: 0 broke the picture twice,
 * 25.97 per mille is the plan it was seen right at). A compile-time figure
 * so a devicetree plan can be checked with BUILD_ASSERT. */
#define YD_SAMSUNG_DSIM_LINK_MARGIN_PERMILLE 25u

/* yd_samsung_dsim_health() bits */
#define YD_SAMSUNG_DSIM_H_PLL_STABLE (1u << 0)
#define YD_SAMSUNG_DSIM_H_HS_CLOCK   (1u << 1)
#define YD_SAMSUNG_DSIM_H_HELD       (1u << 2)  /* data lanes held: attached, video not started */

/* Steps (data.step): 1 plan, 2 PLL, 3 software reset, 4 lanes, 5 held, 6 video. */

struct yd_samsung_dsim_config {
	uint32_t pll_ref_hz;   /* MEDIA_MIPI_PHY1_REF */
	uint32_t lane_rate_hz; /* burst-clock-frequency; 0 = slowest rate carrying the stream */
	uint32_t esc_max_hz;   /* capped at 20 MHz */
	uint32_t lanes;
	uint32_t flags;        /* YD_SAMSUNG_DSIM_* */
};

struct yd_samsung_dsim_mode {
	uint32_t pixel_clock_hz;
	uint32_t hactive, hfront, hsync, hback;
	uint32_t vactive, vfront, vsync, vback;
	uint32_t lanes;        /* 0 = the devicetree's */
};

struct yd_samsung_dsim_data {
	uintptr_t base;
	uint32_t pixel_clock_hz;
	uint32_t lane_rate_hz; /* the PLL's output: bit rate per lane */
	uint32_t byte_hz;
	uint32_t esc_hz;
	uint32_t plltmr;
	uint32_t pll_p, pll_m, pll_s;
	uint32_t phytiming[3]; /* PHYTIMING, PHYTIMING1, PHYTIMING2 as written */
	uint32_t step;         /* reached, or failed in */
	int32_t error;
	uint8_t attached;
	uint8_t reserved[3];
};

/* PLL, software reset, D-PHY timings, clocks, FIFOs, lanes, the mode and
 * MainStandby, with the data lanes held in stop state (ForceStopstate): the
 * clock lane runs in HS, no video. The block's power, clocks, resets and PLL
 * reference are the caller's. */
int yd_samsung_dsim_attach(const struct yd_samsung_dsim_config *cfg,
			   struct yd_samsung_dsim_data *data, const struct yd_env *env,
			   const struct yd_samsung_dsim_mode *mode);
/* Release the data lanes (risk R5: the driver-specific start). */
int yd_samsung_dsim_video_start(struct yd_samsung_dsim_data *data);
/* Lanes held, standby off, clocks off, PLL off. */
void yd_samsung_dsim_detach(struct yd_samsung_dsim_data *data);
uint32_t yd_samsung_dsim_health(const struct yd_samsung_dsim_data *data);

/* ---- TI AM62P DSS1 (soc::am62p::dss over ip::ti_dss7; safeui,am62p-dss) --
 * One video port, the application plane (VID, layer 0) under the safety
 * layer (VIDL1, layer 1). The ISR and the thread share `data` without a
 * lock: every mutable field is read and written atomically on the Rust side
 * (C reads them as plain 32-bit loads). */

#define YD_AM62P_DSS_HSYNC_LOW      (1u << 0)
#define YD_AM62P_DSS_VSYNC_LOW      (1u << 1)
#define YD_AM62P_DSS_DE_LOW         (1u << 2)
#define YD_AM62P_DSS_PIXCLK_NEGEDGE (1u << 3)
#define YD_AM62P_DSS_HAS_IRQ        (1u << 4)

/* yd_am62p_dss_isr() result bits */
#define YD_AM62P_DSS_EV_VSYNC       (1u << 0)
#define YD_AM62P_DSS_EV_SYNC_LOST   (1u << 1)
#define YD_AM62P_DSS_EV_UNDERFLOW   (1u << 2)

struct yd_am62p_dss_config {
	uint32_t pixel_clock_hz;
	uint16_t hactive, hfp, hsync, hbp;
	uint16_t vactive, vfp, vsync, vbp;
	uint32_t flags;          /* YD_AM62P_DSS_* */
	uint32_t vp_index;       /* 0 = VP1 */
	uint32_t safety_width;   /* the safety layer (devicetree width/height) */
	uint32_t safety_height;
};

struct yd_am62p_dss_stats {
	uint32_t vsyncs;
	uint32_t app_frames;
	uint32_t underflows;
	uint32_t sync_lost;
	uint32_t go_timeouts;
	uint32_t go_waits;
	uint32_t safety_flips;
};

struct yd_am62p_dss_data {
	uintptr_t common;        /* device_map() of the regions (the shim) */
	uintptr_t vid;
	uintptr_t vidl1;
	uintptr_t ovr;           /* the overlay and port of vp_index */
	uintptr_t vp;
	uint64_t safety_buf[2];  /* the safety layer's buffers (the shim) */
	uint64_t app_addr;       /* the rest is the Rust side's */
	uint32_t app_width;
	uint32_t app_height;
	uint32_t app_stride;
	uint32_t app_format;
	uint32_t app_seq_pending;
	uint32_t app_seq_shown;
	uint32_t app_commit_pending;
	uint32_t safety_flip_pending;
	uint32_t safety_front;
	uint32_t app_enabled;
	uint32_t output_enabled;
	struct yd_am62p_dss_stats stats;
};

struct yd_am62p_dss_plane {
	uint32_t seq;
	uint32_t format;         /* AM62P_DSS_FMT_* */
	uint64_t addr;
	uint32_t width;
	uint32_t height;
	uint32_t stride_bytes;
	uint32_t reserved;
};

int yd_am62p_dss_init(const struct yd_am62p_dss_config *cfg, const struct yd_am62p_dss_data *data,
		      const struct yd_env *env);
int yd_am62p_dss_set_app_buffer(const struct yd_am62p_dss_data *data,
				const struct yd_am62p_dss_plane *plane);
int yd_am62p_dss_enable_app_plane(const struct yd_am62p_dss_data *data, bool on);
void yd_am62p_dss_safety_premultiplied(const struct yd_am62p_dss_data *data, bool on);
void yd_am62p_dss_safety_flip(const struct yd_am62p_dss_data *data);
uint64_t yd_am62p_dss_safety_back(const struct yd_am62p_dss_data *data);
uint64_t yd_am62p_dss_safety_front(const struct yd_am62p_dss_data *data);
void yd_am62p_dss_output_enable(const struct yd_am62p_dss_data *data, bool on);
uint32_t yd_am62p_dss_isr(const struct yd_am62p_dss_config *cfg, const struct yd_am62p_dss_data *data);
bool yd_am62p_dss_poll_vsync(const struct yd_am62p_dss_config *cfg,
			     const struct yd_am62p_dss_data *data);

/* ---- Cadence DSI host + D-PHY TX (ip::cdns_dsi, ip::cdns_dphy;
 * safeui,cdns-dsi-j721e) ------------------------------------------------- */

#define YD_CDNS_DSI_SYNC_PULSE (1u << 0) /* non-burst with sync pulses (else events) */
#define YD_CDNS_DSI_EOT        (1u << 1) /* HOST_EOT_GEN */

struct yd_cdns_dsi_data {
	uintptr_t dsi;           /* device_map() of the regions (the shim) */
	uintptr_t wrap;
	uintptr_t dphy;
	uint32_t wiz;            /* the D-PHY's WIZ registers' offset (init) */
	uint32_t link_up;
	uint64_t rate;           /* lane rate of the running link, bit/s */
	uint32_t ipdiv, opdiv, fbdiv, band;
	uint32_t line_bytes;
	uint32_t reserved;
};

struct yd_cdns_dsi_request {
	uint32_t pixel_clock_hz;
	uint16_t hactive, hfp, hsync, hbp;
	uint16_t vactive, vfp, vsync, vbp;
	uint32_t bpp;            /* 16 or 24 */
	uint32_t lanes;
	uint32_t flags;          /* YD_CDNS_DSI_* */
	uint32_t datatype;       /* MIPI DSI data type of the pixel packets */
	uint32_t pll_ref_hz;
	uint32_t psm_hz;
};

int yd_cdns_dsi_init(struct yd_cdns_dsi_data *data, const struct yd_env *env);
int yd_cdns_dsi_attach(struct yd_cdns_dsi_data *data, const struct yd_env *env,
		       const struct yd_cdns_dsi_request *req);
/* One direct write command; the bytes sent or a negative errno. */
int yd_cdns_dsi_transfer(struct yd_cdns_dsi_data *data, const struct yd_env *env, uint8_t channel,
			 uint8_t datatype, bool lp, const uint8_t *buf, size_t len);
void yd_cdns_dsi_detach(struct yd_cdns_dsi_data *data);

/* ---- Vivante GC7000 kernel side (gpu::vivante; vivante,gc with
 *      CONFIG_SAFEUI_GPU_VIVANTE) ------------------------------------------
 * The shim is driver/drivers/gpu/gpu_vivante_gc_rust.c. `core` is its
 * struct gc_dev (driver/src/vivante/gc_internal.h), of which the Rust side
 * mirrors the part before the kernel objects; the shim pins those offsets
 * with _Static_asserts against the numbers src/ffi/vivante.rs asserts.
 * `id` is a struct gc_identity (gc.h), `s` a struct gc_submit (gc_render.h).
 * Each function is the gc_internal.h function of the same name without the
 * yd_ prefix; the shim holds the submit mutex around yd_gc_ring_submit and
 * the spinlock around yd_gc_event_claim and yd_gc_irq_handle. */
struct gc_identity;
struct gc_submit;
struct yd_gc_core;

/* `mark` (may be NULL) is called with `ctx` and a gpu::vivante::power::step
 * number before each sub-step of the power-up; it must not touch the CCM, the
 * GPC or the GPU. */
int yd_gc_power_on(uintptr_t ccm, uintptr_t gpc, const struct yd_env *env,
		   void (*mark)(void *ctx, uint32_t step), void *ctx);
int yd_gc_identify(uintptr_t regs, const struct yd_env *env, struct gc_identity *id);
int yd_gc_reset(uintptr_t regs, const struct yd_env *env, const struct gc_identity *id);
int yd_gc_hw_init(struct yd_gc_core *core, const struct yd_env *env);
uint32_t yd_gc_rd_fe(const struct yd_gc_core *core, uint32_t off);
void yd_gc_idle_state_decode(const struct yd_env *env, uint32_t idle);
int yd_gc_mmu_init(struct yd_gc_core *core, const struct yd_env *env);
int yd_gc_mmu_map(struct yd_gc_core *core, const struct yd_env *env, uint32_t gpu_va,
		  uint64_t phys, size_t size);
int yd_gc_mmu_enable(struct yd_gc_core *core, const struct yd_env *env);
void yd_gc_mmu_dump(const struct yd_gc_core *core, const struct yd_env *env);
void yd_gc_fe_start(const struct yd_gc_core *core, uint32_t address, uint32_t prefetch);
int yd_gc_fe_wait_idle(const struct yd_gc_core *core, const struct yd_env *env,
		       uint32_t timeout_ms);
uint32_t yd_gc_cmd_build_pta_config(const struct yd_gc_core *core, uint32_t index);
int yd_gc_ring_init(struct yd_gc_core *core, const struct yd_env *env, uint32_t clock_hz);
int yd_gc_ring_submit(struct yd_gc_core *core, const struct yd_env *env,
		      const struct gc_submit *s, uint32_t event, uint32_t fence,
		      uint32_t clock_hz);
void yd_gc_ring_dump(const struct yd_gc_core *core, const struct yd_env *env);
/* An event id for `fence`, -EAGAIN when all 30 are in flight, -EIO for a
 * corrupt bitmap. */
int yd_gc_event_claim(struct yd_gc_core *core, uint32_t fence);
/* True when this GPU raised the interrupt; *freed: events freed. */
bool yd_gc_irq_handle(struct yd_gc_core *core, uint64_t now_ns, uint32_t *freed);

/* ---- layout pins (the Rust side asserts the same numbers) --------------- */
#if defined(__LP64__)
_Static_assert(sizeof(struct yd_imx8mp_hdmi_config) == 72, "struct yd_imx8mp_hdmi_config");
_Static_assert(offsetof(struct yd_imx8mp_hdmi_config, vback) == 32, "yd_imx8mp_hdmi_config.vback");
_Static_assert(offsetof(struct yd_imx8mp_hdmi_config, flags) == 36, "yd_imx8mp_hdmi_config.flags");
_Static_assert(offsetof(struct yd_imx8mp_hdmi_config, stop_after) == 60, "yd_imx8mp_hdmi_config.stop_after");
_Static_assert(offsetof(struct yd_imx8mp_hdmi_config, test_rgb) == 64, "yd_imx8mp_hdmi_config.test_rgb");
_Static_assert(offsetof(struct yd_imx8mp_hdmi_config, phy_conf0) == 68, "yd_imx8mp_hdmi_config.phy_conf0");
_Static_assert(offsetof(struct yd_imx8mp_hdmi_config, phyrstz) == 69, "yd_imx8mp_hdmi_config.phyrstz");
_Static_assert(sizeof(struct yd_imx8mp_hdmi_data) == 120, "struct yd_imx8mp_hdmi_data");
_Static_assert(offsetof(struct yd_imx8mp_hdmi_data, ccm) == 48, "yd_imx8mp_hdmi_data.ccm");
_Static_assert(offsetof(struct yd_imx8mp_hdmi_data, fb) == 56, "yd_imx8mp_hdmi_data.fb");
_Static_assert(offsetof(struct yd_imx8mp_hdmi_data, prev_fb) == 64, "yd_imx8mp_hdmi_data.prev_fb");
_Static_assert(offsetof(struct yd_imx8mp_hdmi_data, prev_desc3) == 72, "yd_imx8mp_hdmi_data.prev_desc3");
_Static_assert(offsetof(struct yd_imx8mp_hdmi_data, width) == 80, "yd_imx8mp_hdmi_data.width");
_Static_assert(offsetof(struct yd_imx8mp_hdmi_data, tmds_hz) == 92, "yd_imx8mp_hdmi_data.tmds_hz");
_Static_assert(offsetof(struct yd_imx8mp_hdmi_data, err_ppm) == 96, "yd_imx8mp_hdmi_data.err_ppm");
_Static_assert(offsetof(struct yd_imx8mp_hdmi_data, stage) == 108, "yd_imx8mp_hdmi_data.stage");
_Static_assert(offsetof(struct yd_imx8mp_hdmi_data, error) == 112, "yd_imx8mp_hdmi_data.error");
_Static_assert(offsetof(struct yd_imx8mp_hdmi_data, on) == 116, "yd_imx8mp_hdmi_data.on");
_Static_assert(offsetof(struct yd_imx8mp_hdmi_data, adopted) == 117, "yd_imx8mp_hdmi_data.adopted");
_Static_assert(sizeof(struct yd_env) == 56, "struct yd_env");
_Static_assert(offsetof(struct yd_env, ctx) == 0, "yd_env.ctx");
_Static_assert(offsetof(struct yd_env, busy_wait_us) == 8, "yd_env.busy_wait_us");
_Static_assert(offsetof(struct yd_env, sleep_ms) == 16, "yd_env.sleep_ms");
_Static_assert(offsetof(struct yd_env, log) == 24, "yd_env.log");
_Static_assert(offsetof(struct yd_env, i2c_write) == 32, "yd_env.i2c_write");
_Static_assert(offsetof(struct yd_env, i2c_write_read) == 40, "yd_env.i2c_write_read");
_Static_assert(offsetof(struct yd_env, gpio_set) == 48, "yd_env.gpio_set");
_Static_assert(sizeof(struct yd_imx_pwm_config) == 8, "struct yd_imx_pwm_config");
_Static_assert(offsetof(struct yd_imx_pwm_config, ccm_root) == 0, "yd_imx_pwm_config.ccm_root");
_Static_assert(offsetof(struct yd_imx_pwm_config, ccm_gate) == 4, "yd_imx_pwm_config.ccm_gate");
_Static_assert(sizeof(struct yd_imx_pwm_data) == 32, "struct yd_imx_pwm_data");
_Static_assert(offsetof(struct yd_imx_pwm_data, pwm_base) == 0, "yd_imx_pwm_data.pwm_base");
_Static_assert(offsetof(struct yd_imx_pwm_data, ccm_base) == 8, "yd_imx_pwm_data.ccm_base");
_Static_assert(offsetof(struct yd_imx_pwm_data, rate_hz) == 16, "yd_imx_pwm_data.rate_hz");
_Static_assert(offsetof(struct yd_imx_pwm_data, period) == 20, "yd_imx_pwm_data.period");
_Static_assert(offsetof(struct yd_imx_pwm_data, pulse) == 24, "yd_imx_pwm_data.pulse");
_Static_assert(offsetof(struct yd_imx_pwm_data, flags) == 28, "yd_imx_pwm_data.flags");
_Static_assert(sizeof(struct yd_record_writer) == 8, "struct yd_record_writer");
_Static_assert(sizeof(struct yd_imx8mp_hold) == 36, "struct yd_imx8mp_hold");
_Static_assert(offsetof(struct yd_imx8mp_hold, code) == 8, "yd_imx8mp_hold.code");
_Static_assert(offsetof(struct yd_imx8mp_hold, reset_max) == 32, "yd_imx8mp_hold.reset_max");
_Static_assert(offsetof(struct yd_record_writer, seq) == 0, "yd_record_writer.seq");
_Static_assert(offsetof(struct yd_record_writer, have_result) == 1, "yd_record_writer.have_result");
_Static_assert(offsetof(struct yd_record_writer, progress_count) == 4, "yd_record_writer.progress_count");
_Static_assert(sizeof(struct yd_glmark2_outcome) == 56, "struct yd_glmark2_outcome");
_Static_assert(offsetof(struct yd_glmark2_outcome, failed_entry) == 24, "yd_glmark2_outcome.failed_entry");
_Static_assert(offsetof(struct yd_glmark2_outcome, unsupported_entries) == 32,
	       "yd_glmark2_outcome.unsupported_entries");
_Static_assert(offsetof(struct yd_glmark2_outcome, h) == 44, "yd_glmark2_outcome.h");
_Static_assert(offsetof(struct yd_glmark2_outcome, gl_errors) == 48, "yd_glmark2_outcome.gl_errors");
_Static_assert(offsetof(struct yd_glmark2_outcome, incomplete_draws) == 52,
	       "yd_glmark2_outcome.incomplete_draws");
_Static_assert(sizeof(struct yd_record_display) == 32, "struct yd_record_display");
_Static_assert(offsetof(struct yd_record_display, irq_now) == 12, "yd_record_display.irq_now");
_Static_assert(offsetof(struct yd_record_display, pitch) == 28, "yd_record_display.pitch");
_Static_assert(sizeof(struct yd_imx_wdog_arm) == 20, "struct yd_imx_wdog_arm");
_Static_assert(offsetof(struct yd_imx_wdog_arm, wcr_found) == 0, "yd_imx_wdog_arm.wcr_found");
_Static_assert(offsetof(struct yd_imx_wdog_arm, wmcr_found) == 2, "yd_imx_wdog_arm.wmcr_found");
_Static_assert(offsetof(struct yd_imx_wdog_arm, wcr_now) == 4, "yd_imx_wdog_arm.wcr_now");
_Static_assert(offsetof(struct yd_imx_wdog_arm, flags) == 8, "yd_imx_wdog_arm.flags");
_Static_assert(offsetof(struct yd_imx_wdog_arm, refusal) == 12, "yd_imx_wdog_arm.refusal");
_Static_assert(offsetof(struct yd_imx_wdog_arm, timeout_ms) == 16, "yd_imx_wdog_arm.timeout_ms");
_Static_assert(sizeof(struct yd_sn65dsi8x_config) == 8, "struct yd_sn65dsi8x_config");
_Static_assert(offsetof(struct yd_sn65dsi8x_config, flags) == 0, "yd_sn65dsi8x_config.flags");
_Static_assert(offsetof(struct yd_sn65dsi8x_config, lanes) == 4, "yd_sn65dsi8x_config.lanes");
_Static_assert(sizeof(struct yd_sn65dsi8x_mode) == 20, "struct yd_sn65dsi8x_mode");
_Static_assert(offsetof(struct yd_sn65dsi8x_mode, hactive) == 4, "yd_sn65dsi8x_mode.hactive");
_Static_assert(offsetof(struct yd_sn65dsi8x_mode, vfp) == 18, "yd_sn65dsi8x_mode.vfp");
_Static_assert(sizeof(struct yd_sn65dsi8x_data) == 24, "struct yd_sn65dsi8x_data");
_Static_assert(offsetof(struct yd_sn65dsi8x_data, lvds_clk_range) == 9, "yd_sn65dsi8x_data.lvds_clk_range");
_Static_assert(offsetof(struct yd_sn65dsi8x_data, irq_stat) == 13, "yd_sn65dsi8x_data.irq_stat");
_Static_assert(offsetof(struct yd_sn65dsi8x_data, id_tries) == 15, "yd_sn65dsi8x_data.id_tries");
_Static_assert(offsetof(struct yd_sn65dsi8x_data, dsi_clk_khz) == 16, "yd_sn65dsi8x_data.dsi_clk_khz");
_Static_assert(offsetof(struct yd_sn65dsi8x_data, lvds_clk_khz) == 20, "yd_sn65dsi8x_data.lvds_clk_khz");
_Static_assert(sizeof(struct yd_pvr_bus) == 32, "struct yd_pvr_bus");
_Static_assert(offsetof(struct yd_pvr_bus, ctx) == 0, "yd_pvr_bus.ctx");
_Static_assert(offsetof(struct yd_pvr_bus, read32) == 8, "yd_pvr_bus.read32");
_Static_assert(offsetof(struct yd_pvr_bus, write32) == 16, "yd_pvr_bus.write32");
_Static_assert(offsetof(struct yd_pvr_bus, barrier) == 24, "yd_pvr_bus.barrier");
_Static_assert(sizeof(struct yd_pvr_layer) == 64, "struct yd_pvr_layer");
_Static_assert(offsetof(struct yd_pvr_layer, regs) == 0, "yd_pvr_layer.regs");
_Static_assert(offsetof(struct yd_pvr_layer, env) == 8, "yd_pvr_layer.env");
_Static_assert(offsetof(struct yd_pvr_layer, bus) == 16, "yd_pvr_layer.bus");
_Static_assert(offsetof(struct yd_pvr_layer, kernel_pc) == 24, "yd_pvr_layer.kernel_pc");
_Static_assert(offsetof(struct yd_pvr_layer, fw_code_devva) == 32, "yd_pvr_layer.fw_code_devva");
_Static_assert(offsetof(struct yd_pvr_layer, fw_data_devva) == 40, "yd_pvr_layer.fw_data_devva");
_Static_assert(offsetof(struct yd_pvr_layer, core_clock_hz) == 48, "yd_pvr_layer.core_clock_hz");
_Static_assert(offsetof(struct yd_pvr_layer, poll_timeout_us) == 52, "yd_pvr_layer.poll_timeout_us");
_Static_assert(offsetof(struct yd_pvr_layer, poll_step_us) == 56, "yd_pvr_layer.poll_step_us");
_Static_assert(offsetof(struct yd_pvr_layer, powered) == 60, "yd_pvr_layer.powered");
_Static_assert(sizeof(struct yd_pvr_km_ops) == 48, "struct yd_pvr_km_ops");
_Static_assert(offsetof(struct yd_pvr_km_ops, flush) == 8, "yd_pvr_km_ops.flush");
_Static_assert(offsetof(struct yd_pvr_km_ops, invd) == 16, "yd_pvr_km_ops.invd");
_Static_assert(offsetof(struct yd_pvr_km_ops, fence) == 24, "yd_pvr_km_ops.fence");
_Static_assert(offsetof(struct yd_pvr_km_ops, busy_wait_us) == 32, "yd_pvr_km_ops.busy_wait_us");
_Static_assert(offsetof(struct yd_pvr_km_ops, log) == 40, "yd_pvr_km_ops.log");
_Static_assert(sizeof(struct yd_pvr_span) == 16, "struct yd_pvr_span");
_Static_assert(offsetof(struct yd_pvr_km, phys) == 8, "yd_pvr_km.phys");
_Static_assert(offsetof(struct yd_pvr_km, size) == 16, "yd_pvr_km.size");
_Static_assert(offsetof(struct yd_pvr_km, ops) == 24, "yd_pvr_km.ops");
_Static_assert(offsetof(struct yd_pvr_km, bus) == 32, "yd_pvr_km.bus");
_Static_assert(offsetof(struct yd_pvr_km, regs) == 40, "yd_pvr_km.regs");
_Static_assert(offsetof(struct yd_pvr_km, fw_main_size) == 48, "yd_pvr_km.fw_main_size");
_Static_assert(offsetof(struct yd_pvr_km, pool_off) == 64, "yd_pvr_km.pool_off");
_Static_assert(offsetof(struct yd_pvr_km, heap_next) == 68, "yd_pvr_km.heap_next");
_Static_assert(offsetof(struct yd_pvr_km, place_n) == 88, "yd_pvr_km.place_n");
_Static_assert(offsetof(struct yd_pvr_km, place) == 96, "yd_pvr_km.place");
_Static_assert(offsetof(struct yd_pvr_km, kccb_ctl_off) == 608, "yd_pvr_km.kccb_ctl_off");
_Static_assert(offsetof(struct yd_pvr_km, kccb_posts) == 628, "yd_pvr_km.kccb_posts");
_Static_assert(offsetof(struct yd_pvr_km, pm_bump) == 632, "yd_pvr_km.pm_bump");
_Static_assert(offsetof(struct yd_pvr_km, nfls) == 664, "yd_pvr_km.nfls");
_Static_assert(offsetof(struct yd_pvr_km, fl_pool_phys) == 672, "yd_pvr_km.fl_pool_phys");
_Static_assert(offsetof(struct yd_pvr_km, fls) == 680, "yd_pvr_km.fls");
_Static_assert(sizeof(struct yd_pvr_fl) == 64, "struct yd_pvr_fl");
_Static_assert(offsetof(struct yd_pvr_fl, arr) == 16, "yd_pvr_fl.arr");
_Static_assert(offsetof(struct yd_pvr_fl, backing_phys) == 24, "yd_pvr_fl.backing_phys");
_Static_assert(offsetof(struct yd_pvr_fl, npages) == 32, "yd_pvr_fl.npages");
_Static_assert(offsetof(struct yd_pvr_fl, grows) == 56, "yd_pvr_fl.grows");
_Static_assert(offsetof(struct yd_pvr_km, sysinit_off) == 680 + 64 * YD_PVR_FL_MAX, "yd_pvr_km.sysinit_off");
_Static_assert(offsetof(struct yd_pvr_km, hwrinfobuf_off) == 680 + 64 * YD_PVR_FL_MAX + 12,
	       "yd_pvr_km.hwrinfobuf_off");
_Static_assert(offsetof(struct yd_pvr_km, fl_grow_pfn) == 680 + 64 * YD_PVR_FL_MAX + 16,
	       "yd_pvr_km.fl_grow_pfn");
_Static_assert(offsetof(struct yd_pvr_km, fw_stats) ==
		       680 + 64 * YD_PVR_FL_MAX + 16 + 4 * YD_PVR_FL_GROWS_MAX * YD_PVR_FL_MAX + 4 * YD_PVR_FL_MAX + 8,
	       "yd_pvr_km.fw_stats");
_Static_assert(offsetof(struct yd_pvr_km, stop_cause) ==
		       680 + 64 * YD_PVR_FL_MAX + 16 + 4 * YD_PVR_FL_GROWS_MAX * YD_PVR_FL_MAX + 4 * YD_PVR_FL_MAX + 36,
	       "yd_pvr_km.stop_cause");
_Static_assert(offsetof(struct yd_pvr_km, fwccb_check) ==
		       680 + 64 * YD_PVR_FL_MAX + 16 + 4 * YD_PVR_FL_GROWS_MAX * YD_PVR_FL_MAX + 4 * YD_PVR_FL_MAX + 56,
	       "yd_pvr_km.fwccb_check");
_Static_assert(offsetof(struct yd_pvr_km, fw_uncached) ==
		       680 + 64 * YD_PVR_FL_MAX + 16 + 4 * YD_PVR_FL_GROWS_MAX * YD_PVR_FL_MAX + 4 * YD_PVR_FL_MAX + 80,
	       "yd_pvr_km.fw_uncached");
_Static_assert(offsetof(struct yd_pvr_km, pool_top_above) ==
		       680 + 64 * YD_PVR_FL_MAX + 16 + 4 * YD_PVR_FL_GROWS_MAX * YD_PVR_FL_MAX + 4 * YD_PVR_FL_MAX + 88,
	       "yd_pvr_km.pool_top_above");
_Static_assert(sizeof(struct yd_pvr_km) ==
		       680 + 64 * YD_PVR_FL_MAX + 16 + 4 * YD_PVR_FL_GROWS_MAX * YD_PVR_FL_MAX + 4 * YD_PVR_FL_MAX + 104,
	       "struct yd_pvr_km");
_Static_assert(sizeof(struct yd_pvr_fwinit) == 24, "struct yd_pvr_fwinit");
_Static_assert(sizeof(struct yd_pvr_hwrt) == 176, "struct yd_pvr_hwrt");
_Static_assert(offsetof(struct yd_pvr_hwrt, tpc) == 32, "yd_pvr_hwrt.tpc");
_Static_assert(offsetof(struct yd_pvr_hwrt, mlist) == 128, "yd_pvr_hwrt.mlist");
_Static_assert(offsetof(struct yd_pvr_hwrt, next) == 168, "yd_pvr_hwrt.next");
_Static_assert(sizeof(struct yd_pvr_render_cfg) == 48, "struct yd_pvr_render_cfg");
_Static_assert(sizeof(struct yd_pvr_render_ops) == 16, "struct yd_pvr_render_ops");
_Static_assert(offsetof(struct yd_pvr_render, ops) == 48, "yd_pvr_render.ops");
_Static_assert(offsetof(struct yd_pvr_render, gen_b) == 60, "yd_pvr_render.gen_b");
_Static_assert(offsetof(struct yd_pvr_render, last_hs) == 140, "yd_pvr_render.last_hs");
_Static_assert(offsetof(struct yd_pvr_render, vdm_cs) == 152, "yd_pvr_render.vdm_cs");
_Static_assert(offsetof(struct yd_pvr_render, reposts) == 176, "yd_pvr_render.reposts");
_Static_assert(offsetof(struct yd_pvr_render, last_frag_cmd) == 216, "yd_pvr_render.last_frag_cmd");
_Static_assert(offsetof(struct yd_pvr_render, last_kick) == 536, "yd_pvr_render.last_kick");
_Static_assert(offsetof(struct yd_pvr_render, hwrt) == 856, "yd_pvr_render.hwrt");
_Static_assert(sizeof(struct yd_pvr_kick_stamp) == 72, "struct yd_pvr_kick_stamp");
_Static_assert(sizeof(struct yd_pvr_kick_times) == 24 + 72 * (1 + YD_PVR_KT_RING),
	       "struct yd_pvr_kick_times");
_Static_assert(sizeof(struct yd_pvr_kick_us) == 52, "struct yd_pvr_kick_us");
_Static_assert(offsetof(struct yd_pvr_kick_stamp, t_wait) == 48, "yd_pvr_kick_stamp.t_wait");
_Static_assert(offsetof(struct yd_pvr_kick_stamp, seq) == 64, "yd_pvr_kick_stamp.seq");
_Static_assert(offsetof(struct yd_pvr_kick_times, prev_frag) == 16, "yd_pvr_kick_times.prev_frag");
_Static_assert(offsetof(struct yd_pvr_kick_times, ring) == 24 + 72, "yd_pvr_kick_times.ring");
_Static_assert(offsetof(struct yd_pvr_kick_us, kick_geom_us) == 16, "yd_pvr_kick_us.kick_geom_us");
_Static_assert(offsetof(struct yd_pvr_kick_us, frag_frag_us) == 32, "yd_pvr_kick_us.frag_frag_us");
_Static_assert(offsetof(struct yd_pvr_kick_us, flags) == 40, "yd_pvr_kick_us.flags");
_Static_assert(offsetof(struct yd_pvr_render, kick_times) == 856 + 176 * YD_PVR_HWRT_SETS,
	       "yd_pvr_render.kick_times");
_Static_assert(offsetof(struct yd_pvr_render, grow_scene) ==
		       856 + 176 * YD_PVR_HWRT_SETS + sizeof(struct yd_pvr_kick_times),
	       "yd_pvr_render.grow_scene");
_Static_assert(sizeof(struct yd_pvr_grow_scene) == 16, "struct yd_pvr_grow_scene");
_Static_assert(sizeof(struct yd_pvr_render) ==
	       856 + 176 * YD_PVR_HWRT_SETS + sizeof(struct yd_pvr_kick_times) + 16,
	       "struct yd_pvr_render");
_Static_assert(sizeof(struct yd_pvr_bo) == 24, "struct yd_pvr_bo");
_Static_assert(offsetof(struct yd_pvr_bo, heap_off) == 16, "yd_pvr_bo.heap_off");
_Static_assert(sizeof(struct yd_pvr_pool) == YD_PVR_POOL_BYTES, "struct yd_pvr_pool");

_Static_assert(sizeof(struct yd_imx8mp_media_data) == 48, "struct yd_imx8mp_media_data");
_Static_assert(offsetof(struct yd_imx8mp_media_data, anatop_base) == 24, "yd_imx8mp_media_data.anatop_base");
_Static_assert(offsetof(struct yd_imx8mp_media_data, users) == 32, "yd_imx8mp_media_data.users");
_Static_assert(offsetof(struct yd_imx8mp_media_data, last) == 40, "yd_imx8mp_media_data.last");
_Static_assert(sizeof(struct yd_am62p_dss_config) == 36, "struct yd_am62p_dss_config");
_Static_assert(offsetof(struct yd_am62p_dss_config, hactive) == 4, "yd_am62p_dss_config.hactive");
_Static_assert(offsetof(struct yd_am62p_dss_config, vbp) == 18, "yd_am62p_dss_config.vbp");
_Static_assert(offsetof(struct yd_am62p_dss_config, flags) == 20, "yd_am62p_dss_config.flags");
_Static_assert(offsetof(struct yd_am62p_dss_config, vp_index) == 24, "yd_am62p_dss_config.vp_index");
_Static_assert(offsetof(struct yd_am62p_dss_config, safety_height) == 32, "yd_am62p_dss_config.safety_height");
_Static_assert(sizeof(struct yd_am62p_dss_stats) == 28, "struct yd_am62p_dss_stats");
_Static_assert(sizeof(struct yd_am62p_dss_data) == 136, "struct yd_am62p_dss_data");
_Static_assert(offsetof(struct yd_am62p_dss_data, vp) == 32, "yd_am62p_dss_data.vp");
_Static_assert(offsetof(struct yd_am62p_dss_data, safety_buf) == 40, "yd_am62p_dss_data.safety_buf");
_Static_assert(offsetof(struct yd_am62p_dss_data, app_addr) == 56, "yd_am62p_dss_data.app_addr");
_Static_assert(offsetof(struct yd_am62p_dss_data, app_format) == 76, "yd_am62p_dss_data.app_format");
_Static_assert(offsetof(struct yd_am62p_dss_data, app_seq_pending) == 80, "yd_am62p_dss_data.app_seq_pending");
_Static_assert(offsetof(struct yd_am62p_dss_data, app_seq_shown) == 84, "yd_am62p_dss_data.app_seq_shown");
_Static_assert(offsetof(struct yd_am62p_dss_data, app_commit_pending) == 88, "yd_am62p_dss_data.app_commit_pending");
_Static_assert(offsetof(struct yd_am62p_dss_data, safety_flip_pending) == 92, "yd_am62p_dss_data.safety_flip_pending");
_Static_assert(offsetof(struct yd_am62p_dss_data, safety_front) == 96, "yd_am62p_dss_data.safety_front");
_Static_assert(offsetof(struct yd_am62p_dss_data, app_enabled) == 100, "yd_am62p_dss_data.app_enabled");
_Static_assert(offsetof(struct yd_am62p_dss_data, output_enabled) == 104, "yd_am62p_dss_data.output_enabled");
_Static_assert(offsetof(struct yd_am62p_dss_data, stats) == 108, "yd_am62p_dss_data.stats");
_Static_assert(sizeof(struct yd_am62p_dss_plane) == 32, "struct yd_am62p_dss_plane");
_Static_assert(offsetof(struct yd_am62p_dss_plane, addr) == 8, "yd_am62p_dss_plane.addr");
_Static_assert(offsetof(struct yd_am62p_dss_plane, stride_bytes) == 24, "yd_am62p_dss_plane.stride_bytes");
_Static_assert(sizeof(struct yd_cdns_dsi_data) == 64, "struct yd_cdns_dsi_data");
_Static_assert(offsetof(struct yd_cdns_dsi_data, wiz) == 24, "yd_cdns_dsi_data.wiz");
_Static_assert(offsetof(struct yd_cdns_dsi_data, rate) == 32, "yd_cdns_dsi_data.rate");
_Static_assert(offsetof(struct yd_cdns_dsi_data, band) == 52, "yd_cdns_dsi_data.band");
_Static_assert(offsetof(struct yd_cdns_dsi_data, line_bytes) == 56, "yd_cdns_dsi_data.line_bytes");
_Static_assert(sizeof(struct yd_cdns_dsi_request) == 44, "struct yd_cdns_dsi_request");
_Static_assert(offsetof(struct yd_cdns_dsi_request, vbp) == 18, "yd_cdns_dsi_request.vbp");
_Static_assert(offsetof(struct yd_cdns_dsi_request, bpp) == 20, "yd_cdns_dsi_request.bpp");
_Static_assert(offsetof(struct yd_cdns_dsi_request, psm_hz) == 40, "yd_cdns_dsi_request.psm_hz");
_Static_assert(sizeof(struct yd_lcdif_config) == 20, "struct yd_lcdif_config");
_Static_assert(offsetof(struct yd_lcdif_config, desc3_extra) == 12, "yd_lcdif_config.desc3_extra");
_Static_assert(offsetof(struct yd_lcdif_config, panic_high) == 18, "yd_lcdif_config.panic_high");
_Static_assert(sizeof(struct yd_lcdif_mode) == 36, "struct yd_lcdif_mode");
_Static_assert(offsetof(struct yd_lcdif_mode, vback) == 32, "yd_lcdif_mode.vback");
_Static_assert(sizeof(struct yd_lcdif_data) == 56, "struct yd_lcdif_data");
_Static_assert(offsetof(struct yd_lcdif_data, fb) == 8, "yd_lcdif_data.fb");
_Static_assert(offsetof(struct yd_lcdif_data, prev_desc3) == 24, "yd_lcdif_data.prev_desc3");
_Static_assert(offsetof(struct yd_lcdif_data, irq_status) == 48, "yd_lcdif_data.irq_status");
_Static_assert(offsetof(struct yd_lcdif_data, on) == 52, "yd_lcdif_data.on");
_Static_assert(offsetof(struct yd_lcdif_data, ours) == 55, "yd_lcdif_data.ours");
_Static_assert(sizeof(struct yd_lcdif_scanout) == 32, "struct yd_lcdif_scanout");
_Static_assert(offsetof(struct yd_lcdif_scanout, bpp_code) == 16, "yd_lcdif_scanout.bpp_code");
_Static_assert(offsetof(struct yd_lcdif_scanout, phys) == 24, "yd_lcdif_scanout.phys");
_Static_assert(sizeof(struct yd_samsung_dsim_config) == 20, "struct yd_samsung_dsim_config");
_Static_assert(offsetof(struct yd_samsung_dsim_config, flags) == 16, "yd_samsung_dsim_config.flags");
_Static_assert(sizeof(struct yd_samsung_dsim_mode) == 40, "struct yd_samsung_dsim_mode");
_Static_assert(offsetof(struct yd_samsung_dsim_mode, lanes) == 36, "yd_samsung_dsim_mode.lanes");
_Static_assert(sizeof(struct yd_samsung_dsim_data) == 64, "struct yd_samsung_dsim_data");
_Static_assert(offsetof(struct yd_samsung_dsim_data, pixel_clock_hz) == 8, "yd_samsung_dsim_data.pixel_clock_hz");
_Static_assert(offsetof(struct yd_samsung_dsim_data, pll_p) == 28, "yd_samsung_dsim_data.pll_p");
_Static_assert(offsetof(struct yd_samsung_dsim_data, phytiming) == 40, "yd_samsung_dsim_data.phytiming");
_Static_assert(offsetof(struct yd_samsung_dsim_data, step) == 52, "yd_samsung_dsim_data.step");
_Static_assert(offsetof(struct yd_samsung_dsim_data, attached) == 60, "yd_samsung_dsim_data.attached");
#else
#error "yakogl_drivers.h: LP64 targets only"
#endif

#ifdef __cplusplus
}
#endif

#endif /* YAKOGL_DRIVERS_H */
