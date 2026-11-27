// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): dtpu knob mode 4 (`DEPTH_TPU_D24X8_FLUSH`,
//! src/vivante/depth_sample.rs), the code for verdict code 2 of gl_test case
//! 222 (docs/notes/gc-depth-sample.md, check 7: the depth view reads stale
//! depth) that docs/notes/gc-depth-sample-wire.md (a).10 names: mode 1's
//! depth view with, in front of the first draw that samples a surface written
//! since the last such flush, the PE depth flush, the FE-PE stall, the
//! texture-cache flushes and the RA-PE stall.
//!
//! glmark2's shadow frames run through the real GL core and the Vivante back
//! end over the stand-in OSAL of `common/gc_scenes.rs`, each submitted
//! stream decoded at its submit into register writes, front-end stalls and
//! draws: the harness of tests/common/gc_stream.rs. A *flush
//! block* is the seven events `emit_view_flush` writes, in order:
//!
//! ```text
//! FLUSH_CACHE DEPTH; SEMAPHORE FE-PE; STALL FE-PE (front end);
//! FLUSH_CACHE TEXTURE; FLUSH_CACHE TEXTUREVS; SEMAPHORE RA-PE; STALL_TOKEN RA-PE
//! ```
//!
//! Pinned:
//!
//! * mode 4, glmark2 shadow, every frame: exactly one flush block, in the
//!   window of the frame's one sampling draw (a draw whose slot 0 descriptor
//!   is D24X8), before that window's `emit_flushes` FLUSH, and after the
//!   depth pass's draw (the write it makes visible);
//! * mode 4, shadow with its ground drawn twice: one block a frame, before
//!   the first sampling draw only (once per surface write, not per draw);
//! * mode 4's settled frame (frame 6) with the block taken out equals mode
//!   1's, event for event;
//! * modes 1 and 0 write no flush block (and mode 0 no sampling draw).
//!
//! The runs are in one `#[test]` in a fixed order, as in
//! tests/gc_depth_view_flushes.rs: the back end keeps state across runs of one
//! process.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test gc_depth_view_flush_mode
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature = "test-knobs"))]

#[path = "common/gc_scenes.rs"]
mod gc_scenes;
#[path = "common/gc_stream.rs"]
mod gc_stream;

use gc_scenes::*;
use gc_stream::{frames_with, Ev};
use yakogl::vivante::depth_sample::{DEPTH_TPU_D24X8, DEPTH_TPU_D24X8_FLUSH, DEPTH_TPU_OFF, GC_TEX_FORMAT_D24X8};
use yakogl::vivante::regs::*;

/// Run `scene` with the depth-view knob in `mode` and return the decoded
/// kicks of every frame (the harness is tests/common/gc_stream.rs).
fn frames(mode: i32, scene: fn() -> Box<dyn FnMut()>) -> Vec<Vec<Vec<Ev>>> {
    frames_with(mode, scene, |body| body()).into_iter().map(|f| f.0).collect()
}

/// glmark2's shadow (`gc_scenes::shadow`) with the ground, the draw that
/// samples the depth texture, drawn twice: two sampling draws behind one
/// depth write.
fn shadow_ground_twice() -> Box<dyn FnMut()> {
    use yakogl::gl::consts::*;
    use yakogl::gl::framebuffer::{glBindFramebuffer, glFramebufferTexture2D};
    use yakogl::gl::state::*;
    let (w, h) = (CANVAS_W * 2, CANVAS_H * 2);
    let depth_tex = depth_texture(w, h);
    let fbo = framebuffer(None, Depth::Texture(depth_tex), w, h);
    let depth_prog = program(MODEL_VS, FLAT_FS, &[]);
    let ground_prog = program(QUAD_VS, GROUND_FS, &[("ShadowMap", 0)]);
    let lit_prog = program(MODEL_VS, FLAT_FS, &[]);
    let horse = model_vbo();
    let ground = quad_vbo();
    Box::new(move || {
        step("depth pass");
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, depth_tex, 0);
            glViewport(0, 0, w as i32, h as i32);
            glColorMask(0, 0, 0, 0);
            glClear(GL_DEPTH_BUFFER_BIT);
        }
        draw(depth_prog, horse, GL_TRIANGLES, 96);
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glViewport(0, 0, CANVAS_W as i32, CANVAS_H as i32);
            glColorMask(1, 1, 1, 1);
        }
        step("ground");
        bind(GL_TEXTURE0, depth_tex);
        draw(ground_prog, ground, GL_TRIANGLE_STRIP, 4);
        step("ground again");
        draw(ground_prog, ground, GL_TRIANGLE_STRIP, 4);
        bind(GL_TEXTURE0, 0);
        step("horse");
        draw(lit_prog, horse, GL_TRIANGLES, 96);
    })
}

/// The windows of one kick: (start of the window, index of its draw, slot-0
/// CONFIG0 written in it).
fn windows(evs: &[Ev]) -> Vec<(usize, usize, Option<u32>)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut cfg = None;
    for (i, e) in evs.iter().enumerate() {
        match e {
            Ev::Slot0(c) => cfg = *c,
            Ev::Draw => {
                out.push((start, i, cfg.take()));
                start = i + 1;
            }
            _ => {}
        }
    }
    out
}

const DESC_FORMAT_MASK: u32 = 0x1f << 13;

fn is_view(cfg: Option<u32>) -> bool {
    cfg.is_some_and(|c| c & DESC_FORMAT_MASK == texdesc_format(GC_TEX_FORMAT_D24X8))
}

/// The seven events of `emit_view_flush`.
fn block() -> [Ev; 7] {
    let fe_pe = stall_token(GC_SYNC_FE, GC_SYNC_PE);
    let ra_pe = stall_token(GC_SYNC_RA, GC_SYNC_PE);
    [
        Ev::State(GC_GL_FLUSH_CACHE, GC_FLUSH_DEPTH),
        Ev::State(GC_GL_SEMAPHORE_TOKEN, fe_pe),
        Ev::FeStall(fe_pe),
        Ev::State(GC_GL_FLUSH_CACHE, GC_FLUSH_TEXTURE),
        Ev::State(GC_GL_FLUSH_CACHE, GC_FLUSH_TEXTUREVS),
        Ev::State(GC_GL_SEMAPHORE_TOKEN, ra_pe),
        Ev::State(GC_GL_STALL_TOKEN, ra_pe),
    ]
}

/// Where the flush blocks of one kick start.
fn blocks(evs: &[Ev]) -> Vec<usize> {
    let b = block();
    (0..evs.len()).filter(|&i| evs[i..].starts_with(&b)).collect()
}

/// A kick with its flush blocks taken out.
fn without_blocks(evs: &[Ev]) -> Vec<Ev> {
    let n = block().len();
    let at = blocks(evs);
    evs.iter().enumerate().filter(|(i, _)| !at.iter().any(|&s| (s..s + n).contains(i))).map(|(_, e)| *e).collect()
}

/// One frame's flush blocks as (kick, block start), and its sampling draws
/// as (kick, window start, draw index).
type Found = (Vec<(usize, usize)>, Vec<(usize, usize, usize)>);

fn found(kicks: &[Vec<Ev>]) -> Found {
    let mut bl = Vec::new();
    let mut sd = Vec::new();
    for (k, evs) in kicks.iter().enumerate() {
        bl.extend(blocks(evs).into_iter().map(|s| (k, s)));
        sd.extend(windows(evs).into_iter().filter(|w| is_view(w.2)).map(|(s, d, _)| (k, s, d)));
    }
    (bl, sd)
}

/// Mode 4's frames of `name`: one flush block a frame, inside the window of
/// the frame's first sampling draw, before the window's `emit_flushes`
/// FLUSH and behind an earlier draw (the depth pass); none in any later
/// sampling draw's window. Returns the sampling draws of each frame.
fn check_mode4(name: &str, all: &[Vec<Vec<Ev>>]) -> Vec<usize> {
    let emit_flush =
        Ev::State(GC_GL_FLUSH_CACHE, GC_FLUSH_COLOR | GC_FLUSH_DEPTH | GC_FLUSH_TEXTURE | GC_FLUSH_SHADER_L1);
    let mut counts = Vec::new();
    for (f, kicks) in all.iter().enumerate() {
        let at = format!("{name} frame {}", f + 1);
        let (bl, sd) = found(kicks);
        assert!(!sd.is_empty(), "{at}: no draw samples the depth view");
        assert_eq!(bl.len(), 1, "{at}: {} flush blocks, want 1 (at {bl:?})", bl.len());
        let (bk, bs) = bl[0];
        let (k, start, draw) = sd[0];
        assert_eq!(bk, k, "{at}: the block is not in the first sampling draw's kick");
        assert!(
            start <= bs && bs < draw,
            "{at}: the block at {bs} is outside the first sampling draw's window {start}..{draw}"
        );
        let evs = &kicks[k];
        let ef = evs[bs..draw].iter().position(|e| *e == emit_flush).map(|p| p + bs);
        assert!(ef.is_some_and(|i| i > bs), "{at}: the block is not in front of the draw's own flushes");
        /* the write it follows: some draw before it in the frame */
        let drawn_before = kicks[..k].iter().flatten().chain(&evs[..bs]).any(|e| *e == Ev::Draw);
        assert!(drawn_before, "{at}: no draw (the depth pass) before the block");
        counts.push(sd.len());
    }
    counts
}

/// One test, in a fixed order (the back end keeps state across runs of one
/// process that `gc_shutdown` does not clear): the mode 1 run first, the
/// mode 4 runs, then the knob off.
#[test]
fn mode4_flushes_once_per_depth_write_before_the_first_sampling_draw() {
    let m1 = frames(DEPTH_TPU_D24X8, shadow);
    for (f, kicks) in m1.iter().enumerate() {
        let (bl, sd) = found(kicks);
        assert!(bl.is_empty(), "mode 1 frame {}: a flush block at {bl:?}", f + 1);
        assert_eq!(sd.len(), 1, "mode 1 frame {}: one sampling draw", f + 1);
    }

    let m4 = frames(DEPTH_TPU_D24X8_FLUSH, shadow);
    assert_eq!(check_mode4("shadow", &m4), vec![1; FRAMES], "shadow: one sampling draw a frame");
    /* the settled frame: the rest of the stream is mode 1's */
    let (a, b) = (&m1[FRAMES - 1], &m4[FRAMES - 1]);
    assert_eq!(a.len(), b.len(), "frame {FRAMES}: kicks in mode 1 and mode 4");
    for (k, (a, b)) in a.iter().zip(b).enumerate() {
        let b = without_blocks(b);
        if let Some(i) = a.iter().zip(&b).position(|(x, y)| x != y) {
            panic!(
                "frame {FRAMES} kick {k}: mode 4 less its block differs from mode 1 at {i}: {:?} vs {:?}",
                a[i], b[i]
            );
        }
        assert_eq!(a.len(), b.len(), "frame {FRAMES} kick {k}: lengths");
    }

    let twice = frames(DEPTH_TPU_D24X8_FLUSH, shadow_ground_twice);
    assert_eq!(check_mode4("shadow, ground twice", &twice), vec![2; FRAMES], "two sampling draws a frame");

    for (f, kicks) in frames(DEPTH_TPU_OFF, shadow).iter().enumerate() {
        let (bl, sd) = found(kicks);
        assert!(bl.is_empty() && sd.is_empty(), "mode 0 frame {}: blocks {bl:?}, sampling draws {sd:?}", f + 1);
    }
}
