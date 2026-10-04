// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! What a backend keeps of GL-visible depth and stencil between kicks, and
//! when it may let a buffer go. Shared by the three backends (src/pvr,
//! src/v3d, src/vivante), which each keep a small table of depth/stencil
//! buffers in GPU memory.
//!
//! The contents of a depth or stencil renderbuffer (or a depth texture)
//! belong to the GL object, not to the backend. ES 2.0.25 4.4.3:
//!
//! ```text
//! A renderbuffer is a data storage object containing a single image ...
//! After a renderbuffer object is deleted, it has no contents ...
//! RenderbufferStorage deletes any existing data store for the renderbuffer
//! and the contents of the data store after calling RenderbufferStorage are
//! undefined.
//! ```
//!
//! Nothing else in 4.4 undefines them: they persist across framebuffer
//! switches. So a buffer that holds contents is never given to another
//! target. Its slot is free again only when the contents are gone in GL's
//! own terms:
//!
//! - the object is deleted and no framebuffer holds it any more, or its
//!   storage is specified again ([`crate::backend::attachment_gone`], called
//!   by the GL core);
//! - the plane holds nothing a later draw may read (never written, or
//!   undefined after an `eglSwapBuffers`);
//! - the plane was cleared whole and nothing drew into it since: its
//!   contents are one value, which is kept here ([`Parked`]) instead of in a
//!   buffer, and written back when the object is drawn to again.
//!
//! When a draw needs a buffer and every slot holds contents, the draw is
//! refused with `-ENOMEM` ([`refuse`]): `GL_OUT_OF_MEMORY` in the GL core, and
//! one named log line. It is never drawn over another object's contents.

use crate::backend::GlesDraw;

/// The depth plane's index in the `[_; 2]` arrays below; the stencil's is 1.
pub const DEPTH: usize = 0;
/// The stencil plane's index.
pub const STENCIL: usize = 1;

/// What one plane (the depth or the stencil of one GL object) holds in a
/// backend's buffer. All-zero is [`Plane::UNDEFINED`], so a zeroed backend
/// state is valid.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Plane {
    state: u8,
    /// The clear value (depth as `f32` bits, stencil as the value) for
    /// `CLEARED`.
    value: u32,
}

const UNDEFINED: u8 = 0;
const CLEARED: u8 = 1;
const LIVE: u8 = 2;

impl Plane {
    /// Nothing a later draw may read: never written, or undefined (a swap).
    pub const UNDEFINED: Plane = Plane { state: UNDEFINED, value: 0 };
    /// Written by a draw: the buffer holds contents GL keeps.
    pub const LIVE: Plane = Plane { state: LIVE, value: 0 };

    /// Cleared whole to `value` and not drawn into since.
    pub const fn cleared(value: u32) -> Plane {
        Plane { state: CLEARED, value }
    }

    /// The buffer holds contents that exist nowhere else.
    pub const fn is_live(self) -> bool {
        self.state == LIVE
    }

    /// The value it was cleared to, when that is all it holds.
    pub const fn cleared_value(self) -> Option<u32> {
        if self.state == CLEARED {
            Some(self.value)
        } else {
            None
        }
    }

    /// The plane after a kick that cleared it whole to `cleared` (before any
    /// draw of the kick) and whose draws `written` it.
    pub fn after(self, cleared: Option<u32>, written: bool) -> Plane {
        match (written, cleared) {
            (true, _) => Plane::LIVE,
            (false, Some(v)) => Plane::cleared(v),
            (false, None) => self,
        }
    }
}

/// Which planes a draw may write (not merely test): the depth when the depth
/// test is on with writes enabled (ES 2.0 4.1.6: no test, no write), the
/// stencil when the stencil test is on with a write mask on either face. A
/// clear drawn as a quad (scissored or masked) writes what it clears. A
/// full clear is not a write: see [`draw_clears`].
pub fn draw_writes(d: &GlesDraw) -> [bool; 2] {
    let draws = d.index_count != 0;
    let quad = d.clear_quad;
    [
        (draws && d.depth_test && d.depth_write) || (quad && d.do_clear_depth),
        (draws && d.stencil && (d.stencil_write_mask | d.stencil_back_write_mask) != 0) || (quad && d.do_clear_stencil),
    ]
}

/// The whole-plane clears a draw carries (the background clear, not the
/// quad): the depth value as `f32` bits, the stencil value.
pub fn draw_clears(d: &GlesDraw) -> [Option<u32>; 2] {
    let bg = !d.clear_quad;
    [
        (bg && d.do_clear_depth).then_some(d.clear_depth.to_bits()),
        (bg && d.do_clear_stencil).then_some(u32::from(d.clear_stencil)),
    ]
}

/// A 24-bit unsigned normalised depth (the low 24 bits of `word`) as the
/// float a depth texture's R32F texel holds: 0 is 0.0, 2^24 - 1 is 1.0.
pub fn unorm24_depth(word: u32) -> f32 {
    ((word & 0xff_ffff) as f64 / 16_777_215.0) as f32
}

/// Write `depth(x, y)` into the `w` x `h` floats at `dst`, `pitch` floats a
/// row, for `backend::depth_resolve`; a `None` leaves that texel as it is
/// (outside the backend's buffer).
///
/// # Safety
/// `dst` holds `h` rows of `pitch` floats, `w <= pitch`.
pub unsafe fn write_depth_texels(dst: *mut f32, w: u32, h: u32, pitch: u32, depth: impl Fn(u32, u32) -> Option<f32>) {
    for y in 0..h {
        for x in 0..w {
            if let Some(v) = depth(x, y) {
                // SAFETY: the caller's contract: (x, y) is inside the rows.
                unsafe { dst.add(y as usize * pitch as usize + x as usize).write(v) };
            }
        }
    }
}

/// One parked entry: planes whose buffer was given to another target while
/// they held only a clear value.
#[derive(Clone, Copy, Debug)]
pub struct ParkedEntry<K: Copy, P: Copy> {
    pub used: bool,
    /// The backend's buffer key the planes were kept under.
    pub key: K,
    /// The GL object of each plane (`GlesDraw::depth_id` form; 0: none),
    /// so [`Parked::forget`] drops a plane when its object goes.
    pub owners: [u32; 2],
    pub planes: [P; 2],
}

/// Planes kept without a buffer (see the module comment). A slot whose
/// planes hold a clear value is let go only when its entry fits here, so a
/// clear value is never lost either.
pub struct Parked<K: Copy + PartialEq, P: Copy + PartialEq, const N: usize> {
    e: [ParkedEntry<K, P>; N],
    /// The plane value that holds nothing (Undefined).
    empty: P,
}

impl<K: Copy + PartialEq, P: Copy + PartialEq, const N: usize> Parked<K, P, N> {
    /// An empty table; `key` fills the unused entries, `empty` is the plane
    /// value that holds nothing.
    pub const fn new(key: K, empty: P) -> Self {
        Parked { e: [ParkedEntry { used: false, key, owners: [0; 2], planes: [empty; 2] }; N], empty }
    }

    /// Room for one more entry.
    pub fn has_room(&self) -> bool {
        self.room() != 0
    }

    /// Entries free.
    pub fn room(&self) -> usize {
        self.e.iter().filter(|e| !e.used).count()
    }

    /// Keep `planes` of `key`; false when there is no room (the caller then
    /// keeps the buffer).
    pub fn park(&mut self, key: K, owners: [u32; 2], planes: [P; 2]) -> bool {
        self.take(key);
        match self.e.iter_mut().find(|e| !e.used) {
            Some(e) => {
                *e = ParkedEntry { used: true, key, owners, planes };
                true
            }
            None => false,
        }
    }

    /// The planes parked for `key`, removed from the table.
    pub fn take(&mut self, key: K) -> Option<[P; 2]> {
        let e = self.e.iter_mut().find(|e| e.used && e.key == key)?;
        e.used = false;
        Some(e.planes)
    }

    /// Is anything parked for `key`?
    pub fn holds(&self, key: K) -> bool {
        self.e.iter().any(|e| e.used && e.key == key)
    }

    /// Drop the planes of the GL object `owner` (it is gone); an entry left
    /// with nothing goes too. The other plane of an entry stays: a stencil
    /// object deleted does not take the depth object's clear value with it.
    pub fn forget(&mut self, owner: u32) {
        let empty = self.empty;
        for e in self.e.iter_mut().filter(|e| e.used) {
            for p in [DEPTH, STENCIL] {
                if e.owners[p] == owner {
                    (e.owners[p], e.planes[p]) = (0, empty);
                }
            }
            if e.planes == [empty; 2] {
                e.used = false;
            }
        }
    }

    /// The entries in use.
    pub fn iter(&self) -> impl Iterator<Item = &ParkedEntry<K, P>> {
        self.e.iter().filter(|e| e.used)
    }

    /// Drop everything (a backend reset).
    pub fn clear(&mut self) {
        for e in self.e.iter_mut() {
            e.used = false;
        }
    }

    /// Entries in use.
    pub fn len(&self) -> usize {
        self.e.iter().filter(|e| e.used).count()
    }

    /// No entry in use.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The words every refusal line starts with (tests look for them).
pub const REFUSED: &str = "depth/stencil refused";

/// The id the last refusal was logged for: a draw loop refused frame after
/// frame logs its cause once, a refusal for another object logs again.
static LAST_REFUSED: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Refuse a draw that needs a depth/stencil buffer for the object `id`
/// (`GlesDraw::depth_id` form, or the stencil's) when every one of the
/// backend's `slots` holds contents GL keeps: one log line naming both, and
/// `-ENOMEM`, which the GL core reports as `GL_OUT_OF_MEMORY`
/// (`backend::draw`).
pub fn refuse(gpu: &str, id: u32, slots: usize) -> i32 {
    let tag = (1u64 << 32) | u64::from(id);
    if LAST_REFUSED.swap(tag, core::sync::atomic::Ordering::Relaxed) != tag {
        glog!(
            "yakogl/{}: {} for object 0x{:x}: all {} depth/stencil buffers hold contents GL keeps (none deleted, \
             none cleared whole since its last draw); the draw is not recorded (GL_OUT_OF_MEMORY)",
            gpu,
            REFUSED,
            id,
            slots
        );
    }
    -crate::util::ENOMEM
}

/// Refuse a draw that needs a depth/stencil buffer for the object `id` when
/// the GPU memory for one (`bytes`) cannot be had: one log line naming both,
/// and `-ENOMEM` (`GL_OUT_OF_MEMORY`), as for [`refuse`]. The draw is never
/// drawn with depth and stencil kept on-chip only: a later job could not see
/// what it wrote, and GL keeps those contents.
pub fn refuse_no_memory(gpu: &str, id: u32, bytes: usize) -> i32 {
    let tag = (2u64 << 32) | u64::from(id);
    if LAST_REFUSED.swap(tag, core::sync::atomic::Ordering::Relaxed) != tag {
        glog!(
            "yakogl/{}: {} for object 0x{:x}: no GPU memory for its {} byte depth/stencil buffer; the draw is \
             not recorded (GL_OUT_OF_MEMORY)",
            gpu,
            REFUSED,
            id,
            bytes
        );
    }
    -crate::util::ENOMEM
}

/// A draw got its buffers: the next refusal logs again.
pub fn admitted() {
    LAST_REFUSED.store(0, core::sync::atomic::Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plane_is_live_once_written_and_cleared_after_a_whole_clear() {
        let p = Plane::UNDEFINED;
        assert!(!p.is_live());
        assert_eq!(p.after(None, false), Plane::UNDEFINED, "a kick that did nothing to it");
        assert_eq!(p.after(Some(7), false).cleared_value(), Some(7));
        assert!(p.after(Some(7), true).is_live(), "cleared, then drawn into: contents");
        assert!(Plane::LIVE.after(None, false).is_live(), "a kick that only tests keeps it");
        assert_eq!(Plane::LIVE.after(Some(3), false), Plane::cleared(3), "a whole clear replaces it");
        // SAFETY: a Plane is two integers; all-zero is a valid one.
        let zero: Plane = unsafe { core::mem::zeroed() };
        assert_eq!(zero, Plane::UNDEFINED, "a zeroed backend state holds Undefined planes");
    }

    #[test]
    fn parked_planes_are_taken_once_and_forgotten_with_their_object() {
        let mut t: Parked<u32, Plane, 2> = Parked::new(0, Plane::UNDEFINED);
        assert!(t.park(5, [50, 0], [Plane::cleared(1), Plane::UNDEFINED]));
        assert!(t.park(6, [60, 61], [Plane::cleared(4), Plane::cleared(2)]));
        assert!(!t.has_room() && !t.park(7, [70, 0], [Plane::UNDEFINED; 2]), "full: refused, not overwritten");
        assert_eq!(t.take(5).map(|p| p[0]), Some(Plane::cleared(1)));
        assert_eq!(t.take(5), None, "taken once");
        t.forget(61);
        assert_eq!(t.len(), 1, "the stencil object 61 went; the depth object 60's clear value stays");
        t.forget(60);
        assert!(t.is_empty(), "nothing left: the entry goes");
    }

    #[test]
    fn a_draw_writes_only_what_its_state_lets_it_write() {
        let mut d = GlesDraw { index_count: 3, ..GlesDraw::ZERO };
        assert_eq!(draw_writes(&d), [false, false], "no test: nothing written");
        d.depth_test = true;
        assert_eq!(draw_writes(&d), [false, false], "a test with writes off only reads");
        d.depth_write = true;
        (d.stencil, d.stencil_back_write_mask) = (true, 0x0f);
        assert_eq!(draw_writes(&d), [true, true]);
        let quad = GlesDraw { clear_quad: true, do_clear_stencil: true, ..GlesDraw::ZERO };
        assert_eq!(draw_writes(&quad), [false, true], "a scissored clear writes a part");
        assert_eq!(draw_clears(&quad), [None, None], "and is no whole clear");
        let bg = GlesDraw { do_clear_depth: true, clear_depth: 0.25, ..GlesDraw::ZERO };
        assert_eq!(draw_clears(&bg), [Some(0.25f32.to_bits()), None]);
    }
}
