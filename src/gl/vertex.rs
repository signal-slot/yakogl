// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Vertex attribute array state.
//!
//! glVertexAttribPointer records, per attribute index, the source layout (size,
//! type, stride, byte offset) and the ARRAY_BUFFER bound at the time of the call
//! (GLES2 requires a bound VBO - the offset is a byte offset into it). The draw
//! resolver turns each enabled attribute the linked VS consumes into a
//! `GlesAttrib`.
#![allow(non_snake_case)]

use core::ffi::c_void;

use super::consts::*;
use super::context::{self, gl_trace, GlContext, MAX_VERTEX_ATTRIBS};
use super::types::*;

/// ES 3.0 2.9: with a vertex array object other than the default bound, an
/// array must live in a buffer object: a non-null client pointer while
/// ARRAY_BUFFER is 0 is GL_INVALID_OPERATION.
fn client_array_with_vao(c: &GlContext, pointer: *const c_void) -> bool {
    c.bound_vertex_array != 0 && c.bound_array_buffer == 0 && !pointer.is_null()
}

#[no_mangle]
pub unsafe extern "C" fn glVertexAttribPointer(
    index: GLuint,
    size: GLint,
    type_: GLenum,
    normalized: GLboolean,
    stride: GLsizei,
    pointer: *const c_void,
) {
    gl_trace!(glVertexAttribPointer);
    let Some(c) = context::current() else { return };
    if index as usize >= MAX_VERTEX_ATTRIBS || !(1..=4).contains(&size) || stride < 0 {
        c.set_error(GL_INVALID_VALUE, "glVertexAttribPointer");
        return;
    }

    match type_ {
        GL_BYTE | GL_UNSIGNED_BYTE | GL_SHORT | GL_UNSIGNED_SHORT | GL_INT | GL_UNSIGNED_INT | GL_FLOAT | GL_FIXED
        | GL_HALF_FLOAT => {}
        /* ES 3.0 packed types: the draw path unpacks them (draw.rs attrib_row) */
        GL_INT_2_10_10_10_REV | GL_UNSIGNED_INT_2_10_10_10_REV => {
            if size != 4 {
                c.set_error(GL_INVALID_OPERATION, "glVertexAttribPointer");
                return;
            }
        }
        _ => {
            c.set_error(GL_INVALID_ENUM, "glVertexAttribPointer");
            return;
        }
    }
    if client_array_with_vao(c, pointer) {
        c.set_error(GL_INVALID_OPERATION, "glVertexAttribPointer");
        return;
    }

    let buffer = c.bound_array_buffer;
    let a = &mut c.attribs[index as usize];
    a.buffer = buffer; /* 0 => client array (see the draw resolver) */
    a.size = size;
    a.type_ = type_;
    a.normalized = normalized;
    a.stride = stride;
    a.offset = pointer as usize; /* byte offset into the VBO */
    a.integer = false;
}

/// OpenGL ES 3.0: an integer attribute array. The components reach the
/// shader as integers (an ivec/uvec input); 8- and 16-bit types are widened
/// at draw time like every non-dword format (the draw resolver's attribute
/// conversion).
#[no_mangle]
pub unsafe extern "C" fn glVertexAttribIPointer(
    index: GLuint,
    size: GLint,
    type_: GLenum,
    stride: GLsizei,
    pointer: *const c_void,
) {
    gl_trace!(glVertexAttribIPointer);
    let Some(c) = context::current() else { return };
    if index as usize >= MAX_VERTEX_ATTRIBS || !(1..=4).contains(&size) || stride < 0 {
        c.set_error(GL_INVALID_VALUE, "glVertexAttribIPointer");
        return;
    }
    match type_ {
        GL_BYTE | GL_UNSIGNED_BYTE | GL_SHORT | GL_UNSIGNED_SHORT | GL_INT | GL_UNSIGNED_INT => {}
        _ => {
            c.set_error(GL_INVALID_ENUM, "glVertexAttribIPointer");
            return;
        }
    }
    if client_array_with_vao(c, pointer) {
        c.set_error(GL_INVALID_OPERATION, "glVertexAttribIPointer");
        return;
    }
    let buffer = c.bound_array_buffer;
    let a = &mut c.attribs[index as usize];
    a.buffer = buffer;
    a.size = size;
    a.type_ = type_;
    a.normalized = GL_FALSE_B;
    a.stride = stride;
    a.offset = pointer as usize;
    a.integer = true;
}

/// OpenGL ES 3.0 instancing: the attribute advances once per `divisor`
/// instances (0: per vertex). The instanced draws honour it.
#[no_mangle]
pub unsafe extern "C" fn glVertexAttribDivisor(index: GLuint, divisor: GLuint) {
    gl_trace!(glVertexAttribDivisor);
    let Some(c) = context::current() else { return };
    if index as usize >= MAX_VERTEX_ATTRIBS {
        c.set_error(GL_INVALID_VALUE, "glVertexAttribDivisor");
        return;
    }
    c.attribs[index as usize].divisor = divisor;
}

#[no_mangle]
pub unsafe extern "C" fn glEnableVertexAttribArray(index: GLuint) {
    gl_trace!(glEnableVertexAttribArray);
    let Some(c) = context::current() else { return };
    if index as usize >= MAX_VERTEX_ATTRIBS {
        c.set_error(GL_INVALID_VALUE, "glEnableVertexAttribArray");
        return;
    }
    c.attribs[index as usize].enabled = true;
}

#[no_mangle]
pub unsafe extern "C" fn glDisableVertexAttribArray(index: GLuint) {
    gl_trace!(glDisableVertexAttribArray);
    let Some(c) = context::current() else { return };
    if index as usize >= MAX_VERTEX_ATTRIBS {
        c.set_error(GL_INVALID_VALUE, "glDisableVertexAttribArray");
        return;
    }
    c.attribs[index as usize].enabled = false;
}

/* --------------------------------------------------------------------------- */
/* Current (constant) attribute values: what a VS input reads while its array
 * is disabled. The draw resolver fetches them with a zero vertex stride. */

fn set_current(index: GLuint, x: GLfloat, y: GLfloat, z: GLfloat, w: GLfloat) {
    let Some(c) = context::current() else { return };
    if index as usize >= MAX_VERTEX_ATTRIBS {
        /* the C helper raised it, so __func__ named the helper */
        c.set_error(GL_INVALID_VALUE, "set_current");
        return;
    }
    c.current_attrib[index as usize] = [x, y, z, w];
}

#[no_mangle]
pub unsafe extern "C" fn glVertexAttrib1f(index: GLuint, x: GLfloat) {
    set_current(index, x, 0.0, 0.0, 1.0);
}

#[no_mangle]
pub unsafe extern "C" fn glVertexAttrib2f(index: GLuint, x: GLfloat, y: GLfloat) {
    set_current(index, x, y, 0.0, 1.0);
}

#[no_mangle]
pub unsafe extern "C" fn glVertexAttrib3f(index: GLuint, x: GLfloat, y: GLfloat, z: GLfloat) {
    set_current(index, x, y, z, 1.0);
}

#[no_mangle]
pub unsafe extern "C" fn glVertexAttrib4f(index: GLuint, x: GLfloat, y: GLfloat, z: GLfloat, w: GLfloat) {
    set_current(index, x, y, z, w);
}

/// The first `N` floats of an application array. `GLfloat *` carries no
/// alignment guarantee in practice, so each value is read unaligned.
///
/// # Safety
/// `v` must be valid for reading `N` floats.
unsafe fn read_floats<const N: usize>(v: *const GLfloat) -> [GLfloat; N] {
    core::array::from_fn(|i| v.wrapping_add(i).read_unaligned())
}

#[no_mangle]
pub unsafe extern "C" fn glVertexAttrib1fv(index: GLuint, v: *const GLfloat) {
    if v.is_null() {
        return;
    }
    // SAFETY: the application passes at least 1 float, not necessarily aligned.
    let v = read_floats::<1>(v);
    set_current(index, v[0], 0.0, 0.0, 1.0);
}

#[no_mangle]
pub unsafe extern "C" fn glVertexAttrib2fv(index: GLuint, v: *const GLfloat) {
    if v.is_null() {
        return;
    }
    // SAFETY: the application passes at least 2 floats, not necessarily aligned.
    let v = read_floats::<2>(v);
    set_current(index, v[0], v[1], 0.0, 1.0);
}

#[no_mangle]
pub unsafe extern "C" fn glVertexAttrib3fv(index: GLuint, v: *const GLfloat) {
    if v.is_null() {
        return;
    }
    // SAFETY: the application passes at least 3 floats, not necessarily aligned.
    let v = read_floats::<3>(v);
    set_current(index, v[0], v[1], v[2], 1.0);
}

#[no_mangle]
pub unsafe extern "C" fn glVertexAttrib4fv(index: GLuint, v: *const GLfloat) {
    if v.is_null() {
        return;
    }
    // SAFETY: the application passes at least 4 floats, not necessarily aligned.
    let v = read_floats::<4>(v);
    set_current(index, v[0], v[1], v[2], v[3]);
}

/// Integer-valued attribute queries; CURRENT_VERTEX_ATTRIB is float-valued and
/// handled by the callers. `None` for an unknown pname.
fn get_attrib_int(c: &GlContext, index: usize, pname: GLenum) -> Option<GLint> {
    let a = &c.attribs[index];

    Some(match pname {
        GL_VERTEX_ATTRIB_ARRAY_BUFFER_BINDING => {
            super::names::name_of(c, super::names::Kind::Buffer, a.buffer) as GLint
        }
        GL_VERTEX_ATTRIB_ARRAY_ENABLED => a.enabled as GLint,
        GL_VERTEX_ATTRIB_ARRAY_SIZE => {
            if a.size != 0 {
                a.size
            } else {
                4
            }
        }
        GL_VERTEX_ATTRIB_ARRAY_STRIDE => a.stride,
        GL_VERTEX_ATTRIB_ARRAY_TYPE => (if a.type_ != 0 { a.type_ } else { GL_FLOAT }) as GLint,
        GL_VERTEX_ATTRIB_ARRAY_NORMALIZED => a.normalized as GLint,
        GL_VERTEX_ATTRIB_ARRAY_INTEGER => a.integer as GLint,
        GL_VERTEX_ATTRIB_ARRAY_DIVISOR => a.divisor as GLint,
        _ => return None,
    })
}

#[no_mangle]
pub unsafe extern "C" fn glGetVertexAttribfv(index: GLuint, pname: GLenum, params: *mut GLfloat) {
    gl_trace!(glGetVertexAttribfv);
    let Some(c) = context::current() else { return };
    if params.is_null() {
        return;
    }
    if index as usize >= MAX_VERTEX_ATTRIBS {
        c.set_error(GL_INVALID_VALUE, "glGetVertexAttribfv");
        return;
    }
    if pname == GL_CURRENT_VERTEX_ATTRIB {
        for (i, v) in c.current_attrib[index as usize].iter().enumerate() {
            // SAFETY: CURRENT_VERTEX_ATTRIB returns four values; the
            // application passes room for them, not necessarily aligned.
            params.wrapping_add(i).write_unaligned(*v);
        }
        return;
    }
    match get_attrib_int(c, index as usize, pname) {
        // SAFETY: non-null (checked above), room for one value (unaligned).
        Some(iv) => params.write_unaligned(iv as GLfloat),
        None => c.set_error(GL_INVALID_ENUM, "glGetVertexAttribfv"),
    }
}

#[no_mangle]
pub unsafe extern "C" fn glGetVertexAttribiv(index: GLuint, pname: GLenum, params: *mut GLint) {
    gl_trace!(glGetVertexAttribiv);
    let Some(c) = context::current() else { return };
    if params.is_null() {
        return;
    }
    if index as usize >= MAX_VERTEX_ATTRIBS {
        c.set_error(GL_INVALID_VALUE, "glGetVertexAttribiv");
        return;
    }
    if pname == GL_CURRENT_VERTEX_ATTRIB {
        for (i, v) in c.current_attrib[index as usize].iter().enumerate() {
            // SAFETY: CURRENT_VERTEX_ATTRIB returns four values; the
            // application passes room for them, not necessarily aligned.
            params.wrapping_add(i).write_unaligned(*v as GLint);
        }
        return;
    }
    match get_attrib_int(c, index as usize, pname) {
        // SAFETY: non-null (checked above), room for one value (unaligned).
        Some(iv) => params.write_unaligned(iv),
        None => c.set_error(GL_INVALID_ENUM, "glGetVertexAttribiv"),
    }
}

/* OpenGL ES 3.0 integer-valued current attributes. The context keeps one
 * float quadruple per attribute, so a value is exact up to 2^24; the draw path
 * converts to float for the vertex-input registers in any case. */
#[no_mangle]
pub unsafe extern "C" fn glVertexAttribI4i(index: GLuint, x: GLint, y: GLint, z: GLint, w: GLint) {
    gl_trace!(glVertexAttribI4i);
    set_current(index, x as GLfloat, y as GLfloat, z as GLfloat, w as GLfloat);
}

#[no_mangle]
pub unsafe extern "C" fn glVertexAttribI4ui(index: GLuint, x: GLuint, y: GLuint, z: GLuint, w: GLuint) {
    gl_trace!(glVertexAttribI4ui);
    set_current(index, x as GLfloat, y as GLfloat, z as GLfloat, w as GLfloat);
}

/// # Safety
/// `v` is null or points at four GLints.
#[no_mangle]
pub unsafe extern "C" fn glVertexAttribI4iv(index: GLuint, v: *const GLint) {
    if v.is_null() {
        return;
    }
    // SAFETY: the application passes four values, not necessarily aligned.
    let v: [GLint; 4] = core::array::from_fn(|i| unsafe { v.wrapping_add(i).read_unaligned() });
    // SAFETY: no pointer is read below.
    unsafe { glVertexAttribI4i(index, v[0], v[1], v[2], v[3]) };
}

/// # Safety
/// `v` is null or points at four GLuints.
#[no_mangle]
pub unsafe extern "C" fn glVertexAttribI4uiv(index: GLuint, v: *const GLuint) {
    if v.is_null() {
        return;
    }
    // SAFETY: as above.
    let v: [GLuint; 4] = core::array::from_fn(|i| unsafe { v.wrapping_add(i).read_unaligned() });
    // SAFETY: no pointer is read below.
    unsafe { glVertexAttribI4ui(index, v[0], v[1], v[2], v[3]) };
}

/// # Safety
/// `params` is null or has room for the values `pname` returns (four for
/// GL_CURRENT_VERTEX_ATTRIB, one otherwise).
#[no_mangle]
pub unsafe extern "C" fn glGetVertexAttribIiv(index: GLuint, pname: GLenum, params: *mut GLint) {
    gl_trace!(glGetVertexAttribIiv);
    let Some(c) = context::current() else { return };
    if params.is_null() {
        return;
    }
    if index as usize >= MAX_VERTEX_ATTRIBS {
        c.set_error(GL_INVALID_VALUE, "glGetVertexAttribIiv");
        return;
    }
    if pname == GL_CURRENT_VERTEX_ATTRIB {
        for (i, v) in c.current_attrib[index as usize].iter().enumerate() {
            // SAFETY: the caller's contract (four values, any alignment).
            unsafe { params.wrapping_add(i).write_unaligned(*v as GLint) };
        }
        return;
    }
    match get_attrib_int(c, index as usize, pname) {
        // SAFETY: non-null (checked above), room for one value (unaligned).
        Some(iv) => unsafe { params.write_unaligned(iv) },
        None => c.set_error(GL_INVALID_ENUM, "glGetVertexAttribIiv"),
    }
}

/// # Safety
/// As [`glGetVertexAttribIiv`].
#[no_mangle]
pub unsafe extern "C" fn glGetVertexAttribIuiv(index: GLuint, pname: GLenum, params: *mut GLuint) {
    gl_trace!(glGetVertexAttribIuiv);
    let Some(c) = context::current() else { return };
    if params.is_null() {
        return;
    }
    if index as usize >= MAX_VERTEX_ATTRIBS {
        c.set_error(GL_INVALID_VALUE, "glGetVertexAttribIuiv");
        return;
    }
    if pname == GL_CURRENT_VERTEX_ATTRIB {
        for (i, v) in c.current_attrib[index as usize].iter().enumerate() {
            // SAFETY: the caller's contract (four values, any alignment).
            unsafe { params.wrapping_add(i).write_unaligned(*v as GLuint) };
        }
        return;
    }
    match get_attrib_int(c, index as usize, pname) {
        // SAFETY: non-null (checked above), room for one value (unaligned).
        Some(iv) => unsafe { params.write_unaligned(iv as GLuint) },
        None => c.set_error(GL_INVALID_ENUM, "glGetVertexAttribIuiv"),
    }
}

#[no_mangle]
pub unsafe extern "C" fn glGetVertexAttribPointerv(index: GLuint, pname: GLenum, pointer: *mut *mut c_void) {
    gl_trace!(glGetVertexAttribPointerv);
    let Some(c) = context::current() else { return };
    if pointer.is_null() {
        return;
    }
    if index as usize >= MAX_VERTEX_ATTRIBS {
        c.set_error(GL_INVALID_VALUE, "glGetVertexAttribPointerv");
        return;
    }
    if pname != GL_VERTEX_ATTRIB_ARRAY_POINTER {
        c.set_error(GL_INVALID_ENUM, "glGetVertexAttribPointerv");
        return;
    }
    // SAFETY: non-null (checked above), room for one pointer (unaligned).
    pointer.write_unaligned(c.attribs[index as usize].offset as *mut c_void);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gl::buffer::{glBindBuffer, glDeleteBuffers, glGenBuffers};
    use crate::testutil::TestGl;

    const MAX: GLuint = MAX_VERTEX_ATTRIBS as GLuint;

    fn geti(index: GLuint, pname: GLenum) -> GLint {
        let mut v = [-99; 4];
        // SAFETY: room for the four values CURRENT_VERTEX_ATTRIB returns.
        unsafe { glGetVertexAttribiv(index, pname, v.as_mut_ptr()) };
        v[0]
    }

    fn getf(index: GLuint, pname: GLenum) -> GLfloat {
        let mut v = [-99.0; 4];
        // SAFETY: room for the four values CURRENT_VERTEX_ATTRIB returns.
        unsafe { glGetVertexAttribfv(index, pname, v.as_mut_ptr()) };
        v[0]
    }

    fn current(index: GLuint) -> [GLfloat; 4] {
        let mut v = [-99.0; 4];
        // SAFETY: room for four values.
        unsafe { glGetVertexAttribfv(index, GL_CURRENT_VERTEX_ATTRIB, v.as_mut_ptr()) };
        v
    }

    fn pointer(index: GLuint) -> usize {
        let mut p = 0xDEAD as *mut c_void;
        // SAFETY: room for one pointer.
        unsafe { glGetVertexAttribPointerv(index, GL_VERTEX_ATTRIB_ARRAY_POINTER, &mut p) };
        p as usize
    }

    fn bound_buffer() -> GLuint {
        let mut h = 0;
        // SAFETY: room for one name.
        unsafe {
            glGenBuffers(1, &mut h);
            glBindBuffer(GL_ARRAY_BUFFER, h);
        }
        h
    }

    /* ---- glVertexAttribPointer ----------------------------------------------- */

    #[test]
    fn initial_array_state_per_the_spec() {
        let gl = TestGl::new(8, 8);
        for i in [0, MAX - 1] {
            assert_eq!(geti(i, GL_VERTEX_ATTRIB_ARRAY_ENABLED), 0);
            assert_eq!(geti(i, GL_VERTEX_ATTRIB_ARRAY_SIZE), 4);
            assert_eq!(geti(i, GL_VERTEX_ATTRIB_ARRAY_STRIDE), 0);
            assert_eq!(geti(i, GL_VERTEX_ATTRIB_ARRAY_TYPE), GL_FLOAT as GLint);
            assert_eq!(geti(i, GL_VERTEX_ATTRIB_ARRAY_NORMALIZED), 0);
            assert_eq!(geti(i, GL_VERTEX_ATTRIB_ARRAY_INTEGER), 0);
            assert_eq!(geti(i, GL_VERTEX_ATTRIB_ARRAY_DIVISOR), 0);
            assert_eq!(geti(i, GL_VERTEX_ATTRIB_ARRAY_BUFFER_BINDING), 0);
            assert_eq!(pointer(i), 0);
            assert_eq!(current(i), [0.0, 0.0, 0.0, 1.0]);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn pointer_records_the_layout() {
        let gl = TestGl::new(8, 8);
        let h = bound_buffer();
        // SAFETY: the "pointer" is a byte offset into the bound buffer.
        unsafe { glVertexAttribPointer(3, 2, GL_UNSIGNED_SHORT, 1, 12, 20 as *const c_void) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let a = gl.ctx().attribs[3];
        assert_eq!((a.buffer, a.size, a.type_, a.normalized, a.stride, a.offset), (h, 2, GL_UNSIGNED_SHORT, 1, 12, 20));
        assert!(!a.integer && !a.enabled, "setting the pointer does not enable the array");
        assert_eq!(a.divisor, 0);

        assert_eq!(geti(3, GL_VERTEX_ATTRIB_ARRAY_SIZE), 2);
        assert_eq!(geti(3, GL_VERTEX_ATTRIB_ARRAY_TYPE), GL_UNSIGNED_SHORT as GLint);
        assert_eq!(geti(3, GL_VERTEX_ATTRIB_ARRAY_NORMALIZED), 1);
        assert_eq!(geti(3, GL_VERTEX_ATTRIB_ARRAY_STRIDE), 12);
        assert_eq!(geti(3, GL_VERTEX_ATTRIB_ARRAY_BUFFER_BINDING), h as GLint);
        assert_eq!(geti(3, GL_VERTEX_ATTRIB_ARRAY_INTEGER), 0);
        assert_eq!(pointer(3), 20);
        assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_SIZE), 4, "the neighbours are untouched");
        assert_eq!(geti(4, GL_VERTEX_ATTRIB_ARRAY_BUFFER_BINDING), 0);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn pointer_captures_the_array_buffer_bound_at_call_time() {
        let gl = TestGl::new(8, 8);
        let first = bound_buffer();
        // SAFETY: plain GL calls.
        unsafe { glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null()) };
        let second = bound_buffer();
        // SAFETY: plain GL calls.
        unsafe { glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, core::ptr::null()) };
        assert_eq!(
            geti(0, GL_VERTEX_ATTRIB_ARRAY_BUFFER_BINDING),
            first as GLint,
            "a later bind does not re-point attribute 0"
        );
        assert_eq!(geti(1, GL_VERTEX_ATTRIB_ARRAY_BUFFER_BINDING), second as GLint);
        // SAFETY: plain GL calls.
        unsafe { glBindBuffer(GL_ARRAY_BUFFER, 0) };
        assert_eq!(geti(1, GL_VERTEX_ATTRIB_ARRAY_BUFFER_BINDING), second as GLint);
        assert_eq!(gl.error(), GL_NO_ERROR);
        // SAFETY: valid names.
        unsafe { glDeleteBuffers(2, [first, second].as_ptr()) };
    }

    #[test]
    fn pointer_into_client_memory_records_the_address() {
        let gl = TestGl::new(8, 8);
        let verts = [0.0f32; 8];
        // SAFETY: the array is only recorded, never read, in this test.
        unsafe { glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, verts.as_ptr() as *const c_void) };
        assert_eq!(geti(0, GL_VERTEX_ATTRIB_ARRAY_BUFFER_BINDING), 0, "0 = client array");
        assert_eq!(pointer(0), verts.as_ptr() as usize);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn pointer_accepts_every_es2_type_and_size() {
        let gl = TestGl::new(8, 8);
        for t in [GL_BYTE, GL_UNSIGNED_BYTE, GL_SHORT, GL_UNSIGNED_SHORT, GL_INT, GL_UNSIGNED_INT, GL_FLOAT, GL_FIXED] {
            for size in 1..=4 {
                // SAFETY: plain GL calls.
                unsafe { glVertexAttribPointer(5, size, t, 0, 0, core::ptr::null()) };
                assert_eq!(gl.error(), GL_NO_ERROR, "type 0x{t:x} size {size}");
                assert_eq!(geti(5, GL_VERTEX_ATTRIB_ARRAY_TYPE), t as GLint);
                assert_eq!(geti(5, GL_VERTEX_ATTRIB_ARRAY_SIZE), size);
            }
        }
    }

    #[test]
    fn pointer_invalid_arguments_leave_the_attribute_alone() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glVertexAttribPointer(1, 3, GL_SHORT, 0, 6, 8 as *const c_void);
            glVertexAttribPointer(MAX, 4, GL_FLOAT, 0, 0, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_VALUE, "index");
            glVertexAttribPointer(1, 0, GL_FLOAT, 0, 0, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_VALUE, "size 0");
            glVertexAttribPointer(1, 5, GL_FLOAT, 0, 0, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_VALUE, "size 5");
            glVertexAttribPointer(1, -1, GL_FLOAT, 0, 0, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_VALUE, "size -1");
            glVertexAttribPointer(1, 4, GL_FLOAT, 0, -1, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_VALUE, "stride");
            glVertexAttribPointer(1, 4, GL_TEXTURE_2D, 0, 0, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_ENUM, "type");
            glVertexAttribPointer(1, 4, 0, 0, 0, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_ENUM, "type 0");
        }
        let a = gl.ctx().attribs[1];
        assert_eq!((a.size, a.type_, a.stride, a.offset), (3, GL_SHORT, 6, 8));
    }

    #[test]
    fn pointer_accepts_the_es3_types() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glVertexAttribPointer(0, 3, GL_HALF_FLOAT, 0, 0, core::ptr::null());
            assert_eq!(gl.error(), GL_NO_ERROR);
            glVertexAttribPointer(0, 4, GL_INT_2_10_10_10_REV, 1, 0, core::ptr::null());
            assert_eq!(gl.error(), GL_NO_ERROR);
            glVertexAttribPointer(0, 4, GL_UNSIGNED_INT_2_10_10_10_REV, 1, 0, core::ptr::null());
            assert_eq!(gl.error(), GL_NO_ERROR);
            glVertexAttribPointer(0, 3, GL_INT_2_10_10_10_REV, 1, 0, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "a packed type needs size 4");
            glVertexAttribIPointer(0, 4, GL_INT_2_10_10_10_REV, 0, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_ENUM, "no packed integer arrays");
        }
        assert_eq!(geti(0, GL_VERTEX_ATTRIB_ARRAY_TYPE), GL_UNSIGNED_INT_2_10_10_10_REV as GLint);
    }

    #[test]
    fn client_pointer_with_a_vao_bound_is_invalid_operation() {
        let gl = TestGl::new(8, 8);
        let verts = [0.0f32; 4];
        let mut vao = 0;
        // SAFETY: the client array is only recorded.
        unsafe {
            crate::gl::vao::glGenVertexArraysOES(1, &mut vao);
            crate::gl::vao::glBindVertexArrayOES(vao);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, verts.as_ptr() as *const c_void);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glVertexAttribIPointer(0, 4, GL_INT, 0, verts.as_ptr() as *const c_void);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            /* offset 0 into no buffer (a null pointer) and buffer offsets are fine */
            glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, core::ptr::null());
            assert_eq!(gl.error(), GL_NO_ERROR);
            let b = bound_buffer();
            glVertexAttribPointer(2, 4, GL_FLOAT, 0, 0, 8 as *const c_void);
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_BUFFER_BINDING), b as GLint);
            /* the default VAO still takes client arrays */
            crate::gl::vao::glBindVertexArrayOES(0);
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, verts.as_ptr() as *const c_void);
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
        assert_eq!(pointer(0), verts.as_ptr() as usize);
    }

    #[test]
    fn value_setters_and_queries_take_misaligned_pointers() {
        let gl = TestGl::new(8, 8);
        /* four floats / ints at an odd byte offset */
        let mut buf = [0u8; 1 + 16];
        for (i, f) in [1.5f32, 2.5, 3.5, 4.5].iter().enumerate() {
            buf[1 + i * 4..5 + i * 4].copy_from_slice(&f.to_ne_bytes());
        }
        let p = buf[1..].as_mut_ptr();
        // SAFETY: `p` holds four values (unaligned).
        unsafe {
            glVertexAttrib1fv(0, p as *const GLfloat);
            glVertexAttrib2fv(1, p as *const GLfloat);
            glVertexAttrib3fv(2, p as *const GLfloat);
            glVertexAttrib4fv(3, p as *const GLfloat);
        }
        assert_eq!(current(0), [1.5, 0.0, 0.0, 1.0]);
        assert_eq!(current(1), [1.5, 2.5, 0.0, 1.0]);
        assert_eq!(current(2), [1.5, 2.5, 3.5, 1.0]);
        assert_eq!(current(3), [1.5, 2.5, 3.5, 4.5]);
        let floats =
            |b: &[u8]| -> Vec<f32> { b.chunks(4).map(|c| f32::from_ne_bytes(c.try_into().unwrap())).collect() };
        let ints = |b: &[u8]| -> Vec<i32> { b.chunks(4).map(|c| i32::from_ne_bytes(c.try_into().unwrap())).collect() };
        // SAFETY: room for four values (unaligned).
        unsafe { glGetVertexAttribfv(3, GL_CURRENT_VERTEX_ATTRIB, p as *mut GLfloat) };
        assert_eq!(floats(&buf[1..]), [1.5, 2.5, 3.5, 4.5]);
        // SAFETY: as above.
        unsafe { glGetVertexAttribiv(3, GL_CURRENT_VERTEX_ATTRIB, p as *mut GLint) };
        assert_eq!(ints(&buf[1..]), [1, 2, 3, 4]);
        // SAFETY: as above, one value.
        unsafe {
            glGetVertexAttribiv(3, GL_VERTEX_ATTRIB_ARRAY_SIZE, p as *mut GLint);
            assert_eq!(ints(&buf[1..5]), [4]);
            glGetVertexAttribfv(3, GL_VERTEX_ATTRIB_ARRAY_SIZE, p as *mut GLfloat);
            assert_eq!(floats(&buf[1..5]), [4.0]);
        }
        let mut pbuf = [0u8; 1 + core::mem::size_of::<usize>()];
        // SAFETY: room for one pointer (unaligned).
        unsafe {
            glVertexAttribPointer(5, 4, GL_FLOAT, 0, 0, 0x1234 as *const c_void);
            glGetVertexAttribPointerv(5, GL_VERTEX_ATTRIB_ARRAY_POINTER, pbuf[1..].as_mut_ptr() as *mut *mut c_void);
        }
        assert_eq!(usize::from_ne_bytes(pbuf[1..].try_into().unwrap()), 0x1234);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* ---- glVertexAttribIPointer ---------------------------------------------- */

    #[test]
    fn ipointer_records_an_integer_array() {
        let gl = TestGl::new(8, 8);
        let h = bound_buffer();
        // SAFETY: plain GL calls.
        unsafe { glVertexAttribIPointer(2, 3, GL_UNSIGNED_BYTE, 4, 16 as *const c_void) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_INTEGER), 1);
        assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_NORMALIZED), 0);
        assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_SIZE), 3);
        assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_TYPE), GL_UNSIGNED_BYTE as GLint);
        assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_STRIDE), 4);
        assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_BUFFER_BINDING), h as GLint);
        assert_eq!(pointer(2), 16);
    }

    #[test]
    fn pointer_after_ipointer_is_a_float_array_again_and_vice_versa() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glVertexAttribPointer(2, 4, GL_BYTE, 1, 0, core::ptr::null());
            glVertexAttribIPointer(2, 4, GL_INT, 0, core::ptr::null());
        }
        assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_INTEGER), 1);
        assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_NORMALIZED), 0, "integer arrays are never normalized");
        // SAFETY: plain GL calls.
        unsafe { glVertexAttribPointer(2, 4, GL_INT, 0, 0, core::ptr::null()) };
        assert_eq!(geti(2, GL_VERTEX_ATTRIB_ARRAY_INTEGER), 0);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn ipointer_accepts_integer_types_only() {
        let gl = TestGl::new(8, 8);
        for t in [GL_BYTE, GL_UNSIGNED_BYTE, GL_SHORT, GL_UNSIGNED_SHORT, GL_INT, GL_UNSIGNED_INT] {
            // SAFETY: plain GL calls.
            unsafe { glVertexAttribIPointer(0, 1, t, 0, core::ptr::null()) };
            assert_eq!(gl.error(), GL_NO_ERROR, "0x{t:x}");
        }
        for t in [GL_FLOAT, GL_FIXED, GL_HALF_FLOAT, 0] {
            // SAFETY: plain GL calls.
            unsafe { glVertexAttribIPointer(0, 1, t, 0, core::ptr::null()) };
            assert_eq!(gl.error(), GL_INVALID_ENUM, "0x{t:x}");
        }
        assert_eq!(geti(0, GL_VERTEX_ATTRIB_ARRAY_TYPE), GL_UNSIGNED_INT as GLint, "the last valid call");
    }

    #[test]
    fn ipointer_invalid_values() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glVertexAttribIPointer(MAX, 1, GL_INT, 0, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glVertexAttribIPointer(0, 0, GL_INT, 0, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glVertexAttribIPointer(0, 5, GL_INT, 0, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glVertexAttribIPointer(0, 1, GL_INT, -4, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
        assert!(!gl.ctx().attribs[0].integer);
    }

    /* ---- enable, divisor ----------------------------------------------------- */

    #[test]
    fn enable_and_disable_one_array() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glEnableVertexAttribArray(MAX - 1) };
        assert_eq!(geti(MAX - 1, GL_VERTEX_ATTRIB_ARRAY_ENABLED), 1);
        assert_eq!(getf(MAX - 1, GL_VERTEX_ATTRIB_ARRAY_ENABLED), 1.0);
        assert_eq!(geti(MAX - 2, GL_VERTEX_ATTRIB_ARRAY_ENABLED), 0);
        assert!(gl.ctx().attribs[MAX as usize - 1].enabled);
        // SAFETY: plain GL calls.
        unsafe { glDisableVertexAttribArray(MAX - 1) };
        assert_eq!(geti(MAX - 1, GL_VERTEX_ATTRIB_ARRAY_ENABLED), 0);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn enable_disable_divisor_index_out_of_range_is_invalid_value() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glEnableVertexAttribArray(MAX);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glDisableVertexAttribArray(GLuint::MAX);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glVertexAttribDivisor(MAX, 1);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
        assert!(gl.ctx().attribs.iter().all(|a| !a.enabled && a.divisor == 0));
    }

    #[test]
    fn divisor_is_recorded_and_survives_a_new_pointer() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glVertexAttribDivisor(4, 3) };
        assert_eq!(geti(4, GL_VERTEX_ATTRIB_ARRAY_DIVISOR), 3);
        assert_eq!(getf(4, GL_VERTEX_ATTRIB_ARRAY_DIVISOR), 3.0);
        // SAFETY: plain GL calls.
        unsafe { glVertexAttribPointer(4, 4, GL_FLOAT, 0, 0, core::ptr::null()) };
        assert_eq!(geti(4, GL_VERTEX_ATTRIB_ARRAY_DIVISOR), 3, "the divisor is separate state");
        // SAFETY: plain GL calls.
        unsafe { glVertexAttribDivisor(4, 0) };
        assert_eq!(geti(4, GL_VERTEX_ATTRIB_ARRAY_DIVISOR), 0);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* ---- current values ------------------------------------------------------ */

    #[test]
    fn vertex_attrib_nf_fills_the_missing_components_with_0_0_1() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glVertexAttrib1f(0, 0.5);
            glVertexAttrib2f(1, 0.5, -2.0);
            glVertexAttrib3f(2, 0.5, -2.0, 8.0);
            glVertexAttrib4f(3, 0.5, -2.0, 8.0, 0.25);
        }
        assert_eq!(current(0), [0.5, 0.0, 0.0, 1.0]);
        assert_eq!(current(1), [0.5, -2.0, 0.0, 1.0]);
        assert_eq!(current(2), [0.5, -2.0, 8.0, 1.0]);
        assert_eq!(current(3), [0.5, -2.0, 8.0, 0.25]);
        assert_eq!(current(4), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(gl.ctx().current_attrib[3], [0.5, -2.0, 8.0, 0.25]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn vertex_attrib_nfv_reads_exactly_n_components() {
        let gl = TestGl::new(8, 8);
        let v = [3.0f32, 4.0, 5.0, 6.0];
        // SAFETY: `v` holds four floats.
        unsafe {
            glVertexAttrib4f(0, 9.0, 9.0, 9.0, 9.0);
            glVertexAttrib1fv(0, v.as_ptr());
            glVertexAttrib2fv(1, v.as_ptr());
            glVertexAttrib3fv(2, v.as_ptr());
            glVertexAttrib4fv(3, v.as_ptr());
        }
        assert_eq!(current(0), [3.0, 0.0, 0.0, 1.0], "a shorter set resets the rest");
        assert_eq!(current(1), [3.0, 4.0, 0.0, 1.0]);
        assert_eq!(current(2), [3.0, 4.0, 5.0, 1.0]);
        assert_eq!(current(3), [3.0, 4.0, 5.0, 6.0]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn vertex_attrib_index_out_of_range_is_invalid_value() {
        let gl = TestGl::new(8, 8);
        let v = [1.0f32; 4];
        // SAFETY: `v` holds four floats.
        unsafe {
            glVertexAttrib1f(MAX, 1.0);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glVertexAttrib2f(MAX, 1.0, 1.0);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glVertexAttrib3f(MAX, 1.0, 1.0, 1.0);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glVertexAttrib4f(MAX, 1.0, 1.0, 1.0, 1.0);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glVertexAttrib1fv(MAX, v.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glVertexAttrib2fv(MAX, v.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glVertexAttrib3fv(MAX, v.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glVertexAttrib4fv(MAX, v.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
        assert!(gl.ctx().current_attrib.iter().all(|a| *a == [0.0, 0.0, 0.0, 1.0]));
    }

    #[test]
    fn vertex_attrib_fv_null_is_ignored() {
        let gl = TestGl::new(8, 8);
        // SAFETY: null is tolerated.
        unsafe {
            glVertexAttrib1fv(0, core::ptr::null());
            glVertexAttrib2fv(0, core::ptr::null());
            glVertexAttrib3fv(0, core::ptr::null());
            glVertexAttrib4fv(0, core::ptr::null());
        }
        assert_eq!(current(0), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn current_value_is_independent_of_the_array_state() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glVertexAttrib4f(6, 1.0, 2.0, 3.0, 4.0);
            glVertexAttribPointer(6, 2, GL_FLOAT, 0, 0, core::ptr::null());
            glEnableVertexAttribArray(6);
        }
        assert_eq!(current(6), [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(geti(6, GL_VERTEX_ATTRIB_ARRAY_SIZE), 2);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn current_value_as_integers() {
        let gl = TestGl::new(8, 8);
        let mut v = [-99; 4];
        // SAFETY: room for four values.
        unsafe {
            glVertexAttrib4f(0, 1.0, -2.0, 300.0, 0.0);
            glGetVertexAttribiv(0, GL_CURRENT_VERTEX_ATTRIB, v.as_mut_ptr());
        }
        assert_eq!(v, [1, -2, 300, 0]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* ---- query errors -------------------------------------------------------- */

    #[test]
    fn get_vertex_attrib_errors_write_nothing() {
        let gl = TestGl::new(8, 8);
        assert_eq!(geti(MAX, GL_VERTEX_ATTRIB_ARRAY_SIZE), -99);
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        assert_eq!(getf(MAX, GL_CURRENT_VERTEX_ATTRIB), -99.0);
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        assert_eq!(geti(0, GL_VERTEX_ATTRIB_ARRAY_POINTER), -99, "the pointer has its own query");
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        assert_eq!(getf(0, GL_VIEWPORT), -99.0);
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        // SAFETY: null is tolerated.
        unsafe {
            glGetVertexAttribiv(0, GL_VERTEX_ATTRIB_ARRAY_SIZE, core::ptr::null_mut());
            glGetVertexAttribfv(0, GL_CURRENT_VERTEX_ATTRIB, core::ptr::null_mut());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn get_vertex_attrib_pointerv_errors() {
        let gl = TestGl::new(8, 8);
        let mut p = 0xDEAD as *mut c_void;
        // SAFETY: room for one pointer; null is tolerated.
        unsafe {
            glGetVertexAttribPointerv(MAX, GL_VERTEX_ATTRIB_ARRAY_POINTER, &mut p);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glGetVertexAttribPointerv(0, GL_VERTEX_ATTRIB_ARRAY_SIZE, &mut p);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glGetVertexAttribPointerv(0, GL_VERTEX_ATTRIB_ARRAY_POINTER, core::ptr::null_mut());
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
        assert_eq!(p as usize, 0xDEAD);
    }

    #[test]
    fn float_queries_of_integer_state_convert() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glVertexAttribPointer(0, 3, GL_SHORT, 1, 10, core::ptr::null()) };
        assert_eq!(getf(0, GL_VERTEX_ATTRIB_ARRAY_SIZE), 3.0);
        assert_eq!(getf(0, GL_VERTEX_ATTRIB_ARRAY_TYPE), GL_SHORT as GLfloat);
        assert_eq!(getf(0, GL_VERTEX_ATTRIB_ARRAY_STRIDE), 10.0);
        assert_eq!(getf(0, GL_VERTEX_ATTRIB_ARRAY_NORMALIZED), 1.0);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }
}
