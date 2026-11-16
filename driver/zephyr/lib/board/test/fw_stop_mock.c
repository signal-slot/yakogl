/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * fw_stop_mock: fw_stop_plan on all running, partly running and nothing
 * running snapshots; order, masks, citations and idempotence (the plan,
 * applied to a register model, leaves a state whose plan is empty). Built by
 * tools/v3d/check71.d/fw_stop.sh.
 */
#include <stdio.h>
#include <string.h>

#include "fw_stop.h"

static int checks, failed;

#define CHECK(c)                                                              \
	do {                                                                  \
		checks++;                                                     \
		if (!(c)) {                                                   \
			failed++;                                             \
			printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c);   \
		}                                                             \
	} while (0)

/* Apply a plan to the snapshot as the registers would change. */
static void apply(struct fw_stop_snap *s, const struct fw_stop_write *w, int n)
{
	uint32_t c0 = 0;

	fw_stop_hvs_ctrl0_off(s->hvs_gen, s->hvs_chan, &c0);
	for (int i = 0; i < n; i++) {
		uint32_t *r = NULL;

		if (w[i].block == FW_STOP_HDMI_HD && w[i].offset == FW_STOP_HD_VID_CTL)
			r = &s->hdmi_vid_ctl;
		else if (w[i].block == FW_STOP_PV && w[i].offset == FW_STOP_PV_CONTROL)
			r = &s->pv_control;
		else if (w[i].block == FW_STOP_PV && w[i].offset == FW_STOP_PV_V_CONTROL)
			r = &s->pv_vcontrol;
		else if (w[i].block == FW_STOP_HVS && w[i].offset == c0)
			r = &s->hvs_ctrl0;
		else if (w[i].block == FW_STOP_HVS &&
			 w[i].offset == FW_STOP_HVS_DLIST_OFFSET + 4u * (s->hvs_dl & 0xfffu))
			r = &s->hvs_dl_word0;
		CHECK(r != NULL);
		if (r)
			*r = (*r & ~w[i].mask) | (w[i].value & w[i].mask);
	}
}

static struct fw_stop_snap all_running(enum fw_stop_hvs_gen g, uint32_t ch)
{
	struct fw_stop_snap s = {
		.hdmi_vid_ctl = FW_STOP_HD_VID_CTL_ENABLE | 0x1234u,
		.pv_control = FW_STOP_PV_CONTROL_EN | 0x40u,
		.pv_vcontrol = FW_STOP_PV_VCONTROL_VIDEN,
		.hvs_gen = g,
		.hvs_chan = ch,
		.hvs_ctrl0 = FW_STOP_HVS_CTRL0_ENB | 0x438u,
		.hvs_dl = 0x10u,
		.hvs_dl_word0 = 0x40000008u,
	};
	return s;
}

static void check_all(enum fw_stop_hvs_gen g, uint32_t ch, uint32_t want_c0)
{
	struct fw_stop_snap s = all_running(g, ch);
	struct fw_stop_write w[FW_STOP_MAX_WRITES];
	int n = fw_stop_plan(&s, w, FW_STOP_MAX_WRITES);

	CHECK(fw_stop_is_needed(&s));
	CHECK(n == 5);
	if (n != 5)
		return;
	/* order: HDMI, PV (video enable, then enable), HVS channel, list */
	CHECK(w[0].block == FW_STOP_HDMI_HD && w[0].offset == FW_STOP_HD_VID_CTL);
	CHECK(w[0].mask == (1u << 31) && w[0].value == 0 && w[0].wait_us > 0);
	CHECK(w[1].block == FW_STOP_PV && w[1].offset == FW_STOP_PV_V_CONTROL);
	CHECK(w[2].block == FW_STOP_PV && w[2].offset == FW_STOP_PV_CONTROL);
	CHECK(w[2].mask == 1u && w[2].value == 0 && w[2].wait_us > 0);
	CHECK(w[3].block == FW_STOP_HVS && w[3].offset == want_c0);
	CHECK(w[3].mask == (1u << 31) && w[3].value == 0);
	CHECK(w[4].block == FW_STOP_HVS && w[4].offset == 0x4000u + 4u * 0x10u);
	CHECK(w[4].value == (1u << 31) && w[4].mask == 0xffffffffu);
	for (int i = 0; i < n; i++) {
		CHECK(w[i].cite && (strstr(w[i].cite, "rpi:") || strstr(w[i].cite, "facts") ||
				    strstr(w[i].cite, "unverified")));
		CHECK(w[i].value == (w[i].value & w[i].mask));
	}
	/* idempotence: after applying, nothing runs, the plan is empty, the
	 * bits outside the masks are kept */
	apply(&s, w, n);
	CHECK(!fw_stop_is_needed(&s));
	CHECK(fw_stop_plan(&s, w, FW_STOP_MAX_WRITES) == 0);
	CHECK(s.hdmi_vid_ctl == 0x1234u && s.pv_control == 0x40u && s.hvs_ctrl0 == 0x438u);
}

int main(void)
{
	struct fw_stop_write w[FW_STOP_MAX_WRITES];
	struct fw_stop_snap s;
	int n;

	check_all(FW_STOP_HVS_6C, 0, 0x30);
	check_all(FW_STOP_HVS_6C, 2, 0x30 + 2 * 0x20);
	check_all(FW_STOP_HVS_6D, 1, 0x100 + 0x40);

	/* nothing running */
	memset(&s, 0, sizeof(s));
	s.hvs_gen = FW_STOP_HVS_6D;
	s.hvs_dl_word0 = 0x12345678u;
	CHECK(!fw_stop_is_needed(&s));
	CHECK(fw_stop_plan(&s, w, FW_STOP_MAX_WRITES) == 0);
	CHECK(fw_stop_plan(&s, w, 0) == 0); /* an empty plan needs no room */

	/* only HDMI running: one write */
	s.hdmi_vid_ctl = FW_STOP_HD_VID_CTL_ENABLE;
	n = fw_stop_plan(&s, w, FW_STOP_MAX_WRITES);
	CHECK(n == 1 && w[0].block == FW_STOP_HDMI_HD);
	/* only the pixel valve */
	s.hdmi_vid_ctl = 0;
	s.pv_control = FW_STOP_PV_CONTROL_EN;
	n = fw_stop_plan(&s, w, FW_STOP_MAX_WRITES);
	CHECK(n == 1 && w[0].block == FW_STOP_PV && w[0].offset == FW_STOP_PV_CONTROL);
	/* HVS running on an already empty list: channel off, no END write */
	s.pv_control = 0;
	s.hvs_ctrl0 = FW_STOP_HVS_CTRL0_ENB;
	s.hvs_dl_word0 = FW_STOP_HVS_CTL0_END;
	n = fw_stop_plan(&s, w, FW_STOP_MAX_WRITES);
	CHECK(n == 1 && w[0].block == FW_STOP_HVS && w[0].offset == 0x100u);
	/* HDMI and HVS, PV already off: HDMI first, then the channel, then the list */
	s.hdmi_vid_ctl = FW_STOP_HD_VID_CTL_ENABLE;
	s.hvs_dl_word0 = 0x4000000au;
	n = fw_stop_plan(&s, w, FW_STOP_MAX_WRITES);
	CHECK(n == 3 && w[0].block == FW_STOP_HDMI_HD && w[1].block == FW_STOP_HVS &&
	      w[2].block == FW_STOP_HVS && w[1].offset < FW_STOP_HVS_DLIST_OFFSET &&
	      w[2].offset >= FW_STOP_HVS_DLIST_OFFSET);
	/* the list-active field is masked to 12 bits */
	s.hvs_dl = 0xabcd0010u;
	n = fw_stop_plan(&s, w, FW_STOP_MAX_WRITES);
	CHECK(n == 3 && w[2].offset == 0x4040u);

	/* errors */
	CHECK(fw_stop_plan(NULL, w, FW_STOP_MAX_WRITES) == FW_STOP_ERR_ARG);
	CHECK(fw_stop_plan(&s, NULL, FW_STOP_MAX_WRITES) == FW_STOP_ERR_ARG);
	s = all_running(FW_STOP_HVS_6C, 3);
	CHECK(fw_stop_plan(&s, w, FW_STOP_MAX_WRITES) == FW_STOP_ERR_CHAN);
	s = all_running((enum fw_stop_hvs_gen)0, 0);
	CHECK(fw_stop_plan(&s, w, FW_STOP_MAX_WRITES) == FW_STOP_ERR_GEN);
	s = all_running(FW_STOP_HVS_6C, 0);
	CHECK(fw_stop_plan(&s, w, 4) == FW_STOP_ERR_BUFFER);
	CHECK(!fw_stop_is_needed(NULL));

	printf("fw_stop_mock: %d of %d checks, %d failed\n", checks - failed, checks, failed);
	return failed ? 1 : 0;
}
