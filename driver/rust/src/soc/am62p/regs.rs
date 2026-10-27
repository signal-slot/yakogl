// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The AM62P display registers the drivers use: DSS1 (DISPC: common, VID,
//! VIDL1, OVR, VP), the DSI wrapper, the Cadence DSI host controller and the
//! D-PHY TX.
//!
//! Written by hand from the AM62Px Sitara Processors Technical Reference
//! Manual, TI literature number SPRUJ83D (December 2023, revised January
//! 2026), chapter 14.8 "Registers": 14.8.9.1 DSS, 14.8.9.2 DSS_DSI, 14.8.6.3
//! DPHY_TX. TI's Zephyr HAL (hal_ti, d9d5b1c9) has no K3 display headers,
//! so there is nothing to generate from (docs/source-form.md, group E).
//!
//! Every constant names its TRM register in a `// TRM:` comment, a field as
//! `REGISTER.FIELD` with the bit range of the TRM's field table, so
//! tools/ti-trm/check_regs.py can compare this file against a text copy of
//! the TRM (the TRM itself is never copied into the repository). Register
//! offsets are relative to the region the devicetree names (`common`, `vid`,
//! `vidl1`, `ovr1`, `vp1`, ... for the DSS; `dsi`, `wrap`, `dphy` for the DSI
//! host). Names are the TRM's, without the instance prefix.

/// A field mask from the TRM's `hi:lo` notation.
pub const fn bits(hi: u32, lo: u32) -> u32 {
    (u32::MAX >> (31 - hi)) & !((1u32 << lo) - 1)
}

/// One bit.
pub const fn bit(n: u32) -> u32 {
    1 << n
}

/// DSS common region (`DSS_COMMON_*`, 14.8.9.1.2.1-.24). Host 1's interrupt
/// registers; `common1` holds the same set for the second host.
pub mod common {
    use super::{bit, bits};

    pub const DSS_REVISION: u32 = 0x4; // TRM: DSS_COMMON_DSS_REVISION
    pub const DSS_SYSCONFIG: u32 = 0x8; // TRM: DSS_COMMON_DSS_SYSCONFIG
    pub const SYSCONFIG_SOFTRESET: u32 = bit(1); // TRM: DSS_COMMON_DSS_SYSCONFIG.SOFTRESET
    pub const DSS_SYSSTATUS: u32 = 0x20; // TRM: DSS_COMMON_DSS_SYSSTATUS
    pub const SYSSTATUS_DISPC_FUNC_RESETDONE: u32 = bit(0); // TRM: DSS_COMMON_DSS_SYSSTATUS.DISPC_FUNC_RESETDONE
    pub const SYSSTATUS_DISPC_VP_RESETDONE: u32 = bits(2, 1); // TRM: DSS_COMMON_DSS_SYSSTATUS.DISPC_VP_RESETDONE
    pub const DISPC_IRQ_EOI: u32 = 0x24; // TRM: DSS_COMMON_DISPC_IRQ_EOI
    pub const IRQ_EOI_EOI: u32 = bit(0); // TRM: DSS_COMMON_DISPC_IRQ_EOI.EOI
    pub const DISPC_IRQSTATUS: u32 = 0x2c; // TRM: DSS_COMMON_DISPC_IRQSTATUS
    pub const IRQ_VP: u32 = bits(1, 0); // TRM: DSS_COMMON_DISPC_IRQSTATUS.VP_IRQ
    pub const IRQ_VID: u32 = bits(5, 4); // TRM: DSS_COMMON_DISPC_IRQSTATUS.VID_IRQ
    pub const DISPC_IRQENABLE_SET: u32 = 0x30; // TRM: DSS_COMMON_DISPC_IRQENABLE_SET
    pub const DISPC_IRQENABLE_CLR: u32 = 0x40; // TRM: DSS_COMMON_DISPC_IRQENABLE_CLR
    pub const VID_IRQENABLE_0: u32 = 0x44; // TRM: DSS_COMMON_VID_IRQENABLE_0
    pub const VID_IRQENABLE_1: u32 = 0x48; // TRM: DSS_COMMON_VID_IRQENABLE_1
    pub const VID_IRQSTATUS_0: u32 = 0x58; // TRM: DSS_COMMON_VID_IRQSTATUS_0
    pub const VID_IRQSTATUS_1: u32 = 0x5c; // TRM: DSS_COMMON_VID_IRQSTATUS_1
    pub const VID_IRQ_BUFFERUNDERFLOW: u32 = bit(0); // TRM: DSS_COMMON_VID_IRQSTATUS_0.VIDBUFFERUNDERFLOW_IRQ
    pub const VP_IRQENABLE_0: u32 = 0x70; // TRM: DSS_COMMON_VP_IRQENABLE_0
    pub const VP_IRQENABLE_1: u32 = 0x74; // TRM: DSS_COMMON_VP_IRQENABLE_1
    pub const VP_IRQSTATUS_0: u32 = 0x7c; // TRM: DSS_COMMON_VP_IRQSTATUS_0
    pub const VP_IRQSTATUS_1: u32 = 0x80; // TRM: DSS_COMMON_VP_IRQSTATUS_1
    pub const VP_IRQ_FRAMEDONE: u32 = bit(0); // TRM: DSS_COMMON_VP_IRQSTATUS_0.VPFRAMEDONE_IRQ
    pub const VP_IRQ_VSYNC: u32 = bit(1); // TRM: DSS_COMMON_VP_IRQSTATUS_0.VPVSYNC_IRQ
    pub const VP_IRQ_SYNCLOST: u32 = bit(4); // TRM: DSS_COMMON_VP_IRQSTATUS_0.VPSYNCLOST_IRQ
    pub const VP_IRQ_GO_CLEAR: u32 = bit(11); // TRM: DSS_COMMON_VP_IRQSTATUS_0.VPSYNC_IRQ
    pub const DISPC_GLOBAL_MFLAG_ATTRIBUTE: u32 = 0x90; // TRM: DSS_COMMON_DISPC_GLOBAL_MFLAG_ATTRIBUTE
    pub const MFLAG_CTRL: u32 = bits(1, 0); // TRM: DSS_COMMON_DISPC_GLOBAL_MFLAG_ATTRIBUTE.MFLAG_CTRL
    pub const MFLAG_START: u32 = bit(6); // TRM: DSS_COMMON_DISPC_GLOBAL_MFLAG_ATTRIBUTE.MFLAG_START
    /// MFLAG_CTRL 2: "MFLAG out band signal is dynamically set to 0 or 1
    /// depending on the MFLAG rules".
    pub const MFLAG_CTRL_DYNAMIC: u32 = 2;
}

/// A video pipeline: `vid` (DSS_VID_*, 14.8.9.1.2.89-.251) and `vidl1`
/// (DSS_VIDL1_*, .39-.88) share these offsets; `VID_SIZE` and the resizer
/// exist on `vid` only.
pub mod vid {
    use super::{bit, bits};

    pub const ATTRIBUTES: u32 = 0x20; // TRM: DSS_VID_ATTRIBUTES
    pub const ATTR_ENABLE: u32 = bit(0); // TRM: DSS_VID_ATTRIBUTES.ENABLE
    pub const ATTR_FORMAT: u32 = bits(6, 1); // TRM: DSS_VID_ATTRIBUTES.FORMAT
    pub const ATTR_RESIZEENABLE: u32 = bits(8, 7); // TRM: DSS_VID_ATTRIBUTES.RESIZEENABLE
    pub const ATTR_BUFPRELOAD: u32 = bit(19); // TRM: DSS_VID_ATTRIBUTES.BUFPRELOAD
    pub const ATTR_PREMULTIPLYALPHA: u32 = bit(28); // TRM: DSS_VID_ATTRIBUTES.PREMULTIPLYALPHA
    pub const BA_0: u32 = 0x28; // TRM: DSS_VID_BA_0
    pub const BA_1: u32 = 0x2c; // TRM: DSS_VID_BA_1
    pub const BUF_SIZE_STATUS: u32 = 0x38; // TRM: DSS_VID_BUF_SIZE_STATUS
    pub const BUFSIZE: u32 = bits(15, 0); // TRM: DSS_VID_BUF_SIZE_STATUS.BUFSIZE
    pub const BUF_THRESHOLD: u32 = 0x3c; // TRM: DSS_VID_BUF_THRESHOLD
    pub const BUFHIGHTHRESHOLD: u32 = bits(31, 16); // TRM: DSS_VID_BUF_THRESHOLD.BUFHIGHTHRESHOLD
    pub const BUFLOWTHRESHOLD: u32 = bits(15, 0); // TRM: DSS_VID_BUF_THRESHOLD.BUFLOWTHRESHOLD
    pub const GLOBAL_ALPHA: u32 = 0x1fc; // TRM: DSS_VID_GLOBAL_ALPHA
    pub const MFLAG_THRESHOLD: u32 = 0x208; // TRM: DSS_VID_MFLAG_THRESHOLD
    pub const HT_MFLAG: u32 = bits(31, 16); // TRM: DSS_VID_MFLAG_THRESHOLD.HT_MFLAG
    pub const LT_MFLAG: u32 = bits(15, 0); // TRM: DSS_VID_MFLAG_THRESHOLD.LT_MFLAG
    pub const PICTURE_SIZE: u32 = 0x20c; // TRM: DSS_VID_PICTURE_SIZE
    pub const MEMSIZEY: u32 = bits(27, 16); // TRM: DSS_VID_PICTURE_SIZE.MEMSIZEY
    pub const MEMSIZEX: u32 = bits(11, 0); // TRM: DSS_VID_PICTURE_SIZE.MEMSIZEX
    pub const PIXEL_INC: u32 = 0x210; // TRM: DSS_VID_PIXEL_INC
    pub const PRELOAD: u32 = 0x218; // TRM: DSS_VID_PRELOAD
    pub const PRELOAD_MASK: u32 = bits(11, 0); // TRM: DSS_VID_PRELOAD.PRELOAD
    pub const ROW_INC: u32 = 0x21c; // TRM: DSS_VID_ROW_INC
    pub const SIZE: u32 = 0x220; // TRM: DSS_VID_SIZE
    pub const SIZEY: u32 = bits(27, 16); // TRM: DSS_VID_SIZE.SIZEY
    pub const SIZEX: u32 = bits(11, 0); // TRM: DSS_VID_SIZE.SIZEX
    pub const BA_EXT_0: u32 = 0x22c; // TRM: DSS_VID_BA_EXT_0
    pub const BA_EXT_1: u32 = 0x230; // TRM: DSS_VID_BA_EXT_1
    pub const BA_EXT: u32 = bits(15, 0); // TRM: DSS_VID_BA_EXT_0.BA_EXT

    /// FORMAT codes, 12.9.1.4.1.7 "DISPC Pixel Data Formats" (Table: RGB16-565
    /// 0x03, ARGB32-8888 0x07, xRGB32-8888 0x27).
    pub const FMT_RGB565: u32 = 0x03;
    pub const FMT_ARGB8888: u32 = 0x07;
    pub const FMT_XRGB8888: u32 = 0x27;
}

/// An overlay manager (DSS_OVR1_*, 14.8.9.1.2.252-.260; OVR2 the same).
pub mod ovr {
    use super::{bit, bits};

    pub const CONFIG: u32 = 0x0; // TRM: DSS_OVR1_CONFIG
    pub const DEFAULT_COLOR: u32 = 0x8; // TRM: DSS_OVR1_DEFAULT_COLOR
    pub const DEFAULT_COLOR2: u32 = 0xc; // TRM: DSS_OVR1_DEFAULT_COLOR2
    pub const ATTRIBUTES_0: u32 = 0x20; // TRM: DSS_OVR1_ATTRIBUTES_0
    pub const ATTRIBUTES_1: u32 = 0x24; // TRM: DSS_OVR1_ATTRIBUTES_1
    pub const ATTR_ENABLE: u32 = bit(0); // TRM: DSS_OVR1_ATTRIBUTES_0.ENABLE
    pub const ATTR_CHANNELIN: u32 = bits(4, 1); // TRM: DSS_OVR1_ATTRIBUTES_0.CHANNELIN
    pub const ATTR_POSX: u32 = bits(17, 6); // TRM: DSS_OVR1_ATTRIBUTES_0.POSX
    pub const ATTR_POSY: u32 = bits(30, 19); // TRM: DSS_OVR1_ATTRIBUTES_0.POSY
    /// CHANNELIN: "0 VID is connected to the layer, 1 VIDL-1 is connected".
    pub const CHANNEL_VID: u32 = 0;
    pub const CHANNEL_VIDL1: u32 = 1;

    /// `ATTRIBUTES_<layer>`.
    pub const fn attributes(layer: u32) -> u32 {
        ATTRIBUTES_0 + layer * 4
    }
}

/// A video port (DSS_VP1_*, 14.8.9.1.2.270-.282; VP2 the same).
pub mod vp {
    use super::{bit, bits};

    pub const CONFIG: u32 = 0x0; // TRM: DSS_VP1_CONFIG
    pub const CONTROL: u32 = 0x4; // TRM: DSS_VP1_CONTROL
    pub const CTRL_ENABLE: u32 = bit(0); // TRM: DSS_VP1_CONTROL.ENABLE
    pub const CTRL_GOBIT: u32 = bit(5); // TRM: DSS_VP1_CONTROL.GOBIT
    pub const CTRL_DATALINES: u32 = bits(10, 8); // TRM: DSS_VP1_CONTROL.DATALINES
    /// DATALINES 3: "24-bit output aligned on the LSB of the pixel data
    /// interface".
    pub const DATALINES_24: u32 = 3;
    pub const POL_FREQ: u32 = 0x4c; // TRM: DSS_VP1_POL_FREQ
    pub const POL_IVS: u32 = bit(12); // TRM: DSS_VP1_POL_FREQ.IVS
    pub const POL_IHS: u32 = bit(13); // TRM: DSS_VP1_POL_FREQ.IHS
    pub const POL_IPC: u32 = bit(14); // TRM: DSS_VP1_POL_FREQ.IPC
    pub const POL_IEO: u32 = bit(15); // TRM: DSS_VP1_POL_FREQ.IEO
    pub const POL_RF: u32 = bit(16); // TRM: DSS_VP1_POL_FREQ.RF
    pub const POL_ONOFF: u32 = bit(17); // TRM: DSS_VP1_POL_FREQ.ONOFF
    pub const POL_ALIGN: u32 = bit(18); // TRM: DSS_VP1_POL_FREQ.ALIGN
    pub const SIZE_SCREEN: u32 = 0x50; // TRM: DSS_VP1_SIZE_SCREEN
    pub const SCREEN_LPP: u32 = bits(27, 16); // TRM: DSS_VP1_SIZE_SCREEN.LPP
    pub const SCREEN_PPL: u32 = bits(11, 0); // TRM: DSS_VP1_SIZE_SCREEN.PPL
    pub const TIMING_H: u32 = 0x54; // TRM: DSS_VP1_TIMING_H
    pub const TIMING_H_HBP: u32 = bits(31, 20); // TRM: DSS_VP1_TIMING_H.HBP
    pub const TIMING_H_HFP: u32 = bits(19, 8); // TRM: DSS_VP1_TIMING_H.HFP
    pub const TIMING_H_HSW: u32 = bits(7, 0); // TRM: DSS_VP1_TIMING_H.HSW
    pub const TIMING_V: u32 = 0x58; // TRM: DSS_VP1_TIMING_V
    pub const TIMING_V_VBP: u32 = bits(31, 20); // TRM: DSS_VP1_TIMING_V.VBP
    pub const TIMING_V_VFP: u32 = bits(19, 8); // TRM: DSS_VP1_TIMING_V.VFP
    pub const TIMING_V_VSW: u32 = bits(7, 0); // TRM: DSS_VP1_TIMING_V.VSW
}

/// The DSI wrapper (DSS_DSI0_DSI, 0x3027_0000; 14.8.9.2.2.2).
pub mod dsi_wrap {
    use super::bit;

    pub const DPI_CONTROL: u32 = 0x4; // TRM: DSI_DPI_CONTROL
    pub const DPI_0_EN: u32 = bit(0); // TRM: DSI_DPI_CONTROL.DPI_0_EN
}

/// The Cadence DSI host controller (DSI_TOP_VBUSP_CFG_DSI_0_DSI_*,
/// 0x3050_0000; 14.8.9.2.2.21-.101). `// TRM: DSI:` stands for that prefix.
pub mod dsi {
    use super::{bit, bits};

    pub const IP_CONF: u32 = 0x0; // TRM: DSI:IP_CONF
    pub const IP_CONF_DIRCMD_FIFO_DEPTH: u32 = bits(15, 13); // TRM: DSI:IP_CONF.DIRCMD_FIFO_DEPTH
    pub const IP_CONF_MAX_LANE_NB: u32 = bits(7, 6); // TRM: DSI:IP_CONF.MAX_LANE_NB

    pub const MCTL_MAIN_DATA_CTL: u32 = 0x4; // TRM: DSI:MCTL_MAIN_DATA_CTL
    pub const LINK_EN: u32 = bit(0); // TRM: DSI:MCTL_MAIN_DATA_CTL.LINK_EN
    pub const SDI_IF_VID_MODE: u32 = bit(1); // TRM: DSI:MCTL_MAIN_DATA_CTL.SDI_IF_VID_MODE
    pub const VID_IF_SELECT: u32 = bits(3, 2); // TRM: DSI:MCTL_MAIN_DATA_CTL.VID_IF_SELECT
    /// VID_IF_SELECT "[00 : SDI , 01 : DPI, 10 : DSC]".
    pub const VID_IF_DPI: u32 = 1;
    pub const VID_EN: u32 = bit(5); // TRM: DSI:MCTL_MAIN_DATA_CTL.VID_EN
    pub const HOST_EOT_GEN: u32 = bit(17); // TRM: DSI:MCTL_MAIN_DATA_CTL.HOST_EOT_GEN

    pub const MCTL_MAIN_PHY_CTL: u32 = 0x8; // TRM: DSI:MCTL_MAIN_PHY_CTL
    /// LANE2_EN, LANE3_EN, LANE4_EN: bits 0, 1, 2.
    pub const LANE2_EN: u32 = bit(0); // TRM: DSI:MCTL_MAIN_PHY_CTL.LANE2_EN
    pub const CLK_CONTINUOUS: u32 = bit(4); // TRM: DSI:MCTL_MAIN_PHY_CTL.CLK_CONTINUOUS
    pub const WAIT_BURST_TIME: u32 = bits(13, 10); // TRM: DSI:MCTL_MAIN_PHY_CTL.WAIT_BURST_TIME

    pub const MCTL_MAIN_EN: u32 = 0xc; // TRM: DSI:MCTL_MAIN_EN
    pub const PLL_START: u32 = bit(0); // TRM: DSI:MCTL_MAIN_EN.PLL_START
    pub const CKLANE_EN: u32 = bit(3); // TRM: DSI:MCTL_MAIN_EN.CKLANE_EN
    /// DAT1_EN..DAT4_EN: bits 4..7.
    pub const DAT1_EN: u32 = bit(4); // TRM: DSI:MCTL_MAIN_EN.DAT1_EN
    /// IF2_EN "enables DPI interface [i.e. removes stall signal]".
    pub const IF2_EN: u32 = bit(14); // TRM: DSI:MCTL_MAIN_EN.IF2_EN

    pub const MCTL_DPHY_CFG0: u32 = 0x10; // TRM: DSI:MCTL_DPHY_CFG0
    pub const DPHY_CMN_PSO: u32 = bit(0); // TRM: DSI:MCTL_DPHY_CFG0.DPHY_CMN_PSO
    pub const DPHY_PLL_PSO: u32 = bit(1); // TRM: DSI:MCTL_DPHY_CFG0.DPHY_PLL_PSO
    pub const DPHY_D_PDN: u32 = bits(7, 4); // TRM: DSI:MCTL_DPHY_CFG0.DPHY_D_PDN
    pub const DPHY_C_PDN: u32 = bit(8); // TRM: DSI:MCTL_DPHY_CFG0.DPHY_C_PDN
    pub const DPHY_CMN_PDN: u32 = bit(9); // TRM: DSI:MCTL_DPHY_CFG0.DPHY_CMN_PDN
    pub const DPHY_PLL_PDN: u32 = bit(10); // TRM: DSI:MCTL_DPHY_CFG0.DPHY_PLL_PDN
    pub const DPHY_D_RSTB: u32 = bits(19, 16); // TRM: DSI:MCTL_DPHY_CFG0.DPHY_D_RSTB
    pub const DPHY_C_RSTB: u32 = bit(20); // TRM: DSI:MCTL_DPHY_CFG0.DPHY_C_RSTB

    pub const MCTL_DPHY_TIMEOUT1: u32 = 0x14; // TRM: DSI:MCTL_DPHY_TIMEOUT1
    pub const HSTX_TO_VAL: u32 = bits(21, 4); // TRM: DSI:MCTL_DPHY_TIMEOUT1.HSTX_TO_VAL
    pub const CLK_DIV: u32 = bits(3, 0); // TRM: DSI:MCTL_DPHY_TIMEOUT1.CLK_DIV
    pub const MCTL_DPHY_TIMEOUT2: u32 = 0x18; // TRM: DSI:MCTL_DPHY_TIMEOUT2
    pub const LPRX_TO_VAL: u32 = bits(17, 0); // TRM: DSI:MCTL_DPHY_TIMEOUT2.LPRX_TO_VAL
    pub const MCTL_ULPOUT_TIME: u32 = 0x1c; // TRM: DSI:MCTL_ULPOUT_TIME
    pub const DATA_ULPOUT_TIME: u32 = bits(17, 9); // TRM: DSI:MCTL_ULPOUT_TIME.DATA_ULPOUT_TIME
    pub const CKLANE_ULPOUT_TIME: u32 = bits(8, 0); // TRM: DSI:MCTL_ULPOUT_TIME.CKLANE_ULPOUT_TIME

    pub const MCTL_MAIN_STS: u32 = 0x24; // TRM: DSI:MCTL_MAIN_STS
    pub const STS_PLL_LCK: u32 = bit(0); // TRM: DSI:MCTL_MAIN_STS.PLL_LCK
    pub const STS_CLKLANE_READY: u32 = bit(1); // TRM: DSI:MCTL_MAIN_STS.CLKLANE_READY
    /// DAT1_READY..DAT4_READY: bits 2..5.
    pub const STS_DAT1_READY: u32 = bit(2); // TRM: DSI:MCTL_MAIN_STS.DAT1_READY
    pub const STS_HSTX_TO_ERR: u32 = bit(6); // TRM: DSI:MCTL_MAIN_STS.HSTX_TO_ERR
    pub const STS_LPRX_TO_ERR: u32 = bit(7); // TRM: DSI:MCTL_MAIN_STS.LPRX_TO_ERR

    pub const DIRECT_CMD_SEND: u32 = 0x80; // TRM: DSI:DIRECT_CMD_SEND
    pub const DIRECT_CMD_MAIN_SETTINGS: u32 = 0x84; // TRM: DSI:DIRECT_CMD_MAIN_SETTINGS
    pub const CMD_NAT: u32 = bits(2, 0); // TRM: DSI:DIRECT_CMD_MAIN_SETTINGS.CMD_NAT
    pub const CMD_LONGNOTSHORT: u32 = bit(3); // TRM: DSI:DIRECT_CMD_MAIN_SETTINGS.CMD_LONGNOTSHORT
    pub const CMD_HEAD: u32 = bits(13, 8); // TRM: DSI:DIRECT_CMD_MAIN_SETTINGS.CMD_HEAD
    pub const CMD_ID: u32 = bits(15, 14); // TRM: DSI:DIRECT_CMD_MAIN_SETTINGS.CMD_ID
    pub const CMD_SIZE: u32 = bits(23, 16); // TRM: DSI:DIRECT_CMD_MAIN_SETTINGS.CMD_SIZE
    pub const CMD_LP_EN: u32 = bit(24); // TRM: DSI:DIRECT_CMD_MAIN_SETTINGS.CMD_LP_EN
    pub const DIRECT_CMD_STS: u32 = 0x88; // TRM: DSI:DIRECT_CMD_STS
    pub const CMD_STS_WRITE_COMPLETED: u32 = bit(1); // TRM: DSI:DIRECT_CMD_STS.WRITE_COMPLETED
    pub const DIRECT_CMD_WRDAT: u32 = 0x90; // TRM: DSI:DIRECT_CMD_WRDAT
    pub const DIRECT_CMD_FIFO_RST: u32 = 0x94; // TRM: DSI:DIRECT_CMD_FIFO_RST

    pub const VID_MAIN_CTL: u32 = 0xb0; // TRM: DSI:VID_MAIN_CTL
    pub const VID_HEADER: u32 = bits(13, 8); // TRM: DSI:VID_MAIN_CTL.HEADER
    pub const VID_PIXEL_MODE: u32 = bits(17, 14); // TRM: DSI:VID_MAIN_CTL.VID_PIXEL_MODE
    pub const BURST_MODE: u32 = bit(18); // TRM: DSI:VID_MAIN_CTL.BURST_MODE
    pub const SYNC_PULSE_ACTIVE: u32 = bit(19); // TRM: DSI:VID_MAIN_CTL.SYNC_PULSE_ACTIVE
    pub const SYNC_PULSE_HORIZONTAL: u32 = bit(20); // TRM: DSI:VID_MAIN_CTL.SYNC_PULSE_HORIZONTAL
    pub const REG_BLKLINE_MODE: u32 = bits(22, 21); // TRM: DSI:VID_MAIN_CTL.REG_BLKLINE_MODE
    pub const REG_BLKEOL_MODE: u32 = bits(24, 23); // TRM: DSI:VID_MAIN_CTL.REG_BLKEOL_MODE
    pub const RECOVERY_MODE: u32 = bits(26, 25); // TRM: DSI:VID_MAIN_CTL.RECOVERY_MODE
    pub const VID_IGNORE_MISS_VSYNC: u32 = bit(31); // TRM: DSI:VID_MAIN_CTL.VID_IGNORE_MISS_VSYNC
    /// REG_BLKLINE_MODE "[1x: LP, 01: blanking packet - 00: NULL packet]".
    pub const BLK_MODE_BLANKING_PACKET: u32 = 1;
    /// VID_PIXEL_MODE "0000: 16 bits RGB ... 0011: 24 bits RGB".
    pub const PIXEL_MODE_RGB565: u32 = 0b0000;
    pub const PIXEL_MODE_RGB888: u32 = 0b0011;
    pub const VID_VSIZE1: u32 = 0xb4; // TRM: DSI:VID_VSIZE1
    pub const VSA_LENGTH: u32 = bits(5, 0); // TRM: DSI:VID_VSIZE1.VSA_LENGTH
    pub const VBP_LENGTH: u32 = bits(11, 6); // TRM: DSI:VID_VSIZE1.VBP_LENGTH
    pub const VFP_LENGTH: u32 = bits(19, 12); // TRM: DSI:VID_VSIZE1.VFP_LENGTH
    pub const VID_VSIZE2: u32 = 0xb8; // TRM: DSI:VID_VSIZE2
    pub const VACT_LENGTH: u32 = bits(12, 0); // TRM: DSI:VID_VSIZE2.VACT_LENGTH
    pub const VID_HSIZE1: u32 = 0xc0; // TRM: DSI:VID_HSIZE1
    pub const HSA_LENGTH: u32 = bits(9, 0); // TRM: DSI:VID_HSIZE1.HSA_LENGTH
    pub const HBP_LENGTH: u32 = bits(31, 16); // TRM: DSI:VID_HSIZE1.HBP_LENGTH
    pub const VID_HSIZE2: u32 = 0xc4; // TRM: DSI:VID_HSIZE2
    pub const RGB_SIZE: u32 = bits(14, 0); // TRM: DSI:VID_HSIZE2.RGB_SIZE
    pub const HFP_LENGTH: u32 = bits(26, 16); // TRM: DSI:VID_HSIZE2.HFP_LENGTH
    pub const VID_BLKSIZE1: u32 = 0xcc; // TRM: DSI:VID_BLKSIZE1
    pub const BLKLINE_EVENT_PCK: u32 = bits(14, 0); // TRM: DSI:VID_BLKSIZE1.BLKLINE_EVENT_PCK
    pub const VID_BLKSIZE2: u32 = 0xd0; // TRM: DSI:VID_BLKSIZE2
    pub const BLKLINE_PULSE_PCK: u32 = bits(14, 0); // TRM: DSI:VID_BLKSIZE2.BLKLINE_PULSE_PCK
    pub const VID_DPHY_TIME: u32 = 0xdc; // TRM: DSI:VID_DPHY_TIME
    pub const REG_LINE_DURATION: u32 = bits(16, 0); // TRM: DSI:VID_DPHY_TIME.REG_LINE_DURATION
    pub const REG_WAKEUP_TIME: u32 = bits(27, 17); // TRM: DSI:VID_DPHY_TIME.REG_WAKEUP_TIME
    pub const VID_MODE_STS: u32 = 0xf0; // TRM: DSI:VID_MODE_STS
    pub const VID_VCA_SETTING2: u32 = 0xf8; // TRM: DSI:VID_VCA_SETTING2
    pub const MAX_LINE_LIMIT: u32 = bits(31, 16); // TRM: DSI:VID_VCA_SETTING2.MAX_LINE_LIMIT

    pub const MCTL_MAIN_STS_CTL: u32 = 0x130; // TRM: DSI:MCTL_MAIN_STS_CTL
    pub const CMD_MODE_STS_CTL: u32 = 0x134; // TRM: DSI:CMD_MODE_STS_CTL
    pub const DIRECT_CMD_STS_CTL: u32 = 0x138; // TRM: DSI:DIRECT_CMD_STS_CTL
    pub const DIRECT_CMD_RD_STS_CTL: u32 = 0x13c; // TRM: DSI:DIRECT_CMD_RD_STS_CTL
    pub const VID_MODE_STS_CTL: u32 = 0x140; // TRM: DSI:VID_MODE_STS_CTL
    pub const TVG_STS_CTL: u32 = 0x144; // TRM: DSI:TVG_STS_CTL
    pub const MCTL_DPHY_ERR_CTL1: u32 = 0x148; // TRM: DSI:MCTL_DPHY_ERR_CTL1
    pub const MCTL_MAIN_STS_CLR: u32 = 0x150; // TRM: DSI:MCTL_MAIN_STS_CLR
    pub const PLL_LOCK_CLR: u32 = bit(0); // TRM: DSI:MCTL_MAIN_STS_CLR.PLL_LOCK_CLR
    pub const DIRECT_CMD_STS_CLR: u32 = 0x158; // TRM: DSI:DIRECT_CMD_STS_CLR
    pub const ID_REG: u32 = 0x1fc; // TRM: DSI:ID_REG
    pub const REV_VENDOR_ID: u32 = bits(31, 20); // TRM: DSI:ID_REG.REV_VENDOR_ID
    pub const REV_PRODUCT_ID: u32 = bits(19, 12); // TRM: DSI:ID_REG.REV_PRODUCT_ID
    /// ID_REG reset value: vendor 0xCAD (Cadence), product 0xD5.
    pub const VENDOR_CADENCE: u32 = 0xcad;
    pub const PRODUCT_DSI: u32 = 0xd5;
}

/// The D-PHY TX (DPHY_TX0, 0x301C_0000; 14.8.6.3.2). The Cadence PHY's own
/// registers; the WIZ wrapper's are in [`wiz`].
pub mod dphy {
    use super::{bit, bits};

    pub const CMN_DIG_TBIT2: u32 = 0x20; // TRM: DPHY_TX_DPHYTX_CMN0_CMN_DIG_TBIT2
    pub const CMN_SSM_EN: u32 = bit(0); // TRM: DPHY_TX_DPHYTX_CMN0_CMN_DIG_TBIT2.CMN0_O_CMN_SSM_EN
    pub const SSM_WAIT_BGCAL_EN: u32 = bits(8, 1); // TRM: DPHY_TX_DPHYTX_CMN0_CMN_DIG_TBIT2.CMN0_O_SSM_WAIT_BGCAL_EN
    pub const CMN_TX_MODE_EN: u32 = bit(9); // TRM: DPHY_TX_DPHYTX_CMN0_CMN_DIG_TBIT2.CMN0_O_CMN_TX_MODE_EN
    pub const CMN_DIG_TBIT10: u32 = 0x40; // TRM: DPHY_TX_DPHYTX_CMN0_CMN_DIG_TBIT10
    pub const PLL_GM_PWM_DIV_HIGH: u32 = bits(9, 0); // TRM: DPHY_TX_DPHYTX_CMN0_CMN_DIG_TBIT10.CMN0_O_ANA_PLL_GM_PWM_DIV_HIGH
    pub const PLL_GM_PWM_DIV_LOW: u32 = bits(19, 10); // TRM: DPHY_TX_DPHYTX_CMN0_CMN_DIG_TBIT10.CMN0_O_ANA_PLL_GM_PWM_DIV_LOW
    pub const PLL_BYTECLK_DIV: u32 = bits(27, 20); // TRM: DPHY_TX_DPHYTX_CMN0_CMN_DIG_TBIT10.CMN0_O_ANA_PLL_BYTECLK_DIV
    pub const PCS_TX_DIG_TBIT0: u32 = 0xb00; // TRM: DPHY_TX_DPHYTX_PCS_TX_DIG_TBIT0
    pub const PCS_BAND_CTL_REG_L: u32 = bits(4, 0); // TRM: DPHY_TX_DPHYTX_PCS_TX_DIG_TBIT0.PCS_BAND_CTL_REG_L
    pub const PCS_BAND_CTL_REG_R: u32 = bits(9, 5); // TRM: DPHY_TX_DPHYTX_PCS_TX_DIG_TBIT0.PCS_BAND_CTL_REG_R
}

/// The WIZ wrapper of the D-PHY TX (DPHY_TX_WIZ_CONFIG_*, 14.8.6.3.2.12-.19).
/// Offsets from the wrapper's base, which the TRM's summary table puts at
/// the start of the D-PHY region; TI's driver puts it at 0xF00
/// ([`crate::soc::am62p::ti_pr114866::WIZ_PLL_CTRL`] - 4). ip::cdns_dphy
/// identifies it by `MOD_VER` (below).
pub mod wiz {
    use super::{bit, bits};

    pub const MOD_VER: u32 = 0x0; // TRM: DPHY_TX_WIZ_CONFIG_MOD_VER
    pub const MODULE_ID: u32 = bits(27, 16); // TRM: DPHY_TX_WIZ_CONFIG_MOD_VER.MODULE_ID
    /// MODULE_ID reset value: "992h WIZ16B8M4CDT3 module ID".
    pub const MODULE_ID_WIZ16B8M4CDT3: u32 = 0x992;
    pub const PLL_CTRL: u32 = 0x4; // TRM: DPHY_TX_WIZ_CONFIG_PLL_CTRL
    pub const PLL_IPDIV: u32 = bits(4, 0); // TRM: DPHY_TX_WIZ_CONFIG_PLL_CTRL.PLL_IPDIV
    pub const PLL_OPDIV: u32 = bits(13, 8); // TRM: DPHY_TX_WIZ_CONFIG_PLL_CTRL.PLL_OPDIV
    pub const PLL_FBDIV: u32 = bits(25, 16); // TRM: DPHY_TX_WIZ_CONFIG_PLL_CTRL.PLL_FBDIV
    pub const PLL_PD: u32 = bit(28); // TRM: DPHY_TX_WIZ_CONFIG_PLL_CTRL.PLL_PD
    pub const PLL_PSO: u32 = bit(29); // TRM: DPHY_TX_WIZ_CONFIG_PLL_CTRL.PLL_PSO
    pub const PLL_LOCK: u32 = bit(31); // TRM: DPHY_TX_WIZ_CONFIG_PLL_CTRL.PLL_LOCK
    pub const STATUS: u32 = 0x8; // TRM: DPHY_TX_WIZ_CONFIG_STATUS
    pub const O_CMN_READY: u32 = bit(31); // TRM: DPHY_TX_WIZ_CONFIG_STATUS.O_CMN_READY
    pub const RST_CTRL: u32 = 0xc; // TRM: DPHY_TX_WIZ_CONFIG_RST_CTRL
    pub const LANE_RSTB_CMN: u32 = bit(31); // TRM: DPHY_TX_WIZ_CONFIG_RST_CTRL.LANE_RSTB_CMN
    pub const PSM_FREQ: u32 = 0x10; // TRM: DPHY_TX_WIZ_CONFIG_PSM_FREQ
    pub const PSM_CLOCK_FREQ: u32 = bits(7, 0); // TRM: DPHY_TX_WIZ_CONFIG_PSM_FREQ.PSM_CLOCK_FREQ
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_and_the_generated_wiz_offsets_agree_with_the_trm() {
        assert_eq!(bits(31, 0), u32::MAX);
        assert_eq!(bits(6, 1), 0x7e);
        assert_eq!(bits(0, 0), 1);
        // The TI driver's WIZ offsets are the TRM's, moved by one base.
        use crate::soc::am62p::ti_pr114866 as ti;
        let base = ti::WIZ_PLL_CTRL - wiz::PLL_CTRL;
        assert_eq!(base, 0xf00);
        assert_eq!(ti::WIZ_STATUS, base + wiz::STATUS);
        assert_eq!(ti::WIZ_RST_CTRL, base + wiz::RST_CTRL);
        assert_eq!(ti::WIZ_PSM_FREQ, base + wiz::PSM_FREQ);
        // The TI driver's PWM register and shifts are the TRM's TBIT10 fields.
        assert_eq!(ti::CMN_PWM_OFFSET, dphy::CMN_DIG_TBIT10);
        assert_eq!(ti::CMN_PWM_SHIFTS[0], dphy::PLL_GM_PWM_DIV_HIGH.trailing_zeros());
        assert_eq!(ti::CMN_PWM_SHIFTS[1], dphy::PLL_GM_PWM_DIV_LOW.trailing_zeros());
        assert_eq!(ti::CMN_PWM_SHIFTS[2], dphy::PLL_BYTECLK_DIV.trailing_zeros());
        // ... and its MCTL_DPHY_CFG0 values use only the TRM's fields.
        let known = dsi::DPHY_CMN_PSO
            | dsi::DPHY_PLL_PSO
            | dsi::DPHY_D_PDN
            | dsi::DPHY_C_PDN
            | dsi::DPHY_CMN_PDN
            | dsi::DPHY_PLL_PDN
            | dsi::DPHY_D_RSTB
            | dsi::DPHY_C_RSTB;
        for v in ti::CFG0_SEQUENCE {
            assert_eq!(v & !known, 0, "{v:#x}");
        }
        assert_eq!(ti::CFG0_D_RSTB_LANE0, 1 << dsi::DPHY_D_RSTB.trailing_zeros());
    }
}
