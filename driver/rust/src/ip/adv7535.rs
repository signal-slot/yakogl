// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ADV7535 HDMI transmitter, configured over I2C with a DSI input; follows
//! Linux drm/bridge/adv7511/adv7511_drv.c and adv7533.c.
//!
//! Source: mainline Linux master, fetched 2026-11-12 from
//! `drivers/gpu/drm/bridge/adv7511/{adv7511_drv.c,adv7533.c,adv7511.h}`.
//! `drv:N` is a line of adv7511_drv.c, `dsi:N` of adv7533.c, `hdr:N` of
//! adv7511.h in that fetch. The ADV7535 is `adv7535_chip_info` (drv:1444):
//! `has_dsi`, `hpd_override_enable`, `max_lane_freq_khz = 891000`,
//! `max_mode_clock_khz = 148500`, no `link_config`.
//!
//! Linux's order, which [`Bridge`] follows: probe (drv:1210 [`Bridge::init`]),
//! then per enable `adv7511_bridge_atomic_enable` (drv:779): power_on (drv:345,
//! which runs adv7533_dsi_power_on), set_config_csc, mode_set, infoframes.
//!
//! Deviations, all deliberate:
//! - no regcache: Linux replays its cached registers after power-up
//!   (`regcache_sync`, drv:351) because the chip loses them on power-down or
//!   a cable pull; here the caller powers up and configures again
//!   ([`Bridge::start`]) after either.
//! - polled: no interrupt line; `adv7511_wait_for_edid` (drv:494) has the
//!   same polling branch.
//! - a DDC error ends the EDID read with `-EIO` (Linux counts it as "read
//!   done" and returns what is in the buffer), and the segment register gets
//!   `block / 2` (drv:539 writes `block`, which equals it for blocks 0 and 1
//!   and names a segment that does not exist for block 2).
//! - the AVI infoframe bytes are built here ([`avi_infoframe`]), not by the
//!   DRM HDMI helper.
//!
//! The part has four I2C targets, reached through the shim's [`Env`] by bus
//! index (the shim maps each to the address on the board's I2C controller):
//! the main map ([`BUS_MAIN`], 0x3d on the DK), the DSI/CEC map
//! ([`BUS_DSI_CEC`], 0x3c: on the ADV7533/5 the DSI receiver lives there), the
//! EDID map ([`BUS_EDID`], 0x3f) and the packet map ([`BUS_PACKET`], 0x38;
//! never used here, Linux uses it for audio/SPD only). The three secondary
//! addresses are programmed through the main map's 0x43 / 0x45 / 0xE1.
//!
//! Not here: EDID parsing (`crate::video`), CEC, audio, HDCP, the I2C
//! controller.

use core::cell::{Cell, RefCell};

use crate::env::Env;
use crate::errno::{EINVAL, EIO, ENODEV, ETIMEDOUT};
use crate::video::VideoMode;
use crate::ylog;

/// Bus indexes of the part's four I2C targets in the shim's [`Env`].
pub const BUS_MAIN: u32 = 0;
pub const BUS_DSI_CEC: u32 = 1;
pub const BUS_EDID: u32 = 2;
pub const BUS_PACKET: u32 = 3;

/// Main map registers and bits (adv7511.h).
pub mod reg {
    /// ADV7511_REG_CHIP_REVISION.
    pub const CHIP_REVISION: u8 = 0x00;
    /// ADV7511_REG_VIDEO_INPUT_CFG1; set_config_csc owns bits 7 (4:2:2 out)
    /// and 0 (YCbCr out) (drv:250).
    pub const VIDEO_INPUT_CFG1: u8 = 0x16;
    /// ADV7511_REG_CSC_UPPER(0) / (1) = 0x18 / 0x1a; CSC_ENABLE = BIT(7) of
    /// the first, CSC_UPDATE_MODE = BIT(5) of the second (hdr:44,102,103).
    pub const CSC_UPPER0: u8 = 0x18;
    pub const CSC_UPPER1: u8 = 0x1a;
    pub const CSC_ENABLE: u8 = 1 << 7;
    pub const CSC_UPDATE_MODE: u8 = 1 << 5;
    /// SYNC polarity bits of drv:742: VS << 6, HS << 5 (1 = invert).
    pub const SYNC_POLARITY: u8 = 0x17;
    pub const SYNC_POLARITY_MASK: u8 = 0x60;
    /// ADV7511_REG_POWER, ADV7511_POWER_POWER_DOWN.
    pub const POWER: u8 = 0x41;
    pub const POWER_DOWN: u8 = 1 << 6;
    /// ADV7511_REG_PACKET_ENABLE0 / 1 (hdr:53,57); ADV7511_PACKET_ENABLE_AVI_
    /// INFOFRAME = BIT(8+4) is bit 4 of ENABLE1 (hdr:161).
    pub const PACKET_ENABLE0: u8 = 0x40;
    pub const PACKET_ENABLE1: u8 = 0x44;
    pub const PACKET_ENABLE1_AVI: u8 = 1 << 4;
    /// ADV7511_REG_STATUS, ADV7511_STATUS_HPD.
    pub const STATUS: u8 = 0x42;
    pub const STATUS_HPD: u8 = 1 << 6;
    /// ADV7511_REG_EDID_I2C_ADDR, _PACKET_I2C_ADDR, _CEC_I2C_ADDR: the
    /// target's 7-bit address << 1.
    pub const EDID_I2C_ADDR: u8 = 0x43;
    pub const PACKET_I2C_ADDR: u8 = 0x45;
    pub const CEC_I2C_ADDR: u8 = 0xe1;
    /// ADV7511_REG_INFOFRAME_UPDATE (drv:936: BIT(6) = send current values
    /// while the AVI registers change); bits 3:2 are also the low refresh
    /// rate of the non-ADV7511 parts (drv:741).
    pub const INFOFRAME_UPDATE: u8 = 0x4a;
    pub const INFOFRAME_UPDATE_AVI: u8 = 1 << 6;
    pub const LOW_REFRESH_MASK: u8 = 0x0c;
    /// ADV7511_REG_AVI_INFOFRAME_VERSION (hdr:63): version, length,
    /// checksum, then PB1 at 0x55.
    pub const AVI_VERSION: u8 = 0x52;
    /// ADV7511_REG_AUDIO_INFOFRAME_LENGTH and _AUDIO_INFOFRAME(1), written
    /// with 2 and 1 next to the AVI frame (drv:941-942).
    pub const AUDIO_INFOFRAME_LENGTH: u8 = 0x71;
    pub const AUDIO_INFOFRAME1: u8 = 0x74;
    /// ADV7511_REG_INT_ENABLE(1) = 0x95 (hdr:71), INT_ENABLE(0) = 0x94.
    pub const INT_ENABLE1: u8 = 0x95;
    /// ADV7511_REG_INT(0) / (1) = 0x96 / 0x97, write 1 to clear.
    pub const INT0: u8 = 0x96;
    pub const INT0_EDID_READY: u8 = 1 << 2;
    pub const INT1: u8 = 0x97;
    pub const INT1_DDC_ERROR: u8 = 1 << 7;
    /// ADV7511_REG_HDCP_HDMI_CFG with HDMI_CFG_MODE_MASK / _HDMI / _DVI.
    pub const HDMI_CFG: u8 = 0xaf;
    pub const HDMI_CFG_MODE_MASK: u8 = 0x02;
    pub const HDMI_CFG_MODE_HDMI: u8 = 0x02;
    pub const HDMI_CFG_MODE_DVI: u8 = 0x00;
    /// ADV7511_REG_EDID_SEGMENT.
    pub const EDID_SEGMENT: u8 = 0xc4;
    /// ADV7511_REG_DDC_STATUS (hdr:82): get_edid_block skips the segment
    /// fetch when it reads 2 (drv:537).
    pub const DDC_STATUS: u8 = 0xc8;
    /// ADV7511_REG_POWER2 and ADV7535_REG_POWER2_HPD_OVERRIDE (hdr:86,172).
    pub const POWER2: u8 = 0xd6;
    pub const POWER2_HPD_OVERRIDE: u8 = 1 << 6;
    /// ADV7511_REG_CEC_CTRL, ADV7511_CEC_CTRL_POWER_DOWN.
    pub const CEC_CTRL: u8 = 0xe2;
    pub const CEC_CTRL_POWER_DOWN: u8 = 1 << 0;
}

/// adv7533_fixed_registers (dsi:10-17), main map. The ADV7535 gets these and
/// not adv7511_fixed_registers (probe, drv:1280-1285: only `type == ADV7511`
/// takes those), applied by adv7533_patch_registers (dsi:113).
pub const DSI_FIXED_REGISTERS: [(u8, u8); 6] =
    [(0x16, 0x20), (0x9a, 0xe0), (0xba, 0x70), (0xde, 0x82), (0xe4, 0x40), (0xe5, 0x80)];

/// adv7533_cec_fixed_registers (dsi:19-25), DSI/CEC map, applied by
/// adv7533_patch_cec_registers (dsi:120; probe via drv:1115) and again by
/// adv7533_dsi_power_on (dsi:88).
pub const DSI_CEC_FIXED_REGISTERS: [(u8, u8); 5] =
    [(0x15, 0xd0), (0x17, 0xd0), (0x24, 0x20), (0x57, 0x11), (0x05, 0xc8)];

/// clock_div_by_lanes (dsi:32), indexed by lanes - 2.
const CLOCK_DIV_BY_LANES: [u8; 3] = [6, 4, 3];

/// adv7535_chip_info.max_lane_freq_khz (drv:1448); adv7533_mode_valid
/// (dsi:100-111): clock_khz * bpp <= max * lanes.
pub const MAX_LANE_FREQ_KHZ: u32 = 891_000;
/// adv7535_chip_info.max_mode_clock_khz (drv:1447).
pub const MAX_MODE_CLOCK_KHZ: u32 = 148_500;
/// MIPI_DSI_FMT_RGB888 (adv7533_attach_dsi, dsi:151).
pub const DSI_BPP: u32 = 24;

/// ADV7511_{EDID,PACKET,CEC}_I2C_ADDR_DEFAULT (the addresses probe passes to
/// i2c_new_ancillary_device, drv:1291,1303,1099).
pub const EDID_ADDR_DEFAULT: u8 = 0x3f;
pub const PACKET_ADDR_DEFAULT: u8 = 0x38;
pub const CEC_ADDR_DEFAULT: u8 = 0x3c;

/// adv7511_wait_for_edid's polling branch (drv:494-514): up to 200 ms in
/// steps of 25.
pub const EDID_POLL_MS: u32 = 25;
pub const EDID_TIMEOUT_MS: u32 = 200;
/// One EDID block is 128 bytes; a segment (two blocks) is read as four
/// 64-byte transfers (drv:546-571).
pub const EDID_BLOCK: usize = 128;
const EDID_CHUNK: usize = 64;
const SEGMENT: usize = 2 * EDID_BLOCK;

/// What the board's devicetree says (adi,dsi-lanes, adi,disable-timing-
/// generator, the three secondary addresses).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// adi,dsi-lanes, 2..=4 (adv7533_parse_dt, dsi:168).
    pub lanes: u8,
    /// !adi,disable-timing-generator (dsi:177).
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
    /// HDMI mode (infoframe sent) or DVI (no infoframe): the sink's EDID
    /// `is_hdmi` (set_config_csc, drv:243-247).
    pub hdmi: bool,
    /// The CEA-861 video identification code (4 = 1280x720@60), 0 for none.
    pub vic: u8,
    /// Picture aspect ratio 16:9 (else 4:3).
    pub aspect_16_9: bool,
    /// The frame rate in Hz, for the low refresh rate field (drv:727-735).
    pub refresh_hz: u32,
}

/// What [`Bridge::init`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Found {
    pub revision: u8,
}

/// The AVI infoframe as written from 0x52: version, length, checksum, PB1..
/// PB13 (hdmi_infoframe_pack's bytes 1.., drv:937-938). RGB, active format
/// valid and "same as picture", the 4:3 / 16:9 field, the VIC.
pub fn avi_infoframe(sink: &Sink) -> [u8; 16] {
    let mut f = [0u8; 16];
    f[0] = 2; // version
    f[1] = 13; // length
    f[3] = 1 << 4; // PB1: RGB, A0 = 1, no bars, no scan info
    f[4] = (if sink.aspect_16_9 { 2 } else { 1 } << 4) | 0x8; // PB2: M1M0, R3..R0 = 8
    f[6] = sink.vic; // PB4
                     // The type byte 0x82 is not written; it counts in the checksum.
    let sum = f.iter().fold(0x82u8, |a, &b| a.wrapping_add(b));
    f[2] = 0u8.wrapping_sub(sum);
    f
}

/// ADV7511_LOW_REFRESH_RATE_* of drv:727-735: NONE 0, 24 Hz 1, 25 Hz 2,
/// 30 Hz 3 (the ADV7511 field encoding, enum order in adv7511.h).
fn low_refresh_rate(hz: u32) -> u8 {
    match hz {
        0..=24 => 1,
        25 => 2,
        26..=30 => 3,
        _ => 0,
    }
}

/// adv7533_dsi_config_timing_gen's register image (dsi:27-63) in the DSI/CEC
/// map, from a mode in porch terms: (reg, value).
pub fn timing_gen(mode: &VideoMode, lanes: u8) -> [(u8, u8); 17] {
    let (hsw, hfp, hbp) = (mode.hsync, mode.hfront, mode.hback);
    let (vsw, vfp, vbp) = (mode.vsync, mode.vfront, mode.vback);
    let pair = |reg: u8, v: u32| [(reg, (v >> 4) as u8), (reg + 1, ((v << 4) & 0xff) as u8)];
    let mut out = [(0u8, 0u8); 17];
    out[0] = (0x16, CLOCK_DIV_BY_LANES[usize::from(lanes) - 2] << 3);
    for (i, (reg, v)) in [
        (0x28, mode.htotal()),
        (0x2a, hsw),
        (0x2c, hfp),
        (0x2e, hbp),
        (0x30, mode.vtotal()),
        (0x32, vsw),
        (0x34, vfp),
        (0x36, vbp),
    ]
    .into_iter()
    .enumerate()
    {
        let p = pair(reg, v);
        out[1 + 2 * i] = p[0];
        out[2 + 2 * i] = p[1];
    }
    out
}

/// adv7533_mode_valid (dsi:100-111) plus the part's pixel clock limit.
pub fn mode_valid(mode: &VideoMode, lanes: u8) -> bool {
    let khz = mode.pixel_clock_hz / 1000;
    khz <= MAX_MODE_CLOCK_KHZ && u64::from(khz) * u64::from(DSI_BPP) <= u64::from(MAX_LANE_FREQ_KHZ) * u64::from(lanes)
}

/// The part on four buses of an [`Env`].
pub struct Bridge<'a, E: Env> {
    env: &'a E,
    /// adv7511's current_edid_segment: -1 until a segment was fetched.
    segment: Cell<i32>,
    edid_buf: RefCell<[u8; SEGMENT]>,
    /// adv7511's `powered`.
    powered: Cell<bool>,
    /// The EDID target's 7-bit address, rewritten when the chip is powered
    /// for an EDID read (drv:598).
    edid_addr: Cell<u8>,
}

impl<'a, E: Env> Bridge<'a, E> {
    pub fn new(env: &'a E) -> Self {
        Bridge {
            env,
            segment: Cell::new(-1),
            edid_buf: RefCell::new([0; SEGMENT]),
            powered: Cell::new(false),
            edid_addr: Cell::new(EDID_ADDR_DEFAULT),
        }
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

    /// adv7511_probe (drv:1210) for the ADV7535, in Linux's order: read
    /// CHIP_REVISION (drv:1274); adv7533_patch_registers (drv:1285);
    /// adv7511_packet_disable(0xffff) (drv:1289); the EDID, packet and CEC
    /// addresses (drv:1297, 1314, 1103) and adv7533_patch_cec_registers
    /// (drv:1115); adv7511_power_off (drv:1324); CEC_CTRL power down
    /// (drv:1331). Leaves the part powered down.
    pub fn init(&self, cfg: &Config) -> Result<Found, i32> {
        if !(2..=4).contains(&cfg.lanes) {
            return Err(-EINVAL);
        }
        let revision = self.read(BUS_MAIN, reg::CHIP_REVISION).map_err(|e| {
            ylog!(self.env, Err, "adv7535: no answer on the main map ({})", e);
            -EIO
        })?;
        self.segment.set(-1);
        self.edid_addr.set(cfg.edid_addr);
        self.patch(BUS_MAIN, &DSI_FIXED_REGISTERS)?;
        self.update(BUS_MAIN, reg::PACKET_ENABLE0, 0xff, 0)?;
        self.update(BUS_MAIN, reg::PACKET_ENABLE1, 0xff, 0)?;
        self.write(BUS_MAIN, reg::EDID_I2C_ADDR, cfg.edid_addr << 1)?;
        self.write(BUS_MAIN, reg::PACKET_I2C_ADDR, cfg.packet_addr << 1)?;
        self.write(BUS_MAIN, reg::CEC_I2C_ADDR, cfg.cec_addr << 1)?;
        self.patch(BUS_DSI_CEC, &DSI_CEC_FIXED_REGISTERS)?;
        self.power_off()?;
        self.write(BUS_MAIN, reg::CEC_CTRL, reg::CEC_CTRL_POWER_DOWN)?;
        ylog!(self.env, Inf, "adv7535: chip revision {:#04x}", revision);
        Ok(Found { revision })
    }

    /// adv7511_detect (drv:614): STATUS bit 6, then HPD sensing is
    /// re-enabled with the override bit (drv:641-646; the reconnect branch
    /// for a powered part, which needs the interrupt status, is not taken).
    pub fn hpd(&self) -> Result<bool, i32> {
        let up = self.read(BUS_MAIN, reg::STATUS)? & reg::STATUS_HPD != 0;
        self.update(BUS_MAIN, reg::POWER2, reg::POWER2_HPD_OVERRIDE, reg::POWER2_HPD_OVERRIDE)?;
        Ok(up)
    }

    /// __adv7511_power_on (drv:306, polled: no interrupt enables): segment
    /// forgotten, POWER_DOWN cleared, HPD override set (drv:336).
    fn power_on_core(&self) -> Result<(), i32> {
        self.segment.set(-1);
        self.update(BUS_MAIN, reg::POWER, reg::POWER_DOWN, 0)?;
        self.update(BUS_MAIN, reg::POWER2, reg::POWER2_HPD_OVERRIDE, reg::POWER2_HPD_OVERRIDE)
    }

    /// adv7511_power_on (drv:345): the core, then adv7533_dsi_power_on
    /// (dsi:66-90): lanes << 4 (0x1C), the timing generator reset 0xCB, 0x8B,
    /// 0xCB or disabled with 0x0B (0x27), 0x03 <- 0x89 (enable hdmi),
    /// 0x55 <- 0 (test mode off), the CEC fixed registers again.
    pub fn power_up(&self, cfg: &Config) -> Result<(), i32> {
        self.power_on_core()?;
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
        self.patch(BUS_DSI_CEC, &DSI_CEC_FIXED_REGISTERS)?;
        self.powered.set(true);
        Ok(())
    }

    /// __adv7511_power_off (drv:359): override cleared, POWER_DOWN set,
    /// DDC_ERROR interrupt enable cleared; then adv7533_dsi_power_off
    /// (dsi:92-98): 0x03 <- 0x0B, 0x27 <- 0x0B (adv7511_power_off, drv:375).
    fn power_off(&self) -> Result<(), i32> {
        self.update(BUS_MAIN, reg::POWER2, reg::POWER2_HPD_OVERRIDE, 0)?;
        self.update(BUS_MAIN, reg::POWER, reg::POWER_DOWN, reg::POWER_DOWN)?;
        self.update(BUS_MAIN, reg::INT_ENABLE1, reg::INT1_DDC_ERROR, 0)?;
        self.write(BUS_DSI_CEC, 0x03, 0x0b)?;
        self.write(BUS_DSI_CEC, 0x27, 0x0b)?;
        self.powered.set(false);
        self.segment.set(-1);
        Ok(())
    }

    /// Power the part down (adv7511_bridge_atomic_disable's power_off).
    pub fn power_down(&self) -> Result<(), i32> {
        self.power_off()
    }

    /// One EDID block, raw. HPD low is `-ENODEV` before anything touches
    /// the EDID map. An unpowered part is powered for the read and the EDID
    /// address register rewritten (adv7511_edid_read, drv:588-601), then
    /// powered down again (drv:607). A new 256-byte segment is fetched as
    /// adv7511_get_edid_block does (drv:517-580): DDC_STATUS (0xC8) read, if
    /// not 2 the segment register written and the interrupt status polled
    /// until EDID_READY or DDC_ERROR (both are written back to clear), then
    /// four 64-byte reads at offsets 0, 64, 128, 192 of the EDID map; the
    /// block is the first or second half.
    pub fn read_edid_block(&self, block: u32, out: &mut [u8; EDID_BLOCK]) -> Result<(), i32> {
        if !self.hpd()? {
            ylog!(self.env, Err, "adv7535: no sink (HPD low), EDID not read");
            return Err(-ENODEV);
        }
        let was_powered = self.powered.get();
        if !was_powered {
            self.power_on_core()?;
            self.write(BUS_MAIN, reg::EDID_I2C_ADDR, self.edid_addr.get() << 1)?;
        }
        let r = self.fetch_block(block, out);
        if !was_powered {
            self.power_off()?;
        }
        r
    }

    fn fetch_block(&self, block: u32, out: &mut [u8; EDID_BLOCK]) -> Result<(), i32> {
        let seg = (block / 2) as i32;
        if self.segment.get() != seg {
            if self.read(BUS_MAIN, reg::DDC_STATUS)? != 2 {
                self.write(BUS_MAIN, reg::EDID_SEGMENT, seg as u8)?;
                let mut waited = 0;
                loop {
                    let (i0, i1) = (self.read(BUS_MAIN, reg::INT0)?, self.read(BUS_MAIN, reg::INT1)?);
                    self.write(BUS_MAIN, reg::INT0, i0)?;
                    self.write(BUS_MAIN, reg::INT1, i1)?;
                    if i1 & reg::INT1_DDC_ERROR != 0 {
                        ylog!(self.env, Err, "adv7535: DDC error reading EDID segment {}", seg);
                        return Err(-EIO);
                    }
                    if i0 & reg::INT0_EDID_READY != 0 {
                        break;
                    }
                    waited += EDID_POLL_MS;
                    if waited >= EDID_TIMEOUT_MS {
                        ylog!(self.env, Err, "adv7535: EDID segment {} not ready", seg);
                        return Err(-ETIMEDOUT);
                    }
                    self.env.sleep_ms(EDID_POLL_MS);
                }
            }
            let mut buf = self.edid_buf.borrow_mut();
            for (i, chunk) in buf.chunks_mut(EDID_CHUNK).enumerate() {
                self.env.i2c_write_read(BUS_EDID, &[(i * EDID_CHUNK) as u8], chunk)?;
            }
            self.segment.set(seg);
        }
        let base = (block as usize % 2) * EDID_BLOCK;
        out.copy_from_slice(&self.edid_buf.borrow()[base..base + EDID_BLOCK]);
        Ok(())
    }

    /// adv7511_set_config_csc for RGB (drv:215-258): the colour space
    /// converter off (adv7511_set_colormap, drv:147), the 4:2:2 / YCbCr
    /// output bits cleared, HDMI or DVI mode.
    fn set_config_csc(&self, sink: &Sink) -> Result<(), i32> {
        self.update(BUS_MAIN, reg::CSC_UPPER1, reg::CSC_UPDATE_MODE, reg::CSC_UPDATE_MODE)?;
        self.update(BUS_MAIN, reg::CSC_UPPER0, reg::CSC_ENABLE, 0)?;
        self.update(BUS_MAIN, reg::CSC_UPPER1, reg::CSC_UPDATE_MODE, 0)?;
        self.update(BUS_MAIN, reg::VIDEO_INPUT_CFG1, 0x81, 0)?;
        let mode = if sink.hdmi { reg::HDMI_CFG_MODE_HDMI } else { reg::HDMI_CFG_MODE_DVI };
        self.update(BUS_MAIN, reg::HDMI_CFG, reg::HDMI_CFG_MODE_MASK, mode)
    }

    /// adv7511_mode_set (drv:657) for a non-embedded-sync input: the low
    /// refresh rate (0x4A bits 3:2 for a non-ADV7511, drv:741), the sync
    /// polarity inversion (0x17 bits 6:5, drv:743; both 0: the input is
    /// configured pass-through, DSI regenerates the syncs), and
    /// adv7533_dsi_config_timing_gen when the generator is used (drv:752).
    fn mode_set(&self, cfg: &Config, mode: &VideoMode, sink: &Sink) -> Result<(), i32> {
        let lr = low_refresh_rate(sink.refresh_hz);
        self.update(BUS_MAIN, reg::INFOFRAME_UPDATE, reg::LOW_REFRESH_MASK, lr << 2)?;
        self.update(BUS_MAIN, reg::SYNC_POLARITY, reg::SYNC_POLARITY_MASK, 0)?;
        if cfg.use_timing_gen {
            self.patch(BUS_DSI_CEC, &timing_gen(mode, cfg.lanes))?;
        }
        Ok(())
    }

    /// adv7511_bridge_hdmi_write_avi_infoframe (drv:929) or, in DVI mode,
    /// the clear (packet disabled, drv:880).
    fn infoframes(&self, sink: &Sink) -> Result<(), i32> {
        if !sink.hdmi {
            return self.update(BUS_MAIN, reg::PACKET_ENABLE1, reg::PACKET_ENABLE1_AVI, 0);
        }
        self.update(BUS_MAIN, reg::INFOFRAME_UPDATE, reg::INFOFRAME_UPDATE_AVI, reg::INFOFRAME_UPDATE_AVI)?;
        for (i, b) in avi_infoframe(sink).into_iter().enumerate() {
            self.write(BUS_MAIN, reg::AVI_VERSION + i as u8, b)?;
        }
        self.write(BUS_MAIN, reg::AUDIO_INFOFRAME_LENGTH, 0x2)?;
        self.write(BUS_MAIN, reg::AUDIO_INFOFRAME1, 0x1)?;
        self.update(BUS_MAIN, reg::INFOFRAME_UPDATE, reg::INFOFRAME_UPDATE_AVI, 0)?;
        self.update(BUS_MAIN, reg::PACKET_ENABLE1, reg::PACKET_ENABLE1_AVI, reg::PACKET_ENABLE1_AVI)
    }

    /// adv7511_bridge_atomic_enable (drv:779): power up (with the DSI
    /// receiver), the colour/HDMI config, the mode, the infoframes. The DSI
    /// host must already stream video with the clock lane in HS.
    pub fn start(&self, cfg: &Config, mode: &VideoMode, sink: &Sink) -> Result<(), i32> {
        if !(2..=4).contains(&cfg.lanes) || mode.hactive == 0 || !mode_valid(mode, cfg.lanes) {
            ylog!(self.env, Err, "adv7535: {} Hz on {} lanes is outside the part", mode.pixel_clock_hz, cfg.lanes);
            return Err(-EINVAL);
        }
        self.power_up(cfg)?;
        self.set_config_csc(sink)?;
        self.mode_set(cfg, mode, sink)?;
        self.infoframes(sink)?;
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::Level;

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

    const SINK: Sink = Sink { hdmi: true, vic: 4, aspect_16_9: true, refresh_hz: 60 };

    #[test]
    fn init_sequence_is_the_probe() {
        let f = Fake::new();
        let found = Bridge::new(&f).init(&Config::new(4)).unwrap();
        assert_eq!(found.revision, 0x14);
        // the revision read first, then the ADV7533 patch (not the ADV7511
        // list), packet enables cleared, the three addresses << 1, the CEC
        // map patch, power off, CEC_CTRL power down
        let mut want: Vec<(u32, u8, u8)> = vec![];
        want.extend(DSI_FIXED_REGISTERS.iter().map(|&(r, v)| (BUS_MAIN, r, v)));
        want.extend([
            (BUS_MAIN, 0x40, 0),
            (BUS_MAIN, 0x44, 0),
            (BUS_MAIN, 0x43, 0x7e), // edid 0x3f << 1
            (BUS_MAIN, 0x45, 0x70), // packet 0x38 << 1
            (BUS_MAIN, 0xe1, 0x78), // cec 0x3c << 1
        ]);
        want.extend(DSI_CEC_FIXED_REGISTERS.iter().map(|&(r, v)| (BUS_DSI_CEC, r, v)));
        want.extend([
            (BUS_MAIN, 0xd6, 0), // HPD override off
            (BUS_MAIN, 0x41, 0x50 | 0x40),
            (BUS_MAIN, 0x95, 0),
            (BUS_DSI_CEC, 0x03, 0x0b),
            (BUS_DSI_CEC, 0x27, 0x0b),
            (BUS_MAIN, 0xe2, 0x01),
        ]);
        assert_eq!(f.writes(), want);
        assert_eq!(f.ev.borrow()[0], Ev::R(BUS_MAIN, 0x00, 1));
        // dsi:11-16 and :20-24
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
        let at = |bus, r| w.iter().position(|e| e.0 == bus && e.1 == r).unwrap();
        // POWER_DOWN cleared (0x50 -> 0x10), HPD override set
        assert_eq!(f.reg(BUS_MAIN, reg::POWER), 0x10);
        assert_eq!(f.reg(BUS_MAIN, reg::POWER2), reg::POWER2_HPD_OVERRIDE);
        // CSC off, HDMI mode, sync not inverted, low refresh NONE
        assert_eq!(f.reg(BUS_MAIN, reg::CSC_UPPER0) & reg::CSC_ENABLE, 0);
        assert_eq!(f.reg(BUS_MAIN, reg::CSC_UPPER1) & reg::CSC_UPDATE_MODE, 0);
        assert_eq!(f.reg(BUS_MAIN, reg::HDMI_CFG), 0x02);
        assert_eq!(f.reg(BUS_MAIN, reg::SYNC_POLARITY), 0);
        assert_eq!(f.reg(BUS_MAIN, reg::INFOFRAME_UPDATE) & 0xcc, 0);
        // AVI (drv:929): version 2, length 13, PB1 0x10, PB2 0x28, PB4 = VIC 4,
        // header 0x82 + the rest sums to 0; audio length/payload next to it;
        // the update bit released; the packet enabled
        let avi: Vec<u8> = (0..16).map(|i| f.reg(BUS_MAIN, 0x52 + i)).collect();
        assert_eq!(&avi[..2], &[2, 13]);
        assert_eq!(&avi[3..7], &[0x10, 0x28, 0x00, 4]);
        assert_eq!(avi.iter().fold(0x82u8, |a, &b| a.wrapping_add(b)), 0);
        assert_eq!((f.reg(BUS_MAIN, 0x71), f.reg(BUS_MAIN, 0x74)), (2, 1));
        assert_eq!(f.reg(BUS_MAIN, reg::PACKET_ENABLE1), reg::PACKET_ENABLE1_AVI);
        assert!(w.contains(&(BUS_MAIN, reg::INFOFRAME_UPDATE, 0x40)));
        assert_eq!(f.reg(BUS_MAIN, reg::INFOFRAME_UPDATE), 0);
        // timing generator (dsi:42-63): 4 lanes -> /3 << 3 = 0x18,
        // htotal 1650 = 0x672 -> 0x67 0x20, hsw 40 -> 0x02 0x80, hfp 110 ->
        // 0x06 0xe0, hbp 220 -> 0x0d 0xc0, vtotal 750 = 0x2ee -> 0x2e 0xe0,
        // vsw 5 -> 0x00 0x50, vfp 5, vbp 20 -> 0x01 0x40
        assert_eq!(f.reg(BUS_DSI_CEC, 0x16), 0x18);
        let tg: Vec<u8> = (0x28..=0x37).map(|r| f.reg(BUS_DSI_CEC, r)).collect();
        assert_eq!(
            tg,
            [0x67, 0x20, 0x02, 0x80, 0x06, 0xe0, 0x0d, 0xc0, 0x2e, 0xe0, 0x00, 0x50, 0x00, 0x50, 0x01, 0x40]
        );
        // dsi power on (dsi:70-89) is the exact CEC-map prefix of the sequence
        let dsi: Vec<(u8, u8)> = w.iter().filter(|e| e.0 == BUS_DSI_CEC).map(|e| (e.1, e.2)).collect();
        assert_eq!(&dsi[..6], &[(0x1c, 0x40), (0x27, 0xcb), (0x27, 0x8b), (0x27, 0xcb), (0x03, 0x89), (0x55, 0x00)]);
        assert_eq!(&dsi[6..6 + 5], &DSI_CEC_FIXED_REGISTERS);
        assert_eq!(dsi[11], (0x16, 0x18)); // then the timing generator
                                           // order: power, dsi, csc/hdmi, timing, infoframe
        assert!(at(BUS_MAIN, reg::POWER) < at(BUS_DSI_CEC, 0x1c));
        assert!(at(BUS_DSI_CEC, 0x55) < at(BUS_MAIN, reg::HDMI_CFG));
        assert!(at(BUS_MAIN, reg::SYNC_POLARITY) < at(BUS_MAIN, 0x52));
    }

    #[test]
    fn dvi_low_refresh_and_no_timing_generator() {
        let f = Fake::new();
        let b = Bridge::new(&f);
        let mut cfg = Config::new(2);
        cfg.use_timing_gen = false;
        let mode = VideoMode { pixel_clock_hz: 27_000_000, ..mode_720p60() };
        b.start(&cfg, &mode, &Sink { hdmi: false, vic: 0, aspect_16_9: false, refresh_hz: 30 }).unwrap();
        assert_eq!(f.reg(BUS_MAIN, reg::HDMI_CFG) & 2, 0);
        assert_eq!(f.reg(BUS_MAIN, reg::PACKET_ENABLE1) & reg::PACKET_ENABLE1_AVI, 0);
        assert_eq!(f.reg(BUS_MAIN, reg::INFOFRAME_UPDATE) & 0x0c, 3 << 2);
        assert_eq!(f.reg(BUS_DSI_CEC, 0x1c), 0x20);
        assert_eq!(f.reg(BUS_DSI_CEC, 0x27), 0x0b);
        // no timing generator writes: 0x16 and 0x28.. keep their reset value
        assert_eq!(f.reg(BUS_DSI_CEC, 0x16), 0);
        assert_eq!(f.reg(BUS_DSI_CEC, 0x28), 0);
    }

    #[test]
    fn too_fast_for_the_lanes() {
        let f = Fake::new();
        // 2 lanes carry 891000 * 2 / 24 = 74250 kHz
        assert!(mode_valid(&mode_720p60(), 2));
        let fast = VideoMode { pixel_clock_hz: 148_500_000, ..mode_720p60() };
        assert!(!mode_valid(&fast, 2));
        assert!(mode_valid(&fast, 4));
        // the part's own pixel clock limit
        assert!(!mode_valid(&VideoMode { pixel_clock_hz: 154_000_000, ..mode_720p60() }, 4));
        assert_eq!(Bridge::new(&f).start(&Config::new(2), &fast, &SINK), Err(-EINVAL));
        assert!(f.writes().is_empty());
    }

    #[test]
    fn edid_blocks_on_a_powered_part() {
        let f = Fake::new();
        let b = Bridge::new(&f);
        b.start(&Config::new(4), &mode_720p60(), &SINK).unwrap();
        f.clear();
        let mut blk = [0u8; 128];
        b.read_edid_block(0, &mut blk).unwrap();
        assert_eq!(&blk[..], &f.edid[..128]);
        assert_eq!(
            f.ev.borrow().clone(),
            vec![
                Ev::R(BUS_MAIN, reg::STATUS, 1),
                Ev::R(BUS_MAIN, reg::POWER2, 1),
                Ev::W(BUS_MAIN, reg::POWER2, 0x40),
                Ev::R(BUS_MAIN, reg::DDC_STATUS, 1),
                Ev::W(BUS_MAIN, reg::EDID_SEGMENT, 0),
                Ev::R(BUS_MAIN, reg::INT0, 1),
                Ev::R(BUS_MAIN, reg::INT1, 1),
                Ev::W(BUS_MAIN, reg::INT0, 0x04),
                Ev::W(BUS_MAIN, reg::INT1, 0x00),
                Ev::R(BUS_EDID, 0, 64),
                Ev::R(BUS_EDID, 64, 64),
                Ev::R(BUS_EDID, 128, 64),
                Ev::R(BUS_EDID, 192, 64),
            ]
        );
        // block 1: same segment, from the cache, only the HPD probe on the bus
        f.clear();
        b.read_edid_block(1, &mut blk).unwrap();
        assert_eq!(&blk[..], &f.edid[128..256]);
        assert!(!f
            .ev
            .borrow()
            .iter()
            .any(|e| matches!(e, Ev::R(BUS_EDID, ..)) || matches!(e, Ev::W(_, reg::EDID_SEGMENT, _))));
        // block 2: segment 1
        f.clear();
        b.read_edid_block(2, &mut blk).unwrap();
        assert_eq!(&blk[..], &f.edid[256..384]);
        assert!(f.writes().contains(&(BUS_MAIN, reg::EDID_SEGMENT, 1)));
    }

    #[test]
    fn edid_on_an_unpowered_part_powers_it_for_the_read() {
        let f = Fake::new();
        let b = Bridge::new(&f);
        b.init(&Config::new(4)).unwrap();
        f.clear();
        let mut blk = [0u8; 128];
        b.read_edid_block(0, &mut blk).unwrap();
        assert_eq!(&blk[..], &f.edid[..128]);
        let w = f.writes();
        // power on, EDID address rewritten, ..., power off again
        let p = |e| w.iter().position(|x| *x == e).unwrap();
        assert!(p((BUS_MAIN, reg::POWER, 0x10)) < p((BUS_MAIN, reg::EDID_I2C_ADDR, 0x7e)));
        assert!(p((BUS_MAIN, reg::EDID_I2C_ADDR, 0x7e)) < p((BUS_MAIN, reg::EDID_SEGMENT, 0)));
        assert!(p((BUS_MAIN, reg::EDID_SEGMENT, 0)) < p((BUS_MAIN, reg::POWER, 0x50)));
        assert_eq!(f.reg(BUS_MAIN, reg::POWER) & reg::POWER_DOWN, reg::POWER_DOWN);
    }

    #[test]
    fn hpd_low_is_a_clear_error() {
        let f = Fake { hpd: false, ..Fake::new() };
        let b = Bridge::new(&f);
        assert_eq!(b.hpd(), Ok(false));
        f.clear();
        let mut blk = [0u8; 128];
        assert_eq!(b.read_edid_block(0, &mut blk), Err(-ENODEV));
        assert!(!f
            .ev
            .borrow()
            .iter()
            .any(|e| matches!(e, Ev::R(BUS_EDID, ..)) || matches!(e, Ev::W(_, reg::EDID_SEGMENT, _))));
    }

    #[test]
    fn edid_ddc_error_and_timeout() {
        let mut blk = [0u8; 128];
        let f = Fake { ddc_error: true, ..Fake::new() };
        assert_eq!(Bridge::new(&f).read_edid_block(0, &mut blk), Err(-EIO));
        let f = Fake { never_ready: true, ..Fake::new() };
        assert_eq!(Bridge::new(&f).read_edid_block(0, &mut blk), Err(-ETIMEDOUT));
        assert!(!f.ev.borrow().iter().any(|e| matches!(e, Ev::R(BUS_EDID, ..))));
        // 200 ms in 25 ms steps: seven sleeps between eight polls
        assert_eq!(f.ev.borrow().iter().filter(|e| matches!(e, Ev::Sleep(25))).count(), 7);
    }

    #[test]
    fn power_down_undoes_power_up() {
        let f = Fake::new();
        let b = Bridge::new(&f);
        b.start(&Config::new(4), &mode_720p60(), &SINK).unwrap();
        f.clear();
        b.power_down().unwrap();
        assert!(f.writes().contains(&(BUS_DSI_CEC, 0x03, 0x0b)));
        assert_eq!(f.reg(BUS_DSI_CEC, 0x27), 0x0b);
        assert_eq!(f.reg(BUS_MAIN, reg::POWER) & reg::POWER_DOWN, reg::POWER_DOWN);
        assert_eq!(f.reg(BUS_MAIN, reg::POWER2) & reg::POWER2_HPD_OVERRIDE, 0);
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
