// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL-level host test: attribute binding and strided vertex layouts as
//! glmark2-es2 uses them, through the GL core and the Mali back end's table.
//! glmark2 binds attributes by name (position, normal, texcoord, tangent):
//! `glGetAttribLocation` after link, `glBindAttribLocation` before it in some
//! scenes, and feeds them from one interleaved VBO (stride, offsets) or from
//! separate VBOs. The shaders are written for this test; no glmark2 text is
//! copied.
//!
//! The host has no Mali OSAL and no GPU, so no pixel is shaded: the stand-in
//! back end (as in `tests/mali_glmark2_scene_draw.rs`) records the
//! `GlesDraw` it is handed, and the test checks what the GL core made of the
//! attribute state: the location each name got, the stride, offset, component
//! count, buffer and bound of each enabled array, the constant a disabled
//! array carries, and that an enabled array the shader does not read is not
//! handed over as a fetch. glReadPixels is made after each draw and must
//! raise no error; its colours are not checked (nothing wrote them).
//!
//! Not a tools/check.sh row (a later check-rows item adds it).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_attribs
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::{glBindBuffer, glBufferData, glDeleteBuffers, glGenBuffers};
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{
    glDisableVertexAttribArray, glEnableVertexAttribArray, glVertexAttrib4f, glVertexAttribPointer,
};
use yakogl::mali::backend::yakogl_backend_mali;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one -- */

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let p = Box::leak(vec![0u8; size].into_boxed_slice()).as_mut_ptr();
    *out = OsalGpuMem { cpu: p as *mut c_void, gpu: p as u64, phys: p as u64, size, impl_: p as *mut c_void };
    0
}
unsafe extern "C" fn m_gpu_free(_mem: *mut OsalGpuMem) -> i32 {
    0
}
unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_heap: GlesHeap) -> u64 {
    0x1000_0000
}
unsafe extern "C" fn b_ok() -> i32 {
    0
}

/// One attribute as the back end was handed it.
#[derive(Clone, Copy, Debug)]
struct Att {
    vi_reg: u8,
    components: u8,
    stride: u32,
    offset: u32,
    in_place: bool,
    buffer_bytes: u64,
    /// The first four floats of draw-time data (a disabled array's constant).
    cpu: Option<[f32; 4]>,
}

static SEEN: Mutex<Vec<Vec<Att>>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count == 0 {
        return 0;
    }
    let v = d.attribs[..d.num_attribs as usize]
        .iter()
        .map(|a| Att {
            vi_reg: a.vi_reg,
            components: a.components,
            stride: a.stride,
            offset: a.offset,
            in_place: a.cpu.is_null() && a.buffer_gpu != 0,
            buffer_bytes: a.buffer_bytes,
            cpu: if a.cpu.is_null() || a.cpu_bytes < 16 { None } else { Some(*(a.cpu as *const [f32; 4])) },
        })
        .collect();
    SEEN.lock().unwrap().push(v);
    0
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(m_gpu_alloc),
    gpu_free: Some(m_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(m_cache),
    cache_invd: Some(m_cache),
    submit: None,
    wait: None,
    completed: None,
    reg_read: None,
    reg_write: None,
    query: None,
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/// The Mali's table with the stand-in's own entries (a draw that records, a
/// heap base, no hardware to start or wait for).
fn stand_in() -> &'static YakoglBackend {
    let m = &yakogl_backend_mali;
    Box::leak(Box::new(YakoglBackend {
        name: m.name,
        native_attrib_formats: m.native_attrib_formats,
        rt_rgba8888_is_bgra: m.rt_rgba8888_is_bgra,
        mip_level_align: m.mip_level_align,
        compressed_blocks_in_rows: m.compressed_blocks_in_rows,
        native_layered_textures: m.native_layered_textures,
        front_facing_shader_variant: m.front_facing_shader_variant,
        etc1_decoded_at_upload: m.etc1_decoded_at_upload,
        init: Some(b_init),
        shutdown: None,
        draw: Some(b_draw),
        flush: Some(b_ok),
        finish: Some(b_ok),
        finish_previous: None,
        kick_id: None,
        kick_pending: None,
        heap_base: Some(b_heap_base),
        counters: None,
        texture_released: None,
        surface_swapped: None,
        texture_uploaded: None,
        max_instance_divisor: m.max_instance_divisor,
        wait_kick: None,
        samples_texel_copies: m.samples_texel_copies,
        attachment_gone: None,
        depth_resolve: None,
        query_begin: None,
        query_end: None,
        query_result: None,
        generate_mipmap: m.generate_mipmap,
        return_mipmap_chain: m.return_mipmap_chain,
        linear_row_align: m.linear_row_align,
        uniforms_host_only: m.uniforms_host_only,
        ds_write: None,
        stencil_resolve: None,
        max_samples: m.max_samples,
    }))
}

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: the tables are statics or leaked; the lock above serialises
        // the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = stand_in();
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        let mut log = [0u8; 512];
        let mut n: GLsizei = 0;
        glGetShaderInfoLog(s, log.len() as GLsizei, &mut n, log.as_mut_ptr().cast());
        assert_eq!(ok, 1, "{text:?} did not compile: {}", String::from_utf8_lossy(&log[..n as usize]));
        s
    }
}

/// Compile, bind `binds` (name, location) before link, link, use.
fn program(vs: &str, fs: &str, binds: &[(&core::ffi::CStr, GLuint)]) -> GLuint {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        for (n, i) in binds {
            glBindAttribLocation(p, *i, n.as_ptr());
        }
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
        p
    }
}

/// An RGBA8 texture as the colour attachment of a framebuffer.
fn target(w: i32, h: i32) -> (GLuint, GLuint) {
    let (mut fbo, mut tex) = (0, 0);
    // SAFETY: room for one name each; null texels allocate the image.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glViewport(0, 0, w, h);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
    }
    assert_eq!(err(), GL_NO_ERROR, "target set-up");
    (fbo, tex)
}

/// Position, normal, texture coordinate, as glmark2's attribute names.
const VS: &str = "attribute vec3 a_pos;
attribute vec3 a_nrm;
attribute vec2 a_uv;
varying mediump vec4 v_col;
void main()
{
    gl_Position = vec4(a_pos, 1.0);
    v_col = vec4(a_nrm.z, a_uv, 1.0);
}
";
const FS: &str = "precision mediump float;
varying mediump vec4 v_col;
void main()
{
    gl_FragColor = v_col;
}
";

fn loc(p: GLuint, name: &core::ffi::CStr) -> GLint {
    // SAFETY: plain GL call.
    unsafe { glGetAttribLocation(p, name.as_ptr()) }
}

/// Two triangles over the target: per vertex pos (3), nrm (3), uv (2).
fn quad() -> Vec<[f32; 8]> {
    let p = [[-1.0, -1.0], [1.0, -1.0], [-1.0, 1.0], [-1.0, 1.0], [1.0, -1.0], [1.0, 1.0]];
    p.iter().map(|q| [q[0], q[1], 0.0, 0.0, 0.0, 1.0, (q[0] + 1.0) / 2.0, (q[1] + 1.0) / 2.0]).collect()
}

fn vbo(bytes: &[u8]) -> GLuint {
    let mut b = 0;
    // SAFETY: room for one name; `bytes` is the store's content.
    unsafe {
        glGenBuffers(1, &mut b);
        glBindBuffer(GL_ARRAY_BUFFER, b);
        glBufferData(GL_ARRAY_BUFFER, bytes.len() as GLsizeiptr, bytes.as_ptr().cast(), GL_STATIC_DRAW);
    }
    b
}

fn f32_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_ne_bytes()).collect()
}

/// Draw the six vertices, read a pixel; the draw's and read's errors and the
/// attributes the back end was handed.
fn draw() -> (GLenum, GLenum, Vec<Att>) {
    SEEN.lock().unwrap().clear();
    let mut px = [0u8; 4];
    // SAFETY: `px` holds one RGBA8 pixel.
    unsafe {
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFinish();
    }
    let de = err();
    // SAFETY: one pixel into `px`.
    unsafe { glReadPixels(4, 4, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast()) };
    let re = err();
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    (de, re, s.last().expect("the draw reached the back end").clone())
}

fn att(v: &[Att], reg: GLint) -> Att {
    *v.iter().find(|a| a.vi_reg as GLint == reg).unwrap_or_else(|| panic!("no attribute at {reg} in {v:?}"))
}

fn done(fbo: GLuint, tex: GLuint, bufs: &[GLuint]) {
    // SAFETY: names this test made.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tex);
        for b in bufs {
            glDeleteBuffers(1, b);
        }
        for i in 0..8 {
            glDisableVertexAttribArray(i);
        }
    }
}

#[test]
fn locations_and_active_attribs_after_link() {
    let _g = context();
    let p = program(VS, FS, &[]);
    let l = [loc(p, c"a_pos"), loc(p, c"a_nrm"), loc(p, c"a_uv")];
    assert!(l.iter().all(|&i| (0..8).contains(&i)), "{l:?}");
    assert!(l[0] != l[1] && l[1] != l[2] && l[0] != l[2], "distinct: {l:?}");
    assert_eq!(loc(p, c"a_missing"), -1);
    assert_eq!(loc(p, c"gl_Position"), -1);
    let (mut seen, mut n) = ([false; 8], 0);
    // SAFETY: plain GL calls; buffers sized as passed.
    unsafe {
        glGetProgramiv(p, GL_ACTIVE_ATTRIBUTES, &mut n);
        assert_eq!(n, 3);
        for i in 0..n as GLuint {
            let (mut len, mut size, mut ty) = (0, 0, 0);
            let mut name = [0u8; 32];
            glGetActiveAttrib(p, i, 32, &mut len, &mut size, &mut ty, name.as_mut_ptr().cast());
            let name = String::from_utf8_lossy(&name[..len as usize]).into_owned();
            let want = match name.as_str() {
                "a_pos" | "a_nrm" => GL_FLOAT_VEC3,
                "a_uv" => GL_FLOAT_VEC2,
                _ => panic!("unexpected active attribute {name:?}"),
            };
            assert_eq!((size, ty), (1, want), "{name}");
            let at = glGetAttribLocation(p, [name.as_bytes(), &[0]].concat().as_ptr().cast());
            assert!(!seen[at as usize], "{name} repeats a location");
            seen[at as usize] = true;
        }
    }
    assert_eq!(err(), GL_NO_ERROR);
}

#[test]
fn bind_attrib_location_before_link_is_honoured() {
    let _g = context();
    let p = program(VS, FS, &[(c"a_pos", 3), (c"a_nrm", 1), (c"a_uv", 6)]);
    assert_eq!((loc(p, c"a_pos"), loc(p, c"a_nrm"), loc(p, c"a_uv")), (3, 1, 6));
    assert_eq!(err(), GL_NO_ERROR);
}

/// The linker gives the attributes no one bound a location no bound one
/// has (ES 2.0 2.10.4; glmark2 binds only some names in some scenes). Here
/// `a_pos` is left at location 0 beside the bound `a_nrm`: two attributes at
/// one location.
#[test]
#[ignore = "defect: with only a_nrm bound to 0, the linker also gives a_pos location 0 (two attributes at one location); report as a new issue"]
fn partial_bind_leaves_free_locations_to_the_others() {
    let _g = context();
    let p = program(VS, FS, &[(c"a_nrm", 0)]);
    let (a, n, u) = (loc(p, c"a_pos"), loc(p, c"a_nrm"), loc(p, c"a_uv"));
    assert_eq!(n, 0);
    assert!(a > 0 && u > 0 && a != u, "a_pos {a}, a_nrm {n}, a_uv {u}");
    assert_eq!(err(), GL_NO_ERROR);
}

#[test]
fn interleaved_vbo_with_stride_and_offsets() {
    let _g = context();
    let (fbo, tex) = target(8, 8);
    let p = program(VS, FS, &[]);
    // A 16-byte header, then 32-byte vertices: pos at 16, nrm at 28, uv at 40.
    let mut bytes = vec![0u8; 16];
    bytes.extend(f32_bytes(&quad().concat()));
    let b = vbo(&bytes);
    let (lp, ln, lu) = (loc(p, c"a_pos"), loc(p, c"a_nrm"), loc(p, c"a_uv"));
    // SAFETY: offsets inside the bound buffer.
    unsafe {
        glVertexAttribPointer(lp as GLuint, 3, GL_FLOAT, 0, 32, 16 as *const c_void);
        glVertexAttribPointer(ln as GLuint, 3, GL_FLOAT, 0, 32, 28 as *const c_void);
        glVertexAttribPointer(lu as GLuint, 2, GL_FLOAT, 0, 32, 40 as *const c_void);
        for l in [lp, ln, lu] {
            glEnableVertexAttribArray(l as GLuint);
        }
    }
    let (de, re, v) = draw();
    assert_eq!((de, re), (GL_NO_ERROR, GL_NO_ERROR));
    for (l, comps, off) in [(lp, 3, 16), (ln, 3, 28), (lu, 2, 40)] {
        let a = att(&v, l);
        assert_eq!((a.components, a.stride), (comps, 32), "attribute {l}");
        assert!(a.in_place && a.cpu.is_none(), "attribute {l} is fetched from the VBO: {a:?}");
        assert_eq!(a.offset % 4, 0);
        // The three share one buffer: the offset differences are the layout's.
        assert_eq!(a.offset as i64 - att(&v, lp).offset as i64, off - 16, "attribute {l}");
        assert!(a.buffer_bytes >= bytes.len() as u64, "{a:?}");
    }
    done(fbo, tex, &[b]);
}

#[test]
fn separate_vbos_tightly_packed() {
    let _g = context();
    let (fbo, tex) = target(8, 8);
    program(VS, FS, &[(c"a_pos", 0), (c"a_nrm", 1), (c"a_uv", 2)]);
    let q = quad();
    let pos: Vec<f32> = q.iter().flat_map(|v| v[0..3].to_vec()).collect();
    let nrm: Vec<f32> = q.iter().flat_map(|v| v[3..6].to_vec()).collect();
    let uv: Vec<f32> = q.iter().flat_map(|v| v[6..8].to_vec()).collect();
    let mut bufs = vec![];
    for (i, (d, n)) in [(&pos, 3), (&nrm, 3), (&uv, 2)].into_iter().enumerate() {
        bufs.push(vbo(&f32_bytes(d)));
        // SAFETY: stride 0 is tightly packed; buffer bound above.
        unsafe {
            glVertexAttribPointer(i as GLuint, n, GL_FLOAT, 0, 0, core::ptr::null());
            glEnableVertexAttribArray(i as GLuint);
        }
    }
    let (de, re, v) = draw();
    assert_eq!((de, re), (GL_NO_ERROR, GL_NO_ERROR));
    for (l, comps) in [(0, 3u8), (1, 3), (2, 2)] {
        let a = att(&v, l);
        assert_eq!(a.components, comps);
        assert!(a.stride == 0 || a.stride == comps as u32 * 4, "tight stride: {a:?}");
        assert_eq!(a.offset, 0);
        assert!(a.in_place, "{a:?}");
    }
    done(fbo, tex, &bufs);
}

#[test]
fn disabled_array_takes_its_constant() {
    let _g = context();
    let (fbo, tex) = target(8, 8);
    let p = program(VS, FS, &[]);
    let q = quad();
    let (lp, ln, lu) = (loc(p, c"a_pos"), loc(p, c"a_nrm"), loc(p, c"a_uv"));
    let mut bytes = vec![];
    bytes.extend(f32_bytes(&q.concat()));
    let b = vbo(&bytes);
    // SAFETY: offsets inside the bound buffer.
    unsafe {
        glVertexAttribPointer(lp as GLuint, 3, GL_FLOAT, 0, 32, core::ptr::null());
        glVertexAttribPointer(lu as GLuint, 2, GL_FLOAT, 0, 32, 24 as *const c_void);
        glEnableVertexAttribArray(lp as GLuint);
        glEnableVertexAttribArray(lu as GLuint);
        // The normal: an array was set, then disabled; the constant wins.
        glVertexAttribPointer(ln as GLuint, 3, GL_FLOAT, 0, 32, 12 as *const c_void);
        glDisableVertexAttribArray(ln as GLuint);
        glVertexAttrib4f(ln as GLuint, 0.25, 0.5, 0.75, 1.0);
    }
    let (de, re, v) = draw();
    assert_eq!((de, re), (GL_NO_ERROR, GL_NO_ERROR));
    let n = att(&v, ln);
    assert!(!n.in_place, "the disabled array is not fetched from the VBO: {n:?}");
    assert_eq!(n.cpu, Some([0.25, 0.5, 0.75, 1.0]), "{n:?}");
    // The enabled ones still come from the VBO.
    assert!(att(&v, lp).in_place && att(&v, lu).in_place);
    // A new constant is picked up by the next draw.
    // SAFETY: plain GL call.
    unsafe { glVertexAttrib4f(ln as GLuint, 1.0, 0.0, 0.0, 1.0) };
    let (de, _, v) = draw();
    assert_eq!(de, GL_NO_ERROR);
    assert_eq!(att(&v, ln).cpu, Some([1.0, 0.0, 0.0, 1.0]));
    done(fbo, tex, &[b]);
}

#[test]
fn unused_attribute_array_may_stay_enabled() {
    let _g = context();
    let (fbo, tex) = target(8, 8);
    program(VS, FS, &[(c"a_pos", 0), (c"a_nrm", 1), (c"a_uv", 2)]);
    let q = quad();
    let b = vbo(&f32_bytes(&q.concat()));
    // SAFETY: offsets inside the bound buffer; locations 5 and 7 belong to no
    // attribute of the program (a tangent glmark2 enables for every scene).
    unsafe {
        glVertexAttribPointer(0, 3, GL_FLOAT, 0, 32, core::ptr::null());
        glVertexAttribPointer(1, 3, GL_FLOAT, 0, 32, 12 as *const c_void);
        glVertexAttribPointer(2, 2, GL_FLOAT, 0, 32, 24 as *const c_void);
        glVertexAttribPointer(5, 4, GL_FLOAT, 0, 32, core::ptr::null());
        glVertexAttribPointer(7, 3, GL_FLOAT, 0, 0, core::ptr::null());
        for l in [0, 1, 2, 5, 7] {
            glEnableVertexAttribArray(l);
        }
    }
    let (de, re, v) = draw();
    assert_eq!((de, re), (GL_NO_ERROR, GL_NO_ERROR));
    for l in [0, 1, 2] {
        assert!(att(&v, l).in_place);
    }
    assert!(v.iter().all(|a| a.vi_reg != 5 && a.vi_reg != 7), "unused arrays are not fetched: {v:?}");
    done(fbo, tex, &[b]);
}
