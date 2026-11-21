// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The cube sampler words the board's `report-es2` build emits equal Mesa's
//! ES 2.0 rule (es2pin-cube, plan #373 causes 7 and 8).
//!
//! `tests/mali_cube_samplers.rs` and `tests/mali_vcube_lod.rs` are not
//! `report-es2`: they pin the shim's ES 3 words (seamless on, every wrap
//! clamped). The board runs the `report-es2` library, where `vcube-fix` and
//! `misc8-fix` build the ES 2.0 sampler instead (docs/notes/mali-cube-textures.md
//! section (i)): `Seamless Cube Map` clear, GL's S and T wrap modes, GL's
//! default `REPEAT` on R. Mesa 26.2.1's ES 2.0 context does the same
//! (main/version.c:682-690, st_atom_sampler.c:84-94, pan_cmdstream.c:223-231).
//! No ES 2.0 drm-shim trace exists, so the expected words are the reference's
//! (`tools/mali/testdata/pan_tex_ref/vcube.*.desc`, `cubebias.*.desc`) with
//! exactly those three fields changed, the same rule
//! `src/mali/frame_tex_tests.rs` applies; every other field must equal Mesa's.
//!
//! The test drives, through the GL core on the Mali back end over a mock of
//! its OSAL, dEQP's shapes (a cube map made with `glTexImage2D` on the six
//! faces of every level, one triangle into an 8x8 RGBA8 framebuffer object):
//!
//! - texture.vertex.cube.{filtering,wrap}.* (all 42 states, the six
//!   `nearest_mipmap_*_linear_*` ones too: the magnification remap of the
//!   vertex program, `tests/mali_vcube_magswitch.rs`, is not a sampler word),
//! - texture.mipmap.cube.{bias,basic,projected}.linear_* (the bias
//!   reference's two states; basic and projected bind the same cube state),
//! - shaders.texture_functions.vertex.texturecubelod (256x256 RGBA, 9 levels,
//!   NEAREST_MIPMAP_NEAREST / LINEAR, clamp; its words are those of
//!   `vcube.filtering.nearest_mipmap_nearest_linear_clamp`, which do not
//!   depend on size or level count).
//!
//! Each draw must not be refused (`GL_NO_ERROR`, nothing in `draws_refused`),
//! the stage's sampler must equal the expected words, and every other
//! sampler-shaped chunk the back end wrote must equal the expected words of
//! the stage that samples nothing (Mesa's table for it, with the same
//! ES 2.0 rule applied when it is a cube sampler, else Mesa's as it is).
//!
//! The texture descriptor is not compared: it holds the surface address and
//! AFBC plane words of the reference's own allocation, which no test of the
//! mock can reproduce; `src/mali/frame_tex_tests.rs` pins its fields.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table,report-es2 --test mali_cube_samplers_es2
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", feature = "report-es2"))]

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
const GPU: OsalMaliGpu =
    OsalMaliGpu { gpu_id: 0xa867_0005, tiler_features: 0x809, cs_reg_count: 96, unpreserved_cs_reg_count: 4 };
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

/// Mesa's ES 2.0 cube sampler from its ES 3 words (the rule in the header):
/// seamless off, the case's S and T wraps, `REPEAT` on R.
fn es2(mesa: Sampler, c: &Case) -> Sampler {
    Sampler {
        seamless_cube_map: false,
        wrap_mode_s: wrap_mode(c.wrap_s),
        wrap_mode_t: wrap_mode(c.wrap_t),
        wrap_mode_r: WrapMode::REPEAT,
        ..mesa
    }
}

fn wrap_mode(w: GLenum) -> WrapMode {
    match w {
        GL_REPEAT => WrapMode::REPEAT,
        GL_MIRRORED_REPEAT => WrapMode::MIRRORED_REPEAT,
        _ => WrapMode::CLAMP_TO_EDGE,
    }
}

/// The reference's state line: its first line, "# <family> <case>: cube SxS,
/// N levels, FMT; min M, mag M, wrap s W, wrap t W; uniforms".
struct Case {
    size: i32,
    levels: i32,
    rgba: bool,
    min: GLenum,
    mag: GLenum,
    wrap_s: GLenum,
    wrap_t: GLenum,
}

fn filter(v: &str) -> GLenum {
    match v {
        "NEAREST" => GL_NEAREST,
        "LINEAR" => GL_LINEAR,
        "NEAREST_MIPMAP_NEAREST" => GL_NEAREST_MIPMAP_NEAREST,
        "LINEAR_MIPMAP_NEAREST" => GL_LINEAR_MIPMAP_NEAREST,
        "NEAREST_MIPMAP_LINEAR" => GL_NEAREST_MIPMAP_LINEAR,
        "LINEAR_MIPMAP_LINEAR" => GL_LINEAR_MIPMAP_LINEAR,
        _ => panic!("filter {v}"),
    }
}

fn wrap(v: &str) -> GLenum {
    match v {
        "REPEAT" => GL_REPEAT,
        "MIRRORED_REPEAT" => GL_MIRRORED_REPEAT,
        "CLAMP_TO_EDGE" => GL_CLAMP_TO_EDGE,
        _ => panic!("wrap {v}"),
    }
}

fn case_of(desc: &str) -> Case {
    let head = desc.lines().next().expect("a header");
    let field = |k: &str| {
        head.split([';', ',', ':'])
            .map(str::trim)
            .find_map(|p| p.strip_prefix(k))
            .unwrap_or_else(|| panic!("{k} in {head}"))
    };
    let (size, _) = field("cube ").split_once('x').unwrap();
    Case {
        size: size.parse().unwrap(),
        levels: head.split(',').find_map(|p| p.trim().strip_suffix(" levels")).expect("levels").parse().unwrap(),
        rgba: head.contains(", RGBA;"),
        min: filter(field("min ")),
        mag: filter(field("mag ")),
        wrap_s: wrap(field("wrap s ")),
        wrap_t: wrap(field("wrap t ")),
    }
}

/// A cube map as dEQP makes it: every level of every face uploaded, then the
/// filters and wraps set. Texture unit 0 is left bound to it.
fn cube_texture(c: &Case) {
    const FACES: [GLenum; 6] = [
        GL_TEXTURE_CUBE_MAP_POSITIVE_X,
        GL_TEXTURE_CUBE_MAP_NEGATIVE_X,
        GL_TEXTURE_CUBE_MAP_POSITIVE_Y,
        GL_TEXTURE_CUBE_MAP_NEGATIVE_Y,
        GL_TEXTURE_CUBE_MAP_POSITIVE_Z,
        GL_TEXTURE_CUBE_MAP_NEGATIVE_Z,
    ];
    let (fmt, bpp) = if c.rgba { (GL_RGBA, 4) } else { (GL_RGB, 3) };
    // SAFETY: plain GL calls; the texel arrays outlive each call.
    unsafe {
        let mut t = 0;
        glGenTextures(1, &mut t);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_CUBE_MAP, t);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        for level in 0..c.levels {
            let s = (c.size >> level).max(1);
            let texels = vec![0x80u8; (s * s * bpp) as usize];
            for face in FACES {
                glTexImage2D(face, level, fmt as GLint, s, s, 0, fmt, GL_UNSIGNED_BYTE, texels.as_ptr().cast());
            }
        }
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, c.min as GLint);
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, c.mag as GLint);
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_S, c.wrap_s as GLint);
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_T, c.wrap_t as GLint);
    }
}

/// A triangle covering the target and a cube direction per vertex.
static POS2: [f32; 6] = [-1.0, -1.0, 3.0, -1.0, -1.0, 3.0];
static POS4: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
static DIR: [f32; 9] = [-1.0, -1.0, 1.0, 1.0, -0.5, 1.0, -1.0, 0.5, 1.0];

/// Link `vert` and `frag`, bind `pos` (`pos_comps` components) at 0 and the
/// cube direction at 1, set the uniforms (`set`, with the program current)
/// and draw. Returns the draw's `glGetError` and its refusals.
fn draw(vert: &str, frag: &str, pos: &[f32], pos_comps: i32, set: impl FnOnce(GLuint)) -> (GLenum, u32) {
    let (v, f) = (shader(GL_VERTEX_SHADER, vert), shader(GL_FRAGMENT_SHADER, frag));
    let before = backend::draws_refused();
    // SAFETY: plain GL calls; the vertex arrays are static.
    let e = unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"a_position".as_ptr());
        glBindAttribLocation(p, 1, c"a_texCoord".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
        set(p);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, pos_comps, GL_FLOAT, 0, 0, pos.as_ptr().cast());
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, 3, GL_FLOAT, 0, 0, DIR.as_ptr().cast());
        glEnableVertexAttribArray(1);
        while glGetError() != GL_NO_ERROR {}
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
        glGetError()
    };
    (e, backend::draws_refused() - before)
}

fn uniform(p: GLuint, name: &std::ffi::CStr) -> GLint {
    // SAFETY: a plain GL call with a NUL-terminated name.
    let l = unsafe { glGetUniformLocation(p, name.as_ptr()) };
    assert!(l >= 0, "{name:?} is no uniform");
    l
}

/// Zero the sampler-shaped chunks, draw, and check what the back end wrote:
/// at least one chunk is `want`, and every other one is `unused` (the sampler
/// of a stage's table the shader does not read from).
fn check(label: &str, want: &Sampler, unused: &Sampler, drew: impl FnOnce() -> (GLenum, u32)) {
    sampler_chunks(true);
    let (e, refused) = drew();
    assert_eq!((e, refused), (GL_NO_ERROR, 0), "{label}: the draw was refused or failed");
    let (want, unused) = (want.pack().unwrap(), unused.pack().unwrap());
    let got = sampler_chunks(false);
    assert!(got.contains(&want), "{label}: no sampler equal to Mesa's; written {got:?}, Mesa's {want:?}");
    for c in got.iter().filter(|c| **c != want) {
        assert_eq!(c, &unused, "{label}: a sampler is neither Mesa's nor the unused-stage one");
    }
}

/// The sampler Mesa puts in a stage's table when that stage samples nothing
/// (vcube's Fragment table, cubebias's Position table): `Mipmap Mode:
/// Nearest`, maximum LOD 0. The back end writes the same words.
fn unused_stage(desc: &str, table: &str) -> Sampler {
    mesa_sampler(desc_sampler(desc, table))
}

/* ---- the states ------------------------------------------------------------ */

const VCUBE_VERT: &str = include_str!("../tools/mali/testdata/pan_tex_ref/vcube.vert");
const VCUBE_FRAG: &str = include_str!("../tools/mali/testdata/pan_tex_ref/vcube.frag");
const CUBEBIAS_VERT: &str = include_str!("../tools/mali/testdata/pan_tex_ref/cubebias.vert");
const CUBEBIAS_FRAG: &str = include_str!("../tools/mali/testdata/pan_tex_ref/cubebias.frag");

/// The reference fragment shader with dEQP's default precision line and the
/// `highp` of its declarations lowered to `mediump`: the online compiler does not lower a `highp`
/// sampler or uniform yet (a compiler refusal of the M.2 kind, owned by the compiler
/// plans), which says nothing about the sampler descriptor this test pins.
fn cubebias_frag() -> String {
    let frag = CUBEBIAS_FRAG.replace("highp", "mediump");
    assert_ne!(frag, CUBEBIAS_FRAG);
    format!("precision mediump float;\n{frag}")
}

macro_rules! descs {
    ($fam:literal: $($c:literal)*) => {
        [$(($c, include_str!(concat!("../tools/mali/testdata/pan_tex_ref/", $fam, ".", $c, ".desc"))),)*]
    };
}

/// texture.vertex.cube.{filtering,wrap}.*: all 42 states.
const VCUBE: [(&str, &str); 42] = descs!("vcube":
    "filtering.linear_linear_clamp" "filtering.linear_linear_mirror" "filtering.linear_linear_repeat"
    "filtering.linear_mipmap_linear_linear_clamp" "filtering.linear_mipmap_linear_linear_mirror"
    "filtering.linear_mipmap_linear_linear_repeat" "filtering.linear_mipmap_linear_nearest_clamp"
    "filtering.linear_mipmap_linear_nearest_mirror" "filtering.linear_mipmap_linear_nearest_repeat"
    "filtering.linear_mipmap_nearest_linear_clamp" "filtering.linear_mipmap_nearest_linear_mirror"
    "filtering.linear_mipmap_nearest_linear_repeat" "filtering.linear_mipmap_nearest_nearest_clamp"
    "filtering.linear_mipmap_nearest_nearest_mirror" "filtering.linear_mipmap_nearest_nearest_repeat"
    "filtering.linear_nearest_clamp" "filtering.linear_nearest_mirror" "filtering.linear_nearest_repeat"
    "filtering.nearest_linear_clamp" "filtering.nearest_linear_mirror" "filtering.nearest_linear_repeat"
    "filtering.nearest_mipmap_linear_linear_clamp" "filtering.nearest_mipmap_linear_linear_mirror"
    "filtering.nearest_mipmap_linear_linear_repeat" "filtering.nearest_mipmap_linear_nearest_clamp"
    "filtering.nearest_mipmap_linear_nearest_mirror" "filtering.nearest_mipmap_linear_nearest_repeat"
    "filtering.nearest_mipmap_nearest_linear_clamp" "filtering.nearest_mipmap_nearest_linear_mirror"
    "filtering.nearest_mipmap_nearest_linear_repeat" "filtering.nearest_mipmap_nearest_nearest_clamp"
    "filtering.nearest_mipmap_nearest_nearest_mirror" "filtering.nearest_mipmap_nearest_nearest_repeat"
    "filtering.nearest_nearest_clamp" "filtering.nearest_nearest_mirror" "filtering.nearest_nearest_repeat"
    "wrap.clamp_clamp" "wrap.clamp_mirror" "wrap.clamp_repeat"
    "wrap.mirror_clamp" "wrap.mirror_mirror" "wrap.mirror_repeat"
);

/// texture.mipmap.cube.bias.linear_* (the basic and projected cases bind the
/// same cube state).
const CUBEBIAS: [(&str, &str); 2] = descs!("cubebias": "bias.linear_linear" "bias.linear_nearest");

/* ---- shaders.texture_functions.vertex.texturecubelod --------------------- */

/// dEQP's `textureCubeLod` vertex case: coordinate and level are per-vertex
/// attributes (the shader of `tests/mali_vcube_lod.rs`).
const LOD_VERT: &str = "attribute highp vec2 a_position;
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
const LOD_REF: &str =
    include_str!("../tools/mali/testdata/pan_tex_ref/vcube.filtering.nearest_mipmap_nearest_linear_clamp.desc");
/// Levels of detail from -1 to 9, one per vertex.
static LOD: [f32; 3] = [-1.0, 9.0, 4.0];

fn draw_lod() -> (GLenum, u32) {
    let (v, f) = (shader(GL_VERTEX_SHADER, LOD_VERT), shader(GL_FRAGMENT_SHADER, VCUBE_FRAG));
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
        glUniform1i(uniform(p, c"u_sampler"), 0);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, POS2.as_ptr().cast());
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, 3, GL_FLOAT, 0, 0, DIR.as_ptr().cast());
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

#[test]
fn cube_sampler_words_are_mesas_es2_words() {
    setup();
    for (name, desc) in VCUBE {
        let case = case_of(desc);
        assert_eq!((case.size, case.levels, case.rgba), (32, 6, false), "vcube {name}");
        let mesa = mesa_sampler(desc_sampler(desc, "Position"));
        assert_eq!(mesa, mesa_sampler(desc_sampler(desc, "Varying")), "vcube {name}: the two vertex tables");
        let want = es2(mesa, &case);
        assert!(!want.seamless_cube_map && want.wrap_mode_r == WrapMode::REPEAT);
        cube_texture(&case);
        check(&format!("vcube {name}"), &want, &unused_stage(desc, "Fragment"), || {
            draw(VCUBE_VERT, VCUBE_FRAG, &POS2, 2, |p| unsafe {
                glUniform1i(uniform(p, c"u_texture"), 0);
                glUniform1f(uniform(p, c"u_lod"), 0.19264507);
            })
        });
    }
    for (name, desc) in CUBEBIAS {
        let case = case_of(desc);
        assert_eq!((case.size, case.levels, case.rgba), (64, 7, true), "cubebias {name}");
        let want = es2(mesa_sampler(desc_sampler(desc, "Fragment")), &case);
        cube_texture(&case);
        check(&format!("cubebias {name}"), &want, &unused_stage(desc, "Position"), || {
            draw(CUBEBIAS_VERT, &cubebias_frag(), &POS4, 4, |p| unsafe {
                glUniform1i(uniform(p, c"u_sampler"), 0);
                glUniform1f(uniform(p, c"u_bias"), 1.0);
                glUniform1f(uniform(p, c"u_ref"), 0.0);
                glUniform4f(uniform(p, c"u_colorScale"), 1.0, 1.0, 1.0, 1.0);
                glUniform4f(uniform(p, c"u_colorBias"), 0.0, 0.0, 0.0, 0.0);
            })
        });
    }
    // texturecubelod: 256x256 RGBA, 9 levels, NEAREST_MIPMAP_NEAREST / LINEAR, clamp.
    let case = Case {
        size: 256,
        levels: 9,
        rgba: true,
        min: GL_NEAREST_MIPMAP_NEAREST,
        mag: GL_LINEAR,
        wrap_s: GL_CLAMP_TO_EDGE,
        wrap_t: GL_CLAMP_TO_EDGE,
    };
    let want = es2(mesa_sampler(desc_sampler(LOD_REF, "Position")), &case);
    cube_texture(&case);
    check("texturecubelod", &want, &unused_stage(LOD_REF, "Fragment"), draw_lod);
    eprintln!(
        "mali cube samplers (ES 2.0) vs pan_tex_ref: {} vcube + {} cubebias states + texturecubelod",
        VCUBE.len(),
        CUBEBIAS.len()
    );
}
