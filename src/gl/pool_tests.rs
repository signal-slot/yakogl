// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GL core gives back the GPU memory of a deleted object
//! (docs/notes/pvr-render-pool.md, "Who frees").
//!
//! glmark2 creates and deletes each scene's buffers, textures and
//! framebuffers between scenes. On the AM62P the render pool ran dry in the
//! terrain scene (build/glmark2-am62p-20261003-230937). These tests pin the
//! first link of the free path: every object glmark2 deletes hands its
//! storage back to the OSAL (`gpu_free`), in glmark2's own order (texture,
//! then renderbuffer, then framebuffer: scene-terrain/base-renderer.cpp:33-36).
//! What the OSAL and the driver's pool do with it next is C and is not
//! covered here.

use core::ffi::c_void;
use core::ptr;
use std::vec::Vec;

use super::buffer::*;
use super::consts::*;
use super::framebuffer::*;
use super::state::*;
use super::texture::*;
use super::types::*;
use crate::testutil::TestGl;

/// The terrain mesh's position buffer: the first request the pool refused
/// on the board ("render pool exhausted: 4608 KiB requested").
const TERRAIN_VBO: usize = 4608 * 1024;
/// terrain's 800x600 render targets (base-renderer.cpp setup_offscreen).
const RT_W: GLsizei = 800;
const RT_H: GLsizei = 600;
/// Rounds per test: the 4.6 MiB buffer alone, if kept, would pass the
/// AM62P pool's 57.8 MiB after 13 rounds.
const ROUNDS: usize = 16;

/// Every submitted kick completes, so nothing freed stays a zombie.
fn settle(gl: &TestGl) {
    // SAFETY: a current context (the TestGl guard).
    unsafe { glFinish() };
    gl.complete_kicks(gl.submitted());
}

/// Run `round` ROUNDS times; the live bytes after each round.
fn live_after_each(gl: &TestGl, mut round: impl FnMut()) -> Vec<usize> {
    let mut live = Vec::new();
    for _ in 0..ROUNDS {
        round();
        settle(gl);
        assert_eq!(gl.error(), GL_NO_ERROR);
        live.push(gl.live_bytes());
    }
    live
}

fn assert_flat(what: &str, before: usize, live: &[usize]) {
    assert!(
        live.iter().all(|&b| b == before),
        "{what}: the live GPU bytes grow with the rounds (before {before}, after each round {live:?}): \
         a deleted object keeps its storage"
    );
}

/// A 800x600 RGBA texture with no data, bound to unit 0.
///
/// # Safety
/// A current context.
unsafe fn rt_texture() -> GLuint {
    let mut t: GLuint = 0;
    // SAFETY: `t` outlives the calls; a null pixel pointer is allowed.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, RT_W, RT_H, 0, GL_RGBA, GL_UNSIGNED_BYTE, ptr::null());
    }
    t
}

/// terrain's off-screen target with depth: a texture colour attachment and
/// a depth renderbuffer, cleared; the framebuffer stays bound.
///
/// # Safety
/// A current context.
unsafe fn rt_with_depth() -> (GLuint, GLuint, GLuint) {
    let (mut rb, mut fb): (GLuint, GLuint) = (0, 0);
    // SAFETY: the names outlive the calls.
    unsafe {
        let t = rt_texture();
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, RT_W, RT_H);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        (t, rb, fb)
    }
}

#[test]
fn a_deleted_buffer_gives_its_storage_back() {
    let gl = TestGl::new(64, 64);
    settle(&gl);
    let before = gl.live_bytes();
    let data = std::vec![0x5Au8; TERRAIN_VBO];
    let live = live_after_each(&gl, || {
        let mut b: GLuint = 0;
        // SAFETY: `b` and `data` outlive the calls.
        unsafe {
            glGenBuffers(1, &mut b);
            glBindBuffer(GL_ARRAY_BUFFER, b);
            glBufferData(GL_ARRAY_BUFFER, TERRAIN_VBO as GLsizeiptr, data.as_ptr() as *const c_void, GL_STATIC_DRAW);
            glDeleteBuffers(1, &b);
        }
    });
    assert_flat("buffer", before, &live);
}

#[test]
fn a_deleted_texture_gives_its_storage_back() {
    let gl = TestGl::new(64, 64);
    settle(&gl);
    let before = gl.live_bytes();
    let live = live_after_each(&gl, || {
        // SAFETY: the TestGl context is current.
        unsafe {
            let t = rt_texture();
            glDeleteTextures(1, &t);
        }
    });
    assert_flat("texture", before, &live);
}

/// Deleted after unbinding, in glmark2's order: texture, renderbuffer,
/// framebuffer (the texture and the renderbuffer are still attached to a
/// framebuffer that is not bound when they are deleted).
#[test]
fn a_deleted_framebuffer_with_depth_gives_its_storage_back() {
    let gl = TestGl::new(64, 64);
    settle(&gl);
    let before = gl.live_bytes();
    let live = live_after_each(&gl, || {
        // SAFETY: the TestGl context is current.
        unsafe {
            let (t, rb, fb) = rt_with_depth();
            glFinish();
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteTextures(1, &t);
            glDeleteRenderbuffers(1, &rb);
            glDeleteFramebuffers(1, &fb);
        }
    });
    assert_flat("framebuffer with depth", before, &live);
}

/// The same deleted while the framebuffer is bound: the attachments are
/// detached from it at delete (ES 2.0.25 4.4.3), the clear not yet kicked.
#[test]
fn a_bound_framebuffers_deleted_attachments_give_their_storage_back() {
    let gl = TestGl::new(64, 64);
    settle(&gl);
    let before = gl.live_bytes();
    let live = live_after_each(&gl, || {
        // SAFETY: the TestGl context is current.
        unsafe {
            let (t, rb, fb) = rt_with_depth();
            glDeleteTextures(1, &t);
            glDeleteRenderbuffers(1, &rb);
            glDeleteFramebuffers(1, &fb);
        }
    });
    assert_flat("bound framebuffer", before, &live);
}

/// The control: the instrument above can see a leak. With the OSAL refusing
/// every free (a full pending-free queue keeps the memory for good,
/// src/osal/pending_frees.h), the same buffer rounds grow by the buffer each.
#[test]
fn the_control_a_refused_free_shows_as_growth() {
    let gl = TestGl::new(64, 64);
    settle(&gl);
    let before = gl.live_bytes();
    gl.set_free_result(-crate::util::ENOMEM);
    let data = std::vec![0x5Au8; TERRAIN_VBO];
    let live = live_after_each(&gl, || {
        let mut b: GLuint = 0;
        // SAFETY: `b` and `data` outlive the calls.
        unsafe {
            glGenBuffers(1, &mut b);
            glBindBuffer(GL_ARRAY_BUFFER, b);
            glBufferData(GL_ARRAY_BUFFER, TERRAIN_VBO as GLsizeiptr, data.as_ptr() as *const c_void, GL_STATIC_DRAW);
            glDeleteBuffers(1, &b);
        }
    });
    gl.set_free_result(0);
    let want: Vec<usize> = (1..=ROUNDS).map(|n| before + n * TERRAIN_VBO).collect();
    assert_eq!(live, want, "a refused free keeps the buffer: one buffer more per round");
}

/// Two glmark2-like scenes of `SCENE_BUFFERS` terrain VBOs in a pool that
/// holds one scene and a half (src/testutil.rs `set_pool_budget`), drawn by
/// one program from a small vertex buffer of their own.
const SCENE_BUFFERS: usize = 4;

struct Scenes<'a> {
    gl: &'a TestGl,
    data: Vec<u8>,
    tiny: GLuint,
}

impl<'a> Scenes<'a> {
    /// `lag`: the mock GPU's (Some(0): a kick completes when submitted).
    fn new(gl: &'a TestGl, lag: Option<u32>) -> Scenes<'a> {
        use crate::gl::{program::*, vertex::*};
        gl.set_kick_lag(lag);
        let prog = gl.program("passthrough.vert", "uniform_color.frag");
        settle(gl);
        /* one scene and a half (and the frames' small allocations) */
        gl.set_pool_budget(Some(gl.held_bytes() + SCENE_BUFFERS * TERRAIN_VBO * 3 / 2));
        let verts = [0u8; 48];
        let mut tiny: GLuint = 0;
        // SAFETY: plain GL calls; the attributes read the bound buffer.
        unsafe {
            glGenBuffers(1, &mut tiny);
            glBindBuffer(GL_ARRAY_BUFFER, tiny);
            glBufferData(GL_ARRAY_BUFFER, 48, verts.as_ptr() as *const c_void, GL_STATIC_DRAW);
            glUseProgram(prog);
            for i in 0..2 {
                glVertexAttribPointer(i, 4, GL_FLOAT, 0, 0, core::ptr::null());
                glEnableVertexAttribArray(i);
            }
        }
        Scenes { gl, data: std::vec![0x5Au8; TERRAIN_VBO], tiny }
    }

    /// A scene's set-up: its buffers (the GL error says whether they fit).
    fn setup(&self) -> Vec<GLuint> {
        let mut b = [0 as GLuint; SCENE_BUFFERS];
        // SAFETY: `b` and `data` outlive the calls.
        unsafe {
            glGenBuffers(SCENE_BUFFERS as GLsizei, b.as_mut_ptr());
            for &h in &b {
                glBindBuffer(GL_ARRAY_BUFFER, h);
                glBufferData(
                    GL_ARRAY_BUFFER,
                    TERRAIN_VBO as GLsizeiptr,
                    self.data.as_ptr() as *const c_void,
                    GL_STATIC_DRAW,
                );
            }
            glBindBuffer(GL_ARRAY_BUFFER, self.tiny);
        }
        b.to_vec()
    }

    /// One frame recorded; `swap` submits it.
    fn frame(&self, swap: bool) {
        // SAFETY: plain GL calls.
        unsafe {
            crate::gl::draw::glDrawArrays(GL_TRIANGLES, 0, 3);
            if swap {
                glFlush();
            }
        }
    }

    fn teardown(&self, names: &[GLuint]) {
        // SAFETY: the names are the scene's.
        unsafe { glDeleteBuffers(names.len() as GLsizei, names.as_ptr()) };
    }
}

impl Drop for Scenes<'_> {
    fn drop(&mut self) {
        // SAFETY: the frames' buffer.
        unsafe { glDeleteBuffers(1, &self.tiny) };
        self.gl.set_pool_budget(None);
        self.gl.set_osal_reclaim(false);
    }
}

/// Mechanism 2 of docs/notes/pvr-render-pool.md, at the GL
/// level, with the Zephyr OSAL's hold rule the mock models (src/testutil.rs
/// `held`): glmark2 draws a scene, swaps (here: glFlush submits the frame, and
/// the GPU finishes it at once), deletes the scene's buffers, and sets up the
/// next scene before any kick. Every free is held for the kick after the
/// last submitted one, which is not submitted yet, so the next scene's
/// buffers do not fit a pool that holds one scene and a half.
///
/// This pins the behaviour with CONFIG_YAKOGL_PVR_POOL_RECLAIM=n (the
/// default until the AM62P glmark2 cell turned it on), which is wrong:
/// GL_OUT_OF_MEMORY with the whole previous scene deleted and its kick
/// complete. The reclaim's tests are below.
#[test]
fn known_wrong_a_scenes_frees_are_held_through_the_next_scenes_setup() {
    let gl = TestGl::new(16, 16);
    let sc = Scenes::new(&gl, Some(0));

    /* scene A: set up, one frame, the swap */
    let a = sc.setup();
    assert_eq!(gl.error(), GL_NO_ERROR, "scene A fits");
    sc.frame(true);
    assert_eq!(gl.error(), GL_NO_ERROR);
    let done = gl.submitted();
    assert!(done > 0 && !gl.kick_pending(done), "the frame was submitted and has completed");
    /* scene A's teardown */
    let (live, held) = (gl.live_bytes(), gl.held_bytes());
    sc.teardown(&a);
    assert_eq!(gl.live_bytes(), live - SCENE_BUFFERS * TERRAIN_VBO, "the GL core freed scene A");
    assert_eq!(gl.held_bytes(), held, "known wrong: every byte of scene A is still held after its kick completed");

    /* scene B's set-up, before any kick */
    let b = sc.setup();
    assert_eq!(
        gl.error(),
        GL_OUT_OF_MEMORY,
        "known wrong: scene B does not fit beside scene A's deleted buffers, held for a kick nobody submitted \
         (CONFIG_YAKOGL_PVR_POOL_RECLAIM off)"
    );

    /* The control: the budget holds one scene. Once the kick A's frees were
     * tagged with completes, they come back and scene B fits. */
    sc.teardown(&b);
    sc.frame(true);
    assert!(gl.submitted() > done && !gl.kick_pending(gl.submitted()));
    let _ = gl.error();
    let b = sc.setup();
    assert_eq!(gl.error(), GL_NO_ERROR, "the control: scene B fits once scene A's kick has passed");
    sc.teardown(&b);
}

/// With CONFIG_YAKOGL_PVR_POOL_RECLAIM, the same two scenes: a free made with
/// no frame open is held only for the last submitted kick, which has
/// completed, so scene A comes back at its teardown and scene B fits with no
/// kick in between.
#[test]
fn with_the_reclaim_a_scene_deleted_between_frames_comes_back_at_once() {
    let gl = TestGl::new(16, 16);
    let sc = Scenes::new(&gl, Some(0));
    gl.set_osal_reclaim(true);

    let a = sc.setup();
    sc.frame(true);
    assert_eq!(gl.error(), GL_NO_ERROR);
    let held = gl.held_bytes();
    sc.teardown(&a);
    assert_eq!(gl.held_bytes(), held - SCENE_BUFFERS * TERRAIN_VBO, "scene A is back at its teardown");
    let b = sc.setup();
    assert_eq!(gl.error(), GL_NO_ERROR, "scene B fits");
    sc.teardown(&b);
}

/// The reclaim's wait: scene A's kick is still running at its teardown (the
/// mock GPU completes nothing by itself). Scene B's allocation is refused,
/// waits for the submitted kick, and is served.
#[test]
fn with_the_reclaim_a_refused_allocation_waits_for_the_submitted_kicks() {
    let gl = TestGl::new(16, 16);
    let sc = Scenes::new(&gl, None);
    gl.set_osal_reclaim(true);

    let a = sc.setup();
    sc.frame(true);
    let done = gl.submitted();
    assert!(gl.kick_pending(done), "the frame is still running");
    sc.teardown(&a);
    let b = sc.setup();
    assert_eq!(gl.error(), GL_NO_ERROR, "scene B fits after the wait");
    assert!(!gl.kick_pending(done), "the allocation waited for the submitted kick");
    sc.teardown(&b);
}

/// The other side of the reclaim's condition: buffers deleted while a frame
/// is being recorded are held for that frame's kick, which no wait can
/// complete before it is submitted. Scene B is refused, as without the
/// reclaim.
#[test]
fn with_the_reclaim_a_scene_deleted_inside_a_frame_stays_held_until_its_kick() {
    let gl = TestGl::new(16, 16);
    let sc = Scenes::new(&gl, Some(0));
    gl.set_osal_reclaim(true);

    let a = sc.setup();
    sc.frame(false); /* recorded, not submitted */
    let held = gl.held_bytes();
    sc.teardown(&a);
    assert_eq!(gl.held_bytes(), held, "held for the frame being recorded");
    let b = sc.setup();
    assert_eq!(gl.error(), GL_OUT_OF_MEMORY, "no wait completes a kick that is not submitted");
    sc.teardown(&b);
    sc.frame(true);
    let _ = gl.error();
}

/// glmark2's shadow scene on the AM62P (measurement round 6242497b,
/// 2026-10-04): its set-up's 1600x1200 GL_DEPTH_COMPONENT texture (7680000
/// bytes, "texture 1600x1200 x1 fmt 7 ... out of GPU memory") was refused
/// while 23932 KiB of the previous scene's frees were held for a kick not
/// yet submitted ("pending frees now 38 (23932 KiB), oldest fence 103994,
/// last submitted 103993; reclaim off"). With no storage the depth
/// attachment is incomplete, and glCheckFramebufferStatus answers
/// GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT (0x8cd6), the status glmark2
/// printed. Mechanism 2 again (the test above), reached through a texture.
const SHADOW_DEPTH: usize = 1600 * 1200 * 4;

/// A previous scene's buffer of shadow's depth texture's size, drawn once,
/// swapped (its kick completes at once), and deleted; then shadow's
/// set-up: the depth texture and a framebuffer with it alone. Returns the
/// framebuffer's status and the GL error of the texture's allocation.
fn shadow_setup_after_a_scene(gl: &TestGl) -> (GLenum, GLenum) {
    use crate::gl::{program::*, vertex::*};
    gl.set_kick_lag(Some(0));
    let prog = gl.program("passthrough.vert", "uniform_color.frag");
    settle(gl);
    /* the previous scene and shadow's texture do not fit together */
    gl.set_pool_budget(Some(gl.held_bytes() + SHADOW_DEPTH * 3 / 2));
    let data = std::vec![0u8; SHADOW_DEPTH];
    let (mut scene, mut tex, mut fbo): (GLuint, GLuint, GLuint) = (0, 0, 0);
    // SAFETY: plain GL calls; `data` outlives them; null pixels only allocate.
    let (alloc_err, status) = unsafe {
        glGenBuffers(1, &mut scene);
        glBindBuffer(GL_ARRAY_BUFFER, scene);
        glBufferData(GL_ARRAY_BUFFER, SHADOW_DEPTH as GLsizeiptr, data.as_ptr() as *const c_void, GL_STATIC_DRAW);
        glUseProgram(prog);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, ptr::null());
        glEnableVertexAttribArray(0);
        crate::gl::draw::glDrawArrays(GL_TRIANGLES, 0, 3);
        glFlush();
        glDisableVertexAttribArray(0);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glDeleteBuffers(1, &scene);
        assert_eq!(gl.error(), GL_NO_ERROR, "the previous scene drew and was deleted");
        assert!(!gl.kick_pending(gl.submitted()), "and its kick has completed");

        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_DEPTH_COMPONENT as GLint,
            1600,
            1200,
            0,
            GL_DEPTH_COMPONENT,
            GL_UNSIGNED_INT,
            ptr::null(),
        );
        let alloc_err = gl.error();
        glBindTexture(GL_TEXTURE_2D, 0);
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, tex, 0);
        let status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tex);
        (alloc_err, status)
    };
    gl.set_pool_budget(None);
    gl.set_osal_reclaim(false);
    (status, alloc_err)
}

/// Pins the board's 0x8cd6 with CONFIG_YAKOGL_PVR_POOL_RECLAIM=n, which is
/// wrong: the previous scene is deleted and its kick has completed, yet
/// shadow's depth texture is refused, and the framebuffer is
/// GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT (the measurement round 6242497b,
/// built with the reclaim off; it is on by default since).
#[test]
fn known_wrong_shadows_depth_texture_is_refused_beside_held_frees_and_its_framebuffer_is_incomplete() {
    if !crate::backend::DEPTH_ONLY_TARGETS {
        return; /* the framebuffer is UNSUPPORTED before its storage is looked at */
    }
    let gl = TestGl::new(16, 16);
    let (status, alloc_err) = shadow_setup_after_a_scene(&gl);
    assert_eq!(
        alloc_err, GL_OUT_OF_MEMORY,
        "known wrong: the depth texture does not fit beside the previous scene's held frees (reclaim off)"
    );
    assert_eq!(status, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "known wrong: glmark2's 0x8cd6 (reclaim off)");
}

/// The same set-up with CONFIG_YAKOGL_PVR_POOL_RECLAIM: the previous scene
/// comes back at its deletion, the depth texture is served, and the
/// framebuffer is complete.
#[test]
fn with_the_reclaim_shadows_depth_texture_fits_and_its_framebuffer_is_complete() {
    if !crate::backend::DEPTH_ONLY_TARGETS {
        return;
    }
    let gl = TestGl::new(16, 16);
    gl.set_osal_reclaim(true);
    let (status, alloc_err) = shadow_setup_after_a_scene(&gl);
    assert_eq!(alloc_err, GL_NO_ERROR, "the depth texture is served");
    assert_eq!(status, GL_FRAMEBUFFER_COMPLETE);
}

/// The owner report's GL half claims every byte the GL core holds: with
/// buffers, a texture, a renderbuffer, a program drawn once (its code and
/// uniforms) and the draw path's staging live, the "pool owner gl ..." lines
/// sum to the mock pool's live bytes (page rounded; the mock backend
/// allocates nothing of its own). On the board the same balance, with the
/// driver and the backend added, is the "unaccounted" figure.
#[test]
fn the_gl_owner_lines_claim_every_live_byte() {
    use crate::gl::{draw::*, program::*, vertex::*};
    let gl = TestGl::new(16, 16);
    let prog = gl.program("passthrough.vert", "uniform_color.frag");
    let verts = [0u8; 48];
    let (mut b, mut rb): (GLuint, GLuint) = (0, 0);
    // SAFETY: plain GL calls; the names and `verts` outlive them.
    let t = unsafe {
        glGenBuffers(1, &mut b);
        glBindBuffer(GL_ARRAY_BUFFER, b);
        glBufferData(GL_ARRAY_BUFFER, 48, verts.as_ptr() as *const c_void, GL_STATIC_DRAW);
        let t = rt_texture();
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        /* a colour one: a depth renderbuffer's storage is the backend's */
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA4, 64, 64);
        glUseProgram(prog);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null());
        glEnableVertexAttribArray(0);
        /* the second input from client memory: the draw path stages it */
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, verts.as_ptr() as *const c_void);
        glEnableVertexAttribArray(1);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        t
    };
    settle(&gl);
    assert_eq!(gl.error(), GL_NO_ERROR);
    let _ = gl.logs();
    let kib = crate::gl::context::yakogl_pool_owners();
    let logs = gl.logs();
    let lines: Vec<&String> = logs.iter().filter(|l| l.starts_with("pool owner gl ")).collect();
    assert_eq!(lines.len(), 6, "one line per GL owner: {lines:?}");
    for (name, nonzero) in [
        ("gl buffers", true),
        ("gl textures", true),
        ("gl renderbuffers", true),
        ("gl default surfaces", true),
        ("gl programs", true),
        ("gl draw staging", true),
    ] {
        let l = lines.iter().find(|l| l.starts_with(&std::format!("pool owner {name}:"))).expect(name);
        assert_eq!(!l.contains(": 0 blocks"), nonzero, "{l}");
    }
    assert_eq!(
        kib as usize * 1024,
        gl.held_bytes(),
        "the GL owner lines leave live bytes unclaimed (or claim some twice): {lines:?}"
    );
    // SAFETY: the names are this test's.
    unsafe {
        glDeleteBuffers(1, &b);
        glDeleteTextures(1, &t);
        glDeleteRenderbuffers(1, &rb);
    }
}
