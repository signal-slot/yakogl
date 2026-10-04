// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Identify, reset and the register init after reset ("milestone 1" and the
//! non-MMU half of milestone 2).
//!
//! Sources (driver/docs/vivante-kernel-rust.md has the full table):
//! - registers and fields: rnndb ([`super::regs`]);
//! - identity: galcore `gc_hal_kernel_hardware.c` `_IdentifyHardwareByDatabase`
//!   reads CHIP_IDENTITY, MODEL, REV, PRODUCT_ID, DATE, ECO, CUSTOMER_ID, the
//!   feature word and minor feature words 0..6 as registers, and takes the
//!   part's capabilities and limits from its feature database
//!   (`hal/kernel/inc/gc_feature_database.h`, `gcQueryFeatureDB`). [`DB`] is
//!   that database's entry for this part;
//! - reset: galcore `_ResetGPU` (same file): clock gating off, pulse eater,
//!   FSCALE load, isolate, soft reset (through MMUv2_AHB_CONTROL on a part
//!   with the secure AHB), release, check idle, MMU off;
//! - init after reset: galcore `gckHARDWARE_InitializeHardware` and its
//!   helpers (module clock gating, AXI cache attributes, the AHB non-secure
//!   grant, the pulse eater's internal-DFS bit, the interrupt enable).
//!
//! Where galcore and the C that ran on the board differ in a value, the
//! board's value is kept and named here; the sequence is galcore's.

use super::regs::*;
use crate::env::Env;
use crate::errno::{EBUSY, ENODEV};
use crate::mmio::Mmio;
use crate::ylog;

use super::GcIdentity;

/// The part's facts from galcore's feature database, entry "gc7000UL_6200"
/// (gc_feature_database.h, linux-imx lf-6.12.y): chipID 0x7000, product
/// 0x70003, an informal entry (formalRelease 0) that `gcQueryFeatureDB`
/// matches on revision & 0xfff0 == 0x6200. Field names are galcore's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DbEntry {
    pub chip_id: u32,
    pub revision_mask: u32,
    pub revision: u32,
    pub product_id: u32,
    /// The one revision this driver's init has run on (the Verdin iMX8M
    /// Plus reports 0x6204); others are refused as "not in the database".
    pub board_revision: u32,
    pub temp_registers: u32,
    pub thread_count: u32,
    pub num_shader_cores: u32,
    pub instruction_count: u32,
    pub number_of_constants: u32,
    pub num_pixel_pipes: u32,
    pub streams: u32,
    pub varying_count: u32,
    pub vertex_output_buffer_size: u32,
    pub vertex_cache_size: u32,
    /// gcFEATURE_BIT_REG_MMU: MMU version 2.
    pub reg_mmu: bool,
    /// gcFEATURE_BIT_SECURITY and gcFEATURE_BIT_SECURITY_AHB.
    pub security: bool,
    pub security_ahb: bool,
}

pub const DB: DbEntry = DbEntry {
    chip_id: 0x7000,
    revision_mask: 0xfff0,
    revision: 0x6200,
    product_id: 0x70003,
    board_revision: 0x6204,
    temp_registers: 0x40,
    thread_count: 0x200,
    num_shader_cores: 0x2,
    instruction_count: 0x200,
    number_of_constants: 0x140,
    num_pixel_pipes: 0x1,
    streams: 0x8,
    varying_count: 0x10,
    vertex_output_buffer_size: 0x400,
    vertex_cache_size: 0x10,
    reg_mmu: true,
    security: true,
    security_ahb: true,
};

/// The interconnect's answer to a read nobody decodes on this SoC
/// (0xdeadde..). Measured on the board, 2026-09-22 (MMUv2_AHB_CONTROL reads
/// 0xdeadde.., driver/src/vivante/NOTES.md); the RM does not state it.
pub fn reg_absent(v: u32) -> bool {
    v & 0xffff_ff00 == 0xdead_de00
}

/// Read a register, the base being the GPU block's.
fn rd(m: &impl Mmio, regs: usize, off: u32) -> u32 {
    m.read32(regs + off as usize)
}

fn wr(m: &impl Mmio, regs: usize, off: u32, v: u32) {
    m.write32(regs + off as usize, v)
}

/// Front-end registers 0x65c..0x670 read twice: on this family the first
/// read after a change can be stale. Board practice of the C driver's stall
/// dumps (the dump is the only consumer); galcore reads them once.
pub fn rd_fe(m: &impl Mmio, regs: usize, off: u32) -> u32 {
    let _ = rd(m, regs, off);
    rd(m, regs, off)
}

/// Identify the GPU. Refuses (-ENODEV) a block that does not answer: model 0,
/// all ones, or the interconnect's absent pattern. A part that answers but
/// is not [`DB`]'s is identified from its registers alone (`from_table`
/// false) with a warning; nothing past the reset is expected to work on it.
pub fn identify(m: &impl Mmio, regs: usize, env: &impl Env) -> Result<GcIdentity, i32> {
    let mut id = GcIdentity::default();
    let identity = rd(m, regs, VIVS_HI_CHIP_IDENTITY);
    id.model = rd(m, regs, VIVS_HI_CHIP_MODEL);
    id.revision = rd(m, regs, VIVS_HI_CHIP_REV);
    id.date = rd(m, regs, VIVS_HI_CHIP_DATE);
    id.time = rd(m, regs, VIVS_HI_CHIP_TIME);
    id.customer_id = rd(m, regs, VIVS_HI_CHIP_CUSTOMER_ID);
    id.product_id = rd(m, regs, VIVS_HI_CHIP_PRODUCT_ID);
    id.eco_id = rd(m, regs, VIVS_HI_CHIP_ECO_ID);
    id.features = rd(m, regs, VIVS_HI_CHIP_FEATURE);
    id.minor_features[0] = rd(m, regs, VIVS_HI_CHIP_MINOR_FEATURE_0);
    if id.model == 0 || id.model == 0xffff_ffff || reg_absent(id.model) {
        ylog!(env, Err, "no GPU identity (model 0x{:08x}): domain or clock not up", id.model);
        return Err(-ENODEV);
    }
    // Minor feature words 1..5 (galcore reads them unconditionally; rnndb
    // has 1..5 as registers).
    for (i, off) in [
        VIVS_HI_CHIP_MINOR_FEATURE_1,
        VIVS_HI_CHIP_MINOR_FEATURE_2,
        VIVS_HI_CHIP_MINOR_FEATURE_3,
        VIVS_HI_CHIP_MINOR_FEATURE_4,
        VIVS_HI_CHIP_MINOR_FEATURE_5,
    ]
    .into_iter()
    .enumerate()
    {
        id.minor_features[i + 1] = rd(m, regs, off);
    }
    ylog!(
        env,
        Inf,
        "identity 0x{:08x}: GC{:x} rev 0x{:x} product 0x{:x} customer 0x{:x} eco {} (built {:08x} {:08x})",
        identity,
        id.model,
        id.revision,
        id.product_id,
        id.customer_id,
        id.eco_id,
        id.date,
        id.time
    );
    ylog!(
        env,
        Inf,
        "registers say: features 0x{:08x} minor0 0x{:08x} minor1 0x{:08x}",
        id.features,
        id.minor_features[0],
        id.minor_features[1]
    );

    let db = &DB;
    if id.model == db.chip_id
        && id.revision & db.revision_mask == db.revision
        && id.product_id == db.product_id
        && id.revision == db.board_revision
    {
        // The database's facts, folded into the feature words in rnndb's
        // layout so a reader of the words sees them too.
        if db.reg_mmu {
            id.minor_features[1] |= CHIP_MINOR_FEATURES1_MMU_VERSION;
        }
        if db.security {
            id.minor_features[7] |= CHIP_MINOR_FEATURES7_BIT_SECURITY;
        }
        if db.security_ahb {
            id.minor_features[10] |= CHIP_MINOR_FEATURES10_SECURITY_AHB;
        }
        id.stream_count = db.streams;
        id.register_max = db.temp_registers;
        id.thread_count = db.thread_count;
        id.shader_core_count = db.num_shader_cores;
        id.vertex_cache_size = db.vertex_cache_size;
        id.vertex_output_buffer_size = db.vertex_output_buffer_size;
        id.pixel_pipes = db.num_pixel_pipes;
        id.instruction_count = db.instruction_count;
        id.num_constants = db.number_of_constants;
        id.varyings_count = db.varying_count;
        id.from_table = true;
    } else {
        ylog!(
            env,
            Wrn,
            "GC{:x} rev 0x{:x} product 0x{:x} is not in the hardware database",
            id.model,
            id.revision,
            id.product_id
        );
    }
    id.secure_ahb = id.minor_features[7] & CHIP_MINOR_FEATURES7_BIT_SECURITY != 0
        && id.minor_features[10] & CHIP_MINOR_FEATURES10_SECURITY_AHB != 0;
    ylog!(
        env,
        Inf,
        "{}: MMU v{} secure AHB {}, {} shader cores, {} pixel pipe(s)",
        if id.from_table { "database" } else { "registers" },
        if id.minor_features[1] & CHIP_MINOR_FEATURES1_MMU_VERSION != 0 { 2 } else { 1 },
        if id.secure_ahb { "yes" } else { "no" },
        id.shader_core_count,
        id.pixel_pipes
    );
    Ok(id)
}

/// Reset attempts before giving up, and the wait between them (the C's;
/// galcore loops without a bound and waits 1 ms).
pub const RESET_ATTEMPTS: u32 = 100;
pub const RESET_RETRY_US: u32 = 10_000;
/// Soft reset held for this long. galcore: 1 ms (`gckOS_Delay(Os, 1)`);
/// the board has run 20 us since 2026-09-20.
pub const RESET_HOLD_US: u32 = 20;
/// The pulse eater galcore's `_ResetGPU` writes: 0x015B0880, i.e.
/// 0x01590880 with bit 17 (rnndb UNK17) set.
pub const PULSE_EATER_RESET: u32 = 0x015b_0880;
/// FSCALE 64 of 64: full speed (galcore writes 0x00070900, whose FSCALE
/// field is the same 64).
pub const FSCALE_FULL: u32 = 64;

/// Reset the GPU to idle. galcore `_ResetGPU`'s sequence:
/// 1. clock gating off, the pulse eater programmed and its DISABLE pulsed;
/// 2. FSCALE loaded (CMD_LOAD up then down);
/// 3. isolate; soft reset -- through MMUv2_AHB_CONTROL.RESET when the part
///    has the secure AHB (galcore: SECURITY_AHB in "secure in normal"
///    mode), CLOCK_CONTROL.SOFT_RESET otherwise; hold; release; de-isolate;
/// 4. idle: IDLE_STATE (every module but AXI_LP), CLOCK_CONTROL IDLE_3D
///    and IDLE_2D, and the MMU control's ENABLE clear (SEC_CONTROL on the
///    secure AHB, CONTROL otherwise), else retry;
/// 5. debug registers enabled.
///
/// Values that are the board's rather than galcore's: clock gating is
/// turned off in PM_POWER_CONTROLS (galcore writes 0 to PM_MODULE_CONTROLS),
/// the pulse eater is written twice not three times, CLOCK_CONTROL is
/// written with FSCALE alone (galcore also writes the read-only idle bits
/// and DISABLE_DEBUG_REGISTERS), every module must be idle (galcore checks
/// FE only), one clean pass suffices (galcore wants two) and the waits are
/// RESET_HOLD_US / RESET_RETRY_US. That is the sequence that has reset
/// this GPU on every board run.
pub fn reset(m: &impl Mmio, regs: usize, env: &impl Env, id: &GcIdentity) -> Result<(), i32> {
    let idle_mask = !VIVS_HI_IDLE_STATE_AXI_LP;
    let mmu_v2 = id.minor_features[1] & CHIP_MINOR_FEATURES1_MMU_VERSION != 0;
    for _attempt in 0..RESET_ATTEMPTS {
        // 1. clock gating off, pulse eater
        wr(m, regs, VIVS_PM_POWER_CONTROLS, 0);
        let _ = rd(m, regs, VIVS_PM_POWER_CONTROLS);
        let mut pe = PULSE_EATER_RESET;
        wr(m, regs, VIVS_PM_PULSE_EATER, pe);
        pe |= VIVS_PM_PULSE_EATER_DISABLE;
        wr(m, regs, VIVS_PM_PULSE_EATER, pe);
        let _ = rd(m, regs, VIVS_PM_PULSE_EATER);

        // 2. FSCALE
        let mut control = vivs_hi_clock_control_fscale_val(FSCALE_FULL);
        wr(m, regs, VIVS_HI_CLOCK_CONTROL, control | VIVS_HI_CLOCK_CONTROL_FSCALE_CMD_LOAD);
        wr(m, regs, VIVS_HI_CLOCK_CONTROL, control);

        // 3. isolate, soft reset, release, de-isolate
        control |= VIVS_HI_CLOCK_CONTROL_ISOLATE_GPU;
        wr(m, regs, VIVS_HI_CLOCK_CONTROL, control);
        if id.secure_ahb {
            wr(m, regs, VIVS_MMUv2_AHB_CONTROL, VIVS_MMUv2_AHB_CONTROL_RESET);
        } else {
            control |= VIVS_HI_CLOCK_CONTROL_SOFT_RESET;
            wr(m, regs, VIVS_HI_CLOCK_CONTROL, control);
        }
        env.busy_wait_us(RESET_HOLD_US);
        control &= !VIVS_HI_CLOCK_CONTROL_SOFT_RESET;
        wr(m, regs, VIVS_HI_CLOCK_CONTROL, control);
        control &= !VIVS_HI_CLOCK_CONTROL_ISOLATE_GPU;
        wr(m, regs, VIVS_HI_CLOCK_CONTROL, control);

        // 4. idle?
        let idle = rd(m, regs, VIVS_HI_IDLE_STATE);
        if idle & idle_mask != idle_mask {
            env.busy_wait_us(RESET_RETRY_US);
            continue;
        }
        control = rd(m, regs, VIVS_HI_CLOCK_CONTROL);
        if control & VIVS_HI_CLOCK_CONTROL_IDLE_3D == 0 || control & VIVS_HI_CLOCK_CONTROL_IDLE_2D == 0 {
            env.busy_wait_us(RESET_RETRY_US);
            continue;
        }
        if mmu_v2 {
            let mmu = rd(m, regs, if id.secure_ahb { VIVS_MMUv2_SEC_CONTROL } else { VIVS_MMUv2_CONTROL });
            if mmu & VIVS_MMUv2_CONTROL_ENABLE != 0 {
                env.busy_wait_us(RESET_RETRY_US);
                continue;
            }
        }
        // 5. debug registers on
        control &= !VIVS_HI_CLOCK_CONTROL_DISABLE_DEBUG_REGISTERS;
        wr(m, regs, VIVS_HI_CLOCK_CONTROL, control);
        ylog!(env, Inf, "reset to idle: CLOCK_CONTROL=0x{:08x} IDLE_STATE=0x{:08x}", control, idle);
        return Ok(());
    }
    let idle = rd(m, regs, VIVS_HI_IDLE_STATE);
    let control = rd(m, regs, VIVS_HI_CLOCK_CONTROL);
    let not = |b: bool| if b { "" } else { "not " };
    ylog!(
        env,
        Err,
        "GPU failed to reset: FE {}idle, 3D {}idle, 2D {}idle (IDLE=0x{:08x} CLK=0x{:08x})",
        not(idle & VIVS_HI_IDLE_STATE_FE != 0),
        not(control & VIVS_HI_CLOCK_CONTROL_IDLE_3D != 0),
        not(control & VIVS_HI_CLOCK_CONTROL_IDLE_2D != 0),
        idle,
        control
    );
    Err(-EBUSY)
}

/// A write whose posting must complete before what follows (read back).
fn wr_sync(m: &impl Mmio, regs: usize, off: u32, v: u32) {
    wr(m, regs, off, v);
    let _ = rd(m, regs, off);
}

/// The register init after reset. galcore `gckHARDWARE_InitializeHardware`
/// writes the same registers: PM_POWER_CONTROLS bit 0, PM_MODULE_CONTROLS
/// (RA_HZ and RA_EZ unconditionally), the AHB non-secure grant (0x2 on a
/// secure-AHB part), the pulse eater by read-modify-write, INTR_ENBL all
/// ones. Values that are the board's, not galcore's (driver/docs/
/// vivante-kernel-rust.md "Where galcore differs"): galcore ends with
/// module clock gating *off* on revision 0x6202 and sets five more
/// MODULE_CONTROLS bits (0x004300e0 in all), writes AXI_CONFIG only on older
/// parts, and does not set the pulse eater's INTERNAL_DFS without gcdDVFS
/// (it clears bit 16 and sets bit 17 instead). This is the init every board
/// run since 2026-09-20 has used.
pub fn init(m: &impl Mmio, regs: usize, env: &impl Env) {
    // Module-level clock gating on, the rasteriser's HZ and EZ gating
    // disabled.
    let ppc = rd(m, regs, VIVS_PM_POWER_CONTROLS) | VIVS_PM_POWER_CONTROLS_ENABLE_MODULE_CLOCK_GATING;
    let pmc = rd(m, regs, VIVS_PM_MODULE_CONTROLS)
        | VIVS_PM_MODULE_CONTROLS_DISABLE_MODULE_CLOCK_GATING_RA_HZ
        | VIVS_PM_MODULE_CONTROLS_DISABLE_MODULE_CLOCK_GATING_RA_EZ;
    wr_sync(m, regs, VIVS_PM_POWER_CONTROLS, ppc);
    wr_sync(m, regs, VIVS_PM_MODULE_CONTROLS, pmc);

    // AXI cache attributes: AWCACHE and ARCACHE 2 (cacheable, no allocate).
    wr(
        m,
        regs,
        VIVS_HI_AXI_CONFIG,
        (2 << VIVS_HI_AXI_CONFIG_AWCACHE__SHIFT) | (2 << VIVS_HI_AXI_CONFIG_ARCACHE__SHIFT),
    );

    // The secure AHB starts refusing non-secure accesses: grant them. A
    // read of the absent pattern is not folded back (measured, see
    // [`reg_absent`]).
    let mut ahb = rd(m, regs, VIVS_MMUv2_AHB_CONTROL);
    if reg_absent(ahb) {
        ylog!(
            env,
            Wrn,
            "AHB_CONTROL (0x{:03x}) reads 0x{:08x}: the interconnect's pattern for an address no slave decodes; writing the non-secure grant alone",
            VIVS_MMUv2_AHB_CONTROL,
            ahb
        );
        ahb = 0;
    }
    wr(m, regs, VIVS_MMUv2_AHB_CONTROL, ahb | VIVS_MMUv2_AHB_CONTROL_NONSEC_ACCESS);

    // Pulse eater: read-modify-write of the internal-DFS bit.
    let pe = rd(m, regs, VIVS_PM_PULSE_EATER) | VIVS_PM_PULSE_EATER_INTERNAL_DFS;
    wr_sync(m, regs, VIVS_PM_PULSE_EATER, pe);

    // Every event bit and both error bits; there is no per-event enable.
    wr(m, regs, VIVS_HI_INTR_ENBL, 0xffff_ffff);

    let axi = rd(m, regs, VIVS_HI_AXI_CONFIG);
    let ahb = rd(m, regs, VIVS_MMUv2_AHB_CONTROL);
    let intr = rd(m, regs, VIVS_HI_INTR_ENBL);
    ylog!(
        env,
        Inf,
        "hw init: AXI_CONFIG 0x{:08x}{} AHB_CONTROL 0x{:08x}{} INTR_ENBL 0x{:08x}",
        axi,
        if axi == 0 { " (write-only or absent)" } else { "" },
        ahb,
        if reg_absent(ahb) { " (absent: no slave at that address)" } else { "" },
        intr
    );
}

/// The busy modules of an IDLE_STATE word (a set bit means idle), as
/// " FE PE SH": the stall dump's decode.
pub fn busy_modules(idle: u32, out: &mut [u8; 32]) -> usize {
    const MODS: [(&[u8; 2], u32); 9] = [
        (b"FE", VIVS_HI_IDLE_STATE_FE),
        (b"DE", VIVS_HI_IDLE_STATE_DE),
        (b"PE", VIVS_HI_IDLE_STATE_PE),
        (b"SH", VIVS_HI_IDLE_STATE_SH),
        (b"PA", VIVS_HI_IDLE_STATE_PA),
        (b"SE", VIVS_HI_IDLE_STATE_SE),
        (b"RA", VIVS_HI_IDLE_STATE_RA),
        (b"TX", VIVS_HI_IDLE_STATE_TX),
        (b"MC", VIVS_HI_IDLE_STATE_MC),
    ];
    let mut n = 0;
    for (name, bit) in MODS {
        if idle & bit == 0 {
            out[n] = b' ';
            out[n + 1..n + 3].copy_from_slice(name);
            n += 3;
        }
    }
    n
}

/// Log which modules are busy.
pub fn log_idle_state(env: &impl Env, idle: u32) {
    let mut b = [0u8; 32];
    let n = busy_modules(idle, &mut b);
    if n == 0 {
        ylog!(env, Err, "idle: every module idle (0x{:08x})", idle);
    } else {
        let s = core::str::from_utf8(&b[..n]).unwrap_or("?");
        ylog!(env, Err, "idle: BUSY ->{} (0x{:08x}; a set bit means idle)", s, idle);
    }
}
