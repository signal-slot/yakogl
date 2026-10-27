// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The EGL 1.4 and OpenGL ES 2.0 entry points this example calls, and their
//! constants: the C API as include/EGL/egl.h, include/EGL/eglplatform.h and
//! include/GLES2/gl2.h declare it. YakoGL exports these symbols; the image
//! links them like any C program's.

use core::ffi::{c_char, c_void};

pub type EGLBoolean = u32;
pub type EGLint = i32;
pub type EGLDisplay = *mut c_void;
pub type EGLConfig = *mut c_void;
pub type EGLSurface = *mut c_void;
pub type EGLContext = *mut c_void;

/// `struct gles_native_window` (include/EGL/eglplatform.h): the window EGL
/// renders into.
#[repr(C)]
pub struct NativeWindow {
    pub phys: u64,
    pub width: u32,
    pub height: u32,
    pub stride_bytes: u32,
    /// GLES_NATIVE_FORMAT_*
    pub format: u32,
}

const _: () = assert!(core::mem::size_of::<NativeWindow>() == 24);

pub const EGL_DEFAULT_DISPLAY: *mut c_void = core::ptr::null_mut();
pub const EGL_NO_CONTEXT: EGLContext = core::ptr::null_mut();
pub const EGL_NO_SURFACE: EGLSurface = core::ptr::null_mut();
pub const EGL_ALPHA_SIZE: EGLint = 0x3021;
pub const EGL_BLUE_SIZE: EGLint = 0x3022;
pub const EGL_GREEN_SIZE: EGLint = 0x3023;
pub const EGL_RED_SIZE: EGLint = 0x3024;
pub const EGL_SURFACE_TYPE: EGLint = 0x3033;
pub const EGL_NONE: EGLint = 0x3038;
pub const EGL_RENDERABLE_TYPE: EGLint = 0x3040;
pub const EGL_SINGLE_BUFFER: EGLint = 0x3085;
pub const EGL_RENDER_BUFFER: EGLint = 0x3086;
pub const EGL_CONTEXT_CLIENT_VERSION: EGLint = 0x3098;
pub const EGL_WINDOW_BIT: EGLint = 0x0004;
pub const EGL_OPENGL_ES2_BIT: EGLint = 0x0004;

extern "C" {
    pub fn eglGetDisplay(display_id: *mut c_void) -> EGLDisplay;
    pub fn eglInitialize(dpy: EGLDisplay, major: *mut EGLint, minor: *mut EGLint) -> EGLBoolean;
    pub fn eglChooseConfig(
        dpy: EGLDisplay,
        attrib_list: *const EGLint,
        configs: *mut EGLConfig,
        config_size: EGLint,
        num_config: *mut EGLint,
    ) -> EGLBoolean;
    pub fn eglCreateWindowSurface(
        dpy: EGLDisplay,
        config: EGLConfig,
        win: *mut NativeWindow,
        attrib_list: *const EGLint,
    ) -> EGLSurface;
    pub fn eglCreateContext(
        dpy: EGLDisplay,
        config: EGLConfig,
        share_context: EGLContext,
        attrib_list: *const EGLint,
    ) -> EGLContext;
    pub fn eglMakeCurrent(dpy: EGLDisplay, draw: EGLSurface, read: EGLSurface, ctx: EGLContext) -> EGLBoolean;
    pub fn eglSwapBuffers(dpy: EGLDisplay, surface: EGLSurface) -> EGLBoolean;
    pub fn eglGetError() -> EGLint;
}

pub type GLenum = u32;
pub type GLuint = u32;
pub type GLint = i32;
pub type GLsizei = i32;
pub type GLfloat = f32;

pub const GL_NO_ERROR: GLenum = 0;
pub const GL_TRIANGLES: GLenum = 0x0004;
pub const GL_UNSIGNED_BYTE: GLenum = 0x1401;
pub const GL_FLOAT: GLenum = 0x1406;
pub const GL_RGBA: GLenum = 0x1908;
pub const GL_RENDERER: GLenum = 0x1F01;
pub const GL_VERSION: GLenum = 0x1F02;
pub const GL_COLOR_BUFFER_BIT: GLenum = 0x4000;
pub const GL_ARRAY_BUFFER: GLenum = 0x8892;
pub const GL_STATIC_DRAW: GLenum = 0x88E4;
pub const GL_FRAGMENT_SHADER: GLenum = 0x8B30;
pub const GL_VERTEX_SHADER: GLenum = 0x8B31;
pub const GL_COMPILE_STATUS: GLenum = 0x8B81;
pub const GL_LINK_STATUS: GLenum = 0x8B82;

extern "C" {
    pub fn glGetString(name: GLenum) -> *const u8;
    pub fn glGetError() -> GLenum;
    pub fn glCreateShader(kind: GLenum) -> GLuint;
    pub fn glShaderSource(shader: GLuint, count: GLsizei, string: *const *const c_char, length: *const GLint);
    pub fn glCompileShader(shader: GLuint);
    pub fn glGetShaderiv(shader: GLuint, pname: GLenum, params: *mut GLint);
    pub fn glGetShaderInfoLog(shader: GLuint, size: GLsizei, length: *mut GLsizei, log: *mut c_char);
    pub fn glCreateProgram() -> GLuint;
    pub fn glAttachShader(program: GLuint, shader: GLuint);
    pub fn glLinkProgram(program: GLuint);
    pub fn glGetProgramiv(program: GLuint, pname: GLenum, params: *mut GLint);
    pub fn glGetProgramInfoLog(program: GLuint, size: GLsizei, length: *mut GLsizei, log: *mut c_char);
    pub fn glUseProgram(program: GLuint);
    pub fn glGetAttribLocation(program: GLuint, name: *const c_char) -> GLint;
    pub fn glGenBuffers(n: GLsizei, buffers: *mut GLuint);
    pub fn glBindBuffer(target: GLenum, buffer: GLuint);
    pub fn glBufferData(target: GLenum, size: isize, data: *const c_void, usage: GLenum);
    pub fn glVertexAttribPointer(
        index: GLuint,
        size: GLint,
        kind: GLenum,
        normalized: u8,
        stride: GLsizei,
        pointer: *const c_void,
    );
    pub fn glEnableVertexAttribArray(index: GLuint);
    pub fn glViewport(x: GLint, y: GLint, width: GLsizei, height: GLsizei);
    pub fn glClearColor(r: GLfloat, g: GLfloat, b: GLfloat, a: GLfloat);
    pub fn glClear(mask: u32);
    pub fn glDrawArrays(mode: GLenum, first: GLint, count: GLsizei);
    pub fn glReadPixels(
        x: GLint,
        y: GLint,
        width: GLsizei,
        height: GLsizei,
        format: GLenum,
        kind: GLenum,
        pixels: *mut c_void,
    );
}
