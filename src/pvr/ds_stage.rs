// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Depth and stencil written into a GL object that no frame has drawn into
//! yet (`pvr_backend_ds_write` when no buffer holds the plane).
//!
//! The backend makes a depth/stencil buffer the first time a frame draws
//! into an object. `ds_write` is not given the object's size, so it cannot
//! make that buffer itself. Before this module it answered 1 ("nothing
//! held"), and the GL core then refused the blit with GL_INVALID_OPERATION.
//! That is the resolve blit of dEQP-GLES3 `fbo.msaa.{2,4}_samples.<depth or
//! stencil format>`: `resolveFbo`'s depth/stencil renderbuffer is new and
//! never drawn into (tests/es3_msaa_ds_resolve.rs has the trace).
//!
//! Here such a write is kept in a CPU-side plane of its own. It is the
//! packed D24S8 word layout that `ds_write` writes and the resolves read
//! (depth in bits 23..0, stencil in 31..24), row-linear at the width the
//! writes have reached. It grows to the rectangles written. Pixels outside
//! them hold what a frame starts a new buffer with (depth 1.0, stencil 0).
//! The resolves read it. The object's first frame takes it into the buffer
//! that frame gets (`ds_seed_staged` in backend.rs), loads it, and the plane
//! is held from then on like any stored plane. If the object goes, its
//! plane goes too (`attachment_gone`).
//!
//! Pure CPU memory from the general heap: no kick reads it.

use super::backend::HEAP_GENERAL_BASE;
use crate::ds_retention::{self, DEPTH};
use crate::osal::{self, OsalGpuMem, OSAL_MEM_DATA};

/// The objects whose planes can be staged at once. A blit into a fifth one
/// is refused, as before.
pub const STAGED_MAX: usize = 4;

/// The largest side a staged plane may reach (GL_MAX_RENDERBUFFER_SIZE and
/// the window are well inside it).
const SIDE_MAX: u32 = 8192;

/// The word a pixel of a staged plane holds before anything is written
/// there: depth 1.0, stencil 0, as a frame starts a new buffer.
pub const WORD_UNWRITTEN: u32 = 0x00ff_ffff;

/// One object's staged planes.
#[derive(Clone, Copy)]
pub struct Staged {
    /// The GL object (the `GlesDraw::depth_id` form); 0: a free entry.
    pub id: u32,
    /// Which planes (`DEPTH`, `STENCIL` of ds_retention) were written.
    pub planes: [bool; 2],
    /// The extent the writes have reached, in pixels.
    pub w: u32,
    pub h: u32,
    /// `w` x `h` packed D24S8 words, row-linear.
    pub mem: OsalGpuMem,
}

impl Staged {
    const ZERO: Staged = Staged { id: 0, planes: [false; 2], w: 0, h: 0, mem: OsalGpuMem::ZERO };

    /// The words, `w` a row.
    pub fn words(&self) -> &[u32] {
        if self.mem.cpu.is_null() {
            return &[];
        }
        // SAFETY: a CPU mapping of at least w * h words (`grow`).
        unsafe { core::slice::from_raw_parts(self.mem.cpu as *const u32, self.w as usize * self.h as usize) }
    }
}

/// The staged planes of the backend. All-zero is empty.
pub struct DsStage {
    e: [Staged; STAGED_MAX],
}

impl DsStage {
    fn find(&self, id: u32) -> Option<usize> {
        (id != 0).then(|| self.e.iter().position(|s| s.id == id)).flatten()
    }

    /// Whether `id` has a staged plane `pl`.
    pub fn has(&self, id: u32, pl: usize) -> bool {
        self.find(id).is_some_and(|i| self.e[i].planes[pl])
    }

    /// Whether `id` has any staged plane.
    pub fn holds(&self, id: u32) -> bool {
        self.find(id).is_some()
    }

    /// Whether a plane of `id` can be staged: it has an entry, or a free one
    /// is there.
    pub fn can_stage(&self, id: u32) -> bool {
        id != 0 && (self.find(id).is_some() || self.e.iter().any(|s| s.id == 0))
    }

    /// Write a `w` x `h` rectangle at (`x`, `y`) of plane `pl` of `id`,
    /// as `pvr_backend_ds_write` does into a buffer: depth as f32 (to
    /// 24-bit unorm), stencil as u8. 0; `-ENOMEM` when no entry or memory is
    /// left; `-EINVAL` for a rectangle past [`SIDE_MAX`].
    ///
    /// # Safety
    /// `src` holds `h` rows of `pitch` elements of the plane's type,
    /// `w <= pitch`.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn write(
        &mut self,
        id: u32,
        pl: usize,
        src: *const core::ffi::c_void,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
        pitch: u32,
    ) -> i32 {
        let (Some(xe), Some(ye)) = (x.checked_add(w), y.checked_add(h)) else {
            return -crate::util::EINVAL;
        };
        if id == 0 || xe > SIDE_MAX || ye > SIDE_MAX {
            return -crate::util::EINVAL;
        }
        if w == 0 || h == 0 {
            return 0;
        }
        let i = match self.find(id) {
            Some(i) => i,
            None => match self.e.iter().position(|s| s.id == 0) {
                Some(i) => i,
                None => return -crate::util::ENOMEM,
            },
        };
        let s = &mut self.e[i];
        if !grow(s, xe, ye) {
            return -crate::util::ENOMEM;
        }
        s.id = id;
        s.planes[pl] = true;
        let sw = s.w as usize;
        // SAFETY: a CPU mapping of s.w * s.h words, which covers the
        // rectangle (grown above).
        let words = unsafe { core::slice::from_raw_parts_mut(s.mem.cpu as *mut u32, sw * s.h as usize) };
        for row in 0..h as usize {
            for col in 0..w as usize {
                let at = row * pitch as usize + col;
                let word = &mut words[(y as usize + row) * sw + x as usize + col];
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
        0
    }

    /// Plane `pl` of `id` into `w` x `h` elements at `dst`, `pitch` a row:
    /// what was written, and depth 1.0 or stencil 0 at every other pixel (what
    /// the object's first frame starts it with). False when the plane is not
    /// staged.
    ///
    /// # Safety
    /// `dst` holds `h` rows of `pitch` elements of the plane's type
    /// (f32 depth, u8 stencil), `w <= pitch`.
    pub unsafe fn read(&self, id: u32, pl: usize, dst: *mut core::ffi::c_void, w: u32, h: u32, pitch: u32) -> bool {
        let Some(s) = self.find(id).map(|i| &self.e[i]).filter(|s| s.planes[pl]) else {
            return false;
        };
        let words = s.words();
        for y in 0..h as usize {
            for x in 0..w as usize {
                let inside = x < s.w as usize && y < s.h as usize;
                let v = if inside { words[y * s.w as usize + x] } else { WORD_UNWRITTEN };
                let at = y * pitch as usize + x;
                // SAFETY: the caller's contract: (x, y) is inside the rows.
                unsafe {
                    if pl == DEPTH {
                        (dst as *mut f32).add(at).write(ds_retention::unorm24_depth(v));
                    } else {
                        (dst as *mut u8).add(at).write((v >> 24) as u8);
                    }
                }
            }
        }
        true
    }

    /// The staged planes of `id`, taken out: the caller frees `mem` once it
    /// has copied them ([`release`]).
    pub fn take(&mut self, id: u32) -> Option<Staged> {
        let i = self.find(id)?;
        let s = self.e[i];
        self.e[i] = Staged::ZERO;
        Some(s)
    }

    /// Drop the staged planes of `id` (the object's contents are gone).
    pub fn forget(&mut self, id: u32) {
        if let Some(mut s) = self.take(id) {
            release(&mut s);
        }
    }

    /// Drop every staged plane (backend shutdown).
    pub fn clear(&mut self) {
        for s in self.e.iter_mut() {
            release(s);
        }
    }
}

/// Free a staged entry's memory.
pub fn release(s: &mut Staged) {
    if !s.mem.impl_.is_null() {
        osal::gpu_free(&mut s.mem);
    }
    *s = Staged::ZERO;
}

/// Make `s` at least `w` x `h`, keeping what it holds; new pixels hold
/// [`WORD_UNWRITTEN`]. False when the memory could not be had (`s` as it was).
fn grow(s: &mut Staged, w: u32, h: u32) -> bool {
    let (nw, nh) = (w.max(s.w), h.max(s.h));
    if nw == s.w && nh == s.h && !s.mem.cpu.is_null() {
        return true;
    }
    let n = nw as usize * nh as usize;
    let mut m = OsalGpuMem::default();
    if osal::gpu_alloc(HEAP_GENERAL_BASE, n * 4, OSAL_MEM_DATA, &mut m) != 0 || m.cpu.is_null() {
        osal::gpu_free(&mut m);
        return false;
    }
    // SAFETY: a fresh CPU mapping of n words.
    let new = unsafe { core::slice::from_raw_parts_mut(m.cpu as *mut u32, n) };
    new.fill(WORD_UNWRITTEN);
    let old = s.words();
    for y in 0..s.h as usize {
        let row = &old[y * s.w as usize..(y + 1) * s.w as usize];
        new[y * nw as usize..y * nw as usize + s.w as usize].copy_from_slice(row);
    }
    if !s.mem.impl_.is_null() {
        osal::gpu_free(&mut s.mem);
    }
    (s.mem, s.w, s.h) = (m, nw, nh);
    true
}

/// Copy `st`'s staged planes into the `bw` x `bh` packed D24S8 `words` of a
/// buffer, row-linear at `row` words: the corner both cover. A plane not
/// staged keeps its half of each word.
pub fn seed(st: &Staged, words: &mut [u32], row: usize, bw: u32, bh: u32) {
    let mask = match st.planes {
        [true, true] => 0xffff_ffff,
        [true, false] => 0x00ff_ffff,
        [false, true] => 0xff00_0000,
        [false, false] => return,
    };
    let src = st.words();
    for y in 0..bh.min(st.h) as usize {
        for x in 0..bw.min(st.w) as usize {
            let w = &mut words[y * row + x];
            *w = (*w & !mask) | (src[y * st.w as usize + x] & mask);
        }
    }
}
