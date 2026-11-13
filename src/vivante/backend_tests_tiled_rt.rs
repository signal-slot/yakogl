// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Tiled colour shadows, early depth and depth-surface layouts
//! (`tiled_rt.rs`, `ds_cache.rs`; docs/notes/gc-tiled-wire.md W3-W5), on the
//! rig of `backend_tests.rs`. Every test that turns the knob on needs the
//! `test-knobs` feature (`tools/check.sh`'s knob row names them).

use super::*;
use crate::vivante::layout::Layout;
#[cfg(feature = "test-knobs")]
use crate::vivante::{layout, tiled_rt};

/// What the resolve engine was told at one RS_KICKER write.
#[derive(Clone, Copy, Debug, Default)]
struct RsJob {
    config: u32,
    src: u32,
    src_stride: u32,
    dst: u32,
    dst_stride: u32,
    window: u32,
    clear_control: u32,
    fill: u32,
}

/// Every RS kick of the stream, in order, with the registers as the RS block
/// before it left them.
fn rs_jobs(s: &Stream) -> Vec<RsJob> {
    let mut cur = RsJob::default();
    let mut out = Vec::new();
    walk(&s.0, |reg, v, _| match reg {
        GC_RS_CONFIG => cur.config = v,
        GC_RS_SOURCE_STRIDE => cur.src_stride = v,
        GC_RS_DEST_STRIDE => cur.dst_stride = v,
        GC_RS_WINDOW_SIZE => cur.window = v,
        GC_RS_CLEAR_CONTROL => cur.clear_control = v,
        r if r == rs_fill_value(0) => cur.fill = v,
        r if r == rs_pipe_source_addr(0) => cur.src = v,
        r if r == rs_pipe_dest_addr(0) => cur.dst = v,
        GC_RS_KICKER => out.push(cur),
        _ => {}
    });
    out
}

#[cfg(feature = "test-knobs")]
/// Where in the stream each RS kick is (word index).
fn rs_kicks_at(s: &Stream) -> Vec<usize> {
    let mut v = Vec::new();
    walk(&s.0, |r, _, i| {
        if r == GC_RS_KICKER {
            v.push(i)
        }
    });
    v
}

#[cfg(feature = "test-knobs")]
/// The RS interpreter: a 32-bit copy of a window, each side linear or
/// 4x4-tiled as the job says (the stride times 4 for a tiled side, Mesa's
/// `etna_compile_rs_state`), through `layout::offset`. A fill job writes its
/// word to every pixel of the destination window. It shows the job agrees
/// with `layout.rs`; it does not show what the hardware does.
fn rs_run(j: &RsJob) {
    let (w, h) = (j.window & 0xffff, j.window >> 16);
    let (src_tiled, dst_tiled) = (j.config & GC_RS_SOURCE_TILED != 0, j.config & GC_RS_DEST_TILED != 0);
    let side = |tiled: bool, stride: u32| if tiled { (Layout::Tiled, stride >> 2) } else { (Layout::Linear, stride) };
    let (sl, ss) = side(src_tiled, j.src_stride);
    let (dl, ds) = side(dst_tiled, j.dst_stride);
    let fill = j.clear_control & GC_RS_CLEAR_CONTROL_MODE_ENABLED1 != 0;
    with_platform(|p| {
        let find = |p: &mut Platform, gpu: u64| -> (*mut u8, usize) {
            let (base, a) = p.allocs.range_mut(..=gpu).next_back().expect("an allocation");
            ((a.mem.as_mut_ptr() as *mut u8).wrapping_add((gpu - base) as usize), a.size - (gpu - base) as usize)
        };
        let (sp, _) = find(p, u64::from(j.src));
        let (dp, dn) = find(p, u64::from(j.dst));
        for y in 0..h {
            for x in 0..w {
                let to = layout::offset(dl, x, y, 4, ds);
                assert!(to + 4 <= dn, "the RS wrote past its destination: ({x},{y}) at {to}");
                let px = if fill {
                    j.fill.to_ne_bytes()
                } else {
                    let from = layout::offset(sl, x, y, 4, ss);
                    // SAFETY: inside the source allocation by the caller's sizes.
                    unsafe { *(sp.add(from) as *const [u8; 4]) }
                };
                // SAFETY: bounds checked just above.
                unsafe { *(dp.add(to) as *mut [u8; 4]) = px };
            }
        }
    });
}

#[cfg(feature = "test-knobs")]
fn px(x: u32, y: u32) -> u32 {
    0x0100_0000 | (y << 12) | x
}

/// A 64x64 colour target with a CPU view and a depth renderbuffer: the draw
/// qualifies for a shadow once the knob is on.
fn shadow_target(rig: &Rig) -> (OsalGpuMem, GlesDraw) {
    let mut mem = OsalGpuMem::ZERO;
    assert_eq!(osal::gpu_alloc(GC_HEAP_DATA, 64 * 64 * 4, OSAL_MEM_DATA, &mut mem), 0);
    let mut d = rig.draw(3);
    (d.target_gpu, d.target_cpu) = (mem.gpu, mem.cpu);
    d.targets[0].gpu = mem.gpu;
    (d.depth_id, d.stencil_id) = (0x65, 0x65); /* a renderbuffer: no window or texture bit */
    (d.depth_gen, d.stencil_gen) = (1, 1);
    (d.depth_test, d.depth_write, d.depth_func) = (true, true, GLES_CMP_LEQUAL);
    (d.ds_w, d.ds_h) = (64, 64);
    (mem, d)
}

/// The knob on for the life of the guard.
#[cfg(feature = "test-knobs")]
struct TiledRt;
#[cfg(feature = "test-knobs")]
impl TiledRt {
    fn on() -> TiledRt {
        gc_debug_set_tiled_rt(1);
        TiledRt
    }
}
#[cfg(feature = "test-knobs")]
impl Drop for TiledRt {
    fn drop(&mut self) {
        gc_debug_set_tiled_rt(0);
    }
}

/// Without the `test-knobs` feature there is no knob and no draw takes a
/// shadow, whatever it asks.
#[cfg(not(feature = "test-knobs"))]
#[test]
fn without_the_knob_feature_no_draw_takes_a_shadow() {
    let rig = Rig::new();
    let (mut mem, d) = shadow_target(&rig);
    let s = rig.submit(&d);
    assert_eq!(u64::from(s.reg(pe_pipe_color_addr(0))), mem.gpu);
    assert!(rs_jobs(&s).is_empty());
    assert_eq!(s.reg(GC_PE_DEPTH_CONFIG) & (GC_PE_DEPTH_SUPER_TILED | GC_PE_DEPTH_EARLY_Z), 0);
    osal::gpu_free(&mut mem);
}

/// W4. A depth-tested draw into a 64x64 RGBA8 target with a depth
/// renderbuffer renders into a TILED shadow under SINGLE_BUFFER(2), its depth
/// super-tiled, and the same stream tiles the storage in before the draw and
/// copies the shadow back after it, through the resolve engine. Off, it is
/// the stream of today: the storage, SINGLE_BUFFER(1), no RS, a linear depth.
///
/// Deliberate break: drop the `frame_submit` untile and the last RS job is
/// missing; put `GC_RS_DEST_STRIDE_TILING` on the tiled side and the stride
/// assert fails.
#[cfg(feature = "test-knobs")]
#[test]
fn a_depth_tested_draw_renders_into_a_tiled_shadow_the_rs_copies_back() {
    let rig = Rig::new();
    let (mut mem, d) = shadow_target(&rig);

    let off = rig.submit(&d);
    assert_eq!(u64::from(off.reg(pe_pipe_color_addr(0))), mem.gpu, "off: the storage");
    assert_eq!((off.reg(GC_PE_LOGIC_OP) >> 8) & 3, 1);
    assert_eq!(off.reg(GC_PE_DEPTH_CONFIG) & GC_PE_DEPTH_SUPER_TILED, 0);
    assert!(rs_jobs(&off).is_empty(), "off: no resolve");

    /* the depth the linear draw wrote is a live plane in a linear surface,
     * which stays so (`a_depth_surface_keeps_its_layout...`): the object's
     * contents are dropped, as glInvalidateFramebuffer would */
    gc_attachment_gone(0x65);
    let _on = TiledRt::on();
    let on = rig.submit(&d);
    let shadow = on.reg(pe_pipe_color_addr(0));
    assert_eq!(shadow & 0xfff, 0, "a page");
    assert_ne!(u64::from(shadow), mem.gpu);
    assert_eq!(on.reg(pe_pipe_color_addr(1)), shadow);
    assert_eq!(on.reg(GC_PE_COLOR_STRIDE), 64 * 4);
    assert_eq!((on.reg(GC_PE_LOGIC_OP) >> 8) & 3, 2, "SINGLE_BUFFER(2)");
    let f = on.reg(GC_PE_COLOR_FORMAT);
    assert_eq!(f & (GC_PE_COLOR_FORMAT_SUPER_TILED | GC_PE_COLOR_FORMAT_SUPER_TILED_NEW), 0, "TILED, not super-tiled");
    assert_ne!(on.reg(GC_PE_DEPTH_CONFIG) & GC_PE_DEPTH_SUPER_TILED, 0, "the depth surface is super-tiled");
    assert_eq!(ds_cache::layout_of(ds_key((0x65, 1), (0x65, 1))), Layout::SuperTiled);

    let jobs = rs_jobs(&on);
    assert_eq!(jobs.len(), 2, "tile in, then copy back");
    let (tile_in, untile) = (jobs[0], jobs[1]);
    assert!(rs_kicks_at(&on)[1] > on.upto_draw(0).0.len(), "the copy back follows the draw");
    assert_eq!((tile_in.src, tile_in.dst), (mem.gpu as u32, shadow));
    assert_eq!(tile_in.config & (GC_RS_SOURCE_TILED | GC_RS_DEST_TILED), GC_RS_DEST_TILED);
    assert_eq!(tile_in.dst_stride, (64 * 4) << 2, "a tiled side's stride is times four, no TILING bit");
    assert_eq!(tile_in.src_stride, 64 * 4);
    assert_eq!((untile.src, untile.dst), (shadow, mem.gpu as u32));
    assert_eq!(untile.config & (GC_RS_SOURCE_TILED | GC_RS_DEST_TILED), GC_RS_SOURCE_TILED);
    assert_eq!(untile.src_stride, (64 * 4) << 2);
    assert_eq!(untile.dst_stride, 64 * 4, "the storage's own pitch");
    for j in &jobs {
        assert_eq!(j.window, rs_window_width(64) | rs_window_height(64));
        assert_eq!(j.clear_control, 0, "a copy, not a fill");
    }

    /* the interpreter on the stream's jobs: the storage tiled in lands at
     * layout::offset(Tiled), and what comes back is every pixel at its own
     * place */
    // SAFETY: the 16 KiB just allocated, with a CPU view.
    let lin = unsafe { core::slice::from_raw_parts_mut(mem.cpu as *mut u32, 64 * 64) };
    for y in 0..64u32 {
        for x in 0..64u32 {
            lin[(y * 64 + x) as usize] = px(x, y);
        }
    }
    rs_run(&tile_in);
    let sh = gpu_bytes(u64::from(shadow), 64 * 64 * 4);
    for (x, y) in [(0, 0), (5, 3), (17, 40), (63, 63), (4, 4)] {
        let at = layout::offset(Layout::Tiled, x, y, 4, 64 * 4);
        assert_eq!(u32::from_ne_bytes(sh[at..at + 4].try_into().unwrap()), px(x, y), "({x},{y}) tiled in");
    }
    lin.fill(0);
    rs_run(&untile);
    for y in 0..64u32 {
        for x in 0..64u32 {
            assert_eq!(lin[(y * 64 + x) as usize], px(x, y), "({x},{y}) came back at its own place");
        }
    }
    assert!(rig.faults().is_empty());
    osal::gpu_free(&mut mem);
}

/// W5. Into a shadow a depth-tested, depth-writing draw of a program that
/// neither discards nor writes depth takes early depth, both halves from one
/// decision: EARLY_Z and DISABLE_ZS in PE_DEPTH_CONFIG, TEST_ENABLE in
/// RA_EARLY_DEPTH and no WRITE_DISABLE. A program that discards (the
/// compiler's flag, or a TEXKILL in the words) keeps the test early and the
/// write late, as Mesa's `etna_update_zsa` does; one that writes gl_FragDepth
/// neither; and a stencil test, a linear target (the knob off) and the clear
/// quad keep today's words.
#[cfg(feature = "test-knobs")]
#[test]
fn a_draw_into_a_tiled_shadow_takes_early_depth_and_nothing_else_does() {
    let rig = Rig::new();
    let (mut mem, d) = shadow_target(&rig);
    /* Mesa's two decisions: the test early (both halves say so), and the
     * write early (the RA writes, and the PE has nothing left to do) */
    let early = |s: &Stream, what: &str, test: bool, write: bool| {
        let (pe, ra) = (s.reg(GC_PE_DEPTH_CONFIG), s.reg(GC_RA_EARLY_DEPTH));
        assert_eq!(pe & GC_PE_DEPTH_EARLY_Z != 0, test, "{what}: PE EARLY_Z");
        assert_eq!(ra & GC_RA_EARLY_DEPTH_TEST_ENABLE != 0, test, "{what}: RA TEST_ENABLE");
        assert_eq!(ra & GC_RA_EARLY_DEPTH_WRITE_DISABLE == 0, write, "{what}: the RA writes depth");
        assert_eq!(pe & GC_PE_DEPTH_DISABLE_ZS != 0, test && write, "{what}: the PE does nothing late");
    };

    early(&rig.submit(&d), "knob off", false, false);
    gc_attachment_gone(0x65);
    let _on = TiledRt::on();
    early(&rig.submit(&d), "a plain depth-tested draw", true, true);

    let mut fs = *rig.fs;
    fs.uses_discard = 1;
    let mut e = d;
    e.fs = &fs;
    let s = rig.submit(&e);
    early(&s, "discard (compiler flag): the test stays early, the write late", true, false);
    assert_ne!(s.reg(GC_PE_DEPTH_CONFIG) & GC_PE_DEPTH_SUPER_TILED, 0, "still a shadow draw");

    let texkill = [0x17u32, 0, 0, 0, 0, 0, 0, 0];
    let mut fs = *rig.fs;
    (fs.usc, fs.usc_dw) = (texkill.as_ptr(), texkill.len() as u32);
    e.fs = &fs;
    early(&rig.submit(&e), "a TEXKILL in the words", true, false);

    let mut fs = *rig.fs;
    fs.viv.ps_depth_out_reg = 2;
    e.fs = &fs;
    early(&rig.submit(&e), "gl_FragDepth", false, false);

    let mut e = d;
    e.stencil = true;
    early(&rig.submit(&e), "a stencil test", false, false);

    /* the clear quad in front of a draw: the first draw of the stream */
    let mut c = d;
    (c.do_clear, c.do_clear_depth, c.clear_depth) = (true, true, 1.0);
    let s = rig.submit(&c);
    let quad = s.upto_draw(0);
    assert_eq!(quad.reg(GC_PE_DEPTH_CONFIG) & GC_PE_DEPTH_EARLY_Z, 0, "the clear quad: no early depth");
    assert_ne!(quad.reg(GC_RA_EARLY_DEPTH) & GC_RA_EARLY_DEPTH_WRITE_DISABLE, 0);
    assert_eq!(quad.reg(GC_RA_EARLY_DEPTH) & GC_RA_EARLY_DEPTH_TEST_ENABLE, 0);
    early(&s, "the draw behind the clear", true, true);
    assert!(rig.faults().is_empty());
    osal::gpu_free(&mut mem);
}

/// Draws that do not qualify keep today's stream: a width the resolve engine
/// cannot take, a depth texture, the window's depth, and a draw with no depth
/// test.
#[cfg(feature = "test-knobs")]
#[test]
fn draws_that_do_not_qualify_for_a_shadow_are_drawn_as_before() {
    let rig = Rig::new();
    let (mut mem, d) = shadow_target(&rig);
    let _on = TiledRt::on();
    let plain = |s: &Stream, what: &str| {
        assert_eq!(u64::from(s.reg(pe_pipe_color_addr(0))), mem.gpu, "{what}: the storage");
        assert_eq!((s.reg(GC_PE_LOGIC_OP) >> 8) & 3, 1, "{what}");
        assert_eq!(s.reg(GC_PE_DEPTH_CONFIG) & (GC_PE_DEPTH_SUPER_TILED | GC_PE_DEPTH_EARLY_Z), 0, "{what}");
        assert!(rs_jobs(s).is_empty(), "{what}: no resolve");
    };
    let mut e = d;
    (e.target_w, e.target_h) = (60, 60);
    e.targets[0].w = 60;
    e.targets[0].h = 60;
    plain(&rig.submit(&e), "60x60");
    let mut e = d;
    (e.depth_id, e.stencil_id) = (0x4000_0065, 0x4000_0065);
    plain(&rig.submit(&e), "a depth texture");
    let mut e = d;
    (e.depth_id, e.stencil_id) = (0x8000_0065, 0x8000_0065);
    plain(&rig.submit(&e), "the window's depth");
    let mut e = d;
    e.depth_test = false;
    plain(&rig.submit(&e), "no depth test");
    assert!(rig.faults().is_empty());
    osal::gpu_free(&mut mem);
}

/// A draw that opens with a whole colour clear tiles nothing in: the one RS
/// job is the copy back. With the RS clear on, the fill goes into the shadow
/// at the shadow's stride, and the copy back follows it.
#[cfg(feature = "test-knobs")]
#[test]
fn a_shadow_draw_that_clears_the_colour_tiles_nothing_in() {
    let rig = Rig::new();
    let (mut mem, mut d) = shadow_target(&rig);
    (d.do_clear, d.clear_rgba) = (true, [1.0, 0.0, 0.0, 1.0]);
    let _on = TiledRt::on();
    let s = rig.submit(&d);
    let jobs = rs_jobs(&s);
    assert_eq!(jobs.len(), 1, "the copy back only");
    assert_eq!(jobs[0].config & GC_RS_SOURCE_TILED, GC_RS_SOURCE_TILED);
    assert_eq!(jobs[0].dst, mem.gpu as u32);

    gc_debug_rs_clear(true);
    let s = rig.submit(&d);
    gc_debug_rs_clear(false);
    let jobs = rs_jobs(&s);
    assert_eq!(jobs.len(), 2, "the fill, then the copy back");
    assert_ne!(jobs[0].clear_control & GC_RS_CLEAR_CONTROL_MODE_ENABLED1, 0);
    assert_eq!(jobs[0].dst, jobs[1].src, "the fill fills the shadow");
    assert_ne!(u64::from(jobs[0].dst), mem.gpu);
    assert_eq!(jobs[0].dst_stride, 64 * 4);
    assert!(rig.faults().is_empty());
    osal::gpu_free(&mut mem);
}

/// One stream, A (a shadow draw), B (a draw that does not take one), A:
/// A's shadow is copied back before B's draw, and A takes its shadow again
/// with no second tile-in, because it is in step. Three RS jobs in all: tile
/// in A, copy back A (before B), copy back A (the end).
#[cfg(feature = "test-knobs")]
#[test]
fn a_draw_elsewhere_finds_the_shadow_written_back_and_a_return_needs_no_tile_in() {
    let rig = Rig::new();
    let (mut a, da) = shadow_target(&rig);
    let mut db = draw_into(&rig, TARGET_GPU + 0x40_0000);
    db.depth_test = false;
    let _on = TiledRt::on();
    let k = rig.kicks();
    // SAFETY: live draws for the calls.
    unsafe {
        assert_eq!(gc_draw(&da), 0);
        assert_eq!(gc_draw(&db), 0);
        assert_eq!(gc_draw(&da), 0);
    }
    assert_eq!(gc_flush(), 0);
    assert_eq!(rig.kicks(), k + 1, "one kick");
    let s = rig.last_stream();
    let jobs = rs_jobs(&s);
    assert_eq!(jobs.len(), 3, "{jobs:?}");
    assert_eq!(jobs[0].config & GC_RS_DEST_TILED, GC_RS_DEST_TILED);
    assert_eq!(jobs[1].config & GC_RS_SOURCE_TILED, GC_RS_SOURCE_TILED);
    assert_eq!(jobs[2].config & GC_RS_SOURCE_TILED, GC_RS_SOURCE_TILED);
    assert_eq!(s.draws().len(), 3);
    let at = rs_kicks_at(&s);
    assert!(at[1] > s.upto_draw(0).0.len() && at[1] < s.upto_draw(1).0.len(), "A is written back before B's draw");
    assert!(at[2] > s.upto_draw(2).0.len(), "and again at the end");
    assert!(rig.faults().is_empty());
    osal::gpu_free(&mut a);
}

/// A draw into storage the GL core drops while its shadow is the newer copy
/// is not written back into released storage.
#[cfg(feature = "test-knobs")]
#[test]
fn released_storage_is_never_written_back_by_the_stream() {
    let rig = Rig::new();
    let (mut mem, d) = shadow_target(&rig);
    let _on = TiledRt::on();
    // SAFETY: a live draw for the call.
    assert_eq!(unsafe { gc_draw(&d) }, 0);
    gc_texture_released(mem.gpu, 64 * 64 * 4);
    assert_eq!(gc_flush(), 0);
    let jobs = rs_jobs(&rig.last_stream());
    assert_eq!(jobs.len(), 1, "the tile in only: {jobs:?}");
    assert_eq!(tiled_rt::held(), 0, "the shadow went with its storage");
    assert!(rig.faults().is_empty());
    osal::gpu_free(&mut mem);
}

/// A stream filled with shadow draws still ends with its copy back: the room
/// for it is kept while a shadow is active, and a draw that overflows is
/// recorded again in the next stream (which tiles in again).
#[cfg(feature = "test-knobs")]
#[test]
fn the_copy_back_survives_a_stream_filled_with_shadow_draws() {
    let rig = Rig::new();
    let (mut mem, d) = shadow_target(&rig);
    let _on = TiledRt::on();
    /* a draw after an identical one writes no state group (dedup.rs) */
    let _ = rig.submit(&d);
    let first = rig.submit(&d).0.len();
    // SAFETY: `d` is a live draw for the length of the call.
    assert_eq!(unsafe { gc_draw(&d) }, 0);
    let one = rig.submit(&d).0.len() - first;
    let n = 3 * GC_CMD_WORDS / one + 1;
    let k0 = rig.kicks();
    for _ in 0..n {
        // SAFETY: a live draw for the call.
        assert_eq!(unsafe { gc_draw(&d) }, 0);
    }
    assert_eq!(gc_flush(), 0);
    let kicks = with_platform(|p| p.kicks[k0..].to_vec());
    assert!(kicks.len() > 1, "the stream overflowed into more kicks");
    for (i, words) in kicks.iter().enumerate() {
        assert!(whole_commands(words), "kick {i}");
        let jobs = rs_jobs(&Stream(words.clone()));
        let untiles = jobs.iter().filter(|j| j.config & GC_RS_SOURCE_TILED != 0).count();
        assert_eq!(untiles, 1, "kick {i} ends with its copy back: {jobs:?}");
    }
    assert!(rig.faults().is_empty());
    osal::gpu_free(&mut mem);
}

/// (d) Behind a shadow draw, two draws that take none: the shadow is copied
/// back once, before the first of them, and the second finds it written back.
#[cfg(feature = "test-knobs")]
#[test]
fn only_the_first_draw_that_takes_no_shadow_gets_a_copy_back_before_it() {
    let rig = Rig::new();
    let (mut a, da) = shadow_target(&rig);
    let mut db = draw_into(&rig, TARGET_GPU + 0x40_0000);
    db.depth_test = false;
    let _on = TiledRt::on();
    // SAFETY: live draws for the calls.
    unsafe {
        assert_eq!(gc_draw(&da), 0);
        assert_eq!(gc_draw(&db), 0);
        assert_eq!(gc_draw(&db), 0);
    }
    assert_eq!(gc_flush(), 0);
    let s = rig.last_stream();
    let jobs = rs_jobs(&s);
    assert_eq!(jobs.len(), 2, "tile in, one copy back: {jobs:?}");
    assert_eq!(jobs[1].config & GC_RS_SOURCE_TILED, GC_RS_SOURCE_TILED);
    let at = rs_kicks_at(&s);
    assert!(at[1] > s.upto_draw(0).0.len() && at[1] < s.upto_draw(1).0.len(), "before the first plain draw");
    assert!(rig.faults().is_empty());
    osal::gpu_free(&mut a);
}

/// (e) With the knob off none of these streams carries an RS job: a draw
/// elsewhere between two draws, a whole colour clear, and a stream filled to
/// overflow.
#[cfg(feature = "test-knobs")]
#[test]
fn with_the_knob_off_no_stream_carries_a_tile_in_or_a_copy_back() {
    let rig = Rig::new();
    let (mut a, da) = shadow_target(&rig);
    let mut db = draw_into(&rig, TARGET_GPU + 0x40_0000);
    db.depth_test = false;
    // SAFETY: live draws for the calls.
    unsafe {
        assert_eq!(gc_draw(&da), 0);
        assert_eq!(gc_draw(&db), 0);
        assert_eq!(gc_draw(&da), 0);
    }
    assert_eq!(gc_flush(), 0);
    assert!(rs_jobs(&rig.last_stream()).is_empty(), "A, B, A");

    gc_attachment_gone(0x65);
    let mut c = da;
    (c.do_clear, c.clear_rgba) = (true, [1.0, 0.0, 0.0, 1.0]);
    assert!(rs_jobs(&rig.submit(&c)).is_empty(), "a whole colour clear");

    let k0 = rig.kicks();
    for _ in 0..3 * GC_CMD_WORDS / 8 + 1 {
        // SAFETY: a live draw for the call.
        assert_eq!(unsafe { gc_draw(&da) }, 0);
    }
    assert_eq!(gc_flush(), 0);
    let kicks = with_platform(|p| p.kicks[k0..].to_vec());
    assert!(kicks.len() > 1, "the stream overflowed into more kicks");
    for (i, words) in kicks.iter().enumerate() {
        assert!(rs_jobs(&Stream(words.clone())).is_empty(), "kick {i}");
    }
    assert!(rig.faults().is_empty());
    osal::gpu_free(&mut a);
}

/// (b) at the draw: a depth surface a shadow draw left super-tiled with live
/// planes refuses a draw that wants it linear with -ENOTSUP. A whole depth
/// clear that is itself a shadow draw (no live plane after it) lets that draw
/// through.
#[cfg(feature = "test-knobs")]
#[test]
fn a_draw_that_needs_the_other_depth_layout_is_refused_until_a_whole_clear() {
    let rig = Rig::new();
    let (mut a, da) = shadow_target(&rig);
    let _on = TiledRt::on();
    // SAFETY: a live draw for the call.
    assert_eq!(unsafe { gc_draw(&da) }, 0);
    assert_eq!(gc_flush(), 0);
    let key = ds_key((0x65, 1), (0x65, 1));
    assert_eq!(ds_cache::layout_of(key), Layout::SuperTiled);

    /* the same draw with the knob off wants a linear depth */
    gc_debug_set_tiled_rt(0);
    // SAFETY: a live draw for the call.
    assert_eq!(unsafe { gc_draw(&da) }, -crate::util::ENOTSUP, "refused");
    assert_eq!(ds_cache::layout_of(key), Layout::SuperTiled, "and unchanged");

    /* a whole depth clear taken as a shadow draw: nothing live behind it */
    gc_debug_set_tiled_rt(1);
    let mut c = da;
    (c.do_clear, c.do_clear_depth, c.clear_depth) = (true, true, 1.0);
    c.depth_write = false;
    // SAFETY: a live draw for the call.
    assert_eq!(unsafe { gc_draw(&c) }, 0, "a whole depth clear");
    gc_debug_set_tiled_rt(0);
    // SAFETY: a live draw for the call.
    assert_eq!(unsafe { gc_draw(&da) }, 0, "lets the draw through");
    assert_eq!(ds_cache::layout_of(key), Layout::Linear);
    assert_eq!(gc_flush(), 0);
    osal::gpu_free(&mut a);
}

fn select_in(layout: Layout, key: ds_cache::DsKey) -> Result<ds_cache::Selected, i32> {
    ds_cache::select_in(layout, key, 64, 64, GC_DEPTH_GUARD_BYTES, 1, None, |_| false, || 0)
}

/// W3. A depth surface has a layout. A surface with a live plane stays in it:
/// another layout is refused, not remapped; one that holds nothing, or a
/// whole clear's value (the same in every layout), changes. A plane in a
/// surface of another layout is not copied to a new surface.
#[test]
fn a_depth_surface_keeps_its_layout_while_it_holds_contents() {
    let rig = Rig::new();
    let key = ds_key((5, 1), (6, 1));
    select_in(Layout::Linear, key).expect("a linear surface");
    assert_eq!(ds_cache::layout_of(key), Layout::Linear);
    assert!(ds_cache::layout_free(key, Layout::SuperTiled), "it holds nothing yet");
    ds_cache::drawn(key, 1, false, [true, true], [None, None]);
    assert!(!ds_cache::layout_free(key, Layout::SuperTiled), "live planes");
    assert_eq!(select_in(Layout::SuperTiled, key).unwrap_err(), -crate::util::ENOTSUP);
    assert_eq!(ds_cache::layout_of(key), Layout::Linear);

    /* a whole clear of both planes: nothing but a value to keep */
    ds_cache::drawn(key, 2, false, [false, false], [Some(1.0f32.to_bits()), Some(0)]);
    assert!(ds_cache::layout_free(key, Layout::SuperTiled));
    select_in(Layout::SuperTiled, key).expect("a clear value is layout-free");
    assert_eq!(ds_cache::layout_of(key), Layout::SuperTiled);
    assert_eq!(ds_cache::planes_of(key).unwrap()[0].cleared_value(), Some(1.0f32.to_bits()), "the value is kept");

    /* the depth plane (object 5) written in the super-tiled surface does not
     * move into a surface of another layout... */
    ds_cache::drawn(key, 3, false, [true, true], [None, None]);
    let other = ds_key((5, 1), (7, 1));
    assert_eq!(select_in(Layout::Linear, other).unwrap_err(), -crate::util::ENOTSUP);
    /* ...but moves into one of the same layout */
    select_in(Layout::SuperTiled, other).expect("the same layout");
    assert!(rig.faults().is_empty());
}

#[test]
fn a_non_linear_depth_surface_is_not_resolved_by_the_cpu() {
    let rig = Rig::new();
    let key = ds_key((9, 1), (9, 1));
    select_in(Layout::SuperTiled, key).expect("a super-tiled surface");
    ds_cache::drawn(key, 1, false, [true, true], [None, None]);
    let mut out = [7.0f32; 64 * 64];
    // SAFETY: 64 rows of 64 floats.
    let rc = unsafe { ds_cache::resolve_depth(9, out.as_mut_ptr(), 64, 64, 64, GC_DEPTH_GUARD_BYTES, |_| 0) };
    assert_eq!(rc, 1, "nothing for a CPU reader");
    assert!(out.iter().all(|&v| v == 7.0), "and nothing written");
    assert!(rig.faults().is_empty());
}

#[test]
fn only_a_depth_renderbuffer_is_private_to_the_pixel_engine() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    (d.depth_id, d.stencil_id) = (0x65, 0x65);
    assert!(ds_cache::pe_private(&d, 1));
    assert!(!ds_cache::pe_private(&d, 2), "the 4x scratch");
    (d.depth_id, d.stencil_id) = (0x4000_0065, 0x4000_0065);
    assert!(!ds_cache::pe_private(&d, 1), "a depth texture");
    (d.depth_id, d.stencil_id) = (0x65, 0x8000_0065);
    assert!(!ds_cache::pe_private(&d, 1), "the window's stencil");
    (d.depth_id, d.stencil_id) = (0, 0);
    assert!(!ds_cache::pe_private(&d, 1), "no depth");
}

/// The TEXKILL scan finds nothing in the programs the backend runs itself:
/// the clear quad's and the mipmap quad's, and a plain program.
#[test]
fn the_discard_scan_finds_no_discard_in_the_internal_programs() {
    let rig = Rig::new();
    assert!(!fs_discards(&rig.fs), "a plain program");
    assert!(!fs_discards(clear_ps()), "the clear quad's");
    assert!(!fs_discards(crate::vivante::mipmap::mipgen_ps()), "mipgen.frag's");
}
