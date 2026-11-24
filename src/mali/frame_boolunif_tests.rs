// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A true `bool` uniform is pushed as `~0` (0xffffffff), as Mesa 26.2.1
//! panfrost uploads it, whichever setter loaded it: the GL core stores 0 or
//! 1 and the compiled shader reads the word as a mask (`ICMP_OR.v2u16.eq.m1
//! r, mask.h00, u.w.h00`; docs/notes/mali-deqp-boolpix.md cause BW, the
//! 277 wrong-pixel dEQP bool/bvec cases of #11306). `tests/mali_ref` checks
//! the FAU word the reference holds, never what the driver writes: these
//! tests draw GL programs on the OSAL mock and read the push block the frame
//! names, for `bool`, `bvec2`, `bvec3` and `bvec4`, set through `glUniform1i`,
//! `glUniform2i`, `glUniform3i`, `glUniform4i` and `glUniform1f` with true and
//! false values; a bool array and a struct member (shapes the online compiler
//! does not link yet) are pinned on the conversion over hand-made table rows,
//! the same function the draw calls. A `float` uniform keeps its bits.
//!
//! Feature set: host-test,gpu-mali,shaderc-online,no-shader-table.

use super::backend::tests::{reg_at_idvs, with, Rig, VERTICES};
use crate::gl::consts::*;
use crate::gl::context;
use crate::gl::draw::glDrawArrays;
use crate::gl::framebuffer::{
    glBindFramebuffer, glCheckFramebufferStatus, glFramebufferTexture2D, glGenFramebuffers, glReadPixels,
};
use crate::gl::program::{
    glAttachShader, glCreateProgram, glGetProgramInfoLog, glGetProgramiv, glGetUniformLocation, glLinkProgram,
    glUniform1f, glUniform1i, glUniform4i, glUseProgram,
};
use crate::gl::shader::{glCompileShader, glCreateShader, glGetShaderiv, glShaderSource};
use crate::gl::state::{glClear, glClearColor, glGetError, glViewport};
use crate::gl::texture::{glBindTexture, glGenTextures, glTexImage2D};
use crate::gl::types::*;
use crate::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use crate::mali::program;
use std::ffi::CString;

const VS: &str = "attribute vec4 dEQP_Position;\nvoid main() { gl_Position = dEQP_Position; }\n";
const T: u32 = 0xffff_ffff;

fn shader(ty: GLenum, src: &str) -> GLuint {
    let text = CString::new(src).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{src} did not compile");
        s
    }
}

/// One stage's push block as the frame names it: its FAU register, the
/// words it counts.
fn push_block(words: &[u64], reg: u32) -> Vec<u32> {
    let f = reg_at_idvs(words, reg).expect("a FAU register");
    let n = 2 * (f >> 56) as usize;
    with(|m| m.read(f & 0x00ff_ffff_ffff_ffff, 4 * n))
        .chunks(4)
        .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

/// Draw `fs` with `set` called on the linked program, and return the
/// fragment stage's push words.
fn fs_push(fs: &str, set: &dyn Fn(GLuint)) -> Vec<u32> {
    let _rig = Rig::new(true);
    assert_eq!(crate::backend::init(), 0);
    let out;
    // SAFETY: the rig serialises the library; plain GL calls.
    unsafe {
        let ctx = context::gl_context_create(64, 64);
        assert!(!ctx.is_null());
        let (mut fbo, mut tex) = (0, 0);
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 64, 64, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glViewport(0, 0, 64, 64);
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        let mut log = [0u8; 512];
        let mut n: GLsizei = 0;
        glGetProgramInfoLog(p, 512, &mut n, log.as_mut_ptr().cast());
        let log = String::from_utf8_lossy(&log[..n.max(0) as usize]).into_owned();
        assert_eq!(ok, 1, "the program links: {log}");
        glUseProgram(p);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, VERTICES.as_ptr().cast());
        set(p);
        assert_eq!(glGetError(), GL_NO_ERROR, "the uniform calls");
        glDrawArrays(GL_TRIANGLES, 0, 6);
        let mut px = vec![0u8; 64 * 64 * 4];
        glReadPixels(0, 0, 64, 64, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
        assert_eq!(glGetError(), GL_NO_ERROR);
        let frames: Vec<Vec<u64>> = with(|m| m.submits[1..].iter().map(|s| s.1.clone()).collect());
        assert_eq!(frames.len(), 1);
        out = push_block(&frames[0], 12);
        context::gl_context_destroy(ctx);
    }
    out
}

fn loc(p: GLuint, name: &str) -> GLint {
    let n = CString::new(name).unwrap();
    // SAFETY: a NUL-terminated name.
    let l = unsafe { glGetUniformLocation(p, n.as_ptr()) };
    assert!(l >= 0, "{name} is active");
    l
}

const BVEC4: &str = "precision mediump float;\nuniform bvec4 b;\n\
    void main() { gl_FragColor = vec4(b) + vec4(b.x == b.y ? 1.0 : 0.0); }\n";

/// glUniform4i, true as any non-zero value, false as 0.
#[test]
fn a_bvec4_set_through_gl_uniform_4i_is_pushed_as_all_ones_or_zero() {
    let words = fs_push(BVEC4, &|p| unsafe { glUniform4i(loc(p, "b"), 1, 0, 7, -3) });
    assert!(words.windows(4).any(|w| w == [T, 0, T, T]), "{words:x?}");
    let words = fs_push(BVEC4, &|p| unsafe { glUniform4i(loc(p, "b"), 0, 0, 0, 0) });
    assert!(!words.contains(&T) && !words.contains(&1), "{words:x?}");
}

/// glUniform1f: any non-zero float is true (a negative one too).
#[test]
fn a_bool_set_through_gl_uniform_1f_or_1i_is_pushed_as_all_ones() {
    const FS: &str = "precision mediump float;\nuniform bool b;\n\
        void main() { gl_FragColor = vec4(b ? 1.0 : 0.0); }\n";
    for (set, name) in [
        (Box::new(|p| unsafe { glUniform1i(loc(p, "b"), 1) }) as Box<dyn Fn(GLuint)>, "1i 1"),
        (Box::new(|p| unsafe { glUniform1i(loc(p, "b"), 5) }), "1i 5"),
        (Box::new(|p| unsafe { glUniform1f(loc(p, "b"), 1.0) }), "1f 1.0"),
        (Box::new(|p| unsafe { glUniform1f(loc(p, "b"), -0.25) }), "1f -0.25"),
    ] {
        let words = fs_push(FS, &*set);
        assert!(words.contains(&T) && !words.contains(&1), "{name}: {words:x?}");
    }
    for (set, name) in [
        (Box::new(|p| unsafe { glUniform1i(loc(p, "b"), 0) }) as Box<dyn Fn(GLuint)>, "1i 0"),
        (Box::new(|p| unsafe { glUniform1f(loc(p, "b"), 0.0) }), "1f 0.0"),
    ] {
        let words = fs_push(FS, &*set);
        assert!(!words.contains(&T) && !words.contains(&1), "{name}: {words:x?}");
    }
}

/// bvec2 and bvec3.
#[test]
fn bvec2_and_bvec3_are_pushed_as_all_ones() {
    use crate::gl::program::{glUniform2i, glUniform3i};
    let fs2 = "precision mediump float;
uniform bvec2 b;
void main() { gl_FragColor = vec4(b, b.x == b.y ? 1.0 : 0.0, 1.0); }
";
    let w = fs_push(fs2, &|p| unsafe { glUniform2i(loc(p, "b"), 3, 0) });
    assert!(w.windows(2).any(|x| x == [T, 0]), "bvec2: {w:x?}");
    let fs3 = "precision mediump float;
uniform bvec3 b;
void main() { gl_FragColor = vec4(b, b.x == b.z ? 1.0 : 0.0); }
";
    let w = fs_push(fs3, &|p| unsafe { glUniform3i(loc(p, "b"), 0, 9, 1) });
    assert!(w.windows(3).any(|x| x == [0, T, T]), "bvec3: {w:x?}");
}

/// The conversion the draw applies, on a table of the shapes the compiler
/// does not yet link (a bool array, `deqp_unif2_arr_bool_array`: the online
/// compiler refuses a uniform array, so no program draws one here): the
/// bool rows' words become ~0 where non-zero, the float row and the padding
/// word between them are not touched, and a table with no bool returns the
/// words as they are.
#[test]
fn the_conversion_covers_a_bool_array_and_leaves_floats_alone() {
    use crate::shaderc::{ShcUniform, SHC_UNIFORM_NO_BLOCK};
    let row = |off: u16, dwords: u16, gl_type: u32, array_len: u8| ShcUniform {
        dword_off: off,
        dwords,
        gl_type: gl_type as u16,
        array_len,
        ..ShcUniform::ZERO
    };
    let table = [
        row(0, 3, GL_BOOL, 3),
        row(4, 1, GL_FLOAT, 1),
        row(5, 2, GL_BOOL_VEC2, 1),
        /* a struct member is a leaf row of its own ("s.on" of `struct S { bool on; }`) */
        row(8, 1, GL_BOOL, 1),
        row(SHC_UNIFORM_NO_BLOCK, 1, GL_BOOL, 1),
    ];
    let float = 1.0f32.to_bits();
    let words = [1, 0, 7, 0, float, 1, 1, 1, 9, 1];
    let mut buf = [0u32; program::UNIFORM_WORDS_MAX];
    let out = program::mali_uniform_words(&table, &words, &mut buf);
    assert_eq!(out, [T, 0, T, 0, float, T, T, 1, T, 1], "the word past the table is as it was");
    let mut buf = [0u32; program::UNIFORM_WORDS_MAX];
    let plain = [row(0, 1, GL_FLOAT, 1)];
    assert_eq!(program::mali_uniform_words(&plain, &words, &mut buf), words);
}
