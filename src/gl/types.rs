// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Khronos scalar types (`KHR/khrplatform.h`, `GLES2/gl2.h`, `EGL/egl.h`).
#![allow(non_camel_case_types, dead_code)]

use core::ffi::c_void;

pub type GLvoid = c_void;
pub type GLchar = core::ffi::c_char;
pub type GLenum = u32;
pub type GLboolean = u8;
pub type GLbitfield = u32;
pub type GLbyte = i8;
pub type GLubyte = u8;
pub type GLshort = i16;
pub type GLushort = u16;
pub type GLint = i32;
pub type GLuint = u32;
pub type GLsizei = i32;
pub type GLfloat = f32;
pub type GLclampf = f32;
pub type GLfixed = i32;
pub type GLhalf = u16;
pub type GLintptr = isize;
pub type GLsizeiptr = isize;
pub type GLint64 = i64;
pub type GLuint64 = u64;
pub type GLsync = *mut c_void;

pub const GL_FALSE_B: GLboolean = 0;
pub const GL_TRUE_B: GLboolean = 1;

pub type EGLint = i32;
pub type EGLBoolean = u32;
pub type EGLenum = u32;
pub type EGLAttrib = isize;
pub type EGLTime = u64;
pub type EGLDisplay = *mut c_void;
pub type EGLConfig = *mut c_void;
pub type EGLSurface = *mut c_void;
pub type EGLContext = *mut c_void;
pub type EGLClientBuffer = *mut c_void;
pub type EGLSync = *mut c_void;
pub type EGLImage = *mut c_void;
pub type EGLNativeDisplayType = *mut c_void;
pub type EGLNativePixmapType = *mut c_void;
