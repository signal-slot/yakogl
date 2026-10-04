// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Vivante GC7000 backend: one resolved GL draw becomes LOAD_STATE
//! commands and a DRAW_INSTANCED in the frame's command stream.
//!
//! The GPU is immediate mode and has no firmware, so a frame is one command
//! stream that the front end fetches: draws accumulate into it until the
//! render target changes, the buffer fills, or the GL core asks for a flush,
//! and then the stream is submitted through the OSAL, which appends the EVENT
//! that raises the fence ([`super::kick`]).
//!
//! Every draw writes the whole state block, in ascending register order,
//! because the hardware wants some registers written together and because a
//! first backend that is obviously correct beats one that is minimal. Mesa's
//! etnaviv driver (etnaviv_emit.c, etnaviv_state.c) is the reference for the
//! encodings; VIVANTE_NOTES.md says what each group is for.

use core::ffi::c_void;

use crate::backend::*;
use crate::ds_retention;
use crate::osal::{self, OsalGpuMem, OSAL_MEM_DATA};
use crate::shaderc::{
    ShcShader, SHC_BF_CONST_ALPHA, SHC_BF_CONST_COLOR, SHC_BF_DST_ALPHA, SHC_BF_DST_COLOR, SHC_BF_INV_CONST_ALPHA,
    SHC_BF_INV_CONST_COLOR, SHC_BF_INV_DST_ALPHA, SHC_BF_INV_DST_COLOR, SHC_BF_INV_SRC_ALPHA, SHC_BF_INV_SRC_COLOR,
    SHC_BF_ONE, SHC_BF_SRC_ALPHA, SHC_BF_SRC_ALPHA_SATURATE, SHC_BF_SRC_COLOR, SHC_BF_ZERO, SHC_BLEND_ADD,
    SHC_VIV_NO_REG,
};
use crate::util::{Global, ENOMEM};

use super::clear_shader::{GC_CLEAR_PS, GC_CLEAR_VS};
use super::cmd::GcCmd;
use super::kick::{GcKick, GC_KICK_MAGIC, GC_KICK_TAIL_BYTES};
use super::regs::*;
use super::state::*;
use super::{shader, texture};

/* Device-VA heaps. The GC7000's MMU addresses 32 bits, so the bases are low
 * and far enough apart for the pools the GL core allocates in. */
/// Buffers, textures, render targets.
const GC_HEAP_DATA: u64 = 0x4000_0000;

/// The data heap's base (the tiled copies live there too).
pub(super) const fn heap_data() -> u64 {
    GC_HEAP_DATA
}
/// Shader code the ICACHE fetches.
const GC_HEAP_SHADER: u64 = 0x5000_0000;
/// Command streams and frame arenas.
const GC_HEAP_COMMAND: u64 = 0x6000_0000;

/// Command streams in flight. The GL core's swap pipelines the CPU against
/// the GPU (`finish_previous`: everything but the frame just swapped), so the
/// streams of one whole swap-frame are running while the next one records.
/// A swap-frame is one kick when its stream fits, but a full stream, a target
/// change or glFlush closes a kick early, and a stream slot is reused only
/// once its kick has completed (`frame_open`) -- with two slots a two-kick
/// frame made the next frame's first kick wait for this frame's first, which
/// undid most of the overlap. Four leave room for two kicks a frame.
const GC_FRAMES: usize = 4;
/// Words in one command stream. Slint's cluster records about 66 K words a
/// frame (263 draws, 2026-10-01, gc_slint on the Verdin): 64 K split every
/// frame into two kicks.
const GC_CMD_WORDS: usize = 128 * 1024;
const GC_ARENA_BYTES: usize = 1024 * 1024;

/// A guard either side of the depth surface, for the canary probe. The pixel
/// engine is told the surface starts this far into the allocation, so anything
/// it writes outside the surface still lands in memory this driver owns --
/// which is what makes an overrun readable instead of a fatal somewhere else
/// entirely.
#[cfg(feature = "vivante-depth-guard")]
const GC_DEPTH_GUARD_BYTES: usize = 256 * 1024;
#[cfg(not(feature = "vivante-depth-guard"))]
const GC_DEPTH_GUARD_BYTES: usize = 0;

pub(crate) struct GcState {
    inited: bool,
    frames: [GcFrame; GC_FRAMES],
    /// The frame being recorded.
    cur: usize,
    recording: bool,
    /// Id of the last submitted frame.
    kick_id: u32,
    kicks: u32,
    waits: u32,
    /// The fence of the last kick submitted before the latest swap: what the
    /// next swap waits for ([`gc_finish_previous`]).
    swap_fence: i64,
    /// GL_VARYING_COMPONENT_USE may hold a marked point coordinate: the last
    /// draw wrote one, or this process has not written the register yet and
    /// the GPU keeps what a previous client left (the class gl_test 63 found
    /// for the vertex-id register). The next draw writes it back to all
    /// GENERIC (see `shader::emit_shader_state`). True from `gc_init`.
    component_use_marked: bool,
    /// GL_VARYING_COMPONENT_USE has been written since gc_init.
    component_use_written: bool,
    /// Draws refused since init ([`gc_refused_draws`]), and the return codes
    /// already logged.
    refused: u32,
    refused_logged: [i32; 4],
    refused_nlogged: usize,
    /// The draw-buffer-0 refusal (review R-09) has been logged since init.
    refused_draw_buffer0_logged: bool,
    /// The smaller-target refusal (review R-02) has been logged since init.
    refused_sizes_logged: bool,

    /* the target the open frame is drawing to; a change closes the frame */
    target_gpu: u64,
    target_w: u32,
    target_h: u32,

    /* one depth/stencil buffer, grown to the largest target seen */
    pub(super) depth: OsalGpuMem,
    pub(super) depth_stride: u32,
    pub(super) depth_w: u32,
    pub(super) depth_h: u32,
    /// Where the surface starts inside that allocation. Zero normally; with
    /// the guard build it is [`GC_DEPTH_GUARD_BYTES`].
    pub(super) depth_surface_off: u32,

    /* The scratch surface a multisampled draw rasterises into, and where the
     * resolve engine has to copy it back to when the frame ends. */
    msaa: OsalGpuMem,
    msaa_bytes: u32,
    /* The colour surface a draw with no colour target names
     * ([`ensure_no_colour_surface`]); never written. */
    no_colour: OsalGpuMem,
    ms_dest_gpu: u64,
    ms_dest_w: u32,
    ms_dest_h: u32,
    ms_dest_bpp: u32,
    /// The destination's row pitch in bytes (`draw_pitch` x bpp: a window
    /// that is a view into a wider buffer keeps its own).
    ms_dest_stride: u32,

    /* the clear quad's two shaders, uploaded once */
    clear_vs_code: OsalGpuMem,
    clear_ps_code: OsalGpuMem,
    /// One opaque black RGBA8 texel: what GL's incomplete texture samples as
    /// (GL ES 2.0 section 3.7.10). Allocated once, bound in place of any
    /// texture the GL core marks incomplete.
    black_tex: OsalGpuMem,
}

impl GcState {
    const ZERO: GcState = GcState {
        inited: false,
        frames: [GcFrame::ZERO; GC_FRAMES],
        cur: 0,
        recording: false,
        kick_id: 0,
        kicks: 0,
        waits: 0,
        swap_fence: 0,
        component_use_marked: false,
        component_use_written: false,
        refused: 0,
        refused_logged: [0; 4],
        refused_nlogged: 0,
        refused_draw_buffer0_logged: false,
        refused_sizes_logged: false,
        target_gpu: 0,
        target_w: 0,
        target_h: 0,
        depth: OsalGpuMem::ZERO,
        depth_stride: 0,
        depth_w: 0,
        depth_h: 0,
        depth_surface_off: 0,
        msaa: OsalGpuMem::ZERO,
        msaa_bytes: 0,
        no_colour: OsalGpuMem::ZERO,
        ms_dest_gpu: 0,
        ms_dest_w: 0,
        ms_dest_h: 0,
        ms_dest_bpp: 0,
        ms_dest_stride: 0,
        clear_vs_code: OsalGpuMem::ZERO,
        clear_ps_code: OsalGpuMem::ZERO,
        black_tex: OsalGpuMem::ZERO,
    };
}

static G: Global<GcState> = Global::new(GcState::ZERO);

/// The backend's state. Single-threaded by contract, like the rest of the
/// library: an entry point takes it once and passes it down.
/// Leave only `leave` bytes of the open frame's arena, for the tests in
/// `backend_tests.rs`: a descriptor that does not fit is otherwise reachable
/// only by recording thousands of draws. Says what the test wants rather than
/// handing out the backend's state, which is private.
#[cfg(test)]
pub(super) fn squeeze_frame_arena_for_tests(leave: usize) {
    let g = g();
    let cur = g.cur;
    let f = &mut g.frames[cur];

    f.arena_used = (f.arena.size.saturating_sub(leave)) as u32;
}

pub(super) fn g() -> &'static mut GcState {
    // SAFETY: single-threaded by contract; no other reference is live.
    unsafe { G.get() }
}

/* ---- small helpers ------------------------------------------------------ */

/// 16.16 fixed point, the form the viewport and scissor registers take.
fn f2fixp(f: f32) -> u32 {
    (f * 65536.0) as i32 as u32
}

fn unorm8(f: f32) -> u32 {
    if f <= 0.0 {
        0
    } else if f >= 1.0 {
        255
    } else {
        (f * 255.0 + 0.5) as u32
    }
}

/// IEEE binary32 -> binary16, for the half-float blend colour registers.
fn f2half(f: f32) -> u32 {
    let u = f.to_bits();
    let sign = (u >> 16) & 0x8000;
    let exp = ((u >> 23) & 0xff) as i32 - 127 + 15;
    let mant = u & 0x007f_ffff;

    if (u >> 23) & 0xff == 0xff {
        return sign | 0x7c00 | if mant != 0 { 0x200 } else { 0 };
    }
    if exp >= 0x1f {
        return sign | 0x7c00;
    }
    if exp <= 0 {
        return sign; /* denormals flush to zero */
    }
    sign | ((exp as u32) << 10) | (mant >> 13)
}

fn align_up(v: u32, a: u32) -> u32 {
    (v + a - 1) & !(a - 1)
}

/* ---- render target ------------------------------------------------------ */

/// The pixel engine's colour format, and whether it is in the extended set.
/// It writes 32-bit pixels in B,G,R,A order, which is exactly the window
/// surface's XRGB8888; `YakoglBackend::rt_rgba8888_is_bgra` is what tells the
/// GL core to swap an RGBA8888 target back on read-back.
fn pe_color_format(surf_format: u8) -> (u32, bool) {
    match surf_format {
        GLES_SURF_RGB565 => (GC_PE_FORMAT_R5G6B5, false),
        GLES_SURF_XRGB8888 => (GC_PE_FORMAT_X8R8G8B8, false),
        GLES_SURF_RGBA16F => (GC_PE_FORMAT_A16B16G16R16F, true),
        GLES_SURF_R16F => (GC_PE_FORMAT_R16F, true),
        GLES_SURF_R32F => (GC_PE_FORMAT_R32F, true),
        _ => (GC_PE_FORMAT_A8R8G8B8, false),
    }
}

/// The depth/stencil buffer. The pixel engine reads and writes it tiled, so
/// only it ever touches these bytes; the backend just has to give it enough
/// room and a stride. One buffer serves every target: a draw either clears it
/// or continues from what the previous draw to the same target left. It is
/// padded to 64x64, which is a super-tile -- what PE_DEPTH_CONFIG tells the
/// pixel engine this surface is.
/// Multisampling, and whether the rasteriser is told about it.
///
/// 4x rasterises into a scratch surface twice the size on each axis and the
/// resolve engine averages it back. OFF unless [`gc_debug_msaa`] asks, until
/// the board has measured it: the first attempt never wrote the target, and
/// set up the surface and the resolve unlike etnaviv in four ways (Mesa,
/// MIT), which is what the switch now carries --
///
/// 1. a multisampled render target is SUPERTILED (etnaviv_resource.c: the
///    "MSAA render target" layout), padded to 64x64; ours was linear;
/// 2. the PE is told so: PE_COLOR_FORMAT_SUPER_TILED and, on a core with
///    CACHE128B256BPERLINE (this one), _SUPER_TILED_NEW; and never OVERWRITE
///    on a multisampled target (etnaviv_state.c: only when nr_samples <= 1);
/// 3. the resolve reads a tiled source: RS_CONFIG_SOURCE_TILED, the source
///    stride shifted left by 2 with RS_SOURCE_STRIDE_TILING and
///    _SUPER_TILED_NEW (etnaviv_rs.c, etna_compile_rs_state);
/// 4. RS_WINDOW_SIZE is in SOURCE samples, twice the target on each axis
///    (etna_try_rs_blit: "the RS width/height are converted to source
///    samples"); ours was the target's size.
const GC_MSAA_ENABLE: bool = false;
const GC_MSAA_RASTERISE: bool = true;

/// Performance experiments (gc_slint's perf selector), 0 in every product
/// build: bit 0 clips every draw and clear to an 8x8 box (what is left is
/// the GPU's per-draw cost without the fill), bit 1 leaves out the per-draw
/// cache flushes and pipeline stalls (wrong pictures possible; it measures
/// what they cost).
#[no_mangle]
pub static mut gc_debug_perf_exp: u32 = 0;

/// [`gc_debug_perf_exp`] bit 0.
pub const GC_PERF_EXP_TINY_CLIP: u32 = 1;
/// [`gc_debug_perf_exp`] bit 1.
pub const GC_PERF_EXP_NO_DRAW_SYNC: u32 = 2;

/// [`gc_debug_perf_exp`] bit 2: every draw's fragment stage runs the clear
/// shader's one instruction instead of its own (what is left is the fill
/// without the fragment shader's cost).
pub const GC_PERF_EXP_TRIVIAL_PS: u32 = 4;

/// [`gc_debug_perf_exp`] bit 3: only the first `exp >> 5` draws of each
/// frame (counted from the last swap) are clipped to 8x8 -- what the first
/// few full-screen draws (the clear, the background) cost.
pub const GC_PERF_EXP_CLIP_FIRST: u32 = 8;
/// [`gc_debug_perf_exp`] bit 4: every clear (the full-screen quad a glClear
/// becomes) is clipped to 8x8, the draws are not -- what the clears cost.
pub const GC_PERF_EXP_CLIP_CLEARS: u32 = 16;

/// Draws recorded since the last swap, for [`GC_PERF_EXP_CLIP_FIRST`].
static mut PERF_FRAME_DRAWS: u32 = 0;

fn perf_tiny(_d: &GlesDraw) -> bool {
    if perf_exp(GC_PERF_EXP_TINY_CLIP) {
        return true;
    }
    if !perf_exp(GC_PERF_EXP_CLIP_FIRST) {
        return false;
    }
    // SAFETY: plain reads of ints only this thread writes.
    unsafe { core::ptr::addr_of!(PERF_FRAME_DRAWS).read() < (core::ptr::addr_of!(gc_debug_perf_exp).read() >> 5) }
}

/// The clear's fragment code, when [`GC_PERF_EXP_TRIVIAL_PS`] is on.
pub(super) fn perf_trivial_ps() -> Option<(u32, u32)> {
    if !perf_exp(GC_PERF_EXP_TRIVIAL_PS) {
        return None;
    }
    Some((g().clear_ps_code.gpu as u32, super::clear_shader::GC_CLEAR_PS.0.usc_dw / 4))
}

/// Set [`gc_debug_perf_exp`].
#[no_mangle]
pub extern "C" fn gc_debug_set_perf_exp(bits: u32) {
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(gc_debug_perf_exp).write(bits) }
}

fn perf_exp(bit: u32) -> bool {
    // SAFETY: a plain read of an int only the setter writes.
    unsafe { core::ptr::addr_of!(gc_debug_perf_exp).read() & bit != 0 }
}

/// Command words submitted since start-up, for a measurement to read.
#[no_mangle]
pub static mut gc_perf_cmd_words: u64 = 0;

/// Turns 4x multisampling on for measurement (gl_test_user
/// `GL_TEST_GC_MSAA`), and with it the EGL 4x config (gl/egl.rs `offered_configs`).
/// 0 in every product build until the board has run it.
#[no_mangle]
pub static mut gc_debug_msaa: i32 = 0;

/// Set [`gc_debug_msaa`].
#[no_mangle]
pub extern "C" fn gc_debug_set_msaa(on: i32) {
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(gc_debug_msaa).write(on) }
}

/// Flat shading for a program with a flat varying: ON. PA_CONFIG's
/// SHADE_MODEL goes FLAT whenever a fragment input is flat, as Mesa's
/// etnaviv does ("if shader has flat varyings, switch to flat shading",
/// etnaviv_shader.c, MIT): PA_SYSTEM_MODE's PROVOKING_VERTEX_LAST governs
/// only the flat model, and the smooth one takes the first vertex for a flat
/// varying. gl_test 89 (flatlast) reads GL's last vertex with it on. 0 turns
/// it off again, for an A/B.
#[no_mangle]
pub static mut gc_debug_flat_shade: i32 = 1;

/// Set [`gc_debug_flat_shade`].
#[no_mangle]
pub extern "C" fn gc_debug_set_flat_shade(on: i32) {
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(gc_debug_flat_shade).write(on) }
}

/// A colour clear writes only the channels glColorMask had on at the glClear
/// (`GlesDraw::clear_color_mask`) instead of all four: the clear quad programs
/// PE_COLOR_FORMAT's COMPONENTS from it, the field every draw already takes
/// its colour mask from, and loses the OVERWRITE hint with any channel off,
/// as a masked draw does. On by default since the i.MX8MP measured it:
/// gl_test 102 maskclear off FAIL (80ffffff, the partially masked clear wrote
/// all channels), on PASS; dEQP-GLES2 color_clear 18/19 with it on (the one
/// left, long_masked_rgba, is its own row); fbo.render's rbo_rgb5_a1
/// color_clear and stencil_clear end with an alpha-only masked clear and
/// zeroed RGB with it off. 0 is the off arm of an A/B.
///
/// Found by reading the code against dEQP-GLES2.functional.color_clear, whose
/// masked cases clear under random partial masks. The prediction that they
/// were color_clear's eight failures on the i.MX8MP was REFUTED by the
/// per-case record (Zephyr, dafa9962): all eight masked cases passed there,
/// the failures were unmasked clears, and masked_rgb run alone failed. What
/// the masked cases need is measured by gl_test 102 with this switch off and
/// on, not predicted from dEQP.
#[no_mangle]
pub static mut gc_debug_masked_clear: i32 = 1;

/// A plain 2D texture the vertex stage samples is sampled from a tiled copy
/// (tiled_copy.rs `wanted_vs`) rather than its linear storage. Default 0,
/// the off arm, until the board has measured it: every dEQP
/// texture.vertex.2d case fails on the i.MX8MP (55b849ec) while the cube
/// cases, already sampled from the copy, pass, and Mesa resolves every
/// texture into a TILED one before this core's vertex stage reads it
/// (tools/vivante golden mode 26). texture_functions.vertex.texture2dlod
/// passes over a linear 2D texture, so the layout is the hypothesis, not a
/// finding; 1 is the arm that tests it.
#[no_mangle]
pub static mut gc_debug_vs_tiled: i32 = 0;

/// SAMP_CTRL0's INT_FILTER on every sampler of a UNORM texel format that is
/// not a 3D texture (texture.rs `int_filter_wanted`), as Mesa sets it
/// (etnaviv_format.c texture_use_int_filter, reached from
/// etnaviv_texture_desc.c; golden mode 26 carries it on the vertex-stage
/// sampler, we never did). Default 0 until the board has measured it: it
/// is in EVERY such sampler, fragment stage included, so it can move
/// fragment-stage results too. The GL core stores a depth texture in RGBA8
/// and the binding does not say so, so a depth texture gets the bit here
/// where Mesa leaves it off.
#[no_mangle]
pub static mut gc_debug_int_filter: i32 = 0;

/// Set [`gc_debug_int_filter`].
#[no_mangle]
pub extern "C" fn gc_debug_set_int_filter(on: i32) {
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(gc_debug_int_filter).write(on) }
}

/// Is [`gc_debug_int_filter`] on?
pub fn int_filter_on() -> bool {
    // SAFETY: as for gc_debug_msaa; a plain read.
    (unsafe { core::ptr::addr_of!(gc_debug_int_filter).read() }) != 0
}

/// Set [`gc_debug_vs_tiled`].
#[no_mangle]
pub extern "C" fn gc_debug_set_vs_tiled(on: i32) {
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(gc_debug_vs_tiled).write(on) }
}

/// Is [`gc_debug_vs_tiled`] on?
pub fn vs_tiled_on() -> bool {
    // SAFETY: as for gc_debug_msaa; a plain read.
    (unsafe { core::ptr::addr_of!(gc_debug_vs_tiled).read() }) != 0
}

/// A plain 2D texture the fragment stage samples is sampled from a tiled
/// copy too (tiled_copy.rs `wanted_fs`). Default 0: an experiment, whether
/// the texture unit reads a linear texture slower than a tiled one (a
/// full-screen 1:1 textured quad, gc_slint route 3's background, costs
/// ~4.8 ms a frame at 1280x800 with a one-instruction shader).
#[no_mangle]
pub static mut gc_debug_fs_tiled: i32 = 0;

/// Set [`gc_debug_fs_tiled`].
#[no_mangle]
pub extern "C" fn gc_debug_set_fs_tiled(on: i32) {
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(gc_debug_fs_tiled).write(on) }
}

/// Is [`gc_debug_fs_tiled`] on?
pub fn fs_tiled_on() -> bool {
    // SAFETY: as for gc_debug_msaa; a plain read.
    (unsafe { core::ptr::addr_of!(gc_debug_fs_tiled).read() }) != 0
}

/// GL_LUMINANCE, GL_ALPHA and GL_LUMINANCE_ALPHA textures stored narrow (one
/// and two bytes a texel: GLES_TEXFMT_L8 / A8 / LA8) and sampled with the
/// texture unit's own L8 / A8 / A8L8 formats, as Mesa does, instead of
/// expanded into RGBA8 -- a quarter of the memory for a 2048x2048 L8 chain
/// (dEQP texture.size's 2048 cases, 22 MiB as RGBA8). Default 0 until the
/// board has measured it: the linear pitch of a one- and two-byte texel
/// (a 1-texel-wide level is a 2-byte row) is unmeasured on this core;
/// gl_test 132 runs both arms.
#[no_mangle]
pub static mut gc_debug_narrow_la: i32 = 0;

/// Set [`gc_debug_narrow_la`]; returns the previous value (gl_test 132 runs
/// both arms and puts it back, as the PowerVR's `pvr_narrow_la`). A texture
/// keeps the store it was specified with.
#[no_mangle]
pub extern "C" fn gc_debug_set_narrow_la(on: i32) -> i32 {
    // SAFETY: single-threaded by contract; plain loads and stores of an int.
    unsafe { core::ptr::addr_of_mut!(gc_debug_narrow_la).replace(on) }
}

/// Is [`gc_debug_narrow_la`] on?
pub fn narrow_la_on() -> bool {
    // SAFETY: as for gc_debug_msaa; a plain read.
    (unsafe { core::ptr::addr_of!(gc_debug_narrow_la).read() }) != 0
}

/// Set [`gc_debug_masked_clear`].
#[no_mangle]
pub extern "C" fn gc_debug_set_masked_clear(on: i32) {
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(gc_debug_masked_clear).write(on) }
}

/// Is [`gc_debug_masked_clear`] on?
pub fn masked_clear_on() -> bool {
    // SAFETY: as for gc_debug_msaa; a plain read.
    (unsafe { core::ptr::addr_of!(gc_debug_masked_clear).read() }) != 0
}

/// The channels a clear writes: its glClear's colour mask with the switch
/// on, all four otherwise, none when it clears no colour. A mask of 0 with a
/// colour clear is a producer that predates the field, and means all four.
fn clear_components(d: &GlesDraw, clear_color: bool) -> u32 {
    if !clear_color {
        return 0;
    }
    let m = u32::from(d.clear_color_mask) & 0xf;
    if masked_clear_on() && m != 0 {
        m
    } else {
        0xf
    }
}

/// The process's first draw writes GL_VARYING_COMPONENT_USE GENERIC (ON,
/// 02fc8b79). 0 leaves the registers at whatever they hold until a program
/// needs them, as before 02fc8b79: an A/B for gl_test 37, which fails alone
/// since 726e26ce and whose first draw differs from 5c014c9d's in exactly
/// these four writes and the black texel ([`gc_debug_black_unbound`]).
#[no_mangle]
pub static mut gc_debug_first_component_use: i32 = 1;

/// Set [`gc_debug_first_component_use`].
#[no_mangle]
pub extern "C" fn gc_debug_set_first_component_use(on: i32) {
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(gc_debug_first_component_use).write(on) }
}

/// A sampler the shader declares with nothing bound reads the black texel,
/// and the vertex stage's slots start after every declared fragment slot
/// (ON, 00d8b62f, gl_test 61). 0 restores the stream before it: no
/// descriptor for such a slot and VS_SAMPLER_BASE = the bound fragment
/// textures -- the other half of gl_test 37's A/B.
#[no_mangle]
pub static mut gc_debug_black_unbound: i32 = 1;

/// Set [`gc_debug_black_unbound`].
#[no_mangle]
pub extern "C" fn gc_debug_set_black_unbound(on: i32) {
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(gc_debug_black_unbound).write(on) }
}

/// Is [`gc_debug_black_unbound`] on?
pub(crate) fn black_unbound_on() -> bool {
    // SAFETY: as for gc_debug_msaa; a plain read.
    (unsafe { core::ptr::addr_of!(gc_debug_black_unbound).read() }) != 0
}

/// How a draw's texture descriptor writes are ordered and fenced, as a
/// bitmask. texture.rs `emit_desc_seq` emits it, and each bit counts the draws
/// it changed (`texture::desc_seq_emits`).
///
/// The default is [`GC_DESC_SEQ_MESA_ORDER`] alone: on the i.MX8MP (GC7000UL)
/// a slot whose INVALIDATE followed its own ADDR write, interleaved with the
/// next slot's, was sometimes read with the previous draw's descriptor; the
/// vertex sampler sampled a stale texture and dEQP uniform_api's two
/// sampler2D_samplerCube_both render cases hung the GPU. Mesa writes every
/// address first and then every invalidate (etnaviv_texture_desc.c 605-610,
/// Mesa main c8ce6c8bdfbb). With that order the board (gclab sampler_split)
/// drew F3V1 240/240 and F4V1 150/150 right with 0 GPU recoveries, where the
/// old order drew vec4w 0/5 right with 5 recoveries; dEQP uniform_api went
/// from 1250 pass / 2 fail to 1252 / 0 (records/imx8mp/sampler-desc-seq/:
/// table.txt sha256 83df2f1cf99ddc8e, regression-seq1.txt ae3588e340f0049f).
/// 0 is the old order, kept as the control arm of that A/B
/// (YAKOGL_GC_DESC_SEQ=0, CONFIG_GC_GL_TEST_DESC_SEQ=0).
/// - [`GC_DESC_SEQ_MESA_ORDER`] (1): every NTE_DESCRIPTOR_ADDR write of the
///   draw first, then every INVALIDATE in slot order; off, each slot's
///   INVALIDATE follows its own ADDR write.
/// - [`GC_DESC_SEQ_TEX_FLUSH`] (2): GL_FLUSH_CACHE TEXTURE, then TEXTUREVS,
///   then an RA-PE stall, after the descriptor writes (Mesa emits this triple
///   before them, etnaviv_emit.c 268-293, as our emit_flushes does).
/// - [`GC_DESC_SEQ_DESC_FLUSH`] (4): NTE_DESCRIPTOR_FLUSH and GL_FLUSH_CACHE
///   DESCRIPTOR after the descriptor writes (both sides emit the pair only at
///   stream start: our emit_context_init).
/// - [`GC_DESC_SEQ_FE_PE_STALL`] (8): an FE-PE stall after the descriptor
///   writes (both sides stall before them: etnaviv_emit.c 727-731, our
///   emit_draw).
///
/// Bits 2, 4 and 8 are A/B arms only: on the board bit 4 did not remove the
/// stale descriptor, bit 8 did by stalling, bit 2 in part.
#[no_mangle]
pub static mut gc_debug_desc_seq: u32 = GC_DESC_SEQ_MESA_ORDER;

/// [`gc_debug_desc_seq`] bit 1: all addresses, then all invalidates.
pub const GC_DESC_SEQ_MESA_ORDER: u32 = 1;
/// [`gc_debug_desc_seq`] bit 2: texture cache flushes + RA-PE stall after.
pub const GC_DESC_SEQ_TEX_FLUSH: u32 = 2;
/// [`gc_debug_desc_seq`] bit 4: descriptor cache flush after.
pub const GC_DESC_SEQ_DESC_FLUSH: u32 = 4;
/// [`gc_debug_desc_seq`] bit 8: FE-PE stall after.
pub const GC_DESC_SEQ_FE_PE_STALL: u32 = 8;
/// Every bit [`gc_debug_desc_seq`] knows.
pub const GC_DESC_SEQ_ALL: u32 =
    GC_DESC_SEQ_MESA_ORDER | GC_DESC_SEQ_TEX_FLUSH | GC_DESC_SEQ_DESC_FLUSH | GC_DESC_SEQ_FE_PE_STALL;

/// Set [`gc_debug_desc_seq`]. -1, and the value is left as it was, when
/// `bits` holds a bit outside [`GC_DESC_SEQ_ALL`].
#[no_mangle]
pub extern "C" fn gc_debug_set_desc_seq(bits: u32) -> i32 {
    if bits & !GC_DESC_SEQ_ALL != 0 {
        return -1;
    }
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(gc_debug_desc_seq).write(bits) }
    0
}

/// [`gc_debug_desc_seq`]'s value.
pub(crate) fn desc_seq() -> u32 {
    // SAFETY: as for gc_debug_msaa; a plain read.
    unsafe { core::ptr::addr_of!(gc_debug_desc_seq).read() }
}

/// Draw buffers (MRT step 2): with this on, a draw writes every render target
/// the GL core gives it (`GlesDraw::targets`, up to four), programming RT1..3
/// in the 8-RT bank; off, RT0 only, as before. Off by default until the board
/// has measured it (gl_test 106).
#[no_mangle]
pub static mut gc_debug_mrt: i32 = 0;

/// Set [`gc_debug_mrt`].
#[no_mangle]
pub extern "C" fn gc_debug_set_mrt(on: i32) {
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(gc_debug_mrt).write(on) }
}

/// Is [`gc_debug_mrt`] on?
pub fn mrt_on() -> bool {
    // SAFETY: as for gc_debug_msaa; a plain read.
    (unsafe { core::ptr::addr_of!(gc_debug_mrt).read() }) != 0
}

/// Sixteen texture units in the vertex stage: with this on,
/// GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS and gl_MaxVertexTextureImageUnits answer
/// 16 and the link and the draw take that many vertex samplers
/// ([`crate::backend::vs_tex_units`]); off, 8, as before.
///
/// What the hardware has, from Mesa (not measured here): etnaviv gives the
/// vertex stage 16 samplers on every HALTI >= 1 core
/// (etnaviv_screen.c:821-828 `etna_determine_sampler_limits`), and the
/// i.MX8MP's GC7000UL r6204 (hwdb nxp/gc_feature_database.h, the two
/// gc7000ULN_v123 entries at :18841 and :19735) has REG_Halti5 and
/// REG_UnifiedSamplers. On such a core the two stages share one array of
/// descriptor slots, the vertex shader's samplers are 0-based and
/// VS_SAMPLER_BASE says where they start (etnaviv_texture.c:429-439
/// `etna_vs_sampler_base`: after the fragment views;
/// etnaviv_compiler_nir_emit.c:210-214). This backend does the same
/// (texture.rs `emit_textures_inner`): the vertex slots start after every
/// fragment slot, at most 16, so sixteen vertex samplers end at slot 31,
/// inside the 32-slot INVALIDATE mask and NTE_DESCRIPTOR's array.
///
/// On by default since 2026-10-01, after the board measured it: gl_test 149
/// with 9..16 vertex samplers beside 0 and 16 fragment ones, and dEQP-GLES2
/// uniform_api 1,267 / 1,267 on Zephyr and on the Linux harness once the
/// constant file reached the GPU whole (uniform_api.random.3, the first
/// program with 13 vertex samplers, also had 275 constant registers). 0
/// selects the earlier 8 ([`VS_TEX_UNITS_SWITCHED_OFF`]); the samples'
/// Kconfig and YAKOGL_GC_VS_UNITS16 in the Linux OSALs set it. Set it before
/// the first GL call: the limit is read at link and at compile.
#[no_mangle]
pub static mut gc_debug_vs_units16: i32 = 1;

/// The vertex texture units with [`gc_debug_vs_units16`] at 0: the limit
/// this backend had until 2026-10-01.
pub const VS_TEX_UNITS_SWITCHED_OFF: usize = 8;

/// Set [`gc_debug_vs_units16`].
#[no_mangle]
pub extern "C" fn gc_debug_set_vs_units16(on: i32) {
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(gc_debug_vs_units16).write(on) }
}

/// Is [`gc_debug_vs_units16`] on?
///
/// A unit test reads its own thread's setting instead
/// ([`set_vs_units16_for_this_thread`]): the limit is read by compiles and
/// links in tests that do not take the GL lock, and a switch flipped by one
/// test must not change the limit another test running beside it sees.
pub fn vs_units16_on() -> bool {
    #[cfg(test)]
    {
        VS_UNITS16_TEST.with(|c| c.get())
    }
    #[cfg(not(test))]
    {
        // SAFETY: as for gc_debug_msaa; a plain read.
        (unsafe { core::ptr::addr_of!(gc_debug_vs_units16).read() }) != 0
    }
}

#[cfg(test)]
std::thread_local! {
    static VS_UNITS16_TEST: core::cell::Cell<bool> = const { core::cell::Cell::new(true) };
}

/// Unit tests: [`vs_units16_on`] for the calling thread only.
#[cfg(test)]
pub(crate) fn set_vs_units16_for_this_thread(on: bool) {
    VS_UNITS16_TEST.with(|c| c.set(on));
}

/// Does draw buffer `rt` take this draw's colour clear? `clear_target_mask`
/// 0 is every target (glClear), bit n draw buffer n alone
/// (glClearBuffer(GL_COLOR, n)), and 0x80 none at all (glClear with every
/// draw buffer GL_NONE).
fn clear_takes(gl: &GlesDraw, rt: usize) -> bool {
    let m = gl.clear_target_mask;
    m == 0 || (m & 0x80 == 0 && m & (1 << rt) != 0)
}

/// How many render targets this draw writes: the GL core's, up to the four
/// this core drives, with [`gc_debug_mrt`] on; one otherwise.
pub fn mrt_targets(gl: &GlesDraw) -> usize {
    if mrt_on() {
        (gl.num_targets as usize).clamp(1, 4)
    } else {
        1
    }
}

/// The draw buffers this draw programs as hardware render targets, in
/// order: the GL slot of hardware target k at index k, and how many there
/// are. Slot 0 is always the first. A later slot the GL core carries with
/// `gpu` 0 -- a GL_NONE draw buffer, or no attachment, kept so the slots
/// after it keep their numbers (ES 3.0 4.2.1) -- is not a target: nothing is
/// programmed for it and its fragment output is stored nowhere. The slots
/// after it move down one hardware target and take their own shader output
/// through PS_OUTPUT_REG, as Mesa's etnaviv compacts null colour buffers
/// (etnaviv_state.c, etna_set_framebuffer_state: `ps_output_remap[rt] =
/// rt_output[i]`, RT_COUNT from the real targets only).
pub fn hw_targets(gl: &GlesDraw) -> ([u8; 4], usize) {
    let mut slots = [0u8; 4];
    let mut n = 1;
    for slot in 1..mrt_targets(gl) {
        if gl.targets[slot].gpu != 0 {
            slots[n] = slot as u8;
            n += 1;
        }
    }
    (slots, n)
}

/// Does this draw select the flat shade model? Only with the switch on and
/// a fragment input the program declares flat.
fn flat_shade_model(fs: &ShcShader) -> bool {
    // SAFETY: as for gc_debug_msaa; a plain read.
    let on = unsafe { core::ptr::addr_of!(gc_debug_flat_shade).read() } != 0;
    on && fs.viv.in_[..(fs.viv.num_in as usize).min(fs.viv.in_.len())]
        .iter()
        .any(|io| u32::from(io.interp) == GC_INTERP_FLAT)
}

/// With [`gc_debug_msaa`] bit 1 (a value of 3) as well: the three things
/// etnaviv's 4x setup does that this backend did not, for a board run to
/// try as one arm -- PE_LOGIC_OP UNK24 = 5 on a SMALL_MSAA core
/// (etnaviv_state.c), and one more fragment input and temporary
/// (PS_INPUT_COUNT_MSAA, PS_TEMP_REGISTER_CONTROL_MSAA: "MSAA adds another
/// input", etnaviv_shader.c). Mesa, MIT; read and transcribed.
pub fn msaa_extras(gl: &GlesDraw) -> bool {
    // SAFETY: as in msaa_on; a plain read.
    ms_scale(gl) > 1 && unsafe { core::ptr::addr_of!(gc_debug_msaa).read() } & 2 != 0
}

/// What the multisampled scratch surface holds, for the board: `out[0..4]`
/// = its bytes, how many of its dwords are non-zero, the index of the first
/// that is and its value. Read after the frame has finished (the resolve
/// leaves the samples in place), it says which half of a failed 4x frame
/// failed: a scratch with data and a blank target is the resolve's, an
/// empty scratch the draw's. -1 when there is no scratch surface. Reads
/// through the CPU view, which the harnesses map non-cacheable.
///
/// # Safety
/// `out` is null or points at four writable `u32`s.
#[no_mangle]
pub unsafe extern "C" fn gc_debug_msaa_scratch(out: *mut u32) -> i32 {
    let g = g();
    if g.msaa.cpu.is_null() || g.msaa_bytes == 0 || out.is_null() {
        return -1;
    }
    // SAFETY: the scratch surface is `msaa_bytes` long and alive while the
    // backend is; the caller hands four u32s.
    let words = unsafe { core::slice::from_raw_parts(g.msaa.cpu as *const u32, g.msaa_bytes as usize / 4) };
    let first = words.iter().position(|w| *w != 0);
    let res = [
        g.msaa_bytes,
        words.iter().filter(|w| **w != 0).count() as u32,
        first.map_or(u32::MAX, |i| i as u32),
        first.map_or(0, |i| words[i]),
    ];
    // SAFETY: the caller's contract: four writable u32s at `out`.
    unsafe { core::slice::from_raw_parts_mut(out, 4) }.copy_from_slice(&res);
    0
}

/// Which 256-byte chunks of the multisampled scratch surface hold any
/// non-zero byte, one flag per chunk into `out` (at most `n`); returns how
/// many chunks the surface has, or -1 when there is none. A count of
/// written dwords cannot tell a half the PE never reached in y from one in
/// x or from a stride that skips lines; where in the supertiled surface the
/// writes landed can.
///
/// # Safety
/// `out` is null or points at `n` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn gc_debug_msaa_scratch_map(out: *mut u8, n: u32) -> i32 {
    let g = g();
    if g.msaa.cpu.is_null() || g.msaa_bytes == 0 || out.is_null() {
        return -1;
    }
    // SAFETY: the scratch surface is `msaa_bytes` long and alive while the
    // backend is.
    let bytes = unsafe { core::slice::from_raw_parts(g.msaa.cpu as *const u8, g.msaa_bytes as usize) };
    let chunks = bytes.chunks(256);
    let total = chunks.len();
    // SAFETY: the caller's contract: `n` writable bytes at `out`.
    let dst = unsafe { core::slice::from_raw_parts_mut(out, n as usize) };
    for (d, c) in dst.iter_mut().zip(chunks) {
        *d = c.iter().any(|b| *b != 0) as u8;
    }
    total as i32
}

/// Zero the multisampled scratch surface, so a probe can tell what one frame
/// wrote from what an earlier one left. -1 when there is none.
#[no_mangle]
pub extern "C" fn gc_debug_msaa_scratch_zero() -> i32 {
    let g = g();
    if g.msaa.cpu.is_null() || g.msaa_bytes == 0 {
        return -1;
    }
    // SAFETY: the scratch surface is `msaa_bytes` long and alive while the
    // backend is; no frame is in flight when a test calls this after glFinish.
    unsafe { core::ptr::write_bytes(g.msaa.cpu as *mut u8, 0, g.msaa_bytes as usize) };
    0
}

/// Is 4x multisampling on in this build/process?
pub fn msaa_on() -> bool {
    // SAFETY: as above; a plain read.
    GC_MSAA_ENABLE || unsafe { core::ptr::addr_of!(gc_debug_msaa).read() } != 0
}

/// Whether a 4x target of this size can be resolved: the resolve engine
/// takes widths in multiples of 16 (etnaviv aborts otherwise: "scribbling
/// over memory, or the GPU hanging") and, in source samples, heights in
/// multiples of 8 -- a target height in multiples of 4, so the window never
/// reaches past the target.
fn ms_resolvable(gl: &GlesDraw) -> bool {
    gl.target_w & 15 == 0 && gl.target_h & 3 == 0
}

/// The factor a multisampled target's surface is scaled by (1 = none).
fn ms_scale(gl: &GlesDraw) -> u32 {
    if ms_via_scratch(gl) && GC_MSAA_RASTERISE {
        2
    } else {
        1
    }
}

/// Whether this draw renders into the scratch surface instead of the target.
fn ms_via_scratch(gl: &GlesDraw) -> bool {
    msaa_on() && gl.samples > 1 && ms_resolvable(gl)
}

/// The scratch surface's pitch in bytes: twice the target's width, padded to
/// a 64-pixel supertile.
/// The row pitch of a single-sampled target `w` pixels wide: whole pixel
/// pairs, as the GL core stores every colour surface (gl/context.rs
/// `tex_pitch`: FBO storage, read-back, the sampler's stride, and the pitch
/// egl.rs requires of a native window). The pixel engine was told `w * bpp`,
/// one pixel short a row at an odd width, so every row of a 127-, 111- or
/// 55-wide dEQP fbo.render NPOT target landed one pixel further left than
/// the one before (a shear), read back and sampled at the stored pitch.
fn target_pitch(w: u32, bpp: u32) -> u32 {
    (w.wrapping_add(1) & !1) * bpp
}

fn ms_pitch(w: u32, bpp: u32) -> u32 {
    align_up(w * 2, 64) * bpp
}

/// The scratch surface for a 4x draw: twice the target on each axis, padded
/// to 64x64 supertiles.
/// This backend draws into a framebuffer with no colour attachment
/// (`backend::DEPTH_ONLY_TARGETS`): `target_gpu` 0 names
/// [`ensure_no_colour_surface`]'s surface, which is never written.
pub const DRAWS_WITHOUT_COLOUR: bool = true;

/// Bytes of the surface a draw with no colour target names: Mesa's etnaviv
/// dummy render target, 64x64 RGBA8 (etnaviv_screen.c:1144-1151, Mesa 26.2.1).
const NO_COLOUR_SURFACE_BYTES: usize = 64 * 64 * 4;

/// The colour surface for a draw with no colour target (`target_gpu` 0, a
/// depth/stencil-only framebuffer; `backend::DEPTH_ONLY_TARGETS`). The pixel
/// engine is given its address with no components and a stride of 0, so it
/// writes none of it, as etnaviv does for nr_cbufs == 0 (etnaviv_state.c:
/// 397-407). Allocated once, at the first such draw.
fn ensure_no_colour_surface() -> i32 {
    let g = g();
    if !g.no_colour.impl_.is_null() {
        return 0;
    }
    osal::gpu_alloc(GC_HEAP_DATA, NO_COLOUR_SURFACE_BYTES, OSAL_MEM_DATA, &mut g.no_colour)
}

fn ensure_msaa(w: u32, h: u32, bpp: u32) -> i32 {
    let g = g();
    let bytes = ms_pitch(w, bpp) * align_up(h * 2, 64);

    if !g.msaa.impl_.is_null() && bytes <= g.msaa_bytes {
        return 0;
    }
    if !g.msaa.impl_.is_null() {
        osal::gpu_free(&mut g.msaa);
    }
    if osal::gpu_alloc(GC_HEAP_DATA, bytes as usize, OSAL_MEM_DATA, &mut g.msaa) != 0 {
        return -1;
    }
    g.msaa_bytes = bytes;
    0
}

/// The sample positions and centroid table a 4x draw needs, or the single
/// sampled state. The values are the blob's, by way of Mesa.
/// etnaviv's per-draw PS_MSAA_CONFIG for a multisampled draw (etnaviv_emit.c,
/// MIT): sample coverage on at 16/16, the sample mask enabled and all four
/// samples in it -- written, where the context word leaves every one of
/// these fields at the hardware's default. Only with gc_debug_msaa bit 2.
fn ps_msaa_config_written() -> u32 {
    let written = GC_PS_MSAA_SAMPLE_COVERAGE_MASK
        | GC_PS_MSAA_SAMPLE_COVERAGE_VALUE_MASK
        | GC_PS_MSAA_SAMPLE_COVERAGE_VALUE_FIELD
        | GC_PS_MSAA_SAMPLE_COVERAGE_INVERT
        | GC_PS_MSAA_SAMPLE_COVERAGE_INVERT_MASK
        | GC_PS_MSAA_SAMPLE_MASK_MASK
        | GC_PS_MSAA_SAMPLE_MASK_ENABLE_MASK
        | GC_PS_MSAA_SAMPLE_MASK_FIELD;
    (!(GC_PS_MSAA_ALPHA_TO_COVERAGE | GC_PS_MSAA_ALPHA_TO_COVERAGE_MASK) & !written)
        | GC_PS_MSAA_SAMPLE_COVERAGE
        | (16 << 20)
        | GC_PS_MSAA_SAMPLE_MASK_ENABLE
        | 0xf
}

fn emit_msaa_state(c: &mut GcCmd, gl: &GlesDraw) {
    const SAMPLES_4X: [u32; 3] = [0xeaa2_6e26, 0xe6ae_622a, 0xaaa2_2a22];
    const CENTROIDS_4X: [u32; 12] = [
        0x4a6e_2688,
        0x8888_88a2,
        0x8888_88ea,
        0x8888_88c6,
        0x4662_2a88,
        0x8888_88ae,
        0x8888_88e6,
        0x8888_88ca,
        0x262a_2288,
        0x8866_88a2,
        0x8888_66aa,
        0x6688_88a6,
    ];

    if ms_scale(gl) <= 1 {
        c.state(GC_GL_MULTI_SAMPLE_CONFIG, GC_MSAA_SAMPLES_NONE | msaa_enables(0xf));
        return;
    }
    c.state(GC_GL_MULTI_SAMPLE_CONFIG, GC_MSAA_SAMPLES_4X | msaa_enables(0xf));
    // SAFETY: as in msaa_on; a plain read.
    if unsafe { core::ptr::addr_of!(gc_debug_msaa).read() } & 4 != 0 {
        c.state(GC_PS_MSAA_CONFIG, ps_msaa_config_written());
    }
    c.state(GC_RA_MULTISAMPLE_UNK00E04, 0);
    for (i, w) in SAMPLES_4X.iter().enumerate() {
        c.state(ra_multisample_unk00e10(i as u32), *w);
    }
    for (i, w) in CENTROIDS_4X.iter().enumerate() {
        c.state(ra_centroid_table(i as u32), *w);
    }
}

/* ---- the default-framebuffer flip study, OFF in every build ------------ */

/// A debug switch for the gl_FragCoord flip study (tools/cts/RESULTS.md,
/// "option (c)"), set by `gc_debug_set_flip_experiment` and 0 unless a test
/// asks. Nothing in the product sets it.
///
/// 1, [`FLIP_STUDY_NEGATIVE_STRIDE`]: a top-down target (the default
/// framebuffer, `flip_y`) is drawn BOTTOM-UP instead of through a mirrored
/// viewport: the colour surface starts at its last row with a negative
/// stride, and the viewport, scissor, clear clip, cull and front face treat
/// it as the unflipped target an FBO is. If the pixel engine takes a negative
/// stride, the picture is upright AND gl_FragCoord.y, dFdy and point sprites
/// are GL's, for nothing.
///
/// 2, [`FLIP_STUDY_RS_COPY`]: every frame ends with 100 resolve-engine
/// copies of its target into the scratch surface with RS_CONFIG_FLIP set,
/// then one plain copy back -- so the target reads back mirrored, and the
/// frame's fence time over 100 copies is the cost of a flip at present.
#[no_mangle]
pub static mut gc_debug_flip_experiment: i32 = 0;

/// Set [`gc_debug_flip_experiment`] (gl_test's flip-study cases).
#[no_mangle]
pub extern "C" fn gc_debug_set_flip_experiment(mode: i32) {
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(gc_debug_flip_experiment).write(mode) }
}

const FLIP_STUDY_NEGATIVE_STRIDE: i32 = 1;
const FLIP_STUDY_RS_COPY: i32 = 2;
/// Copies per frame in the RS arm: enough that the fence time is theirs.
const FLIP_STUDY_COPIES: u32 = 100;

fn flip_mode() -> i32 {
    // SAFETY: as above; a plain read.
    unsafe { core::ptr::addr_of!(gc_debug_flip_experiment).read() }
}

/// Is this draw's y mirrored by the viewport? `flip_y`, unless the flip
/// study draws the target bottom-up instead.
pub(super) fn mirrors(flip_y: bool) -> bool {
    flip_y && flip_mode() != FLIP_STUDY_NEGATIVE_STRIDE
}

/// Does the colour surface run upwards from its last row?
fn negative_stride(flip_y: bool) -> bool {
    flip_y && flip_mode() == FLIP_STUDY_NEGATIVE_STRIDE
}

/// One resolve-engine copy, linear A8R8G8B8 to linear A8R8G8B8, `flip`
/// setting RS_CONFIG_FLIP (etnaviv state_3d.xml.h: VIVS_RS_CONFIG_FLIP).
fn emit_rs_copy(c: &mut GcCmd, src: u32, dst: u32, w: u32, h: u32, flip: bool) {
    let fmt = GC_RS_FORMAT_A8R8G8B8;
    c.state(GC_GL_FLUSH_CACHE, GC_FLUSH_COLOR | GC_FLUSH_DEPTH);
    c.stall(GC_SYNC_RA, GC_SYNC_PE);
    c.state(GC_RS_CONFIG, rs_source_format(fmt) | rs_dest_format(fmt) | if flip { GC_RS_CONFIG_FLIP } else { 0 });
    c.state(GC_RS_SOURCE_STRIDE, w * 4);
    c.state(GC_RS_DEST_STRIDE, w * 4);
    c.state(rs_pipe_source_addr(0), src);
    c.state(rs_pipe_dest_addr(0), dst);
    c.state(rs_pipe_offset(0), 0);
    c.state(rs_pipe_offset(1), 0);
    c.state(GC_RS_WINDOW_SIZE, rs_window_width(w) | rs_window_height(h));
    c.state(rs_dither(0), 0xffff_ffff);
    c.state(rs_dither(1), 0xffff_ffff);
    c.state(GC_RS_CLEAR_CONTROL, 0);
    for i in 0..4 {
        c.state(rs_fill_value(i), 0);
    }
    c.state(GC_RS_EXTRA_CONFIG, 0);
    c.state(GC_RS_SINGLE_BUFFER, GC_RS_SINGLE_BUFFER_ENABLE);
    c.state(GC_RS_KICKER, GC_RS_KICKER_VALUE);
    c.state(GC_RS_SINGLE_BUFFER, 0);
}

/// The RS arm: [`FLIP_STUDY_COPIES`] flipped copies of the frame's target
/// into the scratch surface, then one plain copy back. A width the resolve
/// engine refuses (not a multiple of 16) is left alone and logged.
fn rs_flip_study(c: &mut GcCmd) {
    let g = g();
    let (w, h, target) = (g.target_w, g.target_h, g.target_gpu as u32);
    if target == 0 {
        return; /* no colour target: nothing to copy */
    }
    if w == 0 || h == 0 || w % 16 != 0 {
        glog!("yakogl/vivante: flip study: {}x{} is not a resolve-engine size; no copies", w, h);
        return;
    }
    if ensure_msaa(w, h, 4) != 0 {
        glog!("yakogl/vivante: flip study: no scratch surface for {}x{}", w, h);
        return;
    }
    let scratch = g.msaa.gpu as u32;
    for _ in 0..FLIP_STUDY_COPIES {
        emit_rs_copy(c, target, scratch, w, h, true);
    }
    emit_rs_copy(c, scratch, target, w, h, false);
}

/// Average the supertiled scratch surface back into the linear target, 2x2
/// samples to a pixel (see GC_MSAA_ENABLE for how this follows etnaviv).
fn emit_resolve(c: &mut GcCmd) {
    let g = g();
    let src_pitch = ms_pitch(g.ms_dest_w, g.ms_dest_bpp);
    let dst_stride = g.ms_dest_stride;
    let fmt = if g.ms_dest_bpp == 2 { GC_RS_FORMAT_A4R4G4B4 } else { GC_RS_FORMAT_A8R8G8B8 };
    let (sw, sh) = (g.ms_dest_w * 2, g.ms_dest_h * 2);

    c.state(GC_GL_FLUSH_CACHE, GC_FLUSH_COLOR | GC_FLUSH_DEPTH);
    c.stall(GC_SYNC_RA, GC_SYNC_PE);

    c.state(
        GC_RS_CONFIG,
        rs_source_format(fmt)
            | rs_dest_format(fmt)
            | GC_RS_SOURCE_TILED
            | if GC_MSAA_RASTERISE { GC_RS_DOWNSAMPLE_X | GC_RS_DOWNSAMPLE_Y } else { 0 },
    );
    /* a tiled source's stride is given times four, with the supertile bits
     * (TS mode 0: this backend runs no tile status) */
    c.state(GC_RS_SOURCE_STRIDE, (src_pitch << 2) | GC_RS_SOURCE_STRIDE_TILING | GC_RS_SOURCE_STRIDE_SUPER_TILED_NEW);
    c.state(GC_RS_DEST_STRIDE, dst_stride);
    c.state(rs_pipe_source_addr(0), g.msaa.gpu as u32);
    c.state(rs_pipe_dest_addr(0), g.ms_dest_gpu as u32);
    c.state(rs_pipe_offset(0), 0);
    c.state(rs_pipe_offset(1), 0);
    /* in source samples */
    c.state(GC_RS_WINDOW_SIZE, rs_window_width(sw) | rs_window_height(sh));
    c.state(rs_dither(0), 0xffff_ffff); /* dithering off */
    c.state(rs_dither(1), 0xffff_ffff);
    c.state(GC_RS_CLEAR_CONTROL, 0); /* a copy, not a fill */
    for i in 0..4 {
        c.state(rs_fill_value(i), 0);
    }
    c.state(GC_RS_EXTRA_CONFIG, 0);
    c.state(GC_RS_SINGLE_BUFFER, GC_RS_SINGLE_BUFFER_ENABLE);
    c.state(GC_RS_KICKER, GC_RS_KICKER_VALUE);
    c.state(GC_RS_SINGLE_BUFFER, 0);
}

/// The depth canary: what the pixel engine actually writes, read back.
///
/// Whether a depth surface may be linear on this part, and whether the write
/// stays inside it, had been argued from Mesa's source and from which build
/// happened to survive. This answers it directly: fill the whole allocation --
/// surface and both guards -- with a pattern, run one depth-tested draw, and
/// look at what changed. Anything altered outside the surface is an overrun,
/// and where it starts and ends says how far.
#[cfg(feature = "vivante-depth-guard")]
mod canary {
    use super::*;

    const GC_DEPTH_CANARY: u32 = 0xa5a5_a5a5;

    #[no_mangle]
    pub extern "C" fn gc_depth_canary_fill() {
        let g = g();
        // SAFETY: the whole CPU mapping of the depth allocation, which no
        // kick is reading (the caller orchestrates that).
        let Some(bytes) = (unsafe { g.depth.bytes_mut() }) else { return };
        for w in bytes.as_chunks_mut::<4>().0 {
            *w = GC_DEPTH_CANARY.to_ne_bytes();
        }
        // SAFETY: as above.
        unsafe { osal::cache_flush(g.depth.cpu, g.depth.size) };
    }

    /// `first`/`last` are byte offsets of the changed range relative to the
    /// surface start, so a negative `first` means the write ran off the front;
    /// `outside` is how many bytes changed beyond either end of the surface.
    ///
    /// Returns 1 when something changed, 0 when the GPU touched none of it (a
    /// finding in itself, not a pass) and -1 when there is no depth
    /// allocation.
    ///
    /// # Safety
    /// The three out pointers must be writable.
    /// The word the depth surface mostly holds. After a clear on its own that
    /// is the clear value as the hardware stored it, which is how a board run
    /// says whether the clear reached memory at all: reading one pixel cannot
    /// tell "the clear wrote this" from "nothing wrote anything".
    ///
    /// Boyer-Moore: one pass to find the candidate, one to count it.
    /// Returns 1 when it really is a majority, 0 when there is none, -1 with
    /// no depth buffer.
    #[no_mangle]
    pub unsafe extern "C" fn gc_depth_modal_word(word: *mut u32) -> i32 {
        let g = g();
        let lo = (g.depth_surface_off / 4) as usize;
        let hi = lo + (g.depth_stride as usize * g.depth_h as usize) / 4;

        // SAFETY: the caller orchestrates that no kick is reading it.
        let Some(bytes) = (unsafe { g.depth.bytes_mut() }) else { return -1 };
        let words = bytes.as_chunks_mut::<4>().0;
        if hi <= lo || hi > words.len() {
            return -1;
        }
        // SAFETY: the whole CPU mapping, as the fill above.
        unsafe { osal::cache_invd(g.depth.cpu, g.depth.size) };

        let (mut cand, mut count) = (0u32, 0usize);
        for w in &words[lo..hi] {
            let v = u32::from_ne_bytes(*w);
            if count == 0 {
                cand = v;
                count = 1;
            } else if v == cand {
                count += 1;
            } else {
                count -= 1;
            }
        }
        let seen = words[lo..hi].iter().filter(|w| u32::from_ne_bytes(**w) == cand).count();
        if !word.is_null() {
            // SAFETY: the caller's out pointer.
            unsafe { *word = cand };
        }
        i32::from(seen * 2 > hi - lo)
    }

    #[no_mangle]
    pub unsafe extern "C" fn gc_depth_canary_scan(first: *mut i32, last: *mut i32, outside: *mut u32) -> i32 {
        let g = g();
        // SAFETY: as in the fill.
        let Some(bytes) = (unsafe { g.depth.bytes_mut() }) else { return -1 };
        if bytes.is_empty() {
            return -1;
        }
        // SAFETY: the whole mapping.
        unsafe { osal::cache_invd(g.depth.cpu, g.depth.size) };
        let surf_lo = g.depth_surface_off as usize;
        let surf_hi = surf_lo + g.depth_stride as usize * g.depth_h as usize;
        let (mut f, mut l, mut n, mut seen) = (0i32, 0i32, 0u32, false);

        for (i, w) in bytes.as_chunks::<4>().0.iter().enumerate() {
            if u32::from_ne_bytes(*w) == GC_DEPTH_CANARY {
                continue;
            }
            let off = i * 4;
            if !seen {
                f = off as i32 - g.depth_surface_off as i32;
                seen = true;
            }
            l = off as i32 - g.depth_surface_off as i32;
            if off < surf_lo || off >= surf_hi {
                n += 4;
            }
        }
        // SAFETY: the caller's contract.
        unsafe {
            first.write(f);
            last.write(l);
            outside.write(n);
        }
        i32::from(seen) /* 0: the GPU changed nothing at all */
    }
}

/* ---- fixed-function state ----------------------------------------------- */

/// `shc_blend_factor` (what gl/draw.rs resolved) -> PE_ALPHA_CONFIG codes.
fn blend_factor(f: u8) -> u32 {
    match f as u32 {
        SHC_BF_ONE => GC_BLEND_ONE,
        SHC_BF_SRC_COLOR => GC_BLEND_SRC_COLOR,
        SHC_BF_SRC_ALPHA => GC_BLEND_SRC_ALPHA,
        SHC_BF_DST_ALPHA => GC_BLEND_DST_ALPHA,
        SHC_BF_DST_COLOR => GC_BLEND_DST_COLOR,
        SHC_BF_SRC_ALPHA_SATURATE => GC_BLEND_SRC_ALPHA_SATURATE,
        SHC_BF_CONST_COLOR => GC_BLEND_CONSTANT_COLOR,
        SHC_BF_CONST_ALPHA => GC_BLEND_CONSTANT_ALPHA,
        SHC_BF_ZERO => GC_BLEND_ZERO,
        SHC_BF_INV_SRC_COLOR => GC_BLEND_ONE_MINUS_SRC_COLOR,
        SHC_BF_INV_SRC_ALPHA => GC_BLEND_ONE_MINUS_SRC_ALPHA,
        SHC_BF_INV_DST_ALPHA => GC_BLEND_ONE_MINUS_DST_ALPHA,
        SHC_BF_INV_DST_COLOR => GC_BLEND_ONE_MINUS_DST_COLOR,
        SHC_BF_INV_CONST_COLOR => GC_BLEND_ONE_MINUS_CONSTANT_COLOR,
        SHC_BF_INV_CONST_ALPHA => GC_BLEND_ONE_MINUS_CONSTANT_ALPHA,
        _ => GC_BLEND_ONE,
    }
}

/// `shc_blend_func` and `gles_cmp` already hold the hardware's codes
/// (add/subtract/reverse subtract/min/max, and never..always).
fn stencil_op(op: GlesStencilOp) -> u32 {
    match op {
        GLES_SOP_ZERO => GC_STENCIL_OP_ZERO,
        GLES_SOP_REPLACE => GC_STENCIL_OP_REPLACE,
        GLES_SOP_INCR_SAT => GC_STENCIL_OP_INCR,
        GLES_SOP_DECR_SAT => GC_STENCIL_OP_DECR,
        GLES_SOP_INVERT => GC_STENCIL_OP_INVERT,
        GLES_SOP_INCR_WRAP => GC_STENCIL_OP_INCR_WRAP,
        GLES_SOP_DECR_WRAP => GC_STENCIL_OP_DECR_WRAP,
        _ => GC_STENCIL_OP_KEEP,
    }
}

fn prim_type(p: GlesPrim) -> u32 {
    match p {
        GLES_PRIM_TRI_STRIP => GC_PRIM_TRIANGLE_STRIP,
        GLES_PRIM_TRI_FAN => GC_PRIM_TRIANGLE_FAN,
        GLES_PRIM_LINES => GC_PRIM_LINES,
        GLES_PRIM_LINE_STRIP => GC_PRIM_LINE_STRIP,
        GLES_PRIM_LINE_LOOP => GC_PRIM_LINE_LOOP,
        GLES_PRIM_POINTS => GC_PRIM_POINTS,
        _ => GC_PRIM_TRIANGLES,
    }
}

/// The clipping rectangle of a CLEAR quad: the whole target, narrowed by the
/// scissor only when the GL core asked for it (`clear_quad`).
///
/// A clear is not a draw. GL's glClear ignores the viewport altogether, and
/// the scissor box that applies to it is the one in force when glClear was
/// called -- not the one the draw it was fused into happens to have. The GL
/// core resolves both: a clear the background object cannot carry out arrives
/// as a `clear_quad` with its own scissor box, and any other clear arrives
/// with the scissor off and covers the target.
fn clear_clip_rect(d: &GlesDraw) -> [i32; 4] {
    let (w, h) = (d.target_w as i32, d.target_h as i32);
    if perf_tiny(d) || perf_exp(GC_PERF_EXP_CLIP_CLEARS) {
        return [0, 0, w.min(8), h.min(8)];
    }
    if !d.clear_quad || !d.scissor {
        return [0, 0, w, h];
    }
    let (sx0, sy0) = (d.scissor_rect[0], d.scissor_rect[1]);
    let (sx1, sy1) = (sx0.saturating_add(d.scissor_rect[2]), sy0.saturating_add(d.scissor_rect[3]));
    let (mut y0, mut y1) = (sy0, sy1);
    if mirrors(d.flip_y) {
        let t = h - sy1;
        y1 = h - sy0;
        y0 = t;
    }
    let x0 = sx0.clamp(0, w);
    let x1 = sx1.clamp(x0, w);
    let y0c = y0.clamp(0, h);
    let y1c = y1.clamp(y0c, h);
    [x0, y0c, x1, y1c]
}

/// The clipping rectangle: the viewport, narrowed by the scissor, clamped to
/// the target. GL's window origin is bottom-left; a top-down target (the
/// window surface) needs the y range mirrored, which is what `flip_y` says.
fn clip_rect(d: &GlesDraw) -> [i32; 4] {
    let (mut x0, mut y0) = (d.viewport[0], d.viewport[1]);
    let (mut x1, mut y1) = (x0 + d.viewport[2], y0 + d.viewport[3]);

    if d.scissor {
        let (sx0, sy0) = (d.scissor_rect[0], d.scissor_rect[1]);
        let (sx1, sy1) = (sx0 + d.scissor_rect[2], sy0 + d.scissor_rect[3]);

        x0 = x0.max(sx0);
        y0 = y0.max(sy0);
        x1 = x1.min(sx1);
        y1 = y1.min(sy1);
    }
    if mirrors(d.flip_y) {
        let h = d.target_h as i32;
        let t = h - y1;

        y1 = h - y0;
        y0 = t;
    }
    /* Every bound into the target, the lower ones as well: the SE takes
     * them as 16.16 (`x << 16` in a u32), so a box wholly right of or above
     * the target with x or y at 65536 or more wrapped -- 70000 became column
     * 4464, and 65546 would have been column 10, inside the target, for a
     * draw GL says is scissored away. Clamped, a box wholly outside is an
     * empty rectangle at the edge, which the clear's rectangle already was. */
    let (w, h) = (d.target_w as i32, d.target_h as i32);
    x0 = x0.clamp(0, w);
    y0 = y0.clamp(0, h);
    x1 = x1.clamp(x0, w);
    y1 = y1.clamp(y0, h);
    if perf_tiny(d) {
        x1 = x1.min(x0 + 8);
        y1 = y1.min(y0 + 8);
    }
    [x0, y0, x1, y1]
}

/* ---- the state block ---------------------------------------------------- */

fn emit_vertex_elements(c: &mut GcCmd, d: &GcDraw) {
    let n = (d.num_elems as usize).min(GC_MAX_VERTEX_ELEMENTS);
    let mut cfg0 = [0u32; GC_MAX_VERTEX_ELEMENTS];
    let mut cfg1 = [0u32; GC_MAX_VERTEX_ELEMENTS];
    let mut scale = [0u32; GC_MAX_VERTEX_ELEMENTS];

    for i in 0..n {
        let e = &d.elems[i];

        cfg0[i] = attrib_type(e.type_ as u32)
            | attrib_endian(0)
            | attrib_stream(e.stream as u32)
            | attrib_num(e.components as u32)
            | attrib_normalize(e.normalize as u32)
            | attrib_start(e.start as u32);
        cfg1[i] = attrib_end(e.end as u32) | if e.nonconsecutive { GC_ATTRIB_NONCONSECUTIVE } else { 0 };
        scale[i] = 1.0f32.to_bits(); /* float attributes: no extra scale */
    }
    c.state_multi(nfe_generic_attrib_config0(0), &cfg0[..n]);
    c.state_multi(nfe_generic_attrib_scale(0), &scale[..n]);
    c.state_multi(nfe_generic_attrib_config1(0), &cfg1[..n]);
}

fn emit_streams(c: &mut GcCmd, d: &GcDraw) {
    let n = (d.num_streams as usize).min(GC_MAX_VERTEX_ELEMENTS);

    for i in 0..n {
        c.state(nfe_vertex_streams_base_addr(i as u32), d.streams[i].gpu as u32);
    }
    for i in 0..n {
        c.state(nfe_vertex_streams_control(i as u32), vertex_stream_stride(d.streams[i].stride));
    }
    for i in 0..n {
        c.state(nfe_vertex_streams_vertex_divisor(i as u32), d.streams[i].divisor);
    }
}

/// Viewport, scissor and the rasteriser: registers 0x00a00..0x00e08.
fn emit_raster_state(c: &mut GcCmd, d: &GcDraw) {
    let gl = d.gl;
    /* A CLEAR IS NOT A DRAW, and this is where the difference is easiest to
     * lose. glClear is a full-screen quad here, written in clip space
     * (-1..1); the viewport transform maps clip space onto the viewport
     * rectangle. So putting a clear through the application's viewport
     * shrinks it to that rectangle and leaves the rest of the surface
     * standing -- which is what the board measured: a 32x32 viewport on a
     * 64x64 surface left 3072 of 4096 pixels untouched, first at (32,0)
     * (cts-runner, driver/tools/gclab/user/clear_viewport.c, both pairs,
     * glGetError 0).
     *
     * ES 2.0 3.7.3 lists what affects a clear -- the scissor test, the
     * colour mask and the write masks -- and the viewport is not among them,
     * which follows from what a viewport IS: it transforms vertices, and a
     * clear has none of its own. The quad is this backend's implementation
     * detail and must not become visible through it.
     *
     * The scissor still applies, and does so through the clipping rectangle
     * (`clear_clip_rect`), not through this transform. */
    let vp = if d.is_clear { [0, 0, gl.target_w as i32, gl.target_h as i32] } else { gl.viewport };
    let half_w = vp[2] as f32 * 0.5;
    let mut half_h = vp[3] as f32 * 0.5;
    let cx = vp[0] as f32 + half_w;
    let mut cy = vp[1] as f32 + half_h;
    /* A clear quad carries GL's window-space clear depth, which is already
     * in 0..1: putting it through the application's glDepthRangef as well
     * would map it twice. */
    let (near, far) = if d.is_clear { (0.0, 1.0) } else { (gl.depth_range[0], gl.depth_range[1]) };

    /* GL's bottom-left window origin against a top-down target */
    if mirrors(gl.flip_y) {
        cy = gl.target_h as f32 - cy;
        half_h = -half_h;
    }
    c.state_fixp(GC_PA_VIEWPORT_SCALE_X, f2fixp(half_w));
    c.state_fixp(GC_PA_VIEWPORT_SCALE_Y, f2fixp(half_h));
    /* The GPU maps clip z 0..1 to the depth range, GL maps -1..1: fold the
     * halving into the scale and the bias into the offset. */
    c.state(GC_PA_VIEWPORT_SCALE_Z, (far - near).to_bits());
    c.state_fixp(GC_PA_VIEWPORT_OFFSET_X, f2fixp(cx));
    c.state_fixp(GC_PA_VIEWPORT_OFFSET_Y, f2fixp(cy));
    c.state(GC_PA_VIEWPORT_OFFSET_Z, near.to_bits());

    c.state(GC_PA_LINE_WIDTH, (gl.line_width * 0.5).to_bits());
    c.state(GC_PA_POINT_SIZE, 0.5f32.to_bits());
    /* PROVOKING_VERTEX_LAST because GL's provoking vertex is the last one, and
     * a flat-shaded varying takes its value from that vertex. Mesa sets it
     * unconditionally for the same reason; this backend never did, so its
     * PA_SYSTEM_MODE read 0x10 against Mesa's 0x11 in the golden diff.
     *
     * Nothing in gl_test can see it today: GLSL ES 1.00 has no `flat`, and
     * none of the ES 3.00 shaders the suite uses declares one either -- the
     * sixteen flat inputs in the table are all Qt3D's. So this is a
     * correctness fix with no case behind it. */
    c.state(GC_PA_SYSTEM_MODE, GC_PA_SYSTEM_MODE_HALF_PIXEL_CENTER | GC_PA_SYSTEM_MODE_PROVOKING_VERTEX_LAST);

    c.state(GC_PA_ATTRIBUTE_ELEMENT_COUNT, pa_attribute_element_count_count(d.fs.viv.num_in as u32));

    let shade = if !d.is_clear && flat_shade_model(d.fs) {
        GC_PA_CONFIG_SHADE_MODEL_FLAT
    } else {
        GC_PA_CONFIG_SHADE_MODEL_SMOOTH
    };
    let mut pa_config = GC_PA_CONFIG_FILL_SOLID | shade | GC_PA_CONFIG_WIDE_LINE;
    if !d.is_clear {
        /*
         * Mirror the culled winding for a target that is NOT flipped.
         *
         * The rasteriser's screen space runs y down. The window surface is
         * top-down, so the viewport flip puts it in exactly that space and the
         * winding needs no correction. An FBO is not flipped: it keeps GL's
         * y-up, which is upside down to the rasteriser, and its winding comes
         * out reversed -- so it is the un-flipped target that needs the
         * mirror.
         *
         * This was here with the polarity the other way round, which made
         * gl_test case 4's first check cull a triangle that should have been
         * drawn. Removing it altogether fixed checks 1 to 4 and left check 5,
         * the same test against an FBO, still failing -- and the golden stream
         * says why: for identical GL calls into an FBO, Mesa emits
         * CULL_FACE_CCW where this backend emitted CW.
         *
         *     ours  PA_CONFIG 0x00412100   CULL_FACE_CW
         *     Mesa  PA_CONFIG 0x00412200   CULL_FACE_CCW
         */
        let cull = if mirrors(gl.flip_y) {
            gl.cull
        } else {
            match gl.cull {
                GLES_CULL_CW => GLES_CULL_CCW,
                GLES_CULL_CCW => GLES_CULL_CW,
                other => other,
            }
        };
        pa_config |= match cull {
            GLES_CULL_CW => GC_PA_CONFIG_CULL_FACE_CW,
            GLES_CULL_CCW => GC_PA_CONFIG_CULL_FACE_CCW,
            _ => GC_PA_CONFIG_CULL_FACE_OFF,
        };
        if d.vs.viv.vs_pointsize_out_reg != SHC_VIV_NO_REG {
            pa_config |= GC_PA_CONFIG_POINT_SIZE_ENABLE;
        }
        /* A point is rasterised as a sprite (ES always does) and the
         * rasteriser generates the coordinate only when the fragment shader
         * reads it: Mesa's etnaviv sets POINT_SPRITE_ENABLE from the
         * rasteriser state and clears it for a fragment shader with no point
         * coordinate input (etnaviv_rasterizer.c:51, etnaviv_shader.c:197-198). */
        if super::shader::reads_point_coord(d.fs) {
            pa_config |= GC_PA_CONFIG_POINT_SPRITE_ENABLE;
        }
    }
    c.state(GC_PA_CONFIG, pa_config);
    c.state(GC_PA_WIDE_LINE_WIDTH0, (gl.line_width * 0.5).to_bits());
    c.state(GC_PA_WIDE_LINE_WIDTH1, (gl.line_width * 0.5).to_bits());

    let clip = if d.is_clear { clear_clip_rect(gl) } else { clip_rect(gl) };
    c.state_fixp(GC_SE_SCISSOR_LEFT, (clip[0] as u32) << 16);
    c.state_fixp(GC_SE_SCISSOR_TOP, (clip[1] as u32) << 16);
    c.state_fixp(GC_SE_SCISSOR_RIGHT, ((clip[2] as u32) << 16) + GC_SE_SCISSOR_MARGIN_RIGHT);
    c.state_fixp(GC_SE_SCISSOR_BOTTOM, ((clip[3] as u32) << 16) + GC_SE_SCISSOR_MARGIN_BOTTOM);
    let offset = gl.polygon_offset && !d.is_clear;
    c.state(GC_SE_DEPTH_SCALE, if offset { gl.polygon_offset_factor } else { 0.0 }.to_bits());
    /* the units are scaled by the depth buffer's resolution, like Mesa's
     * depth_mrd for a 24-bit buffer */
    c.state(GC_SE_DEPTH_BIAS, if offset { gl.polygon_offset_units * 2.0 / 16777215.0 } else { 0.0 }.to_bits());
    c.state(GC_SE_CONFIG, 0);
    c.state_fixp(GC_SE_CLIP_RIGHT, ((clip[2] as u32) << 16) + GC_SE_CLIP_MARGIN);
    c.state_fixp(GC_SE_CLIP_BOTTOM, ((clip[3] as u32) << 16) + GC_SE_CLIP_MARGIN);
}

/// Depth, stencil, blending and the colour surface: registers
/// 0x01400..0x014bc.
fn emit_pe_state(c: &mut GcCmd, d: &GcDraw, depth_stride: u32, depth_addr: u32) {
    let gl = d.gl;
    let depth_test = if d.is_clear { true } else { gl.depth_test };
    /* GL 4.1.5: with the depth test off the buffer is not written, whatever
     * glDepthMask says. */
    let depth_write = if d.is_clear { d.clear_depth } else { gl.depth_test && gl.depth_write };
    let depth_func = if d.is_clear { GLES_CMP_ALWAYS } else { gl.depth_func };
    /* A colour format with no alpha (GL_RGB, GL_RGB565) reads alpha 1 (ES 2.0
     * 4.3.1), and its storage is RGBA8 here: a draw leaves the alpha byte as
     * the clear wrote it -- 1, the GL core rounds a clear's alpha to the
     * format's 0 bits -- instead of storing the fragment's. The PowerVR does
     * the same with the PBE's swizzle (`target_alpha_bits`). Without it the
     * fragment's alpha stayed in the byte and a GL_RGB texture rendered into
     * and sampled afterwards read it (dEQP fbo.render.depth.tex2d_rgb_*). */
    let no_alpha = if !d.is_clear && gl.target_alpha_bits == 0 { 0x7 } else { 0xf };
    /* ...and a clear of such a target writes its alpha (1) whatever the mask:
     * the channel is not there for GL, so a masked clear leaving the byte as
     * it was would only let an old value survive */
    let clear_alpha = if gl.target_alpha_bits == 0 { 0x8 } else { 0 };
    let color_mask = if d.is_clear {
        let m = clear_components(gl, d.clear_color);
        if m != 0 {
            m | clear_alpha
        } else {
            0
        }
    } else {
        gl.color_mask as u32 & no_alpha
    };
    /* A clear writes only the draw buffers `clear_target_mask` names
     * (glClearBuffer(GL_COLOR, n)): RT n's channels are its mask or none. */
    let rt_mask = |rt: usize| if d.is_clear && !clear_takes(gl, rt) { 0 } else { color_mask };
    let (format, ext_format) = pe_color_format(gl.target_format);
    /*
     * ONE / ZERO with ADD is what "no blending" computes, and Mesa's etnaviv
     * never programs the blender for it (etna_blend_state_create: alpha_enable
     * is false for exactly that combination). Nor should we: measured on
     * 2026-09-21, a draw with the blender enabled and those factors writes
     * nothing at all on this part, while the same draw with GL_BLEND off comes
     * out. Enabling the blender also costs the OVERWRITE hint below, so
     * treating it as disabled is both correct and faster.
     */
    let blend = !d.is_clear
        && gl.blend
        && !(gl.blend_src_rgb as u32 == SHC_BF_ONE
            && gl.blend_dst_rgb as u32 == SHC_BF_ZERO
            && gl.blend_eq_rgb as u32 == SHC_BLEND_ADD
            && gl.blend_src_alpha as u32 == SHC_BF_ONE
            && gl.blend_dst_alpha as u32 == SHC_BF_ZERO
            && gl.blend_eq_alpha as u32 == SHC_BLEND_ADD);

    /*
     * Depth: always a D24S8 buffer in memory, tested as GL asks, and
     * super-tiled.
     *
     * Nothing outside the pixel engine ever reads this buffer, so its layout
     * only has to be one the part supports and is told about consistently --
     * and Mesa describes every depth surface it makes as super-tiled.
     * `ensure_depth` pads to align(w,64) by align(h,64), which is exactly the
     * 64x64 a super-tile wants.
     *
     * Measured 2026-09-30 (gl_test 169, the surface's words dumped on the
     * i.MX8MP): with this PE_DEPTH_CONFIG, which never sets
     * GC_PE_DEPTH_SUPER_TILED, the PE writes the surface ROW-LINEAR at
     * `depth_stride`, neither super-tiled nor 4x4-tiled. Only the depth
     * resolve (ds_cache.rs `resolve_depth`) reads it on the CPU.
     */
    let mut pe_depth = GC_PE_DEPTH_MODE_Z
        | GC_PE_DEPTH_FORMAT_D24S8
        | GC_PE_DEPTH_UNK18
        | pe_depth_func(if depth_test { depth_func } else { GLES_CMP_ALWAYS })
        | if depth_write { GC_PE_DEPTH_WRITE_ENABLE } else { 0 };
    if !depth_test && !depth_write && !gl.stencil && !d.is_clear {
        pe_depth |= GC_PE_DEPTH_DISABLE_ZS;
    }
    c.state(GC_PE_DEPTH_CONFIG, pe_depth);
    c.state(GC_PE_DEPTH_NEAR, 0.0f32.to_bits());
    c.state(GC_PE_DEPTH_FAR, 1.0f32.to_bits());
    c.state(GC_PE_DEPTH_NORMALIZE, 16777215.0f32.to_bits()); /* 2^24 - 1 */
    c.state(GC_PE_DEPTH_STRIDE, depth_stride);

    /* stencil, front and back */
    let (pe_stencil_op, ref_front, ref_back, mut mask_front, mut mask_back, pe_stencil_cfg);
    let (cmp_front, cmp_back);
    if d.is_clear {
        pe_stencil_op = pe_stencil_func_front(GLES_CMP_ALWAYS)
            | pe_stencil_func_back(GLES_CMP_ALWAYS)
            | pe_stencil_pass_front(GC_STENCIL_OP_REPLACE)
            | pe_stencil_pass_back(GC_STENCIL_OP_REPLACE)
            | pe_stencil_fail_front(GC_STENCIL_OP_REPLACE)
            | pe_stencil_fail_back(GC_STENCIL_OP_REPLACE)
            | pe_stencil_depth_fail_front(GC_STENCIL_OP_REPLACE)
            | pe_stencil_depth_fail_back(GC_STENCIL_OP_REPLACE);
        ref_front = gl.clear_stencil as u32;
        ref_back = ref_front;
        /* GL: the stencil write mask says which bits a clear may change --
         * the mask of the glClear, which a clear fused into a draw carries
         * apart from the draw's own (gl_test 9, frame 5: glClear under 0xff,
         * glStencilMask(0xfe), draw; the clear used 0xfe, kept bit 0 of the
         * previous frame's stencil 1, and EQUAL 1 then drew everywhere). */
        mask_front =
            if gl.clear_stencil_write_mask != 0 { gl.clear_stencil_write_mask } else { gl.stencil_write_mask } as u32;
        mask_back = mask_front;
        pe_stencil_cfg = if d.clear_stencil { GC_PE_STENCIL_MODE_ONE_SIDED } else { GC_PE_STENCIL_MODE_DISABLED };
        cmp_front = 0xff;
        cmp_back = 0xff;
        if !d.clear_stencil {
            mask_front = 0; /* write nothing */
            mask_back = 0;
        }
    } else {
        /* Which half of the pixel engine's two-sided stencil state is
         * "front" is decided the same way the cull direction is: by
         * glFrontFace against the target's y flip. Sending GL's front state
         * to the PE's front half unconditionally gave a back-facing triangle
         * the front face's operation whenever exactly one of the two held. */
        let swap = gl.front_cw != mirrors(gl.flip_y);
        /* a generic pick, so every pair swaps the same way */
        fn pick<T>(swap: bool, front: T, back: T) -> (T, T) {
            if swap {
                (back, front)
            } else {
                (front, back)
            }
        }
        let (f_func, b_func) = pick(swap, gl.stencil_func, gl.stencil_back_func);
        let (f_sfail, b_sfail) = pick(swap, gl.stencil_sfail, gl.stencil_back_sfail);
        let (f_dpfail, b_dpfail) = pick(swap, gl.stencil_dpfail, gl.stencil_back_dpfail);
        let (f_dppass, b_dppass) = pick(swap, gl.stencil_dppass, gl.stencil_back_dppass);
        let (f_ref, b_ref) = pick(swap, gl.stencil_ref, gl.stencil_back_ref);
        let (f_wmask, b_wmask) = pick(swap, gl.stencil_write_mask, gl.stencil_back_write_mask);
        let (f_cmask, b_cmask) = pick(swap, gl.stencil_cmp_mask, gl.stencil_back_cmp_mask);

        pe_stencil_op = pe_stencil_func_front(f_func)
            | pe_stencil_func_back(b_func)
            | pe_stencil_pass_front(stencil_op(f_dppass))
            | pe_stencil_pass_back(stencil_op(b_dppass))
            | pe_stencil_fail_front(stencil_op(f_sfail))
            | pe_stencil_fail_back(stencil_op(b_sfail))
            | pe_stencil_depth_fail_front(stencil_op(f_dpfail))
            | pe_stencil_depth_fail_back(stencil_op(b_dpfail));
        ref_front = f_ref as u32;
        ref_back = b_ref as u32;
        mask_front = f_wmask as u32;
        mask_back = b_wmask as u32;
        cmp_front = f_cmask as u32;
        cmp_back = b_cmask as u32;
        pe_stencil_cfg = if gl.stencil { GC_PE_STENCIL_MODE_TWO_SIDED } else { GC_PE_STENCIL_MODE_DISABLED };
    }
    c.state(GC_PE_STENCIL_OP, pe_stencil_op);
    c.state(
        GC_PE_STENCIL_CONFIG,
        pe_stencil_cfg
            | pe_stencil_ref_front(ref_front)
            | pe_stencil_mask_front(cmp_front)
            | pe_stencil_write_mask_front(mask_front),
    );
    c.state(GC_PE_ALPHA_OP, 0); /* no alpha test in ES 2.0 */

    /* blending, and the constant colour in both byte and half-float form */
    c.state(
        GC_PE_ALPHA_BLEND_COLOR,
        pe_blend_color_r(unorm8(gl.blend_color[0]))
            | pe_blend_color_g(unorm8(gl.blend_color[1]))
            | pe_blend_color_b(unorm8(gl.blend_color[2]))
            | pe_blend_color_a(unorm8(gl.blend_color[3])),
    );
    /* A target whose format has no alpha reads destination alpha as 1 (ES 2.0
     * 4.3.1): the factors that read it are rewritten to the constant they
     * then are, so the stored byte never matters to a blend -- Mesa's
     * state tracker does the same for an RGB format emulated in RGBA
     * (st_atom_blend.c fix_xrgb_alpha). */
    let factor = |f: u8| {
        let x = blend_factor(f);
        if gl.target_alpha_bits != 0 {
            x
        } else if x == GC_BLEND_DST_ALPHA {
            GC_BLEND_ONE
        } else if x == GC_BLEND_ONE_MINUS_DST_ALPHA || x == GC_BLEND_SRC_ALPHA_SATURATE {
            GC_BLEND_ZERO
        } else {
            x
        }
    };
    let mut pe_alpha = 0;
    if blend {
        let separate = gl.blend_src_rgb != gl.blend_src_alpha
            || gl.blend_dst_rgb != gl.blend_dst_alpha
            || gl.blend_eq_rgb != gl.blend_eq_alpha;

        pe_alpha = GC_PE_ALPHA_BLEND_ENABLE_COLOR
            | if separate { GC_PE_ALPHA_BLEND_SEPARATE_ALPHA } else { 0 }
            | pe_alpha_src_func_color(factor(gl.blend_src_rgb))
            | pe_alpha_dst_func_color(factor(gl.blend_dst_rgb))
            | pe_alpha_eq_color(gl.blend_eq_rgb as u32)
            | pe_alpha_src_func_alpha(factor(gl.blend_src_alpha))
            | pe_alpha_dst_func_alpha(factor(gl.blend_dst_alpha))
            | pe_alpha_eq_alpha(gl.blend_eq_alpha as u32);
    }
    c.state(GC_PE_ALPHA_CONFIG, pe_alpha);

    /* the colour surface. OVERWRITE tells the PE the draw replaces whole
     * pixels, which it cannot do while blending or masking channels. */
    let mut pe_color = if ext_format {
        pe_color_format_format_ext(format) | GC_PE_COLOR_FORMAT_FORMAT_MASK
    } else {
        pe_color_format_format(format)
    } | pe_color_format_components(rt_mask(0));
    let ms = ms_via_scratch(gl);
    /* never OVERWRITE on a multisampled target (etnaviv_state.c) */
    if !blend && rt_mask(0) == 0xf && !ms {
        pe_color |= GC_PE_COLOR_FORMAT_OVERWRITE;
    }
    if ms {
        pe_color |= GC_PE_COLOR_FORMAT_SUPER_TILED | GC_PE_COLOR_FORMAT_SUPER_TILED_NEW;
    }
    c.state(GC_PE_COLOR_FORMAT, pe_color);
    let pitch = if ms {
        ms_pitch(gl.target_w, gles_surf_bytes_per_pixel(gl.target_format))
    } else {
        /* tex_pitch(w), or a view's wider pitch (draw_pitch) */
        crate::backend::draw_pitch(gl) * gles_surf_bytes_per_pixel(gl.target_format)
    };
    /* the flip study's arm (i): a top-down target drawn bottom-up, from its
     * last row with a negative stride (off unless gc_debug_flip_experiment) */
    let upward = negative_stride(gl.flip_y) && !ms_via_scratch(gl);
    let stride = if gl.target_gpu == 0 {
        0 /* no colour target: the dummy surface, never written */
    } else if upward {
        (pitch as i32).wrapping_neg() as u32
    } else {
        pitch
    };
    c.state(GC_PE_COLOR_STRIDE, stride);

    /* Render targets 1..3 (gc_debug_mrt): the 8-RT bank, index rt - 1, as
     * etnaviv programs it for a HALTI2+ core (etnaviv_state.c:250-420,
     * etnaviv_emit.c, etnaviv_blend.c:150-200). ES 3.0 has one colour mask
     * and one blend state for every draw buffer, so each target takes RT0's.
     * Linear, like RT0 here (no SUPER_TILED: RT_CONFIG has no linear bit).
     * RT_COUNT (PS_CONTROL, shader.rs) is what enables them; nothing is
     * written for a single-target draw. A slot with no target (`gpu` 0) is
     * skipped and the slots after it move down ([`hw_targets`]); the colour
     * mask is still the GL slot's, since glClearBuffer names draw buffers. */
    let (slots, n) = hw_targets(gl);
    for (rt, &slot) in slots[..n].iter().enumerate().skip(1) {
        let slot = slot as usize;
        let t = gl.targets[slot];
        let (fmt, _) = pe_color_format(t.format);
        let rt_pitch = target_pitch(t.w, gles_surf_bytes_per_pixel(t.format));
        c.state(pe_rt_addr_8_pipe(rt as u32, 0), t.gpu as u32);
        c.state(pe_rt_config_8(rt as u32), rt_config_stride(rt_pitch) | rt_config_format(fmt));
        c.state(
            pe_halti5_rt_colormask(rt as u32),
            pe_halti5_rt_colormask_components(rt_mask(slot))
                | if !blend && rt_mask(slot) == 0xf { GC_PE_HALTI5_RT_COLORMASK_OVERWRITE } else { 0 },
        );
        c.state(pe_halti5_rt_alpha_config(rt as u32), pe_alpha);
    }
    c.state(GC_PE_HDEPTH_CONTROL, 0); /* hierarchical depth off */
    /* one pixel pipe on this part: both pipe registers name the same surface */
    /* one pixel pipe on this part: both pipe registers name the same
     * surface, the scratch one while a multisampled draw rasterises */
    let color_gpu = if gl.target_gpu == 0 {
        g().no_colour.gpu as u32
    } else if ms_via_scratch(gl) {
        g().msaa.gpu as u32
    } else if upward {
        (gl.target_gpu + u64::from(gl.target_h.saturating_sub(1)) * u64::from(pitch)) as u32
    } else {
        gl.target_gpu as u32
    };
    c.state(pe_pipe_color_addr(0), color_gpu);
    c.state(pe_pipe_color_addr(1), color_gpu);
    c.state(pe_pipe_depth_addr(0), depth_addr);
    c.state(pe_pipe_depth_addr(1), depth_addr);

    c.state(GC_PE_STENCIL_CONFIG_EXT, pe_stencil_ref_back(ref_back));
    /* SINGLE_BUFFER(1) is what a linear colour surface needs; every surface
     * YakoGL renders to is linear. */
    let unk24 = if msaa_extras(gl) { pe_logic_op_unk24(5) } else { 0 };
    c.state(
        GC_PE_LOGIC_OP,
        GC_PE_LOGIC_OP_BASE | pe_logic_op_op(GC_LOGIC_OP_COPY) | pe_logic_op_single_buffer(1) | unk24,
    );
    c.state(pe_dither(0), 0xffff_ffff); /* dithering off */
    c.state(pe_dither(1), 0xffff_ffff);
    c.state(GC_PE_ALPHA_COLOR_EXT0, f2half(gl.blend_color[0]) | (f2half(gl.blend_color[1]) << 16));
    c.state(GC_PE_ALPHA_COLOR_EXT1, f2half(gl.blend_color[2]) | (f2half(gl.blend_color[3]) << 16));
    c.state(GC_PE_STENCIL_CONFIG_EXT2, pe_stencil_mask_back(cmp_back) | pe_stencil_write_mask_back(mask_back));
    c.state(GC_PE_MEM_CONFIG, 0); /* no tile status for either surface */
    c.state(GC_TS_MEM_CONFIG, 0);
}

/// The rasteriser's early-depth decision.
///
/// Early depth: never, here -- and it used to be enabled by halves.
///
/// Mesa drives both halves of early Z from one decision: the rasteriser's
/// RA_EARLY_DEPTH_TEST_ENABLE and the pixel engine's PE_DEPTH_CONFIG_EARLY_Z
/// both follow `early_z_test`, and that needs `early_z_allowed`, which
/// `etna_update_zsa` clears outright when the colour target is linear. Every
/// colour surface this backend renders to is linear, so the answer is always
/// no, and PE_DEPTH_CONFIG correctly never said otherwise. What the rasteriser
/// was told did not match: it got RA_EARLY_DEPTH_TEST_ENABLE whenever the
/// depth test was on and the shader neither discarded nor wrote depth. So the
/// two halves of the pipe disagreed about whether the depth test had already
/// happened, from the first draw that turned the depth test on -- which is
/// gl_test case 3, where a kick first fails to complete.
fn emit_ra_state(c: &mut GcCmd, d: &GcDraw) {
    let mut ra = GC_RA_EARLY_DEPTH_BASE | GC_RA_EARLY_DEPTH_FORWARD_Z | GC_RA_EARLY_DEPTH_FORWARD_W;

    /* the last varying decides how the rasteriser packs the varying stream */
    let n = d.fs.viv.num_in as usize;
    let last_2x = n > 0 && d.fs.viv.in_[n - 1].components <= 2;
    c.state(GC_RA_CONTROL, GC_RA_CONTROL_UNK0 | if last_2x { GC_RA_CONTROL_LAST_VARYING_2X } else { 0 });
    /*
     * So: no RA_EARLY_DEPTH_TEST_ENABLE, and WRITE_DISABLE always. Mesa gates
     * the write-disable on early_zs, which needs the same early_z_allowed a
     * linear colour target denies.
     */
    ra |= GC_RA_EARLY_DEPTH_WRITE_DISABLE;
    c.state(GC_RA_EARLY_DEPTH, ra);
}

/// Which caches must be written back before this draw's state lands. Emitting
/// the whole state block every draw means every one of them may have changed,
/// so the flush is unconditional; narrowing it is an optimisation that needs
/// hardware to verify.
fn emit_flushes(c: &mut GcCmd) {
    if perf_exp(GC_PERF_EXP_NO_DRAW_SYNC) {
        return;
    }
    c.state(GC_GL_FLUSH_CACHE, GC_FLUSH_COLOR | GC_FLUSH_DEPTH | GC_FLUSH_TEXTURE | GC_FLUSH_SHADER_L1);
    c.state(GC_GL_FLUSH_CACHE, GC_FLUSH_TEXTUREVS);
    c.stall(GC_SYNC_RA, GC_SYNC_PE);
    c.state(GC_TS_FLUSH_CACHE, GC_TS_FLUSH);
}

/* ---- one draw ----------------------------------------------------------- */

/// The depth surface's stride and base address are the backend's, not the
/// frame's, so `emit_draw` is handed them rather than reaching for the global
/// state while the frame is borrowed.
fn emit_draw(f: &mut GcFrame, d: &GcDraw, depth_stride: u32, depth_addr: u32) -> i32 {
    emit_flushes(&mut f.w);
    emit_vertex_elements(&mut f.w, d);

    if d.indexed {
        f.w.state(GC_FE_INDEX_STREAM_BASE_ADDR, d.index_gpu as u32);
        f.w.state(GC_FE_INDEX_STREAM_CONTROL, d.index_type);
        f.w.state(GC_FE_PRIMITIVE_RESTART_INDEX, 0);
    }
    emit_streams(&mut f.w, d);
    emit_raster_state(&mut f.w, d);
    emit_ra_state(&mut f.w, d);
    emit_msaa_state(&mut f.w, d.gl);
    emit_pe_state(&mut f.w, d, depth_stride, depth_addr);
    if !shader::emit_shader_state(&mut f.w, d) {
        return -1;
    }

    /* The shader and sampler state is not self-synchronising: hold the front
     * end until the pixel engine has finished the draw already in flight. */
    if !perf_exp(GC_PERF_EXP_NO_DRAW_SYNC) {
        f.w.stall(GC_SYNC_FE, GC_SYNC_PE);
    }
    if texture::emit_textures(f, d) != 0 {
        return -ENOMEM;
    }
    shader::emit_shader_code(&mut f.w, d);
    shader::emit_uniforms(&mut f.w, d);
    shader::emit_prefetch(&mut f.w);
    if !perf_exp(GC_PERF_EXP_NO_DRAW_SYNC) {
        f.w.stall(GC_SYNC_RA, GC_SYNC_PE);
    }

    f.w.draw_instanced(d.indexed, d.prim, d.instance_count.max(1), d.vertex_count, d.first);
    0
}

/// The state a fresh command stream has to establish before its first draw:
/// the 3D pipe, the constants the blob writes once, and the descriptor-based
/// texture unit. Mesa calls this the GPU state reset.
fn emit_context_init(c: &mut GcCmd) {
    let zero = [0u32; GC_NFE_GENERIC_ATTRIB_COUNT];

    c.state(GC_GL_PIPE_SELECT, GC_PIPE_3D);
    c.state(GC_GL_API_MODE, GC_API_MODE_OPENGL);
    c.state(GC_PA_W_CLIP_LIMIT, 0x3400_0001);
    c.state(GC_PA_FLAGS, 0);
    c.state(GC_PA_VIEWPORT_UNK00A80, 0x38a0_1404);
    c.state(GC_PA_VIEWPORT_UNK00A84, 8192.0f32.to_bits());
    c.state(GC_PA_ZFARCLIPPING, 0);
    c.state(GC_RA_HDEPTH_CONTROL, 0x0000_7000);
    c.state(GC_PS_CONTROL_EXT, 0);
    c.state(GC_VS_HALTI1_UNK00884, 0x0000_0808);
    c.state(GC_RA_UNK00E0C, 0);
    c.state(GC_PS_HALTI3_UNK0103C, 0x7654_3210);
    c.state(GC_PE_HALTI4_UNK014C0, 0);
    c.state(GC_NTE_DESCRIPTOR_CONTROL, GC_NTE_DESCRIPTOR_ENABLE);
    c.state(GC_FE_HALTI5_UNK007D8, 0x0000_0002);
    c.state(GC_PS_SAMPLER_BASE, 0);
    c.state(GC_SH_CONFIG, GC_SH_CONFIG_RTNE_ROUNDING);
    /* alpha to coverage off, every sample written */
    /* alpha to coverage off, every sample written. The five masks this used
     * to AND together were a transcription of the blob's word; only two bits
     * of it mean anything and the rest cleared bits Mesa leaves set. */
    c.state(GC_PS_MSAA_CONFIG, !(GC_PS_MSAA_ALPHA_TO_COVERAGE | GC_PS_MSAA_ALPHA_TO_COVERAGE_MASK));
    c.state(GC_GL_BUG_FIXES, 0x6);
    /* the descriptor cache holds nothing from an earlier stream */
    c.state(GC_NTE_DESCRIPTOR_FLUSH, 0);
    c.state(GC_GL_FLUSH_CACHE, GC_FLUSH_DESCRIPTOR);
    c.state(GC_VS_ICACHE_INVALIDATE, GC_VS_ICACHE_INVALIDATE_ALL);
    /* some parts come out of reset with attributes enabled; clearing the whole
     * array gives the front end the edge it needs to disable them */
    c.state_multi(nfe_generic_attrib_config0(0), &zero);
}

/* ---- frames ------------------------------------------------------------- */

fn frame_open(target_gpu: u64, w: u32, h: u32) -> i32 {
    let g = g();
    let cur = g.cur;
    let fence = g.frames[cur].fence;

    /* the frame's buffers are still in the GPU's hands until its fence has
     * completed; wait rather than overwrite a stream being fetched */
    if fence > 0 && completed_fence() < fence {
        g.waits += 1;
        if wait_fence(fence) != 0 {
            return -1;
        }
    }
    let f = &mut g.frames[cur];
    f.arena_used = 0;
    f.extra_used = 0;
    f.w.words = f.cmd.cpu as *mut u32;
    f.w.used = 0;
    /* The last GC_KICK_TAIL_BYTES of the buffer belong to the OSAL (it writes
     * a LINK there to run the stream), so they are not capacity. The
     * allocation is 64-byte aligned and the tail is 8, which leaves an even
     * capacity: that is what lets frame_submit's closing pad word always
     * fit. */
    if f.w.words.is_null() || f.cmd.size <= GC_KICK_TAIL_BYTES as usize {
        return -1;
    }
    f.w.cap = ((f.cmd.size - GC_KICK_TAIL_BYTES as usize) / 4) as u32;
    f.w.overflow = false;
    emit_context_init(&mut f.w);
    g.recording = true;
    frame_open_to_osal(true);
    g.target_gpu = target_gpu;
    g.target_w = w;
    g.target_h = h;
    0
}

/// Tell the OSAL whether a frame that may reference GPU memory is being
/// recorded (OSAL_QUERY_GC_FRAME_OPEN): without one it holds a free only for
/// the last submitted kick instead of the next one, which does not exist
/// until something draws (dEQP texture.size on the i.MX8MP ran out of the
/// carveout with 32 MiB held that way). An OSAL without the query keeps
/// holding for the next kick. The V3D's job_open_to_osal is the same rule.
fn frame_open_to_osal(open: bool) {
    if let Some(q) = osal::ops().and_then(|o| o.query) {
        let mut v = open as u32;
        // SAFETY: a u32 in-parameter of the size passed.
        let _ = unsafe { q(osal::OSAL_QUERY_GC_FRAME_OPEN, &mut v as *mut u32 as *mut c_void, 4) };
    }
}

fn frame_submit() -> i32 {
    let g = g();

    if !g.recording {
        return 0;
    }
    g.recording = false;
    /* closed on every path below: submitted, or lost with a message */
    frame_open_to_osal(false);
    let f = &mut g.frames[g.cur];
    if f.w.overflow {
        glog!("yakogl/vivante: command buffer full; the frame is truncated");
    }

    /* A multisampled frame rasterised into the scratch surface: average it
     * back into the target before the frame ends, while the stream is still
     * being written. */
    if g.ms_dest_gpu != 0 {
        emit_resolve(&mut f.w);
        g.ms_dest_gpu = 0;
    }
    if flip_mode() == FLIP_STUDY_RS_COPY {
        rs_flip_study(&mut g.frames[g.cur].w);
    }
    let f = &mut g.frames[g.cur];

    /* The stream ends where the backend's last command ends: the OSAL adds the
     * flushes and the EVENT that raise the fence (kick.rs). The front end
     * fetches 64-bit words, so hand it an even number of them. */
    f.w.pad();

    // SAFETY: the words written so far, and the arena bytes handed out, are
    // both inside their live mappings.
    unsafe {
        cache_flush_raw(f.cmd.cpu, f.w.used as usize * 4);
        if !f.arena.cpu.is_null() {
            cache_flush_raw(f.arena.cpu, f.arena_used as usize);
        }
        if !f.extra.cpu.is_null() && f.extra_used != 0 {
            cache_flush_raw(f.extra.cpu, f.extra_used as usize);
        }
    }

    // SAFETY: a plain add to a counter only this thread writes.
    unsafe { *core::ptr::addr_of_mut!(gc_perf_cmd_words) += f.w.used as u64 };
    let kick = GcKick { magic: GC_KICK_MAGIC, stream_bytes: f.w.used * 4, stream_gpu: f.cmd.gpu };
    let Some(submit) = osal::ops().and_then(|o| o.submit) else {
        return -1;
    };
    // SAFETY: `kick` is a live GcKick of the size passed.
    let fence = unsafe { submit(&kick as *const GcKick as *const c_void, core::mem::size_of::<GcKick>()) };
    if fence < 0 {
        glog!("yakogl/vivante: submit failed ({})", fence as i32);
        return -1;
    }
    f.fence = fence;
    g.kick_id += 1;
    g.frames[g.cur].kick_id = g.kick_id;
    g.kicks += 1;
    g.cur = (g.cur + 1) % GC_FRAMES;
    0
}

/* ---- the clear quad ----------------------------------------------------- */

/// glClear is a full-screen quad here: the tile-status fast clear is an
/// optimisation this backend leaves out, and a quad also lets one draw clear
/// colour, depth and stencil together with the state the GL asked for.
fn emit_clear(f: &mut GcFrame, gl: &GlesDraw, code: (u64, u64), depth: (u32, u32)) -> i32 {
    const CORNER: [[f32; 2]; 4] = [[-1.0, -1.0], [1.0, -1.0], [-1.0, 1.0], [1.0, 1.0]];
    /* GL's window-space clear depth 0..1, back through the viewport transform
     * into the clip space the quad is drawn in */
    let z = gl.clear_depth * 2.0 - 1.0;

    let Some((cpu, quad_gpu)) = f.arena_alloc(core::mem::size_of::<f32>() * 16, 16) else {
        return -1;
    };
    // SAFETY: the arena handed out 16 floats at `cpu`, 16-byte aligned.
    let quad: &mut [f32] = unsafe { core::slice::from_raw_parts_mut(cpu as *mut f32, 16) };
    for (i, corner) in CORNER.iter().enumerate() {
        quad[i * 4] = corner[0];
        quad[i * 4 + 1] = corner[1];
        quad[i * 4 + 2] = z;
        quad[i * 4 + 3] = 1.0;
    }
    let color = gl.clear_rgba;

    let mut d = GcDraw::new(gl, &GC_CLEAR_VS.0, &GC_CLEAR_PS.0);
    d.vs_code_gpu = code.0;
    d.fs_code_gpu = code.1;
    // SAFETY: `color` is four floats on this stack frame, which outlives the
    // emit below; the clear shader reads them as its four constant dwords.
    d.fs_uniforms = unsafe { core::slice::from_raw_parts(color.as_ptr() as *const u32, 4) };
    d.streams[0] = GcStream { gpu: quad_gpu, stride: 16, divisor: 0 };
    d.num_streams = 1;
    d.elems[0] = GcElem {
        stream: 0,
        type_: GC_DATA_TYPE_FLOAT as u8,
        components: 4,
        normalize: GC_NORMALIZE_OFF as u8,
        start: 0,
        end: 16,
        nonconsecutive: true,
        vs_reg: GC_CLEAR_VS.0.viv.in_[0].reg,
    };
    d.num_elems = 1;
    d.vertex_count = 4;
    d.instance_count = 1;
    d.prim = GC_PRIM_TRIANGLE_STRIP;
    d.is_clear = true;
    d.clear_color = gl.do_clear;
    d.clear_depth = gl.do_clear_depth;
    d.clear_stencil = gl.do_clear_stencil;

    emit_draw(f, &d, depth.0, depth.1)
}

/* ---- the backend entry points ------------------------------------------- */

fn completed_fence() -> i64 {
    match osal::ops().and_then(|o| o.completed) {
        // SAFETY: a plain query of the OSAL.
        Some(f) => unsafe { f() },
        None => i64::MAX, /* no way to ask: nothing is ever still pending */
    }
}

fn wait_fence(fence: i64) -> i32 {
    match osal::ops().and_then(|o| o.wait) {
        // SAFETY: a plain wait on a fence this backend was handed.
        Some(f) => unsafe { f(fence, -1) },
        None => -1,
    }
}

/// The instruction cache fetches a shader from memory, so the backend's own
/// two shaders are uploaded once, the way the GL core uploads a program's.
fn upload_shader(sh: &ShcShader, out: &mut OsalGpuMem) -> i32 {
    let bytes = sh.usc_dw as usize * 4;

    if osal::gpu_alloc(GC_HEAP_SHADER, bytes, OSAL_MEM_DATA, out) != 0 {
        return -1;
    }
    if !out.cpu.is_null() {
        // SAFETY: the allocation is `bytes` long and the shader's code is
        // `usc_dw` dwords.
        unsafe {
            core::ptr::copy_nonoverlapping(sh.usc as *const u8, out.cpu as *mut u8, bytes);
            cache_flush_raw(out.cpu, bytes);
        }
        return 0;
    }
    match osal::ops().and_then(|o| o.gpu_write) {
        // SAFETY: the shader's code is `bytes` long.
        Some(f) => unsafe { f(out, 0, sh.usc as *const c_void, bytes) },
        None => -1,
    }
}

/// Swap in whether GL_VARYING_COMPONENT_USE now holds a marked point
/// coordinate, and say whether it did before.
pub(crate) fn swap_component_use_marked(now: bool) -> bool {
    core::mem::replace(&mut g().component_use_marked, now)
}

/// Must this draw write GL_VARYING_COMPONENT_USE? When it marks a component
/// itself, and on the draw after one that did -- including the process's
/// first draw, unless [`gc_debug_first_component_use`] is 0 and nothing has
/// written the registers yet (read here, at draw time, so a Zephyr image can
/// set it after gc_init).
pub(crate) fn component_use_needs_write(marks: bool) -> bool {
    let was = swap_component_use_marked(marks);
    // SAFETY: as for gc_debug_msaa; a plain read.
    let first = unsafe { core::ptr::addr_of!(gc_debug_first_component_use).read() } != 0;
    let g = g();
    let write = marks || (was && (first || g.component_use_written));
    g.component_use_written |= write;
    write
}

pub extern "C" fn gc_init() -> i32 {
    let g = g();

    if g.inited {
        return 0;
    }
    *g = GcState::ZERO;
    /* Unknown until written: the first draw of the process writes
     * GL_VARYING_COMPONENT_USE GENERIC, whatever an earlier client left. */
    g.component_use_marked = true;
    g.component_use_written = false;
    for i in 0..GC_FRAMES {
        if osal::gpu_alloc(GC_HEAP_COMMAND, GC_CMD_WORDS * 4, OSAL_MEM_DATA, &mut g.frames[i].cmd) != 0
            || osal::gpu_alloc(GC_HEAP_COMMAND, GC_ARENA_BYTES, OSAL_MEM_DATA, &mut g.frames[i].arena) != 0
        {
            return -1;
        }
    }
    if upload_shader(&GC_CLEAR_VS.0, &mut g.clear_vs_code) != 0
        || upload_shader(&GC_CLEAR_PS.0, &mut g.clear_ps_code) != 0
    {
        return -1;
    }
    /* GL's incomplete texture: one opaque black RGBA8 texel (R=G=B=0, A=1) */
    if osal::gpu_alloc(GC_HEAP_SHADER, 4, OSAL_MEM_DATA, &mut g.black_tex) != 0 {
        return -1;
    }
    {
        let texel = 0xFF00_0000u32.to_le_bytes();
        if !g.black_tex.cpu.is_null() {
            // SAFETY: the allocation is four bytes long.
            unsafe {
                core::ptr::copy_nonoverlapping(texel.as_ptr(), g.black_tex.cpu as *mut u8, 4);
                cache_flush_raw(g.black_tex.cpu, 4);
            }
        } else if let Some(f) = osal::ops().and_then(|o| o.gpu_write) {
            // SAFETY: the texel is four bytes; the OSAL bounds the write.
            if unsafe { f(&mut g.black_tex, 0, texel.as_ptr() as *const c_void, 4) } != 0 {
                return -1;
            }
        } else {
            return -1;
        }
    }
    g.inited = true;
    0
}

pub extern "C" fn gc_shutdown() {
    let g = g();

    if !g.inited {
        return;
    }
    for i in 0..GC_FRAMES {
        osal::gpu_free(&mut g.frames[i].cmd);
        osal::gpu_free(&mut g.frames[i].arena);
        if !g.frames[i].extra.impl_.is_null() {
            osal::gpu_free(&mut g.frames[i].extra);
        }
    }
    super::tiled_copy::release_all();
    super::ds_cache::release_all();
    if !g.clear_vs_code.impl_.is_null() {
        osal::gpu_free(&mut g.clear_vs_code);
    }
    if !g.clear_ps_code.impl_.is_null() {
        osal::gpu_free(&mut g.clear_ps_code);
    }
    if !g.black_tex.impl_.is_null() {
        osal::gpu_free(&mut g.black_tex);
    }
    if !g.no_colour.impl_.is_null() {
        osal::gpu_free(&mut g.no_colour);
    }
    *g = GcState::ZERO;
}

/// The GPU address of the 1x1 opaque black texel GL's incomplete texture
/// samples as (0 before gc_init, which leaves the descriptor unbuilt).
pub(super) fn black_texel_gpu() -> u64 {
    g().black_tex.gpu
}

/// What one draw may take from the frame arena, rounded up: every client
/// array and a client index array copied at 64-byte alignment
/// ([`GcFrame::snapshot`]), plus a margin for the rest -- the uniform
/// snapshots, the texture descriptors and the clear quad. A frame whose arena
/// cannot take this much more is submitted before the draw rather than
/// failing in the middle of it: dEQP texture.vertex draws a 127 x 127 grid
/// from client arrays (16,384 vertices, 96,774 u16 indices, ~450 KB) several
/// times in one frame, and the second grid found the 1 MiB arena full and
/// was dropped -- the picture came back the clear colour (i.MX8MP,
/// 2085847f).
fn staging_need(gl: &GlesDraw) -> usize {
    const MARGIN: usize = 64 * 1024;
    let round = |b: u32| (b as usize + 63) & !63;
    let attribs: usize = gl.attribs[..(gl.num_attribs as usize).min(gl.attribs.len())]
        .iter()
        .filter(|a| !a.cpu.is_null())
        .map(|a| round(a.cpu_bytes))
        .sum();
    let index = if gl.index_cpu.is_null() { 0 } else { round(gl.index_bytes) };
    /* the two uniform blocks' copies for `load` (ubo_copy), taken whether the
     * program reads one or not: a few KiB at most */
    let ubo =
        round(u32::from(gl.vs_uniform_dwords) * 4 + UBO_TAIL) + round(u32::from(gl.fs_uniform_dwords) * 4 + UBO_TAIL);
    attribs + index + ubo + MARGIN
}

/// Bytes past a block's last dword that a `load` of its last element may
/// read: a load fetches four dwords whatever the element's width.
const UBO_TAIL: u32 = 16;

/// Does this stage read its uniform block from memory?
fn reads_ubo(sh: &ShcShader) -> bool {
    sh.viv.const_contents().contains(&crate::shaderc::SHC_VIV_CONST_UBO_ADDR)
}

/// Copy a stage's uniform block into the frame arena for `load`, with room
/// for the last element's four-dword fetch; its GPU address, or 0.
fn ubo_copy(f: &mut GcFrame, block: &[u32]) -> u64 {
    let bytes = block.len() * 4;
    let Some((cpu, gpu)) = f.arena_alloc(bytes + UBO_TAIL as usize, 64) else {
        return 0;
    };
    // SAFETY: the arena handed out `bytes + UBO_TAIL` bytes at `cpu`.
    unsafe {
        core::ptr::copy_nonoverlapping(block.as_ptr() as *const u8, cpu, bytes);
        core::ptr::write_bytes(cpu.add(bytes), 0, UBO_TAIL as usize);
    }
    gpu
}

fn attrib_data_type(fmt: u8) -> u8 {
    (match fmt {
        GLES_ATTRIBF_BYTE => GC_DATA_TYPE_BYTE,
        GLES_ATTRIBF_UBYTE => GC_DATA_TYPE_UNSIGNED_BYTE,
        GLES_ATTRIBF_SHORT => GC_DATA_TYPE_SHORT,
        GLES_ATTRIBF_USHORT => GC_DATA_TYPE_UNSIGNED_SHORT,
        GLES_ATTRIBF_FIXED => GC_DATA_TYPE_FIXED,
        _ => GC_DATA_TYPE_FLOAT,
    }) as u8
}

fn attrib_bytes(fmt: u8) -> u8 {
    match fmt {
        GLES_ATTRIBF_BYTE | GLES_ATTRIBF_UBYTE => 1,
        GLES_ATTRIBF_SHORT | GLES_ATTRIBF_USHORT => 2,
        _ => 4,
    }
}

/// One vertex stream per attribute. The hardware can fetch several attributes
/// out of one stream, which is what the NONCONSECUTIVE bit is for; giving each
/// its own stream costs a base address and keeps the offsets trivial.
fn build_vertex_input(f: &mut GcFrame, d: &mut GcDraw, gl: &GlesDraw) -> i32 {
    let n = (gl.num_attribs as usize).min(GC_MAX_VERTEX_ELEMENTS);

    for i in 0..n {
        let a = &gl.attribs[i];
        let size = a.components * attrib_bytes(a.format);
        let mut base = a.buffer_gpu;
        let mut off = a.offset;

        if !a.cpu.is_null() && a.cpu_bytes != 0 {
            // SAFETY: the GL core's staging holds `cpu_bytes` bytes at `cpu`.
            base = unsafe { f.snapshot(a.cpu, a.cpu_bytes) };
            off = 0;
            if base == 0 {
                return -1;
            }
        }
        d.streams[i] = GcStream { gpu: base + off as u64, stride: a.stride, divisor: u32::from(a.per_instance != 0) };
        d.elems[i] = GcElem {
            stream: i as u8,
            type_: attrib_data_type(a.format),
            components: a.components,
            normalize: if a.normalized != 0 { GC_NORMALIZE_SIGN_EXTEND as u8 } else { GC_NORMALIZE_OFF as u8 },
            start: 0,
            end: size,
            nonconsecutive: true,
            vs_reg: a.vi_reg,
        };
    }
    d.num_elems = n as u8;
    d.num_streams = n as u8;
    0
}

/// The uniform blocks of one draw. The constant registers are written as
/// command-stream words, not read from memory, so each stage's block is copied
/// to the CPU first (the C keeps the same two arrays as function statics).
static UNIFORM_BLOCKS: Global<[[u32; GC_MAX_UNIFORM_DWORDS]; 2]> = Global::zeroed();

/// Copy one stage's uniform block out of the draw. `vertex` picks the buffer.
///
/// # Safety
/// `cpu` must be null or hold `dwords` dwords.
unsafe fn take_uniforms(cpu: *const c_void, dwords: u16, vertex: bool) -> &'static [u32] {
    // SAFETY: single-threaded by contract; the two blocks are used only
    // between here and the end of the draw that asked for them.
    let blocks = unsafe { UNIFORM_BLOCKS.get() };
    let block = &mut blocks[usize::from(!vertex)];
    let n = (dwords as usize).min(GC_MAX_UNIFORM_DWORDS);

    if cpu.is_null() {
        /* the constant registers are command-stream words, so the block has
         * to be readable by the CPU */
        block[..n].fill(0);
        glog!("yakogl/vivante: {} uniforms have no CPU mapping", if vertex { "vertex" } else { "fragment" });
    } else {
        // SAFETY: the caller's contract.
        unsafe { core::ptr::copy_nonoverlapping(cpu as *const u32, block.as_mut_ptr(), n) };
    }
    &block[..n]
}

/// # Safety
/// `draw` is null or points at one resolved draw the GL core owns.
pub unsafe extern "C" fn gc_draw(draw: *const GlesDraw) -> i32 {
    // SAFETY: the caller's contract, passed on.
    let rc = unsafe { draw_one(draw) };
    // SAFETY: a plain add to an int only this thread writes.
    unsafe { *core::ptr::addr_of_mut!(PERF_FRAME_DRAWS) += 1 };
    if rc != 0 && g().inited {
        /* Never silent: every refused draw is counted (`gc_refused_draws`),
         * and the first of each return code is logged. dEQP texture.vertex
         * drew nothing for a whole group with no line anywhere (the frame
         * arena, 2085847f). */
        let g = g();
        g.refused = g.refused.wrapping_add(1);
        if !g.refused_logged[..g.refused_nlogged].contains(&rc) {
            if g.refused_nlogged < g.refused_logged.len() {
                g.refused_logged[g.refused_nlogged] = rc;
                g.refused_nlogged += 1;
            }
            glog!("yakogl/vivante: draw refused ({rc}); {} refused since init", g.refused);
        }
    }
    rc
}

/// Draws the backend refused since init, every one (the V3D's
/// `v3d_refused_draws`); a run that reads non-zero here dropped geometry.
#[no_mangle]
pub extern "C" fn gc_refused_draws() -> u32 {
    g().refused
}

/// [`gc_draw`]'s body.
///
/// # Safety
/// As for [`gc_draw`].
unsafe fn draw_one(draw: *const GlesDraw) -> i32 {
    // SAFETY: the caller's contract.
    let Some(gl) = (unsafe { draw.as_ref() }) else {
        return -1;
    };
    if !g().inited {
        return -1;
    }
    /* one instance of an instanced draw the GL core draws instance by
     * instance: the FE starts the instance ID at 0 for every draw, so a
     * program that reads gl_InstanceID is refused (counted by gc_draw) */
    if gl.instance_id != 0 && !gl.vs.is_null() && unsafe { (*gl.vs).vs_instance_id_reg1 } != 0 {
        return -crate::util::EINVAL;
    }
    /* glDrawBuffers({GL_NONE, GL_COLOR_ATTACHMENT1}) (ES 3.0 4.2.1): hardware
     * target 0 is always slot 0 ([`hw_targets`]), so the draw would write
     * attachment 0. Refused by name, never drawn into the wrong image
     * (review R-09); counted and logged by gc_draw too. */
    if gl.num_targets > 1 && gl.targets[0].gpu == 0 {
        let g = g();
        if !g.refused_draw_buffer0_logged {
            g.refused_draw_buffer0_logged = true;
            glog!("yakogl/vivante: draw refused: draw buffer 0 is GL_NONE while a later draw buffer is written");
        }
        return -crate::util::ENOTSUP;
    }
    /* ES 3.0 4.4.4.2 lets colour attachments differ in size; the render
     * area is then their intersection. This core clips once, to RT 0's size
     * (clip_rect: one SE scissor and clip for every target), so a later
     * target smaller than RT 0 would be written past its storage. Refused by
     * name (review R-02); a target at least as large as RT 0 is written
     * inside it. (Clipping to the intersection through that one rectangle is
     * possible; it is not done here.) */
    let (slots, n) = hw_targets(gl);
    if slots[1..n].iter().any(|&s| {
        let t = gl.targets[s as usize];
        t.w < gl.target_w || t.h < gl.target_h
    }) {
        let g = g();
        if !g.refused_sizes_logged {
            g.refused_sizes_logged = true;
            glog!("yakogl/vivante: draw refused: a colour attachment past the first is smaller than the first");
        }
        return -crate::util::ENOTSUP;
    }
    let ms = ms_scale(gl);
    if msaa_on() && gl.samples > 1 && ms == 1 {
        glog!(
            "yakogl/vivante: a {}x{} target is not a resolve size (width a multiple of 16, height of 4); drawing single-sampled",
            gl.target_w,
            gl.target_h
        );
    }
    /* the depth/stencil surface this draw tests and writes (ds_cache.rs),
     * chosen first: moving a plane into it may submit the open frame, which
     * resolves and forgets the multisampled target set up below */
    let key = super::ds_cache::key_of(gl, ms);
    /* sized for the larger of the storage GL gave and the target: ES 3.0 lets
     * them differ, and the pixel engine addresses it in target pixels */
    let (ds_w, ds_h) = (gl.ds_w.max(gl.target_w), gl.ds_h.max(gl.target_h));
    let selected = match super::ds_cache::select(
        key,
        ds_w * ms,
        ds_h * ms,
        GC_DEPTH_GUARD_BYTES,
        gc_kick_id(),
        |k| gc_kick_pending(k),
        || gc_finish(),
    ) {
        Ok(s) => s,
        Err(rc) => return rc,
    };
    if gl.target_gpu == 0 && ensure_no_colour_surface() != 0 {
        return -ENOMEM;
    }
    if ms_via_scratch(gl) {
        let bpp = gles_surf_bytes_per_pixel(gl.target_format);

        if ensure_msaa(gl.target_w, gl.target_h, bpp) != 0 {
            return -ENOMEM;
        }
        let g = g();
        g.ms_dest_gpu = gl.target_gpu;
        g.ms_dest_w = gl.target_w;
        g.ms_dest_h = gl.target_h;
        g.ms_dest_bpp = bpp;
        g.ms_dest_stride = crate::backend::draw_pitch(gl) * bpp;
    }
    #[cfg(feature = "vivante-depth-guard")]
    if selected.fresh {
        /* Lay the canary down as soon as the buffer exists, so everything the
         * pixel engine does to it afterwards is visible without the caller
         * having to orchestrate a draw around the fill. */
        canary::gc_depth_canary_fill();
    }

    /* A mipmapped layered texture is sampled from a tiled copy
     * (tiled_copy.rs), made or refreshed now, before this draw records
     * anything: a render into the texture still in flight is finished first.
     * Without its copy the draw would read the wrong faces, so it is refused. */
    if super::tiled_copy::prepare(gl, |k| gc_kick_pending(k), || gc_finish()) != 0 {
        return -1;
    }

    /* a new target, a stream with no room left, or an arena that cannot take
     * this draw's copies ends the frame */
    let need = staging_need(gl);
    /* A draw whose copies are larger than the arena starts a frame of its
     * own with memory of its own for them (`ensure_extra`), instead of being
     * refused: dEQP texture.vertex.2d draws 127x127 quads, four vertices
     * each, from client arrays -- 1,225,804 bytes -- and every one of its 45
     * cases came back black on the i.MX8MP because the draw was refused with
     * GL_OUT_OF_MEMORY (gl_test 129's grid pass, check 12, ed71ce69). */
    let big = need > GC_ARENA_BYTES;
    {
        let g = g();
        let full = big || g.frames[g.cur].arena_used as usize + need > GC_ARENA_BYTES;
        if g.recording && (gl.target_gpu != g.target_gpu || g.frames[g.cur].w.overflow || full) {
            frame_submit();
        }
    }
    if !g().recording && frame_open(gl.target_gpu, gl.target_w, gl.target_h) != 0 {
        return -1;
    }
    if big && ensure_extra(need) != 0 {
        return -ENOMEM;
    }

    /* The stream has no room check before a draw, and a draw's size is not
     * known until it is written (its state, uniforms and descriptors), so a
     * draw that runs out of room is rolled back and recorded again at the
     * start of a new frame. Before this the writer only noted the overflow:
     * the draw was lost, its last LOAD_STATE could end without its payload
     * right before the kick's tail, and nothing counted it. dEQP
     * color_clear.long_masked_rgba (100 to 500 masked, scissored clears in
     * one frame) is the shape that reaches it. */
    /* A plane that comes back from a whole clear's value only (ds_cache.rs)
     * is written back by a clear of just that plane, before the draw: the
     * surface holds nothing of it yet. Recorded with the draw, so a draw
     * recorded again in a new frame records it again (it is idempotent). */
    let restore = (selected.restore != [None, None]).then(|| GlesDraw {
        index_count: 0,
        do_clear: false,
        clear_quad: false,
        scissor: false,
        do_clear_depth: selected.restore[0].is_some(),
        clear_depth: selected.restore[0].map_or(1.0, f32::from_bits),
        do_clear_stencil: selected.restore[1].is_some(),
        clear_stencil: selected.restore[1].map_or(0, |v| v as u8),
        clear_stencil_write_mask: 0xff,
        ..*gl
    });
    let record_all = || {
        // SAFETY: the caller's contract, passed on; `restore` is a copy of
        // the draw with only its clear fields changed.
        let rc = restore.as_ref().map_or(0, |r| unsafe { record(r) });
        // SAFETY: as above.
        if rc != 0 {
            rc
        } else {
            unsafe { record(gl) }
        }
    };
    let mark = Mark::take();
    let mut rc = record_all();
    if g().frames[g().cur].w.overflow {
        mark.restore();
        frame_submit();
        if frame_open(gl.target_gpu, gl.target_w, gl.target_h) != 0 {
            return -1;
        }
        if big && ensure_extra(need) != 0 {
            return -ENOMEM;
        }
        mark.restore_ms_dest();
        let again = Mark::take();
        rc = record_all();
        if g().frames[g().cur].w.overflow {
            again.restore();
            glog!("yakogl/vivante: a draw does not fit an empty command buffer ({} words)", GC_CMD_WORDS);
            return -ENOMEM;
        }
    }
    if rc == 0 {
        /* what the draw left in the planes: a surface with a live one is
         * never given to another pair (ds_cache.rs) */
        super::ds_cache::drawn(key, ds_retention::draw_writes(gl), ds_retention::draw_clears(gl));
    }
    rc
}

/// The GL core says the depth/stencil contents of object `id` are gone
/// (`YakoglBackend::attachment_gone`): its planes are undefined in every
/// surface, and a surface left holding nothing is freed (ds_cache.rs).
pub extern "C" fn gc_attachment_gone(id: u32) {
    if !g().inited || id == 0 {
        return;
    }
    super::ds_cache::gone(id, |k| gc_kick_pending(k));
}

/// Write the depth plane of GL object `id` into a depth texture's texels
/// (`YakoglBackend::depth_resolve`; ds_cache.rs `resolve_depth`).
///
/// # Safety
/// `dst` holds `h` rows of `pitch` floats, `w <= pitch`.
pub unsafe extern "C" fn gc_depth_resolve(id: u32, dst: *mut f32, w: u32, h: u32, pitch: u32) -> i32 {
    if !g().inited {
        return 1;
    }
    // SAFETY: the caller's contract.
    unsafe { super::ds_cache::resolve_depth(id, dst, w, h, pitch, GC_DEPTH_GUARD_BYTES, |k| gc_kick_pending(k)) }
}

/// The open frame's `extra` block holds at least `need` bytes: a draw's copies
/// that the arena cannot take (see `draw_one`). The frame's fence has
/// completed (frame_open waited for it), so an older block is no longer read
/// and can be replaced. -ENOMEM, logged, when the pool has no such block.
fn ensure_extra(need: usize) -> i32 {
    let g = g();
    let f = &mut g.frames[g.cur];
    /* `need` already rounds each copy to its 64-byte start and has room to
     * spare (`staging_need`) */
    let want = need;
    if f.extra.size >= want {
        return 0;
    }
    if !f.extra.impl_.is_null() {
        osal::gpu_free(&mut f.extra);
    }
    f.extra_used = 0;
    if osal::gpu_alloc(GC_HEAP_COMMAND, want, OSAL_MEM_DATA, &mut f.extra) != 0 {
        f.extra = OsalGpuMem::ZERO;
        glog!("vivante: no {want} bytes for a draw's client arrays and indices; the draw is refused");
        return -ENOMEM;
    }
    0
}

/// Where the frame being recorded stood before a draw, and the backend state
/// a draw changes as it records: enough to take a draw back out of the frame.
struct Mark {
    cur: usize,
    used: u32,
    arena_used: u32,
    extra_used: u32,
    component_use_marked: bool,
    component_use_written: bool,
    ms_dest: (u64, u32, u32, u32, u32),
}

impl Mark {
    fn take() -> Mark {
        let g = g();
        let f = &g.frames[g.cur];
        Mark {
            cur: g.cur,
            used: f.w.used,
            arena_used: f.arena_used,
            extra_used: f.extra_used,
            component_use_marked: g.component_use_marked,
            component_use_written: g.component_use_written,
            ms_dest: (g.ms_dest_gpu, g.ms_dest_w, g.ms_dest_h, g.ms_dest_bpp, g.ms_dest_stride),
        }
    }

    /// Take the draw back out: the frame ends where it did before it.
    fn restore(&self) {
        let g = g();
        let f = &mut g.frames[self.cur];
        f.w.used = self.used;
        f.w.overflow = false;
        f.arena_used = self.arena_used;
        f.extra_used = self.extra_used;
        g.component_use_marked = self.component_use_marked;
        g.component_use_written = self.component_use_written;
    }

    /// Submitting the frame resolved and forgot the multisampled target;
    /// the draw about to be recorded again still renders into it.
    fn restore_ms_dest(&self) {
        let g = g();
        (g.ms_dest_gpu, g.ms_dest_w, g.ms_dest_h, g.ms_dest_bpp, g.ms_dest_stride) = self.ms_dest;
    }
}

/// Record one draw into the open frame: its clear, then its geometry.
///
/// # Safety
/// `draw_one`'s contract; a frame is open.
unsafe fn record(gl: &GlesDraw) -> i32 {
    let g = g();
    let code = (g.clear_vs_code.gpu, g.clear_ps_code.gpu);
    let depth = (g.depth_stride, g.depth.gpu as u32 + g.depth_surface_off);
    let cur = g.cur;

    if (gl.do_clear || gl.do_clear_depth || gl.do_clear_stencil) && emit_clear(&mut g.frames[cur], gl, code, depth) != 0
    {
        return -1;
    }
    if gl.index_count == 0 {
        return 0; /* a clear-only kick has no geometry */
    }
    // SAFETY: the GL core hands a draw whose vs/fs are live compiled shaders.
    let (Some(vs), Some(fs)) = (unsafe { gl.vs.as_ref() }, unsafe { gl.fs.as_ref() }) else {
        return -1;
    };
    let mut d = GcDraw::new(gl, vs, fs);
    d.vs_code_gpu = gl.vs_usc_gpu;
    d.fs_code_gpu = gl.fs_usc_gpu;
    d.textures = &gl.textures[..(gl.num_textures as usize).min(gl.textures.len())];
    d.vs_textures = &gl.vs_textures[..(gl.num_vs_textures as usize).min(gl.vs_textures.len())];
    d.prim = prim_type(gl.prim);
    d.vertex_count = gl.index_count;
    d.instance_count = gl.instance_count.max(1);

    if gl.vs_uniform_dwords != 0 {
        // SAFETY: the GL core's block holds `vs_uniform_dwords` dwords.
        d.vs_uniforms = unsafe { take_uniforms(gl.vs_uniform_cpu, gl.vs_uniform_dwords, true) };
    }
    if gl.fs_uniform_dwords != 0 {
        // SAFETY: as above.
        d.fs_uniforms = unsafe { take_uniforms(gl.fs_uniform_cpu, gl.fs_uniform_dwords, false) };
    }

    let f = &mut g.frames[cur];
    if build_vertex_input(f, &mut d, gl) != 0 {
        return -1;
    }
    /* a stage that reads a uniform array at a run-time index loads it from a
     * copy of its block in memory (select.rs LoadUniformIdx) */
    if reads_ubo(vs) {
        d.vs_ubo_gpu = ubo_copy(f, d.vs_uniforms);
        if d.vs_ubo_gpu == 0 {
            return -1;
        }
    }
    if reads_ubo(fs) {
        d.fs_ubo_gpu = ubo_copy(f, d.fs_uniforms);
        if d.fs_ubo_gpu == 0 {
            return -1;
        }
    }

    if gl.index_buf_gpu != 0 || !gl.index_cpu.is_null() {
        d.indexed = true;
        d.index_type = match gl.index_size {
            1 => GC_INDEX_TYPE_U8,
            4 => GC_INDEX_TYPE_U32,
            _ => GC_INDEX_TYPE_U16,
        };
        d.index_gpu = gl.index_buf_gpu;
        if !gl.index_cpu.is_null() && gl.index_bytes != 0 {
            // SAFETY: the GL core's staging holds `index_bytes` bytes.
            d.index_gpu = unsafe { f.snapshot(gl.index_cpu, gl.index_bytes) };
            if d.index_gpu == 0 {
                return -1;
            }
        }
    }

    emit_draw(f, &d, depth.0, depth.1)
}

pub extern "C" fn gc_flush() -> i32 {
    frame_submit()
}

fn wait_frame(i: usize) -> i32 {
    let g = g();
    let fence = g.frames[i].fence;

    if fence <= 0 || completed_fence() >= fence {
        return 0;
    }
    g.waits += 1;
    wait_fence(fence)
}

pub extern "C" fn gc_finish() -> i32 {
    if frame_submit() != 0 {
        return -1;
    }
    for i in 0..GC_FRAMES {
        let rc = wait_frame(i);
        if rc != 0 {
            return rc;
        }
    }
    0
}

/// What a pipelined swap waits for: every kick submitted before the previous
/// swap -- the frame before the one just swapped -- so the caller may show
/// that frame's buffer while this one renders (the contract of
/// `YakoglBackend::finish_previous`, and what the PowerVR backend does).
///
/// This used to wait for the slot before `cur`, which with two slots is the
/// kick JUST submitted: every swap was a glFinish and the CPU never recorded
/// a frame while the GPU drew one. Measured 2026-10-01 on the Verdin (Slint
/// cluster, 1280x800): 68 ms a frame = 12.6 ms recording + 47.8 ms waiting
/// for the GPU + 7.7 ms for the flip, in series.
pub extern "C" fn gc_finish_previous() -> i32 {
    // SAFETY: a plain store to an int only this thread writes.
    unsafe { core::ptr::addr_of_mut!(PERF_FRAME_DRAWS).write(0) };
    let g = g();
    let newest = g.frames[(g.cur + GC_FRAMES - 1) % GC_FRAMES].fence;
    let prev = core::mem::replace(&mut g.swap_fence, newest);

    if prev <= 0 || completed_fence() >= prev {
        return 0;
    }
    g.waits += 1;
    wait_fence(prev)
}

/// The kick being recorded: the id the frame now open will be submitted
/// under (`frame_submit` numbers them 1, 2, ...). The GL core stamps what a
/// draw reads and writes with it (`last_use`, `texture_target_use`, the
/// staging blocks) and a fence sync takes `kick_id() - 1` as the last kick
/// submitted, the PowerVR backend's contract. Answering the last SUBMITTED
/// kick instead stamped everything the open frame used with the frame
/// before it: once that one completed, glTexSubImage2D wrote the texels in
/// place under a draw not yet submitted, and a fence signalled one kick
/// early.
pub extern "C" fn gc_kick_id() -> u32 {
    g().kick_id.wrapping_add(1)
}

/// Is kick `kick_id` still to complete? The kick being recorded is, while
/// the frame is open: it has not even been submitted.
pub extern "C" fn gc_kick_pending(kick_id: u32) -> bool {
    let g = g();

    if kick_id == 0 {
        return false;
    }
    if kick_id == g.kick_id.wrapping_add(1) {
        return g.recording;
    }
    if kick_id > g.kick_id {
        return false;
    }
    for f in g.frames.iter() {
        if f.kick_id != kick_id || f.fence <= 0 {
            continue;
        }
        return completed_fence() < f.fence;
    }
    false /* too old to still be in flight */
}

pub extern "C" fn gc_heap_base(heap: GlesHeap) -> u64 {
    match heap {
        GLES_HEAP_SHADER | GLES_HEAP_PROGRAM => GC_HEAP_SHADER,
        _ => GC_HEAP_DATA,
    }
}

/// # Safety
/// Both pointers are null or writable.
pub unsafe extern "C" fn gc_counters(kicks: *mut u32, waits: *mut u32) {
    let g = g();

    // SAFETY: the caller's contract.
    unsafe {
        if !kicks.is_null() {
            kicks.write(g.kicks);
        }
        if !waits.is_null() {
            waits.write(g.waits);
        }
    }
}

/// The Vivante implementation of the backend contract ([`crate::backend`]).
#[no_mangle]
pub static yakogl_backend_vivante: YakoglBackend = YakoglBackend {
    name: c"Vivante GC7000UL (i.MX8MP)".as_ptr(),
    /* the front end reads byte and short arrays as they lie, so the GL core
     * does not have to convert them to float32 first */
    native_attrib_formats: gles_attribf_bit(GLES_ATTRIBF_BYTE)
        | gles_attribf_bit(GLES_ATTRIBF_UBYTE)
        | gles_attribf_bit(GLES_ATTRIBF_SHORT)
        | gles_attribf_bit(GLES_ATTRIBF_USHORT)
        | gles_attribf_bit(GLES_ATTRIBF_FIXED),
    /* pe_color_format(): this pixel engine has no RGBA8888 render format, so a
     * GL RGBA8888 target is written as A8R8G8B8, which is B,G,R,A in memory */
    rt_rgba8888_is_bgra: true,
    /* One LINEAR_STRIDE covers all sixteen levels the descriptor names, and a
     * level is fetched from a 64-byte boundary, so the chain is laid out at
     * level 0's pitch with every level aligned. */
    mip_level_align: 64,
    /* Mesa gives a compressed resource ETNA_LAYOUT_LINEAR and pads it by
     * nothing, so its 4x4 blocks lie in raster rows with the row pitch in the
     * descriptor's LINEAR_STRIDE -- not in the PowerVR TPU's twiddled order
     * over a power-of-two block grid. */
    compressed_blocks_in_rows: true,
    /* The texture unit reads a cube map, a 2D array and a 3D texture from the
     * descriptor's type, one layer's size and SLICE, so the GL core stacks
     * the layers plainly and leaves the sampling to the hardware (the PowerVR
     * does it in the shader instead). */
    native_layered_textures: true,
    /* Mesa compiles gl_FrontFacing's comparison into the shader and picks it
     * with a key (etnaviv_nir.c: ine for one winding, ieq for the other),
     * because the hardware value carries no sense. So do we. */
    front_facing_shader_variant: true,
    etc1_decoded_at_upload: false,
    init: Some(gc_init),
    shutdown: Some(gc_shutdown),
    draw: Some(gc_draw),
    flush: Some(gc_flush),
    finish: Some(gc_finish),
    finish_previous: Some(gc_finish_previous),
    kick_id: Some(gc_kick_id),
    kick_pending: Some(gc_kick_pending),
    heap_base: Some(gc_heap_base),
    counters: Some(gc_counters),
    /* keeps nothing per texel storage: it samples the GL core's own */
    texture_released: None,
    /* depth/stencil stay in the GL core's hands across a swap */
    surface_swapped: None,
    texture_uploaded: None,
    /* divisor 1, as before the field (GcStream carries a divisor; whether
     * the FE takes more than 1 is unmeasured) */
    max_instance_divisor: 1,
    /* the sync waits keep finish() until this backend waits for one kick */
    wait_kick: None,
    /* the texture unit reads the storage */
    samples_texel_copies: false,
    attachment_gone: Some(gc_attachment_gone),
    depth_resolve: Some(gc_depth_resolve),
};

/// The backend's own tests (the module is large enough to keep them apart, as
/// `pvr/backend_tests.rs` does).
#[cfg(test)]
#[path = "backend_tests.rs"]
mod backend_tests;
