// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Object names past the fixed pools ("far names").
//!
//! A buffer, texture, renderbuffer or framebuffer name is an index into its
//! table (`GlObjects::buffers`, ..., `GlContext::framebuffers`), and the
//! whole GL core stores and compares those indices. OpenGL ES 2.0 lets an
//! application bind a name it never generated (binding an unused name
//! creates the object). dEQP's `lifetime.bind_no_gen.*` binds a random
//! 32-bit name, expects GL_NO_ERROR, and expects glIs* to say true
//! (glsLifetimeTests.cpp, testBindNoGen). A name that large cannot be an
//! index.
//!
//! So only the API boundary knows about far names:
//!  - a name coming in (bind, delete, is, attach, the C lookups) becomes a
//!    table slot through [`slot_of`] (or [`bind_slot`], which may take one);
//!  - a slot going out (the binding queries, OBJECT_NAME) becomes a name
//!    through [`name_of`].
//!
//! Everything between the two keeps working on slots, as before.
//!
//! A far name borrows a free slot, searched from the top of the pool down,
//! out of glGen*'s way (glGen* hands out the lowest free slot). At most
//! [`FAR_NAMES`] far names of one type live at once; the next one is refused
//! with GL_OUT_OF_MEMORY, as a full pool is today. A small name whose slot a
//! far name holds cannot get its own object while the far name lives: it is
//! refused with GL_OUT_OF_MEMORY and logged, never aliased to the far
//! name's object.
//!
//! The tables are Rust-only statics beside the pools (per share group for
//! the shared types, per context slot for framebuffers, which are not
//! shared), so the `#[repr(C)]` structs that the C suite reads do not change.

use super::context::{GlContext, MAX_BUFFERS, MAX_CONTEXTS, MAX_FRAMEBUFFERS, MAX_RENDERBUFFERS, MAX_TEXTURES};
use super::types::GLuint;
use crate::util::Global;

/// Far names of one type that can live at once, per group (per context for
/// framebuffers). dEQP binds one at a time; 16 leaves room for an
/// application that picks its own names.
pub const FAR_NAMES: usize = 16;

/// The object types whose names an application may bind without glGen*.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Buffer,
    Texture,
    Renderbuffer,
    Framebuffer,
}

impl Kind {
    /// The first name that is not a slot of this type's pool.
    pub const fn pool(self) -> usize {
        match self {
            Kind::Buffer => MAX_BUFFERS,
            Kind::Texture => MAX_TEXTURES,
            Kind::Renderbuffer => MAX_RENDERBUFFERS,
            Kind::Framebuffer => MAX_FRAMEBUFFERS,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Kind::Buffer => "buffer",
            Kind::Texture => "texture",
            Kind::Renderbuffer => "renderbuffer",
            Kind::Framebuffer => "framebuffer",
        }
    }
}

/// One far name and the slot it borrows; `name == 0` is an empty entry
/// (0 is never an object name, so it cannot be a far name either).
#[derive(Clone, Copy)]
struct Far {
    name: GLuint,
    slot: u16,
}

const EMPTY: Far = Far { name: 0, slot: 0 };

type Table = [Far; FAR_NAMES];

/// Buffers, textures, renderbuffers of each share group (index = group).
static GROUP_FAR: Global<[[Table; 3]; MAX_CONTEXTS]> = Global::zeroed();
/// Framebuffers of each context slot (index = `GlContext::index`).
static CONTEXT_FAR: Global<[Table; MAX_CONTEXTS]> = Global::zeroed();

fn table(c: &GlContext, k: Kind) -> &'static mut Table {
    // SAFETY: single-threaded by contract (util::Global); one context is
    // current, and no caller keeps a reference into these tables across a
    // call to this module.
    unsafe {
        match k {
            Kind::Buffer => &mut GROUP_FAR.get()[c.group as usize][0],
            Kind::Texture => &mut GROUP_FAR.get()[c.group as usize][1],
            Kind::Renderbuffer => &mut GROUP_FAR.get()[c.group as usize][2],
            Kind::Framebuffer => &mut CONTEXT_FAR.get()[c.index as usize],
        }
    }
}

/// The far name that holds `slot`, if one does.
fn holder(t: &Table, slot: usize) -> Option<GLuint> {
    t.iter().find(|e| e.name != 0 && e.slot as usize == slot).map(|e| e.name)
}

/// The slot of an existing (or possible) name, without taking one: for
/// delete, glIs* and every call that only refers to an object. `None` for
/// name 0 and for a name no slot stands for (a far name nothing bound, or a
/// small name whose slot a far name holds); such a name is no object. A
/// `Some` slot may still be free: the caller's table says whether it lives.
pub fn slot_of(c: &GlContext, k: Kind, name: GLuint) -> Option<GLuint> {
    if name == 0 {
        return None;
    }
    let t = table(c, k);
    if (name as usize) < k.pool() {
        return holder(t, name as usize).is_none().then_some(name);
    }
    t.iter().find(|e| e.name == name).map(|e| e.slot as GLuint)
}

/// The slot a bind of `name` uses, taking one for a far name that has none
/// yet (0 stays 0). `free(slot)` says whether a slot is unused in the
/// caller's table. `None` is GL_OUT_OF_MEMORY for the caller to raise: the
/// far names of this type are all taken, no slot is free, or `name` is a
/// small name whose slot a far name holds. The slot taken is reserved by
/// the caller's bind making the object there (its `used` flag), in the
/// same call.
pub fn bind_slot(c: &GlContext, k: Kind, name: GLuint, free: impl Fn(usize) -> bool) -> Option<GLuint> {
    if name == 0 {
        return Some(0);
    }
    let t = table(c, k);
    if (name as usize) < k.pool() {
        if let Some(far) = holder(t, name as usize) {
            glog!("{} {}: its slot is held by the far name {}; refused (GL_OUT_OF_MEMORY)", k.label(), name, far);
            return None;
        }
        return Some(name);
    }
    if let Some(e) = t.iter().find(|e| e.name == name) {
        return Some(e.slot as GLuint);
    }
    let Some(entry) = t.iter().position(|e| e.name == 0) else {
        glog!("{} {}: {} far names already live; refused (GL_OUT_OF_MEMORY)", k.label(), name, FAR_NAMES);
        return None;
    };
    let Some(s) = (1..k.pool()).rev().find(|&s| free(s) && holder(t, s).is_none()) else {
        glog!("{} {}: no free slot for a far name; refused (GL_OUT_OF_MEMORY)", k.label(), name);
        return None;
    };
    t[entry] = Far { name, slot: s as u16 };
    Some(s as GLuint)
}

/// The name the application knows a slot by, for the queries that return
/// one: the far name that holds it, else the slot itself (0 stays 0).
pub fn name_of(c: &GlContext, k: Kind, slot: GLuint) -> GLuint {
    if slot == 0 {
        return 0;
    }
    holder(table(c, k), slot as usize).unwrap_or(slot)
}

/// A delete took `slot`'s name away: the far name that held it is gone.
/// The slot may live on as an orphan a framebuffer holds; it is then known
/// by its slot number, as an in-pool orphan is by its old name.
pub fn forget(c: &GlContext, k: Kind, slot: GLuint) {
    for e in table(c, k).iter_mut() {
        if e.name != 0 && e.slot as u32 == slot {
            *e = EMPTY;
        }
    }
}

/// A share group starts (gl_group_alloc zeroes its objects): no far names.
pub fn reset_group(group: usize) {
    // SAFETY: single-threaded by contract; the group has no holder yet.
    unsafe { GROUP_FAR.get()[group] = [[EMPTY; FAR_NAMES]; 3] };
}

/// A context slot starts (alloc_context): no far framebuffer names.
pub fn reset_context(index: usize) {
    // SAFETY: single-threaded by contract; the slot has no current user.
    unsafe { CONTEXT_FAR.get()[index] = [EMPTY; FAR_NAMES] };
}

#[cfg(test)]
// the far-object query test is ES 3.0 only: a report-es2 build leaves its imports idle
#[cfg_attr(feature = "report-es2", allow(unused_imports, dead_code))]
mod tests {
    use super::*;
    use crate::gl::buffer::{glBindBuffer, glBufferData, glDeleteBuffers, glIsBuffer};
    use crate::gl::consts::*;
    use crate::gl::es3::glBindBufferBase;
    use crate::gl::framebuffer::{
        glBindFramebuffer, glBindRenderbuffer, glCheckFramebufferStatus, glDeleteFramebuffers, glDeleteRenderbuffers,
        glFramebufferRenderbuffer, glFramebufferTexture2D, glGetFramebufferAttachmentParameteriv, glIsFramebuffer,
        glIsRenderbuffer, glRenderbufferStorage,
    };
    use crate::gl::state::{glGetIntegeri_v, glGetIntegerv};
    use crate::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glIsTexture, glTexImage2D};
    use crate::gl::types::*;
    use crate::gl::vertex::{glGetVertexAttribiv, glVertexAttribPointer};
    use crate::testutil::TestGl;

    /* names dEQP's testBindNoGen could draw (a random uint32 that is no
     * object yet): far past every pool, and the largest one */
    const FAR: GLuint = 0xdead_beef;
    const TOP: GLuint = u32::MAX;

    fn geti(pname: GLenum) -> GLint {
        let mut v = [-1; 4];
        // SAFETY: room for the values any binding query writes.
        unsafe { glGetIntegerv(pname, v.as_mut_ptr()) };
        v[0]
    }

    fn attachment_name(point: GLenum) -> GLint {
        let mut v = -1;
        // SAFETY: one writable GLint.
        unsafe {
            glGetFramebufferAttachmentParameteriv(GL_FRAMEBUFFER, point, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME, &mut v)
        };
        v
    }

    /// dEQP-GLES2 lifetime.bind_no_gen.*: bind a name nobody generated,
    /// no error, glIs* true, the binding query answers the name, and the
    /// delete takes it away again. The rule before this module refused any
    /// name past the pool with GL_INVALID_VALUE.
    #[test]
    fn a_name_past_the_pool_binds_is_an_object_and_is_deleted() {
        let gl = TestGl::new(8, 8);
        for name in [FAR, TOP] {
            // SAFETY: plain GL calls; the delete arrays hold one name.
            unsafe {
                assert_eq!(glIsBuffer(name), GL_FALSE_B, "buffer {name:#x} exists before the bind");
                glBindBuffer(GL_ARRAY_BUFFER, name);
                assert_eq!(gl.error(), GL_NO_ERROR, "bind buffer {name:#x}");
                assert_eq!(glIsBuffer(name), GL_TRUE_B, "buffer {name:#x}");
                assert_eq!(geti(GL_ARRAY_BUFFER_BINDING) as GLuint, name);
                glDeleteBuffers(1, &name);
                assert_eq!((glIsBuffer(name), geti(GL_ARRAY_BUFFER_BINDING)), (GL_FALSE_B, 0));

                assert_eq!(glIsTexture(name), GL_FALSE_B);
                glBindTexture(GL_TEXTURE_2D, name);
                assert_eq!(gl.error(), GL_NO_ERROR, "bind texture {name:#x}");
                assert_eq!(glIsTexture(name), GL_TRUE_B, "texture {name:#x}");
                assert_eq!(geti(GL_TEXTURE_BINDING_2D) as GLuint, name);
                glDeleteTextures(1, &name);
                assert_eq!((glIsTexture(name), geti(GL_TEXTURE_BINDING_2D)), (GL_FALSE_B, 0));

                assert_eq!(glIsRenderbuffer(name), GL_FALSE_B);
                glBindRenderbuffer(GL_RENDERBUFFER, name);
                assert_eq!(gl.error(), GL_NO_ERROR, "bind renderbuffer {name:#x}");
                assert_eq!(glIsRenderbuffer(name), GL_TRUE_B, "renderbuffer {name:#x}");
                assert_eq!(geti(GL_RENDERBUFFER_BINDING) as GLuint, name);
                glDeleteRenderbuffers(1, &name);
                assert_eq!((glIsRenderbuffer(name), geti(GL_RENDERBUFFER_BINDING)), (GL_FALSE_B, 0));

                assert_eq!(glIsFramebuffer(name), GL_FALSE_B);
                glBindFramebuffer(GL_FRAMEBUFFER, name);
                assert_eq!(gl.error(), GL_NO_ERROR, "bind framebuffer {name:#x}");
                assert_eq!(glIsFramebuffer(name), GL_TRUE_B, "framebuffer {name:#x}");
                assert_eq!(geti(GL_FRAMEBUFFER_BINDING) as GLuint, name);
                glDeleteFramebuffers(1, &name);
                assert_eq!((glIsFramebuffer(name), geti(GL_FRAMEBUFFER_BINDING)), (GL_FALSE_B, 0));
            }
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /// A far name is a whole object, not only a yes from glIs*: storage,
    /// attachment, the attachment's OBJECT_NAME, a vertex array and an
    /// indexed uniform-buffer point all answer the application's name.
    #[test]
    #[cfg(not(feature = "report-es2"))]
    fn far_objects_are_usable_and_every_query_answers_their_name() {
        let gl = TestGl::new(8, 8);
        let (fb, tex, rb, buf) = (FAR, FAR + 1, FAR + 2, FAR + 3);
        let texel = [0u8; 16];
        // SAFETY: plain GL calls; `texel` holds a 2x2 RGBA image, the buffer
        // data 16 bytes.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 2, 2, 0, GL_RGBA, GL_UNSIGNED_BYTE, texel.as_ptr().cast());
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, 2, 2);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb);
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
            assert_eq!(attachment_name(GL_COLOR_ATTACHMENT0) as GLuint, tex);
            assert_eq!(attachment_name(GL_DEPTH_ATTACHMENT) as GLuint, rb);

            glBindBuffer(GL_ARRAY_BUFFER, buf);
            glBufferData(GL_ARRAY_BUFFER, 16, texel.as_ptr().cast(), GL_STATIC_DRAW);
            glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE_B, 0, core::ptr::null());
            let mut v = -1;
            glGetVertexAttribiv(1, GL_VERTEX_ATTRIB_ARRAY_BUFFER_BINDING, &mut v);
            assert_eq!(v as GLuint, buf);
            glBindBufferBase(GL_UNIFORM_BUFFER, 2, buf);
            let mut u = [-1; 4];
            glGetIntegeri_v(GL_UNIFORM_BUFFER_BINDING, 2, u.as_mut_ptr());
            assert_eq!(u[0] as GLuint, buf);
            assert_eq!(geti(GL_UNIFORM_BUFFER_BINDING) as GLuint, buf);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /// A far name borrows the TOP free slot, so the names glGen* hands out
    /// are the ones it would hand out with no far name alive.
    #[test]
    fn a_far_name_leaves_glgen_its_usual_names() {
        let gl = TestGl::new(8, 8);
        let mut got = [0; 2];
        // SAFETY: plain GL calls; `got` has room for two names.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, FAR);
            glGenTextures(2, got.as_mut_ptr());
        }
        assert_eq!(got, [1, 2]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /// The one refusal this module adds: a small name whose slot a far name
    /// holds. It is GL_OUT_OF_MEMORY (the pool cannot give it an object),
    /// never the far name's object under a second name; once the far name
    /// is deleted, the small name binds.
    #[test]
    fn a_small_name_whose_slot_a_far_name_holds_is_refused_until_it_goes() {
        let gl = TestGl::new(8, 8);
        let held = (MAX_TEXTURES - 1) as GLuint;
        // SAFETY: plain GL calls; one name per delete.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, FAR);
            assert_eq!(gl.ctx().bound_texture_2d[0], held, "the far name took the top slot");
            assert_eq!(glIsTexture(held), GL_FALSE_B, "the slot is not an object under its own number");
            glBindTexture(GL_TEXTURE_CUBE_MAP, held);
            assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
            assert_eq!(geti(GL_TEXTURE_BINDING_CUBE_MAP), 0, "a refused bind binds nothing");
            assert_eq!(glIsTexture(FAR), GL_TRUE_B);
            glDeleteTextures(1, &FAR);
            glBindTexture(GL_TEXTURE_CUBE_MAP, held);
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!(geti(GL_TEXTURE_BINDING_CUBE_MAP) as GLuint, held);
        }
    }

    /// FAR_NAMES of one type live at once; the next is GL_OUT_OF_MEMORY and
    /// keeps the binding, and a delete makes room again.
    #[test]
    fn far_names_past_the_table_are_out_of_memory_and_keep_the_binding() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls; one name per delete.
        unsafe {
            for i in 0..FAR_NAMES as GLuint {
                glBindBuffer(GL_ARRAY_BUFFER, FAR + i);
            }
            assert_eq!(gl.error(), GL_NO_ERROR);
            glBindBuffer(GL_ARRAY_BUFFER, TOP);
            assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
            assert_eq!(geti(GL_ARRAY_BUFFER_BINDING) as GLuint, FAR + FAR_NAMES as GLuint - 1);
            assert_eq!(glIsBuffer(TOP), GL_FALSE_B);
            glDeleteBuffers(1, &FAR);
            glBindBuffer(GL_ARRAY_BUFFER, TOP);
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!(glIsBuffer(TOP), GL_TRUE_B);
        }
    }

    /// A new share group and a new context start with no far names: what a
    /// previous context left in the tables does not outlive its objects.
    #[test]
    fn a_new_context_starts_with_no_far_names() {
        {
            let _gl = TestGl::new(8, 8);
            // SAFETY: plain GL calls.
            unsafe {
                glBindTexture(GL_TEXTURE_2D, FAR);
                glBindFramebuffer(GL_FRAMEBUFFER, FAR);
            }
        }
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            assert_eq!((glIsTexture(FAR), glIsFramebuffer(FAR)), (GL_FALSE_B, GL_FALSE_B));
            glBindTexture(GL_TEXTURE_2D, (MAX_TEXTURES - 1) as GLuint);
            glBindFramebuffer(GL_FRAMEBUFFER, (MAX_FRAMEBUFFERS - 1) as GLuint);
        }
        assert_eq!(gl.error(), GL_NO_ERROR, "the top slots are free again");
    }
}
