// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2023 Imagination Technologies Ltd.
// Derived from Mesa 26.2.1 src/imagination (MIT): the tiling constants of
// common/device_info/bxs-4-64.h, the macrotile arithmetic after
// pvr_arch_rt_mtile_info_init, the sample positions after
// pvr_rogue_get_cr_multisamplectl_val. The upstream part stays under its
// licence (LICENSES/MIT.txt); our code is Apache-2.0. See NOTICE.
//! The render service's firmware side: the render context, the per-size
//! render-target datasets (HWRT data), the client CCBs, the submit of one
//! geometry + fragment render, and the wait for its fence.
//!
//! The structures are the DDK's (`rgx_fwif_km.h`, `rgx_fwif_shared.h`:
//! `RGXFWIF_FWMEMCONTEXT`, `RGXFWIF_FWRENDERCONTEXT`, `RGXFWIF_CCCB_CTL`,
//! `RGXFWIF_HWRTDATA[_COMMON]`, `RGXFWIF_CCB_CMD_HEADER`, `RGXFWIF_UFO`, the
//! KCCB kick, MMU-cache and cleanup commands; offsets in [`super::fwif`],
//! pinned). The sequences follow the DDK's `rgxta3d.c` (render context,
//! HWRT data, the combined kick) and `rgxmem.c` (the pre-kick MMU cache
//! command). The client commands past `CMDTA3D_SHARED` (the dword indices of
//! the geometry and fragment commands below), their flags, the context IDs,
//! the deadline and `execute_count` are values from a DDK capture: the DDK's
//! user-mode client headers are not in the vendored kernel source and Mesa
//! 26.2.1 submits through drm/imagination, whose commands are the mainline
//! firmware's (docs/source-form.md group G).
//!
//! Before docs/source-form.md step 7 this was C in `driver/src/pvr_render.c`
//! (ours). What stays C there: the static PDS/USC areas (they call the
//! command-stream generators of group B), the parameter buffer and freelist
//! sizing, the render pool's API (its free lists are Rust: [`super::pool`]),
//! the run-time switches and the Zephyr-facing API. Same bytes,
//! cleans, barriers, register accesses and log lines as the C
//! (`tools/pvr/km`, scenario `render`).

use super::ccb::{fwccb_service_with, kccb_doorbell, kccb_post, kccb_probe_slot, put32, KccbCmd};
use super::fwif::*;
use super::kick_times::{self, YdPvrKickTimes};
use super::km::{km, YdPvrKm, FL_MAX};
use super::mem::{bo_alloc, bo_alloc_pm, heap_alloc, round_up, PvrGpuBo, L0_FLAGS_DATA};
use super::mmu;
use super::rgxlayer::read64;
use crate::errno;
use crate::km_log;
use crate::mmio::Mmio;
use core::ffi::{c_char, c_void, CStr};
use core::mem::{offset_of, size_of};

/// Client CCB ring size per data master (32 KiB; wrap mask = size - 1).
pub const CCCB_SIZE: u32 = 32 * 1024;
/// Firmware objects get their own 128-byte-aligned, 128-byte-padded block,
/// so no firmware cache line is shared between two objects (the DDK
/// allocates every such object as a separate PMR).
const FW_LINE: u32 = 128;
/// One geometry UFO word per in-flight kick, indexed by the kick sequence.
pub const GEOM_UFO_SLOTS: u32 = 16;
/// Two PBE register words per colour attachment in this firmware's 3D
/// command, four dwords apart (measured on the board).
const PBE_STRIDE_DW: usize = 4;
/// How many colour attachments one render can write: the PBE words above
/// cover it, and it is the size of the partial render's RTA arrays
/// (`sRTACtl.ui32MaxRTs`).
pub const MAX_RT: usize = 3;
/// Render-target tiling constants for this core (Mesa bxs-4-64.h): 16x16
/// tiles, simple internal parameter format v2, 4x4 macrotiles.
const TILE_SIZE: u32 = 16;
const NUM_MACROTILES: u32 = 16;
const MACROTILES_XY: u32 = 4;
/// One region header per 2x2 tile group (SIPF v2).
const RGN_HEADER_BYTES: u32 = 6;
const TAIL_POINTER_SZ: u32 = 8;
/// The VHeap table's VA slot (value from DDK capture).
const VHEAP_BYTES: u32 = 0x2000;
const MLIST_BYTES: u32 = 0x1000;
/// Render-target datasets, each kept for one target size ([`hwrt_get`]).
pub const HWRT_SETS: usize = 32;
/// The most datasets one target size takes: as many renders of one size as
/// can be in flight at once. The GL back end records at most its frame sets
/// ahead of the GPU (src/pvr/backend.rs `PVR_BACKEND_FRAME_SETS`, 4), so a
/// size with four members never waits for one of its own renders before the
/// back end's frame-set reuse would wait anyway. With two (the pair this
/// replaced) a run of kicks of one size -- the levels of a mipmap chain
/// (`mip_chain_gpu`: glmark2 terrain asks for four chains of 8 and 9 levels
/// a frame) -- waited at every kick from the third for the one two before
/// it, a wait that ends on a whole millisecond without `pvr-fine-wait`
/// (#42).
pub const HWRT_MEMBERS: usize = 4;
/// The FW trace tail a failed frame dumps (a serial console cannot take more).
const TRACE_DUMP_DWORDS: u32 = 240;
/// RGXFWIF_CMDTA flags (value from DDK capture).
const GEOM_FLAGS_FIRSTKICK: u32 = 0x1;
const GEOM_FLAGS_LASTKICK: u32 = 0x2;
/// RGXFWIF_FWCOMMONCONTEXT.ui32MaxDeadlineMS: the DDK server's workload
/// deadline (value from DDK capture).
const FW_MAX_DEADLINE_MS: u32 = 40000;
/// Each command occupies a whole number of 16-byte units (the stricter of
/// the mainline header's "must be aligned to 16 bytes" and the DDK's 8).
const CCB_CMD_ALIGN: u32 = 16;

pub(super) fn geom_ufo_slot(seq: u32) -> u32 {
    seq & (GEOM_UFO_SLOTS - 1)
}

// ---- the shared structs ---------------------------------------------------------

/// `struct pvr_render_kick` (driver/src/pvr_render.h).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PvrRenderKick {
    pub width: u32,
    pub height: u32,
    pub samples: u32,
    pub geom: PvrRenderGeomRegs,
    pub frag: PvrRenderFragRegs,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PvrRenderGeomRegs {
    pub vdm_ctrl_stream_base: u64,
    pub ppp_ctrl: u32,
    pub te_psg: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PvrRenderFragRegs {
    pub usc_pixel_output_ctrl: u32,
    pub usc_clear_register: [u32; 4],
    pub isp_bgobjdepth: u32,
    pub isp_bgobjvals: u32,
    pub isp_aa: u32,
    pub isp_ctl: u32,
    pub event_pixel_pds_info: u32,
    pub event_pixel_pds_data: u32,
    pub pbe_word: [[u64; 2]; MAX_RT],
    pub num_rt: u32,
    pub pds_bgnd: [u64; 3],
    pub isp_scissor_base: u64,
    pub isp_dbias_base: u64,
    pub isp_zlsctl: u64,
    pub isp_zload_store_base: u64,
    pub isp_stencil_load_store_base: u64,
    pub cmd_flags: u32,
    /// CR_ISP_OCLQRY_BASE: the visibility-test counters (RGXFWIF_CMD3D
    /// dwords 22-23), 0 when no object of the render tests.
    pub isp_oclqry_base: u64,
}

const _: () = {
    assert!(size_of::<PvrRenderKick>() == 216);
    assert!(offset_of!(PvrRenderKick, geom) == 16);
    assert!(offset_of!(PvrRenderKick, frag) == 32);
    assert!(offset_of!(PvrRenderKick, frag) + offset_of!(PvrRenderFragRegs, pbe_word) == 80);
    assert!(offset_of!(PvrRenderKick, frag) + offset_of!(PvrRenderFragRegs, num_rt) == 128);
    assert!(offset_of!(PvrRenderKick, frag) + offset_of!(PvrRenderFragRegs, pds_bgnd) == 136);
    assert!(offset_of!(PvrRenderKick, frag) + offset_of!(PvrRenderFragRegs, isp_scissor_base) == 160);
    assert!(offset_of!(PvrRenderKick, frag) + offset_of!(PvrRenderFragRegs, cmd_flags) == 200);
    assert!(offset_of!(PvrRenderKick, frag) + offset_of!(PvrRenderFragRegs, isp_oclqry_base) == 208);
};

/// `struct yd_pvr_hwrt`: one render-target dataset (an HWRT data and its
/// buffers).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct YdPvrHwrt {
    pub used: u32,
    pub w: u32,
    pub h: u32,
    pub samples: u32,
    pub hcmn_off: u32,
    pub hrt_off: u32,
    pub hrt_fwaddr: u32,
    /// The partial render's render-target arrays, firmware-heap offsets
    /// (`partial_render`; the HWRT data's sRTACtl points at them,
    /// `hwrt_get`). 0 when the knob never built them.
    pub rta_valid_off: u32,
    pub tpc: PvrGpuBo,
    pub vheap: PvrGpuBo,
    pub rgnhdr: PvrGpuBo,
    pub macrotile: PvrGpuBo,
    /// The PM's memory list of this RT data: one per RT data (Mesa
    /// pvr_rt_mlist_data_init allocates mlist_size * num_rt_datas): kick
    /// N+1's geometry appends to its list while kick N's fragment phase still
    /// releases through the other.
    pub mlist: PvrGpuBo,
    /// Bytes behind tpc / rgnhdr (reused on a recycle).
    pub tpc_alloc: u32,
    pub rgn_alloc: u32,
    /// The HWRT data structures are allocated.
    pub fw_built: u32,
    /// The last kick that used this RT data.
    pub last_seq: u32,
    /// Which member of the pair the next render of this size takes (kept on
    /// the even slot).
    pub next: u32,
    /// The partial render's per-render-target partial-render counts array
    /// (`rta_valid_off`'s partner).
    pub rta_partial_off: u32,
}

/// `struct yd_pvr_render_cfg`: the render service's run-time switches
/// (driver/src/pvr_render.c has what each one is for), 0 or 1 but
/// `mmu_ack_reping` (ms).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct YdPvrRenderCfg {
    pub combined_kick: u32,
    pub wait_rering: u32,
    pub wait_repost: u32,
    pub mmu_ack: u32,
    pub mmu_gate: u32,
    pub mmu_ack_probe: u32,
    pub mmu_ack_reping: u32,
    pub hwrt_refuse: u32,
    pub hwrt_cleanup: u32,
    pub hwrt_reinit: u32,
    /// PVR_RENDER_VERBOSE: dumps after every frame and while a kick runs.
    pub verbose: u32,
    /// Serve a freelist grow the reserve and the shared grow pool cannot
    /// serve with pages from the render pool, when the firmware asks
    /// (`CONFIG_YAKOGL_DRV_GPU_PVR_FL_GROW_ON_DEMAND`; the C then builds the
    /// lists with no reserve).
    pub fl_grow_on_demand: u32,
    /// Hand the firmware its partial render: the geometry command's PR fence
    /// words and, per kick, a FENCE_PR, a 3D_PR (a copy of the 3D command)
    /// and an UPDATE on a geometry timeline word
    /// (`CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER`; docs/notes/pvr-render-pool.md,
    /// "Design: the firmware's partial render"). Without it every command
    /// and word is what it was without the knob.
    pub partial_render: u32,
}

/// `struct yd_pvr_render_ops`: what the render side borrows from the C.
#[repr(C)]
pub struct YdPvrRenderOps {
    pub ctx: *mut c_void,
    /// Give a render-pool block back (the pool's free lists, C).
    pub free_bo: unsafe extern "C" fn(ctx: *mut c_void, bo: *const PvrGpuBo, size: u32),
}

/// `struct yd_pvr_render`: the render service's firmware-side state, owned
/// by driver/src/pvr_render.c.
#[repr(C)]
pub struct YdPvrRender {
    pub cfg: YdPvrRenderCfg,
    pub ops: *const YdPvrRenderOps,
    pub inited: u32,
    /// The general heap's device-VA cursor (shared with the C allocations).
    pub gen_b: u32,
    /// The freelists (FW heap offsets of their RGXFWIF_FREELIST), set by the C.
    pub gfl_off: u32,
    pub gfl2_off: u32,
    pub fl_off: u32,
    /// The render context and its client CCBs (FW heap offsets).
    pub fmc_off: u32,
    pub rc_off: u32,
    pub gcccb_off: u32,
    pub gcccbctl_off: u32,
    pub gstate_off: u32,
    pub gufo_off: u32,
    pub fcccb_off: u32,
    pub fcccbctl_off: u32,
    pub fstate_off: u32,
    pub fufo_off: u32,
    /// The client CCB write offsets (bytes, < CCCB_SIZE).
    pub gwo: u32,
    pub fwo: u32,
    pub mc_sync_off: u32,
    pub mc_seq: u32,
    /// The completion fence sequence.
    pub seq: u32,
    /// The firmware's HWR counter at the last successful wait.
    pub hwr_seen: u32,
    /// The dataset the last kick used (-1: none).
    pub last_hs: i32,
    /// The last kick's VDM stream base (diagnostics).
    pub dbg_vdm: u64,
    pub vdm_cs: PvrGpuBo,
    /// Kicks re-posted by the wait (the mitigation, see `wait_for`).
    pub reposts: u32,
    /// How often the reuse wait found the pair member still in flight.
    pub hwrt_busy_reuse: u32,
    pub stall_dumped: u32,
    pub stall_dumped_after_refusal: u32,
    /// The MMUCACHE probes' state.
    pub mc_unseen: u32,
    pub mc_io_armed: u32,
    pub mc_io_want: u32,
    pub mc_io_slot: u32,
    /// The last kick(s) posted, for the re-post.
    pub last_kick_n: u32,
    pub last_frag_seq: u32,
    /// The last fragment command (dumped on a timeout or a HWR).
    pub last_frag_cmd: [u32; 80],
    pub last_kick: [KccbCmd; 2],
    pub hwrt: [YdPvrHwrt; HWRT_SETS],
    /// Per-kick timestamps (feature `pvr-kick-times`; zero without it).
    pub kick_times: YdPvrKickTimes,
    /// The application's scene mark (feature `pvr-grow-scene`; zero
    /// without it).
    pub grow_scene: YdPvrGrowScene,
}

/// `struct yd_pvr_grow_scene`: where the application's current scene began
/// ([`yd_pvr_render_scene_begin`], feature `pvr-grow-scene`), for the grow
/// line's scene context.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct YdPvrGrowScene {
    /// Scenes marked so far (0: none, and the grow line says nothing more).
    pub n: u32,
    /// The last kick submitted when the scene was marked.
    pub seq0: u32,
    /// The clock at the mark (`kick_times::clock` ticks).
    pub t0: u64,
}

const _: () = {
    assert!(size_of::<YdPvrHwrt>() == 176);
    assert!(offset_of!(YdPvrHwrt, tpc) == 32);
    assert!(offset_of!(YdPvrHwrt, mlist) == 128);
    assert!(offset_of!(YdPvrHwrt, tpc_alloc) == 152);
    assert!(offset_of!(YdPvrHwrt, next) == 168);
    // cfg is 52 bytes, so `ops` (a pointer) starts at 56 with the 4 bytes of
    // alignment after it on any target this crate is built for (lp64).
    assert!(size_of::<YdPvrRenderCfg>() == 52);
    assert!(size_of::<YdPvrRenderOps>() == 16);
    assert!(offset_of!(YdPvrRender, ops) == 56);
    assert!(offset_of!(YdPvrRender, inited) == 64);
    assert!(offset_of!(YdPvrRender, gen_b) == 68);
    assert!(offset_of!(YdPvrRender, gfl_off) == 72);
    assert!(offset_of!(YdPvrRender, last_hs) == 148);
    assert!(offset_of!(YdPvrRender, dbg_vdm) == 152);
    assert!(offset_of!(YdPvrRender, vdm_cs) == 160);
    assert!(offset_of!(YdPvrRender, reposts) == 184);
    assert!(offset_of!(YdPvrRender, last_kick_n) == 216);
    assert!(offset_of!(YdPvrRender, last_frag_cmd) == 224);
    assert!(offset_of!(YdPvrRender, last_kick) == 544);
    assert!(offset_of!(YdPvrRender, hwrt) == 864);
    assert!(offset_of!(YdPvrRender, kick_times) == 864 + 176 * HWRT_SETS);
    assert!(offset_of!(YdPvrRender, grow_scene) == 864 + 176 * HWRT_SETS + size_of::<YdPvrKickTimes>());
    assert!(size_of::<YdPvrGrowScene>() == 16);
    assert!(size_of::<YdPvrRender>() == 864 + 176 * HWRT_SETS + size_of::<YdPvrKickTimes>() + 16);
};

// ---- helpers ----------------------------------------------------------------------

fn fw_alloc(k: &mut YdPvrKm, size: u32) -> u32 {
    heap_alloc(k, round_up(u64::from(size), u64::from(FW_LINE)) as u32, FW_LINE)
}

/// A volatile word of shared memory at heap offset `off` (+ `at` bytes).
fn hrd(k: &YdPvrKm, off: u32, at: usize) -> u32 {
    // SAFETY: firmware-heap objects of this context.
    unsafe { rd32(k.heap_cpu(off), at) }
}
/// [`hrd`] for [`super::kick_times`].
#[cfg(feature = "pvr-kick-times")]
pub(super) fn read_heap32(k: &YdPvrKm, off: u32, at: usize) -> u32 {
    hrd(k, off, at)
}
fn hwr(k: &YdPvrKm, off: u32, at: usize, v: u32) {
    // SAFETY: as above.
    unsafe { wr32(k.heap_cpu(off), at, v) }
}

impl YdPvrKm {
    fn reg32(&self, off: u32) -> u32 {
        self.bus().read32(self.regs + off as usize)
    }
    fn reg64(&self, off: u32) -> u64 {
        read64(&self.bus(), self.regs + off as usize)
    }
}

/// The unit a BIF fault came from (rgxmhdefs_km.h MH tag encodings, the XE
/// numbering this core uses): [`MH_TAGS`] is indexed by the encoding.
pub const MH_TAGS: [(&str, &str); 56] = [
    ("MMU", "MMU"),
    ("CPU_MMU", "CPU MMU"),
    ("CPU_IFU", "CPU IFU"),
    ("CPU_LSU", "CPU LSU"),
    ("MIPS", "MIPS"),
    ("CDM_STG0", "CDM stage 0"),
    ("CDM_STG1", "CDM stage 1"),
    ("CDM_STG2", "CDM stage 2"),
    ("CDM_STG3", "CDM stage 3"),
    ("VDM_STG0", "VDM stage 0"),
    ("VDM_STG1", "VDM stage 1"),
    ("VDM_STG2", "VDM stage 2"),
    ("VDM_STG3", "VDM stage 3"),
    ("VDM_STG4", "VDM stage 4"),
    ("PDS_0", "PDS req 0"),
    ("PDS_1", "PDS req 1"),
    ("MCU_USCA", "MCU USCA"),
    ("MCU_USCB", "MCU USCB"),
    ("MCU_USCC", "MCU USCC"),
    ("MCU_USCD", "MCU USCD"),
    ("MCU_PDS_USCA", "MCU PDS USCA"),
    ("MCU_PDS_USCB", "MCU PDS USCB"),
    ("MCU_PDS_USCC", "MCU PDS USCC"),
    ("MCU_PDS_USCD", "MCU PDS USCD"),
    ("MCU_PDSRW", "MCU PDSRW"),
    ("TCU_0", "TCU 0"),
    ("TCU_1", "TCU 1"),
    ("FBCDC_0", "FBCDC 0"),
    ("FBCDC_1", "FBCDC 1"),
    ("FBCDC_2", "FBCDC 2"),
    ("FBCDC_3", "FBCDC 3"),
    ("USC", "USC"),
    ("ISP_ZLS", "ISP ZLS"),
    ("ISP_DS", "ISP DS"),
    ("TPF", "TPF"),
    ("TPF_PBCDBIAS", "TPF PBCDBIAS"),
    ("TPF_SPF", "TPF SPF"),
    ("IPF_CREQ", "IPF CREQ"),
    ("IPF_OTHERS", "IPF others"),
    ("VDM_STG5", "VDM stage 5"),
    ("TA_PPP", "TA PPP"),
    ("TA_TPWRTC", "TA TPWRTC"),
    ("TA_TEACRTC", "TA TEACRTC"),
    ("TA_PSGRTC", "TA PSGRTC"),
    ("TA_PSGREGION", "TA PSGREGION"),
    ("TA_PSGSTREAM", "TA PSGSTREAM"),
    ("TA_TPW", "TA TPW"),
    ("TA_TPC", "TA TPC"),
    ("PM_ALLOC", "PM alloc"),
    ("PM_DEALLOC", "PM dealloc"),
    ("TDM_DMA", "TDM DMA"),
    ("TDM_CTL", "TDM ctl"),
    ("PBE0", "PBE0"),
    ("PBE1", "PBE1"),
    ("PBE2", "PBE2"),
    ("PBE3", "PBE3"),
];

fn mh_tag_name(tag: u32) -> &'static str {
    MH_TAGS.get(tag as usize).map_or("?", |t| t.1)
}

/// The MH tag table for the host check: entry `i`'s encoding name suffix.
///
/// # Safety
/// `name`, `len` valid for writing.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_mh_tag(i: u32, name: *mut *const u8, len: *mut usize) -> bool {
    match MH_TAGS.get(i as usize) {
        Some(t) => {
            // SAFETY: the caller's contract.
            unsafe {
                *name = t.0.as_ptr();
                *len = t.0.len();
            }
            true
        }
        None => false,
    }
}

// ---- the render context -------------------------------------------------------------

/// The render context: the firmware memory context (-> the page catalogue),
/// the two common contexts (geometry, fragment) with their client CCBs and
/// context states, the VDM call stack, the UFO words and the MMU-cache sync
/// word. The freelists are built (`r.gfl_off` ...).
pub fn ctx_init(k: &mut YdPvrKm, r: &mut YdPvrRender) {
    let rc_sz = FWRENDERCONTEXT_SIZE as u32;

    r.fmc_off = fw_alloc(k, FWMEMCONTEXT_SIZE as u32);
    {
        let fmc = k.heap_cpu(r.fmc_off);
        // SAFETY: the memory context just allocated.
        unsafe {
            wr64(fmc, FWMEMCONTEXT_PC_DEV_PADDR, k.pc_phys());
            wr32(fmc, FWMEMCONTEXT_PAGE_CAT_BASE_REG_SET, 0xFFFF_FFFF);
        }
        k.flush(fmc, FWMEMCONTEXT_SIZE);
    }

    r.rc_off = fw_alloc(k, rc_sz);
    r.gcccb_off = fw_alloc(k, CCCB_SIZE);
    r.gcccbctl_off = fw_alloc(k, CCCB_CTL_SIZE as u32);
    r.gstate_off = fw_alloc(k, 1024);
    // One geometry UFO word per in-flight kick: the fragment phase waits on
    // it with the firmware's plain UFO check, an EQUALITY test, so a single
    // word that a geometry phase running ahead moves past the awaited value
    // would never match again. The word after the slots is the geometry
    // timeline of the partial render (`partial_render`, Option A of
    // docs/notes/pvr-render-pool.md, "Design: the firmware's partial
    // render"): monotonic, one UPDATE per geometry command, referenced only
    // by the PR fences, so the slot ring's wrap cannot move it away from the
    // value a pending partial render waits for.
    r.gufo_off = fw_alloc(k, 4 * GEOM_UFO_SLOTS + 4);
    r.fcccb_off = fw_alloc(k, CCCB_SIZE);
    r.fcccbctl_off = fw_alloc(k, CCCB_CTL_SIZE as u32);
    r.fstate_off = fw_alloc(k, 1024);
    r.fufo_off = fw_alloc(k, 8);
    r.mc_sync_off = fw_alloc(k, 4);

    hwr(k, r.gcccbctl_off, CCCB_CTL_WRAP_MASK, CCCB_SIZE - 1);
    hwr(k, r.fcccbctl_off, CCCB_CTL_WRAP_MASK, CCCB_SIZE - 1);
    k.flush(k.heap_cpu(r.gcccbctl_off), CCCB_CTL_SIZE);
    k.flush(k.heap_cpu(r.fcccbctl_off), CCCB_CTL_SIZE);

    // The VDM call stack the geometry context state points at.
    r.vdm_cs = bo_alloc(k, GENERAL_HEAP_BASE, &mut r.gen_b, 4096, L0_FLAGS_DATA);
    // SAFETY: the geometry context state (1024 bytes).
    unsafe { wr64(k.heap_cpu(r.gstate_off), TACTX_STATE_VDM_CALL_STACK_POINTER_INIT, r.vdm_cs.gpu) };
    k.flush(k.heap_cpu(r.gstate_off), 1024);

    // RGXFWIF_FWRENDERCONTEXT: the fields the firmware reads. HAS_DEFER_COUNT
    // and the deadline are required or the firmware defers the kick forever.
    // The geometry context-switch base registers stay 0 (the firmware
    // manages them itself on a first kick). Server context IDs 2 and 3 and
    // PID 1 are the values from the DDK capture.
    let ctxs = [
        (FWRENDERCONTEXT_TA_CONTEXT, r.gcccbctl_off, r.gcccb_off, r.gstate_off, 2u32, DM_GEOM),
        (FWRENDERCONTEXT_3D_CONTEXT, r.fcccbctl_off, r.fcccb_off, r.fstate_off, 3u32, DM_3D),
    ];
    for (at, ctl, ccb, state, id, dm) in ctxs {
        let c = k.heap_cpu(r.rc_off).wrapping_add(at);
        let (ctl_fa, ccb_fa, state_fa, fmc_fa) =
            (k.heap_fwaddr(ctl), k.heap_fwaddr(ccb), k.heap_fwaddr(state), k.heap_fwaddr(r.fmc_off));
        // SAFETY: the render context just allocated (912 bytes).
        unsafe {
            wr32(c, FWCOMMONCONTEXT_CCB_CTL, ctl_fa);
            wr32(c, FWCOMMONCONTEXT_CCB, ccb_fa);
            wr32(c, FWCOMMONCONTEXT_CONTEXT_STATE, state_fa);
            wr32(c, FWCOMMONCONTEXT_PRIORITY, 0);
            wr32(c, FWCOMMONCONTEXT_COMPAT_FLAGS, CONTEXT_COMPAT_FLAGS_HAS_DEFER_COUNT);
            wr32(c, FWCOMMONCONTEXT_MAX_DEADLINE_MS, FW_MAX_DEADLINE_MS);
            wr32(c, FWCOMMONCONTEXT_FW_MEM_CONTEXT, fmc_fa);
            wr32(c, FWCOMMONCONTEXT_SERVER_COMMON_CONTEXT_ID, id);
            wr32(c, FWCOMMONCONTEXT_PID, 1);
            wr32(c, FWCOMMONCONTEXT_RANGE_CHECK_DM, dm);
        }
    }
    k.flush(k.heap_cpu(r.rc_off), rc_sz as usize);

    // The slots and the timeline word: a zeroed timeline is below every seq,
    // so the first kick's partial-render fence simply waits.
    for i in 0..GEOM_UFO_SLOTS + 1 {
        hwr(k, r.gufo_off, (i * 4) as usize, 0);
    }
    hwr(k, r.fufo_off, 0, 0);
    hwr(k, r.mc_sync_off, 0, 0);
    k.flush(k.heap_cpu(r.gufo_off), (4 * GEOM_UFO_SLOTS + 4) as usize);
    k.flush(k.heap_cpu(r.fufo_off), 8);
    k.flush(k.heap_cpu(r.mc_sync_off), 4);
    r.gwo = 0;
    r.fwo = 0;
    r.seq = 0;
    r.mc_seq = 0;

    let hib = k.heap_cpu(k.hwrinfobuf_off);
    k.invd(hib, HWRINFOBUF_SIZE);
    r.hwr_seen = hrd(k, k.hwrinfobuf_off, HWRINFOBUF_HWR_COUNTER);
}

// ---- the render-target datasets -----------------------------------------------------

fn npot2(v: u32) -> u32 {
    let mut p = 1u32;
    while p < v {
        p <<= 1;
    }
    p
}

/// CR_PPP_MULTISAMPLECTL: sample positions in 1/16 pixel, x/y nibbles per
/// sample (Mesa pvr_rogue_get_cr_multisamplectl_val); the "flipped" word has
/// y mirrored (16 - y).
fn multisamplectl(samples: u32, flipped: bool) -> u64 {
    const SX: [[u8; 4]; 3] = [[8, 0, 0, 0], [12, 4, 0, 0], [6, 14, 2, 10]];
    const SY: [[u8; 4]; 3] = [[8, 0, 0, 0], [12, 4, 0, 0], [2, 6, 10, 14]];
    let idx = if samples == 4 {
        2
    } else if samples == 2 {
        1
    } else {
        0
    };
    let n = if samples != 0 { samples } else { 1 };
    let mut v = 0u64;
    for i in 0..(n.min(4) as usize) {
        let y = if flipped { 16u32.wrapping_sub(u32::from(SY[idx][i])) } else { u32::from(SY[idx][i]) };
        v |= u64::from(SX[idx][i]) << (8 * i);
        v |= u64::from(y & 0xF) << (8 * i + 4);
    }
    v
}

fn free_bo(r: &YdPvrRender, bo: &PvrGpuBo, size: u32) {
    // SAFETY: the C side's hook (required).
    let o = unsafe { &*r.ops };
    // SAFETY: as above; the hook reads the block for the call.
    unsafe { (o.free_bo)(o.ctx, bo, size) }
}

/// One CLEANUP command, the DDK's RGXScheduleCleanupCommand sequence: post
/// it, wait (100 ms) for the firmware's answer in its KCCB return slot, and
/// read CLEANUP_BUSY as "still in use". Returns 0 released, -EBUSY in use,
/// -ETIMEDOUT no answer; `answer` the raw slot value.
fn kccb_cleanup(k: &mut YdPvrKm, ty: u32, fwaddr: u32, answer: &mut u32) -> i32 {
    let mut c: KccbCmd = [0; KCCB_CMD_SIZE];
    put32(&mut c, KCCB_CMD_TYPE, KCCB_CMD_CLEANUP);
    put32(&mut c, KCCB_CLEANUP_TYPE, ty);
    // one union, one u32: which member is named is documentation
    put32(&mut c, KCCB_CLEANUP_DATA, fwaddr);
    let slot = kccb_post(k, &c); // clears the slot before the command lands
    let rtn = k.kccb_rtn_off;
    let mut v = KCCB_RTN_SLOT_NO_RESPONSE;
    for _ in 0..2000 {
        k.invd(k.heap_cpu(rtn).wrapping_add(slot as usize * 4), 4);
        v = hrd(k, rtn, slot as usize * 4);
        if v & KCCB_RTN_SLOT_CMD_EXECUTED != 0 {
            break;
        }
        k.busy_wait_us(50);
    }
    *answer = v;
    if v & KCCB_RTN_SLOT_CMD_EXECUTED == 0 {
        return -errno::ETIMEDOUT;
    }
    if v & KCCB_RTN_SLOT_CLEANUP_BUSY != 0 {
        -errno::EBUSY
    } else {
        0
    }
}

/// The dataset for a `w` x `h` target with `samples` samples (Mesa
/// pvr_arch_rt_mtile_info_init and friends, specialised to this core), built
/// or reused; `None` when none can be handed out. Built in pairs, used
/// alternately (the DDK's RGXFWIF_NUM_RTDATAS): the firmware runs kick N+1's
/// geometry while kick N's fragment phase still reads its region headers. A
/// size whose members are all in flight takes a second pair while a pair is
/// free, up to [`HWRT_MEMBERS`], rather than wait (Mesa's pvr has a dataset
/// per framebuffer, so several of one size are the firmware's ordinary
/// case); only then does a render wait for a member of its size.
pub fn hwrt_get(k: &mut YdPvrKm, r: &mut YdPvrRender, w: u32, h: u32, samples: u32) -> Option<usize> {
    // samples spread over x/y (Mesa pvr_get_samples_in_xy): the region
    // headers and tail pointers cover tiles of samples, not pixels
    let sx = if samples == 4 { 2 } else { 1 };
    let sy = if samples >= 2 { 2 } else { 1 };
    let samples = if samples == 0 { 1 } else { samples };
    let mut hs: Option<usize> = None;
    // The size's pair whose next member's last kick is the oldest, while
    // every member seen is in flight: (pair, member, its last kick).
    let mut busy: Option<(usize, usize, u32)> = None;
    let mut pairs = 0usize;
    let mut second = false;

    let mut i = 0;
    while i + 1 < HWRT_SETS {
        let p = &r.hwrt[i];
        if p.used != 0 && p.w == w && p.h == h && p.samples == samples {
            if r.hwrt[i + 1].used == 0 {
                hs = Some(i + 1); // second member still to build
                second = true;
                break;
            }
            // The pair alternates per render OF THIS SIZE: a frame that
            // interleaves sizes advances the global sequence by more than one
            // between two renders of the same size. A member is handed out
            // again once its last kick has completed.
            pairs += 1;
            let pick = i + (r.hwrt[i].next & 1) as usize;
            let last = r.hwrt[pick].last_seq;
            if last == 0 || completed(k, r) >= last {
                r.hwrt[i].next ^= 1;
                return Some(pick);
            }
            if busy.is_none_or(|(_, _, l)| last < l) {
                busy = Some((i, pick, last));
            }
        } else if hs.is_none() && p.used == 0 {
            hs = Some(i);
        }
        i += 2;
    }
    // Every member of the size seen is in flight. Another pair while one is
    // free and the size stays within HWRT_MEMBERS (built below: the new
    // pair's first member); else wait for the oldest member's last kick
    // before handing it out again. A pair whose second member is still to
    // build is built instead.
    if let (false, Some((pair, pick, last))) = (second, busy) {
        if hs.is_none() || 2 * (pairs + 1) > HWRT_MEMBERS {
            r.hwrt[pair].next ^= 1;
            let wrc = wait_for(k, r, last, 20000, "HWRT member in flight");
            if wrc != 0 {
                r.hwrt_busy_reuse += 1;
                km_log!(
                    k,
                    Err,
                    "render: {}x{} set {} still busy at seq {} ({}){}",
                    w,
                    h,
                    pick,
                    last,
                    wrc,
                    if r.cfg.hwrt_refuse != 0 { ": refusing to reuse it" } else { ": handing it out anyway" }
                );
                if r.cfg.hwrt_refuse != 0 {
                    return None;
                }
            }
            return Some(pick);
        }
    }
    let hs = match hs {
        Some(x) => x,
        None => {
            // Every pair is taken: recycle the least recently kicked pair,
            // once its last kick is complete (the firmware reads the RT data
            // until its fragment phase ends).
            let mut victim = 0usize;
            let mut oldest = u32::MAX;
            let mut i = 0;
            while i + 1 < HWRT_SETS {
                let last = r.hwrt[i].last_seq.max(r.hwrt[i + 1].last_seq);
                if last < oldest {
                    oldest = last;
                    victim = i;
                }
                i += 2;
            }
            if oldest != 0 && wait_for(k, r, oldest, 2000, "HWRT pair recycle") != 0 {
                km_log!(k, Err, "render: no free HWRT slot for {}x{} (pair {} busy at seq {})", w, h, victim, oldest);
                return None;
            }
            // Is the firmware finished with the pair? The DDK tracks a
            // per-HWRT lifetime in cleanup_state; logged here, acted on only
            // with hwrt_cleanup.
            for m in 0..2 {
                let v = r.hwrt[victim + m];
                if v.fw_built != 0 {
                    k.invd(k.heap_cpu(v.hrt_off), HWRTDATA_SIZE);
                    km_log!(
                        k,
                        Inf,
                        "render: HWRT recycle: pair {} member {} state={} cleanup sub/exec={}/{}",
                        victim,
                        m,
                        hrd(k, v.hrt_off, HWRTDATA_STATE),
                        hrd(k, v.hrt_off, HWRTDATA_CLEANUP_SUBMITTED),
                        hrd(k, v.hrt_off, HWRTDATA_CLEANUP_EXECUTED)
                    );
                }
            }
            // The DDK's sequence before the memory is reused: CLEANUP the two
            // HWRT datas -- and only those, not the freelists (they are
            // separate objects with their own lifetime, shared by every
            // pair) -- and refuse the recycle on a "busy".
            if r.cfg.hwrt_cleanup != 0 {
                let (mut ans0, mut ans1) = (0u32, 0u32);
                let (mut rc0, mut rc1) = (0i32, 0i32);
                if r.hwrt[victim].fw_built != 0 {
                    let fa = r.hwrt[victim].hrt_fwaddr;
                    rc0 = kccb_cleanup(k, CLEANUP_HWRTDATA, fa, &mut ans0);
                }
                if r.hwrt[victim + 1].fw_built != 0 {
                    let fa = r.hwrt[victim + 1].hrt_fwaddr;
                    rc1 = kccb_cleanup(k, CLEANUP_HWRTDATA, fa, &mut ans1);
                }
                km_log!(
                    k,
                    Inf,
                    "render: HWRT cleanup: pair {} answers {:08x}/{:08x} ({}/{})",
                    victim,
                    ans0,
                    ans1,
                    rc0,
                    rc1
                );
                if rc0 != 0 || rc1 != 0 {
                    km_log!(
                        k,
                        Err,
                        "render: HWRT pair {} refused for {}x{}: firmware {}",
                        victim,
                        w,
                        h,
                        if rc0 == -errno::EBUSY || rc1 == -errno::EBUSY { "says busy" } else { "did not answer" }
                    );
                    return None;
                }
            }
            km_log!(
                k,
                Inf,
                "render: HWRT pair {} ({}x{}) recycled for {}x{}",
                victim,
                r.hwrt[victim].w,
                r.hwrt[victim].h,
                w,
                h
            );
            r.hwrt[victim].used = 0;
            r.hwrt[victim + 1].used = 0;
            victim
        }
    };

    let ntx = w.div_ceil(TILE_SIZE);
    let nty = h.div_ceil(TILE_SIZE);
    let mtx1 = ntx.div_ceil(8) * 2;
    let mty1 = nty.div_ceil(8) * 2;
    let xmax = round_up(u64::from(ntx), 2) as u32 - 1;
    let ymax = round_up(u64::from(nty), 2) as u32 - 1;
    let tiles_x = MACROTILES_XY * mtx1 * sx;
    let tiles_y = MACROTILES_XY * mty1 * sy;
    let rgn_count = mtx1 * sx * mty1 * sy * NUM_MACROTILES / 4;
    let rgn_bytes = round_up(u64::from((tiles_x / 2) * (tiles_y / 2) * RGN_HEADER_BYTES), 64) as u32;
    let m = npot2(tiles_x).max(npot2(tiles_y));
    let tpc_bytes = m * m / 4 * TAIL_POINTER_SZ;

    // The buffers: the self-test's minimums (they rendered), grown from the
    // formulas for bigger targets. A recycled set keeps buffers that are big
    // enough; a too-small one goes back to the pool's free list (the set is
    // not in flight here: the waits above saw to that).
    {
        let tpc_alloc = if tpc_bytes * 2 > 0x4000 { round_up(u64::from(tpc_bytes * 2), 4096) as u32 } else { 0x4000 };
        let rgn_alloc = if rgn_bytes * 4 > 0x10000 { round_up(u64::from(rgn_bytes * 4), 4096) as u32 } else { 0x10000 };
        if r.hwrt[hs].tpc.cpu.is_null() || r.hwrt[hs].tpc_alloc < tpc_alloc {
            if !r.hwrt[hs].tpc.cpu.is_null() {
                free_bo(r, &r.hwrt[hs].tpc, r.hwrt[hs].tpc_alloc);
            }
            r.hwrt[hs].tpc = bo_alloc(k, GENERAL_HEAP_BASE, &mut r.gen_b, tpc_alloc, L0_FLAGS_DATA);
            r.hwrt[hs].tpc_alloc = tpc_alloc;
        }
        if r.hwrt[hs].vheap.cpu.is_null() {
            r.hwrt[hs].vheap = bo_alloc(k, GENERAL_HEAP_BASE, &mut r.gen_b, VHEAP_BYTES, L0_FLAGS_DATA);
        }
        if r.hwrt[hs].rgnhdr.cpu.is_null() || r.hwrt[hs].rgn_alloc < rgn_alloc {
            if !r.hwrt[hs].rgnhdr.cpu.is_null() {
                free_bo(r, &r.hwrt[hs].rgnhdr, r.hwrt[hs].rgn_alloc);
            }
            r.hwrt[hs].rgnhdr = bo_alloc(k, GENERAL_HEAP_BASE, &mut r.gen_b, rgn_alloc, L0_FLAGS_DATA);
            r.hwrt[hs].rgn_alloc = rgn_alloc;
        }
        if r.hwrt[hs].macrotile.cpu.is_null() {
            r.hwrt[hs].macrotile = bo_alloc(k, GENERAL_HEAP_BASE, &mut r.gen_b, 4096, L0_FLAGS_DATA);
        }
        if r.hwrt[hs].mlist.cpu.is_null() {
            let mut pm = k.pm_bump;
            r.hwrt[hs].mlist = bo_alloc_pm(k, PMMETA_PROTECT_HEAP_BASE, &mut pm, MLIST_BYTES);
            k.pm_bump = pm;
        }
    }
    {
        let s = &r.hwrt[hs];
        if s.tpc.cpu.is_null()
            || s.vheap.cpu.is_null()
            || s.rgnhdr.cpu.is_null()
            || s.macrotile.cpu.is_null()
            || s.mlist.cpu.is_null()
        {
            km_log!(k, Err, "render: no pool memory for the {}x{} x{} HWRT", w, h, samples);
            return None;
        }
    }

    if r.hwrt[hs].fw_built == 0 {
        r.hwrt[hs].hcmn_off = fw_alloc(k, HWRTDATA_COMMON_SIZE as u32);
    }
    let hc_off = r.hwrt[hs].hcmn_off;
    let hc = k.heap_cpu(hc_off);
    let te_screen = (xmax & 0xFFF) | ((ymax & 0xFFF) << 12);
    let teaa = (if samples >= 2 { 0x1 } else { 0 }) | (if samples >= 4 { 0x8 } else { 0 });
    // SAFETY: the HWRT data common structure (88 bytes).
    unsafe {
        wr32(hc, HWRTDATA_COMMON_SCREEN_PIXEL_MAX, (w.wrapping_sub(1) & 0xFFFF) | ((h.wrapping_sub(1) & 0xFFFF) << 16));
        wr32(hc, HWRTDATA_COMMON_TE_SCREEN, te_screen);
        wr32(hc, HWRTDATA_COMMON_TE_MTILE1, mtx1 << 18);
        wr32(hc, HWRTDATA_COMMON_TE_MTILE2, mty1 << 18);
        wr32(hc, HWRTDATA_COMMON_ISP_MTILE_SIZE, (mtx1 << 16) | mty1);
        wr32(hc, HWRTDATA_COMMON_MTILE_STRIDE, mtx1 * mty1);
        // CR_TE_AA for an ISP with 4 samples per pixel (Mesa
        // pvr_rogue_ct_te_init): x2 [0] from 2 samples, y2 [3] from 4.
        wr32(hc, HWRTDATA_COMMON_TE_AA, teaa);
        wr64(hc, HWRTDATA_COMMON_MULTI_SAMPLE_CTL, multisamplectl(samples, false));
        wr64(hc, HWRTDATA_COMMON_FLIPPED_MULTI_SAMPLE_CTL, multisamplectl(samples, true));
        wr32(hc, HWRTDATA_COMMON_TPC_STRIDE, 0);
        wr32(hc, HWRTDATA_COMMON_TPC_SIZE, tpc_bytes);
        wr32(hc, HWRTDATA_COMMON_RGN_HEADER_SIZE, rgn_count);
        hc.add(HWRTDATA_COMMON_TA_CACHES_NEED_ZEROING).write_volatile(0);
    }
    k.flush(hc, HWRTDATA_COMMON_SIZE);

    if r.hwrt[hs].fw_built == 0 {
        r.hwrt[hs].hrt_off = fw_alloc(k, HWRTDATA_SIZE as u32);
    }
    r.hwrt[hs].fw_built = 1;
    let hrt_off = r.hwrt[hs].hrt_off;
    let hrt = k.heap_cpu(hrt_off);
    // The DDK builds this struct from a zero buffer every time it creates one
    // (RGXCreateHWRTData_aux); we set our fields and leave the rest, which on
    // a first build is the same thing and on a recycle is not, unless
    // hwrt_reinit.
    if r.cfg.hwrt_reinit != 0 {
        // SAFETY: the HWRT data (256 bytes).
        unsafe { core::ptr::write_bytes(hrt, 0, HWRTDATA_SIZE) };
    }
    // The partial render's render-target accounting (knob only, so with the
    // knob off these words stay what they were, new and recycled). Two
    // dword arrays the HWRT data's sRTACtl points at: the valid render
    // target indices, and the partial renders counted per render target
    // (RGXFWIF_RTA_CTL, rgx_fwif_shared.h:1218-1235; the firmware names a
    // null ValidRenderTargets an invalid setup, rgx_fwif_sf.h:569). The
    // arrays hold MAX_RT (PVR_RENDER_MAX_RT) dwords: the colour attachments
    // one render can write, what sRTACtl.ui32MaxRTs names.
    let pr = r.cfg.partial_render != 0;
    if pr {
        let s = &mut r.hwrt[hs];
        for off in [&mut s.rta_valid_off, &mut s.rta_partial_off] {
            if *off == 0 {
                *off = fw_alloc(k, 4 * MAX_RT as u32);
                // SAFETY: the MAX_RT dwords just allocated.
                unsafe { core::ptr::write_bytes(k.heap_cpu(*off), 0, 4 * MAX_RT) };
                k.flush(k.heap_cpu(*off), 4 * MAX_RT);
            }
        }
        // Valid render targets: the index array. The partial-render counts
        // stay zero; the firmware counts them.
        let v = k.heap_cpu(s.rta_valid_off);
        // SAFETY: MAX_RT dwords.
        unsafe {
            for i in 0..MAX_RT {
                wr32(v, 4 * i, i as u32);
            }
        }
        k.flush(v, 4 * MAX_RT);
    }
    let (hc_fa, fl_fa, gfl_fa, gfl2_fa) =
        (k.heap_fwaddr(hc_off), k.heap_fwaddr(r.fl_off), k.heap_fwaddr(r.gfl_off), k.heap_fwaddr(r.gfl2_off));
    let s = r.hwrt[hs];
    // SAFETY: as above.
    unsafe {
        wr32(hrt, HWRTDATA_COMMON_FWADDR, hc_fa);
        wr32(hrt, HWRTDATA_STATE, RTDATA_STATE_NONE);
        wr32(hrt, HWRTDATA_FREELISTS + 4 * LOCAL_FREELIST, fl_fa);
        wr32(hrt, HWRTDATA_FREELISTS + 4 * GLOBAL_FREELIST, gfl_fa);
        wr32(hrt, HWRTDATA_FREELISTS + 4 * GLOBAL2_FREELIST, gfl2_fa);
        wr64(hrt, HWRTDATA_TAIL_PTRS_DEVVADDR, s.tpc.gpu);
        wr64(hrt, HWRTDATA_VHEAP_TABLE_DEVVADDR, s.vheap.gpu);
        wr64(hrt, HWRTDATA_PM_MLIST_DEVVADDR, s.mlist.gpu);
        wr64(hrt, HWRTDATA_RGN_HEADER_DEVVADDR, s.rgnhdr.gpu);
        wr64(hrt, HWRTDATA_MACROTILE_ARRAY_DEVVADDR, s.macrotile.gpu);
        wr64(hrt, HWRTDATA_RTC_DEVVADDR, 0);
        if pr {
            wr32(hrt, HWRTDATA_RTA_CTL + RTA_CTL_VALID_RENDER_TARGETS, k.heap_fwaddr(s.rta_valid_off));
            wr32(hrt, HWRTDATA_RTA_CTL + RTA_CTL_NUM_PARTIAL_RENDERS, k.heap_fwaddr(s.rta_partial_off));
            wr32(hrt, HWRTDATA_RTA_CTL + RTA_CTL_MAX_RTS, MAX_RT as u32);
            // ui32HWRTDataFlags are the firmware's: PARTIAL_RENDERED is the
            // firmware's to set and use; the host only drops it with a
            // reconstruction (freelist::reset_hwrts), as upstream does.
        }
    }
    k.flush(hrt, HWRTDATA_SIZE);
    r.hwrt[hs].hrt_fwaddr = k.heap_fwaddr(hrt_off);
    r.hwrt[hs].w = w;
    r.hwrt[hs].h = h;
    r.hwrt[hs].samples = samples;
    r.hwrt[hs].used = 1;
    {
        // SAFETY: as above, 88 and 256 bytes.
        let a = |i: usize| unsafe { rd32(hc, 4 * i) };
        km_log!(
            k,
            Inf,
            "render: hc dwords: {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}",
            a(0),
            a(1),
            a(2),
            a(3),
            a(4),
            a(5),
            a(6),
            a(7),
            a(8),
            a(9),
            a(10),
            a(11),
            a(12),
            a(13),
            a(14),
            a(15)
        );
        // SAFETY: as above.
        let b = |i: usize| unsafe { rd32(hrt, 4 * i) };
        let mut i = 0;
        while i + 8 <= HWRTDATA_SIZE / 4 {
            km_log!(
                k,
                Inf,
                "render: hrt[{}]: {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}",
                i,
                b(i),
                b(i + 1),
                b(i + 2),
                b(i + 3),
                b(i + 4),
                b(i + 5),
                b(i + 6),
                b(i + 7)
            );
            i += 8;
        }
    }
    km_log!(
        k,
        Inf,
        "render: HWRT {}x{} x{} te_screen=0x{:08x} mtile1=0x{:08x} mtile2=0x{:08x} isp_mtile=0x{:08x} stride={} rgn={} tpc={} teaa={}",
        w,
        h,
        samples,
        te_screen,
        mtx1 << 18,
        mty1 << 18,
        (mtx1 << 16) | mty1,
        mtx1 * mty1,
        rgn_count,
        tpc_bytes,
        teaa
    );
    // A set built now is the one this render uses, so the pair's next render
    // of this size takes the other member.
    let pair = (hs / 2) * 2;
    r.hwrt[pair].next = if hs == pair { 1 } else { 0 };
    Some(hs)
}

// ---- the client CCBs ------------------------------------------------------------------

/// Make room for `need` bytes at `*wo`; a command that would cross the end
/// is preceded by a PADDING command that runs to the end, and the ring wraps.
fn ring_reserve(k: &YdPvrKm, ring: *mut u8, wo: &mut u32, need: u32) {
    if *wo + need > CCCB_SIZE {
        let ph = ring.wrapping_add(*wo as usize);
        let pad = CCCB_SIZE - *wo;
        // SAFETY: inside the ring.
        unsafe {
            core::ptr::write_bytes(ph, 0, CCB_CMD_HEADER_SIZE);
            wr32(ph, CCB_CMD_HEADER_TYPE, CCB_CMD_TYPE_PADDING);
            wr32(ph, CCB_CMD_HEADER_SIZE_FIELD, pad - CCB_CMD_HEADER_SIZE as u32);
        }
        k.flush(ph, pad as usize);
        *wo = 0;
    }
}

/// One command: header, body padded to 16 bytes.
fn ring_put(k: &YdPvrKm, ring: *mut u8, wo: &mut u32, ty: u32, body: &[u8]) {
    let h = ring.wrapping_add(*wo as usize);
    let padded = round_up(body.len() as u64, u64::from(CCB_CMD_ALIGN)) as u32;
    // SAFETY: the caller reserved the room.
    unsafe {
        core::ptr::write_bytes(h, 0, CCB_CMD_HEADER_SIZE + padded as usize);
        wr32(h, CCB_CMD_HEADER_TYPE, ty);
        wr32(h, CCB_CMD_HEADER_SIZE_FIELD, padded);
        core::ptr::copy_nonoverlapping(body.as_ptr(), h.add(CCB_CMD_HEADER_SIZE), body.len());
    }
    k.flush(h, CCB_CMD_HEADER_SIZE + padded as usize);
    *wo += CCB_CMD_HEADER_SIZE as u32 + padded;
}

fn ufo_bytes(addr: u32, value: u32) -> [u8; 8] {
    let mut u = [0u8; 8];
    u[UFO_ADDR..UFO_ADDR + 4].copy_from_slice(&addr.to_le_bytes());
    u[UFO_VALUE..UFO_VALUE + 4].copy_from_slice(&value.to_le_bytes());
    u
}

fn words_bytes<const N: usize, const B: usize>(w: &[u32; N]) -> [u8; B] {
    let mut b = [0u8; B];
    for (i, x) in w.iter().enumerate() {
        b[4 * i..4 * i + 4].copy_from_slice(&x.to_le_bytes());
    }
    b
}

/// The acknowledgement has arrived: the sync word has reached or passed
/// `want`, compared wrap-safely (the DDK's PVRSRVHasCounter32Advanced).
fn mc_advanced(sync: u32, want: u32) -> bool {
    (sync.wrapping_sub(want) as i32) >= 0
}

/// MMU cache invalidate (page tables + TLB), the DDK's
/// RGXPreKickCacheCommand, sent when a mapping changed (`mmu_gate`), its
/// acknowledgement awaited (`mmu_ack`, 100 ms at 50 us): posting only gets
/// the command accepted, and the fragment phase can otherwise start against
/// a stale TLB (measured 2026-09-22: 7 fence timeouts to 0).
fn mmu_cache_sync(k: &mut YdPvrKm, r: &mut YdPvrRender) {
    let dirty = mmu::take_dirty(k);
    // take_dirty is consumed whether or not the gate is in use.
    if r.cfg.mmu_gate != 0 && !dirty {
        return;
    }
    r.mc_seq = r.mc_seq.wrapping_add(1);
    let want = r.mc_seq;
    let mut mc: KccbCmd = [0; KCCB_CMD_SIZE];
    put32(&mut mc, KCCB_CMD_TYPE, KCCB_CMD_MMUCACHE);
    put32(
        &mut mc,
        KCCB_MMUCACHE_FLAGS,
        MMUCACHEDATA_FLAGS_PT | MMUCACHEDATA_FLAGS_PD | MMUCACHEDATA_FLAGS_PC | MMUCACHEDATA_FLAGS_TLB,
    );
    put32(&mut mc, KCCB_MMUCACHE_SYNC, k.heap_fwaddr(r.mc_sync_off));
    put32(&mut mc, KCCB_MMUCACHE_SYNC_VALUE, want);
    let slot = kccb_post(k, &mc);
    if r.cfg.mmu_ack_probe != 0 {
        kccb_probe_slot(k, slot, "post");
    }
    if r.cfg.mmu_ack == 0 {
        if r.cfg.mmu_ack_probe != 0 {
            r.mc_io_armed = 1;
            r.mc_io_want = want;
            r.mc_io_slot = slot;
        }
        return;
    }
    let sync = k.heap_cpu(r.mc_sync_off);
    let mut rings = 0u32;
    for i in 0..2000u32 {
        k.invd(sync, 4);
        if mc_advanced(hrd(k, r.mc_sync_off, 0), want) {
            if rings != 0 {
                km_log!(
                    k,
                    Inf,
                    "render: MMUCACHE want={} released by re-ring after {} ring(s), {} us",
                    want,
                    rings,
                    i * 50
                );
            }
            if r.cfg.mmu_ack_probe != 0 {
                km_log!(k, Inf, "render: MMUCACHE probe: want={} slot={} acked after {} us", want, slot, i * 50);
            }
            return;
        }
        if r.cfg.mmu_ack_reping != 0 && i > 0 && i % (r.cfg.mmu_ack_reping * 20) == 0 {
            kccb_doorbell(k);
            rings += 1;
        }
        k.busy_wait_us(50);
    }
    k.invd(sync, 4);
    km_log!(k, Wrn, "render: MMUCACHE ack not seen (sync={} want={})", hrd(k, r.mc_sync_off, 0), want);
    if r.cfg.mmu_ack_probe != 0 {
        kccb_probe_slot(k, slot, "miss");
        mc_probe_miss(k, r, slot, want);
    }
}

/// mmu_ack_probe, after a miss: the KCCB state, then the sync word for a
/// further 50 ms at 1 ms steps.
fn mc_probe_miss(k: &mut YdPvrKm, r: &mut YdPvrRender, slot: u32, want: u32) {
    let kctl = k.heap_cpu(k.kccb_ctl_off);
    let rtn = k.heap_cpu(k.kccb_rtn_off).wrapping_add(slot as usize * 4);
    let sync = k.heap_cpu(r.mc_sync_off);
    k.invd(kctl, CCB_CTL_SIZE);
    k.invd(rtn, 4);
    km_log!(
        k,
        Wrn,
        "render: MMUCACHE probe: miss want={} slot={}: KCCB r/w={}/{} rtn[slot]=0x{:x}",
        want,
        slot,
        hrd(k, k.kccb_ctl_off, CCB_CTL_READ_OFFSET),
        hrd(k, k.kccb_ctl_off, CCB_CTL_WRITE_OFFSET),
        hrd(k, k.kccb_rtn_off, slot as usize * 4)
    );
    let (mut at10, mut at50, mut seen_ms, mut seen) = (0u32, 0u32, 0u32, false);
    for ms in 1..=50u32 {
        k.busy_wait_us(1000);
        k.invd(sync, 4);
        let v = hrd(k, r.mc_sync_off, 0);
        if !seen && mc_advanced(v, want) {
            seen = true;
            seen_ms = ms;
        }
        if ms == 10 {
            at10 = hrd(k, r.mc_sync_off, 0);
        }
        if ms == 50 {
            at50 = hrd(k, r.mc_sync_off, 0);
        }
    }
    k.invd(kctl, CCB_CTL_SIZE);
    let (kr, kw) = (hrd(k, k.kccb_ctl_off, CCB_CTL_READ_OFFSET), hrd(k, k.kccb_ctl_off, CCB_CTL_WRITE_OFFSET));
    if seen {
        km_log!(
            k,
            Wrn,
            "render: MMUCACHE probe: want={}: sync +10 ms={} +50 ms={}; want seen {} ms after the miss; KCCB r/w={}/{}",
            want,
            at10,
            at50,
            seen_ms,
            kr,
            kw
        );
    } else {
        km_log!(
            k,
            Wrn,
            "render: MMUCACHE probe: want={}: sync +10 ms={} +50 ms={}; want NOT seen within 50 ms; KCCB r/w={}/{}",
            want,
            at10,
            at50,
            kr,
            kw
        );
        r.mc_unseen = want;
    }
}

/// mmu_ack_probe with mmu_ack off: after the kick, poll (10 us steps, up to
/// 200 ms) for the sync word, the kick's geometry fence and its fragment
/// fence, and say whether the invalidate ran before the kick's first progress.
/// Print only.
fn mc_probe_in_order(k: &mut YdPvrKm, r: &mut YdPvrRender, seq: u32) {
    let sync = k.heap_cpu(r.mc_sync_off);
    let gat = (geom_ufo_slot(seq) * 4) as usize;
    let gufo = k.heap_cpu(r.gufo_off).wrapping_add(gat);
    let fufo = k.heap_cpu(r.fufo_off);
    const NEVER: u32 = u32::MAX;
    let (mut t_sync, mut t_geom, mut t_frag) = (NEVER, NEVER, NEVER);
    r.mc_io_armed = 0;
    let mut i = 0u32;
    while i < 20000 && (t_sync == NEVER || t_geom == NEVER || t_frag == NEVER) {
        k.invd(sync, 4);
        k.invd(gufo, 4);
        k.invd(fufo, 4);
        if t_sync == NEVER && mc_advanced(hrd(k, r.mc_sync_off, 0), r.mc_io_want) {
            t_sync = i;
        }
        if t_geom == NEVER && hrd(k, r.gufo_off, gat) == seq {
            t_geom = i;
        }
        if t_frag == NEVER && mc_advanced(hrd(k, r.fufo_off, 0), seq) {
            t_frag = i;
        }
        k.busy_wait_us(10);
        i += 1;
    }
    if t_geom == NEVER {
        km_log!(
            k,
            Wrn,
            "render: MMUCACHE in-order: want={} seq={}: sync {}, geom fence never, frag fence {} within 200 ms; invalidate executed before kick: unknown",
            r.mc_io_want,
            seq,
            if t_sync == NEVER { "never" } else { "seen" },
            if t_frag == NEVER { "never" } else { "seen" }
        );
        return;
    }
    if t_sync == NEVER {
        km_log!(
            k,
            Wrn,
            "render: MMUCACHE in-order: want={} seq={}: sync never within 200 ms, geom fence at {} us, frag fence {}; invalidate executed before kick: no (not within 200 ms)",
            r.mc_io_want,
            seq,
            t_geom * 10,
            if t_frag == NEVER { "never" } else { "seen" }
        );
        return;
    }
    km_log!(
        k,
        Inf,
        "render: MMUCACHE in-order: want={} seq={}: sync at {} us, geom fence at {} us, frag fence at {} us; invalidate executed before kick: {}, delta {} us",
        r.mc_io_want,
        seq,
        t_sync * 10,
        t_geom * 10,
        if t_frag == NEVER { -1 } else { (t_frag * 10) as i32 },
        if t_sync <= t_geom { "yes" } else { "no" },
        (t_geom as i32 - t_sync as i32) * 10
    );
}

/// mmu_ack_probe: a miss the re-read never saw, read again after the kick
/// that followed it.
fn mc_probe_after_kick(k: &mut YdPvrKm, r: &mut YdPvrRender) {
    let sync = k.heap_cpu(r.mc_sync_off);
    let want = r.mc_unseen;
    r.mc_unseen = 0;
    let mut ms = 0u32;
    while ms <= 20 {
        k.invd(sync, 4);
        if mc_advanced(hrd(k, r.mc_sync_off, 0), want) {
            break;
        }
        k.busy_wait_us(1000);
        ms += 1;
    }
    let v = hrd(k, r.mc_sync_off, 0);
    km_log!(
        k,
        Wrn,
        "render: MMUCACHE probe: want={} after the kick doorbell: sync={} ({}, {} ms)",
        want,
        v,
        if mc_advanced(v, want) { "seen" } else { "not seen" },
        ms
    );
}

// ---- submit -------------------------------------------------------------------------

/// Kick one render (geometry then fragment) with the caller's register
/// blocks; `fence_out` is the sequence `wait_for` waits on.
pub fn submit(k: &mut YdPvrKm, r: &mut YdPvrRender, kick: &PvrRenderKick, fence_out: &mut u32) -> i32 {
    if r.inited == 0 {
        return -errno::EINVAL;
    }
    // A stopped render service takes no kick ([`super::freelist::stop`]).
    if k.stop_cause != 0 {
        return -errno::ESHUTDOWN;
    }
    let t_submit = kick_times::mark();
    let Some(hs) = hwrt_get(k, r, kick.width, kick.height, kick.samples) else {
        return -errno::ENOMEM;
    };
    // A geometry UFO slot is reused every GEOM_UFO_SLOTS kicks, and its value
    // must not be overwritten while the 3D that waits for it is queued.
    if r.seq >= GEOM_UFO_SLOTS {
        let oldest = r.seq + 1 - GEOM_UFO_SLOTS;
        if completed(k, r) < oldest {
            let _ = wait_for(k, r, oldest, 20000, "geometry UFO ring throttle");
        }
    }
    // The waits above (a member in flight, the ring throttle) may have found
    // the stop.
    if k.stop_cause != 0 {
        return -errno::ESHUTDOWN;
    }
    r.seq = r.seq.wrapping_add(1);
    let seq = r.seq;
    r.hwrt[hs].last_seq = seq;
    r.last_hs = hs as i32;
    r.dbg_vdm = kick.geom.vdm_ctrl_stream_base;
    let gring = k.heap_cpu(r.gcccb_off);
    let fring = k.heap_cpu(r.fcccb_off);
    let hrt_fwaddr = r.hwrt[hs].hrt_fwaddr;
    let gufo_base = k.heap_fwaddr(r.gufo_off);
    let gufo_fa = gufo_base + geom_ufo_slot(seq) * 4;
    // With the partial-render knob: the geometry timeline word (the word
    // after the UFO slots, ctx_init) and the invariant it keeps, timeline
    // == seq exactly when this kick's geometry phase has run. The PR fences
    // use it exactly as upstream uses its single sync timeline: the geometry
    // command references seq - 1, the fragment PR fence references seq
    // (Linux drm/imagination pvr_queue.c, "Holds the TA/3D fence value to
    // allow the 3D partial render command to go through"; Mesa srvkm
    // pvr_arch_srv_job_render.c:531-569). Option A of docs/notes/
    // pvr-render-pool.md, "Design: the firmware's partial render".
    let pr = r.cfg.partial_render != 0;
    let timeline_fa = gufo_base + 4 * GEOM_UFO_SLOTS;

    // GEOM: RGXFWIF_CMDTA (16 dwords; past CMDTA3D_SHARED a value from DDK
    // capture) + UPDATE (geometry UFO = seq); with the knob the PR fence
    // words {timeline, seq - 1} in the command and a second UPDATE to the
    // timeline (value = seq, once the GEOM command itself has run).
    {
        let mut dk = [0u32; 16];
        dk[CMDTA3D_SHARED_HWRT_DATA / 4] = hrt_fwaddr;
        dk[4] = kick.geom.vdm_ctrl_stream_base as u32;
        dk[5] = (kick.geom.vdm_ctrl_stream_base >> 32) as u32;
        dk[8] = kick.geom.ppp_ctrl;
        dk[9] = kick.geom.te_psg;
        dk[12] = GEOM_FLAGS_FIRSTKICK | GEOM_FLAGS_LASTKICK;
        dk[13] = gufo_fa; // partial-render fence address
        dk[14] = 0; // ... and value, unwritten today
        if pr {
            // The words after the flags are the command's partial-render
            // fence (RGXFWIF_CMDTA.partial_render_ta_3d_fence, Mesa
            // pvr_rogue_fwif.h; dk[13] already carries the address today,
            // the value never). The offsets are the captured layout
            // (driver/src/pvr_selftest.c's CMDTA3D_SHARED copy; the DDK's
            // own CMDTA struct is not in the vendored headers, so this is
            // as far as a pin reaches).
            dk[13] = timeline_fa;
            dk[14] = seq.wrapping_sub(1);
        }
        let mut gwo = r.gwo;
        ring_reserve(k, gring, &mut gwo, 16 + 64 + 16 + 16 + if pr { 16 + 16 } else { 0 }); // bodies padded to 16
        ring_put(k, gring, &mut gwo, CCB_CMD_TYPE_GEOM, &words_bytes::<16, 64>(&dk));
        ring_put(k, gring, &mut gwo, CCB_CMD_TYPE_UPDATE, &ufo_bytes(gufo_fa, seq));
        if pr {
            ring_put(k, gring, &mut gwo, CCB_CMD_TYPE_UPDATE, &ufo_bytes(timeline_fa, seq));
        }
        r.gwo = gwo;
    }

    // FRAG: with the knob the PR trio FENCE_PR + 3D_PR + UPDATE (geometry
    // timeline), then FENCE (geometry UFO) + RGXFWIF_CMD3D (80 dwords; past
    // CMDTA3D_SHARED a value from DDK capture; the 3D_PR body is a copy of
    // it) + UPDATE (fragment UFO = seq).
    {
        let f = &kick.frag;
        let mut dk = [0u32; 80];
        dk[CMDTA3D_SHARED_HWRT_DATA / 4] = hrt_fwaddr;
        dk[4] = f.usc_pixel_output_ctrl;
        dk[5..9].copy_from_slice(&f.usc_clear_register);
        dk[9] = f.isp_bgobjdepth;
        dk[10] = f.isp_bgobjvals;
        dk[11] = f.isp_aa;
        dk[12] = f.isp_ctl;
        dk[13] = f.event_pixel_pds_info;
        dk[15] = f.event_pixel_pds_data;
        // dw18..29: isp_scissor/dbias/oclqry/zlsctl/zload/stencil bases
        // (docs/gpu-recipes/coeff_base_analysis.md, the DDK 25.2 layout)
        let lohi = |dk: &mut [u32; 80], i: usize, v: u64| {
            dk[i] = v as u32;
            dk[i + 1] = (v >> 32) as u32;
        };
        lohi(&mut dk, 18, f.isp_scissor_base);
        lohi(&mut dk, 20, f.isp_dbias_base);
        lohi(&mut dk, 22, f.isp_oclqry_base);
        lohi(&mut dk, 24, f.isp_zlsctl);
        lohi(&mut dk, 26, f.isp_zload_store_base);
        lohi(&mut dk, 28, f.isp_stencil_load_store_base);
        // pbe_word[8][3] in the firmware's 3D registers, PBE_STRIDE_DW apart
        let nrt = (if f.num_rt != 0 { f.num_rt } else { 1 }).min(MAX_RT as u32) as usize;
        for (i, pbe) in f.pbe_word.iter().enumerate().take(nrt) {
            let o = 30 + i * PBE_STRIDE_DW;
            lohi(&mut dk, o, pbe[0]);
            lohi(&mut dk, o + 2, pbe[1]);
        }
        lohi(&mut dk, 70, f.pds_bgnd[0]);
        lohi(&mut dk, 72, f.pds_bgnd[1]);
        lohi(&mut dk, 74, f.pds_bgnd[2]);
        dk[76] = f.cmd_flags; // RGXFWIF_CMD3D.flags (dw77/78: zls/sls stride, RTAs only)
        dk[79] = 0x0000_0001; // execute_count: value from DDK capture
        if r.cfg.verbose != 0 {
            let mut i = 0;
            while i < 80 {
                km_log!(
                    k,
                    Inf,
                    "render: cmd3d[{:2}]: {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}",
                    i,
                    dk[i],
                    dk[i + 1],
                    dk[i + 2],
                    dk[i + 3],
                    dk[i + 4],
                    dk[i + 5],
                    dk[i + 6],
                    dk[i + 7]
                );
                i += 8;
            }
        }
        let mut fwo = r.fwo;
        // One trio (FENCE + 3D + UPDATE) today's shape; the PR trio adds an
        // equal amount (bodies padded to 16).
        let need = 16 + 16 + 16 + 320 + 16 + 16;
        ring_reserve(k, fring, &mut fwo, if pr { 2 * need } else { need });
        if pr {
            // The partial-render trio first, the order of Linux
            // drm/imagination's fragment queue (pvr_queue.c:698-737):
            // FENCE_PR on the geometry timeline at seq (satisfied exactly
            // when this kick's geometry UPDATE has run), the 3D_PR, and its
            // UPDATE. The 3D_PR body is a byte copy of the 3D command below:
            // for an ordinary single-pass render Mesa builds it exactly so -
            // same attachments, same EOT data, same Z/S and background words
            // (pvr_arch_cmd_buffer.c:1736-1747, pvr_arch_job_render.c:1353-
            // 1407) - and the firmware never executes it unless a partial
            // render is pending. Its UPDATE rewrites the timeline with the
            // same seq (not seq + 1: the next kick's geometry fence names
            // seq by equality, and the firmware's UFO test is an equality
            // test, ctx_init) - a no-op while no partial render is pending.
            ring_put(k, fring, &mut fwo, CCB_CMD_TYPE_FENCE_PR, &ufo_bytes(timeline_fa, seq));
            ring_put(k, fring, &mut fwo, CCB_CMD_TYPE_3D_PR, &words_bytes::<80, 320>(&dk));
            ring_put(k, fring, &mut fwo, CCB_CMD_TYPE_UPDATE, &ufo_bytes(timeline_fa, seq));
        }
        // The FENCE on the geometry UFO first: the firmware schedules a 3D as
        // soon as its kick arrives, so without it the 3D can run before the
        // TA has tiled (observed: an empty frame).
        ring_put(k, fring, &mut fwo, CCB_CMD_TYPE_FENCE, &ufo_bytes(gufo_fa, seq));
        r.last_frag_cmd = dk;
        r.last_frag_seq = seq;
        ring_put(k, fring, &mut fwo, CCB_CMD_TYPE_3D, &words_bytes::<80, 320>(&dk));
        let fufo_fa = k.heap_fwaddr(r.fufo_off);
        ring_put(k, fring, &mut fwo, CCB_CMD_TYPE_UPDATE, &ufo_bytes(fufo_fa, seq));
        r.fwo = fwo;
    }
    k.fence();

    // Pre-kick MMU cache invalidate, acknowledged through a sync word.
    let (t_mmu, mc_before) = (kick_times::mark(), r.mc_seq);
    mmu_cache_sync(k, r);
    let (mmu_ticks, mmu_synced) = (kick_times::mark().saturating_sub(t_mmu), r.mc_seq != mc_before);

    // Both kicks up front (geometry then fragment): the firmware orders them
    // through the 3D's fence. One COMBINED_TA_3D_KICK (the DDK's normal
    // path: rgxta3d.c sets bUseSingleFWCommand for every ordinary render) or
    // two separate KICKs (its exception path).
    {
        let cleanup = hrt_fwaddr + HWRTDATA_CLEANUP_STATE as u32;
        let kick_data = |ctx: u32, cwoff: u32| {
            let mut d = [0u8; KCCB_KICK_DATA_SIZE];
            let at = |f: usize| f - KCCB_KICK_CONTEXT;
            d[at(KCCB_KICK_CONTEXT)..at(KCCB_KICK_CONTEXT) + 4].copy_from_slice(&ctx.to_le_bytes());
            d[at(KCCB_KICK_CWOFF)..at(KCCB_KICK_CWOFF) + 4].copy_from_slice(&cwoff.to_le_bytes());
            d[at(KCCB_KICK_CWRAPMASK)..at(KCCB_KICK_CWRAPMASK) + 4].copy_from_slice(&(CCCB_SIZE - 1).to_le_bytes());
            d[at(KCCB_KICK_NUM_CLEANUP)..at(KCCB_KICK_NUM_CLEANUP) + 4].copy_from_slice(&1u32.to_le_bytes());
            d[at(KCCB_KICK_CLEANUP_CTL)..at(KCCB_KICK_CLEANUP_CTL) + 4].copy_from_slice(&cleanup.to_le_bytes());
            d
        };
        let rc_fa = k.heap_fwaddr(r.rc_off);
        let gdata = kick_data(rc_fa + FWRENDERCONTEXT_TA_CONTEXT as u32, r.gwo);
        let fdata = kick_data(rc_fa + FWRENDERCONTEXT_3D_CONTEXT as u32, r.fwo);
        let mut gkick: KccbCmd = [0; KCCB_CMD_SIZE];
        put32(&mut gkick, KCCB_CMD_TYPE, KCCB_CMD_KICK);
        gkick[KCCB_KICK_CONTEXT..KCCB_KICK_CONTEXT + KCCB_KICK_DATA_SIZE].copy_from_slice(&gdata);
        let mut fkick: KccbCmd = [0; KCCB_CMD_SIZE];
        put32(&mut fkick, KCCB_CMD_TYPE, KCCB_CMD_KICK);
        fkick[KCCB_KICK_CONTEXT..KCCB_KICK_CONTEXT + KCCB_KICK_DATA_SIZE].copy_from_slice(&fdata);

        r.last_kick_n = 0;
        if r.cfg.combined_kick != 0 {
            let mut ck: KccbCmd = [0; KCCB_CMD_SIZE];
            put32(&mut ck, KCCB_CMD_TYPE, KCCB_CMD_COMBINED_TA_3D_KICK);
            ck[KCCB_COMBINED_TA..KCCB_COMBINED_TA + KCCB_KICK_DATA_SIZE].copy_from_slice(&gdata);
            ck[KCCB_COMBINED_3D..KCCB_COMBINED_3D + KCCB_KICK_DATA_SIZE].copy_from_slice(&fdata);
            r.last_kick[0] = ck;
            r.last_kick_n = 1;
            kccb_post(k, &ck);
        } else {
            r.last_kick[0] = gkick;
            r.last_kick[1] = fkick;
            r.last_kick_n = 2;
            kccb_post(k, &gkick);
            kccb_post(k, &fkick);
        }
    }
    if r.mc_unseen != 0 {
        mc_probe_after_kick(k, r);
    }
    if r.mc_io_armed != 0 {
        mc_probe_in_order(k, r, seq);
    }
    km_log!(
        k,
        Dbg,
        "render: kick #{} {}x{} vdm=0x{:x} epds=0x{:x} bgnd0=0x{:x} gwo={} fwo={}",
        seq,
        kick.width,
        kick.height,
        kick.geom.vdm_ctrl_stream_base,
        kick.frag.event_pixel_pds_data,
        kick.frag.pds_bgnd[0],
        r.gwo,
        r.fwo
    );
    kick_times::posted(k, r, seq, t_submit, mmu_ticks, mmu_synced);
    *fence_out = seq;
    0
}

// ---- completion and the wait --------------------------------------------------------

/// The last fragment fence the firmware wrote (0 before init).
pub fn completed(k: &YdPvrKm, r: &YdPvrRender) -> u32 {
    if r.inited == 0 {
        return 0;
    }
    k.invd(k.heap_cpu(r.fufo_off), 4);
    hrd(k, r.fufo_off, 0)
}

/// Bring-up diagnostics: how many words of a buffer are non-zero.
fn nz_count(k: &YdPvrKm, p: *const u8, bytes: u32) -> u32 {
    k.invd(p, bytes as usize);
    (0..bytes / 4)
        // SAFETY: the buffer is at least `bytes` long.
        .filter(|&i| unsafe { rd32(p, 4 * i as usize) } != 0)
        .count() as u32
}

fn lookup_log(k: &YdPvrKm, what: &str, va: u64) {
    let pte = mmu::lookup(k, va);
    km_log!(
        k,
        Inf,
        "render:   map {:<10} va=0x{:010x} pte=0x{:016x}{}",
        what,
        va,
        pte,
        if pte != 0 { "" } else { "  <-- UNMAPPED" }
    );
}

/// The CPU address of a mapped device VA (through its leaf entry).
fn va_cpu(k: &YdPvrKm, pte: u64, va: u64) -> *const u8 {
    k.cpu_at(((pte & 0x0000_00FF_FFFF_F000) - k.phys) + (va & 0xFFF))
}

/// Resolve every address the last kick's geometry phase touches.
fn debug_mappings(k: &YdPvrKm, r: &YdPvrRender) {
    let hs = (0..HWRT_SETS).rev().find(|&i| r.hwrt[i].used != 0);
    lookup_log(k, "PB", GENERAL_HEAP_BASE);
    lookup_log(k, "vdm", r.dbg_vdm);
    if r.dbg_vdm != 0 {
        let v = va_cpu(k, mmu::lookup(k, r.dbg_vdm), r.dbg_vdm);
        // SAFETY: a VDM stream in the carveout (13 words).
        let w = |i: usize| unsafe { rd32(v, 4 * i) };
        let ppp = (u64::from(w(0) & 0xFF) << 32) | u64::from(w(1) & 0xFFFF_FFFC);
        let pds_d = PDSCODEDATA_HEAP_BASE + u64::from(w(3) & !0xF);
        let pds_c = PDSCODEDATA_HEAP_BASE + u64::from(w(4) & !0xF);
        let idx = (u64::from(w(7) & 0xFF) << 32) | u64::from(w(8) & 0xFFFF_FFFC);
        km_log!(
            k,
            Inf,
            "render:   vdm words: {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}",
            w(0),
            w(1),
            w(2),
            w(3),
            w(4),
            w(5),
            w(6),
            w(7),
            w(8),
            w(9),
            w(10),
            w(11),
            w(12)
        );
        lookup_log(k, "ppp", ppp);
        lookup_log(k, "vs_pds_d", pds_d);
        lookup_log(k, "vs_pds_c", pds_c);
        lookup_log(k, "index", idx);
        let pte = mmu::lookup(k, pds_d);
        if pte != 0 {
            let d = va_cpu(k, pte, pds_d);
            // SAFETY: a PDS data segment in the carveout (8 words).
            let dw = |i: usize| unsafe { rd32(d, 4 * i) };
            let vtx = (u64::from(dw(5)) << 32) | u64::from(dw(4));
            km_log!(
                k,
                Inf,
                "render:   vs pds data @mem: {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}",
                dw(0),
                dw(1),
                dw(2),
                dw(3),
                dw(4),
                dw(5),
                dw(6),
                dw(7)
            );
            lookup_log(k, "vs_usc", USCCODE_HEAP_BASE + u64::from(dw(0)));
            lookup_log(k, "vertex", vtx);
        }
    }
    lookup_log(k, "pds0", PDSCODEDATA_HEAP_BASE);
    lookup_log(k, "usc0", USCCODE_HEAP_BASE);
    lookup_log(k, "fl_global1", 0xFA_0000_0000);
    lookup_log(k, "fl_local", 0xFA_0000_8000);
    if let Some(i) = hs {
        let s = &r.hwrt[i];
        lookup_log(k, "mlist", s.mlist.gpu);
        lookup_log(k, "vheap", s.vheap.gpu);
        lookup_log(k, "tpc", s.tpc.gpu);
        lookup_log(k, "rgnhdr", s.rgnhdr.gpu);
        lookup_log(k, "macrotile", s.macrotile.gpu);
    }
}

fn trace_tail(k: &YdPvrKm, lvl_err: bool, label: &str, keep: u32) {
    let tb = k.heap_cpu(k.trace_buf_off);
    let tptr = hrd(k, k.tracebuf_off, TRACEBUF_TRACE_POINTER);
    let wrapc = hrd(k, k.tracebuf_off, TRACEBUF_WRAP_COUNT);
    let from = tptr.saturating_sub(keep);
    k.invd(tb, tptr as usize * 4);
    if lvl_err {
        km_log!(k, Err, "render: {}TracePointer={} WrapCount={} (dumping {}..{})", label, tptr, wrapc, from, tptr);
    } else {
        km_log!(k, Inf, "render: {}TracePointer={} WrapCount={} (dumping {}..{})", label, tptr, wrapc, from, tptr);
    }
    let mut i = from;
    while i + 6 <= tptr {
        // SAFETY: the trace ring (12000 words; tptr is inside it).
        let t = |j: u32| unsafe { rd32(tb, 4 * j as usize) };
        if lvl_err {
            km_log!(
                k,
                Err,
                " t[{}]: {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}",
                i,
                t(i),
                t(i + 1),
                t(i + 2),
                t(i + 3),
                t(i + 4),
                t(i + 5)
            );
        } else {
            km_log!(
                k,
                Inf,
                " t[{}]: {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}",
                i,
                t(i),
                t(i + 1),
                t(i + 2),
                t(i + 3),
                t(i + 4),
                t(i + 5)
            );
        }
        i += 6;
    }
}

fn render_debug_after(k: &mut YdPvrKm, r: &mut YdPvrRender, fence: u32, waited_ms: u32) {
    let gufo = k.heap_cpu(r.gufo_off);
    let fufo = k.heap_cpu(r.fufo_off);
    // The set the LAST KICK used, not merely the last one allocated.
    let hs = if r.last_hs >= 0 { Some(r.last_hs as usize) } else { None };
    k.invd(gufo, 4);
    k.invd(fufo, 4);
    km_log!(
        k,
        Inf,
        "render: fence {} done after {} ms: geom_ufo={} frag_ufo={}",
        fence,
        waited_ms,
        hrd(k, r.gufo_off, 0),
        hrd(k, r.fufo_off, 0)
    );
    debug_mappings(k, r);
    if hrd(k, r.gufo_off, 0) < fence {
        let mut i = 0u32;
        while i < 500 {
            k.invd(gufo, 4);
            if hrd(k, r.gufo_off, 0) >= fence {
                break;
            }
            fwccb_service_r(k, r);
            k.busy_wait_us(1000);
            i += 1;
        }
        km_log!(
            k,
            Inf,
            "render: geom_ufo={} after +{} ms; TA_CYCLE(0x6020)=0x{:x} TA_PHASE(0x6008)=0x{:x} TE_SIG(0x5028)=0x{:x} VCE(0x5030)=0x{:x} VDM_CTX_STORE(0x430)=0x{:x} CLK_STATUS=0x{:x}",
            hrd(k, r.gufo_off, 0),
            i,
            k.reg32(0x6020),
            k.reg32(0x6008),
            k.reg32(0x5028),
            k.reg32(0x5030),
            k.reg32(0x0430),
            k.reg64(0x0008)
        );
        km_log!(
            k,
            Inf,
            "render: MMU_FS1(0xE150)=0x{:x} MMU_FS_PM(0xE130)=0x{:x} EVENT_STATUS(0x130)=0x{:x} BIF_MMU(0x1358)=0x{:x}",
            k.reg64(0xE150),
            k.reg64(0xE130),
            k.reg32(0x0130),
            k.reg32(0x1358)
        );
    }
    if let Some(i) = hs {
        let s = r.hwrt[i];
        km_log!(k, Inf, "render: TA-out for the {}x{} set (seq {}):", s.w, s.h, s.last_seq);
        km_log!(
            k,
            Inf,
            "render: TA-out rgnhdr nz={}/256 vheap nz={}/512 tpc nz={}/32 mlist nz={}/1024 macrotile nz={}/1024",
            nz_count(k, s.rgnhdr.cpu, 1024),
            nz_count(k, s.vheap.cpu, 2048),
            nz_count(k, s.tpc.cpu, 128),
            nz_count(k, s.mlist.cpu, 4096),
            nz_count(k, s.macrotile.cpu, 4096)
        );
    }
    {
        let rtn = k.heap_cpu(k.kccb_rtn_off);
        k.invd(k.heap_cpu(r.gcccbctl_off), CCCB_CTL_SIZE);
        k.invd(k.heap_cpu(r.fcccbctl_off), CCCB_CTL_SIZE);
        k.invd(rtn, 16 * 4);
        let rt = |i: usize| hrd(k, k.kccb_rtn_off, 4 * i);
        km_log!(
            k,
            Inf,
            "render: geom cccb r/w={}/{} frag cccb r/w={}/{} KCCB rtn[0..5]={:08x} {:08x} {:08x} {:08x} {:08x} {:08x}",
            hrd(k, r.gcccbctl_off, CCCB_CTL_READ_OFFSET),
            r.gwo,
            hrd(k, r.fcccbctl_off, CCCB_CTL_READ_OFFSET),
            r.fwo,
            rt(0),
            rt(1),
            rt(2),
            rt(3),
            rt(4),
            rt(5)
        );
        if let Some(i) = hs {
            let off = r.hwrt[i].hrt_off;
            k.invd(k.heap_cpu(off), HWRTDATA_SIZE);
            km_log!(
                k,
                Inf,
                "render: hwrt state={} cleanup submitted/executed={}/{}",
                hrd(k, off, HWRTDATA_STATE),
                hrd(k, off, HWRTDATA_CLEANUP_SUBMITTED),
                hrd(k, off, HWRTDATA_CLEANUP_EXECUTED)
            );
        }
    }
    // The DDK FW trace tail (raw dwords; decode with tools/sfdecode/sfdecode.py).
    trace_tail(k, false, "DDK FW trace ", TRACE_DUMP_DWORDS);
}

fn dump_last_frag_cmd(k: &YdPvrKm, r: &YdPvrRender, fence: u32) {
    km_log!(k, Err, "render: last FRAG cmd3d (seq {}), fence {}:", r.last_frag_seq, fence);
    let c = &r.last_frag_cmd;
    let mut i = 0;
    while i < 80 {
        km_log!(
            k,
            Err,
            " c3d[{:2}]: {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}",
            i,
            c[i],
            c[i + 1],
            c[i + 2],
            c[i + 3],
            c[i + 4],
            c[i + 5],
            c[i + 6],
            c[i + 7]
        );
        i += 8;
    }
}

/// The commands a client CCB holds but the firmware has not read, and what
/// each FENCE/UPDATE needs (`from` the firmware's read offset, `to` ours).
fn dump_cccb_pending(k: &YdPvrKm, what: &str, ring: *const u8, from: u32, to: u32) {
    if from == to {
        km_log!(k, Err, "render: stall: {} ccb has nothing unread", what);
        return;
    }
    let mut off = from;
    let mut n = 0u32;
    while off != to && n < 8 {
        let h = ring.wrapping_add((off & (CCCB_SIZE - 1)) as usize);
        let body = (off + CCB_CMD_HEADER_SIZE as u32) & (CCCB_SIZE - 1);
        // SAFETY: inside the ring.
        let (ty, size) = unsafe { (rd32(h, CCB_CMD_HEADER_TYPE), rd32(h, CCB_CMD_HEADER_SIZE_FIELD)) };
        if size > CCCB_SIZE {
            km_log!(k, Err, "render: stall: {}[{}] off={} bad cmd_size={} (type=0x{:08x})", what, n, off, size, ty);
            return;
        }
        if ty == CCB_CMD_TYPE_FENCE || ty == CCB_CMD_TYPE_FENCE_PR || ty == CCB_CMD_TYPE_UPDATE {
            let u = ring.wrapping_add(body as usize);
            // SAFETY: as above.
            let (addr, value) = unsafe { (rd32(u, UFO_ADDR), rd32(u, UFO_VALUE)) };
            let fwbase = k.heap_fwaddr(0);
            let kind = if ty == CCB_CMD_TYPE_UPDATE {
                "UPDATE"
            } else if ty == CCB_CMD_TYPE_FENCE_PR {
                "FENCE_PR"
            } else {
                "FENCE"
            };
            if addr >= fwbase && addr - fwbase < k.fw_main_size {
                let w = k.heap_cpu(addr - fwbase);
                k.invd(w, 4);
                let is = hrd(k, addr - fwbase, 0);
                let note = if ty == CCB_CMD_TYPE_UPDATE {
                    ""
                } else if is >= value {
                    "  (MET - the firmware should be running this)"
                } else {
                    "  (UNMET - this is what it is waiting for)"
                };
                km_log!(
                    k,
                    Err,
                    "render: stall: {}[{}] off={} {} ufo 0x{:08x} needs {}, is {}{}",
                    what,
                    n,
                    off,
                    kind,
                    addr,
                    value,
                    is,
                    note
                );
            } else {
                km_log!(
                    k,
                    Err,
                    "render: stall: {}[{}] off={} {} ufo 0x{:08x} needs {} (outside the FW heap)",
                    what,
                    n,
                    off,
                    kind,
                    addr,
                    value
                );
            }
        } else {
            km_log!(k, Err, "render: stall: {}[{}] off={} type=0x{:08x} size={}", what, n, off, ty, size);
        }
        off = (off + CCB_CMD_HEADER_SIZE as u32 + size) & (CCCB_SIZE - 1);
        n += 1;
    }
}

/// The step, in microseconds, between two reads of the fragment UFO inside
/// each millisecond of [`wait_for`]. Without the feature `pvr-fine-wait` it
/// is the whole millisecond: one read per ms, so a wait ends on the first
/// whole millisecond after the fence moved
/// (docs/notes/pvr-fence-wait.md). With it, the UFO is read
/// every 20 us; the once-per-ms work (FWCCB service, re-ring, re-post, HWR
/// check) and the timeout stay per ms.
#[cfg(feature = "pvr-fine-wait")]
pub const WAIT_POLL_US: u32 = 20;
#[cfg(not(feature = "pvr-fine-wait"))]
pub const WAIT_POLL_US: u32 = 1000;
const _: () = assert!(WAIT_POLL_US > 0 && 1000 % WAIT_POLL_US == 0);

/// Has the firmware written fence `fence` (or a later one) to the fragment
/// UFO? (With `pvr-kick-times`, every kick in flight is stamped first.)
fn fence_reached(k: &YdPvrKm, r: &mut YdPvrRender, fence: u32) -> bool {
    kick_times::poll(k, r);
    k.invd(k.heap_cpu(r.fufo_off), 4);
    hrd(k, r.fufo_off, 0) >= fence
}

/// The FWCCB service of the render path: with `cfg.fl_grow_on_demand`, a
/// freelist grow the reserve and the shared pool cannot serve takes its pages
/// from the render pool's bump, mapped in the general heap as the lists' own
/// backing is (`bo_alloc`, as `freelist::build` and the grow pool allocate).
/// It does not take blocks from the pool's free lists: those are the C
/// render service's, and reaching them from here would re-enter it.
fn fwccb_service_r(k: &mut YdPvrKm, r: &mut YdPvrRender) -> i32 {
    let (reqs, done) = (k.fl_grow_reqs, k.fl_grows_done);
    let mut snap = None;
    let n = fwccb_service_r_inner(k, r, &mut snap);
    if let Some(s) = snap.filter(|_| k.fl_grow_reqs != reqs) {
        s.say(k);
    }
    let served = k.fl_grows_done.wrapping_sub(done);
    if cfg!(feature = "pvr-grow-scene") && served >= RUNAWAY_BATCH {
        runaway_dump(k, r, served);
    }
    n
}

/// Grows served in one FWCCB batch from which the batch is a runaway's
/// ([`runaway_dump`]). In every clean run each served grow was alone in
/// its batch; fwccb-2 served 12 in one batch, and ab3-on 12. d320c8a5's
/// terrain served 2 in one, and its run was otherwise clean.
pub const RUNAWAY_BATCH: u32 = 2;

/// The runaway batch's dump (feature `pvr-grow-scene`): when one FWCCB
/// batch served [`RUNAWAY_BATCH`] grows or more, the firmware took each
/// answer's pages at once and asked again. That is the runaway of fwccb-2
/// and ab3-on (docs/notes/pvr-render-pool.md), where a
/// bump-height TA used 3328 pages more and never finished, and the ack words
/// alone could not say why. One line, after the batch:
/// - the TA's registers the wait's progress line reads;
/// - the PM's catalogue bases, allocation and stack registers (the stall
///   dump's set);
/// - the last kick's HWRT data: state, flags, and the PM's ALIST and MList
///   stack pointers the firmware keeps there;
/// - the last kick's VDM stream base.
///
/// It reads only. It comes after the batch's synchronous prints, so the
/// registers are tens of ms after the first request: a TA still out of
/// memory is still there, and one that went on is not.
fn runaway_dump(k: &YdPvrKm, r: &YdPvrRender, served: u32) {
    let hs = r.last_hs;
    let (state, flags, alist_sp, mlist_sp) = if hs >= 0 && (hs as usize) < HWRT_SETS {
        let h = k.heap_cpu(r.hwrt[hs as usize].hrt_off);
        k.invd(h, HWRTDATA_SIZE);
        // SAFETY: an HWRT data the render service built (256 bytes).
        unsafe {
            (
                rd32(h, HWRTDATA_STATE),
                rd32(h, HWRTDATA_FLAGS),
                rd64(h, HWRTDATA_PM_ALIST_STACK_POINTER),
                rd32(h, HWRTDATA_PM_MLIST_STACK_POINTER),
            )
        }
    } else {
        (0, 0, 0, 0)
    };
    km_log!(
        k,
        Wrn,
        "render: runaway batch: {} grows served in one FWCCB batch; TA_CYCLE=0x{:x} TA_PHASE=0x{:x} TE_SIG=0x{:x} VCE=0x{:x} VDM_CTX_STORE=0x{:x}; PM_CAT VCE0=0x{:x} TE0=0x{:x} ALIST0=0x{:x} PM 0x190=0x{:x} 0x288=0x{:x} stack 0x270=0x{:x} 0x280=0x{:x} 0x2c0=0x{:x}; HWRT set {} (last kick {}): state {} flags 0x{:x} alist_sp 0x{:x} mlist_sp 0x{:x}; vdm 0x{:x}",
        served,
        k.reg32(0x6020),
        k.reg32(0x6008),
        k.reg32(0x5028),
        k.reg32(0x5030),
        k.reg32(0x0430),
        k.reg64(0x1248),
        k.reg64(0x1250),
        k.reg64(0x1260),
        k.reg32(0x0190),
        k.reg32(0x0288),
        k.reg64(0x0270),
        k.reg64(0x0280),
        k.reg64(0x02c0),
        hs,
        r.seq,
        state,
        flags,
        alist_sp,
        mlist_sp,
        r.dbg_vdm
    );
}

/// The kicks in flight when a FWCCB batch was first read, said when the
/// batch held a freelist grow request: "render: at grow request N (sampled
/// when the batch was first read): last submitted S, last fragment done D;
/// K kicks in flight: A with TA done and 3D pending, B with TA pending".
/// The parameter manager's pages are held by kicks whose geometry has run
/// and whose fragment has not, so A (and the size of their geometry) is what
/// makes a list run out (docs/notes/pvr-render-pool.md, "the
/// grow scene"). A kick's geometry is done when its geometry UFO slot holds its
/// sequence or a later one; the slots are reused every [`GEOM_UFO_SLOTS`]
/// kicks, and `submit` keeps the kicks in flight within that.
///
/// The sample is taken before any command of the batch is handled or said.
/// 157b76e3 took it after the batch, behind its synchronous console lines
/// (tens of ms at 115200 baud), and read "0 kicks in flight" at every
/// request on the AM62P.
///
/// With `pvr-grow-scene` and a scene marked, the line ends "; scene S: K
/// kicks since its start (the first F), T ms": the kicks submitted since the
/// application's mark and the time since it, at the same sample. The
/// firmware's freelist words are not a live measure of the TA's use (its
/// allocated count reads 4 at every request of 70b79a74, and the stack top is
/// current - 1 - that), so two runs' grows are compared on the host's side.
#[derive(Clone, Copy)]
struct InFlight {
    seq: u32,
    done: u32,
    inflight: u32,
    ta_done: u32,
    /// The scene context: (scene, kicks since its mark, its first kick, ms).
    scene: Option<(u32, u32, u32, u32)>,
}

/// The scene context at sequence `seq` (see [`InFlight`]).
fn scene_context(gs: YdPvrGrowScene, seq: u32) -> Option<(u32, u32, u32, u32)> {
    if !cfg!(feature = "pvr-grow-scene") || gs.n == 0 {
        return None;
    }
    let (now, hz) = kick_times::clock()?;
    let ms = (now.wrapping_sub(gs.t0) * 1000 / u64::from(hz)) as u32;
    Some((gs.n, seq.wrapping_sub(gs.seq0), gs.seq0.wrapping_add(1), ms))
}

impl InFlight {
    fn take(k: &YdPvrKm, seq: u32, fufo_off: u32, gufo_off: u32, gs: YdPvrGrowScene) -> InFlight {
        k.invd(k.heap_cpu(fufo_off), 4);
        k.invd(k.heap_cpu(gufo_off), GEOM_UFO_SLOTS as usize * 4);
        let done = hrd(k, fufo_off, 0);
        // a fragment fence at or past the last submitted one: nothing in
        // flight (sequences compared as distances, as the UFOs wrap)
        let inflight = (seq.wrapping_sub(done) as i32).clamp(0, GEOM_UFO_SLOTS as i32) as u32;
        let ta_done = (0..inflight)
            .map(|i| seq.wrapping_sub(i))
            .filter(|&s| hrd(k, gufo_off, (geom_ufo_slot(s) * 4) as usize).wrapping_sub(s) as i32 >= 0)
            .count() as u32;
        InFlight { seq, done, inflight, ta_done, scene: scene_context(gs, seq) }
    }

    fn say(&self, k: &YdPvrKm) {
        if let Some((n, kicks, first, ms)) = self.scene {
            km_log!(
                k,
                Inf,
                "render: at grow request {} (sampled when the batch was first read): last submitted {}, last fragment done {}; {} kicks in flight: {} with TA done and 3D pending, {} with TA pending; scene {}: {} kicks since its start (the first {}), {} ms",
                k.fl_grow_reqs,
                self.seq,
                self.done,
                self.inflight,
                self.ta_done,
                self.inflight - self.ta_done,
                n,
                kicks,
                first,
                ms
            );
            return;
        }
        km_log!(
            k,
            Inf,
            "render: at grow request {} (sampled when the batch was first read): last submitted {}, last fragment done {}; {} kicks in flight: {} with TA done and 3D pending, {} with TA pending",
            k.fl_grow_reqs,
            self.seq,
            self.done,
            self.inflight,
            self.ta_done,
            self.inflight - self.ta_done
        );
    }
}

/// The application's scene starts now (feature `pvr-grow-scene`): the
/// grow lines that follow count kicks and time from here. Without the
/// feature it does nothing, and the grow line is the old one.
pub fn scene_begin(r: &mut YdPvrRender) {
    if !cfg!(feature = "pvr-grow-scene") {
        return;
    }
    let t0 = kick_times::clock().map_or(0, |(t, _)| t);
    r.grow_scene = YdPvrGrowScene { n: r.grow_scene.n.wrapping_add(1), seq0: r.seq, t0 };
}

/// The words [`yd_pvr_render_fl_counters`] writes.
pub const FL_COUNTERS: usize = 9;

/// The freelist and FWCCB counters a reproducer reads between its phases
/// (gl_test case 174, the lost FWCCB command): grow requests, grows served,
/// grows refused, the firmware's out-of-memory notices, FWCCB slots read
/// empty and of those filled within the check's bound (both 0 without
/// `CONFIG_YAKOGL_DRV_GPU_PVR_FWCCB_CHECK`), the wait's re-posts, the stop
/// cause (0: running), and the firmware's partial renders (UPDATE_STATS
/// element 1: 0 in every run so far, the partial-render gap).
pub fn fl_counters(k: &YdPvrKm, r: &YdPvrRender) -> [u32; FL_COUNTERS] {
    [
        k.fl_grow_reqs,
        k.fl_grows_done,
        k.fl_no_grow,
        k.fw_stats[FWCCB_STATS_OUT_OF_MEMORY as usize],
        k.fwccb_empty,
        k.fwccb_filled,
        r.reposts,
        k.stop_cause,
        k.fw_stats[FWCCB_STATS_PARTIAL_RENDERS as usize],
    ]
}

/// # Safety
/// `k`, `r` valid; `out` has [`FL_COUNTERS`] words.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_render_fl_counters(k: *const YdPvrKm, r: *const YdPvrRender, out: *mut u32) {
    // SAFETY: the caller's contract.
    let c = unsafe { fl_counters(&*k, &*r) };
    // SAFETY: as above.
    unsafe { core::ptr::copy_nonoverlapping(c.as_ptr(), out, FL_COUNTERS) };
}

/// # Safety
/// `r` valid (the render service's state, owned by driver/src/pvr_render.c,
/// not in use by a submit or wait for the call).
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_render_scene_begin(r: *mut YdPvrRender) {
    // SAFETY: the caller's contract.
    scene_begin(unsafe { &mut *r })
}

fn fwccb_service_r_inner(k: &mut YdPvrKm, r: &mut YdPvrRender, snap: &mut Option<InFlight>) -> i32 {
    let (seq, fufo_off, gufo_off, gs) = (r.seq, r.fufo_off, r.gufo_off, r.grow_scene);
    let mut first = |k: &YdPvrKm| *snap = Some(InFlight::take(k, seq, fufo_off, gufo_off, gs));
    // the HWRT datas a freelist reconstruction resets: every built member
    let mut hwrts = [0u32; HWRT_SETS];
    let mut n = 0;
    for s in r.hwrt.iter().filter(|s| s.fw_built != 0) {
        hwrts[n] = s.hrt_off;
        n += 1;
    }
    if r.cfg.fl_grow_on_demand == 0 {
        return fwccb_service_with(k, None, &hwrts[..n], Some(&mut first));
    }
    let gen_b = &mut r.gen_b;
    let mut src = |k: &mut YdPvrKm, bytes: u32| {
        let bo = bo_alloc(k, GENERAL_HEAP_BASE, gen_b, bytes, L0_FLAGS_DATA);
        (!bo.cpu.is_null()).then_some(bo)
    };
    fwccb_service_with(k, Some(&mut src), &hwrts[..n], Some(&mut first))
}

/// The time a fence may stand still before the render service stops
/// ([`super::freelist::STOP_KICK_TIMEOUT`]), in every configuration. Half the board's
/// "alive" window (`CONFIG_YAKOGL_BOARD_ALIVE_S`, 20 s): the stop then
/// prints the driver's run-end lines (about 15 lines at 115200 baud, about
/// 0.2 s) and the record well before the watchdog. It is over 12 times the
/// longest frame on record (refract 435 ms; phong in the 536-recovery loop
/// 787 ms). `YD_PVR_STOP_WAIT_MS` in yakogl_drivers.h is the same number,
/// and pvr_render.c asserts the window is at least twice it.
pub const STOP_WAIT_MS: u32 = 10_000;

/// A wait on a stopped render service ([`super::freelist::stop`]): the
/// fence is not waited for. The first one is said and its fence kept for
/// the record. -ESHUTDOWN.
fn stopped(k: &mut YdPvrKm, fence: u32, who: &str) -> i32 {
    if k.stop_fence == 0 {
        k.stop_fence = fence;
    }
    if k.stop_said == 0 {
        k.stop_said = 1;
        km_log!(
            k,
            Err,
            "render: fence {} ({}) cannot complete: the render service stopped ({}); not waited for",
            fence,
            who,
            super::freelist::stop_name(k.stop_cause)
        );
    }
    -errno::ESHUTDOWN
}

/// Wait for fragment fence `fence`, `timeout_ms` at 1 ms, serving the
/// firmware's FWCCB requests meanwhile; `who` names the call site in the
/// log. The UFO is read every [`WAIT_POLL_US`]. 0, -EIO on a hardware
/// recovery during the wait, -ESHUTDOWN once the render service stopped
/// ([`stopped`]), -ETIMEDOUT.
pub fn wait_for(k: &mut YdPvrKm, r: &mut YdPvrRender, fence: u32, timeout_ms: u32, who: &str) -> i32 {
    if r.inited == 0 {
        return -errno::EINVAL;
    }
    let hib = k.heap_cpu(k.hwrinfobuf_off);
    kick_times::wait_begins(r, fence);
    for i in 0..timeout_ms {
        if fence_reached(k, r, fence) {
            if r.cfg.verbose != 0 {
                render_debug_after(k, r, fence, i);
            }
            return 0;
        }
        // A fence that has not moved for STOP_WAIT_MS stops the render
        // service, before the board's watchdog ends the run without a record.
        // In every configuration: with the knob off the old 20 s wait tied
        // with the 20 s alive window (the common set's round 2).
        if i == STOP_WAIT_MS {
            super::freelist::stop(k, super::freelist::STOP_KICK_TIMEOUT, 0, 0);
        }
        if k.stop_cause != 0 {
            return stopped(k, fence, who);
        }
        // Serve the firmware's requests (freelist grow / reconstruction), so
        // a recovering firmware is never left waiting on the host.
        fwccb_service_r(k, r);
        if k.stop_cause != 0 {
            return stopped(k, fence, who);
        }
        // Re-ring the doorbell (wait_rering, a probe for a missed
        // notification).
        if r.cfg.wait_rering != 0 && (i & 0x7) == 0x7 {
            kccb_doorbell(k);
        }
        // Once, 20 ms in, send the same kick again, unchanged (wait_repost:
        // ON by default as a mitigation, 2026-09-24: the firmware sits on a
        // posted kick until another command arrives; 0 of 60 runs of gl_test
        // case 13 stalled with it, about half without).
        if r.cfg.wait_repost != 0 && i == 20 {
            for c in &r.last_kick[..r.last_kick_n as usize] {
                kccb_post(k, c);
            }
            r.reposts = r.reposts.wrapping_add(1);
            // the first few, then every 256th: console output costs
            if r.reposts <= 8 || (r.reposts & 0xff) == 0 {
                km_log!(
                    k,
                    Wrn,
                    "render: fence {} ({}) not moved at 20 ms, re-posted {} kick(s) ({} so far)",
                    fence,
                    who,
                    r.last_kick_n,
                    r.reposts
                );
            }
        }
        if r.cfg.verbose != 0 && (i == 2 || i == 10) {
            verbose_regs(k, r, i);
        }
        k.invd(hib, HWRINFOBUF_SIZE);
        let ctr = hrd(k, k.hwrinfobuf_off, HWRINFOBUF_HWR_COUNTER);
        if ctr != r.hwr_seen {
            km_log!(k, Err, "render: HWR during fence {} (counter {} -> {})", fence, r.hwr_seen, ctr);
            r.hwr_seen = ctr;
            for e in 0..HWINFO_MAX as usize {
                let b = HWRINFOBUF_HWR_INFO + e * HWRINFO_SIZE;
                let evt = hrd(k, k.hwrinfobuf_off, b + HWRINFO_EVENT_STATUS);
                let ty = hrd(k, k.hwrinfobuf_off, b + HWRINFO_HWR_TYPE);
                let dm = hrd(k, k.hwrinfobuf_off, b + HWRINFO_DM);
                // SAFETY: the HWR info buffer.
                let (mmu, req, pc) = unsafe {
                    (
                        rd64(hib, b + HWRINFO_BIF_MMU_STATUS),
                        rd64(hib, b + HWRINFO_BIF_REQ_STATUS),
                        rd64(hib, b + HWRINFO_BIF_PC_ADDRESS),
                    )
                };
                if evt == 0 && ty == 0 && dm == 0 && mmu == 0 {
                    continue;
                }
                // BIF_FAULT_BANK_REQ_STATUS, __XE_MEM__ layout (this core has
                // XE_MEMORY_HIERARCHY): ADDRESS 39..4, TAG_ID 45..40, TAG_SB
                // 51..46, read-not-write 52 (rgx_cr_defs_km.h,
                // RGX_CR_BIF_FAULT_BANK0_REQ_STATUS__XE_MEM__*).
                let tag_id = ((req >> 40) & 0x3F) as u32;
                let tag_sb = ((req >> 46) & 0x3F) as u32;
                km_log!(
                    k,
                    Err,
                    "  HWR[{}] dm={} type={} evt=0x{:08x} rflags=0x{:08x} hwrno={} BIF {} VA=0x{:010x} {} tag_id={} tag_sb={} (req=0x{:016x}) mmu=0x{:016x} pc=0x{:016x}",
                    e,
                    dm,
                    ty,
                    evt,
                    hrd(k, k.hwrinfobuf_off, b + HWRINFO_RECOVERY_FLAGS),
                    hrd(k, k.hwrinfobuf_off, b + HWRINFO_HWR_NUMBER),
                    if req & (1u64 << 52) != 0 { "read" } else { "write" },
                    req & 0x0000_00FF_FFFF_FFF0,
                    mh_tag_name(tag_id),
                    tag_id,
                    tag_sb,
                    req,
                    mmu,
                    pc
                );
            }
            render_debug_after(k, r, fence, i);
            dump_last_frag_cmd(k, r, fence);
            return -errno::EIO;
        }
        // The rest of this millisecond, in WAIT_POLL_US steps with a read
        // after each but the last (the next iteration's read is that one).
        // With the 1000 us step this is one busy_wait_us(1000), the call
        // sequence of before the knob. (A counted loop: with the 1000 us step
        // the range 1..1 is empty, which clippy refuses as a literal range.)
        let mut step = 1;
        while step < 1000 / WAIT_POLL_US {
            k.busy_wait_us(WAIT_POLL_US);
            if fence_reached(k, r, fence) {
                if r.cfg.verbose != 0 {
                    render_debug_after(k, r, fence, i);
                }
                return 0;
            }
            step += 1;
        }
        k.busy_wait_us(WAIT_POLL_US);
    }
    km_log!(k, Err, "render: fence {} timeout (ufo={}) waiting on the {}", fence, hrd(k, r.fufo_off, 0), who);
    dump_last_frag_cmd(k, r, fence);
    // The first stall is the one worth explaining: dump the firmware's trace
    // tail and the freelist the PM is tiling into -- twice: the first stall
    // of a boot, and the first after the host had to refuse a grow.
    let refused = k.fl_no_grow > 0;
    if r.stall_dumped == 0 || (refused && r.stall_dumped_after_refusal == 0) {
        r.stall_dumped = 1;
        r.stall_dumped_after_refusal = refused as u32;
        let gfl = k.heap_cpu(r.gfl_off);
        k.invd(gfl, FREELIST_SIZE);
        // The PM's own view: its catalogue bases and the stack it pops from.
        km_log!(
            k,
            Err,
            "render: stall: PM_CAT VCE0=0x{:x} TE0=0x{:x} ALIST0=0x{:x}  PM alloc 0x190=0x{:x} 0x288=0x{:x}  stack 0x270=0x{:x} 0x280=0x{:x} 0x2c0=0x{:x}",
            k.reg64(0x1248),
            k.reg64(0x1250),
            k.reg64(0x1260),
            k.reg32(0x0190),
            k.reg32(0x0288),
            k.reg64(0x0270),
            k.reg64(0x0280),
            k.reg64(0x02c0)
        );
        {
            // Three offsets per client CCB: the firmware's read, the
            // firmware's write (set from the kick, never by the host) and
            // ours; fw_w < host means the kick never took effect.
            let (g, f, kc) = (r.gcccbctl_off, r.fcccbctl_off, k.kccb_ctl_off);
            k.invd(k.heap_cpu(g), CCCB_CTL_SIZE);
            k.invd(k.heap_cpu(f), CCCB_CTL_SIZE);
            k.invd(k.heap_cpu(kc), CCB_CTL_SIZE);
            k.invd(k.heap_cpu(r.gufo_off), 4);
            k.invd(k.heap_cpu(r.fufo_off), 4);
            k.invd(k.heap_cpu(g), CCCB_CTL_SIZE);
            k.invd(k.heap_cpu(f), CCCB_CTL_SIZE);
            km_log!(
                k,
                Err,
                "render: stall: geom rd={} fw_w={} host={} | frag rd={} fw_w={} host={} | KCCB rd={} w={} | gufo[seq {}]={} (slot0={}) fufo={} seq={} fl_refusals={}",
                hrd(k, g, CCCB_CTL_READ_OFFSET),
                hrd(k, g, CCCB_CTL_WRITE_OFFSET),
                r.gwo,
                hrd(k, f, CCCB_CTL_READ_OFFSET),
                hrd(k, f, CCCB_CTL_WRITE_OFFSET),
                r.fwo,
                hrd(k, kc, CCB_CTL_READ_OFFSET),
                hrd(k, kc, CCB_CTL_WRITE_OFFSET),
                fence,
                hrd(k, r.gufo_off, (geom_ufo_slot(fence) * 4) as usize),
                hrd(k, r.gufo_off, 0),
                hrd(k, r.fufo_off, 0),
                r.seq,
                k.fl_no_grow
            );
            let (grd, frd) = (hrd(k, g, CCCB_CTL_READ_OFFSET), hrd(k, f, CCCB_CTL_READ_OFFSET));
            dump_cccb_pending(k, "geom", k.heap_cpu(r.gcccb_off), grd, r.gwo);
            dump_cccb_pending(k, "frag", k.heap_cpu(r.fcccb_off), frd, r.fwo);
        }
        // grow_pending is written by the firmware; the counters say whether
        // we saw the request and answered.
        // SAFETY: the global freelist's RGXFWIF_FREELIST.
        let pending = unsafe { gfl.add(FREELIST_GROW_PENDING).read_volatile() };
        km_log!(
            k,
            Err,
            "render: stall: freelist1 cur={} top={} max={} grow={} ready={} pending={} alloc={}/{}; fwccb grow req/done={}/{}",
            hrd(k, r.gfl_off, FREELIST_CURRENT_PAGES),
            hrd(k, r.gfl_off, FREELIST_CURRENT_STACK_TOP),
            hrd(k, r.gfl_off, FREELIST_MAX_PAGES),
            hrd(k, r.gfl_off, FREELIST_GROW_PAGES),
            hrd(k, r.gfl_off, FREELIST_READY_PAGES),
            pending,
            hrd(k, r.gfl_off, FREELIST_ALLOCATED_PAGE_COUNT),
            hrd(k, r.gfl_off, FREELIST_ALLOCATED_MMU_PAGE_COUNT),
            k.fl_grow_reqs,
            k.fl_grows_done
        );
        trace_tail(k, true, "stall: FW trace ", 600);
    }
    -errno::ETIMEDOUT
}

/// PVR_RENDER_VERBOSE while a kick runs (t+2 / t+10 ms): the BIF/MMU fault
/// banks, the geometry, PM and freelist-load registers, and at t+10 the FW
/// trace.
fn verbose_regs(k: &YdPvrKm, r: &YdPvrRender, i: u32) {
    km_log!(
        k,
        Inf,
        "render: t+{}ms TEXAS_BIF fault(0x1430)=0x{:x} req(0x1438)=0x{:x} (VA=0x{:x}) MMU_FS1(0xE150)=0x{:x} FS2(0xE158)=0x{:x} PM(0xE130)=0x{:x} META(0xE160)=0x{:x} SAFETY=0x{:x}",
        i,
        k.reg64(0x1430),
        k.reg64(0x1438),
        (k.reg64(0x1438) >> 4) << 4,
        k.reg64(0xE150),
        k.reg64(0xE158),
        k.reg64(0xE130),
        k.reg64(0xE160),
        k.reg32(0xF398)
    );
    km_log!(
        k,
        Inf,
        "render: t+{}ms GEOM: TA_CYCLE=0x{:x} TA_PHASE=0x{:x} TE_SIG=0x{:x} TE_CKSM=0x{:x} VCE=0x{:x} PPP_SIG=0x{:x} USC_UVS0=0x{:x} PDS_DOUTM=0x{:x} VDM_CTX_STORE=0x{:x} CLK=0x{:x} SPU_EN=0x{:x}",
        i,
        k.reg32(0x6020),
        k.reg32(0x6008),
        k.reg32(0x5028),
        k.reg32(0x5110),
        k.reg32(0x5030),
        k.reg32(0x5020),
        k.reg32(0x5000),
        k.reg32(0x5060),
        k.reg32(0x0430),
        k.reg64(0x0008),
        k.reg32(0x0050)
    );
    km_log!(
        k,
        Inf,
        "render: t+{}ms PM: CAT_VCE0=0x{:x} TE0=0x{:x} ALIST0=0x{:x} 0x3d0=0x{:x} 0x270=0x{:x} 0x280=0x{:x} BIF_CTRL=0x{:x} MTS_INT=0x{:x}",
        i,
        k.reg64(0x1248),
        k.reg64(0x1250),
        k.reg64(0x1260),
        k.reg32(0x03d0),
        k.reg64(0x0270),
        k.reg64(0x0280),
        k.reg64(0x12a8),
        k.reg32(0x0bb0)
    );
    km_log!(
        k,
        Inf,
        "render: t+{}ms FLLOAD: 0x200=0x{:x} 0x208=0x{:x} base 0x210=0x{:x} 0x218=0x{:x} stack 0x220=0x{:x} 0x230=0x{:x} gate 0x388=0x{:x} alloc 0x390=0x{:x} 0x398=0x{:x} 0x2d8=0x{:x} 0x268=0x{:x} 0x2b8=0x{:x} BIFCAT_IDX=0x{:x} CAT0=0x{:x} CAT1=0x{:x} CAT2=0x{:x}",
        i,
        k.reg32(0x0200),
        k.reg32(0x0208),
        k.reg64(0x0210),
        k.reg64(0x0218),
        k.reg64(0x0220),
        k.reg64(0x0230),
        k.reg32(0x0388),
        k.reg64(0x0390),
        k.reg64(0x0398),
        k.reg64(0x02d8),
        k.reg64(0x0268),
        k.reg64(0x02b8),
        k.reg64(0x1240),
        k.reg64(0x1200),
        k.reg64(0x1208),
        k.reg64(0x1210)
    );
    if i == 10 {
        let tb = k.heap_cpu(k.trace_buf_off);
        let tptr = hrd(k, k.tracebuf_off, TRACEBUF_TRACE_POINTER);
        let from = tptr.saturating_sub(420);
        k.invd(tb, tptr as usize * 4);
        km_log!(
            k,
            Inf,
            "render: t+10ms SAFETY_EVT(0xF398)=0x{:x} EVENT_STATUS=0x{:x} TA_CYCLE=0x{:x} TA_PHASE=0x{:x} geom_ufo={} trace ptr={}",
            k.reg32(0xF398),
            k.reg32(0x0130),
            k.reg32(0x6020),
            k.reg32(0x6008),
            hrd(k, r.gufo_off, 0),
            tptr
        );
        let mut j = from;
        while j + 6 <= tptr {
            // SAFETY: the trace ring.
            let t = |x: u32| unsafe { rd32(tb, 4 * x as usize) };
            km_log!(
                k,
                Inf,
                " e[{}]: {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}",
                j,
                t(j),
                t(j + 1),
                t(j + 2),
                t(j + 3),
                t(j + 4),
                t(j + 5)
            );
            j += 6;
        }
    }
}

// ---- C entry points ---------------------------------------------------------------
//
// Every one: `k` is the C driver's `struct yd_pvr_km` (see [`km`]) and `r` the
// render service's `struct yd_pvr_render`, both valid and used by nobody else
// for the call.

/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_render_ctx_init(k: *mut YdPvrKm, r: *mut YdPvrRender) {
    // SAFETY: the caller's contract.
    unsafe { ctx_init(km(k), &mut *r) }
}

/// # Safety
/// See the section comment; `kick` and `fence_out` valid.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_render_submit(
    k: *mut YdPvrKm,
    r: *mut YdPvrRender,
    kick: *const PvrRenderKick,
    fence_out: *mut u32,
) -> i32 {
    if kick.is_null() || fence_out.is_null() {
        return -errno::EINVAL;
    }
    // SAFETY: the caller's contract.
    unsafe { submit(km(k), &mut *r, &*kick, &mut *fence_out) }
}

/// # Safety
/// See the section comment; `who` a NUL-terminated string (or null).
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_render_wait(
    k: *mut YdPvrKm,
    r: *mut YdPvrRender,
    fence: u32,
    timeout_ms: u32,
    who: *const c_char,
) -> i32 {
    let who = if who.is_null() {
        ""
    } else {
        // SAFETY: the caller's contract.
        unsafe { CStr::from_ptr(who) }.to_str().unwrap_or("?")
    };
    // SAFETY: the caller's contract.
    unsafe { wait_for(km(k), &mut *r, fence, timeout_ms, who) }
}

/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_render_completed(k: *const YdPvrKm, r: *const YdPvrRender) -> u32 {
    // SAFETY: the caller's contract.
    unsafe { completed(&*k, &*r) }
}

// ---- the run-end owner report ------------------------------------------------

/// Pool bytes of one driver owner, per device-VA heap: [general, pds, usc,
/// pm meta].
#[derive(Clone, Copy, Default)]
struct DrvTally {
    blocks: u32,
    bytes: [u64; 4],
}

impl DrvTally {
    fn add(&mut self, heap: usize, bytes: u64) {
        if bytes == 0 {
            return;
        }
        self.blocks += 1;
        self.bytes[heap] += round_up(bytes, 4096);
    }

    fn add_bo(&mut self, bo: &PvrGpuBo, bytes: u32) {
        if bo.cpu.is_null() {
            return;
        }
        let heap =
            if bo.gpu >= PMMETA_PROTECT_HEAP_BASE && bo.gpu < PMMETA_PROTECT_HEAP_BASE + 0x4000_0000 { 3 } else { 0 };
        self.add(heap, u64::from(bytes));
    }

    /// "pool owner <name>: N blocks, K KiB (general a, pds b, usc c, pm d)";
    /// returns K.
    fn report(&self, k: &YdPvrKm, name: &str) -> u64 {
        let kib = |i: usize| self.bytes[i] / 1024;
        let total = self.bytes.iter().sum::<u64>() / 1024;
        km_log!(
            k,
            Inf,
            "pool owner {}: {} blocks, {} KiB (general {}, pds {}, usc {}, pm {})",
            name,
            self.blocks,
            total,
            kib(0),
            kib(1),
            kib(2),
            kib(3)
        );
        total
    }
}

/// The driver's lines of the run-end owner report
/// (`CONFIG_YAKOGL_PVR_POOL_OWNERS`,
/// docs/notes/pvr-render-pool.md): the render service's
/// one-time objects (`statics`: their page-rounded bytes per heap [general,
/// pds, usc], counted by the C side that made them, in
/// `nstatics` blocks), the freelists (backing pages, PFN arrays, the shared
/// grow pool), the render context's VDM stream, and the HWRT sets with the
/// target sizes they were built for. Returns their KiB.
pub fn pool_owners(k: &YdPvrKm, r: &YdPvrRender, statics: [u32; 3], nstatics: u32) -> u64 {
    let mut st = DrvTally { blocks: nstatics, ..DrvTally::default() };
    for (i, b) in statics.iter().enumerate() {
        st.bytes[i] = u64::from(*b);
    }
    let mut fl = DrvTally::default();
    for f in &k.fls[..(k.nfls as usize).min(FL_MAX)] {
        fl.add(0, u64::from(f.backed + f.reserve) * 4096);
        fl.add(3, round_up(u64::from(f.maxpages) * 4, 0x4000));
        /* the blocks taken from the render pool for on-demand grows and
         * refills, each one grow + 3 pages (freelist::ondemand_bytes) */
        let each = u64::from(super::freelist::ondemand_bytes(f.grow));
        let taken = u64::from(f.pool_kib) * 1024;
        for _ in 0..taken.checked_div(each).unwrap_or(0) {
            fl.add(0, each);
        }
        fl.add(0, taken.checked_rem(each).unwrap_or(taken));
    }
    fl.add(0, u64::from(k.fl_pool_pages) * 4096);
    let mut ctx = DrvTally::default();
    ctx.add_bo(&r.vdm_cs, 4096);
    let mut hw = DrvTally::default();
    let mut sizes = 0u32;
    for s in r.hwrt.iter() {
        // Every block a member holds is counted, built or not: a member whose
        // build was refused part way keeps what it got (hwrt_get), and that
        // is pool memory too (the AM62P, 4c25118c: a 1600x1200 member held
        // only its 8 KiB vheap, which this report used to skip). "Members
        // built" counts the complete ones.
        let blocks = [&s.tpc, &s.vheap, &s.rgnhdr, &s.macrotile, &s.mlist];
        if blocks.iter().all(|b| !b.cpu.is_null()) {
            sizes += 1;
        }
        hw.add_bo(&s.tpc, s.tpc_alloc);
        hw.add_bo(&s.vheap, VHEAP_BYTES);
        hw.add_bo(&s.rgnhdr, s.rgn_alloc);
        hw.add_bo(&s.macrotile, 4096);
        hw.add_bo(&s.mlist, round_up(u64::from(MLIST_BYTES), 0x4000) as u32);
    }
    let kib = st.report(k, "render statics") + fl.report(k, "freelists") + ctx.report(k, "render context");
    let hkib = hw.report(k, "hwrt sets");
    km_log!(k, Inf, "pool owner hwrt sets: {} members built of {}", sizes, HWRT_SETS);
    kib + hkib
}

/// # Safety
/// `k` and `r` are the C driver's, valid and not used by anyone else for the
/// call; `statics` points at 3 words.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_render_pool_owners(
    k: *mut YdPvrKm,
    r: *const YdPvrRender,
    statics: *const u32,
    nstatics: u32,
) -> u64 {
    // SAFETY: the caller's contract.
    let (k, r, st) = unsafe { (km(k), &*r, core::slice::from_raw_parts(statics, 3)) };
    pool_owners(k, r, [st[0], st[1], st[2]], nstatics)
}

// ---- freelist grows and GPU recoveries, for a run's end ------------------------

/// The firmware's hardware-recovery counter (`RGXFWIF_HWRINFOBUF`,
/// `ui32HwrCounter`): it counts every recovery since the firmware booted.
pub fn hwr_counter(k: &YdPvrKm) -> u32 {
    hrd(k, k.hwrinfobuf_off, HWRINFOBUF_HWR_COUNTER)
}

/// # Safety
/// `k` is the C driver's, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_render_hwr_counter(k: *const YdPvrKm) -> u32 {
    // SAFETY: the caller's contract.
    hwr_counter(unsafe { &*k })
}

/// The run-end lines of the freelists' grows and the GPU's recoveries:
/// "render freelists: R grow requests, S served, U unserved, K KiB taken on
/// demand; GPU recoveries N (HWR counter C)", then the refills the render
/// pool refused and the firmware's UPDATE_STATS notices by element
/// ([`super::ccb::update_stats`]). `since` is the HWR counter when the render
/// service started.
pub fn grow_report(k: &mut YdPvrKm, since: u32) {
    let kib: u32 = k.fls[..(k.nfls as usize).min(FL_MAX)].iter().map(|f| f.pool_kib).sum();
    let c = hwr_counter(k);
    km_log!(
        k,
        Inf,
        "render freelists: {} grow requests, {} served, {} unserved, {} KiB taken on demand; GPU recoveries {} (HWR counter {})",
        k.fl_grow_reqs,
        k.fl_grows_done,
        k.fl_no_grow,
        kib,
        c.wrapping_sub(since),
        c
    );
    let s = k.fw_stats;
    km_log!(
        k,
        Inf,
        "render freelists: reserve refills refused {} ({} KiB); firmware UPDATE_STATS: partial renders {}, out of memory {}, TA stores {}, 3D stores {}, CDM stores {}, TDM stores {}, other {}",
        k.fl_refills_refused,
        k.fl_refill_refused_kib,
        s[1],
        s[2],
        s[3],
        s[4],
        s[5],
        s[6],
        s[0]
    );
    if k.fwccb_check != 0 {
        km_log!(
            k,
            Inf,
            "render fwccb: slots read empty {} (filled within 2000 us {}, still empty {}); write offset went back {}",
            k.fwccb_empty,
            k.fwccb_filled,
            k.fwccb_empty.wrapping_sub(k.fwccb_filled),
            k.fwccb_wo_back
        );
    }
}

/// # Safety
/// `k` is the C driver's, valid for the call.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_render_grow_report(k: *mut YdPvrKm, since: u32) {
    // SAFETY: the caller's contract.
    grow_report(unsafe { km(k) }, since)
}

#[cfg(test)]
mod owner_tests {
    use super::*;
    use crate::gpu::pvr::km::YdPvrKmOps;
    use std::boxed::Box;
    use std::cell::RefCell;
    use std::string::String;
    use std::vec::Vec;

    std::thread_local! {
        static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }
    unsafe extern "C" fn nop_range(_: *mut c_void, _: *const c_void, _: usize) {}
    unsafe extern "C" fn nop_fence(_: *mut c_void) {}
    unsafe extern "C" fn nop_wait(_: *mut c_void, _: u32) {}
    unsafe extern "C" fn log(_: *mut c_void, _: u32, buf: *const u8, len: usize) {
        // SAFETY: the crate passes `len` valid bytes.
        let s = unsafe { core::slice::from_raw_parts(buf, len) };
        LOG.with(|l| l.borrow_mut().push(String::from_utf8_lossy(s).into_owned()));
    }

    fn bo(gpu: u64) -> PvrGpuBo {
        PvrGpuBo { gpu, cpu: 0x1000 as *mut u8, heap_off: 0 }
    }

    /// The driver's owner lines, from the formulas in
    /// docs/notes/pvr-render-pool.md: two global freelists of
    /// the 64 MiB carveout (1536 backing pages, 4096-entry PFN arrays) and the
    /// local one (26 pages, 32 entries), the render context's VDM stream, and
    /// one HWRT member at its minimum sizes (TPC 16, VHEAP 8, region headers
    /// 64, macrotile 4 KiB, MList 16 KiB in the PM heap).
    #[test]
    fn the_driver_owner_lines_follow_the_formulas() {
        let ops = Box::new(YdPvrKmOps {
            ctx: core::ptr::null_mut(),
            flush: nop_range,
            invd: nop_range,
            fence: nop_fence,
            busy_wait_us: nop_wait,
            log,
        });
        // SAFETY: all-zero is the documented state before the first call.
        let (mut k, mut r): (Box<YdPvrKm>, Box<YdPvrRender>) =
            unsafe { (Box::new(core::mem::zeroed()), Box::new(core::mem::zeroed())) };
        k.ops = &*ops;
        for (i, (backed, reserve, max)) in [(512, 1024, 4096), (512, 1024, 4096), (26, 0, 32)].into_iter().enumerate() {
            k.fls[i].backed = backed;
            k.fls[i].reserve = reserve;
            k.fls[i].maxpages = max;
        }
        k.nfls = 3;
        r.vdm_cs = bo(GENERAL_HEAP_BASE);
        let s = &mut r.hwrt[0];
        (s.tpc, s.vheap, s.rgnhdr, s.macrotile) =
            (bo(GENERAL_HEAP_BASE), bo(GENERAL_HEAP_BASE), bo(GENERAL_HEAP_BASE), bo(GENERAL_HEAP_BASE));
        s.mlist = bo(PMMETA_PROTECT_HEAP_BASE);
        (s.tpc_alloc, s.rgn_alloc) = (0x4000, 0x10000);
        LOG.with(|l| l.borrow_mut().clear());
        let kib = pool_owners(&k, &r, [2048 * 1024 + 4096 + 2 * 32768, 4096, 3 * 4096], 8);
        let logs = LOG.with(|l| l.borrow().clone());
        assert_eq!(
            logs,
            [
                "pool owner render statics: 8 blocks, 2132 KiB (general 2116, pds 4, usc 12, pm 0)",
                "pool owner freelists: 6 blocks, 12440 KiB (general 12392, pds 0, usc 0, pm 48)",
                "pool owner render context: 1 blocks, 4 KiB (general 4, pds 0, usc 0, pm 0)",
                "pool owner hwrt sets: 5 blocks, 108 KiB (general 92, pds 0, usc 0, pm 16)",
                "pool owner hwrt sets: 1 members built of 32",
            ]
        );
        assert_eq!(kib, 2132 + 12440 + 4 + 108);
    }

    fn km_with_log(ops: &YdPvrKmOps) -> Box<YdPvrKm> {
        // SAFETY: all-zero is the documented state before the first call.
        let mut k: Box<YdPvrKm> = unsafe { Box::new(core::mem::zeroed()) };
        k.ops = ops;
        LOG.with(|l| l.borrow_mut().clear());
        k
    }

    fn ops() -> Box<YdPvrKmOps> {
        Box::new(YdPvrKmOps {
            ctx: core::ptr::null_mut(),
            flush: nop_range,
            invd: nop_range,
            fence: nop_fence,
            busy_wait_us: nop_wait,
            log,
        })
    }

    /// The knob's lists at glmark2's close (gate 3's prediction for the
    /// refill): each global list keeps one grow (256 pages) behind its 512,
    /// list 1 took two refills of 1036 KiB. Every block taken from the render
    /// pool is its own block: 6 + 2 = 8 (gate 3's line counted list 1's two
    /// on-demand blocks as one, "7 blocks, 6320 KiB").
    #[test]
    fn each_block_taken_on_demand_is_its_own_owner_block() {
        let ops = ops();
        let mut k = km_with_log(&ops);
        // SAFETY: all-zero is a valid render state for the report.
        let r: Box<YdPvrRender> = unsafe { Box::new(core::mem::zeroed()) };
        for (i, (backed, reserve, max, grow, kib)) in
            [(768, 0, 4096, 256, 2072), (512, 256, 4096, 256, 0), (26, 0, 32, 0, 0)].into_iter().enumerate()
        {
            let f = &mut k.fls[i];
            (f.backed, f.reserve, f.maxpages, f.grow, f.pool_kib) = (backed, reserve, max, grow, kib);
        }
        k.nfls = 3;
        pool_owners(&k, &r, [0, 0, 0], 0);
        let logs = LOG.with(|l| l.borrow().clone());
        assert_eq!(logs[1], "pool owner freelists: 8 blocks, 8368 KiB (general 8320, pds 0, usc 0, pm 48)");
    }

    /// A member whose build was refused after its vheap was served (the
    /// AM62P, 4c25118c, a 1600x1200 member: tpc refused, vheap 8 KiB served,
    /// rgnhdr, macrotile and mlist refused) holds 8 KiB of the pool. It is
    /// counted in "hwrt sets" and is not a built member. The report used to
    /// skip a member with neither tpc nor rgnhdr, so this was "unaccounted 8
    /// KiB".
    #[test]
    fn a_half_built_hwrt_member_is_counted_and_not_built() {
        let ops = ops();
        let k = km_with_log(&ops);
        // SAFETY: all-zero is a valid render state for the report.
        let mut r: Box<YdPvrRender> = unsafe { Box::new(core::mem::zeroed()) };
        r.hwrt[0].vheap = bo(GENERAL_HEAP_BASE);
        pool_owners(&k, &r, [0, 0, 0], 0);
        let logs = LOG.with(|l| l.borrow().clone());
        assert_eq!(
            logs[3..5],
            [
                "pool owner hwrt sets: 1 blocks, 8 KiB (general 8, pds 0, usc 0, pm 0)",
                "pool owner hwrt sets: 0 members built of 32",
            ]
        );
    }

    /// The run-end lines: the existing one unchanged, then the refused
    /// refills and the firmware's notices by element.
    #[test]
    fn the_grow_report_says_refills_and_notices() {
        let ops = ops();
        let mut k = km_with_log(&ops);
        let mut hib = std::vec![0u8; HWRINFOBUF_HWR_COUNTER + 4];
        hib[HWRINFOBUF_HWR_COUNTER] = 3;
        k.cpu = hib.as_mut_ptr();
        k.size = hib.len() as u64;
        k.fw_config_off = u32::MAX; // heap offsets map 1:1 to the buffer
        (k.fl_grow_reqs, k.fl_grows_done, k.fl_no_grow) = (3, 2, 1);
        k.fls[0].pool_kib = 2072;
        k.nfls = 1;
        (k.fl_refills_refused, k.fl_refill_refused_kib) = (1, 1036);
        k.fw_stats = [1, 0, 2, 0, 0, 0, 0];
        grow_report(&mut k, 1);
        let logs = LOG.with(|l| l.borrow().clone());
        assert_eq!(
            logs,
            [
                "render freelists: 3 grow requests, 2 served, 1 unserved, 2072 KiB taken on demand; GPU recoveries 2 (HWR counter 3)",
                "render freelists: reserve refills refused 1 (1036 KiB); firmware UPDATE_STATS: partial renders 0, out of memory 2, TA stores 0, 3D stores 0, CDM stores 0, TDM stores 0, other 1",
            ]
        );
    }
}

#[cfg(test)]
mod wait_tests {
    //! [`wait_for`] on a simulated clock: the mock's `busy_wait_us` advances
    //! time and writes the fence to the fragment UFO once the "GPU" has
    //! finished, so a test reads how long the wait spun past the completion.
    use super::*;
    use crate::gpu::pvr::km::YdPvrKmOps;
    use std::cell::{Cell, RefCell};

    /// Carveout offsets of the objects the wait reads (all in the firmware
    /// heap's main region: `fw_config_off` is past the buffer, so heap
    /// offsets map 1:1). The FWCCB control block stays zero (nothing
    /// pending), and so does the HWR counter.
    const FWCCB_CTL: u32 = 0x100;
    const FUFO: u32 = 0x200;
    /// The geometry UFO slots (GEOM_UFO_SLOTS words).
    const GUFO: u32 = 0x300;
    const HWRINFO: u32 = 0x1000;
    const CARVEOUT_WORDS: usize = 0x2000 / 4;
    /// The kick-times clock at the start of a rig (us).
    const T0: u64 = 1_000_000;

    struct Sim {
        /// Microseconds of busy wait so far.
        now_us: Cell<u64>,
        /// When the fence is written (None: never).
        done_at_us: Option<u64>,
        /// When the geometry fence is written (None: never).
        geom_at_us: Cell<Option<u64>>,
        fence: u32,
        ufo: *mut u32,
        gufo: *mut u32,
        steps: RefCell<Vec<u32>>,
        lines: RefCell<Vec<String>>,
        /// Write the fence to the fragment UFO when a line is said: the kick
        /// completes while the console prints (115200 baud on the AM62P).
        complete_on_log: Cell<bool>,
    }

    unsafe extern "C" fn nop_range(_: *mut c_void, _: *const c_void, _: usize) {}
    unsafe extern "C" fn nop(_: *mut c_void) {}
    unsafe extern "C" fn busy_wait(ctx: *mut c_void, us: u32) {
        // SAFETY: ctx is the test's Sim.
        let s = unsafe { &*(ctx as *const Sim) };
        s.steps.borrow_mut().push(us);
        s.now_us.set(s.now_us.get() + u64::from(us));
        // The kick-times clock reads the same simulated time, from T0 (a
        // stamp of 0 means "not yet").
        super::kick_times::test_clock::set(T0 + s.now_us.get());
        if s.geom_at_us.get().is_some_and(|t| s.now_us.get() >= t) {
            // SAFETY: the fence's geometry UFO slot of the test's carveout.
            unsafe { s.gufo.add(geom_ufo_slot(s.fence) as usize).write_volatile(s.fence) };
        }
        if s.done_at_us.is_some_and(|t| s.now_us.get() >= t) {
            // SAFETY: the UFO word of the test's carveout.
            unsafe { s.ufo.write_volatile(s.fence) };
        }
    }
    unsafe extern "C" fn log(ctx: *mut c_void, _: u32, b: *const u8, n: usize) {
        // SAFETY: ctx is the test's Sim; b is valid for n bytes.
        let s = unsafe { &*(ctx as *const Sim) };
        let t = String::from_utf8_lossy(unsafe { core::slice::from_raw_parts(b, n) }).into_owned();
        s.lines.borrow_mut().push(t);
        if s.complete_on_log.get() {
            // SAFETY: the UFO word of the test's carveout.
            unsafe { s.ufo.write_volatile(s.fence) };
        }
    }

    struct Rig {
        mem: Vec<u32>,
        sim: Box<Sim>,
        /// Kept alive for `k.ops`.
        _ops: Box<YdPvrKmOps>,
        k: Box<YdPvrKm>,
        r: Box<YdPvrRender>,
    }

    /// A wait on `fence`, the "GPU" writing it after `done_at_us` of busy
    /// wait (None: never); the UFO starts at `ufo0`.
    fn rig(fence: u32, ufo0: u32, done_at_us: Option<u64>) -> Rig {
        let mut mem = vec![0u32; CARVEOUT_WORDS];
        mem[FUFO as usize / 4] = ufo0;
        let cpu = mem.as_mut_ptr() as *mut u8;
        let sim = Box::new(Sim {
            now_us: Cell::new(0),
            done_at_us,
            geom_at_us: Cell::new(None),
            fence,
            ufo: cpu.wrapping_add(FUFO as usize) as *mut u32,
            gufo: cpu.wrapping_add(GUFO as usize) as *mut u32,
            steps: RefCell::new(Vec::new()),
            lines: RefCell::new(Vec::new()),
            complete_on_log: Cell::new(false),
        });
        let ops = Box::new(YdPvrKmOps {
            ctx: &*sim as *const Sim as *mut c_void,
            flush: nop_range,
            invd: nop_range,
            fence: nop,
            busy_wait_us: busy_wait,
            log,
        });
        // SAFETY: both are repr(C) structs of integers, raw pointers and
        // arrays of them; all-zero is a valid value of each.
        let mut k: Box<YdPvrKm> = Box::new(unsafe { core::mem::zeroed() });
        let mut r: Box<YdPvrRender> = Box::new(unsafe { core::mem::zeroed() });
        k.cpu = cpu;
        k.size = (CARVEOUT_WORDS * 4) as u64;
        k.ops = &*ops;
        k.fw_main_size = (CARVEOUT_WORDS * 4) as u32;
        k.fw_config_off = u32::MAX;
        k.fwccb_ctl_off = FWCCB_CTL;
        k.fwccb_cmds_off = FWCCB_CTL + 0x40;
        k.hwrinfobuf_off = HWRINFO;
        r.inited = 1;
        r.fufo_off = FUFO;
        r.gufo_off = GUFO;
        super::kick_times::test_clock::set(T0);
        // The first stall's register dump reads MMIO, which a host test
        // has not got: mark it done, so a timeout only logs.
        r.stall_dumped = 1;
        Rig { mem, sim, _ops: ops, k, r }
    }

    impl Rig {
        fn wait(&mut self, timeout_ms: u32) -> i32 {
            let fence = self.sim.fence;
            wait_for(&mut self.k, &mut self.r, fence, timeout_ms, "test")
        }
        fn waited_us(&self) -> u64 {
            self.sim.now_us.get()
        }
        fn ufo(&self) -> u32 {
            self.mem[FUFO as usize / 4]
        }
        fn log(&self) -> String {
            self.sim.lines.borrow().join("\n")
        }
    }

    #[test]
    fn a_wait_ends_on_the_first_poll_step_after_the_fence_moved() {
        // The GPU finishes 1,130 us into the wait.
        let mut t = rig(7, 6, Some(1130));
        assert_eq!(t.wait(100), 0, "log:\n{}", t.log());
        assert_eq!(t.ufo(), 7);
        // Every step is WAIT_POLL_US, and the wait stops on the first step
        // that ends at or after the completion.
        assert!(t.sim.steps.borrow().iter().all(|&s| s == WAIT_POLL_US), "steps {:?}", t.sim.steps.borrow());
        assert_eq!(t.waited_us(), 1130u64.div_ceil(u64::from(WAIT_POLL_US)) * u64::from(WAIT_POLL_US));
        // Each arm's number, so the two arms are told apart: the 1 ms read
        // rounds 1,130 us up to 2,000 (glmark2's 2,016-2,017 us swaps on
        // the AM62P, 2026-10-03), the 20 us read to 1,140.
        #[cfg(feature = "pvr-fine-wait")]
        assert_eq!(t.waited_us(), 1140);
        #[cfg(not(feature = "pvr-fine-wait"))]
        assert_eq!(t.waited_us(), 2000);
        assert!(t.log().is_empty(), "a clean wait logs nothing: {}", t.log());
    }

    #[test]
    fn a_fence_already_written_returns_without_waiting() {
        let mut t = rig(7, 9, None);
        assert_eq!(t.wait(100), 0);
        assert_eq!(t.waited_us(), 0);
        assert!(t.sim.steps.borrow().is_empty());
    }

    #[test]
    fn a_fence_that_never_moves_times_out_after_exactly_the_timeout() {
        let mut t = rig(7, 6, None);
        assert_eq!(t.wait(3), -errno::ETIMEDOUT);
        // The bound is the same with either step: the timeout in whole ms.
        assert_eq!(t.waited_us(), 3000);
        assert_eq!(t.sim.steps.borrow().len(), 3000 / WAIT_POLL_US as usize);
        let log = t.log();
        assert!(log.contains("render: fence 7 timeout (ufo=6) waiting on the test"), "log:\n{log}");
    }

    #[test]
    fn a_wait_before_init_is_refused_without_waiting() {
        let mut t = rig(7, 6, Some(10));
        t.r.inited = 0;
        assert_eq!(t.wait(100), -errno::EINVAL);
        assert_eq!(t.waited_us(), 0);
    }

    /// Put one UPDATE_STATS notice (element `e`) in the FWCCB: slot 0,
    /// written, not yet read.
    fn post_stats(t: &mut Rig, e: u32) {
        let w = |t: &mut Rig, off: u32, v: u32| t.mem[off as usize / 4] = v;
        let cmd = FWCCB_CTL + 0x40;
        w(t, cmd + FWCCB_CMD_TYPE as u32, FWCCB_CMD_UPDATE_STATS);
        w(t, cmd + FWCCB_STATS_ELEMENT as u32, e);
        w(t, cmd + FWCCB_STATS_ADJUST as u32, 1);
        w(t, FWCCB_CTL + CCB_CTL_WRITE_OFFSET as u32, 1);
        w(t, FWCCB_CTL + CCB_CTL_WRAP_MASK as u32, 7);
    }

    /// Freelist 1 at its maximum (the probe's 512 of 512), growing by 256.
    fn list_at_maximum(t: &mut Rig) {
        let f = &mut t.k.fls[0];
        (f.id, f.npages, f.maxpages, f.grow) = (1, 512, 512, 256);
        t.k.nfls = 1;
    }

    /// The probe's case (0efa0642): an out-of-memory notice, no grow
    /// request, the list at its maximum. With the knob the wait returns at
    /// once, by name, instead of spinning to its timeout (the probe's kick
    /// waited 20 s, twice, and the watchdog ended the run).
    #[test]
    fn the_probes_case_ends_the_wait_at_once_with_the_knob() {
        let mut t = rig(7, 6, None);
        list_at_maximum(&mut t);
        post_stats(&mut t, FWCCB_STATS_OUT_OF_MEMORY);
        t.r.cfg.fl_grow_on_demand = 1;
        assert_eq!(t.wait(20000), -errno::ESHUTDOWN, "log:\n{}", t.log());
        assert_eq!(t.waited_us(), 0, "not waited for");
        assert_eq!((t.k.stop_cause, t.k.stop_fl, t.k.stop_fence), (super::super::freelist::STOP_AT_MAXIMUM, 1, 7));
        assert_eq!(
            t.log(),
            "FWCCB: UPDATE_STATS element=2 (out of memory) adjust=1 (seen 1)\n\
             render: stopped: the parameter manager out of memory with its freelist at the maximum (list 1, 0 KiB); \
             no kick is waited for or taken from now on\n\
             render: fence 7 (test) cannot complete: the render service stopped (the parameter manager out of \
             memory with its freelist at the maximum); not waited for"
        );
        // Every later wait returns at once, and says nothing more.
        assert_eq!(t.wait(20000), -errno::ESHUTDOWN);
        assert_eq!(t.sim.lines.borrow().len(), 3);
    }

    /// The same FWCCB without the knob: the stop is the same. The knob-off
    /// default used to wait out its timeout here, and the watchdog ended the
    /// run with no record.
    #[test]
    fn without_the_knob_the_probes_case_ends_the_wait_too() {
        let mut t = rig(7, 6, None);
        list_at_maximum(&mut t);
        post_stats(&mut t, FWCCB_STATS_OUT_OF_MEMORY);
        assert_eq!(t.wait(20000), -errno::ESHUTDOWN, "log:\n{}", t.log());
        assert_eq!(t.waited_us(), 0, "not waited for");
        assert_eq!((t.k.stop_cause, t.k.stop_fl, t.k.fw_stats[2]), (super::super::freelist::STOP_AT_MAXIMUM, 1, 1));
    }

    /// With the knob, a list below its maximum: the notice is a grow's (or
    /// unexplained), not a stop.
    #[test]
    fn an_out_of_memory_notice_with_room_left_is_no_stop() {
        let mut t = rig(7, 6, Some(1000));
        list_at_maximum(&mut t);
        t.k.fls[0].maxpages = 4096;
        post_stats(&mut t, FWCCB_STATS_OUT_OF_MEMORY);
        t.r.cfg.fl_grow_on_demand = 1;
        assert_eq!(t.wait(100), 0, "log:\n{}", t.log());
        assert_eq!(t.k.stop_cause, 0);
    }

    /// The line at a grow request: kicks 7..=10 in flight (fragment done at
    /// 6), geometry done for 7 and 8 (their UFO slots hold 7 and 8), slot 9
    /// still holding an older sequence and slot 10 nothing.
    #[test]
    fn the_grow_context_counts_the_kicks_that_hold_pm_pages() {
        let mut t = rig(10, 6, None);
        const GUFO: u32 = 0x300;
        t.r.gufo_off = GUFO;
        t.r.seq = 10;
        t.k.fl_grow_reqs = 3;
        for (s, v) in [(7u32, 7u32), (8, 8), (9, 8u32.wrapping_sub(16)), (10, 0)] {
            t.mem[(GUFO / 4 + geom_ufo_slot(s)) as usize] = v;
        }
        InFlight::take(&t.k, t.r.seq, t.r.fufo_off, GUFO, t.r.grow_scene).say(&t.k);
        assert_eq!(
            t.log(),
            "render: at grow request 3 (sampled when the batch was first read): last submitted 10, last fragment done 6; 4 kicks in flight: 2 with TA done and 3D pending, 2 with TA pending"
        );
    }

    /// With `pvr-grow-scene`: the application marked its scene when kick 4
    /// was the last submitted, 1.5 s before the request; the line then says
    /// scene 2, 6 kicks since its start (the first 5), 1500 ms. Without a
    /// mark (the test above) the line is the old one.
    #[cfg(feature = "pvr-grow-scene")]
    #[test]
    fn the_grow_line_says_the_scenes_kicks_and_time() {
        let mut t = rig(10, 6, None);
        const GUFO: u32 = 0x300;
        t.r.gufo_off = GUFO;
        t.k.fl_grow_reqs = 3;
        for (s, v) in [(7u32, 7u32), (8, 8), (9, 8u32.wrapping_sub(16)), (10, 0)] {
            t.mem[(GUFO / 4 + geom_ufo_slot(s)) as usize] = v;
        }
        super::kick_times::test_clock::set(1_000_000);
        t.r.seq = 2;
        scene_begin(&mut t.r);
        t.r.seq = 4;
        scene_begin(&mut t.r);
        super::kick_times::test_clock::set(2_500_000);
        t.r.seq = 10;
        InFlight::take(&t.k, t.r.seq, t.r.fufo_off, GUFO, t.r.grow_scene).say(&t.k);
        assert_eq!(
            t.log(),
            "render: at grow request 3 (sampled when the batch was first read): last submitted 10, last fragment done 6; 4 kicks in flight: 2 with TA done and 3D pending, 2 with TA pending; scene 2: 6 kicks since its start (the first 5), 1500 ms"
        );
    }

    /// The sample is taken when the batch is first read, before the grow is
    /// served and said. In the rig, the kick completes as soon as a line is
    /// said (as the AM62P's console takes tens of ms per batch). Sampled
    /// after the batch, as 157b76e3 did, the line read "0 kicks in flight"
    /// at every request of the board run (fl-kt-glmark2-on-1).
    #[test]
    fn the_grow_line_is_sampled_before_the_batch_is_served() {
        let mut t = rig(10, 6, None);
        const GUFO: u32 = 0x300;
        const KCTL: u32 = 0x400;
        (t.r.gufo_off, t.r.seq) = (GUFO, 10);
        for (s, v) in [(7u32, 7u32), (8, 8)] {
            t.mem[(GUFO / 4 + geom_ufo_slot(s)) as usize] = v;
        }
        // a FREELIST_GROW for list 1 in FWCCB slot 0
        let w = |t: &mut Rig, off: u32, v: u32| t.mem[off as usize / 4] = v;
        let cmd = FWCCB_CTL + 0x40;
        w(&mut t, cmd + FWCCB_CMD_TYPE as u32, FWCCB_CMD_FREELIST_GROW);
        w(&mut t, cmd + FWCCB_FL_GS_ID as u32, 1);
        w(&mut t, FWCCB_CTL + CCB_CTL_WRITE_OFFSET as u32, 1);
        w(&mut t, FWCCB_CTL + CCB_CTL_WRAP_MASK as u32, 7);
        // the KCCB the answer is posted to (one slot), and a register file
        // for the doorbell
        (t.k.kccb_ctl_off, t.k.kccb_cmds_off, t.k.kccb_rtn_off) = (KCTL, KCTL + 0x40, KCTL + 0x200);
        let mut regs = vec![0u32; 0x1000];
        t.k.regs = regs.as_mut_ptr() as usize;
        // list 1: 512 pages listed, its own reserve serves the grow
        let mut arr = vec![0u32; 4096];
        let f = &mut t.k.fls[0];
        (f.id, f.npages, f.maxpages, f.grow, f.backed, f.reserve) = (1, 512, 4096, 256, 512, 1024);
        f.arr = arr.as_mut_ptr();
        f.heap_off = 0x800;
        t.k.nfls = 1;
        t.sim.complete_on_log.set(true);
        fwccb_service_r(&mut t.k, &mut t.r);
        let line = t.sim.lines.borrow().iter().find(|l| l.starts_with("render: at grow request")).cloned();
        assert_eq!(
            line.as_deref(),
            Some("render: at grow request 1 (sampled when the batch was first read): last submitted 10, last fragment done 6; 4 kicks in flight: 2 with TA done and 3D pending, 2 with TA pending"),
            "log:\n{}",
            t.log()
        );
        assert_eq!(t.ufo(), 10, "the kick did complete while the batch was said");
    }

    /// A FWCCB batch of `grows` FREELIST_GROW requests for list 1 (its own
    /// reserve serves them all), with a register file and the last kick's
    /// HWRT data holding known words. Returns the log.
    #[cfg(feature = "pvr-grow-scene")]
    fn runaway_rig(grows: u32) -> String {
        let mut t = rig(10, 6, None);
        const GUFO: u32 = 0x300;
        const KCTL: u32 = 0x400;
        const HRT: u32 = 0x1400;
        (t.r.gufo_off, t.r.seq) = (GUFO, 10);
        let w = |t: &mut Rig, off: u32, v: u32| t.mem[off as usize / 4] = v;
        for i in 0..grows {
            let cmd = FWCCB_CTL + 0x40 + i * FWCCB_CMD_SIZE as u32;
            w(&mut t, cmd + FWCCB_CMD_TYPE as u32, FWCCB_CMD_FREELIST_GROW);
            w(&mut t, cmd + FWCCB_FL_GS_ID as u32, 1);
        }
        w(&mut t, FWCCB_CTL + CCB_CTL_WRITE_OFFSET as u32, grows);
        w(&mut t, FWCCB_CTL + CCB_CTL_WRAP_MASK as u32, 7);
        (t.k.kccb_ctl_off, t.k.kccb_cmds_off, t.k.kccb_rtn_off) = (KCTL, KCTL + 0x40, KCTL + 0x200);
        let mut regs = vec![0u32; 0x2000];
        for (off, v) in [(0x6020u32, 0x85f303u32), (0x6008, 0x11), (0x5028, 0xa1e7), (0x5030, 0xb9c5), (0x0430, 1)] {
            regs[off as usize / 4] = v;
        }
        regs[0x0270 / 4] = 0x1234;
        t.k.regs = regs.as_mut_ptr() as usize;
        // the last kick's HWRT data: state 3, flags 0x9, ALIST 0x40080, MList 0x12
        (t.r.last_hs, t.r.hwrt[1].hrt_off, t.r.dbg_vdm) = (1, HRT, 0x8000_f2c0);
        w(&mut t, HRT + HWRTDATA_STATE as u32, 3);
        w(&mut t, HRT + HWRTDATA_FLAGS as u32, 9);
        w(&mut t, HRT + HWRTDATA_PM_ALIST_STACK_POINTER as u32, 0x40080);
        w(&mut t, HRT + HWRTDATA_PM_MLIST_STACK_POINTER as u32, 0x12);
        let mut arr = vec![0u32; 4096];
        let f = &mut t.k.fls[0];
        (f.id, f.npages, f.maxpages, f.grow, f.backed, f.reserve) = (1, 512, 4096, 256, 512, 1024);
        f.arr = arr.as_mut_ptr();
        f.heap_off = 0x800;
        t.k.nfls = 1;
        fwccb_service_r(&mut t.k, &mut t.r);
        drop(regs);
        t.log()
    }

    /// Two grows served in one batch: the runaway line, with the rig's
    /// registers and HWRT words. Today's code prints no such line.
    #[cfg(feature = "pvr-grow-scene")]
    #[test]
    fn a_batch_of_two_served_grows_dumps_the_runaway_line() {
        let log = runaway_rig(2);
        let line = log.lines().find(|l| l.starts_with("render: runaway batch"));
        assert_eq!(
            line,
            Some("render: runaway batch: 2 grows served in one FWCCB batch; TA_CYCLE=0x85f303 TA_PHASE=0x11 TE_SIG=0xa1e7 VCE=0xb9c5 VDM_CTX_STORE=0x1; PM_CAT VCE0=0x0 TE0=0x0 ALIST0=0x0 PM 0x190=0x0 0x288=0x0 stack 0x270=0x1234 0x280=0x0 0x2c0=0x0; HWRT set 1 (last kick 10): state 3 flags 0x9 alist_sp 0x40080 mlist_sp 0x12; vdm 0x8000f2c0"),
            "log:\n{log}"
        );
    }

    /// One grow in its batch (every clean run's shape): no runaway line.
    #[cfg(feature = "pvr-grow-scene")]
    #[test]
    fn a_batch_of_one_grow_dumps_nothing() {
        let log = runaway_rig(1);
        assert!(log.contains("FREELIST_GROW id=1 -> +256 pages"), "log:\n{log}");
        assert!(!log.contains("runaway batch"), "log:\n{log}");
    }

    /// With the knob, a fence that never moves stops the render service at
    /// STOP_WAIT_MS (10 s), half the board's 20 s alive window, so the run
    /// ends with a GPU_STOP record. fwccb-2's wait ran its full 20 s and
    /// the watchdog fired in the same second, with no record.
    #[test]
    fn with_the_knob_a_still_fence_stops_before_the_watchdog() {
        let mut t = rig(7, 6, None);
        t.r.cfg.fl_grow_on_demand = 1;
        assert_eq!(t.wait(20000), -errno::ESHUTDOWN, "log:\n{}", t.log());
        assert_eq!(t.waited_us(), u64::from(STOP_WAIT_MS) * 1000);
        assert_eq!(t.k.stop_cause, super::super::freelist::STOP_KICK_TIMEOUT);
        assert!(
            t.log().contains("render: stopped: a kick that did not complete in the stop's time (list 0, 0 KiB)"),
            "log:\n{}",
            t.log()
        );
    }

    /// Without the knob the stop is the same: at STOP_WAIT_MS, cause 4. The
    /// knob-off default used to wait its full 20 s, in the same second as the
    /// board's alive window, and the watchdog ended the AM62P's run with no
    /// record (the common set's round 2, 52be99c3).
    #[test]
    fn without_the_knob_a_still_fence_stops_before_the_watchdog_too() {
        let mut t = rig(7, 6, None);
        assert_eq!(t.r.cfg.fl_grow_on_demand, 0);
        assert_eq!(t.wait(20000), -errno::ESHUTDOWN, "log:\n{}", t.log());
        assert_eq!(t.waited_us(), u64::from(STOP_WAIT_MS) * 1000);
        assert_eq!(t.k.stop_cause, super::super::freelist::STOP_KICK_TIMEOUT);
    }

    /// The reproducer's counters, word by word from their fields.
    #[test]
    fn the_fl_counters_are_the_fields_in_order() {
        let mut t = rig(7, 6, None);
        (t.k.fl_grow_reqs, t.k.fl_grows_done, t.k.fl_no_grow) = (9, 3, 6);
        t.k.fw_stats[FWCCB_STATS_OUT_OF_MEMORY as usize] = 11;
        (t.k.fwccb_empty, t.k.fwccb_filled, t.r.reposts, t.k.stop_cause) = (2, 1, 8, 4);
        t.k.fw_stats[FWCCB_STATS_PARTIAL_RENDERS as usize] = 5;
        assert_eq!(fl_counters(&t.k, &t.r), [9, 3, 6, 11, 2, 1, 8, 4, 5]);
        let mut out = [0u32; FL_COUNTERS];
        // SAFETY: the rig's live state; `out` has FL_COUNTERS words.
        unsafe { yd_pvr_render_fl_counters(&*t.k, &*t.r, out.as_mut_ptr()) };
        assert_eq!(out, [9, 3, 6, 11, 2, 1, 8, 4, 5]);
    }

    /// A wait shorter than the stop's time is unchanged: its own timeout.
    #[test]
    fn a_wait_shorter_than_the_stop_times_out_as_before() {
        let mut t = rig(7, 6, None);
        assert_eq!(t.wait(5000), -errno::ETIMEDOUT);
        assert_eq!(t.waited_us(), 5_000_000);
        assert_eq!(t.k.stop_cause, 0);
    }

    /// The C side's YD_PVR_STOP_WAIT_MS (yakogl_drivers.h, which
    /// pvr_render.c's BUILD_ASSERT checks against the alive window) is
    /// STOP_WAIT_MS.
    #[test]
    fn the_headers_stop_wait_is_the_rust_one() {
        let h = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/include/yakogl_drivers.h")).unwrap();
        let v: u32 = h
            .lines()
            .find_map(|l| l.strip_prefix("#define YD_PVR_STOP_WAIT_MS "))
            .and_then(|v| v.trim().parse().ok())
            .expect("YD_PVR_STOP_WAIT_MS in yakogl_drivers.h");
        assert_eq!(v, STOP_WAIT_MS);
    }

    /// A stopped render service takes no kick: the submit returns before it
    /// builds or posts anything.
    #[test]
    fn a_stopped_render_service_takes_no_kick() {
        let mut t = rig(7, 6, None);
        t.k.stop_cause = super::super::freelist::STOP_AT_MAXIMUM;
        // SAFETY: all-zero is a valid kick for a submit that returns first.
        let kick: PvrRenderKick = unsafe { core::mem::zeroed() };
        let mut fence = 0u32;
        assert_eq!(submit(&mut t.k, &mut t.r, &kick, &mut fence), -errno::ESHUTDOWN);
        assert_eq!((t.r.seq, fence), (0, 0));
    }

    // ---- kick times (feature pvr-kick-times) ----------------------------

    use super::kick_times::{self as kt, YdPvrKickUs};
    #[cfg(feature = "pvr-kick-times")]
    use super::kick_times::{KICK_US_GEOM_SEEN, KICK_US_PREV, KICK_US_WAITED};

    fn last(t: &Rig) -> (i32, YdPvrKickUs) {
        let mut o = YdPvrKickUs::default();
        let rc = kt::last_kick_us(&t.r, &mut o);
        (rc, o)
    }

    /// Round `us` up to the poll step: when a wait's read first sees an event.
    #[cfg(feature = "pvr-kick-times")]
    fn seen_at(us: u64) -> u32 {
        us.div_ceil(u64::from(WAIT_POLL_US)) as u32 * WAIT_POLL_US
    }

    #[cfg(not(feature = "pvr-kick-times"))]
    #[test]
    fn without_the_feature_the_reader_answers_not_supported() {
        let mut t = rig(7, 6, Some(1130));
        kt::posted(&t.k, &mut t.r, 7, T0, 0, false);
        assert_eq!(t.wait(100), 0);
        assert_eq!(last(&t).0, -errno::ENOTSUP);
    }

    #[cfg(feature = "pvr-kick-times")]
    #[test]
    fn before_any_kick_completed_the_reader_answers_no_data() {
        let mut t = rig(7, 6, None);
        assert_eq!(last(&t).0, -errno::ENOTSUP, "nothing posted yet: the clock is not started");
        kt::posted(&t.k, &mut t.r, 7, T0, 0, false);
        assert_eq!(last(&t).0, -errno::ENODATA);
    }

    #[cfg(feature = "pvr-kick-times")]
    #[test]
    fn a_waited_kick_is_split_into_geometry_and_fragment_with_error_bars() {
        // Posted at T0 after 100 us in submit (40 of them in the MMU
        // invalidate); the geometry fence lands 400 us in, the fragment fence
        // 1,130 us in, and a wait starts right after the post.
        let mut t = rig(7, 6, Some(1130));
        t.sim.geom_at_us.set(Some(400));
        kt::posted(&t.k, &mut t.r, 7, T0 - 100, 40, true);
        assert_eq!(t.wait(100), 0, "log:\n{}", t.log());
        let (rc, o) = last(&t);
        assert_eq!(rc, 0);
        assert_eq!(o.seq, 7);
        assert_eq!((o.submit_us, o.mmu_us, o.mmu_synced), (100, 40, 1));
        assert_eq!(o.flags, KICK_US_GEOM_SEEN | KICK_US_WAITED, "no earlier kick: no fence-to-fence");
        // Each event is seen by the first read after it, and the error bar
        // is one poll step (the read before it missed).
        assert_eq!(o.kick_geom_us, seen_at(400));
        assert_eq!(o.geom_err_us, WAIT_POLL_US);
        assert_eq!(o.kick_frag_us, seen_at(1130));
        assert_eq!(o.frag_err_us, WAIT_POLL_US);
        assert_eq!(o.wait_us, o.kick_frag_us, "the wait began at the post");
        assert_eq!((o.done, o.lost), (1, 0));
    }

    #[cfg(feature = "pvr-kick-times")]
    #[test]
    fn the_next_kick_reports_fence_to_fence_time() {
        let mut t = rig(7, 6, Some(1130));
        kt::posted(&t.k, &mut t.r, 7, T0, 0, false);
        assert_eq!(t.wait(100), 0);
        let first = last(&t).1.kick_frag_us;
        // Kick 8, posted now; its fence lands 300 us later.
        let posted_at = t.waited_us();
        kt::posted(&t.k, &mut t.r, 8, T0 + posted_at, 0, false);
        let mut t2 = t;
        // The same simulated GPU, now finishing kick 8 (the box stays where
        // k.ops' context points).
        *t2.sim = Sim { done_at_us: Some(posted_at + 300), fence: 8, ..*t2.sim };
        assert_eq!(t2.wait(100), 0, "log:\n{}", t2.log());
        let (rc, o) = last(&t2);
        assert_eq!(rc, 0);
        assert_eq!(o.seq, 8);
        assert_ne!(o.flags & KICK_US_PREV, 0);
        assert_eq!(o.kick_frag_us, seen_at(300));
        assert_eq!(o.frag_frag_us, (posted_at as u32 - first) + o.kick_frag_us);
        assert_eq!(o.flags & KICK_US_GEOM_SEEN, 0, "kick 8's geometry fence was never written");
        assert_eq!(o.done, 2);
    }

    #[cfg(feature = "pvr-kick-times")]
    #[test]
    fn a_kick_nobody_read_while_it_moved_has_the_whole_interval_as_its_error() {
        let mut t = rig(7, 6, None);
        kt::posted(&t.k, &mut t.r, 7, T0, 0, false);
        // The fence lands unobserved; the next read is 5 ms later.
        t.mem[FUFO as usize / 4] = 7;
        kt::test_clock::set(T0 + 5000);
        kt::poll(&t.k, &mut t.r);
        let (rc, o) = last(&t);
        assert_eq!(rc, 0);
        assert_eq!((o.kick_frag_us, o.frag_err_us), (5000, 5000));
        assert_eq!(o.flags & KICK_US_WAITED, 0);
    }

    #[cfg(feature = "pvr-kick-times")]
    #[test]
    fn a_ring_slot_reused_before_its_kick_completed_counts_as_lost() {
        let mut t = rig(7, 0, None);
        for seq in 1..=(kt::KT_RING as u32 + 1) {
            kt::posted(&t.k, &mut t.r, seq, T0, 0, false);
        }
        assert_eq!(t.r.kick_times.lost, 1);
        // Completing the newest completes every older one still tracked, in
        // order, and the last one is the newest.
        t.mem[FUFO as usize / 4] = kt::KT_RING as u32 + 1;
        kt::test_clock::set(T0 + 10);
        kt::poll(&t.k, &mut t.r);
        let (_, o) = last(&t);
        assert_eq!(o.seq, kt::KT_RING as u32 + 1);
        assert_eq!((o.done, o.lost), (kt::KT_RING as u32, 1));
    }
}

#[cfg(test)]
mod submit_tests {
    //! The client CCB bytes of [`submit`], knob off and on
    //! (docs/notes/pvr-render-pool.md, "Design: the firmware's partial
    //! render", host tests): with the knob, the fragment CCB of one kick
    //! holds, in this order, FENCE_PR {timeline, seq}, 3D_PR (the 3D
    //! command's size and words), UPDATE, then FENCE, 3D, UPDATE, and the
    //! geometry command's PR fence words are {timeline, seq - 1}. Without
    //! the knob, the bytes are today's (the km render record unchanged).
    //! Today's code fails the first test: it is the design's known negative.
    use super::*;
    use crate::gpu::pvr::km::YdPvrKmOps;
    use std::cell::RefCell;

    /// Carveout layout (byte offsets; the wait tests' pattern, all objects
    /// in the firmware heap's main region). The rings hold whole kicks, so
    /// a command never wraps during a test.
    const FUFO: u32 = 0x300;
    const MC_SYNC: u32 = 0x310;
    const GUFO: u32 = 0x400;
    const KCTL: u32 = 0x1000;
    const KCMD: u32 = 0x1040; // 8 slots of KCCB commands
    const KRTN: u32 = 0x1600;
    const HWRINFO: u32 = 0x2000;
    const HC0: u32 = 0x3000; // the seeded pair's HWRT data commons and datas
    const HRT0: u32 = 0x3080;
    const HC1: u32 = 0x3180;
    const HRT1: u32 = 0x3200;
    // The pair's partial-render render-target arrays, pre-seeded so the rig
    // never allocates (MAX_RT dwords each; regions past all of the above).
    const RTA0V: u32 = 0x3400;
    const RTA0P: u32 = 0x3410;
    const RTA1V: u32 = 0x3500;
    const RTA1P: u32 = 0x3510;
    // A second pair of the same kind, for a size that takes two
    // (`a_size_with_every_member_in_flight_takes_a_second_pair`).
    const HC2: u32 = 0x3600;
    const HRT2: u32 = 0x3680;
    const HC3: u32 = 0x3780;
    const HRT3: u32 = 0x3800;
    const RTA2V: u32 = 0x3a00;
    const RTA2P: u32 = 0x3a10;
    const RTA3V: u32 = 0x3b00;
    const RTA3P: u32 = 0x3b10;
    const GCCCB: u32 = 0x8000;
    const FCCCB: u32 = 0x10000;
    const CARVEOUT_BYTES: usize = 0x40000;
    const KICK_W: u32 = 8;
    const KICK_H: u32 = 8;

    struct Sim {
        lines: RefCell<Vec<String>>,
    }

    unsafe extern "C" fn nop_range(_: *mut c_void, _: *const c_void, _: usize) {}
    unsafe extern "C" fn nop(_: *mut c_void) {}
    unsafe extern "C" fn busy_wait(_: *mut c_void, _: u32) {}
    unsafe extern "C" fn log(ctx: *mut c_void, _: u32, b: *const u8, n: usize) {
        // SAFETY: ctx is the test's Sim; b is valid for n bytes.
        let s = unsafe { &*(ctx as *const Sim) };
        s.lines.borrow_mut().push(String::from_utf8_lossy(unsafe { core::slice::from_raw_parts(b, n) }).into_owned());
    }

    struct Rig {
        mem: Vec<u32>,
        /// The KCCB doorbell registers `k.regs` points at (alive while k is).
        _regs: Vec<u32>,
        sim: Box<Sim>,
        /// Kept alive for `k.ops`.
        _ops: Box<YdPvrKmOps>,
        k: Box<YdPvrKm>,
        r: Box<YdPvrRender>,
    }

    /// A rig with the two rings, the geometry UFO slots and timeline word,
    /// the KCCB, and one 8x8 render-target pair already "built" - enough of
    /// a context for [`submit`] to write a kick, with nothing in flight
    /// (no waits, no re-post, no grow). `partial_render` is the knob.
    fn rig(partial_render: u32) -> Rig {
        let mut mem = vec![0u32; CARVEOUT_BYTES / 4];
        let cpu = mem.as_mut_ptr() as *mut u8;
        let sim = Box::new(Sim { lines: RefCell::new(Vec::new()) });
        let ops = Box::new(YdPvrKmOps {
            ctx: &*sim as *const Sim as *mut c_void,
            flush: nop_range,
            invd: nop_range,
            fence: nop,
            busy_wait_us: busy_wait,
            log,
        });
        // SAFETY: both are repr(C) structs of integers, raw pointers and
        // arrays of them; all-zero is a valid value of each.
        let mut k: Box<YdPvrKm> = Box::new(unsafe { core::mem::zeroed() });
        let mut r: Box<YdPvrRender> = Box::new(unsafe { core::mem::zeroed() });
        let mut regs = vec![0u32; 0x4000];
        k.cpu = cpu;
        k.size = CARVEOUT_BYTES as u64;
        k.ops = &*ops;
        k.fw_main_size = CARVEOUT_BYTES as u32;
        k.fw_config_off = u32::MAX;
        k.fwccb_ctl_off = 0x100;
        k.fwccb_cmds_off = 0x140;
        k.kccb_ctl_off = KCTL;
        k.kccb_cmds_off = KCMD;
        k.kccb_rtn_off = KRTN;
        k.hwrinfobuf_off = HWRINFO;
        k.regs = regs.as_mut_ptr() as usize;
        // The KCCB's wrap mask, eight slots.
        mem[KCTL as usize / 4 + CCB_CTL_WRAP_MASK / 4] = 7;

        r.inited = 1;
        r.cfg.partial_render = partial_render;
        r.gcccb_off = GCCCB;
        r.fcccb_off = FCCCB;
        r.gufo_off = GUFO;
        r.fufo_off = FUFO;
        r.mc_sync_off = MC_SYNC;
        // Two pairs, every member free but built: buffers "allocated" (the
        // rig only needs non-null block handles), HWRT data and RTA arrays
        // at the seeded offsets, nobody in flight (last_seq 0). Kick 1 then
        // builds member 0, kick 2 the pair's second member: every handout
        // walks the build path, which rewrites the HWRT data words at the
        // seeded offsets without allocating. The second pair is taken only
        // by a size whose first pair is all in flight.
        for (m, (hc, hrt, rtav, rtap)) in [
            (0usize, (HC0, HRT0, RTA0V, RTA0P)),
            (1, (HC1, HRT1, RTA1V, RTA1P)),
            (2, (HC2, HRT2, RTA2V, RTA2P)),
            (3, (HC3, HRT3, RTA3V, RTA3P)),
        ] {
            let s = &mut r.hwrt[m];
            (s.used, s.w, s.h, s.samples, s.fw_built) = (0, 0, 0, 0, 1);
            (s.hcmn_off, s.hrt_off) = (hc, hrt);
            (s.rta_valid_off, s.rta_partial_off) = (rtav, rtap);
            s.hrt_fwaddr = k.heap_fwaddr(hrt);
            let bo = |base: u32| PvrGpuBo {
                gpu: GENERAL_HEAP_BASE + u64::from(base),
                cpu: cpu.wrapping_add(base as usize),
                heap_off: base,
            };
            s.tpc = bo(0x20000);
            s.vheap = bo(0x24000);
            s.rgnhdr = bo(0x28000);
            s.macrotile = bo(0x2c000);
            s.mlist = bo(0x30000);
            s.tpc_alloc = 0x4000;
            s.rgn_alloc = 0x10000;
        }
        super::kick_times::test_clock::set(1_000_000);
        Rig { mem, _regs: regs, sim, _ops: ops, k, r }
    }

    /// An 8x8 kick with distinguishable non-zero registers (tag picks the
    /// values, so a command of kick 1 and of kick 2 differ word by word).
    /// Control-register values use only high bits: submit reads meaning out
    /// of the low bits of ISP CTL, PPP CTRL or the command flags (z-buffer
    /// bits, GETVISRESULTS), and a test value must not claim any of them.
    fn kick(tag: u32) -> PvrRenderKick {
        // SAFETY: every field is a plain integer or an array of them.
        let mut c: PvrRenderKick = unsafe { core::mem::zeroed() };
        let o = |x: u32| x * 0x1000 + tag; // heap offsets (u32)
        let v = |x: u32| u64::from(o(x)); // ... as GPU VAs (u64)
        let r = |x: u32| x * 0x10000 + tag; // control-register values (u32)
        (c.width, c.height, c.samples) = (KICK_W, KICK_H, 1);
        c.geom.vdm_ctrl_stream_base = GENERAL_HEAP_BASE + v(0xF0);
        c.geom.ppp_ctrl = r(0x11);
        c.geom.te_psg = r(0x22);
        c.frag.usc_pixel_output_ctrl = r(0x33);
        c.frag.isp_ctl = r(0x44);
        c.frag.event_pixel_pds_data = o(0x50);
        c.frag.pbe_word[0] = [v(0x60), v(0x70)];
        c.frag.num_rt = 1;
        c.frag.pds_bgnd[0] = v(0x80);
        c.frag.isp_scissor_base = v(0x90);
        c.frag.cmd_flags = r(0x99);
        c
    }

    impl Rig {
        /// The commands written into a client CCB so far, as (type, size,
        /// body dwords). The rings never wrapped during a test (the reserve
        /// checks hold in the asserts below).
        fn cmds(&self, ring: u32, woff: u32) -> Vec<(u32, u32, Vec<u32>)> {
            let mut out = Vec::new();
            let mut o = 0u32;
            while o < woff {
                let b = (ring + o) as usize / 4;
                let (ty, size) = (self.mem[b], self.mem[b + 1]);
                out.push((ty, size, self.mem[b + 4..b + 4 + (size / 4) as usize].to_vec()));
                o += 16 + size;
            }
            out
        }
        fn types(&self, ring: u32, woff: u32) -> Vec<u32> {
            self.cmds(ring, woff).into_iter().map(|(ty, _, _)| ty).collect()
        }
        /// The fence's {address, value} of an UPDATE / FENCE / FENCE_PR body.
        fn ufo(body: &[u32]) -> (u32, u32) {
            (body[0], body[1])
        }
        fn log(&self) -> String {
            self.sim.lines.borrow().join("\n")
        }
        /// A carveout dword at a byte offset.
        fn word(&self, at: usize) -> u32 {
            self.mem[at / 4]
        }
        /// The carveout dwords at a byte offset.
        fn words(&self, at: usize, n: usize) -> Vec<u32> {
            self.mem[at / 4..at / 4 + n].to_vec()
        }
    }

    /// The bytes one kick writes into a ring: 16-byte header per command
    /// plus body, [knob off, knob on].
    const GEOM_BYTES: [u32; 2] = [16 + 64 + 16 + 16, 16 + 64 + 16 + 16 + 16 + 16];
    const FRAG_BYTES: [u32; 2] = [16 + 16 + 16 + 320 + 16 + 16, 2 * (16 + 16 + 16 + 320 + 16 + 16)];

    /// With the knob: the fragment CCB gains FENCE_PR + 3D_PR + UPDATE on
    /// the geometry timeline before today's trio, and the geometry command
    /// carries {timeline, seq - 1} and a second UPDATE to the timeline.
    /// Today's code fails this test (the design's known negative).
    #[test]
    fn with_the_knob_the_pr_trio_comes_first_on_both_ccbs() {
        let mut t = rig(1);
        let gufo_base = t.k.heap_fwaddr(GUFO);
        let timeline = gufo_base + 4 * GEOM_UFO_SLOTS;
        let (mut f1, mut f2) = (0u32, 0u32);
        assert_eq!(submit(&mut t.k, &mut t.r, &kick(1), &mut f1), 0, "log:\n{}", t.log());
        assert_eq!(submit(&mut t.k, &mut t.r, &kick(2), &mut f2), 0, "log:\n{}", t.log());
        assert_eq!((f1, f2), (1, 2));
        assert_eq!((t.r.gwo, t.r.fwo), (2 * GEOM_BYTES[1], 2 * FRAG_BYTES[1]));

        let g = t.cmds(GCCCB, t.r.gwo);
        let f = t.cmds(FCCCB, t.r.fwo);
        assert_eq!(
            g.iter().map(|c| c.0).collect::<Vec<_>>(),
            [CCB_CMD_TYPE_GEOM, CCB_CMD_TYPE_UPDATE, CCB_CMD_TYPE_UPDATE].repeat(2)
        );
        assert_eq!(
            f.iter().map(|c| c.0).collect::<Vec<_>>(),
            [
                CCB_CMD_TYPE_FENCE_PR,
                CCB_CMD_TYPE_3D_PR,
                CCB_CMD_TYPE_UPDATE,
                CCB_CMD_TYPE_FENCE,
                CCB_CMD_TYPE_3D,
                CCB_CMD_TYPE_UPDATE
            ]
            .repeat(2)
        );

        // Kick 2 (the geometry ring's commands 3..=5, the fragment ring's
        // 6..=11). Its geometry PR fence words are {timeline, 2 - 1}.
        let g2 = &g[3];
        assert_eq!((g2.1, g2.2.len()), (64, 16));
        assert_eq!((g2.2[13], g2.2[14]), (timeline, 1));
        assert_eq!(g2.2[1], t.k.heap_fwaddr(HRT1)); // the member this kick took
        assert_eq!(Rig::ufo(&g[4].2), (gufo_base + geom_ufo_slot(2) * 4, 2));
        assert_eq!(Rig::ufo(&g[5].2), (timeline, 2));

        // The fragment PR trio: FENCE_PR waits at the timeline's value seq,
        // the 3D_PR is the 3D command's own bytes (its size 320), and the
        // UPDATE rewrites the timeline with the same seq.
        let fp = &f[6];
        assert_eq!((fp.1, fp.2.len()), (16, 4));
        assert_eq!(Rig::ufo(&fp.2), (timeline, 2));
        assert_eq!((f[7].1, f[7].2.len()), (320, 80));
        assert_eq!(f[7].2, f[10].2, "the 3D_PR body is the 3D command's");
        assert!(f[7].2.iter().any(|&w| w != 0), "the copy carries the kick's words");
        assert_eq!(Rig::ufo(&f[8].2), (timeline, 2));
        // And today's trio after it, unchanged: FENCE on the kick's own
        // geometry slot, 3D, UPDATE on the fragment UFO.
        assert_eq!(Rig::ufo(&f[9].2), (gufo_base + geom_ufo_slot(2) * 4, 2));
        assert_eq!(Rig::ufo(&f[11].2), (t.k.heap_fwaddr(FUFO), 2));
    }

    /// Kicks of one size that have not completed (#42: the levels of a
    /// mipmap chain, one kick each at the chain's one target size). Kicks 1
    /// and 2 build the pair; kick 3 finds both members in flight and takes a
    /// second pair instead of waiting for kick 1, kick 4 builds that pair's
    /// second member; once kick 1 has completed, kick 5 takes its member
    /// again. No wait, nothing handed out busy. With the pair alone (before
    /// HWRT_MEMBERS) kick 3 waited for kick 1, and kick 4 for kick 2.
    #[test]
    fn a_size_with_every_member_in_flight_takes_a_second_pair() {
        let mut t = rig(0);
        let mut f = [0u32; 5];
        for (n, fence) in f.iter_mut().enumerate().take(4) {
            assert_eq!(submit(&mut t.k, &mut t.r, &kick(n as u32 + 1), fence), 0, "log:\n{}", t.log());
        }
        // the fragment UFO: kick 1 done, kicks 2..4 still in flight
        // SAFETY: the UFO word of the rig's carveout, written as the GPU
        // writes it.
        unsafe { (t.k.cpu.wrapping_add(FUFO as usize) as *mut u32).write_volatile(1) };
        assert_eq!(submit(&mut t.k, &mut t.r, &kick(5), &mut f[4]), 0, "log:\n{}", t.log());
        assert_eq!(f, [1, 2, 3, 4, 5]);
        let g = t.cmds(GCCCB, t.r.gwo);
        let members: Vec<u32> = (0..5).map(|n| g[2 * n].2[1]).collect();
        let fa = |o: u32| t.k.heap_fwaddr(o);
        assert_eq!(members, [fa(HRT0), fa(HRT1), fa(HRT2), fa(HRT3), fa(HRT0)], "log:\n{}", t.log());
        assert_eq!(t.r.hwrt_busy_reuse, 0);
        assert!(!t.log().contains("still busy"), "log:\n{}", t.log());
        assert_eq!((t.r.hwrt[0].last_seq, t.r.hwrt[2].last_seq, t.r.hwrt[3].last_seq), (5, 3, 4));
    }

    /// Without the knob: the commands are the ones every board record holds
    /// today - GEOM + one UPDATE per kick, FENCE + 3D + UPDATE per kick -
    /// the old partial-render fence words of the geometry command untouched
    /// (the slot address, value 0), and no FENCE_PR or 3D_PR anywhere.
    #[test]
    fn without_the_knob_the_commands_are_todays() {
        let mut t = rig(0);
        let gufo_base = t.k.heap_fwaddr(GUFO);
        let (mut f1, mut f2) = (0u32, 0u32);
        assert_eq!(submit(&mut t.k, &mut t.r, &kick(1), &mut f1), 0, "log:\n{}", t.log());
        assert_eq!(submit(&mut t.k, &mut t.r, &kick(2), &mut f2), 0, "log:\n{}", t.log());
        assert_eq!((f1, f2), (1, 2));
        assert_eq!((t.r.gwo, t.r.fwo), (2 * GEOM_BYTES[0], 2 * FRAG_BYTES[0]));

        assert_eq!(t.types(GCCCB, t.r.gwo), [CCB_CMD_TYPE_GEOM, CCB_CMD_TYPE_UPDATE].repeat(2));
        assert_eq!(t.types(FCCCB, t.r.fwo), [CCB_CMD_TYPE_FENCE, CCB_CMD_TYPE_3D, CCB_CMD_TYPE_UPDATE].repeat(2));
        let g2 = &t.cmds(GCCCB, t.r.gwo)[2];
        assert_eq!((g2.2[13], g2.2[14]), (gufo_base + geom_ufo_slot(2) * 4, 0));
        assert_eq!(Rig::ufo(&t.cmds(FCCCB, t.r.fwo)[0].2), (gufo_base + geom_ufo_slot(1) * 4, 1));
        assert_eq!(Rig::ufo(&t.cmds(FCCCB, t.r.fwo)[2].2), (t.k.heap_fwaddr(FUFO), 1));
    }

    /// The knob adds the PR commands and their geometry words, and nothing
    /// else: the 3D body, the FENCE and the trailing UPDATE, and every other
    /// geometry word, are byte-identical between knob on and knob off for
    /// the same kick.
    #[test]
    fn the_knob_changes_only_the_pr_commands_and_the_geometry_fence_words() {
        // Both rigs start at seq 0, so the same kick sequence lines up.
        let (mut on, mut off) = (rig(1), rig(0));
        for tag in 1..=2u32 {
            let (mut a, mut b) = (0u32, 0u32);
            assert_eq!(submit(&mut on.k, &mut on.r, &kick(tag), &mut a), 0, "log:\n{}", on.log());
            assert_eq!(submit(&mut off.k, &mut off.r, &kick(tag), &mut b), 0, "log:\n{}", off.log());
            // GEOM: knob off writes 2 commands per kick, knob on 3.
            let g = on.cmds(GCCCB, on.r.gwo);
            let h = off.cmds(GCCCB, off.r.gwo);
            let (geom_on, geom_off) = (&g[3 * (tag - 1) as usize], &h[2 * (tag - 1) as usize]);
            assert_eq!((geom_on.0, geom_on.1), (geom_off.0, geom_off.1));
            for i in 0..16 {
                if i != 13 && i != 14 {
                    assert_eq!(geom_on.2[i], geom_off.2[i], "GEOM dw{i} of kick {tag}");
                }
            }
            // The fragment ring's ordinary trio (knob on: 6 commands per
            // kick, the trio the last three).
            let f = on.cmds(FCCCB, on.r.fwo);
            let e = off.cmds(FCCCB, off.r.fwo);
            let i_on = 6 * (tag - 1) as usize;
            let i_off = 3 * (tag - 1) as usize;
            for (x, y) in f[i_on + 3..i_on + 6].iter().zip(&e[i_off..i_off + 3]) {
                assert_eq!((x.0, x.1, &x.2), (y.0, y.1, &y.2), "fragment command of kick {tag}");
            }
        }
    }

    /// With the knob, building an HWRT dataset fills its partial-render
    /// accounting (design Part 4): sRTACtl names the two render-target
    /// arrays and ui32MaxRTs, the valid array holds the render target
    /// indices, the counts array is zero for the firmware to count into.
    /// Asserted on both pair members (each kick builds one).
    #[test]
    fn with_the_knob_the_hwrt_data_carries_the_partial_render_accounting() {
        let mut t = rig(1);
        let (mut f1, mut f2) = (0u32, 0u32);
        assert_eq!(submit(&mut t.k, &mut t.r, &kick(1), &mut f1), 0, "log:\n{}", t.log());
        assert_eq!(submit(&mut t.k, &mut t.r, &kick(2), &mut f2), 0, "log:\n{}", t.log());
        for (hrt, rtav, rtap) in [(HRT0, RTA0V, RTA0P), (HRT1, RTA1V, RTA1P)] {
            let ctl = hrt as usize + HWRTDATA_RTA_CTL;
            assert_eq!(t.word(ctl + RTA_CTL_VALID_RENDER_TARGETS), t.k.heap_fwaddr(rtav));
            assert_eq!(t.word(ctl + RTA_CTL_NUM_PARTIAL_RENDERS), t.k.heap_fwaddr(rtap));
            assert_eq!(t.word(ctl + RTA_CTL_MAX_RTS), MAX_RT as u32);
            // The counters and flags words are the firmware's; the host
            // hands them out zero.
            assert_eq!(t.words(ctl, 4), [0; 4]);
            assert_eq!(t.word(ctl + RTA_CTL_FLAGS), 0);
            assert_eq!(t.words(rtav as usize, MAX_RT), [0, 1, 2]);
            assert_eq!(t.words(rtap as usize, MAX_RT), [0; MAX_RT]);
        }
    }

    /// Without the knob, none of that is written: the sRTACtl words and the
    /// two array regions stay zero, so a built HWRT data is byte-for-byte
    /// what it was before the feature existed.
    #[test]
    fn without_the_knob_the_partial_render_words_stay_zero() {
        let mut t = rig(0);
        let (mut f1, mut f2) = (0u32, 0u32);
        assert_eq!(submit(&mut t.k, &mut t.r, &kick(1), &mut f1), 0, "log:\n{}", t.log());
        assert_eq!(submit(&mut t.k, &mut t.r, &kick(2), &mut f2), 0, "log:\n{}", t.log());
        for (hrt, rtav, rtap) in [(HRT0, RTA0V, RTA0P), (HRT1, RTA1V, RTA1P)] {
            assert_eq!(t.words(hrt as usize + HWRTDATA_RTA_CTL, HWRTDATA_RTA_CTL_SIZE / 4), [0; 8]);
            assert_eq!(t.words(rtav as usize, MAX_RT), [0; MAX_RT]);
            assert_eq!(t.words(rtap as usize, MAX_RT), [0; MAX_RT]);
        }
    }
}
