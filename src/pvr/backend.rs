// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR BXS-4-64 command backend for YakoGL.
//!
//! The boundary between the GL core (`gl::draw`) and the PowerVR command
//! generator: the core resolves the current GL context into one [`GlesDraw`]
//! and calls [`pvr_backend_draw`]; the backend records the draw into the
//! current frame's command streams (VDM/PPP/PDS in GPU memory, via the OSAL)
//! and [`pvr_backend_flush`] submits the frame as one GEOM+FRAG kick.
//!
//! One kick per frame (not per draw) keeps the on-chip depth buffer and the
//! colour tiles coherent across the draw calls of a frame. When a frame has
//! to be split (draw table / arena limits) or does not start with a clear,
//! the next kick loads the colour back through a full-screen texel-fetch
//! quad and the depth/stencil through the ISP Z load/store unit (a packed
//! D24S8 buffer per target, `GlesDraw::ds_persist`).
//!
//! What is recorded:
//!
//! ```text
//!   per draw   : vertex-fetch PDS (one DMA stream per attribute), fragment
//!                kick-USC PDS, coefficient (DOUTI) PDS for the FS varyings,
//!                common-store DMA PDS (uniforms / texture+sampler state),
//!                one full PPP state block, and VDM_STATE + INDEX_LIST blocks
//!                appended to the frame's VDM control stream;
//!   per frame  : the ISP background object (clear colour), the EOT pixel-event
//!                data (framebuffer address), the PBE surface words, and the
//!                trailing PPP terminate block + STREAM_TERMINATE.
//! ```
//!
//! `pvr_backend_flush()` hands the frame to the platform (OSAL submit/wait ->
//! the driver's render service, src/pvr_render.c) as one GEOM+FRAG kick.
//!
//! Everything the generators need is reused from the driver
//! (src/pvr_vdm_ppp_gen.c, src/pvr_pds_render_gen.c,
//! src/pvr_pds_pixel_event_gen.c, see [`super::ffi`]); constants below that
//! are not derived are the DDK-captured "oracle" values the self-test render
//! uses (src/pvr_fw_boot.c pvr_render_ta3d), each noted where used.
//!
//! Memory: two bump arenas (PDS heap, GENERAL heap) hold the per-frame
//! programs, PPP blocks and descriptors and are reset after every flush; the
//! VDM stream lives in its own buffer. Persistent objects (clear FS, background
//! object programs, EOT data, sequential index scratch) are allocated at init.
//! All GPU memory is allocated through the OSAL; the backend never calls an
//! OS API directly.

// The arithmetic deliberately keeps the C code's shape (the output is compared
// bit for bit with the C backend): no clamp() (NaN behaves differently), no
// saturating_sub rewrites, the C nesting of conditions (div_ceil replaces the
// C's `(x + n - 1) / n` only where it gives the same value without the
// overflow).
//
// Floating point: no expression here is written with f32::mul_add, and rustc
// never contracts `a * b + c` into a fused multiply-add. That matches the C
// library as the silicon-verified Zephyr builds compiled it: Zephyr's C
// standard is `-std=c17` (Kconfig STD_C17, CONFIG_GNU_C_EXTENSIONS off; the
// yakogl and driver modules add no dialect flag), and GCC in an ISO dialect
// defaults to -ffp-contract=off. Checked with the Zephyr SDK 1.0.1 compiler
// (aarch64-zephyr-elf-gcc 14.3.0, -O2 and -Os) on the kick's clear-depth word
// `(uint32_t)(dz * 16777215.0f + 0.5f)`: -std=c17 emits fmul + fadd, while
// -std=gnu11 / gnu17 emit fmadd (so do clang at its default -ffp-contract=on
// and the Linux kernel-module build of the pvrlab harness, now retired, which
// used the kernel's -std=gnu11). The two C builds therefore differ by 1 LSB for some
// depths; the Rust port targets Zephyr and bare metal (baremetal.mk is
// -std=c11 too) and keeps the unfused result.
#![allow(clippy::manual_clamp, clippy::implicit_saturating_sub, clippy::collapsible_if)]

use core::ffi::{c_char, c_int, c_void};
use core::mem::size_of;

use super::csbgen as csb;
use super::ffi::*;
use super::internal::*;
use crate::backend::*;
use crate::ds_retention::{self, Parked, Plane, DEPTH, STENCIL};
use crate::gl::context::{one_wide_level_pitch, pot_level_side, CAP_MIP_LEVELS};
use crate::osal::render::*;
use crate::osal::{self, OsalGpuMem, OSAL_MEM_DATA, OSAL_QUERY_RENDER_STATIC};
use crate::shaderc::{self, ShcShader, SHC_FS_META_DEFAULT, SHC_NO_META, SHC_NO_PSIZE, SHC_UNIFORM_NO_BLOCK};
use crate::util::{self, Bytes, Global, EBUSY, EINVAL, EIO, ENODEV, ENOMEM, ENOSPC, ETIMEDOUT};

/* ---- the backend options (enum pvr_backend_option; defaults: all 0) -------
 *
 * Three are product options: ONE_IN_FLIGHT, FS_TEMPS_MIN and IF_LOWERING.
 * The ones documented as a test knob take effect only with the `test-knobs`
 * feature (`opt_is_test_knob`): without it the backend reads them as 0 and
 * setting one changes nothing. A RETIRED slot keeps its number so every
 * later option keeps its own (ffi_layout_check.c pins them); setting one
 * changes nothing. */
pub type PvrBackendOption = u32;
/// This backend draws into a framebuffer with no colour attachment
/// (`backend::DEPTH_ONLY_TARGETS`): a draw with `target_gpu` 0 and a depth or
/// stencil attachment opens a frame whose kick is a z-only render
/// ([`flush`]: one end-of-tile emit with an empty-tile PBE state that
/// stores nothing, and `CR_ISP_ZLSCTL.zonlyrender`), as Mesa's pvr renders
/// a pass with no colour attachment. The depth/stencil buffer is stored as
/// for any frame.
pub const DRAWS_WITHOUT_COLOUR: bool = true;

/// A test knob. 1: vertex positions are already in screen space (x,y pixels,
/// z, w=1) and the TA must not apply the viewport transform (PPP
/// pretransform). Used to reproduce the silicon test rig through the GL path
/// (gl_test_run's `pretransform`).
pub const PVR_BACKEND_OPT_PRETRANSFORM: PvrBackendOption = 0;
/// A test knob (gl_test 62 clipz sweeps it). The TA clip mode: [`PVR_DEPTH_CLIP_FRONT_REAR`],
/// [`PVR_DEPTH_CLIP_FRONT_REAR_DEPTH`] or [`PVR_DEPTH_CLIP_OFF`]. Unset, and
/// anything else, is FRONT_REAR -- near/far clipping as GL ES 2.0 2.11 asks
/// for it.
///
/// It was off for a while, as a known conformance gap, because FRONT_REAR
/// clipped the wrong volume: dEQP's clip_z cases came back as a sliver of the
/// reference, the shape you get when the hardware clips `0 <= z <= w` and GL
/// wants `-w <= z <= w`. That turned out not to be the clip mode's doing at
/// all but [`ppp_ctrl`]'s `opengl` bit, which selects the volume, and which
/// we inherited clear from Mesa -- a Vulkan driver, and Vulkan clips
/// `0 <= z <= w`. With bit 0 set the TA clips GL's volume, so FRONT_REAR is
/// simply right and is the default.
///
/// The 1 and 2 values are the ones `z_probe` and `gl_test` case 62 already
/// pass, and they still mean what they meant. What changed is that 0 now
/// selects the default rather than "off", and switching clipping off for
/// bring-up takes [`PVR_DEPTH_CLIP_OFF`].
pub const PVR_BACKEND_OPT_DEPTH_CLIP: PvrBackendOption = 1;
/// [`PVR_BACKEND_OPT_DEPTH_CLIP`]: near and far clipping, GL's volume.
pub const PVR_DEPTH_CLIP_FRONT_REAR: u32 = 1;
/// [`PVR_BACKEND_OPT_DEPTH_CLIP`]: the mode that clips against depth. It
/// clips nothing on its own (measured by `z_probe`: not even a quad beyond
/// the far plane), so it is a bring-up value, not an alternative.
pub const PVR_DEPTH_CLIP_FRONT_REAR_DEPTH: u32 = 2;
/// [`PVR_BACKEND_OPT_DEPTH_CLIP`]: no near/far clipping. A conformance gap --
/// geometry beyond the far plane is drawn where GL would cut it -- kept for
/// bracketing a suspected clipping problem during bring-up.
pub const PVR_DEPTH_CLIP_OFF: u32 = 3;
/// A test knob (gl_test 62 clipz sweeps it). CR_PPP_CTRL value override (0 =
/// the default for the current mode).
pub const PVR_BACKEND_OPT_PPP_CTRL: PvrBackendOption = 2;
/// A test knob. 1: never merge adjacent attributes into one DMA stream (one DDMAD per
/// attribute); 0: merge attributes that are contiguous in memory and in their
/// vi registers (fewer DMAs, the layout the test rig used).
pub const PVR_BACKEND_OPT_NO_COALESCE: PvrBackendOption = 3;
/// A test knob. 1: log every generated PPP/VDM/PDS word (bring-up comparison).
pub const PVR_BACKEND_OPT_DEBUG: PvrBackendOption = 4;
/// RETIRED: was TERM0, a raw TA_STATE_TERMINATE0 override for the trailing
/// PPP block, a bisection knob nothing set.
pub const PVR_BACKEND_OPT_RETIRED_TERM0: PvrBackendOption = 5;
/// RETIRED: was NO_SUBMIT, "build the frame but do not submit it", the A/B
/// hand-off to the self-test rig, which went with pvrgpu.ko
/// (driver/tools/pvrlab/README.md). The slot keeps its number so every later
/// option keeps its own (ffi_layout_check.c pins them); setting it changes
/// nothing.
pub const PVR_BACKEND_OPT_RETIRED_NO_SUBMIT: PvrBackendOption = 6;
/// A test knob. 1: after every kick, log a summary of the target's stored depth/stencil
/// buffer (bring-up aid for the ISP Z load/store path).
pub const PVR_BACKEND_OPT_DS_DUMP: PvrBackendOption = 7;
/// A test knob. Primitives one kick may tile before the frame is split (0 = the built-in
/// budget). A test can set it low to exercise the split on a workload small
/// enough for any parameter buffer.
pub const PVR_BACKEND_OPT_PRIM_BUDGET: PvrBackendOption = 8;
/// RETIRED: was MRT_STRIDE, a bring-up override of CR_USC_PIXEL_OUTPUT_CTRL's
/// width (gl_test set it to its default, 0).
pub const PVR_BACKEND_OPT_RETIRED_MRT_STRIDE: PvrBackendOption = 9;
/// RETIRED: was FORCE_MIP0, every texture sampled from level 0, an A/B
/// nothing set.
pub const PVR_BACKEND_OPT_RETIRED_FORCE_MIP0: PvrBackendOption = 10;
/// RETIRED: was DBIAS_INT. ISP_CTL.dbias_is_int is always set in a frame
/// with a depth/stencil buffer (see `isp_ctl`'s caller in `flush`), measured
/// on the AM62P (RESULTS 1i).
pub const PVR_BACKEND_OPT_RETIRED_DBIAS_INT: PvrBackendOption = 11;
/// A test knob, PROBE ONLY: 1 = colour target 0 is written TWIDDLE_2D instead of linear
/// (PBESTATE_REG_WORD0.memlayout = 1 at [32:33]; REG_WORD1 size_x [60:63] and
/// size_y [28:31] = log2 of a power-of-two target; pbestate.xml:53-58, 137-153,
/// 246-281; Mesa pvr_arch_job_common.c:140-157, 280-290). The native-cube
/// probe's second half: whether the PBE can write what a TEXTYPE_CUBE reads.
/// Anything that samples or reads back the target afterwards sees twiddled
/// bytes; only the probe turns it on.
pub const PVR_BACKEND_OPT_PBE_TWIDDLE: PvrBackendOption = 12;
/// MEASUREMENT: 1 = one kick in flight. The swap's wait
/// (`pvr_backend_finish_previous`) waits for the most recent kick too, not
/// only for the ones before it, so every kick starts on an idle GPU
/// (docs/notes/pvr-fence-wait.md, "The next experiment"). It
/// changes only that wait's target; glFinish, the frame-set reuse and every
/// kick are untouched. Off by default (0: wait for all but the most recent
/// kick).
pub const PVR_BACKEND_OPT_ONE_IN_FLIGHT: PvrBackendOption = 13;
/// RETIRED: was SWAP_DROPS_ZS. The swap always drops the window's
/// depth/stencil ([`pvr_backend_surface_swapped`]).
pub const PVR_BACKEND_OPT_RETIRED_SWAP_DROPS_ZS: PvrBackendOption = 14;
/// MEASUREMENT: the fragment kick declares at least this many temporaries
/// per instance (`max(fs.temps, value)`, capped at [`FS_TEMPS_MIN_CAP`]); 0
/// (the default) declares the shader's own count. The program is unchanged:
/// only the per-instance allocation the PDS kick asks the USC for grows, so
/// a run with it set separates the cost of a larger temporary allocation
/// (fewer resident instances) from the cost of the instructions. Never a
/// default: it exists for the online-vs-table A/B of the FemtoVG programs
/// (the online ones declare about twice the table's temporaries), and stays
/// a product option until the shader-compiler rename.
pub const PVR_BACKEND_OPT_FS_TEMPS_MIN: PvrBackendOption = 15;
/// MEASUREMENT: how the online compiler lowers an `if` whose arms are
/// straight-line arithmetic ([`crate::shaderc::IfLowering`]):
/// [`PVR_IF_LOWERING_DEFAULT`] (0, the compiler chooses by the arms'
/// sizes, `select::branch_pays`), [`PVR_IF_LOWERING_PREDICATE`] or [`PVR_IF_LOWERING_BRANCH`]
/// (pco's `lower_if_cond_exec`, the arms under the execution mask with a
/// branch past each). Read when a shader is compiled from its source; the
/// cache keys a shader by it, and its blend and sampler-state variants keep
/// it, so the same source compiles both ways on one context (gl_test 195).
/// Any other value is the default. The table path ignores it.
pub const PVR_BACKEND_OPT_IF_LOWERING: PvrBackendOption = 16;
/// [`PVR_BACKEND_OPT_IF_LOWERING`]'s values.
pub const PVR_IF_LOWERING_DEFAULT: u32 = 0;
pub const PVR_IF_LOWERING_PREDICATE: u32 = 1;
pub const PVR_IF_LOWERING_BRANCH: u32 = 2;
pub const PVR_BACKEND_OPT_COUNT: usize = 17;

/// Is `o` a test knob: an option that takes effect only with the
/// `test-knobs` feature?
const fn opt_is_test_knob(o: PvrBackendOption) -> bool {
    matches!(
        o,
        PVR_BACKEND_OPT_PRETRANSFORM
            | PVR_BACKEND_OPT_DEPTH_CLIP
            | PVR_BACKEND_OPT_PPP_CTRL
            | PVR_BACKEND_OPT_NO_COALESCE
            | PVR_BACKEND_OPT_DEBUG
            | PVR_BACKEND_OPT_DS_DUMP
            | PVR_BACKEND_OPT_PRIM_BUDGET
            | PVR_BACKEND_OPT_PBE_TWIDDLE
    )
}

/// Is `o` a retired slot, which nothing reads?
const fn opt_is_retired(o: PvrBackendOption) -> bool {
    matches!(
        o,
        PVR_BACKEND_OPT_RETIRED_TERM0
            | PVR_BACKEND_OPT_RETIRED_NO_SUBMIT
            | PVR_BACKEND_OPT_RETIRED_MRT_STRIDE
            | PVR_BACKEND_OPT_RETIRED_FORCE_MIP0
            | PVR_BACKEND_OPT_RETIRED_DBIAS_INT
            | PVR_BACKEND_OPT_RETIRED_SWAP_DROPS_ZS
    )
}
/// The largest count [`PVR_BACKEND_OPT_FS_TEMPS_MIN`] declares: the most a
/// DOUTU word can declare.
pub const FS_TEMPS_MIN_CAP: u32 = PVR_PDS_DOUTU_MAX_TEMPS;

/* ---- this GPU's device-VA heap configuration (pvr_rogue_heap_config.h;
 * mirrors the driver's src/pvr_internal.h). The OSAL bump-allocates inside
 * these. ---- */
const HEAP_GENERAL_BASE: u64 = 0x80_0000_0000;
const HEAP_PDS_BASE: u64 = 0xDA_0000_0000;
const HEAP_USC_BASE: u64 = 0xE0_0000_0000;
const HEAP_WINDOW: u64 = 0x4000_0000;

/* ---- sizes ---- */
/* Sized for real UI frames: a Slint dashboard is ~130 draws in one kick (each
 * draw needs its PPP/vpds/uniform-snapshot bytes here). The old 64K/256K sizes
 * forced a frame split at ~57 draws, and continuation kicks currently fail to
 * reload the previous kick's colour (an open multi-kick TODO) so earlier draws
 * were lost -- keep complex frames in one kick until that is fixed. */
/// How many kicks may be recorded ahead of the GPU (one set of per-kick
/// resources each, about 5 MiB): the most a build can ask for, and the
/// default ([`pvr_backend_set_frame_arenas`] may ask for fewer).
const PVR_BACKEND_FRAME_SETS: usize = 4;
/// Colour attachments one kick may write; the driver's end-of-tile programs
/// cover this many (`PVR_RENDER_MAX_RT`).
const PVR_BACKEND_MAX_RT: u32 = PVR_RENDER_MAX_RT as u32;
/// Primitives one kick may tile. The parameter buffer the firmware fills while
/// tiling is finite (the driver grows it out of the GPU carveout, tens of
/// MiB); a Quick 3D scene like simplefog draws 2000 instances of a sphere,
/// about ten million triangles, in ONE call and outgrows any pool this board
/// can spare. Past this many primitives the frame is split: the kick so far is
/// rendered and the next one loads its colour and depth back, which is the
/// same thing a partial render does, decided by the host. ~16 bytes of
/// parameter buffer per primitive measured on this silicon, so 1.5M keeps a
/// kick near 24 MiB.
const PRIMS_PER_KICK_MAX: u32 = 1_500_000;
/// `true`: DMA only the prefix of a uniform block the compiled shader reads
/// (`ShcShader::uniform_sh_dwords`). Off: the count the compiler reports is
/// not a bound on what the shader reads -- Qt Quick 3D's post-processing
/// vertex shader reports 64 dwords of a 100-dword block and reads its
/// transform from dword 64 on, and the truncated block left the transform
/// zero, so the pass drew nothing (sceneeffects rendered a black view).
const PVR_BACKEND_UNIFORM_PREFIX: bool = false;
/// The default arenas, measured
/// (docs/notes/pvr-render-pool.md, the frame-set run):
/// glmark2's largest frame on the AM62P used 325896 bytes of PDS and 516768 of
/// GENERAL. 768 KiB and 1024 KiB are 2.4 and 2.0 times that, so its frames are
/// not split. With 4 sets that is 7488 KiB of the render pool, where
/// 4 x (4 MiB + 1 MiB) took 20800 KiB.
const ARENA_PDS_BYTES: u32 = 768 * 1024;
const ARENA_GEN_BYTES: u32 = 1024 * 1024;
/// The largest arenas [`pvr_backend_set_frame_arenas`] accepts: the sizes
/// the backend built before the measurement (a Slint dashboard's ~130 draws
/// in one kick was the reason, above).
const ARENA_PDS_MAX: u32 = 1024 * 1024;
const ARENA_GEN_MAX: u32 = 4096 * 1024;

/// The frame sets a backend init builds ([`pvr_backend_set_frame_arenas`]):
/// how many, and each one's GENERAL and PDS arena bytes. The defaults are
/// the constants above; a smaller configuration takes less of the render
/// pool (docs/notes/pvr-render-pool.md, the frame sets are
/// 20.3 MiB of the fixed use).
#[derive(Clone, Copy)]
struct ArenaCfg {
    sets: usize,
    gen_bytes: u32,
    pds_bytes: u32,
}

static ARENA_CFG: Global<ArenaCfg> =
    Global::new(ArenaCfg { sets: PVR_BACKEND_FRAME_SETS, gen_bytes: ARENA_GEN_BYTES, pds_bytes: ARENA_PDS_BYTES });

/// Why a frame had no room for a draw ([`EFRAME_FULL`]), named in the
/// refusal line of a draw that does not fit an empty frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FullCause {
    None,
    GenArena,
    PdsArena,
    VdmStream,
    DbiasTable,
    ScissorTable,
}

impl FullCause {
    fn name(self) -> &'static str {
        match self {
            FullCause::None => "an unnamed limit",
            FullCause::GenArena => "the general arena",
            FullCause::PdsArena => "the pds arena",
            FullCause::VdmStream => "the vdm stream",
            FullCause::DbiasTable => "the depth-bias table",
            FullCause::ScissorTable => "the scissor table",
        }
    }
}

/// The cause of the last [`EFRAME_FULL`] (single-threaded by contract).
static FULL_CAUSE: Global<FullCause> = Global::new(FullCause::None);

fn frame_full(cause: FullCause) -> i32 {
    // SAFETY: single-threaded by contract; no other reference is live.
    unsafe { *FULL_CAUSE.get() = cause };
    -EFRAME_FULL
}
const VDM_STREAM_BYTES: u32 = 64 * 1024;
/// PDS data/code segment alignment (self-test pds_put).
const PDS_ALIGN: u32 = 128;
const PPP_ALIGN: u32 = 16;
const MAX_DRAWS_PER_FRAME: u32 = 1024;
/// Slots of [`Backend::pds_seen`], direct-mapped by [`pds_hash`].
const PDS_SEEN_SLOTS: usize = 32;

/// A PDS program (or one segment of one) the open frame's PDS arena already
/// holds: at `off` in the arena, `bytes` long, its words hashing to `hash`
/// ([`pds_put`]). `bytes` 0: an empty slot.
#[derive(Clone, Copy)]
struct PdsSeen {
    off: u32,
    bytes: u32,
    hash: u32,
}

impl PdsSeen {
    const ZERO: PdsSeen = PdsSeen { off: 0, bytes: 0, hash: 0 };
}

/// Entries of each of [`Backend::kick_once`] and [`Backend::coeff_once`]:
/// the (program, variant, sample rate) combinations one frame draws with
/// before the oldest is generated again.
const PDS_ONCE_SLOTS: usize = 16;

/// Off: every draw generates its kick-USC and coefficient programs
/// ([`pvr_backend_set_pds_once`]). A test switch, not a `PVR_BACKEND_OPT_*`.
static PDS_ONCE_OFF: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// A fragment kick-USC PDS program the open frame holds, by everything
/// `pvr_pds_render_kick_usc_program` reads: the FS code's heap offset, its
/// temps and the sample rate. `end` (arena-relative, past its words) 0: an
/// empty slot.
#[derive(Clone, Copy)]
struct KickOnce {
    usc_off: u32,
    temps: u32,
    rate: u32,
    /// The program's PDS heap offset (what the draw points at).
    off: u32,
    end: u32,
}

impl KickOnce {
    const ZERO: KickOnce = KickOnce { usc_off: 0, temps: 0, rate: 0, off: 0, end: 0 };
}

/// A coefficient (DOUTI) PDS program the open frame holds, by everything
/// `pvr_pds_render_coeff_program` reads: the W/Z/point-coord flags and the
/// varyings as iterated (`s.cv`). `end` as in [`KickOnce`]; a program
/// without code takes no bytes and keeps `end` 0 with `used` set.
#[derive(Clone, Copy)]
struct CoeffOnce {
    used: bool,
    flags: u32,
    nv: u32,
    cv: [PvrPdsCoeffVarying; PVR_PDS_COEFF_MAX_VARYINGS as usize],
    info: PvrPdsCoeffInfo,
    off: u32,
    end: u32,
}

impl CoeffOnce {
    const ZERO: CoeffOnce = CoeffOnce {
        used: false,
        flags: 0,
        nv: 0,
        cv: [PvrPdsCoeffVarying::ZERO; PVR_PDS_COEFF_MAX_VARYINGS as usize],
        info: PvrPdsCoeffInfo {
            data_size_dw: 0,
            code_size_dw: 0,
            coeff_dwords: 0,
            pds_varyingsize: 0,
            usc_varyingsize: 0,
        },
        off: 0,
        end: 0,
    };

    fn holds(&self, flags: u32, cv: &[PvrPdsCoeffVarying]) -> bool {
        self.used
            && self.flags == flags
            && self.nv as usize == cv.len()
            && self.cv[..cv.len()]
                .iter()
                .zip(cv)
                .all(|(a, b)| a.components == b.components && a.interp == b.interp && a.vtxout_off == b.vtxout_off)
    }
}
/* worst case one draw records: vertex/kick/coeff/DMA/VS-uniform PDS programs
 * (each PDS_ALIGN-padded), PPP block + terminate + descriptors + index copy,
 * and the VDM blocks (PDS state 3 + PPP 2 + VDM 5 + 2 index lists 6 + 3) */
const PDS_PER_DRAW_MAX: u32 = 8 * 1024;
/// Two 1536-dword uniform snapshots + staged attributes.
const GEN_PER_DRAW_MAX: u32 = 16 * 1024;
const VDM_PER_DRAW_MAX: u32 = 24;
/// How long a kick may take before the wait gives up. A heavy Quick 3D frame
/// on this GPU -- screen-space reflections, a runtime-loaded model, an IBL
/// material at 1024x600 -- takes well over a second, and a few take several:
/// at 3 s the wait returned while the GPU was still working, the frame set was
/// reused underneath it and the picture never arrived. A hang is reported late
/// rather than a slow frame reported as a hang.
const WAIT_MS: u32 = 20000;
/// Size of the per-set static PDS page.
const PDS_STATIC_BYTES: u32 = 4096;

/* ---- Mesa's csbgen packers (src/pvr/csbgen.rs) ------------------------- */
/// Packs one csbgen structure into its `$n` dwords: Mesa's `pvr_csb_pack`
/// (src/imagination/vulkan/pvr_csb.h), over the generated `ROGUE_*_pack()`
/// (driver/src/pvr/generated/csbgen/rogue/, build.rs compiles the wrappers).
macro_rules! csb_pack {
    ($pack:ident, $n:literal, $v:expr) => {{
        let mut dw = [0u32; $n];
        let v = $v;
        // SAFETY: the packer reads one initialised structure and writes its
        // `_length` dwords, which is `$n` (the bindings carry the lengths of
        // the generated header; csb_pack_lengths pins them).
        unsafe { csb::$pack(dw.as_mut_ptr().cast(), &v) };
        dw
    }};
}

/// Two packed dwords as the 64-bit register value the render kick carries.
const fn dw64(dw: [u32; 2]) -> u64 {
    dw[0] as u64 | (dw[1] as u64) << 32
}

/// A device address for an address field.
const fn dev_addr(addr: u64) -> csb::pvr_dev_addr_t {
    csb::pvr_dev_addr_t { addr }
}

/* ---- the fragment registers: Mesa's values for this core ----------------- */
/// Output registers a pixel has: four, this core having no
/// `eight_output_registers` feature (DDK rgxconfig_km_36.V.104.796.h, Mesa
/// `rogue_get_max_output_regs_per_pixel`).
const PVR_BACKEND_MAX_OUT_REGS: u32 = 4;
/// The BXS-4-64's partition count (Mesa common/device_info/bxs-4-64.h
/// `max_partitions`).
const BXS_MAX_PARTITIONS: u32 = 16;

/// CR_USC_PIXEL_OUTPUT_CTRL as Mesa's `pvr_arch_setup_tiles_in_flight`
/// (vulkan/pvr_arch_job_common.c) packs it for a pixel of `width`
/// (ROGUE_CR_PIXEL_WIDTH): this core has `usc_pixel_partition_mask`, so the
/// partition mask is every partition, `(1 << max_partitions) - 1`, and
/// `enable_4th_partition` stays clear. With the four output registers a
/// pixel has here (4REGISTERS) that is 0x0007_fff9, the value the DDK
/// capture had.
fn usc_pixel_output_ctrl(width: u32) -> u32 {
    csb_pack!(
        ROGUE_CR_USC_PIXEL_OUTPUT_CTRL_pack,
        1,
        csb::ROGUE_CR_USC_PIXEL_OUTPUT_CTRL {
            partition_mask: (1 << BXS_MAX_PARTITIONS) - 1,
            enable_4th_partition: false,
            width,
        }
    )[0]
}

/// The USC's reserved partition store, in dwords: what Mesa reads as
/// `pvr_device_runtime_info.total_reserved_partition_size` (the kernel's
/// answer; the DDK firmware here reserves 8192 dwords). Not a Mesa value.
const RESERVED_PARTITION_DWORDS: u32 = 8192;

/// The output registers a pixel has under CR_USC_PIXEL_OUTPUT_CTRL.width
/// (ROGUE_CR_PIXEL_WIDTH: 0: 2, 1: 4, 2: 8, 3: 1).
fn pixel_width_regs(width: u32) -> u32 {
    match width & 3 {
        csb::ROGUE_CR_PIXEL_WIDTH_2REGISTERS => 2,
        csb::ROGUE_CR_PIXEL_WIDTH_4REGISTERS => 4,
        csb::ROGUE_CR_PIXEL_WIDTH_8REGISTERS => 8,
        _ => 1,
    }
}

/// Tiles in flight for a frame of `samples` whose pixels have `pixel_regs`
/// output registers (the width CR_USC_PIXEL_OUTPUT_CTRL says), as Mesa's
/// `pvr_arch_setup_tiles_in_flight` (vulkan/pvr_arch_job_common.c, Mesa
/// 26.2.1, MIT) computes it for the BXS-4-64 (common/device_info/bxs-4-64.h:
/// isp_samples_per_pixel 4, 16x16 tiles, max_partitions 16,
/// isp_max_tiles_in_flight 6, usc_min_output_registers_per_pix 2, one raster
/// pipe, simple_parameter_format_version 2, paired tiles not requested). A
/// tile takes a USC partition of `pixel_regs` x samples x 256 dwords, and
/// the ISP may hold no more tiles than the reserved partition store fits.
/// Mesa derives this and the output control's width from the same pixel
/// width; so does this. At the four registers this backend programs that is
/// six tiles at one sample (PIPE(5), the DDK capture's value) and two at 4x.
///
/// It used to take the pixel at two registers whatever the width said, which
/// let the ISP have four 4x tiles in flight against a store that holds two
/// of the partitions the USC then allocates. gl_test 198's 4x picture --
/// 1024 alpha-to-coverage triangles over 64x64 pixels, 64 objects a 16x16
/// tile -- locked the fragment DM up on the AM62P with no USC slot in use
/// and no page fault (firmware: DM3 timed out, phase count 0; HWR type
/// UNKNOWNFAILURE), twice, both pictures lost (records/am62p/
/// 20261010-1515-c196-198-452d6c31-e85a4597). dEQP's same picture over
/// 256x256 (four objects a tile) passed with the same words, as
/// gl_test 16's one 4x triangle does: the over-commit needs objects enough
/// a tile. Nothing else in the 4x kick contradicts another of its words or
/// Mesa's sizing (HWRT, ZLS extents); a board run of case 198 is the
/// confirmation that this was the lockup.
fn isp_tiles_in_flight(samples: u32, pixel_regs: u32) -> u32 {
    const USC_MIN_OUTPUT_REGS: u32 = 2; /* usc_min_output_registers_per_pix */
    const TILE_DWORDS: u32 = 16 * 16; /* tile_size_x * tile_size_y */
    const ISP_MAX_TILES_IN_FLIGHT: u32 = 6;
    let width = pixel_regs.max(USC_MIN_OUTPUT_REGS).next_power_of_two();
    let partition = width * samples.clamp(1, 4) * TILE_DWORDS;
    let usable = RESERVED_PARTITION_DWORDS.min(partition * BXS_MAX_PARTITIONS);
    let partitions_available = BXS_MAX_PARTITIONS.min(usable / partition);
    /* one phantom (num_raster_pipes): the USC's tiles are the partitions,
     * the ISP's at most isp_max_tiles_in_flight of them (version 2 caps the
     * total by the partitions again, which changes nothing here) */
    ISP_MAX_TILES_IN_FLIGHT.min(partitions_available).max(1)
}

/// CR_ISP_CTL as Mesa's `pvr_render_job_ws_fragment_state_init`
/// (vulkan/pvr_arch_job_render.c) packs it: `sample_pos`, `process_empty_tiles`
/// (every render here wants its empty tiles stored), `skip_init_hdrs` from
/// `gpu_multicore_support` (bxs-4-64.h has it; Mesa clears it again under
/// quirks 72168/72463 -- both set for 36.53.104.796 -- when the render is at
/// most two tiles wide and its tile-group row count is odd; that is NOT
/// applied here, so a target 32 px wide or less keeps the bit where Mesa
/// clears it: a divergence from Mesa kept to keep today's bytes, an open
/// question), `dbias_is_int` for an integer ZLS format
/// on a core with ERN 42307, and `pipe_enable` = tiles in flight - 1
/// ([`isp_tiles_in_flight`] for the frame's samples and pixel width).
/// 0x8003_5000 at one sample, 0x8003_1000 at 4x, with four registers a pixel.
fn isp_ctl(samples: u32, pixel_regs: u32, dbias_is_int: bool) -> u32 {
    csb_pack!(
        ROGUE_CR_ISP_CTL_pack,
        1,
        csb::ROGUE_CR_ISP_CTL {
            skip_init_hdrs: true,
            process_empty_tiles: true,
            sample_pos: true,
            dbias_is_int,
            pipe_enable: isp_tiles_in_flight(samples, pixel_regs) - 1,
            ..Default::default()
        }
    )[0]
}

/// CR_ISP_AA.mode for a frame of `samples`.
fn isp_aa(samples: u32) -> u32 {
    let mode = match samples {
        4 => csb::ROGUE_CR_ISP_AA_MODE_TYPE_AA_4X,
        2 => csb::ROGUE_CR_ISP_AA_MODE_TYPE_AA_2X,
        _ => csb::ROGUE_CR_ISP_AA_MODE_TYPE_AA_NONE,
    };
    csb_pack!(ROGUE_CR_ISP_AA_pack, 1, csb::ROGUE_CR_ISP_AA { mode })[0]
}

/// CR_ISP_BGOBJVALS as Mesa packs it (pvr_arch_job_render.c): `mask` always
/// set, the clear stencil. `enablebgtag` stays clear: enabling it locked the
/// TA on a split frame, and loads use a quad.
fn isp_bgobjvals(stencil: u8) -> u32 {
    csb_pack!(
        ROGUE_CR_ISP_BGOBJVALS_pack,
        1,
        csb::ROGUE_CR_ISP_BGOBJVALS { enablebgtag: false, mask: true, stencil: stencil as u32 }
    )[0]
}
/// PDS DOUTU sample rate (pvr_rogue_pds_defs.h): the fragment program runs
/// once per pixel (INSTANCE) or once per covered sample (FULL).
const PDS_DOUTU_SAMPLE_RATE_INSTANCE: u32 = 0;
const PDS_DOUTU_SAMPLE_RATE_FULL: u32 = 2;
/// The rate a draw's fragment program runs at in a frame of `samples`.
///
/// At INSTANCE rate on a multisampled target the program runs once for the
/// pixel and its output goes to every sample the primitive covers. That is
/// right for a program that only writes, but a program that reads the pixel
/// (blending, lowered into the shader, or a colour mask keeping channels)
/// reads ONE sample's colour and writes the combination to all the covered
/// ones: along an edge the samples another primitive owns take this one's
/// blend of that other colour, which is the 4x config's blend failures and
/// the "overlapping fragments" of rasterization.fill_rules (additive
/// blending over a shared edge). Mesa's pvr runs every fragment program at
/// FULL rate when the rasterization has more than one sample
/// (pvr_arch_cmd_buffer.c setup_pds_fragment_program); here only the
/// programs that read the pixel do, which keeps the plain draws at one
/// invocation per pixel.
///
/// A program with the kill epilogue (`uses_discard`: its own discards, or
/// compiled for the sample state) runs per sample too: the epilogue kills
/// the sample `SR_SAMP_NUM` names when the sample mask, alpha to coverage
/// or the ISP's coverage leave it out, and once per pixel that is sample 0
/// deciding for the whole pixel.
fn fs_sample_rate(samples: u32, per_sample: bool) -> u32 {
    if samples > 1 && per_sample {
        PDS_DOUTU_SAMPLE_RATE_FULL
    } else {
        PDS_DOUTU_SAMPLE_RATE_INSTANCE
    }
}

/// pco's `PVR_FS_META_SAMPLE_MASK` field of the control word
/// (`src/imagination/common/pvr_iface.h`): bits 9..24.
const FS_META_SAMPLE_MASK_SHIFT: u32 = 9;
const FS_META_SAMPLE_MASK_BITS: u32 = 0xffff << FS_META_SAMPLE_MASK_SHIFT;
/// `PVR_FS_META_ALPHA_TO_COVERAGE`: bit 25.
const FS_META_ALPHA_TO_COVERAGE: u32 = 1 << 25;

/// The per-draw control word a program with the kill epilogue reads: the
/// default (every sample, no alpha to coverage, no alpha to one) with the
/// draw's sample mask and alpha-to-coverage bit on a multisampled frame, as
/// Mesa's pvr fills it (pvr_arch_cmd_buffer.c: `ms.sample_mask <<
/// PVR_FS_META_SAMPLE_MASK_OFFSET`, the alpha-to-coverage bit). A frame of
/// one sample keeps the default: GL ignores both there (ES 2.0 4.1.3).
fn fs_meta_word(samples: u32, d: &GlesDraw) -> u32 {
    if samples <= 1 {
        return SHC_FS_META_DEFAULT;
    }
    let full = (1u32 << samples.min(16)) - 1;
    let mask = u32::from(d.sample_mask) & full;
    (SHC_FS_META_DEFAULT & !FS_META_SAMPLE_MASK_BITS & !FS_META_ALPHA_TO_COVERAGE)
        | mask << FS_META_SAMPLE_MASK_SHIFT
        | if d.alpha_to_coverage { FS_META_ALPHA_TO_COVERAGE } else { 0 }
}
/* ISP Z load/store: depth/stencil kept in memory across kicks. The buffer is
 * packed D24S8 (cr.xml ZLS_FORMAT_TYPE 24BITINT, 4 bytes/pixel), laid out in
 * whole ZLS tiles (Mesa aligns the image to the ZLS tile size when it has
 * no zls_subtile feature, as BXS-4-64). The background depth is then a 24-bit
 * unorm (Mesa pvr_arch_job_render.c ISP_BGOBJDEPTH for 24BITINT). */
/// The tile the ISP's Z load/store works in: the 16x16 screen tile doubled,
/// because this core has `simple_internal_parameter_format` version 2
/// (`rogue_get_zls_tile_size_xy`). A depth/stencil buffer aligned to 16 is
/// short of what the ISP then writes, and the store runs off the end of it.
const ZLS_TILE: u32 = 32;

/// CR_ISP_ZLSCTL for a packed D24S8 buffer of `tiles_x` x `tiles_y` ISP
/// tiles (the extents are tiles - 1, ten bits each): both halves stored
/// every kick (`forcezstore`), loaded when the buffer holds a previous
/// kick's values (`forcezload` with either), 24BITINT both ways (Mesa
/// pvr_arch_job_render.c, `pvr_frag_state_stream_init` for this format).
#[cfg(test)] // the csbgen test's name for the storing word; the backend calls isp_zlsctl_stores
fn isp_zlsctl(tiles_x: u32, tiles_y: u32, load_depth: bool, load_stencil: bool) -> u64 {
    isp_zlsctl_stores(tiles_x, tiles_y, load_depth, load_stencil, true)
}

/// [`isp_zlsctl`], with the store halves (`zstoreen`, `sstoreen`,
/// `forcezstore`) on or off: off for a frame whose depth/stencil nothing
/// will read ([`pvr_backend_surface_swapped`]).
fn isp_zlsctl_stores(tiles_x: u32, tiles_y: u32, load_depth: bool, load_stencil: bool, store: bool) -> u64 {
    dw64(csb_pack!(
        ROGUE_CR_ISP_ZLSCTL_pack,
        2,
        csb::ROGUE_CR_ISP_ZLSCTL {
            zlsextent_x_z: (tiles_x - 1) & 0x3ff,
            zlsextent_y_z: (tiles_y - 1) & 0x3ff,
            zloadformat: csb::ROGUE_CR_ZLS_FORMAT_TYPE_24BITINT,
            zstoreformat: csb::ROGUE_CR_ZLS_FORMAT_TYPE_24BITINT,
            zstoreen: store,
            sstoreen: store,
            forcezstore: store,
            zloaden: load_depth,
            sloaden: load_stencil,
            forcezload: load_depth || load_stencil,
            ..Default::default()
        }
    ))
}

/// `CR_ISP_ZLSCTL.zonlyrender`, ORed into [`isp_zlsctl_stores`]'s word for
/// a frame with no colour target. Mesa's pvr sets it for a render with no
/// end-of-tile surface, no fragment side effects and no depth feedback
/// (pvr_arch_cmd_buffer.c:1687-1689, packed at pvr_arch_job_render.c:1005,
/// Mesa 26.2.1, MIT). An ES 2.0 fragment shader has no side effects, and
/// sampling the depth texture being rendered into is a feedback loop GL
/// leaves undefined.
fn isp_zlsctl_zonlyrender() -> u64 {
    dw64(csb_pack!(ROGUE_CR_ISP_ZLSCTL_pack, 2, csb::ROGUE_CR_ISP_ZLSCTL { zonlyrender: true, ..Default::default() }))
}

/// RGXFWIF_CMD3D depth-buffer flag (driver/src/pvr_drv.h
/// PVR_FRAG_FLAGS_DEPTHBUFFER): value from DDK capture; to be derived per
/// Mesa (step 3). Not derivable from Mesa: Mesa submits through the
/// mainline DRM interface (drm_pvr_submit_job, whose fragment flags are
/// the kernel's), and RGXFWIF_CMD3D is a structure of the DDK's user-mode
/// driver, which neither Mesa nor the vendored DDK kernel source defines.
const FRAG_FLAGS_DEPTHBUFFER: u32 = 0x0000_0080;
/// RGXFWIF_CMD3D stencil-buffer flag (PVR_FRAG_FLAGS_STENCILBUFFER): value
/// from DDK capture; to be derived per Mesa (step 3). Not derivable from
/// Mesa, as FRAG_FLAGS_DEPTHBUFFER.
const FRAG_FLAGS_STENCILBUFFER: u32 = 0x0000_0100;
/// RGXFWIF_CMD3D "this render has visibility results" flag: the firmware
/// collects the ISP's visibility-test counts. Mesa's pvrsrvkm firmware
/// interface ROGUE_FWIF_RENDERFLAGS_GETVISRESULTS (pvr_rogue_fwif.h:53-56;
/// set by pvr_arch_srv_job_render.c:750-751 for a job with get_vis_results),
/// in the flag set whose DEPTHBUFFER (0x80) and STENCILBUFFER (0x100) are the
/// two values above the DDK capture gave.
const FRAG_FLAGS_GETVISRESULTS: u32 = 0x0000_0020;
/// Visibility result areas: one per kick in flight that counts, each
/// GLES_QUERY_HANDLES dwords (ISPCTL.visreg = the query handle), 16-byte
/// aligned as ISP_OCLQRY_BASE wants (ROGUE_OQUERY_ALIGN).
const VIS_SLOTS: usize = 8;
const VIS_SLOT_BYTES: usize = GLES_QUERY_HANDLES as usize * 4;
/// Depth/stencil buffers kept at once ([`ds_retention::DS_SLOTS`], shared by
/// every backend): a window surface and every framebuffer object with a
/// depth or stencil attachment wants one. A buffer that holds contents is
/// never given to another target: past this many, or when GPU memory runs
/// out first, a draw is refused by name (`ds_make_room`, `ds_get`).
const PVR_BACKEND_MAX_DS: usize = ds_retention::DS_SLOTS;
/* Depth bias (glPolygonOffset): ISP_DBIAS_BASE points at a table of
 * {constant, slope, clamp} float triples that STATE_ISPDBSC indexes per
 * object. With the 24-bit ZLS format the ISP adds the constant as-is
 * (ISP_CTL.dbias_is_int, ERN 42307 on this GPU), so GL's r * units becomes
 * units / 2^23 (Mesa pvr_calculate_final_depth_bias_contant_factor); with the
 * float on-chip format the hardware scales the constant by 2^(e-23) itself. */
const DBIAS_MAX_ENTRIES: u32 = 64;
/* ISP scissor table (CR_ISP_SCISSOR_BASE): two dwords per entry, pixel
 * rectangles (IPF_SCISSOR_WORD_0/1: min [31:16], max [15:0], max exclusive).
 * The region clip alone is tile granular (16 px). */
const SCISSOR_MAX_ENTRIES: u32 = 256;
/// Draws whose per-draw dump PVR_BACKEND_OPT_DEBUG pays for from when it is
/// switched on (upstream a3b7374).
/// (a Quick 3D scene spends its first forty kicks turning an .hdr into a cube
/// map and its mip chain, so 48 ran out before the scene itself was drawn)
const DBG_DRAW_BUDGET: u32 = 400;
/* Bring-up log budgets (upstream's function-local statics), spent once per
 * backend lifetime: what the first kicks of a run were and what each of their
 * draws ran.
 *
 * The per-kick and per-draw ones default to nothing, where upstream spends 60
 * and 120. They are written to a serial console, and on the AM62P that
 * console IS the frame time for as long as the budget lasts: with 2 draws per
 * frame, 120 draws is the first 60 frames, each of them ~76 ms against a
 * steady-state 5.7 ms. Measured 2026-09-22 with gl_test's panel cube --
 * record_draw 3 994 411 us over the first 200 frames with the budgets, 1 803
 * us without, and the slow frames 63 of 199 against 3. It is the first thing
 * anyone sees an application do, so it is off unless asked for. The dump
 * option (PVR_BACKEND_OPT_DEBUG) still turns the per-draw line on. */
const MRT_LOG_BUDGET: u32 = 8;
const KICK_LOG_BUDGET: u32 = 0;
const DRAW_LOG_BUDGET: u32 = 0;
/// ... and what the per-draw dump raises that budget to, for the same reason
/// [`DBG_DRAW_BUDGET`] is large.
const DRAW_LOG_BUDGET_DEBUG: u32 = 400;
/// CR_TE_PSG as Mesa packs it (pvr_arch_job_render.c,
/// `pvr_geom_state_stream_init`): `completeonterminate` (every geometry
/// phase here ends its render), `region_stride` 0 (one layer, no region
/// headers per view), `forcenewstate` only with quirk 52942, which this core
/// does not have. 0x0002_0000.
fn te_psg() -> u32 {
    csb_pack!(
        ROGUE_CR_TE_PSG_pack,
        1,
        csb::ROGUE_CR_TE_PSG { completeonterminate: true, region_stride: 0, ..Default::default() }
    )[0]
}

/// CR_PPP_CTRL. Mesa (pvr_arch_job_render.c, `pvr_geom_state_stream_init`)
/// sets `wclampen` and `fixed_point_format`; the clip-space word below adds
/// `opengl`. With `pretransform` (PVR_BACKEND_OPT_PRETRANSFORM, the DDK's
/// pretransformed float vertices) the word is `opengl | wclampen`, 0x3:
/// value from DDK capture, a field choice Mesa has no counterpart for (it
/// never draws pretransformed vertices).
///
/// Without it: opengl | wclampen | fixed_point_format (HW viewport), 0x203.
///
/// `fixed_point_format` (bit 9) is Mesa's, and has to be: our VS emits Mesa's
/// clip-space coordinate format and the PSG reads the position word according
/// to this register. `opengl` (bit 0) is the DDK's, and selects which volume
/// the TA clips near and far against:
///
///   bit 0 clear   `0 <= z <= w`, the Vulkan/D3D volume. Mesa is a **Vulkan**
///                 driver, so its 0x202 leaves it clear, and taking that
///                 value for its coordinate format brought the convention
///                 along with it. GL geometry between the eye and the near
///                 plane was clipped away.
///   bit 0 set     `-w <= z <= w`, which is GL's.
///
/// Measured on silicon with `z_probe` (pvrgpu-platform, task #43): at 0x202
/// with FRONT_REAR a quad at NDC z -0.5 is clipped; at 0x203 it is drawn and
/// one beyond the far plane is still clipped. See
/// `driver/docs/gpu-recipes/depth-clip-convention.md`.
fn ppp_ctrl(pretransform: bool) -> u32 {
    csb_pack!(
        ROGUE_CR_PPP_CTRL_pack,
        1,
        csb::ROGUE_CR_PPP_CTRL {
            opengl: true,
            wclampen: true,
            fixed_point_format: if pretransform { 0 } else { 1 },
            ..Default::default()
        }
    )[0]
}

/// PBESTATE_REG_WORD0's channel swizzle (chan0..3) for a target of this
/// surface format: memory R,G,B,A, or B,G,R,A for XRGB8888 scan-out and for
/// RGB565 (the R5G6B5 pack mode puts channel 0 in the low bits, and an
/// RGB565 target keeps red high, DRM order); with the alpha lane SWIZ.ONE
/// when the GL format behind it has no alpha.
///
/// SWIZ.ONE: the PBE writes 1 there, whatever the shader produced.
///
/// A colour target whose GL format has no alpha (GL_RGB, GL_RGB565) still has
/// an alpha byte in memory, because the storage is RGBA8 whatever the format
/// asked for; GL says such a buffer reads 1 (ES 2.0 4.3.1). Doing it in the
/// PBE costs nothing and covers every writer at once -- the background object,
/// the clear quad and any draw -- where a write mask would have needed a
/// fragment shader variant per shader (glColorMask is lowered into the shader
/// on this GPU) and would have left the alpha already in memory alone.
fn pbe_swizzle(fmt: u8, alpha_one: bool) -> [csb::ROGUE_PBESTATE_SWIZ; 4] {
    let mut swiz = if fmt == GLES_SURF_XRGB8888 || fmt == GLES_SURF_RGB565 {
        [
            csb::ROGUE_PBESTATE_SWIZ_SOURCE_CHAN2,
            csb::ROGUE_PBESTATE_SWIZ_SOURCE_CHAN1,
            csb::ROGUE_PBESTATE_SWIZ_SOURCE_CHAN0,
            csb::ROGUE_PBESTATE_SWIZ_SOURCE_CHAN3,
        ]
    } else {
        [
            csb::ROGUE_PBESTATE_SWIZ_SOURCE_CHAN0,
            csb::ROGUE_PBESTATE_SWIZ_SOURCE_CHAN1,
            csb::ROGUE_PBESTATE_SWIZ_SOURCE_CHAN2,
            csb::ROGUE_PBESTATE_SWIZ_SOURCE_CHAN3,
        ]
    };
    /* an integer surface stores the channels it has, alpha included */
    if alpha_one && !gles_surf_is_integer(fmt) {
        swiz[3] = csb::ROGUE_PBESTATE_SWIZ_ONE;
    }
    swiz
}

/// The PBE register words (CR_PBE_WORD0/1 of an attachment) for a `tw` x
/// `th` target of format `fmt` whose rows are `pitch` pixels apart
/// (`tex_pitch(tw)`, or wider for a window that is a view into a wider
/// scan-out buffer): PBESTATE_REG_WORD0 -- tile relative, `downscale`
/// (resolve the samples on emit) on a multisampled frame, linestride = the
/// row pitch in pixel pairs minus one whatever the pixel size (gl_test case
/// 21 checks RGB565 rows), the channel order -- and PBESTATE_REG_WORD1,
/// maxclip_x/y. `twiddle` (PVR_BACKEND_OPT_PBE_TWIDDLE, power-of-two
/// targets only) stores the target twiddled, with its log2 size. An sRGB
/// target sets `gamma` (REG_WORD0 bit 4, pbestate.xml:268): the PBE encodes
/// red, green and blue on store, as Mesa's pvr_arch_pbe_pack_state does for
/// PVR_PBE_GAMMA_ENABLED (pvr_arch_job_common.c:163-171; `twocomp_gamma`
/// only for a two-component format, which this is not).
fn pbe_reg_words(fmt: u8, alpha_one: bool, tw: u32, th: u32, pitch: u32, downscale: bool, twiddle: bool) -> [u64; 2] {
    let [swiz_chan0, swiz_chan1, swiz_chan2, swiz_chan3] = pbe_swizzle(fmt, alpha_one);
    let w0 = csb_pack!(
        ROGUE_PBESTATE_REG_WORD0_pack,
        2,
        csb::ROGUE_PBESTATE_REG_WORD0 {
            tilerelative: true,
            downscale,
            linestride: pitch.wrapping_sub(1) / 2,
            memlayout: if twiddle {
                csb::ROGUE_PBESTATE_MEMLAYOUT_TWIDDLE_2D
            } else {
                csb::ROGUE_PBESTATE_MEMLAYOUT_LINEAR
            },
            swiz_chan0,
            swiz_chan1,
            swiz_chan2,
            swiz_chan3,
            gamma: gles_surf_is_srgb(fmt),
            ..Default::default()
        }
    );
    let w1 = csb_pack!(
        ROGUE_PBESTATE_REG_WORD1_pack,
        2,
        csb::ROGUE_PBESTATE_REG_WORD1 {
            maxclip_x: tw.wrapping_sub(1),
            maxclip_y: th.wrapping_sub(1),
            size_x: if twiddle { tw.trailing_zeros() } else { 0 },
            size_y: if twiddle { th.trailing_zeros() } else { 0 },
            ..Default::default()
        }
    );
    [dw64(w0), dw64(w1)]
}

/// The PBESTATE_STATE_WORD0/1 pair attachment `mrt_index` hands the
/// end-of-tile program: the surface address, which 32-bit output register
/// its source starts at (`source_pos`) and how it is packed. An 8-bit target
/// is 8 per channel and normalised (U8U8U8U8, or R5G6B5); a float target's
/// registers hold halves or floats (F16_PER_CHANNEL), with its pack mode
/// F16F16F16F16 / F16 / F32 and no normalisation, and the fragment shader is
/// the variant compiled for that class. An integer target is 8 per channel
/// (Mesa pvr_arch_pbe_get_src_format_and_gamma, pvr_arch_job_common.c:51-53:
/// any integer format), not normalised, in its integer pack mode
/// ([`pbe_int_packmode`]), its registers the dwords the program packed.
/// PBESTATE_STATE_WORD0/1 for a render with no colour attachment: word 0
/// zero, word 1 only `emptytile`, so the PBE writes nothing. Transcribed from
/// Mesa's pvr (pvr_setup_emit_state, pvr_arch_cmd_buffer.c:1508-1515, and
/// pvr_arch_job_common.c:102-111, Mesa 26.2.1, MIT).
fn pbe_emit_words_z_only() -> [u32; 2] {
    let w1 = csb_pack!(
        ROGUE_PBESTATE_STATE_WORD1_pack,
        1,
        csb::ROGUE_PBESTATE_STATE_WORD1 { emptytile: true, ..Default::default() }
    );
    [0, w1[0]]
}

/// The PBE pack mode of an integer surface: Mesa 26.2.1's
/// pvr_pbe_format_table (vulkan/pvr_arch_formats.c:195-266) for the pipe
/// format of the layout -- R8_UINT U8 :201, R8_SINT S8 :202, R8G8_UINT U8U8
/// :206, R8G8_SINT S8S8 :207, R8G8B8A8_UINT/SINT U8U8U8U8/S8S8S8S8 :211-212,
/// R16_UINT/SINT U16/S16 :230-231, R16G16_* U16U16/S16S16 :235-236,
/// R16G16B16A16_* U16U16U16U16/S16S16S16S16 :243-244, R32_* U32/S32
/// :246-247, R32G32_* U32U32/S32S32 :249-250, R32G32B32A32_*
/// U32U32U32U32/S32S32S32S32 :255-256, and R10G10B10A2_UINT one packed U32
/// dword :227 (the PBE has no 10/10/10/2 integer pack; the program packs it).
fn pbe_int_packmode(l: GlesIntLayout) -> csb::ROGUE_PBESTATE_PACKMODE {
    use csb::*;
    match (l.channels, l.bits, l.signed) {
        (1, 8, false) => ROGUE_PBESTATE_PACKMODE_U8,
        (1, 8, true) => ROGUE_PBESTATE_PACKMODE_S8,
        (1, 16, false) => ROGUE_PBESTATE_PACKMODE_U16,
        (1, 16, true) => ROGUE_PBESTATE_PACKMODE_S16,
        (1, 32, false) => ROGUE_PBESTATE_PACKMODE_U32,
        (1, 32, true) => ROGUE_PBESTATE_PACKMODE_S32,
        (2, 8, false) => ROGUE_PBESTATE_PACKMODE_U8U8,
        (2, 8, true) => ROGUE_PBESTATE_PACKMODE_S8S8,
        (2, 16, false) => ROGUE_PBESTATE_PACKMODE_U16U16,
        (2, 16, true) => ROGUE_PBESTATE_PACKMODE_S16S16,
        (2, 32, false) => ROGUE_PBESTATE_PACKMODE_U32U32,
        (2, 32, true) => ROGUE_PBESTATE_PACKMODE_S32S32,
        (4, 8, false) => ROGUE_PBESTATE_PACKMODE_U8U8U8U8,
        (4, 8, true) => ROGUE_PBESTATE_PACKMODE_S8S8S8S8,
        (4, 16, false) => ROGUE_PBESTATE_PACKMODE_U16U16U16U16,
        (4, 16, true) => ROGUE_PBESTATE_PACKMODE_S16S16S16S16,
        (4, 32, true) => ROGUE_PBESTATE_PACKMODE_S32S32S32S32,
        (4, 32, false) => ROGUE_PBESTATE_PACKMODE_U32U32U32U32,
        /* RGB10_A2UI */
        _ => ROGUE_PBESTATE_PACKMODE_U32,
    }
}

/// The 32-bit pixel output registers a surface's colour takes: the packed
/// dwords of an integer surface ([`ShcIntRt::dwords`], what its program
/// writes), two for RGBA16F's four halves and for an sRGB target's (its
/// linear F16 accumulation), one for every other.
fn surf_out_dwords(fmt: u8) -> u32 {
    match gles_surf_int_layout(fmt) {
        Some(l) => crate::shaderc::ShcIntRt { channels: l.channels, bits: l.bits, signed: l.signed }.dwords(),
        None if fmt == GLES_SURF_RGBA16F || gles_surf_is_srgb(fmt) => 2,
        None => 1,
    }
}

/// The class of the pixel a surface's colour is accumulated in, for the
/// back end's own programs (the frame load, the clears): the shader class
/// ([`gles_surf_rt_class`]), with an sRGB target's linear halves taken as
/// RGBA16F's -- the load's TPU decode and the PBE's encode on store are what
/// differ, not the program.
fn pixel_class(fmt: u8) -> u32 {
    match gles_surf_rt_class(fmt) {
        crate::shaderc::SHC_RT_SRGB => crate::shaderc::SHC_RT_F16X4,
        c => c,
    }
}

fn pbe_emit_words(addr: u64, fmt: u8, source_pos: u32, mrt_index: u32) -> [u32; 2] {
    let float = gles_surf_is_float(fmt);
    let int = gles_surf_int_layout(fmt);
    /* An sRGB target: F16 source (pvr_arch_pbe_get_src_format_and_gamma,
     * pvr_arch_job_common.c:56-60), normalised, packed U8U8U8U8 (Mesa's
     * FORMAT(R8G8B8A8_SRGB, U8U8U8U8, F16), pvr_arch_formats.c:213); the
     * encode is REG_WORD0's gamma ([`pbe_reg_words`]). */
    let srgb = gles_surf_is_srgb(fmt);
    let packmode = match (fmt, int) {
        (_, Some(l)) => pbe_int_packmode(l),
        (GLES_SURF_RGBA16F, _) => csb::ROGUE_PBESTATE_PACKMODE_F16F16F16F16,
        (GLES_SURF_R16F, _) => csb::ROGUE_PBESTATE_PACKMODE_F16,
        _ if float => csb::ROGUE_PBESTATE_PACKMODE_F32,
        (GLES_SURF_RGB565, _) => csb::ROGUE_PBESTATE_PACKMODE_R5G6B5,
        _ => csb::ROGUE_PBESTATE_PACKMODE_U8U8U8U8,
    };
    let w0 =
        csb_pack!(ROGUE_PBESTATE_STATE_WORD0_pack, 1, csb::ROGUE_PBESTATE_STATE_WORD0 { address_low: dev_addr(addr) });
    let w1 = csb_pack!(
        ROGUE_PBESTATE_STATE_WORD1_pack,
        1,
        csb::ROGUE_PBESTATE_STATE_WORD1 {
            address_high: dev_addr(addr),
            source_format: if float || srgb {
                csb::ROGUE_PBESTATE_SOURCE_FORMAT_F16_PER_CHANNEL
            } else {
                csb::ROGUE_PBESTATE_SOURCE_FORMAT_8_PER_CHANNEL
            },
            /* "every channel normalized" (Mesa pvr_vk_format_is_fully_normalized,
             * pvr_formats.h:264-273): neither a float nor an integer one is */
            norm: !float && int.is_none(),
            packmode,
            source_pos,
            mrt_index: mrt_index & 7,
            ..Default::default()
        }
    );
    [w0[0], w1[0]]
}
/// Mesa _PVR_CLEAR_ATTACH_DATA_COUNT.
const CLEAR_ATTACH_DWORDS: u32 = 6;
/// TA_REGION_CLIP unit: the 16x16 tile (PVR_SCISSOR: clip [0,0] = top-left
/// 16x16 px; Mesa pvr_get_geom_region_clip_align_size).
const REGION_CLIP_PIXELS: u32 = 16;
/// The ISP's screen tile (ROGUE_IPF_TILE_SIZE_PIXELS). The ZLS tile is twice
/// it on this core (see `ZLS_TILE`), and CR_ISP_ZLSCTL's extents are counted
/// in THIS one.
const ISP_TILE: u32 = 16;

/// `n` rounded up to a multiple of `a` (a power of two).
const fn align_up_pow2(n: u32, a: u32) -> u32 {
    (n + a - 1) & !(a - 1)
}

/* Texture state (docs/gpu-recipes/mesa_texture_layout.md, silicon-verified),
 * packed with Mesa's TEXSTATE packers (csbgen/rogue/texstate.xml). */
/// One texel format's TEXSTATE_IMAGE_WORD0 fields: the format, its swizzle
/// (swiz0..3, the GL channels made from the texel's), the texture type. The
/// size, and a cube's textype, are added per texture ([`tex_image_word0`]).
#[derive(Clone, Copy)]
struct TexFormat {
    /// texstate.xml FORMAT, or FORMAT_COMPRESSED when `compressed`.
    texformat: u32,
    compressed: bool,
    swiz: [csb::ROGUE_TEXSTATE_SWIZ; 4],
    textype: csb::ROGUE_TEXSTATE_TEXTYPE,
    /// The STRIDE word's `alpha_msb` ([`tex_stride_image_word1`]). Mesa sets
    /// it for an RGB format whose fourth channel is W
    /// (`vk_format_is_alpha_on_msb`, pvr_arch_tex_state.c:253-254); the
    /// formats yakogl sampled before the ES 3.0 classes keep the `true` they
    /// were verified with on the board, and the ES 3.0 classes take Mesa's
    /// value (false for R11G11B10F and RGB9E5, whose alpha is the constant 1).
    alpha_msb: bool,
    /// TEXSTATE_IMAGE_WORD0 `gamma` (texstate.xml:287-288, bit 3): the TPU
    /// decodes red, green and blue from sRGB before it filters, alpha left
    /// linear -- Mesa's for a four-component sRGB format
    /// (pvr_arch_tex_state.c:207-226, `word0.gamma = ROGUE_TEXSTATE_GAMMA_ON`).
    gamma: bool,
}

const SWIZ_R: csb::ROGUE_TEXSTATE_SWIZ = csb::ROGUE_TEXSTATE_SWIZ_SRCCHAN_0;
const SWIZ_G: csb::ROGUE_TEXSTATE_SWIZ = csb::ROGUE_TEXSTATE_SWIZ_SRCCHAN_1;
const SWIZ_B: csb::ROGUE_TEXSTATE_SWIZ = csb::ROGUE_TEXSTATE_SWIZ_SRCCHAN_2;
const SWIZ_A: csb::ROGUE_TEXSTATE_SWIZ = csb::ROGUE_TEXSTATE_SWIZ_SRCCHAN_3;
const SWIZ_ONE: csb::ROGUE_TEXSTATE_SWIZ = csb::ROGUE_TEXSTATE_SWIZ_SRC_ONE;
const SWIZ_ZERO: csb::ROGUE_TEXSTATE_SWIZ = csb::ROGUE_TEXSTATE_SWIZ_SRC_ZERO;
const SWIZ_RGBA: [csb::ROGUE_TEXSTATE_SWIZ; 4] = [SWIZ_R, SWIZ_G, SWIZ_B, SWIZ_A];

/// A STRIDE (linear) texture of `texformat`.
const fn tex_stride(texformat: u32, swiz: [csb::ROGUE_TEXSTATE_SWIZ; 4]) -> TexFormat {
    TexFormat {
        texformat,
        compressed: false,
        swiz,
        textype: csb::ROGUE_TEXSTATE_TEXTYPE_STRIDE,
        alpha_msb: true,
        gamma: false,
    }
}

/// A STRIDE texture of `texformat` whose alpha is the constant 1, without
/// `alpha_msb` (Mesa's value for such a format, [`TexFormat::alpha_msb`]).
const fn tex_stride_no_alpha(texformat: u32, swiz: [csb::ROGUE_TEXSTATE_SWIZ; 4]) -> TexFormat {
    TexFormat { alpha_msb: false, ..tex_stride(texformat, swiz) }
}

/// U8U8U8U8, identity swizzle.
const TEXF_RGBA8: TexFormat = tex_stride(csb::ROGUE_TEXSTATE_FORMAT_U8U8U8U8, SWIZ_RGBA);
/// The sRGB class: U8U8U8U8 (Mesa's FORMAT(R8G8B8A8_SRGB, U8U8U8U8, ...),
/// pvr_arch_formats.c:105) with the TPU's gamma decode on.
const TEXF_SRGB8_A8: TexFormat = TexFormat { gamma: true, ..TEXF_RGBA8 };
/// Its image meta PCK_INFO: Mesa's setup_pck_info (pvr_arch_tex_state.c:
/// 84-112) for R8G8B8A8_SRGB, which pco_pipe_to_pck_format (pco/pco.c:
/// 387-461) does not list: "no pack format" (0x1f) with UNORM (0x100), the
/// format being unsigned normalised.
const TEX_PCK_INFO_SRGB8_A8: u32 = PVR_PCK_FORMAT_INVALID | PCO_PCK_UNORM;
/// ... read as B,G,R,A (swiz0 = chan 2, swiz2 = chan 0).
const TEXF_BGRA8: TexFormat = tex_stride(csb::ROGUE_TEXSTATE_FORMAT_U8U8U8U8, [SWIZ_B, SWIZ_G, SWIZ_R, SWIZ_A]);
/* Float texel formats (Mesa pvr_arch_tex_state.c setup_pck_info /
 * pco_pipe_to_pck_format): F16F16F16F16, F16, F32. The meta PCK_INFO is
 * PCO_PCK_FORMAT_F16F16 (14) with SPLIT for halves and PCO_PCK_FORMAT_F32
 * (15) for floats (pco's, not a csbgen word). A single-channel texture reads
 * (r, 0, 0, 1). */
const TEXF_RGBA16F: TexFormat = tex_stride(csb::ROGUE_TEXSTATE_FORMAT_F16F16F16F16, SWIZ_RGBA);
const TEXF_R16F: TexFormat = tex_stride(csb::ROGUE_TEXSTATE_FORMAT_F16, [SWIZ_R, SWIZ_ZERO, SWIZ_ZERO, SWIZ_ONE]);
const TEXF_R32F: TexFormat = tex_stride(csb::ROGUE_TEXSTATE_FORMAT_F32, [SWIZ_R, SWIZ_ZERO, SWIZ_ZERO, SWIZ_ONE]);
/// R32F read as luminance (r, r, r, 1), the L8 swizzle: a depth texture
/// under GL_OES_depth_texture (`GlesTextureBind::depth_luminance`).
const TEXF_R32F_LUMINANCE: TexFormat = tex_stride(csb::ROGUE_TEXSTATE_FORMAT_F32, [SWIZ_R, SWIZ_R, SWIZ_R, SWIZ_ONE]);
/* The ISP's packed D24S8 depth/stencil buffer read as a depth texture
 * ([`depth_buffer_binding`], docs/notes/pvr-depth-sample.md): Mesa's TPU
 * format for the depth aspect of a D24S8 image, X8U24
 * (pvr_arch_formats.c:156, `FORMAT_DEPTH_STENCIL(Z24_UNORM_S8_UINT, ST8U24,
 * X8U24, U8X24)`: the 24-bit unorm in bits 23..0 as the red channel, the
 * stencil byte ignored), a STRIDE image as every linear texture here. Read
 * as (r, 0, 0, 1), and as the luminance (r, r, r, 1) of GL_OES_depth_texture.
 * `alpha_msb` is Mesa's for the format (false: no alpha). The pack info is
 * Mesa's setup_pck_info for Z24X8_UNORM, which pco_pipe_to_pck_format does
 * not list ("no pack format", 0x1f) with UNORM (0x100). */
const TEXF_X8U24: TexFormat =
    tex_stride_no_alpha(csb::ROGUE_TEXSTATE_FORMAT_X8U24, [SWIZ_R, SWIZ_ZERO, SWIZ_ZERO, SWIZ_ONE]);
const TEXF_X8U24_LUMINANCE: TexFormat =
    tex_stride_no_alpha(csb::ROGUE_TEXSTATE_FORMAT_X8U24, [SWIZ_R, SWIZ_R, SWIZ_R, SWIZ_ONE]);
const TEX_PCK_INFO_X8U24: u32 = PVR_PCK_FORMAT_INVALID | PCO_PCK_UNORM;
/// pco's PCK_INFO SPLIT flag (pco_pipe_to_pck_format).
const PCO_PCK_SPLIT: u32 = 0x20;
const TEX_PCK_INFO_F16: u32 = 14 | PCO_PCK_SPLIT;
const TEX_PCK_INFO_F32: u32 = 15;
/* Compressed (ETC) textures: TEXTYPE 2D (twiddled, no stride word),
 * FORMAT_COMPRESSED ETC2_RGB / ETC2A_RGBA / ETC2_PUNCHTHROUGHA, RGB formats
 * read alpha as SRC_ONE, IMAGE_WORD1 tpu_image_state_v2_compression_mode =
 * TPU; the pack info is "no conversion" (PVR_PCK_FORMAT_INVALID 0x1f) +
 * UNORM (bit 8): Mesa pvr_arch_tex_state.c / setup_pck_info for
 * VK_FORMAT_ETC2_*. */
const fn tex_etc(texformat: u32, swiz: [csb::ROGUE_TEXSTATE_SWIZ; 4]) -> TexFormat {
    TexFormat {
        texformat,
        compressed: true,
        swiz,
        textype: csb::ROGUE_TEXSTATE_TEXTYPE_2D,
        alpha_msb: false,
        gamma: false,
    }
}
const TEXF_ETC2_RGB: TexFormat =
    tex_etc(csb::ROGUE_TEXSTATE_FORMAT_COMPRESSED_ETC2_RGB, [SWIZ_R, SWIZ_G, SWIZ_B, SWIZ_ONE]);
const TEXF_ETC2_RGBA: TexFormat = tex_etc(csb::ROGUE_TEXSTATE_FORMAT_COMPRESSED_ETC2A_RGBA, SWIZ_RGBA);
/* the sRGB blocks: the same TPU formats with the gamma decode on
 * (pvr_arch_tex_state.c:226, docs/es3/pvr-formats.md:406) */
const TEXF_ETC2_SRGB_RGB: TexFormat = TexFormat { gamma: true, ..TEXF_ETC2_RGB };
const TEXF_ETC2_SRGB_RGBA: TexFormat = TexFormat { gamma: true, ..TEXF_ETC2_RGBA };
/// Is the texel class ETC2 blocks, laid out as the twiddled block grid?
fn is_etc2(f: u8) -> bool {
    matches!(
        f,
        GLES_TEXFMT_ETC2_RGB
            | GLES_TEXFMT_ETC2_RGBA
            | GLES_TEXFMT_ETC2_RGB_PT
            | GLES_TEXFMT_ETC2_SRGB_RGB
            | GLES_TEXFMT_ETC2_SRGB_RGBA
    )
}
const TEXF_ETC2_PT: TexFormat = tex_etc(csb::ROGUE_TEXSTATE_FORMAT_COMPRESSED_ETC2_PUNCHTHROUGHA, SWIZ_RGBA);
/* 16-bit R5G6B5: STRIDE textype, alpha read as SRC_ONE. The TPU's channel 0
 * is the low 5 bits; DRM RGB565 keeps red in the high bits, so channels 0
 * and 2 are swapped like the XRGB8888 swizzle. */
const TEXF_RGB565: TexFormat = tex_stride(csb::ROGUE_TEXSTATE_FORMAT_R5G6B5, [SWIZ_B, SWIZ_G, SWIZ_R, SWIZ_ONE]);
const TEX_PCK_INFO_COMPRESSED_UNORM: u32 = 0x11f;
/* The narrow luminance/alpha texels (U8, U8U8), STRIDE textype, the GL
 * channels made by the swizzle: L8 (L,L,L,1), A8 (0,0,0,A), LA8 (L,L,L,A). */
const TEXF_L8: TexFormat = tex_stride(csb::ROGUE_TEXSTATE_FORMAT_U8, [SWIZ_R, SWIZ_R, SWIZ_R, SWIZ_ONE]);
const TEXF_A8: TexFormat = tex_stride(csb::ROGUE_TEXSTATE_FORMAT_U8, [SWIZ_ZERO, SWIZ_ZERO, SWIZ_ZERO, SWIZ_R]);
const TEXF_LA8: TexFormat = tex_stride(csb::ROGUE_TEXSTATE_FORMAT_U8U8, [SWIZ_R, SWIZ_R, SWIZ_R, SWIZ_G]);

/* The ES 3.0 storage classes (src/backend.rs GLES_TEXFMT_R11G11B10F ..
 * GLES_TEXFMT_RGBA32F; docs/es3/sized-formats.md): every one four components
 * wide in memory, a component the GL format lacks stored as (0, 0, 0, 1) by
 * the GL core, so the swizzle is the identity except where the TPU format has
 * no alpha. TPU formats from Mesa 26.2.1's pvr_format_table
 * (vulkan/pvr_arch_formats.c, the row of the pipe format the class holds):
 *   R11G11B10F  R11G11B10_FLOAT     F10F11F11      :150
 *   RGB9E5      R9G9B9E5_FLOAT      SE9995         :151
 *   RGB10A2     R10G10B10A2_UNORM   A2R10B10G10    :115
 *   RGB10A2UI   R10G10B10A2_UINT    A2R10B10G10    :119
 *   RGBA8_SNORM R8G8B8A8_SNORM      S8S8S8S8       :102
 *   RGBA8UI/I   R8G8B8A8_UINT/SINT  U8U8U8U8 / S8S8S8S8  :103-104
 *   RGBA16UI/I  R16G16B16A16_UINT/SINT  U16U16U16U16 / S16S16S16S16  :135-136
 *   RGBA32UI/I  R32G32B32A32_UINT/SINT  U32U32U32U32 / S32S32S32S32  :147-148
 *   RGBA32F     R32G32B32A32_FLOAT  F32F32F32F32   :149
 * The swizzle is the format's util swizzle (u_format.yaml: xyz1 for
 * R11G11B10_FLOAT and R9G9B9E5_FLOAT, xyzw for the others) through
 * pvr_get_hw_swizzle (pvr_arch_tex_state.c:40-64). An integer class is read
 * by an isampler/usampler, whose `smp` has no `fcnorm` (pco_nir_tex.c:236):
 * the TPU hands back the integers, sign-extended for the S formats. */
const TEXF_R11G11B10F: TexFormat =
    tex_stride_no_alpha(csb::ROGUE_TEXSTATE_FORMAT_F10F11F11, [SWIZ_R, SWIZ_G, SWIZ_B, SWIZ_ONE]);
const TEXF_RGB9E5: TexFormat =
    tex_stride_no_alpha(csb::ROGUE_TEXSTATE_FORMAT_SE9995, [SWIZ_R, SWIZ_G, SWIZ_B, SWIZ_ONE]);
const TEXF_RGB10A2: TexFormat = tex_stride(csb::ROGUE_TEXSTATE_FORMAT_A2R10B10G10, SWIZ_RGBA);
const TEXF_RGBA8_SNORM: TexFormat = tex_stride(csb::ROGUE_TEXSTATE_FORMAT_S8S8S8S8, SWIZ_RGBA);
const TEXF_RGBA8UI: TexFormat = tex_stride(csb::ROGUE_TEXSTATE_FORMAT_U8U8U8U8, SWIZ_RGBA);
const TEXF_RGBA8I: TexFormat = tex_stride(csb::ROGUE_TEXSTATE_FORMAT_S8S8S8S8, SWIZ_RGBA);
const TEXF_RGBA16UI: TexFormat = tex_stride(csb::ROGUE_TEXSTATE_FORMAT_U16U16U16U16, SWIZ_RGBA);
const TEXF_RGBA16I: TexFormat = tex_stride(csb::ROGUE_TEXSTATE_FORMAT_S16S16S16S16, SWIZ_RGBA);
const TEXF_RGBA32UI: TexFormat = tex_stride(csb::ROGUE_TEXSTATE_FORMAT_U32U32U32U32, SWIZ_RGBA);
const TEXF_RGBA32I: TexFormat = tex_stride(csb::ROGUE_TEXSTATE_FORMAT_S32S32S32S32, SWIZ_RGBA);
const TEXF_RGBA32F: TexFormat = tex_stride(csb::ROGUE_TEXSTATE_FORMAT_F32F32F32F32, SWIZ_RGBA);
/* Their image meta PCK_INFO, as Mesa's setup_pck_info (pvr_arch_tex_state.c:
 * 84-112) builds it from pco_pipe_to_pck_format (pco/pco.c:387-461; format
 * numbers pco/pco_isa.py:981-1003; bits common/pvr_iface.h:132-142): S8888
 * (1) with SCALE (0x40) for RGBA8 SNORM, U1010102 (8) with SCALE and UNORM
 * (0x100) for RGB10A2 UNORM, F111110 (12) for R11G11B10F, and "no pack
 * format" (PVR_PCK_FORMAT_INVALID, 0x1f) for the integer classes, RGB9E5 and
 * RGBA32F -- pco reads it only for a shadow compare's clamp and a formatless
 * image store, neither of which these reach. */
const PCO_PCK_SCALE: u32 = 0x40;
const PCO_PCK_UNORM: u32 = 0x100;
const PVR_PCK_FORMAT_INVALID: u32 = 0x1f;
const TEX_PCK_INFO_RGBA8_SNORM: u32 = 1 | PCO_PCK_SCALE;
const TEX_PCK_INFO_RGB10A2_UNORM: u32 = 8 | PCO_PCK_SCALE | PCO_PCK_UNORM;
const TEX_PCK_INFO_R11G11B10F: u32 = 12;

/// The ES 3.0 storage class `format`'s texture format and pack info, `None`
/// for every other format ([`build_tex_desc`]).
fn es3_tex_format(format: u8) -> Option<(TexFormat, u32)> {
    Some(match format {
        GLES_TEXFMT_R11G11B10F => (TEXF_R11G11B10F, TEX_PCK_INFO_R11G11B10F),
        GLES_TEXFMT_RGB9E5 => (TEXF_RGB9E5, PVR_PCK_FORMAT_INVALID),
        GLES_TEXFMT_RGB10A2 => (TEXF_RGB10A2, TEX_PCK_INFO_RGB10A2_UNORM),
        GLES_TEXFMT_RGB10A2UI => (TEXF_RGB10A2, PVR_PCK_FORMAT_INVALID),
        GLES_TEXFMT_RGBA8_SNORM => (TEXF_RGBA8_SNORM, TEX_PCK_INFO_RGBA8_SNORM),
        GLES_TEXFMT_RGBA8UI => (TEXF_RGBA8UI, PVR_PCK_FORMAT_INVALID),
        GLES_TEXFMT_RGBA8I => (TEXF_RGBA8I, PVR_PCK_FORMAT_INVALID),
        GLES_TEXFMT_RGBA16UI => (TEXF_RGBA16UI, PVR_PCK_FORMAT_INVALID),
        GLES_TEXFMT_RGBA16I => (TEXF_RGBA16I, PVR_PCK_FORMAT_INVALID),
        GLES_TEXFMT_RGBA32UI => (TEXF_RGBA32UI, PVR_PCK_FORMAT_INVALID),
        GLES_TEXFMT_RGBA32I => (TEXF_RGBA32I, PVR_PCK_FORMAT_INVALID),
        GLES_TEXFMT_RGBA32F => (TEXF_RGBA32F, PVR_PCK_FORMAT_INVALID),
        _ => return None,
    })
}

/// TEXSTATE_IMAGE_WORD0 of a `width` x `height` image of format `f`, as
/// `textype` (the format's own, or CUBE for the native cube).
fn tex_image_word0(f: TexFormat, textype: csb::ROGUE_TEXSTATE_TEXTYPE, width: u32, height: u32) -> [u32; 2] {
    let [swiz0, swiz1, swiz2, swiz3] = f.swiz;
    csb_pack!(
        ROGUE_TEXSTATE_IMAGE_WORD0_pack,
        2,
        csb::ROGUE_TEXSTATE_IMAGE_WORD0 {
            texformat: if f.compressed { 0 } else { f.texformat },
            texformat_compressed: if f.compressed { f.texformat } else { 0 },
            width: width.wrapping_sub(1),
            height: height.wrapping_sub(1),
            swiz0,
            swiz1,
            swiz2,
            swiz3,
            textype,
            gamma: if f.gamma { csb::ROGUE_TEXSTATE_GAMMA_ON } else { csb::ROGUE_TEXSTATE_GAMMA_OFF },
            ..Default::default()
        }
    )
}

/// TEXSTATE_STRIDE_IMAGE_WORD1 of a linear image: `alpha_msb`, the row
/// stride in texels minus one, `levels` mip levels (num_mip_levels, and
/// mipmaps_present when more than one), the image address.
fn tex_stride_image_word1(stride_texels: u32, levels: u32, base_gpu: u64, alpha_msb: bool) -> [u32; 2] {
    csb_pack!(
        ROGUE_TEXSTATE_STRIDE_IMAGE_WORD1_pack,
        2,
        csb::ROGUE_TEXSTATE_STRIDE_IMAGE_WORD1 {
            num_mip_levels: levels,
            alpha_msb,
            mipmaps_present: levels > 1,
            stride: stride_texels.wrapping_sub(1),
            texaddr: dev_addr(base_gpu),
            ..Default::default()
        }
    )
}

/// TEXSTATE_IMAGE_WORD1 of a twiddled image (texstate.xml:292-312: not the
/// stride word): `num_mip_levels` at [0:3], `depth` (array layers - 1),
/// `mipmaps_present`, the image address, baselevel 0. An ETC image is
/// decoded by the TPU (compression mode TPU) and has no alpha_msb; the
/// others have alpha_msb as their stride word.
fn tex_twiddled_image_word1(
    etc: bool,
    num_mip_levels: u32,
    depth: u32,
    mipmaps_present: bool,
    base_gpu: u64,
) -> [u32; 2] {
    csb_pack!(
        ROGUE_TEXSTATE_IMAGE_WORD1_pack,
        2,
        csb::ROGUE_TEXSTATE_IMAGE_WORD1 {
            alpha_msb: !etc,
            tpu_image_state_v2_compression_mode: if etc {
                csb::ROGUE_TEXSTATE_COMPRESSION_MODE_TPU
            } else {
                csb::ROGUE_TEXSTATE_COMPRESSION_MODE_NONE
            },
            num_mip_levels,
            depth,
            mipmaps_present,
            texaddr: dev_addr(base_gpu),
            ..Default::default()
        }
    )
}

/// The LOD clamp's maximum, u4.6 (texstate.xml CLAMP MAX).
const TEX_SAMPLER_MAXLOD_MAX: u32 = csb::ROGUE_TEXSTATE_CLAMP_MAX;

/// GL's wrap mode as the TPU's ADDRMODE, one axis at a time, because GL
/// wraps S and T independently: FLIP is GL's mirrored repeat.
fn tpu_addrmode(wrap: u8) -> csb::ROGUE_TEXSTATE_ADDRMODE {
    match wrap {
        GLES_WRAP_CLAMP_TO_EDGE => csb::ROGUE_TEXSTATE_ADDRMODE_CLAMP_TO_EDGE,
        GLES_WRAP_MIRRORED_REPEAT => csb::ROGUE_TEXSTATE_ADDRMODE_FLIP,
        _ => csb::ROGUE_TEXSTATE_ADDRMODE_REPEAT,
    }
}

/// The wrap mode's name, for the per-draw texture dump.
fn wrap_name(wrap: u8) -> &'static str {
    match wrap {
        GLES_WRAP_CLAMP_TO_EDGE => "clamp",
        GLES_WRAP_MIRRORED_REPEAT => "mirror",
        _ => "repeat",
    }
}

/// pvr_combined_image_sampler_descriptor.
const TEX_DESC_DWORDS: u32 = 20;
/// Descriptor slots per draw: one per sampler the shader declares (the
/// compiler put the uniform block after them, so this must not be the smaller
/// number).
const PVR_BACKEND_MAX_SAMPLERS: usize = GLES_MAX_TEX_BINDS;
/// Image meta PCK_INFO for RGBA8 UNORM (pco_shc: pck_info(rgba8_unorm)).
const TEX_PCK_INFO_RGBA8_UNORM: u32 = 0x140;

/// The C file's `#ifdef PVR_BACKEND_TRACE`: a per-frame console trace (~7 ms
/// per line on a 115200-baud UART). Off, as in every C build configuration.
const PVR_BACKEND_TRACE: bool = false;

#[derive(Clone, Copy)]
struct Arena {
    mem: OsalGpuMem,
    bump: u32,
}

/// Which depth/stencil buffer: [`zs_keys`].
#[derive(Clone, Copy, PartialEq, Eq)]
struct DsKey {
    /// DS_PACKED: one D24S8 per `GlesDraw::ds_id` (both halves). DS_DEPTH:
    /// a depth object's own D24S8 (`depth_id`). DS_STENCIL: a stencil
    /// object's separate buffer (`stencil_id`), read through
    /// CR_ISP_STENCIL_LOAD_BASE's enable -- never the same bytes as a
    /// packed buffer, whose stencil sits in another byte.
    kind: u8,
    id: u32,
    /// The object's `storage_gen`: a respecified object is a new buffer.
    gen: u32,
}

const DS_PACKED: u8 = 0;
const DS_DEPTH: u8 = 1;
const DS_STENCIL: u8 = 2;

impl DsKey {
    const ZERO: DsKey = DsKey { kind: DS_PACKED, id: 0, gen: 0 };
}

/// One depth/stencil buffer the ISP loads and stores (per [`DsKey`]).
#[derive(Clone, Copy)]
struct DsBuf {
    id: DsKey,
    /* the target size/samples it was sized for */
    w: u32,
    h: u32,
    samples: u32,
    /// A completed kick stored into it.
    valid: bool,
    mem: OsalGpuMem,
    /// What its depth and stencil planes hold for GL ([`Plane`]): a buffer
    /// with a live plane is never let go for another target. Only the
    /// planes its kind keeps are tracked (a DS_DEPTH buffer's stencil byte
    /// and a DS_STENCIL buffer's depth stay Undefined).
    planes: [Plane; 2],
    /// The GL object of each plane (`GlesDraw::depth_id` form; 0: none):
    /// `attachment_gone` of one undefines its plane, and a buffer left
    /// holding nothing is freed.
    owners: [u32; 2],
    /// [`Backend::ds_clock`] when a frame last asked for it: the least
    /// recently used buffer that may go is the one given up.
    used: u32,
}

impl DsBuf {
    const ZERO: DsBuf = DsBuf {
        id: DsKey::ZERO,
        w: 0,
        h: 0,
        samples: 0,
        valid: false,
        mem: OsalGpuMem::ZERO,
        planes: [Plane::UNDEFINED; 2],
        owners: [0; 2],
        used: 0,
    };

    /// The planes a buffer of this kind keeps.
    fn kept_planes(&self) -> &'static [usize] {
        match self.id.kind {
            DS_PACKED => &[DEPTH, STENCIL],
            DS_DEPTH => &[DEPTH],
            _ => &[STENCIL],
        }
    }

    /// Holds contents GL keeps: never let go for another target.
    fn holds_contents(&self) -> bool {
        !self.mem.impl_.is_null() && self.planes.iter().any(|p| p.is_live())
    }
}

/// Where a frame keeps one half of its depth/stencil ([`Backend::frame_ds`],
/// [`Backend::frame_s`]). "No attachment" and "no memory" are two states: the
/// first is a legal target (an FBO with no depth or stencil), the second is a
/// target whose attachment exists and whose buffer could not be allocated,
/// so a draw that tests it would draw against on-chip values nothing loads
/// and nothing stores.
///
/// All-zero is [`DsStore::NoAttachment`] (the explicit discriminant 0), so
/// the zero-initialised [`PVR`] stays valid.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DsStore {
    /// No buffer is wanted: the target has no depth/stencil attachment (for
    /// [`Backend::frame_s`]: no separate stencil, it is in `frame_ds`).
    NoAttachment = 0,
    /// The buffer: an index into [`Backend::ds`].
    Buf(u32) = 1,
    /// The target has the attachment and its buffer could not be allocated.
    NoMemory = 2,
}

impl DsStore {
    fn buf(self) -> Option<usize> {
        match self {
            DsStore::Buf(i) => Some(i as usize),
            _ => None,
        }
    }
}

/// Attachments whose depth or stencil contents are lost ([`Backend::ds_lost`]).
#[derive(Clone, Copy)]
struct DsLost {
    key: DsKey,
    z: bool,
    s: bool,
}

impl DsLost {
    const ZERO: DsLost = DsLost { key: DsKey::ZERO, z: false, s: false };
}

/// Lost attachments remembered at once. An entry goes when a clear covers
/// what it lost; one more loss than this forgets the oldest, with a log line.
const DS_LOST_MAX: usize = 16;

/// One set of the per-kick resources (see [`Backend::sets`]).
#[derive(Clone, Copy)]
struct FrameSet {
    pds: Arena,
    gen: Arena,
    vdm: OsalGpuMem,
    dbias: OsalGpuMem,
    scissor: OsalGpuMem,
    pds_static: OsalGpuMem,
    bg_color: OsalGpuMem,
    bg_kick_off: u32,
    bg_dma_data_off: u32,
    bg_dma_code_off: u32,
    eot_data_off: u32,
    eot_data_local: u32,
    /// The kick that last used the set (0: none).
    fence: i64,
}

/// The backend's state (`static struct { ... } B` + `B_stats` of the C file).
struct Backend {
    inited: bool,
    st: PvrRenderStatic,
    opt: [u32; PVR_BACKEND_OPT_COUNT],

    pds: Arena,
    /// The PDS programs the open frame has uploaded, for [`pds_put`] to
    /// point a draw at instead of uploading the same words again.
    pds_seen: [PdsSeen; PDS_SEEN_SLOTS],
    /// The kick-USC and coefficient programs the open frame's draws point
    /// at, by their generators' inputs: a draw that matches one reuses its
    /// offset without generating it ([`pds_once_forget`]). Cleared with the
    /// arena; a failed draw's entries go with the bytes it gives back.
    kick_once: [KickOnce; PDS_ONCE_SLOTS],
    coeff_once: [CoeffOnce; PDS_ONCE_SLOTS],
    /// The slot each table replaces next (oldest first).
    kick_once_next: u8,
    coeff_once_next: u8,
    gen: Arena,
    vdm: OsalGpuMem,
    /// u16 0..N-1.
    seq_idx: OsalGpuMem,
    seq_idx_count: u32,

    /// Sets of the per-kick resources (the arenas, the VDM stream, the
    /// depth-bias table, the background/EOT PDS page and the clear colour):
    /// the next kick is recorded into one while the GPU still reads the
    /// others. The fields above/below stay the "active" copies; a set is
    /// copied in when a frame starts (`frame_set_activate`) and its fence is
    /// waited for before it is reused, so the set count is how many kicks may
    /// be in flight. A Quick 3D frame is several passes (shadow maps, the
    /// scene, effects), and with two sets a pass could not start until the one
    /// before last had finished.
    sets: [FrameSet; PVR_BACKEND_FRAME_SETS],
    /// The sets in use: the first `nsets` of `sets` ([`ArenaCfg`]).
    nsets: usize,
    /// The active set.
    cur: usize,
    /// The most recent kick submitted (0: none).
    last_fence: i64,

    /* persistent frag-side objects */
    /// Clear FS USC (one colour dword).
    clear_fs: OsalGpuMem,
    /// Clear FS USC for two colour dwords (RGBA16F targets).
    clear_fs2: OsalGpuMem,
    /// Its kick-USC PDS program.
    bg_kick2_off: u32,
    /// Clear FS USC for three colour dwords (three 8-bit attachments,
    /// [`mrt_clear_dwords`]), and its kick-USC PDS program.
    clear_fs3: OsalGpuMem,
    bg_kick3_off: u32,
    /// Clear FS USC for four colour dwords: an RGBA32I/UI target's clear
    /// quad (no background kick: integer targets clear by quad).
    clear_fs4: OsalGpuMem,
    /// bg kick program, bg colour DMA program, EOT data.
    pds_static: OsalGpuMem,
    /// CLEAR_ATTACH_DWORDS dwords.
    bg_color: OsalGpuMem,
    bg_kick_off: u32,
    bg_dma_data_off: u32,
    bg_dma_code_off: u32,
    /* Framebuffer load (frames that start without a colour clear): the
     * first draw of the frame is a full-screen quad whose FS fetches the
     * target's previous texel under each pixel (Mesa's index-list load).
     * The quad's shader descriptors and its synthesised draw live in
     * `FbLoad`, outside this struct. */
    fbload_fs: OsalGpuMem,
    fbload_vs: OsalGpuMem,
    fbload_quad: OsalGpuMem,
    /// One opaque black RGBA8 texel: what GL's incomplete texture, and a
    /// sampler with nothing bound, read as (GL ES 2.0 section 3.7.10). It
    /// replaces the driver's zeroed border page, which gave (0,0,0,0).
    black_tex: OsalGpuMem,
    /// The GPU mipmap quad's fragment program ([`PVR_INTERNAL_MIPGEN`]):
    /// draws level n of a chain by sampling level n-1 at the centre of each
    /// of its texels ([`pvr_generate_mipmap`]).
    mipgen_fs: OsalGpuMem,
    /// The fence of the last kick of the last chain made on the GPU (0:
    /// none): what [`pvr_return_mipmap_chain`] waits for before the GL core
    /// reads or writes such a chain's levels on the CPU.
    mip_fence: i64,
    /// Chains made on the GPU, and chains asked for but left to the CPU's
    /// filter, since the backend started (instruments).
    mip_chains: u32,
    mip_chains_refused: u32,
    /// The extent the PBE clips attachment 0 to in the next frame
    /// [`record_mip_level`] opens, and in the open frame (0, 0: the
    /// target's own extent, every frame but a chain level's). Set only for
    /// the duration of one level's `draw_one`, reset after it whatever it
    /// returned. A level's
    /// frame declares level 1's size as its target, so every level of a
    /// chain renders with one hardware render-target set (the driver keeps
    /// one per size, and the first board run of this path recycled a set
    /// for nearly every level kick), and writes only the level's own
    /// texels through this clip ([`pbe_reg_words`] maxclip).
    mip_clip: (u32, u32),
    frame_clip: (u32, u32),
    /// fbload FS variants that pack what they read for a float target
    /// (index = gles_surf_rt_class - 1: RGBA16F, R16F, R32F).
    fbload_fs_rt: [OsalGpuMem; 3],
    /// fbload FS variants for an integer target ([`FBLOAD_INT`]): the
    /// pixel's bits read raw and written back as 1, 2 or 4 dwords.
    fbload_fs_int: [OsalGpuMem; 3],
    /// The masked clear FS variants ([`CLEAR_MASKED`]), in its order.
    clear_mask_fs: [OsalGpuMem; CLEAR_MASKED_N],
    /// This frame begins with a colour clear.
    frame_cleared: bool,
    /// This frame's first object reloads the target's colour (it began
    /// without a colour clear): a colour clear can no longer join it.
    frame_loads_color: bool,
    /// The open frame renders into the surface being swapped: its kick
    /// stores no depth/stencil. Set by `pvr_backend_surface_swapped`,
    /// consumed by the next flush.
    swap_drops_zs: bool,
    /// Kicks whose depth/stencil store the swap dropped (instrument).
    swap_zs_dropped: u32,
    /// The open frame renders into the surface being swapped (set by
    /// `pvr_backend_surface_swapped`, whatever the option above says): its
    /// kick is the frame's last, so it keeps no samples ([`MsStore`]).
    frame_swapped: bool,
    /// The samples of a multisampled target across a split ([`MsStore`]).
    ms: MsStore,
    /// Load/clear quads recorded ahead of the app's draws.
    frame_quads: u32,
    /// Tile buffers this frame's fragment shaders spilled outputs into: the
    /// end-of-tile program loads the last attachment back from one.
    frame_tile_buffers: u32,
    eot_data_off: u32,
    eot_data_local: u32,
    bg_dma_data_dw: u32,
    bg_dma_temps: u32,

    /* current frame */
    frame_open: bool,
    ndraws: u32,
    /// Primitives tiled into the open kick.
    prims: u32,
    target_gpu: u64,
    target_w: u32,
    target_h: u32,
    /// Attachment 0's row pitch in pixels ([`crate::backend::draw_pitch`]):
    /// the PBE's line stride and the load's texture stride.
    target_pitch: u32,
    /// `GLES_SURF_*`.
    target_format: u8,
    /// `GlesDraw::target_alpha_bits` of the frame's attachment 0: 0 means the
    /// GL format has no alpha, so the PBE writes 1 there.
    target_alpha_bits: u8,
    /// The frame's target orientation (for a synthesised object).
    target_flip_y: bool,
    /// Colour attachments beyond the first (a G-buffer pass writes three):
    /// `[0]` mirrors `target_gpu`/`w`/`h`/`format`, and `num_targets` 0 means
    /// one.
    targets: [GlesDrawTarget; PVR_RENDER_MAX_RT],
    num_targets: u8,
    /// The frame's target sample count.
    samples: u32,
    clear_rgba: [f32; 4],
    clear_depth: f32,
    clear_stencil: u8,
    /* depth/stencil memory of the frame's target (none: on-chip only) and
     * whether this kick loads them back (a kick always stores them) */
    ds: [DsBuf; PVR_BACKEND_MAX_DS],
    /// Planes of buffers let go while they held only a clear value
    /// (src/ds_retention.rs): given back when their key is drawn again.
    ds_parked: Parked<DsKey, Plane, PVR_BACKEND_MAX_DS>,
    /// What `ds_make_room` took back from `ds_parked` for the frame it admits:
    /// `frame_begin` starts the new buffers from these clear values.
    ds_restore: [Plane; 2],
    /// The frame's whole clears of the depth and the stencil (Undefined:
    /// none), and whether a draw of the frame wrote them: what the kick
    /// leaves in the frame's buffers (`ds_settle`).
    frame_clr: [Plane; 2],
    frame_wrote: [bool; 2],
    /// `GlesDraw::ds_id` and `ds_persist` of the frame, so a draw that wants
    /// a different depth/stencil buffer ends this frame rather than joining
    /// it. Two framebuffer objects can share one colour buffer and keep
    /// depth/stencil of their own (dEQP's fbo.render.shared_colorbuffer and
    /// .shared_depthbuffer are built on it), and the colour address, size and
    /// sample count -- all this used to compare -- are then identical.
    frame_ds_id: u32,
    frame_ds_persist: bool,
    /// The frame's depth/stencil buffer (the C `frame_ds` pointer), or why
    /// it has none.
    frame_ds: DsStore,
    /// With depth and stencil kept apart ([`split_zs_on`]) and the frame's two
    /// renderbuffers: the stencil's own buffer, which the ISP loads and
    /// stores through CR_ISP_STENCIL_LOAD_BASE with its separate-stencil
    /// enable; `frame_ds` then holds the depth. `NoAttachment`: the stencil
    /// is in `frame_ds` (packed D24S8).
    frame_s: DsStore,
    /// Attachments whose contents were lost (their buffer could not be
    /// allocated), per half: [`ds_refuse_lost`].
    ds_lost: [DsLost; DS_LOST_MAX],
    nds_lost: usize,
    /// Draws and clears refused for a depth/stencil the backend could not
    /// keep (`pvr_backend_ds_refused`), and whether this frame logged one.
    ds_refused: u32,
    ds_refused_logged: bool,
    /// Draws refused because every one of the PVR_BACKEND_MAX_DS buffers held
    /// contents GL keeps ([`ds_make_room`]); not in `ds_refused`.
    ds_full_refused: u32,
    /// The most depth/stencil buffers held at once since the process began,
    /// and the most bytes they held at once ([`ds_report`]); 0 slots: the
    /// backend never allocated one.
    ds_slots_peak: u32,
    ds_bytes_peak: usize,
    /// Buffers given up to make room for another target's ([`ds_make_room`]):
    /// each one is a free and a new allocation, and a count that grows with
    /// the frames says the application uses more targets than there are
    /// slots.
    ds_given_up: u32,
    /// Draws that sampled a depth texture from its depth buffer, no CPU
    /// resolve ([`depth_buffer_binding`]), and draws whose binding named a
    /// buffer that was not there to sample (black instead).
    depth_direct: u32,
    depth_refused: u32,
    /// Depth/stencil buffer requests ([`ds_get`]): the buffers' age.
    ds_clock: u32,
    /// The frame arenas' high-water marks over every kick recorded so far,
    /// bytes: [GENERAL arena, PDS arena, VDM stream] (the owner report).
    arena_hw: [u32; 3],
    /// The frame's (depth_id, depth_gen, stencil_id, stencil_gen).
    frame_zs: [u32; 4],
    /// DBIAS_MAX_ENTRIES x 3 floats.
    dbias: OsalGpuMem,
    /// Entries used by this frame.
    ndbias: u32,
    /// The last entry (draws with the same state share it).
    dbias_last: [f32; 3],
    /// Occlusion queries (docs/es3/occlusion-query.md 3.2): the result areas
    /// (VIS_SLOTS x VIS_SLOT_BYTES, allocated by the first query), the
    /// handles counted by the open frame, per area the kick that writes it
    /// (0: free) and the handles it holds, and per handle the sum of its
    /// completed kicks' counts, and whether some count was lost.
    vis: OsalGpuMem,
    vis_frame: u64,
    vis_fence: [i64; VIS_SLOTS],
    vis_mask: [u64; VIS_SLOTS],
    vis_sum: [u64; GLES_QUERY_HANDLES as usize],
    vis_lost: u64,
    /// SCISSOR_MAX_ENTRIES x 2 dwords.
    scissor: OsalGpuMem,
    /// Entries used by this frame.
    nscissor: u32,
    scissor_last: [u32; 2],
    load_depth: bool,
    load_stencil: bool,
    warned_no_psize: bool,
    warned_load: bool,
    warned_ds: bool,
    /// A fragment varying iterated from an offset the vertex shader does not
    /// write there (an unlinked shader) was reported.
    ///
    /// (Upstream's `warned_dbias_full` / `warned_scissor_full` have no
    /// counterpart: a full per-frame depth-bias or scissor table splits the
    /// frame here instead of reusing another draw's entry, so there is
    /// nothing to warn about.)
    warned_varying: bool,
    /// Sampler image addresses outside the heaps reported so far.
    warned_tex_addr: u32,
    /// A colour attachment past the pixel's output registers was reported.
    warned_out_regs: bool,
    /// A kick with more attachments than its fragment shader writes outputs.
    warned_mrt_outputs: u32,
    /// A draw refused because its fragment stage needed more common-store
    /// DMA kicks than the program holds (reported once).
    warned_dma_kicks: bool,
    /// A draw refused because draw buffer 0 was GL_NONE while a later one
    /// was written (review R-09; reported once).
    warned_draw_buffer0: bool,
    /// Bring-up lines left for the multi-attachment emit words.
    mrt_log_left: u32,
    /// Bring-up lines left for the per-kick summary.
    kick_log_left: u32,
    /// Bring-up lines left for the per-draw shader/texture summary.
    draw_log_left: u32,
    /// A draw was dropped because it cannot fit a frame or be split.
    warned_big: bool,
    vdm_dw: u32,
    /// Per-draw dumps still allowed (see [`pvr_backend_set_option`]).
    dbg_draws_left: u32,
    /// Terminate block of the last draw's PPP.
    term_gpu: u64,

    /// The kick descriptor handed to the OSAL. Built in place (zeroed, then
    /// field writes) so that its padding bytes are zero like the C memset's:
    /// the descriptor crosses the OSAL as raw bytes.
    kick: PvrRenderKick,

    /* `B_stats`: not part of the state pvr_backend_init() resets */
    kicks: u32,
    waits: u32,
    /// Fence waits that gave up (WAIT_MS) with the kick still incomplete.
    wait_timeouts: u32,
    draws_total: u32,
    tiles_total: u32,
    /// What the kicks were spent on: full-screen quads the driver added to
    /// reload colour or clear, and kicks a frame was split into because one
    /// could not hold the geometry.
    quads_total: u32,
    splits_total: u32,
    /// Draws recorded as nothing because they can touch no pixel (draw_box).
    draws_empty_box: u32,
}

/// The framebuffer-load quad's hand-built shader descriptors and the draw
/// synthesised from them. Kept off the stack like the C `B.load_draw`, and
/// outside [`Backend`] because `record_draw` reads the draw (and, through its
/// `vs`/`fs` pointers, the descriptors) while it mutates the backend state.
struct FbLoad {
    vs_sh: ShcShader,
    fs_sh: ShcShader,
    /// The float-target load FS variants (as `Backend::fbload_fs_rt`).
    fs_rt_sh: [ShcShader; 3],
    /* Float targets are cleared by a full-screen quad drawn with the clear
     * USC as its FS (the background object leaves their pixels untouched on
     * silicon: gl_test case 44). The packed colour is the quad's uniform. */
    clear_fs_sh: ShcShader,
    clear_fs2_sh: ShcShader,
    /// The four-dword clear (RGBA32I/UI targets).
    clear_fs4_sh: ShcShader,
    /// The integer-target load FS variants (as `Backend::fbload_fs_int`).
    fs_int_sh: [ShcShader; 3],
    clear_pack: [u32; 4],
    /// The masked clear variants' metadata (as `Backend::clear_mask_fs`), and
    /// the clear colour they read: four floats, clear.frag's uniform.
    clear_mask_sh: [ShcShader; CLEAR_MASKED_N],
    clear_rgba: [f32; 4],
    /// The frame load's uniform, `u_flip`: gl_FragCoord.y's pair again
    /// ([`fragcoord_y_pair`]), so that y * s + o undoes the flip the
    /// compiler applies and the fetch reads the pixel's own memory row.
    load_flip: [f32; 2],
    load_draw: GlesDraw,
    /// The GPU mipmap quad's fragment program as our compiler recorded it
    /// (`mipgen.frag`: one texture, the uniform block `u_scale`) and the
    /// block's two floats, 1 / the level's width and height
    /// ([`record_mip_level`]; its draw is [`MIP_DRAW`]).
    mip_fs_sh: ShcShader,
    mip_scale: [f32; 2],
}

/// Keep a framebuffer's depth and stencil apart when they are two
/// renderbuffers (on, measured on the AM62P: dEQP fbo 646/0 apart against
/// 632/14 packed, and gl_test 123 zsapart passing, both at 5ba5cc97 and
/// again on the identity-keyed build at 0b17df0d; am62p-board): the depth
/// in a D24S8 buffer of its own, under the depth object's identity
/// (`GlesDraw::depth_id` / `depth_gen`, [`zs_keys`]), and the stencil in a
/// buffer of its own under the stencil object's, loaded and stored through
/// CR_ISP_STENCIL_LOAD_BASE with the separate-stencil enable (cr.xml
/// ISP_STENCIL_LOAD_BASE.enable). Packed, one D24S8 buffer per (depth,
/// stencil) pair, two FBOs sharing a depth renderbuffer with stencils of
/// their own each got a fresh depth: dEQP
/// fbo.render.shared_depthbuffer.*_stencil_index8 (7) and, since the orphan
/// rule moves a deleted-but-attached stencil to another name,
/// recreate_stencilbuffer.no_rebind_*_stencil_index8 (7) on the AM62P
/// (a60104c2). The V3D's `v3d_split_zs` is the same fix. Mesa never pairs a
/// depth buffer with a separate stencil (Vulkan has one depth/stencil
/// attachment), so the stencil buffer is given a whole D24S8 buffer's bytes
/// whatever the ISP stores per sample (the ISP's separate-stencil layout is
/// still unread; the pictures agree). With the `test-knobs` feature,
/// `pvr_debug_set_split_zs(false)` packs them again: gl_test 123 runs both
/// arms. Read when a frame opens; buffers kept under the other setting are
/// not reused. A test knob, so in the `pvr_debug_*` namespace the gate reads
/// (tools/check.sh "no test knob in the artefact"), as the GC7000's
/// `gc_debug_*`.
#[cfg(feature = "test-knobs")]
#[no_mangle]
pub static mut pvr_debug_split_zs: i32 = 1;

/// Set [`pvr_debug_split_zs`]; returns the previous setting (gl_test 123's
/// `v3d_split_zs` signature).
#[cfg(feature = "test-knobs")]
#[no_mangle]
pub extern "C" fn pvr_debug_set_split_zs(on: bool) -> bool {
    let was = split_zs_on();
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(pvr_debug_split_zs).write(i32::from(on)) }
    if was != on {
        glog_once!(
            "pvr_backend: depth and stencil {} (pvr_debug_set_split_zs)",
            if on { "apart" } else { "in one buffer" }
        );
    }
    was
}

/// Are a framebuffer's depth and stencil kept apart? Always, unless a
/// `test-knobs` build has set `pvr_debug_set_split_zs(false)`.
pub fn split_zs_on() -> bool {
    #[cfg(feature = "test-knobs")]
    {
        // SAFETY: a plain read of an int only the setter writes.
        (unsafe { core::ptr::addr_of!(pvr_debug_split_zs).read() }) != 0
    }
    #[cfg(not(feature = "test-knobs"))]
    {
        true
    }
}

/// GL_LUMINANCE, GL_ALPHA and GL_LUMINANCE_ALPHA textures are stored narrow
/// (1, 1 and 2 bytes a texel) instead of expanded into RGBA8: a 2048x2048
/// L8 chain is 5.3 MiB instead of 21.3 (the AM62P's texture.size.2d
/// 2048x2048_l8_mipmap ran out of the 128 MiB carve-out), sampled with pack
/// info 0x140 (see `tex_desc_sampler`). Measured on the AM62P at 0b17df0d:
/// the 2048x2048 L8 mip chain frees 16,384 KiB less at close (26,755
/// against 43,139 KiB), and texture is 730/77 in all three arms with
/// identical failing sets; gl_test 132 passes in all three. With the
/// `test-knobs` feature, `pvr_debug_set_narrow_la` selects the arm: 0 the
/// RGBA8 store, 1 the default, 2 narrow with pack info 0x11f. Read when a
/// texture's storage is laid out: a texture keeps the layout it was created
/// with. A test knob, in the `pvr_debug_*` namespace (see
/// [`pvr_debug_split_zs`]); the GC7000's `gc_debug_narrow_la` is its twin.
#[cfg(feature = "test-knobs")]
#[no_mangle]
pub static mut pvr_debug_narrow_la: i32 = 1;

/// Set [`pvr_debug_narrow_la`]; returns the previous value (gl_test 132 runs
/// every arm and puts it back).
#[cfg(feature = "test-knobs")]
#[no_mangle]
pub extern "C" fn pvr_debug_set_narrow_la(mode: i32) -> i32 {
    let was = narrow_la_mode();
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(pvr_debug_narrow_la).write(mode) }
    was
}

/// The narrow store's arm: 1 (narrow, pack info 0x140) unless a `test-knobs`
/// build has set `pvr_debug_set_narrow_la`.
fn narrow_la_mode() -> i32 {
    #[cfg(feature = "test-knobs")]
    {
        // SAFETY: a plain read of an int only the setter writes.
        unsafe { core::ptr::addr_of!(pvr_debug_narrow_la).read() }
    }
    #[cfg(not(feature = "test-knobs"))]
    {
        1
    }
}

/// Are L8 / A8 / LA8 stored narrow?
pub fn narrow_la_on() -> bool {
    narrow_la_mode() != 0
}

/// gl_FragCoord.y's pair for draw `d`, (scale, offset): (-1, height) on a
/// target stored top-down (`flip_y`), (1, 0) on any other (see where
/// `record_draw` DMAs it). The frame load's `u_flip` is the same pair.
fn fragcoord_y_pair(d: &GlesDraw) -> [f32; 2] {
    if d.flip_y {
        [-1.0, d.target_h as f32]
    } else {
        [1.0, 0.0]
    }
}

/// The output dwords the background clear writes: one per attachment when
/// the frame's attachments are several and all 8-bit (at most
/// PVR_BACKEND_MAX_RT, three), else 0 (the single-target programs). Then the
/// clear colour is uploaded once per attachment (shared registers 0..n) and
/// the background object runs the n-dword clear program (two: the RGBA16F
/// one, which is the same moves; three: PVR_USC_CLEAR_FS3), so every
/// attachment starts from the clear colour; the one-dword program writes
/// pixel output 0 alone, and the other attachments were emitted from output
/// registers nothing wrote (gl_test 106 mrt3: the clear reached only draw
/// buffer 0). Measured on the AM62P at cece5c23 (Linux path, online pair):
/// 106 mask 0 with it, mask 6 without, 55 and the regression set 29/29 in
/// both arms. A glClearBuffer of one draw buffer (`clear_target_mask`) is
/// not honoured.
fn mrt_clear_dwords(b: &Backend) -> u32 {
    let n = b.num_targets as usize;
    if !(2..=PVR_BACKEND_MAX_RT as usize).contains(&n) {
        return 0;
    }
    let fmt = |i: usize| if i != 0 { b.targets[i].format } else { b.target_format };
    if (0..n).any(|i| gles_surf_rt_class(fmt(i)) != 0 || gles_surf_is_integer(fmt(i))) {
        return 0;
    }
    n as u32
}

/// The TA object type of a point sprite on this target: SPRITE_10UV on a
/// target stored y-up (an FBO, and a pbuffer since orientation step 1),
/// SPRITE_01UV elsewhere. gl_PointCoord's origin is the top of the WINDOW;
/// with SPRITE_01UV the iterator's t starts at the top of the STORED image,
/// which is the window only on a target stored top-down, so a y-up target
/// takes the kind that mirrors t, for every program, the table's pco
/// programs included, which a shader rewrite could not. ppp.xml's OBJTYPE
/// has three sprite kinds -- SPRITE_10UV (2), SPRITE_UV (3), SPRITE_01UV
/// (4); the board chose: dEQP-GLES2 builtin_variable.pointcoord, which
/// draws into a pbuffer, passes with SPRITE_10UV and fails with SPRITE_UV
/// and with SPRITE_01UV, and gl_test is 83/105 in all three (AM62P Linux,
/// 27ebd89a).
fn point_objtype(flip_y: bool) -> u32 {
    if !flip_y {
        PVR_TA_OBJTYPE_SPRITE_10UV
    } else {
        PVR_TA_OBJTYPE_SPRITE_01UV
    }
}

/// The `ds` keys a frame's depth and stencil live under. Off (or a window,
/// whose ids carry bit 31, or one object on both points): one packed buffer
/// per `ds_id`, as always. With `split` and an FBO whose depth is its own
/// object, the depth under that object's identity (`depth_id`, `depth_gen`:
/// a serial that follows the object through the orphan rule, and a stamp
/// that moves when its storage is specified again), shared by every FBO
/// that attaches it, and a stencil object on the other point under its own
/// (`stencil_id`, `stencil_gen`) as a separate buffer. A stencil-only FBO
/// keeps the packed buffer.
fn zs_keys(d: &GlesDraw, split: bool) -> (DsKey, Option<DsKey>) {
    let window = d.ds_id & 0x8000_0000 != 0 || d.depth_id & 0x8000_0000 != 0;
    if split && !window && d.depth_id != 0 && d.depth_id != d.stencil_id {
        let depth = DsKey { kind: DS_DEPTH, id: d.depth_id, gen: d.depth_gen };
        let stencil = (d.stencil_id != 0).then_some(DsKey { kind: DS_STENCIL, id: d.stencil_id, gen: d.stencil_gen });
        (depth, stencil)
    } else {
        (DsKey { kind: DS_PACKED, id: d.ds_id, gen: 0 }, None)
    }
}

/// The masked clear programs: internal/es/clear.frag's blend variants for a
/// colour mask (the channels left off are read back from the pixel,
/// `uses_fbfetch`), for each partial mask M = 1..=14 (bit 0 R .. 3 A, set =
/// written) -- first the RGBA8 forms, then the RGBA16F ones. The PowerVR has
/// no per-channel colour write mask in the ISP or the PBE: our compiler
/// lowers a write mask into the fragment shader (as Mesa's pvr does,
/// pvr_arch_pipeline.c, `rt->write_mask` into `blend_opts.rt[].colormask`).
/// Rows of [`PVR_INTERNAL_TABLE`] (`internal powervr clear_m*`).
const CLEAR_MASKED_N: usize = 28;
#[rustfmt::skip]
const CLEAR_MASKED: [usize; CLEAR_MASKED_N] = [
    PVR_INTERNAL_CLEAR_M1, PVR_INTERNAL_CLEAR_M2, PVR_INTERNAL_CLEAR_M3, PVR_INTERNAL_CLEAR_M4,
    PVR_INTERNAL_CLEAR_M5, PVR_INTERNAL_CLEAR_M6, PVR_INTERNAL_CLEAR_M7, PVR_INTERNAL_CLEAR_M8,
    PVR_INTERNAL_CLEAR_M9, PVR_INTERNAL_CLEAR_MA, PVR_INTERNAL_CLEAR_MB, PVR_INTERNAL_CLEAR_MC,
    PVR_INTERNAL_CLEAR_MD, PVR_INTERNAL_CLEAR_ME,
    PVR_INTERNAL_CLEAR_M1_F16X4, PVR_INTERNAL_CLEAR_M2_F16X4, PVR_INTERNAL_CLEAR_M3_F16X4,
    PVR_INTERNAL_CLEAR_M4_F16X4, PVR_INTERNAL_CLEAR_M5_F16X4, PVR_INTERNAL_CLEAR_M6_F16X4,
    PVR_INTERNAL_CLEAR_M7_F16X4, PVR_INTERNAL_CLEAR_M8_F16X4, PVR_INTERNAL_CLEAR_M9_F16X4,
    PVR_INTERNAL_CLEAR_MA_F16X4, PVR_INTERNAL_CLEAR_MB_F16X4, PVR_INTERNAL_CLEAR_MC_F16X4,
    PVR_INTERNAL_CLEAR_MD_F16X4, PVR_INTERNAL_CLEAR_ME_F16X4,
];

/// The frame load's programs by render-target class (1 RGBA16F, 2 R16F, 3
/// R32F): internal/es/fbload.frag's variant packing the texel for it.
const FBLOAD_RT: [usize; 3] = [PVR_INTERNAL_FBLOAD_F16X4, PVR_INTERNAL_FBLOAD_F16, PVR_INTERNAL_FBLOAD_F32];

/// The frame load's programs for an integer target, by the dwords its pixel
/// takes (1, 2, 4): internal/es/fbload_int.frag compiled for the R32UI,
/// RG32UI and RGBA32UI layouts, the target bound as a raw unsigned texel of
/// its size ([`int_load_texfmt`]).
const FBLOAD_INT: [usize; 3] = [PVR_INTERNAL_FBLOAD_U32, PVR_INTERNAL_FBLOAD_U32X2, PVR_INTERNAL_FBLOAD_U32X4];

/// Its index in [`FBLOAD_INT`] for a pixel of `dwords` output dwords.
const fn fbload_int_index(dwords: u32) -> usize {
    match dwords {
        1 => 0,
        2 => 1,
        _ => 2,
    }
}

/* Raw unsigned texels the integer frame load binds the target as, by its
 * pixel's bytes (pvr-private texture formats: the GL core never names them;
 * [`build_tex_desc`] decodes them): the TPU's U8, U16, U32, U32U32 and
 * U32U32U32U32 (texstate.xml) read through the load's usampler2D, so the
 * pixel's bits come back unconverted however the PBE packed them. */
const PVR_TEXFMT_RAW8: u8 = 0xf0;
const PVR_TEXFMT_RAW16: u8 = 0xf1;
const PVR_TEXFMT_RAW32: u8 = 0xf2;
const PVR_TEXFMT_RAW64: u8 = 0xf3;
const PVR_TEXFMT_RAW128: u8 = 0xf4;
/// A depth texture's depth/stencil buffer bound in place of its texels
/// (`GlesTextureBind::depth_id`, [`depth_buffer_binding`]): the ISP's
/// packed D24S8 words read as X8U24 ([`TEXF_X8U24`]). pvr-private, as the
/// raw formats above.
const PVR_TEXFMT_DEPTH_BUFFER: u8 = 0xf5;

/// The raw texel an integer surface's frame load reads it as.
fn int_load_texfmt(surf: u8) -> u8 {
    match gles_surf_bytes_per_pixel(surf) {
        1 => PVR_TEXFMT_RAW8,
        2 => PVR_TEXFMT_RAW16,
        4 => PVR_TEXFMT_RAW32,
        8 => PVR_TEXFMT_RAW64,
        _ => PVR_TEXFMT_RAW128,
    }
}

/// A raw texel's texture format and bytes ([`int_load_texfmt`]); `None`
/// for every other format.
fn raw_tex_format(format: u8) -> Option<(TexFormat, u32)> {
    let x = [SWIZ_R, SWIZ_ZERO, SWIZ_ZERO, SWIZ_ONE];
    let xy = [SWIZ_R, SWIZ_G, SWIZ_ZERO, SWIZ_ONE];
    Some(match format {
        PVR_TEXFMT_RAW8 => (tex_stride_no_alpha(csb::ROGUE_TEXSTATE_FORMAT_U8, x), 1),
        PVR_TEXFMT_RAW16 => (tex_stride_no_alpha(csb::ROGUE_TEXSTATE_FORMAT_U16, x), 2),
        PVR_TEXFMT_RAW32 => (tex_stride_no_alpha(csb::ROGUE_TEXSTATE_FORMAT_U32, x), 4),
        PVR_TEXFMT_RAW64 => (tex_stride_no_alpha(csb::ROGUE_TEXSTATE_FORMAT_U32U32, xy), 8),
        PVR_TEXFMT_RAW128 => (tex_stride(csb::ROGUE_TEXSTATE_FORMAT_U32U32U32U32, SWIZ_RGBA), 16),
        _ => return None,
    })
}

/// Row `i` of the internal table: a program our compiler made.
fn internal(i: usize) -> &'static ShcShader {
    &PVR_INTERNAL_TABLE[i].shader
}

/// The words of a program of the internal table.
fn internal_words(sh: &ShcShader) -> &'static [u32] {
    // SAFETY: a row of the generated table points at its own `[u32; usc_dw]`
    // static.
    unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) }
}

/// The clear-attach programs' temporaries: they move shared registers to
/// pixel outputs and touch none.
const CLEAR_ATTACH_TEMPS: u32 = 0;

/// Which [`CLEAR_MASKED`] program clears `mask` (written channels, 1..=14)
/// on a target of render-target class `class` (0 RGBA8, 1 RGBA16F); `None`
/// for a mask that is not partial or a class with no masked form (the
/// one-channel float targets, whose one channel is written or not).
fn clear_masked_index(class: u32, mask: u8) -> Option<usize> {
    let m = (mask & 0xf) as usize;
    if m == 0 || m == 0xf {
        return None;
    }
    match class {
        0 => Some(m - 1),
        1 => Some(14 + m - 1),
        _ => None,
    }
}

/// The masked clear program [`record_clear_quad`] draws for `mask` on a
/// target of class `class`: public so the host tests run exactly the table's
/// entry in the byte model (a table out of order fails there).
pub fn clear_masked_program(class: u32, mask: u8) -> Option<&'static ShcShader> {
    clear_masked_index(class, mask).map(|i| internal(CLEAR_MASKED[i]))
}

/* + the id DOUTWs of pvr_pds_render_vertex_program_ids */
const VDATA_DWORDS: usize = 8 * PVR_PDS_VERTEX_MAX_STREAMS as usize + 8;
const VCODE_DWORDS: usize = PVR_PDS_VERTEX_MAX_STREAMS as usize + 4;
/* iterators: W, the point coordinate, then the varyings */
/* Iterators: the varyings plus Z, W and the point coordinate. */
const CDATA_DWORDS: usize = 2 * (PVR_PDS_COEFF_MAX_VARYINGS as usize + 3) + 2;
const CCODE_DWORDS: usize = PVR_PDS_COEFF_MAX_VARYINGS as usize + 3;

/// The per-draw program scratch (function-level `static` arrays in C): the
/// backend is single-threaded and these arrays would otherwise cost ~2 KB of
/// a small RTOS stack.
struct Scratch {
    streams: [PvrPdsVertexStream; PVR_PDS_VERTEX_MAX_STREAMS as usize],
    vdata: [u32; VDATA_DWORDS],
    vcode: [u32; VCODE_DWORDS],
    cv: [PvrPdsCoeffVarying; PVR_PDS_COEFF_MAX_VARYINGS as usize],
    cdata: [u32; CDATA_DWORDS],
    ccode: [u32; CCODE_DWORDS],
    cprog: [u32; CDATA_DWORDS + CCODE_DWORDS],
    kicks: [PvrPdsDmaKick; PVR_PDS_DMA_MAX_KICKS as usize],
    ppp: [u32; PVR_PPP_BLOCK_MAX_DWORDS as usize],
}

struct Pvr {
    b: Backend,
    fbload: FbLoad,
    scratch: Scratch,
}

// All-zero is a valid Pvr: integers, floats, bools, raw pointers and arrays /
// structs of those (no references, no Option, and no enum but `DsStore`,
// whose discriminant 0 is a variant).
static PVR: Global<Pvr> = Global::zeroed();

/* ------------------------------------------------------------------------- */
fn heap_base_of(gpu: u64) -> u64 {
    if (HEAP_USC_BASE..HEAP_USC_BASE + HEAP_WINDOW).contains(&gpu) {
        return HEAP_USC_BASE;
    }
    if (HEAP_PDS_BASE..HEAP_PDS_BASE + HEAP_WINDOW).contains(&gpu) {
        return HEAP_PDS_BASE;
    }
    HEAP_GENERAL_BASE
}

/// Exec-base-relative offset of a GPU VA inside its heap.
fn heap_off_of(gpu: u64) -> u32 {
    gpu.wrapping_sub(heap_base_of(gpu)) as u32
}

fn cache_flush(m: &OsalGpuMem, off: u32, n: u32) {
    if m.cpu.is_null() || n == 0 {
        return;
    }
    // SAFETY: every caller passes a range it just wrote inside the live
    // allocation `m` (off + n <= m.size).
    unsafe { osal::cache_flush((m.cpu as *mut u8).add(off as usize) as *mut c_void, n as usize) };
}

/// Copy `src` into the CPU mapping of `m` at byte offset `off`. The C code
/// memcpy'd unchecked; a range outside the mapping (never produced by the
/// callers) is dropped instead.
fn mem_write<T: Copy>(m: &OsalGpuMem, off: u32, src: &[T]) {
    let bytes = core::mem::size_of_val(src);

    if m.cpu.is_null() || (off as usize).saturating_add(bytes) > m.size {
        return;
    }
    // SAFETY: m describes a live CPU mapping of m.size bytes and the range was
    // just checked against it; T is plain data (u8/u16/u32/f32 words).
    unsafe { core::ptr::copy_nonoverlapping(src.as_ptr() as *const u8, (m.cpu as *mut u8).add(off as usize), bytes) };
}

/// An allocation the backend writes through its CPU mapping without further
/// checks (the C code would fault on a platform that does not map it; here
/// that is an init/draw error instead).
fn alloc_mapped(heap: u64, size: usize, out: &mut OsalGpuMem) -> i32 {
    let rc = osal::gpu_alloc(heap, size, OSAL_MEM_DATA, out);

    if rc == 0 && out.cpu.is_null() {
        return -EIO; /* bring-up platforms CPU-map the carveout */
    }
    rc
}

impl Backend {
    /// Make a frame set the active one: its resources become the copies the
    /// recording code writes into.
    fn frame_set_activate(&mut self, set: usize) {
        let fs = self.sets[set];

        self.pds = fs.pds;
        self.gen = fs.gen;
        self.vdm = fs.vdm;
        self.dbias = fs.dbias;
        self.scissor = fs.scissor;
        self.pds_static = fs.pds_static;
        self.bg_color = fs.bg_color;
        self.bg_kick_off = fs.bg_kick_off;
        self.bg_dma_data_off = fs.bg_dma_data_off;
        self.bg_dma_code_off = fs.bg_dma_code_off;
        self.eot_data_off = fs.eot_data_off;
        self.eot_data_local = fs.eot_data_local;
    }

    /// The highest fence the GPU has completed (a platform without in-flight
    /// kicks reports every submitted fence as completed).
    fn completed_fence(&self) -> i64 {
        match osal::ops().and_then(|o| o.completed) {
            // SAFETY: an OSAL entry of the documented signature.
            Some(f) => unsafe { f() },
            None => self.last_fence,
        }
    }

    fn wait_fence(&mut self, fence: i64) -> i32 {
        if fence <= 0 || fence <= self.completed_fence() {
            return 0;
        }
        self.waits = self.waits.wrapping_add(1);
        let Some(wait) = osal::ops().and_then(|o| o.wait) else {
            return -ENODEV; /* pvr_backend_init() checked it */
        };
        // SAFETY: an OSAL entry of the documented signature.
        let rc = unsafe { wait(fence, WAIT_MS as i64 * 1000) };
        if rc != 0 {
            self.wait_timeouts = self.wait_timeouts.wrapping_add(1);
            glog_once!("pvr_backend: kick {} did not complete ({})", fence, rc);
            return if rc > 0 { -ETIMEDOUT } else { rc };
        }
        0
    }

    /// Option `o`'s value: 0 for a test knob in a build without
    /// `test-knobs`, so the code behind it folds away there.
    fn opt(&self, o: PvrBackendOption) -> u32 {
        if !cfg!(feature = "test-knobs") && opt_is_test_knob(o) {
            return 0;
        }
        self.opt[o as usize]
    }

    fn frame_ds(&self) -> Option<&DsBuf> {
        self.frame_ds.buf().map(|i| &self.ds[i])
    }
}

fn arena_init(a: &mut Arena, heap: u64, bytes: u32) -> i32 {
    let rc = osal::gpu_alloc(heap, bytes as usize, OSAL_MEM_DATA, &mut a.mem);

    a.bump = 0;
    rc
}

/// Internal "the open frame cannot take this draw" code: an arena, the VDM
/// stream or a per-frame table (depth bias, scissor) is full. The draw is
/// rolled back and retried once in a fresh frame; what leaves the backend is
/// -ENOMEM (see [`crate::backend::draw`]).
const EFRAME_FULL: i32 = ENOSPC;

/// The fragment stage needed more common-store DMA kicks than the program
/// holds: reported once, and the draw fails (-E2BIG) rather than run a
/// shader that reads registers nothing wrote.
fn dma_kicks_full(b: &mut Backend) -> i32 {
    if !b.warned_dma_kicks {
        b.warned_dma_kicks = true;
        glog!("pvr_backend: a draw needs more than {} common-store DMA kicks; refused", PVR_PDS_DMA_MAX_KICKS);
    }
    -E2BIG
}

/// A VDM block or the PPP state block refused a value that does not fit its
/// field (`PVR_VDM_FIELD_REFUSED`): a size, an enum, a table index, a clip
/// tile, or an address that is out of range or misaligned. The draw fails
/// (-E2BIG) with the field and the value named, instead of the hardware
/// reading a masked value (a smaller size, another enum entry, an earlier
/// address).
/// The first few are logged; every one fails its draw.
fn vdm_field_refused(why: &PvrVdmFieldRefusal) -> i32 {
    static LOGGED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
    if LOGGED.fetch_add(1, core::sync::atomic::Ordering::Relaxed) < 4 {
        // SAFETY: the emitter sets `field` to a static NUL-terminated string
        // (or leaves it null, which reads as empty).
        let name = core::str::from_utf8(unsafe { util::cstr_bytes(why.field) }).unwrap_or("?");
        if why.unit_bytes != 0 {
            glog!(
                "yakogl/pvr: draw refused: {} = {} units of {} bytes does not fit (max {})",
                name,
                why.value,
                why.unit_bytes,
                why.max
            );
        } else {
            /* a plain value: an enum, a table index, a tile, the low bits of a misaligned address */
            glog!("yakogl/pvr: draw refused: {} = {} does not fit (max {})", name, why.value, why.max);
        }
    }
    -E2BIG
}

/// Append one common-store DMA kick, or say there is no room: a kick left
/// out is a shared register the shader reads and nobody wrote, a wrong
/// picture with no error, so the caller fails the draw instead.
fn dma_push(kicks: &mut [PvrPdsDmaKick], n: &mut usize, k: PvrPdsDmaKick) -> bool {
    match kicks.get_mut(*n) {
        Some(slot) => {
            *slot = k;
            *n += 1;
            true
        }
        None => false,
    }
}

/// Copy `bytes` into the arena at the next `align`-aligned offset (`align` a
/// power of two). Reports the heap-relative offset (exec-base relative)
/// and/or the GPU VA of the copy. -[`EFRAME_FULL`] when the copy does not fit
/// (every size is checked: `bytes` comes from a C-ABI draw and the release
/// profile has no overflow checks).
///
/// # Safety
/// `src` must be valid for `bytes` bytes.
unsafe fn arena_put(
    a: &mut Arena,
    src: *const u8,
    bytes: u32,
    align: u32,
    heap_off_out: Option<&mut u32>,
    gpu_out: Option<&mut u64>,
) -> i32 {
    debug_assert!(align.is_power_of_two());
    let cause = if a.mem.gpu >= HEAP_PDS_BASE && a.mem.gpu - HEAP_PDS_BASE < HEAP_WINDOW {
        FullCause::PdsArena
    } else {
        FullCause::GenArena
    };
    let Some(off) = a.bump.checked_add(align - 1).map(|v| v & !(align - 1)) else {
        return frame_full(cause);
    };
    let Some(end) = off.checked_add(bytes) else {
        return frame_full(cause);
    };

    if end as usize > a.mem.size {
        return frame_full(cause);
    }
    if a.mem.cpu.is_null() {
        return -EIO; /* bring-up platforms CPU-map the carveout */
    }
    if bytes != 0 {
        if src.is_null() {
            return -EINVAL;
        }
        // The arena is a live mapping and off..end was just checked against it.
        core::ptr::copy_nonoverlapping(src, (a.mem.cpu as *mut u8).add(off as usize), bytes as usize);
    }
    a.bump = end;
    if let Some(o) = heap_off_out {
        /* a device offset inside the heap's window: the arena lies in it */
        *o = heap_off_of(a.mem.gpu).wrapping_add(off);
    }
    if let Some(g) = gpu_out {
        *g = a.mem.gpu.wrapping_add(off as u64);
    }
    0
}

/// [`arena_put`] of backend-built words.
fn arena_put_words(
    a: &mut Arena,
    words: &[u32],
    align: u32,
    heap_off_out: Option<&mut u32>,
    gpu_out: Option<&mut u64>,
) -> i32 {
    let Ok(bytes) = u32::try_from(core::mem::size_of_val(words)) else {
        return frame_full(FullCause::GenArena);
    };
    // SAFETY: the slice is valid for its own length.
    unsafe { arena_put(a, words.as_ptr() as *const u8, bytes, align, heap_off_out, gpu_out) }
}

/// FNV-1a over a PDS program's words: [`pds_put`]'s slot and first check.
fn pds_hash(words: &[u32]) -> u32 {
    words
        .iter()
        .fold(0x811c_9dc5u32, |h, w| w.to_le_bytes().iter().fold(h, |h, &b| (h ^ b as u32).wrapping_mul(0x0100_0193)))
}

/// Put a PDS program, or one segment of one, into the open frame's PDS
/// arena, and give its heap offset in `heap_off`; when the frame already
/// holds the same words, give their offset and upload nothing.
///
/// Most of what a draw uploads is the same for every draw of its program:
/// the fragment kick program, the coefficient program, the code segments
/// of the vertex and common-store DMA programs; only the data segments that
/// carry per-draw addresses differ. Uploaded per draw, each of them
/// PDS_ALIGN-padded, a draw of dEQP's multisample shape (two client arrays,
/// a varying, the fragment control word) took six blocks, 768 bytes, so the
/// 768 KiB arena held 1014 of them -- fewer than the draw cap
/// ([`MAX_DRAWS_PER_FRAME`]) -- and a split 4x frame reloads its resolved
/// colour into every sample (dEQP-GLES2 multisample.constancy_*, gl_test
/// 198). Shared, the same draw takes two. Mesa's pvr keeps those programs
/// with the pipeline and uploads only the per-draw data segments
/// (pvr_cmd_buffer_upload_pds_data): the hardware reads a program's code and
/// data where its state words point, so two draws may point at one.
///
/// The words are compared with what the arena holds at the remembered
/// offset, below the bump: whatever happened since (a draw's transaction
/// rolled back, the slot reused), equal words there are this program, and
/// nothing below the bump is written again in this frame.
fn pds_put(b: &mut Backend, words: &[u32], heap_off: &mut u32) -> i32 {
    let Ok(bytes) = u32::try_from(core::mem::size_of_val(words)) else {
        return frame_full(FullCause::PdsArena);
    };
    if bytes == 0 {
        return arena_put_words(&mut b.pds, words, PDS_ALIGN, Some(heap_off), None);
    }
    let hash = pds_hash(words);
    let slot = hash as usize % PDS_SEEN_SLOTS;
    let seen = b.pds_seen[slot];
    if seen.bytes == bytes
        && seen.hash == hash
        && seen.off.checked_add(bytes).is_some_and(|end| end <= b.pds.bump)
        && !b.pds.mem.cpu.is_null()
    {
        // SAFETY: off..off + bytes lies below the bump of the live, CPU-mapped
        // arena, and every offset `pds_put` remembers is PDS_ALIGN-aligned.
        let held = unsafe {
            core::slice::from_raw_parts((b.pds.mem.cpu as *const u8).add(seen.off as usize) as *const u32, words.len())
        };
        if held == words {
            *heap_off = heap_off_of(b.pds.mem.gpu).wrapping_add(seen.off);
            return 0;
        }
    }
    let mut gpu = 0u64;
    let rc = arena_put_words(&mut b.pds, words, PDS_ALIGN, Some(heap_off), Some(&mut gpu));
    if rc == 0 {
        b.pds_seen[slot] = PdsSeen { off: gpu.wrapping_sub(b.pds.mem.gpu) as u32, bytes, hash };
    }
    rc
}

/// Forget the kick-USC and coefficient programs ([`Backend::kick_once`],
/// [`Backend::coeff_once`]) whose words reach past arena offset `bump`: all
/// of them when a frame resets its PDS arena (`bump` 0), those a failed
/// draw put when its transaction gives the bytes back. Bytes below the bump
/// are not written again in the frame, so an entry below it stays true.
fn pds_once_forget(b: &mut Backend, bump: u32) {
    if bump == 0 {
        b.kick_once = [KickOnce::ZERO; PDS_ONCE_SLOTS];
        b.coeff_once = [CoeffOnce::ZERO; PDS_ONCE_SLOTS];
        (b.kick_once_next, b.coeff_once_next) = (0, 0);
        return;
    }
    for k in b.kick_once.iter_mut().filter(|k| k.end > bump) {
        *k = KickOnce::ZERO;
    }
    for c in b.coeff_once.iter_mut().filter(|c| c.end > bump) {
        *c = CoeffOnce::ZERO;
    }
}

/// The arena-relative end of `dwords` words put at PDS heap offset `off`.
fn pds_end_of(b: &Backend, off: u32, dwords: usize) -> u32 {
    off.wrapping_sub(heap_off_of(b.pds.mem.gpu)).wrapping_add(dwords as u32 * 4)
}

/// Draws generate their kick-USC and coefficient PDS programs once per
/// frame and inputs (`true`, the default) or at every draw (`false`): the
/// switch the host tests compare the two with (tests/pvr_pds_once.rs).
pub fn pvr_backend_set_pds_once(on: bool) {
    PDS_ONCE_OFF.store(!on, core::sync::atomic::Ordering::Relaxed);
}

/* ------------------------------------------------------------------------- */
#[no_mangle]
pub extern "C" fn pvr_backend_heap_base(heap: GlesHeap) -> u64 {
    match heap {
        GLES_HEAP_SHADER => HEAP_USC_BASE,
        GLES_HEAP_PROGRAM => HEAP_PDS_BASE,
        _ => HEAP_GENERAL_BASE,
    }
}

#[no_mangle]
pub extern "C" fn pvr_backend_set_option(opt: PvrBackendOption, value: u32) {
    // SAFETY: single-threaded by contract; no other reference is live.
    let b = &mut unsafe { PVR.get() }.b;

    if (opt as usize) >= PVR_BACKEND_OPT_COUNT || opt_is_retired(opt) {
        return;
    }
    if !cfg!(feature = "test-knobs") && opt_is_test_knob(opt) {
        glog_once!("pvr_backend: option {} is a test knob, not in this build (test-knobs)", opt);
        return;
    }
    b.opt[opt as usize] = value;
    /* read by the compiler, which knows nothing of this array */
    if opt == PVR_BACKEND_OPT_IF_LOWERING {
        crate::shaderc::set_if_lowering(value);
    }
    /* The per-draw dump costs ~30 console lines a draw: switching it on
     * gives it a budget of draws, so a first frame that takes seconds to
     * arrive is still dumped and a running app is not flooded. The budget
     * survives pvr_backend_init(), so switching the dump on before the first
     * GL call works (the C resets it there and prints nothing). */
    if opt == PVR_BACKEND_OPT_DEBUG && value != 0 {
        b.dbg_draws_left = DBG_DRAW_BUDGET;
    }
}

fn dump_words(tag: &str, w: &[u32]) {
    let n = w.len();
    let mut i = 0;

    while i < n {
        match n - i {
            1 => glog!("  {}[{}]={:08x}", tag, i, w[i]),
            2 => glog!("  {}[{}]={:08x} {:08x}", tag, i, w[i], w[i + 1]),
            3 => glog!("  {}[{}]={:08x} {:08x} {:08x}", tag, i, w[i], w[i + 1], w[i + 2]),
            _ => glog!("  {}[{}]={:08x} {:08x} {:08x} {:08x}", tag, i, w[i], w[i + 1], w[i + 2], w[i + 3]),
        }
        i += 4;
    }
}

/* ------------------------------------------------------------------------- *
 * init
 * ------------------------------------------------------------------------- */
/// The per-set static PDS page (background kick program, background colour
/// DMA program, EOT data slot) and the clear-colour dwords, built into the
/// active fields; `static_objects_init()` calls it once per frame set.
fn frame_set_static_init(b: &mut Backend) -> i32 {
    let mut rc = alloc_mapped(HEAP_PDS_BASE, PDS_STATIC_BYTES as usize, &mut b.pds_static);
    if rc != 0 {
        return rc;
    }
    rc = alloc_mapped(HEAP_GENERAL_BASE, 64, &mut b.bg_color);
    if rc != 0 {
        return rc;
    }

    let mut off: u32;
    let mut end: u32;
    /* Background object: kick-USC PDS [data(8) | code(1)] contiguous -> the
     * clear FS; BGRND0.shader_addr points at the DATA start. */
    {
        let mut ki = PvrPdsRenderKickUscInfo::default();
        let mut kd = [0u32; 8];
        let mut kc = [0u32; 1];

        // SAFETY: kd/kc have the sizes the generator documents (8 / 1 dwords).
        if unsafe {
            pvr_pds_render_kick_usc_program(
                heap_off_of(b.clear_fs.gpu) as u64,
                CLEAR_ATTACH_TEMPS,
                0,
                kd.as_mut_ptr(),
                kc.as_mut_ptr(),
                &mut ki,
            )
        } != 0
        {
            glog_once!("pvr_backend: the background kick program for the one-dword clear was refused");
            return -EINVAL;
        }
        /* the generator's counts, clamped to what the buffers hold (the
         * offsets below are derived from them) */
        let (mut dn, mut cn) = (ki.data_size_dw.min(kd.len() as u32), ki.code_size_dw.min(kc.len() as u32));
        off = 0;
        mem_write(&b.pds_static, off, &kd[..dn as usize]);
        mem_write(&b.pds_static, off + dn * 4, &kc[..cn as usize]);
        b.bg_kick_off = heap_off_of(b.pds_static.gpu) + off;
        end = off + (dn + cn) * 4;
        /* the same for the two-dword clear (RGBA16F targets) */
        off = (end + 63) & !63;
        // SAFETY: as above.
        if unsafe {
            pvr_pds_render_kick_usc_program(
                heap_off_of(b.clear_fs2.gpu) as u64,
                CLEAR_ATTACH_TEMPS,
                0,
                kd.as_mut_ptr(),
                kc.as_mut_ptr(),
                &mut ki,
            )
        } != 0
        {
            glog_once!("pvr_backend: the background kick program for the two-dword clear was refused");
            return -EINVAL;
        }
        (dn, cn) = (ki.data_size_dw.min(kd.len() as u32), ki.code_size_dw.min(kc.len() as u32));
        mem_write(&b.pds_static, off, &kd[..dn as usize]);
        mem_write(&b.pds_static, off + dn * 4, &kc[..cn as usize]);
        b.bg_kick2_off = heap_off_of(b.pds_static.gpu) + off;
        end = off + (dn + cn) * 4;
        /* and for three (three 8-bit attachments) */
        off = (end + 63) & !63;
        // SAFETY: as above.
        if unsafe {
            pvr_pds_render_kick_usc_program(
                heap_off_of(b.clear_fs3.gpu) as u64,
                CLEAR_ATTACH_TEMPS,
                0,
                kd.as_mut_ptr(),
                kc.as_mut_ptr(),
                &mut ki,
            )
        } != 0
        {
            glog_once!("pvr_backend: the background kick program for the three-dword clear was refused");
            return -EINVAL;
        }
        (dn, cn) = (ki.data_size_dw.min(kd.len() as u32), ki.code_size_dw.min(kc.len() as u32));
        mem_write(&b.pds_static, off, &kd[..dn as usize]);
        mem_write(&b.pds_static, off + dn * 4, &kc[..cn as usize]);
        b.bg_kick3_off = heap_off_of(b.pds_static.gpu) + off;
        end = off + (dn + cn) * 4;
    }
    /* Background colour DMA (CLEAR_ATTACH_DWORDS dwords -> common store 0..). */
    {
        let mut ci = PvrPdsRenderColorUploadInfo::default();
        let mut cd = [0u32; 3];
        let mut cc = [0u32; 2];

        // SAFETY: cd/cc have the sizes the generator documents (3 / 2 dwords).
        if unsafe {
            pvr_pds_render_color_upload_program(
                b.bg_color.gpu,
                CLEAR_ATTACH_DWORDS,
                0,
                0,
                cd.as_mut_ptr(),
                cc.as_mut_ptr(),
                &mut ci,
            )
        } != 0
        {
            glog_once!("pvr_backend: the background colour DMA program was refused");
            return -EINVAL;
        }
        let (dn, cn) = (ci.data_size_dw.min(cd.len() as u32), ci.code_size_dw.min(cc.len() as u32));
        off = (end + PDS_ALIGN - 1) & !(PDS_ALIGN - 1);
        mem_write(&b.pds_static, off, &cd[..dn as usize]);
        b.bg_dma_data_off = heap_off_of(b.pds_static.gpu) + off;
        off = (off + dn * 4 + PDS_ALIGN - 1) & !(PDS_ALIGN - 1);
        mem_write(&b.pds_static, off, &cc[..cn as usize]);
        b.bg_dma_code_off = heap_off_of(b.pds_static.gpu) + off;
        b.bg_dma_data_dw = dn;
        b.bg_dma_temps = ci.temps_used;
        end = off + cn * 4;
    }
    off = (end + PDS_ALIGN - 1) & !(PDS_ALIGN - 1);
    b.eot_data_local = off;
    b.eot_data_off = heap_off_of(b.pds_static.gpu) + off;
    0
}

/// Upload one program's words into `mem` (USC heap).
fn upload_words(mem: &mut OsalGpuMem, words: &[u32]) -> i32 {
    let bytes = core::mem::size_of_val(words);
    let rc = alloc_mapped(HEAP_USC_BASE, bytes, mem);
    if rc != 0 {
        return rc;
    }
    mem_write(mem, 0, words);
    cache_flush(mem, 0, bytes as u32);
    0
}

fn static_objects_init(b: &mut Backend, fl: &mut FbLoad) -> i32 {
    /* The background object's clear programs (our encoder's clear-attach:
     * shared register k to pixel output k, the packed colour DMAed there). */
    let mut rc = upload_words(&mut b.clear_fs2, &PVR_INTERNAL_CLEAR_ATTACH2);
    if rc == 0 {
        rc = upload_words(&mut b.clear_fs3, &PVR_INTERNAL_CLEAR_ATTACH3);
    }
    if rc == 0 {
        rc = upload_words(&mut b.clear_fs, &PVR_INTERNAL_CLEAR_ATTACH1);
    }
    if rc == 0 {
        rc = upload_words(&mut b.clear_fs4, &PVR_INTERNAL_CLEAR_ATTACH4);
    }
    if rc != 0 {
        return rc;
    }
    /* the same programs as a quad's FS: hand-built descriptors (no compiler
     * record describes a program the encoder built from no source) */
    fl.clear_fs_sh = ShcShader::ZERO;
    fl.clear_fs_sh.meta_off = SHC_NO_META;
    fl.clear_fs_sh.usc = PVR_INTERNAL_CLEAR_ATTACH1.as_ptr();
    fl.clear_fs_sh.usc_dw = PVR_INTERNAL_CLEAR_ATTACH1.len() as u32;
    fl.clear_fs_sh.temps = CLEAR_ATTACH_TEMPS as u16;
    fl.clear_fs_sh.num_outputs = 1;
    fl.clear_fs_sh.uniform_dwords = 1;
    fl.clear_fs2_sh = fl.clear_fs_sh;
    fl.clear_fs2_sh.usc = PVR_INTERNAL_CLEAR_ATTACH2.as_ptr();
    fl.clear_fs2_sh.usc_dw = PVR_INTERNAL_CLEAR_ATTACH2.len() as u32;
    fl.clear_fs2_sh.uniform_dwords = 2;
    fl.clear_fs4_sh = fl.clear_fs_sh;
    fl.clear_fs4_sh.usc = PVR_INTERNAL_CLEAR_ATTACH4.as_ptr();
    fl.clear_fs4_sh.usc_dw = PVR_INTERNAL_CLEAR_ATTACH4.len() as u32;
    fl.clear_fs4_sh.uniform_dwords = 4;
    fl.clear_fs4_sh.num_outputs = 4;

    for i in 0..b.nsets {
        rc = frame_set_static_init(b);
        if rc != 0 {
            return rc;
        }
        let fs = &mut b.sets[i];
        fs.pds_static = b.pds_static;
        fs.bg_color = b.bg_color;
        fs.bg_kick_off = b.bg_kick_off;
        fs.bg_dma_data_off = b.bg_dma_data_off;
        fs.bg_dma_code_off = b.bg_dma_code_off;
        fs.eot_data_off = b.eot_data_off;
        fs.eot_data_local = b.eot_data_local;
    }

    /* Framebuffer load: the quad's VS/FS USC and its six clip-space vertices. */
    const QUAD: [[f32; 4]; 6] = [
        [-1.0, -1.0, 0.0, 1.0],
        [1.0, -1.0, 0.0, 1.0],
        [1.0, 1.0, 0.0, 1.0],
        [-1.0, -1.0, 0.0, 1.0],
        [1.0, 1.0, 0.0, 1.0],
        [-1.0, 1.0, 0.0, 1.0],
    ];
    const QUAD_BYTES: usize = 6 * 4 * size_of::<f32>();

    rc = upload_words(&mut b.fbload_fs, internal_words(internal(PVR_INTERNAL_FBLOAD)));
    if rc == 0 {
        rc = upload_words(&mut b.fbload_vs, internal_words(internal(PVR_INTERNAL_QUAD_VS)));
    }
    if rc == 0 {
        rc = upload_words(&mut b.mipgen_fs, internal_words(internal(PVR_INTERNAL_MIPGEN)));
    }
    if rc == 0 {
        rc = alloc_mapped(HEAP_GENERAL_BASE, QUAD_BYTES, &mut b.fbload_quad);
    }
    if rc == 0 {
        rc = alloc_mapped(HEAP_GENERAL_BASE, 4, &mut b.black_tex);
    }
    if rc != 0 {
        return rc;
    }
    /* GL's incomplete texture, and what a sampler with nothing bound reads:
     * one opaque black RGBA8 texel (R=G=B=0, A=1). */
    mem_write(&b.black_tex, 0, &[0xFF00_0000u32]);
    cache_flush(&b.black_tex, 0, 4);
    for (i, &row) in FBLOAD_RT.iter().enumerate() {
        rc = upload_words(&mut b.fbload_fs_rt[i], internal_words(internal(row)));
        if rc != 0 {
            return rc;
        }
        fl.fs_rt_sh[i] = *internal(row);
    }
    for (i, &row) in FBLOAD_INT.iter().enumerate() {
        rc = upload_words(&mut b.fbload_fs_int[i], internal_words(internal(row)));
        if rc != 0 {
            return rc;
        }
        fl.fs_int_sh[i] = *internal(row);
    }
    for (i, &row) in CLEAR_MASKED.iter().enumerate() {
        rc = upload_words(&mut b.clear_mask_fs[i], internal_words(internal(row)));
        if rc != 0 {
            return rc;
        }
        fl.clear_mask_sh[i] = *internal(row);
    }
    mem_write(&b.fbload_quad, 0, &QUAD);
    cache_flush(&b.fbload_quad, 0, QUAD_BYTES as u32);
    /* The quad's two shaders as our compiler recorded them: the FS's record
     * says it reads one texture (the target, sh0..19), the flip pair
     * `u_flip` (the uniform block, sh20..21) and gl_FragCoord.y's pair
     * after it (`fragcoord_ytf_off1`), and carries no ISP-feedback meta word
     * (`meta_off` SHC_NO_META: a zeroed one would DMA SHC_FS_META_DEFAULT
     * into sh0, the texture descriptor, and the colour reload would sample
     * garbage -- gl_test cases 11/13/21 read zeros once). */
    fl.vs_sh = *internal(PVR_INTERNAL_QUAD_VS);
    fl.fs_sh = *internal(PVR_INTERNAL_FBLOAD);
    /* the GPU mipmap quad's FS: the same shape (one texture at sh0..19, a
     * two-float block after it, gl_FragCoord.y's pair after that) */
    fl.mip_fs_sh = *internal(PVR_INTERNAL_MIPGEN);
    /* EOT (pixel event) DATA segment slot; rewritten per frame with the
     * framebuffer address. The CODE lives at the static PDS offset the
     * firmware expects (PVR_RENDER_EOT_PDS_CODE_OFF, placed by the driver). */
    cache_flush(&b.pds_static, 0, PDS_STATIC_BYTES);
    0
}

#[no_mangle]
pub extern "C" fn pvr_backend_init() -> i32 {
    // SAFETY: single-threaded by contract; taken once per entry point.
    let p = unsafe { PVR.get() };
    let b = &mut p.b;

    if b.inited {
        return 0;
    }
    let Some(ops) = osal::ops() else {
        return -ENODEV;
    };
    if ops.gpu_alloc.is_none() || ops.submit.is_none() || ops.wait.is_none() {
        return -ENODEV;
    }
    {
        /* options may be set before init (and B_stats is not part of B) */
        let opt = b.opt;
        /* ... and so may PVR_BACKEND_OPT_DEBUG, which hands out the dump
         * budget: keep it, or the dump an application asks for before the
         * first GL call never prints a line (the C loses it here). */
        let dbg_left = b.dbg_draws_left;
        let (kicks, waits) = (b.kicks, b.waits);
        let wait_timeouts = b.wait_timeouts;
        let (draws, tiles) = (b.draws_total, b.tiles_total);
        let (quads, splits) = (b.quads_total, b.splits_total);
        let empty_box = b.draws_empty_box;
        let ds_refused = b.ds_refused;
        let ds_full_refused = b.ds_full_refused;
        let (ds_slots_peak, ds_bytes_peak) = (b.ds_slots_peak, b.ds_bytes_peak);
        let ds_given_up = b.ds_given_up;
        let (depth_direct, depth_refused) = (b.depth_direct, b.depth_refused);
        let arena_hw = b.arena_hw;

        // SAFETY: all-zero is a valid Backend (see PVR); b is exclusive.
        unsafe { core::ptr::write_bytes(b as *mut Backend, 0, 1) };
        b.opt = opt;
        b.dbg_draws_left = dbg_left;
        b.kicks = kicks;
        b.waits = waits;
        b.wait_timeouts = wait_timeouts;
        b.draws_total = draws;
        b.tiles_total = tiles;
        b.quads_total = quads;
        b.splits_total = splits;
        b.draws_empty_box = empty_box;
        b.ds_refused = ds_refused;
        b.ds_full_refused = ds_full_refused;
        (b.ds_slots_peak, b.ds_bytes_peak) = (ds_slots_peak, ds_bytes_peak);
        b.ds_given_up = ds_given_up;
        (b.depth_direct, b.depth_refused) = (depth_direct, depth_refused);
        b.arena_hw = arena_hw;
        /* Bring-up budgets. The C keeps these in function-local statics, so
         * they survive an init; here they are part of the state init resets,
         * which makes a run's first kicks dumped whenever the backend starts
         * and keeps the unit tests independent of their order. */
        b.mrt_log_left = MRT_LOG_BUDGET;
        b.kick_log_left = KICK_LOG_BUDGET;
        b.draw_log_left = DRAW_LOG_BUDGET;
    }

    let queried = match ops.query {
        // SAFETY: `st` is a PvrRenderStatic, what OSAL_QUERY_RENDER_STATIC fills.
        Some(query) => unsafe {
            query(
                OSAL_QUERY_RENDER_STATIC,
                &mut b.st as *mut PvrRenderStatic as *mut c_void,
                size_of::<PvrRenderStatic>(),
            ) == 0
        },
        None => false,
    };
    if !queried {
        glog!("pvr_backend: platform has no render service");
        return -ENODEV;
    }
    if b.st.heap_base[PVR_RENDER_HEAP_GENERAL] != HEAP_GENERAL_BASE
        || b.st.heap_base[PVR_RENDER_HEAP_PDS] != HEAP_PDS_BASE
        || b.st.heap_base[PVR_RENDER_HEAP_USC] != HEAP_USC_BASE
    {
        glog!(
            "pvr_backend: heap layout mismatch with the driver: general 0x{:x} pds 0x{:x} usc 0x{:x} (expected 0x{:x} 0x{:x} 0x{:x})",
            b.st.heap_base[PVR_RENDER_HEAP_GENERAL],
            b.st.heap_base[PVR_RENDER_HEAP_PDS],
            b.st.heap_base[PVR_RENDER_HEAP_USC],
            HEAP_GENERAL_BASE,
            HEAP_PDS_BASE,
            HEAP_USC_BASE
        );
        return -EINVAL;
    }

    /* (the C code reads `rc` uninitialised in this loop's condition; every
     * build that works starts it at 0) */
    let mut rc = 0;
    // SAFETY: single-threaded by contract; no other reference is live.
    let cfg = unsafe { *ARENA_CFG.get() };
    b.nsets = cfg.sets;
    for i in 0..b.nsets {
        if rc != 0 {
            break;
        }
        let fs = &mut b.sets[i];

        rc = arena_init(&mut fs.pds, HEAP_PDS_BASE, cfg.pds_bytes);
        if rc == 0 {
            rc = arena_init(&mut fs.gen, HEAP_GENERAL_BASE, cfg.gen_bytes);
        }
        if rc == 0 {
            rc = alloc_mapped(HEAP_GENERAL_BASE, VDM_STREAM_BYTES as usize, &mut fs.vdm);
        }
        if rc == 0 {
            rc = osal::gpu_alloc(HEAP_GENERAL_BASE, (DBIAS_MAX_ENTRIES * 12) as usize, OSAL_MEM_DATA, &mut fs.dbias);
        }
        if rc == 0 {
            rc = osal::gpu_alloc(HEAP_GENERAL_BASE, (SCISSOR_MAX_ENTRIES * 8) as usize, OSAL_MEM_DATA, &mut fs.scissor);
        }
    }
    if rc == 0 {
        rc = static_objects_init(b, &mut p.fbload);
    }
    if rc == 0 {
        b.cur = 0;
        b.frame_set_activate(0);
    }
    if rc != 0 {
        glog!("pvr_backend: init failed ({})", rc);
        return rc;
    }
    b.inited = true;
    glog!(
        "pvr_backend: init ok (pds arena off 0x{:x}, gen arena 0x{:x}, vdm 0x{:x}, clear_fs off 0x{:x})",
        heap_off_of(b.pds.mem.gpu),
        b.gen.mem.gpu,
        b.vdm.gpu,
        heap_off_of(b.clear_fs.gpu)
    );
    0
}

#[no_mangle]
pub extern "C" fn pvr_backend_shutdown() {
    // SAFETY: single-threaded by contract; no other reference is live.
    let p = unsafe { PVR.get() };
    if !p.b.inited {
        return;
    }
    /* A frame still being recorded is submitted, as a finish would, and the
     * last kick is waited for: the GPU may read every block below until it
     * completes. If it does not, the memory is kept (freeing what the GPU
     * still reads would be worse than losing it) and the line says so. */
    if p.b.frame_open && flush(p) != 0 {
        glog!("pvr_backend: shutdown: the open frame did not submit; dropped");
    }
    let b = &mut p.b;
    let last = b.last_fence;
    if b.wait_fence(last) != 0 {
        glog!("pvr_backend: shutdown: kick {} did not complete: the backend's pool memory is kept", last);
        b.inited = false;
        return;
    }
    /* Everything pvr_backend_init() and the draws allocated (the owner
     * report's backend lines name the same set): the frame sets in use, the
     * persistent programs and tables, the depth/stencil buffers. The
     * "active" fields (pds, gen, vdm, ...) are copies of a set's and are not
     * freed twice. */
    for i in 0..b.nsets {
        let fs = &mut b.sets[i];
        for m in [
            &mut fs.pds.mem,
            &mut fs.gen.mem,
            &mut fs.vdm,
            &mut fs.dbias,
            &mut fs.scissor,
            &mut fs.pds_static,
            &mut fs.bg_color,
        ] {
            osal::gpu_free(m);
        }
    }
    for m in [
        &mut b.clear_fs,
        &mut b.clear_fs2,
        &mut b.clear_fs3,
        &mut b.clear_fs4,
        &mut b.fbload_fs,
        &mut b.fbload_vs,
        &mut b.mipgen_fs,
        &mut b.fbload_quad,
        &mut b.black_tex,
        &mut b.seq_idx,
    ] {
        osal::gpu_free(m);
    }
    for m in b.fbload_fs_rt.iter_mut().chain(b.fbload_fs_int.iter_mut()).chain(b.clear_mask_fs.iter_mut()) {
        osal::gpu_free(m);
    }
    for s in b.ds.iter_mut() {
        osal::gpu_free(&mut s.mem);
        *s = DsBuf::ZERO;
    }
    osal::gpu_free(&mut b.ms.mem);
    b.ms = MsStore::ZERO;
    if !b.vis.impl_.is_null() {
        osal::gpu_free(&mut b.vis);
    }
    b.vis = OsalGpuMem::ZERO;
    (b.vis_frame, b.vis_fence, b.vis_mask, b.vis_lost) = (0, [0; VIS_SLOTS], [0; VIS_SLOTS], 0);
    b.vis_sum = [0; GLES_QUERY_HANDLES as usize];
    b.inited = false;
}

/* ------------------------------------------------------------------------- *
 * occlusion queries (docs/es3/occlusion-query.md 3.2)
 *
 * A draw carrying a query handle is a TRANSLUCENT object with
 * TA_STATE_ISPCTL.vistest and visreg = the handle; a kick with such draws
 * names a zeroed result area of its own as CR_ISP_OCLQRY_BASE and sets
 * FRAG_FLAGS_GETVISRESULTS; once the kick has completed the area's dwords
 * are added to the handles' sums on the CPU. Whether the ISP adds into a
 * dword or overwrites it is not known (Mesa keeps one register per query
 * across renders); an area per kick makes the answer the same either way.
 * The GL core ends the frame at glEndQuery through `query_end`: an occluder
 * recorded after the query in the same kick would otherwise hide the
 * query's draws in the ISP.
 * ------------------------------------------------------------------------- */

/// Add the counts of every completed kick's result area to its handles'
/// sums, and free the area.
fn vis_fold(b: &mut Backend) {
    let done = b.completed_fence();
    for i in 0..VIS_SLOTS {
        if b.vis_fence[i] == 0 || b.vis_fence[i] > done || b.vis.cpu.is_null() {
            continue;
        }
        let at = (b.vis.cpu as *mut u8).wrapping_add(i * VIS_SLOT_BYTES) as *mut u32;
        // SAFETY: the area is inside the live mapping; its kick completed.
        unsafe { osal::cache_invd(at as *mut c_void, VIS_SLOT_BYTES) };
        let mut m = b.vis_mask[i];
        while m != 0 {
            let h = m.trailing_zeros() as usize;
            m &= m - 1;
            // SAFETY: h < GLES_QUERY_HANDLES dwords into the area.
            let n = unsafe { at.add(h).read_volatile() };
            b.vis_sum[h] = b.vis_sum[h].wrapping_add(u64::from(n));
        }
        (b.vis_fence[i], b.vis_mask[i]) = (0, 0);
    }
}

/// A free result area for the kick being submitted: one whose kick has
/// completed (folded first), else the oldest one, after waiting for it.
fn vis_take_slot(b: &mut Backend) -> Option<usize> {
    if b.vis.cpu.is_null() {
        return None;
    }
    vis_fold(b);
    if let Some(i) = (0..VIS_SLOTS).find(|&i| b.vis_fence[i] == 0) {
        return Some(i);
    }
    let oldest = (0..VIS_SLOTS).min_by_key(|&i| b.vis_fence[i])?;
    if b.wait_fence(b.vis_fence[oldest]) != 0 {
        return None;
    }
    vis_fold(b);
    (b.vis_fence[oldest] == 0).then_some(oldest)
}

/// `YakoglBackend::query_begin`: a query starts on `handle`. Every kick that
/// counted on it has completed (the GL core waited), so its areas fold into
/// the old sum before the sum is reset.
#[no_mangle]
pub extern "C" fn pvr_backend_query_begin(handle: u32) -> i32 {
    // SAFETY: single-threaded by contract; taken once per entry point.
    let b = unsafe { &mut PVR.get().b };
    if !b.inited {
        return -ENODEV;
    }
    if handle == 0 || handle >= GLES_QUERY_HANDLES {
        return -EINVAL;
    }
    if b.vis.impl_.is_null() && alloc_mapped(HEAP_GENERAL_BASE, VIS_SLOTS * VIS_SLOT_BYTES, &mut b.vis) != 0 {
        if !b.vis.impl_.is_null() {
            osal::gpu_free(&mut b.vis);
        }
        b.vis = OsalGpuMem::ZERO;
        glog_once!("pvr_backend: no GPU memory for the visibility results ({} bytes)", VIS_SLOTS * VIS_SLOT_BYTES);
        return -ENOMEM;
    }
    vis_fold(b);
    let bit = 1u64 << handle;
    b.vis_sum[handle as usize] = 0;
    b.vis_lost &= !bit;
    0
}

/// `YakoglBackend::query_end`: the query on `handle` ended. A frame that
/// holds its draws is submitted now, so nothing recorded after glEndQuery
/// shares their kick (the TBDR ordering hazard, 3.2).
#[no_mangle]
pub extern "C" fn pvr_backend_query_end(handle: u32) {
    // SAFETY: single-threaded by contract; taken once per entry point.
    let p = unsafe { PVR.get() };
    if p.b.inited && p.b.frame_open && handle < GLES_QUERY_HANDLES && p.b.vis_frame & (1u64 << handle) != 0 {
        let _ = flush(p); /* a failed submit is counted by the kick's own result: lost */
    }
}

/// `YakoglBackend::query_result`: the samples counted on `handle` (a
/// non-exact count, Mesa's occlusionQueryPrecise = false; any non-zero value
/// is "passed"), every kick that counted on it completed.
///
/// # Safety
/// `samples` is null or valid for writing one u64.
#[no_mangle]
pub unsafe extern "C" fn pvr_backend_query_result(handle: u32, samples: *mut u64) -> i32 {
    // SAFETY: single-threaded by contract; taken once per entry point.
    let b = unsafe { &mut PVR.get().b };
    if !b.inited || samples.is_null() || handle == 0 || handle >= GLES_QUERY_HANDLES {
        return -EINVAL;
    }
    vis_fold(b);
    let n = b.vis_sum[handle as usize];
    let lost = b.vis_lost & (1u64 << handle) != 0;
    // SAFETY: the caller's u64.
    unsafe { *samples = if lost { n.max(1) } else { n } };
    0
}

/* ------------------------------------------------------------------------- *
 * helpers
 * ------------------------------------------------------------------------- */
/// float -> IEEE half bits (round toward zero, overflow to infinity).
fn f32_to_f16_bits(f: f32) -> u32 {
    let x = f.to_bits();
    let sign = (x >> 16) & 0x8000;
    let exp = (x >> 23) & 0xff;
    let mut mant = x & 0x7f_ffff;

    if exp == 0xff {
        return sign | 0x7c00 | if mant != 0 { 0x200 } else { 0 };
    }
    if exp > 142 {
        return sign | 0x7c00;
    }
    if exp < 103 {
        return sign;
    }
    if exp < 113 {
        mant |= 0x80_0000;
        return sign | (mant >> (126 - exp));
    }
    sign | ((exp - 112) << 10) | (mant >> 13)
}

/// The clear colour as the clear FS's output dwords for a float target:
/// RGBA16F = four halves in two dwords, R16F = one half, R32F = the float.
/// The clear colour as the clear fragment shader receives it.
///
/// For an ordinary target that is RGBA8 in the pixel's output-register order
/// -- the clear FS writes those bytes as its RGBA output and the PBE swizzle
/// puts them in the surface's memory layout on emit -- and for a float
/// render-target class it is the float encoding.
///
/// One function because there were three callers and they disagreed. The
/// background object packed with `pack_rgba8` and used the float encoding
/// only for a float class, which is right; `record_clear_quad` called
/// `pack_clear_float` unconditionally, and its fallthrough -- correct for
/// R32F, where the red channel IS the pixel -- handed an RGBA8888 target the
/// raw float bits of the red channel. A scissored clear therefore wrote
/// 0x3f800000 where it meant opaque red, which is what gl_test case 58's
/// "the clear is after the draw" check reads on the board and what dEQP-EGL's
/// twelve color_clears failures are.
///
/// For an integer target the four values are the raw integers of
/// glClearBuffer{i,ui}v (`GlesDraw::clear_rgba`'s bit patterns), packed into
/// the target's dwords by [`pack_clear_int`]. Zero past the dwords the
/// target takes ([`surf_out_dwords`]).
///
/// An sRGB target takes the linear colour as four halves, clamped to [0, 1]
/// (the target is unsigned normalised, ES 3.0.6 4.2.3), and the PBE encodes
/// it on store -- Mesa packs an sRGB clear colour the same way, as
/// F16F16F16F16 for the gamma'd PBE (pvr_arch_job_transfer.c:2372-2384).
fn clear_uniform(surf: u8, c: &[f32; 4]) -> [u32; 4] {
    if let Some(l) = gles_surf_int_layout(surf) {
        pack_clear_int(l, &c.map(f32::to_bits))
    } else if gles_surf_rt_class(surf) != 0 {
        let [a, b] = pack_clear_float(surf, c);
        [a, b, 0, 0]
    } else {
        [pack_rgba8(c), 0, 0, 0]
    }
}

/// An integer target's clear: Mesa's pvr_get_hw_clear_color
/// (vulkan/pvr_formats.c:145-218) for its accumulation format -- the low
/// byte (UINT8/SINT8, :177-182) or half-word (UINT16/SINT16, :193-198) of
/// each value side by side, a 32-bit value as it is (UINT32/SINT32,
/// :210-215), RGB10_A2UI as one 10/10/10/2 dword
/// (pvr_pack_a2x10y10z10_uint, :103-121) -- which is the program's own
/// packing of an integer output (`powervr::chain::pack_int_output`), so the
/// clear program moves the dwords to the pixel outputs as a draw would
/// write them. Only the target's channels: the rest of each dword is 0.
fn pack_clear_int(l: GlesIntLayout, v: &[u32; 4]) -> [u32; 4] {
    let low = |x: u32, n: u32| if n >= 32 { x } else { x & ((1 << n) - 1) };
    let mut out = [0u32; 4];
    for c in 0..l.channels as usize {
        let (dword, offset, width) = match l.bits {
            32 => (c, 0, 32),
            16 => (c / 2, 16 * (c % 2) as u32, 16),
            8 => (0, 8 * c as u32, 8),
            _ => (0, 10 * c as u32, if c == 3 { 2 } else { 10 }),
        };
        out[dword] |= low(v[c], width) << offset;
    }
    out
}

/// The float encoding of a clear colour for a float render-target class
/// (and an sRGB target's linear halves, clamped: [`clear_colour`]). Only
/// reached for those: [`clear_uniform`] is the way in.
fn pack_clear_float(surf: u8, c: &[f32; 4]) -> [u32; 2] {
    let c = &clear_colour(surf, c);
    if surf == GLES_SURF_RGBA16F || gles_surf_is_srgb(surf) {
        [f32_to_f16_bits(c[0]) | (f32_to_f16_bits(c[1]) << 16), f32_to_f16_bits(c[2]) | (f32_to_f16_bits(c[3]) << 16)]
    } else if surf == GLES_SURF_R16F {
        [f32_to_f16_bits(c[0]), 0]
    } else {
        [c[0].to_bits(), 0]
    }
}

/// The clear colour a program writes into a target of format `surf`: as
/// given, but clamped to [0, 1] for an sRGB target, which is unsigned
/// normalised (ES 3.0.6 4.2.3) and whose F16 accumulation would keep a value
/// past 1 for the next blend to read. (An RGBA8 target's bytes clamp in
/// `pack_rgba8`; a float target keeps the value.) NaN clamps to 0.
fn clear_colour(surf: u8, c: &[f32; 4]) -> [f32; 4] {
    if gles_surf_is_srgb(surf) {
        c.map(|v| {
            if v >= 1.0 {
                1.0
            } else if v > 0.0 {
                v
            } else {
                0.0
            }
        })
    } else {
        *c
    }
}

/// A `ShcShader::vs_*_id_reg1` field (register + 1, 0 = not read) as the
/// vertex program generator's register argument.
fn id_reg_of(reg1: u8) -> u32 {
    if reg1 != 0 {
        reg1 as u32 - 1
    } else {
        PVR_PDS_NO_ID_REG
    }
}

fn pack_rgba8(c: &[f32; 4]) -> u32 {
    let mut v = 0u32;

    for (i, &ch) in c.iter().enumerate() {
        let mut f = ch;

        if f < 0.0 {
            f = 0.0;
        }
        if f > 1.0 {
            f = 1.0;
        }
        let u = (f * 255.0 + 0.5) as i32;
        v |= (u as u32 & 0xff) << (8 * i); /* memory order R,G,B,A (LE 0xAABBGGRR) */
    }
    v
}

fn fbits(f: f32) -> u32 {
    f.to_bits()
}

fn map_cmp(c: GlesCmp) -> u32 {
    /* GlesCmp is in TA_CMPMODE order (NEVER..ALWAYS). */
    c & 7
}

fn seq_indices_ensure(b: &mut Backend, count: u32) -> i32 {
    let want = if count < 1024 { 1024 } else { count };

    if !b.seq_idx.cpu.is_null() && b.seq_idx_count >= count {
        return 0;
    }
    if count > 0xFFFF {
        return -EINVAL; /* u16 indices */
    }
    let mut fresh = OsalGpuMem::ZERO;
    if alloc_mapped(HEAP_GENERAL_BASE, (want * 2) as usize, &mut fresh) != 0 {
        osal::gpu_free(&mut fresh);
        return -ENOMEM;
    }
    /* the smaller one goes back: the OSAL holds it until the kicks that may
     * still read it (the open frame's, those in flight) have completed */
    osal::gpu_free(&mut b.seq_idx);
    b.seq_idx = fresh;
    // SAFETY: a fresh, CPU-mapped (alloc_mapped), allocator-aligned block of
    // want * 2 bytes that nothing else references yet.
    let p = unsafe { core::slice::from_raw_parts_mut(b.seq_idx.cpu as *mut u16, want as usize) };
    for (i, v) in p.iter_mut().enumerate() {
        *v = i as u16;
    }
    cache_flush(&b.seq_idx, 0, want * 2);
    b.seq_idx_count = want;
    0
}

/// The clear-attach program that moves `dwords` packed colour dwords from
/// the uniform block to the pixel outputs ([`clear_uniform`]): its USC
/// address and its descriptor. One dword for the 8-bit targets, R16F and
/// R32F, two for RGBA16F, an integer target's one, two or four.
fn clear_program(b: &Backend, fl: &FbLoad, dwords: u32) -> (u64, *const ShcShader) {
    match dwords {
        2 => (b.clear_fs2.gpu, &fl.clear_fs2_sh),
        4 => (b.clear_fs4.gpu, &fl.clear_fs4_sh),
        _ => (b.clear_fs.gpu, &fl.clear_fs_sh),
    }
}

/// A draw into several sRGB attachments as this back end draws it: into
/// RGBA8 attachments whose bytes are already encoded. The fragment shader
/// encodes its outputs ([`shaderc::SHC_BLEND_KEY_SRGB_ENCODED`]: one output
/// register each, where the halves of an sRGB attachment take two and three
/// would not fit the pixel's four); the clear colour's red, green and blue
/// are encoded here, to the byte the curve gives (`gl::srgb::encode`, the
/// GL core's codec), which the RGBA8 pack keeps exactly. From here on the
/// frame, its load (the bytes read back raw and stored as they were), its
/// clear and its end-of-tile emit (U8 sources, no PBE gamma) are the RGBA8
/// several-attachment path's.
fn srgb_targets_as_rgba8(d: &GlesDraw) -> GlesDraw {
    let mut e = *d;
    let n = (e.num_targets as usize).min(e.targets.len());
    for t in e.targets[..n].iter_mut().filter(|t| gles_surf_is_srgb(t.format)) {
        t.format = GLES_SURF_RGBA8888;
    }
    if gles_surf_is_srgb(e.target_format) {
        e.target_format = GLES_SURF_RGBA8888;
    }
    for c in e.clear_rgba[..3].iter_mut() {
        *c = f32::from(crate::gl::srgb::encode(*c)) / 255.0;
    }
    e
}

/// Is a colour clear of this surface drawn as a quad through the clear
/// program, rather than left to the background object? The float classes
/// (whose pixels the background object leaves untouched on silicon: gl_test
/// case 44) and the integer surfaces, whose clear is raw bits the
/// background program and the USC clear register were never measured with.
fn clears_by_quad(surf: u8) -> bool {
    gles_surf_rt_class(surf) != 0 || gles_surf_is_integer(surf)
}

/// The frame's first object: a full-screen quad that (a) loads the target's
/// colour when the frame does not clear it -- its FS (fbload.frag) fetches
/// the previous texel under every pixel, so earlier kicks into this target
/// survive (frames the GL core splits and apps that draw incrementally;
/// Mesa's index-list load) -- and/or (b) writes the depth/stencil clear
/// values into every pixel. (b) is needed because the ISP stores only the
/// pixels some object covered: uncovered pixels test against the background
/// depth fine, but the Z store writes 0 for them and the next kick would load
/// that (seen on silicon: gl_test case 13). Mesa clears attachments at the
/// start of a render the same way (pvr_perform_start_of_render_clears).
fn record_frame_quad(
    p: &mut Pvr,
    d: &GlesDraw,
    load_color: bool,
    init_depth: bool,
    init_stencil: bool,
    clear_float: bool,
) -> i32 {
    let b = &mut p.b;
    let fl = &mut p.fbload;
    let (vs_sh, fs_sh): (*const ShcShader, *const ShcShader) = (&fl.vs_sh, &fl.fs_sh);
    let rt_class = pixel_class(d.target_format);
    let rt_sh: *const ShcShader = if rt_class != 0 { &fl.fs_rt_sh[rt_class as usize - 1] } else { core::ptr::null() };
    let int = gles_surf_is_integer(d.target_format);
    let dwords = surf_out_dwords(d.target_format);
    let int_sh: *const ShcShader = &fl.fs_int_sh[fbload_int_index(dwords)];
    let (clear_gpu, clear_sh) = clear_program(b, fl, dwords);
    if clear_float {
        fl.clear_pack = clear_uniform(d.target_format, &b.clear_rgba);
    }
    let clear_pack: *const u32 = fl.clear_pack.as_ptr();
    let ld = &mut fl.load_draw;

    *ld = GlesDraw::ZERO;
    ld.target_gpu = d.target_gpu;
    ld.target_w = d.target_w;
    ld.target_h = d.target_h;
    ld.target_format = d.target_format;
    ld.prim = GLES_PRIM_TRIANGLES;
    ld.index_count = 6;
    ld.vs = vs_sh;
    ld.fs = fs_sh;
    ld.vs_usc_gpu = b.fbload_vs.gpu;
    ld.fs_usc_gpu = b.fbload_fs.gpu;
    if rt_class != 0 {
        /* a float target: the load FS packs the texel for its class (an
         * sRGB one's as RGBA16F's, the texture state decoding it,
         * `gles_surf_texfmt`) */
        ld.fs = rt_sh;
        ld.fs_usc_gpu = b.fbload_fs_rt[rt_class as usize - 1].gpu;
    }
    if int {
        /* an integer target: the load FS writes the pixel's bits back as
         * its dwords, read raw (`int_load_texfmt`, below) */
        ld.fs = int_sh;
        ld.fs_usc_gpu = b.fbload_fs_int[fbload_int_index(dwords)].gpu;
    }
    ld.attribs[0].vi_reg = 0;
    ld.attribs[0].components = 4;
    ld.attribs[0].stride = 16;
    ld.attribs[0].buffer_gpu = b.fbload_quad.gpu;
    ld.num_attribs = 1;
    ld.vs_output_size_dwords = 4;
    ld.textures[0].unit = 0;
    ld.textures[0].base_gpu = d.target_gpu;
    ld.textures[0].width = d.target_w;
    ld.textures[0].height = d.target_h;
    /* the target's own pitch: tex_pitch(w), or a view's wider one */
    ld.textures[0].stride_texels = crate::backend::draw_pitch(d);
    ld.target_pitch = d.target_pitch;
    ld.textures[0].wrap_s = GLES_WRAP_CLAMP_TO_EDGE;
    ld.textures[0].wrap_t = GLES_WRAP_CLAMP_TO_EDGE;
    ld.textures[0].mip_levels = 1;
    ld.textures[0].format = if int { int_load_texfmt(d.target_format) } else { gles_surf_texfmt(d.target_format) };
    ld.textures[0].bgra = if d.target_format == GLES_SURF_XRGB8888 { 1 } else { 0 };
    /* A multisampled frame going on from the kick before: the samples that
     * kick stored, not the resolved colour -- the store bound as the target's
     * multisampled image ([`MsStore`]; the descriptor's `smpcnt` is set
     * where it is built, `build_desc_block`), read by the same program at
     * the full sample rate (`loads_samples`), each instance its own sample. */
    if load_color && b.ms.holds(d) {
        ld.textures[0].base_gpu = b.ms.mem.gpu;
    }
    ld.num_textures = 1;
    ld.viewport[2] = d.target_w as i32;
    ld.viewport[3] = d.target_h as i32;
    ld.flip_y = d.flip_y;
    /* the load FS's uniform block: u_flip (fbload.frag) */
    fl.load_flip = fragcoord_y_pair(ld);
    ld.fs_uniform_cpu = fl.load_flip.as_ptr() as *const c_void;
    ld.fs_uniform_dwords = 2;
    ld.color_write = load_color; /* else the ISP writes depth/stencil only */
    ld.line_width = 1.0;
    /* the quad's z is 0 in NDC: a collapsed depth range lands it exactly on
     * the clear depth */
    ld.depth_func = GLES_CMP_ALWAYS;
    if init_depth {
        ld.depth_test = true;
        ld.depth_write = true;
        ld.depth_range = [b.clear_depth, b.clear_depth];
    } else {
        ld.depth_range[1] = 1.0;
    }
    if init_stencil {
        ld.stencil = true;
        ld.stencil_func = GLES_CMP_ALWAYS;
        ld.stencil_back_func = GLES_CMP_ALWAYS;
        ld.stencil_ref = b.clear_stencil;
        ld.stencil_back_ref = b.clear_stencil;
        ld.stencil_cmp_mask = 0xff;
        ld.stencil_back_cmp_mask = 0xff;
        ld.stencil_write_mask = 0xff;
        ld.stencil_back_write_mask = 0xff;
        ld.stencil_sfail = GLES_SOP_REPLACE;
        ld.stencil_dpfail = GLES_SOP_REPLACE;
        ld.stencil_dppass = GLES_SOP_REPLACE;
        ld.stencil_back_sfail = GLES_SOP_REPLACE;
        ld.stencil_back_dpfail = GLES_SOP_REPLACE;
        ld.stencil_back_dppass = GLES_SOP_REPLACE;
    }
    if !load_color && !clear_float {
        /* Depth/stencil only: the colour write is masked, so the quad has no
         * reason to sample the target -- a full-screen texture fetch per kick
         * whose result nothing keeps. The one-dword clear shader is the
         * cheapest program the driver has. */
        ld.fs = &fl.clear_fs_sh;
        ld.fs_usc_gpu = b.clear_fs.gpu;
        ld.num_textures = 0;
        ld.fs_uniform_cpu = clear_pack as *const c_void;
        ld.fs_uniform_dwords = 1;
    }
    if clear_float {
        /* the clear colour through the clear USC, as an ordinary draw: the
         * target's packed dwords moved to its pixel outputs */
        ld.fs = clear_sh;
        ld.fs_usc_gpu = clear_gpu;
        ld.num_textures = 0;
        ld.fs_uniform_cpu = clear_pack as *const c_void;
        ld.fs_uniform_dwords = dwords as u16;
        ld.color_write = true;
    }
    if record_draw_txn(b, &mut p.scratch, &fl.load_draw, 0, 6, None) != 0 {
        if !b.warned_load {
            glog!("pvr_backend: could not record the frame's load/clear quad");
        }
        b.warned_load = true;
        return -1;
    }
    b.frame_quads += 1;
    0
}

/// A glClear the frame's background object cannot carry out: one with the
/// scissor on, or one that must leave some stencil bits alone. The ISP
/// background object covers the whole target and has no scissor and no write
/// masks, so the clear becomes a full-screen quad drawn in sequence with the
/// frame's other objects -- the same quad [`record_frame_quad`] builds, with
/// this draw's scissor, colour write enable, depth write and stencil write
/// mask.
///
/// The colour comes from the clear shader (a constant from a uniform), so a
/// per-channel colour mask (`d.clear_color_mask`, the GL mask captured at
/// the glClear -- never `d.color_mask`, which is the next draw's) is applied
/// by drawing with the
/// masked clear program for it ([`CLEAR_MASKED`]): glColorMask is lowered
/// into the fragment shader on this GPU, as Mesa's pvr lowers a write mask.
/// Measured on the AM62P (Zephyr, f9f4896f): dEQP color_clear 19/19 and the
/// gl_test regression set plus 102, 103 and 107 32/32 masked, 30/32 with
/// every channel cleared (am62p-board).
fn record_clear_quad(p: &mut Pvr, d: &GlesDraw) -> i32 {
    let has_ds = p.b.frame_ds.buf().is_some();
    let class = pixel_class(d.target_format);
    let mut mask = if d.do_clear { d.clear_color_mask & 0xf } else { 0 };
    /* A one-channel float target has only red: it is written or it is not. */
    if class >= 2 {
        mask = if mask & 1 != 0 { 0xf } else { 0 };
    }
    /* An integer target: its own channels written or none. A mask that
     * keeps some of them would need the pixel's other channels kept, which
     * the masked clear programs do for the float classes only (they convert
     * what they read): refused by name. */
    if let Some(l) = gles_surf_int_layout(d.target_format) {
        let own = (1u8 << l.channels) - 1;
        mask = match mask & own {
            0 => 0,
            m if m == own => 0xf,
            _ => {
                glog_once!(
                    "yakogl/pvr: clear refused: a colour mask that keeps some channels of a {} target",
                    gles_surf_name(d.target_format)
                );
                return -util::ENOTSUP;
            }
        };
    }
    let color = d.do_clear && mask != 0;
    let depth = d.do_clear_depth && has_ds;
    let stencil = d.do_clear_stencil && has_ds;

    if !color && !depth && !stencil {
        return 0;
    }
    let b = &mut p.b;
    let fl = &mut p.fbload;
    let dwords = surf_out_dwords(d.target_format);
    let masked = if gles_surf_is_integer(d.target_format) { None } else { clear_masked_index(class, mask) };
    let (clear_gpu, clear_one) = clear_program(b, fl, dwords);
    let clear_sh: *const ShcShader = match masked {
        Some(i) => &fl.clear_mask_sh[i],
        None => clear_one,
    };
    fl.clear_pack = clear_uniform(d.target_format, &d.clear_rgba);
    fl.clear_rgba = clear_colour(d.target_format, &d.clear_rgba);
    let clear_pack: *const c_void = match masked {
        /* clear.frag's uniform: the colour as four floats */
        Some(_) => fl.clear_rgba.as_ptr() as *const c_void,
        None => fl.clear_pack.as_ptr() as *const c_void,
    };
    let vs_sh: *const ShcShader = &fl.vs_sh;
    let ld = &mut fl.load_draw;

    *ld = GlesDraw::ZERO;
    ld.target_gpu = d.target_gpu;
    ld.target_w = d.target_w;
    ld.target_h = d.target_h;
    ld.target_pitch = d.target_pitch;
    ld.target_format = d.target_format;
    ld.samples = d.samples;
    ld.prim = GLES_PRIM_TRIANGLES;
    ld.index_count = 6;
    ld.vs = vs_sh;
    ld.vs_usc_gpu = b.fbload_vs.gpu;
    ld.attribs[0].vi_reg = 0;
    ld.attribs[0].components = 4;
    ld.attribs[0].stride = 16;
    ld.attribs[0].buffer_gpu = b.fbload_quad.gpu;
    ld.num_attribs = 1;
    ld.vs_output_size_dwords = 4;
    ld.viewport[2] = d.target_w as i32;
    ld.viewport[3] = d.target_h as i32;
    ld.flip_y = d.flip_y;
    ld.line_width = 1.0;
    ld.scissor = d.scissor;
    ld.scissor_rect = d.scissor_rect;

    /* the clear colour, packed for the target's class, through the clear USC */
    ld.fs = clear_sh;
    ld.fs_usc_gpu = match masked {
        Some(i) => b.clear_mask_fs[i].gpu,
        None => clear_gpu,
    };
    ld.fs_uniform_cpu = clear_pack;
    ld.fs_uniform_dwords = match masked {
        Some(_) => 4,
        None => dwords as u16,
    };
    ld.color_write = color;
    ld.color_mask = mask;

    /* The quad sits at NDC z 0; a collapsed depth range puts it exactly on
     * the clear depth, and depth ALWAYS makes every covered pixel take it. */
    ld.depth_func = GLES_CMP_ALWAYS;
    if depth {
        ld.depth_test = true;
        ld.depth_write = true;
        ld.depth_range = [d.clear_depth, d.clear_depth];
    } else {
        ld.depth_range[1] = 1.0;
    }
    if stencil {
        ld.stencil = true;
        ld.stencil_func = GLES_CMP_ALWAYS;
        ld.stencil_back_func = GLES_CMP_ALWAYS;
        ld.stencil_ref = d.clear_stencil;
        ld.stencil_back_ref = d.clear_stencil;
        ld.stencil_cmp_mask = 0xff;
        ld.stencil_back_cmp_mask = 0xff;
        /* GL: the stencil write mask says which bits the clear may change */
        ld.stencil_write_mask = d.stencil_write_mask;
        ld.stencil_back_write_mask = d.stencil_write_mask;
        ld.stencil_sfail = GLES_SOP_REPLACE;
        ld.stencil_dpfail = GLES_SOP_REPLACE;
        ld.stencil_dppass = GLES_SOP_REPLACE;
        ld.stencil_back_sfail = GLES_SOP_REPLACE;
        ld.stencil_back_dpfail = GLES_SOP_REPLACE;
        ld.stencil_back_dppass = GLES_SOP_REPLACE;
    }
    /* Not counted as a frame quad: it is one of the frame's objects, so a
     * later background clear still sees draws ahead of it and flushes. */
    if record_draw_txn(b, &mut p.scratch, &fl.load_draw, 0, 6, None) != 0 {
        if !b.warned_load {
            glog!("pvr_backend: could not record the scissored/masked clear quad");
        }
        b.warned_load = true;
        return -1;
    }
    0
}

/// The depth/stencil buffer for ds_id at w x h (an index into `b.ds`; `None`:
/// it could not be allocated): allocated tile aligned on first use. A buffer resized under the same
/// id (renderbuffer re-storage) goes back to the pool and gets a fresh
/// allocation. A free slot is there: `ds_make_room` made room before the
/// frame began (a buffer that holds contents is never given up for another);
/// when there is none, or the allocation fails, the answer is `None` and
/// `ds_resolve` marks the attachment's contents lost.
fn ds_get(b: &mut Backend, id: DsKey, w: u32, h: u32, samples: u32) -> Option<usize> {
    let mut free_slot: Option<usize> = None;
    /* per-sample storage: 4 samples spread 2x2 (Mesa rogue_get_isp_scale_xy_from_samples);
     * a size whose buffer bytes do not fit a usize gets no buffer (the C
     * code wrapped and allocated a buffer smaller than the ISP writes) */
    let bytes = ds_bytes(w, h, samples);
    b.ds_clock = b.ds_clock.wrapping_add(1);
    let now = b.ds_clock;

    for (i, s) in b.ds.iter_mut().enumerate().take(PVR_BACKEND_MAX_DS) {
        if !s.mem.impl_.is_null() && s.id == id {
            if s.w == w && s.h == h && s.samples == samples {
                s.used = now;
                return Some(i);
            }
            /* resized under the same id: the old buffer goes back */
            osal::gpu_free(&mut s.mem);
            *s = DsBuf::ZERO;
            free_slot = Some(i);
            break;
        }
        if s.mem.impl_.is_null() && free_slot.is_none() {
            free_slot = Some(i);
        }
    }
    /* Every slot taken is not reached from draw_one: `ds_make_room` made
     * room before the frame began, or refused the draw. No buffer is given
     * up here. */
    let slot = match (free_slot, bytes) {
        (Some(i), Some(bytes)) if osal::gpu_alloc(HEAP_GENERAL_BASE, bytes, OSAL_MEM_DATA, &mut b.ds[i].mem) == 0 => i,
        _ => {
            if !b.warned_ds {
                glog!(
                    "pvr_backend: no depth/stencil buffer for target {}/{}/{} ({}x{}): a draw or clear that uses it is refused",
                    id.kind,
                    id.id,
                    id.gen,
                    w,
                    h
                );
                b.warned_ds = true;
            }
            return None;
        }
    };
    let s = &mut b.ds[slot];
    s.id = id;
    s.w = w;
    s.h = h;
    s.samples = samples;
    s.valid = false;
    s.used = now;
    ds_note_peak(b);
    Some(slot)
}

/// After an allocation: the buffers held now, against the peaks.
fn ds_note_peak(b: &mut Backend) {
    let held = b.ds.iter().filter(|s| !s.mem.impl_.is_null());
    let (n, bytes) = held.fold((0u32, 0usize), |(n, bytes), s| (n + 1, bytes.saturating_add(s.mem.size)));
    b.ds_slots_peak = b.ds_slots_peak.max(n);
    b.ds_bytes_peak = b.ds_bytes_peak.max(bytes);
}

/// The depth/stencil line a run ends with: the most buffers held at once of
/// PVR_BACKEND_MAX_DS, the most bytes they held at once, the buffers given
/// up to make room for another target's, and the draws
/// refused (slots full: every buffer held contents GL keeps; no buffer: one
/// could not be allocated, or its contents were lost that way). A backend
/// that never allocated a buffer says so instead of a peak of 0, which
/// would read like headroom that was measured.
fn ds_report(b: &Backend, out: &mut impl core::fmt::Write) -> core::fmt::Result {
    if b.ds_slots_peak == 0 {
        out.write_str("ds slots: none allocated")?;
    } else {
        write!(out, "ds slots peak {}/{}, ds bytes peak {}", b.ds_slots_peak, PVR_BACKEND_MAX_DS, b.ds_bytes_peak)?;
    }
    write!(
        out,
        ", ds given up {}, ds refused {} (slots full {}, no buffer {}), depth sampled from the buffer {} (refused {})",
        b.ds_given_up,
        b.ds_full_refused.wrapping_add(b.ds_refused),
        b.ds_full_refused,
        b.ds_refused,
        b.depth_direct,
        b.depth_refused
    )
}

/// The depth/stencil buffers draw `d` wants for its frame (`frame_ds`,
/// `frame_s`), allocated on first use, sized by the target: the ISP stores
/// the whole render extent, and a depth attachment smaller than the colour
/// (ES 3.0 ds_w/ds_h) must not be written past its end. A buffer that could
/// not be allocated is `NoMemory`, and its attachment's contents are marked
/// lost ([`ds_refuse_lost`]).
fn ds_resolve(b: &mut Backend, d: &GlesDraw) -> (DsStore, DsStore) {
    if !d.ds_persist {
        return (DsStore::NoAttachment, DsStore::NoAttachment);
    }
    let samples = frame_samples(d.samples);
    let (depth_key, stencil_key) = zs_keys(d, split_zs_on());
    let Some(z) = ds_get(b, depth_key, d.target_w, d.target_h, samples) else {
        ds_lost_mark(b, depth_key);
        /* a separate stencil is loaded and stored only beside a depth buffer:
         * this frame cannot keep it either (its own buffer keeps what it
         * holds, so it is not marked lost) */
        let s = if stencil_key.is_some() { DsStore::NoMemory } else { DsStore::NoAttachment };
        return (DsStore::NoMemory, s);
    };
    let s = match stencil_key {
        None => DsStore::NoAttachment,
        Some(k) => match ds_get(b, k, d.target_w, d.target_h, samples) {
            Some(i) => DsStore::Buf(i as u32),
            None => {
                ds_lost_mark(b, k);
                DsStore::NoMemory
            }
        },
    };
    (DsStore::Buf(z as u32), s)
}

/// The halves (depth, stencil) of `key` whose contents are lost.
fn ds_lost_find(b: &Backend, key: DsKey) -> (bool, bool) {
    b.ds_lost[..b.nds_lost].iter().find(|l| l.key == key).map_or((false, false), |l| (l.z, l.s))
}

/// Remember that `key`'s buffer could not be allocated: what it held (both
/// halves of a packed buffer, one of a split one) is gone.
fn ds_lost_mark(b: &mut Backend, key: DsKey) {
    let (z, s) = match key.kind {
        DS_DEPTH => (true, false),
        DS_STENCIL => (false, true),
        _ => (true, true),
    };
    let n = b.nds_lost;
    if let Some(l) = b.ds_lost[..n].iter_mut().find(|l| l.key == key) {
        (l.z, l.s) = (l.z || z, l.s || s);
        return;
    }
    if n == DS_LOST_MAX {
        let old = b.ds_lost[0].key;
        glog_once!(
            "yakogl/pvr: {} depth/stencil attachments lost at once: target {}/{}/{} is forgotten, and a draw that tests \
             it is no longer refused",
            DS_LOST_MAX,
            old.kind,
            old.id,
            old.gen
        );
        b.ds_lost.copy_within(1.., 0);
        b.nds_lost -= 1;
    }
    b.ds_lost[b.nds_lost] = DsLost { key, z, s };
    b.nds_lost += 1;
}

/// A clear covered halves `z` / `s` of `key`: they hold defined values again.
fn ds_lost_lift(b: &mut Backend, key: DsKey, z: bool, s: bool) {
    let n = b.nds_lost;
    let Some(i) = b.ds_lost[..n].iter().position(|l| l.key == key) else {
        return;
    };
    let l = &mut b.ds_lost[i];
    (l.z, l.s) = (l.z && !z, l.s && !s);
    if !l.z && !l.s {
        b.ds_lost.copy_within(i + 1..n, i);
        b.ds_lost[n - 1] = DsLost::ZERO;
        b.nds_lost -= 1;
    }
}

/// What a frame can do with one half (depth or stencil) of its target.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ZsHalf {
    /// It has a buffer holding what the application put there, or the
    /// target has no such attachment.
    Kept,
    /// It has a buffer, but what the attachment held was lost with an
    /// earlier failed allocation and no clear has covered it since.
    Lost,
    /// This frame has no buffer for it.
    NoMemory,
}

/// The (depth, stencil) halves of draw `d`'s target, in a frame whose
/// buffers are `zs`.
fn zs_halves(b: &Backend, d: &GlesDraw, (ds, s): (DsStore, DsStore)) -> (ZsHalf, ZsHalf) {
    if !d.ds_persist {
        return (ZsHalf::Kept, ZsHalf::Kept);
    }
    let half = |store: DsStore, lost: bool| match store {
        DsStore::NoMemory => ZsHalf::NoMemory,
        _ if lost => ZsHalf::Lost,
        _ => ZsHalf::Kept,
    };
    let (depth_key, stencil_key) = zs_keys(d, split_zs_on());
    let (z_lost, s_lost) = ds_lost_find(b, depth_key);
    let z = half(ds, z_lost);
    let st = match stencil_key {
        /* a separate stencil object */
        Some(k) => half(s, ds_lost_find(b, k).1),
        /* the packed buffer's stencil byte */
        None if depth_key.kind == DS_PACKED => half(ds, s_lost),
        /* a split depth with no stencil attachment: no stencil to keep */
        None => ZsHalf::Kept,
    };
    (z, st)
}

/// Which halves (depth, stencil) the clear in `d` writes everywhere: no
/// scissor box and, for the stencil, every bit.
fn clear_covers(d: &GlesDraw) -> (bool, bool) {
    let whole = !(d.clear_quad && d.scissor);
    let z = d.do_clear_depth && whole;
    let s = d.do_clear_stencil && whole && (!d.clear_quad || d.clear_stencil_write_mask == 0xff);
    (z, s)
}

/// Refuse draw `d` (-ENOMEM, the contract's "does not fit" row) when it uses
/// a depth or stencil the backend cannot keep, in a frame whose buffers are
/// `zs`.
///
/// The rule, per half (depth, stencil) of an attachment: once its buffer
/// could not be allocated, every draw that tests that half is refused --
/// and every clear of it while there is still no buffer -- until a clear
/// with no scissor box and (for stencil) every bit writes the whole half.
///
/// A draw that neither tests nor clears depth or stencil goes ahead: ES 2.0
/// 4.1.5/4.1.6 let nothing else read them, and this rule covers whoever
/// reads them later.
fn ds_refuse_lost(b: &mut Backend, d: &GlesDraw, zs: (DsStore, DsStore)) -> i32 {
    let (z, s) = zs_halves(b, d, zs);
    let (covers_z, covers_s) = clear_covers(d);
    /* a clear-only draw carries the GL state, but draws no geometry */
    let (tests_z, tests_s) = (d.index_count != 0 && d.depth_test, d.index_count != 0 && d.stencil);
    let why = if z == ZsHalf::NoMemory && (tests_z || d.do_clear_depth) {
        "it tests or clears depth, and no buffer can keep it"
    } else if s == ZsHalf::NoMemory && (tests_s || d.do_clear_stencil) {
        "it tests or clears stencil, and no buffer can keep it"
    } else if z == ZsHalf::Lost && tests_z && !covers_z {
        "it tests depth that was lost with its buffer, and no clear has covered it since"
    } else if s == ZsHalf::Lost && tests_s && !covers_s {
        "it tests stencil that was lost with its buffer, and no clear has covered it since"
    } else {
        return 0;
    };
    b.ds_refused = b.ds_refused.wrapping_add(1);
    if !b.ds_refused_logged {
        b.ds_refused_logged = true;
        glog!(
            "yakogl/pvr: draw refused: no depth/stencil buffer for a {}x{} target; {} ({} refused so far)",
            d.target_w,
            d.target_h,
            why,
            b.ds_refused
        );
    }
    -ENOMEM
}

/// After draw `d` went into the frame (buffers `zs`): the halves its clear
/// covers hold defined values again.
fn ds_lift_cleared(b: &mut Backend, d: &GlesDraw, (ds, s): (DsStore, DsStore)) {
    let (covers_z, covers_s) = clear_covers(d);
    if !d.ds_persist || !(covers_z || covers_s) || b.nds_lost == 0 {
        return;
    }
    let (depth_key, stencil_key) = zs_keys(d, split_zs_on());
    if ds.buf().is_some() {
        ds_lost_lift(b, depth_key, covers_z, covers_s && stencil_key.is_none());
    }
    if let (Some(k), Some(_)) = (stencil_key, s.buf()) {
        ds_lost_lift(b, k, false, covers_s);
    }
}

/// Write the depth texels draw `d` carries (`GlesDraw::depth_texels`: the
/// CPU wrote its depth texture since this buffer matched it) into the depth
/// buffer of the frame `d` opens, `zs`, before the frame begins: the frame
/// then loads them (ZLS load, as after a stored kick) and the draw tests
/// against them.
///
/// The layout is the one `pvr_backend_depth_resolve` reads, written the
/// other way: packed D24S8 words, depth in bits 23..0
/// ([`ds_retention::depth_unorm24`], the exact reverse of the resolve's
/// [`ds_retention::unorm24_depth`]), row-linear at the ZLS-tile-aligned
/// width, row 0 at GL's y 0. The stencil byte (bits 31..24) is kept where
/// a kick stored the buffer (`valid`); a buffer never stored takes the
/// stencil a new frame would have initialised it to (its parked clear
/// value, else 0) and depth 1.0 outside the texels, as the frame's own
/// initialisation would have written. The buffer then counts as stored,
/// its depth plane as contents GL keeps, and a depth lost with an earlier
/// failed allocation as defined again.
///
/// 0 also when the frame has no buffer for its depth (`ds_refuse_lost`
/// refuses the draw then). `-ENOTSUP`, named once in the log, for a
/// multisampled buffer or one with no CPU view: a depth texture is a
/// single-sampled attachment and every buffer here is CPU-mapped, so
/// neither is expected, and the draw is refused rather than tested against
/// depth the texels no longer hold.
fn ds_seed_depth(b: &mut Backend, d: &GlesDraw, (ds, _): (DsStore, DsStore)) -> i32 {
    // SAFETY: the GL core's contract for the draw it hands over.
    let Some(t) = (unsafe { ds_retention::DepthTexels::of(d) }) else { return 0 };
    let Some(i) = ds.buf() else { return 0 };
    let (depth_key, _) = zs_keys(d, split_zs_on());
    let restore_s = b.ds_restore[STENCIL].cleared_value().unwrap_or(0);
    let s = &mut b.ds[i];
    let aw = s.w.div_ceil(ZLS_TILE) as usize * ZLS_TILE as usize;
    let rows = s.h.div_ceil(ZLS_TILE) as usize * ZLS_TILE as usize;
    if s.samples > 1 || s.mem.cpu.is_null() || aw.saturating_mul(rows).saturating_mul(4) > s.mem.size {
        glog_once!(
            "yakogl/pvr: draw refused: the depth the CPU wrote into object 0x{:x} cannot go into its depth buffer \
             ({} samples, {} CPU view)",
            d.depth_id,
            s.samples,
            if s.mem.cpu.is_null() { "no" } else { "a" }
        );
        return -util::ENOTSUP;
    }
    // SAFETY: a live, CPU-mapped allocation of at least aw * rows words
    // (checked above); no kick uses it (the GL core waited for every kick
    // that used the plane, and the frame this draw opens is not begun).
    let words = unsafe {
        osal::cache_invd(s.mem.cpu, aw * rows * 4);
        core::slice::from_raw_parts_mut(s.mem.cpu as *mut u32, aw * rows)
    };
    let (sw, sh, stored) = (s.w as usize, s.h as usize, s.valid);
    for (y, row) in words.chunks_exact_mut(aw).take(sh).enumerate() {
        for (x, w) in row.iter_mut().take(sw).enumerate() {
            let stencil = if stored { *w & 0xff00_0000 } else { restore_s << 24 };
            let inside = (x as u32) < t.w && (y as u32) < t.h;
            let depth = if inside {
                t.unorm24(x as u32, y as u32)
            } else if stored {
                *w & 0xff_ffff
            } else {
                0xff_ffff
            };
            *w = stencil | depth;
        }
    }
    // SAFETY: the words just written, inside the allocation.
    unsafe { osal::cache_flush(s.mem.cpu, aw * rows * 4) };
    s.valid = true;
    (s.planes[DEPTH], s.owners[DEPTH]) = (Plane::LIVE, d.depth_id);
    /* not initialised to a parked clear value: it holds the texels */
    b.ds_restore[DEPTH] = Plane::UNDEFINED;
    ds_lost_lift(b, depth_key, true, false);
    0
}

/// Bytes of the packed D24S8 buffer of a w x h target (whole ZLS tiles, per
/// sample), or `None` when that does not fit a usize.
fn ds_bytes(w: u32, h: u32, samples: u32) -> Option<usize> {
    let aw = (w.div_ceil(ZLS_TILE) as usize).checked_mul(ZLS_TILE as usize * if samples == 4 { 2 } else { 1 })?;
    let ah = (h.div_ceil(ZLS_TILE) as usize).checked_mul(ZLS_TILE as usize * if samples >= 2 { 2 } else { 1 })?;

    aw.checked_mul(ah)?.checked_mul(4)
}

/// The samples of a multisampled target across a split.
///
/// A 4x frame resolves its samples on its end-of-tile store (PBE REG_WORD0
/// `downscale`, pbestate.xml:255), so a frame the back end split into two
/// kicks (a full arena, the draw cap, a glFinish or glReadPixels before more
/// drawing) handed the next kick one colour per pixel, and that kick's
/// load quad wrote it to every sample: a sample the later draws left
/// uncovered held the average, not its own colour (#1289,
/// pvr-msaa-split; dEQP's multisample.constancy_* before `a4f64950`).
///
/// Now a multisampled kick that may not be the frame's last (no swap) emits
/// each tile twice: resolved into the target as before, and unresolved
/// (`downscale` off) into this store, the TPU's multisampled image of the
/// target -- TEXSTATE_IMAGE_WORD0 `smpcnt` (texstate.xml:273, two bits:
/// log2 of the samples) over the target's own width, height and stride,
/// `samples` times the single-sampled bytes. The next kick's load quad
/// binds the store as that image in place of the target and runs at the
/// full sample rate ([`fs_sample_rate`]), so each instance reads and writes
/// its own sample. A kick that stores no samples (a swap's, a 1x frame's)
/// into the store's target leaves it stale (`valid` false), and the next
/// frame reloads the resolved colour as before. There is one store, the
/// most recent such kick's: ES 2.0's one multisampled target is the window
/// surface, and a second 4x target's split takes the store for itself.
///
/// Unmeasured on the board (#1289): that
/// the PBE's unresolved layout is the TPU's `smpcnt` layout over the same
/// stride, and that a per-sample fetch of it reads the running instance's
/// sample.
#[derive(Clone, Copy)]
struct MsStore {
    mem: OsalGpuMem,
    /// The target the store holds the samples of: its colour address,
    /// extent, row pitch, format and sample count.
    target_gpu: u64,
    w: u32,
    h: u32,
    pitch: u32,
    format: u8,
    samples: u32,
    /// A submitted kick stored every sample of that target here, and no
    /// later kick wrote the target without them.
    valid: bool,
}

/// Bytes of the sample store of an `h`-row target of `format` whose rows
/// are `pitch` pixels apart, at `samples` a pixel: the TPU's multisampled
/// image, `samples` single-sampled images' worth. `None` when that does not
/// fit a usize.
fn ms_bytes(h: u32, pitch: u32, format: u8, samples: u32) -> Option<usize> {
    (pitch as usize)
        .checked_mul(h as usize)?
        .checked_mul(gles_surf_bytes_per_pixel(format) as usize)?
        .checked_mul(samples as usize)
}

/// The sample store, allocated or resized for the open frame's target:
/// `false` (once named in the log) when the pool has no room for it, and the
/// frame then stores its resolved colour only, as before.
fn ms_store_ready(b: &mut Backend) -> bool {
    let (w, h, pitch, format, samples) = (b.target_w, b.target_h, b.target_pitch, b.target_format, b.samples);
    let Some(bytes) = ms_bytes(h, pitch, format, samples) else { return false };
    let ms = &mut b.ms;
    if !ms.mem.impl_.is_null() && ms.mem.size >= bytes {
        return true;
    }
    /* too small for this target: the old store goes back (the OSAL holds it
     * until the kicks that may still read it have completed) */
    osal::gpu_free(&mut ms.mem);
    *ms = MsStore::ZERO;
    if osal::gpu_alloc(HEAP_GENERAL_BASE, bytes, OSAL_MEM_DATA, &mut ms.mem) != 0 {
        ms.mem = OsalGpuMem::ZERO;
        glog_once!(
            "pvr_backend: no sample store for the {}x{} {}x target ({} bytes): a split frame reloads its resolved colour",
            w,
            h,
            samples,
            bytes
        );
        return false;
    }
    true
}

impl MsStore {
    const ZERO: MsStore =
        MsStore { mem: OsalGpuMem::ZERO, target_gpu: 0, w: 0, h: 0, pitch: 0, format: 0, samples: 0, valid: false };

    /// Does the store hold every sample of the target draw `d` renders
    /// into, as the frame `d` opens would render it?
    fn holds(&self, d: &GlesDraw) -> bool {
        self.valid
            && !self.mem.impl_.is_null()
            && self.target_gpu == d.target_gpu
            && (self.w, self.h, self.pitch) == (d.target_w, d.target_h, crate::backend::draw_pitch(d))
            && self.format == d.target_format
            && self.samples == frame_samples(d.samples)
    }

    /// After a kick into target `t` was submitted: it stored every sample
    /// here (`stored`), or it wrote the target without them.
    fn settle(&mut self, t: MsStoreTarget, stored: bool) {
        if stored {
            (self.target_gpu, self.w, self.h, self.pitch) = (t.gpu, t.w, t.h, t.pitch);
            (self.format, self.samples, self.valid) = (t.format, t.samples, true);
        } else if self.valid && self.target_gpu == t.gpu {
            self.valid = false;
        }
    }
}

/// The open frame's target, as [`MsStore::settle`] records it.
struct MsStoreTarget {
    gpu: u64,
    w: u32,
    h: u32,
    pitch: u32,
    format: u8,
    samples: u32,
}

/// Is draw `d` the frame's load quad reading the sample store? Nothing else
/// binds the store's address. Such a draw runs at the full sample rate,
/// whatever its program.
fn loads_samples(b: &Backend, d: &GlesDraw) -> bool {
    !b.ms.mem.impl_.is_null() && d.num_textures == 1 && d.textures[0].base_gpu == b.ms.mem.gpu
}

/// Mark descriptor `w`'s image as one of `samples` samples a texel:
/// TEXSTATE_IMAGE_WORD0 `smpcnt` (texstate.xml:273, bits 62..63) is log2 of
/// the count, which its two bits fit up to 8x.
fn tex_desc_set_samples(w: &mut [u32], samples: u32) {
    let s = csb_pack!(
        ROGUE_TEXSTATE_IMAGE_WORD0_pack,
        2,
        csb::ROGUE_TEXSTATE_IMAGE_WORD0 { smpcnt: samples.max(1).ilog2(), ..Default::default() }
    );
    w[0] |= s[0];
    w[1] |= s[1];
}

/// Before a draw opens a frame: every depth/stencil buffer the frame needs
/// ([`zs_keys`]) has a slot, or gets one from a buffer that holds nothing GL
/// keeps -- no live plane ([`Plane`]), not in the open frame. A buffer whose
/// planes hold a clear value is let go only when they fit `ds_parked`, and a
/// key that comes back takes its planes from there (`ds_restore`). With
/// every slot holding contents the draw is refused: `-ENOMEM` and one named
/// line (src/ds_retention.rs). It used to give up the oldest buffer and draw
/// on, so another framebuffer's depth was lost with no error.
fn ds_make_room(b: &mut Backend, d: &GlesDraw) -> i32 {
    b.ds_restore = [Plane::UNDEFINED; 2];
    if !d.ds_persist {
        return 0;
    }
    let (depth_key, stencil_key) = zs_keys(d, split_zs_on());
    let want = [Some(depth_key), stencil_key];
    let held = |b: &Backend, k: DsKey| b.ds.iter().any(|s| !s.mem.impl_.is_null() && s.id == k);
    let missing: usize = want.iter().flatten().filter(|&&k| !held(b, k)).count();
    let mut free = b.ds.iter().filter(|s| s.mem.impl_.is_null()).count();
    while free < missing {
        let open = |i: usize| b.frame_open && (b.frame_ds.buf() == Some(i) || b.frame_s.buf() == Some(i));
        let can_go = |i: usize| {
            let s = &b.ds[i];
            let cleared = s.planes.iter().any(|p| p.cleared_value().is_some());
            !s.mem.impl_.is_null()
                && !open(i)
                && !want.contains(&Some(s.id))
                && !s.holds_contents()
                && (!cleared || b.ds_parked.has_room())
        };
        /* the least recently used of the buffers that may go */
        let now = b.ds_clock;
        let Some(i) = (0..PVR_BACKEND_MAX_DS).filter(|&i| can_go(i)).max_by_key(|&i| now.wrapping_sub(b.ds[i].used))
        else {
            let id = if d.depth_id != 0 && !held(b, depth_key) { d.depth_id } else { d.stencil_id };
            b.ds_full_refused = b.ds_full_refused.wrapping_add(1);
            return ds_retention::refuse("pvr", id, PVR_BACKEND_MAX_DS);
        };
        let s = b.ds[i];
        if s.planes.iter().any(|p| p.cleared_value().is_some()) {
            b.ds_parked.park(s.id, s.owners, s.planes);
        }
        osal::gpu_free(&mut b.ds[i].mem);
        b.ds[i] = DsBuf::ZERO;
        b.ds_given_up = b.ds_given_up.wrapping_add(1);
        free += 1;
    }
    let new_keys = want.map(|k| k.filter(|&k| !held(b, k)));
    for k in new_keys.into_iter().flatten() {
        if let Some(planes) = b.ds_parked.take(k) {
            let kept = DsBuf { id: k, ..DsBuf::ZERO }.kept_planes();
            for &p in kept {
                b.ds_restore[p] = planes[p];
            }
        }
    }
    ds_retention::admitted();
    0
}

/// After the frame's kick: what its buffers hold for GL now (the frame's
/// whole clears, and its writes) and whose planes they are.
fn ds_settle(b: &mut Backend) {
    let [depth_id, _, stencil_id, _] = b.frame_zs;
    for slot in [b.frame_ds, b.frame_s] {
        let Some(i) = slot.buf() else { continue };
        let s = &mut b.ds[i];
        for &p in s.kept_planes() {
            s.planes[p] = s.planes[p].after(b.frame_clr[p].cleared_value(), b.frame_wrote[p]);
        }
        s.owners = match s.id.kind {
            DS_PACKED => [depth_id, stencil_id],
            DS_DEPTH => [depth_id, 0],
            _ => [0, stencil_id],
        };
    }
}

/// The GL core says the depth/stencil contents of object `id` are gone
/// (`YakoglBackend::attachment_gone`): its planes are undefined in every
/// buffer and in `ds_parked`, and a buffer left holding nothing is freed. A
/// packed buffer keeps the other object's plane: deleting a framebuffer's
/// stencil renderbuffer does not take its depth (ES 2.0 4.4.3). A frame
/// still drawing into it is submitted first, so its kick settles before the
/// planes are undefined and no memory is freed under a kick not yet made.
#[no_mangle]
pub extern "C" fn pvr_backend_attachment_gone(id: u32) {
    // SAFETY: single-threaded by contract; taken once per entry point.
    let p = unsafe { PVR.get() };
    if !p.b.inited || id == 0 {
        return;
    }
    p.b.ds_parked.forget(id);
    let [depth_id, _, stencil_id, _] = p.b.frame_zs;
    if p.b.frame_open && p.b.frame_ds.buf().is_some() && (depth_id == id || stencil_id == id) {
        let rc = flush(p);
        if rc != 0 {
            glog_once!("pvr_backend: the frame drawing into object 0x{:x} did not submit ({}) before it went", id, rc);
        }
    }
    for s in p.b.ds.iter_mut().filter(|s| !s.mem.impl_.is_null() && s.owners.contains(&id)) {
        for pl in [DEPTH, STENCIL] {
            if s.owners[pl] == id {
                (s.owners[pl], s.planes[pl]) = (0, Plane::UNDEFINED);
            }
        }
        if s.planes == [Plane::UNDEFINED; 2] {
            osal::gpu_free(&mut s.mem);
            *s = DsBuf::ZERO;
        }
    }
}

/// The slot of the single-sampled buffer whose depth plane GL object `id`
/// (a depth texture's `GlesDraw::depth_id` form) owns and that holds depth a
/// submitted kick stored, at `w` x `h`: what a draw may sample in place of
/// the texels ([`pvr_depth_sampled`], [`depth_buffer_binding`]). A plane
/// that is a whole clear alone, or undefined, is not it (the CPU resolve
/// writes the clear value); nor is a buffer the frame being recorded still
/// draws into (the GL core submitted that kick before it asked, so this is
/// a check, not a path), nor one sized for another target.
fn depth_buffer_sampleable(b: &Backend, id: u32, w: u32, h: u32) -> Option<usize> {
    let i = ds_plane_buf(b, DEPTH, id)?;
    let s = &b.ds[i];
    let open = b.frame_open && (b.frame_ds.buf() == Some(i) || b.frame_s.buf() == Some(i));
    (s.valid && s.planes[DEPTH].is_live() && s.samples <= 1 && (s.w, s.h) == (w, h) && !open).then_some(i)
}

/// Will the next draw sample the depth GL object `id` (a depth texture's
/// `GlesDraw::depth_id` form, `w` x `h`) rendered from the buffer that
/// holds it, instead of the GL core's texels
/// (`crate::backend::samples_rendered_depth`)? True when the buffer owned
/// by `id` holds depth a submitted kick stored at that size
/// ([`depth_buffer_sampleable`]): the GL core then puts `id` into the
/// draw's binding (`GlesTextureBind::depth_id`) and resolves nothing, and
/// [`depth_buffer_binding`] points the draw's texture state at the buffer.
/// False: the GL core resolves on the CPU as before
/// ([`pvr_backend_depth_resolve`]).
///
/// A reading of the pool, nothing more: the GL core has submitted the kick
/// that rendered the depth first (`backend::flush`, as
/// `texture::resolve_depth` did before it read the buffer), so `valid` and
/// the plane say what that kick stored. No submit and no wait here: the GPU
/// runs kicks in order ([`flush`]), so the kick that samples the buffer
/// reads it after the one that stored it. The TPU converts the 24-bit
/// unorm the ISP stored, the bits [`pvr_backend_depth_resolve`] converts
/// on the CPU, so a draw reads the same depth either way, less the
/// resolve: on the AM62P that was the whole of glmark2 shadow's frame
/// (docs/notes/pvr-depth-sample.md).
pub fn pvr_depth_sampled(id: u32, w: u32, h: u32) -> bool {
    // SAFETY: single-threaded by contract; a read of the backend's state.
    let p = unsafe { PVR.get() };
    p.b.inited && id != 0 && depth_buffer_sampleable(&p.b, id, w, h).is_some()
}

/// May the GL core defer a depth texture's R32F storage
/// (`crate::backend::defers_depth_storage`)? Yes while initialised: a draw
/// that samples a rendered depth texture reads this backend's D24S8 buffer
/// ([`pvr_depth_sampled`]) and never the texels, so glmark2's shadow and
/// refract need no 7.68 MB R32F store for their 1600x1200 shadow map. A
/// draw that falls back to the texels, and every CPU read or write of them,
/// has the GL core allocate them first (`gl::texture::ensure_storage`).
pub fn pvr_defers_depth_storage() -> bool {
    // SAFETY: single-threaded by contract; a read of the backend's state.
    unsafe { PVR.get() }.b.inited
}

/// (draws that sampled a depth texture from its depth buffer, draws whose
/// binding named a buffer that was not there), the counts the exit line
/// prints ([`ds_report`]); for the host tests of the path
/// (tests/depth_resolve_pvr_backend.rs).
pub fn pvr_backend_depth_sample_stats() -> (u32, u32) {
    // SAFETY: single-threaded by contract; a read of the backend's state.
    let b = &unsafe { PVR.get() }.b;
    (b.depth_direct, b.depth_refused)
}

/// The binding a draw samples depth texture `t` through when the GL core
/// left its rendered depth in this backend's buffer (`t.depth_id`, set after
/// [`pvr_depth_sampled`] answered true): the buffer's words as an X8U24
/// STRIDE image ([`PVR_TEXFMT_DEPTH_BUFFER`], [`build_tex_desc`]) at the
/// ZLS-tile-aligned width the ISP stores rows at ([`pvr_backend_depth_resolve`]
/// reads the same layout), one level, the API extent, the texture's own
/// sampler state. `None` when the buffer is not there to sample (gone, or
/// holding other depth since): the texels are stale by the GL core's
/// agreement, so the caller binds the black texel and one line says so.
/// Between the answer and the draw nothing frees a buffer holding contents
/// ([`ds_make_room`] never takes one), so that is a check, not a path.
fn depth_buffer_binding(b: &mut Backend, t: &GlesTextureBind) -> Option<GlesTextureBind> {
    let Some(i) = depth_buffer_sampleable(b, t.depth_id, t.width, t.height) else {
        b.depth_refused = b.depth_refused.wrapping_add(1);
        glog_once!(
            "pvr_backend: the depth buffer a {}x{} depth texture was to be sampled from holds no stored depth of its \
             size: the draw samples black",
            t.width,
            t.height
        );
        return None;
    };
    let s = b.ds[i];
    let aw = s.w.div_ceil(ZLS_TILE) * ZLS_TILE;
    let rows = s.h.div_ceil(ZLS_TILE) * ZLS_TILE;
    b.depth_direct = b.depth_direct.wrapping_add(1);
    if b.depth_direct == 1 {
        glog!(
            "pvr_backend: a {}x{} depth texture is sampled from its depth buffer (X8U24, stride {}): no CPU resolve",
            t.width,
            t.height,
            aw
        );
    }
    Some(GlesTextureBind {
        format: PVR_TEXFMT_DEPTH_BUFFER,
        type_: GLES_TEXTYPE_2D,
        layers: 0,
        base_gpu: s.mem.gpu,
        stride_texels: aw,
        bytes: aw.wrapping_mul(rows).wrapping_mul(4),
        mip_filter: 0,
        mip_levels: 1,
        content_gen: 0,
        target_kick: 0,
        cpu: core::ptr::null(),
        ..*t
    })
}

/// Write the depth plane of GL object `id` (a depth texture) into `w` x `h`
/// floats at `dst`, `pitch` a row (`YakoglBackend::depth_resolve`).
///
/// The plane is in the buffer whose depth `id` owns (a packed D24S8 or the
/// object's own). Live: the words the ISP stored, which are packed D24S8
/// (ZLS_FORMAT 24BITINT, depth in bits 23..0) laid out LINEARLY at the
/// ZLS-tile-aligned width: ZLSCTL asks for no twiddling, and Mesa's
/// alignment transfer reads such a store as rows of `bufferRowLength` =
/// the aligned width (pvr_arch_cmd_buffer.c, the ds alignment transfer).
/// Confirmed on the AM62P (2026-10-01, gl_test 169: the rendered ramp read
/// back exactly). A plane that holds a
/// whole clear only (here or parked) is that value everywhere. 1 when it
/// holds nothing; -EBUSY when the frame being recorded still draws into it
/// (the GL core submits and waits for that kick first).
///
/// # Safety
/// `dst` holds `h` rows of `pitch` floats, `w <= pitch`.
#[no_mangle]
pub unsafe extern "C" fn pvr_backend_depth_resolve(id: u32, dst: *mut f32, w: u32, h: u32, pitch: u32) -> i32 {
    // SAFETY: single-threaded by contract; taken once per entry point.
    let p = unsafe { PVR.get() };
    if !p.b.inited || id == 0 || dst.is_null() || w > pitch {
        return 1;
    }
    let b = &p.b;
    if b.frame_open && b.frame_zs[0] == id {
        return -crate::util::EBUSY;
    }
    let fill = |v: f32| {
        // SAFETY: the caller's contract.
        unsafe { ds_retention::write_depth_texels(dst, w, h, pitch, |_, _| Some(v)) };
        0
    };
    let held =
        b.ds.iter().find(|s| !s.mem.impl_.is_null() && s.kept_planes().contains(&DEPTH) && s.owners[DEPTH] == id);
    let plane = held.map_or(Plane::UNDEFINED, |s| s.planes[DEPTH]);
    if let Some(v) = plane.cleared_value() {
        return fill(f32::from_bits(v));
    }
    let Some(s) = held.filter(|_| plane.is_live()) else {
        /* a clear value parked when its buffer went to another target */
        let parked = b.ds_parked.iter().find(|e| e.owners[DEPTH] == id).and_then(|e| e.planes[DEPTH].cleared_value());
        return parked.map_or(1, |v| fill(f32::from_bits(v)));
    };
    if !s.valid || s.samples > 1 || s.mem.cpu.is_null() {
        return 1;
    }
    let aw = s.w.div_ceil(ZLS_TILE) as usize * ZLS_TILE as usize;
    let rows = s.h.div_ceil(ZLS_TILE) as usize * ZLS_TILE as usize;
    if aw.saturating_mul(rows).saturating_mul(4) > s.mem.size {
        return -crate::util::EINVAL;
    }
    // SAFETY: a live, CPU-mapped allocation of at least aw * rows words
    // (checked above), which no pending kick writes (the GL core waited).
    let words = unsafe {
        osal::cache_invd(s.mem.cpu, aw * rows * 4);
        core::slice::from_raw_parts(s.mem.cpu as *const u32, aw * rows)
    };
    let (sw, sh) = (s.w, s.h);
    // SAFETY: the caller's contract.
    unsafe {
        ds_retention::write_depth_texels(dst, w, h, pitch, |x, y| {
            (x < sw && y < sh).then(|| ds_retention::unorm24_depth(words[y as usize * aw + x as usize]))
        })
    };
    0
}

/// The slot of the buffer that holds plane `pl` of GL object `id`.
fn ds_plane_buf(b: &Backend, pl: usize, id: u32) -> Option<usize> {
    b.ds.iter().position(|s| !s.mem.impl_.is_null() && s.kept_planes().contains(&pl) && s.owners[pl] == id)
}

/// A whole clear that was parked for plane `pl` of object `id`.
fn ds_parked_clear(b: &Backend, pl: usize, id: u32) -> Option<u32> {
    b.ds_parked.iter().find(|e| e.owners[pl] == id).and_then(|e| e.planes[pl].cleared_value())
}

/// Read the stencil plane of GL object `id` into `w` x `h` bytes at `dst`,
/// `pitch` a row (`YakoglBackend::stencil_resolve`), as [`pvr_backend_depth_resolve`]
/// writes the depth: the stencil byte (bits 31..24) of each packed D24S8
/// word, in the same row-linear layout at the ZLS-tile-aligned width.
/// Packed buffers only: a stencil kept apart ([`split_zs_on`], a buffer of its
/// own) is stored in the ISP's separate-stencil layout, which is unread, so
/// it answers -ENOTSUP. A whole clear is its value everywhere, here and once
/// parked; 1 when nothing is held (the texels stay); -EBUSY when the open
/// frame draws into it.
///
/// # Safety
/// `dst` holds `h` rows of `pitch` bytes, `w <= pitch`.
#[no_mangle]
pub unsafe extern "C" fn pvr_backend_stencil_resolve(id: u32, dst: *mut u8, w: u32, h: u32, pitch: u32) -> i32 {
    // SAFETY: single-threaded by contract; taken once per entry point.
    let p = unsafe { PVR.get() };
    if !p.b.inited || id == 0 || dst.is_null() || w > pitch {
        return 1;
    }
    let b = &p.b;
    if b.frame_open && b.frame_zs[STENCIL * 2] == id {
        return -crate::util::EBUSY;
    }
    let fill = |v: u32| {
        for y in 0..h as usize {
            for x in 0..w as usize {
                // SAFETY: the caller's contract: (x, y) is inside the rows.
                unsafe { dst.add(y * pitch as usize + x).write(v as u8) };
            }
        }
        0
    };
    let held = ds_plane_buf(b, STENCIL, id).map(|i| &b.ds[i]);
    let plane = held.map_or(Plane::UNDEFINED, |s| s.planes[STENCIL]);
    if let Some(v) = plane.cleared_value() {
        return fill(v);
    }
    let Some(s) = held.filter(|_| plane.is_live()) else {
        return ds_parked_clear(b, STENCIL, id).map_or(1, fill);
    };
    if s.id.kind == DS_STENCIL {
        return -crate::util::ENOTSUP;
    }
    if !s.valid || s.samples > 1 || s.mem.cpu.is_null() {
        return 1;
    }
    let aw = s.w.div_ceil(ZLS_TILE) as usize * ZLS_TILE as usize;
    let rows = s.h.div_ceil(ZLS_TILE) as usize * ZLS_TILE as usize;
    if aw.saturating_mul(rows).saturating_mul(4) > s.mem.size {
        return -crate::util::EINVAL;
    }
    // SAFETY: a live, CPU-mapped allocation of at least aw * rows words
    // (checked above), which no pending kick writes (the GL core waited).
    let words = unsafe {
        osal::cache_invd(s.mem.cpu, aw * rows * 4);
        core::slice::from_raw_parts(s.mem.cpu as *const u32, aw * rows)
    };
    for y in 0..h.min(s.h) as usize {
        for x in 0..w.min(s.w) as usize {
            // SAFETY: the caller's contract: (x, y) is inside the rows.
            unsafe { dst.add(y * pitch as usize + x).write((words[y * aw + x] >> 24) as u8) };
        }
    }
    0
}

/// Write a `w` x `h` rectangle at (`x`, `y`) of plane `plane`
/// ([`crate::backend::DS_PLANE_DEPTH`]: f32 depth; `DS_PLANE_STENCIL`: u8)
/// of GL object `id` into the buffer that holds it (`YakoglBackend::ds_write`).
///
/// The layout is the one [`pvr_backend_depth_resolve`] and
/// [`pvr_backend_stencil_resolve`] read: packed D24S8 words, row-linear at
/// the ZLS-tile-aligned width, the write changing only its own half of each
/// word ([`ds_retention::depth_unorm24`], like the frame's own seeding of
/// CPU-written depth, `ds_seed_depth`). The plane becomes live (a clear value
/// that is no longer the whole plane must not be parked). A buffer no kick
/// has stored into yet is first filled with what a frame would initialise it
/// to (a plane's clear value, else depth 1.0 and stencil 0), so the pixels
/// outside the rectangle are defined. 1 when nothing is held for the plane
/// (nothing written); -EBUSY when the open frame draws into it; -ENOMEM when
/// only a parked clear value remains (no buffer to write into); -ENOTSUP for
/// a stencil kept apart or a multisampled buffer; -EINVAL for a rectangle
/// outside the buffer. An empty rectangle (`w` or `h` 0) is a probe: every
/// check above runs, nothing is written or initialised, and 0 says the buffer
/// can be written.
///
/// # Safety
/// `src` holds `h` rows of `pitch` elements of the plane's type, `w <= pitch`.
#[no_mangle]
pub unsafe extern "C" fn pvr_backend_ds_write(
    id: u32,
    plane: u32,
    src: *const c_void,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    pitch: u32,
) -> i32 {
    // SAFETY: single-threaded by contract; taken once per entry point.
    let p = unsafe { PVR.get() };
    if !p.b.inited || id == 0 || src.is_null() || w > pitch {
        return 1;
    }
    let pl = match plane {
        crate::backend::DS_PLANE_DEPTH => DEPTH,
        crate::backend::DS_PLANE_STENCIL => STENCIL,
        _ => return -crate::util::EINVAL,
    };
    let b = &mut p.b;
    if b.frame_open && b.frame_zs[pl * 2] == id {
        return -crate::util::EBUSY;
    }
    let Some(i) = ds_plane_buf(b, pl, id) else {
        return if ds_parked_clear(b, pl, id).is_some() { -crate::util::ENOMEM } else { 1 };
    };
    let s = &mut b.ds[i];
    if s.id.kind == DS_STENCIL || s.samples > 1 || s.mem.cpu.is_null() {
        return -crate::util::ENOTSUP;
    }
    if x as u64 + w as u64 > s.w as u64 || y as u64 + h as u64 > s.h as u64 {
        return -crate::util::EINVAL;
    }
    let aw = s.w.div_ceil(ZLS_TILE) as usize * ZLS_TILE as usize;
    let rows = s.h.div_ceil(ZLS_TILE) as usize * ZLS_TILE as usize;
    if aw.saturating_mul(rows).saturating_mul(4) > s.mem.size {
        return -crate::util::EINVAL;
    }
    if w == 0 || h == 0 {
        return 0;
    }
    // SAFETY: a CPU-mapped allocation of at least aw * rows words (checked
    // above), which no pending kick touches (the GL core waited).
    let words = unsafe { core::slice::from_raw_parts_mut(s.mem.cpu as *mut u32, aw * rows) };
    if s.valid {
        // SAFETY: the same words.
        unsafe { osal::cache_invd(s.mem.cpu, aw * rows * 4) };
    } else {
        let depth = s.planes[DEPTH].cleared_value().map_or(1.0, f32::from_bits);
        let stencil = s.planes[STENCIL].cleared_value().unwrap_or(0) & 0xff;
        words.fill(ds_retention::depth_unorm24(depth) | stencil << 24);
    }
    for row in 0..h as usize {
        for col in 0..w as usize {
            let at = row * pitch as usize + col;
            let word = &mut words[(y as usize + row) * aw + x as usize + col];
            // SAFETY: the caller's contract: (col, row) is inside the rows.
            *word = unsafe {
                if pl == DEPTH {
                    (*word & 0xff00_0000) | ds_retention::depth_unorm24((src as *const f32).add(at).read())
                } else {
                    (*word & 0x00ff_ffff) | u32::from((src as *const u8).add(at).read()) << 24
                }
            };
        }
    }
    // SAFETY: the same words.
    unsafe { osal::cache_flush(s.mem.cpu, aw * rows * 4) };
    s.valid = true;
    s.planes[pl] = Plane::LIVE;
    0
}

/// PVR_BACKEND_OPT_DS_DUMP: what the ISP stored after the kick `fence`
/// (row-linear and tile-linear readings of a few pixels, and a census).
///
/// The census counts depth AND stencil. It used to count only the depth half
/// (`v & 0xff_ffff`), so every dump of a run that changed nothing but the
/// stencil printed the same three numbers -- which is how this dump was once
/// read as evidence that a stencil clear had written nothing. An instrument
/// that cannot see the thing under test says nothing about it, whatever
/// numbers it prints.
fn ds_dump(b: &DsBuf, fence: i64) {
    const PTS: [[u32; 2]; 6] = [[4, 4], [32, 32], [60, 40], [61, 28], [16, 16], [48, 48]];
    /* (the first sample plane: the dump reads pixel positions) */
    let aw = b.w.div_ceil(ZLS_TILE) as usize * ZLS_TILE as usize;
    let ah = b.h.div_ceil(ZLS_TILE) as usize * ZLS_TILE as usize;
    let Some(n) = aw.checked_mul(ah) else {
        return;
    };
    let (mut zero, mut far, mut other) = (0u32, 0u32, 0u32);
    /* the stencil byte's census: how many pixels hold 0, how many hold the
     * most common other value, and what that value is */
    let (mut s_zero, mut s_other, mut s_val) = (0u32, 0u32, 0u32);

    if b.mem.cpu.is_null() || n.saturating_mul(4) > b.mem.size || n < 4 {
        return;
    }
    // SAFETY: the buffer is a live, CPU-mapped, allocator-aligned allocation of
    // at least n * 4 bytes (checked above); the kick that wrote it completed.
    let p = unsafe {
        osal::cache_invd(b.mem.cpu, n * 4);
        core::slice::from_raw_parts(b.mem.cpu as *const u32, n)
    };
    for &v in p {
        let d = v & 0xff_ffff;

        if d == 0 {
            zero += 1;
        } else if d == 0xff_ffff {
            far += 1;
        } else {
            other += 1;
        }
        let st = v >> 24;

        if st == 0 {
            s_zero += 1;
        } else {
            s_other += 1;
            s_val = st; /* the last non-zero one seen; they are one value in practice */
        }
    }
    glog!(
        "  ds {}x{} (kind {} id {} gen {}) after kick {}: depth zero={} far={} other={}, stencil zero={} nonzero={} (0x{:02x})",
        aw,
        ah,
        b.id.kind,
        b.id.id,
        b.id.gen,
        fence,
        zero,
        far,
        other,
        s_zero,
        s_other,
        s_val
    );
    glog!("  ds [0..3]={:08x} {:08x} {:08x} {:08x}", p[0], p[1], p[2], p[3]);
    for pt in PTS {
        let (x, y) = (pt[0] as u64, pt[1] as u64);
        let (t, aw64) = (ZLS_TILE as u64, aw as u64);
        /* small constants times a u32-derived width: no u64 overflow */
        let row = y * aw64 + x;
        let tile = ((y / t) * (aw64 / t) + x / t) * t * t + (y % t) * t + x % t;

        if row < n as u64 && tile < n as u64 {
            glog!("  ds ({:2},{:2}): row-linear {:08x}  tile-linear {:08x}", x, y, p[row as usize], p[tile as usize]);
        }
    }
}

fn frame_begin(p: &mut Pvr, d: &GlesDraw) {
    let zs = ds_resolve(&mut p.b, d);
    frame_begin_with(p, d, zs);
}

/// [`frame_begin`] with the frame's depth/stencil already found
/// ([`ds_resolve`]): [`draw_one`] finds them first, to refuse a draw whose
/// depth/stencil has no buffer before the frame takes anything from it.
fn frame_begin_with(p: &mut Pvr, d: &GlesDraw, (frame_ds, frame_s): (DsStore, DsStore)) {
    let b = &mut p.b;

    b.frame_open = true;
    b.ndraws = 0;
    b.vis_frame = 0;
    b.target_gpu = d.target_gpu;
    b.target_w = d.target_w;
    b.target_h = d.target_h;
    /* a chain level's frame clips its store to the level (`record_mip_level`) */
    b.frame_clip = b.mip_clip;
    b.mip_clip = (0, 0);
    b.target_pitch = crate::backend::draw_pitch(d);
    b.target_format = d.target_format;
    b.target_alpha_bits = d.target_alpha_bits;
    b.target_flip_y = d.flip_y;
    b.num_targets = (if d.num_targets > 1 { d.num_targets } else { 1 }).min(PVR_BACKEND_MAX_RT as u8);
    b.targets[0] =
        GlesDrawTarget { gpu: d.target_gpu, w: d.target_w, h: d.target_h, format: d.target_format, out_shift: 0 };
    for i in 1..b.num_targets as usize {
        b.targets[i] = d.targets[i];
    }
    b.samples = if d.samples == 4 {
        4
    } else if d.samples == 2 {
        2
    } else {
        1
    };
    /* A clear quad carries its clear values for itself; the frame it opens
     * starts with no background clear at all (the quad, recorded next, is
     * what writes them, inside the scissor). */
    let bg_color = d.do_clear && !d.clear_quad;
    let bg_depth = d.do_clear_depth && !d.clear_quad;
    let bg_stencil = d.do_clear_stencil && !d.clear_quad;

    /* A z-only frame (no colour target) has no colour to load: its tiles
     * are never stored. */
    let load_color = !bg_color && d.target_gpu != 0;

    b.frame_cleared = bg_color;
    b.frame_loads_color = load_color; /* record_frame_quad(.., load_color, ..) below */
    b.swap_drops_zs = false;
    b.frame_swapped = false;
    if bg_color {
        b.clear_rgba = d.clear_rgba;
    }
    b.clear_depth = if bg_depth { d.clear_depth } else { 1.0 };
    b.clear_stencil = if bg_stencil { d.clear_stencil } else { 0 };
    /* A buffer given a slot again after its planes were parked with a clear
     * value (ds_make_room) is new, so the frame initialises it: to that value,
     * which is what it held. The frame counts it as cleared to it. */
    let clears = ds_retention::draw_clears(d);
    for p in [DEPTH, STENCIL] {
        b.frame_clr[p] = clears[p].map_or(b.ds_restore[p], Plane::cleared);
    }
    if let (false, Some(v)) = (bg_depth, b.ds_restore[DEPTH].cleared_value()) {
        b.clear_depth = f32::from_bits(v);
    }
    if let (false, Some(v)) = (bg_stencil, b.ds_restore[STENCIL].cleared_value()) {
        b.clear_stencil = v as u8;
    }
    b.ds_restore = [Plane::UNDEFINED; 2];
    b.frame_wrote = ds_retention::draw_writes(d);
    /* The next frame set: this kick is recorded into it while the GPU may
     * still read the earlier ones'. It is free once the kick that last used
     * it has completed (usually long ago; at worst this waits a few frames). */
    /* the frame recorded last: its arenas' use, before the next set's
     * cursors replace them */
    note_arena_hw(b);
    b.cur = (b.cur + 1) % b.nsets;
    if b.wait_fence(b.sets[b.cur].fence) != 0 {
        glog_once!("pvr_backend: reusing frame set {} before its kick completed", b.cur);
    }
    b.frame_set_activate(b.cur);
    b.pds.bump = 0;
    b.pds_seen = [PdsSeen::ZERO; PDS_SEEN_SLOTS];
    pds_once_forget(b, 0);
    b.gen.bump = 0;
    b.vdm_dw = 0;
    b.term_gpu = 0;
    b.frame_quads = 0;
    b.frame_tile_buffers = 0;
    b.prims = 0;
    b.ndbias = 0;
    b.nscissor = 0;
    /* depth/stencil start from memory unless cleared, or never stored yet
     * (then they are initialised to the clear values, so every pixel of
     * the buffer is defined from the first kick on) */
    b.frame_ds_id = d.ds_id;
    b.frame_ds_persist = d.ds_persist;
    b.frame_zs = [d.depth_id, d.depth_gen, d.stencil_id, d.stencil_gen];
    (b.frame_ds, b.frame_s) = (frame_ds, frame_s);
    {
        let ds_valid = b.frame_ds().map(|ds| ds.valid);
        /* apart, the stencil's own buffer says whether it holds a store
         * (a stencil without a buffer of its own is refused by ds_refuse_lost;
         * the packed buffer's byte then carries nothing anyone reads) */
        let s_valid = match b.frame_s.buf() {
            Some(i) => Some(b.ds[i].valid),
            None => ds_valid,
        };
        let init_depth = ds_valid.is_some_and(|valid| bg_depth || !valid);
        let init_stencil = s_valid.is_some_and(|valid| bg_stencil || !valid);

        b.load_depth = ds_valid.is_some() && !init_depth;
        b.load_stencil = s_valid.is_some() && !init_stencil;
        let clear_float = bg_color && clears_by_quad(d.target_format);

        if load_color || init_depth || init_stencil || clear_float {
            let _ = record_frame_quad(p, d, load_color, init_depth, init_stencil, clear_float);
        }
    }
}

/// The descriptor a draw binds for `t` ([`build_tex_desc`] with the
/// debug option off), for the host tests that pin the state words of a
/// texture kind against their design (tests/es3_pvr_2darray.rs).
pub fn texture_descriptor(t: &GlesTextureBind) -> [u32; TEX_DESC_DWORDS as usize] {
    let mut w = [0u32; TEX_DESC_DWORDS as usize];
    let mut warned = u32::MAX; /* no address warning on a test's address */
    build_tex_desc(&mut w, t, &mut warned);
    w
}

/// One pvr_combined_image_sampler_descriptor (TEX_DESC_DWORDS, zeroed by the
/// caller): image words 0..3, image meta 4..7, sampler words 8..11, sampler
/// meta 12..15, gather words 16..19.
fn build_tex_desc(w: &mut [u32], t: &GlesTextureBind, warned: &mut u32) {
    let fmt: TexFormat;

    /* An image address outside the heaps is a texture fetch the TCU will
     * fault on (the fault's VA is then this address plus a texel offset,
     * which reads as a small unmapped address). Say so here, where the
     * binding is still known. */
    if t.base_gpu < HEAP_GENERAL_BASE && *warned < 4 {
        *warned += 1;
        glog!(
            "pvr_backend: sampler {}: image address 0x{:x} is not a heap address ({}x{} fmt {})",
            t.unit,
            t.base_gpu,
            t.width,
            t.height,
            t.format
        );
    }
    /* the mip chain the sampler may walk: levels consecutive in memory
     * (Mesa pvr_image_plane_setup_mip_levels). A non-power-of-two chain is
     * laid out as its power-of-two extent's (gl::texture pot_chain_layout):
     * the stride is that width, word 0 keeps the API extent, as Mesa's
     * image view states them. */
    let mut levels: u32 = if t.mip_filter != 0 && t.mip_levels > 1 { t.mip_levels as u32 } else { 1 };

    if t.format == GLES_TEXFMT_RGBA8 {
        fmt = if t.bgra != 0 { TEXF_BGRA8 } else { TEXF_RGBA8 };
        w[4] = t.stride_texels.wrapping_mul(t.height).wrapping_mul(4); /* layer size (bytes) */
        w[7] = TEX_PCK_INFO_RGBA8_UNORM;
    } else if t.format == GLES_TEXFMT_SRGB8_A8 {
        /* the RGBA8 texels, decoded by the TPU (never B,G,R,A: no scan-out
         * image is sRGB) */
        fmt = TEXF_SRGB8_A8;
        w[4] = t.stride_texels.wrapping_mul(t.height).wrapping_mul(4);
        w[7] = TEX_PCK_INFO_SRGB8_A8;
    } else if t.format == GLES_TEXFMT_RGB565 {
        fmt = TEXF_RGB565;
        levels = 1;
        w[4] = t.stride_texels.wrapping_mul(t.height).wrapping_mul(2);
        w[7] = TEX_PCK_INFO_COMPRESSED_UNORM; /* no pack conversion: the TPU hands back unorm */
    } else if matches!(t.format, GLES_TEXFMT_L8 | GLES_TEXFMT_A8 | GLES_TEXFMT_LA8) {
        fmt = match t.format {
            GLES_TEXFMT_L8 => TEXF_L8,
            GLES_TEXFMT_A8 => TEXF_A8,
            _ => TEXF_LA8,
        };
        w[4] = t.stride_texels.wrapping_mul(t.height).wrapping_mul(gles_texfmt_bytes(t.format));
        /* Mesa's pack format for R8/R8G8 UNORM is U8888 with the UNORM bit
         * (0x140, pco_pipe_to_pck_format), and pco reads it only for shadow
         * compares and formatless stores; RGB565 samples right with 0x11f.
         * Which one the narrow formats want is the board's to say:
         * `pvr_debug_set_narrow_la` 1 takes 0x140, 2 takes 0x11f. */
        w[7] = if narrow_la_mode() == 2 { TEX_PCK_INFO_COMPRESSED_UNORM } else { TEX_PCK_INFO_RGBA8_UNORM };
    } else if matches!(t.format, GLES_TEXFMT_RGBA16F | GLES_TEXFMT_R16F | GLES_TEXFMT_R32F) {
        fmt = match t.format {
            GLES_TEXFMT_RGBA16F => TEXF_RGBA16F,
            GLES_TEXFMT_R16F => TEXF_R16F,
            _ if t.depth_luminance != 0 => TEXF_R32F_LUMINANCE,
            _ => TEXF_R32F,
        };
        w[4] = t.stride_texels.wrapping_mul(t.height).wrapping_mul(gles_texfmt_bytes(t.format));
        w[7] = if t.format == GLES_TEXFMT_R32F { TEX_PCK_INFO_F32 } else { TEX_PCK_INFO_F16 };
    } else if let Some((f, pck)) = es3_tex_format(t.format) {
        fmt = f;
        w[4] = t.stride_texels.wrapping_mul(t.height).wrapping_mul(gles_texfmt_bytes(t.format));
        w[7] = pck;
    } else if t.format == PVR_TEXFMT_DEPTH_BUFFER {
        /* a depth texture read from its depth buffer (depth_buffer_binding):
         * one level, the ISP's packed D24S8 words as X8U24 at the ZLS-tile-
         * aligned width the binding carries as its stride */
        fmt = if t.depth_luminance != 0 { TEXF_X8U24_LUMINANCE } else { TEXF_X8U24 };
        levels = 1;
        w[4] = t.stride_texels.wrapping_mul(t.height).wrapping_mul(4);
        w[7] = TEX_PCK_INFO_X8U24;
    } else if let Some((f, bytes)) = raw_tex_format(t.format) {
        /* an integer target's frame load: its pixels as raw unsigned bits */
        fmt = f;
        levels = 1;
        w[4] = t.stride_texels.wrapping_mul(t.height).wrapping_mul(bytes);
        w[7] = PVR_PCK_FORMAT_INVALID;
    } else {
        fmt = if t.format == GLES_TEXFMT_ETC2_RGB {
            TEXF_ETC2_RGB
        } else if t.format == GLES_TEXFMT_ETC2_RGBA {
            TEXF_ETC2_RGBA
        } else if t.format == GLES_TEXFMT_ETC2_SRGB_RGB {
            TEXF_ETC2_SRGB_RGB
        } else if t.format == GLES_TEXFMT_ETC2_SRGB_RGBA {
            TEXF_ETC2_SRGB_RGBA
        } else {
            TEXF_ETC2_PT
        };
        /* The TWIDDLED word (texstate.xml IMAGE_WORD1): num_mip_levels at
         * [0:3] and baselevel at [60:63] -- the tail below writes `levels`
         * at [60:63] for the STRIDE word only. It wrote it here too until
         * 2026-09-24, so every ETC texture asked for base level 1 with no
         * levels (the AM62P's filtering.2d *_etc1, format.etc1). The GL
         * core lays the chain out level-major and packed, level l's grid
         * level 0's power-of-two grid halved l times (texture.rs
         * level_block_grid), which is the TPU's walk (Mesa
         * pvr_image_plane_setup_mip_levels: max(1, (POT(W)/4) >> l)
         * blocks), NPOT sizes included. */
        w[4] = t.bytes;
        w[7] = TEX_PCK_INFO_COMPRESSED_UNORM;
    }
    if t.type_ == GLES_TEXTYPE_CUBE_TWIDDLED_PROBE {
        /* The native cube (gl::native_cube, pvr_native_cube): the TPU's own
         * cube over Mesa's twiddled face-major layout, in the format's own
         * word 0 and pack info above. TEXSTATE_IMAGE_WORD0.textype = CUBE (3,
         * texstate.xml:248-253, bits 0..2); the twiddled IMAGE_WORD1
         * (texstate.xml:292-312) is NOT the stride word: num_mip_levels at
         * [0:3], depth = array layers - 1 = 5 at [4:14] (Mesa
         * pvr_arch_tex_state.c:278-286), mipmaps_present [15], texaddr
         * [16:53], baselevel [60:63] = 0, the rest as the format's. The
         * image meta's layer size is one face's full chain, 4 KiB aligned
         * (pvr_image.c), which is what the copy is laid out in. */
        let iw0 = tex_image_word0(fmt, csb::ROGUE_TEXSTATE_TEXTYPE_CUBE, t.width, t.height);
        /* The level count in the IMAGE word is the copy's whole chain, to
         * 1x1 of the face's power-of-two side, whatever the filter: the TPU
         * steps from face to face by the layer size it derives from it,
         * one level's size for a count of 1 and the full chain for any
         * more (Mesa pvr_image_plane_setup_mip_levels adds the rest of the
         * chain only when mip_levels > 1), and gl::native_cube lays every
         * face out as its full chain. With the sampler's count here, every
         * single-level cube read faces 1..5 from the wrong bytes on the
         * AM62P (am62p-board, 33a24bd7: 132 non-mipmap cases with
         * YAKOGL_PVR_NATIVE_CUBE=1). How far the sampler may walk is the
         * SAMPLER's LOD range (`tex_desc_sampler` over `levels`).
         *
         * A draw that samples ONE level gets a one-level copy (level 0 in
         * faces of its own size, gl::native_cube::copy's `single`, decided
         * by the same rule as here: a mipmap filter over more than one
         * level), described as one level. Declaring the whole chain over
         * it let the one-level sampler's maxlod of 1.0 (the min/mag choice
         * the TPU makes on the clamped LOD) read level 1, which the texture
         * never had: 3b634e98's native arm, filtering.cube min != mag (32)
         * and size.cube 256/512 non-mipmap, all white (6). The debug
         * FORCE_MIP0 option does not change the layout here. */
        let single = !(t.mip_filter != 0 && t.mip_levels > 1);
        let chain = if single { 1 } else { 32 - t.width.max(1).next_power_of_two().leading_zeros() };
        /* an ETC cube: the compressed word's mode (TPU decode) and no
         * alpha_msb, as its 2D word above; gl::native_cube lays its faces
         * out as the block grids the TPU walks. Step C's default sent ETC1
         * cubes here with the copy refusing them: filtering.cube.*_etc1 and
         * format.etc1_cube_pot failed on the AM62P (15550645). */
        let etc = is_etc2(t.format);
        let iw1 = tex_twiddled_image_word1(etc, chain.min(15), 5, chain > 1, t.base_gpu);
        w[4] = t.bytes / 6; /* one face, 4 KiB aligned (gl::native_cube) */
        w[0..2].copy_from_slice(&iw0);
        w[2..4].copy_from_slice(&iw1);
        tex_desc_sampler(w, t, levels);
        /* GL ES 2.0 cube maps are not seamless: each face filters and wraps
         * by its own address modes (dEQP's ES 2.0 reference,
         * seamlessCubeMap false), which SAMPLER_WORD1.cemedge_dontfilter
         * selects (texstate.xml, bit 63; Mesa pvr_arch_sampler.c sets it
         * for VK_EXT_non_seamless_cube_map). ES 3.0 requires seamless: the
         * bit stays clear there. With it clear the AM62P's native arm
         * still failed texture_functions.vertex.texturecubelod in its
         * high-LOD corner, where a face is 2x2 and every sample is on an
         * edge (am62p-board, 33a24bd7). */
        if crate::REPORT_ES2 {
            let w1 = csb_pack!(
                ROGUE_TEXSTATE_SAMPLER_WORD1_pack,
                2,
                csb::ROGUE_TEXSTATE_SAMPLER_WORD1 { cemedge_dontfilter: true, ..Default::default() }
            );
            w[10] = w1[0];
            w[11] = w1[1];
        }
        return;
    }
    if t.type_ == GLES_TEXTYPE_2D_ARRAY_TWIDDLED {
        /* The native 2D array (gl::native_array, pvr_native_array;
         * docs/es3/pvr-2darray.md section 3): Mesa's description of an
         * array, which has no hardware array path either
         * (pco_nir_tex.c:468) -- TEXSTATE_IMAGE_WORD0.textype = 2D
         * (pvr_arch_tex_state.c:157-161, the 2D and 2D_ARRAY cases are one
         * line), width and height ONE layer's; the twiddled IMAGE_WORD1
         * with depth = layers - 1 (:284-290; 11 bits, 2048 layers at most,
         * ES 3.0 requires 256), the chain's level count and
         * mipmaps_present; image meta dword 4 one layer's bytes
         * (PCO_IMAGE_META_LAYER_SIZE, :318), which the shader multiplies
         * the clamped layer by and adds to the image address in the two
         * `tao` data words (pco_nir_tex.c:675-688). The base address is
         * published in meta dwords 5 and 6 (Mesa's BUFFER_ELEMS and
         * Z_SLICE, read only by buffer textures and 2D views of 3D,
         * pco_nir_tex.c:75,546-560, never by an array program), low dword
         * first, so the shader adds with a carry instead of decoding
         * texaddr's bitfields (section 4.2, the recommended way).
         *
         * The level count is the copy's whole chain to 1x1 of the layer's
         * power-of-two extent whatever the filter, or 1 for a one-level copy
         * (gl::native_array::copy's `single`, the same rule as here): the
         * TPU derives the layer stride from it, and a single-level sampler
         * over a declared chain read levels the texture never had on the
         * AM62P (the cube's 3b634e98). How far the sampler may walk is the
         * SAMPLER's LOD range (`tex_desc_sampler` over `levels`). */
        let layers = u32::from(t.layers).max(1);
        let iw0 = tex_image_word0(fmt, csb::ROGUE_TEXSTATE_TEXTYPE_2D, t.width, t.height);
        let single = !(t.mip_filter != 0 && t.mip_levels > 1);
        let (pw, ph) = (t.width.max(1).next_power_of_two(), t.height.max(1).next_power_of_two());
        let chain = if single { 1 } else { 32 - pw.max(ph).leading_zeros() };
        let etc = is_etc2(t.format);
        let iw1 = tex_twiddled_image_word1(etc, chain.min(15), (layers - 1).min(2047), chain > 1, t.base_gpu);
        w[0..2].copy_from_slice(&iw0);
        w[2..4].copy_from_slice(&iw1);
        w[4] = t.bytes / layers; /* one layer, its full chain, 4 KiB aligned (gl::native_array) */
        w[5] = t.base_gpu as u32;
        w[6] = (t.base_gpu >> 32) as u32;
        /* the sampler's w axis is not used: the layer is an address, not a
         * coordinate (section 3.4) */
        tex_desc_sampler(w, t, levels);
        return;
    }
    if t.type_ == GLES_TEXTYPE_3D_TWIDDLED {
        /* The native 3D texture (gl::native_vol, pvr_native_3d;
         * docs/notes/pvr-3d-native.md): Mesa's description of a
         * VK_IMAGE_TYPE_3D image in the 3DTWIDDLED layout --
         * TEXSTATE_IMAGE_WORD0.textype = 3D (pvr_arch_tex_state.c:170-176),
         * width and height level 0's API extent (:230-233), the twiddled
         * IMAGE_WORD1 with depth = depth - 1 (:277-282), the chain's level
         * count and mipmaps_present. The sampler's address mode on the third
         * axis is the texture's WRAP_R (`tex_desc_sampler`). The image meta
         * layer size is the copy's bytes (a volume has one "layer"). The
         * level count is the levels the copy holds and the sampler may walk:
         * a one-level draw gets a one-level copy. */
        let iw0 = tex_image_word0(fmt, csb::ROGUE_TEXSTATE_TEXTYPE_3D, t.width, t.height);
        let depth = u32::from(t.layers).max(1);
        let iw1 = tex_twiddled_image_word1(false, levels.min(15), (depth - 1).min(2047), levels > 1, t.base_gpu);
        w[0..2].copy_from_slice(&iw0);
        w[2..4].copy_from_slice(&iw1);
        w[4] = t.bytes;
        tex_desc_sampler(w, t, levels);
        return;
    }
    let iw0 = tex_image_word0(fmt, fmt.textype, t.width, t.height);
    /* the STRIDE word for a linear image; the ETC formats' twiddled word,
     * where [60:63] is baselevel (0) and the level count is [0:3] */
    let twiddled = is_etc2(t.format);
    let iw1 = if twiddled {
        tex_twiddled_image_word1(true, levels & 0xf, 0, levels > 1, t.base_gpu)
    } else {
        tex_stride_image_word1(t.stride_texels, levels, t.base_gpu, fmt.alpha_msb)
    };

    w[0..2].copy_from_slice(&iw0);
    w[2..4].copy_from_slice(&iw1);
    tex_desc_sampler(w, t, levels);
}

/// Sampler words 8 and 9 of a descriptor: address modes, filters and the LOD
/// range over `levels` levels; and sampler meta word 0 (dword 12), the depth
/// compare a shadow look-up reads when it runs (Mesa's
/// PCO_SAMPLER_META_COMPARE_OP; the online compiler's shadow_compare).
fn tex_desc_sampler(w: &mut [u32], t: &GlesTextureBind, levels: u32) {
    w[12] = t.compare_op as u32;
    /* Sampler meta word 1 (dword 13, Mesa's PCO_SAMPLER_META_RSVD0, which
     * no pco program reads), bit 0: the shadow look-up compares per texel
     * (the online compiler's PCF arm, select.rs shadow_compare) -- a
     * LINEAR/LINEAR sampler over one level, where the bilinear footprint
     * at level 0 is the whole filter (docs/es3/pvr-2darray.md section 7.2).
     * A mipmap filter, and a sampler whose min and mag filters differ,
     * keep the filtered-then-compared depth. */
    if levels == 1 && t.linear_filter != 0 && t.min_linear != 0 {
        w[13] = 1;
    }
    let filter =
        |linear: u8| if linear != 0 { csb::ROGUE_TEXSTATE_FILTER_LINEAR } else { csb::ROGUE_TEXSTATE_FILTER_POINT };
    let mut maxlod = 0;
    if levels > 1 {
        /* The LOD range covers the chain and stops at its last level: the
         * levels past it are memory nobody wrote (an application may declare
         * a full chain and fill part of it), and the GL core already clamped
         * `levels` to what was written. u4.6, so 64 per level. */
        maxlod = ((levels - 1) * 64).min(TEX_SAMPLER_MAXLOD_MAX);
    } else if t.min_linear != t.linear_filter {
        /* The TPU chooses between the min and mag filters on the CLAMPED LOD
         * (<= 0 magnifies, > 0 minifies; Mesa pvr_arch_sampler.c:88-97), and
         * GL chooses on the unclamped one. With maxlod 0 every pixel of a
         * single-level texture magnified, so GL_TEXTURE_MIN_FILTER was never
         * applied: dEQP texture.filtering.2d linear_nearest_* and
         * nearest_linear_* failed on the AM62P with either compiler. A max of
         * LOD 1.0 lets the clamped LOD be positive; the image descriptor still
         * has one level, so the texel still comes from level 0. */
        maxlod = 64;
    }
    let w0 = csb_pack!(
        ROGUE_TEXSTATE_SAMPLER_WORD0_pack,
        2,
        csb::ROGUE_TEXSTATE_SAMPLER_WORD0 {
            /* LOD bias 0, minlod 0 */
            dadjust: csb::ROGUE_TEXSTATE_DADJUST_ZERO_UINT,
            minlod: csb::ROGUE_TEXSTATE_CLAMP_MIN,
            maxlod,
            addrmode_u: tpu_addrmode(t.wrap_s),
            addrmode_v: tpu_addrmode(t.wrap_t),
            /* the third axis is sampled only by the native 3D texture; the
             * 2D, emulated cube and array targets address inside one tall 2D
             * image and never read it */
            addrmode_w: if t.type_ == GLES_TEXTYPE_3D_TWIDDLED {
                tpu_addrmode(t.wrap_r)
            } else {
                csb::ROGUE_TEXSTATE_ADDRMODE_CLAMP_TO_EDGE
            },
            magfilter: filter(t.linear_filter),
            minfilter: filter(t.min_linear),
            mipfilter: levels > 1 && t.mip_filter == 2,
            ..Default::default()
        }
    );
    w[8] = w0[0];
    w[9] = w0[1];
}

/* ------------------------------------------------------------------------- *
 * draw
 * ------------------------------------------------------------------------- */
/// Largest vertex count one sequential (non-indexed) index list can address:
/// the indices are u16 (the silicon-proven INDEX_LIST encoding).
const SEQ_MAX_VERTICES: u32 = 0xFFFF;
/// Pieces a split sequential draw may take (at most ~4.2M vertices).
const MAX_PIECES: u32 = 64;

/// The sample count a frame of `samples` renders with (the draw table's 0 and
/// any unsupported count are single-sampled).
fn frame_samples(samples: u8) -> u32 {
    if samples == 4 {
        4
    } else if samples == 2 {
        2
    } else {
        1
    }
}

/// Checks of the parts of a C-ABI draw the recorder would otherwise index or
/// trust blindly (the table is filled by the GL core, but also by the
/// on-silicon test suite and any other C caller).
fn draw_is_well_formed(d: &GlesDraw) -> bool {
    /* zero attributes is legal: a vertex shader may build its geometry from
     * gl_VertexID alone (upstream b879c58) */
    if d.num_attribs as u32 > PVR_PDS_VERTEX_MAX_STREAMS {
        return false;
    }
    /* staged indices must cover the indices the GPU reads (the core stages
     * index_count * 2 bytes; a wrapped product is caught here too) */
    if !d.index_cpu.is_null() && d.index_bytes != 0 && ((d.index_bytes / 2) as u64) < d.index_count as u64 {
        return false;
    }
    true
}

/// Whether `d` cannot join the frame that is open, so the frame is submitted
/// first. Everything the frame fixed at `frame_begin` is here: the colour
/// target, its size, the sample count, how many attachments it has, and which
/// depth/stencil buffer is behind it. A clear after the frame's own draws
/// ends it too -- the frame has one background clear, at its start.
///
/// The depth/stencil buffer is the one that is easy to miss: two framebuffer
/// objects can share a colour buffer and keep depth/stencil of their own, so
/// the colour address, the size and the sample count all agree while the
/// draws must not land in one frame (dEQP's fbo.render.shared_colorbuffer and
/// .shared_depthbuffer).
fn frame_ends_here(b: &Backend, d: &GlesDraw, any_clear: bool) -> bool {
    /* (the C code compared the raw sample count with the frame's normalised
     * one, so a draw with an unsupported count, e.g. 3, split every frame) */
    let dtargets = if d.num_targets != 0 { d.num_targets } else { 1 };

    b.target_gpu != d.target_gpu
        || b.target_w != d.target_w
        || b.target_h != d.target_h
        || b.target_pitch != crate::backend::draw_pitch(d)
        || b.samples != frame_samples(d.samples)
        || b.num_targets != dtargets.min(PVR_BACKEND_MAX_RT as u8)
        /* the other attachments too: a frame stores the slots it began with,
         * so a draw into (A, none, C) after one into (A, B, C) -- or into
         * (A, D, C) -- is another frame, or its output 1 would land in B */
        || (1..b.num_targets as usize).any(|i| {
            let (x, y) = (&b.targets[i], &d.targets[i]);
            (x.gpu, x.w, x.h, x.format) != (y.gpu, y.w, y.h, y.format)
        })
        || b.frame_ds_persist != d.ds_persist
        || (d.ds_persist && b.frame_ds_id != d.ds_id)
        /* apart, a respecified object (same names, a new stamp) is another
         * buffer, and so another frame */
        || (d.ds_persist && split_zs_on() && b.frame_zs != [d.depth_id, d.depth_gen, d.stencil_id, d.stencil_gen])
        || (any_clear && b.ndraws > b.frame_quads)
        /* A colour clear into a frame whose first object reloads the old
         * colour: made the background clear, it would be painted over by
         * that load quad, which the frame draws first. dEQP color_clear's
         * scissored cases (a clear under a box that misses the target opened
         * the frame, then a full clear followed) and glColorMask(0,0,0,0) +
         * glClear(COLOR|DEPTH) before a colour clear both read back the
         * picture from before the clear (AM62P, 2026-09-24). */
        || (any_clear && d.do_clear && b.frame_loads_color)
}

#[no_mangle]
pub unsafe extern "C" fn pvr_backend_draw(draw: *const GlesDraw) -> i32 {
    // SAFETY: single-threaded by contract; taken once per entry point.
    let p = unsafe { PVR.get() };
    // SAFETY: the caller passes null or a valid draw that outlives the call.
    let d = unsafe { draw.as_ref() };

    let (true, Some(d)) = (p.b.inited, d) else {
        return -EINVAL;
    };
    if d.target_w == 0 || d.target_h == 0 {
        return -EINVAL;
    }
    /* No colour target: a depth/stencil-only framebuffer
     * (DRAWS_WITHOUT_COLOUR), a z-only frame. With no depth or stencil
     * either there is nothing the draw could write. */
    if d.target_gpu == 0 && d.depth_id == 0 && d.stencil_id == 0 {
        return -EINVAL;
    }
    /* Draw buffer 0 GL_NONE while a later one is written
     * (glDrawBuffers({GL_NONE, GL_COLOR_ATTACHMENT1}), ES 3.0 4.2.1): the
     * list starts at attachment 1, but the program's output registers are
     * laid out from output 0 and the emit reads them in target order, so
     * the draw would store output 0. The remap is not built; refused by
     * name, never drawn with the wrong output (review R-09). */
    if d.targets[0].out_shift != 0 {
        if !p.b.warned_draw_buffer0 {
            p.b.warned_draw_buffer0 = true;
            glog!("yakogl/pvr: draw refused: draw buffer 0 is GL_NONE while a later draw buffer is written");
        }
        return -util::ENOTSUP;
    }
    /* An integer colour attachment beside another one: the program's
     * integer output is packed for one target's layout (the variant key
     * carries one), and the MRT path packs RGBA8. Refused by name, never
     * drawn with the wrong packing. */
    let ntargets = (d.num_targets as usize).min(d.targets.len());
    if ntargets > 1 && d.targets[..ntargets].iter().any(|t| t.gpu != 0 && gles_surf_is_integer(t.format)) {
        glog_once!("yakogl/pvr: draw refused: an integer colour attachment beside another colour attachment");
        return -util::ENOTSUP;
    }
    /* Several sRGB attachments are drawn as RGBA8 ones whose bytes are
     * already encoded ([`srgb_targets_as_rgba8`]): an sRGB attachment beside
     * one of another format is refused by name (the variant is keyed for one
     * class), and so is a program that does not encode -- the one-target
     * sRGB variant of a blend or a colour mask writes halves, which the RGBA8
     * attachments would take for bytes. The back end's own programs (the
     * frame's load and clear) are RGBA8's then. */
    let several_srgb;
    let d = if ntargets > 1 && d.targets[..ntargets].iter().any(|t| t.gpu != 0 && gles_surf_is_srgb(t.format)) {
        if d.targets[..ntargets].iter().any(|t| t.gpu != 0 && !gles_surf_is_srgb(t.format)) {
            glog_once!("yakogl/pvr: draw refused: an SRGB8_ALPHA8 colour attachment beside one of another format");
            return -util::ENOTSUP;
        }
        /* the program of a draw that runs one (a clear runs the back end's) */
        let key = if d.index_count != 0 && !d.clear_quad {
            // SAFETY: the GL core's draw points `fs` at a live record or at nothing.
            unsafe { d.fs.as_ref() }.map(|fs| fs.blend_key)
        } else {
            None
        };
        if key.is_some_and(|k| k != shaderc::SHC_BLEND_KEY_SRGB_ENCODED && k != shaderc::SHC_BLEND_KEY_KEEP) {
            glog_once!(
                "yakogl/pvr: draw refused: several SRGB8_ALPHA8 attachments with a blend or a colour mask (not built)"
            );
            return -util::ENOTSUP;
        }
        /* A scissored or masked colour clear is a quad whose program writes
         * pixel output 0 alone (`record_clear_quad`): the other attachments
         * would be stored from registers nothing wrote. The full clear is the
         * background object's, which writes every attachment. */
        if d.clear_quad && d.do_clear {
            glog_once!(
                "yakogl/pvr: clear refused: a scissored or masked colour clear of several SRGB8_ALPHA8 attachments"
            );
            return -util::ENOTSUP;
        }
        several_srgb = srgb_targets_as_rgba8(d);
        &several_srgb
    } else {
        d
    };
    /* One instance of an instanced draw the GL core draws instance by
     * instance (a divisor past max_instance_divisor): the PDS starts the
     * instance number at 0 for every draw, so a program that reads
     * gl_InstanceID would read 0 for instance_id. Refused by name, not drawn
     * wrong. */
    if d.instance_id != 0 && !d.vs.is_null() && unsafe { (*d.vs).vs_instance_id_reg1 } != 0 {
        static LOGGED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
        if !LOGGED.swap(true, core::sync::atomic::Ordering::Relaxed) {
            glog!(
                "yakogl/pvr: draw refused: an instance drawn on its own (a divisor past 1) whose program reads \
                 gl_InstanceID"
            );
        }
        return -EINVAL;
    }
    /* A clear quad whose box misses the target clears nothing: it must not
     * open (or end) a frame either -- a frame it opened would begin with a
     * colour-load quad for nothing (see draw_box). The box is judged against
     * the whole target, as record_clear_quad draws it: a clear is not bounded
     * by the application's viewport (ES 2.0 4.2.3). */
    let clear_box_empty = || {
        let mut whole = *d;
        whole.viewport = [0, 0, d.target_w as i32, d.target_h as i32];
        draw_box(&whole).is_none()
    };
    if d.clear_quad && clear_box_empty() {
        p.b.draws_empty_box = p.b.draws_empty_box.wrapping_add(1);
        return 0;
    }
    /* A colour clear under a partial mask that reaches the background
     * object -- alone, or fused into the draw that follows it -- cannot be
     * the background clear, which writes every channel: the colour goes to
     * the masked clear quad (recorded before the draw, where GL put the
     * clear), and the frame keeps the pixels the mask leaves alone. Depth
     * and stencil stay on the background path. */
    let partial = |m: u8| (m & 0xf) != 0 && (m & 0xf) != 0xf;
    if d.do_clear && !d.clear_quad && partial(d.clear_color_mask) {
        let mut rest = *d;
        rest.do_clear = false;
        rest.clear_color_mask = 0;
        let mut quad = *d;
        quad.clear_quad = true;
        quad.do_clear_depth = false;
        quad.do_clear_stencil = false;
        quad.scissor = false;
        quad.index_count = 0;
        /* the depth texels go into the buffer once, with the first part,
         * which opens the frame (the later parts would open another) */
        quad.depth_texels = core::ptr::null();
        let seeds = !rest.depth_texels.is_null();
        /* the depth/stencil part, and the frame, first; then the colour */
        if seeds
            || rest.do_clear_depth
            || rest.do_clear_stencil
            || !p.b.frame_open
            || frame_ends_here(&p.b, &rest, false)
        {
            let rc = draw_one(p, &GlesDraw { index_count: 0, ..rest });
            if rc != 0 {
                return rc;
            }
        }
        let rc = draw_one(p, &quad);
        if rc != 0 || rest.index_count == 0 {
            return rc;
        }
        let tail = GlesDraw { do_clear_depth: false, do_clear_stencil: false, depth_texels: core::ptr::null(), ..rest };
        return draw_one(p, &tail);
    }
    draw_one(p, d)
}

/// One draw (or clear) the GL core sent, the masked-clear split already made.
fn draw_one(p: &mut Pvr, d: &GlesDraw) -> i32 {
    /* Frame boundaries: target change or a clear after draws => new frame.
     * A clear quad is an ordinary object in the frame, not the frame's
     * background clear: it must not force a flush and must not overwrite the
     * frame's clear values. */
    let any_clear = !d.clear_quad && (d.do_clear || d.do_clear_depth || d.do_clear_stencil);
    /* depth texels the CPU wrote go into the buffer before a frame loads
     * it: such a draw always opens a frame of its own */
    let seeds = !d.depth_texels.is_null() && d.ds_persist;
    let ends = |b: &Backend| b.frame_open && (seeds || frame_ends_here(b, d, any_clear));

    /* a frame this draw opens needs its depth/stencil buffers: made room
     * for, or the draw is refused before anything is recorded */
    if !p.b.frame_open || ends(&p.b) {
        let rc = ds_make_room(&mut p.b, d);
        if rc != 0 {
            return rc;
        }
    }
    if ends(&p.b) {
        let rc = flush(p);
        if rc != 0 {
            return public_rc(rc);
        }
    }
    /* the frame's depth/stencil (found now for a frame this draw opens), the
     * texels it starts from, and the refusal of a draw that uses one it
     * cannot keep, before the frame takes anything from the draw */
    let zs = if p.b.frame_open { (p.b.frame_ds, p.b.frame_s) } else { ds_resolve(&mut p.b, d) };
    if seeds {
        let rc = ds_seed_depth(&mut p.b, d, zs);
        if rc != 0 {
            return rc;
        }
    }
    let rc = ds_refuse_lost(&mut p.b, d, zs);
    if rc != 0 {
        return rc;
    }
    /* A program that writes gl_FragDepth drawn to a target with no colour
     * attachment: Mesa's pvr runs it by clearing the job's z-only render
     * (`pvr_arch_cmd_buffer.c` :1689 `job->z_only_render = ... &&
     * !has_depth_feedback`) and giving the draw the depth-feedback pass
     * (`pvr_arch_pipeline.c` :1351); this backend's z-only frame is built
     * whole from the target at flush and has no depth-feedback variant of
     * it, so the draw is refused by name here -- before the frame takes
     * anything, including a clear, from it -- rather than leave the written
     * depth unwritten. */
    // SAFETY: as in record_draw: d.fs is null (skipped here, refused there)
    // or a shader descriptor that outlives the call.
    if d.target_gpu == 0 && !d.fs.is_null() && unsafe { (*d.fs).writes_depth } != 0 {
        glog_once!("yakogl/pvr: draw refused: gl_FragDepth in a draw to a framebuffer without a colour attachment");
        return -EINVAL;
    }
    if !p.b.frame_open {
        frame_begin_with(p, d, zs);
    } else if any_clear {
        let b = &mut p.b;

        /* a clear before any draw of this frame: the frame starts from it */
        if d.do_clear {
            b.frame_cleared = true;
            b.clear_rgba = d.clear_rgba;
        }
        if d.do_clear_depth {
            b.clear_depth = d.clear_depth;
            b.load_depth = false;
            b.frame_clr[DEPTH] = Plane::cleared(d.clear_depth.to_bits());
        }
        if d.do_clear_stencil {
            b.clear_stencil = d.clear_stencil;
            b.load_stencil = false;
            b.frame_clr[STENCIL] = Plane::cleared(u32::from(d.clear_stencil));
        }
        /* the depth/stencil clear values must reach every pixel */
        let clear_float = d.do_clear && clears_by_quad(b.target_format);
        let has_ds = b.frame_ds.buf().is_some();

        if (has_ds && (d.do_clear_depth || d.do_clear_stencil)) || clear_float {
            let _ =
                record_frame_quad(p, d, false, has_ds && d.do_clear_depth, has_ds && d.do_clear_stencil, clear_float);
        }
    }

    let writes = ds_retention::draw_writes(d);
    p.b.frame_wrote = [p.b.frame_wrote[DEPTH] || writes[DEPTH], p.b.frame_wrote[STENCIL] || writes[STENCIL]];
    if d.clear_quad {
        let rc = record_clear_quad(p, d);
        if rc == 0 {
            ds_lift_cleared(&mut p.b, d, zs);
        }
        return public_rc(rc);
    }
    /* the frame carries the background clear from here on */
    ds_lift_cleared(&mut p.b, d, zs);
    if d.index_count == 0 {
        return 0; /* clear-only: the frame carries the clear */
    }
    if d.vs.is_null() || d.fs.is_null() || d.vs_usc_gpu == 0 || d.fs_usc_gpu == 0 {
        glog_once!("pvr_backend: draw without linked shaders");
        return -EINVAL;
    }
    if !draw_is_well_formed(d) {
        glog_once!("pvr_backend: malformed draw ({} attributes, {} indices)", d.num_attribs, d.index_count);
        return -EINVAL;
    }
    public_rc(draw_sized(p, d))
}

/* ------------------------------------------------------------------------- *
 * glGenerateMipmap on the GPU
 *
 * The GL core box-filters a chain on the CPU (gl/texture.rs glGenerateMipmap)
 * unless the backend makes it (`YakoglBackend::generate_mipmap`). Here each
 * level above 0 of a 2D RGBA8 chain is one kick: a frame of its own into the
 * level, holding one full-screen quad whose fragment program
 * (tools/shaders/internal/es/mipgen.frag) samples the level above through a
 * one-level bilinear CLAMP_TO_EDGE sampler at the centre of each texel -- the
 * 2x2 box average where the level is half the one above (every level of a
 * power-of-two chain), the bilinear sample between the nearest four on a
 * NPOT chain's odd steps (ES 3.0.6 3.8.9 leaves the filter open). The kicks
 * go out in order behind the kick that rendered level 0 (the GL core hands
 * the texture over after it) and nothing is waited for. The levels are
 * written straight into the GL core's storage at the offsets the TPU walks
 * (`mip_chain_layout`), so the GL core asks nothing back but a wait
 * (`pvr_return_mipmap_chain`) before it reads or writes them on the CPU.
 *
 * Why: glmark2's terrain (800x600, 10 levels) and refract (1600x1200, 11
 * levels) ask for their chains every frame, and the CPU filter cost them
 * 1467 and 2046 ms of GL-call time a frame on the AM62P (main
 * `e3749a25`).
 * ------------------------------------------------------------------------- */

/// One level of a 2D RGBA8 chain as the TPU walks it from level 0's address
/// and stride: `off` bytes from level 0, the GL extent `w` x `h` in the
/// top-left of a `pitch` x `rows` physical level ([`mip_chain_layout`]).
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
struct MipLevel {
    off: u32,
    w: u32,
    h: u32,
    pitch: u32,
    rows: u32,
}

/// The first `levels` levels of the chain of a `w` x `h` RGBA8 2D texture
/// whose level 0 rows are `stride` texels apart, as the TPU walks them from
/// level 0's address (Mesa 26.2.1 pvr `pvr_image_plane_init_physical_extent`
/// and `pvr_image_plane_setup_mip_levels`, the rules gl/texture.rs
/// `chain_layout_npot` and `pot_chain_layout` lay the storage out by): the
/// physical extent is each side rounded up to a power of two, level l is
/// max(pw >> l, 1) texels a row -- a 1-wide level above a wider base one
/// texel a row, a 1-wide base pair-padded (`one_wide_level_pitch`) -- by
/// max(ph >> l, 1) rows, and each level follows the one before. `None` for
/// a size of 0, fewer than two levels, more than `out` holds, a `stride`
/// that is not level 0's pitch by that rule (a layout this backend did not
/// ask for), or offsets past u32.
fn mip_chain_layout(w: u32, h: u32, stride: u32, levels: u32, out: &mut [MipLevel]) -> Option<usize> {
    if w == 0 || h == 0 || levels < 2 || levels as usize > out.len() {
        return None;
    }
    let (pw, ph) = (w.checked_next_power_of_two()?, h.checked_next_power_of_two()?);
    let mut off: u32 = 0;
    for l in 0..levels {
        let pitch = one_wide_level_pitch(pw, pot_level_side(pw, l), true);
        let rows = pot_level_side(ph, l);
        if l == 0 && pitch != stride {
            return None;
        }
        out[l as usize] = MipLevel { off, w: pot_level_side(w, l), h: pot_level_side(h, l), pitch, rows };
        off = off.checked_add(pitch.checked_mul(rows)?.checked_mul(RGBA8_BYTES)?)?;
    }
    Some(levels as usize)
}

/// Bytes per RGBA8 texel.
const RGBA8_BYTES: u32 = 4;

/// The draw of level `dst` of the chain at `base` from level `src`
/// ([`pvr_generate_mipmap`]): a frame of its own into `dst` -- the level's
/// own pitch, a background clear so the frame loads no colour first -- with
/// one full-screen quad over the level's extent, the fragment program
/// [`PVR_INTERNAL_MIPGEN`] sampling `src` bound as a one-level
/// linear-filtered CLAMP_TO_EDGE texture at gl_FragCoord.xy / the level's
/// size (its uniform block).
///
/// The frame's target is `hwrt` wide and high -- level 1's size, for every
/// level of the chain -- and the store is clipped to the level
/// (`Backend::mip_clip`, the PBE's maxclip): the driver keeps one hardware
/// render-target set per target size and recycles a set through the
/// firmware when they run out, and a chain of nine sizes, eight times a
/// frame (glmark2's terrain), recycled one for nearly every kick (the first
/// board run, 2026-10-16: 3.7 s a frame). The tiles past the level hold the
/// background alone and store nothing.
///
/// Kept in a static of its own (`MIP_DRAW`), as the frame quad's draw is
/// kept off the stack, because [`draw_one`] mutates the backend while it
/// reads the draw. Through `draw_one`, so the frame still open (the one
/// that rendered level 0, or the level before) is submitted first. 0, or
/// the draw's error.
fn record_mip_level(p: &mut Pvr, base: u64, src: &MipLevel, dst: &MipLevel, hwrt: (u32, u32)) -> i32 {
    // SAFETY: single-threaded by contract; `d` is read by draw_one alone
    // until it returns, and nothing else refers to the static meanwhile.
    let d = unsafe { MIP_DRAW.get() };
    let fl = &mut p.fbload;
    fl.mip_scale = [1.0 / dst.w as f32, 1.0 / dst.h as f32];
    let scale: *const f32 = fl.mip_scale.as_ptr();
    let (vs_sh, fs_sh): (*const ShcShader, *const ShcShader) = (&fl.vs_sh, &fl.mip_fs_sh);

    /* the frame this draw opens clips its store to the level */
    p.b.mip_clip = (dst.w, dst.h);
    *d = GlesDraw::ZERO;
    d.target_gpu = base.wrapping_add(dst.off as u64);
    d.target_w = hwrt.0.max(dst.w);
    d.target_h = hwrt.1.max(dst.h);
    d.target_pitch = dst.pitch;
    d.target_format = GLES_SURF_RGBA8888;
    d.target_alpha_bits = 8; /* the sampled alpha, as the CPU filter averages it */
    /* the frame's background clear: the quad covers every pixel, so the
     * colour never shows, and the frame loads nothing first */
    d.do_clear = true;
    d.clear_color_mask = 0xf;
    d.prim = GLES_PRIM_TRIANGLES;
    d.index_count = 6;
    d.vs = vs_sh;
    d.fs = fs_sh;
    d.vs_usc_gpu = p.b.fbload_vs.gpu;
    d.fs_usc_gpu = p.b.mipgen_fs.gpu;
    d.attribs[0] =
        GlesAttrib { vi_reg: 0, components: 4, stride: 16, buffer_gpu: p.b.fbload_quad.gpu, ..GlesAttrib::ZERO };
    d.num_attribs = 1;
    d.vs_output_size_dwords = 4;
    d.textures[0] = GlesTextureBind {
        unit: 0,
        format: GLES_TEXFMT_RGBA8,
        type_: GLES_TEXTYPE_2D,
        layers: 1,
        base_gpu: base.wrapping_add(src.off as u64),
        width: src.w,
        height: src.h,
        stride_texels: src.pitch,
        bytes: src.pitch.wrapping_mul(src.rows).wrapping_mul(RGBA8_BYTES),
        linear_filter: 1,
        min_linear: 1,
        mip_filter: 0,
        mip_levels: 1,
        wrap_s: GLES_WRAP_CLAMP_TO_EDGE,
        wrap_t: GLES_WRAP_CLAMP_TO_EDGE,
        ..GlesTextureBind::ZERO
    };
    d.num_textures = 1;
    d.viewport = [0, 0, dst.w as i32, dst.h as i32];
    d.depth_range = [0.0, 1.0];
    d.depth_func = GLES_CMP_ALWAYS;
    d.fs_uniform_cpu = scale as *const c_void;
    d.fs_uniform_dwords = 2;
    d.color_write = true;
    d.color_mask = 0xf;
    d.line_width = 1.0;
    let rc = draw_one(p, d);
    /* the clip lives for this draw alone: a draw refused before it opened
     * its frame must not clip the next unrelated frame's store */
    p.b.mip_clip = (0, 0);
    rc
}

/// [`record_mip_level`]'s draw: all-zero is a valid `GlesDraw` (integers,
/// floats, bools, null pointers), as `PVR` is.
static MIP_DRAW: Global<GlesDraw> = Global::zeroed();

/// [`pvr_generate_mipmap`]: the chain, or why it is left to the GL core.
fn mip_chain_gpu(p: &mut Pvr, t: &GlesTextureBind, levels: u32) -> Result<(), &'static str> {
    if t.format != GLES_TEXFMT_RGBA8 || t.bgra != 0 {
        return Err("not a plain RGBA8 texture");
    }
    if t.type_ != GLES_TEXTYPE_2D || t.layers > 1 {
        return Err("not a 2D texture");
    }
    /* the GENERAL heap's u32 offsets, as the driver's pool maps them: a
     * texture over 256 KiB sits in the top region, at a heap offset of 1 GiB
     * (TOP_VA, driver/rust/src/gpu/pvr/mem.rs) plus its carve-out offset --
     * every chain glmark2 asks for is there (the first board run of this
     * path refused them all under a 1 GiB window, 2026-10-16) */
    if t.base_gpu < HEAP_GENERAL_BASE || t.base_gpu - HEAP_GENERAL_BASE > u64::from(u32::MAX) {
        return Err("an image address outside the heap");
    }
    if p.b.mipgen_fs.impl_.is_null() {
        return Err("no mipmap program");
    }
    let mut lv = [MipLevel::default(); CAP_MIP_LEVELS];
    let n = mip_chain_layout(t.width, t.height, t.stride_texels, levels, &mut lv)
        .ok_or("a chain layout this backend does not walk")?;
    /* the PBE writes rows in pixel pairs (`pbe_reg_words` linestride): a
     * level one texel a row with rows under it cannot be its target */
    if lv[1..n].iter().any(|l| l.pitch % 2 != 0 && l.rows > 1) {
        return Err("a level one texel wide above a taller tail: the PBE writes pixel pairs");
    }
    for l in 1..n {
        let rc = record_mip_level(p, t.base_gpu, &lv[l - 1], &lv[l], (lv[1].w, lv[1].h));
        if rc != 0 {
            /* the levels recorded so far may be in flight, and the CPU
             * filter is about to rewrite them: wait them out first */
            let _ = flush(p);
            let last = p.b.last_fence;
            let _ = p.b.wait_fence(last);
            return Err("a level's draw was refused");
        }
    }
    if flush(p) != 0 {
        let last = p.b.last_fence;
        let _ = p.b.wait_fence(last);
        return Err("the chain's last kick did not submit");
    }
    p.b.mip_fence = p.b.last_fence;
    Ok(())
}

/// glGenerateMipmap on the 2D texture `b` describes, whose storage holds a
/// chain of `levels` levels (`YakoglBackend::generate_mipmap`): a plain
/// RGBA8 2D texture at a heap address gets its levels 1.. drawn on the GPU
/// ([`mip_chain_gpu`], one kick per level, every kick submitted before this
/// returns) and the answer is 1: the GL core filters nothing. 0 -- the GL
/// core's CPU box filter, as before -- for any other texture (another
/// format, B,G,R,A, a cube or array layer, an address outside the heap), a
/// chain whose layout this backend does not walk (`mip_chain_layout`), a
/// chain with a 1-wide level above a taller tail (the PBE's pixel pairs), a
/// level's draw refused (the kicks recorded so far are waited for, since the
/// CPU filter rewrites their levels), or a backend that is not running.
/// Logged once with the reason, and counted.
///
/// # Safety
/// `b` is null or points at a binding that outlives the call.
pub unsafe extern "C" fn pvr_generate_mipmap(b: *const GlesTextureBind, levels: u32) -> i32 {
    // SAFETY: single-threaded by contract; taken once per entry point.
    let p = unsafe { PVR.get() };
    // SAFETY: the caller's contract.
    let Some(t) = (unsafe { b.as_ref() }) else { return 0 };
    if !p.b.inited {
        return 0;
    }
    match mip_chain_gpu(p, t, levels) {
        Ok(()) => {
            p.b.mip_chains = p.b.mip_chains.wrapping_add(1);
            if p.b.mip_chains == 1 {
                glog!(
                    "pvr_backend: the GPU made the {}-level chain of a {}x{} texture: glGenerateMipmap filters nothing",
                    levels,
                    t.width,
                    t.height
                );
            }
            1
        }
        Err(why) => {
            p.b.mip_chains_refused = p.b.mip_chains_refused.wrapping_add(1);
            if p.b.mip_chains_refused == 1 {
                glog!(
                    "pvr_backend: the {}-level chain of a {}x{} texture (format {}) is left to the CPU: {}",
                    levels,
                    t.width,
                    t.height,
                    t.format,
                    why
                );
            }
            0
        }
    }
}

/// The GL core is about to read or write levels 1.. of a texture on the CPU
/// whose last glGenerateMipmap this backend made (`YakoglBackend::
/// return_mipmap_chain`): the levels are in the storage already, written by
/// the chain's kicks, so the wait for the last of them is all there is.
///
/// # Safety
/// `b` is null or points at a binding that outlives the call.
pub unsafe extern "C" fn pvr_return_mipmap_chain(_b: *const GlesTextureBind, _levels: u32) {
    // SAFETY: single-threaded by contract; taken once per entry point.
    let b = unsafe { &mut PVR.get().b };
    if !b.inited || b.mip_fence == 0 {
        return;
    }
    let fence = b.mip_fence;
    if b.wait_fence(fence) != 0 {
        glog_once!("pvr_backend: the kicks that made a mipmap chain did not complete: the CPU reads what they left");
    }
}

/// The GPU mipmap chains made, and asked for but left to the CPU, since the
/// backend started ([`pvr_generate_mipmap`]).
///
/// # Safety
/// `made` and `refused` must each be null or valid for a write.
#[no_mangle]
pub unsafe extern "C" fn pvr_backend_mip_chains(made: *mut u32, refused: *mut u32) {
    // SAFETY: single-threaded by contract; taken once per entry point.
    let b = unsafe { &PVR.get().b };
    // SAFETY: null or valid, per the contract above.
    unsafe {
        if !made.is_null() {
            *made = b.mip_chains;
        }
        if !refused.is_null() {
            *refused = b.mip_chains_refused;
        }
    }
}

/// Primitives one instance of this draw puts through the tiler.
fn draw_prims(d: &GlesDraw) -> u32 {
    let n = match d.prim {
        GLES_PRIM_TRI_STRIP | GLES_PRIM_TRI_FAN => d.index_count.saturating_sub(2),
        GLES_PRIM_LINES => d.index_count / 2,
        GLES_PRIM_LINE_STRIP => d.index_count.saturating_sub(1),
        _ => d.index_count / 3,
    };
    if n != 0 {
        n
    } else {
        1
    }
}

/// Move every per-instance attribute on to instance `first`: the stream the
/// PDS DMAs is addressed by the instance number, which starts at 0 again in
/// each chunk, so the chunk's base has to move instead.
///
/// The offsets are computed with checked arithmetic (the C wraps them): an
/// attribute whose base would leave its object is emptied rather than handed
/// to the GPU as a wild address.
fn attribs_advance_instances(dd: &mut GlesDraw, first: u32) {
    if first == 0 {
        return;
    }
    for a in dd.attribs[..dd.num_attribs as usize].iter_mut() {
        if a.per_instance == 0 {
            continue;
        }
        let Some(skip) = first.checked_mul(a.stride) else {
            a.cpu_bytes = 0;
            a.offset = u32::MAX;
            continue;
        };
        if !a.cpu.is_null() {
            if skip as usize > a.cpu_bytes as usize {
                a.cpu_bytes = 0;
                continue;
            }
            // SAFETY: `skip <= cpu_bytes`, so the new base is inside the
            // staged copy (or one past its end, which `cpu_bytes = 0` makes
            // unreadable).
            a.cpu = unsafe { (a.cpu as *const u8).add(skip as usize) } as *const c_void;
            a.cpu_bytes -= skip;
        } else {
            a.offset = a.offset.saturating_add(skip);
        }
    }
}

/// Record the draw, splitting the frame when it alone would tile more than a
/// kick's parameter buffer holds. An instanced draw splits by instances; a
/// single non-instanced draw that big goes through whole (nothing here can
/// slice an index list without breaking strips and fans).
fn draw_sized(p: &mut Pvr, d: &GlesDraw) -> i32 {
    let budget = if p.b.opt(PVR_BACKEND_OPT_PRIM_BUDGET) != 0 {
        p.b.opt(PVR_BACKEND_OPT_PRIM_BUDGET)
    } else {
        PRIMS_PER_KICK_MAX
    };
    let inst = if d.instance_count != 0 { d.instance_count } else { 1 };
    let per = draw_prims(d);
    let total = match per.checked_mul(inst) {
        Some(t) if t <= budget => t,
        _ => budget.saturating_add(1),
    };

    if total <= budget && (p.b.prims == 0 || p.b.prims.saturating_add(total) <= budget) {
        p.b.prims = p.b.prims.saturating_add(total);
        return draw_dispatch(p, d);
    }
    /* Whatever follows continues the same picture: the clear belongs to the
     * kick that already carried it, and a second one would wipe it. */
    let mut cont = *d;

    cont.do_clear = false;
    cont.do_clear_depth = false;
    cont.do_clear_stencil = false;
    if p.b.ndraws > 0 {
        /* finish what is tiled first */
        let rc = flush(p);
        if rc != 0 {
            return rc;
        }
        p.b.splits_total = p.b.splits_total.wrapping_add(1);
        frame_begin(p, &cont);
    }
    /* gl_InstanceID counts from 0 in every chunk (the PDS writes the vertex
     * fetch's instance number, which each index list restarts), so a shader
     * that reads it would see the wrong instance. Such a draw goes through
     * whole -- Qt Quick 3D's instanced materials read their transform from a
     * per-instance attribute, which chunking does handle. */
    // SAFETY: checked non-null by pvr_backend_draw; the core's linked program.
    let reads_instance_id = !d.vs.is_null() && unsafe { (*d.vs).vs_instance_id_reg1 } != 0;
    if inst == 1 || per >= budget || reads_instance_id {
        p.b.prims = p.b.prims.saturating_add(total); /* cannot be split any finer */
        return draw_dispatch(p, &cont);
    }
    let chunk = (budget / per).max(1);
    let mut first = 0u32;
    while first < inst {
        let mut dd = cont;
        let n = chunk.min(inst - first);

        dd.instance_count = n;
        attribs_advance_instances(&mut dd, first);
        let rc = draw_dispatch(p, &dd);
        if rc != 0 {
            return rc;
        }
        p.b.prims = p.b.prims.saturating_add(n.saturating_mul(per));
        first += n;
        if first < inst {
            let rc = flush(p);
            if rc != 0 {
                return rc;
            }
            frame_begin(p, &dd);
        }
    }
    0
}

/// One draw into the open frame: whole, or cut into 16-bit index lists when a
/// sequential draw is longer than one list can address, or cut into pieces
/// when its staged data is too large.
///
/// A draw whose staged data (client indices and arrays) is more than half an
/// empty GENERAL arena is cut into pieces that each fit: an indexed one into
/// pieces of its index list ([`draw_indexed_pieces`]), a sequential one into
/// runs of its vertices ([`draw_sequential_pieces`]). Until the second, a
/// glDrawArrays of client arrays larger than an empty arena was refused
/// whole: dEQP-GLES3's builtin_functions.precision vec3/vec4 draws, 24,384
/// vertices of four arrays (1.27 MB), ended AM62P image 7 on f2a13877.
fn draw_dispatch(p: &mut Pvr, d: &GlesDraw) -> i32 {
    let sequential = d.index_buf_gpu == 0 && (d.index_cpu.is_null() || d.index_bytes == 0);
    if sequential && d.index_count > SEQ_MAX_VERTICES {
        draw_chunked(p, d)
    } else if staged_bytes(d) > p.b.gen.mem.size as u64 / 2 {
        if sequential {
            draw_sequential_pieces(p, d)
        } else {
            draw_indexed_pieces(p, d)
        }
    } else {
        draw_part(p, d, 0, d.index_count, None)
    }
}

/// How a draw of mode `prim` is cut on primitive boundaries: the vertices
/// (or indices) per piece beyond the first `overlap` are a multiple of
/// `unit`, and the next piece repeats the last `overlap`. A strip's two
/// (an even number of triangles per piece keeps the winding), a line
/// strip's one. `unit` 0: fans and loops, whose primitives reach back to the
/// first vertex, cannot be cut.
fn cut_unit(prim: GlesPrim) -> (u32, u32) {
    match prim {
        GLES_PRIM_POINTS => (1, 0),
        GLES_PRIM_LINES => (2, 0),
        GLES_PRIM_LINE_STRIP => (1, 1),
        GLES_PRIM_TRI_STRIP => (2, 2),
        GLES_PRIM_TRI_FAN | GLES_PRIM_LINE_LOOP => (0, 0),
        _ => (3, 0), /* a triangle list (record_draw's default topology) */
    }
}

/// What recording `d` whole copies into the GENERAL arena from the GL core's
/// staging: its client indices and every staged attribute.
fn staged_bytes(d: &GlesDraw) -> u64 {
    let idx = if d.index_cpu.is_null() { 0 } else { d.index_bytes as u64 };
    d.attribs[..(d.num_attribs as usize).min(d.attribs.len())]
        .iter()
        .filter(|a| !a.cpu.is_null())
        .fold(idx, |n, a| n + a.cpu_bytes as u64)
}

/// The arena bytes a piece of a draw copies: `n` indices (0 for a piece of a
/// sequential draw, which copies none) and, of each staged attribute,
/// vertices `vmin..=vmax` (a per-instance or constant one whole), each copy
/// 16-byte aligned as [`record_draw`] places it.
fn piece_bytes(d: &GlesDraw, n: u32, vmin: u32, vmax: u32) -> u64 {
    let pad = |b: u64| b + 15;
    d.attribs[..d.num_attribs as usize].iter().filter(|a| !a.cpu.is_null() && a.cpu_bytes != 0).fold(
        if n == 0 { 0 } else { pad(n as u64 * 2) },
        |sum, a| {
            let bytes = if a.per_instance != 0 || a.stride == 0 {
                a.cpu_bytes as u64
            } else {
                let from = vmin as u64 * a.stride as u64;
                let want = (vmax - vmin) as u64 * a.stride as u64 + a.components as u64 * 4;
                (a.cpu_bytes as u64).saturating_sub(from).min(want)
            };
            sum + pad(bytes)
        },
    )
}

/// The next piece of an indexed draw from index `start`: the most indices
/// (a multiple of `unit` beyond the `overlap` the next piece repeats) whose
/// copies stay within `budget` bytes, and the vertex range they read.
/// `None` when not even one primitive fits.
fn next_index_piece(
    d: &GlesDraw,
    idx: &[u16],
    start: usize,
    unit: usize,
    overlap: usize,
    budget: u64,
) -> Option<(u32, u32, u32)> {
    let (mut lo, mut hi) = (u32::MAX, 0u32);
    let mut best = None;
    let mut end = start;
    while end < idx.len() {
        /* the next primitive's indices (a strip's first piece takes its
         * first `overlap` vertices with its first primitive) */
        let step = if end == start { (unit + overlap).max(unit) } else { unit };
        let next = (end + step).min(idx.len());
        for &v in &idx[end..next] {
            lo = lo.min(v as u32);
            hi = hi.max(v as u32);
        }
        if piece_bytes(d, (next - start) as u32, lo, hi) > budget {
            break;
        }
        end = next;
        best = Some(((end - start) as u32, lo, hi - lo + 1));
    }
    best
}

/// An indexed draw whose client indices and arrays do not fit half an empty
/// GENERAL arena, drawn as pieces of its index list that do: each piece
/// copies its own indices and, of every staged attribute, only the vertex
/// window its indices read ([`record_draw`]'s `win`). dEQP's vertex-texture
/// grids are such draws: 127 x 127 quads of four client vertices each,
/// 64,516 vertices of two vec2 arrays and 96,774 indices, 1.2 MB against
/// the 1 MiB default arena (dEQP-GLES2.functional.texture.vertex.2d.*).
///
/// Lists cut anywhere on a primitive boundary; strips repeat the vertices
/// the next piece continues from, an even number of triangles per piece
/// keeping the winding, as [`draw_chunked`]. Fans and loops, whose
/// primitives reach back to the first vertex, and a draw with a second index
/// list go whole (refused when they do not fit). A draw whose indices are
/// scattered so widely that one primitive's window does not fit is refused
/// the same way. A failure after some pieces were recorded leaves those in
/// the frame.
fn draw_indexed_pieces(p: &mut Pvr, d: &GlesDraw) -> i32 {
    let (unit, overlap) = cut_unit(d.prim);
    let (unit, overlap) = (unit as usize, overlap as usize);
    let second = d.index_count2 != 0 && d.index_buf2_gpu != 0;
    if unit == 0
        || second
        || d.index_cpu.is_null()
        || !(d.index_cpu as usize).is_multiple_of(2)
        || (d.index_bytes / 2) < d.index_count
    {
        return draw_part(p, d, 0, d.index_count, None);
    }
    // SAFETY: the core stages index_bytes bytes of u16 indices at index_cpu
    // for this draw (u16-aligned scratch), and index_count <= index_bytes / 2
    // was just checked.
    let idx = unsafe { core::slice::from_raw_parts(d.index_cpu as *const u16, d.index_count as usize) };
    let budget = p.b.gen.mem.size as u64 / 2;
    /* count the pieces first: past MAX_PIECES the draw is refused before
     * anything is recorded (the bound draw_chunked keeps) */
    let mut pieces = 0u32;
    let mut start = 0usize;
    loop {
        let Some((n, _, _)) = next_index_piece(d, idx, start, unit, overlap, budget) else {
            return draw_part(p, d, 0, d.index_count, None); /* refused by name */
        };
        pieces += 1;
        if pieces > MAX_PIECES {
            if !p.b.warned_big {
                glog!(
                    "pvr_backend: an indexed draw of {} indices needs more than {} pieces to fit its client arrays: not drawn",
                    d.index_count,
                    MAX_PIECES
                );
                p.b.warned_big = true;
            }
            return -E2BIG;
        }
        if start + n as usize >= idx.len() {
            break;
        }
        start += n as usize - overlap;
    }
    /* as draw_chunked: a piece that opens a new frame loads the colour */
    let mut cont = *d;
    cont.do_clear = false;
    cont.do_clear_depth = false;
    cont.do_clear_stencil = false;
    let mut start = 0usize;
    loop {
        let Some((n, vmin, vcount)) = next_index_piece(d, idx, start, unit, overlap, budget) else {
            unreachable!("counted above");
        };
        let mut pd = if start == 0 { *d } else { cont };
        // SAFETY: start + n <= index_count indices of the staging.
        pd.index_cpu = unsafe { idx.as_ptr().add(start) } as *const c_void;
        pd.index_bytes = n * 2;
        pd.index_count = n;
        if pd.index_buf_gpu != 0 {
            pd.index_buf_gpu += start as u64 * 2;
        }
        let rc = draw_part(p, &pd, 0, n, Some((vmin, vcount)));
        if rc != 0 {
            return rc;
        }
        if start + n as usize >= idx.len() {
            return 0;
        }
        start += n as usize - overlap;
    }
}

/// The next piece of a sequential draw of `total` vertices from vertex
/// `start`: all the vertices left when their copies fit `budget` bytes,
/// else the most (`overlap` plus a multiple of `unit`, at least one
/// step) that do. `None` when not even one step fits.
fn next_seq_piece(d: &GlesDraw, total: u32, start: u32, unit: u32, overlap: u32, budget: u64) -> Option<u32> {
    let left = total - start;
    let fits = |n: u32| piece_bytes(d, 0, start, start + n - 1) <= budget;
    if fits(left) {
        return Some(left);
    }
    if left <= overlap + unit || !fits(overlap + unit) {
        return None;
    }
    /* the copies grow with the vertices: the largest k that fits, by
     * bisection (k = lo fits; past hi the piece would be the whole rest,
     * which does not) */
    let (mut lo, mut hi) = (1u32, (left - 1 - overlap) / unit);
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if fits(overlap + mid * unit) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    Some(overlap + lo * unit)
}

/// A sequential draw (glDrawArrays) of at most SEQ_MAX_VERTICES vertices
/// whose staged arrays do not fit half an empty GENERAL arena, drawn as runs
/// of its vertices that do: each piece copies, of every staged per-vertex
/// attribute, only its own vertices ([`record_draw`]'s `win`), and reads
/// the sequential index list from its first vertex on, so its indices --
/// and gl_VertexID with them -- are the draw's own and the VBO-resident
/// attributes are fetched as for the whole draw.
///
/// The cut is [`draw_indexed_pieces`]'s: lists on a primitive boundary,
/// strips repeating the vertices the next piece continues from; fans, loops
/// and a draw with a second index list go whole (refused by name when they
/// do not fit), as does a draw one primitive of which is more than half an
/// arena. A failure after some pieces were recorded leaves those in the
/// frame.
fn draw_sequential_pieces(p: &mut Pvr, d: &GlesDraw) -> i32 {
    let (unit, overlap) = cut_unit(d.prim);
    /* every piece starts at an even vertex: its index address is the list's
     * plus two bytes a vertex, and VDMCTRL_INDEX_LIST1 has no address bits
     * 1:0 (index_base_addrlsb must be 0; an odd start was refused by name on
     * the host's csbgen). A triangle list then steps by six, a line strip or
     * a point list by two. */
    let unit = if unit % 2 == 0 { unit } else { unit * 2 };
    let second = d.index_count2 != 0 && d.index_buf2_gpu != 0;
    let total = d.index_count;
    if unit == 0 || second || total == 0 || total > SEQ_MAX_VERTICES {
        return draw_part(p, d, 0, total, None);
    }
    let budget = p.b.gen.mem.size as u64 / 2;
    /* count the pieces first: past MAX_PIECES the draw is refused before
     * anything is recorded (the bound draw_chunked keeps) */
    let mut pieces = 0u32;
    let mut start = 0u32;
    loop {
        let Some(n) = next_seq_piece(d, total, start, unit, overlap, budget) else {
            return draw_part(p, d, 0, total, None); /* refused by name */
        };
        pieces += 1;
        if pieces > MAX_PIECES {
            if !p.b.warned_big {
                glog!(
                    "pvr_backend: a non-indexed draw of {} vertices needs more than {} pieces to fit its client arrays: not drawn",
                    total,
                    MAX_PIECES
                );
                p.b.warned_big = true;
            }
            return -E2BIG;
        }
        if start + n >= total {
            break;
        }
        start += n - overlap;
    }
    /* every piece addresses the one sequential list, made long enough now */
    let rc = seq_indices_ensure(&mut p.b, total);
    if rc != 0 {
        return rc;
    }
    /* as draw_chunked: a piece that opens a new frame loads the colour */
    let mut cont = *d;
    cont.do_clear = false;
    cont.do_clear_depth = false;
    cont.do_clear_stencil = false;
    let mut start = 0u32;
    loop {
        let Some(n) = next_seq_piece(d, total, start, unit, overlap, budget) else {
            unreachable!("counted above");
        };
        let mut pd = if start == 0 { *d } else { cont };
        /* (read each time: a frame a piece opens may record a load quad) */
        pd.index_buf_gpu = p.b.seq_idx.gpu + start as u64 * 2;
        pd.index_count = n;
        let rc = draw_part(p, &pd, 0, n, Some((start, n)));
        if rc != 0 {
            return rc;
        }
        if start + n >= total {
            return 0;
        }
        start += n - overlap;
    }
}

/// The internal "frame full" code leaves the backend as -ENOMEM.
fn public_rc(rc: i32) -> i32 {
    if rc == -EFRAME_FULL {
        -ENOMEM
    } else {
        rc
    }
}

/// Record vertices `first..first + count` of `d` (the whole draw: 0 and
/// `d.index_count`; `win` as [`record_draw`]'s) into the open frame. Split the frame (the next kick
/// loads this one's colour back) when the draw table or an arena could not
/// take another draw -- a draw needs at most PDS_PER_DRAW_MAX /
/// GEN_PER_DRAW_MAX bytes plus its staged data, and VDM_PER_DRAW_MAX stream
/// words -- and once more when the draw still did not fit (large staged
/// arrays or indices, full depth-bias / scissor tables).
fn draw_part(p: &mut Pvr, d: &GlesDraw, first: u32, count: u32, win: Option<(u32, u32)>) -> i32 {
    if p.b.ndraws > 0
        && (p.b.ndraws >= MAX_DRAWS_PER_FRAME
            || p.b.pds.bump.saturating_add(PDS_PER_DRAW_MAX) as usize > p.b.pds.mem.size
            || p.b.gen.bump.saturating_add(GEN_PER_DRAW_MAX) as usize > p.b.gen.mem.size
            || p.b.vdm_dw.saturating_add(VDM_PER_DRAW_MAX).saturating_mul(4) > VDM_STREAM_BYTES)
    {
        let rc = flush(p);
        if rc != 0 {
            return rc;
        }
        frame_begin(p, d);
    }
    let mut rc = record_draw_txn(&mut p.b, &mut p.scratch, d, first, count, win);
    if rc == -EFRAME_FULL && p.b.ndraws > p.b.frame_quads {
        /* the frame's earlier draws took the room: a fresh frame has it */
        rc = flush(p);
        if rc != 0 {
            return rc;
        }
        frame_begin(p, d);
        rc = record_draw_txn(&mut p.b, &mut p.scratch, d, first, count, win);
    }
    if rc == -EFRAME_FULL && !p.b.warned_big {
        // SAFETY: single-threaded by contract; no other reference is live.
        let cause = unsafe { *FULL_CAUSE.get() };
        let size = match cause {
            FullCause::GenArena => p.b.gen.mem.size,
            FullCause::PdsArena => p.b.pds.mem.size,
            FullCause::VdmStream => p.b.vdm.size,
            _ => 0,
        };
        glog!(
            "pvr_backend: a draw ({} vertices/indices) does not fit an empty frame: {} ({} bytes) is too small; not drawn",
            count,
            cause.name(),
            size
        );
        p.b.warned_big = true;
    }
    rc
}

/// A sequential draw of more vertices than one u16 index list addresses: cut
/// into pieces of at most SEQ_MAX_VERTICES vertices, each an ordinary draw
/// whose per-vertex streams start at the piece's first vertex. Strips repeat
/// the vertices the next piece continues from (an even number of triangles
/// per piece keeps the winding). Fans and loops, whose primitives reach back
/// to the first vertex, and shaders that read gl_VertexID (the index restarts
/// per piece) cannot be cut: -E2BIG. A failure after some pieces were
/// recorded leaves those in the frame.
fn draw_chunked(p: &mut Pvr, d: &GlesDraw) -> i32 {
    // SAFETY: checked non-null by the caller; the core's linked program.
    let vs = unsafe { &*d.vs };
    /* (vertices per piece are a multiple of `unit`; `overlap` vertices are
     * drawn again by the next piece) */
    let (unit, overlap) = cut_unit(d.prim);
    /* a second index list would be emitted once per piece */
    let second = d.index_count2 != 0 && d.index_buf2_gpu != 0;
    if unit == 0 || vs.vs_vertex_id_reg1 != 0 || second {
        if !p.b.warned_big {
            glog!(
                "pvr_backend: a non-indexed draw of {} vertices (mode {}{}) cannot be split into 16-bit index lists: not drawn",
                d.index_count,
                d.prim,
                if vs.vs_vertex_id_reg1 != 0 { ", gl_VertexID" } else { "" }
            );
            p.b.warned_big = true;
        }
        return -E2BIG;
    }
    /* Staged (client) arrays are copied per piece: keep a piece's copies
     * within half an empty arena so that every piece fits a fresh frame. */
    let mut len = SEQ_MAX_VERTICES - SEQ_MAX_VERTICES % unit;
    {
        let staged: u64 = d.attribs[..d.num_attribs as usize]
            .iter()
            .filter(|a| !a.cpu.is_null() && a.cpu_bytes != 0 && a.per_instance == 0)
            .map(|a| a.stride as u64)
            .sum();
        if let Some(fit) = (p.b.gen.mem.size as u64 / 2).checked_div(staged) {
            let fit = fit as u32 - fit as u32 % unit;
            if fit < len {
                len = fit;
            }
            if len <= overlap {
                return frame_full(FullCause::GenArena);
            }
        }
    }
    let total = d.index_count;
    /* A bound on the work one draw can cost: MAX_PIECES pieces are several
     * million vertices, more than any real frame, while an extreme count
     * (glDrawArrays(.., INT_MAX) over a 256-byte buffer, the harness's
     * hostile.draw_extremes) would otherwise record tens of thousands of
     * draws and submit hundreds of kicks before returning. */
    if (total as u64).div_ceil((len - overlap) as u64) > MAX_PIECES as u64 {
        if !p.b.warned_big {
            glog!("pvr_backend: a non-indexed draw of {} vertices is past the backend's limit: not drawn", total);
            p.b.warned_big = true;
        }
        return -E2BIG;
    }
    /* A piece that has to open a new frame must not clear it again: the
     * earlier pieces are in the kick just submitted, so the new frame loads
     * the colour back (and the depth/stencil the ISP stored) instead. */
    let mut cont = *d;
    cont.do_clear = false;
    cont.do_clear_depth = false;
    cont.do_clear_stencil = false;
    let mut first = 0u32;
    loop {
        let n = len.min(total - first);
        let rc = draw_part(p, if first == 0 { d } else { &cont }, first, n, None);
        if rc != 0 {
            return rc;
        }
        if n >= total - first {
            return 0;
        }
        first += n - overlap;
    }
}

extern "C" {
    /// The C library's snprintf, which the C backend used for the `%g` of the
    /// PVR_BACKEND_OPT_DEBUG attribute dump (core::fmt has no `%g`).
    fn snprintf(buf: *mut c_char, size: usize, fmt: *const c_char, ...) -> c_int;
}

/// PVR_BACKEND_OPT_DEBUG: the first vertices of a draw-time converted attribute.
fn dump_attrib(a: &GlesAttrib) {
    let f = a.cpu as *const f32;
    /* (components 0 is rejected before the dump; a zero divisor still
     * dumps nothing instead of faulting) */
    let Some(nv) = a.cpu_bytes.checked_div(a.components as u32 * 4) else {
        return;
    };

    if f.is_null() {
        return;
    }
    let mut v = 0u32;
    while v < nv && v < 24 {
        let mut line = [0 as c_char; 96];
        // SAFETY: snprintf writes at most `size` bytes into `line`; the varargs
        // match the format (two unsigned ints).
        let mut n =
            unsafe { snprintf(line.as_mut_ptr(), line.len(), c"  attrib vi%u v%u:".as_ptr(), a.vi_reg as u32, v) };
        for c in 0..a.components as u32 {
            if n < 0 || n as usize >= line.len() {
                break;
            }
            // SAFETY: the staged attribute holds cpu_bytes bytes, i.e. nv whole
            // vertices of `components` floats, and v < nv; the write stays
            // inside `line` (n < len); `%g` takes the double passed.
            n += unsafe {
                let x = f.add((v * a.components as u32 + c) as usize).read_unaligned();
                snprintf(line.as_mut_ptr().add(n as usize), line.len() - n as usize, c" %g".as_ptr(), x as f64)
            };
        }
        // SAFETY: snprintf always NUL-terminates inside `line`.
        osal::log_str(unsafe { core::ffi::CStr::from_ptr(line.as_ptr()) });
        v += 1;
    }
}

/// A texture binding in one line, for the per-draw log: everything the
/// sampler state is built from, since a texture that samples flat
/// (customeffect's repeating checkerboard) is either the wrong memory, the
/// wrong wrap, or a mip chain the LOD picks the top of.
fn log_tex_desc(tag: &str, i: usize, t: &GlesTextureBind) {
    glog!(
        "pvr_backend:    {}{} unit {}: 0x{:x} {}x{} stride {} f{}, {} level(s), mag {}, min {}, mip {}, wrap {}/{}",
        tag,
        i,
        t.unit,
        t.base_gpu,
        t.width,
        t.height,
        t.stride_texels,
        t.format,
        t.mip_levels,
        if t.linear_filter != 0 { "linear" } else { "nearest" },
        if t.min_linear != 0 { "linear" } else { "nearest" },
        t.mip_filter,
        wrap_name(t.wrap_s),
        wrap_name(t.wrap_t)
    );
}

/// Samplers a stage declares. They need a descriptor each, bound or not: an
/// unbound one then reads the 1x1 border texture rather than whatever sh0
/// holds.
fn stage_sampler_count(sh: Option<&ShcShader>) -> u32 {
    sh.map_or(0, |sh| sh.uniform_table().iter().filter(|u| u.dword_off == SHC_UNIFORM_NO_BLOCK).count() as u32)
}

/// One stage's texture descriptors, in binding order, in the per-frame arena.
/// `*dw` is 0 when the stage has no samplers and nothing was written.
///
/// One `pvr_combined_image_sampler_descriptor` per sampler the stage uses,
/// packed contiguously (sh0, sh20, ...) in binding order -- the exact layout
/// the compiler assigns, which puts the shader's uniform block after them, so
/// a short block would have the shader read image state out of its own
/// uniforms. The set is the union of the stage's declared samplers and the
/// textures the draw named (the internal framebuffer-load draw sets its
/// texture directly, with no uniform reflection). Each slot gets the named
/// texture for its binding, or a 1x1 black texture (the zeroed border page)
/// so a stray sample never faults. Descriptor words: image 0..3, image meta
/// 4..7, sampler 8..11, sampler meta 12..15, gather sampler 16..19 (unused).
fn build_desc_block(
    b: &mut Backend,
    sh: Option<&ShcShader>,
    texs: &[GlesTextureBind],
    gpu: &mut u64,
    dw: &mut u32,
) -> i32 {
    const DESC: usize = TEX_DESC_DWORDS as usize;
    let mut sbind = [0u8; PVR_BACKEND_MAX_SAMPLERS];
    let mut ns = 0usize;

    *dw = 0;
    if let Some(sh) = sh {
        for u in sh.uniform_table() {
            if ns >= PVR_BACKEND_MAX_SAMPLERS {
                break;
            }
            if u.dword_off == SHC_UNIFORM_NO_BLOCK {
                sbind[ns] = u.binding;
                ns += 1;
            }
        }
    }
    for t in texs {
        if ns >= PVR_BACKEND_MAX_SAMPLERS {
            break;
        }
        if !sbind[..ns].contains(&t.unit) {
            sbind[ns] = t.unit;
            ns += 1;
        }
    }
    if ns == 0 {
        return 0;
    }
    for i in 1..ns {
        /* insertion sort by binding */
        let v = sbind[i];
        let mut j = i;

        while j > 0 && sbind[j - 1] > v {
            sbind[j] = sbind[j - 1];
            j -= 1;
        }
        sbind[j] = v;
    }
    let mut desc = [0u32; PVR_BACKEND_MAX_SAMPLERS * DESC];

    for (i, &unit) in sbind[..ns].iter().enumerate() {
        let fallback = GlesTextureBind {
            unit,
            base_gpu: b.black_tex.gpu,
            width: 1,
            height: 1,
            stride_texels: 1,
            linear_filter: 0,
            wrap_s: GLES_WRAP_CLAMP_TO_EDGE,
            wrap_t: GLES_WRAP_CLAMP_TO_EDGE,
            ..GlesTextureBind::ZERO
        };
        /* (the last texture naming the binding wins) Nothing bound, or a
         * texture GL calls incomplete: both sample (0,0,0,1) (GL ES 2.0
         * section 3.7.10), which is what the fallback texel holds. */
        let t = texs.iter().rev().find(|t| t.unit == unit && t.incomplete == 0);
        /* a depth texture whose rendered depth the GL core left in this
         * backend's buffer (depth_id, pvr_depth_sampled) reads the buffer;
         * a binding whose buffer is not there samples black (the texels are
         * stale by agreement) */
        let from_buffer = t.filter(|t| t.depth_id != 0).and_then(|t| depth_buffer_binding(b, t));
        let bound = match (&from_buffer, t) {
            (Some(d), _) => d,
            (None, Some(t)) if t.depth_id == 0 => t,
            _ => &fallback,
        };

        build_tex_desc(&mut desc[i * DESC..(i + 1) * DESC], bound, &mut b.warned_tex_addr);
        /* the sample store ([`MsStore`]): the target's multisampled image */
        if t.is_some_and(|t| !b.ms.mem.impl_.is_null() && t.base_gpu == b.ms.mem.gpu) {
            tex_desc_set_samples(&mut desc[i * DESC..(i + 1) * DESC], b.ms.samples);
        }
    }
    let rc = arena_put_words(&mut b.gen, &desc[..ns * DESC], 32, None, Some(gpu));
    if rc != 0 {
        return rc;
    }
    *dw = ns as u32 * TEX_DESC_DWORDS;
    0
}

/// [`record_draw`] as a transaction: a draw that fails leaves the frame as it
/// was (arena bumps, depth-bias / scissor tables, the terminate block), so a
/// failed draw neither wastes the frame's room nor changes what later draws
/// record, and it can be retried in a fresh frame.
fn record_draw_txn(
    b: &mut Backend,
    s: &mut Scratch,
    d: &GlesDraw,
    first: u32,
    count: u32,
    win: Option<(u32, u32)>,
) -> i32 {
    let saved = (b.pds.bump, b.gen.bump, b.ndbias, b.dbias_last, b.nscissor, b.scissor_last, b.term_gpu, b.vdm_dw);
    let rc = record_draw(b, s, d, first, count, win);

    if rc != 0 {
        (b.pds.bump, b.gen.bump, b.ndbias, b.dbias_last, b.nscissor, b.scissor_last, b.term_gpu, b.vdm_dw) = saved;
        pds_once_forget(b, b.pds.bump);
    }
    rc
}

/// The pixels a draw can touch, top-down in the target: the viewport, cut to
/// the scissor box when the test is on, clamped to the target. `None` when
/// nothing is left.
///
/// An empty box has to stop the draw rather than reach the PPP: a box wholly
/// outside the target used to come out inverted (top below bottom after the
/// y flip, or right of left in x) and went to TA_REGION_CLIP as such, with no
/// ISP scissor entry. The firmware then never started the kick's geometry
/// phase (the geometry CCB unread, fence 1 never signalled): `glScissor(0,
/// -200, 100, 50); glClear(GL_COLOR_BUFFER_BIT)` on a 256x256 target wedged
/// the GPU every time, and dEQP-GLES2 color_clear.complex_rgb, whose random
/// boxes include such rectangles, wedged on its first kick (the "shape 2"
/// stall). An x-only miss did not wedge but cleared tile column 0.
fn draw_box(d: &GlesDraw) -> Option<[i32; 4]> {
    let (mut x0, mut y0) = (d.viewport[0], d.viewport[1]);
    let (mut x1, mut y1) = (x0.saturating_add(d.viewport[2]), y0.saturating_add(d.viewport[3]));

    if d.scissor {
        let (sx0, sy0) = (d.scissor_rect[0], d.scissor_rect[1]);
        let (sx1, sy1) = (sx0.saturating_add(d.scissor_rect[2]), sy0.saturating_add(d.scissor_rect[3]));

        x0 = x0.max(sx0);
        y0 = y0.max(sy0);
        x1 = x1.min(sx1);
        y1 = y1.min(sy1);
    }
    let h = d.target_h as i32;
    let top = if d.flip_y { h.saturating_sub(y1) } else { y0 }.max(0);
    let bot = if d.flip_y { h.saturating_sub(y0) } else { y1 }.min(h);
    let (x0, x1) = (x0.max(0), x1.min(d.target_w as i32));

    (x1 > x0 && bot > top).then_some([x0, top, x1, bot])
}

/// Record one draw into the open frame: PDS programs, descriptors, PPP state
/// block and VDM blocks. `first` / `count`: the vertices of a sequential draw
/// to record (a piece of a larger one, see [`draw_chunked`]), otherwise 0 and
/// `d.index_count`. `win`: a piece of a split indexed draw (see
/// [`draw_indexed_pieces`]) reads vertices `win.0 .. win.0 + win.1` only, so
/// each staged per-vertex attribute is copied for that window alone and its
/// stream addressed as if the copy began at vertex 0 (the indices stay
/// absolute: gl_VertexID and the VBO-resident attributes are unchanged).
/// Returns 0, -[`EFRAME_FULL`] (the frame has no room),
/// -EINVAL (a draw the backend cannot encode), -ENOMEM (no memory for the
/// sequential indices) or -EIO (an arena without a CPU mapping).
fn record_draw(b: &mut Backend, s: &mut Scratch, d: &GlesDraw, first: u32, count: u32, win: Option<(u32, u32)>) -> i32 {
    // SAFETY: d.vs / d.fs are null (rejected here) or point at shader
    // descriptors that outlive the draw: the core's linked program, or the
    // backend's own FbLoad descriptors, which nothing mutates while recording.
    let (Some(vs), Some(fs)) = (unsafe { d.vs.as_ref() }, unsafe { d.fs.as_ref() }) else {
        return -EINVAL;
    };
    /* A draw that can touch no pixel is recorded as nothing (see draw_box):
     * before anything is allocated, and not counted in ndraws, so a frame of
     * such draws alone still gets flush()'s covering object. */
    if draw_box(d).is_none() {
        b.draws_empty_box = b.draws_empty_box.wrapping_add(1);
        if b.draws_empty_box == 1 {
            glog!("pvr_backend: a draw outside the target (viewport/scissor box empty) is recorded as nothing");
        }
        return 0;
    }
    let mut rc: i32;

    /* ---- topology ---- */
    let (topology, objtype) = match d.prim {
        GLES_PRIM_TRI_STRIP => (PVR_VDM_TOPOLOGY_TRI_STRIP, PVR_TA_OBJTYPE_TRIANGLE),
        GLES_PRIM_TRI_FAN => (PVR_VDM_TOPOLOGY_TRI_FAN, PVR_TA_OBJTYPE_TRIANGLE),
        GLES_PRIM_LINES => (PVR_VDM_TOPOLOGY_LINE_LIST, PVR_TA_OBJTYPE_LINE),
        GLES_PRIM_LINE_STRIP => (PVR_VDM_TOPOLOGY_LINE_STRIP, PVR_TA_OBJTYPE_LINE),
        GLES_PRIM_LINE_LOOP => (PVR_VDM_TOPOLOGY_LINE_LOOP, PVR_TA_OBJTYPE_LINE),
        GLES_PRIM_POINTS => (PVR_VDM_TOPOLOGY_POINT_LIST, point_objtype(d.flip_y)),
        _ => (PVR_VDM_TOPOLOGY_TRI_LIST, PVR_TA_OBJTYPE_TRIANGLE),
    };

    /* ---- index buffer: the proven path is indexed u16; synthesise 0..N-1 ---- */
    let piece = first != 0 || count != d.index_count;
    let mut index_gpu = d.index_buf_gpu;
    if !d.index_cpu.is_null() && d.index_bytes != 0 {
        /* per-draw copy of the GL core's staged indices */
        // SAFETY: the core stages index_bytes bytes at index_cpu for this draw.
        let irc =
            unsafe { arena_put(&mut b.gen, d.index_cpu as *const u8, d.index_bytes, 16, None, Some(&mut index_gpu)) };
        if irc != 0 {
            return irc;
        }
    }
    if index_gpu == 0 {
        rc = seq_indices_ensure(b, count);
        if rc != 0 {
            return rc;
        }
        index_gpu = b.seq_idx.gpu;
    }

    /* ---- vertex-fetch PDS (one stream per attribute) ---- */
    let mut vinfo = PvrPdsRenderVertexInfo::default();
    let (mut v_pds_data, mut v_pds_code) = (0u32, 0u32);

    /* No attributes at all is a real draw: a vertex shader that builds its
     * geometry from gl_VertexID reads nothing (Qt Quick 3D's post-processing
     * passes draw their full-screen triangle that way). The vertex-fetch
     * program is then just the id registers and the USC launch. */
    if d.num_attribs as u32 > PVR_PDS_VERTEX_MAX_STREAMS {
        glog_once!("pvr_backend: {} attributes not supported", d.num_attribs);
        return -EINVAL;
    }
    let attribs = &d.attribs[..d.num_attribs as usize];
    let mut nstreams = 0usize;
    {
        /* Order the attributes by vi register, then merge runs that are
         * contiguous both in memory (same buffer, same stride, adjacent
         * offsets) and in vi registers into one DMA stream. An interleaved
         * VBO with the attributes in register order becomes a single DMA of
         * the whole vertex (the layout the test rig used). */
        let mut order = [0u8; PVR_PDS_VERTEX_MAX_STREAMS as usize];
        /* GPU address of each attribute's first vertex */
        let mut abase = [0u64; PVR_PDS_VERTEX_MAX_STREAMS as usize];

        for (i, a) in attribs.iter().enumerate() {
            if a.format != 0 || a.components == 0 || a.components > 4 {
                glog_once!("pvr_backend: only float32 vec1..4 attributes are supported");
                return -EINVAL;
            }
            order[i] = i as u8;
            /* a piece of a split sequential draw starts at vertex `first` */
            let skip = if a.per_instance == 0 { first as u64 * a.stride as u64 } else { 0 };
            if !a.cpu.is_null() && a.cpu_bytes != 0 {
                /* draw-time data: the GL core's staging is reused by the
                 * next draw, so this draw gets its own copy in the arena
                 * (a piece copies only the bytes its vertices use) */
                let window = win.filter(|_| a.per_instance == 0 && a.stride != 0);
                let (from, len) = if let Some((vmin, vcount)) = window {
                    /* a piece of a split indexed draw: its vertex window */
                    let from = vmin as u64 * a.stride as u64;
                    let want = (vcount.max(1) - 1) as u64 * a.stride as u64 + a.components as u64 * 4;
                    match (from < a.cpu_bytes as u64)
                        .then(|| (from as u32, (a.cpu_bytes as u64 - from).min(want) as u32))
                    {
                        Some(r) => r,
                        None => return -EINVAL, /* the staging ends before the window */
                    }
                } else if !piece || a.per_instance != 0 || a.stride == 0 {
                    (0, a.cpu_bytes) /* the whole draw, per instance, or a constant */
                } else {
                    let want = count as u64 * a.stride as u64 + a.components as u64 * 4;
                    match (skip < a.cpu_bytes as u64)
                        .then(|| (skip as u32, (a.cpu_bytes as u64 - skip).min(want) as u32))
                    {
                        Some(r) => r,
                        None => return -EINVAL, /* the staging ends before the piece */
                    }
                };
                // SAFETY: the core stages cpu_bytes bytes at a.cpu for this
                // draw, and from + len <= cpu_bytes.
                let arc = unsafe {
                    arena_put(&mut b.gen, (a.cpu as *const u8).add(from as usize), len, 16, None, Some(&mut abase[i]))
                };
                if arc != 0 {
                    return arc;
                }
                if window.is_some() {
                    /* the stream's vertex 0 lies `from` bytes before the
                     * copy; the piece's indices never reach below win.0 */
                    abase[i] = abase[i].wrapping_sub(from as u64);
                }
            } else {
                abase[i] = a.buffer_gpu.wrapping_add(a.offset as u64).wrapping_add(skip);
            }
        }
        for i in 1..attribs.len() {
            /* insertion sort by vi_reg */
            let k = order[i];
            let mut j = i;

            while j > 0 && attribs[order[j - 1] as usize].vi_reg > attribs[k as usize].vi_reg {
                order[j] = order[j - 1];
                j -= 1;
            }
            order[j] = k;
        }
        let coalesce = b.opt(PVR_BACKEND_OPT_NO_COALESCE) == 0;
        for &o in &order[..attribs.len()] {
            nstreams = add_stream(&mut s.streams, nstreams, &attribs[o as usize], abase[o as usize], coalesce);
        }
    }
    /* gl_VertexID / gl_InstanceID (ES 3.0): the vertex fetch writes IR0/IR1
     * into the vtxin registers the compiler reads them from */
    // SAFETY: vdata/vcode have the sizes the generator documents for up to
    // PVR_PDS_VERTEX_MAX_STREAMS streams plus the id DOUTWs; nstreams <=
    // num_attribs <= that.
    if unsafe {
        pvr_pds_render_vertex_program_ids(
            heap_off_of(d.vs_usc_gpu) as u64,
            vs.temps as u32,
            s.streams.as_ptr(),
            nstreams as u32,
            0,
            id_reg_of(vs.vs_vertex_id_reg1),
            id_reg_of(vs.vs_instance_id_reg1),
            s.vdata.as_mut_ptr(),
            s.vcode.as_mut_ptr(),
            &mut vinfo,
        )
    } != 0
    {
        glog_once!(
            "yakogl/pvr: draw refused: the vertex fetch program was refused ({} streams, {} temps)",
            nstreams,
            vs.temps
        );
        return -EINVAL;
    }
    let vdata = &s.vdata[..(vinfo.data_size_dw as usize).min(VDATA_DWORDS)];
    let vcode = &s.vcode[..(vinfo.code_size_dw as usize).min(VCODE_DWORDS)];
    /* Bring-up: one compact line per draw of the first kicks -- which shader
     * it runs and which images it samples. Enough to see what a
     * post-processing pass reads without the full per-draw dump.
     *
     * The budget is larger when the dump was asked for, because a Quick 3D
     * scene with an image-based light spends its first forty kicks turning
     * the .hdr into a cube map and its mip chain, and a budget of 120 draws
     * ran out before the scene itself was drawn. */
    /* (no `draw_log_left != 0` here any more: the default budget is 0, and
     * this is what turns the per-draw line back on) */
    if b.opt(PVR_BACKEND_OPT_DEBUG) != 0 && b.draw_log_left < DRAW_LOG_BUDGET_DEBUG {
        b.draw_log_left = DRAW_LOG_BUDGET_DEBUG;
    }
    if b.draw_log_left != 0 {
        b.draw_log_left -= 1;
        glog!(
            "pvr_backend: k{} d{}: vs {}, fs {}, fs dw {}, vs dw {}, {} tex, fs uniforms {}/{} dw",
            b.last_fence as u32 + 1,
            b.ndraws,
            shaderc::shader_source_name(vs),
            shaderc::shader_source_name(fs),
            fs.usc_dw,
            vs.usc_dw,
            d.num_textures,
            d.fs_uniform_dwords,
            fs.uniform_dwords
        );
        if d.num_vs_textures != 0 {
            glog!("pvr_backend:    {} vertex-stage texture(s)", d.num_vs_textures);
        }
        if !d.vs_uniform_cpu.is_null() && d.vs_uniform_dwords >= 16 {
            // SAFETY: the core's block holds vs_uniform_dwords >= 16 dwords;
            // the application's array may be unaligned, so read it unaligned.
            let f = |i: usize| unsafe { (d.vs_uniform_cpu as *const f32).add(i).read_unaligned() };

            glog!(
                "pvr_backend:    vs block cpu {:?} gpu 0x{:x}, shader prefix {}",
                d.vs_uniform_cpu,
                d.vs_uniform_gpu,
                vs.uniform_sh_dwords
            );
            glog!(
                "pvr_backend:    vs uniforms {} dw, first matrix: {} {} {} {} / {} {} {} {} / {} {} {} {} / {} {} {} {}",
                d.vs_uniform_dwords,
                f(0), f(1), f(2), f(3),
                f(4), f(5), f(6), f(7),
                f(8), f(9), f(10), f(11),
                f(12), f(13), f(14), f(15)
            );
        }
        if !d.fs_uniform_cpu.is_null() && d.fs_uniform_dwords >= 8 {
            /* The head of the fragment block: two draws of the same program
             * that should differ (four custom materials with four different
             * colours) and do not are a snapshot that did not happen. */
            // SAFETY: the block holds fs_uniform_dwords >= 8 dwords; the
            // array may be unaligned, so read it unaligned.
            let f = |i: usize| unsafe { (d.fs_uniform_cpu as *const f32).add(i).read_unaligned() };

            if b.opt(PVR_BACKEND_OPT_DEBUG) != 0 {
                // SAFETY: as above; the block is a dword array of the
                // library's own, so it is aligned.
                let blk = unsafe {
                    core::slice::from_raw_parts(d.fs_uniform_cpu as *const u32, (d.fs_uniform_dwords as usize).min(160))
                };
                dump_words("fs_uni_block", blk);
            }
            glog!(
                "pvr_backend:    fs block gpu 0x{:x}, first 8: {} {} {} {} / {} {} {} {}",
                d.fs_uniform_gpu,
                f(0),
                f(1),
                f(2),
                f(3),
                f(4),
                f(5),
                f(6),
                f(7)
            );
        }
        glog!(
            "pvr_backend:    {} attrib(s), prim {}, {} index/vertices, depth {}{} func {}, blend {}, cull {}/{}, colour_write {}, viewport {},{} {}x{}, scissor {}",
            d.num_attribs,
            d.prim,
            d.index_count,
            if d.depth_test { "test" } else { "off" },
            if d.depth_write { "+write" } else { "" },
            d.depth_func,
            d.blend as u32,
            d.cull,
            d.front_cw as u32,
            d.color_write as u32,
            d.viewport[0],
            d.viewport[1],
            d.viewport[2],
            d.viewport[3],
            d.scissor as u32
        );
        for (i, t) in d.textures[..(d.num_textures as usize).min(d.textures.len())].iter().enumerate() {
            if b.draw_log_left == 0 {
                break;
            }
            b.draw_log_left -= 1;
            log_tex_desc("tex", i, t);
        }
        for (i, t) in d.vs_textures[..(d.num_vs_textures as usize).min(d.vs_textures.len())].iter().enumerate() {
            if b.draw_log_left == 0 {
                break;
            }
            b.draw_log_left -= 1;
            log_tex_desc("vstex", i, t);
        }
    }
    if b.num_targets > 1 && (fs.num_outputs as u32) < b.num_targets as u32 && b.warned_mrt_outputs < 4 {
        b.warned_mrt_outputs += 1;
        glog!(
            "pvr_backend: {} colour attachments but the fragment shader writes {} output(s)",
            b.num_targets,
            fs.num_outputs
        );
    }
    if b.opt(PVR_BACKEND_OPT_DEBUG) != 0 && b.dbg_draws_left != 0 {
        glog!(
            "pvr_backend: draw {}: {} attrib(s) -> {} stream(s), vs usc off 0x{:x} dw {} temps {}, fs usc off 0x{:x} dw {} temps {}, idx {}, uniforms vs {}/{} fs {}/{} dw",
            b.ndraws,
            d.num_attribs,
            nstreams,
            heap_off_of(d.vs_usc_gpu),
            vs.usc_dw,
            vs.temps,
            heap_off_of(d.fs_usc_gpu),
            fs.usc_dw,
            fs.temps,
            d.index_count,
            d.vs_uniform_dwords,
            vs.uniform_dwords,
            d.fs_uniform_dwords,
            fs.uniform_dwords
        );
        glog!("  idx gpu 0x{:x}, vs usc gpu 0x{:x}, fs usc gpu 0x{:x}", index_gpu, d.vs_usc_gpu, d.fs_usc_gpu);
        for (i, st) in s.streams[..nstreams].iter().enumerate() {
            glog!(
                "  stream[{}]: addr 0x{:x} stride {} size {} -> vi{}",
                i,
                st.address,
                st.stride,
                st.dma_size,
                st.dst_reg
            );
        }
        for a in attribs {
            dump_attrib(a);
        }
        dump_words("vpds_data", vdata);
        dump_words("vpds_code", vcode);
        if d.index_buf_gpu == 0 && !b.seq_idx.cpu.is_null() {
            // SAFETY: seq_idx is a CPU-mapped block of at least 1024 u16s
            // (seq_indices_ensure).
            let ix = unsafe { core::slice::from_raw_parts(b.seq_idx.cpu as *const u16, 6) };

            glog!("  seq_idx @0x{:x}: {} {} {} {} {} {}", b.seq_idx.gpu, ix[0], ix[1], ix[2], ix[3], ix[4], ix[5]);
        }
    }
    rc = pds_put(b, vdata, &mut v_pds_data);
    if rc == 0 {
        rc = pds_put(b, vcode, &mut v_pds_code);
    }
    if rc != 0 {
        return rc;
    }

    /* The kick-USC and coefficient programs depend on the fragment program
     * and the sample rate, not on the draw: a frame generates each once and
     * its later draws point at it (docs/notes/pvr-draw-pds.md). The bring-up
     * dump generates every draw's, to print its words. */
    let once = !(PDS_ONCE_OFF.load(core::sync::atomic::Ordering::Relaxed)
        || (b.opt(PVR_BACKEND_OPT_DEBUG) != 0 && b.dbg_draws_left != 0));

    /* ---- fragment kick-USC PDS: [data(8) | code(1)] contiguous ---- */
    let mut k_prog = [0u32; 9];
    let mut k_pds_data = 0u32;
    let usc_off = heap_off_of(d.fs_usc_gpu);
    let temps = (fs.temps as u32).max(b.opt(PVR_BACKEND_OPT_FS_TEMPS_MIN).min(FS_TEMPS_MIN_CAP));
    let rate =
        fs_sample_rate(b.samples, d.blend || fs.uses_fbfetch != 0 || fs.uses_discard != 0 || loads_samples(b, d));
    let kick_key = KickOnce { usc_off, temps, rate, ..KickOnce::ZERO };
    let kick_hit = if once {
        b.kick_once.iter().find(|k| k.end != 0 && (k.usc_off, k.temps, k.rate) == (usc_off, temps, rate)).map(|k| k.off)
    } else {
        None
    };
    if let Some(off) = kick_hit {
        k_pds_data = off;
    } else {
        let mut ki = PvrPdsRenderKickUscInfo::default();

        // SAFETY: k_prog holds the 8 data dwords followed by the 1 code dword.
        if unsafe {
            pvr_pds_render_kick_usc_program(
                usc_off as u64,
                temps,
                rate,
                k_prog.as_mut_ptr(),
                k_prog.as_mut_ptr().add(8),
                &mut ki,
            )
        } != 0
        {
            glog_once!(
                "yakogl/pvr: draw refused: the fragment kick program was refused ({} temps, max {})",
                temps,
                PVR_PDS_DOUTU_MAX_TEMPS
            );
            return -EINVAL;
        }
        let n = ((ki.data_size_dw + ki.code_size_dw) as usize).min(k_prog.len());
        rc = pds_put(b, &k_prog[..n], &mut k_pds_data);
        if rc != 0 {
            return rc;
        }
        if once && n != 0 {
            let slot = b.kick_once_next as usize % PDS_ONCE_SLOTS;
            b.kick_once[slot] = KickOnce { off: k_pds_data, end: pds_end_of(b, k_pds_data, n), ..kick_key };
            b.kick_once_next = ((slot + 1) % PDS_ONCE_SLOTS) as u8;
        }
        if b.opt(PVR_BACKEND_OPT_DEBUG) != 0 && b.dbg_draws_left != 0 {
            dump_words("kick_prog", &k_prog);
        }
    }

    /* ---- coefficient (DOUTI) PDS for the FS varyings: [data | code] ---- */
    let mut coeff = PvrPdsCoeffInfo::default();
    let mut coeff_off = 0u32;
    {
        let nv = fs.num_varyings as u32;

        if nv > PVR_PDS_COEFF_MAX_VARYINGS {
            return -EINVAL;
        }
        for (cv, var) in s.cv.iter_mut().zip(&fs.varyings[..nv as usize]) {
            cv.components = var.components as u32;
            cv.interp = match var.interp {
                2 => PVR_PDS_COEFF_FLAT,
                1 => PVR_PDS_COEFF_NPC,
                _ => PVR_PDS_COEFF_SMOOTH,
            };
            /* Perspective-correct (smooth) iteration divides by the RHW
             * plane, which only exists when the FS asked for it (uses_w:
             * pco always sets it for smooth inputs). A shader without it
             * (the hand-written texture FS) can only be iterated
             * non-perspective; a perspective iterator without the W plane
             * produced v = min(u, v) on silicon. */
            if fs.uses_w == 0 && cv.interp == PVR_PDS_COEFF_SMOOTH {
                cv.interp = PVR_PDS_COEFF_NPC;
            }
            cv.vtxout_off = var.vtxout_off as u32;
            /* These offsets have to be the VERTEX shader's, which is what the
             * link step patches them to. A path that skips linking (a
             * fragment-shader variant used to) leaves the fragment shader's
             * own packing here, and the draw then iterates the wrong vertex
             * outputs without a word. */
            if !b.warned_varying {
                let fname = util::name_bytes(&var.name);
                let vv = vs.varyings[..vs.num_varyings as usize]
                    .iter()
                    .find(|vv| !fname.is_empty() && util::name_bytes(&vv.name) == fname);

                if let Some(vv) = vv.filter(|vv| vv.vtxout_off != var.vtxout_off) {
                    b.warned_varying = true;
                    glog!(
                        "pvr_backend: varying '{}' iterated from vtxout {}, but the vertex shader writes it at {} (unlinked shader)",
                        Bytes(fname),
                        var.vtxout_off,
                        vv.vtxout_off
                    );
                }
            }
        }
        let flags = fs.uses_w as u32 | ((fs.uses_z != 0) as u32) << 16 | ((fs.uses_pntc != 0) as u32) << 17;
        let cv = &s.cv[..nv as usize];
        let hit = if once { b.coeff_once.iter().find(|c| c.holds(flags, cv)) } else { None };
        if let Some(c) = hit {
            (coeff, coeff_off) = (c.info, c.off);
        } else {
            // SAFETY: cdata/ccode have the sizes the generator documents for
            // up to PVR_PDS_COEFF_MAX_VARYINGS varyings; nv was checked
            // against that.
            if unsafe {
                pvr_pds_render_coeff_program(
                    fs.uses_w as i32,
                    /* gl_FragCoord.z: pco reads the Z plane from set 0, ahead
                     * of W (Mesa pvr_arch_pipeline.c:1503-1518); the TE only
                     * hands it over when OUTPUT_SEL says so (z_pres below). */
                    (fs.uses_z != 0) as i32,
                    /* gl_PointCoord: pco reads it from the 8 coefficients
                     * after W, iterated by a point-sprite DOUTI (Mesa
                     * pvr_arch_pipeline.c:1530-1540), and puts the varyings
                     * after it. Without the iterator the shader read
                     * coefficients nobody wrote (case 83: s = t = 1.0 across
                     * the sprite). */
                    (fs.uses_pntc != 0) as i32,
                    s.cv.as_ptr(),
                    nv,
                    s.cdata.as_mut_ptr(),
                    s.ccode.as_mut_ptr(),
                    &mut coeff,
                )
            } != 0
            {
                glog_once!("yakogl/pvr: draw refused: the varying coefficient program was refused ({} varyings)", nv);
                return -EINVAL;
            }
            let mut end = 0u32;
            if coeff.code_size_dw != 0 {
                let nd = (coeff.data_size_dw as usize).min(CDATA_DWORDS);
                let nc = (coeff.code_size_dw as usize).min(CCODE_DWORDS);

                s.cprog[..nd].copy_from_slice(&s.cdata[..nd]);
                s.cprog[nd..nd + nc].copy_from_slice(&s.ccode[..nc]);
                rc = pds_put(b, &s.cprog[..nd + nc], &mut coeff_off);
                if rc != 0 {
                    return rc;
                }
                end = pds_end_of(b, coeff_off, nd + nc);
                if b.opt(PVR_BACKEND_OPT_DEBUG) != 0 && b.dbg_draws_left != 0 {
                    dump_words("coeff_prog", &s.cprog[..nd + nc]);
                }
            }
            if once {
                let slot = b.coeff_once_next as usize % PDS_ONCE_SLOTS;
                let mut c = CoeffOnce { used: true, flags, nv, info: coeff, off: coeff_off, end, ..CoeffOnce::ZERO };
                c.cv[..cv.len()].copy_from_slice(cv);
                b.coeff_once[slot] = c;
                b.coeff_once_next = ((slot + 1) % PDS_ONCE_SLOTS) as u8;
            }
        }
    }

    /* ---- common-store DMA PDS: texture+sampler state and/or FS uniforms ---- */
    let mut nkicks = 0usize;
    let mut shared_dw = 0u32;
    let (mut t_pds_data, mut t_pds_code, mut t_data_bytes, mut t_temps) = (0u32, 0u32, 0u32, 0u32);
    let textures = &d.textures[..(d.num_textures as usize).min(d.textures.len())];

    {
        let mut desc_gpu = 0u64;
        let mut desc_dw = 0u32;

        rc = build_desc_block(b, Some(fs), textures, &mut desc_gpu, &mut desc_dw);
        if rc != 0 {
            return rc;
        }
        if desc_dw != 0 {
            if !dma_push(
                &mut s.kicks,
                &mut nkicks,
                PvrPdsDmaKick { address: desc_gpu, dwords: desc_dw, dest_offset: shared_dw },
            ) {
                return dma_kicks_full(b);
            }
            shared_dw += desc_dw;
        }
    }
    if d.fs_uniform_dwords != 0 {
        /* Snapshot per draw (see the VS uniform note below): pg->uniforms
         * is shared per program, so several draws sharing a program in one
         * kick would otherwise all read the last draw's block.
         * With PVR_BACKEND_UNIFORM_PREFIX only the prefix the shader reads
         * goes to the shared registers (uniform_sh_dwords). */
        let mut fs_uni_gpu = d.fs_uniform_gpu;
        let mut uni_dw = d.fs_uniform_dwords as u32;

        if PVR_BACKEND_UNIFORM_PREFIX && fs.uniform_sh_dwords != 0 && (fs.uniform_sh_dwords as u32) < uni_dw {
            uni_dw = fs.uniform_sh_dwords as u32;
        }
        if !d.fs_uniform_cpu.is_null()
            // SAFETY: the core's uniform block holds fs_uniform_dwords dwords,
            // and uni_dw is at most that.
            && unsafe {
                arena_put(&mut b.gen, d.fs_uniform_cpu as *const u8, uni_dw * 4, 16, None, Some(&mut fs_uni_gpu))
            } != 0
        {
            return -ENOMEM;
        }
        /* sh0.. (sh20.. after a texture) */
        if !dma_push(
            &mut s.kicks,
            &mut nkicks,
            PvrPdsDmaKick { address: fs_uni_gpu, dwords: uni_dw, dest_offset: shared_dw },
        ) {
            return dma_kicks_full(b);
        }
        shared_dw += uni_dw;
        if b.opt(PVR_BACKEND_OPT_DEBUG) != 0 && b.dbg_draws_left != 0 && !d.fs_uniform_cpu.is_null() {
            /* The block as the shader will read it. A loop whose bound is a
             * uniform (Quick 3D's environment prefilter runs
             * "i < ubuf2.sampleCount") turns a wrong dword into a kick that
             * never ends, so the numbers matter, not just the size. */
            for i in 0..uni_dw.min(8) {
                // SAFETY: the core's block holds fs_uniform_dwords >= uni_dw
                // dwords; the application's array may be unaligned.
                let (u, f) = unsafe {
                    let p = (d.fs_uniform_cpu as *const u32).add(i as usize);

                    (p.read_unaligned(), (p as *const f32).read_unaligned())
                };

                glog!("  fs_uniform[{}] = 0x{:08x}  int {}  float {}", i, u, u as i32, f);
            }
        }
    }
    /* A fragment shader that uses discard is compiled in FULL mode: the USC
     * reads a per-draw ISP-feedback control word from a shared register the
     * compiler reserved at sh[meta_off]. Without it the USC reads garbage
     * there, colour writes get disabled and every fragment is dropped. Supply
     * the benign default (colour writes on, all sample-mask bits, no a2c/a2one).
     * meta_off is the absolute shared-register index pco_shc reported (already
     * past the descriptors + uniform prefix), so it is the dest_offset verbatim
     * -- do not add shared_dw. */
    if fs.meta_off != SHC_NO_META {
        let mut meta_gpu = 0u64;

        rc = arena_put_words(&mut b.gen, &[fs_meta_word(b.samples, d)], 4, None, Some(&mut meta_gpu));
        if rc != 0 {
            return rc;
        }
        if !dma_push(
            &mut s.kicks,
            &mut nkicks,
            PvrPdsDmaKick { address: meta_gpu, dwords: 1, dest_offset: fs.meta_off as u32 },
        ) {
            return dma_kicks_full(b);
        }
        if fs.meta_off as u32 + 1 > shared_dw {
            shared_dw = fs.meta_off as u32 + 1;
        }
    }
    /* gl_FrontFacing: the USC combines the ISP's facing bit with a front-face
     * op from a shared register (Mesa PVR_BUFFER_TYPE_FRONT_FACE_OP). The ISP
     * calls its clockwise triangles front; GL's front face is clockwise for
     * the TA exactly as in the two-sided stencil mapping below. Points and
     * lines are always front. */
    if fs.fs_front_face_off1 != 0 {
        let ta_swapped = !d.flip_y && b.opt(PVR_BACKEND_OPT_PRETRANSFORM) == 0;
        let front_cw_in_ta = d.front_cw != ta_swapped;
        let ff_op: u32 = if objtype != PVR_TA_OBJTYPE_TRIANGLE {
            2
        } else if front_cw_in_ta {
            0
        } else {
            1
        };
        let ff_off = fs.fs_front_face_off1 as u32 - 1;
        let mut ff_gpu = 0u64;

        rc = arena_put_words(&mut b.gen, &[ff_op], 4, None, Some(&mut ff_gpu));
        if rc != 0 {
            return rc;
        }
        if !dma_push(&mut s.kicks, &mut nkicks, PvrPdsDmaKick { address: ff_gpu, dwords: 1, dest_offset: ff_off }) {
            return dma_kicks_full(b);
        }
        if ff_off + 1 > shared_dw {
            shared_dw = ff_off + 1;
        }
    }
    /* gl_FragCoord.y: the online compiler's program computes
     * y' = y * scale + offset from the pair the record names
     * (`fragcoord_ytf_off1`), so GL's y counts up from the bottom on every
     * target. The special registers count down from the top of the stored
     * image (gl_test case 82, both targets), so a target stored top-down
     * (`flip_y`: today the window surface) gets (-1, height) and every
     * other target (1, 0). pco's table programs read y unchanged and carry
     * no pair. */
    if fs.fragcoord_ytf_off1 != 0 {
        let [scale, offset] = fragcoord_y_pair(d);
        let ytf_off = fs.fragcoord_ytf_off1 as u32 - 1;
        let mut ytf_gpu = 0u64;

        rc = arena_put_words(&mut b.gen, &[scale.to_bits(), offset.to_bits()], 4, None, Some(&mut ytf_gpu));
        if rc != 0 {
            return rc;
        }
        if !dma_push(&mut s.kicks, &mut nkicks, PvrPdsDmaKick { address: ytf_gpu, dwords: 2, dest_offset: ytf_off }) {
            return dma_kicks_full(b);
        }
        shared_dw = shared_dw.max(ytf_off + 2);
    }
    /* Tile buffers: a fragment shader with more outputs than the pixel's four
     * registers hold stores the rest to memory, addressed from the shared
     * registers the compiler placed after everything else it reads. */
    if fs.num_tile_buffers != 0 {
        let n = (fs.num_tile_buffers as usize).min(PVR_RENDER_TILE_BUFFERS);
        let mut addr = [0u32; PVR_RENDER_TILE_BUFFERS * 2];
        let mut addr_gpu = 0u64;

        for i in 0..n {
            addr[i * 2] = b.st.tile_buffer_gpu[i] as u32;
            addr[i * 2 + 1] = (b.st.tile_buffer_gpu[i] >> 32) as u32;
        }
        rc = arena_put_words(&mut b.gen, &addr[..n * 2], 8, None, Some(&mut addr_gpu));
        if rc != 0 {
            return rc;
        }
        let k = PvrPdsDmaKick { address: addr_gpu, dwords: n as u32 * 2, dest_offset: fs.tile_buffer_sh as u32 };
        if !dma_push(&mut s.kicks, &mut nkicks, k) {
            return dma_kicks_full(b);
        }
        shared_dw = shared_dw.max(fs.tile_buffer_sh as u32 + n as u32 * 2);
        b.frame_tile_buffers = n as u32;
    }
    if nkicks != 0 {
        let mut ui = PvrPdsRenderColorUploadInfo::default();
        let mut ud = [0u32; 3 * PVR_PDS_DMA_MAX_KICKS as usize];
        let mut uc = [0u32; PVR_PDS_DMA_MAX_KICKS as usize + 1];

        // SAFETY: ud/uc have the sizes the generator documents for up to
        // PVR_PDS_DMA_MAX_KICKS kicks; nkicks is within that.
        if unsafe {
            pvr_pds_render_dma_program(s.kicks.as_ptr(), nkicks as u32, 0, ud.as_mut_ptr(), uc.as_mut_ptr(), &mut ui)
        } != 0
        {
            glog_once!(
                "yakogl/pvr: draw refused: the fragment common-store DMA program was refused ({} kicks)",
                nkicks
            );
            return -EINVAL;
        }
        let ud = &ud[..(ui.data_size_dw as usize).min(ud.len())];
        let uc = &uc[..(ui.code_size_dw as usize).min(uc.len())];
        rc = pds_put(b, ud, &mut t_pds_data);
        if rc == 0 {
            rc = pds_put(b, uc, &mut t_pds_code);
        }
        if rc != 0 {
            return rc;
        }
        t_data_bytes = ui.data_size_dw * 4;
        t_temps = ui.temps_used;
    } else {
        /* No common-store DMA: still bind a valid PDS program (the background
         * colour DMA) with pds_texturestatesize = 0, exactly as the proven rig
         * does; a 0 pointer would name the VDM_SYNC program at PDS offset 0. */
        t_pds_data = b.bg_dma_data_off;
        t_pds_code = b.bg_dma_code_off;
    }

    /* ---- PPP state block ---- */
    let mut fi = PvrPppFragInputs::ZERO;
    let mut st = PvrPppState::ZERO;
    let mut ppp_n = 0u32;
    let mut ppp_gpu = 0u64;

    fi.pixel_program_code_offset = k_pds_data;
    fi.texture_program_code_offset = t_pds_code;
    fi.texture_program_data_offset = t_pds_data;
    fi.texture_program_data_size_bytes = t_data_bytes;
    fi.texture_program_temps = t_temps;
    fi.shared_reg_count = shared_dw;
    fi.isp_userpass = 0;
    /* Pass type, as Mesa's pvr chooses it (pvr_arch_pipeline.c):
     *  - a program that writes gl_FragDepth runs DEPTH_FEEDBACK, ahead of
     *    the kill there: the ISP takes the pixel's depth from the feedback
     *    the program emits. A program never both kills and feeds depth
     *    back -- the chain refuses a shader that writes gl_FragDepth and
     *    discards -- so the two branches here are exclusive;
     *  - a shader that kills pixels (discard/texkill) runs PUNCH_THROUGH so
     *    the ISP resolves tag/visibility feedback (TRANSLUCENT deadlocks it);
     *  - depth-writing, non-blended geometry runs OPAQUE, which lets the ISP
     *    do hidden-surface removal: only the front-most fragment per pixel is
     *    shaded, and, crucially, front geometry submitted before back geometry
     *    in the same kick still wins (Qt's opaque batches are sorted
     *    front-to-back and rely on the depth test, not on submission order);
     *  - everything else (blended, reading the pixel -- a colour-masked draw
     *    or clear keeps channels of what is under it, and Mesa's pvr runs
     *    `uses.fbfetch` TRANSLUCENT -- or depth-write off) runs TRANSLUCENT,
     *    which keeps submission order and composites every fragment.
     * A draw with no colour output (a stencil-only or depth-only pass) has no
     * fragment to defer, so it stays TRANSLUCENT. */
    /* A draw counted by an occlusion query is TRANSLUCENT, not OPAQUE: its
     * visibility test must see the depth buffer as the draws before it left
     * it, in submission order, not the hidden-surface result of the whole
     * kick (docs/es3/occlusion-query.md 3.2, "TBDR ordering hazard" 2; a
     * conservative choice, unverified on the ISP). */
    let counted = d.occlusion_query != 0 && d.occlusion_query < GLES_QUERY_HANDLES && !b.vis.cpu.is_null();
    let passtype = if fs.writes_depth != 0 {
        /* a written gl_FragDepth: the ISP takes the depth from the program */
        PVR_PPP_PASSTYPE_DEPTH_FEEDBACK
    } else if fs.uses_discard != 0 {
        PVR_PPP_PASSTYPE_PUNCH_THROUGH
    } else if d.depth_test && d.depth_write && !d.blend && fs.uses_fbfetch == 0 && d.color_write && !counted {
        PVR_PPP_PASSTYPE_OPAQUE
    } else {
        PVR_PPP_PASSTYPE_TRANSLUCENT
    };
    fi.passtype = passtype as u8;
    fi.varying_program_code_offset = coeff_off;

    st.objtype = objtype;
    if counted {
        /* Mesa's pvr_setup_isp_faces_and_control: vistest, visreg = the
         * query's slot (here its handle) in the kick's result area */
        st.vis_test = 1;
        st.vis_reg = d.occlusion_query;
        b.vis_frame |= 1u64 << d.occlusion_query;
    }
    if objtype == PVR_TA_OBJTYPE_LINE {
        /* ISPA.pointlinewidth: 4.4 fixed point, minus 1 (0 stays 0), max 255
         * (Mesa). Point sprites take their size from the VS's gl_PointSize
         * output instead (OUTPUT_SEL.psprite_size_pres), never from here. */
        let px = d.line_width;
        let mut w: u32 = if px <= 0.0 { 0 } else { (px * 16.0 + 0.5) as u32 };

        if w != 0 {
            w -= 1;
        }
        if w > 255 {
            w = 255;
        }
        st.point_line_width = w;
    }
    if d.prim == GLES_PRIM_POINTS && vs.vs_psize_off == SHC_NO_PSIZE && !b.warned_no_psize {
        glog!("pvr_backend: GL_POINTS with a vertex shader that does not write gl_PointSize: the sprite size is undefined");
        b.warned_no_psize = true;
    }
    /* lines (and points) must not be triangle-merged by the PDS (Mesa) */
    st.tri_merge_disable = (objtype != PVR_TA_OBJTYPE_TRIANGLE) as i32;
    if d.depth_test {
        st.depth_cmp = map_cmp(d.depth_func);
        st.depth_write = d.depth_write as i32;
    } else {
        st.depth_cmp = PVR_TA_CMPMODE_ALWAYS;
        st.depth_write = 0;
    }
    if d.stencil {
        /* GlesStencilOp is the ISPB op encoding */
        st.stencil_cmp = map_cmp(d.stencil_func);
        st.stencil_ref = d.stencil_ref as u32;
        st.stencil_cmp_mask = d.stencil_cmp_mask as u32;
        st.stencil_write_mask = d.stencil_write_mask as u32;
        st.stencil_sop_fail = d.stencil_sfail;
        st.stencil_sop_dfail = d.stencil_dpfail;
        st.stencil_sop_pass = d.stencil_dppass;
        st.stencil_back_cmp = map_cmp(d.stencil_back_func);
        st.stencil_back_ref = d.stencil_back_ref as u32;
        st.stencil_back_cmp_mask = d.stencil_back_cmp_mask as u32;
        st.stencil_back_write_mask = d.stencil_back_write_mask as u32;
        st.stencil_back_sop_fail = d.stencil_back_sfail;
        st.stencil_back_sop_dfail = d.stencil_back_dpfail;
        st.stencil_back_sop_pass = d.stencil_back_dppass;
        /* Two-sided only when the faces differ and the object has faces.
         * The TA's front words apply to its clockwise triangles; the
         * winding the TA sees equals the GL window-space winding exactly
         * when the cull mapping below needs no swap (flipped or
         * pretransformed targets), so the GL front state goes into the
         * front words when the GL front face is clockwise there. */
        if objtype == PVR_TA_OBJTYPE_TRIANGLE
            && (st.stencil_back_cmp != st.stencil_cmp
                || st.stencil_back_ref != st.stencil_ref
                || st.stencil_back_cmp_mask != st.stencil_cmp_mask
                || st.stencil_back_write_mask != st.stencil_write_mask
                || st.stencil_back_sop_fail != st.stencil_sop_fail
                || st.stencil_back_sop_dfail != st.stencil_sop_dfail
                || st.stencil_back_sop_pass != st.stencil_sop_pass)
        {
            let ta_winding_swapped = !d.flip_y && b.opt(PVR_BACKEND_OPT_PRETRANSFORM) == 0;
            let front_cw_in_ta = d.front_cw != ta_winding_swapped;

            st.two_sided = 1;
            if !front_cw_in_ta {
                core::mem::swap(&mut st.stencil_cmp, &mut st.stencil_back_cmp);
                core::mem::swap(&mut st.stencil_ref, &mut st.stencil_back_ref);
                core::mem::swap(&mut st.stencil_cmp_mask, &mut st.stencil_back_cmp_mask);
                core::mem::swap(&mut st.stencil_write_mask, &mut st.stencil_back_write_mask);
                core::mem::swap(&mut st.stencil_sop_fail, &mut st.stencil_back_sop_fail);
                core::mem::swap(&mut st.stencil_sop_dfail, &mut st.stencil_back_sop_dfail);
                core::mem::swap(&mut st.stencil_sop_pass, &mut st.stencil_back_sop_pass);
            }
        }
    } else {
        /* GL: stencil test off = always pass, stencil buffer untouched */
        st.stencil_cmp = PVR_TA_CMPMODE_ALWAYS;
        st.stencil_sop_fail = PVR_TA_STENCILOP_KEEP;
        st.stencil_sop_dfail = PVR_TA_STENCILOP_KEEP;
        st.stencil_sop_pass = PVR_TA_STENCILOP_KEEP;
    }
    st.color_write_disable = if d.color_write { 0 } else { 1 };
    if d.polygon_offset && objtype == PVR_TA_OBJTYPE_TRIANGLE && !b.dbias.impl_.is_null() {
        let e: [f32; 3] = [
            if b.frame_ds.buf().is_some() { d.polygon_offset_units / 8388608.0 } else { d.polygon_offset_units },
            d.polygon_offset_factor,
            0.0, /* no clamp */
        ];
        let ebits = e.map(f32::to_bits);

        /* (compared as bytes, like the C memcmp) */
        if b.ndbias == 0 || b.dbias_last.map(f32::to_bits) != ebits {
            /* a full table: the frame is split (the C code reused the last
             * entry, i.e. drew with another draw's offset) */
            if b.ndbias >= DBIAS_MAX_ENTRIES {
                return frame_full(FullCause::DbiasTable);
            }
            {
                if !b.dbias.cpu.is_null() {
                    mem_write(&b.dbias, b.ndbias * 12, &ebits);
                    cache_flush(&b.dbias, b.ndbias * 12, 12);
                } else if let Some(gpu_write) = osal::ops().and_then(|o| o.gpu_write) {
                    // SAFETY: an OSAL entry of the documented signature; the
                    // source is the 12 bytes of `ebits`.
                    unsafe {
                        gpu_write(&mut b.dbias, (b.ndbias * 12) as usize, ebits.as_ptr() as *const c_void, 12);
                    }
                }
                b.dbias_last = e;
                b.ndbias += 1;
            }
        }
        st.dbias_enable = 1;
        st.dbias_index = b.ndbias.wrapping_sub(1);
    }

    /* What bounds rasterisation: the VIEWPORT, narrowed by the scissor box
     * when the test is on.
     *
     * GL clips a primitive at the view volume and the viewport transform maps
     * that volume onto the viewport rectangle, so nothing GL draws lands
     * outside the viewport. The hardware clipper works to a guard band rather
     * than exactly at the volume -- a performance feature, and fine -- but
     * then something has to bound rasterisation to the viewport, and only the
     * scissor did. With GL_SCISSOR_TEST off, a primitive with a vertex far
     * outside was rasterised past the viewport edge: a border strip on
     * exactly the side the vertex went out, which is what dEQP's clipping
     * group sees. Clipping to the viewport can never drop a fragment GL asks
     * for, so this is right whatever the guard band turns out to be.
     *
     * A rectangle that covers the whole target is left off: nothing can be
     * drawn outside the target anyway, so the common full-screen draw emits
     * the same block it always did. */
    {
        /* Region clip is REGION_CLIP_PIXELS granular. GL y is bottom-up; the
         * region clip is top-down. */
        let Some([x0, top, x1, bot]) = draw_box(d) else {
            return 0; /* record_draw returned before this for an empty box */
        };
        /* the whole target: nothing to clip away */
        let whole = x0 <= 0 && top <= 0 && x1 >= d.target_w as i32 && bot >= d.target_h as i32;
        if !whole {
            st.clip_enable = 1;
            st.clip_left = x0 as u32 / REGION_CLIP_PIXELS;
            st.clip_right = if x1 > 0 { (x1 as u32 - 1) / REGION_CLIP_PIXELS } else { 0 };
            st.clip_top = top as u32 / REGION_CLIP_PIXELS;
            st.clip_bottom = if bot > 0 { (bot as u32 - 1) / REGION_CLIP_PIXELS } else { 0 };
            /* the pixel-exact part: an ISP scissor table entry (the region
             * clip above only drops whole tiles; Mesa sets both). Each half
             * is two 16-bit fields the IPF reads straight from this memory
             * table, not a PPP/VDM stream word, but with the same hardware
             * bound (Mesa csbgen rogue/ipf.xml:150-166: SCISSOR_WORD_0
             * scw0_xmin[31:16]/scw0_xmax[15:0], SCISSOR_WORD_1
             * scw1_ymin[31:16]/scw1_ymax[15:0]): a coordinate that does not
             * fit is refused, not folded into the low bits of a smaller,
             * silently wrong rectangle. */
            if !b.scissor.impl_.is_null() && x1 >= x0 && bot >= top {
                let mut w = [0u32; 2];
                let mut why = PvrVdmFieldRefusal::ZERO;

                // SAFETY: `w` holds the two dwords the builder writes.
                if unsafe {
                    pvr_isp_build_scissor_word(x0 as u32, x1 as u32, top as u32, bot as u32, w.as_mut_ptr(), &mut why)
                } < 0
                {
                    return vdm_field_refused(&why);
                }

                if b.nscissor == 0 || b.scissor_last != w {
                    /* a full table: the frame is split (the C code reused the
                     * last entry, i.e. clipped to another draw's rectangle) */
                    if b.nscissor >= SCISSOR_MAX_ENTRIES {
                        return frame_full(FullCause::ScissorTable);
                    }
                    {
                        if !b.scissor.cpu.is_null() {
                            mem_write(&b.scissor, b.nscissor * 8, &w);
                            cache_flush(&b.scissor, b.nscissor * 8, 8);
                        } else if let Some(gpu_write) = osal::ops().and_then(|o| o.gpu_write) {
                            // SAFETY: an OSAL entry of the documented signature; the
                            // source is the 8 bytes of `w`.
                            unsafe {
                                gpu_write(&mut b.scissor, (b.nscissor * 8) as usize, w.as_ptr() as *const c_void, 8);
                            }
                        }
                        b.scissor_last = w;
                        b.nscissor += 1;
                    }
                }
                st.scissor_enable = 1;
                st.scissor_index = b.nscissor.wrapping_sub(1);
            }
        }
    }

    if b.opt(PVR_BACKEND_OPT_PRETRANSFORM) == 0 {
        /* GL viewport -> TA viewport words (Mesa pvr_setup_viewport). Window y
         * is bottom-up in GL; the framebuffer rows are top-down, so flip y with
         * a negative y scale. NDC z [-1,1] -> window [0,1]. */
        let (vx, vy) = (d.viewport[0] as f32, d.viewport[1] as f32);
        let (vw, vh) = (d.viewport[2] as f32, d.viewport[3] as f32);
        let top = if d.flip_y { d.target_h as f32 - vy - vh } else { vy };
        let ysc = if d.flip_y { -vh * 0.5 } else { vh * 0.5 };

        st.viewport_enable = 1;
        st.viewport[0] = fbits(vx + vw * 0.5);
        st.viewport[1] = fbits(vw * 0.5);
        st.viewport[2] = fbits(top + vh * 0.5);
        st.viewport[3] = fbits(ysc);
        st.viewport[4] = fbits((d.depth_range[0] + d.depth_range[1]) * 0.5);
        st.viewport[5] = fbits((d.depth_range[1] - d.depth_range[0]) * 0.5);
        st.pretransform = 0;
        st.drawclippededges = 1;
        st.wclampen = 1;
        st.flatshade_vtx = PVR_TA_FLATSHADE_VERTEX_0;
        /* Near/far clipping is ON by default now that the TA clips GL's
         * volume ([`ppp_ctrl`]'s `opengl` bit). The knob keeps the values
         * z_probe and gl_test case 62 already pass -- 1 FRONT_REAR,
         * 2 FRONT_REAR_DEPTH -- so nothing on a board has to be rebuilt to
         * mean what it meant; what changed is that an application asking for
         * nothing gets the conformant answer, and that turning it OFF now
         * takes an explicit PVR_DEPTH_CLIP_OFF instead of being the default. */
        st.clip_mode = match b.opt(PVR_BACKEND_OPT_DEPTH_CLIP) {
            PVR_DEPTH_CLIP_OFF => PVR_TA_CLIP_MODE_NONE,
            PVR_DEPTH_CLIP_FRONT_REAR_DEPTH => PVR_TA_CLIP_MODE_FRONT_REAR_DEPTH,
            _ => PVR_TA_CLIP_MODE_FRONT_REAR,
        };
    } else {
        st.pretransform = 1; /* screen-space vertices, as the test rig */
    }
    /* d.cull names the winding to cull in GL window space (y up). On
     * silicon the TA's CW/CCW matches that directly when the viewport
     * transform flips y into the top-down window surface (gl_test case 4);
     * rendering into a texture does not flip, which inverts the winding
     * the TA sees, so swap there. Pretransformed vertices are the rig's
     * screen-space layout and are left alone. */
    {
        /* GL culls polygons only: a line or a point is never culled (ES 3.0
         * 13.6.1). The TA applies its cull test to whatever the object type
         * generates, so a line list drawn by a material with back-face
         * culling on -- Qt's PrincipledMaterial by default -- would have the
         * hardware cull lines by the winding of the quad it expands them
         * into. */
        let mut cull = match d.prim {
            GLES_PRIM_TRIANGLES | GLES_PRIM_TRI_STRIP | GLES_PRIM_TRI_FAN => d.cull,
            _ => GLES_CULL_NONE,
        };

        if cull != GLES_CULL_NONE && !d.flip_y && b.opt(PVR_BACKEND_OPT_PRETRANSFORM) == 0 {
            cull = if cull == GLES_CULL_CW { GLES_CULL_CCW } else { GLES_CULL_CW };
        }
        st.cullmode = match cull {
            GLES_CULL_CW => PVR_TA_CULLMODE_CW,
            GLES_CULL_CCW => PVR_TA_CULLMODE_CCW,
            _ => PVR_TA_CULLMODE_NONE,
        };
    }

    st.vtxsize = vs.vs_output_size as u32;
    st.rhw_pres = (fs.uses_w != 0) as i32;
    st.z_pres = (fs.uses_z != 0) as i32;
    st.psprite_size_pres = (vs.vs_psize_off != SHC_NO_PSIZE) as i32;
    /* ShcShader carries no per-VS varying list: every VS varying the
     * offline compiler emits is smooth (perspective), so classify all of the
     * non-position output dwords as f32_linear. Without an RHW plane
     * (uses_w = 0) they are iterated non-perspective (see the coefficient
     * program above), so they belong to the f32_npc partition instead. */
    {
        /* the user varyings: everything after the position, minus the point
         * size dword (which is not interpolated) */
        let mut nvar: u32 = if vs.vs_output_size > 4 { vs.vs_output_size as u32 - 4 } else { 0 };

        if vs.vs_psize_off != SHC_NO_PSIZE && nvar != 0 {
            nvar -= 1;
        }

        st.f32_linear = if fs.uses_w != 0 { nvar } else { 0 };
        st.f32_npc = if fs.uses_w != 0 { 0 } else { nvar };
    }
    st.usc_varyingsize = coeff.usc_varyingsize;
    st.pds_varyingsize = coeff.pds_varyingsize;

    // SAFETY: ppp holds PVR_PPP_BLOCK_MAX_DWORDS, the documented worst case.
    // A size field that does not fit refuses the block (nothing written):
    // the draw fails as a refused VDM state does.
    {
        let mut why = PvrVdmFieldRefusal::ZERO;

        if unsafe { pvr_ppp_build_block(&fi, &st, s.ppp.as_mut_ptr(), &mut ppp_n, &mut why) } < 0 {
            return vdm_field_refused(&why);
        }
    }
    /* The DDK/rig layout: the 3-word terminate block sits right after the
     * PPP block (ppp + words*4); the stream's trailing PPP_STATE_UPDATE
     * references it there. Emit both together so a single reference works. */
    {
        let ppp_words = ppp_n as usize;

        if ppp_words + 3 > s.ppp.len() {
            return -EINVAL; /* beyond the generator's documented maximum */
        }
        let mut why = PvrVdmFieldRefusal::ZERO;
        // SAFETY: three more dwords fit behind the block (checked above).
        let tail = unsafe {
            pvr_ppp_build_terminate_block(d.target_w, d.target_h, s.ppp.as_mut_ptr().add(ppp_words), &mut why)
        };
        if tail < 0 {
            return vdm_field_refused(&why);
        }

        let total = (ppp_words + tail as usize).min(s.ppp.len());
        rc = arena_put_words(&mut b.gen, &s.ppp[..total], PPP_ALIGN, None, Some(&mut ppp_gpu));
        if rc != 0 {
            return rc;
        }
        b.term_gpu = ppp_gpu + ppp_n as u64 * 4;
    }
    if b.opt(PVR_BACKEND_OPT_DEBUG) != 0 {
        glog!(
            "  ppp @0x{:x} ({} words) k_pds=0x{:x} coeff=0x{:x} tex d/c=0x{:x}/0x{:x} shared={}",
            ppp_gpu,
            ppp_n,
            k_pds_data,
            coeff_off,
            t_pds_data,
            t_pds_code,
            shared_dw
        );
        dump_words("ppp", &s.ppp[..ppp_n as usize]);
    }

    /* ---- vertex uniforms: a DOUTD program the VDM runs on a PDS state
     * update, filling the VS's shared registers (sh0..) from the block ---- */
    let mut ps = PvrVdmPdsState::default();
    let mut has_ps = false;

    let vs_textures = &d.vs_textures[..(d.num_vs_textures as usize).min(d.vs_textures.len())];

    if d.vs_uniform_dwords != 0 || d.num_vs_textures != 0 || stage_sampler_count(Some(vs)) != 0 {
        /* The vertex stage's shared registers, in the order the compiler laid
         * them out: its texture descriptors first, then the uniform block. A
         * vertex shader that samples (Qt Quick 3D's bone and morph-target
         * textures) reads its image state from sh0.., so the block cannot
         * start there. */
        let mut vk = [PvrPdsDmaKick::ZERO; PVR_PDS_DMA_MAX_KICKS as usize];
        let mut nvk = 0usize;
        let mut vs_shared_dw = 0u32;
        let mut vdesc_gpu = 0u64;
        let mut vdesc_dw = 0u32;

        if build_desc_block(b, Some(vs), vs_textures, &mut vdesc_gpu, &mut vdesc_dw) != 0 {
            return -ENOMEM;
        }
        if vdesc_dw != 0 {
            vk[nvk] = PvrPdsDmaKick { address: vdesc_gpu, dwords: vdesc_dw, dest_offset: vs_shared_dw };
            nvk += 1;
            vs_shared_dw += vdesc_dw;
        }
        /* Snapshot this draw's VS uniforms into the per-frame arena: the
         * source (pg->uniforms) is shared per program and is overwritten
         * by the next draw, so a kick with several draws sharing a program
         * would otherwise read the last draw's block. */
        let mut vs_uni_gpu = d.vs_uniform_gpu;
        let mut vs_uni_dw = d.vs_uniform_dwords as u32;

        if PVR_BACKEND_UNIFORM_PREFIX && vs.uniform_sh_dwords != 0 && (vs.uniform_sh_dwords as u32) < vs_uni_dw {
            vs_uni_dw = vs.uniform_sh_dwords as u32; /* the prefix the shader reads */
        }
        if vs_uni_dw != 0 && !d.vs_uniform_cpu.is_null() {
            // SAFETY: the core's uniform block holds vs_uniform_dwords dwords,
            // and vs_uni_dw is at most that.
            rc = unsafe {
                arena_put(&mut b.gen, d.vs_uniform_cpu as *const u8, vs_uni_dw * 4, 16, None, Some(&mut vs_uni_gpu))
            };
            if rc != 0 {
                return rc;
            }
        }
        if vs_uni_dw != 0 {
            vk[nvk] = PvrPdsDmaKick { address: vs_uni_gpu, dwords: vs_uni_dw, dest_offset: vs_shared_dw };
            nvk += 1;
            vs_shared_dw += vs_uni_dw;
        }
        let mut ui = PvrPdsRenderColorUploadInfo::default();
        let mut ud = [0u32; 3 * PVR_PDS_DMA_MAX_KICKS as usize];
        let mut uc = [0u32; PVR_PDS_DMA_MAX_KICKS as usize + 1];

        // SAFETY: nvk kicks in `vk`; ud/uc have the sizes the generator documents.
        if nvk == 0
            || unsafe {
                pvr_pds_render_dma_program(vk.as_ptr(), nvk as u32, 0, ud.as_mut_ptr(), uc.as_mut_ptr(), &mut ui)
            } != 0
        {
            glog_once!("yakogl/pvr: draw refused: the vertex common-store DMA program was refused ({} kicks)", nvk);
            return -EINVAL;
        }
        let ud = &ud[..(ui.data_size_dw as usize).min(ud.len())];
        let uc = &uc[..(ui.code_size_dw as usize).min(uc.len())];
        rc = pds_put(b, ud, &mut ps.pds_data_offset);
        if rc == 0 {
            rc = pds_put(b, uc, &mut ps.pds_code_offset);
        }
        if rc != 0 {
            return rc;
        }
        ps.pds_data_size_bytes = ui.data_size_dw * 4;
        ps.pds_temps = ui.temps_used;
        ps.usc_common_size_bytes = vs_shared_dw * 4;
        has_ps = true;
        if b.opt(PVR_BACKEND_OPT_DEBUG) != 0 && b.dbg_draws_left != 0 {
            glog!(
                "  vs uniforms: {} dwords @0x{:x}, pds d/c=0x{:x}/0x{:x}",
                d.vs_uniform_dwords,
                d.vs_uniform_gpu,
                ps.pds_data_offset,
                ps.pds_code_offset
            );
            dump_words("vs_uni_prog", ud);
            /* ...and the block itself, which is what a shader that computes
             * a constant (the skybox's eye direction) is reading. */
            if !d.vs_uniform_cpu.is_null() {
                // SAFETY: the core's block holds vs_uniform_dwords dwords and
                // is a dword array of the library's own (the program's
                // uniforms or the GPU buffer's CPU view), so it is aligned.
                let blk = unsafe {
                    core::slice::from_raw_parts(d.vs_uniform_cpu as *const u32, (d.vs_uniform_dwords as usize).min(160))
                };
                dump_words("vs_uni_block", blk);
            }
        }
    }

    /* ---- VDM stream: [PDS state,] PPP state, VDM state, index list(s) ---- */
    {
        let mut vss = PvrVdmVsState::default();

        if (b.vdm_dw as usize + 3 + 2 + 5 + 4 + 3 + 2 + 1) * 4 > b.vdm.size {
            return frame_full(FullCause::VdmStream);
        }
        vss.vs_pds_data_offset = v_pds_data;
        vss.vs_pds_code_offset = v_pds_code;
        vss.vs_pds_data_size_bytes = vinfo.data_size_dw * 4;
        vss.vs_pds_temps = vinfo.temps_used;
        vss.vs_output_size_bytes = vs.vs_output_size as u32 * 4;
        /* the input vertex: every vtxin register a stream DMAs (the
         * unified store per instance is sized from this, not from the
         * outputs -- a 9-input/8-output shader lost vertices 16.. ) */
        {
            let mut vin = 0u32;

            for a in attribs {
                let end = a.vi_reg as u32 + a.components as u32;

                if end > vin {
                    vin = end;
                }
            }
            vss.vs_input_size_bytes = vin * 4;
        }
        vss.flatshade_control = 0;

        let base = b.vdm.cpu as *mut u32;
        let mut dw = b.vdm_dw as usize;
        let mut why = PvrVdmFieldRefusal::ZERO;
        // SAFETY: the VDM stream is CPU-mapped (alloc_mapped) and the worst
        // case of the blocks emitted here (20 dwords with an instance count) was checked against its
        // size above; each emitter returns the dwords it wrote. A refusal
        // returns before b.vdm_dw moves, so the words written past it are
        // not part of the stream (record_draw_txn rolls the rest back).
        unsafe {
            if has_ps {
                let n = pvr_vdm_emit_pds_state(base.add(dw), &ps, &mut why);
                if n < 0 {
                    return vdm_field_refused(&why);
                }
                dw += n as usize;
            }
            let n = pvr_vdm_emit_ppp_state(base.add(dw), ppp_gpu, ppp_n, &mut why);
            if n < 0 {
                return vdm_field_refused(&why);
            }
            dw += n as usize;
            let n = pvr_vdm_emit_vdm_state(base.add(dw), &vss, &mut why);
            if n < 0 {
                return vdm_field_refused(&why);
            }
            dw += n as usize;
            let n =
                pvr_vdm_emit_index_list_instanced(base.add(dw), topology, index_gpu, count, d.instance_count, &mut why);
            if n < 0 {
                return vdm_field_refused(&why);
            }
            dw += n as usize;
            if d.index_count2 != 0 && d.index_buf2_gpu != 0 {
                let n = pvr_vdm_emit_index_list(base.add(dw), topology, d.index_buf2_gpu, d.index_count2, &mut why);
                if n < 0 {
                    return vdm_field_refused(&why);
                }
                dw += n as usize;
            }
        }
        b.vdm_dw = dw as u32;
    }
    b.dbg_draws_left = b.dbg_draws_left.saturating_sub(1);
    b.ndraws += 1;
    0
}

/* ------------------------------------------------------------------------- *
 * flush: close the stream, build the kick, submit, wait
 * ------------------------------------------------------------------------- */
fn flush(p: &mut Pvr) -> i32 {
    let mut term = [0u32; 3];
    let mut term_gpu = 0u64;

    if !p.b.inited || !p.b.frame_open {
        return 0;
    }

    /* A kick with nothing to tile leaves the region headers as the geometry
     * phase found them, and a freshly allocated HWRT set is zeros: the
     * fragment phase's IPF then follows a null pointer (BIF fault at VA 0,
     * tag IPF_OTHERS -- the IBL and effects demos, which clear a target with
     * no draw behind it). Give such a frame one covering object that writes
     * no colour, so the geometry phase writes real headers and the clear
     * still comes from the background object. */
    if p.b.ndraws == 0 && p.b.frame_quads == 0 {
        let mut q = GlesDraw::ZERO;

        q.target_gpu = p.b.target_gpu;
        q.target_w = p.b.target_w;
        q.target_h = p.b.target_h;
        q.target_format = p.b.target_format;
        q.samples = p.b.samples as u8;
        q.flip_y = p.b.target_flip_y;
        let _ = record_frame_quad(p, &q, false, false, false, false);
    }
    let b = &mut p.b;

    /* trailing PPP terminate block (after the last draw's PPP block, or a
     * standalone one for a clear-only frame) + STREAM_TERMINATE */
    if b.term_gpu != 0 {
        term_gpu = b.term_gpu;
    } else {
        let mut why = PvrVdmFieldRefusal::ZERO;
        // SAFETY: `term` holds the three dwords the builder writes.
        if unsafe { pvr_ppp_build_terminate_block(b.target_w, b.target_h, term.as_mut_ptr(), &mut why) } < 0 {
            /* a frame that cannot be closed is dropped, as below */
            let rc = vdm_field_refused(&why);
            glog_once!("pvr_backend: the frame's terminate block was refused: frame dropped");
            b.frame_open = false;
            return rc;
        }
        let rc = arena_put_words(&mut b.gen, &term, PPP_ALIGN, None, Some(&mut term_gpu));
        if rc != 0 {
            /* a frame that cannot be closed is dropped, not kept open (every
             * later draw and flush would fail on it again) */
            glog_once!("pvr_backend: no room for the frame's terminate block ({}): frame dropped", rc);
            b.frame_open = false;
            return rc;
        }
    }
    if (b.vdm_dw as usize + 2 + 1) * 4 > b.vdm.size {
        /* unreachable: every draw reserves the trailer's room */
        b.frame_open = false;
        return frame_full(FullCause::VdmStream);
    }
    {
        let base = b.vdm.cpu as *mut u32;
        let mut dw = b.vdm_dw as usize;
        let mut why = PvrVdmFieldRefusal::ZERO;
        // SAFETY: the VDM stream is CPU-mapped (alloc_mapped) and the three
        // trailer dwords fit (checked above).
        let n = unsafe { pvr_vdm_emit_ppp_state(base.add(dw), term_gpu, 3, &mut why) };
        if n < 0 {
            let rc = vdm_field_refused(&why);
            glog_once!("pvr_backend: the frame's trailing PPP state update was refused: frame dropped");
            b.frame_open = false;
            return rc;
        }
        dw += n as usize;
        // SAFETY: as above.
        dw += unsafe { pvr_vdm_emit_terminate(base.add(dw)) } as usize;
        b.vdm_dw = dw as u32;
    }
    if b.opt(PVR_BACKEND_OPT_DEBUG) != 0 {
        glog!("  terminate @0x{:x}: {:08x} {:08x} {:08x}", term_gpu, term[0], term[1], term[2]);
        // SAFETY: the first vdm_dw dwords of the CPU-mapped stream were just written.
        dump_words("vdm", unsafe { core::slice::from_raw_parts(b.vdm.cpu as *const u32, b.vdm_dw as usize) });
    }

    /* per-frame frag-side data: clear colour + EOT data (framebuffer address).
     * The clear FS writes these bytes as the pixel's RGBA output registers, so
     * the clear colour is left in RGBA (pack_rgba8) order exactly like a drawn
     * solid colour; the PBE swizzle (pbe_word[0] below: BGRA for XRGB8888)
     * reorders the channels into the surface's memory layout on emit. */
    let packed = clear_uniform(b.target_format, &b.clear_rgba);
    /* Colour attachments the end-of-tile program actually emits (what the
     * pixel's four output registers hold), and whether the last one's data
     * comes back from a tile buffer. */
    let mut nrt = 0u32;
    /* The attachment slot each emitted state is for: a slot with no target
     * (gpu 0, a GL_NONE draw buffer) is skipped, so emit j can be slot i > j. */
    let mut slot_of = [0usize; PVR_BACKEND_MAX_RT as usize];
    let mut tb_emit = false;
    /* this kick stores every sample of its target too ([`MsStore`]) */
    let store_samples;
    {
        let mut col = [0u32; CLEAR_ATTACH_DWORDS as usize];
        let mut ed = [0u32; 3 * PVR_BACKEND_MAX_RT as usize + 2];
        let mut ec = [0u32; PVR_BACKEND_MAX_RT as usize + 2];
        let (mut dsz, mut csz) = (0u32, 0u32);
        let da = b.target_gpu;

        /* the packed colour: one dword, RGBA16F's two, an integer target's
         * one to four (`clear_uniform`; zero past what the target takes) */
        col[..4].copy_from_slice(&packed);
        /* several 8-bit attachments: the same packed colour for each (the
         * PBE converts it to each attachment's format) */
        for c in col.iter_mut().take(mrt_clear_dwords(b) as usize) {
            *c = packed[0];
        }
        mem_write(&b.bg_color, 0, &col);
        cache_flush(&b.bg_color, 0, CLEAR_ATTACH_DWORDS * 4);

        /* PBESTATE_STATE_WORD0/1 per colour attachment ([`pbe_emit_words`]:
         * the address, its source, its packing). The end-of-tile program reads attachment i's
         * pair from shared registers 2i, 2i+1; the PDS CODE that fills them
         * is generated once for the largest count, so the data always carries
         * that many pairs and the unused ones stay zero. */
        let nrt_want = if b.num_targets != 0 { b.num_targets as u32 } else { 1 };
        let mut emit = [0u32; 2 * PVR_BACKEND_MAX_RT as usize];
        let mut out_regs = 0u32;

        for i in 0..nrt_want {
            let a = if i != 0 { b.targets[i as usize].gpu } else { da };
            let fmt = if i != 0 { b.targets[i as usize].format } else { b.target_format };

            /* Which attachment this state is for, and which 32-bit slot of
             * the shader's output registers it takes: attachment i emits
             * output i, and an output is one register for the 8-bit formats,
             * two for RGBA16F, an integer target's packed dwords for it
             * (`surf_out_dwords`; PBESTATE_STATE_WORD1 mrt_index at 16..18,
             * source_pos at 14..15). A 64-bit source starts at an even
             * register (pbe2_in_xe), and a pixel has four of them: the
             * compiler lays the outputs out the same way, and an attachment
             * past the fourth register is not emitted -- writing it would
             * have the USC run past its output registers, which faults the
             * GPU -- unless the shader stored it to a tile buffer, which the
             * end-of-tile program loads back from dword 0. */
            let regs = surf_out_dwords(fmt);
            let mut spos = out_regs;

            if regs >= 2 && (spos & 1) != 0 {
                spos += 1;
            }
            if spos + regs > PVR_BACKEND_MAX_OUT_REGS {
                /* Only the last attachment can come from a tile buffer (the
                 * compiler spills in output order, and one program per count
                 * is captured). */
                if b.frame_tile_buffers != 0 && i + 1 == nrt_want && b.st.eot_doutu_src0_tb_n[nrt_want as usize] != 0 {
                    spos = 0;
                    tb_emit = true;
                } else {
                    if !b.warned_out_regs {
                        b.warned_out_regs = true;
                        glog!(
                            "pvr_backend: colour attachment {} needs output register {} of {}: not emitted",
                            i,
                            spos,
                            PVR_BACKEND_MAX_OUT_REGS
                        );
                    }
                    break;
                }
            }
            if !tb_emit {
                out_regs = spos + regs;
            }
            /* A slot with no target (gpu 0: a GL_NONE draw buffer between two
             * attachments, see resolve_targets) is configured but never
             * stored: the shader still writes its output registers, which are
             * counted above so the later slots read theirs, but it gets no
             * emit, so nothing is stored for it. A zeroed state pair would
             * store the tile to GPU address 0. Slot 0 is always the frame's
             * own target. */
            if i != 0 && a == 0 {
                continue;
            }
            let j = nrt as usize;
            slot_of[j] = i as usize;
            nrt += 1;
            /* A z-only frame (no colour target, i == 0 with a == 0): its one
             * emit is the empty-tile state ([`pbe_emit_words_z_only`]). */
            let words = if a == 0 { pbe_emit_words_z_only() } else { pbe_emit_words(a, fmt, spos, j as u32) };
            emit[j * 2..j * 2 + 2].copy_from_slice(&words);
        }
        if nrt == 0 {
            nrt = 1;
        }
        /* A multisampled frame that may go on in another kick (no swap):
         * its one attachment emitted again, unresolved, into the sample
         * store ([`MsStore`]) -- a second state pair from the same output
         * registers (source_pos 0), mrt_index 1, through the two-emit
         * end-of-tile program. Not with several attachments or a tile
         * buffer (ES 2.0 has neither at 4x), and not for a z-only frame. */
        store_samples = b.samples > 1
            && nrt == 1
            && b.num_targets <= 1
            && !tb_emit
            && b.target_gpu != 0
            && !b.frame_swapped
            && ms_store_ready(b);
        if store_samples {
            emit[2..4].copy_from_slice(&pbe_emit_words(b.ms.mem.gpu, b.target_format, 0, 1));
            nrt = 2;
        }
        if nrt > 1 && b.opt(PVR_BACKEND_OPT_DEBUG) != 0 && b.mrt_log_left != 0 {
            b.mrt_log_left -= 1;
            glog!(
                "pvr_backend: {} colour attachments, emit words {:08x} {:08x} / {:08x} {:08x} / {:08x} {:08x}",
                nrt,
                emit[0],
                emit[1],
                emit[2],
                emit[3],
                emit[4],
                emit[5]
            );
            glog!(
                "pvr_backend: eot doutu[{}]=0x{:x} (1 rt: 0x{:x}), pds info 0x{:x}",
                nrt,
                b.st.eot_doutu_src0_n[nrt as usize],
                b.st.eot_doutu_src0,
                b.st.event_pixel_pds_info_n[nrt as usize]
            );
        }
        let doutu = if tb_emit {
            b.st.eot_doutu_src0_tb_n[nrt as usize]
        } else if b.st.eot_doutu_src0_n[nrt as usize] != 0 {
            b.st.eot_doutu_src0_n[nrt as usize]
        } else {
            b.st.eot_doutu_src0
        };
        // SAFETY: PVR_BACKEND_MAX_RT render targets: emit holds 2 * that
        // dwords, ed/ec the documented 3N+1 / N+1.
        if unsafe {
            pvr_pds_generate_pixel_event_tq(
                doutu,
                emit.as_ptr(),
                PVR_BACKEND_MAX_RT,
                0,
                ed.as_mut_ptr(),
                ec.as_mut_ptr(),
                &mut dsz,
                &mut csz,
            )
        } != 0
        {
            /* a frame without its end-of-tile program cannot be closed */
            glog_once!("pvr_backend: the frame's end-of-tile pixel event program was refused: frame dropped");
            b.frame_open = false;
            return -EINVAL;
        }
        mem_write(&b.pds_static, b.eot_data_local, &ed[..(dsz as usize).min(ed.len())]);
        cache_flush(&b.pds_static, 0, PDS_STATIC_BYTES);
    }

    // SAFETY: all-zero is a valid PvrRenderKick; zeroing it in place (rather
    // than assigning a value) also zeroes its padding, like the C memset.
    unsafe { core::ptr::write_bytes(&mut b.kick as *mut PvrRenderKick, 0, 1) };
    b.kick.width = b.target_w;
    b.kick.height = b.target_h;
    b.kick.samples = b.samples;
    b.kick.geom.vdm_ctrl_stream_base = b.vdm.gpu;
    b.kick.geom.ppp_ctrl = if b.opt(PVR_BACKEND_OPT_PPP_CTRL) != 0 {
        b.opt(PVR_BACKEND_OPT_PPP_CTRL)
    } else {
        ppp_ctrl(b.opt(PVR_BACKEND_OPT_PRETRANSFORM) != 0)
    };
    b.kick.geom.te_psg = te_psg();

    /* CR_USC_PIXEL_OUTPUT_CTRL.width says how many output registers a pixel
     * has (ROGUE_CR_PIXEL_WIDTH: 0: 2, 1: 4, 2: 8, 3: 1); four, the most this
     * core has. */
    let pixel_width = csb::ROGUE_CR_PIXEL_WIDTH_4REGISTERS;
    b.kick.frag.usc_pixel_output_ctrl = usc_pixel_output_ctrl(pixel_width);
    /* The USC clear register is what the tiles start from (the background
     * object only runs on empty tiles); a frame that keeps its colour drew
     * the load quad first, so the register value does not matter then. */
    b.kick.frag.usc_clear_register[0] = packed[0];
    /* Several 8-bit attachments ([`mrt_clear_dwords`]): each output register
     * starts from the colour too. A frame with any object on a tile starts
     * that tile from these registers, not from the background object, and a
     * clear-only frame always has one (the covering quad `flush` records):
     * with register 0 alone, gl_test 106's clear left attachments 1 and 2 at
     * 0x00000000 while the background program wrote all three (am62p-board,
     * 5d194af8, both arms). */
    let n = mrt_clear_dwords(b) as usize;
    for r in b.kick.frag.usc_clear_register.iter_mut().take(n).skip(1) {
        *r = packed[0];
    }
    /* The background object's depth is what the frame's depth buffer starts
     * from: the GL depth clear value (window space, 0..1), in the ZLS
     * format's encoding (24-bit unorm with a depth/stencil buffer, float
     * bits for the on-chip F32Z default). Tiles the ISP loads take the
     * stored values instead. */
    if let Some(ds_gpu) = b.frame_ds().map(|ds| ds.mem.gpu) {
        /* CR_ISP_ZLSCTL's extents count ISP TILES, not ZLS tiles, and the
         * count is rounded up to a multiple of two
         * (Mesa pvr_arch_job_render.c: rogue_get_isp_num_tiles_xy over the
         * extent already aligned to the ZLS tile size, then minus one;
         * rogue_hw_utils.h:154 divides by tile_size_x/y = 16 and
         * :161 aligns the result to 2). Dividing by ZLS_TILE instead gave
         * half the count, and the ISP then stored half the buffer: a 64x64
         * target came back with depth and stencil for y 0..31 only, so a
         * stencil written in one kick was not all there in the next, which
         * is 56 dEQP fbo.render cases (RESULTS.md, 2026-09-22).
         *
         * (the draw rejected a zero size: tx, ty >= 1)
         *
         * The size is the ZLS-tile-aligned one, which is what `ds_bytes`
         * allocates and what the ISP writes. */
        let tx = align_up_pow2(b.target_w.div_ceil(ZLS_TILE) * ZLS_TILE / ISP_TILE, 2);
        let ty = align_up_pow2(b.target_h.div_ceil(ZLS_TILE) * ZLS_TILE / ISP_TILE, 2);
        let dz: f32 = if b.clear_depth < 0.0 {
            0.0
        } else if b.clear_depth > 1.0 {
            1.0
        } else {
            b.clear_depth
        };

        /* unfused multiply-add on purpose: see the floating-point note at
         * the top of the file */
        b.kick.frag.isp_bgobjdepth = (dz * 16777215.0 + 0.5) as u32;
        b.kick.frag.isp_zlsctl = isp_zlsctl_stores(tx, ty, b.load_depth, b.load_stencil, !b.swap_drops_zs)
            | if b.target_gpu == 0 { isp_zlsctl_zonlyrender() } else { 0 };
        /* packed D24S8: one buffer, the stencil base without its
         * separate-stencil enable bit (Mesa pvr_arch_job_render.c) */
        b.kick.frag.isp_zload_store_base =
            dw64(csb_pack!(ROGUE_CR_ISP_ZLOAD_BASE_pack, 2, csb::ROGUE_CR_ISP_ZLOAD_BASE { addr: dev_addr(ds_gpu) }));
        /* apart (split_zs_on): the stencil's own buffer, fetched from this
         * base (CR_ISP_STENCIL_LOAD_BASE.enable) */
        let (stencil_gpu, enable) = match b.frame_s.buf() {
            Some(i) => (b.ds[i].mem.gpu, true),
            None => (ds_gpu, false),
        };
        b.kick.frag.isp_stencil_load_store_base = dw64(csb_pack!(
            ROGUE_CR_ISP_STENCIL_LOAD_BASE_pack,
            2,
            csb::ROGUE_CR_ISP_STENCIL_LOAD_BASE { addr: dev_addr(stencil_gpu), enable }
        ));
        b.kick.frag.cmd_flags = FRAG_FLAGS_DEPTHBUFFER | FRAG_FLAGS_STENCILBUFFER;
        /* The depth-bias constant is a 24-bit integer count, which is what
         * ISP_CTL.dbias_is_int selects -- but the bit is applied with the rest
         * of isp_ctl below, not here: an OR at this point was overwritten by
         * the plain assignment that follows, so the kick never carried it.
         * That was the bug behind polygon_offset.*_with_units (the encoder
         * writes units/2^23 for the integer path while the hardware, left in
         * its float mode, scaled the constant by 2^(e-23) again and the offset
         * vanished; the slope term is unaffected, which is why the *_factor*
         * cases passed). Carried from the C and found by reading, 2026-09-22. */
    } else {
        b.kick.frag.isp_bgobjdepth = fbits(b.clear_depth);
    }
    b.kick.frag.isp_dbias_base =
        dw64(csb_pack!(ROGUE_CR_ISP_DBIAS_BASE_pack, 2, csb::ROGUE_CR_ISP_DBIAS_BASE { addr: dev_addr(b.dbias.gpu) }));
    b.kick.frag.isp_scissor_base = dw64(csb_pack!(
        ROGUE_CR_ISP_SCISSOR_BASE_pack,
        2,
        csb::ROGUE_CR_ISP_SCISSOR_BASE { addr: dev_addr(b.scissor.gpu) }
    ));
    /* the background object's stencil byte is what the frame starts from */
    b.kick.frag.isp_bgobjvals = isp_bgobjvals(b.clear_stencil);
    b.kick.frag.isp_aa = isp_aa(b.samples);
    /* dbias_is_int here, with the rest of the word, or it is lost -- see the
     * depth/stencil arm above. Set in a frame with a depth/stencil buffer,
     * so the ISP takes glPolygonOffset's constant as the 24-bit integer
     * count the encoder writes (Mesa
     * pvr_calculate_final_depth_bias_contant_factor). Measured on the AM62P
     * on Zephyr (RESULTS 1i, a53c0122 / f963dee5): dEQP polygon_offset.*
     * 10/4 with the bit clear, 14/0 set; gl_test 65 polyunit FAIL clear,
     * PASS set; the regression set 37/1 both ways, identical per case. */
    let dbias_is_int = b.frame_ds.buf().is_some();
    /* the ISP's tiles in flight from the same pixel width as the output
     * control above (Mesa pvr_arch_setup_tiles_in_flight) */
    b.kick.frag.isp_ctl = isp_ctl(b.samples, pixel_width_regs(pixel_width), dbias_is_int);
    b.kick.frag.event_pixel_pds_info = if tb_emit && b.st.event_pixel_pds_info_tb_n[nrt as usize] != 0 {
        b.st.event_pixel_pds_info_tb_n[nrt as usize]
    } else if b.st.event_pixel_pds_info_n[nrt as usize] != 0 {
        b.st.event_pixel_pds_info_n[nrt as usize]
    } else {
        b.st.event_pixel_pds_info
    };
    b.kick.frag.event_pixel_pds_data = b.eot_data_off;
    /* PBE REG_WORD0/1 per attachment: [`pbe_reg_words`]. */
    b.kick.frag.num_rt = nrt;
    for (rt, &slot) in slot_of.iter().enumerate().take(nrt as usize) {
        /* the sample store's emit (state 1): the target's words again with
         * `downscale` off, so every sample of the tile goes out, linear over
         * the target's own row pitch ([`MsStore`]) */
        if store_samples && rt == 1 {
            let [w0, w1] = pbe_reg_words(
                b.target_format,
                b.target_alpha_bits == 0,
                b.target_w,
                b.target_h,
                b.target_pitch,
                false,
                false,
            );
            b.kick.frag.pbe_word[rt][0] = w0;
            b.kick.frag.pbe_word[rt][1] = w1;
            continue;
        }
        /* PBE register words per emitted state (its mrt_index), from the
         * attachment slot that state is for */
        let fmt = if slot != 0 { b.targets[slot].format } else { b.target_format };
        /* a chain level's frame: the target is level 1's size (one render
         * target set for the whole chain), the store is clipped to the level */
        let clipped = slot == 0 && b.frame_clip != (0, 0);
        let tw = if slot != 0 {
            b.targets[slot].w
        } else if clipped {
            b.frame_clip.0
        } else {
            b.target_w
        };
        let th = if slot != 0 {
            b.targets[slot].h
        } else if clipped {
            b.frame_clip.1
        } else {
            b.target_h
        };
        let pitch = if slot != 0 { tw.wrapping_add(1) & !1 } else { b.target_pitch };

        /* a view's padded rows are not a twiddled surface */
        let twiddle = rt == 0
            && b.opt(PVR_BACKEND_OPT_PBE_TWIDDLE) != 0
            && tw.is_power_of_two()
            && th.is_power_of_two()
            && pitch == tw.wrapping_add(1) & !1;
        /* a z-only frame stores nothing: its register words are zero, as
         * Mesa's (pvr_arch_job_common.c:102-111) */
        let [w0, w1] = if slot == 0 && b.target_gpu == 0 {
            [0, 0]
        } else {
            pbe_reg_words(fmt, rt == 0 && b.target_alpha_bits == 0, tw, th, pitch, b.samples > 1, twiddle)
        };
        b.kick.frag.pbe_word[rt][0] = w0;
        b.kick.frag.pbe_word[rt][1] = w1;
    }
    /* background object: clear FS kick + colour DMA (CR_PDS_BGRND0/1/3) */
    let shader = match mrt_clear_dwords(b) {
        2 => b.bg_kick2_off,
        3 => b.bg_kick3_off,
        _ if b.target_format == GLES_SURF_RGBA16F || gles_surf_is_srgb(b.target_format) => b.bg_kick2_off,
        _ => b.bg_kick_off,
    };
    b.kick.frag.pds_bgnd[0] = dw64(csb_pack!(
        ROGUE_CR_PDS_BGRND0_BASE_pack,
        2,
        csb::ROGUE_CR_PDS_BGRND0_BASE {
            shader_addr: dev_addr(shader as u64),
            texunicode_addr: dev_addr(b.bg_dma_code_off as u64),
        }
    ));
    b.kick.frag.pds_bgnd[1] = dw64(csb_pack!(
        ROGUE_CR_PDS_BGRND1_BASE_pack,
        2,
        csb::ROGUE_CR_PDS_BGRND1_BASE {
            varying_addr: dev_addr(0),
            texturedata_addr: dev_addr(b.bg_dma_data_off as u64),
        }
    ));
    /* the sizes in the fields' units (cr.xml PDS_BGRND3_SIZEINFO, as Mesa's
     * pvr_arch_cmd_buffer.c fills it): shared registers in 16s (the
     * clear-attach data), texture-state data in 4 dwords, temps in 4s */
    b.kick.frag.pds_bgnd[2] = dw64(csb_pack!(
        ROGUE_CR_PDS_BGRND3_SIZEINFO_pack,
        2,
        csb::ROGUE_CR_PDS_BGRND3_SIZEINFO {
            usc_sharedsize: CLEAR_ATTACH_DWORDS.div_ceil(16),
            pds_texturestatesize: b.bg_dma_data_dw.wrapping_mul(4).div_ceil(4),
            pds_tempsize: b.bg_dma_temps.div_ceil(4),
            ..Default::default()
        }
    ));

    /* publish everything the GPU reads */
    cache_flush(&b.pds.mem, 0, b.pds.bump);
    cache_flush(&b.gen.mem, 0, b.gen.bump);
    cache_flush(&b.vdm, 0, b.vdm_dw * 4);

    if PVR_BACKEND_TRACE {
        glog!(
            "pvr_backend: flush {} draw(s) {}x{} vdm_dw={} clear=0x{:08x} zls={}{}",
            b.ndraws,
            b.target_w,
            b.target_h,
            b.vdm_dw,
            packed[0],
            if b.frame_ds.buf().is_some() { "store" } else { "off" },
            if b.load_depth { "+load" } else { "" }
        );
    }
    if b.opt(PVR_BACKEND_OPT_DEBUG) != 0 {
        let k = &b.kick;

        glog!(
            "  frag: epds_data=0x{:x} pbe0=0x{:x} pbe1=0x{:x} bgnd0=0x{:x} bgnd1=0x{:x} bgnd3=0x{:x}",
            k.frag.event_pixel_pds_data,
            k.frag.pbe_word[0][0],
            k.frag.pbe_word[0][1],
            k.frag.pds_bgnd[0],
            k.frag.pds_bgnd[1],
            k.frag.pds_bgnd[2]
        );
        glog!(
            "  geom: vdm=0x{:x} ppp_ctrl=0x{:x} te_psg=0x{:x} target=0x{:x}",
            k.geom.vdm_ctrl_stream_base,
            k.geom.ppp_ctrl,
            k.geom.te_psg,
            b.target_gpu
        );
    }
    b.frame_open = false;
    b.ds_refused_logged = false; /* the next frame logs its own refusal */

    /* The frame's visibility tests count into a result area of its own,
     * zeroed now, read once the kick has completed (`vis_fold`). */
    let vis_frame = core::mem::take(&mut b.vis_frame);
    let vis_slot = if vis_frame != 0 { vis_take_slot(b) } else { None };
    if let Some(i) = vis_slot {
        let off = (i * VIS_SLOT_BYTES) as u32;
        mem_write(&b.vis, off, &[0u32; GLES_QUERY_HANDLES as usize]);
        cache_flush(&b.vis, off, VIS_SLOT_BYTES as u32);
        let gpu = b.vis.gpu + off as u64;
        b.kick.frag.isp_oclqry_base =
            dw64(csb_pack!(ROGUE_CR_ISP_OCLQRY_BASE_pack, 2, csb::ROGUE_CR_ISP_OCLQRY_BASE { addr: dev_addr(gpu) }));
        b.kick.frag.cmd_flags |= FRAG_FLAGS_GETVISRESULTS;
    } else if vis_frame != 0 {
        b.vis_lost |= vis_frame; /* no area: those queries answer "passed" */
    }

    let Some(submit) = osal::ops().and_then(|o| o.submit) else {
        return -ENODEV; /* pvr_backend_init() checked it */
    };
    // SAFETY: the OSAL's submit takes the kick descriptor and its size.
    let fence = unsafe { submit(&b.kick as *const PvrRenderKick as *const c_void, size_of::<PvrRenderKick>()) };
    if fence < 0 {
        glog_once!("pvr_backend: submit failed ({})", fence as i32);
        b.vis_lost |= vis_frame;
        return fence as i32;
    }
    if let Some(i) = vis_slot {
        (b.vis_fence[i], b.vis_mask[i]) = (fence, vis_frame);
    }
    /* The kick runs while the application records the next frame; the GPU
     * executes kicks in order, so a later kick that samples this target or
     * loads its depth/stencil needs no wait. pvr_backend_finish() (glFinish,
     * read-back, context switch) and the frame-set reuse above wait. */
    b.sets[b.cur].fence = fence;
    b.last_fence = fence;
    b.draws_total = b.draws_total.wrapping_add(b.ndraws);
    b.quads_total = b.quads_total.wrapping_add(b.frame_quads);
    b.tiles_total = b.tiles_total.wrapping_add(b.target_w.wrapping_add(15) / 16 * (b.target_h.wrapping_add(15) / 16));
    b.kicks = b.kicks.wrapping_add(1);
    /* Bring-up: what the first kicks of a run were, to see how many a frame
     * costs and why (a new target, more attachments, a frame the application
     * ended, or a split). */
    if b.kick_log_left != 0 {
        b.kick_log_left -= 1;
        glog!(
            "pvr_backend: kick {}: target 0x{:x} {}x{} fmt {} rt {}, {} draws, {} quads{}",
            fence,
            b.target_gpu,
            b.target_w,
            b.target_h,
            b.target_format,
            nrt,
            b.ndraws,
            b.frame_quads,
            match (b.frame_ds.buf().is_some(), store_samples) {
                (true, true) => ", ds, samples",
                (true, false) => ", ds",
                (false, true) => ", samples",
                (false, false) => "",
            }
        );
    }
    /* the sample store: this kick filled it with every sample of the
     * target, or wrote the target without them */
    let target = MsStoreTarget {
        gpu: b.target_gpu,
        w: b.target_w,
        h: b.target_h,
        pitch: b.target_pitch,
        format: b.target_format,
        samples: b.samples,
    };
    b.ms.settle(target, store_samples);
    ds_settle(b);
    if b.swap_drops_zs {
        /* the swap dropped it (pvr_backend_surface_swapped): this kick
         * stored nothing, so the buffer holds no plane GL keeps (EGL 1.5
         * 3.10.1) */
        b.swap_drops_zs = false;
        b.swap_zs_dropped = b.swap_zs_dropped.wrapping_add(1);
        for slot in [b.frame_ds, b.frame_s] {
            if let Some(i) = slot.buf() {
                b.ds[i].valid = false;
                b.ds[i].planes = [Plane::UNDEFINED; 2];
            }
        }
        return 0;
    }
    if let Some(i) = b.frame_s.buf() {
        b.ds[i].valid = true; /* the ISP stores the separate stencil too */
    }
    if let Some(i) = b.frame_ds.buf() {
        b.ds[i].valid = true; /* the ISP stores this kick's depth/stencil */
        if b.opt(PVR_BACKEND_OPT_DS_DUMP) != 0 {
            let rc = b.wait_fence(fence);
            if rc != 0 {
                return rc;
            }
            ds_dump(&b.ds[i], fence);
        }
    }
    0
}

/// `YakoglBackend::surface_swapped`: `eglSwapBuffers` is posting the window
/// surface whose colour buffer is at `surface_gpu`, before the swap's flush.
/// The open frame rendering into that surface ends with that flush: its
/// kick keeps no samples for a next kick ([`MsStore`]).
///
/// The swap also drops the window's depth/stencil (EGL 1.5 3.10.1: they are
/// undefined after `eglSwapBuffers`): with a depth/stencil buffer behind the
/// frame, the flush that follows stores no depth/stencil (the ZLS store bits
/// off) and the buffer is marked not valid with its planes undefined, so the
/// next frame initialises it instead of loading it. Only that kick changes.
/// The V3D backend does the same (`v3d_surface_swapped`); the hook is called
/// for window surfaces only (`egl.rs`: a pbuffer swap "has no effect").
/// Measured on the AM62P (RESULTS.md, "the swap drops the window's
/// depth/stencil": glmark2 315 -> 550, gl_test 37/38 both ways).
#[no_mangle]
pub extern "C" fn pvr_backend_surface_swapped(surface_gpu: u64) {
    // SAFETY: single-threaded by contract; taken once per entry point.
    let b = unsafe { &mut PVR.get().b };
    if !b.inited || !b.frame_open || b.target_gpu != surface_gpu {
        return;
    }
    b.frame_swapped = true;
    if b.frame_ds.buf().is_some() {
        b.swap_drops_zs = true;
    }
}

#[no_mangle]
pub extern "C" fn pvr_backend_flush() -> i32 {
    // SAFETY: single-threaded by contract; taken once per entry point.
    public_rc(flush(unsafe { PVR.get() }))
}

#[no_mangle]
pub extern "C" fn pvr_backend_finish() -> i32 {
    // SAFETY: single-threaded by contract; taken once per entry point.
    let b = unsafe { &mut PVR.get().b };

    b.wait_fence(b.last_fence)
}

#[no_mangle]
pub extern "C" fn pvr_backend_finish_previous() -> i32 {
    // SAFETY: single-threaded by contract; taken once per entry point.
    let b = unsafe { &mut PVR.get().b };

    /* Everything but the most recent kick: what a swap needs before the
     * previous frame's buffer goes on screen. Waiting by fence, not by set:
     * with several sets in flight the "other" set is not the previous kick.
     * PVR_BACKEND_OPT_ONE_IN_FLIGHT waits for the most recent kick as well. */
    let one = b.opt(PVR_BACKEND_OPT_ONE_IN_FLIGHT) != 0;
    let target = if one {
        b.last_fence
    } else if b.last_fence > 1 {
        b.last_fence - 1
    } else {
        0
    };
    let rc = b.wait_fence(target);
    if rc != 0 && one {
        glog_once!("pvr_backend: one kick in flight: the swap's own kick {} did not complete ({})", target, rc);
    }
    rc
}

/// # Safety
/// `kicks` and `waits` must each be null or valid for a write.
#[no_mangle]
pub unsafe extern "C" fn pvr_backend_counters(kicks: *mut u32, waits: *mut u32) {
    // SAFETY: single-threaded by contract; taken once per entry point.
    let b = unsafe { &PVR.get().b };

    // SAFETY: null or valid, per the contract above.
    unsafe {
        if !kicks.is_null() {
            *kicks = b.kicks;
        }
        if !waits.is_null() {
            *waits = b.waits;
        }
    }
}

/// Objects recorded and 16x16 tiles covered by every kick so far: with the
/// kick latency (`yakogl_zephyr_gpu_times`) this says whether a slow frame is
/// geometry, pixels or per-kick overhead.
///
/// # Safety
/// `draws` and `tiles` must each be null or valid for a write.
#[no_mangle]
pub unsafe extern "C" fn pvr_backend_workload(draws: *mut u32, tiles: *mut u32) {
    // SAFETY: single-threaded by contract; taken once per entry point.
    let b = unsafe { &PVR.get().b };

    // SAFETY: null or valid, per the contract above.
    unsafe {
        if !draws.is_null() {
            *draws = b.draws_total;
        }
        if !tiles.is_null() {
            *tiles = b.tiles_total;
        }
    }
}

/// What the kicks carried beyond the application's draws: full-screen quads
/// the driver added (colour reload / clear) and kicks a frame was split into.
///
/// # Safety
/// `quads` and `splits` must each be null or valid for a write.
#[no_mangle]
pub unsafe extern "C" fn pvr_backend_overhead(quads: *mut u32, splits: *mut u32) {
    // SAFETY: single-threaded by contract; taken once per entry point.
    let b = unsafe { &mut PVR.get().b };

    if !quads.is_null() {
        // SAFETY: the caller guarantees the pointer is writable.
        unsafe { *quads = b.quads_total };
    }
    if !splits.is_null() {
        // SAFETY: the caller guarantees the pointer is writable.
        unsafe { *splits = b.splits_total };
    }
}

#[no_mangle]
pub extern "C" fn pvr_backend_kick_id() -> u32 {
    // SAFETY: single-threaded by contract; taken once per entry point.
    let b = unsafe { &PVR.get().b };

    (b.last_fence as u32).wrapping_add(1) /* the kick being recorded */
}

/// Fence waits that timed out since the process started: a read-back after
/// one of them reads whatever the GPU had finished, not the frame asked for,
/// so a harness counts them around a check instead of trusting its pixels.
#[no_mangle]
pub extern "C" fn pvr_backend_wait_timeouts() -> u32 {
    // SAFETY: single-threaded by contract; taken once per entry point.
    unsafe { PVR.get().b.wait_timeouts }
}

/// Draws and clears refused since the process started because their
/// depth/stencil buffer could not be allocated or its contents were lost
/// that way (`ds_refuse_lost`): each is a GL_OUT_OF_MEMORY the application saw.
#[no_mangle]
pub extern "C" fn pvr_backend_ds_refused() -> u32 {
    // SAFETY: single-threaded by contract; taken once per entry point.
    unsafe { PVR.get().b.ds_refused }
}

/// [`ds_report`]'s line into `buf` (`cap` bytes, NUL-terminated,
/// truncated to fit). For the OSAL's report at process exit.
///
/// # Safety
/// `buf` is null or writable for `cap` bytes.
#[no_mangle]
pub unsafe extern "C" fn pvr_backend_ds_report(buf: *mut c_char, cap: usize) {
    // SAFETY: single-threaded by contract; taken once per entry point.
    let b = unsafe { &PVR.get().b };
    // SAFETY: the caller's contract.
    let mut w = unsafe { crate::util::CStrWriter::new(buf, cap) };
    /* CStrWriter never fails: it truncates */
    let _ = ds_report(b, &mut w);
}

/// The newest kick that may read GPU memory freed now, given the newest kick
/// submitted: the kick being recorded (`last_submitted + 1`) while a frame is
/// open, else `last_submitted` itself. The backend submits only from
/// [`flush`], which closes the frame, so with no frame open no unsubmitted
/// kick holds anything; the memory is free once `last_submitted` completes
/// (docs/notes/pvr-render-pool.md, mechanism 2). The OSAL
/// uses it under `CONFIG_YAKOGL_PVR_POOL_RECLAIM`; it tags every free
/// `last_submitted + 1` otherwise.
#[no_mangle]
pub extern "C" fn pvr_backend_free_fence(last_submitted: u32) -> u32 {
    // SAFETY: single-threaded by contract; taken once per entry point.
    let b = unsafe { &PVR.get().b };

    if b.inited && b.frame_open {
        last_submitted.wrapping_add(1)
    } else {
        last_submitted
    }
}

/// The frame sets the next backend init builds
/// (`CONFIG_YAKOGL_PVR_FRAME_SETS`, `_ARENA_GEN_KB`, `_ARENA_PDS_KB`; the
/// Zephyr OSAL calls this before the backend starts): `sets` kicks recorded
/// ahead of the GPU, each with a GENERAL arena of `gen_kib` and a PDS arena
/// of `pds_kib`. The defaults are 4 x (1024 + 768 KiB). Refused, with nothing changed: -EBUSY once the backend runs with
/// another configuration (the arenas exist; asking for the one it has is
/// 0, as a repeated OSAL init does), -EINVAL for 0 or more than 4 sets, for an arena
/// smaller than one draw's worst case (GEN 16 KiB, PDS 8 KiB), or larger
/// than the sizes before the measurement (4096 KiB GENERAL, 1024 KiB PDS).
#[no_mangle]
pub extern "C" fn pvr_backend_set_frame_arenas(sets: u32, gen_kib: u32, pds_kib: u32) -> i32 {
    // SAFETY: single-threaded by contract; no other reference is live.
    let p = unsafe { PVR.get() };
    let (gen, pds) = (gen_kib.saturating_mul(1024), pds_kib.saturating_mul(1024));
    // SAFETY: as above.
    let cur = unsafe { *ARENA_CFG.get() };
    if p.b.inited {
        /* a repeated OSAL init asks for what the backend already has */
        let same = cur.sets == sets as usize && cur.gen_bytes == gen && cur.pds_bytes == pds;
        return if same { 0 } else { -EBUSY };
    }
    if sets == 0
        || sets as usize > PVR_BACKEND_FRAME_SETS
        || !(GEN_PER_DRAW_MAX..=ARENA_GEN_MAX).contains(&gen)
        || !(PDS_PER_DRAW_MAX..=ARENA_PDS_MAX).contains(&pds)
    {
        return -EINVAL;
    }
    // SAFETY: as above.
    unsafe { *ARENA_CFG.get() = ArenaCfg { sets: sets as usize, gen_bytes: gen, pds_bytes: pds } };
    0
}

/// Before a frame's arenas are reset: what the frame just recorded used.
fn note_arena_hw(b: &mut Backend) {
    let used = [b.gen.bump, b.pds.bump, b.vdm_dw.saturating_mul(4)];
    for (hw, u) in b.arena_hw.iter_mut().zip(used) {
        *hw = (*hw).max(u);
    }
}

/// The backend's lines of the run-end owner report
/// (`CONFIG_YAKOGL_PVR_POOL_OWNERS`,
/// docs/notes/pvr-render-pool.md): the frame sets with their
/// arenas' high-water marks, the persistent programs and tables, and the
/// depth/stencil buffers. Returns their KiB.
#[no_mangle]
pub extern "C" fn pvr_backend_pool_owners() -> u64 {
    use crate::osal::OwnerTally;
    // SAFETY: single-threaded by contract; taken once per entry point.
    let b = unsafe { &mut PVR.get().b };
    if !b.inited {
        crate::glog!("pool owner backend: not initialised");
        return 0;
    }
    /* the last frame's use stays in the cursors until the next frame starts */
    note_arena_hw(b);
    let mut sets = OwnerTally::default();
    for fs in &b.sets {
        for m in [&fs.pds.mem, &fs.gen.mem, &fs.vdm, &fs.dbias, &fs.scissor, &fs.pds_static, &fs.bg_color] {
            sets.add(m);
        }
    }
    let mut statics = OwnerTally::default();
    for m in [
        &b.clear_fs,
        &b.clear_fs2,
        &b.clear_fs3,
        &b.clear_fs4,
        &b.fbload_fs,
        &b.fbload_vs,
        &b.mipgen_fs,
        &b.fbload_quad,
        &b.black_tex,
        &b.seq_idx,
    ] {
        statics.add(m);
    }
    for m in b.fbload_fs_rt.iter().chain(b.fbload_fs_int.iter()).chain(b.clear_mask_fs.iter()) {
        statics.add(m);
    }
    let mut ds = OwnerTally::default();
    for s in &b.ds {
        ds.add(&s.mem);
    }
    /* the sample store ([`MsStore`]) beside the depth/stencil buffers: the
     * per-sample memory a multisampled frame keeps across a split */
    ds.add(&b.ms.mem);
    let kib =
        sets.report("backend frame sets") + statics.report("backend statics") + ds.report("backend depth/stencil");
    let size = |m: &OsalGpuMem| m.size as u32;
    crate::glog!(
        "pool owner backend frame arenas high-water: general {} of {} bytes, pds {} of {} bytes, vdm {} of {} bytes ({} sets)",
        b.arena_hw[0],
        size(&b.sets[0].gen.mem),
        b.arena_hw[1],
        size(&b.sets[0].pds.mem),
        b.arena_hw[2],
        size(&b.sets[0].vdm),
        b.nsets
    );
    kib
}

#[no_mangle]
pub extern "C" fn pvr_backend_kick_pending(kick_id: u32) -> bool {
    // SAFETY: single-threaded by contract; taken once per entry point.
    let b = unsafe { &PVR.get().b };

    kick_id != 0 && kick_id as i64 > b.completed_fence()
}

/// The PowerVR implementation of the backend contract (`crate::backend`).
#[no_mangle]
pub static yakogl_backend_pvr: YakoglBackend = YakoglBackend {
    name: c"PowerVR BXS (Rogue XE)".as_ptr(),
    /* the vertex fetch reads float32 only: the GL core converts the rest */
    native_attrib_formats: 0,
    /* the PBE writes an RGBA8888 target as R,G,B,A */
    rt_rgba8888_is_bgra: false,
    /* the TPU walks the chain itself from one base and one stride: the
     * packed layout is what it wants */
    mip_level_align: 0,
    /* the TPU reads a block format from the twiddled block grid, not rows */
    compressed_blocks_in_rows: false,
    /* a cube map and a 3D texture are sampled by the shader here (glsl2usc
     * rewrites them), so the backend only ever sees one 2D image */
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(pvr_backend_init),
    shutdown: Some(pvr_backend_shutdown),
    draw: Some(pvr_backend_draw),
    flush: Some(pvr_backend_flush),
    finish: Some(pvr_backend_finish),
    finish_previous: Some(pvr_backend_finish_previous),
    kick_id: Some(pvr_backend_kick_id),
    kick_pending: Some(pvr_backend_kick_pending),
    heap_base: Some(pvr_backend_heap_base),
    counters: Some(pvr_backend_counters),
    /* keeps nothing per texel storage: it samples the GL core's own */
    texture_released: None,
    /* depth/stencil stay in the GL core's hands across a swap */
    surface_swapped: Some(pvr_backend_surface_swapped),
    texture_uploaded: None,
    /* a 2D RGBA8 chain is made on the GPU, one quad per level, into the GL
     * core's own storage (`pvr_generate_mipmap`); every other chain stays the
     * GL core's CPU box filter */
    generate_mipmap: Some(pvr_generate_mipmap),
    return_mipmap_chain: Some(pvr_return_mipmap_chain),
    linear_row_align: 0,
    uniforms_host_only: false,
    ds_write: Some(pvr_backend_ds_write),
    stencil_resolve: Some(pvr_backend_stencil_resolve),
    /* the PDS fetches a per-instance array by the instance number: divisor 1 */
    max_instance_divisor: 1,
    /* the sync waits keep finish() until this backend waits for one kick */
    wait_kick: None,
    /* the texture unit reads the storage */
    samples_texel_copies: false,
    attachment_gone: Some(pvr_backend_attachment_gone),
    depth_resolve: Some(pvr_backend_depth_resolve),
    query_begin: Some(pvr_backend_query_begin),
    query_end: Some(pvr_backend_query_end),
    query_result: Some(pvr_backend_query_result),
};

/// Append attribute `a` (first element at `base`) to the vertex-fetch streams
/// `streams[..n]`, visited in vi_reg order: an attribute that continues the
/// previous stream (same rate and stride, the next bytes, the next registers)
/// widens its DMA instead of taking a stream of its own. Returns the new count.
fn add_stream(streams: &mut [PvrPdsVertexStream], n: usize, a: &GlesAttrib, base: u64, coalesce: bool) -> usize {
    if n > 0 && coalesce {
        let prev = &mut streams[n - 1];

        if prev.per_instance == a.per_instance as u32
            && prev.stride == a.stride
            && prev.address.wrapping_add(prev.dma_size as u64) == base
            && prev.dst_reg + prev.dma_size / 4 == a.vi_reg as u32
        {
            prev.dma_size += a.components as u32 * 4;
            return n;
        }
    }
    streams[n] = PvrPdsVertexStream {
        address: base,
        stride: a.stride,
        dma_size: a.components as u32 * 4,
        dst_reg: a.vi_reg as u32,
        per_instance: (a.per_instance != 0) as u32,
    };
    n + 1
}

/* ---- the words before csbgen (tests only) --------------------------------
 * Until docs/source-form.md step 3 these words were written by hand (shifts
 * and masks in this file) or taken from the DDK capture. The tests keep them
 * as literal pins: `csbgen_words_equal_the_hand_packed_ones` proves every
 * packer call above gives these bytes, and the older tests still decode the
 * words they check against them. */
#[cfg(test)]
const TEX_IMAGE_WORD0_BASE: u64 = 0x6000_0A64;
#[cfg(test)]
const TEX_IMAGE_WORD0_BGRA: u64 = 0x6000_8864;
#[cfg(test)]
const TEX_IMAGE_WORD1_BASE: u64 = 0x0800_0000_0000_0000;
#[cfg(test)]
const TEX_IMAGE_WORD1_COMPRESSED_TPU: u64 = 1 << 54;
#[cfg(test)]
const TEX_IMAGE_WORD0_RGBA16F: u64 = (28 << 27) | 0x0A64;
#[cfg(test)]
const TEX_IMAGE_WORD0_R16F: u64 = (11 << 27) | 0x2D84;
#[cfg(test)]
const TEX_IMAGE_WORD0_R32F: u64 = (18 << 27) | 0x2D84;
#[cfg(test)]
const TEX_IMAGE_WORD0_R32F_LUMINANCE: u64 = (18 << 27) | 0x0084;
#[cfg(test)]
const TEX_IMAGE_WORD0_ETC2_RGB: u64 = (68 << 27) | 0x0A81;
#[cfg(test)]
const TEX_IMAGE_WORD0_ETC2_RGBA: u64 = (69 << 27) | 0x0A61;
#[cfg(test)]
const TEX_IMAGE_WORD0_ETC2_PT: u64 = (70 << 27) | 0x0A61;
#[cfg(test)]
const TEX_IMAGE_WORD0_RGB565: u64 = (5 << 27) | 0x8884;
#[cfg(test)]
const TEX_IMAGE_WORD0_L8: u64 = 0x0084;
#[cfg(test)]
const TEX_IMAGE_WORD0_A8: u64 = 0x16D04;
#[cfg(test)]
const TEX_IMAGE_WORD0_LA8: u64 = (7 << 27) | 0x0024;
#[cfg(test)]
const TEX_SAMPLER_WORD0_LO: u32 = 0x0000_0FFF;
#[cfg(test)]
const TEX_SAMPLER_MAG_LINEAR_HI: u32 = 0x0000_0010;
#[cfg(test)]
const TEX_SAMPLER_MIN_LINEAR_HI: u32 = 0x0000_0040;
#[cfg(test)]
const TEX_SAMPLER_MIP_LINEAR_HI: u32 = 0x0000_0100;
#[cfg(test)]
const TEX_SAMPLER_WORD1_CEMEDGE_DONTFILTER_HI: u32 = 1 << 31;
#[cfg(test)]
const TEX_SAMPLER_ADDRMODE_U_SHIFT: u32 = 9;
#[cfg(test)]
const TEX_SAMPLER_ADDRMODE_V_SHIFT: u32 = 12;
#[cfg(test)]
const TEX_SAMPLER_ADDRMODE_W_SHIFT: u32 = 24;
#[cfg(test)]
const TEX_ADDRMODE_REPEAT: u32 = 0;
#[cfg(test)]
const TEX_ADDRMODE_FLIP: u32 = 1;
#[cfg(test)]
const TEX_ADDRMODE_CLAMP_TO_EDGE: u32 = 2;
#[cfg(test)]
const USC_PIXEL_OUTPUT_CTRL: u32 = 0x0007_fff9;
#[cfg(test)]
const ISP_CTL_DEFAULT: u32 = 0x8003_5000;
#[cfg(test)]
const ISP_CTL_MSAA4: u32 = 0x8003_1000;
#[cfg(test)]
const ISP_CTL_DBIAS_IS_INT: u32 = 1 << 20;
#[cfg(test)]
const ISP_BGOBJVALS_MASK: u32 = 0x0000_0100;
#[cfg(test)]
const ISP_STENCIL_BASE_SEPARATE: u64 = 1;
#[cfg(test)]
const TE_PSG_DEFAULT: u32 = 0x0002_0000;
#[cfg(test)]
const PPP_CTRL_PRETRANSFORM: u32 = 0x3;
#[cfg(test)]
const PPP_CTRL_CLIPSPACE: u32 = 0x203;
#[cfg(test)]
const PBE_SWIZZLE_RGBA: u64 = 0x6880_0000;
#[cfg(test)]
const PBE_SWIZZLE_BGRA: u64 = 0x60a0_0000;
#[cfg(test)]
const PBE_SWIZ_ONE: u64 = 4 << 29;
#[cfg(test)]
const PBE_TILERELATIVE: u64 = 1 << 57;
#[cfg(test)]
const PBE_DOWNSCALE: u64 = 1 << 56;
#[cfg(test)]
const EOT_EMIT_WORD1_FLAGS: u32 = 0x82000;
#[cfg(test)]
const ZLSCTL_FORCEZLOAD: u64 = 1 << 1;
#[cfg(test)]
const ZLSCTL_FORCEZSTORE: u64 = 1 << 2;
#[cfg(test)]
const ZLSCTL_SLOADEN: u64 = 1 << 14;
#[cfg(test)]
const ZLSCTL_ZLOADEN: u64 = 1 << 15;
#[cfg(test)]
const ZLSCTL_SSTOREEN: u64 = 1 << 18;
#[cfg(test)]
const ZLSCTL_ZSTOREEN: u64 = 1 << 19;

#[cfg(test)]
#[path = "backend_tests.rs"]
mod rig_tests;

#[cfg(test)]
#[path = "backend_csbgen_tests.rs"]
mod csbgen_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn attr(vi_reg: u8, components: u8, stride: u32, per_instance: u8) -> GlesAttrib {
        GlesAttrib { vi_reg, components, stride, per_instance, ..GlesAttrib::ZERO }
    }

    #[test]
    fn interleaved_attributes_share_a_stream_only_at_the_same_rate() {
        let mut st = [PvrPdsVertexStream::ZERO; 4];
        /* one buffer, one stride: pos (vi0..3) then colour (vi4..7) */
        let mut n = add_stream(&mut st, 0, &attr(0, 4, 40, 0), 0x1000, true);
        n = add_stream(&mut st, n, &attr(4, 4, 40, 0), 0x1010, true);
        assert_eq!(n, 1);
        assert_eq!((st[0].address, st[0].dma_size, st[0].dst_reg, st[0].per_instance), (0x1000, 32, 0, 0));
        /* per-instance offset + colour interleaved in the next bytes, next registers: a stream of their own
         * (indexed by the instance number), then coalesced with each other */
        n = add_stream(&mut st, n, &attr(8, 2, 40, 1), 0x1020, true);
        assert_eq!(n, 2, "a per-instance array never joins a per-vertex stream");
        n = add_stream(&mut st, n, &attr(10, 4, 40, 1), 0x1028, true);
        assert_eq!(n, 2);
        assert_eq!((st[1].address, st[1].dma_size, st[1].dst_reg, st[1].per_instance), (0x1020, 24, 8, 1));
        /* the reverse: a per-vertex array after a per-instance stream */
        n = add_stream(&mut st, n, &attr(14, 4, 40, 0), 0x1038, true);
        assert_eq!((n, st[2].per_instance), (3, 0));
        /* without coalescing every attribute is a stream */
        let n = add_stream(&mut st, 1, &attr(4, 4, 40, 0), 0x1010, false);
        assert_eq!((n, st[1].address, st[1].dst_reg), (2, 0x1010, 4));
    }

    #[test]
    fn a_target_with_no_alpha_takes_the_pbe_swizzle_that_writes_one() {
        /* REG_WORD0.swiz_chan3 [29:31] (pbestate.xml): SOURCE_CHAN3 = 3 is
         * the fragment's alpha, SWIZ.ONE = 4 is the constant 1. Only that
         * field moves -- the colour lanes stay where the format put them. */
        for fmt in [GLES_SURF_RGBA8888, GLES_SURF_XRGB8888, GLES_SURF_RGB565] {
            let with_alpha = pbe_reg_words(fmt, false, 64, 64, 64, false, false)[0];
            let no_alpha = pbe_reg_words(fmt, true, 64, 64, 64, false, false)[0];
            assert_eq!(with_alpha >> 29 & 0x7, 3, "fmt {fmt}: the shader's alpha");
            assert_eq!(no_alpha >> 29 & 0x7, 4, "fmt {fmt}: SWIZ.ONE");
            assert_eq!(with_alpha & !(0x7 << 29), no_alpha & !(0x7 << 29), "fmt {fmt}: colour lanes unchanged");
        }
    }

    #[test]
    fn id_register_fields_map_to_the_generator_argument() {
        assert_eq!(id_reg_of(0), PVR_PDS_NO_ID_REG);
        assert_eq!(id_reg_of(1), 0);
        assert_eq!(id_reg_of(5), 4);
        assert_eq!(id_reg_of(255), 254);
    }

    #[test]
    fn f16_bits_of_common_values() {
        assert_eq!(f32_to_f16_bits(0.0), 0);
        assert_eq!(f32_to_f16_bits(1.0), 0x3C00);
        assert_eq!(f32_to_f16_bits(-1.0), 0xBC00);
        assert_eq!(f32_to_f16_bits(0.5), 0x3800);
        assert_eq!(f32_to_f16_bits(1e9), 0x7C00);
        assert_eq!(f32_to_f16_bits(f32::NAN) & 0x7E00, 0x7E00);
        assert_eq!(f32_to_f16_bits(1e-30), 0);
        assert_eq!(f32_to_f16_bits(-(2f32.powi(-16))), 0x8100, "a negative subnormal");
    }

    #[test]
    fn clear_colour_of_a_float_target_is_packed_for_its_class() {
        let c = [1.0f32, 0.5, 0.0, 2.0];
        /* RGBA16F: two dwords of two halves each (r | g << 16, b | a << 16) */
        assert_eq!(pack_clear_float(GLES_SURF_RGBA16F, &c), [0x3C00 | (0x3800 << 16), 0x4000 << 16]);
        assert_eq!(pack_clear_float(GLES_SURF_R16F, &c), [0x3C00, 0]);
        assert_eq!(pack_clear_float(GLES_SURF_R32F, &[-0.25, 9.0, 9.0, 9.0]), [(-0.25f32).to_bits(), 0]);
        /* unclamped: a float target keeps values past [0, 1] */
        assert_eq!(pack_clear_float(GLES_SURF_R16F, &[-2.0, 0.0, 0.0, 0.0]), [0xC000, 0]);
    }

    #[test]
    fn the_clear_uniform_is_packed_for_the_target_and_not_for_the_first_channel() {
        /* The clear FS gets one dword for an ordinary target and it has to be
         * the RGBA8 pixel. pack_clear_float's fallthrough is right only for
         * R32F, where the red channel IS the pixel; sending it for an
         * RGBA8888 target wrote the float bits of red -- 0x3f800000 for
         * opaque red -- which is what a scissored clear put on the board. */
        let red = [1.0, 0.0, 0.0, 1.0];
        assert_eq!(clear_uniform(GLES_SURF_RGBA8888, &red), [pack_rgba8(&red), 0, 0, 0]);
        assert_ne!(clear_uniform(GLES_SURF_RGBA8888, &red)[0], 1.0f32.to_bits(), "not the red channel's float");
        for surf in [GLES_SURF_RGBA8888, GLES_SURF_XRGB8888, GLES_SURF_RGB565] {
            let c = [0.25, 0.5, 0.75, 1.0];
            assert_eq!(clear_uniform(surf, &c), [pack_rgba8(&c), 0, 0, 0], "surf {surf}");
        }
        /* the float classes keep their encoding */
        let c = [1.0, 0.5, 2.0, 0.0];
        assert_eq!(clear_uniform(GLES_SURF_RGBA16F, &c)[..2], pack_clear_float(GLES_SURF_RGBA16F, &c));
        assert_eq!(clear_uniform(GLES_SURF_R16F, &c)[..2], pack_clear_float(GLES_SURF_R16F, &c));
        assert_eq!(clear_uniform(GLES_SURF_R32F, &c), [1.0f32.to_bits(), 0, 0, 0], "R32F is the red channel");
    }

    /// An integer target's clear is its raw integers packed as Mesa's
    /// pvr_get_hw_clear_color packs them for its accumulation format
    /// (vulkan/pvr_formats.c:145-218): the low byte of each value side by
    /// side (UINT8/SINT8 :177-182), the low half-word (UINT16/SINT16
    /// :193-198), the value as it is (UINT32/SINT32 :210-215), one
    /// 10/10/10/2 dword for RGB10_A2UI (pvr_pack_a2x10y10z10_uint
    /// :103-121) -- the expected words written out by hand -- and the clear
    /// program moves that many dwords.
    #[test]
    fn an_integer_clear_is_the_raw_values_packed_for_the_target() {
        let v = [0x1234_5678u32, 0xfedc_ba98, 0x0000_0302, 0xffff_ffff];
        let c = v.map(f32::from_bits);
        for (surf, want, dwords) in [
            (GLES_SURF_R8UI, [0x78, 0, 0, 0], 1),
            (GLES_SURF_R8I, [0x78, 0, 0, 0], 1),
            (GLES_SURF_RG8UI, [0x9878, 0, 0, 0], 1),
            (GLES_SURF_RGBA8UI, [0xff02_9878, 0, 0, 0], 1),
            (GLES_SURF_RGBA8I, [0xff02_9878, 0, 0, 0], 1),
            (GLES_SURF_R16UI, [0x5678, 0, 0, 0], 1),
            (GLES_SURF_RG16I, [0xba98_5678, 0, 0, 0], 1),
            (GLES_SURF_RGBA16UI, [0xba98_5678, 0xffff_0302, 0, 0], 2),
            (GLES_SURF_R32I, [0x1234_5678, 0, 0, 0], 1),
            (GLES_SURF_RG32UI, [0x1234_5678, 0xfedc_ba98, 0, 0], 2),
            (GLES_SURF_RGBA32UI, v, 4),
            (GLES_SURF_RGBA32I, v, 4),
            /* 0x278 | 0x298 << 10 | 0x302 << 20 | 3 << 30 */
            (GLES_SURF_RGB10_A2UI, [0x278 | 0x298 << 10 | 0x302 << 20 | 3 << 30, 0, 0, 0], 1),
        ] {
            assert_eq!(clear_uniform(surf, &c), want, "{}", gles_surf_name(surf));
            assert_eq!(surf_out_dwords(surf), dwords, "{}", gles_surf_name(surf));
        }
        assert!(clears_by_quad(GLES_SURF_RGBA8UI) && clears_by_quad(GLES_SURF_R16F));
        assert!(!clears_by_quad(GLES_SURF_RGBA8888));
    }

    /// The PBE state of an integer target, field by field against Mesa
    /// 26.2.1: the pack mode of pvr_pbe_format_table
    /// (vulkan/pvr_arch_formats.c:195-266, the row of the layout's pipe
    /// format; texstate numbers from csbgen/rogue/pbestate.xml:60-123),
    /// `norm` 0 (pvr_vk_format_is_fully_normalized, pvr_formats.h:264-273),
    /// `source_format` 8_PER_CHANNEL (pvr_arch_job_common.c:51-53: any
    /// integer format), and the register swizzle the identity with no
    /// SWIZ.ONE for alpha whatever the target's alpha bits.
    #[test]
    fn integer_pbe_words_are_mesas() {
        /* (surface, pipe format, PACKMODE value) */
        let rows: [(u8, &str, u32); 19] = [
            (GLES_SURF_R8I, "R8_SINT", 48),
            (GLES_SURF_R8UI, "R8_UINT", 47),
            (GLES_SURF_R16I, "R16_SINT", 50),
            (GLES_SURF_R16UI, "R16_UINT", 49),
            (GLES_SURF_R32I, "R32_SINT", 53),
            (GLES_SURF_R32UI, "R32_UINT", 52),
            (GLES_SURF_RG8I, "R8G8_SINT", 36),
            (GLES_SURF_RG8UI, "R8G8_UINT", 35),
            (GLES_SURF_RG16I, "R16G16_SINT", 38),
            (GLES_SURF_RG16UI, "R16G16_UINT", 37),
            (GLES_SURF_RG32I, "R32G32_SINT", 41),
            (GLES_SURF_RG32UI, "R32G32_UINT", 40),
            (GLES_SURF_RGBA8I, "R8G8B8A8_SINT", 1),
            (GLES_SURF_RGBA8UI, "R8G8B8A8_UINT", 0),
            (GLES_SURF_RGBA16I, "R16G16B16A16_SINT", 9),
            (GLES_SURF_RGBA16UI, "R16G16B16A16_UINT", 8),
            (GLES_SURF_RGBA32I, "R32G32B32A32_SINT", 12),
            (GLES_SURF_RGBA32UI, "R32G32B32A32_UINT", 11),
            (GLES_SURF_RGB10_A2UI, "R10G10B10A2_UINT", 52),
        ];
        for (surf, pipe, packmode) in rows {
            let [_, w1] = pbe_emit_words(0x80_0000_1000, surf, 0, 0);
            let w1 = w1 as u64;
            /* pbestate.xml STATE_WORD1 (as a 32-bit word): packmode [7:12],
             * norm [13], source_pos [14:15], mrt_index [16:18],
             * source_format [19] */
            assert_eq!((w1 >> 7) & 0x3f, u64::from(packmode), "{pipe}: packmode");
            assert_eq!((w1 >> 13) & 1, 0, "{pipe}: norm");
            assert_eq!(
                (w1 >> 19) & 1,
                u64::from(csb::ROGUE_PBESTATE_SOURCE_FORMAT_8_PER_CHANNEL),
                "{pipe}: source_format"
            );
            for alpha_one in [false, true] {
                let w0 = pbe_reg_words(surf, alpha_one, 64, 64, 64, false, false)[0];
                let ident = pbe_reg_words(GLES_SURF_RGBA8888, false, 64, 64, 64, false, false)[0];
                assert_eq!(w0, ident, "{pipe}: the identity swizzle, alpha_one {alpha_one}");
            }
        }
    }

    /// An sRGB target's PBE state against Mesa 26.2.1: the pack mode of its
    /// FORMAT row, U8U8U8U8 (vulkan/pvr_arch_formats.c:213, PACKMODE 0 in
    /// csbgen/rogue/pbestate.xml:61), `norm` set (the format is fully
    /// normalised), `source_format` F16_PER_CHANNEL (0, pbestate.xml:156;
    /// pvr_arch_pbe_get_src_format_and_gamma, pvr_arch_job_common.c:56-60)
    /// and REG_WORD0's `gamma` (bit 4, pbestate.xml:268; set at
    /// pvr_arch_job_common.c:163-171) -- the RGBA8888 register words with
    /// that one bit, which no other surface sets. Its colour is two output
    /// dwords of halves; its programs are RGBA16F's; it clears by quad.
    #[test]
    fn srgb_pbe_words_are_mesas() {
        let [_, w1] = pbe_emit_words(0x80_0000_1000, GLES_SURF_SRGB8_A8, 2, 1);
        let w1 = w1 as u64;
        assert_eq!((w1 >> 7) & 0x3f, 0, "packmode U8U8U8U8");
        assert_eq!((w1 >> 13) & 1, 1, "norm");
        assert_eq!((w1 >> 19) & 1, u64::from(csb::ROGUE_PBESTATE_SOURCE_FORMAT_F16_PER_CHANNEL), "source_format");
        assert_eq!(csb::ROGUE_PBESTATE_SOURCE_FORMAT_F16_PER_CHANNEL, 0);
        /* the RGBA8888 state but for those fields */
        let [_, rgba] = pbe_emit_words(0x80_0000_1000, GLES_SURF_RGBA8888, 2, 1);
        assert_eq!(w1 & !(1 << 19), rgba as u64 & !(1 << 19), "only source_format differs from RGBA8888");
        for alpha_one in [false, true] {
            let srgb = pbe_reg_words(GLES_SURF_SRGB8_A8, alpha_one, 64, 64, 64, false, false);
            let rgba = pbe_reg_words(GLES_SURF_RGBA8888, alpha_one, 64, 64, 64, false, false);
            assert_eq!(srgb[0], rgba[0] | 1 << 4, "gamma, alpha_one {alpha_one}");
            assert_eq!(srgb[1], rgba[1], "REG_WORD1 alike");
        }
        let all = [GLES_SURF_RGBA8888, GLES_SURF_XRGB8888, GLES_SURF_RGB565, GLES_SURF_RGBA16F, GLES_SURF_R16F]
            .into_iter()
            .chain([GLES_SURF_R32F])
            .chain(GLES_SURF_INT_FIRST..=GLES_SURF_INT_LAST);
        for surf in all {
            let w0 = pbe_reg_words(surf, false, 64, 64, 64, false, false)[0];
            assert_eq!(w0 & 1 << 4, 0, "{}: no gamma", gles_surf_name(surf));
        }
        assert_eq!(surf_out_dwords(GLES_SURF_SRGB8_A8), 2);
        assert_eq!(pixel_class(GLES_SURF_SRGB8_A8), crate::shaderc::SHC_RT_F16X4);
        assert_eq!(pixel_class(GLES_SURF_RGBA16F), crate::shaderc::SHC_RT_F16X4);
        assert!(clears_by_quad(GLES_SURF_SRGB8_A8));
        assert_eq!(gles_surf_texfmt(GLES_SURF_SRGB8_A8), GLES_TEXFMT_SRGB8_A8, "the frame load decodes");
    }

    /// An sRGB target's clear is its linear colour as RGBA16F's two dwords
    /// of halves, clamped to [0, 1] first (unsigned normalised; NaN to 0);
    /// the PBE encodes it. The masked clear's float uniform is clamped alike.
    #[test]
    fn an_srgb_clear_is_the_clamped_linear_colour_in_halves() {
        let c = [1.5f32, 0.5, -1.0, f32::NAN];
        assert_eq!(clear_colour(GLES_SURF_SRGB8_A8, &c), [1.0, 0.5, 0.0, 0.0]);
        assert_eq!(clear_uniform(GLES_SURF_SRGB8_A8, &c), [0x3C00 | (0x3800 << 16), 0, 0, 0]);
        let g = [0.2f32, 0.2, 0.2, 1.0];
        assert_eq!(clear_uniform(GLES_SURF_SRGB8_A8, &g)[..2], pack_clear_float(GLES_SURF_RGBA16F, &g));
        /* a float target is not clamped */
        assert_eq!(clear_colour(GLES_SURF_RGBA16F, &c)[0], 1.5);
    }

    /// An sRGB texture's descriptor is the RGBA8 one with TEXSTATE_IMAGE_WORD0
    /// `gamma` on (bit 3, texstate.xml:287-288; Mesa's for a four-component
    /// sRGB format, pvr_arch_tex_state.c:207-226) and the pack info of
    /// setup_pck_info for R8G8B8A8_SRGB, which pco_pipe_to_pck_format leaves
    /// out: INVALID (0x1f) with UNORM (0x100). No other class sets the bit.
    #[test]
    fn an_srgb_texture_is_the_rgba8_descriptor_with_the_tpu_gamma() {
        let base = GlesTextureBind {
            base_gpu: 0x80_0000_1000,
            width: 3,
            height: 12,
            stride_texels: 4,
            mip_levels: 1,
            ..GlesTextureBind::ZERO
        };
        let srgb = desc(&GlesTextureBind { format: GLES_TEXFMT_SRGB8_A8, ..base });
        let rgba = desc(&GlesTextureBind { format: GLES_TEXFMT_RGBA8, ..base });
        assert_eq!(srgb[0], rgba[0] | 1 << 3, "word 0: gamma");
        assert_eq!(srgb[1..7], rgba[1..7], "the rest of the image words and the meta");
        assert_eq!(srgb[7], 0x11f, "PCK_INFO");
        assert_eq!(srgb[8..], rgba[8..], "the sampler words");
        for format in [
            GLES_TEXFMT_RGBA8,
            GLES_TEXFMT_RGBA16F,
            GLES_TEXFMT_R16F,
            GLES_TEXFMT_R32F,
            GLES_TEXFMT_L8,
            GLES_TEXFMT_RGBA8_SNORM,
            GLES_TEXFMT_RGBA8UI,
            GLES_TEXFMT_RGB10A2,
        ] {
            assert_eq!(desc(&GlesTextureBind { format, ..base })[0] & 1 << 3, 0, "format {format}: no gamma");
        }
    }

    /// The compressed sRGB classes are the ETC2 descriptors with the TPU
    /// gamma on (word 0 bit 3) and nothing else different: the same block
    /// format and the twiddled image word, for one level and for a chain.
    #[test]
    fn a_compressed_srgb_texture_is_the_etc2_descriptor_with_the_tpu_gamma() {
        for mip_levels in [1, 4] {
            let base = GlesTextureBind {
                base_gpu: 0x80_0000_1000,
                width: 8,
                height: 8,
                stride_texels: 8,
                mip_levels,
                mip_filter: u8::from(mip_levels > 1),
                ..GlesTextureBind::ZERO
            };
            for (srgb, linear) in
                [(GLES_TEXFMT_ETC2_SRGB_RGB, GLES_TEXFMT_ETC2_RGB), (GLES_TEXFMT_ETC2_SRGB_RGBA, GLES_TEXFMT_ETC2_RGBA)]
            {
                let s = desc(&GlesTextureBind { format: srgb, ..base });
                let l = desc(&GlesTextureBind { format: linear, ..base });
                assert_eq!(s[0], l[0] | 1 << 3, "format {srgb}, {mip_levels} level(s): word 0 gamma");
                assert_eq!(l[0] & 1 << 3, 0, "format {linear}: no gamma");
                assert_eq!(s[1..], l[1..], "format {srgb}: the twiddled word, the meta and the sampler");
            }
        }
    }

    #[test]
    fn rgba8_clear_packing_clamps_the_unclamped_gl_value() {
        assert_eq!(pack_rgba8(&[-1.0, 2.0, 0.5, 1.0]), 0xFF80_FF00);
    }

    fn desc(t: &GlesTextureBind) -> [u32; TEX_DESC_DWORDS as usize] {
        let mut w = [0u32; TEX_DESC_DWORDS as usize];
        build_tex_desc(&mut w, t, &mut 0);
        w
    }

    /// A shadow look-up reads its compare function from sampler meta word 0,
    /// descriptor dword 12 (select.rs shadow_compare): the bind's compare_op
    /// goes there, and nothing else writes it.
    /// Revert check: without the write, dword 12 stays 0 (NEVER) whatever
    /// the texture's function.
    #[test]
    fn the_compare_op_is_sampler_meta_word_0() {
        let t = GlesTextureBind {
            format: GLES_TEXFMT_R32F,
            type_: GLES_TEXTYPE_2D,
            base_gpu: HEAP_GENERAL_BASE + 0x2000,
            width: 4,
            height: 4,
            stride_texels: 4,
            bytes: 64,
            ..GlesTextureBind::ZERO
        };
        for op in 0..8u8 {
            assert_eq!(desc(&GlesTextureBind { compare_op: op, ..t })[12], op as u32);
        }
        assert_eq!(&desc(&GlesTextureBind { compare_op: 3, ..t })[13..16], &[0, 0, 0], "the other meta words stay 0");
    }

    /// Sampler meta word 1 (dword 13) bit 0 asks the shadow look-up for the
    /// compare per texel (select.rs shadow_compare's PCF arm): a
    /// LINEAR/LINEAR sampler over one level, and nothing else -- a mipmap
    /// filter over a chain, a NEAREST filter, or min and mag that differ
    /// keep the filtered depth's compare. A compare-less sampler gets the
    /// bit too: the shader's final select on bit 4 of word 0 wins.
    #[test]
    fn the_per_texel_compare_bit_is_sampler_meta_word_1_for_linear_over_one_level() {
        let t = GlesTextureBind {
            format: GLES_TEXFMT_R32F,
            type_: GLES_TEXTYPE_2D,
            base_gpu: HEAP_GENERAL_BASE + 0x2000,
            width: 32,
            height: 64,
            stride_texels: 32,
            bytes: 32 * 64 * 4,
            linear_filter: 1,
            min_linear: 1,
            compare_op: 1,
            ..GlesTextureBind::ZERO
        };
        assert_eq!(desc(&t)[13], 1, "LINEAR/LINEAR, one level");
        assert_eq!(desc(&GlesTextureBind { mip_levels: 6, ..t })[13], 1, "a chain the sampler does not walk");
        assert_eq!(desc(&GlesTextureBind { mip_filter: 1, mip_levels: 6, ..t })[13], 0, "a mipmap filter: no");
        assert_eq!(desc(&GlesTextureBind { mip_filter: 2, mip_levels: 6, ..t })[13], 0, "trilinear: no");
        assert_eq!(
            desc(&GlesTextureBind { mip_filter: 2, mip_levels: 1, ..t })[13],
            1,
            "a mipmap filter over one level"
        );
        assert_eq!(desc(&GlesTextureBind { linear_filter: 0, ..t })[13], 0, "NEAREST mag: no");
        assert_eq!(desc(&GlesTextureBind { min_linear: 0, ..t })[13], 0, "NEAREST min: no");
        assert_eq!(desc(&GlesTextureBind { compare_op: 16, ..t })[13], 1, "no compare: the bit stands, bit 4 wins");
        assert_eq!(desc(&GlesTextureBind { format: GLES_TEXFMT_RGBA8, ..t })[13], 1, "any format: the sampler decides");
    }

    /// An ETC texture's IMAGE_WORD1 is the TWIDDLED word (texstate.xml
    /// IMAGE_WORD1): the level count at [0:3], mipmaps_present at [15],
    /// baselevel 0 at [60:63]. The stride word's count at [60:63] was
    /// written into it until 2026-09-24: base level 1 of a chain of none.
    /// Every level the GL core laid out is walked, NPOT sizes included (the
    /// layout is the TPU's, texture.rs level_block_grid).
    #[test]
    fn an_etc_chain_is_described_in_the_twiddled_word() {
        let t = GlesTextureBind {
            format: GLES_TEXFMT_ETC2_RGB,
            type_: GLES_TEXTYPE_2D,
            base_gpu: HEAP_GENERAL_BASE + 0x2000,
            width: 16,
            height: 8,
            bytes: 128,
            mip_filter: 1,
            mip_levels: 4,
            ..GlesTextureBind::ZERO
        };
        let word1 = |t: &GlesTextureBind| {
            let w = desc(t);
            (w[2] as u64 | (w[3] as u64) << 32, w)
        };
        let (iw1, w) = word1(&t);
        assert_eq!(iw1 & 0xf, 4, "num_mip_levels at [0:3]");
        assert_eq!((iw1 >> 15) & 1, 1, "mipmaps_present");
        assert_eq!(iw1 >> 60, 0, "baselevel 0, not the stride word's count");
        assert_eq!((iw1 >> 54) & 7, 1, "compression mode TPU");
        assert_eq!((iw1 >> 16) & 0x3F_FFFF_FFFF, (HEAP_GENERAL_BASE + 0x2000) >> 2);
        assert_eq!(desc_maxlod(&w), 3 * 64);
        let (iw1, _) = word1(&GlesTextureBind { mip_filter: 0, ..t });
        assert_eq!((iw1 & 0xf, (iw1 >> 15) & 1, iw1 >> 60), (1, 0, 0), "one level, base 0");
        /* a NPOT chain too: the GL core lays it out as the TPU walks it
         * (texture.rs the_twiddled_etc_chain_is_laid_out_as_the_tpu_walks_it) */
        let (iw1, _) = word1(&GlesTextureBind { width: 12, ..t });
        assert_eq!((iw1 & 0xf, iw1 >> 60), (4, 0), "a NPOT chain walks its levels");
    }

    /// An ETC cube's copy (gl::native_cube lays the block grids face-major)
    /// is described as TEXTYPE_CUBE in the ETC format's word 0, with the
    /// compressed IMAGE_WORD1 mode (TPU decode) and no alpha_msb, the chain
    /// and six faces as for an uncompressed cube. Step C's default sent ETC1
    /// cubes to this path while it excluded them (AM62P, 15550645).
    #[test]
    fn an_etc_cube_copy_is_a_compressed_textype_cube() {
        let t = GlesTextureBind {
            format: GLES_TEXFMT_ETC2_RGB,
            type_: GLES_TEXTYPE_CUBE_TWIDDLED_PROBE,
            layers: 6,
            base_gpu: HEAP_GENERAL_BASE + 0x3000,
            width: 8,
            height: 8,
            stride_texels: 8,
            bytes: 4096 * 6,
            mip_filter: 1,
            mip_levels: 4,
            ..GlesTextureBind::ZERO
        };
        let w = desc(&t);
        let iw0 = w[0] as u64 | (w[1] as u64) << 32;
        let iw1 = w[2] as u64 | (w[3] as u64) << 32;
        assert_eq!(iw0 & 7, 3, "TEXTYPE_CUBE");
        assert_eq!(iw0 & !7 & 0xffff_ffff, TEX_IMAGE_WORD0_ETC2_RGB & !7 & 0xffff_ffff, "the ETC2 RGB format bits");
        assert_eq!((iw1 >> 54) & 7, 1, "compression mode TPU");
        assert_eq!((iw1 >> 59) & 1, 0, "no alpha_msb");
        assert_eq!(((iw1 >> 4) & 0x7ff, iw1 & 0xf), (5, 4), "six faces, the 8x8 chain's four levels");
        assert_eq!(w[4], 4096, "one face");
    }

    #[test]
    fn the_cube_probe_descriptor_is_a_twiddled_textype_cube() {
        let t = GlesTextureBind {
            format: GLES_TEXFMT_RGBA8,
            type_: GLES_TEXTYPE_CUBE_TWIDDLED_PROBE,
            layers: 6,
            base_gpu: HEAP_GENERAL_BASE + 0x1000,
            width: 8,
            height: 8,
            stride_texels: 8,
            bytes: 4096 * 6,
            mip_filter: 1,
            mip_levels: 4,
            ..GlesTextureBind::ZERO
        };
        let w = desc(&t);
        let iw0 = w[0] as u64 | (w[1] as u64) << 32;
        let iw1 = w[2] as u64 | (w[3] as u64) << 32;
        assert_eq!(iw0 & 7, 3, "TEXTYPE_CUBE");
        assert_eq!(iw0 & !7 & 0xffff_ffff, TEX_IMAGE_WORD0_BASE & !7, "format and swizzle as a stride RGBA8");
        assert_eq!(((iw0 >> 34) & 0x3fff, (iw0 >> 48) & 0x3fff), (7, 7), "one face's size");
        assert_eq!(iw1 & 0xf, 4, "num_mip_levels at [0:3] in the twiddled word");
        assert_eq!((iw1 >> 4) & 0x7ff, 5, "depth = six layers - 1");
        assert_eq!((iw1 >> 15) & 1, 1, "mipmaps_present");
        assert_eq!((iw1 >> 16) & 0x3F_FFFF_FFFF, (HEAP_GENERAL_BASE + 0x1000) >> 2);
        assert_eq!(iw1 >> 60, 0, "baselevel 0, not the stride word's level count");
        assert_eq!((iw1 >> 59) & 1, 1, "alpha_msb as for RGBA8");
        assert_eq!(w[4], 4096, "the meta layer size is one face (its full chain, 4 KiB aligned)");
        assert_eq!(desc_maxlod(&w), 3 * 64);
        /* single-level sampling (no mipmap filter): a one-level copy
         * (gl::native_cube::copy's `single`), described as one level with
         * no mipmaps_present, so the TPU's face stride is level 0's and the
         * sampler cannot leave it (3b634e98's native arm read level 1 of a
         * declared chain: white) */
        let w1 = desc(&GlesTextureBind { mip_filter: 0, bytes: 4096 * 6, ..t });
        let iw1 = w1[2] as u64 | (w1[3] as u64) << 32;
        assert_eq!((iw1 & 0xf, (iw1 >> 15) & 1), (1, 0), "one level, no mipmaps_present");
        assert_eq!(w1[4], 4096, "one face: level 0, 4 KiB aligned");
        assert_eq!(w1[11] >> 31, u32::from(crate::REPORT_ES2), "non-seamless exactly in an ES 2.0 build");
        /* a mipmap filter over one written level is one level too */
        let wl = desc(&GlesTextureBind { mip_levels: 1, ..t });
        assert_eq!((wl[2] as u64) & 0xf, 1, "a one-level chain");
        /* a 6x6 face with a chain is copied into its 8x8 power-of-two chain */
        let w6 = desc(&GlesTextureBind { width: 6, height: 6, ..t });
        assert_eq!((w6[2] as u64) & 0xf, 4, "8, 4, 2, 1");
        /* the stacked cube of every other path is untouched: a stride 2D */
        let w = desc(&GlesTextureBind { type_: GLES_TEXTYPE_CUBE, height: 48, ..t });
        assert_eq!(w[0] & 7, 4, "TEXTYPE_STRIDE");
        assert_eq!(w[11], 0, "no cube sampler bits on a 2D image");
        /* a float cube keeps its format's word 0 and pack info: Qt Quick
         * 3D's light probe is RGBA16F (gl::native_cube copies any linear
         * format) */
        let f = GlesTextureBind { format: GLES_TEXFMT_RGBA16F, bytes: 8192 * 6, ..t };
        let w = desc(&f);
        let iw0 = w[0] as u64 | (w[1] as u64) << 32;
        let iw1 = w[2] as u64 | (w[3] as u64) << 32;
        assert_eq!(iw0 & 7, 3, "TEXTYPE_CUBE");
        assert_eq!(iw0 & !7 & 0xffff_ffff, TEX_IMAGE_WORD0_RGBA16F & !7 & 0xffff_ffff, "the RGBA16F format bits");
        assert_eq!(((iw1 >> 4) & 0x7ff, iw1 & 0xf), (5, 4), "the twiddled word: six faces, four levels");
        assert_eq!(w[7], TEX_PCK_INFO_F16, "halves back as halves");
        assert_eq!(w[4], 8192, "one face");
        /* a compressed cube is a native cube too since the ETC1 cube fix
         * (an_etc_cube_copy_is_a_compressed_textype_cube) */
        let e = desc(&GlesTextureBind { format: GLES_TEXFMT_ETC2_RGB, ..t });
        assert_eq!(e[0] & 7, 3, "TEXTYPE_CUBE for a compressed cube");
    }

    /// The native 3D texture's descriptor (gl::native_vol, pvr_native_3d;
    /// docs/notes/pvr-3d-native.md): TEXTYPE_3D of level 0's size, the
    /// depth less one in the twiddled image word, the levels the copy holds
    /// (one when no mipmap filter walks them), and the third axis's address
    /// mode from WRAP_R -- the field the grid hard-wired to CLAMP_TO_EDGE
    /// because its depth was the shader's.
    #[test]
    fn a_3d_descriptor_is_a_twiddled_textype_3d_with_the_depth_and_the_third_wrap() {
        let base = HEAP_GENERAL_BASE + 0x9000;
        let t = GlesTextureBind {
            format: GLES_TEXFMT_RGBA8,
            type_: GLES_TEXTYPE_3D_TWIDDLED,
            layers: 5,
            base_gpu: base,
            width: 3,
            height: 7,
            stride_texels: 3,
            bytes: 4096,
            mip_filter: 2,
            mip_levels: 4,
            linear_filter: 1,
            min_linear: 1,
            wrap_s: GLES_WRAP_REPEAT,
            wrap_t: GLES_WRAP_MIRRORED_REPEAT,
            wrap_r: GLES_WRAP_REPEAT,
            ..GlesTextureBind::ZERO
        };
        let w = desc(&t);
        let iw0 = w[0] as u64 | (w[1] as u64) << 32;
        let iw1 = w[2] as u64 | (w[3] as u64) << 32;
        assert_eq!(iw0 & 7, 2, "TEXTYPE_3D, not STRIDE");
        assert_eq!(iw0 & !7 & 0xffff_ffff, TEX_IMAGE_WORD0_BASE & !7, "format and swizzle as a stride RGBA8");
        assert_eq!(((iw0 >> 34) & 0x3fff, (iw0 >> 48) & 0x3fff), (2, 6), "level 0's API size, not the power of two");
        assert_eq!(iw1 & 0xf, 4, "the levels the copy holds");
        assert_eq!((iw1 >> 4) & 0x7ff, 4, "depth = 5 - 1");
        assert_eq!((iw1 >> 15) & 1, 1, "mipmaps_present");
        assert_eq!((iw1 >> 16) & 0x3F_FFFF_FFFF, base >> 2, "texaddr");
        assert_eq!((iw1 >> 59) & 1, 1, "alpha_msb as for RGBA8");
        assert_eq!(iw1 >> 60, 0, "baselevel 0");
        assert_eq!(w[7], TEX_PCK_INFO_RGBA8_UNORM);
        assert_eq!(desc_maxlod(&w), 3 * 64, "the sampler walks the written levels");
        let mode = |w: &[u32; TEX_DESC_DWORDS as usize], shift: u32| (w[9] >> shift) & 7;
        assert_eq!(mode(&w, TEX_SAMPLER_ADDRMODE_U_SHIFT), TEX_ADDRMODE_REPEAT);
        assert_eq!(mode(&w, TEX_SAMPLER_ADDRMODE_V_SHIFT), TEX_ADDRMODE_FLIP);
        assert_eq!(mode(&w, TEX_SAMPLER_ADDRMODE_W_SHIFT), TEX_ADDRMODE_REPEAT, "REPEAT on r reaches the TPU");
        let c = desc(&GlesTextureBind { wrap_r: GLES_WRAP_CLAMP_TO_EDGE, ..t });
        assert_eq!(mode(&c, TEX_SAMPLER_ADDRMODE_W_SHIFT), TEX_ADDRMODE_CLAMP_TO_EDGE);
        /* one level (no mipmap filter): one level, no mipmaps_present */
        let w1 = desc(&GlesTextureBind { mip_filter: 0, mip_levels: 1, ..t });
        let iw1 = w1[2] as u64 | (w1[3] as u64) << 32;
        assert_eq!((iw1 & 0xf, (iw1 >> 15) & 1, (iw1 >> 4) & 0x7ff), (1, 0, 4));
        /* the depth's 11 bits */
        for n in [1u16, 2, 256, 2048] {
            let wn = desc(&GlesTextureBind { layers: n, ..t });
            assert_eq!((wn[2] >> 4) & 0x7ff, u32::from(n) - 1, "{n} slices");
        }
        /* every other path keeps its third axis clamped: a stride 2D */
        let s = desc(&GlesTextureBind { type_: GLES_TEXTYPE_2D, ..t });
        assert_eq!(mode(&s, TEX_SAMPLER_ADDRMODE_W_SHIFT), TEX_ADDRMODE_CLAMP_TO_EDGE);
    }

    /// The native 2D array's descriptor (gl::native_array, pvr_native_array;
    /// docs/es3/pvr-2darray.md section 10, test 1): a twiddled TEXTYPE_2D of
    /// one layer's size, the layer count less one in the image word's
    /// `depth`, the chain's level count, one layer's bytes in meta dword 4
    /// and the base address in meta dwords 5 and 6 (low first). The stacked
    /// array of every other path is untouched: a stride 2D.
    #[test]
    fn an_array_descriptor_is_a_twiddled_textype_2d_with_the_layer_count() {
        let base = HEAP_GENERAL_BASE + 0x5000;
        let t = GlesTextureBind {
            format: GLES_TEXFMT_RGBA8,
            type_: GLES_TEXTYPE_2D_ARRAY_TWIDDLED,
            layers: 7,
            base_gpu: base,
            width: 8,
            height: 4,
            stride_texels: 8,
            bytes: 4096 * 7,
            mip_filter: 1,
            mip_levels: 4,
            ..GlesTextureBind::ZERO
        };
        let w = desc(&t);
        let iw0 = w[0] as u64 | (w[1] as u64) << 32;
        let iw1 = w[2] as u64 | (w[3] as u64) << 32;
        assert_eq!(iw0 & 7, 1, "TEXTYPE_2D (twiddled), not STRIDE");
        assert_eq!(iw0 & !7 & 0xffff_ffff, TEX_IMAGE_WORD0_BASE & !7, "format and swizzle as a stride RGBA8");
        assert_eq!(((iw0 >> 34) & 0x3fff, (iw0 >> 48) & 0x3fff), (7, 3), "one layer's size, not the stack's");
        assert_eq!(iw1 & 0xf, 4, "the 8x4 chain's four levels (8x4, 4x2, 2x1, 1x1) at [0:3]");
        assert_eq!((iw1 >> 4) & 0x7ff, 6, "depth = layers - 1");
        assert_eq!((iw1 >> 15) & 1, 1, "mipmaps_present");
        assert_eq!((iw1 >> 16) & 0x3F_FFFF_FFFF, base >> 2, "texaddr");
        assert_eq!((iw1 >> 59) & 1, 1, "alpha_msb as for RGBA8");
        assert_eq!(iw1 >> 60, 0, "baselevel 0, not the stride word's level count");
        assert_eq!(w[4], 4096, "meta dword 4: one layer's bytes (a 4 KiB multiple)");
        assert_eq!((w[5], w[6]), (base as u32, 0x80), "meta dwords 5, 6: the base, low dword first");
        assert_eq!(w[7], TEX_PCK_INFO_RGBA8_UNORM);
        assert_eq!(desc_maxlod(&w), 3 * 64, "the sampler walks the written levels");
        assert_eq!(w[11], 0, "no cube sampler bits");
        /* one level sampled (no mipmap filter): a one-level copy, described
         * as one level with no mipmaps_present */
        let w1 = desc(&GlesTextureBind { mip_filter: 0, ..t });
        let iw1 = w1[2] as u64 | (w1[3] as u64) << 32;
        assert_eq!((iw1 & 0xf, (iw1 >> 15) & 1, (iw1 >> 4) & 0x7ff), (1, 0, 6), "one level, seven layers");
        /* a 5x3 layer with a chain is copied into its 8x4 power-of-two
         * chain: four levels, the API size in word 0 */
        let w5 = desc(&GlesTextureBind { width: 5, height: 3, ..t });
        let iw0 = w5[0] as u64 | (w5[1] as u64) << 32;
        assert_eq!(((iw0 >> 34) & 0x3fff, (iw0 >> 48) & 0x3fff, w5[2] as u64 & 0xf), (4, 2, 4));
        /* the layer count's 11 bits: 1, 2, 256 and 2048 layers */
        for n in [1u16, 2, 256, 2048] {
            let wn = desc(&GlesTextureBind { layers: n, bytes: 4096 * u32::from(n), ..t });
            assert_eq!((wn[2] >> 4) & 0x7ff, u32::from(n) - 1, "{n} layers");
            assert_eq!(wn[4], 4096, "{n} layers: one layer's bytes");
        }
        /* an ETC array: the compressed word's mode (TPU decode), no
         * alpha_msb, TEXTYPE_2D as the format's own */
        let e = desc(&GlesTextureBind { format: GLES_TEXFMT_ETC2_RGB, ..t });
        let iw1 = e[2] as u64 | (e[3] as u64) << 32;
        assert_eq!(e[0] & 7, 1, "TEXTYPE_2D");
        assert_eq!(((iw1 >> 54) & 7, (iw1 >> 59) & 1, (iw1 >> 4) & 0x7ff, iw1 & 0xf), (1, 0, 6, 4));
        assert_eq!((e[5], e[6]), (base as u32, 0x80));
        /* a float array keeps its format's word 0 and pack info */
        let f = desc(&GlesTextureBind { format: GLES_TEXFMT_R32F, bytes: 4096 * 7, ..t });
        let iw0 = f[0] as u64 | (f[1] as u64) << 32;
        assert_eq!(iw0 & !7 & 0xffff_ffff, TEX_IMAGE_WORD0_R32F & !7 & 0xffff_ffff, "the R32F format bits");
        assert_eq!(f[7], TEX_PCK_INFO_F32);
        /* the stacked array of every other path is untouched: a stride 2D
         * of the whole stack's height */
        let s = desc(&GlesTextureBind { type_: GLES_TEXTYPE_2D_ARRAY, height: 28, ..t });
        assert_eq!(s[0] & 7, 4, "TEXTYPE_STRIDE");
        assert_eq!(((s[0] as u64 | (s[1] as u64) << 32) >> 48) & 0x3fff, 27, "the stack's height");
        assert_eq!((s[5], s[6]), (0, 0), "no base in the meta words");
        assert_eq!(texture_descriptor(&t), w, "the host tests' entry point is the same descriptor");
    }

    #[test]
    fn float_texture_descriptors_use_the_float_formats_and_pack_info() {
        let base = GlesTextureBind {
            base_gpu: 0x80_0000_1000,
            width: 3,
            height: 12,
            stride_texels: 4,
            mip_levels: 1,
            ..GlesTextureBind::ZERO
        };
        for (format, word0, bytes, pck) in [
            (GLES_TEXFMT_RGBA16F, TEX_IMAGE_WORD0_RGBA16F, 8u32, TEX_PCK_INFO_F16),
            (GLES_TEXFMT_R16F, TEX_IMAGE_WORD0_R16F, 2, TEX_PCK_INFO_F16),
            (GLES_TEXFMT_R32F, TEX_IMAGE_WORD0_R32F, 4, TEX_PCK_INFO_F32),
        ] {
            let w = desc(&GlesTextureBind { format, ..base });
            let iw0 = word0 | (2u64 << 34) | (11u64 << 48);
            assert_eq!((w[0], w[1]), (iw0 as u32, (iw0 >> 32) as u32), "format {format}");
            /* STRIDE texture: the row pitch minus one in IMAGE_WORD1 */
            let iw1 = TEX_IMAGE_WORD1_BASE | 3 | (1u64 << 60) | ((0x80_0000_1000u64 >> 2) << 16);
            assert_eq!((w[2], w[3]), (iw1 as u32, (iw1 >> 32) as u32), "format {format}");
            assert_eq!(w[4], 4 * 12 * bytes, "layer bytes, format {format}");
            assert_eq!(w[7], pck, "format {format}");
        }
        /* the texstate FORMAT field (bits 27..32) */
        assert_eq!(TEX_IMAGE_WORD0_RGBA16F >> 27, 28);
        assert_eq!(TEX_IMAGE_WORD0_R16F >> 27, 11);
        assert_eq!(TEX_IMAGE_WORD0_R32F >> 27, 18);
        assert_eq!(TEX_PCK_INFO_F16, 0x2e);
    }

    /// A depth texture under GL_OES_depth_texture (`depth_luminance`) is the
    /// same R32F look-up with the luminance swizzle, r,r,r,1 (L8's, which
    /// the board samples as (L, L, L, 1)); nothing else in the word changes.
    #[test]
    fn a_luminance_depth_texture_swizzles_its_red_into_green_and_blue() {
        let base = GlesTextureBind {
            base_gpu: 0x80_0000_1000,
            width: 3,
            height: 12,
            stride_texels: 4,
            mip_levels: 1,
            format: GLES_TEXFMT_R32F,
            ..GlesTextureBind::ZERO
        };
        let red = desc(&base);
        let lum = desc(&GlesTextureBind { depth_luminance: 1, ..base });
        let swiz = |w: u64| (((w >> 14) & 7), ((w >> 11) & 7), ((w >> 8) & 7), ((w >> 5) & 7));
        assert_eq!(swiz(TEX_IMAGE_WORD0_R32F), (0, 5, 5, 4), "(r, 0, 0, 1)");
        assert_eq!(swiz(TEX_IMAGE_WORD0_R32F_LUMINANCE), (0, 0, 0, 4), "(r, r, r, 1)");
        assert_eq!(TEX_IMAGE_WORD0_R32F_LUMINANCE & !0x7fe0, TEX_IMAGE_WORD0_R32F & !0x7fe0, "only the swizzle");
        let iw0 = TEX_IMAGE_WORD0_R32F_LUMINANCE | (2u64 << 34) | (11u64 << 48);
        assert_eq!((lum[0], lum[1]), (iw0 as u32, (iw0 >> 32) as u32));
        assert_eq!(lum[2..], red[2..], "the rest of the descriptor is the R32F one");
    }

    /// A depth texture read from its depth buffer (`depth_buffer_binding`,
    /// `PVR_TEXFMT_DEPTH_BUFFER`) is Mesa's depth-aspect view of a D24S8
    /// image: FORMAT X8U24 (texstate.xml: 21) as a STRIDE image, (r, 0, 0,
    /// 1), or the luminance (r, r, r, 1) under GL_OES_depth_texture, no
    /// `alpha_msb` (Z24X8 has none), one level whatever the sampler's mip
    /// filter asks (the buffer holds level 0 alone), the row stride the
    /// binding carries (the ZLS-tile-aligned width), the layer size that
    /// stride times the height in words, and Mesa's pack info for a format
    /// pco has no pack format for: "no pack format" with UNORM (0x11f). The
    /// sampler words are the texture's own.
    #[test]
    fn a_depth_buffer_binding_is_mesas_x8u24_stride_image() {
        let t = GlesTextureBind {
            format: PVR_TEXFMT_DEPTH_BUFFER,
            base_gpu: 0x80_0000_2000,
            width: 1600,
            height: 1200,
            stride_texels: 1600, /* a 32-aligned width stays itself */
            mip_filter: 2,
            mip_levels: 1,
            linear_filter: 0,
            wrap_s: GLES_WRAP_CLAMP_TO_EDGE,
            wrap_t: GLES_WRAP_CLAMP_TO_EDGE,
            ..GlesTextureBind::ZERO
        };
        let red = desc(&t);
        let lum = desc(&GlesTextureBind { depth_luminance: 1, ..t });
        let swiz = |w: u64| (((w >> 14) & 7), ((w >> 11) & 7), ((w >> 8) & 7), ((w >> 5) & 7));
        let (w0_red, w0_lum) = (red[0] as u64 | (red[1] as u64) << 32, lum[0] as u64 | (lum[1] as u64) << 32);
        assert_eq!((w0_red >> 27) & 0x3f, 21, "FORMAT X8U24");
        assert_eq!(w0_red & 7, 4, "TEXTYPE STRIDE");
        assert_eq!(swiz(w0_red), (0, 5, 5, 4), "(r, 0, 0, 1)");
        assert_eq!(swiz(w0_lum), (0, 0, 0, 4), "(r, r, r, 1)");
        assert_eq!(w0_lum & !0x7fe0, w0_red & !0x7fe0, "only the swizzle");
        assert_eq!(((w0_red >> 34) & 0x3fff, (w0_red >> 48) & 0x3fff), (1599, 1199), "the API extent");
        let stride = tex_stride_image_word1(1600, 1, 0x80_0000_2000, false);
        assert_eq!(red[2..4], stride, "one level at the buffer's stride and address, no alpha_msb");
        assert_eq!(lum[2..], red[2..], "the rest of the descriptor is the same");
        assert_eq!(red[4], 1600 * 1200 * 4, "the layer's bytes");
        assert_eq!(red[7], 0x11f, "Mesa's pack info for Z24X8_UNORM");
        /* an aligned stride wider than the texture: the ISP's rows */
        let wide = desc(&GlesTextureBind { width: 1000, height: 700, stride_texels: 1024, ..t });
        assert_eq!(wide[2..4], tex_stride_image_word1(1024, 1, 0x80_0000_2000, false));
        assert_eq!(wide[4], 1024 * 700 * 4);
    }

    /// The ES 3.0 storage classes' descriptors against Mesa 26.2.1, field by
    /// field, the expected values written down from Mesa's own tables rather
    /// than from `es3_tex_format`: the TPU format is the
    /// `pvr_format_table` row of the class's pipe format
    /// (vulkan/pvr_arch_formats.c:102-151) as a texstate.xml FORMAT number
    /// (csbgen/rogue/texstate.xml:96-193); the swizzle is the pipe format's
    /// util swizzle through `pvr_get_hw_swizzle` (pvr_arch_tex_state.c:40-64,
    /// SRCCHAN_n = n, SRC_ONE = 4); `alpha_msb` is
    /// `vk_format_is_alpha_on_msb` (vulkan/util/vk_format.h:214-226, used at
    /// pvr_arch_tex_state.c:253); the pack info is `setup_pck_info`
    /// (pvr_arch_tex_state.c:84-112) over `pco_pipe_to_pck_format`
    /// (pco/pco.c:387-461) with pco_isa.py's numbers (u8888 0, s8888 1,
    /// u1010102 8, f111110 12) and pvr_iface.h's bits (SCALE 0x40, UNORM
    /// 0x100, INVALID 0x1f).
    #[test]
    fn es3_class_descriptors_are_mesas() {
        let base = GlesTextureBind {
            base_gpu: 0x80_0000_1000,
            width: 3,
            height: 12,
            stride_texels: 4,
            mip_levels: 1,
            ..GlesTextureBind::ZERO
        };
        const ID: [u64; 4] = [0, 1, 2, 3];
        const XYZ1: [u64; 4] = [0, 1, 2, 4];
        /* (class, pipe format, FORMAT, swizzle, alpha_msb, bytes, pack info) */
        type Row = (u8, &'static str, u64, [u64; 4], bool, u32, u32);
        let rows: [Row; 13] = [
            (GLES_TEXFMT_R11G11B10F, "R11G11B10_FLOAT", 88, XYZ1, false, 4, 12),
            (GLES_TEXFMT_RGB9E5, "R9G9B9E5_FLOAT", 26, XYZ1, false, 4, 0x1f),
            (GLES_TEXFMT_RGB10A2, "R10G10B10A2_UNORM", 14, ID, true, 4, 8 | 0x40 | 0x100),
            (GLES_TEXFMT_RGB10A2UI, "R10G10B10A2_UINT", 14, ID, true, 4, 0x1f),
            (GLES_TEXFMT_RGBA8_SNORM, "R8G8B8A8_SNORM", 13, ID, true, 4, 1 | 0x40),
            (GLES_TEXFMT_RGBA8UI, "R8G8B8A8_UINT", 12, ID, true, 4, 0x1f),
            (GLES_TEXFMT_RGBA8I, "R8G8B8A8_SINT", 13, ID, true, 4, 0x1f),
            (GLES_TEXFMT_RGBA16UI, "R16G16B16A16_UINT", 29, ID, true, 8, 0x1f),
            (GLES_TEXFMT_RGBA16I, "R16G16B16A16_SINT", 30, ID, true, 8, 0x1f),
            (GLES_TEXFMT_RGBA32UI, "R32G32B32A32_UINT", 62, ID, true, 16, 0x1f),
            (GLES_TEXFMT_RGBA32I, "R32G32B32A32_SINT", 63, ID, true, 16, 0x1f),
            (GLES_TEXFMT_RGBA32F, "R32G32B32A32_FLOAT", 61, ID, true, 16, 0x1f),
            /* the base classes keep their words: one of them, as a guard */
            (GLES_TEXFMT_RGBA8, "R8G8B8A8_UNORM", 12, ID, true, 4, 0x140),
        ];
        for (format, pipe, texformat, swiz, alpha_msb, bytes, pck) in rows {
            let w = desc(&GlesTextureBind { format, ..base });
            let iw0 = w[0] as u64 | (w[1] as u64) << 32;
            let iw1 = w[2] as u64 | (w[3] as u64) << 32;
            assert_eq!(iw0 & 7, 4, "{pipe}: TEXTYPE_STRIDE");
            assert_eq!((iw0 >> 27) & 0x7f, texformat, "{pipe}: FORMAT");
            let got = [(iw0 >> 14) & 7, (iw0 >> 11) & 7, (iw0 >> 8) & 7, (iw0 >> 5) & 7];
            assert_eq!(got, swiz, "{pipe}: swiz0..3");
            assert_eq!(((iw0 >> 34) & 0x3fff, (iw0 >> 48) & 0x3fff), (2, 11), "{pipe}: width, height less one");
            assert_eq!(
                iw0 & !(0x3fff << 34 | 0x3fff << 48 | 0x7f << 27 | 0xfff << 5 | 7),
                0,
                "{pipe}: nothing else in word 0"
            );
            assert_eq!((iw1 >> 59) & 1 != 0, alpha_msb, "{pipe}: alpha_msb");
            let want1 = (u64::from(alpha_msb) << 59) | 3 | (1u64 << 60) | ((0x80_0000_1000u64 >> 2) << 16);
            assert_eq!(iw1, want1, "{pipe}: the STRIDE word");
            assert_eq!(w[4], 4 * 12 * bytes, "{pipe}: layer bytes");
            assert_eq!(gles_texfmt_bytes(format), bytes, "{pipe}: the class's texel");
            assert_eq!(w[7], pck, "{pipe}: PCK_INFO");
        }
    }

    /// An integer cube, 3D texture and 2D array that a program reads through
    /// the stacked 2D image (a STRIDE textype of the whole stack's height,
    /// the extra axis addressed by the shader) have the integer class's 2D
    /// descriptor over that image: the class's TPU FORMAT (Mesa
    /// `pvr_format_table`, as `es3_class_descriptors_are_mesas`), PCK_INFO
    /// INVALID (the `smp` has no `fcnorm`, so the TPU hands back raw bits),
    /// the stack's height less one in word 0, the row pitch less one in word
    /// 1 and the stack's bytes in meta dword 4; no base in the meta words
    /// (those are the twiddled native targets').
    #[test]
    fn integer_stacked_cube_3d_and_array_descriptors_are_the_class_over_the_stack() {
        /* (target, texel width, stack height: six 4-texel faces, a 3-slice
         * grid of 4 x 4 slices, seven 4-texel layers) */
        let targets = [(GLES_TEXTYPE_CUBE, 4u32, 24u32), (GLES_TEXTYPE_3D, 12, 4), (GLES_TEXTYPE_2D_ARRAY, 4, 28)];
        /* (class, FORMAT, bytes per texel) */
        let classes = [
            (GLES_TEXFMT_RGBA8I, 13u64, 4u32),
            (GLES_TEXFMT_RGBA8UI, 12, 4),
            (GLES_TEXFMT_RGBA16I, 30, 8),
            (GLES_TEXFMT_RGBA16UI, 29, 8),
            (GLES_TEXFMT_RGBA32I, 63, 16),
            (GLES_TEXFMT_RGBA32UI, 62, 16),
        ];
        for (type_, width, height) in targets {
            for (format, texformat, bytes) in classes {
                let t = GlesTextureBind {
                    base_gpu: 0x80_0000_1000,
                    type_,
                    format,
                    width,
                    height,
                    stride_texels: width,
                    mip_levels: 1,
                    ..GlesTextureBind::ZERO
                };
                let w = desc(&t);
                let iw0 = w[0] as u64 | (w[1] as u64) << 32;
                let iw1 = w[2] as u64 | (w[3] as u64) << 32;
                let what = format!("target {type_} format {format}");
                assert_eq!(iw0 & 7, 4, "{what}: TEXTYPE_STRIDE");
                assert_eq!((iw0 >> 27) & 0x7f, texformat, "{what}: FORMAT");
                assert_eq!(
                    ((iw0 >> 34) & 0x3fff, (iw0 >> 48) & 0x3fff),
                    (u64::from(width) - 1, u64::from(height) - 1),
                    "{what}: the stack's size less one"
                );
                assert_eq!((iw1 >> 59) & 1, 1, "{what}: alpha_msb");
                assert_eq!(iw1 & 0xffff, u64::from(width) - 1, "{what}: the row pitch less one");
                assert_eq!(w[4], width * height * bytes, "{what}: the stack's bytes");
                assert_eq!(w[7], PVR_PCK_FORMAT_INVALID, "{what}: PCK_INFO invalid, raw integers");
                assert_eq!((w[5], w[6]), (0, 0), "{what}: no base in the meta words");
            }
        }
    }

    #[test]
    fn narrow_luminance_alpha_descriptors_take_u8_formats_swizzles_and_their_pack_info() {
        let base = GlesTextureBind {
            base_gpu: 0x80_0000_1000,
            width: 3,
            height: 12,
            stride_texels: 4,
            mip_levels: 1,
            ..GlesTextureBind::ZERO
        };
        /* pack info 0x140, the default; 0x11f is a test-knobs arm (mode 2) */
        let modes: &[(i32, u32)] = if cfg!(feature = "test-knobs") {
            &[(1, TEX_PCK_INFO_RGBA8_UNORM), (2, TEX_PCK_INFO_COMPRESSED_UNORM)]
        } else {
            &[(1, TEX_PCK_INFO_RGBA8_UNORM)]
        };
        for &(mode, pck) in modes {
            #[cfg(feature = "test-knobs")]
            let was = pvr_debug_set_narrow_la(mode);
            for (format, word0, bytes) in [
                (GLES_TEXFMT_L8, TEX_IMAGE_WORD0_L8, 1u32),
                (GLES_TEXFMT_A8, TEX_IMAGE_WORD0_A8, 1),
                (GLES_TEXFMT_LA8, TEX_IMAGE_WORD0_LA8, 2),
            ] {
                let w = desc(&GlesTextureBind { format, ..base });
                let iw0 = word0 | (2u64 << 34) | (11u64 << 48);
                assert_eq!((w[0], w[1]), (iw0 as u32, (iw0 >> 32) as u32), "format {format}");
                let iw1 = TEX_IMAGE_WORD1_BASE | 3 | (1u64 << 60) | ((0x80_0000_1000u64 >> 2) << 16);
                assert_eq!((w[2], w[3]), (iw1 as u32, (iw1 >> 32) as u32), "format {format}");
                assert_eq!(w[4], 4 * 12 * bytes, "layer bytes, format {format}");
                assert_eq!(w[7], pck, "format {format} mode {mode}");
            }
            #[cfg(feature = "test-knobs")]
            pvr_debug_set_narrow_la(was);
        }
        /* texstate.xml FORMAT (bits 27..33): U8 = 0, U8U8 = 7; STRIDE textype 4;
         * swizzles (14..16, 11..13, 8..10, 5..7), SRCCHAN_n = n, ONE 4, ZERO 5 */
        let swiz = |w: u64| (((w >> 14) & 7), ((w >> 11) & 7), ((w >> 8) & 7), ((w >> 5) & 7));
        assert_eq!((TEX_IMAGE_WORD0_L8 >> 27, TEX_IMAGE_WORD0_A8 >> 27, TEX_IMAGE_WORD0_LA8 >> 27), (0, 0, 7));
        assert_eq!(swiz(TEX_IMAGE_WORD0_L8), (0, 0, 0, 4), "L8 reads (L, L, L, 1)");
        assert_eq!(swiz(TEX_IMAGE_WORD0_A8), (5, 5, 5, 0), "A8 reads (0, 0, 0, A)");
        assert_eq!(swiz(TEX_IMAGE_WORD0_LA8), (0, 0, 0, 1), "LA8 reads (L, L, L, A)");
        for w0 in [TEX_IMAGE_WORD0_L8, TEX_IMAGE_WORD0_A8, TEX_IMAGE_WORD0_LA8] {
            assert_eq!(w0 & 7, 4, "STRIDE");
        }
    }

    #[test]
    fn a_float_texture_keeps_its_mip_chain() {
        let t = GlesTextureBind {
            format: GLES_TEXFMT_RGBA16F,
            width: 4,
            height: 4,
            stride_texels: 4,
            mip_levels: 3,
            mip_filter: 2,
            ..GlesTextureBind::ZERO
        };
        let w = desc(&t);
        let iw1 = ((w[3] as u64) << 32) | w[2] as u64;
        assert_eq!(iw1 >> 60, 3);
        assert_ne!(iw1 & (1 << 15), 0, "mipmaps_present");
        assert_ne!(w[9] & TEX_SAMPLER_MIP_LINEAR_HI, 0);
    }

    /// The u4.6 maxlod of a descriptor: sampler word0 [23:31] plus word1 [0].
    fn desc_maxlod(w: &[u32; TEX_DESC_DWORDS as usize]) -> u32 {
        ((w[8] >> 23) & 0x1ff) | ((w[9] & 1) << 9)
    }

    #[test]
    fn the_lod_range_stops_at_the_last_level_of_the_chain() {
        /* levels the core says are filled, not the hardware maximum: a
         * sampler allowed past them reads memory nobody wrote */
        let base = GlesTextureBind {
            format: GLES_TEXFMT_RGBA8,
            width: 64,
            height: 64,
            stride_texels: 64,
            mip_filter: 2,
            ..GlesTextureBind::ZERO
        };
        for (levels, maxlod) in [(2u8, 64u32), (3, 128), (7, 384)] {
            let w = desc(&GlesTextureBind { mip_levels: levels, ..base });

            assert_eq!(desc_maxlod(&w), maxlod, "{levels} level(s)");
            assert_eq!(((w[3] as u64) << 32 | w[2] as u64) >> 60, levels as u64);
        }
        /* a chain longer than the hardware's CLAMP MAX is clamped to it */
        assert_eq!(desc_maxlod(&desc(&GlesTextureBind { mip_levels: 16, ..base })), TEX_SAMPLER_MAXLOD_MAX);
        /* one level: no LOD range at all */
        assert_eq!(desc_maxlod(&desc(&GlesTextureBind { mip_levels: 1, ..base })), 0);
    }

    /// glmark2 terrain's 800x600 RGBA8 chain, as the GL core binds it (gl::
    /// draw a_generated_npot_chain_is_complete_with_repeat: ten levels,
    /// LINEAR_MIPMAP_LINEAR, stride 1024): laid out as the 1024x1024 chain
    /// (gl::texture pot_chain_layout, Mesa pvr's physical extent), it is
    /// described as Mesa's pvr_arch_tex_state.c describes a linear image:
    /// - TEXSTATE_IMAGE_WORD0 (texstate.xml:272-291): textype STRIDE (4),
    ///   texformat U8U8U8U8 (12) at [27:33], the API extent minus one,
    ///   width 799 at [34:47] and height 599 at [48:61];
    /// - TEXSTATE_STRIDE_IMAGE_WORD1 (texstate.xml:315-329): stride 1023 at
    ///   [0:14] (the physical width, Mesa's `u_minify(physical_extent.width,
    ///   base_level)`), mipmaps_present at [15], texaddr at [16:53],
    ///   alpha_msb at [59], num_mip_levels 10 at [60:63];
    /// - the sampler's maxlod 9 levels (u4.6: 576).
    ///
    /// The TPU finds level l at the offset Mesa's pvr_image_plane_setup_mip_
    /// levels gives it from that stride and the power-of-two height; the GL
    /// core put it there (gl::texture the_powervr_800x600_chain_has_mesas_
    /// offsets).
    #[test]
    fn an_800x600_npot_chain_is_a_stride_texture_of_the_power_of_two_pitch() {
        let base_gpu = HEAP_GENERAL_BASE + 0x40_0000;
        let t = GlesTextureBind {
            format: GLES_TEXFMT_RGBA8,
            type_: GLES_TEXTYPE_2D,
            layers: 1,
            base_gpu,
            width: 800,
            height: 600,
            stride_texels: 1024,
            bytes: 1024 * 600 * 4,
            linear_filter: 1,
            min_linear: 1,
            mip_filter: 2,
            mip_levels: 10,
            ..GlesTextureBind::ZERO
        };
        let w = desc(&t);
        let iw0 = w[0] as u64 | (w[1] as u64) << 32;
        let iw1 = w[2] as u64 | (w[3] as u64) << 32;
        assert_eq!(iw0 & 7, 4, "textype STRIDE");
        assert_eq!((iw0 >> 27) & 0x7f, 12, "texformat U8U8U8U8");
        assert_eq!(((iw0 >> 34) & 0x3fff, (iw0 >> 48) & 0x3fff), (799, 599), "the API extent, minus one");
        assert_eq!(iw0, 0x0257_0C7C_6000_0A64, "IMAGE_WORD0");
        assert_eq!(iw1 & 0x7fff, 1023, "stride: the power-of-two width, minus one");
        assert_eq!((iw1 >> 15) & 1, 1, "mipmaps_present");
        assert_eq!((iw1 >> 16) & 0x3F_FFFF_FFFF, base_gpu >> 2, "texaddr: level 0");
        assert_eq!((iw1 >> 59) & 1, 1, "alpha_msb");
        assert_eq!(iw1 >> 60, 10, "num_mip_levels: GL's ten");
        assert_eq!(iw1, 0xA800_0000_0000_83FF | (base_gpu >> 2) << 16, "STRIDE_IMAGE_WORD1");
        assert_eq!(desc_maxlod(&w), 9 * 64, "the LOD range stops at level 9 (1x1)");
        /* the same chain without a mipmap filter samples level 0 alone, at
         * the same stride */
        let w = desc(&GlesTextureBind { mip_filter: 0, ..t });
        let iw1 = w[2] as u64 | (w[3] as u64) << 32;
        assert_eq!((iw1 & 0x7fff, (iw1 >> 15) & 1, iw1 >> 60), (1023, 0, 1));
    }

    #[test]
    fn a_single_level_texture_whose_filters_differ_can_still_minify() {
        /* The TPU picks the min or mag filter on the CLAMPED LOD (Mesa
         * pvr_arch_sampler.c:88-97); with maxlod 0 it always magnified, so
         * GL_TEXTURE_MIN_FILTER was ignored (dEQP filtering.2d linear_nearest_*
         * and nearest_linear_*, AM62P). One level, filters differing: LOD up
         * to 1.0, and the image still has one level (the texel is level 0's). */
        let base = GlesTextureBind {
            format: GLES_TEXFMT_RGBA8,
            width: 64,
            height: 64,
            stride_texels: 64,
            mip_levels: 1,
            ..GlesTextureBind::ZERO
        };
        for (min, mag, want) in [(1u8, 0u8, 64u32), (0, 1, 64), (1, 1, 0), (0, 0, 0)] {
            let w = desc(&GlesTextureBind { min_linear: min, linear_filter: mag, ..base });
            assert_eq!(desc_maxlod(&w), want, "min_linear {min} linear_filter {mag}");
            assert_eq!(((w[3] as u64) << 32 | w[2] as u64) >> 60, 1, "one level in the image");
        }
    }

    #[test]
    fn s_and_t_wrap_are_programmed_independently() {
        let base = GlesTextureBind {
            format: GLES_TEXFMT_RGBA8,
            width: 4,
            height: 4,
            stride_texels: 4,
            mip_levels: 1,
            ..GlesTextureBind::ZERO
        };
        /* ADDRMODE is three bits per axis: REPEAT 0, FLIP 1 (GL's mirrored
         * repeat), CLAMP_TO_EDGE 2. W is never sampled and always clamps. */
        let mask = (7 << TEX_SAMPLER_ADDRMODE_U_SHIFT)
            | (7 << TEX_SAMPLER_ADDRMODE_V_SHIFT)
            | (7 << TEX_SAMPLER_ADDRMODE_W_SHIFT);
        let want = |u: u32, v: u32| {
            (u << TEX_SAMPLER_ADDRMODE_U_SHIFT)
                | (v << TEX_SAMPLER_ADDRMODE_V_SHIFT)
                | (TEX_ADDRMODE_CLAMP_TO_EDGE << TEX_SAMPLER_ADDRMODE_W_SHIFT)
        };

        for (ws, wt, u, v) in [
            (GLES_WRAP_REPEAT, GLES_WRAP_REPEAT, TEX_ADDRMODE_REPEAT, TEX_ADDRMODE_REPEAT),
            (GLES_WRAP_CLAMP_TO_EDGE, GLES_WRAP_REPEAT, TEX_ADDRMODE_CLAMP_TO_EDGE, TEX_ADDRMODE_REPEAT),
            (GLES_WRAP_REPEAT, GLES_WRAP_CLAMP_TO_EDGE, TEX_ADDRMODE_REPEAT, TEX_ADDRMODE_CLAMP_TO_EDGE),
            (GLES_WRAP_CLAMP_TO_EDGE, GLES_WRAP_CLAMP_TO_EDGE, TEX_ADDRMODE_CLAMP_TO_EDGE, TEX_ADDRMODE_CLAMP_TO_EDGE),
            (GLES_WRAP_MIRRORED_REPEAT, GLES_WRAP_REPEAT, TEX_ADDRMODE_FLIP, TEX_ADDRMODE_REPEAT),
            (GLES_WRAP_REPEAT, GLES_WRAP_MIRRORED_REPEAT, TEX_ADDRMODE_REPEAT, TEX_ADDRMODE_FLIP),
            (GLES_WRAP_MIRRORED_REPEAT, GLES_WRAP_CLAMP_TO_EDGE, TEX_ADDRMODE_FLIP, TEX_ADDRMODE_CLAMP_TO_EDGE),
        ] {
            let t = GlesTextureBind { wrap_s: ws, wrap_t: wt, ..base };
            assert_eq!(desc(&t)[9] & mask, want(u, v), "wrap {ws}/{wt}");
        }
        /* the bits the descriptor used to be written with, by hand */
        assert_eq!(TEX_ADDRMODE_CLAMP_TO_EDGE << TEX_SAMPLER_ADDRMODE_U_SHIFT, 0x0000_0400);
        assert_eq!(TEX_ADDRMODE_CLAMP_TO_EDGE << TEX_SAMPLER_ADDRMODE_V_SHIFT, 0x0000_2000);
        assert_eq!(TEX_ADDRMODE_CLAMP_TO_EDGE << TEX_SAMPLER_ADDRMODE_W_SHIFT, 0x0200_0000);
    }
}
