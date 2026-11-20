// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Case 247 `dlayout` (docs/notes/gc-ts-probe.md, "Part (ii)", designed as
//! case 244; 243 to 245 were taken by then): the depth surface's override
//! plan and the dump step, as pure functions, and the `test-knobs` hook the
//! case calls.
//!
//! The case draws a 64x64 depth ramp under one of four layout modes. [`plan`]
//! says what the mode asks of the PE (`PE_LOGIC_OP` SINGLE_BUFFER and
//! `PE_DEPTH_CONFIG` SUPER_TILED) and [`dump`] turns the bytes read back into
//! the page words through [`layout::dlayout_decode`]; [`Dump::as_c_words`]
//! gives them as the C twin's three 32-bit pages
//! (`driver/tools/pvrlab/gl_test_probe_decode.inc`, `pd_pack_a/b/c`).
//!
//! The hook `gc_debug_depth_dump` (`test-knobs`) reads the back end's depth
//! surface after the case's GL draw. Today's draws reach two of the four
//! modes: mode 3 (SINGLE_BUFFER 1, no SUPER_TILED: every draw with
//! `gc_tiled_rt` off) and mode 1 (SINGLE_BUFFER 2 with SUPER_TILED: a draw
//! into a tiled shadow with `gc_tiled_rt` on, `ds_cache::select_in`). Modes 0
//! and 2 need an override of those two fields in `backend.rs`'s depth-only
//! draw, which this module does not have; the hook refuses them
//! ([`DUMP_ERR_MODE_UNREACHABLE`]).

use super::layout::{self, DlayoutResult, Layout};
use super::regs::{
    pe_depth_func, pe_logic_op_single_buffer, GC_PE_DEPTH_FORMAT_D24S8, GC_PE_DEPTH_MODE_Z, GC_PE_DEPTH_SUPER_TILED,
    GC_PE_DEPTH_UNK18, GC_PE_DEPTH_WRITE_ENABLE,
};

/// Number of layout modes, one per boot.
pub const MODES: u32 = 4;
/// The code `F` of page A's w1 (a fence timeout), not in `DlayoutCode`.
pub const HANG: u64 = 0xf;
/// Hardware encoding of the compare function GL_ALWAYS (`PE_DEPTH_CONFIG`).
const DEPTH_FUNC_ALWAYS: u32 = 7;
/// The D24S8 word of the clear before the ramp: depth 1.0, stencil 0
/// (`ts::depth_clear_word(S8Z24, 1.0, 0)`).
pub const CLEAR: u32 = 0xffff_ff00;

/// What one mode asks of the depth surface of the ramp draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DepthOverride {
    /// The mode, 0..=3 (the page tag's low bits).
    pub mode: u32,
    /// The value of `PE_LOGIC_OP`'s SINGLE_BUFFER field.
    pub single_buffer: u32,
    /// Whether `PE_DEPTH_CONFIG` carries `GC_PE_DEPTH_SUPER_TILED`.
    pub super_tiled: bool,
}

/// The override of `mode` (the note's table), `None` past mode 3.
pub const fn plan(mode: u32) -> Option<DepthOverride> {
    let (single_buffer, super_tiled) = match mode {
        0 => (2, false),
        1 => (2, true),
        2 => (1, true),
        3 => (1, false),
        _ => return None,
    };
    Some(DepthOverride { mode, single_buffer, super_tiled })
}

/// Whether the back end's draws reach `mode` without an override: mode 3
/// (`gc_tiled_rt` off) and mode 1 (a draw into a tiled shadow with it on).
pub const fn reachable(mode: u32) -> bool {
    mode == 1 || mode == 3
}

impl DepthOverride {
    /// `PE_DEPTH_CONFIG` of the ramp draw: D24S8, GL_ALWAYS, depth writes
    /// on, no early depth, no DISABLE_ZS, plus SUPER_TILED when asked.
    pub const fn depth_config(&self) -> u32 {
        GC_PE_DEPTH_MODE_Z
            | GC_PE_DEPTH_FORMAT_D24S8
            | GC_PE_DEPTH_UNK18
            | pe_depth_func(DEPTH_FUNC_ALWAYS)
            | GC_PE_DEPTH_WRITE_ENABLE
            | if self.super_tiled { GC_PE_DEPTH_SUPER_TILED } else { 0 }
    }

    /// The SINGLE_BUFFER field of `PE_LOGIC_OP`, in place (OR it into the
    /// register's other bits).
    pub const fn logic_op_bits(&self) -> u32 {
        pe_logic_op_single_buffer(self.single_buffer)
    }

    /// The layout the surface is declared with: super-tiled when
    /// SUPER_TILED is on, else the row-linear one the PE has written so far.
    pub const fn layout(&self) -> Layout {
        if self.super_tiled {
            Layout::SuperTiled
        } else {
            Layout::Linear
        }
    }

    /// Bytes of the surface the draw covers (64x64 D24S8, 4 bytes a pixel).
    pub const fn surface_bytes(&self) -> usize {
        layout::DLAYOUT_DWORDS * 4
    }
}

/// What [`dump`] returns for one mode: the decode and the pages' words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dump {
    pub result: layout::DlayoutResult,
    /// The mode and the fence timeouts it was dumped with.
    pub mode: u32,
    pub timeouts: u32,
    /// Tag and words of page A, `0xc0 + mode`.
    pub page_a: (u8, [u64; 3]),
    /// Tag and words of page B, `0xc4 + mode`.
    pub page_b: (u8, [u64; 3]),
}

/// Decodes the surface read back after the ramp draw: `dwords` is the
/// surface in memory order (at least 4096), `clear` the value the clear
/// wrote, `timeouts` the fence timeouts counted. A timeout makes the code
/// HANG, as in the C twin. `None` for a mode past 3 or a short readback.
pub fn dump_words(mode: u32, dwords: &[u32], clear: u32, timeouts: u32) -> Option<Dump> {
    plan(mode)?;
    if dwords.len() < layout::DLAYOUT_DWORDS {
        return None;
    }
    let result: DlayoutResult = layout::dlayout_decode(dwords, clear);
    // The C twin (pd_pack_a) clamps the timeouts to 255 and makes any of
    // them the code HANG (0xf); the case writes that code itself.
    let timeouts = timeouts.min(255);
    let mut a = layout::dlayout_page_a(mode, timeouts, &result);
    if timeouts != 0 {
        a[0] = (a[0] & !0xf) | HANG;
    }
    let page_a = (layout::dlayout_tag_a(mode), a);
    let page_b = (layout::dlayout_tag_b(mode), layout::dlayout_page_b(&result));
    Some(Dump { result, mode, timeouts, page_a, page_b })
}

/// [`dump_words`] over the surface's bytes (little-endian dwords, at least
/// 16 KiB).
#[cfg(feature = "host-test")]
pub fn dump(mode: u32, bytes: &[u8], clear: u32, timeouts: u32) -> Option<Dump> {
    let dwords: Vec<u32> = bytes.as_chunks::<4>().0.iter().map(|b| u32::from_le_bytes(*b)).collect();
    dump_words(mode, &dwords, clear, timeouts)
}

impl Dump {
    /// The code of page A (the low nibble of w1): `HANG` after a timeout.
    pub const fn code(&self) -> u32 {
        (self.page_a.1[0] & 0xf) as u32
    }

    /// The pages as the C twin's 32-bit words, `[A, B, C]` with tags
    /// `0xc0 + mode`, `0xc4 + mode`, `0xc8 + mode`: what `pd_pack_a`,
    /// `pd_pack_b` and `pd_pack_c` give for the same surface. A w1, B w1 and
    /// B w3 are the 64-bit pages' words; the 64-bit w2/w3 of A and B hold,
    /// low half first, the C words A w2, A w3, C w1 and B w2, C w2, C w3.
    pub fn as_c_words(&self) -> [[u32; 3]; 3] {
        let (a, b) = (self.page_a.1, self.page_b.1);
        let lo = |w: u64| w as u32;
        let hi = |w: u64| (w >> 32) as u32;
        [[a[0] as u32, lo(a[1]), hi(a[1])], [b[0] as u32, lo(a[2]), b[2] as u32], [hi(a[2]), lo(b[1]), hi(b[1])]]
    }
}

// ---------------------------------------------------------------------------
// The hook (`test-knobs`): case 247 calls it once a boot, after its draw.
// ---------------------------------------------------------------------------

/// `gc_debug_depth_dump`'s refusals: bad arguments (`pages` null).
pub const DUMP_ERR_ARGS: i32 = -1;
/// The mode is past 3.
pub const DUMP_ERR_MODE: i32 = -2;
/// Mode 0 or 2: no draw of the back end reaches it ([`reachable`]).
pub const DUMP_ERR_MODE_UNREACHABLE: i32 = -3;
/// The back end has no depth surface of 64x64 with a CPU view.
pub const DUMP_ERR_SURFACE: i32 = -4;
/// The surface's layout is not the mode's: mode 1 asked but the draw took no
/// tiled shadow (`gc_tiled_rt` off, or the draw did not qualify), or mode 3
/// with a super-tiled surface.
pub const DUMP_ERR_LAYOUT: i32 = -5;
/// The back end's kicks did not finish.
pub const DUMP_ERR_FINISH: i32 = -7;

#[cfg(feature = "test-knobs")]
pub use self::hook::gc_debug_depth_dump;

#[cfg(feature = "test-knobs")]
mod hook {
    use super::*;
    use crate::osal;
    use crate::vivante::backend::{finish, g};

    /// Case 247's dump of the depth surface the last draw wrote
    /// (gc-ts-probe.md "Part (ii)"): waits for the back end's kicks, checks
    /// the surface is the mode's (64x64 D24S8, its layout the mode's,
    /// 256-byte rows when linear), reads its 4096 dwords through the CPU view
    /// into `surf` (when not null), decodes them over [`CLEAR`] and writes
    /// the C twin's pages A, B and C into `pages[0..9]`
    /// ([`Dump::as_c_words`]). `timeouts` is the fence timeouts the case
    /// counted in its draw (any makes the code HANG). Returns the code
    /// (page A's low nibble), or a `DUMP_ERR_*`. Changes nothing of the back
    /// end's state.
    ///
    /// # Safety
    /// `pages` is null or points at 9 writable `u32`s, `surf` null or at
    /// 4096.
    #[no_mangle]
    pub unsafe extern "C" fn gc_debug_depth_dump(mode: u32, timeouts: u32, surf: *mut u32, pages: *mut u32) -> i32 {
        if pages.is_null() {
            return DUMP_ERR_ARGS;
        }
        let Some(p) = plan(mode) else { return DUMP_ERR_MODE };
        if !reachable(mode) {
            return DUMP_ERR_MODE_UNREACHABLE;
        }
        if finish() != 0 {
            return DUMP_ERR_FINISH;
        }
        let g = g();
        let (off, bytes) = (g.depth_surface_off as usize, p.surface_bytes());
        if g.depth.cpu.is_null()
            || g.depth.size < off + bytes
            || g.depth_w < layout::DLAYOUT_SIDE
            || g.depth_h < layout::DLAYOUT_SIDE
            || !(g.depth.cpu as usize + off).is_multiple_of(4)
        {
            return DUMP_ERR_SURFACE;
        }
        if g.depth_layout != p.layout() || (p.layout() == Layout::Linear && g.depth_stride != 4 * layout::DLAYOUT_SIDE)
        {
            return DUMP_ERR_LAYOUT;
        }
        // SAFETY: the surface's CPU view, `bytes` long from `off` (checked
        // against the allocation) and 4-aligned; the kicks are finished.
        let cpu = unsafe { (g.depth.cpu as *mut u8).add(off) };
        if let Some(f) = osal::ops().and_then(|o| o.cache_invd) {
            // SAFETY: as above.
            unsafe { f(cpu as *mut core::ffi::c_void, bytes) };
        }
        // SAFETY: as above.
        let words = unsafe { core::slice::from_raw_parts(cpu as *const u32, layout::DLAYOUT_DWORDS) };
        let Some(d) = dump_words(mode, words, CLEAR, timeouts) else { return DUMP_ERR_SURFACE };
        if !surf.is_null() {
            // SAFETY: the caller's contract: 4096 writable u32s.
            unsafe { core::slice::from_raw_parts_mut(surf, layout::DLAYOUT_DWORDS) }.copy_from_slice(words);
        }
        // SAFETY: the caller's contract: 9 writable u32s.
        let out = unsafe { core::slice::from_raw_parts_mut(pages, 9) };
        for (o, w) in out.chunks_mut(3).zip(d.as_c_words()) {
            o.copy_from_slice(&w);
        }
        d.code() as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vivante::layout::SuperOrder;
    use crate::vivante::regs::*;
    use std::boxed::Box;
    use std::vec;
    use std::vec::Vec;

    fn ramp(map: &dyn Fn(u32, u32) -> usize) -> Vec<u32> {
        let mut s = vec![CLEAR; layout::DLAYOUT_DWORDS];
        for y in 0..64 {
            for x in 0..64 {
                s[map(x, y)] = layout::dlayout_expect(x, y) << 8;
            }
        }
        s
    }

    fn bytes(s: &[u32]) -> Vec<u8> {
        s.iter().flat_map(|d| d.to_le_bytes()).collect()
    }

    fn row(x: u32, y: u32) -> usize {
        (y * 64 + x) as usize
    }

    fn sup(order: SuperOrder) -> impl Fn(u32, u32) -> usize {
        move |x, y| layout::offset_in(Layout::SuperTiled, order, x, y, 4, 256) / 4
    }

    // ---- the C twin (gl_test_probe_decode.inc), transcribed ----
    fn pd_pack16(lo: u32, hi: u32) -> u32 {
        (lo & 0xffff) | (hi & 0xffff) << 16
    }
    fn pd_pack_offsets(o: &[u16; 12]) -> [u32; 6] {
        core::array::from_fn(|k| pd_pack16(o[2 * k] as u32, o[2 * k + 1] as u32))
    }
    fn pd_pack_a(d: &Dump, mode: u32, timeouts: u32) -> [u32; 3] {
        let o = pd_pack_offsets(&d.result.offsets);
        let code = if timeouts != 0 { 0xf } else { d.result.code as u32 };
        [(code & 0xf) | (mode & 0xf) << 4 | timeouts.min(255) << 8 | d.result.matched << 16, o[0], o[1]]
    }
    fn pd_pack_b(d: &Dump) -> [u32; 3] {
        let o = pd_pack_offsets(&d.result.offsets);
        let (i, z) = d.result.first_non_clear.unwrap_or((0xffff, 0));
        [d.result.crc, o[2], pd_pack16(z as u32, i)]
    }
    fn pd_pack_c(d: &Dump) -> [u32; 3] {
        let o = pd_pack_offsets(&d.result.offsets);
        [o[3], o[4], o[5]]
    }

    #[test]
    fn plan_is_the_notes_table() {
        let want = [(2, false), (2, true), (1, true), (1, false)];
        for (m, (sb, st)) in want.into_iter().enumerate() {
            let p = plan(m as u32).unwrap();
            assert_eq!((p.mode, p.single_buffer, p.super_tiled), (m as u32, sb, st));
            assert_eq!(p.depth_config() & GC_PE_DEPTH_SUPER_TILED != 0, st);
            assert_eq!(p.logic_op_bits(), pe_logic_op_single_buffer(sb));
            assert_eq!(p.layout(), if st { Layout::SuperTiled } else { Layout::Linear });
            let c = p.depth_config() & !GC_PE_DEPTH_SUPER_TILED;
            assert_eq!(c, 0x0004_1711, "D24S8, GL_ALWAYS, writes on");
            assert_eq!(c & (GC_PE_DEPTH_EARLY_Z | GC_PE_DEPTH_DISABLE_ZS), 0);
        }
        assert!(plan(MODES).is_none());
        assert_eq!(layout::stride(Layout::Linear, 64, 4, false), 256);
        assert_eq!(CLEAR, crate::vivante::ts::depth_clear_word(crate::vivante::ts::DepthFormat::S8Z24, 1.0, 0));
        assert_eq!((0..MODES).filter(|&m| reachable(m)).collect::<Vec<_>>(), [1, 3]);
    }

    type Case = (u32, Box<dyn Fn(u32, u32) -> usize>, u64);

    #[test]
    fn ramp_under_each_mode_gives_its_pages() {
        let cases: [Case; 4] = [
            (3, Box::new(row), 2),
            (0, Box::new(row), 2),
            (1, Box::new(sup(SuperOrder::Drm)), 4),
            (2, Box::new(sup(SuperOrder::Interleaved)), 5),
        ];
        for (mode, map, code) in cases {
            let d = dump(mode, &bytes(&ramp(&*map)), CLEAR, 0).unwrap();
            assert_eq!(d.page_a.0, 0xc0 + mode as u8);
            assert_eq!(d.page_b.0, 0xc4 + mode as u8);
            assert_eq!(d.page_a.1[0], code | (mode as u64) << 4 | 4096 << 16, "mode {mode}");
            assert_eq!(d.code() as u64, code);
            for (o, &(x, y)) in d.result.offsets.iter().zip(layout::DLAYOUT_REF_PIXELS.iter()) {
                assert_eq!(*o as usize, map(x, y), "mode {mode} ({x},{y})");
            }
            let (i, z) = d.result.first_non_clear.unwrap();
            assert_eq!(i, 0);
            assert_eq!(z as u32, (layout::dlayout_expect(0, 0) << 8) & 0xff);
            assert_eq!(d.page_b.1[2], (i as u64) << 16 | z as u64);
            assert_eq!(d.page_b.1[0], d.result.crc as u64);
        }
    }

    #[test]
    fn c_words_are_the_c_twins_packing() {
        let maps: [Box<dyn Fn(u32, u32) -> usize>; 3] =
            [Box::new(row), Box::new(sup(SuperOrder::Drm)), Box::new(sup(SuperOrder::Interleaved))];
        let mut surfaces: Vec<Vec<u32>> = maps.iter().map(|m| ramp(&**m)).collect();
        surfaces.push(vec![CLEAR; layout::DLAYOUT_DWORDS]);
        let mut odd = surfaces[0].clone();
        odd[100] = CLEAR; /* a missing id */
        surfaces.push(odd);
        for (n, s) in surfaces.iter().enumerate() {
            for mode in 0..MODES {
                for timeouts in [0, 3, 300] {
                    let d = dump_words(mode, s, CLEAR, timeouts).unwrap();
                    let [a, b, c] = d.as_c_words();
                    assert_eq!(a, pd_pack_a(&d, mode, timeouts), "surface {n} mode {mode} page A");
                    assert_eq!(b, pd_pack_b(&d), "surface {n} mode {mode} page B");
                    assert_eq!(c, pd_pack_c(&d), "surface {n} mode {mode} page C");
                    assert_eq!(d, dump(mode, &bytes(s), CLEAR, timeouts).unwrap(), "bytes and words decode alike");
                }
            }
        }
    }

    #[test]
    fn nothing_written_and_timeouts() {
        let s = vec![CLEAR; layout::DLAYOUT_DWORDS];
        let d = dump_words(0, &s, CLEAR, 0).unwrap();
        assert_eq!(d.code(), 1, "NO_WRITE");
        assert_eq!(d.result.first_non_clear, None);
        assert_eq!(d.page_b.1[2], 0xffff << 16);
        let h = dump_words(1, &s, CLEAR, 2).unwrap();
        assert_eq!(h.page_a.1[0] & 0xfff, 0xf | 1 << 4 | 2 << 8, "HANG with 2 timeouts");
    }

    #[test]
    fn refuses_a_bad_mode_or_short_readback() {
        let s = vec![0u32; layout::DLAYOUT_DWORDS];
        assert!(dump_words(MODES, &s, CLEAR, 0).is_none());
        assert!(dump_words(0, &s[..s.len() - 1], CLEAR, 0).is_none());
    }

    /// The hook over the back end's depth surface, set by hand to a host
    /// buffer: what case 247 reads after its draw.
    #[cfg(feature = "test-knobs")]
    mod hook_tests {
        use super::*;
        use crate::osal::OsalGpuMem;
        use crate::testutil::TestGl;
        use crate::vivante::backend::g;

        /// The serialising guard, and the back end's depth fields put back
        /// on the way out.
        struct Rig {
            saved: (OsalGpuMem, u32, u32, u32, u32, Layout),
            mem: OsalGpuMem,
            _gl: TestGl,
        }

        impl Rig {
            fn new() -> Rig {
                let gl = TestGl::new(4, 4);
                let g = g();
                let saved = (g.depth, g.depth_stride, g.depth_w, g.depth_h, g.depth_surface_off, g.depth_layout);
                Rig { saved, mem: OsalGpuMem::ZERO, _gl: gl }
            }

            /// The back end's depth surface becomes a GPU allocation holding
            /// `s` (64x64, `layout`, rows of `stride` bytes).
            fn surface(&mut self, s: Vec<u32>, layout: Layout, stride: u32) {
                crate::osal::gpu_free(&mut self.mem);
                assert_eq!(
                    crate::osal::gpu_alloc(
                        crate::testutil::HEAP_DATA,
                        s.len() * 4,
                        crate::osal::OSAL_MEM_DATA,
                        &mut self.mem
                    ),
                    0
                );
                // SAFETY: the allocation is `s.len()` dwords, mapped at `cpu`.
                unsafe { core::slice::from_raw_parts_mut(self.mem.cpu as *mut u32, s.len()) }.copy_from_slice(&s);
                /* as the GPU's write: what the hook's invalidate then reads */
                // SAFETY: the allocation's own mapping and size.
                unsafe { crate::vivante::state::cache_flush_raw(self.mem.cpu, s.len() * 4) };
                let g = g();
                g.depth = self.mem;
                (g.depth_stride, g.depth_w, g.depth_h, g.depth_surface_off, g.depth_layout) =
                    (stride, 64, 64, 0, layout);
            }
        }

        impl Drop for Rig {
            fn drop(&mut self) {
                let g = g();
                (g.depth, g.depth_stride, g.depth_w, g.depth_h, g.depth_surface_off, g.depth_layout) = self.saved;
                crate::osal::gpu_free(&mut self.mem);
            }
        }

        fn call(mode: u32, timeouts: u32) -> (i32, [u32; 9], Vec<u32>) {
            let mut pages = [0u32; 9];
            let mut surf = vec![0u32; layout::DLAYOUT_DWORDS];
            // SAFETY: 9 and 4096 writable u32s.
            let rc = unsafe { gc_debug_depth_dump(mode, timeouts, surf.as_mut_ptr(), pages.as_mut_ptr()) };
            (rc, pages, surf)
        }

        #[test]
        fn the_hook_dumps_the_two_reachable_modes() {
            let mut r = Rig::new();
            let lin = ramp(&row);
            r.surface(lin.clone(), Layout::Linear, 256);
            let (rc, pages, surf) = call(3, 0);
            assert_eq!(rc, 2, "mode 3 reads ROW_LINEAR");
            assert_eq!(surf, lin, "the surface handed out as read");
            let want = dump_words(3, &lin, CLEAR, 0).unwrap().as_c_words();
            assert_eq!(pages, *want.as_flattened());

            let st = ramp(&sup(SuperOrder::Drm));
            r.surface(st.clone(), Layout::SuperTiled, 256);
            let (rc, pages, _) = call(1, 0);
            assert_eq!(rc, 4, "mode 1 reads SUPER_DRM");
            assert_eq!(pages, *dump_words(1, &st, CLEAR, 0).unwrap().as_c_words().as_flattened());
            let (rc, pages, _) = call(1, 1);
            assert_eq!(rc, 0xf, "a timeout is HANG");
            assert_eq!(pages[0] & 0xfff, 0xf | 1 << 4 | 1 << 8);
        }

        #[test]
        fn the_hook_refuses_what_it_cannot_read() {
            let mut r = Rig::new();
            r.surface(ramp(&row), Layout::Linear, 256);
            assert_eq!(call(0, 0).0, DUMP_ERR_MODE_UNREACHABLE);
            assert_eq!(call(2, 0).0, DUMP_ERR_MODE_UNREACHABLE);
            assert_eq!(call(4, 0).0, DUMP_ERR_MODE);
            assert_eq!(call(1, 0).0, DUMP_ERR_LAYOUT, "mode 1 without a super-tiled surface");
            // SAFETY: a null `pages` is refused before anything is read.
            assert_eq!(
                unsafe { gc_debug_depth_dump(3, 0, core::ptr::null_mut(), core::ptr::null_mut()) },
                DUMP_ERR_ARGS
            );
            r.surface(ramp(&row), Layout::Linear, 512);
            assert_eq!(call(3, 0).0, DUMP_ERR_LAYOUT, "a wider row is not the 64-wide map");
            r.surface(vec![CLEAR; 16], Layout::Linear, 256);
            assert_eq!(call(3, 0).0, DUMP_ERR_SURFACE, "a surface smaller than 16 KiB");
            r.surface(ramp(&sup(SuperOrder::Drm)), Layout::SuperTiled, 256);
            assert_eq!(call(3, 0).0, DUMP_ERR_LAYOUT, "mode 3 with a super-tiled surface");
        }
    }
}
