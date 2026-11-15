/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Host test of rpi5_scanout/scanout_alive.h: the running/stopped judgement
 * from two register samples, for both HVS6 steppings (C0 0x53, D0 0x54).
 *
 * Build: cc -std=c99 -Wall -Wextra -Werror -I../rpi5_scanout scanout_alive_mock.c
 * (tools/v3d/check71.d/scanout_alive.sh). Prints one line per check, then
 * "scanout_alive: N of N checks, 0 failed"; exits nonzero on a failure.
 */
#include <stdio.h>
#include <string.h>

#include "scanout_alive.h"

static int total;
static int failed;

static void check(int ok, const char *what)
{
	total++;
	if (!ok)
		failed++;
	printf("%s: %s\n", ok ? "ok" : "FAIL", what);
}

#define ST(mode, frcnt, yline) \
	(((uint32_t)(mode) << 13) | ((uint32_t)(frcnt) << 16) | (uint32_t)(yline))

static struct sa_sample good(uint32_t version, uint32_t frcnt, uint32_t yline)
{
	struct sa_sample s;

	s.hvs_version = version;
	s.ctrl0 = SA_CTRL0_ENB | (1919u << 16) | 1079u;
	s.status = ST(2, frcnt, yline);
	s.dl = 0x40u;
	s.pv_control = SA_PV_CONTROL_EN | 0x00200000u;
	s.pv_vcontrol = SA_PV_VCONTROL_VIDEN;
	return s;
}

static int line_is(const struct sa_verdict *v, const char *want)
{
	char buf[64];
	int n = sa_format(v, buf, sizeof buf);

	return n == (int)strlen(want) && strcmp(buf, want) == 0;
}

static void run_step(uint32_t version, const char *tag)
{
	struct sa_sample a, b;
	struct sa_verdict v;
	char msg[96];
	int d0 = version == SA_HVS_VERSION_D0;

#define C(ok, what) do { snprintf(msg, sizeof msg, "%s: %s", tag, what); check((ok), msg); } while (0)

	/* running: frame count advances */
	a = good(version, 5, 100);
	b = good(version, 6, 100);
	v = sa_judge(&a, &b, 0);
	C(v.running && v.reason == SA_OK, "running, frame count advances");
	C(v.d0 == d0, "stepping read from VERSION");
	C(line_is(&v, "u3 scanout: running"), "running line");
	C(v.dl_head_words == 0x40u && v.dl_head_off == 0x4000u + 0x100u, "display list head");
	C(v.frcnt_a == 5 && v.frcnt_b == 6, "frame counts reported");

	/* running: only the line moves */
	b = good(version, 5, 700);
	v = sa_judge(&a, &b, 1);
	C(v.running && v.channel == 1, "running, only the line counter moves");

	/* wrap-around of the 6-bit frame count */
	a = good(version, 63, 10);
	b = good(version, 0, 10);
	v = sa_judge(&a, &b, 0);
	C(v.running, "running across the frame count wrap 63 to 0");

	/* frozen */
	a = good(version, 9, 55);
	b = good(version, 9, 55);
	v = sa_judge(&a, &b, 2);
	C(!v.running && v.reason == SA_COUNTER_FROZEN, "frozen counter");
	C(line_is(&v, "u3 scanout: stopped counter frozen"), "frozen line");

	/* channel off: ENB clear */
	a = good(version, 1, 1);
	b = good(version, 2, 1);
	b.ctrl0 &= ~SA_CTRL0_ENB;
	v = sa_judge(&a, &b, 0);
	C(!v.running && v.reason == SA_CHANNEL_OFF, "channel off, ENB clear");
	C(line_is(&v, "u3 scanout: stopped channel off"), "channel off line");

	/* channel off: MODE disabled */
	b = good(version, 2, 1);
	b.status = ST(0, 2, 1);
	v = sa_judge(&a, &b, 0);
	C(!v.running && v.reason == SA_CHANNEL_OFF, "channel off, MODE disabled");

	/* pixel valve off */
	b = good(version, 2, 1);
	b.pv_control &= ~SA_PV_CONTROL_EN;
	v = sa_judge(&a, &b, 0);
	C(!v.running && v.reason == SA_PV_OFF, "pixel valve off, EN clear");
	C(line_is(&v, "u3 scanout: stopped pixel valve off"), "pixel valve off line");
	b = good(version, 2, 1);
	b.pv_vcontrol = 0;
	v = sa_judge(&a, &b, 0);
	C(!v.running && v.reason == SA_PV_OFF, "pixel valve off, VIDEN clear");

	/* the channel is checked before the pixel valve and the counter */
	b = good(version, 1, 1);
	b.ctrl0 = 0;
	b.pv_control = 0;
	v = sa_judge(&a, &b, 0);
	C(v.reason == SA_CHANNEL_OFF, "channel off wins over pixel valve off and frozen");

	/* all ones: unclocked, never running */
	a = good(version, 1, 1);
	b = good(version, 2, 2);
	b.status = 0xffffffffu;
	v = sa_judge(&a, &b, 0);
	C(!v.running && v.reason == SA_UNCLOCKED, "all-ones status is unclocked");
	C(line_is(&v, "u3 scanout: stopped unclocked"), "unclocked line");
	b = good(version, 2, 2);
	b.pv_control = 0xffffffffu;
	b.pv_vcontrol = 0xffffffffu;
	v = sa_judge(&a, &b, 0);
	C(!v.running && v.reason == SA_UNCLOCKED, "all-ones pixel valve is unclocked");
	b = good(version, 2, 2);
	a.ctrl0 = 0xffffffffu;
	v = sa_judge(&a, &b, 0);
	C(!v.running && v.reason == SA_UNCLOCKED, "all-ones in the first sample is unclocked");

	/* register offsets */
	C(sa_reg_ctrl0(d0, 0) == (d0 ? 0x100u : 0x30u), "channel 0 CTRL0 offset");
	C(sa_reg_status(d0, 2) == (d0 ? 0x118u + 0x80u : 0x44u + 0x40u), "channel 2 STATUS offset");
	C(sa_reg_dl(d0, 1) == (d0 ? 0x11cu + 0x40u : 0x48u + 0x20u), "channel 1 DL offset");
#undef C
}

int main(void)
{
	struct sa_sample a, b;
	struct sa_verdict v;
	char small[8];

	run_step(SA_HVS_VERSION_C0, "C0");
	run_step(SA_HVS_VERSION_D0, "D0");

	/* garbage version */
	a = good(0x12, 1, 1);
	b = good(0x12, 2, 1);
	v = sa_judge(&a, &b, 0);
	check(!v.running && v.reason == SA_UNKNOWN_STEPPING, "unknown stepping is stopped");
	check(line_is(&v, "u3 scanout: stopped unknown stepping"), "unknown stepping line");

	/* version differing between samples */
	a = good(SA_HVS_VERSION_C0, 1, 1);
	b = good(SA_HVS_VERSION_D0, 2, 1);
	v = sa_judge(&a, &b, 0);
	check(!v.running && v.reason == SA_UNKNOWN_STEPPING, "stepping change between samples is stopped");

	/* version bits above 7:0 are ignored */
	a = good(0xabcd0053u, 1, 1);
	b = good(0xabcd0053u, 2, 1);
	v = sa_judge(&a, &b, 0);
	check(v.running && !v.d0, "VERSION bits above 7:0 ignored");

	/* all-ones everything */
	memset(&a, 0xff, sizeof a);
	b = a;
	v = sa_judge(&a, &b, 0);
	check(!v.running && v.reason == SA_UNCLOCKED, "all registers all ones");

	/* all zeros (powered block, nothing set) */
	memset(&a, 0, sizeof a);
	b = a;
	v = sa_judge(&a, &b, 0);
	check(!v.running, "all registers zero is not running");

	/* channel out of range */
	a = good(SA_HVS_VERSION_D0, 1, 1);
	b = good(SA_HVS_VERSION_D0, 2, 1);
	v = sa_judge(&a, &b, 3);
	check(!v.running && v.reason == SA_BAD_INPUT, "channel 3 is bad input");

	/* formatter: no newline, truncation follows snprintf */
	v = sa_judge(&a, &b, 0);
	check(sa_format(&v, small, sizeof small) == (int)strlen("u3 scanout: running") &&
	      small[7] == '\0', "formatter truncates into a small buffer");

	printf("scanout_alive: %d of %d checks, %d failed\n", total - failed, total, failed);
	return failed ? 1 : 0;
}
