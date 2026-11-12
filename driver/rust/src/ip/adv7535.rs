// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ADV7535 HDMI transmitter, configured over I2C with a DSI input; follows
//! Linux drm/bridge/adv7511/adv7511_drv.c and adv7533.c.
//!
//! Source: mainline Linux `drivers/gpu/drm/bridge/adv7511/{adv7511_drv.c,
//! adv7533.c,adv7511.h}` (master, fetched 2026-11-12; the ADV7535 is the
//! `adv7535_chip_info` entry: `has_dsi`, `hpd_override_enable`,
//! `max_lane_freq_khz = 891000`). The raw files could not be downloaded on
//! the build host, only read through a summarising fetch that returns no line
//! numbers, so citations are by function and identifier, not file:line:
//!
//! - `[V]` read verbatim from the fetched text (adv7533.c in full,
//!   adv7511.h register values);
//! - `[R]` taken from the summary of adv7511_drv.c or recalled; to be
//!   re-checked against a local copy (and given line numbers) by whoever has
//!   one. The golden register dump of the DK's Linux (item linux-golden) is
//!   the arbiter for every `[R]`.
//!
//! The part has four I2C targets, all reached through the shim's [`Env`] by
//! bus index (the shim maps each to the address on the board's I2C
//! controller): the main map ([`BUS_MAIN`], 0x3d on the DK), the DSI/CEC map
//! ([`BUS_DSI_CEC`], 0x3c: on the ADV7533/5 the DSI receiver lives there),
//! the EDID map ([`BUS_EDID`], 0x3f) and the packet map ([`BUS_PACKET`],
//! 0x38). The three secondary addresses are programmed through the main
//! map's 0xE1 / 0x43 / 0x45 ([`Bridge::init`]).
//!
//! Not here: EDID parsing (`crate::video`), CEC, audio, HDCP, the I2C
//! controller. Polling only, no interrupt line.

use core::cell::Cell;

use crate::env::Env;
use crate::errno::{EINVAL, EIO, ENODEV, ETIMEDOUT};
use crate::video::VideoMode;
use crate::ylog;

/// Bus indexes of the part's four I2C targets in the shim's [`Env`].
pub const BUS_MAIN: u32 = 0;
pub const BUS_DSI_CEC: u32 = 1;
pub const BUS_EDID: u32 = 2;
pub const BUS_PACKET: u32 = 3;

/// Main map registers and bits (adv7511.h `[V]`).
pub mod reg {
    /// ADV7511_REG_CHIP_REVISION.
    pub const CHIP_REVISION: u8 = 0x00;
    /// ADV7511_REG_POWER, ADV7511_POWER_POWER_DOWN.
    pub const POWER: u8 = 0x41;
    pub const POWER_DOWN: u8 = 1 << 6;
    /// ADV7511_REG_STATUS, ADV7511_STATUS_HPD.
    pub const STATUS: u8 = 0x42;
    pub const STATUS_HPD: u8 = 1 << 6;
    /// ADV7511_REG_INT(0), ADV7511_REG_INT(1); write 1 to clear.
    pub const INT0: u8 = 0x96;
    pub const INT0_HPD: u8 = 1 << 7;
    pub const INT0_EDID_READY: u8 = 1 << 2;
    pub const INT1: u8 = 0x97;
    pub const INT1_DDC_ERROR: u8 = 1 << 7;
    /// ADV7511_REG_POWER2 and ADV7535_REG_POWER2_HPD_OVERRIDE.
    pub const POWER2: u8 = 0xd6;
    pub const POWER2_HPD_OVERRIDE: u8 = 1 << 6;
    /// ADV7511_REG_EDID_I2C_ADDR, _PACKET_I2C_ADDR, _CEC_I2C_ADDR: the
    /// target's 7-bit address << 1.
    pub const EDID_I2C_ADDR: u8 = 0x43;
    pub const PACKET_I2C_ADDR: u8 = 0x45;
    pub const CEC_I2C_ADDR: u8 = 0xe1;
    /// ADV7511_REG_CEC_CTRL, ADV7511_CEC_CTRL_POWER_DOWN.
    pub const CEC_CTRL: u8 = 0xe2;
    pub const CEC_CTRL_POWER_DOWN: u8 = 1 << 0;
    /// ADV7511_REG_EDID_SEGMENT: writing the 256-byte segment starts the
    /// DDC read into the EDID map.
    pub const EDID_SEGMENT: u8 = 0xc4;
    /// ADV7511_REG_HDCP_HDMI_CFG, ADV7511_HDMI_CFG_MODE_MASK / _HDMI / _DVI.
    pub const HDMI_CFG: u8 = 0xaf;
    pub const HDMI_CFG_MODE_MASK: u8 = 0x02;
    pub const HDMI_CFG_MODE_HDMI: u8 = 0x02;
    pub const HDMI_CFG_MODE_DVI: u8 = 0x00;
    /// ADV7511_REG_PACKET_ENABLE0 / 1; ADV7511_PACKET_ENABLE_AVI_INFOFRAME
    /// is BIT(12) of the pair, so bit 4 of ENABLE1.
    pub const PACKET_ENABLE1: u8 = 0x44;
    pub const PACKET_ENABLE1_AVI: u8 = 1 << 4;
    /// ADV7511_REG_AVI_INFOFRAME_VERSION, _LENGTH; the checksum follows at
    /// 0x54 and ADV7511_REG_AVI_INFOFRAME(x) = 0x55 + x holds PB1...
    pub const AVI_VERSION: u8 = 0x52;
    /// 0x30 + x, ADV7511_REG_SYNC_DECODER is not the polarity register;
    /// `[R]` adv7511_mode_set writes 0x17 bits 6:5 (VS, HS negative).
    pub const SYNC_POLARITY: u8 = 0x17;
    pub const SYNC_POLARITY_VS_NEG: u8 = 1 << 6;
    pub const SYNC_POLARITY_HS_NEG: u8 = 1 << 5;
    pub const SYNC_POLARITY_MASK: u8 = 0x60;
}

/// adv7511_fixed_registers `[R]` (the first nine are what the summary lists:
/// 0x98 0x9a 0x9c 0x9d 0xa2 0xa3 0xe0 0xf9 0x55), applied by the probe's
/// regmap_register_patch for every variant.
pub const FIXED_REGISTERS: [(u8, u8); 9] = [
    (0x98, 0x03),
    (0x9a, 0xe0),
    (0x9c, 0x30),
    (0x9d, 0x61),
    (0xa2, 0xa4),
    (0xa3, 0xa4),
    (0xe0, 0xd0),
    (0xf9, 0x00),
    (0x55, 0x02),
];

/// adv7533_fixed_registers `[V]` (adv7533.c:10-17), main map, applied by
/// adv7533_patch_registers when the chip has a DSI input.
pub const DSI_FIXED_REGISTERS: [(u8, u8); 6] =
    [(0x16, 0x20), (0x9a, 0xe0), (0xba, 0x70), (0xde, 0x82), (0xe4, 0x40), (0xe5, 0x80)];

/// adv7533_cec_fixed_registers `[V]` (adv7533.c:19-25), DSI/CEC map, applied
/// by adv7533_patch_cec_registers (probe) and again by adv7533_dsi_power_on
/// (adv7533.c:88-89).
pub const DSI_CEC_FIXED_REGISTERS: [(u8, u8); 5] =
    [(0x15, 0xd0), (0x17, 0xd0), (0x24, 0x20), (0x57, 0x11), (0x05, 0xc8)];

/// adv7533_dsi_config_timing_gen's clock_div_by_lanes `[V]` (adv7533.c:32),
/// indexed by lanes - 2.
const CLOCK_DIV_BY_LANES: [u8; 3] = [6, 4, 3];

/// `adv7535_chip_info.max_lane_freq_khz` `[R]` (adv7533_mode_valid,
/// adv7533.c:100-111 `[V]`: clock_khz * bpp <= max * lanes).
pub const MAX_LANE_FREQ_KHZ: u32 = 891_000;
/// MIPI_DSI_FMT_RGB888 (adv7533_attach_dsi, adv7533.c:151 `[V]`).
pub const DSI_BPP: u32 = 24;

/// Default 7-bit secondary addresses (ADV7511_EDID_I2C_ADDR_DEFAULT etc.
/// `[R]`).
pub const EDID_ADDR_DEFAULT: u8 = 0x3f;
pub const PACKET_ADDR_DEFAULT: u8 = 0x38;
pub const CEC_ADDR_DEFAULT: u8 = 0x3c;

/// Poll of INT0.EDID_READY: ms between reads and how many (ours; Linux waits
/// up to 200 ms for the interrupt, `adv7511_wait_for_edid(adv7511, 200)`
/// `[R]`).
pub const EDID_POLL_MS: u32 = 5;
pub const EDID_POLLS: u32 = 40;
/// One EDID block is 128 bytes, read as 64-byte chunks
/// (adv7511_get_edid_block `[R]`: four-chunk loop of 64 bytes).
pub const EDID_BLOCK: usize = 128;
const EDID_CHUNK: usize = 64;

/// What the board's devicetree says (adi,dsi-lanes, adi,disable-timing-
/// generator, the three secondary addresses).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// adi,dsi-lanes, 2..=4 (adv7533_parse_dt, adv7533.c:166-169 `[V]`).
    pub lanes: u8,
    /// !adi,disable-timing-generator (adv7533.c:177 `[V]`).
    pub use_timing_gen: bool,
    pub edid_addr: u8,
    pub packet_addr: u8,
    pub cec_addr: u8,
}

impl Config {
    pub const fn new(lanes: u8) -> Self {
        Config {
            lanes,
            use_timing_gen: true,
            edid_addr: EDID_ADDR_DEFAULT,
            packet_addr: PACKET_ADDR_DEFAULT,
            cec_addr: CEC_ADDR_DEFAULT,
        }
    }
}

/// What the sink takes: HDMI or DVI, and the AVI infoframe's content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sink {
    /// HDMI mode (infoframe sent) or DVI (no infoframe).
    pub hdmi: bool,
    /// The CEA-861 video identification code (4 = 1280x720@60), 0 for none.
    pub vic: u8,
    /// Picture aspect ratio 16:9 (else 4:3).
    pub aspect_16_9: bool,
}

/// What [`Bridge::init`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Found {
    pub revision: u8,
}

/// The AVI infoframe registers 0x52.. as written: version, length,
/// checksum, PB1..PB13 (hdmi_infoframe_pack's bytes 1.. for an RGB frame
/// with "active format valid" and "same as picture", which is what
/// drm_hdmi_avi_infoframe_from_display_mode gives for an RGB CEA mode `[R]`).
pub fn avi_infoframe(sink: &Sink) -> [u8; 16] {
    let mut f = [0u8; 16];
    f[0] = 2; // version
    f[1] = 13; // length
    f[3] = 1 << 4; // PB1: RGB, A0 = 1 (active format valid), no bars, no scan info
    f[4] = (if sink.aspect_16_9 { 2 } else { 1 } << 4) | 0x8; // PB2: M1M0, R3..R0 = 8 (same as picture)
    f[6] = sink.vic; // PB4
                     // checksum: header type 0x82 + version + length + payload sums to 0.
    let sum =
        0x82u8.wrapping_add(f[0]).wrapping_add(f[1]).wrapping_add(f[3..].iter().fold(0u8, |a, &b| a.wrapping_add(b)));
    f[2] = 0u8.wrapping_sub(sum);
    f
}

/// adv7533_dsi_config_timing_gen's register image `[V]` (adv7533.c:41-63) in
/// the DSI/CEC map, from a mode in porch terms: (reg, value).
pub fn timing_gen(mode: &VideoMode, lanes: u8) -> [(u8, u8); 17] {
    let (hsw, hfp, hbp) = (mode.hsync, mode.hfront, mode.hback);
    let (vsw, vfp, vbp) = (mode.vsync, mode.vfront, mode.vback);
    let (htotal, vtotal) = (mode.htotal(), mode.vtotal());
    let pair = |reg: u8, v: u32| [(reg, (v >> 4) as u8), (reg + 1, ((v << 4) & 0xff) as u8)];
    let mut out = [(0u8, 0u8); 17];
    out[0] = (0x16, CLOCK_DIV_BY_LANES[usize::from(lanes) - 2] << 3);
    for (i, (reg, v)) in
        [(0x28, htotal), (0x2a, hsw), (0x2c, hfp), (0x2e, hbp), (0x30, vtotal), (0x32, vsw), (0x34, vfp), (0x36, vbp)]
            .into_iter()
            .enumerate()
    {
        let p = pair(reg, v);
        out[1 + 2 * i] = p[0];
        out[2 + 2 * i] = p[1];
    }
    out
}

/// adv7533_mode_valid `[V]`: the DSI lanes can carry the pixel clock.
pub fn mode_valid(mode: &VideoMode, lanes: u8) -> bool {
    u64::from(mode.pixel_clock_hz / 1000) * u64::from(DSI_BPP) <= u64::from(MAX_LANE_FREQ_KHZ) * u64::from(lanes)
}

/// The part on four buses of an [`Env`].
pub struct Bridge<'a, E: Env> {
    env: &'a E,
    /// adv7511's current_edid_segment: -1 until a segment was fetched.
    segment: Cell<i32>,
}

impl<'a, E: Env> Bridge<'a, E> {
    pub fn new(env: &'a E) -> Self {
        Bridge { env, segment: Cell::new(-1) }
    }

    fn read(&self, bus: u32, reg: u8) -> Result<u8, i32> {
        let mut b = [0u8; 1];
        self.env.i2c_write_read(bus, &[reg], &mut b)?;
        Ok(b[0])
    }

    fn write(&self, bus: u32, reg: u8, val: u8) -> Result<(), i32> {
        self.env.i2c_write(bus, &[reg, val])
    }

    /// regmap_update_bits: read-modify-write of the bits this driver owns.
    fn update(&self, bus: u32, reg: u8, mask: u8, val: u8) -> Result<(), i32> {
        let old = self.read(bus, reg)?;
        self.write(bus, reg, (old & !mask) | (val & mask))
    }

    fn patch(&self, bus: u32, regs: &[(u8, u8)]) -> Result<(), i32> {
        regs.iter().try_for_each(|&(r, v)| self.write(bus, r, v))
    }

    /// The probe (adv7511_probe `[R]`): read CHIP_REVISION, apply the fixed
    /// registers (adv7511_fixed_registers, then adv7533_patch_registers),
    /// program the packet, CEC and EDID addresses (0x45, 0xE1, 0x43 get the
    /// 7-bit address << 1), power the CEC block down (0xE2 bit 0), then patch
    /// the DSI/CEC map (adv7533_patch_cec_registers, adv7533.c:120-125).
    /// Leaves the part powered down (POWER_DOWN is its reset state).
    pub fn init(&self, cfg: &Config) -> Result<Found, i32> {
        if !(2..=4).contains(&cfg.lanes) {
            return Err(-EINVAL);
        }
        let revision = self.read(BUS_MAIN, reg::CHIP_REVISION).map_err(|e| {
            ylog!(self.env, Err, "adv7535: no answer on the main map ({})", e);
            -EIO
        })?;
        self.segment.set(-1);
        self.patch(BUS_MAIN, &FIXED_REGISTERS)?;
        self.patch(BUS_MAIN, &DSI_FIXED_REGISTERS)?;
        self.write(BUS_MAIN, reg::PACKET_I2C_ADDR, cfg.packet_addr << 1)?;
        self.write(BUS_MAIN, reg::CEC_I2C_ADDR, cfg.cec_addr << 1)?;
        self.write(BUS_MAIN, reg::EDID_I2C_ADDR, cfg.edid_addr << 1)?;
        self.write(BUS_MAIN, reg::CEC_CTRL, reg::CEC_CTRL_POWER_DOWN)?;
        self.patch(BUS_DSI_CEC, &DSI_CEC_FIXED_REGISTERS)?;
        ylog!(self.env, Inf, "adv7535: chip revision {:#04x}", revision);
        Ok(Found { revision })
    }

    /// STATUS (0x42) bit 6: a sink is plugged. (Linux reads the same bit in
    /// adv7511_detect; with `hpd_override_enable` the override bit is set in
    /// power-up, [`Bridge::power_up`].)
    pub fn hpd(&self) -> Result<bool, i32> {
        Ok(self.read(BUS_MAIN, reg::STATUS)? & reg::STATUS_HPD != 0)
    }

    /// One 128-byte EDID block, raw (adv7511_get_edid_block `[R]`). The
    /// 256-byte segment is fetched by writing its number to 0xC4 after
    /// clearing EDID_READY / DDC_ERROR and clearing POWER_DOWN, then
    /// waiting for EDID_READY; the block is read from the EDID map as four
    /// 64-byte chunks at offset `(block % 2) * 128 + i * 64`. The segment is
    /// fetched only when it differs from the last one. HPD low is `-ENODEV`
    /// before anything touches the EDID map.
    pub fn read_edid_block(&self, block: u32, out: &mut [u8; EDID_BLOCK]) -> Result<(), i32> {
        if !self.hpd()? {
            ylog!(self.env, Err, "adv7535: no sink (HPD low), EDID not read");
            return Err(-ENODEV);
        }
        let seg = (block / 2) as i32;
        if self.segment.get() != seg {
            self.write(BUS_MAIN, reg::INT0, reg::INT0_EDID_READY)?;
            self.write(BUS_MAIN, reg::INT1, reg::INT1_DDC_ERROR)?;
            self.update(BUS_MAIN, reg::POWER, reg::POWER_DOWN, 0)?;
            self.write(BUS_MAIN, reg::EDID_SEGMENT, seg as u8)?;
            let mut polls = 0;
            loop {
                if self.read(BUS_MAIN, reg::INT1)? & reg::INT1_DDC_ERROR != 0 {
                    ylog!(self.env, Err, "adv7535: DDC error reading EDID segment {}", seg);
                    return Err(-EIO);
                }
                if self.read(BUS_MAIN, reg::INT0)? & reg::INT0_EDID_READY != 0 {
                    break;
                }
                polls += 1;
                if polls >= EDID_POLLS {
                    ylog!(self.env, Err, "adv7535: EDID segment {} not ready", seg);
                    return Err(-ETIMEDOUT);
                }
                self.env.sleep_ms(EDID_POLL_MS);
            }
            self.write(BUS_MAIN, reg::INT0, reg::INT0_EDID_READY)?;
            self.segment.set(seg);
        }
        for (i, chunk) in out.chunks_mut(EDID_CHUNK).enumerate() {
            let off = (block as usize % 2) * EDID_BLOCK + i * EDID_CHUNK;
            self.env.i2c_write_read(BUS_EDID, &[off as u8], chunk)?;
        }
        Ok(())
    }

    /// __adv7511_power_on `[R]`: POWER_DOWN cleared; on the ADV7535
    /// (`hpd_override_enable`) POWER2.HPD_OVERRIDE set instead of the HPD
    /// source mask.
    pub fn power_up(&self) -> Result<(), i32> {
        self.update(BUS_MAIN, reg::POWER, reg::POWER_DOWN, 0)?;
        self.update(BUS_MAIN, reg::POWER2, reg::POWER2_HPD_OVERRIDE, reg::POWER2_HPD_OVERRIDE)?;
        self.segment.set(-1);
        Ok(())
    }

    /// adv7511_mode_set `[R]` and adv7533_dsi_config_timing_gen `[V]`:
    /// the sync polarity (0x17), the HDMI or DVI mode (0xAF), the AVI
    /// infoframe (packet disabled, registers 0x52.., packet enabled; none in
    /// DVI) and the DSI timing generator in the DSI/CEC map.
    pub fn mode_set(&self, cfg: &Config, mode: &VideoMode, sink: &Sink) -> Result<(), i32> {
        if !(2..=4).contains(&cfg.lanes) || !mode_valid(mode, cfg.lanes) || mode.hactive == 0 {
            return Err(-EINVAL);
        }
        let mut pol = 0;
        if !mode.hsync_high {
            pol |= reg::SYNC_POLARITY_HS_NEG;
        }
        if !mode.vsync_high {
            pol |= reg::SYNC_POLARITY_VS_NEG;
        }
        self.update(BUS_MAIN, reg::SYNC_POLARITY, reg::SYNC_POLARITY_MASK, pol)?;
        // The packet engine is off while the frame changes.
        self.update(BUS_MAIN, reg::PACKET_ENABLE1, reg::PACKET_ENABLE1_AVI, 0)?;
        if sink.hdmi {
            self.update(BUS_MAIN, reg::HDMI_CFG, reg::HDMI_CFG_MODE_MASK, reg::HDMI_CFG_MODE_HDMI)?;
            for (i, b) in avi_infoframe(sink).into_iter().enumerate() {
                self.write(BUS_MAIN, reg::AVI_VERSION + i as u8, b)?;
            }
            self.update(BUS_MAIN, reg::PACKET_ENABLE1, reg::PACKET_ENABLE1_AVI, reg::PACKET_ENABLE1_AVI)?;
        } else {
            self.update(BUS_MAIN, reg::HDMI_CFG, reg::HDMI_CFG_MODE_MASK, reg::HDMI_CFG_MODE_DVI)?;
        }
        // adv7511_mode_set ends with adv7533_dsi_config_timing_gen, which
        // runs for any mode (the generator is then enabled or not by
        // dsi_enable).
        self.patch(BUS_DSI_CEC, &timing_gen(mode, cfg.lanes))?;
        Ok(())
    }

    /// adv7533_dsi_power_on `[V]` (adv7533.c:66-90): lanes << 4 (0x1C), the
    /// timing generator reset 0xCB, 0x8B, 0xCB or disabled with 0x0B (0x27),
    /// 0x03 <- 0x89 (enable hdmi), 0x55 <- 0 (test mode off), the CEC fixed
    /// registers again.
    pub fn dsi_enable(&self, cfg: &Config) -> Result<(), i32> {
        self.write(BUS_DSI_CEC, 0x1c, cfg.lanes << 4)?;
        if cfg.use_timing_gen {
            for v in [0xcb, 0x8b, 0xcb] {
                self.write(BUS_DSI_CEC, 0x27, v)?;
            }
        } else {
            self.write(BUS_DSI_CEC, 0x27, 0x0b)?;
        }
        self.write(BUS_DSI_CEC, 0x03, 0x89)?;
        self.write(BUS_DSI_CEC, 0x55, 0x00)?;
        self.patch(BUS_DSI_CEC, &DSI_CEC_FIXED_REGISTERS)
    }

    /// Power up, set the mode, enable the DSI receiver: the order of
    /// adv7511_bridge_atomic_enable with the mode already known.
    pub fn start(&self, cfg: &Config, mode: &VideoMode, sink: &Sink) -> Result<(), i32> {
        if !mode_valid(mode, cfg.lanes) {
            ylog!(self.env, Err, "adv7535: {} Hz is more than {} lanes carry", mode.pixel_clock_hz, cfg.lanes);
            return Err(-EINVAL);
        }
        self.power_up()?;
        self.mode_set(cfg, mode, sink)?;
        self.dsi_enable(cfg)?;
        ylog!(
            self.env,
            Inf,
            "adv7535: up, {}x{} at {} Hz, {} lanes",
            mode.hactive,
            mode.vactive,
            mode.pixel_clock_hz,
            cfg.lanes
        );
        Ok(())
    }

    /// adv7511_power_off `[R]` + adv7533_dsi_power_off `[V]` (adv7533.c:
    /// 92-98): 0x03 <- 0x0B, 0x27 <- 0x0B, then POWER_DOWN set and the HPD
    /// override cleared.
    pub fn power_down(&self) -> Result<(), i32> {
        self.write(BUS_DSI_CEC, 0x03, 0x0b)?;
        self.write(BUS_DSI_CEC, 0x27, 0x0b)?;
        self.update(BUS_MAIN, reg::POWER2, reg::POWER2_HPD_OVERRIDE, 0)?;
        self.update(BUS_MAIN, reg::POWER, reg::POWER_DOWN, reg::POWER_DOWN)?;
        self.segment.set(-1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::Level;
    use core::cell::RefCell;

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Ev {
        W(u32, u8, u8),
        R(u32, u8, usize),
        Sleep(u32),
    }

    /// Four register maps of 256 bytes, an EDID of two segments.
    struct Fake {
        regs: RefCell<[[u8; 256]; 4]>,
        ev: RefCell<Vec<Ev>>,
        hpd: bool,
        ddc_error: bool,
        never_ready: bool,
        edid: Vec<u8>,
    }

    impl Fake {
        fn new() -> Self {
            let mut regs = [[0u8; 256]; 4];
            regs[0][0] = 0x14;
            regs[0][0x41] = 0x50; // POWER_DOWN at reset
            let edid = (0..512usize).map(|i| (i % 251) as u8).collect();
            Fake {
                regs: RefCell::new(regs),
                ev: RefCell::new(vec![]),
                hpd: true,
                ddc_error: false,
                never_ready: false,
                edid,
            }
        }
        fn writes(&self) -> Vec<(u32, u8, u8)> {
            self.ev
                .borrow()
                .iter()
                .filter_map(|e| if let Ev::W(b, r, v) = e { Some((*b, *r, *v)) } else { None })
                .collect()
        }
        fn clear(&self) {
            self.ev.borrow_mut().clear();
        }
        fn reg(&self, bus: u32, r: u8) -> u8 {
            self.regs.borrow()[bus as usize][r as usize]
        }
    }

    impl Env for Fake {
        fn busy_wait_us(&self, _us: u32) {}
        fn sleep_ms(&self, ms: u32) {
            self.ev.borrow_mut().push(Ev::Sleep(ms));
        }
        fn log_str(&self, _l: Level, _line: &str) {}
        fn i2c_write(&self, bus: u32, buf: &[u8]) -> Result<(), i32> {
            assert_eq!(buf.len(), 2, "one register per transaction");
            self.ev.borrow_mut().push(Ev::W(bus, buf[0], buf[1]));
            let mut r = self.regs.borrow_mut();
            let m = &mut r[bus as usize];
            match (bus, buf[0]) {
                // write-1-to-clear interrupt status
                (BUS_MAIN, reg::INT0) | (BUS_MAIN, reg::INT1) => m[buf[0] as usize] &= !buf[1],
                (BUS_MAIN, reg::EDID_SEGMENT) => {
                    m[reg::EDID_SEGMENT as usize] = buf[1];
                    if self.ddc_error {
                        m[reg::INT1 as usize] |= reg::INT1_DDC_ERROR;
                    } else if !self.never_ready {
                        m[reg::INT0 as usize] |= reg::INT0_EDID_READY;
                    }
                }
                _ => m[buf[0] as usize] = buf[1],
            }
            Ok(())
        }
        fn i2c_write_read(&self, bus: u32, w: &[u8], rbuf: &mut [u8]) -> Result<(), i32> {
            assert_eq!(w.len(), 1);
            self.ev.borrow_mut().push(Ev::R(bus, w[0], rbuf.len()));
            let r = self.regs.borrow();
            if bus == BUS_EDID {
                let seg = r[BUS_MAIN as usize][reg::EDID_SEGMENT as usize] as usize;
                let base = seg * 256 + w[0] as usize;
                rbuf.copy_from_slice(&self.edid[base..base + rbuf.len()]);
            } else if bus == BUS_MAIN && w[0] == reg::STATUS {
                rbuf[0] = if self.hpd { reg::STATUS_HPD } else { 0 };
            } else {
                rbuf.copy_from_slice(&r[bus as usize][w[0] as usize..w[0] as usize + rbuf.len()]);
            }
            Ok(())
        }
        fn gpio_set(&self, _l: u32, _v: bool) -> Result<(), i32> {
            Ok(())
        }
    }

    /// CEA-861 VIC 4, 1280x720@60: 74.25 MHz, 1650x750, +hsync +vsync.
    fn mode_720p60() -> VideoMode {
        VideoMode {
            pixel_clock_hz: 74_250_000,
            hactive: 1280,
            hfront: 110,
            hsync: 40,
            hback: 220,
            vactive: 720,
            vfront: 5,
            vsync: 5,
            vback: 20,
            hsync_high: true,
            vsync_high: true,
            de_high: true,
        }
    }

    const SINK: Sink = Sink { hdmi: true, vic: 4, aspect_16_9: true };

    #[test]
    fn init_sequence() {
        let f = Fake::new();
        let found = Bridge::new(&f).init(&Config::new(4)).unwrap();
        assert_eq!(found.revision, 0x14);
        let mut want: Vec<(u32, u8, u8)> = vec![];
        want.extend(FIXED_REGISTERS.iter().map(|&(r, v)| (BUS_MAIN, r, v)));
        want.extend(DSI_FIXED_REGISTERS.iter().map(|&(r, v)| (BUS_MAIN, r, v)));
        want.extend([
            (BUS_MAIN, 0x45, 0x70), // packet 0x38 << 1
            (BUS_MAIN, 0xe1, 0x78), // cec 0x3c << 1
            (BUS_MAIN, 0x43, 0x7e), // edid 0x3f << 1
            (BUS_MAIN, 0xe2, 0x01),
        ]);
        want.extend(DSI_CEC_FIXED_REGISTERS.iter().map(|&(r, v)| (BUS_DSI_CEC, r, v)));
        assert_eq!(f.writes(), want);
        // adv7533.c:11-16 and :20-24
        assert_eq!(DSI_FIXED_REGISTERS[0], (0x16, 0x20));
        assert_eq!(DSI_CEC_FIXED_REGISTERS[4], (0x05, 0xc8));
        assert!(Bridge::new(&f).init(&Config::new(1)).is_err());
    }

    #[test]
    fn power_up_and_mode_set_720p60() {
        let f = Fake::new();
        let b = Bridge::new(&f);
        let cfg = Config::new(4);
        b.start(&cfg, &mode_720p60(), &SINK).unwrap();
        let w = f.writes();
        let at = |bus, r| w.iter().position(|e| e.0 == bus && e.1 == r);
        // power: POWER_DOWN cleared (0x50 -> 0x10), HPD override set
        assert_eq!(f.reg(BUS_MAIN, reg::POWER), 0x10);
        assert_eq!(f.reg(BUS_MAIN, reg::POWER2) & reg::POWER2_HPD_OVERRIDE, reg::POWER2_HPD_OVERRIDE);
        // HDMI mode, AVI enabled, sync polarity untouched (both positive)
        assert_eq!(f.reg(BUS_MAIN, reg::HDMI_CFG), 0x02);
        assert_eq!(f.reg(BUS_MAIN, reg::PACKET_ENABLE1), reg::PACKET_ENABLE1_AVI);
        assert_eq!(f.reg(BUS_MAIN, reg::SYNC_POLARITY), 0);
        // AVI: version 2, length 13, PB1 0x10, PB2 0x28 (16:9, same as
        // picture), PB4 = VIC 4; header 0x82+2+13 plus payload sums to 0
        let avi: Vec<u8> = (0..16).map(|i| f.reg(BUS_MAIN, 0x52 + i)).collect();
        assert_eq!(&avi[..2], &[2, 13]);
        assert_eq!(&avi[3..7], &[0x10, 0x28, 0x00, 4]);
        let sum = avi.iter().fold(0x82u8, |a, &b| a.wrapping_add(b));
        assert_eq!(sum, 0);
        // the packet is disabled while the registers change
        let dis = w.iter().position(|e| *e == (BUS_MAIN, reg::PACKET_ENABLE1, 0)).unwrap();
        assert!(dis < at(BUS_MAIN, 0x52).unwrap());
        // timing generator (adv7533.c:42-63): 4 lanes -> /3 << 3 = 0x18,
        // htotal 1650 = 0x672 -> 0x67 0x20, hsw 40 -> 0x02 0x80, hfp 110 ->
        // 0x06 0xe0, hbp 220 -> 0x0d 0xc0, vtotal 750 = 0x2ee -> 0x2e 0xe0,
        // vsw 5 -> 0x00 0x50, vfp 5, vbp 20 -> 0x01 0x40
        let cec = |r| f.reg(BUS_DSI_CEC, r);
        assert_eq!(cec(0x16), 0x18);
        let tg: Vec<u8> = (0x28..=0x37).map(cec).collect();
        assert_eq!(
            tg,
            [0x67, 0x20, 0x02, 0x80, 0x06, 0xe0, 0x0d, 0xc0, 0x2e, 0xe0, 0x00, 0x50, 0x00, 0x50, 0x01, 0x40]
        );
        // dsi power on (adv7533.c:70-89): lanes, generator reset, hdmi on
        let dsi: Vec<(u8, u8)> = w.iter().filter(|e| e.0 == BUS_DSI_CEC).map(|e| (e.1, e.2)).collect();
        let tail = &dsi[dsi.len() - (1 + 3 + 2 + DSI_CEC_FIXED_REGISTERS.len())..];
        assert_eq!(&tail[..6], &[(0x1c, 0x40), (0x27, 0xcb), (0x27, 0x8b), (0x27, 0xcb), (0x03, 0x89), (0x55, 0x00)]);
        assert_eq!(&tail[6..], &DSI_CEC_FIXED_REGISTERS);
        // order: power, mode, dsi
        assert!(at(BUS_MAIN, reg::POWER).unwrap() < at(BUS_MAIN, reg::HDMI_CFG).unwrap());
        assert!(at(BUS_MAIN, reg::HDMI_CFG).unwrap() < at(BUS_DSI_CEC, 0x1c).unwrap());
    }

    #[test]
    fn negative_sync_dvi_and_no_timing_generator() {
        let f = Fake::new();
        let b = Bridge::new(&f);
        let mut cfg = Config::new(2);
        cfg.use_timing_gen = false;
        let mode = VideoMode { pixel_clock_hz: 25_175_000, hsync_high: false, vsync_high: false, ..mode_720p60() };
        b.start(&cfg, &mode, &Sink { hdmi: false, vic: 0, aspect_16_9: false }).unwrap();
        assert_eq!(f.reg(BUS_MAIN, reg::SYNC_POLARITY), 0x60);
        assert_eq!(f.reg(BUS_MAIN, reg::HDMI_CFG) & 2, 0);
        assert_eq!(f.reg(BUS_MAIN, reg::PACKET_ENABLE1) & reg::PACKET_ENABLE1_AVI, 0);
        assert_eq!(f.reg(BUS_DSI_CEC, 0x16), 6 << 3);
        assert_eq!(f.reg(BUS_DSI_CEC, 0x1c), 0x20);
        assert_eq!(f.reg(BUS_DSI_CEC, 0x27), 0x0b);
    }

    #[test]
    fn too_fast_for_the_lanes() {
        let f = Fake::new();
        // 2 lanes carry 891000 * 2 / 24 = 74250 kHz
        assert!(mode_valid(&mode_720p60(), 2));
        let fast = VideoMode { pixel_clock_hz: 148_500_000, ..mode_720p60() };
        assert!(!mode_valid(&fast, 2));
        assert!(mode_valid(&fast, 4));
        assert_eq!(Bridge::new(&f).start(&Config::new(2), &fast, &SINK), Err(-EINVAL));
        assert!(f.writes().is_empty());
    }

    #[test]
    fn edid_blocks() {
        let f = Fake::new();
        let b = Bridge::new(&f);
        let mut blk = [0u8; 128];
        b.read_edid_block(0, &mut blk).unwrap();
        assert_eq!(&blk[..], &f.edid[..128]);
        assert_eq!(
            f.ev.borrow().clone(),
            vec![
                Ev::R(BUS_MAIN, reg::STATUS, 1),
                Ev::W(BUS_MAIN, reg::INT0, 0x04),
                Ev::W(BUS_MAIN, reg::INT1, 0x80),
                Ev::R(BUS_MAIN, reg::POWER, 1),
                Ev::W(BUS_MAIN, reg::POWER, 0x10),
                Ev::W(BUS_MAIN, reg::EDID_SEGMENT, 0),
                Ev::R(BUS_MAIN, reg::INT1, 1),
                Ev::R(BUS_MAIN, reg::INT0, 1),
                Ev::W(BUS_MAIN, reg::INT0, 0x04),
                Ev::R(BUS_EDID, 0, 64),
                Ev::R(BUS_EDID, 64, 64),
            ]
        );
        // block 1: same segment, no segment write, offset 128
        f.clear();
        b.read_edid_block(1, &mut blk).unwrap();
        assert_eq!(&blk[..], &f.edid[128..256]);
        assert_eq!(f.writes(), vec![]);
        assert_eq!(
            f.ev.borrow().iter().filter(|e| matches!(e, Ev::R(BUS_EDID, ..))).cloned().collect::<Vec<_>>(),
            vec![Ev::R(BUS_EDID, 128, 64), Ev::R(BUS_EDID, 192, 64)]
        );
        // block 2: segment 1, offset 0
        f.clear();
        b.read_edid_block(2, &mut blk).unwrap();
        assert_eq!(&blk[..], &f.edid[256..384]);
        assert!(f.writes().contains(&(BUS_MAIN, reg::EDID_SEGMENT, 1)));
    }

    #[test]
    fn hpd_low_is_a_clear_error() {
        let f = Fake { hpd: false, ..Fake::new() };
        let b = Bridge::new(&f);
        assert_eq!(b.hpd(), Ok(false));
        f.clear();
        let mut blk = [0u8; 128];
        assert_eq!(b.read_edid_block(0, &mut blk), Err(-ENODEV));
        assert_eq!(f.ev.borrow().clone(), vec![Ev::R(BUS_MAIN, reg::STATUS, 1)]);
        assert_eq!(Fake::new().ev.borrow().len(), 0);
    }

    #[test]
    fn edid_ddc_error_and_timeout() {
        let f = Fake { ddc_error: true, ..Fake::new() };
        let mut blk = [0u8; 128];
        assert_eq!(Bridge::new(&f).read_edid_block(0, &mut blk), Err(-EIO));
        let f = Fake { never_ready: true, ..Fake::new() };
        assert_eq!(Bridge::new(&f).read_edid_block(0, &mut blk), Err(-ETIMEDOUT));
        assert!(!f.ev.borrow().iter().any(|e| matches!(e, Ev::R(BUS_EDID, ..))));
    }

    #[test]
    fn power_down_undoes_power_up() {
        let f = Fake::new();
        let b = Bridge::new(&f);
        b.start(&Config::new(4), &mode_720p60(), &SINK).unwrap();
        f.clear();
        b.power_down().unwrap();
        let w = f.writes();
        assert_eq!(&w[..2], &[(BUS_DSI_CEC, 0x03, 0x0b), (BUS_DSI_CEC, 0x27, 0x0b)]);
        assert_eq!(f.reg(BUS_MAIN, reg::POWER) & reg::POWER_DOWN, reg::POWER_DOWN);
        assert_eq!(f.reg(BUS_MAIN, reg::POWER2) & reg::POWER2_HPD_OVERRIDE, 0);
        // a new EDID read fetches the segment again
        let mut blk = [0u8; 128];
        f.clear();
        b.read_edid_block(0, &mut blk).unwrap();
        assert!(f.writes().contains(&(BUS_MAIN, reg::EDID_SEGMENT, 0)));
    }

    #[test]
    fn no_answer_is_eio() {
        struct Dead;
        impl Env for Dead {
            fn busy_wait_us(&self, _: u32) {}
            fn sleep_ms(&self, _: u32) {}
            fn log_str(&self, _: Level, _: &str) {}
            fn i2c_write(&self, _: u32, _: &[u8]) -> Result<(), i32> {
                Err(-5)
            }
            fn i2c_write_read(&self, _: u32, _: &[u8], _: &mut [u8]) -> Result<(), i32> {
                Err(-5)
            }
            fn gpio_set(&self, _: u32, _: bool) -> Result<(), i32> {
                Ok(())
            }
        }
        assert_eq!(Bridge::new(&Dead).init(&Config::new(4)).err(), Some(-EIO));
    }
}
