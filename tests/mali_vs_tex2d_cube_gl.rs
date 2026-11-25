// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The vertex stage's sampler and texture descriptors of a vertex shader
//! that reads a sampler2D (unit 0) and a samplerCube (unit 1), drawn through
//! the GL core on the real Mali back end over a mock of its OSAL, equal what
//! Mesa 26.2.1 sets for the same draw (item vs2tex-gl, plan #406, issue
//! #11308; `tests/mali_vs_tex2d_cube.rs` pins the reference).
//!
//! The reference is `tools/mali/testdata/vs_tex2d_cube.{vert,frag,trace}`:
//! gl_ref.py's textures (a 2x2 RGBA8 texture of one level and a 16x16 RGBA8
//! cube map of five levels, both GL_NEAREST with GL_CLAMP_TO_EDGE, the cube
//! under GL_NEAREST_MIPMAP_NEAREST) and the vertex stage's resource table
//! (entry 3 the two samplers, entry 4 the two textures, the 2D one first).
//! The expected fields are the trace's (`trace_descriptors` of the
//! reference test); Mesa's context is ES 3, so cube maps are seamless:
//!
//! - samplers: wraps s and t clamp-to-edge, r REPEAT (2D) and clamp (cube,
//!   seamless), minify and magnify nearest, mipmap mode None (2D) and
//!   Nearest (cube), Seamless Cube Map set, LOD range 0 to 8191/256.
//! - textures: Dimension 2D and Cube, format RGBA8 UNORM RGBA (0xbb), 2x2
//!   and 16x16, Levels 1 and 5, Maximum LOD 0 and 4, Array size 1; the
//!   cube's 30 planes, one a face and level.
//!
//! Not asserted, because ours differs by design: `Surfaces` (an address),
//! `Texel interleave` and the plane type (our storage is linear, Mesa's is
//! AFBC on the board), and the fragment stage's table.
//!
//! The compiler's part: the reference's plain-uniform text is refused by
//! the online compiler at the tip (`tests/mali_vs_tex2d_cube.rs`,
//! `compiler_pinned`), which is a compiler refusal and not the back end's.
//! The test draws the reference text first and, when that is refused or
//! does not link, dEQP's own struct text (`vs_tex2d_cube_deqp.vert`, the
//! samplers set through `u_var.m0` and `u_var.m1`); the log says which was
//! built. Either draw must be built by the back end (no refusal).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_vs_tex2d_cube_gl
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali"))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use yakogl::backend;
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glDisableVertexAttribArray, glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::mali::genxml::v10::{
    DescriptorType, GenericPlane, MipmapMode, Sampler, Texture, TextureDimension, WrapMode,
};
use yakogl::osal::mali::{OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP, OsalMaliGpu, OsalMaliHeap};
use yakogl::osal::{OsalGpuMem, OsalOps, osal_set_ops};
use yakogl::util::ENOTSUP;

/* ---- the Mali over a mock OSAL ------------------------------------------ */

/// The OSAL's answers: the board's (panthor-frame on the T6).
const GPU: OsalMaliGpu = OsalMaliGpu::T6;
const HEAP: OsalMaliHeap =
    OsalMaliHeap { ctx_gpu: 0x8000_0001_5000, first_chunk_gpu: 0x8000_00a0_0000, chunk_size: 2 << 20, reserved: 0 };

/// Buffers made so far: each one's GPU VA is its number times 16 MiB, far
/// from any CPU address. Nothing is given back (a few MiB for the run). Each
/// one's CPU address and size are kept so the test can read them.
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static FENCES: AtomicI64 = AtomicI64::new(0);
static BUFFERS: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

unsafe extern "C" fn m_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let n = ALLOCS.fetch_add(1, Ordering::Relaxed) + 1;
    assert!(size <= 16 << 20);
    let cpu = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice()).as_mut_ptr().cast::<c_void>();
    BUFFERS.lock().unwrap().push((cpu as usize, size));
    *out = OsalGpuMem { cpu, gpu: 0x40_0000_0000 + n * (16 << 20), phys: 0, size, impl_: n as *mut c_void };
    0
}
unsafe extern "C" fn m_free(mem: *mut OsalGpuMem) -> i32 {
    *mem = OsalGpuMem::ZERO;
    0
}
unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn m_init(_arg: *mut c_void) -> i32 {
    0
}
unsafe extern "C" fn m_submit(_kick: *const c_void, _len: usize) -> i64 {
    FENCES.fetch_add(1, Ordering::Relaxed) + 1
}
unsafe extern "C" fn m_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}
unsafe extern "C" fn m_completed() -> i64 {
    FENCES.load(Ordering::Relaxed)
}
unsafe extern "C" fn m_query(what: u32, out: *mut c_void, sz: usize) -> i32 {
    match what {
        OSAL_QUERY_MALI_GPU if sz == core::mem::size_of::<OsalMaliGpu>() => *out.cast::<OsalMaliGpu>() = GPU,
        OSAL_QUERY_MALI_TILER_HEAP if sz == core::mem::size_of::<OsalMaliHeap>() => *out.cast::<OsalMaliHeap>() = HEAP,
        _ => return -ENOTSUP,
    }
    0
}

/// The OSAL's log hook: the library only calls it as `log("%s", line)`, so a
/// two-pointer function takes its place (the same calling convention for
/// these arguments).
unsafe extern "C" fn m_log(_fmt: *const c_char, line: *const c_char) {
    eprintln!("osal log: {}", core::ffi::CStr::from_ptr(line).to_string_lossy());
}

static OSAL: OsalOps = OsalOps {
    init: Some(m_init),
    shutdown: None,
    gpu_alloc: Some(m_alloc),
    gpu_free: Some(m_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(m_cache),
    cache_invd: Some(m_cache),
    submit: Some(m_submit),
    wait: Some(m_wait),
    completed: Some(m_completed),
    reg_read: None,
    reg_write: None,
    query: Some(m_query),
    // SAFETY: see `m_log`.
    log: Some(unsafe {
        core::mem::transmute::<
            unsafe extern "C" fn(*const c_char, *const c_char),
            unsafe extern "C" fn(*const c_char, ...),
        >(m_log)
    }),
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/* ---- reading the draw's descriptors back ----------------------------------- */

/// The mock's GPU address of buffer `i` (`m_alloc`).
fn gpu_base(i: usize) -> u64 {
    0x40_0000_0000 + (i as u64 + 1) * (16 << 20)
}

/// `len` bytes at GPU address `gpu`, when a buffer of the mock holds them.
fn at_gpu(gpu: u64, len: usize) -> Option<Vec<u8>> {
    let bufs = BUFFERS.lock().unwrap();
    bufs.iter().enumerate().find_map(|(i, &(cpu, size))| {
        let off = usize::try_from(gpu.checked_sub(gpu_base(i))?).ok()?;
        if off.checked_add(len)? > size {
            return None;
        }
        // SAFETY: the buffer is a leaked allocation of `size` bytes and
        // nothing draws while the test reads it.
        Some(unsafe { std::slice::from_raw_parts((cpu + off) as *const u8, len) }.to_vec())
    })
}

const TABLE_ENTRIES: usize = 7;
const TABLE_BYTES: usize = TABLE_ENTRIES * 16;
const ENTRY_ATTRIBUTE: usize = 1;
const ENTRY_SAMPLER: usize = 3;
const ENTRY_TEXTURE: usize = 4;

/// A resource table: each entry's (address, size in bytes), (0, 0) for an
/// empty one.
type Table = [(u64, u64); TABLE_ENTRIES];

/// The table a 112-byte chunk holds, if it is shaped like one: every entry
/// empty or `Contains descriptors` with an address and a size (a multiple of
/// 32 bytes) inside a buffer of the mock, a sampler entry, and nothing in
/// the two entries the driver leaves empty.
fn table_shaped(c: &[u8]) -> Option<Table> {
    let mut t = [(0, 0); TABLE_ENTRIES];
    for (i, e) in t.iter_mut().enumerate() {
        let lo = u64::from_le_bytes(c[i * 16..i * 16 + 8].try_into().unwrap());
        let size = u64::from_le_bytes(c[i * 16 + 8..i * 16 + 16].try_into().unwrap());
        if (lo, size) == (0, 0) {
            continue;
        }
        let address = lo & ((1 << 56) - 1);
        if lo >> 56 != 1 || size == 0 || size % 32 != 0 || size > 32 * 64 || i > ENTRY_TEXTURE {
            return None;
        }
        at_gpu(address, size as usize)?;
        *e = (address, size);
    }
    (t[ENTRY_SAMPLER].1 != 0).then_some(t)
}

/// Every table-shaped chunk of the mock's buffers, 64-byte aligned.
fn tables(zero: bool) -> Vec<Table> {
    let bufs: Vec<(usize, usize)> = BUFFERS.lock().unwrap().clone();
    let mut found = Vec::new();
    for (cpu, size) in bufs {
        for off in (0..size.saturating_sub(TABLE_BYTES - 1)).step_by(64) {
            // SAFETY: as in `at_gpu`.
            let chunk = unsafe { std::slice::from_raw_parts_mut((cpu + off) as *mut u8, TABLE_BYTES) };
            if let Some(t) = table_shaped(chunk) {
                found.push(t);
                if zero {
                    chunk.fill(0);
                }
            }
        }
    }
    found
}

/* ---- Mesa's words ----------------------------------------------------------- */

/// The 13-bit `ulod` maximum the CSO's 1000 packs to (v10.xml:1147).
const MAX_LOD: f32 = 8191.0 / 256.0;
/// RGBA8 UNORM read RGBA (pan_format.c:508).
const FORMAT_RGBA8: u32 = 0xbb << 12;
/// `pan_translate_swizzle_4` of RGBA.
const SWIZZLE_RGBA: u32 = 1 << 3 | 2 << 6 | 3 << 9;

/// Mesa's sampler of the 2D texture (unit 0) and of the cube map (unit 1),
/// from the reference trace.
fn mesa_sampler(cube: bool) -> Sampler {
    Sampler {
        wrap_mode_s: WrapMode::CLAMP_TO_EDGE,
        wrap_mode_t: WrapMode::CLAMP_TO_EDGE,
        /* the seamless cube map's r is clamped; the 2D sampler keeps GL's REPEAT */
        wrap_mode_r: if cube { WrapMode::CLAMP_TO_EDGE } else { WrapMode::REPEAT },
        round_to_nearest_even: false,
        seamless_cube_map: true,
        normalized_coordinates: true,
        magnify_nearest: true,
        minify_nearest: true,
        mipmap_mode: if cube { MipmapMode::NEAREST } else { MipmapMode::NONE },
        lod_bias: 0.0,
        minimum_lod: 0.0,
        maximum_lod: MAX_LOD,
        ..Default::default()
    }
}

/// Mesa's texture descriptor (the fields the reference names), at `surfaces`.
fn mesa_texture(cube: bool, surfaces: u64) -> Texture {
    let (size, levels) = if cube { (16, 5) } else { (2, 1) };
    Texture {
        dimension: if cube { TextureDimension::CUBE } else { TextureDimension::_2D },
        format: FORMAT_RGBA8,
        width: size,
        height: size,
        sample_count: 1,
        swizzle: SWIZZLE_RGBA,
        levels,
        array_size: 1,
        surfaces,
        minimum_lod: 0.0,
        maximum_lod: (levels - 1) as f32,
        minimum_level: 0,
        depth: 1,
        ..Default::default()
    }
}

/// The vertex stage's two samplers, two textures and the textures' planes.
struct Stage {
    samplers: Vec<[u8; 32]>,
    textures: Vec<Texture>,
    /// Each texture's planes' (type, width, height).
    planes: Vec<Vec<(DescriptorType, u32, u32)>>,
}

fn stage_of(label: &str, t: &Table) -> Stage {
    assert_eq!(t[ENTRY_SAMPLER].1, 64, "{label}: two samplers in entry 3");
    assert_eq!(t[ENTRY_TEXTURE].1, 64, "{label}: two textures in entry 4");
    let bytes = at_gpu(t[ENTRY_SAMPLER].0, 64).unwrap();
    let samplers = bytes.as_chunks::<32>().0.to_vec();
    let bytes = at_gpu(t[ENTRY_TEXTURE].0, 64).unwrap();
    let mut textures = Vec::new();
    let mut planes = Vec::new();
    for raw in bytes.as_chunks::<32>().0 {
        let tex = Texture::unpack(raw).unwrap_or_else(|e| panic!("{label}: a texture does not unpack: {e:?}"));
        assert_eq!(tex.type_, DescriptorType::TEXTURE, "{label}");
        let n = if tex.dimension == TextureDimension::CUBE { 6 } else { 1 } * tex.levels as usize;
        let p = at_gpu(tex.surfaces, n * 32).unwrap_or_else(|| panic!("{label}: {n} planes at Surfaces"));
        planes.push(
            p.as_chunks::<32>()
                .0
                .iter()
                .map(|p| {
                    let g = GenericPlane::unpack(p).unwrap_or_else(|e| panic!("{label}: a plane: {e:?}"));
                    (g.type_, g.width, g.height)
                })
                .collect(),
        );
        textures.push(tex);
    }
    Stage { samplers, textures, planes }
}

/// The vertex stage's sampler and texture words are Mesa's.
fn check(label: &str, s: &Stage) {
    for (unit, cube) in [(0, false), (1, true)] {
        let what = if cube { "samplerCube at unit 1" } else { "sampler2D at unit 0" };
        let got = Sampler::unpack(&s.samplers[unit]).unwrap();
        assert_eq!(got, mesa_sampler(cube), "{label}: {what}: the sampler's fields are not Mesa's");
        assert_eq!(s.samplers[unit], mesa_sampler(cube).pack().unwrap(), "{label}: {what}: the sampler's words");
        let tex = &s.textures[unit];
        assert_eq!(*tex, mesa_texture(cube, tex.surfaces), "{label}: {what}: the texture's fields are not Mesa's");
        let want: Vec<_> = if cube {
            (0..30).map(|k| (DescriptorType::PLANE, 16 >> (k % 5), 16 >> (k % 5))).collect()
        } else {
            vec![(DescriptorType::PLANE, 2, 2)]
        };
        assert_eq!(s.planes[unit], want, "{label}: {what}: the planes");
    }
}

/* ---- the GL calls --------------------------------------------------------- */

/// Compile one shader; `Err` with the log when the compiler refuses it.
fn shader(ty: GLenum, text: &str) -> Result<GLuint, String> {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        let (mut log, mut n) = ([0u8; 512], 0);
        glGetShaderInfoLog(s, log.len() as GLsizei, &mut n, log.as_mut_ptr().cast());
        let log = String::from_utf8_lossy(&log[..n.max(0) as usize]).into_owned();
        if ok == 1 { Ok(s) } else { Err(log) }
    }
}

/// The GL context and an 8x8 RGBA8 framebuffer object bound.
fn setup() {
    backend::HOST_DRAW_TARGETS.store(1, Ordering::Relaxed);
    // SAFETY: static tables and one context; this is the only test of the
    // file.
    unsafe {
        osal_set_ops(&OSAL);
        backend::g_backend = &yakogl::mali::backend::yakogl_backend_mali;
        assert_eq!(yakogl::mali::backend::mali_init(), 0);
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        let (mut fb, mut rb) = (0, 0);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 8, 8);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glClearColor(0.5, 0.25, 0.75, 0.4);
        glClear(GL_COLOR_BUFFER_BIT);
        while glGetError() != GL_NO_ERROR {}
    }
}

/// The reference's textures: a 2x2 RGBA8 texture at unit 0 and a 16x16 RGBA8
/// cube map of five levels at unit 1, GL_NEAREST (the cube's min filter
/// NEAREST_MIPMAP_NEAREST) and GL_CLAMP_TO_EDGE.
fn textures() {
    const FACES: [GLenum; 6] = [
        GL_TEXTURE_CUBE_MAP_POSITIVE_X,
        GL_TEXTURE_CUBE_MAP_NEGATIVE_X,
        GL_TEXTURE_CUBE_MAP_POSITIVE_Y,
        GL_TEXTURE_CUBE_MAP_NEGATIVE_Y,
        GL_TEXTURE_CUBE_MAP_POSITIVE_Z,
        GL_TEXTURE_CUBE_MAP_NEGATIVE_Z,
    ];
    // SAFETY: plain GL calls; the texel arrays outlive each call.
    unsafe {
        let (mut t2, mut tc) = (0, 0);
        glGenTextures(1, &mut t2);
        glGenTextures(1, &mut tc);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);

        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, t2);
        let texels = [0x80u8; 2 * 2 * 4];
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 2, 2, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels.as_ptr().cast());
        for (p, v) in [
            (GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE),
            (GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE),
            (GL_TEXTURE_MIN_FILTER, GL_NEAREST),
            (GL_TEXTURE_MAG_FILTER, GL_NEAREST),
        ] {
            glTexParameteri(GL_TEXTURE_2D, p, v as GLint);
        }

        glActiveTexture(GL_TEXTURE1);
        glBindTexture(GL_TEXTURE_CUBE_MAP, tc);
        for level in 0..5 {
            let s = 16 >> level;
            let texels = vec![0x80u8; (s * s * 4) as usize];
            for face in FACES {
                glTexImage2D(face, level, GL_RGBA as GLint, s, s, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels.as_ptr().cast());
            }
        }
        for (p, v) in [
            (GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE),
            (GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE),
            (GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST),
            (GL_TEXTURE_MAG_FILTER, GL_NEAREST),
        ] {
            glTexParameteri(GL_TEXTURE_CUBE_MAP, p, v as GLint);
        }
    }
}

/// A triangle covering the target.
static POS4: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

/// The reference's pair (plain uniforms) and dEQP's own vertex shader.
const REF_VERT: &str = include_str!("../tools/mali/testdata/vs_tex2d_cube.vert");
const DEQP_VERT: &str = include_str!("../tools/mali/testdata/vs_tex2d_cube_deqp.vert");
const FRAG: &str = include_str!("../tools/mali/testdata/vs_tex2d_cube.frag");

/// Link `vert` and `frag`, set the two samplers (units 0 and 1, by the names
/// given), draw. `Err` for a shader or program the compiler does not take;
/// `Ok` the draw's `glGetError` and its refusals.
fn draw(vert: &str, frag: &str, names: [&std::ffi::CStr; 2]) -> Result<(GLenum, u32), String> {
    let v = shader(GL_VERTEX_SHADER, vert).map_err(|l| format!("vertex shader: {l}"))?;
    let f = shader(GL_FRAGMENT_SHADER, frag).map_err(|l| format!("fragment shader: {l}"))?;
    let before = backend::draws_refused();
    // SAFETY: plain GL calls; the vertex array is static.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"a_position".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        if ok != 1 {
            return Err("link failed".into());
        }
        glUseProgram(p);
        for (unit, name) in names.iter().enumerate() {
            let l = glGetUniformLocation(p, name.as_ptr());
            assert!(l >= 0, "{name:?} is no uniform");
            glUniform1i(l, unit as GLint);
        }
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        for i in 0..4 {
            glDisableVertexAttribArray(i);
        }
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS4.as_ptr().cast());
        glEnableVertexAttribArray(0);
        while glGetError() != GL_NO_ERROR {}
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
        let e = glGetError();
        Ok((e, backend::draws_refused() - before))
    }
}

/// The vertex stage's table of the draw just made: the tables with
/// attributes (entry 1) and a texture (entry 4); the stage's sampler and
/// plane words must be one set whichever of them the draw gave.
fn vertex_stage(label: &str) -> Stage {
    let found = tables(false);
    let vs: Vec<&Table> = found.iter().filter(|t| t[ENTRY_ATTRIBUTE].1 != 0 && t[ENTRY_TEXTURE].1 != 0).collect();
    assert!(!vs.is_empty(), "{label}: no vertex table with textures, found {found:x?}");
    let stages: Vec<Stage> = vs.iter().map(|t| stage_of(label, t)).collect();
    for s in &stages[1..] {
        assert_eq!(s.samplers, stages[0].samplers, "{label}: the vertex tables' samplers differ");
        assert_eq!(s.planes, stages[0].planes, "{label}: the vertex tables' planes differ");
    }
    stages.into_iter().next().unwrap()
}

#[test]
fn vertex_sampler2d_samplercube_descriptors_equal_mesas() {
    setup();
    textures();

    tables(true);
    let (label, e, refused) = match draw(REF_VERT, FRAG, [c"u_var_m0", c"u_var_m1"]) {
        Ok((GL_NO_ERROR, 0)) => ("the reference text (plain uniforms)", GL_NO_ERROR, 0),
        other => {
            eprintln!("the reference text is not built at the tip ({other:?}); drawing dEQP's struct text");
            tables(true);
            let (e, refused) =
                draw(DEQP_VERT, FRAG, [c"u_var.m0", c"u_var.m1"]).unwrap_or_else(|e| panic!("dEQP's text: {e}"));
            ("dEQP's struct text", e, refused)
        }
    };
    assert_eq!((e, refused), (GL_NO_ERROR, 0), "{label}: the draw was refused or failed");
    check(label, &vertex_stage(label));
    eprintln!(
        "mali vertex sampler2D + samplerCube descriptors vs mesa: {label} built, vertex table equals the reference"
    );
}
