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

use super::blobs::*;
use super::csbgen as csb;
use super::ffi::*;
use crate::backend::*;
use crate::ds_retention::{self, Parked, Plane, DEPTH, STENCIL};
use crate::osal::render::*;
use crate::osal::{self, OsalGpuMem, OSAL_MEM_DATA, OSAL_QUERY_RENDER_STATIC};
use crate::shaderc::{self, ShcShader, SHC_FS_META_DEFAULT, SHC_NO_META, SHC_NO_PSIZE, SHC_UNIFORM_NO_BLOCK};
use crate::util::{self, Bytes, Global, EBUSY, EINVAL, EIO, ENODEV, ENOMEM, ENOSPC, ETIMEDOUT};

/* ---- bring-up options (enum pvr_backend_option; defaults: all 0) ---------- */
pub type PvrBackendOption = u32;
/// This backend draws into a framebuffer with no colour attachment
/// (`backend::DEPTH_ONLY_TARGETS`): a draw with `target_gpu` 0 and a depth or
/// stencil attachment opens a frame whose kick is a z-only render
/// ([`flush`]: one end-of-tile emit with an empty-tile PBE state that
/// stores nothing, and `CR_ISP_ZLSCTL.zonlyrender`), as Mesa's pvr renders
/// a pass with no colour attachment. The depth/stencil buffer is stored as
/// for any frame.
pub const DRAWS_WITHOUT_COLOUR: bool = true;

/// 1: vertex positions are already in screen space (x,y pixels, z, w=1) and
/// the TA must not apply the viewport transform (PPP pretransform). Used to
/// reproduce the silicon test rig through the GL path.
pub const PVR_BACKEND_OPT_PRETRANSFORM: PvrBackendOption = 0;
/// The TA clip mode: [`PVR_DEPTH_CLIP_FRONT_REAR`],
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
/// CR_PPP_CTRL value override (0 = the default for the current mode).
pub const PVR_BACKEND_OPT_PPP_CTRL: PvrBackendOption = 2;
/// 1: never merge adjacent attributes into one DMA stream (one DDMAD per
/// attribute); 0: merge attributes that are contiguous in memory and in their
/// vi registers (fewer DMAs, the layout the test rig used).
pub const PVR_BACKEND_OPT_NO_COALESCE: PvrBackendOption = 3;
/// 1: log every generated PPP/VDM/PDS word (bring-up comparison).
pub const PVR_BACKEND_OPT_DEBUG: PvrBackendOption = 4;
/// Raw TA_STATE_TERMINATE0 override for the trailing PPP block (0 = computed
/// from the target size). Bring-up bisection knob.
pub const PVR_BACKEND_OPT_TERM0: PvrBackendOption = 5;
/// RETIRED: was NO_SUBMIT, "build the frame but do not submit it", the A/B
/// hand-off to the self-test rig, which went with pvrgpu.ko
/// (driver/tools/pvrlab/README.md). The slot keeps its number so every later
/// option keeps its own (ffi_layout_check.c pins them); setting it changes
/// nothing.
pub const PVR_BACKEND_OPT_RETIRED_NO_SUBMIT: PvrBackendOption = 6;
/// 1: after every kick, log a summary of the target's stored depth/stencil
/// buffer (bring-up aid for the ISP Z load/store path).
pub const PVR_BACKEND_OPT_DS_DUMP: PvrBackendOption = 7;
/// Primitives one kick may tile before the frame is split (0 = the built-in
/// budget). A test can set it low to exercise the split on a workload small
/// enough for any parameter buffer.
pub const PVR_BACKEND_OPT_PRIM_BUDGET: PvrBackendOption = 8;
/// Bring-up: the step between one colour attachment's source position in the
/// shader's output registers and the next (0 = the format's own register
/// count).
pub const PVR_BACKEND_OPT_MRT_STRIDE: PvrBackendOption = 9;
/// 1: sample every texture from level 0, whatever chain it has (the sampler's
/// level count is forced to one). An A/B for a texture that renders flat
/// where it should show a minified pattern.
pub const PVR_BACKEND_OPT_FORCE_MIP0: PvrBackendOption = 10;
/// ISP_CTL.dbias_is_int, the depth-bias constant's units (glPolygonOffset):
/// the option's number for C clients. The value lives in [`pvr_dbias_int`],
/// which carries the default; setting this option writes it.
///
/// 1 (the default since the AM62P A/B, RESULTS 1i): set the bit when the
/// frame has a depth/stencil buffer, so the ISP takes the constant as the
/// 24-bit integer count the encoder writes (Mesa
/// `pvr_calculate_final_depth_bias_contant_factor`).
///
/// 0: leave the bit clear, which is what the driver did until then (an OR in
/// the depth/stencil arm was overwritten by the plain assignment that
/// follows). `gl_test` case 3 passes either way, so a cleared bit is not
/// simply "the constant vanishes"; the four dEQP
/// `polygon_offset.*_with_units` cases fail with it clear.
pub const PVR_BACKEND_OPT_DBIAS_INT: PvrBackendOption = 11;
/// PROBE ONLY: 1 = colour target 0 is written TWIDDLE_2D instead of linear
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
/// (driver/docs/pvr-fence-wait.md, "The next experiment"). It changes only
/// that wait's target; glFinish, the frame-set reuse and every kick are
/// untouched. Off by default (0: wait for all but the most recent kick).
pub const PVR_BACKEND_OPT_ONE_IN_FLIGHT: PvrBackendOption = 13;
/// 1 = the swap drops the window's depth/stencil (EGL 1.5 3.10.1: they are
/// undefined after `eglSwapBuffers`). When the frame being swapped renders
/// into the swapped surface, its kick does not store depth/stencil (the ZLS
/// store bits off) and the buffer is marked not valid with its planes
/// undefined, so the next frame initialises it instead of loading it. Only
/// that kick changes. The V3D backend does the same (`v3d_surface_swapped`).
/// The hook is called for window surfaces only (`egl.rs`: a pbuffer swap
/// "has no effect"). ON by default ([`PVR_SWAP_DROPS_ZS_DEFAULT`]): measured
/// on the AM62P (RESULTS.md, "the swap drops the window's depth/stencil":
/// glmark2 315 -> 550, gl_test 37/38 in both arms). The value lives in
/// [`pvr_swap_drops_zs`]; setting this option writes it.
pub const PVR_BACKEND_OPT_SWAP_DROPS_ZS: PvrBackendOption = 14;
/// MEASUREMENT: the fragment kick declares at least this many temporaries
/// per instance (`max(fs.temps, value)`, capped at [`FS_TEMPS_MIN_CAP`]); 0
/// (the default) declares the shader's own count. The program is unchanged:
/// only the per-instance allocation the PDS kick asks the USC for grows, so
/// a run with it set separates the cost of a larger temporary allocation
/// (fewer resident instances) from the cost of the instructions. Never a
/// default: it exists for the online-vs-table A/B of the FemtoVG programs
/// (the online ones declare about twice the table's temporaries).
pub const PVR_BACKEND_OPT_FS_TEMPS_MIN: PvrBackendOption = 15;
pub const PVR_BACKEND_OPT_COUNT: usize = 16;
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
/// The default arenas, measured (driver/docs/pvr-render-pool.md, the
/// frame-set run): glmark2's largest frame on the AM62P used 325896 bytes of
/// PDS and 516768 of GENERAL. 768 KiB and 1024 KiB are 2.4 and 2.0 times
/// that, so its frames are not split. With 4 sets that is 7488 KiB of the
/// render pool, where 4 x (4 MiB + 1 MiB) took 20800 KiB.
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
/// pool (driver/docs/pvr-render-pool.md, the frame sets are 20.3 MiB of
/// the fixed use).
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

/// Tiles in flight for a frame of `samples`, as Mesa's
/// `pvr_arch_setup_tiles_in_flight` (vulkan/pvr_arch_job_common.c) computes
/// it for the BXS-4-64 (common/device_info/bxs-4-64.h: isp_samples_per_pixel
/// 4, 16x16 tiles, max_partitions 16, isp_max_tiles_in_flight 6, one raster
/// pipe, simple_parameter_format_version 2, paired tiles not requested) with
/// the pixel width at `usc_min_output_registers_per_pix` (2). That reproduces
/// both captured values -- PIPE(5) at one sample, PIPE(3) at 4x -- and is
/// the one thing taken differently from Mesa, which derives this and
/// CR_USC_PIXEL_OUTPUT_CTRL.width from the same pixel width: at the four
/// registers the output control says, Mesa would allow two tiles at 4x.
fn isp_tiles_in_flight(samples: u32) -> u32 {
    const PIXEL_WIDTH: u32 = 2; /* usc_min_output_registers_per_pix */
    const TILE_DWORDS: u32 = 16 * 16; /* tile_size_x * tile_size_y */
    const ISP_MAX_TILES_IN_FLIGHT: u32 = 6;
    let partition = PIXEL_WIDTH * samples.clamp(1, 4) * TILE_DWORDS;
    let usable = RESERVED_PARTITION_DWORDS.min(partition * BXS_MAX_PARTITIONS);
    let partitions_available = BXS_MAX_PARTITIONS.min(usable / partition);
    /* one phantom (num_raster_pipes): the USC's tiles are the partitions,
     * the ISP's at most isp_max_tiles_in_flight of them (version 2 caps the
     * total by the partitions again, which changes nothing here) */
    ISP_MAX_TILES_IN_FLIGHT.min(partitions_available)
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
/// (`pvr_arch_setup_tiles_in_flight`). 0x8003_5000 at one sample, 0x8003_3000
/// at 4x: the values the DDK capture had.
fn isp_ctl(samples: u32, dbias_is_int: bool) -> u32 {
    csb_pack!(
        ROGUE_CR_ISP_CTL_pack,
        1,
        csb::ROGUE_CR_ISP_CTL {
            skip_init_hdrs: true,
            process_empty_tiles: true,
            sample_pos: true,
            dbias_is_int,
            pipe_enable: isp_tiles_in_flight(samples) - 1,
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
/// will read ([`PVR_BACKEND_OPT_SWAP_DROPS_ZS`]).
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
/// Depth/stencil buffers kept at once: a window surface and every framebuffer
/// object with a depth or stencil attachment wants one, and a deferred
/// renderer (Qt Quick 3D's userpasses) has several passes live at a time.
/// A buffer that holds contents is never given to another target: past this
/// many, a draw is refused (`ds_make_room`, src/ds_retention.rs).
const PVR_BACKEND_MAX_DS: usize = 8;
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
    if alpha_one {
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
/// targets only) stores the target twiddled, with its log2 size.
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
/// the variant compiled for that class.
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

fn pbe_emit_words(addr: u64, fmt: u8, source_pos: u32, mrt_index: u32) -> [u32; 2] {
    let float = gles_surf_rt_class(fmt) != 0;
    let packmode = match fmt {
        GLES_SURF_RGBA16F => csb::ROGUE_PBESTATE_PACKMODE_F16F16F16F16,
        GLES_SURF_R16F => csb::ROGUE_PBESTATE_PACKMODE_F16,
        _ if float => csb::ROGUE_PBESTATE_PACKMODE_F32,
        GLES_SURF_RGB565 => csb::ROGUE_PBESTATE_PACKMODE_R5G6B5,
        _ => csb::ROGUE_PBESTATE_PACKMODE_U8U8U8U8,
    };
    let w0 =
        csb_pack!(ROGUE_PBESTATE_STATE_WORD0_pack, 1, csb::ROGUE_PBESTATE_STATE_WORD0 { address_low: dev_addr(addr) });
    let w1 = csb_pack!(
        ROGUE_PBESTATE_STATE_WORD1_pack,
        1,
        csb::ROGUE_PBESTATE_STATE_WORD1 {
            address_high: dev_addr(addr),
            source_format: if float {
                csb::ROGUE_PBESTATE_SOURCE_FORMAT_F16_PER_CHANNEL
            } else {
                csb::ROGUE_PBESTATE_SOURCE_FORMAT_8_PER_CHANNEL
            },
            norm: !float,
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
    TexFormat { texformat, compressed: false, swiz, textype: csb::ROGUE_TEXSTATE_TEXTYPE_STRIDE }
}

/// U8U8U8U8, identity swizzle.
const TEXF_RGBA8: TexFormat = tex_stride(csb::ROGUE_TEXSTATE_FORMAT_U8U8U8U8, SWIZ_RGBA);
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
    TexFormat { texformat, compressed: true, swiz, textype: csb::ROGUE_TEXSTATE_TEXTYPE_2D }
}
const TEXF_ETC2_RGB: TexFormat =
    tex_etc(csb::ROGUE_TEXSTATE_FORMAT_COMPRESSED_ETC2_RGB, [SWIZ_R, SWIZ_G, SWIZ_B, SWIZ_ONE]);
const TEXF_ETC2_RGBA: TexFormat = tex_etc(csb::ROGUE_TEXSTATE_FORMAT_COMPRESSED_ETC2A_RGBA, SWIZ_RGBA);
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
            ..Default::default()
        }
    )
}

/// TEXSTATE_STRIDE_IMAGE_WORD1 of a linear image: `alpha_msb`, the row
/// stride in texels minus one, `levels` mip levels (num_mip_levels, and
/// mipmaps_present when more than one), the image address.
fn tex_stride_image_word1(stride_texels: u32, levels: u32, base_gpu: u64) -> [u32; 2] {
    csb_pack!(
        ROGUE_TEXSTATE_STRIDE_IMAGE_WORD1_pack,
        2,
        csb::ROGUE_TEXSTATE_STRIDE_IMAGE_WORD1 {
            num_mip_levels: levels,
            alpha_msb: true,
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
    /// [`pvr_mrt_clear`]), and its kick-USC PDS program.
    clear_fs3: OsalGpuMem,
    bg_kick3_off: u32,
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
    /// fbload FS variants that pack what they read for a float target
    /// (index = gles_surf_rt_class - 1: RGBA16F, R16F, R32F).
    fbload_fs_rt: [OsalGpuMem; 3],
    /// The masked clear FS variants ([`CLEAR_MASKED`]), in its order.
    clear_mask_fs: [OsalGpuMem; CLEAR_MASKED_N],
    /// This frame begins with a colour clear.
    frame_cleared: bool,
    /// This frame's first object reloads the target's colour (it began
    /// without a colour clear): a colour clear can no longer join it.
    frame_loads_color: bool,
    /// The open frame renders into the surface being swapped and
    /// [`PVR_BACKEND_OPT_SWAP_DROPS_ZS`] is on: its kick stores no
    /// depth/stencil. Set by `pvr_backend_surface_swapped`, consumed by the
    /// next flush.
    swap_drops_zs: bool,
    /// Kicks whose depth/stencil store the swap dropped (instrument).
    swap_zs_dropped: u32,
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
    /// With [`pvr_split_zs`] on and the frame's depth and stencil two
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
    clear_pack: [u32; 2],
    /// The masked clear variants' metadata (as `Backend::clear_mask_fs`), and
    /// the clear colour they read: four floats, clear.frag's uniform.
    clear_mask_sh: [ShcShader; CLEAR_MASKED_N],
    clear_rgba: [f32; 4],
    load_draw: GlesDraw,
}

/// The PowerVR's masked colour clear and the pass-type rule it needs, behind
/// one switch, default ON since the AM62P A/B (Zephyr, f9f4896f: dEQP
/// color_clear 19/19 and the gl_test regression set plus 102, 103 and 107
/// 32/32 with it on, 30/32 off; am62p-board). On: a
/// colour clear writes only the channels `clear_color_mask` names (the masked
/// clear programs below, and a partially masked background clear turned into
/// that quad), and a fragment shader that reads the pixel runs TRANSLUCENT.
/// Off: every channel is cleared and the pass type is as before. Set by
/// `pvr_debug_set_masked_clear` (Zephyr: CONFIG_GL_TEST_PVR_MASKED_CLEAR /
/// CONFIG_DEQP_PVR_MASKED_CLEAR; Linux gl_test_user: GL_TEST_PVR_MASKED_CLEAR).
#[no_mangle]
pub static mut pvr_debug_masked_clear: i32 = 1;

/// Set [`pvr_debug_masked_clear`].
#[no_mangle]
pub extern "C" fn pvr_debug_set_masked_clear(on: i32) {
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(pvr_debug_masked_clear).write(on) }
}

/// Is [`pvr_debug_masked_clear`] on?
pub fn masked_clear_on() -> bool {
    // SAFETY: a plain read of an int only the setter writes.
    (unsafe { core::ptr::addr_of!(pvr_debug_masked_clear).read() }) != 0
}

/// Keep a framebuffer's depth and stencil apart when they are two
/// renderbuffers (default ON since the AM62P measured it: dEQP fbo 646/0
/// with it against 632/14 without, and gl_test 123 zsapart passing, both
/// at 5ba5cc97 and again on the identity-keyed build at 0b17df0d;
/// am62p-board): the depth
/// in a D24S8 buffer of its own, under the depth object's identity
/// (`GlesDraw::depth_id` / `depth_gen`, [`zs_keys`]), and the stencil in a
/// buffer of its own under the stencil object's, loaded and
/// stored through CR_ISP_STENCIL_LOAD_BASE with the separate-stencil enable
/// (cr.xml ISP_STENCIL_LOAD_BASE.enable). Off, one packed D24S8 buffer per
/// (depth, stencil) pair, and two FBOs sharing a depth renderbuffer with
/// stencils of their own each got a fresh depth: dEQP
/// fbo.render.shared_depthbuffer.*_stencil_index8 (7) and, since the orphan
/// rule moves a deleted-but-attached stencil to another name,
/// recreate_stencilbuffer.no_rebind_*_stencil_index8 (7) on the AM62P
/// (a60104c2). The V3D's `v3d_split_zs` is the same fix. Mesa never pairs a
/// depth buffer with a separate stencil (Vulkan has one depth/stencil
/// attachment), so this combination is unmeasured: the stencil buffer is
/// given a whole D24S8 buffer's bytes whatever the ISP stores per sample
/// (the ISP's separate-stencil layout is still unread; the pictures agree).
/// Read when a frame opens; buffers kept under the other setting are not
/// reused (their contents start over). gl_test case 123 (zsapart) measures
/// it; Linux: YAKOGL_PVR_SPLIT_ZS=0 turns it off.
#[no_mangle]
pub static mut pvr_split_zs_on: i32 = PVR_SPLIT_ZS_DEFAULT;

/// [`pvr_split_zs_on`]'s value at boot (and the backend rig's).
pub const PVR_SPLIT_ZS_DEFAULT: i32 = 1;

/// Set [`pvr_split_zs_on`]; returns the previous setting (gl_test 123's
/// `v3d_split_zs` signature).
#[no_mangle]
pub extern "C" fn pvr_split_zs(on: bool) -> bool {
    let was = split_zs_on();
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(pvr_split_zs_on).write(i32::from(on)) }
    if was != on {
        glog!("pvr_backend: depth and stencil {}", if on { "apart (pvr_split_zs)" } else { "in one buffer" });
    }
    was
}

/// GL_LUMINANCE, GL_ALPHA and GL_LUMINANCE_ALPHA textures stored narrow
/// (1, 1 and 2 bytes a texel) instead of expanded into RGBA8: a 2048x2048
/// L8 chain is 5.3 MiB instead of 21.3 (the AM62P's texture.size.2d
/// 2048x2048_l8_mipmap ran out of the 128 MiB carve-out). 0 off (the A/B
/// arm), 1 on with pack info 0x140 (the default), 2 on with 0x11f (see
/// `tex_desc_sampler`). Measured on the AM62P at 0b17df0d: the 2048x2048
/// L8 mip chain frees 16,384 KiB less at close (26,755 against 43,139 KiB),
/// and texture is 730/77 in all three arms with identical failing sets;
/// gl_test 132 passes in all three. Read when a texture's storage is laid
/// out: a texture keeps the layout it was created with. Linux:
/// YAKOGL_PVR_NARROW_LA=0, 1 or 2 (unset keeps the default).
#[no_mangle]
pub static mut pvr_narrow_la_mode: i32 = 1;

/// Set [`pvr_narrow_la_mode`]; returns the previous value.
#[no_mangle]
pub extern "C" fn pvr_narrow_la(mode: i32) -> i32 {
    let was = narrow_la_mode();
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(pvr_narrow_la_mode).write(mode) }
    was
}

/// The current [`pvr_narrow_la_mode`] (the Linux OSAL logs it).
#[no_mangle]
pub extern "C" fn pvr_narrow_la_get() -> i32 {
    narrow_la_mode()
}

fn narrow_la_mode() -> i32 {
    // SAFETY: a plain read of an int only the setter writes.
    unsafe { core::ptr::addr_of!(pvr_narrow_la_mode).read() }
}

/// Is [`pvr_narrow_la_mode`] on?
pub fn narrow_la_on() -> bool {
    narrow_la_mode() != 0
}

/// Is [`pvr_split_zs_on`] on?
pub fn split_zs_on() -> bool {
    // SAFETY: a plain read of an int only the setter writes.
    (unsafe { core::ptr::addr_of!(pvr_split_zs_on).read() }) != 0
}

/// The TA object type a point sprite is drawn as on a target stored y-up
/// (an FBO, and a pbuffer since orientation step 1), or 0 for the one used
/// everywhere (SPRITE_01UV). ppp.xml's OBJTYPE has three sprite kinds --
/// SPRITE_10UV (2), SPRITE_UV (3), SPRITE_01UV (4) -- which by their names
/// set where the sprite's u and v start; which one mirrors v is UNMEASURED.
/// gl_PointCoord's origin is the top of the WINDOW; with SPRITE_01UV the
/// iterator's t starts at the top of the STORED image, which is the window
/// only on a target stored top-down. dEQP-GLES2
/// builtin_variable.pointcoord draws into a pbuffer and has failed since step
/// 1 (AM62P Zephyr sweep at 38905bef, as on the GC7000). This mirrors t for
/// every program, the table's pco programs included, which a shader rewrite
/// could not. The board ran pointcoord with each value (AM62P Linux, 27ebd89a):
/// it passes with 2 (SPRITE_10UV) and fails with 3 and with 0, and gl_test is
/// 83/105 in all three, so the default is 2 ([`PVR_SPRITE_YUP_DEFAULT`]).
/// Linux: YAKOGL_PVR_SPRITE_YUP=0 turns it off.
#[no_mangle]
pub static mut pvr_sprite_yup: u32 = PVR_SPRITE_YUP_DEFAULT;

/// The default of [`pvr_sprite_yup`]: SPRITE_10UV, measured (see there).
pub const PVR_SPRITE_YUP_DEFAULT: u32 = 2;

/// Set [`pvr_sprite_yup`] (0, or an OBJTYPE sprite value 2..4).
#[no_mangle]
pub extern "C" fn pvr_set_sprite_yup(objtype: u32) {
    let v = if (2..=4).contains(&objtype) { objtype } else { 0 };
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(pvr_sprite_yup).write(v) }
}

/// The background clear of a frame with several colour attachments, all
/// 8 bits per channel: on, the clear colour is uploaded once per attachment
/// (shared registers 0..n) and the background object runs the n-dword clear
/// program (two: the RGBA16F one, which is the same moves; three:
/// PVR_USC_CLEAR_FS3), so every attachment starts from the clear colour.
/// Off: the one-dword program, which writes pixel output 0 alone, and the
/// other attachments are emitted from output registers nothing wrote (gl_test
/// 106 mrt3: the clear reaches only draw buffer 0). A glClearBuffer of one
/// draw buffer (`clear_target_mask`) is not honoured either way. Default ON
/// ([`PVR_MRT_CLEAR_DEFAULT`]) since the AM62P ran it at cece5c23 (Linux path,
/// online pair): 106 mask 0 with it, mask 6 without, 55 and the regression
/// set 29/29 in both arms. Linux: YAKOGL_PVR_MRT_CLEAR=0 turns it off.
#[no_mangle]
pub static mut pvr_mrt_clear: u32 = PVR_MRT_CLEAR_DEFAULT;

/// The default of [`pvr_mrt_clear`]: on, measured (see there).
pub const PVR_MRT_CLEAR_DEFAULT: u32 = 1;

/// Set [`pvr_mrt_clear`] (0 off, anything else on).
#[no_mangle]
pub extern "C" fn pvr_set_mrt_clear(on: u32) {
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(pvr_mrt_clear).write(u32::from(on != 0)) }
}

/// ISP_CTL.dbias_is_int ([`PVR_BACKEND_OPT_DBIAS_INT`]): 1 sets the bit in
/// a frame with a depth/stencil buffer, 0 leaves it clear. Default ON
/// ([`PVR_DBIAS_INT_DEFAULT`]) since the AM62P A/B on Zephyr (RESULTS 1i,
/// a53c0122 / f963dee5): dEQP polygon_offset.* 10/4 with it off, 14/0 on;
/// gl_test 65 polyunit FAIL off, PASS on; the gl_test regression set 37/1
/// in both arms, identical per case (case 3 passes in both). Linux:
/// YAKOGL_PVR_DBIAS_INT=0 turns it off; Zephyr: CONFIG_DEQP_PVR_DBIAS_INT /
/// CONFIG_GL_TEST_PVR_DBIAS_INT = 0.
#[no_mangle]
pub static mut pvr_dbias_int: u32 = PVR_DBIAS_INT_DEFAULT;

/// The default of [`pvr_dbias_int`]: on, measured (see there).
pub const PVR_DBIAS_INT_DEFAULT: u32 = 1;

/// [`PVR_BACKEND_OPT_SWAP_DROPS_ZS`]: 1 = the swap drops the window's
/// depth/stencil. Its own switch because its default is 1, which the
/// all-zero option array cannot carry.
#[no_mangle]
pub static mut pvr_swap_drops_zs: u32 = PVR_SWAP_DROPS_ZS_DEFAULT;

/// The default of [`pvr_swap_drops_zs`]: on, measured (see there).
pub const PVR_SWAP_DROPS_ZS_DEFAULT: u32 = 1;

/// Set [`pvr_swap_drops_zs`] (0 off, anything else on).
#[no_mangle]
pub extern "C" fn pvr_set_swap_drops_zs(on: u32) {
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(pvr_swap_drops_zs).write(u32::from(on != 0)) }
}

/// Set [`pvr_dbias_int`] (0 off, anything else on).
#[no_mangle]
pub extern "C" fn pvr_set_dbias_int(on: u32) {
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(pvr_dbias_int).write(u32::from(on != 0)) }
}

/// The output dwords the background clear writes: one per attachment when
/// [`pvr_mrt_clear`] is on and the frame's attachments are several and all
/// 8-bit (at most PVR_BACKEND_MAX_RT, three), else 0 (the single-target
/// programs).
fn mrt_clear_dwords(b: &Backend) -> u32 {
    // SAFETY: a plain read of an int only the setter writes.
    let on = unsafe { core::ptr::addr_of!(pvr_mrt_clear).read() } != 0;
    let n = b.num_targets as usize;
    if !on || !(2..=PVR_BACKEND_MAX_RT as usize).contains(&n) {
        return 0;
    }
    let fmt = |i: usize| if i != 0 { b.targets[i].format } else { b.target_format };
    if (0..n).any(|i| gles_surf_rt_class(fmt(i)) != 0) {
        return 0;
    }
    n as u32
}

/// The TA object type of a point sprite on this target.
fn point_objtype(flip_y: bool) -> u32 {
    // SAFETY: a plain read of an int only the setter writes.
    let yup = unsafe { core::ptr::addr_of!(pvr_sprite_yup).read() };
    if !flip_y && yup != 0 {
        yup
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

/// The masked clear programs: clear.frag with glColorMask lowered by pco (the
/// channels left off are read back from the pixel, `uses_fbfetch`), for each
/// partial mask M = 1..=14 (bit 0 R .. 3 A, set = written) -- first the
/// RGBA8 forms, then the RGBA16F ones. The PowerVR has no per-channel colour
/// write mask in the ISP or the PBE: Mesa's pvr lowers a write mask into the
/// fragment shader the same way (pvr_arch_pipeline.c, `rt->write_mask` into
/// `blend_opts.rt[].colormask`). Generated by tools/glsl2usc.sh (usc-blob).
const CLEAR_MASKED_N: usize = 28;
#[rustfmt::skip]
const CLEAR_MASKED: [(&[u32], u32); CLEAR_MASKED_N] = [
    (&PVR_USC_CLEAR_M1, PVR_USC_CLEAR_M1_TEMPS), (&PVR_USC_CLEAR_M2, PVR_USC_CLEAR_M2_TEMPS),
    (&PVR_USC_CLEAR_M3, PVR_USC_CLEAR_M3_TEMPS), (&PVR_USC_CLEAR_M4, PVR_USC_CLEAR_M4_TEMPS),
    (&PVR_USC_CLEAR_M5, PVR_USC_CLEAR_M5_TEMPS), (&PVR_USC_CLEAR_M6, PVR_USC_CLEAR_M6_TEMPS),
    (&PVR_USC_CLEAR_M7, PVR_USC_CLEAR_M7_TEMPS), (&PVR_USC_CLEAR_M8, PVR_USC_CLEAR_M8_TEMPS),
    (&PVR_USC_CLEAR_M9, PVR_USC_CLEAR_M9_TEMPS), (&PVR_USC_CLEAR_MA, PVR_USC_CLEAR_MA_TEMPS),
    (&PVR_USC_CLEAR_MB, PVR_USC_CLEAR_MB_TEMPS), (&PVR_USC_CLEAR_MC, PVR_USC_CLEAR_MC_TEMPS),
    (&PVR_USC_CLEAR_MD, PVR_USC_CLEAR_MD_TEMPS), (&PVR_USC_CLEAR_ME, PVR_USC_CLEAR_ME_TEMPS),
    (&PVR_USC_CLEAR_M1_F16X4, PVR_USC_CLEAR_M1_F16X4_TEMPS), (&PVR_USC_CLEAR_M2_F16X4, PVR_USC_CLEAR_M2_F16X4_TEMPS),
    (&PVR_USC_CLEAR_M3_F16X4, PVR_USC_CLEAR_M3_F16X4_TEMPS), (&PVR_USC_CLEAR_M4_F16X4, PVR_USC_CLEAR_M4_F16X4_TEMPS),
    (&PVR_USC_CLEAR_M5_F16X4, PVR_USC_CLEAR_M5_F16X4_TEMPS), (&PVR_USC_CLEAR_M6_F16X4, PVR_USC_CLEAR_M6_F16X4_TEMPS),
    (&PVR_USC_CLEAR_M7_F16X4, PVR_USC_CLEAR_M7_F16X4_TEMPS), (&PVR_USC_CLEAR_M8_F16X4, PVR_USC_CLEAR_M8_F16X4_TEMPS),
    (&PVR_USC_CLEAR_M9_F16X4, PVR_USC_CLEAR_M9_F16X4_TEMPS), (&PVR_USC_CLEAR_MA_F16X4, PVR_USC_CLEAR_MA_F16X4_TEMPS),
    (&PVR_USC_CLEAR_MB_F16X4, PVR_USC_CLEAR_MB_F16X4_TEMPS), (&PVR_USC_CLEAR_MC_F16X4, PVR_USC_CLEAR_MC_F16X4_TEMPS),
    (&PVR_USC_CLEAR_MD_F16X4, PVR_USC_CLEAR_MD_F16X4_TEMPS), (&PVR_USC_CLEAR_ME_F16X4, PVR_USC_CLEAR_ME_F16X4_TEMPS),
];

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
pub fn clear_masked_program(class: u32, mask: u8) -> Option<&'static [u32]> {
    clear_masked_index(class, mask).map(|i| CLEAR_MASKED[i].0)
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
            glog!("pvr_backend: kick {} did not complete ({})", fence, rc);
            return if rc > 0 { -ETIMEDOUT } else { rc };
        }
        0
    }

    fn opt(&self, o: PvrBackendOption) -> u32 {
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

    if (opt as usize) < PVR_BACKEND_OPT_COUNT {
        b.opt[opt as usize] = value;
    }
    /* this option's value carries a default of 1, which the all-zero option
     * array cannot: it lives in its own switch */
    if opt == PVR_BACKEND_OPT_DBIAS_INT {
        pvr_set_dbias_int(value);
    }
    if opt == PVR_BACKEND_OPT_SWAP_DROPS_ZS {
        pvr_set_swap_drops_zs(value);
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
                PVR_USC_CLEAR_FS_TEMPS,
                0,
                kd.as_mut_ptr(),
                kc.as_mut_ptr(),
                &mut ki,
            )
        } != 0
        {
            glog!("pvr_backend: the background kick program for the one-dword clear was refused");
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
                PVR_USC_CLEAR_FS2_TEMPS,
                0,
                kd.as_mut_ptr(),
                kc.as_mut_ptr(),
                &mut ki,
            )
        } != 0
        {
            glog!("pvr_backend: the background kick program for the two-dword clear was refused");
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
                PVR_USC_CLEAR_FS3_TEMPS,
                0,
                kd.as_mut_ptr(),
                kc.as_mut_ptr(),
                &mut ki,
            )
        } != 0
        {
            glog!("pvr_backend: the background kick program for the three-dword clear was refused");
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
            glog!("pvr_backend: the background colour DMA program was refused");
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

fn static_objects_init(b: &mut Backend, fl: &mut FbLoad) -> i32 {
    /* Clear FS (Mesa clear-attachment USC: reads the packed colour from sh0). */
    let mut rc = alloc_mapped(HEAP_USC_BASE, PVR_USC_CLEAR_FS2_SIZE_BYTES, &mut b.clear_fs2);
    if rc != 0 {
        return rc;
    }
    mem_write(&b.clear_fs2, 0, &PVR_USC_CLEAR_FS2_BYTES);
    cache_flush(&b.clear_fs2, 0, PVR_USC_CLEAR_FS2_SIZE_BYTES as u32);
    let bytes = core::mem::size_of_val(&PVR_USC_CLEAR_FS3_WORDS);
    rc = alloc_mapped(HEAP_USC_BASE, bytes, &mut b.clear_fs3);
    if rc != 0 {
        return rc;
    }
    mem_write(&b.clear_fs3, 0, &PVR_USC_CLEAR_FS3_WORDS);
    cache_flush(&b.clear_fs3, 0, bytes as u32);
    rc = alloc_mapped(HEAP_USC_BASE, PVR_USC_CLEAR_FS_SIZE_BYTES, &mut b.clear_fs);
    if rc != 0 {
        return rc;
    }
    mem_write(&b.clear_fs, 0, &PVR_USC_CLEAR_FS_BYTES);
    cache_flush(&b.clear_fs, 0, PVR_USC_CLEAR_FS_SIZE_BYTES as u32);
    fl.clear_fs_sh = ShcShader::ZERO;
    fl.clear_fs_sh.meta_off = SHC_NO_META;
    fl.clear_fs_sh.usc = PVR_USC_CLEAR_FS_WORDS.as_ptr();
    fl.clear_fs_sh.usc_dw = (PVR_USC_CLEAR_FS_SIZE_BYTES / 4) as u32;
    fl.clear_fs_sh.temps = PVR_USC_CLEAR_FS_TEMPS as u16;
    fl.clear_fs_sh.num_outputs = 1;
    fl.clear_fs_sh.uniform_dwords = 1;
    fl.clear_fs2_sh = fl.clear_fs_sh;
    fl.clear_fs2_sh.usc = PVR_USC_CLEAR_FS2_WORDS.as_ptr();
    fl.clear_fs2_sh.usc_dw = (PVR_USC_CLEAR_FS2_SIZE_BYTES / 4) as u32;
    fl.clear_fs2_sh.temps = PVR_USC_CLEAR_FS2_TEMPS as u16;
    fl.clear_fs2_sh.uniform_dwords = 2;

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

    rc = alloc_mapped(HEAP_USC_BASE, PVR_USC_FBLOAD_DWORDS * 4, &mut b.fbload_fs);
    if rc == 0 {
        rc = alloc_mapped(HEAP_USC_BASE, PVR_USC_FBLOAD_VS_DWORDS * 4, &mut b.fbload_vs);
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
    {
        const RT_USC: [&[u32]; 3] = [&PVR_USC_FBLOAD_F16X4, &PVR_USC_FBLOAD_F16, &PVR_USC_FBLOAD_F32];
        const RT_TEMPS: [u32; 3] = [PVR_USC_FBLOAD_F16X4_TEMPS, PVR_USC_FBLOAD_F16_TEMPS, PVR_USC_FBLOAD_F32_TEMPS];

        for i in 0..3 {
            let usc = RT_USC[i];
            let bytes = usc.len() * 4;

            rc = alloc_mapped(HEAP_USC_BASE, bytes, &mut b.fbload_fs_rt[i]);
            if rc != 0 {
                return rc;
            }
            mem_write(&b.fbload_fs_rt[i], 0, usc);
            cache_flush(&b.fbload_fs_rt[i], 0, bytes as u32);
            let sh = &mut fl.fs_rt_sh[i];
            *sh = ShcShader::ZERO;
            sh.meta_off = SHC_NO_META;
            sh.usc = usc.as_ptr();
            sh.usc_dw = usc.len() as u32;
            sh.temps = RT_TEMPS[i] as u16;
            sh.num_outputs = 1;
        }
    }
    for (i, (usc, temps)) in CLEAR_MASKED.iter().enumerate() {
        let bytes = usc.len() * 4;

        rc = alloc_mapped(HEAP_USC_BASE, bytes, &mut b.clear_mask_fs[i]);
        if rc != 0 {
            return rc;
        }
        mem_write(&b.clear_mask_fs[i], 0, usc);
        cache_flush(&b.clear_mask_fs[i], 0, bytes as u32);
        let sh = &mut fl.clear_mask_sh[i];
        *sh = ShcShader::ZERO;
        sh.meta_off = SHC_NO_META;
        sh.usc = usc.as_ptr();
        sh.usc_dw = usc.len() as u32;
        sh.temps = *temps as u16;
        /* RGBA16F is two output dwords, as pco's f16x4 records say */
        sh.num_outputs = if i < 14 { 1 } else { 2 };
        sh.uniform_dwords = 4;
        sh.uses_fbfetch = 1;
    }
    mem_write(&b.fbload_fs, 0, &PVR_USC_FBLOAD);
    mem_write(&b.fbload_vs, 0, &PVR_USC_FBLOAD_VS);
    mem_write(&b.fbload_quad, 0, &QUAD);
    cache_flush(&b.fbload_fs, 0, (PVR_USC_FBLOAD_DWORDS * 4) as u32);
    cache_flush(&b.fbload_vs, 0, (PVR_USC_FBLOAD_VS_DWORDS * 4) as u32);
    cache_flush(&b.fbload_quad, 0, QUAD_BYTES as u32);
    /* the shader metadata the recorder needs for the two internal programs */
    /* Hand-built shader descriptors: no ISP-feedback meta word. A zeroed
     * meta_off would make record_draw() DMA SHC_FS_META_DEFAULT into sh0 --
     * for the load FS that is the texture descriptor, and the colour reload
     * then sampled garbage (gl_test cases 11/13/21 read zeros). */
    fl.vs_sh = ShcShader::ZERO;
    fl.vs_sh.meta_off = SHC_NO_META;
    fl.vs_sh.usc = PVR_USC_FBLOAD_VS.as_ptr();
    fl.vs_sh.usc_dw = PVR_USC_FBLOAD_VS_DWORDS as u32;
    fl.vs_sh.temps = PVR_USC_FBLOAD_VS_TEMPS as u16;
    fl.vs_sh.num_attribs = 1;
    fl.vs_sh.attribs[0].components = 4;
    fl.vs_sh.vs_output_size = 4;
    fl.vs_sh.vs_psize_off = SHC_NO_PSIZE;
    fl.fs_sh = ShcShader::ZERO;
    fl.fs_sh.meta_off = SHC_NO_META;
    fl.fs_sh.usc = PVR_USC_FBLOAD.as_ptr();
    fl.fs_sh.usc_dw = PVR_USC_FBLOAD_DWORDS as u32;
    fl.fs_sh.temps = PVR_USC_FBLOAD_TEMPS as u16;
    fl.fs_sh.num_outputs = 1;
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
        &mut b.fbload_fs,
        &mut b.fbload_vs,
        &mut b.fbload_quad,
        &mut b.black_tex,
        &mut b.seq_idx,
    ] {
        osal::gpu_free(m);
    }
    for m in b.fbload_fs_rt.iter_mut().chain(b.clear_mask_fs.iter_mut()) {
        osal::gpu_free(m);
    }
    for s in b.ds.iter_mut() {
        osal::gpu_free(&mut s.mem);
        *s = DsBuf::ZERO;
    }
    b.inited = false;
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
fn clear_uniform(surf: u8, c: &[f32; 4]) -> [u32; 2] {
    if gles_surf_rt_class(surf) != 0 {
        pack_clear_float(surf, c)
    } else {
        [pack_rgba8(c), 0]
    }
}

/// The float encoding of a clear colour for a float render-target class.
/// Only reached for those: [`clear_uniform`] is the way in.
fn pack_clear_float(surf: u8, c: &[f32; 4]) -> [u32; 2] {
    if surf == GLES_SURF_RGBA16F {
        [f32_to_f16_bits(c[0]) | (f32_to_f16_bits(c[1]) << 16), f32_to_f16_bits(c[2]) | (f32_to_f16_bits(c[3]) << 16)]
    } else if surf == GLES_SURF_R16F {
        [f32_to_f16_bits(c[0]), 0]
    } else {
        [c[0].to_bits(), 0]
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
    let rt_class = gles_surf_rt_class(d.target_format);
    let rt_sh: *const ShcShader = if rt_class != 0 { &fl.fs_rt_sh[rt_class as usize - 1] } else { core::ptr::null() };
    let two = d.target_format == GLES_SURF_RGBA16F;
    let clear_sh: *const ShcShader = if two { &fl.clear_fs2_sh } else { &fl.clear_fs_sh };
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
        /* a float target: the load FS packs the texel for its class */
        ld.fs = rt_sh;
        ld.fs_usc_gpu = b.fbload_fs_rt[rt_class as usize - 1].gpu;
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
    ld.textures[0].format = gles_surf_texfmt(d.target_format);
    ld.textures[0].bgra = if d.target_format == GLES_SURF_XRGB8888 { 1 } else { 0 };
    ld.num_textures = 1;
    ld.viewport[2] = d.target_w as i32;
    ld.viewport[3] = d.target_h as i32;
    ld.flip_y = d.flip_y;
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
        /* the clear colour through the clear USC, as an ordinary draw */
        ld.fs = clear_sh;
        ld.fs_usc_gpu = if two { b.clear_fs2.gpu } else { b.clear_fs.gpu };
        ld.num_textures = 0;
        ld.fs_uniform_cpu = clear_pack as *const c_void;
        ld.fs_uniform_dwords = if two { 2 } else { 1 };
        ld.color_write = true;
    }
    if record_draw_txn(b, &mut p.scratch, &fl.load_draw, 0, 6) != 0 {
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
fn record_clear_quad(p: &mut Pvr, d: &GlesDraw) -> i32 {
    let has_ds = p.b.frame_ds.buf().is_some();
    let class = gles_surf_rt_class(d.target_format);
    /* with the switch off, every channel: the clear as it always was */
    let mut mask = match (d.do_clear, masked_clear_on()) {
        (false, _) => 0,
        (true, true) => d.clear_color_mask & 0xf,
        (true, false) => 0xf,
    };
    /* A one-channel float target has only red: it is written or it is not. */
    if class >= 2 {
        mask = if mask & 1 != 0 { 0xf } else { 0 };
    }
    let color = d.do_clear && mask != 0;
    let depth = d.do_clear_depth && has_ds;
    let stencil = d.do_clear_stencil && has_ds;

    if !color && !depth && !stencil {
        return 0;
    }
    let b = &mut p.b;
    let fl = &mut p.fbload;
    let two = d.target_format == GLES_SURF_RGBA16F;
    let masked = clear_masked_index(class, mask);
    let clear_sh: *const ShcShader = match masked {
        Some(i) => &fl.clear_mask_sh[i],
        None if two => &fl.clear_fs2_sh,
        None => &fl.clear_fs_sh,
    };
    fl.clear_pack = clear_uniform(d.target_format, &d.clear_rgba);
    fl.clear_rgba = d.clear_rgba;
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
        None if two => b.clear_fs2.gpu,
        None => b.clear_fs.gpu,
    };
    ld.fs_uniform_cpu = clear_pack;
    ld.fs_uniform_dwords = match masked {
        Some(_) => 4,
        None if two => 2,
        None => 1,
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
    if record_draw_txn(b, &mut p.scratch, &fl.load_draw, 0, 6) != 0 {
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

    for i in 0..PVR_BACKEND_MAX_DS {
        let s = &mut b.ds[i];

        if !s.mem.impl_.is_null() && s.id == id {
            if s.w == w && s.h == h && s.samples == samples {
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
        ", ds given up {}, ds refused {} (slots full {}, no buffer {})",
        b.ds_given_up,
        b.ds_full_refused.wrapping_add(b.ds_refused),
        b.ds_full_refused,
        b.ds_refused
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
        glog!(
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

/// Bytes of the packed D24S8 buffer of a w x h target (whole ZLS tiles, per
/// sample), or `None` when that does not fit a usize.
fn ds_bytes(w: u32, h: u32, samples: u32) -> Option<usize> {
    let aw = (w.div_ceil(ZLS_TILE) as usize).checked_mul(ZLS_TILE as usize * if samples == 4 { 2 } else { 1 })?;
    let ah = (h.div_ceil(ZLS_TILE) as usize).checked_mul(ZLS_TILE as usize * if samples >= 2 { 2 } else { 1 })?;

    aw.checked_mul(ah)?.checked_mul(4)
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
        let Some(i) = (0..PVR_BACKEND_MAX_DS).find(|&i| can_go(i)) else {
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
            glog!("pvr_backend: the frame drawing into object 0x{:x} did not submit ({}) before it went", id, rc);
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
    b.target_gpu = d.target_gpu;
    b.target_w = d.target_w;
    b.target_h = d.target_h;
    b.target_pitch = crate::backend::draw_pitch(d);
    b.target_format = d.target_format;
    b.target_alpha_bits = d.target_alpha_bits;
    b.target_flip_y = d.flip_y;
    b.num_targets = (if d.num_targets > 1 { d.num_targets } else { 1 }).min(PVR_BACKEND_MAX_RT as u8);
    b.targets[0] = GlesDrawTarget { gpu: d.target_gpu, w: d.target_w, h: d.target_h, format: d.target_format };
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
        glog!("pvr_backend: reusing frame set {} before its kick completed", b.cur);
    }
    b.frame_set_activate(b.cur);
    b.pds.bump = 0;
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
        let clear_float = bg_color && gles_surf_rt_class(d.target_format) != 0;

        if load_color || init_depth || init_stencil || clear_float {
            let _ = record_frame_quad(p, d, load_color, init_depth, init_stencil, clear_float);
        }
    }
}

/// One pvr_combined_image_sampler_descriptor (TEX_DESC_DWORDS, zeroed by the
/// caller): image words 0..3, image meta 4..7, sampler words 8..11, sampler
/// meta 12..15, gather words 16..19.
fn build_tex_desc(w: &mut [u32], t: &GlesTextureBind, warned: &mut u32, force_mip0: bool) {
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
     * (Mesa pvr_image_plane_setup_mip_levels) */
    let mut levels: u32 = if t.mip_filter != 0 && t.mip_levels > 1 && !force_mip0 { t.mip_levels as u32 } else { 1 };

    if t.format == GLES_TEXFMT_RGBA8 {
        fmt = if t.bgra != 0 { TEXF_BGRA8 } else { TEXF_RGBA8 };
        w[4] = t.stride_texels.wrapping_mul(t.height).wrapping_mul(4); /* layer size (bytes) */
        w[7] = TEX_PCK_INFO_RGBA8_UNORM;
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
         * `pvr_narrow_la` 1 takes 0x140, 2 takes 0x11f. */
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
    } else {
        fmt = if t.format == GLES_TEXFMT_ETC2_RGB {
            TEXF_ETC2_RGB
        } else if t.format == GLES_TEXFMT_ETC2_RGBA {
            TEXF_ETC2_RGBA
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
        let etc = matches!(t.format, GLES_TEXFMT_ETC2_RGB | GLES_TEXFMT_ETC2_RGBA | GLES_TEXFMT_ETC2_RGB_PT);
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
    let iw0 = tex_image_word0(fmt, fmt.textype, t.width, t.height);
    /* the STRIDE word for a linear image; the ETC formats' twiddled word,
     * where [60:63] is baselevel (0) and the level count is [0:3] */
    let twiddled = matches!(t.format, GLES_TEXFMT_ETC2_RGB | GLES_TEXFMT_ETC2_RGBA | GLES_TEXFMT_ETC2_RGB_PT);
    let iw1 = if twiddled {
        tex_twiddled_image_word1(true, levels & 0xf, 0, levels > 1, t.base_gpu)
    } else {
        tex_stride_image_word1(t.stride_texels, levels, t.base_gpu)
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
            /* the third axis is never sampled here (2D, and the emulated
             * cube and array targets address inside one tall 2D image) */
            addrmode_w: csb::ROGUE_TEXSTATE_ADDRMODE_CLAMP_TO_EDGE,
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
     * (glDrawBuffers({GL_NONE, GL_COLOR_ATTACHMENT1}), ES 3.0 4.2.1): slot 0
     * is the frame's own target and its end-of-tile emit always stores it
     * (draw_frame), so this draw would write attachment 0. Refused by name,
     * never drawn into the wrong image (review R-09). */
    if d.num_targets > 1 && d.targets[0].gpu == 0 {
        if !p.b.warned_draw_buffer0 {
            p.b.warned_draw_buffer0 = true;
            glog!("yakogl/pvr: draw refused: draw buffer 0 is GL_NONE while a later draw buffer is written");
        }
        return -util::ENOTSUP;
    }
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
    if masked_clear_on() && d.do_clear && !d.clear_quad && partial(d.clear_color_mask) {
        let mut rest = *d;
        rest.do_clear = false;
        rest.clear_color_mask = 0;
        let mut quad = *d;
        quad.clear_quad = true;
        quad.do_clear_depth = false;
        quad.do_clear_stencil = false;
        quad.scissor = false;
        quad.index_count = 0;
        /* the depth/stencil part, and the frame, first; then the colour */
        if rest.do_clear_depth || rest.do_clear_stencil || !p.b.frame_open || frame_ends_here(&p.b, &rest, false) {
            let rc = draw_one(p, &GlesDraw { index_count: 0, ..rest });
            if rc != 0 {
                return rc;
            }
        }
        let rc = draw_one(p, &quad);
        if rc != 0 || rest.index_count == 0 {
            return rc;
        }
        return draw_one(p, &GlesDraw { do_clear_depth: false, do_clear_stencil: false, ..rest });
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

    /* a frame this draw opens needs its depth/stencil buffers: made room
     * for, or the draw is refused before anything is recorded */
    if !p.b.frame_open || frame_ends_here(&p.b, d, any_clear) {
        let rc = ds_make_room(&mut p.b, d);
        if rc != 0 {
            return rc;
        }
    }
    if p.b.frame_open && frame_ends_here(&p.b, d, any_clear) {
        let rc = flush(p);
        if rc != 0 {
            return public_rc(rc);
        }
    }
    /* the frame's depth/stencil (found now for a frame this draw opens), and
     * the refusal of a draw that uses one it cannot keep, before the frame
     * takes anything from the draw */
    let zs = if p.b.frame_open { (p.b.frame_ds, p.b.frame_s) } else { ds_resolve(&mut p.b, d) };
    let rc = ds_refuse_lost(&mut p.b, d, zs);
    if rc != 0 {
        return rc;
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
        let clear_float = d.do_clear && gles_surf_rt_class(b.target_format) != 0;
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
        glog!("pvr_backend: draw without linked shaders");
        return -EINVAL;
    }
    if !draw_is_well_formed(d) {
        glog!("pvr_backend: malformed draw ({} attributes, {} indices)", d.num_attribs, d.index_count);
        return -EINVAL;
    }
    public_rc(draw_sized(p, d))
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
/// sequential draw is longer than one list can address.
fn draw_dispatch(p: &mut Pvr, d: &GlesDraw) -> i32 {
    let sequential = d.index_buf_gpu == 0 && (d.index_cpu.is_null() || d.index_bytes == 0);
    if sequential && d.index_count > SEQ_MAX_VERTICES {
        draw_chunked(p, d)
    } else {
        draw_part(p, d, 0, d.index_count)
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
/// `d.index_count`) into the open frame. Split the frame (the next kick
/// loads this one's colour back) when the draw table or an arena could not
/// take another draw -- a draw needs at most PDS_PER_DRAW_MAX /
/// GEN_PER_DRAW_MAX bytes plus its staged data, and VDM_PER_DRAW_MAX stream
/// words -- and once more when the draw still did not fit (large staged
/// arrays or indices, full depth-bias / scissor tables).
fn draw_part(p: &mut Pvr, d: &GlesDraw, first: u32, count: u32) -> i32 {
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
    let mut rc = record_draw_txn(&mut p.b, &mut p.scratch, d, first, count);
    if rc == -EFRAME_FULL && p.b.ndraws > p.b.frame_quads {
        /* the frame's earlier draws took the room: a fresh frame has it */
        rc = flush(p);
        if rc != 0 {
            return rc;
        }
        frame_begin(p, d);
        rc = record_draw_txn(&mut p.b, &mut p.scratch, d, first, count);
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
    let (unit, overlap): (u32, u32) = match d.prim {
        GLES_PRIM_POINTS => (1, 0),
        GLES_PRIM_LINES => (2, 0),
        GLES_PRIM_LINE_STRIP => (1, 1),
        GLES_PRIM_TRI_STRIP => (2, 2),
        GLES_PRIM_TRI_FAN | GLES_PRIM_LINE_LOOP => (0, 0),
        _ => (3, 0), /* a triangle list (record_draw's default topology) */
    };
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
        let rc = draw_part(p, if first == 0 { d } else { &cont }, first, n);
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

        let force_mip0 = b.opt[PVR_BACKEND_OPT_FORCE_MIP0 as usize] != 0;

        build_tex_desc(&mut desc[i * DESC..(i + 1) * DESC], t.unwrap_or(&fallback), &mut b.warned_tex_addr, force_mip0);
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
fn record_draw_txn(b: &mut Backend, s: &mut Scratch, d: &GlesDraw, first: u32, count: u32) -> i32 {
    let saved = (b.pds.bump, b.gen.bump, b.ndbias, b.dbias_last, b.nscissor, b.scissor_last, b.term_gpu, b.vdm_dw);
    let rc = record_draw(b, s, d, first, count);

    if rc != 0 {
        (b.pds.bump, b.gen.bump, b.ndbias, b.dbias_last, b.nscissor, b.scissor_last, b.term_gpu, b.vdm_dw) = saved;
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
/// `d.index_count`. Returns 0, -[`EFRAME_FULL`] (the frame has no room),
/// -EINVAL (a draw the backend cannot encode), -ENOMEM (no memory for the
/// sequential indices) or -EIO (an arena without a CPU mapping).
fn record_draw(b: &mut Backend, s: &mut Scratch, d: &GlesDraw, first: u32, count: u32) -> i32 {
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
        glog!("pvr_backend: {} attributes not supported", d.num_attribs);
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
                glog!("pvr_backend: only float32 vec1..4 attributes are supported");
                return -EINVAL;
            }
            order[i] = i as u8;
            /* a piece of a split sequential draw starts at vertex `first` */
            let skip = if a.per_instance == 0 { first as u64 * a.stride as u64 } else { 0 };
            if !a.cpu.is_null() && a.cpu_bytes != 0 {
                /* draw-time data: the GL core's staging is reused by the
                 * next draw, so this draw gets its own copy in the arena
                 * (a piece copies only the bytes its vertices use) */
                let (from, len) = if !piece || a.per_instance != 0 || a.stride == 0 {
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
        glog!(
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
    rc = arena_put_words(&mut b.pds, vdata, PDS_ALIGN, Some(&mut v_pds_data), None);
    if rc == 0 {
        rc = arena_put_words(&mut b.pds, vcode, PDS_ALIGN, Some(&mut v_pds_code), None);
    }
    if rc != 0 {
        return rc;
    }

    /* ---- fragment kick-USC PDS: [data(8) | code(1)] contiguous ---- */
    let mut k_prog = [0u32; 9];
    let mut k_pds_data = 0u32;
    {
        let mut ki = PvrPdsRenderKickUscInfo::default();

        let temps = (fs.temps as u32).max(b.opt(PVR_BACKEND_OPT_FS_TEMPS_MIN).min(FS_TEMPS_MIN_CAP));
        // SAFETY: k_prog holds the 8 data dwords followed by the 1 code dword.
        if unsafe {
            pvr_pds_render_kick_usc_program(
                heap_off_of(d.fs_usc_gpu) as u64,
                temps,
                fs_sample_rate(b.samples, d.blend || fs.uses_fbfetch != 0 || fs.uses_discard != 0),
                k_prog.as_mut_ptr(),
                k_prog.as_mut_ptr().add(8),
                &mut ki,
            )
        } != 0
        {
            glog!(
                "yakogl/pvr: draw refused: the fragment kick program was refused ({} temps, max {})",
                temps,
                PVR_PDS_DOUTU_MAX_TEMPS
            );
            return -EINVAL;
        }
        let n = ((ki.data_size_dw + ki.code_size_dw) as usize).min(k_prog.len());
        rc = arena_put_words(&mut b.pds, &k_prog[..n], PDS_ALIGN, Some(&mut k_pds_data), None);
        if rc != 0 {
            return rc;
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
        // SAFETY: cdata/ccode have the sizes the generator documents for up to
        // PVR_PDS_COEFF_MAX_VARYINGS varyings; nv was checked against that.
        if unsafe {
            pvr_pds_render_coeff_program(
                fs.uses_w as i32,
                /* gl_FragCoord.z: pco reads the Z plane from set 0, ahead of
                 * W (Mesa pvr_arch_pipeline.c:1503-1518); the TE only hands
                 * it over when OUTPUT_SEL says so (z_pres below). */
                (fs.uses_z != 0) as i32,
                /* gl_PointCoord: pco reads it from the 8 coefficients after W,
                 * iterated by a point-sprite DOUTI (Mesa pvr_arch_pipeline.c:
                 * 1530-1540), and puts the varyings after it. Without the
                 * iterator the shader read coefficients nobody wrote (case 83:
                 * s = t = 1.0 across the sprite). */
                (fs.uses_pntc != 0) as i32,
                s.cv.as_ptr(),
                nv,
                s.cdata.as_mut_ptr(),
                s.ccode.as_mut_ptr(),
                &mut coeff,
            )
        } != 0
        {
            glog!("yakogl/pvr: draw refused: the varying coefficient program was refused ({} varyings)", nv);
            return -EINVAL;
        }
        if coeff.code_size_dw != 0 {
            let nd = (coeff.data_size_dw as usize).min(CDATA_DWORDS);
            let nc = (coeff.code_size_dw as usize).min(CCODE_DWORDS);

            s.cprog[..nd].copy_from_slice(&s.cdata[..nd]);
            s.cprog[nd..nd + nc].copy_from_slice(&s.ccode[..nc]);
            rc = arena_put_words(&mut b.pds, &s.cprog[..nd + nc], PDS_ALIGN, Some(&mut coeff_off), None);
            if rc != 0 {
                return rc;
            }
            if b.opt(PVR_BACKEND_OPT_DEBUG) != 0 && b.dbg_draws_left != 0 {
                dump_words("coeff_prog", &s.cprog[..nd + nc]);
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
        let (scale, offset) = if d.flip_y { (-1.0f32, d.target_h as f32) } else { (1.0, 0.0) };
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
            glog!("yakogl/pvr: draw refused: the fragment common-store DMA program was refused ({} kicks)", nkicks);
            return -EINVAL;
        }
        let ud = &ud[..(ui.data_size_dw as usize).min(ud.len())];
        let uc = &uc[..(ui.code_size_dw as usize).min(uc.len())];
        rc = arena_put_words(&mut b.pds, ud, PDS_ALIGN, Some(&mut t_pds_data), None);
        if rc == 0 {
            rc = arena_put_words(&mut b.pds, uc, PDS_ALIGN, Some(&mut t_pds_code), None);
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
    let passtype = if fs.uses_discard != 0 {
        PVR_PPP_PASSTYPE_PUNCH_THROUGH
    } else if d.depth_test && d.depth_write && !d.blend && (fs.uses_fbfetch == 0 || !masked_clear_on()) && d.color_write
    {
        PVR_PPP_PASSTYPE_OPAQUE
    } else {
        PVR_PPP_PASSTYPE_TRANSLUCENT
    };
    fi.passtype = passtype as u8;
    fi.varying_program_code_offset = coeff_off;

    st.objtype = objtype;
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

        if b.opt(PVR_BACKEND_OPT_TERM0) != 0 {
            s.ppp[ppp_words + 1] = b.opt(PVR_BACKEND_OPT_TERM0);
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
            glog!("yakogl/pvr: draw refused: the vertex common-store DMA program was refused ({} kicks)", nvk);
            return -EINVAL;
        }
        let ud = &ud[..(ui.data_size_dw as usize).min(ud.len())];
        let uc = &uc[..(ui.code_size_dw as usize).min(uc.len())];
        rc = arena_put_words(&mut b.pds, ud, PDS_ALIGN, Some(&mut ps.pds_data_offset), None);
        if rc == 0 {
            rc = arena_put_words(&mut b.pds, uc, PDS_ALIGN, Some(&mut ps.pds_code_offset), None);
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
            glog!("pvr_backend: the frame's terminate block was refused: frame dropped");
            b.frame_open = false;
            return rc;
        }
        if b.opt(PVR_BACKEND_OPT_TERM0) != 0 {
            term[1] = b.opt(PVR_BACKEND_OPT_TERM0);
        }
        let rc = arena_put_words(&mut b.gen, &term, PPP_ALIGN, None, Some(&mut term_gpu));
        if rc != 0 {
            /* a frame that cannot be closed is dropped, not kept open (every
             * later draw and flush would fail on it again) */
            glog!("pvr_backend: no room for the frame's terminate block ({}): frame dropped", rc);
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
            glog!("pvr_backend: the frame's trailing PPP state update was refused: frame dropped");
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
    {
        let mut col = [0u32; CLEAR_ATTACH_DWORDS as usize];
        let mut ed = [0u32; 3 * PVR_BACKEND_MAX_RT as usize + 2];
        let mut ec = [0u32; PVR_BACKEND_MAX_RT as usize + 2];
        let (mut dsz, mut csz) = (0u32, 0u32);
        let da = b.target_gpu;

        let rt = gles_surf_rt_class(b.target_format);

        col[0] = packed[0];
        if rt != 0 {
            col[1] = packed[1];
        }
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
            let irt = gles_surf_rt_class(fmt);

            /* Which attachment this state is for, and which 32-bit slot of
             * the shader's output registers it takes: attachment i emits
             * output i, and an output is one register for the 8-bit formats,
             * two for RGBA16F (PBESTATE_STATE_WORD1 mrt_index at 16..18,
             * source_pos at 14..15). A 64-bit source starts at an even
             * register (pbe2_in_xe), and a pixel has four of them: the
             * compiler lays the outputs out the same way, and an attachment
             * past the fourth register is not emitted -- writing it would
             * have the USC run past its output registers, which faults the
             * GPU -- unless the shader stored it to a tile buffer, which the
             * end-of-tile program loads back from dword 0. */
            let regs = if irt == 1 { 2 } else { 1 };
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
            glog!("pvr_backend: the frame's end-of-tile pixel event program was refused: frame dropped");
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
     * core has. PVR_BACKEND_OPT_MRT_STRIDE overrides the width for bring-up
     * (its value - 1, in the field's two bits). */
    b.kick.frag.usc_pixel_output_ctrl = usc_pixel_output_ctrl(if b.opt(PVR_BACKEND_OPT_MRT_STRIDE) != 0 {
        (b.opt(PVR_BACKEND_OPT_MRT_STRIDE) - 1) & 3
    } else {
        csb::ROGUE_CR_PIXEL_WIDTH_4REGISTERS
    });
    /* The USC clear register is what the tiles start from (the background
     * object only runs on empty tiles); a frame that keeps its colour drew
     * the load quad first, so the register value does not matter then. */
    b.kick.frag.usc_clear_register[0] = packed[0];
    /* Several 8-bit attachments ([`pvr_mrt_clear`]): each output register
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
        /* apart (pvr_split_zs): the stencil's own buffer, fetched from this
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
     * depth/stencil arm above */
    // SAFETY: a plain read of an int only the setter writes.
    let dbias_is_int = b.frame_ds.buf().is_some() && unsafe { core::ptr::addr_of!(pvr_dbias_int).read() } != 0;
    b.kick.frag.isp_ctl = isp_ctl(b.samples, dbias_is_int);
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
        /* PBE register words per emitted state (its mrt_index), from the
         * attachment slot that state is for */
        let fmt = if slot != 0 { b.targets[slot].format } else { b.target_format };
        let tw = if slot != 0 { b.targets[slot].w } else { b.target_w };
        let th = if slot != 0 { b.targets[slot].h } else { b.target_h };
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
        _ if b.target_format == GLES_SURF_RGBA16F => b.bg_kick2_off,
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

    let Some(submit) = osal::ops().and_then(|o| o.submit) else {
        return -ENODEV; /* pvr_backend_init() checked it */
    };
    // SAFETY: the OSAL's submit takes the kick descriptor and its size.
    let fence = unsafe { submit(&b.kick as *const PvrRenderKick as *const c_void, size_of::<PvrRenderKick>()) };
    if fence < 0 {
        glog!("pvr_backend: submit failed ({})", fence as i32);
        return fence as i32;
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
            if b.frame_ds.buf().is_some() { ", ds" } else { "" }
        );
    }
    ds_settle(b);
    if b.swap_drops_zs {
        /* PVR_BACKEND_OPT_SWAP_DROPS_ZS: this kick stored nothing, so the
         * buffer holds no plane GL keeps (EGL 1.5 3.10.1) */
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
/// With [`PVR_BACKEND_OPT_SWAP_DROPS_ZS`] on (the default) and the open
/// frame rendering into that surface with a depth/stencil buffer, the flush
/// that follows stores no depth/stencil. Nothing else changes, and nothing
/// at all with the option off.
#[no_mangle]
pub extern "C" fn pvr_backend_surface_swapped(surface_gpu: u64) {
    // SAFETY: single-threaded by contract; taken once per entry point.
    let b = unsafe { &mut PVR.get().b };
    // SAFETY: a plain read of an int only the setter writes.
    if !b.inited || unsafe { core::ptr::addr_of!(pvr_swap_drops_zs).read() } == 0 {
        return;
    }
    if b.frame_open && b.target_gpu == surface_gpu && b.frame_ds.buf().is_some() {
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
        glog!("pvr_backend: one kick in flight: the swap's own kick {} did not complete ({})", target, rc);
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
/// (driver/docs/pvr-render-pool.md, mechanism 2). The OSAL uses it under
/// `CONFIG_YAKOGL_PVR_POOL_RECLAIM`; it tags every free `last_submitted + 1`
/// otherwise.
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
/// (`CONFIG_YAKOGL_PVR_POOL_OWNERS`, driver/docs/pvr-render-pool.md): the
/// frame sets with their arenas' high-water marks, the persistent programs
/// and tables, and the depth/stencil buffers. Returns their KiB.
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
    for m in
        [&b.clear_fs, &b.clear_fs2, &b.clear_fs3, &b.fbload_fs, &b.fbload_vs, &b.fbload_quad, &b.black_tex, &b.seq_idx]
    {
        statics.add(m);
    }
    for m in b.fbload_fs_rt.iter().chain(b.clear_mask_fs.iter()) {
        statics.add(m);
    }
    let mut ds = OwnerTally::default();
    for s in &b.ds {
        ds.add(&s.mem);
    }
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
    /* the PDS fetches a per-instance array by the instance number: divisor 1 */
    max_instance_divisor: 1,
    /* the sync waits keep finish() until this backend waits for one kick */
    wait_kick: None,
    /* the texture unit reads the storage */
    samples_texel_copies: false,
    attachment_gone: Some(pvr_backend_attachment_gone),
    depth_resolve: Some(pvr_backend_depth_resolve),
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
const ISP_CTL_MSAA4: u32 = 0x8003_3000;
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
        assert_eq!(clear_uniform(GLES_SURF_RGBA8888, &red), [pack_rgba8(&red), 0]);
        assert_ne!(clear_uniform(GLES_SURF_RGBA8888, &red)[0], 1.0f32.to_bits(), "not the red channel's float");
        for surf in [GLES_SURF_RGBA8888, GLES_SURF_XRGB8888, GLES_SURF_RGB565] {
            let c = [0.25, 0.5, 0.75, 1.0];
            assert_eq!(clear_uniform(surf, &c), [pack_rgba8(&c), 0], "surf {surf}");
        }
        /* the float classes keep their encoding */
        let c = [1.0, 0.5, 2.0, 0.0];
        assert_eq!(clear_uniform(GLES_SURF_RGBA16F, &c), pack_clear_float(GLES_SURF_RGBA16F, &c));
        assert_eq!(clear_uniform(GLES_SURF_R16F, &c), pack_clear_float(GLES_SURF_R16F, &c));
        assert_eq!(clear_uniform(GLES_SURF_R32F, &c), [1.0f32.to_bits(), 0], "R32F is the red channel");
    }

    #[test]
    fn rgba8_clear_packing_clamps_the_unclamped_gl_value() {
        assert_eq!(pack_rgba8(&[-1.0, 2.0, 0.5, 1.0]), 0xFF80_FF00);
    }

    fn desc(t: &GlesTextureBind) -> [u32; TEX_DESC_DWORDS as usize] {
        desc_opt(t, false)
    }

    /// [`desc`] with PVR_BACKEND_OPT_FORCE_MIP0 on or off.
    fn desc_opt(t: &GlesTextureBind, force_mip0: bool) -> [u32; TEX_DESC_DWORDS as usize] {
        let mut w = [0u32; TEX_DESC_DWORDS as usize];
        build_tex_desc(&mut w, t, &mut 0, force_mip0);
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

    #[test]
    fn narrow_luminance_alpha_descriptors_take_u8_formats_swizzles_and_either_pack_info() {
        let base = GlesTextureBind {
            base_gpu: 0x80_0000_1000,
            width: 3,
            height: 12,
            stride_texels: 4,
            mip_levels: 1,
            ..GlesTextureBind::ZERO
        };
        for (mode, pck) in [(1, TEX_PCK_INFO_RGBA8_UNORM), (2, TEX_PCK_INFO_COMPRESSED_UNORM)] {
            let was = pvr_narrow_la(mode);
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
            pvr_narrow_la(was);
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
    fn force_mip0_samples_the_top_level_only() {
        let t = GlesTextureBind {
            format: GLES_TEXFMT_RGBA8,
            width: 64,
            height: 64,
            stride_texels: 64,
            mip_levels: 4,
            mip_filter: 2,
            ..GlesTextureBind::ZERO
        };
        let w = desc_opt(&t, true);

        assert_eq!(((w[3] as u64) << 32 | w[2] as u64) >> 60, 1, "one level");
        assert_eq!(((w[3] as u64) << 32 | w[2] as u64) & (1 << 15), 0, "no mipmaps_present");
        assert_eq!(desc_maxlod(&w), 0);
        assert_eq!(w[9] & TEX_SAMPLER_MIP_LINEAR_HI, 0);
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
