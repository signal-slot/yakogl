// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The i.MX 95 DPU ("Seeris", 0x4B400000): one XRGB8888 plane to display
//! stream 0. FetchLayer0's sublayer 0 is the secondary input of LayerBlend1
//! over ConstFrame0 (black), LayerBlend1 feeds ExtDst0, the content stream
//! reaches FrameGen0 through DomainBlend0; FrameGen0 runs the mode and kicks
//! the pipeline. Signature0 measures CRCs of what the display engine sends.
//! driver/docs/imx95-display.md sections 1 and 4(b) have the block map.
//!
//! Sources, by their licence (docs/source-form.md):
//! - registers and fields: NXP's BSD-3 `PERI_DISPLAY_SEERIS.h` (hal_nxp
//!   1e91007b), generated into [`super::regs`];
//! - the sequence and every value: hal_nxp's BSD-3 `drivers/dpu_1/fsl_dpu.c`
//!   (`DPU_Init` :1086, `DPU_PreparePathConfig` :1151, `DPU_InitPipeline`
//!   :1286, `DPU_InitExtDst` :2175, `DPU_InitLayerBlend` :1559,
//!   `DPU_SetLayerBlendConfig` :1597, `DPU_EnableLayerBlend` :1628,
//!   `DPU_InitDomainBlend` :2543, `DPU_InitConstFrame` :1648,
//!   `DPU_InitFetchUnit` :1885, `DPU_SetFetchUnitSrcBufferConfig` :2040,
//!   `DPU_InitDisplayTiming` :2315, `DPU_SetDisplayConfig` :2426,
//!   `DPU_StartDisplay` :2478, the triggers :1323, :2513, :2567) in the
//!   order Zephyr's Apache-2.0 `display_socionext_dpu.c` calls them for this
//!   SoC (`dpu_init` :209-330, `dpu_write` :63-126), with FetchLayer0 in
//!   place of Zephyr's FetchYUV0 (the plane Linux and U-Boot use, the note's
//!   section 1);
//! - the signature unit: hal_nxp's BSD-3 `drivers/dpu/fsl_dpu.c`, the same
//!   unit on the earlier DPU (`DPU_InitSignature` :3429,
//!   `DPU_SetSignatureWindowConfig` :3490, `DPU_TriggerSignatureShadowLoad`),
//!   on the i.MX 95 header's registers;
//! - facts only (GPL, no code): NXP's `dpu95` and `imx95-pixel-*` drivers
//!   (Toradex's 6.18 tree) for what the permissive sources leave open; each
//!   is named where it is used.
//!
//! Where the header and `fsl_dpu.c` differ, the header wins: FetchLayer0's
//! `STATICCONTROL` has only `ShdEn` (the driver also writes 0xFF000000, the
//! sticky bits of an older layout, which this header puts in
//! `SHDLDREQCONTROL`, set by `DPU_PreparePathConfig` and by Linux's
//! `dpu95_fl_shdldreq_sticky(fu, 0xff)`). LayerBlend1's clock enable is
//! written as "automatic" (Linux `dpu95_lb_pec_clken(lb, CLKEN_AUTOMATIC)`),
//! where `fsl_dpu.c` keeps the reset value, which no source states.
//!
//! Only the units on this path are touched: the other endpoints (ExtDst1/4/5,
//! Store9) and units are left at their power-on state, which is what a cold
//! DISPLAY domain gives. An image that adopts a running display needs
//! `DPU_PreparePathConfig`'s full disconnect first (not here).

use super::regs::display_seeris as r;
use crate::mmio::{field, get, Mmio};
use crate::video::VideoMode;

/// `DISPLAY_SEERIS_BASE` (MIMX9596 `_COMMON.h`; `nxp,imx95-dpu` reg).
pub const DPU_BASE: usize = 0x4B40_0000;

/// Unit ids of the pixel engine's dynamic source selects
/// (`PIXENG_*CFG_*_DYNAMIC` value lists in the header; `fsl_dpu.h`
/// `kDPU_UnitSrc*`).
pub const SRC_CONSTFRAME0: u32 = 0b001100;
pub const SRC_LAYERBLEND1: u32 = 0b010100;
pub const SRC_FETCHLAYER0: u32 = 0b011010;

/// `ClkEn` "automatic": the unit's clock runs while it is used.
pub const CLKEN_AUTOMATIC: u32 = 0b01;

/// LayerBlend shadow token and load select "any input" (header 0b10;
/// `kDPU_LayerBlendShadowTokenBoth`, `...LoadBoth`).
const LB_ANY: u32 = 0b10;
/// DomainBlend shadow token and load select "from ControlTrigger" (header
/// 0b100; `DPU_DOMAINBLEND_SW`).
const DB_CONTROLTRIGGER: u32 = 0b100;
/// Blend functions (header: 0b000 `Cin * 0`, 0b001 `Cin * 1`).
const BLEND_ZERO: u32 = 0;
const BLEND_ONE: u32 = 1;
/// FrameGen display modes (header `FgDm`): primary input only; the panic
/// mode `fsl_dpu.c` defaults to (white with test pattern).
pub const FGDM_PRIM: u32 = 0b010;
pub const FGDM_TEST: u32 = 0b110;

/// `fsl_dpu.h` `DPU_FETCH_UNIT_BURST_LENGTH` and the fetch units' buffer
/// count (`DPU_InitFetchUnit`).
const BURST_LENGTH: u32 = 16;
const NUM_BUFFERS: u32 = 16;
/// The fetch unit's resampling deltas for 1:1 (`DPU_MAKE_FETCH_RESAMPLING(0,
/// 4, 4, 0, 0)`).
const RESAMPLE_DELTA: u32 = 4;

/// `kDPU_Group0ExtDst0ShadowLoadInterrupt` (`fsl_dpu.h`): bit 3 of
/// `IRQ_INTERRUPTSTATUS0`, set when ExtDst0's pipeline loaded its shadows.
pub const IRQ_EXTDST0_SHDLD: u32 = 1 << 3;

/// Bytes per XRGB8888 pixel.
pub const BPP: u32 = 4;
/// The largest timing value FrameGen takes (14-bit fields).
pub const MAX_TIMING: u32 = 0x3fff;
/// A fetch address has 40 bits (`BASEADDRESSMSB0` holds 39:32).
pub const ADDR_BITS: u32 = 40;
/// The poll step of the waits, µs.
pub const POLL_STEP_US: u32 = 100;

/// The scan-out buffer: XRGB8888, the mode's size, `stride` bytes a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Framebuffer {
    /// The bus address (the DPU node has no IOMMU).
    pub addr: u64,
    pub stride: u32,
}

/// Why a mode or a buffer is refused, before any register is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A field of the mode is 0, or a total does not fit FrameGen's 14 bits.
    Mode,
    /// The address or the stride is not a multiple of 4 (`fsl_dpu.c`
    /// `DPU_CheckBufferAlignment`, 32 bpp).
    Align,
    /// The stride is below a line of the mode, or above the 16-bit field.
    Stride,
    /// The address does not fit 40 bits.
    Address,
}

/// The FrameGen timing words and the stream's polarity, from a mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    pub htcfg1: u32,
    pub htcfg2: u32,
    pub vtcfg1: u32,
    pub vtcfg2: u32,
    /// `PKICKCONFIG` and `SKICKCONFIG`.
    pub kick: u32,
    /// `POLARITYCTRL0` of Dither0's configuration block.
    pub polarity: u32,
}

/// Where stream 0 goes after the display engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// Pixel interleaver channel 0 in bypass, pixel link 0, the DSI host.
    Dsi,
}

/// The FrameGen words for `mode` on `route` (`DPU_InitDisplayTiming`).
///
/// For DSI the vertical sync starts one line earlier (front porch - 1,
/// back porch + 1), as Zephyr's driver for this SoC passes its timing
/// (`display_socionext_dpu.c:303-304`) and Linux's `dpu95` does for DSI on
/// the i.MX 95 (`vsbp_quirk`, a fact); and the stream's syncs are active
/// low and DE active high whatever the panel's (Linux `dpu95_crtc_mode_set_nofb`
/// for a DSI encoder): the panel's polarities are the DSI host's and the
/// bridge's business.
pub fn timing(mode: &VideoMode, route: Route) -> Result<Timing, Refusal> {
    let (vfront, vback) = match route {
        Route::Dsi => (mode.vfront.wrapping_sub(1), mode.vback + 1),
    };
    if mode.check().is_err() || vfront == 0 || mode.htotal() > MAX_TIMING + 1 || mode.vtotal() > MAX_TIMING {
        return Err(Refusal::Mode);
    }
    let htotal = mode.htotal();
    let vtotal = mode.vactive + vfront + mode.vsync + vback - 1;
    let polarity = match route {
        Route::Dsi => r::diseng_dither0cfg_polarityctrl0::POLEN0_MASK,
    };
    Ok(Timing {
        htcfg1: field(r::diseng_framegen0_htcfg1::HTOTAL_MASK, htotal - 1)
            | field(r::diseng_framegen0_htcfg1::HACT_MASK, mode.hactive),
        htcfg2: field(r::diseng_framegen0_htcfg2::HSYNC_MASK, mode.hsync - 1)
            | field(r::diseng_framegen0_htcfg2::HSBP_MASK, mode.hback + mode.hsync - 1)
            | r::diseng_framegen0_htcfg2::HSEN_MASK,
        vtcfg1: field(r::diseng_framegen0_vtcfg1::VTOTAL_MASK, vtotal)
            | field(r::diseng_framegen0_vtcfg1::VACT_MASK, mode.vactive),
        vtcfg2: field(r::diseng_framegen0_vtcfg2::VSYNC_MASK, mode.vsync - 1)
            | field(r::diseng_framegen0_vtcfg2::VSBP_MASK, vback + mode.vsync - 1)
            | r::diseng_framegen0_vtcfg2::VSEN_MASK,
        kick: field(r::diseng_framegen0_pkickconfig::PKICKROW_MASK, vtotal + 1)
            | field(r::diseng_framegen0_pkickconfig::PKICKCOL_MASK, htotal)
            | r::diseng_framegen0_pkickconfig::PKICKEN_MASK,
        polarity,
    })
}

/// The buffer checks.
pub fn check_fb(mode: &VideoMode, fb: &Framebuffer) -> Result<(), Refusal> {
    if fb.addr & 3 != 0 || fb.stride & 3 != 0 {
        return Err(Refusal::Align);
    }
    if fb.stride < mode.hactive * BPP || fb.stride - 1 > r::pixeng_fetchlayer0_sourcebufferattributes0::STRIDE0_MASK {
        return Err(Refusal::Stride);
    }
    if fb.addr >> ADDR_BITS != 0 {
        return Err(Refusal::Address);
    }
    Ok(())
}

/// `height - 1` and `width - 1` as the dimension registers take them.
fn dim(w: u32, h: u32) -> u32 {
    field(r::pixeng_constframe0_framedimensions::FRAMEHEIGHT_MASK, h - 1)
        | field(r::pixeng_constframe0_framedimensions::FRAMEWIDTH_MASK, w - 1)
}

fn w(m: &impl Mmio, base: usize, off: u32, v: u32) {
    m.write32(base + off as usize, v);
}

fn rd(m: &impl Mmio, base: usize, off: u32) -> u32 {
    m.read32(base + off as usize)
}

fn modify(m: &impl Mmio, base: usize, off: u32, mask: u32, v: u32) {
    m.modify32(base + off as usize, mask, v);
}

/// The fetch address registers of sublayer 0 (low word, then bits 39:32).
fn set_address(m: &impl Mmio, base: usize, addr: u64) {
    w(m, base, r::PIXENG_FETCHLAYER0_BASEADDRESS0, addr as u32);
    w(m, base, r::PIXENG_FETCHLAYER0_BASEADDRESSMSB0, (addr >> 32) as u32);
}

/// Configure the pipeline and FrameGen0 for `mode` with `fb` on screen; the
/// FrameGen is not enabled ([`start`]). The order is `fsl_dpu.c`'s as
/// Zephyr calls it (module documentation). `t` comes from [`timing`] and
/// `fb` has passed [`check_fb`].
pub fn configure(m: &impl Mmio, base: usize, mode: &VideoMode, t: &Timing, fb: &Framebuffer) {
    let (wd, ht) = (mode.hactive, mode.vactive);
    // DPU_Init: every unit allowed in ExtDst0's domain.
    w(m, base, r::DOMAINMASK_EXTDST0_DOMAIN_MASK0, 0xffff_ffff);
    // DPU_PreparePathConfig, for this path: ExtDst0's pipeline without
    // shadow loads, its and LayerBlend1's sources disconnected, every
    // FetchLayer0 sublayer's shadow load request sticky.
    modify(m, base, r::PIXENG_EXTDST0CFG_EXTDST0_STATIC, r::pixeng_extdst0cfg_extdst0_static::SHDEN_MASK, 0);
    modify(m, base, r::PIXENG_EXTDST0CFG_EXTDST0_DYNAMIC, r::pixeng_extdst0cfg_extdst0_dynamic::SRC_SEL_MASK, 0);
    let lb_src = r::pixeng_layerblend1cfg_layerblend1_dynamic::PRIM_SEL_MASK
        | r::pixeng_layerblend1cfg_layerblend1_dynamic::SEC_SEL_MASK;
    modify(m, base, r::PIXENG_LAYERBLEND1CFG_LAYERBLEND1_DYNAMIC, lb_src, 0);
    w(m, base, r::PIXENG_FETCHLAYER0_SHDLDREQCONTROL, 0xff);
    // DPU_InitPipeline(ExtDst0): powered, shadow loads on.
    modify(
        m,
        base,
        r::PIXENG_EXTDST0CFG_EXTDST0_STATIC,
        r::pixeng_extdst0cfg_extdst0_static::POWERDOWN_MASK | r::pixeng_extdst0cfg_extdst0_static::SHDEN_MASK,
        r::pixeng_extdst0cfg_extdst0_static::SHDEN_MASK,
    );
    // DPU_InitExtDst(ExtDst0, LayerBlend1): kicked by FrameGen0.
    w(
        m,
        base,
        r::PIXENG_EXTDST0_STATICCONTROL,
        r::pixeng_extdst0_staticcontrol::KICK_MODE_MASK | r::pixeng_extdst0_staticcontrol::SHDEN_MASK,
    );
    modify(
        m,
        base,
        r::PIXENG_EXTDST0CFG_EXTDST0_DYNAMIC,
        r::pixeng_extdst0cfg_extdst0_dynamic::SRC_SEL_MASK,
        SRC_LAYERBLEND1,
    );
    // DPU_InitLayerBlend(LayerBlend1, ConstFrame0, FetchLayer0).
    w(
        m,
        base,
        r::PIXENG_LAYERBLEND1_STATICCONTROL,
        r::pixeng_layerblend1_staticcontrol::SHDEN_MASK
            | field(r::pixeng_layerblend1_staticcontrol::SHDTOKSEL_MASK, LB_ANY)
            | field(r::pixeng_layerblend1_staticcontrol::SHDLDSEL_MASK, LB_ANY),
    );
    w(m, base, r::PIXENG_LAYERBLEND1_POSITION, 0);
    modify(
        m,
        base,
        r::PIXENG_LAYERBLEND1CFG_LAYERBLEND1_DYNAMIC,
        lb_src | r::pixeng_layerblend1cfg_layerblend1_dynamic::CLKEN_MASK,
        field(r::pixeng_layerblend1cfg_layerblend1_dynamic::PRIM_SEL_MASK, SRC_CONSTFRAME0)
            | field(r::pixeng_layerblend1cfg_layerblend1_dynamic::SEC_SEL_MASK, SRC_FETCHLAYER0)
            | field(r::pixeng_layerblend1cfg_layerblend1_dynamic::CLKEN_MASK, CLKEN_AUTOMATIC),
    );
    // DPU_LayerBlendGetDefaultConfig + DPU_SetLayerBlendConfig: the
    // secondary (the plane) over a zeroed primary, no alpha mask; then
    // DPU_EnableLayerBlend: blending mode.
    w(
        m,
        base,
        r::PIXENG_LAYERBLEND1_BLENDCONTROL,
        field(r::pixeng_layerblend1_blendcontrol::PRIM_C_BLD_FUNC_MASK, BLEND_ZERO)
            | field(r::pixeng_layerblend1_blendcontrol::SEC_C_BLD_FUNC_MASK, BLEND_ONE)
            | field(r::pixeng_layerblend1_blendcontrol::PRIM_A_BLD_FUNC_MASK, BLEND_ZERO)
            | field(r::pixeng_layerblend1_blendcontrol::SEC_A_BLD_FUNC_MASK, BLEND_ONE),
    );
    modify(
        m,
        base,
        r::PIXENG_LAYERBLEND1_CONTROL,
        r::pixeng_layerblend1_control::MODE_MASK
            | r::pixeng_layerblend1_control::ALPHAMASKENABLE_MASK
            | r::pixeng_layerblend1_control::ALPHAMASKMODE_MASK,
        r::pixeng_layerblend1_control::MODE_MASK,
    );
    // DPU_InitDomainBlend(DomainBlend0): shadows from its ControlTrigger.
    w(
        m,
        base,
        r::DISENG_DOMAINBLEND0_STATICCONTROL,
        r::diseng_domainblend0_staticcontrol::SHDEN_MASK
            | field(r::diseng_domainblend0_staticcontrol::SHDTOKSEL_MASK, DB_CONTROLTRIGGER)
            | field(r::diseng_domainblend0_staticcontrol::SHDLDSEL_MASK, DB_CONTROLTRIGGER),
    );
    w(
        m,
        base,
        r::DISENG_DOMAINBLEND0_BLENDCONTROL,
        field(r::diseng_domainblend0_blendcontrol::SEC_C_BLD_FUNC_MASK, BLEND_ONE),
    );
    // ConstFrame0: the mode's size, black.
    w(m, base, r::PIXENG_CONSTFRAME0_STATICCONTROL, r::pixeng_constframe0_staticcontrol::SHDEN_MASK);
    w(m, base, r::PIXENG_CONSTFRAME0_FRAMEDIMENSIONS, dim(wd, ht));
    w(m, base, r::PIXENG_CONSTFRAME0_CONSTANTCOLOR, 0);
    // DPU_InitFetchUnit(FetchLayer0).
    w(m, base, r::PIXENG_FETCHLAYER0_STATICCONTROL, r::pixeng_fetchlayer0_staticcontrol::SHDEN_MASK);
    w(
        m,
        base,
        r::PIXENG_FETCHLAYER0_BURSTBUFFERMANAGEMENT,
        field(r::pixeng_fetchlayer0_burstbuffermanagement::SETNUMBUFFERS_MASK, NUM_BUFFERS)
            | field(r::pixeng_fetchlayer0_burstbuffermanagement::SETBURSTLENGTH_MASK, BURST_LENGTH),
    );
    w(m, base, r::PIXENG_FETCHLAYER0_FRAMEDIMENSIONS, dim(wd, ht));
    modify(m, base, r::PIXENG_FETCHLAYER0_CONTROL, r::pixeng_fetchlayer0_control::RAWPIXEL_MASK, 0);
    w(
        m,
        base,
        r::PIXENG_FETCHLAYER0_FRAMERESAMPLING,
        field(r::pixeng_fetchlayer0_frameresampling::DELTAY_MASK, RESAMPLE_DELTA)
            | field(r::pixeng_fetchlayer0_frameresampling::DELTAX_MASK, RESAMPLE_DELTA),
    );
    // DPU_SetFetchUnitSrcBufferConfig(sublayer 0): XRGB8888 is
    // kDPU_PixelFormatRGB888's components (8/8/8 at 16/8/0, no alpha) in 32
    // bits a pixel; the missing alpha comes from the constant colour, opaque.
    set_address(m, base, fb.addr);
    w(
        m,
        base,
        r::PIXENG_FETCHLAYER0_SOURCEBUFFERATTRIBUTES0,
        field(r::pixeng_fetchlayer0_sourcebufferattributes0::BITSPERPIXEL0_MASK, 8 * BPP)
            | field(r::pixeng_fetchlayer0_sourcebufferattributes0::STRIDE0_MASK, fb.stride - 1),
    );
    w(
        m,
        base,
        r::PIXENG_FETCHLAYER0_SOURCEBUFFERDIMENSION0,
        field(r::pixeng_fetchlayer0_sourcebufferdimension0::LINECOUNT0_MASK, ht - 1)
            | field(r::pixeng_fetchlayer0_sourcebufferdimension0::LINEWIDTH0_MASK, wd - 1),
    );
    w(
        m,
        base,
        r::PIXENG_FETCHLAYER0_COLORCOMPONENTBITS0,
        field(r::pixeng_fetchlayer0_colorcomponentbits0::COMPONENTBITSRED0_MASK, 8)
            | field(r::pixeng_fetchlayer0_colorcomponentbits0::COMPONENTBITSGREEN0_MASK, 8)
            | field(r::pixeng_fetchlayer0_colorcomponentbits0::COMPONENTBITSBLUE0_MASK, 8),
    );
    w(
        m,
        base,
        r::PIXENG_FETCHLAYER0_COLORCOMPONENTSHIFT0,
        field(r::pixeng_fetchlayer0_colorcomponentshift0::COMPONENTSHIFTRED0_MASK, 16)
            | field(r::pixeng_fetchlayer0_colorcomponentshift0::COMPONENTSHIFTGREEN0_MASK, 8),
    );
    w(m, base, r::PIXENG_FETCHLAYER0_CONSTANTCOLOR0, r::pixeng_fetchlayer0_constantcolor0::CONSTANTALPHA0_MASK);
    // DPU_SetFetchUnitOffset, DPU_EnableFetchUnitSrcBuffer.
    w(m, base, r::PIXENG_FETCHLAYER0_LAYEROFFSET0, 0);
    modify(
        m,
        base,
        r::PIXENG_FETCHLAYER0_LAYERPROPERTY0,
        r::pixeng_fetchlayer0_layerproperty0::SOURCEBUFFERENABLE0_MASK,
        r::pixeng_fetchlayer0_layerproperty0::SOURCEBUFFERENABLE0_MASK,
    );
    // The content stream's and DomainBlend0's shadow loads
    // (DPU_TriggerPipelineShadowLoad, DPU_TriggerDisplayDbShadowLoad).
    w(m, base, r::PIXENG_EXTDST0CFG_EXTDST0_TRIGGER, r::pixeng_extdst0cfg_extdst0_trigger::SYNC_TRIGGER_MASK);
    w(m, base, r::DISENG_DOMAINBLEND0_CONTROLTRIGGER, r::diseng_domainblend0_controltrigger::SHDTOKGEN_MASK);
    // DPU_InitDisplayTiming(0). The side-by-side sync modes off and the
    // shadow token local (Linux sets both; they are the reset values the
    // permissive sources leave).
    modify(
        m,
        base,
        r::DISENG_FRAMEGEN0_FGSTCTRL,
        r::diseng_framegen0_fgstctrl::SHDEN_MASK
            | r::diseng_framegen0_fgstctrl::FGSYNCMODE_MASK
            | r::diseng_framegen0_fgstctrl::FGSHDTOKGENSYNCMODE_MASK,
        r::diseng_framegen0_fgstctrl::SHDEN_MASK,
    );
    w(m, base, r::DISENG_DITHER0CFG_POLARITYCTRL0, t.polarity);
    w(m, base, r::DISENG_FRAMEGEN0_HTCFG1, t.htcfg1);
    w(m, base, r::DISENG_FRAMEGEN0_HTCFG2, t.htcfg2);
    w(m, base, r::DISENG_FRAMEGEN0_VTCFG1, t.vtcfg1);
    w(m, base, r::DISENG_FRAMEGEN0_VTCFG2, t.vtcfg2);
    w(m, base, r::DISENG_FRAMEGEN0_PKICKCONFIG, t.kick);
    w(m, base, r::DISENG_FRAMEGEN0_SKICKCONFIG, t.kick);
    // DPU_SetDisplayConfig(0): the areas at (1, 1) as Zephyr passes them,
    // the primary input only, the default panic mode and constant colour.
    let start = field(r::diseng_framegen0_pacfg::PSTARTX_MASK, 1) | field(r::diseng_framegen0_pacfg::PSTARTY_MASK, 1);
    w(m, base, r::DISENG_FRAMEGEN0_PACFG, start);
    w(m, base, r::DISENG_FRAMEGEN0_SACFG, start);
    w(m, base, r::DISENG_FRAMEGEN0_FGINCTRL, FGDM_PRIM);
    w(m, base, r::DISENG_FRAMEGEN0_FGINCTRLPANIC, FGDM_TEST);
    w(
        m,
        base,
        r::DISENG_FRAMEGEN0_FGCCR,
        r::diseng_framegen0_fgccr::CCRED_MASK
            | r::diseng_framegen0_fgccr::CCGREEN_MASK
            | r::diseng_framegen0_fgccr::CCBLUE_MASK
            | field(r::diseng_framegen0_fgccr::CCALPHA_MASK, 1),
    );
    // DPU_TriggerDisplayShadowLoad(0).
    w(m, base, r::DISENG_FRAMEGEN0_FGSLR, r::diseng_framegen0_fgslr::SHDTOKGEN_MASK);
}

/// `DPU_StartDisplay(0)`.
pub fn start(m: &impl Mmio, base: usize) {
    w(m, base, r::DISENG_FRAMEGEN0_FGENABLE, r::diseng_framegen0_fgenable::FGEN_MASK);
}

/// `DPU_StopDisplay(0)`.
pub fn stop(m: &impl Mmio, base: usize) {
    w(m, base, r::DISENG_FRAMEGEN0_FGENABLE, 0);
}

/// FrameGen0's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FgStatus {
    pub fgenable: u32,
    pub fgensts: u32,
    pub fgchstat: u32,
}

impl FgStatus {
    pub const fn running(&self) -> bool {
        self.fgensts & r::diseng_framegen0_fgensts::ENSTS_MASK != 0
    }
    pub const fn in_sync(&self) -> bool {
        self.running() && self.fgchstat & r::diseng_framegen0_fgchstat::PRIMSYNCSTAT_MASK != 0
    }
}

pub fn fg_status(m: &impl Mmio, base: usize) -> FgStatus {
    FgStatus {
        fgenable: rd(m, base, r::DISENG_FRAMEGEN0_FGENABLE),
        fgensts: rd(m, base, r::DISENG_FRAMEGEN0_FGENSTS),
        fgchstat: rd(m, base, r::DISENG_FRAMEGEN0_FGCHSTAT),
    }
}

/// Wait up to `budget_us` until FrameGen0 runs with its primary channel in
/// sync (Linux polls `PrimSyncStat` up to 100 ms, a fact). Returns the µs
/// waited, or the last status on a timeout.
pub fn wait_sync(m: &impl Mmio, base: usize, budget_us: u32, delay_us: &mut impl FnMut(u32)) -> Result<u32, FgStatus> {
    let mut waited = 0;
    loop {
        let s = fg_status(m, base);
        if s.in_sync() {
            return Ok(waited);
        }
        if waited >= budget_us {
            return Err(s);
        }
        delay_us(POLL_STEP_US);
        waited += POLL_STEP_US;
    }
}

/// Wait up to `budget_us` until FrameGen0 reads stopped.
pub fn wait_stopped(
    m: &impl Mmio,
    base: usize,
    budget_us: u32,
    delay_us: &mut impl FnMut(u32),
) -> Result<u32, FgStatus> {
    let mut waited = 0;
    loop {
        let s = fg_status(m, base);
        if !s.running() {
            return Ok(waited);
        }
        if waited >= budget_us {
            return Err(s);
        }
        delay_us(POLL_STEP_US);
        waited += POLL_STEP_US;
    }
}

/// Put `addr` on screen at the next frame: the new address, then ExtDst0's
/// pipeline shadow load, which FrameGen0's kick carries out (`dpu_write`:
/// `DPU_SetFetchUnitSrcBufferAddr`, `DPU_TriggerPipelineShadowLoad`). The
/// shadow-load flag is cleared first so [`wait_flip`] sees this load.
pub fn flip(m: &impl Mmio, base: usize, addr: u64) {
    w(m, base, r::IRQ_INTERRUPTCLEAR0, IRQ_EXTDST0_SHDLD);
    set_address(m, base, addr);
    w(m, base, r::PIXENG_EXTDST0CFG_EXTDST0_TRIGGER, r::pixeng_extdst0cfg_extdst0_trigger::SYNC_TRIGGER_MASK);
}

/// Wait up to `budget_us` for ExtDst0's shadow load (Zephyr waits for the
/// same interrupt; here its status bit is polled), then clear it.
pub fn wait_flip(m: &impl Mmio, base: usize, budget_us: u32, delay_us: &mut impl FnMut(u32)) -> Result<u32, u32> {
    let mut waited = 0;
    loop {
        let st = rd(m, base, r::IRQ_INTERRUPTSTATUS0);
        if st & IRQ_EXTDST0_SHDLD != 0 {
            w(m, base, r::IRQ_INTERRUPTCLEAR0, IRQ_EXTDST0_SHDLD);
            return Ok(waited);
        }
        if waited >= budget_us {
            return Err(st);
        }
        delay_us(POLL_STEP_US);
        waited += POLL_STEP_US;
    }
}

/// A Signature0 window: corners in the active area, both inclusive (the
/// earlier DPU's `upperLeft`/`lowerRight`; whether the i.MX 95's lower right
/// is inclusive the board says, design note U7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

impl Window {
    /// The whole active area of `mode`.
    pub const fn full(mode: &VideoMode) -> Window {
        Window { x0: 0, y0: 0, x1: mode.hactive - 1, y1: mode.vactive - 1 }
    }
}

/// Start Signature0 measuring window 0 continuously: shadow loads per
/// window on request, no panic colour, continuous mode, the window's
/// corners, its evaluation on (no reference check), its shadow load
/// requested (`DPU_InitSignature`, `DPU_SetSignatureWindowConfig`,
/// `DPU_EnableSignatureWindowCompute`, `DPU_TriggerSignatureShadowLoad`).
pub fn sig_start(m: &impl Mmio, base: usize, win: &Window) {
    w(m, base, r::DISENG_SIG0_STATICCONTROL, r::diseng_sig0_staticcontrol::SHDEN_MASK);
    w(m, base, r::DISENG_SIG0_PANICCOLOR, 0);
    w(m, base, r::DISENG_SIG0_CONTINUOUSMODE, r::diseng_sig0_continuousmode::ENCONT_MASK);
    w(
        m,
        base,
        r::DISENG_SIG0_UPPERLEFT_WINDOW0,
        field(r::diseng_sig0_upperleft_window0::UPPERLEFT_X_WINDOW0_MASK, win.x0)
            | field(r::diseng_sig0_upperleft_window0::UPPERLEFT_Y_WINDOW0_MASK, win.y0),
    );
    w(
        m,
        base,
        r::DISENG_SIG0_LOWERRIGHT_WINDOW0,
        field(r::diseng_sig0_lowerright_window0::LOWERRIGHT_X_WINDOW0_MASK, win.x1)
            | field(r::diseng_sig0_lowerright_window0::LOWERRIGHT_Y_WINDOW0_MASK, win.y1),
    );
    w(m, base, r::DISENG_SIG0_CONTROL_WINDOW0, r::diseng_sig0_control_window0::EN_WINDOW0_MASK);
    w(m, base, r::DISENG_SIG0_SHADOWLOAD, 1);
}

/// Window 0's CRCs, red, green, blue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Crc {
    pub r: u32,
    pub g: u32,
    pub b: u32,
}

/// The CRCs once `STATUS.SigValid` says the measurement is valid.
pub fn sig_read(m: &impl Mmio, base: usize) -> Option<Crc> {
    let st = rd(m, base, r::DISENG_SIG0_STATUS);
    if st & r::diseng_sig0_status::SIGVALID_MASK == 0 {
        return None;
    }
    Some(Crc {
        r: rd(m, base, r::DISENG_SIG0_CRC_R_WINDOW0),
        g: rd(m, base, r::DISENG_SIG0_CRC_G_WINDOW0),
        b: rd(m, base, r::DISENG_SIG0_CRC_B_WINDOW0),
    })
}

/// What the DPU says it scans out (the note's "proof without a camera").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadBack {
    pub ipidentifier: u32,
    pub fg: FgStatus,
    /// `FGTIMESTAMP` twice, a frame apart.
    pub timestamps: [u32; 2],
    pub fetch_addr: u64,
    pub attributes: u32,
    pub dimension: u32,
    pub layerproperty: u32,
    pub htcfg1: u32,
    pub vtcfg1: u32,
    /// ExtDst0's pipeline status (`pipeline_status`, `sync_busy`).
    pub pipeline: u32,
}

impl ReadBack {
    /// The frame index of `FGTIMESTAMP` moved between the two reads.
    pub fn frames_advance(&self) -> bool {
        let f = |t| get(r::diseng_framegen0_fgtimestamp::FRAMEINDEX_MASK, t);
        f(self.timestamps[1]) != f(self.timestamps[0])
    }
}

/// Read the DPU's side of the proof; `frame_us` apart for the timestamps.
pub fn read_back(m: &impl Mmio, base: usize, frame_us: u32, delay_us: &mut impl FnMut(u32)) -> ReadBack {
    let t0 = rd(m, base, r::DISENG_FRAMEGEN0_FGTIMESTAMP);
    delay_us(frame_us);
    let t1 = rd(m, base, r::DISENG_FRAMEGEN0_FGTIMESTAMP);
    ReadBack {
        ipidentifier: rd(m, base, r::COMCTRL_IPIDENTIFIER),
        fg: fg_status(m, base),
        timestamps: [t0, t1],
        fetch_addr: rd(m, base, r::PIXENG_FETCHLAYER0_BASEADDRESS0) as u64
            | (rd(m, base, r::PIXENG_FETCHLAYER0_BASEADDRESSMSB0) as u64) << 32,
        attributes: rd(m, base, r::PIXENG_FETCHLAYER0_SOURCEBUFFERATTRIBUTES0),
        dimension: rd(m, base, r::PIXENG_FETCHLAYER0_SOURCEBUFFERDIMENSION0),
        layerproperty: rd(m, base, r::PIXENG_FETCHLAYER0_LAYERPROPERTY0),
        htcfg1: rd(m, base, r::DISENG_FRAMEGEN0_HTCFG1),
        vtcfg1: rd(m, base, r::DISENG_FRAMEGEN0_VTCFG1),
        pipeline: rd(m, base, r::PIXENG_EXTDST0CFG_EXTDST0_STATUS),
    }
}
