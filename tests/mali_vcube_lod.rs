// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! dEQP-GLES2 shaders.texture_functions.vertex.texturecubelod draws with the
//! cube sampler `vcube-fix` built (item vcubelod-pin, plan #373, row 11 of
//! docs/es2-cts-failures.md section M.1).
//!
//! Investigation inv-texcubelod found the case has the cause of the vertex
//! cube look-ups `vcube-fix` and `attrgap-fix` corrected
//! (docs/notes/mali-cube-textures.md); no test held its shape. dEQP's case
//! (`es2fShaderTextureFunctionTests.cpp`, `textureCubeLod`, vertex stage) is a
//! vertex shader `v_color = textureCubeLod(u_sampler, a_coord, a_lod)` over a
//! 256x256 RGBA cube map with 9 levels (256 .. 1), min NEAREST_MIPMAP_NEAREST,
//! mag LINEAR, clamp to edge, and levels of detail from -1 to 9 (one per
//! vertex, an attribute). The shader and the setup here follow that case from
//! the investigation's description; the dEQP source is not in the tree.
//! This test builds that program and map through the GL core on the Mali
//! back end over a mock OSAL, draws a triangle into an 8x8 RGBA8 framebuffer
//! object, and asserts that
//!
//! - the draw is not refused (`GL_NO_ERROR`, no more in `draws_refused`), and
//! - the vertex stage's Position-table sampler the back end wrote equals the
//!   words of Mesa 26.2.1's sampler for a vertex cube look-up with those
//!   filters (`tools/mali/testdata/pan_tex_ref/vcube.filtering.nearest_mipmap_
//!   nearest_linear_clamp.desc`, the one `tests/mali_cube_samplers.rs` pins):
//!   seamless, clamp wraps, `Minify nearest`, `Magnify nearest` off, mipmap
//!   mode Nearest, maximum LOD 31.996094. Those words do not depend on the
//!   size or the level count of the map (the level count lives in the
//!   texture descriptor), so the 32x32, 6-level reference serves the
//!   256x256, 9-level map.
//!
//! The magnification switch at the ES 2.0 level of detail
//! (`tests/mali_vcube_magswitch.rs`, `lodkey::remap_mag_switch`) is not a
//! sampler word: it is a remap in the vertex program, keyed in a `report-es2`
//! build only. This build is not `report-es2`; the sampler is the shim's
//! ES 3 one (seamless).
//!
//! ```text
//! cargo test --test mali_vcube_lod --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Mutex;
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
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::mali::genxml::v10::{Func, LodAlgorithm, MipmapMode, Sampler, WrapMode};
use yakogl::osal::mali::{OsalMaliGpu, OsalMaliHeap, OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
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

/* ---- reading the sampler descriptors back -------------------------------- */

/// Whether a 32-byte chunk is shaped like a packed sampler: type `Sampler`
/// (1), reduction mode and the reserved bits of word 0 clear, `Normalized
/// Coordinates` set, bits 88..=127 clear and a zero border colour. (A packed
/// texture, a uniform or a shader word does not match all of these.)
fn sampler_shaped(c: &[u8; 32]) -> bool {
    let w0 = u32::from_le_bytes(c[0..4].try_into().unwrap());
    let w2 = u32::from_le_bytes(c[8..12].try_into().unwrap());
    w0 & 0xf == 1 && w0 & 0x30 == 0 && w0 & (1 << 25) != 0 && w2 >> 24 == 0 && c[12..16] == [0; 4] && c[16..] == [0; 16]
}

/// Every sampler-shaped chunk of the mock's buffers, 32-byte aligned.
fn sampler_chunks(zero: bool) -> Vec<[u8; 32]> {
    let mut found = Vec::new();
    for &(cpu, size) in BUFFERS.lock().unwrap().iter() {
        for off in (0..size & !31).step_by(32) {
            // SAFETY: the buffer is a leaked allocation of `size` bytes and
            // nothing draws while the test reads it.
            let chunk = unsafe { &mut *((cpu + off) as *mut [u8; 32]) };
            if sampler_shaped(chunk) {
                found.push(*chunk);
                if zero {
                    *chunk = [0; 32];
                }
            }
        }
    }
    found
}

/* ---- Mesa's sampler from a `.desc` ---------------------------------------- */

type Fields = BTreeMap<String, String>;

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The decoded fields of the first sampler of `table` ("Position",
/// "Varying" or "Fragment") in a `.desc`.
fn desc_sampler(desc: &str, table: &str) -> Fields {
    let lines: Vec<&str> = desc.lines().collect();
    let at = lines
        .iter()
        .position(|l| l.starts_with(&format!("{table} resource table @")))
        .unwrap_or_else(|| panic!("a {table} table"));
    let s = at + lines[at..].iter().position(|l| l.trim_start().starts_with("Sampler @")).expect("a sampler");
    let base = indent(lines[s]);
    let mut f = Fields::new();
    for l in lines[s + 1..].iter().take_while(|l| indent(l) > base) {
        let (k, v) = l.trim().split_once(": ").expect("a decoded field");
        assert!(f.insert(k.to_string(), v.to_string()).is_none(), "{k} twice");
    }
    f
}

fn take(f: &mut Fields, k: &str) -> String {
    f.remove(k).unwrap_or_else(|| panic!("no {k}"))
}

fn flag(f: &mut Fields, k: &str) -> bool {
    match take(f, k).as_str() {
        "true" => true,
        "false" => false,
        v => panic!("{k}: {v}"),
    }
}

fn wrap_named(v: &str) -> WrapMode {
    match v {
        "Repeat" => WrapMode::REPEAT,
        "Clamp to Edge" => WrapMode::CLAMP_TO_EDGE,
        "Mirrored Repeat" => WrapMode::MIRRORED_REPEAT,
        _ => panic!("wrap mode {v}"),
    }
}

/// Mesa's sampler from its decoded fields (every field must be one the cube
/// cases hold; an unknown value or an unread field panics).
fn mesa_sampler(mut f: Fields) -> Sampler {
    let f = &mut f;
    assert_eq!((take(f, "Type"), take(f, "Reduction mode")), ("Sampler".into(), "Average".into()));
    let border = |f: &mut Fields, k: &str| {
        u32::from_str_radix(take(f, k).split(' ').next().unwrap().trim_start_matches("0x"), 16).unwrap()
    };
    let s = Sampler {
        wrap_mode_r: wrap_named(&take(f, "Wrap Mode R")),
        wrap_mode_t: wrap_named(&take(f, "Wrap Mode T")),
        wrap_mode_s: wrap_named(&take(f, "Wrap Mode S")),
        round_to_nearest_even: flag(f, "Round to nearest even"),
        srgb_override: flag(f, "sRGB override"),
        seamless_cube_map: flag(f, "Seamless Cube Map"),
        clamp_integer_coordinates: flag(f, "Clamp integer coordinates"),
        normalized_coordinates: flag(f, "Normalized Coordinates"),
        clamp_integer_array_indices: flag(f, "Clamp integer array indices"),
        minify_nearest: flag(f, "Minify nearest"),
        magnify_nearest: flag(f, "Magnify nearest"),
        magnify_cutoff: flag(f, "Magnify cutoff"),
        mipmap_mode: match take(f, "Mipmap Mode").as_str() {
            "None" => MipmapMode::NONE,
            "Nearest" => MipmapMode::NEAREST,
            "Trilinear" => MipmapMode::TRILINEAR,
            v => panic!("mipmap mode {v}"),
        },
        minimum_lod: take(f, "Minimum LOD").parse().unwrap(),
        compare_function: match take(f, "Compare Function").as_str() {
            "Never" => Func::NEVER,
            v => panic!("compare function {v}"),
        },
        maximum_lod: take(f, "Maximum LOD").parse().unwrap(),
        lod_bias: take(f, "LOD bias").parse().unwrap(),
        maximum_anisotropy: take(f, "Maximum anisotropy").parse().unwrap(),
        lod_algorithm: match take(f, "LOD algorithm").as_str() {
            "Isotropic" => LodAlgorithm::ISOTROPIC,
            v => panic!("LOD algorithm {v}"),
        },
        border_color_r: border(f, "Border Color R"),
        border_color_g: border(f, "Border Color G"),
        border_color_b: border(f, "Border Color B"),
        border_color_a: border(f, "Border Color A"),
        ..Default::default()
    };
    assert!(f.is_empty(), "fields not read: {f:?}");
    s
}

/* ---- the GL calls --------------------------------------------------------- */

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
        let (mut log, mut n) = ([0u8; 512], 0);
        glGetShaderInfoLog(s, log.len() as GLsizei, &mut n, log.as_mut_ptr().cast());
        let log = String::from_utf8_lossy(&log[..n.max(0) as usize]).into_owned();
        assert_eq!(ok, 1, "{text:?} did not compile: {log}");
        s
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

/// dEQP's `textureCubeLod` vertex case: coordinate and level are per-vertex
/// attributes, the colour goes to the fragment stage as a varying.
const VERT: &str = "attribute highp vec2 a_position;
attribute highp vec3 a_coord;
attribute highp float a_lod;
uniform highp samplerCube u_sampler;
varying mediump vec4 v_color;

void main()
{
    gl_Position = vec4(a_position, 0.0, 1.0);
    v_color = textureCubeLod(u_sampler, a_coord, a_lod);
}
";
const FRAG: &str = include_str!("../tools/mali/testdata/pan_tex_ref/vcube.frag");
const REF: &str =
    include_str!("../tools/mali/testdata/pan_tex_ref/vcube.filtering.nearest_mipmap_nearest_linear_clamp.desc");

static POS: [f32; 6] = [-1.0, -1.0, 3.0, -1.0, -1.0, 3.0];
static COORD: [f32; 9] = [-1.0, -1.0, 1.0, 1.0, -0.5, 1.0, -1.0, 0.5, 1.0];
/// Levels of detail from -1 to 9 (dEQP's range for the case, 256x256 with 9
/// levels); the third vertex is the middle one.
static LOD: [f32; 3] = [-1.0, 9.0, 4.0];

/// The case's cube map: 256x256 RGBA, 9 levels (256 .. 1) on all six faces,
/// NEAREST_MIPMAP_NEAREST / LINEAR, clamp to edge. Unit 0 stays bound.
fn cube_texture() {
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
        let mut t = 0;
        glGenTextures(1, &mut t);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_CUBE_MAP, t);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        for level in 0..9 {
            let s = 256 >> level;
            let texels = vec![0x80u8; (s * s * 4) as usize];
            for face in FACES {
                glTexImage2D(face, level, GL_RGBA as GLint, s, s, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels.as_ptr().cast());
            }
        }
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
}

/// Link the case's program and draw one triangle. Returns the draw's
/// `glGetError` and its refusals.
fn draw() -> (GLenum, u32) {
    let (v, f) = (shader(GL_VERTEX_SHADER, VERT), shader(GL_FRAGMENT_SHADER, FRAG));
    let before = backend::draws_refused();
    // SAFETY: plain GL calls; the vertex arrays are static.
    let e = unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"a_position".as_ptr());
        glBindAttribLocation(p, 1, c"a_coord".as_ptr());
        glBindAttribLocation(p, 2, c"a_lod".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
        let l = glGetUniformLocation(p, c"u_sampler".as_ptr());
        assert!(l >= 0, "no u_sampler");
        glUniform1i(l, 0);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, POS.as_ptr().cast());
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, 3, GL_FLOAT, 0, 0, COORD.as_ptr().cast());
        glEnableVertexAttribArray(1);
        glVertexAttribPointer(2, 1, GL_FLOAT, 0, 0, LOD.as_ptr().cast());
        glEnableVertexAttribArray(2);
        while glGetError() != GL_NO_ERROR {}
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
        glGetError()
    };
    (e, backend::draws_refused() - before)
}

/// One test: the GL context is process-global, so (a) and (b) share it.
#[test]
fn texturecubelod_draws_with_mesas_vertex_cube_sampler() {
    setup();
    cube_texture();
    sampler_chunks(true);
    // (a) not refused
    let (e, refused) = draw();
    assert_eq!((e, refused), (GL_NO_ERROR, 0), "the draw was refused or failed");
    // (b) the vertex stage's sampler words are Mesa's
    let mesa = mesa_sampler(desc_sampler(REF, "Position"));
    assert_eq!(mesa, mesa_sampler(desc_sampler(REF, "Varying")), "the two vertex tables");
    let unused = mesa_sampler(desc_sampler(REF, "Fragment"));
    let (want, unused) = (mesa.pack().unwrap(), unused.pack().unwrap());
    let got = sampler_chunks(false);
    assert!(got.contains(&want), "no sampler equal to Mesa's; written {got:?}, Mesa's {want:?}");
    for c in got.iter().filter(|c| **c != want) {
        assert_eq!(c, &unused, "a sampler is neither Mesa's nor the unused-stage one");
    }
}
