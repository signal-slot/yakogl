/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * rgxlayer_check.c -- the TI/Img DDK 25.2's RGXStart, RGXStop and firmware
 * image loader (rgxstartstop.c, rgxfwimageutils.c, vendored unmodified,
 * compiled through driver/compat/pvr_rgxstartstop.c and
 * pvr_rgxfwimageutils.c exactly as the board's build compiles them) run on the
 * host against our rgxlayer (driver/rust/src/gpu/pvr/rgxlayer.rs, the
 * yakogl-drivers crate built for the host with feature `pvr`) with a recording
 * register bus.
 *
 *   rgxlayer_check <outdir> [<firmware image>]
 *
 * Checks:
 *  1. every feature, erratum and feature value the DDK knows: the Rust
 *     answer = what the configuration headers say (driver/compat/pvr_rgx_core.h);
 *  2. RGXStart returns PVRSRV_OK and sets the powered state; its register
 *     sequence is written to <outdir>/rgxstart.trace (tools/check.sh compares
 *     it with the sequence the driver used before, tools/pvr/rgxstart/);
 *  3. RGXStop on an idle GPU returns PVRSRV_OK, clears the powered state
 *     (<outdir>/rgxstop.trace), and on a GPU that never idles times out;
 *  4. RGXGetFWImageAllocSize / RGXProcessFWImage on a synthetic image with the
 *     shape of rgx.fw.36.53.104.796 (4 layout entries: two code, RW data, RO
 *     data; 4 PT_LOAD segments) place every byte where the driver's former
 *     loader did (code at its allocation offset, data at 0x80000 + (vaddr -
 *     0xa0000000)), zero the bss tails and write a zero boot-data block;
 *  5. with a firmware image given (the real one is TI/Img property and never
 *     committed), the same placement rule over every byte of it.
 *
 * Trace lines: "W <off> <val>" 32-bit write, "R <off> <val>" 32-bit read,
 * "B <off>" write barrier, "D <us>" busy wait. 64-bit registers appear as two
 * 32-bit accesses, low word first (that is how both the old driver and the
 * Rust rgxlayer access them).
 */
#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "pvr_ddk.h"
#include "rgxstartstop.h"
#include "rgxfwimageutils.h"
#include "pvrversion.h"
#include "pvr_rgx_core.h"
#include <yakogl_drivers.h>

static unsigned g_run, g_failed;

#define CHECK(cond, ...)                                       \
	do {                                                   \
		g_run++;                                       \
		if (!(cond)) {                                 \
			g_failed++;                            \
			printf("FAIL %s:%d: ", __FILE__, __LINE__); \
			printf(__VA_ARGS__);                   \
			printf("\n");                          \
		}                                              \
	} while (0)

/* ---- the recording bus: a plain register file ------------------------- */

#define REGS_SIZE 0x10000u
static uint32_t g_regs[REGS_SIZE / 4];
static FILE *g_trace;

static uint32_t bus_read32(void *ctx, uintptr_t addr)
{
	(void)ctx;
	if (addr >= REGS_SIZE || addr % 4) {
		printf("FAIL read outside the register bank: 0x%lx\n", (unsigned long)addr);
		exit(2);
	}
	if (g_trace)
		fprintf(g_trace, "R 0x%04lx 0x%08x\n", (unsigned long)addr, g_regs[addr / 4]);
	return g_regs[addr / 4];
}

static void bus_write32(void *ctx, uintptr_t addr, uint32_t val)
{
	(void)ctx;
	if (addr >= REGS_SIZE || addr % 4) {
		printf("FAIL write outside the register bank: 0x%lx\n", (unsigned long)addr);
		exit(2);
	}
	if (g_trace)
		fprintf(g_trace, "W 0x%04lx 0x%08x\n", (unsigned long)addr, val);
	g_regs[addr / 4] = val;
}

static void bus_barrier(void *ctx, uintptr_t addr)
{
	(void)ctx;
	if (g_trace)
		fprintf(g_trace, "B 0x%04lx\n", (unsigned long)addr);
}

static void poke64(uint32_t off, uint64_t v)
{
	g_regs[off / 4] = (uint32_t)v;
	g_regs[off / 4 + 1] = (uint32_t)(v >> 32);
}

/* ---- the OS services ---------------------------------------------------- */

static bool g_verbose;
static unsigned g_err_lines;

static void env_busy_wait_us(void *ctx, uint32_t us)
{
	(void)ctx;
	if (g_trace)
		fprintf(g_trace, "D %u\n", us);
}

static void env_sleep_ms(void *ctx, uint32_t ms)
{
	(void)ctx;
	(void)ms;
}

static void env_log(void *ctx, uint32_t level, const uint8_t *buf, size_t len)
{
	(void)ctx;
	if (level == 1)
		g_err_lines++;
	if (g_verbose)
		printf("  log%u: %.*s\n", level, (int)len, (const char *)buf);
}

static int env_i2c_write(void *ctx, uint32_t bus, const uint8_t *buf, size_t len)
{
	(void)ctx;
	(void)bus;
	(void)buf;
	(void)len;
	return -134;
}

static int env_i2c_write_read(void *ctx, uint32_t bus, const uint8_t *wbuf, size_t wlen,
			      uint8_t *rbuf, size_t rlen)
{
	(void)ctx;
	(void)bus;
	(void)wbuf;
	(void)wlen;
	(void)rbuf;
	(void)rlen;
	return -134;
}

static int env_gpio_set(void *ctx, uint32_t line, uint32_t value)
{
	(void)ctx;
	(void)line;
	(void)value;
	return -134;
}

/* The crate's panic handler calls this (yd_zephyr.c on the board). */
void yd_panic(const uint8_t *file, size_t file_len, uint32_t line)
{
	printf("FAIL Rust panic at %.*s:%u\n", (int)file_len, (const char *)file, line);
	exit(3);
}

/* The personality routine the host's precompiled `core` names is the crate's
 * own (driver/rust/src/lib.rs, eh_personality): rustc mangles its name since
 * mid-2025, so a C definition here no longer matches it. */

static const struct yd_env g_env = {
	.ctx = NULL,
	.busy_wait_us = env_busy_wait_us,
	.sleep_ms = env_sleep_ms,
	.log = env_log,
	.i2c_write = env_i2c_write,
	.i2c_write_read = env_i2c_write_read,
	.gpio_set = env_gpio_set,
};

static const struct yd_pvr_bus g_bus = {
	.ctx = NULL,
	.read32 = bus_read32,
	.write32 = bus_write32,
	.barrier = bus_barrier,
};

/* The inputs both the old sequence's recording (tools/pvr/rgxstart/README.md)
 * and this one use: the kernel page catalogue at 0xe0300000, the firmware
 * code at the bottom of the firmware heap, its data 0x80000 above, an
 * 800 MHz core clock. */
#define TEST_PC_PHYS 0xe0300000ull
static struct yd_pvr_layer g_layer = {
	.regs = 0,
	.env = &g_env,
	.bus = &g_bus,
	.kernel_pc = TEST_PC_PHYS,
	.fw_code_devva = RGX_FIRMWARE_RAW_HEAP_BASE,
	.fw_data_devva = RGX_FIRMWARE_RAW_HEAP_BASE + 0x80000,
	.core_clock_hz = 800000000,
	.poll_timeout_us = 0,
	.poll_step_us = 0,
	.powered = 0,
};

static FILE *open_out(const char *dir, const char *name)
{
	char path[4096];
	FILE *f;

	snprintf(path, sizeof(path), "%s/%s", dir, name);
	f = fopen(path, "w");
	if (!f) {
		printf("FAIL cannot write %s\n", path);
		exit(2);
	}
	return f;
}

/* ---- 1. the core ---------------------------------------------------------- */

static void check_core(void)
{
#define FEAT(name, mask, present)                                                       \
	CHECK(RGXDeviceHasFeature(&g_layer, RGX_FEATURE_##name##_ARRAY_INDEX, mask) == (bool)(present), \
	      "feature " #name ": rgxlayer says %d, the configuration %d",                \
	      RGXDeviceHasFeature(&g_layer, RGX_FEATURE_##name##_ARRAY_INDEX, mask), (present));
	PVR_RGX_FEATURE_BITS(FEAT)
#undef FEAT
#define ERR(name, mask, present)                                                         \
	CHECK(RGXDeviceHasErnBrn(&g_layer, mask) == (bool)(present),                     \
	      #name ": rgxlayer says %d, the core header %d", RGXDeviceHasErnBrn(&g_layer, mask), (present));
	PVR_RGX_ERRATA(ERR)
#undef ERR
#define VAL(name, idx, value)                                                            \
	CHECK(RGXDeviceGetFeatureValue(&g_layer, idx) == (value),                         \
	      "value " #name ": rgxlayer says %d, the configuration %d",                  \
	      RGXDeviceGetFeatureValue(&g_layer, idx), (value));
	PVR_RGX_FEATURE_VALUES(VAL)
#undef VAL
	CHECK(RGXDeviceGetFeatureValue(&g_layer, RGX_FEATURE_WITH_VALUES_MAX_IDX) == -1, "index past the table");
	CHECK(RGXGetDeviceSLCBanks(&g_layer) == (IMG_UINT32)PVR_RGX_VALUE_SLC_BANKS, "SLC banks");
	CHECK(RGXGetDeviceCacheLineSize(&g_layer) == (IMG_UINT32)PVR_RGX_VALUE_SLC_CACHE_LINE_SIZE_BITS,
	      "SLC line");
	CHECK(RGXGetDevicePhysBusWidth(&g_layer) == (IMG_UINT32)PVR_RGX_VALUE_PHYS_BUS_WIDTH, "bus width");
	CHECK(RGXGetFWCorememSize(&g_layer) == 0, "coremem");
}

/* ---- 2./3. start and stop ------------------------------------------------- */

static void check_start(const char *outdir)
{
	PVRSRV_ERROR e;

	memset(g_regs, 0, sizeof(g_regs));
	/* SLC_CTRL_MISC as found: the PSG hazard check enabled plus other bits
	 * the start must drop (only that one bit is kept). */
	g_regs[RGX_CR_SLC_CTRL_MISC / 4] = RGX_CR_SLC_CTRL_MISC_ENABLE_PSG_HAZARD_CHECK_EN | 0x00f00000u;
	g_layer.powered = 0;
	g_err_lines = 0;
	g_trace = open_out(outdir, "rgxstart.trace");
	e = RGXStart(&g_layer);
	fclose(g_trace);
	g_trace = NULL;
	CHECK(e == PVRSRV_OK, "RGXStart returned %d", (int)e);
	CHECK(g_layer.powered == 1, "RGXStart did not set the powered state");
	CHECK(g_err_lines == 0, "RGXStart logged %u errors", g_err_lines);
	CHECK(g_regs[RGX_CR_FWCORE_BOOT / 4] == 1, "FWCORE_BOOT not released");
	CHECK(g_regs[RGX_CR_SOFT_RESET / 4] == 0 && g_regs[RGX_CR_SOFT_RESET / 4 + 1] == 0,
	      "soft reset left asserted");
	CHECK(g_regs[RGX_CR_BIF_CAT_BASE0 / 4] == (uint32_t)TEST_PC_PHYS, "CAT_BASE0");
	CHECK(g_regs[RGX_CR_FWCORE_MEM_CAT_BASE0 / 4] == (uint32_t)TEST_PC_PHYS, "FWCORE_MEM_CAT_BASE0");
}

static void check_stop(const char *outdir)
{
	PVRSRV_ERROR e;

	/* An idle GPU: MARS idle (the FW core and system arbiter), every BIF and
	 * SLC status 0. A pending OS0 event the stop acknowledges. */
	memset(g_regs, 0, sizeof(g_regs));
	g_regs[RGX_CR_MARS_IDLE / 4] = RGX_CR_MARS_IDLE_CPU_EN | RGX_CR_MARS_IDLE_MH_SYSARB0_EN;
	g_regs[RGX_CR_IRQ_OS0_EVENT_STATUS / 4] = RGX_CR_IRQ_OS0_EVENT_STATUS_SOURCE_EN;
	g_layer.powered = 1;
	g_err_lines = 0;
	g_trace = open_out(outdir, "rgxstop.trace");
	e = RGXStop(&g_layer);
	fclose(g_trace);
	g_trace = NULL;
	CHECK(e == PVRSRV_OK, "RGXStop returned %d", (int)e);
	CHECK(g_layer.powered == 0, "RGXStop did not clear the powered state");
	CHECK(g_err_lines == 0, "RGXStop logged %u errors", g_err_lines);
	CHECK(g_regs[RGX_CR_IRQ_OS0_EVENT_CLEAR / 4] == RGX_CR_IRQ_OS0_EVENT_CLEAR_SOURCE_EN,
	      "the pending OS0 event was not cleared");

	/* A firmware core that never idles: a bounded wait, then the error. */
	memset(g_regs, 0, sizeof(g_regs));
	g_layer.poll_timeout_us = 200;
	g_layer.poll_step_us = 50;
	g_err_lines = 0;
	e = RGXStop(&g_layer);
	g_layer.poll_timeout_us = 0;
	g_layer.poll_step_us = 0;
	CHECK(e == PVRSRV_ERROR_TIMEOUT, "RGXStop on a busy GPU returned %d", (int)e);
	CHECK(g_err_lines == 1, "the timeout logged %u lines", g_err_lines);
	(void)poke64;
}

/* ---- 4./5. the image loader --------------------------------------------- */

#define HEAP_SIZE 0x100000u
#define CODE_OFF 0x0u
#define DATA_OFF 0x80000u   /* value from DDK capture (the board's heap layout) */
#define RODATA_OFF 0xa0000u /* DATA_OFF + the RW data section's maximum size */

/* Where the driver's loader before this change put a byte of segment memory:
 * code at heap 0 + the entry's allocation offset, RW and RO data at heap
 * 0x80000 + (vaddr - RGXRISCVFW_BOOTLDR_DATA_BASE). */
static long old_dst(const RGX_FW_LAYOUT_ENTRY *t, unsigned n, uint32_t vaddr)
{
	for (unsigned i = 0; i < n; i++) {
		if (vaddr >= t[i].ui32BaseAddr && vaddr < t[i].ui32BaseAddr + t[i].ui32AllocSize) {
			if (t[i].eType == FW_CODE)
				return (long)(CODE_OFF + t[i].ui32AllocOffset + (vaddr - t[i].ui32BaseAddr));
			return (long)(DATA_OFF + (vaddr - RGXRISCVFW_BOOTLDR_DATA_BASE));
		}
	}
	return -1;
}

static void check_image(const uint8_t *fw, uint32_t size, const char *what)
{
	IMG_DEVMEM_SIZE_T code = 0, data = 0, rodata = 0, cmc = 0, cmd = 0;
	RGX_FW_INFO_HEADER hdr;
	PVRSRV_FW_BOOT_PARAMS params;
	const RGX_FW_INFO_HEADER *ih = (const void *)(fw + size - FW_BLOCK_SIZE);
	const RGX_FW_LAYOUT_ENTRY *lt = (const void *)(fw + size - FW_BLOCK_SIZE + ih->ui32HeaderLen);
	const IMG_ELF_HDR *eh = (const void *)fw;
	uint8_t *heap = malloc(HEAP_SIZE), *want = malloc(HEAP_SIZE);
	PVRSRV_ERROR e;
	unsigned bad = 0;

	memset(heap, 0xa5, HEAP_SIZE);
	memset(want, 0xa5, HEAP_SIZE);
	g_err_lines = 0;
	e = RGXGetFWImageAllocSize(&g_layer, fw, size, &code, &data, &rodata, &cmc, &cmd, &hdr);
	CHECK(e == PVRSRV_OK, "%s: RGXGetFWImageAllocSize returned %d", what, (int)e);
	CHECK(code == 0x30000 && data == 0x3000 && rodata == 0x1000 && cmc == 0 && cmd == 0,
	      "%s: sizes code 0x%llx data 0x%llx rodata 0x%llx coremem 0x%llx/0x%llx", what,
	      (unsigned long long)code, (unsigned long long)data, (unsigned long long)rodata,
	      (unsigned long long)cmc, (unsigned long long)cmd);
	CHECK(RGXGetFWImageSectionMaxSize(&g_layer, RISCV_PRIVATE_DATA) == RODATA_OFF - DATA_OFF,
	      "%s: the RW data section's maximum size is not 0x%x", what, RODATA_OFF - DATA_OFF);
	CHECK(RGXGetFWImageSectionAddress(&g_layer, RISCV_PRIVATE_RODATA) - RGXRISCVFW_BOOTLDR_DATA_BASE ==
		      RODATA_OFF - DATA_OFF,
	      "%s: RO data is not where the former loader put it", what);

	/* The former loader's result. */
	for (unsigned i = 0; i < eh->ui32Ephnum; i++) {
		const IMG_ELF_PROGRAM_HDR *ph = (const void *)(fw + eh->ui32Ephoff + i * eh->ui32Ephentsize);
		long d;

		if (ph->ui32Ptype != ELF_PT_LOAD)
			continue;
		d = old_dst(lt, ih->ui32LayoutEntryNum, ph->ui32Pvaddr);
		CHECK(d >= 0, "%s: no layout entry for 0x%x", what, ph->ui32Pvaddr);
		if (d < 0)
			continue;
		memcpy(want + d, fw + ph->ui32Poffset, ph->ui32Pfilesz);
		memset(want + d + ph->ui32Pfilesz, 0, ph->ui32Pmemsz - ph->ui32Pfilesz);
	}
	memset(want + DATA_OFF + RGXRISCVFW_BOOTLDR_CONF_OFFSET, 0, sizeof(RGXRISCVFW_BOOT_DATA));

	/* The DDK's. */
	memset(&params, 0, sizeof(params));
	e = RGXProcessFWImage(&g_layer, fw, heap + CODE_OFF, heap + DATA_OFF, heap + RODATA_OFF, NULL, NULL,
			      &params);
	CHECK(e == PVRSRV_OK, "%s: RGXProcessFWImage returned %d", what, (int)e);
	for (uint32_t i = 0; i < HEAP_SIZE; i++)
		bad += heap[i] != want[i];
	CHECK(bad == 0, "%s: %u heap bytes differ from the former loader's", what, bad);
	CHECK(g_err_lines == 0, "%s: %u error lines", what, g_err_lines);
	free(heap);
	free(want);
}

/* A firmware image with the shape of rgx.fw.36.53.104.796 (its ELF program
 * headers and layout table, measured): every byte of a segment is a function
 * of its position, so a misplaced copy shows. */
static uint8_t *synth_image(uint32_t *size_out)
{
	static const struct {
		uint32_t vaddr, filesz, memsz;
	} seg[] = {
		{ 0x30000000, 0x10000, 0x10000 },
		{ 0xa0000000, 0x2800, 0x3000 }, /* a bss tail */
		{ 0xa0020000, 0x1000, 0x1000 },
		{ 0xb0000000, 0x20000, 0x20000 },
	};
	static const RGX_FW_LAYOUT_ENTRY layout[] = {
		{ RISCV_UNCACHED_CODE, FW_CODE, 0x30000000, 0x10000000, 0x10000, 0x0 },
		{ RISCV_CACHED_CODE, FW_CODE, 0xb0000000, 0x10000000, 0x20000, 0x10000 },
		{ RISCV_PRIVATE_DATA, FW_RWDATA, 0xa0000000, 0x20000, 0x3000, 0x0 },
		{ RISCV_PRIVATE_RODATA, FW_RODATA, 0xa0020000, 0xffe0000, 0x1000, 0x0 },
	};
	const uint32_t nph = 5, phoff = sizeof(IMG_ELF_HDR);
	uint32_t off = 0x1000, size;
	uint8_t *fw;
	IMG_ELF_HDR *eh;
	RGX_FW_INFO_HEADER *ih;

	size = 0x1000;
	for (unsigned i = 0; i < 4; i++)
		size += seg[i].filesz;
	size += FW_BLOCK_SIZE;
	fw = calloc(1, size);
	eh = (void *)fw;
	memcpy(eh->ui32Eident, "\x7f" "ELF\x01\x01\x01", 7);
	eh->ui32Etype = 2;
	eh->ui32Emachine = 0xf3;
	eh->ui32Ephoff = phoff;
	eh->ui32Ephentsize = sizeof(IMG_ELF_PROGRAM_HDR);
	eh->ui32Ephnum = nph;
	{
		IMG_ELF_PROGRAM_HDR *ph = (void *)(fw + phoff);

		ph[0].ui32Ptype = 0x70000003; /* RISCV_ATTRIBUTES, not loaded */
		for (unsigned i = 0; i < 4; i++) {
			ph[i + 1].ui32Ptype = ELF_PT_LOAD;
			ph[i + 1].ui32Poffset = off;
			ph[i + 1].ui32Pvaddr = seg[i].vaddr;
			ph[i + 1].ui32Ppaddr = seg[i].vaddr;
			ph[i + 1].ui32Pfilesz = seg[i].filesz;
			ph[i + 1].ui32Pmemsz = seg[i].memsz;
			for (uint32_t b = 0; b < seg[i].filesz; b++)
				fw[off + b] = (uint8_t)((b * 7u + i * 31u + (b >> 8)) | 1u);
			/* The bootloader's data block at the start of the RW data is
			 * zero in the image, as in rgx.fw.36.53.104.796. The former
			 * loader zeroed all of it; RGXProcessFWImage writes every field
			 * but ui32Reserved, which so stays the image's zero. */
			if (seg[i].vaddr == RGXRISCVFW_BOOTLDR_DATA_BASE)
				memset(fw + off + RGXRISCVFW_BOOTLDR_CONF_OFFSET, 0, sizeof(RGXRISCVFW_BOOT_DATA));
			off += seg[i].filesz;
		}
	}
	ih = (void *)(fw + size - FW_BLOCK_SIZE);
	ih->ui32InfoVersion = FW_INFO_VERSION;
	ih->ui32HeaderLen = sizeof(RGX_FW_INFO_HEADER);
	ih->ui32LayoutEntryNum = 4;
	ih->ui32LayoutEntrySize = sizeof(RGX_FW_LAYOUT_ENTRY);
	ih->ui16PVRVersionMajor = FW_VERSION_MAJ;
	ih->ui16PVRVersionMinor = FW_VERSION_MIN;
	memcpy((uint8_t *)ih + ih->ui32HeaderLen, layout, sizeof(layout));
	*size_out = size;
	return fw;
}

static uint8_t *read_file(const char *path, uint32_t *size_out)
{
	FILE *f = fopen(path, "rb");
	uint8_t *b;
	long n;

	if (!f) {
		printf("FAIL cannot read %s\n", path);
		exit(2);
	}
	fseek(f, 0, SEEK_END);
	n = ftell(f);
	fseek(f, 0, SEEK_SET);
	b = malloc((size_t)n);
	if (fread(b, 1, (size_t)n, f) != (size_t)n) {
		printf("FAIL short read of %s\n", path);
		exit(2);
	}
	fclose(f);
	*size_out = (uint32_t)n;
	return b;
}

int main(int argc, char **argv)
{
	uint32_t size;
	uint8_t *fw;

	if (argc < 2) {
		fprintf(stderr, "usage: %s <outdir> [<firmware image>]\n", argv[0]);
		return 2;
	}
	g_verbose = getenv("RGXLAYER_CHECK_VERBOSE") != NULL;
	check_core();
	check_start(argv[1]);
	check_stop(argv[1]);
	fw = synth_image(&size);
	check_image(fw, size, "synthetic image");
	free(fw);
	if (argc > 2) {
		fw = read_file(argv[2], &size);
		check_image(fw, size, argv[2]);
		free(fw);
	}
	/* 43 feature bits + 20 errata + 30 values + 5 more of the core, 7 of
	 * the start, 6 of the stop, 11 per image. */
	unsigned planned = 98 + 7 + 6 + 11 + (argc > 2 ? 11 : 0);

	printf("rgxlayer_check: %u checks run (planned %u), %u failed\n", g_run, planned, g_failed);
	return g_failed || g_run != planned ? 1 : 0;
}
