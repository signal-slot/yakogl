// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A 4x4-tiled shadow of a colour target with 64-bit pixels (RGBA16F, and
//! the integer RGBA16I/UI and RG32I/UI), for the pixel engine to render into.
//! It is chosen by pixel size ([`wanted`]) and copies whole 8-byte pixels, so
//! the format's channels never matter here.
//!
//! The pixel engine writes 64-bit pixels into a LINEAR surface at the wrong
//! places on this core: gl_test 44's probe (#6, cba8b199, record `got
//! 0x20000004 want 0x00c2073e`) drew one picture into RGBA16F targets 8, 4,
//! 16, 24 and 32 pixels wide and an RGBA8 control. Only the control came back
//! whole. In every row of every RGBA16F width, pixels 0 and 1 were right,
//! slot 2 held pixel 6's colour and slot 7 pixel 5's: the PE moves 16-byte
//! pixel pairs inside a row, at every row pitch from 32 to 256 bytes
//! (VIVANTE_NOTES.md, "RGBA16F targets", hypothesis 1).
//!
//! Mesa never renders a LINEAR target on this core (LINEAR_PE is used only
//! under the `linear_pe` debug flag, etnaviv_screen.c), so it never meets
//! this. Its render targets are tiled: `etna_resource_get_render_compatible`
//! (etnaviv_resource.c) gives a texture that is not render compatible a
//! tiled render copy, and the CPU untiles a tiled resource when it maps it
//! (this core has no BLT engine, and the RS copies no 64-bit format:
//! `etna_compatible_rs_format`). The same here, for 64-bit targets only:
//!
//! - A draw into such a target renders into this shadow: TILED (4x4 tiles,
//!   no SUPER_TILED bit), its width padded to 16 and its height to 4 as
//!   `etna_layout_multiple` pads a TILED render target on a core without the
//!   BLT engine, its row stride `padded width * 8` as Mesa programs
//!   PE_COLOR_STRIDE (`level->stride`), and PE_LOGIC_OP SINGLE_BUFFER(2),
//!   Mesa's value for a tiled 32/64-bit target on a SINGLE_BUFFER core
//!   (etnaviv_state.c). The shadow starts on a page, as every buffer Mesa
//!   hands this GPU does.
//! - Before the first draw of a kick into it, while the linear storage is
//!   the newer copy, the CPU tiles the linear storage into the shadow, so a
//!   draw that does not clear keeps what the target held.
//! - When the backend learns that the last kick rendering into it has
//!   completed (`retire`: from `gc_kick_pending`, `gc_finish`,
//!   `gc_finish_previous`), the CPU untiles the shadow into the linear
//!   storage. Every reader of the storage in the GL core asks one of those
//!   first (glReadPixels and glCopyTexImage through `finish`, a CPU write
//!   through `kick_pending` on the render's kick, the tiled sampling copy
//!   through `kick_pending`), so it reads the rendered pixels.
//! - The GL core drops the storage through `texture_released` before its
//!   address can be handed out again; the shadow goes with it, untouched.
//!
//! Not covered: a draw with more than one colour target (the 8-RT bank's
//! RT_CONFIG has no tiled bit of its own: SINGLE_BUFFER switches every
//! target), which keeps the linear path (an RGBA16F target there still
//! does; `draw_one` refuses the 64-bit integer ones); and the depth surface
//! of a draw into a shadow, which SINGLE_BUFFER(2) switches too: its layout
//! under (2) is unmeasured, which matters only to `depth_resolve` and to a
//! depth buffer shared with draws into 32-bit targets (logged once when a
//! draw reaches it). ES 2.0 has no 64-bit colour format to render into.
//!
//! Prepared, not reached: a draw described target by target ([`Target`],
//! [`wanted_targets`], [`prepare_targets`], [`bind_targets`]), every target
//! of it on a shadow of its own pixel size, and a 128-bit target (RGBA32I/UI)
//! as one shadow of two 8-byte planes (`split_planes` in, `join_planes` out),
//! Mesa's two G32R32F render targets. `backend.rs` still refuses RGBA32I/UI
//! and calls only the one-target functions; docs/notes/gc-128bit-render-targets.md,
//! "Implementation sequence", step 1 says what it must call.

use crate::backend::{gles_surf_bytes_per_pixel, GlesDraw};
use crate::osal::{self, OsalGpuMem, OSAL_MEM_DATA};
use crate::util::Global;

use super::layout::{self, Layout};

/// Bytes a pixel of a target the one-target path ([`wanted`]) takes.
const BPP: u32 = 8;
/// Bytes a 32-bit pixel takes: a shadow of it is written back by the RS in
/// the stream that renders into it, never by [`retire`].
const BPP32: u32 = 4;
/// Where the shadow starts: a page.
const BASE_ALIGN: usize = 4096;
/// Shadows kept at once.
const ENTRIES: usize = 8;

/// The target a shadow stands for: a new value of any field is a new shadow.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Key {
    /// The linear storage's GPU address (`GlesDraw::target_gpu`).
    gpu: u64,
    w: u32,
    h: u32,
    /// The linear storage's row pitch in bytes.
    pitch: u32,
    /// Bytes a pixel: 8 on the one-target path; 16 is a two-plane entry.
    bpp: u32,
}

/// Who holds the newest pixels of a target.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    /// The shadow is rendered into and written back inside the stream being
    /// recorded; no CPU tile-in or untile is due for it.
    InStep,
    /// The shadow is the newer copy: the last kick rendering into it.
    ShadowNewer(u32),
    /// The linear storage is the newer copy.
    StorageNewer,
}

struct Entry {
    key: Key,
    /// Bytes a pixel (`key.bpp`).
    bpp: u32,
    state: State,
    /// The linear storage's CPU view.
    cpu: *mut u8,
    mem: OsalGpuMem,
    /// The shadow's GPU address and CPU view: the first page boundary in
    /// `mem`.
    gpu: u64,
    shadow: *mut u8,
    /// For the LRU.
    stamp: u32,
}

struct Shadows {
    entries: [Option<Entry>; ENTRIES],
    clock: u32,
    /// Bytes untiled and tiled since init (`counts`).
    untiled: u64,
    tiled: u64,
}

static SHADOWS: Global<Shadows> =
    Global::new(Shadows { entries: [const { None }; ENTRIES], clock: 0, untiled: 0, tiled: 0 });

fn shadows() -> &'static mut Shadows {
    // SAFETY: single-threaded by contract; each entry point takes it once.
    unsafe { SHADOWS.get() }
}

const fn align_up(v: u32, a: u32) -> u32 {
    (v + a - 1) & !(a - 1)
}

/// The shadow's size in pixels: Mesa's TILED render-target padding on a core
/// without the BLT engine (`etna_layout_multiple`: paddingX 16, paddingY 4).
pub const fn padded(w: u32, h: u32) -> (u32, u32) {
    (layout::aligned_width(Layout::Tiled, w, true), layout::aligned_height(Layout::Tiled, h, true))
}

/// PE_COLOR_STRIDE for a shadow of a `w`-pixel-wide target: one row of
/// pixels of the padded width (Mesa's `level->stride`; only the RS takes a
/// tiled stride times 4).
pub const fn stride(w: u32) -> u32 {
    stride_bpp(w, BPP)
}

/// [`stride`] for pixels of `bpp` bytes (`layout::stride`).
pub const fn stride_bpp(w: u32, bpp: u32) -> u32 {
    layout::stride(Layout::Tiled, w, bpp, true)
}

/// Byte offset of pixel (x, y) in a tiled surface `tw` pixels wide (a
/// multiple of 4): 4x4 tiles row-major, pixels row-major inside a tile.
const fn tiled_offset(x: u32, y: u32, tw: u32) -> usize {
    tiled_offset_bpp(x, y, tw, BPP)
}

/// [`tiled_offset`] for pixels of `bpp` bytes.
const fn tiled_offset_bpp(x: u32, y: u32, tw: u32, bpp: u32) -> usize {
    let tile = (y / 4) * (tw / 4) + x / 4;
    ((tile * 16 + (y % 4) * 4 + x % 4) * bpp) as usize
}

/// Bytes a texel of a 128-bit target takes in the linear storage.
const WIDE_BPP: usize = 16;

/// Where the second (B,A) plane of a 128-bit target starts in its two-plane
/// image, `tw` pixels wide (a multiple of 4) and `h` tall: half-way, Mesa's
/// `etna_resource_level_second_plane_offset`. Asked of
/// [`super::tiled_copy::second_plane_offset`], the texture side's rule.
pub fn plane_offset(tw: u32, h: u32) -> usize {
    let t =
        crate::backend::GlesTextureBind { width: tw, height: h, layers: 1, ..crate::backend::GlesTextureBind::ZERO };
    super::tiled_copy::second_plane_offset(&t, 0)
}

/// Bytes of the two-plane image of a `tw` x `h` 128-bit target.
pub fn planes_bytes(tw: u32, h: u32) -> usize {
    2 * plane_offset(tw, h)
}

/// A linear image of 16-byte pixels (R,G,B,A as four 32-bit words, rows
/// `pitch` bytes apart) into the two 4x4-tiled 8-byte planes Mesa renders a
/// 128-bit target into: bytes 0-7 of each pixel (R,G) tiled into the first
/// plane, bytes 8-15 (B,A) into the second at [`plane_offset`]. Mesa's
/// `etna_texture_tile_rgba32f` (etnaviv_tiling.c:137-158). `tw` is the tiled
/// width, at least `w` and a multiple of 4; the padding stays as `dst` had
/// it. Inert: nothing draws through it yet.
pub fn split_planes(src: &[u8], pitch: usize, (w, h): (u32, u32), tw: u32, dst: &mut [u8]) {
    let second = plane_offset(tw, h.next_multiple_of(4));
    for y in 0..h {
        for x in 0..w {
            let from = y as usize * pitch + x as usize * WIDE_BPP;
            let to = tiled_offset(x, y, tw);
            dst[to..to + 8].copy_from_slice(&src[from..from + 8]);
            dst[second + to..second + to + 8].copy_from_slice(&src[from + 8..from + 16]);
        }
    }
}

/// The inverse of [`split_planes`]: the two tiled planes joined back into
/// interleaved 16-byte pixels (`etna_texture_untile_rgba32f`). Bytes of `dst`
/// outside the `w` x `h` pixels are left alone.
pub fn join_planes(src: &[u8], tw: u32, (w, h): (u32, u32), pitch: usize, dst: &mut [u8]) {
    let second = plane_offset(tw, h.next_multiple_of(4));
    for y in 0..h {
        for x in 0..w {
            let to = y as usize * pitch + x as usize * WIDE_BPP;
            let from = tiled_offset(x, y, tw);
            dst[to..to + 8].copy_from_slice(&src[from..from + 8]);
            dst[to + 8..to + 16].copy_from_slice(&src[second + from..second + from + 8]);
        }
    }
}

/// The two planes' colour write masks of a 128-bit target from the GL
/// 4-bit RGBA mask (bit 0 R, 1 G, 2 B, 3 A): plane 0 holds (R,G) and takes
/// the low two bits, plane 1 holds (B,A) and takes the high two. Inert:
/// `backend.rs` does not call it yet (docs/notes/gc-128bit-render-targets.md,
/// "Implementation sequence", step 4).
pub const fn plane_masks(mask: u8) -> (u8, u8) {
    (mask & 3, (mask >> 2) & 3)
}

/// The two planes' 64-bit clear words of a 128-bit clear value, `rgba` the
/// raw bits of R,G,B,A (an integer's two's complement, a float's IEEE bits):
/// plane 0 is `g << 32 | r`, plane 1 is `a << 32 | b`, so the planes hold the
/// pixel's own bytes. Inert (step 5 of the same sequence).
pub const fn plane_clears(rgba: [u32; 4]) -> (u64, u64) {
    ((rgba[1] as u64) << 32 | rgba[0] as u64, (rgba[3] as u64) << 32 | rgba[2] as u64)
}

/// Does this draw render into a shadow? One colour target (`single`: the
/// caller's count of hardware targets is 1), with 64-bit pixels, a CPU view
/// of its storage, and drawn with a positive stride straight into it (not
/// through the 4x scratch, not bottom-up).
pub fn wanted(gl: &GlesDraw, single: bool, straight: bool) -> bool {
    single
        && straight
        && gl.target_gpu != 0
        && !gl.target_cpu.is_null()
        && gles_surf_bytes_per_pixel(gl.target_format) == BPP
}

fn key_of(gl: &GlesDraw) -> Key {
    Key { gpu: gl.target_gpu, w: gl.target_w, h: gl.target_h, pitch: crate::backend::draw_pitch(gl) * BPP, bpp: BPP }
}

/// Bytes a shadow of `key` takes from its page boundary on: the padded
/// pixels, and for a 128-bit target its two planes ([`planes_bytes`]).
fn shadow_bytes(key: &Key) -> usize {
    let (pw, ph) = padded(key.w, key.h);
    if key.bpp as usize == WIDE_BPP {
        planes_bytes(pw, ph)
    } else {
        (pw * ph * key.bpp) as usize
    }
}

/// GL attachments one draw describes at most ([`Target`]). A 128-bit one
/// takes two hardware targets, so up to twice as many PE slots.
pub const MAX_TARGETS: usize = crate::backend::GLES_MAX_TARGETS;

/// One colour target of a draw, for the multi-target shadow. The caller
/// fills it: `GlesDrawTarget` carries no CPU view or pitch of the
/// attachments after the first, so only `target_of` builds one from a draw.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Target {
    /// The linear storage: GPU address and CPU view.
    pub gpu: u64,
    pub cpu: *mut u8,
    pub w: u32,
    pub h: u32,
    /// The linear storage's row pitch in bytes.
    pub pitch: u32,
    /// Bytes a pixel (`gles_surf_bytes_per_pixel`): 2, 4, 8 or 16.
    pub bpp: u32,
}

/// The draw's own target (`target_*`, attachment 0) as a [`Target`].
pub fn target_of(gl: &GlesDraw) -> Target {
    let bpp = gles_surf_bytes_per_pixel(gl.target_format);
    Target {
        gpu: gl.target_gpu,
        cpu: gl.target_cpu as *mut u8,
        w: gl.target_w,
        h: gl.target_h,
        pitch: crate::backend::draw_pitch(gl) * bpp,
        bpp,
    }
}

const fn target_key(t: &Target) -> Key {
    Key { gpu: t.gpu, w: t.w, h: t.h, pitch: t.pitch, bpp: t.bpp }
}

/// PE_COLOR_STRIDE / PE_RT_CONFIG stride of a shadow of `t`: the padded
/// width in pixels of its size, for a 128-bit target of one 8-byte plane
/// (Mesa's 64-bit twin's `level->stride`).
pub const fn target_stride(t: &Target) -> u32 {
    let bpp = if t.bpp as usize == WIDE_BPP { BPP } else { t.bpp };
    stride_bpp(t.w, bpp)
}

/// Does a draw into `targets` render into shadows, one a target? Every
/// target with a CPU view and a pixel size the shadow copies, at least one
/// of them 64 or 128 bits (SINGLE_BUFFER then tiles all of them, a 32-bit
/// target beside a 128-bit one too), drawn `straight` as in [`wanted`]. For
/// one 64-bit target, the same answer as [`wanted`] with `single`.
pub fn wanted_targets(targets: &[Target], straight: bool) -> bool {
    straight
        && !targets.is_empty()
        && targets.len() <= MAX_TARGETS
        && targets.iter().all(|t| t.gpu != 0 && !t.cpu.is_null() && matches!(t.bpp, 2 | 4 | 8 | 16))
        && targets.iter().any(|t| t.bpp >= BPP)
}

/// Make the shadows of every target of a draw current ([`prepare`], one a
/// target); none of them pushes out another of the same draw. Non-zero
/// when one has no shadow: the draw is then refused.
pub fn prepare_targets(targets: &[Target], finish: impl Fn() -> i32) -> i32 {
    let mut keys = [Key { gpu: 0, w: 0, h: 0, pitch: 0, bpp: 0 }; MAX_TARGETS];
    if targets.len() > MAX_TARGETS {
        return -1;
    }
    for (i, t) in targets.iter().enumerate() {
        keys[i] = target_key(t);
        if prepare_key(keys[i], t.cpu, &keys[..i], &finish) != 0 {
            return -1;
        }
    }
    0
}

/// Where a target of a draw renders: its shadow's GPU address, the second
/// (B,A) plane's for a 128-bit target (0 otherwise), and the stride.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Bound {
    pub gpu: u64,
    pub plane1: u64,
    pub stride: u32,
}

impl Bound {
    pub const ZERO: Bound = Bound { gpu: 0, plane1: 0, stride: 0 };
}

/// The shadows `targets` render into, all marked as written by `kick` (the
/// kick being recorded), in the order of `targets`. `None`, and nothing
/// marked, when [`prepare_targets`] left one without a shadow.
pub fn bind_targets(targets: &[Target], kick: u32) -> Option<[Bound; MAX_TARGETS]> {
    if targets.len() > MAX_TARGETS {
        return None;
    }
    let s = shadows();
    let mut slots = [0usize; MAX_TARGETS];
    for (i, t) in targets.iter().enumerate() {
        let key = target_key(t);
        slots[i] = s.entries.iter().position(|e| e.as_ref().is_some_and(|e| e.key == key))?;
    }
    let mut out = [Bound::ZERO; MAX_TARGETS];
    for (i, t) in targets.iter().enumerate() {
        let e = s.entries[slots[i]].as_mut()?;
        e.state = State::ShadowNewer(kick);
        let (pw, ph) = padded(t.w, t.h);
        let plane1 = if t.bpp as usize == WIDE_BPP { e.gpu + plane_offset(pw, ph) as u64 } else { 0 };
        out[i] = Bound { gpu: e.gpu, plane1, stride: target_stride(t) };
    }
    Some(out)
}

/// Make the shadow for `gl`'s target current, before the draw records
/// anything: allocate it, and tile the linear storage into it when the
/// storage is the newer copy. `finish` waits for every kick (and retires
/// them) when every slot holds a shadow a pending kick renders into.
/// Non-zero when there is no shadow; the draw is then refused rather than
/// rendered into the linear storage, which the PE writes wrong.
pub fn prepare(gl: &GlesDraw, finish: impl Fn() -> i32) -> i32 {
    prepare_key(key_of(gl), gl.target_cpu as *mut u8, &[], &finish)
}

/// [`prepare`] of one target, `keep` the shadows of the same draw already
/// prepared, which no slot is taken from.
fn prepare_key(key: Key, cpu: *mut u8, keep: &[Key], finish: &impl Fn() -> i32) -> i32 {
    let s = shadows();
    s.clock = s.clock.wrapping_add(1);
    let clock = s.clock;
    if let Some(e) = s.entries.iter_mut().flatten().find(|e| e.key == key) {
        e.stamp = clock;
        return if e.state == State::StorageNewer { tile_in(e) } else { 0 };
    }
    /* a shadow of the same storage at another size or pitch: the storage was
     * specified again; one that is still the newer copy goes back first */
    if s.entries.iter().flatten().any(|e| e.key.gpu == key.gpu && e.kick() != 0) && finish() != 0 {
        return -1;
    }
    for e in s.entries.iter_mut() {
        if e.as_ref().is_some_and(|e| e.key.gpu == key.gpu && !keep.contains(&e.key)) {
            if let Some(mut old) = e.take() {
                osal::gpu_free(&mut old.mem);
            }
        }
    }
    let slot = match free_slot(s, keep) {
        Some(i) => i,
        None => {
            /* every shadow is the newer copy of its target: let the kicks
             * complete, which hands them all back (`retire`) */
            if finish() != 0 {
                return -1;
            }
            match free_slot(s, keep) {
                Some(i) => i,
                None => return -1,
            }
        }
    };
    if let Some(mut old) = s.entries[slot].take() {
        osal::gpu_free(&mut old.mem);
    }
    let bytes = shadow_bytes(&key);
    let mut mem = OsalGpuMem::ZERO;
    if osal::gpu_alloc(super::backend::heap_data(), bytes + BASE_ALIGN - 1, OSAL_MEM_DATA, &mut mem) != 0 {
        crate::glog_once!("vivante: no GPU memory for a {bytes}-byte tiled colour target");
        return -1;
    }
    if mem.cpu.is_null() {
        osal::gpu_free(&mut mem);
        return -1;
    }
    let skip = (mem.gpu.wrapping_neg() & (BASE_ALIGN as u64 - 1)) as usize;
    let e = Entry {
        key,
        cpu,
        gpu: mem.gpu + skip as u64,
        // SAFETY: `skip` is below BASE_ALIGN, inside the allocation.
        shadow: unsafe { (mem.cpu as *mut u8).add(skip) },
        mem,
        bpp: key.bpp,
        state: State::StorageNewer,
        stamp: clock,
    };
    // SAFETY: `bytes` from the page boundary on, inside the allocation.
    unsafe { core::ptr::write_bytes(e.shadow, 0, bytes) };
    s.entries[slot] = Some(e);
    let Some(e) = s.entries[slot].as_mut() else { return -1 };
    tile_in(e)
}

/// An empty slot, else the least recently used one whose shadow is not the
/// newer copy of its target, nor one of `keep`.
fn free_slot(s: &Shadows, keep: &[Key]) -> Option<usize> {
    if let Some(i) = s.entries.iter().position(|e| e.is_none()) {
        return Some(i);
    }
    s.entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| e.as_ref().map(|e| (i, e)))
        .filter(|(_, e)| e.state == State::StorageNewer && !keep.contains(&e.key))
        .min_by_key(|(_, e)| e.stamp)
        .map(|(i, _)| i)
}

/// Bytes of the linear storage `key`'s pixels span: `h` rows of `pitch`,
/// the last one only as far as its last pixel.
fn linear_bytes(key: &Key) -> usize {
    (key.pitch as usize) * (key.h as usize - 1) + (key.w * key.bpp) as usize
}

/// The linear storage into the shadow (the storage is the newer copy, and
/// no kick writes either). A 128-bit target splits into its two planes.
fn tile_in(e: &mut Entry) -> i32 {
    let Key { w, h, pitch, bpp, .. } = e.key;
    let tw = padded(w, h).0;
    let rows = linear_bytes(&e.key);
    let bytes = shadow_bytes(&e.key);
    // SAFETY: the storage holds `h` rows of `pitch` bytes (the GL core's
    // target), the shadow `shadow_bytes`; both CPU views are live.
    unsafe {
        osal::cache_invd(e.cpu as *mut core::ffi::c_void, rows);
        if bpp as usize == WIDE_BPP {
            let src = core::slice::from_raw_parts(e.cpu, rows);
            split_planes(src, pitch as usize, (w, h), tw, core::slice::from_raw_parts_mut(e.shadow, bytes));
        } else {
            for y in 0..h {
                for x in 0..w {
                    let from = (y * pitch + x * bpp) as usize;
                    let to = tiled_offset_bpp(x, y, tw, bpp);
                    core::ptr::copy_nonoverlapping(e.cpu.add(from), e.shadow.add(to), bpp as usize);
                }
            }
        }
        osal::cache_flush(e.shadow as *mut core::ffi::c_void, bytes);
    }
    shadows().tiled += u64::from(w * h * bpp);
    0
}

/// The shadow `gl` renders into, marked as written by `kick` (the kick being
/// recorded): its GPU address. `None` when [`prepare`] made none.
pub fn bind(gl: &GlesDraw, kick: u32) -> Option<u64> {
    let key = key_of(gl);
    let e = shadows().entries.iter_mut().flatten().find(|e| e.key == key)?;
    e.state = State::ShadowNewer(kick);
    Some(e.gpu)
}

impl Entry {
    /// The kick that last rendered into the shadow while it is the newer
    /// copy; 0 otherwise.
    fn kick(&self) -> u32 {
        match self.state {
            State::ShadowNewer(k) => k,
            _ => 0,
        }
    }
}

/// The entry of the target at `gpu`, if it has a shadow.
fn entry_at(gpu: u64) -> Option<&'static mut Entry> {
    shadows().entries.iter_mut().flatten().find(|e| e.key.gpu == gpu)
}

/// The linear storage at `gpu` is the newer copy of its target (the GL core
/// wrote it); the next [`prepare`] tiles it in. False when it has no shadow.
pub fn mark_storage_newer(gpu: u64) -> bool {
    entry_at(gpu).map(|e| e.state = State::StorageNewer).is_some()
}

/// The shadow at `gpu` is rendered into and written back inside the stream
/// being recorded: [`retire`] leaves it alone. False when it has no shadow.
pub fn mark_in_step(gpu: u64) -> bool {
    entry_at(gpu).map(|e| e.state = State::InStep).is_some()
}

/// Does the target at `gpu` have a shadow that is in-step?
pub fn in_step(gpu: u64) -> bool {
    entry_at(gpu).is_some_and(|e| e.state == State::InStep)
}

/// The state of the shadow of the target at `gpu`, if it has one.
pub fn state_of(gpu: u64) -> Option<State> {
    entry_at(gpu).map(|e| e.state)
}

/// Every shadow whose last rendering kick has completed (`pending` says it
/// has not) goes back into its linear storage, which is the newer copy from
/// then on. A 128-bit target's two planes join into its 16-byte pixels.
pub fn retire(pending: impl Fn(u32) -> bool) {
    let s = shadows();
    let mut untiled = 0u64;
    for e in s.entries.iter_mut().flatten() {
        if e.kick() == 0 || pending(e.kick()) {
            continue;
        }
        if e.bpp == BPP32 {
            /* the RS wrote it back in its own stream (a later step): no
             * untile here, the storage is the newer copy */
            e.state = State::StorageNewer;
            continue;
        }
        let Key { w, h, pitch, bpp, .. } = e.key;
        let pw = padded(w, h).0;
        let rows = linear_bytes(&e.key);
        let bytes = shadow_bytes(&e.key);
        // SAFETY: as in `tile_in`; the kick that wrote the shadow completed.
        unsafe {
            osal::cache_invd(e.shadow as *mut core::ffi::c_void, bytes);
            if bpp as usize == WIDE_BPP {
                let src = core::slice::from_raw_parts(e.shadow, bytes);
                join_planes(src, pw, (w, h), pitch as usize, core::slice::from_raw_parts_mut(e.cpu, rows));
            } else {
                for y in 0..h {
                    for x in 0..w {
                        let to = (y * pitch + x * bpp) as usize;
                        let from = tiled_offset_bpp(x, y, pw, bpp);
                        core::ptr::copy_nonoverlapping(e.shadow.add(from), e.cpu.add(to), bpp as usize);
                    }
                }
            }
            osal::cache_flush(e.cpu as *mut core::ffi::c_void, rows);
        }
        untiled += u64::from(w * h * bpp);
        e.state = State::StorageNewer;
    }
    s.untiled += untiled;
}

/// The GL core drops the storage `[base, base + bytes)`
/// (`YakoglBackend::texture_released`): the shadows of targets in it go too,
/// without being written back.
pub fn released(base: u64, bytes: u64) {
    for e in shadows().entries.iter_mut() {
        if e.as_ref().is_some_and(|e| e.key.gpu >= base && e.key.gpu < base.wrapping_add(bytes)) {
            if let Some(mut old) = e.take() {
                osal::gpu_free(&mut old.mem);
            }
        }
    }
}

/// Bytes tiled into shadows and untiled out of them since init.
pub fn counts() -> (u64, u64) {
    let s = shadows();
    (s.tiled, s.untiled)
}

/// Free every shadow (backend shutdown).
pub fn release_all() {
    let s = shadows();
    for e in s.entries.iter_mut() {
        if let Some(mut old) = e.take() {
            osal::gpu_free(&mut old.mem);
        }
    }
    s.clock = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shadow_is_padded_as_mesa_pads_a_tiled_render_target() {
        assert_eq!(padded(8, 8), (16, 8));
        assert_eq!(padded(55, 63), (64, 64));
        assert_eq!(padded(16, 4), (16, 4));
        assert_eq!(stride(8), 128);
        assert_eq!(stride(32), 256);
    }

    #[test]
    fn a_pixel_lands_in_its_4x4_tile_row_major() {
        /* 16 wide: four tiles a tile row, 128 bytes a tile */
        assert_eq!(tiled_offset(0, 0, 16), 0);
        assert_eq!(tiled_offset(1, 0, 16), 8);
        assert_eq!(tiled_offset(0, 1, 16), 32);
        assert_eq!(tiled_offset(4, 0, 16), 128);
        assert_eq!(tiled_offset(0, 4, 16), 512);
        assert_eq!(tiled_offset(7, 5, 16), 512 + 128 + 32 + 24);
    }

    /// R,G,B,A: negative, above 2^24 (the float mantissa), and a pattern.
    fn texel(x: u32, y: u32) -> [u8; 16] {
        let words = [
            (-(x as i32) - 1 - 1000 * y as i32) as u32,
            0x0100_0001 + x + 77 * y,
            0xffff_ff00 - x,
            0x8000_0000 | (y << 8) | x,
        ];
        let mut b = [0u8; 16];
        for (i, v) in words.iter().enumerate() {
            b[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    fn image(w: u32, h: u32, pitch: usize) -> std::vec::Vec<u8> {
        let mut v = std::vec![0xa5u8; pitch * h as usize];
        for y in 0..h {
            for x in 0..w {
                let at = y as usize * pitch + x as usize * 16;
                v[at..at + 16].copy_from_slice(&texel(x, y));
            }
        }
        v
    }

    #[test]
    fn the_second_plane_starts_half_way() {
        assert_eq!(plane_offset(16, 8), 16 * 8 * 8);
        assert_eq!(planes_bytes(16, 8), 2 * 16 * 8 * 8);
        assert_eq!(plane_offset(4, 4), 128);
    }

    #[test]
    fn a_texel_splits_rg_into_plane_0_and_ba_into_plane_1() {
        let (w, h, tw) = (8u32, 8u32, 16u32);
        let src = image(w, h, 8 * 16);
        let mut dst = std::vec![0u8; planes_bytes(tw, h)];
        split_planes(&src, 8 * 16, (w, h), tw, &mut dst);
        let second = plane_offset(tw, h);
        for (x, y) in [(0, 0), (1, 0), (0, 1), (4, 0), (0, 4), (7, 5), (7, 7)] {
            let at = tiled_offset(x, y, tw);
            let t = texel(x, y);
            assert_eq!(&dst[at..at + 8], &t[..8], "RG of ({x},{y})");
            assert_eq!(&dst[second + at..second + at + 8], &t[8..], "BA of ({x},{y})");
        }
        /* the padding columns stay as they were */
        assert!(dst[tiled_offset(8, 0, tw)..][..8].iter().all(|&b| b == 0));
    }

    #[test]
    fn split_then_join_gives_every_pixel_back_bit_for_bit() {
        for (w, h) in [(8u32, 8u32), (16, 4), (1, 1), (5, 3), (7, 9), (13, 6), (17, 17)] {
            let tw = align_up(w, 4);
            let pitch = (w as usize + 3) * 16;
            let src = image(w, h, pitch);
            let mut planes = std::vec![0u8; planes_bytes(tw, align_up(h, 4))];
            split_planes(&src, pitch, (w, h), tw, &mut planes);
            let mut back = std::vec![0xa5u8; pitch * h as usize];
            join_planes(&planes, tw, (w, h), pitch, &mut back);
            assert_eq!(back, src, "{w}x{h}");
        }
    }

    #[test]
    fn the_render_target_padding_round_trips_too() {
        /* the shadow's own padding: width to 16, height to 4 */
        let (w, h) = (21u32, 10u32);
        let (tw, th) = padded(w, h);
        let pitch = w as usize * 16;
        let src = image(w, h, pitch);
        let mut planes = std::vec![0u8; planes_bytes(tw, th)];
        split_planes(&src, pitch, (w, h), tw, &mut planes);
        let mut back = std::vec![0u8; src.len()];
        join_planes(&planes, tw, (w, h), pitch, &mut back);
        assert_eq!(back, src);
    }

    #[test]
    fn negative_values_and_values_above_2_pow_24_keep_all_32_bits() {
        let t = [i32::MIN as u32, -1i32 as u32, 0x0100_0001, u32::MAX];
        let mut px = [0u8; 16];
        for (i, v) in t.iter().enumerate() {
            px[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        let mut planes = std::vec![0u8; planes_bytes(4, 4)];
        split_planes(&px, 16, (1, 1), 4, &mut planes);
        assert_eq!(&planes[..8], &px[..8]);
        assert_eq!(&planes[128..136], &px[8..]);
        let mut back = [0u8; 16];
        join_planes(&planes, 4, (1, 1), 16, &mut back);
        assert_eq!(back, px);
    }

    #[test]
    fn a_write_mask_splits_into_the_two_planes() {
        assert_eq!(plane_masks(0b1111), (3, 3));
        assert_eq!(plane_masks(0b0000), (0, 0));
        assert_eq!(plane_masks(0b0001), (1, 0), "R");
        assert_eq!(plane_masks(0b0010), (2, 0), "G");
        assert_eq!(plane_masks(0b0100), (0, 1), "B");
        assert_eq!(plane_masks(0b1000), (0, 2), "A");
        assert_eq!(plane_masks(0b1010), (2, 2));
        for m in 0..16u8 {
            let (a, b) = plane_masks(m);
            assert_eq!(a | b << 2, m);
        }
    }

    #[test]
    fn a_clear_value_splits_into_two_plane_words() {
        assert_eq!(plane_clears([1, 2, 3, 4]), (0x0000_0002_0000_0001, 0x0000_0004_0000_0003));
        /* negative integers and bits above 2^24 keep all 32 bits */
        let (p0, p1) = plane_clears([-5i32 as u32, 0x0100_0001, i32::MIN as u32, u32::MAX]);
        assert_eq!(p0, 0x0100_0001_ffff_fffb);
        assert_eq!(p1, 0xffff_ffff_8000_0000);
        /* a float's bits go through unchanged */
        let (p0, _) = plane_clears([1.0f32.to_bits(), (-2.0f32).to_bits(), 0, 0]);
        assert_eq!(p0.to_le_bytes()[..4], 1.0f32.to_le_bytes());
        assert_eq!(p0.to_le_bytes()[4..], (-2.0f32).to_le_bytes());
    }

    #[test]
    fn a_clear_word_is_what_the_planes_hold_for_that_pixel() {
        let w = [0xdead_beef, 0x0123_4567, 0x89ab_cdef, 0xffff_0001u32];
        let mut px = [0u8; 16];
        for (i, v) in w.iter().enumerate() {
            px[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        let (p0, p1) = plane_clears(w);
        assert_eq!(p0.to_le_bytes(), px[..8]);
        assert_eq!(p1.to_le_bytes(), px[8..]);
    }

    /* ---- the shadow itself, on a host OSAL ---------------------------- */

    use crate::backend::{GLES_SURF_RGBA16F, GLES_SURF_RGBA8888};
    use crate::osal::OsalOps;
    use crate::testutil::TestGl;
    use core::ffi::c_void;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Live allocations of [`HOST_OSAL`].
    static LIVE: AtomicUsize = AtomicUsize::new(0);

    /// Host memory with the CPU address as the GPU address, so a test reads
    /// a shadow at the address [`bind`] hands out; filled with 0x5a.
    unsafe extern "C" fn h_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
        let cpu = std::boxed::Box::into_raw(std::vec![0x5au8; size].into_boxed_slice()) as *mut u8;
        LIVE.fetch_add(1, Ordering::Relaxed);
        // SAFETY: the caller's out-parameter.
        unsafe {
            *out = OsalGpuMem { cpu: cpu as *mut c_void, gpu: cpu as u64, phys: 0, size, impl_: core::ptr::null_mut() };
        }
        0
    }

    unsafe extern "C" fn h_free(mem: *mut OsalGpuMem) -> i32 {
        // SAFETY: the caller's descriptor, made by `h_alloc` or empty.
        let m = unsafe { &mut *mem };
        if !m.cpu.is_null() {
            // SAFETY: `h_alloc`'s box of `size` bytes.
            drop(unsafe { std::boxed::Box::from_raw(core::ptr::slice_from_raw_parts_mut(m.cpu as *mut u8, m.size)) });
            LIVE.fetch_sub(1, Ordering::Relaxed);
        }
        *m = OsalGpuMem::ZERO;
        0
    }

    unsafe extern "C" fn h_cache(_cpu: *mut c_void, _n: usize) {}

    static HOST_OSAL: OsalOps = OsalOps {
        init: None,
        shutdown: None,
        gpu_alloc: Some(h_alloc),
        gpu_free: Some(h_free),
        mmu_map: None,
        map_phys: None,
        gpu_write: None,
        gpu_read: None,
        cache_flush: Some(h_cache),
        cache_invd: Some(h_cache),
        submit: None,
        wait: None,
        completed: None,
        reg_read: None,
        reg_write: None,
        query: None,
        log: None,
        wait_vblank: None,
        present: None,
        lock: None,
        unlock: None,
        thread_id: None,
        display_busy: None,
    };

    /// The serialising guard, [`HOST_OSAL`] installed and no shadow.
    struct Rig {
        prev: *const OsalOps,
        _gl: TestGl,
    }

    impl Rig {
        fn new() -> Rig {
            let gl = TestGl::new(4, 4);
            // SAFETY: tests are serialised by the TestGl guard.
            let prev = unsafe {
                let prev = crate::osal::g_osal;
                crate::osal::osal_set_ops(&HOST_OSAL);
                prev
            };
            release_all();
            LIVE.store(0, Ordering::Relaxed);
            Rig { prev, _gl: gl }
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            release_all();
            // SAFETY: as in `new`.
            unsafe { crate::osal::osal_set_ops(self.prev) };
        }
    }

    /// (key, allocation bytes, shadow gpu, kick) of every shadow, by slot.
    fn entries() -> std::vec::Vec<(Key, usize, u64, u32)> {
        shadows().entries.iter().flatten().map(|e| (e.key, e.mem.size, e.gpu, e.kick())).collect()
    }

    /// `n` bytes of the shadow at `gpu` (an `h_alloc` address).
    fn shadow_at(gpu: u64, n: usize) -> std::vec::Vec<u8> {
        // SAFETY: an `h_alloc` address with at least `n` bytes after it.
        unsafe { core::slice::from_raw_parts(gpu as *const u8, n) }.to_vec()
    }

    /// A 64-bit pixel's bytes: (x, y) and a generation.
    fn px8(x: u32, y: u32, g: u8) -> [u8; 8] {
        [x as u8, y as u8, g, 0x80, 0xff, 0x01, !(x as u8), g ^ 0x55]
    }

    #[test]
    fn a_single_64_bit_target_keeps_its_entry_and_bytes() {
        /* pinned before the multi-target shadow: what backend.rs's draw of an
         * RGBA16I/UI, RG32I/UI or RGBA16F target reaches */
        let _rig = Rig::new();
        let (w, h, pitch_px) = (5u32, 3u32, 6u32);
        let pitch = (pitch_px * 8) as usize;
        let mut linear = std::vec![0xeeu8; pitch * h as usize];
        for y in 0..h {
            for x in 0..w {
                let at = y as usize * pitch + x as usize * 8;
                linear[at..at + 8].copy_from_slice(&px8(x, y, 1));
            }
        }
        let mut gl = GlesDraw::ZERO;
        gl.target_gpu = 0x4010_0000;
        gl.target_cpu = linear.as_mut_ptr() as *mut c_void;
        gl.target_w = w;
        gl.target_h = h;
        gl.target_pitch = pitch_px;
        gl.target_format = GLES_SURF_RGBA16F;
        assert!(wanted(&gl, true, true));
        assert!(!wanted(&gl, false, true), "more than one hardware target");
        assert!(!wanted(&gl, true, false), "through the scratch or bottom-up");
        let mut rgba8 = gl;
        rgba8.target_format = GLES_SURF_RGBA8888;
        assert!(!wanted(&rgba8, true, true), "a 32-bit target stays linear");
        assert_eq!(stride(w), 128);

        let (tiled0, untiled0) = counts();
        assert_eq!(prepare(&gl, || panic!("no finish with free slots")), 0);
        let e = entries();
        assert_eq!(e.len(), 1);
        let key = Key { gpu: 0x4010_0000, w, h, pitch: pitch as u32, bpp: 8 };
        assert_eq!(e[0].0, key);
        assert_eq!(e[0].1, 16 * 4 * 8 + BASE_ALIGN - 1, "padded 16 x 4, 8 bytes a pixel, plus the page");
        assert_eq!(e[0].3, 0);
        assert_eq!(counts(), (tiled0 + 5 * 3 * 8, untiled0));
        assert_eq!(LIVE.load(Ordering::Relaxed), 1);

        let gpu = bind(&gl, 7).expect("the shadow");
        assert_eq!(gpu, e[0].2);
        assert_eq!(gpu % BASE_ALIGN as u64, 0);
        let mut want = std::vec![0u8; 16 * 4 * 8];
        for y in 0..h {
            for x in 0..w {
                let at = tiled_offset(x, y, 16);
                want[at..at + 8].copy_from_slice(&px8(x, y, 1));
            }
        }
        assert_eq!(shadow_at(gpu, want.len()), want, "tiled in, the padding zero");

        /* the same target again while kick 7 is pending: no second tile-in */
        assert_eq!(prepare(&gl, || panic!("no finish")), 0);
        assert_eq!(counts(), (tiled0 + 120, untiled0));

        /* the GPU renders generation 2; the kick completes */
        for y in 0..h {
            for x in 0..w {
                let at = tiled_offset(x, y, 16);
                // SAFETY: inside the shadow.
                unsafe { core::ptr::copy_nonoverlapping(px8(x, y, 2).as_ptr(), (gpu as *mut u8).add(at), 8) };
            }
        }
        retire(|k| k == 7);
        assert_eq!(counts().1, untiled0, "still pending");
        retire(|_| false);
        assert_eq!(counts(), (tiled0 + 120, untiled0 + 120));
        assert_eq!(entries()[0].3, 0);
        for y in 0..h {
            for x in 0..pitch_px {
                let at = y as usize * pitch + x as usize * 8;
                let want = if x < w { px8(x, y, 2) } else { [0xee; 8] };
                assert_eq!(linear[at..at + 8], want, "({x},{y})");
            }
        }

        /* the storage is the newer copy again: the next prepare tiles in */
        assert_eq!(prepare(&gl, || panic!("no finish")), 0);
        assert_eq!(counts().0, tiled0 + 240);

        /* specified again at another size: the old shadow goes */
        gl.target_w = 2;
        assert_eq!(prepare(&gl, || panic!("no finish")), 0);
        let e = entries();
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].0.w, 2);
        assert_eq!(e[0].1, 16 * 4 * 8 + BASE_ALIGN - 1);
        assert_eq!(LIVE.load(Ordering::Relaxed), 1);

        released(0x4010_0000, 64);
        assert!(entries().is_empty());
        assert_eq!(LIVE.load(Ordering::Relaxed), 0);
    }

    /// A target over `buf`, `pitch` bytes a row.
    fn target(buf: &mut [u8], gpu: u64, (w, h): (u32, u32), pitch: u32, bpp: u32) -> Target {
        assert!(buf.len() >= (pitch * (h - 1) + w * bpp) as usize);
        Target { gpu, cpu: buf.as_mut_ptr(), w, h, pitch, bpp }
    }

    #[test]
    fn one_64_bit_target_described_as_targets_is_the_one_target_path() {
        let _rig = Rig::new();
        let mut linear = std::vec![0u8; 6 * 8 * 3];
        let mut gl = GlesDraw::ZERO;
        gl.target_gpu = 0x4020_0000;
        gl.target_cpu = linear.as_mut_ptr() as *mut c_void;
        gl.target_w = 5;
        gl.target_h = 3;
        gl.target_pitch = 6;
        gl.target_format = GLES_SURF_RGBA16F;
        let t = target_of(&gl);
        assert_eq!(t, target(&mut linear, 0x4020_0000, (5, 3), 48, 8));
        for straight in [true, false] {
            assert_eq!(wanted_targets(&[t], straight), wanted(&gl, true, straight));
        }
        assert_eq!(target_key(&t), key_of(&gl));
        assert_eq!(target_stride(&t), stride(5));

        assert_eq!(prepare_targets(&[t], || panic!("no finish")), 0);
        let e = entries();
        assert_eq!(e.len(), 1);
        assert_eq!((e[0].0, e[0].1), (key_of(&gl), 16 * 4 * 8 + BASE_ALIGN - 1));
        let b = bind_targets(&[t], 9).expect("bound");
        assert_eq!(b[0], Bound { gpu: e[0].2, plane1: 0, stride: 128 });
        assert_eq!(b[1..], [Bound::ZERO; MAX_TARGETS - 1]);
        /* the one-target functions find the same shadow */
        assert_eq!(bind(&gl, 9), Some(e[0].2));
        assert_eq!(prepare(&gl, || panic!("no finish")), 0);
        assert_eq!(entries().len(), 1);
    }

    /// RGBA32I/UI words: negative, above 2^24, and both ends.
    fn wide(x: u32, y: u32, g: u32) -> [u8; 16] {
        let words = [
            i32::MIN as u32 + x + g,
            (-(1 + x as i32 + 16 * y as i32)) as u32,
            0x0100_0001 + (g << 26) + x,
            0xffff_ff01 - y - g,
        ];
        let mut b = [0u8; 16];
        for (i, v) in words.iter().enumerate() {
            b[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    #[test]
    fn a_128_bit_target_round_trips_through_its_two_planes_bit_for_bit() {
        let _rig = Rig::new();
        let (w, h, pitch) = (5u32, 3u32, 6 * 16usize);
        let mut linear = std::vec![0xeeu8; pitch * h as usize];
        let image = |g: u32| {
            let mut v = std::vec![0xeeu8; pitch * h as usize];
            for y in 0..h {
                for x in 0..w {
                    let at = y as usize * pitch + x as usize * 16;
                    v[at..at + 16].copy_from_slice(&wide(x, y, g));
                }
            }
            v
        };
        linear.copy_from_slice(&image(1));
        let t = target(&mut linear, 0x4030_0000, (w, h), pitch as u32, 16);
        assert!(wanted_targets(&[t], true));
        assert_eq!(target_stride(&t), 16 * 8, "one 8-byte plane a row");

        let (tiled0, untiled0) = counts();
        assert_eq!(prepare_targets(&[t], || panic!("no finish")), 0);
        let e = entries();
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].0, Key { gpu: 0x4030_0000, w, h, pitch: pitch as u32, bpp: 16 });
        assert_eq!(e[0].1, planes_bytes(16, 4) + BASE_ALIGN - 1);
        assert_eq!(counts(), (tiled0 + 5 * 3 * 16, untiled0));

        let b = bind_targets(&[t], 3).expect("bound")[0];
        assert_eq!(b, Bound { gpu: e[0].2, plane1: e[0].2 + plane_offset(16, 4) as u64, stride: 128 });
        assert_eq!(plane_offset(16, 4), 512);
        /* R,G in plane 0 and B,A in plane 1, at the same tiled place */
        let planes = shadow_at(b.gpu, planes_bytes(16, 4));
        for (x, y) in [(0, 0), (4, 0), (1, 2), (4, 2)] {
            let at = tiled_offset(x, y, 16);
            assert_eq!(planes[at..at + 8], wide(x, y, 1)[..8], "R,G of ({x},{y})");
            assert_eq!(planes[512 + at..512 + at + 8], wide(x, y, 1)[8..], "B,A of ({x},{y})");
        }

        /* the GPU renders generation 2 into both planes; the kick completes */
        let mut drawn = std::vec![0u8; planes_bytes(16, 4)];
        split_planes(&image(2), pitch, (w, h), 16, &mut drawn);
        // SAFETY: the shadow holds `planes_bytes(16, 4)` bytes.
        unsafe { core::ptr::copy_nonoverlapping(drawn.as_ptr(), b.gpu as *mut u8, drawn.len()) };
        retire(|_| false);
        assert_eq!(counts(), (tiled0 + 240, untiled0 + 240));
        assert_eq!(linear, image(2), "every word back, the pitch padding untouched");
        let px = &linear[pitch + 3 * 16..][..16];
        assert_eq!(i32::from_le_bytes(px[4..8].try_into().unwrap()), -(1 + 3 + 16));
        assert_eq!(u32::from_le_bytes(px[8..12].try_into().unwrap()), 0x0100_0001 + (2 << 26) + 3);

        /* in and straight back out, nothing drawn: bit for bit */
        assert_eq!(prepare_targets(&[t], || panic!("no finish")), 0);
        bind_targets(&[t], 4).expect("bound");
        retire(|_| false);
        assert_eq!(linear, image(2));
    }

    #[test]
    fn a_mixed_128_and_32_bit_draw_gets_one_shadow_a_target() {
        let _rig = Rig::new();
        let (w, h) = (21u32, 10u32);
        let mut wide_buf = std::vec![0u8; (w * 16 * h) as usize];
        let mut rgba8 = std::vec![0u8; (w * 4 * h) as usize];
        for (i, b) in rgba8.iter_mut().enumerate() {
            *b = i as u8 ^ 0x3c;
        }
        let before8 = rgba8.clone();
        let t128 = target(&mut wide_buf, 0x4040_0000, (w, h), w * 16, 16);
        let t32 = target(&mut rgba8, 0x4050_0000, (w, h), w * 4, 4);

        assert!(wanted_targets(&[t128, t32], true));
        assert!(wanted_targets(&[t32, t128], true));
        assert!(!wanted_targets(&[t128, t32], false));
        assert!(!wanted_targets(&[t32], true), "32-bit targets alone stay linear");
        assert!(!wanted_targets(&[t32, t32], true));
        assert!(!wanted_targets(&[t128; MAX_TARGETS + 1], true));
        assert!(!wanted_targets(&[t128, Target { cpu: core::ptr::null_mut(), ..t32 }], true));
        assert!(!wanted_targets(&[], true));

        assert_eq!(prepare_targets(&[t128, t32], || panic!("no finish")), 0);
        let e = entries();
        assert_eq!(e.len(), 2, "one shadow a target");
        let (pw, ph) = padded(w, h);
        assert_eq!((pw, ph), (32, 12));
        assert_eq!(e[0].0.bpp, 16);
        assert_eq!(e[0].1, planes_bytes(32, 12) + BASE_ALIGN - 1);
        assert_eq!(planes_bytes(32, 12), 32 * 12 * 16);
        assert_eq!(e[1].0.bpp, 4);
        assert_eq!(e[1].1, 32 * 12 * 4 + BASE_ALIGN - 1);

        let b = bind_targets(&[t128, t32], 5).expect("bound");
        assert_eq!(b[0], Bound { gpu: e[0].2, plane1: e[0].2 + 32 * 12 * 8, stride: 32 * 8 });
        assert_eq!(b[1], Bound { gpu: e[1].2, plane1: 0, stride: 32 * 4 });
        assert!(b[..2].iter().all(|b| b.gpu % BASE_ALIGN as u64 == 0));
        assert!(entries().iter().all(|e| e.3 == 5), "both written by the kick");
        /* the 32-bit shadow is 4x4-tiled at 4 bytes a pixel */
        let s32 = shadow_at(b[1].gpu, 32 * 12 * 4);
        for (x, y) in [(0u32, 0u32), (3, 1), (4, 0), (20, 9)] {
            let at = tiled_offset_bpp(x, y, 32, 4);
            let from = (y * w * 4 + x * 4) as usize;
            assert_eq!(s32[at..at + 4], before8[from..from + 4], "({x},{y})");
        }
        assert_eq!(tiled_offset_bpp(4, 0, 32, 4), 64);
        assert_eq!(tiled_offset_bpp(0, 4, 32, 4), 8 * 64);

        /* the GPU leaves both alone; both come back as they were */
        retire(|_| false);
        assert_eq!(rgba8, before8);
        assert!(wide_buf.iter().all(|&b| b == 0));
        assert!(entries().iter().all(|e| e.3 == 0));

        /* a target missing its shadow binds nothing */
        let other = Target { gpu: 0x4060_0000, ..t32 };
        assert_eq!(bind_targets(&[t128, other], 6), None);
        assert!(entries().iter().all(|e| e.3 == 0), "nothing marked");
    }

    #[test]
    fn a_draw_does_not_push_out_its_own_shadows() {
        let _rig = Rig::new();
        let mut bufs = std::vec![std::vec![0u8; 4 * 8 * 4]; ENTRIES - 2 + 3];
        let mut all: std::vec::Vec<Target> = bufs
            .iter_mut()
            .enumerate()
            .map(|(i, b)| target(b, 0x4100_0000 + 0x1_0000 * i as u64, (4, 4), 32, 8))
            .collect();
        let draw = all.split_off(ENTRIES - 2);
        /* six shadows a pending kick renders into, two slots free */
        for (i, t) in all.iter().enumerate() {
            assert_eq!(prepare_targets(&[*t], || panic!("no finish")), 0);
            assert!(bind_targets(&[*t], 10 + i as u32).is_some());
        }
        let finishes = core::cell::Cell::new(0);
        let finish = || {
            finishes.set(finishes.get() + 1);
            retire(|_| false);
            0
        };
        /* the third target finds no free slot but the draw's own two */
        assert_eq!(prepare_targets(&draw, finish), 0);
        assert_eq!(finishes.get(), 1, "the kicks complete instead");
        let b = bind_targets(&draw, 20).expect("all three shadows");
        assert!(b[..3].iter().all(|b| b.gpu != 0));
        assert_eq!(entries().len(), ENTRIES);
        assert_eq!(LIVE.load(Ordering::Relaxed), ENTRIES);
    }

    #[test]
    fn padded_and_stride_follow_layout_for_4_and_8_bytes() {
        for (w, h) in [(1u32, 1u32), (8, 8), (16, 4), (21, 10), (55, 63), (64, 64)] {
            assert_eq!(
                padded(w, h),
                (layout::aligned_width(Layout::Tiled, w, true), layout::aligned_height(Layout::Tiled, h, true))
            );
            for bpp in [4u32, 8] {
                assert_eq!(stride_bpp(w, bpp), layout::stride(Layout::Tiled, w, bpp, true));
                assert_eq!(stride_bpp(w, bpp), padded(w, h).0 * bpp);
            }
            assert_eq!(stride(w), stride_bpp(w, 8));
        }
        assert_eq!(stride_bpp(21, 4), 128);
        assert_eq!(stride_bpp(21, 8), 256);
    }

    #[test]
    fn a_shadow_moves_between_the_three_states() {
        let _rig = Rig::new();
        let mut buf = std::vec![0u8; 16 * 4 * 8];
        let t = target(&mut buf, 0x4200_0000, (16, 4), 128, 8);
        assert_eq!(state_of(t.gpu), None);
        assert!(!mark_storage_newer(t.gpu) && !mark_in_step(t.gpu) && !in_step(t.gpu));
        assert_eq!(prepare_targets(&[t], || panic!("no finish")), 0);
        assert_eq!(state_of(t.gpu), Some(State::StorageNewer));
        bind_targets(&[t], 5).expect("bound");
        assert_eq!(state_of(t.gpu), Some(State::ShadowNewer(5)));
        assert!(!in_step(t.gpu));
        assert!(mark_in_step(t.gpu));
        assert!(in_step(t.gpu));
        /* an in-step shadow is neither tiled in again nor written back */
        let tiled = counts().0;
        assert_eq!(prepare_targets(&[t], || panic!("no finish")), 0);
        assert_eq!(counts().0, tiled);
        retire(|_| false);
        assert_eq!(state_of(t.gpu), Some(State::InStep));
        assert!(mark_storage_newer(t.gpu));
        assert_eq!(state_of(t.gpu), Some(State::StorageNewer));
        /* the next prepare tiles the storage in */
        assert_eq!(prepare_targets(&[t], || panic!("no finish")), 0);
        assert_eq!(counts().0, tiled + 16 * 4 * 8);
        /* a 64-bit shadow retires by untiling */
        bind_targets(&[t], 6).expect("bound");
        retire(|k| k == 6);
        assert_eq!(state_of(t.gpu), Some(State::ShadowNewer(6)));
        let untiled = counts().1;
        retire(|_| false);
        assert_eq!(state_of(t.gpu), Some(State::StorageNewer));
        assert_eq!(counts().1, untiled + 16 * 4 * 8);
    }

    #[test]
    fn retire_does_not_untile_a_4_byte_entry() {
        let _rig = Rig::new();
        let mut buf = std::vec![0x11u8; 16 * 4 * 4];
        let t = target(&mut buf, 0x4300_0000, (16, 4), 64, 4);
        assert_eq!(prepare_targets(&[t], || panic!("no finish")), 0);
        let b = bind_targets(&[t], 8).expect("bound")[0];
        assert_eq!(b.stride, 64);
        // SAFETY: the shadow holds 16 * 4 * 4 bytes.
        unsafe { core::ptr::write_bytes(b.gpu as *mut u8, 0xcc, 16 * 4 * 4) };
        let untiled = counts().1;
        retire(|_| false);
        assert!(buf.iter().all(|&x| x == 0x11), "the storage is left as the RS stream wrote it");
        assert_eq!(counts().1, untiled);
        assert_eq!(state_of(t.gpu), Some(State::StorageNewer));
    }

    #[test]
    fn a_4_byte_target_is_still_not_wanted_on_its_own() {
        let mut buf = std::vec![0u8; 4 * 4 * 4];
        let mut gl = GlesDraw::ZERO;
        gl.target_gpu = 0x4400_0000;
        gl.target_cpu = buf.as_mut_ptr() as *mut c_void;
        gl.target_w = 4;
        gl.target_h = 4;
        gl.target_pitch = 4;
        gl.target_format = GLES_SURF_RGBA8888;
        assert!(!wanted(&gl, true, true));
    }
}
