// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! What the three Vivante backend modules share: the resolved draw
//! ([`GcDraw`]) and the frame being recorded ([`GcFrame`]).

use crate::backend::{GlesDraw, GlesTextureBind};
use crate::osal::{self, OsalGpuMem};
use crate::shaderc::ShcShader;

use super::cmd::GcCmd;

/* GPU limits of the i.MX8MP's GC7000UL, from the hardware database entry
 * (model 0x7000, revision 0x6204, product 0x70003). */
pub const GC_MAX_VERTEX_ELEMENTS: usize = 16;
pub const GC_MAX_VARYINGS: usize = 16;
/// Per stage; the two stages share one array.
pub const GC_MAX_SAMPLERS: usize = 16;
/// The constant register file in dwords: 320 vec4 registers
/// ([`super::regs::GC_NUM_CONSTANTS`], hwdb NumberOfConstants 0x140) hold
/// both stages, the vertex stage's first. One stage may take more than 256
/// of them when the other leaves room: dEQP uniform_api.random.3's vertex
/// shader has 275 (174 of uniforms, 101 of literals). Until 2026-10-01 this
/// was 1024, and the upload silently dropped registers 256 and up.
pub const GC_MAX_UNIFORM_DWORDS: usize = super::regs::GC_NUM_CONSTANTS as usize * 4;

/// One vertex element, as the front end wants it.
#[derive(Clone, Copy, Default)]
pub struct GcElem {
    /// Vertex stream index.
    pub stream: u8,
    /// `GC_DATA_TYPE_*`.
    pub type_: u8,
    /// 1..4.
    pub components: u8,
    /// `GC_NORMALIZE_*`.
    pub normalize: u8,
    /// Byte offset of the element within a vertex.
    pub start: u8,
    /// Byte offset one past it, from the stretch start.
    pub end: u8,
    /// This element ends a run of adjacent elements.
    pub nonconsecutive: bool,
    /// The vertex shader input register it feeds.
    pub vs_reg: u8,
}

#[derive(Clone, Copy, Default)]
pub struct GcStream {
    pub gpu: u64,
    pub stride: u32,
    /// 0: per vertex, 1: per instance.
    pub divisor: u32,
}

/// Everything one DRAW_INSTANCED needs once the GL state has been resolved.
/// A clear builds one of these with the backend's own quad and shaders, an
/// application draw with the program's, so both take the same path.
pub struct GcDraw<'a> {
    /// Render target and fixed-function state.
    pub gl: &'a GlesDraw,
    pub vs: &'a ShcShader,
    pub fs: &'a ShcShader,
    /// The shader binaries in GPU memory.
    pub vs_code_gpu: u64,
    pub fs_code_gpu: u64,
    pub vs_uniforms: &'a [u32],
    pub fs_uniforms: &'a [u32],
    /// The frame-arena copies of those two blocks, for a program that reads
    /// a uniform array at a run-time index with `load`
    /// (`SHC_VIV_CONST_UBO_ADDR`); 0 when the stage reads none.
    pub vs_ubo_gpu: u64,
    pub fs_ubo_gpu: u64,

    pub streams: [GcStream; GC_MAX_VERTEX_ELEMENTS],
    pub elems: [GcElem; GC_MAX_VERTEX_ELEMENTS],
    pub num_streams: u8,
    pub num_elems: u8,

    pub textures: &'a [GlesTextureBind],
    pub vs_textures: &'a [GlesTextureBind],

    pub index_gpu: u64,
    /// `GC_INDEX_TYPE_*`.
    pub index_type: u32,
    pub indexed: bool,
    pub vertex_count: u32,
    pub instance_count: u32,
    pub first: u32,
    /// `GC_PRIM_*`.
    pub prim: u32,

    /// A clear quad overrides the GL depth/stencil/blend state: depth test
    /// always with write, stencil replace with the clear value, no cull.
    pub is_clear: bool,
    pub clear_color: bool,
    pub clear_depth: bool,
    pub clear_stencil: bool,
}

impl<'a> GcDraw<'a> {
    /// The zeroed draw of `memset(&d, 0, sizeof(d))`, with the two shaders
    /// and the GL state the caller has.
    pub fn new(gl: &'a GlesDraw, vs: &'a ShcShader, fs: &'a ShcShader) -> GcDraw<'a> {
        GcDraw {
            gl,
            vs,
            fs,
            vs_code_gpu: 0,
            fs_code_gpu: 0,
            vs_uniforms: &[],
            fs_uniforms: &[],
            vs_ubo_gpu: 0,
            fs_ubo_gpu: 0,
            streams: [GcStream::default(); GC_MAX_VERTEX_ELEMENTS],
            elems: [GcElem::default(); GC_MAX_VERTEX_ELEMENTS],
            num_streams: 0,
            num_elems: 0,
            textures: &[],
            vs_textures: &[],
            index_gpu: 0,
            index_type: 0,
            indexed: false,
            vertex_count: 0,
            instance_count: 0,
            first: 0,
            prim: 0,
            is_clear: false,
            clear_color: false,
            clear_depth: false,
            clear_stencil: false,
        }
    }
}

/// The frame being recorded: one command stream plus an arena for everything
/// the stream points at (uniform snapshots, converted attributes, texture
/// descriptors, the clear quad). Both are reused once the frame's fence has
/// completed.
pub struct GcFrame {
    pub cmd: OsalGpuMem,
    pub arena: OsalGpuMem,
    pub arena_used: u32,
    /// Memory of the frame's own for a draw whose client arrays and indices
    /// are larger than the arena (dEQP texture.vertex.2d's 127x127 quad
    /// grid, 1.2 MB): [`GcFrame::arena_alloc`] takes from it once the arena
    /// is full. Kept and grown, and reused only once the frame's fence has
    /// completed, as the arena is.
    pub extra: OsalGpuMem,
    pub extra_used: u32,
    pub w: GcCmd,
    /// 0: never submitted.
    pub fence: i64,
    pub kick_id: u32,
    /// The colour target and depth surface (GPU addresses) the frame's
    /// previous draw wrote; `None` before its first. Only K1
    /// (`YAKOGL_GC_FLUSH_AS_ETNAVIV`) reads it.
    pub prev_targets: Option<(u64, u32)>,
}

impl GcFrame {
    pub const ZERO: GcFrame = GcFrame {
        cmd: OsalGpuMem::ZERO,
        arena: OsalGpuMem::ZERO,
        arena_used: 0,
        extra: OsalGpuMem::ZERO,
        extra_used: 0,
        w: GcCmd::ZERO,
        fence: 0,
        kick_id: 0,
        prev_targets: None,
    };

    /// Reserve `bytes` of frame arena at `align`; returns the CPU pointer and
    /// the GPU address, or `None` when the arena is full -- and the frame's
    /// `extra` block, when it has one, is full too.
    pub fn arena_alloc(&mut self, bytes: usize, align: usize) -> Option<(*mut u8, u64)> {
        let off = (self.arena_used as usize + (align - 1)) & !(align - 1);

        if !self.arena.cpu.is_null() && off.checked_add(bytes)? <= self.arena.size {
            self.arena_used = (off + bytes) as u32;
            // SAFETY: `off + bytes` is inside the arena's mapping (checked).
            return Some((unsafe { (self.arena.cpu as *mut u8).add(off) }, self.arena.gpu + off as u64));
        }
        let off = (self.extra_used as usize + (align - 1)) & !(align - 1);
        if self.extra.cpu.is_null() || off.checked_add(bytes)? > self.extra.size {
            return None;
        }
        self.extra_used = (off + bytes) as u32;
        // SAFETY: `off + bytes` is inside the extra block's mapping (checked).
        Some((unsafe { (self.extra.cpu as *mut u8).add(off) }, self.extra.gpu + off as u64))
    }

    /// Copy `src` into the frame arena and return its GPU address, or 0. The
    /// GL core's staging buffers (converted attributes, widened indices,
    /// uniform blocks) are reused by the next draw, and this frame's stream is
    /// still pending.
    ///
    /// # Safety
    /// `cpu` must be valid for `bytes` bytes, or null.
    pub unsafe fn snapshot(&mut self, cpu: *const c_void, bytes: u32) -> u64 {
        if cpu.is_null() || bytes == 0 {
            return 0;
        }
        let Some((dst, gpu)) = self.arena_alloc(bytes as usize, 64) else {
            return 0;
        };
        // SAFETY: the caller's contract for `cpu`; `dst` has room for `bytes`.
        unsafe { core::ptr::copy_nonoverlapping(cpu as *const u8, dst, bytes as usize) };
        gpu
    }
}

use core::ffi::c_void;

/// `if (g_osal->cache_flush) g_osal->cache_flush(cpu, n)` -- as in the C, with
/// no null / zero shortcut, so the OSAL call stream is the one the C makes.
///
/// # Safety
/// `cpu..cpu + n` must be inside a live CPU mapping, or `cpu` null.
pub unsafe fn cache_flush_raw(cpu: *mut c_void, n: usize) {
    if let Some(f) = osal::ops().and_then(|o| o.cache_flush) {
        // SAFETY: the caller's contract.
        unsafe { f(cpu, n) };
    }
}
