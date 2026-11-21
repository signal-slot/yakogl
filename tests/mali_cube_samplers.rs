// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The cube sampler words the GL core hands the Mali back end equal Mesa's
//! (cube-pin, plan #373 causes 7 and 8).
//!
//! `vcube-fix` corrected the sampler of a vertex-shader cube look-up
//! (dEQP-GLES2 texture.vertex.cube.filtering.* and .wrap.*) and `misc8-fix`
//! the cube sampler of texture.mipmap.cube.{basic,bias,projected}.linear_*;
//! `src/mali/frame_tex_tests.rs` pins both from a texture the test builds
//! itself. This test goes through the GL core: it creates the cube map as
//! dEQP does (`glTexImage2D` on the six faces of every level, `glTexParameteri`
//! for the filters and the wraps named in the reference's first line), links
//! the reference's own shader pair (`vcube.vert`/`.frag`, `cubebias.vert`/
//! `.frag`), draws one triangle through `glDrawArrays` into an 8x8 RGBA8
//! framebuffer object, into the real Mali back end over a mock of its OSAL,
//! and asserts that
//!
//! - the draw is not refused (`GL_NO_ERROR`, no more in `draws_refused`), and
//! - the 32-byte sampler descriptor the back end wrote for it equals the
//!   words of Mesa 26.2.1's sampler in `tools/mali/testdata/pan_tex_ref/*.desc`
//!   (`tools/mali/pan_tex_ref.sh`, docs/notes/mali-cube-textures.md) packed by
//!   the same `genxml` packer: the vertex stage's Position-table sampler for
//!   the 42 vcube states (6 `wrap.*`, 36 `filtering.*`) and the Fragment-table
//!   sampler for the 2 cubebias states.
//!
//! The back end also writes the sampler of the stage that samples nothing
//! (vcube's Fragment table, cubebias's Position table); it must equal Mesa's
//! sampler of that table too. The cubebias fragment shader is the reference's
//! with its `highp` declarations lowered to `mediump`: the online compiler
//! refuses a `highp` sampler and `highp` uniforms (a compiler refusal, not
//! this test's), and the sampler descriptor does not depend on precision.
//!
//! The mock gives the test no descriptor reader, so it reads the buffers the
//! mock handed out: before a draw it zeroes every 32-byte chunk there that is
//! shaped like a sampler (type `Sampler`, normalized coordinates, a zero
//! border and the rest of the sampler's unused bits), after it every such
//! chunk must be the expected one and at least one must exist.
//!
//! This build is not `report-es2`, so the context's cube maps are seamless
//! and clamped (the shim's ES 3 words): the words are Mesa's as they are.
//! The ES 2.0 words (seamless off, GL's wraps) are pinned by the frame test.
//!
//! Left out: the six vcube `filtering.nearest_mipmap_*_linear_*` states
//! (min NEAREST_MIPMAP_*, mag LINEAR with a level of detail of 0.19): their
//! remaining error is the λ=0.5 magnification switch in the vertex compiler,
//! owned by item vcube-magswitch, not the sampler. They are not listed
//! below; their samplers are covered by the frame test.
//!
//! ```text
//! cargo test --test mali_cube_samplers --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table
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
const GPU: OsalMaliGpu = OsalMaliGpu {
    gpu_id: 0xa867_0005,
    tiler_features: 0x809,
    cs_reg_count: 96,
    unpreserved_cs_reg_count: 4,
    core_features: 0,
};
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

/// texture.vertex.cube.{filtering,wrap}.*, but the six
/// `filtering.nearest_mipmap_*_linear_*` states (their remaining error is
/// the λ=0.5 magnification switch in the vertex compiler, item
/// vcube-magswitch; not the sampler). 36 states.
const VCUBE: [(&str, &str); 36] = descs!("vcube":
    "filtering.linear_linear_clamp" "filtering.linear_linear_mirror" "filtering.linear_linear_repeat"
    "filtering.linear_mipmap_linear_linear_clamp" "filtering.linear_mipmap_linear_linear_mirror"
    "filtering.linear_mipmap_linear_linear_repeat" "filtering.linear_mipmap_linear_nearest_clamp"
    "filtering.linear_mipmap_linear_nearest_mirror" "filtering.linear_mipmap_linear_nearest_repeat"
    "filtering.linear_mipmap_nearest_linear_clamp" "filtering.linear_mipmap_nearest_linear_mirror"
    "filtering.linear_mipmap_nearest_linear_repeat" "filtering.linear_mipmap_nearest_nearest_clamp"
    "filtering.linear_mipmap_nearest_nearest_mirror" "filtering.linear_mipmap_nearest_nearest_repeat"
    "filtering.linear_nearest_clamp" "filtering.linear_nearest_mirror" "filtering.linear_nearest_repeat"
    "filtering.nearest_linear_clamp" "filtering.nearest_linear_mirror" "filtering.nearest_linear_repeat"
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

#[test]
fn cube_sampler_words_equal_mesas() {
    setup();
    for (name, desc) in VCUBE {
        let case = case_of(desc);
        assert_eq!((case.size, case.levels, case.rgba), (32, 6, false), "vcube {name}");
        let mesa = mesa_sampler(desc_sampler(desc, "Position"));
        assert_eq!(mesa, mesa_sampler(desc_sampler(desc, "Varying")), "vcube {name}: the two vertex tables");
        cube_texture(&case);
        check(&format!("vcube {name}"), &mesa, &unused_stage(desc, "Fragment"), || {
            draw(VCUBE_VERT, VCUBE_FRAG, &POS2, 2, |p| unsafe {
                glUniform1i(uniform(p, c"u_texture"), 0);
                glUniform1f(uniform(p, c"u_lod"), 0.19264507);
            })
        });
    }
    for (name, desc) in CUBEBIAS {
        let case = case_of(desc);
        assert_eq!((case.size, case.levels, case.rgba), (64, 7, true), "cubebias {name}");
        let mesa = mesa_sampler(desc_sampler(desc, "Fragment"));
        cube_texture(&case);
        check(&format!("cubebias {name}"), &mesa, &unused_stage(desc, "Position"), || {
            draw(CUBEBIAS_VERT, &cubebias_frag(), &POS4, 4, |p| unsafe {
                glUniform1i(uniform(p, c"u_sampler"), 0);
                glUniform1f(uniform(p, c"u_bias"), 1.0);
                glUniform1f(uniform(p, c"u_ref"), 0.0);
                glUniform4f(uniform(p, c"u_colorScale"), 1.0, 1.0, 1.0, 1.0);
                glUniform4f(uniform(p, c"u_colorBias"), 0.0, 0.0, 0.0, 0.0);
            })
        });
    }
    eprintln!("mali cube samplers vs pan_tex_ref: {} vcube + {} cubebias states", VCUBE.len(), CUBEBIAS.len());
}
