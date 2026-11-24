/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * imx95_lvds_ids: is the X12 LVDS path there, and what do its parts hold
 * before the cold run writes them (README.md)? Read only, with one SCMI
 * write: the DISPLAY domain on (and back off if it was off).
 *   1. SCMI, read only: DISPLAY's state, the DISP level, and for the LDB
 *      PLL (37, 38), DISP1PIX (80), TPM5 (154, the X12 backlight's PWM) and
 *      DISPAPB (75): CLOCK_ATTRIBUTES (enabled), CLOCK_RATE_GET and
 *      CLOCK_GET_PERMISSIONS (state, parent, rate: what AP-NS may set)
 *   2. DISPLAY (13) on over SCMI (POWER_STATE_SET, read back)
 *   3. one 32-bit read each, the LDB PLL untouched: the LVDS CSR's four
 *      words; the display CSR 0x00..0x20; the DPU's COMCTRL_IPIDENTIFIER
 *      and the words the cold path's defaults and Linux disagree on or
 *      leave at reset (FetchLayer0 BURSTBUFFERMANAGEMENT and LAYERPROPERTY0,
 *      FetchYUV0 BURSTBUFFERMANAGEMENT, FrameGen0 SKICKCONFIG,
 *      SECSTATCONFIG, FGINCTRLPANIC and FGCCR, Dither0 POLARITYCTRL0); the
 *      interleaver's channel 0 CTRL. None is a write-only or reserved word
 *      (driver/docs/imx95-display-allow.txt, #@noread).
 *   4. DISPLAY back as it was found
 * then the verdict and the PSCI reset. TPM5 and GPIO5 are not read: their
 * clocks may be off (TPM5's state is in step 1's line).
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/sys_io.h>
#include <errno.h>

#include <yakogl_drivers.h>

#include "vi95_probe.h"

#define PD_DISPLAY 13u
#define PERF_DISP  12u
#define POWER_ON   0u
#define POWER_OFF  0x40000000u

#define PROTO_POWER 0x11
#define PROTO_PERF  0x13
#define PROTO_CLOCK 0x14

/* the DPU's identity in the board's Linux dump (2026-11-25) */
#define DPU_ID_DUMP 0x21260400u

/* CLOCK_GET_PERMISSIONS (NXP's 0xf): state, parent, rate */
#define PERM_STATE  (1u << 31)
#define PERM_PARENT (1u << 30)
#define PERM_RATE   (1u << 29)

static int pd_state(uint32_t pd, uint32_t *state)
{
	int32_t st;
	int n = vi95_scmi(PROTO_POWER, 0x5, &pd, 1, state, 1, &st);

	return n < 0 ? n : n < 1 ? -EBADMSG : st != 0 ? (int)st : 0;
}

static int pd_set(uint32_t pd, uint32_t state)
{
	uint32_t w[3] = { 0, pd, state }, got = 0, r[1];
	int32_t st;
	int n = vi95_scmi(PROTO_POWER, 0x4, w, 3, r, 0, &st);

	if (n < 0) {
		return n;
	}
	if (st != 0) {
		VI95_P("pd %u set 0x%08x = %s (%d)", pd, state, vi95_scmi_status(st), st);
		return st;
	}
	n = pd_state(pd, &got);
	VI95_P("pd %u set 0x%08x, reads 0x%08x (rc %d)", pd, state, got, n);
	return n != 0 ? n : got == state ? 0 : -EIO;
}

/* One clock's state, rate and permissions; returns the permissions (0 on a
 * refusal). */
static uint32_t clock_facts(uint32_t clk, const char *what)
{
	uint32_t a[5] = { 0 }, r[2] = { 0 }, p[1] = { 0 };
	int32_t sa = 0, sr = 0, sp = 0;
	int na = vi95_scmi(PROTO_CLOCK, 0x3, &clk, 1, a, 5, &sa);
	int nr = vi95_scmi(PROTO_CLOCK, 0x6, &clk, 1, r, 2, &sr);
	int np = vi95_scmi(PROTO_CLOCK, 0xf, &clk, 1, p, 1, &sp);

	VI95_P("clk %u (%s) = %s, %llu Hz, permissions 0x%08x (%s%s%s) [rc %d %d %d, %s %s %s]",
	       clk, what, na >= 1 && sa == 0 ? (a[0] & 1 ? "on" : "off") : "?",
	       nr >= 2 && sr == 0 ? ((unsigned long long)r[1] << 32 | r[0]) : 0ull,
	       np >= 1 && sp == 0 ? p[0] : 0u, np >= 1 && sp == 0 && (p[0] & PERM_STATE) ? "state " : "",
	       np >= 1 && sp == 0 && (p[0] & PERM_PARENT) ? "parent " : "",
	       np >= 1 && sp == 0 && (p[0] & PERM_RATE) ? "rate" : "", na, nr, np,
	       vi95_scmi_status(sa), vi95_scmi_status(sr), vi95_scmi_status(sp));
	return np >= 1 && sp == 0 ? p[0] : 0u;
}

/* `n` words at `phys + offs[i]`, one 32-bit read each, as "<what> 0xADDR =
 * 0xVALUE"; the values into `out`. */
static void words(const char *what, uintptr_t phys, size_t size, const uint32_t *offs, size_t n,
		  uint32_t *out)
{
	mm_reg_t base;

	device_map(&base, phys, size, K_MEM_CACHE_NONE);
	for (size_t i = 0; i < n; i++) {
		out[i] = sys_read32(base + offs[i]);
		VI95_P("%s 0x%08lx = 0x%08x", what, (unsigned long)(phys + offs[i]), out[i]);
	}
}

int main(void)
{
	static const uint32_t ldb_offs[] = { 0x0, 0x4, 0x8, 0xc };
	static const uint32_t csr_offs[] = { 0x00, 0x04, 0x08, 0x0c, 0x10, 0x14, 0x18, 0x1c, 0x20 };
	/* COMCTRL_IPIDENTIFIER; FetchLayer0 BURSTBUFFERMANAGEMENT, LAYERPROPERTY0;
	 * FetchYUV0 BURSTBUFFERMANAGEMENT; FrameGen0 SKICKCONFIG, SECSTATCONFIG,
	 * FGINCTRLPANIC, FGCCR; Dither0 POLARITYCTRL0 */
	static const uint32_t dpu_offs[] = { 0x0,       0x1d0010, 0x1d0048, 0x20000c, 0x2b0030,
					     0x2b0034, 0x2b0084, 0x2b0088, 0x311008 };
	static const uint32_t il_offs[] = { 0x0 };
	uint32_t found = 0, level = 0, ldb[4] = { 0 }, csr[9], dpu[9] = { 0 }, il[1];
	uint32_t p37, p38, p154;
	int32_t st;
	bool on = false;
	int rc;

	vi95_probe_banner();

	/* 1. the SCMI side, read only */
	rc = pd_state(PD_DISPLAY, &found);
	VI95_P("before pd %u state = 0x%08x (rc %d)", PD_DISPLAY, found, rc);
	rc = vi95_scmi(PROTO_PERF, 0x8, (const uint32_t[]){ PERF_DISP }, 1, &level, 1, &st);
	VI95_P("perf %u level = %u (rc %d, %s)", PERF_DISP, level, rc, vi95_scmi_status(st));
	p37 = clock_facts(37, "LDBPLL_VCO");
	p38 = clock_facts(38, "LDBPLL");
	(void)clock_facts(80, "DISP1PIX");
	p154 = clock_facts(154, "TPM5");
	(void)clock_facts(75, "DISPAPB");

	/* 2. DISPLAY on */
	on = pd_set(PD_DISPLAY, POWER_ON) == 0;
	if (!on) {
		VI95_P("DISPLAY did not come on: no register is read");
	} else {
		/* 3. the blocks, read only */
		words("ldb", YD_IMX95_LDB_PHYS, YD_IMX95_LDB_SIZE, ldb_offs, ARRAY_SIZE(ldb_offs),
		      ldb);
		words("csr", YD_IMX95_DISPLAY_CSR_PHYS, YD_IMX95_DISPLAY_CSR_SIZE, csr_offs,
		      ARRAY_SIZE(csr_offs), csr);
		words("dpu", YD_IMX95_DPU_PHYS, YD_IMX95_DPU_SIZE, dpu_offs, ARRAY_SIZE(dpu_offs),
		      dpu);
		words("il", YD_IMX95_INTERLEAVER_PHYS, YD_IMX95_INTERLEAVER_SIZE, il_offs,
		      ARRAY_SIZE(il_offs), il);
		VI95_P("lvds csr = 0x%08x 0x%08x 0x%08x 0x%08x", ldb[0], ldb[1], ldb[2], ldb[3]);
	}

	/* 4. DISPLAY as found */
	if (on && found == POWER_OFF) {
		(void)pd_set(PD_DISPLAY, POWER_OFF);
	}

	if (!on) {
		VI95_P("FAIL DISPLAY did not come on over SCMI");
	} else if ((p37 & (PERM_STATE | PERM_RATE)) != (PERM_STATE | PERM_RATE) ||
		   (p38 & (PERM_STATE | PERM_RATE)) != (PERM_STATE | PERM_RATE) ||
		   !(p154 & PERM_STATE)) {
		VI95_P("FAIL permissions: LDBPLL_VCO 0x%08x, LDBPLL 0x%08x (state and rate "
		       "needed), TPM5 0x%08x (state needed)",
		       p37, p38, p154);
	} else {
		VI95_P("PASS lvds csr 0x%08x 0x%08x 0x%08x 0x%08x, dpu id 0x%08x (%s), LDB PLL "
		       "and TPM5 permitted",
		       ldb[0], ldb[1], ldb[2], ldb[3], dpu[0],
		       dpu[0] == DPU_ID_DUMP ? "the dump's" : "NOT the dump's");
	}
	vi95_probe_end();
	return 0;
}
