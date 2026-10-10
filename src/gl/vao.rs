// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Vertex array objects (GL_OES_vertex_array_object; core in ES 3.0).
//!
//! A VAO snapshots the vertex-attribute arrays (glVertexAttribPointer layout +
//! the enable bits) and the ELEMENT_ARRAY_BUFFER binding. The library keeps the
//! currently bound VAO's state *live* in the context (`attribs` +
//! `bound_element_buffer`), exactly where the ES 2.0 path already reads it, so a
//! VAO object is just a saved copy swapped in on glBindVertexArray. The
//! ARRAY_BUFFER binding is deliberately not VAO state: glVertexAttribPointer
//! captures it into each attribute's `buffer` at call time.
//!
//! `vaos[0]` backs the default VAO (name 0): it holds the default's state while a
//! non-default VAO is bound. Generated names use slots 1..MAX_VAOS-1.
#![allow(non_snake_case)]

use super::buffer::debug_bind_hist;
use super::consts::*;
use super::context::{self, gl_trace, GlContext, GlVao, GlVertexAttrib, MAX_VAOS, MAX_VERTEX_ATTRIBS};
use super::types::*;

/// A VAO slot as memset(0) leaves it.
const fn zeroed_vao() -> GlVao {
    const ATTRIB: GlVertexAttrib = GlVertexAttrib {
        enabled: false,
        buffer: 0,
        size: 0,
        type_: 0,
        normalized: 0,
        stride: 0,
        offset: 0,
        divisor: 0,
        integer: false,
    };
    GlVao { used: false, attribs: [ATTRIB; MAX_VERTEX_ATTRIBS], element_buffer: 0 }
}

/// `GlVao::element_buffer` of a name glGenVertexArrays returned that has not
/// been bound yet. ES 3.0 6.1.10: such a name is not a vertex array object
/// until the first glBindVertexArray creates it, so glIsVertexArray answers
/// GL_FALSE for it. `GlVao` (a C-view struct, see context.rs) has no field for
/// that state; no buffer name reaches `u32::MAX` (names are table indices), so
/// the element binding carries it and the first bind replaces it with 0.
const VAO_NOT_YET_BOUND: GLuint = GLuint::MAX;

/// Application pointers to GLuint arrays carry no alignment guarantee: the
/// names are read and written one by one, unaligned.
///
/// # Safety
/// `p` must be valid for reading `i + 1` GLuints.
unsafe fn name_at(p: *const GLuint, i: usize) -> GLuint {
    p.wrapping_add(i).read_unaligned()
}

fn vao_save_live(c: &mut GlContext, v: usize) {
    c.vaos[v].attribs = c.attribs;
    c.vaos[v].element_buffer = c.bound_element_buffer;
}

fn vao_load_live(c: &mut GlContext, v: usize) {
    c.attribs = c.vaos[v].attribs;
    c.bound_element_buffer = c.vaos[v].element_buffer;
}

#[no_mangle]
pub unsafe extern "C" fn glGenVertexArraysOES(n: GLsizei, arrays: *mut GLuint) {
    gl_trace!(glGenVertexArraysOES);
    let Some(c) = context::current() else { return };
    if arrays.is_null() {
        return;
    }
    if n < 0 {
        c.set_error(GL_INVALID_VALUE, "glGenVertexArraysOES");
        return;
    }
    let n = n as usize;
    let mut made = 0usize;
<<<<<<< HEAD
    for (h, vao) in c.vaos.iter_mut().enumerate().take(MAX_VAOS).skip(1) {
=======
    for (h, vao) in c.vaos.iter_mut().enumerate().skip(1) {
>>>>>>> feace3b018878aa4f6f5201da2eb14d5e987f2e3
        if made >= n {
            break;
        }
        if !vao.used {
            *vao = zeroed_vao();
            vao.used = true;
            vao.element_buffer = VAO_NOT_YET_BOUND;
            // SAFETY: the application passes room for `n` names (n >= 0
            // checked above); written unaligned (see name_at).
            arrays.wrapping_add(made).write_unaligned(h as GLuint);
            made += 1;
        }
    }
    /* pool exhausted: 0 is not a valid VAO name */
    for i in made..n {
        // SAFETY: as above.
        arrays.wrapping_add(i).write_unaligned(0);
    }
}

#[no_mangle]
pub unsafe extern "C" fn glBindVertexArrayOES(array: GLuint) {
    gl_trace!(glBindVertexArrayOES);
    let Some(c) = context::current() else { return };
    if array as usize >= MAX_VAOS || (array != 0 && !c.vaos[array as usize].used) {
        c.set_error(GL_INVALID_OPERATION, "glBindVertexArrayOES"); /* not a name returned by glGenVertexArrays */
        return;
    }
    debug_bind_hist(3, array);
    if array == c.bound_vertex_array {
        return;
    }
    if c.vaos[array as usize].element_buffer == VAO_NOT_YET_BOUND {
        c.vaos[array as usize].element_buffer = 0; /* the first bind creates the object */
    }
    vao_save_live(c, c.bound_vertex_array as usize);
    vao_load_live(c, array as usize);
    c.bound_vertex_array = array;
}

#[no_mangle]
pub unsafe extern "C" fn glDeleteVertexArraysOES(n: GLsizei, arrays: *const GLuint) {
    gl_trace!(glDeleteVertexArraysOES);
    let Some(c) = context::current() else { return };
    /* a negative count is the error whatever the pointer (dEQP-GLES3
     * negative_api.vertex_array.delete_vertex_arrays passes null) */
    if n < 0 {
        c.set_error(GL_INVALID_VALUE, "glDeleteVertexArraysOES");
        return;
    }
    if arrays.is_null() {
        return;
    }
    for i in 0..n as usize {
        // SAFETY: the application passes `n` names (n >= 0 checked above).
        let h = name_at(arrays, i);
        if h == 0 || h as usize >= MAX_VAOS || !c.vaos[h as usize].used {
            continue;
        }
        if h == c.bound_vertex_array {
            /* deleting the bound VAO reverts the binding to the default (0) */
            vao_load_live(c, 0);
            c.bound_vertex_array = 0;
        }
        c.vaos[h as usize] = zeroed_vao();
    }
}

#[no_mangle]
pub unsafe extern "C" fn glIsVertexArrayOES(array: GLuint) -> GLboolean {
    gl_trace!(glIsVertexArrayOES);
    let Some(c) = context::current() else { return GL_FALSE_B };
    if array == 0 || array as usize >= MAX_VAOS {
        return GL_FALSE_B;
    }
    let v = &c.vaos[array as usize];
    if v.used && v.element_buffer != VAO_NOT_YET_BOUND {
        GL_TRUE_B
    } else {
        GL_FALSE_B
    }
}

/* GL_OES_vertex_array_object became core in OpenGL ES 3.0 under names without
 * the suffix. They are the same objects and the same calls; an application
 * built against gl3.h (and dEQP, which resolves every ES 3.0 entry point and
 * calls it unconditionally) sees only these. */

/// # Safety
/// As [`glGenVertexArraysOES`].
#[no_mangle]
pub unsafe extern "C" fn glGenVertexArrays(n: GLsizei, arrays: *mut GLuint) {
    // SAFETY: the caller's contract, forwarded.
    unsafe { glGenVertexArraysOES(n, arrays) };
}

/// # Safety
/// As [`glBindVertexArrayOES`].
#[no_mangle]
pub unsafe extern "C" fn glBindVertexArray(array: GLuint) {
    // SAFETY: as above.
    unsafe { glBindVertexArrayOES(array) };
}

/// # Safety
/// As [`glDeleteVertexArraysOES`].
#[no_mangle]
pub unsafe extern "C" fn glDeleteVertexArrays(n: GLsizei, arrays: *const GLuint) {
    // SAFETY: as above.
    unsafe { glDeleteVertexArraysOES(n, arrays) };
}

/// # Safety
/// As [`glIsVertexArrayOES`].
#[no_mangle]
pub unsafe extern "C" fn glIsVertexArray(array: GLuint) -> GLboolean {
    // SAFETY: as above.
    unsafe { glIsVertexArrayOES(array) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gl::buffer::{glBindBuffer, glDeleteBuffers, glGenBuffers};
    use crate::gl::vertex::*;
    use crate::testutil::TestGl;
    use core::ffi::c_void;

    fn gen_vao() -> GLuint {
        let mut v = 0;
        // SAFETY: room for one name.
        unsafe { glGenVertexArraysOES(1, &mut v) };
        assert_ne!(v, 0);
        v
    }

    fn gen_buffer() -> GLuint {
        let mut h = 0;
        // SAFETY: room for one name.
        unsafe { glGenBuffers(1, &mut h) };
        assert_ne!(h, 0);
        h
    }

    fn binding() -> GLint {
        let mut v = -1;
        // SAFETY: room for one value.
        unsafe { crate::gl::state::glGetIntegerv(GL_VERTEX_ARRAY_BINDING, &mut v) };
        v
    }

    fn element_binding() -> GLint {
        let mut v = -1;
        // SAFETY: room for one value.
        unsafe { crate::gl::state::glGetIntegerv(GL_ELEMENT_ARRAY_BUFFER_BINDING, &mut v) };
        v
    }

    #[test]
    fn zeroed_vao_is_the_default_array_state() {
        let v = zeroed_vao();
        assert!(!v.used);
        assert_eq!(v.element_buffer, 0);
        assert!(v.attribs.iter().all(|a| !a.enabled && a.buffer == 0 && a.offset == 0 && a.divisor == 0 && !a.integer));
    }

    #[test]
    fn gen_returns_distinct_nonzero_names() {
        let gl = TestGl::new(8, 8);
        let mut names = [0; 4];
        // SAFETY: room for four names.
        unsafe { glGenVertexArraysOES(4, names.as_mut_ptr()) };
        for (i, n) in names.iter().enumerate() {
            assert!(*n != 0 && (*n as usize) < MAX_VAOS);
            assert!(!names[..i].contains(n));
        }
        assert!(!names.contains(&gen_vao()));
        assert_eq!(binding(), 0, "generating does not bind");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn gen_negative_count_is_invalid_value() {
        let gl = TestGl::new(8, 8);
        let mut v = 55;
        // SAFETY: nothing is written for n < 0; null is tolerated.
        unsafe {
            glGenVertexArraysOES(-1, &mut v);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glGenVertexArraysOES(2, core::ptr::null_mut());
        }
        assert_eq!(v, 55);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn gen_beyond_the_pool_returns_zero_names() {
        let gl = TestGl::new(8, 8);
        let mut names = [77; MAX_VAOS + 1];
        // SAFETY: room for every name asked for.
        unsafe { glGenVertexArraysOES(names.len() as GLsizei, names.as_mut_ptr()) };
        /* slot 0 backs the default VAO: MAX_VAOS - 1 names exist */
        assert!(names[..MAX_VAOS - 1].iter().all(|&n| n != 0));
        assert_eq!(names[MAX_VAOS - 1..], [0, 0]);
        // SAFETY: valid names.
        unsafe { glDeleteVertexArraysOES(1, &names[3]) };
        assert_eq!(gen_vao(), names[3], "a deleted name is available again");
        drop(gl);
    }

    #[test]
    fn is_vertex_array_is_false_until_first_bound() {
        let gl = TestGl::new(8, 8);
        let v = gen_vao();
        // SAFETY: plain GL calls.
        unsafe {
            assert_eq!(glIsVertexArrayOES(v), GL_FALSE_B);
            glBindVertexArrayOES(v);
            assert_eq!(glIsVertexArrayOES(v), GL_TRUE_B);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn is_vertex_array_for_bound_zero_unknown_and_deleted_names() {
        let gl = TestGl::new(8, 8);
        let v = gen_vao();
        // SAFETY: plain GL calls.
        unsafe {
            glBindVertexArrayOES(v);
            assert_eq!(glIsVertexArrayOES(v), GL_TRUE_B);
            assert_eq!(glIsVertexArrayOES(0), GL_FALSE_B, "the default VAO is not an object name");
            assert_eq!(glIsVertexArrayOES(v + 1), GL_FALSE_B);
            assert_eq!(glIsVertexArrayOES(MAX_VAOS as GLuint), GL_FALSE_B);
            assert_eq!(glIsVertexArrayOES(GLuint::MAX), GL_FALSE_B);
            glDeleteVertexArraysOES(1, &v);
            assert_eq!(glIsVertexArrayOES(v), GL_FALSE_B);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn bind_updates_the_binding_query() {
        let gl = TestGl::new(8, 8);
        let v = gen_vao();
        assert_eq!(binding(), 0);
        // SAFETY: plain GL calls.
        unsafe { glBindVertexArrayOES(v) };
        assert_eq!(binding(), v as GLint);
        // SAFETY: plain GL calls.
        unsafe { glBindVertexArrayOES(0) };
        assert_eq!(binding(), 0);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn bind_of_a_name_that_was_not_generated_is_invalid_operation() {
        let gl = TestGl::new(8, 8);
        let v = gen_vao();
        // SAFETY: plain GL calls.
        unsafe {
            glBindVertexArrayOES(v);
            glBindVertexArrayOES(v + 1);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glBindVertexArrayOES(MAX_VAOS as GLuint);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glBindVertexArrayOES(GLuint::MAX);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
        assert_eq!(binding(), v as GLint, "the binding is unchanged");
    }

    #[test]
    fn a_new_vao_starts_with_default_array_state() {
        let gl = TestGl::new(8, 8);
        let (vbo, ibo) = (gen_buffer(), gen_buffer());
        // SAFETY: plain GL calls.
        unsafe {
            glBindBuffer(GL_ARRAY_BUFFER, vbo);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ibo);
            glVertexAttribPointer(0, 2, GL_SHORT, 1, 8, 4 as *const c_void);
            glEnableVertexAttribArray(0);
            glVertexAttribDivisor(0, 2);
            glBindVertexArrayOES(gen_vao());
        }
        let c = gl.ctx();
        let a = c.attribs[0];
        assert!(!a.enabled && !a.integer);
        assert_eq!((a.buffer, a.size, a.type_, a.normalized, a.stride, a.offset, a.divisor), (0, 0, 0, 0, 0, 0, 0));
        assert_eq!(element_binding(), 0);
        assert_eq!(c.bound_array_buffer, vbo, "ARRAY_BUFFER is not VAO state");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn vao_snapshots_and_restores_arrays_and_the_element_buffer() {
        let gl = TestGl::new(8, 8);
        let (vbo_a, ibo_a, vbo_b, ibo_b) = (gen_buffer(), gen_buffer(), gen_buffer(), gen_buffer());
        let (va, vb) = (gen_vao(), gen_vao());
        // SAFETY: plain GL calls; the pointers are buffer offsets.
        unsafe {
            glBindVertexArrayOES(va);
            glBindBuffer(GL_ARRAY_BUFFER, vbo_a);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ibo_a);
            glVertexAttribPointer(1, 3, GL_FLOAT, 0, 12, core::ptr::null());
            glEnableVertexAttribArray(1);

            glBindVertexArrayOES(vb);
            glBindBuffer(GL_ARRAY_BUFFER, vbo_b);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ibo_b);
            glVertexAttribIPointer(2, 1, GL_UNSIGNED_BYTE, 4, 7 as *const c_void);
            glVertexAttribDivisor(2, 5);
            glEnableVertexAttribArray(2);

            glBindVertexArrayOES(va);
        }
        let c = gl.ctx();
        assert_eq!(c.bound_element_buffer, ibo_a);
        let a1 = c.attribs[1];
        assert!(a1.enabled);
        assert_eq!((a1.buffer, a1.size, a1.type_, a1.stride, a1.offset), (vbo_a, 3, GL_FLOAT, 12, 0));
        assert!(!c.attribs[2].enabled, "vb's attribute 2 does not leak into va");
        assert_eq!(c.attribs[2].divisor, 0);

        // SAFETY: plain GL calls.
        unsafe { glBindVertexArrayOES(vb) };
        let c = gl.ctx();
        assert_eq!(c.bound_element_buffer, ibo_b);
        let a2 = c.attribs[2];
        assert!(a2.enabled && a2.integer);
        assert_eq!(
            (a2.buffer, a2.size, a2.type_, a2.stride, a2.offset, a2.divisor),
            (vbo_b, 1, GL_UNSIGNED_BYTE, 4, 7, 5)
        );
        assert!(!c.attribs[1].enabled);
        assert_eq!(c.bound_array_buffer, vbo_b, "ARRAY_BUFFER follows the last glBindBuffer, not the VAO");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn the_default_vao_keeps_its_state_across_other_bindings() {
        let gl = TestGl::new(8, 8);
        let ibo = gen_buffer();
        let v = gen_vao();
        // SAFETY: plain GL calls.
        unsafe {
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ibo);
            glVertexAttribPointer(0, 4, GL_UNSIGNED_BYTE, 1, 0, 3 as *const c_void);
            glEnableVertexAttribArray(0);

            glBindVertexArrayOES(v);
            glDisableVertexAttribArray(0);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
            glBindVertexArrayOES(0);
        }
        let c = gl.ctx();
        assert!(c.attribs[0].enabled);
        assert_eq!((c.attribs[0].type_, c.attribs[0].offset), (GL_UNSIGNED_BYTE, 3));
        assert_eq!(c.bound_element_buffer, ibo);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn rebinding_the_bound_vao_keeps_the_live_state() {
        let gl = TestGl::new(8, 8);
        let v = gen_vao();
        // SAFETY: plain GL calls.
        unsafe {
            glBindVertexArrayOES(v);
            glEnableVertexAttribArray(3);
            glBindVertexArrayOES(v);
        }
        assert!(gl.ctx().attribs[3].enabled);
        assert_eq!(binding(), v as GLint);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn current_attribute_values_are_not_vao_state() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glVertexAttrib4f(0, 1.0, 2.0, 3.0, 4.0);
            glBindVertexArrayOES(gen_vao());
        }
        assert_eq!(gl.ctx().current_attrib[0], [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn deleting_the_bound_vao_reverts_to_the_default_vao() {
        let gl = TestGl::new(8, 8);
        let (ibo0, ibo1) = (gen_buffer(), gen_buffer());
        let v = gen_vao();
        // SAFETY: plain GL calls.
        unsafe {
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ibo0);
            glEnableVertexAttribArray(0);

            glBindVertexArrayOES(v);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ibo1);
            glEnableVertexAttribArray(5);
            glDeleteVertexArraysOES(1, &v);
        }
        assert_eq!(binding(), 0);
        let c = gl.ctx();
        assert!(c.attribs[0].enabled && !c.attribs[5].enabled, "the default VAO's arrays are live again");
        assert_eq!(c.bound_element_buffer, ibo0);
        assert!(!c.vaos[v as usize].used);
        assert_eq!(gl.error(), GL_NO_ERROR);
        // SAFETY: plain GL calls.
        unsafe { glBindVertexArrayOES(v) };
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "the name is gone");
    }

    #[test]
    fn deleting_an_unbound_vao_keeps_the_current_one() {
        let gl = TestGl::new(8, 8);
        let (keep, victim) = (gen_vao(), gen_vao());
        // SAFETY: plain GL calls.
        unsafe {
            glBindVertexArrayOES(keep);
            glEnableVertexAttribArray(1);
            glDeleteVertexArraysOES(1, &victim);
        }
        assert_eq!(binding(), keep as GLint);
        assert!(gl.ctx().attribs[1].enabled);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn a_reused_name_does_not_inherit_the_deleted_vaos_state() {
        let gl = TestGl::new(8, 8);
        let v = gen_vao();
        // SAFETY: plain GL calls.
        unsafe {
            glBindVertexArrayOES(v);
            glEnableVertexAttribArray(2);
            glBindVertexArrayOES(0); /* saves the state into the slot */
            glDeleteVertexArraysOES(1, &v);
        }
        let again = gen_vao();
        assert_eq!(again, v);
        // SAFETY: plain GL calls.
        unsafe { glBindVertexArrayOES(again) };
        assert!(!gl.ctx().attribs[2].enabled);
    }

    #[test]
    fn delete_ignores_zero_and_unknown_names_and_rejects_a_negative_count() {
        let gl = TestGl::new(8, 8);
        let v = gen_vao();
        let names = [0, v + 1, MAX_VAOS as GLuint, GLuint::MAX];
        // SAFETY: `names` holds the four names passed; null is tolerated.
        unsafe {
            glBindVertexArrayOES(v);
            glDeleteVertexArraysOES(names.len() as GLsizei, names.as_ptr());
            glDeleteVertexArraysOES(1, core::ptr::null());
            assert_eq!(gl.error(), GL_NO_ERROR);
            glDeleteVertexArraysOES(-1, &v);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            assert_eq!(glIsVertexArrayOES(v), GL_TRUE_B);
        }
    }

    /// A negative count is GL_INVALID_VALUE whatever the pointer (dEQP-GLES3
    /// negative_api.vertex_array.delete_vertex_arrays passes null); the null
    /// early-out came first until 2026-09-24.
    #[test]
    fn delete_vertex_arrays_with_a_negative_count_and_null_is_invalid_value() {
        let gl = TestGl::new(4, 4);
        // SAFETY: the pointer is never read.
        unsafe { glDeleteVertexArrays(-1, core::ptr::null()) };
        assert_eq!(gl.error(), GL_INVALID_VALUE);
    }

    #[test]
    fn a_generated_but_unbound_vao_binds_with_no_element_buffer() {
        let gl = TestGl::new(8, 8);
        let v = gen_vao();
        // SAFETY: plain GL call.
        unsafe { glBindVertexArrayOES(v) };
        assert_eq!(element_binding(), 0, "the not-yet-bound marker never reaches the binding");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn gen_and_delete_take_misaligned_name_arrays() {
        let gl = TestGl::new(8, 8);
        /* names at an odd byte offset: a *const GLuint the ABI does not align */
        let mut buf = [0u8; 1 + 2 * 4];
        let p = buf[1..].as_mut_ptr() as *mut GLuint;
        // SAFETY: `p` has room for two names (unaligned).
        unsafe { glGenVertexArraysOES(2, p) };
        let a = u32::from_ne_bytes(buf[1..5].try_into().unwrap());
        let b = u32::from_ne_bytes(buf[5..9].try_into().unwrap());
        assert!(a != 0 && b != 0 && a != b);
        // SAFETY: as above.
        unsafe {
            glBindVertexArrayOES(a);
            glDeleteVertexArraysOES(2, p);
            assert_eq!(glIsVertexArrayOES(a), GL_FALSE_B);
        }
        assert_eq!(binding(), 0);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn deleting_the_element_buffer_unbinds_it_from_the_bound_vao() {
        let gl = TestGl::new(8, 8);
        let ibo = gen_buffer();
        let v = gen_vao();
        // SAFETY: plain GL calls.
        unsafe {
            glBindVertexArrayOES(v);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ibo);
            glDeleteBuffers(1, &ibo);
            assert_eq!(element_binding(), 0);
            glBindVertexArrayOES(0);
            glBindVertexArrayOES(v);
        }
        assert_eq!(element_binding(), 0, "the VAO does not resurrect the deleted buffer");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }
}
