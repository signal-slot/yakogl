// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 program binaries (ES 3.0.6 2.12.4; G7,
//! `docs/notes/es3-program-binary.md` sections (b) to (e)): the binary each
//! linked program keeps, `glGetProgramBinary`'s copy of it and
//! `glProgramBinary`'s load.
//!
//! A binary is the program's link inputs (the two sources, the
//! `glBindAttribLocation` bindings and the transform feedback request) in the
//! format of [`super::program_binary_format`]. It is made at the end of a
//! successful link ([`capture`]) and kept until the next link, load or delete
//! ([`forget`]), so that a change to the shader objects without a relink
//! leaves its bytes as they were. A load ([`load`]) compiles both sources
//! again and runs the normal link ([`program::link_with`]), which resets
//! everything a link resets: link status, info log, uniforms, validation.
//!
//! The store is one heap block per `[MAX_CONTEXTS][MAX_PROGRAMS]` slot,
//! keyed by (share group, program name), holding the binary's bytes.
//!
//! Not in a `report-es2` build: `es3.rs` keeps the ES 2.0 answers there and
//! never calls into this module.

use core::ffi::c_char;

use super::consts::*;
use super::context::{obj_index, GlAttribBind, GlContext, MAX_ATTRIB_BINDS, MAX_CONTEXTS, MAX_PROGRAMS};
use super::program::{self, LinkInputs};
use super::program_binary_format as fmt;
use super::shader::INFO_LOG_SIZE;
use super::tf::TfRequest;
use super::types::*;
use crate::shaderc::{self, sha256::Sha256, ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};
use crate::util::{self, Global};

/// `GL_PROGRAM_BINARY_FORMATS`' one value (design (b)).
pub const GL_PROGRAM_BINARY_FORMAT_YAKOGL: GLenum = fmt::GL_PROGRAM_BINARY_FORMAT_YAKOGL;

struct Store {
    /// The kept binary of each program (`util::malloc`), or null. Its length
    /// is in its own header.
    kept: [[*mut u8; MAX_PROGRAMS]; MAX_CONTEXTS],
    /// The program linked, but its binary could not be kept (out of
    /// memory): `glGetProgramBinary` answers GL_OUT_OF_MEMORY (design (d) 4).
    lost: [[bool; MAX_PROGRAMS]; MAX_CONTEXTS],
}

static STORE: Global<Store> = Global::zeroed();

fn slot(group: u8, program: GLuint) -> (usize, usize) {
    (group as usize % MAX_CONTEXTS, obj_index(program) % MAX_PROGRAMS)
}

/// The kept binary of `program`, if any.
fn kept(group: u8, program: GLuint) -> Option<&'static [u8]> {
    let (g, h) = slot(group, program);
    // SAFETY: single-threaded by contract; no other reference to STORE is live.
    let block = unsafe { STORE.get() }.kept[g][h];
    if block.is_null() {
        return None;
    }
    // SAFETY: a block keep() filled with a whole binary, whose header holds
    // its payload length; it lives until forget().
    unsafe {
        let header = core::slice::from_raw_parts(block, fmt::HEADER_BYTES);
        let payload = u32::from_le_bytes([
            header[fmt::OFF_PAYLOAD_BYTES],
            header[fmt::OFF_PAYLOAD_BYTES + 1],
            header[fmt::OFF_PAYLOAD_BYTES + 2],
            header[fmt::OFF_PAYLOAD_BYTES + 3],
        ]);
        Some(core::slice::from_raw_parts(block, fmt::HEADER_BYTES + payload as usize))
    }
}

/// Keep a binary of `len` bytes, written by `fill`, for `program`; when the
/// allocation or `fill` fails keep nothing and remember that.
fn keep(group: u8, program: GLuint, len: usize, fill: impl FnOnce(&mut [u8]) -> bool) {
    forget(group, program);
    let (g, h) = slot(group, program);
    // SAFETY: a plain heap block, owned by STORE until forget().
    let block = unsafe { util::malloc(len.max(1)) } as *mut u8;
    // SAFETY: single-threaded by contract; no other reference to STORE is live.
    let st = unsafe { STORE.get() };
    if block.is_null() {
        st.lost[g][h] = true;
        return;
    }
    // SAFETY: `block` holds `len` bytes.
    if fill(unsafe { core::slice::from_raw_parts_mut(block, len) }) {
        st.kept[g][h] = block;
    } else {
        // SAFETY: the block taken above, not stored.
        unsafe { util::free(block.cast()) };
        st.lost[g][h] = true;
    }
}

/// Drop the kept binary of `program`: a link starts, a load starts, or the
/// program is deleted.
pub(super) fn forget(group: u8, program: GLuint) {
    let (g, h) = slot(group, program);
    // SAFETY: single-threaded by contract; no other reference to STORE is live.
    let st = unsafe { STORE.get() };
    if !st.kept[g][h].is_null() {
        // SAFETY: a block keep() took and nothing else frees.
        unsafe { util::free(st.kept[g][h].cast()) };
        st.kept[g][h] = core::ptr::null_mut();
    }
    st.lost[g][h] = false;
}

/// The GPU tag and name of the identity string (design (b)).
fn gpu() -> ([u8; 4], &'static str) {
    if cfg!(feature = "gpu-v3d") {
        (*b"V3D4", "v3d")
    } else if cfg!(feature = "gpu-powervr") {
        (*b"PVRB", "powervr")
    } else if cfg!(feature = "gpu-vivante") {
        (*b"GC7K", "vivante")
    } else if cfg!(feature = "gpu-mali") {
        (*b"MALI", "mali")
    } else {
        (*b"NONE", "none")
    }
}

/// The SHA-256 of `yakogl <version> gpu=<gpu> es=3.0 shaderc=<..> fmt=1
/// id=<YAKOGL_BUILD_ID>`: a binary loads only into the build that made it.
fn identity() -> [u8; 32] {
    let shaderc_kind = match (cfg!(feature = "shaderc-online"), cfg!(feature = "precompiled-shaders")) {
        (true, true) => "online+precompiled",
        (true, false) => "online",
        (false, true) => "precompiled",
        (false, false) => "offline",
    };
    let mut h = Sha256::new();
    let parts: [&str; 8] = [
        "yakogl ",
        env!("CARGO_PKG_VERSION"),
        " gpu=",
        gpu().1,
        " es=3.0 shaderc=",
        shaderc_kind,
        " fmt=1 id=",
        option_env!("YAKOGL_BUILD_ID").unwrap_or(""),
    ];
    for p in parts {
        h.update(p.as_bytes());
    }
    h.finalize()
}

/// The text of a NUL-terminated source, empty for null.
///
/// # Safety
/// `s` is null or NUL-terminated.
unsafe fn src_bytes<'a>(s: *const c_char) -> &'a [u8] {
    if s.is_null() {
        &[]
    } else {
        util::cstr_bytes(s)
    }
}

fn bind_name(b: &GlAttribBind) -> &[u8] {
    // SAFETY: c_char and u8 have the same layout; the array is 32 bytes.
    let raw = unsafe { core::slice::from_raw_parts(b.name.as_ptr().cast::<u8>(), b.name.len()) };
    &raw[..raw.iter().position(|&c| c == 0).unwrap_or(raw.len())]
}

/// After a successful link of `program` from `inputs`: make its binary and
/// keep it until the next link, load or delete.
///
/// g7-state calls this from `glLinkProgram` (`program.rs`, after
/// `link_with` returns true, outside `REPORT_ES2`); until then only the
/// tests below do.
///
/// # Safety
/// `inputs.vs_src` and `inputs.fs_src` are null or NUL-terminated.
pub(super) unsafe fn capture(c: &GlContext, program: GLuint, inputs: &LinkInputs) {
    let Some(p) = c.program_ref(program) else {
        return;
    };
    let mut binds = [fmt::Binding { name: &[], index: 0 }; MAX_ATTRIB_BINDS];
    let mut n = 0;
    /* a binding with an empty name matches no attribute; it is not a link input */
    for b in p.attrib_binds[..(p.num_attrib_binds as usize).min(MAX_ATTRIB_BINDS)].iter() {
        if !bind_name(b).is_empty() && n < binds.len() {
            binds[n] = fmt::Binding { name: bind_name(b), index: b.index };
            n += 1;
        }
    }
    let snap = fmt::Snapshot {
        identity: identity(),
        tag: gpu().0,
        vs_src: src_bytes(inputs.vs_src),
        fs_src: src_bytes(inputs.fs_src),
        binds: &binds[..n],
        tf: fmt::TfRequest { names: inputs.tf.names, count: inputs.tf.count, mode: inputs.tf.mode },
    };
    keep(c.group, program, fmt::serialized_len(&snap), |out| fmt::write(&snap, out).is_ok());
}

/// `GL_PROGRAM_BINARY_LENGTH`: the kept binary's length when linked, else 0
/// (Table 6.18).
pub(super) fn length(c: &GlContext, program: GLuint) -> GLint {
    if !c.program_ref(program).is_some_and(|p| p.linked) {
        return 0;
    }
    kept(c.group, program).map_or(0, |b| b.len() as GLint)
}

/// `glGetProgramBinary` after the name check (design (d), steps 2-5).
///
/// # Safety
/// `length` and `binary_format` are null or valid for one value; `binary` is
/// null or valid for `buf_size` bytes (GL contract).
pub(super) unsafe fn get(
    c: &mut GlContext,
    program: GLuint,
    buf_size: GLsizei,
    length: *mut GLsizei,
    binary_format: *mut GLenum,
    binary: *mut GLvoid,
) {
    const FUNC: &str = "glGetProgramBinary";
    let refuse = |c: &mut GlContext, e: GLenum| {
        util::app_put(length, 0);
        c.set_error(e, FUNC);
    };
    /* 2.12.4: an unlinked program's binary length is zero, and the call is
     * GL_INVALID_OPERATION */
    if !c.program_ref(program).is_some_and(|p| p.linked) {
        return refuse(c, GL_INVALID_OPERATION);
    }
    let Some(bytes) = kept(c.group, program) else {
        let (g, h) = slot(c.group, program);
        /* linked with no binary: the allocation failed at link, or the link
         * did not capture one (length 0, so any bufSize is too small) */
        let e = if STORE.get().lost[g][h] { GL_OUT_OF_MEMORY } else { GL_INVALID_OPERATION };
        return refuse(c, e);
    };
    /* a negative bufSize is less than any length (design (d) 3) */
    if (buf_size as i64) < bytes.len() as i64 {
        return refuse(c, GL_INVALID_OPERATION);
    }
    if !binary.is_null() {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), binary.cast::<u8>(), bytes.len());
    }
    util::app_put(length, bytes.len() as GLsizei);
    util::app_put(binary_format, GL_PROGRAM_BINARY_FORMAT_YAKOGL);
}

/// Why a load did not link the program. Each leaves link status FALSE and
/// an info log, and no GL error (2.12.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LoadError {
    /// The bytes are not a binary of this build (the format's reason).
    Rejected(&'static str),
    /// The relink of the binary's inputs failed (its log is the program's).
    Link,
}

/// A NUL-terminated heap copy of a source, freed on drop.
struct CSrc(*mut c_char);

impl CSrc {
    fn new(text: &[u8]) -> Option<CSrc> {
        // SAFETY: a plain heap block of text.len() + 1 bytes, filled below.
        let p = unsafe { util::malloc(text.len() + 1) } as *mut u8;
        if p.is_null() {
            return None;
        }
        // SAFETY: `p` holds text.len() + 1 bytes; `text` has no NUL (parse).
        unsafe {
            core::ptr::copy_nonoverlapping(text.as_ptr(), p, text.len());
            *p.add(text.len()) = 0;
        }
        Some(CSrc(p.cast()))
    }
}

impl Drop for CSrc {
    fn drop(&mut self) {
        // SAFETY: the block new() took.
        unsafe { util::free(self.0.cast()) };
    }
}

/// A compile of one of the binary's stages, released on drop (the link
/// keeps its own copies).
struct Compiled(ShcShader);

impl Drop for Compiled {
    fn drop(&mut self) {
        shaderc::release_shader(&self.0);
    }
}

fn compile(stage: u32, src: &CSrc) -> Option<Compiled> {
    let compile = shaderc::backend().and_then(|b| b.compile)?;
    // SAFETY: every field of ShcShader is valid when all-zero.
    let mut out = Compiled(unsafe { core::mem::zeroed() });
    let mut log = [0u8; INFO_LOG_SIZE];
    // SAFETY: a NUL-terminated source; the log buffer is INFO_LOG_SIZE bytes.
    let rc = unsafe { compile(stage, src.0, &mut out.0, log.as_mut_ptr() as *mut c_char, INFO_LOG_SIZE) };
    (rc == 0).then_some(out)
}

/// A link that fails at once, with `why` in the info log: link status FALSE
/// and everything a link resets is reset (2.12.4: "Any information about a
/// previous link or load of that program object is lost").
///
/// `program.rs` keeps the info log private, so until g7-state adds a log
/// setter there, the reason goes in through the one link error that quotes
/// its input: a transform feedback varying (named `why`) that the vertex
/// shader does not declare. The log then reads "link error: Transform
/// feedback varying program binary rejected: ... undeclared.".
unsafe fn reject(c: &mut GlContext, program: GLuint, why: &'static str) -> Result<(), LoadError> {
    const VS: &core::ffi::CStr = c"#version 300 es\nvoid main() { gl_Position = vec4(0.0); }\n";
    let mut name = [0u8; 128];
    let n = why.len().min(name.len() - 1);
    name[..n].copy_from_slice(&why.as_bytes()[..n]);
    let inputs = LinkInputs {
        vs_src: VS.as_ptr(),
        fs_src: core::ptr::null(),
        // SAFETY: every field of ShcShader is valid when all-zero; the link
        // fails at the varyings, before it reads either stage.
        rvs: core::mem::zeroed(),
        rfs: core::mem::zeroed(),
        tf: TfRequest { names: &name[..n + 1], count: 1, mode: GL_INTERLEAVED_ATTRIBS },
    };
    let linked = program::link_with(c, program, &inputs);
    debug_assert!(!linked);
    Err(LoadError::Rejected(why))
}

const ERR_MEMORY: &str = "program binary rejected: out of memory for its sources";
const ERR_NO_COMPILER: &str = "program binary rejected: no shader compiler in this build";
const ERR_VS: &str = "program binary rejected: its vertex shader does not compile";
const ERR_FS: &str = "program binary rejected: its fragment shader does not compile";

/// `glProgramBinary` after its error checks (design (c), (d) step 5): load
/// `bytes` into `program`, which is linked again from the binary's inputs.
/// On success the bytes themselves are kept, so `glGetProgramBinary`
/// returns them.
///
/// The program's own `glBindAttribLocation` bindings and its pending
/// transform feedback request are not changed, whatever the outcome: the
/// binary's are used for this link only (2.12.4).
///
/// # Safety
/// Called from a GL entry point with `c` the current context.
pub(super) unsafe fn load(c: &mut GlContext, program: GLuint, bytes: &[u8]) -> Result<(), LoadError> {
    forget(c.group, program);
    let parsed = match fmt::parse(bytes, &identity(), &gpu().0) {
        Ok(p) => p,
        Err(why) => return reject(c, program, why),
    };
    let (Some(vs_src), Some(fs_src)) = (CSrc::new(parsed.vs_src), CSrc::new(parsed.fs_src)) else {
        return reject(c, program, ERR_MEMORY);
    };
    if shaderc::backend().and_then(|b| b.compile).is_none() {
        return reject(c, program, ERR_NO_COMPILER);
    }
    let Some(rvs) = compile(SHC_STAGE_VERTEX, &vs_src) else {
        return reject(c, program, ERR_VS);
    };
    let Some(rfs) = compile(SHC_STAGE_FRAGMENT, &fs_src) else {
        return reject(c, program, ERR_FS);
    };

    /* the binary's bindings for this link; the program's own come back after */
    let Some(p) = c.program(program) else {
        return Err(LoadError::Link);
    };
    let own = (p.attrib_binds, p.num_attrib_binds);
    let zero = GlAttribBind { name: [0; 32], index: 0 };
    p.attrib_binds = [zero; MAX_ATTRIB_BINDS];
    p.num_attrib_binds = 0;
    for (b, dst) in parsed.binds.iter().zip(p.attrib_binds.iter_mut()) {
        for (d, &s) in dst.name.iter_mut().zip(b.name) {
            *d = s as c_char;
        }
        dst.index = b.index;
        p.num_attrib_binds += 1;
    }
    let inputs = LinkInputs {
        vs_src: vs_src.0,
        fs_src: fs_src.0,
        rvs: rvs.0,
        rfs: rfs.0,
        tf: TfRequest { names: parsed.tf.names, count: parsed.tf.count, mode: parsed.tf.mode },
    };
    let linked = program::link_with(c, program, &inputs);
    if let Some(p) = c.program(program) {
        (p.attrib_binds, p.num_attrib_binds) = own;
    }
    if !linked {
        return Err(LoadError::Link);
    }
    keep(c.group, program, bytes.len(), |out| {
        out.copy_from_slice(bytes);
        true
    });
    Ok(())
}

/// Unit tests: free every kept binary.
#[cfg(test)]
pub(crate) fn reset_for_tests() {
    for g in 0..MAX_CONTEXTS {
        for h in 0..MAX_PROGRAMS {
            forget(g as u8, (h as GLuint) << 1);
        }
    }
}

#[cfg(all(test, feature = "shaderc-online", not(feature = "report-es2")))]
mod tests {
    #![allow(clippy::undocumented_unsafe_blocks)]
    use super::*;
    use crate::gl::draw::glDrawArrays;
    use crate::gl::es3::*;
    use crate::gl::program::*;
    use crate::gl::shader::*;
    use crate::gl::state::*;
    use crate::gl::vertex::*;
    use crate::testutil::TestGl;
    use std::ffi::CString;
    use std::string::String;
    use std::vec;
    use std::vec::Vec;

    /// The in-process compiler for the length of a test.
    struct BackEnd(*const crate::shaderc::ShcBackend);
    impl Drop for BackEnd {
        fn drop(&mut self) {
            // SAFETY: restoring what was saved, under the suite's guard.
            unsafe { crate::shaderc::g_shaderc = self.0 };
        }
    }

    fn fresh() -> (TestGl, BackEnd) {
        let gl = TestGl::new(8, 8);
        super::reset_for_tests();
        let be = BackEnd(unsafe { crate::shaderc::g_shaderc });
        unsafe { crate::shaderc::g_shaderc = &crate::shaderc::online::SHC_ONLINE };
        (gl, be)
    }

    const VS: &str = "#version 300 es\nin highp vec4 a_position;\nout highp vec4 v;\nout highp float f;\n\
        void main() { v = a_position; f = a_position.x; gl_Position = a_position; }\n";
    const FS: &str = "#version 300 es\nlayout(location = 0) out mediump vec4 o_fragColor;\n\
        void main() { o_fragColor = vec4(1.0); }\n";

    fn shader_src(ty: GLenum, text: &str) -> GLuint {
        let text = CString::new(text).unwrap();
        unsafe {
            let s = glCreateShader(ty);
            let p: [*const c_char; 1] = [text.as_ptr()];
            glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
            s
        }
    }

    fn shader(ty: GLenum, text: &str) -> GLuint {
        let s = shader_src(ty, text);
        unsafe {
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            assert_eq!(ok, 1, "{text:?} did not compile");
        }
        s
    }

    fn attach(vs: &str, fs: &str) -> (GLuint, GLuint, GLuint) {
        let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, v);
            glAttachShader(p, f);
            (p, v, f)
        }
    }

    fn progi(p: GLuint, pname: GLenum) -> GLint {
        let mut v = i32::MIN;
        unsafe { glGetProgramiv(p, pname, &mut v) };
        v
    }

    fn log(p: GLuint) -> String {
        let mut buf = [0u8; 1024];
        let mut len: GLsizei = 0;
        unsafe { glGetProgramInfoLog(p, buf.len() as GLsizei, &mut len, buf.as_mut_ptr() as *mut c_char) };
        String::from_utf8_lossy(&buf[..len.max(0) as usize]).into_owned()
    }

    fn cname(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    /// glLinkProgram, then what g7-state's hook in it will do: capture the
    /// binary from the inputs the link read (the attached shader objects and
    /// the pending transform feedback request).
    fn link(gl: &TestGl, p: GLuint) -> bool {
        unsafe { glLinkProgram(p) };
        let ok = progi(p, GL_LINK_STATUS) == 1;
        forget(gl.ctx().group, p);
        if ok {
            let c = gl.ctx();
            let pr = c.program_ref(p).unwrap();
            let src = |h: GLuint| c.shader_ref(h).map_or(core::ptr::null(), |s| s.source as *const c_char);
            let inputs = LinkInputs {
                vs_src: src(pr.vs),
                fs_src: src(pr.fs),
                rvs: pr.rvs,
                rfs: pr.rfs,
                tf: crate::gl::tf::pending(c.group, p),
            };
            unsafe { capture(c, p, &inputs) };
        }
        ok
    }

    fn get_binary(gl: &TestGl, p: GLuint) -> (GLenum, Vec<u8>) {
        let len = length(gl.ctx(), p);
        assert!(len > 0, "no binary kept: {}", log(p));
        let mut buf = vec![0u8; len as usize];
        let (mut written, mut format) = (-1, 0);
        unsafe { glGetProgramBinary(p, len, &mut written, &mut format, buf.as_mut_ptr().cast()) };
        assert_eq!(gl.error(), GL_NO_ERROR, "glGetProgramBinary");
        assert_eq!(written, len);
        (format, buf)
    }

    fn load_into(p: GLuint, format: GLenum, bytes: &[u8]) {
        unsafe { glProgramBinary(p, format, bytes.as_ptr().cast(), bytes.len() as GLsizei) };
    }

    fn linked(gl: &TestGl) -> (GLuint, GLuint, GLuint) {
        let pr = attach(VS, FS);
        assert!(link(gl, pr.0), "{}", log(pr.0));
        pr
    }

    /// Design (g) case 2, and G2/G6 of inv-pb-mesa: a binary loaded into a
    /// new program is the same program, with an empty log, validation reset,
    /// and the same bytes kept.
    #[test]
    fn round_trip_into_a_new_program() {
        let (gl, _be) = fresh();
        let (un, _, _) = attach(VS, FS);
        assert_eq!(length(gl.ctx(), un), 0, "unlinked");
        let (src, _, _) = linked(&gl);
        let (fmt, bytes) = get_binary(&gl, src);
        assert_eq!(fmt, GL_PROGRAM_BINARY_FORMAT_YAKOGL);
        assert!(bytes.len() > fmt::HEADER_BYTES);
        assert_eq!(&bytes[..4], b"YKPB");

        let (dst, _, _) = attach(VS, FS);
        unsafe { glValidateProgram(dst) };
        load_into(dst, fmt, &bytes);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(progi(dst, GL_LINK_STATUS), 1, "{}", log(dst));
        assert_eq!(log(dst), "", "a successful load leaves an empty log");
        assert_eq!(progi(dst, GL_VALIDATE_STATUS), 0, "a load resets the validation");
        unsafe { glValidateProgram(dst) };
        assert_eq!(progi(dst, GL_VALIDATE_STATUS), 1);
        for q in [GL_ACTIVE_UNIFORMS, GL_ACTIVE_ATTRIBUTES, GL_ACTIVE_ATTRIBUTE_MAX_LENGTH] {
            assert_eq!(progi(dst, q), progi(src, q), "{q:#x}");
        }
        let a = cname("a_position");
        unsafe {
            assert_eq!(glGetAttribLocation(dst, a.as_ptr()), glGetAttribLocation(src, a.as_ptr()));
            assert!(glGetAttribLocation(dst, a.as_ptr()) >= 0);
        }
        assert_eq!(length(gl.ctx(), dst), bytes.len() as GLint);
        assert_eq!(get_binary(&gl, dst), (fmt, bytes), "the loaded bytes are kept");
    }

    /// dEQP uniform_reset_on_binary_load, with G1: the locations taken before
    /// the load still name the same uniforms, and every one reads 0.
    #[test]
    fn a_load_into_the_current_program_resets_its_uniforms() {
        let (gl, _be) = fresh();
        let vs = "#version 300 es\nin highp vec4 a_position;\nuniform bool u_boolVar;\nuniform highp int u_intVar;\n\
            uniform highp float u_floatVar;\nvoid main() {\n \
            highp float s = (u_boolVar ? 1.0 : 0.0) + float(u_intVar) + u_floatVar;\n \
            gl_Position = a_position + vec4(s * 0.0); }\n";
        let fs = "#version 300 es\nlayout(location = 0) out mediump vec4 o_fragColor;\nuniform bool u_boolVar;\n\
            uniform highp int u_intVar;\nuniform highp float u_floatVar;\nvoid main() {\n \
            highp float s = (u_boolVar ? 1.0 : 0.0) + float(u_intVar) + u_floatVar;\n \
            o_fragColor = vec4(s * 0.0); }\n";
        let (p, _, _) = attach(vs, fs);
        assert!(link(&gl, p), "{}", log(p));
        let (fmt, bytes) = get_binary(&gl, p);
        let names = [cname("u_boolVar"), cname("u_intVar"), cname("u_floatVar")];
        let locs: Vec<GLint> = names.iter().map(|n| unsafe { glGetUniformLocation(p, n.as_ptr()) }).collect();
        assert!(locs.iter().all(|&l| l >= 0));
        let quad: [f32; 16] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0];
        unsafe {
            glUseProgram(p);
            glUniform1i(locs[0], GL_TRUE as GLint);
            glUniform1i(locs[1], 417);
            glUniform1f(locs[2], 733.0);
            let a = glGetAttribLocation(p, c"a_position".as_ptr()) as GLuint;
            glEnableVertexAttribArray(a);
            glVertexAttribPointer(a, 4, GL_FLOAT, GL_FALSE as GLboolean, 0, quad.as_ptr().cast());
            glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
            assert_eq!(gl.error(), GL_NO_ERROR, "the first draw");

            load_into(p, fmt, &bytes);
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!(progi(p, GL_LINK_STATUS), 1, "{}", log(p));
            let again: Vec<GLint> = names.iter().map(|n| glGetUniformLocation(p, n.as_ptr())).collect();
            assert_eq!(again, locs, "the same locations after the load");
            let (mut b, mut i, mut f) = (-1, -1, -1.0f32);
            glGetUniformiv(p, locs[0], &mut b);
            glGetUniformiv(p, locs[1], &mut i);
            glGetUniformfv(p, locs[2], &mut f);
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!((b, i, f), (0, 0, 0.0), "a load resets every uniform");
            let mut cur = 0;
            glGetIntegerv(GL_CURRENT_PROGRAM, &mut cur);
            assert_eq!(cur as GLuint, p, "still current");
            glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
            assert_eq!(gl.error(), GL_NO_ERROR, "and it draws");
        }
    }

    /// The kept binary does not read the shader objects again: a change to
    /// them without a relink leaves its bytes (dEQP binary_persistence).
    #[test]
    fn the_kept_binary_survives_changes_to_the_shader_objects() {
        let (gl, _be) = fresh();
        let (p, v, f) = linked(&gl);
        let before = get_binary(&gl, p);
        unsafe {
            let t = cname("#version 300 es\nvoid main() { gl_Position = vec4(1.0); }\n");
            let pp: [*const c_char; 1] = [t.as_ptr()];
            glShaderSource(v, 1, pp.as_ptr(), core::ptr::null());
            glCompileShader(v);
            glDetachShader(p, f);
            glDeleteShader(f);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(get_binary(&gl, p), before, "Program binary changed");
    }

    /// The binary's attribute assignments are used for the load; the
    /// program's own binding survives it (2.12.4).
    #[test]
    fn attribute_bindings_come_from_the_binary() {
        let (gl, _be) = fresh();
        let (src, _, _) = attach(VS, FS);
        let a = cname("a_position");
        unsafe { glBindAttribLocation(src, 3, a.as_ptr()) };
        assert!(link(&gl, src), "{}", log(src));
        let (fmt, bytes) = get_binary(&gl, src);
        let (dst, _, _) = attach(VS, FS);
        unsafe {
            glBindAttribLocation(dst, 5, a.as_ptr());
            load_into(dst, fmt, &bytes);
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!(progi(dst, GL_LINK_STATUS), 1, "{}", log(dst));
            assert_eq!(glGetAttribLocation(dst, a.as_ptr()), 3, "the binary's assignment");
        }
        assert!(link(&gl, dst), "{}", log(dst));
        assert_eq!(unsafe { glGetAttribLocation(dst, a.as_ptr()) }, 5, "the program's own binding");
    }

    /// The binary's transform feedback request is linked; the target's own
    /// pending request is what its next glLinkProgram uses.
    #[test]
    fn transform_feedback_state_round_trips() {
        let (gl, _be) = fresh();
        let varyings = |p: GLuint, names: &[&str], mode: GLenum| {
            let names: Vec<CString> = names.iter().map(|n| cname(n)).collect();
            let ptrs: Vec<*const c_char> = names.iter().map(|n| n.as_ptr()).collect();
            unsafe { glTransformFeedbackVaryings(p, ptrs.len() as GLsizei, ptrs.as_ptr(), mode) };
        };
        let (src, _, _) = attach(VS, FS);
        varyings(src, &["v", "f"], GL_SEPARATE_ATTRIBS);
        assert!(link(&gl, src), "{}", log(src));
        let (fmt, bytes) = get_binary(&gl, src);
        let (dst, _, _) = attach(VS, FS);
        varyings(dst, &["f"], GL_INTERLEAVED_ATTRIBS);
        load_into(dst, fmt, &bytes);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(progi(dst, GL_LINK_STATUS), 1, "{}", log(dst));
        assert_eq!(progi(dst, GL_TRANSFORM_FEEDBACK_VARYINGS), 2);
        assert_eq!(progi(dst, GL_TRANSFORM_FEEDBACK_BUFFER_MODE), GL_SEPARATE_ATTRIBS as GLint);
        assert!(link(&gl, dst), "{}", log(dst));
        assert_eq!(progi(dst, GL_TRANSFORM_FEEDBACK_VARYINGS), 1);
        assert_eq!(progi(dst, GL_TRANSFORM_FEEDBACK_BUFFER_MODE), GL_INTERLEAVED_ATTRIBS as GLint);
    }

    /// A binary that does not load: link status FALSE, a log naming the
    /// reason, no GL error, no kept binary; the shaders and bindings stay.
    #[test]
    fn rejected_binaries_set_the_link_status_and_raise_no_error() {
        let (gl, _be) = fresh();
        let (src, _, _) = linked(&gl);
        let (fmt, good) = get_binary(&gl, src);
        let mut cases: Vec<(Vec<u8>, &str)> = Vec::new();
        for (off, why) in [
            (fmt::OFF_MAGIC, fmt::ERR_MAGIC),
            (fmt::OFF_VERSION, fmt::ERR_VERSION),
            (fmt::OFF_HEADER_BYTES, fmt::ERR_HEADER_BYTES),
            (fmt::OFF_TAG, fmt::ERR_TAG),
            (fmt::OFF_IDENTITY, fmt::ERR_IDENTITY),
            (fmt::OFF_PAYLOAD_HASH, fmt::ERR_PAYLOAD_HASH),
            (fmt::HEADER_BYTES, fmt::ERR_PAYLOAD_HASH),
        ] {
            let mut b = good.clone();
            b[off] ^= 1;
            cases.push((b, why));
        }
        cases.push((good[..good.len() - 1].to_vec(), fmt::ERR_PAYLOAD_BYTES));
        cases.push((Vec::new(), fmt::ERR_SHORT));
        /* a source that does not compile, in an otherwise good binary */
        let snap = fmt::Snapshot {
            identity: identity(),
            tag: gpu().0,
            vs_src: b"#version 300 es\nvoid main() { nonsense; }\n",
            fs_src: FS.as_bytes(),
            binds: &[],
            tf: fmt::TfRequest { names: &[], count: 0, mode: 0 },
        };
        let mut bad_vs = vec![0u8; fmt::serialized_len(&snap)];
        fmt::write(&snap, &mut bad_vs).unwrap();
        cases.push((bad_vs, ERR_VS));

        let a = cname("a_position");
        for (bytes, why) in &cases {
            let (dst, _, _) = attach(VS, FS);
            unsafe { glBindAttribLocation(dst, 6, a.as_ptr()) };
            /* linked first: a rejected load loses the previous link */
            assert!(link(&gl, dst));
            load_into(dst, fmt, bytes);
            assert_eq!(gl.error(), GL_NO_ERROR, "{why}");
            assert_eq!(progi(dst, GL_LINK_STATUS), 0, "{why}");
            assert!(log(dst).contains(why), "{why}: {}", log(dst));
            assert_eq!(length(gl.ctx(), dst), 0, "{why}");
            unsafe { glUseProgram(dst) };
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "{why}: not usable");
            assert_eq!(progi(dst, GL_ATTACHED_SHADERS), 2, "{why}");
            assert!(link(&gl, dst), "{why}: {}", log(dst));
            assert_eq!(unsafe { glGetAttribLocation(dst, a.as_ptr()) }, 6, "{why}: the binding stays");
        }
        /* a null binary with a length */
        let (dst, _, _) = attach(VS, FS);
        unsafe { glProgramBinary(dst, fmt, core::ptr::null(), good.len() as GLsizei) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(progi(dst, GL_LINK_STATUS), 0);
        assert!(!log(dst).is_empty());
    }

    /// The errors of design (d).
    #[test]
    fn errors_of_the_binary_calls() {
        let (gl, _be) = fresh();
        let (p, vs, _) = linked(&gl);
        let (un, _, _) = attach(VS, FS);
        let (fmt, bytes) = get_binary(&gl, p);
        let len = bytes.len() as GLsizei;
        let mut buf = vec![0u8; bytes.len()];
        let mut get = |p: GLuint, size: GLsizei| -> (GLenum, GLsizei) {
            let (mut written, mut f) = (77, 0);
            unsafe { glGetProgramBinary(p, size, &mut written, &mut f, buf.as_mut_ptr().cast()) };
            (gl.error(), written)
        };
        assert_eq!(get(0, len).0, GL_INVALID_VALUE, "name 0");
        assert_eq!(get(p + 2000, len).0, GL_INVALID_VALUE, "no object");
        assert_eq!(get(vs, len).0, GL_INVALID_OPERATION, "a shader name");
        assert_eq!(get(un, len), (GL_INVALID_OPERATION, 0), "unlinked");
        for size in [0, len - 1, -1] {
            assert_eq!(get(p, size), (GL_INVALID_OPERATION, 0), "bufSize {size}");
        }
        assert_eq!(get(p, len), (GL_NO_ERROR, len));
        assert_eq!(get(p, len + 9), (GL_NO_ERROR, len), "a larger buffer");
        unsafe {
            /* a null binary pointer copies nothing and does not crash */
            glGetProgramBinary(p, len, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut());
            assert_eq!(gl.error(), GL_NO_ERROR);

            let pb = |p: GLuint, f: GLenum, n: GLsizei| {
                glProgramBinary(p, f, bytes.as_ptr().cast(), n);
                gl.error()
            };
            assert_eq!(pb(0, fmt, len), GL_INVALID_VALUE, "name 0");
            assert_eq!(pb(vs, fmt, len), GL_INVALID_OPERATION, "a shader name");
            assert_eq!(pb(un, fmt, -1), GL_INVALID_VALUE, "length -1");
            assert_eq!(pb(p, u32::MAX, len), GL_INVALID_ENUM, "format -1");
            assert_eq!(pb(p, 0, len), GL_INVALID_ENUM, "format 0");
            assert_eq!(progi(p, GL_LINK_STATUS), 1, "a refused format changes nothing");
            assert_eq!(length(gl.ctx(), p), len, "nor the kept binary");

            glProgramParameteri(0, GL_PROGRAM_BINARY_RETRIEVABLE_HINT, GL_TRUE as GLint);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "glProgramParameteri(0)");
            glProgramParameteri(vs, GL_PROGRAM_BINARY_RETRIEVABLE_HINT, GL_TRUE as GLint);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "glProgramParameteri(a shader)");
        }
    }

    /// 2.15.2: the program of an active (also a paused) transform feedback
    /// object cannot be loaded into.
    #[test]
    fn a_program_in_active_transform_feedback_is_not_loaded() {
        let (gl, _be) = fresh();
        let (p, _, _) = attach(VS, FS);
        let f = cname("f");
        let ptrs = [f.as_ptr()];
        unsafe { glTransformFeedbackVaryings(p, 1, ptrs.as_ptr(), GL_INTERLEAVED_ATTRIBS) };
        assert!(link(&gl, p), "{}", log(p));
        let (fmt, bytes) = get_binary(&gl, p);
        unsafe {
            let mut b = 0;
            crate::gl::buffer::glGenBuffers(1, &mut b);
            crate::gl::buffer::glBindBuffer(GL_TRANSFORM_FEEDBACK_BUFFER, b);
            crate::gl::buffer::glBufferData(GL_TRANSFORM_FEEDBACK_BUFFER, 256, core::ptr::null(), GL_DYNAMIC_COPY);
            glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, b);
            glUseProgram(p);
            glBeginTransformFeedback(GL_POINTS);
            assert_eq!(gl.error(), GL_NO_ERROR, "begin");
            load_into(p, fmt, &bytes);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "active");
            glPauseTransformFeedback();
            load_into(p, fmt, &bytes);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "paused");
            glEndTransformFeedback();
        }
        assert_eq!(progi(p, GL_LINK_STATUS), 1, "the refused load changed nothing");
    }

    /// forget() drops the kept binary; an unlinked program has length 0, and
    /// one whose capture never ran answers glGetProgramBinary as before G7.
    #[test]
    fn forget_drops_the_binary() {
        let (gl, _be) = fresh();
        let (p, _, _) = linked(&gl);
        assert!(length(gl.ctx(), p) > 0);
        forget(gl.ctx().group, p);
        assert_eq!(length(gl.ctx(), p), 0);
        let mut n = 7;
        unsafe { glGetProgramBinary(p, 1 << 20, &mut n, core::ptr::null_mut(), core::ptr::null_mut()) };
        assert_eq!((gl.error(), n), (GL_INVALID_OPERATION, 0));
    }
}
