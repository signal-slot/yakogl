/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * BCM2712 HDMI0 hotplug detect and DDC EDID block read, without the
 * firmware (route 3 of driver/docs/rpi5-display.md, for the case that the
 * mailbox GET_EDID_BLOCK tag is not available to a kernel= image).
 *
 * Header-only C11. All register access goes through the caller's read32 and
 * write32, so a host mock can model the registers. Addresses are plain
 * uint64_t: the caller passes the address of the block as it mapped it. No
 * EDID parsing here (edid.h does that); this only returns the 128 bytes of
 * segment 0.
 *
 * Facts (Linux, GPL: facts and file:line only; pins in
 * driver/docs/rpi5-v3d71-facts.md section 1, rpi: = rpi-6.12.y 5bd564aa):
 *  - Hotplug: `vc5_hdmi_hp_detect` reads HDMI_HOTPLUG and returns bit
 *    VC4_HDMI_HOTPLUG_CONNECTED; bcm2712_hdmi0_variant uses it as hp_detect
 *    (rpi:drivers/gpu/drm/vc4/vc4_hdmi.c, variant tables at the end of the
 *    file; line not fixed here). The bit is bit 0:
 *    rpi:drivers/gpu/drm/vc4/vc4_regs.h:743. HDMI_HOTPLUG is at 0x1c8 of the
 *    "hdmi" region for the BCM2712 (vc6_hdmi_hdmi0_fields):
 *    rpi:drivers/gpu/drm/vc4/vc4_hdmi_regs.h:424. There is no hpd-gpios: the
 *    hdmi0 node only has the "hpd-connected" and "hpd-removed" interrupts on
 *    aon_intr (rpi:arch/arm64/boot/dts/broadcom/bcm2712.dtsi:393-396); this
 *    header polls the level.
 *  - The "hdmi" region of hdmi0 is 0x7c701400, size 0x300 (bcm2712.dtsi:373).
 *  - DDC: hdmi0 has `ddc = <&ddc0>` (bcm2712.dtsi:397); ddc0 is
 *    `brcm,brcmstb-i2c` at 0x7d508200, size 0x58 (bcm2712.dtsi:334-336).
 *    The register layout is struct bsc_regs of
 *    rpi:drivers/i2c/busses/i2c-brcmstb.c:68-77, the bits :19-62, the
 *    transfer order brcmstb_i2c_xfer_bsc_data and brcmstb_send_i2c_cmd
 *    (:275-386). The compatible is not `brcm,brcmper-i2c`, so the data
 *    registers are 4 bytes wide: 8 registers, 32 bytes per burst (:182-184).
 *
 * The addresses above are the SoC bus view of the device tree. Whether the
 * ARM sees them at 0x10_0000_0000 + addr (as the HVS at 0x10_7c580000) is
 * for the bus-addr item; the caller passes the final address.
 *
 * Line numbers of i2c-brcmstb.c come from a read of the file at the pinned
 * commit; the board checks below settle what the sources do not.
 */
#ifndef RPI5_SCANOUT_HDMI_HPD_H
#define RPI5_SCANOUT_HDMI_HPD_H

#include <stddef.h>
#include <stdint.h>

/* Bus-view bases from the device tree. */
#define HDMI_HPD_HDMI0_BASE 0x7c701400ull /* "hdmi" region of hdmi0 */
#define HDMI_HPD_DDC0_BASE 0x7d508200ull  /* ddc0 (brcmstb BSC) */

/* HDMI region register and bit. */
#define HDMI_HPD_REG_HOTPLUG 0x1c8u
#define HDMI_HPD_HOTPLUG_CONNECTED (1u << 0)
/*
 * TODO(board check "hpd"): read HDMI_HOTPLUG at +0x1c8 with a monitor plugged
 * and unplugged, under Linux (devmem) and from the probe. The bit must be 1
 * only with a sink, and the block must still be powered for a kernel= image.
 */

/* BSC register offsets (struct bsc_regs, i2c-brcmstb.c:68-77). */
#define HDMI_HPD_BSC_CHIP_ADDRESS 0x00u
#define HDMI_HPD_BSC_DATA_IN(n) (0x04u + 4u * (unsigned)(n)) /* n 0..7 */
#define HDMI_HPD_BSC_CNT 0x24u
#define HDMI_HPD_BSC_CTL 0x28u
#define HDMI_HPD_BSC_IIC_ENABLE 0x2cu
#define HDMI_HPD_BSC_DATA_OUT(n) (0x30u + 4u * (unsigned)(n)) /* n 0..7 */
#define HDMI_HPD_BSC_CTLHI 0x50u

/* CTL_REG (i2c-brcmstb.c:27-33): DTF in bits 1:0. */
#define HDMI_HPD_CTL_DTF_WR 0x0u
#define HDMI_HPD_CTL_DTF_RD 0x1u
/* IIC_ENABLE (i2c-brcmstb.c:35-41). */
#define HDMI_HPD_EN_RESTART 0x40u
#define HDMI_HPD_EN_NOSTART 0x20u
#define HDMI_HPD_EN_NOSTOP 0x10u
#define HDMI_HPD_EN_NOACK 0x04u
#define HDMI_HPD_EN_INTRP 0x02u
#define HDMI_HPD_EN_ENABLE 0x01u
/* CTLHI_REG (i2c-brcmstb.c:43-47): 4-byte data registers. */
#define HDMI_HPD_CTLHI_DATAREG_SIZE 0x40u

#define HDMI_HPD_BURST 32u /* 8 data registers of 4 bytes */
#define HDMI_HPD_EDID_BLOCK 128u
#define HDMI_HPD_EDID_I2C_ADDR 0x50u /* DDC EDID, segment 0 */

/*
 * TODO(board check "ddc"): (1) INTRP (bit 1) rises when a burst is done and
 * clearing CNT_REG and IIC_ENABLE (i2c-brcmstb.c:312-313) is enough to
 * re-arm; (2) CHIP_ADDRESS takes the 7-bit address shifted left by one with
 * the read bit in bit 0; (3) the NOSTOP/NOSTART combination across the bursts
 * of one read; (4) the BSC clock divider left by the firmware is usable (this
 * header does not program it). Read the EDID of a plugged monitor from the
 * probe and compare it with Linux's the "edid" file of its HDMI-A-1 connector in sysfs.
 */

typedef struct hdmi_hpd_io {
	uint32_t (*read32)(void *ctx, uint64_t addr);
	void (*write32)(void *ctx, uint64_t addr, uint32_t val);
	void *ctx;
} hdmi_hpd_io;

enum hdmi_hpd_err {
	HDMI_HPD_OK = 0,
	HDMI_HPD_ERR_ARG = -1,     /* null pointer, no access function, no polls */
	HDMI_HPD_ERR_NAK = -2,     /* the sink did not acknowledge (NOACK) */
	HDMI_HPD_ERR_TIMEOUT = -3, /* INTRP did not rise within max_polls reads */
};

/* 1 if a sink is connected (HDMI_HOTPLUG bit 0), 0 if not or io is unusable. */
static inline int hdmi_hpd_detect(const hdmi_hpd_io *io, uint64_t hdmi_base)
{
	if (!io || !io->read32)
		return 0;
	return (io->read32(io->ctx, hdmi_base + HDMI_HPD_REG_HOTPLUG) &
		HDMI_HPD_HOTPLUG_CONNECTED) != 0;
}

/* One burst: program, start, poll for INTRP, check NOACK, clear. */
static inline int hdmi_hpd_xfer(const hdmi_hpd_io *io, uint64_t bsc,
				uint32_t addr_rw, uint32_t dtf, uint32_t len,
				uint32_t cond, uint32_t max_polls)
{
	uint32_t en = 0, i;

	io->write32(io->ctx, bsc + HDMI_HPD_BSC_CTLHI, HDMI_HPD_CTLHI_DATAREG_SIZE);
	io->write32(io->ctx, bsc + HDMI_HPD_BSC_CHIP_ADDRESS, addr_rw);
	io->write32(io->ctx, bsc + HDMI_HPD_BSC_CTL, dtf);
	io->write32(io->ctx, bsc + HDMI_HPD_BSC_CNT, len);
	io->write32(io->ctx, bsc + HDMI_HPD_BSC_IIC_ENABLE, cond | HDMI_HPD_EN_ENABLE);
	for (i = 0; i < max_polls; i++) {
		en = io->read32(io->ctx, bsc + HDMI_HPD_BSC_IIC_ENABLE);
		if (en & HDMI_HPD_EN_INTRP)
			break;
	}
	if (!(en & HDMI_HPD_EN_INTRP)) {
		io->write32(io->ctx, bsc + HDMI_HPD_BSC_IIC_ENABLE, 0);
		return HDMI_HPD_ERR_TIMEOUT;
	}
	io->write32(io->ctx, bsc + HDMI_HPD_BSC_CNT, 0);
	io->write32(io->ctx, bsc + HDMI_HPD_BSC_IIC_ENABLE, 0);
	return (en & HDMI_HPD_EN_NOACK) ? HDMI_HPD_ERR_NAK : HDMI_HPD_OK;
}

/*
 * Read EDID segment 0 (128 bytes) from the DDC controller at bsc_base: a one
 * byte write of offset 0 without stop, then four 32-byte read bursts, the
 * last one with a stop. max_polls bounds every IIC_ENABLE poll loop.
 */
static inline int hdmi_hpd_read_edid(const hdmi_hpd_io *io, uint64_t bsc_base,
				     uint32_t max_polls,
				     uint8_t out[HDMI_HPD_EDID_BLOCK])
{
	uint32_t off, j, v;
	int r;

	if (!io || !io->read32 || !io->write32 || !out || max_polls == 0)
		return HDMI_HPD_ERR_ARG;

	/* Write the EDID offset (0); keep the bus for the repeated start. */
	io->write32(io->ctx, bsc_base + HDMI_HPD_BSC_DATA_IN(0), 0);
	r = hdmi_hpd_xfer(io, bsc_base, HDMI_HPD_EDID_I2C_ADDR << 1,
			  HDMI_HPD_CTL_DTF_WR, 1, HDMI_HPD_EN_NOSTOP, max_polls);
	if (r != HDMI_HPD_OK)
		return r;

	for (off = 0; off < HDMI_HPD_EDID_BLOCK; off += HDMI_HPD_BURST) {
		uint32_t cond = 0;

		if (off != 0)
			cond |= HDMI_HPD_EN_NOSTART;
		if (off + HDMI_HPD_BURST < HDMI_HPD_EDID_BLOCK)
			cond |= HDMI_HPD_EN_NOSTOP;
		r = hdmi_hpd_xfer(io, bsc_base, (HDMI_HPD_EDID_I2C_ADDR << 1) | 1u,
				  HDMI_HPD_CTL_DTF_RD, HDMI_HPD_BURST, cond,
				  max_polls);
		if (r != HDMI_HPD_OK)
			return r;
		for (j = 0; j < HDMI_HPD_BURST / 4; j++) {
			v = io->read32(io->ctx, bsc_base + HDMI_HPD_BSC_DATA_OUT(j));
			out[off + 4 * j + 0] = (uint8_t)v;
			out[off + 4 * j + 1] = (uint8_t)(v >> 8);
			out[off + 4 * j + 2] = (uint8_t)(v >> 16);
			out[off + 4 * j + 3] = (uint8_t)(v >> 24);
		}
	}
	return HDMI_HPD_OK;
}

#endif /* RPI5_SCANOUT_HDMI_HPD_H */
