// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! P10 through the V3D 7.1 table (src/v3d/table71.rs, src/v3d/table71tex.rs):
//! textured draws and `glGenerateMipmap` by the TFU, driven through the GL
//! core's entry points with `yakogl_backend_v3d71` as `g_backend`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d71,shaderc-online,no-shader-table --test v3d71_table_tex
//! ```
//!
//! The OSAL and the C builders are tests/v3d71_table_rig/mod.rs's (the real
//! driver/src/v3d/v3d_job71.c, v3d_tex71.c, v3d_tfu71.c and Mesa's tiler;
//! `osal_v3d71_tfu` is a mock that keeps the job). Checked:
//!
//! * a 2D texture is sampled through a tiled copy: the job's texture and
//!   sampler records are what texture71 / sampler71 compose for the GL
//!   description at the copy's address, the address-free words of both equal
//!   Mesa's `gl_tex2d` job 1 (tools/v3d/pi5/README.md, the record table), and
//!   the copy untiled is the texels uploaded;
//! * the copy cache never frees a copy a draw can still sample: with
//!   `MAX_COPIES` copies live, a draw that binds the oldest copy and a new
//!   texture keeps the first (the review of bb472dc4), and a draw queued
//!   before an eviction still finds its copy allocated when its job is
//!   submitted; dead copies are freed once nothing runs;
//! * `glGenerateMipmap` of a 64x64 RGBA8 texture is one TFU job whose
//!   registers are Mesa's `gl_mipgen` job (tools/v3d/pi5/gl_mipgen.stdout:
//!   ICFG, IIS, IOS, IOC 0x00086061, the coefficients; IIA = IOA = the
//!   copy's level 0), the chain is the backend's (`MipChain::Whole`) and a
//!   mipmapped draw samples that copy; a failed TFU job leaves the chain to
//!   the CPU;
//! * a cube map is sampled seamlessly (U8: the record has no seamless bit,
//!   the sampler's wraps are CLAMP_TO_EDGE as Mesa's state tracker forces),
//!   and an incomplete texture samples the table's black stand-in.
#![cfg(all(feature = "gpu-v3d71", feature = "host-test", feature = "shaderc-online"))]

mod v3d71_table_rig;

use std::ffi::c_void;

use v3d71_table_rig::{is_live, kicks, live_allocs, log_lines, logs_with, rig, set_tfu_rc, tfu_jobs, Rig};
use yakogl::backend;
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::context::{gl_context_create, gl_context_destroy, GlContext};
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::v3d::table71::{self, REFUSAL_PREFIX};
use yakogl::v3d::table71tex::MAX_COPIES;
use yakogl::v3d::{texfill71, texglue71, texture71};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");
const W: GLsizei = 64;

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/* ---- Mesa's references ---------------------------------------------------------------- */

fn hex(s: &str) -> u32 {
    let s = s.trim().trim_start_matches("0x");
    u32::from_str_radix(s, 16).unwrap_or_else(|_| panic!("not a hex word: {s}"))
}

/// The texture and sampler record words of `shape` job `job` in
/// tools/v3d/pi5/README.md's record table.
fn readme_records(shape: &str, job: u32) -> ([u32; 6], [u32; 6]) {
    let text = read("tools/v3d/pi5/README.md");
    let head = format!("| `{shape}` {job} |");
    let row = text.lines().find(|l| l.starts_with(&head)).unwrap_or_else(|| panic!("no README row {head}"));
    let cells: Vec<&str> = row.split('|').map(str::trim).collect();
    let words = |c: &str| -> [u32; 6] {
        let w: Vec<u32> = c.trim_matches('`').split_whitespace().map(hex).collect();
        w.try_into().unwrap_or_else(|_| panic!("{shape} {job}: not six words: {c}"))
    };
    (words(cells[2]), words(cells[4]))
}

/// The registers of gl_mipgen's SUBMIT_TFU line.
fn mipgen_tfu() -> Vec<(String, u32)> {
    let text = read("tools/v3d/pi5/gl_mipgen.stdout");
    let line = text.lines().find(|l| l.contains("SUBMIT_TFU icfg")).expect("gl_mipgen's SUBMIT_TFU");
    let mut it = line.split_whitespace().skip_while(|w| *w != "icfg");
    let mut out = vec![];
    while let Some(name) = it.next() {
        if name == "bo_handles" {
            break;
        }
        if name == "coef" {
            for k in 0..4 {
                out.push((format!("coef{k}"), hex(it.next().unwrap())));
            }
        } else {
            out.push((name.to_string(), hex(it.next().unwrap())));
        }
    }
    out
}

fn words_of(b: &[u8]) -> [u32; 6] {
    core::array::from_fn(|i| u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap()))
}

/* ---- the GL side ---------------------------------------------------------------------- */

const VS: &str = "attribute vec4 a_pos;\nattribute vec2 a_uv;\nvarying vec2 v_uv;\n\
                  void main() { gl_Position = a_pos; v_uv = a_uv; }\n\0";
const FS1: &str = "precision mediump float;\nuniform sampler2D u_a;\nvarying vec2 v_uv;\n\
                   void main() { gl_FragColor = texture2D(u_a, v_uv); }\n\0";
const FS2: &str = "precision mediump float;\nuniform sampler2D u_a;\nuniform sampler2D u_b;\nvarying vec2 v_uv;\n\
                   void main() { gl_FragColor = texture2D(u_a, v_uv) + texture2D(u_b, v_uv); }\n\0";
const FS_CUBE: &str = "precision mediump float;\nuniform samplerCube u_a;\nvarying vec2 v_uv;\n\
                       void main() { gl_FragColor = textureCube(u_a, vec3(v_uv, 1.0)); }\n\0";

struct Gl {
    ctx: *mut GlContext,
    /// The texture record arena (the table's fifth allocation at init).
    recs: u32,
    _rig: Rig,
}

impl Gl {
    fn new() -> Gl {
        let rig = rig();
        assert_eq!(backend::init(), 0, "{:?}", log_lines());
        let allocs = live_allocs();
        assert_eq!(allocs.len(), 5, "the job arenas");
        assert_eq!(allocs[4].1, table71::RECS_BYTES);
        // SAFETY: the OSAL and the table are installed; the rig's lock makes
        // this the only context.
        let ctx = unsafe { gl_context_create(W as u32, W as u32) };
        assert!(!ctx.is_null());
        Gl { ctx, recs: allocs[4].0, _rig: rig }
    }
}

impl Drop for Gl {
    fn drop(&mut self) {
        // SAFETY: the context `new` made.
        unsafe { gl_context_destroy(self.ctx) };
    }
}

/// Compile and link `fs` with [`VS`]; the samplers named are bound to
/// units 0, 1, ...
unsafe fn program(fs: &str, samplers: &[&str]) -> GLuint {
    let compile = |ty: GLenum, src: &str| {
        let s = glCreateShader(ty);
        let p = src.as_ptr() as *const GLchar;
        glShaderSource(s, 1, &p, core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_ne!(ok, 0, "shader {ty:#x} did not compile: {:?}", log_lines());
        s
    };
    let prog = glCreateProgram();
    glAttachShader(prog, compile(GL_VERTEX_SHADER, VS));
    glAttachShader(prog, compile(GL_FRAGMENT_SHADER, fs));
    glBindAttribLocation(prog, 0, c"a_pos".as_ptr());
    glBindAttribLocation(prog, 1, c"a_uv".as_ptr());
    glLinkProgram(prog);
    let mut ok: GLint = 0;
    glGetProgramiv(prog, GL_LINK_STATUS, &mut ok);
    assert_ne!(ok, 0, "the program did not link: {:?}", log_lines());
    glUseProgram(prog);
    for (unit, name) in samplers.iter().enumerate() {
        let n = std::ffi::CString::new(*name).unwrap();
        glUniform1i(glGetUniformLocation(prog, n.as_ptr()), unit as GLint);
    }
    prog
}

/// A full-screen quad: position, uv.
unsafe fn quad() {
    let v: [[f32; 6]; 6] = [
        [-1.0, -1.0, 0.0, 1.0, 0.0, 0.0],
        [1.0, -1.0, 0.0, 1.0, 1.0, 0.0],
        [1.0, 1.0, 0.0, 1.0, 1.0, 1.0],
        [-1.0, -1.0, 0.0, 1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0, 1.0, 1.0, 1.0],
        [-1.0, 1.0, 0.0, 1.0, 0.0, 1.0],
    ];
    let mut vbo: GLuint = 0;
    glGenBuffers(1, &mut vbo);
    glBindBuffer(GL_ARRAY_BUFFER, vbo);
    glBufferData(
        GL_ARRAY_BUFFER,
        core::mem::size_of_val(&v) as GLsizeiptr,
        v.as_ptr() as *const c_void,
        GL_STATIC_DRAW,
    );
    glVertexAttribPointer(0, 4, GL_FLOAT, 0, 24, core::ptr::null());
    glVertexAttribPointer(1, 2, GL_FLOAT, 0, 24, 16usize as *const c_void);
    glEnableVertexAttribArray(0);
    glEnableVertexAttribArray(1);
}

/// Texels of texture number `n`: every byte says which texture and texel.
fn texels(n: u32, w: u32, h: u32) -> Vec<u8> {
    (0..w * h).flat_map(|i| [(i & 0xff) as u8, (i >> 8) as u8, n as u8, (n >> 8) as u8 | 0x80]).collect()
}

/// A `w` x `h` RGBA8 2D texture on the active unit with `texels(n)`,
/// filtered `min`/`mag`.
unsafe fn tex2d(n: u32, w: GLsizei, h: GLsizei, filter: GLenum) -> GLuint {
    let mut t: GLuint = 0;
    glGenTextures(1, &mut t);
    glBindTexture(GL_TEXTURE_2D, t);
    let px = texels(n, w as u32, h as u32);
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, px.as_ptr() as *const c_void);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, filter as GLint);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
    t
}

fn no_errors(what: &str) {
    // SAFETY: a plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: {:?}", log_lines());
    assert!(logs_with(REFUSAL_PREFIX).is_empty(), "{what}: {:?}", logs_with(REFUSAL_PREFIX));
}

/// Every copy a submitted job samples was allocated when it was submitted.
fn sampled_copies_were_live() -> usize {
    let mut n = 0;
    for (k, kick) in kicks().iter().enumerate() {
        for gpu in table71::sampled_by(k as u32 + 1).unwrap_or_default() {
            assert!(
                kick.live.iter().any(|&(a, len)| gpu >= a && ((gpu - a) as usize) < len),
                "job {} samples the copy at {gpu:#x}, which was freed before it was submitted",
                k + 1
            );
            n += 1;
        }
    }
    n
}

/* ---- the tests -------------------------------------------------------------------------- */

#[test]
fn a_2d_texture_is_sampled_through_its_tiled_copy() {
    let g = Gl::new();
    // SAFETY: the context is current.
    unsafe {
        program(FS1, &["u_a"]);
        quad();
        tex2d(7, W, W, GL_NEAREST);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFinish();
    }
    no_errors("the textured draw");
    let ks = kicks();
    assert_eq!(ks.len(), 1);
    let copies = table71::texture_copies();
    assert_eq!(copies.len(), 1, "{copies:?}");
    let copy = copies[0];
    assert!(!copy.dead && !copy.chain);
    assert_eq!(table71::sampled_by(1), Some(vec![copy.gpu]));
    assert_eq!(sampled_copies_were_live(), 1);

    /* the records: what texture71 and sampler71 compose at the copy's
     * address, packed by v3d_tex71.c */
    let t = texglue71::GlTexBind71::tex2d(texglue71::GLES_TEXFMT_RGBA8, 64, 64, 1, copy.gpu);
    let s = texglue71::GlSampler71::filters(false, false, 0);
    let glued = texglue71::glue(&t, &s, 71, 10).unwrap();
    let f = texfill71::fill(&glued.tex, &glued.sampler).unwrap();
    let (mut tex, mut smp) = ([0u8; 24], [0u8; 24]);
    // SAFETY: 24-byte outputs, live structs.
    unsafe {
        assert!(v3d71_table_rig::v3d_tex71_pack_state(tex.as_mut_ptr(), &f.state));
        assert!(v3d71_table_rig::v3d_tex71_pack_sampler(smp.as_mut_ptr(), &f.sampler));
    }
    let job = &ks[0];
    assert_eq!(job.at(g.recs, 24), tex, "the texture record at the arena's start");
    assert_eq!(job.at(g.recs + 24, 24), smp, "the sampler record after it");
    /* Mesa's gl_tex2d job 1: the words that hold no address, and the
     * sampler record whole */
    let (mesa_tex, mesa_smp) = readme_records("gl_tex2d", 1);
    let ours = words_of(&tex);
    assert_eq!(ours[1..3], mesa_tex[1..3], "array stride, size, levels, swizzle");
    assert_eq!(ours[0] & 0x3f, mesa_tex[0] & 0x3f, "the flag bits below the base address");
    assert_eq!(ours[0] & !0x3f, copy.gpu, "the base address is the copy's level 0");
    assert_eq!(words_of(&smp), mesa_smp, "the sampler record");

    /* the copy untiled is the uploaded texels */
    let layout = texture71::layout(64, 64, 1, 4, 1).unwrap();
    let image = job.at(copy.gpu, layout.size as usize).to_vec();
    let mut back = vec![0u8; 64 * 64 * 4];
    let s0 = layout.slices[0];
    let b: [i32; 5] = [0, 64, 0, 64, 1 << 16];
    // SAFETY: the level's bytes and a raster buffer of 64 rows of 256 bytes.
    unsafe {
        v3d71_table_rig::v3d_load_tiled_image(
            back.as_mut_ptr() as *mut c_void,
            256,
            image.as_ptr().add(s0.offset as usize) as *mut c_void,
            s0.stride,
            s0.tiling as i32,
            4,
            s0.padded_height,
            b.as_ptr() as *const c_void,
        )
    };
    assert_eq!(back, texels(7, 64, 64), "the copy holds the texels, tiled");
    drop(g);
    assert!(!is_live(copy.gpu), "shutdown frees the copies too");
}

/// The review of bb472dc4: with the cache full, a draw that binds the
/// oldest copy and a texture without one must not lose the first to the
/// second's eviction; and a draw queued before an eviction keeps its copy
/// until its job has run.
#[test]
fn a_full_cache_never_frees_a_copy_a_draw_can_still_sample() {
    let _g = Gl::new();
    let n = MAX_COPIES as u32;
    // SAFETY: the context is current.
    let names: Vec<GLuint> = unsafe {
        program(FS2, &["u_a", "u_b"]);
        quad();
        /* MAX_COPIES textures, each sampled by one draw (on both units) */
        let names: Vec<GLuint> = (0..n).map(|i| tex2d(i, 4, 4, GL_NEAREST)).collect();
        for &t in &names {
            for unit in [GL_TEXTURE0, GL_TEXTURE1] {
                glActiveTexture(unit);
                glBindTexture(GL_TEXTURE_2D, t);
            }
            glDrawArrays(GL_TRIANGLES, 0, 6);
        }
        glFinish();
        names
    };
    no_errors("the draws that fill the cache");
    let copies = table71::texture_copies();
    assert_eq!(copies.iter().filter(|c| !c.dead).count(), MAX_COPIES, "{copies:?}");
    let oldest = copies[0];

    /* a draw binding the oldest copy (unit 0) and a new texture (unit 1) */
    // SAFETY: the context is current.
    unsafe {
        glActiveTexture(GL_TEXTURE1);
        tex2d(1000, 4, 4, GL_NEAREST);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, names[0]);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        /* a queued draw, then more new textures than one eviction: each
         * new copy evicts one the queue may still name */
        for i in 0..4 {
            glActiveTexture(GL_TEXTURE0);
            tex2d(2000 + i, 4, 4, GL_NEAREST);
            glActiveTexture(GL_TEXTURE1);
            glBindTexture(GL_TEXTURE_2D, names[1 + i as usize]);
            glDrawArrays(GL_TRIANGLES, 0, 6);
        }
        let queued = table71::texture_copies();
        assert!(queued.iter().any(|c| c.dead), "an eviction happened while draws were queued");
        for c in queued.iter().filter(|c| c.dead) {
            assert!(is_live(c.gpu), "a dead copy is not freed while draws are queued: {c:?}");
        }
        glFinish();
    }
    no_errors("the draws after the cache is full");
    let k = kicks().len() as u32;
    let sampled = table71::sampled_by(k).expect("the last job's copies");
    assert!(sampled.contains(&oldest.gpu), "the oldest copy was bound again, not replaced");
    assert!(sampled_copies_were_live() >= 2 * MAX_COPIES + 10);
    /* nothing runs any more: the dead are freed, the live stay */
    let left = table71::texture_copies();
    assert!(left.iter().all(|c| !c.dead), "{left:?}");
    assert!(left.len() <= MAX_COPIES);
    assert!(left.iter().any(|c| c.gpu == oldest.gpu));
}

#[test]
fn generate_mipmap_runs_mesas_tfu_job_and_the_draw_samples_its_chain() {
    let g = Gl::new();
    // SAFETY: the context is current.
    unsafe {
        program(FS1, &["u_a"]);
        quad();
        tex2d(3, W, W, GL_NEAREST_MIPMAP_NEAREST);
        glGenerateMipmap(GL_TEXTURE_2D);
    }
    no_errors("glGenerateMipmap");
    let jobs = tfu_jobs();
    assert_eq!(jobs.len(), 1, "one TFU job makes the chain: {:?}", log_lines());
    let copies = table71::texture_copies();
    assert_eq!(copies.len(), 1);
    assert!(copies[0].chain, "the chain is the copy's");
    let j = jobs[0];
    let ours = [
        ("icfg", j.icfg),
        ("iia", j.iia),
        ("iis", j.iis),
        ("ica", j.ica),
        ("iua", j.iua),
        ("ioa", j.ioa),
        ("ios", j.ios),
        ("coef0", j.coef[0]),
        ("coef1", j.coef[1]),
        ("coef2", j.coef[2]),
        ("coef3", j.coef[3]),
        ("ioc", j.ioc),
    ];
    let layout = texture71::layout(64, 64, 7, 4, 1).unwrap();
    let level0 = copies[0].gpu + layout.slices[0].offset;
    let mesa = mipgen_tfu();
    assert_eq!(mesa.len(), ours.len());
    let mesa_l0 = mesa.iter().find(|m| m.0 == "iia").unwrap().1;
    for ((name, v), (mname, mv)) in ours.iter().zip(&mesa) {
        assert_eq!(*name, mname);
        match *name {
            /* addresses: level 0 of the copy, Mesa's of its BO */
            "iia" | "ioa" => {
                assert_eq!(*v, level0, "{name}");
                assert_eq!(*mv, mesa_l0, "Mesa's {name}");
            }
            _ => assert_eq!(v, mv, "{name}"),
        }
    }
    assert_eq!(j.ioc, 0x0008_6061, "DIMTW 1, NUMMM 6, FORMAT 6, STRIDE 8");

    /* a mipmapped draw samples the chain's copy: no new copy */
    // SAFETY: the context is current.
    unsafe {
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFinish();
    }
    no_errors("the mipmapped draw");
    let after = table71::texture_copies();
    assert_eq!(after.len(), 1, "{after:?}");
    assert_eq!(table71::sampled_by(1), Some(vec![after[0].gpu]));
    assert_eq!(table71::tfu_counts(), (1, 0));
    let rec = words_of(kicks()[0].at(g.recs, 24));
    assert_eq!(rec[0] & !0x3f, level0, "the record's base is the chain's level 0 (v3d_layer_offset)");

    /* a CPU write of the texture takes the chain back first */
    // SAFETY: the context is current.
    unsafe {
        let px = texels(9, 1, 1);
        glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_ptr() as *const c_void);
    }
    no_errors("glTexSubImage2D");
    assert!(table71::texture_copies().iter().all(|c| !c.chain), "the chain went back to the storage");
    drop(g);
}

#[test]
fn a_failed_tfu_job_leaves_the_chain_to_the_cpu() {
    let _g = Gl::new();
    set_tfu_rc(-5);
    // SAFETY: the context is current.
    unsafe {
        tex2d(3, W, W, GL_NEAREST_MIPMAP_NEAREST);
        glGenerateMipmap(GL_TEXTURE_2D);
    }
    no_errors("glGenerateMipmap");
    assert_eq!(tfu_jobs().len(), 1);
    assert_eq!(table71::tfu_counts(), (1, 1));
    assert_eq!(logs_with("glGenerateMipmap left to the CPU: the TFU job failed (-5").len(), 1, "{:?}", log_lines());
    assert!(table71::texture_copies().iter().all(|c| !c.chain));
}

#[test]
fn a_cube_map_is_sampled_seamlessly() {
    let g = Gl::new();
    // SAFETY: the context is current.
    unsafe {
        program(FS_CUBE, &["u_a"]);
        quad();
        let mut t: GLuint = 0;
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_CUBE_MAP, t);
        for f in 0..6 {
            let px = texels(f, 16, 16);
            glTexImage2D(
                GL_TEXTURE_CUBE_MAP_POSITIVE_X + f,
                0,
                GL_RGBA as GLint,
                16,
                16,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                px.as_ptr() as *const c_void,
            );
        }
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, GL_LINEAR as GLint);
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFinish();
    }
    no_errors("the cube map draw");
    let copies = table71::texture_copies();
    assert_eq!(copies.len(), 1);
    let layout = texture71::layout(16, 16, 1, 4, 6).unwrap();
    assert_eq!(copies[0].bytes, layout.size as usize, "six faces");
    /* the sampler: wrap S, T, R CLAMP_TO_EDGE (texfill71's cube rule) */
    let t = texglue71::GlTexBind71::cube(texglue71::GLES_TEXFMT_RGBA8, 16, 1, copies[0].gpu);
    let s = texglue71::GlSampler71::filters(true, true, 0);
    let glued = texglue71::glue(&t, &s, 71, 10).unwrap();
    let f = texfill71::fill(&glued.tex, &glued.sampler).unwrap();
    let mut smp = [0u8; 24];
    // SAFETY: a 24-byte output, a live struct.
    unsafe { assert!(v3d71_table_rig::v3d_tex71_pack_sampler(smp.as_mut_ptr(), &f.sampler)) };
    assert_eq!(kicks()[0].at(g.recs + 24, 24), smp);
    assert_eq!(f.sampler.wrap_s, f.sampler.wrap_r);
}

#[test]
fn an_incomplete_texture_samples_the_black_stand_in() {
    let _g = Gl::new();
    // SAFETY: the context is current.
    unsafe {
        program(FS1, &["u_a"]);
        quad();
        /* the default min filter wants mipmaps the texture does not have */
        let mut t: GLuint = 0;
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        let px = texels(1, 8, 8);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGBA as GLint,
            8,
            8,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            px.as_ptr() as *const c_void,
        );
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFinish();
    }
    no_errors("the draw of an incomplete texture");
    let copies = table71::texture_copies();
    assert_eq!(copies.len(), 1);
    assert_eq!(copies[0].base_gpu, 0, "the stand-in, not the texture");
    let layout = texture71::layout(1, 1, 1, 4, 1).unwrap();
    let k = &kicks()[0];
    assert_eq!(&k.at(copies[0].gpu + layout.slices[0].offset, 4), &[0, 0, 0, 255], "(0, 0, 0, 1)");
}
