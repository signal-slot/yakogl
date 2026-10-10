// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Shader objects (glCreateShader .. glCompileShader).
//!
//! glShaderSource keeps an owned copy of the concatenated source (the only
//! host heap use in the core); glCompileShader hands that text to the shader
//! back end (`shaderc::backend().compile`), which returns the compiled USC +
//! metadata ([`crate::shaderc::ShcShader`]) the program/draw layers wire up.
#![allow(non_snake_case)]

use core::ffi::{c_char, c_void};

use super::consts::*;
use super::context::{self, gl_trace, obj_index, obj_name, GlContext, GlShader, MAX_CONTEXTS, MAX_SHADERS, OBJ_SHADER};
use super::program;
use super::types::*;
use crate::osal;
use crate::shaderc::{self, ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};
use crate::util::{self, Bytes, Global};

/// Size of one info log (shader and program alike).
pub(super) const INFO_LOG_SIZE: usize = 256;

/// File-static state of this module.
struct ShaderState {
    /// Per-shader compile log (side storage; keeps `GlShader` small), per context.
    log: [[[u8; INFO_LOG_SIZE]; MAX_SHADERS]; MAX_CONTEXTS],
    /// glDeleteShader of an attached shader: flagged (DELETE_STATUS true,
    /// still a shader) until no program has it attached (GL ES 3.0 2.12.1).
    delete_pending: [[bool; MAX_SHADERS]; MAX_CONTEXTS],
    /// The 16-hex-digit SHA prefixes whose text `log_missing_source` has
    /// already spelled out, and how many of them are in use.
    seen: [[u8; SHA16]; MAX_SEEN],
    nseen: usize,
}

/// Missing-shader texts logged before the dump gives up (the C `seen`).
const MAX_SEEN: usize = 64;
/// Hex digits of the SHA-256 that identify a missing shader in the log.
const SHA16: usize = 16;
/// Bytes of source text per `shsrc` log line.
const SHSRC_CHUNK: usize = 96;

static STATE: Global<ShaderState> = Global::zeroed();

/// The shader object `shader`, or `None` after recording the GL error for a
/// name that is not one (GL_INVALID_OPERATION for a program name, else
/// GL_INVALID_VALUE).
fn shader_or_error<'a>(c: &'a mut GlContext, shader: GLuint, func: &str) -> Option<&'a mut GlShader> {
    if c.shader_ref(shader).is_none() {
        let e = program::not_a_shader(c, shader);
        c.set_error(e, func);
        return None;
    }
    c.shader(shader)
}

/// Free a shader object now.
fn destroy_shader(c: &mut GlContext, shader: GLuint) {
    let s = &mut context::objects(c.group).shaders[obj_index(shader)];
    /* The compiled shader goes back to the compiler unless a program that
     * linked it still holds it. */
    shaderc::release_shader(&s.compiled_shader);
    // SAFETY: source is null or the malloc block glShaderSource made; every
    // field of GlShader is valid when all-zero.
    unsafe {
        util::free(s.source as *mut c_void);
        core::ptr::write_bytes(s as *mut GlShader, 0, 1);
    }
    // SAFETY: single-threaded by contract; the callers hold no other reference to STATE.
    unsafe { STATE.get() }.delete_pending[c.group as usize][obj_index(shader)] = false;
}

/// A shader glDeleteShader flagged goes once no program has it attached
/// (glDetachShader, glDeleteProgram call this).
pub(super) fn release_if_orphaned(c: &mut GlContext, shader: GLuint) {
    if c.shader_ref(shader).is_none() {
        return;
    }
    // SAFETY: single-threaded by contract; the callers hold no other reference to STATE.
    let pending = unsafe { STATE.get() }.delete_pending[c.group as usize][obj_index(shader)];
    if pending && !program::shader_is_attached(c, shader) {
        destroy_shader(c, shader);
    }
}

/// The text of a NUL-terminated info log buffer.
pub(super) fn log_text(log: &[u8; INFO_LOG_SIZE]) -> &[u8] {
    let n = log.iter().position(|&b| b == 0).unwrap_or(log.len());
    &log[..n]
}

/// Hand `src` to the application the way glGet*InfoLog / glGetShaderSource
/// do: at most `buf_size - 1` bytes plus a NUL when there is a buffer;
/// `length` receives the (clamped) size even when `buf` is null.
///
/// # Safety
/// `buf` must be null or valid for `buf_size` bytes; `length` null or valid.
pub(super) unsafe fn copy_out_text(src: &[u8], buf_size: GLsizei, length: *mut GLsizei, buf: *mut GLchar) {
    let room = if buf_size > 0 { buf_size as usize - 1 } else { 0 };
    let n = src.len().min(room);
    if !buf.is_null() && buf_size > 0 {
        core::ptr::copy_nonoverlapping(src.as_ptr(), buf as *mut u8, n);
        *buf.add(n) = 0;
    }
    util::app_put(length, n as GLsizei);
}

#[no_mangle]
pub unsafe extern "C" fn glCreateShader(type_: GLenum) -> GLuint {
    gl_trace!(glCreateShader);
    let Some(c) = context::current() else {
        return 0;
    };
    if type_ != GL_VERTEX_SHADER && type_ != GL_FRAGMENT_SHADER {
        c.set_error(GL_INVALID_ENUM, "glCreateShader");
        return 0;
    }
    for h in 1..MAX_SHADERS {
        if !c.objs().shaders[h].used {
            let s = &mut c.objs().shaders[h];
            // SAFETY: every field of GlShader is valid when all-zero.
            core::ptr::write_bytes(s as *mut GlShader, 0, 1);
            s.used = true;
            s.type_ = type_;
            // SAFETY: single-threaded by contract; taken once in this entry point.
            let st = STATE.get();
            st.log[c.group as usize][h][0] = 0;
            st.delete_pending[c.group as usize][h] = false;
            return obj_name(h, OBJ_SHADER);
        }
    }
    c.set_error(GL_OUT_OF_MEMORY, "glCreateShader");
    0
}

/// Bytes of source fragment `i`: `length[i]` when `length` is given and the
/// value is non-negative, else up to the NUL.
///
/// # Safety
/// `s` must be non-null and valid for that many bytes / NUL-terminated;
/// `length` null or holding more than `i` values (any alignment).
unsafe fn fragment_len(s: *const GLchar, length: *const GLint, i: usize) -> usize {
    let l = if length.is_null() { -1 } else { util::app_get(length, i) };
    if l >= 0 {
        l as usize
    } else {
        util::cstr_len(s)
    }
}

#[no_mangle]
pub unsafe extern "C" fn glShaderSource(
    shader: GLuint,
    count: GLsizei,
    string: *const *const GLchar,
    length: *const GLint,
) {
    gl_trace!(glShaderSource);
    const FUNC: &str = "glShaderSource";
    if count < 0 {
        context::set_error_no_ctx(GL_INVALID_VALUE, FUNC);
        return;
    }
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return;
    };
    if shader_or_error(c, shader, FUNC).is_none() {
        return;
    }
    if string.is_null() {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    let count = count as usize;

    /* Total length across the source fragments. The application passes
     * `count` string pointers and, when `length` is non-null, `count`
     * lengths (GL contract), in arrays of any alignment. */
    let mut total = 0usize;
    for i in 0..count {
        let s = util::app_get(string, i);
        if s.is_null() {
            continue;
        }
        let Some(t) = total.checked_add(fragment_len(s, length, i)) else {
            c.set_error(GL_OUT_OF_MEMORY, FUNC);
            return;
        };
        total = t;
    }

    let buf = match total.checked_add(1) {
        Some(n) => util::malloc(n) as *mut u8,
        None => core::ptr::null_mut(),
    };
    if buf.is_null() {
        c.set_error(GL_OUT_OF_MEMORY, FUNC);
        return;
    }

    let mut off = 0usize;
    for i in 0..count {
        let s = util::app_get(string, i);
        if s.is_null() {
            continue;
        }
        let n = fragment_len(s, length, i);
        // SAFETY: the fragments sum up to `total`, the block holds total + 1.
        core::ptr::copy_nonoverlapping(s as *const u8, buf.add(off), n);
        off += n;
    }
    *buf.add(off) = 0;

    let s = &mut c.objs().shaders[obj_index(shader)];
    // SAFETY: source is null or the malloc block of the previous glShaderSource.
    util::free(s.source as *mut c_void);
    s.source = buf as *mut c_char;
    s.compiled = false;
}

/// The text of a shader the offline table lacks, once per text: one log line
/// per <= 96-byte chunk, `shsrc <sha16> <line>.<chunk>:<text>`, then
/// `shsrc <sha16> end <lines> <bytes>`. The harvester reassembles it by index
/// (other threads' output may interleave) and checks the SHA-256.
fn log_missing_source(st: &mut ShaderState, ctx: usize, shader: GLuint, src: &[u8], stage: u32) {
    let mut sha = [0u8; SHA16];

    if osal::ops().and_then(|o| o.log).is_none() {
        return;
    }
    {
        let log = log_text(&st.log[ctx][obj_index(shader)]);
        /* the compile log names the text: "... (sha256=<64 hex>) ..." */
        let Some(h) = log.windows(7).position(|w| w == b"sha256=") else {
            return;
        };
        let Some(s) = log.get(h + 7..h + 7 + SHA16) else {
            return;
        };
        sha.copy_from_slice(s);
        glog!(
            "glCompileShader({}): {} [{} stage, {} bytes]",
            shader,
            Bytes(log),
            if stage == SHC_STAGE_VERTEX { "vertex" } else { "fragment" },
            src.len() as u32
        );
    }
    if st.seen[..st.nseen].contains(&sha) {
        return; /* this text has been spelled out already */
    }
    if st.nseen < MAX_SEEN {
        st.seen[st.nseen] = sha;
        st.nseen += 1;
    }

    let sha16 = Bytes(&sha);
    let mut p = src;
    let mut line = 0u32;

    while !p.is_empty() {
        let nl = p.iter().position(|&b| b == b'\n');
        let mut len = nl.unwrap_or(p.len());
        let mut chunk = 0u32;

        loop {
            let n = len.min(SHSRC_CHUNK);

            glog!("shsrc {} {}.{}:{}", sha16, line, chunk, Bytes(&p[..n]));
            chunk += 1;
            p = &p[n..];
            len -= n;
            if len == 0 {
                break;
            }
        }
        line += 1;
        match nl {
            Some(_) => p = &p[1..], /* past the newline */
            None => break,
        }
    }
    glog!("shsrc {} end {} {}", sha16, line, src.len() as u32);
}

#[no_mangle]
pub unsafe extern "C" fn glCompileShader(shader: GLuint) {
    gl_trace!(glCompileShader);
    const FUNC: &str = "glCompileShader";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return;
    };
    if shader_or_error(c, shader, FUNC).is_none() {
        return;
    }
    // SAFETY: single-threaded by contract; taken once in this entry point.
    let st = STATE.get();
    let log = &mut st.log[c.group as usize][obj_index(shader)];

    c.objs().shaders[obj_index(shader)].compiled = false;
    log[0] = 0;

    let Some(compile) = shaderc::backend().and_then(|b| b.compile) else {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    };
    let s = &mut context::objects(c.group).shaders[obj_index(shader)];
    if s.source.is_null() {
        return; /* nothing to compile; COMPILE_STATUS stays false */
    }

    let stage = if s.type_ == GL_VERTEX_SHADER { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
    /* The previous compile's result is this object's to give back; a
     * program that linked it holds its own reference. */
    shaderc::release_shader(&s.compiled_shader);
    // SAFETY: every field of ShcShader is valid when all-zero.
    core::ptr::write_bytes(&mut s.compiled_shader as *mut ShcShader, 0, 1);
    // SAFETY: source is the NUL-terminated copy glShaderSource made; the log
    // buffer is INFO_LOG_SIZE bytes.
    let rc = compile(stage, s.source, &mut s.compiled_shader, log.as_mut_ptr() as *mut c_char, INFO_LOG_SIZE);
    s.compiled = rc == 0;
    s.missing = false;
    // SAFETY: source is the NUL-terminated copy glShaderSource made.
    let src = util::cstr_bytes(s.source);
    let ctx = c.group as usize;
    let table_lacks_it =
        !s.compiled && log_text(&st.log[ctx][obj_index(shader)]).windows(20).any(|w| w == b"not in offline table");
    if table_lacks_it {
        /* A run-time generated shader the table lacks: log its text for
         * zephyr-module/tools/harvest-shaders.py and let the application
         * carry on to its other stage; glLinkProgram refuses the pair
         * (upstream 18692b4). */
        let s = &mut c.objs().shaders[obj_index(shader)];
        s.compiled = true;
        s.missing = true;
        log_missing_source(st, ctx, shader, src, stage);
    } else if !s.compiled && osal::ops().and_then(|o| o.log).is_some() {
        glog!(
            "glCompileShader({}): {} [{} stage, {} bytes]",
            shader,
            Bytes(log_text(&st.log[ctx][obj_index(shader)])),
            if stage == SHC_STAGE_VERTEX { "vertex" } else { "fragment" },
            src.len() as u32
        );
    }
}

#[no_mangle]
pub unsafe extern "C" fn glGetShaderiv(shader: GLuint, pname: GLenum, params: *mut GLint) {
    gl_trace!(glGetShaderiv);
    const FUNC: &str = "glGetShaderiv";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return;
    };
    let (ctx, h) = (c.group as usize, obj_index(shader));
    let Some(s) = shader_or_error(c, shader, FUNC) else {
        return;
    };
    let s = &*s;
    // SAFETY: single-threaded by contract; taken once in this entry point.
    let st = STATE.get();

    let v = match pname {
        GL_SHADER_TYPE => s.type_ as GLint,
        GL_COMPILE_STATUS => (if s.compiled { GL_TRUE } else { GL_FALSE }) as GLint,
        GL_DELETE_STATUS => st.delete_pending[ctx][h] as GLint,
        GL_INFO_LOG_LENGTH => {
            let n = log_text(&st.log[ctx][h]).len();
            if n != 0 {
                (n + 1) as GLint
            } else {
                0
            }
        }
        GL_SHADER_SOURCE_LENGTH => {
            // SAFETY: source is null or the NUL-terminated copy glShaderSource made.
            if s.source.is_null() {
                0
            } else {
                (util::cstr_len(s.source) + 1) as GLint
            }
        }
        _ => {
            c.set_error(GL_INVALID_ENUM, FUNC);
            return;
        }
    };
    // SAFETY: params is null or points at one GLint (GL contract), any alignment.
    util::app_put(params, v);
}

#[no_mangle]
pub unsafe extern "C" fn glGetShaderInfoLog(
    shader: GLuint,
    bufSize: GLsizei,
    length: *mut GLsizei,
    infoLog: *mut GLchar,
) {
    gl_trace!(glGetShaderInfoLog);
    const FUNC: &str = "glGetShaderInfoLog";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return;
    };
    if shader_or_error(c, shader, FUNC).is_none() {
        return;
    }
    if bufSize < 0 {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    // SAFETY: single-threaded by contract; taken once in this entry point.
    let log = &STATE.get().log[c.group as usize][obj_index(shader)];
    // SAFETY: infoLog holds bufSize bytes, length one GLsizei, or they are null.
    copy_out_text(log_text(log), bufSize, length, infoLog);
}

#[no_mangle]
pub unsafe extern "C" fn glDeleteShader(shader: GLuint) {
    gl_trace!(glDeleteShader);
    const FUNC: &str = "glDeleteShader";
    if shader == 0 {
        return;
    }
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_VALUE, FUNC);
        return;
    };
    if shader_or_error(c, shader, FUNC).is_none() {
        return;
    }
    if program::shader_is_attached(c, shader) {
        /* GL ES 3.0 2.12.1: flagged; it goes with its last attachment
         * (glDetachShader, glDeleteProgram). Linked programs never needed it:
         * they snapshot the compiled shader. */
        // SAFETY: single-threaded by contract; taken once in this entry point.
        STATE.get().delete_pending[c.group as usize][obj_index(shader)] = true;
        return;
    }
    destroy_shader(c, shader);
}

#[no_mangle]
pub unsafe extern "C" fn glIsShader(shader: GLuint) -> GLboolean {
    gl_trace!(glIsShader);
    match context::current().and_then(|c| c.shader(shader)) {
        Some(_) => GL_TRUE_B,
        None => GL_FALSE_B,
    }
}

#[no_mangle]
pub unsafe extern "C" fn glGetShaderSource(
    shader: GLuint,
    bufSize: GLsizei,
    length: *mut GLsizei,
    source: *mut GLchar,
) {
    gl_trace!(glGetShaderSource);
    const FUNC: &str = "glGetShaderSource";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return;
    };
    let Some(s) = shader_or_error(c, shader, FUNC) else {
        return;
    };
    let src = s.source;
    if bufSize < 0 {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    // SAFETY: src is null or the NUL-terminated copy glShaderSource made;
    // source holds bufSize bytes, length one GLsizei, or they are null.
    copy_out_text(util::cstr_bytes(src), bufSize, length, source);
}

/// No binary shader formats: the offline table is keyed by source text.
#[no_mangle]
pub unsafe extern "C" fn glShaderBinary(
    _count: GLsizei,
    _shaders: *const GLuint,
    _binaryformat: GLenum,
    _binary: *const c_void,
    _length: GLsizei,
) {
    gl_trace!(glShaderBinary);
    context::set_error_no_ctx(GL_INVALID_ENUM, "glShaderBinary");
}

#[no_mangle]
pub unsafe extern "C" fn glReleaseShaderCompiler() {
    gl_trace!(glReleaseShaderCompiler);
    /* nothing to release: the offline backend keeps no compiler state */
}

#[no_mangle]
pub unsafe extern "C" fn glGetShaderPrecisionFormat(
    shadertype: GLenum,
    precisiontype: GLenum,
    range: *mut GLint,
    precision: *mut GLint,
) {
    gl_trace!(glGetShaderPrecisionFormat);
    const FUNC: &str = "glGetShaderPrecisionFormat";
    if shadertype != GL_VERTEX_SHADER && shadertype != GL_FRAGMENT_SHADER {
        context::set_error_no_ctx(GL_INVALID_ENUM, FUNC);
        return;
    }
    // SAFETY: range points at two GLints, precision at one, or they are null
    // (GL contract).
    match precisiontype {
        GL_HIGH_FLOAT if shadertype == GL_FRAGMENT_SHADER && !crate::backend::FRAGMENT_HIGHP => {
            /* ES 2.0 6.1.8: a precision the stage lacks reads as range and
             * precision 0 */
            if !range.is_null() {
                util::app_set(range, 0, 0);
                util::app_set(range, 1, 0);
            }
            if !precision.is_null() {
                util::app_set(precision, 0, 0);
            }
        }
        GL_LOW_FLOAT | GL_MEDIUM_FLOAT | GL_HIGH_FLOAT => {
            /* every float is IEEE binary32 (backend::FRAGMENT_HIGHP) */
            if !range.is_null() {
                util::app_set(range, 0, 127);
                util::app_set(range, 1, 127);
            }
            if !precision.is_null() {
                util::app_set(precision, 0, 23);
            }
        }
        GL_LOW_INT | GL_MEDIUM_INT | GL_HIGH_INT => {
            if !range.is_null() {
                util::app_set(range, 0, 31);
                util::app_set(range, 1, 30);
            }
            if !precision.is_null() {
                util::app_set(precision, 0, 0);
            }
        }
        _ => context::set_error_no_ctx(GL_INVALID_ENUM, FUNC),
    }
}

/// Unit tests: clear the info logs and the unknown-shader dump counter
/// (process-global; testutil::TestGl calls this when it is created and dropped).
#[cfg(test)]
pub(crate) fn reset_for_tests() {
    // SAFETY: tests holding a TestGl are serialised; every field is valid when all-zero.
    unsafe { core::ptr::write_bytes(STATE.as_ptr(), 0, 1) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gl::program::{
        glAttachShader, glCreateProgram, glDeleteProgram, glDetachShader, glGetProgramInfoLog, glGetProgramiv,
        glLinkProgram,
    };
    use crate::testutil::TestGl;
    use std::ffi::CString;
    use std::format;
    use std::string::String;
    use std::vec::Vec;

    /* ---- helpers ---- */

    /// glShaderSource with one NUL-terminated string.
    fn set_source(shader: GLuint, text: &str) {
        let text = CString::new(text).unwrap();
        let strings = [text.as_ptr()];
        // SAFETY: one valid NUL-terminated string, no lengths.
        unsafe { glShaderSource(shader, 1, strings.as_ptr(), core::ptr::null()) };
    }

    /// glGetShaderSource into a buffer large enough for any test source.
    fn get_source(shader: GLuint) -> String {
        let mut buf = [0x55u8; 1024];
        let mut len: GLsizei = -1;
        // SAFETY: buf holds the announced 1024 bytes.
        unsafe { glGetShaderSource(shader, buf.len() as GLsizei, &mut len, buf.as_mut_ptr() as *mut GLchar) };
        assert_eq!(buf[len as usize], 0, "NUL right after the reported length");
        String::from_utf8(buf[..len as usize].to_vec()).unwrap()
    }

    fn get_info_log(shader: GLuint) -> String {
        let mut buf = [0x55u8; INFO_LOG_SIZE + 8];
        let mut len: GLsizei = -1;
        // SAFETY: buf holds the announced size.
        unsafe { glGetShaderInfoLog(shader, buf.len() as GLsizei, &mut len, buf.as_mut_ptr() as *mut GLchar) };
        assert_eq!(buf[len as usize], 0, "NUL right after the reported length");
        String::from_utf8(buf[..len as usize].to_vec()).unwrap()
    }

    fn get_iv(shader: GLuint, pname: GLenum) -> GLint {
        let mut v: GLint = -12345;
        // SAFETY: v is one GLint.
        unsafe { glGetShaderiv(shader, pname, &mut v) };
        v
    }

    /// The text (without its NUL) of a named test shader: the offline
    /// table's row, or in a `no-shader-table` build the file it comes from
    /// ([`crate::testutil::shader_source`]).
    fn table_source(name: &str) -> String {
        crate::testutil::shader_source(name).trim_end_matches('\0').into()
    }

    #[cfg(not(feature = "no-shader-table"))]
    fn hex(d: &[u8]) -> String {
        d.iter().map(|b| format!("{b:02x}")).collect()
    }

    /* ---- pure helpers ---- */

    #[test]
    fn log_text_stops_at_the_first_nul() {
        let mut log = [0u8; INFO_LOG_SIZE];
        assert_eq!(log_text(&log), b"");
        log[..5].copy_from_slice(b"ab\0cd");
        assert_eq!(log_text(&log), b"ab");
    }

    #[test]
    fn log_text_without_nul_is_the_whole_buffer() {
        let log = [b'x'; INFO_LOG_SIZE];
        assert_eq!(log_text(&log).len(), INFO_LOG_SIZE);
    }

    #[test]
    fn copy_out_text_fits_with_nul_and_length() {
        let mut buf = [0x55u8; 8];
        let mut len: GLsizei = -1;
        // SAFETY: buf holds 8 bytes.
        unsafe { copy_out_text(b"abc", 8, &mut len, buf.as_mut_ptr() as *mut GLchar) };
        assert_eq!(len, 3);
        assert_eq!(&buf, b"abc\0\x55\x55\x55\x55");
    }

    #[test]
    fn copy_out_text_truncates_to_buf_size_minus_one() {
        let mut buf = [0x55u8; 8];
        let mut len: GLsizei = -1;
        // SAFETY: only 4 of the 8 bytes are announced.
        unsafe { copy_out_text(b"abcdefgh", 4, &mut len, buf.as_mut_ptr() as *mut GLchar) };
        assert_eq!(len, 3);
        assert_eq!(&buf, b"abc\0\x55\x55\x55\x55");
    }

    #[test]
    fn copy_out_text_buf_size_one_writes_only_the_nul() {
        let mut buf = [0x55u8; 4];
        let mut len: GLsizei = -1;
        // SAFETY: one byte announced.
        unsafe { copy_out_text(b"abc", 1, &mut len, buf.as_mut_ptr() as *mut GLchar) };
        assert_eq!(len, 0);
        assert_eq!(&buf, b"\0\x55\x55\x55");
    }

    #[test]
    fn copy_out_text_buf_size_zero_touches_nothing() {
        let mut buf = [0x55u8; 4];
        let mut len: GLsizei = -1;
        // SAFETY: zero bytes announced.
        unsafe { copy_out_text(b"abc", 0, &mut len, buf.as_mut_ptr() as *mut GLchar) };
        assert_eq!(len, 0);
        assert_eq!(&buf, b"\x55\x55\x55\x55");
    }

    #[test]
    fn copy_out_text_null_buffer_still_reports_the_clamped_length() {
        let mut len: GLsizei = -1;
        // SAFETY: a null buffer is allowed; length is valid.
        unsafe { copy_out_text(b"abcdef", 4, &mut len, core::ptr::null_mut()) };
        assert_eq!(len, 3, "documented: length receives the clamped size even when buf is null");
        // SAFETY: both null is allowed.
        unsafe { copy_out_text(b"abcdef", 4, core::ptr::null_mut(), core::ptr::null_mut()) };
    }

    #[test]
    fn copy_out_text_null_length_is_accepted() {
        let mut buf = [0x55u8; 4];
        // SAFETY: buf holds 4 bytes; a null length is allowed.
        unsafe { copy_out_text(b"ab", 4, core::ptr::null_mut(), buf.as_mut_ptr() as *mut GLchar) };
        assert_eq!(&buf, b"ab\0\x55");
    }

    #[test]
    fn fragment_len_uses_explicit_length_or_the_nul() {
        let s = c"hello".as_ptr();
        let lengths = [3, -1, 0, -7];
        // SAFETY: s is NUL-terminated and longer than every explicit length.
        unsafe {
            assert_eq!(fragment_len(s, core::ptr::null(), 0), 5);
            assert_eq!(fragment_len(s, lengths.as_ptr(), 0), 3);
            assert_eq!(fragment_len(s, lengths.as_ptr(), 1), 5, "negative: NUL-terminated");
            assert_eq!(fragment_len(s, lengths.as_ptr(), 2), 0);
            assert_eq!(fragment_len(s, lengths.as_ptr(), 3), 5, "any negative value");
        }
    }

    /* ---- glCreateShader / glIsShader / glDeleteShader ---- */

    #[test]
    fn create_shader_returns_distinct_nonzero_handles_of_the_requested_type() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        let (v, f) = unsafe { (glCreateShader(GL_VERTEX_SHADER), glCreateShader(GL_FRAGMENT_SHADER)) };
        assert!(v != 0 && f != 0 && v != f);
        assert_eq!(get_iv(v, GL_SHADER_TYPE), GL_VERTEX_SHADER as GLint);
        assert_eq!(get_iv(f, GL_SHADER_TYPE), GL_FRAGMENT_SHADER as GLint);
        // SAFETY: plain GL calls.
        unsafe {
            assert_eq!(glIsShader(v), GL_TRUE_B);
            assert_eq!(glIsShader(f), GL_TRUE_B);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn create_shader_rejects_other_types() {
        let gl = TestGl::new(8, 8);
        for t in [0, GL_FLOAT, GL_FRAGMENT_SHADER + 2, GL_TEXTURE_2D] {
            // SAFETY: plain GL call.
            assert_eq!(unsafe { glCreateShader(t) }, 0);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
        }
    }

    #[test]
    fn new_shader_starts_uncompiled_with_no_source_and_no_log() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL call.
        let s = unsafe { glCreateShader(GL_VERTEX_SHADER) };
        assert_eq!(get_iv(s, GL_COMPILE_STATUS), 0);
        assert_eq!(get_iv(s, GL_DELETE_STATUS), 0);
        assert_eq!(get_iv(s, GL_INFO_LOG_LENGTH), 0);
        assert_eq!(get_iv(s, GL_SHADER_SOURCE_LENGTH), 0);
        assert_eq!(get_source(s), "");
        assert_eq!(get_info_log(s), "");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn create_shader_out_of_handles_is_out_of_memory() {
        let gl = TestGl::new(8, 8);
        let mut handles = Vec::new();
        // SAFETY: plain GL calls.
        unsafe {
            for _ in 1..MAX_SHADERS {
                let s = glCreateShader(GL_FRAGMENT_SHADER);
                assert_ne!(s, 0);
                handles.push(s);
            }
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!(glCreateShader(GL_FRAGMENT_SHADER), 0);
            assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
            /* a deleted handle is available again */
            glDeleteShader(handles[10]);
            assert_eq!(glCreateShader(GL_VERTEX_SHADER), handles[10]);
        }
        assert_eq!(get_iv(handles[10], GL_SHADER_TYPE), GL_VERTEX_SHADER as GLint);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn create_shader_without_a_current_context_returns_zero() {
        let gl = TestGl::new(8, 8);
        // SAFETY: the context is made current again before any other GL call.
        unsafe {
            context::gl_context_set_current(core::ptr::null_mut());
            let s = glCreateShader(GL_VERTEX_SHADER);
            let is = glIsShader(1);
            context::gl_context_set_current(gl.ctx());
            assert_eq!(s, 0);
            assert_eq!(is, GL_FALSE_B);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn is_shader_is_false_for_zero_unused_and_out_of_range_names() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            let s = glCreateShader(GL_VERTEX_SHADER);
            assert_eq!(glIsShader(0), GL_FALSE_B);
            assert_eq!(glIsShader(s + 1), GL_FALSE_B);
            assert_eq!(glIsShader(MAX_SHADERS as GLuint), GL_FALSE_B);
            assert_eq!(glIsShader(GLuint::MAX), GL_FALSE_B);
        }
        assert_eq!(gl.error(), GL_NO_ERROR, "glIsShader never raises an error");
    }

    #[test]
    fn delete_shader_zero_is_silently_ignored() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL call.
        unsafe { glDeleteShader(0) };
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn delete_shader_unknown_name_is_invalid_value() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glDeleteShader(5);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glDeleteShader(GLuint::MAX);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            let s = glCreateShader(GL_VERTEX_SHADER);
            glDeleteShader(s);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glDeleteShader(s);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "second delete of the same name");
        }
    }

    #[test]
    fn deleted_shader_is_gone_and_its_slot_is_clean_when_reused() {
        let gl = TestGl::new(8, 8);
        let s = gl.shader(GL_VERTEX_SHADER, "passthrough.vert");
        // SAFETY: plain GL calls.
        unsafe {
            glDeleteShader(s);
            assert_eq!(glIsShader(s), GL_FALSE_B);
            assert!(gl.ctx().objs().shaders[obj_index(s)].source.is_null());
            let again = glCreateShader(GL_FRAGMENT_SHADER);
            assert_eq!(again, s, "lowest free handle is reused");
        }
        assert_eq!(get_iv(s, GL_COMPILE_STATUS), 0, "the previous shader's compile status does not leak");
        assert_eq!(get_iv(s, GL_SHADER_SOURCE_LENGTH), 0);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn create_shader_clears_the_previous_users_info_log() {
        let _gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            let s = glCreateShader(GL_FRAGMENT_SHADER);
            /* not GLSL, so every back end leaves a log: the offline one
             * "not in offline table", the online one a syntax error */
            set_source(s, "void main() { not glsl: reuse }");
            glCompileShader(s);
            assert!(get_iv(s, GL_INFO_LOG_LENGTH) > 0);
            glDeleteShader(s);
            assert_eq!(glCreateShader(GL_FRAGMENT_SHADER), s);
            assert_eq!(get_iv(s, GL_INFO_LOG_LENGTH), 0);
            assert_eq!(get_info_log(s), "");
        }
    }

    #[test]
    fn deleting_an_attached_shader_is_deferred_until_it_is_detached() {
        /* GL ES 3.0 2.12.1: an attached shader is only flagged (DELETE_STATUS true, still a shader); it goes with
         * its last attachment. The linked program keeps working either way (it snapshot the compiled shaders). */
        let gl = TestGl::new(8, 8);
        let v = gl.shader(GL_VERTEX_SHADER, "passthrough.vert");
        let f = gl.shader(GL_FRAGMENT_SHADER, "uniform_color.frag");
        // SAFETY: plain GL calls.
        unsafe {
            let p = glCreateProgram();
            let q = glCreateProgram();
            glAttachShader(p, v);
            glAttachShader(p, f);
            glAttachShader(q, v);
            glLinkProgram(p);
            glDeleteShader(v);
            glDeleteShader(f);
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!((glIsShader(v), glIsShader(f)), (GL_TRUE_B, GL_TRUE_B));
            assert_eq!((get_iv(v, GL_DELETE_STATUS), get_iv(f, GL_DELETE_STATUS)), (1, 1));
            let mut linked: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut linked);
            assert_eq!(linked, 1, "the program snapshot survives its shaders");

            glDetachShader(p, f);
            assert_eq!(glIsShader(f), GL_FALSE_B, "f had one attachment");
            glDetachShader(p, v);
            assert_eq!(glIsShader(v), GL_TRUE_B, "v is still attached to q");
            glDeleteProgram(q);
            assert_eq!(glIsShader(v), GL_FALSE_B, "deleting q detached v");
            assert_eq!(glCreateShader(GL_VERTEX_SHADER), v, "the slot is free and clean again");
            assert_eq!(get_iv(v, GL_DELETE_STATUS), 0);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn deleting_a_detached_shader_is_immediate() {
        let gl = TestGl::new(8, 8);
        let v = gl.shader(GL_VERTEX_SHADER, "passthrough.vert");
        // SAFETY: plain GL calls.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, v);
            glDetachShader(p, v);
            glDeleteShader(v);
            assert_eq!(glIsShader(v), GL_FALSE_B);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn shader_source_reads_misaligned_pointer_and_length_arrays() {
        let gl = TestGl::new(8, 8);
        let strings = [c"abcdef".as_ptr(), c"XYZ".as_ptr()];
        let lengths: [GLint; 2] = [2, -1];
        /* copies at an odd offset: neither array is aligned for its type */
        let mut sbuf = [0u8; 1 + 2 * core::mem::size_of::<*const GLchar>()];
        let mut lbuf = [0u8; 1 + 8];
        // SAFETY: the byte buffers hold the two elements each at offset 1.
        unsafe {
            core::ptr::copy_nonoverlapping(
                strings.as_ptr() as *const u8,
                sbuf.as_mut_ptr().add(1),
                2 * core::mem::size_of::<*const GLchar>(),
            );
            core::ptr::copy_nonoverlapping(lengths.as_ptr() as *const u8, lbuf.as_mut_ptr().add(1), 8);
            let s = glCreateShader(GL_VERTEX_SHADER);
            glShaderSource(s, 2, sbuf.as_ptr().add(1) as *const *const GLchar, lbuf.as_ptr().add(1) as *const GLint);
            assert_eq!(get_source(s), "abXYZ");
            let mut iv = [0u8; 5];
            glGetShaderiv(s, GL_SHADER_SOURCE_LENGTH, iv.as_mut_ptr().add(1) as *mut GLint);
            assert_eq!(GLint::from_ne_bytes(iv[1..].try_into().unwrap()), 6);
            let mut rp = [0u8; 13];
            glGetShaderPrecisionFormat(
                GL_VERTEX_SHADER,
                GL_HIGH_INT,
                rp.as_mut_ptr().add(1) as *mut GLint,
                rp.as_mut_ptr().add(9) as *mut GLint,
            );
            assert_eq!(GLint::from_ne_bytes(rp[1..5].try_into().unwrap()), 31);
            assert_eq!(GLint::from_ne_bytes(rp[5..9].try_into().unwrap()), 30);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn a_program_name_passed_as_a_shader_is_invalid_operation() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            let p = glCreateProgram();
            glCompileShader(p);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glDeleteShader(p);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glCompileShader(p + 50);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
    }

    /* ---- glShaderSource / glGetShaderSource ---- */

    #[test]
    fn shader_source_concatenates_nul_terminated_strings() {
        let gl = TestGl::new(8, 8);
        let strings = [c"#version 100\n".as_ptr(), c"".as_ptr(), c"void main() {}".as_ptr()];
        // SAFETY: three NUL-terminated strings, no lengths.
        let s = unsafe {
            let s = glCreateShader(GL_VERTEX_SHADER);
            glShaderSource(s, 3, strings.as_ptr(), core::ptr::null());
            s
        };
        assert_eq!(get_source(s), "#version 100\nvoid main() {}");
        assert_eq!(get_iv(s, GL_SHADER_SOURCE_LENGTH), 27 + 1, "length includes the NUL");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn shader_source_explicit_lengths_take_prefixes() {
        let gl = TestGl::new(8, 8);
        let strings = [c"abcdef".as_ptr(), c"ghijkl".as_ptr(), c"mnopqr".as_ptr()];
        let lengths: [GLint; 3] = [2, 0, 6];
        // SAFETY: every length is within its string.
        let s = unsafe {
            let s = glCreateShader(GL_VERTEX_SHADER);
            glShaderSource(s, 3, strings.as_ptr(), lengths.as_ptr());
            s
        };
        assert_eq!(get_source(s), "abmnopqr");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn shader_source_explicit_length_needs_no_nul_terminator() {
        let _gl = TestGl::new(8, 8);
        let raw: [u8; 4] = *b"wxyz"; /* no NUL anywhere */
        let strings = [raw.as_ptr() as *const GLchar];
        let lengths: [GLint; 1] = [4];
        // SAFETY: the length covers exactly the array.
        let s = unsafe {
            let s = glCreateShader(GL_VERTEX_SHADER);
            glShaderSource(s, 1, strings.as_ptr(), lengths.as_ptr());
            s
        };
        assert_eq!(get_source(s), "wxyz");
    }

    #[test]
    fn shader_source_negative_length_means_nul_terminated() {
        let gl = TestGl::new(8, 8);
        let strings = [c"abc".as_ptr(), c"defg".as_ptr(), c"hi".as_ptr()];
        let lengths: [GLint; 3] = [-1, 2, GLint::MIN];
        // SAFETY: strings are NUL-terminated; the explicit length is within its string.
        let s = unsafe {
            let s = glCreateShader(GL_FRAGMENT_SHADER);
            glShaderSource(s, 3, strings.as_ptr(), lengths.as_ptr());
            s
        };
        assert_eq!(get_source(s), "abcdehi");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn shader_source_skips_null_string_entries() {
        let _gl = TestGl::new(8, 8);
        let strings = [c"ab".as_ptr(), core::ptr::null(), c"cd".as_ptr()];
        let lengths: [GLint; 3] = [-1, 100, 1];
        // SAFETY: the null entry must not be read, whatever its length says.
        let s = unsafe {
            let s = glCreateShader(GL_FRAGMENT_SHADER);
            glShaderSource(s, 3, strings.as_ptr(), lengths.as_ptr());
            s
        };
        assert_eq!(get_source(s), "abc");
    }

    #[test]
    fn shader_source_replaces_the_previous_source() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL call.
        let s = unsafe { glCreateShader(GL_VERTEX_SHADER) };
        set_source(s, "first, rather long source text");
        set_source(s, "second");
        assert_eq!(get_source(s), "second");
        assert_eq!(get_iv(s, GL_SHADER_SOURCE_LENGTH), 7);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn shader_source_count_zero_gives_an_empty_source() {
        let gl = TestGl::new(8, 8);
        let strings = [c"unused".as_ptr()];
        // SAFETY: count 0: nothing is read.
        let s = unsafe {
            let s = glCreateShader(GL_VERTEX_SHADER);
            set_source(s, "old");
            glShaderSource(s, 0, strings.as_ptr(), core::ptr::null());
            s
        };
        assert_eq!(get_source(s), "");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn shader_source_negative_count_is_invalid_value_and_keeps_the_source() {
        let gl = TestGl::new(8, 8);
        let strings = [c"new".as_ptr()];
        // SAFETY: a negative count must not read anything.
        let s = unsafe {
            let s = glCreateShader(GL_VERTEX_SHADER);
            set_source(s, "old");
            glShaderSource(s, -1, strings.as_ptr(), core::ptr::null());
            s
        };
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        assert_eq!(get_source(s), "old");
    }

    #[test]
    fn shader_source_null_string_array_is_invalid_value() {
        let gl = TestGl::new(8, 8);
        // SAFETY: a null array must not be read.
        let s = unsafe {
            let s = glCreateShader(GL_VERTEX_SHADER);
            set_source(s, "old");
            glShaderSource(s, 1, core::ptr::null(), core::ptr::null());
            s
        };
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        assert_eq!(get_source(s), "old");
    }

    #[test]
    fn get_shader_source_truncates_and_reports_the_written_length() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls; buf holds the announced sizes.
        unsafe {
            let s = glCreateShader(GL_VERTEX_SHADER);
            set_source(s, "0123456789");
            let mut buf = [0x55u8; 16];
            let mut len: GLsizei = -1;
            glGetShaderSource(s, 5, &mut len, buf.as_mut_ptr() as *mut GLchar);
            assert_eq!(len, 4);
            assert_eq!(&buf[..6], b"0123\0\x55");

            /* exactly fitting: 10 chars + NUL */
            let mut buf = [0x55u8; 16];
            glGetShaderSource(s, 11, &mut len, buf.as_mut_ptr() as *mut GLchar);
            assert_eq!(len, 10);
            assert_eq!(&buf[..12], b"0123456789\0\x55");

            /* a null length pointer is fine */
            let mut buf = [0x55u8; 16];
            glGetShaderSource(s, 16, core::ptr::null_mut(), buf.as_mut_ptr() as *mut GLchar);
            assert_eq!(&buf[..11], b"0123456789\0");
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn get_shader_source_buf_size_zero_writes_nothing() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls; nothing may be written with bufSize 0.
        unsafe {
            let s = glCreateShader(GL_VERTEX_SHADER);
            set_source(s, "0123456789");
            let mut buf = [0x55u8; 4];
            let mut len: GLsizei = -1;
            glGetShaderSource(s, 0, &mut len, buf.as_mut_ptr() as *mut GLchar);
            assert_eq!(len, 0);
            assert_eq!(buf, [0x55; 4]);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn get_shader_source_negative_buf_size_is_invalid_value() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls; nothing may be written.
        unsafe {
            let s = glCreateShader(GL_VERTEX_SHADER);
            set_source(s, "0123456789");
            let mut buf = [0x55u8; 4];
            let mut len: GLsizei = -1;
            glGetShaderSource(s, -1, &mut len, buf.as_mut_ptr() as *mut GLchar);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            assert_eq!(len, -1);
            assert_eq!(buf, [0x55; 4]);
        }
    }

    /* ---- invalid names ---- */

    #[test]
    fn shader_entry_points_reject_names_that_are_not_shaders() {
        let gl = TestGl::new(8, 8);
        let strings = [c"x".as_ptr()];
        let mut iv: GLint = -7;
        let mut len: GLsizei = -7;
        let mut buf = [0x55u8; 8];
        // SAFETY: plain GL calls; the outputs must stay untouched.
        unsafe {
            for bad in [0, 9, MAX_SHADERS as GLuint, GLuint::MAX] {
                glShaderSource(bad, 1, strings.as_ptr(), core::ptr::null());
                assert_ne!(gl.error(), GL_NO_ERROR, "glShaderSource({bad})");
                glCompileShader(bad);
                assert_ne!(gl.error(), GL_NO_ERROR, "glCompileShader({bad})");
                glGetShaderiv(bad, GL_COMPILE_STATUS, &mut iv);
                assert_ne!(gl.error(), GL_NO_ERROR, "glGetShaderiv({bad})");
                glGetShaderInfoLog(bad, 8, &mut len, buf.as_mut_ptr() as *mut GLchar);
                assert_ne!(gl.error(), GL_NO_ERROR, "glGetShaderInfoLog({bad})");
                glGetShaderSource(bad, 8, &mut len, buf.as_mut_ptr() as *mut GLchar);
                assert_ne!(gl.error(), GL_NO_ERROR, "glGetShaderSource({bad})");
            }
        }
        assert_eq!((iv, len, buf), (-7, -7, [0x55; 8]));
    }

    #[test]
    fn shader_entry_points_raise_invalid_value_for_unknown_names() {
        let gl = TestGl::new(8, 8);
        let strings = [c"x".as_ptr()];
        let mut iv: GLint = 0;
        let mut buf = [0u8; 8];
        // SAFETY: plain GL calls.
        unsafe {
            glShaderSource(9, 1, strings.as_ptr(), core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glCompileShader(9);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glGetShaderiv(9, GL_COMPILE_STATUS, &mut iv);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glGetShaderInfoLog(9, 8, core::ptr::null_mut(), buf.as_mut_ptr() as *mut GLchar);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glGetShaderSource(9, 8, core::ptr::null_mut(), buf.as_mut_ptr() as *mut GLchar);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
    }

    /* ---- glCompileShader ---- */

    #[test]
    fn compile_of_a_table_source_succeeds_with_an_empty_log() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        let s = unsafe {
            let s = glCreateShader(GL_VERTEX_SHADER);
            set_source(s, &table_source("passthrough.vert"));
            glCompileShader(s);
            s
        };
        assert_eq!(get_iv(s, GL_COMPILE_STATUS), 1);
        assert_eq!(get_iv(s, GL_INFO_LOG_LENGTH), 0);
        assert_eq!(get_info_log(s), "");
        let c = gl.ctx();
        let sh = &c.objs_ref().shaders[obj_index(s)].compiled_shader;
        if cfg!(feature = "gpu-v3d") {
            /* the V3D compiles a program, not a stage: a compiled stage is
             * reflection only and glLinkProgram makes the code
             * (src/shaderc/online/v3d_abi.rs, `stage_shader`) */
            assert!(sh.usc.is_null() && sh.usc_dw == 0, "a V3D stage carries no code: {}", sh.usc_dw);
        } else {
            assert!(!sh.usc.is_null() && sh.usc_dw > 0);
        }
        assert_eq!(sh.num_attribs, 2, "passthrough.vert: a_pos, a_color");
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(gl.logs().is_empty(), "nothing is logged for a successful compile");
    }

    #[test]
    fn compile_matches_the_concatenation_of_split_sources() {
        let gl = TestGl::new(8, 8);
        let text = table_source("uniform_color.frag");
        let (a, b) = text.split_at(text.len() / 2);
        let (a, b) = (CString::new(a).unwrap(), CString::new(b).unwrap());
        let strings = [a.as_ptr(), b.as_ptr()];
        // SAFETY: two NUL-terminated strings.
        let s = unsafe {
            let s = glCreateShader(GL_FRAGMENT_SHADER);
            glShaderSource(s, 2, strings.as_ptr(), core::ptr::null());
            glCompileShader(s);
            s
        };
        assert_eq!(get_iv(s, GL_COMPILE_STATUS), 1);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    /* The offline table's own contract: a text the table lacks compiles
     * (COMPILE_STATUS true, `missing` set, "not in offline table" logged) and
     * glLinkProgram refuses it. A `no-shader-table` build has no offline back
     * end, so this runs only with a table; the online compiler refuses such
     * a text at glCompileShader instead, and the `..._online` twin next to
     * `an_invalid_source_is_refused_at_compile_time_by_the_online_compiler`
     * pins that. */
    #[cfg(not(feature = "no-shader-table"))]
    fn compile_of_an_unknown_source_keeps_the_log_and_raises_no_gl_error() {
        let gl = TestGl::new(8, 8);
        let src = "void main() { gl_Position = vec4(0.125); }";
        // SAFETY: plain GL calls.
        let s = unsafe {
            let s = glCreateShader(GL_VERTEX_SHADER);
            set_source(s, src);
            glCompileShader(s);
            s
        };
        assert_eq!(gl.error(), GL_NO_ERROR, "a compile failure is not a GL error");
        /* upstream 18692b4: a text the table lacks reports COMPILE_STATUS
         * true (the application compiles its other stage too) and is refused
         * by glLinkProgram; the info log still says what is missing */
        assert_eq!(get_iv(s, GL_COMPILE_STATUS), 1);
        let log = get_info_log(s);
        let digest = hex(&crate::shaderc::sha256::sha256(src.as_bytes()));
        assert!(log.contains("vertex shader not in offline table"), "{log}");
        assert!(log.contains(&digest), "the log names the digest to add to the table: {log}");
        assert!(log.contains("glsl2usc") && log.contains("blobs.rs"), "and the generator and its output: {log}");
        assert!(log.len() < INFO_LOG_SIZE, "the whole message fits an info log");
        assert_eq!(get_iv(s, GL_INFO_LOG_LENGTH), log.len() as GLint + 1, "length includes the NUL");
    }

    #[test]
    /* The offline table's own contract: a text the table lacks compiles
     * (COMPILE_STATUS true, `missing` set, "not in offline table" logged) and
     * glLinkProgram refuses it. A `no-shader-table` build has no offline back
     * end, so this runs only with a table; the online compiler refuses such
     * a text at glCompileShader instead, and the `..._online` twin next to
     * `an_invalid_source_is_refused_at_compile_time_by_the_online_compiler`
     * pins that. */
    #[cfg(not(feature = "no-shader-table"))]
    fn compile_looks_the_source_up_for_the_shaders_own_stage() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        let s = unsafe {
            let s = glCreateShader(GL_FRAGMENT_SHADER);
            set_source(s, &table_source("passthrough.vert"));
            glCompileShader(s);
            s
        };
        /* the table is looked up for the fragment stage, so the vertex text
         * is not found (COMPILE_STATUS true is upstream 18692b4's "missing") */
        assert!(get_info_log(s).contains("fragment shader not in offline table"));
        assert!(gl.ctx().objs().shaders[obj_index(s)].missing, "a vertex source is not a fragment shader");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn compile_without_source_leaves_status_false_without_error() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        let s = unsafe {
            let s = glCreateShader(GL_VERTEX_SHADER);
            glCompileShader(s);
            s
        };
        assert_eq!(get_iv(s, GL_COMPILE_STATUS), 0);
        assert_eq!(get_iv(s, GL_INFO_LOG_LENGTH), 0);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    /* The offline table's own contract: a text the table lacks compiles
     * (COMPILE_STATUS true, `missing` set, "not in offline table" logged) and
     * glLinkProgram refuses it. A `no-shader-table` build has no offline back
     * end, so this runs only with a table; the online compiler refuses such
     * a text at glCompileShader instead, and the `..._online` twin next to
     * `an_invalid_source_is_refused_at_compile_time_by_the_online_compiler`
     * pins that. */
    #[cfg(not(feature = "no-shader-table"))]
    fn recompile_after_fixing_the_source_clears_status_and_log() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            let s = glCreateShader(GL_FRAGMENT_SHADER);
            set_source(s, "void main() { /* unknown */ }");
            glCompileShader(s);
            assert!(gl.ctx().objs().shaders[obj_index(s)].missing);
            assert!(get_iv(s, GL_INFO_LOG_LENGTH) > 0);

            set_source(s, &table_source("uniform_color.frag"));
            glCompileShader(s);
            assert_eq!(get_iv(s, GL_COMPILE_STATUS), 1);
            assert!(!gl.ctx().objs().shaders[obj_index(s)].missing, "a table hit clears the missing flag");
            assert_eq!(get_iv(s, GL_INFO_LOG_LENGTH), 0);
            assert_eq!(get_info_log(s), "");

            /* and back: a recompile of a text the table lacks is missing again */
            set_source(s, "void main() { /* unknown again */ }");
            glCompileShader(s);
            assert!(gl.ctx().objs().shaders[obj_index(s)].missing);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn compile_without_a_shader_backend_is_invalid_operation() {
        let gl = TestGl::new(8, 8);
        // SAFETY: TestGl serialises the tests; the back end is restored before anything else runs.
        unsafe {
            let s = glCreateShader(GL_VERTEX_SHADER);
            set_source(s, &table_source("passthrough.vert"));
            let saved = shaderc::g_shaderc;
            shaderc::g_shaderc = core::ptr::null();
            glCompileShader(s);
            shaderc::g_shaderc = saved;
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            assert_eq!(get_iv(s, GL_COMPILE_STATUS), 0);
        }
    }

    /* ---- a back end that refuses a shader, as a compiler does ----
     *
     * A compiling back end (the in-process compiler) answers a shader it
     * rejects with -EINVAL and its log. That must
     * reach the application as GL_COMPILE_STATUS false with the log, which is
     * how dEQP-GLES2 reads a negative shader case -- and it must not be
     * confused with the offline table's -ENOENT "not in offline table", which
     * is a *missing* shader (status true, refused at link). These two tests
     * pin that difference with a back end of their own. */

    unsafe extern "C" fn refusing_compile(
        _stage: u32,
        _glsl: *const c_char,
        _out: *mut ShcShader,
        log: *mut c_char,
        log_sz: usize,
    ) -> i32 {
        use core::fmt::Write as _;
        // SAFETY: the caller passes a null log or one valid for log_sz bytes.
        let mut w = unsafe { crate::util::CStrWriter::new(log, log_sz) };
        let _ = write!(w, "ERROR: shader.frag:3: 'oops' : undeclared identifier");
        -crate::util::EINVAL
    }

    static REFUSING: crate::shaderc::ShcBackend = crate::shaderc::ShcBackend {
        compile: Some(refusing_compile),
        link: None,
        blend_variant: None,
        compile_vs_linked: None,
        lod_sensitivity: None,
        sampler_variant: None,
    };

    /// Compile `text` with a back end that refuses it; returns the status and
    /// the info log.
    fn compile_with_refusing_backend(gl: &TestGl, text: &str) -> (GLint, String, bool) {
        // SAFETY: TestGl serialises the tests; the back end is restored right after.
        unsafe {
            let s = glCreateShader(GL_FRAGMENT_SHADER);
            set_source(s, text);
            let saved = crate::shaderc::g_shaderc;
            crate::shaderc::g_shaderc = &REFUSING;
            glCompileShader(s);
            crate::shaderc::g_shaderc = saved;
            (get_iv(s, GL_COMPILE_STATUS), get_info_log(s), gl.ctx().objs().shaders[obj_index(s)].missing)
        }
    }

    #[test]
    fn a_shader_the_compiler_refuses_is_status_false_with_the_compilers_own_log() {
        let gl = TestGl::new(8, 8);
        let (status, log, missing) = compile_with_refusing_backend(&gl, "void main() { gl_FragColor = vec4(oops); }");
        assert_eq!(status, GL_FALSE as GLint, "a refused shader does not compile");
        assert!(log.contains("undeclared identifier"), "the compiler's log reaches the application: {log}");
        assert!(!missing, "a refused shader is not a shader the table lacks");
        assert_eq!(gl.error(), GL_NO_ERROR, "a compile failure is not a GL error");
    }

    #[test]
    fn a_later_compile_clears_the_refusal() {
        /* held, not read: TestGl is what serialises the tests that swap the
         * back end, and it resets the GL state when it drops */
        let _gl = TestGl::new(8, 8);
        /* the same shader object, recompiled against the real table: the
         * failed status and the log must not survive */
        // SAFETY: TestGl serialises the tests; the back end is restored right after.
        unsafe {
            let s = glCreateShader(GL_FRAGMENT_SHADER);
            set_source(s, "void main() { gl_FragColor = vec4(oops); }");
            let saved = crate::shaderc::g_shaderc;
            crate::shaderc::g_shaderc = &REFUSING;
            glCompileShader(s);
            crate::shaderc::g_shaderc = saved;
            assert_eq!(get_iv(s, GL_COMPILE_STATUS), GL_FALSE as GLint);

            set_source(s, &table_source("uniform_color.frag"));
            glCompileShader(s);
            assert_eq!(get_iv(s, GL_COMPILE_STATUS), GL_TRUE as GLint);
            assert_eq!(get_info_log(s), "", "the refusal's log is gone");
        }
    }

    /// The online compiler, called through glCompileShader as an application
    /// would, refuses a text that is not GLSL ES 1.00 at compile time:
    /// COMPILE_STATUS false, no GL error, and the compiler's own diagnostic in
    /// the info log. This is what a `no-shader-table` build does where the
    /// offline table's "missing" tests above do not apply.
    #[test]
    #[cfg(feature = "shaderc-online")]
    fn an_invalid_source_is_refused_at_compile_time_by_the_online_compiler() {
        let gl = TestGl::new(8, 8);
        // SAFETY: TestGl serialises the tests; the back end is restored right after.
        let (status, log) = unsafe {
            let s = glCreateShader(GL_FRAGMENT_SHADER);
            set_source(s, "void main() { gl_FragColor = vec4(oops); }");
            let saved = crate::shaderc::g_shaderc;
            crate::shaderc::g_shaderc = &crate::shaderc::online::backend::SHC_ONLINE;
            glCompileShader(s);
            crate::shaderc::g_shaderc = saved;
            (get_iv(s, GL_COMPILE_STATUS), get_info_log(s))
        };
        assert_eq!(gl.error(), GL_NO_ERROR, "a compile failure is not a GL error");
        assert_eq!(status, GL_FALSE as GLint, "{log}");
        assert!(log.contains("oops"), "the log names the undeclared identifier: {log}");
    }

    /// Online twins of the offline-table contract tests above: the same
    /// behaviours with the online compiler as the back end, which refuses a
    /// text it cannot compile at glCompileShader (status false, `missing`
    /// never set) where the table accepts it and glLinkProgram refuses.
    #[cfg(feature = "shaderc-online")]
    fn online_compile(s: GLuint) {
        // SAFETY: TestGl serialises the tests; the back end is restored right after.
        unsafe {
            let saved = crate::shaderc::g_shaderc;
            crate::shaderc::g_shaderc = &crate::shaderc::online::backend::SHC_ONLINE;
            glCompileShader(s);
            crate::shaderc::g_shaderc = saved;
        }
    }

    /// Twin of `compile_of_an_unknown_source_keeps_the_log_and_raises_no_gl_error`:
    /// a text the compiler cannot compile is status false with the compiler's
    /// own message, no GL error, and never `missing`. (A text that is valid
    /// GLSL compiles, table or not: the table twin's "unknown" source is one.)
    #[test]
    #[cfg(feature = "shaderc-online")]
    fn an_unknown_source_online_is_refused_with_the_compilers_log_and_no_gl_error() {
        let gl = TestGl::new(8, 8);
        let src = "void main() { gl_Position = vec4(0.125); }";
        // SAFETY: plain GL calls.
        let (ok, s) = unsafe {
            let s = glCreateShader(GL_VERTEX_SHADER);
            set_source(s, src);
            online_compile(s);
            (get_iv(s, GL_COMPILE_STATUS), s)
        };
        assert_eq!((ok, get_info_log(s).as_str()), (1, ""), "valid GLSL compiles, table or no table");
        set_source(s, "// first\n\nvoid main() { oops }\n");
        online_compile(s);
        let log = get_info_log(s);
        assert_eq!(get_iv(s, GL_COMPILE_STATUS), 0, "{log}");
        assert_eq!(log, "0:3: '}' : expected ';'");
        assert_eq!(get_iv(s, GL_INFO_LOG_LENGTH), log.len() as GLint + 1, "length includes the NUL");
        assert!(!gl.ctx().objs().shaders[obj_index(s)].missing, "a refused text is not one the table lacks");
        assert_eq!(gl.error(), GL_NO_ERROR, "a compile failure is not a GL error");
    }

    /// Texts the online twins compile as they are (the table's own rows are
    /// not all GLSL ES 1.00 the compiler takes, so these do not come from it).
    #[cfg(feature = "shaderc-online")]
    const VERTEX_TEXT: &str = "attribute float a_pos;\nvoid main() { gl_Position = vec4(a_pos); }\n";
    #[cfg(feature = "shaderc-online")]
    const FRAGMENT_TEXT: &str =
        "precision mediump float;\nuniform vec4 u_color;\nvoid main() { gl_FragColor = u_color; }\n";

    /// Twin of `compile_looks_the_source_up_for_the_shaders_own_stage`: the
    /// online compiler judges a text for the stage it is compiled as, so a
    /// vertex text in a fragment shader is refused (its attribute needs a
    /// default precision in a fragment shader).
    #[test]
    #[cfg(feature = "shaderc-online")]
    fn a_vertex_text_in_a_fragment_shader_is_refused_by_the_online_compiler() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        let s = unsafe {
            let s = glCreateShader(GL_FRAGMENT_SHADER);
            set_source(s, VERTEX_TEXT);
            s
        };
        online_compile(s);
        let log = get_info_log(s);
        assert_eq!(get_iv(s, GL_COMPILE_STATUS), 0, "{log}");
        assert_eq!(log, "0:1: 'a_pos' : 'float' : type requires declaration of default precision qualifier");
        assert!(!gl.ctx().objs().shaders[obj_index(s)].missing);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /// Twin of `recompile_after_fixing_the_source_clears_status_and_log`.
    #[test]
    #[cfg(feature = "shaderc-online")]
    fn recompile_after_fixing_the_source_clears_status_and_log_online() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        let s = unsafe { glCreateShader(GL_FRAGMENT_SHADER) };
        set_source(s, "void main() { /* unknown */ ");
        online_compile(s);
        assert_eq!(get_iv(s, GL_COMPILE_STATUS), 0);
        assert!(get_iv(s, GL_INFO_LOG_LENGTH) > 0);

        set_source(s, FRAGMENT_TEXT);
        online_compile(s);
        assert_eq!(get_iv(s, GL_COMPILE_STATUS), 1);
        assert_eq!(get_iv(s, GL_INFO_LOG_LENGTH), 0);
        assert_eq!(get_info_log(s), "");

        /* and back: a recompile of a refused text is refused again, with a log */
        set_source(s, "void main() { /* unknown again */ ");
        online_compile(s);
        assert_eq!(get_iv(s, GL_COMPILE_STATUS), 0);
        assert!(get_iv(s, GL_INFO_LOG_LENGTH) > 0);
        assert!(!gl.ctx().objs().shaders[obj_index(s)].missing);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /// Twin of `a_shader_the_table_lacks_compiles_and_its_text_is_logged_in_indexed_chunks`:
    /// the online compiler logs one line, the compiler's message with the
    /// stage and the source size, and no "shsrc" chunks (those are the
    /// table-miss harvester's).
    #[test]
    #[cfg(feature = "shaderc-online")]
    fn a_refused_shader_online_logs_one_summary_line_and_no_source_chunks() {
        let gl = TestGl::new(8, 8);
        let src = "// first\n\nvoid main() { oops }\n";
        // SAFETY: plain GL calls.
        let s = unsafe { glCreateShader(GL_FRAGMENT_SHADER) };
        set_source(s, src);
        online_compile(s);
        let logs = gl.logs();
        assert_eq!(
            logs,
            std::vec![format!("glCompileShader({s}): 0:3: '}}' : expected ';' [fragment stage, {} bytes]", src.len())]
        );
        /* the same text again logs again: nothing is remembered as "seen" */
        online_compile(s);
        assert_eq!(gl.logs().len(), 1);
    }

    /// Twin of `a_program_with_a_shader_the_table_lacks_does_not_link`: the
    /// online compiler refuses the stage at glCompileShader, so the program
    /// links no better; glLinkProgram fails without a GL error.
    #[test]
    #[cfg(feature = "shaderc-online")]
    fn a_program_with_a_shader_the_online_compiler_refused_does_not_link() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            let vs = glCreateShader(GL_VERTEX_SHADER);
            let fs = glCreateShader(GL_FRAGMENT_SHADER);
            set_source(vs, "a vertex shader the compiler has never seen\n");
            set_source(fs, FRAGMENT_TEXT);
            online_compile(vs);
            online_compile(fs);
            assert_eq!(get_iv(vs, GL_COMPILE_STATUS), 0);
            assert_eq!(get_iv(fs, GL_COMPILE_STATUS), 1);

            let p = glCreateProgram();
            glAttachShader(p, vs);
            glAttachShader(p, fs);
            glLinkProgram(p);
            let mut linked: GLint = -1;
            glGetProgramiv(p, GL_LINK_STATUS, &mut linked);
            assert_eq!(linked, 0);
            let mut log = [0u8; 128];
            glGetProgramInfoLog(p, log.len() as GLsizei, core::ptr::null_mut(), log.as_mut_ptr() as *mut GLchar);
            let log = core::str::from_utf8(&log[..log.iter().position(|&b| b == 0).unwrap()]).unwrap();
            assert_eq!(log, "link error: vertex and fragment shaders must be attached and compiled");
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /// The 16-hex-digit prefix the "shsrc" lines of a compile log carry.
    #[cfg(not(feature = "no-shader-table"))]
    fn sha16_of(log: &str) -> String {
        let h = log.find("sha256=").expect("the compile log names the text");
        log[h + 7..h + 7 + 16].to_string()
    }

    #[test]
    /* The offline table's own contract: a text the table lacks compiles
     * (COMPILE_STATUS true, `missing` set, "not in offline table" logged) and
     * glLinkProgram refuses it. A `no-shader-table` build has no offline back
     * end, so this runs only with a table; the online compiler refuses such
     * a text at glCompileShader instead, and the `..._online` twin next to
     * `an_invalid_source_is_refused_at_compile_time_by_the_online_compiler`
     * pins that. */
    #[cfg(not(feature = "no-shader-table"))]
    fn a_shader_the_table_lacks_compiles_and_its_text_is_logged_in_indexed_chunks() {
        /* upstream 18692b4: the harvester needs the text of every run-time
         * shader in one run, so the missing shader reports COMPILE_STATUS
         * true and its source goes out as indexed 96-byte chunks */
        let gl = TestGl::new(8, 8);
        // SAFETY: TestGl serialises the tests, so nothing else uses the module state.
        unsafe { STATE.get().nseen = 0 };
        let long_line = "x".repeat(150);
        let src = format!("// first\n\n{long_line}\nlast line without newline");
        // SAFETY: plain GL calls.
        unsafe {
            let s = glCreateShader(GL_FRAGMENT_SHADER);
            set_source(s, &src);
            glCompileShader(s);
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!(get_iv(s, GL_COMPILE_STATUS), 1, "the application goes on to its other stage");
            let logs = gl.logs();
            assert_eq!(logs.len(), 7, "{logs:?}");
            assert!(logs[0].starts_with(&format!("glCompileShader({s}): offline shaderc: fragment shader not in")));
            let sha = sha16_of(&logs[0]);
            assert_eq!(logs[1], format!("shsrc {sha} 0.0:// first"));
            assert_eq!(logs[2], format!("shsrc {sha} 1.0:"), "an empty line is one empty chunk");
            assert_eq!(logs[3], format!("shsrc {sha} 2.0:{}", "x".repeat(96)));
            assert_eq!(logs[4], format!("shsrc {sha} 2.1:{}", "x".repeat(54)), "a long line is chunked, not cut");
            assert_eq!(logs[5], format!("shsrc {sha} 3.0:last line without newline"));
            assert_eq!(logs[6], format!("shsrc {sha} end 4 {}", src.len()));

            /* the same text again: the summary line only, the chunks once */
            glCompileShader(s);
            let logs = gl.logs();
            assert_eq!(logs.len(), 1, "{logs:?}");
            assert!(logs[0].starts_with("glCompileShader("));

            /* another unknown text: its own chunks */
            set_source(s, "second\nunknown\n");
            glCompileShader(s);
            let logs = gl.logs();
            let sha2 = sha16_of(&logs[0]);
            assert_ne!(sha2, sha);
            assert_eq!(logs[1], format!("shsrc {sha2} 0.0:second"));
            assert_eq!(logs[2], format!("shsrc {sha2} 1.0:unknown"));
            assert_eq!(logs[3], format!("shsrc {sha2} end 2 15"), "a trailing newline ends the last line");
            assert_eq!(logs.len(), 4, "{logs:?}");
        }
    }

    #[test]
    /* The offline table's own contract: a text the table lacks compiles
     * (COMPILE_STATUS true, `missing` set, "not in offline table" logged) and
     * glLinkProgram refuses it. A `no-shader-table` build has no offline back
     * end, so this runs only with a table; the online compiler refuses such
     * a text at glCompileShader instead, and the `..._online` twin next to
     * `an_invalid_source_is_refused_at_compile_time_by_the_online_compiler`
     * pins that. */
    #[cfg(not(feature = "no-shader-table"))]
    fn a_program_with_a_shader_the_table_lacks_does_not_link() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            let vs = glCreateShader(GL_VERTEX_SHADER);
            let fs = glCreateShader(GL_FRAGMENT_SHADER);
            set_source(vs, "a vertex shader the table has never seen\n");
            set_source(fs, &table_source("uniform_color.frag"));
            glCompileShader(vs);
            glCompileShader(fs);
            assert_eq!(get_iv(vs, GL_COMPILE_STATUS), 1);
            assert_eq!(get_iv(fs, GL_COMPILE_STATUS), 1);

            let p = glCreateProgram();
            glAttachShader(p, vs);
            glAttachShader(p, fs);
            glLinkProgram(p);
            let mut linked: GLint = -1;
            glGetProgramiv(p, GL_LINK_STATUS, &mut linked);
            assert_eq!(linked, 0);
            let mut log = [0u8; 128];
            glGetProgramInfoLog(p, log.len() as GLsizei, core::ptr::null_mut(), log.as_mut_ptr() as *mut GLchar);
            let log = core::str::from_utf8(&log[..log.iter().position(|&b| b == 0).unwrap()]).unwrap();
            assert_eq!(log, "link error: the vertex shader is not in the offline table");
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn compile_failure_summary_names_stage_and_source_size() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            let s = glCreateShader(GL_VERTEX_SHADER);
            set_source(s, "0123456789");
            glCompileShader(s);
        }
        let logs = gl.logs();
        assert!(logs.iter().any(|l| l.ends_with("[vertex stage, 10 bytes]")), "{logs:?}");
    }

    /* ---- glGetShaderiv / glGetShaderInfoLog ---- */

    #[test]
    fn get_shaderiv_unknown_pname_is_invalid_enum() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            let s = glCreateShader(GL_VERTEX_SHADER);
            let mut v: GLint = -7;
            glGetShaderiv(s, GL_LINK_STATUS, &mut v);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            assert_eq!(v, -7);
        }
    }

    #[test]
    fn get_shaderiv_null_params_is_ignored() {
        let gl = TestGl::new(8, 8);
        // SAFETY: a null params must not be written.
        unsafe {
            let s = glCreateShader(GL_VERTEX_SHADER);
            glGetShaderiv(s, GL_COMPILE_STATUS, core::ptr::null_mut());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn get_shader_info_log_buffer_semantics() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls; buf holds the announced sizes.
        unsafe {
            let s = glCreateShader(GL_VERTEX_SHADER);
            set_source(s, "unknown");
            glCompileShader(s);
            let full = get_info_log(s);
            /* the text is the back end's; the buffer rules below are GL's */
            #[cfg(not(feature = "no-shader-table"))]
            assert!(full.starts_with("offline shaderc: "), "{full}");
            assert!(full.len() > 7, "a log long enough to be cut: {full}");

            let mut buf = [0x55u8; 32];
            let mut len: GLsizei = -1;
            glGetShaderInfoLog(s, 8, &mut len, buf.as_mut_ptr() as *mut GLchar);
            assert_eq!(len, 7);
            assert_eq!(&buf[..7], &full.as_bytes()[..7], "the first seven bytes of the log");
            assert_eq!(&buf[7..9], b"\0\x55", "then the NUL, and nothing past the buffer size");

            let mut buf = [0x55u8; 4];
            glGetShaderInfoLog(s, 0, &mut len, buf.as_mut_ptr() as *mut GLchar);
            assert_eq!(len, 0);
            assert_eq!(buf, [0x55; 4]);

            glGetShaderInfoLog(s, 1, &mut len, buf.as_mut_ptr() as *mut GLchar);
            assert_eq!(len, 0);
            assert_eq!(buf, [0, 0x55, 0x55, 0x55]);

            assert_eq!(gl.error(), GL_NO_ERROR);
            len = -1;
            glGetShaderInfoLog(s, -1, &mut len, buf.as_mut_ptr() as *mut GLchar);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            assert_eq!(len, -1);
        }
    }

    #[test]
    fn info_logs_are_kept_per_shader() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            let bad = glCreateShader(GL_VERTEX_SHADER);
            let good = glCreateShader(GL_VERTEX_SHADER);
            set_source(bad, "unknown");
            set_source(good, &table_source("passthrough.vert"));
            glCompileShader(bad);
            glCompileShader(good);
            assert!(!get_info_log(bad).is_empty());
            assert_eq!(get_info_log(good), "");
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* ---- the rest ---- */

    #[test]
    fn shader_binary_is_rejected_with_invalid_enum() {
        let gl = TestGl::new(8, 8);
        // SAFETY: nothing is read: there are no binary formats.
        unsafe { glShaderBinary(0, core::ptr::null(), 0x1234, core::ptr::null(), 0) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
    }

    #[test]
    fn release_shader_compiler_keeps_compiling_possible() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL call.
        unsafe { glReleaseShaderCompiler() };
        gl.shader(GL_VERTEX_SHADER, "passthrough.vert");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn precision_format_floats_are_ieee_binary32() {
        let gl = TestGl::new(8, 8);
        for stage in [GL_VERTEX_SHADER, GL_FRAGMENT_SHADER] {
            for p in [GL_LOW_FLOAT, GL_MEDIUM_FLOAT, GL_HIGH_FLOAT] {
                let (mut range, mut precision): ([GLint; 2], GLint) = ([0; 2], 0);
                // SAFETY: range holds two GLints, precision one.
                unsafe { glGetShaderPrecisionFormat(stage, p, range.as_mut_ptr(), &mut precision) };
                /* GL ES: floor(log2(max magnitude)) = 127 both ways, 23 mantissa bits */
                assert_eq!((range, precision), ([127, 127], 23));
            }
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn precision_format_ints_are_32_bit_twos_complement() {
        let gl = TestGl::new(8, 8);
        for p in [GL_LOW_INT, GL_MEDIUM_INT, GL_HIGH_INT] {
            let (mut range, mut precision): ([GLint; 2], GLint) = ([0; 2], -1);
            // SAFETY: range holds two GLints, precision one.
            unsafe { glGetShaderPrecisionFormat(GL_FRAGMENT_SHADER, p, range.as_mut_ptr(), &mut precision) };
            /* GL ES: [-2^31, 2^31 - 1] is reported as (31, 30); integers have precision 0 */
            assert_eq!((range, precision), ([31, 30], 0));
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn precision_format_rejects_unknown_enums_and_accepts_null_outputs() {
        let gl = TestGl::new(8, 8);
        let (mut range, mut precision): ([GLint; 2], GLint) = ([-7; 2], -7);
        // SAFETY: range holds two GLints, precision one; null outputs are skipped.
        unsafe {
            glGetShaderPrecisionFormat(GL_TEXTURE_2D, GL_HIGH_FLOAT, range.as_mut_ptr(), &mut precision);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glGetShaderPrecisionFormat(GL_VERTEX_SHADER, GL_FLOAT, range.as_mut_ptr(), &mut precision);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            assert_eq!((range, precision), ([-7; 2], -7));
            glGetShaderPrecisionFormat(GL_VERTEX_SHADER, GL_HIGH_FLOAT, core::ptr::null_mut(), core::ptr::null_mut());
            glGetShaderPrecisionFormat(GL_VERTEX_SHADER, GL_HIGH_INT, core::ptr::null_mut(), core::ptr::null_mut());
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
    }

    /// dEQP `state_query.shader.shader_compile_status` opens by asserting that
    /// a shader that has been created and given nothing reports
    /// GL_COMPILE_STATUS false. That half needs no compiler, so it is the only
    /// part of that case which could be ours; the rest is the compile itself.
    #[test]
    fn a_fresh_shader_has_not_compiled_and_has_no_source_or_log() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        let (vs, fs) = unsafe { (glCreateShader(GL_VERTEX_SHADER), glCreateShader(GL_FRAGMENT_SHADER)) };

        for (sh, want_type) in [(vs, GL_VERTEX_SHADER), (fs, GL_FRAGMENT_SHADER)] {
            let mut v: GLint = -1;
            // SAFETY: `v` holds one value.
            unsafe { glGetShaderiv(sh, GL_COMPILE_STATUS, &mut v) };
            assert_eq!(v, GL_FALSE as GLint, "a shader with no source has not compiled");
            // SAFETY: as above.
            unsafe { glGetShaderiv(sh, GL_SHADER_TYPE, &mut v) };
            assert_eq!(v as GLenum, want_type);
            // SAFETY: as above.
            unsafe { glGetShaderiv(sh, GL_DELETE_STATUS, &mut v) };
            assert_eq!(v, GL_FALSE as GLint);
            // SAFETY: as above.
            unsafe { glGetShaderiv(sh, GL_INFO_LOG_LENGTH, &mut v) };
            assert_eq!(v, 0, "nothing has been logged about it");
            // SAFETY: as above.
            unsafe { glGetShaderiv(sh, GL_SHADER_SOURCE_LENGTH, &mut v) };
            assert_eq!(v, 0, "and it has no source");
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }
}
