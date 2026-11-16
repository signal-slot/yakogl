/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_scanout_all: every header of driver/zephyr/lib/board/rpi5_scanout/ in
 * one translation unit, so that a duplicate macro, a clashing type or a
 * missing include guard between the Pi 5 display headers shows as a compile
 * error. The headers are listed in sorted order and included a second time:
 * a header without its include guard clashes in the second pass.
 *
 * main() touches one symbol of every header (a function's address, so that
 * each definition is instantiated and the link sees every header); the
 * program is built and never run.
 *
 * Built by tools/v3d/check71.d/rpi5_scanout_all.sh: with the host cc under
 * -std=c11 -Wall -Wextra -Werror (linked), and for aarch64 under -std=c11
 * -ffreestanding -Wall -Wextra -Werror -c (the Zephyr SDK's
 * aarch64-zephyr-elf-gcc or aarch64-linux-gnu-gcc, the compiler the Pi 5
 * image builds with). A header added to the directory must be added here as
 * well: the script checks that the two lists agree.
 */
#include <stdint.h>

#include "../rpi5_scanout/adopt.h"
#include "../rpi5_scanout/bg_fill.h"
#include "../rpi5_scanout/bus_addr.h"
#include "../rpi5_scanout/dlist_alloc.h"
#include "../rpi5_scanout/edid.h"
#include "../rpi5_scanout/fb_flush.h"
#include "../rpi5_scanout/fb_query.h"
#include "../rpi5_scanout/fw_clk.h"
#include "../rpi5_scanout/fw_info.h"
#include "../rpi5_scanout/fw_stop.h"
#include "../rpi5_scanout/hdmi_ctrl.h"
#include "../rpi5_scanout/hdmi_hpd.h"
#include "../rpi5_scanout/hdmi_phy.h"
#include "../rpi5_scanout/hdmi_rates.h"
#include "../rpi5_scanout/hvs6_chan.h"
#include "../rpi5_scanout/hvs6_dlist.h"
#include "../rpi5_scanout/hvs6_dlist_write.h"
#include "../rpi5_scanout/hvs6_fmt.h"
#include "../rpi5_scanout/infoframe.h"
#include "../rpi5_scanout/pan.h"
#include "../rpi5_scanout/pattern.h"
#include "../rpi5_scanout/pv_hdmi.h"
#include "../rpi5_scanout/pv_irq.h"
#include "../rpi5_scanout/pv_timing.h"
#include "../rpi5_scanout/scanout_alive.h"
#include "../rpi5_scanout/snapshot.h"
#include "../rpi5_scanout/status_text.h"
#include "../rpi5_scanout/throttled.h"

/* Second pass: the guards above must make every header inert again. */
#include "../rpi5_scanout/adopt.h"
#include "../rpi5_scanout/bg_fill.h"
#include "../rpi5_scanout/bus_addr.h"
#include "../rpi5_scanout/dlist_alloc.h"
#include "../rpi5_scanout/edid.h"
#include "../rpi5_scanout/fb_flush.h"
#include "../rpi5_scanout/fb_query.h"
#include "../rpi5_scanout/fw_clk.h"
#include "../rpi5_scanout/fw_info.h"
#include "../rpi5_scanout/fw_stop.h"
#include "../rpi5_scanout/hdmi_ctrl.h"
#include "../rpi5_scanout/hdmi_hpd.h"
#include "../rpi5_scanout/hdmi_phy.h"
#include "../rpi5_scanout/hdmi_rates.h"
#include "../rpi5_scanout/hvs6_chan.h"
#include "../rpi5_scanout/hvs6_dlist.h"
#include "../rpi5_scanout/hvs6_dlist_write.h"
#include "../rpi5_scanout/hvs6_fmt.h"
#include "../rpi5_scanout/infoframe.h"
#include "../rpi5_scanout/pan.h"
#include "../rpi5_scanout/pattern.h"
#include "../rpi5_scanout/pv_hdmi.h"
#include "../rpi5_scanout/pv_irq.h"
#include "../rpi5_scanout/pv_timing.h"
#include "../rpi5_scanout/scanout_alive.h"
#include "../rpi5_scanout/snapshot.h"
#include "../rpi5_scanout/status_text.h"
#include "../rpi5_scanout/throttled.h"

static volatile intptr_t touched;

int main(void)
{
	touched = (intptr_t)&rpi5_adopt_check; /* adopt.h */
	touched = (intptr_t)&yd5_bgf_regs; /* bg_fill.h */
	touched = (intptr_t)&bcm2712_bus_to_arm; /* bus_addr.h */
	touched = (intptr_t)&dlist_place; /* dlist_alloc.h */
	touched = (intptr_t)&edid_build_request; /* edid.h */
	touched = (intptr_t)&fb_flush_range; /* fb_flush.h */
	touched = (intptr_t)&fbq_build_query; /* fb_query.h */
	touched = (intptr_t)&yd5_fwclk_build_query; /* fw_clk.h */
	touched = (intptr_t)&fw_info_request; /* fw_info.h */
	touched = (intptr_t)&fw_stop_plan; /* fw_stop.h */
	touched = (intptr_t)&yd5_hdmi_log_init; /* hdmi_ctrl.h */
	touched = (intptr_t)&hdmi_hpd_detect; /* hdmi_hpd.h */
	touched = (intptr_t)&yd5_hdmi_phy_capture; /* hdmi_phy.h */
	touched = (intptr_t)&hdmi_rates_core_min; /* hdmi_rates.h */
	touched = (intptr_t)&yd5_hvs6_log_init; /* hvs6_chan.h */
	touched = (intptr_t)&hvs6_gen_from_version; /* hvs6_dlist.h */
	touched = (intptr_t)&hvs6w_bpp; /* hvs6_dlist_write.h */
	touched = (intptr_t)&hvs6_fmt_lookup; /* hvs6_fmt.h */
	touched = (intptr_t)&avi_infoframe_build; /* infoframe.h */
	touched = (intptr_t)&pan_pointer; /* pan.h */
	touched = (intptr_t)&pattern_hash; /* pattern.h */
	touched = (intptr_t)&pv_running; /* pv_hdmi.h */
	touched = (intptr_t)&pv_irq_decode; /* pv_irq.h */
	touched = (intptr_t)&pvt_find_vic; /* pv_timing.h */
	touched = (intptr_t)&sa_is_d0; /* scanout_alive.h */
	touched = (intptr_t)&snap_block_name; /* snapshot.h */
	touched = (intptr_t)&status_text_geometry; /* status_text.h */
	touched = (intptr_t)&thr_request; /* throttled.h */
	return 0;
}
