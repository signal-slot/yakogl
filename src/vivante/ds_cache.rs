// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The D24S8 surfaces the pixel engine tests and writes depth and stencil
//! in, one per (depth object, stencil object) pair, with the depth and the
//! stencil of each GL object followed plane by plane.
//!
//! GL gives every framebuffer its own depth and stencil buffers: two
//! framebuffers sharing a colour buffer keep separate depth (dEQP
//! fbo.render.shared_colorbuffer*), two sharing a depth renderbuffer share
//! it (shared_depthbuffer*), and a renderbuffer's contents persist across
//! framebuffer switches (ES 2.0 4.4.3). This part has no separate stencil
//! surface: the pixel engine reads and writes one D24S8 word a pixel, the
//! depth in bits 31..8 and the stencil in bits 7..0 (Mesa's resolve-engine
//! clear word for this part, `tools/vivante/README.md` "mode 8": 0xffffff00
//! is depth 1.0, stencil 0).
//!
//! So a surface is keyed by the pair the draw attaches ([`key_of`]: both
//! objects and their generations, and the multisample scale), and each of
//! its two planes says whether it holds its object's latest contents
//! ([`Plane`]). A depth object is attached with more than one stencil object
//! over time (or a stencil with more than one depth): its contents live in
//! exactly one surface at a time, and when a draw needs them in another
//! surface they are copied there, plane by plane, with a word mask on the
//! CPU ([`copy_plane`]): the GPU is idle for it and the surfaces have no
//! tile status (`emit_pe_state`: TS mode 0), so memory holds every pixel.
//! A plane that holds a whole clear's value only is written back by a clear
//! of that plane ([`Selected::restore`]), which the backend records before
//! the draw.
//!
//! A surface that holds a live plane is never given to another pair: with
//! every surface holding one, the draw is refused (src/ds_retention.rs).

use super::backend::{g, heap_data};
use crate::backend::GlesDraw;
use crate::ds_retention::{self, Parked, Plane, DEPTH, STENCIL};
use crate::osal::{self, OsalGpuMem, OSAL_MEM_DATA};
use crate::util::Global;

/// Surfaces kept at once: the default framebuffer's and the FBOs'
/// ([`ds_retention::DS_SLOTS`], shared by every backend).
pub const ENTRIES: usize = ds_retention::DS_SLOTS;

/// Parked planes: two a surface.
const PARKED: usize = 2 * ENTRIES;

/// The D24S8 word's depth bits and stencil bits (see the module comment).
const PLANE_MASK: [u32; 2] = [0xffff_ff00, 0x0000_00ff];

/// Which depth/stencil storage a draw uses: a new value of any field is a
/// different surface (or the same one re-specified, whose contents are gone).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct DsKey {
    pub depth_id: u32,
    pub depth_gen: u32,
    pub stencil_id: u32,
    pub stencil_gen: u32,
    /// 1, or 2 for a 4x draw through the scratch surface (its depth is
    /// twice the size on each axis).
    pub scale: u32,
}

/// One plane of one GL object: whose, which, at what scale. The unit the
/// contents follow from surface to surface.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct PlaneKey {
    id: u32,
    gen: u32,
    scale: u32,
    plane: u8,
}

impl DsKey {
    /// Plane `p`'s identity; `None` when that attachment point is empty.
    fn plane(&self, p: usize) -> Option<PlaneKey> {
        let (id, gen) = if p == DEPTH { (self.depth_id, self.depth_gen) } else { (self.stencil_id, self.stencil_gen) };
        (id != 0).then_some(PlaneKey { id, gen, scale: self.scale, plane: p as u8 })
    }
}

struct Surface {
    key: DsKey,
    mem: OsalGpuMem,
    /// Bytes a row, and rows: 64-pixel aligned both ways (a supertile).
    stride: u32,
    rows: u32,
    /// The kick that last used it: not freed until that one completed.
    used_by: u32,
    stamp: u32,
    /// What each plane holds for its object: Live or a clear value when
    /// this surface has the object's latest contents, Undefined otherwise.
    planes: [Plane; 2],
}

impl Surface {
    /// Holds contents that exist nowhere else: never given to another pair.
    fn holds_contents(&self) -> bool {
        self.planes.iter().any(|p| p.is_live())
    }

    fn cleared_planes(&self) -> usize {
        self.planes.iter().filter(|p| p.cleared_value().is_some()).count()
    }
}

struct Cache {
    entries: [Option<Surface>; ENTRIES],
    clock: u32,
    /// Planes whose surface went to another pair while they held a clear
    /// value only (src/ds_retention.rs).
    parked: Parked<PlaneKey, Plane, PARKED>,
    /// For the line a run ends with ([`report`]); they survive
    /// [`release_all`], so a process-wide line counts the whole run.
    counts: Counts,
}

/// What [`report`] prints: the most surfaces and bytes held at once, the
/// surfaces given up for another pair's, and the draws refused.
#[derive(Clone, Copy)]
struct Counts {
    peak: u32,
    bytes_peak: usize,
    given_up: u32,
    /// Every surface held a live plane ([`Victim::AllLive`]).
    full: u32,
    /// No surface could be had for another reason: its memory was not
    /// there, or every one that could go was in use by a pending kick.
    no_surface: u32,
}

static CACHE: Global<Cache> = Global::new(Cache {
    entries: [const { None }; ENTRIES],
    clock: 0,
    parked: Parked::new(PlaneKey { id: 0, gen: 0, scale: 0, plane: 0 }, Plane::UNDEFINED),
    counts: Counts { peak: 0, bytes_peak: 0, given_up: 0, full: 0, no_surface: 0 },
});

/// After an allocation: the surfaces held now, against the peaks.
fn note_peak(c: &mut Cache) {
    let (n, bytes) = c.entries.iter().flatten().fold((0u32, 0usize), |(n, b), e| (n + 1, b.saturating_add(e.mem.size)));
    c.counts.peak = c.counts.peak.max(n);
    c.counts.bytes_peak = c.counts.bytes_peak.max(bytes);
}

/// The depth/stencil line a run ends with, the GC7000's counterpart of the
/// PowerVR's (`pvr_backend_ds_report`): the most surfaces held at once of
/// [`ENTRIES`], the most bytes, the surfaces given up for another pair's,
/// and the draws refused (all live: every surface held a plane GL keeps; no
/// surface: memory, or every candidate in use by a pending kick). A run
/// that never allocated a surface says so instead of a peak of 0.
pub fn report(out: &mut impl core::fmt::Write) -> core::fmt::Result {
    let k = cache().counts;
    if k.peak == 0 {
        out.write_str("ds surfaces: none allocated")?;
    } else {
        write!(out, "ds surfaces peak {}/{}, ds bytes peak {}", k.peak, ENTRIES, k.bytes_peak)?;
    }
    write!(
        out,
        ", ds given up {}, ds refused {} (all live {}, no surface {})",
        k.given_up,
        k.full.wrapping_add(k.no_surface),
        k.full,
        k.no_surface
    )
}

/// [`report`]'s line into `buf` (`cap` bytes, NUL-terminated, truncated to
/// fit), for the OSAL's record at close.
///
/// # Safety
/// `buf` is null or writable for `cap` bytes.
#[no_mangle]
pub unsafe extern "C" fn gc_backend_ds_report(buf: *mut core::ffi::c_char, cap: usize) {
    // SAFETY: the caller's contract.
    let mut w = unsafe { crate::util::CStrWriter::new(buf, cap) };
    /* CStrWriter never fails: it truncates */
    let _ = report(&mut w);
}

/// The counts back to zero (tests that read the line from a clean start).
#[cfg(test)]
pub fn reset_counts() {
    cache().counts = Counts { peak: 0, bytes_peak: 0, given_up: 0, full: 0, no_surface: 0 };
}

fn cache() -> &'static mut Cache {
    // SAFETY: single-threaded by contract; each entry point takes it once.
    unsafe { CACHE.get() }
}

#[cfg(test)]
static KEY_OVERRIDE: Global<Option<DsKey>> = Global::new(None);

/// The key the tests draw with, in place of the GL core's.
#[cfg(test)]
pub fn set_key_override(k: Option<DsKey>) {
    // SAFETY: tests are serialised by the TestGl guard.
    unsafe { *KEY_OVERRIDE.get() = k };
}

/// The draw's depth/stencil pair (`GlesDraw::depth_id` and friends): id 0 is
/// "no attachment", 0x8000_0000 | page the window surface's (on both
/// points), otherwise the object's serial. A draw with neither attachment
/// still needs a surface for the pixel engine to address (the GL core has
/// turned both tests off for it), and all of those share key 0.
///
/// Both objects are in the key. Keying by the depth object alone (the rule
/// until 2026-09-29) carried a separate stencil renderbuffer's bits over to
/// the object that replaced it, and split a stencil shared by two depth
/// objects between their surfaces; keying by both without moving the
/// planes (bf6a0136) lost a shared depth (dEQP shared_depthbuffer.*_
/// depth_component16_stencil_index8). The planes now follow their objects
/// ([`select`]).
pub fn key_of(gl: &GlesDraw, scale: u32) -> DsKey {
    #[cfg(test)]
    {
        // SAFETY: as above.
        if let Some(k) = unsafe { *KEY_OVERRIDE.get() } {
            return DsKey { scale, ..k };
        }
    }
    DsKey {
        depth_id: gl.depth_id,
        depth_gen: gl.depth_gen,
        stencil_id: gl.stencil_id,
        stencil_gen: gl.stencil_gen,
        scale,
    }
}

const fn align64(v: u32) -> u32 {
    (v + 63) & !63
}

/// What [`select`] made current.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Selected {
    /// The surface was (re)allocated: its memory holds nothing yet.
    pub fresh: bool,
    /// Planes that come back from a whole clear's value only (depth as `f32`
    /// bits, stencil as the value): the backend records a clear of just
    /// those planes before the draw.
    pub restore: [Option<u32>; 2],
}

/// The surface for `key`, at least `w` x `h` pixels, made current: the
/// backend's `depth` / `depth_stride` / `depth_h` mirror it (the depth-guard
/// canary reads the buffer through them), with each of the pair's planes
/// holding its object's latest contents: copied from the surface that had
/// them, or to be restored from a clear value.
///
/// A new pair takes an empty slot, or the least recently used surface that
/// holds no live plane and that no pending kick uses (its clear values
/// parked). `Err(-ENOMEM)` when every surface holds a live plane: the draw
/// is refused ([`ds_retention::refuse`]). `Err(-1)` when the memory is not
/// there, or every surface that could go is still in use by a pending kick.
/// `finish` submits the open frame and waits for every kick: called before
/// a plane is copied.
///
/// `seed`: depth texels the draw carries (`GlesDraw::depth_texels`): they
/// become the surface's depth plane ([`seed_depth`]), and no depth is
/// restored from a clear value.
#[allow(clippy::too_many_arguments)]
pub fn select(
    key: DsKey,
    w: u32,
    h: u32,
    guard: usize,
    kick: u32,
    seed: Option<ds_retention::DepthTexels>,
    pending: impl Fn(u32) -> bool,
    mut finish: impl FnMut() -> i32,
) -> Result<Selected, i32> {
    let c = cache();
    c.clock = c.clock.wrapping_add(1);
    let clock = c.clock;
    let stride = align64(w) * 4;
    let rows = align64(h);

    let mut fresh = false;
    let slot = match c.entries.iter().position(|e| e.as_ref().is_some_and(|e| e.key == key)) {
        Some(i) => {
            let e = c.entries[i].as_mut().expect("found above");
            if stride > e.stride || rows > e.rows {
                /* grown: the old contents cannot be kept at a new stride */
                let (s, r) = (stride.max(e.stride), rows.max(e.rows));
                osal::gpu_free(&mut e.mem);
                if alloc(&mut e.mem, s, r, guard) != 0 {
                    c.entries[i] = None;
                    c.counts.no_surface = c.counts.no_surface.wrapping_add(1);
                    return Err(-1);
                }
                (e.stride, e.rows, e.planes) = (s, r, [Plane::UNDEFINED; 2]);
                note_peak(c);
                fresh = true;
            }
            i
        }
        None => {
            let i = match victim(c, &pending) {
                Victim::Slot(i) => i,
                Victim::AllPending => {
                    crate::glog_once!("vivante: every depth/stencil surface is in use by a pending kick");
                    c.counts.no_surface = c.counts.no_surface.wrapping_add(1);
                    return Err(-1);
                }
                Victim::AllLive => {
                    let who = if key.depth_id != 0 { key.depth_id } else { key.stencil_id };
                    c.counts.full = c.counts.full.wrapping_add(1);
                    return Err(ds_retention::refuse("vivante", who, ENTRIES));
                }
            };
            if let Some(mut old) = c.entries[i].take() {
                park(&mut c.parked, &old);
                osal::gpu_free(&mut old.mem);
                c.counts.given_up = c.counts.given_up.wrapping_add(1);
            }
            let mut mem = OsalGpuMem::ZERO;
            if alloc(&mut mem, stride, rows, guard) != 0 {
                c.counts.no_surface = c.counts.no_surface.wrapping_add(1);
                return Err(-1);
            }
            c.entries[i] =
                Some(Surface { key, mem, stride, rows, used_by: 0, stamp: 0, planes: [Plane::UNDEFINED; 2] });
            note_peak(c);
            fresh = true;
            i
        }
    };
    let mut restore = [None; 2];
    for p in [DEPTH, STENCIL] {
        let Some(pk) = key.plane(p) else { continue };
        if p == DEPTH && seed.is_some() {
            /* the texels replace the depth wherever it is: nothing is
             * copied or restored (a copy from a surface of another stride
             * would even refuse the draw), and no other surface or parked
             * entry holds it any more */
            for (j, e) in c.entries.iter_mut().enumerate() {
                if let Some(e) = e.as_mut().filter(|e| j != slot && e.key.plane(p) == Some(pk)) {
                    e.planes[p] = Plane::UNDEFINED;
                }
            }
            let _ = c.parked.take(pk);
            continue;
        }
        if c.entries[slot].as_ref().is_some_and(|e| e.planes[p] != Plane::UNDEFINED) {
            continue; /* this surface has the latest contents */
        }
        let holder = (0..ENTRIES).find(|&j| {
            j != slot
                && c.entries[j].as_ref().is_some_and(|e| e.key.plane(p) == Some(pk) && e.planes[p] != Plane::UNDEFINED)
        });
        let now = match holder {
            Some(j) => {
                let from = c.entries[j].as_ref().map_or(Plane::UNDEFINED, |e| e.planes[p]);
                if from.is_live() {
                    copy_plane(c, j, slot, p, guard, &mut finish)?;
                } else {
                    restore[p] = from.cleared_value();
                }
                if let Some(e) = c.entries[j].as_mut() {
                    e.planes[p] = Plane::UNDEFINED;
                }
                from
            }
            None => match c.parked.take(pk) {
                Some([parked, _]) => {
                    restore[p] = parked.cleared_value();
                    parked
                }
                None => Plane::UNDEFINED,
            },
        };
        if let Some(e) = c.entries[slot].as_mut() {
            e.planes[p] = now;
        }
    }
    if let Some(t) = seed {
        seed_depth(c, slot, t, guard, &pending, &mut finish)?;
        restore[DEPTH] = None;
    }
    ds_retention::admitted();
    let Some(e) = c.entries[slot].as_mut() else { return Err(-1) };
    e.used_by = kick;
    e.stamp = clock;
    let g = g();
    g.depth = e.mem;
    g.depth_stride = e.stride;
    g.depth_w = w;
    g.depth_h = e.rows;
    g.depth_surface_off = guard as u32;
    Ok(Selected { fresh, restore })
}

/// What a recorded draw left in the planes of `key`'s surface: its whole
/// clears (before its geometry) and what its state let it write.
pub fn drawn(key: DsKey, writes: [bool; 2], clears: [Option<u32>; 2]) {
    let c = cache();
    let Some(e) = c.entries.iter_mut().flatten().find(|e| e.key == key) else { return };
    for p in [DEPTH, STENCIL] {
        if key.plane(p).is_some() {
            e.planes[p] = e.planes[p].after(clears[p], writes[p]);
        }
    }
}

/// The contents of GL object `id` are gone (`attachment_gone`): its planes
/// are Undefined everywhere and its parked clear values forgotten. A
/// surface of it left holding nothing, that no pending kick uses, is freed.
pub fn gone(id: u32, pending: impl Fn(u32) -> bool) {
    let c = cache();
    c.parked.forget(id);
    for slot in c.entries.iter_mut() {
        let Some(e) = slot.as_mut() else { continue };
        for p in [DEPTH, STENCIL] {
            if e.key.plane(p).is_some_and(|pk| pk.id == id) {
                e.planes[p] = Plane::UNDEFINED;
            }
        }
        let names = [e.key.depth_id, e.key.stencil_id].contains(&id);
        if names && e.planes == [Plane::UNDEFINED; 2] && (e.used_by == 0 || !pending(e.used_by)) {
            if let Some(mut old) = slot.take() {
                osal::gpu_free(&mut old.mem);
            }
        }
    }
}

/// Copy plane `p` of surface `from` into surface `to`, leaving `to`'s other
/// plane as it is: a masked word copy on the CPU, after `finish` has made
/// the GPU idle. Two surfaces of one row stride have one layout (rows and
/// supertiles both 64-pixel aligned, `align64`), so the shorter one's rows
/// are copied word for word. Another stride is a layout this code does not
/// remap: the draw is refused with -ENOTSUP and a line naming it.
fn copy_plane(
    c: &mut Cache,
    from: usize,
    to: usize,
    p: usize,
    guard: usize,
    finish: &mut impl FnMut() -> i32,
) -> Result<(), i32> {
    let (Some(s), Some(d)) = (c.entries[from].as_ref(), c.entries[to].as_ref()) else { return Err(-1) };
    if s.stride != d.stride {
        crate::glog_once!(
            "vivante: the {} of object 0x{:x} is in a surface of another row stride ({} against {} bytes): not \
             copied, the draw is refused",
            if p == DEPTH { "depth" } else { "stencil" },
            if p == DEPTH { d.key.depth_id } else { d.key.stencil_id },
            s.stride,
            d.stride
        );
        return Err(-crate::util::ENOTSUP);
    }
    let (src, dst) = (s.mem, d.mem);
    let bytes = s.stride as usize * s.rows.min(d.rows) as usize;
    let rc = finish();
    if rc != 0 {
        return Err(rc);
    }
    // SAFETY: two live allocations of this cache (distinct surfaces), each
    // guard + stride * rows + guard bytes; the GPU is idle (finish above).
    let (Some(sb), Some(db)) = (unsafe { src.bytes_mut() }, unsafe { dst.bytes_mut() }) else {
        crate::glog_once!("vivante: a depth/stencil surface has no CPU mapping: the plane is not copied");
        return Err(-1);
    };
    // SAFETY: the whole CPU mappings, as above.
    unsafe {
        osal::cache_invd(src.cpu, src.size);
        osal::cache_invd(dst.cpu, dst.size);
    }
    let mask = PLANE_MASK[p];
    let (sb, db) = (&sb[guard..guard + bytes], &mut db[guard..guard + bytes]);
    for (sw, dw) in sb.as_chunks::<4>().0.iter().zip(db.as_chunks_mut::<4>().0.iter_mut()) {
        let (sv, dv) = (u32::from_ne_bytes(*sw), u32::from_ne_bytes(*dw));
        *dw = ((dv & !mask) | (sv & mask)).to_ne_bytes();
    }
    // SAFETY: as above.
    unsafe { osal::cache_flush(dst.cpu, dst.size) };
    Ok(())
}

/// Make texels `t` the depth plane of surface `slot` (a draw's
/// `GlesDraw::depth_texels`: the CPU wrote the depth texture since the
/// plane matched it), the reverse of [`resolve_depth`]: each word's bits
/// 31..8 take the texel's 24-bit depth ([`ds_retention::depth_unorm24`]),
/// row-linear at the row stride, row 0 at GL's y 0, its stencil bits kept.
/// Pixels past the texels keep what they held. The plane is then contents
/// GL keeps. A pending kick that used the surface is waited for first
/// (`finish`; the GL core waited for every kick that used the plane, so
/// this is a guard). No tile status (TS mode 0): the pixel engine reads
/// memory, so the next draw tests against the texels.
///
/// `Err(-ENOTSUP)`, named once in the log, for a 4x surface (scale 2, a
/// depth texture is single-sampled) or one with no CPU view: the draw is
/// refused rather than tested against depth the texels no longer hold.
fn seed_depth(
    c: &mut Cache,
    slot: usize,
    t: ds_retention::DepthTexels,
    guard: usize,
    pending: &impl Fn(u32) -> bool,
    finish: &mut impl FnMut() -> i32,
) -> Result<(), i32> {
    let Some(e) = c.entries[slot].as_ref() else { return Err(-1) };
    let busy = e.used_by != 0 && pending(e.used_by);
    let (scale, id) = (e.key.scale, e.key.depth_id);
    if busy {
        let rc = finish();
        if rc != 0 {
            return Err(rc);
        }
    }
    let Some(e) = c.entries[slot].as_mut() else { return Err(-1) };
    let bytes = e.stride as usize * e.rows as usize;
    // SAFETY: a live allocation of this cache, guard + stride * rows + guard
    // bytes, which no kick uses (waited for above).
    let all = if scale == 1 { unsafe { e.mem.bytes_mut() } } else { None };
    let Some(px) = all.and_then(|a| a.get_mut(guard..guard + bytes)) else {
        crate::glog_once!(
            "vivante: draw refused: the depth the CPU wrote into object 0x{:x} cannot go into its surface \
             (scale {}, or no CPU view)",
            id,
            scale
        );
        return Err(-crate::util::ENOTSUP);
    };
    // SAFETY: the whole CPU mapping.
    unsafe { osal::cache_invd(e.mem.cpu, e.mem.size) };
    let stride = e.stride as usize;
    let (cols, rows) = ((t.w as usize).min(stride / 4), (t.h as usize).min(e.rows as usize));
    for y in 0..rows {
        for (x, word) in px[y * stride..y * stride + cols * 4].as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let old = u32::from_ne_bytes(*word);
            let depth = t.unorm24(x as u32, y as u32) << 8;
            *word = ((old & PLANE_MASK[STENCIL]) | depth).to_ne_bytes();
        }
    }
    // SAFETY: as above.
    unsafe { osal::cache_flush(e.mem.cpu, e.mem.size) };
    e.planes[DEPTH] = Plane::LIVE;
    Ok(())
}

/// Write the depth plane of GL object `id` (a depth texture) into `w` x `h`
/// floats at `dst`, `pitch` a row (`YakoglBackend::depth_resolve`).
///
/// Live: the surface's D24S8 words (depth in bits 31..8), ROW-LINEAR at the
/// surface's row stride, row 0 at GL's y 0. Measured on the i.MX8MP
/// (2026-09-30, gl_test 169 on the gcgpu Linux path with the words dumped):
/// every word of a 64x64 surface held (x + y + 1) / 128 at x + y * 64, the
/// ramp the case draws. The first reading here was etnaviv's 4x4 TILED one
/// (PE_DEPTH_CONFIG carries no SUPER_TILED here), and it put check 10 of
/// case 169 at another pixel's depth. No tile status (TS mode 0), so memory
/// holds every pixel. A plane that holds a whole clear only (here or
/// parked) is that value everywhere. 1 when nothing holds it or it
/// is multisampled; -EBUSY when a pending kick still uses its surface (the
/// GL core waits for the kicks rendering into it first).
///
/// # Safety
/// `dst` holds `h` rows of `pitch` floats, `w <= pitch`.
pub unsafe fn resolve_depth(
    id: u32,
    dst: *mut f32,
    w: u32,
    h: u32,
    pitch: u32,
    guard: usize,
    pending: impl Fn(u32) -> bool,
) -> i32 {
    if id == 0 || dst.is_null() || w > pitch {
        return 1;
    }
    let c = cache();
    let fill = |v: f32| {
        // SAFETY: the caller's contract.
        unsafe { ds_retention::write_depth_texels(dst, w, h, pitch, |_, _| Some(v)) };
        0
    };
    let mine = |k: Option<PlaneKey>| k.is_some_and(|k| k.id == id && k.scale == 1 && k.plane == DEPTH as u8);
    let held = c.entries.iter().flatten().find(|e| mine(e.key.plane(DEPTH)) && e.planes[DEPTH] != Plane::UNDEFINED);
    let Some(e) = held else {
        /* a clear value parked when its surface went to another pair */
        let parked = c.parked.iter().find(|p| mine(Some(p.key))).and_then(|p| p.planes[0].cleared_value());
        return parked.map_or(1, |v| fill(f32::from_bits(v)));
    };
    if let Some(v) = e.planes[DEPTH].cleared_value() {
        return fill(f32::from_bits(v));
    }
    if e.used_by != 0 && pending(e.used_by) {
        return -crate::util::EBUSY;
    }
    let bytes = e.stride as usize * e.rows as usize;
    // SAFETY: a live allocation of this cache, guard + stride * rows + guard
    // bytes, which no pending kick uses (checked above).
    let Some(all) = (unsafe { e.mem.bytes_mut() }) else { return 1 };
    // SAFETY: the whole CPU mapping.
    unsafe { osal::cache_invd(e.mem.cpu, e.mem.size) };
    let Some(px) = all.get(guard..guard + bytes) else { return -crate::util::EINVAL };
    /* a row at a time on plain slices: no closure, no checked index per
     * texel (the per-texel form cost ~2 ms a 1024 x 1024 resolve on the
     * i.MX8MP, docs/notes/gc-depth-resolve.md). A texel past the surface is
     * left as it is. */
    let stride = e.stride as usize;
    let (cols, rows) = ((w as usize).min(stride / 4), (h as usize).min(e.rows as usize));
    for y in 0..rows {
        let src = &px[y * stride..y * stride + cols * 4];
        // SAFETY: the caller's contract: `h` rows of `pitch` floats, `cols <= w <= pitch`.
        let out = unsafe { core::slice::from_raw_parts_mut(dst.add(y * pitch as usize), cols) };
        for (o, s) in out.iter_mut().zip(src.as_chunks::<4>().0) {
            *o = ds_retention::unorm24_depth(u32::from_ne_bytes(*s) >> 8);
        }
    }
    0
}

/// Keep the clear values of a surface that is let go.
fn park(parked: &mut Parked<PlaneKey, Plane, PARKED>, e: &Surface) {
    for p in [DEPTH, STENCIL] {
        if let (Some(pk), Some(_)) = (e.key.plane(p), e.planes[p].cleared_value()) {
            parked.park(pk, [pk.id, 0], [e.planes[p], Plane::UNDEFINED]);
        }
    }
}

fn alloc(mem: &mut OsalGpuMem, stride: u32, rows: u32, guard: usize) -> i32 {
    let bytes = stride as usize * rows as usize + 2 * guard;
    osal::gpu_alloc(heap_data(), bytes, OSAL_MEM_DATA, mem)
}

enum Victim {
    Slot(usize),
    /// Every surface holds a live plane: the draw is refused.
    AllLive,
    /// Some hold none, but a pending kick still uses each of those.
    AllPending,
}

/// An empty slot, or the least recently used surface that holds no live
/// plane, whose clear values fit the parked table, and that no pending
/// kick uses.
fn victim(c: &Cache, pending: &impl Fn(u32) -> bool) -> Victim {
    if let Some(i) = c.entries.iter().position(|e| e.is_none()) {
        return Victim::Slot(i);
    }
    let room = c.parked.room();
    let may_go = |e: &Surface| !e.holds_contents() && e.cleared_planes() <= room;
    if !c.entries.iter().flatten().any(may_go) {
        return Victim::AllLive;
    }
    c.entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| e.as_ref().map(|e| (i, e)))
        .filter(|(_, e)| may_go(e) && (e.used_by == 0 || !pending(e.used_by)))
        .min_by_key(|(_, e)| e.stamp)
        .map_or(Victim::AllPending, |(i, _)| Victim::Slot(i))
}

/// Free every surface (backend shutdown) and forget the parked planes. The
/// backend's mirror is cleared with them: it never owned the memory.
pub fn release_all() {
    let c = cache();
    for e in c.entries.iter_mut() {
        if let Some(mut old) = e.take() {
            osal::gpu_free(&mut old.mem);
        }
    }
    c.parked.clear();
    let g = g();
    g.depth = OsalGpuMem::ZERO;
    g.depth_stride = 0;
    g.depth_w = 0;
    g.depth_h = 0;
}

/// The planes of `key`'s surface (tests).
#[cfg(test)]
pub fn planes_of(key: DsKey) -> Option<[Plane; 2]> {
    cache().entries.iter().flatten().find(|e| e.key == key).map(|e| e.planes)
}

/// `key`'s surface: its memory and the bytes of its pixels past the guard
/// (tests).
#[cfg(test)]
pub fn memory_of(key: DsKey) -> Option<(OsalGpuMem, usize)> {
    cache().entries.iter().flatten().find(|e| e.key == key).map(|e| (e.mem, (e.stride * e.rows) as usize))
}

/// `key`'s surface's row stride in bytes (tests).
#[cfg(test)]
pub fn stride_of(key: DsKey) -> Option<u32> {
    cache().entries.iter().flatten().find(|e| e.key == key).map(|e| e.stride)
}

/// Planes parked now (tests).
#[cfg(test)]
pub fn parked_len() -> usize {
    cache().parked.len()
}
