// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Tiled shadow targets resolved by the RS, the depth surface's layout, and
//! early depth (docs/notes/gc-tiled-wire.md W2-W5), on `backend_tests.rs`'s
//! rig. The W3 cases run in every build; the knob's (`gc_tiled_rt`) only
//! with `test-knobs`.

use super::*;

/// The RS block's length is what a tiled-shadow run keeps off the stream's
/// capacity for its end (`RS_BLIT_WORDS`).
#[test]
fn an_rs_copy_takes_the_words_a_run_keeps_for_it() {
    let mut buf = [0u32; 128];
    let mut c = GcCmd { words: buf.as_mut_ptr(), used: 0, cap: buf.len() as u32, overflow: false };
    let r = crate::vivante::tiled_rt::Run { shadow: 0x4000_1000, storage: 0x4010_0000, pitch: 256, w: 64, h: 64 };
    emit_rs(&mut c, &crate::vivante::tiled_rt::resolve_blit(&r));
    assert!(!c.overflow);
    assert_eq!(c.used, RS_BLIT_WORDS);
}

/// W3: only a surface the GL core never hands to a CPU reader is PE-private:
/// no texture plane (bit 30), not the window's (bit 31), single-sampled.
#[test]
fn pe_private_is_false_for_a_texture_the_window_and_a_4x_surface() {
    assert!(ds_cache::pe_private(&ds_key((5, 1), (0, 0))), "a depth renderbuffer");
    assert!(ds_cache::pe_private(&ds_key((5, 1), (6, 1))), "depth and stencil renderbuffers");
    assert!(!ds_cache::pe_private(&ds_key((0x4000_0005, 1), (0, 0))), "a depth texture");
    assert!(!ds_cache::pe_private(&ds_key((5, 1), (0x4000_0006, 1))), "a stencil texture");
    assert!(!ds_cache::pe_private(&ds_key((0x8000_0010, 0), (0x8000_0010, 0))), "the window");
    let ms = ds_cache::DsKey { scale: 2, ..ds_key((5, 1), (0, 0)) };
    assert!(!ds_cache::pe_private(&ms), "a 4x surface");
}

/// W3: a surface asked for SUPER_TILED gets it only when PE-private; a live
/// plane keeps the layout it has, a surface holding nothing live takes the
/// one asked for (its cleared value is the same words in any layout); a
/// live plane is copied within one layout and refused across two; and the
/// depth resolve reads nothing from a tiled surface.
#[test]
fn a_depth_surface_keeps_its_layout_while_a_plane_is_live_and_is_never_read_tiled() {
    use crate::vivante::layout::Layout::{Linear, SuperTiled};
    let _rig = Rig::new();
    let sel = |k: ds_cache::DsKey, l| ds_cache::select_in(k, l, 64, 64, GC_DEPTH_GUARD_BYTES, 1, None, |_| false, || 0);
    let tex = ds_key((0x4000_0005, 1), (0, 0));
    assert_eq!(sel(tex, SuperTiled).expect("a surface").layout, Linear, "a texture's surface stays linear");
    let a = ds_key((5, 1), (6, 1));
    assert_eq!(sel(a, SuperTiled).expect("a surface").layout, SuperTiled);
    ds_cache::drawn(a, 1, false, [true, true], [None, None]);
    assert_eq!(sel(a, Linear).expect("the same surface").layout, SuperTiled, "a live plane keeps the layout");
    assert_eq!(ds_cache::layout_of(a), Some(SuperTiled));
    /* depth 5 with another stencil: its live plane would have to move */
    let b = ds_key((5, 1), (7, 1));
    assert_eq!(sel(b, Linear), Err(-crate::util::ENOTSUP), "not copied across layouts");
    assert_eq!(sel(b, SuperTiled).expect("copied within one layout").layout, SuperTiled);
    ds_cache::drawn(b, 1, false, [true, true], [None, None]);
    /* the resolve: depth 5's plane is live in a SUPER_TILED surface */
    let mut out = [7.0f32; 4];
    // SAFETY: `out` holds 2 rows of 2 floats.
    let rc = unsafe { ds_cache::resolve_depth(5, out.as_mut_ptr(), 2, 2, 2, GC_DEPTH_GUARD_BYTES, |_| 0) };
    assert_eq!((rc, out), (1, [7.0; 4]), "a tiled surface is never read");
    /* a whole clear and nothing live: the next draw may take another layout */
    let c = ds_key((9, 1), (0, 0));
    assert_eq!(sel(c, SuperTiled).expect("a surface").layout, SuperTiled);
    ds_cache::drawn(c, 1, false, [false, false], [Some(1.0f32.to_bits()), None]);
    assert_eq!(sel(c, Linear).expect("the same surface").layout, Linear);
    assert_eq!(ds_cache::planes_of(c).map(|p| p[0].cleared_value()), Some(Some(1.0f32.to_bits())));
}

/// The knob's helpers: a 64x64 RGBA8 target of its own memory, a depth
/// renderbuffer (object 5, PE-private) tested LESS and written, no stencil.
#[cfg(feature = "test-knobs")]
mod knob {
    use super::*;
    use crate::vivante::layout::{self, Layout};
    use crate::vivante::tiled_rt::gc_debug_set_tiled_rt;

    /// The knob on for the length of a test, off again however it ends.
    pub struct On;
    impl On {
        pub fn new() -> On {
            gc_debug_set_tiled_rt(1);
            On
        }
    }
    impl Drop for On {
        fn drop(&mut self) {
            gc_debug_set_tiled_rt(0);
        }
    }

    pub const W: u32 = 64;
    pub const H: u32 = 64;

    pub fn target(rig: &Rig) -> (OsalGpuMem, GlesDraw) {
        let mut mem = OsalGpuMem::ZERO;
        assert_eq!(osal::gpu_alloc(GC_HEAP_DATA, (W * H * 4) as usize, OSAL_MEM_DATA, &mut mem), 0);
        let mut d = rig.draw(3);
        (d.target_gpu, d.target_cpu, d.target_w, d.target_h) = (mem.gpu, mem.cpu, W, H);
        d.targets[0] = GlesDrawTarget { gpu: mem.gpu, w: W, h: H, format: GLES_SURF_RGBA8888, out_shift: 0 };
        (d.depth_id, d.depth_gen, d.stencil_id, d.stencil_gen) = (5, 1, 0, 0);
        (d.depth_test, d.depth_write, d.depth_func) = (true, true, GLES_CMP_LESS);
        (mem, d)
    }

    /// One RS copy of a stream: the word index of its kicker, and RS_CONFIG,
    /// source, source stride, dest, dest stride and window as they stood.
    pub type Copy = (usize, [u32; 6]);

    /// Every RS copy of the stream, in order.
    pub fn rs_copies(s: &Stream) -> Vec<Copy> {
        let (mut regs, mut out) = (BTreeMap::new(), Vec::new());
        walk(&s.0, |r, v, i| {
            regs.insert(r, v);
            if r == GC_RS_KICKER {
                let g = |r: u32| regs.get(&r).copied().unwrap_or(0);
                let words = [
                    g(GC_RS_CONFIG),
                    g(rs_pipe_source_addr(0)),
                    g(GC_RS_SOURCE_STRIDE),
                    g(rs_pipe_dest_addr(0)),
                    g(GC_RS_DEST_STRIDE),
                    g(GC_RS_WINDOW_SIZE),
                ];
                out.push((i, words));
            }
        });
        out
    }

    /// Word index of each DRAW_INSTANCED.
    pub fn draw_words(s: &Stream) -> Vec<usize> {
        let (mut out, mut i) = (Vec::new(), 0usize);
        while i < s.0.len() {
            let w = s.0[i];
            if w & 0xf800_0000 == GC_OP_DRAW_INSTANCED {
                out.push(i);
            }
            i += match w & 0xf800_0000 {
                GC_OP_LOAD_STATE => 1 + ((w >> 16) & 0x3ff) as usize,
                GC_OP_DRAW_INSTANCED => 4,
                GC_OP_STALL => 2,
                _ => 1,
            };
        }
        out
    }

    /// `n` bytes of the rig's memory at `gpu`.
    pub fn read(gpu: u32, n: usize) -> Vec<u8> {
        with_platform(|p| {
            let gpu = u64::from(gpu);
            let (base, a) = p.allocs.range(..=gpu).next_back().expect("an allocation");
            let off = (gpu - base) as usize;
            assert!(off + n <= a.size, "0x{gpu:x}+{n} runs off the allocation at 0x{base:x}");
            // SAFETY: inside a live allocation of the rig (checked above).
            unsafe { core::slice::from_raw_parts((a.mem.as_ptr() as *const u8).add(off), n) }.to_vec()
        })
    }

    /// Write `bytes` into the rig's memory at `gpu`.
    pub fn write(gpu: u32, bytes: &[u8]) {
        with_platform(|p| {
            let gpu = u64::from(gpu);
            let (base, a) = p.allocs.range_mut(..=gpu).next_back().expect("an allocation");
            let off = (gpu - base) as usize;
            assert!(off + bytes.len() <= a.size, "0x{gpu:x}+{} runs off the allocation", bytes.len());
            // SAFETY: inside a live allocation of the rig (checked above).
            let dst = unsafe { core::slice::from_raw_parts_mut((a.mem.as_mut_ptr() as *mut u8).add(off), bytes.len()) };
            dst.copy_from_slice(bytes);
        })
    }

    /// The byte offset of pixel (x, y) of one side of an RS copy: a TILED
    /// side's stride is given times four, a linear one's as it is; neither
    /// carries the TILING or SUPER_TILED_NEW bit, which are a super-tiled
    /// side's.
    fn side(tiled: bool, stride: u32) -> impl Fn(u32, u32) -> usize {
        assert_eq!(stride & 0xf800_0000, 0, "no TILING or SUPER_TILED_NEW bit for a linear or TILED side");
        let (layout, st) = if tiled { (Layout::Tiled, stride >> 2) } else { (Layout::Linear, stride) };
        move |x, y| layout::offset(layout, x, y, 4, st)
    }

    /// The RS interpreter: one recorded copy with Mesa's semantics for the
    /// subset the shadows use (4-byte pixels, a linear or 4x4-TILED side, no
    /// fill, no TS), through `layout::offset`. It shows that the words agree
    /// with layout.rs, not what the hardware does.
    pub fn run_rs(c: &[u32; 6]) {
        let [config, src, src_stride, dst, dst_stride, window] = *c;
        assert_eq!(config & 0x1f, GC_RS_FORMAT_A8R8G8B8, "a bit copy of 4-byte pixels");
        assert_eq!((config >> 8) & 0x1f, GC_RS_FORMAT_A8R8G8B8);
        let (w, h) = (window & 0xffff, window >> 16);
        let from = side(config & GC_RS_SOURCE_TILED != 0, src_stride);
        let to = side(config & GC_RS_DEST_TILED != 0, dst_stride);
        let span = |off: &dyn Fn(u32, u32) -> usize| {
            (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| off(x, y)).max().unwrap_or(0) + 4
        };
        let s = read(src, span(&from));
        let mut d = read(dst, span(&to));
        for y in 0..h {
            for x in 0..w {
                let (a, b) = (from(x, y), to(x, y));
                d[b..b + 4].copy_from_slice(&s[a..a + 4]);
            }
        }
        write(dst, &d);
    }

    pub fn pixel(x: u32, y: u32) -> [u8; 4] {
        [x as u8, y as u8, (x ^ y) as u8, 0xa5]
    }

    pub const LATE_RA: u32 = GC_RA_EARLY_DEPTH_BASE | GC_RA_EARLY_DEPTH_FORWARD_Z | GC_RA_EARLY_DEPTH_FORWARD_W;
}

/// W4 and W5: with the knob on, a depth-tested draw into a 64x64 RGBA8
/// target with a depth renderbuffer renders into a page-aligned TILED shadow
/// under SINGLE_BUFFER(2), its depth SUPER_TILED, with early depth on both
/// halves as Mesa's depth-tested draw has it (golden mode 8,
/// tools/vivante/testdata/ts-clear.golden: PE_DEPTH_CONFIG 0x05051111,
/// RA_EARLY_DEPTH 0x05000031). The stream tiles the storage in before the
/// draw (the draw does not clear) and copies the shadow back after it, in
/// the same kick; the RS interpreter run over both copies takes a pattern
/// from the storage into the shadow and back.
#[cfg(feature = "test-knobs")]
#[test]
fn a_depth_tested_draw_into_a_tiled_shadow_resolves_into_its_storage_in_the_same_stream() {
    use crate::vivante::layout::{self, Layout};
    use knob::*;
    let rig = Rig::new();
    let _on = On::new();
    let (mut mem, d) = target(&rig);
    let st = rig.submit(&d);
    let shadow = st.reg(pe_pipe_color_addr(0));
    assert_eq!(shadow & 0xfff, 0, "the shadow starts on a page");
    assert_ne!(u64::from(shadow), mem.gpu, "the PE never writes the linear storage");
    assert_eq!(st.reg(pe_pipe_color_addr(1)), shadow);
    assert_eq!(st.reg(GC_PE_COLOR_STRIDE), W * 4, "rows of the 16-padded width");
    assert_eq!((st.reg(GC_PE_LOGIC_OP) >> 8) & 3, 2, "SINGLE_BUFFER(2): a tiled target");
    /* golden mode 9's target is super-tiled; this one is TILED, as Mesa's
     * 128-bit targets are: the two SUPER_TILED bits differ, by design */
    let f = st.reg(GC_PE_COLOR_FORMAT);
    assert_eq!(f & (GC_PE_COLOR_FORMAT_SUPER_TILED | GC_PE_COLOR_FORMAT_SUPER_TILED_NEW), 0);
    assert_eq!(st.reg(GC_PE_DEPTH_CONFIG), 0x0505_1111, "golden mode 8's depth-tested draw");
    assert_eq!(st.reg(GC_RA_EARLY_DEPTH), 0x0500_0031, "golden mode 8's RA_EARLY_DEPTH");
    let copies = rs_copies(&st);
    let draws = draw_words(&st);
    assert_eq!((copies.len(), draws.len()), (2, 1), "the tile-in, the draw, the resolve: {copies:x?}");
    let (tile_in, resolve) = (copies[0], copies[1]);
    assert!(tile_in.0 < draws[0] && resolve.0 > draws[0], "the tile-in before the draw, the resolve after it");
    let fmt = GC_RS_FORMAT_A8R8G8B8;
    let fmts = rs_source_format(fmt) | rs_dest_format(fmt);
    let window = rs_window_width(W) | rs_window_height(H);
    let storage = mem.gpu as u32;
    assert_eq!(tile_in.1, [fmts | GC_RS_DEST_TILED, storage, W * 4, shadow, (W * 4) << 2, window]);
    assert_eq!(resolve.1, [fmts | GC_RS_SOURCE_TILED, shadow, (W * 4) << 2, storage, W * 4, window]);
    /* the RS interpreter: storage -> shadow -> storage gives every pixel back */
    let image: Vec<u8> = (0..H).flat_map(|y| (0..W).flat_map(move |x| pixel(x, y))).collect();
    write(storage, &image);
    run_rs(&tile_in.1);
    let sh = read(shadow, (W * H * 4) as usize);
    for (x, y) in [(0, 0), (1, 0), (0, 1), (4, 0), (0, 4), (17, 5), (63, 63)] {
        let at = layout::offset(Layout::Tiled, x, y, 4, W * 4);
        assert_eq!(sh[at..at + 4], pixel(x, y), "({x},{y}) in the shadow");
    }
    write(storage, &vec![0u8; image.len()]);
    run_rs(&resolve.1);
    assert!(read(storage, image.len()) == image, "every pixel back in the storage, at its place");
    assert_eq!(crate::vivante::tiled_rt::counts().resolves, 1);
    assert!(rig.faults().is_empty());
    osal::gpu_free(&mut mem);
}

/// The knob off, or a draw the rule leaves out (a width the RS does not
/// copy whole, a depth texture, the window's depth, no depth attachment),
/// keeps the linear stream: the target itself, SINGLE_BUFFER(1), no
/// SUPER_TILED depth, no early depth, no RS copy.
#[cfg(feature = "test-knobs")]
#[test]
fn without_the_knob_or_outside_the_rule_a_draw_keeps_the_linear_stream() {
    use knob::*;
    let rig = Rig::new();
    let (mut mem, d) = target(&rig);
    let linear = |st: &Stream, what: &str| {
        assert_eq!(u64::from(st.reg(pe_pipe_color_addr(0))), d.target_gpu, "{what}: the target");
        assert_eq!((st.reg(GC_PE_LOGIC_OP) >> 8) & 3, 1, "{what}: SINGLE_BUFFER(1)");
        let z = st.reg(GC_PE_DEPTH_CONFIG);
        assert_eq!(z & (GC_PE_DEPTH_SUPER_TILED | GC_PE_DEPTH_EARLY_Z), 0, "{what}: 0x{z:08x}");
        assert_eq!(st.reg(GC_RA_EARLY_DEPTH), LATE_RA | GC_RA_EARLY_DEPTH_WRITE_DISABLE, "{what}");
        assert!(rs_copies(st).is_empty(), "{what}: no RS copy");
    };
    linear(&rig.submit(&d), "the knob off");
    let _on = On::new();
    let mut narrow = d;
    /* a width the RS does not copy whole; depth 6, a surface of its own */
    (narrow.target_w, narrow.targets[0].w, narrow.depth_id) = (56, 56, 6);
    linear(&rig.submit(&narrow), "56 wide");
    let mut tex = d;
    tex.depth_id = 0x4000_0007;
    linear(&rig.submit(&tex), "a depth texture");
    let mut window = d;
    (window.depth_id, window.stencil_id) = (0x8000_0001, 0x8000_0001);
    linear(&rig.submit(&window), "the window's depth");
    let mut none = d;
    none.depth_id = 0;
    linear(&rig.submit(&none), "no depth attachment");
    assert!(rig.faults().is_empty());
    osal::gpu_free(&mut mem);
}

/// A run that opens with a whole, unmasked colour clear needs no tile-in: the
/// clear quad (not the RS fill, which describes a linear target) writes every
/// pixel of the shadow. A masked clear tiles in first.
#[cfg(feature = "test-knobs")]
#[test]
fn a_run_opened_by_a_whole_colour_clear_tiles_nothing_in() {
    use knob::*;
    let rig = Rig::new();
    gc_debug_rs_clear(true);
    let _on = On::new();
    let (mut mem, mut d) = target(&rig);
    (d.do_clear, d.do_clear_depth, d.clear_depth) = (true, true, 1.0);
    let st = rig.submit(&d);
    let copies = rs_copies(&st);
    assert_eq!(copies.len(), 1, "the resolve alone, no RS fill: {copies:x?}");
    assert_eq!(copies[0].1[0] & GC_RS_SOURCE_TILED, GC_RS_SOURCE_TILED, "the resolve");
    assert_eq!(draw_words(&st).len(), 2, "the clear quad and the draw");
    d.clear_color_mask = 0x7;
    let st = rig.submit(&d);
    let copies = rs_copies(&st);
    assert_eq!(copies.len(), 2, "a masked clear keeps what the alpha held: {copies:x?}");
    assert!(copies[0].0 < draw_words(&st)[0], "tiled in before the clear quad");
    assert!(rig.faults().is_empty());
    osal::gpu_free(&mut mem);
}

/// Two draws into the shadow in one stream are one run: one tile-in, one
/// resolve, the PE group written once (the second draw's key is a hit). A
/// draw into another target after them ends the run there, before it, and
/// rewrites the PE group and the RA word.
#[cfg(feature = "test-knobs")]
#[test]
fn draws_into_one_shadow_are_one_run_and_another_target_ends_it() {
    use knob::*;
    let rig = Rig::new();
    let _on = On::new();
    let (mut mem, d) = target(&rig);
    let other = rig.draw(3);
    // SAFETY: the draws are live for the length of the calls.
    unsafe {
        assert_eq!(gc_draw(&d), 0);
        assert_eq!(gc_draw(&d), 0);
        assert_eq!(gc_draw(&other), 0);
    }
    assert_eq!(gc_flush(), 0);
    let st = rig.last_stream();
    let copies = rs_copies(&st);
    let draws = draw_words(&st);
    assert_eq!((copies.len(), draws.len()), (2, 3), "{copies:x?}");
    assert!(copies[0].0 < draws[0], "the tile-in first");
    assert!(copies[1].0 > draws[1] && copies[1].0 < draws[2], "the resolve between the run and the next target");
    assert_eq!(st.writes(GC_PE_LOGIC_OP).1, 2, "the PE group: the run's once, the other target's once");
    assert_eq!(st.writes(GC_RA_EARLY_DEPTH).1, 2, "the RA word: early, then linear");
    assert!(rig.faults().is_empty());
    osal::gpu_free(&mut mem);
}

/// W5's exceptions, from Mesa's rule (early_z.rs): a discarding program
/// still tests early but writes late (RA WRITE_DISABLE, no DISABLE_ZS); a
/// program that writes gl_FragDepth gets no early test and writes late; and
/// a clear quad into the shadow is never early, with its depth SUPER_TILED.
#[cfg(feature = "test-knobs")]
#[test]
fn a_discard_writes_depth_late_and_gl_fragdepth_and_clears_get_no_early_depth() {
    use knob::*;
    let mut rig = Rig::new();
    let _on = On::new();
    let (mut mem, d) = target(&rig);
    rig.fs.uses_discard = 1;
    let mut dd = d;
    dd.fs = &*rig.fs;
    let st = rig.submit(&dd);
    let want = LATE_RA | GC_RA_EARLY_DEPTH_TEST_ENABLE | GC_RA_EARLY_DEPTH_WRITE_DISABLE;
    assert_eq!(st.reg(GC_RA_EARLY_DEPTH), want, "a discard");
    let z = st.reg(GC_PE_DEPTH_CONFIG);
    assert_eq!(z & (GC_PE_DEPTH_EARLY_Z | GC_PE_DEPTH_DISABLE_ZS), GC_PE_DEPTH_EARLY_Z, "0x{z:08x}");
    rig.fs.uses_discard = 0;
    rig.fs.viv.ps_depth_out_reg = 0;
    let mut fd = d;
    fd.fs = &*rig.fs;
    let st = rig.submit(&fd);
    assert_eq!(st.reg(GC_RA_EARLY_DEPTH), LATE_RA | GC_RA_EARLY_DEPTH_WRITE_DISABLE, "gl_FragDepth");
    assert_eq!(st.reg(GC_PE_DEPTH_CONFIG) & (GC_PE_DEPTH_EARLY_Z | GC_PE_DEPTH_DISABLE_ZS), 0);
    rig.fs.viv.ps_depth_out_reg = SHC_VIV_NO_REG;
    let mut clear = d;
    (clear.do_clear, clear.index_count) = (true, 0);
    clear.fs = &*rig.fs;
    let st = rig.submit(&clear);
    assert_eq!(draw_words(&st).len(), 1, "the clear quad");
    assert_eq!(st.reg(GC_RA_EARLY_DEPTH), LATE_RA | GC_RA_EARLY_DEPTH_WRITE_DISABLE, "the clear quad");
    let z = st.reg(GC_PE_DEPTH_CONFIG);
    assert_eq!(z & (GC_PE_DEPTH_EARLY_Z | GC_PE_DEPTH_SUPER_TILED), GC_PE_DEPTH_SUPER_TILED, "0x{z:08x}");
    assert!(rig.faults().is_empty());
    osal::gpu_free(&mut mem);
}
